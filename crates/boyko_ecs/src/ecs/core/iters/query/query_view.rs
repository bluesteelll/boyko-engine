//! `QueryView<'w, D, F>` — direct query API handle (Phase 12.5 Track B Wave D).
//!
//! Wave D Step 3 lands `QueryView` as the return type of the
//! [`EcsMaster::query`](crate::ecs::core::ecs_master::ecs_master::EcsMaster::query)
//! direct API. Bypasses the `FunctionSystem` wrapper / `FilteredAccessSet`
//! overhead by relying on `&mut EcsMaster` to gate aliasing at the type
//! level.
//!
//! See `docs/PHASE-12.5-QUERY-OPTIMIZATIONS-PLAN.md` §4.3 (data layout),
//! §5 (API surface), §7.5 (Send/Sync proof), and §10.2 (memory layout).
//!
//! # Send/Sync (W1 / I-NEW-5 — single canonical assertion)
//!
//! `QueryView<'w, D, F>: Send + Sync` whenever `D::State: Send + Sync` and
//! `F::State: Send + Sync` (the `QueryData::State` / `QueryFilter::State`
//! trait bound at `data.rs:90`/`filter.rs:76` already requires both).
//! Equivalent to the existing [`Query<'w, 's, D, F>`](super::query::Query)
//! SystemParam's Send/Sync surface.
//!
//! The module-scope `assert_impl_all!` below fires on **every compile**
//! (debug + release + test + doctest) — single canonical Send/Sync assertion
//! per the W1 fold; no `cfg(test)`-gated alternative exists.

use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ptr::NonNull;

use static_assertions::assert_impl_all;

use crate::ecs::core::component::component_registry::TagId;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::iters::query::chunk_iter;
use crate::ecs::core::iters::query::chunked_data::ChunkedQueryData;
use crate::ecs::core::iters::query::data::{QueryData, ReadOnlyQueryData};
use crate::ecs::core::iters::query::filter::{ArchetypalQueryFilter, QueryFilter};
use crate::ecs::core::iters::query::iter::{QueryIter, QueryIterMut};
use crate::ecs::core::iters::query::par_chunk;
use crate::ecs::core::iters::query::par_iter::{BatchingStrategy, ParQuery, ParQueryMut};
use crate::ecs::core::iters::query::state::QueryDataState;
use crate::ecs::core::iters::query::tag_terms::{
    TagTerms, any_term_matched, archetype_passes_tag_terms, count_term_matched,
};
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// Direct query iteration handle returned by
/// [`EcsMaster::query`](crate::ecs::core::ecs_master::ecs_master::EcsMaster::query).
///
/// Bypasses the `FunctionSystem` wrapper used by the in-system
/// [`Query<'w, 's, D, F>`](super::query::Query) SystemParam — no
/// `FilteredAccessSet` allocation, no per-call `QueryDataState::new` (the
/// state is cached in the world's `query_state_cache`), and no apply pass.
///
/// # Lifetime
///
/// `'w` is the world-borrow lifetime (`&'w mut EcsMaster` upstream); the
/// cached state pointer is valid for `'w` because the cache slot lives in
/// the same `EcsMaster` whose `&mut` borrow produced this view (QV6).
///
/// # Restrictions (QV11 / I-NEW-4 / W4)
///
/// The direct API does **not** support change-detection filters. Any
/// `EcsMaster::query<D, F>()` call with `D::NEEDS_CHANGE_DETECTION ||
/// F::NEEDS_CHANGE_DETECTION == true` panics at runtime with the canonical
/// W4 message. Use `Query<D, F>` as a SystemParam inside a system body via
/// `Schedule` for change-detection queries.
///
/// # Layout (§10.2, amended by Phase 22 D4)
///
/// `UnsafeEcsCell<'w>` (8 B) + `NonNull<UnsafeCell<...>>` (8 B) + inline
/// [`TagTerms`] (stack-only, `MAX_DYN_TAG_TERMS` slots) + ZST `PhantomData`.
/// Hot pointer fields plus an inline 72-B term block; no heap indirection.
/// Field order is left to rustc (no `#[repr(C)]`); the no-terms fast path
/// reads only the terms' `len == 0` byte once per archetype transition.
pub struct QueryView<'w, D: QueryData, F: QueryFilter = ()> {
    /// World-access cell. By-value copy — `UnsafeEcsCell` is `Copy + Send + Sync`
    /// per SEND3 and never holds a `&` retag.
    world: UnsafeEcsCell<'w>,

    /// Type-erased pointer to the world-owned
    /// `Box<UnsafeCell<QueryDataState<D, F>>>` minted by
    /// `EcsMaster::query_cold_init` and stored in the cache slot.
    ///
    /// `UnsafeCell` wrapper exists for I1 (Tree Borrows hygiene) — the
    /// cache permits both `&` and `&mut` reborrows of the inner state
    /// across distinct `query` calls; `UnsafeCell::get` mints a `*mut`
    /// without raising a SharedReadOnly retag.
    state: NonNull<UnsafeCell<QueryDataState<D, F>>>,

    /// Phase 22 D4: per-view dynamic-tag terms. Stack-only `Copy` payload —
    /// EMPTY for every `EcsMaster::query`-minted view; populated by
    /// [`Self::with_tag`] / [`Self::without_tag`]. NEVER written into the
    /// world-owned cached `QueryDataState` (QS1 stays term-agnostic).
    terms: TagTerms,

    /// Lifetime binding to `&'w mut EcsMaster` plus invariance over
    /// `(D, F)`. Two separate marker fields keep the type signature
    /// readable (clippy::type_complexity) — the lifetime carrier and the
    /// `fn` invariance carrier each have a single responsibility.
    _world_borrow: PhantomData<&'w mut ()>,
    _data_filter_invariance: PhantomData<fn() -> (D, F)>,
}

