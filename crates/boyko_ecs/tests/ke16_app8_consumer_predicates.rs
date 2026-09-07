//! KE16 App-8 — the consumers of `is_in_system_run()` are BOOLEAN predicates, in BOTH polarities.
//!
//! ## The claim under test
//!
//! App-8 turned `InSystemRunGuard` from a `Cell<bool>` into a `Cell<u32>` depth counter
//! (`docs/threadpool/KE16-DESIGN-APP.md` §7) so that a helping joiner may run a sibling
//! conflict-free system INLINE inside another system's body. The design attached one obligation to
//! that change and stated it as already discharged:
//!
//! > Every consumer of `is_in_system_run()` is a boolean predicate (`time.rs:183`,
//! > `ecs_master.rs:677`, `event_writer.rs:113,157`, `event_reader.rs:111,176,192`,
//! > `profiling/store.rs:749`, `fold.rs:72`) and reads `depth > 0` unchanged.
//! > — `KE16-DESIGN-B.md` §2.5
//!
//! That sentence was verified by reading the sources. Reading proves what the eight sites say
//! TODAY; it does not keep them boolean through the next edit, and the two ways the predicate can
//! stop being boolean are both quiet:
//!
//! * `is_in_system_run()` becomes `depth == 1` (a plausible "the guard is entered once per body"
//!   simplification). Every POSITIVE consumer then refuses inside an inline sibling — the exact
//!   case App-8 exists to permit.
//! * `InSystemRunGuard::drop` stops decrementing, or decrements to a floor. Every NEGATIVE
//!   consumer is then armed for the rest of the process, and the frame driver's own clock advance
//!   is refused between frames.
//!
//! ## What this file adds over its siblings
//!
//! The consumer surface has two polarities and eight sites. Two of them were already gated when
//! this file was written, and both are POSITIVE-polarity `EventWriter` sites:
//! `tests/ke16_nested_system_events.rs` drives `EventWriter::send` at depth 2 and after the inner
//! guard drops; `tests/ke16_in_system_depth_balance.rs` drives `EcsMaster::delete_entity`'s
//! SAFETY-7 assertion at depth 0. Nothing exercised
//!
//! 1. the **`EventReader` family** (`read` / `len` / `is_empty`, three of the eight sites) at
//!    depth 2 — the sibling system that a helping joiner runs inline is as likely to READ events
//!    as to write them; and
//! 2. the **negative polarity at depth 2** — that the guards are still ARMED two deep, and
//!    DISARMED again once the nesting unwinds. `Time::advance_with` (`time/time.rs:183`) is this
//!    file's representative of that family: it is public, needs no world, and its assertion
//!    message is unique enough to pin.
//!
//! Every test here records the predicate's VALUE as well as relying on the `debug_assert!`, so
//! none of them passes silently in a release run where the assertions are compiled out — with the
//! one exception marked `#[cfg(debug_assertions)]`, which IS the assertion and says so.
//!
//! That one exception makes the file's test COUNT profile-dependent: `running 3 tests` in the
//! default (debug) profile, `running 2 tests` under `cargo test --release`. It is `#[cfg]`, not
//! `#[ignore]`, because in a release build the behaviour under test does not exist — a
//! `should_panic` test whose panic is compiled out is a gate that cannot fail. Compare names, not
//! counts (`KE16-DESIGN-MEASUREMENT.md` §5 shape 4).
//!
//! No test in this file claims that a joiner produced the nesting. It produces the SHAPE a joiner
//! produces, by entering a second [`InSystemRunGuard`] inside a running body — which is exactly
//! what `Schedule::run` brackets every system body in (`schedule.rs:1299`). The joiner-driven
//! receipt is `tests/ke16_nested_system_inline.rs`, and is `#[ignore]`d for the reason given
//! there.
//!
//! ## Instruments are per-pass
//!
//! libtest runs a binary's tests concurrently. The only cross-test state here is the process-wide
//! event registry, which is `OnceLock`-based and same-type idempotent
//! (`component_registry/mod.rs`), and the two event types are distinct. This file needs no
//! `--test-threads=1`. Event ids 132 and 133 are reserved for this binary (in use elsewhere in
//! `boyko_ecs`'s test corpus: 10-29, 50-70, 80, 90 Phase 9, 70-79 Phase 10, 100-119 Phase 12,
//! 130-131 `ke16_nested_system_events`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::events::event_registry::register_event;
use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;
use boyko_ecs::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::EventReader;
use boyko_ecs::ecs::core::time::Time;
use boyko_threadpool::{
    InSystemRunGuard, MAX_WORKERS, ThreadPoolBuilder, current_worker_id, is_in_system_run,
};

