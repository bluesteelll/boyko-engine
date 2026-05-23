use std::ops::{Index, IndexMut};

use anyhow::{Result, bail};
use boyko_utils::sparse_map::sparse_map::SparseMap;

use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::identifiers::primitives::{ComponentId, InlandPoolId};
use crate::ecs::memory::arena::Arena;
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
    pub fn with_component_ids(arena: &Arena, component_ids: &[ComponentId]) -> Self {
        let mut bundle = Self::with_capacity(component_ids.len());
        
        // Add pools for all specified component IDs
        for &component_id in component_ids {
            bundle.add_pool(arena, component_id);
        }
        
        bundle
    }

    /// Adds a component pool for a specific component ID
    /// Returns the internal index assigned to this pool
    pub fn add_pool(&mut self, arena: &Arena, component_id: ComponentId) -> InlandPoolId {
        // Check if pool for this component type already exists
        if let Some(&inland_id) = self.sparse_indexes.get(component_id) {
            return inland_id;
        }

        // Verify component is registered - only in debug builds
        debug_assert!(component_registry::get_layout(component_id).is_some(),
            "Component ID {} not registered in layout registry", component_id);

        // Create a new pool for this component type
        let pool = ComponentPool::with_default_sizes(arena, component_id);

        // Add pool to the bundle
        let inland_id = self.pools.len();
        self.pools.push(pool);
        self.sparse_indexes.insert(component_id, inland_id);

        inland_id
    }

    /// Gets a component pool by component ID
    pub fn get_pool(&self, component_id: ComponentId) -> Option<&ComponentPool> {
        let inland_id = self.sparse_indexes.get(component_id)?;
        Some(&self.pools[*inland_id])
    }

    /// Gets a mutable component pool by component ID
    pub fn get_pool_mut(&mut self, component_id: ComponentId) -> Option<&mut ComponentPool> {
        let inland_id = *self.sparse_indexes.get(component_id)?;
        Some(&mut self.pools[inland_id])
    }

    /// Checks if the bundle contains a pool for a component with the specified ID
    pub fn contains(&self, component_id: ComponentId) -> bool {
        self.sparse_indexes.contains(component_id)
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

    /// Gets a raw pointer to a component by ComponentId and EntityInland
    pub fn get_unit_raw(&self, component_id: ComponentId, entity_inland: &EntityInland) -> Option<*const u8> {
        // Debug assert for component existence for better error messages
        debug_assert!(self.contains(component_id), 
            "Component ID {} not found in bundle", component_id);
            
        let inland_id = self.sparse_indexes.get(component_id)?;
        
        // Debug assert for entity index bounds
        debug_assert!(entity_inland.unit_index() < self.pools[*inland_id].count(),
            "Entity unit index out of bounds: {} >= {}", 
            entity_inland.unit_index(), self.pools[*inland_id].count());
            
        self.pools[*inland_id].get_raw(entity_inland.unit_index())
    }
    
    /// Gets raw pointers to multiple components for a single EntityInland
    /// Returns a vector of (ComponentId, *const u8) pairs
    pub fn get_units_raw_indexed(&self, component_ids: &[ComponentId], entity_inland: &EntityInland) -> Vec<(ComponentId, *const u8)> {
        let mut result = Vec::with_capacity(component_ids.len());
        
        for &component_id in component_ids {
            if let Some(ptr) = self.get_unit_raw(component_id, entity_inland) {
                result.push((component_id, ptr));
            }
        }
        
        result
    }
    
    /// Gets raw pointers to multiple components for a single EntityInland
    /// Returns a vector of pointers in the same order as the input component IDs
    /// Missing components will have null pointers (represented as std::ptr::null())
    pub fn get_units_raw(&self, component_ids: &[ComponentId], entity_inland: &EntityInland) -> Vec<*const u8> {
        let mut result = Vec::with_capacity(component_ids.len());
        
        for &component_id in component_ids {
            if let Some(ptr) = self.get_unit_raw(component_id, entity_inland) {
                result.push(ptr);
            } else {
                result.push(std::ptr::null());
            }
        }
        
        result
    }
    
    /// Gets a mutable component by entity inland and component ID
    pub fn get_unit_raw_mut(&mut self, component_id: ComponentId, entity_inland: &EntityInland) -> Option<*mut u8> {
        // Debug assert for component existence for better error messages
        debug_assert!(self.contains(component_id), 
            "Component ID {} not found in bundle", component_id);
        
        let inland_id = *self.sparse_indexes.get(component_id)?;
        
        // Debug assert for entity index bounds
        debug_assert!(entity_inland.unit_index() < self.pools[inland_id].count(),
            "Entity unit index out of bounds: {} >= {}", 
            entity_inland.unit_index(), self.pools[inland_id].count());
            
        self.pools[inland_id].get_raw_mut(entity_inland.unit_index())
    }
    
    /// Gets mutable raw pointers to multiple components for a single EntityInland
    /// Returns a vector of (ComponentId, *mut u8) pairs
    pub fn get_units_raw_indexed_mut(&mut self, component_ids: &[ComponentId], entity_inland: &EntityInland) -> Vec<(ComponentId, *mut u8)> {
        let mut result = Vec::with_capacity(component_ids.len());
        
        // We need to handle mutability carefully
        // Can't use a simple loop because of borrow checking limitations
        for &component_id in component_ids {
            if let Some(inland_id) = self.sparse_indexes.get(component_id).copied() {
                // Debug assert for entity index bounds
                debug_assert!(entity_inland.unit_index() < self.pools[inland_id].count(),
                    "Entity unit index out of bounds: {} >= {}", 
                    entity_inland.unit_index(), self.pools[inland_id].count());
                
                // Get the raw pointer
                if let Some(ptr) = self.pools[inland_id].get_raw_mut(entity_inland.unit_index()) {
                    result.push((component_id, ptr));
                }
            }
        }
        
        result
    }
    
    /// Gets mutable raw pointers to multiple components for a single EntityInland
    /// Returns a vector of pointers in the same order as the input component IDs
    /// Missing components will have null pointers (represented as std::ptr::null_mut())
    pub fn get_units_raw_mut(&mut self, component_ids: &[ComponentId], entity_inland: &EntityInland) -> Vec<*mut u8> {
        let mut result = Vec::with_capacity(component_ids.len());
        
        for &component_id in component_ids {
            if let Some(ptr) = self.get_unit_raw_mut(component_id, entity_inland) {
                result.push(ptr);
            } else {
                result.push(std::ptr::null_mut());
            }
        }
        
        result
    }

    /// Adds a component to the appropriate pool
    pub fn add_component(&mut self, component_id: ComponentId, component_bytes: &[u8]) -> Option<usize> {
        debug_assert!(self.contains(component_id), 
            "Component ID {} not found in bundle", component_id);
        
        // Verify component size matches registry - debug only check
        debug_assert_eq!(
            component_bytes.len(), 
            component_registry::get_component_size(component_id).unwrap_or(0),
            "Component size mismatch for ID {}", component_id
        );
        
        let inland_id = *self.sparse_indexes.get(component_id)?;
        self.pools[inland_id].add(component_bytes)
    }

    /// Sets a component's value
    pub fn set_component(&mut self, component_id: ComponentId, entity_inland: &EntityInland, component_bytes: &[u8]) -> bool {
        debug_assert!(self.contains(component_id), 
            "Component ID {} not found in bundle", component_id);
            
        // Verify component size matches registry - debug only check
        debug_assert_eq!(
            component_bytes.len(), 
            component_registry::get_component_size(component_id).unwrap_or(0),
            "Component size mismatch for ID {}", component_id
        );
        
        if let Some(inland_id) = self.sparse_indexes.get(component_id) {
            // Debug assert for entity index bounds
            debug_assert!(entity_inland.unit_index() < self.pools[*inland_id].count(),
                "Entity unit index out of bounds: {} >= {}", 
                entity_inland.unit_index(), self.pools[*inland_id].count());
                
            self.pools[*inland_id].set_component(entity_inland.unit_index(), component_bytes)
        } else {
            false
        }
    }

    /// Adds multiple components for an entity across all pools
    /// Takes a vector of (component_id, byte_data) pairs
    pub fn add_entity_components(&mut self, components: Vec<(ComponentId, &[u8])>) -> Option<Vec<usize>> {
        // Check all components exist in debug builds
        debug_assert!(components.iter().all(|(id, _)| self.contains(*id)),
            "Not all component IDs found in bundle");
        
        // Vector to store indices
        let mut indices = Vec::with_capacity(components.len());
        
        // First try to add all components
        for (component_id, bytes) in &components {
            // Verify component size matches registry - debug only check
            debug_assert_eq!(
                bytes.len(), 
                component_registry::get_component_size(*component_id).unwrap_or(0),
                "Component size mismatch for ID {}", component_id
            );
            
            let inland_id = self.sparse_indexes.get(*component_id).copied()?;
            let idx = self.pools[inland_id].add(bytes)?;
            indices.push(idx);
        }
        
        // If we successfully added all components, return the indices
        Some(indices)
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
pub fn swap_remove_unit(&mut self, unit_index: usize) -> Result<()> {
    let mut success = true;

    // Debug check for valid unit index
    debug_assert!(self.pools.iter().all(|pool| unit_index < pool.count()),
        "Unit index {} out of bounds in some pools", unit_index);

    // Remove components from each pool using the unit index
    for pool in self.pools.iter_mut() {
        success &= pool.swap_remove(unit_index);
    }

    if !success {
        bail!("Error: in ComponentPoolBundle.swap_remove_unit()")
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
        let inland_id = self.sparse_indexes.get(component_id).copied()?;
        self.pools[inland_id].add_typed(value)
    }

    /// Type-checked in-place overwrite. On missing component_id or out-of-bounds
    /// `entity_inland`, `value` drops at scope exit; bundle is not modified.
    ///
    /// # Panic safety
    /// Inherits the panic policy of [`ComponentPool::set_component_typed`] —
    /// if the existing component's `Drop` impl panics, the pool is poisoned.
    /// See `ComponentPool::set_component_typed` docs.
    ///
    /// # Panics (debug only)
    /// `debug_assert!` on TypeId mismatch inside the pool.
    #[inline]
    pub fn set_component_typed<T: Component>(
        &mut self,
        entity_inland: &EntityInland,
        value: T,
    ) -> bool {
        let component_id = T::component_id();
        let Some(inland_id) = self.sparse_indexes.get(component_id).copied() else {
            return false;
        };
        debug_assert!(
            entity_inland.unit_index() < self.pools[inland_id].count(),
            "Entity unit index out of bounds"
        );
        self.pools[inland_id].set_component_typed(entity_inland.unit_index(), value)
    }
}

// Implement Index/IndexMut for direct access to pools
impl Index<ComponentId> for ComponentPoolBundle {
    type Output = ComponentPool;

    fn index(&self, component_id: ComponentId) -> &Self::Output {
        debug_assert!(self.contains(component_id), 
            "Component ID {} not found in bundle", component_id);
            
        let inland_id = self.sparse_indexes[component_id];
        &self.pools[inland_id]
    }
}

impl IndexMut<ComponentId> for ComponentPoolBundle {
    fn index_mut(&mut self, component_id: ComponentId) -> &mut Self::Output {
        debug_assert!(self.contains(component_id), 
            "Component ID {} not found in bundle", component_id);
            
        let inland_id = self.sparse_indexes[component_id];
        &mut self.pools[inland_id]
    }
}