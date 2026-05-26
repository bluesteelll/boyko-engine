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

    /// Phase 12.5 Track B NCD1 — compile-time flag for change-detection use.
    ///
    /// `true` iff this `QueryFilter` reads per-row tick fields (`Added<C>`,
    /// `Changed<C>`, etc.). The dispatcher's NCD6 const-fold branches on
    /// `D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION` to pick
    /// between the meta-bearing and meta-free `set_table_*` variants.
    ///
    /// No default — every impl MUST declare (NCD5 / I4 invariant).
    /// Archetypal filters (`()`, `With<C>`, `Without<C>`) are `false`;
    /// tick-based filters (`Added<C>`, `Changed<C>`) are `true`;
    /// `Or<F>` propagates by AND/OR of inner elements (see NCD4).
    const NEEDS_CHANGE_DETECTION: bool;

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
    /// # Phase 10 Round 2 W7 — `meta` parameter
    ///
    /// `meta` carries the active system's per-frame tick snapshot
    /// (`last_run` / `this_run`). Non-archetypal filters (Wave C
    /// `Added<C>` / `Changed<C>`) copy the ticks by value into
    /// `Self::Fetch<'w>` so the per-row hot loop pays no indirection.
    /// Archetypal filters (`()`, `With<C>`, `Without<C>`) accept and
    /// ignore the parameter. **`meta` is read-only INPUT** — it must
    /// never be stored into the `Fetch<'w>` (the `Fetch`'s lifetime is
    /// `'w`, the meta's is anonymous-input).
    ///
    /// # Safety
    ///
    /// * `archetype` MUST be a live `*const Archetype` for `'w` with
    ///   provenance from `UnsafeEcsCell::archetype_ptr(id)`.
    /// * `archetype` MUST satisfy `matches_component_set(state, archetype.mask())`.
    /// * `meta` MUST reference the currently-active system's
    ///   [`SystemMeta`]; the ticks it carries must be the per-frame
    ///   snapshot published by `System::set_change_ticks` (Wave A).
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        meta: &'_ SystemMeta,
    );

    /// Refreshes the `Fetch` from a write-capable archetype pointer.
    ///
    /// Called by `QueryIterMut::next` when crossing into a new archetype.
    /// See [`Self::set_table_readonly`] for the `meta` contract.
    ///
    /// # Safety
    ///
    /// * `archetype` MUST be a live `*mut Archetype` for `'w` with
    ///   write-capable provenance from `UnsafeEcsCell::archetype_ptr_mut(id)`.
    /// * `archetype` MUST satisfy `matches_component_set(state, archetype.mask())`.
    /// * `meta` MUST reference the currently-active system's
    ///   [`SystemMeta`].
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,
    );

    /// Phase 12.5 Track B NCD5 — meta-free variant of
    /// [`Self::set_table_readonly`]. See the `QueryData` analogue for the
    /// dispatcher contract and the no-default-body rationale.
    ///
    /// # Safety
    ///
    /// Same contract as [`Self::set_table_readonly`] minus the `meta`
    /// invariant. NCD6 routes this method only when
    /// `Self::NEEDS_CHANGE_DETECTION == false`.
    unsafe fn set_table_readonly_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    );

    /// Phase 12.5 Track B NCD5 — meta-free variant of
    /// [`Self::set_table_mut`].
    ///
    /// # Safety
    ///
    /// Same contract as [`Self::set_table_mut`] minus the `meta` invariant.
    unsafe fn set_table_mut_no_meta<'w>(
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
    // Phase 12.5 Track B NCD2: vacuous — `()` reads no per-row state.
    const NEEDS_CHANGE_DETECTION: bool = false;

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
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY: no-op archetypal filter, no per-archetype state cached.
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY: no-op archetypal filter, no per-archetype state cached.
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // SAFETY: meta-free body for NCD = false — no-op (same as the
        //   meta-bearing variant minus the unused arg).
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {
        // SAFETY: same as the readonly no-meta variant.
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
    // Phase 12.5 Track B NCD2: `With<C>` reads only the archetype mask bit
    // (compile-time on the QueryDataState path); no per-row ticks.
    const NEEDS_CHANGE_DETECTION: bool = false;

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
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY: no-op archetypal filter, no per-archetype state cached.
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY: no-op archetypal filter, no per-archetype state cached.
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // SAFETY: meta-free body for NCD = false — no-op archetypal.
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {
        // SAFETY: same as the readonly no-meta variant.
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
    // Phase 12.5 Track B NCD2: `Without<C>` inspects bit absence; no ticks.
    const NEEDS_CHANGE_DETECTION: bool = false;

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
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY: no-op archetypal filter, no per-archetype state cached.
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY: no-op archetypal filter, no per-archetype state cached.
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // SAFETY: meta-free body for NCD = false — no-op archetypal.
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {
        // SAFETY: same as the readonly no-meta variant.
    }

    #[inline]
    unsafe fn filter_fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> bool {
        // SAFETY: archetypal filter (QF1) — returns true unconditionally.
        true
    }
}

// ── Added<C> (Phase 10 Wave C Step 9) ───────────────────────────────────────

/// Per-row change-detection filter: matches rows whose component `C` was
/// added since the active system's `last_run` tick (plan §2.3 FLT1-FLT8).
///
/// `Added<C>` is **non-archetypal** (`IS_ARCHETYPAL = false`) — the per-row
/// `filter_fetch` evaluates a tick comparison against the cached
/// `(last_run, this_run]` window.
///
/// # Composition
///
/// * In tuple-AND, e.g. `Query<&Pos, (With<Tag>, Added<Vel>)>`: yields rows
///   in archetypes containing both `Tag` and `Vel` whose `Vel.added_tick`
///   falls in the window.
/// * In `Or<F>`, e.g. `Or<(With<A>, Added<B>)>`: the archetype set walks
///   every archetype matched by `With<A>` OR by the presence of `B`. On
///   archetypes lacking `B`, the [`AddedFetch::tick_base`] pointer is NULL
///   and `filter_fetch` short-circuits to `false` (plan §5.4-bis — Round 2 C4
///   null-base branch).
///
/// # Access surface (plan FLT2 — conservative)
///
/// `init_access` declares a **read** of `C`. The filter inspects the per-row
/// `added_ticks` column (logically a property of `C`'s lifecycle), so the
/// read declaration keeps the intra-system aliasing detector consistent
/// with the rest of the trait surface (mirrors [`With<C>`]'s declaration).
/// Consequence for `Or<F>` composition: a system with
/// `Or<(_, Added<C>)>` is serialised by the Phase 9 conflict graph against
/// any concurrent writer of `C`, even on `C`-absent archetypes. Mirrors
/// Bevy; per-archetype access narrowing is deferred to a future phase.
pub struct Added<C: Component> {
    _marker: PhantomData<fn() -> C>,
}

/// Per-system cached state for [`Added<C>`]: a resolved [`ComponentId`].
#[derive(Clone, Copy)]
pub struct AddedState<C: Component> {
    pub(crate) id: ComponentId,
    pub(crate) _marker: PhantomData<fn() -> C>,
}

/// Per-archetype `Fetch` scratch for [`Added<C>`].
///
/// Caches the `added_ticks` base pointer for the current archetype plus the
/// active system's `(last_run, this_run]` snapshot. The per-row hot loop
/// reads a single `Tick` and calls [`Tick::is_newer_than`] — no
/// indirection beyond this struct (plan §10.3 per-row breakdown).
///
/// `tick_base` is NULL when [`Added<C>::set_table_readonly`] /
/// [`Added<C>::set_table_mut`] ran on an archetype that lacks `C` — legal
/// only inside `Or<F>` composition (plan §5.4-bis Round 2 C4). The
/// null-base branch in [`Added<C>::filter_fetch`] returns `false`.
pub struct AddedFetch<'w> {
    /// Base pointer to the active archetype's `added_ticks` column. NULL
    /// for `Or<F>`-only archetypes that lack `C` (plan §5.4-bis).
    pub(crate) tick_base: *const UnsafeCell<Tick>,
    /// Active system's `last_run` snapshot at `set_table_*` time.
    pub(crate) last_run: Tick,
    /// Active system's `this_run` snapshot at `set_table_*` time.
    pub(crate) this_run: Tick,
    /// Type binding tying the fetch lifetime to `'w`.
    pub(crate) _marker: PhantomData<&'w ()>,
}

