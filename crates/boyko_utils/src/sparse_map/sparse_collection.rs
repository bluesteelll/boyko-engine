use std::ops::{Index, IndexMut};

/// Shared interface for sparse collection types.
///
/// Implemented by `SparseSlotMap`. Kept as an extension point for future
/// containers; the module is private so external consumers cannot depend on it.
#[allow(dead_code)]
pub trait SparseCollection<K, V>: Index<K, Output = V> + IndexMut<K, Output = V> {
    /// Returns the number of elements in the collection
    fn len(&self) -> usize;

    /// Returns the total capacity of the sparse array
    fn sparse_len(&self) -> usize;
}
