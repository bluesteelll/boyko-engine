//! KE16 — the wall-clock harness the candidate fixes for defect A were compared on, kept as the
//! shipped configuration's route receipt.
//!
//! Two dispatch routes, one grid, stable ids:
//!
//! - **`dispatcher/…`** — the wave is spawned from a non-worker thread, so `tls::worker_lane_for`
//!   answers `None` and `place_task` sends every task to `injector_global`, which every worker
//!   drains. It is the *ceiling* the nested route is measured against.
//! - **`worker/…`** — the wave is spawned from inside a task that is itself running on a worker,
//!   through `try_with_active_pool` + `PoolInner::scope`. This is verbatim the shape of every ECS
//!   system body, and of all four parallel physics sites. It lands on that worker's OWN registered
//!   deque, whose stealer every sibling scans, so the wave is reachable by the whole pool — the
//!   placement that closed defect A.
//!
//! Grid: body ∈ {1 us, 10 us, 100 us, 1 ms} × tasks ∈ {W, 4W, 64W}, where `W` is
//! `available_parallelism`. The body sweep is the load-bearing axis: a fix that adds
//! synchronisation to reach more cores pays that cost per task, so it can win at 1 ms and lose at
//! 1 us. **The acceptance criterion is throughput, not occupancy** — a variant that occupies more
//! cores and finishes slower loses, and this grid is where that shows.
//!
//! ## What is inside and outside the timed region
//!
//! Outside: pool construction (one pool for the whole run), the `Duration` grid, criterion's own
//! bookkeeping. Inside: exactly one wave — the spawn loop, the fan-out, the bodies, and the join.
//! The `worker` route additionally pays, inside the timed region, one `ThreadPool::spawn` of the
//! outer task and its `park`/`unpark` handshake (~a few us); that cost is paid by the `worker` row
//! and not by the `dispatcher` ceiling, so a route-to-route reading owes it. It also means the
//! `worker` row of the `1us x W` cell is dominated by it and should not be read as a fan-out
//! number.
//!
//! Bodies **spin** on `Instant::elapsed`, never sleep: a sleeping body would let a serialised wave
//! look distributed at the wall clock.
//!
//! ## The `ke16_park_timeout` group (App-7)
//!
//! `Scope`'s pre-park backstop asks for 50 us and `Schedule`'s for 100 us, but both reach
//! `WaitOnAddress` through `dur2timeout`, which rounds nanoseconds UP to whole milliseconds. What
//! the expiry then IS depends on the system timer resolution in effect — process-wide state that
//! this crate does not own. Since App-12 the shipped host holds it at 1 ms for the whole run
//! (`boyko_app::timer_resolution::TimerResolutionGuard`, bound in the host runner closure), and
//! App-12 measured this box at **1 021 us guarded against 15 296 us unguarded** — a 15x spread on
//! the identical call.
//!
//! **This bench binary does not link `boyko_app`, so it holds no guard of its own**: its rows are
//! UNGUARDED unless some other process on the box happens to be holding a fine period. That is a
//! 15x ambiguity in a number the campaign uses as the backstop truth for every latency statement,
//! so the group PROBES the resolution at entry (one 50 us park, the same primitive the rows
//! measure) and prints `KE16 park_timeout configuration=<guarded|unguarded>` with the probe value.
//! **The tester copies that line into `KE16-RESULTS.md` beside the three medians**; a row recorded
//! without its configuration is not a datum. The three rows ask for 50 us, 1 ms and 2 ms with no
//! unpark pending; criterion's median IS the expiry latency, and three rows rather than one make
//! the quantum visible instead of inferred.
//!
//! ## Run
//!
//! ```text
//! cargo bench -p boyko-threadpool --bench ke16_nested_scope
//! cargo bench -p boyko-threadpool --bench ke16_nested_scope -- "body_100us_tasks_4W"
//! cargo bench -p boyko-threadpool --bench ke16_nested_scope -- "ke16_park_timeout"
//! ```

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use boyko_threadpool::{
    MAX_WORKERS, Scope, ThreadPool, ThreadPoolBuilder, current_worker_id, try_with_active_pool,
};

/// Body durations, with the label that goes into the benchmark id. The labels are spelled out
/// rather than derived so that a later edit to the grid cannot silently rename a row and orphan
/// its criterion baseline.
const BODIES: [(&str, Duration); 4] = [
    ("1us", Duration::from_micros(1)),
    ("10us", Duration::from_micros(10)),
    ("100us", Duration::from_micros(100)),
    ("1ms", Duration::from_millis(1)),
];

