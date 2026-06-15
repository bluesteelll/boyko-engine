//! `QueryIter` / `QueryIterMut` — hot-path Query iterators.
//!
//! Two cursor shapes:
//!
//! * [`QueryIter`] — read-only cursor. Iterator impl is gated on
//!   `D: ReadOnlyQueryData`. Mints archetype pointers via
//!   [`UnsafeEcsCell::archetype_ptr`] (read-only) and dispatches every
//!   per-archetype refresh through [`QueryData::set_table_readonly`] /
//!   [`QueryFilter::set_table_readonly`]. No `*const → *mut` cast occurs on
//!   this path.
//!
//! * [`QueryIterMut`] — mutable cursor. Iterator impl accepts any
//!   `D: QueryData`. Mints archetype pointers via
//!   [`UnsafeEcsCell::archetype_ptr_mut`] (write-capable) and dispatches every
//!   per-archetype refresh through [`QueryData::set_table_mut`] /
//!   [`QueryFilter::set_table_mut`].
//!
//! # Hot loop shape (M2; Phase 22.1 Area A — term-free cursors)
//!
//! `next()` is a two-level `loop { while { ... } }`: the outer loop advances
//! the archetype cursor, the inner loop walks the rows of the current
//! archetype. The per-row `filter_fetch` call is guarded by
//! `if !const { F::IS_ARCHETYPAL }`, which const-folds the entire branch away
//! for archetypal filters at monomorphisation time. See §7.1 of the Phase 8b
//! plan and the §7.2 walkthrough for the per-row cost model.
//!
//! Phase 22.1 Area A restored both `next()` bodies to their byte-identical
//! pre-Phase-22 form. The cursors carry **no term state** and walk a plain
//! caller-supplied `&[ArchetypeId]` slice — either the shared
//! `matched_ids_pre_terms()` cache (no terms) or a per-epoch memoised
//! term-filtered slice (resolved once at the driver entry; see
//! [`term_list`](super::term_list)). The Phase 22 F1 per-transition tag-term
//! test (and its cold/inline scan asymmetry across the two cursors) measured
//! a nonzero floor in `next()` even when no terms were set (+3.6% on a bare
//! len-read with an unreachable scan); only the ABSENCE of all term code
//! reaches the 0% gate. The transition block is now: `slice-iter next →
//! archetype_ptr(_mut)` None-skip (Q5) → `set_table_*` → `entity_count`.
//!
//! # Stale id skip (Q5)
//!
//! `archetype_ptr(_mut)` returns `None` for archetype ids whose archetype was
//! removed after this iterator was constructed (or after the underlying
//! `QueryDataState` was last synced). The outer loop's `continue` transparently
//! skips those entries — the cursor never yields a stale entity row.
//!
//! [`QueryData::set_table_readonly`]: crate::ecs::core::iters::query::data::QueryData::set_table_readonly
//! [`QueryData::set_table_mut`]: crate::ecs::core::iters::query::data::QueryData::set_table_mut
//! [`QueryFilter::set_table_readonly`]: crate::ecs::core::iters::query::filter::QueryFilter::set_table_readonly
//! [`QueryFilter::set_table_mut`]: crate::ecs::core::iters::query::filter::QueryFilter::set_table_mut
//! [`UnsafeEcsCell::archetype_ptr`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::archetype_ptr
//! [`UnsafeEcsCell::archetype_ptr_mut`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::archetype_ptr_mut

// Step 7 lands the cursors as `pub(crate)` so the in-file unit tests can
// consume `QueryIter::new` / `QueryIterMut::new`. Step 8 introduces
// `Query::iter` / `Query::iter_mut` and turns the cursors into the canonical
// consumer of `new`, removing the lib-only dead-code warning. The blanket
// allow here mirrors the pattern in `system/unsafe_ecs_cell.rs` (Phase 8a
// Step 5).
#![allow(dead_code)]

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::iters::query::data::{QueryData, ReadOnlyQueryData};
use crate::ecs::core::iters::query::enable_terms::{EnableTermCols, EnableTerms};
use crate::ecs::core::iters::query::filter::QueryFilter;
use crate::ecs::core::iters::query::state::QueryDataState;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use crate::ecs::identifiers::primitives::ArchetypeId;

// ── QueryIter (read-only cursor) ───────────────────────────────────────────

