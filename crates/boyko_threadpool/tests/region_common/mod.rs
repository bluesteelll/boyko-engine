//! Shared harness of the region test binaries (`region_protocol.rs`, `region_panic.rs`,
//! `region_nested_scope.rs`): a caller-owned frame, a table builder, a configurable
//! [`RegionStages`] with the exactly-once / visibility instruments, the two routes, and the
//! policies the tests run.
//!
//! Each binary uses a subset, hence the file-level `dead_code` allowance.
#![allow(dead_code)]

use std::cell::UnsafeCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use boyko_threadpool::{
    Ladder, PoolInner, RegionFrame, RegionLine, RegionLines, RegionPolicy, RegionReceipt,
    RegionReport, RegionStages, SchedItem, StageEntry, ThreadPool, ThreadPoolBuilder,
    claim_lines, current_worker_id, link_hints, try_with_active_pool,
};

/// The bound every test policy carries: a protocol defect panics with `RegionWaitBound` instead
/// of hanging. Miri's wall clock does not measure the protocol, so it gets a long one.
pub const TEST_BOUND_NS: u64 = if cfg!(miri) { 120_000_000_000 } else { 2_000_000_000 };

/// The most blocks any test entry has per participant.
pub const MAX_BPP: u32 = 8;

/// Slots of the generation instrument (each block of an item owns a contiguous run of them).
pub const SLOTS: usize = if cfg!(miri) { 24 } else { 256 };

// ---------------------------------------------------------------------------
// Policies
// ---------------------------------------------------------------------------

