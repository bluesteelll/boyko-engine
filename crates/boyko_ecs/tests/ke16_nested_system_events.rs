//! KE16 App-8 — what a nested system body does to the EVENT lane.
//!
//! ## The claim under test
//!
//! App-8 turned `InSystemRunGuard` into a depth counter because a helping joiner may run a
//! sibling conflict-free system INLINE inside another system's body
//! (`docs/threadpool/KE16-DESIGN-APP.md` §7). The design attached one behavioural consequence to
//! that change and wrote it into EVT1's doc comment
//! (`src/ecs/core/events/event_dispatcher.rs:274-283`, `KE16-DESIGN.md` §7):
//!
//! > a sibling system run inline by a helping joiner appends to the SAME lane sequentially;
//! > events of the two systems interleave within the lane; no reader relies on per-system
//! > contiguity.
//!
//! That sentence is a claim about observable behaviour and it had no test. This file is that
//! test. It is deterministic: rather than waiting for a joiner to take a sibling task — a race
//! the design itself declines to gate on (`tests/ke16_nested_system_inline.rs` §3) — it produces
//! the *shape* the joiner produces, by entering a second [`InSystemRunGuard`] inside a running
//! system body, which is exactly what `Schedule::run` brackets every system body in
//! (`schedule.rs:1299`).
//!
//! ## What each test gates
//!
//! 1. [`nested_system_events_interleave_in_one_lane_and_all_are_readable`] — the EVT1 sentence
//!    itself: every event of both depths reaches the reader, in write order, with the outer
//!    system's events on both sides of the inner system's. A reader that assumed per-system
//!    contiguity would be reading the inner system's events as the outer's.
//! 2. [`event_writer_send_after_the_nested_guard_drops_is_still_inside_a_system_body`] — the
//!    regression the depth counter exists to prevent, through a real consumer. `EventWriter::send`
//!    carries `debug_assert!(is_in_system_run())` (`system/params/event_writer.rs:112-116`), so a
//!    guard whose `drop` CLEARED the flag instead of decrementing the depth would abort this test
//!    in a debug build — which is the profile `cargo test` uses by default. The predicate's value
//!    is also recorded and asserted, so the test still fails (rather than passing silently) in a
//!    release run where the `debug_assert!` is compiled out.
//!
//! Neither test claims that a joiner actually produced the nesting; that receipt lives in
//! `tests/ke16_nested_system_inline.rs` and is `#[ignore]`d for the reason given there.
//!
//! ## Instruments are per-pass
//!
//! libtest runs a binary's tests concurrently by default. Every counter here is an `Arc` created
//! by the test that reads it and moved into its own system, and the two tests use DISTINCT event
//! types, so this file needs no `--test-threads=1`.
//!
//! Event ids 130 and 131 are reserved for this binary (in use elsewhere: 10-29 Phase 6, 50-70
//! and 80, 90 Phase 9, 70-79 Phase 10, 100-119 Phase 12).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::events::event_registry::register_event;
use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;
use boyko_ecs::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::EventWriter;
use boyko_threadpool::{
    InSystemRunGuard, MAX_WORKERS, ThreadPoolBuilder, current_worker_id, is_in_system_run,
};

// ── Event stubs ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct NoParticipants;
impl Participants for NoParticipants {
    fn participant_count() -> usize {
        0
    }
    fn participant_info() -> &'static [ParticipantInfo] {
        &[]
    }
}

#[derive(Clone, Copy)]
struct NoParameters;
impl Parameters for NoParameters {}

macro_rules! decl_event {
    ($name:ident, $id:expr) => {
        /// A non-ZST event carrying the write order in `value`, so the reader can tell WHICH
        /// send produced which slot.
        #[derive(Clone, Copy, Debug, PartialEq)]
        struct $name {
            value: u32,
        }

        impl Event for $name {
            type Participants = NoParticipants;
            type Parameters = NoParameters;
            fn event_id() -> u64 {
                $id
            }
            fn event_name() -> &'static str {
                stringify!($name)
            }
            fn new(_: NoParticipants, _: NoParameters) -> Self {
                $name { value: 0 }
            }
            fn participants(&self) -> &NoParticipants {
                unimplemented!()
            }
            fn participants_mut(&mut self) -> &mut NoParticipants {
                unimplemented!()
            }
            fn parameters(&self) -> &NoParameters {
                unimplemented!()
            }
            fn parameters_mut(&mut self) -> &mut NoParameters {
                unimplemented!()
            }
        }
    };
}

decl_event!(NestedLaneEvent, 130);
decl_event!(AfterDropEvent, 131);

/// Workers. Two is the smallest pool the parallel scheduler dispatches concurrent bodies on.
const WORKERS: u32 = 2;

// ── Tests ───────────────────────────────────────────────────────────────────

