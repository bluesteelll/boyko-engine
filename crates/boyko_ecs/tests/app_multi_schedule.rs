//! Phase 20 — multi-schedule semantics: change detection across Main/Fixed,
//! the D6 event-swap gate (plan P20-B5, ★M1 pause-hold, ★m4 cold start), and
//! states in a two-schedule world (D7).
//!
//! Reserved EventId range for Phase 20 tests: **130-139** (the suites before
//! us hold 10-29, 50-69, 70-79, 80, 90, 100-119).
//!
//! Harness discipline matches `app_plugin.rs`: per-test `Arc<Atomic*>`
//! counters, single-worker pools (1 worker + dispatcher ⇒ 2 event lanes),
//! scripted deltas only.

#![cfg(not(miri))]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::event_registry::register_event;
use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;
use boyko_ecs::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use boyko_ecs::prelude::*;
use boyko_macros::{Bundle, Component, Resource};

/// One 64 Hz step exactly.
const STEP: Duration = Duration::from_nanos(15_625_000);
/// A delta too small to ever produce a substep on its own at 64 Hz... is NOT
/// trivially true (overstep accumulates) — 1 µs keeps accumulation negligible
/// across any script length used here.
const TINY: Duration = Duration::from_micros(1);

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

fn counter() -> Arc<AtomicU32> {
    Arc::new(AtomicU32::new(0))
}

// ── Event plumbing (manual impls — the `#[event]` macro path is exercised by
//    the Phase 6/12 suites; ids 130-132 reserved above) ──────────────────────

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

decl_event!(HoldEvt, 130);
decl_event!(PauseEvt, 131);
decl_event!(FrameEvt, 132);
decl_event!(ColdEvt, 133);

// ── Components / resources ───────────────────────────────────────────────────

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Wobble(u32);

