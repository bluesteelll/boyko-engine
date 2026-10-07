//! SR (1): the region's unwind liveness and its recovery after a caught panic
//! (`02-SR-DESIGN.md`; cut §1.3, §3 A2; critique r1 W3).
//!
//! **Cases 1–4** (rev 3 R3-10, O9): participant 0 panics in an inline entry placed after a
//! multi-block one (helpers live); participant 0 panics in a claimed block; participant 0 fires the
//! region's `n_blocks ≤ 8·P` debug assertion (debug profile only); one helper panics in one claimed
//! block (a single `(entry, block)` with `block ≠ 0`, injected only on a participant ≠ 0, while
//! participant 0 spends 50 µs per block of that entry; up to 8 attempts, never firing is red).
//! Every case asserts, at P = 2, 8 and 16:
//! * (a) the payload `catch_unwind` returns is the injected one (by downcast);
//! * (b) every receipt carries this region's `base`, exactly one reads PANICKED — the injected
//!   participant — the rest END or POISONED, none BOUND or NOT_RUN. (a) alone cannot see a broken
//!   guard: the owner's payload wins over discarded task payloads;
//! * (c) the call returned in under 1 s, half the 2 s bound (not under Miri).
//!
//! **W8** (the open item): a poisoned region is caught, then a second region over the SAME frame
//! (same columns, same table) must complete in under 1 s with every block run exactly once, END and
//! B's base in every receipt, and `B.base == A.base + A.len + 1` — the epoch range was reserved at
//! A's entry, so B's epochs are above every word A wrote. Mutation M-W8 ("advance the counter only
//! after a normal END") predicted red per profile: debug on the open's claim check, release on the
//! `RegionWaitBound` payload (B's claims are all skipped and its done-wait reaches the bound).
//!
//! **W3** (critique r1; re-expressed for ruling 17 B1): A is poisoned while entry 2's done line
//! holds a partial count; B has only two entries, so A's count is still on line 2 when C opens; C
//! has four entries and runs entry 2 first. The intent is unchanged — a stale done line must not
//! complete an item early — and so is the scenario. What changed is the mechanism the gate holds
//! the region to: each published item now resets its own line just before its publish, so the
//! gate no longer asks for "every line 0" (lines hold their last count now) but for C to complete
//! with every block exactly once while a helper holds block 1 of C's first entry-2 item, and for
//! every line C published to end at exactly its entry's block count. Mutation **M-W3**, re-expressed
//! the same way, is "the done line is reset after the item's done-wait (the old order), not before
//! its publish".
//!
//! **F-FRAME failure B** (review r4 W1; ruling 17 B1): a region poisoned after a helper counted
//! block 1 leaves `done[0] == 1`; the caller re-creates its sync line (`RegionLine::ZERO`, which
//! hides the poison from the next open); region B then runs entry 0 and an inline entry 1 over the
//! same columns under the shipped `V2Policy`. Before B1 the stale 1 completed entry 0 after
//! participant 0's own block, and item 1 ran while a helper was still inside block 1 (5 of 5 runs
//! in release; debug panicked at the open's done-line check). The test must read no overlap, and
//! runs under a watchdog, so a hang is a red, never a hung suite.
//!
//! **CR-F (the finisher advance, `WithAdvance<_, true>`).** Every case above has a `fin_` twin
//! under `Fin<TestPolicy>`, and F-FRAME B gains a `Fin<V2Policy>` round. Under the finisher the
//! oversize entry of case 3 is published by whichever participant completed the item before it, so
//! `fin_case3` asserts exactly one PANICKED receipt, that publisher's. The orchestrator cases also
//! assert that no receipt of their poisoned region counted an advance (CR-F fix r1: participant
//! 0's count is recorded after `run_orchestrator` returns, and only after a normal END). **G-CR-F-PANIC**
//! (`fin_helper_panic_while_participant0_waits`) places a helper's panic in a published block
//! while participant 0 waits in its publish wait: entry 0's block 0 holds participant 0 until a
//! helper starts block 1 (only a helper can, participant 0 being inside block 0), and that helper
//! panics 5 ms later, by which time participant 0 has swept the rest of the item. Nothing then
//! publishes an END (only participant 0's guard would), so participant 0's poison poll in that
//! wait is its only exit: the unbounded `Fin<V2Policy>` region runs under a 10 s watchdog, and a
//! wait without the poll (N-FIN-NOPOLL) is "no verdict within 10 s".

mod region_common;

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::{
    Advance, RegionExit, RegionLine, RegionPolicy, RegionReceipt, RegionReport, RegionStages,
    RegionWaitBound, ThreadPool, V2Policy, WithAdvance, try_with_active_pool,
};

use region_common::{
    Frame, Inject, Injected, LatePanic, Ran, Rendezvous, Route, Stages, TestPolicy, Who,
    panic_participant_counts, pool, run_region, within,
};

