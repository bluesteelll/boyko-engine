//! KE16 — the pool-level occupancy harness for **nested** (worker-spawned) scopes.
//!
//! ## What this file measures and why it is at the pool level
//!
//! The 2026-08-30 census measured `par_iter` inside a scheduled system at **1.01x** against
//! **7.69x** for the identical driver called from the dispatcher thread, and localized the cause
//! to the pool, not to the query drivers: `push_task` (`src/worker.rs`) routed a task spawned by a
//! worker of *this* pool into `injector_local[wid]`, and no thread ever polled another thread's
//! local injector — sibling stealing walks `inner.stealers`, which holds worker **deques** only.
//! Work spawned from inside a system body was therefore reachable by its own worker alone
//! (**defect A**), and `Scope::drop` additionally batch-stole about half the wave into a private,
//! unregistered FIFO `scratch` deque and ran it inline (**defect B**). This file is the instrument
//! that was built to see both, and it stays as the standing gate over the placement that ships.
//!
//! Every ECS system body executes on a worker, so every `par_iter` in every system was on that
//! path. Measuring it through `boyko_ecs` would confound the pool with the query driver's own
//! chunking constant (`MIN_ARCHETYPE_FOR_PARALLEL = 1024`, which caps fan-out at `ceil(N/1024)`
//! independently of the pool). **This harness therefore drives the pool directly**: two dispatch
//! routes, identical waves, identical bodies. The only difference between them is *who* pushed the
//! tasks.
//!
//! ## The instrument
//!
//! Counting distinct thread ids is **not** a valid instrument here — both censuses hit that
//! independently. A joining thread drains its own queue inline, so a correctly fanned-out wave
//! with a cheap body still reports one thread. Two instruments that fail independently are used
//! instead:
//!
//! - **max-in-flight**: `fetch_add` on body entry, `fetch_max` of the running value into a
//!   separate cell, `fetch_sub` on exit. This observes *simultaneity*, not identity, and it is
//!   independent of wall-clock.
//! - **wall-clock against the serial floor** (`tasks x body`). A wave that is serialised cannot
//!   beat the floor no matter which threads ran it.
//! - **per-lane work accounting** (`lanes_used` / `top_lane` / `off_pool`): how many bodies each
//!   lane actually completed. This is the instrument that separates the two defects, and it is
//!   why the first two are not enough on their own — the dispatcher route peaks at 16 in flight
//!   and still spends most of its wall-clock on one lane.
//!
//! Bodies **spin** on `Instant::elapsed` rather than sleeping, so a serialised wave shows up as
//! wall-clock rather than as idle time, and a sleeping body cannot fake occupancy by overlapping
//! at the scheduler.
//!
//! ## Run
//!
//! ```text
//! cargo test -p boyko-threadpool --test ke16_nested_scope_occupancy -- --nocapture --test-threads=1
//! ```
//!
//! That single invocation runs every test in this file, the red-first gate
//! ([`worker_spawned_wave_reaches_at_least_half_the_workers`]) included: nothing here is
//! `#[ignore]`d, so no test in this file can pass by never having run.

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::{
    MAX_WORKERS, ThreadPool, ThreadPoolBuilder, WORKER_ID_DISPATCHER, WORKER_ID_UNATTACHED,
    current_worker_id, try_with_active_pool,
};

/// Per-task body duration. 200 us is far above the ~100 ns Windows `Instant` granule and far
/// above the ~1 us push/steal cost, so neither the clock nor the dispatch overhead is in the
/// signal.
const BODY: Duration = Duration::from_micros(200);

/// Wave size as a multiple of the worker count. Four tasks per worker gives the pool room to
/// distribute even if it first hands one task to each worker and only then steals.
const TASKS_PER_WORKER: usize = 4;

/// Repetitions per (route, width) cell in the recording test. Three shows the run-to-run spread of
/// a cell without turning a 40 ms test into a benchmark; the bench file is where distributions are
/// measured properly.
const REPEATS: usize = 3;

