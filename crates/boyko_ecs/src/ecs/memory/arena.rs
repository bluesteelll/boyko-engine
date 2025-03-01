use std::alloc::{alloc, dealloc, Layout};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ecs::constants::{DEFAULT_ARENA_SIZE, CACHE_LINE_SIZE};
use super::utils::fast_align;

/// Memory arena for an archetype
/// Allocates a single large block of memory that's divided into chunks
#[repr(align(64))] // Align to cache line to prevent false sharing
pub struct Arena {
    ptr: NonNull<u8>,
    capacity: usize,
    cursor: AtomicUsize,
    layout: Layout,
}

unsafe impl Send for Arena {}
unsafe impl Sync for Arena {}

impl Arena {
    #[inline(always)]
    pub fn new(capacity: usize) -> Self {
        let aligned_capacity = fast_align(capacity, CACHE_LINE_SIZE);
        let layout = Layout::from_size_align(aligned_capacity, CACHE_LINE_SIZE)
            .expect("Invalid layout for arena");

        let ptr = unsafe {
            let ptr = alloc(layout);
            NonNull::new(ptr).expect("Memory allocation failed for arena")
        };

        Self {
            ptr,
            capacity: aligned_capacity,
            cursor: AtomicUsize::new(0),
            layout,
        }
    }

    #[inline(always)]
    pub fn with_default_size() -> Self {
        Self::new(DEFAULT_ARENA_SIZE)
    }

    /// Allocates a memory region of the specified size from the arena
    /// Returns a pointer to the allocated memory
    #[inline(always)]
    pub fn allocate(&self, size: usize, align: usize) -> Option<NonNull<u8>> {
        if size == 0 {
            return Some(self.ptr);
        }

        let align = align.max(CACHE_LINE_SIZE);

        let align_mask = align - 1;

        let mut current = self.cursor.load(Ordering::Relaxed);

        loop {
            let aligned_addr = (current + align_mask) & !align_mask;
            let next = aligned_addr + size;

            if next > self.capacity {
                return None;
            }

            match self.cursor.compare_exchange_weak(
                current,
                next,
                Ordering::Release,
                Ordering::Relaxed
            ) {
                Ok(_) => {
                    let ptr = unsafe { self.ptr.as_ptr().add(aligned_addr) };
                    return Some(unsafe { NonNull::new_unchecked(ptr) });
                }
                Err(actual) => {
                    current = actual;
                    core::hint::spin_loop();
                }
            }
        }
    }

    #[inline(always)]
    pub fn available_space(&self) -> usize {
        self.capacity - self.cursor.load(Ordering::Relaxed)
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[inline(always)]
    pub fn used_space(&self) -> usize {
        self.cursor.load(Ordering::Relaxed)
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}