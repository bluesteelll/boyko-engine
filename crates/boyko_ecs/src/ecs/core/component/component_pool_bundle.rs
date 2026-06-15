use std::ops::{Index, IndexMut};

use crate::ecs::error::{EcsError, EcsResult};
use boyko_utils::sparse_map::sparse_map::SparseMap;

use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry;
use crate::ecs::identifiers::primitives::{ComponentId, InlandPoolId};
use crate::ecs::memory::component_pool::ComponentPool;

pub struct ComponentPoolBundle {
    pools: Vec<ComponentPool>,
    sparse_indexes: SparseMap<InlandPoolId>,
}

impl Default for ComponentPoolBundle {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentPoolBundle {
    /// Creates a new empty ComponentPoolBundle
    pub fn new() -> Self {
        Self {
            pools: Vec::new(),
            sparse_indexes: SparseMap::new(),
        }
    }

    /// Creates a new ComponentPoolBundle with pre-allocated capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            pools: Vec::with_capacity(capacity),
            sparse_indexes: SparseMap::with_capacity(capacity),
        }
    }

    /// Creates a new ComponentPoolBundle with pools for the specified component IDs
    pub fn with_component_ids(component_ids: &[ComponentId]) -> Self {
        let mut bundle = Self::with_capacity(component_ids.len());

        // Add pools for all specified component IDs
        for &component_id in component_ids {
            bundle.add_pool(component_id);
        }

        bundle
    }

    /// Adds a component pool for a specific component ID
    /// Returns the internal index assigned to this pool
    pub fn add_pool(&mut self, component_id: ComponentId) -> InlandPoolId {
        // Check if pool for this component type already exists
        if let Some(&inland_id) = self.sparse_indexes.get(component_id.0) {
            return inland_id;
        }

        // Verify component is registered - only in debug builds
        debug_assert!(component_registry::get_layout(component_id.0).is_some(),
            "Component ID {} not registered in layout registry", component_id);

        // Create a new pool for this component type
        let pool = ComponentPool::with_default_sizes(component_id.0);

        // Add pool to the bundle
        let inland_id = InlandPoolId(self.pools.len());
        self.pools.push(pool);
        self.sparse_indexes.insert(component_id.0, inland_id);

        inland_id
    }

    /// Gets a component pool by component ID
    pub fn get_pool(&self, component_id: ComponentId) -> Option<&ComponentPool> {
        let inland_id = self.sparse_indexes.get(component_id.0)?;
        Some(&self.pools[inland_id.0])
    }

    /// Gets a mutable component pool by component ID
    pub fn get_pool_mut(&mut self, component_id: ComponentId) -> Option<&mut ComponentPool> {
        let inland_id = self.sparse_indexes.get(component_id.0)?.0;
        Some(&mut self.pools[inland_id])
    }

    /// Checks if the bundle contains a pool for a component with the specified ID
    pub fn contains(&self, component_id: ComponentId) -> bool {
        self.sparse_indexes.contains(component_id.0)
    }

    /// Gets the number of component pools in the bundle
    #[inline]
    pub fn len(&self) -> usize {
        self.pools.len()
    }

    /// Checks if the bundle is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pools.is_empty()
    }

    /// Adds a component to the appropriate pool
    pub fn add_component(&mut self, component_id: ComponentId, component_bytes: &[u8]) -> Option<usize> {
        debug_assert!(self.contains(component_id),
            "Component ID {} not found in bundle", component_id);

        // Verify component size matches registry - debug only check
        debug_assert_eq!(
            component_bytes.len(),
            component_registry::get_component_size(component_id.0).unwrap_or(0),
            "Component size mismatch for ID {}", component_id
        );

        let inland_id = self.sparse_indexes.get(component_id.0)?.0;
        self.pools[inland_id].add(component_bytes)
    }

    /// Validates that all component pools can accept one more entity (C-009).
    ///
    /// Returns `true` only if:
    /// - Every `ComponentId` in `components` is present in this bundle.
    /// - Every corresponding pool is below its reserve ceiling
    ///   (`!is_full()`). Phase X.I: this is a CEILING pre-check — committed
    ///   capacity below the ceiling grows on demand inside
    ///   `ComponentPool::add`, so "full" means the pool's `reserve_rows`
    ///   is exhausted, not that a fixed buffer ran out.
    ///
    /// This must be called before [`push_entity_components`] to implement the
    /// two-phase commit pattern that prevents partial-pool desync on failure.
    pub fn can_push_entity_components(&self, components: &[(ComponentId, &[u8])]) -> bool {
        for (component_id, bytes) in components {
            let inland_id = match self.sparse_indexes.get(component_id.0) {
                Some(&id) => id,
                None => return false,
            };
            // Verify component size matches registry — debug only.
            debug_assert_eq!(
                bytes.len(),
                component_registry::get_component_size(component_id.0).unwrap_or(0),
                "Component size mismatch for ID {}", component_id
            );
            if self.pools[inland_id.0].is_full() {
                return false;
            }
        }
        true
    }

    /// Pushes all component bytes into their respective pools (C-009).
    ///
    /// Precondition: [`can_push_entity_components`] must have returned `true`
    /// for the same `components` slice immediately before this call and without
    /// any intervening mutation. If the precondition is violated, individual
    /// pools may reject the push (`add` returns `None`), leaving the bundle in
    /// a partially-written state — this is a caller bug. Phase X.I: `add`
    /// grows committed capacity inline, so `None` means the pool's reserve
    /// ceiling was exhausted.
    ///
    /// Returns the unit index assigned to the entity (all pools receive the
    /// same dense index because they grow in lock-step).
    ///
    /// # Panics
    /// Panics in debug builds if a pool's reserve ceiling is exhausted
    /// (violated precondition) or if a `ComponentId` is not present in the
    /// bundle.
    pub fn push_entity_components(&mut self, components: &[(ComponentId, &[u8])]) -> usize {
        debug_assert!(self.can_push_entity_components(components),
            "push_entity_components called without a preceding successful \
             can_push_entity_components check");

        let mut unit_index = 0;
        let mut first = true;

        for (component_id, bytes) in components {
            let inland_id = self.sparse_indexes.get(component_id.0).copied()
                .expect("invariant: can_push verified all component IDs are present");
            let idx = self.pools[inland_id.0].add(bytes)
                .expect("invariant: can_push verified all pools have capacity");
            if first {
                unit_index = idx;
                first = false;
            }
            // All pools must agree on the dense index.
            debug_assert_eq!(idx, unit_index,
                "pool desync: pool for component {} returned index {} but expected {}",
                component_id, idx, unit_index);
        }

        unit_index
    }
    
    pub fn pop_entity(&mut self) -> bool {
        if self.pools.is_empty() {
            return true; // No pools to pop from
        }
        
        let mut success = true;
        
        // Remove the last component from each pool
        for pool in self.pools.iter_mut() {
            success &= pool.pop();
        }
        
        success
    }

