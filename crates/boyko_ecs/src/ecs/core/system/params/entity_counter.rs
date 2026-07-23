//! `EntityCounter<'s>` — minimal projection of `EntityMaster::next_entity_id`.
//!
//! Phase 11 Round 3 (C-N1, EM6 — plan §5.5, §11.10, §12.1). The newtype
//! encapsulates a raw pointer to the world's atomic entity-id counter so
//! that worker code holding a [`Commands<'s>`](super::commands::Commands) can
//! reserve Entity IDs without exposing the full `&EntityMaster`. The field
//! type of the inner pointer (`*const AtomicUsize`) makes the EM6
//! field-restriction invariant **type-enforced**: there is no compile-time
//! path from an `EntityCounter` to any non-atomic `EntityMaster` field.
//!
//! # Soundness contract (`EntityCounter::from_ptr` SAFETY)
//!
//! The `from_ptr` constructor is `unsafe`; callers (Phase 11 limits them
//! to [`crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::entity_counter`])
//! must guarantee:
//!
//! 1. The pointer is valid for the lifetime `'s` (provenance + non-dangling).
//! 2. The atomic is `EntityMaster::next_entity_id` of an `EcsMaster` whose
//!    lifetime contains `'s`.
//! 3. EM6: no other code path may use the pointer to reach other
//!    `EntityMaster` fields — guaranteed by construction because the
//!    pointer's destination type is `AtomicUsize`, not `EntityMaster`.
//!
//! # Send / Sync (EC1 / EM5)
//!
//! `EntityCounter<'s>: Send + Sync` via explicit impls — the only operation
//! is atomic RMW (`fetch_add(Relaxed)`), which is data-race-free from any
//! thread. Workers carry `EntityCounter` inside their per-system
//! `Commands<'s>` view, which is `!Sync` for an independent reason
//! (`&mut CommandQueue`, CQ-SEND2).

// `EntityCounter` is consumed exclusively by `Commands<'s>` (Wave B) and the
// `UnsafeEcsCell::entity_counter` projection. The lib build does not exercise
// the path until Wave C wires the `Commands::spawn` return path; mirror the
// pattern of `commands.rs` / `command_queue.rs` and suppress dead_code until
// the first public consumer lands.
#![allow(dead_code)]

use core::marker::PhantomData;
use core::ops::Range;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::ecs::core::entity::entity::Entity;
use crate::ecs::error::{EcsError, EcsResult};
use crate::ecs::identifiers::primitives::EntityId;

/// Phase 12.5 (SBO17): the maximum number of entities one `spawn_batch` call
/// may reserve. Hard cap; over-budget batches return `Err` without advancing
/// the counter. See `ecs_master.rs` for the world-side capacity contract.
pub(crate) const MAX_BATCH_HINT: usize = 8_192;

/// Minimal projection of [`crate::ecs::core::entity::entity_master::EntityMaster`]
/// exposing only the atomic `next_entity_id` counter for thread-safe entity
/// reservation from system bodies.
///
/// # Layout (plan §11.10)
///
/// ```text
/// +0  : next_id_ptr: *const AtomicUsize  (8 B)
/// +8  : _marker: PhantomData             (0 B ZST)
/// +8  : end
/// ```
///
/// Size 8 B; align 8. `#[derive(Clone, Copy)]` — trivial copy through
/// register-passing arguments.
///
/// # Lifetime (`'s`)
///
/// `'s` is the system's state scope (the lifetime of the per-system
/// `CommandQueue` slot that the parent [`Commands<'s>`](super::commands::Commands)
/// borrows). The pointer is minted from `UnsafeEcsCell<'w>` once per
/// system invocation (plan §8.7 — `'w >= 's`) and re-tagged to `'s` via
/// `PhantomData`.
#[derive(Clone, Copy)]
pub struct EntityCounter<'s> {
    /// Raw pointer to `EntityMaster::next_entity_id`.
    ///
    /// Minted from `UnsafeEcsCell<'w>` inside `Commands::get_param` and
    /// dereferenced only through `fetch_add(Relaxed)` in
    /// [`reserve_entity`](Self::reserve_entity).
    next_id_ptr: *const AtomicUsize,

    /// Variance marker — ties the pointer's apparent validity to `'s`.
    _marker: PhantomData<&'s AtomicUsize>,
}

// SAFETY (EC1, EM5, plan §5.5):
//   `EntityCounter` carries a `*const AtomicUsize`. The only path that
//   dereferences the pointer is `reserve_entity`, which performs an atomic
//   RMW (`fetch_add(Relaxed)`). Atomic operations from any thread are
//   data-race-free; no plain memory access is possible through this type
//   (the destination type is `AtomicUsize`, never reinterpreted).
unsafe impl<'s> Send for EntityCounter<'s> {}

