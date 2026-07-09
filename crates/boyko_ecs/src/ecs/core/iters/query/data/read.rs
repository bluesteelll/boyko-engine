//! Split from `data.rs` (mechanical move; see `super` for the shared
//! `QueryData` / `ReadOnlyQueryData` traits and imports).

use super::*;

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