/// `P` under the finisher advance (CR-F).
type Fin<P> = WithAdvance<P, true>;

/// Attempts allowed for an injection that depends on which participant claims a block.
const ATTEMPTS: usize = 8;

/// The injection kinds of the W8 test.
#[derive(Clone, Copy, Debug)]
enum Kind {
    OrchestratorInline,
    OrchestratorClaimed,
    HelperClaimed,
}

/// The table of a kind: `(blocks, order, injection)`; entry 0 is always multi-block so helpers
/// are live before the injection.
fn table(kind: Kind, p: u32) -> (Vec<u16>, Vec<u16>, Inject) {
    let wide = (2 * p) as u16;
    match kind {
        Kind::OrchestratorInline => (
            vec![wide, 1],
            vec![0, 1, 0],
            Inject { entry: 1, block: 0, who: Who::Orchestrator },
        ),
        Kind::OrchestratorClaimed => (
            vec![wide, 1],
            vec![0, 1, 0, 0],
            Inject { entry: 0, block: 0, who: Who::Orchestrator },
        ),
        // Block 2 is helper 1's start (h·n/P = 2): a helper reaches it long before participant 0,
        // which spends 50 µs on each block of entry 0.
        Kind::HelperClaimed => (
            vec![wide, 1],
            vec![1, 0, 1, 0],
            Inject { entry: 0, block: 2, who: Who::Helper },
        ),
    }
}

/// Runs the injected region of policy `P` until the injection fires (at most [`ATTEMPTS`]);
/// returns the run.
fn poisoned_run<P: RegionPolicy + 'static>(
    pool: &Arc<boyko_threadpool::ThreadPool>,
    mut frame: Frame,
    kind: Kind,
    p: u32,
    inject: Inject,
) -> (Ran, u64) {
    for attempt in 0..ATTEMPTS {
        let base = frame.epoch.max(1);
        let mut stages = Stages::new(&frame).with_inject(inject);
        if matches!(kind, Kind::HelperClaimed) {
            // O9: participant 0 spends 50 µs per block of the entry, and (so that a parked helper's
            // wake latency cannot decide the case) its block 0 waits until a helper has started.
            stages = stages.with_slow(0, Duration::from_micros(50)).with_await_helper(0);
        }
        let route = if attempt % 2 == 0 { Route::External } else { Route::Worker };
        let ran = run_region::<P, false>(pool, route, frame, Arc::new(stages), p);
        if ran.result.is_err() {
            return (ran, base);
        }
        assert!(
            !ran.stages.fired.load(std::sync::atomic::Ordering::Relaxed),
            "{kind:?} P{p}: the injection fired but the region returned normally"
        );
        frame = ran.frame;
    }
    panic!("{kind:?} P{p}: the injection never fired in {ATTEMPTS} attempts");
}

/// (a), (b), (c) on a poisoned run based at `base`.
fn assert_poisoned(ran: &Ran, p: u32, base: u64, who: Who, what: &str) -> u32 {
    let payload: &(dyn Any + Send) = ran.result.as_ref().expect_err("the region panicked").as_ref();
    let injected = payload.downcast_ref::<Injected>().unwrap_or_else(|| {
        panic!(
            "{what}: (a) the payload is not the injected one: bound {:?}, message {:?}",
            payload.downcast_ref::<boyko_threadpool::RegionWaitBound>(),
            payload.downcast_ref::<String>()
        )
    });
    match who {
        Who::Orchestrator => assert_eq!(injected.participant, 0, "{what}: (a) participant 0 panicked"),
        Who::Helper => assert_ne!(injected.participant, 0, "{what}: (a) a helper panicked"),
    }
    assert_receipts(&ran.frame.receipts(p), base, injected.participant, what);
    if !cfg!(miri) {
        assert!(ran.wall < Duration::from_secs(1), "{what}: (c) the call took {:?}", ran.wall);
    }
    injected.participant
}

/// (b): every receipt is this region's; exactly one PANICKED, the injected participant; the rest
/// END or POISONED.
fn assert_receipts(receipts: &[RegionReceipt], base: u64, panicked: u32, what: &str) {
    for (p, r) in receipts.iter().enumerate() {
        assert_eq!(r.region, base, "{what}: (b) participant {p}'s receipt tag ({receipts:?})");
        let want_panicked = p as u32 == panicked;
        match r.exit {
            Some(RegionExit::Panicked) => assert!(want_panicked, "{what}: (b) participant {p} PANICKED ({receipts:?})"),
            Some(RegionExit::End | RegionExit::Poisoned) => {
                assert!(!want_panicked, "{what}: (b) the injected participant {p} did not read PANICKED ({receipts:?})");
            }
            other => panic!("{what}: (b) participant {p} exited {other:?} ({receipts:?})"),
        }
    }
}