// SAFETY (EC1, EM5, plan §5.5):
//   Same composition as `Send`. `&EntityCounter` exposes only
//   `reserve_entity(&self)` which performs an atomic RMW; concurrent
//   immutable references from multiple threads alias only the atomic and
//   are sound.
unsafe impl<'s> Sync for EntityCounter<'s> {}

// Compile-time size + align contract (plan §11.10 — 8 B).
// `EntityCounter` wraps a single `*const AtomicUsize`, so its size/align equal
// the pointer width; the 8-byte figures encode the 64-bit ABI. Gated to 64-bit
// (the engine's supported platform) — see CLAUDE.md target platform.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<EntityCounter<'static>>() == 8);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::align_of::<EntityCounter<'static>>() == 8);

impl<'s> EntityCounter<'s> {
    /// Constructs an `EntityCounter` from a raw pointer to the atomic
    /// counter.
    ///
    /// # Safety (plan §5.5)
    ///
    /// * `ptr` must be a valid `*const AtomicUsize` for the entirety of `'s`.
    /// * The pointed-to atomic must be the `next_entity_id` field of an
    ///   `EntityMaster` whose lifetime contains `'s` — minted by
    ///   [`crate::ecs::core::entity::entity_master::EntityMaster::next_id_atomic`].
    /// * Caller asserts the EM6 invariant: no other code path uses the
    ///   pointer to reach a different `EntityMaster` field. Type-enforced
    ///   by construction — the pointer's destination type is `AtomicUsize`.
    #[inline]
    pub(crate) unsafe fn from_ptr(ptr: *const AtomicUsize) -> Self {
        Self { next_id_ptr: ptr, _marker: PhantomData }
    }

    /// Atomically reserves a fresh Entity ID — lock-free.
    ///
    /// Performs `fetch_add(1, Ordering::Relaxed)` on the atomic counter and
    /// returns an [`Entity`] with generation `0` (fresh path, EM1). The
    /// caller is responsible for enqueueing a [`crate::ecs::core::commands::Command`]
    /// (typically `SpawnAtCommand`) that will register the reserved ID into
    /// the fast store on apply (EM3).
    ///
    /// # Cost
    ///
    /// `~10 ns` single-thread (uncontended `lock xadd` on x86_64); up to
    /// `~60 ns` under N=8 worker contention (plan §10.5).
    ///
    /// # Atomic ordering
    ///
    /// `Ordering::Relaxed` is sufficient — uniqueness only; happens-before
    /// for the returned ID is established later by the apply-window barrier
    /// (SCH7) when workers join and the dispatcher reads the queue.
    #[inline]
    pub fn reserve_entity(&self) -> Entity {
        // SAFETY (EM5, EM6, plan §5.5):
        //   * `next_id_ptr` was minted by `UnsafeEcsCell::entity_counter`
        //     (plan §12.7) from a live `EntityMaster::next_id_atomic()`
        //     projection.
        //   * The pointer's apparent lifetime `'s` is bounded by `'w >= 's`
        //     per the Phase 8c IntoSystem contract (plan §8.7 — `get_param`
        //     runs once per system invocation; `Commands<'s>::Item<'w, 's>`
        //     is dropped at body end so the pointer never outlives `'w`).
        //   * Atomic RMW from any thread is data-race-free.
        let id = unsafe { (*self.next_id_ptr).fetch_add(1, Ordering::Relaxed) };
        debug_assert!(id < usize::MAX / 2, "EntityId counter near exhaustion");
        Entity::new(EntityId(id), 0)
    }