/// Read-only cursor over a `Query<D, F>` — yields `D::Item<'q>` per row.
///
/// Constructed via [`QueryIter::new`]; the iterator impl is gated on
/// `D: ReadOnlyQueryData` so the type system prevents instantiation with a
/// `D` containing `&mut T`.
///
/// # Fields
///
/// * `archetype_ids` — slice cursor over a caller-supplied `&'q
///   [ArchetypeId]` (Phase 22.1: either the shared
///   `matched_ids_pre_terms()` slice, or a per-epoch memoised term-filtered
///   slice — both live at `'q`). As long as the iterator holds the slice
///   iter, the underlying ids cannot be mutated.
/// * `data_state` / `filter_state` — borrowed references to the per-system
///   cached states; consumed by `set_table_readonly` on every archetype
///   transition.
/// * `world` — `Copy` cell handle; methods take `self` by-value to preserve
///   the raw pointer's provenance (Phase 8a C1 fix).
/// * `data_fetch` / `filter_fetch` — per-archetype scratch refreshed by the
///   `set_table_readonly` dispatch. Held inside the iterator (not the
///   `Query`) so re-iter is sound.
/// * `current_row` / `current_len` — inner-loop cursor over the active
///   archetype's row range.
pub struct QueryIter<'q, 's, D: QueryData, F: QueryFilter> {
    archetype_ids: std::slice::Iter<'q, ArchetypeId>,
    data_state: &'s D::State,
    filter_state: &'s F::State,
    world: UnsafeEcsCell<'q>,
    data_fetch: D::Fetch<'q>,
    filter_fetch: F::Fetch<'q>,
    current_row: usize,
    current_len: usize,
    /// Phase 10 Round 2 C2: per-system tick snapshot. Forwarded to
    /// `set_table_*` on every archetype boundary so non-archetypal
    /// filters (Wave C `Added<C>` / `Changed<C>`) and `Ref<T>` / `Mut<T>`
    /// data impls can copy `last_run` / `this_run` into their `Fetch<'q>`.
    ///
    /// `&'s` — the meta lives in the same system-state slot as
    /// `QueryDataState`, so they share the `'s` lifetime by construction.
    meta: &'s SystemMeta,
    /// EnableTag Step 9: per-view dynamic enable terms (the per-row twin of
    /// typed `Enabled<T>` / `Disabled<T>`). `EMPTY` for every query without a
    /// `with_enabled` / `without_enabled` builder — gated behind one
    /// `is_empty()` branch so the no-term cursor is byte-identical (0%-gate).
    enable_terms: EnableTerms,
    /// Per-archetype resolved enable-term columns, refreshed at each archetype
    /// transition ONLY when `enable_terms` is non-empty (mirrors the typed
    /// `EnableFetch` `set_table_*` discipline).
    enable_cols: EnableTermCols,
    _marker: PhantomData<&'s ()>,
}

impl<'q, 's, D: QueryData, F: QueryFilter> QueryIter<'q, 's, D, F>
where
    's: 'q,
{
    /// Builds a fresh read-only cursor over the caller-supplied `ids` slice.
    ///
    /// `state` must already be synced against `world` (via
    /// [`QueryDataState::update`]). Phase 22.1 Area A: the driver entry
    /// resolves the id slice ONCE before constructing the cursor — no terms
    /// → `state.archetype_state.matched_ids_pre_terms()`; terms →
    /// [`TermScratch::resolve_term_filtered`](super::term_list::TermScratch::resolve_term_filtered).
    /// The cursor itself carries no term state.
    ///
    /// # Phase 10 Round 2 C2 — `meta` parameter
    ///
    /// `meta` references the currently-active system's [`SystemMeta`]
    /// (lives at `'s` because the meta slot and the state slot are both
    /// owned by the same system struct). The cursor forwards `meta` to
    /// every `D::set_table_readonly` / `F::set_table_readonly` call so
    /// Wave C filters and data impls can read the per-frame tick snapshot.
    ///
    /// # Lifetime bound (`'s: 'q`)
    ///
    /// The `archetype_ids` slice iter is borrowed from `&'s QueryDataState`
    /// but typed as `std::slice::Iter<'q, _>`. For this re-projection to be
    /// sound, the state borrow must outlive the world borrow. `'s: 'q` is
    /// the standard owner-borrows-state-then-world layering: the
    /// `QueryDataState` is cached per-system and survives the world cell
    /// (typically `'static`-ish from the system's POV).
    ///
    /// # Safety (Q1, QD4)
    ///
    /// * **Q1**: the caller asserts that no aliased `&mut Archetype` is live
    ///   through any sibling cell copy. Phase 9 scheduler enforces
    ///   cross-system aliasing; the `FilteredAccessSet` accumulator enforces
    ///   intra-system aliasing.
    /// * **QD4**: the iterator dispatches only `set_table_readonly` (not the
    ///   `_mut` variant). For correctness this is upheld by the type-level
    ///   `D: ReadOnlyQueryData` bound on the [`Iterator`] impl below; for
    ///   provenance correctness the read-only mint
    ///   `UnsafeEcsCell::archetype_ptr` is the sole route to an archetype
    ///   pointer here — no `*const → *mut` cast exists on this path.
    /// * **U_C2**: `world` must satisfy the read-access contract declared by
    ///   the active `SystemParam::init_access`.
    #[inline]
    pub(crate) unsafe fn new(
        state: &'s QueryDataState<D, F>,
        ids: &'q [ArchetypeId],
        world: UnsafeEcsCell<'q>,
        meta: &'s SystemMeta,
        enable_terms: EnableTerms,
    ) -> Self {
        Self {
            archetype_ids: ids.iter(),
            data_state: &state.data_state,
            filter_state: &state.filter_state,
            world,
            data_fetch: <D as QueryData>::init_fetch(&state.data_state),
            filter_fetch: <F as QueryFilter>::init_fetch(&state.filter_state),
            current_row: 0,
            current_len: 0,
            meta,
            enable_terms,
            enable_cols: EnableTermCols::EMPTY,
            _marker: PhantomData,
        }
    }
}

