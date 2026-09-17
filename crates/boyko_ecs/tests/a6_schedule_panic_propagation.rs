//! Defect A6, scheduler half — a panic inside a system must resume on the thread that called
//! `Schedule::run`, and must never leave that thread blocked.
//!
//! # The observed defect
//!
//! Twice: a system parameter that failed to resolve panicked on a worker under the windowed runner
//! and the window hung "not responding" for nine minutes; a `debug_assert!` in `physics_apply`
//! fired on `boyko-worker-0` and the test process never exited. The windowed runner drives frames
//! by calling `App::update_with_delta` on its own thread, which calls `Schedule::run` — so a
//! blocked `Schedule::run` is a blocked window.
//!
//! # The shapes covered
//!
//! | test | dispatch path the panic takes |
//! |---|---|
//! | [`parallel_system_panic_resumes_on_schedule_caller`] | a concurrent system on a 4-worker pool |
//! | [`single_worker_system_panic_resumes_on_schedule_caller`] | the same, with ONE worker |
//! | [`exclusive_system_panic_resumes_on_schedule_caller`] | an exclusive system, run inline on the dispatcher |
//! | [`par_iter_panic_in_system_resumes_on_schedule_caller`] | a `par_iter` row body inside a concurrent system |
//! | [`par_for_each_chunk_panic_in_system_resumes_on_schedule_caller`] | a `par_for_each_chunk` body inside a concurrent system |
//! | [`par_iter_panic_in_exclusive_system_resumes_on_schedule_caller`] | a `par_iter` row body inside an exclusive system |
//! | [`app_update_resumes_system_panic_on_caller`] | `App::update_with_delta`, the runner's entry |
//! | [`schedule_runs_again_after_an_exclusive_panic`] / [`schedule_runs_again_after_a_parallel_panic`] | the schedule and its pool stay usable |
//!
//! # How a hang becomes a red
//!
//! Every scenario runs on a spawned thread through [`watch`]; the test thread waits on a channel
//! with a [`DEADLINE`] and fails with a `WATCHDOG` message when it expires. The blocked thread is
//! leaked on purpose and dies with the process.
//!
//! # Controls
//!
//! [`parallel_fixture_returns_when_nothing_panics`] and
//! [`par_iter_fixture_returns_when_nothing_panics`] run the SAME fixtures with the panic switched
//! off, under the same watchdog. A red above is therefore about the panic, not about a fixture that
//! cannot finish.
//!
//! Component id 499 is reserved for this test binary.
//!
//! # Under Miri
//!
//! Only [`a_cancelled_parallel_run_on_a_small_world_is_reusable`] runs under Miri; the others are
//! `miri-slow` (a wall-clock watchdog and a thread leaked by design). Its recipe:
//!
//! ```bash
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance \
//!   -Zmiri-ignore-leaks -Zmiri-many-seeds=0..8" \
//!   cargo +nightly-x86_64-pc-windows-msvc miri test -p boyko-ecs --test a6_schedule_panic_propagation
//! ```
//!
//! `-Zmiri-ignore-leaks` is required for the reason `miri_schedule_parallel.rs` gives: the pool's
//! worker threads leave `crossbeam_epoch` thread-exit allocations that crossbeam never reclaims at
//! process exit. Measured 2026-09-17 (msvc nightly, miri 2026-09-09): without the flag the run
//! reports 7 such leaks and no undefined behaviour; with it, seeds 0..8 pass.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{BatchingStrategy, Query};
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

/// How long a scenario may take before it is declared BLOCKED. Every fixture here finishes in well
/// under a second when it does finish (the controls prove it); the margin is for a loaded machine.
const DEADLINE: Duration = Duration::from_secs(10);

const SLOT_A6_ROW: ComponentId = ComponentId(499);

#[repr(C)]
#[derive(Clone, Copy)]
struct A6Row(u32);

impl Component for A6Row {
    fn component_id() -> ComponentId {
        SLOT_A6_ROW
    }
}

/// Rows in the one archetype: four times `MIN_ARCHETYPE_FOR_PARALLEL`, so `par_iter` and
/// `par_for_each_chunk` genuinely fan out onto the pool instead of taking the inline
/// small-archetype path.
const N_ROWS: u32 = 4096;

/// The row whose visit panics in the parallel-iteration scenarios. Mid-range, so the panicking
/// chunk is neither the first nor the last one dispatched.
const PANIC_ROW: u32 = 2_000;

const MSG_PARALLEL: &str = "A6 parallel system panic";
const MSG_SINGLE: &str = "A6 single-worker system panic";
const MSG_EXCLUSIVE: &str = "A6 exclusive system panic";
const MSG_PAR_ITER: &str = "A6 par_iter row panic";
const MSG_PAR_CHUNK: &str = "A6 par_for_each_chunk panic";
const MSG_EXCLUSIVE_PAR_ITER: &str = "A6 par_iter row panic inside an exclusive system";
const MSG_APP: &str = "A6 App::update system panic";
const MSG_ONCE: &str = "A6 first-run-only panic";

// ── Harness ─────────────────────────────────────────────────────────────────

