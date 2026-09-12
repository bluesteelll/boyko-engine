//! [`EntityReservoir`] — the worker-reachable source of entity ids (EM2′).
//!
//! # Why it exists
//!
//! Before EM2′ the recycled-id free list was dispatcher-only (Phase 11 EM2)
//! while `Commands::spawn` minted every id on a worker through a bare
//! `fetch_add` on the fresh counter. Despawns applied through `Commands`
//! pushed their ids onto a list nothing on the deferred route ever popped, so a
//! flat population churned through `Commands` grew the free list AND the
//! `entities_inland` slot store by one entry per despawn, forever. The fix
//! cannot live in `CommandQueue::apply`: `.id()` has already handed the id to
//! user code when `Commands::spawn` returns, so recycling has to happen at
//! reserve time, on the worker.
//!
//! # The protocol (plan D1, D3, D4)
//!
//! The free list IS a claimable LIFO stack, `free: VmColumn<Entity>`, whose
//! logical length is `free_top`:
//!
//! * **Workers** claim with `r = free_top.fetch_sub(1)`: `r > 0` hands out
//!   `free[r - 1]`, otherwise they mint a fresh id with `fetch_add` on
//!   `next_entity_id`. Within a phase the stack is pop-only and its entries
//!   are immutable, so `fetch_sub` returns each positive value at most once —
//!   no double issue and no ABA.
//! * **The dispatcher** changes the stack's structure only under `&mut`
//!   (SCH7): every `&mut` method that touches it first [`settle`]s — clamps a
//!   `free_top` that claims drove negative back to 0 and truncates the claimed
//!   entries off the physical top — so no apply site needs an end-of-window
//!   hook.
//!
//! Entries carry their generation (plan D2), so a claim never reads
//! `entities_inland` (EM3 kept word for word).
//!
//! # Invariants
//!
//! * **F1** — after [`settle`], `free_top == free.len()`.
//! * **F2** — always `free_top <= free.len()`; in a phase `free.len()` is
//!   constant and `free_top` only decreases.
//! * **F3** — every `e` in `free[0..top)` names a null inland slot whose
//!   generation is `e.generation()`; all ids are distinct (checked by
//!   `EntityMaster`, which owns the inland store).
//! * **EM2′-K** — no `&mut EntityMaster` operation runs while a system that
//!   may defer is dispatched. Breaking it double-issues ids (`settle`'s plain
//!   store erases a concurrent `fetch_sub`) and races (a push writes an index
//!   a worker may be reading).
//!
//! [`settle`]: EntityReservoir::settle

use core::ops::Range;

// Phase 9.1 lesson C1: loom must drive REAL code. The `cfg(loom)` aliases let
// a loom harness exercise the claim protocol verbatim.
#[cfg(not(loom))]
use core::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
#[cfg(loom)]
use loom::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::system::params::entity_counter::MAX_BATCH_HINT;
use crate::ecs::error::{EcsError, EcsResult};
use crate::ecs::identifiers::primitives::EntityId;
use crate::ecs::memory::vm_column::VmColumn;

/// Worker-reachable id source (EM6′): the fresh counter plus the claimable
/// recycled-entity stack.
///
/// `#[repr(C, align(64))]`: the two RMW'd atomics and the `free.base` a claim
/// reads share line 0 of the reservoir, and the reservoir owns that line —
/// a worker spawn's locked RMW no longer invalidates the `entities_inland`
/// header every `get_component_raw` reads (plan D5). The 64-byte alignment is
/// also what frees bit 0 of a reservoir address for `EntityCounter`'s
/// EXHAUSTED tag (plan D4).
#[repr(C, align(64))]
pub(crate) struct EntityReservoir {
    /// Claimable prefix length of `free`. Workers: `fetch_sub(1, Relaxed)`,
    /// skipped once the claiming counter's EXHAUSTED bit is set. Goes negative
    /// within a phase, one step per `fetch_sub` that found `<= 0`. Dispatcher:
    /// plain `&mut` access, clamped to `>= 0` by [`settle`](Self::settle).
    /// AUTHORITATIVE for the stack's logical length.
    free_top: AtomicIsize,
    /// Fresh-id counter (EM1). Monotone except a Fresh-ticket
    /// `rewind_allocate` under `&mut`.
    next_entity_id: AtomicUsize,
    /// Recycled entities, LIFO, each carrying the generation
    /// `deallocate_entity` wrote. `free.len()` is the PHYSICAL top: equal to
    /// `free_top` at rest, `>= free_top` in a phase. Workers read only
    /// `free.base` and the entries below their claim value; every other field
    /// is dispatcher-only and written only in apply windows.
    free: VmColumn<Entity>,
}

