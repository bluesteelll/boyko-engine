use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::ComponentId;
use std::alloc::Layout;

/// Trait for event participants - entities involved in an event
pub trait Participants: 'static + Sized {
    /// Returns the layout for this participants structure
    fn layout() -> Layout {
        Layout::new::<Self>()
    }
    
    /// Returns the number of participants
    fn participant_count() -> usize;
    
    /// Returns participant metadata (name and required components for each)
    fn participant_info() -> &'static [ParticipantInfo];
    
    /// Serializes participants to bytes
    fn to_bytes(&self) -> Vec<u8> {
        let size = std::mem::size_of::<Self>();
        let mut bytes = Vec::with_capacity(size);
        unsafe {
            let ptr = self as *const Self as *const u8;
            bytes.extend_from_slice(std::slice::from_raw_parts(ptr, size));
        }
        bytes
    }
    
    /// Deserializes participants from bytes.
    ///
    /// # Safety
    /// Caller guarantees that `bytes` contains a valid bit-pattern of `Self`
    /// (i.e. produced by `to_bytes()` or `ptr::write` on a live `Self`).
    /// The source buffer may have any alignment — we use `read_unaligned`
    /// which tolerates unaligned pointers (`ParticipantBuffer` stores data
    /// in a byte-aligned `Vec<u8>`, see Q-002 in docs/AUDIT-2026-05-23.md).
    unsafe fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != std::mem::size_of::<Self>() {
            return None;
        }
        // SAFETY: length validated above; `read_unaligned` requires only that
        // `bytes.as_ptr()` is valid for reads of `size_of::<Self>()` bytes —
        // which holds since `bytes` is a `&[u8]` of exactly that length.
        Some(unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const Self) })
    }
}

/// Information about a single participant in an event
#[derive(Clone, Debug)]
pub struct ParticipantInfo {
    /// Name of the participant (e.g., "attacker", "victim")
    pub name: &'static str,

    /// Component IDs required from this participant
    pub required_components: &'static [ComponentId],
}

#[cfg(test)]
mod tests {
    use super::*;

    // A concrete Participants impl with a non-trivial alignment to
    // trigger the old UB (ptr::read on an unaligned pointer).
    #[repr(C)]
    #[derive(Debug, PartialEq, Clone, Copy)]
    struct TwoU64Participants {
        a: u64,
        b: u64,
    }

    impl Participants for TwoU64Participants {
        fn participant_count() -> usize { 1 }
        fn participant_info() -> &'static [ParticipantInfo] { &[] }
    }

    // --- to_bytes / from_bytes round-trip ---

    #[test]
    fn to_bytes_then_from_bytes_returns_original() {
        let original = TwoU64Participants { a: 0xDEAD_BEEF, b: 0xCAFE_BABE };
        let bytes = original.to_bytes();
        assert_eq!(
            bytes.len(),
            std::mem::size_of::<TwoU64Participants>(),
            "to_bytes must produce exactly size_of bytes"
        );
        // SAFETY: bytes were produced by to_bytes() — valid bit-pattern guaranteed.
        let recovered = unsafe { TwoU64Participants::from_bytes(&bytes) };
        assert_eq!(
            recovered,
            Some(original),
            "round-trip to_bytes → from_bytes must reproduce original value"
        );
    }

    #[test]
    fn from_bytes_wrong_length_returns_none() {
        let too_short = vec![0u8; std::mem::size_of::<TwoU64Participants>() - 1];
        // SAFETY: wrong length — function must return None without UB.
        let result = unsafe { TwoU64Participants::from_bytes(&too_short) };
        assert!(result.is_none(), "from_bytes must return None on size mismatch");
    }

    #[test]
    fn from_bytes_on_unaligned_buffer_no_ub() {
        // Q-002 regression: the old code used ptr::read which requires alignment.
        // The fix uses ptr::read_unaligned. We place participant bytes at offset +1
        // inside a Vec<u8> to guarantee misalignment relative to u64 (align=8).
        let value = TwoU64Participants { a: 0x1122_3344, b: 0x5566_7788 };
        let size = std::mem::size_of::<TwoU64Participants>();

        // Build a buffer that is intentionally offset by 1 byte.
        let mut buf = vec![0xFFu8; size + 1];
        // Write participant bytes starting at offset 1.
        let src = value.to_bytes();
        buf[1..=size].copy_from_slice(&src);

        // Read from the unaligned slice (offset 1).
        let unaligned_slice = &buf[1..=size];
        // SAFETY: bytes were written by to_bytes() at this exact slice position.
        let recovered = unsafe { TwoU64Participants::from_bytes(unaligned_slice) };
        assert_eq!(
            recovered,
            Some(value),
            "from_bytes must succeed on an unaligned buffer — Q-002 regression"
        );
    }

    #[test]
    fn from_bytes_empty_slice_returns_none_for_nonempty_type() {
        // SAFETY: empty slice — from_bytes must detect length mismatch and return None.
        let result = unsafe { TwoU64Participants::from_bytes(&[]) };
        assert!(result.is_none(), "empty slice must return None");
    }
}