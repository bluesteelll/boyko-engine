//! `Related<R, D>` — the relation JOIN query-data term.
//!
//! A read-only join: for each SOURCE row, read its `R` foreign key, resolve the
//! FK TARGET entity, and gather `D` from the TARGET's row — yielding
//! `Option<D::Item>` (`None` when the FK is absent, the target is dead/stale, or
//! the target's archetype does not host `D`).
//!
//! ```ignore
//! Query<(&Velocity, Related<ChildOf, &Transform>)>   // (&Velocity, Option<&Transform>)
//! ```
//!
//! # Read-only only (by design)
//!
//! `D: ReadOnlyQueryData` is a hard bound — a `&mut` join is FORBIDDEN. A
//! mutable join would let two source rows pointing at the same target alias a
//! `&mut` into the target's row; the read-only restriction structurally rules
//! that out.
//!
//! # Aliasing safety is the conflict graph's job (no per-row alias check)
//!
//! [`init_access`](crate::ecs::core::iters::query::data::QueryData::init_access)
//! declares `R`'s read FIRST, then forwards to `D::init_access` against the SAME
//! set in declaration order. So `Query<(&mut T, Related<R, &T>)>` (and the
//! reverse order) trips the existing
//! [`ComponentReadVsWrite`](crate::ecs::core::system::filtered_access_set)
//! / `ComponentWriteVsRead` detector at build time (boyko-B0002) — we rely on
//! that and add NO runtime per-row alias check.
//!
//! # Sequential-only
//!
//! `HAS_RELATED = true` is const-REJECTED on `par_iter` (the chunk runner has no
//! world cell to resolve the FK target's archetype per row). `Related` rides the
//! sequential `QueryIter` / `QueryIterMut` cursors, which cache the world cell
//! via [`resolve_related`](crate::ecs::core::iters::query::data::QueryData::resolve_related).
//!
//! # Chunked iteration is excluded
//!
//! The join breaks the contiguous-slice contract (the target rows are scattered
//! across archetypes), so `Related` does NOT implement
//! [`ChunkedQueryData`](crate::ecs::core::iters::query::chunked_data::ChunkedQueryData)
//! — exactly like `Ref` / `Changed` / dense.

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::iters::query::data::{QueryData, ReadOnlyQueryData};
use crate::ecs::core::relationship::Relationship;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// The relation JOIN term: read `D` from the entity that this row's `R` foreign
/// key points at. Yields `Option<D::Item>` per source row.
///
/// `R` is the relationship FK on the SOURCE; `D` is the read-only data fetched
/// from the TARGET. See the module docs for the read-only / sequential-only /
/// aliasing contracts.
///
/// # Usable as a `Query` SystemParam (FINDING-1)
///
/// `Related<R, D>` carries NO `D: 'static` bound, so the inner reference `D`
/// borrows for the per-call lifetime rather than being pinned to `&'static T`.
/// It therefore type-checks as a `Query` SystemParam inside a system body and is
/// iterated SEQUENTIALLY via `.iter()` (the join is sequential-only — `.par_iter()`
/// const-rejects it). This doctest is a COMPILE-ONLY proof (the inner borrow no
/// longer escapes):
///
/// ```no_run
/// use boyko_ecs::prelude::*;
/// use boyko_macros::Component;
///
/// #[derive(Component, Clone, Copy)]
/// #[repr(C)]
/// struct Vel { x: f32 }
///
/// #[derive(Component, Clone, Copy)]
/// #[repr(C)]
/// struct Pos { x: f32 }
///
/// // A SystemParam Query whose data tuple contains a relation join. Before the
/// // FINDING-1 fix this failed with E0521 ("borrowed data escapes"); now it
/// // type-checks and iterates sequentially.
/// fn read_parent_pos(q: Query<(&Vel, Related<ChildOf, &Pos>)>) {
///     for (_vel, parent_pos) in q.iter() {
///         // `parent_pos: Option<&Pos>` — `Some` when the FK target hosts `Pos`.
///         let _ = parent_pos;
///     }
/// }
///
/// // Tuple, `Ref`, and `Option` inners are equally usable as SystemParams.
/// fn variants(
///     _a: Query<Related<ChildOf, (&Pos, &Vel)>>,
///     _b: Query<Related<ChildOf, Ref<Pos>>>,
///     _c: Query<Related<ChildOf, Option<&Pos>>>,
/// ) {
/// }
///
/// let _ = (read_parent_pos, variants);
/// ```
pub struct Related<R: Relationship, D: ReadOnlyQueryData> {
    _marker: PhantomData<fn() -> (R, D)>,
}

