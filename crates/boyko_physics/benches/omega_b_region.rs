//! W8S's two dispatch microbenches: ω_b, the stage barrier of a persistent solve region, and
//! ω(W, gap), a zero-work `pool.scope` after a serial gap.
//!
//! Built by the W8S lane and never run timed by it: the numbers are read in a quiet window
//! (`docs/physics/perf-campaign/levers/scaling/01-DESIGN.md` §7, "Microbenches"). They are the
//! inputs of the S1 build decision (the region's per-stage cost against today's per-wave scope)
//! and of the S2/S2′ choice.
//!
//! # ω_b: the region protocol, exactly (design §6.1 as revised, rev 1 W1)
//!
//! A bench-only copy of the protocol S1 would ship — no engine code, and no cheaper protocol,
//! because the bench prices this one:
//!
//! * **The stage table** is built before the first publish and never written while the region
//!   is open: per entry, its block count (at most `4 × participants`) and its first claim word.
//!   A stage that runs several times in one region keeps one entry and one set of blocks.
//! * **One claim word per block** (`AtomicU32`, 0 at build). A participant claims block `b` of an
//!   entry at execution `exec` by `compare_exchange(exec − 1, exec)`, so each block of each
//!   execution runs exactly once, and a late participant still scanning an old execution fails
//!   every CAS (Box2D `b2ExecuteStage`).
//! * **The publish word**: `(exec << 16) | stage`, `Release`; `0` is "nothing yet" and
//!   `u32::MAX` is END.
//! * **A completion count per entry**: `fetch_add(1, Release)` after each block. The
//!   orchestrator spins until it reads EXACTLY the entry's block count (`Acquire`), then resets it
//!   to 0 before the next publish.
//! * **One-block stages run inline** on the orchestrator with no publish (Box2D's
//!   `blockCount == 1` rule).
//! * **Helpers** spin on the publish word: `PAUSE` up to [`SPIN_PAUSES`] times, then `yield_now`
//!   (Box2D's `spinCount > 5`). They never park inside the region. Each claims from a staggered
//!   start (participant index × blocks / participants) and sweeps the ring once.
//!
//! One `pool.scope` holds the region: the orchestrator is the thread that opens it, the
//! `participants − 1` helpers are its tasks, and the scope's join outlives every helper, so a
//! helper that starts after END reads END and returns (work-conserving: the orchestrator alone
//! can finish every stage). The block payload is a run of `AtomicU64` slots with `Relaxed`
//! stores, so the bench has no `unsafe` and needs no Miri leg; the protocol's race-freedom is S1's
//! loom work (models M-R1 to M-R6), not this bench's.
//!
//! # Routes (rev 1 W6)
//!
//! Each row runs on the route production takes, named in its summary:
//!
//! * `worker`: the region's scope is opened by a pool task spawned inside `install`, so the
//!   orchestrator is a worker (the physics system's usual route);
//! * `external`: the scope is opened on the `install` frame's own thread, the dispatcher.
//!
//! The pool has `participants` workers on both routes, and the region spawns `participants − 1`
//! helpers, so `participants` threads can take part.
//!
//! # ω(W, gap) (cut Q7)
//!
//! A zero-work `pool.scope` with [`OMEGA_TASKS`] tasks, opened after a serial busy-wait of `gap`
//! µs on the opening thread, on a pool of W workers, on both routes. At gap 0 the helpers are
//! still spinning from the previous scope; past the pool's backoff they have parked, and the
//! scope pays the park→wake cost. The difference is what separates dispatch from wake in W8S's
//! per-wave loss.
//!
//! # Running
//!
//! ```text
//! cargo bench --no-run --locked --profile parity -p boyko-physics --bench omega_b_region
//! omega_b_region.exe --bench --mode omega-b --participants 2,4,8,16 --route worker
//! omega_b_region.exe --bench --mode omega --workers 8,16 --gap-us 0,5,20,80 --route external
//! ```
//!
//! Optional: `--regions R` (default 2000), `--stages S` (default 36: 12 passes × 3 colours),
//! `--blocks B` (default: `4 × participants`), `--block-slots N` (default 16), `--reps R`
//! (ω, default 2000). Each row prints one `SUMMARY {json}` line.
//!
//! Without `--bench` (the `cargo test --all-targets` run) the binary runs an untimed functional
//! self-check: the region at 1, 2 and 4 participants on both routes, every block of every
//! execution run exactly once, each stage's writes visible to the next, the completion count's
//! reset exercised by re-executing entries, and a bounded spin that panics rather than hangs; and
//! ω's scope at W 2 on both routes.

