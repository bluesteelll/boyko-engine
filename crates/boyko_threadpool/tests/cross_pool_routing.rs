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
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::{ThreadPool, ThreadPoolBuilder, WORKER_ID_DISPATCHER, current_worker_id};

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
    assert_eq!(
        after_first, FIRST,
        "first detached batch: {after_first}/{FIRST}"
    );

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

// =========================================================================
// KE16 App-6 — the ONE identity predicate (`tls::worker_lane_for`).
//
// Before it, three places decided "am I a worker of this pool" three different
// ways, and the joiner's version asked `wid < len` with no pool identity at
// all: a pool-A worker joining a pool-B scope drained B's
// `injector_local[wid_A]` — a slot owned by B's worker `wid_A`, the only
// thread that may drain it (nothing else polls it; that is KE16 defect A).
// These three tests pin the three answers the predicate gives: a cross-pool
// joiner is EXTERNAL to the target pool, an `install` frame on a foreign
// worker is external, and an `install` frame on a worker of its OWN pool is
// external too (`install` rewrites the TLS id to the dispatcher sentinel).
//
// The receipt is the PAIR `(pool address, worker id)` recorded inside every
// body: worker ids of two pools both range over `[0, W)`, so an id alone
// cannot say which pool ran a body.
//
// A further row rides in this section for that receipt machinery rather than
// for the predicate: `cross_pool_fire_and_forget_from_a_worker_is_drained_by_b_workers`
// is the DETERMINISTIC gate on cross-pool REACHABILITY — "the push landed in a
// queue B's own workers poll", which the JOINED rows can observe but must not
// assert, because an external joiner is entitled to drain the whole wave.
// =========================================================================

/// Where task bodies ran: how many ran at all, how many reported a
/// `(pool address, worker id)` pair outside the permitted set, and the first
/// such pair.
struct Receipts {
    runs: AtomicUsize,
    violations: AtomicUsize,
    bad_pool: AtomicUsize,
    bad_wid: AtomicU32,
}

impl Receipts {
    fn new() -> Self {
        Self {
            runs: AtomicUsize::new(0),
            violations: AtomicUsize::new(0),
            bad_pool: AtomicUsize::new(0),
            bad_wid: AtomicU32::new(u32::MAX),
        }
    }

    /// Record this thread's `(pool address, worker id)`; `permitted` decides.
    /// Returns the recorded pair so a caller can count a second property.
    fn record<F: Fn(usize, u32) -> bool>(&self, permitted: F) -> (usize, u32) {
        let pool = ThreadPool::current_pool().map_or(0, |p| p.as_ptr() as usize);
        let wid = current_worker_id();
        if !permitted(pool, wid) {
            self.bad_pool.store(pool, Ordering::Release);
            self.bad_wid.store(wid, Ordering::Release);
            self.violations.fetch_add(1, Ordering::Release);
        }
        self.runs.fetch_add(1, Ordering::Release);
        (pool, wid)
    }

    fn assert_clean(&self, what: &str) {
        let violations = self.violations.load(Ordering::Acquire);
        assert_eq!(
            violations,
            0,
            "{what}: {violations} bodies ran on a thread outside the permitted set; \
             first offender = (pool {:#x}, worker id {})",
            self.bad_pool.load(Ordering::Acquire),
            self.bad_wid.load(Ordering::Acquire)
        );
    }
}

/// The `PoolInner` address of `pool` — the identity every receipt is compared
/// against. `install` deposits it in the calling thread's TLS.
fn pool_address(pool: &ThreadPool) -> usize {
    pool.install(|_scope| {
        ThreadPool::current_pool()
            .expect("invariant: install deposits the active pool")
            .as_ptr() as usize
    })
}