/// Per-system state for [`Related<R, D>`]: `R`'s id + the inner `D`'s state.
///
/// Parameterised over the inner STATE type `S = D::State` rather than over `D`
/// itself (FINDING-1). A nominal `RelatedState<R, D>` would require `D: 'static`
/// to prove `RelatedState<R, D>: 'static` (the `QueryData::State: 'static`
/// obligation) — pinning a `D = &'a T` inner to `&'static T`. Carrying `S`
/// directly sidesteps that: `RelatedState<R, S>: 'static` follows from `R: 'static`
/// (`Relationship: Component: 'static`) and `S: 'static` (the inner `D::State`'s
/// own trait bound), with NO `D: 'static`. The `QueryData` impl instantiates it as
/// `RelatedState<R, D::State>`.
pub struct RelatedState<R: Relationship, S: Send + Sync + 'static> {
    /// Cached [`ComponentId`](crate::ecs::identifiers::primitives::ComponentId)
    /// of the source FK component `R`.
    r_id: crate::ecs::identifiers::primitives::ComponentId,
    /// The inner data's per-system state (resolved against the TARGET's row).
    inner: S,
    _marker: PhantomData<fn() -> R>,
}

/// Per-archetype + world fetch scratch for [`Related<R, D>`].
///
/// * `world` — the world cell, cached ONCE by `resolve_related` (the
///   world-global resolution base for the per-row FK target lookup).
/// * `r_base` / `r_stride` — the SOURCE archetype's `R` column, cached per
///   source archetype by `set_table_readonly`.
/// * `inner_state` / `meta` — pointers used to build a TRANSIENT `D::Fetch`
///   against the target's archetype per row.
pub struct RelatedFetch<'w, R: Relationship, D: ReadOnlyQueryData> {
    /// The world cell, cached by `resolve_related`. `None` until then (QD2);
    /// `init_fetch` has no world, so the placeholder is `None` and every `fetch`
    /// runs after `resolve_related` has filled it (the cursor calls
    /// `resolve_related` once in `new`, before any `fetch`).
    world: Option<UnsafeEcsCell<'w>>,
    /// Base pointer to the SOURCE archetype's `R` column. NULL until
    /// `set_table_readonly` runs.
    r_base: *const R,
    /// `R` column stride (bytes): `r_ptr = r_base + row*r_stride`.
    r_stride: usize,
    /// Pointer to the inner `D`'s per-system state (lives in `RelatedState`,
    /// stable for the system-state borrow).
    inner_state: *const D::State,
    /// Pointer to the active system's per-frame tick snapshot (forwarded into
    /// the transient inner `D::set_table_readonly`).
    meta: *const SystemMeta,
    /// Binds `R` / `D` WITHOUT an `&'w D` outlives obligation: `fn() -> (R, D)` is
    /// contravariant and imposes no `D: 'w` bound, so `D = &'a T` (a non-`'static`
    /// inner) is admissible (FINDING-1). `'w` is already carried by the real
    /// `world: Option<UnsafeEcsCell<'w>>` field, so the marker need not repeat it.
    /// Mirrors `OptionFetch`, which likewise never stores `&'w D`.
    _marker: PhantomData<fn() -> (R, D)>,
}

impl<R: Relationship, D: ReadOnlyQueryData> Clone for RelatedFetch<'_, R, D> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: Relationship, D: ReadOnlyQueryData> Copy for RelatedFetch<'_, R, D> {}

