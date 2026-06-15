/// Maximum number of distinct archetypes a single `ArchetypeMaster` will track.
/// Mirrors `MAX_COMPONENTS = 512`: a static upper bound that lets `QueryState`
/// preallocate without heap reallocation. Sized at 1024 because realistic ECS
/// workloads rarely exceed a few hundred archetypes (Bevy stress tests: <200);
/// 1024 gives 5x headroom at the cost of one extra cache line per bitset.
/// Overflow policy: release-mode panic via `expect` in `insert`/`contains`.
pub const MAX_ARCHETYPES: usize = 1024;

const ARCH_BITSET_WORDS: usize = MAX_ARCHETYPES / 64; // 16

/// Dense 1024-bit bitset for archetype-id dedup inside `QueryState`.
/// Fits in 2 cache lines (128 bytes). No heap allocation.
///
/// Sole purpose: O(1) "is this archetype already matched?" check during
/// `QueryState::update_archetypes` delta classification.
#[derive(Clone, Debug)]
#[repr(C, align(64))]
pub struct ArchetypeBitSet {
    bits: [u64; ARCH_BITSET_WORDS],
}

/// Cold path: panic for out-of-range archetype IDs.
///
/// Extracted from `insert`/`contains` hot bodies so the panic machinery
/// (format string, `format_args!`, stack unwinding prep) does not inflate
/// the hot-path binary size and pollute I-cache (principle #3 / #7).
#[cold]
#[inline(never)]
fn archetype_id_out_of_range(id: usize) -> ! {
    panic!(
        "invariant: archetype_id {} < MAX_ARCHETYPES ({})",
        id, MAX_ARCHETYPES
    );
}

impl ArchetypeBitSet {
    /// Creates an empty bitset.
    pub const fn new() -> Self {
        Self {
            bits: [0u64; ARCH_BITSET_WORDS],
        }
    }

    /// Marks the given archetype ID as present.
    ///
    /// # Panics
    /// Panics in all builds when `archetype_id >= MAX_ARCHETYPES`.
    /// Mirrors the `MAX_COMPONENTS` overflow policy in `ComponentRegistry`.
    #[inline]
    pub fn insert(&mut self, archetype_id: usize) {
        if archetype_id >= MAX_ARCHETYPES {
            archetype_id_out_of_range(archetype_id);
        }
        let w = archetype_id >> 6;
        let b = archetype_id & 63;
        self.bits[w] |= 1u64 << b;
    }

    /// Returns true if the given archetype ID is marked as present.
    ///
    /// # Panics
    /// Panics in all builds when `archetype_id >= MAX_ARCHETYPES`.
    #[inline]
    pub fn contains(&self, archetype_id: usize) -> bool {
        if archetype_id >= MAX_ARCHETYPES {
            archetype_id_out_of_range(archetype_id);
        }
        let w = archetype_id >> 6;
        let b = archetype_id & 63;
        (self.bits[w] >> b) & 1 == 1
    }

    /// Clears all bits in O(1) — single memset of 128 bytes.
    /// Used by `QueryState::reset`.
    #[inline]
    pub fn clear_all(&mut self) {
        self.bits = [0u64; ARCH_BITSET_WORDS];
    }

    /// Overwrites word `word_idx` with `value`.
    ///
    /// Used by `EnablePresence::snapshot_present` to build a plain bitset from
    /// the per-tag `AtomicU64` words in one pass (word index spaces coincide —
    /// both arrays are `MAX_ARCHETYPES / 64 == 16` words wide).
    ///
    /// # Panics
    /// Panics (debug) if `word_idx >= ARCH_BITSET_WORDS`.
    #[inline]
    pub fn set_word(&mut self, word_idx: usize, value: u64) {
        debug_assert!(
            word_idx < ARCH_BITSET_WORDS,
            "invariant: word_idx {} < ARCH_BITSET_WORDS ({})",
            word_idx,
            ARCH_BITSET_WORDS,
        );
        self.bits[word_idx] = value;
    }

    /// Clears the bit for `archetype_id`. Required by
    /// `QueryState::remove_matched_at` to preserve the M1/QS1
    /// dual-structure invariant.
    ///
    /// Idempotent: clearing an already-clear bit is a no-op. Out-of-range
    /// ids are silently ignored — `remove` is the inverse of `insert` only
    /// for in-range ids, and the dedup bitset never carries an
    /// out-of-range bit, so the no-op is sound.
    #[inline]
    pub fn remove(&mut self, archetype_id: usize) {
        if archetype_id >= MAX_ARCHETYPES {
            return;
        }
        let word_idx = archetype_id / 64;
        let bit = archetype_id % 64;
        self.bits[word_idx] &= !(1u64 << bit);
    }

