//! **`BOYKO_PROFILE_ON` arms the profiler** — clause 3 of the host-reachability gate, and the first
//! time in this campaign that a **non-test caller** arms the store.
//!
//! # Why this is a separate file from its own other two clauses
//!
//! Not tidiness. MEASURED: **`EnginePlugins` cannot be built twice in one process** — the second
//! build panics in `register_component_hooks::<boyko_render::light::DirectionalLight>`, because
//! component hooks are process-global and the derive's installation is not idempotent. A separate
//! integration test is a separate BINARY and therefore a separate process, which is the only place a
//! second `EnginePlugins` can exist. The sibling clauses live in `profiling_host_reachable.rs`,
//! which carries the full account of what was found and why the wiring was missing.
//!
//! # What this claims, and what it does not
//!
//! It claims the enable path reaches the store: the flag is read, `arm` runs, and the store comes
//! back armed with a real geometry. It does **not** claim the profiler then records anything — that
//! needs zones running on a claimed lane, which is the store's own tests' subject, and which rung 15
//! learned the hard way is a separate obligation (`set_lane`) invisible from the public API.

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_ecs::ecs::core::profiling::Profiler;

/// **Clause 3** — with the launch flag set, a host's profiler is armed and has committed columns.
///
/// RED: delete `arm_profiler_from_env(app)` from `EnginePlugins::build` ⇒ the store stays disarmed
/// and no host can turn the profiler on, which is the state the whole ladder sat in until this
/// gate — complete, gated, and unreachable.
#[test]
fn the_launch_flag_arms_a_hosts_profiler() {
    // SAFETY: this binary holds exactly one test, so no other thread reads the environment while it
    //   is being set. It is set rather than assumed, because the whole point is that the flag is
    //   what does the work.
    unsafe { std::env::set_var("BOYKO_PROFILE_ON", "1") };

    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("profiling-arm-gate", 64, 64));

    let profiler = app
        .world()
        .try_resource::<Profiler>()
        .expect("EnginePlugins must install the profiling store; see profiling_host_reachable.rs");
    assert!(
        profiler.is_armed(),
        "BOYKO_PROFILE_ON was set and the profiler is still disarmed -- the enable path does not \
         reach the store, so no host can turn the profiler on and fifteen rungs of profiler stay \
         unreachable from anything but a test"
    );
    assert!(
        profiler.zone_stride() > 0,
        "the armed store reports a zero zone stride, so it committed no columns and every cell \
         lookup downstream would answer `None` -- armed in name only"
    );

    // SAFETY: single-threaded; leaves the environment as it was found for anything sharing this
    //   process afterwards.
    unsafe { std::env::remove_var("BOYKO_PROFILE_ON") };
}
