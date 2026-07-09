//! Phase 17 — Miri validation for application/game states.
//!
//! Run under:
//!
//! ```powershell
//! $env:MIRIFLAGS = "-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-ecs --test miri_phase17
//! ```
//!
//! `-Zmiri-tree-borrows` is the workspace default (`.cargo/config.toml`);
//! `-Zmiri-ignore-leaks` is appended because the `#[cfg(not(miri))]`
//! `Schedule::run` smoke tests build an `Arc<ThreadPool>` whose OS worker
//! threads Miri reports as "leaked" at process exit (a harness shutdown check,
//! NOT a UB).
//!
//! # What Phase 17 adds — and how Miri reaches it WITHOUT spawn
//!
//! Phase 17 introduces **zero new `unsafe`** (plan §8). Its observable memory
//! surface from a public, single-threaded entry point is:
//!
//! * `EcsMaster::insert_state` / `init_state` — inserts the three backing
//!   resources (`State<S>`, `NextState<S>`, `StateTransitionRecord<S>`) into the
//!   resource slab. Exercises the slab's `Box::into_raw` store + `drop_fn`
//!   registration (pre-existing Phase-8a unsafe, unchanged).
//! * `EcsMaster::set_next_state` — `resource_mut::<NextState<S>>().set(..)`: the
//!   slab pointer fetch + `&mut` reborrow + a plain enum write.
//! * `EcsMaster::state` / `resource::<..>` — the slab pointer fetch + `&` reborrow.
//! * `<State<S> as Resource>::resource_id()` — the D3 `TypeId`-keyed
//!   `OnceLock<Mutex<HashMap<..>>>` registry (identical to the Miri-clean
//!   `query_type_registry`).
//!
//! None of these touch the thread pool. The transition ALGORITHM
//! (`apply_state_transition`) is `pub(crate)`, so an external test crate cannot
//! call it — its Miri-clean direct-drive validation lives in the in-crate unit
//! tests (`src/ecs/core/state/transition_record.rs`, run via
//! `cargo +nightly miri test --lib`). Here we validate the PUBLIC resource
//! surface the conditions + the pass read/write, single-threaded, no spawn.
//!
//! # Why `Schedule::run` is NOT used under Miri (Phase-9 deferral, NOT Phase 17)
//!
//! A schedule that drives a real transition must dispatch a `ResMut<NextState>`
//! system, which is non-exclusive (resource write) and so runs via
//! `boyko_threadpool::Scope::spawn` — even on `num_threads(1)`. That worker
//! raw-pointer handshake hits the known Tree-Borrows protected-tag conflict
//! documented in `miri_phase9.rs` (sound by design, deferred to Phase 9.1).
//! This is a Wave-1 thread-pool layer issue, NOT a Phase-17 defect. The full
//! `Schedule::run` transition path is validated under the regular `cargo test`
//! suite (`tests/phase17_states.rs`) and the `#[cfg(not(miri))]` smoke tests
//! below.
//!
//! Like the other `miri_phase*.rs` files this is NOT gated on `#[cfg(miri)]`, so
//! it also runs as a fast smoke test under the regular `cargo test`.

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::resources::resource::Resource;
use boyko_ecs::ecs::core::schedule::{in_state, on_enter};
use boyko_ecs::ecs::core::state::{NextState, State, States};

#[cfg(not(miri))]
use std::sync::Arc;
#[cfg(not(miri))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(not(miri))]
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
#[cfg(not(miri))]
use boyko_ecs::ecs::core::system::ResMut;
#[cfg(not(miri))]
use boyko_threadpool::ThreadPoolBuilder;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
enum AppState {
    #[default]
    Menu,
    InGame,
    Paused,
}
impl States for AppState {}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
enum NetState {
    #[default]
    Offline,
    Online,
}
impl States for NetState {}

// =============================================================================
// Miri-CLEAN tests — exercise the PUBLIC state resource surface WITHOUT spawn.
// These run under both `cargo +nightly miri test` AND regular `cargo test`.
// =============================================================================

/// `init_state` inserts the three backing resources; reading them back through
/// `state` / `resource::<NextState>` exercises the slab store + reborrow. The
/// initial value is `Default` (Menu) and `NextState` is `Unchanged`.
#[test]
fn miri_init_state_insert_and_read_no_ub() {
    let mut world = EcsMaster::new();
    world.init_state::<AppState>();

    assert!(*world.state::<AppState>() == AppState::Menu, "initial = Default (Menu)");
    assert!(
        matches!(world.resource::<NextState<AppState>>(), NextState::Unchanged),
        "NextState starts Unchanged"
    );
}

/// `insert_state(v)` stores an explicit initial; `set_next_state` then performs
/// the `resource_mut::<NextState>` reborrow + enum write, observed via
/// `resource::<NextState>`. Validates the write reborrow is UB-clean.
#[test]
fn miri_set_next_state_write_reborrow_no_ub() {
    let mut world = EcsMaster::new();
    world.insert_state::<AppState>(AppState::Paused);
    assert!(*world.state::<AppState>() == AppState::Paused, "explicit initial = Paused");

    world.set_next_state::<AppState>(AppState::InGame);
    match world.resource::<NextState<AppState>>() {
        NextState::Pending(v) => assert!(*v == AppState::InGame, "pending = InGame"),
        NextState::Unchanged => panic!("expected a pending request after set_next_state"),
    }
}