// ---------------------------------------------------------------------------
// Instrument
// ---------------------------------------------------------------------------

/// Occupancy counters shared by every task of one wave.
///
/// Every cell is an atomic, so the struct is `Sync` and can be borrowed by every task body without
/// a lock — a lock would itself serialise the wave being measured.
struct Occupancy {
    /// Bodies currently between entry and exit.
    in_flight: AtomicUsize,
    /// Running maximum of `in_flight`. This is the whole point of the file.
    max_in_flight: AtomicUsize,
    /// Bodies that reached exit. Guards against a "fast" result produced by tasks that never ran.
    ran: AtomicUsize,
    /// Bodies per worker lane. This is *work accounting*, not thread identity: a lane that ran
    /// half the wave held half the wall-clock, which a distinct-thread count cannot express and
    /// which max-in-flight alone does not either (a wave can peak at W and still spend most of
    /// its wall-clock on one lane — the measured signature of defect B).
    lanes: [AtomicUsize; MAX_WORKERS],
    /// Bodies that ran on a thread that is not a registered worker of the pool, i.e. on the
    /// joining/dispatcher thread that drained the scope inline.
    off_pool: AtomicUsize,
}

impl Occupancy {
    fn new() -> Self {
        Self {
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            ran: AtomicUsize::new(0),
            lanes: std::array::from_fn(|_| AtomicUsize::new(0)),
            off_pool: AtomicUsize::new(0),
        }
    }

    fn enter(&self) {
        let now = self.in_flight.fetch_add(1, Ordering::AcqRel) + 1;
        self.max_in_flight.fetch_max(now, Ordering::AcqRel);
    }

    fn exit(&self) {
        // The lane is read at exit, not entry, so it names the thread that actually executed the
        // body to completion.
        let wid = current_worker_id() as usize;
        if wid < MAX_WORKERS {
            self.lanes[wid].fetch_add(1, Ordering::AcqRel);
        } else {
            self.off_pool.fetch_add(1, Ordering::AcqRel);
        }
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
        self.ran.fetch_add(1, Ordering::AcqRel);
    }

    fn max(&self) -> usize {
        self.max_in_flight.load(Ordering::Acquire)
    }

    fn ran(&self) -> usize {
        self.ran.load(Ordering::Acquire)
    }

    /// `(lanes that ran at least one body, bodies on the busiest lane, bodies off-pool)`.
    fn distribution(&self) -> (usize, usize, usize) {
        let off_pool = self.off_pool.load(Ordering::Acquire);
        let mut used = usize::from(off_pool > 0);
        let mut top = off_pool;
        for lane in &self.lanes {
            let n = lane.load(Ordering::Acquire);
            if n > 0 {
                used += 1;
                top = top.max(n);
            }
        }
        (used, top, off_pool)
    }
}

/// Busy-wait for `d`. Spin, never sleep: a sleeping body yields its core, so a serialised wave
/// would be indistinguishable from a distributed one at the wall clock.
fn spin(d: Duration) {
    let started = Instant::now();
    let mut acc = 0u64;
    while started.elapsed() < d {
        acc = acc.wrapping_add(black_box(1));
        std::hint::spin_loop();
    }
    black_box(acc);
}

/// One wave's result.
struct Wave {
    /// Peak number of bodies simultaneously between entry and exit.
    max_in_flight: usize,
    /// Bodies that completed. Compared against the wave size by every test.
    ran: usize,
    /// Wall-clock of the wave itself, measured on the thread that opened the scope — the test
    /// thread for the dispatcher route, the worker for the nested route. Pool construction and
    /// the queue latency of the outer task are outside it in both cases.
    wall: Duration,
    /// Worker id observed *inside* the outer task (`u32::MAX` when the route has no outer task).
    /// Reconciles the two contradictory census measurements of the nested shape (matrix U1).
    outer_worker_id: u32,
    /// Whether the pool attached to the outer task is pointer-identical to the pool that was
    /// installed. The nested wave is opened on the *active* pool, so a `false` here would mean
    /// the wave was never spawned into the pool under test and the nested route was never
    /// exercised at all (matrix U1).
    outer_same_pool: bool,
    /// Lanes that executed at least one body of the wave.
    lanes_used: usize,
    /// Bodies executed by the busiest single lane.
    top_lane: usize,
    /// Bodies executed off-pool, i.e. inline on the joining thread.
    off_pool: usize,
}

