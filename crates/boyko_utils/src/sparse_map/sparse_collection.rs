use std::ops::{Index, IndexMut};

pub trait SparseCollection<K, V>: Index<K, Output = V> + IndexMut<K, Output = V> {
    /// Returns the number of elements in the collection
    fn len(&self) -> usize;

    /// Returns the total capacity of the sparse array
    fn sparse_len(&self) -> usize;
}