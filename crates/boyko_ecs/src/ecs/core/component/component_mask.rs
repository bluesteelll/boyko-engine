use crate::ecs::identifiers::primitives::ComponentId;
use boyko_utils::bit_mask::bit_set::BitSet;
/// 512-bit component mask
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(align(32))] 
pub struct ComponentMask {
    pub blocks: [BitSet<u64>; 8],
}

impl ComponentMask {
    pub fn new() -> Self {
        Self { blocks: [BitSet::new(); 8] }
    }
    
    #[inline]
    pub fn set(&mut self, component_id: ComponentId) {
        let block = (component_id / 64) % 8;
        let bit = component_id % 64;
        self.blocks[block].set(bit as usize);
    }
    
    #[inline]
    pub fn unset(&mut self, component_id: ComponentId) {
        let block = (component_id / 64) % 8;
        let bit = component_id % 64;
        self.blocks[block].clear(bit as usize);
    }
    
    #[inline]
    pub fn contains(&self, component_id: ComponentId) -> bool {
        let block = (component_id / 64) % 8;
        let bit = component_id % 64;
        self.blocks[block].is_set(bit as usize)
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
}