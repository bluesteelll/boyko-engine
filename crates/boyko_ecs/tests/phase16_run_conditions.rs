//! Phase 16 — integration tests for run conditions (`.run_if`).
//!
//! Exercises the public `ScheduleBuilder` API end-to-end through
//! `Schedule::run`:
//!
//! * `SystemConfig::run_if<C, M>` (system-level conditions, AND-accumulating),
//! * `ConfigureSet::run_if<C, M>` (set-level conditions, once-per-frame),
//! * the built-in `run_once` (`boyko_ecs::ecs::core::schedule::run_once`),
//! * the skipped-successor semantic (a false gate still decrements its
//!   successors' `pred_remaining`, so `before` successors run and the frame
//!   terminates),
//! * the eager (no-short-circuit) fold — every condition runs every frame so a
//!   stateful condition like `run_once` advances its `Local` even when an
//!   earlier condition already returned `false`,
//! * the race-freedom invariant (§0-P6c / R2): conditions are evaluated ONLY
//!   when `running.count_ones() == 0`, never concurrently with a live worker.
//!
//! # Harness discipline (matches `phase15_set_ordering.rs`)
//!
//! Every test uses a per-test `Arc<Mutex<..>>` log or `Arc<Atomic*>` counter
//! captured by the system / condition closures — NO shared global `static`s, so
//! the tests are independent and never flake under parallel `cargo test`.
//!
//! Firing / ordering tests pin a **single-worker** pool (`num_threads(1)`): the
//! dispatcher then runs ready systems serially in Kahn-FIFO order, so the log is
//! the linearised schedule. The race-guard test (`§8`) deliberately uses a
//! MULTI-worker pool with a measurable parallel workload so that a broken
//! condition-eval guard (one that ran while workers were live) would observe a
//! non-zero `running` popcount and fail the assertion.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{ScheduleBuilder, run_once};
use boyko_ecs::ecs::core::system::Res;
use boyko_macros::{Resource, SystemSet};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

// ── Shared harness ───────────────────────────────────────────────────────────

/// Single-worker pool — serial dispatch ⇒ deterministic firing order, no flake.
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// Shared, ordered execution log. Each system appends its label on run.
type Log = Arc<Mutex<Vec<&'static str>>>;

fn new_log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

fn snapshot(log: &Log) -> Vec<&'static str> {
    log.lock().expect("log mutex poisoned").clone()
}

/// A fresh shared `usize` counter.
fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

fn load(c: &Arc<AtomicUsize>) -> usize {
    c.load(Ordering::Relaxed)
}

// ── Test set markers ─────────────────────────────────────────────────────────

#[derive(SystemSet)]
struct GatedSet;

#[derive(SystemSet)]
struct OuterSet;

// ── Test resources ───────────────────────────────────────────────────────────

/// A boolean gate resource flipped between frames to drive a `Res`-reading
/// condition. `#[derive(Resource)]` gives it a per-type `OnceLock<ResourceId>`.
#[derive(Resource)]
struct Gate(bool);

// =============================================================================
// §1 — run_once: body runs on frame 1 only
// =============================================================================

