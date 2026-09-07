//! KE16 axis B — the three WORKER-ARM properties of `KE16-DESIGN-B.md` §2.2/§2.6/§2.7 that no
//! other target in the tree states.
//!
//! `tests/ke16_b_join_arms.rs` (the developer's) separates the two EXTERNAL arms and pins §2.2's
//! "a re-check per task" by wall clock. What it does not reach is the worker arm's *order* and the
//! *inline-ness* of what a claimed joiner runs, and those are the two statements the B1 candidate
//! is actually made of:
//!
//! | Design | Statement | Test here |
//! |---|---|---|
//! | §2.2 step 1 + "Own scope first under LIFO" | the worker joiner pops its OWN deque before it touches the global injector, and the end it pops is the A arm's | `the_worker_joiner_takes_its_own_wave_before_a_foreign_one` |
//! | §2.7 | a claimed joiner runs the foreign task **INLINE inside its live join**, not after the join returned | `the_claimed_joiner_runs_the_foreign_task_inside_its_live_join` |
//! | §2.6 case (a) | a joiner that is the pool's ONLY lane runs its own wave and the scope completes | `a_nested_scope_on_a_single_worker_pool_completes` |
//!
//! **Why §2.7 needs a second receipt.** `ke16_nested_scope_occupancy.rs::
//! parked_joiner_is_claimed_by_a_foreign_wave` asserts `foreign_lane == joiner_lane`. That equality
//! has a second producer: the fixture's sibling body gives up on a bounded 10 s timeout, after
//! which the scope drains anyway, the joiner returns from its join, finishes its outer task and
//! re-enters `worker_main` — where it can pop the same foreign task from the global injector. The
//! lane reading is then identical and the receipt reads green for a mechanism that is NOT "a parked
//! joiner was claimed". The test below closes that channel with a flag the joiner itself owns: the
//! foreign body records whether the joiner's `scope` call had returned yet. Both stores are on the
//! SAME thread when the receipt holds, so the reading is exact by program order, not by timing.
//!
//! **Why §2.2's order needs `num_threads(1)`.** At W ≥ 2 a sibling can take either wave and the
//! order is a race. At W = 1 the joining worker is the only lane in the pool, so nothing runs until
//! its scope closure returns and everything that then runs is its join's own choice, in its own
//! order. That makes the step order of `KE16-DESIGN-B.md` §2.2 (own deque → global injector →
//! sibling sweep) a deterministic observation rather than a statistical one.
//!
//! **Every test here runs in EVERY build**, selecting its assertion from
//! [`boyko_threadpool::KE16_B`] / [`boyko_threadpool::KE16_A`] at run time — a file that vanishes
//! under `#[cfg]` reports `running 0 tests`, a vacuous pass (`KE16-DESIGN-MEASUREMENT.md` §5 item
//! 1). The `b0` readings are recorded, never asserted: `b0` is a candidate under measurement, and
//! its number here is what shows the instrument can tell the arms apart at all.
//!
//! ## Run
//!
//! ```text
//! cargo test -p boyko-threadpool --test ke16_b_join_properties -- --test-threads=1 --nocapture
//! cargo test -p boyko-threadpool --test ke16_b_join_properties --features ke16-a1,ke16-b1 -- --test-threads=1 --nocapture
//! cargo test -p boyko-threadpool --test ke16_b_join_properties --features ke16-a1-fifo,ke16-b1 -- --test-threads=1 --nocapture
//! cargo test -p boyko-threadpool --test ke16_b_join_properties --features ke16-a1,ke16-b3 -- --test-threads=1 --nocapture
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::{
    KE16_A, KE16_B, ThreadPool, ThreadPoolBuilder, WORKER_ID_DISPATCHER, WORKER_ID_UNATTACHED,
    current_worker_id, ke16_check_expected_variant, try_with_active_pool,
};

