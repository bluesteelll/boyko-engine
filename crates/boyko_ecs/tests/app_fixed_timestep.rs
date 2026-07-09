//! Phase 20 — fixed-timestep catch-up loop integration (plan P20-B3 / P20-B4).
//!
//! Drives `App::update_with_delta` / `run_n_with_delta` with SCRIPTED deltas
//! (never the self-clocked `update()` — determinism) and pins the binding
//! catch-up arithmetic from `docs/PHASE-20-PLAN.md` §Metrics:
//!
//! - P20-B3: 250 ms @ 64 Hz ⇒ exactly 16 steps; 1 s raw ⇒ clamped ⇒ 16 steps
//!   with `real_delta` carrying the unclamped 1 s; steady 60 FPS ⇒ step counts
//!   of only 1s and 2s summing to exactly 64 per simulated second; post-loop
//!   `overstep < timestep` always.
//! - P20-B4: bit-exact determinism of per-frame step counts and
//!   `FixedTime::elapsed` across two runs of the same hitchy 1000-frame script.
//! - Pause semantics (D2/D4): paused ⇒ 0 substeps while real time advances.
//! - `relative_speed` scaling (D2): speed 2.0 doubles the expended step rate.
//!
//! Harness discipline matches `app_plugin.rs`: per-test `Arc<AtomicU32>`
//! counters, single-worker pools, no shared statics.

#![cfg(not(miri))]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use boyko_ecs::prelude::*;

/// 64 Hz timestep exactly (the engine default; plan D4).
const TIMESTEP: Duration = Duration::from_nanos(15_625_000);

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// An App with one counting fixed system; returns (app, substep counter).
fn counting_app() -> (App, Arc<AtomicU32>) {
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let mut app = App::with_pool(serial_pool());
    app.add_systems_in(CoreSchedule::Fixed, move || {
        c.fetch_add(1, Ordering::Relaxed);
    });
    (app, counter)
}

/// P20-B3: a single 250 ms frame at the default 64 Hz expends EXACTLY 16 steps
/// (250 ms is an exact multiple of 15.625 ms and overstep starts at zero).
#[test]
fn p20_b3_250ms_frame_is_exactly_16_steps() {
    let (mut app, counter) = counting_app();
    app.update_with_delta(Duration::from_millis(250));
    assert_eq!(counter.load(Ordering::Relaxed), 16, "250 ms / 15.625 ms = exactly 16");
    let fixed = app.world_mut().resource::<FixedTime>();
    assert_eq!(fixed.steps_this_frame(), 16, "steps_this_frame mirrors the loop");
    assert_eq!(fixed.overstep(), Duration::ZERO, "exact multiple leaves zero overstep");
}

/// P20-B3: a 1 s raw delta is CLAMPED to max_delta (250 ms) before
/// accumulation — still 16 steps — while `real_delta` carries the unclamped
/// raw value (the D2 real-vs-virtual split).
#[test]
fn p20_b3_huge_delta_clamps_to_16_steps_real_unclamped() {
    let (mut app, counter) = counting_app();
    app.update_with_delta(Duration::from_secs(1));
    assert_eq!(counter.load(Ordering::Relaxed), 16, "inflow clamp bounds the catch-up");
    let world = app.world_mut();
    let time = world.resource::<Time>();
    assert_eq!(time.real_delta(), Duration::from_secs(1), "real_delta is unclamped");
    assert_eq!(time.delta(), Duration::from_millis(250), "virtual delta is the clamp");
}

/// P20-B3: steady 60 FPS against a 64 Hz timestep — per-frame counts are only
/// 1 or 2, and 60 frames (one simulated second) expend exactly 64 steps.
#[test]
fn p20_b3_steady_60fps_expends_64_steps_per_second() {
    let (mut app, counter) = counting_app();
    let frame = Duration::from_nanos(16_666_667); // ~1/60 s
    let mut per_frame = Vec::with_capacity(60);
    let mut prev = 0u32;
    for _ in 0..60 {
        app.update_with_delta(frame);
        let now = counter.load(Ordering::Relaxed);
        per_frame.push(now - prev);
        prev = now;
    }
    assert!(
        per_frame.iter().all(|&s| s == 1 || s == 2),
        "steady 60 FPS vs 64 Hz must alternate 1- and 2-step frames, got {per_frame:?}"
    );
    // 60 × 16,666,667 ns = 1,000,000,020 ns accumulated ⇒ exactly 64 steps
    // (floor(1,000,000,020 / 15,625,000) = 64).
    assert_eq!(prev, 64, "one simulated second at 64 Hz = 64 steps");
}

/// P20-B3: the post-loop invariant `overstep < timestep` holds after every
/// frame of an irregular script (the debug_assert inside fixed_advance is the
/// belt; this is the public witness).
#[test]
fn p20_b3_overstep_below_timestep_after_every_frame() {
    let (mut app, _counter) = counting_app();
    let script = [3_000_000u64, 16_666_667, 40_000_000, 15_624_999, 15_625_001, 250_000_000];
    for &ns in script.iter().cycle().take(60) {
        app.update_with_delta(Duration::from_nanos(ns));
        let fixed = app.world_mut().resource::<FixedTime>();
        assert!(
            fixed.overstep() < TIMESTEP,
            "post-loop overstep {:?} must stay below the timestep",
            fixed.overstep()
        );
    }
}