// SAFETY (QD1-QD4 + relation join):
//   - QD1: `state.r_id` is `R::component_id()`; `init_access` declares `R`'s
//     read THEN forwards to `D::init_access` (the join's full read surface).
//   - QD2: `init_fetch` NULLs `r_base`; `resolve_related` caches `world` and
//     `set_table_readonly` caches `r_base`/`r_stride` before any `fetch`.
//   - QD3: every cached pointer is scoped to `'w`.
//   - QD4: read-only data — `set_table_mut` delegates to `set_table_readonly`
//     (the join never writes); the `_no_meta` variants forward likewise.
//
// Bound: NO `D: 'static` (FINDING-1). The associated
// `State = RelatedState<R, D::State>` is `Send + Sync + 'static` from its parts
// alone: `r_id: ComponentId` is `'static`, `inner: D::State` is `Send + Sync +
// 'static` by the `QueryData` trait's own `State` bound, and `R: Relationship:
// Component: 'static`. Parameterising the state struct over `D::State` (not over
// `D`) is what lets the compiler prove `'static` without `D: 'static` — a nominal
// `RelatedState<R, D>` would have demanded `D: 'static`, pinning `D = &'a T` to
// `&'static T`, and `Query<'w,'s,D,F>`'s invariance over `D` then rejected the
// per-call borrow (E0521), blocking `Related` as a `Query` SystemParam. Mirrors
// `Option<D>`'s bare `D: QueryData` (it reuses `State = D::State` for the same
// reason).
unsafe impl<R: Relationship, D: ReadOnlyQueryData> QueryData for Related<R, D> {
    type State = RelatedState<R, D::State>;
    type Fetch<'w> = RelatedFetch<'w, R, D>;
    type Item<'w> = Option<D::Item<'w>>;
    const IS_READ_ONLY: bool = true;
    // The inner `D` may itself need change detection (e.g. `Related<R, Ref<T>>`);
    // propagate so the cursor forwards `meta`.
    const NEEDS_CHANGE_DETECTION: bool = D::NEEDS_CHANGE_DETECTION;
    // `Related` does not REQUIRE the source to host a data component of its own
    // (the SOURCE only needs `R`); the inner `D` is fetched from the TARGET, so
    // it adds no source-side include bit. The matched set is bounded by `R`'s
    // presence via `aggregate_include` below.
    const HAS_DATA_COMPONENT: bool = true;
    // The relation-join seam — gates `resolve_related` + the `par_iter`
    // const-rejection.
    const HAS_RELATED: bool = true;