/// Sentinel for "this slot was never written". `usize::MAX` cannot collide with a task tag.
const UNSET: usize = usize::MAX;

/// Tag offset that separates a FOREIGN task's index from an OWN task's index in one `usize`, so the
/// completion order can be recorded with plain atomics and no per-task allocation.
const FOREIGN_TAG_BASE: usize = 1_000;

/// Own tasks in the order fixture. Small: at `num_threads(1)` every one of them runs on the joiner,
/// so the fixture's cost is linear in this number and nothing is learned past a handful.
const OWN_TASKS: usize = 8;

/// Foreign tasks in the order fixture. Enough that the global injector is unmistakably non-empty
/// when the join starts, which is the precondition §2.2's step order is about.
const FOREIGN_TASKS: usize = 8;

/// Wall-clock bound for every wait in this file. Generous: a Windows scheduling quantum is ~15 ms
/// and a parked thread's backstop is an OS timer tick, so a bound in seconds cannot be tripped by
/// scheduling alone — only by a joiner that never resumes.
const WAIT_BOUND: Duration = Duration::from_secs(30);

/// Bound on the ONE precondition that a build is allowed to miss: "the scope's body is already
/// running on a sibling when the join starts".
///
/// Under `ke16-a1` / `ke16-a1-fifo` a sibling reaches a worker-spawned body within microseconds, so
/// this expires only in a build that still has defect A — where a worker's spawn goes to
/// `injector_local[wid]` that no sibling polls, and the body cannot run until the joiner runs it
/// itself. MEASURED at this checkout: the default (`a0`) build waits it out in full and the `a1`
/// builds never touch it. Short, because paying `WAIT_BOUND` there would make the default-build row
/// of this file a 30 s gate that is measuring the A axis, not the B one.
const BODY_START_BOUND: Duration = Duration::from_secs(2);

/// Spin (never sleep) until `flag` is set or `WAIT_BOUND` expires; returns whether it was set.
///
/// A sleeping waiter can be resumed late and read as "the event never happened", which is the one
/// wrong answer a receipt must not produce.
fn spin_until(flag: &AtomicBool) -> bool {
    let deadline = Instant::now() + WAIT_BOUND;
    loop {
        if flag.load(Ordering::Acquire) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::hint::spin_loop();
    }
}

/// Whether `id` is a registered worker lane of `pool` rather than one of the two TLS sentinels.
fn is_registered_lane(pool: &ThreadPool, id: usize) -> bool {
    id != WORKER_ID_DISPATCHER as usize
        && id != WORKER_ID_UNATTACHED as usize
        && id < pool.worker_count() as usize
}

// =============================================================================================
// §2.2 step 1 — the worker joiner takes its OWN wave first, from the A arm's end
// =============================================================================================

