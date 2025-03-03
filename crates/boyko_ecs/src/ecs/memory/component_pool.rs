use std::marker::PhantomData;

use super::arena::Arena;
use super::chunk::Chunk;
use super::utils::calculate_thread_entity_range;
use crate::ecs::core::component::Component;
use crate::ecs::constants::{DEFAULT_COMPONENTS_PER_CHUNK, DEFAULT_CHUNKS_PER_POOL};

// Compaction threshold - 30% fragmentation
const COMPACTION_THRESHOLD: f32 = 0.3;
const MIN_COMPONENTS_FOR_COMPACTION: usize = 16;

/// Component location within the pool
#[derive(Clone, Copy, Debug)]
pub struct ComponentLocation {
    /// Chunk index (0 = primary, 1+ = overflow)
    pub chunk_index: usize,
    /// Component index within the chunk
    pub component_index: usize,
}

/// Memory pool for managing chunks of components
/// Optimized for cache locality and memory reuse
#[repr(align(64))]
pub struct ComponentPool<T: Component> {
    /// Primary chunk - larger and preferred for allocations
    primary_chunk: Option<Chunk<T>>,

    /// Overflow chunks - used when primary is full
    overflow_chunks: Vec<Chunk<T>>,

    /// Total active component count
    active_component_count: usize,

    /// Maps global indices to local (chunk, index) pairs
    /// If a component is removed, its entry becomes None
    index_map: Vec<Option<ComponentLocation>>,

    /// Free indices that can be reused
    free_indices: Vec<usize>,

    /// Components per overflow chunk
    components_per_chunk: usize,

    /// Maximum allowed chunks
    max_chunks: usize,

    /// Flag to indicate if we're at maximum capacity
    at_max_capacity: bool,

