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
    ///
    /// For a DENSE `T` this column does NOT exist in the archetype (dense is
    /// signature-excluded), so this field stays NULL and the per-row item is
    /// gathered through `dense` + `entity_ids` instead (Dense plan D3).
    pub(crate) base: *const T,
    /// Dense plan D3 — the global `DenseStore` for `T`, resolved ONCE by
    /// [`resolve_dense`](QueryData::resolve_dense) from the world cell. NULL
    /// for a TABLE `T` (the field is never read on that path — gated by
    /// `const { T::STORAGE_IS_DENSE }`, so the 0%-gate holds). Address-stable
    /// for `'w` (the store lives in the world's `DenseRegistry` slot array).
    pub(crate) dense: *const crate::ecs::core::component::dense::DenseStore,
    /// Dense plan D3 — the current archetype's `entity_ids` column base, cached
    /// by `set_table_*` for the dense `T` path (`entity = entity_ids[row]`).
    /// NULL / unused for a TABLE `T`.
    pub(crate) entity_ids: *const crate::ecs::identifiers::primitives::EntityId,
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
    // EnableTag C2: `&T` is a real data component (positive include bit).
    const HAS_DATA_COMPONENT: bool = true;
    // Dense plan D3: dense-ness is a compile-time property of `T`.
    const HAS_DENSE: bool = T::STORAGE_IS_DENSE;
    // Dense plan D3: a dense `&T` is a dense INCLUDE term (seeds candidates).
    const HAS_DENSE_INCLUDE: bool = T::STORAGE_IS_DENSE;

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
        // A dense component is signature-excluded, so a matched archetype's mask
        // NEVER carries its bit. The dense membership is the per-row `e2s` oracle
        // (D3); at the archetype level a dense include must NOT gate the mask, or
        // it would reject every archetype. The candidate seed (`arch_presence`)
        // is what bounds the dense archetype set.
        if const { T::STORAGE_IS_DENSE } {
            return true;
        }
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        // Dense is signature-excluded: it contributes NO include bit (its bit
        // would never be set on any archetype, so the query would match nothing).
        // Candidate archetypes are seeded from `DenseStore::arch_presence`
        // instead (D3 seed wiring in `QueryDataState`).
        if const { T::STORAGE_IS_DENSE } {
            return;
        }
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        ReadFetch {
            base: std::ptr::null(),
            dense: std::ptr::null(),
            entity_ids: std::ptr::null(),
            _marker: PhantomData,
        }
    }

    #[inline]
    unsafe fn resolve_dense<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
    ) {
        // Gated by `const { Self::HAS_DENSE }` at the cursor; for a TABLE `T`
        // this whole body is never emitted (the 0%-gate). For a dense `T`,
        // cache the global `DenseStore` pointer once. A query whose dense store
        // does not exist yet (no entity ever inserted) leaves `dense` NULL — the
        // per-row gather then skips every row (no member exists).
        // SAFETY (resolve_dense contract): `world` upholds the read access
        //   contract; `world()` yields `&'w EcsMaster`. The `DenseStore` (if
        //   present) lives in the address-stable `DenseRegistry` slot array,
        //   valid for `'w`. The raw pointer carries no `!Send` payload — it is
        //   confined to this `'w`-scoped `Fetch`, never the Send `State`.
        let registry = unsafe { world.world().dense_registry() };
        fetch.dense = match registry.store(state.id) {
            Some(store) => store as *const _,
            None => std::ptr::null(),
        };
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        _meta: &'_ SystemMeta,
    ) {
        if const { T::STORAGE_IS_DENSE } {
            // Dense `T`: the archetype has NO column for it. Cache the
            // `entity_ids` base for the per-row gather instead.
            // SAFETY (QD3): `archetype` is a live `*const Archetype` for `'w`;
            //   the shared reborrow only reads the immutable `entity_ids` slice
            //   base. The dense store pointer was resolved by `resolve_dense`.
            let arch_ref: &Archetype = unsafe { &*archetype };
            fetch.entity_ids = arch_ref.entity_ids_slice().as_ptr();
            return;
        }
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
        if const { T::STORAGE_IS_DENSE } {
            // SAFETY (QD3): see `set_table_readonly`'s dense arm.
            let arch_ref: &Archetype = unsafe { &*archetype };
            fetch.entity_ids = arch_ref.entity_ids_slice().as_ptr();
            return;
        }
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
    unsafe fn dense_row_passes<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
        if const { T::STORAGE_IS_DENSE } {
            // SAFETY (D3): on the dense path `entity_ids` was cached by
            //   `set_table_*` for the current archetype and `row < entity_count`
            //   (caller contract). A NULL `dense` (store never created) ⟹ no
            //   member ⟹ skip every row.
            if fetch.dense.is_null() {
                return false;
            }
            let entity = unsafe { *fetch.entity_ids.add(row) };
            let store = unsafe { &*fetch.dense };
            return store.slot_of(entity).is_some();
        }
        true
    }

    #[inline]
    fn dense_include_candidates(
        state: &Self::State,
        registry: &crate::ecs::core::component::dense::DenseRegistry,
        out: &mut crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet,
    ) {
        // Gated by `const { Self::HAS_DENSE_INCLUDE }`; for a table `T` this is
        // never called. OR-in every archetype that has ever hosted a member of
        // `T`'s dense store (the conservative candidate seed — false positives
        // are trimmed per-row by `dense_row_passes`).
        if const { T::STORAGE_IS_DENSE }
            && let Some(store) = registry.store(state.id)
        {
            store.arch_presence().for_each_set_bit(|a| out.insert(a));
        }
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        if const { T::STORAGE_IS_DENSE } {
            // Dense per-row mixed gather: row → entity → slot → row_ptr.
            // `dense_row_passes` already proved the slot exists for this row
            // (the cursor calls it first and `continue`s on a miss), so
            // `slot_of` is `Some` here.
            // SAFETY (D3): `entity_ids`/`dense` were cached by `set_table_*` /
            //   `resolve_dense`; `row < entity_count`; the entity is a live
            //   member (proved by `dense_row_passes`), so `row_ptr(slot)` is a
            //   live, stride-aligned pointer into the address-stable column.
            //   The cast to `&'w T` matches the store's registered type; `'w`
            //   ties to the world borrow. No aliasing: the cursor borrow
            //   discipline + conflict graph serialise the dense column.
            unsafe {
                let entity = *fetch.entity_ids.add(row);
                let store = &*fetch.dense;
                let slot = store
                    .slot_of(entity)
                    .expect("invariant: dense_row_passes proved the slot is live");
                let ptr = store.solve_view().row_ptr(slot as usize);
                return &*(ptr as *const T);
            }
        }
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
    ///
    /// For a DENSE `T` this archetype column does NOT exist (dense is
    /// signature-excluded); the field stays NULL and the per-row write target
    /// is `DenseSolveView::row_ptr(slot)` (Dense plan D3).
    pub(crate) base: *mut T,
    /// Dense plan D3 — the global `DenseStore` for `T`, resolved ONCE by
    /// [`resolve_dense`](QueryData::resolve_dense). NULL for a TABLE `T`. Its
    /// `solve_view().row_ptr(slot)` is the write-through target. Address-stable
    /// for `'w`.
    pub(crate) dense: *const crate::ecs::core::component::dense::DenseStore,
    /// Dense plan D3 — the current archetype's `entity_ids` column base, cached
    /// by `set_table_mut` for the dense gather. NULL / unused for TABLE `T`.
    pub(crate) entity_ids: *const crate::ecs::identifiers::primitives::EntityId,
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
    // EnableTag C2: `&mut T` is a real data component (positive include bit).
    const HAS_DATA_COMPONENT: bool = true;
    // Dense plan D3: dense-ness is a compile-time property of `T`.
    const HAS_DENSE: bool = T::STORAGE_IS_DENSE;
    // Dense plan D3: a dense `&mut T` is a dense INCLUDE term (seeds candidates).
    const HAS_DENSE_INCLUDE: bool = T::STORAGE_IS_DENSE;

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
        // Dense is signature-excluded (see `&T`'s matches_component_set): the
        // mask never carries its bit, so a dense include must NOT gate at the
        // archetype level. Per-row membership is the `e2s` oracle (D3).
        if const { T::STORAGE_IS_DENSE } {
            return true;
        }
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        // Dense contributes NO include bit (signature-excluded); candidates are
        // seeded from `arch_presence` (D3 seed wiring).
        if const { T::STORAGE_IS_DENSE } {
            return;
        }
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        WriteFetch {
            base: std::ptr::null_mut(),
            dense: std::ptr::null(),
            entity_ids: std::ptr::null(),
            _marker: PhantomData,
        }
    }

    #[inline]
    unsafe fn resolve_dense<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
    ) {
        // Gated by `const { Self::HAS_DENSE }` at the cursor — never emitted for
        // a TABLE `T` (0%-gate). For a dense `T`, cache the global `DenseStore`
        // pointer (the write-through target's owner).
        // SAFETY (resolve_dense contract): `world` upholds the access contract;
        //   the `DenseStore` lives in the address-stable `DenseRegistry` slot
        //   array, valid for `'w`. The pointer is confined to the `'w`-scoped
        //   `Fetch`, never the Send `State`.
        let registry = unsafe { world.world().dense_registry() };
        fetch.dense = match registry.store(state.id) {
            Some(store) => store as *const _,
            None => std::ptr::null(),
        };
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
        if const { T::STORAGE_IS_DENSE } {
            // Dense `T`: no archetype column. Cache `entity_ids` for the gather;
            // the write target is `DenseSolveView::row_ptr(slot)`.
            // SAFETY (QD3): `archetype` is live for `'w`; reading the immutable
            //   `entity_ids` slice base needs only shared access.
            let arch_ref: &Archetype = unsafe { &*archetype };
            fetch.entity_ids = arch_ref.entity_ids_slice().as_ptr();
            return;
        }
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
        if const { T::STORAGE_IS_DENSE } {
            // SAFETY (QD3): see `set_table_mut`'s dense arm.
            let arch_ref: &Archetype = unsafe { &*archetype };
            fetch.entity_ids = arch_ref.entity_ids_slice().as_ptr();
            return;
        }
        // Meta-free body — identical to `set_table_mut` minus the unused
        // `_meta`. NCD = false (`&mut T` does not consult ticks).
        // SAFETY (QD1, QD3): same conditions as `set_table_mut`.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "QD2: column was unexpectedly null");
        fetch.base = column.ptr as *mut T;
    }

    #[inline]
    unsafe fn dense_row_passes<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
        if const { T::STORAGE_IS_DENSE } {
            // SAFETY (D3): `entity_ids` cached by `set_table_mut`; `row <
            //   entity_count`. NULL `dense` ⟹ no member ⟹ skip.
            if fetch.dense.is_null() {
                return false;
            }
            let entity = unsafe { *fetch.entity_ids.add(row) };
            let store = unsafe { &*fetch.dense };
            return store.slot_of(entity).is_some();
        }
        true
    }

    #[inline]
    fn dense_include_candidates(
        state: &Self::State,
        registry: &crate::ecs::core::component::dense::DenseRegistry,
        out: &mut crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet,
    ) {
        // See `&T::dense_include_candidates` — OR-in the dense store's
        // `arch_presence`. Gated by `const { Self::HAS_DENSE_INCLUDE }`.
        if const { T::STORAGE_IS_DENSE }
            && let Some(store) = registry.store(state.id)
        {
            store.arch_presence().for_each_set_bit(|a| out.insert(a));
        }
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        if const { T::STORAGE_IS_DENSE } {
            // Dense write-through: row → entity → slot → row_ptr (mut).
            // `dense_row_passes` proved the slot is live before this call.
            // SAFETY (D3): `entity_ids`/`dense` cached; `row < entity_count`;
            //   the entity is a live member, so `row_ptr(slot)` is a live,
            //   stride-aligned WRITE-capable pointer into the address-stable
            //   column. Exclusivity: the `&mut`-cursor borrow discipline +
            //   the conflict graph (Decision 6 — one dense node) serialise the
            //   dense column, so no other writer aliases this slot. The
            //   `*mut u8 -> *mut T` cast matches the registered type; `'w`
            //   ties to the world borrow.
            unsafe {
                let entity = *fetch.entity_ids.add(row);
                let store = &*fetch.dense;
                let slot = store
                    .slot_of(entity)
                    .expect("invariant: dense_row_passes proved the slot is live");
                let ptr = store.solve_view().row_ptr(slot as usize);
                return &mut *(ptr as *mut T);
            }
        }
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
///   `Archetype::tick_column_base`.
/// * `last_run` / `this_run` — the system's tick snapshot captured at
///   `set_table_*` time so the per-row hot loop pays no indirection.
///
/// All fields are populated by `set_table_*` before any `fetch` call; the
/// tick columns are write-once sub-regions of the pool's own
/// `VmReservation`, so their base addresses are stable for the pool's
/// lifetime (Phase X.I vm-reservation stability).
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
    // EnableTag C2: `Ref<T>` is a real data component (positive include bit).
    const HAS_DATA_COMPONENT: bool = true;

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
    /// deref guard. Stable for the pool's lifetime (write-once tick
    /// sub-region of the pool's own `VmReservation` — Phase X.I).
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
            // The assignment above writes THROUGH the inner `&mut T`
            // (`self.value`) directly — it does NOT route through
            // `Mut::deref_mut`, so the deref-bump guard never fired. Bump the
            // changed tick manually here.
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
            //     slot lives in a write-once tick sub-region of the pool's
            //     own `VmReservation`, address-stable for the pool's
            //     lifetime (Phase X.I).
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
/// the vm-reservation address stability (write-once sub-regions of the
/// pool's own reservation — Phase X.I).
pub struct MutFetch<'w, T: Component> {
    pub(crate) value_base: *mut T,
    pub(crate) added_base: *const UnsafeCell<Tick>,
    pub(crate) changed_base: *const UnsafeCell<Tick>,
    pub(crate) last_run: Tick,
    pub(crate) this_run: Tick,
    /// Dense plan D4: the global `DenseStore` for a DENSE `T`, resolved once by
    /// `resolve_dense`. NULL for a TABLE `T` (the field is never read — the dense
    /// arm const-folds out, the 0%-gate) or when no dense store exists yet.
    pub(crate) dense: *const crate::ecs::core::component::dense::DenseStore,
    /// Dense plan D4: the current archetype's `entity_ids` column base, cached by
    /// `set_table_mut` for the per-row gather (`entity = entity_ids[row]`). NULL
    /// for a TABLE `T`.
    pub(crate) entity_ids: *const crate::ecs::identifiers::primitives::EntityId,
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
    // EnableTag C2: `Mut<T>` is a real data component (positive include bit).
    const HAS_DATA_COMPONENT: bool = true;
    // Dense plan D4: dense-ness is a compile-time property of `T`.
    const HAS_DENSE: bool = T::STORAGE_IS_DENSE;
    // Dense plan D4: a dense `Mut<T>` is a dense INCLUDE term (seeds candidates).
    const HAS_DENSE_INCLUDE: bool = T::STORAGE_IS_DENSE;

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
        // Dense is signature-excluded (mirrors `&mut T`): the mask never carries
        // its bit, so a dense include must NOT gate at the archetype level. Per-row
        // membership is the `e2s` oracle (D4).
        if const { T::STORAGE_IS_DENSE } {
            return true;
        }
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        // Dense contributes NO include bit (signature-excluded); candidates are
        // seeded from `arch_presence` (D4 seed wiring, via `dense_include_candidates`).
        if const { T::STORAGE_IS_DENSE } {
            return;
        }
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
            dense: std::ptr::null(),
            entity_ids: std::ptr::null(),
            _marker: PhantomData,
        }
    }

    #[inline]
    unsafe fn resolve_dense<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
    ) {
        // Gated by `const { Self::HAS_DENSE }` at the cursor — never emitted for a
        // TABLE `T` (the 0%-gate). For a dense `T`, cache the global `DenseStore`
        // pointer (the write-through target's owner; its tick sub-regions back the
        // deref guard's changed-tick bump).
        // SAFETY (resolve_dense contract): `world` upholds the access contract; the
        //   `DenseStore` lives in the address-stable `DenseRegistry` slot array,
        //   valid for `'w`; the pointer is confined to the `'w`-scoped `Fetch`.
        let registry = unsafe { world.world().dense_registry() };
        fetch.dense = match registry.store(state.id) {
            Some(store) => store as *const _,
            None => std::ptr::null(),
        };
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
        if const { T::STORAGE_IS_DENSE } {
            // Dense `T`: no archetype column / tick column. Cache `entity_ids` for
            // the per-row gather; the write target + tick sub-regions come from the
            // resolved `DenseStore` in `fetch` (D4). The deref guard's changed-tick
            // bump and `is_added`/`is_changed` read the dense column's per-slot
            // ticks, indexed by SLOT (not row).
            // SAFETY (QD3): `archetype` is live for `'w`; reading the immutable
            //   `entity_ids` slice base needs only shared access.
            let arch_ref: &Archetype = unsafe { &*archetype };
            fetch.entity_ids = arch_ref.entity_ids_slice().as_ptr();
            fetch.last_run = meta.last_run();
            fetch.this_run = meta.this_run();
            return;
        }
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
    unsafe fn dense_row_passes<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
        if const { T::STORAGE_IS_DENSE } {
            // SAFETY (D4): `entity_ids` cached by `set_table_mut`; `row <
            //   entity_count`. NULL `dense` ⟹ no member ⟹ skip.
            if fetch.dense.is_null() {
                return false;
            }
            let entity = unsafe { *fetch.entity_ids.add(row) };
            let store = unsafe { &*fetch.dense };
            return store.slot_of(entity).is_some();
        }
        true
    }

    #[inline]
    fn dense_include_candidates(
        state: &Self::State,
        registry: &crate::ecs::core::component::dense::DenseRegistry,
        out: &mut crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet,
    ) {
        // See `&mut T::dense_include_candidates` — OR-in the dense store's
        // `arch_presence`. Gated by `const { Self::HAS_DENSE_INCLUDE }`.
        if const { T::STORAGE_IS_DENSE }
            && let Some(store) = registry.store(state.id)
        {
            store.arch_presence().for_each_set_bit(|a| out.insert(a));
        }
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        if const { T::STORAGE_IS_DENSE } {
            // Dense `Mut`: row → entity → slot → row_ptr + per-slot tick pointers.
            // `dense_row_passes` proved the slot is live before this call. The
            // returned `Mut` carries the slot's `changed_tick` pointer (into the
            // dense column's changed sub-region) so `deref_mut` / `set_if_neq` bump
            // the dense slot's changed tick exactly like the archetypal path.
            // SAFETY (D4): `entity_ids`/`dense` cached; `row < entity_count`; the
            //   entity is a live member, so `slot < column.count()` and
            //   `row_ptr(slot)` is a live, stride-aligned WRITE-capable pointer into
            //   the address-stable column. The dense conflict node (Decision 6) +
            //   the `&mut`-cursor discipline serialise the column, so no other writer
            //   aliases this slot. `added_ticks_ptr`/`changed_ticks_ptr` are the
            //   column's address-stable per-slot tick bases; `slot < count <=
            //   committed_rows`, so `[slot]` is in the committed prefix. `Tick` is
            //   `Copy`. `'w` ties to the world borrow.
            unsafe {
                let entity = *fetch.entity_ids.add(row);
                let store = &*fetch.dense;
                let slot = store
                    .slot_of(entity)
                    .expect("invariant: dense_row_passes proved the slot is live")
                    as usize;
                let ptr = store.solve_view().row_ptr(slot);
                let value = &mut *(ptr as *mut T);
                let added = *(*store.added_ticks_ptr().add(slot)).get();
                let changed_tick = store.changed_ticks_ptr().add(slot);
                return Mut {
                    value,
                    added,
                    changed_tick,
                    last_run: fetch.last_run,
                    this_run: fetch.this_run,
                    deref_mut_called: false,
                };
            }
        }
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

// ── Option<D> (task #9 — non-filtering optional data) ──────────────────────
//
// `Option<D>` yields `Some(D::Item)` for archetypes that contain `D`'s
// component(s) and `None` for those that do not — WITHOUT filtering the
// archetype out. This is the inverse of every leaf: `matches_component_set`
// is unconditionally `true` (the archetype is admitted either way), and the
// per-archetype `set_table_*` GATES the inner forward on whether the inner
// `D` actually matches (Decision 1).

/// Per-archetype fetch scratch for `Option<D>: QueryData`.
///
/// Holds the inner `D::Fetch<'w>` plus a `matches` flag computed in
/// `set_table_*` (`true` iff the active archetype contains `D`'s
/// component(s)). When `matches` is `false`, `inner` stays at its
/// `D::init_fetch` NULL-init value and is NEVER read (`fetch` returns `None`).
///
/// `Copy` / `Clone` are implemented manually so the auto-derive does not
/// require `D::Fetch<'w>: Copy` via an unwanted blanket bound (it already is
/// `Copy` per the `QueryData::Fetch: Copy` bound, but the manual impls mirror
/// `ReadFetch` and keep the derive heuristics out of the picture).
pub struct OptionFetch<'w, D: QueryData> {
    /// Inner fetch. Valid only when `matches` is `true`; otherwise the
    /// NULL-init value from `D::init_fetch` (never dereferenced).
    pub(crate) inner: D::Fetch<'w>,
    /// `true` iff the active archetype contains `D`'s component(s).
    pub(crate) matches: bool,
}

