//! Phase 17 — integration tests for application/game states.
//!
//! # Miri
//!
//! This whole file is `#![cfg(not(miri))]`. Every active test drives
//! `Schedule::run`, which dispatches its (non-exclusive `ResMut`-writing)
//! systems through `boyko_threadpool::Scope::spawn` — even on a
//! `num_threads(1)` pool — and that worker raw-pointer handshake hits the
//! known Tree-Borrows protected-tag conflict deferred to Phase 9.1 (see
//! `miri_phase9.rs` / `miri_phase16.rs`). The Miri-clean validation of the
//! state algorithm lives elsewhere: the in-crate `apply_state_transition`
//! direct-drive unit tests (`state/transition_record.rs`, no pool) and
//! `tests/miri_phase17.rs` (the public resource-surface reborrows).
//!
//! # IMPORTANT — two halves
//!
//! This file is split into two parts:
//!
//! 1. **Active state-machine tests** (always compiled): drive the built-in
//!    transition pass through `Schedule::run` and observe the result by reading
//!    `EcsMaster::state::<S>()` / `resource::<NextState<S>>()` directly — WITHOUT
//!    the run conditions. These cover the core transition algorithm (§5.1):
//!    initial synthesis (D7), identity no-op (D6), last-write-wins /
//!    one-transition-per-frame, fire-exactly-once, orthogonal-state
//!    independence (D9), the direct `set_next_state` API, M2 idempotency, and
//!    the 0-cost-when-unused smoke. This is the `apply_state_transition` path
//!    the plan §9 calls Miri-testable.
//!
//! 2. **Condition tests** (in the `condition_gated` submodule, always compiled):
//!    the plan §9 tests that wire `in_state` / `on_enter` / `on_exit` /
//!    `on_transition` through `.run_if(...)`. These were previously **BLOCKED by
//!    a compile bug** (bug F1): the public condition functions returned an opaque
//!    `impl FnMut(Res<..>) -> bool` whose bound was insufficient to satisfy the
//!    `SystemParamFunction` double-`FnMut` HRTB bound, so `.run_if(in_state(..))`
//!    did not compile. The developer fixed it by returning `impl System<Out =
//!    bool>` (a concrete System trait that survives the opaque-return boundary)
//!    re-bridged to `IntoSystem` via an identity blanket impl. These tests now
//!    compile + run by default.
//!
//! # Harness discipline (matches `phase16_run_conditions.rs`)
//!
//! Every test uses per-test `Arc<Atomic*>` counters / a per-test world — NO
//! shared global `static`s, so the tests are independent and never flake under
//! parallel `cargo test`. State *types* are defined at module scope and reused;
//! the per-`S` `ResourceId` they mint is process-global and stable, but the
//! tests only assert behaviour observed through `state::<S>()` / counters, never
//! absolute id values, so cross-test global state cannot perturb them.

#![cfg(not(miri))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::state::{NextState, States};
use boyko_ecs::ecs::core::system::ResMut;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

// ── Shared harness ───────────────────────────────────────────────────────────

/// Single-worker pool — serial dispatch ⇒ deterministic firing order, no flake.
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// A fresh shared `usize` counter.
fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

fn load(c: &Arc<AtomicUsize>) -> usize {
    c.load(Ordering::Relaxed)
}

// ── Shared state types (used across several tests) ────────────────────────────

/// The canonical app-phase state. `Default == Menu` so `init_state` starts here.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
enum AppState {
    #[default]
    Menu,
    InGame,
    Paused,
}
impl States for AppState {}

/// An orthogonal network-phase state, for the independence test.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
enum NetState {
    #[default]
    Offline,
    Online,
}
impl States for NetState {}

// =============================================================================
// PART 1 — Active state-machine tests (no run conditions; observe state())
// =============================================================================