use std::hint::spin_loop;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use boyko_threadpool::{
    PoolInner, ThreadPool, ThreadPoolBuilder, WORKER_ID_DISPATCHER, current_worker_id,
    try_with_active_pool,
};

/// `PAUSE`s a helper spins before it yields (Box2D: `spinCount > 5`).
const SPIN_PAUSES: u32 = 5;
/// The END publish value.
const END: u32 = u32::MAX;
/// The self-check's bound on one wait, in spin iterations: a protocol defect hangs, and a hung
/// self-check must fail rather than stall `cargo test`.
const SELF_CHECK_SPIN_BOUND: u64 = 200_000_000;
/// ω's tasks per scope: a colour's task ceiling (L11 G7).
const OMEGA_TASKS: usize = 32;
/// Exit code: bad flags.
const EXIT_USAGE: u8 = 2;

// ── The region ────────────────────────────────────────────────────────────────

/// One entry of the stage table: its blocks and where their claim words and slots begin.
#[derive(Clone, Copy, Debug)]
struct Stage {
    /// Blocks, `1..=4 × participants`.
    blocks: u32,
    /// The entry's first claim word (and first block's index into `payload`).
    first_block: u32,
}

/// One region: the table, the claim words, the completion counts, the publish word and the
/// payload. Built before the first publish; only the atomics change while it is open.
struct Region {
    table: Vec<Stage>,
    /// Stage indices in execution order; an entry may repeat.
    schedule: Vec<u32>,
    claims: Vec<AtomicU32>,
    done: Vec<AtomicU32>,
    publish: AtomicU32,
    /// `slots` words per block.
    payload: Vec<AtomicU64>,
    slots: usize,
    /// Blocks run, over every execution: the exactly-once receipt.
    runs: AtomicU64,
    /// Bounded waits (the self-check) or unbounded (timed).
    bounded: bool,
}

impl Region {
    /// A region of `stages` entries of `blocks` blocks each, executed `repeats` times in order
    /// (so every entry re-runs, which exercises the completion count's reset), every block
    /// `slots` payload words. Entry 0 has one block when `one_block_first` (the inline rule).
    fn new(stages: u32, blocks: u32, repeats: u32, slots: usize, one_block_first: bool, bounded: bool) -> Self {
        let mut table = Vec::with_capacity(stages as usize);
        let mut first = 0u32;
        for s in 0..stages {
            let b = if s == 0 && one_block_first { 1 } else { blocks };
            table.push(Stage { blocks: b, first_block: first });
            first += b;
        }
        let schedule = (0..repeats).flat_map(|_| 0..stages).collect();
        Self {
            table,
            schedule,
            claims: (0..first).map(|_| AtomicU32::new(0)).collect(),
            done: (0..stages).map(|_| AtomicU32::new(0)).collect(),
            publish: AtomicU32::new(0),
            payload: (0..first as usize * slots).map(|_| AtomicU64::new(0)).collect(),
            slots,
            runs: AtomicU64::new(0),
            bounded,
        }
    }

    /// Resets every atomic for a fresh region over the same table: only between regions, after
    /// the previous region's scope has joined and its result was received (which orders every
    /// helper's last access before these stores).
    fn reset(&self) {
        for w in &self.claims {
            w.store(0, Ordering::Relaxed);
        }
        for d in &self.done {
            d.store(0, Ordering::Relaxed);
        }
        self.publish.store(0, Ordering::Relaxed);
        self.runs.store(0, Ordering::Relaxed);
    }

    /// Runs block `b` (global index) of entry `stage` at execution `exec`: stamps its slots with
    /// `(exec, stage)` and, in the self-check, reads the previous entry's slots of the same block
    /// position, which the stage order makes visible (M-R1).
    fn run_block(&self, stage: u32, b: u32, exec: u32) {
        let tag = (u64::from(exec) << 32) | u64::from(stage);
        let base = b as usize * self.slots;
        if self.bounded && stage > 0 {
            let prev = self.table[stage as usize - 1];
            let pb = prev.first_block + (b - self.table[stage as usize].first_block) % prev.blocks;
            let want = (u64::from(exec) << 32) | u64::from(stage - 1);
            let got = self.payload[pb as usize * self.slots].load(Ordering::Relaxed);
            assert_eq!(got, want, "M-R1: stage {stage} exec {exec} did not see stage {} of the same execution", stage - 1);
        }
        for w in &self.payload[base..base + self.slots] {
            w.store(tag, Ordering::Relaxed);
        }
        self.runs.fetch_add(1, Ordering::Relaxed);
    }

