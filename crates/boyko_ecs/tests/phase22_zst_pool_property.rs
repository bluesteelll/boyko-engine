//! Phase 22 (Tags) — property-based test (proptest), plan suite (b):
//! a model test for the tick-only ZST `ComponentPool` (plan D1/D6).
//!
//! A random `add` / `swap_remove` / `pop` sequence runs against the REAL
//! pool and a trivial reference model; after EVERY operation the public
//! observables must agree:
//!
//! * `count()` == model length; `capacity()` is constant;
//! * `committed_rows()` follows the GROW1-ZST frontier (0 before the first
//!   add; `min(granule_rows, reserve)` == reserve after it — one tick
//!   granule covers 16,384 rows, far above the test reserve) and is
//!   monotone;
//! * `add_typed` returns the tail index until the reserve ceiling, then
//!   `None` with ZERO observable state change;
//! * `swap_remove(i)` succeeds iff `i < len`; `pop()` iff `len > 0`;
//! * `get_raw` / `get_typed` answer `Some` exactly on `[0, len)`, always at
//!   the SAME dangling base (`row_ptr == buffer` at stride 0);
//! * a Drop-impl ZST is dropped EXACTLY once per logical removal and once
//!   per survivor at pool Drop (the dual model in the second test).
//!
//! # Scope note — ticks
//!
//! The pool's tick accessors (`read_added_tick` / `fill_ticks` / ...) are
//! `pub(crate)` and unreachable from `tests/` (same constraint the
//! `miri_pool_growth` suite documents). The tick half of the plan's
//! "len/tick/grow model" is pinned by the in-crate Wave-0 unit tests
//! (`zst_pool_tick_stamping_and_swap_lockstep`,
//! `zst_pool_growth_two_successive_commits`) and at the world level by
//! `phase22_static_tags::tag_ticks_survive_neighbor_swap_remove`. This file
//! models the publicly observable len/grow/drop surface.

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::memory::component_pool::ComponentPool;
use boyko_macros::Component;
use proptest::prelude::*;

const SEQ: Ordering = Ordering::SeqCst;

/// Pool reserve ceiling — small so random sequences actually REACH the
/// ceiling arm (`add -> None`), yet still well under one tick granule
/// (16,384 rows), so the frontier jumps 0 -> RESERVE on the first add.
const RESERVE: usize = 48;

/// A generated pool operation. `swap_remove` carries a RAW index that may
/// deliberately fall out of bounds (the `false` arm is part of the model).
#[derive(Debug, Clone)]
enum PoolOp {
    Add,
    SwapRemove(usize),
    Pop,
}

fn pool_op_strategy() -> impl Strategy<Value = PoolOp> {
    prop_oneof![
        4 => Just(PoolOp::Add),
        2 => (0..RESERVE + 4).prop_map(PoolOp::SwapRemove),
        1 => Just(PoolOp::Pop),
    ]
}

// ════════════════════════════════════════════════════════════════════════════
// Model 1 — len / grow observables (drop-less ZST)
// ════════════════════════════════════════════════════════════════════════════

/// Data-less, drop-less tag fixture (derive-minted id, no pinned slot).
#[derive(Component)]
#[derive(Clone, Copy)]
struct PModelTag;