impl<D: QueryData> Clone for OptionFetch<'_, D> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<D: QueryData> Copy for OptionFetch<'_, D> {}

// SAFETY (QD1-QD4):
//   - QD1: `init_access` forwards to `D::init_access`, declaring `D`'s exact
//     read/write surface (Decision 8 — conservative, correct; still trips
//     B0002 for `(&mut A, Option<&A>)`).
//   - QD2: `init_fetch` produces `(D::init_fetch (NULL), matches = false)`;
//     `set_table_*` overwrites `matches` and, when `matches == true`, the
//     inner via the gated forward, before any `fetch` call.
//   - QD3: `OptionFetch<'w, D>` carries `D::Fetch<'w>`, so the inner's
//     lifetime invariants ride `'w`.
//   - QD4: each `set_table_*` variant forwards to the matching inner variant
//     (readonly→readonly, mut→mut, no_meta→no_meta); the inner's own QD4
//     backstop-panic is preserved (NEVER gated away — only the FORWARD is
//     gated on `matches`).
unsafe impl<D: QueryData> QueryData for Option<D> {
    type State = D::State;
    type Fetch<'w> = OptionFetch<'w, D>;
    type Item<'w> = Option<D::Item<'w>>;

    const IS_READ_ONLY: bool = D::IS_READ_ONLY;
    // The inner participates in change detection iff `D` does; the dispatcher
    // routes via the same `if const { D::NCD || F::NCD }` const-fold.
    const NEEDS_CHANGE_DETECTION: bool = D::NEEDS_CHANGE_DETECTION;
    // Non-filtering: `Option<D>` contributes NO positive include bit (it never
    // requires `D`'s component present), so it is NOT a bounding data
    // component for an `Enabled`/`Disabled` term.
    const HAS_DATA_COMPONENT: bool = false;
    // `matches_component_set` is unconditionally `true` ⇒ nothing to trim.
    const REQUIRES_POST_FILTER_TRIM: bool = false;
    // Dense plan D3: forward the inner's dense-ness so the cursor resolves the
    // inner's `DenseStore` pointer (otherwise `Option<&Dense>`'s inner `fetch`
    // would deref a NULL store). W1 (None-on-absence): `Option<&Dense>` yields
    // `Some(&val)` for a present member and `None` for an absent one — the
    // correct `Option` semantics. The per-row membership is the inner's
    // `dense_row_passes` (≡ `slot_of(entity).is_some()`), checked inside
    // `fetch` (NOT via `Self::dense_row_passes`, which stays the default `true`
    // so `Option` never SKIPS a row — it maps an absent member to `None`).
    const HAS_DENSE: bool = D::HAS_DENSE;

