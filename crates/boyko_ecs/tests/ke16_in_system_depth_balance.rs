//! KE16 App-8 — the depth counter must come back to ZERO on every thread that entered a system
//! body, and it must be zero on the threads the NEGATIVE-polarity consumers run on.
//!
//! ## Why this file exists beside `ke16_nested_system_inline.rs`
//!
//! App-8 (`docs/threadpool/KE16-DESIGN-APP.md` §7) turns `InSystemRunGuard`'s `Cell<bool>` into a
//! `Cell<u32>` depth counter. `ke16_nested_system_inline.rs` gates the half of that change a
//! reader thinks of first: nesting is legal, and `is_in_system_run()` stays TRUE while two guards
//! are held. This file gates the other half, which a boolean flag could not get wrong and a
//! counter can: the counter must come back DOWN.
//!
//! A flag is idempotent — an unbalanced clear leaves `false`, which is the resting value. A
//! counter is not: one `enter` whose guard is forgotten, or one decrement that a `saturating_sub`
//! swallows, leaves a thread at `depth > 0` FOREVER. Nothing on that thread panics; the four
//! consumers that read the predicate in the NEGATIVE polarity simply stop firing:
//!
//! - `ecs_master.rs`'s `drain_deferred_hook_queue` — `debug_assert!(!is_in_system_run(),
//!   "SAFETY-7: hook drain must run with IN_SYSTEM_RUN == false")`,
//! - `time.rs`'s advance guard, `profiling/store.rs`'s and `profiling/fold.rs`'s
//!   outside-a-system asserts (the site list is `KE16-DESIGN-B.md` §2.5).
//!
//! That is the silent direction: a leaked depth does not break a run, it retires four tripwires.
//! And the leak would sit on a POOL WORKER, where `ke16_nested_system_inline.rs`'s
//! `assert!(!is_in_system_run())` — which runs on the test thread — cannot see it.
//!
//! ## What each test observes
//!
//! 1. [`every_probed_worker_leaves_the_system_depth_at_zero`] — the worker-side balance, probed
//!    on the pool's own threads AFTER a multi-system schedule has run on them. Non-vacuous by
//!    construction: it asserts that at least one probe landed on a registered worker and prints
//!    how many distinct workers answered.
//! 2. [`the_dispatcher_runs_its_safety_7_drains_at_depth_zero`] — the dispatcher-side balance,
//!    DRIVEN rather than assumed: the test raises and lowers the depth on the dispatcher thread
//!    itself, with the same `InSystemRunGuard` bracket the scheduler puts around a system body
//!    (`schedule.rs:1299`), and then runs the two SAFETY-7 consumers that a leak would disarm on
//!    that thread — the inline drain that follows an EXCLUSIVE system (`schedule.rs:1176`) and
//!    `delete_entity`'s own drain (`ecs_master.rs:676-679`).
//!
//! ## What the dispatcher does NOT do — traced, because an earlier revision asserted the opposite
//!
//! That revision said the dispatcher "runs system bodies itself whenever it helps its own scope,
//! so its depth is as leakable as a worker's", and rested a bare `assert!(!is_in_system_run())`
//! on it. The sentence is FALSE for `Schedule::run` and the assertion could not fail. The trace,
//! at this checkout:
//!
//! - the only `InSystemRunGuard::enter()` in the ECS is inside `scope.spawn`
//!   (`schedule.rs:1299`); the inline-exclusive loop (`schedule.rs:1085-1191`) calls
//!   `System::run_dispatcher` directly (`:1152`) with NO guard around it;
//! - the executor never helps: a round that dispatches nothing PARKS (`schedule.rs:695-706`), it
//!   does not steal;
//! - `executor_main_loop` returns only at `completed.count_ones(..) == n` (`schedule.rs:666-668`),
//!   and a `completed` bit is set only after that system's body has run and been drained — so
//!   when `pool.install`'s scope drops (`schedule.rs:433-435`) no un-run system task is left for
//!   `join_workers_until_drained` to find, and it returns on its first `is_drained()`. That
//!   function is cited by NAME, not by line: it lives in `boyko_threadpool/src/scope.rs` as two
//!   `#[cfg]`-selected bodies of one signature (the B0 arm and the B1/B3 arm), and the KE16
//!   tournament moves both. Under `ke16-b1`/`ke16-b3` the external arm likewise finds nothing to
//!   steal.
//!
//! So no *guarded body* runs on the dispatcher of a schedule run. The fact is worth recording
//! rather than merely avoiding: `KE16-DESIGN.md` §8's App-8 row reads inline sibling execution as
//! "already reachable today through the global-injector drain", and that is reachable on a WORKER
//! joiner — which `ke16_nested_system_inline.rs` gates — not on this thread.
//!
//! Test 2 therefore does not wait for a race to hand the dispatcher a guarded body. It enters and
//! drops the bracket itself, which is the whole of what the counter owes any thread: raise, then
//! return to zero. Its losable claims are listed on the test.
//!
//! Both tests are deterministic and neither depends on which worker takes what.
//!
//! Component id 495 is reserved for this test binary (492 `ke16_par_iter_in_system`,
//! 493 `ke16_occupancy_gate`, 494 `ke16_nested_system_inline`).

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::thread::available_parallelism;
use std::time::{Duration, Instant};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_threadpool::{
    InSystemRunGuard, MAX_WORKERS, ThreadPoolBuilder, WORKER_ID_DISPATCHER, WORKER_ID_UNATTACHED,
    current_worker_id, is_in_system_run, ke16_check_expected_variant,
};

