use std::alloc::Layout;
use std::any::TypeId;
use std::ptr::NonNull;

use boyko_ecs::ecs::core::component::Component;
use boyko_ecs::ecs::memory::arena::Arena;
use boyko_ecs::ecs::memory::component_pool::ComponentPool;
use boyko_macros::Component;

// Test components
#[derive(Component, Debug, PartialEq, Clone, Copy)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Component, Debug, PartialEq, Clone, Copy)]
struct Velocity {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Component, Debug, PartialEq, Clone, Copy)]
struct Health {
    value: i32,
}

#[derive(Component, Debug, PartialEq, Clone, Copy)]
struct TinyComponent {
    value: u8,
}

#[derive(Component, Debug, PartialEq, Clone, Copy)]
struct BigComponent {
    data: [f32; 100], // Large component
}

fn main() {
    println!("Running ComponentPool tests...");

    test_pool_creation();
    test_add_component();
    test_get_component();
    test_set_component();
    test_swap_remove();
    test_swap_remove_verification();
    test_chunk_operations();
    test_pool_full();
    test_component_type_safety();
    test_dirty_flags();
    test_stress_test();

    println!("All tests passed!");
}

fn test_pool_creation() {
    println!("Testing pool creation...");

    let arena = Arena::new();

    // Small pool
    let pool = ComponentPool::new::<Position>(&arena, 2, 10);
    assert_eq!(pool.count(), 0);
    assert_eq!(pool.capacity(), 20);
    assert_eq!(pool.chunks_count(), 2);

    // Default sizes pool
    let pool = ComponentPool::with_default_sizes::<Position>(&arena);
    assert_eq!(pool.type_id(), TypeId::of::<Position>());
    assert!(pool.component_layout() == Layout::new::<Position>());

    // Different component sizes should have different optimal capacities
    let tiny_pool = ComponentPool::with_default_sizes::<TinyComponent>(&arena);
    let big_pool = ComponentPool::with_default_sizes::<BigComponent>(&arena);
    assert!(tiny_pool.capacity() > big_pool.capacity());

    println!("Pool creation tests passed!");
}

fn test_add_component() {
    println!("Testing component addition...");

    let arena = Arena::new();
    let mut pool = ComponentPool::new::<Position>(&arena, 2, 10);

    // Add first component
    let index1 = pool.add(Position { x: 1.0, y: 2.0, z: 3.0 }).expect("Failed to add component");
    assert_eq!(index1, 0);
    assert_eq!(pool.count(), 1);

    // Add second component
    let index2 = pool.add(Position { x: 4.0, y: 5.0, z: 6.0 }).expect("Failed to add component");
    assert_eq!(index2, 1);
    assert_eq!(pool.count(), 2);

    // Raw add
    let pos = Position { x: 7.0, y: 8.0, z: 9.0 };
    let index3 = unsafe {
        pool.raw_add(&pos as *const Position as *const u8)
    }.expect("Failed to add raw component");
    assert_eq!(index3, 2);
    assert_eq!(pool.count(), 3);

    println!("Component addition tests passed!");
}

fn test_get_component() {
    println!("Testing component access...");

    let arena = Arena::new();
    let mut pool = ComponentPool::new::<Position>(&arena, 1, 10);

    // Add components
    let pos1 = Position { x: 1.0, y: 2.0, z: 3.0 };
    let pos2 = Position { x: 4.0, y: 5.0, z: 6.0 };

    let index1 = pool.add(pos1).unwrap();
    let index2 = pool.add(pos2).unwrap();

    // Get components
    let got_pos1 = pool.get::<Position>(index1).unwrap();
    let got_pos2 = pool.get::<Position>(index2).unwrap();

    assert_eq!(*got_pos1, pos1);
    assert_eq!(*got_pos2, pos2);

    // Get mutable components
    let got_pos1_mut = pool.get_mut::<Position>(index1).unwrap();
    got_pos1_mut.x = 10.0;

    // Verify the value was changed
    let updated_pos1 = pool.get::<Position>(index1).unwrap();
    assert_eq!(updated_pos1.x, 10.0);

    // Raw access
    let raw_ptr = pool.raw_get(index2).unwrap();
    unsafe {
        let pos_ptr = raw_ptr as *const Position;
        assert_eq!((*pos_ptr).x, 4.0);
    }

    // Raw mutable access
    let raw_mut_ptr = pool.raw_get_mut(index2).unwrap();
    unsafe {
        let pos_mut_ptr = raw_mut_ptr as *mut Position;
        (*pos_mut_ptr).y = 15.0;
    }

    // Verify the value was changed
    let updated_pos2 = pool.get::<Position>(index2).unwrap();
    assert_eq!(updated_pos2.y, 15.0);

    println!("Component access tests passed!");
}

