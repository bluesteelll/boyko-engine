use std::alloc::Layout;
use std::any::TypeId;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::ComponentId;

/// Unique identifier for events
pub type EventId = u64;

/// Information about a participant in an event
#[derive(Clone, Debug)]
pub struct ParticipantInfo {
    /// Name of the participant (e.g., "attacker", "victim")
    pub name: &'static str,
    
    /// Component IDs required from this participant
    pub required_components: Vec<ComponentId>,
}

/// Trait for event types in the ECS system
pub trait Event: 'static + Sized {
    /// Returns the unique identifier for this event type
    fn event_id() -> EventId;
    
    /// Returns the name of this event type
    fn event_name() -> &'static str;
    
    /// Returns the memory layout for this event type
    fn layout() -> Layout {
        Layout::new::<Self>()
    }
    
    /// Returns information about participants and their required components
    fn participant_info() -> &'static [ParticipantInfo];
    
    /// Returns the number of participants in this event
    fn participant_count() -> usize {
        Self::participant_info().len()
    }
    
    /// Gets the participants (entities) from the event instance
    fn get_participants(&self) -> &[Entity];
    
    /// Gets a mutable reference to participants
    fn get_participants_mut(&mut self) -> &mut [Entity];
    
    /// Returns the TypeId of this event
    fn type_id() -> TypeId {
        TypeId::of::<Self>()
    }
}

/// Type-erased event data for storage in pools
pub struct ErasedEvent {
    /// The event ID for identifying the type
    pub event_id: EventId,
    
    /// Raw bytes of the event data
    pub data: Vec<u8>,
    
    /// Layout information for proper alignment
    pub layout: Layout,
}

impl ErasedEvent {
    /// Creates a new erased event from a typed event
    pub fn new<E: Event>(event: E) -> Self {
        let layout = E::layout();
        let size = layout.size();
        
        // Convert event to bytes
        let mut data = Vec::with_capacity(size);
        unsafe {
            let event_ptr = &event as *const E as *const u8;
            data.extend_from_slice(std::slice::from_raw_parts(event_ptr, size));
            std::mem::forget(event); // Prevent double drop
        }
        
        Self {
            event_id: E::event_id(),
            data,
            layout,
        }
    }
    
    /// Attempts to get a reference to the typed event
    /// Returns None if the event_id doesn't match
    pub unsafe fn get<E: Event>(&self) -> Option<&E> {
        if self.event_id != E::event_id() {
            return None;
        }
        
        Some(&*(self.data.as_ptr() as *const E))
    }
    
    /// Attempts to get a mutable reference to the typed event
    /// Returns None if the event_id doesn't match
    pub unsafe fn get_mut<E: Event>(&mut self) -> Option<&mut E> {
        if self.event_id != E::event_id() {
            return None;
        }
        
        Some(&mut *(self.data.as_mut_ptr() as *mut E))
    }
}