use std::ptr::NonNull;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::archetype::archetype_signature::ArchetypeSignature;
use crate::ecs::core::archetype::component_pool_bundle::ComponentPoolBundle;
use crate::ecs::memory::arena::Arena;

/// Archetype represents a unique combination of component types
/// All entities with the same component types belong to the same archetype
pub struct Archetype {
    /// Unique identifier for this archetype
    id: ArchetypeId,

    /// Storage for components organized by component type
    component_pools: ComponentPoolBundle,

    /// Current index for the next entity (equals number of entities)
    current_index: usize,

    /// Component signature for this archetype (bit mask of component IDs)
    signature: ArchetypeSignature,
    
    /// Reference to the arena used for memory allocation
    arena: NonNull<Arena>,
    
    /// Set of component IDs in this archetype for efficient iteration
    component_ids: Vec<ComponentId>,
}

impl Archetype {
    /// Creates a new archetype with the given ID and arena
    pub fn new(id: ArchetypeId, arena: &Arena) -> Self {
        Self {
            id,
            component_pools: ComponentPoolBundle::new(),
            current_index: 0,
            signature: ArchetypeSignature::new(ComponentMask::new()),
            arena: NonNull::from(arena),
            component_ids: Vec::new(),
        }
    }


    /// Creates a new archetype from a slice of component IDs
    pub fn create_by_ids(id: ArchetypeId, component_ids: &[ComponentId], arena: &Arena) -> Self {
        // Create a mask from the component IDs
        let mut mask = ComponentMask::new();
        for &comp_id in component_ids {
            mask.set(comp_id);
        }
        
        // Initialize archetype with mask and empty component pools
        let mut archetype = Self {
            id,
            component_pools: ComponentPoolBundle::new(),
            current_index: 0,
            signature: ArchetypeSignature::new(mask),
            arena: NonNull::from(arena),
            component_ids: component_ids.to_vec(),
        };
        
        // Create component pools for each component ID
        for &comp_id in component_ids {
            archetype.component_pools.add_pool(arena, comp_id);
        }
        
        archetype
    }

    /// Gets the unique ID of this archetype
    #[inline]
    pub fn id(&self) -> ArchetypeId {
        self.id
    }

    /// Registers a component type by ID
    pub fn register_component(&mut self, component_id: ComponentId) -> bool {
        // Check if this component type is already registered
        if self.signature.mask.contains(component_id) {
            return false;
        }

        // Get the arena for component pool creation
        let arena = unsafe { &*self.arena.as_ptr() };

        // Add a pool for this component type
        self.component_pools.add_pool(arena, component_id);
        
        // Update signature mask
        let mut new_mask = self.signature.mask;
        new_mask.set(component_id);
        self.signature = ArchetypeSignature::new(new_mask);
        
        // Add component ID to our list
        self.component_ids.push(component_id);

        true
    }

    /// Checks if this archetype contains a component with the given ID
    #[inline]
    pub fn has_component_id(&self, component_id: ComponentId) -> bool {
        self.signature.mask.contains(component_id)
    }

    /// Gets the number of component types in this archetype
    #[inline]
    pub fn component_count(&self) -> usize {
        self.component_ids.len()
    }

    /// Gets the number of entities in this archetype
    #[inline]
    pub fn entity_count(&self) -> usize {
        self.current_index
    }

    /// Creates a new entity in this archetype with the given components
    /// Takes a reference to EntityInland and a vector of (component_id, component_bytes) pairs
    /// Updates the EntityInland with the unit index of the new entity
    pub fn create_entity(&mut self, inland: &mut EntityInland, components: Vec<(ComponentId, &[u8])>) -> bool {
        debug_assert_eq!(inland.archetype_id(), self.id, 
            "EntityInland archetype_id mismatch");
        
        // Make sure all required components are provided
        for &comp_id in &self.component_ids {
            if !components.iter().any(|(id, _)| *id == comp_id) {
                return false; // Missing a required component
            }
        }
        
        // Add components to pools
        let unit_indices = match self.component_pools.add_entity_components(components) {
            Some(indices) => indices,
            None => return false,
        };
        
        if unit_indices.is_empty() {
            return false;
        }
        
        // Use the first component's unit index
        let unit_index = unit_indices[0];
        
        // Update the inland reference with the unit index
        inland.set_unit_index(unit_index);
        
        // Increment entity counter
        self.current_index += 1;
        
        true
    }

