use crate::identifiers::primitives::Generation;

pub struct Slot<T> {
    index: T,
    generation: Generation
}