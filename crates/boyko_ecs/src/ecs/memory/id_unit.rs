/// Direct pointer to a component with metadata
#[derive(Debug, Clone, Copy)]
pub struct Unit {
    /// Direct pointer to the component in memory
    ptr: *mut u8,

    /// Index in the buffer (for quick calculation of buffer position)
    buffer_index: usize,
}

impl Unit {
    /// Creates a new Unit
    #[inline]
    pub fn new(ptr: *mut u8, buffer_index: usize) -> Self {
        Self {
            ptr,
            buffer_index,
        }
    }

    /// Returns the pointer to the component
    #[inline]
    pub fn ptr(&self) -> *mut u8 {
        self.ptr
    }

    /// Returns the buffer index
    #[inline]
    pub fn buffer_index(&self) -> usize {
        self.buffer_index
    }
}