/// Runs `f` on its own thread and returns what it did within [`DEADLINE`]: `Ok(value)` if it
/// returned, `Err(payload)` if it panicked. Panics with a `WATCHDOG` message if it did neither.
///
/// `catch_unwind` runs on the SAME thread as `f`, so an `Err` is a panic that surfaced on the thread
/// that called `Schedule::run`.
fn watch<T, F>(label: &str, f: F) -> Result<T, Box<dyn Any + Send>>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name(format!("a6-watched-{label}"))
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(f));
            // The receiver is gone only when the watchdog already fired; nothing to report to.
            let _ = tx.send(outcome);
        })
        .expect("test setup: spawn the watched thread");
    match rx.recv_timeout(DEADLINE) {
        Ok(outcome) => outcome,
        Err(RecvTimeoutError::Timeout) => panic!(
            "WATCHDOG: `{label}` neither returned nor panicked within {DEADLINE:?} -- the thread \
             that called Schedule::run is BLOCKED (defect A6)"
        ),
        Err(RecvTimeoutError::Disconnected) => panic!(
            "`{label}`: the watched thread ended without reporting an outcome -- an unwind \
             escaped `catch_unwind`, which is not a shape this harness can classify"
        ),
    }
}

/// The text of a string payload, or `None` for any other payload type.
fn payload_text(payload: &(dyn Any + Send)) -> Option<&str> {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        Some(s)
    } else {
        payload.downcast_ref::<String>().map(String::as_str)
    }
}

/// The verdict every propagation test shares: the run must have PANICKED, and the payload must
/// carry the system's own message.
fn expect_message(label: &str, outcome: Result<(), Box<dyn Any + Send>>, message: &str) {
    match outcome {
        Ok(()) => panic!(
            "`{label}`: Schedule::run RETURNED normally -- the system's panic was swallowed \
             instead of being resumed on the caller"
        ),
        Err(payload) => match payload_text(&*payload) {
            Some(text) => assert!(
                text.contains(message),
                "`{label}`: the caller received a panic with the wrong message: {text:?} \
                 (expected it to contain {message:?})"
            ),
            None => panic!("`{label}`: the caller received a panic whose payload is not a string"),
        },
    }
}

// ── Fixtures ────────────────────────────────────────────────────────────────

fn pool(workers: usize) -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(workers).build()
}

fn world_with_rows() -> EcsMaster {
    register_layout::<A6Row>(SLOT_A6_ROW.0);
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[SLOT_A6_ROW]);
    for i in 0..N_ROWS {
        world
            .spawn_one(arch, A6Row(i))
            .expect("test setup: seeding the A6 archetype must succeed");
    }
    world
}

/// Three conflict-free systems (shared reads of one component), so the scheduler co-dispatches
/// them onto the pool. System 1 panics iff `panic` is set. Every body entry is counted in `runs`.
fn parallel_schedule(
    pool: &Arc<ThreadPool>,
    world: &mut EcsMaster,
    panic: bool,
    runs: &Arc<AtomicUsize>,
) -> Schedule {
    let mut builder = ScheduleBuilder::new(Arc::clone(pool));
    for idx in 0..3usize {
        let runs = Arc::clone(runs);
        builder.add_system(move |q: Query<&A6Row>| {
            runs.fetch_add(1, Ordering::AcqRel);
            let rows = q.iter().count();
            assert_eq!(rows, N_ROWS as usize, "fixture: every row is visible to the system");
            if panic && idx == 1 {
                panic!("{MSG_PARALLEL}");
            }
        });
    }
    builder.build(world)
}

/// One concurrent system whose `par_iter` visits every row and panics on [`PANIC_ROW`] iff `panic`
/// is set. Every visited row is counted in `seen`.
fn par_iter_schedule(
    pool: &Arc<ThreadPool>,
    world: &mut EcsMaster,
    panic: bool,
    seen: &Arc<AtomicUsize>,
) -> Schedule {
    let mut builder = ScheduleBuilder::new(Arc::clone(pool));
    let seen = Arc::clone(seen);
    builder.add_system(move |q: Query<&A6Row>| {
        let seen = &seen;
        q.par_iter().for_each(|row: &A6Row| {
            if panic && row.0 == PANIC_ROW {
                panic!("{MSG_PAR_ITER}");
            }
            seen.fetch_add(1, Ordering::Relaxed);
        });
    });
    builder.build(world)
}

// ── Controls ────────────────────────────────────────────────────────────────

/// CONTROL. The parallel fixture with the panic switched OFF returns, with all three systems run.
///
/// Red if the fixture itself cannot finish, which would make the parallel reds meaningless.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn parallel_fixture_returns_when_nothing_panics() {
    let outcome = watch("control-parallel", || {
        let pool = pool(4);
        let mut world = world_with_rows();
        let runs = Arc::new(AtomicUsize::new(0));
        let mut schedule = parallel_schedule(&pool, &mut world, false, &runs);
        schedule.run(&mut world);
        runs.load(Ordering::Acquire)
    });
    match outcome {
        Ok(runs) => assert_eq!(runs, 3, "the control run must enter each of the three systems once"),
        Err(p) => panic!("the control run panicked: {:?}", payload_text(&*p)),
    }
}

