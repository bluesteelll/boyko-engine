//! Phase 22.1 Area A — immutable epoch-stamped term-filtered id lists with
//! lock-free CAS publication and mint-point reclamation.
//!
//! This module is the SOUNDNESS-CRITICAL core of the tag-term prefilter. A
//! `TermList` is an immutable, heap-published, epoch-stamped archetype-id
//! list; a `TermScratch` is the two-`AtomicPtr` cold tail of every
//! `QueryDataState<D, F>` that memoises the most recent published list and
//! holds at most one retired list pending reclamation. The no-terms path
//! never touches the scratch.
//!
//! # The protocol — SAFETY contract P1–P4
//!
//! Every `unsafe` block in this module cites one of these clauses. They are
//! the obligations the loom + multi-threaded Miri-TB gates must prove.
//!
//! ## Definitions
//!
//! * *Epoch* of a state slot = the triple
//!   `(live-prefix terms, archetype_generation, structural_generation)`.
//! * *Owner value* = the single live `Query`/`QueryView` minted for the slot.
//!   Verified single-owner: `EcsMaster::query(&mut self)`, `Query` is neither
//!   `Clone` nor `Copy`, `ParQuery{Mut}::for_each(self)` consumes,
//!   `with_tag(mut self)` / `without_tag(mut self)` move the owner.
//!
//! ## P1 — at most one successful publish per epoch
//!
//! All racing resolvers within an epoch loaded the same expected pointer
//! (null or the same stale list); the first successful `compare_exchange`
//! changes `current`, so every other CAS in that epoch fails. Racers cannot
//! span epochs: an in-flight resolve holds a borrow of the owner, and an
//! epoch change requires either `with_tag(mut self)` (owner moved — blocked
//! by any live borrow, enforced cross-thread by `scope` borrow regions) or
//! `&mut EcsMaster` (blocked by the view's world borrow; for systems,
//! structural ops are deferred to the apply window, which the executor's
//! Acquire/Release completion machinery orders after all system borrows end).
//! Corollary: a CAS loser's `winner` pointer is same-epoch ⇒ stamps match ⇒
//! adopt without waiting (**lock-free: losers never spin**).
//!
//! ## P2 — at most one retired list pending per slot; freed only under slot
//! exclusivity
//!
//! One publish per epoch (P1) ⇒ one retire per epoch; every epoch change
//! passes through a mint funnel (a new owner value is required to observe it,
//! and both funnels are exclusive: `&mut state` in `Query::get_param`,
//! `&mut self` in `EcsMaster::query`), and `TermScratch::reclaim_retired`
//! runs there first ⇒ the retired slot is empty when the next retire arrives.
//! The reclaim point cannot overlap an in-flight resolve on the same slot (a
//! system is never dispatched concurrently with itself; a live `QueryView`
//! blocks `query(&mut self)`), so nobody can be reading the old list's header
//! when it is freed. The reclaim swap leaves `null` behind so a hypothetical
//! double-reclaim frees `null` (defense in depth: leak, never double-free;
//! the `debug_assert` pins the impossible case).
//!
//! **Residual proof risk (critic round 2 MAJOR — VERIFIED-SOUND, flagged):**
//! the cross-thread half of P2 rests on two project invariants OUTSIDE this
//! module — (a) a system is never dispatched concurrently with itself, and
//! (b) structural epoch changes are deferred to the apply window ordered
//! after all system borrows end by the Phase 9 completion channel. These are
//! NOT carried by the [ordering table](#memory-orderings) below; the
//! multi-threaded Miri-TB / loom gate 11b ("resolve-stale + later reclaim
//! interleave") MUST model a reader still holding the old pointer at the
//! reclaim point or the central P2 claim ships unproven.
//!
//! ## P3 — publication completeness
//!
//! `TermList::build` returns a complete `Box<TermList>`; the CAS site only
//! ever sees a finished `Box` (type-structural — you cannot publish what the
//! constructor has not returned). `Release` on CAS success pairs with
//! `Acquire` on every load ⇒ readers see fully-initialised contents. A panic
//! mid-build unwinds before the CAS: `current` keeps its old (null/stale)
//! value, the next resolve simply retries. No half-built list is reachable.
//!
//! ## P4 — slice lifetime
//!
//! `TermScratch::resolve_term_filtered` returns `&'q [ArchetypeId]`: valid
//! because (i) within an epoch the published pointer never changes after the
//! single publish (P1), and (ii) freeing requires retire (epoch change —
//! impossible while the owner is borrowed) followed by reclaim (mint-point
//! exclusivity — impossible while `'q` is alive). No ABA: a pointer is freed
//! only at reclaim points where no resolve is in flight, so no racer can hold
//! a stale `expected` across a free/realloc.
//!
//! # Memory orderings
//!
//! | Atomic op | Ordering | Why |
//! |---|---|---|
//! | `current.load` (fast path) | `Acquire` | pairs with publish `Release` ⇒ list contents visible |
//! | `current.compare_exchange` | `Release` (success) / `Acquire` (failure) | success publishes the build; failure returns the winner pointer ready to deref |
//! | `retired.swap` (winner) | `AcqRel` | RMW publishing the ownership transfer to the reclaimer |
//! | `retired.swap(null)` (reclaim) | `Acquire` | pairs with the winner's `Release` half ⇒ safe `Box::from_raw` |
//! | `retired.load` (reclaim fast path) | `Relaxed` | null-check hint only; the swap re-validates |
//!
//! The "last reader of old has finished before free" edge is NOT carried by
//! these atomics — it is carried by the slot-exclusivity of the reclaim point
//! (P2), which rests on the executor's existing synchronization (Phase 9
//! completion channel) and the borrow checker.
//!
//! # Allocation discipline (principle 5)
//!
//! Steady state (stamps match) = zero allocations, one `Acquire` load +
//! ≤ 8 id compares + 2 generation compares per term-bearing driver entry,
//! construction-time, off the row loop. Allocation happens only on epoch
//! change (structural-rare) or term-set change. The no-terms path never loads
//! the scratch.
//!
//! ## O2 thrash model (documented anti-pattern, not a regression)
//!
//! Two distinct term sets alternating on one state slot rebuild once per
//! alternation (one `O(matched)` build + one alloc + one deferred free per
//! alternation) — not a regression versus the shipped per-transition test,
//! but callers should use **one term set per view per frame** to stay on the
//! zero-allocation steady-state path.

