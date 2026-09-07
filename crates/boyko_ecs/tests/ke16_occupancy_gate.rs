//! KE16 — the occupancy gate for threadpool defect A.
//!
//! Two tests over ONE fixture and ONE instrument, differing only in where the
//! `par_iter` is driven from:
//!
//!   * [`par_iter_from_dispatcher_reaches_more_than_one_thread`] — the CONTROL.
//!     Driven from the test thread inside `pool.install`, where `push_task`
//!     routes to `injector_global`. It must be GREEN today; if it is not, the
//!     instrument is broken and the red gate below proves nothing.
//!   * [`par_iter_in_system_reaches_more_than_one_thread`] — the GATE. The same
//!     call from inside a scheduled system body, which the parallel scheduler
//!     runs ON A WORKER, so `push_task` routes to `injector_local[wid]` that no
//!     sibling ever polls. RED today; `#[ignore]`d so the branch stays green,
//!     and to be un-ignored by whoever fixes defect A.
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
//! That is why every invocation in the measurement protocol
//! (`docs/threadpool/KE16-DESIGN-APP.md` §10) passes `--test-threads=1`, and why the
//! ignored gate's reason repeats the requirement: un-ignoring it without that flag would let
//! libtest run the two passes concurrently and mix their occupancy readings.
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
use boyko_threadpool::{
    MAX_WORKERS, ThreadPoolBuilder, current_worker_id, ke16_check_expected_variant,
};

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

/// Prints the build's variant witness, then refuses to produce a reading for a
/// build that is not the one the tester named.
///
/// The print and the `KE16_EXPECT` check are TWO obligations, not one
/// (`KE16-DESIGN.md` §4). With `KE16_EXPECT` unset the check is silent, so
/// without the banner a recorded red-to-green flip of the gate below carries no
/// record of which build produced it — and that flip IS this campaign's
/// verdict. The spelling matches the pool crate's own KE16 harness files, so one
/// `grep 'KE16 variant'` collects every row of the protocol.
///
/// The banner is printed HERE rather than inside `ke16_check_expected_variant`:
/// a print from `crates/*/src/**.rs` reds `boyko-log`'s print census, and a test
/// binary is where the census does not look.
fn ke16_witness() {
    println!("KE16 variant: {}", boyko_threadpool::ke16_variant());
    ke16_check_expected_variant();
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
/// This test is the reason the ignored gate below is evidence rather than an
/// assertion: it proves the fixture, the instrument and the pool all work.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: 4096 rows x 100 us of real wall-clock spin across a real OS thread pool"
)]
fn par_iter_from_dispatcher_reaches_more_than_one_thread() {
    // Per TEST, not per file: the measurement protocol runs these FILTERED, so a witness printed
    // once from a harness main would not appear on the invocation whose number is recorded.
    ke16_witness();
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
         instrument is broken or the pool is — and until it is fixed the ignored \
         `par_iter_in_system_reaches_more_than_one_thread` gate below proves nothing."
    );
}

/// GATE (red-first) — the shipping route.
///
/// The parallel scheduler runs concurrent system bodies on WORKERS
/// (`schedule.rs`: only exclusive systems run inline on the dispatcher). From a
/// worker, `worker.rs::push_task` routes each `par_iter` chunk to
/// `injector_local[wid]`; sibling stealing walks `inner.stealers`, which holds
/// worker DEQUES only, and the owner's local injector is polled by the owner
/// alone. The worker then blocks in `Scope::drop`, which drains that injector
/// into a private `scratch` deque with no registered stealer and runs the batch
/// inline. So the whole wave executes on one thread and this assertion fails.
///
/// Un-ignore it when defect A is fixed; the control above already proves the
/// instrument reports >= 2 when the dispatch is healthy.
#[test]
#[ignore = "deferred: KE16 red-first occupancy gate; un-ignore when defect A \
            (worker-spawned work unreachable by siblings) is fixed, and run it with \
            --test-threads=1: this file's occupancy instrument is process-global and \
            shared with the control above"]
fn par_iter_in_system_reaches_more_than_one_thread() {
    ke16_witness();
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
         {workers}-worker pool over {N_ROWS} rows (wall {:.1} ms). Defect A: the system body runs \
         on a worker, so every chunk `push_task` writes lands in `injector_local[wid]`, which no \
         sibling polls, and `Scope::drop` runs the batch inline out of an unregistered scratch \
         deque.",
        wall.as_secs_f64() * 1e3
    );
}
