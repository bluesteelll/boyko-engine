use std::cmp::Ordering;

/// Manages free chunks for optimal cache locality and memory reuse
///
/// Uses a sorted vector to efficiently track and retrieve chunks in
/// order of index value (highest indices first for better cache locality).
pub struct FreeChunkMaster {
    /// Vector of chunk indices, maintained in descending order
    /// (highest indices first for optimal cache locality)
    indices: Vec<usize>,

    /// Count of currently free chunks (same as indices.len() but cached for performance)
    count: usize,
}

impl FreeChunkMaster {
    /// Creates a new free chunk master with default capacity
    pub fn new() -> Self {
        Self {
            indices: Vec::new(),
            count: 0,
        }
    }

    /// Creates a free chunk master with the specified capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            indices: Vec::with_capacity(capacity),
            count: 0,
        }
    }

    /// Adds a chunk to the free chunk pool
    ///
    /// Uses binary search to maintain descending order and prevent duplicates.
    #[inline]
    pub fn add_chunk(&mut self, chunk_index: usize) {
        // Binary search to find insertion point or check for duplicate
        match self.binary_search(chunk_index) {
            Ok(_) => return, // Already exists
            Err(insert_at) => {
                // Insert at the correct position to maintain descending order
                self.indices.insert(insert_at, chunk_index);
                self.count += 1;
            }
        }
    }

    /// Gets the best chunk for reuse (highest index for cache locality)
    ///
    /// Since the indices are maintained in descending order, this is just
    /// removing the first element from the vector.
    #[inline]
    pub fn get_best_chunk(&mut self) -> Option<usize> {
        if self.indices.is_empty() {
            return None;
        }

        // Remove and return the first (highest) index
        let index = self.indices.remove(0);
        self.count -= 1;
        Some(index)
    }

    /// Checks if a chunk index is already in the free list
    ///
    /// Uses binary search for O(log n) lookups.
    #[inline]
    pub fn contains(&self, chunk_index: usize) -> bool {
        self.binary_search(chunk_index).is_ok()
    }

    /// Gets the current number of free chunks
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Checks if there are no free chunks
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Binary search for an index in the sorted indices vector
    ///
    /// Returns Ok(position) if found, Err(insert_position) if not found.
    /// Since we maintain indices in descending order, we need to invert
    /// the comparison to get the correct search behavior.
    #[inline]
    fn binary_search(&self, chunk_index: usize) -> Result<usize, usize> {
        self.indices.binary_search_by(|&index| {
            // Reverse comparison for descending order
            index.cmp(&chunk_index).reverse()
        })
    }

    /// Gets the chunks to remove during compaction
    ///
    /// Returns a list of the lowest-indexed chunks, keeping the
    /// specified number of highest-indexed chunks.
    pub fn get_chunks_to_remove(&self, keep_count: usize) -> Vec<usize> {
        if self.count <= keep_count {
            return Vec::new();
        }

        // Since indices are already in descending order,
        // we just need to take the last (count - keep_count) elements
        let start_idx = keep_count;
        self.indices[start_idx..].to_vec()
    }

    /// Removes specific chunks from the free chunk master
    ///
    /// Updates the internal data structure to exclude the specified chunks.
    pub fn remove_chunks(&mut self, chunks_to_remove: &[usize]) {
        if chunks_to_remove.is_empty() {
            return;
        }

        // For small removal sets, linear approach is more efficient
        if chunks_to_remove.len() <= 8 {
            for &chunk_index in chunks_to_remove {
                if let Ok(position) = self.binary_search(chunk_index) {
                    self.indices.remove(position);
                    self.count -= 1;
                }
            }
            return;
        }

        // For larger removal sets, it's more efficient to create a filtered vector
        let removal_set: Vec<usize> = chunks_to_remove.to_vec();
        let original_len = self.indices.len();

        // Filter in-place to keep only indices not in the removal set
        self.indices.retain(|&index| !removal_set.contains(&index));

        // Update count based on how many were actually removed
        self.count = self.indices.len();
    }

    /// Clears all free chunks
    pub fn clear(&mut self) {
        self.indices.clear();
        self.count = 0;
    }
}

impl Clone for FreeChunkMaster {
    fn clone(&self) -> Self {
        Self {
            indices: self.indices.clone(),
            count: self.count,
        }
    }
}

