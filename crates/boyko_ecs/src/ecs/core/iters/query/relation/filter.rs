//! Relation [`QueryFilter`]s — `HasRelation<R>`, `NoRelation<R>`,
//! `RelatedTo<R>`.
//!
//! * [`HasRelation<R>`] / [`NoRelation<R>`] — ARCHETYPAL: the source either
//!   carries (or lacks) the `R` foreign-key component. They reuse the
//!   [`With`](crate::ecs::core::iters::query::filter::With) /
//!   [`Without`](crate::ecs::core::iters::query::filter::Without) archetype-mask
//!   bit-test on `R`'s [`ComponentId`] — zero per-row cost.
//! * [`RelatedTo<R>`] — PER-ROW: keeps source rows whose `R` FK target equals a
//!   fixed entity. It reads the source row's OWN `R` column (no other-entity
//!   access), so it is `par_iter`-safe.
//!
//! `R` is a [`Relationship`] (always a table-storage
//! [`Component`](crate::ecs::core::component::component::Component) FK), so the
//! dense-storage arms the core filters carry are statically irrelevant here —
//! these filters take the table-archetypal / table-per-row paths only.

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::iters::query::filter::QueryFilter;
use crate::ecs::core::relationship::Relationship;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::params::diagnostics::intra_system_conflict_panic;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::identifiers::primitives::ComponentId;

// ── HasRelation<R> (archetypal: source carries the R FK) ────────────────────

/// Archetypal filter: matches sources that HAVE the relationship `R` (i.e. the
/// archetype carries `R`'s foreign-key component). The roots-complement of
/// [`NoRelation<R>`].
///
/// Reuses the [`With`](crate::ecs::core::iters::query::filter::With) machinery
/// on `R`'s [`ComponentId`] — a single archetype-mask bit-test, zero per-row
/// cost.
pub struct HasRelation<R: Relationship> {
    _marker: PhantomData<fn() -> R>,
}

/// Per-system cached state for [`HasRelation<R>`]: `R`'s resolved
/// [`ComponentId`].
#[derive(Clone, Copy)]
pub struct HasRelationState<R: Relationship> {
    id: ComponentId,
    _marker: PhantomData<fn() -> R>,
}

// SAFETY (QF1, QF2, QF3):
//   - QF1: `IS_ARCHETYPAL = true`; `filter_fetch` returns `true`.
//   - QF2: `init_access` declares a read of `R`'s id (mirrors `With<R>`).
//   - QF3: `Fetch<'w> = ()` — no per-archetype pointers cached.
unsafe impl<R: Relationship> QueryFilter for HasRelation<R> {
    type State = HasRelationState<R>;
    type Fetch<'w> = ();
    const IS_ARCHETYPAL: bool = true;
    const NEEDS_CHANGE_DETECTION: bool = false;
    // Contributes a positive archetypal include bit (the source MUST host `R`).
    const HAS_POSITIVE_ARCHETYPAL: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        HasRelationState {
            id: R::component_id(),
            _marker: PhantomData,
        }
    }

    #[inline]
    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        // Mirrors `With<R>`: a conservative read declaration of `R` keeps the
        // intra-system aliasing detector consistent with a sibling `&mut R`.
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
        // SAFETY: archetypal filter — no per-archetype state cached.
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY: archetypal filter — no per-archetype state cached.
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // SAFETY: meta-free body for NCD = false — no-op.
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {
        // SAFETY: meta-free body for NCD = false — no-op.
    }

    #[inline]
    unsafe fn filter_fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> bool {
        // SAFETY: archetypal filter (QF1) — returns true unconditionally; the
        //   cursor const-folds this call away (IS_ARCHETYPAL = true).
        true
    }
}

// ── NoRelation<R> (archetypal: source lacks the R FK — roots) ───────────────

/// Archetypal filter: matches sources that LACK the relationship `R` (the
/// archetype does NOT carry `R`'s foreign-key component) — i.e. the relation
/// roots.
///
/// Reuses the [`Without`](crate::ecs::core::iters::query::filter::Without)
/// machinery on `R`'s [`ComponentId`] — a single archetype-mask absence
/// bit-test, zero per-row cost.
pub struct NoRelation<R: Relationship> {
    _marker: PhantomData<fn() -> R>,
}

/// Per-system cached state for [`NoRelation<R>`]: `R`'s resolved
/// [`ComponentId`].
#[derive(Clone, Copy)]
pub struct NoRelationState<R: Relationship> {
    id: ComponentId,
    _marker: PhantomData<fn() -> R>,
}

