//! KE16 — the pool-level occupancy harness for **nested** (worker-spawned) scopes.
//!
//! ## What this file measures and why it is at the pool level
//!
//! The 2026-08-30 census measured `par_iter` inside a scheduled system at **1.01x** against
//! **7.69x** for the identical driver called from the dispatcher thread, and localized the cause
//! to the pool, not to the query drivers: `push_task` (`src/worker.rs`) routes a task spawned by a
//! worker of *this* pool into `injector_local[wid]`, and no thread ever polls another thread's
//! local injector — sibling stealing walks `inner.stealers`, which holds worker **deques** only.
//! So work spawned from inside a system body is reachable by its own worker alone (**defect A**),
//! and `Scope::drop` additionally batch-steals about half the wave into a private, unregistered
//! FIFO `scratch` deque and runs it inline (**defect B**).
//!
//! Every ECS system body executes on a worker, so every `par_iter` in every system is on the
//! defective path. Measuring that through `boyko_ecs` would confound the pool with the query
//! driver's own chunking constant (`MIN_ARCHETYPE_FOR_PARALLEL = 1024`, which caps fan-out at
//! `ceil(N/1024)` independently of the pool). **This harness therefore drives the pool directly**:
//! two dispatch routes, identical waves, identical bodies. The only difference between them is
//! *who* pushed the tasks.
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
//! cargo test -p boyko-threadpool --test ke16_nested_scope_occupancy -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The second line is the DEFAULT build's only way to reach the red-first gate
//! ([`worker_spawned_wave_reaches_at_least_half_the_workers`]), where it is red by design. Under
//! any A arm that gate is not ignored and the first line runs it, so a feature build needs no
//! second invocation and cannot pass by never having run.

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::{
    MAX_WORKERS, ThreadPool, ThreadPoolBuilder, WORKER_ID_DISPATCHER, WORKER_ID_UNATTACHED,
    current_worker_id, ke16_check_expected_variant, try_with_active_pool,
};

/// Per-task body duration. 200 us is far above the ~100 ns Windows `Instant` granule and far
/// above the ~1 us push/steal cost, so neither the clock nor the dispatch overhead is in the
/// signal.
const BODY: Duration = Duration::from_micros(200);

/// Wave size as a multiple of the worker count. Four tasks per worker gives the pool room to
/// distribute even if it first hands one task to each worker and only then steals.
const TASKS_PER_WORKER: usize = 4;

/// Repetitions per (route, width) cell in the recording test. Three is enough to expose the
/// dispatcher route's bimodality without turning a 40 ms test into a benchmark; the bench file is
/// where distributions are measured properly.
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
    /// installed. `push_task`'s local-injector fast path is taken only when this is true, so a
    /// `false` here would mean the nested route never exercised defect A at all (matrix U1).
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
/// pool identity token for the same `same_pool` comparison `push_task` itself performs.
fn pool_addr(pool: &ThreadPool) -> usize {
    pool.install(|_| ThreadPool::current_pool().map_or(0, |p| p.as_ptr() as usize))
}

// ---------------------------------------------------------------------------
// The two routes
// ---------------------------------------------------------------------------

