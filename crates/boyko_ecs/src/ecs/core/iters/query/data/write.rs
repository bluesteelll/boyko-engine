//! Split from `data.rs` (mechanical move; see `super` for the shared
//! `QueryData` / `ReadOnlyQueryData` traits and imports).

use super::*;

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

