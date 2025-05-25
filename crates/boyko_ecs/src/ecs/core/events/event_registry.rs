use std::alloc::Layout;
use std::any::TypeId;
use std::sync::atomic::{AtomicBool, Ordering};
use crate::ecs::core::events::event::{EventId, ParticipantInfo};

/// Maximum number of events supported by the ECS system
const MAX_EVENTS: usize = 256;

/// Holds information about a specific event type
#[derive(Clone)]
pub struct EventInfo {
    /// Event type name (for debugging)
    pub type_name: &'static str,
    
    /// Memory layout of the event
    pub layout: Layout,
    
    /// Unique type identifier
    pub type_id: TypeId,
    
    /// Information about participants and their required components
    pub participant_info: &'static [ParticipantInfo],
}

/// Static registry for event information
struct StaticEventRegistry {
    /// Initialization flags for each event
    initialized: [AtomicBool; MAX_EVENTS],
}

// Create a zeroed array of AtomicBool
const ZEROED_ATOMIC_BOOL: AtomicBool = AtomicBool::new(false);

/// The global static registry instance
static REGISTRY: StaticEventRegistry = StaticEventRegistry {
    initialized: [ZEROED_ATOMIC_BOOL; MAX_EVENTS],
};

// Static storage for event information
static mut EVENT_INFO: [Option<EventInfo>; MAX_EVENTS] = [const { None }; MAX_EVENTS];

/// Registers an event's information in the global registry
/// This is typically called during program initialization
pub fn register_event<E: crate::ecs::core::events::event::Event>(event_id: EventId) {
    let event_id_usize = event_id as usize;
    
    // Check bounds only in debug builds
    debug_assert!(event_id_usize < MAX_EVENTS, 
        "Event ID {} exceeds maximum allowed ({})", event_id, MAX_EVENTS);
    
    if event_id_usize >= MAX_EVENTS {
        panic!("Event ID {} exceeds maximum allowed ({})", event_id, MAX_EVENTS);
    }
    
    // First check if the event is already initialized (fast path)
    if !REGISTRY.initialized[event_id_usize].load(Ordering::Relaxed) {
        // Create the event info
        let info = EventInfo {
            type_name: std::any::type_name::<E>(),
            layout: E::layout(),
            type_id: E::type_id(),
            participant_info: E::participant_info(),
        };
        
        // Try to mark as initialized using atomic swap
        if !REGISTRY.initialized[event_id_usize].swap(true, Ordering::AcqRel) {
            // We won the race - write the info to the static array
            unsafe {
                EVENT_INFO[event_id_usize] = Some(info);
            }
        }
    }
}

/// Retrieves event information by its ID
/// Returns None if the event hasn't been registered yet
#[inline]
pub fn get_event_info(event_id: EventId) -> Option<&'static EventInfo> {
    let event_id_usize = event_id as usize;
    
    // Check bounds only in debug builds
    debug_assert!(event_id_usize < MAX_EVENTS, 
        "Event ID {} is out of bounds", event_id);
    
    if event_id_usize >= MAX_EVENTS {
        return None;
    }
    
    // Check if the event has been initialized
    if REGISTRY.initialized[event_id_usize].load(Ordering::Acquire) {
        // Safe to return a reference since the data is static
        unsafe {
            EVENT_INFO[event_id_usize].as_ref()
        }
    } else {
        None
    }
}

/// Gets the layout for an event by ID
#[inline]
pub fn get_event_layout(event_id: EventId) -> Option<Layout> {
    get_event_info(event_id).map(|info| info.layout)
}

/// Gets the participant information for an event by ID
#[inline]
pub fn get_event_participants(event_id: EventId) -> Option<&'static [ParticipantInfo]> {
    get_event_info(event_id).map(|info| info.participant_info)
}

/// Gets the type name for an event by ID
#[inline]
pub fn get_event_type_name(event_id: EventId) -> Option<&'static str> {
    get_event_info(event_id).map(|info| info.type_name)
}

/// Checks if an event is registered
#[inline]
pub fn is_event_registered(event_id: EventId) -> bool {
    let event_id_usize = event_id as usize;
    event_id_usize < MAX_EVENTS && REGISTRY.initialized[event_id_usize].load(Ordering::Relaxed)
}

/// Gets the number of registered events
pub fn registered_event_count() -> usize {
    let mut count = 0;
    for i in 0..MAX_EVENTS {
        if REGISTRY.initialized[i].load(Ordering::Relaxed) {
            count += 1;
        }
    }
    count
}

/// Ultra-fast access to event info when you're confident the event exists
/// Will cause undefined behavior if event_id is invalid - use with caution!
#[inline(always)]
pub unsafe fn get_event_info_unchecked(event_id: EventId) -> &'static EventInfo {
    let event_id_usize = event_id as usize;
    debug_assert!(event_id_usize < MAX_EVENTS && 
        REGISTRY.initialized[event_id_usize].load(Ordering::Relaxed),
        "Event ID {} is invalid or not initialized", event_id);
    
    EVENT_INFO[event_id_usize].as_ref().unwrap_unchecked()
}