/// Fixed-size 256-bit bitset backed by four `u64` words.
///
/// Aligned to 32 B so the whole set fits in a single AVX2 register and
/// straddles at most one cache line on x86_64 (32 < 64).
///
/// Bit `i` lives in `words[i >> 6]` at bit position `i & 63`.
/// Word 0 carries bits 0..=63, word 1 carries bits 64..=127, and so on.
///
/// # Usage
///
/// ```
/// use boyko_utils::bit_mask::bit_set_256::BitSet256;
///
/// let mut bs = BitSet256::new();
/// bs.set(7);
/// assert!(bs.get(7));
/// assert_eq!(bs.count_ones(), 1);
/// ```
#[derive(Copy, Clone, PartialEq, Eq, Default)]
#[repr(C, align(32))]
pub struct BitSet256 {
    /// Little-endian word order: word 0 = bits 0..=63, word 1 = bits 64..=127,
    /// word 2 = bits 128..=191, word 3 = bits 192..=255.
    words: [u64; 4],
}

const _: () = assert!(core::mem::size_of::<BitSet256>() == 32);
const _: () = assert!(core::mem::align_of::<BitSet256>() == 32);

impl BitSet256 {
    /// Returns an all-zeros (empty) bitset.
    #[inline]
    pub const fn new() -> Self {
        Self { words: [0u64; 4] }
    }

    /// Sets bit `index` to 1.
    ///
    /// # Panics (debug only)
    /// Panics if `index >= 256`.
    #[inline]
    pub fn set(&mut self, index: usize) {
        debug_assert!(index < 256, "bit index out of range: {index}");
        self.words[index >> 6] |= 1u64 << (index & 63);
    }

    /// Clears bit `index` to 0.
    ///
    /// # Panics (debug only)
    /// Panics if `index >= 256`.
    #[inline]
    pub fn clear(&mut self, index: usize) {
        debug_assert!(index < 256, "bit index out of range: {index}");
        self.words[index >> 6] &= !(1u64 << (index & 63));
    }

    /// Returns `true` if bit `index` is set.
    ///
    /// # Panics (debug only)
    /// Panics if `index >= 256`.
    #[inline]
    pub fn get(&self, index: usize) -> bool {
        debug_assert!(index < 256, "bit index out of range: {index}");
        (self.words[index >> 6] >> (index & 63)) & 1 == 1
    }

    /// Returns `true` iff no bits are set.
    #[inline]
    pub fn is_empty(&self) -> bool {
        (self.words[0] | self.words[1] | self.words[2] | self.words[3]) == 0
    }

    /// Returns `true` iff every one of the 256 bits is set.
    ///
    /// Used by the Phase 9 parallel scheduler to detect "universal access"
    /// (e.g. exclusive systems / `ApplyDeferred`) whose access surface covers
    /// the entire resource or event space — see `Access::is_universal`.
    ///
    /// Lowers to four 64-bit equality compares; constant-fold friendly when
    /// `self` is known statically.
    #[inline]
    pub fn is_all_set(&self) -> bool {
        self.words[0] == u64::MAX
            && self.words[1] == u64::MAX
            && self.words[2] == u64::MAX
            && self.words[3] == u64::MAX
    }

    /// Sets every one of the 256 bits to 1.
    ///
    /// Used by the Phase 9 parallel scheduler `Access::universal()` constructor
    /// when building the access surface for exclusive systems.
    #[inline]
    pub fn set_all(&mut self) {
        self.words[0] = u64::MAX;
        self.words[1] = u64::MAX;
        self.words[2] = u64::MAX;
        self.words[3] = u64::MAX;
    }

    /// Returns the total number of set bits (popcount across all four words).
    #[inline]
    pub fn count_ones(&self) -> u32 {
        self.words[0].count_ones()
            + self.words[1].count_ones()
            + self.words[2].count_ones()
            + self.words[3].count_ones()
    }

    /// Returns `true` iff `self` and `other` share at least one set bit.
    ///
    /// Tests the four 64-bit words in turn — the loop is bounded and fully
    /// unrolled by the compiler. Lowers to four `AND` + `OR` instructions on
    /// x86_64; the whole bitset fits in one cache line so the cost is one
    /// L1d load per operand.
    #[inline]
    pub fn intersects(&self, other: &Self) -> bool {
        // Word 0..=3; bounded loop, compiler unrolls.
        for i in 0..4usize {
            if (self.words[i] & other.words[i]) != 0 {
                return true;
            }
        }
        false
    }