impl<'q, 's, D: QueryData, F: QueryFilter> Iterator for QueryIter<'q, 's, D, F>
where
    D: ReadOnlyQueryData,
{
    type Item = D::Item<'q>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            while self.current_row < self.current_len {
                let row = self.current_row;
                self.current_row += 1;

                // Const-folded at monomorphisation: for `F::IS_ARCHETYPAL =
                // true` (every Phase 8b filter) the entire `filter_fetch`
                // call vanishes. Phase 10's `Changed<C>` / `Added<C>` will
                // flip this to `false` and reactivate the branch.
                // TODO(phase-8b/step-14): the golden expand test will lock
                // this const-fold into a CI snapshot.
                if !const { F::IS_ARCHETYPAL } {
                    // SAFETY (QF1): `set_table_readonly` was invoked for the
                    //   current archetype before the inner loop began
                    //   (outer-loop body below). `row < self.current_len ==
                    //   archetype.entity_count()` per the inner-loop guard.
                    let pass = unsafe {
                        <F as QueryFilter>::filter_fetch(&self.filter_fetch, row)
                    };
                    if !pass {
                        continue;
                    }
                }

                // EnableTag Step 9: dynamic per-row enable terms. Gated behind
                // one RUNTIME `is_empty()` branch — loop-invariant, so the
                // compiler hoists it and a query with no
                // `with_enabled`/`without_enabled` term pays a single
                // predicted-not-taken branch (the 0%-gate; bench-verified flat,
                // NOT a const-fold — the enable bit is genuinely per-row). When
                // set, `enable_cols` was resolved for this archetype at the
                // transition below.
                if !self.enable_terms.is_empty() {
                    // SAFETY (ENBL-9): `enable_cols` was resolved by
                    //   `EnableTerms::resolve` for the current archetype at the
                    //   outer-loop transition below; `row < self.current_len ==
                    //   entity_count()` per the inner-loop guard. The cached
                    //   column pointers are valid for `'q` (the archetype
                    //   outlives the cursor; a directory regrow runs only inside
                    //   a `&mut` apply window where no cursor is live — same
                    //   contract as the typed `EnableFetch`).
                    let pass = unsafe { self.enable_cols.passes(row) };
                    if !pass {
                        continue;
                    }
                }

                // SAFETY (QD2, QD3): `set_table_readonly` was invoked for the
                //   current archetype before the inner loop began; the
                //   `data_fetch` therefore carries valid column pointers.
                //   `row < self.current_len` (inner-loop guard) ≤
                //   `archetype.entity_count()` cached at the outer-loop
                //   transition.
                return Some(unsafe {
                    <D as QueryData>::fetch(&self.data_fetch, row)
                });
            }

            // Inner loop drained; advance to the next archetype or finish.
            let arch_id = *self.archetype_ids.next()?;

            // M2: read-only mint — `archetype_ptr`, not the `_mut` variant.
            // No `*const → *mut` cast exists on this path.
            //
            // SAFETY (U_C2, Q5): the cell is scoped to `'q` per `new`'s
            //   contract; `archetype_ptr` returns `None` for stale (removed)
            //   ids — those are transparently skipped via `continue` (Q5).
            let Some(archetype_ptr) = (unsafe { self.world.archetype_ptr(arch_id) })
            else {
                continue;
            };

            // SAFETY (QD3, QD4, QF3): `set_table_readonly` accepts a
            //   `*const Archetype` directly — no provenance cast. The pointer
            //   is live for `'q` (Phase 7 U1/U2 slab stability); `data_state`
            //   / `filter_state` correspond to this `D` / `F` and outlive
            //   `'s`. QD4 holds because the trait gate
            //   `D: ReadOnlyQueryData` on this Iterator impl forbids
            //   instantiation with `&mut T`, which is the only `QueryData`
            //   impl that traps in `set_table_readonly`.
            //
            //   Phase 12.5 Track B NCD6: const-fold dispatcher. When
            //   neither `D` nor `F` declares `NEEDS_CHANGE_DETECTION = true`,
            //   route through the `_no_meta` variants — `self.meta` is
            //   never loaded on this monomorphisation. The `_no_meta`
            //   methods panic for `Ref<T>` / `Mut<T>` / `Added<C>` /
            //   `Changed<C>` impls, so reaching them when NCD = true would
            //   be a contract violation; the `if const` branch guarantees
            //   that cannot happen.
            //
            //   Phase 10 Round 2 W7: meta-bearing branch — `self.meta`
            //   references the active system's `SystemMeta`; non-archetypal
            //   filters / `Ref<T>` / `Mut<T>` copy `last_run` / `this_run`
            //   into their Fetch by value.
            unsafe {
                if const { D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION } {
                    <D as QueryData>::set_table_readonly(
                        &mut self.data_fetch,
                        self.data_state,
                        archetype_ptr,
                        self.meta,
                    );
                    <F as QueryFilter>::set_table_readonly(
                        &mut self.filter_fetch,
                        self.filter_state,
                        archetype_ptr,
                        self.meta,
                    );
                } else {
                    <D as QueryData>::set_table_readonly_no_meta(
                        &mut self.data_fetch,
                        self.data_state,
                        archetype_ptr,
                    );
                    <F as QueryFilter>::set_table_readonly_no_meta(
                        &mut self.filter_fetch,
                        self.filter_state,
                        archetype_ptr,
                    );
                }
            }

            // Read-only probe to extract `entity_count`. The raw deref
            // materialises only an immutable view; no `&mut Archetype` is
            // constructed.
            //
            // SAFETY (U1, U2): `archetype_ptr` is live for `'q` (slab
            //   stability); the `&Archetype` reborrow is scoped to this
            //   block. No aliasing `&mut Archetype` exists on the read-only
            //   path.
            let arch_ref: &Archetype = unsafe { &*archetype_ptr };
            self.current_row = 0;
            self.current_len = arch_ref.entity_count();
            // EnableTag Step 9: refresh the per-archetype enable-term columns
            // ONLY when terms are set (the no-term cursor never touches this —
            // 0%-gate). `arch_ref` is the live `&Archetype` for this transition.
            if !self.enable_terms.is_empty() {
                self.enable_cols = self.enable_terms.resolve(arch_ref);
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        // The total row count is the sum of `entity_count()` over the
        // remaining archetypes — we would have to materialise every archetype
        // to compute it, which defeats the purpose of size_hint. Defer to
        // the conservative `(0, None)` until Phase 11 needs an upper bound.
        (0, None)
    }
}