/// `init_state` starts the world at `Default` (Menu). The first `run` applies
/// the transition pass (synthesizing `none → Menu`), leaving the state at Menu.
///
/// Covers the initial-state half of plan §9 `initial_on_enter_fires_once_at_startup`
/// at the state-value level (the `on_enter`-counter half is blocked, see PART 2).
#[test]
fn init_state_starts_at_default_and_survives_runs() {
    let pool = serial_pool();
    let mut builder = ScheduleBuilder::new(pool);
    builder.init_state::<AppState>(); // initial = Menu (Default)
    builder.add_system(|| {}); // a no-op system so the schedule is non-empty

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    // Even before the first run, the resource exists at its initial value
    // (inserted at build).
    assert!(*world.state::<AppState>() == AppState::Menu, "registered at Menu");

    for _ in 0..3 {
        schedule.run(&mut world);
        assert!(
            *world.state::<AppState>() == AppState::Menu,
            "no transition requested ⇒ state stays Menu across frames"
        );
    }
}

/// `insert_state(value)` registers the state at an explicit initial value (not
/// `Default`). Confirms the builder path threads the captured initial through.
#[test]
fn insert_state_uses_explicit_initial() {
    let pool = serial_pool();
    let mut builder = ScheduleBuilder::new(pool);
    builder.insert_state::<AppState>(AppState::Paused); // explicit, not Default
    builder.add_system(|| {});

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    assert!(*world.state::<AppState>() == AppState::Paused, "explicit initial = Paused");
    schedule.run(&mut world);
    assert!(
        *world.state::<AppState>() == AppState::Paused,
        "no transition ⇒ stays at the explicit initial"
    );
}

/// A system requests `NextState=InGame` on frame 1 (via `ResMut<NextState>`).
/// The transition pass at the TOP of frame 2 swaps `State` to InGame. Covers
/// the state-swap timing of plan §9 `transition_fires_enter_and_exit_on_right_frame`.
#[test]
fn next_state_request_applies_on_following_frame() {
    let pool = serial_pool();

    let mut builder = ScheduleBuilder::new(pool);
    builder.init_state::<AppState>();

    // Request exactly once, on the first frame this system runs.
    let requested = Arc::new(AtomicUsize::new(0));
    let requested_cl = Arc::clone(&requested);
    builder.add_system(move |mut next: ResMut<NextState<AppState>>| {
        if requested_cl.fetch_add(1, Ordering::Relaxed) == 0 {
            next.set(AppState::InGame);
        }
    });

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    // Frame 1: the request is queued during the frame; the pass at the TOP of
    // frame 1 ran before the system, so the state is still Menu after frame 1.
    schedule.run(&mut world);
    assert!(
        *world.state::<AppState>() == AppState::Menu,
        "frame 1: request queued mid-frame ⇒ state still Menu (pass ran at the top)"
    );

    // Frame 2: the pass at the top drains the request ⇒ state is InGame.
    schedule.run(&mut world);
    assert!(
        *world.state::<AppState>() == AppState::InGame,
        "frame 2: the queued request is applied at the top ⇒ state is InGame"
    );

    // NextState was drained back to Unchanged.
    assert!(
        matches!(world.resource::<NextState<AppState>>(), NextState::Unchanged),
        "NextState drained to Unchanged after the transition applied"
    );
}

/// `set(current_value)` records NO transition (D6): the `State` is unchanged and
/// `NextState` is drained to `Unchanged`. State-level half of plan §9
/// `identity_transition_is_noop`.
#[test]
fn identity_transition_is_noop() {
    let pool = serial_pool();

    let mut builder = ScheduleBuilder::new(pool);
    builder.init_state::<AppState>(); // Menu

    // Request set(Menu) — the SAME value as current — on frame 2 only.
    let frame = Arc::new(AtomicUsize::new(0));
    let frame_cl = Arc::clone(&frame);
    builder.add_system(move |mut next: ResMut<NextState<AppState>>| {
        if frame_cl.fetch_add(1, Ordering::Relaxed) == 1 {
            next.set(AppState::Menu); // identity request
        }
    });

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world); // frame 1
    schedule.run(&mut world); // frame 2: identity set(Menu) queued
    schedule.run(&mut world); // frame 3: identity request drained at the top

    assert!(
        *world.state::<AppState>() == AppState::Menu,
        "identity set(Menu) leaves the state unchanged (D6 no-op)"
    );
    assert!(
        matches!(world.resource::<NextState<AppState>>(), NextState::Unchanged),
        "the identity request is still drained to Unchanged"
    );
}