const SLOT_KE16_DEPTH: ComponentId = ComponentId(495);

/// The worker receipt below is a 64-bit mask indexed by worker id, so a pool wider than 64 lanes
/// would shift out of range rather than under-report. The pool's own cap is what makes the mask
/// legal; if `MAX_WORKERS` ever grows past 64 this stops the build instead of the test.
const _: () = assert!(MAX_WORKERS <= 64, "the worker receipt mask holds one bit per worker id");

#[repr(C)]
#[derive(Clone, Copy)]
struct Ke16Depth(u32);

impl Component for Ke16Depth {
    fn component_id() -> ComponentId {
        SLOT_KE16_DEPTH
    }
}

/// Worker cap. The balance property is per THREAD, so a wide pool only buys more threads to
/// probe; 8 is enough to spread the systems over several lanes without making the probe wave
/// long on a 16-lane box.
const WORKERS: usize = 8;

/// Systems in the balance schedule. More systems than workers, so a worker enters (and must
/// leave) several bodies within one `Schedule::run` — an unbalanced pair inside ONE body would
/// otherwise be indistinguishable from a body that never ran.
const SYSTEMS: usize = 24;

/// Probe tasks per worker after the run. They are what carries the question "what is your depth?"
/// onto the pool's threads; more than one per worker so a lane that finishes early takes a second
/// one instead of leaving a worker unprobed.
const PROBES_PER_WORKER: usize = 8;

/// Per-system and per-probe busy-wait. Long enough that the scheduler spreads the wave over
/// several lanes, short enough that the whole file stays well inside a test budget.
const SPIN: Duration = Duration::from_micros(200);

/// The KE16 witness. Printed per TEST rather than per binary because the measurement protocol
/// runs these filtered, so a banner emitted once from a harness `main` would not appear on the
/// invocation whose reading is recorded (`KE16-DESIGN.md` §4).
fn ke16_witness() {
    println!("KE16 variant: {}", boyko_threadpool::ke16_variant());
    ke16_check_expected_variant();
}

fn spin_for(d: Duration) {
    let deadline = Instant::now() + d;
    let mut acc = 0u64;
    while Instant::now() < deadline {
        acc = black_box(acc.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1));
    }
    black_box(acc);
}

/// What the probe wave reports back. Every field is written by the probe tasks of ONE test pass
/// and read after the wave has joined, so two passes of this binary may run concurrently
/// (libtest's default) without either observing the other.
struct Probes {
    /// Probes that answered `is_in_system_run() == true` — i.e. found a leaked depth. Must be 0.
    inside: AtomicUsize,
    /// Probes that ran at all. Guards against a vacuous pass over an empty wave.
    total: AtomicUsize,
    /// One bit per registered worker id that answered. The receipt: a probe that only ever ran on
    /// the joining test thread would leave this zero and prove nothing about the workers.
    worker_mask: AtomicU64,
}

impl Probes {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inside: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
            worker_mask: AtomicU64::new(0),
        })
    }

    /// Runs on a pool thread: record the predicate, then keep the lane busy briefly so the rest
    /// of the wave spreads instead of being drained by one lane.
    fn probe(&self) {
        if is_in_system_run() {
            self.inside.fetch_add(1, Ordering::AcqRel);
        }
        let id = current_worker_id() as usize;
        // The dispatcher and unattached sentinels are `u32::MAX - 1` / `u32::MAX`, so this also
        // rejects the joining test thread's own observations from the worker receipt.
        if id < MAX_WORKERS {
            self.worker_mask.fetch_or(1u64 << id, Ordering::AcqRel);
        }
        self.total.fetch_add(1, Ordering::AcqRel);
        spin_for(SPIN);
    }
}

fn worker_count() -> usize {
    available_parallelism().map_or(WORKERS, |n| n.get().min(WORKERS)).max(2)
}

