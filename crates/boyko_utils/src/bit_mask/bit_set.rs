use std::fmt::{Debug, Formatter};
use std::ops::{BitAnd, BitOr, BitXor, Not, Shl, Shr, Sub};

/// Trait for supported integer types that can be used in BitSet
/// Defines the requirements and operations for bit manipulation
pub trait BitInteger: 
    Copy + Default + PartialEq + 
    BitOr<Output = Self> + BitAnd<Output = Self> + BitXor<Output = Self> + Not<Output = Self> +
    Shl<usize, Output = Self> + Shr<usize, Output = Self> + From<u8> + Sub<Output = Self>
{
    /// Number of bits in this integer type
    const BITS: usize;
    
    /// Count the number of set bits (1s)
    fn count_ones(self) -> u32;
    
    /// Count leading zeros (from the most significant bit)
    fn leading_zeros(self) -> u32;
    
    /// Count trailing zeros (from the least significant bit)
    fn trailing_zeros(self) -> u32;
}

// Implementation of BitInteger trait for supported types
impl BitInteger for u8 {
    const BITS: usize = 8;
    
    #[inline(always)]
    fn count_ones(self) -> u32 { self.count_ones() }
    
    #[inline(always)]
    fn leading_zeros(self) -> u32 { self.leading_zeros() }
    
    #[inline(always)]
    fn trailing_zeros(self) -> u32 { self.trailing_zeros() }
}

impl BitInteger for u32 {
    const BITS: usize = 32;
    
    #[inline(always)]
    fn count_ones(self) -> u32 { self.count_ones() }
    
    #[inline(always)]
    fn leading_zeros(self) -> u32 { self.leading_zeros() }
    
    #[inline(always)]
    fn trailing_zeros(self) -> u32 { self.trailing_zeros() }
}

impl BitInteger for u64 {
    const BITS: usize = 64;
    
    #[inline(always)]
    fn count_ones(self) -> u32 { self.count_ones() }
    
    #[inline(always)]
    fn leading_zeros(self) -> u32 { self.leading_zeros() }
    
    #[inline(always)]
    fn trailing_zeros(self) -> u32 { self.trailing_zeros() }
}

impl BitInteger for u128 {
    const BITS: usize = 128;
    
    #[inline(always)]
    fn count_ones(self) -> u32 { self.count_ones() }
    
    #[inline(always)]
    fn leading_zeros(self) -> u32 { self.leading_zeros() }
    
    #[inline(always)]
    fn trailing_zeros(self) -> u32 { self.trailing_zeros() }
}

/// BitSet - A wrapper around integer types for convenient bit operations
/// Provides high-performance bit manipulation methods and bitwise operations
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct BitSet<T: BitInteger> {
    bits: T,
}

impl<T: BitInteger> BitSet<T> {
    /// Creates a new empty BitSet (all bits set to 0)
    #[inline(always)]
    pub fn new() -> Self {
        Self { bits: T::default() }
    }

    /// Creates a BitSet with the specified initial value
    #[inline(always)]
    pub fn from_value(value: T) -> Self {
        Self { bits: value }
    }

    /// Returns the underlying value
    #[inline(always)]
    pub fn value(&self) -> T {
        self.bits
    }

    /// Sets the bit at position 'index' to 1
    #[inline(always)]
    pub fn set(&mut self, index: usize) {
        debug_assert!(index < T::BITS, "Bit index out of range");
        self.bits = self.bits | (T::from(1) << index);
    }

    /// Sets the bit at position 'index' to 0
    #[inline(always)]
    pub fn unset(&mut self, index: usize) {
        debug_assert!(index < T::BITS, "Bit index out of range");
        self.bits = self.bits & !(T::from(1) << index);
    }

    /// Creates a new BitSet with the bit at position 'index' set to 1
    #[inline(always)]
    pub fn with_bit(mut self, index: usize) -> Self {
        self.set(index);
        self
    }

    /// Clears the bit at position 'index' (sets to 0)
    #[inline(always)]
    pub fn clear(&mut self, index: usize) {
        debug_assert!(index < T::BITS, "Bit index out of range");
        self.bits = self.bits & !(T::from(1) << index);
    }

    /// Toggles (inverts) the bit at position 'index'
    #[inline(always)]
    pub fn toggle(&mut self, index: usize) {
        debug_assert!(index < T::BITS, "Bit index out of range");
        self.bits = self.bits ^ (T::from(1) << index);
    }

    /// Checks if the bit at position 'index' is set (is 1)
    #[inline(always)]
    pub fn is_set(&self, index: usize) -> bool {
        debug_assert!(index < T::BITS, "Bit index out of range");
        let mask = T::from(1) << index;
        (self.bits & mask) == mask
    }

    /// Returns a bitmask with only the bit at position 'index' set to 1
    #[inline(always)]
    pub fn bit_mask(index: usize) -> Self {
        debug_assert!(index < T::BITS, "Bit index out of range");
        Self { bits: T::from(1) << index }
    }

