//! KE16 — the occupancy gate for threadpool defect A.
//!
//! Two tests over ONE fixture and ONE instrument, differing only in where the
//! `par_iter` is driven from:
//!
//!   * [`par_iter_from_dispatcher_reaches_more_than_one_thread`] — the CONTROL.
//!     Driven from the test thread inside `pool.install`, where `push_task`
//!     routes to `injector_global`. It must be GREEN today; if it is not, the
//!     instrument is broken and the gate below proves nothing.
//!   * [`par_iter_in_system_reaches_more_than_one_thread`] — the GATE. The same
//!     call from inside a scheduled system body, which the parallel scheduler
//!     runs ON A WORKER. It was written RED against the placement that routed
//!     such a push into `injector_local[wid]`, which no sibling ever polled; the
//!     placement that ships pushes onto the worker's OWN registered deque
//!     (`worker::push_on_lane_no_wake`), whose `Stealer` every sibling scans, so
//!     the wave fans out and the gate is expected GREEN. It is no longer
//!     `#[ignore]`d — Step App orders that removal by name
//!     (`docs/threadpool/KE16-DESIGN-APP.md` §10).
//!
//! ## Why max-in-flight and not distinct thread ids
//!
//! Counting distinct thread ids is a known-bad instrument on this path: a
//! scope's LIFO drain can retire a light batch on the calling thread before any
//! steal lands, so a CORRECT dispatch can report one thread. The counter here
//! measures the property that matters directly — how many bodies were executing
//! at the same instant — and a spin body of `SPIN_PER_ROW` makes the window
//! four orders of magnitude wider than a steal.
//!
//! ## Why both tests share one instrument, and what that requires of the runner
//!
//! `IN_FLIGHT` / `MAX_IN_FLIGHT` / `ROWS_SEEN` / `SYSTEM_WORKER_ID` are process-global, and both
//! tests reset them at the start of their own pass, so the two must not run at the same time.
//! [`INSTRUMENT`] enforces that inside the binary; the measurement protocol
//! (`docs/threadpool/KE16-DESIGN-APP.md` §10) additionally passes `--test-threads=1` so a
//! reading is taken with nothing else on the box at all.
//!
//! ⚠ Until 2026-09-09 nothing enforced it and the paragraph here credited the flag. What was
//! actually keeping the passes apart was the GATE's plain `#[ignore]` — so the moment Step App
//! removed that by name, the default workspace command (`cargo test --workspace --all-targets`,
//! which passes no `--test-threads=1`) would have run them together. The lock is the property
//! the ignore had been providing by accident, made explicit.
//!
//! Component id 493 is reserved for this test binary.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::hint::black_box;
use std::thread::available_parallelism;
use std::time::{Duration, Instant};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_threadpool::{MAX_WORKERS, ThreadPoolBuilder, current_worker_id};

const SLOT_KE16_GATE: ComponentId = ComponentId(493);

#[repr(C)]
#[derive(Clone, Copy)]
struct Ke16Gate(u32);

impl Component for Ke16Gate {
    fn component_id() -> ComponentId {
        SLOT_KE16_GATE
    }
}

/// 4096 rows in ONE archetype. With the default `BatchingStrategy` the chunk
/// size clamps UP to `MIN_ARCHETYPE_FOR_PARALLEL` (1024), so this yields four
/// chunks — enough to tell 1 from >1 without depending on the worker count.
const N_ROWS: usize = 4096;

/// Per-row busy-wait. 4096 x 100 µs = ~0.41 s serial, ~0.10 s across four
/// chunks: long enough that a steal cannot be outrun, short enough for CI.
const SPIN_PER_ROW: Duration = Duration::from_micros(100);

/// Occupancy floor both routes are held to. Deliberately 2 rather than the
/// worker count: the claim under test is "work spawned inside a system is
/// reachable by its own worker ALONE", and 2 is the smallest number that
/// refutes it, so the gate cannot be argued away as a scheduling artefact.
const MIN_EXPECTED_IN_FLIGHT: usize = 2;

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static MAX_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static ROWS_SEEN: AtomicUsize = AtomicUsize::new(0);
static SINK: AtomicU64 = AtomicU64::new(0);
/// Highest worker id the GATE's system body ever ran on — the route receipt.
///
/// A `fetch_max` rather than a store: the scheduler places the system on whichever worker is free,
/// so the datum is not "which one" but "was it ever off-pool". The sentinels
/// (`WORKER_ID_DISPATCHER = u32::MAX - 1`, `WORKER_ID_UNATTACHED = u32::MAX`) are the two largest
/// `u32`s, so a max is exactly the test for "some run was not on a registered worker".
static SYSTEM_WORKER_ID: AtomicU32 = AtomicU32::new(0);