    #[inline]
    fn init_state(world: &mut EcsMaster) -> Self::State {
        D::init_state(world)
    }

    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        // Decision 8: forward — declares `D`'s read/write surface so
        // `(&mut A, Option<&A>)` and `AnyOf<(&mut A, …)>` still trip B0002.
        D::init_access(state, access_set);
    }

    #[inline]
    fn matches_component_set(_state: &Self::State, _mask: &ComponentMask) -> bool {
        // Non-filtering: the archetype is admitted whether or not it contains
        // `D`. The per-archetype `matches` flag (computed in `set_table_*`)
        // decides Some vs None per row, NOT archetype membership.
        true
    }

    #[inline]
    fn aggregate_include(_state: &Self::State, _include: &mut ComponentMask) {
        // No-op: `Option<D>` adds no required bit (Decision: do NOT populate
        // `include` — that would WRONGLY require `D`'s component present).
    }

    #[inline]
    fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
        OptionFetch {
            inner: D::init_fetch(state),
            matches: false,
        }
    }

    #[inline]
    unsafe fn resolve_dense<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
    ) {
        // Dense plan D3: forward into the inner so its `DenseStore` pointer is
        // resolved (gated internally by `const { D::HAS_DENSE }`).
        // SAFETY (D3): the `world` cell is `Copy`, forwarded by value to
        //   preserve provenance; the inner gates its body on its own dense-ness.
        unsafe { D::resolve_dense(&mut fetch.inner, state, world); }
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        meta: &'_ SystemMeta,
    ) {
        // SAFETY (Decision 1, QD3, QD4): `archetype` is a live `*const
        //   Archetype` for `'w` (caller contract). `matches` re-derives the
        //   inner predicate from the archetype's own mask. When `matches ==
        //   true`, `D::matches_component_set` held ⇒ every column `D` reads is
        //   non-null ⇒ the forwarded `D::set_table_readonly`'s QD1/QD3 + its
        //   internal `debug_assert!(!ptr.is_null())` hold; the inner's QD4
        //   readonly backstop-panic (for a write-inner) is reached only if a
        //   custom impl falsely claimed `ReadOnlyQueryData for Option<&mut T>`
        //   — preserved verbatim. When `matches == false`, the forward is
        //   skipped and `fetch.inner` stays at its NULL-init value, NEVER read
        //   (`fetch` returns `None`).
        let matches = D::matches_component_set(state, unsafe { (*archetype).component_mask() });
        if matches {
            unsafe { D::set_table_readonly(&mut fetch.inner, state, archetype, meta) };
        }
        fetch.matches = matches;
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,
    ) {
        // SAFETY (Decision 1, QD1, QD3, QD4): same gate as
        //   `set_table_readonly` with the strictly-stronger caller guarantee
        //   that `archetype` carries write-capable provenance. When `matches
        //   == true`, the forwarded `D::set_table_mut` consumes that
        //   provenance for `D`'s columns. When `false`, the inner stays
        //   NULL-init and is never read. The mask read uses a shared reborrow
        //   (`component_mask` needs no write provenance).
        let matches =
            D::matches_component_set(state, unsafe { (*archetype).component_mask() });
        if matches {
            unsafe { D::set_table_mut(&mut fetch.inner, state, archetype, meta) };
        }
        fetch.matches = matches;
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    ) {
        // SAFETY (Decision 1/2, QD3, QD4): identical gate to
        //   `set_table_readonly` minus the unused `meta`. For an NCD=false
        //   inner (`&T`) this forwards to the inner's real meta-free body
        //   (Decision 2 row 1). For an NCD=true inner (`Ref<T>`) the forward
        //   reaches the inner's `#[cold]` no-meta panic — UNREACHABLE, because
        //   `Option<Ref<T>>::NCD = true` routes the driver
        //   (iter.rs:298 `if const { D::NCD || F::NCD }`) to the meta path
        //   (Decision 2 note b). The inner's QD4 readonly backstop on a
        //   write-inner is preserved verbatim.
        let matches = D::matches_component_set(state, unsafe { (*archetype).component_mask() });
        if matches {
            unsafe { D::set_table_readonly_no_meta(&mut fetch.inner, state, archetype) };
        }
        fetch.matches = matches;
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // SAFETY (Decision 1/2, QD1, QD3, QD4): same gate, write-capable
        //   `archetype`, meta-free. For an NCD=false inner (`&mut T`) this
        //   forwards to the inner's real meta-free body (Decision 2 row 2).
        //   For an NCD=true inner (`Mut<T>`) the forward reaches the inner's
        //   `#[cold]` no-meta panic — UNREACHABLE because `Option<Mut<T>>::NCD
        //   = true` routes through the meta path.
        let matches =
            D::matches_component_set(state, unsafe { (*archetype).component_mask() });
        if matches {
            unsafe { D::set_table_mut_no_meta(&mut fetch.inner, state, archetype) };
        }
        fetch.matches = matches;
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        // SAFETY (Decision 1, QD2, QD3): when `fetch.matches`, the inner was
        //   initialised by the gated `set_table_*` forward (caller called
        //   `set_table_*` before any `fetch`), so `D::fetch`'s contract holds
        //   (`row < entity_count`, inner bases non-null). When `!fetch.matches`
        //   the inner is the NULL-init value — NOT read here (we return
        //   `None`).
        if fetch.matches {
            // Dense plan D3 (W1 — None-on-absence): for a dense inner, the
            // archetype-level `matches` is unconditionally `true` (a dense
            // component is signature-excluded). The REAL membership oracle is
            // the per-row dense slot lookup. Gate the inner forward on the
            // inner's own `dense_row_passes` (≡ `slot_of(entity).is_some()`):
            // an entity that lacks the dense member yields `None`, the correct
            // `Option` semantics. For a table inner this const-folds OUT — the
            // unconditional `Some(D::fetch)` path is restored (0%-gate).
            if const { D::HAS_DENSE } {
                // SAFETY (D3): the inner's `set_table_*` forward cached
                //   `entity_ids` and `resolve_dense` cached the `DenseStore`
                //   pointer (both run for `matches == true` before any
                //   `fetch`); `row < entity_count` (caller contract). A NULL
                //   store / absent slot ⟹ `dense_row_passes` returns `false`
                //   ⟹ we never call `D::fetch` (no NULL/missing-slot deref).
                if unsafe { D::dense_row_passes(&fetch.inner, row) } {
                    Some(unsafe { D::fetch(&fetch.inner, row) })
                } else {
                    None
                }
            } else {
                Some(unsafe { D::fetch(&fetch.inner, row) })
            }
        } else {
            None
        }
    }
}

