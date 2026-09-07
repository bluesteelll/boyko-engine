//! KE16 App-8 — a system body may be entered while another system body is already
//! running on the same thread.
//!
//! ## What changed and why this file exists
//!
//! `InSystemRunGuard` used to be a `Cell<bool>` with `debug_assert!(!c.get(), "InSystemRunGuard
//! nested; SCH7 violation")` on entry: entering a second system body on one thread aborted the
//! process in every debug build. That was a claim about the pool, not about the scheduler — it
//! held only because a joining worker never ran anything but its own scope's tasks.
//!
//! It is not a claim the pool can keep. A worker blocked in `Scope::drop` drains the GLOBAL
//! injector (`boyko_threadpool`'s `scope.rs`, step 2 of `join_workers_until_drained`), which is
//! where `Schedule::run` puts every concurrent system task, so a worker that opened a `par_iter`
//! scope inside system A can pick up conflict-free system B and run it INLINE inside A's body.
//!
//! The path EXISTS at this checkout; it is not REACHED here, and the difference matters for what
//! this file can claim. The reasoning below is the DEFAULT build's — the a0 arm of KE16's A axis:
//! under `ke16-a1` / `ke16-a1-fifo` / `ke16-a3` the joiner's step 1 is compiled out entirely
//! (those arms never feed `injector_local`, so there is no own slot to drain) and a joiner meets a
//! worker-spawned wave only through the sibling sweep at step 3. The pool-side sites are cited by
//! SYMBOL and STEP NAME, not by line: the A-axis switches move that file, and a coordinate that
//! has drifted is worse than none.
//!
//! In the default build `join_workers_until_drained` returns as soon as its top-of-loop
//! `is_drained()` poll is true, and its step 1 drains the joiner's own `injector_local[wid]` in
//! <= 33-task batches that `drain_scratch` runs to completion, so the joiner reaches the global
//! injector at step 2 only in the window where its own local injector is empty while its scope
//! still has tasks pending. Under today's placement that window opens only if a sibling stole
//! from that local injector — which is defect A itself: nobody polls it. The KE16 joiner
//! candidates are what make helping, and therefore the nesting, the joiner's normal behaviour.
//!
//! KE16 axis B (deleted with the features): under `ke16-b1` / `ke16-b3` the joiner of a `par_iter`
//! scope opened on a worker reaches the global injector as its SECOND source on every pass of its
//! loop — it pops its own deque, then batch-steals the injector INTO that deque — so picking up a
//! co-dispatched sibling system is the joiner's ordinary behaviour there rather than a window that
//! opens only if somebody stole from it first. It is still not a CERTAINTY, and the difference
//! decides what the receipt below may assert: the joiner reaches that second source only once its
//! own chunks are exhausted, and at W=2 the idle sibling worker usually takes the other system's
//! task before then. MEASURED at this checkout under `ke16-a1,ke16-b1`, W=2, 8 runs:
//! `max_same_thread_system_depth=1`, i.e. the same reading as the default build. So under the B
//! arms the nesting stays a RECORDED number — printed by every pass — and the deterministic gate
//! remains the guard test.
//!
//! App-8 therefore turns the guard into a DEPTH COUNTER: nesting is legal, and
//! `is_in_system_run()` stays the boolean every consumer reads it as.
//!
//! Soundness is unchanged by the nesting, and it is worth being precise about why: both systems
//! are in the scheduler's `running` set (they were co-dispatched, so the conflict graph already
//! proved their accesses disjoint), and the apply-window gate counts COMPLETIONS
//! (`pending == running.count_ones()`), not lanes, so it does not care that two of them retired
//! on one thread. What DOES change is what an observer sees: a profiler sees two overlapping
//! `SystemSpan`s on one lane, and EVT1's per-lane single-writer rule is per THREAD, so the two
//! systems' events interleave inside the one lane (soundly — still one writer at any instant).
//!
//! ## What this file asserts, and what it does NOT
//!
//! Two live tests and one ignored receipt. They are separated because exactly one of them gates
//! App-8's behaviour change without depending on a race.
//!
//! 1. [`a_second_system_guard_inside_a_system_body_is_legal`] — the GATE on the behaviour change,
//!    and it is deterministic. It enters a second `InSystemRunGuard` inside a running system
//!    body, which is precisely what an inline sibling system produces, because `Schedule::run`
//!    brackets every system body in one (`schedule.rs:1299`). Under the old `Cell<bool>` guard
//!    that tripped `debug_assert!(!c.get(), "InSystemRunGuard nested; SCH7 violation")` and
//!    aborted the process in every debug build; under the depth counter it is legal, and
//!    `is_in_system_run()` stays true through the inner guard's drop because the predicate is
//!    depth-insensitive. Nothing in it waits on a joiner.
//! 2. [`conflict_free_systems_complete_with_nesting_legal`] — the two-system schedule. What it
//!    proves is COMPLETION and NON-LEAKAGE: both systems retire once per run, every row is
//!    visited, and the test thread does not read as inside a system afterwards. It does NOT prove
//!    that nesting occurred, and in the default build it does not occur — MEASURED here as
//!    `max_same_thread_system_depth=1` over 8 runs at W=2, for the structural reason in the
//!    paragraph above — so this test passes byte-identically under the OLD `Cell<bool>` guard.
//!    Read it as an anti-regression on the schedule, never as evidence for App-8.
//! 3. [`a_sibling_system_is_run_inline_inside_another_system_body`] — the receipt that the JOINER
//!    produced the nesting. `#[ignore]`d, because in the default build whether a joiner takes a
//!    sibling task is a race, and a flaky gate is worth less than no gate. Un-ignore it under the
//!    joiner candidate that makes helping the joiner's normal behaviour.
//!
//! ## Why the instruments are per-run
//!
//! libtest runs the tests of one binary CONCURRENTLY by default, and every test here builds its
//! own pool and runs its own schedule. Process-global counters would therefore be reset and read
//! by several passes at once, and the failure would read as a fixture defect ("the par_iter
//! system did not complete once per schedule run"). The counters are consequently owned by an
//! `Arc<Instruments>` / `Arc<NestingProbe>` created per pass and handed to the systems, so this
//! file needs no `--test-threads=1` and the un-ignore instruction below carries no flag. The one
//! thread-local ([`DEPTH`]) is per-thread by nature and each run owns its own pool's threads.
//!
//! Component id 494 is reserved for this test binary (492 `ke16_par_iter_in_system`,
//! 493 `ke16_occupancy_gate`).

