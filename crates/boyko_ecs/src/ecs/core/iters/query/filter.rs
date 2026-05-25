//! `QueryFilter` trait — per-archetype and per-row filtering for queries.
//!
//! Step 3 of the Phase 8b roll-out: lands the full trait surface plus the
//! three archetypal filters required by 8b (`()`, `With<C>`, `Without<C>`).
//! Tuple impls and the post-filter `Or<F>` combinator arrive in Step 4.
//!
//! # Design — M2 split
//!
//! `set_table` is split into `set_table_readonly(*const Archetype)` and
//! `set_table_mut(*mut Archetype)` mirroring [`super::data::QueryData`]. The
//! Phase 8b filter set is fully archetypal — both methods are no-ops here —
//! but Phase 10 (`Changed<C>` / `Added<C>`) will need the kind-correct split
//! to avoid `*const → *mut` casts under Tree Borrows.
//!
//! # Per-row predicate
//!
//! `filter_fetch` is the per-row gate. For archetypal-only filters
//! (`IS_ARCHETYPAL = true`) it returns `true` unconditionally and the
//! iterator's `if const { F::IS_ARCHETYPAL } { ... }` branch elides the call
//! entirely at monomorphisation time.

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::params::diagnostics::intra_system_conflict_panic;
use crate::ecs::identifiers::primitives::ComponentId;

/// Filter applied to query matches.
///
/// Implemented by:
/// * `()` — the no-op filter.
/// * `With<C>` — archetypes containing `C`.
/// * `Without<C>` — archetypes NOT containing `C`.
/// * Tuples and `Or<F>` (Step 4).
///
/// # Trait shape
///
/// Mirrors [`super::data::QueryData`] minus the per-row `Item`/`fetch`. Three
/// associated items + a const flag:
///
/// * `State` — per-system cached metadata (e.g. resolved `ComponentId`s).
///   `Send + Sync + 'static` for Phase 9 cross-thread migration.
/// * `Fetch<'w>` — per-archetype scratch cached by `set_table_*`. For all
///   Phase 8b filters this is `()` (archetypal-only — no per-row state).
/// * `IS_ARCHETYPAL` — compile-time flag enabling const-fold of the per-row
///   `filter_fetch` branch in the iterator.
///
/// # Split set_table (M2)
///
/// `set_table_readonly(_: *const Archetype)` is called from `QueryIter`
/// (read-only cursor); `set_table_mut(_: *mut Archetype)` from `QueryIterMut`
/// (mutable cursor). Phase 8b filters cache no per-archetype state so both
/// bodies are no-ops, but the split is part of the public contract for
/// Phase 10 filters that will fetch column pointers.
///
/// # Safety
///
/// Implementations MUST uphold:
///
/// 1. **QF1** — if `IS_ARCHETYPAL = true`, `filter_fetch` returns `true`
///    unconditionally; the iterator may skip calling it.
/// 2. **QF2** — `init_access` declares every component read performed in
///    `filter_fetch`. Archetypal-only filters declare nothing.
/// 3. **QF3** — `Fetch<'w>` lifetime is scoped to the `*const/*mut Archetype`
///    minted by [`super::super::super::ecs_master::unsafe_ecs_cell::UnsafeEcsCell`]
///    for `'w`. Phase 8b filters do not cache pointers; the lifetime is vacuous.
pub unsafe trait QueryFilter: Sized {
    /// Long-lived per-system state (resolved IDs, cached masks).
    type State: Send + Sync + 'static;

    /// Per-archetype scratch cached by `set_table_*` and consumed by
    /// `filter_fetch`. `Copy` so tuple destructuring (Step 4) works.
    type Fetch<'w>: Copy;

    /// `true` iff `filter_fetch` returns `true` unconditionally.
    ///
    /// Const-folded by the iterator's `if const { F::IS_ARCHETYPAL }` branch
    /// — the entire per-row predicate vanishes at monomorphisation when set.
    const IS_ARCHETYPAL: bool;

    /// Builds the per-system state. Called once at system registration.
    fn init_state(world: &mut EcsMaster) -> Self::State;

