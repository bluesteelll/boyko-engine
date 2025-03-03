use std::ptr::NonNull;
use std::marker::PhantomData;
use std::mem::MaybeUninit;

use super::arena::Arena;
use super::utils::{align_up, next_power_of_2, calculate_simd_chunk_size, is_power_of_two, test_bit, set_bit, clear_bit};
use crate::ecs::core::component::Component;
use crate::ecs::constants::{DEFAULT_COMPONENTS_PER_CHUNK, MIN_ALIGNMENT, CACHE_LINE_SIZE};

/// A contiguous block of memory for storing components of a specific type
/// Optimized for cache locality and SIMD operations
#[repr(align(64))]
pub struct Chunk<T: Component> {
    /// Pointer to component data (using MaybeUninit for uninitialized memory)
    ptr: NonNull<MaybeUninit<T>>,

    /// Maximum number of components this chunk can hold
    capacity: usize,

    /// Current number of allocated components
    count: usize,

    /// Number of active components (allocated - freed)
    active_count: usize,

    /// Size of the chunk in bytes
    chunk_size: usize,

    /// Component alignment
    alignment: usize,

    /// Size of each aligned component
    component_size: usize,

    /// Free slots bitmap (1=free, 0=occupied)
    /// Each u64 can track 64 components
    free_bitmap: Vec<u64>,

    /// Marker for component type
    _marker: PhantomData<T>,
}

unsafe impl<T: Component> Send for Chunk<T> {}
unsafe impl<T: Component> Sync for Chunk<T> {}

