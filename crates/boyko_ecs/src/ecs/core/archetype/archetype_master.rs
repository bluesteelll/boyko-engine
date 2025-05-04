use std::ptr::NonNull;
use crate::ecs::core::archetype::archetype_bundle::ArchetypeBundle;
use crate::ecs::core::archetype::archetype_registry::ArchetypeRegistry;
use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use crate::ecs::memory::arena::Arena;

/// Master manager for archetypes, providing creation and lookup capabilities
/// Integrates ArchetypeBundle for storage and ArchetypeRegistry for efficient queries
pub struct ArchetypeMaster {
    /// Storage for archetypes with direct access by ID
    archetypes: ArchetypeBundle,
    
    /// Registry for efficient component-based lookups
    registry: ArchetypeRegistry,
    
    /// Memory arena for component allocation
    arena: NonNull<Arena>,
    
    /// Next available archetype ID
    next_archetype_id: ArchetypeId,
}

impl ArchetypeMaster {
    /// Creates a new ArchetypeMaster with the given arena
    pub fn new(arena: &Arena) -> Self {
        Self {
            archetypes: ArchetypeBundle::new(),
            registry: ArchetypeRegistry::with_capacity(64), // Initial capacity for registry
            arena: NonNull::from(arena),
            next_archetype_id: 1, // Start from ID 1
        }
    }
    
    /// Creates a new ArchetypeMaster with the given capacity
    pub fn with_capacity(arena: &Arena, capacity: usize) -> Self {
        Self {
            archetypes: ArchetypeBundle::with_capacity(capacity),
            registry: ArchetypeRegistry::with_capacity(capacity),
            arena: NonNull::from(arena),
            next_archetype_id: 1,
        }
    }
    
    /// Gets the arena reference
    #[inline]
    fn arena(&self) -> &Arena {
        unsafe { self.arena.as_ref() }
    }
    
    /// Creates a new archetype from a slice of component IDs
    /// Returns the ID of the created archetype
    pub fn create_archetype(&mut self, component_ids: &[ComponentId]) -> ArchetypeId {
        // First check if an archetype with exactly these components already exists
        let mask = ComponentMask::from_components(component_ids);
        let existing = self.registry.find_exact_match(&mask);
        
        if let Some(first_id) = existing.first() {
            return *first_id; // Return existing archetype ID
        }
        
        // Allocate a new archetype ID
        let archetype_id = self.next_archetype_id;
        self.next_archetype_id += 1;
        
        // Access the arena pointer directly to avoid borrowing self
        let arena = unsafe { self.arena.as_ref() };
        
        // Create a new archetype with these component IDs
        let inland_id = self.archetypes.add_archetype_from_components(
            archetype_id, 
            component_ids, 
            arena
        );
        
        // Register the archetype with the registry
        self.registry.register_archetype(archetype_id, mask);
        
        archetype_id
    }
    
    /// Gets a reference to an archetype by ID
    pub fn get_archetype(&self, archetype_id: ArchetypeId) -> Option<&Archetype> {
        self.archetypes.get_archetype(archetype_id)
    }
    
    /// Gets a mutable reference to an archetype by ID
    pub fn get_archetype_mut(&mut self, archetype_id: ArchetypeId) -> Option<&mut Archetype> {
        self.archetypes.get_archetype_mut(archetype_id)
    }
    
    /// Finds all archetypes that contain the specified components
    pub fn find_archetypes_with_components(&self, component_ids: &[ComponentId]) -> Vec<ArchetypeId> {
        self.registry.find_archetypes_with_components(component_ids)
    }
    
    /// Finds all archetypes containing all components in the specified mask
    pub fn find_matching_archetypes(&self, mask: &ComponentMask) -> Vec<ArchetypeId> {
        self.registry.find_matching_archetypes(mask)
    }
    
