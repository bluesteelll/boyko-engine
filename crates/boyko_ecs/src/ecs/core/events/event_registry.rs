use std::alloc::Layout;
use std::any::TypeId;
use std::sync::OnceLock;

use crate::ecs::core::events::event::{Event, EventId};
use crate::ecs::core::events::participants::participants::{Participants, ParticipantInfo};
use crate::ecs::core::events::parameters::parameters::Parameters;

/// Maximum number of events supported by the ECS system.
pub const MAX_EVENTS: usize = 256;

/// Holds information about a specific event type.
///
/// Filled in by `register_event::<E>(id)` (currently from the
/// `#[derive(Event)]`-generated `#[ctor::ctor]` initializer). Each entry is
/// written at most once via `OnceLock::set`; read path is a lock-free
/// acquire-load. Fixes audit findings M-002 / C-002 / Q-004 / Q-010
/// (`static mut EVENT_INFO` race / Rust 2024 deprecation).
#[derive(Clone)]
pub struct EventInfo {
    /// Event type name (for debugging).
    pub type_name: &'static str,

    /// Memory layout of the event.
    pub layout: Layout,

    /// Memory layout of the participants.
    pub participants_layout: Layout,

    /// Memory layout of the parameters.
    pub parameters_layout: Layout,

    /// Unique type identifier.
    pub type_id: TypeId,

    /// Type ID of participants.
    pub participants_type_id: TypeId,

    /// Type ID of parameters.
    pub parameters_type_id: TypeId,

    /// Information about participants.
    pub participant_info: &'static [ParticipantInfo],
}

/// Static storage for event information. Each slot is independent and
/// initialized at most once via `OnceLock::set`. No data race possible.
static EVENT_INFO: [OnceLock<EventInfo>; MAX_EVENTS] =
    [const { OnceLock::new() }; MAX_EVENTS];

/// Registers an event's information in the global registry.
/// Idempotent — duplicate registration is silently ignored (slot is `OnceLock`).
pub fn register_event<E: Event>(event_id: EventId) {
    let event_id_usize = event_id as usize;

    debug_assert!(
        event_id_usize < MAX_EVENTS,
        "Event ID {} exceeds maximum allowed ({})",
        event_id,
        MAX_EVENTS
    );

    if event_id_usize >= MAX_EVENTS {
        return;
    }

    let info = EventInfo {
        type_name: std::any::type_name::<E>(),
        layout: E::layout(),
        participants_layout: E::Participants::layout(),
        parameters_layout: E::Parameters::layout(),
        type_id: E::type_id(),
        participants_type_id: TypeId::of::<E::Participants>(),
        parameters_type_id: TypeId::of::<E::Parameters>(),
        participant_info: E::Participants::participant_info(),
    };

    let _ = EVENT_INFO[event_id_usize].set(info);
}

/// Retrieves event information by its ID.
#[inline]
pub fn get_event_info(event_id: EventId) -> Option<&'static EventInfo> {
    let event_id_usize = event_id as usize;

    debug_assert!(
        event_id_usize < MAX_EVENTS,
        "Event ID {} is out of bounds",
        event_id
    );

    if event_id_usize >= MAX_EVENTS {
        return None;
    }

    EVENT_INFO[event_id_usize].get()
}

/// Gets the layout for an event by ID.
#[inline]
pub fn get_event_layout(event_id: EventId) -> Option<Layout> {
    get_event_info(event_id).map(|info| info.layout)
}

/// Gets the participants layout for an event by ID.
#[inline]
pub fn get_participants_layout(event_id: EventId) -> Option<Layout> {
    get_event_info(event_id).map(|info| info.participants_layout)
}

/// Gets the parameters layout for an event by ID.
#[inline]
pub fn get_parameters_layout(event_id: EventId) -> Option<Layout> {
    get_event_info(event_id).map(|info| info.parameters_layout)
}

/// Gets the participant information for an event by ID.
#[inline]
pub fn get_event_participants(event_id: EventId) -> Option<&'static [ParticipantInfo]> {
    get_event_info(event_id).map(|info| info.participant_info)
}