/// App-6 reachability, the DETERMINISTIC arm: a fire-and-forget wave pushed
/// into pool B from a pool-A worker must be run by pool B's own workers.
///
/// This row — not its `scope`-shaped sibling below — is what pins the routing
/// invariant "a cross-pool push lands in a queue B's workers can poll". What
/// the detached path removes is the observer's ambiguity, not any part of the
/// mechanism: `ThreadPool::spawn` leaves pool B with no joiner, and no thread
/// of pool A ever polls a queue of pool B, so a body that ran AT ALL ran on a
/// pool-B worker. A push that landed in a slot only some other B worker polls
/// is drained by nobody, the bounded wait expires, and the count assertion is
/// red — instead of the scheduling coin-flip an "a B worker ran one of them"
/// receipt is on the joined path.
///
/// The routing DECISION under test is the same one the scope path takes: both
/// reach `worker::push_task`, which compares the TLS active-pool pointer with
/// the target pool and falls back to B's global injector on a mismatch.
///
/// Anti-vacuity is asserted rather than hoped for. The pushing frame records
/// that it really ran as a REGISTERED WORKER OF POOL A — the input the identity
/// check must reject — which a detached `pool_a.spawn` guarantees by never
/// running on the calling thread; a dispatcher-issued push is the separate row
/// `cross_pool_spawn_from_dispatcher_completes_promptly`.
#[test]
fn cross_pool_fire_and_forget_from_a_worker_is_drained_by_b_workers() {
    let pool_a = ThreadPoolBuilder::new().num_threads(4).build();
    let pool_b = ThreadPoolBuilder::new().num_threads(4).build();
    let a_addr = pool_address(&pool_a);
    let b_addr = pool_address(&pool_b);
    let w_a = pool_a.worker_count();
    let w_b = pool_b.worker_count();

    const N: usize = 64;
    let receipts = Arc::new(Receipts::new());
    let pushed_from_a_worker = Arc::new(AtomicUsize::new(0));

    {
        let pool_b = Arc::clone(&pool_b);
        let receipts = Arc::clone(&receipts);
        let pushed_from_a_worker = Arc::clone(&pushed_from_a_worker);
        pool_a.spawn(move || {
            let pusher = ThreadPool::current_pool().map_or(0, |p| p.as_ptr() as usize);
            if pusher == a_addr && current_worker_id() < w_a {
                pushed_from_a_worker.fetch_add(1, Ordering::Release);
            }
            for _ in 0..N {
                let receipts = Arc::clone(&receipts);
                pool_b.spawn(move || {
                    receipts.record(|pool, wid| pool == b_addr && wid < w_b);
                });
            }
        });
    }

    let ran = wait_until(&receipts.runs, N, Duration::from_secs(30));
    assert_eq!(
        ran, N,
        "only {ran}/{N} cross-pool detached bodies ran within the bound; pool B has no joiner \
         and no pool-A thread polls a pool-B queue, so a shortfall means the push landed in a \
         queue no pool-B worker polls"
    );
    assert_eq!(
        pushed_from_a_worker.load(Ordering::Acquire),
        1,
        "the wave was not pushed by a registered pool-A worker, so the identity check under \
         test was never handed the input it must reject"
    );
    receipts.assert_clean("cross-pool fire-and-forget");

    drop(pool_a);
    drop(pool_b);
}

