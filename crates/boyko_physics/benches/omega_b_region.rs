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
//! # The participation receipt (the instrument's review, N4)
//!
//! A scope wall alone cannot say whether any helper took part: an opener that ran all
//! [`OMEGA_TASKS`] tasks itself before a helper woke reads the same short wall as a pool that was
//! spinning. Each task therefore reads `current_worker_id()`, and a task on a thread other than
//! the opener's records the ns from the scope's opening to its start with one `Relaxed`
//! `fetch_min`. A row's summary carries `helped_reps` (reps where some task ran off the opener)
//! and `first_helper_ns_median` over those reps. The receipt's cost lands inside the timed scope:
//! one thread-local read per task, and a clock read and a `fetch_min` per task off the opener.
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
//! reset exercised by re-executing entries, and a bounded spin that panics rather than hangs; ω's
//! scope at W 2 on both routes; and the participation receipt reading no helper on a one-worker
//! pool. Then v2's self-check (below).
//!
//! # ω_b v2 (`--mode omega-b2`; window 8's `analysis.md` §5 and §9 item 3)
//!
//! Window 8 read ω_b as 2.8–3.0 µs a stage at eight participants and could not decide S1 on it:
//! v1's blocks do no work (16 relaxed stores), so every participant contends at once, and v1's
//! bench-only `runs.fetch_add` per block sits in `Region` beside `publish` and the `Vec` headers.
//! v2 is the same protocol (the stage table, one claim word per block claimed by
//! `compare_exchange(exec − 1, exec)`, the publish word, the exact completion count, the inline
//! one-block rule) with five changes, and v1's modes run unchanged beside it:
//!
//! * **Real work per block.** Each block owns one 64-byte line of eight `AtomicU64`s: it loads
//!   them, runs `work_iters` rounds of a fixed xorshift64* mix per word and stores them back.
//!   `work_iters` is calibrated once at start on the bench thread to `--work-ns` (default 700 ns;
//!   the median of three probes) and printed as one `CALIBRATION {json}` line, and every v2
//!   summary repeats it.
//! * **No shared read-modify-write per block beyond the protocol's own.** The claim CAS and the
//!   completion `fetch_add` ARE the protocol S1 would ship, and stay. The blocks a participant
//!   ran are counted in a local and stored once, at END, into its own line; the counts are summed
//!   after the join, and the exactly-once receipt is asserted on every timed region too (a failure
//!   panics, so the process exits non-zero).
//! * **Every claim word, completion word and the publish word on a line of its own**
//!   ([`Line`]), apart from each other and from every `Box` header.
//! * **Blocks per stage at 1×, 2× and 4× the participants** (`--blocks-per-participant`), and
//!   the participants capped at 8 by default (`--participants 2,4,8`; 16 is still accepted).
//! * **Both helper variants** (`--helper spin,park`), the 2026-09-26 ruling 6's owner-value
//!   question. `spin` is v1's: `PAUSE` up to [`SPIN_PAUSES`] times, then `yield_now`, never park.
//!   `park` spins `--park-spins` `PAUSE`s and `--park-yields` `yield_now`s (default 127 and 4,
//!   the pool's own idle backoff), then parks on the pool's own primitive, `std::thread::park`
//!   (bounded by [`PARK_TIMEOUT`]), and is woken by the publish: the helper registers its thread
//!   handle, stores its sleeping flag and re-reads the publish word, both `SeqCst`; the
//!   orchestrator stores each publish `SeqCst`, then reads every helper's flag `SeqCst` and
//!   unparks the ones it claims. That store→load pair on each side is the Dekker shape of the
//!   pool's own wake, so a publish is never missed: either the helper's re-read sees it, or the
//!   orchestrator's read sees the flag.
//!
//! **The participation receipt, per region:** how many participants ran at least one block (the
//! orchestrator included), and the ns from the region's opening to the first block a helper
//! claimed. **`--gap-us`** (default 0) adds a serial busy-wait on the orchestrator before every
//! publish — S1's serial stretches between stages (the inline narrow colours, warm apply,
//! integrate) — which is where the park variant's helpers outwait their spin budget; with stages
//! back to back they do not.
//!
//! ```text
//! omega_b_region.exe --bench --mode omega-b2 --participants 2,4,8 --stages 36,72 \
//!     --blocks-per-participant 1,2,4 --helper spin,park --route worker
//! ```
//!
//! Optional: `--regions R` (default 1000), `--work-ns N` (700), `--park-spins N` (127),
//! `--park-yields N` (4), `--gap-us G,..` (0). One `CALIBRATION` line, then one `SUMMARY {json}`
//! line per (participants, stages, blocks per participant, helper, gap), in that nesting order,
//! each naming `"bench":"omega_b2","version":2`.
//!
//! v2's self-check runs 1, 2 and 4 participants × both routes × both helpers × 1, 2 and 4 blocks
//! per participant, the park variant FORCED onto its park path (no spin, no yield), plus the park
//! variant on its default budget across a 2 ms gap, which must park; it asserts exactly-once, no lost wakeup,
//! `1 ≤ active ≤ participants`, and that a participant ran a block exactly when it recorded a
//! first claim. Every wait is bounded: a spin by [`SELF_CHECK_SPIN_BOUND`], a park by
//! [`PARK_TIMEOUT`], so a broken protocol fails rather than hangs.
//!
//! # ω_b v3 (`--mode region`; SR's M3, window 9b's SR-OMB3)
//!
//! v3 drives `boyko_threadpool::region` itself — the protocol SR ships, not a bench copy — through
//! its public API, under the shipped `V2Policy` (`v2epoch`) and under one policy per refinement axis
//! (`docs/physics/perf-campaign/levers/scaling/02-SR-DESIGN.md` §1.4): `Ra` batched completion,
//! `Rb` home claim lines, `Rc` TTAS claims, `Rd` helpers PAUSE for 50 µs before yielding, `Rd2`
//! helpers PAUSE only (ruling 2026-09-30 Q6's "PAUSE-only vs PAUSE+yield" axis), `Re` the
//! orchestrator yields after `REGION_STALL_NS`, and `all`. Each of these eight arms also has a
//! `+fin` twin (`v2epoch+fin`, `Ra+fin`, …, `all+fin`; CR-F, `levers/CR-colour-passes/
//! 01-CR-F-DESIGN.md`): the same policy under `WithAdvance<_, true>`, where the participant whose
//! completion add makes an item's count exact publishes the next item, every other const being the
//! arm's own (participant 0 still waits on the arm's orchestrator ladder, helpers on its helper
//! ladder). v2's own arm (`--mode omega-b2`) is the reference and is unchanged. Each block runs v2's calibrated work on its own payload line, and
//! `--gap-us` puts an inline item of that length before every stage. The receipt of every region —
//! exactly once, every participant's END and tag, every claim word at its entry's last epoch, and
//! the advance identity (`RegionReport::advances`: Σ receipt advances == published + 1, no helper
//! advance under the orchestrator) — is checked outside the timing, timed rows included.
//!
//! **The schedule's shape (`--entries`).** By default (`--entries stages`) every item is its own
//! entry: `--stages S` distinct stages, each run once per region, which is v2's timed shape (its rows
//! run `S` distinct stages once each, on claim words reset outside the timing). It is M3 check (1)'s
//! comparator: the same claim-line footprint as v2's row, and every item is its entry's first
//! execution in the region, so every claim takes the epoch claim's one retry — the upper bound of
//! what the epoch claims cost against v2's fresh words. `--entries E` instead cycles the items over
//! `E` entries (item k executes entry k mod E), so an entry re-executes within a region as a colour
//! does across a step's passes and only its first execution takes the retry; that shape is not v2's
//! and is not M3 (1)'s comparator.
//!
//! ```text
//! omega_b_region.exe --bench --mode region --policy v2epoch,Ra,Rb,Rc,Rd,Rd2,Re,all \
//!     --participants 2,4,8,16 --stages 36,72 --blocks-per-participant 1,2,4,6 --route worker
//! ```
//!
//! With the finisher twins (window SR's SR-OMB3 block with CR's `+fin` arms, 512 cells):
//!
//! ```text
//! omega_b_region.exe --bench --mode region \
//!     --policy v2epoch,Ra,Rb,Rc,Rd,Rd2,Re,all,v2epoch+fin,Ra+fin,Rb+fin,Rc+fin,Rd+fin,Rd2+fin,Re+fin,all+fin \
//!     --participants 2,4,8,16 --stages 36,72 --blocks-per-participant 1,2,4,6 --route worker
//! ```
//!
//! `--policy` defaults to the eight arms without `+fin`, so a row that omits it is unchanged.
//!
//! Optional: `--regions R` (1000), `--stages S,..` (36), `--entries stages|E` (stages),
//! `--work-ns N` (700), `--gap-us G,..` (0); `--bpp` is `--blocks-per-participant`. One `CALIBRATION`
//! line, then one `SUMMARY {json}` line per (participants, stages, blocks per participant, policy,
//! gap), in that nesting order, each naming `"bench":"omega_b3","version":3`. **`--plan`** appended
//! to a row's arguments times nothing: it parses them, prints one `PLAN {json}` line per cell the row
//! would run (in the same order, with the same identifying fields) and `PLAN_CELLS n`, and exits 0 —
//! no calibration, no pool. A window's pre-flight runs each v3 row that way, so an argument the
//! parser rejects stops the pre-flight, not the timed block. Every v3 SUMMARY also carries
//! `"helper_advances_median"` and `"helper_advanced_regions"` (the regions in which a helper made a
//! publish): a `+fin` row at two or more participants that reads 0 there never ran the finisher.
//!
//! v3's self-check (the bare run, after v1's and v2's) runs every policy (all sixteen arms), bounded,
//! at 2, 4, 8 and 16 participants, 1 and 6 blocks per participant, both routes, with and without a
//! 20 µs gap, 12 items over 3 entries and over 12 (one entry per item, the comparator's shape),
//! three regions over one frame each, and asserts the receipt above plus that helpers ran blocks
//! somewhere and that every `+fin` arm had a helper advance in some region (its bounded twin
//! `Bounded<WithAdvance<…>>` must forward `ADVANCE`, or it would run the orchestrator).

