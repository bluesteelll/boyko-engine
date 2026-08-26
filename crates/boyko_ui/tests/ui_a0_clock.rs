//! UI-ADVANCED rung **A0** — the UI clock (`docs/UI-PLAN-ANIMATION.md` A0, AD1,
//! AD9, AM6, AM7).
//!
//! Six legs. **Each states what it does NOT prove**, because on this branch two
//! legs were written believing a third covered them, and the rung's own gate
//! passed green over a permanently-zero `dt_virtual` — the field every unflagged
//! consumer reads.
//!
//! | Leg | Test | Owns red mutation |
//! |---|---|---|
//! | 1 | [`clock_tick_ran`] | 7 — drop `ui_clock_tick` from `UiAnimationPlugin::build` |
//! | 2 | [`clock_paused_advances_real_not_virtual`] | 1 — `dt_real` sourced from `delta_secs`; 2 — `dt_virtual` sourced from `real_delta` |
//! | 3 | [`clock_clamps_a_hitch`] | 3 / 4 — delete either clamp |
//! | 4 | [`clock_virtual_is_positive_clamped_and_scaled`] | 2 — the unscaled source |
//! | 5 | [`plugin_adds_no_shared_schedule_surface`] + [`the_probes_are_not_vacuous`] | 5 — register on `CoreSchedule::Fixed` |
//! | 6 | [`flipbook_reads_the_virtual_delta`] (+ the SHIPPED `g5_2_…` in `boyko_render`) | 6 — the flipbook reads `dt_real` |
//! | 7 | [`a_consumer_after_the_set_observes_a_written_clock`] + [`the_ordering_probe_is_not_vacuous`] | 8 — delete `.in_set(UiAnimationSet)` from `UiAnimationPlugin::build` |
//! | 8 | [`a_host_configured_clock_survives_the_plugin`] | 9 — replace the insert-if-absent guard with an unconditional `insert_resource` |
//!
//! # Legs 7 and 8 exist because both were MEASURED ungated at the A0 landing
//!
//! Deleting `.in_set(UiAnimationSet)` left this file at 7/7 and `boyko-ui --lib`
//! at 20/20; replacing the insert-if-absent guard with an unconditional
//! `insert_resource` left this file at 7/7. Both are the campaign's signature
//! class — a **doc-comment contract no test can see** — and both are load-bearing
//! promises rather than decoration: the set is what a downstream host's
//! `.after_set(UiAnimationSet)` resolves against (a set with no members expands
//! to zero edges, so every downstream ordering edge silently becomes a no-op),
//! and the guard is the escape hatch §7 Q1 leans on ("`set_max_delta` exists per
//! host") — a host that configures its clamp BEFORE `add_plugin` loses it.
//!
//! # Leg 1 cannot tell the two sources apart, and that is MEASURED
//!
//! Under leg 1's own precondition (unpaused, `relative_speed == 1.0`, raw below
//! both clamps) `Time::advance_with` takes the integer-nanosecond branch and
//! assigns `delta = clamped = raw`, which the kernel pins itself
//! (`advance_with_default_path_is_integer_exact`). So `delta_secs()` and
//! `real_delta().as_secs_f32()` are the SAME `f32` there. Legs 2 and 4 are what
//! separate the two sources; leg 1's one unique claim is *the system ran at all*.
//!
//! # Leg 5 ACTS; it does not read
//!
//! `App` exposes no getter for its resolved `EventUpdatePolicy` (private field,
//! setter only) and no enumeration of registered schedule labels
//! (`fixed_builder` private), and `CoreSchedule` is a closed two-variant enum. So
//! "an identical registered schedule-label set" is not a statement anything in
//! this tree can make. The two behavioural probes below are borrowed verbatim
//! from `boyko_render/tests/particle_containment.rs` — **together with its
//! non-vacuity control**, which is the exact import that file's own header warns
//! against omitting.

#![cfg(not(miri))]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;
use boyko_ecs::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use boyko_ecs::prelude::*;
use boyko_macros::Resource;

use boyko_ui::animation::{ui_clock_tick, UiAnimationPlugin, UiAnimationSet, UiClock};
use boyko_ui::components::{SpriteAnimMode, UiSpriteAnim, UiSpriteSheet};
use boyko_ui::sprite::{ui_sprite_flipbook, UI_FALLBACK_MAX_DELTA};