/// CONTROL. The `par_iter` fixture with the panic switched OFF returns, with every row visited.
///
/// Red if the fixture itself cannot finish or does not fan out over all rows.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn par_iter_fixture_returns_when_nothing_panics() {
    let outcome = watch("control-par-iter", || {
        let pool = pool(4);
        let mut world = world_with_rows();
        let seen = Arc::new(AtomicUsize::new(0));
        let mut schedule = par_iter_schedule(&pool, &mut world, false, &seen);
        schedule.run(&mut world);
        seen.load(Ordering::Acquire)
    });
    match outcome {
        Ok(seen) => assert_eq!(seen, N_ROWS as usize, "the control run must visit every row once"),
        Err(p) => panic!("the control run panicked: {:?}", payload_text(&*p)),
    }
}

// ── Propagation ─────────────────────────────────────────────────────────────

/// A concurrent system panics on a 4-worker pool. `Schedule::run` must panic on the caller with
/// the system's message.
///
/// Red: the watchdog fires (the executor waits forever for a completion the panicking system never
/// published), `run` returns normally, or the message is not the system's.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn parallel_system_panic_resumes_on_schedule_caller() {
    let outcome = watch("parallel", || {
        let pool = pool(4);
        let mut world = world_with_rows();
        let runs = Arc::new(AtomicUsize::new(0));
        let mut schedule = parallel_schedule(&pool, &mut world, true, &runs);
        schedule.run(&mut world);
    });
    expect_message("parallel", outcome, MSG_PARALLEL);
}

/// A plain system panics on a ONE-worker pool — the configuration with no second worker to mask
/// anything.
///
/// Red: watchdog, a normal return, or the wrong message.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn single_worker_system_panic_resumes_on_schedule_caller() {
    let outcome = watch("single-worker", || {
        let pool = pool(1);
        let mut world = world_with_rows();
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        builder.add_system(|q: Query<&A6Row>| {
            let _rows = q.iter().count();
            panic!("{MSG_SINGLE}");
        });
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);
    });
    expect_message("single-worker", outcome, MSG_SINGLE);
}

/// An exclusive system (`&mut EcsMaster`) panics. It runs inline on the dispatcher, so its unwind
/// leaves the executor loop directly.
///
/// Red: watchdog, a normal return, or the wrong message.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn exclusive_system_panic_resumes_on_schedule_caller() {
    let outcome = watch("exclusive", || {
        let pool = pool(2);
        let mut world = world_with_rows();
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        builder.add_system(|_world: &mut EcsMaster| {
            panic!("{MSG_EXCLUSIVE}");
        });
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);
    });
    expect_message("exclusive", outcome, MSG_EXCLUSIVE);
}

/// A `par_iter` row body panics inside a concurrent system. The `par_iter` scope resumes the panic
/// into the system body on its worker; the schedule must then resume it on the caller.
///
/// Red: watchdog, a normal return, or the wrong message.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn par_iter_panic_in_system_resumes_on_schedule_caller() {
    let outcome = watch("par-iter", || {
        let pool = pool(4);
        let mut world = world_with_rows();
        let seen = Arc::new(AtomicUsize::new(0));
        let mut schedule = par_iter_schedule(&pool, &mut world, true, &seen);
        schedule.run(&mut world);
    });
    expect_message("par-iter", outcome, MSG_PAR_ITER);
}

/// A `par_for_each_chunk` body panics (on the chunk holding [`PANIC_ROW`]) inside a concurrent
/// system.
///
/// Red: watchdog, a normal return, or the wrong message.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn par_for_each_chunk_panic_in_system_resumes_on_schedule_caller() {
    let outcome = watch("par-chunk", || {
        let pool = pool(4);
        let mut world = world_with_rows();
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        builder.add_system(|mut q: Query<&A6Row>| {
            q.par_for_each_chunk(
                |slice: &[A6Row]| {
                    if slice.iter().any(|row| row.0 == PANIC_ROW) {
                        panic!("{MSG_PAR_CHUNK}");
                    }
                },
                BatchingStrategy::default(),
            );
        });
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);
    });
    expect_message("par-chunk", outcome, MSG_PAR_CHUNK);
}

/// A `par_iter` row body panics inside an EXCLUSIVE system — the `par_iter` scope is joined by the
/// dispatcher itself (an external joiner), then the unwind leaves the executor loop inline.
///
/// Red: watchdog, a normal return, or the wrong message.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn par_iter_panic_in_exclusive_system_resumes_on_schedule_caller() {
    let outcome = watch("exclusive-par-iter", || {
        let pool = pool(4);
        let mut world = world_with_rows();
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        builder.add_system(|world: &mut EcsMaster| {
            let view = world.query::<&A6Row, ()>();
            view.par_iter().for_each(|row: &A6Row| {
                if row.0 == PANIC_ROW {
                    panic!("{MSG_EXCLUSIVE_PAR_ITER}");
                }
            });
        });
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);
    });
    expect_message("exclusive-par-iter", outcome, MSG_EXCLUSIVE_PAR_ITER);
}

