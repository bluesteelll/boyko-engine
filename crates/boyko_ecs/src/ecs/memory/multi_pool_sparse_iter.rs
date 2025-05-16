use std::alloc::Layout;
use std::marker::PhantomData;
use crate::ecs::memory::component_pool::ComponentPool;
use crate::ecs::identifiers::primitives::{EntityId, ComponentId};
use crate::ecs::core::component::Component;
use crate::ecs::memory::sparse_iter_component_pool::{
    ComponentPoolSparseIter, ComponentPoolSparseIterMut, 
    ComponentPtr, ComponentMutPtr
};

/// Manages multiple component iterators for efficient parallel iteration
pub struct MultiPoolSparseIter {
    /// Array of component iterators, one for each component type
    iterators: Box<[ComponentPoolSparseIter]>,
    
    /// Number of components to iterate over
    component_count: usize,
    
    /// Current position in the iteration
    current: usize,
    
    /// Total number of entities
    entity_count: usize,
    
    /// Component IDs tracked by this iterator
    component_ids: Box<[ComponentId]>,
}

impl MultiPoolSparseIter {
    /// Creates a new MultiPoolSparseIter from an array of pools and entity indices
    pub fn new(
        pools: &[&ComponentPool], 
        entity_indices: &[usize],
        component_ids: &[ComponentId]
    ) -> Self {
        debug_assert_eq!(pools.len(), component_ids.len(), 
            "Number of pools must match number of component IDs");
            
        // Create an iterator for each pool
        let iterators: Vec<ComponentPoolSparseIter> = pools.iter()
            .map(|pool| ComponentPoolSparseIter::new(pool, entity_indices))
            .collect();
        
        Self {
            iterators: iterators.into_boxed_slice(),
            component_count: pools.len(),
            current: 0,
            entity_count: entity_indices.len(),
            component_ids: component_ids.to_vec().into_boxed_slice(),
        }
    }
    
    /// Creates a new MultiPoolSparseIter from specific entity indices across multiple pools
    pub fn from_entities(
        pools: &[&ComponentPool],
        entity_indices: &[usize],
        component_ids: &[ComponentId]
    ) -> Self {
        Self::new(pools, entity_indices, component_ids)
    }
    
    /// Creates a new MultiPoolSparseIter for all entities in the given pools
    pub fn from_pools(
        pools: &[&ComponentPool],
        component_ids: &[ComponentId]
    ) -> Self {
        // Get all valid entity indices (up to the smallest pool size)
        let min_count = pools.iter().map(|p| p.count()).min().unwrap_or(0);
        let indices: Vec<usize> = (0..min_count).collect();
        
        Self::new(pools, &indices, component_ids)
    }
    
    /// Returns the number of entities in this iterator
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.entity_count
    }
    
    /// Checks if the iterator is empty
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.entity_count == 0
    }
    
    /// Resets the iterator to the beginning
    #[inline(always)]
    pub fn reset(&mut self) {
        self.current = 0;
        for iter in self.iterators.iter_mut() {
            iter.reset();
        }
    }
    
    /// Get the component IDs being tracked by this iterator
    #[inline(always)]
    pub fn component_ids(&self) -> &[ComponentId] {
        &self.component_ids
    }
    
    /// Advances the iterator and returns pointers to all components for the current entity
    pub fn next_raw(&mut self) -> Option<Box<[ComponentPtr]>> {
        if self.current >= self.entity_count {
            return None;
        }
        
        // Advance position
        self.current += 1;
        
        // Collect pointers from all component iterators
        let mut pointers = Vec::with_capacity(self.component_count);
        for iter in self.iterators.iter_mut() {
            if let Some(ptr) = iter.next() {
                pointers.push(ptr);
            } else {
                // If any iterator is exhausted, we're done
                return None;
            }
        }
        
        Some(pointers.into_boxed_slice())
    }
    
    /// Creates a typed iterator adapter for specific component types
    pub fn typed<T: Component, U: Component>(self) -> TypedMultiComponentIter<T, U> {
        // Verify component types match component IDs
        debug_assert_eq!(self.component_count, 2, 
            "TypedMultiComponentIter<T, U> requires exactly 2 components");
        debug_assert!(
            self.component_ids.contains(&T::component_id()) &&
            self.component_ids.contains(&U::component_id()),
            "Component types don't match component IDs"
        );
        
        TypedMultiComponentIter {
            inner: self,
            _phantom_t: PhantomData,
            _phantom_u: PhantomData,
        }
    }
}

/// Mutable variant of MultiPoolSparseIter
pub struct MultiPoolSparseIterMut {
    /// Array of mutable component iterators, one for each component type
    iterators: Box<[ComponentPoolSparseIterMut]>,
    
    /// Number of components to iterate over
    component_count: usize,
    
    /// Current position in the iteration
    current: usize,
    
    /// Total number of entities
    entity_count: usize,
    
    /// Component IDs tracked by this iterator
    component_ids: Box<[ComponentId]>,
}

impl MultiPoolSparseIterMut {
    /// Creates a new MultiPoolSparseIterMut from an array of pools and entity indices
    pub fn new(
        pools: &mut [&mut ComponentPool], 
        entity_indices: &[usize],
        component_ids: &[ComponentId]
    ) -> Self {
        debug_assert_eq!(pools.len(), component_ids.len(), 
            "Number of pools must match number of component IDs");
            
        // Create an iterator for each pool
        let iterators: Vec<ComponentPoolSparseIterMut> = pools.iter_mut()
            .map(|pool| ComponentPoolSparseIterMut::new(pool, entity_indices))
            .collect();
        
        Self {
            iterators: iterators.into_boxed_slice(),
            component_count: pools.len(),
            current: 0,
            entity_count: entity_indices.len(),
            component_ids: component_ids.to_vec().into_boxed_slice(),
        }
    }
    
    /// Resets the iterator to the beginning
    #[inline(always)]
    pub fn reset(&mut self) {
        self.current = 0;
        for iter in self.iterators.iter_mut() {
            iter.reset();
        }
    }
    
    /// Advances the iterator and returns mutable pointers to all components for the current entity
    pub fn next_raw_mut(&mut self) -> Option<Box<[ComponentMutPtr]>> {
        if self.current >= self.entity_count {
            return None;
        }
        
        // Advance position
        self.current += 1;
        
        // Collect pointers from all component iterators
        let mut pointers = Vec::with_capacity(self.component_count);
        for iter in self.iterators.iter_mut() {
            if let Some(ptr) = iter.next() {
                pointers.push(ptr);
            } else {
                // If any iterator is exhausted, we're done
                return None;
            }
        }
        
        Some(pointers.into_boxed_slice())
    }
    
    /// Creates a typed mutable iterator adapter for specific component types
    pub fn typed_mut<T: Component, U: Component>(self) -> TypedMultiComponentIterMut<T, U> {
        // Verify component types match component IDs
        debug_assert_eq!(self.component_count, 2, 
            "TypedMultiComponentIterMut<T, U> requires exactly 2 components");
        debug_assert!(
            self.component_ids.contains(&T::component_id()) &&
            self.component_ids.contains(&U::component_id()),
            "Component types don't match component IDs"
        );
        
        TypedMultiComponentIterMut {
            inner: self,
            _phantom_t: PhantomData,
            _phantom_u: PhantomData,
        }
    }
}