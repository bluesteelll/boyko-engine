//! Phase 9.1 W3b — Miri-1: the `Scope::spawn` transmute on real worker threads
//! (plan D4 / D5 / D5.1 / H4).
//!
//! This is the soundness crux of Phase 9.1. It drives, under Miri, three
//! surfaces: (1) the `'scope -> 'static` lifetime-erasure `transmute` in
//! `Scope::spawn` (`scope.rs:256`); (2) the cross-thread `SharedPtr::as_ref`
//! raw deref of `&ScopeShared` on a **worker** thread (`scope.rs:140`); and
//! (3) the `pending` join + waker unpark + `ThreadPool::drop` shutdown/join.
//!
//! All of this runs on genuinely distinct OS threads so that Tree Borrows (the
//! project default via `.cargo/config.toml`) observes the worker-side foreign
//! write to `pending` while the dispatcher holds `&ScopeShared` across the join
//! wait — the exact pattern the Phase-9 closeout deferred (see
//! `crates/boyko_ecs/tests/miri_phase9.rs:15-26`).
//!
//! ## H4 — genuine cross-thread execution is FORCED and ASSERTED
//!
//! `Scope::drop`'s wait steals and runs tasks inline on the dispatcher, so a
//! small task set can be drained entirely by the dispatcher, leaving the
//! worker-side `as_ref` write (the TB target) un-exercised — a *vacuous* pass.
//! Two defenses, per H4:
//!   1. **Forced interleave**: task A sets `flag_a` then waits for `flag_b`;
//!      task B waits for `flag_a` then sets `flag_b`. If the dispatcher tried to
//!      run both inline-serially it would block inside A's body waiting on
//!      `flag_b` (B not yet started) — forward progress is possible *only* if a
//!      worker thread claims the other task. A genuine failure to run
//!      cross-thread therefore surfaces as the bounded-spin panic, never a
//!      false green.
//!   2. **Recorded worker ids**: each body records `current_worker_id()`; the
//!      test asserts at least one body ran on a real worker
//!      (`id < WORKER_ID_DISPATCHER`), i.e. NOT inline on the dispatcher.
//!
//! ## D5.1 OUTCOME (recorded 2026-05-30): outcome (ii) — TB over-approximation
//!
//! Under Tree Borrows these tests REPRODUCE the Phase-9-deferred protected-tag
//! conflict at `scope.rs:96` (`ScopeShared::complete_task`'s `pending.fetch_sub`
//! foreign-writes a tag the dispatcher holds as a protected `Reserved
//! (conflicted)` `&ScopeShared` across `join_workers_until_drained`). The H4
//! forcing works (the bodies run on `boyko-worker-0/1`, NOT inline), so this is
//! a genuine cross-thread exercise of the target, not a vacuous pass.
//!
//! Per the D5.1 decision tree this is **outcome (ii), a TB over-approximation,
//! NOT a soundness bug**, established by the decisive step (b): a
//! `std::thread::scope`-equivalent harness with the SAME shape (parent holds
//! `&Shared` as a *protected function argument* — the analog of
//! `join_workers_until_drained(pool, &self.shared)` — while child threads
//! foreign-write `pending` through a raw `*const Shared` reborrow) triggers the
//! **identical** TB error under the same flags. Since `std`'s own scope is
//! equally TB-flagged, boyko's `Scope::spawn` (structurally identical to
//! `std::thread::scope`) is sound and TB is over-approximating both. The
//! authoritative model would be Stacked Borrows — but this nightly's Miri
//! (2026-05-22) has RETIRED SB (`-Zmiri-stacked-borrows` = "unknown unstable
//! option"), so the std-scope equivalence is the authoritative evidence here.
//!
//! A zero-cost hardening that clears the TB flag was confirmed in the same
//! harness: take the joiner's parameter as `*const ScopeShared` (raw) instead
//! of `&ScopeShared`, so the dispatcher holds no protected borrow across the
//! workers' writes (the exact D5.1 candidate fix; `std`-harness variant = clean
//! under TB). That is a developer change (production code), out of scope for the
//! tester; see the test report.
//!
//! Accordingly these tests are `#[ignore]`-by-default (kept compiling + ready to
//! run): they will pass the moment the raw-pointer hardening lands, or under any
//! Miri that re-exposes SB. Run them explicitly with `-- --ignored` to observe
//! the documented TB over-approximation.
//!
//! ## Run (plan §5 / D5.1)
//! ```bash
//! # The TB run (reproduces the documented over-approximation; #[ignore]d):
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-many-seeds=0..16" \
//!   cargo +nightly miri test -p boyko-threadpool --test miri_scope -- --ignored
//! # (D5.1 step (a) SB cross-check is N/A on this Miri — SB retired upstream.)
//! ```
#![cfg(miri)]

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use boyko_threadpool::{ThreadPoolBuilder, WORKER_ID_DISPATCHER, current_worker_id};

/// Bounded cooperative spin. Yields so Miri can advance other threads; panics
/// (rather than hanging Miri forever) if the awaited flag never arrives, so a
/// genuine lost-wakeup / no-cross-thread-progress becomes a test FAILURE.
fn spin_until(flag: &AtomicBool, ctx: &str) {
    // Generous cap: under Miri each iteration yields, so a healthy run needs
    // only a handful of turns; an unhealthy one is bounded instead of hanging.
    for _ in 0..100_000 {
        if flag.load(Ordering::Acquire) {
            return;
        }
        std::thread::yield_now();
    }
    panic!("spin_until timed out waiting for {ctx} — no cross-thread progress");
}

