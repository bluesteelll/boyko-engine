use std::ops::{BitAnd, BitOr, BitXor, Not};
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
        assert!(bs.get_bit(0)); // Проверка бита 1
        assert!(!bs.get_bit(1)); // Проверка бита 0
        assert!(bs.get_bit(2)); // Проверка бита 1
        assert!(!bs.get_bit(3)); // Проверка бита 0
    }

    #[test]
    fn test_bit_operations() {
        let mut bs = BitSet512::default();
        
        // Проверка функции with_bit_set
        let bs1 = bs.with_bit_set(5);
        assert_eq!(bs1.count_ones(), 1);
        assert!(bs1.get_bit(5));
        
        // Проверка установки битов в разных блоках u64
        let bs2 = bs1.with_bit_set(70); // второй блок
        assert_eq!(bs2.count_ones(), 2);
        assert!(bs2.get_bit(5));
        assert!(bs2.get_bit(70));
        
        // Проверка with_bit_cleared
        let bs3 = bs2.with_bit_cleared(5);
        assert_eq!(bs3.count_ones(), 1);
        assert!(!bs3.get_bit(5));
        assert!(bs3.get_bit(70));
        
        // Проверка границ блоков
        let bs4 = bs3.with_bit_set(63); // край первого блока
        let bs5 = bs4.with_bit_set(64); // начало второго блока
        assert_eq!(bs5.count_ones(), 3);
        assert!(bs5.get_bit(63));
        assert!(bs5.get_bit(64));
        
        // Проверка на невалидных позициях
        let bs6 = bs5.with_bit_set(600); // за пределами битсета
        assert_eq!(bs5, bs6); // не должно измениться
        
        assert!(!bs5.get_bit(600)); // запрос бита за пределами должен вернуть false
    }

    #[test]
    fn test_bitwise_operations() {
        let mut bs1 = BitSet512::default();
        let mut bs2 = BitSet512::default();
        
        // Установим биты в первом множестве
        let bs1 = bs1.with_bit_set(10).with_bit_set(20).with_bit_set(30);
        
        // Установим биты во втором множестве
        let bs2 = bs2.with_bit_set(20).with_bit_set(30).with_bit_set(40);
        
        // Проверка AND
        let and_result = bs1 & bs2;
        assert_eq!(and_result.count_ones(), 2);
        assert!(!and_result.get_bit(10));
        assert!(and_result.get_bit(20));
        assert!(and_result.get_bit(30));
        assert!(!and_result.get_bit(40));
        
        // Проверка OR
        let or_result = bs1 | bs2;
        assert_eq!(or_result.count_ones(), 4);
        assert!(or_result.get_bit(10));
        assert!(or_result.get_bit(20));
        assert!(or_result.get_bit(30));
        assert!(or_result.get_bit(40));
        
        // Проверка XOR
        let xor_result = bs1 ^ bs2;
        assert_eq!(xor_result.count_ones(), 2);
        assert!(xor_result.get_bit(10));
        assert!(!xor_result.get_bit(20));
        assert!(!xor_result.get_bit(30));
        assert!(xor_result.get_bit(40));
        
        // Проверка NOT
        let not_result = !bs1;
        assert_eq!(not_result.count_ones(), 512 - 3); // инвертировали 3 бита
        assert!(!not_result.get_bit(10));
        assert!(!not_result.get_bit(20));
        assert!(!not_result.get_bit(30));
        assert!(not_result.get_bit(40));
    }

    #[test]
    fn test_large_positions() {
        let mut bs = BitSet512::default();
        
        // Проверяем установку бита в последнем блоке
        let bs = bs.with_bit_set(500);
        assert_eq!(bs.count_ones(), 1);
        assert!(bs.get_bit(500));
        
        // Проверяем границы 512 бит
        let bs = bs.with_bit_set(0).with_bit_set(511);
        assert_eq!(bs.count_ones(), 3);
        assert!(bs.get_bit(0));
        assert!(bs.get_bit(500));
        assert!(bs.get_bit(511));
    }

    #[test]
    fn test_across_all_blocks() {
        let mut bs = BitSet512::default();
        
        // Устанавливаем по одному биту в каждом блоке
        let bs = bs
            .with_bit_set(0)     // блок 0
            .with_bit_set(64)    // блок 1
            .with_bit_set(128)   // блок 2
            .with_bit_set(192)   // блок 3
            .with_bit_set(256)   // блок 4
            .with_bit_set(320)   // блок 5
            .with_bit_set(384)   // блок 6
            .with_bit_set(448);  // блок 7
        
        assert_eq!(bs.count_ones(), 8);
        
        // Проверяем наличие битов
        assert!(bs.get_bit(0));
        assert!(bs.get_bit(64));
        assert!(bs.get_bit(128));
        assert!(bs.get_bit(192));
        assert!(bs.get_bit(256));
        assert!(bs.get_bit(320));
        assert!(bs.get_bit(384));
        assert!(bs.get_bit(448));
        
        // Проверяем отсутствие соседних битов
        assert!(!bs.get_bit(1));
        assert!(!bs.get_bit(65));
        assert!(!bs.get_bit(129));
        assert!(!bs.get_bit(193));
        assert!(!bs.get_bit(257));
        assert!(!bs.get_bit(321));
        assert!(!bs.get_bit(385));
        assert!(!bs.get_bit(449));
    }
} 