    /// Removes an entity and all its components from this archetype
    /// Takes reference to EntityInland for the entity to remove
    /// Uses swap_remove for entities in the middle, and pop for the last entity
    ///
    /// WARNING: This function should not be used with the same entity_inland and last_entity_inland
    pub fn remove_entity(&mut self, entity_inland: &mut EntityInland, last_entity_inland: &mut EntityInland) -> bool {
        debug_assert_eq!(entity_inland.archetype_id(), self.id, 
            "EntityInland archetype_id mismatch");
        
            // Call swap_remove on component pools
            if let Err(_) = self.component_pools.swap_remove_unit(entity_inland, last_entity_inland) {
                return false;
            }
            
            // Increment generation of deleted entity
            entity_inland.increment_generation();
        
        // Decrement entity counter
        self.current_index -= 1;
        
        true
    }

    /// Gets a raw pointer to a component using EntityInland for direct access
    #[inline]
    pub fn get_component_raw(&self, inland: &EntityInland, component_id: ComponentId) -> Option<*const u8> {
        debug_assert_eq!(inland.archetype_id(), self.id, 
            "EntityInland archetype_id mismatch");
        
        let unit_index = inland.unit_index();
        
        // Get the component pool for this component type
        let pool = self.component_pools.get_pool(component_id)?;
        
        // Use the unit index directly
        pool.get_raw(unit_index)
    }

    /// Gets a mutable raw pointer to a component using EntityInland for direct access
    #[inline]
    pub fn get_component_raw_mut(&mut self, inland: &EntityInland, component_id: ComponentId) -> Option<*mut u8> {
        debug_assert_eq!(inland.archetype_id(), self.id, 
            "EntityInland archetype_id mismatch");
        
        let unit_index = inland.unit_index();
        
        // Get the component pool for this component type
        let pool = self.component_pools.get_pool_mut(component_id)?;
        
        // Use the unit index directly
        pool.get_raw_mut(unit_index)
    }

    /// Sets a component value using EntityInland for direct access
    #[inline]
    pub fn set_component(&mut self, inland: &EntityInland, component_id: ComponentId, bytes: &[u8]) -> bool {
        debug_assert_eq!(inland.archetype_id(), self.id, 
            "EntityInland archetype_id mismatch");
        
        let unit_index = inland.unit_index();
        
        // Get the component pool for this component type
        let pool = match self.component_pools.get_pool_mut(component_id) {
            Some(p) => p,
            None => return false,
        };
        
        // Set the component using the unit index directly
        pool.set_component(unit_index, bytes)
    }

    /// Gets a reference to the component pool bundle
    #[inline]
    pub fn component_pools(&self) -> &ComponentPoolBundle {
        &self.component_pools
    }

    /// Gets a mutable reference to the component pool bundle
    #[inline]
    pub fn component_pools_mut(&mut self) -> &mut ComponentPoolBundle {
        &mut self.component_pools
    }
    
    /// Gets the archetype signature
    #[inline]
    pub fn signature(&self) -> &ArchetypeSignature {
        &self.signature
    }
    
    /// Gets the component IDs in this archetype
    #[inline]
    pub fn component_ids(&self) -> &[ComponentId] {
        &self.component_ids
    }
    
    /// Checks if this archetype has all the specified component IDs
    pub fn matches_component_ids(&self, component_ids: &[ComponentId]) -> bool {
        // Check if this archetype contains all the requested components
        for &comp_id in component_ids {
            if !self.signature.mask.contains(comp_id) {
                return false;
            }
        }
        
        true
    }
    
    /// Initialize an EntityInland for the next entity slot in this archetype
    #[inline]
    pub fn init_entity_inland(&self, inland: &mut EntityInland) {
        inland.set_archetype_id(self.id);
        // Unit index will be set during component creation
        // Generation is set by the ECS master
    }

    /// Removes the last entity from this archetype
    /// Takes a reference to the last entity's EntityInland to update its generation
    pub fn pop(&mut self, last_entity_inland: &mut EntityInland) -> bool {
        debug_assert!(self.current_index > 0, "Attempting to pop from an empty archetype");
        debug_assert!(self.component_pools.pop_entity(), "Failed to pop entity from component pools");
        
        // Increment generation of the popped entity
        last_entity_inland.increment_generation();
        
        // Decrement entity counter
        self.current_index -= 1;
        
        true
    }
}