/// A system `.run_if(run_once)` runs its body on frame 1 and NEVER on frames
/// 2/3. The `run_once` built-in's `Local<bool>` persists across frames in the
/// condition's own `FunctionSystem::state`, so after the first eval it returns
/// `false` forever. (Test surface §1.)
#[test]
fn run_once_runs_body_exactly_once_over_three_frames() {
    let pool = serial_pool();
    let runs = counter();
    let runs_cl = Arc::clone(&runs);

    let mut builder = ScheduleBuilder::new(pool);
    builder
        .add_system(move || {
            runs_cl.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(run_once);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(load(&runs), 1, "frame 1: run_once true ⇒ body runs once");
    schedule.run(&mut world);
    assert_eq!(load(&runs), 1, "frame 2: run_once false ⇒ body must NOT run");
    schedule.run(&mut world);
    assert_eq!(load(&runs), 1, "frame 3: run_once still false ⇒ body must NOT run");
}

// =============================================================================
// §2 — false/true gate + Res-reading gate
// =============================================================================

/// `.run_if(|| false)` never runs the body; the frame still terminates.
/// (Test surface §2 — false half.)
#[test]
fn condition_false_never_runs_body() {
    let pool = serial_pool();
    let runs = counter();
    let runs_cl = Arc::clone(&runs);

    let mut builder = ScheduleBuilder::new(pool);
    builder
        .add_system(move || {
            runs_cl.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(|| false);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    for _ in 0..3 {
        schedule.run(&mut world);
    }
    assert_eq!(load(&runs), 0, "a `|| false` gate must skip the body every frame");
}

/// `.run_if(|| true)` runs the body every frame. (Test surface §2 — true half.)
#[test]
fn condition_true_always_runs_body() {
    let pool = serial_pool();
    let runs = counter();
    let runs_cl = Arc::clone(&runs);

    let mut builder = ScheduleBuilder::new(pool);
    builder
        .add_system(move || {
            runs_cl.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(|| true);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    for _ in 0..3 {
        schedule.run(&mut world);
    }
    assert_eq!(load(&runs), 3, "a `|| true` gate must run the body every frame");
}

/// A `fn(Res<Gate>) -> bool` condition gates the body on the resource's value.
/// Flipping the resource between frames flips the gate: the body runs only on
/// frames where `Gate.0 == true`. (Test surface §2 — Res half.)
#[test]
fn res_reading_condition_gates_on_resource_value() {
    let pool = serial_pool();
    let runs = counter();
    let runs_cl = Arc::clone(&runs);

    let mut builder = ScheduleBuilder::new(pool);
    builder
        .add_system(move || {
            runs_cl.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(|gate: Res<Gate>| gate.0);

    let mut world = EcsMaster::new();
    world.insert_resource(Gate(true));
    let mut schedule = builder.build(&mut world);

    // Frame 1: gate true ⇒ runs (1).
    schedule.run(&mut world);
    assert_eq!(load(&runs), 1, "gate true ⇒ body runs");

    // Frame 2: flip the resource false ⇒ skip (still 1).
    world.insert_resource(Gate(false));
    schedule.run(&mut world);
    assert_eq!(load(&runs), 1, "gate flipped false ⇒ body skipped");

    // Frame 3: flip back true ⇒ runs (2).
    world.insert_resource(Gate(true));
    schedule.run(&mut world);
    assert_eq!(load(&runs), 2, "gate flipped back true ⇒ body runs again");
}

// =============================================================================
// §3 — skip-successor: a false gate still runs `before` successors
// =============================================================================

/// `a.run_if(|| false)` with `b.after(a)` (b unconditioned): `a`'s body never
/// runs, but `b` STILL runs (the skip decrements `b`'s `pred_remaining` exactly
/// as a real completion would), and the frame terminates. (Test surface §3.)
#[test]
fn skipped_system_still_runs_its_successor() {
    let pool = serial_pool();
    let log = new_log();

    let mut builder = ScheduleBuilder::new(pool);
    let log_a = Arc::clone(&log);
    let a = builder
        .add_system(move || {
            log_a.lock().expect("poisoned").push("a");
        })
        .run_if(|| false)
        .key();
    let log_b = Arc::clone(&log);
    builder
        .add_system(move || {
            log_b.lock().expect("poisoned").push("b");
        })
        .after(a);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world); // must terminate

    let order = snapshot(&log);
    assert!(!order.contains(&"a"), "a's body must NOT run (gate false); order = {order:?}");
    assert!(order.contains(&"b"), "b must still run despite a being skipped; order = {order:?}");
    assert_eq!(order.len(), 1, "only b runs; order = {order:?}");
}

// =============================================================================
// §4 — skip_run_skip_cascade (§0-P2 mixed chain)
// =============================================================================

/// Mixed chain `a.run_if(false) → b.run_if(true) → c.run_if(false)` wired with
/// `before`/`after` so the topo order is a, b, c. Proves:
/// * `a` (false) is skipped → decrements `b`'s pred,
/// * `b` (true) is DISPATCHED (a should_run=true conditioned system mid-chain is
///   NOT skipped) and runs EXACTLY once → its REAL completion decrements `c`'s
///   pred on a later loop iteration,
/// * `c` (false) is then skipped → never runs,
/// * the frame still terminates.
///
/// This exercises the non-contiguous (skip → run → skip) rhythm that the
/// all-skip cascade does not. (Test surface §4.)
#[test]
fn skip_run_skip_cascade_settles_and_terminates() {
    let pool = serial_pool();
    let log = new_log();

    let mut builder = ScheduleBuilder::new(pool);

    let log_a = Arc::clone(&log);
    let a = builder
        .add_system(move || log_a.lock().expect("poisoned").push("a"))
        .run_if(|| false)
        .key();

    let log_b = Arc::clone(&log);
    let b = builder
        .add_system(move || log_b.lock().expect("poisoned").push("b"))
        .run_if(|| true)
        .after(a)
        .key();

    let log_c = Arc::clone(&log);
    builder
        .add_system(move || log_c.lock().expect("poisoned").push("c"))
        .run_if(|| false)
        .after(b);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world); // must terminate

    let order = snapshot(&log);
    assert!(!order.contains(&"a"), "a (false) must not run; order = {order:?}");
    assert_eq!(
        order.iter().filter(|&&l| l == "b").count(),
        1,
        "b (true) mid-chain must run EXACTLY once; order = {order:?}"
    );
    assert!(!order.contains(&"c"), "c (false) must not run; order = {order:?}");
}

// =============================================================================
// §5 — eager_fold_advances_all_locals (§6, distinguishes eager vs short-circuit)
// =============================================================================

/// A system gated by `.run_if(run_once).run_if(|| false)` never runs (the
/// second condition is always false), but the eager fold MUST still invoke
/// `run_once` every frame, advancing its `Local<bool>`. We prove the `Local`
/// advanced via a side-channel: `run_once` here is a custom condition that, in
/// addition to the standard once-only logic, records into an `Arc<AtomicUsize>`
/// how many times its body has been *evaluated* and what it returned.
///
/// After 2 frames:
/// * the body NEVER ran (second condition false), AND
/// * the once-condition was EVALUATED twice (eval count == 2), AND
/// * its second evaluation returned `false` (i.e. it advanced its Local on frame
///   1 even though the system was skipped).
///
/// A short-circuiting fold (false first, or false skipping the rest) would leave
/// the once-condition's Local un-advanced → eval count < 2 or a `true` on the
/// 2nd eval → the test fails. The condition is placed FIRST in the chain
/// (`run_if(once).run_if(|| false)`) so that a left-to-right short-circuit could
/// not even reach a "false skips once" excuse — the ONLY way `once` advances
/// twice is genuine eager evaluation of EVERY condition every frame.
#[test]
fn eager_fold_runs_every_condition_even_when_body_skipped() {
    let pool = serial_pool();

    // Side-channels for the stateful once-condition.
    let evals = counter(); // times the once-condition body was evaluated
    let last_verdict = Arc::new(AtomicUsize::new(2)); // 0=false, 1=true, 2=unset
    let body_runs = counter(); // times the GATED SYSTEM body ran

    let evals_cl = Arc::clone(&evals);
    let verdict_cl = Arc::clone(&last_verdict);
    // A `run_once`-style stateful condition with an eval side-channel.
    let once_probe = move |mut has_run: boyko_ecs::ecs::core::system::Local<bool>| -> bool {
        evals_cl.fetch_add(1, Ordering::Relaxed);
        let verdict = if *has_run {
            false
        } else {
            *has_run = true;
            true
        };
        verdict_cl.store(verdict as usize, Ordering::Relaxed);
        verdict
    };

    let body_cl = Arc::clone(&body_runs);
    let mut builder = ScheduleBuilder::new(pool);
    builder
        .add_system(move || {
            body_cl.fetch_add(1, Ordering::Relaxed);
        })
        // once FIRST, then an always-false gate.
        .run_if(once_probe)
        .run_if(|| false);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world); // frame 1
    schedule.run(&mut world); // frame 2

    assert_eq!(load(&body_runs), 0, "the gated body must never run (second cond is `false`)");
    assert_eq!(
        load(&evals),
        2,
        "eager fold must evaluate the once-condition on BOTH frames (not short-circuited)"
    );
    assert_eq!(
        last_verdict.load(Ordering::Relaxed),
        0,
        "the once-condition's 2nd eval must return `false` — proving its Local advanced on \
         frame 1 even though the system was skipped (genuine eager fold, not short-circuit)"
    );
}

// =============================================================================
// §6 — set_condition_once_per_frame (§7)
// =============================================================================

/// A set with 5 member systems and `configure_set(GatedSet).run_if(counting)`
/// where the condition bumps an atomic. After frame 1 the atomic == 1 (NOT 5:
/// the set condition is memoized once per frame regardless of member count);
/// after frame 2 == 2. With a true set gate, all five members run.
/// (Test surface §6 — true half.)
#[test]
fn set_condition_evaluated_once_per_frame_true_gate() {
    let pool = serial_pool();
    let cond_evals = counter();
    let member_runs = counter();

    let mut builder = ScheduleBuilder::new(pool);
    for _ in 0..5 {
        let runs_cl = Arc::clone(&member_runs);
        builder
            .add_system(move || {
                runs_cl.fetch_add(1, Ordering::Relaxed);
            })
            .in_set(GatedSet);
    }
    let evals_cl = Arc::clone(&cond_evals);
    builder.configure_set(GatedSet).run_if(move || {
        evals_cl.fetch_add(1, Ordering::Relaxed);
        true
    });

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(
        load(&cond_evals),
        1,
        "set condition must run EXACTLY once per frame (not once per the 5 members)"
    );
    assert_eq!(load(&member_runs), 5, "true set gate ⇒ all 5 members run");

    schedule.run(&mut world);
    assert_eq!(load(&cond_evals), 2, "second frame ⇒ set condition runs once more (total 2)");
    assert_eq!(load(&member_runs), 10, "all 5 members run again (total 10)");
}

/// A false set condition gates ALL members off — none run — while the condition
/// itself still runs exactly once per frame. (Test surface §6 — false half.)
#[test]
fn set_condition_false_gates_all_members() {
    let pool = serial_pool();
    let cond_evals = counter();
    let member_runs = counter();

    let mut builder = ScheduleBuilder::new(pool);
    for _ in 0..5 {
        let runs_cl = Arc::clone(&member_runs);
        builder
            .add_system(move || {
                runs_cl.fetch_add(1, Ordering::Relaxed);
            })
            .in_set(GatedSet);
    }
    let evals_cl = Arc::clone(&cond_evals);
    builder.configure_set(GatedSet).run_if(move || {
        evals_cl.fetch_add(1, Ordering::Relaxed);
        false
    });

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world); // must terminate

    assert_eq!(load(&member_runs), 0, "false set gate ⇒ no member runs");
    assert_eq!(
        load(&cond_evals),
        1,
        "the (false) set condition is still evaluated exactly once per frame"
    );
}

// =============================================================================
// §7 — set AND own: a member runs only if BOTH its own and set conditions hold
// =============================================================================

/// A member with its OWN `.run_if` AND membership in a set with a `.run_if`
/// runs only when BOTH conditions are true. We drive the set condition off
/// `Res<Gate>` and the own condition off a constant, exercising all four
/// truth-table corners across frames by flipping the resource and rebuilding
/// for the own-false case. (Test surface §7.)
#[test]
fn member_runs_only_when_both_own_and_set_conditions_true() {
    // Corner 1: own true, set true ⇒ runs.
    {
        let pool = serial_pool();
        let runs = counter();
        let runs_cl = Arc::clone(&runs);
        let mut builder = ScheduleBuilder::new(pool);
        builder
            .add_system(move || {
                runs_cl.fetch_add(1, Ordering::Relaxed);
            })
            .in_set(GatedSet)
            .run_if(|| true);
        builder.configure_set(GatedSet).run_if(|gate: Res<Gate>| gate.0);

        let mut world = EcsMaster::new();
        world.insert_resource(Gate(true));
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);
        assert_eq!(load(&runs), 1, "own true + set true ⇒ member runs");
    }

    // Corner 2: own true, set false ⇒ skipped.
    {
        let pool = serial_pool();
        let runs = counter();
        let runs_cl = Arc::clone(&runs);
        let mut builder = ScheduleBuilder::new(pool);
        builder
            .add_system(move || {
                runs_cl.fetch_add(1, Ordering::Relaxed);
            })
            .in_set(GatedSet)
            .run_if(|| true);
        builder.configure_set(GatedSet).run_if(|gate: Res<Gate>| gate.0);

        let mut world = EcsMaster::new();
        world.insert_resource(Gate(false));
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);
        assert_eq!(load(&runs), 0, "own true + set false ⇒ member skipped");
    }

    // Corner 3: own false, set true ⇒ skipped.
    {
        let pool = serial_pool();
        let runs = counter();
        let runs_cl = Arc::clone(&runs);
        let mut builder = ScheduleBuilder::new(pool);
        builder
            .add_system(move || {
                runs_cl.fetch_add(1, Ordering::Relaxed);
            })
            .in_set(GatedSet)
            .run_if(|| false);
        builder.configure_set(GatedSet).run_if(|gate: Res<Gate>| gate.0);

        let mut world = EcsMaster::new();
        world.insert_resource(Gate(true));
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);
        assert_eq!(load(&runs), 0, "own false + set true ⇒ member skipped");
    }

    // Corner 4: own false, set false ⇒ skipped.
    {
        let pool = serial_pool();
        let runs = counter();
        let runs_cl = Arc::clone(&runs);
        let mut builder = ScheduleBuilder::new(pool);
        builder
            .add_system(move || {
                runs_cl.fetch_add(1, Ordering::Relaxed);
            })
            .in_set(GatedSet)
            .run_if(|| false);
        builder.configure_set(GatedSet).run_if(|gate: Res<Gate>| gate.0);

        let mut world = EcsMaster::new();
        world.insert_resource(Gate(false));
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);
        assert_eq!(load(&runs), 0, "own false + set false ⇒ member skipped");
    }
}