impl Wave {
    /// Serial floor for `tasks` bodies of `BODY`. A wave cannot beat this unless it overlapped.
    fn speedup(&self, tasks: usize) -> f64 {
        let floor = BODY.as_secs_f64() * tasks as f64;
        floor / self.wall.as_secs_f64()
    }
}

/// Address of the pool's `PoolInner`, taken from inside an `install` frame on the calling thread.
///
/// Only an integer crosses the thread boundary — the pointer is never dereferenced here; it is a
/// pool identity token for the same `ptr::eq(pool, inner)` comparison `tls::worker_lane_for`
/// itself performs.
fn pool_addr(pool: &ThreadPool) -> usize {
    pool.install(|_| ThreadPool::current_pool().map_or(0, |p| p.as_ptr() as usize))
}

// ---------------------------------------------------------------------------
// The two routes
// ---------------------------------------------------------------------------

/// **(a) Dispatcher path** — the healthy route. The test thread is not a worker of this pool, so
/// `tls::worker_lane_for` answers `None` and `place_task` sends every task to `injector_global`,
/// which every worker drains.
fn dispatcher_path(pool: &ThreadPool, tasks: usize) -> Wave {
    let occ = Arc::new(Occupancy::new());
    let started = Instant::now();
    pool.install(|scope| {
        for _ in 0..tasks {
            let occ = Arc::clone(&occ);
            scope.spawn(move || {
                occ.enter();
                spin(BODY);
                occ.exit();
            });
        }
    });
    let wall = started.elapsed();
    let (lanes_used, top_lane, off_pool) = occ.distribution();
    Wave {
        max_in_flight: occ.max(),
        ran: occ.ran(),
        wall,
        outer_worker_id: u32::MAX,
        outer_same_pool: false,
        lanes_used,
        top_lane,
        off_pool,
    }
}

