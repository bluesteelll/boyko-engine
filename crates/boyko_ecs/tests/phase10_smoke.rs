//! Phase 10 Wave D Steps 12-14 — smoke integration tests.
//!
//! Pins three load-bearing invariants of the scheduler refit:
//!
//! 1. [`schedule_run_bumps_change_tick`] — every `Schedule::run` advances
//!    `EcsMaster::current_tick()` exactly once (plan §4.5 PHASE9.1).
//! 2. [`schedule_run_dispatches_before_observable_tick`] — the frame's
//!    `change_tick` bump completes before any system body runs; an
//!    exclusive system observes the post-bump value via
//!    `world.current_tick()` (plan §2.6 SCT4 / §4.5).
//! 3. [`exclusive_system_initialize_refits_meta_ticks_from_world`] — the
//!    Step 14 `ExclusiveFunctionSystem::initialize` refit (Option B)
//!    re-seeds `meta.last_run` / `meta.this_run` against the world's
//!    current tick on first init, replacing the constructor-time
//!    `for_testing` sentinel (plan §15.2 W5).
//!
//! These tests pin the contract through the public API surface only
//! (no `pub(crate)` access). The hand-rolled `System`-impl path and
//! the full `set_change_ticks` dispatch verification land in the
//! Wave E end-to-end suite alongside `Added<T>` / `Changed<T>` filters
//! and Miri.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use boyko_ecs::ecs::core::change_detection::{MAX_CHANGE_AGE, Tick};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::ExclusiveFunctionSystem;
use boyko_ecs::ecs::core::system::system::System;
use boyko_threadpool::ThreadPoolBuilder;

/// `Schedule::run` MUST bump `EcsMaster::current_tick()` by 1 on each
/// call — this is the single dispatcher-owned `change_tick.fetch_add`
/// site (plan §4.5 PHASE9.1).
#[test]
fn schedule_run_bumps_change_tick() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let tick_before = world.current_tick();
    assert_eq!(
        tick_before,
        Tick::new(0),
        "fresh EcsMaster starts at Tick(0)"
    );

    let builder = ScheduleBuilder::new(pool);
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(
        world.current_tick(),
        Tick::new(1),
        "first Schedule::run must advance current_tick from 0 to 1"
    );

    schedule.run(&mut world);
    schedule.run(&mut world);
    assert_eq!(
        world.current_tick(),
        Tick::new(3),
        "three Schedule::run calls must advance current_tick to 3"
    );
}

/// The frame's `change_tick.fetch_add` MUST complete before any system
/// dispatch — every system body that reads `world.current_tick()` sees
/// the new value (plan §4.5 / SCT4).
///
/// This indirectly validates the per-system `set_change_ticks` dispatch
/// step in `Schedule::run` (after the tick bump, before the executor
/// loop) — the body's observation of the new tick proves the bump ran
/// first.
#[test]
fn schedule_run_dispatches_before_observable_tick() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let observed = Arc::new(AtomicU32::new(u32::MAX));
    let observed_cl = Arc::clone(&observed);

    let mut builder = ScheduleBuilder::new(pool);
    // Exclusive system body: receives `&mut EcsMaster`; reads the world's
    // tick at dispatch time. The tick at that moment must equal the
    // value the dispatcher published via `bump_change_tick` at the top
    // of `Schedule::run`.
    builder.add_system(move |world: &mut EcsMaster| {
        observed_cl.store(world.current_tick().get(), Ordering::Relaxed);
    });

    let mut schedule = builder.build(&mut world);

    // Pre-bump the world clock by 4 frames against an empty schedule —
    // ensures the observed value cannot be confused with the initial
    // Tick(0).
    let empty_pool = ThreadPoolBuilder::new().num_threads(1).build();
    for _ in 0..4 {
        let mut empty_sched = ScheduleBuilder::new(Arc::clone(&empty_pool))
            .build(&mut world);
        empty_sched.run(&mut world);
    }

    let pre_tick = world.current_tick();
    assert_eq!(pre_tick, Tick::new(4));

    // Now run our probe schedule once.
    schedule.run(&mut world);

    // Expected: world's tick advanced from 4 → 5; the system body
    // observed Tick(5).
    assert_eq!(world.current_tick(), Tick::new(5));
    assert_eq!(
        observed.load(Ordering::Relaxed),
        Tick::new(5).get(),
        "system body must observe the post-bump tick value"
    );
}

/// `ExclusiveFunctionSystem::initialize` (Step 14 refit, Option B) MUST
/// re-seed `meta.last_run` / `meta.this_run` to `world.current_tick() -
/// MAX_CHANGE_AGE` on first init, replacing the Wave A `for_testing`
/// sentinel (plan §15.2 W5).
///
/// We probe directly on a hoisted `ExclusiveFunctionSystem` (publicly
/// exported) to read its meta after `initialize` returns.
#[test]
fn exclusive_system_initialize_refits_meta_ticks_from_world() {
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut world = EcsMaster::new();

    // Pre-bump the world clock so the refit value (read from the world
    // at initialize time) is distinguishable from the `for_testing`
    // sentinel (`current_tick = 1`).
    let empty = ScheduleBuilder::new(Arc::clone(&pool)).build(&mut world);
    let mut empty_sched = empty;
    for _ in 0..9 {
        empty_sched.run(&mut world);
    }
    assert_eq!(world.current_tick(), Tick::new(9));

    // Build an ExclusiveFunctionSystem and call `initialize` directly.
    let mut sys = ExclusiveFunctionSystem::new(|_w: &mut EcsMaster| {});

    // Pre-init meta: `for_testing` sentinel = `Tick(1) - MAX_CHANGE_AGE`.
    let pre_init_last = sys.meta().last_run();
    let pre_init_this = sys.meta().this_run();
    let sentinel = Tick::new(1u32.wrapping_sub(MAX_CHANGE_AGE));
    assert_eq!(
        pre_init_last, sentinel,
        "pre-init meta carries `for_testing` sentinel (1 - MAX_CHANGE_AGE)"
    );
    assert_eq!(pre_init_this, sentinel);

    // Run initialize — the refit fires.
    sys.initialize(&mut world);

    // Post-init meta: both ticks should be `world.current_tick() -
    // MAX_CHANGE_AGE` (mirroring `SystemMeta::new` shape).
    let expected = Tick::new(9u32.wrapping_sub(MAX_CHANGE_AGE));
    assert_eq!(
        sys.meta().last_run(),
        expected,
        "refit: last_run = world.current_tick() - MAX_CHANGE_AGE"
    );
    assert_eq!(
        sys.meta().this_run(),
        expected,
        "refit: this_run = world.current_tick() - MAX_CHANGE_AGE (pre-first-run sentinel)"
    );

    // Idempotence: re-initialize MUST NOT overwrite the ticks (FS1-like
    // semantic) — `Schedule::run` may have written new values between
    // build and a hypothetical re-init.
    //
    // We simulate by manually mutating-by-running (advance world ticks
    // and re-init). The internal `initialized` flag guards the refit;
    // the meta ticks stay at the first-init values.
    let empty2 = ScheduleBuilder::new(pool).build(&mut world);
    let mut empty_sched2 = empty2;
    for _ in 0..5 {
        empty_sched2.run(&mut world);
    }
    assert_eq!(world.current_tick(), Tick::new(14));

    sys.initialize(&mut world);
    assert_eq!(
        sys.meta().last_run(),
        expected,
        "FS1-like idempotence: re-initialize must NOT clobber ticks"
    );
}
