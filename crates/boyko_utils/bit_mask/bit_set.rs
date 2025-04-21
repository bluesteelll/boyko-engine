use std::ops::{BitAnd, BitOr, BitXor, Not, Sub};
use std::fmt::Debug;
use std::hash::Hash;

use super::bit_storage::BitStorage;

/// A generic bit set structure that supports various bitwise operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BitSet<T: BitStorage> {
    bits: T,
}

impl<T: BitStorage> BitSet<T> {
    /// Create a new empty bit set.
    pub fn new() -> Self {
        Self { bits: T::default() }
    }

    /// Create a bit set with the given bits.
    pub fn from_bits(bits: T) -> Self {
        Self { bits }
    }

    /// Get the underlying bits.
    pub fn bits(&self) -> T {
        self.bits
    }

    /// Set a specific bit at the given position.
    pub fn set(&mut self, position: u32) {
        self.bits = self.bits.with_bit_set(position);
    }

    /// Create a new BitSet with a bit set at the given position.
    pub fn with_bit_set(&self, position: u32) -> Self {
        Self::from_bits(self.bits.with_bit_set(position))
    }

    /// Clear a specific bit at the given position.
    pub fn clear(&mut self, position: u32) {
        self.bits = self.bits.with_bit_cleared(position);
    }

    /// Create a new BitSet with a bit cleared at the given position.
    pub fn with_bit_cleared(&self, position: u32) -> Self {
        Self::from_bits(self.bits.with_bit_cleared(position))
    }

    /// Check if a specific bit is set.
    pub fn is_set(&self, position: u32) -> bool {
        self.bits.get_bit(position)
    }

    /// Check if the bit set is empty.
    pub fn is_empty(&self) -> bool {
        self.bits.is_zero()
    }

    /// Count the number of set bits.
    pub fn count_ones(&self) -> u32 {
        self.bits.count_ones()
    }
}

// Default implementation for BitSet
impl<T: BitStorage> Default for BitSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

// Intersection operation using BitAnd trait
impl<T: BitStorage> BitAnd for BitSet<T> {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self::from_bits(self.bits & rhs.bits)
    }
}

// Union operation using BitOr trait
impl<T: BitStorage> BitOr for BitSet<T> {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self::from_bits(self.bits | rhs.bits)
    }
}

// Difference operation using Sub trait
impl<T: BitStorage> Sub for BitSet<T> {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::from_bits(self.bits & !rhs.bits)
    }
}

// Symmetric difference operation using BitXor trait
impl<T: BitStorage> BitXor for BitSet<T> {
    type Output = Self;

    fn bitxor(self, rhs: Self) -> Self::Output {
        Self::from_bits(self.bits ^ rhs.bits)
    }
}

// Complement operation using Not trait
impl<T: BitStorage> Not for BitSet<T> {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self::from_bits(!self.bits)
    }
}

// Implement reference versions of the operations
impl<T: BitStorage> BitAnd<&BitSet<T>> for BitSet<T> {
    type Output = Self;

    fn bitand(self, rhs: &Self) -> Self::Output {
        Self::from_bits(self.bits & rhs.bits)
    }
}

impl<T: BitStorage> BitAnd<BitSet<T>> for &BitSet<T> {
    type Output = BitSet<T>;

    fn bitand(self, rhs: BitSet<T>) -> Self::Output {
        BitSet::from_bits(self.bits & rhs.bits)
    }
}

impl<T: BitStorage> BitAnd for &BitSet<T> {
    type Output = BitSet<T>;

    fn bitand(self, rhs: Self) -> Self::Output {
        BitSet::from_bits(self.bits & rhs.bits)
    }
}

impl<T: BitStorage> BitOr<&BitSet<T>> for BitSet<T> {
    type Output = Self;

    fn bitor(self, rhs: &Self) -> Self::Output {
        Self::from_bits(self.bits | rhs.bits)
    }
}

impl<T: BitStorage> BitOr<BitSet<T>> for &BitSet<T> {
    type Output = BitSet<T>;

    fn bitor(self, rhs: BitSet<T>) -> Self::Output {
        BitSet::from_bits(self.bits | rhs.bits)
    }
}

impl<T: BitStorage> BitOr for &BitSet<T> {
    type Output = BitSet<T>;

    fn bitor(self, rhs: Self) -> Self::Output {
        BitSet::from_bits(self.bits | rhs.bits)
    }
}

impl<T: BitStorage> BitXor<&BitSet<T>> for BitSet<T> {
    type Output = Self;