// ── Send/Sync — W1 single canonical assertion ──────────────────────────────
//
// SAFETY (SEND10 / I-NEW-5):
//   - `UnsafeEcsCell<'w>: Send + Sync` per `unsafe_ecs_cell.rs:341-342`.
//   - `NonNull<UnsafeCell<QueryDataState<D, F>>>` carries no auto-Send/Sync.
//     The hand-marked impls below mirror the bounds the existing
//     `Query<'w, 's, D, F>` SystemParam carries (data.rs:90 — `D::State:
//     Send + Sync, F::State: Send + Sync`).
//   - `QueryDataState<D, F>` is `Send + Sync` whenever both inner states
//     are; the `UnsafeCell` wrapper does NOT relax this requirement (auto
//     `!Sync` only) — and the `Send` we need for cross-thread cache reads
//     under the future `query_ref<&self>` API is preserved by the
//     `Send + Sync` bound on `D::State`/`F::State`.

// SAFETY (SEND10): `QueryView` holds an `UnsafeEcsCell` (Send + Sync per
// SEND2) and a `NonNull<UnsafeCell<QueryDataState<D, F>>>` whose pointee is
// owned by `EcsMaster` (Send + Sync per the const SEND1 gate at
// `ecs_master.rs:1737-1746`). The trait bounds on `QueryData::State` /
// `QueryFilter::State` (Send + Sync + 'static) guarantee `QueryDataState`
// itself is Send. The `&'w mut EcsMaster` borrow upstream enforces that
// the view is only handed across threads inside a scope where the world
// is exclusively owned — equivalent to the existing `Query` SystemParam.
unsafe impl<D: QueryData, F: QueryFilter> Send for QueryView<'_, D, F> {}
// SAFETY: same composition as Send; `UnsafeEcsCell` is Sync, the NonNull is
// an opaque address (not dereferenced from sibling threads), the pointee
// inside the cache is read-only across `&self` views (cache mutation only
// under `&mut EcsMaster`).
unsafe impl<D: QueryData, F: QueryFilter> Sync for QueryView<'_, D, F> {}

