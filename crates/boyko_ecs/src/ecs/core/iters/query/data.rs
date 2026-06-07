//! `QueryData` trait — typed component access for queries.
//!
//! Step 2 lands the trait body and the two leaf impls (`&T`, `&mut T`). The
//! variadic tuple impls follow in Step 4; the per-row access in iterators
//! follows in Step 7.
//!
//! See Phase 8b plan §4 for the full design rationale.

use std::cell::UnsafeCell;
use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::params::diagnostics::intra_system_conflict_panic;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::identifiers::primitives::ComponentId;

/// Maximum tuple arity supported by [`QueryData`] variadic impls.
///
/// Tuples beyond this arity trip a `panic!()` in Step 4 — the limit
/// keeps macro expansion bounded and the I-cache budget honest.
pub const MAX_QUERY_DATA_ARITY: usize = 12;

/// Per-row data fetched by a `Query<D, F>`.
///
/// Implemented by:
/// * `&T` for any `T: Component` — yields `&'w T` per row.
/// * `&mut T` for any `T: Component` — yields `&'w mut T` per row.
/// * Tuples of `QueryData` up to arity 12 (Step 4) — yields a tuple of
///   element items.
///
/// # Trait shape — three GATs
///
/// * `State` — long-lived per-system caches (e.g. cached [`ComponentId`]s).
///   `Send + Sync + 'static` for Phase 9 cross-thread migration.
/// * `Fetch<'w>` — per-archetype cached column pointers. Held inside the
///   iterator (not the query), so re-iter is sound. `Copy` so the variadic
///   tuple impl can destructure cleanly.
/// * `Item<'w>` — the per-row yielded value (e.g. `&'w T` or `&'w mut T`).
///
/// # Split `set_table` (M2)
///
/// [`QueryData::set_table_readonly`] / [`QueryData::set_table_mut`] are split
/// into two kind-correct methods:
///
/// * `set_table_readonly(_: *const Archetype)` — called by `QueryIter::next`
///   when the cursor is read-only. Never produces write-capable provenance
///   downstream.
/// * `set_table_mut(_: *mut Archetype)` — called by `QueryIterMut::next`
///   when the cursor is mutable. Pointer carries write-capable provenance
///   (minted via `UnsafeEcsCell::archetype_ptr_mut`).
///
/// For read-only `QueryData` (`&T` and tuples of read-only), `set_table_mut`
/// is implemented as `unsafe { set_table_readonly(fetch, state, archetype as *const _) }`
/// — i.e., for read-only data, the two paths converge to the same code. For
/// `&mut T`, `set_table_readonly` is forbidden: the impl `panic!()`s at
/// runtime (would be `unreachable!()` in release; the read-only cursor
/// never calls it because the type-level `D: ReadOnlyQueryData` bound on
/// `Query::iter()` rules out `&mut T`).
///
/// # `IS_READ_ONLY` const
///
/// Compile-time flag for read-vs-write classification. `&T` and tuples of
/// read-only data have `IS_READ_ONLY = true`; `&mut T` has `false`. Used by
/// `Query::iter()` to gate read-only iteration (Q1).
///
/// # Safety
///
/// Implementations MUST uphold:
///
/// 1. **QD1** — `init_state` produces a `State` whose embedded
///    [`ComponentId`]s cover every component that `fetch(row)` will read or
///    write. Reflected in `init_access`.
/// 2. **QD2** — `init_fetch` produces a `Fetch<'w>` with all column pointers
///    NULL. Exactly one of `set_table_readonly` / `set_table_mut` overwrites
///    them with valid pointers before any `fetch(row)` call.
/// 3. **QD3** — `Fetch<'w>` lifetime is bound to `'w`; cached pointers are
///    scoped to the `*const/*mut Archetype` minted by `UnsafeEcsCell` for `'w`.
/// 4. **QD4** — `QueryIter::next` calls only `set_table_readonly`;
///    `QueryIterMut::next` calls only `set_table_mut`. The split signature
///    structurally prevents the wrong-kind dispatch.
pub unsafe trait QueryData: Sized {
    /// Long-lived per-system cache (e.g. cached [`ComponentId`]s). Populated
    /// once by [`Self::init_state`] and consumed unchanged by every
    /// per-archetype `set_table_*` call.
    type State: Send + Sync + 'static;

    /// Per-archetype scratch fetched once when the iterator transitions to a
    /// new archetype. Held by `QueryIter`/`QueryIterMut`, not by the query
    /// itself, so re-iteration is sound.
    type Fetch<'w>: Copy;

    /// Per-row item yielded from the iterator (e.g. `&'w T`, `&'w mut T`, or
    /// a tuple of items).
    type Item<'w>;

    /// `true` iff every component the impl touches is read-only.
    const IS_READ_ONLY: bool;

    /// Phase 12.5 Track B NCD1 — compile-time flag for change-detection use.
    ///
    /// `true` iff this `QueryData` reads or writes per-row tick fields
    /// (`Ref<T>`, `Mut<T>`, etc.). The dispatcher's NCD6 const-fold
    /// branches on this flag at monomorphisation: `false` impls dispatch
    /// to the `_no_meta` variant of `set_table_*` and never load
    /// `meta.last_run` / `meta.this_run`; `true` impls dispatch to the
    /// meta-bearing variant.
    ///
    /// Default: NONE — every impl MUST declare. The plan's NCD5 / I4
    /// invariant forbids a default body because a silent fallthrough on
    /// a future `Ref<T>`-equivalent impl would compile cleanly while
    /// quietly disabling change detection — caught at compile time by
    /// the lack-of-default-impl forcing explicit declaration.
    const NEEDS_CHANGE_DETECTION: bool;

    /// Builds the per-system [`Self::State`].
    ///
    /// Called once per `(system, world)` pair at registration time. Performs
    /// any [`Component::component_id`] cache priming so the hot path skips
    /// the `OnceLock` load.
    fn init_state(world: &mut EcsMaster) -> Self::State;

    /// Declares the read/write surface of this data against the active
    /// system's [`FilteredAccessSet`]. Surfaces intra-system aliasing
    /// conflicts as `boyko-B0002` panics (cold path).
    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet);

    /// Archetype-level inclusion predicate. Returns `true` iff the archetype
    /// with `mask` contains every component this `QueryData` touches.
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool;

    /// Accumulates this data's required component bits into `include`. Used
    /// by `QueryDataState` to build the `include` mask that drives archetype
    /// matching (M1).
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask);

    /// Builds a fresh [`Self::Fetch`] with NULL column pointers (QD2). Must
    /// be paired with a subsequent `set_table_readonly` / `set_table_mut`
    /// before any [`Self::fetch`] call.
    fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w>;

    /// Sets the `Fetch`'s cached column pointers from a read-only archetype
    /// pointer. Called by `QueryIter::next` (the read-only cursor).
    ///
    /// # Phase 10 Round 2 W7 — `meta` parameter
    ///
    /// `meta` carries the active system's per-frame tick snapshot
    /// (`last_run` / `this_run`). Wave C `Ref<T>` / `Mut<T>` impls copy
    /// the ticks by value into `Self::Fetch<'w>`. Leaf `&T` / `&mut T`
    /// impls accept and ignore. **`meta` is read-only INPUT** — never
    /// stored into the `Fetch` (different lifetime domain).
    ///
    /// # Safety
    ///
    /// * `archetype` MUST be a live `*const Archetype` for `'w`, with
    ///   provenance from `UnsafeEcsCell::archetype_ptr(id)` (read-only mint).
    /// * `archetype` MUST contain every [`ComponentId`] in `state`.
    /// * For `D` containing `&mut T`, this method MUST NOT be called. Impls
    ///   for `&mut T` `panic!()` here as a runtime backstop; the type-level
    ///   `D: ReadOnlyQueryData` bound on `Query::iter()` prevents this in
    ///   well-typed code.
    /// * `meta` MUST reference the currently-active system's
    ///   [`SystemMeta`].
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        meta: &'_ SystemMeta,
    );

    /// Sets the `Fetch`'s cached column pointers from a write-capable
    /// archetype pointer. Called by `QueryIterMut::next` (the mutable cursor).
    /// See [`Self::set_table_readonly`] for the `meta` contract.
    ///
    /// # Safety
    ///
    /// * `archetype` MUST be a live `*mut Archetype` for `'w`, with
    ///   write-capable provenance from `UnsafeEcsCell::archetype_ptr_mut(id)`.
    /// * `archetype` MUST contain every [`ComponentId`] in `state`.
    /// * `meta` MUST reference the currently-active system's
    ///   [`SystemMeta`].
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,
    );

    /// Phase 12.5 Track B NCD5 — meta-free variant of
    /// [`Self::set_table_readonly`].
    ///
    /// Dispatched by [`QueryIter::next`](crate::ecs::core::iters::query::iter::QueryIter)
    /// / [`for_each_impl`](crate::ecs::core::iters::query::par_iter)
    /// when `Self::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION
    /// == false` (NCD6 const-fold). Identical to `set_table_readonly`
    /// minus the `meta` parameter — the dispatcher avoids loading the
    /// meta when no downstream code reads it.
    ///
    /// **NO DEFAULT BODY** (I4): every impl must declare the body
    /// explicitly. For `NEEDS_CHANGE_DETECTION = false` impls the body
    /// duplicates `set_table_readonly` minus the unused `_meta`. For
    /// `NEEDS_CHANGE_DETECTION = true` impls the body is a `#[cold]`
    /// panic — the dispatcher should never reach the no-meta variant
    /// when the const is true.
    ///
    /// # Safety
    ///
    /// Same contract as [`Self::set_table_readonly`] minus the `meta`
    /// invariant. The dispatcher upholds NCD6: this method is reached
    /// only when `Self::NEEDS_CHANGE_DETECTION == false`.
    unsafe fn set_table_readonly_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    );

    /// Phase 12.5 Track B NCD5 — meta-free variant of
    /// [`Self::set_table_mut`]. See [`Self::set_table_readonly_no_meta`]
    /// for the dispatcher contract and the no-default-body rationale.
    ///
    /// # Safety
    ///
    /// Same contract as [`Self::set_table_mut`] minus the `meta`
    /// invariant.
    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    );

    /// Returns the per-row value for `row`.
    ///
    /// # Safety
    ///
    /// * `fetch` MUST have been initialised by a prior `set_table_*` call.
    /// * `row < archetype.entity_count()` of the archetype that `set_table_*`
    ///   cached.
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w>;
}

