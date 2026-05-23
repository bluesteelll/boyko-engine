//! Global registry of event information.
//!
//! # Event ID assignment
//!
//! Each distinct type `E` implementing [`Event`](crate::ecs::core::events::event::Event)
//! is assigned a unique [`EventId`] the first time `E::event_id()` is called
//! in the current process. The assignment is lazy, lock-free on the cached
//! read path, and stable for the lifetime of the process — but **not** stable
//! across processes or across runs of the same binary if the order of first
//! calls differs.
//!
//! # Startup warm-up contract
//!
//! Code that ingests `EventId`s from external sources (network, save files,
//! scripts, etc.) MUST warm up the registry by calling `E::event_id()` for
//! every event type `E` it expects to receive, *before* the first external ID
//! arrives. Without warm-up, an incoming id `i` may refer to type `A` in this
//! process but type `B` in a peer process — IDs are assigned in first-call
//! order.
//!
//! Recommended pattern: at engine startup, call `<E as Event>::event_id()`
//! for every event type that will be serialized, in a deterministic order.
//!
//! # Collision detection
//!
//! Every `set` call site ([`register_event_new`] and [`register_event`])
//! checks the slot before declaring success. If the slot is already occupied
//! by a *different* type than the one being registered, the call panics in
//! both debug and release builds, naming both types. This catches accidental
//! ID-space overlaps between the production counter and the test escape hatch
//! immediately.
//!
//! # Threading
//!
//! All registry operations are safe to call from any thread. The global
//! `NEXT_EVENT_ID` counter uses `Relaxed` ordering (uniqueness is sufficient;
//! cross-thread happens-before is provided by `OnceLock::set` / `get`).
//! Per-slot `OnceLock`s provide acquire/release synchronization of the
//! `EventInfo` payload.

use std::alloc::Layout;
use std::any::TypeId;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ecs::core::events::event::{Event, EventId};
use crate::ecs::core::events::participants::participants::{Participants, ParticipantInfo};
use crate::ecs::core::events::parameters::parameters::Parameters;

/// Maximum number of events supported by the ECS system.
pub const MAX_EVENTS: usize = 256;

