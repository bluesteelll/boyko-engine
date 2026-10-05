//! SR (1): the region protocol's behavioural gates (`02-SR-DESIGN.md`, cut §3 A2).
//!
//! * **T1 exactly-once / T2 visibility** over seeded random tables — all-inline, one wide stage,
//!   a stage run 12×, mixed — at every participant count, on both routes, under every policy axis,
//!   armed and disarmed. The generation instrument (`region_common::Stages`) makes every block of
//!   an item read the slots the previous item's blocks wrote (other participants' blocks whenever
//!   the two items' block counts differ) and write the next generation, so a block that ran twice,
//!   was skipped, or read before the previous item finished is a mismatch — and under Miri a
//!   missing happens-before is a reported data race on those plain slots.
//! * **T3** the report formulas.
//! * **T4** work conservation: every worker held busy until the region returns.
//! * **T5** two regions with different tables over one frame (the hint-retry path), the second
//!   larger; the epoch base sequence.
//! * **T6** the wait bound fires on every ladder, in both roles, within 1 s.
//! * **T7** the stall census: nothing when disarmed, a counted stall when armed.
//! * **T8** a hint (`SchedItem::prev_off`) that names no earlier item — the item's own epoch, the
//!   next item's, one past the region, `u32::MAX` — is no hint: every block still runs exactly once
//!   and the region completes, under the bounded test policy and under the shipped unbounded
//!   `V2Policy` on a watchdog thread (review r3 W1).
//!
//! Miri runs a reduced matrix (participants 2 and 3, small tables): `cfg(miri)` in
//! `region_common`. Its wall clock does not measure the protocol, so no time is asserted there.

mod region_common;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::{
    Ladder, REGION_MAX_BLOCKS_PER_PARTICIPANT, RegionExit, RegionPolicy, RegionReport,
    RegionStages, RegionWaitBound, SchedItem, V2Policy,
};

use region_common::{
    AllAxesPolicy, BatchedPolicy, Frame, HomePolicy, PureSpinHelpersPolicy, Ran, Rendezvous, Rng,
    Route, Stages, TestPolicy, TtasPolicy, hold_workers, participant_counts, pool,
    run_region, within,
};

/// A random table for `participants`: `(blocks per entry, schedule)`.
fn random_table(rng: &mut Rng, participants: u32, shape: u32) -> (Vec<u16>, Vec<u16>) {
    let max_n = REGION_MAX_BLOCKS_PER_PARTICIPANT * participants;
    let wide = |rng: &mut Rng| (2 + rng.below(u64::from(max_n - 1))) as u16;
    let small = cfg!(miri);
    match shape {
        // All inline.
        0 => {
            let k = 1 + rng.below(4) as usize;
            let len = if small { 3 } else { 1 + rng.below(24) as usize };
            (vec![1; k], (0..len).map(|_| rng.below(k as u64) as u16).collect())
        }
        // One wide stage, at the cap.
        1 => (vec![max_n as u16], vec![0]),
        // A stage run 12× back to back.
        2 => (vec![wide(rng)], vec![0; if small { 4 } else { 12 }]),
        // Mixed.
        _ => {
            let k = 1 + rng.below(6) as usize;
            let blocks: Vec<u16> = (0..k).map(|_| if rng.below(3) == 0 { 1 } else { wide(rng) }).collect();
            let len = if small { 5 } else { 1 + rng.below(40) as usize };
            (blocks, (0..len).map(|_| rng.below(k as u64) as u16).collect())
        }
    }
}

