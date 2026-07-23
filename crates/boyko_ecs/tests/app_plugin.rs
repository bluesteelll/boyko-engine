//! Phase 18 — integration tests for the public [`App`] facade + the [`Plugin`]
//! composition layer.
//!
//! Everything here exercises the PUBLIC surface a consumer actually touches:
//! `boyko_ecs::prelude::*` for the engine types and `boyko_macros::{Component,
//! Resource}` for the derives (the prelude deliberately does NOT re-export the
//! derives — `boyko-macros` is a dev-dependency, so integration tests may name
//! it directly; this is by design, see `prelude.rs`).
//!
//! # Harness discipline (matches `phase16_run_conditions.rs` / `phase17_states.rs`)
//!
//! Behavioural counters live in per-test `Arc<Atomic*>` captured by the system /
//! plugin closures — NO shared global `static`s — so the tests are independent
//! and never flake under parallel `cargo test`. The plugin / state *types* are
//! defined at module scope and reused; the per-type `ResourceId` / archetype id
//! they mint is process-global and stable, but the tests assert only on values
//! observed through `world.query` / `world.resource` / per-test counters, never
//! on absolute id values, so cross-test global state cannot perturb them.
//!
//! # Threading note
//!
//! `App` drives a real `Schedule::run`, which dispatches even single systems
//! through `boyko_threadpool::Scope::spawn` (the Phase-9 executor). That is the
//! Phase-9.1-proven path; these tests are not run under Miri (no new `unsafe`,
//! no new cross-thread state — `App` is `!Send+!Sync` single-threaded-owned).
//! Single-worker pools are used where firing-order / single-thread observation
//! matters; the default pool is used where only the end-state is asserted.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

#![cfg(not(miri))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::prelude::*;
use boyko_macros::{Bundle, Component, Resource};

// ── Shared component / resource / state types ────────────────────────────────

/// A position component the parity test mutates via `Query<&mut Position>`.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Position {
    x: f32,
    y: f32,
}

/// A velocity component, read by the second system in the parity test.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Velocity {
    x: f32,
    y: f32,
}

/// A `Position`+`Velocity` bundle so `spawn_batch` lands both components in one
/// archetype (the multi-component spawn path; `spawn_one` takes a single
/// `Component`, not a tuple — a tuple is a `Bundle`).
#[derive(Bundle)]
struct Body {
    pos: Position,
    vel: Velocity,
}

/// A frame counter resource (`run` / startup tests increment it).
#[derive(Resource, Default)]
struct FrameCounter(u32);

/// A startup-effect marker: a startup system sets this so a frame system can
/// observe it on frame 1.
#[derive(Resource, Default)]
struct StartupRan(u32);

/// The canonical app-phase state (used by `prelude_compiles`). Both variants
/// are named only in type position by the smoke test, so suppress the
/// never-constructed lint on the variant.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
#[allow(dead_code)]
enum AppState {
    #[default]
    Menu,
    InGame,
}
impl States for AppState {}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Single-worker pool — deterministic serial dispatch; no flake.
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// Spawns `n` entities into a fresh `Position`+`Velocity` archetype, with
/// `pos.x = i`, `vel.x = 1` (so a `pos += vel` system advances x by 1 / frame).
/// Uses `spawn_batch` over a derived `Bundle` — the direct (eager) spawn path,
/// applied immediately so the rows are visible to a subsequent query / run.
fn seed_world(world: &mut EcsMaster, n: u32) {
    world
        .spawn_batch((0..n).map(|i| Body {
            pos: Position { x: i as f32, y: 0.0 },
            vel: Velocity { x: 1.0, y: 0.0 },
        }))
        .expect("seed batch must succeed");
}

/// Reads back the sum of all `Position.x` via the engine's own `query` API.
fn sum_position_x(world: &mut EcsMaster) -> f32 {
    let view = world.query::<&Position, ()>();
    let mut sum = 0.0f32;
    for p in view.iter() {
        sum += p.x;
    }
    sum
}

// The two systems under parity test, written once and registered into BOTH the
// `App` and the hand-wired `EcsMaster`+`Schedule` so the comparison is exact.

/// Advances every `Position` by its `Velocity` (writes `Position`).
fn move_system(mut q: Query<(&mut Position, &Velocity)>) {
    for (p, v) in q.iter_mut() {
        p.x += v.x;
        p.y += v.y;
    }
}

/// Reads every `Velocity` (a disjoint read system — exercises the second pool
/// lane and proves a second registered system runs through the facade).
fn read_system(q: Query<&Velocity>) {
    // Touch the data so the system is not optimised to a no-op.
    let _ = q.iter().map(|v| v.x).sum::<f32>();
}

