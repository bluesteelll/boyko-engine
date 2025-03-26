use std::alloc::{alloc, Layout};
use std::cell::UnsafeCell;
use std::ptr::NonNull;
use crate::ecs::constants::{CACHE_LINE_SIZE, DEFAULT_ARENA_SIZE};
use crate::ecs::memory::free_mem_block::MemFreeBlockMaster;
use crate::ecs::memory::utils::align_up;

pub struct Arena {
    ptr: NonNull<u8>,

    capacity: usize,

    cursor: UnsafeCell<usize>,

    layout: Layout,

    free_blocks: UnsafeCell<MemFreeBlockMaster>

}

impl Arena {
    pub fn with_capacity(capacity: usize) -> Self {
        let aligned_capacity = align_up(capacity, CACHE_LINE_SIZE);

        let layout = Layout::from_size_align(aligned_capacity, CACHE_LINE_SIZE)
            .expect("Invalid layout for arena");

        let ptr = unsafe { alloc(layout) };
        let ptr = NonNull::new(ptr).expect("Failed to allocate memory for arena");

        Self {
            ptr,
            capacity: aligned_capacity,
            cursor: UnsafeCell::new(0),
            layout,
            free_blocks: UnsafeCell::new(MemFreeBlockMaster::new_init(capacity)),
        }
    }

    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_ARENA_SIZE)
    }

    pub fn allocate_layout(&self, layout: Layout) -> NonNull<u8> {
        match self.allocate_from_free_blocks(layout) {
            Some(ptr) => ptr,
            None => panic!("Arena out of memory: no suitable free blocks available")
        }
    }

    pub fn allocate_from_free_blocks(&self, layout: Layout) -> Option<NonNull<u8>> {
        let size = layout.size();
        let align = layout.align();

        let free_blocks = unsafe { &mut *self.free_blocks.get() };

        let block = free_blocks.allocate_aligned(size, align)?;

        let ptr = unsafe {
            self.ptr.as_ptr().add(block.start)
        };

        NonNull::new(ptr)
    }

    pub fn allocate<T: Sized>(&self) -> NonNull<T> {
        let layout = Layout::new::<T>();
        let ptr = self.allocate_layout(layout);
        unsafe {
            ptr.cast()
        }
    }
}