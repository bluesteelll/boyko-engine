//! Zero-per-frame-allocation test for the layout pair (Principle 5), via a
//! counting global allocator and BASELINE SUBTRACTION.
//!
//! # Why baseline subtraction (and not an absolute "0")
//!
//! The layout pair is driven the way a host drives it: through a parallel
//! [`Schedule`]. `Schedule::run`'s parallel executor allocates a small FIXED
//! number of objects per frame for its own dispatch (measured 2026-10-11: two, the
//! install's 256-byte `ScopeShared` and its 4 KiB task chunk, both on the driver),
//! entirely independent of the systems' bodies. That overhead is the executor's,
//! and it cannot be driven to zero from inside `boyko_ui`. (Driving the pair via
//! `run_cached_system` to bypass the executor is NOT a valid alternative: that
//! path does not advance the per-system change-detection tick window the way
//! `Schedule::run` does, so discovery never observes a change and apply never runs
//! — the layout work would be silently skipped, making any "0 allocs" vacuous.)
//!
//! So the test measures the layout pair's OWN per-frame allocations as the DELTA
//! between two schedules with identical system SHAPE (one normal/parallel system
//! + one exclusive system) over the same warmed UI tree:
//!   * baseline = `[noop_normal, noop_exclusive]`
//!   * pair     = `[ui_layout_discovery, ui_layout_apply]`
//!
//! A DELTA of 0 proves the layout pair allocates nothing of its own per frame. A
//! window counts the threads that run the schedule it measures (the driver and that
//! schedule's pool workers) and no other, and each of them has made its first touch
//! before any window opens: see "What a window counts", below the tests. The cases
//! asserted (each at DELTA 0): (a) unchanged/steady-state frame,
//! (b) a non-structural size tweak within high-water, (c) a resize frame at the
//! same root count (cached root list reused — no per-frame `query_entities`).

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ───────────────────────── armed-window serialization ─────────────────────
//
// The counting allocator's `OPEN`/`ALLOCS` are PROCESS-GLOBAL: two windows open at
// once on two test threads would add into, and reset, each other's counts. The
// default test runner runs these three tests on separate threads in parallel; until
// windows counted only their census's threads (2026-10-11), a sibling's allocations
// also landed in this test's armed window (observed: an impossible NEGATIVE "own
// alloc" delta of -10, and inflated steady-state counts). Every test here holds
// this lock so exactly one armed window is live at a time. Mirrors the
// `PROBE_LOCK` guard the in-crate `layout.rs` change-detection tests use for the
// same process-global-`static` reason. Poison-tolerant (a panicking measurement
// must not cascade a `PoisonError` into the others).
static ARM_LOCK: Mutex<()> = Mutex::new(());

fn lock_arm() -> std::sync::MutexGuard<'static, ()> {
    ARM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_ui::components::{ComputedRect, UiLayout, UiRoot};
use boyko_ui::layout::{ui_layout_apply, ui_layout_discovery};
use boyko_ui::resources::{LayoutScratch, UiViewport};
use boyko_ui::units::{LayoutType, Unit};

// ───────────────────────── counting allocator ─────────────────────────────

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
/// The census whose window is open, `0` while none is (see [`count_allocs`]).
static OPEN: AtomicU32 = AtomicU32::new(0);

// SAFETY: forwards every call verbatim to the system allocator; the only added work is
// [`tally`]: a read of a const-initialised, destructor-free thread-local `Cell<u32>` and
// relaxed atomic loads and increments. None of them allocates, so the allocator cannot
// re-enter itself, and none affects allocation correctness.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        tally(layout, true);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        tally(layout, false);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Worker threads in every pool a census measures.
const WORKERS: usize = 2;
/// How long a first-touch step waits for the threads it needs before it reds.
const FIRST_TOUCH_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
/// crossbeam-epoch's per-thread `Local` (`Collector::register`, 2304 B at align 128): the
/// object a thread allocates on its first epoch pin ("What a window counts", below the tests).
const EPOCH_LOCAL_BYTES: usize = 2304;
const EPOCH_LOCAL_ALIGN: usize = 128;

// ───────────────────────── world / schedule builders ──────────────────────

fn col(width: Unit, height: Unit) -> UiLayout {
    UiLayout { layout_type: LayoutType::Column, width, height, ..UiLayout::default() }
}
fn px(v: f32) -> Unit {
    Unit::Px(v)
}

