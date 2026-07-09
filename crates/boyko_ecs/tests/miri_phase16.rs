//! Phase 16 — Miri validation for run conditions.
//!
//! Run under:
//!
//! ```powershell
//! $env:MIRIFLAGS = "-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-ecs --test miri_phase16
//! ```
//!
//! `-Zmiri-tree-borrows` is the workspace default (`.cargo/config.toml`);
//! `-Zmiri-ignore-leaks` is appended because the `#[cfg(not(miri))]`
//! `Schedule::run` smoke tests construct an `Arc<ThreadPool>` whose OS worker
//! threads Miri reports as "leaked" at process exit (a harness shutdown check,
//! NOT a UB).
//!
//! # The unsafe surface Phase 16 adds — and how Miri reaches it WITHOUT spawn
//!
//! The single new `unsafe` is the condition reborrow:
//! `EcsMaster::run_condition` (`ecs_master.rs`) does
//! `UnsafeEcsCell::new_mutable(self)` → `condition.run_unsafe(cell)` →
//! `SystemParam::get_param` (`Local::get_param` for `run_once`, `Res::get_param`
//! for a resource gate). This is BYTE-IDENTICAL to the reborrow performed by
//! the already-public [`EcsMaster::run_closure_once`] /
//! [`EcsMaster::run_cached_system`] (both mint `UnsafeEcsCell::new_mutable` and
//! call `run_unsafe`). `run_condition` is `pub(crate)`, so an external test
//! crate cannot call it directly — but running the SAME condition body through
//! `run_closure_once` / `run_cached_system` exercises the identical
//! `new_mutable → run_unsafe → get_param` retag chain on the dispatcher thread,
//! single-threaded, with NO `Scope::spawn`. That is what the Miri-clean tests
//! below do.
//!
//! # Why `Schedule::run` is NOT used under Miri (Phase-9 deferral, NOT Phase 16)
//!
//! A schedule with a conditioned system that runs or skips MUST dispatch at
//! least the gated body, and a non-exclusive (empty-`Access`) body is dispatched
//! via `boyko_threadpool::Scope::spawn` — even on a `num_threads(1)` pool (only
//! universal-`Access` exclusive systems run inline on the dispatcher). The
//! `Scope::spawn` worker raw-pointer handshake hits a KNOWN Tree Borrows
//! protected-tag conflict under Miri (documented in `miri_phase9.rs`: the
//! worker's `ArrayQueue::push` / `pending_apply.fetch_add` foreign-writes a tag
//! the dispatcher still holds; sound by design, deferred to Phase 9.1). This is
//! a Wave-1 thread-pool layer issue, NOT a Phase-16 defect. `miri_phase9.rs`
//! itself gates its only `Schedule::run` test on `#[cfg(not(miri))]` and runs
//! schedules with ZERO systems under Miri for the same reason.
//!
//! Accordingly: the FULL `Schedule::run` condition path (run/skip/cascade) is
//! validated under the regular `cargo test` suite
//! (`tests/phase16_run_conditions.rs`) and the `#[cfg(not(miri))]` smoke tests
//! below; under Miri we isolate the Phase-16 condition reborrow via
//! `run_cached_system` / `run_closure_once`.
//!
//! Like the other `miri_phase*.rs` files this is NOT gated on `#[cfg(miri)]`, so
//! it also runs as a fast smoke test under the regular `cargo test`.
//!
//! [`EcsMaster::run_closure_once`]: boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster::run_closure_once
//! [`EcsMaster::run_cached_system`]: boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster::run_cached_system

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::run_once;
use boyko_ecs::ecs::core::system::{FunctionSystem, IntoSystem, Local, Res};
use boyko_macros::Resource;

#[cfg(not(miri))]
use std::sync::Arc;
#[cfg(not(miri))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(not(miri))]
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
#[cfg(not(miri))]
use boyko_macros::SystemSet;
#[cfg(not(miri))]
use boyko_threadpool::ThreadPoolBuilder;

#[derive(Resource)]
struct MiriGate(bool);

// =============================================================================
// Miri-CLEAN tests — exercise the Phase-16 condition reborrow WITHOUT spawn.
// These run under both `cargo +nightly miri test` AND regular `cargo test`.
// =============================================================================

/// The `run_once` built-in run as a CACHED system across three "frames" via
/// `run_cached_system` — the same `new_mutable → run_unsafe → Local::get_param`
/// reborrow `run_condition` performs, isolated from `Scope::spawn`. Validates
/// the `Local<bool>` state borrow is UB-clean and persists across runs (frame 1
/// true, frames 2/3 false). This is the Miri twin of the integration
/// `run_once_runs_body_exactly_once_over_three_frames`.
#[test]
fn miri_run_once_condition_reborrow_no_ub() {
    let mut world = EcsMaster::new();
    // Build the condition ONCE and hoist it so the `Local<bool>` state persists
    // across runs (a fresh build each call would reset the Local — see the
    // Phase 13 cached-system note).
    let mut cond: FunctionSystem<_, _> =
        IntoSystem::into_system(|l: Local<bool>| run_once(l));

    let r1 = world.run_cached_system(&mut cond);
    let r2 = world.run_cached_system(&mut cond);
    let r3 = world.run_cached_system(&mut cond);

    assert!(r1, "frame 1: run_once true");
    assert!(!r2, "frame 2: run_once false (Local advanced)");
    assert!(!r3, "frame 3: run_once false");
}