/// 1-field bundle newtype — `spawn_batch`/`Commands::spawn` take a `Bundle`
/// (the Phase-19 single-component-bundle pattern).
#[derive(Bundle)]
struct WobbleBundle {
    w: Wobble,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct SpawnedMark(u32);

#[derive(Bundle)]
struct MarkBundle {
    m: SpawnedMark,
}

#[derive(Resource, Default)]
struct SendBudget(u32);

// =============================================================================
// Change detection across schedules (plan §Test matrix, the per-run #56 form)
// =============================================================================

/// A Main `Changed<Wobble>` query sees N fixed-substep mutations of one frame
/// exactly ONCE (the window spans all substeps since Main's previous run).
#[test]
fn main_changed_sees_substep_mutations_once() {
    let main_hits = counter();
    let mh = Arc::clone(&main_hits);

    let mut app = App::with_pool(serial_pool());
    app.world_mut()
        .spawn_batch(std::iter::once(WobbleBundle { w: Wobble(0) }))
        .expect("spawn");
    // Fixed: mutate every substep THROUGH `Mut<T>` — the stamping write form
    // (plain `&mut T` deliberately elides tick stamping, Phase 12.5 NCD).
    app.add_systems_in(
        CoreSchedule::Fixed,
        |mut q: Query<boyko_ecs::ecs::core::iters::query::Mut<Wobble>>| {
            for mut w in q.iter_mut() {
                w.0 = w.0.wrapping_add(1);
            }
        },
    );
    // Main: count Changed matches.
    app.add_systems(move |q: Query<&Wobble, boyko_ecs::ecs::core::iters::query::Changed<Wobble>>| {
        mh.fetch_add(q.iter().count() as u32, Ordering::Relaxed);
    });

    // Frame 1: 3 substeps (3 × STEP). Main sees the entity once (spawn +
    // mutations fold into one window).
    app.update_with_delta(STEP * 3);
    assert_eq!(main_hits.load(Ordering::Relaxed), 1, "frame 1: one match, not 3");

    // Frame 2: 3 more substeps ⇒ exactly one more match.
    app.update_with_delta(STEP * 3);
    assert_eq!(main_hits.load(Ordering::Relaxed), 2, "frame 2: substep mutations fold to one");

    // Frame 3: 0 substeps ⇒ no mutation ⇒ no new match.
    app.update_with_delta(TINY);
    assert_eq!(main_hits.load(Ordering::Relaxed), 2, "0-substep frame adds nothing");
}

/// `Added<T>` from a deferred Commands spawn inside a Fixed substep becomes
/// visible to a Fixed reader on the NEXT SUBSTEP (the per-run #56 contract —
/// "next run of that schedule", which inside one frame is the next substep).
#[test]
fn fixed_added_from_commands_visible_next_substep() {
    let spawner_fired = counter();
    let sf = Arc::clone(&spawner_fired);
    let added_seen = counter();
    let asn = Arc::clone(&added_seen);

    let mut app = App::with_pool(serial_pool());
    app.add_systems_cfg_in(CoreSchedule::Fixed, |b| {
        b.add_system(move |mut cmds: Commands| {
            // Spawn exactly once, on the first substep.
            if sf.fetch_add(1, Ordering::Relaxed) == 0 {
                cmds.spawn(MarkBundle { m: SpawnedMark(7) });
            }
        });
        let asn2 = Arc::clone(&asn);
        b.add_system(
            move |q: Query<&SpawnedMark, boyko_ecs::ecs::core::iters::query::Added<SpawnedMark>>| {
                asn2.fetch_add(q.iter().count() as u32, Ordering::Relaxed);
            },
        );
    });

    // One frame, 3 substeps: spawn applies at substep 1's apply window;
    // the Added window matches at substep 2 exactly once; substep 3 is past it.
    app.update_with_delta(STEP * 3);
    assert_eq!(spawner_fired.load(Ordering::Relaxed), 3, "spawner ran every substep");
    assert_eq!(added_seen.load(Ordering::Relaxed), 1, "Added seen exactly once, next substep");
}

// =============================================================================
// P20-B5 — the D6 event gate (WaitForFixed default with a Fixed schedule)
// =============================================================================

/// Builds the canonical sender/receiver app: a Main system sends `$evt {
/// value }` once per frame while `SendBudget > 0`; a Fixed system reads and
/// accumulates `value`. Returns (app, received-sum counter, received-count).
fn gated_event_app<E: Event + HasValue + Send + Sync + 'static>(
    make: fn(u32) -> E,
) -> (App, Arc<AtomicU32>, Arc<AtomicU32>) {
    let sum = counter();
    let cnt = counter();
    let s = Arc::clone(&sum);
    let c = Arc::clone(&cnt);

    let mut app = App::with_pool(serial_pool());
    // Lane count derived from the pool by the App wiring (1 worker +
    // dispatcher ⇒ 2 lanes) — never hand-coded, so a pool-width change
    // cannot silently under-provision the buffer.
    app.world_mut()
        .preregister_event_default::<E>()
        .expect("preregister");
    app.world_mut().insert_resource(SendBudget(0));

    let seq = Arc::new(AtomicU32::new(1));
    app.add_systems(move |mut budget: ResMut<SendBudget>, mut w: EventWriter<E>| {
        if budget.0 > 0 {
            budget.0 -= 1;
            let v = seq.fetch_add(1, Ordering::Relaxed);
            w.send(make(v)).expect("send within lane capacity");
        }
    });
    app.add_systems_in(CoreSchedule::Fixed, move |mut r: EventReader<E>| {
        for e in r.read() {
            s.fetch_add(event_value(e), Ordering::Relaxed);
            c.fetch_add(1, Ordering::Relaxed);
        }
    });
    (app, sum, cnt)
}

/// Extracts the `value` payload (each decl_event type has the same shape; a
/// tiny trait avoids four copies of the closure).
trait HasValue {
    fn value(&self) -> u32;
}
macro_rules! has_value {
    ($t:ty) => {
        impl HasValue for $t {
            fn value(&self) -> u32 {
                self.value
            }
        }
    };
}
has_value!(HoldEvt);
has_value!(PauseEvt);
has_value!(FrameEvt);
has_value!(ColdEvt);

fn event_value<E: HasValue>(e: &E) -> u32 {
    e.value()
}

/// P20-B5: a Fixed reader over a script with interleaved 0-substep frames
/// observes EVERY event exactly once — the WaitForFixed gate holds the swap on
/// 0-substep frames instead of dropping a generation.
#[test]
fn wait_for_fixed_hold_loses_no_events() {
    register_event::<HoldEvt>(130);
    let (mut app, sum, cnt) = gated_event_app::<HoldEvt>(|v| HoldEvt { value: v });

    const SENDS: u32 = 20;
    app.finish();
    app.world_mut().resource_mut::<SendBudget>().0 = SENDS;

    // Script: each iteration = one stepping frame followed by two 0-substep
    // frames (the hold pattern). Sends happen on EVERY frame with budget.
    for _ in 0..SENDS {
        app.update_with_delta(Duration::from_millis(20)); // ≥1 substep
        app.update_with_delta(TINY); // 0 substeps — swap held next frame
        app.update_with_delta(TINY); // 0 substeps — still held
    }
    // Flush: a few stepping frames so the tail generation swaps + is read.
    for _ in 0..4 {
        app.update_with_delta(Duration::from_millis(20));
    }

    let expected_sum: u32 = (1..=SENDS).sum();
    assert_eq!(cnt.load(Ordering::Relaxed), SENDS, "every event observed exactly once");
    assert_eq!(sum.load(Ordering::Relaxed), expected_sum, "payload sum exact (no dup/loss)");
}