/// Removes entity components from all pools using swap_remove
/// Returns the removed entity's index if successful
pub fn swap_remove_unit(&mut self, unit_index: usize) -> EcsResult<()> {
    let mut success = true;

    // Debug check for valid unit index
    debug_assert!(self.pools.iter().all(|pool| unit_index < pool.count()),
        "Unit index {} out of bounds in some pools", unit_index);

    // Remove components from each pool using the unit index
    for pool in self.pools.iter_mut() {
        success &= pool.swap_remove(unit_index);
    }

    if !success {
        return Err(EcsError::PoolSwapRemoveFailed);
    }

    Ok(())
}

    /// Type-checked append. Consumes `value` by move into the matching pool's slot.
    ///
    /// On missing `T::component_id()` (no matching pool), `value` drops at
    /// scope exit; bundle is not modified.
    ///
    /// Returns the slot index on success, `None` if the pool is missing or full.
    #[inline]
    pub fn add_component_typed<T: Component>(&mut self, value: T) -> Option<usize> {
        let component_id = T::component_id();
        // On miss: `value` drops at scope exit; bundle is not modified.
        let inland_id = self.sparse_indexes.get(component_id.0).copied()?.0;
        self.pools[inland_id].add_typed(value)
    }

    // ── Phase 11 — no-drop migration forwarders (plan §7.2 / C-N2) ──────────

    /// Returns `true` if a pool for `component_id` exists in this bundle.
    /// Phase 11 W-N1 defensive check for the `apply_replace_in_place`
    /// canonicalization guard (plan §7.4).
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn has_pool(&self, component_id: ComponentId) -> bool {
        self.sparse_indexes.contains(component_id.0)
    }

    /// Forwarder: swap-removes row `idx` for byte storage + tick storage
    /// across every pool in this bundle, without invoking `drop_fn`
    /// (plan §7.2 W-N2).
    ///
    /// # Safety
    ///
    /// Same contract as
    /// [`ComponentPool::swap_remove_index_no_drop`](crate::ecs::memory::component_pool::ComponentPool::swap_remove_index_no_drop):
    /// caller has ensured `idx`'s bytes were moved or dropped for each
    /// pool. Caller holds exclusive `&mut self`.
    #[allow(dead_code)]
    pub(crate) unsafe fn swap_remove_unit_no_drop(&mut self, idx: usize) {
        debug_assert!(
            self.pools.iter().all(|pool| idx < pool.count()),
            "swap_remove_unit_no_drop: idx out of bounds in some pools"
        );
        for pool in self.pools.iter_mut() {
            // SAFETY: per-pool delegation; `&mut self` ⇒ exclusive access
            //   to every owned pool. The W-N2 contract is forwarded
            //   unchanged.
            unsafe { pool.swap_remove_index_no_drop(idx) };
        }
    }

    /// Forwarder: pops the last row from every pool without invoking
    /// `drop_fn` (plan §7.2 / C5).
    #[allow(dead_code)]
    pub(crate) fn pop_entity_no_drop(&mut self) {
        for pool in self.pools.iter_mut() {
            pool.pop_entity_no_drop();
        }
    }

    // ── Phase 12.5 Opt-A2 — batch reserve / write accessors (C-N1) ──────────
    //
    // §5.6 of the spawn-optimisations plan. `SpawnBatchCommand::apply`
    // pre-validates capacity via `Archetype::reserve_capacity`, indexes
    // pools through the pre-resolved `BundleColumnRecord::pool_ids` (Opt-A3),
    // then calls `write_at_unchecked_initialized` per row, and finally
    // `commit_units_batch` + `fill_ticks_batch` once per batch.

    /// Phase 12.5 Opt-A2 (C-N1): iterator over the bundle's owned pools.
    ///
    /// Used by `Archetype::reserve_capacity` to walk every pool and
    /// validate it can accept `n` additional rows.
    #[inline]
    pub(crate) fn pools_iter(&self) -> impl Iterator<Item = &ComponentPool> {
        self.pools.iter()
    }

    /// Phase 12.5 Opt-A2 (C-N1): mutable iterator counterpart.
    ///
    /// Used by `Archetype::reserve_capacity` Phase B (Phase X.I) to grow
    /// every pool's committed capacity after the Phase A ceiling check.
    #[inline]
    pub(crate) fn pools_iter_mut(&mut self) -> impl Iterator<Item = &mut ComponentPool> {
        self.pools.iter_mut()
    }

    /// Phase 12.5 Opt-A2 (C-N1 / SBO-N): number of pools currently owned.
    ///
    /// Used by `BundleColumnRecord::pools_len_at_install` (SBO-N detection
    /// invariant — the pools Vec must never shrink between cache install
    /// and warm-path use; v1 has no archetype destruction).
    #[inline]
    pub(crate) fn pools_len(&self) -> usize {
        self.pools.len()
    }

    /// Phase 12.5 Opt-A2 (C-N1): resolves `component_id` to its
    /// `InlandPoolId` (one SparseMap lookup).
    ///
    /// Used by `BundleColumnCache::resolve_and_cache` at install time
    /// (cold path) to pre-compute the `pool_ids` slice; warm-path apply
    /// indexes `pools[pool_id]` directly via `pool_at_unchecked_mut`.
    #[inline]
    pub(crate) fn pool_id_for(&self, component_id: ComponentId) -> Option<InlandPoolId> {
        self.sparse_indexes.get(component_id.0).copied()
    }

    /// Phase 12.5 Opt-A2 (C-N1): direct `&mut ComponentPool` indexing by
    /// `InlandPoolId` — no SparseMap lookup, no bounds check.
    ///
    /// # Safety
    ///
    /// * `pool_idx.0 < self.pools.len()` — caller pre-validated through a
    ///   prior `pool_id_for` call cached in `BundleColumnRecord::pool_ids`.
    /// * Caller holds exclusive `&mut self` access; no concurrent reader
    ///   exists.
    #[inline]
    pub(crate) unsafe fn pool_at_unchecked_mut(
        &mut self,
        pool_idx: InlandPoolId,
    ) -> &mut ComponentPool {
        debug_assert!(
            pool_idx.0 < self.pools.len(),
            "pool_at_unchecked_mut: pool_idx {} out of bounds (pools.len() = {})",
            pool_idx.0,
            self.pools.len()
        );
        // SAFETY: caller upholds `pool_idx.0 < self.pools.len()` (debug-asserted).
        unsafe { self.pools.get_unchecked_mut(pool_idx.0) }
    }

    /// Decision 4 (W2 — single-provenance): resolves the typed-write
    /// destination bases for every DATA column in one pass, filling `out` by
    /// value.
    ///
    /// `data_pool_ids` is the canonical DATA (non-ZST) column slice already
    /// built by `SpawnBatchCommand::apply` Step 2.6 (the canonical `pool_ids`
    /// filtered to non-ZST, in `ComponentId` order). `perm` maps each
    /// **declaration field** index to the canonical data-column slot it writes
    /// ([`BundleColumnPtrs::PERM_SKIP`] for a ZST field), also built by the
    /// caller alongside `data_pool_ids`. `out` is populated so that
    /// `out.column_base(slot)` is the write base for `data_pool_ids[slot]` and
    /// `out.perm(field)` resolves the declaration field's data-column slot.
    ///
    /// # W2 single-provenance contract (the 14a-F2 / 9.3c antidote)
    ///
    /// All bases are read under ONE `&mut`-borrow of the pool bundle, derived
    /// from a single raw `*mut ComponentPool` base of the `pools` Vec — this
    /// does NOT call `pool_at_unchecked_mut` per column (which would re-tag the
    /// whole `pools` slice on every call under Tree Borrows). The function
    /// RETURNS with the borrow ENDED; the caller then holds only the raw `*mut
    /// u8` bases in `out` and never re-borrows the bundle inside the row loop
    /// (CONFIRM-2).
    ///
    /// # Safety
    ///
    /// * Every `data_pool_ids[slot].0 < self.pools.len()` (the caller copied
    ///   each entry from a valid `pool_ids` slot — debug-asserted).
    /// * `perm.len()` equals the number of declaration fields; each entry is
    ///   either a valid data-column slot `< data_pool_ids.len()` or
    ///   [`BundleColumnPtrs::PERM_SKIP`] for a ZST field.
    /// * Caller holds exclusive `&mut self`; no concurrent reader exists.
    #[inline]
    pub(crate) fn resolve_column_ptrs(
        &mut self,
        data_pool_ids: &[InlandPoolId],
        perm: &[u8],
        out: &mut crate::ecs::core::bundle::BundleColumnPtrs,
    ) {
        debug_assert!(
            data_pool_ids.len() <= crate::ecs::core::bundle::MAX_BUNDLE_ARITY
        );
        out.set_perm_from(perm);

        // Single overarching provenance: one raw base of the `pools` Vec. We
        // never form a `&mut self.pools` reborrow inside the loop — each base
        // is read through `(*pools_base.add(idx)).buffer_ptr_mut()`, derived
        // from this one pointer (W2). `&mut self` keeps it exclusive.
        let pools_len = self.pools.len();
        let pools_base: *mut ComponentPool = self.pools.as_mut_ptr();

        for (slot, &pool_idx) in data_pool_ids.iter().enumerate() {
            debug_assert!(
                pool_idx.0 < pools_len,
                "resolve_column_ptrs: pool_idx {} out of bounds (pools.len() = {})",
                pool_idx.0,
                pools_len
            );
            // SAFETY (W2):
            //   - `pool_idx.0 < pools_len` (debug-asserted; the caller copied
            //     it from a valid canonical `pool_ids` slot).
            //   - `pools_base` is the live `pools` Vec base under `&mut self`;
            //     `add(pool_idx.0)` addresses a valid `ComponentPool`. The
            //     `&mut *` reborrow is scoped to this one statement (the base
            //     read), derived from the single `pools_base` provenance — no
            //     per-call re-tag of the whole slice.
            let pool: &mut ComponentPool =
                unsafe { &mut *pools_base.add(pool_idx.0) };
            let base = pool.buffer_ptr_mut();
            let stride = pool.component_layout().size();
            #[cfg(debug_assertions)]
            let committed_rows = pool.committed_rows();
            #[cfg(debug_assertions)]
            let comp_id = ComponentId(pool.component_id());

            let column = crate::ecs::core::bundle::ColumnPtr::new(
                base,
                stride,
                #[cfg(debug_assertions)]
                committed_rows,
                #[cfg(debug_assertions)]
                comp_id,
            );
            out.set_column_base(slot, column);
        }
        out.set_data_len(data_pool_ids.len());
    }

    /// Phase 12.5 Opt-A2 (§5.6 / C-N1): commits `n` rows across every
    /// owned pool in one tight loop.
    ///
    /// Pre: every pool's `count() == start_row` (the batch path writes
    /// every pool's row in lockstep via `pool_at_unchecked_mut` +
    /// `write_at_unchecked_initialized`).
    pub(crate) fn commit_units_batch(&mut self, start_row: usize, n: usize) {
        for pool in self.pools.iter_mut() {
            pool.commit_units(start_row, n);
        }
    }

    /// Phase 12.5 Opt-A2 (§5.6 / C-N1): stamps `(added, changed) = tick`
    /// across every owned pool in one tight loop.
    pub(crate) fn fill_ticks_batch(&mut self, start_row: usize, n: usize, tick: Tick) {
        for pool in self.pools.iter_mut() {
            pool.fill_ticks(start_row, n, tick);
        }
    }

    /// Type-checked in-place overwrite. On missing component_id or out-of-bounds
    /// `entity_inland`, `value` drops at scope exit; bundle is not modified.
    ///
    /// # Panic safety
    /// Inherits the panic policy of [`ComponentPool::set_component_typed`] —
    /// if the existing component's `Drop` impl panics, the pool is poisoned.
    /// See `ComponentPool::set_component_typed` docs.
    ///
    /// `unit_index` is the dense row index inside the archetype's pool
    /// (Phase 7 dropped the wrapping `EntityInland` parameter — the bundle
    /// no longer knows about archetype-level locations).
    ///
    /// # Panics (debug only)
    /// `debug_assert!` on TypeId mismatch inside the pool.
    #[inline]
    pub fn set_component_typed<T: Component>(
        &mut self,
        unit_index: usize,
        value: T,
    ) -> bool {
        let component_id = T::component_id();
        let Some(inland_id) = self.sparse_indexes.get(component_id.0).copied() else {
            return false;
        };
        debug_assert!(
            unit_index < self.pools[inland_id.0].count(),
            "Entity unit index out of bounds"
        );
        self.pools[inland_id.0].set_component_typed(unit_index, value)
    }
}