/// Marker trait for [`QueryData`] that performs no writes.
///
/// `Query::iter()` (read-only iteration) is gated on this bound so the type
/// system prevents `&mut T` from being iterated through the read cursor.
///
/// # Safety
///
/// Implementations MUST be [`QueryData`] impls whose `IS_READ_ONLY = true`.
pub unsafe trait ReadOnlyQueryData: QueryData {}

// ── `&T: QueryData + ReadOnlyQueryData` impl (§4.2) ────────────────────────

/// Per-system state for `&T: QueryData`. Caches the [`ComponentId`] minted by
/// `T::component_id()` so the hot path skips the per-call `OnceLock` load.
///
/// `Copy` because the state carries only the id and a zero-sized
/// `PhantomData`. `PhantomData<fn() -> T>` keeps `T` invariant without
/// imposing `Send + Sync` constraints from `T` itself onto the state.
///
/// `Copy` / `Clone` are implemented manually so the auto-derive does not
/// synthesise an unwanted `T: Copy` blanket bound.
pub struct ReadState<T: Component> {
    /// Cached component id; bound to `T` by construction.
    pub(crate) id: ComponentId,
    /// Type binding without forcing `T: Send + Sync` onto the state.
    pub(crate) _marker: PhantomData<fn() -> T>,
}

impl<T: Component> Clone for ReadState<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Component> Copy for ReadState<T> {}

/// Per-archetype fetch scratch for `&T: QueryData`. Holds the base pointer to
/// the active archetype's column for component `T`; the per-row item is
/// `&*base.add(row)`.
///
/// `Copy` / `Clone` are implemented manually because the auto-derive would
/// synthesise a `T: Copy` blanket bound (driven by `PhantomData<&'w T>`'s
/// derive heuristic) that is not actually required for this struct's
/// representation.
pub struct ReadFetch<'w, T: Component> {
    /// Base pointer to the current archetype's column for `T`. NULL until
    /// `set_table_readonly` / `set_table_mut` runs (QD2).
    pub(crate) base: *const T,
    /// Type binding tying the fetch lifetime to `'w`.
    pub(crate) _marker: PhantomData<&'w T>,
}

impl<T: Component> Clone for ReadFetch<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Component> Copy for ReadFetch<'_, T> {}

// SAFETY (QD1-QD4):
//   - QD1: `state.id` is `T::component_id()`; `init_access` declares a read.
//   - QD2: `init_fetch` sets `base = ptr::null()`; either `set_table_readonly`
//     or `set_table_mut` overwrites before any `fetch` call. (Note: for `&T`,
//     both methods do the same thing — read `column.ptr` as `*const T`.)
//   - QD3: `Fetch<'w>` lifetime is `'w` via `PhantomData<&'w T>`.
//   - QD4: both `set_table_*` methods share the same body (read-only data
//     does not care about the pointer kind); the split exists for the
//     mutable case in `&mut T`.
unsafe impl<T: Component> QueryData for &T {
    type State = ReadState<T>;
    type Fetch<'w> = ReadFetch<'w, T>;
    type Item<'w> = &'w T;
    const IS_READ_ONLY: bool = true;
    // Phase 12.5 Track B NCD2: `&T` reads no per-row ticks.
    const NEEDS_CHANGE_DETECTION: bool = false;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        ReadState {
            id: T::component_id(),
            _marker: PhantomData,
        }
    }

    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        access_set
            .add_component_read(state.id, std::any::type_name::<Self>())
            .unwrap_or_else(|conflict| intra_system_conflict_panic(conflict));
    }

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        ReadFetch {
            base: std::ptr::null(),
            _marker: PhantomData,
        }
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY (QD3): `archetype` is a live `*const Archetype` for `'w`
        //   (caller contract of this `unsafe fn`); `columns` is at offset 0
        //   per Phase 7 D4; `state.id.0 < MAX_COMPONENTS` by construction of
        //   the cached id.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "QD2: column was unexpectedly null");
        fetch.base = column.ptr as *const T;
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,
    ) {
        // For `&T`, the mutable variant degrades to the same read. Re-borrow
        // as `*const` internally; no write-capable provenance is consumed.
        // SAFETY (QD3, QD4): same conditions as `set_table_readonly` with the
        //   additional caller guarantee that `archetype` carries fresh
        //   `archetype_ptr_mut` provenance — strictly stronger than what we
        //   need here.
        unsafe { Self::set_table_readonly(fetch, state, archetype as *const _, meta) }
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    ) {
        // Meta-free re-implementation — identical body to
        // `set_table_readonly` minus the unused `_meta` arg (NCD = false).
        // SAFETY (QD3): same as `set_table_readonly`.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "QD2: column was unexpectedly null");
        fetch.base = column.ptr as *const T;
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // `&T` degrades to read; forward to the readonly path.
        // SAFETY (QD3, QD4): `archetype` carries strictly-stronger
        //   write-capable provenance than the read-only path requires.
        unsafe { Self::set_table_readonly_no_meta(fetch, state, archetype as *const _) }
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        // SAFETY (QD2, QD3): `set_table_*` was called before any `fetch`
        //   (caller contract), so `fetch.base` is non-null and points at the
        //   archetype's column for `T`. `row < entity_count` (caller
        //   contract). The returned `&'w T` lifetime is tied to `'w` via
        //   `PhantomData<&'w T>` in `ReadFetch`.
        unsafe { &*fetch.base.add(row) }
    }
}

// SAFETY: `&T: QueryData` has `IS_READ_ONLY = true`; the impl never writes.
unsafe impl<T: Component> ReadOnlyQueryData for &T {}

// ── `&mut T: QueryData` impl (§4.3) ────────────────────────────────────────

/// Per-system state for `&mut T: QueryData`. Same shape as [`ReadState<T>`] —
/// only the trait-impl side differs (write vs. read in `init_access`, mutable
/// pointer in `set_table_mut`/`fetch`).
///
/// `Copy` / `Clone` are implemented manually so the auto-derive does not
/// synthesise an unwanted `T: Copy` blanket bound.
pub struct WriteState<T: Component> {
    /// Cached component id; bound to `T` by construction.
    pub(crate) id: ComponentId,
    /// Type binding without forcing `T: Send + Sync` onto the state.
    pub(crate) _marker: PhantomData<fn() -> T>,
}

impl<T: Component> Clone for WriteState<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Component> Copy for WriteState<T> {}

/// Per-archetype fetch scratch for `&mut T: QueryData`. Holds the
/// write-capable base pointer to the active archetype's column for `T`; the
/// per-row item is `&mut *base.add(row)`.
///
/// `Copy` / `Clone` are implemented manually because the auto-derive would
/// synthesise a `T: Copy` blanket bound (driven by `PhantomData<&'w mut T>`'s
/// derive heuristic) that is not actually required.
pub struct WriteFetch<'w, T: Component> {
    /// Base pointer to the current archetype's column for `T`. NULL until
    /// `set_table_mut` runs (QD2). The provenance is write-capable because
    /// `set_table_mut` receives a `*mut Archetype` minted by
    /// `UnsafeEcsCell::archetype_ptr_mut` (Phase 7 U7).
    pub(crate) base: *mut T,
    /// Type binding tying the fetch lifetime to `'w`.
    pub(crate) _marker: PhantomData<&'w mut T>,
}

impl<T: Component> Clone for WriteFetch<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Component> Copy for WriteFetch<'_, T> {}

