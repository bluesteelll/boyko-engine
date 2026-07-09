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

    /// EnableTag C2 — `true` iff this data touches at least one real component
    /// (`&T`, `&mut T`, `Ref<T>`, `Mut<T>`, or a tuple containing one); `false`
    /// for `()`.
    ///
    /// A real data component contributes a positive include bit, bounding an
    /// `Enabled<T>`/`Disabled<T>` term's matched set. Feeds the `(D, F)`
    /// shape const-assert at the construction seam (Step 7a).
    ///
    /// No default — every impl MUST declare (mirrors the `NEEDS_CHANGE_DETECTION`
    /// I4 discipline: a silent fallthrough on a future leaf would
    /// mis-classify the query shape).
    const HAS_DATA_COMPONENT: bool;

    /// `true` iff this data needs a per-archetype post-filter trim — i.e. its
    /// [`Self::matches_component_set`] is **not** unconditionally `true`.
    ///
    /// Only [`AnyOf`] sets this (its ≥1-member OR predicate must run in the
    /// post-filter pass). Every leaf (`&T`, `&mut T`, `Ref`, `Mut`) requires
    /// the matched archetype to contain its component, so the existing
    /// `aggregate_include` / candidate-seed path already bounds them — no trim
    /// needed. `Option<D>` is `false` too: its `matches_component_set` is
    /// unconditionally `true`, so there is nothing to trim away.
    ///
    /// # Why defaulted (vs the no-default `NEEDS_CHANGE_DETECTION` / I4 rule)
    ///
    /// A `false` default is SAFE: it only ever needs flipping for an
    /// OR-matching data type, and a future such type forgetting to set it
    /// affects only the exotic `Query<AnyOf<…>, Enabled<C>>` candidate-seed
    /// combo, never a common hot path. The default keeps the
    /// [`QueryDataState::IS_CANDIDATE_SEEDED`] formula identical for every
    /// existing impl (0%-gate: additive, cold-only use).
    ///
    /// [`QueryDataState::IS_CANDIDATE_SEEDED`]: crate::ecs::core::iters::query::state::QueryDataState
    const REQUIRES_POST_FILTER_TRIM: bool = false;

    /// Dense plan D3 — `true` iff this data touches at least one dense
    /// (non-fragmenting) component (`&T` / `&mut T` where
    /// [`Component::STORAGE_IS_DENSE`]; a tuple OR-folds its members).
    ///
    /// Gates the entire dense-data machinery at monomorphisation:
    /// * the cursor's [`Self::resolve_dense`] call (the global `DenseStore`
    ///   pointer resolution from the world cell);
    /// * the per-row [`Self::dense_row_passes`] skip arm in `QueryIter`/
    ///   `QueryIterMut::next` (a dense row whose entity is absent from the
    ///   store is skipped like a non-match, mirroring `F::IS_ARCHETYPAL=false`).
    ///
    /// `false` by default, so every existing impl keeps it and pays nothing:
    /// the cursor's `if const { D::HAS_DENSE }` arms const-fold OUT and a
    /// no-dense query is byte-identical (the 0%-gate).
    ///
    /// [`Component::STORAGE_IS_DENSE`]: crate::ecs::core::component::component::Component::STORAGE_IS_DENSE
    const HAS_DENSE: bool = false;

    /// Dense plan D3 — resolves this data's dense term(s) against the world.
    ///
    /// Called ONCE per cursor construction (`QueryIter`/`QueryIterMut::new`,
    /// where the [`UnsafeEcsCell`] is available) under
    /// `if const { Self::HAS_DENSE }`, so the call is emitted ONLY into a
    /// dense monomorphisation. The default body is empty — every non-dense
    /// impl inherits it and the cursor's gated call folds to nothing (the
    /// 0%-gate). A dense `&T` / `&mut T` overrides it to cache the global
    /// `DenseStore` pointer (address-stable for the world borrow `'w`) into
    /// the `Fetch`; a non-dense leaf keeps the no-op.
    ///
    /// # Safety
    ///
    /// * `world` MUST satisfy the read contract declared by the active
    ///   `SystemParam::init_access` (the same cell `set_table_*` rides).
    /// * The resolved `DenseStore` pointer is valid for `'w` (the store lives
    ///   in the world's address-stable `DenseRegistry` slot array).
    ///
    /// [`UnsafeEcsCell`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell
    #[inline]
    unsafe fn resolve_dense<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
    ) {
        // Default: non-dense data resolves nothing (the 0%-gate). Overridden
        // only by a dense `&T` / `&mut T` leaf.
    }

    /// Dense plan D3 — per-row mixed-gather skip predicate.
    ///
    /// Called per row in `QueryIter`/`QueryIterMut::next` under
    /// `if const { Self::HAS_DENSE }` (folds OUT for non-dense `D`). Returns
    /// `false` iff some dense term in this data lacks the row's entity in its
    /// store — the row is then skipped like a filter non-match (the ruling's
    /// "None ⟹ skip" semantic). For non-dense data the default `true` is
    /// const-folded away.
    ///
    /// # Safety
    ///
    /// * `fetch` MUST have been initialised by `resolve_dense` (dense pointer)
    ///   AND a prior `set_table_*` call (the per-archetype `entity_ids` base).
    /// * `row < entity_count` of the cached archetype.
    #[inline]
    unsafe fn dense_row_passes<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> bool {
        true
    }

    /// Dense plan D3 — `true` iff this data contributes a dense INCLUDE term
    /// (`&T` / `&mut T` where `T::STORAGE_IS_DENSE`; a tuple OR-folds).
    ///
    /// A dense include is signature-excluded (no mask bit), so it cannot be
    /// bounded by the include-mask scan; the candidate archetypes are seeded
    /// from the dense store's `arch_presence` instead (the
    /// [`QueryDataState`] dense-seed path). `Option<&Dense>` / `AnyOf` do NOT
    /// set this (non-filtering — they never REQUIRE the dense member).
    /// `false` by default (the 0%-gate).
    ///
    /// [`QueryDataState`]: crate::ecs::core::iters::query::state::QueryDataState
    const HAS_DENSE_INCLUDE: bool = false;

    /// Dense plan D3 — ORs every dense INCLUDE term's `arch_presence` into
    /// `out` (the candidate-seed bitset).
    ///
    /// Called ONLY under `if const { Self::HAS_DENSE_INCLUDE }` by the
    /// `QueryDataState` dense-seed path. The default is a no-op. A dense `&T` /
    /// `&mut T` overrides it to read its store's `arch_presence` from
    /// `registry`; a non-dense leaf keeps the no-op.
    #[inline]
    fn dense_include_candidates(
        _state: &Self::State,
        _registry: &crate::ecs::core::component::dense::DenseRegistry,
        _out: &mut crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet,
    ) {
    }

    /// Relation-DSL join — `true` iff this data contains a
    /// [`Related<R, D>`](crate::ecs::core::iters::query::relation::Related)
    /// term (a single leaf or a tuple OR-folding one).
    ///
    /// Gates the relation-join machinery at monomorphisation:
    /// * the cursor's [`Self::resolve_related`] call (caches the world cell so
    ///   `fetch` can resolve the FK target's archetype per row);
    /// * the `par_iter` const-rejection (the chunk runner has no world cell, so
    ///   a `Related` join is sequential-only in v1 — mirrors `HAS_DENSE`).
    ///
    /// `false` by default, so every existing impl keeps it and pays nothing:
    /// the cursor's `if const { D::HAS_RELATED }` arm const-folds OUT and a
    /// non-relation query is byte-identical (the 0%-gate).
    const HAS_RELATED: bool = false;

    /// Relation-DSL join — resolves this data's relation term(s) against the
    /// world.
    ///
    /// Called ONCE per cursor construction (`QueryIter`/`QueryIterMut::new`,
    /// where the [`UnsafeEcsCell`] is available) under
    /// `if const { Self::HAS_RELATED }`, so the call is emitted ONLY into a
    /// relation monomorphisation. The default body is empty — every
    /// non-relation impl inherits it and the cursor's gated call folds to
    /// nothing (the 0%-gate). A
    /// [`Related<R, D>`](crate::ecs::core::iters::query::relation::Related)
    /// overrides it to cache the world cell (the world-global resolution base)
    /// into its `Fetch` so the per-row `fetch` can resolve the FK target's
    /// `entities_inland` record and the joined column.
    ///
    /// # Safety
    ///
    /// * `world` MUST satisfy the read contract declared by the active
    ///   `SystemParam::init_access` (the same cell `set_table_*` rides).
    /// * The cell is valid for `'w` (the cursor lifetime).
    ///
    /// [`UnsafeEcsCell`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell
    #[inline]
    unsafe fn resolve_related<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
    ) {
        // Default: non-relation data resolves nothing (the 0%-gate). Overridden
        // only by a `Related<R, D>` leaf.
    }

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

// ── Fetch-type submodules (mechanical split of the former monolith) ────────
//
// Each submodule holds one query-data fetch type plus its `QueryData` /
// `ReadOnlyQueryData` (and `AnyOfArm`) impls, moved verbatim. `pub use <sub>::*`
// re-exports every item at this module's path so `query::data::X` still
// resolves for every in-crate and downstream caller. `tuple_impls` holds only
// trait impls (globally in effect via `mod`; no names to re-export).
mod anyof;
mod mut_;
mod option;
mod read;
mod ref_;
mod tuple_impls;
mod write;

pub use anyof::*;
pub use mut_::*;
pub use option::*;
pub use read::*;
pub use ref_::*;
pub use write::*;

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