// Layout pins (plan O2). Gated to the native 64-bit build: under Miri
// `VmReservation` carries an extra `Layout` field, and under loom the atomics
// are not 8 bytes. Native: free at +16 .. +88, size 128.
#[cfg(all(not(miri), not(loom), target_pointer_width = "64"))]
const _: () = {
    assert!(core::mem::offset_of!(EntityReservoir, free) == 16);
    assert!(size_of::<EntityReservoir>() == 128);
};
// True on every build: `EntityCounter`'s EXHAUSTED bit relies on it.
const _: () = assert!(align_of::<EntityReservoir>() == 64);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Entity>() == 16);

// SAFETY (Send): the reservoir owns its `VmColumn` (a unique reservation, no
// shared ownership) and two atomics; moving it to another thread moves sole
// ownership of all three. The auto-impl is suppressed only by the `NonNull`s
// inside `VmColumn`, which carry no thread affinity.
unsafe impl Send for EntityReservoir {}

// SAFETY (Sync): `&EntityReservoir` exposes exactly (a) atomic RMWs / loads on
// `free_top` and `next_entity_id`, data-race-free from any thread, and (b) plain
// reads of `free.base`, `free.len` and the entries below a claim value. Every
// plain write to those — push, truncate, clear, sort, unpop — takes
// `&mut self`, reachable only on the dispatcher inside an apply window (SCH7),
// which EM2′-K keeps disjoint from every phase in which a claim can run. The
// happens-before between a window's writes and the next phase's reads is the
// thread pool's task publication; between a phase's claims and the next
// window, the completion Release / dispatcher Acquire (`schedule.rs`, the
// apply-window gate).
unsafe impl Sync for EntityReservoir {}

impl EntityReservoir {
    /// Label naming the stack in `VmColumn` exhaustion / bounds panics.
    const FREE_LABEL: &'static str = "EntityMaster.free";

    /// A reservoir whose recycled stack can hold `free_reserve_elems` entries.
    ///
    /// Callers pass the inland store's slot ceiling (`InlandStore::ceiling_slots`):
    /// free entries are distinct ids, each below the inland length, so the
    /// stack can never outgrow it (plan D6). The reservation is lazy — no
    /// syscall until the first despawn.
    pub(crate) fn new(free_reserve_elems: usize) -> Self {
        Self {
            free_top: AtomicIsize::new(0),
            next_entity_id: AtomicUsize::new(0),
            free: VmColumn::new(Self::FREE_LABEL, free_reserve_elems),
        }
    }

    /// Commits room for `n` recycled entries up front (`with_capacity`).
    #[cold]
    pub(crate) fn precommit(&mut self, n: usize) {
        if n > 0 {
            self.free.precommit(n);
        }
    }

    // ── Worker-safe (`&self`) ────────────────────────────────────────────

    /// Claims one recycled entity, or `None` if the stack is (or has just been
    /// driven) empty. One locked RMW; on success one plain read of an entry.
    ///
    /// # Atomic ordering
    ///
    /// `Relaxed`: uniqueness comes from the RMW's single modification order;
    /// visibility of the entry comes from SCH7's edges (the window that wrote
    /// it happens-before the task publication that started this phase).
    #[inline]
    pub(crate) fn try_claim_recycled(&self) -> Option<Entity> {
        let r = self.free_top.fetch_sub(1, Ordering::Relaxed);
        if r > 0 {
            let idx = (r - 1) as usize;
            debug_assert!(
                idx < self.free.len(),
                "F2 violated: claim index {idx} >= physical top {}",
                self.free.len()
            );
            // SAFETY: `0 <= idx = r - 1 < r <= free_top at phase start <=
            //   free.len()` (F2), so the slot lies in `[0, len)`: committed,
            //   `Entity`-aligned, and initialized by a `push` in an earlier
            //   apply window. That window happens-before this read (SCH7: its
            //   writes precede the task publication that started this phase),
            //   and no writer can run until the phase ends (EM2′-K), so the
            //   entry is immutable while we read it. `fetch_sub` returns each
            //   positive value at most once while `free_top` only decreases, so
            //   no other claimant reads `idx` as ITS entry. `Entity: Copy`.
            Some(unsafe { self.free.as_ptr().add(idx).read() })
        } else {
            None
        }
    }