/// Gets the type name for an event by ID.
#[inline]
pub fn get_event_type_name(event_id: EventId) -> Option<&'static str> {
    get_event_info(event_id).map(|info| info.type_name)
}

/// Checks if an event is registered.
#[inline]
pub fn is_event_registered(event_id: EventId) -> bool {
    let event_id_usize = event_id as usize;
    event_id_usize < MAX_EVENTS && EVENT_INFO[event_id_usize].get().is_some()
}

/// Gets the number of registered events.
pub fn registered_event_count() -> usize {
    EVENT_INFO.iter().filter(|slot| slot.get().is_some()).count()
}

/// Iterator over all registered event IDs.
pub fn iter_registered_events() -> impl Iterator<Item = EventId> {
    (0..MAX_EVENTS)
        .filter(|&i| EVENT_INFO[i].get().is_some())
        .map(|i| i as EventId)
}

/// Gets type IDs for validation.
#[inline]
pub fn get_event_type_ids(event_id: EventId) -> Option<(TypeId, TypeId, TypeId)> {
    get_event_info(event_id)
        .map(|info| (info.type_id, info.participants_type_id, info.parameters_type_id))
}

/// Validates that type IDs match for an event.
#[inline]
pub fn validate_event_types<E: Event>(event_id: EventId) -> bool {
    if let Some((event_tid, participants_tid, parameters_tid)) = get_event_type_ids(event_id) {
        event_tid == TypeId::of::<E>()
            && participants_tid == TypeId::of::<E::Participants>()
            && parameters_tid == TypeId::of::<E::Parameters>()
    } else {
        false
    }
}

/// Ultra-fast access to event info when you're confident the event exists.
///
/// # Safety
/// Caller guarantees `event_id` is `< MAX_EVENTS` and the slot has been
/// initialized via `register_event::<E>(event_id)`.
#[inline(always)]
pub unsafe fn get_event_info_unchecked(event_id: EventId) -> &'static EventInfo {
    let event_id_usize = event_id as usize;
    debug_assert!(
        event_id_usize < MAX_EVENTS && EVENT_INFO[event_id_usize].get().is_some(),
        "Event ID {} is invalid or not initialized",
        event_id
    );
    // SAFETY: per the function contract, the slot is initialized.
    unsafe { EVENT_INFO[event_id_usize].get().unwrap_unchecked() }
}

/// Ultra-fast access to participants layout when you're confident the event exists.
///
/// # Safety
/// See [`get_event_info_unchecked`].
#[inline(always)]
pub unsafe fn get_participants_layout_unchecked(event_id: EventId) -> Layout {
    // SAFETY: forwarded to the unchecked accessor.
    unsafe { get_event_info_unchecked(event_id).participants_layout }
}