    /// Returns the number of registered archetypes
    #[inline]
    pub fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }
    
    /// Adds an existing archetype to the master
    /// This is used when loading archetypes from external sources or for cloning
    pub fn add_existing_archetype(&mut self, archetype: Archetype) -> ArchetypeId {
        let archetype_id = archetype.id();
        
        // Extract component IDs before moving the archetype
        let component_ids = archetype.component_ids().to_vec();
        
        // Register with the bundle
        self.archetypes.add_archetype(archetype);
        
        // Create a mask from the component IDs
        let mask = ComponentMask::from_components(&component_ids);
        
        // Register with the registry
        self.registry.register_archetype(archetype_id, mask);
        
        // Update next ID if necessary
        if archetype_id >= self.next_archetype_id {
            self.next_archetype_id = archetype_id + 1;
        }
        
        archetype_id
    }
    
    /// Removes an archetype by ID
    /// Returns true if the archetype was found and removed
    pub fn remove_archetype(&mut self, archetype_id: ArchetypeId) -> bool {
        // First unregister from the registry
        let registry_success = self.registry.unregister_archetype(archetype_id);
        
        // If registry removal failed, the archetype wasn't registered
        if !registry_success {
            return false;
        }
        
        // Now remove from the archetype bundle
        let bundle_success = self.archetypes.remove_archetype(archetype_id);
        
        debug_assert!(bundle_success, "Registry and bundle are out of sync");
        
        bundle_success
    }
    
    /// Finds or creates an archetype with the specified component IDs
    /// This is an optimized version that first tries to find an existing archetype
    pub fn get_or_create_archetype(&mut self, component_ids: &[ComponentId]) -> ArchetypeId {
        // First try to find an existing archetype with the exact components
        let mask = ComponentMask::from_components(component_ids);
        let existing = self.registry.find_exact_match(&mask);
        
        if let Some(first_id) = existing.first() {
            *first_id // Return existing archetype ID
        } else {
            // Create a new archetype
            self.create_archetype(component_ids)
        }
    }
    
    /// Adds a component type to an existing archetype
    /// Returns the ID of the new archetype containing the added component
    pub fn add_component_to_archetype(
        &mut self, 
        source_archetype_id: ArchetypeId, 
        component_id: ComponentId
    ) -> Option<ArchetypeId> {
        // Get the source archetype
        let source_archetype = self.get_archetype(source_archetype_id)?;
        
        // Get all component IDs from the source archetype
        let mut new_components = source_archetype.component_ids().to_vec();
        
        // Check if the component already exists in the archetype
        if new_components.contains(&component_id) {
            return Some(source_archetype_id); // No change needed
        }
        
        // Add the new component ID
        new_components.push(component_id);
        
        // Create or get the new archetype
        Some(self.get_or_create_archetype(&new_components))
    }
    
    /// Removes a component type from an existing archetype
    /// Returns the ID of the new archetype without the component
    pub fn remove_component_from_archetype(
        &mut self, 
        source_archetype_id: ArchetypeId, 
        component_id: ComponentId
    ) -> Option<ArchetypeId> {
        // Get the source archetype
        let source_archetype = self.get_archetype(source_archetype_id)?;
        
        // Get all component IDs from the source archetype
        let source_components = source_archetype.component_ids();
        
        // Check if the component exists in the archetype
        if !source_components.contains(&component_id) {
            return Some(source_archetype_id); // No change needed
        }
        
        // Create a new component list without the specified component
        let new_components: Vec<ComponentId> = source_components
            .iter()
            .filter(|&&id| id != component_id)
            .copied()
            .collect();
        
        // Create or get the new archetype
        Some(self.get_or_create_archetype(&new_components))
    }
    
    /// Gets a reference to the underlying ArchetypeBundle
    #[inline]
    pub fn archetype_bundle(&self) -> &ArchetypeBundle {
        &self.archetypes
    }
    
    /// Gets a mutable reference to the underlying ArchetypeBundle
    #[inline]
    pub fn archetype_bundle_mut(&mut self) -> &mut ArchetypeBundle {
        &mut self.archetypes
    }
    
    /// Gets a reference to the underlying ArchetypeRegistry
    #[inline]
    pub fn archetype_registry(&self) -> &ArchetypeRegistry {
        &self.registry
    }
    
    /// Gets a mutable reference to the underlying ArchetypeRegistry
    #[inline]
    pub fn archetype_registry_mut(&mut self) -> &mut ArchetypeRegistry {
        &mut self.registry
    }
    
    /// Returns an iterator over all archetypes
    pub fn iter_archetypes(&self) -> impl Iterator<Item = &Archetype> {
        self.archetypes.iter()
    }
    
    /// Clear all archetypes and reset the ID counter
    pub fn clear(&mut self) {
        // Create new empty collections
        self.archetypes = ArchetypeBundle::new();
        self.registry.clear();
        self.next_archetype_id = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    /// Helper to create a test arena
    fn create_test_arena() -> Arena {
        Arena::new()
    }
    
    #[test]
    fn test_create_archetype() {
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);
        
        // Create an archetype with components 1, 2, 3
        let archetype_id = master.create_archetype(&[1, 2, 3]);
        
        // Verify archetype was created
        assert_eq!(archetype_id, 1);
        assert_eq!(master.archetype_count(), 1);
        
        // Get the archetype
        let archetype = master.get_archetype(archetype_id).unwrap();
        
        // Verify component IDs
        let component_ids = archetype.component_ids();
        assert_eq!(component_ids.len(), 3);
        assert!(component_ids.contains(&1));
        assert!(component_ids.contains(&2));
        assert!(component_ids.contains(&3));
    }
    
    #[test]
    fn test_remove_archetype() {
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);
        
        // Create archetypes
        let archetype1 = master.create_archetype(&[1, 2]);
        let archetype2 = master.create_archetype(&[2, 3]);
        let archetype3 = master.create_archetype(&[1, 3]);
        
        assert_eq!(master.archetype_count(), 3);
        
        // Remove archetype2
        let success = master.remove_archetype(archetype2);
        assert!(success);
        assert_eq!(master.archetype_count(), 2);
        
        // Try to get the removed archetype
        assert!(master.get_archetype(archetype2).is_none());
        
        // Other archetypes should still exist
        assert!(master.get_archetype(archetype1).is_some());
        assert!(master.get_archetype(archetype3).is_some());
        
        // Remove non-existent archetype
        let fail = master.remove_archetype(999);
        assert!(!fail);
    }
    
    #[test]
    fn test_find_archetypes() {
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);
        
        // Create several archetypes
        let archetype1 = master.create_archetype(&[1, 2]);
        let archetype2 = master.create_archetype(&[2, 3]);
        let archetype3 = master.create_archetype(&[1, 3]);
        let archetype4 = master.create_archetype(&[1, 2, 3]);
        
        // Find archetypes with component 1
        let with_comp1 = master.find_archetypes_with_components(&[1]);
        assert_eq!(with_comp1.len(), 3);
        assert!(with_comp1.contains(&archetype1));
        assert!(with_comp1.contains(&archetype3));
        assert!(with_comp1.contains(&archetype4));
        
        // Find archetypes with components 1 and 2
        let with_comp1_2 = master.find_archetypes_with_components(&[1, 2]);
        assert_eq!(with_comp1_2.len(), 2);
        assert!(with_comp1_2.contains(&archetype1));
        assert!(with_comp1_2.contains(&archetype4));
        
        // Find archetypes with components 1, 2, and 3
        let with_comp1_2_3 = master.find_archetypes_with_components(&[1, 2, 3]);
        assert_eq!(with_comp1_2_3.len(), 1);
        assert!(with_comp1_2_3.contains(&archetype4));
    }
    
    #[test]
    fn test_add_component_to_archetype() {
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);
        
        // Create an archetype with components 1, 2
        let archetype1 = master.create_archetype(&[1, 2]);
        
        // Add component 3
        let archetype2 = master.add_component_to_archetype(archetype1, 3).unwrap();
        
        // Verify the new archetype
        let with_comp3 = master.get_archetype(archetype2).unwrap();
        let component_ids = with_comp3.component_ids();
        
        assert_eq!(component_ids.len(), 3);
        assert!(component_ids.contains(&1));
        assert!(component_ids.contains(&2));
        assert!(component_ids.contains(&3));
        
        // Adding the same component should return the same archetype
        let same_archetype = master.add_component_to_archetype(archetype2, 3).unwrap();
        assert_eq!(same_archetype, archetype2);
    }
    
    #[test]
    fn test_remove_component_from_archetype() {
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);
        
        // Create an archetype with components 1, 2, 3
        let archetype1 = master.create_archetype(&[1, 2, 3]);
        
        // Remove component 3
        let archetype2 = master.remove_component_from_archetype(archetype1, 3).unwrap();
        
        // Verify the new archetype
        let without_comp3 = master.get_archetype(archetype2).unwrap();
        let component_ids = without_comp3.component_ids();
        
        assert_eq!(component_ids.len(), 2);
        assert!(component_ids.contains(&1));
        assert!(component_ids.contains(&2));
        assert!(!component_ids.contains(&3));
        
        // Removing a non-existent component should return the same archetype
        let same_archetype = master.remove_component_from_archetype(archetype2, 3).unwrap();
        assert_eq!(same_archetype, archetype2);
    }
    
    #[test]
    fn test_reuse_existing_archetype() {
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);
        
        // Create an archetype with components 1, 2, 3
        let archetype1 = master.create_archetype(&[1, 2, 3]);
        
        // Create the same archetype again, should get the existing one
        let archetype2 = master.create_archetype(&[1, 2, 3]);
        assert_eq!(archetype1, archetype2);
        
        // Create with different order, should still get the same archetype
        let archetype3 = master.create_archetype(&[3, 1, 2]);
        assert_eq!(archetype1, archetype3);
        
        // Verify we only have one archetype
        assert_eq!(master.archetype_count(), 1);
    }
}