// SAFETY (QD1-QD4):
//   - QD1: `state.id` is `T::component_id()`; `init_access` declares a WRITE.
//   - QD2: `set_table_mut` overwrites `base`; `set_table_readonly` panics
//     (QD4 runtime backstop).
//   - QD3: lifetime bound by `PhantomData<&'w mut T>` in `WriteFetch`.
//   - QD4: `set_table_readonly` is forbidden for `&mut T` — the type system
//     prevents the call (`Query::iter()` requires `D: ReadOnlyQueryData`,
//     which `&mut T` does not implement). The runtime `panic!()` is a
//     defence-in-depth backstop, expected to be `unreachable_unchecked!()`
//     in a future phase after Miri verification.
unsafe impl<T: Component> QueryData for &mut T {
    type State = WriteState<T>;
    type Fetch<'w> = WriteFetch<'w, T>;
    type Item<'w> = &'w mut T;
    const IS_READ_ONLY: bool = false;
    // Phase 12.5 Track B NCD2: `&mut T` writes the underlying value but
    // does NOT consult per-row tick fields (the tick bump for `&mut T`
    // queries lives in `Mut<T>`'s deref guard, not here).
    const NEEDS_CHANGE_DETECTION: bool = false;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        WriteState {
            id: T::component_id(),
            _marker: PhantomData,
        }
    }

    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        access_set
            .add_component_write(state.id, std::any::type_name::<Self>())
            .unwrap_or_else(|conflict| intra_system_conflict_panic(conflict));
    }

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        WriteFetch {
            base: std::ptr::null_mut(),
            _marker: PhantomData,
        }
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // QD4: the read-only cursor calling on `&mut T` data is forbidden by
        // the trait gate `D: ReadOnlyQueryData` on `Query::iter()`. Reaching
        // this branch indicates a contract violation by a hand-written
        // `QueryData` impl (it implemented `ReadOnlyQueryData` for a type
        // containing `&mut T`). Panic loudly.
        panic!(
            "QD4 violation: set_table_readonly called for &mut T (T = {}). \
             Did a custom QueryData impl falsely claim ReadOnlyQueryData?",
            std::any::type_name::<T>()
        );
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY (QD1, QD3): `archetype` carries write-capable provenance
        //   (caller obtained it via `archetype_ptr_mut`). `columns` at offset
        //   0 per Phase 7 D4; `state.id.0 < MAX_COMPONENTS` by construction.
        //   `column.ptr` is `*mut u8` with write-capable provenance preserved
        //   from `refresh_column` at pool-add time (Phase 7 U7); the cast
        //   preserves the Unique tag.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "QD2: column was unexpectedly null");
        fetch.base = column.ptr as *mut T;
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // QD4: read-only cursor on `&mut T` is forbidden by the trait gate
        // `D: ReadOnlyQueryData` on `Query::iter()`. Mirrors the meta-bearing
        // backstop above.
        panic!(
            "QD4 violation: set_table_readonly_no_meta called for &mut T (T = {}). \
             Did a custom QueryData impl falsely claim ReadOnlyQueryData?",
            std::any::type_name::<T>()
        );
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // Meta-free body — identical to `set_table_mut` minus the unused
        // `_meta`. NCD = false (`&mut T` does not consult ticks).
        // SAFETY (QD1, QD3): same conditions as `set_table_mut`.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "QD2: column was unexpectedly null");
        fetch.base = column.ptr as *mut T;
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        // SAFETY (QD2, QD3): `set_table_mut` set `base` (caller contract);
        //   `row < entity_count` (caller contract); no aliasing per
        //   `FilteredAccessSet` declaration + cursor `&mut self`. The
        //   returned `&'w mut T` lifetime is tied to `'w` via
        //   `PhantomData<&'w mut T>` in `WriteFetch`.
        unsafe { &mut *fetch.base.add(row) }
    }
}

// NOTE: No `ReadOnlyQueryData for &mut T` impl — `&mut T` writes.

// ── Ref<'w, T> (Phase 10 Wave C Step 11 — read-only with ticks) ─────────────

/// Read-only access to component `T` with attached change-detection info
/// (plan §2.5 REF1-REF4 / §6.1).
///
/// Compared to `&T` (no tick exposure) and `Changed<T>` (forces a filter
/// through the type system), `Ref<T>` is the "I want to read the tick
/// without filtering" path. Use it when the system needs to know whether
/// `T` was added or changed and to read the underlying value in the same
/// pass.
///
/// # Boundary semantics ([`Ref::is_added`] / [`Ref::is_changed`])
///
/// Per plan §6.2 / §6.2-bis (Round 2 O1) — both predicates use the inclusive
/// lower-bound trick (`last_run - 1`) so a self-write within the current
/// system reports as changed. The match formula is
/// `tick.is_newer_than(last_run - 1, this_run)`, equivalent to
/// `tick >= last_run` under bounded ages. See [`Tick::is_newer_than`] for
/// the precise wrapping semantics.
pub struct Ref<'w, T: Component> {
    /// Borrowed view into the component slot.
    pub(crate) value: &'w T,
    /// Snapshot of the row's `added` tick at fetch time.
    pub(crate) added: Tick,
    /// Snapshot of the row's `changed` tick at fetch time.
    pub(crate) changed: Tick,
    /// System's `last_run` snapshot.
    pub(crate) last_run: Tick,
    /// System's `this_run` snapshot.
    pub(crate) this_run: Tick,
}

impl<'w, T: Component> Ref<'w, T> {
    /// Returns `true` if `T` was inserted into this row since the system's
    /// `last_run` tick.
    ///
    /// Uses the inclusive-lower-bound trick (plan §6.2 / Round 2 O1):
    /// `last_run - 1` is passed as `is_newer_than`'s exclusive lower bound,
    /// promoting it to inclusive. Equivalent to `added >= last_run` under
    /// the bounded-age discipline of plan §9.3.
    #[inline]
    pub fn is_added(&self) -> bool {
        self.added
            .is_newer_than(Tick::new(self.last_run.get().wrapping_sub(1)), self.this_run)
    }

    /// Returns `true` if `T` was added OR mutated in this row since the
    /// system's `last_run` tick.
    ///
    /// Same inclusive semantic as [`Self::is_added`] (plan §6.2-bis): a
    /// self-write within the current frame reports as changed.
    #[inline]
    pub fn is_changed(&self) -> bool {
        self.changed
            .is_newer_than(Tick::new(self.last_run.get().wrapping_sub(1)), self.this_run)
    }
}

impl<'w, T: Component> std::ops::Deref for Ref<'w, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

/// Per-system state for `Ref<T>: QueryData`. Identical to [`ReadState`] —
/// only the trait-impl side differs (the Fetch carries tick column bases).
#[derive(Clone, Copy)]
pub struct RefState<T: Component> {
    pub(crate) id: ComponentId,
    pub(crate) _marker: PhantomData<fn() -> T>,
}

/// Per-archetype fetch scratch for `Ref<T>: QueryData`.
///
/// Caches:
/// * `value_base` — the component column for `T` (cast from `column.ptr`).
/// * `added_base` / `changed_base` — the tick column bases returned by
///   [`Archetype::tick_column_base`].
/// * `last_run` / `this_run` — the system's tick snapshot captured at
///   `set_table_*` time so the per-row hot loop pays no indirection.
///
/// All fields are populated by `set_table_*` before any `fetch` call; the
/// `Box<[_]>` backing for the tick columns gives stable addresses (plan
/// STORE2).
pub struct RefFetch<'w, T: Component> {
    pub(crate) value_base: *const T,
    pub(crate) added_base: *const UnsafeCell<Tick>,
    pub(crate) changed_base: *const UnsafeCell<Tick>,
    pub(crate) last_run: Tick,
    pub(crate) this_run: Tick,
    pub(crate) _marker: PhantomData<&'w T>,
}

impl<T: Component> Clone for RefFetch<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Component> Copy for RefFetch<'_, T> {}

// SAFETY (QD1-QD4):
//   - QD1: `state.id` is `T::component_id()`; `init_access` declares a read.
//   - QD2: `init_fetch` produces NULL pointers; `set_table_*` overwrites
//     them before any `fetch` call.
//   - QD3: every cached pointer is scoped to `'w` via `PhantomData<&'w T>`.
//   - QD4: read-only data — both `set_table_*` methods produce identical
//     behaviour (the mutable variant delegates to the read-only path).
unsafe impl<T: Component> QueryData for Ref<'_, T> {
    type State = RefState<T>;
    type Fetch<'w> = RefFetch<'w, T>;
    type Item<'w> = Ref<'w, T>;
    const IS_READ_ONLY: bool = true;
    // Phase 12.5 Track B NCD2: `Ref<T>` exposes per-row tick info; the
    // dispatcher MUST forward `meta` so `set_table_*` can copy
    // `last_run` / `this_run` into the Fetch.
    const NEEDS_CHANGE_DETECTION: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        RefState {
            id: T::component_id(),
            _marker: PhantomData,
        }
    }

    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        access_set
            .add_component_read(state.id, std::any::type_name::<Self>())
            .unwrap_or_else(|conflict| intra_system_conflict_panic(conflict));
    }

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        RefFetch {
            value_base: std::ptr::null(),
            added_base: std::ptr::null(),
            changed_base: std::ptr::null(),
            last_run: Tick::ZERO,
            this_run: Tick::ZERO,
            _marker: PhantomData,
        }
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        meta: &'_ SystemMeta,
    ) {
        // SAFETY (QD3): `archetype` is a live `*const Archetype` for `'w`
        //   (caller contract); `columns` at offset 0 per Phase 7 D4;
        //   `state.id.0 < MAX_COMPONENTS` by construction of the cached id.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "QD2: column was unexpectedly null");
        fetch.value_base = column.ptr as *const T;

        // SAFETY (STORE3): shared reborrow of the archetype is sound; the
        //   sparse map read does not produce write provenance. `archetype`
        //   contains `state.id` per QD1 / archetype matching, so
        //   `tick_column_base` returns `Some`.
        let archetype_ref: &Archetype = unsafe { &*archetype };
        let (added_base, changed_base) = archetype_ref
            .tick_column_base(state.id)
            .expect("QD1: matched archetype must contain T's pool");
        fetch.added_base = added_base;
        fetch.changed_base = changed_base;

        fetch.last_run = meta.last_run();
        fetch.this_run = meta.this_run();
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,
    ) {
        // For `Ref<T>`, the mutable cursor variant degrades to the same
        // read-only setup; no write provenance is consumed.
        // SAFETY (QD3, QD4): same conditions as `set_table_readonly` with
        //   the strictly-stronger caller guarantee that `archetype` carries
        //   write-capable provenance.
        unsafe { Self::set_table_readonly(fetch, state, archetype as *const _, meta) }
    }

    #[inline(never)]
    #[cold]
    unsafe fn set_table_readonly_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // NCD5 backstop: dispatcher's NCD6 const-fold must route Ref<T>
        // through the meta-bearing path. Reaching here means a contributor
        // broke the dispatch contract.
        panic!(
            "NCD violation: set_table_readonly_no_meta called for {} \
             (NEEDS_CHANGE_DETECTION = true).",
            std::any::type_name::<Self>()
        );
    }

    #[inline(never)]
    #[cold]
    unsafe fn set_table_mut_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {
        panic!(
            "NCD violation: set_table_mut_no_meta called for {} \
             (NEEDS_CHANGE_DETECTION = true).",
            std::any::type_name::<Self>()
        );
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        // SAFETY (QD2, QD3, STORE3):
        //   - `set_table_*` was called before `fetch` (caller contract), so
        //     every base pointer is non-null and points at the active
        //     archetype's column for `T`.
        //   - `row < entity_count` (caller contract).
        //   - The returned `Ref<'w, T>` lifetime is tied to `'w` via
        //     `PhantomData<&'w T>` in `RefFetch`.
        //   - STORE3: the tick reads through `UnsafeCell::get()` need no
        //     concurrent writer of this `(archetype, T)` slot. The guarantee is
        //     named by the construction origin: on the scheduler/query path
        //     (the only origin that builds a read-only `Ref`), Phase 9 SCH3 —
        //     the conflict graph — grants exclusive access; the system-less
        //     `&mut World` path (which `EcsMaster::get_component_mut` uses for
        //     `Mut`) would supply whole-world exclusivity instead. `Tick` is
        //     `Copy`.
        unsafe {
            let value = &*fetch.value_base.add(row);
            let added = *(*fetch.added_base.add(row)).get();
            let changed = *(*fetch.changed_base.add(row)).get();
            Ref {
                value,
                added,
                changed,
                last_run: fetch.last_run,
                this_run: fetch.this_run,
            }
        }
    }
}