/// One ordinary frame: below BOTH clamps, so nothing here is truncated.
const FRAME: Duration = Duration::from_millis(16);
/// The alt-tab stall AM6 is about — eight times `Time`'s own clamp and twenty
/// times the UI's.
const HITCH: Duration = Duration::from_secs(2);

/// A finished app carrying only [`UiAnimationPlugin`].
fn clock_app() -> App {
    let mut app = App::with_pool(ThreadPoolBuilder::new().num_threads(1).build());
    app.add_plugin(UiAnimationPlugin);
    app.finish();
    app
}

/// The clock after `app`'s last frame. `UiClock` is `Copy`, so the borrow ends
/// here and the assertions read a value rather than hold the world.
fn clock_of(app: &App) -> UiClock {
    *app.world().resource::<UiClock>()
}

// ───────────────────────── leg 1 ───────────────────────────────────────────

/// **A0 leg 1 — the system RAN.** Named for what it actually proves.
///
/// `dt_real` equals `Time::real_delta()` in seconds after one 16 ms frame, and
/// the pre-frame value was zero, so the equality cannot be satisfied by a clock
/// nobody wrote.
///
/// **Does NOT prove `dt_real` came from the real side** — see the module doc:
/// below both clamps at speed 1.0 the two `f32` are bit-identical and no
/// assertion here can separate them. Legs 2 and 4 do that.
///
/// Red mutation 7 (drop `ui_clock_tick` from the plugin) reds exactly this.
#[test]
fn clock_tick_ran() {
    let mut app = clock_app();
    assert_eq!(
        clock_of(&app).dt_real(),
        0.0,
        "the pre-frame clock is zero, so the post-frame equality below is a WRITE, \
         not a coincidence"
    );

    app.update_with_delta(FRAME);

    let raw = app.world().resource::<Time>().real_delta().as_secs_f32();
    let clock = clock_of(&app);
    assert!(raw > 0.0, "the frame driver advanced Time");
    assert_eq!(
        clock.dt_real(),
        raw,
        "after one frame the UI clock holds Time's real delta — the plugin registered \
         the tick and the tick ran"
    );
    assert_eq!(
        clock.max_delta(),
        UI_FALLBACK_MAX_DELTA,
        "the clamp is the sprite const itself (AD9 (3)), never a second 0.1"
    );
}

// ───────────────────────── leg 2 ───────────────────────────────────────────

/// **A0 leg 2 — PAUSED: the virtual delta is zero AND the real one is not.**
/// D15's whole reason, as a test, and the leg that catches BOTH cross-wirings:
/// `dt_real` sourced from `delta_secs` (red mutation 1) and `dt_virtual` sourced
/// from `real_delta` (red mutation 2) each red here.
///
/// **Does NOT prove `dt_virtual` is ever non-zero** — an implementation that
/// hardwires it to `0.0` passes this leg. Leg 4 exists because of that.
#[test]
fn clock_paused_advances_real_not_virtual() {
    let mut app = clock_app();
    app.world_mut().run_system(|mut time: ResMut<Time>| time.pause());
    app.update_with_delta(FRAME);

    let clock = clock_of(&app);
    assert_eq!(
        clock.dt_virtual(),
        0.0,
        "a paused Time yields a ZERO virtual delta, and the UI clock carries it through — \
         sourcing dt_virtual from real_delta breaks exactly here"
    );
    assert!(
        clock.dt_real() > 0.0,
        "…while the REAL delta keeps advancing: real time does not pause, and sourcing \
         dt_real from delta_secs breaks exactly here"
    );
    assert_eq!(
        clock.dt_real(),
        FRAME.as_secs_f32(),
        "and it is the frame's own raw delta, unscaled and pause-blind"
    );
}

// ───────────────────────── leg 3 ───────────────────────────────────────────

