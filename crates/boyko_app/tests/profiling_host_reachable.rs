//! **The profiler is REACHABLE from a host** — the gate for the finding that closed the profiling
//! ladder.
//!
//! # What was measured, and why this file exists
//!
//! After profiling rung 15 the subsystem was complete and **unreachable**. `ProfilerPlugin` was
//! added NOWHERE outside tests: a workspace grep for it returned a re-export, three doc comments and
//! one test. Fifteen rungs — the store, the fold, the GPU channel, the retention tiers, the
//! telemetry writer, the overlay — all read one `Resource` that no host ever inserted.
//!
//! The fold was not the missing piece and never had been: `App::update_with_delta` has called
//! `fold_frame(&mut self.world)` all along. It looked for a `Profiler`, found none, and returned.
//! That is the shape worth naming — **the wiring was one `add_plugin` short, and every individual
//! part of it was green.** Nothing in fifteen rungs of gates could see it, because every one of them
//! built its own world.
//!
//! # What this gate asserts
//!
//! 1. `EnginePlugins` installs the store. Without it the whole subsystem is inert.
//! 2. **Without the flag the store is DISARMED** — the default host pays nothing. `Profiler::new`
//!    *"reserves nothing, commits nothing, calibrates nothing"*, which is what makes clause 1 safe
//!    to do unconditionally, so this clause is the other half of that argument rather than a
//!    separate nicety.
//! 3. **With `BOYKO_PROFILE_ON` the store is ARMED**, with a real zone stride. This is the clause
//!    that makes 1 and 2 more than bookkeeping: it is the first time in the campaign that a
//!    non-test caller arms the profiler.
//!
//! # What it cannot claim
//!
//! Nothing about a *windowed* run: `EnginePlugins::build` is exercised here without `App::run`, so
//! no device is booted and no frame is presented. It claims the composition and the enable path,
//! which is exactly the part that was missing. It also cannot claim the profiler produces samples —
//! that needs zones running on a lane, which the store's own tests cover.
//!
//! # Why the flag leg is a SEPARATE TEST BINARY
//!
//! MEASURED while writing this: **`EnginePlugins` cannot be built twice in one process.** The second
//! build panics in `register_component_hooks::<boyko_render::light::DirectionalLight>` — component
//! hooks are process-global and the derive's installation is not idempotent. The first draft ran
//! both legs in one `#[test]` for a different and wrong reason (that `BOYKO_PROFILE_ON` is
//! process-global, which is true and is not the binding constraint), and it failed on the second
//! `build_app`.
//!
//! So the flag leg lives in `profiling_host_arm_flag.rs`: a separate integration test is a separate
//! BINARY and therefore a separate process, which is the only place a second `EnginePlugins` can be
//! built. The constraint is pre-existing and belongs to the render plugins, not to the profiler —
//! it is written down here because it is invisible until a test builds the host twice.

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_ecs::ecs::core::profiling::Profiler;

/// Builds an `EnginePlugins` app without running it, under a chosen `BOYKO_PROFILE_ON` state.
///
/// `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run` installs
/// the windowed runner, which this never calls.
fn build_app() -> App {
    // SAFETY: this binary holds exactly one test, so no other thread reads the environment while it
    //   is being cleared. Cleared rather than assumed absent: an operator running the suite with
    //   `BOYKO_PROFILE_ON` already set would otherwise silently invert this leg.
    unsafe { std::env::remove_var("BOYKO_PROFILE_ON") };
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("profiling-host-gate", 64, 64));
    app
}

/// **Clause 1 and 2** — the host installs the store, and leaves it disarmed without the flag.
///
/// RED 1: delete `app.add_plugin(ProfilerPlugin)` from `EnginePlugins::build` ⇒ the first assertion
/// fails and the subsystem is back to being unreachable — the exact state this gate was written to
/// end.
///
/// RED 2: make the arm unconditional (drop the `BOYKO_PROFILE_ON` check) ⇒ the second assertion
/// fails, and that is the failure that matters: every host would then commit the reservation and
/// calibrate the clock at startup, which is the syscall the launch flag exists to authorise.
#[test]
fn a_host_installs_the_profiling_store_and_leaves_it_disarmed() {
    let app = build_app();
    let profiler = app
        .world()
        .try_resource::<Profiler>()
        .expect(
            "EnginePlugins did not install the profiling store. Every rung from 2 to 15 reads this \
             one resource, and without it the fold at App::update_with_delta finds nothing and \
             returns -- the whole subsystem is inert while each of its own gates stays green.",
        );
    assert!(
        !profiler.is_armed(),
        "a host with no BOYKO_PROFILE_ON armed the profiler anyway. `arm` is where every one-time \
         cost lives -- the reservation commit, the clock calibration, the per-lane slab publish -- \
         and doing it unasked is the syscall the launch flag exists to authorise."
    );
    assert_eq!(
        profiler.zone_stride(),
        0,
        "a disarmed store must have no geometry; a non-zero stride means something committed"
    );
}