/// A `Res<MiriGate>`-reading condition run via `run_closure_once` — exercises
/// the condition reborrow feeding `Res::get_param` (the resource-pointer fetch
/// through the cell). Validates the reborrow + resource borrow under Miri.
#[test]
fn miri_res_condition_reborrow_no_ub() {
    let mut world = EcsMaster::new();
    world.insert_resource(MiriGate(true));

    let verdict: bool = world.run_closure_once(|gate: Res<MiriGate>| gate.0);
    assert!(verdict, "gate true ⇒ condition returns true");

    world.insert_resource(MiriGate(false));
    let verdict2: bool = world.run_closure_once(|gate: Res<MiriGate>| gate.0);
    assert!(!verdict2, "gate false ⇒ condition returns false");
}

/// A constant `|| bool` condition run via `run_closure_once` — the minimal
/// reborrow (empty param tuple), confirming the `new_mutable → run_unsafe`
/// chain itself is UB-clean for a zero-param `Out = bool` system.
#[test]
fn miri_constant_condition_reborrow_no_ub() {
    let mut world = EcsMaster::new();
    let yes: bool = world.run_closure_once(|| true);
    let no: bool = world.run_closure_once(|| false);
    assert!(yes);
    assert!(!no);
}

// =============================================================================
// Full-schedule smoke tests — `#[cfg(not(miri))]` (Phase-9 Scope::spawn
// deferral). These run ONLY under regular `cargo test`, validating the
// end-to-end run/skip path that Miri cannot reach without the pre-existing
// thread-pool issue.
// =============================================================================

#[cfg(not(miri))]
#[derive(SystemSet)]
struct MiriGatedSet;

/// Single-worker `Schedule::run` with a `.run_if(run_once)` system across 3
/// frames — the full executor path (dispatch frame 1, mark_skipped frames 2/3).
/// Skipped under Miri (`Scope::spawn`); a regular-`cargo test` smoke check.
#[cfg(not(miri))]
#[test]
fn miri_schedule_run_once_smoke() {
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let runs = Arc::new(AtomicUsize::new(0));
    let runs_cl = Arc::clone(&runs);

    let mut builder = ScheduleBuilder::new(pool);
    builder
        .add_system(move || {
            runs_cl.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(run_once);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);
    schedule.run(&mut world);
    schedule.run(&mut world);

    assert_eq!(runs.load(Ordering::Relaxed), 1, "run_once body runs once over 3 frames");
}

/// Single-worker `Schedule::run` with a set-level condition gating 3 members —
/// the `set_gate` memo path through the full executor. Skipped under Miri.
#[cfg(not(miri))]
#[test]
fn miri_schedule_set_condition_smoke() {
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let runs = Arc::new(AtomicUsize::new(0));

    let mut builder = ScheduleBuilder::new(pool);
    for _ in 0..3 {
        let cl = Arc::clone(&runs);
        builder
            .add_system(move || {
                cl.fetch_add(1, Ordering::Relaxed);
            })
            .in_set(MiriGatedSet);
    }
    builder.configure_set(MiriGatedSet).run_if(|| true);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);
    schedule.run(&mut world);

    assert_eq!(runs.load(Ordering::Relaxed), 6, "3 members × 2 frames under a true set gate");
}

/// Multi-worker `Schedule::run` with a parallel bank + a conditioned tail
/// system — validates the condition reborrow coexists with REAL parallel
/// dispatch. Skipped under Miri (Phase-9 `Scope::spawn`); regular `cargo test`
/// smoke check.
#[cfg(not(miri))]
#[test]
fn miri_schedule_multi_worker_condition_smoke() {
    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let cond_runs = Arc::new(AtomicUsize::new(0));
    let worker_runs = Arc::new(AtomicUsize::new(0));

    let mut builder = ScheduleBuilder::new(pool);
    let mut keys = Vec::new();
    for _ in 0..4 {
        let cl = Arc::clone(&worker_runs);
        let key = builder
            .add_system(move || {
                cl.fetch_add(1, Ordering::Relaxed);
            })
            .key();
        keys.push(key);
    }
    let cond_cl = Arc::clone(&cond_runs);
    let mut gated = builder.add_system(move || {
        cond_cl.fetch_add(1, Ordering::Relaxed);
    });
    for &k in &keys {
        gated = gated.after(k);
    }
    gated.run_if(|| true);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    assert_eq!(worker_runs.load(Ordering::Relaxed), 4);
    assert_eq!(cond_runs.load(Ordering::Relaxed), 1);
}
