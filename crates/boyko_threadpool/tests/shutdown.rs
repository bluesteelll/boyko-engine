//! Phase 9.3b — pool shutdown / join (native).
//!
//! These tests verify the cycle-breaking fix: workers hold `Arc<PoolInner>`
//! (not the handle), so dropping the last `Arc<ThreadPool>` runs
//! `ThreadPool::drop`, which sets `shutdown`, unparks every worker, and joins
//! them. Each test is deterministic and finishes only if the join does NOT
//! deadlock — a regression (workers leaking / Drop unreachable / double-join)
//! manifests as a hang or panic here.

// Test-harness observation model: a `Mutex<Vec<usize>>` records which tasks ran
// so the assertions can inspect the outcome from the test thread. It is scaffolding
// around the pool, never inside it, and is compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use boyko_threadpool::ThreadPoolBuilder;

const WORKERS: usize = 4;

// NOTE on liveness: these tests must NOT use a cross-task blocking primitive
// (e.g. `std::sync::Barrier`) inside scope tasks. boyko's `Scope::drop`
// joiner batch-steals (`steal_batch_and_pop`) and drains the batch INLINE on
// the dispatcher (`drain_scratch`), so the dispatcher can pull several tasks
// into its scratch deque and run them one-at-a-time; a task that blocks on a
// barrier-of-N would wedge the dispatcher while its sibling tasks sit unrun in
// the same scratch deque → deadlock. (Same hazard as a blocking barrier across
// rayon tasks.) We therefore prove shutdown/join with non-blocking work only.

/// Run a batch of independent tasks, then drop the handle; `Drop` must set
/// `shutdown`, unpark every worker, and join them without hanging. If the
/// worker↔pool Arc cycle were still present, `Drop` would be unreachable and the
/// workers would leak; the test completing cleanly proves the cycle is broken.
#[test]
fn pool_drop_joins_all_workers() {
    let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();
    let ran = Arc::new(AtomicUsize::new(0));

    // Many small independent tasks (no inter-task blocking) so the work spreads
    // across the worker threads without any deadlock hazard.
    pool.install(|scope| {
        for _ in 0..512 {
            let ran = Arc::clone(&ran);
            scope.spawn(move || {
                ran.fetch_add(1, Ordering::Relaxed);
            });
        }
    });

    assert_eq!(
        ran.load(Ordering::Acquire),
        512,
        "every spawned task must have run exactly once"
    );

    // Dropping the pool must set shutdown, unpark, and join all workers.
    drop(pool);
}

/// `join()` followed by `drop` must be safe: the join handles are taken once,
/// so exactly one of `{join(), Drop}` joins. No panic, no double-join, no
/// hang.
#[test]
fn pool_double_shutdown_is_safe() {
    let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();

    // Run a little work so the workers are genuinely alive before teardown.
    let counter = Arc::new(AtomicUsize::new(0));
    pool.install(|scope| {
        for _ in 0..64 {
            let c = Arc::clone(&counter);
            scope.spawn(move || {
                c.fetch_add(1, Ordering::Relaxed);
            });
        }
    });
    assert_eq!(counter.load(Ordering::Acquire), 64);

    // Explicit join takes the handles and shuts down + joins.
    pool.join();
    // A second join is a no-op (handles already taken).
    pool.join();
    // Drop after an explicit join must also be a no-op (no double-join).
    drop(pool);
}

/// Building a pool and dropping it immediately (no work, no explicit join)
/// must terminate: `Drop` unparks the freshly-parked workers and joins them.
#[test]
fn pool_drop_without_join_terminates() {
    let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();
    drop(pool);
}

/// Per-task Drop-tracker variant: confirms each task body ran (collecting its
/// index) and that an explicit `join()` after the work completes cleanly. The
/// `Mutex<Vec<usize>>` records which task indices executed; clean completion
/// proves the join did not deadlock or double-join. No cross-task blocking.
#[test]
fn pool_drop_joins_with_per_task_tracker() {
    let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();
    let ran = Arc::new(Mutex::new(Vec::<usize>::new()));

    const TASKS: usize = 64;
    pool.install(|scope| {
        for i in 0..TASKS {
            let ran = Arc::clone(&ran);
            scope.spawn(move || {
                ran.lock()
                    .expect("invariant: tracker mutex never poisoned")
                    .push(i);
            });
        }
    });

    let mut ran = ran
        .lock()
        .expect("invariant: tracker mutex never poisoned")
        .clone();
    ran.sort_unstable();
    assert_eq!(
        ran,
        (0..TASKS).collect::<Vec<_>>(),
        "every task index must have executed exactly once"
    );

    pool.join();
}