/// The report's formulas against the table and the receipts (T3's checks, run on every region).
fn check_report(ran: &Ran, participants: u32, what: &str) {
    let r: RegionReport = match &ran.result {
        Ok(r) => *r,
        Err(e) => panic!("{what}: the region panicked: {:?}", e.downcast_ref::<RegionWaitBound>()),
    };
    let f = &ran.frame;
    let multi = f.schedule.iter().filter(|it| f.entries[usize::from(it.entry)].n_blocks >= 2);
    assert_eq!(r.published as usize, multi.clone().count(), "{what}: published");
    assert_eq!(r.inline as usize, f.schedule.len() - r.published as usize, "{what}: inline");
    let max_published = multi.map(|it| u32::from(f.entries[usize::from(it.entry)].n_blocks)).max().unwrap_or(0);
    assert_eq!(r.max_blocks, max_published, "{what}: max_blocks");
    assert!(r.max_blocks <= REGION_MAX_BLOCKS_PER_PARTICIPANT * participants, "{what}: max_blocks ≤ 8P");
    let receipts = f.receipts(participants);
    let total: u64 = receipts.iter().map(|rc| rc.blocks).sum();
    assert_eq!(total, f.expected_blocks(), "{what}: Σ receipt blocks == Σ n_blocks");
    assert_eq!(r.helper_blocks, total - receipts[0].blocks, "{what}: helper_blocks");
    for (p, rc) in receipts.iter().enumerate() {
        assert_eq!(rc.region, r.base, "{what}: participant {p}'s receipt tag");
        assert_eq!(rc.exit, Some(RegionExit::End), "{what}: participant {p}'s exit");
    }
}

/// T1 + T2 + T3 for one policy and arming over the random-table matrix.
fn exactly_once_matrix<P: RegionPolicy + 'static, const ARMED: bool>(label: &str) {
    let seeds: u64 = if cfg!(miri) { 1 } else { 6 };
    let shapes: &[u32] = if cfg!(miri) { &[2, 3] } else { &[0, 1, 2, 3] };
    let routes: &[Route] = &[Route::External, Route::Worker];
    for &p in participant_counts() {
        let pool = pool(p);
        // Anti-vacuity: at P >= 2 the helpers must have run blocks somewhere in the matrix, or the
        // claims above were only ever exercised by participant 0.
        let mut helper_blocks = 0u64;
        for &route in routes {
            for seed in 0..seeds {
                for &shape in shapes {
                    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (seed * 977 + u64::from(p) * 131 + u64::from(shape)));
                    let (blocks, order) = random_table(&mut rng, p, shape);
                    let mut frame = Frame::new(blocks.len(), p as usize, blocks.len() * (8 * p as usize).max(1));
                    frame.set_table::<P>(&blocks, &order, p, 0);
                    // 2 µs of work per block, and participant 0's block 0 of the first published item
                    // waits (bounded) for a helper to start: without them participant 0 sweeps whole
                    // items before a parked helper wakes, and the anti-vacuity guard below measured no
                    // helper block at all — first with trivially short blocks, then again under the
                    // crate's parallel test run (other binaries' spinning helpers on every core).
                    let mut stages = Stages::new(&frame).with_work(Duration::from_micros(2));
                    let first_published = order.iter().find(|&&e| blocks[usize::from(e)] >= 2);
                    if let Some(&e) = first_published
                        && p >= 2
                    {
                        stages = stages.with_await_helper(u32::from(e));
                    }
                    let stages = Arc::new(stages);
                    let what = format!("{label} P{p} {route:?} seed {seed} shape {shape} blocks {blocks:?} order {order:?}");
                    let ran = run_region::<P, ARMED>(&pool, route, frame, stages, p);
                    check_report(&ran, p, &what);
                    ran.stages.assert_exactly_once(&ran.frame, &what);
                    helper_blocks += ran.result.as_ref().map_or(0, |r| r.helper_blocks);
                    if ARMED {
                        for (i, it) in ran.frame.schedule.iter().enumerate() {
                            for b in 0..usize::from(ran.frame.entries[usize::from(it.entry)].n_blocks) {
                                assert_eq!(ran.stages.item_runs(i, b), 1, "{what}: item {i} block {b} (armed per-item count)");
                            }
                        }
                    }
                }
            }
        }
        assert!(p < 2 || helper_blocks > 0, "{label} P{p}: no helper ran a block anywhere in the matrix");
    }
}

#[test]
fn t1_t2_exactly_once_and_visibility_v2_disarmed() {
    exactly_once_matrix::<TestPolicy, false>("v2 disarmed");
}

#[test]
fn t1_t2_exactly_once_and_visibility_v2_armed() {
    exactly_once_matrix::<TestPolicy, true>("v2 armed");
}

#[test]
fn t1_t2_exactly_once_and_visibility_batched_completion() {
    exactly_once_matrix::<BatchedPolicy, false>("R-a batched");
}

#[test]
fn t1_t2_exactly_once_and_visibility_home_lines() {
    exactly_once_matrix::<HomePolicy, false>("R-b home lines");
}

