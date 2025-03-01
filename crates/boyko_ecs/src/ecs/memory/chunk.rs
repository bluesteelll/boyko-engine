use std::ptr::NonNull;
use std::marker::PhantomData;
use std::mem::MaybeUninit;

use super::arena::Arena;
use super::utils::{align_up, next_power_of_2, calculate_simd_chunk_size, is_power_of_two};
use crate::ecs::core::component::Component;
use crate::ecs::constants::{DEFAULT_COMPONENTS_PER_CHUNK, MIN_ALIGNMENT, CACHE_LINE_SIZE};

/// A contiguous block of memory for storing components of a specific type
#[repr(align(64))]
pub struct Chunk<T: Component> {
    /// Pointer to component data
    ptr: NonNull<MaybeUninit<T>>, // Use MaybeUninit for uninitialized memory

    /// Maximum number of components this chunk can hold
    capacity: usize,

    /// Current number of allocated components
    count: usize,

    /// Number of active components (allocated - freed)
    active_count: usize,

    /// Size of the chunk in bytes
    chunk_size: usize,

    alignment: usize,

    component_size: usize,

    /// Free slots bitmap for tracking free components
    /// Each u64 can track 64 components
    free_bitmap: Vec<u64>,

    _marker: PhantomData<T>,
}

unsafe impl<T: Component> Send for Chunk<T> {}
unsafe impl<T: Component> Sync for Chunk<T> {}

impl<T: Component> Chunk<T> {
    #[inline(always)]
    pub fn new(arena: &mut Arena, component_count: usize) -> Option<Self> {
        let component_size = std::mem::size_of::<T>();

        // Ensure proper alignment for SIMD operations
        let alignment = std::mem::align_of::<T>().max(MIN_ALIGNMENT);

        // Calculate aligned component size
        let aligned_component_size = align_up(component_size, alignment);

        // Calculate chunk size with padding for proper alignment
        // Ensure chunk size is a multiple of cache line size for better performance
        let chunk_size = align_up(aligned_component_size * component_count, CACHE_LINE_SIZE);

        // Allocate memory from arena
        let memory_ptr = arena.allocate(chunk_size, CACHE_LINE_SIZE)?;

        // Calculate number of u64 needed for bitmap
        let bitmap_size = (component_count + 63) / 64;

        // Create all-ones bitmap (all slots free)
        let mut free_bitmap = Vec::with_capacity(bitmap_size);
        for i in 0..bitmap_size {
            let remaining = component_count - i * 64;
            let bits = if remaining >= 64 {
                !0u64 // All 64 bits set
            } else {
                (1u64 << remaining) - 1 // Just the bits we need
            };
            free_bitmap.push(bits);
        }

        Some(Self {
            ptr: unsafe { NonNull::new_unchecked(memory_ptr.as_ptr() as *mut MaybeUninit<T>) },
            capacity: component_count,
            count: 0,
            active_count: 0,
            chunk_size,
            alignment,
            component_size: aligned_component_size,
            free_bitmap,
            _marker: PhantomData,
        })
    }

    #[inline(always)]
    pub fn with_default_component_count(arena: &mut Arena) -> Option<Self> {
        Self::new(arena, DEFAULT_COMPONENTS_PER_CHUNK)
    }

    /// Allocate a component slot in this chunk
    /// Returns the index of the allocated component or None if chunk is full
    #[inline(always)]
    pub fn allocate_component(&mut self) -> Option<usize> {
        // Fast path: check if we're at capacity
        if self.count >= self.capacity {
            return None;
        }

        // If we have free slots (from previously freed components)
        if self.count > self.active_count {
            // Find first free slot using bitmap
            for (bitmap_idx, bitmap) in self.free_bitmap.iter_mut().enumerate() {
                if *bitmap != 0 {
                    // Found a bitmap with free slots
                    let bit_pos = bitmap.trailing_zeros() as usize;

                    // Calculate global index
                    let index = bitmap_idx * 64 + bit_pos;

                    // Ensure index is valid
                    if index >= self.capacity {
                        continue; // This shouldn't happen with proper bitmap setup
                    }

                    // Mark slot as used (clear bit)
                    *bitmap &= !(1u64 << bit_pos);

                    // Update active count
                    self.active_count += 1;

                    return Some(index);
                }
            }
        }

        // No free slots found, allocate at the end
        let index = self.count;

        // Update bitmap (mark as used)
        let bitmap_idx = index / 64;
        let bit_pos = index % 64;

        if bitmap_idx < self.free_bitmap.len() {
            self.free_bitmap[bitmap_idx] &= !(1u64 << bit_pos);
        }

        // Update counts
        self.count += 1;
        self.active_count += 1;

        Some(index)
    }

