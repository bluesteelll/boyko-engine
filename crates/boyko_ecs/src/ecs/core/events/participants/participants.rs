use crate::ecs::identifiers::primitives::ComponentId;
use std::alloc::Layout;

/// Trait for event participants — entities involved in an event.
///
/// Implementers must be `Copy` and contain only POD-like fields suitable for
/// bitwise duplication into the type-erased buffer.
pub trait Participants: 'static + Sized + Copy {
    /// Returns the layout for this participants structure.
    fn layout() -> Layout {
        Layout::new::<Self>()
    }

    /// Returns the number of participants.
    fn participant_count() -> usize;

    /// Returns participant metadata (name and required components for each).
    fn participant_info() -> &'static [ParticipantInfo];
}

/// Information about a single participant in an event.
#[derive(Clone, Debug)]
pub struct ParticipantInfo {
    /// Name of the participant (e.g., "attacker", "victim").
    pub name: &'static str,

    /// Component IDs required from this participant.
    pub required_components: &'static [ComponentId],
}