/// App-6: a pool-A worker joining a pool-B scope is an EXTERNAL joiner of B —
/// it never touches B's per-worker structures with its own id, and B's own
/// workers are free to carry the wave.
///
/// `pool_b.scope` (not `install`) is deliberate: it leaves the A worker's TLS
/// alone, which is exactly the shape whose bare `wid < len` joiner test used to
/// pass and which then drained `B.injector_local[wid_A]`.
///
/// What this test can and cannot see. That the joiner no longer READS the
/// foreign slot is a white-box fact — a task drained from it and a task stolen
/// from a B worker's deque are indistinguishable from outside (a B worker moves
/// its local-injector batch into its own deque as it drains it, so those tasks
/// are legitimately stealable). The predicate's answer itself is asserted in
/// `tls.rs`'s unit tests. What is ASSERTED here is the ROUTING receipt: the
/// whole wave ran, and every body ran either on a registered B worker or inline
/// on the joining A thread — no third identity.
///
/// "A pool-B worker reached the wave" is RECORDED here and asserted NOWHERE,
/// because the invariant does not imply it: the joining A thread is an external
/// joiner of B, and `join_workers_until_drained` lets it `steal_batch_and_pop`
/// B's global injector and run the whole batch itself, so a perfectly routed
/// pool legitimately yields zero B-worker receipts. It WAS asserted until
/// 2026-09-03, behind a five-wave retry that is a mitigation and not a fix, and
/// MEASURED red in 6 of 32 whole-binary runs against 0 of 12 when run by name —
/// the other seven tests of this binary are the load that lets the joiner win
/// the race.
///
/// The reachability property that assertion was reaching for is owned by
/// `cross_pool_fire_and_forget_from_a_worker_is_drained_by_b_workers`, which
/// puts the same `push_task` decision on the DETACHED path: pool B has no
/// joiner there, so an unreachable push is a timeout rather than a coin-flip.
#[test]
fn cross_pool_join_does_not_drain_foreign_local_slot() {
    let pool_a = ThreadPoolBuilder::new().num_threads(4).build();
    let pool_b = ThreadPoolBuilder::new().num_threads(4).build();
    let a_addr = pool_address(&pool_a);
    let b_addr = pool_address(&pool_b);
    let w_a = pool_a.worker_count();
    let w_b = pool_b.worker_count();

    const N: usize = 64;
    let receipts = Arc::new(Receipts::new());
    let joined = Arc::new(AtomicUsize::new(0));
    let b_worker_runs = Arc::new(AtomicUsize::new(0));

    {
        // A fire-and-forget task never runs on the calling thread, so the
        // join below genuinely happens on a pool-A worker.
        let pool_b = Arc::clone(&pool_b);
        let receipts = Arc::clone(&receipts);
        let joined = Arc::clone(&joined);
        let b_worker_runs = Arc::clone(&b_worker_runs);
        pool_a.spawn(move || {
            pool_b.scope(|scope| {
                for _ in 0..N {
                    let receipts = Arc::clone(&receipts);
                    let b_worker_runs = Arc::clone(&b_worker_runs);
                    scope.spawn(move || {
                        let (pool, wid) = receipts.record(|pool, wid| {
                            (pool == b_addr && wid < w_b) || (pool == a_addr && wid < w_a)
                        });
                        if pool == b_addr && wid < w_b {
                            b_worker_runs.fetch_add(1, Ordering::Release);
                        }
                        std::thread::yield_now();
                    });
                }
            });
            joined.fetch_add(1, Ordering::Release);
        });
    }

    let done = wait_until(&joined, 1, Duration::from_secs(30));
    assert_eq!(done, 1, "the cross-pool join never returned");
    assert_eq!(
        receipts.runs.load(Ordering::Acquire),
        N,
        "not every body of the cross-pool wave ran"
    );
    receipts.assert_clean("cross-pool join");

    // Recorded, never asserted — see the doc comment for which test owns the
    // reachability property and why this split is not one. `--nocapture` prints
    // it; it is a scheduling observation kept for diagnosis, not a gate.
    println!(
        "cross-pool join: {}/{N} bodies ran on a pool-B worker (observation, not an invariant)",
        b_worker_runs.load(Ordering::Acquire)
    );

    drop(pool_a);
    drop(pool_b);
}

/// App-6: a pool-A worker inside `pool_b.install` is an EXTERNAL joiner of B.
/// Its spawns route to B's global injector and its bodies run either on a B
/// worker or inline on the A thread — which, inside B's install frame, reports
/// `(B, WORKER_ID_DISPATCHER)`. A receipt naming pool A would mean a body ran
/// under A's identity, i.e. the routing decision consulted the wrong pool.
#[test]
fn install_on_foreign_worker_routes_to_global() {
    let pool_a = ThreadPoolBuilder::new().num_threads(4).build();
    let pool_b = ThreadPoolBuilder::new().num_threads(4).build();
    let b_addr = pool_address(&pool_b);
    let w_b = pool_b.worker_count();

    const N: usize = 64;
    let receipts = Arc::new(Receipts::new());
    let joined = Arc::new(AtomicUsize::new(0));
    let outer_on_worker = Arc::new(AtomicUsize::new(0));

    {
        let pool_b = Arc::clone(&pool_b);
        let receipts = Arc::clone(&receipts);
        let joined = Arc::clone(&joined);
        let outer_on_worker = Arc::clone(&outer_on_worker);
        pool_a.spawn(move || {
            // Anti-vacuity: prove the install really ran on an A worker.
            if current_worker_id() < WORKER_ID_DISPATCHER {
                outer_on_worker.fetch_add(1, Ordering::Release);
            }
            pool_b.install(|scope| {
                for _ in 0..N {
                    let receipts = Arc::clone(&receipts);
                    scope.spawn(move || {
                        receipts.record(|pool, wid| {
                            pool == b_addr && (wid < w_b || wid == WORKER_ID_DISPATCHER)
                        });
                    });
                }
            });
            joined.fetch_add(1, Ordering::Release);
        });
    }

    let done = wait_until(&joined, 1, Duration::from_secs(30));
    assert_eq!(done, 1, "the foreign install never returned");
    assert_eq!(
        outer_on_worker.load(Ordering::Acquire),
        1,
        "the install did not run on a pool-A worker; the test would be vacuous"
    );
    assert_eq!(
        receipts.runs.load(Ordering::Acquire),
        N,
        "not every body ran"
    );
    receipts.assert_clean("foreign install");

    drop(pool_a);
    drop(pool_b);
}

