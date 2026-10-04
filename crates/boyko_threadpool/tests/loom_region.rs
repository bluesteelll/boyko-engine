//! SR (2): loom models M-R1…M-R15 of the region protocol (`02-SR-DESIGN.md`; cut §3 A3; M-R14 from
//! review r3 W1; M-R15 from tester r4 G-PE / review r4 O1).
//!
//! The models drive the PRODUCTION protocol core — `open` (the epoch reservation, the open reset,
//! OPEN), `run_orchestrator`, `run_helper`, the claim and completion paths and the unwind guards —
//! through `boyko_threadpool::loom_exports::region`, over `LoomRegionWords` (model-owned loom
//! atomics and a loom-tracked table). What they cannot drive is `PoolInner::region`'s own body
//! (no `PoolInner` can be built under loom): its scope and spawns are mirrored here by loom
//! threads, and its guard placement is mirrored by [`LoomOrchestrator`] in M-R10 — the one
//! transcription, flagged like `loom_pool.rs`'s claim core.
//!
//! The blocks' data is loom-tracked: every block of an item reads and writes plain
//! `loom::cell::UnsafeCell` slots (the generation instrument of `region_common`), and every
//! `(entry, block)` run count is an `UnsafeCell` too, so a missing happens-before is a loom
//! causality error and a lost or doubled block is a count or generation mismatch.
//!
//! ## Run — debug profile, one model per process
//!
//! ```bash
//! cargo --config 'target."cfg(windows)".rustflags=["--cfg","loom"]' \
//!   test -p boyko-threadpool --test loom_region -- --list   # must print 15 `: test` lines
//! LOOM_MAX_PREEMPTIONS=2 cargo --config 'target."cfg(windows)".rustflags=["--cfg","loom"]' \
//!   test -p boyko-threadpool --test loom_region -- --exact <model> --test-threads=1 --nocapture
//! ```
//!
//! Lint: `cargo clippy --config '<the same cfg>' -p boyko-threadpool --test loom_region -- -D warnings`
//! — the `--config` AFTER `clippy` (placed before it, clippy's check carries no `--cfg loom`, which
//! the `-v` rustc line shows; measured again here).
//!
//! A filter that matches nothing exits 0: require `running 1 test` per model. The `cfg(windows)`
//! form is the only one that reaches rustc on both Windows toolchains (`loom_pool.rs`' header).
//! M-R1, M-R4 and M-R15 (three participants) pin preemption bound 1 inside the model ([`model3`]).
//! Every other model runs at bound 2 when `LOOM_MAX_PREEMPTIONS` is unset ([`model`]), so the
//! default run and `LOOM_MAX_PREEMPTIONS=2` are the same bound; `=3` is a different one.
//!
//! CI (`.github/workflows/ci.yml`, the `loom` job's `loom_region` step; ruling 17 N5) runs all 15
//! in one debug process at `LOOM_MAX_PREEMPTIONS=3`, names listed and the count pinned. A model
//! added here must be added there, or that step reds on `filtered out`.
//!
//! ## Reading, x86_64-pc-windows-msvc, debug, one model per process
//!
//! 2026-10-01: the first 13 green, 0.01–16.5 s each. Each model's mutation, applied to a scratch
//! copy and restored by content, is RED; tester round 3 re-ran the 13 twice at bound 2, unset and
//! `=2` (`scratchpad/sr/r3/mut/loom_summary.txt`, `loom_p2_summary.txt`; the round-1 logs this line
//! cited are gone). Tester round 4 ran 14 at bound 3 as well (`scratchpad/sr/r4/logs/loom_b3/`).
//! M-R14 (fix round 3) is red on its parent `fb03a33c`, whose `hint_of` ignores the item index
//! (`scratchpad/sr/fix_r3/logs/n1_red_loom_mr14.log`). M-R15 (fix round 4) closes a gap rather
//! than a defect: its mutation survived all 14 earlier models (`scratchpad/sr/r4/` section 4,
//! "PE"), and is red on M-R15 (`scratchpad/sr/fix_r4/logs/`):
//!
//! | model | mutation | red as |
//! |---|---|---|
//! | M-R5 | the sweep claims at the LIVE publish epoch (the change-over hazard) | causality violation |
//! | M-R6 | the table rebuild before region 1's helper joins | causality violation (table) |
//! | M-R7 | R-a: the count lands before the block's writes | causality violation |
//! | M-R8 | R-b: every home offset 0 (two blocks share a word) | branch budget (a block never claimed) |
//! | M-R9 | M-TTAS: `>` for `>=` in the R-c skip | causality violation (two winners) |
//! | M-R10 | the guard dropped after the join | branch budget (the helper never sees END) |
//! | M-R11 | `swap` for the claim CAS | causality violation |
//! | M-R11 | no retry on `w < g` | branch budget (a block never claimed) |
//! | M-R12 | no OPEN store | the END-tag debug assertion |
//! | M-R13 | M-W8: epochs advanced only after a normal END | the open's claim check (debug) |
//! | M-R13 | M-W3 (ruling 17 B1): the done reset after the done-wait, the old order | branch budget (A's count overshoots B's) |
//! | M-R14 | the parent's `hint_of` (no clamp of `prev_off > i`) | causality violation (a block won twice) |
//! | M-R15 | `poisoned_exit`'s END stored Relaxed | helper 2 records END, not POISONED |
#![cfg(loom)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Once;

