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
use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::params::diagnostics::intra_system_conflict_panic;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

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
///    minted by `super::super::super::ecs_master::unsafe_ecs_cell::UnsafeEcsCell`
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

    /// EnableTag C2 — `true` iff this filter contributes a **positive
    /// archetypal include bit** (`With<C>`, or an AND-tuple containing one).
    ///
    /// Feeds the `(D, F)`-construction-seam shape const-assert (Step 7a):
    /// an `Enabled<T>`/`Disabled<T>` term is only admitted when the query is
    /// bounded by a positive term — a data component (`D::HAS_DATA_COMPONENT`)
    /// or a `With<_>` (this flag). Additive default `false`; only `With<C>`
    /// (and tuples containing one — OR-folded by the tuple/`Or` macros) set it
    /// `true`. Zero ABI change to existing filters; zero runtime cost.
    const HAS_POSITIVE_ARCHETYPAL: bool = false;

    /// EnableTag C2/C3 — `true` iff this filter contains an `Enabled<T>` or
    /// `Disabled<T>` term (a leaf, an AND-tuple, or an `Or` containing one —
    /// OR-folded).
    ///
    /// Input to the `(D, F)` shape const-asserts (Step 7a). Additive default
    /// `false`; only the [`Enabled`](super::filter_enable::Enabled) /
    /// [`Disabled`](super::filter_enable::Disabled) leaves set it `true`.
    const CONTAINS_ENABLE_TERM: bool = false;

    /// EnableTag C3 — `true` iff this filter contains a change-detection term
    /// (`Added<C>` / `Changed<C>`, a leaf or a tuple/`Or` containing one —
    /// OR-folded).
    ///
    /// Input to the C3 shape const-assert (Step 7a): an enable term cannot be
    /// combined with change detection in one query (point lookups apply the
    /// enable bit but not change detection, which would silently mislead).
    /// Additive default `false`; only `Added<C>` / `Changed<C>` set it `true`.
    const CONTAINS_CHANGE_DETECTION: bool = false;

    /// EnableTag amendment A3.3 — `true` ONLY for a single `Enabled<T>` /
    /// `Disabled<T>` leaf.
    ///
    /// `false` for `()`, `With`, `Without`, `Added`, `Changed`, **every**
    /// tuple, and `Or` (the tuple/`Or` macros do NOT override the default).
    /// Distinguishes the candidate-seedable sole-single-enable shape
    /// (`Query<(), Enabled<A>>`) — admitted by the narrowed `_C2` assert — from
    /// an enable-tuple with no positive term (still rejected). Consumed by
    /// Step 7a's `IS_CANDIDATE_SEEDED` classification at the `(D, F)` seam.
    const IS_SOLE_SINGLE_ENABLE: bool = false;

    /// Dense plan D3 — `true` iff this filter contains a dense
    /// (non-fragmenting) term (`With<C>` / `Without<C>` where
    /// [`Component::STORAGE_IS_DENSE`]; a tuple OR-folds its members).
    ///
    /// Gates the cursor's [`Self::resolve_dense`] call (the global `DenseStore`
    /// pointer resolution). A dense `With`/`Without` ALSO sets
    /// `IS_ARCHETYPAL = false`, so the EXISTING per-row `filter_fetch` arm runs
    /// the `e2s` membership test — no separate cursor arm is needed (unlike the
    /// data side, which adds `dense_row_passes`). `false` by default, so a
    /// no-dense filter keeps it and pays nothing (the 0%-gate).
    ///
    /// [`Component::STORAGE_IS_DENSE`]: crate::ecs::core::component::component::Component::STORAGE_IS_DENSE
    const HAS_DENSE: bool = false;

    /// EnableTag amendment A2.1 — returns the resolved tag id of a SOLE enable
    /// term.
    ///
    /// Default = an `unreachable!()` backstop; overridden ONLY by the
    /// [`Enabled`](super::filter_enable::Enabled) /
    /// [`Disabled`](super::filter_enable::Disabled) leaves to return
    /// `state.id`. Step 7a calls it ONLY under `if const {
    /// IS_CANDIDATE_SEEDED }` (a sole single enable term), so for any non-sole
    /// filter the unreachable backstop is never emitted into a reachable path.
    #[inline]
    fn sole_enable_tag_id(_state: &Self::State) -> ComponentId {
        unreachable!(
            "sole_enable_tag_id called on a filter that is not a sole single \
             Enabled<T>/Disabled<T> term (IS_SOLE_SINGLE_ENABLE = false)"
        )
    }

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

    /// EnableTag positive-term cull verdict (Decision 2) — `true` iff archetype
    /// `arch` MAY contain a row this filter matches.
    ///
    /// Consulted ONLY under `const { F::CONTAINS_ENABLE_TERM }` by
    /// `QueryDataState`'s recull to drop archetypes that the typed enable term
    /// proves row-empty (an `Enabled<A>` archetype with no `A` column has every
    /// row disabled ⇒ no `Enabled<A>` row survives). Default: keep (no cull) —
    /// every non-enable leaf (`()`, `With`, `Without`, `Added`, `Changed`, `Or`)
    /// inherits this and is never reached anyway (their `CONTAINS_ENABLE_TERM`
    /// is `false`, so the gated recull never calls them).
    ///
    /// Conservative direction: returning `true` is always sound (the per-row
    /// `filter_fetch` still applies the exact gate); returning `false` MUST be
    /// proven row-empty for the term, or it would silently drop matching rows.
    #[inline]
    fn enable_cull_keeps_archetype(
        _state: &Self::State,
        _master: &ArchetypeMaster,
        _arch: ArchetypeId,
    ) -> bool {
        true
    }

    /// Dense plan D3 — resolves this filter's dense term(s) against the world.
    ///
    /// Called ONCE per cursor construction under `if const { Self::HAS_DENSE }`
    /// (the [`UnsafeEcsCell`] is available there), so it is emitted ONLY into a
    /// dense monomorphisation. The default body is empty — every non-dense
    /// filter (`()`, table `With`/`Without`, `Added`, `Changed`, …) inherits it
    /// and the cursor's gated call folds to nothing (the 0%-gate). A dense
    /// `With<C>` / `Without<C>` overrides it to cache the global `DenseStore`
    /// pointer into its `Fetch`; the per-row `filter_fetch` then runs the `e2s`
    /// membership test.
    ///
    /// # Safety
    ///
    /// * `world` MUST satisfy the read contract declared by the active
    ///   `SystemParam::init_access`.
    /// * The resolved `DenseStore` pointer is valid for `'w` (address-stable
    ///   `DenseRegistry` slot).
    ///
    /// [`UnsafeEcsCell`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell
    #[inline]
    unsafe fn resolve_dense<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
    ) {
        // Default: non-dense filter resolves nothing (the 0%-gate). Overridden
        // only by a dense `With<C>` / `Without<C>` leaf.
    }

    /// Dense plan D3 — `true` iff this filter contributes a dense INCLUDE term
    /// (`With<C>` where `C::STORAGE_IS_DENSE`; a tuple OR-folds). `Without<C>`
    /// does NOT (it admits non-members too — no candidate bound). Drives the
    /// `QueryDataState` dense-seed path. `false` by default (the 0%-gate).
    const HAS_DENSE_INCLUDE: bool = false;

    /// Dense plan D3 — ORs every dense INCLUDE term's `arch_presence` into
    /// `out` (the candidate-seed bitset). Called ONLY under
    /// `if const { Self::HAS_DENSE_INCLUDE }`. Default no-op; overridden by a
    /// dense `With<C>` to read its store's `arch_presence`.
    #[inline]
    fn dense_include_candidates(
        _state: &Self::State,
        _registry: &crate::ecs::core::component::dense::DenseRegistry,
        _out: &mut crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet,
    ) {
    }

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