    #[inline]
    fn init_state(world: &mut EcsMaster) -> Self::State {
        // v1 scope: the inner `D` is a PLAIN read of the TARGET's table columns.
        // A DENSE inner (`&DenseComponent`) is signature-excluded — its column
        // does not exist on the target archetype and the join does not resolve a
        // `DenseStore` per row — so it would silently mis-resolve; reject it at
        // monomorphisation. A NESTED relation inner (`Related<R, Related<..>>`)
        // is likewise out of v1 scope. Both fold to `assert!(true)` for the
        // common `&T` / tuple-of-`&T` / `Ref<T>` / `Option<&T>` inner (the
        // 0%-gate).
        const {
            assert!(
                !D::HAS_DENSE,
                "Related<R, D>: a DENSE inner component is not supported in v1 — \
                 the join reads the FK target's table columns directly. Query the \
                 dense component on the target via a separate dense query."
            );
            assert!(
                !D::HAS_RELATED,
                "Related<R, D>: a nested relation inner (Related<.., Related<..>>) \
                 is not supported in v1 — flatten the join or traverse via \
                 `ancestors` / `descendants`."
            );
        };
        RelatedState {
            r_id: R::component_id(),
            inner: D::init_state(world),
            _marker: PhantomData,
        }
    }

    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        // C2: declare `R`'s read FIRST, THEN the inner `D` — against the SAME
        // set, in declaration order, NO deferral. This makes
        // `Query<(&mut T, Related<R, &T>)>` (and the reverse) panic at build via
        // the existing ComponentReadVsWrite / ComponentWriteVsRead detector.
        access_set
            .add_component_read(state.r_id, std::any::type_name::<Self>())
            .unwrap_or_else(|conflict| {
                crate::ecs::core::system::params::diagnostics::intra_system_conflict_panic(conflict)
            });
        D::init_access(&state.inner, access_set);
    }

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        // The SOURCE row must host `R` (its FK); the inner `D` is checked
        // per-row against the TARGET's archetype, NOT here.
        mask.contains(state.r_id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        // Bound the matched (SOURCE) archetypes to those hosting `R`. The inner
        // `D`'s components belong to the TARGET, so they MUST NOT be required of
        // the source archetype.
        include.set(state.r_id);
    }

    #[inline]
    fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
        RelatedFetch {
            // No world available at `init_fetch`; `resolve_related` fills it
            // before any `fetch` (QD2 + the cursor's `resolve_related` call in
            // `new`).
            world: None,
            r_base: std::ptr::null(),
            r_stride: 0,
            inner_state: &state.inner as *const D::State,
            meta: std::ptr::null(),
            _marker: PhantomData,
        }
    }

    #[inline]
    unsafe fn resolve_related<'w>(
        fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        world: UnsafeEcsCell<'w>,
    ) {
        // Cache the world cell ONCE (the world-global resolution base). Gated by
        // `const { Self::HAS_RELATED }` at the cursor, so this is emitted only
        // into a relation monomorphisation.
        // SAFETY (relation join): `world` is the cursor's cell scoped to `'w`;
        //   the `Copy` cell is stored by value, preserving provenance.
        fetch.world = Some(world);
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        meta: &'_ SystemMeta,
    ) {
        // Cache the SOURCE archetype's `R` column + the meta pointer (forwarded
        // into the per-row transient inner fetch).
        // SAFETY (QD3): `archetype` is a live `*const Archetype` for `'w`
        //   (caller contract); `columns` at offset 0 (Phase 7 D4); `r_id.0 <
        //   MAX_COMPONENTS`. `matches_component_set` proved `R` present, so the
        //   column is non-null.
        let column = unsafe { (*archetype).columns.get_unchecked(state.r_id.0) };
        debug_assert!(!column.ptr.is_null(), "Related: R column was unexpectedly null");
        fetch.r_base = column.ptr as *const R;
        fetch.r_stride = column.stride as usize;
        fetch.meta = meta as *const SystemMeta;
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,
    ) {
        // Read-only join: degrade the mutable variant to the read path (the
        // source `R` column is read-only; the target `D` is read-only).
        // SAFETY (QD3, QD4): `archetype` carries strictly-stronger write-capable
        //   provenance than the read-only path needs.
        unsafe { Self::set_table_readonly(fetch, state, archetype as *const _, meta) }
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    ) {
        // NCD-free path (reached only when `D::NEEDS_CHANGE_DETECTION == false`).
        // The inner `D::set_table_readonly_no_meta` is what `fetch` will call, so
        // a NULL `meta` is never read by the inner term on this monomorphisation.
        // SAFETY (QD3): same as `set_table_readonly` minus the unused meta.
        let column = unsafe { (*archetype).columns.get_unchecked(state.r_id.0) };
        debug_assert!(!column.ptr.is_null(), "Related: R column was unexpectedly null");
        fetch.r_base = column.ptr as *const R;
        fetch.r_stride = column.stride as usize;
        fetch.meta = std::ptr::null();
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // SAFETY (QD3, QD4): read-only join; forward to the read-only path.
        unsafe { Self::set_table_readonly_no_meta(fetch, state, archetype as *const _) }
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        // THE only new unsafe in this phase. Two dependent RANDOM loads per row
        // (`entities_inland[target.id]` then the target column) — UNPREFETCHABLE
        // (the FK target is data-dependent, scattered across archetypes), so this
        // join is inherently random-access on the target side.
        //
        // SAFETY (the `get_component` contract, verbatim):
        //   - `fetch.r_base` was set by `set_table_*` for the current SOURCE
        //     archetype (non-null, `R`-typed); `row < entity_count` (cursor
        //     guard), so `r_base + row*r_stride` is the in-bounds, initialised
        //     `R` slot. The shared `&R` reborrow lives only for the `target()`
        //     read.
        let r_ptr = unsafe { fetch.r_base.byte_add(row * fetch.r_stride) };
        let r: &R = unsafe { &*r_ptr };
        let target = r.target();

        // `resolve_related` cached the cell before any `fetch` (QD2); a `None`
        // here would be a contract violation by the cursor — fall back to `None`
        // defensively rather than panic on a malformed custom driver.
        let cell = fetch.world?;
        // SAFETY (relation join): `cell` was cached by `resolve_related` (the
        //   cursor calls it under `const { HAS_RELATED }` BEFORE any `fetch`);
        //   the cell is valid for `'w`. `world()` upholds the read contract
        //   declared by `init_access` (the same cell `set_table_*` rides).
        let world: &EcsMaster = unsafe { cell.world() };

        // Resolve the TARGET's location (`entities_inland[target.id]`) — the
        // first dependent random load. Null/generation check ⇒ `None` (dead /
        // stale / never-registered target), mirroring `get_component_raw`.
        let inland = world.entity_master.entities_inland.get(target.id().0)?;
        if inland.is_null() || inland.generation() != target.generation() {
            return None;
        }
        let target_arch_ptr = inland.archetype_ptr();
        let target_row = inland.unit_index() as usize;

        // Archetype-level membership of `D` in the TARGET's archetype. If the
        // target does not host `D`'s component(s), the join yields `None`.
        // SAFETY (slab address-stability, `get_component_raw`'s U1/U2/F1
        //   contract): `target_arch_ptr` is a live, address-stable slab pointer
        //   (non-null + generation-matched above ⇒ the slot is live); `&self`
        //   shared access reads only the immutable `signature` mask. No sibling
        //   structural migration interleaves (single-threaded sequential cursor
        //   under the conflict-graph borrow discipline).
        let target_mask: &ComponentMask = unsafe { (*target_arch_ptr).component_mask() };
        // SAFETY: `inner_state` points at `RelatedState::inner`, stable for the
        //   system-state borrow that outlives `'w`.
        let inner_state: &D::State = unsafe { &*fetch.inner_state };
        if !D::matches_component_set(inner_state, target_mask) {
            return None;
        }
        debug_assert!(
            target_row < unsafe { (*target_arch_ptr).entity_count() },
            "Related: target unit_index out of bounds for its archetype"
        );

        // Build a TRANSIENT inner `D::Fetch` against the TARGET's archetype and
        // gather `D` at the target's row. `D` is read-only, so the read-only
        // `set_table_*` path is the sole route; the NCD const-fold picks the
        // meta-bearing or meta-free variant matching the cursor's dispatch.
        let mut inner_fetch = <D as QueryData>::init_fetch(inner_state);
        // The inner `D` is a plain table read of the TARGET's columns: v1
        // const-rejects a dense or nested-relation inner (`init_state`), so
        // neither `resolve_dense` nor `resolve_related` is needed here — only the
        // table `set_table_*` + `fetch` path is exercised.
        //
        // SAFETY (QD3, QD4): `target_arch_ptr` is a live `*const Archetype` for
        //   `'w` (slab stability); it hosts every `D` component (proved by
        //   `matches_component_set`); `D: ReadOnlyQueryData` ⇒ the read-only
        //   `set_table_*` never traps; `target_row < entity_count` (asserted).
        unsafe {
            if const { D::NEEDS_CHANGE_DETECTION } {
                // SAFETY: `fetch.meta` is non-null on the NCD monomorphisation
                //   (the meta-bearing `set_table_readonly` cached it); the inner
                //   `Ref<T>` copies the ticks by value.
                let meta: &SystemMeta = &*fetch.meta;
                <D as QueryData>::set_table_readonly(
                    &mut inner_fetch,
                    inner_state,
                    target_arch_ptr,
                    meta,
                );
            } else {
                <D as QueryData>::set_table_readonly_no_meta(
                    &mut inner_fetch,
                    inner_state,
                    target_arch_ptr,
                );
            }
            Some(<D as QueryData>::fetch(&inner_fetch, target_row))
        }
    }
}

// SAFETY: `Related<R, D>` performs no writes — it joins a read-only `D` from the
// FK target. `IS_READ_ONLY = true`. No `D: 'static` (FINDING-1) — see the
// `QueryData` impl above for why the bound was unnecessary and harmful.
unsafe impl<R: Relationship, D: ReadOnlyQueryData> ReadOnlyQueryData for Related<R, D> {}
