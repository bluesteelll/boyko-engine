/* use std::ops::{BitAnd, BitOr, BitXor, Not};
use std::fmt::Debug;
use std::hash::Hash;

use super::bit_storage::BitStorage;

/// A 512-bit bit set implemented using eight u64 values (8 * 64 = 512 bits)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BitSet512 {
    /// Eight u64 values storing the 512 bits
    pub bits: [u64; 8],
}

impl Default for BitSet512 {
    fn default() -> Self {
        Self { bits: [0; 8] }
    }
}

impl From<u8> for BitSet512 {
    fn from(value: u8) -> Self {
        let mut result = Self::default();
        result.bits[0] = value as u64;
        result
    }
}

impl BitAnd for BitSet512 {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        let mut result = Self::default();
        for i in 0..8 {
            result.bits[i] = self.bits[i] & rhs.bits[i];
        }
        result
    }
}

impl BitOr for BitSet512 {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        let mut result = Self::default();
        for i in 0..8 {
            result.bits[i] = self.bits[i] | rhs.bits[i];
        }
        result
    }
}

impl BitXor for BitSet512 {
    type Output = Self;

    fn bitxor(self, rhs: Self) -> Self::Output {
        let mut result = Self::default();
        for i in 0..8 {
            result.bits[i] = self.bits[i] ^ rhs.bits[i];
        }
        result
    }
}

impl Not for BitSet512 {
    type Output = Self;

    fn not(self) -> Self::Output {
        let mut result = Self::default();
        for i in 0..8 {
            result.bits[i] = !self.bits[i];
        }
        result
    }
}

impl BitStorage for BitSet512 {
    fn count_ones(&self) -> u32 {
        self.bits.iter().map(|&x| x.count_ones()).sum()
    }
    
    fn is_zero(&self) -> bool {
        self.bits.iter().all(|&x| x == 0)
    }
    
    fn get_bit(&self, position: u32) -> bool {
        let array_idx = (position / 64) as usize;
        let bit_idx = position % 64;
        
        if array_idx >= 8 {
            return false;
        }
        
        let mask = 1u64 << bit_idx;
        (self.bits[array_idx] & mask) != 0
    }
    
    fn with_bit_set(&self, position: u32) -> Self {
        let array_idx = (position / 64) as usize;
        let bit_idx = position % 64;
        
        if array_idx >= 8 {
            return *self;
        }
        
        let mut result = *self;
        let mask = 1u64 << bit_idx;
        result.bits[array_idx] |= mask;
        result
    }
    
    fn with_bit_cleared(&self, position: u32) -> Self {
        let array_idx = (position / 64) as usize;
        let bit_idx = position % 64;
        
        if array_idx >= 8 {
            return *self;
        }
        
        let mut result = *self;
        let mask = !(1u64 << bit_idx);
        result.bits[array_idx] &= mask;
        result
    }
}

impl BitSet512 {
    /// Number of 64-bit chunks in the bit set
    pub const CHUNK_COUNT: usize = 8;
    
    /// Number of bits per chunk
    pub const BITS_PER_CHUNK: usize = 64;
    
    /// Checks if this bit set is a superset of the other bit set
    /// Returns true if this bit set contains all bits set in other
    #[inline]
    pub fn contains_all(&self, other: &Self) -> bool {
        for i in 0..Self::CHUNK_COUNT {
            if (self.bits[i] & other.bits[i]) != other.bits[i] {
                return false;
            }
        }
        true
    }
    
    /// Checks if this bit set is a subset of the other bit set
    /// Returns true if all bits set in this bit set are also set in other
    #[inline]
    pub fn is_subset_of(&self, other: &Self) -> bool {
        other.contains_all(self)
    }
    
    /// Checks if the bit set has any bits in common with another bit set
    /// Returns true if at least one bit is set in both sets
    #[inline]
    pub fn has_any_common_bits(&self, other: &Self) -> bool {
        for i in 0..Self::CHUNK_COUNT {
            if (self.bits[i] & other.bits[i]) != 0 {
                return true;
            }
        }
        false
    }
    
    /// Gets a bit in a specific chunk at a specific position
    #[inline]
    pub fn get_bit_in_chunk(&self, chunk_idx: usize, bit_idx: usize) -> bool {
        if chunk_idx >= Self::CHUNK_COUNT || bit_idx >= Self::BITS_PER_CHUNK {
            return false;
        }
        
        let mask = 1u64 << bit_idx;
        (self.bits[chunk_idx] & mask) != 0
    }
    
    /// Checks if all bits not in the mask are zero
    /// Useful for filtering archetypes with only specific components
    #[inline]
    pub fn has_only_bits_in_mask(&self, mask: &Self) -> bool {
        for i in 0..Self::CHUNK_COUNT {
            // Calculate bits that are in self but not in the mask
            // If any such bits exist, then self has bits outside the mask
            if (self.bits[i] & !mask.bits[i]) != 0 {
                return false;
            }
        }
        true
    }
    
    /// Returns a new bit set that is the union of this bit set and another
    /// Equivalent to the | operator but more explicit
    #[inline]
    pub fn union(&self, other: &Self) -> Self {
        *self | *other
    }
    
    /// Returns a new bit set that is the intersection of this bit set and another
    /// Equivalent to the & operator but more explicit
    #[inline]
    pub fn intersection(&self, other: &Self) -> Self {
        *self & *other
    }
    
    /// Returns true if the bit set is empty (no bits set)
    /// Equivalent to is_zero() but more semantically clear
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.is_zero()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let empty = BitSet512::default();
        assert!(empty.is_zero());
        assert_eq!(empty.count_ones(), 0);
    }

    #[test]
    fn test_from_u8() {
        let bs = BitSet512::from(5u8); // binary 101
        assert_eq!(bs.bits[0], 5);
        assert_eq!(bs.count_ones(), 2);
        assert!(bs.get_bit(0)); // Check bit 1
        assert!(!bs.get_bit(1)); // Check bit 0
        assert!(bs.get_bit(2)); // Check bit 1
        assert!(!bs.get_bit(3)); // Check bit 0
    }

    #[test]
    fn test_bit_operations() {
        let mut bs = BitSet512::default();
        
        // Check the with_bit_set function
        let bs1 = bs.with_bit_set(5);
        assert_eq!(bs1.count_ones(), 1);
        assert!(bs1.get_bit(5));

        // Check setting bits in different u64 blocks
        let bs2 = bs1.with_bit_set(70); // second block
        assert_eq!(bs2.count_ones(), 2);
        assert!(bs2.get_bit(5));
        assert!(bs2.get_bit(70));

        // Check with_bit_cleared
        let bs3 = bs2.with_bit_cleared(5);
        assert_eq!(bs3.count_ones(), 1);
        assert!(!bs3.get_bit(5));
        assert!(bs3.get_bit(70));

        // Check block boundaries
        let bs4 = bs3.with_bit_set(63); // edge of the first block
        let bs5 = bs4.with_bit_set(64); // start of the second block
        assert_eq!(bs5.count_ones(), 3);
        assert!(bs5.get_bit(63));
        assert!(bs5.get_bit(64));

        // Check invalid positions
        let bs6 = bs5.with_bit_set(600); // out of bit set range
        assert_eq!(bs5, bs6); // must not change

        assert!(!bs5.get_bit(600)); // requesting an out-of-range bit must return false
    }

    #[test]
    fn test_bitwise_operations() {
        let mut bs1 = BitSet512::default();
        let mut bs2 = BitSet512::default();
        
        // Set bits in the first set
        let bs1 = bs1.with_bit_set(10).with_bit_set(20).with_bit_set(30);

        // Set bits in the second set
        let bs2 = bs2.with_bit_set(20).with_bit_set(30).with_bit_set(40);

        // Check AND
        let and_result = bs1 & bs2;
        assert_eq!(and_result.count_ones(), 2);
        assert!(!and_result.get_bit(10));
        assert!(and_result.get_bit(20));
        assert!(and_result.get_bit(30));
        assert!(!and_result.get_bit(40));

        // Check OR
        let or_result = bs1 | bs2;
        assert_eq!(or_result.count_ones(), 4);
        assert!(or_result.get_bit(10));
        assert!(or_result.get_bit(20));
        assert!(or_result.get_bit(30));
        assert!(or_result.get_bit(40));

        // Check XOR
        let xor_result = bs1 ^ bs2;
        assert_eq!(xor_result.count_ones(), 2);
        assert!(xor_result.get_bit(10));
        assert!(!xor_result.get_bit(20));
        assert!(!xor_result.get_bit(30));
        assert!(xor_result.get_bit(40));

        // Check NOT
        let not_result = !bs1;
        assert_eq!(not_result.count_ones(), 512 - 3); // inverted 3 bits
        assert!(!not_result.get_bit(10));
        assert!(!not_result.get_bit(20));
        assert!(!not_result.get_bit(30));
        assert!(not_result.get_bit(40));
    }

    #[test]
    fn test_large_positions() {
        let mut bs = BitSet512::default();
        
        // Check setting a bit in the last block
        let bs = bs.with_bit_set(500);
        assert_eq!(bs.count_ones(), 1);
        assert!(bs.get_bit(500));

        // Check the 512-bit boundary
        let bs = bs.with_bit_set(0).with_bit_set(511);
        assert_eq!(bs.count_ones(), 3);
        assert!(bs.get_bit(0));
        assert!(bs.get_bit(500));
        assert!(bs.get_bit(511));
    }

    #[test]
    fn test_across_all_blocks() {
        let mut bs = BitSet512::default();
        
        // Set one bit in each block
        let bs = bs
            .with_bit_set(0)     // block 0
            .with_bit_set(64)    // block 1
            .with_bit_set(128)   // block 2
            .with_bit_set(192)   // block 3
            .with_bit_set(256)   // block 4
            .with_bit_set(320)   // block 5
            .with_bit_set(384)   // block 6
            .with_bit_set(448);  // block 7

        assert_eq!(bs.count_ones(), 8);

        // Check the presence of bits
        assert!(bs.get_bit(0));
        assert!(bs.get_bit(64));
        assert!(bs.get_bit(128));
        assert!(bs.get_bit(192));
        assert!(bs.get_bit(256));
        assert!(bs.get_bit(320));
        assert!(bs.get_bit(384));
        assert!(bs.get_bit(448));

        // Check the absence of neighboring bits
        assert!(!bs.get_bit(1));
        assert!(!bs.get_bit(65));
        assert!(!bs.get_bit(129));
        assert!(!bs.get_bit(193));
        assert!(!bs.get_bit(257));
        assert!(!bs.get_bit(321));
        assert!(!bs.get_bit(385));
        assert!(!bs.get_bit(449));
    }
    
    #[test]
    fn test_new_methods() {
        let bs1 = BitSet512::default()
            .with_bit_set(1)
            .with_bit_set(2)
            .with_bit_set(3);
            
        let bs2 = BitSet512::default()
            .with_bit_set(2)
            .with_bit_set(3)
            .with_bit_set(4);
            
        let bs3 = BitSet512::default()
            .with_bit_set(1)
            .with_bit_set(2);
            
        // Test contains_all
        assert!(bs1.contains_all(&bs3));
        assert!(!bs3.contains_all(&bs1));
        
        // Test is_subset_of
        assert!(bs3.is_subset_of(&bs1));
        assert!(!bs1.is_subset_of(&bs3));
        
        // Test has_any_common_bits
        assert!(bs1.has_any_common_bits(&bs2));
        
        let bs4 = BitSet512::default().with_bit_set(10);
        let bs5 = BitSet512::default().with_bit_set(20);
        assert!(!bs4.has_any_common_bits(&bs5));
        
        // Test get_bit_in_chunk
        let bs6 = BitSet512::default().with_bit_set(65); // Bit 1 in chunk 1
        assert!(bs6.get_bit_in_chunk(1, 1));
        assert!(!bs6.get_bit_in_chunk(1, 2));
        
        // Test has_only_bits_in_mask
        let mask = BitSet512::default()
            .with_bit_set(1)
            .with_bit_set(2)
            .with_bit_set(3)
            .with_bit_set(4);
            
        assert!(bs3.has_only_bits_in_mask(&mask));
        assert!(!bs1.has_only_bits_in_mask(&bs3));
        
        // Test union and intersection
        let union = bs1.union(&bs2);
        let intersection = bs1.intersection(&bs2);
        
        assert_eq!(union.count_ones(), 4);
        assert_eq!(intersection.count_ones(), 2);
        
        // Test is_empty
        assert!(BitSet512::default().is_empty());
        assert!(!bs1.is_empty());
    }
} */ 