impl Clone for AddedFetch<'_> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for AddedFetch<'_> {}

// SAFETY (QF1, QF2, QF3):
//   - QF1: `IS_ARCHETYPAL = false`; `filter_fetch` performs a real per-row
//     check (tick compare with null-base short-circuit).
//   - QF2: `init_access` declares a component read of `state.id`
//     (plan FLT2 — conservative; mirrors `With<C>`).
//   - QF3: `Fetch<'w>` holds a `*const UnsafeCell<Tick>` scoped to `'w`
//     via `PhantomData<&'w ()>`; the pointer is refreshed by every
//     `set_table_*` call before any `filter_fetch` (QD2-analogue).
unsafe impl<C: Component> QueryFilter for Added<C> {
    type State = AddedState<C>;
    type Fetch<'w> = AddedFetch<'w>;
    const IS_ARCHETYPAL: bool = false;
    // Phase 12.5 Track B NCD2: `Added<C>` reads per-row added_ticks; the
    // dispatcher MUST forward `meta` so `set_table_*` captures the
    // (last_run, this_run] window.
    const NEEDS_CHANGE_DETECTION: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        AddedState {
            id: C::component_id(),
            _marker: PhantomData,
        }
    }

    #[inline]
    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        // FLT2: conservative read declaration — Added<C> inspects the per-row
        // added_ticks column, which is logically part of C's lifecycle. The
        // intra-system aliasing detector treats it as a read of C.
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
        // FLT4: contributing the include bit makes `QueryDataState::update_archetypes`
        // select only archetypes that contain `C`. The null-base branch in
        // `filter_fetch` remains the safety net for the `Or<F>` post-filter
        // path (plan §3 Q7.3).
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        AddedFetch {
            tick_base: std::ptr::null(),
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
        // SAFETY (QF3, plan §5.3 Round 2 W7):
        //   - `archetype` is a live `*const Archetype` for `'w` (caller
        //     contract of this `unsafe fn`).
        //   - The shared reborrow is scoped to this block; `tick_column_base`
        //     reads `component_pools` (sparse map) under the `&Archetype`.
        //   - `meta` is read-only INPUT (Round 2 W7); ticks are `Copy`-extracted
        //     into the Fetch by value.
        let archetype_ref: &Archetype = unsafe { &*archetype };
        // STORE3: `added_ticks_ptr` returns the base of `Box<[UnsafeCell<Tick>]>`;
        // pointer is stable for the pool's lifetime (Phase 10 STORE2).
        fetch.tick_base = match archetype_ref.tick_column_base(state.id) {
            Some((added_base, _changed_base)) => added_base,
            // Round 2 C4 — `Or<F>` may walk archetypes that lack `C`;
            // `filter_fetch` checks for the NULL sentinel.
            None => std::ptr::null(),
        };
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
        // SAFETY (QF3, plan §5.3): tick reads are shared regardless of the
        // archetype pointer's mutability; reborrow the write-capable pointer
        // as `*const Archetype` and forward to the read-only path. No
        // write-capable provenance is consumed here.
        unsafe { Self::set_table_readonly(fetch, state, archetype as *const _, meta) }
    }

    #[inline(never)]
    #[cold]
    unsafe fn set_table_readonly_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // NCD5 backstop: NCD = true ⇒ dispatcher must route through the
        // meta-bearing variant. Reaching here means a contributor broke
        // the NCD6 const-fold dispatch contract.
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
    unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
        // Round 2 C4 — predicted-not-taken null-base check for archetypes
        // that lack `C` (only possible under `Or<F>` post-filter path).
        // Cold branch under normal queries.
        if fetch.tick_base.is_null() {
            return false;
        }
        // SAFETY (STORE3, QF3, plan §5.4-bis):
        //   - `fetch.tick_base` is the `added_ticks` base for the archetype
        //     cached by the prior `set_table_*` call (QF3 contract).
        //   - `row < archetype.entity_count()` per QF3; the buffer length
        //     equals `pool.max_components()` (plan STORE1) which bounds
        //     `entity_count()` from above (the pool is the row's storage).
        //   - Reading through `UnsafeCell::get()` is sound: Phase 9 SCH3
        //     guarantees no concurrent writer of this `(archetype, C)` slot,
        //     and `Tick` is `Copy` (plain `u32`).
        let tick: Tick = unsafe { *(*fetch.tick_base.add(row)).get() };
        tick.is_newer_than(fetch.last_run, fetch.this_run)
    }
}