// ── QueryIterMut (mutable cursor) ──────────────────────────────────────────

/// Mutable cursor over a `Query<D, F>` — yields `D::Item<'q>` per row.
///
/// Constructed via [`QueryIterMut::new`]; the iterator impl is unconditional
/// (any `D: QueryData` works, including `&mut T`).
///
/// Field shape mirrors [`QueryIter`]; the only difference is that
/// `archetype_ptr_mut` / `set_table_mut` are dispatched on the hot path,
/// preserving write-capable provenance end-to-end.
pub struct QueryIterMut<'q, 's, D: QueryData, F: QueryFilter> {
    archetype_ids: std::slice::Iter<'q, ArchetypeId>,
    data_state: &'s D::State,
    filter_state: &'s F::State,
    world: UnsafeEcsCell<'q>,
    data_fetch: D::Fetch<'q>,
    filter_fetch: F::Fetch<'q>,
    current_row: usize,
    current_len: usize,
    /// Phase 10 Round 2 C2: per-system tick snapshot. Same shape and
    /// purpose as [`QueryIter::meta`].
    meta: &'s SystemMeta,
    /// EnableTag Step 9: per-view dynamic enable terms. Same shape and 0%-gate
    /// discipline as [`QueryIter::enable_terms`].
    enable_terms: EnableTerms,
    /// EnableTag Step 9: per-archetype resolved enable-term columns. Same shape
    /// as [`QueryIter::enable_cols`].
    enable_cols: EnableTermCols,
    _marker: PhantomData<&'s ()>,
}

impl<'q, 's, D: QueryData, F: QueryFilter> QueryIterMut<'q, 's, D, F>
where
    's: 'q,
{
    /// Builds a fresh mutable cursor over the caller-supplied `ids` slice.
    ///
    /// `state` must already be synced against `world`. Phase 22.1 Area A: the
    /// driver entry resolves the id slice ONCE before constructing the cursor
    /// (no terms → `matched_ids_pre_terms()`; terms →
    /// [`TermScratch::resolve_term_filtered`](super::term_list::TermScratch::resolve_term_filtered)).
    /// The cursor carries no term state.
    ///
    /// # Phase 10 Round 2 C2 — `meta` parameter
    ///
    /// Same contract as [`QueryIter::new`] — `meta` is forwarded to every
    /// `D::set_table_mut` / `F::set_table_mut` call so Wave C `Mut<T>` /
    /// `Changed<C>` impls can read the per-frame tick snapshot.
    ///
    /// # Lifetime bound (`'s: 'q`)
    ///
    /// Same rationale as [`QueryIter::new`] — the state borrow underwrites the
    /// world borrow.
    ///
    /// # Safety (Q1, QD4)
    ///
    /// * **Q1**: caller asserts no aliased `&mut Archetype` is live through
    ///   any sibling cell copy. Enforced by `FilteredAccessSet` at
    ///   `init_access` time and by Phase 9's scheduler at run time.
    /// * **QD4**: the iterator dispatches only `set_table_mut`. Together with
    ///   the write-capable mint from `archetype_ptr_mut` this gives a
    ///   `*mut Archetype` end-to-end with no cast.
    /// * **U_C3**: `world` must have been minted via
    ///   [`UnsafeEcsCell::new_mutable`]. The debug-mode `allows_mutable_access`
    ///   sentinel inside `archetype_ptr_mut` will panic otherwise.
    ///
    /// [`UnsafeEcsCell::new_mutable`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::new_mutable
    #[inline]
    pub(crate) unsafe fn new(
        state: &'s QueryDataState<D, F>,
        ids: &'q [ArchetypeId],
        world: UnsafeEcsCell<'q>,
        meta: &'s SystemMeta,
        enable_terms: EnableTerms,
    ) -> Self {
        Self {
            archetype_ids: ids.iter(),
            data_state: &state.data_state,
            filter_state: &state.filter_state,
            world,
            data_fetch: <D as QueryData>::init_fetch(&state.data_state),
            filter_fetch: <F as QueryFilter>::init_fetch(&state.filter_state),
            current_row: 0,
            current_len: 0,
            meta,
            enable_terms,
            enable_cols: EnableTermCols::EMPTY,
            _marker: PhantomData,
        }
    }
}

