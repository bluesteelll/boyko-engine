use std::any::TypeId;
use std::marker::PhantomData;
use std::mem::size_of;
use std::ptr::NonNull;
use std::collections::HashMap;
use crate::ecs::core::component::Component;
use crate::ecs::memory::arena::Arena;
use crate::ecs::memory::chunk::Chunk;
use crate::ecs::memory::component_index::ComponentIndex;
use crate::ecs::memory::free_chunk_master::FreeChunkMaster;
use crate::ecs::constants::{
    DEFAULT_CHUNKS_PER_POOL,
    DEFAULT_COMPONENTS_PER_CHUNK,
    TINY_COMPONENTS_PER_CHUNK,
    SMALL_COMPONENTS_PER_CHUNK,
    MEDIUM_COMPONENTS_PER_CHUNK,
    LARGE_COMPONENTS_PER_CHUNK,
    TINY_COMPONENT_THRESHOLD,
    SMALL_COMPONENT_THRESHOLD,
    MEDIUM_COMPONENT_THRESHOLD,
    MAX_EMPTY_CHUNKS_RATIO,
    INITIAL_FREE_SLOTS_CAPACITY,
};

/// Type-erased ComponentPool interface
pub trait IComponentPool {
    /// Gets the TypeId of the component type stored in this pool
    fn component_type_id(&self) -> TypeId;

    /// Gets the size in bytes of the component type
    fn component_size(&self) -> usize;

    /// Gets the number of active components in the pool
    fn count(&self) -> usize;

    /// Gets the total capacity of the pool
    fn capacity(&self) -> usize;

    /// Removes a component at the specified index
    fn swap_remove(&mut self, index: ComponentIndex) -> bool;
}

/// Static component pool handling components of specific type
///
/// Provides cache-friendly component storage using chunks and smart reuse
/// of freed memory for optimal performance.
pub struct ComponentPool<T: Component> {
    /// Reference to the arena used for memory allocation
    arena: NonNull<Arena>,

    /// Vector of component chunks
    chunks: Vec<Chunk<T>>,

    /// Free chunk manager for optimal reuse
    free_chunks: FreeChunkMaster,

    /// Number of active components in the pool
    count: usize,

    /// Number of components each chunk can hold
    capacity_per_chunk: usize,

    /// List of freed component slots for reuse
    free_slots: Vec<ComponentIndex>,

    /// Component type marker
    _marker: PhantomData<T>,
}

impl<T: Component> ComponentPool<T> {
    /// Creates a new component pool with the specified parameters
    pub fn new(arena: &Arena, chunks_per_pool: usize, components_per_chunk: usize) -> Self {
        Self {
            arena: NonNull::from(arena),
            chunks: Vec::with_capacity(chunks_per_pool),
            free_chunks: FreeChunkMaster::with_capacity(chunks_per_pool / 4),
            count: 0,
            capacity_per_chunk: components_per_chunk,
            free_slots: Vec::with_capacity(INITIAL_FREE_SLOTS_CAPACITY),
            _marker: PhantomData,
        }
    }

    /// Creates a new component pool with default sizes based on component type
    pub fn with_default_sizes(arena: &Arena) -> Self {
        let components_per_chunk = Self::get_optimal_chunk_capacity();
        Self::new(arena, DEFAULT_CHUNKS_PER_POOL, components_per_chunk)
    }

    /// Determines the optimal number of components per chunk based on component size
    fn get_optimal_chunk_capacity() -> usize {
        let size = size_of::<T>();
        if size <= TINY_COMPONENT_THRESHOLD {
            TINY_COMPONENTS_PER_CHUNK
        } else if size <= SMALL_COMPONENT_THRESHOLD {
            SMALL_COMPONENTS_PER_CHUNK
        } else if size <= MEDIUM_COMPONENT_THRESHOLD {
            MEDIUM_COMPONENTS_PER_CHUNK
        } else {
            LARGE_COMPONENTS_PER_CHUNK
        }
    }

    /// Adds a component to the pool, returning its index
    ///
    /// Uses a cache-friendly allocation strategy prioritizing:
    /// 1. Last active chunk (hot cache)
    /// 2. Free slots in newest chunks
    /// 3. Recently emptied chunks (managed by FreeChunkMaster)
    /// 4. New chunk allocation as last resort
    pub fn add(&mut self, component: T) -> Option<ComponentIndex> {
        // 1. First check if the last active chunk has space (best for cache locality)
        if !self.chunks.is_empty() {
            let last_chunk_index = self.chunks.len() - 1;
            let last_chunk = &mut self.chunks[last_chunk_index];

            if last_chunk.count() < self.capacity_per_chunk {
                let id_inland = last_chunk.add(component).unwrap();
                self.count += 1;
                return Some(ComponentIndex::new(last_chunk_index, id_inland));
            }
        }

        // 2. Check for free slots, prioritizing those in higher-indexed chunks
        if !self.free_slots.is_empty() {
            // Look for a slot in the last chunk first (hot cache)
            if !self.chunks.is_empty() {
                let last_chunk_index = self.chunks.len() - 1;

                // Try to find a slot in the last chunk
                for i in 0..self.free_slots.len() {
                    if self.free_slots[i].id_chunk as usize == last_chunk_index {
                        let slot = self.free_slots.swap_remove(i);
                        let chunk = &mut self.chunks[slot.id_chunk as usize];
                        chunk.set(slot.id_inland as usize, component);
                        self.count += 1;
                        return Some(slot);
                    }
                }
            }

            // If no slots in the last chunk, just use any available slot
            let slot = self.free_slots.pop().unwrap();
            let chunk = &mut self.chunks[slot.id_chunk as usize];
            chunk.set(slot.id_inland as usize, component);
            self.count += 1;
            return Some(slot);
        }

        // 3. Try to reuse an empty chunk from the free chunk pool
        // FreeChunkMaster automatically gives us the best chunk for cache locality
        if let Some(chunk_index) = self.free_chunks.get_best_chunk() {
            let chunk = &mut self.chunks[chunk_index];
            let id_inland = chunk.add(component).unwrap();
            self.count += 1;
            return Some(ComponentIndex::new(chunk_index, id_inland));
        }

        // 4. If no available slots or chunks, create a new chunk
        let arena = unsafe { &*self.arena.as_ptr() };
        let mut new_chunk = Chunk::<T>::new(arena, self.capacity_per_chunk);
        let id_inland = new_chunk.add(component).unwrap();
        let id_chunk = self.chunks.len();
        self.chunks.push(new_chunk);
        self.count += 1;

        Some(ComponentIndex::new(id_chunk, id_inland))
    }