    /// Declares the filter's access surface to the intra-system aliasing
    /// detector. Archetypal-only filters declare nothing.
    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet);

    /// Archetype-level predicate. Called once per archetype during
    /// `QueryDataState::post_filter_matched` and `update_archetypes`.
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool;

    /// Contributes bits to the `include` mask passed to `QueryState`'s
    /// archetype match cache. Default no-op for filters that cannot be
    /// reduced to a single positive include bit (e.g. `Or<F>`).
    #[inline]
    fn aggregate_include(_state: &Self::State, _include: &mut ComponentMask) {}

    /// Contributes bits to the `exclude` mask. Default no-op.
    #[inline]
    fn aggregate_exclude(_state: &Self::State, _exclude: &mut ComponentMask) {}

    /// Creates the initial per-archetype `Fetch` slot (typically all-NULL or
    /// the unit `()`).
    fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w>;

    /// Refreshes the `Fetch` from a read-only archetype pointer.
    ///
    /// Called by `QueryIter::next` when crossing into a new archetype.
    ///
    /// # Safety
    ///
    /// * `archetype` MUST be a live `*const Archetype` for `'w` with
    ///   provenance from `UnsafeEcsCell::archetype_ptr(id)`.
    /// * `archetype` MUST satisfy `matches_component_set(state, archetype.mask())`.
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    );

    /// Refreshes the `Fetch` from a write-capable archetype pointer.
    ///
    /// Called by `QueryIterMut::next` when crossing into a new archetype.
    ///
    /// # Safety
    ///
    /// * `archetype` MUST be a live `*mut Archetype` for `'w` with
    ///   write-capable provenance from `UnsafeEcsCell::archetype_ptr_mut(id)`.
    /// * `archetype` MUST satisfy `matches_component_set(state, archetype.mask())`.
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    );

    /// Per-row predicate. Archetypal filters return `true` unconditionally
    /// (QF1); only Phase 10 component-level filters do real work here.
    ///
    /// # Safety
    ///
    /// * `fetch` MUST have been initialised by a prior `set_table_*` call.
    /// * `row` MUST be in range for the cached archetype.
    /// * The iterator MUST skip calling this when `IS_ARCHETYPAL = true`.
    unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool;
}

// ── () — empty filter ───────────────────────────────────────────────────────

// SAFETY (QF1, QF2, QF3):
//   - QF1: `IS_ARCHETYPAL = true`; `filter_fetch` returns `true`.
//   - QF2: `init_access` declares nothing — `()` reads nothing.
//   - QF3: `Fetch<'w> = ()` — no pointers cached, lifetime vacuous.
unsafe impl QueryFilter for () {
    type State = ();
    type Fetch<'w> = ();
    const IS_ARCHETYPAL: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {}

    #[inline]
    fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {}

    #[inline]
    fn matches_component_set(_state: &Self::State, _mask: &ComponentMask) -> bool {
        true
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {}

    #[inline]
    unsafe fn set_table_readonly<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // SAFETY: no-op archetypal filter, no per-archetype state cached.
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {
        // SAFETY: no-op archetypal filter, no per-archetype state cached.
    }

    #[inline]
    unsafe fn filter_fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> bool {
        // SAFETY: archetypal filter (QF1) — returns true unconditionally.
        true
    }
}

// ── With<C> ─────────────────────────────────────────────────────────────────

/// Archetype-level inclusion filter: matches archetypes that contain `C`.
///
/// `With<C>` does not yield the component into the query item — only the
/// archetype-level predicate is contributed. To read `C`, pair it with
/// `&C` (or `&mut C`) in the `QueryData` slot of `Query<D, F>`.
///
/// # Access surface
///
/// `With<C>` declares a **read** of `C` in `init_access`. The filter inspects
/// `C`'s presence at the archetype level; the conservative declaration keeps
/// the intra-system aliasing detector consistent with a sibling `&mut C`
/// param (which would otherwise alias the filter's mask read).
pub struct With<C: Component> {
    _marker: PhantomData<fn() -> C>,
}

/// Per-system cached state for [`With<C>`]: a resolved [`ComponentId`].
#[derive(Clone, Copy)]
pub struct WithState<C: Component> {
    pub(crate) id: ComponentId,
    pub(crate) _marker: PhantomData<fn() -> C>,
}

// SAFETY (QF1, QF2, QF3):
//   - QF1: `IS_ARCHETYPAL = true`; `filter_fetch` returns `true`.
//   - QF2: `init_access` declares a component read of `state.id`; the
//     filter's archetype predicate logically inspects `C`'s presence bit.
//   - QF3: `Fetch<'w> = ()` — no pointers cached.
unsafe impl<C: Component> QueryFilter for With<C> {
    type State = WithState<C>;
    type Fetch<'w> = ();
    const IS_ARCHETYPAL: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        WithState { id: C::component_id(), _marker: PhantomData }
    }

    #[inline]
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
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {}

    #[inline]
    unsafe fn set_table_readonly<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // SAFETY: no-op archetypal filter, no per-archetype state cached.
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {
        // SAFETY: no-op archetypal filter, no per-archetype state cached.
    }

    #[inline]
    unsafe fn filter_fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> bool {
        // SAFETY: archetypal filter (QF1) — returns true unconditionally.
        true
    }
}

// ── Without<C> ──────────────────────────────────────────────────────────────

/// Archetype-level exclusion filter: matches archetypes that do NOT contain `C`.
///
/// # Access surface
///
/// `Without<C>` declares **no** access. It inspects the absence of a bit; it
/// performs no read of `C`'s data and cannot conflict with any sibling param.
pub struct Without<C: Component> {
    _marker: PhantomData<fn() -> C>,
}