/// App-6: `install` on a worker of its OWN pool is external to that pool for
/// the frame — `install` rewrites the TLS worker id to the dispatcher
/// sentinel, so the predicate answers `None` and no per-worker structure is
/// indexed with a sentinel that is not a worker id. The row exists because a
/// predicate that asked for the deque pointer alone would have indexed
/// `inner.workers[WORKER_ID_DISPATCHER]` here and killed the process; the test
/// asserts the pool is alive and still running work afterwards.
#[test]
fn install_on_same_pool_worker_is_external() {
    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let addr = pool_address(&pool);
    let w = pool.worker_count();

    const N: usize = 64;
    let receipts = Arc::new(Receipts::new());
    let joined = Arc::new(AtomicUsize::new(0));

    {
        let pool_inner = Arc::clone(&pool);
        let receipts = Arc::clone(&receipts);
        let joined = Arc::clone(&joined);
        pool.spawn(move || {
            pool_inner.install(|scope| {
                for _ in 0..N {
                    let receipts = Arc::clone(&receipts);
                    scope.spawn(move || {
                        receipts
                            .record(|p, wid| p == addr && (wid < w || wid == WORKER_ID_DISPATCHER));
                    });
                }
            });
            joined.fetch_add(1, Ordering::Release);
        });
    }

    let done = wait_until(&joined, 1, Duration::from_secs(30));
    assert_eq!(done, 1, "the same-pool install never returned");
    assert_eq!(
        receipts.runs.load(Ordering::Acquire),
        N,
        "not every body ran"
    );
    receipts.assert_clean("same-pool install");

    // The pool survived the nested install frame and still runs work.
    let after = Arc::new(AtomicUsize::new(0));
    let after_cl = Arc::clone(&after);
    pool.spawn(move || {
        after_cl.fetch_add(1, Ordering::Release);
    });
    assert_eq!(
        wait_until(&after, 1, Duration::from_secs(10)),
        1,
        "the pool stopped running work after the same-pool install"
    );

    drop(pool);
}

