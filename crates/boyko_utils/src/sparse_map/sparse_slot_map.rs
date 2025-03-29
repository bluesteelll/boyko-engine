use std::ops::{Index, IndexMut};
use crate::identifiers::slot::Slot;
use crate::identifiers::primitives::Generation;
use super::sparse_collection::SparseCollection;

/// High-performance sparse set implementation with generation tracking
/// Uses Slot<T> directly for a clean and efficient design
pub struct SparseSlotMap<T: From<usize> + Into<usize>, U> {
    sparse: Vec<Option<Slot<T>>>,

    dense: Vec<U>,

    // Reverse mapping: external indices for each element in dense
    indices: Vec<T>,
}

impl<T, U> SparseSlotMap<T, U>
where
    T: Copy + Into<usize> + From<usize> + Eq
{
    /// Creates a new empty SparseSlotMap
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            sparse: Vec::new(),
            dense: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Creates a SparseSlotMap with pre-allocated capacity
    #[inline(always)]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            sparse: Vec::with_capacity(capacity),
            dense: Vec::with_capacity(capacity),
            indices: Vec::with_capacity(capacity),
        }
    }

    /// Creates a new slot for a given index with generation 0
    /// This should be used for initial slot creation
    #[inline(always)]
    pub fn create_slot(&self, index: T) -> Slot<T> {
        Slot::new(index, 0)
    }

    /// Inserts a value using the provided slot
    /// Returns the previous value if the slot with matching generation existed
    #[inline]
    pub fn insert(&mut self, slot: Slot<T>, value: U) -> Option<U> {
        let idx: usize = slot.index().into();
        let generation = slot.generation();

        // Ensure sparse array is large enough
        if idx >= self.sparse.len() {
            self.sparse.resize(idx + 1, None);
        }

        match &self.sparse[idx] {
            Some(stored_slot) if stored_slot.generation() == generation => {
                // Replace existing value, generations match
                let dense_idx = stored_slot.index().into();
                let old = std::mem::replace(&mut self.dense[dense_idx], value);
                Some(old)
            },
            _ => {
                // Insert new value with provided generation
                let dense_idx = self.dense.len();
                self.dense.push(value);
                self.indices.push(slot.index());

                // Store a slot with dense index and the original generation
                self.sparse[idx] = Some(Slot::new(T::from(dense_idx), generation));
                None
            }
        }
    }

    /// Removes an element by slot and returns its value
    /// Only succeeds if the generation matches to prevent ABA problems
    #[inline]
    pub fn remove(&mut self, slot: Slot<T>) -> Option<U> {
        let idx: usize = slot.index().into();
        let generation = slot.generation();

        if idx >= self.sparse.len() {
            return None;
        }

        if let Some(stored_slot) = &self.sparse[idx] {
            if stored_slot.generation() != generation {
                return None; // Generation mismatch - stale reference
            }

            let dense_idx: usize = stored_slot.index().into();

            // Increment generation to prevent ABA problem
            let new_generation = generation.wrapping_add(1);

            // Remove entry from sparse array
            self.sparse[idx] = None;

            // Remove from dense with swap removal strategy
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

                if swapped_idx < self.sparse.len() {
                    if let Some(swapped_slot) = &self.sparse[swapped_idx] {
                        // Create a new slot with updated dense index but same generation
                        self.sparse[swapped_idx] = Some(Slot::new(
                            T::from(dense_idx),
                            swapped_slot.generation()
                        ));
                    }
                }

                value
            };

            return Some(value);
        }

        None
    }

    /// Checks if an element exists with the specified slot, including generation verification
    #[inline(always)]
    pub fn contains(&self, slot: Slot<T>) -> bool {
        let idx: usize = slot.index().into();

        idx < self.sparse.len() &&
            self.sparse[idx].as_ref().map_or(false, |stored_slot|
                stored_slot.generation() == slot.generation()
            )
    }

    /// Returns a reference to the value for the specified slot
    #[inline]
    pub fn get(&self, slot: Slot<T>) -> Option<&U> {
        let idx: usize = slot.index().into();

        if idx >= self.sparse.len() {
            return None;
        }

        match &self.sparse[idx] {
            Some(stored_slot) if stored_slot.generation() == slot.generation() => {
                let dense_idx: usize = stored_slot.index().into();
                Some(&self.dense[dense_idx])
            },
            _ => None, // Generation mismatch or empty slot
        }
    }

    /// Returns a mutable reference to the value for the specified slot
    #[inline]
    pub fn get_mut(&mut self, slot: Slot<T>) -> Option<&mut U> {
        let idx: usize = slot.index().into();

        if idx >= self.sparse.len() {
            return None;
        }

        match &self.sparse[idx] {
            Some(stored_slot) if stored_slot.generation() == slot.generation() => {
                let dense_idx: usize = stored_slot.index().into();
                Some(&mut self.dense[dense_idx])
            },
            _ => None, // Generation mismatch or empty slot
        }
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

impl<T, U> Index<Slot<T>> for SparseSlotMap<T, U>
where
    T: Copy + Into<usize> + From<usize> + Eq
{
    type Output = U;

    fn index(&self, slot: Slot<T>) -> &Self::Output {
        self.get(slot).expect("Slot not found or generation mismatch")
    }
}

impl<T, U> IndexMut<Slot<T>> for SparseSlotMap<T, U>
where
    T: Copy + Into<usize> + From<usize> + Eq
{
    fn index_mut(&mut self, slot: Slot<T>) -> &mut Self::Output {
        self.get_mut(slot).expect("Slot not found or generation mismatch")
    }
}

impl<T, U> SparseCollection<Slot<T>, U> for SparseSlotMap<T, U>
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