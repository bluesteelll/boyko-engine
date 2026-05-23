use std::alloc::Layout;

/// Trait for event parameters - data passed when invoking an event
/// 
/// This trait has a blanket implementation for all Sized types,
/// so any type can be used as event parameters without explicit implementation
pub trait Parameters: 'static + Sized {
    /// Returns the layout for this parameters structure
    fn layout() -> Layout {
        Layout::new::<Self>()
    }
    
    /// Serializes parameters to bytes
    /// Default implementation handles any Sized type
    fn to_bytes(&self) -> Vec<u8> {
        let size = std::mem::size_of::<Self>();
        let mut bytes = Vec::with_capacity(size);
        unsafe {
            let ptr = self as *const Self as *const u8;
            bytes.extend_from_slice(std::slice::from_raw_parts(ptr, size));
        }
        bytes
    }
    
    /// Deserializes parameters from bytes.
    ///
    /// # Safety
    /// Caller guarantees that `bytes` contains a valid bit-pattern of `Self`
    /// (i.e. produced by `to_bytes()` or `ptr::write` on a live `Self`).
    /// The source buffer may have any alignment — we use `read_unaligned`
    /// which tolerates unaligned pointers (`ParametersBuffer` stores data
    /// in a byte-aligned `Vec<u8>`, see Q-002 in docs/AUDIT-2026-05-23.md).
    unsafe fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != std::mem::size_of::<Self>() {
            return None;
        }
        // SAFETY: length validated above; `read_unaligned` requires only that
        // `bytes.as_ptr()` is valid for reads of `size_of::<Self>()` bytes.
        Some(unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const Self) })
    }
    
    /// Creates a raw pointer to the parameters
    fn as_ptr(&self) -> *const u8 {
        self as *const Self as *const u8
    }
    
    /// Creates a mutable raw pointer to the parameters
    fn as_mut_ptr(&mut self) -> *mut u8 {
        self as *mut Self as *mut u8
    }
}
