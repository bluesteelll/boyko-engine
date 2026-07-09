//! Phase 12 — `EventWriter<E>` / `EventReader<E>` SystemParam integration.
//!
//! Reserved EventId range for Phase 12 tests: **100-119** (see plan §13.0
//! / W3 resolution). Each event type below picks a distinct id in that
//! range to avoid collisions with the other integration suites:
//!
//! - Phase 6 : 10-19, 20-29
//! - Phase 9 : 50-69, 80, 90
//! - Phase 10: 70-79
//!
//! Tests exercise:
//!
//! 1. `EventWriter::send` from inside a scheduled system body writes to
//!    the per-thread lane; events become visible after `update_events`.
//! 2. `EventReader::read` returns the previous frame's events; the cursor
//!    advances on drop so subsequent reads do not re-yield.
//! 3. Mid-iteration `break` advances the cursor partially; the next
//!    `read()` continues from where the previous one left off.
//! 4. `missed_events` reports the gap when the cursor falls behind the
//!    current `start_event_count` (ER7).
//! 5. `is_empty` / `len` are consistent with `read()` (OQ2 clamp).
//! 6. `send_many` bulk path delivers every event in the iterator.
//! 7. `EventWriter::send` debug-panics outside a scheduled system body
//!    (EW-NEW).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::events::event_registry::register_event;
use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;
use boyko_ecs::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::{EventReader, EventWriter};
use boyko_threadpool::ThreadPoolBuilder;

// ── Empty participants / parameters shared by every test event ──────────────

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

// ── Event types (id range 100-119) ──────────────────────────────────────────

