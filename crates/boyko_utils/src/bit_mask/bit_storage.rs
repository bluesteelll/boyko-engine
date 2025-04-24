/* use std::ops::{BitAnd, BitOr, BitXor, Not};
use std::fmt::Debug;
use std::hash::Hash;

/// Trait for types that can be used as bit storage
pub trait BitStorage: 
    Clone + Copy + PartialEq + Eq + Debug + Hash +
    BitAnd<Output = Self> + BitOr<Output = Self> + BitXor<Output = Self> + Not<Output = Self> +
    From<u8> + Default
{
    /// Returns the number of one bits in the binary representation of self
    fn count_ones(&self) -> u32;
    
    /// Check if all bits are zero
    fn is_zero(&self) -> bool;
    
    /// Get the bit at the given position
    fn get_bit(&self, position: u32) -> bool;
    
    /// Set the bit at the given position
    fn with_bit_set(&self, position: u32) -> Self;
    
    /// Clear the bit at the given position
    fn with_bit_cleared(&self, position: u32) -> Self;
}

// Implement BitStorage for common integer types
macro_rules! impl_bit_storage {
    ($($t:ty),*) => {
        $(
            impl BitStorage for $t {
                fn count_ones(&self) -> u32 {
                    <$t>::count_ones(*self)
                }
                
                fn is_zero(&self) -> bool {
                    *self == <$t>::from(0u8)
                }
                
                fn get_bit(&self, position: u32) -> bool {
                    let mask = <$t>::from(1u8) << position;
                    (*self & mask) != <$t>::from(0u8)
                }
                
                fn with_bit_set(&self, position: u32) -> Self {
                    let mask = <$t>::from(1u8) << position;
                    *self | mask
                }
                
                fn with_bit_cleared(&self, position: u32) -> Self {
                    let mask = !(<$t>::from(1u8) << position);
                    *self & mask
                }
            }
        )*
    };
}

// Implement for standard integer types
impl_bit_storage!(u8, u16, u32, u64, u128, usize);  */