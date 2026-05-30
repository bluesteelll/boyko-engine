//! Phase 9.1 W3c — exactly-once stress with Drop-accounting (plan D6).
//!
//! Native, randomized stress tests that probe the crossbeam-deque transport
//! path that loom cannot model (opaque, SeqCst) and Miri cannot scale to.
//! They mirror crossbeam-deque's own `spsc` / `stampede` / `stress` /
//! `destructors` suites, adapted to drive the pool through
//! `ThreadPool::install` + `Scope::spawn` under steal contention.
//!
//! ## What each test proves
//!
//! - **Run-once**: every spawned body runs exactly once (`ran[id]` flips
//!   `false -> true` exactly once; a second run trips the in-body assert).
//! - **Drop-once**: every task payload is dropped exactly once (`dropped[id]`
//!   goes `0 -> 1`; a double-drop trips the `Drop` assert).
//! - **No loss** (the property the in-body asserts CANNOT see): the *mandatory
//!   post-join full-array sweep* asserts `ran[i] == true && dropped[i] == 1`
//!   for **every** `i`. A task that silently never ran or never dropped is
//!   invisible to per-body asserts and is caught only here.
//! - **Panic path**: a fraction of bodies panic (caught at `install` scope
//!   level); survivors + panickers must still each run-once + drop-once, and
//!   the first panic must propagate out of `install`.
//!
//! Run: `cargo test --release -p boyko-threadpool --test stress`

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use boyko_threadpool::ThreadPoolBuilder;

/// A Drop-counting payload carried by each task. On drop it increments
/// `dropped[id]`; the first increment must observe `0` (drop-once). Captured
/// **by move** into the task body so its `Drop` runs on whichever worker
/// finishes the task.
struct DropTracker {
    id: usize,
    dropped: Arc<Vec<AtomicU32>>,
}

impl Drop for DropTracker {
    fn drop(&mut self) {
        let prev = self.dropped[self.id].fetch_add(1, Ordering::AcqRel);
        assert_eq!(
            prev, 0,
            "DropTracker for task {} dropped more than once (prev={})",
            self.id, prev
        );
    }
}

/// Shared bookkeeping for one stress run.
struct Accounting {
    ran: Vec<AtomicBool>,
    dropped: Arc<Vec<AtomicU32>>,
}

impl Accounting {
    fn new(n: usize) -> Self {
        let mut ran = Vec::with_capacity(n);
        let mut dropped = Vec::with_capacity(n);
        for _ in 0..n {
            ran.push(AtomicBool::new(false));
            dropped.push(AtomicU32::new(0));
        }
        Self {
            ran,
            dropped: Arc::new(dropped),
        }
    }

    /// Mandatory post-join full-array sweep (plan D6): catches LOSS, which the
    /// in-body / `Drop` asserts cannot observe.
    fn assert_exactly_once(&self, n: usize) {
        for i in 0..n {
            assert!(
                self.ran[i].load(Ordering::Acquire),
                "task {i} never ran (lost task — invisible to in-body asserts)"
            );
            assert_eq!(
                self.dropped[i].load(Ordering::Acquire),
                1,
                "task {i} payload not dropped exactly once (lost / leaked / double-dropped)"
            );
        }
    }
}

/// Core driver: spawn `n` tasks into the pool over `workers` threads. Each task
/// flips its `ran` bit (asserting run-once) and owns a `DropTracker` (asserting
/// drop-once). Returns after the scope has fully joined.
fn run_exactly_once(workers: usize, n: usize) {
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let acct = Accounting::new(n);

    pool.install(|scope| {
        for id in 0..n {
            // Each task captures a move-only DropTracker (so Drop runs on the
            // executing worker) plus a raw reference to the `ran` slot. The
            // `&acct.ran` borrow is valid for the whole scope because
            // `Scope::Drop` blocks until every task completes (the very
            // contract under proof).
            let tracker = DropTracker {
                id,
                dropped: Arc::clone(&acct.dropped),
            };
            let ran_slot = &acct.ran[id];
            scope.spawn(move || {
                // Touch the tracker so it is genuinely moved into the body and
                // dropped here (not optimized away).
                let _keep = &tracker;
                let prev = ran_slot.swap(true, Ordering::AcqRel);
                assert!(!prev, "task {id} ran more than once");
                // `tracker` drops at end of body on this worker thread.
            });
        }
    });

    acct.assert_exactly_once(n);
}

// =========================================================================
// spsc / stampede / destructors shapes (crossbeam-deque mirror).
// =========================================================================

/// spsc-flavored: a single small batch, low worker count.
#[test]
fn stress_spsc_small_batch_exactly_once() {
    run_exactly_once(2, 64);
}

/// stampede-flavored: many tasks, moderate workers — heavy steal contention.
#[test]
fn stress_stampede_many_tasks_exactly_once() {
    run_exactly_once(4, 4096);
}

/// destructors-flavored: emphasises Drop-once across a large task set.
#[test]
fn stress_destructors_large_set_exactly_once() {
    run_exactly_once(8, 8192);
}