// SAFETY: `Ref<T>::IS_READ_ONLY = true`; the impl never writes.
unsafe impl<T: Component> ReadOnlyQueryData for Ref<'_, T> {}

// ── Mut<'w, T> (Phase 10 Wave C Step 11 — write with deref guard) ──────────

/// Mutable access to component `T` with a deref guard that bumps the row's
/// `changed_tick` on first `DerefMut` (plan §2.5 MUT1-MUT8 / §6.2 / §3 Q6).
///
/// Compared to `&mut T` (no change tracking), `Mut<T>` is the path that
/// participates in change detection. Any `DerefMut` call counts as
/// "changed" — the Bevy deref-bump semantic (plan §3 Q6 adopted). Use
/// [`Self::set_if_neq`] or [`Self::bypass_change_detection`] to opt into
/// stricter behaviour.
///
/// # Boundary semantics ([`Self::is_added`] / [`Self::is_changed`])
///
/// Inclusive lower-bound semantic (plan §6.2 / Round 2 O1 + Round 3 O1):
/// a self-write within the same system reports as changed.
///
/// # Once-only deref guard
///
/// The first `deref_mut()` call on a given `Mut<T>` writes `this_run` to
/// the row's `changed_tick`. Subsequent calls within the same guard
/// instance skip the write (`deref_mut_called` flag). This is a
/// micro-optimisation: even if the compiler does not elide the duplicate
/// store the cost is one extra u32 store per call, and the semantic is
/// identical.
pub struct Mut<'w, T: Component> {
    /// Borrowed view into the component slot.
    pub(crate) value: &'w mut T,
    /// Snapshot of the row's `added` tick at fetch time.
    pub(crate) added: Tick,
    /// Pointer to the row's `changed_tick` slot. The write target for the
    /// deref guard. Stable for the pool's lifetime (`Box<[_]>` — plan STORE2).
    pub(crate) changed_tick: *const UnsafeCell<Tick>,
    /// System's `last_run` snapshot.
    pub(crate) last_run: Tick,
    /// System's `this_run` snapshot.
    pub(crate) this_run: Tick,
    /// Has `deref_mut` already bumped the changed tick this guard?
    /// Skips duplicate stores on repeated `deref_mut()` calls.
    pub(crate) deref_mut_called: bool,
}

impl<'w, T: Component> Mut<'w, T> {
    /// Returns `true` if `T` was inserted into this row since the system's
    /// `last_run` tick (inclusive lower bound; plan §6.2 / Round 2 O1).
    #[inline]
    pub fn is_added(&self) -> bool {
        self.added
            .is_newer_than(Tick::new(self.last_run.get().wrapping_sub(1)), self.this_run)
    }

    /// Returns `true` if `T` was added OR mutated in this row since the
    /// system's `last_run` tick.
    ///
    /// Reads the current `changed_tick` slot (per-row); a self-write earlier
    /// in this system reports as changed thanks to the inclusive-lower-bound
    /// trick (plan §6.2-bis worked proof).
    #[inline]
    pub fn is_changed(&self) -> bool {
        // SAFETY (STORE3): `changed_tick` is a live `UnsafeCell<Tick>` slot for
        //   the row (set by `set_table_mut` on the query path, or by
        //   `EcsMaster::get_component_mut` on the direct path). Exclusivity of
        //   this read rests on the `Mut`'s construction origin: on the
        //   scheduler/query path, Phase 9 SCH3 (the conflict graph) grants
        //   exclusive `(archetype, T)` access; on the system-less
        //   `EcsMaster::get_component_mut` path, `&mut World` whole-world
        //   exclusivity (the method borrows the entire `EcsMaster` for the
        //   `Mut`'s lifetime). Both independently guarantee no concurrent
        //   reader/writer of this row's tick. `Tick` is `Copy`.
        let tick: Tick = unsafe { *(*self.changed_tick).get() };
        tick.is_newer_than(Tick::new(self.last_run.get().wrapping_sub(1)), self.this_run)
    }

    /// Sets the value only if it differs from the current one (per
    /// `PartialEq`). Avoids triggering `Changed<T>` when the new value
    /// equals the old.
    ///
    /// Returns `true` if the write happened, `false` otherwise.
    pub fn set_if_neq(&mut self, new_value: T) -> bool
    where
        T: PartialEq,
    {
        if *self.value != new_value {
            *self.value = new_value;
            // The assignment above uses `DerefMut` (`*self.value`) — wait,
            // it actually goes through the `&mut T` borrow directly. We
            // must bump the tick manually here because the assignment did
            // not route through `Mut::deref_mut`.
            if !self.deref_mut_called {
                self.deref_mut_called = true;
                // SAFETY (STORE3, plan §2.5 MUT4): exclusivity of this
                //   `changed_tick` write rests on the `Mut`'s construction
                //   origin — on the scheduler/query path, Phase 9 SCH3 (the
                //   conflict graph) grants exclusive `(archetype, T)` access (the
                //   system declared a write to `T`); on the system-less
                //   `EcsMaster::get_component_mut` path, `&mut World` whole-world
                //   exclusivity. Both independently guarantee no concurrent
                //   reader/writer of this row's tick. `Tick` is `Copy`.
                unsafe {
                    *(*self.changed_tick).get() = self.this_run;
                }
            }
            true
        } else {
            false
        }
    }

    /// Returns `&mut T` without bumping the changed tick. Breaks
    /// `Changed<T>` for this row until the next legitimate write — use
    /// sparingly (plan §2.5 MUT5).
    #[inline]
    pub fn bypass_change_detection(&mut self) -> &mut T {
        self.value
    }
}

impl<'w, T: Component> std::ops::Deref for Mut<'w, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<'w, T: Component> std::ops::DerefMut for Mut<'w, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        if !self.deref_mut_called {
            self.deref_mut_called = true;
            // SAFETY (STORE3, plan §2.5 MUT3):
            //   - `self.changed_tick` is a live `UnsafeCell<Tick>` slot for the
            //     cached row (set by `set_table_mut` on the query path, or by
            //     `EcsMaster::get_component_mut` on the direct path); the
            //     `Box<[_]>` backing is stable for the pool's lifetime
            //     (plan STORE2).
            //   - Exclusivity of this write is named by the construction origin:
            //     on the scheduler/query path, Phase 9 SCH3 (the conflict graph)
            //     guarantees the system holding `Mut<T>` has exclusive access to
            //     this `(archetype, T)` slot — no concurrent reader/writer of
            //     this tick exists, and per-row adjacent writes from sibling
            //     `par_iter` chunks ride disjoint memory locations (Round 2 C3 —
            //     distinct `UnsafeCell<u32>`s); on the system-less
            //     `EcsMaster::get_component_mut` path, `&mut World` whole-world
            //     exclusivity supplies the same guarantee. `Tick` is `Copy`.
            unsafe {
                *(*self.changed_tick).get() = self.this_run;
            }
        }
        self.value
    }
}

/// Per-system state for `Mut<T>: QueryData`. Same shape as [`WriteState`].
#[derive(Clone, Copy)]
pub struct MutState<T: Component> {
    pub(crate) id: ComponentId,
    pub(crate) _marker: PhantomData<fn() -> T>,
}

