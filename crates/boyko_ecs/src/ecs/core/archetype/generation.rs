use std::num::NonZeroUsize;

/// Monotonic counter incremented on every `create_archetype`.
/// `NonZeroUsize` so `Option<ArchetypeGeneration>` fits in one word (niche optimization).
///
/// Never reset — not even by `ArchetypeMaster::clear()`. `QueryState` relies on
/// this invariant to detect stale caches after `clear()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct ArchetypeGeneration(NonZeroUsize);

impl ArchetypeGeneration {
    /// First generation value. Used as initial state in `ArchetypeMaster::new`
    /// and the floor for `Option<ArchetypeGeneration>::None`-replacement
    /// patterns in `QueryState`.
    pub(crate) const FIRST: Self = unsafe {
        // SAFETY: 1 is non-zero by construction.
        Self(NonZeroUsize::new_unchecked(1))
    };

    /// Monotonic step. Overflow at 2^63 increments is physically unreachable
    /// (290 years at 1 GHz bump rate); `wrapping_add` documents the policy.
    #[inline]
    pub(crate) fn bump(&mut self) {
        // SAFETY: wrapping usize+1 cannot produce zero unless start was usize::MAX,
        // which is unreachable per above. `new_unchecked` is sound.
        self.0 = unsafe { NonZeroUsize::new_unchecked(self.0.get().wrapping_add(1)) };
    }

    /// Returns the raw counter value.
    #[inline]
    pub fn get(&self) -> usize {
        self.0.get()
    }
}

impl Default for ArchetypeGeneration {
    #[inline]
    fn default() -> Self {
        Self::FIRST
    }
}
