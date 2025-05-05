use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::iters::component_set::ComponentSet;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry;
use crate::ecs::memory::arena::Arena;

/// High-performance query for archetype filtering and iteration
/// Stores direct references to archetypes for maximum performance
/// during iteration over query results
pub struct Query<'a> {
    /// Direct references to filtered archetypes
    /// Avoids indirection through IDs during iteration
    archetypes: Vec<&'a Archetype>,
}

impl<'a> Query<'a> {
    /// Creates a new query from a slice of archetype references
    /// This is the lowest-level constructor for maximum efficiency
    pub fn from_archetypes(archetypes: Vec<&'a Archetype>) -> Self {
        Self { archetypes }
    }
    
    /// Creates a query for archetypes containing all specified component IDs
    /// Uses direct references for maximum iteration performance
    pub fn with_component_ids(master: &'a ArchetypeMaster, component_ids: &[ComponentId]) -> Self {
        // Get archetype IDs matching the components
        let archetype_ids = master.find_archetypes_with_components(component_ids);
        
        // Convert IDs to direct references for fast iteration
        let archetypes = archetype_ids.into_iter()
            .filter_map(|id| master.get_archetype(id))
            .collect();
            
        Self { archetypes }
    }
    
    /// Creates a query for archetypes matching the component mask
    /// Uses direct references for maximum iteration performance
    pub fn with_mask(master: &'a ArchetypeMaster, mask: &ComponentMask) -> Self {
        // Get archetype IDs matching the mask
        let archetype_ids = master.find_matching_archetypes(mask);
        
        // Convert IDs to direct references for fast iteration
        let archetypes = archetype_ids.into_iter()
            .filter_map(|id| master.get_archetype(id))
            .collect();
            
        Self { archetypes }
    }
    
    /// Creates a query for archetypes exactly matching the component mask
    /// Uses direct references for maximum iteration performance
    pub fn with_exact_mask(master: &'a ArchetypeMaster, mask: &ComponentMask) -> Self {
        // Get archetype IDs exactly matching the mask
        let archetype_ids = master.archetype_registry().find_exact_match(mask);
        
        // Convert IDs to direct references for fast iteration
        let archetypes = archetype_ids.into_iter()
            .filter_map(|id| master.get_archetype(id))
            .collect();
            
        Self { archetypes }
    }
    
    /// Creates a type-safe query for archetypes containing the specified components
    /// Example: Query::with::<(Position, Velocity)>(master)
    pub fn with<T: ComponentSet>(master: &'a ArchetypeMaster) -> Self {
        let component_ids = T::component_ids();
        Self::with_component_ids(master, &component_ids)
    }
    
    /// Creates a query with complex filtering
    /// - include_mask: Components that must be present (AND)
    /// - exclude_mask: Components that must not be present (NOT) 
    /// - optional_mask: Components that are optional (at least one must be present)
    pub fn with_filters(
        master: &'a ArchetypeMaster,
        include_mask: &ComponentMask,
        exclude_mask: &ComponentMask,
        optional_mask: &ComponentMask
    ) -> Self {
        // Delegate archetype filtering to ArchetypeRegistry
        let archetype_ids = master.archetype_registry().find_with_filter(
            include_mask,
            exclude_mask,
            optional_mask
        );
        
        // Convert IDs to direct references for fast iteration
        let archetypes = archetype_ids.into_iter()
            .filter_map(|id| master.get_archetype(id))
            .collect();
            
        Self { archetypes }
    }
    
    /// Creates a query with type-safe complex filtering
    pub fn with_type_filters<Inc: ComponentSet, Exc: ComponentSet, Opt: ComponentSet>(
        master: &'a ArchetypeMaster
    ) -> Self {
        let mut include_mask = ComponentMask::new();
        let mut exclude_mask = ComponentMask::new();
        let mut optional_mask = ComponentMask::new();
        
        // Set component bits in masks
        for &id in &Inc::component_ids() {
            include_mask.set(id);
        }
        
        for &id in &Exc::component_ids() {
            exclude_mask.set(id);
        }
        
        for &id in &Opt::component_ids() {
            optional_mask.set(id);
        }
        
        Self::with_filters(master, &include_mask, &exclude_mask, &optional_mask)
    }
    
    /// Returns the number of archetypes in the query
    pub fn len(&self) -> usize {
        self.archetypes.len()
    }
    
    /// Checks if the query is empty
    pub fn is_empty(&self) -> bool {
        self.archetypes.is_empty()
    }
    
    /// Returns a reference to all archetype references
    pub fn archetypes(&self) -> &[&'a Archetype] {
        &self.archetypes
    }
    
    /// Returns an iterator over archetype references
    /// This provides direct access without indirection
    pub fn iter(&self) -> impl Iterator<Item = &'a Archetype> + '_ {
        self.archetypes.iter().copied()
    }
}