/// Wave sizes as multiples of the worker count, with their id labels. Multiples rather than
/// absolute counts, so the same id means the same *shape* on a 4-core and a 64-core box.
const TASK_MULTIPLES: [(&str, usize); 3] = [("W", 1), ("4W", 4), ("64W", 64)];

/// The requested `park_timeout` durations of the App-7 group, with their row labels.
const PARK_TIMEOUTS: [(&str, Duration); 3] = [
    ("park_timeout_50us", Duration::from_micros(50)),
    ("park_timeout_1ms", Duration::from_millis(1)),
    ("park_timeout_2ms", Duration::from_millis(2)),
];

/// Worker id observed inside the LAST outer task of the `worker` route.
///
/// Written by the outer task, read by `worker_wave` after the join has established
/// happens-before, so a `Relaxed` store would also be sound; `Release`/`Acquire` costs nothing
/// here (the wave is microseconds wide) and keeps the receipt readable without the join argument.
static OUTER_WORKER_ID: AtomicU32 = AtomicU32::new(u32::MAX);

/// The width the grid is parameterised by: the pool's worker count, the `tasks` axis multiplier,
/// and the number the receipt prints.
///
/// ONE function for all three, so a reading cannot be printed against a width the grid did not run
/// at. The `4` fallback is the grid's: `available_parallelism` failing is not a reason to build a
/// 0-worker pool.
fn grid_width() -> usize {
    std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get)
}

/// Prints the width the grid is parameterised by.
///
/// "The two baselines were taken at different W" is a false-green shape this campaign has already
/// met; this print is what makes it visible in the bench's own output rather than inferred from the
/// box a run happened on.
fn print_grid_width() {
    println!("KE16 available_parallelism={}", grid_width());
}

/// Busy-wait for `d`.
fn spin(d: Duration) {
    let started = Instant::now();
    let mut acc = 0u64;
    while started.elapsed() < d {
        acc = acc.wrapping_add(black_box(1));
        std::hint::spin_loop();
    }
    black_box(acc);
}

/// How a wave is handed to the pool.
type Spawner = fn(&Scope<'_>, usize, Duration);

/// One `spawn` per task, so one `pending` RMW and one wake decision each.
fn spawn_per_task(scope: &Scope<'_>, tasks: usize, body: Duration) {
    for _ in 0..tasks {
        scope.spawn(move || spin(body));
    }
}

/// One wave pushed from a non-worker thread, joined by `Scope::drop`.
fn dispatcher_wave(pool: &ThreadPool, tasks: usize, body: Duration, spawn: Spawner) {
    pool.install(|scope| {
        spawn(scope, tasks, body);
    });
}

/// One wave pushed from inside a worker task, joined by the nested `Scope::drop` on that worker.
///
/// The outer task goes through `ThreadPool::spawn` rather than through a scope on purpose: a scope
/// is drained inline by its joining thread, so which thread runs a single scope-spawned task is a
/// race, and on the runs where it stays on the dispatcher the nested scope pushes to
/// `injector_global` and this benchmark would silently measure the `dispatcher` route twice.
/// MEASURED at this checkout — both outcomes occurred on consecutive runs of the sibling test.
fn worker_wave(pool: &ThreadPool, tasks: usize, body: Duration, spawn: Spawner) {
    let done = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&done);
    let waiter = std::thread::current();

    pool.spawn(move || {
        OUTER_WORKER_ID.store(current_worker_id(), Ordering::Release);
        let dispatched = try_with_active_pool(|inner| {
            inner.scope(|nested| {
                spawn(nested, tasks, body);
            });
        });
        assert!(
            dispatched.is_some(),
            "the outer task saw no active pool; the nested route was never exercised"
        );
        signal.store(true, Ordering::Release);
        waiter.unpark();
    });

    // Park rather than spin: the calling thread must not hold a core that the wave under
    // measurement could otherwise be using.
    while !done.load(Ordering::Acquire) {
        std::thread::park_timeout(Duration::from_millis(1));
    }

    // The route receipt. `ThreadPool::spawn` has no join loop on the calling thread, so only a
    // worker CAN run the outer task -- but a receipt that is only argued is not a receipt, and
    // the sentinel ids (DISPATCHER = u32::MAX - 1, UNATTACHED = u32::MAX) are exactly what a
    // future refactor would leave here if the outer task ever ran off-pool. A nested scope opened
    // from a non-worker pushes to `injector_global` and fans out perfectly, so that run would
    // measure the `dispatcher` route twice and report it as the nested one.
    let outer = OUTER_WORKER_ID.load(Ordering::Acquire);
    assert!(
        (outer as usize) < MAX_WORKERS,
        "outer_worker_id={outer}: the outer task ran on the dispatcher; healthy route measured twice"
    );
}

