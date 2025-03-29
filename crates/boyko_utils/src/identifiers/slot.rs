use crate::identifiers::primitives::Generation;

/// A Slot represents an index with a generation counter
/// to detect stale references and handle recycled indices safely
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Slot<T: Sized> {
    index: T,
    generation: Generation
}

impl<T> Slot<T>
where T: Sized + Copy + From<usize> + Into<usize>{
    #[inline(always)]
    pub fn new(index: T, generation: Generation) -> Self {
        Self { index, generation }
    }

    #[inline(always)]
    pub fn index(&self) -> T {
        self.index
    }

    #[inline(always)]
    pub fn generation(&self) -> Generation {
        self.generation
    }

    /// Increments the generation counter, wrapping around if necessary
    #[inline(always)]
    pub fn increment_generation(&mut self) -> Generation {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }
}