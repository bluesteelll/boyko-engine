use std::ops::{Index, IndexMut};

/// High-performance sparse set implementation
/// Provides O(1) insertion, removal, and lookup with optimal cache locality
pub struct SparseMap<U> {
    // Maps external indices to dense array indices
    sparse: Vec<Option<usize>>,

    // Dense storage for values
    dense: Vec<U>,

    // Reverse mapping: indices for each element in dense array
    indices: Vec<usize>,
}

impl<U: Clone> Default for SparseMap<U> {
    fn default() -> Self {
        Self::new()
    }
}

impl<U: Clone> SparseMap<U> {
    /// Creates a new empty SparseMap
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            sparse: Vec::new(),
            dense: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Creates a SparseMap with pre-allocated capacity
    #[inline(always)]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            sparse: Vec::with_capacity(capacity),
            dense: Vec::with_capacity(capacity),
            indices: Vec::with_capacity(capacity),
        }
    }

    /// Inserts a value at the specified index
    /// Returns the previous value if one existed
    #[inline]
    pub fn insert(&mut self, index: usize, value: U) -> Option<U> {
        // Ensure sparse array is large enough
        if index >= self.sparse.len() {
            self.sparse.resize(index + 1, None);
        }

        match self.sparse[index] {
            Some(dense_idx) => {
                // Replace existing value
                let old = std::mem::replace(&mut self.dense[dense_idx], value);
                Some(old)
            },
            None => {
                // Insert new value
                let dense_idx = self.dense.len();
                self.dense.push(value);
                self.indices.push(index);
                self.sparse[index] = Some(dense_idx);
                None
            }
        }
    }

    /// Removes an element by index and returns its value
    /// Uses swap_remove for O(1) removal time
    #[inline]
    pub fn swap_remove(&mut self, index: usize) -> Option<U> {
        if index >= self.sparse.len() {
            return None;
        }

        let dense_idx_opt = self.sparse[index].take();

        if let Some(dense_idx) = dense_idx_opt {
            // Fast removal by swapping with the last element
            let last_idx = self.dense.len() - 1;

            let value = if dense_idx == last_idx {
                // Last element, simply remove without swapping
                let value = self.dense.pop().unwrap();
                self.indices.pop();
                value
            } else {
                // Get the value being removed
                let value = self.dense[dense_idx].clone();

                // Get the last element and its index
                let last_element = self.dense.pop().unwrap();
                let moved_entity_index = self.indices.pop().unwrap();

                // Put the last element in place of the removed element
                self.dense[dense_idx] = last_element;
                self.indices[dense_idx] = moved_entity_index;

                // Update the sparse map for the moved entity (the one that was previously last)
                if moved_entity_index < self.sparse.len() {
                    self.sparse[moved_entity_index] = Some(dense_idx);
                }

                value
            };

            Some(value)
        } else {
            None
        }
    }
    /// Checks if an element exists at the specified index
    #[inline(always)]
    pub fn contains(&self, index: usize) -> bool {
        index < self.sparse.len() && self.sparse[index].is_some()
    }

    /// Returns a reference to the value at the specified index
    #[inline]
    pub fn get(&self, index: usize) -> Option<&U> {
        if index >= self.sparse.len() {
            return None;
        }

        match self.sparse[index] {
            Some(dense_idx) if dense_idx < self.dense.len() => Some(&self.dense[dense_idx]),
            _ => None
        }
    }

    /// Returns a mutable reference to the value at the specified index
    #[inline]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut U> {
        if index >= self.sparse.len() {
            return None;
        }

        match self.sparse[index] {
            Some(dense_idx) if dense_idx < self.dense.len() => Some(&mut self.dense[dense_idx]),
            _ => None
        }
    }

    /// Returns the number of elements in the collection
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.dense.len()
    }

    /// Checks if the collection is empty
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    /// Clears the collection, removing all elements
    #[inline]
    pub fn clear(&mut self) {
        self.sparse.iter_mut().for_each(|v| *v = None);
        self.dense.clear();
        self.indices.clear();
    }

    /// Validates internal consistency - only used for debugging
    pub fn validate(&self) -> bool {
        if self.dense.len() != self.indices.len() {
            return false;
        }

        // Check that all sparse entries point to valid dense indices
        for (sparse_idx, dense_idx_opt) in self.sparse.iter().enumerate() {
            if let Some(dense_idx) = dense_idx_opt {
                // Dense index should be in bounds
                if *dense_idx >= self.dense.len() {
                    return false;
                }

                // The indices array should point back to this sparse index
                if self.indices[*dense_idx] != sparse_idx {
                    return false;
                }
            }
        }

        // Check that all indices point to valid sparse entries
        for (dense_idx, sparse_idx) in self.indices.iter().enumerate() {
            // Sparse index should be in bounds
            if *sparse_idx >= self.sparse.len() {
                return false;
            }

            // The sparse array should point back to this dense index
            match self.sparse[*sparse_idx] {
                Some(idx) if idx == dense_idx => {}, // Correct
                _ => return false // Incorrect
            }
        }

        true
    }
}

impl<U: Clone> Index<usize> for SparseMap<U> {
    type Output = U;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("Index not found in SparseMap")
    }
}

impl<U: Clone> IndexMut<usize> for SparseMap<U> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_mut(index).expect("Index not found in SparseMap")
    }
}