/// Per-system cached state for [`Without<C>`]: a resolved [`ComponentId`].
#[derive(Clone, Copy)]
pub struct WithoutState<C: Component> {
    pub(crate) id: ComponentId,
    pub(crate) _marker: PhantomData<fn() -> C>,
}

// SAFETY (QF1, QF2, QF3):
//   - QF1: `IS_ARCHETYPAL = true`; `filter_fetch` returns `true`.
//   - QF2: `init_access` declares nothing — exclusion inspects bit absence
//     only, never accesses `C`'s data.
//   - QF3: `Fetch<'w> = ()` — no pointers cached.
unsafe impl<C: Component> QueryFilter for Without<C> {
    type State = WithoutState<C>;
    type Fetch<'w> = ();
    const IS_ARCHETYPAL: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        WithoutState { id: C::component_id(), _marker: PhantomData }
    }

    #[inline]
    fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {}

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        !mask.contains(state.id)
    }

    #[inline]
    fn aggregate_exclude(state: &Self::State, exclude: &mut ComponentMask) {
        exclude.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {}

    #[inline]
    unsafe fn set_table_readonly<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // SAFETY: no-op archetypal filter, no per-archetype state cached.
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {
        // SAFETY: no-op archetypal filter, no per-archetype state cached.
    }

    #[inline]
    unsafe fn filter_fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> bool {
        // SAFETY: archetypal filter (QF1) — returns true unconditionally.
        true
    }
}

// ── Or<F> combinator (§5.4, M8) ─────────────────────────────────────────────

/// Disjunctive filter combinator: matches an archetype iff ANY element of
/// the inner tuple matches.
///
/// `Or<F>` is non-decomposable into the simple `include`/`exclude` mask
/// aggregation pipeline used by `QueryDataState`. Per **M8** the
/// `aggregate_include` / `aggregate_exclude` methods are explicit no-op
/// overrides — emitting them here prevents a future contributor from
/// adding a non-trivial default that would silently break the
/// post-filter contract.
///
/// # Complexity (C4)
///
/// `Query<(), Or<F>>` (empty include + Or filter) populates
/// `matched_ids` with every live archetype (`update_archetypes`'s base
/// case), then `post_filter_matched` scans linearly: O(archetype_count
/// × Or-arity). Worst case at boyko's 1024-archetype ceiling and
/// arity-12 inner tuple: ~60 µs per generation bump (see plan §6.4 /
/// §15.5). Acceptable: cold path only, per generation bump.
///
/// # Phase-10 contribution language
///
/// Phase 10's `Changed<C>` / `Added<C>` filters will retrofit `Or` to
/// short-circuit on the first per-row `filter_fetch` match (currently
/// the `IS_ARCHETYPAL = true` const-fold makes the per-row method a
/// no-op; the OR composition will degenerate from "all archetypal" to
/// "mixed archetypal + per-row" in Phase 10).
pub struct Or<F>(PhantomData<fn() -> F>);

// ── Tuple-as-AND impl (§5.6) ────────────────────────────────────────────────
//
// A tuple of `QueryFilter` types implements `QueryFilter` itself with
// matches_component_set folded as AND. Same paired-ident macro shape as
// the `QueryData` tuple impl (§4.6) — see `data.rs` for the
// invocation pattern.

/// Emits a `QueryFilter` impl for a tuple `(F0, F1, ..)` with the
/// match semantics folded as AND. Paired-ident invocation per plan
/// §5.6 — `(TypeIdent, state_value_ident, fetch_value_ident)`.
macro_rules! impl_query_filter_tuple_and {
    ( $( ($F:ident, $s:ident, $f:ident) ),* ) => {
        // SAFETY (QF1, QF2, QF3): the tuple impl forwards every method
        //   to its per-element delegate, which upholds QF1/QF2/QF3 by
        //   its own contract. `IS_ARCHETYPAL` folds as AND over the
        //   elements — the tuple is archetypal iff every element is.
        #[allow(non_snake_case)]
        unsafe impl< $($F: QueryFilter),* > QueryFilter for ( $($F,)* ) {
            type State = ( $($F::State,)* );
            type Fetch<'w> = ( $($F::Fetch<'w>,)* );
            const IS_ARCHETYPAL: bool = true $( && $F::IS_ARCHETYPAL )*;

            #[inline]
            fn init_state(world: &mut EcsMaster) -> Self::State {
                ( $( <$F as QueryFilter>::init_state(world), )* )
            }

            #[inline]
            fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
                let ( $($s,)* ) = state;
                $( <$F as QueryFilter>::init_access($s, access_set); )*
            }

            #[inline]
            fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
                let ( $($s,)* ) = state;
                true $( && <$F as QueryFilter>::matches_component_set($s, mask) )*
            }

            #[inline]
            fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
                let ( $($s,)* ) = state;
                $( <$F as QueryFilter>::aggregate_include($s, include); )*
            }

            #[inline]
            fn aggregate_exclude(state: &Self::State, exclude: &mut ComponentMask) {
                let ( $($s,)* ) = state;
                $( <$F as QueryFilter>::aggregate_exclude($s, exclude); )*
            }

            #[inline]
            fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
                let ( $($s,)* ) = state;
                ( $( <$F as QueryFilter>::init_fetch($s), )* )
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
                    // SAFETY (QF3): forwarded per-element; `archetype`
                    //   carries read-only provenance and is identical for
                    //   every element.
                    unsafe { <$F as QueryFilter>::set_table_readonly($f, $s, archetype); }
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
                    // SAFETY (QF3): write-capable `archetype` forwarded
                    //   per-element.
                    unsafe { <$F as QueryFilter>::set_table_mut($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
                if const { Self::IS_ARCHETYPAL } {
                    return true;
                }
                let ( $($f,)* ) = fetch;
                true $(
                    // SAFETY (QF1): per-element contract; `row` in range.
                    && unsafe { <$F as QueryFilter>::filter_fetch($f, row) }
                )*
            }
        }
    };
}