/// App-6 / KE16 axis B: an `install` frame on a worker of a **single-worker**
/// pool must still complete — the frame's joiner is the only thread that can run
/// what the frame spawned.
///
/// The predicate calls that joiner EXTERNAL (`install` rewrote its worker id to
/// the dispatcher sentinel), and its pushes go to `injector_global` accordingly.
/// With `num_threads(1)` there is no second worker to steal them, so an external
/// arm that refused to help would have no completer to wait for and the join
/// would never return. The shipped external arm does help — it steals from the
/// global injector one task at a time — and this row is the gate on that. The
/// four-worker `install_on_same_pool_worker_is_external` above cannot be that
/// gate: its three idle siblings carry the wave whatever the joiner does.
///
/// The pool is deliberately LEAKED on the timeout path: a wedged worker is
/// exactly what this row detects, and `ThreadPool::drop` joins its workers, so
/// dropping it would turn a red into a hung test binary.
#[test]
fn install_on_the_only_worker_of_its_pool_completes() {
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let addr = pool_address(&pool);
    assert_eq!(pool.worker_count(), 1, "the fixture needs a single-worker pool");

    const N: usize = 64;
    let receipts = Arc::new(Receipts::new());
    let joined = Arc::new(AtomicUsize::new(0));

    {
        let pool_inner = Arc::clone(&pool);
        let receipts = Arc::clone(&receipts);
        let joined = Arc::clone(&joined);
        pool.spawn(move || {
            pool_inner.install(|scope| {
                for _ in 0..N {
                    let receipts = Arc::clone(&receipts);
                    scope.spawn(move || {
                        // Worker 0 either as itself (a task run from
                        // `worker_main`) or as the frame's dispatcher (a task the
                        // joiner ran inline).
                        receipts
                            .record(|p, wid| p == addr && (wid == 0 || wid == WORKER_ID_DISPATCHER));
                    });
                }
            });
            joined.fetch_add(1, Ordering::Release);
        });
    }

    let done = wait_until(&joined, 1, Duration::from_secs(30));
    if done != 1 {
        let ran = receipts.runs.load(Ordering::Acquire);
        std::mem::forget(pool);
        panic!(
            "the install frame on the pool's only worker never returned ({ran}/{N} bodies ran): \
             its joiner is classified external and refused to help, so nothing in the pool can \
             drain the wave it pushed to the global injector"
        );
    }
    assert_eq!(
        receipts.runs.load(Ordering::Acquire),
        N,
        "not every body ran"
    );
    receipts.assert_clean("single-worker install");

    drop(pool);
}

/// App-6, the shape the equal-sized pools above cannot reach: a joiner whose
/// own worker id is OUT OF RANGE for the pool it is joining.
///
/// Every other test in this file builds both pools the same size, so a pool-A
/// worker joining pool B always has `wid_A < W_B` and only the predicate's POOL
/// conjunct is ever the one that answers. The pre-App-6 joiner asked
/// `wid < len` with no pool identity at all (`join_workers_until_drained`), and
/// under equal sizes that mistake is a wrong-slot drain — B's worker `wid_A`
/// loses tasks it owned. Under `W_A > W_B` the same expression indexes B's
/// per-worker arrays PAST THEIR END, which is the round-2 out-of-bounds the
/// design names, and no test in the tree reaches it.
///
/// `pool_b.scope` (not `install`) again: it leaves the A worker's TLS alone, so
/// `current_worker_id()` inside the frame is the A worker's real id — the input
/// the bound conjunct must reject — rather than the dispatcher sentinel that
/// `install` would substitute.
///
/// The receipt is the joining thread's own id: an attempt whose join ran on
/// A's worker 0 exercises nothing (0 is in range for every pool), so the wave
/// is repeated until a join is observed from an id at or above `W_B`, and the
/// assertion names that count. The wave holds A's workers busy at the same
/// time, which is what spreads the joins across ids rather than replaying one.
#[test]
fn cross_pool_join_from_a_worker_out_of_the_target_pools_id_range_is_external() {
    // 4 against 1: every A worker except 0 has an id B does not have.
    let pool_a = ThreadPoolBuilder::new().num_threads(4).build();
    let pool_b = ThreadPoolBuilder::new().num_threads(1).build();
    let a_addr = pool_address(&pool_a);
    let b_addr = pool_address(&pool_b);
    let w_a = pool_a.worker_count();
    let w_b = pool_b.worker_count();
    assert!(
        w_a > w_b,
        "the shape under test needs A wider than B; got W_A={w_a}, W_B={w_b}"
    );

    /// Tasks per cross-pool wave. Small: B has ONE worker to drain them all.
    const N: usize = 8;
    const ATTEMPTS: usize = 8;

    let mut out_of_range_joins = 0usize;
    for _ in 0..ATTEMPTS {
        let joiners = w_a as usize;
        let receipts = Arc::new(Receipts::new());
        let started = Arc::new(AtomicUsize::new(0));
        let joined = Arc::new(AtomicUsize::new(0));
        let high_id_joins = Arc::new(AtomicUsize::new(0));

        for _ in 0..joiners {
            let pool_b = Arc::clone(&pool_b);
            let receipts = Arc::clone(&receipts);
            let started = Arc::clone(&started);
            let joined = Arc::clone(&joined);
            let high_id_joins = Arc::clone(&high_id_joins);
            pool_a.spawn(move || {
                // Occupancy, not synchronisation: the wait is bounded and its
                // expiry is not a failure. Holding the other A workers inside
                // their own task is what makes the joins land on distinct ids;
                // if the pool hands two tasks to one worker the wave simply
                // proceeds and the attempt contributes fewer distinct ids.
                started.fetch_add(1, Ordering::Release);
                let deadline = std::time::Instant::now() + Duration::from_millis(200);
                while started.load(Ordering::Acquire) < joiners
                    && std::time::Instant::now() < deadline
                {
                    std::thread::yield_now();
                }

                let wid = current_worker_id();
                if wid >= w_b && wid < w_a {
                    high_id_joins.fetch_add(1, Ordering::Release);
                }

                pool_b.scope(|scope| {
                    for _ in 0..N {
                        let receipts = Arc::clone(&receipts);
                        scope.spawn(move || {
                            receipts.record(|pool, wid| {
                                (pool == b_addr && wid < w_b) || (pool == a_addr && wid < w_a)
                            });
                        });
                    }
                });
                joined.fetch_add(1, Ordering::Release);
            });
        }

        let done = wait_until(&joined, joiners, Duration::from_secs(30));
        assert_eq!(
            done, joiners,
            "only {done}/{joiners} cross-pool joins from A returned; a joiner classified as one \
             of B's own workers would wait on a lane it must not own"
        );
        assert_eq!(
            receipts.runs.load(Ordering::Acquire),
            joiners * N,
            "not every body of the cross-pool waves ran"
        );
        receipts.assert_clean("out-of-range cross-pool join");

        out_of_range_joins += high_id_joins.load(Ordering::Acquire);
        if out_of_range_joins > 0 {
            break;
        }
    }

    assert!(
        out_of_range_joins > 0,
        "across {ATTEMPTS} waves no join ever ran on an A worker with an id at or above W_B={w_b}, \
         so the out-of-range shape was never exercised and this test proved nothing"
    );

    drop(pool_a);
    drop(pool_b);
}

