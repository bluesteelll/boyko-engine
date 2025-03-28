use std::ops::{Index, IndexMut};
use super::sparse_collection::SparseCollection;

pub struct SparseMap<T: Sized, U>{
    sparse: Vec<T>,
    dense: Vec<U>
}

impl<T, U> SparseMap<T, U> {
    todo!();
}

impl<T, U> Index<T> for SparseMap<T, U> {
    type Output = ();

    fn index(&self, index: T) -> &Self::Output {
        todo!()
    }
}

impl<T, U> IndexMut<T> for SparseMap<T, U> {
    fn index_mut(&mut self, index: T) -> &mut Self::Output {
        todo!()
    }
}

impl<T, U> SparseCollection<T, U> for SparseMap<T, U> {
    fn size() -> usize {
        todo!()
    }

    fn sparse_size() -> usize {
        todo!()
    }
}