    /// Phase 12.5 Opt-A2 (SBO17 / plan §5.3): atomically reserves a
    /// contiguous range of `n` fresh entity IDs.
    ///
    /// Validates `n ≤ MAX_BATCH_HINT` BEFORE any atomic operation. Returns
    /// `Err(EcsError::SpawnBatchExceedsCapacity)` on overrun — **the
    /// counter is not advanced** (SBO17 strong form).
    ///
    /// On success, performs a single `fetch_add(n, Ordering::Relaxed)` and
    /// returns the half-open range `start..(start + n)`. Workers calling
    /// `reserve_batch(n)` in parallel observe disjoint ranges (EM4
    /// atomic-uniqueness).
    ///
    /// # Cost
    ///
    /// `~10 ns` single-thread (one `lock xadd`); contention scales with
    /// the number of concurrent batch-callers — but since each call
    /// reserves up to `MAX_BATCH_HINT = 8 192` IDs in one atomic, the
    /// amortised per-entity cost is sub-nanosecond.
    ///
    /// # Atomic ordering
    ///
    /// `Ordering::Relaxed` — same rationale as
    /// [`Self::reserve_entity`]. The happens-before edge that publishes
    /// the range's bytes to the dispatcher is established by the
    /// apply-window barrier (SCH7).
    #[inline]
    pub fn reserve_batch(&self, n: usize) -> EcsResult<Range<usize>> {
        // SBO17 cap check BEFORE atomic — the counter never advances on Err.
        if n > MAX_BATCH_HINT {
            return Err(EcsError::SpawnBatchExceedsCapacity {
                requested: n,
                max: MAX_BATCH_HINT,
            });
        }
        // SAFETY (EM5, EM6, plan §5.5): same contract as `reserve_entity`.
        //   The atomic is `EntityMaster::next_entity_id` projected through
        //   `next_id_ptr`; the destination type is `AtomicUsize`, so no
        //   non-atomic field is reachable. Atomic RMW from any thread is
        //   data-race-free.
        let start = unsafe { (*self.next_id_ptr).fetch_add(n, Ordering::Relaxed) };
        debug_assert!(
            start.checked_add(n).is_some_and(|end| end < usize::MAX / 2),
            "EntityId counter near exhaustion"
        );
        Ok(start..(start + n))
    }
}

#[cfg(test)]
mod tests {
    // Test-only harness state (shared counters / observation channels behind a
    // std lock); the reference model the param's atomic behaviour is checked
    // against. Compiled out of every shipping build.
    #![allow(clippy::disallowed_types)]

    use super::*;
    use core::sync::atomic::AtomicUsize;

    /// `EntityCounter` carries exactly a single pointer — 8 B, align 8.
    /// Mirrors the plan §11.10 layout contract and the module-level
    /// `const _: () = assert!(...)` guards.
    #[test]
    fn entity_counter_size_is_8_bytes() {
        assert_eq!(core::mem::size_of::<EntityCounter<'_>>(), 8);
        assert_eq!(core::mem::align_of::<EntityCounter<'_>>(), 8);
    }

    /// `EntityCounter<'static>` satisfies Send + Sync. Compile-time gate;
    /// the unsafe impl blocks above are the load-bearing declarations.
    #[test]
    fn entity_counter_is_send_and_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<EntityCounter<'static>>();
        assert_sync::<EntityCounter<'static>>();
    }

    /// Repeated reserves yield strictly distinct IDs — the atomic counter
    /// advances monotonically (EM4).
    #[test]
    fn entity_counter_reserve_distinct_ids() {
        let counter_storage = AtomicUsize::new(0);
        // SAFETY: `counter_storage` lives for the entirety of this test.
        let counter = unsafe { EntityCounter::from_ptr(&counter_storage) };
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1024 {
            let e = counter.reserve_entity();
            assert_eq!(e.generation(), 0, "fresh reserves carry generation 0 (EM1)");
            assert!(seen.insert(e.id().0), "reserved ID {} repeated", e.id().0);
        }
        assert_eq!(seen.len(), 1024);
    }

    /// 8-thread × 1000 reserves = 8000 distinct IDs (EM4 atomic
    /// uniqueness proof, scaled-down version of the loom test in §13.7).
    #[test]
    fn entity_counter_reserve_lock_free_8_threads() {
        use std::sync::Arc;
        use std::thread;

        let counter_storage = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::with_capacity(8);
        for _ in 0..8 {
            let storage = Arc::clone(&counter_storage);
            handles.push(thread::spawn(move || {
                // SAFETY: `storage` keeps the atomic alive for the
                // thread's lifetime; the raw pointer is valid until
                // the Arc is dropped on the main thread (after joins).
                let counter = unsafe { EntityCounter::from_ptr(Arc::as_ptr(&storage)) };
                let mut ids = Vec::with_capacity(1000);
                for _ in 0..1000 {
                    ids.push(counter.reserve_entity().id().0);
                }
                ids
            }));
        }
        let mut all_ids = std::collections::HashSet::new();
        for h in handles {
            let ids = h.join().expect("worker thread must not panic");
            for id in ids {
                assert!(all_ids.insert(id), "ID {} collided across threads", id);
            }
        }
        assert_eq!(all_ids.len(), 8 * 1000, "8 threads × 1000 reserves must yield 8000 unique IDs");
    }
}