    #[inline(always)]
    pub fn free_component(&mut self, index: usize) {
        debug_assert!(index < self.capacity,
                      "Component index out of bounds: {} >= {}",
                      index, self.capacity);

        // Update bitmap (mark as free)
        let bitmap_idx = index / 64;
        let bit_pos = index % 64;

        if bitmap_idx < self.free_bitmap.len() {
            // Check if the slot is already freed
            if (self.free_bitmap[bitmap_idx] & (1u64 << bit_pos)) != 0 {
                return; // Already freed
            }

            // Mark as free
            self.free_bitmap[bitmap_idx] |= 1u64 << bit_pos;
        }

        // Update active count
        if self.active_count > 0 {
            self.active_count -= 1;
        }

        // Zero the memory for safety
        unsafe {
            std::ptr::write(self.get_component_ptr(index), MaybeUninit::zeroed());
        }
    }

    #[inline(always)]
    pub unsafe fn get_component_ptr(&self, index: usize) -> *mut MaybeUninit<T> {
        debug_assert!(index < self.capacity,
                      "Component index out of bounds: {} >= {}",
                      index, self.capacity);

        // Calculate pointer
        self.ptr.as_ptr().add(index)
    }

    #[inline(always)]
    pub fn is_occupied(&self, index: usize) -> bool {
        if index >= self.capacity || index >= self.count {
            return false;
        }

        let bitmap_idx = index / 64;
        let bit_pos = index % 64;

        if bitmap_idx < self.free_bitmap.len() {
            // Bit is 0 if occupied, 1 if free
            (self.free_bitmap[bitmap_idx] & (1u64 << bit_pos)) == 0
        } else {
            false
        }
    }

    /// Get a reference to a component
    /// SAFETY: index must be valid and the component must be initialized
    #[inline(always)]
    pub unsafe fn get_component(&self, index: usize) -> &T {
        debug_assert!(self.is_occupied(index), "Trying to access freed component at index {}", index);
        (*self.get_component_ptr(index)).assume_init_ref()
    }

    /// Get a mutable reference to a component
    /// SAFETY: index must be valid and the component must be initialized
    #[inline(always)]
    pub unsafe fn get_component_mut(&self, index: usize) -> &mut T {
        debug_assert!(self.is_occupied(index), "Trying to access freed component at index {}", index);
        (*self.get_component_ptr(index)).assume_init_mut()
    }

    /// Write a component at the specified index
    /// SAFETY: index must be valid
    #[inline(always)]
    pub unsafe fn write_component(&self, index: usize, value: T) {
        debug_assert!(index < self.capacity,
                      "Component index out of bounds: {} >= {}",
                      index, self.capacity);

        // Write the value
        (*self.get_component_ptr(index)).write(value);
    }

    /// Process components in a specific range
    #[inline(always)]
    pub unsafe fn process_components_in_range<F>(&self, start: usize, end: usize, mut processor: F)
    where
        F: FnMut(usize, &mut T)
    {
        let real_end = end.min(self.count);

        if start >= real_end {
            return;
        }

        // Determine the SIMD processing approach
        let simd_width = self.get_simd_width();

        if simd_width > 1 && self.count == self.active_count {
            // All slots are occupied - use fast SIMD path

            // Process in SIMD-friendly groups
            let simd_start = align_up(start, simd_width);
            let simd_end = real_end & !(simd_width - 1); // Round down to simd width

            // Process pre-SIMD elements
            for i in start..simd_start.min(real_end) {
                processor(i, self.get_component_mut(i));
            }

            // SIMD-friendly processing of aligned components
            for i in (simd_start..simd_end).step_by(simd_width) {
                // Process SIMD group
                for j in 0..simd_width {
                    processor(i + j, self.get_component_mut(i + j));
                }
            }

            // Process post-SIMD elements
            for i in simd_end..real_end {
                processor(i, self.get_component_mut(i));
            }
        } else {
            // Mixed occupied/free slots - check bitmap
            for i in start..real_end {
                if self.is_occupied(i) {
                    processor(i, self.get_component_mut(i));
                }
            }
        }
    }