    /// Claims and runs blocks of `stage` at `exec` from the staggered `start`, sweeping the ring
    /// once.
    fn claim_and_run(&self, stage: u32, exec: u32, start: u32) {
        let st = self.table[stage as usize];
        for i in 0..st.blocks {
            let b = st.first_block + (start + i) % st.blocks;
            if self.claims[b as usize]
                .compare_exchange(exec - 1, exec, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                self.run_block(stage, b, exec);
                // Release: the block's writes happen-before the orchestrator's Acquire of the
                // count that includes this increment.
                self.done[stage as usize].fetch_add(1, Ordering::Release);
            }
        }
    }

    /// The orchestrator: every scheduled execution, then END.
    fn orchestrate(&self, participants: u32) {
        let mut exec = vec![0u32; self.table.len()];
        for &stage in &self.schedule {
            let e = &mut exec[stage as usize];
            *e += 1;
            let st = self.table[stage as usize];
            if st.blocks == 1 {
                // Box2D's rule: a one-block stage runs here, published to no one. Its claim word
                // still advances, so a later re-execution's CAS sees the right value.
                let b = st.first_block;
                let _ = self.claims[b as usize].compare_exchange(*e - 1, *e, Ordering::AcqRel, Ordering::Relaxed);
                self.run_block(stage, b, *e);
                continue;
            }
            // Release: the reset of `done` below and every earlier stage's writes happen-before a
            // helper's Acquire of this value.
            self.publish.store((*e << 16) | stage, Ordering::Release);
            self.claim_and_run(stage, *e, 0);
            let mut spins = 0u64;
            // Acquire: pairs with each block's Release increment. EXACT equality: a count that
            // overshot (a missing reset) must not read as complete.
            while self.done[stage as usize].load(Ordering::Acquire) != st.blocks {
                spin_loop();
                spins += 1;
                assert!(
                    !self.bounded || spins < SELF_CHECK_SPIN_BOUND,
                    "the orchestrator's wait on stage {stage} exec {} never completed: {} of {} blocks \
                     (participants {participants})",
                    *e,
                    self.done[stage as usize].load(Ordering::Relaxed),
                    st.blocks
                );
            }
            self.done[stage as usize].store(0, Ordering::Relaxed);
        }
        self.publish.store(END, Ordering::Release);
    }

    /// A helper: waits for a new publish value, claims from its staggered start, until END.
    fn help(&self, index: u32, participants: u32) {
        let mut last = 0u32;
        loop {
            let mut spins = 0u32;
            let mut waited = 0u64;
            let v = loop {
                // Acquire: pairs with the orchestrator's publish.
                let v = self.publish.load(Ordering::Acquire);
                if v != last {
                    break v;
                }
                if spins < SPIN_PAUSES {
                    spin_loop();
                    spins += 1;
                } else {
                    std::thread::yield_now();
                }
                waited += 1;
                assert!(
                    !self.bounded || waited < SELF_CHECK_SPIN_BOUND,
                    "helper {index} never saw a new publish value after {last:#x}"
                );
            };
            if v == END {
                return;
            }
            last = v;
            let (stage, exec) = (v & 0xFFFF, v >> 16);
            let blocks = self.table[stage as usize].blocks;
            self.claim_and_run(stage, exec, index * blocks / participants.max(1));
        }
    }

    /// One region on `pool`, opened from the calling thread: `participants − 1` helpers in one
    /// scope, the caller orchestrating.
    fn run_on(&self, pool: &PoolInner, participants: u32) {
        pool.scope(|scope| {
            for h in 1..participants {
                scope.spawn(move || self.help(h, participants));
            }
            self.orchestrate(participants);
        });
    }

    /// The blocks every scheduled execution runs.
    fn expected_runs(&self) -> u64 {
        self.schedule.iter().map(|&s| u64::from(self.table[s as usize].blocks)).sum()
    }
}

/// The two routes (module docs, "Routes").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Worker,
    External,
}

impl Route {
    fn name(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::External => "external",
        }
    }
}

/// Runs `f` on `route` of `pool` and returns its value and the thread's `current_worker_id()`:
/// on the `install` frame's own thread (external), or in a detached pool task (worker), which a
/// worker runs because the bench thread is no joiner and cannot steal it.
fn on_route<R: Send + 'static>(
    pool: &ThreadPool,
    route: Route,
    f: impl FnOnce(&PoolInner) -> R + Send + 'static,
) -> (R, u32) {
    let run = move || {
        let id = current_worker_id();
        let r = try_with_active_pool(f).expect("invariant: a pool is active on both routes");
        (r, id)
    };
    match route {
        Route::External => pool.install(|_| run()),
        Route::Worker => {
            let (tx, rx) = mpsc::channel();
            pool.spawn(move || {
                let _ = tx.send(run());
            });
            rx.recv().expect("invariant: the worker-route task sends its result")
        }
    }
}