/// Serialises the two passes, which share the process-global instrument above.
///
/// Held from BEFORE [`reset_instruments`] until past the last assertion, so no pass can zero a
/// counter under another's live wave. The concrete damage without it: one pass's reset zeroes
/// `ROWS_SEEN` mid-flight, so the other's `assert_eq!(rows, N_ROWS)` is flaky-red; and zeroing
/// `IN_FLIGHT` under live bodies makes their later `fetch_sub(1)` wrap a `usize`, which then
/// feeds `MAX_IN_FLIGHT.fetch_max` — garbage in the PASSING direction, which is worse.
///
/// Poisoning is absorbed (`into_inner`) deliberately: if one pass panics, the other should
/// report its own result rather than a poison error naming the wrong test.
///
/// `#[allow(clippy::disallowed_types)]` with its rationale, per CLAUDE.md's exception rule: the
/// `Mutex` ban is about the engine's hot path. This is a test-binary lock taken twice per run,
/// outside every measured region — the wave itself runs with the guard already held and never
/// contends for it.
#[allow(clippy::disallowed_types)]
static INSTRUMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Takes [`INSTRUMENT`] for the caller's whole pass, absorbing poisoning.
///
/// Fully qualified rather than imported: an `#[allow]` on an item does not reach a `use`
/// statement, so importing the type would put a bare `Mutex` in the file with no rationale
/// attached to it — which is exactly what the ban exists to make visible.
#[allow(clippy::disallowed_types)]
fn hold_instrument() -> std::sync::MutexGuard<'static, ()> {
    INSTRUMENT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn reset_instruments() {
    IN_FLIGHT.store(0, Ordering::SeqCst);
    MAX_IN_FLIGHT.store(0, Ordering::SeqCst);
    ROWS_SEEN.store(0, Ordering::SeqCst);
    SYSTEM_WORKER_ID.store(0, Ordering::SeqCst);
}

#[inline(never)]
fn spin_body(v: &Ke16Gate) {
    let now_in_flight = IN_FLIGHT.fetch_add(1, Ordering::AcqRel) + 1;
    MAX_IN_FLIGHT.fetch_max(now_in_flight, Ordering::AcqRel);
    ROWS_SEEN.fetch_add(1, Ordering::Relaxed);

    let deadline = Instant::now() + SPIN_PER_ROW;
    let mut acc = v.0 as u64;
    while Instant::now() < deadline {
        acc = black_box(acc.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1));
    }
    SINK.fetch_add(acc & 1, Ordering::Relaxed);

    IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
}

fn build_world() -> EcsMaster {
    register_layout::<Ke16Gate>(SLOT_KE16_GATE.0);
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[SLOT_KE16_GATE]);
    for i in 0..N_ROWS {
        world
            .spawn_one(arch, Ke16Gate(i as u32))
            .expect("invariant: gate fixture seed must succeed");
    }
    world
}

fn worker_count() -> usize {
    available_parallelism().map(|n| n.get()).unwrap_or(4).max(4)
}