use loom::cell::UnsafeCell;
use loom::sync::Arc;
use loom::thread;

use boyko_threadpool::loom_exports::region::{self, LoomOrchestrator, LoomRegionWords};
use boyko_threadpool::{
    Ladder, RegionExit, RegionPolicy, RegionStages, SchedItem, StageEntry, V2Policy, claim_lines,
    link_hints,
};

/// Generation slots: 6 = every block count of these models (1, 2, 3) divides it, so every block of
/// every item owns at least one slot.
const SLOTS: usize = 6;

// ---------------------------------------------------------------------------
// The model's stages and table
// ---------------------------------------------------------------------------

/// The payload of an injected panic.
#[derive(Debug)]
struct Injected;

/// Silences the default hook for [`Injected`] panics only (loom re-runs a model many times).
fn quiet_injected_panics() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if info.payload().downcast_ref::<Injected>().is_none() {
                prev(info);
            }
        }));
    });
}

/// Loom-tracked stages: the generation instrument and per-(entry, block) run counts, all plain
/// `UnsafeCell`s, plus an optional one-shot panic on participant 0 or on one helper.
struct ModelStages {
    blocks: Vec<u16>,
    slots: Vec<UnsafeCell<u64>>,
    runs: Vec<UnsafeCell<u32>>,
    stride: usize,
    panic_at: Option<(u32, u32)>,
    panic_helper: Option<u32>,
    fired: std::sync::atomic::AtomicBool,
}

// SAFETY: every cell is accessed through loom's `with`/`with_mut`, which REPORT (rather than
// permit) an access that is not ordered by the region's happens-before edges; `fired` is a std
// atomic, swapped only by the one participant an injection names and read after the joins. The
// region contract gives each (entry, block) of an item one runner, and the barrier orders items.
unsafe impl Sync for ModelStages {}