macro_rules! policy {
    ($(#[$m:meta])* $name:ident, helper = $h:expr, orch = $o:expr, bound = $b:expr,
     batched = $ba:expr, home = $ho:expr, ttas = $t:expr) => {
        $(#[$m])*
        pub struct $name;
        impl RegionPolicy for $name {
            const HELPER_WAIT: Ladder = $h;
            const ORCH_WAIT: Ladder = $o;
            const BOUND_NS: u64 = $b;
            const DONE_BATCHED: bool = $ba;
            const HOME_LINES: bool = $ho;
            const TTAS: bool = $t;
        }
    };
}

policy!(
    /// v2's ladders and protocol with the test bound (the shipped `V2Policy` plus a bound).
    TestPolicy, helper = Ladder::PauseThenYield { pauses: 5 }, orch = Ladder::PureSpin,
    bound = TEST_BOUND_NS, batched = false, home = false, ttas = false
);
policy!(
    /// R-a: batched completion.
    BatchedPolicy, helper = Ladder::PauseThenYield { pauses: 5 }, orch = Ladder::PureSpin,
    bound = TEST_BOUND_NS, batched = true, home = false, ttas = false
);
policy!(
    /// R-b: home claim lines.
    HomePolicy, helper = Ladder::PauseThenYield { pauses: 5 }, orch = Ladder::PureSpin,
    bound = TEST_BOUND_NS, batched = false, home = true, ttas = false
);
policy!(
    /// R-c: TTAS claims.
    TtasPolicy, helper = Ladder::PauseThenYield { pauses: 5 }, orch = Ladder::PureSpin,
    bound = TEST_BOUND_NS, batched = false, home = false, ttas = true
);
policy!(
    /// Every axis on, both budget ladders.
    AllAxesPolicy, helper = Ladder::BudgetThenYield { spin_ns: 2_000 },
    orch = Ladder::BudgetThenYield { spin_ns: 20_000 }, bound = TEST_BOUND_NS,
    batched = true, home = true, ttas = true
);
policy!(
    /// Helpers PAUSE only (R-d′).
    PureSpinHelpersPolicy, helper = Ladder::PureSpin, orch = Ladder::PureSpin,
    bound = TEST_BOUND_NS, batched = false, home = false, ttas = false
);

// ---------------------------------------------------------------------------
// The frame
// ---------------------------------------------------------------------------

/// A caller-owned region frame: plain lines plus the table and the epoch counter.
pub struct Frame {
    pub sync: RegionLine,
    pub done: Vec<RegionLine>,
    pub receipts: Vec<RegionLine>,
    pub claims: Vec<RegionLine>,
    pub entries: Vec<StageEntry>,
    pub schedule: Vec<SchedItem>,
    pub epoch: u64,
}

impl Frame {
    /// A zero frame with room for `done_lines` entries, `receipt_lines` participants and
    /// `claim_lines` claim lines.
    pub fn new(done_lines: usize, receipt_lines: usize, claim_lines: usize) -> Self {
        Self {
            sync: RegionLine::ZERO,
            done: vec![RegionLine::ZERO; done_lines],
            receipts: vec![RegionLine::ZERO; receipt_lines],
            claims: vec![RegionLine::ZERO; claim_lines],
            entries: Vec::new(),
            schedule: Vec::new(),
            epoch: 0,
        }
    }

    /// Installs a table: entry `e` has `blocks[e]` blocks, the schedule executes `order`. Claim
    /// lines are laid out from `claim_offset` for policy `P` at `participants`; the hints are
    /// linked. Panics if the frame is too small (the test's own bug).
    pub fn set_table<P: RegionPolicy>(
        &mut self,
        blocks: &[u16],
        order: &[u16],
        participants: u32,
        claim_offset: u32,
    ) {
        let mut first = claim_offset;
        self.entries = blocks
            .iter()
            .enumerate()
            .map(|(e, &n)| {
                let entry = StageEntry::new(0, e as u16, n, first, 0);
                first += claim_lines::<P>(n, participants);
                entry
            })
            .collect();
        assert!(first as usize <= self.claims.len(), "test frame: {first} claim lines needed");
        assert!(blocks.len() <= self.done.len(), "test frame: too few done lines");
        self.schedule = order.iter().map(|&e| SchedItem::new(e, None)).collect();
        let mut last = vec![0u32; blocks.len()];
        link_hints(&mut self.schedule, &mut last);
    }

    /// This frame borrowed as a region frame for a region that runs policy `P` with
    /// `participants` participants.
    ///
    /// The claim layout and the schedule's entries ([`RegionFrame::new`]'s "sized for its
    /// schedule" condition, beyond the receipt and done counts `RegionFrame::new` asserts itself)
    /// are checked here, not assumed: `assert_table_fits` panics unless the installed table is
    /// laid out for `P` at `participants`. A table laid out by `set_table` for another policy or
    /// participant count than the region runs is the test's own bug.
    ///
    /// # Safety
    ///
    /// [`RegionFrame::new`]'s storage contract, for this `Frame`:
    ///
    /// * its line columns and its `epoch` have stayed this `Frame`'s own since [`Frame::new`]
    ///   built them together — no column or counter was moved in from, or shared with, another
    ///   `Frame`, and the counter was never rewound;
    /// * the region the returned frame is given to runs `P`: the layout is checked for `P` only,
    ///   so a region under another policy could run over claim lines not laid out for it.
    ///
    /// Rewriting a line's CONTENTS between regions (as the F-FRAME failure-B test does with the
    /// sync line) keeps the contract: the storage is still this `Frame`'s alone.
    pub unsafe fn region_frame<P: RegionPolicy>(&mut self, participants: u32) -> RegionFrame<'_> {
        self.assert_table_fits::<P>(participants);
        // SAFETY: the caller guarantees the columns and the counter are still this Frame's own and
        // that the region runs `P` (above). `Frame::new` built them together, as zero lines and a
        // zero counter, so every claim epoch starts at or below the counter, and `open` only ever
        // raises the counter past what a region writes. `assert_table_fits::<P>` (above) checked
        // that every entry's claim lines for `P` at `participants` lie inside the claim column and
        // belong to that entry alone, and that every item names an entry of the table;
        // `RegionFrame::new` itself asserts the receipt and done line counts. The four groups are
        // borrowed `&mut` from `self` for the frame's lifetime, so no other frame is over them
        // while it lives.
        unsafe {
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
        }
    }

    /// Panics unless the installed table is laid out for policy `P` at `participants`: every
    /// entry's [`claim_lines`] lines from its `first_claim` lie inside the claim column and share
    /// no line with another entry's, and every scheduled item names an entry of the table.
    fn assert_table_fits<P: RegionPolicy>(&self, participants: u32) {
        let mut ranges: Vec<(usize, usize, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(e, entry)| {
                let need = claim_lines::<P>(entry.n_blocks, participants) as usize;
                let first = entry.first_claim as usize;
                (need > 0).then_some((first, first + need, e))
            })
            .collect();
        ranges.sort_unstable();
        for &(first, end, e) in &ranges {
            assert!(
                end <= self.claims.len(),
                "test frame: entry {e}'s claim lines {first}..{end} pass the column's {}",
                self.claims.len()
            );
        }
        // Sorted by first line, the ranges are pairwise disjoint iff each ends before the next.
        for pair in ranges.windows(2) {
            let ((first_a, end_a, a), (first_b, end_b, b)) = (pair[0], pair[1]);
            assert!(
                end_a <= first_b,
                "test frame: entries {a} ({first_a}..{end_a}) and {b} ({first_b}..{end_b}) share claim \
                 lines under this policy at {participants} participants — the table was laid out for \
                 another policy or participant count than the region runs"
            );
        }
        for (i, item) in self.schedule.iter().enumerate() {
            assert!(
                usize::from(item.entry) < self.entries.len(),
                "test frame: item {i} names entry {} of {}",
                item.entry,
                self.entries.len()
            );
        }
    }

    /// The first `participants` receipts.
    pub fn receipts(&self, participants: u32) -> Vec<RegionReceipt> {
        self.receipts[..participants as usize].iter().map(RegionReceipt::read).collect()
    }

    /// Blocks every scheduled execution runs (inline items count one).
    pub fn expected_blocks(&self) -> u64 {
        self.schedule.iter().map(|it| u64::from(self.entries[usize::from(it.entry)].n_blocks)).sum()
    }

    /// Executions of entry `e` in the schedule.
    pub fn execs(&self, e: usize) -> u32 {
        self.schedule.iter().filter(|it| usize::from(it.entry) == e).count() as u32
    }
}

// ---------------------------------------------------------------------------
// The stages
// ---------------------------------------------------------------------------

/// One plain (non-atomic) slot of the generation instrument.
pub struct Slot(UnsafeCell<u64>);

// SAFETY: a slot is written only by the one block of an item that owns it, and items never
// overlap (the region's barrier orders every write of item i before every block of item i + 1).
// That ordering is exactly what the tests check: under Miri a missing happens-before is reported
// as a data race on these slots.
unsafe impl Sync for Slot {}

/// Which participants an injection fires on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    /// Participant 0 only.
    Orchestrator,
    /// Any participant but 0.
    Helper,
}

