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
    
    /// Deserializes participants from bytes
    unsafe fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != std::mem::size_of::<Self>() {
            return None;
        }
        Some(std::ptr::read(bytes.as_ptr() as *const Self))
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