/// Per-archetype fetch scratch for `Mut<T>: QueryData`.
///
/// Caches the write-capable component column base plus both tick column
/// bases (so `fetch` can produce a `Mut<T>` carrying the per-row
/// `changed_tick` pointer for the deref guard). Tick base lifetimes ride
/// the `Box<[_]>` stability (plan STORE2).
pub struct MutFetch<'w, T: Component> {
    pub(crate) value_base: *mut T,
    pub(crate) added_base: *const UnsafeCell<Tick>,
    pub(crate) changed_base: *const UnsafeCell<Tick>,
    pub(crate) last_run: Tick,
    pub(crate) this_run: Tick,
    pub(crate) _marker: PhantomData<&'w mut T>,
}

impl<T: Component> Clone for MutFetch<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Component> Copy for MutFetch<'_, T> {}

// SAFETY (QD1-QD4):
//   - QD1: `state.id` is `T::component_id()`; `init_access` declares a write.
//   - QD2: `set_table_mut` overwrites the bases; `set_table_readonly`
//     panics (QD4 runtime backstop — same as `&mut T`).
//   - QD3: lifetime bound by `PhantomData<&'w mut T>` in `MutFetch`.
//   - QD4: `set_table_readonly` is forbidden (the trait gate on
//     `Query::iter()` requires `D: ReadOnlyQueryData`, which `Mut<T>` does
//     not implement).
unsafe impl<T: Component> QueryData for Mut<'_, T> {
    type State = MutState<T>;
    type Fetch<'w> = MutFetch<'w, T>;
    type Item<'w> = Mut<'w, T>;
    const IS_READ_ONLY: bool = false;
    // Phase 12.5 Track B NCD2: `Mut<T>` exposes per-row tick info via the
    // deref guard and `is_added`/`is_changed`; the dispatcher MUST forward
    // `meta`.
    const NEEDS_CHANGE_DETECTION: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        MutState {
            id: T::component_id(),
            _marker: PhantomData,
        }
    }

    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        access_set
            .add_component_write(state.id, std::any::type_name::<Self>())
            .unwrap_or_else(|conflict| intra_system_conflict_panic(conflict));
    }

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        MutFetch {
            value_base: std::ptr::null_mut(),
            added_base: std::ptr::null(),
            changed_base: std::ptr::null(),
            last_run: Tick::ZERO,
            this_run: Tick::ZERO,
            _marker: PhantomData,
        }
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // QD4: read-only cursor on `Mut<T>` is forbidden by the trait gate
        // `D: ReadOnlyQueryData` on `Query::iter()`. Reaching here means a
        // custom `QueryData` impl falsely claimed `ReadOnlyQueryData` for
        // a type containing `Mut<T>`. Panic loudly.
        panic!(
            "QD4 violation: set_table_readonly called for Mut<T> (T = {}). \
             Did a custom QueryData impl falsely claim ReadOnlyQueryData?",
            std::any::type_name::<T>()
        );
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,
    ) {
        // SAFETY (QD1, QD3): `archetype` carries write-capable provenance
        //   (caller minted it via `archetype_ptr_mut`); `columns` at offset
        //   0 per Phase 7 D4; `state.id.0 < MAX_COMPONENTS`.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "QD2: column was unexpectedly null");
        fetch.value_base = column.ptr as *mut T;

        // SAFETY (STORE3): shared reborrow for the sparse-map read; no
        //   write-capable provenance is needed for the tick column lookup
        //   (the per-row write goes through `UnsafeCell::get()`, separately
        //   ridden by the access-set declaration).
        let archetype_ref: &Archetype = unsafe { &*archetype };
        let (added_base, changed_base) = archetype_ref
            .tick_column_base(state.id)
            .expect("QD1: matched archetype must contain T's pool");
        fetch.added_base = added_base;
        fetch.changed_base = changed_base;

        fetch.last_run = meta.last_run();
        fetch.this_run = meta.this_run();
    }

    #[inline(never)]
    #[cold]
    unsafe fn set_table_readonly_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // NCD5 backstop: dispatcher's NCD6 const-fold must route Mut<T>
        // through the meta-bearing path (and the read-only cursor is
        // additionally gated out by `D: ReadOnlyQueryData` on `iter`).
        panic!(
            "NCD violation: set_table_readonly_no_meta called for {} \
             (NEEDS_CHANGE_DETECTION = true).",
            std::any::type_name::<Self>()
        );
    }

    #[inline(never)]
    #[cold]
    unsafe fn set_table_mut_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {
        panic!(
            "NCD violation: set_table_mut_no_meta called for {} \
             (NEEDS_CHANGE_DETECTION = true).",
            std::any::type_name::<Self>()
        );
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        // SAFETY (QD2, QD3, STORE3):
        //   - `set_table_mut` was called before `fetch` (caller contract);
        //     every base pointer is live and non-null.
        //   - `row < entity_count` (caller contract).
        //   - STORE3: exclusivity of the `&mut *value_base.add(row)` reborrow
        //     and the tick access is named by the construction origin. This
        //     `fetch` is the scheduler/query origin, where Phase 9 SCH3 (the
        //     conflict graph) grants exclusive `(archetype, T)` access (no
        //     aliasing reader/writer). The `EcsMaster::get_component_mut` direct
        //     origin builds its `Mut` separately, resting on `&mut World`
        //     whole-world exclusivity. The returned `Mut<'w, T>` lifetime is
        //     tied to `'w` via `PhantomData<&'w mut T>` in `MutFetch`.
        //   - The tick reads through `UnsafeCell::get()` are sound for the same
        //     reason; `Tick` is `Copy`.
        //   - `changed_tick` pointer is captured as `*const UnsafeCell<Tick>`
        //     for the row; the deref guard writes through `UnsafeCell::get()`
        //     (per-row distinct memory location — Round 2 C3).
        unsafe {
            let value = &mut *fetch.value_base.add(row);
            let added = *(*fetch.added_base.add(row)).get();
            let changed_tick = fetch.changed_base.add(row);
            Mut {
                value,
                added,
                changed_tick,
                last_run: fetch.last_run,
                this_run: fetch.this_run,
                deref_mut_called: false,
            }
        }
    }
}

// NOTE: No `ReadOnlyQueryData for Mut<T>` impl — `Mut<T>` writes (deref guard).

// ── Variadic tuple impls (§4.6 / §10.1, M4) ────────────────────────────────
//
// A single `macro_rules!` site emits `QueryData` impls for tuple arities
// `1..=MAX_QUERY_DATA_ARITY` (= 12). The paired-ident invocation syntax
// `((D, s, f), ...)` carries three distinct ident kinds per tuple element:
//
// * `$D` — type-ident used in trait bounds (`D0: QueryData`, etc.).
// * `$s` — value-ident bound to the per-element `State` inside `let
//   ($($s,)*) = state` destructures.
// * `$f` — value-ident bound to the per-element `Fetch<'w>` inside `let
//   ($($f,)*) = fetch` destructures.
//
// The pairing avoids `paste!` (no external dep) and the Round-1 pseudo
// `[< state_ $d >]` syntax (rejected per M4). See plan §25 for the
// concrete arity-3 expansion.
//
// `ReadOnlyQueryData` is auto-emitted alongside in a dedicated
// `impl_read_only_query_data_tuple!` macro (avoids requiring every
// `$D` simultaneously satisfy both `QueryData` and `ReadOnlyQueryData`
// at the `unsafe impl` site of the working macro — the gated
// `ReadOnlyQueryData` blanket has its own bound set).

/// Emits a `QueryData` impl for a tuple of the given paired idents (one
/// `(TypeIdent, state_value_ident, fetch_value_ident)` triple per
/// element). Invoked for arity `1..=MAX_QUERY_DATA_ARITY`.
macro_rules! impl_query_data_tuple {
    ( $( ($D:ident, $s:ident, $f:ident) ),* ) => {
        // SAFETY (QD1-QD4): the tuple impl forwards every method to its
        //   per-element delegate, which upholds QD1-QD4 by its own
        //   contract. `archetype` is the same pointer for every element in
        //   one `set_table_*` call (each element caches its own column).
        //   Intra-tuple aliasing among `$D`s is detected at `init_access`
        //   via `FilteredAccessSet`.
        #[allow(non_snake_case)]
        unsafe impl< $($D: QueryData),* > QueryData for ( $($D,)* ) {
            type State = ( $($D::State,)* );
            type Fetch<'w> = ( $($D::Fetch<'w>,)* );
            type Item<'w> = ( $($D::Item<'w>,)* );

            const IS_READ_ONLY: bool = true $( && $D::IS_READ_ONLY )*;

            // Phase 12.5 Track B NCD3: tuple propagation — any element
            // needing change detection forces the dispatcher to use the
            // meta-bearing variant for the whole tuple.
            const NEEDS_CHANGE_DETECTION: bool = false $( || $D::NEEDS_CHANGE_DETECTION )*;

            #[inline]
            fn init_state(world: &mut EcsMaster) -> Self::State {
                ( $( <$D as QueryData>::init_state(world), )* )
            }

            #[inline]
            fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
                let ( $($s,)* ) = state;
                $( <$D as QueryData>::init_access($s, access_set); )*
            }

            #[inline]
            fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
                let ( $($s,)* ) = state;
                true $( && <$D as QueryData>::matches_component_set($s, mask) )*
            }

            #[inline]
            fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
                let ( $($s,)* ) = state;
                $( <$D as QueryData>::aggregate_include($s, include); )*
            }

            #[inline]
            fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
                let ( $($s,)* ) = state;
                ( $( <$D as QueryData>::init_fetch($s), )* )
            }

            #[inline]
            unsafe fn set_table_readonly<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *const Archetype,
                meta: &'_ SystemMeta,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (QD3, QD4): forwarded per-element; `archetype`
                    //   carries read-only provenance and is identical for
                    //   every element. The caller of the tuple impl
                    //   upheld QD3/QD4 for every `$D`. `meta` is forwarded
                    //   by reference per Round 2 W7.
                    unsafe { <$D as QueryData>::set_table_readonly($f, $s, archetype, meta); }
                )*
            }

            #[inline]
            unsafe fn set_table_mut<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *mut Archetype,
                meta: &'_ SystemMeta,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (QD3, QD4): write-capable `archetype` is
                    //   forwarded to every element; the caller upheld
                    //   QD3/QD4. `meta` forwarded by reference.
                    unsafe { <$D as QueryData>::set_table_mut($f, $s, archetype, meta); }
                )*
            }

            #[inline]
            unsafe fn set_table_readonly_no_meta<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *const Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (QD3, QD4): forwarded per-element; the tuple's
                    //   NCD3 propagation guarantees this method is only
                    //   reached when no element needs change detection,
                    //   so every element's `_no_meta` body is the
                    //   meta-free re-impl (not the cold panic backstop).
                    unsafe { <$D as QueryData>::set_table_readonly_no_meta($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn set_table_mut_no_meta<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *mut Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (QD3, QD4): write-capable `archetype` forwarded;
                    //   same NCD3-propagation note as the readonly variant.
                    unsafe { <$D as QueryData>::set_table_mut_no_meta($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
                let ( $($f,)* ) = fetch;
                (
                    $(
                        // SAFETY (QD2, QD3): per-element fetch contract
                        //   held by the caller; `row` is in range for the
                        //   archetype previously cached by `set_table_*`.
                        unsafe { <$D as QueryData>::fetch($f, row) },
                    )*
                )
            }
        }
    };
}

