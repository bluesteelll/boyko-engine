//! Phase X.A Wave 7 Step 7A — property-based tests for the chunked-iter
//! drivers (§11.3 of `docs/PHASE-X.A-PLAN.md`).
//!
//! The §11.3 properties pin three structural invariants of
//! [`Query::for_each_chunk`] and [`Query::par_for_each_chunk`] that the
//! single-shape Wave 4-6 unit tests cannot cover:
//!
//! 1. **Total elements seen via `for_each_chunk` across N archetypes equals
//!    the total spawned row count.** Regression check against an off-by-one
//!    in the per-archetype dispatch loop or a missed archetype in the matched
//!    set.
//! 2. **`par_for_each_chunk` results match `for_each_chunk` results modulo
//!    accumulator commutativity.** Sum-over-rows is commutative; the parallel
//!    and sequential drivers must agree on the sum.
//! 3. **No overlapping rows in the parallel variant.** Discharged via an
//!    `AtomicUsize` counter that increments by slice length — full coverage
//!    (counter == total spawned) implies disjointness (any double-counted row
//!    would push the counter strictly above the spawn count).
//!
//! # Component-id slot reservations
//!
//! This file reserves component-id slots 473-479 for the proptest pack
//! (extending the Wave 4 (460-461), Wave 5 (463), Wave 6 (466), and Wave 7
//! 7A (467-472) reservations):
//!
//!   * 473 — `PropPos` (the only payload component; all archetypes built by
//!     the property tests carry it)
//!   * 474-479 — `Marker0` .. `Marker5` (per-archetype distinguishers; up to
//!     6 unique archetypes per property)
//!
//! # Run scope (Miri)
//!
//! Run under `cargo +nightly miri test --test chunk_proptest` is **NOT**
//! recommended at the default 256-case proptest budget — each case spawns
//! thousands of entities and Miri's per-allocation overhead is prohibitive.
//! Per plan §11.5 / §11.3 the small-generator proptest variants live inside
//! `chunk_iter::tests` (module-scope; future Wave 7 expansion) where the
//! generators are bounded to ≤ 64 rows per archetype for Miri compatibility.
//! This integration-level harness uses the default proptest cargo-test budget
//! with up-to-2000-rows-per-archetype generators.

#![allow(clippy::needless_borrow)]

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::par_iter::BatchingStrategy;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use proptest::prelude::*;

// ── Component pack ──────────────────────────────────────────────────────────

const COMP_POS: ComponentId = ComponentId(473);
const COMP_M0: ComponentId = ComponentId(474);
const COMP_M1: ComponentId = ComponentId(475);
const COMP_M2: ComponentId = ComponentId(476);
const COMP_M3: ComponentId = ComponentId(477);
const COMP_M4: ComponentId = ComponentId(478);
const COMP_M5: ComponentId = ComponentId(479);

#[repr(C)]
#[derive(Clone, Copy)]
struct PropPos(u32);

impl Component for PropPos {
    fn component_id() -> ComponentId {
        COMP_POS
    }
}

macro_rules! marker {
    ($name:ident, $slot:expr) => {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct $name(u32);
        impl Component for $name {
            fn component_id() -> ComponentId {
                $slot
            }
        }
    };
}
marker!(Marker0, COMP_M0);
marker!(Marker1, COMP_M1);
marker!(Marker2, COMP_M2);
marker!(Marker3, COMP_M3);
marker!(Marker4, COMP_M4);
marker!(Marker5, COMP_M5);

const MARKER_SLOTS: [ComponentId; 6] = [COMP_M0, COMP_M1, COMP_M2, COMP_M3, COMP_M4, COMP_M5];

/// Idempotent global registry priming. `register_layout` is `set`-style on the
/// global slot table — calling it from every proptest case is sound (the slot
/// is written once per process; subsequent calls are no-ops via the slot's
/// `Layout`-equality guard).
fn prime_registry() {
    component_registry::register_layout::<PropPos>(COMP_POS.0);
    component_registry::register_layout::<Marker0>(COMP_M0.0);
    component_registry::register_layout::<Marker1>(COMP_M1.0);
    component_registry::register_layout::<Marker2>(COMP_M2.0);
    component_registry::register_layout::<Marker3>(COMP_M3.0);
    component_registry::register_layout::<Marker4>(COMP_M4.0);
    component_registry::register_layout::<Marker5>(COMP_M5.0);
}

