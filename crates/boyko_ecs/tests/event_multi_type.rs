/// Integration tests for Phase 6: multi-type event isolation.
///
/// Test #25 from the plan. Validates that events of different types are
/// completely independent — registering, sending, and reading one type does
/// not affect any other type.
use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::events::event_dispatcher::EventDispatcher;
use boyko_ecs::ecs::core::events::event_registry::register_event;
use boyko_ecs::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;

// ── Minimal event stubs ───────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct MtNoParticipants;
impl Participants for MtNoParticipants {
    fn participant_count() -> usize { 0 }
    fn participant_info() -> &'static [ParticipantInfo] { &[] }
}

#[derive(Clone, Copy)]
struct MtNoParameters;
impl Parameters for MtNoParameters {}

/// Event type X — ID range 60-69 for this test file.
#[derive(Clone, Copy, Debug, PartialEq)]
struct EventX { x: u32 }
impl Event for EventX {
    type Participants = MtNoParticipants;
    type Parameters = MtNoParameters;
    fn event_id() -> u64 { 60 }
    fn event_name() -> &'static str { "EventX" }
    fn new(_: MtNoParticipants, _: MtNoParameters) -> Self { EventX { x: 0 } }
    fn participants(&self) -> &MtNoParticipants { unimplemented!() }
    fn participants_mut(&mut self) -> &mut MtNoParticipants { unimplemented!() }
    fn parameters(&self) -> &MtNoParameters { unimplemented!() }
    fn parameters_mut(&mut self) -> &mut MtNoParameters { unimplemented!() }
}

/// Event type Y — ID 61.
#[derive(Clone, Copy, Debug, PartialEq)]
struct EventY { y: f32 }
impl Event for EventY {
    type Participants = MtNoParticipants;
    type Parameters = MtNoParameters;
    fn event_id() -> u64 { 61 }
    fn event_name() -> &'static str { "EventY" }
    fn new(_: MtNoParticipants, _: MtNoParameters) -> Self { EventY { y: 0.0 } }
    fn participants(&self) -> &MtNoParticipants { unimplemented!() }
    fn participants_mut(&mut self) -> &mut MtNoParticipants { unimplemented!() }
    fn parameters(&self) -> &MtNoParameters { unimplemented!() }
    fn parameters_mut(&mut self) -> &mut MtNoParameters { unimplemented!() }
}

/// Event type Z — ID 62.
#[derive(Clone, Copy, Debug, PartialEq)]
struct EventZ { z: u8 }
impl Event for EventZ {
    type Participants = MtNoParticipants;
    type Parameters = MtNoParameters;
    fn event_id() -> u64 { 62 }
    fn event_name() -> &'static str { "EventZ" }
    fn new(_: MtNoParticipants, _: MtNoParameters) -> Self { EventZ { z: 0 } }
    fn participants(&self) -> &MtNoParticipants { unimplemented!() }
    fn participants_mut(&mut self) -> &mut MtNoParticipants { unimplemented!() }
    fn parameters(&self) -> &MtNoParameters { unimplemented!() }
    fn parameters_mut(&mut self) -> &mut MtNoParameters { unimplemented!() }
}

fn register_all() {
    register_event::<EventX>(60);
    register_event::<EventY>(61);
    register_event::<EventZ>(62);
}

// ── Integration test #25 ─────────────────────────────────────────────────────

/// Three event types registered on the same dispatcher remain fully isolated.
/// Sending X events does not affect Y or Z slices, and vice versa.
#[test]
fn multi_type_isolation() {
    register_all();
    let mut d = EventDispatcher::new(1).unwrap();
    let cfg = EventConfig::new(1, 32).unwrap();
    d.preregister::<EventX>(cfg).unwrap();
    d.preregister::<EventY>(cfg).unwrap();
    d.preregister::<EventZ>(cfg).unwrap();

    // Send only X events.
    d.send(0, EventX { x: 1 }).unwrap();
    d.send(0, EventX { x: 2 }).unwrap();
    d.update_events();

    assert_eq!(d.events::<EventX>().len(), 2, "X has 2 events");
    assert_eq!(d.events::<EventY>().len(), 0, "Y untouched");
    assert_eq!(d.events::<EventZ>().len(), 0, "Z untouched");

    // Send Y and Z; send nothing for X this frame.
    d.send(0, EventY { y: 3.14 }).unwrap();
    d.send(0, EventZ { z: 7 }).unwrap();
    d.send(0, EventZ { z: 8 }).unwrap();
    d.update_events();

    assert_eq!(d.events::<EventX>().len(), 0, "X empty this frame");
    assert_eq!(d.events::<EventY>().len(), 1);
    assert_eq!(d.events::<EventZ>().len(), 2);

    // Validate Y content.
    assert!((d.events::<EventY>()[0].y - 3.14_f32).abs() < 1e-5);
    // Validate Z content.
    assert_eq!(d.events::<EventZ>()[0].z, 7);
    assert_eq!(d.events::<EventZ>()[1].z, 8);
}

/// Sending nothing for a registered type produces an empty slice.
#[test]
fn multi_type_empty_on_no_send() {
    register_all();
    let mut d = EventDispatcher::new(1).unwrap();
    let cfg = EventConfig::new(1, 8).unwrap();
    d.preregister::<EventX>(cfg).unwrap();
    d.preregister::<EventY>(cfg).unwrap();

    // Register and swap without sending anything.
    d.update_events();

    assert!(d.events::<EventX>().is_empty());
    assert!(d.events::<EventY>().is_empty());
}

/// Querying an unregistered type returns an empty slice without panicking.
#[test]
fn unregistered_type_returns_empty_slice() {
    register_all();
    let d = EventDispatcher::new(1).unwrap();
    // EventZ is not registered on this dispatcher.
    let slice: &[EventZ] = d.events::<EventZ>();
    assert!(slice.is_empty(), "unregistered type must return empty slice");
}

/// Different dispatchers (simulating different EcsMasters) are independent.
#[test]
fn two_dispatchers_are_independent() {
    register_all();
    let mut d1 = EventDispatcher::new(1).unwrap();
    let mut d2 = EventDispatcher::new(1).unwrap();
    let cfg = EventConfig::new(1, 16).unwrap();

    d1.preregister::<EventX>(cfg).unwrap();
    d2.preregister::<EventX>(cfg).unwrap();

    d1.send(0, EventX { x: 10 }).unwrap();
    d2.send(0, EventX { x: 20 }).unwrap();

    d1.update_events();
    d2.update_events();

    assert_eq!(d1.events::<EventX>()[0].x, 10);
    assert_eq!(d2.events::<EventX>()[0].x, 20);
    assert_eq!(d1.events::<EventX>().len(), 1);
    assert_eq!(d2.events::<EventX>().len(), 1);
}