    /// Mints one fresh id at generation 0 (EM1′). One locked RMW.
    #[inline]
    pub(crate) fn mint_fresh(&self) -> Entity {
        let id = self.next_entity_id.fetch_add(1, Ordering::Relaxed);
        debug_assert!(id < usize::MAX / 2, "EntityId counter near exhaustion");
        Entity::new(EntityId(id), 0)
    }

    /// The UNGATED claim: a recycled entity if one is claimable, otherwise a
    /// fresh one. Used where each claim stands alone (hooks'
    /// `DeferredCommands::spawn`, `EntityMaster::reserve_entity`); a
    /// `Commands` system goes through `EntityCounter`'s gated claim instead.
    #[inline]
    pub(crate) fn claim(&self) -> Entity {
        match self.try_claim_recycled() {
            Some(e) => e,
            None => self.mint_fresh(),
        }
    }

    /// `true` iff nothing is claimable right now — the one plain load
    /// `EntityCounter` pays at creation to preset its EXHAUSTED bit (plan D4).
    ///
    /// `Relaxed` is exact here, not a hint: the window's settle happens-before
    /// the task publication that happens-before this load, so coherence rules
    /// out reading an older value, and nothing can push until the phase ends
    /// (X1) — a `<= 0` read stays true for the counter's whole life.
    #[inline]
    pub(crate) fn is_exhausted_hint(&self) -> bool {
        self.free_top.load(Ordering::Relaxed) <= 0
    }

    /// Mints a contiguous range of `n` fresh ids (the `spawn_batch` route,
    /// fresh-and-contiguous in Stage A — plan D7). Validates `n <=
    /// MAX_BATCH_HINT` BEFORE any atomic, so on `Err` the counter is not
    /// advanced (SBO17).
    #[inline]
    pub(crate) fn mint_fresh_batch(&self, n: usize) -> EcsResult<Range<usize>> {
        if n > MAX_BATCH_HINT {
            return Err(EcsError::SpawnBatchExceedsCapacity {
                requested: n,
                max: MAX_BATCH_HINT,
            });
        }
        let start = self.next_entity_id.fetch_add(n, Ordering::Relaxed);
        debug_assert!(
            start.checked_add(n).is_some_and(|end| end < usize::MAX / 2),
            "EntityId counter near exhaustion"
        );
        Ok(start..(start + n))
    }

    /// Entries claimable right now: `max(free_top, 0)`. Equals the stack
    /// length at rest.
    #[inline]
    pub(crate) fn claimable(&self) -> usize {
        self.free_top.load(Ordering::Relaxed).max(0) as usize
    }

    /// The next fresh id the counter would mint (observational, `Relaxed`).
    #[inline]
    pub(crate) fn next_fresh(&self) -> usize {
        self.next_entity_id.load(Ordering::Relaxed)
    }

    /// Raw `free_top`, negative drift included (test / loom probe). Its drift
    /// below 0 counts exactly the `fetch_sub`s that found the stack empty since
    /// the last settle, which is how the EXHAUSTED bit's RMW saving is pinned
    /// as an integer rather than a timing.
    #[inline]
    pub(crate) fn free_top_raw(&self) -> isize {
        self.free_top.load(Ordering::Relaxed)
    }

    /// Resident bytes of the recycled stack (its commit frontier).
    #[inline]
    pub(crate) fn free_committed_bytes(&self) -> usize {
        self.free.committed_elems() * size_of::<Entity>()
    }

    // ── Dispatcher-only (`&mut self`) ────────────────────────────────────

    #[inline]
    fn top_mut(&mut self) -> isize {
        #[cfg(not(loom))]
        {
            *self.free_top.get_mut()
        }
        #[cfg(loom)]
        {
            self.free_top.with_mut(|v| *v)
        }
    }

    #[inline]
    fn set_top_mut(&mut self, top: isize) {
        #[cfg(not(loom))]
        {
            *self.free_top.get_mut() = top;
        }
        #[cfg(loom)]
        {
            self.free_top.with_mut(|v| *v = top);
        }
    }

    /// The fresh counter under `&mut` (a plain read).
    #[inline]
    pub(crate) fn next_fresh_mut(&mut self) -> usize {
        #[cfg(not(loom))]
        {
            *self.next_entity_id.get_mut()
        }
        #[cfg(loom)]
        {
            self.next_entity_id.with_mut(|v| *v)
        }
    }