// I-NEW-5 / W1 — single canonical Send/Sync assertion at module scope,
// outside `cfg(test)`. Fires on every compile (debug, release, doctest,
// test). Uses unit `()` for both `D` and `F` because:
//
//   * `(): QueryData` and `(): QueryFilter` exist at `data.rs:1155` /
//     `filter.rs:179` as the Phase 8b empty-tuple/empty-filter stubs.
//   * Both `State` types are `()` (trivially `Send + Sync + 'static`).
//   * The structural Send/Sync geometry is identical to any non-trivial
//     `(D, F)` pair — the trait bounds on `D::State` / `F::State` are the
//     universal ground truth (data.rs:90), and a regression on a
//     non-trivial pair would still surface at the SystemParam construction
//     site of `Query<'w, 's, D, F>`.
assert_impl_all!(QueryView<'static, (), ()>: Send, Sync);

impl<'w, D: QueryData, F: QueryFilter> QueryView<'w, D, F> {
    /// **Internal constructor — called only from `EcsMaster::query`.**
    ///
    /// Bundles the world cell and the cache-slot pointer into the public
    /// handle. The `&mut EcsMaster` upstream of this call gates aliasing
    /// at the type level — the `world` cell carries write-capable
    /// provenance and the `state` pointer is unique for the duration of
    /// `'w`.
    ///
    /// # Safety
    ///
    /// * `world` MUST have been minted via
    ///   [`UnsafeEcsCell::new_mutable`] from the same `&mut EcsMaster` that
    ///   owns the `state` cache slot — i.e. both must descend from the
    ///   same exclusive borrow.
    /// * `state` MUST be a valid `NonNull<UnsafeCell<QueryDataState<D, F>>>`
    ///   pointing into the world's `query_state_cache` slot for
    ///   `<(D, F) as QueryTypeKey>::query_type_id()`.
    /// * The state pointed to MUST already have been updated (via
    ///   `QueryDataState::update`) against the world's archetype master
    ///   for the current `'w` scope.
    #[inline]
    pub(crate) unsafe fn from_parts(
        world: UnsafeEcsCell<'w>,
        state: NonNull<UnsafeCell<QueryDataState<D, F>>>,
    ) -> Self {
        Self {
            world,
            state,
            // Phase 22 D4: a freshly minted view carries no dynamic-tag
            // terms; `with_tag` / `without_tag` populate them.
            terms: TagTerms::EMPTY,
            _world_borrow: PhantomData,
            _data_filter_invariance: PhantomData,
        }
    }

    /// Adds a dynamic-tag presence term (Phase 22 D4): only archetypes
    /// carrying `tag` participate in every driver of this view
    /// (`iter`/`iter_mut`, `par_iter`/`par_iter_mut`, `for_each_chunk`,
    /// `par_for_each_chunk`, `get`/`get_mut`, `single`/`single_mut`,
    /// `archetype_count`/`is_empty`).
    ///
    /// Archetype-level filtering only — zero per-row cost; the no-terms fast
    /// path stays byte-identical (one predicted branch per archetype
    /// transition).
    ///
    /// # Panics
    /// Loud release panic past
    /// [`MAX_DYN_TAG_TERMS`](crate::ecs::core::iters::query::MAX_DYN_TAG_TERMS)
    /// combined `with_tag`/`without_tag` terms (setup-time, cold).
    #[must_use]
    #[inline]
    pub fn with_tag(mut self, tag: TagId) -> Self {
        self.terms.push_with(tag);
        self
    }

    /// Adds a dynamic-tag absence term (Phase 22 D4): archetypes carrying
    /// `tag` are excluded. Same cost model and panic contract as
    /// [`Self::with_tag`].
    #[must_use]
    #[inline]
    pub fn without_tag(mut self, tag: TagId) -> Self {
        self.terms.push_without(tag);
        self
    }

    /// Shared reborrow of the cached state.
    ///
    /// Used by every `QueryView` method that consumes the state by
    /// reference (`iter`, `single`, `get`, `par_iter`, etc.). Never
    /// produces a `&mut` — that is reserved for `EcsMaster::query`'s
    /// post-mint `state.update(master)` call.
    #[inline]
    fn state(&self) -> &QueryDataState<D, F> {
        // SAFETY (QV6 / I1 / I-NEW-3):
        //   - `self.state` is a valid `NonNull<UnsafeCell<...>>` per the
        //     `from_parts` contract.
        //   - `UnsafeCell::get` returns `*mut T`; we reborrow as `&T`
        //     only. The `&mut` retag inside `EcsMaster::query`'s
        //     `state.update(master)` is produced once per call via the
        //     `&mut self` uniqueness gate and is dropped before
        //     `from_parts` runs.
        //   - The `&` reborrow lifetime is bound to `&self` (sub-lifetime
        //     of `'w`); no aliasing `&mut` to the cache slot can exist
        //     concurrently because `&mut EcsMaster` produced this view.
        unsafe { &*(*self.state.as_ptr()).get() }
    }

    /// Returns the number of currently-matched archetypes.
    ///
    /// No-terms path: O(1) — reads the length of the cached `matched_ids`
    /// slice. With dynamic-tag terms: O(matched) signature-filtered walk
    /// (archetype-level membership only — `entity_count` is never consulted,
    /// matching the no-terms semantics; stale removed ids do not count on
    /// the term path, since the term test needs the live signature).
    #[inline]
    pub fn archetype_count(&self) -> usize {
        let state = self.state();
        let ids = state.archetype_state.matched_ids_pre_terms();
        if self.terms.is_empty() {
            return ids.len();
        }
        // SAFETY (U_C2): shared read mint — `world()` yields `&EcsMaster`
        //   scoped to this statement; no `&mut` access occurs through it
        //   (same pattern as `get`'s world reborrow).
        let master = unsafe { self.world.world().archetype_master() };
        count_term_matched(&self.terms, master, ids)
    }

    /// Returns `true` if no archetypes are currently matched.
    ///
    /// Same caveat as [`Query::is_empty`](super::query::Query::is_empty):
    /// an archetype-count of zero does not imply a zero-row iteration.
    ///
    /// Term semantics mirror [`Self::archetype_count`].
    #[inline]
    pub fn is_empty(&self) -> bool {
        let state = self.state();
        let ids = state.archetype_state.matched_ids_pre_terms();
        if self.terms.is_empty() {
            return ids.is_empty();
        }
        // SAFETY (U_C2): shared read mint scoped to this statement — see
        //   `archetype_count`.
        let master = unsafe { self.world.world().archetype_master() };
        !any_term_matched(&self.terms, master, ids)
    }

    /// Returns a read-only iterator over `D::Item<'_>` for every matched row.
    ///
    /// `D` must be [`ReadOnlyQueryData`]. For mutable iteration use
    /// [`Self::iter_mut`].
    ///
    /// # Meta plumbing (NCD7)
    ///
    /// Passes `SystemMeta::dummy()` as the cursor's meta argument. For
    /// `D::NEEDS_CHANGE_DETECTION == false` paths (the common `&T` /
    /// tuple-of-`&T` case) the NCD6 const-fold in `QueryIter::next` elides
    /// any `meta.last_run` / `meta.this_run` read; the dummy is touched
    /// only as a register-resident `&'static SystemMeta` reference.
    #[inline]
    pub fn iter(&self) -> QueryIter<'_, '_, D, F>
    where
        D: ReadOnlyQueryData,
    {
        // SAFETY (Q1, QD4, U_C2): `D: ReadOnlyQueryData` ⇒ no `&mut T` in
        //   `D`; `QueryIter::new` will call `cell.archetype_ptr(_)` and
        //   `D::set_table_readonly` only. The `world` cell is `Copy`;
        //   passing it by value preserves raw-pointer provenance.
        //   `SystemMeta::dummy()` returns a stable `'static` reference
        //   (NCD7); the NCD6 const-fold makes the dummy contents
        //   unobservable on the !NCD path. Phase 22 D4: `self.terms` is
        //   copied into the cursor — applied at each archetype transition.
        unsafe { QueryIter::new(self.state(), self.world, SystemMeta::dummy(), self.terms) }
    }

    /// Returns a mutable iterator over `D::Item<'_>` for every matched row.
    ///
    /// Accepts any `D: QueryData` (including `&mut T`). The `&mut self`
    /// borrow gates cursor uniqueness; no two `iter_mut` cursors can be
    /// live simultaneously.
    #[inline]
    pub fn iter_mut(&mut self) -> QueryIterMut<'_, '_, D, F> {
        // SAFETY (Q1, Q3, QD4, U_C3): `&mut self` enforces cursor
        //   uniqueness; the cell carries write-capable provenance from
        //   the upstream `&mut EcsMaster` (QV1). See `iter` SAFETY for
        //   meta plumbing rationale. Phase 22 D4: `self.terms` is copied
        //   into the cursor.
        unsafe { QueryIterMut::new(self.state(), self.world, SystemMeta::dummy(), self.terms) }
    }

    /// Returns a parallel read-only iteration handle.
    ///
    /// Same semantics as
    /// [`Query::par_iter`](super::query::Query::par_iter) — fans the work
    /// across the current `ThreadPool`'s workers via `pool.scope`; degrades
    /// to a sequential walk on the calling thread if no pool is attached
    /// (PAR7).
    #[inline]
    pub fn par_iter<'q>(&'q self) -> ParQuery<'q, 'q, D, F>
    where
        D: ReadOnlyQueryData,
    {
        ParQuery {
            state: self.state(),
            world: self.world,
            batching: BatchingStrategy::default(),
            meta: SystemMeta::dummy(),
            terms: self.terms,
        }
    }

    /// Returns a parallel mutable iteration handle.
    ///
    /// Same semantics as
    /// [`Query::par_iter_mut`](super::query::Query::par_iter_mut). The
    /// `&mut self` borrow gates cursor uniqueness.
    #[inline]
    pub fn par_iter_mut<'q>(&'q mut self) -> ParQueryMut<'q, 'q, D, F> {
        ParQueryMut {
            state: self.state(),
            world: self.world,
            batching: BatchingStrategy::default(),
            meta: SystemMeta::dummy(),
            terms: self.terms,
            _mut_marker: PhantomData,
        }
    }

    /// Returns the single matched row, panicking if the query yields
    /// anything other than exactly one row.
    ///
    /// `D` must be [`ReadOnlyQueryData`]. For the mutable variant use
    /// [`Self::single_mut`].
    ///
    /// # Panics
    ///
    /// Panics with a diagnostic message if the iterator yields zero rows
    /// or more than one row. Cold path — no overhead on the success path
    /// beyond the iteration itself.
    #[inline]
    pub fn single(&self) -> D::Item<'_>
    where
        D: ReadOnlyQueryData,
    {
        let mut iter = self.iter();
        let first = iter
            .next()
            .unwrap_or_else(|| query_view_single_panic_empty::<D, F>());
        if iter.next().is_some() {
            query_view_single_panic_many::<D, F>();
        }
        first
    }

    /// Returns the single matched mutable row, panicking if the query
    /// yields anything other than exactly one row.
    ///
    /// Accepts any `D: QueryData`.
    ///
    /// # Panics
    ///
    /// Same as [`Self::single`] — zero or many rows trip a cold panic.
    #[inline]
    pub fn single_mut(&mut self) -> D::Item<'_> {
        let mut iter = self.iter_mut();
        let first = iter
            .next()
            .unwrap_or_else(|| query_view_single_panic_empty::<D, F>());
        if iter.next().is_some() {
            query_view_single_panic_many::<D, F>();
        }
        first
    }

    /// Returns the row corresponding to `entity` if it is alive AND lives
    /// in a matched archetype.
    ///
    /// `D` must be [`ReadOnlyQueryData`]. For the mutable variant use
    /// [`Self::get_mut`].
    ///
    /// # Cost
    ///
    /// O(1) entity-master lookup + a single archetype dispatch. The
    /// matched-set membership check is an `ArchetypeBitSet::contains`
    /// (one load + bit test).
    pub fn get(&self, entity: Entity) -> Option<D::Item<'_>>
    where
        D: ReadOnlyQueryData,
    {
        let state = self.state();
        // SAFETY (U_C2): cell scoped to '_; `world()` returns a shared
        //   reborrow of the EcsMaster.
        let ecs = unsafe { self.world.world() };
        let inland = ecs.entity_master.entities_inland.get(entity.id().0)?;
        if inland.is_null() {
            return None;
        }
        if inland.generation() != entity.generation() {
            return None;
        }
        // SAFETY (U1, U2, U11, F1): archetype_ptr was minted via the bundle's
        //   `UnsafeCell::raw_get` helper at register time; slab heap address is
        //   stable for `'w`, and the pointer is interior-mutable
        //   (`SharedReadWrite`, F4-rooted) so it survives sibling structural
        //   writes under TB/SB (whole slab element is `UnsafeCell`-wrapped).
        let arch_ptr: *const _ = inland.archetype_ptr();
        let arch_ref = unsafe { &*arch_ptr };
        // Membership check — the bitset is the dedup-mirror of matched_ids.
        let bitset = state.archetype_state.matched_archetypes_bitset();
        if !bitset.contains(arch_ref.id().0) {
            return None;
        }
        // Phase 22 D4: per-entity term test on the in-hand archetype ref —
        // ≤ 8 signature bit tests; `len == 0` is one predicted branch.
        if !archetype_passes_tag_terms(&self.terms, arch_ref) {
            return None;
        }
        let row = inland.unit_index() as usize;
        let mut data_fetch = <D as QueryData>::init_fetch(&state.data_state);
        // SAFETY (QD3, QD4): read-only mint via the raw pointer; the
        //   archetype was matched by `D::matches_component_set` (post-filter
        //   guarantee), so every cached column is non-null. `row` is the
        //   live `unit_index` from the fast store — strictly < entity_count.
        unsafe {
            <D as QueryData>::set_table_readonly(
                &mut data_fetch,
                &state.data_state,
                arch_ptr,
                SystemMeta::dummy(),
            );
        }
        // SAFETY (QD2, QD3): set_table_readonly cached the column pointers;
        //   `row < entity_count` per the fast store invariant.
        Some(unsafe { <D as QueryData>::fetch(&data_fetch, row) })
    }

    /// Returns the mutable row corresponding to `entity` if it is alive AND
    /// lives in a matched archetype.
    ///
    /// Accepts any `D: QueryData`.
    pub fn get_mut(&mut self, entity: Entity) -> Option<D::Item<'_>> {
        let state = self.state();
        // SAFETY (U_C2): cell scoped to '_; `world()` returns a shared
        //   reborrow of the EcsMaster. The eventual `archetype_ptr_mut`
        //   call below upgrades to a write-capable raw pointer via the
        //   cell's own retag-free path (cell carries write-capable
        //   provenance per QV1).
        let ecs = unsafe { self.world.world() };
        let inland_copy = *ecs.entity_master.entities_inland.get(entity.id().0)?;
        if inland_copy.is_null() {
            return None;
        }
        if inland_copy.generation() != entity.generation() {
            return None;
        }
        // SAFETY (U1, U2, U11, U14, F1): the inland-cached `archetype_ptr` was
        //   minted with write-capable, interior-mutable (`SharedReadWrite`,
        //   F4-rooted) provenance via the bundle's `UnsafeCell::raw_get` helper
        //   at register time (Phase 7 W7); slab heap address is stable for
        //   `'w`; the pointer survives sibling structural writes under TB/SB
        //   (whole slab element is `UnsafeCell`-wrapped); `&mut self` upstream
        //   of this view forbids any aliasing `&` to the same archetype slot.
        let arch_ptr: *mut _ = inland_copy.archetype_ptr();
        let arch_ref = unsafe { &*arch_ptr };
        let bitset = state.archetype_state.matched_archetypes_bitset();
        if !bitset.contains(arch_ref.id().0) {
            return None;
        }
        // Phase 22 D4: per-entity term test on the in-hand archetype ref —
        // mirrors `get`.
        if !archetype_passes_tag_terms(&self.terms, arch_ref) {
            return None;
        }
        let row = inland_copy.unit_index() as usize;
        let mut data_fetch = <D as QueryData>::init_fetch(&state.data_state);
        // SAFETY (QD3, QD4): write-capable mint; archetype matched.
        unsafe {
            <D as QueryData>::set_table_mut(
                &mut data_fetch,
                &state.data_state,
                arch_ptr,
                SystemMeta::dummy(),
            );
        }
        // SAFETY (QD2, QD3): set_table_mut cached the column pointers;
        //   `row` in range per fast store invariant.
        Some(unsafe { <D as QueryData>::fetch(&data_fetch, row) })
    }

    /// Direct-API mirror of
    /// [`Query::for_each_chunk`](super::query::Query::for_each_chunk).
    /// Invokes `f` once per matched archetype, passing a slice (or tuple of
    /// slices) covering every row in that archetype.
    ///
    /// `D` must satisfy [`ChunkedQueryData`]; `F` must satisfy
    /// [`ArchetypalQueryFilter`]. Both bounds are compile-time —
    /// `QueryView<&T, Changed<U>>::for_each_chunk` is a type error. Use
    /// [`Self::iter`] / [`Self::iter_mut`] for per-row change-detection
    /// flows; the direct API explicitly rejects change-detection filters at
    /// `EcsMaster::query` mint time anyway (QV11 / W4).
    ///
    /// # Performance
    ///
    /// Identical cost model to
    /// [`Query::for_each_chunk`](super::query::Query::for_each_chunk) — see
    /// plan §1.2. Empty matched archetypes are skipped at the
    /// `entity_count == 0` guard; stale-id entries (Q5) are skipped
    /// transparently via the driver's `archetype_ptr(_mut)` `None` arm.
    ///
    /// # See also
    ///
    /// * [`Query::for_each_chunk`](super::query::Query::for_each_chunk)
    ///   — SystemParam mirror used inside system bodies.
    /// * [`Self::par_for_each_chunk`] — parallel variant (Phase X.A Wave 6).
    ///
    /// [`ChunkedQueryData`]: super::chunked_data::ChunkedQueryData
    /// [`ArchetypalQueryFilter`]: super::filter::ArchetypalQueryFilter
    #[inline]
    pub fn for_each_chunk<Func>(&mut self, f: Func)
    where
        D: ChunkedQueryData,
        F: ArchetypalQueryFilter,
        Func: for<'c> FnMut(D::ChunkItem<'c>),
    {
        // SAFETY (Q1, Q3, CD1-CD4): mirrors `Query::for_each_chunk`.
        //   `&mut self` enforces cursor uniqueness on the view;
        //   `D::IS_READ_ONLY` selects the readonly / mut chunk-dispatch
        //   arm inside the driver. `QueryView` does not carry `meta` —
        //   `NEEDS_CHANGE_DETECTION` const-folds to `false` at this
        //   monomorphisation because `D: ChunkedQueryData` excludes
        //   `Ref<T>` / `Mut<T>` and `F: ArchetypalQueryFilter` excludes
        //   `Added<C>` / `Changed<C>`, so the meta-bearing branch from
        //   `iter.rs` does not appear in this driver. The cell
        //   `self.world` is `Copy`; passing by value preserves the
        //   raw-pointer provenance through the call (Phase 8a C1 fix).
        let mutable = !D::IS_READ_ONLY;
        unsafe {
            chunk_iter::for_each_chunk_impl(self.state(), self.world, mutable, &self.terms, f);
        }
    }

    /// Direct-API mirror of
    /// [`Query::par_for_each_chunk`](super::query::Query::par_for_each_chunk).
    /// Splits each matched archetype's row range into sub-ranges per
    /// [`BatchingStrategy`] and dispatches each sub-range to a
    /// [`boyko_threadpool::ThreadPool`] worker via
    /// [`boyko_threadpool::ThreadPool::scope`]. Archetypes with fewer than
    /// [`MIN_ARCHETYPE_FOR_PARALLEL`][min] rows run inline on the calling
    /// thread (PAR9). PAR7 fallback (no active pool → sequential walk)
    /// preserved.
    ///
    /// # Closure invocation frequency
    ///
    /// Identical semantics to
    /// [`Query::par_for_each_chunk`](super::query::Query::par_for_each_chunk):
    /// the closure fires once per archetype sub-range, NOT once per archetype.
    /// See the linked method for the per-regime worked examples and the
    /// thread-safe-accumulator pattern for reductions.
    ///
    /// # Compile-time bounds
    ///
    /// `D` must satisfy [`ChunkedQueryData`]; `F` must satisfy
    /// [`ArchetypalQueryFilter`]; `Func` must be `Fn + Send + Sync`. The
    /// direct-API change-detection guard at `EcsMaster::query` mint time
    /// (QV11 / W4) is subsumed by these bounds — `Ref<T>` / `Mut<T>` /
    /// `Added<C>` / `Changed<C>` cannot reach this method.
    ///
    /// # See also
    ///
    /// * [`Query::par_for_each_chunk`](super::query::Query::par_for_each_chunk)
    ///   — SystemParam mirror used inside system bodies.
    /// * [`Self::for_each_chunk`] — sequential variant.
    ///
    /// [`ChunkedQueryData`]: super::chunked_data::ChunkedQueryData
    /// [`ArchetypalQueryFilter`]: super::filter::ArchetypalQueryFilter
    /// [`BatchingStrategy`]: super::par_iter::BatchingStrategy
    /// [min]: super::par_iter::MIN_ARCHETYPE_FOR_PARALLEL
    #[inline]
    pub fn par_for_each_chunk<Func>(&mut self, f: Func, batching: BatchingStrategy)
    where
        D: ChunkedQueryData,
        F: ArchetypalQueryFilter,
        Func: for<'c> Fn(D::ChunkItem<'c>) + Send + Sync,
    {
        // SAFETY (Q1, Q3, CD1-CD4, §9): mirrors `Query::par_for_each_chunk`.
        //   `&mut self` enforces cursor uniqueness on the view (QV1 plus the
        //   `&mut EcsMaster` borrow that produced the cache slot upstream).
        //   `D::IS_READ_ONLY` selects the readonly / mut chunk-dispatch arm.
        //   `QueryView` carries no `meta` — the `ChunkedQueryData` /
        //   `ArchetypalQueryFilter` gates force NCD const-fold to `false`,
        //   so the meta-bearing branch is unreachable. The cell `self.world`
        //   is `Copy`; passing by value preserves raw-pointer provenance.
        let mutable = !D::IS_READ_ONLY;
        unsafe {
            par_chunk::par_for_each_chunk_impl(
                self.state(),
                self.world,
                mutable,
                batching,
                &self.terms,
                f,
            );
        }
    }
}