    /// Removes and returns the index of the lowest set bit, or `None` if empty.
    ///
    /// Uses the BLSR-equivalent `word & (word - 1)` to clear the lowest set bit
    /// in O(1). On x86_64 with BMI1, the compiler lowers `word.trailing_zeros()`
    /// to `TZCNT` and the clear to `BLSR` (1-3 cycles combined).
    ///
    /// Designed for sparse iteration:
    ///
    /// ```
    /// use boyko_utils::bit_mask::bit_set_256::BitSet256;
    ///
    /// let mut mask = BitSet256::new();
    /// mask.set(5);
    /// mask.set(100);
    /// while let Some(idx) = mask.pop_lowest_set_bit() {
    ///     // process idx
    ///     let _ = idx;
    /// }
    /// assert!(mask.is_empty());
    /// ```
    ///
    /// Total cost for k set bits is O(k), not O(256). Reads at most one cache line.
    #[inline]
    pub fn pop_lowest_set_bit(&mut self) -> Option<u32> {
        // Scan words 0..4 for first non-zero.
        // Loop is bounded (max 4 iterations); compiler unrolls due to constant bound.
        for w in 0..4usize {
            let word = self.words[w];
            if word != 0 {
                let bit = word.trailing_zeros();
                // Clear lowest set bit: BLSR-equivalent (x & (x - 1)).
                self.words[w] = word & word.wrapping_sub(1);
                return Some((w as u32) * 64 + bit);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::BitSet256;

    #[test]
    fn set_get_clear_basic() {
        let mut bs = BitSet256::new();
        for &idx in &[0usize, 63, 64, 127, 255] {
            bs.set(idx);
            assert!(bs.get(idx), "bit {idx} should be set");
        }
        for &idx in &[0usize, 63, 64, 127, 255] {
            bs.clear(idx);
            assert!(!bs.get(idx), "bit {idx} should be cleared");
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    fn bounds_panic_debug() {
        let result = std::panic::catch_unwind(|| {
            let mut bs = BitSet256::new();
            bs.set(256);
        });
        assert!(result.is_err(), "set(256) must panic in debug mode");
    }

    #[test]
    fn count_ones_matches_sets() {
        let mut bs = BitSet256::new();
        let indices = [1usize, 2, 63, 64, 100, 128, 200, 255];
        for &i in &indices {
            bs.set(i);
        }
        assert_eq!(bs.count_ones(), indices.len() as u32);
        // Clear some and verify again.
        bs.clear(2);
        bs.clear(128);
        assert_eq!(bs.count_ones(), (indices.len() - 2) as u32);
    }

    #[test]
    fn pop_lowest_empty_returns_none() {
        let mut bs = BitSet256::new();
        assert_eq!(bs.pop_lowest_set_bit(), None);
    }

    #[test]
    fn pop_lowest_iteration_order() {
        let mut bs = BitSet256::new();
        bs.set(200);
        bs.set(5);
        bs.set(100);
        assert_eq!(bs.pop_lowest_set_bit(), Some(5));
        assert_eq!(bs.pop_lowest_set_bit(), Some(100));
        assert_eq!(bs.pop_lowest_set_bit(), Some(200));
        assert_eq!(bs.pop_lowest_set_bit(), None);
    }

    #[test]
    fn pop_lowest_consumes() {
        let mut bs = BitSet256::new();
        bs.set(10);
        bs.set(42);
        bs.set(255);
        while bs.pop_lowest_set_bit().is_some() {}
        assert!(bs.is_empty());
    }

    #[test]
    fn bit_set_256_is_all_set_returns_true_when_full() {
        let mut bs = BitSet256::new();
        assert!(!bs.is_all_set(), "empty bitset must not report all-set");
        for i in 0..256usize {
            bs.set(i);
        }
        assert!(bs.is_all_set(), "after setting every bit, is_all_set must be true");
        // Clearing a single bit must flip the result.
        bs.clear(123);
        assert!(!bs.is_all_set(), "clearing any bit must drop is_all_set to false");
    }

    #[test]
    fn bit_set_256_set_all_then_is_all_set() {
        let mut bs = BitSet256::new();
        bs.set_all();
        assert!(bs.is_all_set());
        assert_eq!(bs.count_ones(), 256);
        // Every individual bit is observable as set.
        for i in [0usize, 1, 63, 64, 127, 128, 191, 192, 254, 255] {
            assert!(bs.get(i), "bit {i} must be set after set_all");
        }
    }
}