/// ★M1: a pause spanning many frames holds the swap; on unpause the backlog
/// arrives exactly once — nothing lost, nothing doubled.
#[test]
fn pause_spanning_hold_delivers_backlog_once() {
    register_event::<PauseEvt>(131);
    let (mut app, sum, cnt) = gated_event_app::<PauseEvt>(|v| PauseEvt { value: v });

    app.finish();
    app.world_mut().resource_mut::<SendBudget>().0 = 6;

    // 3 normal frames (3 sends, flowing).
    for _ in 0..3 {
        app.update_with_delta(Duration::from_millis(20));
    }
    // Pause: 5 frames — Main still runs (sends 3 more while budget lasts),
    // 0 substeps each, swap held.
    app.world_mut().resource_mut::<Time>().pause();
    for _ in 0..5 {
        app.update_with_delta(Duration::from_millis(20));
    }
    app.world_mut().resource_mut::<Time>().unpause();
    // Unpause + flush.
    for _ in 0..4 {
        app.update_with_delta(Duration::from_millis(20));
    }

    let expected_sum: u32 = (1..=6).sum();
    assert_eq!(cnt.load(Ordering::Relaxed), 6, "backlog delivered once after unpause");
    assert_eq!(sum.load(Ordering::Relaxed), expected_sum, "exact payloads, no dup/loss");
}

/// The EveryFrame policy (auto-default when no Fixed schedule exists; here set
/// explicitly WITH one) swaps every frame: a Main reader sees each event on
/// the very next frame and never double-reads.
#[test]
fn every_frame_policy_swaps_each_frame() {
    register_event::<FrameEvt>(132);
    let recv = counter();
    let r = Arc::clone(&recv);

    let mut app = App::with_pool(serial_pool());
    // Lane count derived from the pool by the App wiring (see gated_event_app).
    app.world_mut()
        .preregister_event_default::<FrameEvt>()
        .expect("preregister");
    app.world_mut().insert_resource(SendBudget(0));
    app.set_event_update_policy(EventUpdatePolicy::EveryFrame);

    app.add_systems(move |mut budget: ResMut<SendBudget>, mut w: EventWriter<FrameEvt>| {
        if budget.0 > 0 {
            budget.0 -= 1;
            w.send(FrameEvt { value: 1 }).expect("send");
        }
    });
    // Reader in MAIN this time; a dormant Fixed schedule exists to prove the
    // policy override (without it WaitForFixed would hold on 0-substep frames).
    app.add_systems(move |mut rd: EventReader<FrameEvt>| {
        r.fetch_add(rd.read().count() as u32, Ordering::Relaxed);
    });
    app.add_systems_in(CoreSchedule::Fixed, || {});

    app.finish();
    app.world_mut().resource_mut::<SendBudget>().0 = 5;
    // ALL frames are 0-substep — EveryFrame must still flow events through.
    for _ in 0..8 {
        app.update_with_delta(TINY);
    }
    assert_eq!(recv.load(Ordering::Relaxed), 5, "EveryFrame flows on 0-substep frames");
}

/// ★m4 cold start: events sent on frame 1 (which has 0 substeps under a tiny
/// first delta) reach a Fixed reader within the documented bounded delay once
/// stepping frames arrive — nothing is lost at startup.
#[test]
fn cold_start_events_bounded_delay_no_loss() {
    register_event::<ColdEvt>(133);
    let (mut app, sum, cnt) = gated_event_app::<ColdEvt>(|v| ColdEvt { value: v });

    app.finish();
    app.world_mut().resource_mut::<SendBudget>().0 = 1;

    app.update_with_delta(TINY); // frame 1: send happens; 0 substeps; swap held
    assert_eq!(cnt.load(Ordering::Relaxed), 0, "nothing visible yet (held)");
    app.update_with_delta(Duration::from_millis(20)); // frame 2: substep fires
    app.update_with_delta(Duration::from_millis(20)); // frame 3: swap + read
    assert_eq!(cnt.load(Ordering::Relaxed), 1, "frame-1 event visible by frame 3");
    assert_eq!(sum.load(Ordering::Relaxed), 1, "payload intact");
}