/// A poisoned region run by the orchestrator counts no advance in any receipt: helpers never
/// advance under it, a poisoned END is no advance, and participant 0's count is recorded only
/// after it left by a normal END (`RegionReceipt::advances`; CR-F fix r1, where the count moved out
/// of `run_orchestrator` behind an exit check). Under the finisher a receipt counts the publishes
/// its participant made before the poison, so nothing is asserted there.
fn assert_orchestrated_advances_zero<P: RegionPolicy>(receipts: &[RegionReceipt], what: &str) {
    if P::ADVANCE == Advance::Orchestrator {
        for (q, r) in receipts.iter().enumerate() {
            assert_eq!(r.advances, 0, "{what}: participant {q} of a poisoned orchestrated region counted an advance ({receipts:?})");
        }
    }
}

fn case<P: RegionPolicy + 'static>(kind: Kind, who: Who, label: &str) {
    for &p in panic_participant_counts() {
        let pool = pool(p);
        let (blocks, order, inject) = table(kind, p);
        let mut frame = Frame::new(blocks.len(), p as usize, 4 * 16 * 8);
        frame.set_table::<P>(&blocks, &order, p, 0);
        let (ran, base) = poisoned_run::<P>(&pool, frame, kind, p, inject);
        let what = format!("{label}{kind:?} P{p}");
        assert_poisoned(&ran, p, base, who, &what);
        assert_orchestrated_advances_zero::<P>(&ran.frame.receipts(p), &what);
    }
}

#[test]
fn case1_orchestrator_panics_in_an_inline_entry() {
    case::<TestPolicy>(Kind::OrchestratorInline, Who::Orchestrator, "");
}

#[test]
fn case2_orchestrator_panics_in_a_claimed_block() {
    case::<TestPolicy>(Kind::OrchestratorClaimed, Who::Orchestrator, "");
}

/// Case 3: the region's own `n_blocks ≤ 8·P` debug assertion fires on the participant that
/// publishes an oversize entry placed after a multi-block one: participant 0 under the
/// orchestrator, the completer of the item before it under the finisher (`any_publisher`). Debug
/// profile only (it is a `debug_assert!`).
#[cfg(debug_assertions)]
fn case3_body<P: RegionPolicy + 'static>(label: &str, any_publisher: bool) {
    for &p in panic_participant_counts() {
        let pool = pool(p);
        let oversize = (8 * p + 1) as u16;
        let mut frame = Frame::new(2, p as usize, 4 * 16 * 8 + 64);
        frame.set_table::<P>(&[(2 * p) as u16, oversize], &[0, 1], p, 0);
        let base = frame.epoch.max(1);
        let stages = Arc::new(Stages::new(&frame));
        let ran = run_region::<P, false>(&pool, Route::External, frame, stages, p);
        let what = format!("{label} P{p}");
        let payload = ran.result.as_ref().expect_err("case 3: the region panicked");
        let msg = payload
            .downcast_ref::<String>()
            .unwrap_or_else(|| panic!("{what}: (a) the payload is not the assertion's message"));
        assert!(msg.contains("more than 8 per participant"), "{what}: (a) the payload is {msg:?}");
        let receipts = ran.frame.receipts(p);
        let panicked = if any_publisher {
            let who: Vec<usize> = receipts
                .iter()
                .enumerate()
                .filter(|(_, rc)| rc.exit == Some(RegionExit::Panicked))
                .map(|(q, _)| q)
                .collect();
            assert_eq!(who.len(), 1, "{what}: exactly one PANICKED receipt, the oversize entry's publisher ({receipts:?})");
            who[0] as u32
        } else {
            0
        };
        assert_receipts(&receipts, base, panicked, &what);
        assert_orchestrated_advances_zero::<P>(&receipts, &what);
        if !cfg!(miri) {
            assert!(ran.wall < Duration::from_secs(1), "{what}: (c) the call took {:?}", ran.wall);
        }
    }
}

#[cfg(debug_assertions)]
#[test]
fn case3_a_region_debug_assertion_fires_on_the_orchestrator() {
    case3_body::<TestPolicy>("case 3", false);
}

#[test]
fn case4_a_helper_panics_in_a_claimed_block() {
    case::<TestPolicy>(Kind::HelperClaimed, Who::Helper, "");
}

// The finisher twins (G-CR-F-PANIC's bounded part): the same cases, every receipt and payload
// rule unchanged, under `Fin<TestPolicy>`.

#[test]
fn fin_case1_orchestrator_panics_in_an_inline_entry() {
    case::<Fin<TestPolicy>>(Kind::OrchestratorInline, Who::Orchestrator, "fin ");
}

#[test]
fn fin_case2_orchestrator_panics_in_a_claimed_block() {
    case::<Fin<TestPolicy>>(Kind::OrchestratorClaimed, Who::Orchestrator, "fin ");
}