/// KE16 W-d′ native obligation (`KE16-DESIGN-W.md` §3.7, index §8): a worker of
/// pool A joins a scope of pool B and finishes it inline.
///
/// This is the EXTERNAL arm of the count gate, reached by the one caller that
/// looks least external: a thread that is a registered worker — of the wrong
/// pool. `tls::worker_lane_for(B)` answers `None` for it (the deposited deque's
/// pool tag is A's), so `ScopeShared.joiner_wake` is null, `complete_task` takes
/// today's unpark-before-decrement order, and the wake target is the `waker`
/// inside the scope's own allocation.
///
/// What would break if the predicate were `wid < worker_count` instead of the
/// pool-tagged identity test: the target would be `B.workers[wid_A].thread` —
/// some other thread entirely — and the joiner would sleep to the backstop while
/// a stranger collected its wake. The assertion here is only that the scope
/// completes and the process survives; the aliasing claim is what the predicate
/// makes unrepresentable.
#[test]
fn cross_pool_worker_joining_a_foreign_scope_takes_the_external_arm() {
    let pool_a = ThreadPoolBuilder::new().num_threads(2).build();
    let pool_b = ThreadPoolBuilder::new().num_threads(2).build();

    const N: usize = 64;
    let ran = Arc::new(AtomicUsize::new(0));
    let joined = Arc::new(AtomicUsize::new(0));

    {
        let pool_b = Arc::clone(&pool_b);
        let ran = Arc::clone(&ran);
        let joined = Arc::clone(&joined);
        pool_a.spawn(move || {
            pool_b.install(|scope| {
                for _ in 0..N {
                    let ran = Arc::clone(&ran);
                    scope.spawn(move || {
                        ran.fetch_add(1, Ordering::Release);
                    });
                }
            });
            joined.fetch_add(1, Ordering::Release);
        });
    }

    let done = wait_until(&joined, 1, Duration::from_secs(30));
    assert_eq!(
        done, 1,
        "a pool-A worker joining a pool-B scope never returned — the count gate must not \
         apply to it, and its wake must reach the joining thread itself"
    );
    assert_eq!(
        ran.load(Ordering::Acquire),
        N,
        "not every body of the foreign scope ran"
    );
}
