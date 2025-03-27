use std::alloc::Layout;
use std::any::TypeId;
use std::ptr::NonNull;
use crate::ecs::core::component::Component;
use crate::ecs::memory::arena::Arena;
use crate::ecs::memory::chunk::Chunk;
use crate::ecs::identifiers::id_unit::UnitId;
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

/// Component pool that manages chunks of components with centralized type information.
/// Holds all type metadata in the pool and passes it to chunks as needed for operations.
pub struct ComponentPool {
    /// Reference to the arena used for memory allocation
    arena: NonNull<Arena>,

    /// Vector of component chunks
    chunks: Vec<Chunk>,

    /// Index of the current chunk for new allocations
    current_chunk_index: usize,

    /// Number of active components in the pool
    count: usize,

    /// Number of components each chunk can hold
    capacity_per_chunk: usize,

    /// Type information - centralized in the pool
    type_id: TypeId,
    component_id: usize,
    component_layout: Layout,
}

impl ComponentPool {
    /// Creates a new component pool for a specific component type
    pub fn new<T: Component>(
        arena: &Arena,
        num_chunks: usize,
        components_per_chunk: usize
    ) -> Self {
        let component_layout = Layout::new::<T>();
        let type_id = TypeId::of::<T>();
        let component_id = T::component_id();

        let mut chunks = Vec::with_capacity(num_chunks);

        // Pre-allocate all chunks
        for _ in 0..num_chunks {
            chunks.push(Chunk::new(arena, components_per_chunk, component_layout));
        }

        Self {
            arena: NonNull::from(arena),
            chunks,
            current_chunk_index: 0,
            count: 0,
            capacity_per_chunk: components_per_chunk,
            type_id,
            component_id,
            component_layout,
        }
    }

    /// Creates a new component pool with default sizes based on component type
    pub fn with_default_sizes<T: Component>(arena: &Arena) -> Self {
        let component_size = std::mem::size_of::<T>();
        let components_per_chunk = Self::get_optimal_chunk_capacity(component_size);

        Self::new::<T>(arena, DEFAULT_CHUNKS_PER_POOL, components_per_chunk)
    }

    /// Determines the optimal number of components per chunk based on size
    fn get_optimal_chunk_capacity(component_size: usize) -> usize {
        if component_size <= TINY_COMPONENT_THRESHOLD {
            TINY_COMPONENTS_PER_CHUNK
        } else if component_size <= SMALL_COMPONENT_THRESHOLD {
            SMALL_COMPONENTS_PER_CHUNK
        } else if component_size <= MEDIUM_COMPONENT_THRESHOLD {
            MEDIUM_COMPONENTS_PER_CHUNK
        } else {
            LARGE_COMPONENTS_PER_CHUNK
        }
    }

    //
    // Raw operations
    //

    /// Adds raw component bytes to the pool
    ///
    /// # Safety
    /// The caller must ensure the bytes represent a valid component of the pool's type
    pub unsafe fn raw_add(&mut self, bytes: *const u8) -> Option<UnitId> {
        // Check if we have any chunks
        if self.chunks.is_empty() {
            return None;
        }

        // Find a chunk with space
        while self.current_chunk_index < self.chunks.len() {
            let chunk = &mut self.chunks[self.current_chunk_index];

            if chunk.count() < self.capacity_per_chunk {
                // This chunk has space, use it
                let inland_index = match chunk.raw_add(bytes, self.component_layout) {
                    Some(idx) => idx,
                    None => return None, // Shouldn't happen if count < capacity
                };

                self.count += 1;
                return Some(UnitId::new(self.current_chunk_index, inland_index));
            }

            // Current chunk is full, try the next one
            self.current_chunk_index += 1;
        }

        // All chunks are full
        None
    }

    /// Gets a raw pointer to a component by its index
    pub fn raw_get(&self, index: UnitId) -> Option<*const u8> {
        let chunk_index = index.chunk_index();
        if chunk_index >= self.chunks.len() {
            return None;
        }

        self.chunks[chunk_index].raw_get(index.inland_index(), self.component_layout)
    }

