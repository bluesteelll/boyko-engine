//! KE16 axis W — the wake protocol and the count-gated completion (W-d′),
//! observed from outside the crate as LIVENESS and EXACTLY-ONCE accounting.
//!
//! ## Why this file exists
//!
//! `KE16-DESIGN.md` §8 discharges the W-d′ obligation through loom, and that
//! model RUNS on this box. Its colour, measured 2026-09-03 in this worktree
//! with the recipe recorded in `tests/loom_pool.rs`'s header
//! (`cargo --config 'target.x86_64-pc-windows-gnu.rustflags=["-C","target-cpu=
//! x86-64-v3","--cfg","loom"]' … --no-run`, then the emitted exe per test with
//! `LOOM_MAX_PREEMPTIONS=3 --test-threads=1 --exact`) — that gnu-triple key is
//! the one the 2026-09-03 run used and is kept here as the record of it; the
//! recipe to RUN TODAY is re-keyed to `target."cfg(windows)"` (§"The loom
//! recipe" below, and `tests/loom_pool.rs`'s header in full):
//!
//! | Obligation | Design's gate | Reading on this box |
//! |---|---|---|
//! | W-d′ (i)–(iii) (count-gated completion) | loom M1c (`KE16-DESIGN-W.md` §3.7) | `loom_m1c_count_gated_completion_wakes_the_parked_worker_joiner` **green** (0.40 s), with a REAL parked joiner — not the "count only" degradation §3.7 allows for |
//! | W-d′ route-(b) liveness | Miri `nested_scope_from_worker_is_stolen_by_sibling` at 32 seeds (`tests/miri_scope.rs`) | runs unconditionally; that target is `#![cfg(miri)]`, so a NATIVE `cargo test` of it lists nothing — see the route-(b) row below |
//!
//! So this file is NOT the W-d′ obligation's discharge and must never be filed
//! as one — the model is. It is its CORROBORATION on real hardware plus the
//! backstop census: an exhaustive model over a toy transport says nothing about
//! whether the shipped `crossbeam` push, the Windows `park`/`unpark` pair and
//! the real worker loop compose the way the model's transcription assumes, and
//! nothing here can say anything about an interleaving the hardware did not
//! happen to produce.
//!
//! Its honest boundary, unchanged: a lost wake that the `park_timeout` backstop
//! recovers is invisible here BY CONSTRUCTION — [`DEADLINE`] is three orders of
//! magnitude above the backstop — so these tests red only on a wake that is lost
//! with no recovery at all. The backstop's own presence is therefore pinned
//! mechanically (`the_join_backstop_source_constant_is_still_50_us`), because it
//! is the premise that silence rests on. The one route with NO backstop on the
//! stack is `ThreadPool::spawn`, and it has its own row here
//! (`w_b_a_fire_and_forget_wave_into_a_parked_pool_…`).
//!
//! ## The loom recipe, because the obvious one is a trap
//!
//! `RUSTFLAGS="--cfg loom" cargo test --release -p boyko-threadpool --test
//! loom_pool` — the spelling this file's header carried until 2026-09-03 and the
//! one `KE16-DESIGN-MEASUREMENT.md` inherited — was observed on 2026-09-02 to
//! produce a binary that dies at startup with `STATUS_ACCESS_VIOLATION`
//! (0xC0000005) before libtest prints `running N tests`, and that reading was
//! then written up as "loom does not run on this box". It is a RECIPE fault, not
//! a box fault: `RUSTFLAGS` REPLACES `[target.<triple>].rustflags` rather than
//! merging with it (`.cargo/config.toml`, the `x86-64-v3` baseline), so that
//! command builds a differently-configured tree. The spelling that survives it
//! is `cargo --config 'target."cfg(windows)".rustflags=["--cfg","loom"]'`, and
//! under it the models list and run.
//!
//! ⚠ **That key was `target.x86_64-pc-windows-gnu.rustflags=["-C","target-cpu=
//! x86-64-v3","--cfg","loom"]` until 2026-09-10**, when this tree's Windows
//! recipes moved to `stable-x86_64-pc-windows-msvc`, spelled explicitly through
//! `RUSTUP_TOOLCHAIN`. (The rustup DEFAULT host is still gnu as of that date;
//! `rustup set default-host` is a later, owner-run step — which is precisely why
//! the key must match EITHER host.) A triple key that does
//! not match the build simply contributes nothing: `--cfg loom` never reaches
//! rustc, the models compile away and the binary exits **0** on `running 0
//! tests`. `cfg(windows)` matches either host and JOINS `.cargo/config.toml`'s
//! per-triple `-C target-cpu=x86-64-v3` rather than replacing it, so the ISA
//! baseline no longer has to be restated (measured 2026-09-10 off `cargo -v`).
//!
//! ## The receipt against a blind pass
//!
//! The wake protocol is only under test when the destination's siblings are
//! actually ASLEEP: a pool whose workers are all spinning finds the task
//! without any wake, and the run proves nothing. Each liveness test therefore
//! counts the repeats in which `ThreadPool::parked_mask()` showed EVERY worker
//! parked immediately before the wave, prints that count, and asserts it is
//! non-zero. A run that never managed to park the pool is reported as such
//! instead of passing.
//!
//! ## Run
//!
//! ```text
//! cargo test -p boyko-threadpool --test ke16_w_wake_completion -- --test-threads=1 --nocapture
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::{ThreadPool, ThreadPoolBuilder, current_worker_id};