/// **`KE16-DESIGN-B.md` §2.2 — "1. Own deque, owner end" and "Own scope first under LIFO".**
///
/// The B1 joiner's four sources are ordered, and the order is the candidate: its own deque before
/// the global injector before a sibling sweep. B0 has no first step at all (under A1 its stage 1 is
/// compiled out and `injector_global` is the first thing it touches, `KE16-DESIGN-B.md` §1 items
/// 2–3), so "which wave does the joiner start with" is exactly what the B axis changes and nothing
/// else in the tree observes it.
///
/// The fixture is `num_threads(1)`. The single worker runs the outer task, and while it is inside
/// the scope closure NOTHING else in the pool can run — so the completion order recorded below is
/// the join's own pick order, with no sibling to perturb it:
///
/// - own tasks go to the joiner's own destination (its registered deque under `ke16-a1` /
///   `ke16-a1-fifo`);
/// - foreign tasks were pushed from the TEST thread, so they are in `injector_global`;
/// - both waves are fully queued before the join begins, which the `join_open` flag records.
///
/// Two readings come out of one run. `order[0] < FOREIGN_TAG_BASE` is the step order. Under the B
/// arms the *value* of `order[0]` is additionally the END discipline of the A arm: `ke16-a1` pops
/// the owner end of a LIFO deque, so the newest own task (`OWN_TASKS - 1`) comes first; the
/// `ke16-a1-fifo` row inverts that to `0`. That is the A1-vs-A1-fifo question asked at the JOINER,
/// where §2.2 says it is asked, rather than at the worker loop (`tests/ke16_a_end_discipline.rs`).
#[test]
fn the_worker_joiner_takes_its_own_wave_before_a_foreign_one() {
    println!("KE16 variant: {}", boyko_threadpool::ke16_variant());
    ke16_check_expected_variant();

    let pool = ThreadPoolBuilder::new().num_threads(1).build();

    let total = OWN_TASKS + FOREIGN_TASKS;
    let order: Arc<Vec<AtomicUsize>> =
        Arc::new((0..total).map(|_| AtomicUsize::new(UNSET)).collect());
    let seq = Arc::new(AtomicUsize::new(0));
    let outer_started = Arc::new(AtomicBool::new(false));
    let foreign_ready = Arc::new(AtomicBool::new(false));
    let outer_done = Arc::new(AtomicBool::new(false));
    let outer_lane = Arc::new(AtomicUsize::new(UNSET));
    let ran_before_join = Arc::new(AtomicUsize::new(0));
    let join_open = Arc::new(AtomicBool::new(false));

    {
        let order = Arc::clone(&order);
        let seq = Arc::clone(&seq);
        let outer_started = Arc::clone(&outer_started);
        let foreign_ready = Arc::clone(&foreign_ready);
        let outer_done = Arc::clone(&outer_done);
        let outer_lane = Arc::clone(&outer_lane);
        let ran_before_join = Arc::clone(&ran_before_join);
        let join_open = Arc::clone(&join_open);
        pool.spawn(move || {
            outer_lane.store(current_worker_id() as usize, Ordering::Release);
            outer_started.store(true, Ordering::Release);
            // The foreign wave must be in `injector_global` BEFORE the join starts; otherwise the
            // join could drain its own deque against an empty injector and the step order would be
            // untested rather than confirmed.
            assert!(
                spin_until(&foreign_ready),
                "the test thread never finished pushing the foreign wave"
            );

            let dispatched = try_with_active_pool(|inner| {
                inner.scope(|nested| {
                    for i in 0..OWN_TASKS {
                        let order = Arc::clone(&order);
                        let seq = Arc::clone(&seq);
                        let ran_before_join = Arc::clone(&ran_before_join);
                        let join_open = Arc::clone(&join_open);
                        nested.spawn(move || {
                            if !join_open.load(Ordering::Acquire) {
                                ran_before_join.fetch_add(1, Ordering::AcqRel);
                            }
                            let slot = seq.fetch_add(1, Ordering::AcqRel);
                            order[slot].store(i, Ordering::Release);
                        });
                    }
                    // Last statement of the closure: the join begins when it returns. At W = 1 no
                    // task can have run yet, and `ran_before_join` is the assertion that says so
                    // rather than the comment.
                    join_open.store(true, Ordering::Release);
                });
                join_open.store(false, Ordering::Release);
            });
            assert!(
                dispatched.is_some(),
                "the outer task saw no active pool; the measured scope was never opened"
            );
            outer_done.store(true, Ordering::Release);
        });
    }

    assert!(
        spin_until(&outer_started),
        "the outer task never started; the one-worker pool never dispatched the joiner"
    );

    for j in 0..FOREIGN_TASKS {
        let order = Arc::clone(&order);
        let seq = Arc::clone(&seq);
        // Pushed from the TEST thread, so this wave lands in `injector_global` under every A arm —
        // it is "somebody else's wave" from the joiner's point of view, which is what step 2 is.
        pool.spawn(move || {
            let slot = seq.fetch_add(1, Ordering::AcqRel);
            order[slot].store(FOREIGN_TAG_BASE + j, Ordering::Release);
        });
    }
    foreign_ready.store(true, Ordering::Release);

    assert!(
        spin_until(&outer_done),
        "the outer task never completed: the joiner did not drain its own wave at W = 1, which is \
         case (a) of the deadlock argument (`KE16-DESIGN-B.md` §2.6)"
    );

    let deadline = Instant::now() + WAIT_BOUND;
    while seq.load(Ordering::Acquire) < total && Instant::now() < deadline {
        std::hint::spin_loop();
    }

    let observed: Vec<usize> = order.iter().map(|s| s.load(Ordering::Acquire)).collect();
    let lane = outer_lane.load(Ordering::Acquire);
    println!(
        "[ke16 b-order] a_arm={KE16_A} b_arm={KE16_B} joiner_lane={lane} \
         own={OWN_TASKS} foreign={FOREIGN_TASKS} ran_before_join={} order={observed:?}",
        ran_before_join.load(Ordering::Acquire)
    );

    assert!(
        is_registered_lane(&pool, lane),
        "the outer task did not run on a registered worker (id={lane}); the fixture measured the \
         dispatcher route, where the joiner has no own deque and §2.2 does not apply"
    );
    assert_eq!(
        ran_before_join.load(Ordering::Acquire),
        0,
        "a scope task ran before the closure returned, so the pool has more than one lane in play \
         and the recorded order is a race, not the join's pick order"
    );
    assert_eq!(
        seq.load(Ordering::Acquire),
        total,
        "only {} of {total} tasks ran; the order below is incomplete",
        seq.load(Ordering::Acquire)
    );

    if KE16_B == "b0" {
        // Recorded, not asserted: b0 is the behaviour under measurement. Under `a0` its stage 1
        // drains `injector_local`, so it too starts with its own wave; under `ke16-a1` that stage
        // is compiled out and the first thing it touches is the global injector. Both readings are
        // the baseline this test exists to be compared against.
        return;
    }

    let first = observed[0];
    assert!(
        first < FOREIGN_TAG_BASE,
        "the worker joiner's first task under arm `{KE16_B}` was FOREIGN (tag {first}); \
         `KE16-DESIGN-B.md` §2.2 makes step 1 the joiner's OWN deque and the global injector step \
         2, so a foreign task first means the own-deque pop is not happening"
    );

    let expected_first = match KE16_A {
        // `ke16-a1`: LIFO deque, owner end — the joiner pops the NEWEST of its own wave.
        "a1" => OWN_TASKS - 1,
        // `ke16-a1-fifo`: FIFO owner end (today's end discipline) — the OLDEST comes back first.
        "a1f" => 0,
        // No other A arm can reach this line: `any(ke16-b1, ke16-b3)` without an A1 arm is a
        // `compile_error!` (`src/lib.rs`, `KE16-DESIGN.md` §4), so the B arms imply a1 / a1f.
        other => unreachable!("axis B is built without an A1 arm (KE16_A = {other})"),
    };
    assert_eq!(
        first, expected_first,
        "the joiner popped own task {first} first under `{KE16_A}`; that arm's owner end makes it \
         {expected_first} (`KE16-DESIGN-B.md` §2.2, \"Own scope first under LIFO\" — the FIFO row \
         inverts the order and is measured)"
    );
}

