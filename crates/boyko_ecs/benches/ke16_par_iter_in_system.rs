//! KE16 — the ECS-level measurement harness for the worker-spawn route.
//!
//! A task pushed from a WORKER thread lands on that worker's own REGISTERED
//! deque (`worker.rs::push_task` -> `push_on_lane_no_wake`), whose stealer sits
//! in `inner.stealers` and is scanned by every sibling. Work spawned from
//! inside a worker is therefore reachable by the whole pool, exactly as work
//! pushed from off-pool is.
//!
//! That was not always so, and this harness is the instrument that measured it.
//! Defect A (measured 2026-08-30): the same push landed in
//! `injector_local[wid]`, which no other thread ever polls — sibling stealing
//! walks `inner.stealers`, which holds worker DEQUES only — and a worker
//! blocked inside `Scope::drop` drained its own local injector into a private,
//! unregistered `scratch` deque and ran the batch inline. Work spawned from
//! inside a worker was reachable by that one worker alone.
//!
//! The engine's parallel scheduler runs every concurrent system body on a
//! worker (`schedule.rs` — concurrent systems go through `scope.spawn`;
//! only EXCLUSIVE systems run inline on the dispatcher). So EVERY `par_iter`
//! written inside a system body takes the worker-spawn route, while the same
//! `par_iter` driven from a non-worker thread inside `pool.install` takes the
//! off-pool one (the dispatcher's worker id is `WORKER_ID_DISPATCHER`, so
//! `push_task` routes to `injector_global`, which every worker polls). This
//! bench measures the two side by side on IDENTICAL work, plus a sequential
//! baseline, so each speedup is `seq / route`.
//!
//! The pair is a REGRESSION receipt now rather than a diagnosis: the placement
//! that closed defect A also has to keep it closed, and a system route drifting
//! back toward its own sequential baseline is how a reopening would show.
//! Both ends are measured (`docs/threadpool/KE16-RESULTS.md`): before the fix
//! `par_in_system` equalled `seq` to four significant figures at BOTH
//! populations with `max_in_flight = 1` — the prior protocol's "1.01x inside a
//! system vs 7.69x from the dispatcher"; after it, the same route reached a
//! 14.0-15.3x speedup.
//!
//! ## Instrument
//!
//! Wall-clock alone cannot distinguish "did not fan out" from "fanned out onto
//! a slow machine", and counting distinct thread ids is known to lie here (a
//! scope's LIFO drain finishes a light batch before anyone can steal). So two
//! independent instruments run together:
//!
//!   1. **wall-clock** of one full pass over N rows, against a sequential
//!      baseline over the same rows;
//!   2. **max-in-flight** — `IN_FLIGHT.fetch_add` on body entry, the running
//!      value folded into `MAX_IN_FLIGHT` with `fetch_max`, `fetch_sub` on
//!      exit. This reports occupancy directly and is immune to the LIFO-drain
//!      objection.
//!
//! The body SPINS on `Instant` rather than sleeping, so a serialised wave
//! shows up as wall-clock and not as idle time, and every result goes through
//! `black_box` so the spin cannot be optimised away.
//!
//! ## Why 20 µs per row
//!
//! The prior protocol established that a trivial body is not a valid
//! instrument: the calling thread's drain retires the batch before a steal can
//! land. 20 µs per row puts a default chunk (>= `MIN_ARCHETYPE_FOR_PARALLEL` =
//! 1024 rows) at >= 20 ms — four orders of magnitude above any steal latency.
//!
//! ## Fan-out ceiling, which is NOT a routing property
//!
//! `BatchingStrategy::default()` clamps chunk size UP to
//! `MIN_ARCHETYPE_FOR_PARALLEL` (1024), so an archetype of N rows yields
//! `ceil(N / max(N/W, 1024))` chunks: at N = 4096 that is 4 chunks (ceiling
//! 4x) regardless of worker count, and at N = 65536 it is 16. Both N are
//! reported so the ceiling is visible next to the route comparison rather than
//! confused with it — it caps BOTH routes equally, so it can never be the
//! reason one of them trails the other.
//!
//! Component id 492 is reserved for this bench (470-479 query_iter,
//! 480-489 swap_remove; MAX_COMPONENTS = 512).

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::thread::available_parallelism;
use std::time::{Duration, Instant};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_threadpool::{MAX_WORKERS, ThreadPool, ThreadPoolBuilder, current_worker_id};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

// ── Fixture ─────────────────────────────────────────────────────────────────

const SLOT_KE16_SPIN: ComponentId = ComponentId(492);

#[repr(C)]
#[derive(Clone, Copy)]
struct Ke16Spin(u32);