/// `App::update_with_delta` — the call the windowed runner makes once per frame — with a system
/// that panics. The frame call must panic on the caller.
///
/// Red: watchdog (the runner's frame would hang exactly as observed), a normal return, or the
/// wrong message.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn app_update_resumes_system_panic_on_caller() {
    let outcome = watch("app-update", || {
        register_layout::<A6Row>(SLOT_A6_ROW.0);
        let mut app = App::with_threads(2);
        app.add_systems(|q: Query<&A6Row>| {
            let _rows = q.iter().count();
            panic!("{MSG_APP}");
        });
        app.update_with_delta(Duration::from_millis(16));
    });
    expect_message("app-update", outcome, MSG_APP);
}

// ── Recovery ────────────────────────────────────────────────────────────────

/// After an exclusive system's panic has been resumed, the SAME schedule, world and pool must run
/// a second frame to completion (the system panics on its first run only).
///
/// Red: stage 1 does not panic, stage 2 blocks (watchdog) or panics, or the system is not entered
/// twice.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn schedule_runs_again_after_an_exclusive_panic() {
    let outcome = watch("exclusive-recovery", || {
        let pool = pool(2);
        let mut world = world_with_rows();
        let runs = Arc::new(AtomicUsize::new(0));
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        let body_runs = Arc::clone(&runs);
        builder.add_system(move |_world: &mut EcsMaster| {
            if body_runs.fetch_add(1, Ordering::AcqRel) == 0 {
                panic!("{MSG_ONCE}");
            }
        });
        let mut schedule = builder.build(&mut world);

        let first = catch_unwind(AssertUnwindSafe(|| schedule.run(&mut world)));
        assert!(first.is_err(), "stage 1: the first run must resume the system's panic");

        schedule.run(&mut world);
        runs.load(Ordering::Acquire)
    });
    match outcome {
        Ok(runs) => assert_eq!(runs, 2, "stage 2: the system must have been entered on both runs"),
        Err(p) => panic!("exclusive-recovery failed: {:?}", payload_text(&*p)),
    }
}

/// After a concurrent system's panic, the SAME schedule, world and pool must run a second frame
/// to completion (system 1 panics on its first run only).
///
/// Red: stage 1 blocks (watchdog — the defect itself), stage 1 does not panic, stage 2 blocks or
/// panics, or the three systems are not each entered on both runs.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn schedule_runs_again_after_a_parallel_panic() {
    let outcome = watch("parallel-recovery", || {
        let pool = pool(4);
        let mut world = world_with_rows();
        let runs = Arc::new(AtomicUsize::new(0));
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        for idx in 0..3usize {
            let body_runs = Arc::clone(&runs);
            let first_of_one = Arc::new(AtomicUsize::new(0));
            builder.add_system(move |q: Query<&A6Row>| {
                body_runs.fetch_add(1, Ordering::AcqRel);
                let _rows = q.iter().count();
                if idx == 1 && first_of_one.fetch_add(1, Ordering::AcqRel) == 0 {
                    panic!("{MSG_ONCE}");
                }
            });
        }
        let mut schedule = builder.build(&mut world);

        let first = catch_unwind(AssertUnwindSafe(|| schedule.run(&mut world)));
        assert!(first.is_err(), "stage 1: the first run must resume the system's panic");

        schedule.run(&mut world);
        runs.load(Ordering::Acquire)
    });
    match outcome {
        Ok(runs) => assert_eq!(runs, 6, "stage 2: each of the three systems must run on both runs"),
        Err(p) => panic!("parallel-recovery failed: {:?}", payload_text(&*p)),
    }
}

// ── Design Rev 7 validation rows (P2, P3, P9, P12, P13) ─────────────────────
//
// Everything below this line is the validation set the A6 design specifies by
// name. Each row states, at its own site, the mutation that reds it — a test
// whose red cannot be named is a claim rather than a gate, and every one of
// these was RUN against its mutation before being reported green.

/// P2's message: only the panicking system's own, so a red cannot be satisfied by any other panic.
const MSG_CANCEL: &str = "A6 cancellation-granularity panic";
/// P12's message — raised by a queued COMMAND inside the apply window, not by a system body.
const MSG_CMD: &str = "A6 queued command panic in the apply window";
/// P9's message on the second panicking run, so runs 1 and 2 are distinguishable in the payload.
const MSG_SECOND: &str = "A6 second consecutive panicking run";

/// P12's command: counts its own `apply`, then panics on the FIRST apply of the process-global
/// latch it is handed.
///
/// The latch lives in the COMMAND rather than in the system, which is the design's own repair: pop
/// order is completion order, so a latch in the system could fire on the window's LAST entry and
/// leave zero leftovers — making M16 a mutation with no red. With the latch here the panic lands
/// on the first entry the window applies, whatever the completion order, so `target - drained >= 1`
/// is a fact rather than a hope.
struct Boom {
    /// Incremented by every `apply`, panicking or not.
    applied: Arc<AtomicUsize>,
    /// `false -> true` exactly once; the swapper panics.
    latch: Arc<AtomicBool>,
}

impl Command for Boom {
    fn apply(self, _world: &mut EcsMaster) {
        self.applied.fetch_add(1, Ordering::AcqRel);
        if !self.latch.swap(true, Ordering::AcqRel) {
            panic!("{MSG_CMD}");
        }
    }
}

