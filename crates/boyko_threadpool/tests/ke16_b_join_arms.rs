//! KE16 axis B — the JOIN ARM, observed from outside the crate.
//!
//! `docs/threadpool/KE16-DESIGN-B.md` distinguishes the B candidates by *what a joining thread does
//! while it waits*. The axis closed on B1, whose behaviour is now unconditional, and it reduces to
//! three statements about that arm that nothing else in the tree measures:
//!
//! | Design | Statement | Test here |
//! |---|---|---|
//! | §2.3 | the EXTERNAL joiner helps — one task at a time, no scratch | `the_external_joiner_helps` |
//! | §6 | a worker of ANOTHER pool joining this pool's scope is EXTERNAL, so it takes that same arm | `a_worker_of_another_pool_obeys_the_external_arm` |
//! | §2.2 | the WORKER joiner re-checks its scope between tasks, so it never runs a whole residue first | `the_worker_joiner_does_not_run_a_residue_before_re_checking_its_scope` |
//!
//! Two design points make these tests worth their runtime rather than duplicates of the occupancy
//! harness. First, the harness measures OCCUPANCY (how many lanes a wave reaches) and is blind to
//! *which* thread the joiner is. Second, §2.2's "a re-check per task" is the property that *fixes
//! defect B*; §1 item 4 prices B0's alternative at a serial batch of up to 33 tasks, and that is a
//! wall-clock difference of two orders of magnitude, not a matter of taste.
//!
//! ## Run
//!
//! ```text
//! cargo test -p boyko-threadpool --test ke16_b_join_arms -- --test-threads=1 --nocapture
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use boyko_threadpool::{
    MAX_WORKERS, ThreadPool, ThreadPoolBuilder, current_worker_id, try_with_active_pool,
};

/// Tasks in the external-arm fixture. Only the first one blocks; the rest are trivial, and their
/// only job is to still be pending while the pool's one worker is held.
const EXTERNAL_ARM_TASKS: usize = 48;

/// How long the fixture's blocking task holds the pool's only worker.
///
/// It is a BOUND, not a delay: the joiner takes a task within microseconds and releases it, so the
/// only run that pays this in wall clock is one where the property under test FAILS and the pool's
/// own worker has to finish the wave. Short enough that such a run costs ~0.3 s, long enough that a
/// helping joiner has to be descheduled for a third of a second to be missed.
const WORKER_HOLD: Duration = Duration::from_millis(300);

/// Spin rather than sleep: a sleeping fixture can be woken late and read as "the joiner never
/// helped", which is the one wrong answer this file must not produce.
fn spin_until(flag: &AtomicBool, bound: Duration) {
    let deadline = Instant::now() + bound;
    while !flag.load(Ordering::Acquire) && Instant::now() < deadline {
        std::hint::spin_loop();
    }
}

/// Spin until `counter` reaches `target`, returning whether it did within `bound`.
fn spin_until_count(counter: &AtomicUsize, target: usize, bound: Duration) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        if counter.load(Ordering::Acquire) >= target {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::hint::spin_loop();
    }
}

