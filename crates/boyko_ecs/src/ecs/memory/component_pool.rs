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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::Component;
    use crate::ecs::memory::arena::Arena;
    use crate::ecs::constants::{TINY_COMPONENT_THRESHOLD, SMALL_COMPONENT_THRESHOLD, MEDIUM_COMPONENT_THRESHOLD, TINY_COMPONENTS_PER_CHUNK, SMALL_COMPONENTS_PER_CHUNK, MEDIUM_COMPONENTS_PER_CHUNK, LARGE_COMPONENTS_PER_CHUNK};
    use std::any::TypeId;

    // Define a simple component type for testing
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct TestComponent {
        value: u32
    }

    impl Component for TestComponent {
        #[inline(always)]
        fn component_id() -> usize {
            0 // Static ID for testing
        }
    }

    // Create a helper function to set up the pool
    fn setup_pool() -> (Arena, ComponentPool) {
        let arena = Arena::new();
        let pool = ComponentPool::with_default_sizes::<TestComponent>(&arena);
        (arena, pool)
    }

    // Helper to calculate components per chunk based on TestComponent size
    fn get_components_per_chunk() -> usize {
        if std::mem::size_of::<TestComponent>() <= TINY_COMPONENT_THRESHOLD {
            TINY_COMPONENTS_PER_CHUNK
        } else if std::mem::size_of::<TestComponent>() <= SMALL_COMPONENT_THRESHOLD {
            SMALL_COMPONENTS_PER_CHUNK
        } else if std::mem::size_of::<TestComponent>() <= MEDIUM_COMPONENT_THRESHOLD {
            MEDIUM_COMPONENTS_PER_CHUNK
        } else {
            LARGE_COMPONENTS_PER_CHUNK
        }
    }

    #[test]
    fn test_create_pool() {
        let (_, pool) = setup_pool();
        assert_eq!(pool.count(), 0);
        assert_eq!(pool.component_id(), 0);
        assert!(pool.component_layout().size() > 0);
        assert!(!pool.is_full());
    }

    #[test]
    fn test_add_component() {
        let (_, mut pool) = setup_pool();
        let component = TestComponent { value: 42 };

        let index = pool.add(component).expect("Failed to add component");

        assert_eq!(pool.count(), 1);

        let retrieved = pool.get::<TestComponent>(index).expect("Failed to get component");
        assert_eq!(retrieved.value, 42);
    }

    #[test]
    fn test_add_multiple_components() {
        let (_, mut pool) = setup_pool();

        // Add components
        let indices = vec![
            pool.add(TestComponent { value: 1 }).expect("Failed to add component 1"),
            pool.add(TestComponent { value: 2 }).expect("Failed to add component 2"),
            pool.add(TestComponent { value: 3 }).expect("Failed to add component 3"),
        ];

        assert_eq!(pool.count(), 3);

        // Check if we can retrieve all components correctly
        for (i, &index) in indices.iter().enumerate() {
            let component = pool.get::<TestComponent>(index).expect("Failed to get component");
            assert_eq!(component.value, i as u32 + 1);
        }
    }

    #[test]
    fn test_swap_remove_basic() {
        let (_, mut pool) = setup_pool();

        // Add components
        let idx1 = pool.add(TestComponent { value: 1 }).expect("Failed to add component 1");
        let idx2 = pool.add(TestComponent { value: 2 }).expect("Failed to add component 2");

        assert_eq!(pool.count(), 2);

        // Remove the first component
        let success = pool.swap_remove(idx1);
        assert!(success);

        // Verify count decreased
        assert_eq!(pool.count(), 1);

        // Since it was swap_remove, the second component should now be at index 0
        let component = pool.get::<TestComponent>(0).expect("Failed to get component after swap_remove");
        assert_eq!(component.value, 2);

        // Trying to get the removed component (by its original index) should fail
        assert!(pool.get::<TestComponent>(1).is_none());
    }

    #[test]
    fn test_swap_remove_last() {
        let (_, mut pool) = setup_pool();

        // Add components
        let _idx1 = pool.add(TestComponent { value: 1 }).expect("Failed to add component 1");
        let idx2 = pool.add(TestComponent { value: 2 }).expect("Failed to add component 2");

        assert_eq!(pool.count(), 2);

        // Remove the last component
        let success = pool.swap_remove(idx2);
        assert!(success);

        // Verify count decreased
        assert_eq!(pool.count(), 1);

        // First component should still be at index 0
        let component = pool.get::<TestComponent>(0).expect("Failed to get component after swap_remove");
        assert_eq!(component.value, 1);

        // Trying to get the removed component should fail
        assert!(pool.get::<TestComponent>(1).is_none());
    }

    #[test]
    fn test_swap_remove_invalid_index() {
        let (_, mut pool) = setup_pool();

        // Add a component
        let _idx = pool.add(TestComponent { value: 1 }).expect("Failed to add component");

        // Try to remove with an invalid index
        let success = pool.swap_remove(999);
        assert!(!success);

        // Pool should still have 1 component
        assert_eq!(pool.count(), 1);
    }

    #[test]
    fn test_swap_remove_empty_pool() {
        let (_, mut pool) = setup_pool();

        // Try to remove from an empty pool
        let success = pool.swap_remove(0);
        assert!(!success);

        // Pool should still be empty
        assert_eq!(pool.count(), 0);
    }

    #[test]
    fn test_type_safety() {
        let (_, mut pool) = setup_pool();

        // Add a component
        let idx = pool.add(TestComponent { value: 42 }).expect("Failed to add component");

        // Try to get it as a different type
        struct WrongComponent {
            x: f32
        }

        impl Component for WrongComponent {
            fn component_id() -> usize {
                1 // Different ID than TestComponent
            }
        }

        // This should fail because the type is wrong
        assert!(pool.get::<WrongComponent>(idx).is_none());
    }

    #[test]
    fn test_memory_safety() {
        let (_, mut pool) = setup_pool();

        // Add many components
        let mut indices = Vec::new();
        for i in 0..100 {
            let idx = pool.add(TestComponent { value: i }).expect("Failed to add component");
            indices.push(idx);
        }

        // Remove some components in the middle
        for i in 25..75 {
            let success = pool.swap_remove(indices[i]);
            assert!(success);
        }

        // Add more components
        for i in 100..150 {
            let _idx = pool.add(TestComponent { value: i }).expect("Failed to add component");
        }

        // Check components 0-24 (should be unchanged)
        for i in 0..25 {
            let component = pool.get::<TestComponent>(indices[i]).expect("Failed to get component");
            assert_eq!(component.value, i as u32);
        }

        // Check that all components are still accessible
        let count = pool.count();
        for i in 0..count {
            let component = pool.get::<TestComponent>(i);
            assert!(component.is_some(), "Component at index {} is missing", i);
        }
    }

    #[test]
    fn test_get_mut_and_set() {
        let (_, mut pool) = setup_pool();

        // Add a component
        let idx = pool.add(TestComponent { value: 42 }).expect("Failed to add component");

        // Modify it through get_mut
        {
            let component = pool.get_mut::<TestComponent>(idx).expect("Failed to get component mutably");
            component.value = 100;
        }

        // Check if the change took effect
        let component = pool.get::<TestComponent>(idx).expect("Failed to get component");
        assert_eq!(component.value, 100);

        // Test set_component
        let success = pool.set_component(idx, TestComponent { value: 200 });
        assert!(success);

        // Check if the change took effect
        let component = pool.get::<TestComponent>(idx).expect("Failed to get component");
        assert_eq!(component.value, 200);
    }

    #[test]
    fn test_chunk_components() {
        let (_, mut pool) = setup_pool();

        // Add enough components to span multiple chunks
        let components_per_chunk = get_components_per_chunk();
        let num_components = components_per_chunk + 10; // Enough to have components in two chunks

        for i in 0..num_components {
            let _idx = pool.add(TestComponent { value: i as u32 }).expect("Failed to add component");
        }

        // Check components in the first chunk
        let chunk0_components = pool.chunk_components::<TestComponent>(0).expect("Failed to get chunk components");
        assert_eq!(chunk0_components.len(), components_per_chunk);

        for (i, component) in chunk0_components.iter().enumerate() {
            assert_eq!(component.value, i as u32);
        }

        // Check components in the second chunk
        let chunk1_components = pool.chunk_components::<TestComponent>(1).expect("Failed to get chunk components");
        assert_eq!(chunk1_components.len(), 10); // We added 10 extra components

        for (i, component) in chunk1_components.iter().enumerate() {
            assert_eq!(component.value, (components_per_chunk + i) as u32);
        }
    }

    #[test]
    fn test_swap_remove_across_chunks() {
        let (_, mut pool) = setup_pool();

        // Add enough components to span multiple chunks
        let components_per_chunk = get_components_per_chunk();
        let num_components = components_per_chunk + 10; // Enough to have components in two chunks

        // Add components
        let mut indices = Vec::new();
        for i in 0..num_components {
            let idx = pool.add(TestComponent { value: i as u32 }).expect("Failed to add component");
            indices.push(idx);
        }

        // Remove a component from the first chunk
        let removed_index = components_per_chunk / 2; // Middle of first chunk
        pool.swap_remove(indices[removed_index]);

        // This will have moved the last component (from the second chunk) to the first chunk
        // Verify that all components are still accessible and have correct values

        // First chunk should have all original components except the removed one
        // The last component from second chunk should now be in first chunk
        for i in 0..removed_index {
            let component = pool.get::<TestComponent>(indices[i]).expect("Failed to get component");
            assert_eq!(component.value, i as u32);
        }

        // The removed component's slot should now contain the last component
        let component = pool.get::<TestComponent>(indices[removed_index]).expect("Failed to get component");
        assert_eq!(component.value, (num_components - 1) as u32);

        // The components after the removed one should be unchanged
        for i in (removed_index + 1)..num_components - 1 {
            let component = pool.get::<TestComponent>(indices[i]).expect("Failed to get component");
            assert_eq!(component.value, i as u32);
        }
    }

    #[test]
    fn test_consecutive_swap_removes() {
        let (_, mut pool) = setup_pool();

        // Add several components
        let indices = vec![
            pool.add(TestComponent { value: 1 }).expect("Failed to add component 1"),
            pool.add(TestComponent { value: 2 }).expect("Failed to add component 2"),
            pool.add(TestComponent { value: 3 }).expect("Failed to add component 3"),
            pool.add(TestComponent { value: 4 }).expect("Failed to add component 4"),
            pool.add(TestComponent { value: 5 }).expect("Failed to add component 5"),
        ];

        // Remove components in the middle multiple times
        pool.swap_remove(indices[1]); // Remove value 2, value 5 moves to position 1
        pool.swap_remove(indices[2]); // Remove value 3, value 4 moves to position 2

        // Check remaining components
        assert_eq!(pool.count(), 3);

        // First component should still be at index 0
        let component = pool.get::<TestComponent>(indices[0]).expect("Failed to get component");
        assert_eq!(component.value, 1);

        // Index 1 should now have value 5 (from last position after first swap_remove)
        let component = pool.get::<TestComponent>(indices[1]).expect("Failed to get component");
        assert_eq!(component.value, 5);

        // Index 2 should now have value 4 (from last position after second swap_remove)
        let component = pool.get::<TestComponent>(indices[2]).expect("Failed to get component");
        assert_eq!(component.value, 4);
    }
    #[test]
    fn test_large_interleaved_add_remove() {
        let (_, mut pool) = setup_pool();


        let mut indices = Vec::with_capacity(1000);
        for i in 0..1000 {
            let idx = pool.add(TestComponent { value: i }).expect("Failed to add component");
            indices.push(idx);
        }


        let removed_indices = [250, 500, 750];

        let mut swapped_values = Vec::new();

        for &idx_pos in &removed_indices {
            let last_pos = indices.len() - 1 - swapped_values.len();
            let last_value = pool.get::<TestComponent>(indices[last_pos])
                .expect("Failed to get last component").value;
            swapped_values.push(last_value);

            pool.swap_remove(indices[idx_pos]);
        }

        let mut new_indices = Vec::new();
        for i in 0..removed_indices.len() {
            let idx = pool.add(TestComponent { value: 1000 + i as u32 })
                .expect("Failed to add new component");
            new_indices.push(idx);
        }

        assert_eq!(pool.count(), 1000);

        for (i, &idx_pos) in removed_indices.iter().enumerate() {
            let component = pool.get::<TestComponent>(indices[idx_pos])
                .expect("Failed to get component after swap_remove");
            assert_eq!(component.value, swapped_values[i],
                       "Component at position {} should have value {} after swap_remove",
                       idx_pos, swapped_values[i]);
        }

        for (i, &idx) in new_indices.iter().enumerate() {
            let component = pool.get::<TestComponent>(idx)
                .expect("Failed to get new component");
            assert_eq!(component.value, 1000 + i as u32,
                       "New component should have correct value");
        }

        for (i, &idx_pos) in removed_indices.iter().enumerate() {
            let component1 = pool.get::<TestComponent>(indices[idx_pos])
                .expect("Failed to get component at old index");
            let component2 = pool.get::<TestComponent>(new_indices[i])
                .expect("Failed to get component at new index");

            assert_ne!(component1.value, component2.value, "Component values should be different for old and new positions");
        }

        for i in 0..1000 {
            if removed_indices.contains(&i) {
                continue;
            }

            let last_positions = (1000 - removed_indices.len()..1000).collect::<Vec<_>>();
            if last_positions.contains(&i) {
                continue;
            }

            if let Some(component) = pool.get::<TestComponent>(indices[i]) {
                assert_eq!(component.value, i as u32,
                           "Original component at {} should maintain its value {}", i, i);
            } else {
                panic!("Could not get component at index {}", indices[i]);
            }
        }
    }
    #[test]
    fn test_interleaved_add_remove() {
        let (_, mut pool) = setup_pool();

        // Add components
        let idx1 = pool.add(TestComponent { value: 1 }).expect("Failed to add component");
        let _idx2 = pool.add(TestComponent { value: 2 }).expect("Failed to add component");

        // Remove the first component - это переместит value=2 на позицию idx1
        pool.swap_remove(idx1);

        // Add more components - новый компонент будет на позиции 1
        let idx3 = pool.add(TestComponent { value: 3 }).expect("Failed to add component");

        // Verify values are as expected
        assert_eq!(pool.count(), 2);

        // После swap_remove, value=2 должен быть на позиции idx1
        let component = pool.get::<TestComponent>(idx1).expect("Failed to get component at idx1");
        assert_eq!(component.value, 2, "Component at idx1 should have value 2 (moved during swap_remove)");

        // Новый компонент с value=3 должен быть на позиции idx3
        let component1= pool.get::<TestComponent>(idx3).expect("Failed to get component at idx3");
        assert_eq!(component1.value, 3);
        let component2= pool.get::<TestComponent>(_idx2).expect("Failed to get component at idx3");
        assert_eq!(component1.value, 3);

        // Старый idx2 теперь равен id3
        assert_eq!(component1, component2);
    }
}