// SAFETY: `Option<D>` performs no writes when `D` does not — `IS_READ_ONLY =
//   D::IS_READ_ONLY`, and `D: ReadOnlyQueryData` guarantees `D::IS_READ_ONLY
//   = true`. So `Option<&mut T>` / `Option<Mut<T>>` are rejected from
//   `iter()` / `par_iter()` (only `iter_mut()` admits them).
unsafe impl<D: ReadOnlyQueryData> ReadOnlyQueryData for Option<D> {}

// ── AnyOf<(D0, D1, …)> (task #9 — OR over real-component leaves) ────────────
//
// `AnyOf<(D0, …, Dn)>` yields a tuple `(Option<D0::Item>, …)` where at least
// one element is `Some`. It is the OR analogue of a data tuple's AND.
//
// Cost note (Decision 8): a SOLE `Query<AnyOf<(&A, &B)>>` has an EMPTY include
// mask ⇒ `update_archetypes` matches EVERY live archetype, then
// `post_filter_matched` trims to those containing (A or B). This is the
// `Or<F>` cost profile — paid per generation bump (per `update`), NOT per
// `iter()`. A `Query<(&A, AnyOf<(&B, &C)>)>` is bounded by `&A`'s include and
// pays no full-world scan. The full-world-scan cost scales with archetype
// count; do not mistake it for a bug (filed as an archetype-count-scaling
// bench note).

