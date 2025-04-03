use std::ops::{Index, IndexMut};
use crate::ecs::core::component::Component;
use crate::ecs::identifiers::primitives::{ComponentId, InlandComponentId, InlandPoolId};
use crate::ecs::memory::component_pool::ComponentPool;
use crate::ecs::memory::arena::Arena;
use boyko_utils::sparse_map::sparse_map::SparseMap;
use crate::ecs::identifiers::id_unit::UnitId;

pub struct ComponentPoolBundle {
    pools: Vec<ComponentPool>,
    sparse_indexes: SparseMap<InlandPoolId>,
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

    /// Adds a component pool for a specific component type
    /// Returns the internal index assigned to this pool
    pub fn add_pool<T: Component>(&mut self, arena: &Arena) -> InlandPoolId {
        let component_id = T::component_id();

        // Check if pool for this component type already exists
        if let Some(&inland_id) = self.sparse_indexes.get(component_id) {
            return inland_id;
        }

        // Create a new pool for this component type
        let pool = ComponentPool::with_default_sizes::<T>(arena);

        // Add pool to the bundle
        let inland_id = self.pools.len();
        self.pools.push(pool);
        self.sparse_indexes.insert(component_id, inland_id);

        inland_id
    }

    /// Adds a component pool and returns self for method chaining
    /// Useful for fluent initialization of the bundle
    pub fn with_component<T: Component>(mut self, arena: &Arena) -> Self {
        self.add_pool::<T>(arena);
        self
    }

    /// Gets a reference to a component pool by component type
    pub fn get_pool<T: Component>(&self) -> Option<&ComponentPool> {
        let component_id = T::component_id();
        self.sparse_indexes.get(component_id).map(|&inland_id| &self.pools[inland_id])
    }

    /// Gets a mutable reference to a component pool by component type
    pub fn get_pool_mut<T: Component>(&mut self) -> Option<&mut ComponentPool> {
        let component_id = T::component_id();
        self.sparse_indexes.get(component_id).copied().map(move |inland_id| &mut self.pools[inland_id])
    }

    /// Gets a reference to a component pool by its component ID
    pub fn get_pool_by_id(&self, component_id: ComponentId) -> Option<&ComponentPool> {
        self.sparse_indexes.get(component_id).map(|&inland_id| &self.pools[inland_id])
    }

    /// Gets a mutable reference to a component pool by its component ID
    pub fn get_pool_mut_by_id(&mut self, component_id: ComponentId) -> Option<&mut ComponentPool> {
        self.sparse_indexes.get(component_id).copied().map(move |inland_id| &mut self.pools[inland_id])
    }

    /// Checks if the bundle contains a pool for a specific component type
    pub fn contains<T: Component>(&self) -> bool {
        let component_id = T::component_id();
        self.sparse_indexes.contains(component_id)
    }

    /// Checks if the bundle contains a pool for a component with the specified ID
    pub fn contains_id(&self, component_id: ComponentId) -> bool {
        self.sparse_indexes.contains(component_id)
    }

    /// Gets the number of component pools in the bundle
    pub fn len(&self) -> usize {
        self.pools.len()
    }

    /// Checks if the bundle is empty
    pub fn is_empty(&self) -> bool {
        self.pools.is_empty()
    }

    /// Gets an iterator over all component pools
    pub fn iter(&self) -> impl Iterator<Item = &ComponentPool> {
        self.pools.iter()
    }

    /// Gets a mutable iterator over all component pools
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut ComponentPool> {
        self.pools.iter_mut()
    }