/// Two orthogonal states' resources coexist in the slab without aliasing: a
/// `NetState` write does not perturb the `AppState` slot, and reading both back
/// is UB-clean. The D9 independence property at the resource level.
#[test]
fn miri_orthogonal_state_resources_no_alias_no_ub() {
    let mut world = EcsMaster::new();
    world.init_state::<AppState>(); // Menu
    world.init_state::<NetState>(); // Offline

    world.set_next_state::<NetState>(NetState::Online);

    // AppState's slot is untouched; NetState's request is recorded.
    assert!(*world.state::<AppState>() == AppState::Menu, "AppState slot untouched");
    assert!(
        matches!(world.resource::<NextState<NetState>>(), NextState::Pending(NetState::Online)),
        "NetState request recorded in its own slot"
    );
    assert!(
        matches!(world.resource::<NextState<AppState>>(), NextState::Unchanged),
        "AppState's NextState slot is not aliased by the NetState write"
    );
}

/// The D3 `TypeId`-keyed registry mints DISTINCT, STABLE ids per generic state
/// resource — the Miri twin of the in-crate `state_resource_ids_distinct_per_type`
/// unit test, validating the `OnceLock<Mutex<HashMap>>` access is UB-clean
/// (no torn reads, no double-mint) under Miri's stricter model.
#[test]
fn miri_resource_id_registry_no_ub() {
    let app = State::<AppState>::resource_id();
    let net = State::<NetState>::resource_id();
    let app_next = NextState::<AppState>::resource_id();

    assert!(app != net, "distinct state types ⇒ distinct ids");
    assert!(app != app_next, "State<A> and NextState<A> are distinct slots");
    // Stable across repeated calls (cached, not re-minted).
    assert!(State::<AppState>::resource_id() == app, "id stable across calls");
}

/// The F1-fix surface: a run condition built from `in_state` is an opaque
/// `impl System<Out = bool>` (a `FunctionSystem` wrapping
/// `move |current: Res<State<S>>| current.get() == &target`). Driving it once
/// via the public `EcsMaster::run_system_once` (which runs the SAME
/// `initialize` → `UnsafeEcsCell::new_mutable` → `run_unsafe` sequence the
/// scheduler's `pub(crate) run_condition` uses, minus the deliberately-omitted
/// `apply`) exercises the condition body's `Res<State<S>>` SystemParam read
/// path under Tree Borrows — WITHOUT the thread pool / `Scope::spawn` that the
/// `#[cfg(not(miri))]` schedule tests need. No new `unsafe` in the test:
/// `run_system_once` is a safe facade that takes `&mut self` (enforcing
/// invariant S1 trivially). This is the read-path twin of
/// `run_condition_reads_resource_value` in `ecs_master.rs`, specialised to the
/// Phase 17 state conditions and run under Miri.
#[test]
fn miri_in_state_condition_read_path_no_ub() {
    let mut world = EcsMaster::new();
    world.init_state::<AppState>(); // State<AppState> == Menu (Default)

    // `in_state(Menu)` reads `Res<State<AppState>>` and compares — must be true
    // at the initial value. The reborrow of the resource slab slot through the
    // `Res` SystemParam is the TB-checked surface here.
    let mut cond_menu = in_state(AppState::Menu);
    assert!(
        world.run_system_once(&mut cond_menu),
        "in_state(Menu) reads State<AppState> == Menu ⇒ true"
    );

    // A non-matching target must read the same slot UB-clean and return false.
    let mut cond_game = in_state(AppState::InGame);
    assert!(
        !world.run_system_once(&mut cond_game),
        "in_state(InGame) reads State<AppState> == Menu ⇒ false"
    );

    // Re-running the SAME initialized condition reuses its cached per-system
    // state (the FS1-idempotent `initialize` no-op + a second `run_unsafe`),
    // exercising the read reborrow twice — the cached-state reuse path.
    assert!(
        world.run_system_once(&mut cond_menu),
        "re-running the cached in_state(Menu) condition still reads true"
    );
}

/// Companion to the above for the `on_enter` family: `on_enter` reads
/// `Res<StateTransitionRecord<S>>` (a DIFFERENT backing resource than
/// `in_state`'s `State<S>`). Before any transition pass has run the record's
/// `current()` is `None`, so `on_enter(Menu)` is false — driving it once
/// exercises that record's slab reborrow read path under Tree Borrows. No new
/// `unsafe`; same safe `run_system_once` facade.
#[test]
fn miri_on_enter_condition_record_read_no_ub() {
    let mut world = EcsMaster::new();
    world.init_state::<AppState>(); // inserts StateTransitionRecord<AppState> (current = None)

    let mut cond = on_enter(AppState::Menu);
    assert!(
        !world.run_system_once(&mut cond),
        "no transition pass has run ⇒ record.current() is None ⇒ on_enter(Menu) is false"
    );
}

// =============================================================================
// Full-schedule smoke tests — `#[cfg(not(miri))]` (Phase-9 Scope::spawn
// deferral). These run ONLY under regular `cargo test`, validating the
// end-to-end transition-pass path Miri cannot reach without the pre-existing
// thread-pool issue.
// =============================================================================

/// Single-worker `Schedule::run`: a `ResMut<NextState>` system requests
/// Menu→InGame on frame 1; the pass at the top of frame 2 swaps the state.
/// Skipped under Miri (`Scope::spawn`); a regular-`cargo test` smoke check.
#[cfg(not(miri))]
#[test]
fn miri_schedule_transition_pass_smoke() {
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut builder = ScheduleBuilder::new(pool);
    builder.init_state::<AppState>();

    let requested = Arc::new(AtomicUsize::new(0));
    let requested_cl = Arc::clone(&requested);
    builder.add_system(move |mut next: ResMut<NextState<AppState>>| {
        if requested_cl.fetch_add(1, Ordering::Relaxed) == 0 {
            next.set(AppState::InGame);
        }
    });

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert!(*world.state::<AppState>() == AppState::Menu, "frame 1: still Menu");
    schedule.run(&mut world);
    assert!(*world.state::<AppState>() == AppState::InGame, "frame 2: pass applied ⇒ InGame");
}