// ── Dense filter fetch (Dense plan D3) ──────────────────────────────────────

/// Per-archetype `Fetch` scratch for a DENSE [`With<C>`] / [`Without<C>`] term.
///
/// A dense component is signature-excluded, so its presence cannot be inspected
/// at the archetype-mask level (the mask never carries its bit). Instead the
/// membership is the per-row `e2s` oracle: `with` keeps a row iff the store
/// contains its entity; `without` keeps a row iff it does NOT. This struct
/// caches the two pointers the per-row `filter_fetch` needs.
///
/// For a TABLE `C` the filter keeps `Fetch = ()` (this struct is never
/// instantiated), so a non-dense `With`/`Without` is byte-identical (the
/// 0%-gate).
#[derive(Clone, Copy)]
pub struct DenseFilterFetch {
    /// The global `DenseStore` for `C`, resolved ONCE by
    /// [`resolve_dense`](QueryFilter::resolve_dense). NULL when the store does
    /// not exist yet (no entity ever inserted) — then `with` rejects every row
    /// and `without` keeps every row (no member exists).
    pub(crate) dense: *const crate::ecs::core::component::dense::DenseStore,
    /// The current archetype's `entity_ids` column base, cached by
    /// `set_table_*` (`entity = entity_ids[row]`).
    pub(crate) entity_ids: *const crate::ecs::identifiers::primitives::EntityId,
}

impl DenseFilterFetch {
    /// NULL-init value (pre-`resolve_dense` / pre-`set_table_*`).
    pub(crate) const NULL: Self = Self {
        dense: std::ptr::null(),
        entity_ids: std::ptr::null(),
    };

    /// Reads the dense membership of `row`'s entity (`true` iff present).
    ///
    /// # Safety
    /// * `dense` was resolved by `resolve_dense` and `entity_ids` cached by
    ///   `set_table_*` for the current archetype.
    /// * `row < entity_count` of the cached archetype.
    #[inline]
    pub(crate) unsafe fn contains_row(&self, row: usize) -> bool {
        if self.dense.is_null() {
            return false;
        }
        // SAFETY: `entity_ids` is the current archetype's slice base and `row <
        //   entity_count` (caller contract); `dense` is the address-stable
        //   store for `'w`.
        let entity = unsafe { *self.entity_ids.add(row) };
        let store = unsafe { &*self.dense };
        store.contains(entity)
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
    type Fetch<'w> = DenseFilterFetch;
    // Table `With<C>` is archetypal (`true`); a DENSE `With<C>` is NOT — the
    // dense bit is signature-excluded, so the per-row `e2s` membership test
    // runs in `filter_fetch` (Dense plan D3, the `IS_ARCHETYPAL=false` arm).
    const IS_ARCHETYPAL: bool = !C::STORAGE_IS_DENSE;
    // Phase 12.5 Track B NCD2: `With<C>` reads only the archetype mask bit
    // (compile-time on the QueryDataState path); no per-row ticks.
    const NEEDS_CHANGE_DETECTION: bool = false;
    // EnableTag C2: a TABLE `With<C>` contributes a positive archetypal include
    // bit (bounds an enable term); a DENSE `With<C>` contributes none (the bit
    // is signature-excluded — see `aggregate_include`).
    const HAS_POSITIVE_ARCHETYPAL: bool = !C::STORAGE_IS_DENSE;
    // Dense plan D3.
    const HAS_DENSE: bool = C::STORAGE_IS_DENSE;
    // Dense plan D3: a dense `With<C>` is a dense INCLUDE term (seeds candidates).
    const HAS_DENSE_INCLUDE: bool = C::STORAGE_IS_DENSE;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        WithState { id: C::component_id(), _marker: PhantomData }
    }

