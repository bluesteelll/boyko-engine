use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::marker::PhantomData;

use super::arena::Arena;
use super::utils::align_up;
use crate::ecs::core::component::Component;

pub const DEFAULT_COMPONENTS_PER_CHUNK: usize = 1024;   // Number of components per chunk
pub const MIN_ALIGNMENT: usize = 8;                     // Minimum 8-byte alignment
pub const CACHE_LINE_SIZE: usize = 64;                  // Typical cache line size

/// A chunk of memory for storing components of a single type
/// Serves as the basic allocation unit for component data
#[repr(align(64))] // Align to cache line to prevent false sharing
pub struct Chunk<T: Component> {
    ptr: NonNull<u8>,
    capacity: usize,
    count: AtomicUsize,
    chunk_size: usize,
    alignment: usize,
    component_size: usize,
    _marker: PhantomData<T>,
}

unsafe impl<T: Component> Send for Chunk<T> {}
unsafe impl<T: Component> Sync for Chunk<T> {}

impl<T: Component> Chunk<T> {
    #[inline(always)]
    pub fn new(arena: &Arena, component_count: usize) -> Option<Self> {
        let component_size = std::mem::size_of::<T>();
        // Align small components to at least 8 bytes for better memory access patterns
        let alignment = std::mem::align_of::<T>().max(MIN_ALIGNMENT);
        let aligned_component_size = align_up(component_size, alignment);

        // Calculate chunk size and align to cache line
        let chunk_size = align_up(aligned_component_size * component_count, CACHE_LINE_SIZE);

        // Allocate memory for the chunk from the arena
        let ptr = arena.allocate(chunk_size, CACHE_LINE_SIZE)?;

        // Zero-initialize memory for safety
        unsafe {
            std::ptr::write_bytes(ptr.as_ptr(), 0, chunk_size);
        }

        Some(Self {
            ptr,
            capacity: component_count,
            count: AtomicUsize::new(0),
            chunk_size,
            alignment,
            component_size: aligned_component_size,
            _marker: PhantomData,
        })
    }

    #[inline(always)]
    pub fn with_default_component_count(arena: &Arena) -> Option<Self> {
        Self::new(arena, DEFAULT_COMPONENTS_PER_CHUNK)
    }

    /// Allocates a component slot in the chunk
    /// Returns the index of the allocated component
    #[inline(always)]
    pub fn allocate_component(&self) -> Option<usize> {
        let mut current_count = self.count.load(Ordering::Relaxed);
        loop {
            // Check if the chunk is full
            if current_count >= self.capacity {
                return None;
            }

            // Atomically update the count
            match self.count.compare_exchange_weak(
                current_count,
                current_count + 1,
                Ordering::AcqRel,
                Ordering::Relaxed
            ) {
                Ok(_) => {
                    // Successfully allocated a component slot
                    return Some(current_count);
                }
                Err(actual) => {
                    // Someone else allocated, try again with the new value
                    current_count = actual;
                }
            }
        }
    }

    #[inline(always)]
    pub unsafe fn get_component_ptr(&self, index: usize) -> *mut T {
        debug_assert!(index < self.count.load(Ordering::Relaxed),
                      "Component index out of bounds: {} >= {}",
                      index, self.count.load(Ordering::Relaxed));

        // Calculate pointer with alignment
        let offset = index * self.component_size;
        let ptr = self.ptr.as_ptr().add(offset);

        ptr as *mut T
    }

    #[inline(always)]
    pub unsafe fn get_component(&self, index: usize) -> &T {
        &*self.get_component_ptr(index)
    }

    #[inline(always)]
    pub unsafe fn get_component_mut(&self, index: usize) -> &mut T {
        &mut *self.get_component_ptr(index)
    }

    /// Process components in a specific range (for thread partitioning)
    /// Each thread will process a contiguous range of entities
    #[inline(always)]
    pub unsafe fn process_components_in_range<F>(&self, start: usize, end: usize, mut processor: F)
    where
        F: FnMut(usize, &mut T)
    {
        let count = self.count.load(Ordering::Acquire);
        let real_end = end.min(count);

        if start >= real_end {
            return;
        }

        // Process each component in the range
        for i in start..real_end {
            let component = self.get_component_mut(i);
            processor(i, component);
        }
    }

    #[inline(always)]
    pub fn component_count(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[inline(always)]
    pub fn size(&self) -> usize {
        self.chunk_size
    }

    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.component_count() >= self.capacity
    }

    #[inline(always)]
    pub fn reset(&self) {
        self.count.store(0, Ordering::Release);

        // Zero memory for safety when reusing
        unsafe {
            std::ptr::write_bytes(self.ptr.as_ptr(), 0, self.chunk_size);
        }
    }
}