    /// Rolls the fresh counter back by one under `&mut` (the Fresh-ticket
    /// rewind, plan D9). The caller has proven `next_fresh_mut() > 0`.
    #[inline]
    pub(crate) fn unmint_fresh_mut(&mut self) {
        #[cfg(not(loom))]
        {
            *self.next_entity_id.get_mut() -= 1;
        }
        #[cfg(loom)]
        {
            self.next_entity_id.with_mut(|v| *v -= 1);
        }
    }

    /// Absorbs the previous phase's claims: `top = max(free_top, 0)`, drop the
    /// claimed entries off the physical top, store `top` back (plan D3).
    /// Branch-free; every `&mut` method that touches the stack calls it first.
    /// Returns the settled length.
    ///
    /// Overwriting the claimed entries later is harmless: each was copied into
    /// its handle at claim time.
    #[inline]
    pub(crate) fn settle(&mut self) -> usize {
        let top = self.top_mut().max(0) as usize;
        debug_assert!(
            top <= self.free.len(),
            "F2 violated: free_top {top} above the physical top {}",
            self.free.len()
        );
        self.free.truncate(top);
        self.set_top_mut(top as isize);
        top
    }

    /// Pushes a recycled entity (after `deallocate_entity` bumped its
    /// generation). Plain instructions — no `lock` prefix.
    #[inline]
    pub(crate) fn push_free(&mut self, e: Entity) {
        let top = self.settle();
        self.free.push(e);
        self.set_top_mut(top as isize + 1);
    }

    /// Pops the most recently recycled entity (the dispatcher's
    /// `allocate_entity`), or `None` if nothing is left once the previous
    /// phase's claims are absorbed.
    #[inline]
    pub(crate) fn pop_free(&mut self) -> Option<Entity> {
        let top = self.settle();
        if top == 0 {
            return None;
        }
        let e = self.free.get(top - 1);
        self.free.truncate(top - 1);
        self.set_top_mut(top as isize - 1);
        e
    }

    /// Puts back an entity `pop_free` handed out (the Recycled-ticket rewind,
    /// plan D9). The caller has checked the slot is null and its generation
    /// matches, so the entry is exactly what the pop removed.
    #[cold]
    pub(crate) fn unpop(&mut self, e: Entity) {
        self.push_free(e);
    }

    /// World reset: empties the stack (keeping its commit frontier) and resets
    /// the fresh counter.
    pub(crate) fn clear(&mut self) {
        self.free.clear();
        self.set_top_mut(0);
        #[cfg(not(loom))]
        {
            *self.next_entity_id.get_mut() = 0;
        }
        #[cfg(loom)]
        {
            self.next_entity_id.with_mut(|v| *v = 0);
        }
    }

    /// `EntityMaster::compact`: orders the stack so the LOWEST id sits on top
    /// (it is popped / claimed first). In place, no allocation, no shrink.
    pub(crate) fn sort_free_low_ids_first(&mut self) {
        self.settle();
        self.free
            .as_mut_slice()
            .sort_unstable_by_key(|e| core::cmp::Reverse(e.id()));
    }