/// The worker-side balance: after a schedule whose bodies ran on the pool's threads, no thread of
/// that pool still reads as being inside a system body.
///
/// The probe wave is spawned from the TEST thread, i.e. through the dispatcher route, which is
/// healthy in every configuration of this campaign (`ke16_occupancy_gate.rs`'s control test) —
/// so this test measures the depth counter, never the placement defect KE16 is fixing. It holds
/// in the default build and under every candidate; a failure here is a leaked guard, nothing else.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds a real OS thread pool, runs a 24-system schedule and a probe wave \
              of real wall-clock spins"
)]
fn every_probed_worker_leaves_the_system_depth_at_zero() {
    ke16_witness();
    let workers = worker_count();
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();

    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let bodies_run = Arc::new(AtomicUsize::new(0));
    for _ in 0..SYSTEMS {
        let ran = Arc::clone(&bodies_run);
        // No world access: the systems are conflict-free by construction, so the scheduler is
        // free to co-dispatch them and a worker may retire several within one `run`.
        builder.add_system(move || {
            spin_for(SPIN);
            ran.fetch_add(1, Ordering::AcqRel);
        });
    }
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    assert_eq!(
        bodies_run.load(Ordering::SeqCst),
        SYSTEMS,
        "the schedule did not run every system body once: no guard was entered on some lane, so \
         the probe below would report balance over threads that never had a depth to leak"
    );

    // The probe wave. `install` makes the test thread the joiner, so some probes may run here;
    // those carry `WORKER_ID_DISPATCHER` and are counted in `total` but not in `worker_mask`,
    // which is exactly why the mask is asserted separately below.
    let probes = Probes::new();
    let probes_in = Arc::clone(&probes);
    pool.install(|scope| {
        for _ in 0..(workers * PROBES_PER_WORKER) {
            let p = Arc::clone(&probes_in);
            scope.spawn(move || p.probe());
        }
    });

    let inside = probes.inside.load(Ordering::SeqCst);
    let total = probes.total.load(Ordering::SeqCst);
    let mask = probes.worker_mask.load(Ordering::SeqCst);
    println!(
        "[ke16 app-8 balance] workers={workers} systems={SYSTEMS} probes={total} \
         distinct_worker_ids={} leaked={inside}",
        mask.count_ones()
    );

    assert_eq!(
        total,
        workers * PROBES_PER_WORKER,
        "the probe wave did not complete: the reading below would be taken over a partial wave"
    );
    assert!(
        mask.count_ones() >= 1,
        "no probe ran on a registered worker (every one was drained by the joining test thread), \
         so this pass says nothing about the pool's threads — the worker-side balance is exactly \
         what it exists to observe"
    );
    assert_eq!(
        inside, 0,
        "{inside} of {total} probes found their thread still inside a system body after the \
         schedule returned: an `InSystemRunGuard` depth leaked. Every `!is_in_system_run()` \
         tripwire on that thread (SAFETY-7's hook drain, the `Time` advance guard, the two \
         profiling asserts) is now permanently disarmed"
    );
}

/// What the dispatcher-side pass records. Every field is written on the dispatcher thread — the
/// exclusive system runs INLINE there (`schedule.rs:1085-1191`) — and read after `Schedule::run`
/// has returned, so the atomics carry the write across nothing; they are atomics because a
/// `System` body must be `Send + Sync`, not because two threads race for these fields.
struct DispatcherProbe {
    /// The worker id the exclusive body saw. The receipt that the INLINE-exclusive path ran it:
    /// it must be [`WORKER_ID_DISPATCHER`], never a registered worker id.
    exclusive_worker_id: AtomicU32,
    /// Whether the exclusive body read as inside a system body. The inline-exclusive path enters
    /// no `InSystemRunGuard`, so it must be `false`.
    exclusive_in_system: AtomicBool,
    /// How many times the exclusive body ran. Exactly once per `Schedule::run`.
    exclusive_runs: AtomicUsize,
}

impl DispatcherProbe {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            exclusive_worker_id: AtomicU32::new(WORKER_ID_UNATTACHED),
            exclusive_in_system: AtomicBool::new(false),
            exclusive_runs: AtomicUsize::new(0),
        })
    }
}