/// The finisher's case 3: the oversize entry's publisher is the completer of item 0, any
/// participant; exactly one PANICKED receipt is that participant's.
#[cfg(debug_assertions)]
#[test]
fn fin_case3_a_region_debug_assertion_fires_on_the_publisher() {
    case3_body::<Fin<TestPolicy>>("fin case 3", true);
}

#[test]
fn fin_case4_a_helper_panics_in_a_claimed_block() {
    case::<Fin<TestPolicy>>(Kind::HelperClaimed, Who::Helper, "fin ");
}

/// W8: a caught poisoned region, then a healthy region over the same frame, at P = 2, 8, 16, for
/// every injection kind.
fn w8_body<P: RegionPolicy + 'static>(label: &str) {
    for &p in panic_participant_counts() {
        let pool = pool(p);
        for kind in [Kind::OrchestratorInline, Kind::OrchestratorClaimed, Kind::HelperClaimed] {
            let (blocks, order, inject) = table(kind, p);
            let mut frame = Frame::new(blocks.len(), p as usize, 4 * 16 * 8);
            frame.set_table::<P>(&blocks, &order, p, 0);
            let what = format!("{label} {kind:?} P{p}");
            let (ran, base_a) = poisoned_run::<P>(&pool, frame, kind, p, inject);
            let who = if matches!(kind, Kind::HelperClaimed) { Who::Helper } else { Who::Orchestrator };
            assert_poisoned(&ran, p, base_a, who, &what);
            let frame = ran.frame;
            let len = frame.schedule.len() as u64;
            let epoch_after_a = frame.epoch;
            // Region B: the SAME frame and table, no injection. Its outcome is asserted before the
            // counter arithmetic, so mutation M-W8 reds on B's behaviour (the cut's prediction).
            let stages = Arc::new(Stages::new(&frame));
            let ran = run_region::<P, false>(&pool, Route::External, frame, stages, p);
            let r: RegionReport = match &ran.result {
                Ok(r) => *r,
                Err(e) => panic!(
                    "{what}: region B panicked: {:?} / {:?}",
                    e.downcast_ref::<boyko_threadpool::RegionWaitBound>(),
                    e.downcast_ref::<String>()
                ),
            };
            assert_eq!(epoch_after_a, base_a + len + 1, "{what}: the poisoned region's whole epoch range stayed reserved");
            assert_eq!(r.base, base_a + len + 1, "{what}: B.base == A.base + A.len + 1");
            ran.stages.assert_exactly_once(&ran.frame, &format!("{what} region B"));
            for (q, rc) in ran.frame.receipts(p).iter().enumerate() {
                assert_eq!((rc.region, rc.exit), (r.base, Some(RegionExit::End)), "{what}: B participant {q}");
            }
            if !cfg!(miri) {
                assert!(ran.wall < Duration::from_secs(1), "{what}: region B took {:?}", ran.wall);
            }
        }
    }
}

#[test]
fn w8_poisoned_region_then_same_frame_completes() {
    w8_body::<TestPolicy>("W8");
}

#[test]
fn fin_w8_poisoned_region_then_same_frame_completes() {
    w8_body::<Fin<TestPolicy>>("fin W8");
}

/// Every done line of an entry the frame's current schedule PUBLISHED (`n_blocks ≥ 2`) ends at
/// exactly that entry's block count: its last execution counted from 0 (ruling 17 B1 resets a line
/// just before each publish). A count that had started from a stale value ends above it.
fn assert_published_lines_end_at_their_counts(frame: &Frame, what: &str) {
    for (e, entry) in frame.entries.iter().enumerate() {
        let published = entry.n_blocks >= 2 && frame.schedule.iter().any(|it| usize::from(it.entry) == e);
        if published {
            assert_eq!(
                frame.done[e].word(0),
                u64::from(entry.n_blocks),
                "{what}: done line {e} after the region (its last execution's count, from 0)"
            );
        }
    }
}