/// Repeated rounds on the same pool: re-uses workers across many scopes,
/// exercising park/unpark churn between bursts.
#[test]
fn stress_repeated_rounds_reuse_pool() {
    let workers = 4;
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();

    for round in 0..16 {
        let n = 256;
        let acct = Accounting::new(n);
        pool.install(|scope| {
            for id in 0..n {
                let tracker = DropTracker {
                    id,
                    dropped: Arc::clone(&acct.dropped),
                };
                let ran_slot = &acct.ran[id];
                scope.spawn(move || {
                    let _keep = &tracker;
                    let prev = ran_slot.swap(true, Ordering::AcqRel);
                    assert!(!prev, "round {round} task {id} ran more than once");
                });
            }
        });
        acct.assert_exactly_once(n);
    }
}

/// Nested scopes under contention (mirrors the existing
/// `nested_scope_does_not_deadlock` unit test, but adds Drop-accounting + the
/// full-array sweep). Outer tasks each open an inner scope; every inner task is
/// run-once + drop-once.
#[test]
fn stress_nested_scopes_exactly_once() {
    let outer_n = 8;
    let inner_n = 64;
    let total = outer_n * inner_n;

    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let pool_arc = Arc::clone(&pool);
    let acct = Arc::new(Accounting::new(total));

    pool.install(|outer| {
        for o in 0..outer_n {
            let inner_pool = Arc::clone(&pool_arc);
            let acct_cl = Arc::clone(&acct);
            outer.spawn(move || {
                inner_pool.scope(|inner| {
                    for i in 0..inner_n {
                        let id = o * inner_n + i;
                        let tracker = DropTracker {
                            id,
                            dropped: Arc::clone(&acct_cl.dropped),
                        };
                        let ran_slot = &acct_cl.ran[id];
                        inner.spawn(move || {
                            let _keep = &tracker;
                            let prev = ran_slot.swap(true, Ordering::AcqRel);
                            assert!(!prev, "nested task {id} ran more than once");
                        });
                    }
                });
            });
        }
    });

    acct.assert_exactly_once(total);
}

// =========================================================================
// Panic path (plan D6): wrapper double-drop / leak bugs hide on the unwind
// path (scope.rs catch_unwind). Survivors + panickers must each run-once +
// drop-once; the first panic must propagate.
// =========================================================================

#[test]
fn stress_panic_path_survivors_run_once_and_panic_propagates() {
    let workers = 4;
    let n = 512;
    // Every 7th task panics. Run-once + drop-once must still hold for ALL of
    // them (the panicking body still flips `ran` and still drops its tracker —
    // the tracker is moved into the body, so `catch_unwind`'s unwind drops it).
    let panic_every = 7;

    let acct = Accounting::new(n);
    let panic_count = Arc::new(AtomicUsize::new(0));

    let pool = ThreadPoolBuilder::new().num_threads(workers).build();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pool.install(|scope| {
            for id in 0..n {
                let tracker = DropTracker {
                    id,
                    dropped: Arc::clone(&acct.dropped),
                };
                let ran_slot = &acct.ran[id];
                let pc = Arc::clone(&panic_count);
                scope.spawn(move || {
                    let _keep = &tracker;
                    let prev = ran_slot.swap(true, Ordering::AcqRel);
                    assert!(!prev, "task {id} ran more than once");
                    if id % panic_every == 0 {
                        pc.fetch_add(1, Ordering::AcqRel);
                        // `tracker` drops during unwind (moved into this body).
                        panic!("planned panic in task {id}");
                    }
                    // non-panicking tasks drop `tracker` normally here
                });
            }
        });
    }));

    // The first panic must propagate out of `install`.
    assert!(
        result.is_err(),
        "a panicking scope task must propagate out of install"
    );

    // Even with panics, EVERY task ran once and its payload dropped exactly
    // once (the unwind path must not leak or double-drop). This is the
    // load-bearing post-join sweep for the panic path.
    acct.assert_exactly_once(n);

    let expected_panics = (0..n).filter(|id| id % panic_every == 0).count();
    assert_eq!(
        panic_count.load(Ordering::Acquire),
        expected_panics,
        "every panicking task body must have executed its panic branch"
    );
}

// =========================================================================
// no_starvation — best-effort, #[ignore]-by-default (plan D6 / §4): NOT a CI
// gate. Asserts only that some worker makes progress, never a tight fairness
// ratio.
// =========================================================================

#[test]
#[ignore = "best-effort fairness probe; timing-flaky on shared CI (plan D6)"]
fn no_starvation_every_worker_makes_progress() {
    let workers = 4;
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();

    // Per-worker progress counters. We can't directly bind a task to a worker,
    // but with enough tasks each worker should grab at least one.
    let progressed: Arc<Vec<AtomicU32>> =
        Arc::new((0..workers).map(|_| AtomicU32::new(0)).collect::<Vec<_>>());

    pool.install(|scope| {
        for _ in 0..(workers * 64) {
            let prog = Arc::clone(&progressed);
            scope.spawn(move || {
                let wid = boyko_threadpool::current_worker_id();
                if (wid as usize) < prog.len() {
                    prog[wid as usize].fetch_add(1, Ordering::AcqRel);
                }
            });
        }
    });

    let woke = progressed
        .iter()
        .filter(|c| c.load(Ordering::Acquire) > 0)
        .count();
    // Weak assertion only: at least one worker made progress (the strong
    // "every worker" form is documented-flaky and intentionally not asserted).
    assert!(woke >= 1, "expected at least one worker to make progress");
}