    /// The settled stack, bottom to top (the F1–F3 walk of
    /// `EntityMaster::check_invariants`).
    pub(crate) fn settled_entries(&mut self) -> &[Entity] {
        let top = self.settle();
        &self.free.as_slice()[..top]
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    // Test-only oracle state (std threads, `Vec` result buffers, a sort) — the
    // reference the reservoir is checked against, never engine data.
    #![allow(clippy::disallowed_types)]

    use super::*;
    use crate::ecs::core::system::params::entity_counter::EntityCounter;

    fn ent(id: usize, generation: u32) -> Entity {
        Entity::new(EntityId(id), generation)
    }

    /// A reservoir that minted ids `0..n` and then recycled them in order, so
    /// the stack holds `0..n` at generation 1 (id `n - 1` on top) and the
    /// fresh counter stands at `n` — the state a world reaches after spawning
    /// and despawning `n` entities.
    fn with_stack(n: usize) -> EntityReservoir {
        let mut r = EntityReservoir::new(n + 64);
        for _ in 0..n {
            let _ = r.mint_fresh();
        }
        for id in 0..n {
            r.push_free(ent(id, 1));
        }
        r
    }

    #[test]
    fn claim_takes_the_stack_lifo_then_mints_fresh() {
        let r = with_stack(3);
        let got: Vec<Entity> = (0..5).map(|_| r.claim()).collect();
        assert_eq!(
            got,
            vec![ent(2, 1), ent(1, 1), ent(0, 1), ent(3, 0), ent(4, 0)],
            "recycled entries come off the top first, then fresh ids at generation 0"
        );
    }

    #[test]
    fn failed_claims_drive_free_top_negative_by_exactly_their_count() {
        let r = with_stack(2);
        for _ in 0..5 {
            let _ = r.try_claim_recycled();
        }
        assert_eq!(r.free_top_raw(), -3, "two claims succeed, three find the stack empty");
    }

    #[test]
    fn settle_clamps_drift_and_drops_the_claimed_entries() {
        let mut r = with_stack(2);
        for _ in 0..5 {
            let _ = r.try_claim_recycled();
        }
        assert_eq!(r.settle(), 0, "settled length");
        assert_eq!((r.free_top_raw(), r.free.len()), (0, 0), "F1: free_top == free.len() after settle");
    }

    #[test]
    fn settle_keeps_the_unclaimed_prefix() {
        let mut r = with_stack(4);
        assert_eq!(r.try_claim_recycled(), Some(ent(3, 1)), "fixture: one claim");
        assert_eq!(r.settled_entries(), &[ent(0, 1), ent(1, 1), ent(2, 1)], "the claimed top is gone, the rest is intact");
    }

    /// RED if `push_free` stops settling first: the push would land above the
    /// claimed entries and `free_top + 1` would expose an entry that was
    /// already handed out (double issue).
    #[test]
    fn push_after_a_drained_phase_exposes_only_the_new_entry() {
        let mut r = with_stack(2);
        for _ in 0..3 {
            let _ = r.try_claim_recycled();
        }
        r.push_free(ent(9, 2));
        assert_eq!(r.claimable(), 1, "only the entry pushed after the phase is claimable");
    }

    #[test]
    fn pop_free_pops_below_what_the_phase_claimed() {
        let mut r = with_stack(3);
        assert_eq!(r.try_claim_recycled(), Some(ent(2, 1)), "fixture: one claim");
        assert_eq!(r.pop_free(), Some(ent(1, 1)), "the dispatcher never re-issues a claimed entry");
    }

    #[test]
    fn pop_free_on_a_drained_stack_returns_none_and_settles() {
        let mut r = with_stack(1);
        for _ in 0..2 {
            let _ = r.try_claim_recycled();
        }
        assert_eq!((r.pop_free(), r.free_top_raw()), (None, 0), "None, and the drift is absorbed");
    }

    #[test]
    fn unpop_restores_the_popped_entry_on_top() {
        let mut r = with_stack(2);
        let e = r.pop_free().expect("fixture: stack holds 2");
        r.unpop(e);
        assert_eq!(r.claim(), ent(1, 1), "the rewound entry is the next one claimed");
    }

    #[test]
    fn sort_free_low_ids_first_puts_the_lowest_id_on_top() {
        let mut r = EntityReservoir::new(64);
        for id in [5, 1, 9, 3] {
            r.push_free(ent(id, 1));
        }
        r.sort_free_low_ids_first();
        let got: Vec<usize> = (0..4).map(|_| r.claim().id().0).collect();
        assert_eq!(got, vec![1, 3, 5, 9], "compact reuses the lowest ids first");
    }

    #[test]
    fn clear_empties_the_stack_and_resets_the_fresh_counter() {
        let mut r = with_stack(2);
        r.clear();
        assert_eq!((r.claimable(), r.claim()), (0, ent(0, 0)), "an empty stack and a counter back at 0");
    }

    #[test]
    fn mint_fresh_batch_over_the_cap_errs_without_advancing() {
        let r = EntityReservoir::new(64);
        assert!(r.mint_fresh_batch(MAX_BATCH_HINT + 1).is_err(), "over-cap batch must be refused");
        assert_eq!(r.next_fresh(), 0, "SBO17: a refused batch does not advance the counter");
    }

    #[test]
    fn mint_fresh_batch_never_touches_the_stack() {
        let r = with_stack(4);
        let range = r.mint_fresh_batch(8).expect("within cap");
        assert_eq!((range, r.claimable()), (4..12, 4), "batches are fresh and contiguous (plan D7)");
    }

    // ── `EntityCounter`'s EXHAUSTED bit, pinned as integers ──────────────

    /// RED if `from_ptr` stops presetting the bit: the first claim pays a
    /// failing `fetch_sub` (drift -1, measured) before the in-claim set stops
    /// the rest.
    #[test]
    fn counter_over_an_empty_stack_never_fetch_subs() {
        let r = EntityReservoir::new(64);
        // SAFETY: `r` outlives the counter and no `&mut` to it is formed while
        // the counter lives.
        let c = unsafe { EntityCounter::from_ptr(&r) };
        for _ in 0..100 {
            let _ = c.reserve_entity();
        }
        assert_eq!(r.free_top_raw(), 0, "the preset EXHAUSTED bit suppresses every fetch_sub");
    }

    /// RED if `reserve_entity` stops setting the bit on the first failing
    /// claim (drift -97 instead of -1).
    #[test]
    fn counter_that_runs_the_stack_dry_pays_exactly_one_failing_fetch_sub() {
        let r = with_stack(3);
        // SAFETY: as above.
        let c = unsafe { EntityCounter::from_ptr(&r) };
        for _ in 0..100 {
            let _ = c.reserve_entity();
        }
        assert_eq!(r.free_top_raw(), -1, "only the claim that ran the stack dry fails");
    }

    // ── The claim race: several counters, one populated stack ────────────

    /// `threads` counters claim `per_thread` each, concurrently, from a stack
    /// of `stack` recycled ids. Checks exact uniqueness, the exact id set, the
    /// generations, the EXHAUSTED-bounded drift and the settle that follows.
    ///
    /// Every claimer waits at a start rendezvous so the claim loops overlap:
    /// without it each thread can finish before the next is scheduled, and a
    /// lost-update `fetch_sub` (load + store) measured GREEN here while the
    /// scheduler-level twin caught it. The rendezvous spins with `yield_now`,
    /// which is also what lets Miri (`-Zmiri-preemption-rate=0`) switch
    /// threads instead of spinning forever.
    fn claim_race(stack: usize, threads: usize, per_thread: usize) {
        use core::sync::atomic::AtomicUsize as StdAtomicUsize;

        let mut r = with_stack(stack);
        let arrived = StdAtomicUsize::new(0);
        let all: Vec<Entity> = std::thread::scope(|s| {
            let rr = &r;
            let arrived = &arrived;
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    s.spawn(move || {
                        // SAFETY: `r` outlives the scope, and no `&mut r` is
                        // formed until every claimer has joined — the
                        // phase / apply-window disjointness EM2′-K demands.
                        let c = unsafe { EntityCounter::from_ptr(rr) };
                        let mut out = Vec::with_capacity(per_thread);
                        arrived.fetch_add(1, Ordering::AcqRel);
                        while arrived.load(Ordering::Acquire) < threads {
                            std::thread::yield_now();
                        }
                        for _ in 0..per_thread {
                            out.push(c.reserve_entity());
                        }
                        out
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().expect("claimer must not panic")).collect()
        });

        let total = threads * per_thread;
        let mut ids: Vec<usize> = all.iter().map(|e| e.id().0).collect();
        ids.sort_unstable();
        let want: Vec<usize> = if total <= stack { (stack - total..stack).collect() } else { (0..total).collect() };
        assert_eq!(ids, want, "every claimed id is distinct and the set is exactly the top of the stack plus the next fresh ids");
        for e in &all {
            let want_gen = u32::from(e.id().0 < stack);
            assert_eq!(e.generation(), want_gen, "{e:?}: recycled ids carry generation 1, fresh ids 0");
        }
        let drift = r.free_top_raw();
        if total <= stack {
            assert_eq!(drift, (stack - total) as isize, "no claim ran dry, so free_top is exact");
        } else {
            assert!(
                (-(threads as isize)..=0).contains(&drift),
                "free_top drifted to {drift}: at most one failing fetch_sub per counter (EXHAUSTED)"
            );
        }
        let left = stack.saturating_sub(total);
        assert_eq!(r.settle(), left, "settle absorbs the phase");
        assert_eq!(r.free.len(), left, "F1 after the race");
    }

    /// Native: 8 rounds, each a fresh reservoir — an interleaving that exposes
    /// a lost update is likely in one round, not guaranteed in every one.
    #[test]
    fn claim_race_that_runs_the_stack_dry_issues_each_id_once() {
        if cfg!(miri) {
            claim_race(16, 3, 8);
        } else {
            for _ in 0..8 {
                claim_race(4096, 8, 1024);
            }
        }
    }

    #[test]
    fn claim_race_that_leaves_the_stack_populated_takes_exactly_its_top() {
        if cfg!(miri) {
            claim_race(32, 3, 8);
        } else {
            for _ in 0..8 {
                claim_race(8192, 8, 512);
            }
        }
    }
}
