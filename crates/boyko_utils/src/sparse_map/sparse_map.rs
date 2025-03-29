use std::ops::{Index, IndexMut};
use super::sparse_collection::SparseCollection;

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

impl<U> SparseMap<U> {
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
    pub fn remove(&mut self, index: usize) -> Option<U> {
        if index >= self.sparse.len() {
            return None;
        }

        self.sparse[index].take().map(|dense_idx| {
            // Fast removal by swapping with the last element
            let last_idx = self.dense.len() - 1;

            let value = if dense_idx == last_idx {
                // Last element, simply remove
                let value = self.dense.pop().unwrap();
                self.indices.pop();
                value
            } else {
                // Swap with last and remove
                let value = self.dense.swap_remove(dense_idx);

                // Update mapping for moved element
                let swapped_index = self.indices.swap_remove(dense_idx);
                self.sparse[swapped_index] = Some(dense_idx);

                value
            };

            value
        })
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

        self.sparse[index].map(|dense_idx| &self.dense[dense_idx])
    }

    /// Returns a mutable reference to the value at the specified index
    #[inline]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut U> {
        if index >= self.sparse.len() {
            return None;
        }

        self.sparse[index].map(move |dense_idx| &mut self.dense[dense_idx])
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
}

impl<U> Index<usize> for SparseMap<U> {
    type Output = U;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("Index not found in SparseMap")
    }
}

impl<U> IndexMut<usize> for SparseMap<U> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_mut(index).expect("Index not found in SparseMap")
    }
}

impl<U> SparseCollection<usize, U> for SparseMap<U> {
    fn len(&self) -> usize {
        self.dense.len()
    }

    fn sparse_len(&self) -> usize {
        self.sparse.len()
    }
}