    #[inline]
    fn dense_include_candidates(
        state: &Self::State,
        registry: &crate::ecs::core::component::dense::DenseRegistry,
        out: &mut crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet,
    ) {
        // Gated by `const { Self::HAS_DENSE_INCLUDE }`; OR-in the dense store's
        // `arch_presence` (the conservative candidate seed for a dense `With`).
        if const { C::STORAGE_IS_DENSE }
            && let Some(store) = registry.store(state.id)
        {
            store.arch_presence().for_each_set_bit(|a| out.insert(a));
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
        // Dense is signature-excluded: the mask never carries its bit, so a
        // dense `With` admits every archetype at this level (candidates are
        // bounded by the `arch_presence` seed; the exact per-row gate is
        // `filter_fetch`'s `e2s.contains`).
        if const { C::STORAGE_IS_DENSE } {
            return true;
        }
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        // Dense `With` contributes NO include bit (signature-excluded);
        // candidates are seeded from `arch_presence` (D3 seed wiring).
        if const { C::STORAGE_IS_DENSE } {
            return;
        }
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        DenseFilterFetch::NULL
    }

    #[inline]
    unsafe fn resolve_dense<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
    ) {
        // Gated by `const { Self::HAS_DENSE }` at the cursor (never emitted for
        // a table `C` — the 0%-gate). Caches the global `DenseStore` pointer.
        // SAFETY (resolve_dense contract): `world` upholds the access contract;
        //   the store lives in the address-stable `DenseRegistry` slot, valid
        //   for `'w`; the pointer is confined to the `'w`-scoped `Fetch`.
        let registry = unsafe { world.world().dense_registry() };
        fetch.dense = match registry.store(state.id) {
            Some(store) => store as *const _,
            None => std::ptr::null(),
        };
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        archetype: *const Archetype,
        _meta: &'_ SystemMeta,
    ) {
        if const { C::STORAGE_IS_DENSE } {
            // SAFETY (QD3): `archetype` is live for `'w`; reading the immutable
            //   `entity_ids` slice base needs only shared access.
            let arch_ref: &Archetype = unsafe { &*archetype };
            fetch.entity_ids = arch_ref.entity_ids_slice().as_ptr();
        }
        // Table `With`: no per-archetype state (archetypal filter).
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,
    ) {
        // SAFETY (QD3): the dense arm reads only the immutable `entity_ids`
        //   base; forward to the readonly path (no write provenance consumed).
        unsafe { Self::set_table_readonly(fetch, state, archetype as *const _, meta) }
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        archetype: *const Archetype,
    ) {
        if const { C::STORAGE_IS_DENSE } {
            // SAFETY (QD3): see `set_table_readonly`.
            let arch_ref: &Archetype = unsafe { &*archetype };
            fetch.entity_ids = arch_ref.entity_ids_slice().as_ptr();
        }
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // SAFETY (QD3): dense arm reads the immutable `entity_ids` base only.
        unsafe { Self::set_table_readonly_no_meta(fetch, state, archetype as *const _) }
    }

    #[inline]
    unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
        if const { C::STORAGE_IS_DENSE } {
            // Dense `With`: keep the row iff its entity is a member.
            // SAFETY (D3): `fetch` was set up by `resolve_dense` +
            //   `set_table_*`; `row < entity_count` (cursor guard).
            return unsafe { fetch.contains_row(row) };
        }
        // Table `With`: archetypal filter (QF1, IS_ARCHETYPAL=true) — the
        // cursor never calls this; returns true unconditionally.
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
    type Fetch<'w> = DenseFilterFetch;
    // Table `Without<C>` is archetypal; a DENSE `Without<C>` is NOT — the dense
    // bit is signature-excluded (never on the mask), so the per-row `!e2s`
    // membership test runs in `filter_fetch` (Dense plan D3).
    const IS_ARCHETYPAL: bool = !C::STORAGE_IS_DENSE;
    // Phase 12.5 Track B NCD2: `Without<C>` inspects bit absence; no ticks.
    const NEEDS_CHANGE_DETECTION: bool = false;
    // Dense plan D3.
    const HAS_DENSE: bool = C::STORAGE_IS_DENSE;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        WithoutState { id: C::component_id(), _marker: PhantomData }
    }

    #[inline]
    fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {}

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        // Dense is signature-excluded: the mask never carries its bit, so a
        // dense `Without` admits every archetype at this level (a row passes
        // iff it is NOT a member — the per-row gate is `filter_fetch`). It
        // contributes NO exclude bit either (see `aggregate_exclude`).
        if const { C::STORAGE_IS_DENSE } {
            return true;
        }
        !mask.contains(state.id)
    }

    #[inline]
    fn aggregate_exclude(state: &Self::State, exclude: &mut ComponentMask) {
        // Dense `Without` contributes NO exclude bit (the bit is never present
        // on any archetype, so excluding it would be a no-op anyway). The exact
        // per-row gate is `filter_fetch`'s `!e2s.contains`.
        if const { C::STORAGE_IS_DENSE } {
            return;
        }
        exclude.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        DenseFilterFetch::NULL
    }

    #[inline]
    unsafe fn resolve_dense<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
    ) {
        // SAFETY (resolve_dense contract): see `With::resolve_dense`.
        let registry = unsafe { world.world().dense_registry() };
        fetch.dense = match registry.store(state.id) {
            Some(store) => store as *const _,
            None => std::ptr::null(),
        };
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        archetype: *const Archetype,
        _meta: &'_ SystemMeta,
    ) {
        if const { C::STORAGE_IS_DENSE } {
            // SAFETY (QD3): `archetype` is live; reads the immutable
            //   `entity_ids` slice base only.
            let arch_ref: &Archetype = unsafe { &*archetype };
            fetch.entity_ids = arch_ref.entity_ids_slice().as_ptr();
        }
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,
    ) {
        // SAFETY (QD3): dense arm reads the immutable `entity_ids` base only.
        unsafe { Self::set_table_readonly(fetch, state, archetype as *const _, meta) }
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        archetype: *const Archetype,
    ) {
        if const { C::STORAGE_IS_DENSE } {
            // SAFETY (QD3): see `set_table_readonly`.
            let arch_ref: &Archetype = unsafe { &*archetype };
            fetch.entity_ids = arch_ref.entity_ids_slice().as_ptr();
        }
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // SAFETY (QD3): dense arm reads the immutable `entity_ids` base only.
        unsafe { Self::set_table_readonly_no_meta(fetch, state, archetype as *const _) }
    }

    #[inline]
    unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
        if const { C::STORAGE_IS_DENSE } {
            // Dense `Without`: keep the row iff its entity is NOT a member.
            // SAFETY (D3): `fetch` set up by `resolve_dense` + `set_table_*`;
            //   `row < entity_count` (cursor guard).
            return !unsafe { fetch.contains_row(row) };
        }
        // Table `Without`: archetypal filter — never called by the cursor.
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
///   archetypes lacking `B`, the `AddedFetch::tick_base` pointer is NULL
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