/// P20-B4: two runs over the SAME hitchy 1000-frame script produce identical
/// per-frame step counts and bit-equal `FixedTime::elapsed` (integer-ns
/// Duration math end to end — no float in the accumulator chain).
#[test]
fn p20_b4_determinism_over_hitchy_script() {
    // Deterministic pseudo-random dt script (LCG), hitches included.
    fn script() -> impl Iterator<Item = Duration> {
        let mut state = 0x2545F491_4F6CDD1Du64;
        (0..1000).map(move |_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            // 0..40 ms typical, every 97th frame a 300 ms hitch (clamped to 250).
            let ns = (state >> 33) % 40_000_000;
            let hitch = state.is_multiple_of(97);
            Duration::from_nanos(if hitch { 300_000_000 } else { ns })
        })
    }

    let run = || -> (Vec<u32>, Duration) {
        let (mut app, counter) = counting_app();
        let mut counts = Vec::with_capacity(1000);
        let mut prev = 0u32;
        for dt in script() {
            app.update_with_delta(dt);
            let now = counter.load(Ordering::Relaxed);
            counts.push(now - prev);
            prev = now;
        }
        let elapsed = app.world_mut().resource::<FixedTime>().elapsed();
        (counts, elapsed)
    };

    let (counts_a, elapsed_a) = run();
    let (counts_b, elapsed_b) = run();
    assert_eq!(counts_a, counts_b, "per-frame step counts must be bit-deterministic");
    assert_eq!(elapsed_a, elapsed_b, "FixedTime::elapsed must be bit-equal across runs");
}

/// D2/D4 pause: paused ⇒ virtual delta ZERO ⇒ 0 substeps and frozen virtual
/// elapsed, while the real fields keep advancing.
#[test]
fn pause_freezes_substeps_but_real_time_advances() {
    let (mut app, counter) = counting_app();
    let frame = Duration::from_millis(20);

    app.update_with_delta(frame); // 1 step (20 ms > 15.625 ms)
    let after_warm = counter.load(Ordering::Relaxed);
    assert!(after_warm >= 1, "warm frame must step at least once");

    app.world_mut().resource_mut::<Time>().pause();
    let virtual_before = app.world_mut().resource::<Time>().elapsed();
    let real_before = app.world_mut().resource::<Time>().real_elapsed();
    for _ in 0..10 {
        app.update_with_delta(frame);
    }
    assert_eq!(counter.load(Ordering::Relaxed), after_warm, "paused frames expend 0 substeps");
    let world = app.world_mut();
    let time = world.resource::<Time>();
    assert_eq!(time.elapsed(), virtual_before, "virtual elapsed frozen while paused");
    assert_eq!(
        time.real_elapsed(),
        real_before + frame * 10,
        "real elapsed advances through the pause"
    );

    // Unpause: stepping resumes.
    app.world_mut().resource_mut::<Time>().unpause();
    app.update_with_delta(Duration::from_millis(250));
    assert_eq!(
        counter.load(Ordering::Relaxed),
        after_warm + 16,
        "post-unpause frame steps normally (no backlog from the paused frames)"
    );
}

/// D2 relative_speed: speed 2.0 doubles the virtual delta, hence the step
/// count over the same wall script (exact integer expectation).
#[test]
fn relative_speed_two_doubles_step_rate() {
    let (mut app, counter) = counting_app();
    app.finish(); // Time/FixedTime are seeded by finish(); configure after it.
    app.world_mut().resource_mut::<Time>().set_relative_speed(2.0);
    // 10 frames × 31.25 ms wall × 2.0 speed = 625 ms virtual = exactly 40 steps.
    for _ in 0..10 {
        app.update_with_delta(Duration::from_nanos(31_250_000));
    }
    assert_eq!(counter.load(Ordering::Relaxed), 40, "2× speed ⇒ exactly double the steps");
}

/// finish() seeds `Time`/`FixedTime` if absent; a user-inserted `FixedTime`
/// (custom Hz) WINS over the default (D5 finish contract).
#[test]
fn user_inserted_fixed_time_wins_over_default() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let mut app = App::with_pool(serial_pool());
    // 100 Hz instead of the 64 Hz default.
    app.world_mut().insert_resource(FixedTime::from_hz(100.0));
    app.add_systems_in(CoreSchedule::Fixed, move || {
        c.fetch_add(1, Ordering::Relaxed);
    });
    app.update_with_delta(Duration::from_millis(250));
    assert_eq!(counter.load(Ordering::Relaxed), 25, "250 ms at 100 Hz = exactly 25 steps");
}

/// A no-Fixed app: `FixedTime` stays inert (steps_this_frame reads a permanent
/// 0 — the documented D5/Q5 contract) and Main runs every frame.
#[test]
fn no_fixed_schedule_keeps_fixed_time_inert() {
    let frames = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&frames);
    let mut app = App::with_pool(serial_pool());
    app.add_systems(move || {
        f.fetch_add(1, Ordering::Relaxed);
    });
    app.run_n_with_delta(5, Duration::from_millis(100));
    assert_eq!(frames.load(Ordering::Relaxed), 5, "Main runs every frame");
    let world = app.world_mut();
    let fixed = world.resource::<FixedTime>();
    assert_eq!(fixed.steps_this_frame(), 0, "no fixed schedule ⇒ permanent 0");
    assert_eq!(fixed.elapsed(), Duration::ZERO, "no accumulation without a fixed schedule");
}