impl Component for Ke16Spin {
    fn component_id() -> ComponentId {
        SLOT_KE16_SPIN
    }
}

/// Per-row busy-wait budget. See the module header for why it is this large.
const SPIN_PER_ROW: Duration = Duration::from_micros(20);

/// Row counts the harness sweeps. 4096 is the prior protocol's population;
/// 65536 is the first population at which the default batching strategy can
/// reach 16 chunks on a 16-worker machine.
const POPULATIONS: [usize; 2] = [4096, 65536];

// ── Occupancy instrument ────────────────────────────────────────────────────

/// Bodies currently executing.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
/// High-water mark of `IN_FLIGHT` over the pass. This is the occupancy datum.
static MAX_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
/// Rows visited, so a pass that silently matched nothing cannot report a win.
static ROWS_SEEN: AtomicUsize = AtomicUsize::new(0);
/// Consumes the spin's result so the loop cannot be elided even across LTO.
static SINK: AtomicU64 = AtomicU64::new(0);
/// Highest worker id the SYSTEM route's body ever ran on — half of the route receipt.
///
/// A `fetch_max` rather than a plain store: the scheduler is free to place the system on any
/// worker, so the datum that matters is not "which one" but "was it ever off-pool". The sentinels
/// (`WORKER_ID_DISPATCHER = u32::MAX - 1`, `WORKER_ID_UNATTACHED = u32::MAX`) are the two largest
/// `u32`s, so a max is exactly the test for "some run was not on a registered worker".
static SYSTEM_WORKER_ID: AtomicU32 = AtomicU32::new(0);
/// Runs of the SYSTEM route's body — the OTHER half of the route receipt.
///
/// Worker 0 is a legal id, so the id alone cannot separate "ran on worker 0" from "never ran".
/// It does NOT detect a filtered-out row: the receipt is evaluated inside the row's own routine,
/// and criterion never invokes a routine whose id the filter rejected (nor under `--list`), so a
/// filtered row yields no reading and takes no receipt. What the count separates is the shape
/// where the row WAS measured and the system body still never ran — a schedule built without the
/// system, or a `run_if` that skipped it — all of which leave `SYSTEM_WORKER_ID` at the legal id
/// 0. The "never ran" sentinel cannot live in `SYSTEM_WORKER_ID` itself: every value outside
/// `[0, MAX_WORKERS)` is ABOVE the legal ids and would survive the `fetch_max` fold the off-pool
/// half needs, so existence is counted apart.
static SYSTEM_BODY_RUNS: AtomicUsize = AtomicUsize::new(0);

fn reset_instruments() {
    IN_FLIGHT.store(0, Ordering::SeqCst);
    MAX_IN_FLIGHT.store(0, Ordering::SeqCst);
    ROWS_SEEN.store(0, Ordering::SeqCst);
    reset_route_receipt();
}

/// Clears both halves of the route receipt.
///
/// Separate from [`reset_instruments`] because the criterion path clears the receipt inside the
/// `par_in_system` routine, around a whole `b.iter` batch, while the occupancy counters are
/// cleared per protocol pass.
fn reset_route_receipt() {
    SYSTEM_WORKER_ID.store(0, Ordering::SeqCst);
    SYSTEM_BODY_RUNS.store(0, Ordering::SeqCst);
}

/// Panics unless every recorded system body ran on a registered worker.
///
/// The whole point of the `par_in_system` route is that its `par_iter` is pushed FROM a worker.
/// If the scheduler ever ran the body on the dispatcher instead, the chunks would reach
/// `injector_global` — which is the OTHER route of this bench, so the row would be that route
/// measured twice and reported under two names (`KE16-DESIGN-MEASUREMENT.md` §5 shape 2,
/// "healthy route twice").
fn assert_system_route_receipt() {
    let runs = SYSTEM_BODY_RUNS.load(Ordering::SeqCst);
    assert!(
        runs > 0,
        "the `par_in_system` route was measured but recorded NO system body at all: the schedule \
         never dispatched the system, so the number this row produced is not a measurement of \
         this route"
    );
    let observed = SYSTEM_WORKER_ID.load(Ordering::SeqCst);
    assert!(
        (observed as usize) < MAX_WORKERS,
        "the `par_in_system` route recorded worker id {observed} over {runs} runs (>= \
         MAX_WORKERS = {MAX_WORKERS}): a system body ran off-pool, so its `par_iter` reached \
         injector_global and this row is the healthy route measured twice"
    );
}