#[test]
fn t1_t2_exactly_once_and_visibility_ttas() {
    exactly_once_matrix::<TtasPolicy, false>("R-c TTAS");
}

#[test]
fn t1_t2_exactly_once_and_visibility_all_axes_armed() {
    exactly_once_matrix::<AllAxesPolicy, true>("all axes armed");
}

#[test]
fn t1_t2_exactly_once_and_visibility_pure_spin_helpers() {
    exactly_once_matrix::<PureSpinHelpersPolicy, false>("R-d' pure-spin helpers");
}

/// T3 on a fixed table whose numbers are written out: 2 inline items, 3 published items of
/// widths 3, 2·P and 3.
#[test]
fn t3_report_formulas_on_a_fixed_table() {
    for &p in participant_counts() {
        if p < 2 {
            continue;
        }
        let pool = pool(p);
        let blocks = [1u16, 3, (2 * p) as u16];
        let order = [0u16, 1, 2, 0, 1];
        let mut frame = Frame::new(3, p as usize, 64);
        frame.set_table::<TestPolicy>(&blocks, &order, p, 0);
        let stages = Arc::new(Stages::new(&frame));
        let ran = run_region::<TestPolicy, false>(&pool, Route::External, frame, stages, p);
        let what = format!("T3 P{p}");
        check_report(&ran, p, &what);
        let r = ran.result.as_ref().expect("T3: the region completes");
        assert_eq!((r.published, r.inline, r.max_blocks), (3, 2, 2 * p), "{what}: the written-out numbers");
        assert_eq!(ran.frame.expected_blocks(), 2 + 3 + u64::from(2 * p) + 3, "{what}");
        assert_eq!(r.base, 1, "{what}: a fresh counter's first region is based at 1 (O1)");
    }
}

/// T4: work conservation. Every worker is held by a gated task until the region returns, so the
/// orchestrator (the dispatcher) completes every item alone; the helpers run late, at the scope's
/// join, read END with this region's tag, and run nothing.
#[test]
fn t4_the_region_completes_alone_and_late_helpers_read_its_end() {
    let counts: &[u32] = if cfg!(miri) { &[2] } else { &[2, 4, 8] };
    for &p in counts {
        let pool = pool(p);
        let (_, release) = hold_workers(&pool, p);
        let blocks = [3u16, 1, (2 * p) as u16];
        let order = [0u16, 1, 2, 2, 0];
        let mut frame = Frame::new(3, p as usize, 64);
        frame.set_table::<TestPolicy>(&blocks, &order, p, 0);
        let stages = Arc::new(Stages::new(&frame));
        let ran = run_region::<TestPolicy, false>(&pool, Route::External, frame, stages, p);
        release.store(true, Ordering::Release);
        let what = format!("T4 P{p}");
        check_report(&ran, p, &what);
        ran.stages.assert_exactly_once(&ran.frame, &what);
        let r = ran.result.as_ref().expect("T4: the region completes");
        assert_eq!(r.helper_blocks, 0, "{what}: no helper could run while every worker was held");
        assert_eq!(ran.frame.receipts(p)[0].blocks, ran.frame.expected_blocks(), "{what}: participant 0 ran all");
    }
}

