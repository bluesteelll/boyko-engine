/// Integration tests for Phase 6: event double-buffer (Model B) semantics.
///
/// Tests #24 from the plan. Validates that events sent during frame N are
/// visible only after `update_events` (beginning of frame N+1), not during
/// frame N itself.
use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::events::event_dispatcher::EventDispatcher;
use boyko_ecs::ecs::core::events::event_registry::register_event;
use boyko_ecs::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;

// ── Minimal event stubs ───────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct DbNoParticipants;
impl Participants for DbNoParticipants {
    fn participant_count() -> usize { 0 }
    fn participant_info() -> &'static [ParticipantInfo] { &[] }
}

#[derive(Clone, Copy)]
struct DbNoParameters;
impl Parameters for DbNoParameters {}

/// Integer payload event. Uses ID range 50-59 for this test file.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScoreEvent {
    score: i64,
}

impl Event for ScoreEvent {
    type Participants = DbNoParticipants;
    type Parameters = DbNoParameters;
    fn event_id() -> u64 { 50 }
    fn event_name() -> &'static str { "ScoreEvent" }
    fn new(_: DbNoParticipants, _: DbNoParameters) -> Self { ScoreEvent { score: 0 } }
    fn participants(&self) -> &DbNoParticipants { unimplemented!() }
    fn participants_mut(&mut self) -> &mut DbNoParticipants { unimplemented!() }
    fn parameters(&self) -> &DbNoParameters { unimplemented!() }
    fn parameters_mut(&mut self) -> &mut DbNoParameters { unimplemented!() }
}

fn register_score_event() {
    register_event::<ScoreEvent>(50);
}

// ── Integration test #24 ─────────────────────────────────────────────────────

/// Events sent in frame N are not visible until after `update_events`.
/// Validates Model B next-frame visibility guarantee.
#[test]
fn double_buffer_next_frame_visibility() {
    register_score_event();
    let mut d = EventDispatcher::new(1).unwrap();
    d.preregister::<ScoreEvent>(EventConfig::new(1, 16).unwrap()).unwrap();

    // Frame 0: no events sent — reader slice must be empty.
    assert!(d.events::<ScoreEvent>().is_empty(), "empty before any swap");

    // Frame 1 send phase.
    d.send(0, ScoreEvent { score: 100 }).unwrap();
    d.send(0, ScoreEvent { score: 200 }).unwrap();

    // Events not yet visible (no update_events called).
    assert!(d.events::<ScoreEvent>().is_empty(), "not visible before swap");

    // Frame 1 end: flush.
    d.update_events();

    // Frame 2: events from frame 1 are now visible.
    let visible = d.events::<ScoreEvent>();
    assert_eq!(visible.len(), 2, "both events visible after swap");
    assert_eq!(visible[0].score, 100);
    assert_eq!(visible[1].score, 200);

    // Frame 2 send phase: new events not yet visible.
    d.send(0, ScoreEvent { score: 300 }).unwrap();
    let still_frame1 = d.events::<ScoreEvent>();
    assert_eq!(still_frame1.len(), 2, "frame 1 events still visible during frame 2 send");
    assert_eq!(still_frame1[0].score, 100);

    // Frame 2 end.
    d.update_events();

    // Frame 3: only frame 2 events visible.
    let visible = d.events::<ScoreEvent>();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].score, 300);

    // Frame 3: no new sends.
    d.update_events();

    // Frame 4: empty reader — previous frame sent nothing.
    assert!(d.events::<ScoreEvent>().is_empty(), "no events from empty frame");
}

/// Sending the same event type across multiple frames produces independent slices.
#[test]
fn double_buffer_independent_frames() {
    register_score_event();
    let mut d = EventDispatcher::new(1).unwrap();
    d.preregister::<ScoreEvent>(EventConfig::new(1, 32).unwrap()).unwrap();

    for frame in 0u32..5 {
        let val = (frame + 1) as i64 * 10;
        d.send(0, ScoreEvent { score: val }).unwrap();
        d.update_events();
        let evs = d.events::<ScoreEvent>();
        assert_eq!(evs.len(), 1, "exactly one event per frame");
        assert_eq!(evs[0].score, val, "score matches frame {frame}");
    }
}

/// `send_many` delivers all events as a contiguous reader slice.
#[test]
fn double_buffer_send_many_contiguous() {
    register_score_event();
    let mut d = EventDispatcher::new(1).unwrap();
    d.preregister::<ScoreEvent>(EventConfig::new(1, 64).unwrap()).unwrap();

    let batch: Vec<ScoreEvent> = (0..10).map(|i| ScoreEvent { score: i as i64 }).collect();
    d.send_many(0, batch.into_iter()).unwrap();
    d.update_events();

    let evs = d.events::<ScoreEvent>();
    assert_eq!(evs.len(), 10);
    for (i, ev) in evs.iter().enumerate() {
        assert_eq!(ev.score, i as i64);
    }
}