/// A one-shot panic injection: the first run of `(entry, block)` on a matching participant
/// panics with [`Injected`].
#[derive(Clone, Copy, Debug)]
pub struct Inject {
    pub entry: u32,
    pub block: u32,
    pub who: Who,
}

/// The payload of an injected panic.
#[derive(Debug, PartialEq, Eq)]
pub struct Injected {
    pub participant: u32,
}

/// A rendezvous: block 0 of `entry` (always participant 0's first claim) waits, bounded, until
/// some helper has started block 1; block 1 then holds for `hold`.
#[derive(Clone, Copy, Debug)]
pub struct Rendezvous {
    pub entry: u32,
    pub hold: Duration,
    /// Sleep (true) or spin (false) through the hold.
    pub sleep: bool,
}

/// The configurable stages.
pub struct Stages {
    /// Blocks per entry, copied from the table (the slot partition needs it).
    blocks: Vec<u16>,
    /// The generation instrument: every block of an item reads its slots (all must hold the same
    /// generation) and writes generation + 1.
    slots: Box<[Slot]>,
    /// Generation mismatches seen (a block that ran twice, was skipped, or saw stale data).
    pub mismatches: AtomicU32,
    /// Runs per (entry, block): `entry * stride + block`.
    runs: Box<[AtomicU32]>,
    stride: usize,
    /// Armed hooks: the current item (participant 0 stores it before the item).
    cur_item: AtomicU32,
    /// Armed hooks: runs per (item, block).
    item_runs: Box<[AtomicU32]>,
    /// Participant 0 spins this long in every block of `slow_entry`.
    slow: Option<(u32, Duration)>,
    /// Every block spins this long (so helpers have blocks left to claim when they arrive).
    work: Duration,
    inject: Option<Inject>,
    pub fired: AtomicBool,
    rendezvous: Option<Rendezvous>,
    started_b1: AtomicBool,
    /// Participant 0's block 0 of this entry waits (bounded) until a helper has started a block of it.
    await_helper: Option<u32>,
    helper_started: AtomicBool,
    /// A block of this entry opens a nested pool scope (the A4 debug guard's test).
    nested_scope_entry: Option<u32>,
    pub nested_returned: AtomicU32,
}

