/// Direct pointer to a component slot inside a `ComponentPool`'s arena buffer.
///
/// `Unit` is intentionally minimal: a single raw pointer. The pool's index of the
/// component in its dense `Vec<Unit>` is implied by the position within that Vec —
/// duplicating it as a field made `buffer_index` redundant (audit M-005: every
/// caller passed the same value as `self.units.len()` and no one read it back).
///
/// Layout: `#[repr(transparent)]` — same size and alignment as `*mut u8`, no padding.
/// `*mut u8` propagates `!Send + !Sync` to the struct, so no `PhantomData` marker
/// is needed (audit M-006 was a false alarm — the raw pointer already opts out).
#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct Unit {
    /// Direct pointer to the component in memory.
    ptr: *mut u8,
}

impl Unit {
    /// Creates a new Unit pointing at `ptr`.
    #[inline]
    pub fn new(ptr: *mut u8) -> Self {
        Self { ptr }
    }

    /// Returns the pointer to the component.
    #[inline]
    pub fn ptr(&self) -> *mut u8 {
        self.ptr
    }
}