use std::ops::{Index, IndexMut};
use crate::ecs::core::component::component::Component;
use crate::ecs::identifiers::primitives::{ComponentId, InlandComponentId, InlandPoolId};
use crate::ecs::memory::component_pool::ComponentPool;
use crate::ecs::memory::arena::Arena;
use boyko_utils::sparse_map::sparse_map::SparseMap;
use anyhow::{Result, bail};

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


    fn inland_get_unit<T: Component>(&self, inland_component_id: InlandComponentId) -> anyhow::Result<&T> {
    let pool = &self.pools[inland_component_id as usize];
        Ok(pool.get(inland_component_id).unwrap())

    }
    


    pub fn get_unit <T: Component>(&self, component_id: ComponentId) -> anyhow::Result<&T> {
        self.inland_get_unit(self.sparse_indexes[component_id])
    }

    /// Removes components from all pools using indices
    /// Each index corresponds to the component in the respective pool
    /// Returns true if all components were successfully removed
    pub fn remove_entity(&mut self, index: usize) -> anyhow::Result<usize> {
        let mut success = true;


        // Remove components from each pool using their respective indices
        for pool in self.pools.iter_mut() {
            success &= pool.swap_remove(self.sparse_indexes[index]);
        }
        if !success {
            bail!("Error: in ComponentPoolBundle.swap_remove()")
        }
        self.sparse_indexes.swap_remove(index);
        Ok(index)
        
    }
}

// impl Index<InlandComponentId> for ComponentPoolBundle {
//     type Output = ComponentPool;
//
//     fn index(&self, index: InlandComponentId) -> &Self::Output {
//         &self.pools[index]
//     }
// }
//
// impl IndexMut<InlandComponentId> for ComponentPoolBundle {
//     fn index_mut(&mut self, index: InlandComponentId) -> &mut Self::Output {
//         &mut self.pools[index]
//     }
// }