impl Stages {
    /// Stages over `frame`'s current table.
    pub fn new(frame: &Frame) -> Self {
        let blocks: Vec<u16> = frame.entries.iter().map(|e| e.n_blocks).collect();
        let stride = blocks.iter().copied().max().unwrap_or(1).max(1) as usize;
        Self {
            slots: (0..SLOTS).map(|_| Slot(UnsafeCell::new(0))).collect(),
            mismatches: AtomicU32::new(0),
            runs: (0..blocks.len() * stride).map(|_| AtomicU32::new(0)).collect(),
            stride,
            cur_item: AtomicU32::new(u32::MAX),
            item_runs: (0..frame.schedule.len() * stride).map(|_| AtomicU32::new(0)).collect(),
            blocks,
            slow: None,
            work: Duration::ZERO,
            inject: None,
            fired: AtomicBool::new(false),
            rendezvous: None,
            started_b1: AtomicBool::new(false),
            await_helper: None,
            helper_started: AtomicBool::new(false),
            nested_scope_entry: None,
            nested_returned: AtomicU32::new(0),
        }
    }

    /// Participant 0 spins `d` in every block of `entry`.
    pub fn with_slow(mut self, entry: u32, d: Duration) -> Self {
        self.slow = Some((entry, d));
        self
    }

    /// Every block spins `d`.
    pub fn with_work(mut self, d: Duration) -> Self {
        self.work = d;
        self
    }

    /// One-shot panic injection.
    pub fn with_inject(mut self, inject: Inject) -> Self {
        self.inject = Some(inject);
        self
    }

    /// A rendezvous on `r.entry`.
    pub fn with_rendezvous(mut self, r: Rendezvous) -> Self {
        self.rendezvous = Some(r);
        self
    }

    /// Participant 0's block 0 of `entry` waits (bounded, 1 s) until a helper has started a block
    /// of it: the helpers are then certain to reach the blocks after participant 0's position.
    pub fn with_await_helper(mut self, entry: u32) -> Self {
        self.await_helper = Some(entry);
        self
    }

    /// A block of `entry` opens a nested `pool.scope` (participant 0's block 0 only).
    pub fn with_nested_scope(mut self, entry: u32) -> Self {
        self.nested_scope_entry = Some(entry);
        self
    }

    /// Runs of `(entry, block)`.
    pub fn runs(&self, e: usize, b: usize) -> u32 {
        self.runs[e * self.stride + b].load(Ordering::Relaxed)
    }

    /// Armed only: runs of `(item, block)`.
    pub fn item_runs(&self, i: usize, b: usize) -> u32 {
        self.item_runs[i * self.stride + b].load(Ordering::Relaxed)
    }

    /// The generation every slot holds after the region (all equal, or `None`).
    pub fn final_generation(&self) -> Option<u64> {
        // SAFETY: read after the region returned (or after its panic was caught): every block
        // has finished and the scope's join ordered its writes before this read.
        let first = unsafe { *self.slots[0].0.get() };
        // SAFETY: as above.
        self.slots.iter().all(|s| unsafe { *s.0.get() } == first).then_some(first)
    }

    /// Asserts every block of every execution ran exactly once and every slot holds `items`.
    pub fn assert_exactly_once(&self, frame: &Frame, what: &str) {
        assert_eq!(self.mismatches.load(Ordering::Relaxed), 0, "{what}: generation mismatches");
        for (e, &n) in self.blocks.iter().enumerate() {
            let execs = frame.execs(e);
            for b in 0..usize::from(n) {
                assert_eq!(self.runs(e, b), execs, "{what}: entry {e} block {b} ran a wrong number of times");
            }
        }
        assert_eq!(
            self.final_generation(),
            Some(frame.schedule.len() as u64),
            "{what}: the slots do not all hold the item count"
        );
    }

