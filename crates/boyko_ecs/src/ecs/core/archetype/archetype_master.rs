use std::ptr::NonNull;
use crate::ecs::core::archetype::archetype_bundle::ArchetypeBundle;
use crate::ecs::core::archetype::archetype_registry::ArchetypeRegistry;
use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use crate::ecs::memory::arena::Arena;
use crate::ecs::core::iters::query::Query;

use crate::ecs::core::component::component_registry;

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
        
        // SAFETY: `self.arena` was captured from the `Box<Arena>` owned by
        // `EcsMaster` (audit C-001). The `Box` has a stable heap address and
        // outlives every `ArchetypeMaster`/`Archetype` that holds the
        // `NonNull`. Arena is `!Send + !Sync`, so no concurrent reference
        // can exist. We do not let `&Arena` escape `add_archetype_from_components`.
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
    
    /// Find archetypes with complex filtering criteria (include, exclude, optional components)
    pub fn find_archetypes_with_filter(
        &self,
        include_mask: &ComponentMask,
        exclude_mask: &ComponentMask,
        optional_mask: &ComponentMask
    ) -> Vec<ArchetypeId> {
        self.registry.find_with_filter(include_mask, exclude_mask, optional_mask)
    }
    
    /// Find archetypes with components that can be included, excluded, or optional
    pub fn find_archetypes_with_component_filter(
        &self,
        include_components: &[ComponentId],
        exclude_components: &[ComponentId],
        optional_components: &[ComponentId]
    ) -> Vec<ArchetypeId> {
        self.registry.find_with_component_filter(
            include_components,
            exclude_components,
            optional_components
        )
    }
    
    /// Get references to archetypes with complex filtering
    /// Returns direct references to archetypes for faster access
    pub fn get_archetypes_with_filter(
        &self,
        include_mask: &ComponentMask,
        exclude_mask: &ComponentMask,
        optional_mask: &ComponentMask
    ) -> Vec<&Archetype> {
        let archetype_ids = self.find_archetypes_with_filter(include_mask, exclude_mask, optional_mask);
        archetype_ids.into_iter()
            .filter_map(|id| self.get_archetype(id))
            .collect()
    }
    
    /// Get references to archetypes with component filtering
    /// Returns direct references to archetypes for faster access
    pub fn get_archetypes_with_component_filter(
        &self,
        include_components: &[ComponentId],
        exclude_components: &[ComponentId],
        optional_components: &[ComponentId]
    ) -> Vec<&Archetype> {
        let archetype_ids = self.find_archetypes_with_component_filter(
            include_components,
            exclude_components,
            optional_components
        );
        archetype_ids.into_iter()
            .filter_map(|id| self.get_archetype(id))
            .collect()
    }
    
    /// Get references to archetypes matching a simple component set
    /// Returns direct references to archetypes for faster access
    pub fn get_archetypes_with_components(&self, component_ids: &[ComponentId]) -> Vec<&Archetype> {
        let archetype_ids = self.find_archetypes_with_components(component_ids);
        archetype_ids.into_iter()
            .filter_map(|id| self.get_archetype(id))
            .collect()
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
        let mut new_components: Vec<ComponentId> = source_archetype.component_ids()
            .iter()
            .filter(|&&c| c != component_id)
            .copied()
            .collect();
        
        // If no components were removed, return the source archetype
        if new_components.len() == source_archetype.component_ids().len() {
            return Some(source_archetype_id);
        }
        
        // Create or get the new archetype
        Some(self.get_or_create_archetype(&new_components))
    }
    
    /// Returns a reference to the internal archetype bundle
    pub fn archetype_bundle(&self) -> &ArchetypeBundle {
        &self.archetypes
    }
    
    /// Returns a mutable reference to the internal archetype bundle
    pub fn archetype_bundle_mut(&mut self) -> &mut ArchetypeBundle {
        &mut self.archetypes
    }
    
    /// Returns a reference to the internal archetype registry
    pub fn archetype_registry(&self) -> &ArchetypeRegistry {
        &self.registry
    }
    
    /// Returns a mutable reference to the internal archetype registry
    pub fn archetype_registry_mut(&mut self) -> &mut ArchetypeRegistry {
        &mut self.registry
    }
    
    /// Creates a new query for archetypes containing all specified component IDs
    pub fn query_with_components<'a>(&'a self, component_ids: &[ComponentId]) -> Query<'a> {
        Query::with_component_ids(self, component_ids)
    }
    
    /// Creates a new query for archetypes matching the component mask
    pub fn query_with_mask<'a>(&'a self, mask: &ComponentMask) -> Query<'a> {
        Query::with_mask(self, mask)
    }
    
    /// Creates a new query for archetypes exactly matching the component mask
    pub fn query_with_exact_mask<'a>(&'a self, mask: &ComponentMask) -> Query<'a> {
        Query::with_exact_mask(self, mask)
    }
    
    /// Creates a type-safe query for archetypes containing the specified components
    /// Example: master.query::<(Position, Velocity)>()
    pub fn query<'a, T: crate::ecs::core::iters::component_set::ComponentSet>(&'a self) -> Query<'a> {
        Query::with::<T>(self)
    }
    
    /// Creates a query with complex filtering criteria
    /// - include_mask: Components that must be present (AND)
    /// - exclude_mask: Components that must not be present (NOT)
    /// - optional_mask: Components that are optional (at least one must be present)
    pub fn query_with_filters<'a>(
        &'a self,
        include_mask: &ComponentMask,
        exclude_mask: &ComponentMask,
        optional_mask: &ComponentMask
    ) -> Query<'a> {
        Query::with_filters(self, include_mask, exclude_mask, optional_mask)
    }
    
    /// Creates a type-safe query with complex filtering
    pub fn query_with_type_filters<'a, Inc: crate::ecs::core::iters::component_set::ComponentSet, 
                                     Exc: crate::ecs::core::iters::component_set::ComponentSet, 
                                     Opt: crate::ecs::core::iters::component_set::ComponentSet>(
        &'a self
    ) -> Query<'a> {
        Query::with_type_filters::<Inc, Exc, Opt>(self)
    }
    
    /// Returns an iterator over all archetypes
    pub fn iter_archetypes(&self) -> impl Iterator<Item = &Archetype> {
        self.archetypes.iter()
    }
    
    /// Clears all archetypes
    pub fn clear(&mut self) {
        self.archetypes = ArchetypeBundle::new();
        self.registry.clear();
        self.next_archetype_id = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::memory::arena::Arena;
    
    fn create_test_arena() -> Arena {
        Arena::new()
    }
    
    // Each test module owns its own ComponentId range to avoid `OnceLock`
    // collisions across tests (see audit C-003 / Phase 1b). `archetype_master`
    // uses 300-309. All mock components share the same backing type (`u32`)
    // because the tests only exercise mask logic, never byte-level layout.
    const MOCK_ID_BASE: ComponentId = 300;

    /// Translate a test-local "logical" ID (1..=8) into the actual
    /// `ComponentId` used in the registry.
    #[inline]
    fn mock(local: ComponentId) -> ComponentId {
        MOCK_ID_BASE + local
    }

    /// Translate a slice of logical IDs into actual ComponentIds — keeps the
    /// test bodies readable (`master.create_archetype(&mocks(&[1, 2, 3]))`).
    fn mocks<const N: usize>(local: [ComponentId; N]) -> [ComponentId; N] {
        local.map(mock)
    }

    /// Register mock components for testing
    fn register_mock_components() {
        // Register `u32` under each test-local ID. `OnceLock::set` is
        // idempotent: re-registration after the first call is a no-op, so
        // running this from every test is safe.
        for local in 1..=8 {
            component_registry::register_layout::<u32>(mock(local));
        }
    }
    
    #[test]
    fn test_create_archetype() {
        register_mock_components();
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);

        // Create a new archetype
        let id1 = master.create_archetype(&mocks([1, 2, 3]));
        assert_eq!(id1, 1);

        // Create another archetype
        let id2 = master.create_archetype(&mocks([1, 2]));
        assert_eq!(id2, 2);

        // Try to create an archetype with the same components - should return existing ID
        let id3 = master.create_archetype(&mocks([1, 2, 3]));
        assert_eq!(id3, id1); // Should return the ID of the first archetype

        // Verify both archetypes exist
        assert!(master.get_archetype(id1).is_some());
        assert!(master.get_archetype(id2).is_some());
    }

    #[test]
    fn test_remove_archetype() {
        register_mock_components();
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);

        // Create a new archetype
        let id = master.create_archetype(&mocks([1, 2, 3]));
        assert!(master.get_archetype(id).is_some());

        // Remove the archetype
        let result = master.remove_archetype(id);
        assert!(result);

        // Verify the archetype doesn't exist anymore
        assert!(master.get_archetype(id).is_none());

        // Try to remove a non-existent archetype
        let result = master.remove_archetype(999);
        assert!(!result);
    }

    #[test]
    fn test_find_archetypes() {
        register_mock_components();
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);

        // Create different archetypes
        let id1 = master.create_archetype(&mocks([1, 2, 3]));
        let id2 = master.create_archetype(&mocks([1, 2]));
        let id3 = master.create_archetype(&mocks([2, 3]));

        // Find archetypes with component 1
        let results = master.find_archetypes_with_components(&mocks([1]));
        assert_eq!(results.len(), 2);
        assert!(results.contains(&id1));
        assert!(results.contains(&id2));

        // Find archetypes with components 2 and 3
        let results = master.find_archetypes_with_components(&mocks([2, 3]));
        assert_eq!(results.len(), 2);
        assert!(results.contains(&id1));
        assert!(results.contains(&id3));

        // Find archetypes with components 1, 2, and 3
        let results = master.find_archetypes_with_components(&mocks([1, 2, 3]));
        assert_eq!(results.len(), 1);
        assert!(results.contains(&id1));
    }

    #[test]
    fn test_add_component_to_archetype() {
        register_mock_components();
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);

        // Create an archetype with components 1 and 2
        let id1 = master.create_archetype(&mocks([1, 2]));

        // Add component 3 to the archetype
        let id2 = master.add_component_to_archetype(id1, mock(3)).unwrap();

        // The new archetype should have components 1, 2, and 3
        let archetype = master.get_archetype(id2).unwrap();
        assert!(archetype.has_component_id(mock(1)));
        assert!(archetype.has_component_id(mock(2)));
        assert!(archetype.has_component_id(mock(3)));

        // Adding a component that already exists should return the same archetype
        let id3 = master.add_component_to_archetype(id2, mock(2)).unwrap();
        assert_eq!(id3, id2);
    }

    #[test]
    fn test_remove_component_from_archetype() {
        register_mock_components();
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);

        // Create an archetype with components 1, 2, and 3
        let id1 = master.create_archetype(&mocks([1, 2, 3]));

        // Remove component 3 from the archetype
        let id2 = master.remove_component_from_archetype(id1, mock(3)).unwrap();

        // The new archetype should have only components 1 and 2
        let archetype = master.get_archetype(id2).unwrap();
        assert!(archetype.has_component_id(mock(1)));
        assert!(archetype.has_component_id(mock(2)));
        assert!(!archetype.has_component_id(mock(3)));

        // Removing a component that doesn't exist should return the same archetype
        let id3 = master.remove_component_from_archetype(id2, mock(3)).unwrap();
        assert_eq!(id3, id2);
    }

    #[test]
    fn test_reuse_existing_archetype() {
        register_mock_components();
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);

        // Create an archetype with components 1, 2, and 3
        let id1 = master.create_archetype(&mocks([1, 2, 3]));

        // Create an archetype with components 1 and 2
        let id2 = master.create_archetype(&mocks([1, 2]));

        // Add component 3 to the second archetype, which should result in
        // reusing the first archetype
        let id3 = master.add_component_to_archetype(id2, mock(3)).unwrap();
        assert_eq!(id3, id1);
    }

    #[test]
    fn test_get_archetypes_with_filter() {
        register_mock_components();
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);

        // Create different archetypes
        master.create_archetype(&mocks([1, 2]));          // Position, Velocity
        master.create_archetype(&mocks([1, 3]));          // Position, Health
        master.create_archetype(&mocks([2, 4]));          // Velocity, Damage
        master.create_archetype(&mocks([1, 2, 3]));       // Position, Velocity, Health
        master.create_archetype(&mocks([1, 2, 4]));       // Position, Velocity, Damage

        // Filter: Position AND Velocity, but NOT Damage
        let mut include_mask = ComponentMask::new();
        include_mask.set(mock(1));  // Position
        include_mask.set(mock(2));  // Velocity

        let mut exclude_mask = ComponentMask::new();
        exclude_mask.set(mock(4));  // Damage

        let optional_mask = ComponentMask::new();

        // Get archetypes with references
        let archetypes = master.get_archetypes_with_filter(
            &include_mask,
            &exclude_mask,
            &optional_mask
        );

        // Should match [Position, Velocity] and [Position, Velocity, Health]
        assert_eq!(archetypes.len(), 2);

        // Verify components
        for archetype in archetypes {
            assert!(archetype.has_component_id(mock(1)));  // Position
            assert!(archetype.has_component_id(mock(2)));  // Velocity
            assert!(!archetype.has_component_id(mock(4))); // Not Damage
        }
    }

    #[test]
    fn test_get_archetypes_with_component_filter() {
        register_mock_components();
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);

        // Create different archetypes
        master.create_archetype(&mocks([1, 2]));          // Position, Velocity
        master.create_archetype(&mocks([1, 3]));          // Position, Health
        master.create_archetype(&mocks([2, 4]));          // Velocity, Damage
        master.create_archetype(&mocks([1, 2, 3]));       // Position, Velocity, Health
        master.create_archetype(&mocks([1, 2, 4]));       // Position, Velocity, Damage

        // Filter: Position AND at least one of [Health, Damage]
        let include = mocks([1]);                // Position
        let exclude: [ComponentId; 0] = [];
        let optional = mocks([3, 4]);           // Health or Damage

        // Get archetypes with references
        let archetypes = master.get_archetypes_with_component_filter(
            &include,
            &exclude,
            &optional
        );

        // Should match [Position, Health], [Position, Velocity, Health], [Position, Velocity, Damage]
        assert_eq!(archetypes.len(), 3);

        // Verify components
        for archetype in archetypes {
            assert!(archetype.has_component_id(mock(1)));  // Position

            // At least one of Health or Damage
            assert!(archetype.has_component_id(mock(3)) || archetype.has_component_id(mock(4)));
        }
    }
}