/// **A0 leg 3 — a HITCH is clamped, on BOTH deltas (AM6).**
///
/// A 2 000 ms raw delta yields (a) `dt_real == max_delta`, not 2.0, and
/// (b) `dt_virtual == max_delta`, not 0.25. Leg (b) is live rather than
/// decorative because `Time`'s own 250 ms clamp lands first, so an unclamped
/// `dt_virtual` reads **0.25** — a value that is neither the input nor the
/// answer, and would otherwise look plausible.
///
/// Both comparisons are against `clock.max_delta()` itself, never a `0.1`
/// literal: the `min` is taken against that very value, so the equality is exact
/// by construction. The two negative assertions name the numbers each deleted
/// clamp would produce, read from the engine rather than typed in.
///
/// **Does NOT prove either delta's SOURCE** — at the clamp both fields are the
/// same number by definition. Legs 2 and 4 own the source.
#[test]
fn clock_clamps_a_hitch() {
    let mut app = clock_app();
    app.update_with_delta(HITCH);

    let clock = clock_of(&app);
    let engine_clamp = app.world().resource::<Time>().max_delta().as_secs_f32();

    // (a) the real delta: unclamped by `Time` itself (AM6), so this `min` is the
    //     only thing between an alt-tab stall and a two-second UI step.
    assert_eq!(
        clock.dt_real(),
        clock.max_delta(),
        "a 2 s stall is truncated to the UI clamp on the REAL delta — Time::real_delta is \
         documented unclamped and assigns BEFORE Time's own min()"
    );
    assert_ne!(
        clock.dt_real(),
        HITCH.as_secs_f32(),
        "…and specifically is NOT the raw 2.0 s, which is what deleting that min() yields"
    );

    // (b) the virtual delta: `Time` clamped it to 250 ms, which is four times the
    //     UI's clamp — plausible-looking, and wrong.
    assert_eq!(
        clock.dt_virtual(),
        clock.max_delta(),
        "AD1's clamp applies to BOTH deltas, and this is the only leg that says so"
    );
    assert_ne!(
        clock.dt_virtual(),
        engine_clamp,
        "…and specifically is NOT Time's own 250 ms showing through, which is what \
         deleting that min() yields"
    );
    assert!(
        clock.max_delta() < engine_clamp,
        "the UI clamp must be TIGHTER than the engine's, or leg (b) has nothing to catch"
    );
}

// ───────────────────────── leg 4 ───────────────────────────────────────────

/// **A0 leg 4 — `dt_virtual` is POSITIVE, SCALED, and is NOT `dt_real`.**
///
/// Half speed and an **80 ms** raw delta — deliberately below the 100 ms clamp,
/// so this leg tests the SOURCE and the SCALING with the clamp out of the
/// picture. It is also the only leg that separates the two fields on an
/// **unpaused** frame, which leg 1 provably cannot.
///
/// Both expectations are computed `Duration`s, never `0.04` / `0.08` decimal
/// literals — this project does not gamble a gate on a literal's ULP.
///
/// **Why this leg exists:** without it an implementation that hardwires
/// `dt_virtual = 0.0` unconditionally passes legs 1, 2, 3 and 5 green, and so
/// does one that drops `relative_speed`. `dt_virtual` is the field AD9 makes
/// every unflagged consumer read.
#[test]
fn clock_virtual_is_positive_clamped_and_scaled() {
    const RAW: Duration = Duration::from_millis(80);
    const SCALED: Duration = Duration::from_millis(40);

    let mut app = clock_app();
    app.world_mut()
        .run_system(|mut time: ResMut<Time>| time.set_relative_speed(0.5));
    app.update_with_delta(RAW);

    let clock = clock_of(&app);
    assert!(
        RAW.as_secs_f32() < clock.max_delta(),
        "the raw delta must sit BELOW the clamp, or this leg silently becomes leg 3"
    );
    assert!(
        clock.dt_virtual() > 0.0,
        "dt_virtual is POSITIVE on an unpaused frame — a field hardwired to zero passes \
         every other leg in this file"
    );
    assert_eq!(
        clock.dt_virtual(),
        SCALED.as_secs_f32(),
        "…and it is SCALED by Time::relative_speed — half speed halves it"
    );
    assert_eq!(
        clock.dt_real(),
        RAW.as_secs_f32(),
        "…while dt_real ignores the scale entirely: that is what makes it the REAL delta"
    );
    assert_ne!(
        clock.dt_virtual(),
        clock.dt_real(),
        "the two fields are distinguishable on an UNPAUSED frame — the statement leg 1 \
         cannot make and leg 2 makes only under pause"
    );
}

// ───────────────────────── leg 5 ───────────────────────────────────────────
//
// Borrowed verbatim from `boyko_render/tests/particle_containment.rs`, WITH its
// non-vacuity control. See the module doc for why this is an ACTING probe.