use std::hint::spin_loop;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::thread::Thread;
use std::time::{Duration, Instant};

use boyko_threadpool::{
    Advance, Ladder, PoolInner, REGION_MAX_BLOCKS_PER_PARTICIPANT, REGION_STALL_NS, RegionFrame,
    RegionLine, RegionLines, RegionPolicy, RegionReceipt, RegionReport, RegionStages, SchedItem,
    StageEntry, ThreadPool, ThreadPoolBuilder, V2Policy, WORKER_ID_DISPATCHER, WithAdvance,
    claim_lines, current_worker_id, link_hints, try_with_active_pool,
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
    if is_v3(&raw) {
        return match parse_args_v3(&raw) {
            Err(msg) => {
                eprintln!("omega_b_region: {msg}");
                ExitCode::from(EXIT_USAGE)
            }
            Ok(args) => {
                run_v3(&args);
                ExitCode::SUCCESS
            }
        };
    }
    if is_v2(&raw) {
        return match parse_args_v2(&raw) {
            Err(msg) => {
                eprintln!("omega_b_region: {msg}");
                ExitCode::from(EXIT_USAGE)
            }
            Ok(args) => {
                run_v2(&args);
                ExitCode::SUCCESS
            }
        };
    }
    match parse_args(&raw) {
        Err(msg) => {
            eprintln!("omega_b_region: {msg}");
            ExitCode::from(EXIT_USAGE)
        }
        Ok(Mode::SelfCheck) => {
            self_check();
            self_check_v2();
            self_check_v3();
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
        let ((wall, _), id) = on_route(&pool, route, |inner| omega_scope(inner, 0));
        assert_route(route, id, 2);
        assert!(wall > Duration::ZERO, "the zero-work scope ran");
        println!("  omega: W 2, {} route: ok", route.name());
    }
    // The participation receipt cannot claim a helper where none exists: one worker opening the
    // scope on the worker route is the only thread that can run its tasks (the bench thread is
    // no joiner).
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let (firsts, id) = on_route(&pool, Route::Worker, |inner| {
        (0..8).map(|_| omega_scope(inner, 0).1).collect::<Vec<_>>()
    });
    assert_route(Route::Worker, id, 1);
    assert!(
        firsts.iter().all(Option::is_none),
        "the receipt saw a helper on a one-worker pool: {firsts:?}"
    );
    println!("  omega receipt: W 1, worker route, no helper: ok");
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
/// wall and its participation receipt: the ns from the scope's opening to the first task a thread
/// other than the opener started, `None` when the opener ran every task (module docs, "The
/// participation receipt").
fn omega_scope(pool: &PoolInner, gap_us: u64) -> (Duration, Option<u64>) {
    let gap = Duration::from_micros(gap_us);
    let t = Instant::now();
    while t.elapsed() < gap {
        spin_loop();
    }
    let opener = current_worker_id();
    let first_helper = AtomicU64::new(u64::MAX);
    let t0 = Instant::now();
    pool.scope(|scope| {
        for _ in 0..OMEGA_TASKS {
            scope.spawn(|| {
                if current_worker_id() != opener {
                    let ns = u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX - 1);
                    first_helper.fetch_min(ns, Ordering::Relaxed);
                }
            });
        }
    });
    let wall = t0.elapsed();
    // Relaxed: the scope's join orders every task's `fetch_min` before this load.
    let first = first_helper.load(Ordering::Relaxed);
    (wall, (first != u64::MAX).then_some(first))
}

/// One ω(W, gap) row.
fn omega_row(workers: usize, gap_us: u64, route: Route, reps: u32) {
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let (runs, id): (Vec<(u64, Option<u64>)>, u32) = on_route(&pool, route, move |inner| {
        (0..reps)
            .map(|_| {
                let (wall, first) = omega_scope(inner, gap_us);
                (wall.as_nanos() as u64, first)
            })
            .collect()
    });
    assert_route(route, id, workers);
    let mut sorted: Vec<u64> = runs.iter().map(|&(wall, _)| wall).collect();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    let mean = sorted.iter().sum::<u64>() as f64 / sorted.len() as f64;
    let mut firsts: Vec<u64> = runs.iter().filter_map(|&(_, first)| first).collect();
    firsts.sort_unstable();
    let first_median = firsts.get(firsts.len() / 2).map_or_else(|| "null".to_owned(), u64::to_string);
    println!(
        "SUMMARY {{\"bench\":\"omega\",\"route\":\"{}\",\"workers\":{workers},\"gap_us\":{gap_us},\
         \"tasks\":{OMEGA_TASKS},\"reps\":{reps},\"scope_ns_median\":{median},\"scope_ns_mean\":{mean},\
         \"helped_reps\":{},\"first_helper_ns_median\":{first_median}}}",
        route.name(),
        firsts.len()
    );
}

// ── ω_b v2 (module docs, "ω_b v2") ────────────────────────────────────────────

/// A park's bound: a helper parked this long wakes and re-reads the publish word, so a lost
/// wakeup costs a bounded stall and is counted rather than hanging the region.
const PARK_TIMEOUT: Duration = Duration::from_millis(100);
/// `PAUSE`s the park variant spins before its yields: the pool's own idle backoff (design §3).
const PARK_SPINS: u32 = 127;
/// `yield_now`s the park variant takes after its spins, before it parks.
const PARK_YIELDS: u32 = 4;
/// The self-check's work per block, in mix rounds: the protocol under test, not its timing.
const SELF_CHECK_WORK_ITERS: u32 = 8;

/// One 64-byte line around `T`: v2 gives the publish word, every claim word, every completion
/// word, every block's payload and every participant's receipt a line of its own.
#[repr(C, align(64))]
struct Line<T>(T);

const _: () = assert!(
    size_of::<Line<AtomicU32>>() == 64 && align_of::<Line<AtomicU32>>() == 64,
    "a claim, completion or publish word owns one cache line"
);
const _: () = assert!(size_of::<Line<[AtomicU64; 8]>>() == 64, "a block's payload is one cache line");

/// How a v2 helper waits for the next publish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Helper {
    /// v1's wait: `PAUSE` up to [`SPIN_PAUSES`] times, then `yield_now`, never parked.
    Spin,
    /// A bounded spin and yield, then a park the publish wakes (module docs, "ω_b v2").
    Park,
}

impl Helper {
    fn name(self) -> &'static str {
        match self {
            Self::Spin => "spin",
            Self::Park => "park",
        }
    }
}

/// One participant's receipt for one region, read after the join.
#[derive(Clone, Copy, Debug)]
struct Receipt {
    /// Blocks it ran.
    ran: u64,
    /// ns from the region's opening to its first claimed block; `u64::MAX` when it ran none.
    first_ns: u64,
    /// Parks it took (helpers of the park variant only).
    parks: u64,
    /// Parks it woke from by timeout with its flag unclaimed and the publish word moved.
    lost: u64,
}

/// A participant's running tallies, local to its thread until END.
struct Tally2 {
    ran: u64,
    first_ns: u64,
    parks: u64,
    lost: u64,
}

impl Tally2 {
    fn new() -> Self {
        Self { ran: 0, first_ns: u64::MAX, parks: 0, lost: 0 }
    }
}

/// A v2 region's shape and knobs.
#[derive(Clone, Copy, Debug)]
struct Shape2 {
    participants: u32,
    stages: u32,
    blocks: u32,
    /// Executions of the whole table in order: more than one re-runs every entry (the
    /// self-check), which exercises the completion count's reset.
    repeats: u32,
    /// Entry 0 has one block (the inline rule).
    one_block_first: bool,
    work_iters: u32,
    helper: Helper,
    park_spins: u32,
    park_yields: u32,
    /// The orchestrator's serial busy-wait before every publish.
    gap: Duration,
    /// Bounded spins (the self-check) or unbounded (timed); a park is always bounded.
    bounded: bool,
}