/// Sealed marker for the leaf types admissible as an [`AnyOf`] arm.
///
/// `AnyOf<(D0, …)>` bounds every arm `Di: AnyOfArm`. The seal compile-rejects
/// arms whose `matches_component_set` is not a single-component predicate —
/// `Option<_>` (unconditionally `true`), `()` (unconditionally `true`), nested
/// `AnyOf`, and tuple arms — every one of which would break the OR's ≥1-member
/// trim by matching the whole world (Decision 3). Mirrors the sealed
/// `OrComposable` bound (`filter.rs`).
///
/// # Members
///
/// `&T`, `&mut T`, [`Ref<'_, T>`](Ref), [`Mut<'_, T>`](Mut) for any
/// `T: Component`. NOT members: `Option<_>`, `()`, `AnyOf<_>`, tuples.
///
/// # Safety
///
/// A purely declarative marker — no method contract. `unsafe` signals that
/// membership is a deliberate, audited choice: an `AnyOfArm` must have a
/// single-component `matches_component_set` so the OR-trim is well-defined.
pub unsafe trait AnyOfArm: QueryData {}

// SAFETY: `&T::matches_component_set` is `mask.contains(id)` — a single
//   real-component predicate; the OR-trim over arms is well-defined.
unsafe impl<T: Component> AnyOfArm for &T {}