/// The shared fixture of the two external-arm tests: a one-worker pool whose only worker is held by
/// the first task, so for `WORKER_HOLD` the JOINING thread is the only thread that can make the
/// wave progress. Returns how many bodies ran on the joining thread, the holding task included.
///
/// Why a zero return is a real answer and not a race. If the joining thread ran nothing, then every
/// body ran on the pool's one worker; the worker therefore entered the holding task, and it stays
/// there for `WORKER_HOLD` because only a body running on the joining thread releases it. So a zero
/// reading means the joiner sat next to `EXTERNAL_ARM_TASKS - 1` tasks, visible in the global
/// injector and in a registered deque, for a third of a second, and took none.
///
/// Called from the caller's own thread — that thread is the joiner, whatever it is (the test
/// thread in one test, a worker of a different pool in the other), which is precisely the variable
/// §6 turns on.
fn bodies_run_by_the_joining_thread(pool: &ThreadPool) -> usize {
    let joiner: ThreadId = std::thread::current().id();
    let ran_on_joiner = Arc::new(AtomicUsize::new(0));
    let ran_off_worker_ids = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let helper_seen = Arc::new(AtomicBool::new(false));
    let started = Instant::now();

    pool.install(|scope| {
        {
            // Task 0 holds the pool's worker. It is spawned FIRST, so a FIFO injector hands it to
            // the worker before anything else: from then on the worker is out of the picture and
            // every remaining task is reachable only by a joiner that helps.
            //
            // It refuses to hold the JOINING thread, and that refusal is load-bearing. A helping
            // joiner steals from the same queues the worker pops from, so it can take this very
            // task — and then the thread the fixture wanted to keep free is the one blocked, the
            // worker races through the rest, and the reading is "the joiner helped with nothing"
            // when in fact it helped with this. MEASURED: without the refusal this row read 0 on
            // this route while the same build read 27 when the test ran alone.
            let helper_seen = Arc::clone(&helper_seen);
            let completed = Arc::clone(&completed);
            let ran_on_joiner = Arc::clone(&ran_on_joiner);
            let ran_off_worker_ids = Arc::clone(&ran_off_worker_ids);
            scope.spawn(move || {
                let on_joiner = std::thread::current().id() == joiner;
                if on_joiner {
                    ran_on_joiner.fetch_add(1, Ordering::AcqRel);
                    helper_seen.store(true, Ordering::Release);
                } else {
                    spin_until(&helper_seen, WORKER_HOLD);
                }
                if current_worker_id() as usize >= MAX_WORKERS {
                    ran_off_worker_ids.fetch_add(1, Ordering::AcqRel);
                }
                completed.fetch_add(1, Ordering::AcqRel);
            });
        }
        for _ in 1..EXTERNAL_ARM_TASKS {
            let ran_on_joiner = Arc::clone(&ran_on_joiner);
            let ran_off_worker_ids = Arc::clone(&ran_off_worker_ids);
            let helper_seen = Arc::clone(&helper_seen);
            let completed = Arc::clone(&completed);
            scope.spawn(move || {
                // Two independent instruments for "this body ran on the joining thread", because
                // one of them alone cannot tell a wrong answer from a wrong reading: thread
                // IDENTITY (which thread executed it) and pool IDENTITY (`current_worker_id`,
                // which is a worker id on a pool worker and the dispatcher / unattached sentinel
                // on any joiner that is not a worker of this pool).
                if std::thread::current().id() == joiner {
                    ran_on_joiner.fetch_add(1, Ordering::AcqRel);
                    // Released after the count, so the held worker never resumes on a receipt the
                    // reader has not been given yet.
                    helper_seen.store(true, Ordering::Release);
                }
                if current_worker_id() as usize >= MAX_WORKERS {
                    ran_off_worker_ids.fetch_add(1, Ordering::AcqRel);
                }
                completed.fetch_add(1, Ordering::AcqRel);
            });
        }
    });

    let elapsed = started.elapsed();
    let by_thread = ran_on_joiner.load(Ordering::Acquire);
    let by_id = ran_off_worker_ids.load(Ordering::Acquire);
    println!(
        "[ke16 b-ext raw] by_thread_identity={by_thread} by_worker_id_sentinel={by_id} \
         scope_wall={elapsed:?}"
    );
    assert_eq!(
        by_thread, by_id,
        "the two instruments disagree: {by_thread} bodies ran on the joining THREAD but {by_id} \
         reported a non-worker id. One of the two is measuring the wrong thing"
    );
    assert_eq!(
        completed.load(Ordering::Acquire),
        EXTERNAL_ARM_TASKS,
        "the scope returned with tasks unfinished: the join is not a join"
    );
    by_thread
}

/// **§2.3 — the external joiner HELPS.**
///
/// A one-worker pool is joined from an unattached thread with the pool's worker held busy. The
/// joining thread must run at least one body, because for `WORKER_HOLD` it is the only thread that
/// can.
///
/// The reading is deterministic rather than a matter of timing: the wave cannot finish without the
/// joiner, so "zero" can only mean the external arm did not help.
#[test]
fn the_external_joiner_helps() {
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let helped = bodies_run_by_the_joining_thread(&pool);

    println!(
        "[ke16 b-ext] route=unattached tasks={EXTERNAL_ARM_TASKS} \
         bodies_on_the_joining_thread={helped}"
    );

    assert!(
        helped >= 1,
        "the external joiner ran no body, but its arm helps (KE16-DESIGN-B.md §2.3): with the \
         pool's only worker held for {WORKER_HOLD:?} and {} tasks pending, the joining thread was \
         the only thread that could make progress",
        EXTERNAL_ARM_TASKS - 1
    );
}

