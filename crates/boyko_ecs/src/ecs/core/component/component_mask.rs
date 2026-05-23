use crate::ecs::identifiers::primitives::ComponentId;
use crate::ecs::core::component::component_registry::MAX_COMPONENTS;
use boyko_utils::bit_mask::bit_set::BitSet;
use std::ops::{BitAnd, BitOr, BitXor, Not};
/// 512-bit component mask
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(align(32))] 
pub struct ComponentMask {
    pub blocks: [BitSet<u64>; 8],
}

impl Default for ComponentMask {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentMask {
    pub fn new() -> Self {
        Self { blocks: [BitSet::new(); 8] }
    }
    
    #[inline]
    pub fn set(&mut self, component_id: ComponentId) {
        debug_assert!(
            component_id < MAX_COMPONENTS,
            "ComponentId {component_id} out of range (MAX_COMPONENTS = {MAX_COMPONENTS})"
        );
        let block = component_id / 64;
        let bit = component_id % 64;
        self.blocks[block].set(bit);
    }

    #[inline]
    pub fn unset(&mut self, component_id: ComponentId) {
        debug_assert!(
            component_id < MAX_COMPONENTS,
            "ComponentId {component_id} out of range (MAX_COMPONENTS = {MAX_COMPONENTS})"
        );
        let block = component_id / 64;
        let bit = component_id % 64;
        self.blocks[block].clear(bit);
    }

    #[inline]
    pub fn contains(&self, component_id: ComponentId) -> bool {
        debug_assert!(
            component_id < MAX_COMPONENTS,
            "ComponentId {component_id} out of range (MAX_COMPONENTS = {MAX_COMPONENTS})"
        );
        let block = component_id / 64;
        let bit = component_id % 64;
        self.blocks[block].is_set(bit)
    }
    
    pub fn from_components(components: &[ComponentId]) -> Self {
        let mut mask = Self::new();
        for &comp_id in components {
            mask.set(comp_id);
        }
        mask
    }

    /// Performs a union operation between this mask and another
    /// Returns a new mask with bits set that are in either this mask or the other
    pub fn union(&self, other: &Self) -> Self {
        let mut result = Self::new();
        for i in 0..8 {
            result.blocks[i] = self.blocks[i] | other.blocks[i];
        }
        result
    }

    /// Updates this mask to be the union of itself and another mask
    pub fn union_with(&mut self, other: &Self) {
        for i in 0..8 {
            self.blocks[i] = self.blocks[i] | other.blocks[i];
        }
    }

    /// Performs an intersection operation between this mask and another
    /// Returns a new mask with bits set only where both masks have bits set
    pub fn intersection(&self, other: &Self) -> Self {
        let mut result = Self::new();
        for i in 0..8 {
            result.blocks[i] = self.blocks[i] & other.blocks[i];
        }
        result
    }

    /// Updates this mask to be the intersection of itself and another mask
    pub fn intersection_with(&mut self, other: &Self) {
        for i in 0..8 {
            self.blocks[i] = self.blocks[i] & other.blocks[i];
        }
    }

    /// Performs a difference operation (this - other)
    /// Returns a new mask with bits set that are in this mask but not in the other
    pub fn difference(&self, other: &Self) -> Self {
        let mut result = Self::new();
        for i in 0..8 {
            result.blocks[i] = self.blocks[i] & !other.blocks[i];
        }
        result
    }

    /// Updates this mask to be the difference of itself and another mask
    pub fn difference_with(&mut self, other: &Self) {
        for i in 0..8 {
            self.blocks[i] = self.blocks[i] & !other.blocks[i];
        }
    }

    /// Performs a symmetric difference operation
    /// Returns a new mask with bits set that are in either this mask or the other, but not both
    pub fn symmetric_difference(&self, other: &Self) -> Self {
        let mut result = Self::new();
        for i in 0..8 {
            result.blocks[i] = self.blocks[i] ^ other.blocks[i];
        }
        result
    }

    /// Updates this mask to be the symmetric difference of itself and another mask
    pub fn symmetric_difference_with(&mut self, other: &Self) {
        for i in 0..8 {
            self.blocks[i] = self.blocks[i] ^ other.blocks[i];
        }
    }

    /// Returns true if this mask is a subset of the other mask
    pub fn is_subset(&self, other: &Self) -> bool {
        for i in 0..8 {
            if (self.blocks[i] & other.blocks[i]) != self.blocks[i] {
                return false;
            }
        }
        true
    }

    /// Returns true if this mask is empty (no bits set)
    pub fn is_empty(&self) -> bool {
        for i in 0..8 {
            if self.blocks[i] != BitSet::new() {
                return false;
            }
        }
        true
    }

    /// Returns the count of set bits across all 8 blocks.
    pub fn popcount(&self) -> usize {
        self.blocks.iter().map(|b| b.count_ones() as usize).sum()
    }

    /// Returns true if this mask shares any bits with another mask
    /// This is useful for determining if a component set intersects with another
    pub fn intersects(&self, other: &Self) -> bool {
        for i in 0..8 {
            if !(self.blocks[i] & other.blocks[i]).is_empty() {
                return true;
            }
        }
        false
    }
}

/// Implement the bitwise AND operator for ComponentMask
/// This performs an intersection of two masks
impl BitAnd for ComponentMask {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        self.intersection(&rhs)
    }
}