/// Emits a `ReadOnlyQueryData` impl for the tuple of the given type-idents.
/// Gated separately from [`impl_query_data_tuple!`] so the bound set is
/// `$D: ReadOnlyQueryData` (which transitively implies `$D: QueryData`)
/// without conflating the two trait bounds in a single `impl<>` header.
macro_rules! impl_read_only_query_data_tuple {
    ( $( $D:ident ),* ) => {
        // SAFETY: every `$D` is `ReadOnlyQueryData` (each `$D::IS_READ_ONLY
        //   = true` and the impl is gated to perform no writes). The tuple
        //   impl forwards every fetch to per-element fetch, which is
        //   read-only by induction.
        unsafe impl< $($D: ReadOnlyQueryData),* > ReadOnlyQueryData for ( $($D,)* ) {}
    };
}

// Empty-tuple base case: `Query<(), F>` yields `()` per row. Useful for
// entity-only / filter-only queries (e.g. `Query<(), With<Player>>`).
// SAFETY (QD1-QD4): vacuous — no state, no access surface, no fetched
//   columns, no per-row dereferences. All four invariants hold trivially.
unsafe impl QueryData for () {
    type State = ();
    type Fetch<'w> = ();
    type Item<'w> = ();
    const IS_READ_ONLY: bool = true;
    // Phase 12.5 Track B NCD2: vacuous — `()` touches no components.
    const NEEDS_CHANGE_DETECTION: bool = false;

    #[inline] fn init_state(_world: &mut EcsMaster) -> Self::State {}
    #[inline] fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {}
    #[inline] fn matches_component_set(_state: &Self::State, _mask: &ComponentMask) -> bool { true }
    #[inline] fn aggregate_include(_state: &Self::State, _include: &mut ComponentMask) {}
    #[inline] fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {}
    #[inline] unsafe fn set_table_readonly<'w>(_f: &mut Self::Fetch<'w>, _s: &Self::State, _a: *const Archetype, _meta: &'_ SystemMeta) {}
    #[inline] unsafe fn set_table_mut<'w>(_f: &mut Self::Fetch<'w>, _s: &Self::State, _a: *mut Archetype, _meta: &'_ SystemMeta) {}
    #[inline] unsafe fn set_table_readonly_no_meta<'w>(_f: &mut Self::Fetch<'w>, _s: &Self::State, _a: *const Archetype) {}
    #[inline] unsafe fn set_table_mut_no_meta<'w>(_f: &mut Self::Fetch<'w>, _s: &Self::State, _a: *mut Archetype) {}
    #[inline] unsafe fn fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> Self::Item<'w> {}
}

// SAFETY: () has IS_READ_ONLY = true.
unsafe impl ReadOnlyQueryData for () {}

impl_query_data_tuple!((D0, s0, f0));
impl_query_data_tuple!((D0, s0, f0), (D1, s1, f1));
impl_query_data_tuple!((D0, s0, f0), (D1, s1, f1), (D2, s2, f2));
impl_query_data_tuple!((D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3));
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11)
);

impl_read_only_query_data_tuple!(D0);
impl_read_only_query_data_tuple!(D0, D1);
impl_read_only_query_data_tuple!(D0, D1, D2);
impl_read_only_query_data_tuple!(D0, D1, D2, D3);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5, D6);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5, D6, D7);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5, D6, D7, D8);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5, D6, D7, D8, D9);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5, D6, D7, D8, D9, D10);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5, D6, D7, D8, D9, D10, D11);

// ── Arity-overflow stubs (arity 13..=24) — M7 + C-NEW-2 ────────────────────
//
// Same pattern as Phase 8a's `params/tuple_impl.rs::
// impl_system_param_tuple_too_large!`: each method body is `const {
// panic!(...) }`, which evaluates ONLY at monomorphization. Crates that
// never instantiate a 13+ arity `QueryData` tuple compile cleanly.
//
// `compile_error!` was rejected in Phase 8a (C-NEW-2): it fires at
// macro-expansion time, breaking the wider crate. `panic!()`
// requires `rustc >= 1.79`; boyko targets Rust 2024 (`>= 1.85`).

/// Emits a stub `QueryData` impl whose every method body is
/// `panic!(...)`. The const block fires at monomorphization;
/// the impl is never *successfully* used at runtime. `State`, `Fetch<'w>`,
/// and `Item<'w>` collapse to `()` so the stub type-checks in isolation.
macro_rules! impl_query_data_tuple_too_large {
    ( $( ($D:ident, $s:ident, $f:ident) ),* ) => {
        // SAFETY: stub impl whose every method body is `panic!(...)`.
        //   The impl is never *successfully* used at runtime — the const
        //   block fails at monomorphization with the diagnostic in
        //   `init_state`. QD1-QD4 are vacuously upheld because no code
        //   path that respects the contract ever observes the impl's
        //   effects.
        #[allow(non_snake_case, unused_variables)]
        unsafe impl< $($D: QueryData),* > QueryData for ( $($D,)* ) {
            type State = ();
            type Fetch<'w> = ();
            type Item<'w> = ();
            const IS_READ_ONLY: bool = true;
            // Vacuous — every method is a `panic!()` at monomorphisation,
            // so the const is unobservable on any reachable path.
            const NEEDS_CHANGE_DETECTION: bool = false;

            fn init_state(_world: &mut EcsMaster) -> Self::State {
                panic!(
                        "tuple has too many QueryData elements. \
                         boyko-engine supports up to \
                         MAX_QUERY_DATA_ARITY = 12. Split your query into \
                         smaller queries or wrap related elements in a \
                         struct that implements QueryData."
                    )
            }

            fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {
                panic!("tuple too large: see init_state diagnostic")
            }

            fn matches_component_set(_state: &Self::State, _mask: &ComponentMask) -> bool {
                panic!("tuple too large: see init_state diagnostic")
            }

            fn aggregate_include(_state: &Self::State, _include: &mut ComponentMask) {
                panic!("tuple too large: see init_state diagnostic")
            }

            fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
                panic!("tuple too large: see init_state diagnostic")
            }

            unsafe fn set_table_readonly<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *const Archetype,
                _meta: &'_ SystemMeta,
            ) {
                panic!("tuple too large: see init_state diagnostic")
            }

            unsafe fn set_table_mut<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *mut Archetype,
                _meta: &'_ SystemMeta,
            ) {
                panic!("tuple too large: see init_state diagnostic")
            }

            unsafe fn set_table_readonly_no_meta<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *const Archetype,
            ) {
                panic!("tuple too large: see init_state diagnostic")
            }

            unsafe fn set_table_mut_no_meta<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *mut Archetype,
            ) {
                panic!("tuple too large: see init_state diagnostic")
            }

            unsafe fn fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> Self::Item<'w> {
                panic!("tuple too large: see init_state diagnostic")
            }
        }
    };
}

impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17), (D18, s18, f18)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17), (D18, s18, f18), (D19, s19, f19)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17), (D18, s18, f18), (D19, s19, f19),
    (D20, s20, f20)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17), (D18, s18, f18), (D19, s19, f19),
    (D20, s20, f20), (D21, s21, f21)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17), (D18, s18, f18), (D19, s19, f19),
    (D20, s20, f20), (D21, s21, f21), (D22, s22, f22)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17), (D18, s18, f18), (D19, s19, f19),
    (D20, s20, f20), (D21, s21, f21), (D22, s22, f22), (D23, s23, f23)
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component_registry;

    // ID range 480-485 reserved for Phase 8b Step 2 unit tests (per plan
    // §17.4 "100-149: Phase 8b QueryData unit tests" — but the table refers
    // to test numbering, not ComponentIds; we pick disjoint ComponentIds to
    // avoid collisions with archetype.rs (400-417), legacy_query.rs (200-203),
    // query_state.rs (490-493), and component_set.rs (495-497) tests).
    // Slots 480-482 are reserved by archetype_bundle::miri_tests
    // (BundleCompX=480, BundleCompY=481, PanicDropComp=482). Pick a free
    // range that doesn't collide with resource_registry's CompThenRes=510.
    const COMP_MY_ID: ComponentId = ComponentId(503);
    const COMP_OTHER_ID: ComponentId = ComponentId(504);

    /// Primary component fixture: a simple POD struct so the QueryData impls
    /// can be type-checked end-to-end without engaging archetype storage.
    ///
    /// Phase 10 Wave C — `PartialEq` is required by `Mut<T>::set_if_neq`'s
    /// trait bound; tests in the `Mut` block invoke it.
    #[repr(C)]
    #[derive(Clone, Copy, PartialEq)]
    struct MyComp(u32);

    impl Component for MyComp {
        fn component_id() -> ComponentId {
            COMP_MY_ID
        }
    }

    /// Second component fixture for disjoint-access tests.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct OtherComp(#[allow(dead_code)] u64);

    impl Component for OtherComp {
        fn component_id() -> ComponentId {
            COMP_OTHER_ID
        }
    }

    /// One-shot registry priming for the test components. Idempotent —
    /// re-registration with the same `TypeId` is a silent no-op.
    fn register_test_components() {
        component_registry::register_layout::<MyComp>(COMP_MY_ID.0);
        component_registry::register_layout::<OtherComp>(COMP_OTHER_ID.0);
    }

    #[test]
    // Intentional const check: asserting the value of a `QueryData` associated
    // const is the whole point of the test, so clippy's "constant in assert"
    // lint does not apply.
    #[allow(clippy::assertions_on_constants)]
    fn ref_t_is_read_only() {
        assert!(
            <&MyComp as QueryData>::IS_READ_ONLY,
            "&T must report IS_READ_ONLY = true"
        );
    }

    #[test]
    #[allow(clippy::assertions_on_constants)] // intentional associated-const check
    fn mut_t_is_not_read_only() {
        assert!(
            !<&mut MyComp as QueryData>::IS_READ_ONLY,
            "&mut T must report IS_READ_ONLY = false"
        );
    }

    #[test]
    fn init_state_caches_component_id() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let read_state = <&MyComp as QueryData>::init_state(&mut ecs);
        assert_eq!(
            read_state.id,
            MyComp::component_id(),
            "ReadState must cache T::component_id()"
        );
        let write_state = <&mut MyComp as QueryData>::init_state(&mut ecs);
        assert_eq!(
            write_state.id,
            MyComp::component_id(),
            "WriteState must cache T::component_id()"
        );
    }

    #[test]
    fn init_access_declares_read_for_ref_t() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let state = <&MyComp as QueryData>::init_state(&mut ecs);
        let mut set = FilteredAccessSet::new();
        <&MyComp as QueryData>::init_access(&state, &mut set);
        // Probe: a sibling write to the same id must conflict, proving the
        // read bit landed.
        let mut writer = crate::ecs::core::system::access::Access::new();
        writer.add_component_write(state.id);
        assert!(
            set.combined().conflicts_with(&writer),
            "FilteredAccessSet must carry the &MyComp read bit"
        );
    }

    #[test]
    fn init_access_declares_write_for_mut_t() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let state = <&mut MyComp as QueryData>::init_state(&mut ecs);
        let mut set = FilteredAccessSet::new();
        <&mut MyComp as QueryData>::init_access(&state, &mut set);
        // Probe: a sibling read of the same id must conflict, proving the
        // write bit landed.
        let mut reader = crate::ecs::core::system::access::Access::new();
        reader.add_component_read(state.id);
        assert!(
            set.combined().conflicts_with(&reader),
            "FilteredAccessSet must carry the &mut MyComp write bit"
        );
    }

    #[test]
    #[should_panic(expected = "boyko-B0002")]
    fn init_access_intra_conflict_panics_for_resmut_pattern() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let state = <&mut MyComp as QueryData>::init_state(&mut ecs);
        let mut set = FilteredAccessSet::new();
        // First call: declares the write — succeeds.
        <&mut MyComp as QueryData>::init_access(&state, &mut set);
        // Second call: same write on the same id — must panic with B0002.
        <&mut MyComp as QueryData>::init_access(&state, &mut set);
    }

    #[test]
    fn matches_component_set_returns_true_iff_id_present() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let state = <&MyComp as QueryData>::init_state(&mut ecs);

        let mut mask = ComponentMask::new();
        assert!(
            !<&MyComp as QueryData>::matches_component_set(&state, &mask),
            "empty mask must not match"
        );
        mask.set(state.id);
        assert!(
            <&MyComp as QueryData>::matches_component_set(&state, &mask),
            "mask with the id set must match"
        );

        // Mirror check for &mut T.
        let mut_state = <&mut MyComp as QueryData>::init_state(&mut ecs);
        let mut mut_mask = ComponentMask::new();
        assert!(!<&mut MyComp as QueryData>::matches_component_set(
            &mut_state, &mut_mask
        ));
        mut_mask.set(mut_state.id);
        assert!(<&mut MyComp as QueryData>::matches_component_set(
            &mut_state, &mut_mask
        ));
    }

    #[test]
    fn aggregate_include_sets_id_bit() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let read_state = <&MyComp as QueryData>::init_state(&mut ecs);
        let mut include = ComponentMask::new();
        <&MyComp as QueryData>::aggregate_include(&read_state, &mut include);
        assert!(
            include.contains(read_state.id),
            "&T::aggregate_include must set the component-id bit"
        );

        let write_state = <&mut MyComp as QueryData>::init_state(&mut ecs);
        let mut write_include = ComponentMask::new();
        <&mut MyComp as QueryData>::aggregate_include(&write_state, &mut write_include);
        assert!(
            write_include.contains(write_state.id),
            "&mut T::aggregate_include must set the component-id bit"
        );
    }

    // ── Variadic tuple impl tests (Step 4) ──────────────────────────────

    /// Compile-only shim: instantiating `assert_impl::<T>()` proves `T`
    /// satisfies `QueryData`. Used by the test bodies below.
    fn assert_impl<T: QueryData>() {}

    #[test]
    fn tuple_2_is_query_data() {
        // Compile-only existence check that the arity-2 macro invocation
        // emitted the tuple impl.
        assert_impl::<(&MyComp, &OtherComp)>();
    }

    #[test]
    #[allow(clippy::assertions_on_constants)] // intentional associated-const check
    fn tuple_2_all_read_is_read_only() {
        assert!(
            <(&MyComp, &OtherComp) as QueryData>::IS_READ_ONLY,
            "all-read tuple must AND-fold IS_READ_ONLY = true"
        );
    }

    #[test]
    #[allow(clippy::assertions_on_constants)] // intentional associated-const check
    fn tuple_2_with_mut_is_not_read_only() {
        assert!(
            !<(&MyComp, &mut OtherComp) as QueryData>::IS_READ_ONLY,
            "tuple containing a &mut element must AND-fold IS_READ_ONLY = false"
        );
        assert!(
            !<(&mut MyComp, &OtherComp) as QueryData>::IS_READ_ONLY,
            "tuple containing a &mut element must AND-fold IS_READ_ONLY = false \
             regardless of element order"
        );
    }

    #[test]
    fn arity_12_query_data_compiles() {
        // The 12-arity cap from `MAX_QUERY_DATA_ARITY`. Mix `&T` and
        // `&mut T` so the compile-only check exercises both element
        // shapes simultaneously.
        assert_impl::<(
            &MyComp,
            &OtherComp,
            &MyComp,
            &OtherComp,
            &MyComp,
            &OtherComp,
            &MyComp,
            &mut OtherComp,
            &mut MyComp,
            &mut OtherComp,
            &mut MyComp,
            &mut OtherComp,
        )>();
    }

    // ── Ref<T> / Mut<T> tests (Phase 10 Wave C Step 11) ─────────────────

    /// `Ref<T>` MUST report `IS_READ_ONLY = true` and satisfy the
    /// `ReadOnlyQueryData` bound; `Mut<T>` MUST report `false`.
    #[test]
    fn ref_wrapper_is_read_only() {
        const { assert!(<Ref<'_, MyComp> as QueryData>::IS_READ_ONLY) };
        fn assert_read_only<T: ReadOnlyQueryData>() {}
        assert_read_only::<Ref<'_, MyComp>>();
    }

    #[test]
    fn mut_wrapper_is_not_read_only() {
        const { assert!(!<Mut<'_, MyComp> as QueryData>::IS_READ_ONLY) };
    }

    /// Access surface: `Ref<T>` declares a read, `Mut<T>` declares a write
    /// (plan §2.5 REF2 / MUT8).
    #[test]
    fn ref_wrapper_init_access_declares_read() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let state = <Ref<'_, MyComp> as QueryData>::init_state(&mut ecs);
        let mut set = FilteredAccessSet::new();
        <Ref<'_, MyComp> as QueryData>::init_access(&state, &mut set);
        let mut writer = crate::ecs::core::system::access::Access::new();
        writer.add_component_write(state.id);
        assert!(
            set.combined().conflicts_with(&writer),
            "Ref<T>::init_access must declare a read"
        );
    }

    #[test]
    fn mut_wrapper_init_access_declares_write() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let state = <Mut<'_, MyComp> as QueryData>::init_state(&mut ecs);
        let mut set = FilteredAccessSet::new();
        <Mut<'_, MyComp> as QueryData>::init_access(&state, &mut set);
        let mut reader = crate::ecs::core::system::access::Access::new();
        reader.add_component_read(state.id);
        assert!(
            set.combined().conflicts_with(&reader),
            "Mut<T>::init_access must declare a write"
        );
    }

    /// `Ref<T>::is_added` / `is_changed` expose the tick info captured at
    /// fetch time, with the inclusive lower-bound semantic (plan §6.2-bis).
    #[test]
    fn ref_provides_value_and_tick_info() {
        let value = MyComp(42);
        let r = Ref {
            value: &value,
            added: Tick::new(5),
            changed: Tick::new(8),
            last_run: Tick::new(2),
            this_run: Tick::new(10),
        };
        assert_eq!(r.0, 42, "Deref must expose the underlying value");
        assert!(r.is_added(), "added=5 ∈ [2, 10] under inclusive-lower-bound");
        assert!(r.is_changed(), "changed=8 ∈ [2, 10]");

        // Same `last_run` boundary — inclusive in `is_added/is_changed`
        // semantic so a tick exactly at `last_run` reports true (plan §6.2-bis
        // worked proof: this matches Bevy's documented `>=` semantic).
        let r_boundary = Ref {
            value: &value,
            added: Tick::new(2),
            changed: Tick::new(2),
            last_run: Tick::new(2),
            this_run: Tick::new(10),
        };
        assert!(r_boundary.is_added(), "added=last_run → inclusive lower bound");
        assert!(r_boundary.is_changed(), "changed=last_run → inclusive lower bound");

        // Strictly before last_run — must report false.
        let r_old = Ref {
            value: &value,
            added: Tick::new(1),
            changed: Tick::new(1),
            last_run: Tick::new(2),
            this_run: Tick::new(10),
        };
        assert!(!r_old.is_added());
        assert!(!r_old.is_changed());
    }

    /// `Mut<T>::deref_mut` MUST bump the row's `changed_tick` to `this_run`
    /// (plan §2.5 MUT3 — Bevy deref-bump semantic).
    #[test]
    fn mut_deref_mut_bumps_changed_tick() {
        let mut value = MyComp(1);
        let changed_cell = UnsafeCell::new(Tick::new(0));
        {
            let mut m = Mut {
                value: &mut value,
                added: Tick::new(5),
                changed_tick: &changed_cell as *const UnsafeCell<Tick>,
                last_run: Tick::new(2),
                this_run: Tick::new(10),
                deref_mut_called: false,
            };
            // Trigger the deref guard.
            *m = MyComp(99);
        }
        // SAFETY: `changed_cell` is dropped at the end of this scope; the
        //   raw pointer captured by `Mut` was valid for the prior block.
        let observed = unsafe { *changed_cell.get() };
        assert_eq!(
            observed,
            Tick::new(10),
            "deref_mut must write this_run into the changed_tick slot"
        );
        assert_eq!(value.0, 99, "deref_mut must expose the underlying &mut T");
    }

    /// Subsequent `deref_mut()` calls on the same `Mut` instance MUST skip
    /// the tick write (once-only flag). Verified by mutating the
    /// `changed_tick` cell between the two calls and confirming the second
    /// `deref_mut` does NOT overwrite it.
    #[test]
    fn mut_deref_mut_is_once_only_per_guard() {
        let mut value = MyComp(1);
        let changed_cell = UnsafeCell::new(Tick::new(0));
        let mut m = Mut {
            value: &mut value,
            added: Tick::new(5),
            changed_tick: &changed_cell as *const UnsafeCell<Tick>,
            last_run: Tick::new(2),
            this_run: Tick::new(10),
            deref_mut_called: false,
        };
        // First deref_mut → bumps tick to this_run.
        *m = MyComp(2);
        // SAFETY: changed_cell is live in this scope.
        unsafe {
            *changed_cell.get() = Tick::new(42);
        }
        // Second deref_mut on the same guard → MUST NOT touch the slot.
        *m = MyComp(3);
        let observed = unsafe { *changed_cell.get() };
        assert_eq!(
            observed,
            Tick::new(42),
            "second deref_mut on the same guard must skip the tick write"
        );
    }

    /// `Mut<T>::set_if_neq` MUST NOT bump the changed tick when the new
    /// value equals the current one (plan §2.5 MUT4).
    #[test]
    fn mut_set_if_neq_no_tick_bump_when_equal() {
        let mut value = MyComp(7);
        let changed_cell = UnsafeCell::new(Tick::new(3));
        let mut m = Mut {
            value: &mut value,
            added: Tick::new(5),
            changed_tick: &changed_cell as *const UnsafeCell<Tick>,
            last_run: Tick::new(2),
            this_run: Tick::new(10),
            deref_mut_called: false,
        };
        let wrote = m.set_if_neq(MyComp(7));
        assert!(!wrote, "set_if_neq with equal value must return false");
        // SAFETY: changed_cell live in this scope.
        let observed = unsafe { *changed_cell.get() };
        assert_eq!(
            observed,
            Tick::new(3),
            "set_if_neq with equal value must NOT bump the tick"
        );
        assert_eq!(value.0, 7, "set_if_neq with equal value must not modify");
    }

    /// `Mut<T>::set_if_neq` MUST bump the changed tick when the new value
    /// differs.
    #[test]
    fn mut_set_if_neq_bumps_tick_when_different() {
        let mut value = MyComp(7);
        let changed_cell = UnsafeCell::new(Tick::new(3));
        let mut m = Mut {
            value: &mut value,
            added: Tick::new(5),
            changed_tick: &changed_cell as *const UnsafeCell<Tick>,
            last_run: Tick::new(2),
            this_run: Tick::new(10),
            deref_mut_called: false,
        };
        let wrote = m.set_if_neq(MyComp(99));
        assert!(wrote, "set_if_neq with new value must return true");
        // SAFETY: changed_cell live in this scope.
        let observed = unsafe { *changed_cell.get() };
        assert_eq!(observed, Tick::new(10), "set_if_neq must bump the tick to this_run");
        assert_eq!(value.0, 99);
    }

    /// `Mut<T>::bypass_change_detection` MUST NOT bump the changed tick even
    /// after the returned `&mut T` is used to mutate the value (plan §2.5
    /// MUT5).
    #[test]
    fn mut_bypass_change_detection_no_tick_bump() {
        let mut value = MyComp(1);
        let changed_cell = UnsafeCell::new(Tick::new(3));
        let mut m = Mut {
            value: &mut value,
            added: Tick::new(5),
            changed_tick: &changed_cell as *const UnsafeCell<Tick>,
            last_run: Tick::new(2),
            this_run: Tick::new(10),
            deref_mut_called: false,
        };
        let raw = m.bypass_change_detection();
        *raw = MyComp(50);
        // SAFETY: changed_cell live in this scope.
        let observed = unsafe { *changed_cell.get() };
        assert_eq!(
            observed,
            Tick::new(3),
            "bypass_change_detection must NOT bump the tick"
        );
        assert_eq!(value.0, 50);
    }

    /// `Mut<T>::is_changed` reflects the current tick slot (re-read each
    /// call). After `deref_mut` bumps the tick, `is_changed` MUST report
    /// true even though the write came from this same system.
    #[test]
    fn mut_is_changed_after_self_write_observes_change() {
        let mut value = MyComp(1);
        let changed_cell = UnsafeCell::new(Tick::new(0));
        let mut m = Mut {
            value: &mut value,
            added: Tick::new(5),
            changed_tick: &changed_cell as *const UnsafeCell<Tick>,
            last_run: Tick::new(2),
            this_run: Tick::new(10),
            deref_mut_called: false,
        };
        // Before the self-write: tick=0 < last_run=2 → not changed.
        assert!(!m.is_changed());
        // Trigger deref_mut → bumps tick to this_run=10.
        *m = MyComp(2);
        // After the self-write: tick=10 ∈ [2, 10] → is_changed reports true
        // (plan §6.2-bis worked proof).
        assert!(
            m.is_changed(),
            "self-write within the same system MUST report as changed"
        );
    }

    /// Phase 12.5 Track B NCD5 — backstop test.
    ///
    /// `Ref<T>::set_table_readonly_no_meta` is the meta-free dispatch
    /// variant. For `NEEDS_CHANGE_DETECTION = true` impls (which `Ref<T>`
    /// is) the body is a `#[cold]` `panic!()` — reaching it means the
    /// NCD6 const-fold dispatcher routed the wrong way. This test pins
    /// the panic at the trait level so any future regression that drops
    /// the backstop fails loudly.
    #[test]
    fn query_data_no_meta_panic_for_ref() {
        use std::panic::{self, AssertUnwindSafe};
        register_test_components();
        let mut ecs = EcsMaster::new();
        let state = <Ref<'_, MyComp> as QueryData>::init_state(&mut ecs);
        let mut fetch = <Ref<'_, MyComp> as QueryData>::init_fetch(&state);
        // The body never reads the `archetype` argument — it panics
        // unconditionally — so a dangling pointer is fine here.
        let archetype: *const Archetype = std::ptr::null();
        // SAFETY: the trait method is `unsafe`, but in this test it is
        //   the *panic itself* we exercise — the body short-circuits
        //   before reading the dangling pointer, so the contract
        //   violation (null archetype) never observes any UB.
        let result = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            <Ref<'_, MyComp> as QueryData>::set_table_readonly_no_meta(
                &mut fetch,
                &state,
                archetype,
            );
        }));
        assert!(
            result.is_err(),
            "Ref<T>::set_table_readonly_no_meta MUST panic (NCD5 backstop)"
        );
    }
}