/// Asserts that a row ran on the route it names (the route receipt).
fn assert_route(route: Route, id: u32, workers: usize) {
    match route {
        Route::External => {
            assert_eq!(id, WORKER_ID_DISPATCHER, "the external route ran on the install frame's thread");
        }
        Route::Worker => assert!((id as usize) < workers, "the worker route ran on a worker, not id {id:#x}"),
    }
}

// ── Command line ──────────────────────────────────────────────────────────────

/// What `main` does.
enum Mode {
    SelfCheck,
    OmegaB { participants: Vec<u32>, route: Route, regions: u32, stages: u32, blocks: Option<u32>, slots: usize },
    Omega { workers: Vec<usize>, gaps_us: Vec<u64>, route: Route, reps: u32 },
}

fn parse_list<T: std::str::FromStr>(flag: &str, v: Option<String>) -> Result<Vec<T>, String> {
    let v = v.ok_or_else(|| format!("{flag} needs a value"))?;
    v.split(',')
        .map(|x| x.parse().map_err(|_| format!("{flag}: cannot parse {x:?}")))
        .collect()
}

fn parse_one<T: std::str::FromStr>(flag: &str, v: Option<String>) -> Result<T, String> {
    let v = v.ok_or_else(|| format!("{flag} needs a value"))?;
    v.parse().map_err(|_| format!("{flag}: cannot parse {v:?}"))
}

fn parse_args(raw: &[String]) -> Result<Mode, String> {
    if !raw.iter().any(|a| a == "--bench") || !raw.iter().any(|a| a == "--mode") {
        return Ok(Mode::SelfCheck);
    }
    let mut mode = None;
    let mut participants = vec![2, 4, 8, 16];
    let mut workers = vec![8, 16];
    let mut gaps_us = vec![0, 5, 20, 80];
    let mut route = Route::Worker;
    let mut regions = 2000u32;
    let mut stages = 36u32;
    let mut blocks = None;
    let mut slots = 16usize;
    let mut reps = 2000u32;
    let mut it = raw.iter().cloned();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--bench" => {}
            "--mode" => mode = Some(it.next().ok_or("--mode needs omega-b|omega")?),
            "--participants" => participants = parse_list("--participants", it.next())?,
            "--workers" => workers = parse_list("--workers", it.next())?,
            "--gap-us" => gaps_us = parse_list("--gap-us", it.next())?,
            "--route" => {
                route = match it.next().as_deref() {
                    Some("worker") => Route::Worker,
                    Some("external") => Route::External,
                    other => return Err(format!("--route: expected worker|external, got {other:?}")),
                }
            }
            "--regions" => regions = parse_one("--regions", it.next())?,
            "--stages" => stages = parse_one("--stages", it.next())?,
            "--blocks" => blocks = Some(parse_one("--blocks", it.next())?),
            "--block-slots" => slots = parse_one("--block-slots", it.next())?,
            "--reps" => reps = parse_one("--reps", it.next())?,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    if participants.contains(&0) || workers.contains(&0) || stages == 0 || slots == 0 {
        return Err("participants, workers, stages and block slots must be at least 1".into());
    }
    match mode.as_deref() {
        Some("omega-b") => Ok(Mode::OmegaB { participants, route, regions, stages, blocks, slots }),
        Some("omega") => Ok(Mode::Omega { workers, gaps_us, route, reps }),
        other => Err(format!("--mode: expected omega-b|omega, got {other:?}")),
    }
}

// ── The modes ─────────────────────────────────────────────────────────────────

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.iter().any(|a| a == "--list") {
        return ExitCode::SUCCESS;
    }
    match parse_args(&raw) {
        Err(msg) => {
            eprintln!("omega_b_region: {msg}");
            ExitCode::from(EXIT_USAGE)
        }
        Ok(Mode::SelfCheck) => {
            self_check();
            ExitCode::SUCCESS
        }
        Ok(Mode::OmegaB { participants, route, regions, stages, blocks, slots }) => {
            for p in participants {
                omega_b_row(p, route, regions, stages, blocks.unwrap_or(4 * p), slots);
            }
            ExitCode::SUCCESS
        }
        Ok(Mode::Omega { workers, gaps_us, route, reps }) => {
            for w in workers {
                for &gap in &gaps_us {
                    omega_row(w, gap, route, reps);
                }
            }
            ExitCode::SUCCESS
        }
    }
}