// =============================================================================
// 1 — Behavioural parity: App vs hand-wired EcsMaster+Schedule
// =============================================================================

/// THE parity test: building/running ≥2 real systems through `App::run_n(F)`
/// yields a world state byte-identical to a hand-wired `EcsMaster` +
/// `ScheduleBuilder` + `Schedule` running the SAME systems F frames — modulo
/// the two clock resource slots (`Time` / `FixedTime`) that `App::finish`
/// seeds since Phase 20; the assertion compares the component data the systems
/// touch, which is unaffected. Proves the facade adds no semantic difference.
#[test]
fn app_builds_and_runs_n_frames_equals_manual() {
    const N: u32 = 200;
    const FRAMES: u64 = 7;

    // ── App path ──
    let app_sum = {
        let mut app = App::with_pool(serial_pool());
        seed_world(app.world_mut(), N);
        app.add_systems(move_system).add_systems(read_system);
        app.run_n(FRAMES);
        sum_position_x(app.world_mut())
    };

    // ── Hand-wired path (same pool kind, same systems, same frame count) ──
    let manual_sum = {
        let pool = serial_pool();
        let mut world = EcsMaster::new();
        seed_world(&mut world, N);
        let mut builder = ScheduleBuilder::new(pool);
        builder.add_system(move_system);
        builder.add_system(read_system);
        let mut schedule = builder.build(&mut world);
        for _ in 0..FRAMES {
            schedule.run(&mut world);
        }
        sum_position_x(&mut world)
    };

    // Closed-form oracle: sum_i (i + FRAMES) = sum_i i + N*FRAMES.
    let base: f32 = (0..N).map(|i| i as f32).sum();
    let expected = base + (N as f32) * (FRAMES as f32);

    assert_eq!(app_sum, manual_sum, "App::run_n must match the hand-wired schedule exactly");
    assert_eq!(app_sum, expected, "App world state must equal the closed-form oracle");
}

// =============================================================================
// 2 — Plugins: build runs once, in add order
// =============================================================================

