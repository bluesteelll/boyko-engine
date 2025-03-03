use std::alloc::{alloc, dealloc, Layout};
use std::ptr::NonNull;

use crate::ecs::constants::DEFAULT_ARENA_SIZE;
use super::utils::{align_up, is_power_of_two};

/// Maximum number of free blocks to track in the freelist
const MAX_FREE_BLOCKS: usize = 32;

/// Minimum size of a free block to be tracked
const MIN_FREE_BLOCK_SIZE: usize = 64;

/// Memory block metadata for the freelist
#[derive(Clone, Copy, Debug)]
struct FreeBlock {
    /// Offset from the start of the arena
    offset: usize,
    /// Size of the block in bytes
    size: usize,
}

/// Memory arena for component storage
/// Uses a bump allocator with a simple freelist for reuse
#[repr(align(64))] // Align to cache line
pub struct Arena {
    /// Pointer to the arena memory
    ptr: NonNull<u8>,

    /// Total capacity in bytes
    capacity: usize,

    /// Current allocation position
    cursor: usize,

    /// Memory layout for deallocation
    layout: Layout,

    /// Free blocks list for memory reuse
    /// Stores (offset, size) pairs in order of size (largest first)
    free_blocks: Vec<FreeBlock>,
}

unsafe impl Send for Arena {}
unsafe impl Sync for Arena {}

impl Arena {
    /// Create a new arena with the specified capacity in bytes
    #[inline(always)]
    pub fn new(capacity: usize) -> Self {
        let aligned_capacity = align_up(capacity, 64);
        let layout = Layout::from_size_align(aligned_capacity, 64)
            .expect("Invalid layout for arena");

        let ptr = unsafe {
            let ptr = alloc(layout);
            NonNull::new(ptr).expect("Memory allocation failed for arena")
        };

        // Zero-initialize memory for safety
        unsafe {
            std::ptr::write_bytes(ptr.as_ptr(), 0, aligned_capacity);
        }

        Self {
            ptr,
            capacity: aligned_capacity,
            cursor: 0,
            layout,
            free_blocks: Vec::with_capacity(MAX_FREE_BLOCKS),
        }
    }

    /// Create a new arena with the default size
    #[inline(always)]
    pub fn with_default_size() -> Self {
        Self::new(DEFAULT_ARENA_SIZE)
    }

    /// Allocate a memory region from the arena
    /// Returns a pointer to the allocated memory or None if out of memory
    #[inline(always)]
    pub fn allocate(&mut self, size: usize, align: usize) -> Option<NonNull<u8>> {
        if size == 0 {
            return Some(self.ptr);
        }

        debug_assert!(is_power_of_two(align), "Alignment must be a power of 2");

        // First try to reuse a free block if available
        if !self.free_blocks.is_empty() {
            return self.allocate_from_free_blocks(size, align);
        }

        // No suitable free block, use bump allocation
        // Calculate aligned address
        let align_mask = align - 1;
        let aligned_addr = (self.cursor + align_mask) & !align_mask;
        let next = aligned_addr + size;

        // Check if we have enough space
        if next > self.capacity {
            return None;
        }

        // Update cursor
        self.cursor = next;

        // Return pointer to allocated memory
        let ptr = unsafe { self.ptr.as_ptr().add(aligned_addr) };
        Some(unsafe { NonNull::new_unchecked(ptr) })
    }

    /// Try to allocate from free blocks
    /// Returns a pointer to the allocated memory or None if no suitable block found
    #[inline]
    fn allocate_from_free_blocks(&mut self, size: usize, align: usize) -> Option<NonNull<u8>> {
        let align_mask = align - 1;

        // Find best-fit block
        let mut best_fit_idx = None;
        let mut best_fit_size = usize::MAX;
        let mut best_aligned_addr = 0;

        for (i, block) in self.free_blocks.iter().enumerate() {
            // Calculate aligned address within the block
            let block_start = block.offset;
            let aligned_addr = (block_start + align_mask) & !align_mask;
            let alignment_waste = aligned_addr - block_start;

            // Check if block is large enough
            let total_size = size + alignment_waste;
            if block.size >= total_size && block.size < best_fit_size {
                best_fit_idx = Some(i);
                best_fit_size = block.size;
                best_aligned_addr = aligned_addr;

                // If perfect fit, break early
                if block.size <= total_size + MIN_FREE_BLOCK_SIZE { // Allow small waste
                    break;
                }
            }
        }

        // If found a suitable block
        if let Some(idx) = best_fit_idx {
            let block = self.free_blocks[idx];

            // Calculate aligned address
            let block_start = block.offset;
            let aligned_addr = best_aligned_addr;
            let alignment_waste = aligned_addr - block_start;
            let total_size = size + alignment_waste;

            // Check if the remaining space is worth tracking
            if block.size - total_size >= MIN_FREE_BLOCK_SIZE {
                // Update the block with remaining space
                self.free_blocks[idx].offset = block_start + total_size;
                self.free_blocks[idx].size = block.size - total_size;

                // If there's wasted space at the beginning due to alignment, and it's large enough,
                // add it as a new free block
                if alignment_waste >= MIN_FREE_BLOCK_SIZE {
                    if self.free_blocks.len() < MAX_FREE_BLOCKS {
                        self.free_blocks.push(FreeBlock {
                            offset: block_start,
                            size: alignment_waste,
                        });
                    }
                }
            } else {
                // Remove the block as we're using all of it
                self.free_blocks.swap_remove(idx);
            }

            // Return pointer to the aligned address
            let ptr = unsafe { self.ptr.as_ptr().add(aligned_addr) };
            return Some(unsafe { NonNull::new_unchecked(ptr) });
        }

        None
    }