// ── Changed<C> (Phase 10 Wave C Step 10) ────────────────────────────────────

/// Per-row change-detection filter: matches rows whose component `C` was
/// added OR mutated since the active system's `last_run` tick
/// (plan §2.3 FLT1-FLT8, §3 Q6 adopted Bevy deref-bump semantics).
///
/// `Changed<C>` mirrors [`Added<C>`] exactly except that it reads the
/// `changed_ticks` column rather than `added_ticks`. The two filters share
/// the access declaration shape (a single read of `C`), the archetype-level
/// predicate (`mask.contains(C::component_id())`), and the null-base
/// short-circuit for `Or<F>` composition (plan §5.4-bis Round 2 C4).
///
/// # Semantics
///
/// A row reports as `Changed<C>` whenever its `changed_ticks[row]` falls in
/// the system's `(last_run, this_run]` window. Writes that triggered the
/// tick bump come from:
/// * The initial insert (`Archetype::create_entity` writes
///   `changed = current_tick`).
/// * Any `Mut<T>::deref_mut` (Step 11 — Bevy deref-bump pattern).
/// * `Mut<T>::set_if_neq` on inequality (Step 11).
///
/// Reads through `Mut<T>::bypass_change_detection` deliberately skip the
/// tick bump and remain invisible to `Changed<C>` (plan §2.5 MUT5).
pub struct Changed<C: Component> {
    _marker: PhantomData<fn() -> C>,
}

