// crates/boyko_ecs/src/ecs/memory/component_pool.rs

use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::arena::Arena;
use super::chunk::{Chunk, DEFAULT_COMPONENTS_PER_CHUNK};
use crate::ecs::core::component::Component;

pub const DEFAULT_CHUNKS_PER_POOL: usize = 32;  // Fixed number of chunks per pool

/// Memory pool that manages chunks for a component type
/// Each archetype has a pool for each component type
#[repr(align(64))]
pub struct ComponentPool<T: Component> {
    arena: *const Arena,
    chunks: Vec<Chunk<T>>,
    components_per_chunk: usize,
    max_chunks: usize,
}

unsafe impl<T: Component> Send for ComponentPool<T> {}
unsafe impl<T: Component> Sync for ComponentPool<T> {}

impl<T: Component> ComponentPool<T> {

    #[inline(always)]
    pub fn new(
        arena: &Arena,
        components_per_chunk: usize,
        chunks_per_pool: usize
    ) -> Self {
        Self {
            arena,
            chunks: Vec::with_capacity(chunks_per_pool),
            components_per_chunk,
            max_chunks: chunks_per_pool,
        }
    }

    #[inline(always)]
    pub fn with_default_settings(arena: &Arena) -> Self {
        Self::new(
            arena,
            DEFAULT_COMPONENTS_PER_CHUNK,
            DEFAULT_CHUNKS_PER_POOL
        )
    }

    #[inline(always)]
    pub fn allocate_component(&mut self) -> Option<(usize, usize)> {
        // Try to allocate from existing chunks
        for (chunk_idx, chunk) in self.chunks.iter().enumerate() {
            if let Some(component_idx) = chunk.allocate_component() {
                return Some((chunk_idx, component_idx));
            }
        }

        // All chunks are full or no chunks exist, create a new one if we haven't reached max_chunks
        if self.chunks.len() < self.max_chunks {
            let new_chunk = Chunk::<T>::new(
                unsafe { &*self.arena },
                self.components_per_chunk
            )?;

            // Allocate from the new chunk
            let component_idx = new_chunk.allocate_component().unwrap(); // This should always succeed
            let chunk_idx = self.chunks.len();
            self.chunks.push(new_chunk);

            return Some((chunk_idx, component_idx));
        }

        None // Pool is full
    }

    #[inline(always)]
    pub unsafe fn get_component(&self, chunk_idx: usize, component_idx: usize) -> &T {
        debug_assert!(chunk_idx < self.chunks.len(),
                      "Chunk index out of bounds: {} >= {}",
                      chunk_idx, self.chunks.len());

        self.chunks[chunk_idx].get_component(component_idx)
    }

    #[inline(always)]
    pub unsafe fn get_component_mut(&self, chunk_idx: usize, component_idx: usize) -> &mut T {
        debug_assert!(chunk_idx < self.chunks.len(),
                      "Chunk index out of bounds: {} >= {}",
                      chunk_idx, self.chunks.len());

        self.chunks[chunk_idx].get_component_mut(component_idx)
    }

    #[inline(always)]
    pub unsafe fn set_component(&self, chunk_idx: usize, component_idx: usize, value: T) {
        debug_assert!(chunk_idx < self.chunks.len(),
                      "Chunk index out of bounds: {} >= {}",
                      chunk_idx, self.chunks.len());

        *self.chunks[chunk_idx].get_component_mut(component_idx) = value;
    }

    #[inline(always)]
    pub unsafe fn process_components_in_range<F>(&self, start: usize, end: usize, mut processor: F)
    where
        F: FnMut(usize, &mut T)
    {
        let total_component_count = self.component_count();
        if start >= total_component_count {
            return;
        }

        let real_end = end.min(total_component_count);

        // Process components across chunks
        let mut current_index = 0;
        for (chunk_idx, chunk) in self.chunks.iter().enumerate() {
            let chunk_count = chunk.component_count();
            let chunk_end = current_index + chunk_count;

            // Check if this chunk has components in our range
            if current_index < real_end && chunk_end > start {
                // Calculate overlap between chunk and target range
                let range_start = start.saturating_sub(current_index);
                let range_end = (real_end - current_index).min(chunk_count);

                // Process components in this chunk's range
                if range_start < range_end {
                    chunk.process_components_in_range(range_start, range_end, |local_idx, component| {
                        let global_idx = current_index + local_idx;
                        processor(global_idx, component);
                    });
                }
            }

            current_index = chunk_end;
            if current_index >= real_end {
                break;
            }
        }
    }

    #[inline(always)]
    pub fn component_count(&self) -> usize {
        self.chunks.iter().map(|chunk| chunk.component_count()).sum()
    }

    #[inline(always)]
    pub fn reset(&mut self) {
        for chunk in &self.chunks {
            chunk.reset();
        }
    }

    #[inline(always)]
    pub fn get_thread_entity_range(&self, thread_id: usize, thread_count: usize) -> (usize, usize) {
        let entity_count = self.component_count();
        let entities_per_thread = (entity_count + thread_count - 1) / thread_count;

        let start = thread_id * entities_per_thread;
        let end = (start + entities_per_thread).min(entity_count);

        (start, end)
    }
}


#[derive(Clone, Copy, Debug)]
pub struct ComponentLocation {
    pub pool_index: usize,
    pub chunk_index: usize,
    pub component_index: usize,
}