/// T5: two regions with different tables over one frame. The second maps the first's claim words
/// to other entries and blocks and has more entries, so its first executions take the retry path
/// on words holding the first region's epochs. The bases follow `base' = base + len + 1`.
#[test]
fn t5_two_tables_over_one_frame_take_the_retry_path() {
    for &p in participant_counts() {
        if p < 2 {
            continue;
        }
        let pool = pool(p);
        let mut frame = Frame::new(6, p as usize, 256);
        let (blocks_a, order_a) = (vec![3u16, (2 * p) as u16], vec![0u16, 1, 1, 0]);
        frame.set_table::<TestPolicy>(&blocks_a, &order_a, p, 0);
        let stages = Arc::new(Stages::new(&frame));
        let ran = run_region::<TestPolicy, false>(&pool, Route::Worker, frame, stages, p);
        let what = format!("T5 P{p} region A");
        check_report(&ran, p, &what);
        ran.stages.assert_exactly_once(&ran.frame, &what);
        let base_a = ran.result.as_ref().expect("T5: A completes").base;
        assert_eq!(base_a, 1, "{what}: O1, the first base is 1");
        let mut frame = ran.frame;
        assert_eq!(frame.epoch, base_a + order_a.len() as u64 + 1, "{what}: the counter advanced by len + 1");
        // B: shifted by one claim line, more entries, a different order — every word of A now
        // belongs to another (entry, block) and holds an older epoch.
        // B opens with a 2 ms inline item, so its helpers start (and read the publish word) before
        // its first publish: a missing OPEN store then shows as a helper reading A's END
        // (mutation M-OPEN, red on the END-tag debug assertion).
        let blocks_b = vec![(2 * p) as u16, 1, 4, 3, (p + 1) as u16];
        let order_b = vec![1u16, 2, 0, 1, 3, 4, 0, 2, 4];
        frame.set_table::<TestPolicy>(&blocks_b, &order_b, p, 1);
        let stages = Arc::new(Stages::new(&frame).with_slow(1, Duration::from_millis(2)));
        let ran = run_region::<TestPolicy, false>(&pool, Route::External, frame, stages, p);
        let what = format!("T5 P{p} region B");
        check_report(&ran, p, &what);
        ran.stages.assert_exactly_once(&ran.frame, &what);
        let base_b = ran.result.as_ref().expect("T5: B completes").base;
        assert_eq!(base_b, base_a + order_a.len() as u64 + 1, "{what}: B's base");
    }
}

/// A policy whose both ladders are `$l`, bounded at 50 ms.
macro_rules! bound_policy {
    ($name:ident, $l:expr) => {
        struct $name;
        impl RegionPolicy for $name {
            const HELPER_WAIT: Ladder = $l;
            const ORCH_WAIT: Ladder = $l;
            const BOUND_NS: u64 = 50_000_000;
            const DONE_BATCHED: bool = false;
            const HOME_LINES: bool = false;
            const TTAS: bool = false;
        }
    };
}

bound_policy!(BoundPause, Ladder::PauseThenYield { pauses: 5 });
bound_policy!(BoundSpin, Ladder::PureSpin);
bound_policy!(BoundBudget, Ladder::BudgetThenYield { spin_ns: 1_000 });

/// The payload of a region that should have hit its bound, by role.
fn bound_payload<P: RegionPolicy + 'static>(role_helper: bool) -> Option<Result<RegionWaitBound, String>> {
    within(Duration::from_secs(if cfg!(miri) { 600 } else { 1 }), move || {
        let pool = pool(2);
        // Item 0: block 0 (participant 0) waits for a helper to start block 1, which then holds
        // 300 ms — the orchestrator's done-wait passes the bound. Item 1 (helper role): an inline
        // item that holds participant 0 for 300 ms — the recruited helper's publish-wait passes it.
        let (blocks, order, hold_inline) = if role_helper {
            (vec![2u16, 1], vec![0u16, 1], true)
        } else {
            (vec![2u16], vec![0u16], false)
        };
        let mut frame = Frame::new(2, 2, 8);
        frame.set_table::<P>(&blocks, &order, 2, 0);
        let hold = Duration::from_millis(300);
        let mut stages = Stages::new(&frame).with_rendezvous(Rendezvous {
            entry: 0,
            hold: if role_helper { Duration::ZERO } else { hold },
            sleep: true,
        });
        if hold_inline {
            stages = stages.with_slow(1, hold);
        }
        let ran = run_region::<P, false>(&pool, Route::External, frame, Arc::new(stages), 2);
        match ran.result {
            Ok(_) => Err("the region completed".to_owned()),
            Err(e) => e.downcast::<RegionWaitBound>().map(|b| *b).map_err(|_| "another payload".to_owned()),
        }
    })
}