    fn bitxor(self, rhs: &Self) -> Self::Output {
        Self::from_bits(self.bits ^ rhs.bits)
    }
}

impl<T: BitStorage> BitXor<BitSet<T>> for &BitSet<T> {
    type Output = BitSet<T>;

    fn bitxor(self, rhs: BitSet<T>) -> Self::Output {
        BitSet::from_bits(self.bits ^ rhs.bits)
    }
}

impl<T: BitStorage> BitXor for &BitSet<T> {
    type Output = BitSet<T>;

    fn bitxor(self, rhs: Self) -> Self::Output {
        BitSet::from_bits(self.bits ^ rhs.bits)
    }
}

impl<T: BitStorage> Sub<&BitSet<T>> for BitSet<T> {
    type Output = Self;

    fn sub(self, rhs: &Self) -> Self::Output {
        Self::from_bits(self.bits & !rhs.bits)
    }
}

impl<T: BitStorage> Sub<BitSet<T>> for &BitSet<T> {
    type Output = BitSet<T>;

    fn sub(self, rhs: BitSet<T>) -> Self::Output {
        BitSet::from_bits(self.bits & !rhs.bits)
    }
}

impl<T: BitStorage> Sub for &BitSet<T> {
    type Output = BitSet<T>;

    fn sub(self, rhs: Self) -> Self::Output {
        BitSet::from_bits(self.bits & !rhs.bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_u8_operations() {
        let mut mask1: BitSet<u8> = BitSet::new();
        mask1.set(0);
        mask1.set(2);
        mask1.set(4);
        
        let mut mask2: BitSet<u8> = BitSet::new();
        mask2.set(0);
        mask2.set(3);
        mask2.set(5);
        
        // Test intersection (AND)
        let intersection = mask1 & mask2;
        assert_eq!(intersection.bits(), 1); // Only bit 0 is common
        
        // Test union (OR)
        let union = mask1 | mask2;
        assert_eq!(union.bits(), 0b101101); // Bits 0, 2, 3, 4, 5
        
        // Test difference
        let difference = mask1 - mask2;
        assert_eq!(difference.bits(), 0b10100); // Bits 2, 4
        
        // Test symmetric difference (XOR)
        let sym_diff = mask1 ^ mask2;
        assert_eq!(sym_diff.bits(), 0b101100); // Bits 2, 3, 4, 5
        
        // Test complement (NOT)
        let complement = !mask1;
        assert_eq!(complement.bits(), !0b10101 as u8);
    }
    
    #[test]
    fn test_u16_operations() {
        let mut mask1: BitSet<u16> = BitSet::new();
        mask1.set(8);
        mask1.set(10);
        
        let mut mask2: BitSet<u16> = BitSet::new();
        mask2.set(8);
        mask2.set(12);
        
        // Test intersection (AND)
        let intersection = mask1 & mask2;
        assert_eq!(intersection.bits(), 0x100); // Only bit 8 is common
        
        // Test union (OR)
        let union = mask1 | mask2;
        assert_eq!(union.bits(), 0x1500); // Bits 8, 10, 12
        
        // Test reference operations
        let result1 = &mask1 & &mask2;
        assert_eq!(result1.bits(), 0x100);
    }
    
    #[test]
    fn test_u32_operations() {
        let mut mask1: BitSet<u32> = BitSet::new();
        mask1.set(16);
        mask1.set(20);
        
        let mut mask2: BitSet<u32> = BitSet::new();
        mask2.set(16);
        mask2.set(24);
        
        let result = mask1 & mask2;
        assert_eq!(result.bits(), 0x10000); // Only bit 16 is common
    }
    
    #[test]
    fn test_u64_operations() {
        let mut mask1: BitSet<u64> = BitSet::new();
        mask1.set(32);
        mask1.set(40);
        
        let mut mask2: BitSet<u64> = BitSet::new();
        mask2.set(32);
        mask2.set(48);
        
        let result = mask1 & mask2;
        assert_eq!(result.bits(), 0x100000000); // Only bit 32 is common
    }
    
    #[test]
    fn test_u128_operations() {
        let mut mask1: BitSet<u128> = BitSet::new();
        mask1.set(64);
        mask1.set(80);
        
        let mut mask2: BitSet<u128> = BitSet::new();
        mask2.set(64);
        mask2.set(96);
        
        let result = mask1 & mask2;
        assert_eq!(result.bits(), 0x1_0000_0000_0000_0000); // Only bit 64 is common
    }
}