/// Above this the box is on its default quantum; below it a fine (~1 ms) timer period is in
/// effect. App-12 measured the two populations on this box as 1 021 us and 15 296 us, so any
/// threshold between them classifies correctly; 4 ms sits far from both and also catches a 2 ms
/// period requested by a third party without calling it "the default quantum".
const FINE_PERIOD_PROBE_CEILING: Duration = Duration::from_millis(4);

/// Probes, and prints, which timer-resolution configuration the App-7 rows are being taken in.
///
/// The probe is one `park_timeout(50 us)` with no token pending — the same primitive the rows
/// measure, so it cannot disagree with them the way a `NtQueryTimerResolution` reading could. The
/// caller must have drained any pending unpark token first, or the probe returns instantly and
/// reports a fine period that is not there.
fn report_park_timeout_configuration() {
    let started = Instant::now();
    std::thread::park_timeout(Duration::from_micros(50));
    let probe = started.elapsed();

    // This binary holds no `TimerResolutionGuard` (it does not link `boyko_app`), so a fine
    // reading here can only be another process's request — which is exactly why the row must
    // carry the configuration instead of the reader assuming one.
    let configuration = if probe < FINE_PERIOD_PROBE_CEILING {
        "guarded"
    } else {
        "unguarded"
    };
    println!(
        "KE16 park_timeout configuration={configuration} (probe: a 50 us park expired after {} us; \
         App-12 reference: 1021 us guarded / 15296 us unguarded). Record this line beside the \
         three medians.",
        probe.as_micros()
    );
}

/// App-7 -- the real expiry of a `park_timeout` on this box, with no unpark pending.
///
/// Each row parks a thread that nothing will wake, so the iteration time IS the timed wait's
/// expiry. The bench thread is the parker: it holds no pool lane, and criterion runs the rows
/// serially, so nothing else is contending for the timer.
///
/// The medians are meaningless without the timer-resolution configuration they were taken in
/// (App-12: 15x between the two), so the group prints it before the first row.
fn bench_park_timeout(c: &mut Criterion) {
    // Drain a token the `worker` route may have left on this thread: `worker_wave` unparks the
    // bench thread once per wave and exits its loop as soon as `done` is visible, so the last
    // token can outlive the wave. A pending token makes `park_timeout` return at once, and one
    // such sample would drag the median of the 50 us row toward zero -- the exact reading this
    // group exists to establish. `Duration::ZERO` consumes the token without ever waiting.
    std::thread::park_timeout(Duration::ZERO);

    report_park_timeout_configuration();

    let mut group = c.benchmark_group("ke16_park_timeout");
    // Every sample costs at least the requested duration and, on Windows, at least one timer
    // quantum; 20 samples over 2 s is enough to read a median that is quantised to whole
    // milliseconds and keeps the group under a minute.
    group.sample_size(20);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    for (label, d) in PARK_TIMEOUTS {
        group.bench_function(label, |b| {
            b.iter(|| std::thread::park_timeout(black_box(d)));
        });
    }

    group.finish();
}

fn bench_nested_scope(c: &mut Criterion) {
    print_grid_width();
    let workers = grid_width();
    // One pool for the whole run. Construction spawns W OS threads and is emphatically not part of
    // what this file measures.
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();

    let mut group = c.benchmark_group("ke16_nested_scope");
    // The wave is milliseconds wide in the larger cells, so criterion's default 100 samples x 5 s
    // would put the full grid in the tens of minutes. Ten samples is the floor criterion accepts
    // and is enough to separate 1x from 8x, which is the size of the effect under study.
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(300));

    for (body_label, body) in BODIES {
        for (mult_label, mult) in TASK_MULTIPLES {
            let tasks = workers * mult;
            let param = format!("body_{body_label}_tasks_{mult_label}");

            // Budget from the serial floor, so a cell that is genuinely serialised still completes
            // its samples instead of emitting a "unable to complete N samples" warning that reads
            // like a harness fault.
            let serial_floor = body.mul_f64(tasks as f64);
            let budget = (serial_floor * 12).clamp(Duration::from_secs(1), Duration::from_secs(20));
            group.measurement_time(budget);

            group.bench_function(BenchmarkId::new("dispatcher", &param), |b| {
                b.iter(|| dispatcher_wave(&pool, tasks, body, spawn_per_task));
            });
            group.bench_function(BenchmarkId::new("worker", &param), |b| {
                b.iter(|| worker_wave(&pool, tasks, body, spawn_per_task));
            });
        }
    }

    group.finish();
}

criterion_group!(benches, bench_nested_scope, bench_park_timeout);
criterion_main!(benches);