/// One 64 Hz fixed step, plus slack — a frame delta guaranteed to expend at
/// least one substep IF a Fixed schedule exists.
const STEP: Duration = Duration::from_millis(20);
/// A delta far too small to complete a 64 Hz substep across any script here.
const TINY: Duration = Duration::from_micros(1);
/// Events the probe script sends.
const SENDS: u32 = 5;
/// 0-substep frames the event probe runs. More than `SENDS`, so a delivered
/// event has several frames in which to arrive and "0 observed" cannot be a
/// timing artifact.
const PROBE_FRAMES: u32 = 8;

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

/// The probe event. A fresh id is safe without a reserved range: each
/// integration test file is its own process, so this binary's event registry is
/// not shared with any other suite.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ContainmentEvt {
    value: u32,
}

impl Event for ContainmentEvt {
    type Participants = NoParticipants;
    type Parameters = NoParameters;
    fn event_id() -> u64 {
        1
    }
    fn event_name() -> &'static str {
        "ContainmentEvt"
    }
    fn new(_: NoParticipants, _: NoParameters) -> Self {
        ContainmentEvt { value: 0 }
    }
    fn participants(&self) -> &NoParticipants {
        unimplemented!("the probe event carries no participants")
    }
    fn participants_mut(&mut self) -> &mut NoParticipants {
        unimplemented!("the probe event carries no participants")
    }
    fn parameters(&self) -> &NoParameters {
        unimplemented!("the probe event carries no parameters")
    }
    fn parameters_mut(&mut self) -> &mut NoParameters {
        unimplemented!("the probe event carries no parameters")
    }
}

/// Remaining sends, so the writer stops on its own and the counts are exact.
#[derive(Resource, Default)]
struct SendBudget(u32);

/// Everything leg 5 compares between two apps. Every field is the result of an
/// ACTION — none of them is a field read, because none of them CAN be.
#[derive(Debug, PartialEq, Eq)]
struct AppSignature {
    /// `true` iff a `CoreSchedule::Fixed` schedule exists (the driver advanced
    /// the fixed clock).
    has_fixed_schedule: bool,
    /// Events a Main reader observed over `PROBE_FRAMES` 0-substep frames.
    events_delivered: u32,
    /// The live schedule labels, as their stable spellings.
    labels: Vec<&'static str>,
}

/// Builds a probe app, optionally with [`UiAnimationPlugin`] and optionally with
/// a Fixed schedule.
///
/// `with_fixed` exists ONLY for the non-vacuity canary; both gate apps pass
/// `false`.
fn probe_app(with_plugin: bool, with_fixed: bool) -> (App, Arc<AtomicU32>) {
    let received = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&received);

    // One worker + the dispatcher ⇒ two event lanes; a serial pool keeps the
    // script deterministic.
    let mut app = App::with_pool(ThreadPoolBuilder::new().num_threads(1).build());
    app.world_mut()
        .preregister_event_default::<ContainmentEvt>()
        .expect("invariant: the probe event preregisters on a fresh world");
    app.world_mut().insert_resource(SendBudget(0));

    app.add_systems(
        |mut budget: ResMut<SendBudget>, mut w: EventWriter<ContainmentEvt>| {
            if budget.0 > 0 {
                budget.0 -= 1;
                w.send(ContainmentEvt { value: 1 })
                    .expect("invariant: send within lane capacity");
            }
        },
    );
    app.add_systems(move |mut r: EventReader<ContainmentEvt>| {
        counter.fetch_add(r.read().count() as u32, Ordering::Relaxed);
    });

    if with_plugin {
        app.add_plugin(UiAnimationPlugin);
    }
    if with_fixed {
        // The canary's only difference: a Fixed schedule EXISTS. This is
        // precisely what `UiAnimationPlugin` must never do.
        app.add_systems_in(CoreSchedule::Fixed, || {});
    }

    (app, received)
}

/// Runs both probes against `app` and returns its observable signature.
fn signature_of(app: &mut App, received: &AtomicU32) -> AppSignature {
    app.finish();
    app.world_mut().resource_mut::<SendBudget>().0 = SENDS;

    // Probe 1 — the event swap, over 0-substep frames ONLY. Run first: a
    // stepping frame would release a held backlog and erase the very difference
    // this probe exists to see.
    for _ in 0..PROBE_FRAMES {
        app.update_with_delta(TINY);
    }
    let events_delivered = received.load(Ordering::Relaxed);

    // Probe 2 — does a Fixed schedule exist? `fixed_advance` is the sole writer
    // of `FixedTime`'s clock and runs only when one does.
    for _ in 0..4 {
        app.update_with_delta(STEP);
    }
    let fixed = app.world().resource::<FixedTime>();
    let has_fixed_schedule = fixed.elapsed() > Duration::ZERO || fixed.overstep() > Duration::ZERO;

    let mut labels = vec!["Main"];
    if has_fixed_schedule {
        labels.push("Fixed");
    }

    AppSignature {
        has_fixed_schedule,
        events_delivered,
        labels,
    }
}