impl<T: Component> Chunk<T> {
    /// Create a new chunk with the given capacity
    #[inline(always)]
    pub fn new(arena: &mut Arena, component_count: usize) -> Option<Self> {
        let component_size = std::mem::size_of::<T>();
        if component_size == 0 {
            return None; // Don't support zero-sized types
        }

        // Calculate alignment for better memory access
        let alignment = std::mem::align_of::<T>().max(MIN_ALIGNMENT);

        // Ensure aligned component size
        let aligned_component_size = align_up(component_size, alignment);

        // Calculate total chunk size with padding
        let chunk_size = align_up(aligned_component_size * component_count, CACHE_LINE_SIZE);

        // Allocate memory from arena
        let memory_ptr = arena.allocate(chunk_size, CACHE_LINE_SIZE)?;

        // Calculate bitmap size to track free/occupied slots
        let bitmap_size = (component_count + 63) / 64;
        let mut free_bitmap = Vec::with_capacity(bitmap_size);

        // Initialize bitmap with all slots free
        for i in 0..bitmap_size {
            let remaining = component_count.saturating_sub(i * 64);
            let bits = if remaining >= 64 {
                !0u64 // All 64 bits set (all free)
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

    /// Create a chunk with the default number of components
    #[inline(always)]
    pub fn with_default_component_count(arena: &mut Arena) -> Option<Self> {
        Self::new(arena, DEFAULT_COMPONENTS_PER_CHUNK)
    }

    /// Allocate a component slot in this chunk
    /// Returns the index of the allocated component or None if chunk is full
    #[inline(always)]
    pub fn allocate_component(&mut self) -> Option<usize> {
        // Quick check if we're at capacity
        if self.active_count >= self.capacity {
            return None;
        }

        // If we have freed slots (count > active_count), find one to reuse
        if self.count > self.active_count {
            // Find first free slot using bitmap
            for (bitmap_idx, bitmap) in self.free_bitmap.iter_mut().enumerate() {
                if *bitmap != 0 {
                    // Found a bitmap with free slots
                    let bit_pos = bitmap.trailing_zeros() as usize;
                    let index = bitmap_idx * 64 + bit_pos;

                    // Ensure index is valid
                    if index >= self.capacity || index >= self.count {
                        continue;
                    }

                    // Mark as occupied (clear bit)
                    *bitmap = clear_bit(*bitmap, bit_pos);

                    // Update active count
                    self.active_count += 1;

                    return Some(index);
                }
            }
        }

        // No free slots found, allocate at the end
        if self.count < self.capacity {
            let index = self.count;

            // Mark as occupied in bitmap
            let bitmap_idx = index / 64;
            let bit_pos = index % 64;

            if bitmap_idx < self.free_bitmap.len() {
                self.free_bitmap[bitmap_idx] = clear_bit(self.free_bitmap[bitmap_idx], bit_pos);
            }

            // Update counts
            self.count += 1;
            self.active_count += 1;

            return Some(index);
        }

        None // Chunk is full
    }

    /// Free a component slot
    #[inline(always)]
    pub fn free_component(&mut self, index: usize) {
        if index >= self.capacity || index >= self.count {
            return; // Invalid index
        }

        // Check bitmap to see if already freed
        let bitmap_idx = index / 64;
        let bit_pos = index % 64;

        if bitmap_idx >= self.free_bitmap.len() {
            return;
        }

        // If bit is already set, component is already freed
        if test_bit(self.free_bitmap[bitmap_idx], bit_pos) {
            return;
        }

        // Mark as free in bitmap
        self.free_bitmap[bitmap_idx] = set_bit(self.free_bitmap[bitmap_idx], bit_pos);

        // Update active count
        if self.active_count > 0 {
            self.active_count -= 1;
        }

        // Zero memory for safety
        unsafe {
            let ptr = self.get_component_ptr(index);
            std::ptr::write(ptr, MaybeUninit::zeroed());
        }
    }

    /// Check if a component slot is occupied
    #[inline(always)]
    pub fn is_occupied(&self, index: usize) -> bool {
        if index >= self.capacity || index >= self.count {
            return false;
        }

        let bitmap_idx = index / 64;
        let bit_pos = index % 64;

        // Bit is 0 if occupied, 1 if free
        bitmap_idx < self.free_bitmap.len() && !test_bit(self.free_bitmap[bitmap_idx], bit_pos)
    }

    /// Get pointer to component
    /// SAFETY: The index must be valid
    #[inline(always)]
    pub unsafe fn get_component_ptr(&self, index: usize) -> *mut MaybeUninit<T> {
        debug_assert!(index < self.capacity,
                      "Component index out of bounds: {} >= {}",
                      index, self.capacity);

        self.ptr.as_ptr().add(index)
    }

    /// Get reference to component
    /// SAFETY: The index must be valid and the component must be initialized
    #[inline(always)]
    pub unsafe fn get_component(&self, index: usize) -> &T {
        debug_assert!(self.is_occupied(index),
                      "Trying to access freed component at index {}", index);

        (*self.get_component_ptr(index)).assume_init_ref()
    }

    /// Get mutable reference to component
    /// SAFETY: The index must be valid and the component must be initialized
    #[inline(always)]
    pub unsafe fn get_component_mut(&self, index: usize) -> &mut T {
        debug_assert!(self.is_occupied(index),
                      "Trying to access freed component at index {}", index);

        (*self.get_component_ptr(index)).assume_init_mut()
    }

    /// Write a component at the specified index
    /// SAFETY: The index must be valid
    #[inline(always)]
    pub unsafe fn write_component(&self, index: usize, value: T) {
        debug_assert!(index < self.capacity,
                      "Component index out of bounds: {} >= {}",
                      index, self.capacity);

        // Write the value directly
        (*self.get_component_ptr(index)).write(value);
    }

    /// Process components in a specific range
    /// Optimized for SIMD operations where appropriate
    #[inline(always)]
    pub unsafe fn process_components_in_range<F>(&self, start: usize, end: usize, mut processor: F)
    where
        F: FnMut(usize, &mut T)
    {
        let real_end = end.min(self.count);

        if start >= real_end {
            return;
        }

        // Determine optimal SIMD processing approach
        let simd_width = calculate_simd_chunk_size(std::mem::size_of::<T>(), self.alignment);

        if simd_width > 1 && self.count == self.active_count {
            // All slots are occupied - use fast SIMD path

            // Align start to SIMD boundary
            let simd_start = align_up(start, simd_width);

            // Align end down to SIMD boundary
            let simd_end = real_end & !(simd_width - 1);

            // Process pre-SIMD elements
            for i in start..simd_start.min(real_end) {
                processor(i, self.get_component_mut(i));
            }

            // Process SIMD-aligned elements
            for i in (simd_start..simd_end).step_by(simd_width) {
                // Process in SIMD-friendly groups
                for j in 0..simd_width {
                    processor(i + j, self.get_component_mut(i + j));
                }
            }

            // Process remaining elements
            for i in simd_end..real_end {
                processor(i, self.get_component_mut(i));
            }
        } else {
            // Mixed occupied/free slots - check bitmap for each
            for i in start..real_end {
                if self.is_occupied(i) {
                    processor(i, self.get_component_mut(i));
                }
            }
        }
    }

    /// Compact the chunk by moving components to fill gaps
    /// This improves cache locality and SIMD performance
    /// Returns the new count of components
    ///
    /// The callback is called for each component that is moved with (old_index, new_index)
    pub fn compact<C: FnMut(usize, usize)>(&mut self, mut on_component_moved: C) -> usize {
        // Skip if no fragmentation
        if self.count == self.active_count || self.count == 0 {
            return self.count;
        }

        // Find first free slot
        let mut write_idx = 0;
        while write_idx < self.count && self.is_occupied(write_idx) {
            write_idx += 1;
        }

        // No free slots found
        if write_idx >= self.count {
            return self.count;
        }

        // Scan for occupied slots after first free slot
        for read_idx in (write_idx + 1)..self.count {
            if self.is_occupied(read_idx) {
                // Move component from read_idx to write_idx
                unsafe {
                    // Copy memory directly
                    std::ptr::copy_nonoverlapping(
                        self.get_component_ptr(read_idx) as *const u8,
                        self.get_component_ptr(write_idx) as *mut u8,
                        std::mem::size_of::<T>()
                    );

                    // Update bitmap - mark read_idx as free, write_idx as occupied
                    let read_bitmap_idx = read_idx / 64;
                    let read_bit_pos = read_idx % 64;
                    let write_bitmap_idx = write_idx / 64;
                    let write_bit_pos = write_idx % 64;

                    self.free_bitmap[read_bitmap_idx] = set_bit(self.free_bitmap[read_bitmap_idx], read_bit_pos);
                    self.free_bitmap[write_bitmap_idx] = clear_bit(self.free_bitmap[write_bitmap_idx], write_bit_pos);

                    // Notify about component movement
                    on_component_moved(read_idx, write_idx);
                }

                // Find next free slot
                write_idx += 1;
                while write_idx < read_idx && self.is_occupied(write_idx) {
                    write_idx += 1;
                }

                if write_idx >= read_idx {
                    break;
                }
            }
        }

        // Find new count (last occupied slot + 1)
        let mut new_count = 0;
        for i in (0..self.count).rev() {
            if self.is_occupied(i) {
                new_count = i + 1;
                break;
            }
        }

        // Update count
        self.count = new_count;

        new_count
    }

    /// Get current component count
    #[inline(always)]
    pub fn component_count(&self) -> usize {
        self.count
    }

    /// Get active component count
    #[inline(always)]
    pub fn active_component_count(&self) -> usize {
        self.active_count
    }

    /// Get maximum capacity
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get chunk size in bytes
    #[inline(always)]
    pub fn size(&self) -> usize {
        self.chunk_size
    }

    /// Check if chunk is full
    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.active_count >= self.capacity
    }

    /// Get fragmentation ratio (0.0 = no fragmentation, 1.0 = fully fragmented)
    #[inline(always)]
    pub fn fragmentation_ratio(&self) -> f32 {
        if self.count == 0 {
            return 0.0;
        }

        1.0 - (self.active_count as f32 / self.count as f32)
    }

    /// Reset chunk, clearing all components
    pub fn reset(&mut self) {
        // Reset counts
        self.count = 0;
        self.active_count = 0;

        // Reset bitmap (all slots free)
        for i in 0..self.free_bitmap.len() {
            let remaining = self.capacity - i * 64;
            self.free_bitmap[i] = if remaining >= 64 {
                !0u64 // All 64 bits set
            } else {
                (1u64 << remaining) - 1 // Just the bits we need
            };
        }

        // Zero memory for safety
        unsafe {
            std::ptr::write_bytes(self.ptr.as_ptr() as *mut u8, 0, self.chunk_size);
        }
    }
}