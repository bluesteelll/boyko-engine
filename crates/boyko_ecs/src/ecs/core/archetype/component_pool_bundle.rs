use std::ops::{Index, IndexMut};
use crate::ecs::core::component::Component;
use crate::ecs::identifiers::primitives::{ComponentId, InlandComponentId, InlandPoolId};
use crate::ecs::memory::component_pool::ComponentPool;
use crate::ecs::memory::arena::Arena;
use boyko_utils::sparse_map::sparse_map::SparseMap;
use anyhow::{result, bali};

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


    /// Adds a set of components for an entity to all pools in the bundle
    /// Returns a vector of indices, one for each component pool in the same order as the pools
    ///
    /// # Type Safety
    /// This method uses type erasure - it's the caller's responsibility to ensure
    /// that components are paired with the correct pools.
    pub fn add_entity_components(&mut self, components: Vec<(usize, *const u8)>) -> Option<Vec<usize>> {
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
            let index = unsafe {
                self.pools[pool_idx].raw_add(*component_ptr)
            };

            if let Some(idx) = index {
                result.push(idx);
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


    /// Removes components from all pools using indices
    /// Each index corresponds to the component in the respective pool
    /// Returns true if all components were successfully removed
    pub fn remove_entity(&mut self, index: usize) -> anyhow::Result<usize> {
        let mut success = true;


        // Remove components from each pool using their respective indices
        for mut pool in self.pools.iter() {

            success &= pool.swap_remove(self.sparse_indexes[index]);
        }
        if !success {
            bali!("Error: in ComponentPoolBundle.swap_remove()")
        }
        self.sparse_indexes.swap_remove(index);
        Ok(index)
        
    }
}

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