impl<'q, 's, D: QueryData, F: QueryFilter> Iterator for QueryIterMut<'q, 's, D, F> {
    type Item = D::Item<'q>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            while self.current_row < self.current_len {
                let row = self.current_row;
                self.current_row += 1;

                // See QueryIter::next for the const-fold rationale.
                // TODO(phase-8b/step-14): golden expand test locks the
                // archetypal-only branch out of the inner loop.
                if !const { F::IS_ARCHETYPAL } {
                    // SAFETY (QF1): `set_table_mut` was invoked for the
                    //   current archetype before the inner loop began.
                    //   `row < self.current_len` per the inner-loop guard.
                    let pass = unsafe {
                        <F as QueryFilter>::filter_fetch(&self.filter_fetch, row)
                    };
                    if !pass {
                        continue;
                    }
                }

                // EnableTag Step 9: dynamic per-row enable terms (see
                // `QueryIter::next` for the 0%-gate rationale). The enable bit
                // is read shared regardless of the cursor's mutability.
                if !self.enable_terms.is_empty() {
                    // SAFETY (ENBL-9): `enable_cols` was resolved for the current
                    //   archetype at the transition below; `row < self.current_len`
                    //   per the inner-loop guard; column pointers valid for `'q`.
                    let pass = unsafe { self.enable_cols.passes(row) };
                    if !pass {
                        continue;
                    }
                }