impl_query_filter_tuple_and!((F0, s0, f0));
impl_query_filter_tuple_and!((F0, s0, f0), (F1, s1, f1));
impl_query_filter_tuple_and!((F0, s0, f0), (F1, s1, f1), (F2, s2, f2));
impl_query_filter_tuple_and!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3)
);
impl_query_filter_tuple_and!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4)
);
impl_query_filter_tuple_and!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5)
);
impl_query_filter_tuple_and!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6)
);
impl_query_filter_tuple_and!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7)
);
impl_query_filter_tuple_and!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8)
);
impl_query_filter_tuple_and!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9)
);
impl_query_filter_tuple_and!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10)
);
impl_query_filter_tuple_and!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11)
);

// ── Or<(...)> impl (§5.4, M8) ───────────────────────────────────────────────
//
// `Or<(F0, F1, ..)>` matches iff ANY element matches. The
// `aggregate_include` / `aggregate_exclude` methods are explicit no-op
// overrides (M8) — `Or`'s match predicate is non-decomposable into the
// simple positive/negative mask aggregation used by `QueryDataState`'s
// `update_archetypes` path. The OR semantics are enforced by the
// post-filter pass in `QueryDataState`, which calls
// `Or::matches_component_set` per matched archetype id.
//
// `IS_ARCHETYPAL` folds as AND over elements — `Or` is archetypal iff
// every element is. Per O-NEW-2 the AND formula is the conservatively
// correct const-fold: if any element is non-archetypal (e.g. Phase 10
// `Changed<C>`), `Or` must call the per-row `filter_fetch`. A future
// optimisation may exploit OR semantics directly (`Or<archetypal,
// non-archetypal>` could elide per-row when the archetypal half
// matches), but that is deferred to Phase 10+ once tick-based filters
// are real.

/// Emits a `QueryFilter` impl for `Or<(F0, F1, ..)>`. Paired-ident
/// invocation per plan §5.4 (M8 — explicit `aggregate_*` no-op
/// overrides). `matches_component_set` folds as OR; `filter_fetch`
/// short-circuits at the first match.
macro_rules! impl_or_filter_tuple {
    ( $( ($F:ident, $s:ident, $f:ident) ),* ) => {
        // SAFETY (QF1, QF2, QF3):
        //   - QF1: `IS_ARCHETYPAL` is the AND of element flags. When
        //     `true`, every element's `filter_fetch` returns `true`
        //     unconditionally, so the OR fold trivially returns `true`.
        //     When `false`, the per-row `filter_fetch` performs an OR
        //     short-circuit fold over element `filter_fetch` calls.
        //   - QF2: `init_access` forwards to every element; each declares
        //     its own reads. The OR semantics do not change the access
        //     surface — every component touched by any inner filter is
        //     read.
        //   - QF3: per-element forwarding; `archetype` shared across
        //     elements.
        #[allow(non_snake_case)]
        unsafe impl< $($F: QueryFilter),* > QueryFilter for Or<( $($F,)* )> {
            type State = ( $($F::State,)* );
            type Fetch<'w> = ( $($F::Fetch<'w>,)* );
            const IS_ARCHETYPAL: bool = true $( && $F::IS_ARCHETYPAL )*;

            #[inline]
            fn init_state(world: &mut EcsMaster) -> Self::State {
                ( $( <$F as QueryFilter>::init_state(world), )* )
            }

            #[inline]
            fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
                let ( $($s,)* ) = state;
                $( <$F as QueryFilter>::init_access($s, access_set); )*
            }

            #[inline]
            fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
                let ( $($s,)* ) = state;
                false $( || <$F as QueryFilter>::matches_component_set($s, mask) )*
            }

            // M8: explicit no-op overrides — `Or`'s match predicate is
            // non-decomposable into simple include/exclude mask
            // aggregation. The OR semantics are enforced by
            // `QueryDataState::post_filter_matched`. Locked here to
            // prevent a future contributor reaching for a default
            // forwarding impl that would silently break the contract.
            #[inline]
            fn aggregate_include(_state: &Self::State, _include: &mut ComponentMask) {}

            #[inline]
            fn aggregate_exclude(_state: &Self::State, _exclude: &mut ComponentMask) {}

            #[inline]
            fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
                let ( $($s,)* ) = state;
                ( $( <$F as QueryFilter>::init_fetch($s), )* )
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
                    // SAFETY (QF3): per-element forwarding; `archetype`
                    //   carries read-only provenance.
                    unsafe { <$F as QueryFilter>::set_table_readonly($f, $s, archetype); }
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
                    // SAFETY (QF3): write-capable `archetype` forwarded
                    //   per-element.
                    unsafe { <$F as QueryFilter>::set_table_mut($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
                if const { Self::IS_ARCHETYPAL } {
                    return true;
                }
                let ( $($f,)* ) = fetch;
                false $(
                    // SAFETY (QF1): per-element contract; `row` in range.
                    || unsafe { <$F as QueryFilter>::filter_fetch($f, row) }
                )*
            }
        }
    };
}