// =============================================================================================
// §2.7 — a claimed joiner runs the foreign task INSIDE its live join
// =============================================================================================

/// **`KE16-DESIGN-B.md` §2.7 — the B1-P receipt, with the "inline" half of the claim asserted.**
///
/// §2.7's sentence is: the parked joiner "was claimed, woke, unmarked, scanned, found the foreign
/// task in `injector_global` and ran it INLINE inside its join". The receipt already in
/// `ke16_nested_scope_occupancy.rs` asserts the lane equality and stops there, and the lane
/// equality alone is also produced by the fixture's fallback path (the sibling body's bounded
/// timeout releases the scope, the joiner returns, finishes its outer task and picks the foreign
/// task up from `worker_main` instead). This test adds the discriminator.
///
/// `join_open` is stored by the joining thread itself — `true` as the last act inside the scope
/// closure, `false` as the first act after `scope` returns — and READ by the foreign body. When the
/// receipt holds, both stores and the read are on one thread, so "the join had not returned yet"
/// is a program-order fact, not a timing estimate.
///
/// Under `b0` the joiner never marks its idle bit, so no claim is possible and the fixture cannot
/// build the state at all; that reading is RECORDED (it is what B1-P adds) and the test returns
/// without asserting.
#[test]
fn the_claimed_joiner_runs_the_foreign_task_inside_its_live_join() {
    println!("KE16 variant: {}", boyko_threadpool::ke16_variant());
    ke16_check_expected_variant();

    const WORKERS: usize = 2;
    let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();

    let joiner_lane = Arc::new(AtomicUsize::new(UNSET));
    let body_lane = Arc::new(AtomicUsize::new(UNSET));
    let foreign_lane = Arc::new(AtomicUsize::new(UNSET));
    // `0` = the foreign body never ran, `1` = it ran while the join was open, `2` = after it.
    let foreign_saw_open_join = Arc::new(AtomicUsize::new(0));
    let body_started = Arc::new(AtomicBool::new(false));
    let release_body = Arc::new(AtomicBool::new(false));
    let outer_done = Arc::new(AtomicBool::new(false));
    let join_open = Arc::new(AtomicBool::new(false));

    {
        let joiner_lane = Arc::clone(&joiner_lane);
        let body_lane = Arc::clone(&body_lane);
        let body_started = Arc::clone(&body_started);
        let release_body = Arc::clone(&release_body);
        let outer_done = Arc::clone(&outer_done);
        let join_open = Arc::clone(&join_open);
        pool.spawn(move || {
            joiner_lane.store(current_worker_id() as usize, Ordering::Release);
            let dispatched = try_with_active_pool(|inner| {
                inner.scope(|nested| {
                    let body_lane = Arc::clone(&body_lane);
                    let started_in_body = Arc::clone(&body_started);
                    let release_body = Arc::clone(&release_body);
                    nested.spawn(move || {
                        body_lane.store(current_worker_id() as usize, Ordering::Release);
                        started_in_body.store(true, Ordering::Release);
                        // Held until the foreign task releases it, so the parked joiner is the ONLY
                        // claimable lane. Bounded so a build that never runs the foreign task fails
                        // the receipt below instead of wedging the suite.
                        let deadline = Instant::now() + WAIT_BOUND;
                        while !release_body.load(Ordering::Acquire) && Instant::now() < deadline {
                            std::hint::spin_loop();
                        }
                    });
                    // Wait INSIDE the closure: if the body had not started, this thread would pop
                    // its own task at step 1 of the join and never park, and the fixture would be
                    // measuring nothing. Bounded by `BODY_START_BOUND`, not `WAIT_BOUND`, because
                    // the one build that cannot satisfy this precondition is the one with defect A
                    // still in it, and there the wait is pure dead time.
                    let deadline = Instant::now() + BODY_START_BOUND;
                    while !body_started.load(Ordering::Acquire) && Instant::now() < deadline {
                        std::hint::spin_loop();
                    }
                    join_open.store(true, Ordering::Release);
                });
                join_open.store(false, Ordering::Release);
            });
            assert!(
                dispatched.is_some(),
                "the outer task saw no active pool; the nested scope was never opened"
            );
            outer_done.store(true, Ordering::Release);
        });
    }

    // Wait for the joiner to be identified AND idle-marked. The mask flickers (the park has a
    // backstop, so the joiner wakes, rescans and re-marks); any observation of the bit is enough.
    let mask_deadline = Instant::now() + Duration::from_secs(2);
    let mut parked_joiner = UNSET;
    while Instant::now() < mask_deadline {
        let id = joiner_lane.load(Ordering::Acquire);
        if id < pool.worker_count() as usize && pool.parked_mask() & (1u64 << id) != 0 {
            parked_joiner = id;
            break;
        }
        std::hint::spin_loop();
    }

    if KE16_B == "b0" {
        println!(
            "[ke16 b1-p inline] arm=b0 joiner_bit_observed={} — B0's joiner does not mark its idle \
             bit, so it is invisible to every foreign wave's wake decision (`KE16-DESIGN-B.md` §1 \
             item 5). Recorded, not asserted.",
            parked_joiner != UNSET
        );
        // Let the fixture unwind: nothing will claim the joiner, so release the body by hand.
        release_body.store(true, Ordering::Release);
        assert!(
            spin_until(&outer_done),
            "the b0 fixture never completed even after the body was released by hand"
        );
        return;
    }

    assert_ne!(
        parked_joiner, UNSET,
        "no joiner bit was ever set in parked_mask (joiner_lane={}, mask={:#x}) under arm \
         `{KE16_B}`: the worker joiner did not park idle-marked, so rule B1-P is not in this build",
        joiner_lane.load(Ordering::Acquire),
        pool.parked_mask()
    );

    {
        let foreign_lane = Arc::clone(&foreign_lane);
        let foreign_saw_open_join = Arc::clone(&foreign_saw_open_join);
        let release_body = Arc::clone(&release_body);
        let join_open = Arc::clone(&join_open);
        pool.spawn(move || {
            foreign_lane.store(current_worker_id() as usize, Ordering::Release);
            foreign_saw_open_join.store(
                if join_open.load(Ordering::Acquire) { 1 } else { 2 },
                Ordering::Release,
            );
            // Ordered after both receipts: releasing the sibling ends the scope, and a reader must
            // not see the release without the values it is paired with.
            release_body.store(true, Ordering::Release);
        });
    }

    assert!(
        spin_until(&outer_done),
        "the outer task never completed: the parked joiner was never woken, or its scope never \
         drained"
    );

    let joiner = joiner_lane.load(Ordering::Acquire);
    let body = body_lane.load(Ordering::Acquire);
    let foreign = foreign_lane.load(Ordering::Acquire);
    let inline = foreign_saw_open_join.load(Ordering::Acquire);
    println!(
        "[ke16 b1-p inline] arm={KE16_B} joiner_lane={joiner} body_lane={body} \
         foreign_lane={foreign} foreign_ran_inside_join={} workers={}",
        inline == 1,
        pool.worker_count()
    );

    assert!(
        is_registered_lane(&pool, joiner),
        "the outer task did not run on a registered worker (id={joiner}); the fixture measured the \
         dispatcher route, which has no idle bit to claim"
    );
    assert_ne!(
        body, joiner,
        "the scope's body ran on the joining worker itself, so that worker never parked and the \
         fixture never built the state B1-P is about"
    );
    assert_eq!(
        foreign, joiner,
        "the foreign task ran on lane {foreign}, not on the parked joiner ({joiner}): the joiner's \
         idle bit was set but its wake did not reach it, or another lane was claimable"
    );
    assert_eq!(
        inline, 1,
        "the foreign task ran on the joiner's lane but NOT inside its live join (marker {inline}; \
         1 = inside, 2 = after the join returned, 0 = never ran). Lane equality alone is also \
         produced by the joiner returning from its join and picking the task up in `worker_main`, \
         which is not what `KE16-DESIGN-B.md` §2.7 claims: B1-P's content is that a PARKED JOINER \
         is a claimable lane, i.e. that the foreign task runs INLINE inside the join"
    );
}