/// Implement the bitwise AND operator for references
impl BitAnd for &ComponentMask {
    type Output = ComponentMask;

    fn bitand(self, rhs: Self) -> Self::Output {
        self.intersection(rhs)
    }
}

/// Implement the bitwise OR operator for ComponentMask
/// This performs a union of two masks
impl BitOr for ComponentMask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(&rhs)
    }
}

/// Implement the bitwise OR operator for references
impl BitOr for &ComponentMask {
    type Output = ComponentMask;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

/// Implement the bitwise XOR operator for ComponentMask
/// This performs a symmetric difference of two masks
impl BitXor for ComponentMask {
    type Output = Self;

    fn bitxor(self, rhs: Self) -> Self::Output {
        self.symmetric_difference(&rhs)
    }
}

/// Implement the bitwise XOR operator for references
impl BitXor for &ComponentMask {
    type Output = ComponentMask;

    fn bitxor(self, rhs: Self) -> Self::Output {
        self.symmetric_difference(rhs)
    }
}

/// Implement the bitwise NOT operator for ComponentMask
/// This inverts all bits in the mask
impl Not for ComponentMask {
    type Output = Self;

    fn not(self) -> Self::Output {
        let mut result = Self::new();
        for i in 0..8 {
            result.blocks[i] = !self.blocks[i];
        }
        result
    }
}

/// Implement the bitwise NOT operator for references
impl Not for &ComponentMask {
    type Output = ComponentMask;

    fn not(self) -> Self::Output {
        let mut result = ComponentMask::new();
        for i in 0..8 {
            result.blocks[i] = !self.blocks[i];
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operator_and() {
        let mut mask1 = ComponentMask::new();
        let mut mask2 = ComponentMask::new();

        // Set bits in mask1: 1, 2, 3
        mask1.set(1);
        mask1.set(2);
        mask1.set(3);

        // Set bits in mask2: 2, 3, 4
        mask2.set(2);
        mask2.set(3);
        mask2.set(4);

        // Result should have bits 2, 3 set
        let result = mask1 & mask2;
        assert!(result.contains(2));
        assert!(result.contains(3));
        assert!(!result.contains(1));
        assert!(!result.contains(4));

        // Test reference version
        let result_ref = mask1 & mask2;
        assert!(result_ref.contains(2));
        assert!(result_ref.contains(3));
        assert!(!result_ref.contains(1));
        assert!(!result_ref.contains(4));
    }

    #[test]
    fn test_operator_or() {
        let mut mask1 = ComponentMask::new();
        let mut mask2 = ComponentMask::new();

        // Set bits in mask1: 1, 2
        mask1.set(1);
        mask1.set(2);

        // Set bits in mask2: 2, 3
        mask2.set(2);
        mask2.set(3);

        // Result should have bits 1, 2, 3 set
        let result = mask1 | mask2;
        assert!(result.contains(1));
        assert!(result.contains(2));
        assert!(result.contains(3));

        // Test reference version
        let result_ref = mask1 | mask2;
        assert!(result_ref.contains(1));
        assert!(result_ref.contains(2));
        assert!(result_ref.contains(3));
    }

    #[test]
    fn test_operator_xor() {
        let mut mask1 = ComponentMask::new();
        let mut mask2 = ComponentMask::new();

        // Set bits in mask1: 1, 2, 3
        mask1.set(1);
        mask1.set(2);
        mask1.set(3);

        // Set bits in mask2: 2, 3, 4
        mask2.set(2);
        mask2.set(3);
        mask2.set(4);

        // Result should have bits 1, 4 set (symmetric difference)
        let result = mask1 ^ mask2;
        assert!(result.contains(1));
        assert!(!result.contains(2));
        assert!(!result.contains(3));
        assert!(result.contains(4));

        // Test reference version
        let result_ref = mask1 ^ mask2;
        assert!(result_ref.contains(1));
        assert!(!result_ref.contains(2));
        assert!(!result_ref.contains(3));
        assert!(result_ref.contains(4));
    }

    #[test]
    fn test_operator_not() {
        let mut mask = ComponentMask::new();
        
        // Set bits 1, 2, 3
        mask.set(1);
        mask.set(2);
        mask.set(3);
        
        // Invert the mask - all bits should be set except 1, 2, 3
        let result = !mask;
        
        // Check the first few bits
        assert!(!result.contains(1));
        assert!(!result.contains(2));
        assert!(!result.contains(3));
        assert!(result.contains(4));
        assert!(result.contains(5));
        
        // Test reference version
        let result_ref = !&mask;
        assert!(!result_ref.contains(1));
        assert!(!result_ref.contains(2));
        assert!(!result_ref.contains(3));
        assert!(result_ref.contains(4));
        assert!(result_ref.contains(5));
    }

    /// Verify that `set` panics in debug builds when `component_id >= MAX_COMPONENTS`.
    /// The `% 8` was previously silently wrapping; removing it and adding
    /// `debug_assert!` makes out-of-range access detectable at test/dev time.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "out of range")]
    fn test_set_out_of_range_panics() {
        let mut mask = ComponentMask::new();
        mask.set(MAX_COMPONENTS); // must panic
    }
}