/// One v2 region: the table, the schedule with each entry's execution number, and the lines.
/// Built before the first publish; only the atomics change while it is open.
struct Region2 {
    /// The publish word, `(exec << 16) | stage`; `0` is "nothing yet" and [`END`] is END.
    publish: Line<AtomicU32>,
    table: Vec<Stage>,
    /// `(stage, exec)` in execution order: the execution number is fixed here, so the
    /// orchestrator keeps no per-region scratch.
    schedule: Vec<(u32, u32)>,
    claims: Box<[Line<AtomicU32>]>,
    done: Box<[Line<AtomicU32>]>,
    payload: Box<[Line<[AtomicU64; 8]>]>,
    /// Per participant, stored once at END.
    ran: Box<[Line<AtomicU64>]>,
    first_ns: Box<[Line<AtomicU64>]>,
    parks: Box<[Line<AtomicU64>]>,
    lost: Box<[Line<AtomicU64>]>,
    /// Per participant: 1 while a park-variant helper is about to park or parked, unclaimed.
    sleeping: Box<[Line<AtomicU32>]>,
    shape: Shape2,
}

/// The fixed per-word mix one block runs `iters` times: xorshift64* (Vigna), a serial data
/// dependency the compiler cannot fold.
#[inline]
fn mix(mut x: u64, iters: u32) -> u64 {
    for _ in 0..iters {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
    }
    x
}

/// One block's work on its own payload line: load the eight words, mix each, store them back.
#[inline]
fn run_work(line: &Line<[AtomicU64; 8]>, iters: u32) {
    for w in &line.0 {
        let x = w.load(Ordering::Relaxed);
        w.store(mix(std::hint::black_box(x) | 1, iters), Ordering::Relaxed);
    }
}

/// `blocks` blocks of `iters` rounds on one line, timed on this thread: ns per block.
fn time_blocks(iters: u32, blocks: u32) -> f64 {
    let line = Line([const { AtomicU64::new(0) }; 8]);
    let t0 = Instant::now();
    for _ in 0..blocks {
        run_work(&line, iters);
    }
    let ns = t0.elapsed().as_nanos() as f64;
    std::hint::black_box(&line);
    ns / f64::from(blocks)
}

/// The calibration (module docs, "ω_b v2"): three probes of the per-round cost, their median
/// scaled to `work_ns`, then one check run at the result. Returns the rounds and the measured ns
/// per block, and prints the `CALIBRATION` line.
fn calibrate(work_ns: u64) -> (u32, f64) {
    const PROBE_ITERS: u32 = 256;
    const PROBE_BLOCKS: u32 = 4000;
    // Warm the clock, the line and the core.
    let _ = time_blocks(PROBE_ITERS, PROBE_BLOCKS);
    let probes: [f64; 3] =
        std::array::from_fn(|_| time_blocks(PROBE_ITERS, PROBE_BLOCKS) / f64::from(PROBE_ITERS));
    let mut sorted = probes;
    sorted.sort_by(f64::total_cmp);
    let per_iter = sorted[1].max(f64::MIN_POSITIVE);
    let iters = ((work_ns as f64 / per_iter).round() as u32).max(1);
    let calibrated = time_blocks(iters, PROBE_BLOCKS);
    println!(
        "CALIBRATION {{\"bench\":\"omega_b2\",\"version\":2,\"work_ns_target\":{work_ns},\"work_iters\":{iters},\
         \"work_ns_calibrated\":{calibrated:.1},\"probe_ns_per_iter\":[{:.4},{:.4},{:.4}]}}",
        probes[0], probes[1], probes[2]
    );
    (iters, calibrated)
}

/// `n` fresh lines.
fn lines<T>(n: usize, init: impl Fn() -> T) -> Box<[Line<T>]> {
    (0..n).map(|_| Line(init())).collect()
}

impl Region2 {
    /// A region of `s.stages` entries of `s.blocks` blocks each, executed `s.repeats` times.
    fn new(s: Shape2) -> Self {
        let mut table = Vec::with_capacity(s.stages as usize);
        let mut first = 0u32;
        for i in 0..s.stages {
            let b = if i == 0 && s.one_block_first { 1 } else { s.blocks.max(1) };
            table.push(Stage { blocks: b, first_block: first });
            first += b;
        }
        let schedule = (1..=s.repeats).flat_map(|e| (0..s.stages).map(move |st| (st, e))).collect();
        let p = s.participants as usize;
        Self {
            publish: Line(AtomicU32::new(0)),
            table,
            schedule,
            claims: lines(first as usize, || AtomicU32::new(0)),
            done: lines(s.stages as usize, || AtomicU32::new(0)),
            payload: lines(first as usize, || [const { AtomicU64::new(0) }; 8]),
            ran: lines(p, || AtomicU64::new(0)),
            first_ns: lines(p, || AtomicU64::new(u64::MAX)),
            parks: lines(p, || AtomicU64::new(0)),
            lost: lines(p, || AtomicU64::new(0)),
            sleeping: lines(p, || AtomicU32::new(0)),
            shape: s,
        }
    }

    /// Resets every atomic for a fresh region over the same table: only between regions, after
    /// the previous region's scope has joined and its result was received (which orders every
    /// helper's last access before these stores).
    fn reset(&self) {
        for w in self.claims.iter().chain(self.done.iter()).chain(self.sleeping.iter()) {
            w.0.store(0, Ordering::Relaxed);
        }
        for w in self.ran.iter().chain(self.parks.iter()).chain(self.lost.iter()) {
            w.0.store(0, Ordering::Relaxed);
        }
        for w in self.first_ns.iter() {
            w.0.store(u64::MAX, Ordering::Relaxed);
        }
        self.publish.0.store(0, Ordering::Relaxed);
    }

    /// Runs block `b`, counted in `t` (the first one timed from `open`).
    #[inline]
    fn run_block(&self, b: u32, t: &mut Tally2, open: Instant) {
        if t.ran == 0 {
            t.first_ns = u64::try_from(open.elapsed().as_nanos()).unwrap_or(u64::MAX - 1);
        }
        run_work(&self.payload[b as usize], self.shape.work_iters);
        t.ran += 1;
    }

    /// Claims and runs blocks of `stage` at `exec` from the staggered `start`, sweeping the ring
    /// once (v1's `claim_and_run`, counted locally).
    fn claim_and_run(&self, stage: u32, exec: u32, start: u32, t: &mut Tally2, open: Instant) {
        let st = self.table[stage as usize];
        for i in 0..st.blocks {
            let b = st.first_block + (start + i) % st.blocks;
            if self.claims[b as usize]
                .0
                .compare_exchange(exec - 1, exec, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                self.run_block(b, t, open);
                // Release: the block's writes happen-before the orchestrator's Acquire of the
                // count that includes this increment.
                self.done[stage as usize].0.fetch_add(1, Ordering::Release);
            }
        }
    }

    /// The orchestrator's publish: a `SeqCst` store, then (park variant) a `SeqCst` read of every
    /// helper's flag — the Dekker pair with the flag store and publish re-read in
    /// [`wait`](Self::wait), so either that re-read sees `v` or this read sees the flag. A flag
    /// read as set is claimed with a swap and its helper unparked.
    fn publish(&self, v: u32, parkers: &[OnceLock<Thread>]) {
        self.publish.0.store(v, Ordering::SeqCst);
        if self.shape.helper == Helper::Park {
            for (flag, parker) in self.sleeping.iter().zip(parkers).skip(1) {
                if flag.0.load(Ordering::SeqCst) == 1 && flag.0.swap(0, Ordering::SeqCst) == 1 {
                    // The helper registered its handle before it stored the flag (`wait`), and
                    // the SeqCst load above synchronises with that store.
                    parker.get().expect("invariant: a sleeping helper registered its thread").unpark();
                }
            }
        }
    }

    /// The orchestrator: every scheduled execution, then END; its receipt last.
    fn orchestrate(&self, parkers: &[OnceLock<Thread>], open: Instant) {
        let mut t = Tally2::new();
        for &(stage, exec) in &self.schedule {
            if !self.shape.gap.is_zero() {
                let g = Instant::now();
                while g.elapsed() < self.shape.gap {
                    spin_loop();
                }
            }
            let st = self.table[stage as usize];
            if st.blocks == 1 {
                // Box2D's rule: a one-block stage runs here, published to no one. Its claim word
                // still advances, so a later re-execution's CAS sees the right value.
                let b = st.first_block;
                let _ = self.claims[b as usize].0.compare_exchange(exec - 1, exec, Ordering::AcqRel, Ordering::Relaxed);
                self.run_block(b, &mut t, open);
                continue;
            }
            // SeqCst (a release): the reset of `done` below and every earlier stage's writes
            // happen-before a helper's acquiring read of this value.
            self.publish((exec << 16) | stage, parkers);
            self.claim_and_run(stage, exec, 0, &mut t, open);
            let mut spins = 0u64;
            // Acquire: pairs with each block's Release increment. EXACT equality: a count that
            // overshot (a missing reset) must not read as complete.
            while self.done[stage as usize].0.load(Ordering::Acquire) != st.blocks {
                spin_loop();
                spins += 1;
                assert!(
                    !self.shape.bounded || spins < SELF_CHECK_SPIN_BOUND,
                    "v2: the orchestrator's wait on stage {stage} exec {exec} never completed: {} of {} blocks \
                     ({} participants, {} helper)",
                    self.done[stage as usize].0.load(Ordering::Relaxed),
                    st.blocks,
                    self.shape.participants,
                    self.shape.helper.name()
                );
            }
            self.done[stage as usize].0.store(0, Ordering::Relaxed);
        }
        self.publish(END, parkers);
        self.store_receipt(0, &t);
    }