    /// Adds a set of components for an entity to all pools in the bundle
    /// Returns a vector of UnitIds, one for each component pool in the same order as the pools
    ///
    /// # Type Safety
    /// This method uses type erasure - it's the caller's responsibility to ensure
    /// that components are paired with the correct pools.
    pub fn add_entity_components(&mut self, components: Vec<(usize, *const u8)>) -> Option<Vec<UnitId>> {
        if components.len() != self.pools.len() {
            return None; // Number of components doesn't match number of pools
        }

        let mut result = Vec::with_capacity(self.pools.len());

        // First check if all components can be added
        for (pool_idx, (component_id, _)) in components.iter().enumerate() {
            // Verify component type matches the pool
            if self.pools[pool_idx].component_id() != *component_id {
                return None; // Type mismatch
            }
        }

        // Then add all components
        for (pool_idx, (_, component_ptr)) in components.iter().enumerate() {
            // Unsafe: We're trusting the caller to provide the correct component types
            let unit_id = unsafe {
                self.pools[pool_idx].raw_add(*component_ptr)
            };

            if let Some(id) = unit_id {
                result.push(id);
            } else {
                // If any component fails to add, we need to roll back
                for i in 0..result.len() {
                    self.pools[i].swap_remove(result[i]);
                }
                return None;
            }
        }

        Some(result)
    }

    /// Type-safe version to add components for an entity
    /// Takes any number of components that implement Component trait
    /// Returns a vector of UnitIds in the same order as the components
    pub fn add_entity<Components: ComponentTuple>(&mut self, components: Components) -> Option<Vec<UnitId>> {
        components.add_to_pool_bundle(self)
    }

    /// Removes an entity's components from all pools
    /// Takes a vector of UnitIds, one for each pool in the same order as the pools
    pub fn remove_entity(&mut self, component_ids: Vec<UnitId>) -> bool {
        if component_ids.len() != self.pools.len() {
            return false; // Number of IDs doesn't match number of pools
        }

        let mut success = true;

        // Remove components from all pools
        for (pool_idx, unit_id) in component_ids.iter().enumerate() {
            if !self.pools[pool_idx].swap_remove(*unit_id) {
                success = false;
                // Continue removing other components even if one fails
            }
        }

        success
    }
}

// These are already implemented in the code, included for completeness
impl Index<InlandComponentId> for ComponentPoolBundle {
    type Output = ComponentPool;

    fn index(&self, index: InlandComponentId) -> &Self::Output {
        &self.pools[index]
    }
}

impl IndexMut<InlandComponentId> for ComponentPoolBundle {
    fn index_mut(&mut self, index: InlandComponentId) -> &mut Self::Output {
        &mut self.pools[index]
    }
}


/// Trait to support variadic component adding
pub trait ComponentTuple {
    fn add_to_pool_bundle(self, bundle: &mut ComponentPoolBundle) -> Option<Vec<UnitId>>;
}

// Implement for empty tuple (no components)
impl ComponentTuple for () {
    fn add_to_pool_bundle(self, _bundle: &mut ComponentPoolBundle) -> Option<Vec<UnitId>> {
        Some(vec![])
    }
}

// Implement for single component
impl<T: Component> ComponentTuple for T {
    fn add_to_pool_bundle(self, bundle: &mut ComponentPoolBundle) -> Option<Vec<UnitId>> {
        if let Some(pool) = bundle.get_pool_mut::<T>() {
            pool.add(self).map(|id| vec![id])
        } else {
            None
        }
    }
}

// Implement for tuple of two components
impl<T1: Component, T2: Component> ComponentTuple for (T1, T2) {
    fn add_to_pool_bundle(self, bundle: &mut ComponentPoolBundle) -> Option<Vec<UnitId>> {
        let pool1 = bundle.get_pool_mut::<T1>()?;
        let id1 = pool1.add(self.0)?;

        let pool2 = bundle.get_pool_mut::<T2>()?;
        if let Some(id2) = pool2.add(self.1) {
            Some(vec![id1, id2])
        } else {
            // Roll back if second component fails
            let pool1 = bundle.get_pool_mut::<T1>().unwrap();
            pool1.swap_remove(id1);
            None
        }
    }
}
impl<T: Component> ComponentTuple for (T,) {
    fn add_to_pool_bundle(self, bundle: &mut ComponentPoolBundle) -> Option<Vec<UnitId>> {
        if let Some(pool) = bundle.get_pool_mut::<T>() {
            pool.add(self.0).map(|id| vec![id])
        } else {
            None
        }
    }
}