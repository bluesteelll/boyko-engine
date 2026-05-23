use std::alloc::Layout;

/// Trait for event parameters — data passed when invoking an event.
///
/// Implementers must be `Copy` and contain only POD-like fields suitable for
/// bitwise duplication into the type-erased buffer.
pub trait Parameters: 'static + Sized + Copy {
    /// Returns the layout for this parameters structure.
    fn layout() -> Layout {
        Layout::new::<Self>()
    }
}
