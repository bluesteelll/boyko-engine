use std::ops::{Index, IndexMut};
use crate::identifiers::slot::Slot;
use super::sparse_collection::SparseCollection;

pub struct SparseSlotMap<T: Sized, U>{
    sparse: Vec<Slot<T>>,
    dense: Vec<U>
}

impl<T, U> SparseSlotMap<T, U> {
    todo!();
}

impl<T, U> Index<T> for SparseSlotMap<T, U> {
    type Output = ();

    fn index(&self, index: T) -> &Self::Output {
        todo!()
    }
}

impl<T, U> IndexMut<T> for SparseSlotMap<T, U> {
    fn index_mut(&mut self, index: T) -> &mut Self::Output {
        todo!()
    }
}

impl<T, U> SparseCollection<T, U> for SparseSlotMap<T, U> {
    fn size() -> usize {
        todo!()
    }

    fn sparse_size() -> usize {
        todo!()
    }
}