/// Workers in every fixture pool.
///
/// Four, not `available_parallelism()`: the property under test is "a task is
/// not stranded while the siblings sleep", which needs siblings that can sleep,
/// not a wide pool. A fixed width also makes the printed receipt comparable
/// between machines.
const WORKERS: u32 = 4;

/// Tasks per wave.
///
/// 64, not a handful: a wide wave gives the pusher many chances to store a task
/// while a thief is draining behind it, which is the interleaving a lost wake
/// needs. The one-, two- and three-task corner is covered separately by
/// [`w_b_the_boundary_wave_sizes_one_two_and_three_drain_a_parked_pool`].
const TASKS_PER_WAVE: usize = 64;

/// Waves per liveness test.
///
/// The window a lost wake needs is nanoseconds wide on the spawner against a
/// whole task body on a thief — so a single wave is not a measurement of
/// anything. Repetition is the only lever a native test has on a race this
/// narrow.
const WAVES: usize = 64;

/// How long a driver thread may take before the test calls it a hang.
///
/// Three orders of magnitude above the ~1 ms the `park_timeout` backstop costs
/// on Windows (`KE16-DESIGN.md` §6 ruling 4 measured 1 021 µs with the guard
/// held), so a backstop-recovered wake is NOT what reds this test. Only a wake
/// that is lost with no recovery at all reaches this deadline.
const DEADLINE: Duration = Duration::from_secs(30);

/// How long to wait for every worker to be parked and idle-marked before a
/// wave.
///
/// Best effort: a repeat that does not reach it still runs its wave (and still
/// carries the exactly-once oracle), it just does not count towards the
/// "siblings were actually asleep" receipt.
const QUIESCE_BUDGET: Duration = Duration::from_millis(200);

/// A pool whose workers have all parked, or the budget expired.
///
/// Returns `true` iff every worker's bit was set in `parked_mask()` — the
/// receipt that this repeat put the wake protocol under load rather than
/// handing the wave to a spinning worker.
fn quiesce(pool: &ThreadPool) -> bool {
    let deadline = Instant::now() + QUIESCE_BUDGET;
    while pool.parked_mask().count_ones() < WORKERS {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::yield_now();
    }
    true
}