/// **§6 — the cross-pool joiner is EXTERNAL, and takes that arm.**
///
/// The same fixture, with the joining thread being a worker of a DIFFERENT pool.
/// `tls::worker_lane_for` answers `None` for it — the pool tag deposited on that thread is pool A,
/// not the joined pool B — so the join dispatches to the external arm instead of acting as a lane
/// of B.
///
/// A registered worker of A is the nearest caller class to a lane of B that must still not be
/// treated as one.
#[test]
fn a_worker_of_another_pool_obeys_the_external_arm() {
    let pool_a = ThreadPoolBuilder::new().num_threads(1).build();
    let pool_b = ThreadPoolBuilder::new().num_threads(1).build();

    let helped = Arc::new(AtomicUsize::new(usize::MAX));
    let joiner_worker_id = Arc::new(AtomicUsize::new(usize::MAX));
    let outer_done = Arc::new(AtomicBool::new(false));

    {
        let pool_b = Arc::clone(&pool_b);
        let helped = Arc::clone(&helped);
        let joiner_worker_id = Arc::clone(&joiner_worker_id);
        let outer_done = Arc::clone(&outer_done);
        // `spawn`, not `install` — the U1 rule the occupancy harness rests on. A scope joined from
        // the test thread would put the external joiner on this body from the TEST thread:
        // MEASURED, the row then reported `joiner_worker_id=4294967294`, i.e. the fixture had
        // measured the test thread joining B rather than a worker of A joining B, which is the
        // other test.
        pool_a.spawn(move || {
            joiner_worker_id.store(current_worker_id() as usize, Ordering::Release);
            helped.store(bodies_run_by_the_joining_thread(&pool_b), Ordering::Release);
            // Released last: it publishes both stores above to the reader below.
            outer_done.store(true, Ordering::Release);
        });
    }

    assert!(
        {
            spin_until(&outer_done, Duration::from_secs(60));
            outer_done.load(Ordering::Acquire)
        },
        "the cross-pool join never completed within 60 s"
    );

    let joiner_id = joiner_worker_id.load(Ordering::Acquire);
    let helped = helped.load(Ordering::Acquire);
    println!(
        "[ke16 b-ext] route=worker_of_another_pool joiner_worker_id={joiner_id} \
         tasks={EXTERNAL_ARM_TASKS} bodies_on_the_joining_thread={helped}"
    );

    assert!(
        joiner_id < pool_a.worker_count() as usize,
        "the cross-pool join did not run on a registered worker of pool A (id={joiner_id}); the \
         fixture measured some other thread and §6's route was never taken"
    );
    assert!(
        helped >= 1,
        "a worker of pool A ran none of pool B's tasks, but it is external to B (§6) and the \
         external arm helps (§2.3): B's only worker was held and A's worker was the only thread \
         that could make B's wave progress"
    );
}

/// Foreign tasks pushed into the global injector before the measured join.
const FOREIGN_TASKS: usize = 96;

/// Body of one foreign task. Chosen so that B0's serial residue would be unmistakable: B0
/// batch-stole `min((len − 1)/2, 32) + 1 ≤ 33` tasks into its unregistered `scratch` and ran them
/// ALL before the next `is_drained` check (`KE16-DESIGN-B.md` §1 item 4), i.e. ≥ 33 × 4 ms
/// ≈ 132 ms, while a per-task re-check costs at most one body.
const FOREIGN_BODY: Duration = Duration::from_millis(4);

/// Wall-clock bound on the measured join. Three times a single foreign body and a third of B0's
/// residue floor: wide enough that a preempted thread (a Windows quantum is ~15 ms) does not fail
/// it, narrow enough that it cannot be met by running a residue.
const JOIN_BUDGET: Duration = Duration::from_millis(60);

fn spin_for(d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end {
        std::hint::spin_loop();
    }
}