impl<C: Component> Added<C> {
    /// EnableTag D4 — the storage-shape const-assert for `Added<C>`.
    ///
    /// A bitset enable tag (`C::STORAGE_IS_BITSET == true`) has NO per-row tick
    /// storage, so `Added<C>` cannot be honored on it. Compile-rejecting the
    /// monomorphization is the correct fix rather than silently matching nothing
    /// (the Phase-22 D1 "compile-but-lie" lesson).
    ///
    /// Two triggers are required, exactly as the `(D, F)`-seam
    /// `QueryDataState::assert_query_shape` (Step 7a): an inline `const {}` block
    /// in [`Added::init_state`] fires at CODEGEN (`build` / `test`), while this
    /// `pub const fn` is referenced from a `const ITEM` in the `trybuild`
    /// `compile_fail` fixture to force evaluation under a metadata-only
    /// `cargo check` (the mode `trybuild` runs). Neither trigger alone covers
    /// every build path — a generic-fn `const {}` block is evaluated only at
    /// codegen, and `trybuild` checks.
    ///
    /// # Examples
    ///
    /// A normal table-storage component keeps the trait default
    /// `STORAGE_IS_BITSET == false`, so the assert is a no-op and
    /// `Added<C>` compiles:
    ///
    /// ```
    /// use boyko_ecs::ecs::core::component::component::Component;
    /// use boyko_ecs::ecs::core::iters::query::Added;
    /// use boyko_ecs::ecs::identifiers::primitives::ComponentId;
    ///
    /// struct Health(u32);
    /// impl Component for Health {
    ///     fn component_id() -> ComponentId { ComponentId(1) }
    /// }
    ///
    /// // `STORAGE_IS_BITSET == false` (the default) ⇒ the assert passes.
    /// const _: () = Added::<Health>::assert_storage_supports_change_detection();
    /// ```
    ///
    /// A bitset enable tag has no per-row tick storage, so the assert fails to
    /// compile (a bitset tag's `Added<C>` would silently match nothing):
    ///
    /// ```compile_fail
    /// use boyko_ecs::ecs::core::component::component::Component;
    /// use boyko_ecs::ecs::core::iters::query::Added;
    /// use boyko_ecs::ecs::identifiers::primitives::ComponentId;
    ///
    /// struct Stunned(u32);
    /// impl Component for Stunned {
    ///     const STORAGE_IS_BITSET: bool = true; // an enable tag
    ///     fn component_id() -> ComponentId { ComponentId(1) }
    /// }
    ///
    /// // `STORAGE_IS_BITSET == true` ⇒ compile error: no tick storage.
    /// const _: () = Added::<Stunned>::assert_storage_supports_change_detection();
    /// ```
    pub const fn assert_storage_supports_change_detection() {
        assert!(
            !C::STORAGE_IS_BITSET,
            "Added/Changed are not supported on bitset enable tags (no tick \
             storage); use Enabled<T>/with_enabled or query the underlying data"
        );
    }
}

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
    // EnableTag C3: `Added<C>` is a change-detection term; it cannot be mixed
    // with an enable term in one query (enforced at the (D, F) seam — Step 7a).
    const CONTAINS_CHANGE_DETECTION: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        // EnableTag D4 — codegen-time trigger (fires under `build` / `test`).
        // The check-time trigger for `trybuild` is the referenced `pub const fn`
        // `Self::assert_storage_supports_change_detection` in a `const ITEM`.
        const { Self::assert_storage_supports_change_detection() };
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
        // STORE3: `tick_column_base` returns the write-once added-tick
        // sub-region base of the pool's own `VmReservation`; the pointer is
        // stable for the pool's lifetime (Phase X.I vm-reservation stability).
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
        //   - `row < archetype.entity_count()` per QF3, and
        //     `entity_count() <= committed_rows` (Phase X.I: growth runs only
        //     inside `&mut` apply windows where no Fetch is live, and every
        //     `set_table_*` re-reads the bases per archetype), so the read
        //     stays inside the committed prefix of the tick sub-region.
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

