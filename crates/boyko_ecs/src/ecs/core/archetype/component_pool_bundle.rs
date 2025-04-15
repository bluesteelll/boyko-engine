use std::ops::{Index, IndexMut};
use crate::ecs::core::component::Component;
use crate::ecs::identifiers::primitives::{ComponentId, InlandComponentId, InlandPoolId};
use crate::ecs::memory::component_pool::ComponentPool;
use crate::ecs::memory::arena::Arena;
use boyko_utils::sparse_map::sparse_map::SparseMap;

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

    /// Type-safe version to add components for an entity
    /// Takes any number of components that implement Component trait
    /// Returns a vector of indices in the same order as the components
    pub fn add_entity<Components: ComponentTuple>(&mut self, components: Components) -> Option<Vec<usize>> {
        components.add_to_pool_bundle(self)
    }

    /// Removes components from all pools using indices
    /// Each index corresponds to the component in the respective pool
    /// Returns true if all components were successfully removed
    pub fn remove_entity(&mut self, indices: &[usize]) -> bool {
        if indices.len() != self.pools.len() {
            return false; // Number of indices doesn't match number of pools
        }

        let mut success = true;

        // Remove components from each pool using their respective indices
        for (pool_idx, &component_idx) in indices.iter().enumerate() {
            success &= self.pools[pool_idx].swap_remove(component_idx);
        }

        success
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

/// Trait to support variadic component adding
pub trait ComponentTuple {
    fn add_to_pool_bundle(self, bundle: &mut ComponentPoolBundle) -> Option<Vec<usize>>;
}

// Implement for empty tuple (no components)
impl ComponentTuple for () {
    fn add_to_pool_bundle(self, _bundle: &mut ComponentPoolBundle) -> Option<Vec<usize>> {
        Some(vec![])
    }
}

// Implement for single component
impl<T: Component> ComponentTuple for T {
    fn add_to_pool_bundle(self, bundle: &mut ComponentPoolBundle) -> Option<Vec<usize>> {
        if let Some(pool) = bundle.get_pool_mut::<T>() {
            pool.add(self).map(|id| vec![id])
        } else {
            None
        }
    }
}

// Implement for tuple of two components
impl<T1: Component, T2: Component> ComponentTuple for (T1, T2) {
    fn add_to_pool_bundle(self, bundle: &mut ComponentPoolBundle) -> Option<Vec<usize>> {
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
    fn add_to_pool_bundle(self, bundle: &mut ComponentPoolBundle) -> Option<Vec<usize>> {
        if let Some(pool) = bundle.get_pool_mut::<T>() {
            pool.add(self.0).map(|id| vec![id])
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::Component;
    use crate::ecs::memory::arena::Arena;
    use std::any::TypeId;

    // Define test component types
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct PositionComponent {
        x: f32,
        y: f32,
    }

    impl Component for PositionComponent {
        #[inline(always)]
        fn component_id() -> usize {
            1 // Static ID for testing
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct VelocityComponent {
        dx: f32,
        dy: f32,
    }

    impl Component for VelocityComponent {
        #[inline(always)]
        fn component_id() -> usize {
            2 // Static ID for testing
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct HealthComponent {
        current: i32,
        max: i32,
    }

    impl Component for HealthComponent {
        #[inline(always)]
        fn component_id() -> usize {
            3 // Static ID for testing
        }
    }

    // Helper function to create a bundle with some predefined pools
    fn setup_bundle() -> (Arena, ComponentPoolBundle) {
        let arena = Arena::new();
        let mut bundle = ComponentPoolBundle::new();
        
        bundle.add_pool::<PositionComponent>(&arena);
        bundle.add_pool::<VelocityComponent>(&arena);
        
        (arena, bundle)
    }

    #[test]
    fn test_create_bundle() {
        let (_, bundle) = setup_bundle();
        
        assert_eq!(bundle.len(), 2);
        assert!(!bundle.is_empty());
        assert!(bundle.contains::<PositionComponent>());
        assert!(bundle.contains::<VelocityComponent>());
        assert!(!bundle.contains::<HealthComponent>());
        
        assert!(bundle.contains_id(PositionComponent::component_id()));
        assert!(bundle.contains_id(VelocityComponent::component_id()));
        assert!(!bundle.contains_id(HealthComponent::component_id()));
    }

    #[test]
    fn test_with_capacity() {
        let arena = Arena::new();
        let bundle = ComponentPoolBundle::with_capacity(10);
        
        assert_eq!(bundle.len(), 0);
        assert!(bundle.is_empty());
        
        // Verify the capacity by adding pools
        let mut bundle = bundle;
        for i in 0..10 {
            assert_eq!(bundle.len(), i);
            bundle.add_pool::<PositionComponent>(&arena);
        }
        
        assert_eq!(bundle.len(), 10);
    }

    #[test]
    fn test_with_component_chaining() {
        let arena = Arena::new();
        let bundle = ComponentPoolBundle::new()
            .with_component::<PositionComponent>(&arena)
            .with_component::<VelocityComponent>(&arena);
        
        assert_eq!(bundle.len(), 2);
        assert!(bundle.contains::<PositionComponent>());
        assert!(bundle.contains::<VelocityComponent>());
    }

    #[test]
    fn test_get_pool() {
        let (arena, bundle) = setup_bundle();
        
        // Test getting pools by type
        let pos_pool = bundle.get_pool::<PositionComponent>();
        assert!(pos_pool.is_some());
        assert_eq!(pos_pool.unwrap().component_id(), PositionComponent::component_id());
        
        let vel_pool = bundle.get_pool::<VelocityComponent>();
        assert!(vel_pool.is_some());
        assert_eq!(vel_pool.unwrap().component_id(), VelocityComponent::component_id());
        
        let health_pool = bundle.get_pool::<HealthComponent>();
        assert!(health_pool.is_none());
        
        // Test getting pools by ID
        let pos_pool_by_id = bundle.get_pool_by_id(PositionComponent::component_id());
        assert!(pos_pool_by_id.is_some());
        
        let unknown_pool = bundle.get_pool_by_id(999);
        assert!(unknown_pool.is_none());
    }

    #[test]
    fn test_get_pool_mut() {
        let (arena, mut bundle) = setup_bundle();
        
        // Test getting mutable pools by type
        let pos_pool = bundle.get_pool_mut::<PositionComponent>();
        assert!(pos_pool.is_some());
        
        let vel_pool = bundle.get_pool_mut::<VelocityComponent>();
        assert!(vel_pool.is_some());
        
        let health_pool = bundle.get_pool_mut::<HealthComponent>();
        assert!(health_pool.is_none());
        
        // Test getting mutable pools by ID
        let pos_pool_by_id = bundle.get_pool_mut_by_id(PositionComponent::component_id());
        assert!(pos_pool_by_id.is_some());
        
        let unknown_pool = bundle.get_pool_mut_by_id(999);
        assert!(unknown_pool.is_none());
    }

    #[test]
    fn test_add_entity_single_component() {
        let (arena, mut bundle) = setup_bundle();
        
        // Add a position component
        let position = PositionComponent { x: 10.0, y: 20.0 };
        let indices = bundle.add_entity(position);
        
        assert!(indices.is_some());
        let indices = indices.unwrap();
        assert_eq!(indices.len(), 1);
        
        // Verify the component was added correctly
        let pos_pool = bundle.get_pool::<PositionComponent>().unwrap();
        let retrieved_pos = pos_pool.get::<PositionComponent>(indices[0]).unwrap();
        assert_eq!(retrieved_pos.x, 10.0);
        assert_eq!(retrieved_pos.y, 20.0);
    }

    #[test]
    fn test_add_entity_multiple_components() {
        let (arena, mut bundle) = setup_bundle();
        
        // Add position and velocity components
        let position = PositionComponent { x: 10.0, y: 20.0 };
        let velocity = VelocityComponent { dx: 1.0, dy: 2.0 };
        
        let indices = bundle.add_entity((position, velocity));
        
        assert!(indices.is_some());
        let indices = indices.unwrap();
        assert_eq!(indices.len(), 2);
        
        // Verify the components were added correctly
        let pos_pool = bundle.get_pool::<PositionComponent>().unwrap();
        let retrieved_pos = pos_pool.get::<PositionComponent>(indices[0]).unwrap();
        assert_eq!(retrieved_pos.x, 10.0);
        assert_eq!(retrieved_pos.y, 20.0);
        
        let vel_pool = bundle.get_pool::<VelocityComponent>().unwrap();
        let retrieved_vel = vel_pool.get::<VelocityComponent>(indices[1]).unwrap();
        assert_eq!(retrieved_vel.dx, 1.0);
        assert_eq!(retrieved_vel.dy, 2.0);
    }

    #[test]
    fn test_add_entity_components() {
        let (arena, mut bundle) = setup_bundle();
        
        // Create raw components
        let position = PositionComponent { x: 10.0, y: 20.0 };
        let velocity = VelocityComponent { dx: 1.0, dy: 2.0 };
        
        // Add components using raw pointers
        let components = vec![
            (PositionComponent::component_id(), &position as *const _ as *const u8),
            (VelocityComponent::component_id(), &velocity as *const _ as *const u8),
        ];
        
        let indices = bundle.add_entity_components(components);
        
        assert!(indices.is_some());
        let indices = indices.unwrap();
        assert_eq!(indices.len(), 2);
        
        // Verify the components were added correctly
        let pos_pool = bundle.get_pool::<PositionComponent>().unwrap();
        let retrieved_pos = pos_pool.get::<PositionComponent>(indices[0]).unwrap();
        assert_eq!(retrieved_pos.x, 10.0);
        assert_eq!(retrieved_pos.y, 20.0);
        
        let vel_pool = bundle.get_pool::<VelocityComponent>().unwrap();
        let retrieved_vel = vel_pool.get::<VelocityComponent>(indices[1]).unwrap();
        assert_eq!(retrieved_vel.dx, 1.0);
        assert_eq!(retrieved_vel.dy, 2.0);
    }

    #[test]
    fn test_add_entity_components_type_mismatch() {
        let (arena, mut bundle) = setup_bundle();
        
        // Create components with mismatched types
        let position = PositionComponent { x: 10.0, y: 20.0 };
        let velocity = VelocityComponent { dx: 1.0, dy: 2.0 };
        
        // Add components with incorrect component IDs
        let components = vec![
            (VelocityComponent::component_id(), &position as *const _ as *const u8), // Type mismatch
            (PositionComponent::component_id(), &velocity as *const _ as *const u8), // Type mismatch
        ];
        
        let indices = bundle.add_entity_components(components);
        
        // This should fail due to type mismatch
        assert!(indices.is_none());
        
        // Verify no components were added
        assert_eq!(bundle.get_pool::<PositionComponent>().unwrap().count(), 0);
        assert_eq!(bundle.get_pool::<VelocityComponent>().unwrap().count(), 0);
    }

    #[test]
    fn test_add_entity_with_missing_pools() {
        let arena = Arena::new();
        let mut bundle = ComponentPoolBundle::new();
        bundle.add_pool::<PositionComponent>(&arena);
        
        // Try to add an entity with a component that doesn't have a pool
        let velocity = VelocityComponent { dx: 1.0, dy: 2.0 };
        let indices = bundle.add_entity(velocity);
        
        // This should fail because there's no VelocityComponent pool
        assert!(indices.is_none());
    }

    #[test]
    fn test_remove_entity() {
        let (arena, mut bundle) = setup_bundle();
        
        // Add an entity
        let position = PositionComponent { x: 10.0, y: 20.0 };
        let velocity = VelocityComponent { dx: 1.0, dy: 2.0 };
        
        let indices = bundle.add_entity((position, velocity)).unwrap();
        
        // Remove the entity
        let success = bundle.remove_entity(&indices);
        assert!(success);
        
        // Verify the components were removed
        let pos_pool = bundle.get_pool::<PositionComponent>().unwrap();
        assert_eq!(pos_pool.count(), 0);
        
        let vel_pool = bundle.get_pool::<VelocityComponent>().unwrap();
        assert_eq!(vel_pool.count(), 0);
    }

    #[test]
    fn test_remove_entity_with_invalid_indices() {
        let (arena, mut bundle) = setup_bundle();
       // Try to remove an entity with invalid indices
        let invalid_indices = vec![999, 999];
        let success = bundle.remove_entity(&invalid_indices);
        
        // This should fail because the indices are invalid
        assert!(!success);
    }

    #[test]
    fn test_remove_entity_with_mismatched_indices_count() {
        let (arena, mut bundle) = setup_bundle();
        
        // Try to remove an entity with too few indices
        let invalid_indices = vec![0]; // Only one index, but we have two pools
        let success = bundle.remove_entity(&invalid_indices);
        
        // This should fail because the number of indices doesn't match the number of pools
        assert!(!success);
    }

    #[test]
    fn test_multiple_entities() {
    let (arena, mut bundle) = setup_bundle();
    
    // Add multiple entities
    let pos1 = PositionComponent { x: 10.0, y: 20.0 };
    let vel1 = VelocityComponent { dx: 1.0, dy: 2.0 };
    
    let pos2 = PositionComponent { x: 30.0, y: 40.0 };
    let vel2 = VelocityComponent { dx: 3.0, dy: 4.0 };
    
    let indices1 = bundle.add_entity((pos1, vel1)).unwrap();
    let indices2 = bundle.add_entity((pos2, vel2)).unwrap();
    
    // Check pool counts in a separate scope to end the borrow
    {
        let pos_pool = bundle.get_pool::<PositionComponent>().unwrap();
        let vel_pool = bundle.get_pool::<VelocityComponent>().unwrap();
        assert_eq!(pos_pool.count(), 2);
        assert_eq!(vel_pool.count(), 2);
    } // Immutable borrows end here
    
    // Now we can mutably borrow the bundle
    let success = bundle.remove_entity(&indices1);
    assert!(success);
    
    // Create new borrows after the mutable operation
    {
        let pos_pool = bundle.get_pool::<PositionComponent>().unwrap();
        let vel_pool = bundle.get_pool::<VelocityComponent>().unwrap();
        
        // Verify counts after removal
        assert_eq!(pos_pool.count(), 1);
        assert_eq!(vel_pool.count(), 1);
        
        // Check the remaining components
        let remaining_pos = pos_pool.get::<PositionComponent>(indices2[0]).unwrap();
        assert_eq!(remaining_pos.x, 30.0);
        assert_eq!(remaining_pos.y, 40.0);
        
        let remaining_vel = vel_pool.get::<VelocityComponent>(indices2[1]).unwrap();
        assert_eq!(remaining_vel.dx, 3.0);
        assert_eq!(remaining_vel.dy, 4.0);
    }
    }

    #[test]
    fn test_iterator_methods() {
        let (arena, mut bundle) = setup_bundle();
        
        // Test iterators
        let pool_count = bundle.iter().count();
        assert_eq!(pool_count, 2);
        
        // Modify pools through iterator
        for pool in bundle.iter_mut() {
            // Just verify we can iterate mutably
            assert!(pool.count() == 0);
        }
    }

    #[test]
    fn test_index_operators() {
        let (arena, bundle) = setup_bundle();
        
        // Get the inland IDs for the pools
        let pos_id = bundle.sparse_indexes.get(PositionComponent::component_id()).unwrap();
        let vel_id = bundle.sparse_indexes.get(VelocityComponent::component_id()).unwrap();
        
        // Test index operator access
        let pos_pool = &bundle[*pos_id];
        assert_eq!(pos_pool.component_id(), PositionComponent::component_id());
        
        let vel_pool = &bundle[*vel_id];
        assert_eq!(vel_pool.component_id(), VelocityComponent::component_id());
    }

    #[test]
    fn test_rollback_on_partial_failure() {
        let arena = Arena::new();
        let mut bundle = ComponentPoolBundle::new();
        
        // Add pools with limited capacity for testing
        let pos_pool = ComponentPool::new::<PositionComponent>(&arena, 1, 1); // Only 1 component capacity
        let vel_pool = ComponentPool::new::<VelocityComponent>(&arena, 1, 1); // Only 1 component capacity
        
        let pos_id = PositionComponent::component_id();
        let vel_id = VelocityComponent::component_id();
        
        bundle.pools.push(pos_pool);
        bundle.pools.push(vel_pool);
        bundle.sparse_indexes.insert(pos_id, 0);
        bundle.sparse_indexes.insert(vel_id, 1);
        
        // Add a first entity successfully
        let pos1 = PositionComponent { x: 10.0, y: 20.0 };
        let vel1 = VelocityComponent { dx: 1.0, dy: 2.0 };
        let indices1 = bundle.add_entity((pos1, vel1)).unwrap();
        
        // Try to add a second entity - should fail and rollback
        let pos2 = PositionComponent { x: 30.0, y: 40.0 };
        let vel2 = VelocityComponent { dx: 3.0, dy: 4.0 };
        let indices2 = bundle.add_entity((pos2, vel2));
        
        // This should be None because we can't add more than one component
        assert!(indices2.is_none());
        
        // Verify only the first entity exists
        assert_eq!(bundle.get_pool::<PositionComponent>().unwrap().count(), 1);
        assert_eq!(bundle.get_pool::<VelocityComponent>().unwrap().count(), 1);
        
        // Verify the first entity's components are intact
        let pos_pool = bundle.get_pool::<PositionComponent>().unwrap();
        let retrieved_pos = pos_pool.get::<PositionComponent>(indices1[0]).unwrap();
        assert_eq!(retrieved_pos.x, 10.0);
        assert_eq!(retrieved_pos.y, 20.0);
    }
}