/// **A0 leg 5 — plugin containment, in the ACTING form.**
///
/// Two apps differing ONLY by `add_plugin(UiAnimationPlugin)` have identical
/// observable schedule/event configuration: the same resolved event-swap
/// behaviour, no Fixed schedule in either, and the same live schedule-label set.
///
/// The failure this forbids is not hypothetical. A UI clock registered on
/// `CoreSchedule::Fixed` — a plausible reading of "a clock ticks on a timestep" —
/// creates the lazy `fixed_builder`, which makes `App::finish` resolve the
/// process-wide event policy to `WaitForFixed`, which holds the event swap on
/// every 0-substep frame. At 200 fps against a 64 Hz step that is two frames in
/// three, for INPUT, UI and COLLISION events, in an app whose only change was
/// installing the UI clock. Red mutation 5 flips BOTH probes.
///
/// **Does NOT prove the tick runs, or writes anything** — a plugin that
/// registered nothing at all passes this leg. Legs 1–4 own that.
#[test]
fn plugin_adds_no_shared_schedule_surface() {
    let (mut without, without_rx) = probe_app(false, false);
    let (mut with, with_rx) = probe_app(true, false);

    let baseline = signature_of(&mut without, &without_rx);
    let armed = signature_of(&mut with, &with_rx);

    assert_eq!(
        armed, baseline,
        "installing UiAnimationPlugin must leave the app's observable schedule/event \
         configuration identical"
    );
    assert!(
        !baseline.has_fixed_schedule,
        "the baseline app has no Fixed schedule"
    );
    assert!(
        !armed.has_fixed_schedule,
        "UiAnimationPlugin must NOT create a CoreSchedule::Fixed schedule — doing so flips \
         the process-wide EventUpdatePolicy to WaitForFixed"
    );
    assert_eq!(
        baseline.labels,
        vec!["Main"],
        "only the Main schedule is live without the plugin"
    );
    assert_eq!(
        armed.labels,
        vec!["Main"],
        "and only the Main schedule is live with it"
    );
    assert_eq!(
        armed.events_delivered, SENDS,
        "every sent event reaches its Main reader on a 0-substep frame — the every-frame swap"
    );
}

/// **Leg 5's non-vacuity control.** A gate that cannot fail is not a gate.
///
/// This builds the app [`UiAnimationPlugin`] must never produce — one with a
/// Fixed schedule — and shows BOTH probes flip, so a future regression really
/// would be caught above. Copied WITH the probes deliberately: borrowing them
/// without their control is the exact import
/// `particle_containment.rs`'s own header warns against.
#[test]
fn the_probes_are_not_vacuous() {
    let (mut clean, clean_rx) = probe_app(false, false);
    let (mut with_fixed, fixed_rx) = probe_app(false, true);

    let clean = signature_of(&mut clean, &clean_rx);
    let dirty = signature_of(&mut with_fixed, &fixed_rx);

    assert_ne!(
        clean, dirty,
        "the probes must distinguish a Fixed-schedule app from a clean one"
    );
    assert!(
        dirty.has_fixed_schedule,
        "the fixed probe must SEE a Fixed schedule when one exists"
    );
    assert_eq!(dirty.labels, vec!["Main", "Fixed"]);
    assert_eq!(
        dirty.events_delivered, 0,
        "WaitForFixed holds the swap across 0-substep frames"
    );
    assert_eq!(
        clean.events_delivered, SENDS,
        "and the clean app delivers all of them"
    );
}

// ───────────────────────── leg 6 (A0b) ─────────────────────────────────────