// =============================================================================================
// §2.6 case (a) — the joiner that is the pool's only lane
// =============================================================================================

/// **`KE16-DESIGN-B.md` §2.6 — "it either (a) runs a ready task, ... or (b) parks with a wake".**
///
/// At `num_threads(1)` case (b) has no completer: no other thread exists to finish the wave, so the
/// scope terminates only if the joiner itself runs every task. That makes W = 1 the sharpest
/// liveness gate for the worker arm — if step 1 ever stopped reaching the joiner's own deque (the
/// A1 deposit lost, the identity predicate answering `None`, the pop taking the wrong end of an
/// empty deque), this hangs where a W ≥ 2 fixture would quietly be rescued by a sibling.
///
/// It is also the route-(b) mirror of `tests/cross_pool_routing.rs::
/// install_on_the_only_worker_of_its_pool_completes`, which covers the same W = 1 corner on the
/// EXTERNAL arm (an `install` frame, whose id App-6 rewrites to the dispatcher sentinel).
///
/// Deliberately nested TWO deep: §2.6's case (a) says a ready task "completes unless it opens a
/// nested scope — whose join helps in the same way", and one level would not exercise the recursion
/// the sentence licenses.
#[test]
fn a_nested_scope_on_a_single_worker_pool_completes() {
    println!("KE16 variant: {}", boyko_threadpool::ke16_variant());
    ke16_check_expected_variant();

    const OUTER: usize = 4;
    const INNER: usize = 4;

    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let ran = Arc::new(AtomicUsize::new(0));
    let lanes_seen = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));

    {
        let ran = Arc::clone(&ran);
        let lanes_seen = Arc::clone(&lanes_seen);
        let done = Arc::clone(&done);
        pool.spawn(move || {
            let dispatched = try_with_active_pool(|inner| {
                inner.scope(|outer| {
                    for _ in 0..OUTER {
                        let ran = Arc::clone(&ran);
                        let lanes_seen = Arc::clone(&lanes_seen);
                        outer.spawn(move || {
                            lanes_seen.fetch_or(1usize << current_worker_id(), Ordering::AcqRel);
                            // Every one of these opens a nested scope of its own, so the joiner
                            // recurses: an outer task's join runs an inner task whose join runs
                            // another. Nothing else in the pool can help.
                            let nested = try_with_active_pool(|inner2| {
                                inner2.scope(|deep| {
                                    for _ in 0..INNER {
                                        let ran = Arc::clone(&ran);
                                        deep.spawn(move || {
                                            ran.fetch_add(1, Ordering::AcqRel);
                                        });
                                    }
                                });
                            });
                            assert!(
                                nested.is_some(),
                                "an outer task saw no active pool; the second level never opened"
                            );
                            ran.fetch_add(1, Ordering::AcqRel);
                        });
                    }
                });
            });
            assert!(
                dispatched.is_some(),
                "the driving task saw no active pool; the first level never opened"
            );
            done.store(true, Ordering::Release);
        });
    }

    let completed = spin_until(&done);
    let total = ran.load(Ordering::Acquire);
    println!(
        "[ke16 b-w1] arm={KE16_B} completed={completed} tasks_run={total} \
         expected={} lane_mask={:#x}",
        OUTER + OUTER * INNER,
        lanes_seen.load(Ordering::Acquire)
    );

    assert!(
        completed,
        "a two-level nested scope on a one-worker pool did not complete within {WAIT_BOUND:?} \
         under arm `{KE16_B}`: the joining worker is the pool's ONLY lane, so case (b) of \
         `KE16-DESIGN-B.md` §2.6 has no completer and the wave can only be run by the joiner \
         itself ({total} of {} tasks had run)",
        OUTER + OUTER * INNER
    );
    assert_eq!(
        total,
        OUTER + OUTER * INNER,
        "the scopes returned but not every body ran; a join returned before its wave drained"
    );
    assert_eq!(
        lanes_seen.load(Ordering::Acquire),
        1,
        "a body ran on a lane other than worker 0 on a one-worker pool"
    );
}
