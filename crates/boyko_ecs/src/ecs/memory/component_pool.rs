use std::alloc::Layout;
use std::ptr::NonNull;
use crate::ecs::constants::{DEFAULT_CHUNKS_PER_POOL, LARGE_COMPONENTS_PER_CHUNK, MEDIUM_COMPONENTS_PER_CHUNK, MEDIUM_COMPONENT_THRESHOLD, SMALL_COMPONENTS_PER_CHUNK, SMALL_COMPONENT_THRESHOLD, TINY_COMPONENTS_PER_CHUNK, TINY_COMPONENT_THRESHOLD};
use crate::ecs::memory::id_unit::Unit;
use crate::ecs::memory::arena::Arena;
use crate::ecs::memory::chunk::Chunk;
use crate::ecs::core::component::component_registry;

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

    /// Component ID - used to look up layout information
    component_id: usize,
    
    /// Component layout (cached from registry for performance)
    component_layout: Layout,
}

impl ComponentPool {
    /// Creates a new component pool with direct memory allocation
    pub fn new(
        arena: &Arena,
        component_id: usize,
        num_chunks: usize,
        components_per_chunk: usize
    ) -> Self {
        // Get layout information from the registry
        debug_assert!(component_id < 512, "Component ID exceeds maximum allowed");
        
        // Get layout from registry - use unsafe fast path for performance
        let registry_layout = unsafe { component_registry::get_layout_unchecked(component_id) };
        let component_layout = registry_layout.layout();
        
        // Calculate total capacity
        let max_components = num_chunks * components_per_chunk;
        let buffer_capacity_bytes = max_components * component_layout.size();

        // Allocate one large buffer for all components
        let buffer_layout = unsafe { 
            Layout::from_size_align_unchecked(
                buffer_capacity_bytes,
                component_layout.align()
            )
        };

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
            component_id,
            component_layout,
        }
    }

    /// Creates a new pool with optimal sizes for the given component type
    pub fn with_default_sizes(arena: &Arena, component_id: usize) -> Self {
        // Get component size from registry
        let component_size = component_registry::get_component_size(component_id)
            .expect("Component not registered");
            
        let components_per_chunk = Self::get_optimal_chunk_capacity(component_size);

        Self::new(arena, component_id, DEFAULT_CHUNKS_PER_POOL, components_per_chunk)
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
    /// 
    /// The component data should be provided as a byte slice
    pub fn add(&mut self, component_bytes: &[u8]) -> Option<usize> {
        // Verify component size
        debug_assert_eq!(component_bytes.len(), self.component_layout.size(), 
            "Component size mismatch: expected {}, got {}", 
            self.component_layout.size(), component_bytes.len());

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
                component_bytes.as_ptr(),
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

    /// Removes the last component from the pool
    pub fn pop(&mut self) -> bool {
        if self.units.is_empty() {
            return false;
        }
        
        // Get the chunk containing this component
        let last_index = self.units.len() - 1;
        let chunk_index = last_index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
        }
        
        // Remove the last unit
        self.units.pop();
        
        true
    }
    
    /// Returns the index of the last component in the pool
    /// This is useful when we need to know what will be affected by a swap_remove
    #[inline]
    pub fn last_index(&self) -> Option<usize> {
        if self.units.is_empty() {
            None
        } else {
            Some(self.units.len() - 1)
        }
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

    /// Gets a pointer to a component by index
    pub fn get_raw(&self, index: usize) -> Option<*const u8> {
        if index >= self.units.len() {
            return None; // Invalid index
        }

        // Get the Unit
        let unit = &self.units[index];

        // Return pointer to the component
        Some(unit.ptr())
    }

    /// Gets a mutable pointer to a component by index
    pub fn get_raw_mut(&mut self, index: usize) -> Option<*mut u8> {
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

        // Return mutable pointer to the component
        Some(unit.ptr())
    }

    /// Sets a component's value at the specified index
    pub fn set_component(&mut self, index: usize, component_bytes: &[u8]) -> bool {
        // Verify component size
        debug_assert_eq!(component_bytes.len(), self.component_layout.size(), 
            "Component size mismatch: expected {}, got {}", 
            self.component_layout.size(), component_bytes.len());

        if index >= self.units.len() {
            return false; // Invalid index
        }

        // Get the Unit
        let unit = &self.units[index];

        // Copy new component value
        unsafe {
            std::ptr::copy_nonoverlapping(
                component_bytes.as_ptr(),
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

    /// Gets all components in a chunk as raw pointers
    pub fn get_chunk_component_pointers(&self, chunk_index: usize) -> Option<Vec<*const u8>> {
        if chunk_index >= self.chunks.len() {
            return None;
        }

        // Collect pointers for all components in this chunk
        let pointers = self.units.iter()
            .enumerate()
            .filter(|(idx, _)| *idx / self.components_per_chunk == chunk_index)
            .map(|(_, unit)| unit.ptr() as *const u8)
            .collect();

        Some(pointers)
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