/// Cold panic site for [`QueryView::single`] when the iterator yields zero
/// rows. `#[cold] + #[inline(never)]` so it lives outside the hot path's
/// instruction cache.
#[cold]
#[inline(never)]
fn query_view_single_panic_empty<D: QueryData, F: QueryFilter>() -> ! {
    panic!(
        "QueryView::single<{}, {}>(): query yielded zero rows; \
         expected exactly one",
        std::any::type_name::<D>(),
        std::any::type_name::<F>(),
    );
}

/// Cold panic site for [`QueryView::single`] when the iterator yields more
/// than one row.
#[cold]
#[inline(never)]
fn query_view_single_panic_many<D: QueryData, F: QueryFilter>() -> ! {
    panic!(
        "QueryView::single<{}, {}>(): query yielded more than one row; \
         expected exactly one",
        std::any::type_name::<D>(),
        std::any::type_name::<F>(),
    );
}

#[cfg(test)]
mod tests {
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
    use crate::ecs::identifiers::primitives::ComponentId;

    // Component id reserved for the Phase X.A Wave 5 QueryView chunk test.
    // The free slot below was verified at write time against existing
    // crate-wide allocations:
    //   * 400-422 — archetype.rs / archetype_bundle.rs / component_pool_bundle.rs
    //   * 450-456 — component_registry TEST_BASE+0..+6
    //   * 457-461 — component_registry "reserved" + Phase X.A Wave 4 (460-461)
    //   * 462    — component_registry collision_with_different_type test
    //   * 465    — component_registry collision_with_same_type test
    //   * 480-482 — archetype_bundle miri tests
    //   * 483-485 — query/iter.rs
    //   * 486-488 — query/query.rs
    //   * 490-497 — query_state / component_set
    //   * 503-504 — query/data.rs
    //   * 506-510 — query/state.rs / resource_registry
    // Slot 463 is free (between the collision-different and collision-same
    // anchors at 462 / 465, both inside the component_registry 450-465 zone).
    const COMP_A: ComponentId = ComponentId(463);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompA(u32);