/// P3's command: records that it was applied. A queued command of an ABORTED run must not run.
struct MarkApplied(Arc<AtomicBool>);

impl Command for MarkApplied {
    fn apply(self, _world: &mut EcsMaster) {
        self.0.store(true, Ordering::Release);
    }
}

/// P2 — a system panic cancels the REST of the schedule at round granularity: a successor of the
/// panicking system must not run, while a sibling co-dispatched in the same wave must.
///
/// Red: `B_ran == true` (no cancellation — M2 deletes `panicked_claim`, M3 deletes the round's
/// cancel check, and both land here), `C_ran == false` (the granularity became per-system), a
/// watchdog, or a run that does not panic at all.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn panic_cancels_the_rest_of_the_schedule() {
    let b_ran = Arc::new(AtomicBool::new(false));
    let c_ran = Arc::new(AtomicBool::new(false));
    let outcome = watch("cancel-granularity", {
        let b_ran = Arc::clone(&b_ran);
        let c_ran = Arc::clone(&c_ran);
        move || {
            let pool = pool(4);
            let mut world = world_with_rows();
            let mut builder = ScheduleBuilder::new(Arc::clone(&pool));

            // A: panics. Conflict-free with C (both read `A6Row`), so they share one wave.
            let a = builder
                .add_system(|q: Query<&A6Row>| {
                    let _rows = q.iter().count();
                    panic!("{MSG_CANCEL}");
                })
                .key();

            // B: ordered AFTER A, so it can only be dispatched in a LATER round — the round the
            // cancel check must never reach.
            builder
                .add_system(move |q: Query<&A6Row>| {
                    let _rows = q.iter().count();
                    b_ran.store(true, Ordering::Release);
                })
                .after(a);

            // C: conflict-free with A and unordered, so it is co-dispatched in A's own wave.
            // Cancellation is at ROUND granularity, so C runs.
            builder.add_system(move |q: Query<&A6Row>| {
                let _rows = q.iter().count();
                c_ran.store(true, Ordering::Release);
            });

            let mut schedule = builder.build(&mut world);
            schedule.run(&mut world);
        }
    });
    expect_message("cancel-granularity", outcome, MSG_CANCEL);
    assert!(
        !b_ran.load(Ordering::Acquire),
        "P2: the successor of the panicking system must NOT run -- the cancel check must stop the \
         dispatcher before the next round"
    );
    assert!(
        c_ran.load(Ordering::Acquire),
        "P2: the sibling co-dispatched in the panicking system's OWN wave must still run -- \
         cancellation is at round granularity, not per system (and a false here would make the B \
         assertion above vacuous, since nothing would have run at all)"
    );
}

/// P3 — a panicking system's queued commands are not applied by the aborted run.
///
/// The system is a ONE-SHOT (`MSG_ONCE`), so run 2 completes and the leftover command applies
/// there: that is the design's post-panic contract, "the commands stay queued and the next run
/// applies them", and asserting it is what stops this test passing over a run that silently
/// DISCARDED the queue.
///
/// Red: run 1 growing `entity_count` (M4 — `cancel_after_panic` calling `apply_window_drain`
/// instead of the accounting-only drain), run 2 blocking or panicking, or the leftover never
/// applying at all.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn a_panicking_systems_queued_commands_are_not_applied() {
    let applied = Arc::new(AtomicBool::new(false));
    let applied_after_run1 = Arc::new(AtomicBool::new(false));
    let grown_by_run1 = Arc::new(AtomicUsize::new(usize::MAX));
    let outcome = watch("queued-commands", {
        let applied = Arc::clone(&applied);
        let applied_after_run1 = Arc::clone(&applied_after_run1);
        let grown_by_run1 = Arc::clone(&grown_by_run1);
        move || {
            let pool = pool(4);
            let mut world = world_with_rows();
            let seeded = world.entity_count();

            let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
            let once = Arc::new(AtomicUsize::new(0));
            let mark = Arc::clone(&applied);
            builder.add_system(move |mut c: Commands| {
                c.spawn_empty();
                c.add(MarkApplied(Arc::clone(&mark)));
                if once.fetch_add(1, Ordering::AcqRel) == 0 {
                    panic!("{MSG_ONCE}");
                }
            });
            let mut schedule = builder.build(&mut world);

            let first = catch_unwind(AssertUnwindSafe(|| schedule.run(&mut world)));
            assert!(first.is_err(), "stage 1: the first run must resume the system's panic");
            grown_by_run1.store(world.entity_count() - seeded, Ordering::Release);
            applied_after_run1.store(applied.load(Ordering::Acquire), Ordering::Release);

            // Run 2: the schedule is reusable and the leftover queue applies here.
            schedule.run(&mut world);
            world.entity_count() - seeded
        }
    });
    match outcome {
        Ok(grown_total) => {
            assert_eq!(
                grown_by_run1.load(Ordering::Acquire),
                0,
                "P3: the aborted run must not have applied its own `Commands::spawn_empty` -- the \
                 cancel path performs accounting only and never takes `world_mut()`"
            );
            assert!(
                !applied_after_run1.load(Ordering::Acquire),
                "P3: the aborted run must not have applied its own marker command"
            );
            assert_eq!(
                grown_total, 2,
                "P3 (anti-vacuity): run 2 must apply BOTH run 1's leftover spawn and its own -- a \
                 zero here would mean the queue was discarded, which is a different contract and \
                 would make the run-1 assertions pass over an empty queue"
            );
            assert!(
                applied.load(Ordering::Acquire),
                "P3 (anti-vacuity): the marker command must have applied by the end of run 2"
            );
        }
        Err(p) => panic!("queued-commands failed: {:?}", payload_text(&*p)),
    }
}