/// EVT1 under App-8: the outer body's events and the inline sibling's events share ONE lane and
/// arrive interleaved, and every one of them is readable.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: spins a real OS thread pool and runs the parallel scheduler"
)]
fn nested_system_events_interleave_in_one_lane_and_all_are_readable() {
    register_event::<NestedLaneEvent>(130);

    let pool = ThreadPoolBuilder::new().num_threads(WORKERS as usize).build();
    let mut world = EcsMaster::new();
    world
        .preregister_event::<NestedLaneEvent>(EventConfig::default_for(WORKERS + 1).unwrap())
        .expect("invariant: preregister of a freshly registered event id must succeed");

    // The lane the body wrote on, recorded as the route receipt: a body that ran on the
    // dispatcher would write the RESERVED dispatcher lane, not a worker lane, and the sentence
    // this test gates is about a worker's lane.
    let route = Arc::new(AtomicU32::new(u32::MAX));
    let route_in = Arc::clone(&route);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(move |mut writer: EventWriter<NestedLaneEvent>| {
        route_in.store(current_worker_id(), Ordering::SeqCst);

        let send = |w: &mut EventWriter<NestedLaneEvent>, value: u32| {
            w.send(NestedLaneEvent { value })
                .expect("invariant: the event is preregistered and the lane buffer is not full");
        };

        // The outer system's first event.
        send(&mut writer, 1);
        {
            // The inline sibling: `Schedule::run` brackets every system body in exactly this
            // guard, so a body entered by a helping joiner is this scope.
            let _inner = InSystemRunGuard::enter();
            send(&mut writer, 100);
            send(&mut writer, 101);
        }
        // The outer system continues after the sibling retired — the second half of the
        // interleaving.
        send(&mut writer, 2);
    });
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let observed_route = route.load(Ordering::SeqCst);
    assert!(
        (observed_route as usize) < MAX_WORKERS,
        "the writing system body ran with worker id {observed_route} (>= MAX_WORKERS = \
         {MAX_WORKERS}), i.e. off-pool: its events went to the reserved dispatcher lane, and \
         this test would be gating the dispatcher's lane instead of a worker's"
    );

    world.update_events();
    let events = world.events_of::<NestedLaneEvent>();
    let values: Vec<u32> = events.iter().map(|e| e.value).collect();

    assert_eq!(
        values,
        vec![1, 100, 101, 2],
        "the four sends of the outer body and its inline sibling must arrive in write order. \
         Only one lane was written (one body, one thread), so the reader's lane-flattened order \
         IS that lane's order — and it shows the sibling's events (100, 101) BETWEEN the outer \
         system's (1, 2). That interleaving is EVT1's documented consequence of App-8: a reader \
         that inferred per-system grouping from lane order would attribute 100 and 101 to the \
         outer system"
    );
}

/// The depth counter, through a real consumer: after the inline sibling's guard drops, the outer
/// body is STILL inside a system run, so `EventWriter::send` keeps working.
///
/// Under a guard whose `drop` cleared a flag (the pre-App-8 `Cell<bool>` shape, had it been
/// extended to permit nesting) this send would trip
/// `debug_assert!(is_in_system_run(), "EventWriter::send called outside a scheduled system body")`
/// in the default debug profile. The recorded predicate makes the test fail for the same reason
/// in a release run, where the assertion is compiled out.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: spins a real OS thread pool and runs the parallel scheduler"
)]
fn event_writer_send_after_the_nested_guard_drops_is_still_inside_a_system_body() {
    register_event::<AfterDropEvent>(131);

    let pool = ThreadPoolBuilder::new().num_threads(WORKERS as usize).build();
    let mut world = EcsMaster::new();
    world
        .preregister_event::<AfterDropEvent>(EventConfig::default_for(WORKERS + 1).unwrap())
        .expect("invariant: preregister of a freshly registered event id must succeed");

    let in_system_after_drop = Arc::new(AtomicBool::new(false));
    let recorded = Arc::clone(&in_system_after_drop);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(move |mut writer: EventWriter<AfterDropEvent>| {
        {
            let _inner = InSystemRunGuard::enter();
        }
        recorded.store(is_in_system_run(), Ordering::SeqCst);
        writer
            .send(AfterDropEvent { value: 7 })
            .expect("invariant: the event is preregistered and the lane buffer is not full");
    });
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    assert!(
        in_system_after_drop.load(Ordering::SeqCst),
        "`is_in_system_run()` read FALSE inside a system body after an inline sibling's guard \
         dropped: the guard is clearing the state instead of decrementing the depth, so every \
         context-restricted consumer (EventWriter::send, EventReader::read, Commands, Time) \
         would debug-panic in the remainder of the outer body"
    );

    world.update_events();
    let events = world.events_of::<AfterDropEvent>();
    assert_eq!(
        events.len(),
        1,
        "the send issued after the inner guard dropped did not reach the reader"
    );
}
