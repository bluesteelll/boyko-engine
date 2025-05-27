use std::alloc::Layout;
use std::any::TypeId;
use crate::ecs::core::events::participants::participants::{Participants, ParticipantInfo};
use crate::ecs::core::events::parameters::parameters::Parameters;

/// Unique identifier for events
pub type EventId = u64;

/// Trait for event types in the ECS system
pub trait Event: 'static + Sized {
    /// The participants type for this event
    type Participants: Participants;
    
    /// The parameters type for this event
    type Parameters: Parameters;
    
    /// Returns the unique identifier for this event type
    fn event_id() -> EventId;
    
    /// Returns the name of this event type
    fn event_name() -> &'static str;
    
    /// Returns the memory layout for this event type
    fn layout() -> Layout {
        Layout::new::<Self>()
    }
    
    /// Returns the TypeId of this event
    fn type_id() -> TypeId {
        TypeId::of::<Self>()
    }
    
    /// Creates an event instance from participants and parameters
    fn new(participants: Self::Participants, parameters: Self::Parameters) -> Self;
    
    /// Gets a reference to the participants
    fn participants(&self) -> &Self::Participants;
    
    /// Gets a mutable reference to the participants
    fn participants_mut(&mut self) -> &mut Self::Participants;
    
    /// Gets a reference to the parameters
    fn parameters(&self) -> &Self::Parameters;
    
    /// Gets a mutable reference to the parameters  
    fn parameters_mut(&mut self) -> &mut Self::Parameters;
}