use std::marker::PhantomData;
use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, EntityId};
use crate::ecs::memory::component_pool::ComponentPool;
use crate::ecs::memory::sparse_iter_component_pool::{
    ComponentPoolSparseIter, ComponentPoolSparseIterMut,
    ComponentPtr, ComponentMutPtr
};

/// Iterator over components across multiple archetypes
/// Allows querying entities with specific components regardless of archetype
pub struct SparseIter {
    /// Array of multi-pool iterators, one for each matching archetype
    archetype_iterators: Vec<MultiPoolSparseIter>,
    
    /// Currently active iterator index
    current_archetype: usize,
    
    /// Component IDs being queried (same for all archetypes)
    component_ids: Box<[ComponentId]>,
    
    /// Total number of entities across all archetypes
    total_entity_count: usize,
    
    /// Archetype IDs for each iterator (for diagnostic/debugging)
    archetype_ids: Box<[ArchetypeId]>,
}

impl SparseIter {
    /// Creates a new SparseIter from a collection of archetypes, filtering for specific component types
    pub fn new(
        archetypes: &[&Archetype],
        component_ids: &[ComponentId],
        archetype_ids: &[ArchetypeId]
    ) -> Self {
        debug_assert_eq!(archetypes.len(), archetype_ids.len(),
            "Number of archetypes must match number of archetype IDs");
        
        let mut total_count = 0;
        let mut archetype_iterators = Vec::with_capacity(archetypes.len());
        
        // For each archetype, create a MultiPoolSparseIter if it has all the required components
        for (i, archetype) in archetypes.iter().enumerate() {
            // Verify the archetype contains all required component types
            if !component_ids.iter().all(|&id| archetype.has_component_id(id)) {
                continue;
            }
            
            // Get component pools for this archetype
            let mut pools = Vec::with_capacity(component_ids.len());
            for &comp_id in component_ids {
                if let Some(pool) = archetype.component_pools().get_pool(comp_id) {
                    pools.push(pool);
                } else {
                    // This should not happen if has_component_id is true
                    debug_assert!(false, "Component pool not found for ID {}", comp_id);
                    continue;
                }
            }
            
            // Get all entity indices in this archetype
            let entity_count = archetype.entity_count();
            let indices: Vec<usize> = (0..entity_count).collect();
            
            // Create a MultiPoolSparseIter for this archetype
            let pools_ref: Vec<&ComponentPool> = pools.iter().map(|p| *p).collect();
            let iter = MultiPoolSparseIter::new(&pools_ref, &indices, component_ids);
            
            archetype_iterators.push(iter);
            total_count += entity_count;
        }
        
        Self {
            archetype_iterators,
            current_archetype: 0,
            component_ids: component_ids.to_vec().into_boxed_slice(),
            total_entity_count: total_count,
            archetype_ids: archetype_ids.to_vec().into_boxed_slice(),
        }
    }
    
    /// Creates a new SparseIter from archetype master, querying all matching archetypes
    pub fn from_archetype_master(
        master: &ArchetypeMaster,
        component_ids: &[ComponentId]
    ) -> Self {
        // Find all archetypes containing the required components
        let archetype_ids = master.find_archetypes_with_components(component_ids);
        
        // Get references to archetypes
        let archetypes: Vec<&Archetype> = archetype_ids.iter()
            .filter_map(|&id| master.get_archetype(id))
            .collect();
            
        Self::new(&archetypes, component_ids, &archetype_ids)
    }
    
    /// Get the total number of entities across all archetypes
    #[inline(always)]
    pub fn total_entity_count(&self) -> usize {
        self.total_entity_count
    }
    
    /// Get the number of matching archetypes
    #[inline(always)]
    pub fn archetype_count(&self) -> usize {
        self.archetype_iterators.len()
    }
    
    /// Get the component IDs being queried
    #[inline(always)]
    pub fn component_ids(&self) -> &[ComponentId] {
        &self.component_ids
    }
    
    /// Reset the iterator to start from the beginning
    #[inline]
    pub fn reset(&mut self) {
        self.current_archetype = 0;
        for iter in &mut self.archetype_iterators {
            iter.reset();
        }
    }
    
    /// Advance to the next component set across all archetypes
    /// Returns raw component pointers and the archetype ID they belong to
    pub fn next_raw(&mut self) -> Option<(Box<[ComponentPtr]>, ArchetypeId)> {
        // If we're out of archetypes, we're done
        if self.current_archetype >= self.archetype_iterators.len() {
            return None;
        }
        
        // Try to get components from the current archetype iterator
        match self.archetype_iterators[self.current_archetype].next_raw() {
            Some(components) => {
                // Found components in the current archetype
                let archetype_id = self.archetype_ids[self.current_archetype];
                Some((components, archetype_id))
            },
            None => {
                // Current archetype exhausted, move to the next one
                self.current_archetype += 1;
                self.next_raw() // Recursive call to try the next archetype
            }
        }
    }
    
    /// Get iterator over archetype IDs
    pub fn archetype_ids(&self) -> impl Iterator<Item = ArchetypeId> + '_ {
        self.archetype_ids.iter().cloned()
    }
    
    
    
    /// For each entity with the matching components, call the provided function
    pub fn for_each<F>(&mut self, mut f: F)
    where
        F: FnMut(Box<[ComponentPtr]>, ArchetypeId),
    {
        self.reset();
        
        while let Some((components, archetype_id)) = self.next_raw() {
            f(components, archetype_id);
        }
    }
    
}