/// One 10 fps `Forward` flipbook node over frames `0..=3`, in a world holding
/// [`Time`] and [`UiClock`].
///
/// No cursor is spelled: `UiSpriteAnim`'s `on_add` hook materializes it (S6),
/// which `g6_4` pins.
fn flipbook_world(fps: f32) -> (EcsMaster, Entity) {
    let mut world = EcsMaster::new();
    world.insert_resource(Time::default());
    // `run_system` returns the closure's own value, so the spawned id comes back
    // directly — no `Arc<Mutex<Option<Entity>>>` probe, and therefore no
    // `#[allow(clippy::disallowed_types)]` exception to justify. Same shape as
    // `particle_containment.rs`'s hook test.
    let node = world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(UiSpriteSheet { sheet: 0, index: 0 });
        e.insert(UiSpriteAnim {
            first: 0,
            last: 3,
            fps,
            mode: SpriteAnimMode::Forward,
            repeats: 0,
            _pad: [0; 2],
        });
        e.id()
    });
    (world, node)
}

/// The clock tick ordered AHEAD of the flipbook — A0b's whole wiring.
fn clock_then_flipbook(world: &mut EcsMaster) -> Schedule {
    world.insert_resource(UiClock::default());
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let tick = b.add_system(ui_clock_tick).key();
    b.add_system(ui_sprite_flipbook).after(tick);
    b.build(world)
}

fn tick(world: &mut EcsMaster, schedule: &mut Schedule, dt: Duration) {
    world.resource_mut::<Time>().advance_with(dt);
    schedule.run(world);
}

fn index_of(world: &EcsMaster, e: Entity) -> u16 {
    world
        .get_component::<UiSpriteSheet>(e)
        .map(|s| s.index)
        .expect("the node carries UiSpriteSheet")
}

/// **A0b / AD9 (1) — the flipbook reads `dt_virtual`, not `dt_real`.**
///
/// Two legs, and each one asserts the field it is NOT reading is simultaneously
/// non-zero and different — which is what makes this a FIELD-CHOICE test rather
/// than a second copy of `g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware`
/// (which re-runs UNEDITED in `boyko_render`, and is leg 6's other half).
///
/// 1. **PAUSED.** `dt_real` is a live 100 ms while `dt_virtual` is zero, and the
///    animation is frozen. A flipbook on `dt_real` animates a paused game.
/// 2. **HALF SPEED, below the clamp.** Three 80 ms frames at 10 fps advance the
///    virtual accumulator to 0.12 s ⇒ **one** frame. On `dt_real` the same script
///    accumulates 0.24 s ⇒ **two**. The clamp is out of the picture (80 ms and
///    40 ms both sit below 100 ms), so the only thing separating 1 from 2 is
///    which field was read.
///
/// **Does NOT prove the clamp reaches the flipbook** — G5-2 (a) does, and it is
/// not edited by this rung.
#[test]
fn flipbook_reads_the_virtual_delta() {
    const FPS: f32 = 10.0;

    // (1) PAUSED — the animation is frozen while the real delta is live.
    let (mut world, node) = flipbook_world(FPS);
    let mut schedule = clock_then_flipbook(&mut world);
    world.run_system(|mut time: ResMut<Time>| time.pause());
    for _ in 0..5 {
        tick(&mut world, &mut schedule, Duration::from_millis(100));
    }
    let clock = *world.resource::<UiClock>();
    assert_eq!(
        clock.dt_virtual(),
        0.0,
        "precondition: the virtual delta is zero under pause"
    );
    assert!(
        clock.dt_real() > 0.0,
        "precondition: the field the flipbook must NOT read is LIVE — without this the leg \
         proves nothing about which field was chosen"
    );
    assert_eq!(
        index_of(&world, node),
        0,
        "a PAUSED game animates nothing: the flipbook reads dt_virtual"
    );

    // (2) HALF SPEED, both deltas below the clamp.
    let (mut world, node) = flipbook_world(FPS);
    let mut schedule = clock_then_flipbook(&mut world);
    world.run_system(|mut time: ResMut<Time>| time.set_relative_speed(0.5));
    for _ in 0..3 {
        tick(&mut world, &mut schedule, Duration::from_millis(80));
    }
    let clock = *world.resource::<UiClock>();
    assert_eq!(
        clock.dt_virtual(),
        Duration::from_millis(40).as_secs_f32(),
        "precondition: half speed halved the virtual delta"
    );
    assert_eq!(
        clock.dt_real(),
        Duration::from_millis(80).as_secs_f32(),
        "precondition: and left the real one alone — the two fields differ THIS frame"
    );
    assert!(
        clock.dt_real() < clock.max_delta(),
        "precondition: the clamp is out of the picture, so only the SOURCE separates the arms"
    );
    assert_eq!(
        index_of(&world, node),
        1,
        "three 80 ms frames at half speed advance ONE 10 fps frame — on dt_real the same \
         script advances TWO"
    );
}

