// crates/boyko_ecs/src/ecs/memory/component_pool.rs

use std::marker::PhantomData;

use super::arena::Arena;
use super::chunk::Chunk;
use crate::ecs::core::component::Component;
use crate::ecs::constants::{DEFAULT_COMPONENTS_PER_CHUNK, DEFAULT_CHUNKS_PER_POOL};

// Compaction threshold - 30% fragmentation
const COMPACTION_THRESHOLD: f32 = 0.3;

/// Component location within the pool
#[derive(Clone, Copy, Debug)]
pub struct ComponentLocation {
    /// Chunk index (0 = primary, 1+ = overflow)
    pub chunk_index: usize,
    /// Component index within the chunk
    pub component_index: usize,
}

/// Memory pool for managing chunks of components
/// Optimized for cache locality and memory contiguity
#[repr(align(64))]
pub struct ComponentPool<T: Component> {
    /// Primary chunk - larger and preferred for allocations
    primary_chunk: Option<Chunk<T>>,

    /// Overflow chunks - used when primary is full
    overflow_chunks: Vec<Chunk<T>>,

    /// Total component count (including freed)
    component_count: usize,

    /// Active component count (excluding freed)
    active_component_count: usize,

    /// Maps global indices to local (chunk, index) pairs
    index_map: Vec<(usize, usize)>, // (chunk_index, component_index)

    /// Components per overflow chunk
    components_per_chunk: usize,

    /// Maximum allowed chunks
    max_chunks: usize,

    /// Flag indicating compaction need
    needs_compaction: bool,

    /// Component type marker
    _marker: PhantomData<T>,
}

unsafe impl<T: Component> Send for ComponentPool<T> {}
unsafe impl<T: Component> Sync for ComponentPool<T> {}

impl<T: Component> ComponentPool<T> {
    /// Create a new component pool with specified parameters
    #[inline(always)]
    pub fn new(
        arena: &mut Arena,
        components_per_chunk: usize,
        chunks_per_pool: usize
    ) -> Self {
        // Primary chunk is twice the size for better cache locality
        let primary_chunk_size = components_per_chunk * 2;
        let primary_chunk = Chunk::<T>::new(arena, primary_chunk_size);

        Self {
            primary_chunk,
            overflow_chunks: Vec::with_capacity(chunks_per_pool),
            component_count: 0,
            active_component_count: 0,
            index_map: Vec::new(),
            components_per_chunk,
            max_chunks: chunks_per_pool,
            needs_compaction: false,
            _marker: PhantomData,
        }
    }

    /// Create a new component pool with default settings
    #[inline(always)]
    pub fn with_default_settings(arena: &mut Arena) -> Self {
        Self::new(
            arena,
            DEFAULT_COMPONENTS_PER_CHUNK,
            DEFAULT_CHUNKS_PER_POOL
        )
    }

    /// Allocate a new component
    #[inline]
    pub fn allocate_component(&mut self, arena: &mut Arena) -> Option<ComponentLocation> {
        // Try primary chunk first for better cache locality
        if let Some(ref mut primary) = self.primary_chunk {
            if let Some(index) = primary.allocate_component() {
                let location = ComponentLocation {
                    chunk_index: 0,
                    component_index: index,
                };

                self.component_count += 1;
                self.active_component_count += 1;
                self.update_index_map(location, self.component_count - 1);

                return Some(location);
            }
        }

        // Try existing overflow chunks
        for (chunk_idx, chunk) in self.overflow_chunks.iter_mut().enumerate() {
            if let Some(component_idx) = chunk.allocate_component() {
                let location = ComponentLocation {
                    chunk_index: chunk_idx + 1, // +1 because 0 is primary
                    component_index: component_idx,
                };

                self.component_count += 1;
                self.active_component_count += 1;
                self.update_index_map(location, self.component_count - 1);

                return Some(location);
            }
        }

        // Create new overflow chunk if allowed
        if self.overflow_chunks.len() < self.max_chunks {
            if let Some(mut new_chunk) = Chunk::<T>::new(arena, self.components_per_chunk) {
                let component_idx = new_chunk.allocate_component().unwrap();
                let chunk_idx = self.overflow_chunks.len();

                self.overflow_chunks.push(new_chunk);
                self.component_count += 1;
                self.active_component_count += 1;

                let location = ComponentLocation {
                    chunk_index: chunk_idx + 1,
                    component_index: component_idx,
                };

                self.update_index_map(location, self.component_count - 1);

                // Mark for compaction with multiple overflow chunks
                if self.overflow_chunks.len() > 1 {
                    self.needs_compaction = true;
                }

                return Some(location);
            }
        }

        self.needs_compaction = true;
        None
    }

    /// Update index mapping
    #[inline]
    fn update_index_map(&mut self, location: ComponentLocation, global_index: usize) {
        if global_index >= self.index_map.len() {
            self.index_map.resize(global_index + 1, (0, 0));
        }

        self.index_map[global_index] = (location.chunk_index, location.component_index);
    }

