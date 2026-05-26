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

use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::iters::query::data::{QueryData, ReadOnlyQueryData};
use crate::ecs::core::iters::query::filter::QueryFilter;
use crate::ecs::core::iters::query::iter::{QueryIter, QueryIterMut};
use crate::ecs::core::iters::query::par_iter::{BatchingStrategy, ParQuery, ParQueryMut};
use crate::ecs::core::iters::query::state::QueryDataState;
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
/// # Layout (§10.2)
///
/// 16 B total — `UnsafeEcsCell<'w>` (8 B) + `NonNull<UnsafeCell<...>>` (8 B)
/// + ZST `PhantomData`. Single cache-line addressable on x86_64.
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
            _world_borrow: PhantomData,
            _data_filter_invariance: PhantomData,
        }
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
    /// O(1) — reads the length of the cached `matched_ids` slice.
    #[inline]
    pub fn archetype_count(&self) -> usize {
        self.state().archetype_state.matched_ids().len()
    }

    /// Returns `true` if no archetypes are currently matched.
    ///
    /// Same caveat as [`Query::is_empty`](super::query::Query::is_empty):
    /// an archetype-count of zero does not imply a zero-row iteration.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.state().archetype_state.matched_ids().is_empty()
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
        //   unobservable on the !NCD path.
        unsafe { QueryIter::new(self.state(), self.world, SystemMeta::dummy()) }
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
        //   meta plumbing rationale.
        unsafe { QueryIterMut::new(self.state(), self.world, SystemMeta::dummy()) }
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
        // SAFETY (U1, U2, U11): archetype_ptr was minted via raw arithmetic
        //   from the bundle slab at register time; slab heap address is
        //   stable for `'w`.
        let arch_ptr: *const _ = inland.archetype_ptr();
        let arch_ref = unsafe { &*arch_ptr };
        // Membership check — the bitset is the dedup-mirror of matched_ids.
        let bitset = state.archetype_state.matched_archetypes_bitset();
        if !bitset.contains(arch_ref.id().0) {
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
        // SAFETY (U1, U2, U11, U14): the inland-cached `archetype_ptr` was
        //   minted with write-capable provenance under `&mut EcsMaster` at
        //   register time (Phase 7 W7); slab heap address is stable for
        //   `'w`; `&mut self` upstream of this view forbids any aliasing
        //   `&` to the same archetype slot.
        let arch_ptr: *mut _ = inland_copy.archetype_ptr();
        let arch_ref = unsafe { &*arch_ptr };
        let bitset = state.archetype_state.matched_archetypes_bitset();
        if !bitset.contains(arch_ref.id().0) {
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
