//! `EntityCounter<'s>` — the worker's projection of the world's
//! [`EntityReservoir`]: the fresh-id counter plus the claimable recycled-entity
//! stack.
//!
//! Phase 11 Round 3 (C-N1, EM6), re-based by EM2′. The newtype carries a raw
//! pointer to `EntityMaster::reservoir` so that worker code holding a
//! [`Commands<'s>`](super::commands::Commands) can reserve Entity IDs without
//! exposing the full `&EntityMaster`. The pointer's destination type
//! (`*const EntityReservoir`) makes the EM6′ field-restriction invariant
//! **type-enforced**: there is no compile-time path from an `EntityCounter` to
//! `entities_inland`, `live_count`, or any other `EntityMaster` field.
//!
//! # What a claim does (EM1′, EM2′, EM4′)
//!
//! [`EntityCounter::reserve_entity`] takes the most recently recycled entity —
//! with the generation `deallocate_entity` bumped — if one is claimable
//! (`free_top.fetch_sub(1)` returned `> 0`), and otherwise mints a fresh id at
//! generation 0 (`next_entity_id.fetch_add(1)`). The returned handle stays valid
//! for its owner: every id handed out is in exactly one of {recycled stack,
//! live, claimed-pending, leaked} (EM4′), and the recycled stack's structure
//! changes only on the dispatcher under `&mut` (EM2′).
//!
//! # The EXHAUSTED bit (plan D4, invariant X1)
//!
//! Bit 0 of the carried reservoir address (free: the reservoir is
//! `align(64)`) records "this counter has seen the stack empty". It is preset at
//! creation from one `Relaxed` load and set by the first `fetch_sub` that finds
//! `<= 0`; once set, claims go straight to the fresh counter. The bit is always
//! TRUE: nothing can push onto the stack while a counter lives (a push needs
//! `&mut EntityMaster`, which SCH7 keeps out of every phase), so `free_top`
//! only decreases during the counter's lifetime. A wrong bit could only cost a
//! missed recycle, never a double issue — it only ever suppresses a
//! `fetch_sub`. The saving: a steady-state claim is ONE locked RMW on either
//! path, and only the one claim per system call that runs the stack dry pays
//! two.
//!
//! # Soundness contract (`EntityCounter::from_ptr` SAFETY)
//!
//! The `from_ptr` constructor is `unsafe`; callers (limited to
//! [`crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::entity_counter`])
//! must guarantee:
//!
//! 1. The pointer is valid for the lifetime `'s` (provenance + non-dangling).
//! 2. It points at `EntityMaster::reservoir` of an `EcsMaster` whose lifetime
//!    contains `'s`.
//! 3. For the whole of `'s` nothing mutates the `EntityMaster` except through
//!    the reservoir's atomics (SCH7 / EM2′-K): no `&mut EntityMaster` is formed
//!    or used while the counter lives. A `&mut EcsMaster` the pointer was itself
//!    derived from — `run_system` called inside a `Command::apply` — satisfies
//!    this as long as it is not used again until the counter is dropped, which
//!    the nested system's body scope guarantees.
//!
//! # Send / !Sync
//!
//! `EntityCounter<'s>: Send` via an explicit impl; it is `!Sync` (and `!Copy`)
//! because the EXHAUSTED bit lives in a `Cell`. Nothing needs more: the
//! counter's only owner is its `Commands<'s>`, which is `!Sync` for an
//! independent reason (`&mut CommandQueue`, CQ-SEND2).

// `EntityCounter` is consumed exclusively by `Commands<'s>` (Wave B) and the
// `UnsafeEcsCell::entity_counter` projection. The lib build does not exercise
// the path until Wave C wires the `Commands::spawn` return path; mirror the
// pattern of `commands.rs` / `command_queue.rs` and suppress dead_code until
// the first public consumer lands.
#![allow(dead_code)]

use core::cell::Cell;
use core::marker::PhantomData;
use core::ops::Range;

use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_reservoir::EntityReservoir;
use crate::ecs::error::EcsResult;

/// Phase 12.5 (SBO17): the maximum number of entities one `spawn_batch` call
/// may reserve. Hard cap; over-budget batches return `Err` without advancing
/// the counter. See `ecs_master.rs` for the world-side capacity contract.
pub(crate) const MAX_BATCH_HINT: usize = 8_192;

/// The EXHAUSTED tag in bit 0 of the carried reservoir address (plan D4).
const EXHAUSTED: usize = 1;

// The tag bit is free only because the reservoir is over-aligned.
const _: () = assert!(align_of::<EntityReservoir>() > EXHAUSTED);