// Phase 9.1 lesson C1: loom must drive REAL code. The `cfg(loom)` atomic
// aliases let the tester's loom harness exercise `resolve_term_filtered` /
// `reclaim_retired` verbatim under loom's model checker, while production
// builds use `core::sync::atomic`.
#[cfg(not(loom))]
use core::sync::atomic::{AtomicPtr, Ordering};
#[cfg(loom)]
use loom::sync::atomic::{AtomicPtr, Ordering};

use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::archetype::generation::ArchetypeGeneration;
use crate::ecs::core::iters::query::tag_terms::{TagTerms, archetype_passes_tag_terms};
use crate::ecs::core::iters::query_state::QueryState;
use crate::ecs::identifiers::primitives::ArchetypeId;

/// Immutable, heap-published, epoch-stamped term-filtered id list.
///
/// Built fully while private (P3); NEVER mutated after publication. Shared
/// read-only across any thread holding a borrow of the owner.
pub(crate) struct TermList {
    /// Epoch fingerprint — the live-prefix terms the list was built for.
    stamp_terms: TagTerms,
    /// Epoch fingerprint — the archetype generation observed at build time.
    stamp_arch_gen: ArchetypeGeneration,
    /// Epoch fingerprint — the structural generation observed at build time.
    stamp_struct_gen: ArchetypeGeneration,
    /// Archetype-granular term-filtered matched ids.
    ids: Box<[ArchetypeId]>,
}

impl TermList {
    /// Builds a fresh term-filtered list for the current epoch.
    ///
    /// `O(matched)`: one `get_archetype` slab lookup + ≤ 8 signature bit
    /// tests per pre-terms id, pushing the survivors into a private `Vec`
    /// then `into_boxed_slice`. Stale ids (archetypes removed after the
    /// state's last sync) are excluded at build — `get_archetype` returns
    /// `None` and the id is dropped. The returned `Box<TermList>` is complete
    /// (P3): it carries no public mutation path, and the caller publishes it
    /// atomically as the last step.
    fn build(terms: &TagTerms, master: &ArchetypeMaster, state: &QueryState) -> Box<TermList> {
        let pre = state.matched_ids_pre_terms();
        let mut ids: Vec<ArchetypeId> = Vec::with_capacity(pre.len());
        for &id in pre {
            if let Some(arch) = master.get_archetype(id)
                && archetype_passes_tag_terms(terms, arch)
            {
                ids.push(id);
            }
        }
        Box::new(TermList {
            stamp_terms: *terms,
            stamp_arch_gen: master.archetype_generation(),
            stamp_struct_gen: master.structural_generation(),
            ids: ids.into_boxed_slice(),
        })
    }