    /// Participant `p`'s receipt, stored once. Relaxed: read only after the scope's join.
    fn store_receipt(&self, p: usize, t: &Tally2) {
        self.ran[p].0.store(t.ran, Ordering::Relaxed);
        self.first_ns[p].0.store(t.first_ns, Ordering::Relaxed);
        self.parks[p].0.store(t.parks, Ordering::Relaxed);
        self.lost[p].0.store(t.lost, Ordering::Relaxed);
    }

    /// Helper `h`'s wait for a publish value other than `last`.
    fn wait(&self, h: usize, last: u32, parkers: &[OnceLock<Thread>], t: &mut Tally2) -> u32 {
        let (mut spins, mut yields, mut waited) = (0u32, 0u32, 0u64);
        loop {
            // Acquire: pairs with the orchestrator's publish.
            let v = self.publish.0.load(Ordering::Acquire);
            if v != last {
                return v;
            }
            waited += 1;
            match self.shape.helper {
                Helper::Spin => {
                    if spins < SPIN_PAUSES {
                        spin_loop();
                        spins += 1;
                    } else {
                        std::thread::yield_now();
                    }
                    assert!(
                        !self.shape.bounded || waited < SELF_CHECK_SPIN_BOUND,
                        "v2: helper {h} never saw a new publish value after {last:#x}"
                    );
                }
                Helper::Park if spins < self.shape.park_spins => {
                    spin_loop();
                    spins += 1;
                }
                Helper::Park if yields < self.shape.park_yields => {
                    std::thread::yield_now();
                    yields += 1;
                }
                Helper::Park => {
                    parkers[h].get_or_init(std::thread::current);
                    // SeqCst store, then SeqCst load: the helper's half of the Dekker pair
                    // (`publish`).
                    self.sleeping[h].0.store(1, Ordering::SeqCst);
                    let v = self.publish.0.load(Ordering::SeqCst);
                    if v != last {
                        // Cancelled. If the orchestrator claimed the flag first, its unpark leaves
                        // a token that a later park consumes as a spurious wake.
                        self.sleeping[h].0.swap(0, Ordering::SeqCst);
                        return v;
                    }
                    let t0 = Instant::now();
                    std::thread::park_timeout(PARK_TIMEOUT);
                    t.parks += 1;
                    // A flag still set was claimed by no publish. Woken by the timeout with the
                    // publish moved, that is a lost wakeup; a spurious or token wake returns long
                    // before the timeout, and a helper descheduled after a claimed unpark finds its
                    // flag cleared (review O4).
                    let unclaimed = self.sleeping[h].0.swap(0, Ordering::SeqCst) == 1;
                    if unclaimed && t0.elapsed() >= PARK_TIMEOUT && self.publish.0.load(Ordering::SeqCst) != last {
                        t.lost += 1;
                    }
                }
            }
        }
    }

    /// A helper: waits for a new publish value, claims from its staggered start, until END.
    fn help(&self, h: u32, parkers: &[OnceLock<Thread>], open: Instant) {
        let mut t = Tally2::new();
        let mut last = 0u32;
        loop {
            let v = self.wait(h as usize, last, parkers, &mut t);
            if v == END {
                break;
            }
            last = v;
            let (stage, exec) = (v & 0xFFFF, v >> 16);
            let blocks = self.table[stage as usize].blocks;
            self.claim_and_run(stage, exec, h * blocks / self.shape.participants.max(1), &mut t, open);
        }
        self.store_receipt(h as usize, &t);
    }

    /// One region on `pool`, opened from the calling thread: `participants − 1` helpers in one
    /// scope, the caller orchestrating. `open` is the region's opening, for the first-claim times.
    fn run_on(&self, pool: &PoolInner, parkers: &[OnceLock<Thread>], open: Instant) {
        pool.scope(|scope| {
            for h in 1..self.shape.participants {
                scope.spawn(move || self.help(h, parkers, open));
            }
            self.orchestrate(parkers, open);
        });
    }

    /// The blocks every scheduled execution runs.
    fn expected_runs(&self) -> u64 {
        self.schedule.iter().map(|&(s, _)| u64::from(self.table[s as usize].blocks)).sum()
    }

    /// Every participant's receipt, after the join.
    fn receipts(&self) -> Vec<Receipt> {
        (0..self.shape.participants as usize)
            .map(|p| Receipt {
                ran: self.ran[p].0.load(Ordering::Relaxed),
                first_ns: self.first_ns[p].0.load(Ordering::Relaxed),
                parks: self.parks[p].0.load(Ordering::Relaxed),
                lost: self.lost[p].0.load(Ordering::Relaxed),
            })
            .collect()
    }
}

/// One region's reduced receipt.
#[derive(Clone, Copy, Debug)]
struct RegionReading {
    /// Participants that ran at least one block, the orchestrator included.
    active: u32,
    /// The first block a helper claimed, ns from the opening; `None` when no helper ran one.
    first_helper_ns: Option<u64>,
    parks: u64,
    lost: u64,
}

/// Checks one region's receipts — every block of every execution ran exactly once, then a
/// participant ran a block exactly when it recorded a first claim — and reduces them. Panics on
/// a violation, in the timed mode too, so a broken region cannot leave a number behind.
fn check_region(region: &Region2, what: &str) -> RegionReading {
    let rs = region.receipts();
    let sum: u64 = rs.iter().map(|r| r.ran).sum();
    assert_eq!(
        sum,
        region.expected_runs(),
        "v2 exactly-once: {what}: the participants ran {sum} blocks, the schedule has {} ({rs:?})",
        region.expected_runs()
    );
    for (p, r) in rs.iter().enumerate() {
        assert_eq!(
            r.ran >= 1,
            r.first_ns != u64::MAX,
            "v2 receipt: {what}: participant {p} ran {} block(s) and its first-claim time is {} ({rs:?})",
            r.ran,
            r.first_ns
        );
    }
    RegionReading {
        active: rs.iter().filter(|r| r.ran >= 1).count() as u32,
        first_helper_ns: rs.iter().skip(1).filter(|r| r.ran >= 1).map(|r| r.first_ns).min(),
        parks: rs.iter().map(|r| r.parks).sum(),
        lost: rs.iter().map(|r| r.lost).sum(),
    }
}

/// One region of `region` on `route` of `pool`: the wall timed on the orchestrating thread, and
/// the checked receipt. The helpers' park handles are fresh per region, built here, outside the
/// timed call.
fn region_once(pool: &ThreadPool, route: Route, region: &Arc<Region2>, what: &str) -> (u64, RegionReading) {
    region.reset();
    let parkers: Arc<[OnceLock<Thread>]> = (0..region.shape.participants).map(|_| OnceLock::new()).collect();
    let r = Arc::clone(region);
    let (wall, id) = on_route(pool, route, move |inner| {
        let open = Instant::now();
        r.run_on(inner, &parkers, open);
        open.elapsed().as_nanos() as u64
    });
    assert_route(route, id, region.shape.participants as usize);
    (wall, check_region(region, what))
}

/// `--mode omega-b2`'s arguments.
struct ArgsV2 {
    participants: Vec<u32>,
    route: Route,
    regions: u32,
    stages: Vec<u32>,
    blocks_per_participant: Vec<u32>,
    helpers: Vec<Helper>,
    work_ns: u64,
    park_spins: u32,
    park_yields: u32,
    gaps_us: Vec<u64>,
}

/// Whether the command line selects v2 (`--bench --mode omega-b2`).
fn is_v2(raw: &[String]) -> bool {
    raw.iter().any(|a| a == "--bench") && raw.windows(2).any(|w| w[0] == "--mode" && w[1] == "omega-b2")
}

fn parse_args_v2(raw: &[String]) -> Result<ArgsV2, String> {
    let mut a = ArgsV2 {
        participants: vec![2, 4, 8],
        route: Route::Worker,
        regions: 1000,
        stages: vec![36, 72],
        blocks_per_participant: vec![1, 2, 4],
        helpers: vec![Helper::Spin, Helper::Park],
        work_ns: 700,
        park_spins: PARK_SPINS,
        park_yields: PARK_YIELDS,
        gaps_us: vec![0],
    };
    let mut it = raw.iter().cloned();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--bench" => {}
            "--mode" => {
                let _ = it.next();
            }
            "--participants" => a.participants = parse_list("--participants", it.next())?,
            "--route" => {
                a.route = match it.next().as_deref() {
                    Some("worker") => Route::Worker,
                    Some("external") => Route::External,
                    other => return Err(format!("--route: expected worker|external, got {other:?}")),
                }
            }
            "--regions" => a.regions = parse_one("--regions", it.next())?,
            "--stages" => a.stages = parse_list("--stages", it.next())?,
            "--blocks-per-participant" => {
                a.blocks_per_participant = parse_list("--blocks-per-participant", it.next())?;
            }
            "--helper" => {
                let names: Vec<String> = parse_list("--helper", it.next())?;
                a.helpers = names
                    .iter()
                    .map(|n| match n.as_str() {
                        "spin" => Ok(Helper::Spin),
                        "park" => Ok(Helper::Park),
                        other => Err(format!("--helper: expected spin|park, got {other:?}")),
                    })
                    .collect::<Result<_, _>>()?;
            }
            "--work-ns" => a.work_ns = parse_one("--work-ns", it.next())?,
            "--park-spins" => a.park_spins = parse_one("--park-spins", it.next())?,
            "--park-yields" => a.park_yields = parse_one("--park-yields", it.next())?,
            "--gap-us" => a.gaps_us = parse_list("--gap-us", it.next())?,
            other => return Err(format!("omega-b2: unknown argument {other:?}")),
        }
    }
    if a.participants.iter().any(|&p| p == 0 || p > 16) {
        return Err("omega-b2: participants must be 1..=16 (8 is the default cap)".into());
    }
    if a.stages.contains(&0) || a.blocks_per_participant.contains(&0) || a.regions == 0 || a.work_ns == 0 {
        return Err("omega-b2: stages, blocks per participant, regions and work ns must be at least 1".into());
    }
    if a.helpers.is_empty() {
        return Err("omega-b2: --helper names no variant".into());
    }
    Ok(a)
}