    /// Gets a reference to a component by its index
    pub fn get(&self, index: ComponentIndex) -> Option<&T> {
        let chunk_index = index.id_chunk as usize;
        if chunk_index >= self.chunks.len() {
            return None;
        }

        let chunk = &self.chunks[chunk_index];
        chunk.get(index.id_inland as usize)
    }

    /// Gets a mutable reference to a component by its index
    pub fn get_mut(&mut self, index: ComponentIndex) -> Option<&mut T> {
        let chunk_index = index.id_chunk as usize;
        if chunk_index >= self.chunks.len() {
            return None;
        }

        let chunk = &mut self.chunks[chunk_index];
        chunk.get_mut(index.id_inland as usize)
    }

    /// Removes a component at the specified index using swap-remove strategy
    ///
    /// Instead of deleting empty chunks, they are stored in the free chunk pool
    /// for later reuse, improving memory efficiency and allocation performance.
    pub fn swap_remove(&mut self, index: ComponentIndex) -> bool {
        let chunk_index = index.id_chunk as usize;
        if chunk_index >= self.chunks.len() {
            return false;
        }

        let chunk = &mut self.chunks[chunk_index];

        // Try to remove the component from the chunk
        if chunk.swap_remove(index.id_inland as usize) {
            // If the chunk is now empty, add it to free_chunks
            if chunk.count() == 0 {
                // FreeChunkMaster handles duplicate prevention internally
                self.free_chunks.add_chunk(chunk_index);

                // Maybe trigger compaction if too many empty chunks
                self.maybe_compact();
            } else {
                // Add the freed slot to free_slots
                self.free_slots.push(index);
            }

            self.count -= 1;
            return true;
        }

        false
    }

    /// Checks if we should compact the pool and does it if necessary
    fn maybe_compact(&mut self) {
        // Only compact if we have too many empty chunks
        if !self.free_chunks.is_empty() && !self.chunks.is_empty() {
            let empty_ratio = self.free_chunks.len() as f32 / self.chunks.len() as f32;

            if empty_ratio > MAX_EMPTY_CHUNKS_RATIO {
                self.compact();
            }
        }
    }

    /// Compacts the pool by removing some empty chunks to reduce memory usage
    ///
    /// Keeps some empty chunks for future reuse while freeing memory from others.
    /// Maintains indices to ensure component references remain valid.
    pub fn compact(&mut self) {
        // Keep some empty chunks (e.g., 20% of what we have)
        let chunks_to_keep = (self.free_chunks.len() as f32 * 0.2) as usize;

        // Get chunks to remove from FreeChunkMaster (it handles the sorting internally)
        let chunks_to_remove = self.free_chunks.get_chunks_to_remove(chunks_to_keep);

        if chunks_to_remove.is_empty() {
            return;
        }

        // First update the free chunks master
        self.free_chunks.remove_chunks(&chunks_to_remove);

        // Now remove the chunks in descending order to avoid index shifting problems
        let mut sorted_indices = chunks_to_remove;
        sorted_indices.sort_unstable_by(|a, b| b.cmp(a));

        // Track index remapping when chunks are moved
        let mut index_remap = HashMap::new();

        for chunk_index in sorted_indices {
            // Skip if we're about to remove the last chunk
            if self.chunks.len() <= 1 {
                break;
            }

            // Get the index of the last chunk (which will move)
            let last_chunk_index = self.chunks.len() - 1;

            // Only need to remap if the removed chunk is not the last one
            if chunk_index < last_chunk_index {
                index_remap.insert(last_chunk_index, chunk_index);
            }

            // Remove the chunk
            self.chunks.swap_remove(chunk_index);
        }

        // Update all free slots based on the remap
        if !index_remap.is_empty() {
            for slot in self.free_slots.iter_mut() {
                let chunk_idx = slot.id_chunk as usize;
                if let Some(&new_idx) = index_remap.get(&chunk_idx) {
                    slot.id_chunk = new_idx as u32;
                }
            }
        }
    }
}

impl<T: Component> IComponentPool for ComponentPool<T> {
    fn component_type_id(&self) -> TypeId {
        TypeId::of::<T>()
    }

    fn component_size(&self) -> usize {
        size_of::<T>()
    }

    fn count(&self) -> usize {
        self.count
    }

    fn capacity(&self) -> usize {
        self.chunks.len() * self.capacity_per_chunk
    }

    fn swap_remove(&mut self, index: ComponentIndex) -> bool {
        self.swap_remove(index)
    }
}

/// Factory for creating type-erased component pools
pub struct ComponentPoolFactory;

impl ComponentPoolFactory {
    /// Creates a new component pool for the specified component type
    pub fn create<T: Component + 'static>(arena: &Arena) -> Box<dyn IComponentPool> {
        Box::new(ComponentPool::<T>::with_default_sizes(arena))
    }
}