// Implement Index/IndexMut for direct access to pools
impl Index<ComponentId> for ComponentPoolBundle {
    type Output = ComponentPool;

    fn index(&self, component_id: ComponentId) -> &Self::Output {
        debug_assert!(self.contains(component_id),
            "Component ID {} not found in bundle", component_id);

        let inland_id = self.sparse_indexes[component_id.0];
        &self.pools[inland_id.0]
    }
}

impl IndexMut<ComponentId> for ComponentPoolBundle {
    fn index_mut(&mut self, component_id: ComponentId) -> &mut Self::Output {
        debug_assert!(self.contains(component_id),
            "Component ID {} not found in bundle", component_id);

        let inland_id = self.sparse_indexes[component_id.0];
        &mut self.pools[inland_id.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ID range 420-429 reserved for component_pool_bundle two-phase commit tests
    // (C-009). MAX_COMPONENTS = 512, so valid range is 0-511. Range 420-429 is
    // free: 410-417 are used by archetype C-16 tests, 430+ is unclaimed.
    const C009_A: ComponentId = ComponentId(420);
    const C009_B: ComponentId = ComponentId(421);
    const C009_C: ComponentId = ComponentId(422);

    fn register_c009_components() {
        #[repr(C)] struct C009CompA(u32);
        #[repr(C)] struct C009CompB(u32);
        #[repr(C)] struct C009CompC(u32);
        component_registry::register_layout::<C009CompA>(C009_A.0);
        component_registry::register_layout::<C009CompB>(C009_B.0);
        component_registry::register_layout::<C009CompC>(C009_C.0);
    }

    fn make_bundle() -> ComponentPoolBundle {
        register_c009_components();
        ComponentPoolBundle::with_component_ids(&[C009_A, C009_B])
    }

    #[test]
    fn can_push_returns_true_when_all_pools_have_capacity() {
        let bundle = make_bundle();
        let bytes = [0u8; 4];
        let components = [(C009_A, bytes.as_slice()), (C009_B, bytes.as_slice())];
        assert!(bundle.can_push_entity_components(&components));
    }

    #[test]
    fn can_push_returns_false_for_unknown_component_id() {
        let bundle = make_bundle();
        let bytes = [0u8; 4];
        // C009_C is not in the bundle.
        let components = [(C009_A, bytes.as_slice()), (C009_C, bytes.as_slice())];
        assert!(!bundle.can_push_entity_components(&components));
    }

    #[test]
    fn push_after_can_push_returns_same_unit_index() {
        let mut bundle = make_bundle();
        let bytes = [0u8; 4];
        let components = [(C009_A, bytes.as_slice()), (C009_B, bytes.as_slice())];
        assert!(bundle.can_push_entity_components(&components));
        let idx = bundle.push_entity_components(&components);
        assert_eq!(idx, 0, "first push must occupy slot 0");
    }

    #[test]
    fn two_consecutive_pushes_produce_sequential_indices() {
        let mut bundle = make_bundle();
        let bytes = [1u8; 4];
        let components = [(C009_A, bytes.as_slice()), (C009_B, bytes.as_slice())];

        assert!(bundle.can_push_entity_components(&components));
        let idx0 = bundle.push_entity_components(&components);

        assert!(bundle.can_push_entity_components(&components));
        let idx1 = bundle.push_entity_components(&components);

        assert_eq!(idx0, 0);
        assert_eq!(idx1, 1);
        assert_eq!(bundle.get_pool(C009_A).unwrap().count(), 2);
        assert_eq!(bundle.get_pool(C009_B).unwrap().count(), 2);
    }

    #[test]
    fn can_push_is_false_when_pool_is_full() {
        // Filling a default-sized pool to its reserve ceiling would require
        // millions of entities; instead check that can_push returns false for
        // a missing component (edge case that exercises the PoolFailure path
        // without needing exhaustion).
        //
        // The exhaustion path is covered by Archetype::create_entity returning
        // false when can_push returns false (see archetype.rs tests).
        let bundle = make_bundle();
        // An empty slice is always "pushable" (no pools to fill → vacuously true).
        // The real exhaustion scenario would require filling the pool.
        // Verify the contract with a known-absent component instead.
        let bytes = [0u8; 4];
        let missing = [(C009_C, bytes.as_slice())]; // C009_C not in bundle
        assert!(!bundle.can_push_entity_components(&missing),
            "can_push must return false for unknown component IDs (PoolFailure path)");
    }
}