    /// Free a component at the given location
    #[inline]
    pub fn free_component(&mut self, location: ComponentLocation) {
        if location.chunk_index == 0 {
            if let Some(ref mut primary) = self.primary_chunk {
                primary.free_component(location.component_index);
                if self.active_component_count > 0 {
                    self.active_component_count -= 1;
                }
            }
        } else {
            let overflow_index = location.chunk_index - 1;
            if overflow_index < self.overflow_chunks.len() {
                self.overflow_chunks[overflow_index].free_component(location.component_index);
                if self.active_component_count > 0 {
                    self.active_component_count -= 1;
                }
            }
        }

        // Mark as deleted in index map
        for i in 0..self.index_map.len() {
            let (chunk_idx, comp_idx) = self.index_map[i];
            if chunk_idx == location.chunk_index && comp_idx == location.component_index {
                self.index_map[i] = (usize::MAX, usize::MAX); // Special marker for deleted
                break;
            }
        }

        self.check_compaction_needed();
    }

    /// Check if memory fragmentation warrants compaction
    #[inline]
    fn check_compaction_needed(&mut self) {
        if self.active_component_count < self.component_count {
            let fragmentation = 1.0 - (self.active_component_count as f32 / self.component_count as f32);

            if fragmentation > COMPACTION_THRESHOLD {
                self.needs_compaction = true;
            }
        }
    }

    /// Get component reference by location
    #[inline(always)]
    pub unsafe fn get_component(&self, location: ComponentLocation) -> &T {
        if location.chunk_index == 0 {
            if let Some(ref primary) = self.primary_chunk {
                primary.get_component(location.component_index)
            } else {
                panic!("Primary chunk not initialized");
            }
        } else {
            let overflow_index = location.chunk_index - 1;

            debug_assert!(overflow_index < self.overflow_chunks.len(),
                          "Chunk index out of bounds: {} >= {}",
                          overflow_index, self.overflow_chunks.len());

            self.overflow_chunks[overflow_index].get_component(location.component_index)
        }
    }

    /// Get mutable component reference by location
    #[inline(always)]
    pub unsafe fn get_component_mut(&mut self, location: ComponentLocation) -> &mut T {
        if location.chunk_index == 0 {
            if let Some(ref mut primary) = self.primary_chunk {
                primary.get_component_mut(location.component_index)
            } else {
                panic!("Primary chunk not initialized");
            }
        } else {
            let overflow_index = location.chunk_index - 1;

            debug_assert!(overflow_index < self.overflow_chunks.len(),
                          "Chunk index out of bounds: {} >= {}",
                          overflow_index, self.overflow_chunks.len());

            self.overflow_chunks[overflow_index].get_component_mut(location.component_index)
        }
    }

    /// Get component by global index
    #[inline]
    pub unsafe fn get_component_by_index(&self, global_index: usize) -> Option<&T> {
        if global_index >= self.index_map.len() {
            return None;
        }

        let (chunk_idx, comp_idx) = self.index_map[global_index];

        // Check if component is deleted
        if chunk_idx == usize::MAX {
            return None;
        }

        let location = ComponentLocation {
            chunk_index: chunk_idx,
            component_index: comp_idx,
        };

        Some(self.get_component(location))
    }

    /// Get mutable component by global index
    #[inline]
    pub unsafe fn get_component_by_index_mut(&mut self, global_index: usize) -> Option<&mut T> {
        if global_index >= self.index_map.len() {
            return None;
        }

        let (chunk_idx, comp_idx) = self.index_map[global_index];

        // Check if component is deleted
        if chunk_idx == usize::MAX {
            return None;
        }

        let location = ComponentLocation {
            chunk_index: chunk_idx,
            component_index: comp_idx,
        };

        Some(self.get_component_mut(location))
    }

    /// Set component value at location
    #[inline(always)]
    pub unsafe fn set_component(&mut self, location: ComponentLocation, value: T) {
        *self.get_component_mut(location) = value;
    }

