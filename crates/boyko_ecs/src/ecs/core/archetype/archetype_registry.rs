use boyko_utils::sparse_map::sparse_map::SparseMap;
use crate::ecs::core::archetype::archetype_signature::ArchetypeSignature;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

/// Registry for efficiently storing and looking up archetypes by component mask
/// Uses hierarchical bitmap indexing for fast filtering with optimized memory layout
pub struct ArchetypeRegistry {
    /// Maps block patterns (u8 as usize) to groups of archetypes with that pattern
    /// Using SparseMap for O(1) access with better cache locality than HashMap
    block_groups: SparseMap<Vec<(ArchetypeId, ArchetypeSignature)>>,
    
    /// Stores all active block patterns for faster iteration
    active_patterns: Vec<u8>,
}

impl ArchetypeRegistry {
    /// Creates a new empty archetype registry
    pub fn new() -> Self {
        Self {
            block_groups: SparseMap::new(),
            active_patterns: Vec::new(),
        }
    }
    
    /// Creates a registry with pre-allocated capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            block_groups: SparseMap::with_capacity(256), // 256 possible block patterns (8-bit)
            active_patterns: Vec::with_capacity(32), // Expect fewer unique patterns
        }
    }
    
    /// Registers an archetype with its component mask
    pub fn register_archetype(&mut self, archetype_id: ArchetypeId, mask: ComponentMask) {
        // Create hierarchical signature for the mask
        let signature = ArchetypeSignature::new(mask);
        
        // Get 8-bit block summary as index
        let block_pattern = signature.block_summary.value() as usize;
        
        // If this is a new pattern, add it to active patterns list
        if !self.block_groups.contains(block_pattern) {
            self.active_patterns.push(signature.block_summary.value());
        }
        
        // Add archetype to the appropriate group
        if let Some(group) = self.block_groups.get_mut(block_pattern) {
            group.push((archetype_id, signature));
        } else {
            self.block_groups.insert(block_pattern, vec![(archetype_id, signature)]);
        }
    }
    
    /// Removes an archetype from the registry
    pub fn unregister_archetype(&mut self, archetype_id: ArchetypeId) -> bool {
        // Find the archetype in all active block groups
        for &pattern in &self.active_patterns.clone() {
            let pattern_index = pattern as usize;
            if let Some(group) = self.block_groups.get_mut(pattern_index) {
                if let Some(pos) = group.iter().position(|(id, _)| *id == archetype_id) {
                    // Remove the archetype from its group
                    group.swap_remove(pos);
                    
                    // If group is now empty, we should remove the pattern from active_patterns
                    if group.is_empty() {
                        if let Some(pattern_pos) = self.active_patterns.iter().position(|&p| p == pattern) {
                            self.active_patterns.swap_remove(pattern_pos);
                        }
                    }
                    
                    return true;
                }
            }
        }
        
        false
    }
    
    /// Finds archetypes containing all components in the query mask
    pub fn find_matching_archetypes(&self, mask: &ComponentMask) -> Vec<ArchetypeId> {
        // Create query signature
        let query = ArchetypeSignature::new(*mask);
        let mut result = Vec::new();
        
        // Iterate only through active patterns
        for &pattern in &self.active_patterns {
            // Check if all required blocks are present in this pattern
            // If (query & !pattern) != 0, it means the query has bits that pattern doesn't
            if (query.block_summary.value() & !pattern) == 0 {
                let pattern_index = pattern as usize;
                
                // Get the group of archetypes with this pattern
                if let Some(group) = self.block_groups.get(pattern_index) {
                    // Check each archetype in the group
                    for &(id, ref signature) in group {
                        if signature.contains(&query) {
                            result.push(id);
                        }
                    }
                }
            }
        }
        
        result
    }
    
    /// Finds archetypes that match the exact component mask
    pub fn find_exact_match(&self, mask: &ComponentMask) -> Vec<ArchetypeId> {
        // Create query signature
        let query = ArchetypeSignature::new(*mask);
        let block_pattern = query.block_summary.value() as usize;
        
        // Check if there are any archetypes with this exact block pattern
        if let Some(group) = self.block_groups.get(block_pattern) {
            // Filter archetypes that have exactly this mask
            return group.iter()
                .filter(|(_, signature)| signature.mask == query.mask)
                .map(|(id, _)| *id)
                .collect();
        }
        
        Vec::new()
    }
    
    /// Finds archetypes containing all specified components
    /// Optimized for queries with few components
    pub fn find_archetypes_with_components(&self, components: &[ComponentId]) -> Vec<ArchetypeId> {
        // For small queries use specialized logic
        if components.len() <= 3 {
            return self.find_archetypes_with_few_components(components);
        }
        
        // For larger queries use the general mechanism
        let mask = ComponentMask::from_components(components);
        self.find_matching_archetypes(&mask)
    }
    
    /// Specialized finder optimized for 1-3 component queries
    fn find_archetypes_with_few_components(&self, components: &[ComponentId]) -> Vec<ArchetypeId> {
        if components.is_empty() {
            return Vec::new();
        }
        
        // Create mask with only needed components
        let mut query_mask = ComponentMask::new();
        for &comp_id in components {
            query_mask.set(comp_id);
        }
        
        let query = ArchetypeSignature::new(query_mask);
        
        // For 1-3 components we can precisely determine which blocks they're in
        let mut relevant_blocks = Vec::with_capacity(components.len());
        for &comp_id in components {
            let block = (comp_id / 64) % 8;
            relevant_blocks.push(block);
        }
        
        // Remove duplicate blocks
        relevant_blocks.sort();
        relevant_blocks.dedup();
        
        let mut result = Vec::new();
        
        // Iterate only through active patterns
        for &pattern in &self.active_patterns {
            // Check if all needed blocks are present in the pattern
            let mut all_blocks_present = true;
            for &block in &relevant_blocks {
                if (pattern & (1 << block)) == 0 {
                    all_blocks_present = false;
                    break;
                }
            }
            
            if all_blocks_present {
                let pattern_index = pattern as usize;
                
                // Get the group of archetypes with this pattern
                if let Some(group) = self.block_groups.get(pattern_index) {
                    // Optimize check for few components
                    for &(id, ref signature) in group {
                        let mut all_components_present = true;
                        
                        for &comp_id in components {
                            if !signature.mask.contains(comp_id) {
                                all_components_present = false;
                                break;
                            }
                        }
                        
                        if all_components_present {
                            result.push(id);
                        }
                    }
                }
            }
        }
        
        result
    }
    
    /// Find archetypes with complex filtering criteria (include, exclude, optional components)
    /// New method that replaces the filtering logic from QueryBuilder
    pub fn find_with_filter(
        &self,
        include_mask: &ComponentMask,
        exclude_mask: &ComponentMask,
        optional_mask: &ComponentMask
    ) -> Vec<ArchetypeId> {
        // First get all archetypes matching the include mask
        let base_archetypes = if include_mask.is_empty() {
            // If no required components, use all archetypes
            // Get all active archetype IDs from all pattern groups
            let mut all_archetypes = Vec::new();
            for &pattern in &self.active_patterns {
                if let Some(group) = self.block_groups.get(pattern as usize) {
                    all_archetypes.extend(group.iter().map(|(id, _)| *id));
                }
            }
            all_archetypes
        } else {
            // Otherwise use the include mask
            self.find_matching_archetypes(include_mask)
        };
        
        // If no additional filtering needed, return base results
        if exclude_mask.is_empty() && optional_mask.is_empty() {
            return base_archetypes;
        }
        
        // Apply additional filtering
        base_archetypes.into_iter()
            .filter(|&id| {
                // Get the archetype signature
                let signature = self.get_archetype_signature(id);
                if let Some(signature) = signature {
                    // Skip if archetype contains any excluded component
                    if !exclude_mask.is_empty() {
                        let intersection = &signature.mask & exclude_mask;
                        if !intersection.is_empty() {
                            return false;
                        }
                    }
                    
                    // Skip if optional components are required but none are present
                    if !optional_mask.is_empty() {
                        let intersection = &signature.mask & optional_mask;
                        if intersection.is_empty() {
                            return false;
                        }
                    }
                    
                    true
                } else {
                    false
                }
            })
            .collect()
    }
    
    /// Get the signature for an archetype by ID
    /// Helper method for complex queries
    pub fn get_archetype_signature(&self, archetype_id: ArchetypeId) -> Option<ArchetypeSignature> {
        for &pattern in &self.active_patterns {
            let pattern_index = pattern as usize;
            if let Some(group) = self.block_groups.get(pattern_index) {
                if let Some((_, signature)) = group.iter().find(|(id, _)| *id == archetype_id) {
                    return Some(signature.clone());
                }
            }
        }
        None
    }
    
    /// Find archetypes with components that can be included, excluded, or optional
    /// Component-centric alternative to mask-based filtering
    pub fn find_with_component_filter(
        &self,
        include_components: &[ComponentId],
        exclude_components: &[ComponentId],
        optional_components: &[ComponentId]
    ) -> Vec<ArchetypeId> {
        // Convert component arrays to masks
        let include_mask = ComponentMask::from_components(include_components);
        let exclude_mask = ComponentMask::from_components(exclude_components);
        let optional_mask = ComponentMask::from_components(optional_components);
        
        // Use the mask-based filter
        self.find_with_filter(&include_mask, &exclude_mask, &optional_mask)
    }
    
    /// Returns the number of archetypes in the registry
    pub fn len(&self) -> usize {
        let mut count = 0;
        for &pattern in &self.active_patterns {
            let pattern_index = pattern as usize;
            if let Some(group) = self.block_groups.get(pattern_index) {
                count += group.len();
            }
        }
        count
    }
    
    /// Checks if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.active_patterns.is_empty()
    }
    
    /// Clears all archetypes from the registry
    pub fn clear(&mut self) {
        self.block_groups.clear();
        self.active_patterns.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a mask with the given component IDs
    fn create_mask(components: &[ComponentId]) -> ComponentMask {
        let mut mask = ComponentMask::new();
        for &comp_id in components {
            mask.set(comp_id);
        }
        mask
    }

    #[test]
    fn test_register_and_len() {
        let mut registry = ArchetypeRegistry::new();
        assert_eq!(registry.len(), 0);
        assert!(registry.is_empty());

        // Register some archetypes
        registry.register_archetype(1, create_mask(&[1, 2, 3]));
        registry.register_archetype(2, create_mask(&[1, 2]));
        registry.register_archetype(3, create_mask(&[1, 3]));

        assert_eq!(registry.len(), 3);
        assert!(!registry.is_empty());
    }

    #[test]
    fn test_unregister() {
        let mut registry = ArchetypeRegistry::new();
        
        // Register some archetypes
        registry.register_archetype(1, create_mask(&[1, 2, 3]));
        registry.register_archetype(2, create_mask(&[1, 2]));
        registry.register_archetype(3, create_mask(&[1, 3]));
        
        assert_eq!(registry.len(), 3);
        
        // Unregister an archetype
        let result = registry.unregister_archetype(2);
        assert!(result);
        assert_eq!(registry.len(), 2);
        
        // Try to unregister a non-existent archetype
        let result = registry.unregister_archetype(999);
        assert!(!result);
        assert_eq!(registry.len(), 2);
    }
    
    #[test]
    fn test_clear() {
        let mut registry = ArchetypeRegistry::new();
        
        // Register some archetypes
        registry.register_archetype(1, create_mask(&[1, 2, 3]));
        registry.register_archetype(2, create_mask(&[1, 2]));
        
        assert_eq!(registry.len(), 2);
        
        // Clear the registry
        registry.clear();
        assert_eq!(registry.len(), 0);
        assert!(registry.is_empty());
    }
    
    #[test]
    fn test_find_exact_match() {
        let mut registry = ArchetypeRegistry::new();
        
        // Register archetypes with different component combinations
        registry.register_archetype(1, create_mask(&[1, 2, 3]));
        registry.register_archetype(2, create_mask(&[1, 2]));
        registry.register_archetype(3, create_mask(&[1, 3]));
        registry.register_archetype(4, create_mask(&[1, 2, 3])); // Duplicate signature
        
        // Find exact matches
        let results = registry.find_exact_match(&create_mask(&[1, 2, 3]));
        assert_eq!(results.len(), 2);
        assert!(results.contains(&1));
        assert!(results.contains(&4));
        
        let results = registry.find_exact_match(&create_mask(&[1, 2]));
        assert_eq!(results.len(), 1);
        assert!(results.contains(&2));
        
        // No match
        let results = registry.find_exact_match(&create_mask(&[4, 5]));
        assert_eq!(results.len(), 0);
    }
    
    #[test]
    fn test_find_matching_archetypes() {
        let mut registry = ArchetypeRegistry::new();
        
        // Register archetypes with different component combinations
        registry.register_archetype(1, create_mask(&[1, 2, 3, 4]));
        registry.register_archetype(2, create_mask(&[1, 2, 5]));
        registry.register_archetype(3, create_mask(&[1, 3, 6]));
        registry.register_archetype(4, create_mask(&[2, 3, 7]));
        registry.register_archetype(5, create_mask(&[5, 6, 7]));
        
        // Find archetypes with component 1
        let results = registry.find_matching_archetypes(&create_mask(&[1]));
        assert_eq!(results.len(), 3);
        assert!(results.contains(&1));
        assert!(results.contains(&2));
        assert!(results.contains(&3));
        
        // Find archetypes with components 1 and 3
        let results = registry.find_matching_archetypes(&create_mask(&[1, 3]));
        assert_eq!(results.len(), 2);
        assert!(results.contains(&1));
        assert!(results.contains(&3));
        
        // Find archetypes with components 5 and 7
        let results = registry.find_matching_archetypes(&create_mask(&[5, 7]));
        assert_eq!(results.len(), 1);
        assert!(results.contains(&5));
        
        // No match
        let results = registry.find_matching_archetypes(&create_mask(&[8, 9]));
        assert_eq!(results.len(), 0);
    }
    
    #[test]
    fn test_component_arrays() {
        let mut registry = ArchetypeRegistry::new();
        
        // Register archetypes with different component combinations
        registry.register_archetype(1, create_mask(&[1, 2, 3, 4]));
        registry.register_archetype(2, create_mask(&[1, 2, 5]));
        registry.register_archetype(3, create_mask(&[1, 3, 6]));
        registry.register_archetype(4, create_mask(&[2, 3, 7]));
        
        // Find using component arrays
        let results = registry.find_archetypes_with_components(&[1]);
        assert_eq!(results.len(), 3);
        assert!(results.contains(&1));
        assert!(results.contains(&2));
        assert!(results.contains(&3));
        
        // Find with 2 components (small query optimization)
        let results = registry.find_archetypes_with_components(&[2, 3]);
        assert_eq!(results.len(), 2);
        assert!(results.contains(&1));
        assert!(results.contains(&4));
        
        // Find with 3 components (small query optimization)
        let results = registry.find_archetypes_with_components(&[1, 2, 5]);
        assert_eq!(results.len(), 1);
        assert!(results.contains(&2));
        
        // Find with more than 3 components (uses regular query path)
        let results = registry.find_archetypes_with_components(&[1, 2, 3, 4]);
        assert_eq!(results.len(), 1);
        assert!(results.contains(&1));
    }
    
    #[test]
    fn test_with_components_in_different_blocks() {
        let mut registry = ArchetypeRegistry::new();
        
        // Components in different blocks (block 0 and block 1)
        let comp1 = 1;        // Block 0
        let comp2 = 65;       // Block 1 (65 / 64 = 1)
        let comp3 = 128;      // Block 2 (128 / 64 = 2)
        
        registry.register_archetype(1, create_mask(&[comp1, comp2]));
        registry.register_archetype(2, create_mask(&[comp1, comp3]));
        registry.register_archetype(3, create_mask(&[comp2, comp3]));
        registry.register_archetype(4, create_mask(&[comp1, comp2, comp3]));
        
        // Find archetypes with components in different blocks
        let results = registry.find_archetypes_with_components(&[comp1, comp2]);
        assert_eq!(results.len(), 2);
        assert!(results.contains(&1));
        assert!(results.contains(&4));
        
        let results = registry.find_archetypes_with_components(&[comp1, comp3]);
        assert_eq!(results.len(), 2);
        assert!(results.contains(&2));
        assert!(results.contains(&4));
    }
    
    #[test]
    fn test_find_with_filter() {
        let mut registry = ArchetypeRegistry::new();
        
        // Register archetypes with different component combinations
        registry.register_archetype(1, create_mask(&[1, 2]));          // Position, Velocity
        registry.register_archetype(2, create_mask(&[1, 3]));          // Position, Health
        registry.register_archetype(3, create_mask(&[2, 4]));          // Velocity, Damage
        registry.register_archetype(4, create_mask(&[1, 2, 3]));       // Position, Velocity, Health
        registry.register_archetype(5, create_mask(&[1, 2, 4]));       // Position, Velocity, Damage
        
        // Find archetypes with Position, but not Damage
        let include_mask = create_mask(&[1]);    // Position
        let exclude_mask = create_mask(&[4]);    // Damage
        let optional_mask = ComponentMask::new();
        
        let results = registry.find_with_filter(&include_mask, &exclude_mask, &optional_mask);
        assert_eq!(results.len(), 3);
        assert!(results.contains(&1));
        assert!(results.contains(&2));
        assert!(results.contains(&4));
        
        // Find archetypes with Position, and at least one of Health or Damage
        let include_mask = create_mask(&[1]);    // Position
        let exclude_mask = ComponentMask::new();
        let optional_mask = create_mask(&[3, 4]);   // Health or Damage
        
        let results = registry.find_with_filter(&include_mask, &exclude_mask, &optional_mask);
        assert_eq!(results.len(), 3);
        assert!(results.contains(&2));
        assert!(results.contains(&4));
        assert!(results.contains(&5));
        
        // Find archetypes with Position AND Velocity, but NOT Damage
        let include_mask = create_mask(&[1, 2]);  // Position AND Velocity
        let exclude_mask = create_mask(&[4]);     // NOT Damage
        let optional_mask = ComponentMask::new();
        
        let results = registry.find_with_filter(&include_mask, &exclude_mask, &optional_mask);
        assert_eq!(results.len(), 2);
        assert!(results.contains(&1));
        assert!(results.contains(&4));
    }
    
    #[test]
    fn test_find_with_component_filter() {
        let mut registry = ArchetypeRegistry::new();
        
        // Register archetypes with different component combinations
        registry.register_archetype(1, create_mask(&[1, 2]));          // Position, Velocity
        registry.register_archetype(2, create_mask(&[1, 3]));          // Position, Health
        registry.register_archetype(3, create_mask(&[2, 4]));          // Velocity, Damage
        registry.register_archetype(4, create_mask(&[1, 2, 3]));       // Position, Velocity, Health
        registry.register_archetype(5, create_mask(&[1, 2, 4]));       // Position, Velocity, Damage
        
        // Find archetypes with Position, but not Damage
        let results = registry.find_with_component_filter(&[1], &[4], &[]);
        assert_eq!(results.len(), 3);
        assert!(results.contains(&1));
        assert!(results.contains(&2));
        assert!(results.contains(&4));
        
        // Find archetypes with Position, and at least one of Health or Damage
        let results = registry.find_with_component_filter(&[1], &[], &[3, 4]);
        assert_eq!(results.len(), 3);
        assert!(results.contains(&2));
        assert!(results.contains(&4));
        assert!(results.contains(&5));
    }
}