/// The median of `v` (sorted in place); `None` when empty.
fn median(v: &mut [u64]) -> Option<u64> {
    v.sort_unstable();
    v.get(v.len() / 2).copied()
}

/// `--mode omega-b2`: the calibration, then every row.
fn run_v2(a: &ArgsV2) {
    let (work_iters, work_ns_calibrated) = calibrate(a.work_ns);
    for &p in &a.participants {
        let pool = ThreadPoolBuilder::new().num_threads(p as usize).build();
        for &stages in &a.stages {
            for &bpp in &a.blocks_per_participant {
                for &helper in &a.helpers {
                    for &gap_us in &a.gaps_us {
                        let shape = Shape2 {
                            participants: p,
                            stages,
                            blocks: bpp * p,
                            repeats: 1,
                            one_block_first: false,
                            work_iters,
                            helper,
                            park_spins: a.park_spins,
                            park_yields: a.park_yields,
                            gap: Duration::from_micros(gap_us),
                            bounded: false,
                        };
                        omega_b2_row(&pool, a, shape, bpp, gap_us, work_ns_calibrated);
                    }
                }
            }
        }
    }
}

/// One v2 row: `regions` regions of one shape, each timed as a whole and its receipt checked.
fn omega_b2_row(pool: &ThreadPool, a: &ArgsV2, s: Shape2, bpp: u32, gap_us: u64, work_ns_calibrated: f64) {
    let region = Arc::new(Region2::new(s));
    let n = a.regions as usize;
    let (mut walls, mut actives, mut firsts, mut parks) =
        (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
    let (mut all_active, mut lost) = (0u32, 0u64);
    for _ in 0..a.regions {
        let (wall, r) = region_once(pool, a.route, &region, "a timed row");
        walls.push(wall);
        actives.push(u64::from(r.active));
        all_active += u32::from(r.active == s.participants);
        firsts.extend(r.first_helper_ns);
        parks.push(r.parks);
        lost += r.lost;
    }
    let mean = walls.iter().sum::<u64>() as f64 / walls.len() as f64;
    let wall_median = median(&mut walls).unwrap_or(0);
    let helped = firsts.len();
    let first_median = median(&mut firsts).map_or_else(|| "null".to_owned(), |v| v.to_string());
    println!(
        "SUMMARY {{\"bench\":\"omega_b2\",\"version\":2,\"route\":\"{}\",\"helper\":\"{}\",\
         \"participants\":{},\"stages\":{},\"blocks\":{},\"blocks_per_participant\":{bpp},\"gap_us\":{gap_us},\
         \"park_spins\":{},\"park_yields\":{},\"regions\":{},\"work_iters\":{},\
         \"work_ns_calibrated\":{work_ns_calibrated:.1},\"region_ns_median\":{wall_median},\"region_ns_mean\":{mean},\
         \"stage_ns_median\":{},\"exactly_once\":true,\"active_median\":{},\"all_active_reps\":{all_active},\
         \"helped_reps\":{helped},\"first_helper_ns_median\":{first_median},\"parks_per_region_median\":{},\
         \"lost_wakeups\":{lost}}}",
        a.route.name(),
        s.helper.name(),
        s.participants,
        s.stages,
        s.blocks,
        s.park_spins,
        s.park_yields,
        a.regions,
        s.work_iters,
        wall_median as f64 / f64::from(s.stages),
        median(&mut actives).unwrap_or(0),
        median(&mut parks).unwrap_or(0),
    );
}

/// v2's untimed functional self-check (module docs, "ω_b v2").
fn self_check_v2() {
    println!("omega_b_region: v2 self-check");
    let mut forced_parks = 0;
    for route in [Route::External, Route::Worker] {
        for participants in [1u32, 2, 4] {
            let pool = ThreadPoolBuilder::new().num_threads(participants as usize).build();
            for helper in [Helper::Spin, Helper::Park] {
                for bpp in [1u32, 2, 4] {
                    // The park variant is forced onto its park path: no spin, no yield.
                    let shape = Shape2 {
                        participants,
                        stages: 4,
                        blocks: bpp * participants,
                        repeats: 3,
                        one_block_first: true,
                        work_iters: SELF_CHECK_WORK_ITERS,
                        helper,
                        park_spins: 0,
                        park_yields: 0,
                        gap: Duration::ZERO,
                        bounded: true,
                    };
                    forced_parks += self_check_region(&pool, route, shape);
                }
            }
            println!("  v2 region: {participants} participant(s), {} route, spin and forced park: ok", route.name());
        }
    }
    // Anti-vacuity: the forced rows reached the park path, or "no lost wakeup" was never at risk.
    assert!(forced_parks > 0, "v2: the forced park rows never parked");
    println!("  v2 forced park rows: {forced_parks} parks in all");
    // The park variant on its default budget across a gap longer than that budget, so the
    // helpers park between stages the way S1's helpers would in its serial stretches.
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let shape = Shape2 {
        participants: 2,
        stages: 4,
        blocks: 4,
        repeats: 2,
        one_block_first: false,
        work_iters: SELF_CHECK_WORK_ITERS,
        helper: Helper::Park,
        park_spins: PARK_SPINS,
        park_yields: PARK_YIELDS,
        gap: Duration::from_millis(2),
        bounded: true,
    };
    let parks = self_check_region(&pool, Route::Worker, shape);
    assert!(parks > 0, "v2: the default-budget park row never parked across a 2 ms gap");
    println!("  v2 region: 2 participants, worker route, park on its default budget across a 2 ms gap ({parks} parks): ok");
    println!("omega_b_region v2 self-check: ok");
}

/// Three regions of `shape`: the receipts checked, no lost wakeup, `1 <= active <= P`, every
/// claim word at its entry's execution count. Returns the parks the three took.
fn self_check_region(pool: &ThreadPool, route: Route, shape: Shape2) -> u64 {
    let region = Arc::new(Region2::new(shape));
    let what = format!(
        "{} participant(s), {} route, {} helper, {} blocks",
        shape.participants,
        route.name(),
        shape.helper.name(),
        shape.blocks
    );
    let mut parks = 0;
    for _ in 0..3 {
        let (_, r) = region_once(pool, route, &region, &what);
        assert_eq!(r.lost, 0, "v2: {what}: {} lost wakeup(s), a helper slept through a publish", r.lost);
        assert!(
            (1..=shape.participants).contains(&r.active),
            "v2: {what}: {} participants active, of {}",
            r.active,
            shape.participants
        );
        for (b, w) in region.claims.iter().enumerate() {
            let owner = region
                .table
                .iter()
                .position(|s| (s.first_block..s.first_block + s.blocks).contains(&(b as u32)))
                .expect("invariant: every claim word belongs to an entry") as u32;
            let execs = region.schedule.iter().filter(|&&(s, _)| s == owner).count() as u32;
            assert_eq!(w.0.load(Ordering::Relaxed), execs, "v2: {what}: block {b} ended at its entry's execution count");
        }
        parks += r.parks;
    }
    parks
}

// ── ω_b v3: `boyko_threadpool::region` itself (`--mode region`; SR's M3, window 9b SR-OMB3) ─────

/// One v3 axis policy: v2's ladders and protocol with the named axes on, unbounded (timed rows).
macro_rules! v3_policy {
    ($(#[$m:meta])* $name:ident, helper = $h:expr, orch = $o:expr, batched = $ba:expr, home = $ho:expr, ttas = $t:expr) => {
        $(#[$m])*
        struct $name;
        impl RegionPolicy for $name {
            const HELPER_WAIT: Ladder = $h;
            const ORCH_WAIT: Ladder = $o;
            const BOUND_NS: u64 = 0;
            const DONE_BATCHED: bool = $ba;
            const HOME_LINES: bool = $ho;
            const TTAS: bool = $t;
        }
    };
}

/// v3's helper budget for R-d: the 50 µs of the plan's rev 1 KD4(c).
const V3_RD_SPIN_NS: u32 = 50_000;

v3_policy!(
    /// R-a: one batched completion add per participant per stage.
    V3Ra, helper = Ladder::PauseThenYield { pauses: 5 }, orch = Ladder::PureSpin, batched = true, home = false, ttas = false
);
v3_policy!(
    /// R-b: a participant's home claim words share one line.
    V3Rb, helper = Ladder::PauseThenYield { pauses: 5 }, orch = Ladder::PureSpin, batched = false, home = true, ttas = false
);
v3_policy!(
    /// R-c: TTAS claims.
    V3Rc, helper = Ladder::PauseThenYield { pauses: 5 }, orch = Ladder::PureSpin, batched = false, home = false, ttas = true
);
v3_policy!(
    /// R-d: helpers PAUSE for a 50 µs budget before yielding.
    V3Rd, helper = Ladder::BudgetThenYield { spin_ns: V3_RD_SPIN_NS }, orch = Ladder::PureSpin, batched = false, home = false, ttas = false
);
v3_policy!(
    /// R-d′ (ruling 2026-09-30 Q6's axis): helpers PAUSE only, never yield.
    V3Rd2, helper = Ladder::PureSpin, orch = Ladder::PureSpin, batched = false, home = false, ttas = false
);
v3_policy!(
    /// R-e: the orchestrator PAUSEs for `REGION_STALL_NS`, then alternates yields with PAUSE bursts.
    V3Re, helper = Ladder::PauseThenYield { pauses: 5 }, orch = Ladder::BudgetThenYield { spin_ns: REGION_STALL_NS }, batched = false, home = false, ttas = false
);
v3_policy!(
    /// Every axis on.
    V3All, helper = Ladder::BudgetThenYield { spin_ns: V3_RD_SPIN_NS }, orch = Ladder::BudgetThenYield { spin_ns: REGION_STALL_NS }, batched = true, home = true, ttas = true
);

/// The self-check's twin of any policy: the same ladders, axes and advance, with a 2 s wait bound,
/// so a broken protocol fails rather than hangs. `ADVANCE` is forwarded explicitly: the trait
/// defaults it to the orchestrator, so a wrapper that lists only the other consts would silently
/// self-check a `+fin` arm as the orchestrator (review O2).
struct Bounded<P>(std::marker::PhantomData<P>);

impl<P: RegionPolicy> RegionPolicy for Bounded<P> {
    const HELPER_WAIT: Ladder = P::HELPER_WAIT;
    const ORCH_WAIT: Ladder = P::ORCH_WAIT;
    const BOUND_NS: u64 = 2_000_000_000;
    const DONE_BATCHED: bool = P::DONE_BATCHED;
    const HOME_LINES: bool = P::HOME_LINES;
    const TTAS: bool = P::TTAS;
    const ADVANCE: Advance = P::ADVANCE;
}

/// The v3 policies by name (`--policy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum V3Policy {
    /// `V2Policy`, the shipped one (M3 check (1): its w against v2's own arm).
    V2Epoch,
    Ra,
    Rb,
    Rc,
    Rd,
    Rd2,
    Re,
    All,
}

impl V3Policy {
    const ALL: [Self; 8] = [Self::V2Epoch, Self::Ra, Self::Rb, Self::Rc, Self::Rd, Self::Rd2, Self::Re, Self::All];

    fn name(self) -> &'static str {
        match self {
            Self::V2Epoch => "v2epoch",
            Self::Ra => "Ra",
            Self::Rb => "Rb",
            Self::Rc => "Rc",
            Self::Rd => "Rd",
            Self::Rd2 => "Rd2",
            Self::Re => "Re",
            Self::All => "all",
        }
    }

}

/// One v3 arm (`--policy`): an axis policy, and whether it runs under the finisher advance
/// (`<axis>+fin`, `WithAdvance<_, true>`; CR-F).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct V3Arm {
    axis: V3Policy,
    fin: bool,
}

