/// Property-based tests for Phase 6 event dispatch.
///
/// Tests #15 and #16 from the plan:
/// - #15: send_then_read_roundtrip — events sent equal events read after swap.
/// - #16: overflow_count_matches_rejections — overflow counter matches reject count.
use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::events::event_dispatcher::EventDispatcher;
use boyko_ecs::ecs::core::events::event_registry::register_event;
use boyko_ecs::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;
use proptest::prelude::*;

// ── Minimal event stubs ───────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct PtNoParticipants;
impl Participants for PtNoParticipants {
    fn participant_count() -> usize { 0 }
    fn participant_info() -> &'static [ParticipantInfo] { &[] }
}
#[derive(Clone, Copy)]
struct PtNoParameters;
impl Parameters for PtNoParameters {}

/// U32 event. ID range 70-79.
#[derive(Clone, Copy, Debug, PartialEq)]
struct U32Event { val: u32 }
impl Event for U32Event {
    type Participants = PtNoParticipants;
    type Parameters = PtNoParameters;
    fn event_id() -> u64 { 70 }
    fn event_name() -> &'static str { "U32Event" }
    fn new(_: PtNoParticipants, _: PtNoParameters) -> Self { U32Event { val: 0 } }
    fn participants(&self) -> &PtNoParticipants { unimplemented!() }
    fn participants_mut(&mut self) -> &mut PtNoParticipants { unimplemented!() }
    fn parameters(&self) -> &PtNoParameters { unimplemented!() }
    fn parameters_mut(&mut self) -> &mut PtNoParameters { unimplemented!() }
}

fn ensure_registered() {
    register_event::<U32Event>(70);
}

// ── Property-based tests ──────────────────────────────────────────────────────

proptest! {
    /// Test #15: events sent round-trip correctly through the double buffer.
    ///
    /// For any sequence of values (up to capacity), all sent values appear
    /// in the reader slice after `update_events`, in the same order.
    #[test]
    fn send_then_read_roundtrip(values in prop::collection::vec(any::<u32>(), 0..32usize)) {
        ensure_registered();
        let mut d = EventDispatcher::new(1).unwrap();
        d.preregister::<U32Event>(EventConfig::new(1, 32).unwrap()).unwrap();

        for &v in &values {
            d.send(0, U32Event { val: v }).unwrap();
        }
        d.update_events();

        let evs = d.events::<U32Event>();
        prop_assert_eq!(evs.len(), values.len());
        for (ev, &expected) in evs.iter().zip(values.iter()) {
            prop_assert_eq!(ev.val, expected);
        }
    }

    /// Test #16: overflow count matches the number of rejected sends.
    ///
    /// For a capacity of `cap`, sending `cap + extra` events must result in
    /// exactly `extra` rejections and `cap` events in the reader slice after
    /// swap.
    #[test]
    fn overflow_count_matches_rejections(
        cap in 1u32..=16u32,
        extra in 1u32..=8u32,
    ) {
        ensure_registered();
        let mut d = EventDispatcher::new(1).unwrap();
        d.preregister::<U32Event>(EventConfig::new(1, cap).unwrap()).unwrap();

        let mut accepted = 0u32;
        let mut rejected = 0u32;
        for i in 0..(cap + extra) {
            match d.send(0, U32Event { val: i }) {
                Ok(()) => accepted += 1,
                Err(_) => rejected += 1,
            }
        }

        prop_assert_eq!(accepted, cap, "exactly cap events accepted");
        prop_assert_eq!(rejected, extra, "exactly extra events rejected");

        d.update_events();
        prop_assert_eq!(d.events::<U32Event>().len(), cap as usize);
    }
}
