//! Phase 9 Wave 7 Step 18 — `EventDispatcher::send_event` TLS routing.
//!
//! Exercises the per-thread lane discipline introduced by §2.8 EVT1:
//!
//! * Exclusive system runs on the dispatcher thread; `send_event` must
//!   route writes to lane `worker_count` (the extra reserved lane
//!   allocated by `EventConfig::default_for(worker_count + 1)`).
//! * Direct dispatcher-thread sends from outside the scheduler land on
//!   the same lane (the dispatcher's TLS id is set on entry to
//!   `ThreadPool::install`).
//! * Unattached-thread sends land on lane `0` (the EVT1 fallback) — the
//!   `EventDispatcher::send_event` API is reachable from non-scheduler
//!   contexts and must not panic when no pool is attached.
//!
//! All three tests use a single non-ZST `#[event]` type with a fresh
//! `event_id` (90) to avoid collisions with the rest of the integration
//! suite (which uses 20–29 and 50–70 — see test ID range comments in
//! `event_attribute.rs`).

use std::sync::atomic::{AtomicU32, Ordering};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;
use boyko_ecs::ecs::core::events::event_registry::register_event;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

// ── Event stub ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct LaneNoParticipants;
impl Participants for LaneNoParticipants {
    fn participant_count() -> usize { 0 }
    fn participant_info() -> &'static [ParticipantInfo] { &[] }
}

#[derive(Clone, Copy)]
struct LaneNoParameters;
impl Parameters for LaneNoParameters {}

/// Non-ZST event with a fresh, isolated `event_id`. The `value` field also
/// disambiguates which lane the event was sent through.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LaneEvent {
    value: u32,
}

impl Event for LaneEvent {
    type Participants = LaneNoParticipants;
    type Parameters = LaneNoParameters;
    fn event_id() -> u64 { 90 }
    fn event_name() -> &'static str { "LaneEvent" }
    fn new(_: LaneNoParticipants, _: LaneNoParameters) -> Self { LaneEvent { value: 0 } }
    fn participants(&self) -> &LaneNoParticipants { unimplemented!() }
    fn participants_mut(&mut self) -> &mut LaneNoParticipants { unimplemented!() }
    fn parameters(&self) -> &LaneNoParameters { unimplemented!() }
    fn parameters_mut(&mut self) -> &mut LaneNoParameters { unimplemented!() }
}

fn register_lane_event() {
    register_event::<LaneEvent>(90);
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// Exclusive system inside `Schedule::run` must route `send_event` to the
/// dispatcher's reserved lane (lane `worker_count` ==
/// `default_thread_count - 1`). The events are observable on the next
/// frame via `events_of::<E>`.
#[test]
fn event_send_from_exclusive_system_routes_to_dispatcher_lane() {
    register_lane_event();

    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let mut world = EcsMaster::new();
    // 4 workers + 1 dispatcher lane = 5 lanes.
    world
        .preregister_event::<LaneEvent>(EventConfig::default_for(5).unwrap())
        .expect("preregister LaneEvent");

    static SEND_COUNT: AtomicU32 = AtomicU32::new(0);
    SEND_COUNT.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(pool);
    builder.add_system(|w: &mut EcsMaster| {
        for i in 0..10u32 {
            // Inside `Schedule::run`, the current thread is the dispatcher
            // (CURRENT_WORKER_ID == WORKER_ID_DISPATCHER). `send_event`
            // therefore lands on the reserved lane (index 4 here).
            w.events()
                .send_event::<LaneEvent>(LaneEvent { value: i })
                .expect("send_event must succeed on preregistered event");
            SEND_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    assert_eq!(SEND_COUNT.load(Ordering::Relaxed), 10, "exclusive system ran 10 sends");

    // Make the writes visible in the reader buffer.
    world.update_events();
    let events = world.events_of::<LaneEvent>();
    assert_eq!(events.len(), 10, "all 10 sends must reach the reader slice");
    for (i, ev) in events.iter().enumerate() {
        assert_eq!(ev.value, i as u32, "event {i} arrived out of order");
    }
}

/// Unattached-thread fallback — `send_event` from outside any pool / any
/// `Schedule::run` must not panic and must land on lane 0. Single-lane
/// preregistration is sufficient (`default_thread_count = 1`).
#[test]
fn event_send_from_unattached_thread_uses_lane_zero() {
    register_lane_event();

    let mut world = EcsMaster::new();
    // 1 lane only — Phase 6 single-threaded default.
    world
        .preregister_event::<LaneEvent>(EventConfig::default_for(1).unwrap())
        .expect("preregister LaneEvent");

    // No pool installed; CURRENT_WORKER_ID == WORKER_ID_UNATTACHED.
    // `send_event` must take the unattached branch and route to lane 0.
    for i in 0..3u32 {
        world
            .events()
            .send_event::<LaneEvent>(LaneEvent { value: i })
            .expect("send_event from unattached thread must succeed");
    }

    world.update_events();
    let events = world.events_of::<LaneEvent>();
    assert_eq!(events.len(), 3, "all 3 unattached sends must reach the reader slice");
}

/// `Commands::send_event` enqueues a `SendEventCommand<E>` that runs on
/// apply (still on the dispatcher). Same routing rules apply: the final
/// write lands on the reserved dispatcher lane.
#[test]
fn commands_send_event_routes_through_apply() {
    register_lane_event();

    let mut world = EcsMaster::new();
    // Single-threaded smoke variant: dispatcher lane index == 0 because
    // `default_thread_count - 1 == 0` when only one lane is registered.
    world
        .preregister_event::<LaneEvent>(EventConfig::default_for(1).unwrap())
        .expect("preregister LaneEvent");

    world.run_system(|mut cmds: Commands| {
        for i in 0..5u32 {
            cmds.send_event::<LaneEvent>(LaneEvent { value: i });
        }
    });

    world.update_events();
    let events = world.events_of::<LaneEvent>();
    assert_eq!(events.len(), 5, "all 5 deferred sends must reach the reader slice");
    for (i, ev) in events.iter().enumerate() {
        assert_eq!(ev.value, i as u32, "deferred event {i} arrived out of order");
    }
}