// SAFETY: `&mut T::matches_component_set` is `mask.contains(id)` — same.
unsafe impl<T: Component> AnyOfArm for &mut T {}

// SAFETY: `Ref<T>::matches_component_set` is `mask.contains(id)` — same.
unsafe impl<T: Component> AnyOfArm for Ref<'_, T> {}

// SAFETY: `Mut<T>::matches_component_set` is `mask.contains(id)` — same.
unsafe impl<T: Component> AnyOfArm for Mut<'_, T> {}

/// OR-combinator query data: yields `(Option<D0::Item>, …)` with the ≥1-member
/// guarantee (at least one arm is `Some` for every yielded row).
///
/// Every arm must be an [`AnyOfArm`] (a real-component leaf: `&T`, `&mut T`,
/// `Ref<T>`, `Mut<T>`). `Option`, `()`, nested `AnyOf`, and tuple arms are
/// compile-rejected (Decision 3). An empty `AnyOf<()>` has no impl ⇒
/// trait-not-satisfied compile error (Decision 7).
///
/// # Semantics
///
/// * `AnyOf<(&A, &B)>` → `(Option<&A>, Option<&B>)`, matched against
///   archetypes containing A OR B; at least one is `Some` per row.
/// * `AnyOf<(&A,)>` single arm → `(Option<&A>,)`, bounded to A-present
///   archetypes (always `Some`) — NOT equivalent to `&A` (the item is a
///   1-tuple of `Option`).
/// * `AnyOf<(&A, &A)>` overlapping read+read → legal.
/// * `AnyOf<(&mut A, &A)>` / `(&mut A, &mut A)` → trips the B0002 aliasing
///   detector (`init_access` forwards each arm).
///
/// # Cost
///
/// A SOLE `Query<AnyOf<…>>` scans the full archetype set on every generation
/// bump (empty include ⇒ the `Or<F>` cost profile) — see the module note
/// above. Bound it with a positive term (`Query<(&A, AnyOf<…>)>`) when
/// possible.
pub struct AnyOf<T>(PhantomData<fn() -> T>);