    /// Gets a mutable raw pointer to a component by its index
    pub fn raw_get_mut(&mut self, index: UnitId) -> Option<*mut u8> {
        let chunk_index = index.chunk_index();
        if chunk_index >= self.chunks.len() {
            return None;
        }

        self.chunks[chunk_index].raw_get_mut(index.inland_index(), self.component_layout)
    }

    //
    // Type-safe operations
    //

    /// Adds a component to the pool, checking types at runtime
    pub fn add<T: Component>(&mut self, component: T) -> Option<UnitId> {
        if TypeId::of::<T>() != self.type_id {
            return None; // Type mismatch
        }

        unsafe {
            self.raw_add(&component as *const T as *const u8)
        }
    }

    /// Gets a reference to a component by its index, checking types at runtime
    pub fn get<T: Component>(&self, index: UnitId) -> Option<&T> {
        if TypeId::of::<T>() != self.type_id {
            return None; // Type mismatch
        }

        let ptr = self.raw_get(index)?;
        unsafe { Some(&*(ptr as *const T)) }
    }

    /// Gets a mutable reference to a component by its index, checking types at runtime
    pub fn get_mut<T: Component>(&mut self, index: UnitId) -> Option<&mut T> {
        if TypeId::of::<T>() != self.type_id {
            return None; // Type mismatch
        }

        let ptr = self.raw_get_mut(index)?;
        unsafe { Some(&mut *(ptr as *mut T)) }
    }

    /// Removes a component at the specified index using swap_remove strategy
    pub fn swap_remove(&mut self, index: UnitId) -> bool {
        let chunk_index = index.chunk_index();
        if chunk_index >= self.chunks.len() {
            return false;
        }

        let chunk = &mut self.chunks[chunk_index];
        if !chunk.swap_remove(index.inland_index(), self.component_layout) {
            return false;
        }

        self.count -= 1;
        true
    }

    //
    // Chunk-level access
    //

    /// Gets typed access to a chunk's components
    pub fn chunk_components<T: Component>(&self, chunk_index: usize) -> Option<&[T]> {
        if TypeId::of::<T>() != self.type_id || chunk_index >= self.chunks.len() {
            return None;
        }

        let chunk = &self.chunks[chunk_index];
        let count = chunk.count();

        if count == 0 {
            return Some(&[]);
        }

        unsafe {
            let ptr = chunk.data_ptr() as *const T;
            Some(std::slice::from_raw_parts(ptr, count))
        }
    }

    /// Gets mutable typed access to a chunk's components
    pub fn chunk_components_mut<T: Component>(&mut self, chunk_index: usize) -> Option<&mut [T]> {
        if TypeId::of::<T>() != self.type_id || chunk_index >= self.chunks.len() {
            return None;
        }

        let chunk = &mut self.chunks[chunk_index];
        let count = chunk.count();

        if count == 0 {
            return Some(&mut []);
        }

        unsafe {
            let ptr = chunk.data_ptr_mut() as *mut T;
            Some(std::slice::from_raw_parts_mut(ptr, count))
        }
    }

    //
    // Pool information
    //

    #[inline]
    pub fn chunks_count(&self) -> usize {
        self.chunks.len()
    }

    #[inline]
    pub fn current_chunk_index(&self) -> usize {
        self.current_chunk_index
    }

    #[inline]
    pub fn chunk_component_count(&self, chunk_index: usize) -> Option<usize> {
        self.chunks.get(chunk_index).map(|chunk| chunk.count())
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.current_chunk_index >= self.chunks.len() - 1 &&
            self.chunks.last().map_or(true, |chunk| chunk.count() >= self.capacity_per_chunk)
    }

    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        let total_capacity = self.chunks.len() * self.capacity_per_chunk;
        total_capacity.saturating_sub(self.count)
    }

    #[inline]
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    #[inline]
    pub fn component_id(&self) -> usize {
        self.component_id
    }

    #[inline]
    pub fn component_layout(&self) -> Layout {
        self.component_layout
    }

    #[inline]
    pub fn count(&self) -> usize {
        self.count
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.chunks.len() * self.capacity_per_chunk
    }
}