
use std::ops::{Index, IndexMut};
use super::sparse_collection::SparseCollection;

/// High-performance sparse set implementation
/// Provides O(1) insertion, removal, and lookup with optimal cache locality
pub struct SparseMap<T: Sized + Copy + From<usize> + Into<usize>, U> {
    // Maps external indices to dense array indices
    sparse: Vec<Option<T>>,

    dense: Vec<U>,

    // Reverse mapping: indices for each element in dense array
    // Required for efficient O(1) removal
    indices: Vec<T>,
}

impl<T, U> SparseMap<T, U>
where
    T: Copy + Into<usize> + From<usize> + Eq
{
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            sparse: Vec::new(),
            dense: Vec::new(),
            indices: Vec::new(),
        }
    }

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
    pub fn insert(&mut self, index: T, value: U) -> Option<U> {
        let idx: usize = index.into();

        // Ensure sparse array is large enough
        if idx >= self.sparse.len() {
            self.sparse.resize(idx + 1, None);
        }

        match self.sparse[idx] {
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
                self.sparse[idx] = Some(dense_idx);
                None
            }
        }
    }

    /// Removes an element by index and returns its value
    /// Uses swap_remove for O(1) removal time
    #[inline]
    pub fn swap_remove(&mut self, index: T) -> Option<U> {
        let idx: usize = index.into();

        if idx >= self.sparse.len() {
            return None;
        }

        self.sparse[idx].take().map(|dense_idx| {
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
                let swapped_idx: usize = swapped_index.into();
                self.sparse[swapped_idx] = Some(dense_idx);

                value
            };

            value
        })
    }

    #[inline(always)]
    pub fn contains(&self, index: T) -> bool {
        let idx: usize = index.into();
        idx < self.sparse.len() && self.sparse[idx].is_some()
    }

    #[inline]
    pub fn get(&self, index: T) -> Option<&U> {
        let idx: usize = index.into();
        if idx >= self.sparse.len() {
            return None;
        }

        self.sparse[idx].map(|dense_idx| &self.dense[dense_idx])
    }

    #[inline]
    pub fn get_mut(&mut self, index: T) -> Option<&mut U> {
        let idx: usize = index.into();
        if idx >= self.sparse.len() {
            return None;
        }

        self.sparse[idx].map(move |dense_idx| &mut self.dense[dense_idx])
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

impl<T, U> Index<T> for SparseMap<T, U>
where
    T: Copy + Into<usize> + From<usize> + Eq
{
    type Output = U;

    fn index(&self, index: T) -> &Self::Output {
        self.get(index).expect("Index not found in SparseMap")
    }
}

impl<T, U> IndexMut<T> for SparseMap<T, U>
where
    T: Copy + Into<usize> + From<usize> + Eq
{
    fn index_mut(&mut self, index: T) -> &mut Self::Output {
        self.get_mut(index).expect("Index not found in SparseMap")
    }
}

impl<T, U> SparseCollection<T, U> for SparseMap<T, U>
where
    T: Copy + Into<usize> + From<usize> + Eq
{
    fn len(&self) -> usize {
        self.dense.len()
    }

    fn sparse_len(&self) -> usize {
        self.sparse.len()
    }
}