impl<C: Component> Changed<C> {
    /// EnableTag D4 — the storage-shape const-assert for `Changed<C>`. Identical
    /// reasoning and two-trigger pattern to
    /// [`Added::assert_storage_supports_change_detection`]: a bitset enable tag
    /// has no per-row tick storage, so `Changed<C>` on it is compile-rejected.
    pub const fn assert_storage_supports_change_detection() {
        assert!(
            !C::STORAGE_IS_BITSET,
            "Added/Changed are not supported on bitset enable tags (no tick \
             storage); use Enabled<T>/with_enabled or query the underlying data"
        );
    }
}

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
    // EnableTag C3: `Changed<C>` is a change-detection term — see `Added<C>`.
    const CONTAINS_CHANGE_DETECTION: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        // EnableTag D4 — codegen-time trigger; the check-time `trybuild` trigger
        // is `Self::assert_storage_supports_change_detection` in a `const ITEM`.
        const { Self::assert_storage_supports_change_detection() };
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
            // EnableTag C2/C3: OR-fold the shape consts over the elements. An
            // AND-tuple has a positive archetypal term iff ANY element does;
            // it contains an enable / change-detection term iff ANY element
            // does. `IS_SOLE_SINGLE_ENABLE` is deliberately NOT overridden
            // (stays default `false`) — a tuple is never a single leaf, so an
            // enable-tuple with no positive term remains compile-rejected.
            const HAS_POSITIVE_ARCHETYPAL: bool =
                false $( || $F::HAS_POSITIVE_ARCHETYPAL )*;
            const CONTAINS_ENABLE_TERM: bool =
                false $( || $F::CONTAINS_ENABLE_TERM )*;
            const CONTAINS_CHANGE_DETECTION: bool =
                false $( || $F::CONTAINS_CHANGE_DETECTION )*;
            // Dense plan D3: OR-fold — a tuple contains a dense term iff ANY
            // element does. A dense `With`/`Without` element sets
            // `IS_ARCHETYPAL=false`, so the AND-fold above already flips the
            // tuple non-archetypal and routes each element's `filter_fetch`
            // (which runs the dense `e2s` test). `false` for an all-table tuple
            // (the 0%-gate — `resolve_dense` below const-folds to no-ops).
            const HAS_DENSE: bool = false $( || $F::HAS_DENSE )*;
            // Dense plan D3: a tuple has a dense INCLUDE term iff ANY element
            // does (OR-fold) — drives the candidate seed.
            const HAS_DENSE_INCLUDE: bool = false $( || $F::HAS_DENSE_INCLUDE )*;

            #[inline]
            fn init_state(world: &mut EcsMaster) -> Self::State {
                ( $( <$F as QueryFilter>::init_state(world), )* )
            }

            #[inline]
            fn dense_include_candidates(
                state: &Self::State,
                registry: &crate::ecs::core::component::dense::DenseRegistry,
                out: &mut crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet,
            ) {
                let ( $($s,)* ) = state;
                // Each element ORs its own dense-include candidates (gated by its
                // own `HAS_DENSE_INCLUDE`).
                $( <$F as QueryFilter>::dense_include_candidates($s, registry, out); )*
            }

            #[inline]
            fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
                let ( $($s,)* ) = state;
                $( <$F as QueryFilter>::init_access($s, access_set); )*
            }

            #[inline]
            unsafe fn resolve_dense<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (D3): each element gates its body on its own
                    //   `const { $F::HAS_DENSE }`; the `world` cell is `Copy`,
                    //   forwarded by value to preserve provenance.
                    unsafe { <$F as QueryFilter>::resolve_dense($f, $s, world); }
                )*
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

            // EnableTag Decision 2: AND-compose the cull verdict over the
            // elements — keep `arch` iff EVERY member keeps it. Conservative:
            // drop only when some member proves the archetype row-empty for its
            // term. Non-enable elements inherit the default `true`, so the fold
            // reduces to the enable member(s)' verdict.
            #[inline]
            fn enable_cull_keeps_archetype(
                state: &Self::State,
                master: &ArchetypeMaster,
                arch: ArchetypeId,
            ) -> bool {
                let ( $($s,)* ) = state;
                true $(
                    && <$F as QueryFilter>::enable_cull_keeps_archetype($s, master, arch)
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
        // M1 (EnableTag): every element must be `OrComposable` — the sealed
        // marker NOT implemented by `Enabled<T>`/`Disabled<T>`. This makes
        // `Or<(Enabled<A>, ..)>` a compile error: an `Or` folds a
        // non-archetypal per-row `filter_fetch` against an archetypal
        // element's unconditional `true`, which would leak disabled rows.
        #[allow(non_snake_case)]
        unsafe impl< $($F: QueryFilter + OrComposable),* > QueryFilter for Or<( $($F,)* )> {
            type State = ( $($F::State,)* );
            // BUG-ENABLE-PRE-1 (Bevy `OrFetch` pattern): each arm's fetch is
            // paired with a per-archetype `matches: bool` — does THIS arm match
            // the CURRENT archetype. An archetypal arm (`With`/`Without`) returns
            // `true` unconditionally from its `filter_fetch` (it assumes the
            // archetype already admitted it). But under `Or` the archetype may
            // have been admitted via a DIFFERENT arm, so the unconditional `true`
            // must be gated by this flag in the non-archetypal per-row fold.
            type Fetch<'w> = ( $( ($F::Fetch<'w>, bool), )* );
            const IS_ARCHETYPAL: bool = true $( && $F::IS_ARCHETYPAL )*;
            // Phase 12.5 Track B NCD4: `Or<F>` propagates the AND/OR of
            // inner-element flags — any element with `NEEDS_CHANGE_DETECTION
            // = true` forces the meta-bearing dispatch path. Same reduction
            // as the tuple-as-AND variant; OR semantics do not relax the
            // per-element access surface (the dispatcher must still pass
            // `meta` to satisfy the meta-bearing variant of any inner element).
            const NEEDS_CHANGE_DETECTION: bool =
                false $( || $F::NEEDS_CHANGE_DETECTION )*;
            // EnableTag C3: OR-fold the enable / change-detection shape consts.
            // `Enabled`/`Disabled` are NOT `OrComposable` (M1), so an enable
            // term can never actually reach here, but the fold keeps the const
            // honest if a future composable enable variant lands.
            // `HAS_POSITIVE_ARCHETYPAL` stays default `false`: `Or` is a
            // disjunction whose `aggregate_include` is a no-op — it does NOT
            // contribute a positive include bit that bounds the matched set,
            // so it must not be reported as a positive archetypal term.
            // `IS_SOLE_SINGLE_ENABLE` stays default `false` (never a leaf).
            const CONTAINS_ENABLE_TERM: bool =
                false $( || $F::CONTAINS_ENABLE_TERM )*;
            const CONTAINS_CHANGE_DETECTION: bool =
                false $( || $F::CONTAINS_CHANGE_DETECTION )*;

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
                // BUG-ENABLE-PRE-1: each arm's matches flag starts `false`;
                // `set_table_*` sets it per archetype before any `filter_fetch`
                // runs (QF3-analogue — the Fetch is refreshed per archetype).
                ( $( (<$F as QueryFilter>::init_fetch($s), false), )* )
            }

            // BUG-ENABLE-PRE-1: every `set_table_*` variant first forwards to
            // the arm's own `set_table_*` on `$f.0` (sound even when the arm
            // does NOT match this archetype: `With`/`Without` are no-ops and
            // `Added`/`Changed` set a NULL tick base), then records whether the
            // arm matches THIS archetype in `$f.1`. The per-row `filter_fetch`
            // then gates each archetypal arm's unconditional `true` by `$f.1`.
            // The match computation is skipped for a fully-archetypal `Or`
            // (`filter_fetch` const-folds to `true`, so the flags are never
            // read) — keeping that path at zero added cost.
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
                    unsafe { <$F as QueryFilter>::set_table_readonly(&mut $f.0, $s, archetype, meta); }
                    if !const { Self::IS_ARCHETYPAL } {
                        // SAFETY: `archetype` is a live `*const Archetype` for
                        //   the duration of this call (caller contract of this
                        //   `unsafe fn`); the shared reborrow is scoped to the
                        //   `component_mask()` read.
                        $f.1 = <$F as QueryFilter>::matches_component_set(
                            $s, unsafe { (*archetype).component_mask() },
                        );
                    }
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
                    unsafe { <$F as QueryFilter>::set_table_mut(&mut $f.0, $s, archetype, meta); }
                    if !const { Self::IS_ARCHETYPAL } {
                        // SAFETY: `archetype` is a live `*mut Archetype` for the
                        //   duration of this call; reborrowed shared for a
                        //   `component_mask()` read only (no write).
                        $f.1 = <$F as QueryFilter>::matches_component_set(
                            $s, unsafe { (*archetype).component_mask() },
                        );
                    }
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
                    unsafe { <$F as QueryFilter>::set_table_readonly_no_meta(&mut $f.0, $s, archetype); }
                    if !const { Self::IS_ARCHETYPAL } {
                        // SAFETY: same as `set_table_readonly` — read-only
                        //   reborrow of a live `*const Archetype`.
                        $f.1 = <$F as QueryFilter>::matches_component_set(
                            $s, unsafe { (*archetype).component_mask() },
                        );
                    }
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
                    unsafe { <$F as QueryFilter>::set_table_mut_no_meta(&mut $f.0, $s, archetype); }
                    if !const { Self::IS_ARCHETYPAL } {
                        // SAFETY: same as `set_table_mut` — shared reborrow of a
                        //   live `*mut Archetype` for a `component_mask()` read.
                        $f.1 = <$F as QueryFilter>::matches_component_set(
                            $s, unsafe { (*archetype).component_mask() },
                        );
                    }
                )*
            }

            #[inline]
            unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
                if const { Self::IS_ARCHETYPAL } {
                    return true;
                }
                let ( $($f,)* ) = fetch;
                // BUG-ENABLE-PRE-1: gate each arm's per-row result by its
                // per-archetype `matches` flag (`$f.1`). An archetypal arm whose
                // `matches_component_set` was false for THIS archetype must NOT
                // contribute its unconditional `true` to the OR fold.
                false $(
                    // SAFETY (QF1): per-element contract; `row` in range. The
                    //   arm's `filter_fetch` is only evaluated when `$f.1` (the
                    //   arm matches this archetype) holds.
                    || ( $f.1 && unsafe { <$F as QueryFilter>::filter_fetch(&$f.0, row) } )
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
        // M1 (EnableTag): carries the `OrComposable` element bound for parity
        // with the in-range `impl_or_filter_tuple!` impls — an oversized
        // `Or<(Enabled<A>, ..)>` is rejected at the bound, not the arity.
        #[allow(non_snake_case, unused_variables)]
        unsafe impl< $($F: QueryFilter + OrComposable),* > QueryFilter for Or<( $($F,)* )> {
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

// ── OrComposable seal (EnableTag M1) ────────────────────────────────────────

/// Sealed marker for [`QueryFilter`] types that may appear **inside** an
/// [`Or<F>`] combinator.
///
/// `Or<(F0, F1, ..)>` requires every element to be `OrComposable`. The seal
/// exists to compile-reject [`Enabled<T>`](super::filter_enable::Enabled) /
/// [`Disabled<T>`](super::filter_enable::Disabled) inside `Or` (EnableTag M1):
/// `Or` folds a non-archetypal element's per-row `filter_fetch` against an
/// archetypal element's unconditional `true`, which would leak disabled rows on
/// the archetypal branch. Until a safe `Or`-aware enable Fetch lands (the D7
/// seam), an enable term in `Or` is forbidden at the type level.
///
/// # Membership
///
/// * `()`, `With<C>`, `Without<C>`, `Added<C>`, `Changed<C>`.
/// * Nested `Or<F>` (iff every element of `F` is `OrComposable`).
/// * Tuples `(F0, .., Fn)` for `n <= 12` (iff every element is `OrComposable`).
///
/// NOT members: `Enabled<T>`, `Disabled<T>`.
///
/// # Safety
///
/// A purely declarative marker — it adds no method contract. `unsafe` only to
/// signal that membership is a deliberate, audited choice (an element placed in
/// `Or` must have an `Or`-correct `filter_fetch` / archetypal predicate).
pub(crate) unsafe trait OrComposable: QueryFilter {}

// SAFETY: `()` is the no-op filter — vacuously `Or`-correct.
unsafe impl OrComposable for () {}

// SAFETY: `With<C>` is archetypal; its `matches_component_set` is the
//   `Or`-folded predicate and `filter_fetch` is a vacuous `true`.
unsafe impl<C: Component> OrComposable for With<C> {}

// SAFETY: `Without<C>` is archetypal; same reasoning as `With<C>`.
unsafe impl<C: Component> OrComposable for Without<C> {}

// SAFETY: `Added<C>` carries the Round 2 C4 NULL-base short-circuit in
//   `filter_fetch`, so it returns `false` on `C`-absent archetypes reached via
//   the `Or` post-filter path — the existing, audited `Or`-correct behaviour.
unsafe impl<C: Component> OrComposable for Added<C> {}

// SAFETY: `Changed<C>` mirrors `Added<C>` (NULL-base short-circuit).
unsafe impl<C: Component> OrComposable for Changed<C> {}

// SAFETY: a nested `Or<F>` is `Or`-correct iff every element of `F` is — the
//   tuple `OrComposable` impl below forces that element-wise. The
//   `where Or<F>: QueryFilter` clause carries the supertrait obligation (the
//   `QueryFilter for Or<(..)>` impl is only emitted for tuple `F` whose
//   elements are `OrComposable`).
unsafe impl<F: OrComposable> OrComposable for Or<F> where Or<F>: QueryFilter {}

/// Emits an `OrComposable` impl for a tuple `(F0, F1, ..)` — composable iff
/// every element is. The element bound also satisfies the supertrait
/// `QueryFilter for (F0, ..)` obligation.
macro_rules! impl_or_composable_tuple {
    ( $( $F:ident ),* ) => {
        // SAFETY: every element is `OrComposable`; the tuple-as-AND
        //   `QueryFilter` impl preserves the per-element `Or`-correctness.
        unsafe impl< $($F: OrComposable),* > OrComposable for ( $($F,)* ) {}
    };
}

impl_or_composable_tuple!(F0);
impl_or_composable_tuple!(F0, F1);
impl_or_composable_tuple!(F0, F1, F2);
impl_or_composable_tuple!(F0, F1, F2, F3);
impl_or_composable_tuple!(F0, F1, F2, F3, F4);
impl_or_composable_tuple!(F0, F1, F2, F3, F4, F5);
impl_or_composable_tuple!(F0, F1, F2, F3, F4, F5, F6);
impl_or_composable_tuple!(F0, F1, F2, F3, F4, F5, F6, F7);
impl_or_composable_tuple!(F0, F1, F2, F3, F4, F5, F6, F7, F8);
impl_or_composable_tuple!(F0, F1, F2, F3, F4, F5, F6, F7, F8, F9);
impl_or_composable_tuple!(F0, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10);
impl_or_composable_tuple!(F0, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11);

// ── Phase X.A Step 1B — ArchetypalQueryFilter marker trait ──────────────────

/// Marker for [`QueryFilter`] impls whose decision is **archetype-level only**
/// — they cannot reject individual rows. Required as a bound on
/// `Query::for_each_chunk` / `Query::par_for_each_chunk` because the
/// chunk API yields one contiguous slice per archetype with no per-row gate.
///
/// # Membership
///
/// Stable members (Phase X.A):
/// * `()`, `With<C>`, `Without<C>`.
/// * `Or<F>` iff every element of `F` is `ArchetypalQueryFilter`.
/// * Tuples `(F0, F1, ..., Fn)` for `n <= 12` iff every element is
///   `ArchetypalQueryFilter`.
///
/// NOT members:
/// * `Added<C>`, `Changed<C>` — per-row tick comparison.
///
/// # Safety
///
/// Implementations MUST have `IS_ARCHETYPAL = true` AND
/// `NEEDS_CHANGE_DETECTION = false` at the filter level. The chunk API
/// relies on both to elide per-row work statically.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as a filter in `Query::for_each_chunk` — it is not archetype-level",
    label = "non-archetypal filter (likely `Added<T>` / `Changed<T>` / tuple containing one)",
    note = "the chunk API yields one slice per archetype with no per-row gate; use `Query::iter()` for per-row tick filtering"
)]
pub unsafe trait ArchetypalQueryFilter: QueryFilter {}