/// The measured body: spin `SPIN_PER_ROW`, bracketed by the in-flight counter.
///
/// `#[inline(never)]` so all three routes execute byte-identical work — an
/// inlined copy specialised per call site would put the routes on different
/// code and make the ratio a property of codegen.
#[inline(never)]
fn spin_body(v: &Ke16Spin) {
    let now_in_flight = IN_FLIGHT.fetch_add(1, Ordering::AcqRel) + 1;
    MAX_IN_FLIGHT.fetch_max(now_in_flight, Ordering::AcqRel);
    ROWS_SEEN.fetch_add(1, Ordering::Relaxed);

    let deadline = Instant::now() + SPIN_PER_ROW;
    // A cheap dependent chain, so the spin is a busy-wait and not a
    // memory-bound loop that would perturb the cache behaviour under test.
    let mut acc = v.0 as u64;
    while Instant::now() < deadline {
        acc = black_box(acc.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1));
    }
    SINK.fetch_add(acc & 1, Ordering::Relaxed);

    IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
}

// ── World construction ──────────────────────────────────────────────────────

/// One archetype, `n` rows, one component — the best case for `par_iter`
/// (§2.4.4: the unit of parallelism is ONE archetype, so a fragmented world
/// would confound the difference between the routes with the granularity
/// constant).
fn build_world(n: usize) -> EcsMaster {
    register_layout::<Ke16Spin>(SLOT_KE16_SPIN.0);
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[SLOT_KE16_SPIN]);
    for i in 0..n {
        world
            .spawn_one(arch, Ke16Spin(i as u32))
            .expect("invariant: bench fixture seed must succeed");
    }
    world
}

// ── The three routes ────────────────────────────────────────────────────────

/// Route SEQ — sequential `Query::iter` on the calling thread, no pool frame.
/// The denominator of both speedups.
fn route_seq(world: &mut EcsMaster) {
    world.run_closure_once(|q: Query<&Ke16Spin>| {
        for v in q.iter() {
            spin_body(v);
        }
    });
}

/// Route DISPATCHER — the same `par_iter`, driven from the bench thread inside
/// `pool.install`. The bench thread is not a registered worker, so
/// `push_task` routes every chunk to `injector_global`: the OFF-POOL route,
/// which every worker polls directly.
fn route_par_from_dispatcher(world: &mut EcsMaster, pool: &Arc<ThreadPool>) {
    pool.install(|_scope| {
        world.run_closure_once(|q: Query<&Ke16Spin>| {
            q.par_iter().for_each(spin_body);
        });
    });
}

/// Route SYSTEM — the same `par_iter`, inside a system body scheduled by the
/// parallel scheduler. The body executes ON A WORKER, so `push_task` routes
/// every chunk to that worker's own registered deque: the WORKER-SPAWN route,
/// which siblings reach by stealing rather than by polling.
fn route_par_in_system(world: &mut EcsMaster, schedule: &mut Schedule) {
    schedule.run(world);
}

fn build_schedule(world: &mut EcsMaster, pool: &Arc<ThreadPool>) -> Schedule {
    let mut builder = ScheduleBuilder::new(Arc::clone(pool));
    builder.add_system(|q: Query<&Ke16Spin>| {
        // The route receipt, taken BEFORE the wave so it is recorded even if the wave panics.
        SYSTEM_WORKER_ID.fetch_max(current_worker_id(), Ordering::AcqRel);
        SYSTEM_BODY_RUNS.fetch_add(1, Ordering::AcqRel);
        q.par_iter().for_each(spin_body);
    });
    builder.build(world)
}

// ── The protocol pass ───────────────────────────────────────────────────────

/// One timed pass plus its occupancy reading.
struct Pass {
    wall: Duration,
    max_in_flight: usize,
    rows: usize,
}

fn timed<F: FnOnce()>(f: F) -> Pass {
    reset_instruments();
    let t0 = Instant::now();
    f();
    let wall = t0.elapsed();
    Pass {
        wall,
        max_in_flight: MAX_IN_FLIGHT.load(Ordering::SeqCst),
        rows: ROWS_SEEN.load(Ordering::SeqCst),
    }
}

