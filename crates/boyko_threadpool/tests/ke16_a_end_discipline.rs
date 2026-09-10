//! KE16 axis A, `KE16-DESIGN-A.md` §1.4 — the OWNER END of the worker's deque.
//!
//! The end discipline is decided by ONE line in `ThreadPoolBuilder::build`:
//! every worker deque is constructed with `Worker::new_fifo()`, so the owner
//! pops the OLDEST entry — the one pushed first. §1.4 spends its length on what
//! that one line trades — the owner's `front.fetch_add` against a local fence,
//! the thief's one CAS per batch against one per element.
//!
//! Almost nothing in the tree observes that line. The placement receipt in
//! `worker::tests` (`a1_a_spawn_from_a_worker_body_lands_on_its_own_deque`)
//! asserts WHERE the task lands, not from which end it leaves, and the
//! occupancy gates are green either way, because reachability does not depend
//! on the end. So a build whose constructor was quietly changed — to
//! `Worker::new_lifo()`, or to a LIFO deque handed to the worker loop by some
//! other route — would reverse the order in which a worker drains its own
//! spawns, and almost every other test would stay green. One other row reads
//! the same end: `tests/ke16_b_join_properties.rs`'s
//! `the_worker_joiner_takes_its_own_wave_before_a_foreign_one`, which asks it
//! at the JOINER (`KE16-DESIGN-B.md` §2.2) and names this file for the
//! worker-loop reading. Here the pop is not entangled with the join's step
//! order.
//!
//! The gate is the EXECUTION ORDER of a wave spawned from inside a worker body
//! on a ONE-worker pool: with no sibling there is no thief, so the order the
//! worker's own loop pops is the end discipline itself. It never reads
//! `running 0 tests` — the test is unconditional.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::{MAX_WORKERS, ThreadPoolBuilder, current_worker_id};

/// Four tasks: enough to tell `[0,1,2,3]` from `[3,2,1,0]`.
const WAVE: usize = 4;

/// Spin until `counter` reaches `target` or the deadline passes; answers the
/// value observed last. A task body must never assert (a panic inside one goes
/// through `abort_on_task_panic` and takes the whole test binary with it), so
/// every check in this file happens on the harness thread against a receipt.
fn wait_for(counter: &AtomicUsize, target: usize, timeout: Duration) -> usize {
    let deadline = Instant::now() + timeout;
    loop {
        let seen = counter.load(Ordering::Acquire);
        if seen >= target || Instant::now() >= deadline {
            return seen;
        }
        std::thread::yield_now();
    }
}

#[test]
fn a_wave_spawned_from_a_worker_body_is_executed_oldest_first() {
    // ONE worker: no sibling can steal, so the observed order is the owner's
    // pop end and nothing else. The wave cannot start before the outer body
    // returns, which is what makes the whole sequence deterministic.
    let pool = ThreadPoolBuilder::new().num_threads(1).build();

    let order: Arc<Vec<AtomicU32>> =
        Arc::new((0..WAVE).map(|_| AtomicU32::new(u32::MAX)).collect());
    let cursor = Arc::new(AtomicUsize::new(0));
    // The anti-vacuity receipt: the id of the worker each body ran on, folded
    // by `fetch_max`. `u32::MAX` (`WORKER_ID_UNATTACHED`) survives if the wave
    // ran on the harness thread instead of a worker, which would make the order
    // claim say nothing about the pool.
    let widest_worker_id = Arc::new(AtomicU32::new(0));

    let pool_inner = Arc::clone(&pool);
    let order_outer = Arc::clone(&order);
    let cursor_outer = Arc::clone(&cursor);
    let wid_outer = Arc::clone(&widest_worker_id);
    // Fire-and-forget, so the body is guaranteed to run ON the worker: a
    // `scope` would run part of the wave on the calling thread and measure the
    // joiner instead of the deque.
    pool.spawn(move || {
        for i in 0..WAVE {
            let order_task = Arc::clone(&order_outer);
            let cursor_task = Arc::clone(&cursor_outer);
            let wid_task = Arc::clone(&wid_outer);
            // THE SUBJECT: a production spawn issued from inside a worker body,
            // which lands on that worker's own registered deque. The four are
            // pushed oldest-first, so `[0,1,2,3]` is the FIFO reading and
            // `[3,2,1,0]` the LIFO one.
            pool_inner.spawn(move || {
                wid_task.fetch_max(current_worker_id(), Ordering::Relaxed);
                let slot = cursor_task.fetch_add(1, Ordering::AcqRel);
                // `get` rather than an index: a body that indexed out of bounds
                // would abort the process instead of failing this test.
                if let Some(cell) = order_task.get(slot) {
                    cell.store(i as u32, Ordering::Release);
                }
            });
        }
    });

    let ran = wait_for(&cursor, WAVE, Duration::from_secs(10));
    assert_eq!(
        ran, WAVE,
        "the wave spawned from the worker body never finished ({ran} of {WAVE} bodies ran): the \
         end discipline cannot be read from an undrained queue"
    );

    let wid = widest_worker_id.load(Ordering::Relaxed);
    assert!(
        (wid as usize) < MAX_WORKERS,
        "anti-vacuity: the wave must have run on a worker of the pool, but the widest recorded \
         worker id is {wid} (u32::MAX = unattached, i.e. it ran on the harness thread)"
    );

    let observed: Vec<u32> = order.iter().map(|c| c.load(Ordering::Acquire)).collect();
    // The claim is deliberately about the FIRST task executed, not about the
    // whole permutation. On this one-worker pool there is no thief, so every
    // position is decided by the owner's pop end and asserting the full order
    // would be strictly stronger; that strengthening is a separate call and is
    // not taken here. The full order is carried into the failure message
    // rather than asserted.
    let first = observed[0];
    let expected_first: u32 = 0;
    let end = "FIFO (the OLDEST entry, pushed first)";
    assert_eq!(
        first, expected_first,
        "KE16-DESIGN-A.md §1.4: the owner end of a worker's deque is {end}, so a wave pushed \
         0,1,2,3 from inside a worker body must START at {expected_first}; it started at {first} \
         (full observed order {observed:?}). `ThreadPoolBuilder::build` constructs every worker \
         deque with `Worker::new_fifo()` — a build that reached `Worker::new_lifo()` instead, or \
         that handed the loop a deque built elsewhere, would drain a worker's own spawns \
         newest-first"
    );
}