fn noop_normal(_q: Query<&ComputedRect>) {}
fn noop_exclusive(_w: &mut EcsMaster) {}

/// A world seeded with the UI resources and a representative tree (root + mixed
/// fixed/stretch/nested children, so all depth pools grow), returning the world
/// and the first non-root child handle (for the size-tweak case).
fn seeded_world() -> (EcsMaster, Entity) {
    let mut world = EcsMaster::new();
    world.insert_resource(UiViewport { width: 400.0, height: 800.0, scale_factor: 1.0, generation: 0 });
    world.insert_resource(LayoutScratch::with_seeds());

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let mut v = probe.lock().expect("probe");
        let root = {
            let mut e = cmds.spawn(col(px(400.0), px(800.0)));
            e.insert(ComputedRect::default());
            e.insert(UiRoot);
            e.id()
        };
        v.push(root);
        let a = {
            let mut e = cmds.spawn(col(px(100.0), px(40.0)));
            e.insert(ComputedRect::default());
            e.set_parent(root);
            e.id()
        };
        v.push(a);
        for _ in 0..2 {
            let s = {
                let mut e = cmds.spawn(col(px(100.0), Unit::Stretch(1.0)));
                e.insert(ComputedRect::default());
                e.set_parent(root);
                e.id()
            };
            v.push(s);
        }
        for _ in 0..2 {
            let g = {
                let mut e = cmds.spawn(col(px(50.0), px(20.0)));
                e.insert(ComputedRect::default());
                e.set_parent(a);
                e.id()
            };
            v.push(g);
        }
    });
    let ids = sink.lock().expect("probe").clone();
    (world, ids[1])
}

fn build_pair_schedule(world: &mut EcsMaster) -> Measured {
    let (pool, census) = census_pool();
    let mut b = ScheduleBuilder::new(pool);
    let k = b.add_system(ui_layout_discovery).key();
    b.add_system(ui_layout_apply).after(k);
    Measured { sched: b.build(world), census }
}

fn build_baseline_schedule(world: &mut EcsMaster) -> Measured {
    let (pool, census) = census_pool();
    let mut b = ScheduleBuilder::new(pool);
    let k = b.add_system(noop_normal).key();
    b.add_system(noop_exclusive).after(k);
    Measured { sched: b.build(world), census }
}

/// Worst-of-N armed `run()` allocation count, after warming the schedule.
fn warmed_idle_allocs(world: &mut EcsMaster, sched: &mut Measured) -> usize {
    for _ in 0..8 {
        sched.run(world);
    }
    (0..4).map(|_| count_allocs(sched.census, || sched.run(world))).max().unwrap_or(0)
}

fn set_layout(world: &mut EcsMaster, e: Entity, l: UiLayout) {
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(l);
    });
}

fn resize(world: &mut EcsMaster, w: f32, h: f32) {
    let vp = world.resource_mut::<UiViewport>();
    vp.width = w;
    vp.height = h;
    vp.generation = vp.generation.wrapping_add(1);
}

// ───────────────────────── tests (DELTA == 0) ─────────────────────────────

#[test]
fn unchanged_frame_layout_pair_allocates_zero_over_baseline() {
    let _arm = lock_driver();
    let (mut wb, _) = seeded_world();
    let mut sb = build_baseline_schedule(&mut wb);
    let base = warmed_idle_allocs(&mut wb, &mut sb);

    let (mut wp, _) = seeded_world();
    let mut sp = build_pair_schedule(&mut wp);
    let pair = warmed_idle_allocs(&mut wp, &mut sp);

    // Non-vacuity first. The whole method is a SUBTRACTION, and it only means anything while the
    // baseline is actually allocating: this module's own header measures the parallel executor at
    // two allocations per frame, so a baseline of 0 would mean the schedule never ran and every
    // delta below it would be trivially satisfied.
    assert!(
        base > 0,
        "the baseline schedule allocated nothing, so the subtraction has no subtrahend and the \
         delta below would be vacuous"
    );
    // `<=`, not `==`, and this was MEASURED rather than tidied: the sweep of 2026-08-11 caught this
    // firing with `baseline 6, pair 5` — the pair allocating one FEWER than the baseline, which the
    // assertion's own message calls acceptable ("no more than") and which its two sibling tests in
    // this file already spell `assert!(pair <= base)`. What drifted, found on 2026-10-11, was a
    // first touch (a worker's 30-byte name copy, a worker's or the driver's 2304-byte epoch
    // `Local`) in either side's windows, which `warmed_idle_allocs`'s MAX charged to that side.
    // No window holds a first touch now ("What a window counts"); `==` would be a pin change.
    assert!(
        pair <= base,
        "steady-state: layout pair must allocate no more than the scheduler baseline \
         (baseline {base}, pair {pair}; the layout pair's own per-frame allocs = {})",
        pair as i64 - base as i64
    );
}