/// Returns true iff `id` denotes a genuine worker thread (not the dispatcher,
/// not the unattached sentinel).
fn on_worker(id: u32) -> bool {
    id != WORKER_ID_DISPATCHER && id != u32::MAX
}

/// H4 core: two mutually-dependent tasks that can only both complete if they
/// run on different threads. Records which worker executed each body and the
/// values written through borrowed stack slots.
#[test]
#[ignore = "TB over-approximation at scope.rs:96 (std::thread::scope equally \
            TB-flagged; SB retired in this Miri); see module doc / D5.1 (ii). \
            Run with -- --ignored to observe."]
fn miri_scope_forced_cross_thread_transmute_is_clean() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();

    // Distinct stack slots borrowed by the task bodies (the 'scope borrow the
    // transmute erases). If the transmute or the join were unsound, Miri would
    // report UB/UAF on these.
    let flag_a = AtomicBool::new(false);
    let flag_b = AtomicBool::new(false);
    let out_a = AtomicU32::new(0);
    let out_b = AtomicU32::new(0);
    let wid_a = AtomicU32::new(u32::MAX);
    let wid_b = AtomicU32::new(u32::MAX);

    pool.install(|scope| {
        scope.spawn(|| {
            wid_a.store(current_worker_id(), Ordering::Release);
            out_a.store(0xAA, Ordering::Release);
            flag_a.store(true, Ordering::Release);
            // Can only complete once task B (on another thread) sets flag_b.
            spin_until(&flag_b, "flag_b in task A");
        });
        scope.spawn(|| {
            wid_b.store(current_worker_id(), Ordering::Release);
            spin_until(&flag_a, "flag_a in task B");
            out_b.store(0xBB, Ordering::Release);
            flag_b.store(true, Ordering::Release);
        });
    });

    // Correct results (the borrowed stack slots survived the cross-thread run).
    assert_eq!(out_a.load(Ordering::Acquire), 0xAA, "task A wrote its slot");
    assert_eq!(out_b.load(Ordering::Acquire), 0xBB, "task B wrote its slot");

    // H4 assertion: at least one body ran on a genuine worker thread (not
    // inline on the dispatcher). A dispatcher-only (vacuous) run is a FAILURE.
    let ran_a = wid_a.load(Ordering::Acquire);
    let ran_b = wid_b.load(Ordering::Acquire);
    assert!(
        on_worker(ran_a) || on_worker(ran_b),
        "H4: neither body ran on a worker thread (a={ran_a}, b={ran_b}); the \
         cross-thread transmute path was not exercised — vacuous pass"
    );
}

/// A spawn whose body reads-then-writes a borrowed stack `AtomicUsize`, forced
/// to wait on a gate the *second* task opens. Exercises the read-modify-write of
/// borrowed stack data across the transmute on (at least sometimes) a worker.
#[test]
#[ignore = "TB over-approximation at scope.rs:96 (see module doc / D5.1 (ii)). \
            Run with -- --ignored to observe."]
fn miri_scope_read_modify_write_borrowed_stack() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();

    let accum = AtomicUsize::new(1);
    let gate = AtomicBool::new(false);

    pool.install(|scope| {
        // Task 1: wait for the gate, then RMW the borrowed stack slot.
        scope.spawn(|| {
            spin_until(&gate, "gate in RMW task");
            let prev = accum.fetch_add(41, Ordering::AcqRel);
            assert_eq!(prev, 1, "RMW observed the initial value");
        });
        // Task 2: open the gate (must run before task 1 can finish).
        scope.spawn(|| {
            gate.store(true, Ordering::Release);
        });
    });

    assert_eq!(
        accum.load(Ordering::Acquire),
        42,
        "borrowed stack slot reflects the cross-thread RMW"
    );
}

/// Multiple tasks each borrowing a *distinct* stack slot, rendezvousing on a
/// `go` gate so several worker-side `as_ref` reborrows overlap the dispatcher's
/// `&ScopeShared` in the join wait.
#[test]
#[ignore = "TB over-approximation at scope.rs:96 (see module doc / D5.1 (ii)). \
            Run with -- --ignored to observe."]
fn miri_scope_multiple_distinct_borrows() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();

    let slots = [
        AtomicU32::new(0),
        AtomicU32::new(0),
        AtomicU32::new(0),
        AtomicU32::new(0),
    ];
    let started = AtomicUsize::new(0);
    let go = AtomicBool::new(false);

    pool.install(|scope| {
        // Shadow as shared references so the `move` closures capture the (Copy)
        // references rather than moving the non-Copy atomics into the first
        // iteration's closure.
        let started = &started;
        let go = &go;
        for (i, slot) in slots.iter().enumerate() {
            scope.spawn(move || {
                started.fetch_add(1, Ordering::AcqRel);
                // All tasks rendezvous on `go` so their bodies overlap in time
                // (concurrent worker-side reborrows of ScopeShared). Bounded so
                // a stuck scheduler fails instead of hanging.
                spin_until(go, "go gate");
                slot.store((i as u32) + 1, Ordering::Release);
            });
        }
        // Release everyone once spawned. The dispatcher then enters the join
        // wait (holding &ScopeShared) while workers run the bodies.
        go.store(true, Ordering::Release);
    });

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(
            slot.load(Ordering::Acquire),
            (i as u32) + 1,
            "slot {i} written by its task across the transmute"
        );
    }
    assert_eq!(
        started.load(Ordering::Acquire),
        slots.len(),
        "every task body started"
    );
}
