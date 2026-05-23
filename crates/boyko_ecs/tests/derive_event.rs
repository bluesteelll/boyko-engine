/// Integration tests for #[derive(Event)] lazy-mint ID semantics.
///
/// Validates audit finding Q-005: the macro-generated `event_id()` must mint a
/// unique, stable ID on first call and return the cached value on subsequent
/// calls — using the per-type `OnceLock` + `register_event_new` path.
///
/// Also serves as a compile-time regression test for the ParticipantInfo path
/// bug flagged by the code reviewer: if the macro generates an incorrect path
/// this file will fail to compile.
use boyko_macros::Event;
use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::event_registry;

/// A minimal event with no fields (no participants, no parameters).
/// Uses empty named-field syntax to satisfy the macro's `Fields::Named` requirement.
#[allow(dead_code)]
#[derive(Event)]
struct PulseEvent {}

/// A second event type to verify distinct ID assignment.
#[allow(dead_code)]
#[derive(Event)]
struct TickEvent {}

/// A third event with a parameter field (not marked participant).
#[allow(dead_code)]
#[derive(Event)]
struct DamageEvent {
    amount: f32,
}

/// derive(Event) must mint an ID on the first call to event_id().
/// The ID must be < MAX_EVENTS and the registry slot must be populated.
#[test]
fn derive_event_first_call_mints_valid_id() {
    let id = PulseEvent::event_id();
    assert!(
        (id as usize) < event_registry::MAX_EVENTS,
        "event_id must be < MAX_EVENTS, got {id}"
    );
    assert!(
        event_registry::is_event_registered(id),
        "registry slot must be populated after first event_id() call"
    );
}

/// The second call to event_id() must return the same value as the first.
/// This is the OnceLock cache effect — no second trip through register_event_new.
#[test]
fn derive_event_emits_lazy_id_second_call_returns_same() {
    let id_first = PulseEvent::event_id();
    let id_second = PulseEvent::event_id();
    assert_eq!(
        id_first,
        id_second,
        "event_id() must be stable: first={id_first}, second={id_second}"
    );
}

/// Two distinct event types must receive different EventIds.
#[test]
fn derive_event_distinct_types_get_distinct_ids() {
    let id_pulse = PulseEvent::event_id();
    let id_tick = TickEvent::event_id();
    let id_damage = DamageEvent::event_id();

    assert_ne!(
        id_pulse,
        id_tick,
        "PulseEvent and TickEvent must have different event IDs \
         (got id_pulse={id_pulse}, id_tick={id_tick})"
    );
    assert_ne!(
        id_pulse,
        id_damage,
        "PulseEvent and DamageEvent must have different event IDs \
         (got id_pulse={id_pulse}, id_damage={id_damage})"
    );
    assert_ne!(
        id_tick,
        id_damage,
        "TickEvent and DamageEvent must have different event IDs \
         (got id_tick={id_tick}, id_damage={id_damage})"
    );
}

/// The registry slot populated by event_id() must carry the correct TypeId.
#[test]
fn derive_event_registry_slot_carries_correct_type_id() {
    use std::any::TypeId;

    let id = TickEvent::event_id();
    let info = event_registry::get_event_info(id)
        .expect("TickEvent slot must be populated");
    assert_eq!(
        info.type_id,
        TypeId::of::<TickEvent>(),
        "EventInfo at TickEvent's slot must carry TypeId::of::<TickEvent>()"
    );
}

/// An event with a parameter field: the generated EventInfo must reflect the
/// parameters layout (non-zero size for DamageEvent which has an f32 field).
#[test]
fn derive_event_with_parameter_field_has_nonzero_parameters_layout() {
    let id = DamageEvent::event_id();
    let info = event_registry::get_event_info(id)
        .expect("DamageEvent slot must be populated");
    assert_eq!(
        info.parameters_layout.size(),
        std::mem::size_of::<DamageEventParameters>(),
        "parameters_layout.size() must match DamageEventParameters size"
    );
}

/// Multiple repeated calls in a loop must all return the same ID (stress variant).
#[test]
fn derive_event_id_is_stable_across_many_calls() {
    let expected = PulseEvent::event_id();
    for i in 0..100 {
        let id = PulseEvent::event_id();
        assert_eq!(
            id,
            expected,
            "call {i}: event_id() returned {id}, expected {expected}"
        );
    }
}