/// P9 — two consecutive panicking runs each resume on the caller, and the third completes.
///
/// The cancel flag is re-armed per frame, so the cancel path is entered TWICE. A sticky flag would
/// make run 2 cancel round 1, dispatch nothing and return NORMALLY.
///
/// Red: run 2 returning normally (M14 — the deleted `panicked_reset`), either run carrying the
/// wrong payload, run 3 blocking or panicking, or the body not being entered on all three runs.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn two_consecutive_panicking_runs_each_resume_on_the_caller() {
    let outcome = watch("two-panicking-runs", || {
        let pool = pool(4);
        let mut world = world_with_rows();
        let runs = Arc::new(AtomicUsize::new(0));
        let body_runs = Arc::clone(&runs);
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        builder.add_system(move |q: Query<&A6Row>| {
            let n = body_runs.fetch_add(1, Ordering::AcqRel);
            let _rows = q.iter().count();
            match n {
                0 => panic!("{MSG_ONCE}"),
                1 => panic!("{MSG_SECOND}"),
                _ => {}
            }
        });
        let mut schedule = builder.build(&mut world);

        let first = catch_unwind(AssertUnwindSafe(|| schedule.run(&mut world)));
        let second = catch_unwind(AssertUnwindSafe(|| schedule.run(&mut world)));
        schedule.run(&mut world);
        (first, second, runs.load(Ordering::Acquire))
    });
    match outcome {
        Ok((first, second, runs)) => {
            expect_message("two-panicking-runs/1", first, MSG_ONCE);
            expect_message("two-panicking-runs/2", second, MSG_SECOND);
            assert_eq!(
                runs, 3,
                "P9: the body must have been entered on all three runs -- a sticky cancel flag \
                 makes run 2 dispatch nothing and return normally"
            );
        }
        Err(p) => panic!("two-panicking-runs failed: {:?}", payload_text(&*p)),
    }
}

/// P12 — a queued command that panics INSIDE the apply window leaves the schedule reusable.
///
/// Eight systems taking `Commands` only: no component access, so they are conflict-free and share
/// ONE wave, and the window's `target` is 8. `Boom`'s latch fires on the FIRST entry the window
/// applies, so `drained == 1` and seven leftovers are owed to `ApplyDrainGuard::drop`.
///
/// Red: run 1 not panicking with `MSG_CMD`; run 2 blocking (M17 — the two-edit reproduction, which
/// spins in a cleanup pad) or panicking with `invariant SCH6: completion_queue must drain across
/// frames` (M16 — the deleted leftover loop); `bodies != 16` (the wave-width pin, and M16's
/// anti-vacuity floor: eight single-entry rounds would give 9); `applied != 16`.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn a_command_panic_in_the_apply_window_leaves_the_schedule_reusable() {
    const SYSTEMS: usize = 8;
    let bodies = Arc::new(AtomicUsize::new(0));
    let applied = Arc::new(AtomicUsize::new(0));
    let outcome = watch("apply-window-command-panic", {
        let bodies = Arc::clone(&bodies);
        let applied = Arc::clone(&applied);
        move || {
            let pool = pool(4);
            let mut world = world_with_rows();
            let latch = Arc::new(AtomicBool::new(false));

            let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
            for _ in 0..SYSTEMS {
                let bodies = Arc::clone(&bodies);
                let applied = Arc::clone(&applied);
                let latch = Arc::clone(&latch);
                builder.add_system(move |mut c: Commands| {
                    bodies.fetch_add(1, Ordering::AcqRel);
                    c.add(Boom { applied: Arc::clone(&applied), latch: Arc::clone(&latch) });
                });
            }
            let mut schedule = builder.build(&mut world);

            let first = catch_unwind(AssertUnwindSafe(|| schedule.run(&mut world)));
            let payload = first.expect_err(
                "stage 1: the command's panic must reach the caller -- `CommandQueue::apply` \
                 re-raises rather than swallowing",
            );
            let text = payload_text(&*payload).map(str::to_owned);
            assert!(
                text.as_deref().is_some_and(|t| t.contains(MSG_CMD)),
                "stage 1: the caller must receive the COMMAND's panic, not a derived one: {text:?}"
            );

            // Run 2 must return normally. In a debug build this also subsumes the SCH6 clause: a
            // leftover entry surfaces here as a `reset_for_frame` panic with that text.
            schedule.run(&mut world);
        }
    });
    match outcome {
        Ok(()) => {}
        Err(p) => panic!(
            "P12: run 2 must return normally -- a leftover completion entry reds \
             `reset_for_frame`'s SCH6 assert: {:?}",
            payload_text(&*p)
        ),
    }
    assert_eq!(
        bodies.load(Ordering::Acquire),
        2 * SYSTEMS,
        "P12: the wave-width pin. All eight systems must be entered on BOTH runs; if they had \
         serialised into eight single-entry rounds, run 1 would have entered one body and this \
         would read 9 -- and M16's red would be unreachable because the window would owe no \
         leftovers"
    );
    assert_eq!(
        applied.load(Ordering::Acquire),
        2 * SYSTEMS,
        "P12: 1 apply in run 1 (the panicker, whose own queue `CommandQueue::apply`'s recovery \
         then consumed), then 7 leftovers plus 8 fresh ones in run 2"
    );
}