impl V3Arm {
    /// The eight orchestrator arms: `--policy`'s default, SR-OMB3's own grid.
    const ALL8: [Self; 8] = Self::arms(false);
    /// Every arm: the eight orchestrator arms, then their eight `+fin` twins.
    const ALL16: [Self; 16] = {
        let (a, b) = (Self::arms(false), Self::arms(true));
        [a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]
    };

    const fn arms(fin: bool) -> [Self; 8] {
        let p = V3Policy::ALL;
        [
            Self { axis: p[0], fin },
            Self { axis: p[1], fin },
            Self { axis: p[2], fin },
            Self { axis: p[3], fin },
            Self { axis: p[4], fin },
            Self { axis: p[5], fin },
            Self { axis: p[6], fin },
            Self { axis: p[7], fin },
        ]
    }

    fn name(self) -> &'static str {
        match (self.axis, self.fin) {
            (axis, false) => axis.name(),
            (V3Policy::V2Epoch, true) => "v2epoch+fin",
            (V3Policy::Ra, true) => "Ra+fin",
            (V3Policy::Rb, true) => "Rb+fin",
            (V3Policy::Rc, true) => "Rc+fin",
            (V3Policy::Rd, true) => "Rd+fin",
            (V3Policy::Rd2, true) => "Rd2+fin",
            (V3Policy::Re, true) => "Re+fin",
            (V3Policy::All, true) => "all+fin",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL16.into_iter().find(|a| a.name() == s)
    }
}

/// Dispatches `$f::<Policy>($args)` on a [`V3Arm`], optionally wrapped in [`Bounded`]. A `+fin` arm
/// runs `WithAdvance<axis policy, true>`, so under [`Plain`] the timed rows run exactly that type.
macro_rules! with_v3_policy {
    ($p:expr, $wrap:ident, $f:ident ( $($a:expr),* )) => {
        match ($p.axis, $p.fin) {
            (V3Policy::V2Epoch, false) => $f::<$wrap<V2Policy>>($($a),*),
            (V3Policy::Ra, false) => $f::<$wrap<V3Ra>>($($a),*),
            (V3Policy::Rb, false) => $f::<$wrap<V3Rb>>($($a),*),
            (V3Policy::Rc, false) => $f::<$wrap<V3Rc>>($($a),*),
            (V3Policy::Rd, false) => $f::<$wrap<V3Rd>>($($a),*),
            (V3Policy::Rd2, false) => $f::<$wrap<V3Rd2>>($($a),*),
            (V3Policy::Re, false) => $f::<$wrap<V3Re>>($($a),*),
            (V3Policy::All, false) => $f::<$wrap<V3All>>($($a),*),
            (V3Policy::V2Epoch, true) => $f::<$wrap<WithAdvance<V2Policy, true>>>($($a),*),
            (V3Policy::Ra, true) => $f::<$wrap<WithAdvance<V3Ra, true>>>($($a),*),
            (V3Policy::Rb, true) => $f::<$wrap<WithAdvance<V3Rb, true>>>($($a),*),
            (V3Policy::Rc, true) => $f::<$wrap<WithAdvance<V3Rc, true>>>($($a),*),
            (V3Policy::Rd, true) => $f::<$wrap<WithAdvance<V3Rd, true>>>($($a),*),
            (V3Policy::Rd2, true) => $f::<$wrap<WithAdvance<V3Rd2, true>>>($($a),*),
            (V3Policy::Re, true) => $f::<$wrap<WithAdvance<V3Re, true>>>($($a),*),
            (V3Policy::All, true) => $f::<$wrap<WithAdvance<V3All, true>>>($($a),*),
        }
    };
}

/// The identity wrapper (timed rows run the policy itself).
type Plain<P> = P;

/// A v3 region's shape.
#[derive(Clone, Copy, Debug)]
struct Shape3 {
    participants: u32,
    /// Schedule items (stage executions) per region.
    stages: u32,
    /// Distinct entries the items cycle over (item k executes entry k mod `entries`). Equal to
    /// `stages` by default (`--entries stages`): every item is its own entry, run once per region,
    /// v2's timed shape. Fewer (`--entries E`): an entry re-executes within a region the way a
    /// colour does across a step's passes, and only its first execution takes the retry.
    entries: u32,
    /// Blocks per entry, `blocks_per_participant × participants`.
    blocks: u32,
    work_iters: u32,
    /// A serial gap before every stage, as an inline item on participant 0 (v2's `--gap-us`).
    gap: Duration,
}

/// The caller-owned frame and table of one v3 shape (the plain lines `RegionFrame` borrows).
struct Frame3 {
    sync: RegionLine,
    done: Vec<RegionLine>,
    receipts: Vec<RegionLine>,
    claims: Vec<RegionLine>,
    entries: Vec<StageEntry>,
    schedule: Vec<SchedItem>,
    epoch: u64,
}

/// v3's stages: every block of entry `e` runs v2's work on its own payload line (`first_cut` is
/// the entry's first payload line); the gap entry busy-waits on participant 0.
struct Stages3 {
    payload: Box<[Line<[AtomicU64; 8]>]>,
    first: Vec<u32>,
    work_iters: u32,
    gap: Duration,
}

/// `first_cut` of the gap entry.
const GAP_ENTRY: u32 = u32::MAX;

impl RegionStages for Stages3 {
    fn run_block(&self, entry: u32, block: u32, _participant: u32) {
        let first = self.first[entry as usize];
        if first == GAP_ENTRY {
            let t = Instant::now();
            while t.elapsed() < self.gap {
                spin_loop();
            }
        } else {
            run_work(&self.payload[(first + block) as usize], self.work_iters);
        }
    }
}