/// W3: A poisoned with entry 2's done line partial; B with two entries, so line 2 keeps A's count;
/// C with four, running entry 2 first, over that line. C's first entry-2 item holds a helper in
/// block 1 for 50 ms after participant 0's block 0, and the item after it (inline entry 0) reads
/// every slot: a count that started from A's stale value either completes the item while block 1
/// is held (that next item reads block 1's slots unwritten: a generation mismatch) or overshoots
/// and never completes (the test bound). C must complete, every block exactly once — the intent,
/// asserted first so that a stale count reds here — and then every line C published must end at
/// exactly its entry's block count.
fn w3_body<P: RegionPolicy + 'static>(label: &str) {
    for &p in panic_participant_counts() {
        let pool = pool(p);
        let wide = (2 * p) as u16;
        let mut frame = Frame::new(4, p as usize, 4 * 16 * 8);
        // A: entry 2 is the in-flight one; participant 0 panics in its block 1 after counting
        // block 0, so its done line is left at r >= 1.
        frame.set_table::<P>(&[1, 1, wide], &[0, 2, 1, 2], p, 0);
        let what = format!("{label} P{p}");
        let inject = Inject { entry: 2, block: 1, who: Who::Orchestrator };
        let (ran, base_a) = poisoned_run::<P>(&pool, frame, Kind::OrchestratorClaimed, p, inject);
        assert_poisoned(&ran, p, base_a, Who::Orchestrator, &what);
        let mut frame = ran.frame;
        let stale = frame.done[2].word(0);
        assert_ne!(stale, 0, "{what}: precondition — A left entry 2's done line partial");
        // B: two entries (lines 0 and 1 only).
        frame.set_table::<P>(&[wide, 1], &[0, 1, 0], p, 0);
        let stages = Arc::new(Stages::new(&frame));
        let ran = run_region::<P, false>(&pool, Route::External, frame, stages, p);
        if let Err(e) = &ran.result {
            panic!(
                "{what}: region B panicked: {:?} / {:?}",
                e.downcast_ref::<RegionWaitBound>(),
                e.downcast_ref::<String>()
            );
        }
        ran.stages.assert_exactly_once(&ran.frame, &format!("{what} region B"));
        let mut frame = ran.frame;
        // The case is real: nothing in B touched line 2, so C's first entry-2 item opens over A's
        // stale count, and only its own pre-publish reset stands between that count and C.
        assert_eq!(frame.done[2].word(0), stale, "{what}: B left A's stale count on line 2");
        // C: four entries again; entry 2 executes three times, first.
        frame.set_table::<P>(&[1, wide, wide + 1, 3], &[2, 0, 1, 2, 3, 2], p, 0);
        let hold = Rendezvous { entry: 2, hold: Duration::from_millis(50), sleep: true };
        let stages = Arc::new(Stages::new(&frame).with_rendezvous(hold));
        let ran = run_region::<P, false>(&pool, Route::Worker, frame, stages, p);
        if let Err(e) = &ran.result {
            panic!(
                "{what}: region C panicked (a stale count that overshot never completes): {:?} / {:?}",
                e.downcast_ref::<RegionWaitBound>(),
                e.downcast_ref::<String>()
            );
        }
        ran.stages.assert_exactly_once(&ran.frame, &format!("{what} region C"));
        assert_published_lines_end_at_their_counts(&ran.frame, &format!("{what} region C"));
    }
}

#[test]
fn w3_poisoned_then_smaller_then_larger_table_completes() {
    w3_body::<TestPolicy>("W3");
}

/// W3 under the finisher: the stale count is reset by whichever participant publishes the item.
#[test]
fn fin_w3_poisoned_then_smaller_then_larger_table_completes() {
    w3_body::<Fin<TestPolicy>>("fin W3");
}

// ---------------------------------------------------------------------------
// F-FRAME failure B (review r4 W1; ruling 17 B1)
// ---------------------------------------------------------------------------

/// Region B's rendezvous: participant 0 waits at most this long, inside block 0, for a helper to
/// start block 1. Spent only when no helper arrives (the round then reads vacuous, red).
const FRAME_B_RENDEZVOUS: Duration = Duration::from_secs(5);
/// How long the helper holds block 1 of item 0 unless item 1 runs meanwhile. Spent in full on every
/// green round: item 1 cannot run before block 1 is counted.
const FRAME_B_HOLD: Duration = Duration::from_millis(250);
/// Rounds of the test; each is decided by the rendezvous, not by a race.
const FRAME_B_ROUNDS: usize = 3;
/// A round's watchdog (the region runs the shipped, unbounded `V2Policy`).
const FRAME_B_WATCHDOG: Duration = Duration::from_secs(60);

/// Spins (yielding) until `flag` is set, at most `limit`; whether it was set.
fn wait_flag(flag: &AtomicBool, limit: Duration) -> bool {
    let t = Instant::now();
    // Acquire: pairs with the setter's Release store (the flags order no data; they are a rendezvous).
    while !flag.load(Ordering::Acquire) {
        if t.elapsed() > limit {
            return false;
        }
        std::thread::yield_now();
    }
    true
}

/// Region A: participant 0 panics in block 0 of entry 0, but only once a helper has finished
/// block 1, so the helper's count is the one A leaves on line 0 (`done[0] == 1`).
#[derive(Default)]
struct PoisonAfterAHelperCounted {
    helper_finished_b1: AtomicBool,
}

impl RegionStages for PoisonAfterAHelperCounted {
    fn run_block(&self, entry: u32, block: u32, participant: u32) {
        if entry == 0 && block == 0 && participant == 0 {
            let _ = wait_flag(&self.helper_finished_b1, FRAME_B_RENDEZVOUS);
            std::panic::panic_any(Injected { participant });
        }
        if entry == 0 && block == 1 && participant != 0 {
            self.helper_finished_b1.store(true, Ordering::Release);
        }
    }
}