/// P13 — a sibling still running when the cancel flag is set is still accounted for.
///
/// A FORCED interleaving, because `schedule_runs_again_after_a_parallel_panic` has no ordering
/// device and would flake on this property rather than test it: systems 1 and 2 wait for system
/// 0's claim and then sleep, so their publish is provably later than the cancel claim and later
/// than the dispatcher's next round. `cancel_after_panic`'s OUTER loop is what must absorb them;
/// a single pass leaves their entries queued.
///
/// Red: M18 (the single-pass cancel drain) — run 2 panics with `invariant SCH6: completion_queue
/// must drain across frames`. Also red on a watchdog, on run 1 not panicking, or on `runs != 6`.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, a forced 50 ms sleep, and a red path that leaks a blocked thread by design. Runs natively."
)]
#[test]
fn a_sibling_still_running_when_the_cancel_flag_is_set_is_still_accounted() {
    /// How long the two siblings stay inside their bodies after the panicking system has claimed
    /// the flag. Long enough that the dispatcher has provably entered `cancel_after_panic` and
    /// taken its first `pending` load before either publishes.
    const SIBLING_HOLD: Duration = Duration::from_millis(50);
    /// Bound on the spin, so a scheduling accident is a test FAILURE and not a hang.
    const SPIN_CAP: usize = 50_000_000;

    let outcome = watch("cancel-drain-interleaving", || {
        let pool = pool(4);
        let mut world = world_with_rows();
        let runs = Arc::new(AtomicUsize::new(0));
        let panicked = Arc::new(AtomicBool::new(false));
        let once = Arc::new(AtomicBool::new(false));

        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        for idx in 0..3usize {
            let runs = Arc::clone(&runs);
            let panicked = Arc::clone(&panicked);
            let once = Arc::clone(&once);
            builder.add_system(move |q: Query<&A6Row>| {
                // FIRST statement in every body, including the panicking one: `runs == 6` counts
                // three systems over two runs and would be unreachable if system 0 did not
                // increment.
                runs.fetch_add(1, Ordering::Relaxed);
                let _rows = q.iter().count();
                if idx == 0 {
                    if !once.swap(true, Ordering::AcqRel) {
                        panicked.store(true, Ordering::Release);
                        panic!("{MSG_ONCE}");
                    }
                    return;
                }
                // Siblings: publish strictly AFTER the claim, and after the dispatcher's next
                // round. On run 2 nothing panics, so the flag is already set from run 1 and the
                // spin falls through immediately — which is correct: run 2's property is only
                // that it completes.
                let mut spins = 0usize;
                while !panicked.load(Ordering::Acquire) {
                    spins += 1;
                    assert!(
                        spins < SPIN_CAP,
                        "P13 fixture: the panicking system never claimed the flag; the forced \
                         interleaving did not happen and this run would prove nothing"
                    );
                    std::thread::yield_now();
                }
                std::thread::sleep(SIBLING_HOLD);
            });
        }
        let mut schedule = builder.build(&mut world);

        let first = catch_unwind(AssertUnwindSafe(|| schedule.run(&mut world)));
        assert!(first.is_err(), "stage 1: the first run must resume the system's panic");

        schedule.run(&mut world);
        runs.load(Ordering::Acquire)
    });
    match outcome {
        Ok(runs) => assert_eq!(
            runs, 6,
            "P13: each of the three systems must be entered on both runs -- a completion published \
             while the cancel loop was parked must be absorbed by a later iteration of that loop, \
             not left queued for the next frame"
        ),
        Err(p) => panic!(
            "P13: run 2 must return normally -- a completion the cancel drain failed to absorb \
             reds `reset_for_frame`'s SCH6 assert: {:?}",
            payload_text(&*p)
        ),
    }
}

// ── Design validation row C5 for the schedule's record ──────────────────────

/// C5's message for the named system below.
const MSG_NAMED: &str = "A6 named system panic for the cancellation record";

/// A NAMED system, so the cancellation record's name argument is a string this test can look for:
/// a system's name is `std::any::type_name` of its function, and a closure's is an opaque path.
/// Panics on its first run only.
fn a6_named_panicking_system(q: Query<&A6Row>, mut first: boyko_ecs::ecs::core::system::Local<bool>) {
    let _rows = q.iter().count();
    if !*first {
        *first = true;
        panic!("{MSG_NAMED}");
    }
}