// =============================================================================
// D7 — states in a two-schedule world
// =============================================================================

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
enum Mode {
    #[default]
    Alpha,
    Beta,
}
impl States for Mode {}

/// State on FIXED: transitions apply per substep; `on_enter` fires once; a
/// transition queued while paused (0 substeps) applies on the resume substep
/// (the demo's documented queued-while-paused contract).
#[test]
fn state_on_fixed_per_substep_transitions() {
    let in_alpha = counter();
    let ia = Arc::clone(&in_alpha);
    let entered_beta = counter();
    let eb = Arc::clone(&entered_beta);
    let requested = counter();
    let rq = Arc::clone(&requested);

    let mut app = App::with_pool(serial_pool());
    app.insert_state_in(CoreSchedule::Fixed, Mode::Alpha);
    app.add_systems_cfg_in(CoreSchedule::Fixed, |b| {
        b.add_system(move || {
            ia.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(in_state(Mode::Alpha));
        let eb2 = Arc::clone(&eb);
        b.add_system(move || {
            eb2.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(on_enter(Mode::Beta));
        let rq2 = Arc::clone(&rq);
        b.add_system(move |mut next: ResMut<NextState<Mode>>| {
            // Request the switch exactly once, on the first substep.
            if rq2.fetch_add(1, Ordering::Relaxed) == 0 {
                next.set(Mode::Beta);
            }
        });
    });

    // Frame 1, substep 1: Alpha runs; Beta requested.
    app.update_with_delta(STEP);
    assert_eq!(in_alpha.load(Ordering::Relaxed), 1, "substep 1 still Alpha");
    assert_eq!(entered_beta.load(Ordering::Relaxed), 0, "transition not applied yet");

    // Pause-shaped gap: a 0-substep frame leaves the queued transition pending.
    app.update_with_delta(TINY);
    assert_eq!(entered_beta.load(Ordering::Relaxed), 0, "0-substep frame applies nothing");

    // Frame 3, substep 1: the pass applies Alpha→Beta; on_enter fires once;
    // the in_state(Alpha) system is skipped.
    app.update_with_delta(STEP);
    assert_eq!(entered_beta.load(Ordering::Relaxed), 1, "on_enter(Beta) exactly once");
    assert_eq!(in_alpha.load(Ordering::Relaxed), 1, "Alpha system skipped after the switch");

    // Further substeps: no re-fire.
    app.update_with_delta(STEP * 2);
    assert_eq!(entered_beta.load(Ordering::Relaxed), 1, "no on_enter re-fire");
}

/// State on MAIN gating a FIXED system via `in_state` (the value-read form —
/// valid cross-schedule at frame granularity per the D7 contract).
#[test]
fn state_on_main_gates_fixed_system_frame_granular() {
    let fixed_runs = counter();
    let fr = Arc::clone(&fixed_runs);
    let frame_no = Arc::new(AtomicU32::new(0));
    let fno = Arc::clone(&frame_no);

    let mut app = App::with_pool(serial_pool());
    app.insert_state(Mode::Alpha); // Main-registered state
    app.add_systems(move |mut next: ResMut<NextState<Mode>>| {
        // Switch to Beta during frame 2's Main run.
        if fno.fetch_add(1, Ordering::Relaxed) == 1 {
            next.set(Mode::Beta);
        }
    });
    app.add_systems_cfg_in(CoreSchedule::Fixed, |b| {
        b.add_system(move || {
            fr.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(in_state(Mode::Beta));
    });

    // Frame 1 (1 substep): state Alpha ⇒ fixed system skipped.
    app.update_with_delta(STEP);
    assert_eq!(fixed_runs.load(Ordering::Relaxed), 0, "Alpha: gated fixed system skipped");
    // Frame 2 (1 substep): Main's system REQUESTS Beta this frame; the Phase-17
    // transition pass runs at the START of a schedule run, so the request is
    // applied at the start of MAIN run 3 (next frame).
    app.update_with_delta(STEP);
    assert_eq!(fixed_runs.load(Ordering::Relaxed), 0, "request queued, not yet applied");
    // Frame 3 (1 substep): Fixed runs BEFORE Main (D1 order) — the substep
    // still reads Alpha; Main run 3 then applies Alpha→Beta at its pass.
    app.update_with_delta(STEP);
    assert_eq!(fixed_runs.load(Ordering::Relaxed), 0, "applied at Main run 3, after substeps");
    // Frame 4: the substep now reads Beta ⇒ runs (frame-granular visibility).
    app.update_with_delta(STEP);
    assert_eq!(fixed_runs.load(Ordering::Relaxed), 1, "Beta visible to Fixed one frame later");
}