// ── Event stubs (the minimum `Event` surface; mirrors `ke16_nested_system_events.rs`) ─────────

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
        /// A non-ZST event carrying the seed order in `value`, so the reader can tell WHICH send
        /// produced which slot.
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

decl_event!(NestedReadEvent, 132);

/// Two is the smallest pool the parallel scheduler dispatches a concurrent body on, and this
/// file's assertions are about a body running on a WORKER lane.
const WORKERS: u32 = 2;

/// Seeded before the read, so the reader has something to fail to see.
const SEEDED: u32 = 3;

/// The variant banner, so a run of this file under `KE16_EXPECT` is attributable like the
/// measured artifacts are (`KE16-DESIGN.md` §4). Printing from a test binary is not the
/// `boyko-log` print census's subject — that census is about `crates/*/src/**`.
fn ke16_witness() {
    println!("KE16 variant: {}", boyko_threadpool::ke16_variant());
    boyko_threadpool::ke16_check_expected_variant();
}

// ── 1. The EventReader family, at guard depth 2 ───────────────────────────────────────────────

/// The `EventReader` half of the positive-polarity surface — three of the eight consumer sites —
/// read from inside an inline sibling's guard.
///
/// `read` (`event_reader.rs:111`), `is_empty` (`:176`) and `len` (`:192`) each carry
/// `debug_assert!(is_in_system_run())`. Under a predicate that answered `depth == 1` all three
/// would abort here in the default debug profile, and the recorded flag makes the test fail for
/// the same reason in a release run where the assertions are compiled out.
///
/// What could break this test other than the predicate: the seeding protocol (an event sent from
/// the unattached test thread lands in the lane-0 fallback and is swapped by `update_events`), and
/// the reader cursor. Both are gated independently by `tests/phase12_events_systemparam.rs`; a
/// failure HERE with that file green is the depth counter.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: spins a real OS thread pool and runs the parallel scheduler"
)]
fn event_reader_reads_every_event_at_guard_depth_two() {
    ke16_witness();
    register_event::<NestedReadEvent>(132);

    let pool = ThreadPoolBuilder::new().num_threads(WORKERS as usize).build();
    let mut world = EcsMaster::new();
    world
        .preregister_event::<NestedReadEvent>(EventConfig::default_for(WORKERS + 1).unwrap())
        .expect("invariant: preregister of a freshly registered event id must succeed");

    // Frame 1: seed from the test thread (lane-0 fallback) and swap, so the events live in
    // `reader_buf` before the reading system runs.
    for i in 0..SEEDED {
        world
            .events()
            .send_event::<NestedReadEvent>(NestedReadEvent { value: i })
            .expect("invariant: the event is preregistered and the lane buffer is not full");
    }
    world.update_events();

    let route = Arc::new(AtomicU32::new(u32::MAX));
    let in_system_at_depth_two = Arc::new(AtomicBool::new(false));
    let observed = Arc::new(AtomicU32::new(0));
    let reported_len = Arc::new(AtomicUsize::new(usize::MAX));
    let reported_empty = Arc::new(AtomicBool::new(true));

    let route_in = Arc::clone(&route);
    let depth_in = Arc::clone(&in_system_at_depth_two);
    let observed_in = Arc::clone(&observed);
    let len_in = Arc::clone(&reported_len);
    let empty_in = Arc::clone(&reported_empty);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(move |mut reader: EventReader<NestedReadEvent>| {
        route_in.store(current_worker_id(), Ordering::SeqCst);

        // The inline sibling: `Schedule::run` brackets every system body in exactly this guard
        // (`schedule.rs:1299`), so a body a helping joiner entered inside another body IS this
        // scope, two deep.
        let _inner = InSystemRunGuard::enter();
        depth_in.store(is_in_system_run(), Ordering::SeqCst);

        // All three reader sites, in the order a body would use them.
        empty_in.store(reader.is_empty(), Ordering::SeqCst);
        len_in.store(reader.len(), Ordering::SeqCst);
        for _ in reader.read() {
            observed_in.fetch_add(1, Ordering::SeqCst);
        }
    });
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let observed_route = route.load(Ordering::SeqCst);
    assert!(
        (observed_route as usize) < MAX_WORKERS,
        "the reading system body ran with worker id {observed_route} (>= MAX_WORKERS = \
         {MAX_WORKERS}), i.e. off-pool: this run measured the dispatcher lane, not the worker \
         lane the inline-sibling shape lives on"
    );
    assert!(
        in_system_at_depth_two.load(Ordering::SeqCst),
        "`is_in_system_run()` read FALSE two guards deep inside a system body: the predicate is \
         no longer `depth > 0`, and every positive-polarity consumer refuses inside an inline \
         sibling — which is the case App-8 exists to permit"
    );
    assert_eq!(
        observed.load(Ordering::SeqCst),
        SEEDED,
        "`EventReader::read` yielded the wrong number of events at guard depth 2"
    );
    assert_eq!(
        reported_len.load(Ordering::SeqCst),
        SEEDED as usize,
        "`EventReader::len` disagreed with the seeded count at guard depth 2"
    );
    assert!(
        !reported_empty.load(Ordering::SeqCst),
        "`EventReader::is_empty` reported empty at guard depth 2 with {SEEDED} events seeded"
    );
}