#[test]
fn non_structural_size_tweak_layout_pair_allocates_zero_over_baseline() {
    let _arm = lock_driver();
    // Baseline path also gets the tweak (so the command-apply cost cancels out of
    // the DELTA), then measure the relayout frame.
    let (mut wb, ab) = seeded_world();
    let mut sb = build_baseline_schedule(&mut wb);
    for _ in 0..8 {
        sb.run(&mut wb);
    }
    set_layout(&mut wb, ab, col(px(100.0), px(48.0)));
    let base = count_allocs(sb.census, || sb.run(&mut wb));

    let (mut wp, ap) = seeded_world();
    let mut sp = build_pair_schedule(&mut wp);
    for _ in 0..8 {
        sp.run(&mut wp);
    }
    set_layout(&mut wp, ap, col(px(100.0), px(48.0)));
    let pair = count_allocs(sp.census, || sp.run(&mut wp));

    assert!(
        pair <= base,
        "non-structural size-tweak relayout must add no allocations over baseline \
         (baseline {base}, pair {pair})"
    );
}

#[test]
fn resize_frame_layout_pair_allocates_zero_over_baseline() {
    let _arm = lock_driver();
    // Resize is a direct resource write (no command apply) and is NOT a root-SET
    // change, so apply reuses the cached `roots` list (no per-frame
    // `query_entities`). The DELTA over the (also-resized) baseline must be 0.
    let (mut wb, _) = seeded_world();
    let mut sb = build_baseline_schedule(&mut wb);
    for _ in 0..8 {
        sb.run(&mut wb);
    }
    resize(&mut wb, 440.0, 860.0);
    let base = count_allocs(sb.census, || sb.run(&mut wb));

    let (mut wp, _) = seeded_world();
    let mut sp = build_pair_schedule(&mut wp);
    for _ in 0..8 {
        sp.run(&mut wp);
    }
    resize(&mut wp, 440.0, 860.0);
    let pair = count_allocs(sp.census, || sp.run(&mut wp));

    assert!(
        pair <= base,
        "resize relayout (same root count) must add no allocations over baseline \
         — cached roots reused, no query_entities (baseline {base}, pair {pair})"
    );
}

// ───────────────────────── what a window counts ───────────────────────────
//
// A window counts exactly the threads that run the schedule it measures: the driver (the test
// thread, which dispatches, runs the exclusive system and may help run the parallel one) and the
// workers of that schedule's own pool. It counts them from the moment it opens, so each of those
// threads must be done with its FIRST TOUCHES before any window opens: the allocations a thread
// makes once in its life, at a moment its OS scheduler picks, and never again per frame.
//
// Until 2026-10-11 a window counted every thread of the process and assumed that eight warm frames
// had covered every first touch. Neither held. MEASURED on the dev host at `264e5b2d` (msvc,
// release, `--test-threads=4`, one process at a time) with every counted acquisition logged with
// its thread: a window held a first touch in 29 of 50 runs under a 4-CPU affinity mask and in 44
// of 50 at full width. A fresh pool's worker makes two, std's 30-byte UTF-16 copy of the thread's
// name (Windows thread start allocates it before the worker's first instruction) and the 2304-byte
// crossbeam-epoch `Local` its first steal sweep registers; the driver's own `Local` was in a window
// in 8 of the 50 masked runs. One in a pair window is CI run 7's red ("baseline 2, pair 3", both
// Windows release legs); the unmodified test red 19 times in 100 under the mask. One in a baseline
// window raised the subtrahend instead, which would hide a real +1. libtest's main thread, which
// formats a finished test's result line while the next test may have a window open, is the same
// class (round 11, `boyko_render/tests/ui_no_realloc.rs`). Since: 0 red in 495 release and 150
// debug runs over four shapes, and each of 300 instrumented runs of a test read exactly the
// driver's two per frame on both sides.
//
// So [`lock_driver`] registers the driver's `Local` before any census pool exists; [`census_pool`]
// enrols each pool's workers in the pool's own census at a boot barrier and waits until each has
// gone idle again, its first steal sweep behind it; and [`count_allocs`] counts the members of one
// census only, and reds when a `Local` is registered while its window is open.