/// Mutable variant of SparseIter
pub struct SparseIterMut {
    /// Array of mutable multi-pool iterators, one for each matching archetype
    archetype_iterators: Vec<MultiPoolSparseIterMut>,
    
    /// Currently active iterator index
    current_archetype: usize,
    
    /// Component IDs being queried (same for all archetypes)
    component_ids: Box<[ComponentId]>,
    
    /// Total number of entities across all archetypes
    total_entity_count: usize,
    
    /// Archetype IDs for each iterator (for diagnostic/debugging)
    archetype_ids: Box<[ArchetypeId]>,
}

impl SparseIterMut {
    /// Creates a new mutable SparseIter from a collection of archetypes
    pub fn new(
        archetypes: &mut [&mut Archetype],
        component_ids: &[ComponentId],
        archetype_ids: &[ArchetypeId]
    ) -> Self {
        debug_assert_eq!(archetypes.len(), archetype_ids.len(),
            "Number of archetypes must match number of archetype IDs");
        
        let mut total_count = 0;
        let mut archetype_iterators = Vec::with_capacity(archetypes.len());
        
        // For each archetype, create a MultiPoolSparseIterMut if it has all the required components
        for (i, archetype) in archetypes.iter_mut().enumerate() {
            // Verify the archetype contains all required component types
            if !component_ids.iter().all(|&id| archetype.has_component_id(id)) {
                continue;
            }
            
            // Get mutable component pools for this archetype
            let mut pools = Vec::with_capacity(component_ids.len());
            for &comp_id in component_ids {
                if let Some(pool) = archetype.component_pools_mut().get_pool_mut(comp_id) {
                    pools.push(pool);
                } else {
                    // This should not happen if has_component_id is true
                    debug_assert!(false, "Component pool not found for ID {}", comp_id);
                    continue;
                }
            }
            
            // Get all entity indices in this archetype
            let entity_count = archetype.entity_count();
            let indices: Vec<usize> = (0..entity_count).collect();
            
            // Create a MultiPoolSparseIterMut for this archetype
            let pools_ref: Vec<&mut ComponentPool> = pools.iter_mut().map(|p| *p).collect();
            let iter = MultiPoolSparseIterMut::new(&mut pools_ref, &indices, component_ids);
            
            archetype_iterators.push(iter);
            total_count += entity_count;
        }
        
        Self {
            archetype_iterators,
            current_archetype: 0,
            component_ids: component_ids.to_vec().into_boxed_slice(),
            total_entity_count: total_count,
            archetype_ids: archetype_ids.to_vec().into_boxed_slice(),
        }
    }
    
    /// Creates a new mutable SparseIter from archetype master, querying all matching archetypes
    pub fn from_archetype_master(
        master: &mut ArchetypeMaster,
        component_ids: &[ComponentId]
    ) -> Self {
        // Find all archetypes containing the required components
        let archetype_ids = master.find_archetypes_with_components(component_ids);
        
        // Get mutable references to archetypes
        let mut archetypes: Vec<&mut Archetype> = archetype_ids.iter()
            .filter_map(|&id| master.get_archetype_mut(id))
            .collect();
            
        Self::new(&mut archetypes, component_ids, &archetype_ids)
    }
    
    /// Get the total number of entities across all archetypes
    #[inline(always)]
    pub fn total_entity_count(&self) -> usize {
        self.total_entity_count
    }
    
    /// Get the number of matching archetypes
    #[inline(always)]
    pub fn archetype_count(&self) -> usize {
        self.archetype_iterators.len()
    }
    
    /// Get the component IDs being queried
    #[inline(always)]
    pub fn component_ids(&self) -> &[ComponentId] {
        &self.component_ids
    }
    
    /// Reset the iterator to start from the beginning
    #[inline]
    pub fn reset(&mut self) {
        self.current_archetype = 0;
        for iter in &mut self.archetype_iterators {
            iter.reset();
        }
    }
    
    /// Advance to the next component set across all archetypes
    /// Returns raw mutable component pointers and the archetype ID they belong to
    pub fn next_raw_mut(&mut self) -> Option<(Box<[ComponentMutPtr]>, ArchetypeId)> {
        // If we're out of archetypes, we're done
        if self.current_archetype >= self.archetype_iterators.len() {
            return None;
        }
        
        // Try to get components from the current archetype iterator
        match self.archetype_iterators[self.current_archetype].next_raw_mut() {
            Some(components) => {
                // Found components in the current archetype
                let archetype_id = self.archetype_ids[self.current_archetype];
                Some((components, archetype_id))
            },
            None => {
                // Current archetype exhausted, move to the next one
                self.current_archetype += 1;
                self.next_raw_mut() // Recursive call to try the next archetype
            }
        }
    }
    
    /// Get iterator over archetype IDs
    pub fn archetype_ids(&self) -> impl Iterator<Item = ArchetypeId> + '_ {
        self.archetype_ids.iter().cloned()
    }
    
    
    
    
    /// For each entity with the matching components, call the provided function
    pub fn for_each_mut<F>(&mut self, mut f: F)
    where
        F: FnMut(Box<[ComponentMutPtr]>, ArchetypeId),
    {
        self.reset();
        
        while let Some((components, archetype_id)) = self.next_raw_mut() {
            f(components, archetype_id);
        }
    }
        
}