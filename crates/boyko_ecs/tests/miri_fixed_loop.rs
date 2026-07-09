//! Phase 20 — Miri validation for the fixed-timestep loop (plan W4 Miri row).
//!
//! Run under:
//!
//! ```powershell
//! $env:MIRIFLAGS = "-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-ecs --test miri_fixed_loop
//! ```
//!
//! `-Zmiri-ignore-leaks` for the usual `Arc<ThreadPool>` worker-thread
//! shutdown report (harness check, not UB — the miri_phase16 note).
//!
//! Everything here drives `App::update_with_delta` / `fixed_advance` ONLY —
//! never the self-clocked `update()` (`Instant::now` needs
//! `-Zmiri-disable-isolation`; plan D11). The accumulate/expend/clamp
//! bookkeeping is the SAME code on native and under Miri (plan D3, the X.I D9
//! unified-path lesson); the App path additionally walks the ★C1 margin check
//! and the D6 event gate every frame.

#![cfg(miri)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::prelude::*;

/// One 64 Hz step.
const STEP: Duration = Duration::from_nanos(15_625_000);

/// M-P20-1: `fixed_advance` bookkeeping with a counting closure — no pool, no
/// App, pure resource math under Tree Borrows (accumulate, expend, snapshot,
/// steps_this_frame write-back).
#[test]
fn miri_fixed_advance_counting_closure() {
    let mut world = EcsMaster::new();
    world.insert_resource(Time::default());
    world.insert_resource(FixedTime::default());

    let mut steps_seen = 0u32;

    // Frame 1: 2.5 steps of delta ⇒ 2 steps, overstep 0.5 step.
    world.resource_mut::<Time>().advance_with(STEP * 5 / 2);
    let n = fixed_advance(&mut world, |_w| steps_seen += 1);
    assert_eq!(n, 2);
    assert_eq!(steps_seen, 2);

    // Frame 2: 0.5 more ⇒ the carried overstep completes exactly 1 step.
    world.resource_mut::<Time>().advance_with(STEP / 2);
    let n = fixed_advance(&mut world, |_w| steps_seen += 1);
    assert_eq!(n, 1, "carried overstep must complete the step");
    assert_eq!(steps_seen, 3);
    let fixed = world.resource::<FixedTime>();
    assert_eq!(fixed.overstep(), Duration::ZERO);
    assert_eq!(fixed.elapsed(), STEP * 3);
}

/// M-P20-2: the full App driver under Miri — a real 1-worker pool, a tiny
/// fixed system, a 0-substep hold frame, and a multi-substep frame (the D6
/// gate branch + ★C1 margin check + two-bump-per-run contract all walked
/// under TB).
///
/// `#[ignore]` by default: on windows-gnu Miri the executor's worker
/// park/unpark loop makes 4 `Schedule::run`s burn >40 CPU-minutes (the
/// `miri_phase_bugfix_56` wall-time class — environmental, not UB; the
/// executor's own TB coverage lives in `miri_schedule_parallel.rs`). Phase 20
/// adds ZERO unsafe, and everything driver-specific that Miri can check is
/// covered pool-less by M-P20-1. Run explicitly with
/// `cargo miri test … -- --ignored` on a fast box if desired.
#[test]
#[ignore = "windows-gnu Miri executor wall-time (bugfix_56 class); driver logic covered by M-P20-1"]
fn miri_app_driver_substeps_and_hold() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);

    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut app = App::with_pool(pool);
    app.add_systems_in(CoreSchedule::Fixed, move || {
        c.fetch_add(1, Ordering::Relaxed);
    });

    app.update_with_delta(STEP); // 1 substep
    app.update_with_delta(Duration::from_micros(1)); // 0 substeps (hold path)
    app.update_with_delta(STEP * 2); // 2 substeps
    assert_eq!(counter.load(Ordering::Relaxed), 3, "1 + 0 + 2 substeps");

    let world = app.world_mut();
    let fixed = world.resource::<FixedTime>();
    assert_eq!(fixed.steps_this_frame(), 2, "last frame expended 2");
    assert!(fixed.overstep() < Duration::from_nanos(15_625_000));
}
