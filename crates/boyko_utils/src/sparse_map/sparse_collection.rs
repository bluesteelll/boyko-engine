use std::ops::{Index, IndexMut};

pub trait SparseCollection<T: Sized, U>: Index<T> + IndexMut<T> {
    fn len(&self) -> usize;
    fn sparse_len(&self) -> usize;

}