fn test_set_component() {
    println!("Testing component setting...");

    let arena = Arena::new();
    let mut pool = ComponentPool::new::<Position>(&arena, 1, 10);

    // Add component
    let pos1 = Position { x: 1.0, y: 2.0, z: 3.0 };
    let index = pool.add(pos1).unwrap();

    // Set new value
    let new_pos = Position { x: 10.0, y: 20.0, z: 30.0 };
    let success = pool.set_component(index, new_pos);
    assert!(success);

    // Verify the value was changed
    let updated_pos = pool.get::<Position>(index).unwrap();
    assert_eq!(*updated_pos, new_pos);

    // Try to set component with invalid index
    let invalid_index = 100;
    let result = pool.set_component(invalid_index, new_pos);
    assert!(!result);

    // Type mismatch
    let mut velocity_pool = ComponentPool::new::<Velocity>(&arena, 1, 10);
    let vel_index = velocity_pool.add(Velocity { x: 1.0, y: 1.0, z: 1.0 }).unwrap();

    // This should fail due to type mismatch (trying to set a Position in a Velocity pool)
    assert!(!velocity_pool.set_component(vel_index, pos1));

    println!("Component setting tests passed!");
}

fn test_swap_remove() {
    println!("Testing swap_remove basic functionality...");

    let arena = Arena::new();
    let mut pool = ComponentPool::new::<Position>(&arena, 1, 10);

    // Add multiple components
    let pos1 = Position { x: 1.0, y: 2.0, z: 3.0 };
    let pos2 = Position { x: 4.0, y: 5.0, z: 6.0 };
    let pos3 = Position { x: 7.0, y: 8.0, z: 9.0 };

    let index1 = pool.add(pos1).unwrap();
    let index2 = pool.add(pos2).unwrap();
    let index3 = pool.add(pos3).unwrap();

    assert_eq!(pool.count(), 3);

    // Remove the middle component
    let success = pool.swap_remove(index2);
    assert!(success);
    assert_eq!(pool.count(), 2);

    // Verify the first component hasn't changed
    let first = pool.get::<Position>(index1).unwrap();
    assert_eq!(*first, pos1);

    // Verify that the last component (pos3) was moved to index2's position
    let moved = pool.get::<Position>(index2).unwrap();
    assert_eq!(*moved, pos3);

    // Try to access the removed component's original position - should be invalid
    let invalid = pool.get::<Position>(index3);
    assert!(invalid.is_none());

    // Remove again (now removing the last element)
    let success = pool.swap_remove(index1);
    assert!(success);
    assert_eq!(pool.count(), 1);

    // Removing an invalid index should fail
    let fail = pool.swap_remove(100);
    assert!(!fail);

    println!("Basic swap_remove tests passed!");
}

fn test_swap_remove_verification() {
    println!("Testing swap_remove with verification...");

    let arena = Arena::new();
    let mut pool = ComponentPool::new::<Position>(&arena, 1, 10);

    // Add many components to better test swap_remove
    let positions = vec![
        Position { x: 0.0, y: 0.0, z: 0.0 },
        Position { x: 1.0, y: 1.0, z: 1.0 },
        Position { x: 2.0, y: 2.0, z: 2.0 },
        Position { x: 3.0, y: 3.0, z: 3.0 },
        Position { x: 4.0, y: 4.0, z: 4.0 },
    ];

    let mut indices = Vec::new();
    for pos in &positions {
        indices.push(pool.add(*pos).unwrap());
    }

    assert_eq!(pool.count(), 5);

    // Remove from the middle
    pool.swap_remove(indices[2]);

    // Verify that element at index 4 moved to index 2
    let moved = pool.get::<Position>(indices[2]).unwrap();
    assert_eq!(*moved, positions[4]);

    // Count should be decremented
    assert_eq!(pool.count(), 4);

    // Remove the first element
    pool.swap_remove(indices[0]);

    // Verify that element at index 3 moved to index 0
    let moved = pool.get::<Position>(indices[0]).unwrap();
    assert_eq!(*moved, positions[3]);

    // Count should be decremented again
    assert_eq!(pool.count(), 3);

    // Add a new element
    let new_pos = Position { x: 5.0, y: 5.0, z: 5.0 };
    let new_index = pool.add(new_pos).unwrap();

    // Verify it was added at the end
    assert_eq!(new_index, 3);
    assert_eq!(pool.count(), 4);

    // Remove all elements
    while pool.count() > 0 {
        pool.swap_remove(0);
    }

    // Verify the pool is empty
    assert_eq!(pool.count(), 0);

    println!("swap_remove verification tests passed!");
}