/// **(b) Worker path** — the shape every ECS system body has. The whole wave is spawned from
/// *inside* a task that is itself running on a worker, through `try_with_active_pool` — which is
/// verbatim what `crates/boyko_physics/src/resources.rs` and both `colored.rs` solvers do.
///
/// ## Why the outer task is dispatched with `ThreadPool::spawn` and not with a scope
///
/// MEASURED at this checkout, and it is the reason the two censuses disagreed (matrix U1): if the
/// outer task is spawned into a `Scope` from the test thread, **which thread runs it is a race**.
/// The joining thread drains the scope inline, so on some runs the single outer task never leaves
/// the dispatcher — and a nested scope opened from the *dispatcher* pushes to `injector_global`
/// and fans out perfectly. That run measures the healthy route twice while looking like the
/// nested one. Both outcomes were observed on consecutive runs of this very file
/// (`outer_worker_id=WORKER_ID_DISPATCHER, max_in_flight=16` versus
/// `outer_worker_id=14, max_in_flight=1`).
///
/// `ThreadPool::spawn` has no join loop on the calling thread, so the task can only be run by a
/// worker. The test thread then spins on the completion flag. `outer_worker_id` in the result is
/// the receipt: the reader never has to trust that the configuration under test was the intended
/// one.
fn worker_path(pool: &ThreadPool, tasks: usize) -> Wave {
    let occ = Arc::new(Occupancy::new());
    let installed_addr = pool_addr(pool);
    let observed_worker = Arc::new(AtomicUsize::new(WORKER_ID_UNATTACHED as usize));
    let observed_addr = Arc::new(AtomicUsize::new(0));
    let nested_wall_ns = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    // The waiting test thread parks rather than spinning: on a box where W == the hardware
    // parallelism a spinning waiter takes a core away from the very wave being measured, which
    // would bias the fixed route against the fixed pool. The bodies still spin — that is the
    // measurement; this is only the join.
    let waiter = std::thread::current();

    {
        let occ = Arc::clone(&occ);
        let observed_worker = Arc::clone(&observed_worker);
        let observed_addr = Arc::clone(&observed_addr);
        let nested_wall_ns = Arc::clone(&nested_wall_ns);
        let done = Arc::clone(&done);
        pool.spawn(move || {
            observed_worker.store(current_worker_id() as usize, Ordering::Release);
            observed_addr.store(
                ThreadPool::current_pool().map_or(0, |p| p.as_ptr() as usize),
                Ordering::Release,
            );
            let started = Instant::now();
            let dispatched = try_with_active_pool(|inner| {
                inner.scope(|nested| {
                    for _ in 0..tasks {
                        let occ = Arc::clone(&occ);
                        nested.spawn(move || {
                            occ.enter();
                            spin(BODY);
                            occ.exit();
                        });
                    }
                });
            });
            nested_wall_ns.store(
                usize::try_from(started.elapsed().as_nanos()).unwrap_or(usize::MAX),
                Ordering::Release,
            );
            assert!(
                dispatched.is_some(),
                "the outer task saw no active pool; the nested route was never exercised"
            );
            // Released last: it publishes every store above to the waiting test thread.
            done.store(true, Ordering::Release);
            waiter.unpark();
        });
    }

    // Hard cap, so a regression that loses the task fails as a timeout instead of hanging the
    // suite. `park_timeout` may also return spuriously, which the loop condition absorbs.
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "the outer task never completed within 60s; the pool lost a `ThreadPool::spawn` task"
        );
        std::thread::park_timeout(Duration::from_millis(20));
    }

    let outer_worker_id = observed_worker.load(Ordering::Acquire) as u32;
    let outer_addr = observed_addr.load(Ordering::Acquire);
    let (lanes_used, top_lane, off_pool) = occ.distribution();
    Wave {
        max_in_flight: occ.max(),
        ran: occ.ran(),
        wall: Duration::from_nanos(nested_wall_ns.load(Ordering::Acquire) as u64),
        outer_worker_id,
        outer_same_pool: outer_addr != 0 && outer_addr == installed_addr,
        lanes_used,
        top_lane,
        off_pool,
    }
}

// ---------------------------------------------------------------------------
// Widths
// ---------------------------------------------------------------------------

/// Worker counts under test: the machine's own parallelism plus the two fixed widths the KE16
/// brief names, deduplicated so a 16-thread box does not run 16 twice.
fn widths() -> Vec<usize> {
    let mut w = vec![hardware_parallelism(), 4, 16];
    w.sort_unstable();
    w.dedup();
    w
}

fn hardware_parallelism() -> usize {
    std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get)
}

