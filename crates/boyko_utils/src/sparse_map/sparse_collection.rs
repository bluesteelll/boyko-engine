use std::ops::{Index, IndexMut};

pub trait SparseCollection<T: Sized, U>: Index<T> + IndexMut<T> {
    fn size() -> usize;
    fn sparse_size() -> usize;

}