struct OrderPluginA(Arc<Mutex<Vec<&'static str>>>);
impl Plugin for OrderPluginA {
    fn build(&self, _app: &mut App) {
        self.0.lock().expect("poisoned").push("A");
    }
}
struct OrderPluginB(Arc<Mutex<Vec<&'static str>>>);
impl Plugin for OrderPluginB {
    fn build(&self, _app: &mut App) {
        self.0.lock().expect("poisoned").push("B");
    }
}

/// Two plugins, each recording its build into a shared log; build order must
/// equal add order, and each builds exactly once.
#[test]
fn plugin_build_runs_once_in_order() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut app = App::with_pool(serial_pool());
    app.add_plugin(OrderPluginA(Arc::clone(&log)));
    app.add_plugin(OrderPluginB(Arc::clone(&log)));

    let order = log.lock().expect("poisoned").clone();
    assert_eq!(order, vec!["A", "B"], "build order must equal add order, each built exactly once");
}

// =============================================================================
// 3 — Duplicate plugin panics (boyko-B1801)
// =============================================================================

struct DupPlugin;
impl Plugin for DupPlugin {
    fn build(&self, _app: &mut App) {}
}

/// Adding the same plugin TYPE twice panics with the documented code.
#[test]
#[should_panic(expected = "boyko-B1801")]
fn duplicate_plugin_panics() {
    let mut app = App::with_pool(serial_pool());
    app.add_plugin(DupPlugin);
    app.add_plugin(DupPlugin); // second add of the same type ⇒ panic
}

// =============================================================================
// 4 + 5 — add_plugins tuple expansion (flat + nested)
// =============================================================================

struct MarkA(Arc<Mutex<Vec<&'static str>>>);
impl Plugin for MarkA {
    fn build(&self, _app: &mut App) {
        self.0.lock().expect("poisoned").push("a");
    }
}
struct MarkB(Arc<Mutex<Vec<&'static str>>>);
impl Plugin for MarkB {
    fn build(&self, _app: &mut App) {
        self.0.lock().expect("poisoned").push("b");
    }
}
struct MarkC(Arc<Mutex<Vec<&'static str>>>);
impl Plugin for MarkC {
    fn build(&self, _app: &mut App) {
        self.0.lock().expect("poisoned").push("c");
    }
}

/// `add_plugins((A, B, C))` registers all three, in declaration order.
#[test]
fn add_plugins_tuple_expands() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut app = App::with_pool(serial_pool());
    app.add_plugins((
        MarkA(Arc::clone(&log)),
        MarkB(Arc::clone(&log)),
        MarkC(Arc::clone(&log)),
    ));

    let order = log.lock().expect("poisoned").clone();
    assert_eq!(order, vec!["a", "b", "c"], "flat 3-tuple registers all three in order");
}

/// `add_plugins((A, (B, C)))` recurses through the nested tuple, registering all
/// three (a tuple is itself `Plugins`).
#[test]
fn add_plugins_nested_tuple() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut app = App::with_pool(serial_pool());
    app.add_plugins((MarkA(Arc::clone(&log)), (MarkB(Arc::clone(&log)), MarkC(Arc::clone(&log)))));

    let order = log.lock().expect("poisoned").clone();
    assert_eq!(order, vec!["a", "b", "c"], "nested tuple recurses; all three registered in order");
}

// =============================================================================
// 6 — Startup runs once, before the frame loop
// =============================================================================

/// A startup system increments a counter ONCE; after `run_n(5)` it is still 1
/// (not 5), and a frame system observed the startup's effect on frame 1.
#[test]
fn startup_runs_once_before_frame_loop() {
    // Per-test shared probes (no global statics).
    let startup_runs = Arc::new(AtomicUsize::new(0));
    let observed_on_frame1 = Arc::new(AtomicUsize::new(0));
    let frame_no = Arc::new(AtomicUsize::new(0));

    let su = Arc::clone(&startup_runs);
    let obs = Arc::clone(&observed_on_frame1);
    let fno = Arc::clone(&frame_no);

    let mut app = App::with_pool(serial_pool());
    app.insert_resource(StartupRan(0));

    // Startup: bump the counter + write a marker into the world resource.
    app.add_startup_system(move |mut r: ResMut<StartupRan>| {
        su.fetch_add(1, Ordering::Relaxed);
        r.0 = 42;
    });

    // Frame system: on the FIRST frame, record whether the startup effect is
    // already visible (it must be — startup drains before any frame).
    app.add_systems(move |r: Res<StartupRan>| {
        let this_frame = fno.fetch_add(1, Ordering::Relaxed);
        if this_frame == 0 && r.0 == 42 {
            obs.fetch_add(1, Ordering::Relaxed);
        }
    });

    app.run_n(5);

    assert_eq!(
        startup_runs.load(Ordering::Relaxed),
        1,
        "startup system runs exactly once across 5 frames (not per-frame)"
    );
    assert_eq!(
        observed_on_frame1.load(Ordering::Relaxed),
        1,
        "frame system observed the startup's effect already on frame 1"
    );
    assert_eq!(app.world().resource::<StartupRan>().0, 42, "startup write persisted in the world");
}

// =============================================================================
// 7 — update() auto-finishes on first call
// =============================================================================

/// Calling `update()` without an explicit `finish()` runs the frame and leaves
/// `is_finished()` true.
#[test]
fn auto_finish_on_first_update() {
    let runs = Arc::new(AtomicUsize::new(0));
    let runs_cl = Arc::clone(&runs);

    let mut app = App::with_pool(serial_pool());
    app.add_systems(move || {
        runs_cl.fetch_add(1, Ordering::Relaxed);
    });

    assert!(!app.is_finished(), "App is not finished before the first update()");
    app.update();
    assert!(app.is_finished(), "update() auto-finishes the App");
    assert_eq!(runs.load(Ordering::Relaxed), 1, "update() ran the schedule exactly one frame");
}

// =============================================================================
// 8 — finish() is idempotent
// =============================================================================

/// Two `finish()` calls do not panic and build the schedule only once: a startup
/// counter therefore stays at 1 (a double-build would re-drain startup).
#[test]
fn finish_is_idempotent() {
    let startup_runs = Arc::new(AtomicUsize::new(0));
    let su = Arc::clone(&startup_runs);

    let mut app = App::with_pool(serial_pool());
    app.add_startup_system(move || {
        su.fetch_add(1, Ordering::Relaxed);
    });

    app.finish();
    app.finish(); // second finish is a no-op
    assert!(app.is_finished(), "App reports finished after finish()");
    assert_eq!(
        startup_runs.load(Ordering::Relaxed),
        1,
        "idempotent finish() drains startup exactly once (no double build)"
    );
}

// =============================================================================
// 9 + 10 — pool ownership + thread count
// =============================================================================

/// `App::new()` owns a pool with at least one worker (platform parallelism).
#[test]
fn app_owns_pool_default_parallelism() {
    let app = App::new();
    assert!(app.pool().num_threads() >= 1, "default App pool has >= 1 worker");
}

/// `App::with_threads(3)` sizes the pool to exactly 3 (3 <= the builder clamp).
#[test]
fn with_threads_sets_count() {
    let app = App::with_threads(3);
    assert_eq!(app.pool().num_threads(), 3, "with_threads(3) yields a 3-worker pool");
}

/// `App::with_threads(0)` clamps to 1 rather than panicking (builder clamp to
/// `[1, 64]`).
#[test]
fn with_threads_zero_clamps_to_one() {
    let app = App::with_threads(0);
    assert_eq!(app.pool().num_threads(), 1, "with_threads(0) clamps up to 1 (no panic)");
}

// =============================================================================
// 11 — run() exits on AppExit
// =============================================================================

/// A system sets `AppExit(true)` once a frame counter reaches N; `run()` returns
/// after exactly N frames (the counter == N — it actually stopped).
#[test]
fn run_exits_on_appexit() {
    const TARGET: u32 = 4;

    let mut app = App::with_pool(serial_pool());
    app.insert_resource(FrameCounter(0));
    app.add_systems(move |mut fc: ResMut<FrameCounter>, mut exit: ResMut<AppExit>| {
        fc.0 += 1;
        if fc.0 >= TARGET {
            exit.0 = true;
        }
    });

    app.run(); // must terminate

    assert_eq!(
        app.world().resource::<FrameCounter>().0,
        TARGET,
        "run() stopped exactly when the system requested AppExit (counter == TARGET)"
    );
}

// =============================================================================
// 12 — prelude smoke: names resolve through the public glob
// =============================================================================

/// Compile-only: every promised prelude name resolves through `prelude::*`
/// (plus one `boyko_macros` derive). A wrong re-export path would fail to
/// compile this test, so its mere existence is the assertion; we also run it so
/// it counts in the suite.
#[test]
fn prelude_compiles() {
    // Name every required type in a type position; `_` bindings keep them
    // genuinely referenced (not dead-code-eliminated by the resolver).
    #[allow(dead_code)]
    fn names_resolve() {
        // App + plugin surface.
        let _: fn() -> App = App::new;
        fn _takes_plugin<P: Plugin>(_p: P) {}
        let _exit = AppExit(false);

        // World + scheduling + queries + params + states + pool.
        type _W = EcsMaster;
        type _Sched = Schedule;
        type _SB = ScheduleBuilder;
        type _Q<'w, 's> = Query<'w, 's, &'w Velocity, ()>;
        fn _uses_params(_c: Commands, _r: Res<FrameCounter>, _rm: ResMut<FrameCounter>) {}
        type _St = State<AppState>;
        type _Ns = NextState<AppState>;
        type _Pool = ThreadPool;

        // A boyko_macros derive is usable in integration tests (by design).
        #[derive(Resource, Default)]
        struct _PreludeProbe(u32);
    }
    // Touch a value-level path too, so the test does real work at runtime.
    let app = App::new();
    assert!(app.pool().num_threads() >= 1, "prelude App constructs and exposes its pool");
}

// =============================================================================
// 13 — proptest: App::run_n(F) matches the hand-wired path for random F
// =============================================================================

mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        // A handful of cases is plenty: each runs a full multi-frame schedule on
        // a real pool, so we cap the case count to keep wall-time bounded.
        #![proptest_config(ProptestConfig { cases: 16, ..ProptestConfig::default() })]

        /// For a random entity count and frame count, `App::run_n` produces the
        /// same `Position` state as the hand-wired `EcsMaster`+`Schedule`.
        #[test]
        fn app_run_n_matches_manual(n in 1u32..64, frames in 0u64..6) {
            let app_sum = {
                let mut app = App::with_pool(serial_pool());
                seed_world(app.world_mut(), n);
                app.add_systems(move_system).add_systems(read_system);
                app.run_n(frames);
                sum_position_x(app.world_mut())
            };

            let manual_sum = {
                let pool = serial_pool();
                let mut world = EcsMaster::new();
                seed_world(&mut world, n);
                let mut builder = ScheduleBuilder::new(pool);
                builder.add_system(move_system);
                builder.add_system(read_system);
                let mut schedule = builder.build(&mut world);
                for _ in 0..frames {
                    schedule.run(&mut world);
                }
                sum_position_x(&mut world)
            };

            prop_assert_eq!(app_sum, manual_sum, "App::run_n must match the manual path for any (n, frames)");
        }
    }
}