    /// Free a memory region
    /// This adds the region to the free list for future reuse
    #[inline]
    pub unsafe fn free(&mut self, ptr: NonNull<u8>, size: usize) {
        if size == 0 {
            return;
        }

        // Calculate offset from arena start
        let offset = ptr.as_ptr().offset_from(self.ptr.as_ptr()) as usize;

        // Check if this block is at the end of the arena (can move cursor back)
        if offset + size == self.cursor {
            // Just move cursor back
            self.cursor = offset;
            return;
        }

        // Don't track small blocks
        if size < MIN_FREE_BLOCK_SIZE {
            return;
        }

        // Check if we can merge with adjacent blocks
        let mut merged = false;
        let mut i = 0;

        while i < self.free_blocks.len() {
            let block = self.free_blocks[i];

            // Check if this block is adjacent to our freed region
            if block.offset + block.size == offset {
                // Extend block to include our region
                self.free_blocks[i].size += size;
                merged = true;
                break;
            } else if offset + size == block.offset {
                // Extend region to include this block
                self.free_blocks[i].offset = offset;
                self.free_blocks[i].size += size;
                merged = true;
                break;
            }

            i += 1;
        }

        // If we didn't merge, add as a new block
        if !merged {
            // Check if we have space in the freelist
            if self.free_blocks.len() >= MAX_FREE_BLOCKS {
                // Find the smallest block to replace
                let mut smallest_idx = 0;
                let mut smallest_size = self.free_blocks[0].size;

                for (i, block) in self.free_blocks.iter().enumerate().skip(1) {
                    if block.size < smallest_size {
                        smallest_idx = i;
                        smallest_size = block.size;
                    }
                }

                // Replace if new block is larger
                if size > smallest_size {
                    self.free_blocks[smallest_idx] = FreeBlock { offset, size };
                }
            } else {
                // Add to freelist
                self.free_blocks.push(FreeBlock { offset, size });
            }
        }

        // Check if we can also merge other blocks
        self.merge_adjacent_blocks();

        // If the free block is at the end of the arena, we can move the cursor back
        self.try_reclaim_tail();
    }

    /// Merge adjacent blocks in the freelist
    fn merge_adjacent_blocks(&mut self) {
        if self.free_blocks.len() <= 1 {
            return;
        }

        // Sort by offset for easier merging
        self.free_blocks.sort_by_key(|block| block.offset);

        let mut i = 0;
        while i < self.free_blocks.len() - 1 {
            let (curr_offset, curr_size) = (self.free_blocks[i].offset, self.free_blocks[i].size);
            let next_offset = self.free_blocks[i + 1].offset;

            if curr_offset + curr_size == next_offset {
                // Merge current block with next block
                self.free_blocks[i].size += self.free_blocks[i + 1].size;
                self.free_blocks.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }

    /// Try to reclaim memory at the end of the arena
    fn try_reclaim_tail(&mut self) {
        if self.free_blocks.is_empty() {
            return;
        }

        // Find the highest offset block
        let mut highest_idx = 0;
        let mut highest_offset = self.free_blocks[0].offset;

        for (i, block) in self.free_blocks.iter().enumerate().skip(1) {
            if block.offset > highest_offset {
                highest_idx = i;
                highest_offset = block.offset;
            }
        }

        // Check if it's at the end of the allocated area
        let block = self.free_blocks[highest_idx];
        if block.offset + block.size == self.cursor {
            // Move cursor back
            self.cursor = block.offset;
            // Remove the block from freelist
            self.free_blocks.swap_remove(highest_idx);

            // Recursively check for more blocks at the tail
            self.try_reclaim_tail();
        }
    }

    /// Get available space in bytes
    #[inline(always)]
    pub fn available_space(&self) -> usize {
        // Calculate free space from cursors
        let bump_free = self.capacity - self.cursor;

        // Add space from freelist
        let freelist_free = self.free_blocks.iter().map(|block| block.size).sum::<usize>();

        bump_free + freelist_free
    }

    /// Get pointer to arena memory
    #[inline(always)]
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// Get total capacity in bytes
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get used space in bytes
    #[inline(always)]
    pub fn used_space(&self) -> usize {
        self.capacity - self.available_space()
    }

    /// Reset the arena, clearing all allocations
    pub fn reset(&mut self) {
        self.cursor = 0;
        self.free_blocks.clear();

        // Zero memory for safety
        unsafe {
            std::ptr::write_bytes(self.ptr.as_ptr(), 0, self.capacity);
        }
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}