    /// Returns the number of set bits. Used by
    /// `QueryDataState::assert_dual_invariant` (Phase 8b Step 6) to verify
    /// the M1/QS1 bijection between `matched_ids` and the bitset.
    #[inline]
    pub fn popcount(&self) -> u32 {
        self.bits.iter().map(|w| w.count_ones()).sum()
    }

    /// Invokes `f` with each set archetype id, in ascending order.
    ///
    /// The walk is **popcount-bounded**: each word is consumed via a
    /// `trailing_zeros` + clear-lowest-set-bit loop, so the body runs exactly
    /// `popcount()` times, never `MAX_ARCHETYPES`. This is the bounded
    /// enumeration primitive the EnableTag candidate-seeded global scan
    /// (`QueryState::seed_from_candidates`, amendment A1.3) walks — it must
    /// never degrade into a full `0..MAX_ARCHETYPES` sweep (the M2 hazard).
    #[inline]
    pub fn for_each_set_bit(&self, mut f: impl FnMut(usize)) {
        for (word_idx, &word) in self.bits.iter().enumerate() {
            let mut w = word;
            while w != 0 {
                let bit = w.trailing_zeros() as usize;
                f((word_idx << 6) | bit);
                // Clear the lowest set bit; the loop runs once per set bit.
                w &= w - 1;
            }
        }
    }
}

impl Default for ArchetypeBitSet {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set_contains_nothing() {
        let s = ArchetypeBitSet::new();
        assert!(!s.contains(0));
        assert!(!s.contains(63));
        assert!(!s.contains(64));
        assert!(!s.contains(1023));
    }

    #[test]
    fn boundary_inserts() {
        let mut s = ArchetypeBitSet::new();
        s.insert(0);
        s.insert(63);
        s.insert(64);
        s.insert(1023);
        assert!(s.contains(0));
        assert!(s.contains(63));
        assert!(s.contains(64));
        assert!(s.contains(1023));
        assert!(!s.contains(1)); // not inserted
        assert!(!s.contains(1022));
    }

    #[test]
    fn clear_all_resets() {
        let mut s = ArchetypeBitSet::new();
        for i in 0..1024 {
            s.insert(i);
        }
        assert!(s.contains(500));
        s.clear_all();
        for i in 0..1024 {
            assert!(!s.contains(i), "bit {} still set after clear_all", i);
        }
    }

    #[test]
    #[should_panic(expected = "invariant: archetype_id 1024")]
    fn insert_overflow_panics() {
        let mut s = ArchetypeBitSet::new();
        s.insert(1024);
    }

    #[test]
    #[should_panic(expected = "invariant: archetype_id 9999")]
    fn contains_overflow_panics() {
        let s = ArchetypeBitSet::new();
        let _ = s.contains(9999);
    }

    #[test]
    fn remove_clears_bit() {
        let mut s = ArchetypeBitSet::new();
        s.insert(42);
        assert!(s.contains(42), "precondition: bit must be set after insert");
        s.remove(42);
        assert!(!s.contains(42), "remove must clear the bit");
    }

    #[test]
    fn remove_idempotent() {
        let mut s = ArchetypeBitSet::new();
        s.insert(7);
        s.remove(7);
        // Second remove on already-clear bit: no panic, bit stays clear.
        s.remove(7);
        assert!(!s.contains(7), "double remove must leave bit clear");
    }

    #[test]
    fn remove_out_of_range_no_op() {
        let mut s = ArchetypeBitSet::new();
        s.insert(0);
        s.insert(1023);
        // Out-of-range remove must not panic and must not perturb in-range bits.
        s.remove(MAX_ARCHETYPES + 1);
        s.remove(usize::MAX);
        assert!(s.contains(0));
        assert!(s.contains(1023));
    }

    #[test]
    fn popcount_matches_set_bits() {
        let mut s = ArchetypeBitSet::new();
        assert_eq!(s.popcount(), 0, "empty bitset must have popcount 0");
        s.insert(0);
        s.insert(63);
        s.insert(64);
        s.insert(500);
        s.insert(1023);
        assert_eq!(s.popcount(), 5, "popcount must match inserted bit count");
        // After clearing one bit, popcount must drop by exactly 1.
        s.remove(500);
        assert_eq!(s.popcount(), 4, "popcount must decrement on remove");
    }
}