impl Frame3 {
    /// The frame and stages of `s` under policy `P`.
    fn new<P: RegionPolicy>(s: Shape3) -> (Self, Stages3) {
        let p = s.participants;
        let n = s.blocks.max(2) as u16;
        let mut entries = Vec::new();
        let mut first_claim = 0u32;
        let mut first = Vec::new();
        for e in 0..s.entries {
            entries.push(StageEntry::new(0, e as u16, n, first_claim, e * u32::from(n)));
            first_claim += claim_lines::<P>(n, p);
            first.push(e * u32::from(n));
        }
        let gap_entry = s.entries as u16;
        if !s.gap.is_zero() {
            entries.push(StageEntry::new(0, gap_entry, 1, first_claim, GAP_ENTRY));
            first.push(GAP_ENTRY);
        }
        let mut schedule = Vec::new();
        for k in 0..s.stages {
            if !s.gap.is_zero() {
                schedule.push(SchedItem::new(gap_entry, None));
            }
            schedule.push(SchedItem::new((k % s.entries) as u16, None));
        }
        let mut last = vec![0u32; entries.len()];
        link_hints(&mut schedule, &mut last);
        let frame = Self {
            sync: RegionLine::ZERO,
            done: vec![RegionLine::ZERO; entries.len()],
            receipts: vec![RegionLine::ZERO; p as usize],
            claims: vec![RegionLine::ZERO; first_claim as usize],
            entries,
            schedule,
            epoch: 0,
        };
        let stages = Stages3 {
            payload: lines((s.entries * u32::from(n)) as usize, || [const { AtomicU64::new(0) }; 8]),
            first,
            work_iters: s.work_iters,
            gap: s.gap,
        };
        (frame, stages)
    }

    /// Blocks every scheduled execution runs.
    fn expected_blocks(&self) -> u64 {
        self.schedule.iter().map(|it| u64::from(self.entries[usize::from(it.entry)].n_blocks)).sum()
    }

    /// One region of this frame on `pool` (participant 0 = the calling thread).
    fn run<P: RegionPolicy>(&mut self, pool: &PoolInner, stages: &Stages3, participants: u32) -> RegionReport {
        // SAFETY: `Frame3::new` built these four line groups and the counter together, as zero
        // lines and a zero counter, and they are private to this `Frame3`: nothing outside it reads
        // or writes them, no other frame is built over them, and the counter only grows (through
        // `open`). They are sized for its one table: one done line per entry, one receipt line per
        // `s.participants`, and `claim_lines::<P>` claim lines per entry at `s.participants`; both
        // callers pass that same `s.participants` and the same `P` the frame was built for. The
        // groups are borrowed `&mut` from `self` for the frame's lifetime.
        let frame = unsafe {
            RegionFrame::new(
                RegionLines {
                    sync: &mut self.sync,
                    done: &mut self.done,
                    receipts: &mut self.receipts,
                    claims: &mut self.claims,
                },
                &self.entries,
                &self.schedule,
                &mut self.epoch,
                participants,
            )
        };
        pool.region::<Stages3, P, false>(frame, stages)
    }

    /// The structural receipt of a completed region (panics on a violation, timed rows included):
    /// every block of every execution ran exactly once (the receipts' blocks sum), every receipt
    /// reads END with this region's tag, and every claim word of an entry holds that entry's last
    /// epoch of the region (the epoch analogue of v2's "every claim word at its execution count").
    fn check<P: RegionPolicy>(&self, r: &RegionReport, participants: u32, what: &str) {
        let receipts: Vec<RegionReceipt> = self.receipts.iter().map(RegionReceipt::read).collect();
        let total: u64 = receipts.iter().map(|rc| rc.blocks).sum();
        assert_eq!(total, self.expected_blocks(), "v3 exactly-once: {what}: {receipts:?}");
        for (q, rc) in receipts.iter().enumerate() {
            assert_eq!(
                (rc.region, rc.exit),
                (r.base, Some(boyko_threadpool::RegionExit::End)),
                "v3 receipt: {what}: participant {q}"
            );
        }
        // The identity on `RegionReport::advances` (in release too): every published item's epoch
        // and the END were each published once. Under the finisher `published` is a table walk, so
        // this is what witnesses that the schedule ran.
        let advances: u64 = receipts.iter().map(|rc| rc.advances).sum();
        assert_eq!(advances, r.advances, "v3 advances: {what}: Σ receipt advances == report.advances");
        assert_eq!(r.advances, u64::from(r.published) + 1, "v3 advances: {what}: Σ advances == published + 1");
        let helper_advances: u64 = receipts.iter().skip(1).map(|rc| rc.advances).sum();
        assert_eq!(r.helper_advances, helper_advances, "v3 advances: {what}: helper_advances == Σ over participants 1..");
        if P::ADVANCE == Advance::Orchestrator {
            assert_eq!(r.helper_advances, 0, "v3 advances: {what}: a helper advanced under the orchestrator");
        }
        for (e, entry) in self.entries.iter().enumerate() {
            if entry.n_blocks < 2 {
                continue;
            }
            let last = self.schedule.iter().rposition(|it| usize::from(it.entry) == e);
            let Some(last) = last else { continue };
            let want = r.base + 1 + last as u64;
            let lines = claim_lines::<P>(entry.n_blocks, participants) as usize;
            let first = entry.first_claim as usize;
            let words: u64 = if P::HOME_LINES {
                self.claims[first..first + lines]
                    .iter()
                    .flat_map(|l| (0..8).map(move |w| l.word(w)))
                    .filter(|&v| v == want)
                    .count() as u64
            } else {
                self.claims[first..first + lines].iter().filter(|l| l.word(0) == want).count() as u64
            };
            assert_eq!(words, u64::from(entry.n_blocks), "v3: {what}: entry {e}'s claim words at its last epoch");
        }
    }
}

/// How a v3 row maps its schedule items onto stage entries (`--entries`; module docs, "The
/// schedule's shape").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entries {
    /// One entry per item — v2's timed shape, M3 check (1)'s comparator (`--entries stages`).
    PerItem,
    /// The items cycle over this many entries (`--entries E`).
    Cycle(u32),
}

impl Entries {
    /// The entry count of a row with `stages` items.
    fn resolve(self, stages: u32) -> u32 {
        match self {
            Self::PerItem => stages,
            Self::Cycle(e) => e,
        }
    }
}

/// The largest entry count a v3 frame takes: entry indices are `u16`, and the gap entry takes the
/// index after the last stage entry.
const V3_MAX_ENTRIES: u32 = u16::MAX as u32 - 1;

/// `--mode region`'s arguments.
struct ArgsV3 {
    participants: Vec<u32>,
    route: Route,
    regions: u32,
    stages: Vec<u32>,
    entries: Entries,
    blocks_per_participant: Vec<u32>,
    policies: Vec<V3Arm>,
    work_ns: u64,
    gaps_us: Vec<u64>,
    /// `--plan`: print the cells, run nothing.
    plan: bool,
}

/// Whether the command line selects v3 (`--bench --mode region`).
fn is_v3(raw: &[String]) -> bool {
    raw.iter().any(|a| a == "--bench") && raw.windows(2).any(|w| w[0] == "--mode" && w[1] == "region")
}

fn parse_args_v3(raw: &[String]) -> Result<ArgsV3, String> {
    let mut a = ArgsV3 {
        participants: vec![2, 4, 8, 16],
        route: Route::Worker,
        regions: 1000,
        stages: vec![36],
        entries: Entries::PerItem,
        blocks_per_participant: vec![1, 2, 4, 6],
        policies: V3Arm::ALL8.to_vec(),
        work_ns: 700,
        gaps_us: vec![0],
        plan: false,
    };
    let mut it = raw.iter().cloned();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--bench" => {}
            "--mode" => {
                let _ = it.next();
            }
            "--participants" => a.participants = parse_list("--participants", it.next())?,
            "--route" => {
                a.route = match it.next().as_deref() {
                    Some("worker") => Route::Worker,
                    Some("external") => Route::External,
                    other => return Err(format!("--route: expected worker|external, got {other:?}")),
                }
            }
            "--regions" => a.regions = parse_one("--regions", it.next())?,
            "--stages" => a.stages = parse_list("--stages", it.next())?,
            "--entries" => {
                a.entries = match it.next() {
                    Some(v) if v == "stages" => Entries::PerItem,
                    v => Entries::Cycle(parse_one("--entries", v)?),
                }
            }
            "--plan" => a.plan = true,
            "--bpp" | "--blocks-per-participant" => a.blocks_per_participant = parse_list("--bpp", it.next())?,
            "--policy" => {
                let names: Vec<String> = parse_list("--policy", it.next())?;
                a.policies = names
                    .iter()
                    .map(|n| {
                        V3Arm::parse(n).ok_or_else(|| {
                            format!("--policy: unknown {n:?} (v2epoch|Ra|Rb|Rc|Rd|Rd2|Re|all, each optionally +fin)")
                        })
                    })
                    .collect::<Result<_, _>>()?;
            }
            "--work-ns" => a.work_ns = parse_one("--work-ns", it.next())?,
            "--gap-us" => a.gaps_us = parse_list("--gap-us", it.next())?,
            other => return Err(format!("region: unknown argument {other:?}")),
        }
    }
    if a.participants.iter().any(|&p| p == 0 || p > 16) {
        return Err("region: participants must be 1..=16".into());
    }
    if a.blocks_per_participant.iter().any(|&b| b == 0 || b > REGION_MAX_BLOCKS_PER_PARTICIPANT) {
        return Err(format!("region: blocks per participant must be 1..={REGION_MAX_BLOCKS_PER_PARTICIPANT}"));
    }
    if a.stages.contains(&0) || a.entries == Entries::Cycle(0) || a.regions == 0 || a.work_ns == 0 || a.policies.is_empty() {
        return Err("region: stages, entries, regions, work ns and the policy list must be non-empty".into());
    }
    if a.stages.iter().any(|&s| a.entries.resolve(s) > V3_MAX_ENTRIES) {
        return Err(format!("region: at most {V3_MAX_ENTRIES} entries (with --entries stages, at most that many stages)"));
    }
    Ok(a)
}

