//! KE16 axis A, `KE16-DESIGN-A.md` §1.4 — the OWNER END of the worker's deque.
//!
//! `ke16-a1` and `ke16-a1-fifo` are the SAME placement arm; the whole
//! difference between the two candidates is one `#[cfg]`-selected constructor
//! in `ThreadPoolBuilder::build` (`Worker::new_lifo()` under `ke16-a1`,
//! `Worker::new_fifo()` everywhere else). §1.4 spends its length on what that
//! one line trades — the owner's `front.fetch_add` against a local fence, the
//! thief's one CAS per batch against one per element — and the tournament
//! decides the pair on those numbers.
//!
//! Nothing else in the tree observes that line. `KE16_A` reads `"a1"` from the
//! FEATURE, not from the deque; the placement receipt in `worker::tests`
//! (`a1_a_spawn_from_a_worker_body_lands_on_its_own_deque`) asserts WHERE the
//! task lands, not from which end it leaves; and both A1 arms are green on
//! every occupancy gate, because reachability does not depend on the end. So a
//! build whose constructor arm was lost — a `#[cfg]` edited to the wrong
//! feature, the two branches swapped, the LIFO branch deleted with a losing
//! candidate — would run the tournament's `a1` row over a FIFO deque, report
//! `a1+b0+w0+c0`, and be certified by `KE16_EXPECT`: the quiet mislabelling
//! `KE16-DESIGN.md` §4 built the witness apparatus to forbid, in the one place
//! the witness cannot see, and the two rows it would corrupt are the pair the
//! rule decides symmetrically.
//!
//! The gate is the EXECUTION ORDER of a wave spawned from inside a worker body
//! on a ONE-worker pool: with no sibling there is no thief, so the order the
//! worker's own loop pops is the end discipline itself. It is meaningful in
//! every configuration of the tournament, not only under the A1 arms — every
//! other arm parks the wave in an `Injector` (FIFO by contract) or on a FIFO
//! deque — so the row is a pin on `ke16-a1` being the ONLY build in the grid
//! with a LIFO owner end, and it never reads `running 0 tests`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::{MAX_WORKERS, ThreadPoolBuilder, current_worker_id, ke16_variant};

/// Four tasks: enough to tell `[0,1,2,3]` from `[3,2,1,0]` and to survive one
/// `steal_batch_and_pop` grab on the injector arms (a single batch takes all of
/// them, so no arm can reorder the wave by splitting it).
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
fn a_wave_spawned_from_a_worker_body_is_executed_newest_first_only_under_ke16_a1() {
    boyko_threadpool::ke16_check_expected_variant();
    println!("KE16 variant: {}", ke16_variant());

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
            // which is what the A arms place. The four are pushed oldest-first,
            // so `[0,1,2,3]` is the FIFO reading and `[3,2,1,0]` the LIFO one.
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
    // The claim is about the FIRST task executed, not about the whole
    // permutation. On the A1 arms the wave sits entirely on the owner's deque
    // and the whole order is the end discipline; on the injector arms it is
    // not — stage 1 grabs a BATCH, returns one task and parks the remainder on
    // the deque, so the tail interleaves with the deque pops in a way that is
    // crossbeam's batching policy rather than an end. MEASURED in the default
    // build: `[0, 2, 3, 1]`, oldest-first at the head and batch-shaped after
    // it. The head is the invariant every arm shares a definition of, and it is
    // the one the LIFO / FIFO constructor decides.
    let first = observed[0];
    let (expected_first, end) = if cfg!(feature = "ke16-a1") {
        (WAVE as u32 - 1, "LIFO (the NEWEST entry, pushed last)")
    } else {
        (0, "FIFO (the OLDEST entry, pushed first)")
    };
    assert_eq!(
        first,
        expected_first,
        "KE16-DESIGN-A.md §1.4: this build is `{}`, whose owner end must be {end}, so the wave \
         pushed 0,1,2,3 from inside a worker body must START at {expected_first}; it started at \
         {first} (full observed order {observed:?}). The `Worker::new_lifo()` / \
         `Worker::new_fifo()` arm in `ThreadPoolBuilder::build` does not match the feature this \
         binary reports — the a1 and a1f rows of the tournament, which the design decides \
         symmetrically against each other, would be measured over the wrong deque under a \
         witness that cannot see the difference",
        ke16_variant()
    );
}