    /// `true` when `self`'s epoch fingerprint matches the current `(terms,
    /// master)` epoch — the memo fast-path predicate.
    ///
    /// Live-prefix term equality ([`TagTerms::same`]) plus two generation
    /// compares. Within an epoch the owner is borrowed, so the generations
    /// are frozen (P1); the compares catch a stale memo across owners.
    #[inline]
    fn matches(&self, terms: &TagTerms, master: &ArchetypeMaster) -> bool {
        self.stamp_arch_gen == master.archetype_generation()
            && self.stamp_struct_gen == master.structural_generation()
            && self.stamp_terms.same(terms)
    }
}

/// 16-byte cold tail of `QueryDataState<D, F>`. Auto `Send + Sync` (two
/// `AtomicPtr`); soundness carried by the protocol P1–P4 (module doc).
///
/// `current` memoises the most recently published [`TermList`] (null = never
/// built); `retired` holds at most one superseded list pending reclamation at
/// the next mint funnel.
pub(crate) struct TermScratch {
    /// The published list for the current epoch. `null` = never built. At
    /// most one successful publish per epoch (P1).
    current: AtomicPtr<TermList>,
    /// The superseded list pending reclamation. At most one pending per slot;
    /// freed only under slot exclusivity at a mint funnel (P2).
    retired: AtomicPtr<TermList>,
}

