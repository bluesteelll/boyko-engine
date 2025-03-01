// crates/boyko_ecs/src/ecs/memory/utils.rs

/// Fast alignment of a value to a power-of-2 boundary using bitwise operations
/// This is an optimized version using direct bitwise manipulation
///
/// # Arguments
///
/// * `val` - The value to align
/// * `align` - The alignment (must be a power of 2)
///
/// # Returns
///
/// The aligned value, which is always >= val
#[inline(always)]
pub fn fast_align(val: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two(), "Alignment must be a power of 2");
    let align_mask = align - 1;
    (val + align_mask) & !align_mask
}

/// Helper function to align a value up to the specified alignment
/// Traditional implementation, kept for compatibility
///
/// # Arguments
///
/// * `val` - The value to align
/// * `align` - The alignment (must be a power of 2)
///
/// # Returns
///
/// The aligned value, which is always >= val
#[inline(always)]
pub fn align_up(val: usize, align: usize) -> usize {
    fast_align(val, align)
}

/// Checks if a pointer is aligned to the specified alignment
///
/// # Arguments
///
/// * `ptr` - The pointer to check
/// * `align` - The alignment (must be a power of 2)
///
/// # Returns
///
/// `true` if the pointer is aligned, `false` otherwise
#[inline(always)]
pub fn is_aligned(ptr: *const u8, align: usize) -> bool {
    debug_assert!(align.is_power_of_two(), "Alignment must be a power of 2");
    (ptr as usize & (align - 1)) == 0
}

/// Rounds up to the next power of 2
/// If the value is already a power of 2, returns the value unchanged
///
/// # Arguments
///
/// * `n` - The value to round up
///
/// # Returns
///
/// The next power of 2 >= n
#[inline(always)]
pub fn next_power_of_2(n: usize) -> usize {
    if n == 0 {
        return 1;
    }

    if n.is_power_of_two() {
        return n;
    }

    // Use the leading zeros count to find the next power of 2
    let leading_zeros = n.leading_zeros() as usize;
    let bits = std::mem::size_of::<usize>() * 8;

    1 << (bits - leading_zeros)
}

/// Calculates the number of chunks needed to store a given number of components
///
/// # Arguments
///
/// * `component_count` - The number of components to store
/// * `components_per_chunk` - The number of components per chunk
///
/// # Returns
///
/// The number of chunks needed
#[inline(always)]
pub fn calculate_chunk_count(component_count: usize, components_per_chunk: usize) -> usize {
    (component_count + components_per_chunk - 1) / components_per_chunk
}

/// Calculates the offset of a component in a chunk
///
/// # Arguments
///
/// * `component_idx` - The index of the component in the chunk
/// * `component_size` - The size of each component in bytes
/// * `alignment` - The alignment requirement of the component
///
/// # Returns
///
/// The byte offset of the component
#[inline(always)]
pub fn calculate_component_offset(component_idx: usize, component_size: usize, alignment: usize) -> usize {
    let aligned_size = fast_align(component_size, alignment);
    component_idx * aligned_size
}

/// Calculate the range of entities for a specific thread
/// Used for thread workload partitioning
///
/// # Arguments
///
/// * `thread_id` - The ID of the thread (0-based)
/// * `thread_count` - The total number of threads
/// * `entity_count` - The total number of entities to process
///
/// # Returns
///
/// A tuple of (start_index, end_index) for the thread to process
#[inline(always)]
pub fn calculate_thread_entity_range(thread_id: usize, thread_count: usize, entity_count: usize) -> (usize, usize) {
    if thread_count == 0 || entity_count == 0 {
        return (0, 0);
    }

    let entities_per_thread = (entity_count + thread_count - 1) / thread_count;
    let start = thread_id * entities_per_thread;

    if start >= entity_count {
        return (entity_count, entity_count);
    }

    let end = (start + entities_per_thread).min(entity_count);
    (start, end)
}