use std::cell::Cell;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::thread::available_parallelism;
use std::time::{Duration, Instant};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_threadpool::{
    InSystemRunGuard, ThreadPool, ThreadPoolBuilder, is_in_system_run, ke16_check_expected_variant,
};

const SLOT_KE16_INLINE: ComponentId = ComponentId(494);

#[repr(C)]
#[derive(Clone, Copy)]
struct Ke16Inline(u32);

impl Component for Ke16Inline {
    fn component_id() -> ComponentId {
        SLOT_KE16_INLINE
    }
}

/// 4096 rows in ONE archetype. The default `BatchingStrategy` clamps chunk size UP to
/// `MIN_ARCHETYPE_FOR_PARALLEL` (1024), so this is four chunks — a real `par_iter` wave with a
/// join wide enough for the joining worker to reach the global injector while it is open.
const N_ROWS: usize = 4096;

/// Per-row busy-wait in the `par_iter` system. 4096 x 10 us is ~41 ms serial, ~10 ms across the
/// four chunks: wide enough that the sibling system is still pending when the wave's joiner looks
/// for work, short enough that eight repeats stay inside a normal test budget.
const SPIN_PER_ROW: Duration = Duration::from_micros(10);

/// Schedule runs per test. Whether a joiner takes the sibling task is a race, so the receipt is
/// read over repeats rather than over one run.
const REPEATS: usize = 8;

