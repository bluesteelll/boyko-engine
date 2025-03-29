use crate::identifiers::primitives::Generation;

/// A Slot represents an index with a generation counter
/// to detect stale references and handle recycled indices safely
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Slot {
    index: usize,
    generation: Generation
}

impl Slot {
    /// Creates a new slot with the specified index and generation
    #[inline(always)]
    pub fn new(index: usize, generation: Generation) -> Self {
        Self { index, generation }
    }

    /// Returns the index component of the slot
    #[inline(always)]
    pub fn index(&self) -> usize {
        self.index
    }

    /// Returns the generation component of the slot
    #[inline(always)]
    pub fn generation(&self) -> Generation {
        self.generation
    }

    /// Increments the generation counter, wrapping around if necessary
    /// Returns the new generation value
    #[inline(always)]
    pub fn increment_generation(&mut self) -> Generation {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }
}