/// Region B: participant 0's block 0 of item 0 returns once a helper has STARTED block 1; the
/// helper holds block 1 for up to [`FRAME_B_HOLD`] unless item 1 (entry 1, inline) runs meanwhile.
/// Item 1 running while block 1 is in flight is the overlap, seen from either side: the helper
/// sees `item1_ran` during its hold, or item 1 sees `b1_in_flight`.
#[derive(Default)]
struct OverlapProbe {
    b1_started: AtomicBool,
    b1_in_flight: AtomicBool,
    item1_ran: AtomicBool,
    overlap: AtomicBool,
}

impl RegionStages for OverlapProbe {
    fn run_block(&self, entry: u32, block: u32, participant: u32) {
        // SeqCst on `b1_in_flight` and `item1_ran` (a Dekker pair): if item 1 runs inside the
        // helper's block 1, at least one of the two sides reads the other's store.
        match (entry, block) {
            (0, 0) if participant == 0 => {
                let _ = wait_flag(&self.b1_started, FRAME_B_RENDEZVOUS);
            }
            (0, 1) if participant != 0 => {
                self.b1_in_flight.store(true, Ordering::SeqCst);
                self.b1_started.store(true, Ordering::Release);
                let t = Instant::now();
                while t.elapsed() < FRAME_B_HOLD {
                    if self.item1_ran.load(Ordering::SeqCst) {
                        self.overlap.store(true, Ordering::Relaxed);
                        break;
                    }
                    std::thread::yield_now();
                }
                self.b1_in_flight.store(false, Ordering::SeqCst);
            }
            (1, _) => {
                self.item1_ran.store(true, Ordering::SeqCst);
                if self.b1_in_flight.load(Ordering::SeqCst) {
                    self.overlap.store(true, Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }
}

/// A panic payload, as text.
fn payload_text(e: &(dyn Any + Send)) -> String {
    if let Some(b) = e.downcast_ref::<RegionWaitBound>() {
        format!("{b:?}")
    } else if let Some(s) = e.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else if let Some(i) = e.downcast_ref::<Injected>() {
        format!("{i:?}")
    } else {
        "a non-text payload".to_owned()
    }
}

/// One region of `P` (`V2Policy`, or its finisher twin) of `frame` at P = 2 through the public
/// entry, its panic caught.
fn frame_b_region<S: RegionStages, P: RegionPolicy>(
    pool: &ThreadPool,
    frame: &mut Frame,
    stages: &S,
) -> std::thread::Result<RegionReport> {
    pool.install(|_| {
        try_with_active_pool(|inner| {
            catch_unwind(AssertUnwindSafe(|| {
                // SAFETY: `frame` is this round's own, built by `Frame::new(2, 2, 2)` (two receipt
                // lines for P = 2) and never mixed with another `Frame`'s columns or counter; its
                // counter only grows, through `open`. Between regions A and B the round rewrites
                // the sync line's CONTENTS (`RegionLine::ZERO`): the same storage, still this
                // frame's alone, so the contract holds — and the state that leaves (a poisoned
                // region's count on line 0, no poison to show for it) is the input this test feeds
                // the region on purpose. The region below runs `P`, the policy `region_frame`
                // checks the table's claim layout for.
                let region_frame = unsafe { frame.region_frame::<P>(2) };
                inner.region::<S, P, false>(region_frame, stages)
            }))
        })
        .expect("test setup: install sets the active pool")
    })
}

/// One round under `P`: A poisoned with `done[0] == 1`, the sync line re-created, then B.
fn frame_b_round<P: RegionPolicy>() -> Result<(), String> {
    let p = 2;
    let pool = pool(p);
    let mut frame = Frame::new(2, p as usize, 2);
    frame.set_table::<P>(&[2, 1], &[0], p, 0);
    let mut left = 0;
    for _ in 0..ATTEMPTS {
        let a = PoisonAfterAHelperCounted::default();
        let r = frame_b_region::<_, P>(&pool, &mut frame, &a);
        left = frame.done[0].word(0);
        if r.is_err() && left == 1 {
            break;
        }
    }
    if left != 1 {
        return Err(format!("vacuous: region A never left done line 0 at 1 in {ATTEMPTS} attempts (last {left})"));
    }
    // The caller re-creates its sync line: the poison A left is gone, so B's open has no poison
    // to react to, and only the done line still says what A did.
    frame.sync = RegionLine::ZERO;
    frame.set_table::<P>(&[2, 1], &[0, 1], p, 0);
    let b = OverlapProbe::default();
    if let Err(e) = frame_b_region::<_, P>(&pool, &mut frame, &b) {
        return Err(format!("region B panicked: {}", payload_text(&*e)));
    }
    if !b.b1_started.load(Ordering::Acquire) {
        return Err("vacuous: no helper reached block 1 of item 0 within the rendezvous".to_owned());
    }
    if b.overlap.load(Ordering::Relaxed) {
        return Err(
            "OVERLAP: item 1 ran while a helper was inside item 0's block 1 — A's stale count \
             completed item 0 early"
                .to_owned(),
        );
    }
    Ok(())
}

/// F-FRAME failure B: a stale done line (a poisoned region's count) under a re-created sync line
/// never completes an item early. Red before ruling 17 B1 in release (overlap) and debug (the
/// open's done-line check); `V2Policy` is unbounded, so each round runs under a watchdog. The
/// `Fin<V2Policy>` rounds (CR-F) hold B1 to "whichever participant advances": in B, participant
/// 0 publishes item 0 at the region's start, and its own add is not the completing one (the stale
/// count was reset), so it reaches item 1 only through its done-wait.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: the overlap is detected by a 250 ms wall-clock hold and a 5 s rendezvous under a 60 s watchdog, proportions Miri's interpreter does not keep"
)]
fn frame_b_a_stale_done_line_never_completes_an_item_early() {
    for round in 0..FRAME_B_ROUNDS {
        match within(FRAME_B_WATCHDOG, frame_b_round::<V2Policy>) {
            Some(Ok(())) => {}
            Some(Err(e)) => panic!("F-FRAME B round {round}: {e}"),
            None => panic!("F-FRAME B round {round}: no verdict within {FRAME_B_WATCHDOG:?} (a hang)"),
        }
    }
    for round in 0..FRAME_B_ROUNDS {
        match within(FRAME_B_WATCHDOG, frame_b_round::<Fin<V2Policy>>) {
            Some(Ok(())) => {}
            Some(Err(e)) => panic!("F-FRAME B fin round {round}: {e}"),
            None => panic!("F-FRAME B fin round {round}: no verdict within {FRAME_B_WATCHDOG:?} (a hang)"),
        }
    }
}

// ---------------------------------------------------------------------------
// G-CR-F-PANIC (CR-F): a helper panics while participant 0 waits for a publish
// ---------------------------------------------------------------------------

/// G-CR-F-PANIC's watchdog, per attempt.
const GCRF_WATCHDOG: Duration = Duration::from_secs(10);
/// How long the panicking helper holds block 1 before it panics: participant 0 has left block 0,
/// swept the rest of the item and entered its publish wait well within it.
const GCRF_LATE: Duration = Duration::from_millis(5);

/// G-CR-F-PANIC: under the unbounded `Fin<V2Policy>`, at W = 2, 4, 8, entries `[2W, 2W]`, items
/// `[0, 1]`. Entry 0's block 0 is participant 0's first claim and holds participant 0 until block 1
/// has started (`Rendezvous`), so block 1 is claimed by a helper — participant 0 is inside block 0
/// — and that helper panics [`GCRF_LATE`] later (`LatePanic`, helpers only), in stage entry 0, on
/// the helper role entry (`run_helper_participant`'s finisher). Item 0 then never completes and no
/// participant can publish an END but participant 0 itself, which by then waits in its publish
/// wait: its poison poll there is its only way out. The region must re-raise the injected payload
/// (the helper's) within the 10 s watchdog, with participant 0's receipt POISONED and exactly one
/// PANICKED receipt, the helper's. An attempt whose rendezvous ran out (participant 0 then claims
/// block 1 itself and the helper-only panic never fires) completes normally and is retried, up to
/// [`ATTEMPTS`]; never firing is red (`fired` anti-vacuity).
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: a 10 s wall-clock watchdog over an unbounded policy and a 5 ms hold that places participant 0 in its publish wait, proportions Miri's interpreter does not keep"
)]
fn fin_helper_panic_while_participant0_waits() {
    for w in [2u32, 4, 8] {
        let pool = pool(w);
        let mut frame = Frame::new(2, w as usize, 4 * w as usize);
        frame.set_table::<Fin<V2Policy>>(&[(2 * w) as u16, (2 * w) as u16], &[0, 1], w, 0);
        let mut fired = false;
        for attempt in 0..ATTEMPTS {
            let base = frame.epoch.max(1);
            let stages = Stages::new(&frame)
                .with_rendezvous(Rendezvous { entry: 0, hold: Duration::ZERO, sleep: false })
                .with_late_helper_panic(LatePanic { entry: 0, block: 1, delay: GCRF_LATE });
            let route = if attempt % 2 == 0 { Route::External } else { Route::Worker };
            let on = Arc::clone(&pool);
            let ran = within(GCRF_WATCHDOG, move || run_region::<Fin<V2Policy>, false>(&on, route, frame, Arc::new(stages), w))
                .unwrap_or_else(|| {
                    panic!(
                        "G-CR-F-PANIC W{w} attempt {attempt}: no verdict within {GCRF_WATCHDOG:?} (participant 0 never left its publish wait)"
                    )
                });
            let what = format!("G-CR-F-PANIC W{w} attempt {attempt} ({route:?})");
            let fired_now = ran.stages.fired.load(Ordering::Relaxed);
            if ran.result.is_err() {
                assert!(fired_now, "{what}: the region panicked but the injection did not fire");
                assert_poisoned(&ran, w, base, Who::Helper, &what);
                assert_eq!(
                    ran.frame.receipts(w)[0].exit,
                    Some(RegionExit::Poisoned),
                    "{what}: participant 0 left through its poison poll"
                );
                fired = true;
                break;
            }
            assert!(!fired_now, "{what}: the injection fired but the region returned normally");
            frame = ran.frame;
        }
        assert!(fired, "G-CR-F-PANIC W{w}: the injection never fired in {ATTEMPTS} attempts");
    }
}