    /// Checks if the BitSet is empty (all bits are 0)
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.bits == T::default()
    }

    /// Counts the number of set bits (1s)
    #[inline(always)]
    pub fn count_ones(&self) -> u32 {
        self.bits.count_ones()
    }
    
    /// Returns the index of the first set bit, or None if no bits are set
    #[inline(always)]
    pub fn first_set_bit(&self) -> Option<usize> {
        if self.bits == T::default() {
            None
        } else {
            Some(self.bits.trailing_zeros() as usize)
        }
    }

    /// Returns the index of the next set bit after 'after_index', or None if no more bits are set
    pub fn next_set_bit(&self, after_index: usize) -> Option<usize> {
        if after_index >= T::BITS - 1 {
            return None;
        }

        // Create a mask that zeroes all bits up to after_index (inclusive)
        let mask = !(((T::from(1) << (after_index + 1)) - T::from(1)));
        let masked_bits = self.bits & mask;

        if masked_bits == T::default() {
            None
        } else {
            Some(masked_bits.trailing_zeros() as usize)
        }
    }

    /// Returns the result of bitwise AND with another BitSet
    #[inline(always)]
    pub fn and(&self, other: &Self) -> Self {
        Self { bits: self.bits & other.bits }
    }

    /// Applies bitwise AND with another BitSet in-place
    #[inline(always)]
    pub fn and_assign(&mut self, other: &Self) {
        self.bits = self.bits & other.bits;
    }

    /// Returns the result of bitwise OR with another BitSet
    #[inline(always)]
    pub fn or(&self, other: &Self) -> Self {
        Self { bits: self.bits | other.bits }
    }

    /// Applies bitwise OR with another BitSet in-place
    #[inline(always)]
    pub fn or_assign(&mut self, other: &Self) {
        self.bits = self.bits | other.bits;
    }

    /// Returns the result of bitwise XOR with another BitSet
    #[inline(always)]
    pub fn xor(&self, other: &Self) -> Self {
        Self { bits: self.bits ^ other.bits }
    }

    /// Applies bitwise XOR with another BitSet in-place
    #[inline(always)]
    pub fn xor_assign(&mut self, other: &Self) {
        self.bits = self.bits ^ other.bits;
    }

    /// Returns the result of bitwise NOT (complement)
    #[inline(always)]
    pub fn not(&self) -> Self {
        Self { bits: !self.bits }
    }

    /// Returns the result of shifting the bits left by 'shift' positions
    #[inline(always)]
    pub fn shift_left(&self, shift: usize) -> Self {
        Self { bits: self.bits << shift }
    }

    /// Returns the result of shifting the bits right by 'shift' positions
    #[inline(always)]
    pub fn shift_right(&self, shift: usize) -> Self {
        Self { bits: self.bits >> shift }
    }
    
    /// Creates an iterator over all set bits (positions of 1s)
    pub fn iter_ones(&self) -> BitSetIterator<T> {
        BitSetIterator {
            bitset: *self,
            next_index: 0,
        }
    }
}

/// Iterator over set bits in a BitSet
pub struct BitSetIterator<T: BitInteger> {
    bitset: BitSet<T>,
    next_index: usize,
}

impl<T: BitInteger> Iterator for BitSetIterator<T> {
    type Item = usize;
    
    fn next(&mut self) -> Option<Self::Item> {
        if self.next_index >= T::BITS {
            return None;
        }
        
        // Find the next set bit
        let masked_bits = self.bitset.bits & !((T::from(1) << self.next_index) - T::from(1));
        
        if masked_bits == T::default() {
            None
        } else {
            let pos = masked_bits.trailing_zeros() as usize;
            self.next_index = pos + 1;
            Some(pos)
        }
    }
}

/// Operator implementations for convenient usage

impl<T: BitInteger> BitAnd for BitSet<T> {
    type Output = Self;
    
    #[inline(always)]
    fn bitand(self, rhs: Self) -> Self::Output {
        Self { bits: self.bits & rhs.bits }
    }
}

impl<T: BitInteger> BitOr for BitSet<T> {
    type Output = Self;
    
    #[inline(always)]
    fn bitor(self, rhs: Self) -> Self::Output {
        Self { bits: self.bits | rhs.bits }
    }
}

impl<T: BitInteger> BitXor for BitSet<T> {
    type Output = Self;
    
    #[inline(always)]
    fn bitxor(self, rhs: Self) -> Self::Output {
        Self { bits: self.bits ^ rhs.bits }
    }
}

impl<T: BitInteger> Not for BitSet<T> {
    type Output = Self;
    
    #[inline(always)]
    fn not(self) -> Self::Output {
        Self { bits: !self.bits }
    }
}

impl<T: BitInteger> Default for BitSet<T> {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

impl<T: BitInteger + Debug> Debug for BitSet<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "BitSet({:?}) [", self.bits)?;
        
        let mut first = true;
        for bit in self.iter_ones() {
            if !first {
                write!(f, ", ")?;
            }
            first = false;
            write!(f, "{}", bit)?;
        }
        
        write!(f, "]")
    }
}