/// **(a) Dispatcher path** — the healthy route. The test thread is not a worker of this pool, so
/// `push_task`'s `same_pool` test is false and every task lands in `injector_global`, which every
/// worker drains.
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
    // Per TEST, not per file: the measurement protocol runs these FILTERED, so a witness printed
    // once from a harness main would not appear on the invocation whose number is recorded. The
    // banner is printed HERE rather than inside `ke16_check_expected_variant` because a print in
    // `crates/*/src/**.rs` reds `boyko-log`'s print census; the library owes the panic, the gate
    // owes the record (`KE16-DESIGN.md` §4).
    println!("KE16 variant: {}", boyko_threadpool::ke16_variant());
    ke16_check_expected_variant();
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

        // Repeated rather than averaged: the dispatcher route is bimodal at this checkout (how
        // much of the wave the joining thread batch-steals into its scratch varies run to run,
        // and that single quantity moves the wall-clock by 4x), and an average would report a
        // number that neither mode ever produces.
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
/// This is un-ignored and un-deferred because it is the *precondition* of the RED gate below: a
/// gate that reds for the wrong configuration is worth nothing, and a gate that greens because the
/// configuration drifted is worth less than nothing.
#[test]
fn worker_route_outer_task_runs_on_a_registered_worker_of_the_installed_pool() {
    println!("KE16 variant: {}", boyko_threadpool::ke16_variant());
    ke16_check_expected_variant();
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
        "the outer task's active pool is not the installed pool; push_task's same_pool test is \
         false and the local-injector path was never taken"
    );
}

/// **The RED-first gate, and it now RUNS wherever it can pass.** A wave spawned from inside a
/// worker task must reach at least half the workers simultaneously.
///
/// In the DEFAULT build it is RED, and that red is the finding rather than a fault: the wave lands
/// in `injector_local[wid]`, which only worker `wid` polls, and `Scope::drop` then batch-steals
/// about half of it into an unregistered `scratch` and runs it inline on that same worker. So the
/// `#[ignore]` is `cfg_attr`'d to exactly that build instead of being unconditional: the moment any
/// A arm is enabled the gate runs in the ORDINARY `cargo test` leg, with no `--ignored` needed.
/// `KE16-DESIGN-B.md` §2.7 makes the un-ignoring an obligation of the B axis, which is the last of
/// the four to land; until it was discharged this gate could not fail in ANY configuration, which
/// is the one property a red-first gate must never have.
///
/// MEASURED at this checkout, `max_in_flight` at W = 4 / W = 16 (floor 2 / 8): `a0` **1** — RED;
/// `a1` 4 / 16; `a1f` 4 / 16; `a2` 4 / 16; `a3` 4 / 15; `a5` 4 / 16; `a1+b1` 4 / 16; `a1+b3`
/// 4 / 16. Every A arm clears the floor, so the condition is "any A arm" and not "A1 only" — a
/// narrower cfg would keep a passing gate switched off and read as coverage the run does not have.
/// `ke16-a5` is absent from the list because Cargo implies `ke16-a2` from it (`KE16-DESIGN.md`
/// §4), so naming it again would be a cfg arm that cannot be reached on its own.
///
/// Do not weaken the threshold to make it pass — `W/2` is already a floor, not the target (the
/// dispatcher route reaches it on the same fixture).
#[test]
#[cfg_attr(
    not(any(
        feature = "ke16-a1",
        feature = "ke16-a1-fifo",
        feature = "ke16-a2",
        feature = "ke16-a3"
    )),
    ignore = "deferred: KE16 red-first occupancy gate; RED in the default build BY DESIGN because defect A (worker-spawned work unreachable by siblings) is fixed only under an A arm, and under any A arm this gate is NOT ignored and runs in the ordinary leg"
)]
fn worker_spawned_wave_reaches_at_least_half_the_workers() {
    println!("KE16 variant: {}", boyko_threadpool::ke16_variant());
    ke16_check_expected_variant();
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
             floor of {serial:.3}ms (speedup {sp:.2}x). Defect A: the tasks are in \
             injector_local[{owid}] and no sibling polls it.",
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
/// and the hang is bounded and reported. Under B0 the test cannot pass (X's bit is never set),
/// which is why it is compiled only under the B arms.
// === KE16 B switch: ke16-b1 / ke16-b3 ===
#[cfg(any(feature = "ke16-b1", feature = "ke16-b3"))]
#[test]
fn parked_joiner_is_claimed_by_a_foreign_wave() {
    println!("KE16 variant: {}", boyko_threadpool::ke16_variant());
    ke16_check_expected_variant();

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
             joiner did not park idle-marked, so rule B1-P is not in this build",
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