/// `--mode region`: the calibration, then every row; under `--plan`, only the cells (one loop nest
/// serves both, so the plan is exactly what a run would execute).
fn run_v3(a: &ArgsV3) {
    let (work_iters, work_ns_calibrated) = if a.plan { (0, 0.0) } else { calibrate(a.work_ns) };
    let mut cells = 0usize;
    for &p in &a.participants {
        let pool = (!a.plan).then(|| ThreadPoolBuilder::new().num_threads(p as usize).build());
        for &stages in &a.stages {
            for &bpp in &a.blocks_per_participant {
                for &policy in &a.policies {
                    for &gap_us in &a.gaps_us {
                        let shape = Shape3 {
                            participants: p,
                            stages,
                            entries: a.entries.resolve(stages),
                            blocks: bpp * p,
                            work_iters,
                            gap: Duration::from_micros(gap_us),
                        };
                        cells += 1;
                        match &pool {
                            Some(pool) => with_v3_policy!(
                                policy,
                                Plain,
                                omega_b3_row(pool, a, shape, policy, bpp, gap_us, work_ns_calibrated)
                            ),
                            None => println!(
                                "PLAN {{\"bench\":\"omega_b3\",\"version\":3,\"route\":\"{}\",\"policy\":\"{}\",\
                                 \"participants\":{p},\"stages\":{stages},\"entries\":{},\"blocks\":{},\
                                 \"blocks_per_participant\":{bpp},\"gap_us\":{gap_us},\"regions\":{}}}",
                                a.route.name(),
                                policy.name(),
                                shape.entries,
                                shape.blocks,
                                a.regions,
                            ),
                        }
                    }
                }
            }
        }
    }
    if a.plan {
        println!("PLAN_CELLS {cells}");
    }
}

/// One v3 row: `regions` regions of one shape and policy, each timed as a whole on the
/// orchestrating thread and its receipt checked outside the timing.
fn omega_b3_row<P: RegionPolicy + 'static>(
    pool: &ThreadPool,
    a: &ArgsV3,
    s: Shape3,
    policy: V3Arm,
    bpp: u32,
    gap_us: u64,
    work_ns_calibrated: f64,
) {
    let (frame, stages) = Frame3::new::<P>(s);
    let stages = Arc::new(stages);
    let n = a.regions as usize;
    let (mut walls, mut helper_blocks) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut helper_advances = Vec::with_capacity(n);
    let mut frame = Some(frame);
    for _ in 0..a.regions {
        let (mut f, st) = (frame.take().expect("invariant: the frame returns each region"), Arc::clone(&stages));
        let ((f, wall, r), id) = on_route(pool, a.route, move |inner| {
            let t0 = Instant::now();
            let r = f.run::<P>(inner, &st, s.participants);
            let wall = t0.elapsed().as_nanos() as u64;
            (f, wall, r)
        });
        assert_route(a.route, id, s.participants as usize);
        f.check::<P>(&r, s.participants, "a timed row");
        walls.push(wall);
        helper_blocks.push(r.helper_blocks);
        helper_advances.push(r.helper_advances);
        frame = Some(f);
    }
    let mean = walls.iter().sum::<u64>() as f64 / walls.len() as f64;
    let wall_median = median(&mut walls).unwrap_or(0);
    let helped = helper_blocks.iter().filter(|&&b| b > 0).count();
    let advanced = helper_advances.iter().filter(|&&v| v > 0).count();
    println!(
        "SUMMARY {{\"bench\":\"omega_b3\",\"version\":3,\"route\":\"{}\",\"policy\":\"{}\",\"participants\":{},\
         \"stages\":{},\"entries\":{},\"blocks\":{},\"blocks_per_participant\":{bpp},\"gap_us\":{gap_us},\
         \"regions\":{},\"work_iters\":{},\"work_ns_calibrated\":{work_ns_calibrated:.1},\
         \"region_ns_median\":{wall_median},\"region_ns_mean\":{mean},\"stage_ns_median\":{},\"exactly_once\":true,\
         \"helped_regions\":{helped},\"helper_blocks_median\":{},\"helper_advanced_regions\":{advanced},\"helper_advances_median\":{}}}",
        a.route.name(),
        policy.name(),
        s.participants,
        s.stages,
        s.entries,
        s.blocks,
        a.regions,
        s.work_iters,
        wall_median as f64 / f64::from(s.stages),
        median(&mut helper_blocks).unwrap_or(0),
        median(&mut helper_advances).unwrap_or(0),
    );
}

/// v3's untimed self-check: every policy at 2, 4, 8 and 16 participants, 1 and 6 blocks per
/// participant, both routes, with and without a gap, 12 items over 3 entries and over 12 (one entry
/// per item, the timed rows' default shape); three regions each over one frame (so the second and
/// third take the first-execution retry on words the previous region wrote); exactly once, every
/// receipt, every claim word at its entry's last epoch. Bounded waits.
fn self_check_v3() {
    println!("omega_b_region: v3 (region.rs) self-check");
    let mut helped = 0usize;
    // Per arm (in `V3Arm::ALL16` order): regions in which a helper made a publish.
    let mut advanced = [0usize; 16];
    for route in [Route::External, Route::Worker] {
        for participants in [2u32, 4, 8, 16] {
            let pool = ThreadPoolBuilder::new().num_threads(participants as usize).build();
            for (k, policy) in V3Arm::ALL16.into_iter().enumerate() {
                for bpp in [1u32, 6] {
                    for gap in [Duration::ZERO, Duration::from_micros(20)] {
                        for entries in [3u32, 12] {
                            let shape = Shape3 {
                                participants,
                                stages: 12,
                                entries,
                                blocks: bpp * participants,
                                work_iters: SELF_CHECK_WORK_ITERS,
                                gap,
                            };
                            let (h, adv) =
                                with_v3_policy!(policy, Bounded, self_check_v3_shape(&pool, route, shape, policy));
                            helped += h;
                            advanced[k] += adv;
                        }
                    }
                }
            }
            println!("  v3 region: {participants} participants, {} route, every policy (16 arms): ok", route.name());
        }
    }
    // Anti-vacuity: the helpers took part somewhere, or "exactly once" was only participant 0's.
    assert!(helped > 0, "v3: no helper ran a block in the whole self-check");
    println!("  v3: {helped} regions with helper blocks");
    // Every `+fin` arm ran the finisher: a helper advanced in some region of its cells (a
    // `Bounded` that dropped `ADVANCE` would run them all as the orchestrator, and read 0 here).
    for (k, arm) in V3Arm::ALL16.into_iter().enumerate() {
        if arm.fin {
            assert!(advanced[k] > 0, "v3: no region of {} had a helper advance", arm.name());
        } else {
            assert_eq!(advanced[k], 0, "v3: {} (the orchestrator) had a helper advance", arm.name());
        }
    }
    println!(
        "  v3: regions with a helper advance per +fin arm: {}",
        V3Arm::ALL16
            .into_iter()
            .enumerate()
            .filter(|(_, a)| a.fin)
            .map(|(k, a)| format!("{} {}", a.name(), advanced[k]))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("omega_b_region v3 self-check: ok");
}

/// Three regions of one shape and policy; returns how many had helper blocks and how many had a
/// helper advance.
fn self_check_v3_shape<P: RegionPolicy + 'static>(pool: &ThreadPool, route: Route, s: Shape3, policy: V3Arm) -> (usize, usize) {
    let (frame, stages) = Frame3::new::<P>(s);
    let stages = Arc::new(stages);
    let what = format!(
        "{} {} participants, {} route, {} blocks, {} items over {} entries, gap {:?}",
        policy.name(),
        s.participants,
        route.name(),
        s.blocks,
        s.stages,
        s.entries,
        s.gap
    );
    let mut frame = Some(frame);
    let (mut helped, mut advanced) = (0, 0);
    for _ in 0..3 {
        let (mut f, st) = (frame.take().expect("invariant: the frame returns each region"), Arc::clone(&stages));
        let ((f, r), id) = on_route(pool, route, move |inner| {
            let r = f.run::<P>(inner, &st, s.participants);
            (f, r)
        });
        assert_route(route, id, s.participants as usize);
        f.check::<P>(&r, s.participants, &what);
        helped += usize::from(r.helper_blocks > 0);
        advanced += usize::from(r.helper_advances > 0);
        frame = Some(f);
    }
    (helped, advanced)
}