fn print_wave(route: &str, w: usize, tasks: usize, rep: usize, wave: &Wave) {
    println!(
        "[KE16] route={route:<10} W={w:<2} tasks={tasks:<3} rep={rep} body={body_us}us \
         max_in_flight={mif:<2} ran={ran:<3} wall={wall:>9.3}ms \
         speedup_vs_serial_floor={sp:.2}x lanes_used={lanes:<2} top_lane={top:<3} \
         off_pool={off:<3} outer_worker_id={owid} outer_same_pool={osp}",
        body_us = BODY.as_micros(),
        mif = wave.max_in_flight,
        ran = wave.ran,
        wall = wave.wall.as_secs_f64() * 1e3,
        sp = wave.speedup(tasks),
        lanes = wave.lanes_used,
        top = wave.top_lane,
        off = wave.off_pool,
        owid = wave.outer_worker_id,
        osp = wave.outer_same_pool,
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Records both routes at every width and asserts only that every task ran.
///
/// This test must stay green whatever the pool does — it is the number carrier, not the gate. It
/// also carries the healthy-path max-in-flight, which is the direct reading of defect B (the
/// census reported "only 4-5 of 16 tasks simultaneously live" even on the dispatcher route).
#[test]
fn nested_scope_occupancy_numbers_are_recorded_for_both_routes() {
    // Inside the test rather than once per binary: the measurement protocol runs these FILTERED,
    // so a line printed from a harness main would not appear on the invocation whose number is
    // actually recorded.
    println!(
        "[KE16] available_parallelism={} widths={:?} body={}us tasks_per_worker={}",
        hardware_parallelism(),
        widths(),
        BODY.as_micros(),
        TASKS_PER_WORKER
    );
    for w in widths() {
        let pool = ThreadPoolBuilder::new().num_threads(w).build();
        let tasks = TASKS_PER_WORKER * w;

        // Repeated rather than averaged: the dispatcher route was bimodal against the placement
        // this file was written against (how much of the wave the joining thread batch-stole into
        // its private `scratch` varied run to run, and that single quantity moved the wall-clock
        // by 4x), and an average would report a number that neither mode ever produces.
        for rep in 0..REPEATS {
            let a = dispatcher_path(&pool, tasks);
            print_wave("dispatcher", w, tasks, rep, &a);
            assert_eq!(a.ran, tasks, "dispatcher route lost tasks at W={w}");

            let b = worker_path(&pool, tasks);
            print_wave("worker", w, tasks, rep, &b);
            assert_eq!(b.ran, tasks, "worker route lost tasks at W={w}");
        }
    }
}

/// Settles matrix U1: the two censuses disagreed on whether the "nested" shape they measured was
/// really opened from a registered worker of the same pool. If the outer task runs on the calling
/// thread instead, the nested wave goes to `injector_global` and fans out — a healthy number that
/// says nothing about the defect.
///
/// It is the *precondition* of the red-first gate below, and that is why it is a gate in its own
/// right: a gate that reds for the wrong configuration is worth nothing, and a gate that greens
/// because the configuration drifted is worth less than nothing.
#[test]
fn worker_route_outer_task_runs_on_a_registered_worker_of_the_installed_pool() {
    let w = hardware_parallelism();
    let pool = ThreadPoolBuilder::new().num_threads(w).build();
    let tasks = TASKS_PER_WORKER * w;
    let wave = worker_path(&pool, tasks);
    print_wave("worker", w, tasks, 0, &wave);

    assert!(
        wave.outer_worker_id != WORKER_ID_DISPATCHER
            && wave.outer_worker_id != WORKER_ID_UNATTACHED,
        "the outer task did not run on a registered worker (worker_id={}); the nested wave was \
         therefore pushed to injector_global and this harness measures the healthy route twice",
        wave.outer_worker_id
    );
    assert!(
        wave.outer_same_pool,
        "the outer task's active pool is not the installed pool; the nested wave was therefore \
         opened on some other pool and this harness did not measure the pool it built"
    );
}

/// **The red-first gate.** A wave spawned from inside a worker task must reach at least half the
/// workers simultaneously.
///
/// It is an ordinary always-run gate: no `#[ignore]`, no `--ignored` leg, and no build in which it
/// is switched off. That property is the point rather than a detail — the gate was written RED,
/// against a placement it could not pass, and a red-first gate nothing ever runs is the one shape
/// such a gate must never have.
///
/// What it was written against: the wave landed in `injector_local[wid]`, which only worker `wid`
/// polls, and `Scope::drop` then batch-stole about half of it into an unregistered `scratch` and
/// ran it inline on that same worker. The placement that ships — a worker's own spawns go to its
/// own registered deque, where a sibling can steal them — clears the floor with margin.
///
/// Do not weaken the threshold to make it pass — `W/2` is already a floor, not the target (the
/// dispatcher route reaches it on the same fixture).
#[test]
fn worker_spawned_wave_reaches_at_least_half_the_workers() {
    for w in widths() {
        let pool = ThreadPoolBuilder::new().num_threads(w).build();
        let tasks = TASKS_PER_WORKER * w;
        let wave = worker_path(&pool, tasks);
        print_wave("worker", w, tasks, 0, &wave);
        assert_eq!(wave.ran, tasks, "worker route lost tasks at W={w}");
        assert!(
            wave.max_in_flight >= w / 2,
            "W={w}: a wave of {tasks} tasks spawned from inside a worker reached at most \
             {mif} simultaneously live bodies (floor {floor}); wall={wall:.3}ms against a serial \
             floor of {serial:.3}ms (speedup {sp:.2}x). Work spawned from inside worker {owid} is \
             not reaching its siblings — the regression this gate was written against parked it \
             in a queue no sibling polls.",
            mif = wave.max_in_flight,
            floor = w / 2,
            wall = wave.wall.as_secs_f64() * 1e3,
            serial = BODY.as_secs_f64() * tasks as f64 * 1e3,
            sp = wave.speedup(tasks),
            owid = wave.outer_worker_id,
        );
    }
}

/// **The B1-P receipt** (`KE16-DESIGN-B.md` §2.7). A joiner that parked inside its own scope's
/// join is a CLAIMABLE lane: another thread's wave claims its idle bit, wakes it, and the task
/// runs on that joiner INSIDE the join.
///
/// This is a receipt, not a tournament number, and it is deterministic by construction rather than
/// by luck. W = 2:
///
/// 1. The test thread `ThreadPool::spawn`s an OUTER task, so a worker runs it — call it X (the
///    same U1 rule the fixed route above rests on).
/// 2. X opens a scope and spawns ONE body that marks `started`, then spins until `release`. Still
///    inside the scope closure — i.e. before its join can begin — X waits for `started`, so the
///    body is running on the SIBLING Y by the time X joins and X cannot pop it back.
/// 3. X's join therefore finds nothing anywhere (own deque empty, global injector empty, Y's deque
///    empty because Y is inside the body) and parks. Under B1-P it parks IDLE-MARKED, which is the
///    property under test: the test thread spins on [`ThreadPool::parked_mask`] until X's bit is
///    set, bounded at 2 s so a regression that never marks fails instead of hanging.
/// 4. The test thread then spawns a FOREIGN task. Y is busy, so Y's bit is clear and X's is the
///    only claimable one: `push_task` -> `injector_global` -> `claim_one_idle` -> X.
///
/// The receipt is the foreign task's own `current_worker_id()`: it must be X. Note what makes the
/// assertion honest rather than racy — Y cannot take that task at all, because Y only leaves its
/// body when `release` is set and the foreign task is what sets it. So the reading is X or a hang,
/// and the hang is bounded and reported. The whole receipt rests on the joiner parking
/// idle-marked: a joiner that parks without marking is unclaimable, and step 4 would then have no
/// lane to reach at all.
#[test]
fn parked_joiner_is_claimed_by_a_foreign_wave() {
    const WORKERS: usize = 2;
    let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();

    let joiner_id = Arc::new(AtomicUsize::new(WORKER_ID_UNATTACHED as usize));
    let body_id = Arc::new(AtomicUsize::new(WORKER_ID_UNATTACHED as usize));
    let foreign_id = Arc::new(AtomicUsize::new(WORKER_ID_UNATTACHED as usize));
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let outer_done = Arc::new(AtomicBool::new(false));
    let waiter = std::thread::current();

    {
        let joiner_id = Arc::clone(&joiner_id);
        let body_id = Arc::clone(&body_id);
        let started = Arc::clone(&started);
        let release = Arc::clone(&release);
        let outer_done = Arc::clone(&outer_done);
        pool.spawn(move || {
            joiner_id.store(current_worker_id() as usize, Ordering::Release);
            let dispatched = try_with_active_pool(|inner| {
                inner.scope(|nested| {
                    let body_id = Arc::clone(&body_id);
                    let started_in = Arc::clone(&started);
                    let release_in = Arc::clone(&release);
                    nested.spawn(move || {
                        body_id.store(current_worker_id() as usize, Ordering::Release);
                        started_in.store(true, Ordering::Release);
                        // Hold this lane busy until the foreign task releases it: the fixture's
                        // whole content is that the parked joiner is the ONLY claimable lane.
                        // Bounded, so a pool that never runs the foreign task fails the receipt
                        // below instead of wedging the suite.
                        let deadline = Instant::now() + Duration::from_secs(10);
                        while !release_in.load(Ordering::Acquire) && Instant::now() < deadline {
                            std::hint::spin_loop();
                        }
                    });
                    // Still INSIDE the scope closure, i.e. before the join: if the body had not
                    // started, this thread would pop its own task at step 1 of the join and never
                    // park at all. The receipt would then read "the body ran on the joiner", which
                    // is what the assertion below reports.
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while !started.load(Ordering::Acquire) && Instant::now() < deadline {
                        std::hint::spin_loop();
                    }
                });
            });
            assert!(
                dispatched.is_some(),
                "the outer task saw no active pool; the nested scope was never opened"
            );
            // Released last: it publishes every store above to the waiting test thread.
            outer_done.store(true, Ordering::Release);
            waiter.unpark();
        });
    }

    // Wait for X to be identified AND idle-marked. The mask flickers: the joiner's park has a
    // backstop, so it wakes, rescans and re-marks. Any observation of the bit is the receipt.
    let mask_deadline = Instant::now() + Duration::from_secs(2);
    let joiner = loop {
        let id = joiner_id.load(Ordering::Acquire);
        if id < pool.worker_count() as usize && pool.parked_mask() & (1u64 << id) != 0 {
            break id;
        }
        assert!(
            Instant::now() < mask_deadline,
            "no joiner bit was ever set in parked_mask (joiner_id={id}, mask={:#x}): the worker \
             joiner did not park idle-marked, so rule B1-P is not being honoured",
            pool.parked_mask()
        );
        std::hint::spin_loop();
    };

    {
        let foreign_id = Arc::clone(&foreign_id);
        let release = Arc::clone(&release);
        pool.spawn(move || {
            foreign_id.store(current_worker_id() as usize, Ordering::Release);
            // Ordered after the receipt store: releasing the sibling ends the scope, and the
            // reader must not see the release without the id it is paired with.
            release.store(true, Ordering::Release);
        });
    }

    let deadline = Instant::now() + Duration::from_secs(60);
    while !outer_done.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "the outer task never completed within 60s: the parked joiner was never woken, or \
             its scope never drained"
        );
        std::thread::park_timeout(Duration::from_millis(20));
    }

    let body = body_id.load(Ordering::Acquire);
    let foreign = foreign_id.load(Ordering::Acquire);
    println!(
        "[ke16 b1-p] joiner={joiner} body_lane={body} foreign_lane={foreign} \
         workers={}",
        pool.worker_count()
    );

    assert!(
        joiner != WORKER_ID_DISPATCHER as usize && joiner != WORKER_ID_UNATTACHED as usize,
        "the outer task did not run on a registered worker (id={joiner}); the fixture measured \
         the dispatcher route"
    );
    assert_ne!(
        body, joiner,
        "the scope's body ran on the joining worker itself, so that worker never parked and the \
         fixture never built the state B1-P is about"
    );
    assert_eq!(
        foreign, joiner,
        "the foreign task ran on lane {foreign}, not on the parked joiner ({joiner}): the joiner's \
         idle bit was set but its wake did not reach it, or another lane was claimable"
    );
}