    /// Determine the SIMD width based on component type
    #[inline(always)]
    fn get_simd_width(&self) -> usize {
        // Determine appropriate SIMD width based on component size and alignment
        calculate_simd_chunk_size(std::mem::size_of::<T>(), self.alignment)
    }

    /// Compact the chunk by moving components to fill gaps
    /// Returns the new count of components
    ///
    /// Note: This method requires T to implement Copy or Clone
    pub fn compact<C: FnMut(usize, usize)>(&mut self, mut on_component_moved: C) -> usize {
        // Skip if no fragmentation
        if self.count == self.active_count {
            return self.count;
        }

        // Find gaps and move components to fill them
        let mut read_idx = self.count - 1;
        let mut write_idx = 0;

        // Find first free slot from the beginning
        while write_idx < self.count && self.is_occupied(write_idx) {
            write_idx += 1;
        }

        // No free slots found
        if write_idx >= self.count {
            return self.count;
        }

        // Find occupied slots from the end to move into gaps
        while read_idx > write_idx {
            // Skip free slots
            while read_idx > write_idx && !self.is_occupied(read_idx) {
                read_idx -= 1;
            }

            // Skip occupied slots in the write region
            while write_idx < read_idx && self.is_occupied(write_idx) {
                write_idx += 1;
            }

            // Move component if we found a valid pair
            if read_idx > write_idx && self.is_occupied(read_idx) && !self.is_occupied(write_idx) {
                unsafe {
                    // Copy memory directly instead of cloning
                    let src_ptr = self.get_component_ptr(read_idx);
                    let dst_ptr = self.get_component_ptr(write_idx);

                    // Copy the component memory directly
                    std::ptr::copy_nonoverlapping(
                        src_ptr,
                        dst_ptr,
                        1
                    );

                    // Update bitmap
                    let read_bitmap_idx = read_idx / 64;
                    let read_bit_pos = read_idx % 64;
                    let write_bitmap_idx = write_idx / 64;
                    let write_bit_pos = write_idx % 64;

                    self.free_bitmap[read_bitmap_idx] |= 1u64 << read_bit_pos;

                    self.free_bitmap[write_bitmap_idx] &= !(1u64 << write_bit_pos);

                    // Notify about component movement
                    on_component_moved(read_idx, write_idx);
                }

                // Move indices
                read_idx -= 1;
                write_idx += 1;
            }
        }

        // Update count to reflect the compacted state
        self.count = self.active_count;

        // Return new count
        self.count
    }

    #[inline(always)]
    pub fn component_count(&self) -> usize {
        self.count
    }

    #[inline(always)]
    pub fn active_component_count(&self) -> usize {
        self.active_count
    }

    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get chunk size in bytes
    #[inline(always)]
    pub fn size(&self) -> usize {
        self.chunk_size
    }

    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.active_count >= self.capacity
    }

    /// Check fragmentation ratio (0.0 = no fragmentation, 1.0 = fully fragmented)
    #[inline(always)]
    pub fn fragmentation_ratio(&self) -> f32 {
        if self.count == 0 {
            return 0.0;
        }

        1.0 - (self.active_count as f32 / self.count as f32)
    }

    /// Clearing all components
    pub fn reset(&mut self) {
        // Reset counts
        self.count = 0;
        self.active_count = 0;

        // Reset free bitmap
        for i in 0..self.free_bitmap.len() {
            let remaining = self.capacity - i * 64;
            self.free_bitmap[i] = if remaining >= 64 {
                !0u64 // All 64 bits set
            } else {
                (1u64 << remaining) - 1 // Just the bits we need
            };
        }

        // Zero memory for safety when reusing
        unsafe {
            std::ptr::write_bytes(self.ptr.as_ptr() as *mut u8, 0, self.chunk_size);
        }
    }
}