/// Run `f` on its own thread and fail the test if it has not returned within
/// [`DEADLINE`].
///
/// The driver must not be the test thread: every shape here ends in a blocking
/// join, so a lost wake with no backstop would hang the harness itself and be
/// reported as a timeout by whoever ran it rather than as a named failure. On
/// expiry the driver thread is abandoned deliberately — the process is about to
/// unwind this test, and a strand that is still stranded cannot be joined.
fn drive_with_deadline<F>(label: &str, f: F)
where
    F: FnOnce() + Send + 'static,
{
    let done = Arc::new(AtomicBool::new(false));
    let done_driver = Arc::clone(&done);
    std::thread::spawn(move || {
        f();
        done_driver.store(true, Ordering::Release);
    });

    let expiry = Instant::now() + DEADLINE;
    while !done.load(Ordering::Acquire) {
        assert!(
            Instant::now() < expiry,
            "{label}: the driver did not finish within {DEADLINE:?}. Every pushed task is \
             eventually run and the delay is bounded by one task body; the `park_timeout` \
             backstop bounds it again at ~1 ms. A wave outstanding this long is a wake that was \
             lost AND not recovered."
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// One `AtomicU32` per task of a wave, so "exactly once" is an assertion about
/// each task and not about a sum that two errors could cancel in.
fn fresh_counters(n: usize) -> Arc<Vec<AtomicU32>> {
    Arc::new((0..n).map(|_| AtomicU32::new(0)).collect())
}

fn assert_each_ran_once(counters: &[AtomicU32], label: &str) {
    for (i, c) in counters.iter().enumerate() {
        assert_eq!(
            c.load(Ordering::Acquire),
            1,
            "{label}: task {i} of the wave ran {} times, not once",
            c.load(Ordering::Acquire)
        );
    }
}

/// W-b, the NON-LANE pusher: the dispatcher pushes a wave into `injector_global`
/// while every worker of the pool is parked, and every task must run exactly
/// once.
///
/// This test's deadline is far above the joiner's `park_timeout` backstop: a red
/// here is a strand that nothing recovered, and a green here does NOT claim the
/// window is absent — only that it is bounded.
#[test]
fn w_b_a_dispatcher_wave_into_a_parked_pool_runs_every_task_exactly_once() {
    println!(
        "[KE16 W] dispatcher (non-lane) pusher, W={WORKERS} tasks={TASKS_PER_WAVE} waves={WAVES}"
    );

    let quiesced = Arc::new(AtomicU32::new(0));
    let worst_us = Arc::new(AtomicU32::new(0));
    let counters = fresh_counters(TASKS_PER_WAVE * WAVES);

    let quiesced_driver = Arc::clone(&quiesced);
    let worst_driver = Arc::clone(&worst_us);
    let counters_driver = Arc::clone(&counters);

    drive_with_deadline(
        "w_b_a_dispatcher_wave_into_a_parked_pool_runs_every_task_exactly_once",
        move || {
            let pool = ThreadPoolBuilder::new()
                .num_threads(WORKERS as usize)
                .build();

            for wave in 0..WAVES {
                if quiesce(&pool) {
                    quiesced_driver.fetch_add(1, Ordering::AcqRel);
                }
                let base = wave * TASKS_PER_WAVE;
                let started = Instant::now();
                // `install`, not `scope`: the dispatcher is outside the pool, so
                // it has no ambient-pool TLS and `ThreadPool::scope`'s
                // `debug_assert` refuses it. `install` opens the frame AND the
                // scope, and the pushes it drives are the dispatcher's own —
                // the non-lane pusher this row exists to put under load.
                pool.install(|s| {
                    for i in 0..TASKS_PER_WAVE {
                        let counters = Arc::clone(&counters_driver);
                        s.spawn(move || {
                            counters[base + i].fetch_add(1, Ordering::AcqRel);
                        });
                    }
                });
                let elapsed = started.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;
                worst_driver.fetch_max(elapsed, Ordering::AcqRel);
            }
        },
    );

    println!(
        "[KE16 W] route=dispatcher waves={WAVES} quiesced_before_wave={} \
         worst_wave_wall={} us",
        quiesced.load(Ordering::Acquire),
        worst_us.load(Ordering::Acquire)
    );

    assert!(
        quiesced.load(Ordering::Acquire) > 0,
        "not one of the {WAVES} waves was pushed into a fully parked pool, so the wake protocol \
         was never under load and this run measures nothing (`KE16-DESIGN-MEASUREMENT.md` §5 \
         item 1 applied to a receipt rather than a test count)"
    );
    assert_each_ran_once(
        &counters,
        "w_b_a_dispatcher_wave_into_a_parked_pool_runs_every_task_exactly_once",
    );
}

/// W-b, the NON-LANE pusher with NO JOINER AT ALL: a fire-and-forget
/// [`ThreadPool::spawn`] wave into a parked pool.
///
/// This is the row the code reviewer of 2026-09-03 added, and it is the
/// *unbounded* instance rather than another copy of the dispatcher row above.
/// `ThreadPool::spawn` (`thread_pool.rs` → `PoolInner::spawn` →
/// `worker::push_task`) opens no scope, so nothing ever calls
/// `join_workers_until_drained` and the `park_timeout` backstop that bounds
/// every other non-lane push (`scope.rs`'s `JOIN_BACKSTOP`) is never on the
/// stack. Meanwhile a worker parks UNTIMED (`worker::worker_main`'s
/// `std::thread::park()`).
///
/// The test therefore keeps the pool ALIVE while it waits: the wave is awaited
/// on a per-task counter, never by dropping the pool, so teardown cannot be
/// what completes it. A red here is a strand nothing recovered; a green does
/// NOT claim the window is absent — [`DEADLINE`] is 30 s against a window of
/// nanoseconds.
#[test]
fn w_b_a_fire_and_forget_wave_into_a_parked_pool_runs_every_task_exactly_once() {
    println!(
        "[KE16 W] fire-and-forget (non-lane, no joiner) pusher, \
         W={WORKERS} tasks={TASKS_PER_WAVE} waves={WAVES}"
    );

    let quiesced = Arc::new(AtomicU32::new(0));
    let worst_us = Arc::new(AtomicU32::new(0));
    let counters = fresh_counters(TASKS_PER_WAVE * WAVES);

    let quiesced_driver = Arc::clone(&quiesced);
    let worst_driver = Arc::clone(&worst_us);
    let counters_driver = Arc::clone(&counters);

    drive_with_deadline(
        "w_b_a_fire_and_forget_wave_into_a_parked_pool_runs_every_task_exactly_once",
        move || {
            let pool = ThreadPoolBuilder::new()
                .num_threads(WORKERS as usize)
                .build();

            for wave in 0..WAVES {
                if quiesce(&pool) {
                    quiesced_driver.fetch_add(1, Ordering::AcqRel);
                }
                let base = wave * TASKS_PER_WAVE;
                let ran = Arc::new(AtomicU32::new(0));
                let started = Instant::now();

                for i in 0..TASKS_PER_WAVE {
                    let counters = Arc::clone(&counters_driver);
                    let ran = Arc::clone(&ran);
                    pool.spawn(move || {
                        counters[base + i].fetch_add(1, Ordering::AcqRel);
                        ran.fetch_add(1, Ordering::AcqRel);
                    });
                }

                // Wait on the wave's OWN completion count with the pool still
                // alive. Dropping it here would unpark every worker and thereby
                // recover exactly the strand this row exists to expose.
                while (ran.load(Ordering::Acquire) as usize) < TASKS_PER_WAVE {
                    std::thread::yield_now();
                }

                let elapsed = started.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;
                worst_driver.fetch_max(elapsed, Ordering::AcqRel);
            }
        },
    );

    println!(
        "[KE16 W] route=fire_and_forget waves={WAVES} quiesced_before_wave={} \
         worst_wave_wall={} us",
        quiesced.load(Ordering::Acquire),
        worst_us.load(Ordering::Acquire)
    );

    assert!(
        quiesced.load(Ordering::Acquire) > 0,
        "not one of the {WAVES} waves was pushed into a fully parked pool, so the wake protocol \
         was never under load and this run measures nothing (`KE16-DESIGN-MEASUREMENT.md` §5 \
         item 1 applied to a receipt rather than a test count)"
    );
    assert_each_ran_once(
        &counters,
        "w_b_a_fire_and_forget_wave_into_a_parked_pool_runs_every_task_exactly_once",
    );
}

/// W-b, the LANE pusher: the wave is spawned from INSIDE a worker's task body.
///
/// The wave lands on that worker's OWN registered deque, where a sibling can
/// steal it, so this row puts the wake protocol under load from a pusher that is
/// itself a worker of the destination pool. Its assertion is the same as the
/// dispatcher row's — the wave completes and every task runs exactly once — and
/// having both rows in one file is what makes the pusher axis visible at all.
#[test]
fn w_b_a_worker_spawned_wave_runs_every_task_exactly_once() {
    println!("[KE16 W] worker (lane) pusher, W={WORKERS} tasks={TASKS_PER_WAVE} waves={WAVES}");

    let quiesced = Arc::new(AtomicU32::new(0));
    let on_worker = Arc::new(AtomicU32::new(0));
    let counters = fresh_counters(TASKS_PER_WAVE * WAVES);

    let quiesced_driver = Arc::clone(&quiesced);
    let on_worker_driver = Arc::clone(&on_worker);
    let counters_driver = Arc::clone(&counters);

    drive_with_deadline(
        "w_b_a_worker_spawned_wave_runs_every_task_exactly_once",
        move || {
            let pool = ThreadPoolBuilder::new()
                .num_threads(WORKERS as usize)
                .build();

            for wave in 0..WAVES {
                if quiesce(&pool) {
                    quiesced_driver.fetch_add(1, Ordering::AcqRel);
                }
                let base = wave * TASKS_PER_WAVE;
                let outer_done = Arc::new(AtomicBool::new(false));

                let inner_pool = Arc::clone(&pool);
                let inner_counters = Arc::clone(&counters_driver);
                let inner_on_worker = Arc::clone(&on_worker_driver);
                let inner_done = Arc::clone(&outer_done);
                // `spawn`, not `scope`: the outer body must run on a WORKER, and
                // a scope's join would let the test thread drain the wave itself
                // — the pusher would then be the dispatcher and this row would
                // duplicate the one above.
                pool.spawn(move || {
                    if current_worker_id() < WORKERS {
                        inner_on_worker.fetch_add(1, Ordering::AcqRel);
                    }
                    inner_pool.scope(|s| {
                        for i in 0..TASKS_PER_WAVE {
                            let counters = Arc::clone(&inner_counters);
                            s.spawn(move || {
                                counters[base + i].fetch_add(1, Ordering::AcqRel);
                            });
                        }
                    });
                    inner_done.store(true, Ordering::Release);
                });

                while !outer_done.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
            }
        },
    );

    println!(
        "[KE16 W] route=worker waves={WAVES} quiesced_before_wave={} \
         outer_body_on_a_registered_worker={}",
        quiesced.load(Ordering::Acquire),
        on_worker.load(Ordering::Acquire)
    );

    assert_eq!(
        on_worker.load(Ordering::Acquire) as usize,
        WAVES,
        "every outer body must have run on a registered worker of the pool; a body that ran \
         elsewhere makes this the dispatcher row again (`KE16-DESIGN-MEASUREMENT.md` §5 item 2, \
         the healthy-route-twice shape)"
    );
    assert_each_ran_once(
        &counters,
        "w_b_a_worker_spawned_wave_runs_every_task_exactly_once",
    );
}

/// W-d′ route (b): the scope's join is a WORKER's own, which is the join that
/// decrements first and unparks only on the last completion, through the
/// `PoolInner`-owned `WakeHandle`.
///
/// `KE16-DESIGN-W.md` §3.7 makes the 32-seed Miri run of
/// `nested_scope_from_worker_is_stolen_by_sibling` the SOLE liveness gate for
/// this route whenever M1c degrades to count-only. M1c did NOT degrade: it runs
/// on this box with a real parked loom joiner and is green (0.40 s, 2026-09-03
/// — see `tests/loom_pool.rs`'s header), so route (b) has its exhaustive model
/// and this row is corroboration on real hardware rather than a substitute.
///
/// That Miri run is a `cargo +nightly miri test` run. `miri_scope.rs` carries a
/// file-level `#![cfg(miri)]`, so a NATIVE `cargo test` of that target lists
/// nothing and prints `running 0 tests` — that native reading is not evidence
/// about this gate in either direction, and must not be read as one.
///
/// The shape that makes it route (b) and keeps it there: the outer body is
/// delivered with `pool.spawn`, so it runs on a worker; the ONLY scope join in
/// the test is that worker's; the test thread waits on an `AtomicBool` and
/// never opens a scope of its own.
#[test]
fn w_d_prime_a_worker_route_join_terminates_and_runs_every_task_exactly_once() {
    println!("[KE16 W] route (b), worker joiner, W={WORKERS} tasks={TASKS_PER_WAVE} joins={WAVES}");

    let joins_on_worker = Arc::new(AtomicU32::new(0));
    let worst_us = Arc::new(AtomicU32::new(0));
    let counters = fresh_counters(TASKS_PER_WAVE * WAVES);

    let joins_driver = Arc::clone(&joins_on_worker);
    let worst_driver = Arc::clone(&worst_us);
    let counters_driver = Arc::clone(&counters);

    drive_with_deadline(
        "w_d_prime_a_worker_route_join_terminates_and_runs_every_task_exactly_once",
        move || {
            let pool = ThreadPoolBuilder::new()
                .num_threads(WORKERS as usize)
                .build();

            for wave in 0..WAVES {
                let base = wave * TASKS_PER_WAVE;
                let outer_done = Arc::new(AtomicBool::new(false));

                let inner_pool = Arc::clone(&pool);
                let inner_counters = Arc::clone(&counters_driver);
                let inner_joins = Arc::clone(&joins_driver);
                let inner_worst = Arc::clone(&worst_driver);
                let inner_done = Arc::clone(&outer_done);
                pool.spawn(move || {
                    let started = Instant::now();
                    inner_pool.scope(|s| {
                        for i in 0..TASKS_PER_WAVE {
                            let counters = Arc::clone(&inner_counters);
                            s.spawn(move || {
                                counters[base + i].fetch_add(1, Ordering::AcqRel);
                            });
                        }
                    });
                    // Read AFTER the join returns: the id identifies the thread
                    // that performed the join, which is the predicate
                    // `worker_lane_for` uses to pick the count-gated arm.
                    if current_worker_id() < WORKERS {
                        inner_joins.fetch_add(1, Ordering::AcqRel);
                    }
                    let elapsed = started.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;
                    inner_worst.fetch_max(elapsed, Ordering::AcqRel);
                    inner_done.store(true, Ordering::Release);
                });

                while !outer_done.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
            }
        },
    );

    println!(
        "[KE16 W] route=b joins={WAVES} joined_on_a_registered_worker={} \
         worst_join_wall={} us",
        joins_on_worker.load(Ordering::Acquire),
        worst_us.load(Ordering::Acquire)
    );

    assert_eq!(
        joins_on_worker.load(Ordering::Acquire) as usize,
        WAVES,
        "every join must have been performed by a registered worker of the pool — otherwise this \
         is the EXTERNAL arm, whose lost-wakeup window `KE16-DESIGN-W.md` §3.5 keeps on purpose, \
         and the row would be green for a reason that has nothing to do with W-d′"
    );
    assert_each_ran_once(
        &counters,
        "w_d_prime_a_worker_route_join_terminates_and_runs_every_task_exactly_once",
    );
}

/// The `park_timeout` backstop is the premise every "green" above rests on, so
/// its disappearance must be a test failure and not a silent widening of the
/// window.
///
/// `KE16-DESIGN-W.md` §3.4: "The 50 µs constant at `scope.rs:512` stays as a
/// defensive bound (it costs nothing when the wake arrives)". A source census
/// rather than a behavioural check because the behaviour it guards is precisely
/// the one no native test can see — a wake that was lost and then recovered
/// looks exactly like a wake that arrived.
#[test]
fn the_join_backstop_source_constant_is_still_50_us() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/scope.rs");
    let src =
        std::fs::read_to_string(path).expect("test setup: crates/boyko_threadpool/src/scope.rs");

    assert!(
        src.contains("const JOIN_BACKSTOP: Duration = Duration::from_micros(50);"),
        "`JOIN_BACKSTOP` is no longer the design's 50 µs source constant in {path}. Every \
         liveness row of this file is deliberately blind to a wake that the backstop recovers \
         (its deadline is {DEADLINE:?}); with the backstop gone or lengthened that blindness \
         stops being safe and the rows must be re-derived, not re-blessed."
    );
    assert!(
        src.contains("std::thread::park_timeout(JOIN_BACKSTOP)"),
        "no join path in {path} parks with the backstop any more; see the message above"
    );
}

// ===========================================================================
// Tester-added rows (KE16 axis W). The rows above cover the two pusher routes
// at one wave size and the count-gated join at one shape; these cover the
// small-wave corner and the empty-wave corner of the count gate.
// ===========================================================================

/// Repeats per wave size in the small-wave row.
///
/// Below [`WAVES`] because each repeat there also pays a [`QUIESCE_BUDGET`]
/// poll: three sizes x 64 repeats already gives every push position of a small
/// wave many chances against a parked pool.
const BOUNDARY_REPEATS: usize = 64;

/// Waves of exactly one, two and three tasks against a parked pool.
///
/// Every other liveness row in this file runs [`TASKS_PER_WAVE`] = 64, where a
/// single lost wake is masked: sixty-three other pushes wake the pool behind it
/// and the wave drains anyway.
///
/// The route is the dispatcher's: the test's own driver thread opens each
/// scope, so the pushes land in the pool's non-lane destination.
#[test]
fn w_b_the_boundary_wave_sizes_one_two_and_three_drain_a_parked_pool() {
    println!(
        "[KE16 W] dispatcher pusher, W={WORKERS} wave sizes 1,2,3 x {BOUNDARY_REPEATS} repeats"
    );

    let quiesced = Arc::new(AtomicU32::new(0));
    let counters = fresh_counters((1 + 2 + 3) * BOUNDARY_REPEATS);

    let quiesced_driver = Arc::clone(&quiesced);
    let counters_driver = Arc::clone(&counters);

    drive_with_deadline(
        "w_b_the_boundary_wave_sizes_one_two_and_three_drain_a_parked_pool",
        move || {
            let pool = ThreadPoolBuilder::new()
                .num_threads(WORKERS as usize)
                .build();

            let mut base = 0usize;
            for size in 1..=3usize {
                for _ in 0..BOUNDARY_REPEATS {
                    if quiesce(&pool) {
                        quiesced_driver.fetch_add(1, Ordering::AcqRel);
                    }
                    // `install`, not `scope`: the driver is not a worker of
                    // this pool, and `ThreadPool::scope` panics off an active
                    // pool by design (`thread_pool.rs`). This is also what
                    // keeps the row on the dispatcher route.
                    pool.install(|s| {
                        for i in 0..size {
                            let counters = Arc::clone(&counters_driver);
                            s.spawn(move || {
                                counters[base + i].fetch_add(1, Ordering::AcqRel);
                            });
                        }
                    });
                    base += size;
                }
            }
        },
    );

    println!(
        "[KE16 W] route=dispatcher shape=boundary \
         repeats_per_size={BOUNDARY_REPEATS} quiesced_before_wave={}",
        quiesced.load(Ordering::Acquire)
    );

    assert!(
        quiesced.load(Ordering::Acquire) > 0,
        "not one of the {} repeats found every worker parked before its wave, so the wake \
         protocol was never under test — the waves were handed to spinning workers (the receipt \
         this file's header calls the guard against a blind pass)",
        3 * BOUNDARY_REPEATS
    );
    assert_each_ran_once(
        &counters,
        "w_b_the_boundary_wave_sizes_one_two_and_three_drain_a_parked_pool",
    );
}

/// Empty scopes opened back to back inside one outer body.
///
/// More than one because the shape's failure mode is a leftover token or a
/// leftover count, which the FIRST empty join would create and the SECOND would
/// trip over.
const EMPTY_SCOPES: usize = 8;

/// Tasks in the wave that proves the pool still works after the empty joins.
const AFTER_EMPTY: usize = 16;

/// W-d′ at the empty-wave corner: a WORKER opens a scope that spawns nothing.
///
/// This is the one route-(b) shape in which the count gate's wake can never
/// fire: `pending` is zero from the start, so no completer ever runs and no
/// `fetch_sub` ever returns 1. The scope's `joiner_wake` is nevertheless
/// non-null (the opener is a registered worker of this pool,
/// `KE16-DESIGN-W.md` §3.2), so the join must return on the drained check
/// alone. A join that parked here would be recovered only by the 50 µs
/// backstop, and — since nothing will ever wake it — would re-park on every
/// expiry, i.e. spin at the backstop's period for as long as the scope lives.
///
/// `tests/smoke.rs::pool_install_empty_scope` covers the same corner on the
/// EXTERNAL arm (an `install` frame from the test thread, whose `joiner_wake`
/// is null), which is a different branch of `complete_task` and cannot stand in
/// for this one.
///
/// The second half — a wave AFTER the empty scopes, on the same pool — is what
/// separates "the empty join returned" from "the empty join returned and left
/// the pool usable".
#[test]
fn w_d_prime_an_empty_worker_scope_returns_with_no_completer_to_wake_it() {
    println!("[KE16 W] route (b), empty wave");

    let joined_on_worker = Arc::new(AtomicU32::new(0));
    let counters = fresh_counters(AFTER_EMPTY);

    let joined_driver = Arc::clone(&joined_on_worker);
    let counters_driver = Arc::clone(&counters);

    drive_with_deadline(
        "w_d_prime_an_empty_worker_scope_returns_with_no_completer_to_wake_it",
        move || {
            let pool = ThreadPoolBuilder::new()
                .num_threads(WORKERS as usize)
                .build();

            let outer_done = Arc::new(AtomicBool::new(false));
            let inner_pool = Arc::clone(&pool);
            let inner_done = Arc::clone(&outer_done);
            let inner_joined = Arc::clone(&joined_driver);
            let inner_counters = Arc::clone(&counters_driver);

            // `spawn`, not `scope`: the joiner must be a registered worker of
            // this pool, which is the only configuration in which the count
            // gate is armed at all.
            pool.spawn(move || {
                let on_worker = current_worker_id() < WORKERS;
                for _ in 0..EMPTY_SCOPES {
                    inner_pool.scope(|_s| {});
                    if on_worker {
                        inner_joined.fetch_add(1, Ordering::AcqRel);
                    }
                }
                inner_pool.scope(|s| {
                    for i in 0..AFTER_EMPTY {
                        let counters = Arc::clone(&inner_counters);
                        s.spawn(move || {
                            counters[i].fetch_add(1, Ordering::AcqRel);
                        });
                    }
                });
                inner_done.store(true, Ordering::Release);
            });

            while !outer_done.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        },
    );

    println!(
        "[KE16 W] route=b shape=empty empty_joins_on_a_registered_worker={}",
        joined_on_worker.load(Ordering::Acquire)
    );

    assert_eq!(
        joined_on_worker.load(Ordering::Acquire) as usize,
        EMPTY_SCOPES,
        "every empty scope must have been opened and joined by a registered worker of the pool; \
         a body that ran elsewhere makes this the external arm, whose wake target is null and \
         whose completion order `KE16-DESIGN-W.md` §3.5 leaves unchanged — the row would then be \
         green for a reason that has nothing to do with W-d′"
    );
    assert_each_ran_once(
        &counters,
        "w_d_prime_an_empty_worker_scope_returns_with_no_completer_to_wake_it",
    );
}