// === Leaf impls ===

// SAFETY: `()` has `IS_ARCHETYPAL = true` and `NEEDS_CHANGE_DETECTION = false`
//   per the `QueryFilter for ()` impl above (lines 219-284). No per-row state.
unsafe impl ArchetypalQueryFilter for () {}

// SAFETY: `With<C>` has `IS_ARCHETYPAL = true` and `NEEDS_CHANGE_DETECTION =
//   false` per the `QueryFilter for With<C>` impl above (lines 316-392). It
//   rejects only archetypes lacking `C`; `filter_fetch` is a vacuous `true`.
unsafe impl<C: Component> ArchetypalQueryFilter for With<C> {}

// SAFETY: `Without<C>` has `IS_ARCHETYPAL = true` and `NEEDS_CHANGE_DETECTION
//   = false` per the `QueryFilter for Without<C>` impl above (lines 418-489).
//   It rejects only archetypes containing `C`; `filter_fetch` is a vacuous
//   `true`.
unsafe impl<C: Component> ArchetypalQueryFilter for Without<C> {}

// === Or<F> propagation ===

// SAFETY: the concrete `QueryFilter for Or<F>` impl in this file is
//   monomorphised as `Or<(F0, F1, …)>` — the inner `F` is always a tuple. The
//   tuple impl below ensures `(F0, F1, …)` implements `ArchetypalQueryFilter`
//   iff every element does. Therefore the bound `F: ArchetypalQueryFilter` is
//   sufficient: it forces the inner tuple to be archetypal element-wise,
//   which propagates the `IS_ARCHETYPAL = true ∧ NEEDS_CHANGE_DETECTION =
//   false` invariants transitively to the `Or<F>` wrapper.
//
// The `where Or<F>: QueryFilter` clause carries the supertrait obligation:
//   `QueryFilter` is only implemented for `Or<(F0, F1, …)>` (the tuple
//   pattern), not for arbitrary `F`. The bound makes this dependency
//   explicit at the type-checker level — when the user writes
//   `Query<_, Or<(With<A>, Without<B>)>>::for_each_chunk(…)`, the tuple
//   `ArchetypalQueryFilter` impl below populates the inner tuple bound
//   and the existing `Or<(F0, F1)>: QueryFilter` impl populates the
//   supertrait obligation.
unsafe impl<F: ArchetypalQueryFilter> ArchetypalQueryFilter for Or<F>
where
    Or<F>: QueryFilter,
{
}