proptest! {
    /// Random add/swap_remove/pop sequence: every public observable of the
    /// ZST pool must match the reference model after every operation.
    #[test]
    fn zst_pool_len_and_grow_match_model(
        ops in proptest::collection::vec(pool_op_strategy(), 1..96)
    ) {
        let mut pool = ComponentPool::new(PModelTag::component_id().0, RESERVE);
        prop_assert_eq!(pool.component_layout().size(), 0, "fixture must be a ZST");
        prop_assert_eq!(pool.capacity(), RESERVE);
        prop_assert_eq!(pool.committed_rows(), 0, "zero initial commit");

        // The dangling base exists before any growth and never moves.
        let base = pool.buffer_ptr();

        let mut len = 0usize; // the reference model
        let mut grew = false; // committed frontier: 0 until the first add

        for (step, op) in ops.into_iter().enumerate() {
            match op {
                PoolOp::Add => {
                    if len < RESERVE {
                        prop_assert_eq!(
                            pool.add_typed(PModelTag),
                            Some(len),
                            "step {}: add must return the tail index",
                            step
                        );
                        len += 1;
                        grew = true;
                    } else {
                        let before = (pool.count(), pool.committed_rows());
                        prop_assert_eq!(
                            pool.add_typed(PModelTag),
                            None,
                            "step {}: add at the reserve ceiling must be rejected",
                            step
                        );
                        prop_assert_eq!(
                            (pool.count(), pool.committed_rows()),
                            before,
                            "step {}: rejected add must change NOTHING",
                            step
                        );
                    }
                }
                PoolOp::SwapRemove(idx) => {
                    let in_bounds = idx < len;
                    prop_assert_eq!(
                        pool.swap_remove(idx),
                        in_bounds,
                        "step {}: swap_remove({}) success iff idx < len ({})",
                        step, idx, len
                    );
                    if in_bounds {
                        len -= 1;
                    }
                }
                PoolOp::Pop => {
                    prop_assert_eq!(
                        pool.pop(),
                        len > 0,
                        "step {}: pop success iff the pool is non-empty",
                        step
                    );
                    len = len.saturating_sub(1);
                }
            }

            // Public observables vs the model, after EVERY op.
            prop_assert_eq!(pool.count(), len, "step {}: count", step);
            prop_assert_eq!(pool.capacity(), RESERVE, "step {}: capacity is constant", step);
            prop_assert_eq!(
                pool.committed_rows(),
                if grew { RESERVE } else { 0 },
                "step {}: GROW1-ZST frontier (one granule covers the reserve)",
                step
            );
            prop_assert_eq!(pool.is_full(), len == RESERVE, "step {}: is_full", step);
            prop_assert_eq!(
                pool.remaining_capacity(),
                RESERVE - len,
                "step {}: remaining_capacity",
                step
            );
            prop_assert_eq!(
                pool.buffer_ptr(),
                base,
                "step {}: the dangling base must never move",
                step
            );
            // Row visibility boundary: Some on [0, len), None at len.
            if len > 0 {
                prop_assert_eq!(
                    pool.get_raw(len - 1),
                    Some(base),
                    "step {}: last live row reads back at the dangling base",
                    step
                );
                prop_assert!(
                    pool.get_typed::<PModelTag>(len - 1).is_some(),
                    "step {}: typed ZST read on the last live row",
                    step
                );
            }
            prop_assert!(
                pool.get_raw(len).is_none(),
                "step {}: first dead row must read None",
                step
            );
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Model 2 — drop accounting (Drop-impl ZST)
// ════════════════════════════════════════════════════════════════════════════

/// A ZST WITH a Drop impl (`needs_drop` => `drop_fn` Some). The counter is
/// a static — a counting FIELD would make the type non-zero-sized — so this
/// fixture is owned by exactly ONE test (the proptest cases of a single
/// `#[test]` run sequentially, so baseline deltas are race-free).
#[derive(Component)]
struct PDropTag;

static P_DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

impl Drop for PDropTag {
    fn drop(&mut self) {
        P_DROP_COUNT.fetch_add(1, SEQ);
    }
}

proptest! {
    /// Every logical removal drops the Drop-impl ZST exactly once; pool
    /// Drop accounts exactly the survivors; total drops == total adds.
    #[test]
    fn zst_pool_drop_accounting_matches_model(
        ops in proptest::collection::vec(pool_op_strategy(), 1..96)
    ) {
        let mut pool = ComponentPool::new(PDropTag::component_id().0, RESERVE);
        let baseline = P_DROP_COUNT.load(SEQ);

        let mut len = 0usize;
        let mut constructed = 0usize; // every PDropTag value ever created
        let mut drops = 0usize; // model: one drop per logical death

        for (step, op) in ops.into_iter().enumerate() {
            match op {
                PoolOp::Add => {
                    constructed += 1;
                    if len < RESERVE {
                        prop_assert!(pool.add_typed(PDropTag).is_some());
                        len += 1;
                    } else {
                        // The rejected value is dropped by the CALLER frame
                        // (by-value argument of a failed add) — still
                        // exactly once, never zero, never twice.
                        prop_assert!(pool.add_typed(PDropTag).is_none());
                        drops += 1;
                    }
                }
                PoolOp::SwapRemove(idx) => {
                    if pool.swap_remove(idx) {
                        prop_assert!(idx < len, "step {}: swap_remove oracle", step);
                        len -= 1;
                        drops += 1;
                    } else {
                        prop_assert!(idx >= len, "step {}: rejected swap_remove oracle", step);
                    }
                }
                PoolOp::Pop => {
                    if pool.pop() {
                        prop_assert!(len > 0, "step {}: pop oracle", step);
                        len -= 1;
                        drops += 1;
                    } else {
                        prop_assert_eq!(len, 0, "step {}: rejected pop oracle", step);
                    }
                }
            }
            prop_assert_eq!(pool.count(), len, "step {}: count", step);
            prop_assert_eq!(
                P_DROP_COUNT.load(SEQ) - baseline,
                drops,
                "step {}: each removal drops EXACTLY once (no leak, no double drop)",
                step
            );
        }

        // Teardown: pool Drop accounts exactly the survivors...
        drop(pool);
        prop_assert_eq!(
            P_DROP_COUNT.load(SEQ) - baseline,
            drops + len,
            "pool Drop must drop each survivor exactly once"
        );
        // ...and the global balance closes: every constructed value died
        // exactly once across its whole lifecycle (add / reject / remove /
        // teardown) — no leak, no double drop.
        prop_assert_eq!(
            P_DROP_COUNT.load(SEQ) - baseline,
            constructed,
            "drop balance: every constructed PDropTag dropped exactly once"
        );
    }
}