// SAFETY (QF1, QF2, QF3):
//   - QF1: `IS_ARCHETYPAL = true`; `filter_fetch` returns `true`.
//   - QF2: `init_access` declares nothing — absence inspection reads no `R`
//     data (mirrors `Without<R>`).
//   - QF3: `Fetch<'w> = ()` — no per-archetype pointers cached.
unsafe impl<R: Relationship> QueryFilter for NoRelation<R> {
    type State = NoRelationState<R>;
    type Fetch<'w> = ();
    const IS_ARCHETYPAL: bool = true;
    const NEEDS_CHANGE_DETECTION: bool = false;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        NoRelationState {
            id: R::component_id(),
            _marker: PhantomData,
        }
    }

    #[inline]
    fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {
        // Mirrors `Without<R>`: inspects only bit absence — declares no access.
    }

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
        // SAFETY: archetypal filter — no per-archetype state cached.
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY: archetypal filter — no per-archetype state cached.
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // SAFETY: meta-free body for NCD = false — no-op.
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {
        // SAFETY: meta-free body for NCD = false — no-op.
    }

    #[inline]
    unsafe fn filter_fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> bool {
        // SAFETY: archetypal filter (QF1) — returns true unconditionally.
        true
    }
}

// ── RelatedTo<R> (per-row: source FK target == self.target) ─────────────────

/// Per-row filter: matches source rows whose `R` foreign-key target equals a
/// fixed `target` entity.
///
/// Built via [`RelatedTo::new`]. NON-archetypal (`IS_ARCHETYPAL = false`): the
/// per-row predicate reads the source row's OWN `R` column and compares
/// `r.target() == self.target`. Because it touches only the source's own row
/// (no other-entity access), it is `par_iter`-safe.
///
/// Use it through the filtered-query seam:
/// `world.query_filtered::<&Transform, _>(RelatedTo::<ChildOf>::new(parent))`.
pub struct RelatedTo<R: Relationship> {
    /// The fixed FK target a source row must point at to match.
    target: Entity,
    _marker: PhantomData<fn() -> R>,
}

impl<R: Relationship> RelatedTo<R> {
    /// The uninitialised-`target` POISON sentinel (Relations W1).
    ///
    /// [`QueryFilter::init_state`] is value-less (the typed-query cache keys on
    /// `(D, F)` TYPES only), so the runtime `target` is unavailable there. The
    /// state is primed with this sentinel and the value-carrying
    /// [`query_filtered`](crate::ecs::core::ecs_master::ecs_master::EcsMaster::query_filtered)
    /// entry OVERWRITES it via [`QueryFilter::seed_state`]. `EntityId(usize::MAX)`
    /// is unmistakable — the entity-id space is bounded far below `usize::MAX`,
    /// so no real source can carry it. A `filter_fetch` that observes the poison
    /// (the value-less `query::<_, RelatedTo<R>>()` path that skips
    /// `query_filtered`) is a LOUD `#[cold]` panic, never a silent id-0 match.
    const POISON: Entity =
        Entity::with_id(crate::ecs::identifiers::primitives::EntityId(usize::MAX));

    /// Builds a per-row filter matching sources whose `R` FK points at
    /// `target`.
    #[inline]
    pub fn new(target: Entity) -> Self {
        Self {
            target,
            _marker: PhantomData,
        }
    }
}

/// Per-system cached state for [`RelatedTo<R>`]: `R`'s resolved [`ComponentId`]
/// plus the fixed `target` to match against.
#[derive(Clone, Copy)]
pub struct RelatedToState<R: Relationship> {
    id: ComponentId,
    target: Entity,
    _marker: PhantomData<fn() -> R>,
}

/// Per-archetype `Fetch` scratch for [`RelatedTo<R>`]: the source archetype's
/// `R` column base + stride, plus the fixed target to compare each row against.
///
/// `base` is NULL before the first `set_table_*` (the column is guaranteed
/// present because `matches_component_set` requires `R`).
///
/// `Clone` / `Copy` are implemented manually because the auto-derive would
/// synthesise an unwanted `R: Copy` blanket bound (driven by the `*const R`
/// field), which the trait does not require.
pub struct RelatedToFetch<R: Relationship> {
    /// Base pointer to the current archetype's `R` column.
    base: *const R,
    /// `R` column stride (bytes) — `r_ptr = base + row*stride`.
    stride: usize,
    /// The fixed FK target to compare each row's `r.target()` against.
    target: Entity,
}

impl<R: Relationship> Clone for RelatedToFetch<R> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: Relationship> Copy for RelatedToFetch<R> {}