/// Miri's Stacked Borrows leg: W8 and W3 on the pool-free twin (`region_on_threads`). Participant 0
/// injections only — `std::thread::scope` replaces a helper's payload with its own message.
#[cfg(miri)]
#[test]
fn threads_w8_and_w3_after_a_caught_poisoned_region() {
    threads_w8_and_w3::<TestPolicy>("threads");
    threads_w8_and_w3::<Fin<TestPolicy>>("threads fin");
}

/// The SB leg's body for one policy (the orchestrator's, and the finisher's twin).
#[cfg(miri)]
fn threads_w8_and_w3<P: RegionPolicy>(label: &str) {
    use region_common::run_threads;
    fn poisoned<P: RegionPolicy>(mut frame: Frame, p: u32, inject: Inject) -> (Ran, u64) {
        for _ in 0..ATTEMPTS {
            let base = frame.epoch.max(1);
            let stages = Arc::new(Stages::new(&frame).with_inject(inject));
            let ran = run_threads::<P, false>(frame, stages, p);
            if ran.result.is_err() {
                return (ran, base);
            }
            frame = ran.frame;
        }
        panic!("threads P{p}: the injection never fired");
    }
    for p in [2u32, 3] {
        // W8, participant 0 in an inline item and in a claimed block.
        for kind in [Kind::OrchestratorInline, Kind::OrchestratorClaimed] {
            let (blocks, order, inject) = table(kind, p);
            let mut frame = Frame::new(blocks.len(), p as usize, 64);
            frame.set_table::<P>(&blocks, &order, p, 0);
            let what = format!("{label} W8 {kind:?} P{p}");
            let (ran, base_a) = poisoned::<P>(frame, p, inject);
            assert_poisoned(&ran, p, base_a, Who::Orchestrator, &what);
            let frame = ran.frame;
            let len = frame.schedule.len() as u64;
            let stages = Arc::new(Stages::new(&frame));
            let ran = run_threads::<P, false>(frame, stages, p);
            let r = ran.result.as_ref().unwrap_or_else(|_| panic!("{what}: region B panicked"));
            assert_eq!(r.base, base_a + len + 1, "{what}: B.base");
            ran.stages.assert_exactly_once(&ran.frame, &format!("{what} region B"));
        }
        // W3: A poisoned with entry 2 partial, then two entries, then four.
        let wide = (2 * p) as u16;
        let mut frame = Frame::new(4, p as usize, 64);
        frame.set_table::<P>(&[1, 1, wide], &[0, 2, 1, 2], p, 0);
        let what = format!("{label} W3 P{p}");
        let (ran, base_a) = poisoned::<P>(frame, p, Inject { entry: 2, block: 1, who: Who::Orchestrator });
        assert_poisoned(&ran, p, base_a, Who::Orchestrator, &what);
        let mut frame = ran.frame;
        let stale = frame.done[2].word(0);
        assert_ne!(stale, 0, "{what}: precondition");
        frame.set_table::<P>(&[wide, 1], &[0, 1, 0], p, 0);
        let stages = Arc::new(Stages::new(&frame));
        let ran = run_threads::<P, false>(frame, stages, p);
        assert!(ran.result.is_ok(), "{what}: region B completes");
        let mut frame = ran.frame;
        assert_eq!(frame.done[2].word(0), stale, "{what}: B left A's stale count on line 2");
        frame.set_table::<P>(&[1, wide, wide + 1, 3], &[2, 0, 1, 2, 3, 2], p, 0);
        let stages = Arc::new(Stages::new(&frame));
        let ran = run_threads::<P, false>(frame, stages, p);
        assert!(ran.result.is_ok(), "{what}: region C completes");
        ran.stages.assert_exactly_once(&ran.frame, &format!("{what} region C"));
        assert_published_lines_end_at_their_counts(&ran.frame, &format!("{what} region C"));
    }
}