    /// Process components in specified range (for multithreaded processing)
    #[inline]
    pub unsafe fn process_components_in_range<F>(&self, start: usize, end: usize, mut processor: F)
    where
        F: FnMut(usize, &mut T)
    {
        if start >= self.component_count || start >= end {
            return;
        }

        let real_end = end.min(self.component_count);

        // Process only active components within range
        for global_idx in start..real_end {
            if global_idx < self.index_map.len() {
                let (chunk_idx, comp_idx) = self.index_map[global_idx];

                // Skip deleted components
                if chunk_idx == usize::MAX {
                    continue;
                }

                let component = if chunk_idx == 0 {
                    if let Some(ref primary) = self.primary_chunk {
                        if comp_idx < primary.component_count() && primary.is_occupied(comp_idx) {
                            primary.get_component_mut(comp_idx)
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    }
                } else {
                    let overflow_idx = chunk_idx - 1;
                    if overflow_idx < self.overflow_chunks.len() {
                        let chunk = &self.overflow_chunks[overflow_idx];
                        if comp_idx < chunk.component_count() && chunk.is_occupied(comp_idx) {
                            chunk.get_component_mut(comp_idx)
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    }
                };

                processor(global_idx, component);
            }
        }
    }

    /// Get total component count (including freed)
    #[inline(always)]
    pub fn component_count(&self) -> usize {
        self.component_count
    }

    /// Get active component count (excluding freed)
    #[inline(always)]
    pub fn active_component_count(&self) -> usize {
        self.active_component_count
    }

    /// Get total capacity
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        let primary_capacity = self.primary_chunk.as_ref().map_or(0, |c| c.capacity());
        let overflow_capacity = self.overflow_chunks.iter().map(|c| c.capacity()).sum::<usize>();
        primary_capacity + overflow_capacity
    }

    /// Reset pool, clearing all components
    pub fn reset(&mut self) {
        if let Some(ref mut primary) = self.primary_chunk {
            primary.reset();
        }

        for chunk in &mut self.overflow_chunks {
            chunk.reset();
        }

        self.component_count = 0;
        self.active_component_count = 0;
        self.index_map.clear();
        self.needs_compaction = false;
    }

    /// Calculate entity range for a thread (deterministic, no atomics)
    #[inline(always)]
    pub fn get_thread_entity_range(&self, thread_id: usize, thread_count: usize) -> (usize, usize) {
        if thread_count == 0 || self.active_component_count == 0 {
            return (0, 0);
        }

        let base_items_per_thread = self.active_component_count / thread_count;
        let remainder = self.active_component_count % thread_count;

        // First 'remainder' threads get one extra item
        let start = if thread_id < remainder {
            thread_id * (base_items_per_thread + 1)
        } else {
            (remainder * (base_items_per_thread + 1)) +
                ((thread_id - remainder) * base_items_per_thread)
        };

        let items_for_this_thread = if thread_id < remainder {
            base_items_per_thread + 1
        } else {
            base_items_per_thread
        };

        let end = start + items_for_this_thread;

        (start, end)
    }

    /// Check if compaction is needed
    #[inline(always)]
    pub fn needs_compaction(&self) -> bool {
        self.needs_compaction
    }

    /// Compact the pool by moving components to a new contiguous primary chunk
    /// Returns relocation map for external indices
    pub fn compact(&mut self, arena: &mut Arena) -> Vec<(usize, usize)> {
        if !self.needs_compaction || self.active_component_count == 0 {
            self.needs_compaction = false;
            return Vec::new();
        }

        let mut relocation_map = Vec::new();

        // Calculate new size with 50% extra capacity
        let new_capacity = (self.active_component_count as f32 * 1.5) as usize;

        if let Some(mut new_primary) = Chunk::<T>::new(arena, new_capacity) {
            // Collect active components and their indices
            let mut active_component_locations = Vec::with_capacity(self.active_component_count);
            let mut active_component_indices = Vec::with_capacity(self.active_component_count);

            for (global_idx, (chunk_idx, comp_idx)) in self.index_map.iter().enumerate() {
                if *chunk_idx != usize::MAX {
                    active_component_locations.push(ComponentLocation {
                        chunk_index: *chunk_idx,
                        component_index: *comp_idx,
                    });
                    active_component_indices.push(global_idx);
                }
            }

            // Move components to new primary chunk
            for (i, location) in active_component_locations.iter().enumerate() {
                if let Some(new_idx) = new_primary.allocate_component() {
                    unsafe {
                        // Get source component pointer
                        let src_ptr = if location.chunk_index == 0 {
                            if let Some(ref primary) = self.primary_chunk {
                                primary.get_component_ptr(location.component_index)
                            } else {
                                continue;
                            }
                        } else {
                            let overflow_idx = location.chunk_index - 1;
                            if overflow_idx < self.overflow_chunks.len() {
                                self.overflow_chunks[overflow_idx].get_component_ptr(location.component_index)
                            } else {
                                continue;
                            }
                        };

                        // Get destination pointer
                        let dst_ptr = new_primary.get_component_ptr(new_idx);

                        // Direct memory copy
                        std::ptr::copy_nonoverlapping(
                            src_ptr as *const u8,
                            dst_ptr as *mut u8,
                            std::mem::size_of::<T>()
                        );

                        // Add to relocation map
                        let global_idx = active_component_indices[i];
                        relocation_map.push((global_idx, i));

                        // Update index map
                        self.index_map[global_idx] = (0, new_idx);
                    }
                }
            }

            // Replace primary chunk and clear overflow
            self.primary_chunk = Some(new_primary);
            self.overflow_chunks.clear();
            self.component_count = self.active_component_count;
        }

        self.needs_compaction = false;
        relocation_map
    }

    /// Get fragmentation ratio (0.0 = none, 1.0 = fully fragmented)
    #[inline(always)]
    pub fn fragmentation_ratio(&self) -> f32 {
        if self.component_count == 0 {
            return 0.0;
        }

        1.0 - (self.active_component_count as f32 / self.component_count as f32)
    }
}