/// Holds information about a specific event type.
///
/// Filled in by [`register_event_new`] or [`register_event`]. Each entry is
/// written at most once via `OnceLock::set`; read path is a lock-free
/// acquire-load. Fixes audit findings M-002 / C-002 / Q-004 / Q-010.
#[derive(Clone)]
pub struct EventInfo {
    /// Event type name (for debugging and collision messages).
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

/// Monotonic counter for event IDs minted via [`register_event_new`].
/// Test code that needs explicit IDs uses [`register_event`] and bypasses
/// this counter — collisions are detected per-slot.
static NEXT_EVENT_ID: AtomicUsize = AtomicUsize::new(0);

/// Allocates a fresh `EventId` from the global counter and stores `EventInfo`
/// derived from `E` in the corresponding `EVENT_INFO` slot.
///
/// Production path: called from `#[derive(Event)]`-generated `E::event_id()`
/// via a per-monomorphization `OnceLock`. Each concrete `E` gets exactly one
/// ID across the process lifetime, regardless of how many threads call
/// `E::event_id()` concurrently.
///
/// See [`crate::ecs::core::component::component_registry::register_new`] for
/// the mirror design on the Component side.
///
/// # Panics
/// - If `NEXT_EVENT_ID` reaches `MAX_EVENTS`.
/// - If the slot is already occupied by a *different* `Event` type.
pub fn register_event_new<E: Event>() -> EventId {
    let raw = NEXT_EVENT_ID.fetch_add(1, Ordering::Relaxed);
    assert!(
        raw < MAX_EVENTS,
        "EventRegistry exhausted: NEXT_EVENT_ID reached {}, MAX_EVENTS = {}",
        raw,
        MAX_EVENTS
    );
    let id = raw as EventId;
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
    match EVENT_INFO[raw].set(info) {
        Ok(()) => id,
        Err(_) => {
            let existing = EVENT_INFO[raw]
                .get()
                .expect("invariant: OnceLock::set Err implies the slot is occupied");
            if existing.type_id == TypeId::of::<E>() {
                id
            } else {
                panic!(
                    "EventId {} occupied by type {}, refused to register {}",
                    id,
                    existing.type_name,
                    std::any::type_name::<E>()
                )
            }
        }
    }
}

/// Test-only escape hatch: registers `E` under an explicit `event_id`.
///
/// Production code must not call this — use `E::event_id()` (which goes
/// through [`register_event_new`]). Tests use this to install events under
/// known, fixed IDs without depending on `NEXT_EVENT_ID`'s value.
///
/// # Panics
/// - If `event_id >= MAX_EVENTS` (as `usize`).
/// - If the slot is already occupied by a *different* `Event` type. Same-type
///   re-registration is silently idempotent.
#[doc(hidden)]
pub fn register_event<E: Event>(event_id: EventId) {
    let id_usize = event_id as usize;
    assert!(
        id_usize < MAX_EVENTS,
        "Event ID {} exceeds maximum allowed ({})",
        event_id,
        MAX_EVENTS
    );
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
    match EVENT_INFO[id_usize].set(info) {
        Ok(()) => {}
        Err(_) => {
            let existing = EVENT_INFO[id_usize]
                .get()
                .expect("invariant: OnceLock::set Err implies the slot is occupied");
            if existing.type_id != TypeId::of::<E>() {
                panic!(
                    "EventId {} occupied by type {}, refused to register {}",
                    event_id,
                    existing.type_name,
                    std::any::type_name::<E>()
                )
            }
            // Same type — silent no-op (idempotent).
        }
    }
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
/// Caller guarantees that `event_id < MAX_EVENTS` (as `usize`) and that one
/// of the following has already completed for the corresponding type `E`:
/// - [`register_event_new::<E>()`] (production path, via `E::event_id()`), or
/// - [`register_event::<E>(event_id)`] (test-only escape hatch).
/// Violating either yields UB.
#[inline(always)]
pub unsafe fn get_event_info_unchecked(event_id: EventId) -> &'static EventInfo {
    let event_id_usize = event_id as usize;
    debug_assert!(
        event_id_usize < MAX_EVENTS && EVENT_INFO[event_id_usize].get().is_some(),
        "Event ID {} is invalid or not initialized",
        event_id
    );
    // SAFETY: per the function contract, the slot is initialized and
    // `event_id_usize < MAX_EVENTS`.
    unsafe { EVENT_INFO[event_id_usize].get().unwrap_unchecked() }
}

/// Ultra-fast access to participants layout when you're confident the event exists.
///
/// # Safety
/// Caller guarantees that `event_id < MAX_EVENTS` (as `usize`) and that one
/// of the following has already completed for the corresponding type `E`:
/// - [`register_event_new::<E>()`] (production path, via `E::event_id()`), or
/// - [`register_event::<E>(event_id)`] (test-only escape hatch).
/// Violating either yields UB.
#[inline(always)]
pub unsafe fn get_participants_layout_unchecked(event_id: EventId) -> Layout {
    // SAFETY: forwarded to the unchecked accessor; caller satisfies the same contract.
    unsafe { get_event_info_unchecked(event_id).participants_layout }
}

/// Ultra-fast access to parameters layout when you're confident the event exists.
///
/// # Safety
/// Caller guarantees that `event_id < MAX_EVENTS` (as `usize`) and that one
/// of the following has already completed for the corresponding type `E`:
/// - [`register_event_new::<E>()`] (production path, via `E::event_id()`), or
/// - [`register_event::<E>(event_id)`] (test-only escape hatch).
/// Violating either yields UB.
#[inline(always)]
pub unsafe fn get_parameters_layout_unchecked(event_id: EventId) -> Layout {
    // SAFETY: forwarded to the unchecked accessor; caller satisfies the same contract.
    unsafe { get_event_info_unchecked(event_id).parameters_layout }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::events::participants::participants::{Participants, ParticipantInfo};
    use crate::ecs::core::events::parameters::parameters::Parameters;

    // -- Minimal stub types for testing ----------------------------------------

    /// Zero-sized participants stub.
    #[derive(Clone, Copy)]
    struct NoParticipants;

    impl Participants for NoParticipants {
        fn participant_count() -> usize { 0 }
        fn participant_info() -> &'static [ParticipantInfo] { &[] }
    }

    /// Zero-sized parameters stub.
    #[derive(Clone, Copy)]
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

    // -- Additional types for new Phase 1b tests (Q-005 / C-003 mirror) ----------

    /// Collision-A event — distinct type used for collision-detection tests.
    /// ID 210 reserved; must not be used by any other test in this crate.
    struct ColEventA;

    impl Event for ColEventA {
        type Participants = NoParticipants;
        type Parameters = NoParameters;
        fn event_id() -> EventId { 210 }
        fn event_name() -> &'static str { "ColEventA" }
        fn new(_p: NoParticipants, _q: NoParameters) -> Self { ColEventA }
        fn participants(&self) -> &NoParticipants { unimplemented!() }
        fn participants_mut(&mut self) -> &mut NoParticipants { unimplemented!() }
        fn parameters(&self) -> &NoParameters { unimplemented!() }
        fn parameters_mut(&mut self) -> &mut NoParameters { unimplemented!() }
    }

    /// Collision-B event — different type, also targeting ID 210 to trigger collision.
    struct ColEventB;

    impl Event for ColEventB {
        type Participants = NoParticipants;
        type Parameters = NoParameters;
        fn event_id() -> EventId { 210 }  // same ID as ColEventA — used in collision test
        fn event_name() -> &'static str { "ColEventB" }
        fn new(_p: NoParticipants, _q: NoParameters) -> Self { ColEventB }
        fn participants(&self) -> &NoParticipants { unimplemented!() }
        fn participants_mut(&mut self) -> &mut NoParticipants { unimplemented!() }
        fn parameters(&self) -> &NoParameters { unimplemented!() }
        fn parameters_mut(&mut self) -> &mut NoParameters { unimplemented!() }
    }

    /// Idempotency-A event — same type registered twice under ID 215.
    struct IdempEventA;

    impl Event for IdempEventA {
        type Participants = NoParticipants;
        type Parameters = NoParameters;
        fn event_id() -> EventId { 215 }
        fn event_name() -> &'static str { "IdempEventA" }
        fn new(_p: NoParticipants, _q: NoParameters) -> Self { IdempEventA }
        fn participants(&self) -> &NoParticipants { unimplemented!() }
        fn participants_mut(&mut self) -> &mut NoParticipants { unimplemented!() }
        fn parameters(&self) -> &NoParameters { unimplemented!() }
        fn parameters_mut(&mut self) -> &mut NoParameters { unimplemented!() }
    }

    /// Two events for register_event_new distinctness test.
    struct NewEventTypeA;

    impl Event for NewEventTypeA {
        type Participants = NoParticipants;
        type Parameters = NoParameters;
        fn event_id() -> EventId {
            static ID: ::std::sync::OnceLock<EventId> = ::std::sync::OnceLock::new();
            *ID.get_or_init(|| register_event_new::<Self>())
        }
        fn event_name() -> &'static str { "NewEventTypeA" }
        fn new(_p: NoParticipants, _q: NoParameters) -> Self { NewEventTypeA }
        fn participants(&self) -> &NoParticipants { unimplemented!() }
        fn participants_mut(&mut self) -> &mut NoParticipants { unimplemented!() }
        fn parameters(&self) -> &NoParameters { unimplemented!() }
        fn parameters_mut(&mut self) -> &mut NoParameters { unimplemented!() }
    }

    struct NewEventTypeB;

    impl Event for NewEventTypeB {
        type Participants = NoParticipants;
        type Parameters = NoParameters;
        fn event_id() -> EventId {
            static ID: ::std::sync::OnceLock<EventId> = ::std::sync::OnceLock::new();
            *ID.get_or_init(|| register_event_new::<Self>())
        }
        fn event_name() -> &'static str { "NewEventTypeB" }
        fn new(_p: NoParticipants, _q: NoParameters) -> Self { NewEventTypeB }
        fn participants(&self) -> &NoParticipants { unimplemented!() }
        fn participants_mut(&mut self) -> &mut NoParticipants { unimplemented!() }
        fn parameters(&self) -> &NoParameters { unimplemented!() }
        fn parameters_mut(&mut self) -> &mut NoParameters { unimplemented!() }
    }

    // -- Tests -----------------------------------------------------------------

    // ----- NEW TESTS: Phase 1b Q-005 -----

    /// register_event_new for two distinct types must return different EventIds.
    ///
    /// Uses OnceLock-wrapped event_id() to mirror the macro-generated pattern.
    /// The OnceLock ensures each type mints exactly one ID for the process lifetime.
    #[test]
    fn register_event_new_assigns_distinct_ids_for_distinct_types() {
        let id_a = NewEventTypeA::event_id();
        let id_b = NewEventTypeB::event_id();
        assert_ne!(
            id_a,
            id_b,
            "register_event_new must assign different IDs to NewEventTypeA and NewEventTypeB \
             (got id_a={id_a}, id_b={id_b})"
        );
        // Both slots must be populated with matching type info.
        let info_a = get_event_info(id_a).expect("slot for NewEventTypeA must be populated");
        let info_b = get_event_info(id_b).expect("slot for NewEventTypeB must be populated");
        assert_eq!(
            info_a.type_id,
            TypeId::of::<NewEventTypeA>(),
            "EventInfo at id_a must carry NewEventTypeA type_id"
        );
        assert_eq!(
            info_b.type_id,
            TypeId::of::<NewEventTypeB>(),
            "EventInfo at id_b must carry NewEventTypeB type_id"
        );
    }

    /// register_event_new is idempotent when called through a per-type OnceLock:
    /// second call returns the cached ID, same as the first.
    #[test]
    fn register_event_new_returns_same_id_on_repeat_via_oncelock() {
        let id_first = NewEventTypeA::event_id();
        let id_second = NewEventTypeA::event_id();
        assert_eq!(
            id_first,
            id_second,
            "OnceLock-wrapped event_id() must return the same ID on every call"
        );
    }

    /// Collision detection: registering a different event type in an already-occupied
    /// slot must panic with a message naming both types.
    ///
    /// ID 210: first occupied by ColEventA, then ColEventB triggers the panic.
    /// Expected panic substring: "occupied by type" (matches the format string
    /// "EventId {} occupied by type {}, refused to register {}").
    #[test]
    #[should_panic(expected = "occupied by type")]
    fn register_event_collision_with_different_type_panics() {
        register_event::<ColEventA>(210);
        register_event::<ColEventB>(210);
    }

    /// Collision idempotent path: registering the SAME event type twice under the
    /// same ID is a silent no-op. The slot must remain populated and valid.
    ///
    /// ID 215: both calls use IdempEventA.
    #[test]
    fn register_event_collision_with_same_type_is_silent_noop() {
        register_event::<IdempEventA>(215);
        register_event::<IdempEventA>(215); // second call — must not panic
        let info = get_event_info(215)
            .expect("slot must remain populated after idempotent re-registration");
        assert_eq!(
            info.type_id,
            TypeId::of::<IdempEventA>(),
            "slot type_id must remain IdempEventA after silent no-op"
        );
    }

    /// register_event panics with the expected message when event_id >= MAX_EVENTS.
    ///
    /// Uses #[should_panic] with a tighter expected substring to lock in the
    /// panic message format "Event ID {} exceeds maximum allowed ({})".
    #[test]
    #[should_panic(expected = "exceeds maximum allowed")]
    fn register_event_at_max_events_panics_with_expected_message() {
        register_event::<PingEvent>(MAX_EVENTS as EventId);
    }

    // NOTE: register_event_new exhaustion test (driving NEXT_EVENT_ID to MAX_EVENTS)
    // is NOT included. NEXT_EVENT_ID is private with no test-only reset accessor.
    // The same limitation applies as in component_registry.rs.
    // TODO: developer to add #[cfg(test)] fn set_next_event_id_for_test(v: usize).

    // ----- END NEW TESTS -----

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
        assert!(after >= 1, "at least one event must be registered");
    }

    #[test]
    fn register_event_out_of_range_panics() {
        // register_event now uses assert! (not debug_assert) — always panics OOB.
        let result = std::panic::catch_unwind(|| {
            register_event::<PingEvent>(MAX_EVENTS as EventId);
        });
        assert!(result.is_err(), "out-of-range register_event must panic");
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