/// Per-system cached state for [`Changed<C>`]: a resolved [`ComponentId`].
#[derive(Clone, Copy)]
pub struct ChangedState<C: Component> {
    pub(crate) id: ComponentId,
    pub(crate) _marker: PhantomData<fn() -> C>,
}

/// Per-archetype `Fetch` scratch for [`Changed<C>`].
///
/// Same shape as [`AddedFetch`] but `tick_base` points at the
/// `changed_ticks` column rather than `added_ticks`. The null-base
/// sentinel is interpreted identically (plan §5.4-bis Round 2 C4).
pub struct ChangedFetch<'w> {
    /// Base pointer to the active archetype's `changed_ticks` column.
    /// NULL for `Or<F>`-only archetypes that lack `C`.
    pub(crate) tick_base: *const UnsafeCell<Tick>,
    /// Active system's `last_run` snapshot at `set_table_*` time.
    pub(crate) last_run: Tick,
    /// Active system's `this_run` snapshot at `set_table_*` time.
    pub(crate) this_run: Tick,
    /// Type binding tying the fetch lifetime to `'w`.
    pub(crate) _marker: PhantomData<&'w ()>,
}

impl Clone for ChangedFetch<'_> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for ChangedFetch<'_> {}

// SAFETY (QF1, QF2, QF3): identical reasoning to `Added<C>` — see the
//   `unsafe impl QueryFilter for Added<C>` above. The only behavioural
//   delta is the source column (`changed_ticks` vs `added_ticks`); the
//   access surface and per-row contract are unchanged.
unsafe impl<C: Component> QueryFilter for Changed<C> {
    type State = ChangedState<C>;
    type Fetch<'w> = ChangedFetch<'w>;
    const IS_ARCHETYPAL: bool = false;
    // Phase 12.5 Track B NCD2: same rationale as `Added<C>`.
    const NEEDS_CHANGE_DETECTION: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        ChangedState {
            id: C::component_id(),
            _marker: PhantomData,
        }
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
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        ChangedFetch {
            tick_base: std::ptr::null(),
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
        // SAFETY (QF3, plan §5.3 Round 2 W7): identical to `Added::set_table_readonly`
        //   except the captured base is the `changed_ticks` column.
        let archetype_ref: &Archetype = unsafe { &*archetype };
        fetch.tick_base = match archetype_ref.tick_column_base(state.id) {
            Some((_added_base, changed_base)) => changed_base,
            None => std::ptr::null(),
        };
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
        // SAFETY: tick reads are shared regardless of pointer mutability —
        // delegate to the read-only path.
        unsafe { Self::set_table_readonly(fetch, state, archetype as *const _, meta) }
    }