/// In one frame, `set(InGame); set(Paused)` keeps only `Paused`: the final state
/// is `Paused` — exactly one transition per frame, last-write-wins. State-level
/// half of plan §9 `last_write_wins_one_transition_per_frame`.
#[test]
fn last_write_wins_one_transition_per_frame() {
    let pool = serial_pool();

    let mut builder = ScheduleBuilder::new(pool);
    builder.init_state::<AppState>(); // Menu

    let frame = Arc::new(AtomicUsize::new(0));
    let frame_cl = Arc::clone(&frame);
    builder.add_system(move |mut next: ResMut<NextState<AppState>>| {
        if frame_cl.fetch_add(1, Ordering::Relaxed) == 0 {
            // Two sets in one frame — last (Paused) wins.
            next.set(AppState::InGame);
            next.set(AppState::Paused);
        }
    });

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world); // frame 1: both sets queued (Paused wins)
    assert!(
        *world.state::<AppState>() == AppState::Menu,
        "frame 1: still Menu (request applies next frame)"
    );

    schedule.run(&mut world); // frame 2: Menu→Paused applied (not InGame)
    assert!(
        *world.state::<AppState>() == AppState::Paused,
        "the last-written value (Paused) wins; InGame was overwritten before the pass drained it"
    );
}

/// Drives a Menu→InGame→Paused walk via the direct `set_next_state` API between
/// frames, asserting the state value tracks each transition exactly. State-level
/// coverage of plan §9 `in_state_gates_systems` + `set_next_state_from_direct_api`.
#[test]
fn set_next_state_from_direct_api_drives_walk() {
    let pool = serial_pool();
    let mut builder = ScheduleBuilder::new(pool);
    builder.init_state::<AppState>();
    builder.add_system(|| {});

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert!(*world.state::<AppState>() == AppState::Menu, "frame 1: Menu");

    world.set_next_state::<AppState>(AppState::InGame);
    schedule.run(&mut world);
    assert!(*world.state::<AppState>() == AppState::InGame, "frame 2: InGame");

    world.set_next_state::<AppState>(AppState::Paused);
    schedule.run(&mut world);
    assert!(*world.state::<AppState>() == AppState::Paused, "frame 3: Paused");

    // Back to Menu — full cycle.
    world.set_next_state::<AppState>(AppState::Menu);
    schedule.run(&mut world);
    assert!(*world.state::<AppState>() == AppState::Menu, "frame 4: back to Menu");
}

/// `AppState` + `NetState` registered together: a `NetState` transition leaves
/// `AppState` untouched and vice-versa (D9, per-`S` independence). State-level
/// half of plan §9 `multiple_orthogonal_states_independent`.
#[test]
fn multiple_orthogonal_states_independent() {
    let pool = serial_pool();
    let mut builder = ScheduleBuilder::new(pool);
    builder.init_state::<AppState>(); // Menu
    builder.init_state::<NetState>(); // Offline
    builder.add_system(|| {});

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert!(*world.state::<AppState>() == AppState::Menu, "AppState starts Menu");
    assert!(*world.state::<NetState>() == NetState::Offline, "NetState starts Offline");

    // Transition ONLY NetState. AppState must stay Menu.
    world.set_next_state::<NetState>(NetState::Online);
    schedule.run(&mut world);
    assert!(
        *world.state::<AppState>() == AppState::Menu,
        "a NetState transition must not change AppState"
    );
    assert!(*world.state::<NetState>() == NetState::Online, "NetState is Online");

    // Transition ONLY AppState. NetState must stay Online.
    world.set_next_state::<AppState>(AppState::InGame);
    schedule.run(&mut world);
    assert!(*world.state::<AppState>() == AppState::InGame, "AppState is InGame");
    assert!(
        *world.state::<NetState>() == NetState::Online,
        "an AppState transition must not change NetState"
    );
}