impl Default for TermScratch {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl TermScratch {
    /// A fresh scratch — no list ever built, nothing retired.
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            current: AtomicPtr::new(core::ptr::null_mut()),
            retired: AtomicPtr::new(core::ptr::null_mut()),
        }
    }

    /// Lock-free memoised resolve of the term-filtered id slice for the
    /// current epoch.
    ///
    /// Fast path: one `Acquire` load; if a published list matches the epoch,
    /// return its slice with no allocation, no CAS. Slow path (cold-outlined
    /// [`Self::rebuild_publish`]): build a candidate, publish it via CAS, and
    /// retire the superseded list. Losers free their own candidate and adopt
    /// the winner — no spinning (P1).
    ///
    /// `terms` MUST be non-empty (the no-terms path never calls this).
    ///
    /// # Safety contract (P4)
    ///
    /// The returned slice is valid for `'q` because (i) within the epoch the
    /// published pointer never changes after the single publish (P1) and (ii)
    /// the pointee is freed only at a mint funnel where no resolve is in
    /// flight (P2) — which cannot overlap `'q`.
    #[inline]
    pub(crate) fn resolve_term_filtered<'q>(
        &'q self,
        terms: &TagTerms,
        master: &ArchetypeMaster,
        state: &QueryState,
    ) -> &'q [ArchetypeId] {
        debug_assert!(
            !terms.is_empty(),
            "invariant: resolve_term_filtered is the term-bearing path; the \
             no-terms path must walk matched_ids_pre_terms() directly and \
             never load the scratch",
        );

        // Fast path. Acquire pairs with the publish Release (P3) so the
        // pointee's contents are fully visible once the pointer is non-null.
        let current = self.current.load(Ordering::Acquire);
        if !current.is_null() {
            // SAFETY (P1, P3, P4): `current` is non-null, so a `Release`
            //   publish happened; the Acquire load above synchronises with it
            //   ⇒ the `TermList` is fully initialised. The pointee is freed
            //   only at a slot-exclusive mint funnel where no resolve is in
            //   flight (P2), so it stays live for `'q` (P4). The list is
            //   immutable after publication (P3) — only a shared reborrow is
            //   taken.
            let list = unsafe { &*current };
            if list.matches(terms, master) {
                return &list.ids;
            }
        }

        self.rebuild_publish(terms, master, state, current)
    }

    /// Slow path of [`Self::resolve_term_filtered`]: build a candidate for
    /// the current epoch and publish it via CAS against `expected`.
    ///
    /// On CAS success: the candidate is published; the superseded `expected`
    /// (if non-null) is moved into `retired` for reclamation at the next mint
    /// funnel; the new slice is returned. On CAS failure: a same-epoch racer
    /// published first (P1); the own candidate (never published, sole
    /// ownership) is freed and the winner's slice is adopted without spinning.
    #[cold]
    #[inline(never)]
    fn rebuild_publish<'q>(
        &'q self,
        terms: &TagTerms,
        master: &ArchetypeMaster,
        state: &QueryState,
        expected: *mut TermList,
    ) -> &'q [ArchetypeId] {
        // O1: a stale-but-matching memo would otherwise persist a wrong list
        // until the next generation bump. Any entry point that forgets
        // `state.update(master)` before resolving is a bug — pin it here, on
        // the cold rebuild arm only.
        debug_assert!(
            state.generations_synced(master),
            "invariant: QueryState must be synced against the master (via \
             QueryDataState::update) before a term resolve rebuilds — see O1",
        );

        let candidate: Box<TermList> = TermList::build(terms, master, state);
        let raw: *mut TermList = Box::into_raw(candidate);

        // Publish. Release (success) pairs with the Acquire load on every
        // reader (P3); Acquire (failure) makes the winner pointer ready to
        // deref.
        match self.current.compare_exchange(
            expected,
            raw,
            Ordering::Release,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                // We won the publish (P1). Retire the superseded list, if
                // any, for reclamation at the next mint funnel (P2).
                if !expected.is_null() {
                    // AcqRel: RMW publishing the ownership transfer to the
                    // reclaimer. P2 guarantees `retired` was empty (the prior
                    // retire was reclaimed at the last mint funnel), so the
                    // swap returns null in the steady protocol; the
                    // debug_assert pins that.
                    let prev = self.retired.swap(expected, Ordering::AcqRel);
                    debug_assert!(
                        prev.is_null(),
                        "P2 violation: a retired list was already pending — \
                         reclaim_retired must run at every mint funnel before \
                         the next retire",
                    );
                    // Defense in depth: if the impossible `prev` is non-null
                    // we leak it rather than double-free. (Leak, never UAF.)
                    let _ = prev;
                }

                // SAFETY (P1, P3, P4): we just published `raw`; it is the
                //   current epoch's list, fully initialised by `build`
                //   (P3). It is freed only at a slot-exclusive mint funnel
                //   (P2), so the slice is valid for `'q` (P4). Immutable
                //   after publication — shared reborrow only.
                let list = unsafe { &*raw };
                &list.ids
            }
            Err(winner) => {
                // A same-epoch racer published first (P1). Our candidate was
                // never published — we hold sole ownership — so free it.
                //
                // SAFETY (P1): `raw` came from `Box::into_raw` on the line
                //   above and the CAS failed, so it was never installed in
                //   `current` and no other thread can observe it. We are its
                //   sole owner; reconstructing the Box frees it exactly once.
                unsafe { drop(Box::from_raw(raw)); }

                // Adopt the winner. `winner` is non-null (a successful CAS
                // installed it) and same-epoch (P1) ⇒ stamps match.
                //
                // SAFETY (P1, P3, P4): the CAS failure ordering is Acquire,
                //   pairing with the winner's Release publish (P3) ⇒ the
                //   winner's `TermList` is fully initialised. Same-epoch (P1)
                //   ⇒ it is the current list, freed only at a mint funnel
                //   (P2) ⇒ valid for `'q` (P4). Immutable after publication.
                let list = unsafe { &*winner };
                debug_assert!(
                    list.matches(terms, master),
                    "P1 violation: CAS loser adopted a list whose epoch \
                     stamp does not match — a racer spanned an epoch change",
                );
                &list.ids
            }
        }
    }

    /// Frees the retired list, if any. Called ONLY at slot-exclusive mint
    /// funnels: `Query::get_param` (`&mut state`) and `EcsMaster::query`
    /// (`&mut self`).
    ///
    /// Fast path: one `Relaxed` null-load + a predicted-not-taken branch. The
    /// `Relaxed` load is a hint only; the `Acquire` swap re-validates and
    /// pairs with the winner's `Release` half of the retire transfer (P2).
    ///
    /// # Safety contract (P2)
    ///
    /// The reclaim point is slot-exclusive: no resolve on the same slot can
    /// be in flight (a system is never dispatched concurrently with itself; a
    /// live `QueryView` blocks `query(&mut self)`), so no reader holds the
    /// retired pointer's header when it is freed.
    #[inline]
    pub(crate) fn reclaim_retired(&self) {
        // Relaxed: cheap null-check hint off the row loop. The swap below
        // re-validates with Acquire.
        if self.retired.load(Ordering::Relaxed).is_null() {
            return;
        }
        self.reclaim_retired_slow();
    }

    /// Cold tail of [`Self::reclaim_retired`] — the actual free.
    #[cold]
    #[inline(never)]
    fn reclaim_retired_slow(&self) {
        // Acquire: pairs with the winner's Release half of the retire
        // transfer (`retired.swap(.., AcqRel)`) ⇒ a safe `Box::from_raw`.
        // The swap-to-null leaves an empty slot so a racing/duplicate reclaim
        // frees null (defense in depth, P2).
        let old = self.retired.swap(core::ptr::null_mut(), Ordering::Acquire);
        if old.is_null() {
            return;
        }
        // SAFETY (P2): `old` was moved into `retired` by a winning publish
        //   (`retired.swap(expected, AcqRel)`); the Acquire swap above
        //   transfers ownership to us and leaves null behind. The reclaim
        //   point is slot-exclusive — no resolve is in flight on this slot —
        //   so no reader holds `old`'s header. We are its sole owner;
        //   reconstructing the Box frees it exactly once.
        unsafe { drop(Box::from_raw(old)); }
    }
}