/// Prints the headline table the campaign reads: wall-clock, occupancy and the
/// two speedups, once per population. Runs BEFORE criterion so the numbers are
/// a single clean pass rather than a statistic over a warmed cache.
///
/// Best-of-`REPS` on wall-clock: the machine is shared with a concurrent build,
/// and a min over a few passes rejects a scheduler hiccup without pretending to
/// be a distribution.
fn protocol_pass(workers: usize) {
    const REPS: usize = 3;

    println!("\n=== KE16 protocol pass (release={}) ===", !cfg!(debug_assertions));
    println!(
        "workers={workers}  available_parallelism={}  spin_per_row={:?}",
        available_parallelism().map(|n| n.get()).unwrap_or(0),
        SPIN_PER_ROW
    );

    for &n in POPULATIONS.iter() {
        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        let mut world = build_world(n);
        let mut schedule = build_schedule(&mut world, &pool);

        let mut best_seq = Pass { wall: Duration::MAX, max_in_flight: 0, rows: 0 };
        let mut best_disp = Pass { wall: Duration::MAX, max_in_flight: 0, rows: 0 };
        let mut best_sys = Pass { wall: Duration::MAX, max_in_flight: 0, rows: 0 };

        for _ in 0..REPS {
            let p = timed(|| route_seq(&mut world));
            if p.wall < best_seq.wall {
                best_seq = p;
            }
            let p = timed(|| route_par_from_dispatcher(&mut world, &pool));
            if p.wall < best_disp.wall {
                best_disp = p;
            }
            let p = timed(|| route_par_in_system(&mut world, &mut schedule));
            if p.wall < best_sys.wall {
                best_sys = p;
            }
        }

        // Anti-vacuity: a route that visited the wrong number of rows is not a
        // measurement of anything, and its speedup would be a pure artefact.
        for (label, p) in [
            ("seq", &best_seq),
            ("par_from_dispatcher", &best_disp),
            ("par_in_system", &best_sys),
        ] {
            assert_eq!(
                p.rows, n,
                "route `{label}` visited {} of {n} rows — the fixture, not the pool, is broken",
                p.rows
            );
        }
        assert_system_route_receipt();

        let seq_s = best_seq.wall.as_secs_f64();
        println!(
            "\n-- N={n} rows, one archetype, {:.0} us/row --",
            SPIN_PER_ROW.as_secs_f64() * 1e6
        );
        println!(
            "  {:<22} {:>10.2} ms   max_in_flight={:<3}  speedup={:>6.2}x",
            "seq (baseline)",
            seq_s * 1e3,
            best_seq.max_in_flight,
            1.0
        );
        println!(
            "  {:<22} {:>10.2} ms   max_in_flight={:<3}  speedup={:>6.2}x",
            "par_from_dispatcher",
            best_disp.wall.as_secs_f64() * 1e3,
            best_disp.max_in_flight,
            seq_s / best_disp.wall.as_secs_f64()
        );
        println!(
            "  {:<22} {:>10.2} ms   max_in_flight={:<3}  speedup={:>6.2}x",
            "par_in_system",
            best_sys.wall.as_secs_f64() * 1e3,
            best_sys.max_in_flight,
            seq_s / best_sys.wall.as_secs_f64()
        );
        println!(
            // The label used to read `dispatcher/system` while the expression below
            // computes in_system / from_dispatcher. Anyone who trusted the label and
            // inverted the value got the reciprocal of the receipt.
            "  RATIO in_system/from_dispatcher wall = {:.2}x  (occupancy: dispatcher {} vs in_system {})",
            best_sys.wall.as_secs_f64() / best_disp.wall.as_secs_f64(),
            best_disp.max_in_flight,
            best_sys.max_in_flight
        );
    }
    println!();
}

// ── Criterion groups ────────────────────────────────────────────────────────

fn bench_ke16_par_iter_in_system(c: &mut Criterion) {
    let workers = available_parallelism().map(|n| n.get()).unwrap_or(4);
    protocol_pass(workers);

    let mut group = c.benchmark_group("ke16_par_iter_in_system");
    // A single pass costs N x 20 us of pure spin (1.3 s at N = 65536 on the
    // sequential baseline), so the default 100 samples would run for minutes.
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(5));

    for &n in POPULATIONS.iter() {
        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        let mut world = build_world(n);
        let mut schedule = build_schedule(&mut world, &pool);

        group.bench_with_input(BenchmarkId::new("seq", n), &n, |b, _| {
            b.iter(|| route_seq(black_box(&mut world)));
        });
        group.bench_with_input(BenchmarkId::new("par_from_dispatcher", n), &n, |b, _| {
            b.iter(|| route_par_from_dispatcher(black_box(&mut world), &pool));
        });
        group.bench_with_input(BenchmarkId::new("par_in_system", n), &n, |b, _| {
            // Both halves live INSIDE the routine: criterion does not invoke a routine the
            // filter rejected (nor under `--list`), so a row that produced no reading takes no
            // receipt, and a row that did produce one is judged on its own runs only.
            reset_route_receipt();
            b.iter(|| route_par_in_system(black_box(&mut world), &mut schedule));
            // Once per `b.iter` batch, not per iteration: the receipt is a property of the
            // route, and reading an atomic inside the timed loop would price the instrument
            // into the number.
            assert_system_route_receipt();
        });
    }
    group.finish();
}

criterion_group!(benches, bench_ke16_par_iter_in_system);
criterion_main!(benches);