    /// Type marker
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
    ) -> Result<Self, &'static str> {
        // Primary chunk is twice the size for better cache locality
        let primary_chunk_size = components_per_chunk * 2;
        let primary_chunk = Chunk::<T>::new(arena, primary_chunk_size)
            .ok_or("Failed to allocate primary chunk")?;

        Ok(Self {
            primary_chunk: Some(primary_chunk),
            overflow_chunks: Vec::with_capacity(chunks_per_pool),
            active_component_count: 0,
            index_map: Vec::new(),
            free_indices: Vec::new(),
            components_per_chunk,
            max_chunks: chunks_per_pool,
            at_max_capacity: false,
            _marker: PhantomData,
        })
    }

    /// Create a component pool with default settings
    #[inline(always)]
    pub fn with_default_settings(arena: &mut Arena) -> Result<Self, &'static str> {
        Self::new(
            arena,
            DEFAULT_COMPONENTS_PER_CHUNK,
            DEFAULT_CHUNKS_PER_POOL
        )
    }

    /// Allocate a new component with the given value
    /// Returns global index and component location
    #[inline]
    pub fn allocate(&mut self, arena: &mut Arena, value: T) -> Option<(usize, ComponentLocation)> {
        // First try to reuse a freed index
        let global_idx = if let Some(idx) = self.free_indices.pop() {
            idx
        } else {
            self.index_map.len()
        };

        // Try primary chunk first
        if let Some(ref mut primary) = self.primary_chunk {
            if let Some(comp_idx) = primary.allocate_component() {
                // Initialize component
                unsafe {
                    primary.write_component(comp_idx, value);
                }

                let location = ComponentLocation {
                    chunk_index: 0,
                    component_index: comp_idx,
                };

                // Update index map
                if global_idx < self.index_map.len() {
                    self.index_map[global_idx] = Some(location);
                } else {
                    self.index_map.push(Some(location));
                }

                self.active_component_count += 1;
                return Some((global_idx, location));
            }
        }

        // Try existing overflow chunks
        for (chunk_idx, chunk) in self.overflow_chunks.iter_mut().enumerate() {
            if let Some(comp_idx) = chunk.allocate_component() {
                // Initialize component
                unsafe {
                    chunk.write_component(comp_idx, value);
                }

                let location = ComponentLocation {
                    chunk_index: chunk_idx + 1, // +1 since 0 is primary
                    component_index: comp_idx,
                };

                // Update index map
                if global_idx < self.index_map.len() {
                    self.index_map[global_idx] = Some(location);
                } else {
                    self.index_map.push(Some(location));
                }

                self.active_component_count += 1;
                return Some((global_idx, location));
            }
        }

        // Create new overflow chunk if allowed
        if self.overflow_chunks.len() < self.max_chunks {
            if let Some(mut new_chunk) = Chunk::<T>::new(arena, self.components_per_chunk) {
                if let Some(comp_idx) = new_chunk.allocate_component() {
                    // Initialize component
                    unsafe {
                        new_chunk.write_component(comp_idx, value);
                    }

                    let chunk_idx = self.overflow_chunks.len();
                    let location = ComponentLocation {
                        chunk_index: chunk_idx + 1, // +1 since 0 is primary
                        component_index: comp_idx,
                    };

                    self.overflow_chunks.push(new_chunk);

                    // Update index map
                    if global_idx < self.index_map.len() {
                        self.index_map[global_idx] = Some(location);
                    } else {
                        self.index_map.push(Some(location));
                    }

                    self.active_component_count += 1;
                    return Some((global_idx, location));
                }
            }
        }

        // If allocation failed and we need compaction, try that
        if self.should_compact() {
            self.compact(arena);
            // Try allocation again
            let result = self.allocate(arena, value);
            if result.is_none() {
                // We've reached our maximum capacity
                self.at_max_capacity = true;
            }
            return result;
        }

        // We're at maximum capacity now
        self.at_max_capacity = true;
        None
    }

    /// Remove a component by global index
    #[inline]
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.index_map.len() {
            return false;
        }

        // Get location
        let location = match self.index_map[index] {
            Some(loc) => loc,
            None => return false, // Already removed
        };

        // Free the component
        let result = if location.chunk_index == 0 {
            if let Some(ref mut primary) = self.primary_chunk {
                primary.free_component(location.component_index);
                true
            } else {
                false
            }
        } else {
            let overflow_idx = location.chunk_index - 1;
            if overflow_idx < self.overflow_chunks.len() {
                self.overflow_chunks[overflow_idx].free_component(location.component_index);
                true
            } else {
                false
            }
        };

        if result {
            // Mark index as free
            self.index_map[index] = None;
            self.free_indices.push(index);

            if self.active_component_count > 0 {
                self.active_component_count -= 1;
            }
        }

        result
    }

    /// Get component reference by global index
    #[inline]
    pub unsafe fn get_component_by_index(&self, index: usize) -> Option<&T> {
        if index >= self.index_map.len() {
            return None;
        }

        // Get location
        let location = match self.index_map[index] {
            Some(loc) => loc,
            None => return None, // Already removed
        };

        // Get the component
        if location.chunk_index == 0 {
            if let Some(ref primary) = self.primary_chunk {
                if primary.is_occupied(location.component_index) {
                    return Some(primary.get_component(location.component_index));
                }
            }
        } else {
            let overflow_idx = location.chunk_index - 1;
            if overflow_idx < self.overflow_chunks.len() {
                let chunk = &self.overflow_chunks[overflow_idx];
                if chunk.is_occupied(location.component_index) {
                    return Some(chunk.get_component(location.component_index));
                }
            }
        }

        None
    }

    /// Get mutable component reference by global index
    #[inline]
    pub unsafe fn get_component_by_index_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.index_map.len() {
            return None;
        }

        // Get location
        let location = match self.index_map[index] {
            Some(loc) => loc,
            None => return None, // Already removed
        };

        // Get the component
        if location.chunk_index == 0 {
            if let Some(ref mut primary) = self.primary_chunk {
                if primary.is_occupied(location.component_index) {
                    return Some(primary.get_component_mut(location.component_index));
                }
            }
        } else {
            let overflow_idx = location.chunk_index - 1;
            if overflow_idx < self.overflow_chunks.len() {
                let chunk = &mut self.overflow_chunks[overflow_idx];
                if chunk.is_occupied(location.component_index) {
                    return Some(chunk.get_component_mut(location.component_index));
                }
            }
        }

        None
    }

    /// Check if compaction is needed
    #[inline]
    pub fn should_compact(&self) -> bool {
        // Don't compact if too few components
        if self.active_component_count < MIN_COMPONENTS_FOR_COMPACTION {
            return false;
        }

        // Check fragmentation ratio
        let fragmentation = self.fragmentation_ratio();
        fragmentation > COMPACTION_THRESHOLD ||
            (self.overflow_chunks.len() > 1 && !self.free_indices.is_empty())
    }

    /// Compact the pool to improve memory locality
    /// Returns a map of moved components for external update
    pub fn compact(&mut self, arena: &mut Arena) -> Vec<(usize, usize)> {
        let mut relocation_map = Vec::new();

        // Calculate new capacity with additional space
        let new_capacity = std::cmp::max(
            ((self.active_component_count as f32) * 1.5) as usize,
            DEFAULT_COMPONENTS_PER_CHUNK
        );

        // Create new primary chunk
        if let Some(mut new_primary) = Chunk::<T>::new(arena, new_capacity) {
            // Create new index map
            let mut new_index_map = vec![None; self.index_map.len()];

            // Collect active components
            for (global_idx, location_opt) in self.index_map.iter().enumerate() {
                if let Some(location) = location_opt {
                    // Get source chunk
                    let src_chunk = if location.chunk_index == 0 {
                        if let Some(ref primary) = self.primary_chunk {
                            primary
                        } else {
                            continue;
                        }
                    } else {
                        let overflow_idx = location.chunk_index - 1;
                        if overflow_idx >= self.overflow_chunks.len() {
                            continue;
                        }
                        &self.overflow_chunks[overflow_idx]
                    };

                    // Skip if not active
                    if !src_chunk.is_occupied(location.component_index) {
                        continue;
                    }

                    // Allocate in new chunk
                    if let Some(new_idx) = new_primary.allocate_component() {
                        unsafe {
                            // Copy component data
                            let src_comp = src_chunk.get_component(location.component_index);

                            // We need to copy the actual data, not just a reference
                            // The safest way is to use a direct memory copy
                            std::ptr::copy_nonoverlapping(
                                src_comp as *const T,
                                (*new_primary.get_component_ptr(new_idx)).as_mut_ptr(),
                                1
                            );

                            // Update index map
                            new_index_map[global_idx] = Some(ComponentLocation {
                                chunk_index: 0,
                                component_index: new_idx,
                            });

                            // Add to relocation map
                            relocation_map.push((global_idx, new_idx));
                        }
                    } else {
                        // This shouldn't happen with proper size calculation
                        break;
                    }
                }
            }

            // Replace chunks
            let _old_primary = self.primary_chunk.take();
            let _old_overflow = std::mem::take(&mut self.overflow_chunks);

            // Update pool with new data
            self.primary_chunk = Some(new_primary);
            self.index_map = new_index_map;
            self.free_indices.clear();
        }

        relocation_map
    }

    /// Process components in a range (for multithreading)
    #[inline]
    pub unsafe fn process_components_in_range<F>(&self, start: usize, end: usize, mut processor: F)
    where
        F: FnMut(usize, &mut T)
    {
        let real_end = end.min(self.index_map.len());

        if start >= real_end {
            return;
        }

        // Process active components in range
        for global_idx in start..real_end {
            if let Some(location) = self.index_map[global_idx] {
                // Get component
                let component_opt = if location.chunk_index == 0 {
                    if let Some(ref primary) = self.primary_chunk {
                        if primary.is_occupied(location.component_index) {
                            Some(primary.get_component_mut(location.component_index))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    let overflow_idx = location.chunk_index - 1;
                    if overflow_idx < self.overflow_chunks.len() {
                        let chunk = &self.overflow_chunks[overflow_idx];
                        if chunk.is_occupied(location.component_index) {
                            Some(chunk.get_component_mut(location.component_index))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                // Process if found
                if let Some(component) = component_opt {
                    processor(global_idx, component);
                }
            }
        }
    }

    /// Get active component count
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.active_component_count
    }

    /// Check if pool is empty
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.active_component_count == 0
    }

    /// Get fragmentation ratio (0.0 = none, 1.0 = fully fragmented)
    #[inline(always)]
    pub fn fragmentation_ratio(&self) -> f32 {
        if self.index_map.is_empty() {
            return 0.0;
        }

        self.free_indices.len() as f32 / self.index_map.len() as f32
    }

    /// Get thread workload range for parallel processing
    #[inline(always)]
    pub fn get_thread_entity_range(&self, thread_id: usize, thread_count: usize) -> (usize, usize) {
        calculate_thread_entity_range(thread_id, thread_count, self.active_component_count)
    }

    /// Reset pool, clearing all components
    pub fn reset(&mut self) {
        if let Some(ref mut primary) = self.primary_chunk {
            primary.reset();
        }

        for chunk in &mut self.overflow_chunks {
            chunk.reset();
        }

        self.active_component_count = 0;
        self.index_map.clear();
        self.free_indices.clear();
        self.at_max_capacity = false;
    }

    /// Check if the pool is at maximum capacity
    #[inline(always)]
    pub fn is_at_max_capacity(&self) -> bool {
        self.at_max_capacity
    }
}