// SAFETY (QF1, QF2, QF3):
//   - QF1: `IS_ARCHETYPAL = false`; `filter_fetch` runs a real per-row check.
//   - QF2: `init_access` declares a read of `R` (the per-row column it reads).
//   - QF3: `Fetch` caches the `R` column base scoped to `'w`; refreshed by
//     every `set_table_*` before any `filter_fetch`.
unsafe impl<R: Relationship> QueryFilter for RelatedTo<R> {
    type State = RelatedToState<R>;
    type Fetch<'w> = RelatedToFetch<R>;
    const IS_ARCHETYPAL: bool = false;
    const NEEDS_CHANGE_DETECTION: bool = false;
    // The source MUST host `R` to be related, so this contributes a positive
    // archetypal include bit (bounds the matched set to `R`-hosting archetypes).
    const HAS_POSITIVE_ARCHETYPAL: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        // `QueryFilter::init_state` is value-less (the typed-query cache keys on
        // `(D, F)` TYPES only), so the runtime `target` is NOT available here.
        // It is primed with `Self::POISON` and OVERWRITTEN by the value-carrying
        // `query_filtered` entry via `seed_state`. The poison (`EntityId::MAX`)
        // is unmistakable: a `filter_fetch` that still observes it (the value-less
        // `query::<_, RelatedTo<R>>()` path) is a LOUD `#[cold]` panic, never a
        // silent wrong-target match.
        RelatedToState {
            id: R::component_id(),
            target: Self::POISON,
            _marker: PhantomData,
        }
    }

    #[inline]
    fn seed_state(state: &mut Self::State, value: &Self) {
        // Relations W1 — the value-carrying entry injects the runtime `target`
        // (the only runtime-valued piece of the state), clearing the poison.
        state.target = value.target;
    }

    #[inline]
    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        access_set
            .add_component_read(state.id, std::any::type_name::<Self>())
            .unwrap_or_else(|conflict| intra_system_conflict_panic(conflict));
    }

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        // Bound to archetypes hosting `R`; the per-row `filter_fetch` then
        // applies the exact `target` match.
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
        RelatedToFetch {
            base: std::ptr::null(),
            stride: 0,
            target: state.target,
        }
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY (QF3): `archetype` is a live `*const Archetype` for `'w`
        //   (caller contract). `columns` is at offset 0 (Phase 7 D4);
        //   `state.id.0 < MAX_COMPONENTS` by construction. `matches_component_set`
        //   guaranteed `R` present, so the column is non-null.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "RelatedTo: R column was unexpectedly null");
        fetch.base = column.ptr as *const R;
        fetch.stride = column.stride as usize;
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,
    ) {
        // SAFETY (QF3): the per-row read needs only shared access to the `R`
        //   column; reborrow the write-capable pointer as `*const` and forward.
        unsafe { Self::set_table_readonly(fetch, state, archetype as *const _, meta) }
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    ) {
        // Meta-free body — identical to `set_table_readonly` minus `_meta`.
        // SAFETY (QF3): same conditions as `set_table_readonly`.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "RelatedTo: R column was unexpectedly null");
        fetch.base = column.ptr as *const R;
        fetch.stride = column.stride as usize;
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // SAFETY (QF3): the per-row read needs only shared access; forward.
        unsafe { Self::set_table_readonly_no_meta(fetch, state, archetype as *const _) }
    }

    #[inline]
    unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
        // Relations W1 — eliminate the silent sentinel: a `RelatedTo<R>` that
        // reached a driver WITHOUT `query_filtered` seeding the runtime target
        // still carries `Self::POISON`. Fail LOUDLY rather than silently match
        // sources pointing at a phantom entity. The check is one compare against
        // a const; the panic body is `#[cold]` + `#[inline(never)]`, so the hot
        // per-row path keeps a single predicted-not-taken branch.
        if fetch.target == Self::POISON {
            related_to_unseeded_panic::<R>();
        }
        // Read the source row's own `R` FK and compare its target.
        // SAFETY (QF3): `fetch.base` was set by `set_table_*` for the current
        //   archetype (non-null, `R`-typed column); `row < entity_count` (the
        //   cursor's inner-loop guard). `base + row*stride` is the in-bounds,
        //   initialised `R` slot for this row. The shared `&R` reborrow lives
        //   only for the `target()` read; no other-entity access occurs (the
        //   `par_iter`-safety property).
        let r_ptr = unsafe { fetch.base.byte_add(row * fetch.stride) };
        let r: &R = unsafe { &*r_ptr };
        r.target() == fetch.target
    }
}

/// Relations W1 — the `#[cold]` loud-failure body for a `RelatedTo<R>` driven
/// without `query_filtered` seeding its runtime target (the poison-sentinel
/// guard in [`RelatedTo::filter_fetch`]). Isolating it keeps the per-row hot
/// path a single predicted-not-taken branch.
#[cold]
#[inline(never)]
fn related_to_unseeded_panic<R: Relationship>() -> ! {
    panic!(
        "RelatedTo<{}> was used through the value-less `query::<_, RelatedTo<_>>()` \
         path, which cannot supply a runtime target. Use \
         `world.query_filtered::<D, _>(RelatedTo::<{}>::new(target))` instead — the \
         value entry seeds the target the per-row predicate needs.",
        std::any::type_name::<R>(),
        std::any::type_name::<R>(),
    )
}