/// T6: the wait bound fires in every ladder (both roles), as a `RegionWaitBound` within 1 s — a
/// ladder phase that never reads the clock reads red (`None`), not hung.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: which role's wait passes the 50 ms bound first is decided by wall-clock proportions that Miri's interpreter does not keep"
)]
fn t6_ladder_bound_fires_in_every_phase() {
    fn one<P: RegionPolicy + 'static>(name: &str) {
        let orch = bound_payload::<P>(false);
        let b = orch.unwrap_or_else(|| panic!("{name}: the orchestrator's wait did not fire its bound within 1 s"));
        let b = b.unwrap_or_else(|e| panic!("{name}: orchestrator role: {e}"));
        assert_eq!(b.participant, 0, "{name}: the orchestrator's bound");
        let helper = bound_payload::<P>(true);
        let b = helper.unwrap_or_else(|| panic!("{name}: the helper's wait did not fire its bound within 1 s"));
        let b = b.unwrap_or_else(|e| panic!("{name}: helper role: {e}"));
        assert_eq!(b.participant, 1, "{name}: the helper's bound");
    }
    one::<BoundPause>("PauseThenYield{5}");
    one::<BoundSpin>("PureSpin");
    one::<BoundBudget>("BudgetThenYield{1 µs}");
}

/// T7: the census is compiled out when disarmed and counts a stall when armed. The block a helper
/// holds is 200 µs (the cut wrote 50): the orchestrator's done-wait sees only the hold's remainder
/// after the rendezvous, and a first run at 50 µs measured a 19.9 µs wait — under the 20 µs
/// threshold — so 50 µs could not show the census counting.
#[test]
fn t7_the_stall_census_is_armed_only() {
    let run = |armed: bool| -> RegionReport {
        let pool = pool(2);
        let mut frame = Frame::new(1, 2, 8);
        frame.set_table::<TestPolicy>(&[2], &[0], 2, 0);
        let stages = Arc::new(Stages::new(&frame).with_rendezvous(Rendezvous {
            entry: 0,
            hold: Duration::from_micros(200),
            sleep: false,
        }));
        let ran = if armed {
            run_region::<TestPolicy, true>(&pool, Route::External, frame, stages, 2)
        } else {
            run_region::<TestPolicy, false>(&pool, Route::External, frame, stages, 2)
        };
        assert_eq!(ran.stages.runs(0, 1), 1, "T7: block 1 ran");
        ran.result.expect("T7: the region completes")
    };
    let off = run(false);
    assert_eq!((off.stalls, off.max_wait_ns), (0, 0), "T7: a disarmed region records no census");
    let on = run(true);
    if !cfg!(miri) {
        assert!(on.stalls >= 1, "T7: the orchestrator's ~200 µs done-wait is a stall when armed: {on:?}");
        assert!(on.max_wait_ns >= 20_000, "T7: the longest wait is recorded when armed: {on:?}");
    }
}

/// T8's hand-built hints: every one a value the safe API builds (`SchedItem::new`, then
/// `RegionFrame::new` and `ThreadPool::region`, none of which rejects it).
#[derive(Clone, Copy, Debug)]
enum Hint {
    /// Item `i` ← `SchedItem::new(0, Some(i))`: the item's OWN epoch `g`. Before the fix, a
    /// participant that reached a block already claimed at `g` won it again (`CAS(g → g)`).
    OwnEpoch,
    /// Item `i` ← `SchedItem::new(0, Some(i + 1))`: the NEXT item's epoch. Before the fix, a sweep
    /// of item `i` still running after item `i + 1` claimed a block won that block again and
    /// lowered its word (`CAS(g + 1 → g)`).
    NextItem,
    /// Item `i` ← `SchedItem::new(0, Some(len + i))`: an epoch in the next region's range.
    PastRegion,
    /// Every item ← `SchedItem::new(0, Some(u32::MAX))`, the constructor's largest input.
    Max,
    /// Control: two entries alternate and item `i ≥ 1` names item `i − 1`, the OTHER entry's — an
    /// earlier item, so a wrong hint below `g` (one retry).
    EarlierWrong,
}

/// Items per T8 region.
const T8_ITEMS: u32 = if cfg!(miri) { 4 } else { 12 };

impl Hint {
    /// `(blocks per entry, schedule)` at `participants`.
    fn table(self, participants: u32) -> (Vec<u16>, Vec<SchedItem>) {
        let per = if cfg!(miri) { 2 } else { REGION_MAX_BLOCKS_PER_PARTICIPANT };
        let n = (per * participants) as u16;
        let item = |i: u32| match self {
            Self::OwnEpoch => SchedItem::new(0, Some(i)),
            Self::NextItem => SchedItem::new(0, Some(i + 1)),
            Self::PastRegion => SchedItem::new(0, Some(T8_ITEMS + i)),
            Self::Max => SchedItem::new(0, Some(u32::MAX)),
            Self::EarlierWrong => SchedItem::new((i % 2) as u16, i.checked_sub(1)),
        };
        let entries = if matches!(self, Self::EarlierWrong) { 2 } else { 1 };
        (vec![n; entries], (0..T8_ITEMS).map(item).collect())
    }
}