// ── 2. The negative polarity, at depth 2 and after the unwind ─────────────────────────────────

/// The negative-polarity family is ARMED two guards deep.
///
/// `Time::advance_with` is the frame driver's entry and refuses to run inside a system body
/// (`time/time.rs:183`); `Profiler::arm` (`profiling/store.rs:749`), `profiling::fold`
/// (`fold.rs:72`) and `EcsMaster::delete_entity` (`ecs_master.rs:677`) carry the same predicate
/// with the same polarity. If `is_in_system_run()` answered `depth == 1`, this call would be
/// PERMITTED two deep — an inline sibling could re-advance the clock mid-frame, and the assertion
/// that exists to catch exactly that would say nothing.
///
/// `#[cfg(debug_assertions)]` because the assertion IS the behaviour under test: in a release
/// build there is nothing here to observe, and a `should_panic` test that cannot panic is a gate
/// that cannot fail. The depth-2 predicate value itself is asserted in both profiles by
/// [`event_reader_reads_every_event_at_guard_depth_two`] above.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "Time::advance_with called inside a scheduled system body")]
fn time_advance_with_is_refused_at_guard_depth_two() {
    ke16_witness();
    let _outer = InSystemRunGuard::enter();
    let _inner = InSystemRunGuard::enter();

    let mut time = Time::default();
    time.advance_with(Duration::from_millis(16));
}

/// The negative-polarity family is DISARMED again once the nesting unwinds — the property that
/// makes the depth counter a counter rather than a latch.
///
/// A `drop` that stopped decrementing (or decremented to a floor of 1) would leave the frame
/// driver unable to advance its own clock for the rest of the process, and in a release build the
/// symptom would not be an assertion at all: it would be every context-restricted path in the
/// engine taking the in-system branch forever. So the predicate is asserted, not merely relied on.
#[test]
fn time_advance_with_is_permitted_again_after_the_depth_two_unwind() {
    ke16_witness();
    assert!(
        !is_in_system_run(),
        "the test thread already reads as inside a system body before this test entered a guard: \
         a previous test in this binary leaked one"
    );
    {
        let _outer = InSystemRunGuard::enter();
        let _inner = InSystemRunGuard::enter();
    }
    assert!(
        !is_in_system_run(),
        "two nested guards dropped and the depth did not return to zero: the counter is a latch, \
         and every negative-polarity consumer (Time::advance_with, Profiler::arm, \
         profiling::fold, EcsMaster::delete_entity) stays armed for the rest of the process"
    );

    // The consumer itself, not just the predicate: in a debug build this is the live
    // `debug_assert!` that the previous test proves is armed at depth 2.
    let mut time = Time::default();
    time.advance_with(Duration::from_millis(16));
    assert_eq!(
        time.delta(),
        Duration::from_millis(16),
        "the clock did not advance after the nesting unwound"
    );
}
