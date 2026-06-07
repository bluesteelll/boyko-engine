//! Phase 10 Wave D Steps 12-14 — smoke integration tests.
//!
//! Pins three load-bearing invariants of the scheduler refit:
//!
//! 1. [`schedule_run_bumps_change_tick`] — every `Schedule::run` advances
//!    `EcsMaster::current_tick()` by TWO (Bug #56): the frame-start bump
//!    (plan §4.5 PHASE9.1) plus the apply-window bump (`schedule.rs` ~276),
//!    so deferred-command applies stamp at `this_run + 1` and are observed by
//!    `Added<T>` / `Changed<T>` exactly once the next frame.
//! 2. [`schedule_run_dispatches_before_observable_tick`] — the frame's
//!    `change_tick` bumps complete before any system body runs; an
//!    exclusive system observes the post-bump value via
//!    `world.current_tick()` (plan §2.6 SCT4 / §4.5). After Bug #56 the value
//!    it reads is the apply-window tick (`frame_start_this_run + 1`), since
//!    the apply-window bump precedes the executor loop.
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

/// `Schedule::run` MUST bump `EcsMaster::current_tick()` by TWO on each
/// call (Bug #56): the dispatcher-owned frame-start `change_tick.fetch_add`
/// (plan §4.5 PHASE9.1) plus the apply-window `change_tick.fetch_add`
/// (`schedule.rs` ~276) that lands deferred-command stamps at `this_run + 1`.
///
/// # Updated for Bug #56 (was: "exactly once" / 0→1, 1→3)
///
/// This test previously encoded the pre-#56 single-bump regime. The #56 fix
/// adds the apply-window bump, so the per-frame advance is now 2. The
/// `CHECK_TICK_THRESHOLD` doc comment in `change_detection/tick.rs` documents
/// the "~2 Ticks per Schedule::run" regime this test now pins.
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
        Tick::new(2),
        "first Schedule::run must advance current_tick from 0 to 2 (Bug#56: frame-start + apply-window bump)"
    );

    schedule.run(&mut world);
    schedule.run(&mut world);
    assert_eq!(
        world.current_tick(),
        Tick::new(6),
        "three Schedule::run calls must advance current_tick to 6 (2 per frame, Bug#56)"
    );
}

/// The frame's `change_tick.fetch_add`s MUST complete before any system
/// dispatch — every system body that reads `world.current_tick()` sees
/// the new value (plan §4.5 / SCT4).
///
/// This indirectly validates the per-system `set_change_ticks` dispatch
/// step in `Schedule::run` (after the tick bump, before the executor
/// loop) — the body's observation of the new tick proves the bump ran
/// first.
///
/// # Updated for Bug #56
///
/// The world advances by 2 per frame now (frame-start + apply-window). Both
/// bumps execute BEFORE the executor loop (the apply-window bump sits at
/// `schedule.rs` ~276, ahead of `pool.install`), so the body reads the
/// apply-window tick = `frame_start_this_run + 1`. After 4 empty frames the
/// clock is at 8; the probe frame bumps to 9 (frame start) then 10 (apply
/// window), and the body observes 10.
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
    assert_eq!(pre_tick, Tick::new(8), "4 empty frames × 2 bumps/frame (Bug#56) = 8");

    // Now run our probe schedule once.
    schedule.run(&mut world);

    // Expected (Bug#56): world's tick advanced 8 → 10 (frame-start bump to 9,
    // apply-window bump to 10). The apply-window bump precedes the executor
    // loop, so the exclusive body reads the apply-window tick Tick(10).
    assert_eq!(world.current_tick(), Tick::new(10));
    assert_eq!(
        observed.load(Ordering::Relaxed),
        Tick::new(10).get(),
        "system body must observe the post-bump tick value (apply-window tick after Bug#56)"
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
    // sentinel (`current_tick = 1`). Bug#56: 2 bumps/frame ⇒ 9 frames = 18.
    let empty = ScheduleBuilder::new(Arc::clone(&pool)).build(&mut world);
    let mut empty_sched = empty;
    for _ in 0..9 {
        empty_sched.run(&mut world);
    }
    assert_eq!(world.current_tick(), Tick::new(18), "9 empty frames × 2 bumps/frame (Bug#56) = 18");

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
    // MAX_CHANGE_AGE` (mirroring `SystemMeta::new` shape). current_tick == 18.
    let expected = Tick::new(18u32.wrapping_sub(MAX_CHANGE_AGE));
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
    assert_eq!(world.current_tick(), Tick::new(28), "18 + 5 frames × 2 bumps/frame (Bug#56) = 28");

    sys.initialize(&mut world);
    assert_eq!(
        sys.meta().last_run(),
        expected,
        "FS1-like idempotence: re-initialize must NOT clobber ticks"
    );
}