/// **§2.2 — "a re-check per task": the WORKER joiner stops helping the instant its scope drains.**
///
/// This is the property that fixes defect B.
///
/// The fixture separates it from occupancy. A worker X opens a scope holding ONE trivial task while
/// the global injector holds a large FOREIGN wave of long bodies. What is measured is the wall
/// clock of X's own `scope` call:
///
/// - a joiner that re-checks between tasks pops its own task, sees the scope drained and returns —
///   at most one foreign body of exposure if a sibling stole its task first;
/// - B0's joiner batch-stole up to 33 foreign tasks into `scratch` and ran the whole batch with no
///   `is_drained` in between, so the same call could not return before ~33 bodies.
///
/// MEASURED while the tournament's arms were still buildable, W=2, median of 5 runs each: the B0
/// joiner with the A1 placement in place — the configuration §1 is written about — **192 ms**; B1
/// **4 µs**; B3 **5 µs**. The pre-tournament default, the A0 placement with the B0 joiner, read
/// ~11 µs and was NOT the B0 baseline: under A0 the joiner's stage 1 drained `injector_local`,
/// where its own task was, so it returned before it ever reached the global injector. B0's residue
/// is a property of B0 *after* the A1 edits, which is exactly why §1 derives it from "the code as
/// it will be after A1" rather than from the code as it then stood.
#[test]
fn the_worker_joiner_does_not_run_a_residue_before_re_checking_its_scope() {
    const WORKERS: usize = 2;
    let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();

    let foreign_ready = Arc::new(AtomicBool::new(false));
    let foreign_done = Arc::new(AtomicUsize::new(0));
    let outer_running = Arc::new(AtomicBool::new(false));
    let join_micros = Arc::new(AtomicUsize::new(usize::MAX));
    let outer_worker = Arc::new(AtomicUsize::new(usize::MAX));
    let outer_done = Arc::new(AtomicBool::new(false));

    {
        let foreign_ready = Arc::clone(&foreign_ready);
        let outer_running = Arc::clone(&outer_running);
        let join_micros = Arc::clone(&join_micros);
        let outer_worker = Arc::clone(&outer_worker);
        let outer_done = Arc::clone(&outer_done);
        // Spawned BEFORE the foreign wave, so it is not queued behind it: the joiner has to be
        // running when the injector is full, which is the state the residue rule is about.
        pool.spawn(move || {
            outer_worker.store(current_worker_id() as usize, Ordering::Release);
            outer_running.store(true, Ordering::Release);
            spin_until(&foreign_ready, Duration::from_secs(5));

            let dispatched = try_with_active_pool(|inner| {
                let started = Instant::now();
                inner.scope(|nested| {
                    nested.spawn(|| {
                        // Trivial on purpose: everything the measurement below sees beyond a few
                        // microseconds is work the joiner took from somebody else's wave.
                        std::hint::black_box(0u32);
                    });
                });
                let elapsed = started.elapsed();
                join_micros.store(elapsed.as_micros() as usize, Ordering::Release);
            });
            assert!(
                dispatched.is_some(),
                "the outer task saw no active pool; the measured scope was never opened"
            );
            outer_done.store(true, Ordering::Release);
        });
    }

    spin_until(&outer_running, Duration::from_secs(5));
    assert!(
        outer_running.load(Ordering::Acquire),
        "the outer task never started within 5 s; the pool never dispatched the joiner"
    );

    for _ in 0..FOREIGN_TASKS {
        let foreign_done = Arc::clone(&foreign_done);
        pool.spawn(move || {
            spin_for(FOREIGN_BODY);
            foreign_done.fetch_add(1, Ordering::AcqRel);
        });
    }
    foreign_ready.store(true, Ordering::Release);

    let deadline = Instant::now() + Duration::from_secs(60);
    while !outer_done.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "the measured join never returned within 60 s"
        );
        std::thread::yield_now();
    }

    let micros = join_micros.load(Ordering::Acquire);
    let worker = outer_worker.load(Ordering::Acquire);
    println!(
        "[ke16 b-recheck] joiner_worker_id={worker} foreign_tasks={FOREIGN_TASKS} \
         foreign_body={FOREIGN_BODY:?} measured_join={micros}us b0_residue_floor={}us",
        33 * FOREIGN_BODY.as_micros()
    );

    assert!(
        worker < pool.worker_count() as usize,
        "the measured scope was not opened on a registered worker (id={worker}): the fixture \
         measured the dispatcher route, where the residue rule is a different one"
    );
    assert!(
        micros < JOIN_BUDGET.as_micros() as usize,
        "the worker joiner's scope took {micros} us, over the {JOIN_BUDGET:?} budget. Its own wave \
         was one trivial task, so the time is somebody else's wave run without an `is_drained` in \
         between — the B0 residue behaviour (KE16-DESIGN-B.md §1 item 4), which §2.2's per-task \
         re-check removes"
    );

    assert!(
        spin_until_count(&foreign_done, FOREIGN_TASKS, Duration::from_secs(60)),
        "the foreign wave did not finish within 60 s ({} of {FOREIGN_TASKS} done)",
        foreign_done.load(Ordering::Acquire)
    );
}