// === Tuple propagation ===

/// Emits an `ArchetypalQueryFilter` impl for a tuple `(F0, F1, …)`. The
/// tuple is archetypal iff every element is — propagation is by per-element
/// trait bound, identical in spirit to the `IS_ARCHETYPAL` AND-fold in the
/// `QueryFilter` tuple impl above.
macro_rules! impl_archetypal_filter_tuple {
    ( $( $F:ident ),* ) => {
        // SAFETY: every element is `ArchetypalQueryFilter`, hence each has
        //   `IS_ARCHETYPAL = true` and `NEEDS_CHANGE_DETECTION = false`. The
        //   tuple-AND propagation in the `QueryFilter` impl above (lines
        //   944-951) preserves both invariants for the tuple wrapper.
        unsafe impl< $($F: ArchetypalQueryFilter),* > ArchetypalQueryFilter for ( $($F,)* ) {}
    };
}

impl_archetypal_filter_tuple!(F0);
impl_archetypal_filter_tuple!(F0, F1);
impl_archetypal_filter_tuple!(F0, F1, F2);
impl_archetypal_filter_tuple!(F0, F1, F2, F3);
impl_archetypal_filter_tuple!(F0, F1, F2, F3, F4);
impl_archetypal_filter_tuple!(F0, F1, F2, F3, F4, F5);
impl_archetypal_filter_tuple!(F0, F1, F2, F3, F4, F5, F6);
impl_archetypal_filter_tuple!(F0, F1, F2, F3, F4, F5, F6, F7);
impl_archetypal_filter_tuple!(F0, F1, F2, F3, F4, F5, F6, F7, F8);
impl_archetypal_filter_tuple!(F0, F1, F2, F3, F4, F5, F6, F7, F8, F9);
impl_archetypal_filter_tuple!(F0, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10);
impl_archetypal_filter_tuple!(F0, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11);