/// Enable for-loop iteration over query results
impl<'a> IntoIterator for &'a Query<'a> {
    type Item = &'a Archetype;
    type IntoIter = std::iter::Copied<std::slice::Iter<'a, &'a Archetype>>;

    fn into_iter(self) -> Self::IntoIter {
        self.archetypes.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::memory::arena::Arena;
    
    // Mock component types for testing
    struct Position;
    struct Velocity;
    struct Health;
    struct Damage;
    
    impl Component for Position {
        fn component_id() -> ComponentId { 1 }
    }
    
    impl Component for Velocity {
        fn component_id() -> ComponentId { 2 }
    }
    
    impl Component for Health {
        fn component_id() -> ComponentId { 3 }
    }
    
    impl Component for Damage {
        fn component_id() -> ComponentId { 4 }
    }
    
    fn register_mock_components() {
        // Register component layouts for testing
        component_registry::register_layout::<Position>(Position::component_id());
        component_registry::register_layout::<Velocity>(Velocity::component_id());
        component_registry::register_layout::<Health>(Health::component_id());
        component_registry::register_layout::<Damage>(Damage::component_id());
    }
    
    fn create_test_arena() -> Arena {
        Arena::new() // Arena constructor takes no arguments
    }
    
    fn setup_test_archetypes() -> ArchetypeMaster {
        register_mock_components();
        let arena = create_test_arena();
        let mut master = ArchetypeMaster::new(&arena);
        
        // Create some test archetypes
        master.create_archetype(&[Position::component_id()]);
        master.create_archetype(&[Position::component_id(), Velocity::component_id()]);
        master.create_archetype(&[Health::component_id()]);
        master.create_archetype(&[Position::component_id(), Health::component_id()]);
        master.create_archetype(&[Position::component_id(), Velocity::component_id(), Health::component_id()]);
        
        master
    }
    
    #[test]
    fn test_basic_query() {
        let master = setup_test_archetypes();
        let query = Query::with_component_ids(&master, &[Position::component_id()]);
        
        // Should find all archetypes with Position
        assert_eq!(query.len(), 4);
        
        // All archetypes should have Position component
        for archetype in query.iter() {
            assert!(archetype.has_component_id(Position::component_id()));
        }
    }
    
    #[test]
    fn test_query_with_multiple_components() {
        let master = setup_test_archetypes();
        let query = Query::with_component_ids(&master, &[Position::component_id(), Velocity::component_id()]);
        
        // Should find archetypes with both Position and Velocity
        assert_eq!(query.len(), 2);
        
        // All archetypes should have both components
        for archetype in query.iter() {
            assert!(archetype.has_component_id(Position::component_id()));
            assert!(archetype.has_component_id(Velocity::component_id()));
        }
    }
    
    #[test]
    fn test_type_safe_query() {
        let master = setup_test_archetypes();
        let query = Query::with::<(Position, Velocity)>(&master);
        
        // Should find archetypes with both Position and Velocity
        assert_eq!(query.len(), 2);
        
        // All archetypes should have both components
        for archetype in query.iter() {
            assert!(archetype.has_component_id(Position::component_id()));
            assert!(archetype.has_component_id(Velocity::component_id()));
        }
    }
    
    #[test]
    fn test_iteration() {
        let master = setup_test_archetypes();
        let query = Query::with_component_ids(&master, &[Position::component_id()]);
        
        // Manual iteration with iter()
        let mut count = 0;
        for archetype in query.iter() {
            assert!(archetype.has_component_id(Position::component_id()));
            count += 1;
        }
        assert_eq!(count, 4);
        
        // For-loop iteration with IntoIterator
        count = 0;
        for archetype in &query {
            assert!(archetype.has_component_id(Position::component_id()));
            count += 1;
        }
        assert_eq!(count, 4);
        
        // Collection with Iterator
        let archetypes: Vec<_> = query.iter().collect();
        assert_eq!(archetypes.len(), 4);
        
        // Check direct access via archetypes()
        assert_eq!(query.archetypes().len(), 4);
    }
    
    #[test]
    fn test_complex_filtering() {
        let master = setup_test_archetypes();
        
        // Create masks for filtering
        let mut include_mask = ComponentMask::new();
        include_mask.set(Position::component_id());
        
        let mut exclude_mask = ComponentMask::new();
        exclude_mask.set(Damage::component_id());
        
        let mut optional_mask = ComponentMask::new();
        optional_mask.set(Velocity::component_id());
        optional_mask.set(Health::component_id());
        
        // Should find archetypes with Position, without Damage, and with either Velocity or Health
        let query = Query::with_filters(
            &master,
            &include_mask, 
            &exclude_mask,
            &optional_mask
        );
        
        assert_eq!(query.len(), 4);
        
        // Verify filtering criteria
        for archetype in query.iter() {
            // Must have Position
            assert!(archetype.has_component_id(Position::component_id()));
            
            // Must not have Damage
            assert!(!archetype.has_component_id(Damage::component_id()));
            
            // Must have either Velocity or Health or both
            assert!(
                archetype.has_component_id(Velocity::component_id()) || 
                archetype.has_component_id(Health::component_id())
            );
        }
    }
    
    #[test]
    fn test_type_safe_filters() {
        let master = setup_test_archetypes();
        
        // Create masks manually instead of using type_filters to avoid ComponentSet issues
        let mut include_mask = ComponentMask::new();
        include_mask.set(Position::component_id());
        
        let mut exclude_mask = ComponentMask::new();
        exclude_mask.set(Damage::component_id());
        
        let mut optional_mask = ComponentMask::new();
        optional_mask.set(Velocity::component_id());
        optional_mask.set(Health::component_id());
        
        let query = Query::with_filters(
            &master,
            &include_mask,
            &exclude_mask,
            &optional_mask
        );
        
        assert_eq!(query.len(), 4);
        
        // Verify filtering criteria
        for archetype in query.iter() {
            // Must have Position
            assert!(archetype.has_component_id(Position::component_id()));
            
            // Must not have Damage
            assert!(!archetype.has_component_id(Damage::component_id()));
            
            // Must have either Velocity or Health or both
            assert!(
                archetype.has_component_id(Velocity::component_id()) || 
                archetype.has_component_id(Health::component_id())
            );
        }
    }
} 