/// Workers. Two is the smallest pool in which the scheduler can co-dispatch the two systems, and
/// the smallest in which a joiner has a sibling to take — a wider pool only makes the inline case
/// rarer, because an idle worker takes the sibling first.
const WORKERS: usize = 2;

// ── Instruments ─────────────────────────────────────────────────────────────

thread_local! {
    /// System bodies currently entered ON THIS THREAD. This is the quantity App-8 legalises, and
    /// it is deliberately the test's OWN counter rather than a read of the pool's: what is under
    /// test is that the pool's guard permits the nesting, so an instrument that reused it would
    /// be measuring itself.
    ///
    /// Thread-local, so it is per-run without any further care: a run's system bodies execute on
    /// that run's own pool threads (or its own test thread).
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Consumes the spin's result so the loop survives optimisation. Static because no assertion ever
/// reads it: concurrent runs adding into it is harmless, which is not true of anything below.
static SINK: AtomicU64 = AtomicU64::new(0);

/// The counters one [`run_repeats`] pass asserts on, owned by that pass.
///
/// Shared with the pass's systems through an `Arc`, never through a `static`: both tests in this
/// binary run a pass, libtest runs them concurrently, and a shared counter would make each pass
/// read the other's completions.
struct Instruments {
    /// High-water mark of [`DEPTH`] over every thread of this run — the receipt.
    max_depth: AtomicU32,
    /// Completions of the `par_iter` system.
    ran_wide: AtomicUsize,
    /// Completions of the sibling system.
    ran_sibling: AtomicUsize,
    /// Rows the `par_iter` system visited, so a run that silently matched nothing cannot pass.
    rows_seen: AtomicUsize,
}

impl Instruments {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            max_depth: AtomicU32::new(0),
            ran_wide: AtomicUsize::new(0),
            ran_sibling: AtomicUsize::new(0),
            rows_seen: AtomicUsize::new(0),
        })
    }
}

/// Prints the build's variant witness, then refuses to produce a reading for a build that is not
/// the one the tester named.
///
/// The print and the `KE16_EXPECT` check are TWO obligations, not one (`KE16-DESIGN.md` §4): the
/// check is silent when the variable is unset, so without the banner a run of this file under a
/// joiner feature would leave no record of WHICH build it was. The spelling matches the pool
/// crate's KE16 harness files, so one `grep "KE16 variant"` collects every row of the protocol.
///
/// The banner is printed here rather than inside `ke16_check_expected_variant`, because a print
/// from `crates/*/src/**.rs` reds `boyko-log`'s print census.
fn ke16_witness() {
    println!("KE16 variant: {}", boyko_threadpool::ke16_variant());
    ke16_check_expected_variant();
}

/// What the deterministic gate observes inside one system body that enters a SECOND guard.
///
/// Owned by the pass through an `Arc`, for the reason [`Instruments`] is: libtest runs this
/// binary's tests concurrently and a `static` would let one pass read another's writes.
struct NestingProbe {
    /// `is_in_system_run()` as the scheduler's own guard leaves it, before the second `enter`.
    before_inner: AtomicBool,
    /// `is_in_system_run()` while both guards are held. Depth 2 must still read as "in a system".
    inside_inner: AtomicBool,
    /// `is_in_system_run()` after the inner guard has dropped and the outer one is still held.
    /// This is the reading a `Cell<bool>` guard could not produce: its drop path clears the flag,
    /// so the OUTER body would afterwards read as outside a system.
    after_inner: AtomicBool,
    /// System-body runs, so a pass in which the body never executed cannot report green.
    ///
    /// There is no depth field: `is_in_system_run()` is a boolean by design (every consumer reads
    /// it as one, `KE16-DESIGN-B.md` §2.5), so the pool's depth is not observable from here. The
    /// three booleans above are what the counter changes that the flag could not do.
    runs: AtomicUsize,
}