#[cfg(test)]
mod archetypal_marker_tests {
    use super::*;
    use crate::ecs::core::component::component::Component;

    fn assert_archetypal<F: ArchetypalQueryFilter>() {}

    // Local test components — Component impls are compile-only shims
    // (`component_id` panics if ever invoked, but `assert_archetypal` is a
    // pure type-system check that never executes the body).
    struct CA;
    struct CB;
    impl Component for CA {
        fn component_id() -> crate::ecs::identifiers::primitives::ComponentId {
            unimplemented!("compile-only test component")
        }
    }
    impl Component for CB {
        fn component_id() -> crate::ecs::identifiers::primitives::ComponentId {
            unimplemented!("compile-only test component")
        }
    }

    #[test]
    fn unit_is_archetypal() {
        assert_archetypal::<()>();
    }

    #[test]
    fn with_is_archetypal() {
        assert_archetypal::<With<CA>>();
    }

    #[test]
    fn without_is_archetypal() {
        assert_archetypal::<Without<CA>>();
    }

    #[test]
    fn tuple_with_and_without_is_archetypal() {
        assert_archetypal::<(With<CA>, Without<CB>)>();
    }

    #[test]
    fn or_of_archetypal_is_archetypal() {
        assert_archetypal::<Or<(With<CA>, Without<CB>)>>();
    }

    #[test]
    fn tuple_arity_12_is_archetypal() {
        assert_archetypal::<(
            With<CA>, With<CA>, With<CA>, With<CA>,
            With<CA>, With<CA>, With<CA>, With<CA>,
            With<CA>, With<CA>, With<CA>, With<CA>,
        )>();
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
    // Intentional const check: asserting a `QueryFilter` associated const is
    // the test's purpose, so clippy's "constant in assert" lint does not apply.
    #[allow(clippy::assertions_on_constants)]
    fn unit_filter_is_archetypal() {
        assert!(<() as QueryFilter>::IS_ARCHETYPAL);
    }

    #[test]
    #[allow(clippy::assertions_on_constants)] // intentional associated-const check
    fn with_filter_is_archetypal() {
        assert!(<With<A> as QueryFilter>::IS_ARCHETYPAL);
    }

    #[test]
    #[allow(clippy::assertions_on_constants)] // intentional associated-const check
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
    #[allow(clippy::assertions_on_constants)] // intentional associated-const check
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
    #[allow(clippy::assertions_on_constants)] // intentional associated-const check
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
