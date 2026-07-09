//! FIX-2 / FIX-1 regression tests (native).
//!
//! ## FIX 2 — cross-pool push routing
//!
//! `push_task` used to route a fire-and-forget task by the bare TLS worker id
//! with no pool-identity check: a worker of pool A (id `wid`) that pushed into
//! pool B landed the task in pool B's `injector_local[wid]`, a slot only pool
//! B's worker `wid` ever polls — while `unpark_one_idle` wakes the lowest idle
//! bit. A cross-pool `spawn` could therefore sit undrained. The fix compares
//! the TLS active-pool pointer against the target pool and falls back to the
//! global injector on a mismatch. These tests drive a cross-pool `spawn` from
//! inside a worker of another pool and assert it completes promptly.
//!
//! ## FIX 1 — fire-and-forget panic policy
//!
//! A panicking `ThreadPool::spawn` task now aborts the process (rayon's
//! `spawn` policy). That is deliberately not unit-testable here without a
//! child-process harness; instead we assert the NON-panicking detached path is
//! unchanged (the task runs to completion, the worker survives, and further
//! detached work still runs — proving the added `catch_unwind` frame did not
//! alter normal-path behaviour).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::ThreadPoolBuilder;

/// Spin-wait until `counter` reaches `target` or `deadline` elapses. Returns
/// the final observed value. Bounded so a routing regression fails fast
/// (assertion) instead of hanging the suite.
fn wait_until(counter: &AtomicUsize, target: usize, timeout: Duration) -> usize {
    let deadline = Instant::now() + timeout;
    loop {
        let v = counter.load(Ordering::Acquire);
        if v >= target || Instant::now() >= deadline {
            return v;
        }
        std::thread::yield_now();
    }
}

/// FIX 2: a fire-and-forget `spawn` targeting pool B, issued from inside a
/// worker task of pool A, must be drained promptly by a pool-B worker.
///
/// Pre-fix, the task would land in B's `injector_local[wid_of_A]` (only B's
/// worker `wid_of_A` polls it, and only when it is the one woken), so a
/// mismatched id or an unlucky wake could leave it undrained past the timeout.
#[test]
fn cross_pool_spawn_from_worker_completes_promptly() {
    let pool_a = ThreadPoolBuilder::new().num_threads(4).build();
    let pool_b = ThreadPoolBuilder::new().num_threads(4).build();

    const N: usize = 256;
    let done = Arc::new(AtomicUsize::new(0));

    // Run a scope on A. Its spawned tasks execute on A's WORKER threads (not the
    // dispatcher), so inside them the TLS active pool is A while they push into
    // B — exactly the cross-pool misrouting scenario.
    pool_a.install(|scope| {
        for _ in 0..N {
            let pb = Arc::clone(&pool_b);
            let done = Arc::clone(&done);
            scope.spawn(move || {
                pb.spawn(move || {
                    done.fetch_add(1, Ordering::Release);
                });
            });
        }
    });

    // All N cross-pool detached tasks must run within a generous bound. A
    // routing regression manifests as some tasks stuck in a per-worker
    // injector nobody drains → count short of N at the deadline.
    let got = wait_until(&done, N, Duration::from_secs(10));
    assert_eq!(
        got, N,
        "all cross-pool fire-and-forget tasks must be drained by pool B \
         (got {got}/{N}); a shortfall means push_task misrouted to a \
         per-worker injector"
    );

    drop(pool_a);
    drop(pool_b);
}

/// FIX 2 (dispatcher variant): a cross-pool `spawn` issued from pool A's
/// `install` dispatcher thread must also complete promptly. The dispatcher's
/// TLS worker id is the `WORKER_ID_DISPATCHER` sentinel; the identity check
/// still routes the push to B's global injector.
#[test]
fn cross_pool_spawn_from_dispatcher_completes_promptly() {
    let pool_a = ThreadPoolBuilder::new().num_threads(2).build();
    let pool_b = ThreadPoolBuilder::new().num_threads(2).build();

    const N: usize = 128;
    let done = Arc::new(AtomicUsize::new(0));

    pool_a.install(|_scope| {
        for _ in 0..N {
            let done = Arc::clone(&done);
            pool_b.spawn(move || {
                done.fetch_add(1, Ordering::Release);
            });
        }
    });

    let got = wait_until(&done, N, Duration::from_secs(10));
    assert_eq!(got, N, "cross-pool dispatcher spawn shortfall: {got}/{N}");

    drop(pool_a);
    drop(pool_b);
}

/// FIX 1 (non-panicking path unchanged): fire-and-forget `spawn` tasks that do
/// NOT panic must run to completion, and the worker must survive to run more
/// detached work afterwards. This proves the added `catch_unwind` abort guard
/// in `run_task` did not perturb the normal detached path.
#[test]
fn detached_spawn_non_panicking_path_unchanged() {
    let pool = ThreadPoolBuilder::new().num_threads(4).build();

    const FIRST: usize = 200;
    const SECOND: usize = 200;
    let done = Arc::new(AtomicUsize::new(0));

    for _ in 0..FIRST {
        let done = Arc::clone(&done);
        pool.spawn(move || {
            done.fetch_add(1, Ordering::Release);
        });
    }
    let after_first = wait_until(&done, FIRST, Duration::from_secs(10));
    assert_eq!(after_first, FIRST, "first detached batch: {after_first}/{FIRST}");

    // Second batch proves the workers are still alive (a raw unwind in the old
    // code would have killed a worker; the abort guard leaves the non-panicking
    // path identical, so all workers remain).
    for _ in 0..SECOND {
        let done = Arc::clone(&done);
        pool.spawn(move || {
            done.fetch_add(1, Ordering::Release);
        });
    }
    let total = FIRST + SECOND;
    let after_second = wait_until(&done, total, Duration::from_secs(10));
    assert_eq!(
        after_second, total,
        "second detached batch after the first: {after_second}/{total}"
    );

    drop(pool);
}
