use std::alloc::Layout;

use crate::ecs::core::entity::entity::Entity;

/// Unique identifier for events
pub type EventId = u64;


/// Trait for event types in the ECS system
pub trait Event: 'static + Sized {
    /// Returns the unique identifier for this event type
    fn event_id() -> EventId;
    
    /// Returns the name of this event type
    fn event_name() -> &'static str;
    
    /// Returns the memory layout for this event type
    fn artuments_layout() -> Layout; 
    
    /// Returns the entities that are participants in this event
    fn get_participents() -> [Entity];
}