// ───────────────────────── leg 7 (set membership) ──────────────────────────

/// What the downstream probe saw on its FIRST run — the only run that can tell
/// the ordering apart, since from frame 2 onward the clock carries the PREVIOUS
/// frame's non-zero deltas whatever the order was.
#[derive(Resource, Default)]
struct FirstObservation {
    /// Frames the probe ran. Asserted `== 1`, so the fields below are frame 1's.
    runs: u32,
    /// `UiClock::dt_real` as the probe read it on frame 1.
    dt_real: f32,
    /// `UiClock::dt_virtual` as the probe read it on frame 1.
    dt_virtual: f32,
}

/// The downstream consumer leg 7 is about.
///
/// `Res<UiClock>` against `ui_clock_tick`'s `ResMut<UiClock>` is a read/write
/// conflict, so the scheduler can never overlap the two and MUST pick an order.
/// Which order it picks is the entire question.
fn downstream_probe(clock: Res<UiClock>, mut obs: ResMut<FirstObservation>) {
    obs.runs += 1;
    if obs.runs == 1 {
        obs.dt_real = clock.dt_real();
        obs.dt_virtual = clock.dt_virtual();
    }
}

/// A finished app whose probe is registered **before** the plugin. `ordered`
/// decides whether the probe carries `.after_set(UiAnimationSet)`.
///
/// The probe-first registration order is the load-bearing detail: with the set
/// empty, `.after_set` expands to zero edges and the probe stays where it was
/// registered — ahead of the tick, reading a clock nobody has written. So the
/// edge is the ONLY thing that can move it, and
/// [`the_ordering_probe_is_not_vacuous`] shows the un-edged app really does
/// observe the zero.
fn ordered_probe_app(ordered: bool) -> App {
    let mut app = App::with_pool(ThreadPoolBuilder::new().num_threads(1).build());
    app.world_mut().insert_resource(FirstObservation::default());
    app.add_systems_cfg_in(CoreSchedule::Main, |b| {
        if ordered {
            b.add_system(downstream_probe).after_set(UiAnimationSet);
        } else {
            b.add_system(downstream_probe);
        }
    });
    app.add_plugin(UiAnimationPlugin);
    app.finish();
    app
}

/// **A0 leg 7 — `ui_clock_tick` really is IN [`UiAnimationSet`], proven by a
/// downstream consumer that observes a WRITTEN clock.**
///
/// `UiAnimationSet` is documented as the seam a host orders against
/// (`.after_set(UiAnimationSet)`). That promise is worth exactly the set's
/// membership: `SystemAfterSet` expands into `member → X` edges over the
/// transitive membership at `build`, so a set with **no members** expands into
/// **no edges** and every downstream ordering edge in the tree silently becomes
/// a no-op — with nothing red anywhere. Measured at the A0 landing: deleting
/// `.in_set(UiAnimationSet)` left this file at 7/7 and `boyko-ui --lib` at
/// 20/20.
///
/// The assertion is an OBSERVATION, not an inspection of schedule metadata:
/// `ScheduleBuilder` exposes no accessor for a set's resolved membership, and
/// even if it did, "the descriptor lists the set" is not "the edge fired". The
/// probe reads the clock and reports a non-zero delta that only the tick can
/// have put there, on the ONE frame where a zero and a written value differ.
///
/// **Does NOT prove any particular delta is correct** — legs 1–4 own that. This
/// leg's single claim is *the ordering edge resolved to something*.
#[test]
fn a_consumer_after_the_set_observes_a_written_clock() {
    let mut app = ordered_probe_app(true);
    app.update_with_delta(FRAME);

    let obs = app.world().resource::<FirstObservation>();
    assert_eq!(
        obs.runs, 1,
        "exactly one frame ran, so the readings below are frame 1's — the only frame on which \
         an unwritten clock is still distinguishable from a written one"
    );
    assert_eq!(
        obs.dt_real,
        FRAME.as_secs_f32(),
        "a consumer ordered .after_set(UiAnimationSet) reads THIS frame's delta: the tick is a \
         member of that set and the edge put it first. Deleting .in_set(UiAnimationSet) leaves \
         the set empty, the edge expands to nothing, and this reads 0.0"
    );
    assert!(
        obs.dt_virtual > 0.0,
        "…and the field every unflagged consumer actually reads is written too, not merely the \
         one the equality above happens to name"
    );
}