/// Builds an `EcsMaster` populated with `archetype_counts[i]` entities in
/// archetype `i`. Archetype `i` carries `(PropPos, Marker{i})` — a unique
/// component shape ⇒ a unique archetype slot, regardless of the row count.
/// Returns the master plus the spawned `(ArchetypeId, row_count)` pairs.
fn build_world(archetype_counts: &[usize]) -> (EcsMaster, Vec<(ArchetypeId, usize)>) {
    prime_registry();
    let mut ecs = EcsMaster::new();
    let mut arches: Vec<(ArchetypeId, usize)> = Vec::with_capacity(archetype_counts.len());

    for (idx, &count) in archetype_counts.iter().enumerate() {
        let marker_slot = MARKER_SLOTS[idx];
        let arch = ecs.create_archetype(&[COMP_POS, marker_slot]);
        for row in 0..count {
            let pos = PropPos(row as u32);
            // Marker payload is unused — the marker exists only to distinguish
            // the archetype shape.
            let marker_payload: u32 = 0;
            // SAFETY: `PropPos` and `MarkerN` are `#[repr(C)]` POD; the byte
            //   slices are valid for the duration of this call.
            let pos_bytes = unsafe {
                std::slice::from_raw_parts(
                    &pos as *const PropPos as *const u8,
                    std::mem::size_of::<PropPos>(),
                )
            };
            let marker_bytes = unsafe {
                std::slice::from_raw_parts(
                    &marker_payload as *const u32 as *const u8,
                    std::mem::size_of::<u32>(),
                )
            };
            ecs.create_entity(
                arch,
                &[(COMP_POS, pos_bytes), (marker_slot, marker_bytes)],
            )
            .expect("proptest build_world: create_entity must succeed");
        }
        arches.push((arch, count));
    }
    (ecs, arches)
}

/// Sums every `PropPos.0` across every matched archetype via
/// `EcsMaster::query::<&PropPos, ()>().for_each_chunk(...)`. This is the
/// "expected" reference value the parallel driver must match.
fn sum_sequential(ecs: &mut EcsMaster) -> u64 {
    let mut sum: u64 = 0;
    {
        let mut view = ecs.query::<&PropPos, ()>();
        view.for_each_chunk(|slice: &[PropPos]| {
            for p in slice {
                sum = sum.wrapping_add(u64::from(p.0));
            }
        });
    }
    sum
}

/// Sums every `PropPos.0` across every matched archetype via the parallel
/// driver. The atomic counter sidesteps the `Fn` (not `FnMut`) constraint on
/// the per-row body — sum-via-AtomicU64 is commutative, so out-of-order
/// dispatch by the worker pool does not perturb the result.
fn sum_parallel(ecs: &mut EcsMaster, pool_threads: usize) -> u64 {
    let acc = std::sync::atomic::AtomicU64::new(0);
    let pool = boyko_threadpool::ThreadPoolBuilder::new()
        .num_threads(pool_threads)
        .build();
    pool.install(|_scope| {
        let mut view = ecs.query::<&PropPos, ()>();
        view.par_for_each_chunk(
            |slice: &[PropPos]| {
                let mut local: u64 = 0;
                for p in slice {
                    local = local.wrapping_add(u64::from(p.0));
                }
                acc.fetch_add(local, Ordering::Relaxed);
            },
            BatchingStrategy::default(),
        );
    });
    acc.load(Ordering::Relaxed)
}

/// Total rows seen by the sequential driver — sums slice lengths instead of
/// values. Pinned by the §11.3 invariant 1.
fn total_rows_sequential(ecs: &mut EcsMaster) -> usize {
    let mut total: usize = 0;
    {
        let mut view = ecs.query::<&PropPos, ()>();
        view.for_each_chunk(|slice: &[PropPos]| {
            total += slice.len();
        });
    }
    total
}