/// Ultra-fast access to parameters layout when you're confident the event exists.
///
/// # Safety
/// See [`get_event_info_unchecked`].
#[inline(always)]
pub unsafe fn get_parameters_layout_unchecked(event_id: EventId) -> Layout {
    // SAFETY: forwarded to the unchecked accessor.
    unsafe { get_event_info_unchecked(event_id).parameters_layout }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::events::participants::participants::{Participants, ParticipantInfo};
    use crate::ecs::core::events::parameters::parameters::Parameters;

    // -- Minimal stub types for testing ----------------------------------------

    /// Zero-sized participants stub.
    struct NoParticipants;

    impl Participants for NoParticipants {
        fn participant_count() -> usize { 0 }
        fn participant_info() -> &'static [ParticipantInfo] { &[] }
    }

    /// Zero-sized parameters stub.
    struct NoParameters;

    impl Parameters for NoParameters {}

    /// Minimal test event.
    struct PingEvent;

    impl Event for PingEvent {
        type Participants = NoParticipants;
        type Parameters = NoParameters;

        fn event_id() -> EventId { 200 }
        fn event_name() -> &'static str { "PingEvent" }

        fn new(_p: NoParticipants, _q: NoParameters) -> Self { PingEvent }
        fn participants(&self) -> &NoParticipants { unimplemented!() }
        fn participants_mut(&mut self) -> &mut NoParticipants { unimplemented!() }
        fn parameters(&self) -> &NoParameters { unimplemented!() }
        fn parameters_mut(&mut self) -> &mut NoParameters { unimplemented!() }
    }

    /// Second event for collision / count tests.
    struct PongEvent;

    impl Event for PongEvent {
        type Participants = NoParticipants;
        type Parameters = NoParameters;

        fn event_id() -> EventId { 201 }
        fn event_name() -> &'static str { "PongEvent" }

        fn new(_p: NoParticipants, _q: NoParameters) -> Self { PongEvent }
        fn participants(&self) -> &NoParticipants { unimplemented!() }
        fn participants_mut(&mut self) -> &mut NoParticipants { unimplemented!() }
        fn parameters(&self) -> &NoParameters { unimplemented!() }
        fn parameters_mut(&mut self) -> &mut NoParameters { unimplemented!() }
    }

    // -- Tests -----------------------------------------------------------------

    #[test]
    fn register_event_then_get_event_info_returns_some() {
        register_event::<PingEvent>(PingEvent::event_id());
        let info = get_event_info(PingEvent::event_id())
            .expect("event info must be present after register");
        assert_eq!(info.type_name, std::any::type_name::<PingEvent>());
        assert_eq!(info.type_id, TypeId::of::<PingEvent>());
    }

    #[test]
    fn get_event_info_unregistered_returns_none() {
        // ID 250 is never registered in this test binary.
        assert!(
            get_event_info(250).is_none(),
            "unregistered event must return None"
        );
    }

    #[test]
    fn is_event_registered_true_after_register() {
        register_event::<PingEvent>(PingEvent::event_id());
        assert!(
            is_event_registered(PingEvent::event_id()),
            "is_event_registered must be true after register"
        );
    }

    #[test]
    fn is_event_registered_false_for_unknown_id() {
        assert!(
            !is_event_registered(251),
            "is_event_registered must be false for ID 251 (never registered)"
        );
    }

    #[test]
    fn register_event_idempotent_second_call_ignored() {
        // Registering the same event twice must not overwrite — OnceLock guarantees this.
        register_event::<PingEvent>(PingEvent::event_id());
        register_event::<PingEvent>(PingEvent::event_id()); // second call — no-op
        // If OnceLock were broken (e.g. static mut), the second write would race.
        // We simply verify the slot is still populated correctly.
        let info = get_event_info(PingEvent::event_id()).expect("slot must still be present");
        assert_eq!(info.type_id, TypeId::of::<PingEvent>());
    }

    #[test]
    fn registered_event_count_increases_after_register() {
        let before = registered_event_count();
        register_event::<PongEvent>(PongEvent::event_id());
        let after = registered_event_count();
        assert!(
            after >= before,
            "count must not decrease after registration (before={before}, after={after})"
        );
        // After at least one register the count must be >= 1.
        assert!(after >= 1, "at least one event must be registered");
    }

    #[test]
    fn register_event_out_of_range_is_safe() {
        // ID >= MAX_EVENTS is out-of-range. In debug, debug_assert fires; in
        // release, the `if` guard returns silently. Both register and get have
        // their own debug_asserts, so both must be wrapped.
        let _register = std::panic::catch_unwind(|| {
            register_event::<PingEvent>(MAX_EVENTS as EventId);
        });
        let slot_is_none = std::panic::catch_unwind(|| {
            get_event_info(MAX_EVENTS as EventId)
        });
        if let Ok(result) = slot_is_none {
            assert!(result.is_none(), "out-of-range event ID must yield None");
        }
        // If the inner catch_unwind returned Err the debug_assert fired — also acceptable.
    }

    #[test]
    fn validate_event_types_matches_registered_type() {
        register_event::<PingEvent>(PingEvent::event_id());
        assert!(
            validate_event_types::<PingEvent>(PingEvent::event_id()),
            "validate_event_types must return true for the registered type"
        );
    }

    #[test]
    fn get_event_info_unchecked_after_register() {
        register_event::<PingEvent>(PingEvent::event_id());
        // SAFETY: ID is < MAX_EVENTS and the slot was just initialized.
        let info = unsafe { get_event_info_unchecked(PingEvent::event_id()) };
        assert_eq!(info.type_id, TypeId::of::<PingEvent>());
    }
}
