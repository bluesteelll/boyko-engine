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
}