fn test_chunk_operations() {
    println!("Testing chunk operations...");

    let arena = Arena::new();
    let mut pool = ComponentPool::new::<Position>(&arena, 2, 5);

    // Add components across multiple chunks
    for i in 0..8 {
        let x = i as f32;
        pool.add(Position { x, y: x, z: x }).unwrap();
    }

    // First chunk should contain indices 0-4
    let chunk0_comps = pool.chunk_components::<Position>(0).unwrap();
    assert_eq!(chunk0_comps.len(), 5);

    // Second chunk should contain indices 5-7
    let chunk1_comps = pool.chunk_components::<Position>(1).unwrap();
    assert_eq!(chunk1_comps.len(), 3);

    // Modify components in the first chunk
    let chunk0_comps_mut = pool.chunk_components_mut::<Position>(0).unwrap();
    for comp in chunk0_comps_mut {
        comp.x += 100.0;
    }

    // Verify modifications
    let component0 = pool.get::<Position>(0).unwrap();
    assert_eq!(component0.x, 100.0);

    // Test invalid chunk index
    let invalid_chunk = pool.chunk_components::<Position>(5);
    assert!(invalid_chunk.is_none());

    println!("Chunk operations tests passed!");
}

fn test_pool_full() {
    println!("Testing pool capacity limits...");

    let arena = Arena::new();
    let mut pool = ComponentPool::new::<Position>(&arena, 1, 3);

    // Fill the pool
    for i in 0..3 {
        let x = i as f32;
        pool.add(Position { x, y: x, z: x }).unwrap();
    }

    assert_eq!(pool.count(), 3);
    assert!(pool.is_full());
    assert_eq!(pool.remaining_capacity(), 0);

    // Try to add one more component
    let result = pool.add(Position { x: 10.0, y: 10.0, z: 10.0 });
    assert!(result.is_none());

    // Remove one component
    pool.swap_remove(1);
    assert_eq!(pool.count(), 2);
    assert!(!pool.is_full());
    assert_eq!(pool.remaining_capacity(), 1);

    // Now we should be able to add another component
    let result = pool.add(Position { x: 10.0, y: 10.0, z: 10.0 });
    assert!(result.is_some());
    assert_eq!(pool.count(), 3);

    println!("Pool capacity tests passed!");
}

fn test_component_type_safety() {
    println!("Testing component type safety...");

    let arena = Arena::new();
    let mut pool = ComponentPool::new::<Position>(&arena, 1, 10);

    // Add a Position component
    let index = pool.add(Position { x: 1.0, y: 2.0, z: 3.0 }).unwrap();

    // Try to get it as a Velocity (wrong type)
    let result = pool.get::<Velocity>(index);
    assert!(result.is_none());

    // Try to get it mutably as a Velocity (wrong type)
    let result = pool.get_mut::<Velocity>(index);
    assert!(result.is_none());

    // Try to set it as a Velocity (wrong type)
    let result = pool.set_component(index, Velocity { x: 1.0, y: 1.0, z: 1.0 });
    assert!(!result);

    println!("Component type safety tests passed!");
}

fn test_dirty_flags() {
    println!("Testing dirty flags...");

    let arena = Arena::new();
    let mut pool = ComponentPool::new::<Position>(&arena, 2, 5);

    // Add components to both chunks
    for i in 0..8 {
        let x = i as f32;
        pool.add(Position { x, y: x, z: x }).unwrap();
    }

    // Access components in the first chunk
    for i in 0..5 {
        let comp = pool.get_mut::<Position>(i).unwrap();
        comp.x += 1.0;
    }

    // The first chunk should be marked dirty, but not necessarily the second
    let chunk = unsafe {
        // This is just for testing - we need to access the private fields
        let chunks_ptr = &pool as *const ComponentPool as *mut ComponentPool;
        let chunks = &mut (*chunks_ptr).chunks;
        &chunks[0]
    };

    assert!(chunk.is_dirty());

    // Clear the dirty flag
    unsafe {
        let chunks_ptr = &pool as *const ComponentPool as *mut ComponentPool;
        let chunks = &mut (*chunks_ptr).chunks;
        chunks[0].clear_dirty_flag();
        assert!(!chunks[0].is_dirty());
    }

    println!("Dirty flag tests passed!");
}

fn test_stress_test() {
    println!("Running stress test...");

    let arena = Arena::new();
    let mut pool = ComponentPool::new::<TinyComponent>(&arena, 10, 1000);

    // Add many components
    let count = 9000;
    for i in 0..count {
        let value = (i % 255) as u8;
        pool.add(TinyComponent { value }).unwrap();
    }

    assert_eq!(pool.count(), count);

    // Randomly remove components
    let mut rng = rand::thread_rng();
    use rand::Rng;

    for _ in 0..1000 {
        let index = rng.gen_range(0..pool.count());
        pool.swap_remove(index);
    }

    assert_eq!(pool.count(), count - 1000);

    // Add more components
    for i in 0..500 {
        let value = (i % 255) as u8;
        pool.add(TinyComponent { value }).unwrap();
    }

    assert_eq!(pool.count(), count - 500);

    // Do random gets and sets
    for _ in 0..2000 {
        let index = rng.gen_range(0..pool.count());

        if rng.gen_bool(0.5) {
            // Get component
            let _ = pool.get::<TinyComponent>(index);
        } else {
            // Set component
            let value = rng.r#gen::<u8>();
            pool.set_component(index, TinyComponent { value });
        }
    }

    println!("Stress test passed!");
}