/// Emits a `QueryData` impl for `AnyOf<(D0, …)>` over the given paired idents.
/// Each arm is bounded `$D: AnyOfArm` (the seal — Decision 3). Mirrors
/// [`impl_query_data_tuple!`]'s `(TypeIdent, state_ident, fetch_ident)`
/// triples; the `bool` flag rides alongside each arm's `Fetch` as
/// `($D::Fetch<'w>, bool)`.
macro_rules! impl_any_of {
    ( $( ($D:ident, $s:ident, $f:ident) ),+ ) => {
        // SAFETY (QD1-QD4): each arm forwards to its own `QueryData` impl
        //   (QD1-QD4 by induction). Per-arm `set_table` is GATED on that
        //   arm's own `matches` (Decision 1) — never an unconditional
        //   forward. `archetype` is identical for every arm in one call.
        //   Intra-`AnyOf` aliasing among arms is detected at `init_access`
        //   via `FilteredAccessSet` (Decision 8).
        #[allow(non_snake_case)]
        unsafe impl< $($D: AnyOfArm),+ > QueryData for AnyOf<( $($D,)+ )> {
            type State = ( $($D::State,)+ );
            type Fetch<'w> = ( $(($D::Fetch<'w>, bool),)+ );
            type Item<'w> = ( $(Option<$D::Item<'w>>,)+ );

            const IS_READ_ONLY: bool = true $( && $D::IS_READ_ONLY )+;
            const NEEDS_CHANGE_DETECTION: bool = false $( || $D::NEEDS_CHANGE_DETECTION )+;
            // Non-filtering at the archetype level (the OR-trim lives in
            // `post_filter_matched`, not in a positive include bit).
            const HAS_DATA_COMPONENT: bool = false;
            // The ≥1-member OR-trim runs in the post-filter pass — so
            // `Query<AnyOf<…>, Enabled<C>>` must NOT be candidate-seeded
            // (Decision 4).
            const REQUIRES_POST_FILTER_TRIM: bool = true;

            // Dense plan D3: an `AnyOf` arm may be a dense leaf; OR-fold so the
            // cursor resolves each dense arm's `DenseStore` pointer (avoids a
            // NULL-store deref in the arm's `fetch`). `dense_row_passes` is NOT
            // forwarded (AnyOf's ≥1-member semantics keep the default `true` —
            // a row missing one OR-arm still yields `(…, None, …)`, never a
            // skip). W1 (None-on-absence): a present dense arm yields its value;
            // an absent dense arm yields `None` (per-row membership via the
            // arm's `dense_row_passes`, checked inside `fetch`).
            const HAS_DENSE: bool = false $( || $D::HAS_DENSE )+;

            #[inline]
            fn init_state(world: &mut EcsMaster) -> Self::State {
                ( $( <$D as QueryData>::init_state(world), )+ )
            }

            #[inline]
            fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
                let ( $($s,)+ ) = state;
                // Decision 8: forward each arm — declares the full read/write
                // surface so `AnyOf<(&mut A, &A)>` trips B0002.
                $( <$D as QueryData>::init_access($s, access_set); )+
            }

            #[inline]
            unsafe fn resolve_dense<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
            ) {
                let ( $($f,)+ ) = fetch;
                let ( $($s,)+ ) = state;
                $(
                    // SAFETY (D3): each arm gates its body on its own
                    //   `const { $D::HAS_DENSE }`; the `world` cell is `Copy`,
                    //   forwarded by value to preserve provenance. `$f.0` is the
                    //   arm's inner `Fetch`.
                    unsafe { <$D as QueryData>::resolve_dense(&mut $f.0, $s, world); }
                )+
            }

            #[inline]
            fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
                let ( $($s,)+ ) = state;
                // OR of arms (the ≥1-member predicate).
                false $( || <$D as QueryData>::matches_component_set($s, mask) )+
            }

            #[inline]
            fn aggregate_include(_state: &Self::State, _include: &mut ComponentMask) {
                // No-op: AnyOf's OR predicate has NO common required bit; the
                // membership trim is applied in `post_filter_matched` via
                // `matches_component_set`. Populating `include` would WRONGLY
                // require ALL arms present (an AND, not an OR).
            }

            #[inline]
            fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
                let ( $($s,)+ ) = state;
                ( $( (<$D as QueryData>::init_fetch($s), false), )+ )
            }

            #[inline]
            unsafe fn set_table_readonly<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *const Archetype,
                meta: &'_ SystemMeta,
            ) {
                let ( $($f,)+ ) = fetch;
                let ( $($s,)+ ) = state;
                $(
                    // SAFETY (Decision 1, QD3, QD4): per-arm gate. The mask
                    //   read uses a shared reborrow; `matches` re-derives the
                    //   arm's predicate. When `true`, the forwarded
                    //   `set_table_readonly`'s QD1/QD3 hold (arm's columns
                    //   non-null); the arm's QD4 readonly backstop on a
                    //   write-arm is preserved (unreachable in well-typed
                    //   `iter()` code). When `false`, the arm's inner stays
                    //   NULL-init and is never read.
                    {
                        let m = <$D as QueryData>::matches_component_set(
                            $s,
                            unsafe { (*archetype).component_mask() },
                        );
                        if m {
                            unsafe {
                                <$D as QueryData>::set_table_readonly(
                                    &mut $f.0, $s, archetype, meta,
                                );
                            }
                        }
                        $f.1 = m;
                    }
                )+
            }

            #[inline]
            unsafe fn set_table_mut<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *mut Archetype,
                meta: &'_ SystemMeta,
            ) {
                let ( $($f,)+ ) = fetch;
                let ( $($s,)+ ) = state;
                $(
                    // SAFETY (Decision 1, QD1, QD3, QD4): per-arm gate with
                    //   write-capable `archetype`. When `true`, the forwarded
                    //   `set_table_mut` consumes that arm's write provenance.
                    {
                        let m = <$D as QueryData>::matches_component_set(
                            $s,
                            unsafe { (*archetype).component_mask() },
                        );
                        if m {
                            unsafe {
                                <$D as QueryData>::set_table_mut(
                                    &mut $f.0, $s, archetype, meta,
                                );
                            }
                        }
                        $f.1 = m;
                    }
                )+
            }

            #[inline]
            unsafe fn set_table_readonly_no_meta<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *const Archetype,
            ) {
                let ( $($f,)+ ) = fetch;
                let ( $($s,)+ ) = state;
                $(
                    // SAFETY (Decision 1/2, QD3, QD4): per-arm gate, meta-free.
                    //   Reached only when no arm needs change detection
                    //   (`AnyOf::NCD == false` ⇒ every arm's NCD == false ⇒
                    //   every arm's `_no_meta` is its real meta-free body, not
                    //   the cold panic).
                    {
                        let m = <$D as QueryData>::matches_component_set(
                            $s,
                            unsafe { (*archetype).component_mask() },
                        );
                        if m {
                            unsafe {
                                <$D as QueryData>::set_table_readonly_no_meta(
                                    &mut $f.0, $s, archetype,
                                );
                            }
                        }
                        $f.1 = m;
                    }
                )+
            }

            #[inline]
            unsafe fn set_table_mut_no_meta<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *mut Archetype,
            ) {
                let ( $($f,)+ ) = fetch;
                let ( $($s,)+ ) = state;
                $(
                    // SAFETY (Decision 1/2, QD1, QD3, QD4): per-arm gate,
                    //   write-capable, meta-free. Same NCD-propagation note as
                    //   the readonly variant.
                    {
                        let m = <$D as QueryData>::matches_component_set(
                            $s,
                            unsafe { (*archetype).component_mask() },
                        );
                        if m {
                            unsafe {
                                <$D as QueryData>::set_table_mut_no_meta(
                                    &mut $f.0, $s, archetype,
                                );
                            }
                        }
                        $f.1 = m;
                    }
                )+
            }

            #[inline]
            unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
                let ( $($f,)+ ) = fetch;
                (
                    $(
                        // SAFETY (Decision 1, QD2, QD3): when the arm's flag is
                        //   set, its inner was initialised by the gated
                        //   `set_table_*` forward and `row` is in range; when
                        //   clear, the inner is NULL-init and not read.
                        // Dense plan D3 (W1 — None-on-absence): a dense arm's
                        //   `$f.1` is archetype-level (always `true` — dense is
                        //   signature-excluded), so the REAL per-row membership
                        //   is the arm's `dense_row_passes` (≡ `slot_of(...)`).
                        //   An entity lacking the dense member yields `None` for
                        //   that arm; `AnyOf` still matches the row iff any arm
                        //   is `Some` (the post-filter trim already admitted the
                        //   row). For a table arm this const-folds OUT (0%-gate).
                        if $f.1 {
                            if const { <$D as QueryData>::HAS_DENSE } {
                                // SAFETY (D3): the arm's `set_table_*` forward
                                //   cached `entity_ids` and `resolve_dense` the
                                //   `DenseStore` pointer (both run for `$f.1 ==
                                //   true` before any `fetch`); `row` in range.
                                //   An absent slot ⟹ `dense_row_passes` is
                                //   `false` ⟹ `$D::fetch` is never called.
                                if unsafe { <$D as QueryData>::dense_row_passes(&$f.0, row) } {
                                    Some(unsafe { <$D as QueryData>::fetch(&$f.0, row) })
                                } else {
                                    None
                                }
                            } else {
                                Some(unsafe { <$D as QueryData>::fetch(&$f.0, row) })
                            }
                        } else {
                            None
                        },
                    )+
                )
            }
        }
    };
}

