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

impl<U> Default for SparseMap<U> {
    fn default() -> Self {
        Self::new()
    }
}

impl<U> SparseMap<U> {
    /// Creates a new empty SparseMap
    #[inline]
    pub fn new() -> Self {
        Self {
            sparse: Vec::new(),
            dense: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Creates a SparseMap with pre-allocated capacity
    #[inline]
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

    /// Removes an element by index and returns its value.
    /// O(1): moves the removed value out and swaps the last dense element into
    /// its slot (no clone), mirroring [`Vec::swap_remove`].
    #[inline]
    pub fn swap_remove(&mut self, index: usize) -> Option<U> {
        if index >= self.sparse.len() {
            return None;
        }

        let dense_idx = self.sparse[index].take()?;

        // Move the removed value out; `Vec::swap_remove` fills the hole with the
        // last element (a move, not a clone) and keeps `dense`/`indices` in lockstep.
        // Its return value is the REMOVED element (discarded for `indices` — the
        // removed external index was already cleared from `sparse` above).
        let value = self.dense.swap_remove(dense_idx);
        self.indices.swap_remove(dense_idx);

        // If an element was relocated into the freed slot (i.e. the removed one
        // was not the last), its external index now sits at `indices[dense_idx]`;
        // repoint its sparse entry at the new dense position. When the removed
        // element WAS the last, `swap_remove` merely popped it and
        // `dense_idx == self.dense.len()`, so no fix-up is needed.
        if dense_idx < self.dense.len() {
            let moved_entity_index = self.indices[dense_idx];
            debug_assert!(
                moved_entity_index < self.sparse.len(),
                "invariant: a live dense element's external index is always within sparse",
            );
            self.sparse[moved_entity_index] = Some(dense_idx);
        }

        Some(value)
    }
    /// Checks if an element exists at the specified index
    #[inline]
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
    #[inline]
    pub fn len(&self) -> usize {
        self.dense.len()
    }

    /// Checks if the collection is empty
    #[inline]
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

    /// Returns a slice of all currently-occupied external indices in dense order
    /// (the order they were inserted, modified by swap_remove rearrangement).
    ///
    /// Cost: O(1) borrow, no allocation. Iteration is O(active_count), not
    /// O(capacity) — this is the whole point of a sparse set's dense array.
    #[inline]
    pub fn active_indices(&self) -> &[usize] {
        &self.indices
    }

    /// Returns an iterator over the values in dense order.
    ///
    /// Cost: O(active_count) total, no allocation. Pairs with [`Self::active_indices`]
    /// if the caller needs both the external index and the value.
    #[inline]
    pub fn iter_dense(&self) -> std::slice::Iter<'_, U> {
        self.dense.iter()
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

#[cfg(test)]
mod tests {
    // Test oracle model: `HashSet` is the order-agnostic REFERENCE set that
    // `SparseMap::active_indices()` (whose dense order is insertion order) is
    // differentially compared against. `#[cfg(test)]` — compiled out of every
    // shipping build; the map itself never touches a std collection.
    #![allow(clippy::disallowed_types)]

    use super::*;
    use std::collections::HashSet;

    #[test]
    fn t_active_indices_after_inserts() {
        let mut map: SparseMap<&str> = SparseMap::new();
        map.insert(10, "a");
        map.insert(20, "b");
        map.insert(30, "c");

        // dense order is insertion order; collect into a set to ignore order
        let got: HashSet<usize> = map.active_indices().iter().copied().collect();
        let expected: HashSet<usize> = [10, 20, 30].into_iter().collect();
        assert_eq!(got, expected);
        assert_eq!(map.active_indices().len(), 3);
    }

    #[test]
    fn t_active_indices_after_swap_remove() {
        let mut map: SparseMap<&str> = SparseMap::new();
        map.insert(10, "a");
        map.insert(20, "b");
        map.insert(30, "c");

        // remove the middle external index
        map.swap_remove(20);

        assert_eq!(map.active_indices().len(), 2);
        let remaining: HashSet<usize> = map.active_indices().iter().copied().collect();
        assert!(remaining.contains(&10));
        assert!(remaining.contains(&30));
        assert!(!remaining.contains(&20));
    }

    #[test]
    fn t_iter_dense_matches_active_indices_count() {
        let mut map: SparseMap<u32> = SparseMap::new();
        for i in 0..8usize {
            map.insert(i * 3, i as u32); // non-contiguous external indices
        }

        assert_eq!(map.iter_dense().count(), map.active_indices().len());
        assert_eq!(map.iter_dense().count(), map.len());
    }

    #[test]
    fn t_swap_remove_last_element() {
        let mut map: SparseMap<&str> = SparseMap::new();
        map.insert(10, "a");
        map.insert(20, "b");
        map.insert(30, "c");

        // 30 was inserted last, so it is the last dense element.
        assert_eq!(map.swap_remove(30), Some("c"));
        assert!(!map.contains(30));
        assert_eq!(map.get(10), Some(&"a"));
        assert_eq!(map.get(20), Some(&"b"));
        assert_eq!(map.len(), 2);
        assert!(map.validate());
    }

    #[test]
    fn t_swap_remove_middle_relocates_swapped_element() {
        let mut map: SparseMap<&str> = SparseMap::new();
        map.insert(10, "a"); // dense 0
        map.insert(20, "b"); // dense 1
        map.insert(30, "c"); // dense 2 (last)

        // Removing the middle external index must swap the last element ("c" at 30)
        // into the freed dense slot and keep it reachable at its new dense index.
        assert_eq!(map.swap_remove(20), Some("b"));
        assert!(!map.contains(20));
        assert_eq!(map.get(30), Some(&"c")); // relocated, still reachable
        assert_eq!(map.get(10), Some(&"a"));
        assert_eq!(map.len(), 2);
        assert!(map.validate());
    }

    #[test]
    fn t_swap_remove_then_reinsert() {
        let mut map: SparseMap<u32> = SparseMap::new();
        map.insert(1, 100);
        map.insert(2, 200);
        map.insert(3, 300);

        assert_eq!(map.swap_remove(2), Some(200));
        assert!(!map.contains(2));

        // Reinserting a previously-removed index takes a fresh dense slot.
        assert_eq!(map.insert(2, 222), None);
        assert_eq!(map.get(2), Some(&222));
        assert_eq!(map.get(1), Some(&100));
        assert_eq!(map.get(3), Some(&300));
        assert_eq!(map.len(), 3);
        assert!(map.validate());
    }

    #[test]
    fn t_swap_remove_absent_index_returns_none() {
        let mut map: SparseMap<u32> = SparseMap::new();
        map.insert(5, 55);

        assert_eq!(map.swap_remove(999), None); // beyond sparse len
        assert_eq!(map.swap_remove(0), None); // in range but unoccupied
        assert_eq!(map.len(), 1);
        assert!(map.validate());
    }

    #[test]
    fn t_swap_remove_does_not_require_clone() {
        // A non-Clone value type must compile and round-trip through swap_remove,
        // proving the removed `U: Clone` bound was gratuitous.
        struct NoClone(u32);

        let mut map: SparseMap<NoClone> = SparseMap::new();
        map.insert(0, NoClone(1));
        map.insert(1, NoClone(2));

        let removed = map.swap_remove(0).expect("index 0 was inserted");
        assert_eq!(removed.0, 1);
        assert_eq!(map.get(1).map(|v| v.0), Some(2));
        assert!(map.validate());
    }
}