/// C5 — a cancelled run emits exactly ONE `E1502` record, on the thread that called
/// `Schedule::run`, naming the system that panicked; a clean run emits none.
///
/// Observed rather than merely named: the probe counts emissions charged to the calling thread
/// (the dispatcher runs `cancel_after_panic` there), and renders the record's arguments.
///
/// Red: zero records (the emission deleted, or moved to a worker thread); two or more (emitted
/// per drain iteration instead of once per cancelled run); a record that does not name
/// `a6_named_panicking_system` (the index-to-name read is wrong); a record on the clean second
/// run; or the usual watchdog / wrong-payload reds.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the parallel executor is Miri-covered by `miri_schedule_parallel.rs`. Runs natively."
)]
#[test]
fn a_cancelled_run_reports_e1502_naming_the_panicking_system() {
    let outcome = watch("e1502-record", || {
        // Raise the schedule target's ceiling BEFORE the emission, or the record is never
        // constructed and a count of zero would be green for the wrong reason.
        boyko_log::probe::arm::<boyko_log::Schedule>();
        let pool = pool(4);
        let mut world = world_with_rows();
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        builder.add_system(a6_named_panicking_system);
        // A conflict-free sibling, so the cancel drain has another completion to account for.
        builder.add_system(|q: Query<&A6Row>| {
            let _rows = q.iter().count();
        });
        let mut schedule = builder.build(&mut world);

        boyko_log::probe::watch(b'E', boyko_log::codes::E1502.number());
        let first = catch_unwind(AssertUnwindSafe(|| schedule.run(&mut world)));
        let records_after_cancel = boyko_log::probe::watched();
        let message = boyko_log::probe::last_message();

        schedule.run(&mut world);
        let records_after_clean = boyko_log::probe::watched();
        (first, records_after_cancel, message, records_after_clean)
    });
    match outcome {
        Ok((first, records_after_cancel, message, records_after_clean)) => {
            expect_message("e1502-record", first, MSG_NAMED);
            assert_eq!(
                records_after_cancel, 1,
                "C5/E1502: a cancelled run must emit exactly one record on the thread that called \
                 Schedule::run; last message: {message:?}"
            );
            assert!(
                message.contains("a6_named_panicking_system"),
                "C5/E1502: the record must name the system that panicked: {message:?}"
            );
            assert!(
                message.contains("cancelled"),
                "C5/E1502: the record must say the rest of the run was cancelled: {message:?}"
            );
            assert_eq!(
                records_after_clean, 1,
                "C5/E1502: a run in which nothing panics must emit no cancellation record"
            );
        }
        Err(p) => panic!("e1502-record failed: {:?}", payload_text(&*p)),
    }
}

// ── The cancel path under Miri ──────────────────────────────────────────────

/// The whole A6 recovery path — a panicking guard's `Drop` (claim + publish on the unwinding
/// path), the round's cancel check, `cancel_after_panic`'s Miri-cooperative drain, `Scope::drop`'s
/// re-raise, and the next run's `reset_for_frame` re-arm — on a world small enough for Miri.
///
/// Every other test in this file is ignored under Miri for its wall-clock watchdog, so without
/// this one the `#[cfg(miri)]` arms of the cancel drain are compiled by nothing and executed by
/// nothing. Natively it runs under the same watchdog as the rest; under Miri it runs bare, because
/// Miri reports a deadlock itself and a leaked blocked thread would be an error there.
///
/// Red: run 1 not resuming the system's panic, run 2 failing, or `runs != 4` (two systems, two
/// runs); under Miri, any UB report or a livelock in the cancel drain.
#[test]
fn a_cancelled_parallel_run_on_a_small_world_is_reusable() {
    fn scenario() -> usize {
        register_layout::<A6Row>(SLOT_A6_ROW.0);
        let pool = pool(2);
        let mut world = EcsMaster::new();
        let arch = world.create_archetype(&[SLOT_A6_ROW]);
        for i in 0..4 {
            world
                .spawn_one(arch, A6Row(i))
                .expect("test setup: seeding the small archetype must succeed");
        }
        let runs = Arc::new(AtomicUsize::new(0));
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        for idx in 0..2usize {
            let runs = Arc::clone(&runs);
            let once = Arc::new(AtomicBool::new(false));
            builder.add_system(move |q: Query<&A6Row>| {
                runs.fetch_add(1, Ordering::AcqRel);
                let _rows = q.iter().count();
                if idx == 1 && !once.swap(true, Ordering::AcqRel) {
                    panic!("{MSG_ONCE}");
                }
            });
        }
        let mut schedule = builder.build(&mut world);

        let first = catch_unwind(AssertUnwindSafe(|| schedule.run(&mut world)));
        expect_message("small-world/1", first, MSG_ONCE);
        schedule.run(&mut world);
        runs.load(Ordering::Acquire)
    }

    #[cfg(miri)]
    let runs = scenario();
    #[cfg(not(miri))]
    let runs = match watch("small-world-cancel", scenario) {
        Ok(runs) => runs,
        Err(p) => panic!("small-world-cancel failed: {:?}", payload_text(&*p)),
    };
    assert_eq!(runs, 4, "both systems must be entered on both runs");
}