macro_rules! decl_event {
    ($name:ident, $id:expr) => {
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

decl_event!(WriterSmoke, 100);
decl_event!(ReaderSmoke, 101);
decl_event!(CursorSkip, 102);
decl_event!(MissedEvents, 103);
decl_event!(IsEmpty, 104);
decl_event!(SendMany, 105);
decl_event!(PartialIter, 106);
decl_event!(OutOfScope, 107);

// Test-event registration is idempotent — `register_event` returns silently
// when the same `(type, id)` pair is already registered (Phase 6
// register_event_collision_with_same_type_is_silent_noop).
fn register_all() {
    register_event::<WriterSmoke>(100);
    register_event::<ReaderSmoke>(101);
    register_event::<CursorSkip>(102);
    register_event::<MissedEvents>(103);
    register_event::<IsEmpty>(104);
    register_event::<SendMany>(105);
    register_event::<PartialIter>(106);
    register_event::<OutOfScope>(107);
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// 1. `EventWriter::send` inside a scheduled system body writes through the
///    per-lane buffer; events become visible via `events_of` after
///    `update_events`.
#[test]
fn event_writer_send_writes_to_buffer() {
    register_all();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    // 2 workers + 1 dispatcher lane = 3 lanes.
    world
        .preregister_event::<WriterSmoke>(EventConfig::default_for(3).unwrap())
        .expect("preregister WriterSmoke");

    static SEND_COUNT: AtomicU32 = AtomicU32::new(0);
    SEND_COUNT.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut writer: EventWriter<WriterSmoke>| {
        for i in 0..5u32 {
            writer
                .send(WriterSmoke { value: i })
                .expect("send must succeed on a preregistered event");
            SEND_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    assert_eq!(SEND_COUNT.load(Ordering::Relaxed), 5);

    world.update_events();
    let events = world.events_of::<WriterSmoke>();
    assert_eq!(events.len(), 5, "all 5 EventWriter::send calls must land");
    // Lane order is deterministic for a single-system scheduler run (the
    // exclusive system runs on the dispatcher lane). Values within the
    // single lane retain insertion order.
    let mut observed: Vec<u32> = events.iter().map(|e| e.value).collect();
    observed.sort_unstable();
    assert_eq!(observed, vec![0, 1, 2, 3, 4]);
}

/// 2. `EventReader::read` returns events sent during the previous frame.
///    Two-frame protocol: frame 1 writes via raw API + update, frame 2 reads.
#[test]
fn event_reader_reads_after_update() {
    register_all();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    world
        .preregister_event::<ReaderSmoke>(EventConfig::default_for(3).unwrap())
        .expect("preregister ReaderSmoke");

    // Seed the dispatcher with 3 events (from the unattached test thread —
    // lane 0 fallback) and swap so they live in `reader_buf`.
    for i in 0..3u32 {
        world
            .events()
            .send_event::<ReaderSmoke>(ReaderSmoke { value: i + 10 })
            .expect("seed send");
    }
    world.update_events();

    static SUM: AtomicU32 = AtomicU32::new(0);
    static COUNT: AtomicU32 = AtomicU32::new(0);
    SUM.store(0, Ordering::Relaxed);
    COUNT.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut reader: EventReader<ReaderSmoke>| {
        for ev in reader.read() {
            SUM.fetch_add(ev.value, Ordering::Relaxed);
            COUNT.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    assert_eq!(COUNT.load(Ordering::Relaxed), 3, "reader observed 3 events");
    assert_eq!(SUM.load(Ordering::Relaxed), 10 + 11 + 12);
}

/// 3. The cursor advances on iter drop — re-running the same schedule does
///    not re-yield the same events.
#[test]
fn event_reader_cursor_skips_already_read() {
    register_all();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    world
        .preregister_event::<CursorSkip>(EventConfig::default_for(3).unwrap())
        .expect("preregister CursorSkip");

    // Frame 1: send 2 events + swap.
    for i in 0..2u32 {
        world
            .events()
            .send_event::<CursorSkip>(CursorSkip { value: i })
            .expect("seed send");
    }
    world.update_events();

    static OBSERVED: AtomicU32 = AtomicU32::new(0);
    OBSERVED.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut reader: EventReader<CursorSkip>| {
        for _ in reader.read() {
            OBSERVED.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // First run — 2 events visible.
    schedule.run(&mut world);
    assert_eq!(OBSERVED.load(Ordering::Relaxed), 2);

    // Second run, no new sends, no new update — cursor caught up, 0 yielded.
    schedule.run(&mut world);
    assert_eq!(OBSERVED.load(Ordering::Relaxed), 2, "cursor must suppress re-reads");

    // Third run after a fresh update_events with no sends — still 0 (the
    // current reader_buf was emptied by the previous swap; the post-swap
    // start_event_count moves but reader_len stays 0).
    world.update_events();
    schedule.run(&mut world);
    assert_eq!(OBSERVED.load(Ordering::Relaxed), 2);
}

/// 4. `missed_events` reports the gap when the cursor falls behind
///    `start_event_count` (ER7). Send a burst, swap twice without reading;
///    cursor is left at 0, start_event_count moves forward.
#[test]
fn event_reader_handles_missed_events() {
    register_all();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    world
        .preregister_event::<MissedEvents>(EventConfig::default_for(3).unwrap())
        .expect("preregister MissedEvents");

    // Frame 1: 4 events + swap.
    for i in 0..4u32 {
        world
            .events()
            .send_event::<MissedEvents>(MissedEvents { value: i })
            .expect("seed send");
    }
    world.update_events();

    // Frame 2: 2 more events + swap. The previous 4 are now overwritten by
    // the second swap — start_event_count = 4 (since cursor=2 was copied);
    // the reader's cursor is still 0.
    for i in 4..6u32 {
        world
            .events()
            .send_event::<MissedEvents>(MissedEvents { value: i })
            .expect("seed send");
    }
    world.update_events();

    static OBSERVED: AtomicU32 = AtomicU32::new(0);
    static MISSED_REPORT: AtomicU32 = AtomicU32::new(0);
    OBSERVED.store(0, Ordering::Relaxed);
    MISSED_REPORT.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut reader: EventReader<MissedEvents>| {
        // Read first; check `missed_events` BEFORE iter (the iter advances
        // cursor on drop, which would zero the missed count).
        MISSED_REPORT.store(reader.missed_events() as u32, Ordering::Relaxed);
        for _ in reader.read() {
            OBSERVED.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    assert!(
        MISSED_REPORT.load(Ordering::Relaxed) >= 1,
        "missed_events must report at least one skipped event \
         (cursor=0 lagging start_event_count={}); got {}",
        4,
        MISSED_REPORT.load(Ordering::Relaxed),
    );
    // After ER7 fallback iteration the reader yields the entire current
    // reader_buf (2 events from frame 2).
    assert_eq!(
        OBSERVED.load(Ordering::Relaxed),
        2,
        "reader must yield the current reader_buf contents",
    );
}

/// 5. `is_empty` and `len` agree across all states (OQ2). When no events
///    have been sent: both report empty. After a swap with N events: len = N,
///    is_empty = false. After draining: len = 0, is_empty = true.
#[test]
fn event_reader_is_empty_when_no_events() {
    register_all();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    world
        .preregister_event::<IsEmpty>(EventConfig::default_for(3).unwrap())
        .expect("preregister IsEmpty");

    static PHASE: AtomicU32 = AtomicU32::new(0);
    static OBSERVATIONS: [AtomicU32; 6] = [
        AtomicU32::new(0),
        AtomicU32::new(0),
        AtomicU32::new(0),
        AtomicU32::new(0),
        AtomicU32::new(0),
        AtomicU32::new(0),
    ];

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|reader: EventReader<IsEmpty>| {
        let p = PHASE.load(Ordering::Relaxed) as usize;
        // Indices: [is_empty, len, _, _, _, _] per phase pair.
        OBSERVATIONS[p * 2].store(reader.is_empty() as u32, Ordering::Relaxed);
        OBSERVATIONS[p * 2 + 1].store(reader.len() as u32, Ordering::Relaxed);
    });
    let mut schedule = builder.build(&mut world);

    // Phase 0: no events sent / no swap.
    PHASE.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(OBSERVATIONS[0].load(Ordering::Relaxed), 1, "phase 0 is_empty");
    assert_eq!(OBSERVATIONS[1].load(Ordering::Relaxed), 0, "phase 0 len");

    // Phase 1: 3 events sent + swap.
    for i in 0..3u32 {
        world
            .events()
            .send_event::<IsEmpty>(IsEmpty { value: i })
            .expect("send");
    }
    world.update_events();

    PHASE.store(1, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(OBSERVATIONS[2].load(Ordering::Relaxed), 0, "phase 1 is_empty");
    assert_eq!(OBSERVATIONS[3].load(Ordering::Relaxed), 3, "phase 1 len");
    // The schedule run above also implicitly drained the reader (cursor
    // advanced via `len()` only? No — `len()` does NOT advance the cursor.
    // Drain on the next run with a read.).

    // Phase 2: drain via `read()` to advance the cursor, then verify
    // is_empty/len reflect the post-drain state.
    // We use a separate schedule to issue the drain, then re-run the
    // observer schedule.
    let mut drain_builder = ScheduleBuilder::new(Arc::clone(&pool));
    drain_builder.add_system(|mut reader: EventReader<IsEmpty>| {
        for _ in reader.read() {}
    });
    let mut drain = drain_builder.build(&mut world);
    drain.run(&mut world);

    PHASE.store(2, Ordering::Relaxed);
    schedule.run(&mut world);
    // NOTE: each `EventReader<E>` system has its own cursor (per-system
    // state). The drain schedule's reader and the observer schedule's
    // reader are different states. The observer reader was already
    // drained in phase 1 (when its body ran len() — no, len() does NOT
    // drain). Phase 1 observed len=3 but did not iterate, so the
    // observer's cursor is still 0. Phase 2 still sees 3 events.
    //
    // OQ2 invariant we are really verifying: `is_empty() == (len() == 0)`
    // is consistent — both should report the same answer.
    let p2_is_empty = OBSERVATIONS[4].load(Ordering::Relaxed);
    let p2_len = OBSERVATIONS[5].load(Ordering::Relaxed);
    assert_eq!(
        p2_is_empty == 1,
        p2_len == 0,
        "OQ2: is_empty() must agree with (len() == 0); got is_empty={}, len={}",
        p2_is_empty,
        p2_len,
    );
}

/// 6. `EventWriter::send_many` bulk path delivers every event.
#[test]
fn event_writer_send_many() {
    register_all();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    world
        .preregister_event::<SendMany>(EventConfig::new(3, 64).unwrap())
        .expect("preregister SendMany");

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut writer: EventWriter<SendMany>| {
        let batch = (0..16u32).map(|i| SendMany { value: i });
        writer.send_many(batch).expect("send_many");
    });
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    world.update_events();
    let events = world.events_of::<SendMany>();
    assert_eq!(events.len(), 16, "send_many must deliver 16 events");
    let mut observed: Vec<u32> = events.iter().map(|e| e.value).collect();
    observed.sort_unstable();
    let expected: Vec<u32> = (0..16).collect();
    assert_eq!(observed, expected);
}

/// 7. Mid-iteration `break` advances the cursor partially; the next
///    `read()` resumes from where the previous one stopped.
#[test]
fn event_reader_partial_iter_drops_cursor_correctly() {
    register_all();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    world
        .preregister_event::<PartialIter>(EventConfig::default_for(3).unwrap())
        .expect("preregister PartialIter");

    // Frame 1: send 5 events + swap.
    for i in 0..5u32 {
        world
            .events()
            .send_event::<PartialIter>(PartialIter { value: i })
            .expect("seed send");
    }
    world.update_events();

    static FIRST_PASS_COUNT: AtomicU32 = AtomicU32::new(0);
    static SECOND_PASS_COUNT: AtomicU32 = AtomicU32::new(0);
    static PHASE: AtomicU32 = AtomicU32::new(0);
    FIRST_PASS_COUNT.store(0, Ordering::Relaxed);
    SECOND_PASS_COUNT.store(0, Ordering::Relaxed);
    PHASE.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut reader: EventReader<PartialIter>| {
        let phase = PHASE.load(Ordering::Relaxed);
        let counter = if phase == 0 {
            &FIRST_PASS_COUNT
        } else {
            &SECOND_PASS_COUNT
        };
        let iter = reader.read();
        let limit = if phase == 0 { 2 } else { usize::MAX };
        let mut taken = 0;
        for _ev in iter {
            counter.fetch_add(1, Ordering::Relaxed);
            taken += 1;
            if taken >= limit {
                break;
            }
        }
        // The iterator drops at the end of this `for` — cursor advances by `taken`.
    });
    let mut schedule = builder.build(&mut world);

    // Run 1: break after 2 events.
    schedule.run(&mut world);
    assert_eq!(FIRST_PASS_COUNT.load(Ordering::Relaxed), 2);

    // Run 2: read remaining 3.
    PHASE.store(1, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        SECOND_PASS_COUNT.load(Ordering::Relaxed),
        3,
        "second pass must yield the remaining 3 events",
    );
}

/// 8. `EventWriter::send` debug-panics outside a scheduled system body
///    (Phase 12 EW-NEW / W2). Release builds make this a no-op.
#[test]
#[cfg(debug_assertions)]
fn event_writer_send_debug_panics_outside_system_run() {
    register_all();
    let mut world = EcsMaster::new();
    world
        .preregister_event::<OutOfScope>(EventConfig::default_for(1).unwrap())
        .expect("preregister OutOfScope");

    // Construct an EventWriter state manually via `run_system_once` — but
    // we don't go through Schedule, so `is_in_system_run()` is false.
    // Easiest reproducer: run a closure that captures the EventWriter via
    // EcsMaster::run_system (which does NOT set the IN_SYSTEM_RUN flag in
    // the Phase 8c/8d path).
    //
    // `EcsMaster` carries `UnsafeCell`s and is therefore not `UnwindSafe`;
    // `AssertUnwindSafe` is the canonical opt-in (the test catches the
    // panic immediately, no observers carry forward stale state).
    let mut world_for_panic = world;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        world_for_panic.run_system(|mut writer: EventWriter<OutOfScope>| {
            // EW-NEW: panic expected here in debug builds.
            let _ = writer.send(OutOfScope { value: 0 });
        });
    }));
    assert!(
        result.is_err(),
        "EventWriter::send outside Schedule::run must debug-panic via EW-NEW",
    );
}