/// One pass's readings, taken after the repeats have finished.
struct Readings {
    depth: u32,
    wide: usize,
    sibling: usize,
    rows: usize,
}

/// RAII bracket around a system body that records this thread's nesting depth.
struct DepthProbe;

impl DepthProbe {
    fn enter(inst: &Instruments) -> Self {
        let depth = DEPTH.with(|d| {
            let next = d.get() + 1;
            d.set(next);
            next
        });
        inst.max_depth.fetch_max(depth, Ordering::AcqRel);
        Self
    }
}

impl Drop for DepthProbe {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get() - 1));
    }
}

/// The measured body: spin `SPIN_PER_ROW` on a dependent chain, so a serialised wave shows up as
/// wall-clock rather than as idle time.
#[inline(never)]
fn spin_row(v: &Ke16Inline, inst: &Instruments) {
    inst.rows_seen.fetch_add(1, Ordering::Relaxed);
    let deadline = Instant::now() + SPIN_PER_ROW;
    let mut acc = v.0 as u64;
    while Instant::now() < deadline {
        acc = black_box(acc.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1));
    }
    SINK.fetch_add(acc & 1, Ordering::Relaxed);
}

// ── Fixture ─────────────────────────────────────────────────────────────────

fn build_world() -> EcsMaster {
    register_layout::<Ke16Inline>(SLOT_KE16_INLINE.0);
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[SLOT_KE16_INLINE]);
    for i in 0..N_ROWS {
        world
            .spawn_one(arch, Ke16Inline(i as u32))
            .expect("invariant: inline-nesting fixture seed must succeed");
    }
    world
}

/// Two systems whose accesses are DISJOINT by the conflict graph's rule — both take the same
/// component by shared reference, and two shared reads never conflict — so the scheduler
/// co-dispatches them and either may be taken by the other's joiner.
///
/// The wide one opens a `par_iter` scope; the sibling is a short sequential pass, so it is the
/// one that is still pending when the wide system's joiner looks for work.
fn build_schedule(
    world: &mut EcsMaster,
    pool: &Arc<ThreadPool>,
    inst: &Arc<Instruments>,
) -> Schedule {
    let mut builder = ScheduleBuilder::new(Arc::clone(pool));
    let wide = Arc::clone(inst);
    builder.add_system(move |q: Query<&Ke16Inline>| {
        let _probe = DepthProbe::enter(&wide);
        assert!(is_in_system_run(), "a system body must observe is_in_system_run()");
        q.par_iter().for_each(|v| spin_row(v, &wide));
        wide.ran_wide.fetch_add(1, Ordering::AcqRel);
    });
    let sibling = Arc::clone(inst);
    builder.add_system(move |q: Query<&Ke16Inline>| {
        let _probe = DepthProbe::enter(&sibling);
        assert!(is_in_system_run(), "a system body must observe is_in_system_run()");
        let mut acc = 0u64;
        for v in q.iter().take(64) {
            acc = acc.wrapping_add(v.0 as u64);
        }
        SINK.fetch_add(black_box(acc) & 1, Ordering::Relaxed);
        sibling.ran_sibling.fetch_add(1, Ordering::AcqRel);
    });
    builder.build(world)
}