/// CONTROL — the healthy route. The test thread is not a registered worker, so
/// `install` labels it `WORKER_ID_DISPATCHER` and every chunk reaches
/// `injector_global`, which every worker polls.
///
/// This test is the reason the gate below is evidence rather than an
/// assertion: it proves the fixture, the instrument and the pool all work.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: 4096 rows x 100 us of real wall-clock spin across a real OS thread pool"
)]
fn par_iter_from_dispatcher_reaches_more_than_one_thread() {
    let _instrument = hold_instrument();
    let workers = worker_count();
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let mut world = build_world();

    reset_instruments();
    let t0 = Instant::now();
    pool.install(|_scope| {
        world.run_closure_once(|q: Query<&Ke16Gate>| {
            q.par_iter().for_each(spin_body);
        });
    });
    let wall = t0.elapsed();

    let rows = ROWS_SEEN.load(Ordering::SeqCst);
    let peak = MAX_IN_FLIGHT.load(Ordering::SeqCst);
    println!(
        "[ke16 control] workers={workers} rows={rows} wall={:.1} ms max_in_flight={peak}",
        wall.as_secs_f64() * 1e3
    );

    assert_eq!(rows, N_ROWS, "the control must visit every row exactly once");
    assert!(
        peak >= MIN_EXPECTED_IN_FLIGHT,
        "par_iter driven from the dispatcher peaked at {peak} concurrent bodies on a \
         {workers}-worker pool over {N_ROWS} rows. This is the HEALTHY route, so either the \
         instrument is broken or the pool is — and until it is fixed the \
         `par_iter_in_system_reaches_more_than_one_thread` gate below proves nothing."
    );
}

/// GATE (red-first) — the shipping route.
///
/// The parallel scheduler runs concurrent system bodies on WORKERS
/// (`schedule.rs`: only exclusive systems run inline on the dispatcher). It was
/// written RED against the placement that routed each `par_iter` chunk from a
/// worker into `injector_local[wid]`: sibling stealing walks `inner.stealers`,
/// which holds worker DEQUES only, so the owner's local injector was polled by
/// the owner alone, and `Scope::drop` then drained that injector into a private
/// `scratch` deque with no registered stealer and ran the batch inline — the
/// whole wave on one thread. The placement that ships pushes onto the worker's
/// own registered deque (`worker.rs::push_on_lane_no_wake`), whose `Stealer`
/// every sibling scans, so the wave fans out.
///
/// The control above proves the instrument reports >= 2 when the dispatch is
/// healthy, which is what makes this reading evidence rather than an assertion.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: 4096 rows x 100 us of real wall-clock spin across a real OS thread pool"
)]
fn par_iter_in_system_reaches_more_than_one_thread() {
    let _instrument = hold_instrument();
    let workers = worker_count();
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let mut world = build_world();

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&Ke16Gate>| {
        // The route receipt, taken BEFORE the wave so it is recorded even if the wave panics.
        SYSTEM_WORKER_ID.fetch_max(current_worker_id(), Ordering::AcqRel);
        q.par_iter().for_each(spin_body);
    });
    let mut schedule = builder.build(&mut world);

    reset_instruments();
    let t0 = Instant::now();
    schedule.run(&mut world);
    let wall = t0.elapsed();

    let rows = ROWS_SEEN.load(Ordering::SeqCst);
    let peak = MAX_IN_FLIGHT.load(Ordering::SeqCst);
    let route = SYSTEM_WORKER_ID.load(Ordering::SeqCst);
    println!(
        "[ke16 gate] workers={workers} rows={rows} wall={:.1} ms max_in_flight={peak} \
         system_worker_id={route}",
        wall.as_secs_f64() * 1e3
    );

    assert_eq!(rows, N_ROWS, "the gate must visit every row exactly once");
    // The route receipt is asserted BEFORE the occupancy floor: a body that ran off-pool pushes
    // its chunks to `injector_global`, where the wave fans out perfectly and `peak` reads high
    // — the healthy route reported under the defective route's name
    // (`KE16-DESIGN-MEASUREMENT.md` §5 shape 2). Without this check the gate
    // could flip red-to-green for the wrong reason, and that flip IS this campaign's verdict.
    assert!(
        (route as usize) < MAX_WORKERS,
        "the gate's system body recorded worker id {route} (>= MAX_WORKERS = \
         {MAX_WORKERS}): it ran off-pool (WORKER_ID_DISPATCHER = u32::MAX - 1, \
         WORKER_ID_UNATTACHED = u32::MAX), so its par_iter reached injector_global and \
         this reading is the healthy route measured twice, not the worker route this \
         gate exists to measure"
    );
    assert!(
        peak >= MIN_EXPECTED_IN_FLIGHT,
        "par_iter inside a scheduled system peaked at {peak} concurrent bodies on a \
         {workers}-worker pool over {N_ROWS} rows (wall {:.1} ms). Work spawned from inside the \
         system's own worker is not reaching its siblings — the regression this gate was written \
         against parked it in a queue no sibling polls.",
        wall.as_secs_f64() * 1e3
    );
}
