use std::any::TypeId;
use std::marker::PhantomData;
use std::mem::size_of;
use std::ptr::NonNull;
use crate::ecs::core::component::Component;
use crate::ecs::memory::arena::Arena;
use crate::ecs::memory::chunk::Chunk;
use crate::ecs::memory::component_index::UnitId;
use crate::ecs::constants::{
    DEFAULT_CHUNKS_PER_POOL,
    TINY_COMPONENTS_PER_CHUNK,
    SMALL_COMPONENTS_PER_CHUNK,
    MEDIUM_COMPONENTS_PER_CHUNK,
    LARGE_COMPONENTS_PER_CHUNK,
    TINY_COMPONENT_THRESHOLD,
    SMALL_COMPONENT_THRESHOLD,
    MEDIUM_COMPONENT_THRESHOLD,
};


/// Static component pool handling components of specific type
///
/// Provides cache-friendly component storage using pre-allocated static chunks.
/// Uses swap_remove strategy to maintain data densely packed within each chunk.
/// All chunks are pre-allocated during initialization for maximum performance.
pub struct ComponentPool<T: Component> {
    /// Reference to the arena used for memory allocation
    arena: NonNull<Arena>,

    /// Vector of pre-allocated component chunks
    chunks: Vec<Chunk<T>>,

    /// Index of the current chunk for new component allocations
    current_chunk_index: usize,

    /// Number of active components in the pool
    count: usize,

    /// Number of components each chunk can hold
    capacity_per_chunk: usize,

    component_id: usize,

    /// Component type marker
    _marker: PhantomData<T>,
}

impl<T: Component> ComponentPool<T> {
    /// Creates a new component pool with pre-allocated chunks
    pub fn new(arena: &Arena, num_chunks: usize, components_per_chunk: usize) -> Self {
        let mut chunks = Vec::with_capacity(num_chunks);

        // Pre-allocate all chunks
        for _ in 0..num_chunks {
            chunks.push(Chunk::<T>::new(arena, components_per_chunk));
        }

        Self {
            arena: NonNull::from(arena),
            chunks,
            current_chunk_index: 0,  // Start with the first chunk
            count: 0,
            capacity_per_chunk: components_per_chunk,
            component_id: T::component_id(),
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
    /// O(1) implementation: Always adds to the current chunk,
    /// moving to the next pre-allocated chunk when full.
    pub fn add(&mut self, component: T) -> Option<UnitId> {
        // Check if we have any chunks
        if self.chunks.is_empty() {
            return None;  // Pool is not properly initialized
        }

        // Get the current chunk
        let chunk = &mut self.chunks[self.current_chunk_index];

        // If the current chunk is full, move to the next one
        if chunk.count() >= self.capacity_per_chunk {
            self.current_chunk_index += 1;

            // Check if we've exhausted all chunks
            if self.current_chunk_index >= self.chunks.len() {
                // We've run out of pre-allocated chunks
                return None;
            }
        }

        // Now we're pointing at a chunk with space, use it
        let chunk = &mut self.chunks[self.current_chunk_index];
        let id_inland = match chunk.add(component) {
            Some(idx) => idx,
            None => return None, // This shouldn't happen if capacity is respected
        };

        self.count += 1;
        Some(UnitId::new(self.current_chunk_index, id_inland))
    }

    /// Gets a reference to a component by its index
    pub fn get(&self, index: UnitId) -> Option<&T> {
        let chunk_index = index.id_chunk as usize;
        if chunk_index >= self.chunks.len() {
            return None;
        }

        let chunk = &self.chunks[chunk_index];
        chunk.get(index.id_inland as usize)
    }

    /// Gets a mutable reference to a component by its index
    pub fn get_mut(&mut self, index: UnitId) -> Option<&mut T> {
        let chunk_index = index.id_chunk as usize;
        if chunk_index >= self.chunks.len() {
            return None;
        }

        let chunk = &mut self.chunks[chunk_index];
        chunk.get_mut(index.id_inland as usize)
    }

    /// Removes a component at the specified index using swap_remove strategy
    pub fn swap_remove(&mut self, index: UnitId) -> bool {
        let chunk_index = index.id_chunk as usize;
        if chunk_index >= self.chunks.len() {
            return false;
        }

        let chunk = &mut self.chunks[chunk_index];

        // Try to remove the component from the chunk
        if !chunk.swap_remove(index.id_inland as usize) {
            return false;
        }

        self.count -= 1;
        true
    }

    /// Find all components in a chunk and return them as references
    pub fn chunk_components(&self, chunk_index: usize) -> Option<&[T]> {
        if chunk_index >= self.chunks.len() {
            return None;
        }

        Some(self.chunks[chunk_index].as_slice())
    }

    /// Find all components in a chunk and return them as mutable references
    pub fn chunk_components_mut(&mut self, chunk_index: usize) -> Option<&mut [T]> {
        if chunk_index >= self.chunks.len() {
            return None;
        }

        Some(self.chunks[chunk_index].as_mut_slice())
    }

    /// Gets the number of chunks in this pool
    pub fn chunks_count(&self) -> usize {
        self.chunks.len()
    }

    /// Gets the index of the current top chunk
    pub fn current_chunk_index(&self) -> usize {
        self.current_chunk_index
    }

    /// Gets the count of components in a specific chunk
    pub fn chunk_component_count(&self, chunk_index: usize) -> Option<usize> {
        if chunk_index >= self.chunks.len() {
            return None;
        }

        Some(self.chunks[chunk_index].count())
    }

    /// Check if the pool is full (all chunks are at capacity)
    pub fn is_full(&self) -> bool {
        self.current_chunk_index >= self.chunks.len() - 1 &&
            self.chunks.last().map_or(true, |chunk| chunk.count() >= self.capacity_per_chunk)
    }

    /// Gets the remaining capacity in the pool
    pub fn remaining_capacity(&self) -> usize {
        let total_capacity = self.chunks.len() * self.capacity_per_chunk;
        total_capacity - self.count
    }

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

}
