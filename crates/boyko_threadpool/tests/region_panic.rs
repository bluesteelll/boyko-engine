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
//! **W3** (critique r1): A is poisoned while entry 2's done line holds a partial count; B has only
//! two entries; C has four again. The open reset after A must clear EVERY done line of the column,
//! or C's entry 2 starts from A's stale count.

mod region_common;

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use boyko_threadpool::{RegionExit, RegionReceipt, RegionReport};

use region_common::{
    Frame, Inject, Injected, Ran, Route, Stages, TestPolicy, Who, panic_participant_counts, pool,
    run_region,
};

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

/// Runs the injected region until the injection fires (at most [`ATTEMPTS`]); returns the run.
fn poisoned_run(pool: &Arc<boyko_threadpool::ThreadPool>, mut frame: Frame, kind: Kind, p: u32, inject: Inject) -> (Ran, u64) {
    for attempt in 0..ATTEMPTS {
        let base = frame.epoch.max(1);
        let mut stages = Stages::new(&frame).with_inject(inject);
        if matches!(kind, Kind::HelperClaimed) {
            // O9: participant 0 spends 50 µs per block of the entry, and (so that a parked helper's
            // wake latency cannot decide the case) its block 0 waits until a helper has started.
            stages = stages.with_slow(0, Duration::from_micros(50)).with_await_helper(0);
        }
        let route = if attempt % 2 == 0 { Route::External } else { Route::Worker };
        let ran = run_region::<TestPolicy, false>(pool, route, frame, Arc::new(stages), p);
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

fn case(kind: Kind, who: Who) {
    for &p in panic_participant_counts() {
        let pool = pool(p);
        let (blocks, order, inject) = table(kind, p);
        let mut frame = Frame::new(blocks.len(), p as usize, 4 * 16 * 8);
        frame.set_table::<TestPolicy>(&blocks, &order, p, 0);
        let (ran, base) = poisoned_run(&pool, frame, kind, p, inject);
        assert_poisoned(&ran, p, base, who, &format!("{kind:?} P{p}"));
    }
}

#[test]
fn case1_orchestrator_panics_in_an_inline_entry() {
    case(Kind::OrchestratorInline, Who::Orchestrator);
}

#[test]
fn case2_orchestrator_panics_in_a_claimed_block() {
    case(Kind::OrchestratorClaimed, Who::Orchestrator);
}

/// Case 3: the region's own `n_blocks ≤ 8·P` debug assertion fires on participant 0, on an
/// oversize entry placed after a multi-block one. Debug profile only (it is a `debug_assert!`).
#[cfg(debug_assertions)]
#[test]
fn case3_a_region_debug_assertion_fires_on_the_orchestrator() {
    for &p in panic_participant_counts() {
        let pool = pool(p);
        let oversize = (8 * p + 1) as u16;
        let mut frame = Frame::new(2, p as usize, 4 * 16 * 8 + 64);
        frame.set_table::<TestPolicy>(&[(2 * p) as u16, oversize], &[0, 1], p, 0);
        let base = frame.epoch.max(1);
        let stages = Arc::new(Stages::new(&frame));
        let ran = run_region::<TestPolicy, false>(&pool, Route::External, frame, stages, p);
        let what = format!("case 3 P{p}");
        let payload = ran.result.as_ref().expect_err("case 3: the region panicked");
        let msg = payload
            .downcast_ref::<String>()
            .unwrap_or_else(|| panic!("{what}: (a) the payload is not the assertion's message"));
        assert!(msg.contains("more than 8 per participant"), "{what}: (a) the payload is {msg:?}");
        assert_receipts(&ran.frame.receipts(p), base, 0, &what);
        if !cfg!(miri) {
            assert!(ran.wall < Duration::from_secs(1), "{what}: (c) the call took {:?}", ran.wall);
        }
    }
}

#[test]
fn case4_a_helper_panics_in_a_claimed_block() {
    case(Kind::HelperClaimed, Who::Helper);
}

/// W8: a caught poisoned region, then a healthy region over the same frame, at P = 2, 8, 16, for
/// every injection kind.
#[test]
fn w8_poisoned_region_then_same_frame_completes() {
    for &p in panic_participant_counts() {
        let pool = pool(p);
        for kind in [Kind::OrchestratorInline, Kind::OrchestratorClaimed, Kind::HelperClaimed] {
            let (blocks, order, inject) = table(kind, p);
            let mut frame = Frame::new(blocks.len(), p as usize, 4 * 16 * 8);
            frame.set_table::<TestPolicy>(&blocks, &order, p, 0);
            let what = format!("W8 {kind:?} P{p}");
            let (ran, base_a) = poisoned_run(&pool, frame, kind, p, inject);
            let who = if matches!(kind, Kind::HelperClaimed) { Who::Helper } else { Who::Orchestrator };
            assert_poisoned(&ran, p, base_a, who, &what);
            let frame = ran.frame;
            let len = frame.schedule.len() as u64;
            let epoch_after_a = frame.epoch;
            // Region B: the SAME frame and table, no injection. Its outcome is asserted before the
            // counter arithmetic, so mutation M-W8 reds on B's behaviour (the cut's prediction).
            let stages = Arc::new(Stages::new(&frame));
            let ran = run_region::<TestPolicy, false>(&pool, Route::External, frame, stages, p);
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

/// W3: A poisoned with entry 2's done line partial; B with two entries; C with four. Every done
/// line must be 0 after B's open (the reset clears the column, not B's table) and after C.
#[test]
fn w3_poisoned_then_smaller_then_larger_table_completes() {
    for &p in panic_participant_counts() {
        let pool = pool(p);
        let wide = (2 * p) as u16;
        let mut frame = Frame::new(4, p as usize, 4 * 16 * 8);
        // A: entry 2 is the in-flight one; participant 0 panics in its block 1 after counting
        // block 0, so its done line is left at r >= 1.
        frame.set_table::<TestPolicy>(&[1, 1, wide], &[0, 2, 1, 2], p, 0);
        let what = format!("W3 P{p}");
        let inject = Inject { entry: 2, block: 1, who: Who::Orchestrator };
        let (ran, base_a) = poisoned_run(&pool, frame, Kind::OrchestratorClaimed, p, inject);
        assert_poisoned(&ran, p, base_a, Who::Orchestrator, &what);
        let mut frame = ran.frame;
        let stale = frame.done[2].word(0);
        assert_ne!(stale, 0, "{what}: precondition — A left entry 2's done line partial");
        // B: two entries (lines 0 and 1 only).
        frame.set_table::<TestPolicy>(&[wide, 1], &[0, 1, 0], p, 0);
        let stages = Arc::new(Stages::new(&frame));
        let ran = run_region::<TestPolicy, false>(&pool, Route::External, frame, stages, p);
        if let Err(e) = &ran.result {
            panic!(
                "{what}: region B panicked: {:?} / {:?}",
                e.downcast_ref::<boyko_threadpool::RegionWaitBound>(),
                e.downcast_ref::<String>()
            );
        }
        ran.stages.assert_exactly_once(&ran.frame, &format!("{what} region B"));
        let mut frame = ran.frame;
        for (e, line) in frame.done.iter().enumerate() {
            assert_eq!(line.word(0), 0, "{what}: after B, done line {e} (A left {stale} on line 2)");
        }
        // C: four entries again; entry 2 executes three times.
        frame.set_table::<TestPolicy>(&[1, wide, wide + 1, 3], &[2, 0, 1, 2, 3, 2], p, 0);
        let stages = Arc::new(Stages::new(&frame));
        let ran = run_region::<TestPolicy, false>(&pool, Route::Worker, frame, stages, p);
        let ok = match &ran.result {
            Ok(_) => true,
            Err(e) => panic!(
                "{what}: region C panicked: {:?} / {:?}",
                e.downcast_ref::<boyko_threadpool::RegionWaitBound>(),
                e.downcast_ref::<String>()
            ),
        };
        assert!(ok);
        ran.stages.assert_exactly_once(&ran.frame, &format!("{what} region C"));
        for (e, line) in ran.frame.done.iter().enumerate() {
            assert_eq!(line.word(0), 0, "{what}: after C, done line {e}");
        }
    }
}

/// Miri's Stacked Borrows leg: W8 and W3 on the pool-free twin (`region_on_threads`). Participant 0
/// injections only — `std::thread::scope` replaces a helper's payload with its own message.
#[cfg(miri)]
#[test]
fn threads_w8_and_w3_after_a_caught_poisoned_region() {
    use region_common::run_threads;
    fn poisoned(mut frame: Frame, p: u32, inject: Inject) -> (Ran, u64) {
        for _ in 0..ATTEMPTS {
            let base = frame.epoch.max(1);
            let stages = Arc::new(Stages::new(&frame).with_inject(inject));
            let ran = run_threads::<TestPolicy, false>(frame, stages, p);
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
            frame.set_table::<TestPolicy>(&blocks, &order, p, 0);
            let what = format!("threads W8 {kind:?} P{p}");
            let (ran, base_a) = poisoned(frame, p, inject);
            assert_poisoned(&ran, p, base_a, Who::Orchestrator, &what);
            let frame = ran.frame;
            let len = frame.schedule.len() as u64;
            let stages = Arc::new(Stages::new(&frame));
            let ran = run_threads::<TestPolicy, false>(frame, stages, p);
            let r = ran.result.as_ref().unwrap_or_else(|_| panic!("{what}: region B panicked"));
            assert_eq!(r.base, base_a + len + 1, "{what}: B.base");
            ran.stages.assert_exactly_once(&ran.frame, &format!("{what} region B"));
        }
        // W3: A poisoned with entry 2 partial, then two entries, then four.
        let wide = (2 * p) as u16;
        let mut frame = Frame::new(4, p as usize, 64);
        frame.set_table::<TestPolicy>(&[1, 1, wide], &[0, 2, 1, 2], p, 0);
        let what = format!("threads W3 P{p}");
        let (ran, base_a) = poisoned(frame, p, Inject { entry: 2, block: 1, who: Who::Orchestrator });
        assert_poisoned(&ran, p, base_a, Who::Orchestrator, &what);
        let mut frame = ran.frame;
        assert_ne!(frame.done[2].word(0), 0, "{what}: precondition");
        frame.set_table::<TestPolicy>(&[wide, 1], &[0, 1, 0], p, 0);
        let stages = Arc::new(Stages::new(&frame));
        let ran = run_threads::<TestPolicy, false>(frame, stages, p);
        assert!(ran.result.is_ok(), "{what}: region B completes");
        let mut frame = ran.frame;
        frame.set_table::<TestPolicy>(&[1, wide, wide + 1, 3], &[2, 0, 1, 2, 3, 2], p, 0);
        let stages = Arc::new(Stages::new(&frame));
        let ran = run_threads::<TestPolicy, false>(frame, stages, p);
        assert!(ran.result.is_ok(), "{what}: region C completes");
        ran.stages.assert_exactly_once(&ran.frame, &format!("{what} region C"));
        for (e, line) in ran.frame.done.iter().enumerate() {
            assert_eq!(line.word(0), 0, "{what}: after C, done line {e}");
        }
    }
}