impl Drop for TermScratch {
    fn drop(&mut self) {
        // Exclusive `&mut self` — no concurrent access; plain loads suffice.
        // Frees the bounded leak (`current` + an unreclaimed `retired`) on
        // teardown of the state slot. Every borrowing cursor is bounded by
        // the owner's lifetime (P4), so the slot outlives all readers.
        let current = self.current.load(Ordering::Relaxed);
        if !current.is_null() {
            // SAFETY (P4, Drop exclusivity): `current` was published via
            //   `Box::into_raw` and never freed elsewhere (`reclaim_retired`
            //   only frees `retired`). `&mut self` guarantees no live reader.
            //   Sole ownership ⇒ freed exactly once.
            unsafe { drop(Box::from_raw(current)); }
        }
        let retired = self.retired.load(Ordering::Relaxed);
        if !retired.is_null() {
            // SAFETY (P2, Drop exclusivity): `retired` holds at most one
            //   list moved there by a winning publish and not yet reclaimed.
            //   `&mut self` guarantees no live reader. Sole ownership ⇒ freed
            //   exactly once.
            unsafe { drop(Box::from_raw(retired)); }
        }
    }
}

/// Phase 22.1 gate-11 test surface (test-only; `#[doc(hidden)]`, never part of
/// the published API — same escape-hatch class as
/// `component_registry::register_layout`).
///
/// The loom models (`tests/loom_term_list.rs`, `#![cfg(loom)]`) and the
/// multi-threaded Miri-TB harness (`tests/miri_phase22_1.rs`) are EXTERNAL
/// integration crates that cannot reach the crate-internal (`pub(crate)`)
/// `TermScratch` / `resolve_term_filtered`
/// / `reclaim_retired`. A `pub use` of a
/// `pub(crate)` item is rejected (E0364/E0365). To honor the Phase-9.1 C1
/// lesson — the gates MUST drive the *real* production protocol, not a copy —
/// this module exposes thin `pub` shims that forward, one call each, to the
/// unchanged `pub(crate)` items. Loom/Miri therefore observe the genuine
/// `Acquire`/`Release`/`AcqRel` orderings and the real `Box::from_raw` frees of
/// `resolve_term_filtered` / `reclaim_retired` verbatim.
///
/// This module adds NO production code path: every shipped caller still goes
/// through the `pub(crate)` items directly; the shims are reachable only from a
/// test binary that imports them by their `#[doc(hidden)]` paths. Native
/// codegen of the protocol is byte-identical with or without this module.
#[doc(hidden)]
pub mod test_exports {
    use super::{TagTerms, TermScratch};
    use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::core::component::component_registry::TagId;
    use crate::ecs::core::iters::query_state::QueryState;
    use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