/// Total rows seen by the parallel driver via an `AtomicUsize` counter —
/// any overlap between worker sub-ranges would push this strictly above the
/// spawn count, so equality verifies disjointness (§11.3 invariant 3).
fn total_rows_parallel(ecs: &mut EcsMaster, pool_threads: usize) -> usize {
    let counter = AtomicUsize::new(0);
    let pool = boyko_threadpool::ThreadPoolBuilder::new()
        .num_threads(pool_threads)
        .build();
    pool.install(|_scope| {
        let mut view = ecs.query::<&PropPos, ()>();
        view.par_for_each_chunk(
            |slice: &[PropPos]| {
                counter.fetch_add(slice.len(), Ordering::Relaxed);
            },
            BatchingStrategy::default(),
        );
    });
    counter.load(Ordering::Relaxed)
}

// ── Generators ──────────────────────────────────────────────────────────────

/// Generator: 1..=6 archetypes, each with 0..=2000 rows. The 6-archetype
/// upper bound matches the `MARKER_SLOTS` table; the 2000-row upper bound
/// keeps each property case under 12k total entities for cargo-test budget.
fn archetype_counts_strategy() -> impl Strategy<Value = Vec<usize>> {
    prop::collection::vec(0usize..=2000usize, 1..=6)
}

// ── Properties ──────────────────────────────────────────────────────────────

proptest! {
    /// §11.3 invariant 1: total rows seen via the sequential
    /// `for_each_chunk` equals the total spawned row count, for any
    /// archetype-count layout.
    ///
    /// Catches: off-by-one in per-archetype dispatch, missed archetype in the
    /// matched set, double-counted rows in the slice-length sum.
    #[test]
    fn prop_multi_archetype_total_rows_equals_entity_count(
        counts in archetype_counts_strategy(),
    ) {
        let expected: usize = counts.iter().sum();
        let (mut ecs, _arches) = build_world(&counts);
        let observed = total_rows_sequential(&mut ecs);
        prop_assert_eq!(
            observed, expected,
            "sequential for_each_chunk total ({}) must equal spawn count ({}); \
             counts = {:?}",
            observed, expected, counts,
        );
    }

    /// §11.3 invariant 3 (parallel variant of invariant 1): under the
    /// parallel driver with a 2-worker pool, every row is processed exactly
    /// once — total slice-length sum equals spawn count. Equality verifies
    /// disjointness structurally (any overlap would push the counter
    /// strictly above the spawn count).
    #[test]
    fn prop_parallel_total_rows_no_overlap(
        counts in archetype_counts_strategy(),
    ) {
        let expected: usize = counts.iter().sum();
        let (mut ecs, _arches) = build_world(&counts);
        let observed = total_rows_parallel(&mut ecs, 2);
        prop_assert_eq!(
            observed, expected,
            "parallel par_for_each_chunk total ({}) must equal spawn count ({}); \
             any overlap would push observed strictly above expected. counts = {:?}",
            observed, expected, counts,
        );
    }

    /// §11.3 invariant 2: parallel and sequential drivers agree on the sum
    /// of every `PropPos.0` across every matched archetype. The user closure
    /// body is a sum (commutative ⇒ insensitive to dispatch order); the
    /// drivers must converge on the same value modulo accumulator
    /// commutativity.
    ///
    /// Catches: stride / slice-bounds bug in `par_for_each_chunk` that drops
    /// rows or duplicates them (either would shift the sum), bug in the
    /// `BatchingStrategy::chunk_size` math that yields off-by-one sub-ranges.
    #[test]
    fn prop_parallel_sum_matches_sequential_sum(
        counts in archetype_counts_strategy(),
    ) {
        let (mut ecs_seq, _) = build_world(&counts);
        let seq_sum = sum_sequential(&mut ecs_seq);

        let (mut ecs_par, _) = build_world(&counts);
        let par_sum = sum_parallel(&mut ecs_par, 2);

        prop_assert_eq!(
            par_sum, seq_sum,
            "parallel sum ({}) must equal sequential sum ({}); counts = {:?}",
            par_sum, seq_sum, counts,
        );
    }
}
