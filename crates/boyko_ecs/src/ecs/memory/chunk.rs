use std::alloc::Layout;
use std::ptr::NonNull;
use crate::ecs::memory::arena::Arena;
use crate::ecs::constants::DEFAULT_COMPONENTS_PER_CHUNK;

/// A chunk stores a fixed number of components of the same type.
/// This implementation is type-agnostic and only deals with raw memory.
pub struct Chunk {
    /// Raw pointer to the allocated memory
    data: NonNull<u8>,

    /// Maximum number of components this chunk can hold
    capacity: usize,

    /// Current number of occupied slots
    count: usize,
}

impl Chunk {
    /// Creates a new chunk with memory allocated for components with specified layout
    pub fn new(arena: &Arena, capacity: usize, component_layout: Layout) -> Self {
        // Calculate memory layout for the component array
        let array_layout = Layout::array::<u8>(capacity * component_layout.size())
            .expect("Invalid array layout for chunk")
            .align_to(component_layout.align())
            .expect("Invalid alignment for chunk");

        // Allocate memory in the arena
        let ptr = arena.allocate_layout(array_layout);

        Self {
            data: ptr,
            capacity,
            count: 0,
        }
    }

    /// Creates a new chunk with default capacity
    pub fn with_default_capacity(arena: &Arena, component_layout: Layout) -> Self {
        Self::new(arena, DEFAULT_COMPONENTS_PER_CHUNK, component_layout)
    }

    //
    // Raw memory operations
    //

    /// Raw add operation - adds raw bytes to the chunk
    ///
    /// # Safety
    /// Caller must ensure the bytes represent a valid component
    pub unsafe fn raw_add(&mut self, bytes: *const u8, layout: Layout) -> Option<usize> {
        if self.count >= self.capacity {
            return None;
        }

        let index = self.count;
        let dst = self.data.as_ptr().add(index * layout.size());

        // Copy the component data
        std::ptr::copy_nonoverlapping(bytes, dst, layout.size());

        self.count += 1;
        Some(index)
    }

    /// Raw set operation - overwrites a component with raw bytes
    ///
    /// # Safety
    /// Caller must ensure the bytes represent a valid component
    pub unsafe fn raw_set(&mut self, index: usize, bytes: *const u8, layout: Layout) -> bool {
        if index >= self.capacity {
            return false;
        }

        let dst = self.data.as_ptr().add(index * layout.size());

        // Copy the new component data
        std::ptr::copy_nonoverlapping(bytes, dst, layout.size());

        // Update count if necessary
        if index >= self.count {
            self.count = index + 1;
        }

        true
    }

    /// Raw get operation - returns a raw pointer to the component's bytes
    pub fn raw_get(&self, index: usize, layout: Layout) -> Option<*const u8> {
        if index >= self.count {
            return None;
        }

        let ptr = unsafe { self.data.as_ptr().add(index * layout.size()) };
        Some(ptr)
    }

    /// Raw get mut operation - returns a mutable raw pointer to the component's bytes
    pub fn raw_get_mut(&mut self, index: usize, layout: Layout) -> Option<*mut u8> {
        if index >= self.count {
            return None;
        }

        let ptr = unsafe { self.data.as_ptr().add(index * layout.size()) };
        Some(ptr)
    }

    /// Removes a component, swapping it with the last component for O(1) removal
    pub fn swap_remove(&mut self, index: usize, layout: Layout) -> bool {
        if index >= self.count {
            return false;
        }

        // If it's not the last element, swap with the last one
        if index < self.count - 1 {
            let last_index = self.count - 1;

            unsafe {
                let src = self.data.as_ptr().add(last_index * layout.size());
                let dst = self.data.as_ptr().add(index * layout.size());
                std::ptr::copy_nonoverlapping(src, dst, layout.size());
            }
        }

        self.count -= 1;
        true
    }

    /// Removes a component, shifting all subsequent elements
    pub fn remove(&mut self, index: usize, layout: Layout) -> bool {
        if index >= self.count {
            return false;
        }

        // Move all subsequent elements one position back
        let elements_to_move = self.count - index - 1;
        if elements_to_move > 0 {
            unsafe {
                let src = self.data.as_ptr().add((index + 1) * layout.size());
                let dst = self.data.as_ptr().add(index * layout.size());
                std::ptr::copy(src, dst, elements_to_move * layout.size());
            }
        }

        self.count -= 1;
        true
    }

    /// Clears the chunk, resetting the count without deallocating
    pub fn clear(&mut self) {
        // Just reset the count - we don't need to run destructors since
        // the memory is managed by the arena
        self.count = 0;
    }

    //
    // Accessor methods
    //

    #[inline]
    pub fn count(&self) -> usize {
        self.count
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.count >= self.capacity
    }

    #[inline]
    pub fn data_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    #[inline]
    pub fn data_ptr_mut(&mut self) -> *mut u8 {
        self.data.as_ptr()
    }
}