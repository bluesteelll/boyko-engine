use crate::identifiers::primitives::Generation;


pub struct Slot<T: Sized> {
    index: T,
    generation: Generation
}