    fn generation(&self, e: u32, b: u32) {
        let n = usize::from(self.blocks[e as usize]);
        let (lo, hi) = (b as usize * SLOTS / n, (b as usize + 1) * SLOTS / n);
        // SAFETY: slots [lo, hi) belong to this block of this item alone (`Slot`'s Sync
        // argument); the region's barrier orders the previous item's writes before these reads.
        let g0 = unsafe { *self.slots[lo].0.get() };
        let mut ok = true;
        for s in &self.slots[lo..hi] {
            // SAFETY: as above.
            let v = unsafe { *s.0.get() };
            ok &= v == g0;
            // SAFETY: as above.
            unsafe { *s.0.get() = g0 + 1 };
        }
        if !ok {
            self.mismatches.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Spins (not sleeps) for `d`.
pub fn spin_for(d: Duration) {
    let t = Instant::now();
    while t.elapsed() < d {
        std::hint::spin_loop();
    }
}

impl RegionStages for Stages {
    fn run_block(&self, entry: u32, block: u32, participant: u32) {
        if self.await_helper == Some(entry) {
            if participant != 0 {
                self.helper_started.store(true, Ordering::Release);
            } else if block == 0 {
                let t = Instant::now();
                while !self.helper_started.load(Ordering::Acquire) && t.elapsed() < Duration::from_secs(1) {
                    std::hint::spin_loop();
                }
            }
        }
        if let Some(inj) = self.inject {
            let who_ok = match inj.who {
                Who::Orchestrator => participant == 0,
                Who::Helper => participant != 0,
            };
            if inj.entry == entry
                && inj.block == block
                && who_ok
                && self.fired.compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed).is_ok()
            {
                std::panic::panic_any(Injected { participant });
            }
        }
        if let Some(r) = self.rendezvous
            && r.entry == entry
        {
            if block == 0 {
                let t = Instant::now();
                // Spin, not yield: the waiter must leave block 0 within ns of block 1's start, or
                // the hold that follows is partly spent before anyone waits on it.
                while !self.started_b1.load(Ordering::Acquire) && t.elapsed() < Duration::from_secs(1) {
                    std::hint::spin_loop();
                }
            } else if block == 1 {
                self.started_b1.store(true, Ordering::Release);
                if r.sleep {
                    std::thread::sleep(r.hold);
                } else {
                    spin_for(r.hold);
                }
            }
        }
        if let Some((e, d)) = self.slow
            && e == entry
            && participant == 0
        {
            spin_for(d);
        }
        if !self.work.is_zero() {
            spin_for(self.work);
        }
        if self.nested_scope_entry == Some(entry) && participant == 0 && block == 0 {
            try_with_active_pool(|pool| pool.scope(|s| s.spawn(|| {})))
                .expect("test setup: a region block runs inside a pool");
            self.nested_returned.fetch_add(1, Ordering::Relaxed);
        }
        self.generation(entry, block);
        self.runs[entry as usize * self.stride + block as usize].fetch_add(1, Ordering::Relaxed);
        let item = self.cur_item.load(Ordering::Relaxed);
        if item != u32::MAX {
            self.item_runs[item as usize * self.stride + block as usize].fetch_add(1, Ordering::Relaxed);
        }
    }

    fn item_begin(&self, item: u32) {
        // Relaxed: the publish that follows (Release) orders this store before every helper's
        // Acquire of the item's epoch, and no block of this item runs before that publish.
        self.cur_item.store(item, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

/// Where participant 0 runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// On the `install` frame's own thread (the dispatcher).
    External,
    /// In a detached pool task, i.e. on a worker (the physics system's usual route).
    Worker,
}

/// The outcome of one region run on a route.
pub struct Ran {
    pub frame: Frame,
    pub stages: Arc<Stages>,
    pub result: std::thread::Result<RegionReport>,
    /// `current_worker_id()` of participant 0.
    pub orchestrator_id: u32,
    pub wall: Duration,
}

fn run_on<P: RegionPolicy, const ARMED: bool>(
    inner: &PoolInner,
    frame: &mut Frame,
    stages: &Stages,
    participants: u32,
) -> (std::thread::Result<RegionReport>, Duration) {
    let t = Instant::now();
    // Outside `catch_unwind`: a table that does not fit `P` panics the test as a setup bug, never
    // reads as a poisoned region.
    // SAFETY: `frame` is the test's own `Frame`, built by `Frame::new` and moved by value through
    // `run_region` from region to region. No test moves a column or the counter in from another
    // `Frame` or rewinds the counter (tests only read the columns and install tables). The region
    // below runs the same `P` this frame's layout is checked for.
    let frame = unsafe { frame.region_frame::<P>(participants) };
    let r = catch_unwind(AssertUnwindSafe(|| inner.region::<Stages, P, ARMED>(frame, stages)));
    (r, t.elapsed())
}

/// Runs one region of `frame` with `stages` on `route` of `pool`, catching its panic.
pub fn run_region<P: RegionPolicy + 'static, const ARMED: bool>(
    pool: &ThreadPool,
    route: Route,
    mut frame: Frame,
    stages: Arc<Stages>,
    participants: u32,
) -> Ran {
    match route {
        Route::External => {
            let (result, wall, id) = pool.install(|_| {
                let id = current_worker_id();
                let (r, wall) = try_with_active_pool(|inner| {
                    run_on::<P, ARMED>(inner, &mut frame, &stages, participants)
                })
                .expect("test setup: install sets the active pool");
                (r, wall, id)
            });
            Ran { frame, stages, result, orchestrator_id: id, wall }
        }
        Route::Worker => {
            let (tx, rx) = mpsc::channel();
            pool.spawn(move || {
                let id = current_worker_id();
                let (r, wall) = try_with_active_pool(|inner| {
                    run_on::<P, ARMED>(inner, &mut frame, &stages, participants)
                })
                .expect("test setup: a worker has its pool active");
                let _ = tx.send(Ran { frame, stages, result: r, orchestrator_id: id, wall });
            });
            rx.recv().expect("test setup: the worker-route task reports back")
        }
    }
}

/// Miri only: one region on scoped std threads (`region_on_threads`, the pool-free twin the
/// Stacked Borrows leg drives), catching its panic.
#[cfg(miri)]
pub fn run_threads<P: RegionPolicy, const ARMED: bool>(
    mut frame: Frame,
    stages: Arc<Stages>,
    participants: u32,
) -> Ran {
    let t = Instant::now();
    let result = {
        // Outside `catch_unwind`, as in `run_on`.
        // SAFETY: as in `run_on`: `frame` is the test's own `Frame`, moved by value from region
        // to region; no test mixes in another `Frame`'s columns or counter or rewinds the counter.
        // The region below runs the same `P` this frame's layout is checked for.
        let region_frame = unsafe { frame.region_frame::<P>(participants) };
        catch_unwind(AssertUnwindSafe(|| {
            boyko_threadpool::region_on_threads::<Stages, P, ARMED>(region_frame, &stages)
        }))
    };
    Ran { frame, stages, result, orchestrator_id: u32::MAX, wall: t.elapsed() }
}

/// A pool of `n` workers.
pub fn pool(n: u32) -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(n as usize).build()
}

/// Participant counts the tests sweep.
pub fn participant_counts() -> &'static [u32] {
    if cfg!(miri) { &[2, 3] } else { &[1, 2, 3, 4, 5, 8, 16] }
}

/// Participant counts the panic tests sweep.
pub fn panic_participant_counts() -> &'static [u32] {
    if cfg!(miri) { &[2, 3] } else { &[2, 8, 16] }
}