impl_or_filter_tuple!((F0, s0, f0));
impl_or_filter_tuple!((F0, s0, f0), (F1, s1, f1));
impl_or_filter_tuple!((F0, s0, f0), (F1, s1, f1), (F2, s2, f2));
impl_or_filter_tuple!((F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3));
impl_or_filter_tuple!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4)
);
impl_or_filter_tuple!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5)
);
impl_or_filter_tuple!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6)
);
impl_or_filter_tuple!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7)
);
impl_or_filter_tuple!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8)
);
impl_or_filter_tuple!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9)
);
impl_or_filter_tuple!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10)
);
impl_or_filter_tuple!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11)
);

// ── Arity-overflow stubs (arity 13..=24) — M7 + C-NEW-2 ────────────────────
//
// Same pattern as `data.rs::impl_query_data_tuple_too_large!`: each
// method body is `panic!(...)`, evaluated only at
// monomorphization. The stub `QueryFilter for (F0, .., F12, ..)` impl
// has `State = ()` / `Fetch<'w> = ()` so it type-checks in isolation.
//
// Two stub macros, one for the tuple-as-AND impl and one for
// `Or<(...)>`, so the two impl shapes can co-exist.

/// Emits a stub `QueryFilter` impl for an oversized tuple-as-AND. Every
/// method body is `panic!(...)`; the panic fires at
/// monomorphization, not at macro-expand.
macro_rules! impl_query_filter_tuple_and_too_large {
    ( $( ($F:ident, $s:ident, $f:ident) ),* ) => {
        // SAFETY: stub impl with `panic!(...)` method bodies.
        //   The const block prevents successful instantiation at runtime;
        //   QF1/QF2/QF3 are vacuously upheld.
        #[allow(non_snake_case, unused_variables)]
        unsafe impl< $($F: QueryFilter),* > QueryFilter for ( $($F,)* ) {
            type State = ();
            type Fetch<'w> = ();
            const IS_ARCHETYPAL: bool = true;

            fn init_state(_world: &mut EcsMaster) -> Self::State {
                panic!(
                        "tuple has too many QueryFilter elements. \
                         boyko-engine supports up to arity 12. Split your \
                         filter into smaller tuples or wrap related \
                         filters in a struct that implements QueryFilter."
                    )
            }

            fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {
                panic!("tuple too large: see init_state diagnostic")
            }

            fn matches_component_set(_state: &Self::State, _mask: &ComponentMask) -> bool {
                panic!("tuple too large: see init_state diagnostic")
            }

            fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
                panic!("tuple too large: see init_state diagnostic")
            }

            unsafe fn set_table_readonly<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *const Archetype,
            ) {
                panic!("tuple too large: see init_state diagnostic")
            }

            unsafe fn set_table_mut<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *mut Archetype,
            ) {
                panic!("tuple too large: see init_state diagnostic")
            }

            unsafe fn filter_fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> bool {
                panic!("tuple too large: see init_state diagnostic")
            }
        }
    };
}

/// Emits a stub `QueryFilter` impl for `Or<(F0, .., F12, ..)>`. Every
/// method body is `panic!(...)`; the panic fires at
/// monomorphization.
macro_rules! impl_or_filter_tuple_too_large {
    ( $( ($F:ident, $s:ident, $f:ident) ),* ) => {
        // SAFETY: stub impl with `panic!(...)` method bodies.
        #[allow(non_snake_case, unused_variables)]
        unsafe impl< $($F: QueryFilter),* > QueryFilter for Or<( $($F,)* )> {
            type State = ();
            type Fetch<'w> = ();
            const IS_ARCHETYPAL: bool = true;

            fn init_state(_world: &mut EcsMaster) -> Self::State {
                panic!(
                        "Or<F> has too many QueryFilter elements. \
                         boyko-engine supports up to arity 12. Split your \
                         filter into smaller Or tuples or wrap related \
                         filters in a struct that implements QueryFilter."
                    )
            }

            fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {
                panic!("Or<F> too large: see init_state diagnostic")
            }

            fn matches_component_set(_state: &Self::State, _mask: &ComponentMask) -> bool {
                panic!("Or<F> too large: see init_state diagnostic")
            }

            fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
                panic!("Or<F> too large: see init_state diagnostic")
            }

            unsafe fn set_table_readonly<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *const Archetype,
            ) {
                panic!("Or<F> too large: see init_state diagnostic")
            }

            unsafe fn set_table_mut<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *mut Archetype,
            ) {
                panic!("Or<F> too large: see init_state diagnostic")
            }

            unsafe fn filter_fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> bool {
                panic!("Or<F> too large: see init_state diagnostic")
            }
        }
    };
}