/// A schedule under measurement and the census its pool's workers are enrolled in.
struct Measured {
    sched: Schedule,
    census: u32,
}

impl Measured {
    fn run(&mut self, world: &mut EcsMaster) {
        self.sched.run(world);
    }
}

thread_local! {
    /// The census this thread counts toward while that census's window is open (`0`: none).
    /// `const`-initialised and without a destructor, so reading it from inside the allocator
    /// never allocates and never fails, at any point of a thread's life.
    static MEMBER: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Whether [`touch_driver_epoch_local`] has registered this thread's epoch `Local`.
    static DRIVER_TOUCHED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The next census number; `0` means "no census".
static NEXT_CENSUS: AtomicU32 = AtomicU32::new(1);

/// crossbeam-epoch `Local`s registered by any thread since the process started.
static EPOCH_LOCALS: AtomicUsize = AtomicUsize::new(0);

/// Whether a pool worker's steal sweep probes a sibling whose deque is EMPTY, which pins
/// crossbeam-epoch and so registers the worker's `Local` on its first sweep.
///
/// The negation of `boyko_threadpool`'s `STEAL_EMPTY_GATE` (`worker.rs`, `pub(crate)`), which is
/// on only on `x86_64-pc-windows-gnu`. There a worker registers its `Local` only when it steals
/// from a sibling that has work, so [`census_pool`] asserts no more than one `Local` per worker
/// instead of exactly one, and the in-window check in [`count_allocs`] is what stands for the rest
/// (by reasoning only: no CI leg and no measurement here ran windows-gnu).
const IDLE_SWEEP_PINS: bool = !cfg!(all(windows, target_env = "gnu"));

/// What [`Counting`] adds to an acquisition: the epoch-`Local` count, at all times, and the window
/// count when the acquiring thread is a member of the census whose window is open.
#[inline]
fn tally(layout: Layout, fresh: bool) {
    if fresh && layout.size() == EPOCH_LOCAL_BYTES && layout.align() == EPOCH_LOCAL_ALIGN {
        EPOCH_LOCALS.fetch_add(1, Ordering::Relaxed);
    }
    let open = OPEN.load(Ordering::Relaxed);
    if open != 0 && MEMBER.try_with(std::cell::Cell::get).unwrap_or(0) == open {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Opens `census`'s window, runs `f` (frames of that census's schedule) and returns what the
/// census's threads acquired meanwhile. The calling thread is the driver and joins the census
/// here, so it is counted along with that census's pool workers; no other thread is.
///
/// RED if a crossbeam-epoch `Local` was registered while the window was open: that is a first
/// touch, which [`lock_driver`] and [`census_pool`] take out of every window, and a window that
/// holds one has priced a thread's start-up, not a frame.
fn count_allocs(census: u32, f: impl FnOnce()) -> usize {
    MEMBER.with(|m| m.set(census));
    let locals = EPOCH_LOCALS.load(Ordering::Relaxed);
    ALLOCS.store(0, Ordering::Relaxed);
    // Relaxed: a worker allocates for this window only inside a task that `f` hands the pool, and
    // the pool's push/steal is a release/acquire edge that orders this store before that task.
    OPEN.store(census, Ordering::Relaxed);
    f();
    OPEN.store(0, Ordering::Relaxed);
    // Every task `f` handed out has completed: the scope join is the acquire edge for its counts.
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let touched = EPOCH_LOCALS.load(Ordering::Relaxed) - locals;
    assert_eq!(
        touched, 0,
        "census {census}: {touched} epoch Local(s) ({EPOCH_LOCAL_BYTES} B, align \
         {EPOCH_LOCAL_ALIGN}) registered while the window was open — a thread's first touch \
         landed in a priced frame"
    );
    allocs
}

/// Takes [`ARM_LOCK`] and readies the calling thread to drive this file's censuses: its epoch
/// `Local` is registered here, before any census pool exists ([`touch_driver_epoch_local`], once
/// per thread).
fn lock_driver() -> std::sync::MutexGuard<'static, ()> {
    let guard = lock_arm();
    if !DRIVER_TOUCHED.with(std::cell::Cell::get) {
        touch_driver_epoch_local();
        DRIVER_TOUCHED.with(|t| t.set(true));
    }
    guard
}

/// Children the driver touch puts on its worker's deque for the driver to steal.
const DRIVER_TOUCH_CHILDREN: usize = 8;

/// Registers the DRIVER thread's epoch `Local`, and proves it.
///
/// The driver is no pool's worker, and the join it runs inside `install` HELPS: once the injector
/// is empty it probes the workers' deques, and its first probe registers its `Local`. When that
/// first probe comes is the schedule's choice: a join whose one task the driver takes back from
/// the injector itself, before a worker wakes, never probes. So without this the driver's `Local`
/// could land in any window. (The construction is `alloc_frame_attribution`'s and
/// `e1_engine_ui_alloc_census`'s, stated independently here.)
///
/// Since ci-fix-1 the touch pool's `build()` registers its builder -- the driver -- and its one
/// worker before it returns (`ThreadPoolBuilder::build`), so the touch counts two `Local`s, both
/// from `build()`, and the forced steal registers none. The steal stays as the witness that the
/// driver's join really probes a worker's deque: the `install` body spins until that worker has
/// taken the leader and pushed [`DRIVER_TOUCH_CHILDREN`] children onto its own deque, and the
/// leader spins until they have run, so the driver's join finds the injector empty and the
/// worker's deque full, and every child it runs it stole. Both spins are bounded by
/// [`FIRST_TOUCH_WAIT`].
fn touch_driver_epoch_local() {
    let driver = std::thread::current().id();
    let pushed = AtomicBool::new(false);
    let done = AtomicUsize::new(0);
    let on_driver = AtomicUsize::new(0);
    let before = EPOCH_LOCALS.load(Ordering::Relaxed);
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    {
        let (p, pushed, done, on_driver) = (&*pool, &pushed, &done, &on_driver);
        p.install(|s| {
            s.spawn(move || {
                p.scope(|inner| {
                    for _ in 0..DRIVER_TOUCH_CHILDREN {
                        inner.spawn(move || {
                            if std::thread::current().id() == driver {
                                on_driver.fetch_add(1, Ordering::Relaxed);
                            }
                            done.fetch_add(1, Ordering::Release);
                        });
                    }
                    pushed.store(true, Ordering::Release);
                    let deadline = std::time::Instant::now() + FIRST_TOUCH_WAIT;
                    while done.load(Ordering::Acquire) < DRIVER_TOUCH_CHILDREN
                        && std::time::Instant::now() < deadline
                    {
                        std::hint::spin_loop();
                    }
                });
            });
            // Acquire: pairs with the leader's Release store above, made after its pushes.
            let deadline = std::time::Instant::now() + FIRST_TOUCH_WAIT;
            while !pushed.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
                std::hint::spin_loop();
            }
        });
    }
    let registered = EPOCH_LOCALS.load(Ordering::Relaxed) - before;
    drop(pool);
    let stolen = on_driver.load(Ordering::Relaxed);
    assert!(
        stolen >= 1,
        "driver touch: the driver ran none of the {DRIVER_TOUCH_CHILDREN} children on its \
         worker's deque — it never stole, so the touch cannot vouch for the driver's epoch Local"
    );
    // ci-fix-1: 1 -> 2. Rule: a build registers its builder and every worker inside `build()`; the
    // touch pool's builder is the driver and it has one worker, and the forced steal adds none.
    assert_eq!(
        registered, 2,
        "driver touch: {registered} epoch Locals registered; the touch pool's build() registers \
         its builder (the driver) and its one worker, and the forced steal registers none"
    );
}