impl ModelStages {
    fn new(blocks: &[u16]) -> Self {
        let stride = usize::from(*blocks.iter().max().unwrap_or(&1));
        Self {
            blocks: blocks.to_vec(),
            slots: (0..SLOTS).map(|_| UnsafeCell::new(0)).collect(),
            runs: (0..blocks.len() * stride).map(|_| UnsafeCell::new(0)).collect(),
            stride,
            panic_at: None,
            panic_helper: None,
            fired: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Participant 0 panics, once, at the start of `(entry, block)`.
    fn with_panic_at(mut self, entry: u32, block: u32) -> Self {
        quiet_injected_panics();
        self.panic_at = Some((entry, block));
        self
    }

    /// Helper `h` panics, once, at the start of the first block it runs (before touching a slot).
    fn with_helper_panic(mut self, h: u32) -> Self {
        quiet_injected_panics();
        self.panic_helper = Some(h);
        self
    }

    fn fired(&self) -> bool {
        self.fired.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Runs of `(entry, block)` — after every participant has joined.
    fn runs(&self, e: usize, b: usize) -> u32 {
        // SAFETY: read after the joins, which order every run's write before it (loom checks).
        self.runs[e * self.stride + b].with(|p| unsafe { *p })
    }

    /// Every slot's generation — after every participant has joined.
    fn generations(&self) -> Vec<u64> {
        // SAFETY: as in `runs`.
        self.slots.iter().map(|c| c.with(|p| unsafe { *p })).collect()
    }

    /// Every block of every execution ran exactly once, and every slot holds the item count.
    fn assert_exactly_once(&self, schedule: &[u16], what: &str) {
        for (e, &n) in self.blocks.iter().enumerate() {
            let execs = schedule.iter().filter(|&&s| usize::from(s) == e).count() as u32;
            for b in 0..usize::from(n) {
                assert_eq!(self.runs(e, b), execs, "{what}: entry {e} block {b}");
            }
        }
        let want = schedule.len() as u64;
        assert!(self.generations().iter().all(|&g| g == want), "{what}: generations {:?}", self.generations());
    }
}

impl RegionStages for ModelStages {
    fn run_block(&self, entry: u32, block: u32, participant: u32) {
        let injected = if participant == 0 {
            self.panic_at == Some((entry, block))
        } else {
            self.panic_helper == Some(participant)
        };
        if injected && !self.fired.swap(true, std::sync::atomic::Ordering::Relaxed) {
            std::panic::panic_any(Injected);
        }
        let n = usize::from(self.blocks[entry as usize]);
        let (lo, hi) = (block as usize * SLOTS / n, (block as usize + 1) * SLOTS / n);
        // SAFETY: slots [lo, hi) belong to this block of this item; loom reports an unordered access.
        let g0 = self.slots[lo].with(|p| unsafe { *p });
        for c in &self.slots[lo..hi] {
            // SAFETY: as above.
            let g = c.with(|p| unsafe { *p });
            assert_eq!(g, g0, "generation mismatch in entry {entry} block {block}");
            // SAFETY: as above.
            c.with_mut(|p| unsafe { *p = g0 + 1 });
        }
        // SAFETY: one runner per (entry, block) per item; items are barriered.
        self.runs[entry as usize * self.stride + block as usize].with_mut(|p| unsafe { *p += 1 });
    }
}

/// A table: entries of `blocks` (claim lines laid out from `claim_offset` for policy `P` at
/// `participants`) and a linked schedule.
fn table<P: RegionPolicy>(blocks: &[u16], order: &[u16], participants: u32, claim_offset: u32) -> (Vec<StageEntry>, Vec<SchedItem>, u32) {
    let mut first = claim_offset;
    let entries = blocks
        .iter()
        .map(|&n| {
            let e = StageEntry::new(0, 0, n, first, 0);
            first += claim_lines::<P>(n, participants);
            e
        })
        .collect();
    let mut schedule: Vec<SchedItem> = order.iter().map(|&e| SchedItem::new(e, None)).collect();
    let mut last = vec![0u32; blocks.len()];
    link_hints(&mut schedule, &mut last);
    (entries, schedule, first)
}

/// Fresh model words over a table, with one done line per entry.
fn words<P: RegionPolicy>(blocks: &[u16], order: &[u16], participants: u32, claim_lines: usize, words_per_line: usize) -> Arc<LoomRegionWords> {
    words_with_done::<P>(blocks, order, participants, claim_lines, words_per_line, blocks.len())
}

/// Fresh model words over a table, with `done_lines` done lines (room for a later, larger table).
fn words_with_done<P: RegionPolicy>(
    blocks: &[u16],
    order: &[u16],
    participants: u32,
    claim_lines: usize,
    words_per_line: usize,
    done_lines: usize,
) -> Arc<LoomRegionWords> {
    let (entries, schedule, used) = table::<P>(blocks, order, participants, 0);
    assert!(used as usize <= claim_lines, "model frame: {used} claim lines needed");
    assert!(blocks.len() <= done_lines, "model frame: {} done lines needed", blocks.len());
    Arc::new(LoomRegionWords::new(entries, schedule, done_lines, claim_lines, words_per_line, participants))
}

/// One region: open, `helpers` helper threads, participant 0 on this thread, the joins. Returns
/// the base.
fn region_once<P: RegionPolicy>(w: &Arc<LoomRegionWords>, s: &Arc<ModelStages>, epoch: &mut u64, helpers: u32) -> u64 {
    let base = region::open(w, epoch);
    let hs: Vec<_> = (1..=helpers)
        .map(|h| {
            let (w, s) = (Arc::clone(w), Arc::clone(s));
            thread::spawn(move || region::run_helper::<_, P>(&w, base, h, &*s))
        })
        .collect();
    region::run_orchestrator::<_, P>(w, base, &**s);
    for h in hs {
        h.join().expect("model: a helper panicked");
    }
    base
}

/// Every receipt of `w` reads `exit` for this region.
fn assert_receipts(w: &LoomRegionWords, participants: u32, base: u64, what: &str) {
    for p in 0..participants {
        let r = w.receipt(p);
        assert_eq!((r.region, r.exit), (base, Some(RegionExit::End)), "{what}: participant {p}");
    }
}

macro_rules! policy {
    ($name:ident, batched = $ba:expr, home = $ho:expr, ttas = $t:expr) => {
        struct $name;
        impl RegionPolicy for $name {
            const HELPER_WAIT: Ladder = Ladder::PauseThenYield { pauses: 5 };
            const ORCH_WAIT: Ladder = Ladder::PureSpin;
            const BOUND_NS: u64 = 0;
            const DONE_BATCHED: bool = $ba;
            const HOME_LINES: bool = $ho;
            const TTAS: bool = $t;
        }
    };
}
policy!(Batched, batched = true, home = false, ttas = false);
policy!(Home, batched = false, home = true, ttas = false);
policy!(Ttas, batched = false, home = false, ttas = true);

/// One model run: preemption bound 2 unless `LOOM_MAX_PREEMPTIONS` sets one, and a branch budget
/// sized for spin waits — every iteration of a waiter's loop is a loom branch, and three
/// participants of which two wait measured past loom's default 1 000 on M-R1 and M-R4 (a budget,
/// not a livelock: a real livelock exceeds any budget, which is what M-R10's and M-R13's mutations
/// show). `LOOM_MAX_BRANCHES` overrides it.
fn model(f: impl Fn() + Sync + Send + 'static) {
    let mut b = loom::model::Builder::new();
    if b.preemption_bound.is_none() {
        b.preemption_bound = Some(2);
    }
    if std::env::var_os("LOOM_MAX_BRANCHES").is_none() {
        b.max_branches = 20_000;
    }
    b.check(f);
}

/// A three-participant model: preemption bound 1, whatever `LOOM_MAX_PREEMPTIONS` says. At bound 2
/// M-R1 and M-R4 (two helpers spinning beside participant 0) gave no verdict in 12 and 21 minutes
/// (measured 2026-10-01, stopped by PID); at bound 1 each finishes in under a second. Every
/// two-participant model runs at 2.
fn model3(f: impl Fn() + Sync + Send + 'static) {
    let mut b = loom::model::Builder::new();
    b.preemption_bound = Some(1);
    if std::env::var_os("LOOM_MAX_BRANCHES").is_none() {
        b.max_branches = 20_000;
    }
    b.check(f);
}

// ---------------------------------------------------------------------------
// M-R1…M-R6: the base protocol (`01-DESIGN.md` §6.1)
// ---------------------------------------------------------------------------

/// M-R1: two stages, two helpers; every write of stage 1 (made by any participant) is visible to
/// stage 2's inline block, which reads every slot.
#[test]
fn m_r1_stage_writes_are_visible_to_the_next_stage() {
    model3(|| {
        let (blocks, order) = ([2u16, 1], [0u16, 1]);
        let w = words::<V2Policy>(&blocks, &order, 3, 2, 1);
        let s = Arc::new(ModelStages::new(&blocks));
        let mut epoch = 0;
        let base = region_once::<V2Policy>(&w, &s, &mut epoch, 2);
        s.assert_exactly_once(&order, "M-R1");
        assert_receipts(&w, 3, base, "M-R1");
    });
}

/// M-R2: a helper that never sees a publish (the table is one inline item) touches nothing, and
/// exits on END wherever it starts.
#[test]
fn m_r2_a_helper_that_only_sees_end_touches_nothing() {
    model(|| {
        let (blocks, order) = ([1u16], [0u16]);
        let w = words::<V2Policy>(&blocks, &order, 2, 1, 1);
        let s = Arc::new(ModelStages::new(&blocks));
        let mut epoch = 0;
        let base = region_once::<V2Policy>(&w, &s, &mut epoch, 1);
        s.assert_exactly_once(&order, "M-R2");
        assert_receipts(&w, 2, base, "M-R2");
        assert_eq!(w.receipt(1).blocks, 0, "M-R2: the helper ran nothing");
    });
}

/// M-R3: the orchestrator alone completes a region whose helper never runs during it; the helper
/// started afterwards reads END and runs nothing.
#[test]
fn m_r3_the_orchestrator_alone_completes_the_region() {
    model(|| {
        let (blocks, order) = ([2u16], [0u16, 0]);
        let w = words::<V2Policy>(&blocks, &order, 2, 2, 1);
        let s = Arc::new(ModelStages::new(&blocks));
        let mut epoch = 0;
        let base = region::open(&w, &mut epoch);
        region::run_orchestrator::<_, V2Policy>(&w, base, &*s);
        let (w2, s2) = (Arc::clone(&w), Arc::clone(&s));
        thread::spawn(move || region::run_helper::<_, V2Policy>(&w2, base, 1, &*s2))
            .join()
            .expect("model: the late helper");
        s.assert_exactly_once(&order, "M-R3");
        assert_receipts(&w, 2, base, "M-R3");
        assert_eq!(w.receipt(1).blocks, 0, "M-R3: the late helper ran nothing");
    });
}

/// M-R4: three participants on one entry of three blocks; each block is claimed exactly once.
#[test]
fn m_r4_each_block_is_claimed_exactly_once() {
    model3(|| {
        let (blocks, order) = ([3u16], [0u16]);
        let w = words::<V2Policy>(&blocks, &order, 3, 3, 1);
        let s = Arc::new(ModelStages::new(&blocks));
        let mut epoch = 0;
        let base = region_once::<V2Policy>(&w, &s, &mut epoch, 2);
        s.assert_exactly_once(&order, "M-R4");
        let total: u64 = (0..3).map(|p| w.receipt(p).blocks).sum();
        assert_eq!(total, 3, "M-R4: Σ receipt blocks");
        assert_receipts(&w, 3, base, "M-R4");
    });
}

/// M-R5: a late helper across a change-over — A (2 blocks), B (2), A again. Every block of every
/// execution runs exactly once and sees the previous item's writes. The rev-0 mutation ("one
/// shared cursor reset per stage") has no counterpart in the epoch protocol; its hazard — a late
/// helper acting on newer state than the publish it acquired — is the mutation "the sweep claims
/// at the LIVE publish epoch", which must be red.
#[test]
fn m_r5_late_helper_across_a_change_over() {
    model(|| {
        let (blocks, order) = ([2u16, 2], [0u16, 1, 0]);
        let w = words::<V2Policy>(&blocks, &order, 2, 4, 1);
        let s = Arc::new(ModelStages::new(&blocks));
        let mut epoch = 0;
        let base = region_once::<V2Policy>(&w, &s, &mut epoch, 1);
        s.assert_exactly_once(&order, "M-R5");
        assert_receipts(&w, 2, base, "M-R5");
    });
}

/// M-R6: two consecutive regions; the second region's table rebuild happens after the first
/// region's helper has joined, so no helper of the first observes it. The mutation "rebuild before
/// the join returns" must be red (loom: a table write concurrent with a helper's table read).
#[test]
fn m_r6_two_consecutive_regions_rebuild_after_the_join() {
    model(|| {
        let (blocks, order) = ([2u16], [0u16]);
        let w = words_with_done::<V2Policy>(&blocks, &order, 2, 4, 1, 2);
        let s = Arc::new(ModelStages::new(&blocks));
        let mut epoch = 0;
        let base = region::open(&w, &mut epoch);
        let (w1, s1) = (Arc::clone(&w), Arc::clone(&s));
        let h = thread::spawn(move || region::run_helper::<_, V2Policy>(&w1, base, 1, &*s1));
        region::run_orchestrator::<_, V2Policy>(&w, base, &*s);
        h.join().expect("model: region 1's helper");
        s.assert_exactly_once(&order, "M-R6 region 1");
        // The rebuild: a different table over the same words.
        let (blocks2, order2) = ([2u16, 2], [1u16, 0]);
        let (entries, schedule, _) = table::<V2Policy>(&blocks2, &order2, 2, 0);
        // SAFETY: region 1's only helper has joined; no participant reads the table now.
        unsafe { w.set_table(entries, schedule) };
        let s2 = Arc::new(ModelStages::new(&blocks2));
        let base2 = region_once::<V2Policy>(&w, &s2, &mut epoch, 1);
        s2.assert_exactly_once(&order2, "M-R6 region 2");
        assert_receipts(&w, 2, base2, "M-R6 region 2");
    });
}

// ---------------------------------------------------------------------------
// M-R7…M-R9: the axes (R-a, R-b, R-c)
// ---------------------------------------------------------------------------

/// M-R7 (R-a, batched completion): one `fetch_add(k)` per participant after its sweep still orders
/// every block's writes before the next stage. The mutation "count before the block's writes"
/// must be red.
#[test]
fn m_r7_batched_completion_orders_the_writes() {
    model(|| {
        let (blocks, order) = ([2u16, 1], [0u16, 1]);
        let w = words::<Batched>(&blocks, &order, 2, 2, 1);
        let s = Arc::new(ModelStages::new(&blocks));
        let mut epoch = 0;
        let base = region_once::<Batched>(&w, &s, &mut epoch, 1);
        s.assert_exactly_once(&order, "M-R7");
        assert_receipts(&w, 2, base, "M-R7");
    });
}

/// M-R8 (R-b, home lines): the home-line index translation claims each block exactly once (P = 2,
/// three blocks: homes [0, 1) and [1, 3), two words of line 1). Mutation "every home offset is 0"
/// (two blocks share a word) must be red.
#[test]
fn m_r8_home_lines_claim_each_block_exactly_once() {
    model(|| {
        let (blocks, order) = ([3u16], [0u16]);
        let w = words::<Home>(&blocks, &order, 2, 2, 2);
        let s = Arc::new(ModelStages::new(&blocks));
        let mut epoch = 0;
        let base = region_once::<Home>(&w, &s, &mut epoch, 1);
        s.assert_exactly_once(&order, "M-R8");
        assert_receipts(&w, 2, base, "M-R8");
    });
}

/// M-R9 (R-c, TTAS): the pre-load never admits two winners and never loses a block, across a stage
/// executed twice (a helper's sweep of the first execution can lag into the second). Mutation
/// M-TTAS (`>` for `>=` in the skip) must be red.
#[test]
fn m_r9_ttas_never_admits_two_winners() {
    model(|| {
        let (blocks, order) = ([2u16], [0u16, 0]);
        let w = words::<Ttas>(&blocks, &order, 2, 2, 1);
        let s = Arc::new(ModelStages::new(&blocks));
        let mut epoch = 0;
        let base = region_once::<Ttas>(&w, &s, &mut epoch, 1);
        s.assert_exactly_once(&order, "M-R9");
        assert_receipts(&w, 2, base, "M-R9");
    });
}

// ---------------------------------------------------------------------------
// M-R10: unwind
// ---------------------------------------------------------------------------

/// M-R10: participant 0 unwinds — in a claimed block of a published item (block 0, while the
/// helper may hold block 1), and in an inline item — and the helper exits in every interleaving:
/// the guard, dropped by the unwind before the join, published the tagged END. The mutation
/// "guard dropped after the join" (the model owns the guard outside `catch_unwind` and drops it
/// after joining) must be red: the helper waits for an END that never comes.
#[test]
fn m_r10_an_orchestrator_unwind_releases_every_helper() {
    model(|| {
        for (entry, block) in [(0u32, 0u32), (1, 0)] {
            let (blocks, order) = ([2u16, 1], [0u16, 1]);
            let w = words::<V2Policy>(&blocks, &order, 2, 2, 1);
            let s = Arc::new(ModelStages::new(&blocks).with_panic_at(entry, block));
            let mut epoch = 0;
            let base = region::open(&w, &mut epoch);
            let (w1, s1) = (Arc::clone(&w), Arc::clone(&s));
            let h = thread::spawn(move || region::run_helper::<_, V2Policy>(&w1, base, 1, &*s1));
            let r = catch_unwind(AssertUnwindSafe(|| {
                let mut orch = LoomOrchestrator::new(&w, base);
                orch.run::<_, V2Policy>(&w, &*s);
            }));
            h.join().expect("model: the helper exits");
            if s.fired() {
                assert!(r.is_err(), "M-R10: the injected panic propagated");
                assert_eq!(w.receipt(0).exit, Some(RegionExit::Panicked), "M-R10: participant 0");
                assert_eq!(w.receipt(1).exit, Some(RegionExit::Poisoned), "M-R10: the helper");
                assert_eq!(w.poison_value(), 1, "M-R10: poison");
            } else {
                assert!(r.is_ok(), "M-R10: no injection, no panic");
                assert_receipts(&w, 2, base, "M-R10 (not fired)");
            }
        }
    });
}

// ---------------------------------------------------------------------------
// M-R11…M-R13: epoch claims, the open, the W8 recovery
// ---------------------------------------------------------------------------

/// M-R11: epoch claims. (i) A stage executed twice: a helper's stale sweep at g′ can overlap the
/// live sweep at g; each block runs exactly once per epoch and the stale sweep never wins. (ii) A
/// second region whose table maps one claim word to another entry: the first execution's single
/// retry claims each block exactly once. Mutations: "`swap` instead of CAS" (a stale sweep lowers
/// a word) and "no retry on `w < g`" (a block is never claimed) must be red.
#[test]
fn m_r11_epoch_claims() {
    model(|| {
        let (blocks, order) = ([2u16], [0u16, 0]);
        let w = words::<V2Policy>(&blocks, &order, 2, 3, 1);
        let s = Arc::new(ModelStages::new(&blocks));
        let mut epoch = 0;
        let base = region_once::<V2Policy>(&w, &s, &mut epoch, 1);
        s.assert_exactly_once(&order, "M-R11 (i)");
        assert_receipts(&w, 2, base, "M-R11 (i)");
        // (ii) entry 0 now owns claim lines 1 and 2: line 1 holds region 1's epoch.
        let (entries, schedule, _) = table::<V2Policy>(&blocks, &[0], 2, 1);
        // SAFETY: region 1's helper has joined.
        unsafe { w.set_table(entries, schedule) };
        let s2 = Arc::new(ModelStages::new(&blocks));
        let base2 = region_once::<V2Policy>(&w, &s2, &mut epoch, 1);
        s2.assert_exactly_once(&[0], "M-R11 (ii)");
        assert_receipts(&w, 2, base2, "M-R11 (ii)");
        assert_eq!((w.claim_value(1, 0), w.claim_value(2, 0)), (base2 + 1, base2 + 1), "M-R11 (ii): both words at g");
    });
}

/// M-R12: the region open. After a region that left its END in the publish word, two consecutive
/// regions with one helper each; each helper reads only its own region's OPEN, epochs and END.
/// The mutation "no OPEN store" must be red on the END-tag assertion (a helper reads the previous
/// region's END).
#[test]
fn m_r12_the_open_store_hides_the_previous_end() {
    model(|| {
        let (blocks, order) = ([2u16], [0u16]);
        let w = words::<V2Policy>(&blocks, &order, 2, 2, 1);
        let mut epoch = 0;
        // Region 0: participant 0 alone, so the publish word holds END | base0 before region 1.
        let s0 = Arc::new(ModelStages::new(&blocks));
        let base0 = region::open(&w, &mut epoch);
        region::run_orchestrator::<_, V2Policy>(&w, base0, &*s0);
        for k in 1..=2 {
            let s = Arc::new(ModelStages::new(&blocks));
            let base = region_once::<V2Policy>(&w, &s, &mut epoch, 1);
            s.assert_exactly_once(&order, &format!("M-R12 region {k}"));
            assert_receipts(&w, 2, base, &format!("M-R12 region {k}"));
        }
    });
}

/// M-R13 (the W8 fix): region A is poisoned — participant 0 panics in block 0 of a published
/// item, possibly after the helper counted block 1 — and caught; region B over the SAME words and
/// table must complete with every block exactly once, its base above every epoch A wrote. The
/// mutation M-W8 ("reserve the epochs only on a normal END") must be red.
///
/// The last assertion changed with ruling 17 B1 (the done line is reset just before each publish,
/// no longer after the done-wait and in the open after a poisoned region): entry 0's line must end
/// at exactly 2, B's own count from its pre-publish reset, whatever A left on it (0 or 1). It was
/// `== 0`, the old order's "0 between items", which B1 retires.
#[test]
fn m_r13_a_region_after_a_caught_poisoned_one_completes() {
    model(|| {
        let (blocks, order) = ([2u16, 1], [0u16, 1]);
        let w = words::<V2Policy>(&blocks, &order, 2, 2, 1);
        let mut epoch = 0;
        // Region A, poisoned.
        let s = Arc::new(ModelStages::new(&blocks).with_panic_at(0, 0));
        let base_a = region::open(&w, &mut epoch);
        let (w1, s1) = (Arc::clone(&w), Arc::clone(&s));
        let h = thread::spawn(move || region::run_helper::<_, V2Policy>(&w1, base_a, 1, &*s1));
        let r = catch_unwind(AssertUnwindSafe(|| region::run_orchestrator::<_, V2Policy>(&w, base_a, &*s)));
        h.join().expect("model: region A's helper exits");
        assert_eq!(r.is_err(), s.fired(), "M-R13: region A panicked iff the injection fired");
        // Region B: same words, same table, no injection.
        let s2 = Arc::new(ModelStages::new(&blocks));
        let base_b = region_once::<V2Policy>(&w, &s2, &mut epoch, 1);
        assert_eq!(base_b, base_a + order.len() as u64 + 1, "M-R13: B.base == A.base + A.len + 1");
        s2.assert_exactly_once(&order, "M-R13 region B");
        assert_receipts(&w, 2, base_b, "M-R13 region B");
        assert_eq!(w.poison_value(), 0, "M-R13: B's open cleared the poison");
        assert_eq!(w.done_value(0), 2, "M-R13: entry 0's line holds B's count from its own reset");
    });
}

// ---------------------------------------------------------------------------
// M-R14: a hint that names no earlier item (review r3 W1)
// ---------------------------------------------------------------------------

/// M-R14: a hint that names no earlier item is no hint. One entry of two blocks runs twice, and
/// BOTH items carry `SchedItem::new(0, Some(1))` (`prev_off` 2, expected value `base + 2`). For
/// item 0 that is the NEXT item's epoch; for item 1 it is its OWN epoch. Every block of each
/// execution must run exactly once. The pre-fix `hint_of`, which ignores the item index, must be red
/// in two ways. First, item 1's helper wins again a block participant 0 already claimed (`g → g`).
/// Second, a helper whose sweep of item 0 lags past that item's completion wins again a block that
/// item 1 claimed, and lowers its word (`g + 1 → g`).
#[test]
fn m_r14_a_hint_that_names_no_earlier_item_is_no_hint() {
    model(|| {
        let (blocks, order) = ([2u16], [0u16, 0]);
        let entries = vec![StageEntry::new(0, 0, 2, 0, 0)];
        let schedule = vec![SchedItem::new(0, Some(1)); 2];
        let w = Arc::new(LoomRegionWords::new(entries, schedule, 1, 2, 1, 2));
        let s = Arc::new(ModelStages::new(&blocks));
        let mut epoch = 0;
        let base = region_once::<V2Policy>(&w, &s, &mut epoch, 1);
        s.assert_exactly_once(&order, "M-R14");
        assert_receipts(&w, 2, base, "M-R14");
    });
}

// ---------------------------------------------------------------------------
// M-R15: the orchestrator's poisoned END, read by a third participant (tester r4 G-PE)
// ---------------------------------------------------------------------------

/// M-R15: three participants on one entry of three blocks; helper 1 panics, once, in the first
/// block it runs. Its block is never counted, so participant 0 can only leave through the poison
/// (its done-wait's Acquire load) and `poisoned_exit`. Helper 2 never synchronises with helper 1:
/// it reads that END with Acquire and then loads `poison` Relaxed, so `poisoned_exit`'s Release
/// END is the one edge that carries helper 1's poison store to it, and it must record POISONED.
/// The mutation "`poisoned_exit`'s END stored Relaxed" survives M-R1…M-R14, because no helper
/// poisons a region in any of them, so `poisoned_exit` never runs (in M-R10 and M-R13 participant
/// 0 itself unwinds, and its END is `on_unwind`'s). It must be red here.
#[test]
fn m_r15_the_poisoned_exit_end_carries_the_poison_to_a_third_participant() {
    model3(|| {
        let (blocks, order) = ([3u16], [0u16]);
        let w = words::<V2Policy>(&blocks, &order, 3, 3, 1);
        let s = Arc::new(ModelStages::new(&blocks).with_helper_panic(1));
        let mut epoch = 0;
        let base = region::open(&w, &mut epoch);
        let hs: Vec<_> = (1..=2u32)
            .map(|h| {
                let (w, s) = (Arc::clone(&w), Arc::clone(&s));
                // A panic that leaves a loom thread ends the model, so helper 1's unwind is caught
                // inside its thread, after its guard has run (the pool's task boundary does that).
                thread::spawn(move || {
                    catch_unwind(AssertUnwindSafe(|| region::run_helper::<_, V2Policy>(&w, base, h, &*s))).is_err()
                })
            })
            .collect();
        region::run_orchestrator::<_, V2Policy>(&w, base, &*s);
        let unwound: Vec<bool> = hs.into_iter().map(|h| h.join().expect("model: a helper thread")).collect();
        for p in 0..3 {
            assert_eq!(w.receipt(p).region, base, "M-R15: participant {p}'s receipt tag");
        }
        if s.fired() {
            assert_eq!(unwound, [true, false], "M-R15: only helper 1 unwound");
            assert_eq!(w.receipt(0).exit, Some(RegionExit::Poisoned), "M-R15: participant 0 took poisoned_exit");
            assert_eq!(w.receipt(1).exit, Some(RegionExit::Panicked), "M-R15: helper 1");
            assert_eq!(w.receipt(2).exit, Some(RegionExit::Poisoned), "M-R15: helper 2 read the poison through the END");
            assert_eq!(w.poison_value(), 1, "M-R15: poison");
            for b in 0..3 {
                assert!(s.runs(0, b) <= 1, "M-R15: block {b} ran {} times", s.runs(0, b));
            }
        } else {
            assert_eq!(unwound, [false, false], "M-R15 (not fired): no helper unwound");
            s.assert_exactly_once(&order, "M-R15 (not fired)");
            assert_receipts(&w, 3, base, "M-R15 (not fired)");
        }
    });
}
