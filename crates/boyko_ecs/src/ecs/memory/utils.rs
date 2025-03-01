// crates/boyko_ecs/src/ecs/memory/utils.rs

/// Проверка, является ли число степенью двойки
///
/// # Arguments
///
/// * `n` - число для проверки
///
/// # Returns
///
/// `true` если число является степенью двойки, иначе `false`
#[inline(always)]
pub fn is_power_of_two(n: usize) -> bool {
    n != 0 && (n & (n - 1)) == 0
}

/// Fast alignment of a value to a power-of-2 boundary using bitwise operations
/// Optimized implementation with no branches
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
    debug_assert!(is_power_of_two(align), "Alignment must be a power of 2");
    (val + align - 1) & !(align - 1)
}

/// Align a value down to a power-of-2 boundary
///
/// # Arguments
///
/// * `val` - The value to align
/// * `align` - The alignment (must be a power of 2)
///
/// # Returns
///
/// The aligned value, which is always <= val
#[inline(always)]
pub fn align_down(val: usize, align: usize) -> usize {
    debug_assert!(is_power_of_two(align), "Alignment must be a power of 2");
    val & !(align - 1)
}

/// Check if a pointer is aligned to the specified alignment
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
    debug_assert!(is_power_of_two(align), "Alignment must be a power of 2");
    (ptr as usize & (align - 1)) == 0
}

/// Rounds up to the next power of 2
/// Uses bitwise operations for maximum efficiency
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

    if is_power_of_two(n) {
        return n;
    }

    1 << (usize::BITS - (n - 1).leading_zeros())
}

/// Calculate the thread workload distribution for a given number of items
/// This balances work evenly among threads without atomic operations
///
/// # Arguments
///
/// * `thread_id` - The ID of the thread (0-based)
/// * `thread_count` - The total number of threads
/// * `item_count` - The total number of items to process
///
/// # Returns
///
/// A tuple of (start_index, end_index) for the thread to process
#[inline(always)]
pub fn calculate_thread_workload(thread_id: usize, thread_count: usize, item_count: usize) -> (usize, usize) {
    if thread_count == 0 || thread_id >= thread_count || item_count == 0 {
        return (0, 0);
    }

    // Use a more balanced approach for thread workload
    // This handles uneven divisions better than the simple approach
    let base_items_per_thread = item_count / thread_count;
    let remainder = item_count % thread_count;

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

/// Calculate the SIMD-friendly chunk size for processing
/// Returns a size that is optimized for SIMD operations
///
/// # Arguments
///
/// * `element_size` - Size of each element in bytes
/// * `alignment` - Required alignment of elements
///
/// # Returns
///
/// Recommended number of elements to process together with SIMD
#[inline(always)]
pub fn calculate_simd_chunk_size(element_size: usize, alignment: usize) -> usize {
    // Target SIMD register sizes based on architecture
    #[cfg(target_arch = "x86_64")]
    const TARGET_SIMD_BYTES: usize = 32; // 256-bit AVX

    #[cfg(target_arch = "aarch64")]
    const TARGET_SIMD_BYTES: usize = 16; // 128-bit NEON

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    const TARGET_SIMD_BYTES: usize = 16; // Generic default

    if element_size == 0 {
        return 1;
    }

    // Calculate how many elements fit in a SIMD register
    let elements_per_simd = TARGET_SIMD_BYTES / element_size;

    // Ensure at least 1 element and adjust for alignment
    let result = elements_per_simd.max(1);

    // Round to power of 2 for more efficient calculation
    // but cap at reasonable limits based on element size
    let max_simd_width = match element_size {
        1..=4 => 8,   // Small elements: process up to 8 at once
        5..=8 => 4,   // Medium elements: process up to 4 at once
        9..=16 => 2,  // Large elements: process up to 2 at once
        _ => 1,       // Very large elements: process 1 at a time
    };

    next_power_of_2(result).min(max_simd_width)
}

/// Calculate the number of chunks needed to store a given number of components
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
    if components_per_chunk == 0 {
        return 0;
    }
    (component_count + components_per_chunk - 1) / components_per_chunk
}

/// Check if pointer is cache-line aligned
/// Useful for optimizing memory access patterns
///
/// # Arguments
///
/// * `ptr` - The pointer to check
/// * `cache_line_size` - Cache line size (usually 64 bytes)
///
/// # Returns
///
/// `true` if the pointer is cache-line aligned
#[inline(always)]
pub fn is_cache_aligned(ptr: *const u8, cache_line_size: usize) -> bool {
    debug_assert!(is_power_of_two(cache_line_size), "Cache line size must be a power of 2");
    (ptr as usize & (cache_line_size - 1)) == 0
}