/// The untimed functional self-check (module docs, "Running").
fn self_check() {
    println!("omega_b_region: no --bench --mode, so this is the untimed self-check");
    for route in [Route::External, Route::Worker] {
        for participants in [1u32, 2, 4] {
            let pool = ThreadPoolBuilder::new().num_threads(participants as usize).build();
            // Entry 0 has one block (the inline rule); three repeats re-run every entry, so the
            // completion count's reset is exercised twice per entry.
            let region = Arc::new(Region::new(4, 4 * participants, 3, 4, true, true));
            for _ in 0..3 {
                region.reset();
                let r = Arc::clone(&region);
                let ((), id) = on_route(&pool, route, move |inner| r.run_on(inner, participants));
                assert_route(route, id, participants as usize);
                assert_eq!(
                    region.runs.load(Ordering::Relaxed),
                    region.expected_runs(),
                    "every block of every execution runs exactly once ({} participants, {} route)",
                    participants,
                    route.name()
                );
                for (b, w) in region.claims.iter().enumerate() {
                    let owner = region.table.iter().position(|s| (s.first_block..s.first_block + s.blocks).contains(&(b as u32)));
                    let owner = owner.expect("invariant: every claim word belongs to an entry") as u32;
                    let execs = region.schedule.iter().filter(|&&s| s == owner).count() as u32;
                    assert_eq!(w.load(Ordering::Relaxed), execs, "block {b} ended at its entry's execution count");
                }
            }
            println!("  region: {participants} participant(s), {} route: ok", route.name());
        }
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let (wall, id) = on_route(&pool, route, |inner| omega_scope(inner, 0));
        assert_route(route, id, 2);
        assert!(wall > Duration::ZERO, "the zero-work scope ran");
        println!("  omega: W 2, {} route: ok", route.name());
    }
    println!("omega_b_region self-check: ok");
}

/// One ω_b row: `regions` regions of `stages` entries × `blocks` blocks, timed as a whole.
fn omega_b_row(participants: u32, route: Route, regions: u32, stages: u32, blocks: u32, slots: usize) {
    let pool = ThreadPoolBuilder::new().num_threads(participants as usize).build();
    let region = Arc::new(Region::new(stages, blocks.max(1), 1, slots, false, false));
    let mut walls = Vec::with_capacity(regions as usize);
    for _ in 0..regions {
        region.reset();
        let r = Arc::clone(&region);
        // Timed on the orchestrating thread, around the region alone.
        let (wall, id) = on_route(&pool, route, move |inner| {
            let t0 = Instant::now();
            r.run_on(inner, participants);
            t0.elapsed().as_nanos() as u64
        });
        assert_route(route, id, participants as usize);
        walls.push(wall);
    }
    walls.sort_unstable();
    let median = walls[walls.len() / 2];
    let mean = walls.iter().sum::<u64>() as f64 / walls.len() as f64;
    println!(
        "SUMMARY {{\"bench\":\"omega_b\",\"route\":\"{}\",\"participants\":{participants},\"regions\":{regions},\
         \"stages\":{stages},\"blocks\":{blocks},\"block_slots\":{slots},\"region_ns_median\":{median},\
         \"region_ns_mean\":{mean},\"stage_ns_median\":{}}}",
        route.name(),
        median as f64 / f64::from(stages)
    );
}

/// One zero-work scope of [`OMEGA_TASKS`] tasks after a `gap_us` busy-wait; returns the scope's
/// wall.
fn omega_scope(pool: &PoolInner, gap_us: u64) -> Duration {
    let gap = Duration::from_micros(gap_us);
    let t = Instant::now();
    while t.elapsed() < gap {
        spin_loop();
    }
    let t0 = Instant::now();
    pool.scope(|scope| {
        for _ in 0..OMEGA_TASKS {
            scope.spawn(|| {});
        }
    });
    t0.elapsed()
}

/// One ω(W, gap) row.
fn omega_row(workers: usize, gap_us: u64, route: Route, reps: u32) {
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let (walls, id): (Vec<u64>, u32) = on_route(&pool, route, move |inner| {
        (0..reps).map(|_| omega_scope(inner, gap_us).as_nanos() as u64).collect()
    });
    assert_route(route, id, workers);
    let mut sorted = walls;
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    let mean = sorted.iter().sum::<u64>() as f64 / sorted.len() as f64;
    println!(
        "SUMMARY {{\"bench\":\"omega\",\"route\":\"{}\",\"workers\":{workers},\"gap_us\":{gap_us},\
         \"tasks\":{OMEGA_TASKS},\"reps\":{reps},\"scope_ns_median\":{median},\"scope_ns_mean\":{mean}}}",
        route.name()
    );
}