/// A xorshift64* stream, for seeded random tables.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Sends `f`'s result over a channel from a fresh thread and waits at most `limit`: a hung `f`
/// reads `None` (red), never a hung test.
pub fn within<R: Send + 'static>(limit: Duration, f: impl FnOnce() -> R + Send + 'static) -> Option<R> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(limit).ok()
}

/// Counts every helper of a pool held by a gate: `n` detached tasks that each mark themselves
/// started, then spin until `release` is set.
pub fn hold_workers(pool: &ThreadPool, n: u32) -> (Arc<AtomicU32>, Arc<AtomicBool>) {
    let started = Arc::new(AtomicU32::new(0));
    let release = Arc::new(AtomicBool::new(false));
    for _ in 0..n {
        let (s, r) = (Arc::clone(&started), Arc::clone(&release));
        pool.spawn(move || {
            s.fetch_add(1, Ordering::AcqRel);
            while !r.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        });
    }
    let t = Instant::now();
    while started.load(Ordering::Acquire) < n {
        assert!(t.elapsed() < Duration::from_secs(10), "test setup: the gated tasks did not all start");
        std::thread::yield_now();
    }
    (started, release)
}

/// The total of a slice of `AtomicU64`s (helper for bench-shaped tests).
pub fn sum(v: &[AtomicU64]) -> u64 {
    v.iter().map(|a| a.load(Ordering::Relaxed)).sum()
}