/// **Leg 7's non-vacuity control.** A gate that cannot fail is not a gate.
///
/// The same app WITHOUT the `.after_set(UiAnimationSet)` edge: the probe keeps
/// its registration position ahead of the tick and observes the zero clock. That
/// is what makes the green above an ORDERING result rather than an accident of
/// how this scheduler happens to sequence two unordered systems.
#[test]
fn the_ordering_probe_is_not_vacuous() {
    let mut app = ordered_probe_app(false);
    app.update_with_delta(FRAME);

    let obs = app.world().resource::<FirstObservation>();
    assert_eq!(
        obs.runs, 1,
        "one frame, one probe run — the same script as the leg above"
    );
    assert_eq!(
        obs.dt_real, 0.0,
        "without the ordering edge the probe runs BEFORE the tick and sees the pre-frame zero. \
         If this ever reads 0.016 the probe is ordered after the tick for some reason OTHER than \
         the edge, and the leg above proves nothing"
    );
    assert_eq!(obs.dt_virtual, 0.0, "…on both fields");
}

// ───────────────────────── leg 8 (insert-if-absent) ────────────────────────

/// **A0 leg 8 — a host-configured [`UiClock`] SURVIVES `UiAnimationPlugin`.**
///
/// `UiAnimationPlugin::build` inserts the clock only when the world does not
/// already hold one, and its doc comment promises that a host which configured
/// its own clock keeps it — the `UiSafeArea` precedent. That promise is what §7
/// Q1 (the 100 ms VALUES call) leans on when it answers *"`set_max_delta` exists
/// per host"*: a host that sets its clamp **before** `add_plugin` is the shape
/// the escape hatch has to survive, and an unconditional `insert_resource`
/// silently restores the default instead. Measured at the A0 landing: dropping
/// the guard left this file at 7/7.
///
/// The clamp is checked twice — once statically, right after `build`, and once
/// **behaviourally**, by feeding a 2 s hitch and reading the truncation. The
/// second is what makes this leg immune to a future clock that retains the
/// host's value in a field nothing consults.
///
/// `HOST_CLAMP` is deliberately neither `UI_FALLBACK_MAX_DELTA` (0.1 — what a
/// lost host value degrades to) nor `Time`'s own 250 ms clamp, so no other
/// number in the system can impersonate it.
#[test]
fn a_host_configured_clock_survives_the_plugin() {
    const HOST_CLAMP: f32 = 0.05;

    let mut app = App::with_pool(ThreadPoolBuilder::new().num_threads(1).build());
    let mut configured = UiClock::default();
    configured.set_max_delta(HOST_CLAMP);
    assert_ne!(
        configured.max_delta(),
        UI_FALLBACK_MAX_DELTA,
        "precondition: the host's clamp DIFFERS from the plugin's default, or losing it is \
         invisible"
    );
    app.world_mut().insert_resource(configured);
    app.add_plugin(UiAnimationPlugin);
    app.finish();

    assert_eq!(
        clock_of(&app).max_delta(),
        HOST_CLAMP,
        "the plugin's insert is guarded: a UiClock already in the world is left alone. An \
         unconditional insert_resource replaces it with UiClock::default() and this reads 0.1"
    );

    app.update_with_delta(HITCH);

    let clock = clock_of(&app);
    assert_eq!(
        clock.max_delta(),
        HOST_CLAMP,
        "…and the tick does not restore the default either — the survival is durable, not a \
         one-frame artifact of build order"
    );
    assert_eq!(
        clock.dt_real(),
        HOST_CLAMP,
        "the host's clamp is the value that actually TRUNCATES the hitch: the promise is \
         behavioural, not a field that merely retains a number"
    );
    assert_ne!(
        clock.dt_real(),
        UI_FALLBACK_MAX_DELTA,
        "…and specifically NOT the 0.1 an unconditional insert would have restored"
    );
    assert_eq!(
        clock.dt_virtual(),
        HOST_CLAMP,
        "…on both deltas, which is what AD1's 'the clamp applies to BOTH' means for a host \
         value too"
    );
}