/// Minimal projection of [`crate::ecs::core::entity::entity_master::EntityMaster`]
/// exposing only the [`EntityReservoir`] — the fresh counter and the claimable
/// recycled stack — for entity reservation from system bodies.
///
/// # Layout (plan §11.10)
///
/// ```text
/// +0  : tagged: Cell<*const EntityReservoir>  (8 B; bit 0 = EXHAUSTED)
/// +8  : _marker: PhantomData                  (0 B ZST)
/// +8  : end
/// ```
///
/// Size 8 B; align 8 (`Cell` is `repr(transparent)`), so `Commands<'s>` stays
/// 16 B.
///
/// # Lifetime (`'s`)
///
/// `'s` is the system's state scope (the lifetime of the per-system
/// `CommandQueue` slot that the parent [`Commands<'s>`](super::commands::Commands)
/// borrows). The pointer is minted from `UnsafeEcsCell<'w>` once per
/// system invocation (plan §8.7 — `'w >= 's`) and re-tagged to `'s` via
/// `PhantomData`.
pub struct EntityCounter<'s> {
    /// Pointer to `EntityMaster::reservoir`, bit 0 = EXHAUSTED (X1).
    ///
    /// Minted from `UnsafeEcsCell<'w>` inside `Commands::get_param`. The tag is
    /// set and cleared with `map_addr`, which keeps the pointer's provenance.
    tagged: Cell<*const EntityReservoir>,

    /// Variance marker — ties the pointer's apparent validity to `'s`.
    _marker: PhantomData<&'s EntityReservoir>,
}

// SAFETY (EC1, EM5, EM6′, plan §5.5 / D4):
//   Moving an `EntityCounter` to another thread moves the `Cell` with it; the
//   EXHAUSTED bit is state the counter alone owns, never shared. Through the
//   carried pointer a counter performs only (a) atomic RMWs and one `Relaxed`
//   load on the reservoir's `free_top` / `next_entity_id`, data-race-free from
//   any thread, and (b) plain reads of `free.base` and of entries below its own
//   claim value. Those plain reads are race-free by SCH7 (+ EM2′-K), NOT by
//   the pointer type: every write to them takes `&mut EntityMaster`, which
//   runs only in an apply window, and no counter outlives the phase it was
//   minted in (`Commands<'s>` is dropped at system-body end).
unsafe impl<'s> Send for EntityCounter<'s> {}

// Compile-time size + align contract (plan §11.10 — 8 B).
// The 8-byte figures encode the 64-bit ABI. Gated to 64-bit (the engine's
// supported platform) — see CLAUDE.md target platform.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<EntityCounter<'static>>() == 8);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::align_of::<EntityCounter<'static>>() == 8);

impl<'s> EntityCounter<'s> {
    /// Constructs an `EntityCounter` over the world's reservoir, presetting the
    /// EXHAUSTED bit from one `Relaxed` load (a stack that is empty now stays
    /// empty for the counter's whole life — X1).
    ///
    /// # Safety (plan §5.5)
    ///
    /// * `ptr` must be valid for reads for the entirety of `'s`, derived with
    ///   provenance over the whole `EntityReservoir`.
    /// * The pointee must be the `reservoir` field of an `EntityMaster` whose
    ///   lifetime contains `'s`.
    /// * Nothing may mutate the `EntityMaster` during `'s` except through the
    ///   reservoir's atomics — no `&mut EntityMaster` is formed or used while
    ///   the counter lives (SCH7 / EM2′-K): the counter reads recycled entries
    ///   that only a `&mut` writer could change. (A parent `&mut EcsMaster`
    ///   the pointer was derived from may exist, unused, for the nested
    ///   `run_system`-in-apply case — module doc, contract 3.)
    #[inline]
    pub(crate) unsafe fn from_ptr(ptr: *const EntityReservoir) -> Self {
        debug_assert_eq!(ptr.addr() & EXHAUSTED, 0, "EntityReservoir pointer not 64-aligned");
        // SAFETY: the caller guarantees `ptr` is valid for reads for `'s` and
        //   that no `&mut EntityMaster` is used while the counter lives, so the
        //   `&EntityReservoir` formed for this call conflicts with no write;
        //   the load itself is atomic.
        let exhausted = unsafe { (*ptr).is_exhausted_hint() };
        let tagged = ptr.map_addr(|a| a | (exhausted as usize));
        Self {
            tagged: Cell::new(tagged),
            _marker: PhantomData,
        }
    }

    /// The untagged reservoir pointer and whether EXHAUSTED is set.
    #[inline]
    fn split(&self) -> (*const EntityReservoir, bool) {
        let tagged = self.tagged.get();
        (tagged.map_addr(|a| a & !EXHAUSTED), tagged.addr() & EXHAUSTED != 0)
    }