/// T8's stages. Atomic run counts only: before the fix a block ran twice, concurrently, and the
/// generation instrument's plain slots would have made that red a data race. Participant 0's
/// block 0 of every item waits (bounded) until a helper has started a block in this region, so the
/// helpers' sweeps reach blocks participant 0 has already claimed.
struct HintStages {
    stride: usize,
    runs: Box<[AtomicU32]>,
    helper_started: AtomicBool,
}

impl HintStages {
    fn new(blocks: &[u16]) -> Self {
        let stride = usize::from(blocks.iter().copied().max().unwrap_or(1));
        Self {
            stride,
            runs: (0..blocks.len() * stride).map(|_| AtomicU32::new(0)).collect(),
            helper_started: AtomicBool::new(false),
        }
    }

    fn runs(&self, e: usize, b: usize) -> u32 {
        self.runs[e * self.stride + b].load(Ordering::Relaxed)
    }
}

impl RegionStages for HintStages {
    fn run_block(&self, entry: u32, block: u32, participant: u32) {
        // Relaxed: a liveness nudge only; it orders no data.
        if participant != 0 {
            self.helper_started.store(true, Ordering::Relaxed);
        } else if block == 0 {
            let t = Instant::now();
            while !self.helper_started.load(Ordering::Relaxed) && t.elapsed() < Duration::from_millis(250) {
                std::hint::spin_loop();
            }
        }
        self.runs[entry as usize * self.stride + block as usize].fetch_add(1, Ordering::Relaxed);
    }
}

/// A panic payload, as text.
fn payload_text(e: &(dyn std::any::Any + Send)) -> String {
    if let Some(b) = e.downcast_ref::<RegionWaitBound>() {
        format!("{b:?}")
    } else if let Some(s) = e.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else {
        "a non-text payload".to_owned()
    }
}

/// `regions` consecutive regions of `hint`'s table over one frame, on a pool of `p`, through the
/// public entry (`ThreadPool::region` inside `install`). Ok: the helpers' blocks, summed. Err: the
/// first region that panicked or ran a block other than exactly once per execution.
fn hint_regions<P: RegionPolicy>(p: u32, hint: Hint, regions: u32) -> Result<u64, String> {
    let pool = pool(p);
    let (blocks, schedule) = hint.table(p);
    let order: Vec<u16> = schedule.iter().map(|it| it.entry).collect();
    let mut frame = Frame::new(blocks.len(), p as usize, blocks.len() * usize::from(blocks[0]));
    frame.set_table::<P>(&blocks, &order, p, 0);
    frame.schedule = schedule;
    let mut helper_blocks = 0;
    for r in 0..regions {
        let stages = HintStages::new(&blocks);
        let report = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: `frame` was built above by `Frame::new` and is only ever this loop's: its
            // columns and counter were never mixed with another `Frame`'s, and the counter only
            // grows. The region below runs `P`, the policy `region_frame` checks the table's claim
            // layout and the hand-built schedule's entries for.
            let region_frame = unsafe { frame.region_frame::<P>(p) };
            pool.install(|_| pool.region::<HintStages, P, false>(region_frame, &stages))
        }))
        .map_err(|e| format!("region {r} panicked: {}", payload_text(&*e)))?;
        for (e, &n) in blocks.iter().enumerate() {
            let execs = frame.execs(e);
            for b in 0..usize::from(n) {
                let runs = stages.runs(e, b);
                if runs != execs {
                    return Err(format!("region {r}: entry {e} block {b} ran {runs} times in {execs} executions"));
                }
            }
        }
        let total: u64 = frame.receipts(p).iter().map(|rc| rc.blocks).sum();
        if total != frame.expected_blocks() {
            return Err(format!("region {r}: Σ receipt blocks {total} != Σ n_blocks {}", frame.expected_blocks()));
        }
        helper_blocks += report.helper_blocks;
    }
    Ok(helper_blocks)
}