/// The dispatcher-side balance: the depth is RAISED and lowered on this thread by the same
/// bracket the scheduler puts around a system body, and then the thread's two SAFETY-7 consumers
/// are made to run.
///
/// The module header traces why the raise is driven here instead of waited for: under
/// `Schedule::run` no guarded body ever reaches the dispatcher, so a test that only asserted
/// `!is_in_system_run()` afterwards could not fail. What this test asserts, and what each
/// assertion loses to:
///
/// 1. `is_in_system_run()` is TRUE inside the bracket — lost by an `enter` that stops
///    incrementing (under the retired `Cell<bool>` this was the whole of the guard).
/// 2. It is FALSE again after the bracket — lost by a `drop` that stops decrementing, or by a
///    `saturating_sub` swallowing one. That is the App-8 failure this file exists for: it is
///    silent, and it disarms every negative-polarity tripwire on the thread for the rest of the
///    process.
/// 3. The exclusive system ran on the DISPATCHER lane, exactly once, and read depth 0 there —
///    lost if the executor stopped taking the inline path, or if a refactor ever bracketed that
///    path in an `InSystemRunGuard`. The second case aborts a DEBUG build one line later, at the
///    `drain_deferred_hook_queue` that follows the body inline (`schedule.rs:1176`, SAFETY-7);
///    in a RELEASE build, where that `debug_assert!` is compiled out, this receipt is the only
///    observer of it.
/// 4. `delete_entity` still succeeds — the second SAFETY-7 drain (`ecs_master.rs:676-679`), on
///    the direct-API path every caller uses after a frame.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds a real OS thread pool and runs a 25-system schedule"
)]
fn the_dispatcher_runs_its_safety_7_drains_at_depth_zero() {
    ke16_witness();
    register_layout::<Ke16Depth>(SLOT_KE16_DEPTH.0);

    assert!(!is_in_system_run(), "the test thread must start outside a system body");
    {
        let _bracket = InSystemRunGuard::enter();
        assert!(
            is_in_system_run(),
            "`InSystemRunGuard::enter` did not raise this thread's depth: the positive-polarity \
             consumers (`EventWriter`/`EventReader`'s `debug_assert!(is_in_system_run())`) would \
             fire inside every system body"
        );
    }
    assert!(
        !is_in_system_run(),
        "`InSystemRunGuard`'s drop did not lower this thread's depth back to zero: every \
         `!is_in_system_run()` tripwire on this thread (SAFETY-7's hook drain, the `Time` advance \
         guard, the two profiling asserts) is now permanently disarmed, silently"
    );

    let workers = worker_count();
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();

    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[SLOT_KE16_DEPTH]);
    let victim = world
        .spawn_one(arch, Ke16Depth(0))
        .expect("invariant: the {Ke16Depth} archetype accepts a Ke16Depth");

    let probe = DispatcherProbe::new();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    for _ in 0..SYSTEMS {
        builder.add_system(|| spin_for(SPIN));
    }
    // An EXCLUSIVE system: `ExclusiveFunctionSystem` declares `Access::universal()` at
    // construction, so the conflict graph forces `running == 0` and the executor runs the body
    // inline on this thread rather than spawning it. That is the one system body of a schedule
    // run that a non-worker thread executes, and the inline `drain_deferred_hook_queue`
    // immediately after it is the tripwire a leaked depth would have retired.
    let probe_in = Arc::clone(&probe);
    builder.add_system(move |_w: &mut EcsMaster| {
        probe_in.exclusive_worker_id.store(current_worker_id(), Ordering::Release);
        probe_in.exclusive_in_system.store(is_in_system_run(), Ordering::Release);
        probe_in.exclusive_runs.fetch_add(1, Ordering::AcqRel);
    });

    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let exclusive_runs = probe.exclusive_runs.load(Ordering::SeqCst);
    let exclusive_worker_id = probe.exclusive_worker_id.load(Ordering::SeqCst);
    println!(
        "[ke16 app-8 dispatcher] workers={workers} systems={} exclusive_runs={exclusive_runs} \
         exclusive_worker_id={exclusive_worker_id} (dispatcher sentinel = {WORKER_ID_DISPATCHER})",
        SYSTEMS + 1
    );

    assert_eq!(
        exclusive_runs, 1,
        "the exclusive system did not run exactly once, so the readings below were not taken over \
         the inline path this test exists to drive"
    );
    assert_eq!(
        exclusive_worker_id, WORKER_ID_DISPATCHER,
        "the exclusive system body did not run on the dispatcher lane: the executor no longer \
         takes the inline path, and the SAFETY-7 drain that follows it is no longer on this \
         thread"
    );
    assert!(
        !probe.exclusive_in_system.load(Ordering::SeqCst),
        "the inline-exclusive body read as INSIDE a system body: something now brackets that path \
         in an `InSystemRunGuard`, and the `drain_deferred_hook_queue` one line later \
         (`schedule.rs:1176`) violates SAFETY-7 — it aborts a debug build and is silent in release"
    );
    assert!(
        !is_in_system_run(),
        "the dispatcher thread reads as inside a system body after `Schedule::run` returned"
    );
    assert!(
        world.delete_entity(victim),
        "the post-frame delete failed: the fixture, not the depth counter, is broken"
    );
}
