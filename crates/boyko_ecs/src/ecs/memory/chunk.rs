/// Lightweight chunk holding only metadata
pub struct Chunk {
    /// Start index in the component buffer
    start_index: usize,

    /// Chunk capacity
    capacity: usize,

    /// Flag indicating that data has been modified
    is_dirty: bool,
}

impl Chunk {
    /// Creates a new chunk
    pub fn new(start_index: usize, capacity: usize) -> Self {
        Self {
            start_index,
            capacity,
            is_dirty: false,
        }
    }

    /// Returns the start index
    #[inline]
    pub fn start_index(&self) -> usize {
        self.start_index
    }

    /// Returns the capacity
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Marks the chunk as dirty
    #[inline]
    pub fn mark_dirty(&mut self) {
        self.is_dirty = true;
    }

    /// Checks if the chunk is dirty
    #[inline]
    pub fn is_dirty(&self) -> bool {
        self.is_dirty
    }

    /// Clears the dirty flag
    #[inline]
    pub fn clear_dirty_flag(&mut self) {
        self.is_dirty = false;
    }
}