/// Emits a `ReadOnlyQueryData` impl for `AnyOf<(D0, …)>` — read-only iff every
/// arm is. Gated separately from [`impl_any_of!`] so the bound is
/// `$D: AnyOfArm + ReadOnlyQueryData` without conflating the two at the
/// working impl's header.
macro_rules! impl_any_of_read_only {
    ( $( $D:ident ),+ ) => {
        // SAFETY: every arm is `ReadOnlyQueryData` (each arm's
        //   `IS_READ_ONLY = true`); `AnyOf`'s per-arm fetch forwards to
        //   read-only arm fetches by induction.
        unsafe impl< $($D: AnyOfArm + ReadOnlyQueryData),+ > ReadOnlyQueryData
            for AnyOf<( $($D,)+ )> {}
    };
}

impl_any_of!((D0, s0, f0));
impl_any_of!((D0, s0, f0), (D1, s1, f1));
impl_any_of!((D0, s0, f0), (D1, s1, f1), (D2, s2, f2));
impl_any_of!((D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3));
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11)
);

impl_any_of_read_only!(D0);
impl_any_of_read_only!(D0, D1);
impl_any_of_read_only!(D0, D1, D2);
impl_any_of_read_only!(D0, D1, D2, D3);
impl_any_of_read_only!(D0, D1, D2, D3, D4);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5, D6);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5, D6, D7);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5, D6, D7, D8);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5, D6, D7, D8, D9);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5, D6, D7, D8, D9, D10);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5, D6, D7, D8, D9, D10, D11);

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

            // EnableTag C2: a tuple touches a data component iff ANY element
            // does (OR-fold) — bounds an enable term's matched set.
            const HAS_DATA_COMPONENT: bool = false $( || $D::HAS_DATA_COMPONENT )*;

            // Task #9 M1: a tuple requires post-filter trim iff ANY element
            // does (OR-fold). Without this, a tuple wrapping `AnyOf<..>` falls
            // back to the trait default `false`, which re-seeds the candidate
            // path and skips `post_filter_matched` — AnyOf's >=1-member OR-trim
            // never runs, yielding phantom `(None,)` rows.
            const REQUIRES_POST_FILTER_TRIM: bool = false $( || $D::REQUIRES_POST_FILTER_TRIM )*;

            // Dense plan D3: a tuple touches dense storage iff ANY element does
            // (OR-fold). Drives the cursor's gated `resolve_dense` /
            // `dense_row_passes` forwarders; `false` for an all-table tuple
            // (the 0%-gate — the forwarders below const-fold to no-ops).
            const HAS_DENSE: bool = false $( || $D::HAS_DENSE )*;
            // Dense plan D3: a tuple has a dense INCLUDE term iff ANY element
            // does (OR-fold) — drives the candidate seed.
            const HAS_DENSE_INCLUDE: bool = false $( || $D::HAS_DENSE_INCLUDE )*;

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
            unsafe fn resolve_dense<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (D3): each element gates its own body on
                    //   `const { $D::HAS_DENSE }` (a non-dense element's
                    //   `resolve_dense` is the empty default — folds out). The
                    //   `world` cell is `Copy`, forwarded by value to preserve
                    //   provenance.
                    unsafe { <$D as QueryData>::resolve_dense($f, $s, world); }
                )*
            }

            #[inline]
            unsafe fn dense_row_passes<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
                let ( $($f,)* ) = fetch;
                // AND over elements: every REQUIRED dense term must have the
                // row's entity in its store. A non-dense element's
                // `dense_row_passes` is the const-`true` default (folds out).
                // SAFETY (D3): per-element contract — `fetch`/`row` were set up
                //   by `resolve_dense` + `set_table_*`; `row < entity_count`.
                true $( && unsafe { <$D as QueryData>::dense_row_passes($f, row) } )*
            }

            #[inline]
            fn dense_include_candidates(
                state: &Self::State,
                registry: &crate::ecs::core::component::dense::DenseRegistry,
                out: &mut crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet,
            ) {
                let ( $($s,)* ) = state;
                // Each element ORs its own dense-include candidates (gated by its
                // own `HAS_DENSE_INCLUDE`). The UNION over a tuple of dense terms
                // is conservative (false positives trimmed per-row); the exact
                // AND-membership is the per-row `dense_row_passes`.
                $( <$D as QueryData>::dense_include_candidates($s, registry, out); )*
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
    // EnableTag C2: `()` touches no data component (no positive include bit).
    const HAS_DATA_COMPONENT: bool = false;

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
            const HAS_DATA_COMPONENT: bool = false;

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
