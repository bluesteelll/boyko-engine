use std::alloc::Layout;
use std::any::TypeId;
use std::ptr::NonNull;
use crate::ecs::constants::{DEFAULT_CHUNKS_PER_POOL, LARGE_COMPONENTS_PER_CHUNK, MEDIUM_COMPONENTS_PER_CHUNK, MEDIUM_COMPONENT_THRESHOLD, SMALL_COMPONENTS_PER_CHUNK, SMALL_COMPONENT_THRESHOLD, TINY_COMPONENTS_PER_CHUNK, TINY_COMPONENT_THRESHOLD};
use crate::ecs::core::component::Component;
use crate::ecs::identifiers::id_unit::Unit;
use crate::ecs::memory::arena::Arena;
use crate::ecs::memory::chunk::Chunk;

/// Pool of components of a specific type with direct pointers
pub struct ComponentPool {
    /// Reference to the arena for memory allocation
    arena: NonNull<Arena>,

    /// Buffer for storing components, allocated directly from the arena
    buffer: NonNull<u8>,

    /// Buffer capacity in bytes
    buffer_capacity_bytes: usize,

    /// Maximum number of components
    max_components: usize,

    /// Array of units with direct pointers (always densely packed)
    units: Vec<Unit>,

    /// Chunk metadata
    pub chunks: Vec<Chunk>,

    /// Components per chunk
    components_per_chunk: usize,

    /// Component type information
    type_id: TypeId,
    component_id: usize,
    component_layout: Layout,
}

impl ComponentPool {
    /// Creates a new component pool with direct memory allocation
    pub fn new<T: Component>(
        arena: &Arena,
        num_chunks: usize,
        components_per_chunk: usize
    ) -> Self {
        let component_layout = Layout::new::<T>();
        let type_id = TypeId::of::<T>();
        let component_id = T::component_id();

        // Calculate total capacity
        let max_components = num_chunks * components_per_chunk;
        let buffer_capacity_bytes = max_components * component_layout.size();

        // Allocate one large buffer for all components
        let buffer_layout = Layout::from_size_align(
            buffer_capacity_bytes,
            component_layout.align()
        ).expect("Invalid buffer layout");

        let buffer = arena.allocate_layout(buffer_layout);

        // Create chunk metadata
        let mut chunks = Vec::with_capacity(num_chunks);
        for i in 0..num_chunks {
            let start_index = i * components_per_chunk;
            chunks.push(Chunk::new(start_index, components_per_chunk));
        }

        Self {
            arena: NonNull::from(arena),
            buffer,
            buffer_capacity_bytes,
            max_components,
            units: Vec::with_capacity(max_components),
            chunks,
            components_per_chunk,
            type_id,
            component_id,
            component_layout,
        }
    }

    /// Creates a new pool with optimal sizes for the given component type
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

    /// Adds a component to the pool
    pub fn add<T: Component>(&mut self, component: T) -> Option<usize> {
        if TypeId::of::<T>() != self.type_id {
            return None; // Type mismatch
        }

        if self.units.len() >= self.max_components {
            return None; // Pool is full
        }

        // Calculate buffer index for the new component
        let buffer_index = self.units.len();

        // Calculate pointer to the next free position in the buffer
        let component_ptr = unsafe {
            let ptr = self.buffer.as_ptr().add(buffer_index * self.component_layout.size());

            // Copy component to buffer
            std::ptr::copy_nonoverlapping(
                &component as *const T as *const u8,
                ptr,
                self.component_layout.size()
            );

            ptr as *mut u8
        };

        // Create Unit with direct pointer
        let unit = Unit::new(component_ptr, buffer_index);

        // Calculate chunk index and mark it as dirty
        let chunk_index = buffer_index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
        }

        // Store Unit
        self.units.push(unit);