/// Runs the schedule `REPEATS` times over instruments owned by this call, and returns their
/// readings. Every counter it reports was written only by this pass, so two passes may run
/// concurrently (libtest's default) without either observing the other.
fn run_repeats() -> Readings {
    // Per PASS, not per file: this test binary is also run under the joiner features (the
    // un-ignore below), and a run whose features were not actually enabled must not produce a
    // reading. `KE16_EXPECT` unset leaves only the banner.
    ke16_witness();
    let workers = available_parallelism().map_or(WORKERS, |n| n.get().min(WORKERS)).max(1);
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let inst = Instruments::new();
    let mut world = build_world();
    let mut schedule = build_schedule(&mut world, &pool, &inst);

    for _ in 0..REPEATS {
        schedule.run(&mut world);
    }

    let readings = Readings {
        depth: inst.max_depth.load(Ordering::SeqCst),
        wide: inst.ran_wide.load(Ordering::SeqCst),
        sibling: inst.ran_sibling.load(Ordering::SeqCst),
        rows: inst.rows_seen.load(Ordering::SeqCst),
    };
    println!(
        "[ke16 app-8] workers={workers} repeats={REPEATS} \
         max_same_thread_system_depth={} wide_runs={} sibling_runs={} rows={}",
        readings.depth, readings.wide, readings.sibling, readings.rows
    );
    readings
}

/// Runs a ONE-system schedule whose body enters a second [`InSystemRunGuard`] — the guard shape
/// an inline sibling system produces — and returns what that body observed.
///
/// A one-system schedule, not the two-system one: this pass is deterministic on purpose, so it
/// must not depend on which worker takes what. The scheduler still dispatches the body to a
/// worker and still brackets it in its own guard (`schedule.rs:1299`), so the second `enter` here
/// is the same nesting a helping joiner produces, minus the race that produces it.
fn run_explicit_nesting() -> Arc<NestingProbe> {
    ke16_witness();
    let workers = available_parallelism().map_or(WORKERS, |n| n.get().min(WORKERS)).max(1);
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let probe = Arc::new(NestingProbe {
        before_inner: AtomicBool::new(false),
        inside_inner: AtomicBool::new(false),
        after_inner: AtomicBool::new(false),
        runs: AtomicUsize::new(0),
    });
    let mut world = build_world();

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let body = Arc::clone(&probe);
    builder.add_system(move || {
        body.before_inner.store(is_in_system_run(), Ordering::Release);
        {
            let _inner = InSystemRunGuard::enter();
            body.inside_inner.store(is_in_system_run(), Ordering::Release);
        }
        body.after_inner.store(is_in_system_run(), Ordering::Release);
        body.runs.fetch_add(1, Ordering::AcqRel);
    });
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    probe
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// The App-8 gate on the behaviour change, and the only one here that does not depend on a race.
///
/// Under the old `Cell<bool>` guard the `InSystemRunGuard::enter()` inside the system body tripped
/// `debug_assert!(!c.get(), "InSystemRunGuard nested; SCH7 violation")` and aborted the process in
/// every debug build, so in a debug run the fact that this returns IS an assertion. The three
/// booleans are the part a release build still gates, and they are what a flag could not deliver:
/// a `Cell<bool>` drop path clears the flag outright, so once the inner guard retired the OUTER
/// body would have read as outside a system, and every `debug_assert!(is_in_system_run())` in
/// `EventWriter` / `EventReader` reached after that point would have fired.
#[test]
#[cfg_attr(miri, ignore = "miri-slow: builds a real OS thread pool and runs a real schedule")]
fn a_second_system_guard_inside_a_system_body_is_legal() {
    let p = run_explicit_nesting();
    let runs = p.runs.load(Ordering::SeqCst);
    println!(
        "[ke16 app-8 nesting] runs={runs} before_inner={} inside_inner={} after_inner={}",
        p.before_inner.load(Ordering::SeqCst),
        p.inside_inner.load(Ordering::SeqCst),
        p.after_inner.load(Ordering::SeqCst),
    );

    assert_eq!(runs, 1, "the system body did not run: nothing was gated");
    assert!(
        p.before_inner.load(Ordering::SeqCst),
        "a system body must observe is_in_system_run() before any nesting"
    );
    assert!(
        p.inside_inner.load(Ordering::SeqCst),
        "with two guards held the thread must still read as inside a system body: the predicate \
         is depth-INSENSITIVE, and every consumer reads it as a boolean"
    );
    assert!(
        p.after_inner.load(Ordering::SeqCst),
        "after the inner guard dropped, the outer body no longer reads as inside a system: the \
         guard is clearing a flag instead of decrementing a depth, so an inline sibling system \
         would leave its host body outside the allocation-discipline window"
    );
    assert!(!is_in_system_run(), "the test thread reads as inside a system body: a guard leaked");
}

/// Anti-regression on the two-system schedule — NOT evidence for App-8.
///
/// It asserts what it can assert without a race: both conflict-free systems retire once per
/// `Schedule::run`, the `par_iter` system visits every row, at least one body was entered, and no
/// guard leaks onto the test thread. All four hold under the OLD `Cell<bool>` guard as well, so
/// this test cannot fail because of App-8 — in the default build the joiner does not reach the
/// global injector while its own scope is open (module header), and the observed depth is 1
/// (printed by [`run_repeats`], MEASURED as 1 over 8 runs at W=2).
///
/// The behaviour change is gated by [`a_second_system_guard_inside_a_system_body_is_legal`]
/// above; the joiner-driven receipt is the ignored test below.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: 4096 rows x 10 us of real wall-clock spin across a real OS thread pool, \
              eight times over"
)]
fn conflict_free_systems_complete_with_nesting_legal() {
    let r = run_repeats();

    assert_eq!(r.wide, REPEATS, "the par_iter system did not complete once per schedule run");
    assert_eq!(r.sibling, REPEATS, "the sibling system did not complete once per schedule run");
    assert_eq!(
        r.rows,
        N_ROWS * REPEATS,
        "the par_iter system visited the wrong number of rows; the fixture, not the pool, is \
         broken"
    );
    assert!(r.depth >= 1, "no system body was entered at all — the instrument is not wired");
    // Deliberately NOT `r.depth >= 2`: see this test's doc comment. The nesting receipt lives in
    // the ignored test below, and asserting it here would be a gate that fails on a race.
    assert!(
        !is_in_system_run(),
        "the test thread still reads as inside a system body: a guard leaked, and the depth \
         counter would drift up over a process lifetime"
    );
}