/// Registering the same `S` twice on one builder (`init_state` then
/// `insert_state`) is idempotent (M2): the FIRST registration's initial (Menu)
/// wins over the duplicate's Paused, and the schedule builds + runs cleanly with
/// exactly one entry (a double entry would drain `NextState` twice / re-synth
/// the initial). State-level half of plan §9 `init_state_twice_is_idempotent`.
#[test]
fn init_state_twice_is_idempotent() {
    let pool = serial_pool();
    let mut builder = ScheduleBuilder::new(pool);
    builder.init_state::<AppState>(); // first: initial = Menu (Default)
    builder.insert_state::<AppState>(AppState::Paused); // duplicate: ignored
    builder.add_system(|| {});

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    // The FIRST registration's initial (Menu) wins, NOT the duplicate's Paused.
    assert!(
        *world.state::<AppState>() == AppState::Menu,
        "M2: the FIRST registration's initial (Menu) wins; the duplicate insert_state(Paused) is ignored"
    );

    // A transition still drains exactly once (a double entry would leave the
    // state correct but the test below pins the single-drain via a request that
    // a double-entry would still resolve — the real M2 evidence is the initial
    // value above + the clean single build).
    world.set_next_state::<AppState>(AppState::InGame);
    schedule.run(&mut world);
    assert!(
        *world.state::<AppState>() == AppState::InGame,
        "M2: a single transition entry drains the request once to InGame"
    );
}

/// A schedule with ordinary systems but NO state registered runs normally — the
/// state-pass gate (`state_entries.is_empty()`) is inert. Plan §9
/// `no_states_zero_overhead_smoke`.
#[test]
fn no_states_zero_overhead_smoke() {
    let pool = serial_pool();
    let a = counter();
    let b = counter();
    let a_cl = Arc::clone(&a);
    let b_cl = Arc::clone(&b);

    let mut builder = ScheduleBuilder::new(pool);
    builder.add_system(move || {
        a_cl.fetch_add(1, Ordering::Relaxed);
    });
    builder.add_system(move || {
        b_cl.fetch_add(1, Ordering::Relaxed);
    });

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    for _ in 0..3 {
        schedule.run(&mut world);
    }
    assert_eq!(load(&a), 3, "no-state schedule runs system a every frame");
    assert_eq!(load(&b), 3, "no-state schedule runs system b every frame");
}

/// A state registered but never gated by any condition adds exactly one inert
/// transition entry: ordinary systems run normally and the state's resources are
/// present + readable. Covers the §13-OQ "built but never run" sub-question.
#[test]
fn registered_but_unused_state_is_inert() {
    let pool = serial_pool();
    let runs = counter();
    let runs_cl = Arc::clone(&runs);

    let mut builder = ScheduleBuilder::new(pool);
    builder.init_state::<AppState>(); // registered, but NO condition references it.
    builder.add_system(move || {
        runs_cl.fetch_add(1, Ordering::Relaxed);
    });

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    for _ in 0..3 {
        schedule.run(&mut world);
    }
    assert_eq!(load(&runs), 3, "an unused registered state does not perturb ordinary systems");
    assert!(
        *world.state::<AppState>() == AppState::Menu,
        "the registered (unused) state is present at its initial value"
    );
}

// =============================================================================
// PART 2 — Condition tests (run conditions wired through `.run_if`)
// =============================================================================
//
// These are the plan §9 tests that wire the run conditions through `.run_if`.
// They were previously BLOCKED by bug F1: `in_state` / `on_enter` / `on_exit` /
// `on_transition` returned an opaque `impl FnMut(Res<..>) -> bool`, and that
// opaque return type could not satisfy the `SystemParamFunction` double-`FnMut`
// HRTB bound that `.run_if`'s `IntoSystem<(), bool, M>` requires. The developer
// fixed it: the conditions now return `impl System<Out = bool>` (a plain System
// trait that survives the opaque-return boundary), re-bridged to `IntoSystem`
// via an identity blanket. The tests below compile + run by default.

mod condition_gated {
    use super::*;
    use boyko_ecs::ecs::core::schedule::{
        in_state, on_enter, on_exit, on_transition, run_once,
    };
    use boyko_ecs::ecs::core::state::StateTransitionSet;
    use boyko_macros::SystemSet;
    use std::sync::Mutex;