impl_query_filter_tuple_and_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12)
);
impl_query_filter_tuple_and_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13)
);
impl_query_filter_tuple_and_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14)
);
impl_query_filter_tuple_and_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15)
);
impl_query_filter_tuple_and_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16)
);
impl_query_filter_tuple_and_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17)
);
impl_query_filter_tuple_and_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17), (F18, s18, f18)
);
impl_query_filter_tuple_and_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17), (F18, s18, f18), (F19, s19, f19)
);
impl_query_filter_tuple_and_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17), (F18, s18, f18), (F19, s19, f19),
    (F20, s20, f20)
);
impl_query_filter_tuple_and_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17), (F18, s18, f18), (F19, s19, f19),
    (F20, s20, f20), (F21, s21, f21)
);
impl_query_filter_tuple_and_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17), (F18, s18, f18), (F19, s19, f19),
    (F20, s20, f20), (F21, s21, f21), (F22, s22, f22)
);
impl_query_filter_tuple_and_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17), (F18, s18, f18), (F19, s19, f19),
    (F20, s20, f20), (F21, s21, f21), (F22, s22, f22), (F23, s23, f23)
);

impl_or_filter_tuple_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12)
);
impl_or_filter_tuple_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13)
);
impl_or_filter_tuple_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14)
);
impl_or_filter_tuple_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15)
);
impl_or_filter_tuple_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16)
);
impl_or_filter_tuple_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17)
);
impl_or_filter_tuple_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17), (F18, s18, f18)
);
impl_or_filter_tuple_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17), (F18, s18, f18), (F19, s19, f19)
);
impl_or_filter_tuple_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17), (F18, s18, f18), (F19, s19, f19),
    (F20, s20, f20)
);
impl_or_filter_tuple_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17), (F18, s18, f18), (F19, s19, f19),
    (F20, s20, f20), (F21, s21, f21)
);
impl_or_filter_tuple_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17), (F18, s18, f18), (F19, s19, f19),
    (F20, s20, f20), (F21, s21, f21), (F22, s22, f22)
);
impl_or_filter_tuple_too_large!(
    (F0, s0, f0), (F1, s1, f1), (F2, s2, f2), (F3, s3, f3),
    (F4, s4, f4), (F5, s5, f5), (F6, s6, f6), (F7, s7, f7),
    (F8, s8, f8), (F9, s9, f9), (F10, s10, f10), (F11, s11, f11),
    (F12, s12, f12), (F13, s13, f13), (F14, s14, f14), (F15, s15, f15),
    (F16, s16, f16), (F17, s17, f17), (F18, s18, f18), (F19, s19, f19),
    (F20, s20, f20), (F21, s21, f21), (F22, s22, f22), (F23, s23, f23)
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component_registry;

    // Component IDs 150-159: reserved for Phase 8b QueryFilter unit tests
    // (see Phase 8b plan §17.5).
    #[repr(C)]
    struct A(#[allow(dead_code)] u32);
    #[repr(C)]
    struct B(#[allow(dead_code)] u32);

    impl Component for A {
        fn component_id() -> ComponentId {
            ComponentId(150)
        }
    }

    impl Component for B {
        fn component_id() -> ComponentId {
            ComponentId(151)
        }
    }

    fn register_components() {
        component_registry::register_layout::<A>(A::component_id().0);
        component_registry::register_layout::<B>(B::component_id().0);
    }

    // ── IS_ARCHETYPAL flags ─────────────────────────────────────────────

    #[test]
    fn unit_filter_is_archetypal() {
        assert!(<() as QueryFilter>::IS_ARCHETYPAL);
    }

    #[test]
    fn with_filter_is_archetypal() {
        assert!(<With<A> as QueryFilter>::IS_ARCHETYPAL);
    }

    #[test]
    fn without_filter_is_archetypal() {
        assert!(<Without<A> as QueryFilter>::IS_ARCHETYPAL);
    }

    // ── matches_component_set ───────────────────────────────────────────

    #[test]
    fn with_matches_component_set() {
        register_components();
        let state = WithState::<A> {
            id: A::component_id(),
            _marker: PhantomData,
        };

        let mut mask = ComponentMask::new();
        assert!(
            !<With<A> as QueryFilter>::matches_component_set(&state, &mask),
            "With<A> must reject mask without A"
        );

        mask.set(A::component_id());
        assert!(
            <With<A> as QueryFilter>::matches_component_set(&state, &mask),
            "With<A> must accept mask containing A"
        );
    }

    #[test]
    fn without_matches_component_set() {
        register_components();
        let state = WithoutState::<A> {
            id: A::component_id(),
            _marker: PhantomData,
        };

        let mut mask = ComponentMask::new();
        assert!(
            <Without<A> as QueryFilter>::matches_component_set(&state, &mask),
            "Without<A> must accept mask without A"
        );

        mask.set(A::component_id());
        assert!(
            !<Without<A> as QueryFilter>::matches_component_set(&state, &mask),
            "Without<A> must reject mask containing A"
        );
    }

    // ── aggregate_include / aggregate_exclude ───────────────────────────

    #[test]
    fn with_aggregate_sets_include_bit() {
        register_components();
        let state = WithState::<A> {
            id: A::component_id(),
            _marker: PhantomData,
        };

        let mut include = ComponentMask::new();
        let mut exclude = ComponentMask::new();
        <With<A> as QueryFilter>::aggregate_include(&state, &mut include);
        <With<A> as QueryFilter>::aggregate_exclude(&state, &mut exclude);

        assert!(
            include.contains(A::component_id()),
            "aggregate_include must set the include bit for A"
        );
        assert!(
            exclude.is_empty(),
            "With<A>::aggregate_exclude must be a no-op"
        );
    }

    #[test]
    fn without_aggregate_sets_exclude_bit() {
        register_components();
        let state = WithoutState::<A> {
            id: A::component_id(),
            _marker: PhantomData,
        };

        let mut include = ComponentMask::new();
        let mut exclude = ComponentMask::new();
        <Without<A> as QueryFilter>::aggregate_include(&state, &mut include);
        <Without<A> as QueryFilter>::aggregate_exclude(&state, &mut exclude);

        assert!(
            exclude.contains(A::component_id()),
            "aggregate_exclude must set the exclude bit for A"
        );
        assert!(
            include.is_empty(),
            "Without<A>::aggregate_include must be a no-op"
        );
    }

    // ── init_access ─────────────────────────────────────────────────────

    #[test]
    fn with_init_access_adds_read() {
        register_components();
        let state = WithState::<A> {
            id: A::component_id(),
            _marker: PhantomData,
        };

        let mut access_set = FilteredAccessSet::new();
        <With<A> as QueryFilter>::init_access(&state, &mut access_set);

        // After init_access, a sibling write of A must conflict, proving the
        // read bit was registered.
        let err = access_set
            .add_component_write(A::component_id(), "probe-writer")
            .expect_err("sibling write must conflict with With<A>'s declared read");
        assert_eq!(err.id, A::component_id().0);
    }

    #[test]
    fn without_init_access_adds_nothing() {
        register_components();
        let state = WithoutState::<A> {
            id: A::component_id(),
            _marker: PhantomData,
        };

        let mut access_set = FilteredAccessSet::new();
        <Without<A> as QueryFilter>::init_access(&state, &mut access_set);

        // No read or write registered: a probe write must succeed.
        access_set
            .add_component_write(A::component_id(), "probe-writer")
            .expect("Without<A>::init_access must declare nothing");

        // And a probe read must also succeed (no prior write blocks it).
        let mut access_set = FilteredAccessSet::new();
        <Without<A> as QueryFilter>::init_access(&state, &mut access_set);
        access_set
            .add_component_read(A::component_id(), "probe-reader")
            .expect("Without<A>::init_access must declare nothing");
    }

    // ── Variadic tuple / Or<F> tests (Step 4) ───────────────────────────

    /// Compile-only shim: instantiating `assert_impl::<T>()` proves `T`
    /// satisfies `QueryFilter`.
    fn assert_impl<T: QueryFilter>() {}

    #[test]
    fn tuple_2_is_query_filter() {
        assert_impl::<(With<A>, Without<B>)>();
    }

    #[test]
    fn tuple_2_with_and_without_is_archetypal() {
        // (With<A>, Without<B>) is the canonical archetypal-AND tuple.
        // AND-fold of two `true` flags must yield `true`.
        assert!(
            <(With<A>, Without<B>) as QueryFilter>::IS_ARCHETYPAL,
            "tuple of archetypal filters must AND-fold IS_ARCHETYPAL = true"
        );
    }

    #[test]
    fn tuple_2_and_matches_component_set() {
        register_components();
        let mut ecs = EcsMaster::new();
        let state = <(With<A>, Without<B>) as QueryFilter>::init_state(&mut ecs);

        // Mask with A only (no B) — both With<A> and Without<B> match.
        let mut mask = ComponentMask::new();
        mask.set(A::component_id());
        assert!(
            <(With<A>, Without<B>) as QueryFilter>::matches_component_set(&state, &mask),
            "(With<A>, Without<B>) must match mask with A only"
        );

        // Mask with A and B — With<A> matches, Without<B> fails → AND false.
        let mut mask_ab = ComponentMask::new();
        mask_ab.set(A::component_id());
        mask_ab.set(B::component_id());
        assert!(
            !<(With<A>, Without<B>) as QueryFilter>::matches_component_set(&state, &mask_ab),
            "(With<A>, Without<B>) must reject mask containing B"
        );

        // Empty mask — With<A> fails → AND false.
        let empty = ComponentMask::new();
        assert!(
            !<(With<A>, Without<B>) as QueryFilter>::matches_component_set(&state, &empty),
            "(With<A>, Without<B>) must reject mask without A"
        );
    }

    #[test]
    fn tuple_2_and_aggregates_include_and_exclude() {
        register_components();
        let mut ecs = EcsMaster::new();
        let state = <(With<A>, Without<B>) as QueryFilter>::init_state(&mut ecs);

        let mut include = ComponentMask::new();
        let mut exclude = ComponentMask::new();
        <(With<A>, Without<B>) as QueryFilter>::aggregate_include(&state, &mut include);
        <(With<A>, Without<B>) as QueryFilter>::aggregate_exclude(&state, &mut exclude);

        assert!(
            include.contains(A::component_id()),
            "tuple aggregate_include must forward With<A>'s contribution"
        );
        assert!(
            exclude.contains(B::component_id()),
            "tuple aggregate_exclude must forward Without<B>'s contribution"
        );
    }

    #[test]
    fn arity_12_query_filter_compiles() {
        // Twelve With<A> filters — the documented cap.
        assert_impl::<(
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
        )>();
    }

    // ── Or<F> tests ─────────────────────────────────────────────────────

    #[test]
    fn or_filter_is_archetypal_iff_all_inner_archetypal() {
        // Or of two archetypal filters must AND-fold IS_ARCHETYPAL = true.
        assert!(
            <Or<(With<A>, With<B>)> as QueryFilter>::IS_ARCHETYPAL,
            "Or<archetypal-only> must AND-fold IS_ARCHETYPAL = true"
        );
    }

    #[test]
    fn or_matches_component_set_or_semantics() {
        register_components();
        let mut ecs = EcsMaster::new();
        let state = <Or<(With<A>, With<B>)> as QueryFilter>::init_state(&mut ecs);

        // Mask with only A: With<A> matches → OR is true.
        let mut mask_a = ComponentMask::new();
        mask_a.set(A::component_id());
        assert!(
            <Or<(With<A>, With<B>)> as QueryFilter>::matches_component_set(&state, &mask_a),
            "Or<(With<A>, With<B>)> must match mask with only A"
        );

        // Mask with only B: With<B> matches → OR is true.
        let mut mask_b = ComponentMask::new();
        mask_b.set(B::component_id());
        assert!(
            <Or<(With<A>, With<B>)> as QueryFilter>::matches_component_set(&state, &mask_b),
            "Or<(With<A>, With<B>)> must match mask with only B"
        );

        // Mask with both: both With elements match → OR is true.
        let mut mask_ab = ComponentMask::new();
        mask_ab.set(A::component_id());
        mask_ab.set(B::component_id());
        assert!(
            <Or<(With<A>, With<B>)> as QueryFilter>::matches_component_set(&state, &mask_ab),
            "Or<(With<A>, With<B>)> must match mask with both A and B"
        );

        // Empty mask: neither element matches → OR is false.
        let empty = ComponentMask::new();
        assert!(
            !<Or<(With<A>, With<B>)> as QueryFilter>::matches_component_set(&state, &empty),
            "Or<(With<A>, With<B>)> must reject mask containing neither A nor B"
        );
    }

    #[test]
    fn or_aggregate_include_is_noop() {
        // M8 verification: Or<F> MUST emit explicit no-op aggregate_include
        // / aggregate_exclude overrides. The post-filter pass in
        // QueryDataState is the sole enforcement path for the OR
        // predicate; the include/exclude masks driving update_archetypes
        // must NOT carry any of Or's bits.
        register_components();
        let mut ecs = EcsMaster::new();
        let state = <Or<(With<A>, With<B>)> as QueryFilter>::init_state(&mut ecs);

        let mut include = ComponentMask::new();
        let mut exclude = ComponentMask::new();
        <Or<(With<A>, With<B>)> as QueryFilter>::aggregate_include(&state, &mut include);
        <Or<(With<A>, With<B>)> as QueryFilter>::aggregate_exclude(&state, &mut exclude);

        assert!(
            include.is_empty(),
            "Or<F>::aggregate_include must be a no-op (M8 contract)"
        );
        assert!(
            exclude.is_empty(),
            "Or<F>::aggregate_exclude must be a no-op (M8 contract)"
        );
    }

    #[test]
    fn or_arity_12_compiles() {
        // Or<F> with arity-12 inner tuple — the documented cap.
        assert_impl::<Or<(
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
            With<A>,
        )>>();
    }
}