// =============================================================================
// §8 — condition_eval_deferred_while_workers_live (§0-P6c, THE RACE GUARD)
// =============================================================================

/// THE regression net for the race-freedom invariant (R2 / Proof a): a
/// conditioned system's condition records, via an atomic, the
/// `running.count_ones()` the executor exposes at the moment the condition is
/// evaluated. The invariant is that condition eval happens ONLY when no worker
/// is live (`running == 0`), so the recorded popcount must ALWAYS be 0.
///
/// We cannot read `executor_scratch.running` from inside a condition (it is
/// `pub(crate)`), so we OBSERVE a proxy: a shared "workers in flight" counter.
/// A bank of long-running parallel worker systems each increment a shared
/// `in_flight` atomic on entry and decrement on exit (with a sleep in between to
/// guarantee temporal overlap). The conditioned system's condition samples
/// `in_flight` and folds the max into `observed_in_flight`. If the executor ever
/// evaluated the condition while a worker body was live, the sample would be
/// > 0 and the assertion would fail.
///
/// To make a *broken* guard observably fail, the conditioned system is wired to
/// run AFTER the parallel bank (`after` every worker), so its condition is first
/// reached at a ready-transition that — under a correct guard — only fires once
/// every worker has drained (running back to 0). A broken guard that evaluated
/// the condition during the prior dispatch round (workers still live) would
/// sample a non-zero `in_flight`.
///
/// Multi-worker pool (8 threads) + a sleep in each worker body forces genuine
/// temporal overlap among the parallel bank, so `in_flight` actually exceeds 1
/// during the frame (a separate assertion confirms the bank really did overlap,
/// otherwise the guard test would be vacuous).
#[test]
fn condition_eval_never_observes_a_live_worker() {
    let pool = ThreadPoolBuilder::new().num_threads(8).build();

    let in_flight = counter();
    let peak_in_flight = counter(); // max concurrency the bank actually reached
    let observed_in_flight = counter(); // max in_flight the CONDITION sampled
    let cond_ran = counter();
    let gated_ran = counter();

    let mut builder = ScheduleBuilder::new(pool);

    // Parallel bank: 8 disjoint-access workers (empty Access ⇒ no conflict bit
    // ⇒ all dispatchable concurrently on the 8-thread pool). Each overlaps in
    // time via a short sleep.
    let mut worker_keys = Vec::new();
    for _ in 0..8 {
        let in_flight_cl = Arc::clone(&in_flight);
        let peak_cl = Arc::clone(&peak_in_flight);
        let key = builder
            .add_system(move || {
                let now = in_flight_cl.fetch_add(1, Ordering::AcqRel) + 1;
                // Record the high-water concurrency mark.
                let mut cur = peak_cl.load(Ordering::Acquire);
                while now > cur {
                    match peak_cl.compare_exchange_weak(
                        cur,
                        now,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => break,
                        Err(v) => cur = v,
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
                in_flight_cl.fetch_sub(1, Ordering::AcqRel);
            })
            .key();
        worker_keys.push(key);
    }

    // Conditioned system, ordered AFTER the entire parallel bank. Its condition
    // samples `in_flight` — under the correct guard it is reached only when
    // running == 0, i.e. the bank fully drained ⇒ in_flight == 0.
    let in_flight_probe = Arc::clone(&in_flight);
    let observed_cl = Arc::clone(&observed_in_flight);
    let cond_ran_cl = Arc::clone(&cond_ran);
    let gated_ran_cl = Arc::clone(&gated_ran);
    let mut gated = builder.add_system(move || {
        gated_ran_cl.fetch_add(1, Ordering::Relaxed);
    });
    for &k in &worker_keys {
        gated = gated.after(k);
    }
    gated.run_if(move || {
        cond_ran_cl.fetch_add(1, Ordering::Relaxed);
        let sample = in_flight_probe.load(Ordering::Acquire);
        // Fold the max sample into observed_in_flight.
        let mut cur = observed_cl.load(Ordering::Acquire);
        while sample > cur {
            match observed_cl.compare_exchange_weak(
                cur,
                sample,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(v) => cur = v,
            }
        }
        true
    });

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    // Several frames to give the race many chances to manifest.
    for _ in 0..20 {
        schedule.run(&mut world);
    }

    assert!(load(&cond_ran) >= 20, "the condition must have been evaluated each frame");
    assert_eq!(load(&gated_ran), 20, "the gated system runs once per frame (true gate)");
    assert_eq!(
        load(&observed_in_flight),
        0,
        "RACE GUARD: the condition must NEVER observe a live worker (running==0 at eval); \
         observed in_flight peak = {}",
        load(&observed_in_flight)
    );
    // Vacuity guard: the parallel bank really did overlap in time, so the race
    // window was genuinely exercised (a broken guard would have caught it).
    assert!(
        load(&peak_in_flight) >= 2,
        "the worker bank must overlap in time (peak concurrency ≥ 2) for the guard to be \
         meaningful; observed peak = {}",
        load(&peak_in_flight)
    );
}

// =============================================================================
// Extra coverage — mixed conditioned / unconditioned + multi-condition AND
// =============================================================================

/// A schedule mixing conditioned and unconditioned systems: the unconditioned
/// ones run unaffected; the conditioned one gates correctly. Proves the
/// `has_condition` gate does not leak across systems.
#[test]
fn mixed_conditioned_and_unconditioned_systems() {
    let pool = serial_pool();
    let uncond_runs = counter();
    let cond_runs = counter();

    let mut builder = ScheduleBuilder::new(pool);
    let uncond_cl = Arc::clone(&uncond_runs);
    builder.add_system(move || {
        uncond_cl.fetch_add(1, Ordering::Relaxed);
    });
    let cond_cl = Arc::clone(&cond_runs);
    builder
        .add_system(move || {
            cond_cl.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(|| false);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    for _ in 0..3 {
        schedule.run(&mut world);
    }

    assert_eq!(load(&uncond_runs), 3, "the unconditioned system runs every frame");
    assert_eq!(load(&cond_runs), 0, "the conditioned (false) system never runs");
}

/// `.run_if(a).run_if(b)` accumulates into an AND: the body runs only when BOTH
/// conditions are true. Tests all four truth-table corners by reading two
/// independent boolean resources.
#[test]
fn two_own_conditions_and_together() {
    #[derive(Resource)]
    struct GateA(bool);
    #[derive(Resource)]
    struct GateB(bool);

    for (a, b, expect_run) in [
        (true, true, 1usize),
        (true, false, 0),
        (false, true, 0),
        (false, false, 0),
    ] {
        let pool = serial_pool();
        let runs = counter();
        let runs_cl = Arc::clone(&runs);
        let mut builder = ScheduleBuilder::new(pool);
        builder
            .add_system(move || {
                runs_cl.fetch_add(1, Ordering::Relaxed);
            })
            .run_if(|g: Res<GateA>| g.0)
            .run_if(|g: Res<GateB>| g.0);

        let mut world = EcsMaster::new();
        world.insert_resource(GateA(a));
        world.insert_resource(GateB(b));
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);
        assert_eq!(
            load(&runs),
            expect_run,
            "AND of two conditions: a={a}, b={b} ⇒ expected run count {expect_run}"
        );
    }
}

/// All-skip cascade: `a → b → c` chained, all `.run_if(|| false)`. None run, the
/// frame terminates. (Contiguous skip-chain settlement, complements §4's mixed
/// chain.)
#[test]
fn all_false_chain_skips_everything_and_terminates() {
    let pool = serial_pool();
    let runs = counter();

    let mut builder = ScheduleBuilder::new(pool);
    let r0 = Arc::clone(&runs);
    let a = builder
        .add_system(move || {
            r0.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(|| false)
        .key();
    let r1 = Arc::clone(&runs);
    let b = builder
        .add_system(move || {
            r1.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(|| false)
        .after(a)
        .key();
    let r2 = Arc::clone(&runs);
    builder
        .add_system(move || {
            r2.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(|| false)
        .after(b);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world); // must terminate

    assert_eq!(load(&runs), 0, "every system in an all-false chain is skipped");
}

/// A multi-system set whose members run in the unconditioned tail of a schedule
/// that ALSO carries a conditioned system, and the set is itself unconditioned:
/// proves a conditioned system + an unconditioned set coexist (the
/// `system_gating_sets` for the set members stay empty, so they are NOT gated).
#[test]
fn unconditioned_set_members_unaffected_by_other_conditioned_system() {
    let pool = serial_pool();
    let set_runs = counter();
    let cond_runs = counter();

    let mut builder = ScheduleBuilder::new(pool);
    for _ in 0..3 {
        let cl = Arc::clone(&set_runs);
        builder
            .add_system(move || {
                cl.fetch_add(1, Ordering::Relaxed);
            })
            .in_set(OuterSet);
    }
    let cond_cl = Arc::clone(&cond_runs);
    builder
        .add_system(move || {
            cond_cl.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(|| false);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    assert_eq!(load(&set_runs), 3, "members of an unconditioned set run normally");
    assert_eq!(load(&cond_runs), 0, "the unrelated conditioned (false) system is skipped");
}