    #[inline(never)]
    #[cold]
    unsafe fn set_table_readonly_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // NCD5 backstop — see `Added::set_table_readonly_no_meta` rationale.
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
    unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
        // Round 2 C4 — null-base predicted-not-taken check.
        if fetch.tick_base.is_null() {
            return false;
        }
        // SAFETY (STORE3, QF3, plan §5.4-bis): identical to `Added::filter_fetch`
        //   except the source column is `changed_ticks`. Phase 9 SCH3 ensures
        //   no concurrent writer; `Tick` is `Copy`.
        let tick: Tick = unsafe { *(*fetch.tick_base.add(row)).get() };
        tick.is_newer_than(fetch.last_run, fetch.this_run)
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
            // Phase 12.5 Track B NCD3: tuple-as-AND propagation — any
            // non-archetypal element forces the meta-bearing dispatch path.
            const NEEDS_CHANGE_DETECTION: bool =
                false $( || $F::NEEDS_CHANGE_DETECTION )*;

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
                meta: &'_ SystemMeta,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (QF3): forwarded per-element; `archetype`
                    //   carries read-only provenance and is identical for
                    //   every element. `meta` is shared INPUT — copied
                    //   into each element's Fetch by value if needed
                    //   (Phase 10 Round 2 W7).
                    unsafe { <$F as QueryFilter>::set_table_readonly($f, $s, archetype, meta); }
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
                    // SAFETY (QF3): write-capable `archetype` forwarded
                    //   per-element; `meta` forwarded by reference.
                    unsafe { <$F as QueryFilter>::set_table_mut($f, $s, archetype, meta); }
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
                    // SAFETY (QF3): per-element forwarding. NCD3 propagation
                    //   guarantees this method is only reached when no
                    //   element needs change detection — every per-element
                    //   `_no_meta` body is the meta-free re-impl.
                    unsafe { <$F as QueryFilter>::set_table_readonly_no_meta($f, $s, archetype); }
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
                    // SAFETY (QF3): write-capable forwarding; same NCD3
                    //   note as the readonly variant.
                    unsafe { <$F as QueryFilter>::set_table_mut_no_meta($f, $s, archetype); }
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
            // Phase 12.5 Track B NCD4: `Or<F>` propagates the AND/OR of
            // inner-element flags — any element with `NEEDS_CHANGE_DETECTION
            // = true` forces the meta-bearing dispatch path. Same reduction
            // as the tuple-as-AND variant; OR semantics do not relax the
            // per-element access surface (the dispatcher must still pass
            // `meta` to satisfy the meta-bearing variant of any inner element).
            const NEEDS_CHANGE_DETECTION: bool =
                false $( || $F::NEEDS_CHANGE_DETECTION )*;

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
                meta: &'_ SystemMeta,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (QF3): per-element forwarding; `archetype`
                    //   carries read-only provenance. `meta` forwarded
                    //   by reference per Round 2 W7.
                    unsafe { <$F as QueryFilter>::set_table_readonly($f, $s, archetype, meta); }
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
                    // SAFETY (QF3): write-capable `archetype` forwarded
                    //   per-element; `meta` forwarded by reference.
                    unsafe { <$F as QueryFilter>::set_table_mut($f, $s, archetype, meta); }
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
                    // SAFETY (QF3): per-element forwarding. NCD4 propagation
                    //   guarantees this method only fires when every inner
                    //   element is `NEEDS_CHANGE_DETECTION = false`.
                    unsafe { <$F as QueryFilter>::set_table_readonly_no_meta($f, $s, archetype); }
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
                    // SAFETY (QF3): write-capable forwarding; same NCD4
                    //   note as the readonly variant.
                    unsafe { <$F as QueryFilter>::set_table_mut_no_meta($f, $s, archetype); }
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
            // Vacuous — every method is a `panic!()` at monomorphisation.
            const NEEDS_CHANGE_DETECTION: bool = false;

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
            // Vacuous — every method is a `panic!()` at monomorphisation.
            const NEEDS_CHANGE_DETECTION: bool = false;

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
                _meta: &'_ SystemMeta,
            ) {
                panic!("Or<F> too large: see init_state diagnostic")
            }

            unsafe fn set_table_mut<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *mut Archetype,
                _meta: &'_ SystemMeta,
            ) {
                panic!("Or<F> too large: see init_state diagnostic")
            }

            unsafe fn set_table_readonly_no_meta<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *const Archetype,
            ) {
                panic!("Or<F> too large: see init_state diagnostic")
            }

            unsafe fn set_table_mut_no_meta<'w>(
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

    // ── Added<C> / Changed<C> tests (Phase 10 Wave C Steps 9-10) ────────

    use crate::ecs::core::change_detection::{MAX_CHANGE_AGE, Tick};
    use std::cell::UnsafeCell;

    /// `Added<C>` and `Changed<C>` MUST be non-archetypal so the iterator's
    /// const-fold path activates the per-row branch (plan FLT1).
    #[test]
    fn added_filter_is_not_archetypal() {
        const { assert!(!<Added<A> as QueryFilter>::IS_ARCHETYPAL) };
    }

    #[test]
    fn changed_filter_is_not_archetypal() {
        const { assert!(!<Changed<A> as QueryFilter>::IS_ARCHETYPAL) };
    }

    /// Archetype-level predicates: both filters must reject masks that lack `A`
    /// and accept masks containing it (plan FLT3).
    #[test]
    fn added_matches_component_set() {
        register_components();
        let state = AddedState::<A> {
            id: A::component_id(),
            _marker: PhantomData,
        };

        let mut mask = ComponentMask::new();
        assert!(!<Added<A> as QueryFilter>::matches_component_set(&state, &mask));

        mask.set(A::component_id());
        assert!(<Added<A> as QueryFilter>::matches_component_set(&state, &mask));
    }

    #[test]
    fn changed_matches_component_set() {
        register_components();
        let state = ChangedState::<A> {
            id: A::component_id(),
            _marker: PhantomData,
        };

        let mut mask = ComponentMask::new();
        assert!(!<Changed<A> as QueryFilter>::matches_component_set(&state, &mask));

        mask.set(A::component_id());
        assert!(<Changed<A> as QueryFilter>::matches_component_set(&state, &mask));
    }

    /// `aggregate_include` must set the bit for `C` (plan FLT4) so that the
    /// `QueryDataState::update_archetypes` path selects only archetypes that
    /// contain `C`.
    #[test]
    fn added_aggregate_sets_include_bit() {
        register_components();
        let state = AddedState::<A> {
            id: A::component_id(),
            _marker: PhantomData,
        };

        let mut include = ComponentMask::new();
        let mut exclude = ComponentMask::new();
        <Added<A> as QueryFilter>::aggregate_include(&state, &mut include);
        <Added<A> as QueryFilter>::aggregate_exclude(&state, &mut exclude);

        assert!(include.contains(A::component_id()));
        assert!(exclude.is_empty());
    }

    #[test]
    fn changed_aggregate_sets_include_bit() {
        register_components();
        let state = ChangedState::<A> {
            id: A::component_id(),
            _marker: PhantomData,
        };

        let mut include = ComponentMask::new();
        let mut exclude = ComponentMask::new();
        <Changed<A> as QueryFilter>::aggregate_include(&state, &mut include);
        <Changed<A> as QueryFilter>::aggregate_exclude(&state, &mut exclude);

        assert!(include.contains(A::component_id()));
        assert!(exclude.is_empty());
    }

    /// FLT2 conservative read declaration: a sibling write of `A` must
    /// conflict with `Added<A>`'s declared read.
    #[test]
    fn added_init_access_declares_read() {
        register_components();
        let mut ecs = EcsMaster::new();
        let state = <Added<A> as QueryFilter>::init_state(&mut ecs);
        let mut access_set = FilteredAccessSet::new();
        <Added<A> as QueryFilter>::init_access(&state, &mut access_set);

        let err = access_set
            .add_component_write(A::component_id(), "probe-writer")
            .expect_err("sibling write must conflict with Added<A>'s declared read");
        assert_eq!(err.id, A::component_id().0);
    }

    #[test]
    fn changed_init_access_declares_read() {
        register_components();
        let mut ecs = EcsMaster::new();
        let state = <Changed<A> as QueryFilter>::init_state(&mut ecs);
        let mut access_set = FilteredAccessSet::new();
        <Changed<A> as QueryFilter>::init_access(&state, &mut access_set);

        let err = access_set
            .add_component_write(A::component_id(), "probe-writer")
            .expect_err("sibling write must conflict with Changed<A>'s declared read");
        assert_eq!(err.id, A::component_id().0);
    }

    /// `filter_fetch` with a row tick strictly inside the
    /// `(last_run, this_run]` window must return `true` (plan FLT6).
    ///
    /// We synthesise a 1-element tick buffer behind a stack-pinned
    /// `UnsafeCell<Tick>` so the test exercises the per-row read path without
    /// engaging full archetype storage.
    #[test]
    fn added_filter_matches_row_with_recent_added_tick() {
        let cells: [UnsafeCell<Tick>; 1] = [UnsafeCell::new(Tick::new(10))];
        let fetch = AddedFetch::<'_> {
            tick_base: cells.as_ptr(),
            last_run: Tick::new(2),
            this_run: Tick::new(10),
            _marker: PhantomData,
        };
        // SAFETY: tick_base points at the 1-element `cells` array allocated
        // on the stack and live for this scope; row 0 is in range.
        let matched = unsafe { <Added<A> as QueryFilter>::filter_fetch(&fetch, 0) };
        assert!(matched, "tick=10 ∈ (2, 10] must match");
    }

    /// `filter_fetch` with a row tick at or before `last_run` must return
    /// `false` (exclusive lower bound, plan TICK3).
    #[test]
    fn added_filter_excludes_row_with_old_added_tick() {
        let cells: [UnsafeCell<Tick>; 1] = [UnsafeCell::new(Tick::new(1))];
        let fetch = AddedFetch::<'_> {
            tick_base: cells.as_ptr(),
            last_run: Tick::new(2),
            this_run: Tick::new(10),
            _marker: PhantomData,
        };
        // SAFETY: same as above; row 0 in range.
        let matched = unsafe { <Added<A> as QueryFilter>::filter_fetch(&fetch, 0) };
        assert!(!matched, "tick=1 < last_run=2 must not match");
    }

    /// Round 2 C4 null-base branch: when an archetype lacks `C` (legal under
    /// `Or<F>` composition), `set_table_*` leaves `tick_base = null()` and
    /// `filter_fetch` MUST short-circuit to `false` (plan §5.4-bis).
    #[test]
    fn added_filter_or_with_archetypal_null_base_branch() {
        let fetch = AddedFetch::<'_> {
            tick_base: std::ptr::null(),
            last_run: Tick::new(2),
            this_run: Tick::new(10),
            _marker: PhantomData,
        };
        // SAFETY: null-base path executes the early return; the tick read is
        // never reached so no provenance is consumed.
        let matched = unsafe { <Added<A> as QueryFilter>::filter_fetch(&fetch, 0) };
        assert!(!matched, "null tick_base (Or<F> non-C archetype) must return false");
    }

    #[test]
    fn changed_filter_matches_row_with_recent_changed_tick() {
        let cells: [UnsafeCell<Tick>; 1] = [UnsafeCell::new(Tick::new(7))];
        let fetch = ChangedFetch::<'_> {
            tick_base: cells.as_ptr(),
            last_run: Tick::new(2),
            this_run: Tick::new(10),
            _marker: PhantomData,
        };
        // SAFETY: cells live for the scope; row 0 in range.
        let matched = unsafe { <Changed<A> as QueryFilter>::filter_fetch(&fetch, 0) };
        assert!(matched, "tick=7 ∈ (2, 10] must match");
    }

    #[test]
    fn changed_filter_excludes_row_with_old_changed_tick() {
        // Stored tick equal to last_run — exclusive lower bound rejects.
        let cells: [UnsafeCell<Tick>; 1] = [UnsafeCell::new(Tick::new(2))];
        let fetch = ChangedFetch::<'_> {
            tick_base: cells.as_ptr(),
            last_run: Tick::new(2),
            this_run: Tick::new(10),
            _marker: PhantomData,
        };
        // SAFETY: cells live for the scope; row 0 in range.
        let matched = unsafe { <Changed<A> as QueryFilter>::filter_fetch(&fetch, 0) };
        assert!(!matched, "tick=2 == last_run must NOT match (exclusive lower bound)");
    }

    /// `Or<(With<A>, Changed<B>)>` must AND-fold `IS_ARCHETYPAL` to `false`
    /// because `Changed<B>` is non-archetypal (plan §3 Q7.1).
    #[test]
    fn or_with_changed_is_not_archetypal() {
        const { assert!(!<Or<(With<A>, Changed<B>)> as QueryFilter>::IS_ARCHETYPAL) };
    }

    /// First-run semantic guard: after `SystemMeta::new(name, current_tick)`,
    /// every pre-existing tick within `MAX_CHANGE_AGE` of `current_tick`
    /// reports as `Changed` on the first observation (plan §9.4 / TICK8).
    #[test]
    fn changed_filter_first_run_observes_pre_existing_tick() {
        let stored = Tick::new(100);
        let last_run = Tick::new(100u32.wrapping_sub(MAX_CHANGE_AGE));
        let this_run = Tick::new(100);
        let cells: [UnsafeCell<Tick>; 1] = [UnsafeCell::new(stored)];
        let fetch = ChangedFetch::<'_> {
            tick_base: cells.as_ptr(),
            last_run,
            this_run,
            _marker: PhantomData,
        };
        // SAFETY: stack-allocated cells live for the scope; row 0 in range.
        let matched = unsafe { <Changed<A> as QueryFilter>::filter_fetch(&fetch, 0) };
        assert!(
            matched,
            "first-run last_run = current - MAX_CHANGE_AGE must observe pre-existing ticks as changed"
        );
    }

    /// Phase 12.5 Track B NCD5 — backstop test.
    ///
    /// `Added<C>::set_table_readonly_no_meta` is the meta-free dispatch
    /// variant. For `NEEDS_CHANGE_DETECTION = true` impls (which `Added<C>`
    /// is) the body is a `#[cold]` `panic!()` — reaching it means the
    /// NCD6 const-fold dispatcher routed the wrong way. This test pins
    /// the panic at the trait level so any future regression that drops
    /// the backstop fails loudly.
    #[test]
    fn query_filter_no_meta_panic_for_added() {
        use std::panic::{self, AssertUnwindSafe};
        register_components();
        let mut ecs = EcsMaster::new();
        let state = <Added<A> as QueryFilter>::init_state(&mut ecs);
        let mut fetch = <Added<A> as QueryFilter>::init_fetch(&state);
        // The body never reads the `archetype` argument — it panics
        // unconditionally — so a null pointer is fine here.
        let archetype: *const Archetype = std::ptr::null();
        // SAFETY: the trait method is `unsafe`, but the body short-
        //   circuits to `panic!()` before reading any argument — the
        //   null pointer is never dereferenced.
        let result = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            <Added<A> as QueryFilter>::set_table_readonly_no_meta(
                &mut fetch,
                &state,
                archetype,
            );
        }));
        assert!(
            result.is_err(),
            "Added<C>::set_table_readonly_no_meta MUST panic (NCD5 backstop)"
        );
    }
}
