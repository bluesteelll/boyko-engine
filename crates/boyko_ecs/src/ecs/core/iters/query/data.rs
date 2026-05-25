//! `QueryData` trait — typed component access for queries.
//!
//! Step 2 lands the trait body and the two leaf impls (`&T`, `&mut T`). The
//! variadic tuple impls follow in Step 4; the per-row access in iterators
//! follows in Step 7.
//!
//! See Phase 8b plan §4 for the full design rationale.

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::params::diagnostics::intra_system_conflict_panic;
use crate::ecs::identifiers::primitives::ComponentId;

/// Maximum tuple arity supported by [`QueryData`] variadic impls.
///
/// Tuples beyond this arity trip a `const { panic!() }` in Step 4 — the limit
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
    /// # Safety
    ///
    /// * `archetype` MUST be a live `*const Archetype` for `'w`, with
    ///   provenance from `UnsafeEcsCell::archetype_ptr(id)` (read-only mint).
    /// * `archetype` MUST contain every [`ComponentId`] in `state`.
    /// * For `D` containing `&mut T`, this method MUST NOT be called. Impls
    ///   for `&mut T` `panic!()` here as a runtime backstop; the type-level
    ///   `D: ReadOnlyQueryData` bound on `Query::iter()` prevents this in
    ///   well-typed code.
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    );

    /// Sets the `Fetch`'s cached column pointers from a write-capable
    /// archetype pointer. Called by `QueryIterMut::next` (the mutable cursor).
    ///
    /// # Safety
    ///
    /// * `archetype` MUST be a live `*mut Archetype` for `'w`, with
    ///   write-capable provenance from `UnsafeEcsCell::archetype_ptr_mut(id)`.
    /// * `archetype` MUST contain every [`ComponentId`] in `state`.
    unsafe fn set_table_mut<'w>(
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
    ) {
        // For `&T`, the mutable variant degrades to the same read. Re-borrow
        // as `*const` internally; no write-capable provenance is consumed.
        // SAFETY (QD3, QD4): same conditions as `set_table_readonly` with the
        //   additional caller guarantee that `archetype` carries fresh
        //   `archetype_ptr_mut` provenance — strictly stronger than what we
        //   need here.
        unsafe { Self::set_table_readonly(fetch, state, archetype as *const _) }
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
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (QD3, QD4): forwarded per-element; `archetype`
                    //   carries read-only provenance and is identical for
                    //   every element. The caller of the tuple impl
                    //   upheld QD3/QD4 for every `$D`.
                    unsafe { <$D as QueryData>::set_table_readonly($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn set_table_mut<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *mut Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (QD3, QD4): write-capable `archetype` is
                    //   forwarded to every element; the caller upheld
                    //   QD3/QD4.
                    unsafe { <$D as QueryData>::set_table_mut($f, $s, archetype); }
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
// macro-expansion time, breaking the wider crate. `const { panic!() }`
// requires `rustc >= 1.79`; boyko targets Rust 2024 (`>= 1.85`).

/// Emits a stub `QueryData` impl whose every method body is
/// `const { panic!(...) }`. The const block fires at monomorphization;
/// the impl is never *successfully* used at runtime. `State`, `Fetch<'w>`,
/// and `Item<'w>` collapse to `()` so the stub type-checks in isolation.
macro_rules! impl_query_data_tuple_too_large {
    ( $( ($D:ident, $s:ident, $f:ident) ),* ) => {
        // SAFETY: stub impl whose every method body is `const { panic!(...) }`.
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

            fn init_state(_world: &mut EcsMaster) -> Self::State {
                const {
                    panic!(
                        "tuple has too many QueryData elements. \
                         boyko-engine supports up to \
                         MAX_QUERY_DATA_ARITY = 12. Split your query into \
                         smaller queries or wrap related elements in a \
                         struct that implements QueryData."
                    )
                }
            }

            fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {
                const { panic!("tuple too large: see init_state diagnostic") }
            }

            fn matches_component_set(_state: &Self::State, _mask: &ComponentMask) -> bool {
                const { panic!("tuple too large: see init_state diagnostic") }
            }

            fn aggregate_include(_state: &Self::State, _include: &mut ComponentMask) {
                const { panic!("tuple too large: see init_state diagnostic") }
            }

            fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
                const { panic!("tuple too large: see init_state diagnostic") }
            }

            unsafe fn set_table_readonly<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *const Archetype,
            ) {
                const { panic!("tuple too large: see init_state diagnostic") }
            }

            unsafe fn set_table_mut<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *mut Archetype,
            ) {
                const { panic!("tuple too large: see init_state diagnostic") }
            }

            unsafe fn fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> Self::Item<'w> {
                const { panic!("tuple too large: see init_state diagnostic") }
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
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct MyComp(#[allow(dead_code)] u32);

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
    fn ref_t_is_read_only() {
        assert!(
            <&MyComp as QueryData>::IS_READ_ONLY,
            "&T must report IS_READ_ONLY = true"
        );
    }

    #[test]
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
    fn tuple_2_all_read_is_read_only() {
        assert!(
            <(&MyComp, &OtherComp) as QueryData>::IS_READ_ONLY,
            "all-read tuple must AND-fold IS_READ_ONLY = true"
        );
    }

    #[test]
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
}