/// T8: hints that name no earlier item are no hint. Each (P, hint) runs consecutive regions over
/// one frame under the bounded `TestPolicy` (a defect panics with `RegionWaitBound`) and under the
/// shipped `V2Policy` (unbounded: a defect spins forever, so it runs on a watchdog thread that
/// reads a hang as red; after one hang a hint's later `V2Policy` legs are skipped, so a red run
/// leaves one spinning region behind per hint, not one per P). Every leg must also have had a
/// helper run blocks, or no hint was ever contended.
#[test]
fn t8_a_hint_that_names_no_earlier_item_is_no_hint() {
    let counts: &[u32] = if cfg!(miri) { &[2] } else { &[2, 4, 8] };
    let regions = if cfg!(miri) { 1 } else { 24 };
    let limit = Duration::from_secs(60);
    let hints = [Hint::OwnEpoch, Hint::NextItem, Hint::PastRegion, Hint::Max, Hint::EarlierWrong];
    let mut failures = Vec::new();
    for hint in hints {
        let mut v2_hung = false;
        for &p in counts {
            let mut legs = Vec::new();
            if cfg!(miri) {
                legs.push(("TestPolicy", Some(hint_regions::<TestPolicy>(p, hint, regions))));
            } else {
                legs.push(("TestPolicy", within(limit, move || hint_regions::<TestPolicy>(p, hint, regions))));
                if !v2_hung {
                    let r = within(limit, move || hint_regions::<V2Policy>(p, hint, regions));
                    v2_hung = r.is_none();
                    legs.push(("V2Policy", r));
                }
            }
            for (policy, r) in legs {
                match r {
                    None => failures.push(format!(
                        "P{p} {hint:?} {policy}: no result within {limit:?} (hung, or panicked outside a region)"
                    )),
                    Some(Err(e)) => failures.push(format!("P{p} {hint:?} {policy}: {e}")),
                    Some(Ok(0)) if !cfg!(miri) => {
                        failures.push(format!("P{p} {hint:?} {policy}: no helper ran a block (vacuous)"));
                    }
                    Some(Ok(_)) => {}
                }
            }
        }
    }
    assert!(failures.is_empty(), "T8: {} failing legs:\n{}", failures.len(), failures.join("\n"));
}

/// Miri's Stacked Borrows leg: T1/T2/T3 on the pool-free twin (`region_on_threads`), because the
/// pool's crossbeam transport is not SB-clean. The same frame projection, protocol core and
/// instruments as the pool tests above; only the helpers' spawn differs.
#[cfg(miri)]
#[test]
fn threads_t1_t2_exactly_once_and_visibility() {
    use region_common::run_threads;
    fn leg<P: RegionPolicy, const ARMED: bool>(blocks: &[u16], order: &[u16], p: u32) -> Ran {
        let mut frame = Frame::new(blocks.len(), p as usize, blocks.len() * 8 * p as usize);
        frame.set_table::<P>(blocks, order, p, 0);
        let stages = Arc::new(Stages::new(&frame).with_work(Duration::from_micros(2)));
        run_threads::<P, ARMED>(frame, stages, p)
    }
    let mut helper_blocks = 0u64;
    for p in [2u32, 3] {
        for shape in [2u32, 3] {
            let mut rng = Rng(0x5EED ^ (u64::from(p) * 131 + u64::from(shape)));
            let (blocks, order) = random_table(&mut rng, p, shape);
            for armed in [false, true] {
                let what = format!("threads P{p} shape {shape} armed {armed} blocks {blocks:?} order {order:?}");
                // Each leg lays its table out for the policy it runs: `AllAxesPolicy` (home lines,
                // one claim line per participant) and `TestPolicy` (one per block) differ.
                let ran = if armed {
                    leg::<AllAxesPolicy, true>(&blocks, &order, p)
                } else {
                    leg::<TestPolicy, false>(&blocks, &order, p)
                };
                check_report(&ran, p, &what);
                ran.stages.assert_exactly_once(&ran.frame, &what);
                helper_blocks += ran.result.as_ref().map_or(0, |r| r.helper_blocks);
            }
        }
    }
    assert!(helper_blocks > 0, "threads: no helper ran a block — the twin exercised no concurrency");
}