/// Determine if a memory range crosses a cache line boundary
/// This is important for avoiding false sharing
///
/// # Arguments
///
/// * `ptr` - Start pointer of the memory range
/// * `size` - Size of the memory range in bytes
/// * `cache_line_size` - Cache line size (usually 64 bytes)
///
/// # Returns
///
/// `true` if the memory range crosses a cache line boundary
#[inline(always)]
pub fn crosses_cache_line(ptr: *const u8, size: usize, cache_line_size: usize) -> bool {
    let start_line = (ptr as usize) / cache_line_size;
    let end_line = ((ptr as usize) + size - 1) / cache_line_size;
    start_line != end_line
}

/// Calculate padding needed to avoid false sharing
/// Adds padding to ensure data doesn't share a cache line with other data
///
/// # Arguments
///
/// * `ptr` - Pointer to the data
/// * `size` - Size of the data in bytes
/// * `cache_line_size` - Cache line size (usually 64 bytes)
///
/// # Returns
///
/// Padding in bytes needed to align to the next cache line
#[inline(always)]
pub fn calculate_false_sharing_padding(ptr: *const u8, size: usize, cache_line_size: usize) -> usize {
    debug_assert!(is_power_of_two(cache_line_size), "Cache line size must be a power of 2");

    let end_addr = (ptr as usize) + size;
    let next_cache_line = ((end_addr + cache_line_size - 1) / cache_line_size) * cache_line_size;

    next_cache_line - end_addr
}

/// Find the first set bit in a u64
/// This is useful for bitmap operations
///
/// # Arguments
///
/// * `bitmap` - The bitmap to search
///
/// # Returns
///
/// The index of the first set bit, or 64 if no bits are set
#[inline(always)]
pub fn find_first_set_bit(bitmap: u64) -> usize {
    if bitmap == 0 {
        return 64;
    }
    bitmap.trailing_zeros() as usize
}

/// Count the number of set bits in a u64
/// This is useful for bitmap operations
///
/// # Arguments
///
/// * `bitmap` - The bitmap to count
///
/// # Returns
///
/// The number of set bits
#[inline(always)]
pub fn count_set_bits(bitmap: u64) -> usize {
    bitmap.count_ones() as usize
}

/// Set a bit in a bitmap
///
/// # Arguments
///
/// * `bitmap` - The bitmap to modify
/// * `bit_index` - The index of the bit to set
///
/// # Returns
///
/// The modified bitmap
#[inline(always)]
pub fn set_bit(bitmap: u64, bit_index: usize) -> u64 {
    debug_assert!(bit_index < 64, "Bit index out of bounds: {} >= 64", bit_index);
    bitmap | (1u64 << bit_index)
}

/// Clear a bit in a bitmap
///
/// # Arguments
///
/// * `bitmap` - The bitmap to modify
/// * `bit_index` - The index of the bit to clear
///
/// # Returns
///
/// The modified bitmap
#[inline(always)]
pub fn clear_bit(bitmap: u64, bit_index: usize) -> u64 {
    debug_assert!(bit_index < 64, "Bit index out of bounds: {} >= 64", bit_index);
    bitmap & !(1u64 << bit_index)
}

/// Test if a bit is set in a bitmap
///
/// # Arguments
///
/// * `bitmap` - The bitmap to test
/// * `bit_index` - The index of the bit to test
///
/// # Returns
///
/// `true` if the bit is set, `false` otherwise
#[inline(always)]
pub fn test_bit(bitmap: u64, bit_index: usize) -> bool {
    debug_assert!(bit_index < 64, "Bit index out of bounds: {} >= 64", bit_index);
    (bitmap & (1u64 << bit_index)) != 0
}

/// Find contiguous free bits in a bitmap
/// Useful for finding space for multiple components
///
/// # Arguments
///
/// * `bitmap` - The bitmap to search (1=free, 0=used)
/// * `count` - The number of contiguous bits needed
///
/// # Returns
///
/// The starting index of the free bits, or None if not found
#[inline]
pub fn find_contiguous_bits(bitmap: u64, count: usize) -> Option<usize> {
    if count == 0 {
        return Some(0);
    }

    if count > 64 {
        return None;
    }

    if count == 1 {
        // Fast path for single bit
        let bit = find_first_set_bit(bitmap);
        if bit < 64 {
            return Some(bit);
        }
        return None;
    }

    // For contiguous bits, we need to check each possible starting position
    let mask = (1u64 << count) - 1;

    for i in 0..=64-count {
        // Shift the mask to the current position
        let shifted_mask = mask << i;

        // Check if all bits in the mask are free
        if (bitmap & shifted_mask) == shifted_mask {
            return Some(i);
        }
    }

    None
}