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
        assert!(
            archetype_id < MAX_ARCHETYPES,
            "invariant: archetype_id {} < MAX_ARCHETYPES ({})",
            archetype_id,
            MAX_ARCHETYPES,
        );
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
        assert!(
            archetype_id < MAX_ARCHETYPES,
            "invariant: archetype_id {} < MAX_ARCHETYPES ({})",
            archetype_id,
            MAX_ARCHETYPES,
        );
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
}