        // Return the index of the newly added component
        Some(buffer_index)
    }

    /// Adds raw component bytes to the pool
    ///
    /// # Safety
    /// Caller must ensure the bytes represent a valid component of the pool's type
    pub unsafe fn raw_add(&mut self, bytes: *const u8) -> Option<usize> {
        if self.units.len() >= self.max_components {
            return None; // Pool is full
        }

        // Calculate buffer index for the new component
        let buffer_index = self.units.len();

        // Calculate pointer to the next free position in the buffer
        let component_ptr = {
            let ptr = self.buffer.as_ptr().add(buffer_index * self.component_layout.size());

            // Copy component to buffer
            std::ptr::copy_nonoverlapping(
                bytes,
                ptr,
                self.component_layout.size()
            );

            ptr as *mut u8
        };

        // Create Unit with direct pointer
        let unit = Unit::new(component_ptr, buffer_index);

        // Calculate chunk index and mark it as dirty
        let chunk_index = buffer_index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
        }

        // Store Unit
        self.units.push(unit);

        // Return the index of the newly added component
        Some(buffer_index)
    }

    /// Removes a component by index using swap_remove to maintain dense storage
    pub fn swap_remove(&mut self, index: usize) -> bool {
        if index >= self.units.len() {
            return false; // Invalid index
        }

        // Mark the chunk containing the component as dirty
        let chunk_index = index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
        }

        // Get the pointer to the component being removed
        let removed_unit_ptr = self.units[index].ptr();

        // If this is not the last component, replace it with the last one
        if index < self.units.len() - 1 {
            // Get the last unit
            let last_unit_index = self.units.len() - 1;
            let last_unit = self.units[last_unit_index];

            // Copy the last component's data to the removed position
            unsafe {
                std::ptr::copy_nonoverlapping(
                    last_unit.ptr(),
                    removed_unit_ptr,
                    self.component_layout.size()
                );
            }

            // Update the Unit in the array to reflect the new position
            self.units[index] = Unit::new(
                removed_unit_ptr,
                index // New buffer index is the index in the array
            );

            // Mark the chunk of the last component as dirty
            let last_chunk_index = last_unit_index / self.components_per_chunk;
            if let Some(chunk) = self.chunks.get_mut(last_chunk_index) {
                chunk.mark_dirty();
            }
        }

        // Remove the last Unit from the array
        self.units.pop();

        true
    }

    /// Gets a reference to a component by index
    pub fn get<T: Component>(&self, index: usize) -> Option<&T> {
        if TypeId::of::<T>() != self.type_id {
            return None; // Type mismatch
        }

        if index >= self.units.len() {
            return None; // Invalid index
        }

        // Get the Unit
        let unit = &self.units[index];

        // Return reference to the component
        unsafe {
            Some(&*(unit.ptr() as *const T))
        }
    }

    /// Gets a mutable reference to a component by index
    pub fn get_mut<T: Component>(&mut self, index: usize) -> Option<&mut T> {
        if TypeId::of::<T>() != self.type_id {
            return None; // Type mismatch
        }

        if index >= self.units.len() {
            return None; // Invalid index
        }

        // Get the Unit
        let unit = &self.units[index];

        // Mark the chunk as dirty
        let chunk_index = index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
        }

        // Return mutable reference to the component
        unsafe {
            Some(&mut *(unit.ptr() as *mut T))
        }
    }

    /// Sets a component's value at the specified index
    pub fn set_component<T: Component>(&mut self, index: usize, component: T) -> bool {
        if TypeId::of::<T>() != self.type_id {
            return false; // Type mismatch
        }

        if index >= self.units.len() {
            return false; // Invalid index
        }

        // Get the Unit
        let unit = &self.units[index];

        // Copy new component value
        unsafe {
            std::ptr::copy_nonoverlapping(
                &component as *const T as *const u8,
                unit.ptr(),
                self.component_layout.size()
            );
        }

        // Mark the chunk as dirty
        let chunk_index = index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
        }

        true
    }

    /// Gets a direct pointer to a component
    pub fn raw_get(&self, index: usize) -> Option<*const u8> {
        if index >= self.units.len() {
            return None; // Invalid index
        }

        Some(self.units[index].ptr())
    }

    /// Gets a mutable direct pointer to a component
    pub fn raw_get_mut(&mut self, index: usize) -> Option<*mut u8> {
        if index >= self.units.len() {
            return None; // Invalid index
        }

        // Mark the chunk as dirty
        let chunk_index = index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
        }

        Some(self.units[index].ptr())
    }

    /// Gets all components in a chunk
    pub fn chunk_components<T: Component>(&self, chunk_index: usize) -> Option<Vec<&T>> {
        if TypeId::of::<T>() != self.type_id || chunk_index >= self.chunks.len() {
            return None;
        }

        // Filter Units that belong to this chunk
        let components_in_chunk: Vec<&T> = self.units.iter()
            .enumerate()
            .filter(|(idx, _)| *idx / self.components_per_chunk == chunk_index)
            .map(|(_, unit)| unsafe { &*(unit.ptr() as *const T) })
            .collect();

        Some(components_in_chunk)
    }

    /// Gets mutable components in a chunk
    pub fn chunk_components_mut<T: Component>(&mut self, chunk_index: usize) -> Option<Vec<&mut T>> {
        if TypeId::of::<T>() != self.type_id || chunk_index >= self.chunks.len() {
            return None;
        }

        let chunk = &mut self.chunks[chunk_index];
        chunk.mark_dirty();

        // This is a bit tricky since we need to collect mutable references
        // We need to manually create raw pointers first then convert to mutable references
        let mut components_in_chunk = Vec::new();

        for (idx, unit) in self.units.iter().enumerate() {
            if idx / self.components_per_chunk == chunk_index {
                unsafe {
                    components_in_chunk.push(&mut *(unit.ptr() as *mut T));
                }
            }
        }

        Some(components_in_chunk)
    }



    /// Gets the number of active components
    #[inline]
    pub fn count(&self) -> usize {
        self.units.len()
    }

    /// Gets the total pool capacity
    #[inline]
    pub fn capacity(&self) -> usize {
        self.max_components
    }

    /// Gets the number of chunks
    #[inline]
    pub fn chunks_count(&self) -> usize {
        self.chunks.len()
    }

    /// Gets the component type ID
    #[inline]
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Gets the component ID
    #[inline]
    pub fn component_id(&self) -> usize {
        self.component_id
    }

    /// Gets the component layout
    #[inline]
    pub fn component_layout(&self) -> Layout {
        self.component_layout
    }

    /// Checks if the pool is full
    #[inline]
    pub fn is_full(&self) -> bool {
        self.units.len() >= self.max_components
    }

    /// Gets the remaining capacity
    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        self.max_components - self.units.len()
    }
}