    /// Opaque `pub` handle wrapping the real `pub(crate)` `TermScratch`, so
    /// the external loom / Miri harness can construct, share, and drive it
    /// without the `pub(crate)` type appearing in a `pub` signature (mirrors
    /// `boyko_threadpool::loom_exports::LoomScopeShared`). Auto `Send + Sync`
    /// because `TermScratch` is (two `AtomicPtr`) — the harness shares
    /// `&TestScratch` across threads, exactly the production sharing shape.
    pub struct TestScratch(TermScratch);

    impl TestScratch {
        /// A real, default-constructed [`TermScratch`] (two null `AtomicPtr`s).
        /// Drives the genuine protocol — loom/Miri see the real atomics.
        #[inline]
        #[allow(clippy::new_without_default)]
        pub fn new() -> Self {
            Self(TermScratch::new())
        }

        /// Forwards verbatim to the real
        /// [`TermScratch::resolve_term_filtered`] — the fast-path `Acquire`
        /// load, the cold `rebuild_publish` CAS / `retired.swap`, the loser
        /// `Box::from_raw`, and the published/winner slice deref are all the
        /// production code under test.
        #[inline]
        pub fn resolve<'q>(
            &'q self,
            terms: &TestTerms,
            master: &ArchetypeMaster,
            state: &QueryState,
        ) -> &'q [ArchetypeId] {
            self.0.resolve_term_filtered(&terms.0, master, state)
        }

        /// Forwards verbatim to the real [`TermScratch::reclaim_retired`] — the
        /// `Relaxed` null-check hint + the cold `Acquire` swap-to-null +
        /// `Box::from_raw` free. This is the P2 reclaim point the gate-11b race
        /// interleaves against an in-flight [`Self::resolve`].
        #[inline]
        pub fn reclaim(&self) {
            self.0.reclaim_retired();
        }
    }

    /// Opaque `pub` handle wrapping the real `pub(crate)` `TagTerms` epoch
    /// fingerprint.
    #[derive(Clone, Copy)]
    pub struct TestTerms(TagTerms);

    /// Builds a real `TagTerms` carrying ONE `with` term over `tag` (the
    /// non-empty, term-bearing path `resolve_term_filtered` debug-asserts).
    #[inline]
    pub fn one_with_term(tag: TagId) -> TestTerms {
        let mut t = TagTerms::EMPTY;
        t.push_with(tag);
        TestTerms(t)
    }

    /// Registers a zero-sized tag layout under `id` (the `register_layout`
    /// escape hatch) and returns its [`TagId`]. The harness uses this to mint a
    /// real, archetype-testable tag without the global dynamic-tag counter.
    #[inline]
    pub fn register_tag_layout(id: usize) -> TagId {
        // The term test only reads the archetype's component mask, never the
        // layout. A ZST layout keeps the archetype's pool trivial.
        struct TestTag;
        component_registry::register_layout::<TestTag>(id);
        TagId(ComponentId(id))
    }

    /// Real `ArchetypeMaster` + a one-archetype setup so the harness can drive
    /// `resolve` against genuine generations and a genuine `get_archetype` /
    /// `archetype_passes_tag_terms` build.
    #[inline]
    pub fn master_with_tag_archetype(tag: TagId) -> ArchetypeMaster {
        let mut master = ArchetypeMaster::new();
        master.create_archetype(&[tag.component_id()]);
        master
    }

    /// A real, synced [`QueryState`] matching the tag's archetype, with its
    /// generation pair stamped against `master` (so the O1 `generations_synced`
    /// debug-assert in `rebuild_publish` holds).
    #[inline]
    pub fn synced_state(master: &ArchetypeMaster, tag: TagId) -> QueryState {
        let mut state = QueryState::with_component_ids(&[tag.component_id()]);
        state.update_archetypes(master);
        state
    }

    /// Bumps `master`'s archetype generation by minting an unrelated archetype,
    /// then re-syncs `state` — produces a genuine epoch change so the harness
    /// can drive the rebuild/retire arm against the SAME slot.
    #[inline]
    pub fn bump_epoch_and_resync(
        master: &mut ArchetypeMaster,
        state: &mut QueryState,
        unrelated_tag: TagId,
    ) {
        master.create_archetype(&[unrelated_tag.component_id()]);
        state.update_archetypes(master);
    }

    /// Number of ids a resolved slice carries — lets the harness assert "both
    /// threads adopted the same list" without exposing `TermList`'s internals.
    #[inline]
    pub fn list_len(ids: &[ArchetypeId]) -> usize {
        ids.len()
    }
}