/// A [`WORKERS`]-worker pool enrolled in a fresh census, every thread of which has made its first
/// touches: VERIFIED, not assumed.
///
/// * [`enrol`] is a boot barrier: every worker holds one of its tasks at the same moment, so every
///   worker has started (std's name copy on Windows is behind it), and each joins the census.
/// * [`settle`]: every worker has gone idle since. (Each worker's epoch `Local` is registered
///   inside `build()` since ci-fix-1, so the count below is complete before the barrier.)
/// * The count: one `Local` per worker since before the pool was built where a sweep probes an
///   empty sibling ([`IDLE_SWEEP_PINS`]), and never more than one per worker anywhere.
///
/// Nothing here hands the pool a task beyond the barrier's `WORKERS + 1`, so every schedule
/// reaches its windows after the same number of injector pushes. That keeps the injector's block
/// allocation (one per 63 pushes, crossbeam-deque's `Injector::push`) out of the windows and at
/// the same phase for the baseline and the pair. A forcing round, one extra spawn each, would tie
/// that phase to how many rounds a run happened to need.
fn census_pool() -> (Arc<ThreadPool>, u32) {
    let census = NEXT_CENSUS.fetch_add(1, Ordering::Relaxed);
    let before = EPOCH_LOCALS.load(Ordering::Relaxed);
    let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();
    enrol(&pool, census);
    settle(&pool);
    // Every `Local` counted here was allocated before its worker's `mark_idle`, whose Release
    // `fetch_or` the Acquire load in `parked_mask` (`settle`) observed.
    let registered = EPOCH_LOCALS.load(Ordering::Relaxed) - before;
    if IDLE_SWEEP_PINS {
        assert_eq!(
            registered, WORKERS,
            "census {census}: a fresh {WORKERS}-worker pool registered {registered} epoch \
             Locals by the time every worker had swept and gone idle; one per worker is the \
             premise that keeps a worker's first touch out of the windows"
        );
    } else {
        assert!(
            registered <= WORKERS,
            "census {census}: a fresh {WORKERS}-worker pool registered {registered} epoch \
             Locals, more than one per worker"
        );
    }
    (pool, census)
}