/// The App-8 RECEIPT — a sibling system actually ran INLINE inside another system's body.
///
/// This is what App-8 exists for, and it is separated from the gate above because it is a race at
/// this checkout: a joining worker reaches the global injector only on the path through
/// `Scope::drop` that does not return on its first `is_drained()`, so on most runs an idle worker
/// takes the sibling first and the depth stays 1. Asserting that in the default build would be a
/// flaky test, and a flaky gate is worth less than no gate.
///
/// KE16 axis B kept it ignored, and the reason is in the module header: `ke16-b1` / `ke16-b3` make
/// the joiner HELP, which is a necessary condition for the nesting and not a sufficient one — the
/// joiner reaches the global injector only after its own chunks are gone, and a free sibling
/// worker normally takes the other system first. The gate the design runs under those features is
/// this file WITHOUT `--ignored` (`KE16-DESIGN-MEASUREMENT.md` §6, "Additional gates by feature"),
/// i.e. the two live tests; this one stays a recorded reading, available on demand, until a
/// configuration is measured in which the depth is 2 deterministically.
#[test]
#[ignore = "deferred: KE16 App-8 inline-nesting receipt; whether a joiner takes a sibling system \
            is a race, and MAKING THE JOINER HELP IS NOT ENOUGH — measured depth 1 over 8 runs at \
            W=2 under `ke16-a1,ke16-b1` as well as in the default build, because the joiner reaches \
            the global injector only after its own chunks are gone; un-ignore only under a \
            configuration measured to nest deterministically"]
fn a_sibling_system_is_run_inline_inside_another_system_body() {
    let r = run_repeats();
    assert!(
        r.depth >= 2,
        "the deepest same-thread system nesting over {REPEATS} runs was {}: no joiner ever ran \
         the sibling system inline, so this configuration does not exercise App-8",
        r.depth
    );
}
