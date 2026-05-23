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
    
    /// Finds archetypes containing all components in the query mask.
    ///
    /// Thin wrapper around [`find_matching_archetypes_into`] for backward compatibility.
    #[inline]
    pub fn find_matching_archetypes(&self, mask: &ComponentMask) -> Vec<ArchetypeId> {
        let mut out = Vec::new();
        self.find_matching_archetypes_into(mask, &mut out);
        out
    }

    /// Writes matching archetype IDs into `out`.
    ///
    /// # API contract
    /// `out` is **cleared at function entry**. Any existing contents are
    /// discarded. The caller's `Vec` is reused only for capacity, not data —
    /// this enables zero-allocation reuse across calls.
    #[inline]
    pub fn find_matching_archetypes_into(&self, mask: &ComponentMask, out: &mut Vec<ArchetypeId>) {
        out.clear();
        let query = ArchetypeSignature::new(*mask);

        for &pattern in &self.active_patterns {
            // If (query & !pattern) != 0 the query requires a block the pattern lacks.
            if (query.block_summary.value() & !pattern) == 0 {
                let pattern_index = pattern as usize;
                if let Some(group) = self.block_groups.get(pattern_index) {
                    for &(id, ref signature) in group {
                        if signature.contains(&query) {
                            out.push(id);
                        }
                    }
                }
            }
        }
    }
    
    /// Finds archetypes that match the exact component mask.
    ///
    /// Thin wrapper around [`find_exact_match_into`] for backward compatibility.
    #[inline]
    pub fn find_exact_match(&self, mask: &ComponentMask) -> Vec<ArchetypeId> {
        let mut out = Vec::new();
        self.find_exact_match_into(mask, &mut out);
        out
    }

    /// Writes archetypes with exactly `mask` into `out`.
    ///
    /// # API contract
    /// `out` is **cleared at function entry**. Any existing contents are
    /// discarded. The caller's `Vec` is reused only for capacity, not data —
    /// this enables zero-allocation reuse across calls.
    #[inline]
    pub fn find_exact_match_into(&self, mask: &ComponentMask, out: &mut Vec<ArchetypeId>) {
        out.clear();
        let query = ArchetypeSignature::new(*mask);
        let block_pattern = query.block_summary.value() as usize;

        if let Some(group) = self.block_groups.get(block_pattern) {
            for (id, signature) in group {
                if signature.mask == query.mask {
                    out.push(*id);
                }
            }
        }
    }
    
    /// Finds archetypes containing all specified components.
    ///
    /// Thin wrapper around [`find_archetypes_with_components_into`] for backward compatibility.
    #[inline]
    pub fn find_archetypes_with_components(&self, components: &[ComponentId]) -> Vec<ArchetypeId> {
        let mut out = Vec::new();
        self.find_archetypes_with_components_into(components, &mut out);
        out
    }

    /// Writes matching archetype IDs into `out`.
    ///
    /// Optimized for queries with few (≤ 3) components via the stack-only
    /// relevant-blocks path; larger queries fall back to the mask-based scan.
    ///
    /// # API contract
    /// `out` is **cleared at function entry**. Any existing contents are
    /// discarded. The caller's `Vec` is reused only for capacity, not data —
    /// this enables zero-allocation reuse across calls.
    #[inline]
    pub fn find_archetypes_with_components_into(
        &self,
        components: &[ComponentId],
        out: &mut Vec<ArchetypeId>,
    ) {
        out.clear();
        if components.len() <= 3 {
            self.find_archetypes_with_few_components_into(components, out);
        } else {
            let mask = ComponentMask::from_components(components);
            self.find_matching_archetypes_into(&mask, out);
        }
    }
    
    /// Writes results of a 1-3 component query into `out`.
    ///
    /// Uses a stack-only `[u8; 3]` buffer for the relevant-block set with
    /// inline insertion-sort-with-dedup, eliminating all heap allocation on the
    /// bookkeeping path.
    ///
    /// # Caller invariant
    /// `components.len() <= 3` — enforced by `debug_assert!`.
    ///
    /// # API contract
    /// `out` is assumed to be cleared by the public wrapper that called this
    /// helper (`find_archetypes_with_components_into`). Do not clear here to
    /// avoid a redundant double-clear on every call.
    fn find_archetypes_with_few_components_into(
        &self,
        components: &[ComponentId],
        out: &mut Vec<ArchetypeId>,
    ) {
        debug_assert!(
            components.len() <= 3,
            "find_archetypes_with_few_components_into: caller invariant violated (len={})",
            components.len()
        );

        // Load-bearing early-exit: without this, an empty `components` slice
        // would match every archetype, because the inner
        // `for &block in relevant_blocks` loop never executes and
        // `all_blocks_present` stays true (vacuous truth).
        if components.is_empty() {
            return;
        }

        // Build a component mask for the signature check.
        let mut query_mask = ComponentMask::new();
        for &comp_id in components {
            query_mask.set(comp_id);
        }
        let query = ArchetypeSignature::new(query_mask);

        // Compute the relevant blocks (which 64-bit word each component lives in)
        // using a stack-only [u8; 3] buffer + inline insertion-sort-with-dedup.
        // `components.len() <= 3` is guaranteed by the debug_assert above.
        let mut blocks: [u8; 3] = [0; 3];
        let mut blocks_len: usize = 0;

        for &comp_id in components {
            let block = ((comp_id / 64) % 8) as u8;

            // Insertion-sort-with-dedup: find the insertion position or skip duplicate.
            let mut insert_pos = blocks_len;
            let mut duplicate = false;
            for i in 0..blocks_len {
                if blocks[i] == block {
                    duplicate = true;
                    break;
                }
                if blocks[i] > block {
                    insert_pos = i;
                    break;
                }
            }
            if !duplicate {
                // Shift elements right to make room.
                let mut j = blocks_len;
                while j > insert_pos {
                    blocks[j] = blocks[j - 1];
                    j -= 1;
                }
                blocks[insert_pos] = block;
                blocks_len += 1;
            }
        }

        let relevant_blocks = &blocks[..blocks_len];

        for &pattern in &self.active_patterns {
            // Check if all needed blocks are present in the pattern bit-field.
            let mut all_blocks_present = true;
            for &block in relevant_blocks {
                if (pattern & (1 << block)) == 0 {
                    all_blocks_present = false;
                    break;
                }
            }

            if all_blocks_present {
                let pattern_index = pattern as usize;
                if let Some(group) = self.block_groups.get(pattern_index) {
                    for &(id, ref signature) in group {
                        if signature.contains(&query) {
                            out.push(id);
                        }
                    }
                }
            }
        }
    }
    
    /// Find archetypes with complex filtering criteria (include, exclude, optional components).
    ///
    /// Thin wrapper around [`find_with_filter_into`] for backward compatibility.
    #[inline]
    pub fn find_with_filter(
        &self,
        include_mask: &ComponentMask,
        exclude_mask: &ComponentMask,
        optional_mask: &ComponentMask,
    ) -> Vec<ArchetypeId> {
        let mut out = Vec::new();
        self.find_with_filter_into(include_mask, exclude_mask, optional_mask, &mut out);
        out
    }

    /// Writes matching archetype IDs into `out` using include/exclude/optional masks.
    ///
    /// # API contract
    /// `out` is **cleared at function entry**. Any existing contents are
    /// discarded. The caller's `Vec` is reused only for capacity, not data —
    /// this enables zero-allocation reuse across calls.
    #[inline]
    pub fn find_with_filter_into(
        &self,
        include_mask: &ComponentMask,
        exclude_mask: &ComponentMask,
        optional_mask: &ComponentMask,
        out: &mut Vec<ArchetypeId>,
    ) {
        out.clear();

        // Collect base archetypes (those matching the include mask).
        // We write into `out` temporarily, then apply exclude/optional in-place.
        if include_mask.is_empty() {
            for &pattern in &self.active_patterns {
                if let Some(group) = self.block_groups.get(pattern as usize) {
                    out.extend(group.iter().map(|(id, _)| *id));
                }
            }
        } else {
            self.find_matching_archetypes_into(include_mask, out);
        }

        // If no additional filtering, we are done.
        if exclude_mask.is_empty() && optional_mask.is_empty() {
            return;
        }

        // Filter in-place: retain only archetypes that pass exclude + optional checks.
        out.retain(|&id| {
            let Some(signature) = self.get_archetype_signature(id) else {
                return false;
            };
            if !exclude_mask.is_empty() {
                let intersection = &signature.mask & exclude_mask;
                if !intersection.is_empty() {
                    return false;
                }
            }
            if !optional_mask.is_empty() {
                let intersection = &signature.mask & optional_mask;
                if intersection.is_empty() {
                    return false;
                }
            }
            true
        });
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
    
    /// Find archetypes with components that can be included, excluded, or optional.
    ///
    /// Component-centric alternative to mask-based filtering. Thin wrapper around
    /// [`find_with_component_filter_into`] for backward compatibility.
    #[inline]
    pub fn find_with_component_filter(
        &self,
        include_components: &[ComponentId],
        exclude_components: &[ComponentId],
        optional_components: &[ComponentId],
    ) -> Vec<ArchetypeId> {
        let mut out = Vec::new();
        self.find_with_component_filter_into(
            include_components,
            exclude_components,
            optional_components,
            &mut out,
        );
        out
    }

    /// Writes matching archetype IDs into `out` using component-array filters.
    ///
    /// # API contract
    /// `out` is **cleared at function entry**. Any existing contents are
    /// discarded. The caller's `Vec` is reused only for capacity, not data —
    /// this enables zero-allocation reuse across calls.
    #[inline]
    pub fn find_with_component_filter_into(
        &self,
        include_components: &[ComponentId],
        exclude_components: &[ComponentId],
        optional_components: &[ComponentId],
        out: &mut Vec<ArchetypeId>,
    ) {
        let include_mask = ComponentMask::from_components(include_components);
        let exclude_mask = ComponentMask::from_components(exclude_components);
        let optional_mask = ComponentMask::from_components(optional_components);
        self.find_with_filter_into(&include_mask, &exclude_mask, &optional_mask, out);
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

    // --- _into API tests ---

    #[test]
    fn t_into_writes_into_buffer() {
        let mut registry = ArchetypeRegistry::new();
        registry.register_archetype(1, create_mask(&[1, 2]));
        registry.register_archetype(2, create_mask(&[1, 3]));

        let mut out: Vec<ArchetypeId> = Vec::new();
        registry.find_archetypes_with_components_into(&[1], &mut out);
        assert_eq!(out.len(), 2);
        assert!(out.contains(&1));
        assert!(out.contains(&2));
    }

    #[test]
    fn t_into_clears_buffer_on_entry() {
        let mut registry = ArchetypeRegistry::new();
        registry.register_archetype(10, create_mask(&[5, 6]));

        // Pre-fill with garbage values.
        let mut out: Vec<ArchetypeId> = vec![999, 888, 777];
        registry.find_archetypes_with_components_into(&[5], &mut out);

        // Garbage must be gone; only real matches remain.
        assert_eq!(out.len(), 1);
        assert!(out.contains(&10));
        assert!(!out.contains(&999));
    }

    #[test]
    fn t_into_is_zero_alloc_after_warmup() {
        let mut registry = ArchetypeRegistry::new();
        registry.register_archetype(1, create_mask(&[1, 2]));
        registry.register_archetype(2, create_mask(&[1, 3]));

        let mut out: Vec<ArchetypeId> = Vec::with_capacity(8);
        // Warmup: fill and let `out` grow to stable capacity.
        registry.find_archetypes_with_components_into(&[1], &mut out);
        out.clear();
        let cap_before = out.capacity();

        // Steady state: 1 000 calls — capacity must not grow.
        for _ in 0..1_000 {
            registry.find_archetypes_with_components_into(&[1], &mut out);
            out.clear();
        }
        assert_eq!(
            out.capacity(),
            cap_before,
            "capacity must not grow after warmup (no reallocations)"
        );
    }

    #[test]
    fn t_few_components_max_arity_dedup() {
        // Three component IDs all mapping to the same 64-bit block (block 0).
        // After dedup, relevant_blocks should contain exactly one entry.
        let comp_a: ComponentId = 1;  // block 0
        let comp_b: ComponentId = 2;  // block 0
        let comp_c: ComponentId = 3;  // block 0

        let mut registry = ArchetypeRegistry::new();
        registry.register_archetype(1, create_mask(&[comp_a, comp_b, comp_c]));
        registry.register_archetype(2, create_mask(&[comp_a, comp_b]));
        registry.register_archetype(3, create_mask(&[comp_a]));

        // Query for all three — only archetype 1 qualifies.
        let results = registry.find_archetypes_with_components(&[comp_a, comp_b, comp_c]);
        assert_eq!(results.len(), 1);
        assert!(results.contains(&1));

        // Via _into path — same result.
        let mut out = Vec::new();
        registry.find_archetypes_with_components_into(&[comp_a, comp_b, comp_c], &mut out);
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));
    }
}