    impl Component for CompA {
        fn component_id() -> ComponentId {
            COMP_A
        }
    }

    /// Idempotent registry priming.
    fn register_test_components() {
        component_registry::register_layout::<CompA>(COMP_A.0);
    }

    /// Mirror of `chunk_iter::tests::sequential_single_archetype_yields_full_slice`
    /// but routed through `ecs.query::<&CompA>().for_each_chunk(...)`. Verifies
    /// the direct-API entry point hits the same driver (exactly one closure
    /// invocation; the slice covers every row).
    #[test]
    fn query_view_for_each_chunk_yields_full_slice() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[COMP_A]);

        for i in 0..10u32 {
            let comp = CompA(i + 500);
            // SAFETY: `CompA` is `#[repr(C)]` POD; reading its bytes
            //   produces a valid byte slice for the duration of this call.
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    &comp as *const CompA as *const u8,
                    std::mem::size_of::<CompA>(),
                )
            };
            ecs.create_entity(arch, &[(COMP_A, bytes)])
                .expect("query_view_for_each_chunk: create_entity must succeed");
        }

        let mut invocations = 0usize;
        let mut collected: Vec<u32> = Vec::with_capacity(10);
        {
            let mut view = ecs.query::<&CompA, ()>();
            view.for_each_chunk(|slice: &[CompA]| {
                invocations += 1;
                for c in slice {
                    collected.push(c.0);
                }
            });
        }

        assert_eq!(
            invocations, 1,
            "single archetype ⇒ exactly one closure invocation",
        );
        assert_eq!(collected.len(), 10, "slice must cover every row");
        for expected in 500..510u32 {
            assert!(
                collected.contains(&expected),
                "row {expected} must appear in collected = {collected:?}",
            );
        }
    }
}