    type Log = Arc<Mutex<Vec<&'static str>>>;
    fn new_log() -> Log {
        Arc::new(Mutex::new(Vec::new()))
    }
    fn snapshot(log: &Log) -> Vec<&'static str> {
        log.lock().expect("log mutex poisoned").clone()
    }

    /// Plan §9 `initial_on_enter_fires_once_at_startup`.
    #[test]
    fn initial_on_enter_fires_once_at_startup() {
        let pool = serial_pool();
        let runs = counter();
        let runs_cl = Arc::clone(&runs);

        let mut builder = ScheduleBuilder::new(pool);
        builder.init_state::<AppState>();
        builder
            .add_system(move || {
                runs_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(on_enter(AppState::Menu));

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        schedule.run(&mut world);
        assert_eq!(load(&runs), 1, "frame 1: synth none→Menu fires on_enter(Menu) once");
        schedule.run(&mut world);
        assert_eq!(load(&runs), 1, "frame 2: no transition ⇒ on_enter(Menu) does NOT re-fire");
        schedule.run(&mut world);
        assert_eq!(load(&runs), 1, "frame 3: still no transition");
    }

    /// Plan §9 `transition_fires_enter_and_exit_on_right_frame`.
    #[test]
    fn transition_fires_enter_and_exit_on_right_frame() {
        let pool = serial_pool();
        let in_menu = counter();
        let in_game = counter();
        let exit_menu = counter();
        let enter_game = counter();
        let in_menu_cl = Arc::clone(&in_menu);
        let in_game_cl = Arc::clone(&in_game);
        let exit_menu_cl = Arc::clone(&exit_menu);
        let enter_game_cl = Arc::clone(&enter_game);

        let mut builder = ScheduleBuilder::new(pool);
        builder.init_state::<AppState>();

        let requested = Arc::new(AtomicUsize::new(0));
        let requested_cl = Arc::clone(&requested);
        builder.add_system(move |mut next: ResMut<NextState<AppState>>| {
            if requested_cl.fetch_add(1, Ordering::Relaxed) == 0 {
                next.set(AppState::InGame);
            }
        });
        builder
            .add_system(move || {
                in_menu_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(in_state(AppState::Menu));
        builder
            .add_system(move || {
                in_game_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(in_state(AppState::InGame));
        builder
            .add_system(move || {
                exit_menu_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(on_exit(AppState::Menu));
        builder
            .add_system(move || {
                enter_game_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(on_enter(AppState::InGame));

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        schedule.run(&mut world);
        assert_eq!(load(&in_menu), 1, "frame 1: in_state(Menu) runs");
        assert_eq!(load(&in_game), 0, "frame 1: in_state(InGame) does NOT run");
        assert_eq!(load(&exit_menu), 0, "frame 1: no exit yet");
        assert_eq!(load(&enter_game), 0, "frame 1: no enter(InGame) yet");

        schedule.run(&mut world);
        assert_eq!(load(&in_menu), 1, "frame 2: Menu left");
        assert_eq!(load(&in_game), 1, "frame 2: in_state(InGame) starts");
        assert_eq!(load(&exit_menu), 1, "frame 2: on_exit(Menu) fires once");
        assert_eq!(load(&enter_game), 1, "frame 2: on_enter(InGame) fires once");

        schedule.run(&mut world);
        assert_eq!(load(&in_menu), 1, "frame 3: still not in Menu");
        assert_eq!(load(&in_game), 2, "frame 3: in_state(InGame) keeps running");
        assert_eq!(load(&exit_menu), 1, "frame 3: on_exit(Menu) does not re-fire");
        assert_eq!(load(&enter_game), 1, "frame 3: on_enter(InGame) does not re-fire");
    }

    /// Plan §9 `identity_transition_is_noop` (condition half).
    #[test]
    fn identity_transition_is_noop() {
        let pool = serial_pool();
        let enter_menu = counter();
        let exit_menu = counter();
        let enter_menu_cl = Arc::clone(&enter_menu);
        let exit_menu_cl = Arc::clone(&exit_menu);

        let mut builder = ScheduleBuilder::new(pool);
        builder.init_state::<AppState>();

        let frame = Arc::new(AtomicUsize::new(0));
        let frame_cl = Arc::clone(&frame);
        builder.add_system(move |mut next: ResMut<NextState<AppState>>| {
            if frame_cl.fetch_add(1, Ordering::Relaxed) == 1 {
                next.set(AppState::Menu);
            }
        });
        builder
            .add_system(move || {
                enter_menu_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(on_enter(AppState::Menu));
        builder
            .add_system(move || {
                exit_menu_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(on_exit(AppState::Menu));

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        schedule.run(&mut world); // frame 1: synth enter(Menu)
        schedule.run(&mut world); // frame 2: identity set(Menu)
        schedule.run(&mut world); // frame 3: identity applied at the top

        assert_eq!(load(&enter_menu), 1, "identity set(Menu) must NOT re-fire on_enter(Menu)");
        assert_eq!(load(&exit_menu), 0, "identity set(Menu) must NOT fire on_exit(Menu)");
    }

    /// Plan §9 `last_write_wins_one_transition_per_frame` (condition half).
    #[test]
    fn last_write_wins_one_transition_per_frame() {
        let pool = serial_pool();
        let enter_game = counter();
        let enter_paused = counter();
        let enter_game_cl = Arc::clone(&enter_game);
        let enter_paused_cl = Arc::clone(&enter_paused);

        let mut builder = ScheduleBuilder::new(pool);
        builder.init_state::<AppState>();

        let frame = Arc::new(AtomicUsize::new(0));
        let frame_cl = Arc::clone(&frame);
        builder.add_system(move |mut next: ResMut<NextState<AppState>>| {
            if frame_cl.fetch_add(1, Ordering::Relaxed) == 0 {
                next.set(AppState::InGame);
                next.set(AppState::Paused);
            }
        });
        builder
            .add_system(move || {
                enter_game_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(on_enter(AppState::InGame));
        builder
            .add_system(move || {
                enter_paused_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(on_enter(AppState::Paused));

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        schedule.run(&mut world);
        schedule.run(&mut world);

        assert_eq!(load(&enter_game), 0, "the overwritten InGame request must NOT fire");
        assert_eq!(load(&enter_paused), 1, "only the last-written Paused fires, once");
    }

    /// Plan §9 `transition_fires_exactly_once`.
    #[test]
    fn transition_fires_exactly_once() {
        let pool = serial_pool();
        let enters = counter();
        let enters_cl = Arc::clone(&enters);

        let mut builder = ScheduleBuilder::new(pool);
        builder.init_state::<AppState>();

        let frame = Arc::new(AtomicUsize::new(0));
        let frame_cl = Arc::clone(&frame);
        builder.add_system(move |mut next: ResMut<NextState<AppState>>| {
            if frame_cl.fetch_add(1, Ordering::Relaxed) == 0 {
                next.set(AppState::InGame);
            }
        });
        builder
            .add_system(move || {
                enters_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(on_enter(AppState::InGame));

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        for _ in 0..5 {
            schedule.run(&mut world);
        }
        assert_eq!(load(&enters), 1, "on_enter(InGame) fires exactly once across 5 frames");
    }

    /// Plan §9 `in_state_gates_systems`.
    #[test]
    fn in_state_gates_systems() {
        let pool = serial_pool();
        let menu = counter();
        let game = counter();
        let paused = counter();
        let menu_cl = Arc::clone(&menu);
        let game_cl = Arc::clone(&game);
        let paused_cl = Arc::clone(&paused);

        let mut builder = ScheduleBuilder::new(pool);
        builder.init_state::<AppState>();
        builder
            .add_system(move || {
                menu_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(in_state(AppState::Menu));
        builder
            .add_system(move || {
                game_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(in_state(AppState::InGame));
        builder
            .add_system(move || {
                paused_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(in_state(AppState::Paused));

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        schedule.run(&mut world);
        assert_eq!((load(&menu), load(&game), load(&paused)), (1, 0, 0), "Menu");

        world.set_next_state::<AppState>(AppState::InGame);
        schedule.run(&mut world);
        assert_eq!((load(&menu), load(&game), load(&paused)), (1, 1, 0), "InGame");

        world.set_next_state::<AppState>(AppState::Paused);
        schedule.run(&mut world);
        assert_eq!((load(&menu), load(&game), load(&paused)), (1, 1, 1), "Paused");
    }

    /// Plan §9 `on_transition_fires_only_for_exact_pair`.
    #[test]
    fn on_transition_fires_only_for_exact_pair() {
        let pool = serial_pool();
        let exact = counter();
        let exact_cl = Arc::clone(&exact);

        let mut builder = ScheduleBuilder::new(pool);
        builder.init_state::<AppState>();
        builder
            .add_system(move || {
                exact_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(on_transition(AppState::Menu, AppState::InGame));

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        schedule.run(&mut world);
        assert_eq!(load(&exact), 0, "synth none→Menu is not Menu→InGame");

        world.set_next_state::<AppState>(AppState::InGame);
        schedule.run(&mut world);
        assert_eq!(load(&exact), 1, "Menu→InGame fires on_transition(Menu, InGame)");

        world.set_next_state::<AppState>(AppState::Paused);
        schedule.run(&mut world);
        assert_eq!(load(&exact), 1, "InGame→Paused must NOT fire");

        world.set_next_state::<AppState>(AppState::Menu);
        schedule.run(&mut world);
        world.set_next_state::<AppState>(AppState::Paused);
        schedule.run(&mut world);
        assert_eq!(load(&exact), 1, "Menu→Paused must NOT fire; only the exact pair counts");
    }

    /// Plan §9 `multiple_orthogonal_states_independent` (condition half).
    #[test]
    fn multiple_orthogonal_states_independent() {
        let pool = serial_pool();
        let app_enter_game = counter();
        let net_enter_online = counter();
        let app_cl = Arc::clone(&app_enter_game);
        let net_cl = Arc::clone(&net_enter_online);

        let mut builder = ScheduleBuilder::new(pool);
        builder.init_state::<AppState>();
        builder.init_state::<NetState>();
        builder
            .add_system(move || {
                app_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(on_enter(AppState::InGame));
        builder
            .add_system(move || {
                net_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(on_enter(NetState::Online));

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        schedule.run(&mut world);
        assert_eq!(load(&app_enter_game), 0, "AppState in Menu");
        assert_eq!(load(&net_enter_online), 0, "NetState in Offline");

        world.set_next_state::<NetState>(NetState::Online);
        schedule.run(&mut world);
        assert_eq!(load(&app_enter_game), 0, "NetState transition must not fire AppState enter");
        assert_eq!(load(&net_enter_online), 1, "NetState→Online fires once");

        world.set_next_state::<AppState>(AppState::InGame);
        schedule.run(&mut world);
        assert_eq!(load(&app_enter_game), 1, "AppState→InGame fires once");
        assert_eq!(load(&net_enter_online), 1, "AppState transition must not re-fire NetState enter");
    }

    /// Plan §9 `interaction_with_phase15_ordering`.
    #[test]
    fn interaction_with_phase15_ordering() {
        let pool = serial_pool();
        let log = new_log();

        let mut builder = ScheduleBuilder::new(pool);
        builder.init_state::<AppState>();

        let frame = Arc::new(AtomicUsize::new(0));
        let frame_cl = Arc::clone(&frame);
        builder.add_system(move |mut next: ResMut<NextState<AppState>>| {
            if frame_cl.fetch_add(1, Ordering::Relaxed) == 0 {
                next.set(AppState::InGame);
            }
        });

        let log_first = Arc::clone(&log);
        let first = builder
            .add_system(move || {
                log_first.lock().expect("poisoned").push("first");
            })
            .run_if(on_enter(AppState::InGame))
            .key();
        let log_second = Arc::clone(&log);
        builder
            .add_system(move || {
                log_second.lock().expect("poisoned").push("second");
            })
            .run_if(on_enter(AppState::InGame))
            .after(first);

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        schedule.run(&mut world);
        assert!(snapshot(&log).is_empty(), "frame 1: still Menu");

        schedule.run(&mut world);
        assert_eq!(
            snapshot(&log),
            vec!["first", "second"],
            "on_enter(InGame) systems run in declared .after order on the transition frame"
        );
    }

    /// Plan §9 `interaction_with_phase16_conditions` — `in_state(InGame)` AND
    /// `run_once`, eager-AND fold (no short-circuit).
    #[test]
    fn interaction_with_phase16_conditions() {
        let pool = serial_pool();
        let runs = counter();
        let runs_cl = Arc::clone(&runs);

        let mut builder = ScheduleBuilder::new(pool);
        builder.init_state::<AppState>();

        let frame = Arc::new(AtomicUsize::new(0));
        let frame_cl = Arc::clone(&frame);
        builder.add_system(move |mut next: ResMut<NextState<AppState>>| {
            if frame_cl.fetch_add(1, Ordering::Relaxed) == 0 {
                next.set(AppState::InGame);
            }
        });
        builder
            .add_system(move || {
                runs_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(in_state(AppState::InGame))
            .run_if(run_once);

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        // Frame 1: Menu ⇒ in_state false; eager fold still flips run_once.
        schedule.run(&mut world);
        assert_eq!(load(&runs), 0, "frame 1: in Menu ⇒ skipped");
        // Frame 2: InGame, but run_once exhausted on frame 1 (eager AND) ⇒ skipped.
        schedule.run(&mut world);
        assert_eq!(load(&runs), 0, "frame 2: run_once exhausted on frame 1 ⇒ skipped");
        schedule.run(&mut world);
        assert_eq!(load(&runs), 0, "frame 3: stays skipped");
    }

    /// Plan §9 `set_next_state_from_direct_api` (condition half).
    #[test]
    fn set_next_state_from_direct_api() {
        let pool = serial_pool();
        let enter_game = counter();
        let enter_game_cl = Arc::clone(&enter_game);

        let mut builder = ScheduleBuilder::new(pool);
        builder.init_state::<AppState>();
        builder
            .add_system(move || {
                enter_game_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(on_enter(AppState::InGame));

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        schedule.run(&mut world);
        assert_eq!(load(&enter_game), 0, "frame 1: still Menu");

        world.set_next_state::<AppState>(AppState::InGame);
        schedule.run(&mut world);
        assert_eq!(load(&enter_game), 1, "direct set_next_state drives the next run's transition");
    }

    /// Plan §9 `init_state_twice_is_idempotent` (condition half).
    #[test]
    fn init_state_twice_is_idempotent() {
        let pool = serial_pool();
        let enter_menu = counter();
        let enter_menu_cl = Arc::clone(&enter_menu);

        let mut builder = ScheduleBuilder::new(pool);
        builder.init_state::<AppState>();
        builder.insert_state::<AppState>(AppState::Paused); // duplicate, ignored

        builder
            .add_system(move || {
                enter_menu_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(on_enter(AppState::Menu));

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        schedule.run(&mut world);
        assert_eq!(load(&enter_menu), 1, "M2: synth on_enter(Menu) fires ONCE, not twice");
        schedule.run(&mut world);
        assert_eq!(load(&enter_menu), 1, "M2: frame 2 no re-fire (single entry)");
    }

    /// Opt-in `StateTransitionSet` composes with `configure_set` ordering (D10).
    #[test]
    fn state_transition_set_orders_before_gameplay() {
        #[derive(SystemSet)]
        struct GameplaySet;

        let pool = serial_pool();
        let log = new_log();

        let mut builder = ScheduleBuilder::new(pool);
        builder.init_state::<AppState>();

        let frame = Arc::new(AtomicUsize::new(0));
        let frame_cl = Arc::clone(&frame);
        builder.add_system(move |mut next: ResMut<NextState<AppState>>| {
            if frame_cl.fetch_add(1, Ordering::Relaxed) == 0 {
                next.set(AppState::InGame);
            }
        });

        let log_setup = Arc::clone(&log);
        builder
            .add_system(move || {
                log_setup.lock().expect("poisoned").push("setup");
            })
            .run_if(on_enter(AppState::InGame))
            .in_set(StateTransitionSet);

        let log_play = Arc::clone(&log);
        builder
            .add_system(move || {
                log_play.lock().expect("poisoned").push("play");
            })
            .run_if(in_state(AppState::InGame))
            .in_set(GameplaySet);

        builder.configure_set(StateTransitionSet).before(GameplaySet);

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        schedule.run(&mut world);
        assert!(snapshot(&log).is_empty(), "frame 1: in Menu");

        schedule.run(&mut world);
        assert_eq!(
            snapshot(&log),
            vec!["setup", "play"],
            "StateTransitionSet ordered before GameplaySet on the transition frame"
        );
    }
}