/// The boot barrier: `WORKERS + 1` tasks, each waiting until all of them have arrived. A thread
/// runs at most one at a time, and the driver's helping join is the only thread besides the
/// workers that runs this pool's tasks, so the driver holds one and each worker holds one. Each
/// holder enrols its thread in `census`. RED if they have not all arrived within
/// [`FIRST_TOUCH_WAIT`].
fn enrol(pool: &ThreadPool, census: u32) {
    let holders = WORKERS + 1;
    let arrived = AtomicUsize::new(0);
    // The fewest arrivals a holder saw when it gave up (`usize::MAX`: none gave up). A holder that
    // gives up frees its thread for a queued holder, so `arrived` alone can still reach `holders`.
    let short_at = AtomicUsize::new(usize::MAX);
    pool.install(|s| {
        for _ in 0..holders {
            let (arrived, short_at) = (&arrived, &short_at);
            s.spawn(move || {
                MEMBER.with(|m| m.set(census));
                arrived.fetch_add(1, Ordering::AcqRel);
                let deadline = std::time::Instant::now() + FIRST_TOUCH_WAIT;
                loop {
                    let seen = arrived.load(Ordering::Acquire);
                    if seen >= holders {
                        break;
                    }
                    if std::time::Instant::now() >= deadline {
                        short_at.fetch_min(seen, Ordering::Relaxed);
                        break;
                    }
                    std::hint::spin_loop();
                }
            });
        }
    });
    let short_at = short_at.load(Ordering::Relaxed);
    assert!(
        short_at == usize::MAX,
        "census {census}: {short_at} of {holders} holders had reached the boot barrier of a \
         {WORKERS}-worker pool when the first gave up — a worker never took a task, so it may \
         still be booting, and it is not enrolled"
    );
}

/// Waits until every worker of `pool` has gone idle again after [`enrol`].
///
/// A worker's idle bit is clear while it runs a task, and only the worker itself sets it, in
/// `mark_idle`, after a backoff of polls that each found nothing (and each swept its sibling).
/// Every barrier holder completed before this runs, so a set bit seen here was set after that
/// worker's holder. `ThreadPool::parked_mask` is the pool's public read-only receipt of the bits.
/// RED if they are not all set within [`FIRST_TOUCH_WAIT`].
fn settle(pool: &ThreadPool) {
    let all = (1u64 << WORKERS) - 1;
    let deadline = std::time::Instant::now() + FIRST_TOUCH_WAIT;
    loop {
        let parked = pool.parked_mask() & all;
        if parked == all {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "a {WORKERS}-worker pool's idle bits read {parked:#b} of {all:#b} after \
             {FIRST_TOUCH_WAIT:?}: a worker has not gone idle since its barrier task, so its \
             first steal sweep may still be ahead of a window"
        );
        std::thread::yield_now();
    }
}