    /// Reserves an Entity — the most recently recycled one if the stack still
    /// holds any, otherwise a fresh id at generation 0 — lock-free.
    ///
    /// The caller enqueues a [`crate::ecs::core::commands::Command`]
    /// (typically `SpawnAtCommand`) that registers the handle into the fast
    /// store on apply (EM3); the handle is invalid until then, exactly like a
    /// fresh reserved id.
    ///
    /// # Cost
    ///
    /// One locked RMW on either path in steady state (`fetch_sub` while the
    /// stack holds entries, `fetch_add` once this counter's EXHAUSTED bit is
    /// set), plus one plain read of the entry on the recycled path. Only the
    /// one claim that runs the stack dry pays both RMWs.
    ///
    /// # Atomic ordering
    ///
    /// `Ordering::Relaxed` is sufficient — uniqueness comes from the RMWs;
    /// happens-before for the handle and for the entry it was read from is
    /// SCH7's (the window that pushed the entry precedes this phase's task
    /// publication; this phase's completion precedes the next window).
    #[inline]
    pub fn reserve_entity(&self) -> Entity {
        let (ptr, exhausted) = self.split();
        // SAFETY (EM5, EM6′, plan §5.5): `ptr` was minted by
        //   `UnsafeEcsCell::entity_counter` from a live `EntityMaster::reservoir`
        //   projection and is valid for `'w >= 's` (Phase 8c IntoSystem
        //   contract: `get_param` runs once per system invocation and
        //   `Commands<'s>` is dropped at body end). No `&mut EntityMaster` is
        //   used during `'s` (from_ptr contract 3), so the shared reborrow
        //   conflicts with no write: the stack entries below this counter's
        //   claim value and `free.base` are immutable for the phase, and the
        //   atomics are only ever accessed atomically.
        let reservoir = unsafe { &*ptr };
        if !exhausted {
            if let Some(e) = reservoir.try_claim_recycled() {
                return e;
            }
            self.tagged.set(ptr.map_addr(|a| a | EXHAUSTED));
        }
        reservoir.mint_fresh()
    }

    /// Phase 12.5 Opt-A2 (SBO17 / plan §5.3): atomically reserves a
    /// contiguous range of `n` FRESH entity IDs (batches stay fresh and
    /// contiguous in Stage A — EM2′ plan D7).
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
    /// # Atomic ordering
    ///
    /// `Ordering::Relaxed` — same rationale as [`Self::reserve_entity`].
    #[inline]
    pub fn reserve_batch(&self, n: usize) -> EcsResult<Range<usize>> {
        let (ptr, _) = self.split();
        // SAFETY (EM5, EM6′, plan §5.5): same contract as `reserve_entity`;
        //   the batch path touches only the reservoir's fresh-id atomic.
        unsafe { (*ptr).mint_fresh_batch(n) }
    }
}

#[cfg(test)]
mod tests {
    // Test-only harness state (shared counters / observation channels behind a
    // std lock); the reference model the param's atomic behaviour is checked
    // against. Compiled out of every shipping build.
    #![allow(clippy::disallowed_types)]

    use super::*;

    /// `EntityCounter` carries exactly a single tagged pointer — 8 B, align 8.
    /// Mirrors the plan §11.10 layout contract and the module-level
    /// `const _: () = assert!(...)` guards.
    #[test]
    fn entity_counter_size_is_8_bytes() {
        assert_eq!(core::mem::size_of::<EntityCounter<'_>>(), 8);
        assert_eq!(core::mem::align_of::<EntityCounter<'_>>(), 8);
    }

    /// `EntityCounter<'static>` is `Send` (and, by the `Cell`, not `Sync`).
    /// Compile-time gate; the unsafe impl above is the load-bearing
    /// declaration.
    #[test]
    fn entity_counter_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<EntityCounter<'static>>();
    }

    /// Repeated reserves on an empty stack yield strictly distinct fresh IDs
    /// at generation 0 (EM1′, EM4′).
    #[test]
    fn entity_counter_reserve_distinct_ids() {
        let reservoir = EntityReservoir::new(64);
        // SAFETY: `reservoir` lives for the entirety of this test and no
        // `&mut` to it exists while the counter is alive.
        let counter = unsafe { EntityCounter::from_ptr(&reservoir) };
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1024 {
            let e = counter.reserve_entity();
            assert_eq!(e.generation(), 0, "fresh reserves carry generation 0 (EM1)");
            assert!(seen.insert(e.id().0), "reserved ID {} repeated", e.id().0);
        }
        assert_eq!(seen.len(), 1024);
    }

    /// 8-thread × 1000 reserves on an empty stack = 8000 distinct IDs (EM4
    /// atomic uniqueness, scaled-down version of the loom model). One counter
    /// per thread, as one `Commands` per system.
    #[test]
    fn entity_counter_reserve_lock_free_8_threads() {
        use std::sync::Arc;
        use std::thread;

        let reservoir = Arc::new(EntityReservoir::new(64));
        let mut handles = Vec::with_capacity(8);
        for _ in 0..8 {
            let storage = Arc::clone(&reservoir);
            handles.push(thread::spawn(move || {
                // SAFETY: `storage` keeps the reservoir alive for the
                // thread's lifetime and no `&mut` to it exists anywhere.
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
        assert!(all_ids.iter().all(|&id| id < 8000), "an empty stack mints 0..8000");
    }
}