                // SAFETY (QD2, QD3): `set_table_mut` was invoked for the
                //   current archetype before the inner loop began; the
                //   `data_fetch` carries valid (write-capable) column
                //   pointers. `row < self.current_len`.
                return Some(unsafe {
                    <D as QueryData>::fetch(&self.data_fetch, row)
                });
            }

            let arch_id = *self.archetype_ids.next()?;

            // M2: write-capable mint — `archetype_ptr_mut`. In debug builds
            // the cell's `allows_mutable_access` sentinel fires here if the
            // caller of `QueryIterMut::new` violated Q1 by handing in a
            // read-only cell (covers the type-level Q1 gap that exists
            // because this impl block has no `D: !ReadOnlyQueryData` bound).
            //
            // SAFETY (U_C3, Q1, Q5): cell is write-capable (caller contract);
            //   `archetype_ptr_mut` returns `None` for stale ids — those are
            //   skipped via `continue` (Q5).
            let Some(archetype_ptr) = (unsafe { self.world.archetype_ptr_mut(arch_id) })
            else {
                continue;
            };

            // SAFETY (QD3, QD4, QF3): `set_table_mut` accepts a
            //   `*mut Archetype` directly — no provenance cast or downgrade.
            //   The pointer is live for `'q` (Phase 7 U1/U2 slab stability).
            //   `data_state` / `filter_state` correspond to this `D` / `F`
            //   and outlive `'s`.
            //
            //   Phase 12.5 Track B NCD6: const-fold dispatcher. Same
            //   shape as the read-only cursor — when neither `D` nor
            //   `F` declares `NEEDS_CHANGE_DETECTION = true`, route
            //   through the `_no_meta` variants and skip the meta load
            //   entirely. The `_no_meta` methods panic on `Ref<T>` /
            //   `Mut<T>` / `Added<C>` / `Changed<C>` impls; the `if
            //   const` branch guarantees they are never reached on the
            //   wrong monomorphisation.
            //
            //   Phase 10 Round 2 W7: meta-bearing branch — `self.meta`
            //   references the active system's `SystemMeta`; Wave C
            //   consumers copy the per-frame ticks into Fetch by value.
            unsafe {
                if const { D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION } {
                    <D as QueryData>::set_table_mut(
                        &mut self.data_fetch,
                        self.data_state,
                        archetype_ptr,
                        self.meta,
                    );
                    <F as QueryFilter>::set_table_mut(
                        &mut self.filter_fetch,
                        self.filter_state,
                        archetype_ptr,
                        self.meta,
                    );
                } else {
                    <D as QueryData>::set_table_mut_no_meta(
                        &mut self.data_fetch,
                        self.data_state,
                        archetype_ptr,
                    );
                    <F as QueryFilter>::set_table_mut_no_meta(
                        &mut self.filter_fetch,
                        self.filter_state,
                        archetype_ptr,
                    );
                }
            }

            // Read-only probe to extract `entity_count` from the
            // write-capable pointer. No `&mut Archetype` is materialised;
            // the raw deref produces only an `&Archetype` view that lives
            // exactly for the duration of the `entity_count()` call.
            //
            // SAFETY (U1, U2): `archetype_ptr` is slab-stable for `'q`.
            //   The `&Archetype` is scoped to this block; no aliasing
            //   `&mut Archetype` reborrow exists in this scope.
            let arch_ref: &Archetype = unsafe { &*archetype_ptr };
            self.current_row = 0;
            self.current_len = arch_ref.entity_count();
            // EnableTag Step 9: refresh per-archetype enable-term columns only
            // when terms are set (the no-term cursor never touches this). The
            // enable bit is read shared, so the `&Archetype` view suffices even
            // on the mutable cursor.
            if !self.enable_terms.is_empty() {
                self.enable_cols = self.enable_terms.resolve(arch_ref);
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        // See `QueryIter::size_hint` rationale.
        (0, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
    use crate::ecs::core::iters::query::filter::Without;
    use crate::ecs::core::system::system_meta::SystemMeta;
    use crate::ecs::identifiers::primitives::ComponentId;

    // Component IDs 483-489 reserved for Phase 8b Step 7 iter tests.
    // Free range verified at write time:
    //   * 480-482 — archetype_bundle / drop_safety / swap_remove bench
    //   * 490-499 — query_state / random_access bench
    //   * 500-501 — random_access bench
    //   * 503-504 — query/data.rs tests
    //   * 506-509 — query/state.rs tests
    //   * 510      — resource_registry CompThenRes
    // MAX_COMPONENTS = 512 caps valid ids at 511; the orchestrator-suggested
    // 514-520 is out of range, so 483-489 is the closest contiguous block.
    const COMP_A: ComponentId = ComponentId(483);
    const COMP_B: ComponentId = ComponentId(484);
    const COMP_C: ComponentId = ComponentId(485);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompA(u32);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompB(u32);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompC(u32);

    impl Component for CompA {
        fn component_id() -> ComponentId {
            COMP_A
        }
    }
    impl Component for CompB {
        fn component_id() -> ComponentId {
            COMP_B
        }
    }
    impl Component for CompC {
        fn component_id() -> ComponentId {
            COMP_C
        }
    }

    /// Idempotent registry priming.
    fn register_test_components() {
        component_registry::register_layout::<CompA>(COMP_A.0);
        component_registry::register_layout::<CompB>(COMP_B.0);
        component_registry::register_layout::<CompC>(COMP_C.0);
    }

    /// Spawns a `CompA(value)` entity into `arch_id`.
    fn spawn_a(ecs: &mut EcsMaster, arch_id: crate::ecs::identifiers::primitives::ArchetypeId, value: u32) {
        let comp = CompA(value);
        // SAFETY: `CompA` is `#[repr(C)]` POD; reading its bytes produces a
        //   valid byte slice for the duration of this call.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &comp as *const CompA as *const u8,
                std::mem::size_of::<CompA>(),
            )
        };
        ecs.create_entity(arch_id, &[(COMP_A, bytes)])
            .expect("spawn_a: create_entity must succeed");
    }

    /// Spawns a `(CompA(a), CompB(b))` entity into `arch_id`.
    fn spawn_ab(
        ecs: &mut EcsMaster,
        arch_id: crate::ecs::identifiers::primitives::ArchetypeId,
        a: u32,
        b: u32,
    ) {
        let ca = CompA(a);
        let cb = CompB(b);
        // SAFETY: both are `#[repr(C)]` POD; the byte slices are valid for
        //   this call's duration.
        let a_bytes = unsafe {
            std::slice::from_raw_parts(
                &ca as *const CompA as *const u8,
                std::mem::size_of::<CompA>(),
            )
        };
        let b_bytes = unsafe {
            std::slice::from_raw_parts(
                &cb as *const CompB as *const u8,
                std::mem::size_of::<CompB>(),
            )
        };
        ecs.create_entity(arch_id, &[(COMP_A, a_bytes), (COMP_B, b_bytes)])
            .expect("spawn_ab: create_entity must succeed");
    }

    /// One archetype with three entities — the cursor must yield all three.
    #[test]
    fn single_archetype_yields_all() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[COMP_A]);
        for i in 0..3 {
            spawn_a(&mut ecs, arch, i as u32 + 100);
        }

        let state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        let meta = SystemMeta::for_testing("test");
        // SAFETY (U_C1): `cell` does not outlive the `&mut ecs` borrow
        //   below — it is consumed by `iter` within this function.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        let ids = state.archetype_state.matched_ids_pre_terms();
        // SAFETY (Q1, QD4, U_C2): direct test of `QueryIter::next`; no
        //   aliasing accessor is live in this scope.
        let iter = unsafe { QueryIter::<&CompA, ()>::new(&state, ids, cell, &meta, crate::ecs::core::iters::query::enable_terms::EnableTerms::EMPTY) };

        let collected: Vec<u32> = iter.map(|a: &CompA| a.0).collect();
        assert_eq!(collected.len(), 3, "single archetype must yield 3 rows");
        assert!(collected.contains(&100));
        assert!(collected.contains(&101));
        assert!(collected.contains(&102));
    }

    /// Two archetypes — the cursor must traverse both (archetype-major order).
    #[test]
    fn archetype_transition() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch_a = ecs.create_archetype(&[COMP_A]);
        let arch_ab = ecs.create_archetype(&[COMP_A, COMP_B]);

        // 2 entities in arch_a, 3 in arch_ab → 5 total rows.
        spawn_a(&mut ecs, arch_a, 200);
        spawn_a(&mut ecs, arch_a, 201);
        spawn_ab(&mut ecs, arch_ab, 300, 0);
        spawn_ab(&mut ecs, arch_ab, 301, 0);
        spawn_ab(&mut ecs, arch_ab, 302, 0);

        let state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        assert_eq!(
            state.archetype_state.matched_ids_pre_terms().len(),
            2,
            "both CompA-bearing archetypes must be matched",
        );

        let meta = SystemMeta::for_testing("test");
        // SAFETY (U_C1): cell consumed inside this function.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        let ids = state.archetype_state.matched_ids_pre_terms();
        // SAFETY (Q1, QD4, U_C2): direct cursor test, no aliasing.
        let iter = unsafe { QueryIter::<&CompA, ()>::new(&state, ids, cell, &meta, crate::ecs::core::iters::query::enable_terms::EnableTerms::EMPTY) };

        let collected: Vec<u32> = iter.map(|a: &CompA| a.0).collect();
        assert_eq!(collected.len(), 5, "two archetypes must yield 5 rows");
        for expected in [200u32, 201, 300, 301, 302] {
            assert!(
                collected.contains(&expected),
                "row {} must appear in collected = {:?}",
                expected,
                collected,
            );
        }
    }

    /// An archetype with zero entities forces the outer loop to advance —
    /// the inner loop body must not run.
    #[test]
    fn empty_archetype_skipped() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch_empty = ecs.create_archetype(&[COMP_A]);
        let arch_full = ecs.create_archetype(&[COMP_A, COMP_B]);
        // Leave arch_empty without entities; populate arch_full.
        let _ = arch_empty;
        spawn_ab(&mut ecs, arch_full, 999, 0);

        let state = QueryDataState::<&CompA, ()>::new(&mut ecs);

        let meta = SystemMeta::for_testing("test");
        // SAFETY (U_C1): cell consumed below.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        let ids = state.archetype_state.matched_ids_pre_terms();
        // SAFETY (Q1, QD4, U_C2): direct cursor test.
        let iter = unsafe { QueryIter::<&CompA, ()>::new(&state, ids, cell, &meta, crate::ecs::core::iters::query::enable_terms::EnableTerms::EMPTY) };

        let collected: Vec<u32> = iter.map(|a: &CompA| a.0).collect();
        assert_eq!(
            collected,
            vec![999],
            "empty archetype must be skipped; only arch_full's row yields",
        );
    }

    /// Manually push a non-existent `ArchetypeId` into `matched_ids` via the
    /// `matched_ids_mut` escape hatch; the cursor's `archetype_ptr` returns
    /// `None` for the stale id and the outer loop's `continue` skips it.
    /// Verifies Q5 — the stale-id-skip path.
    #[test]
    fn stale_id_skipped() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[COMP_A]);
        spawn_a(&mut ecs, arch, 42);

        let mut state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        // Corrupt the matched_ids with a synthetic stale id. `archetype_ptr`
        // will return None for ArchetypeId(999) (not registered with the
        // master) — the cursor must skip transparently. We push a duplicate
        // of the real id first so the dual invariant assertion in
        // `post_filter_matched` is not re-triggered by this test (the
        // assertion runs only inside `post_filter_matched`, not when the
        // cursor walks).
        state.archetype_state.matched_ids_pre_terms_mut().push(crate::ecs::identifiers::primitives::ArchetypeId(999));

        let meta = SystemMeta::for_testing("test");
        // SAFETY (U_C1): cell consumed below.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        let ids = state.archetype_state.matched_ids_pre_terms();
        // SAFETY (Q1, QD4, U_C2, Q5): the stale id is exactly the case the
        //   `continue` branch handles.
        let iter = unsafe { QueryIter::<&CompA, ()>::new(&state, ids, cell, &meta, crate::ecs::core::iters::query::enable_terms::EnableTerms::EMPTY) };

        let collected: Vec<u32> = iter.map(|a: &CompA| a.0).collect();
        assert_eq!(
            collected,
            vec![42],
            "cursor must skip the stale id and yield only the real entity",
        );
    }

    /// `iter_mut` cursor — mutating the per-row `&mut CompA` persists in the
    /// underlying storage. Re-iterating with a fresh read-only cursor
    /// observes the post-mutation values.
    #[test]
    fn iter_mut_mutations_persist() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[COMP_A]);
        for i in 0..3 {
            spawn_a(&mut ecs, arch, i as u32);
        }

        // Phase 1: mutate every CompA to 99 via QueryIterMut.
        {
            let state = QueryDataState::<&mut CompA, ()>::new(&mut ecs);
            let meta = SystemMeta::for_testing("test");
            // SAFETY (U_C1): cell consumed in this block.
            let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
            let ids = state.archetype_state.matched_ids_pre_terms();
            // SAFETY (Q1, QD4, U_C3): direct mut-cursor test, no aliasing.
            let iter = unsafe { QueryIterMut::<&mut CompA, ()>::new(&state, ids, cell, &meta, crate::ecs::core::iters::query::enable_terms::EnableTerms::EMPTY) };
            for a in iter {
                a.0 = 99;
            }
        }

        // Phase 2: re-read with a fresh QueryIter — every row must be 99.
        let state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        let meta = SystemMeta::for_testing("test");
        // SAFETY (U_C1): cell consumed in this block.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        let ids = state.archetype_state.matched_ids_pre_terms();
        // SAFETY (Q1, QD4, U_C2): no aliasing accessor live.
        let iter = unsafe { QueryIter::<&CompA, ()>::new(&state, ids, cell, &meta, crate::ecs::core::iters::query::enable_terms::EnableTerms::EMPTY) };
        let collected: Vec<u32> = iter.map(|a: &CompA| a.0).collect();
        assert_eq!(collected.len(), 3, "three rows must remain after mutation");
        assert!(
            collected.iter().all(|&v| v == 99),
            "every row must have been mutated to 99; got {:?}",
            collected,
        );
    }

    /// `Query<&CompA, Without<CompB>>` skips an archetype that contains
    /// `CompB`. Verifies the archetypal `Without` filter at the
    /// match-cache level (post_filter_matched).
    #[test]
    fn without_filter_excludes_archetype() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch_a_only = ecs.create_archetype(&[COMP_A]);
        let arch_ab = ecs.create_archetype(&[COMP_A, COMP_B]);
        spawn_a(&mut ecs, arch_a_only, 10);
        spawn_ab(&mut ecs, arch_ab, 20, 0);

        let state = QueryDataState::<&CompA, Without<CompB>>::new(&mut ecs);
        assert_eq!(
            state.archetype_state.matched_ids_pre_terms().len(),
            1,
            "Without<CompB> must drop the (CompA, CompB) archetype",
        );

        let meta = SystemMeta::for_testing("test");
        // SAFETY (U_C1): cell consumed below.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        let ids = state.archetype_state.matched_ids_pre_terms();
        // SAFETY (Q1, QD4, U_C2): direct cursor test.
        let iter = unsafe { QueryIter::<&CompA, Without<CompB>>::new(&state, ids, cell, &meta, crate::ecs::core::iters::query::enable_terms::EnableTerms::EMPTY) };

        let collected: Vec<u32> = iter.map(|a: &CompA| a.0).collect();
        assert_eq!(
            collected,
            vec![10],
            "Without<CompB> must yield only the arch_a_only row",
        );
    }

    /// Compile-only smoke check that `QueryIter` is constructible for an
    /// archetypal filter. The const-fold itself is verified at the assembly
    /// / `cargo expand` level by Step 14's golden snapshot; this test only
    /// proves the call typechecks.
    // TODO(phase-8b/step-14): replace with a `cargo expand` golden snapshot
    // assertion confirming no `filter_fetch` symbol in the inner loop for
    // `Query<&CompA, Without<CompB>>`.
    #[test]
    fn const_fold_archetypal_no_filter_fetch_call() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[COMP_A]);
        spawn_a(&mut ecs, arch, 0);
        let state = QueryDataState::<&CompA, Without<CompB>>::new(&mut ecs);
        let meta = SystemMeta::for_testing("test");
        // SAFETY (U_C1): cell consumed below.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        let ids = state.archetype_state.matched_ids_pre_terms();
        // SAFETY (Q1, QD4, U_C2): const-fold path; no aliasing.
        let iter = unsafe { QueryIter::<&CompA, Without<CompB>>::new(&state, ids, cell, &meta, crate::ecs::core::iters::query::enable_terms::EnableTerms::EMPTY) };
        // The very act of consuming the iterator without panic confirms that
        // the const-folded branch did not produce a runtime call that
        // mis-dispatches. The golden expand snapshot in Step 14 nails it
        // down at the source-expansion level.
        let _: Vec<&CompA> = iter.collect();
    }
}
