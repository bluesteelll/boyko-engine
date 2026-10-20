//! GATE 3 (no_alloc) — the data-bind pair allocates NOTHING of its own per
//! frame in steady state, over many bound nodes, via a counting global allocator
//! and BASELINE SUBTRACTION (the same methodology as the P1 `zero_alloc.rs`).
//!
//! `Schedule::run`'s parallel executor allocates a small FIXED number of bytes
//! per frame for its own task machinery, independent of the systems' bodies. So
//! the test measures the bind pair's OWN per-frame allocations as the DELTA
//! between two schedules of identical SHAPE (three exclusive systems, which the
//! executor runs inline on the driver) over the same warmed bound tree:
//!   * baseline = `[mutator_system, noop_discovery, noop_apply]`
//!   * pair     = `[mutator_system, ui_bind_discovery, ui_bind_apply]`
//!
//! A DELTA of 0 proves the bind pair allocates nothing of its own per frame. A
//! window counts the threads that run the schedule it measures and no other, each
//! past its first touches: see "Why the shape" and "What a window counts", below
//! the tests. Two steady-state cases at DELTA 0:
//!   (a) a STILL frame (no source changed — discovery returns false, apply
//!       early-returns): the 0%-gate path must not allocate;
//!   (b) a DIRTY frame (every source changed → every widget re-formats): the
//!       apply path's retained scratch buffers (widget query, arch-id scratch,
//!       stack-buffer format) must allocate nothing once warmed.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_macros::{Bindable, Component};

use boyko_ui::binding::bind_system::{ui_bind_apply, ui_bind_discovery, UiBindScratch};
use boyko_ui::binding::components::{BindText, TemplateId, UiTextBuffer, NO_FIELD};
use boyko_ui::binding::Bindable;

// ───────────────────────── armed-window serialization ──────────────────────

static ARM_LOCK: Mutex<()> = Mutex::new(());

fn lock_arm() -> std::sync::MutexGuard<'static, ()> {
    ARM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// ───────────────────────── counting allocator ──────────────────────────────

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

// ───────────────────────── bindable source ─────────────────────────────────

#[derive(Component, Bindable, Clone, Copy, Debug)]
#[repr(C)]
struct Health {
    current: f32,
    max: f32,
}

// ───────────────────────── noop baseline systems ───────────────────────────

use boyko_macros::Resource;

/// In-schedule mutation queue (the realistic gameplay-system mutate path; an
/// out-of-band `run_system` mutate collides with the schedule's `last_run`, a
/// harness artifact — see p4_bind.rs).
#[derive(Resource, Default)]
struct MutQueue {
    pending: Vec<(Entity, Health)>,
}

/// In-schedule mutator drained ahead of the bind/baseline systems.
#[allow(clippy::needless_pass_by_ref_mut)]
fn mutator_system(world: &mut EcsMaster) {
    let pending = std::mem::take(&mut world.resource_mut::<MutQueue>().pending);
    for (e, h) in pending {
        if let Some(mut g) = world.get_component_mut::<Health>(e) {
            *g = h;
        }
    }
}

/// Baseline discovery: an exclusive no-op in `ui_bind_discovery`'s slot. Both bind systems take
/// `&mut EcsMaster`, so the executor runs them inline on the driver and hands the pool no task;
/// a `Query` system here handed the pool one per frame, whose allocation the pair never paid
/// and so could spend on its own unseen ("Why the shape", below the tests).
#[allow(clippy::needless_pass_by_ref_mut)]
fn noop_discovery(_world: &mut EcsMaster) {}

/// Baseline apply: an exclusive no-op in `ui_bind_apply`'s slot.
#[allow(clippy::needless_pass_by_ref_mut)]
fn noop_apply(_world: &mut EcsMaster) {}

// ───────────────────────── world builders ──────────────────────────────────

const N_WIDGETS: usize = 256;

/// Builds a world with `N_WIDGETS` `Health` sources each driving a `BindText` widget, and its
/// schedule on a census pool. Returns the world, the schedule + census, and the sources (for the
/// dirty-frame case). Both schedule shapes carry the SAME `mutator_system` ahead of the pair so
/// the mutation churn cancels out of the alloc delta.
fn build_bound_world(schedule_pair: bool) -> (EcsMaster, Measured, Vec<Entity>) {
    let (pool, census) = census_pool();
    let mut world = EcsMaster::new();

    let mut scratch = UiBindScratch::default();
    Health::register_bind_accessor();
    scratch.register_bound_id(Health::component_id());
    world.insert_resource(scratch);
    world.insert_resource(MutQueue::default());

    let mut builder = ScheduleBuilder::new(pool);
    let mutate = builder.add_system(mutator_system).key();
    if schedule_pair {
        let discovery = builder.add_system(ui_bind_discovery).after(mutate).key();
        builder.add_system(ui_bind_apply).after(discovery);
    } else {
        let n = builder.add_system(noop_discovery).after(mutate).key();
        builder.add_system(noop_apply).after(n);
    }
    let schedule = Measured {
        sched: builder.build(&mut world),
        census,
    };

    // Spawn sources + widgets.
    let sources: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let sp = Arc::clone(&sources);
    let comp = Health::component_id();
    world.run_system(move |mut cmds: Commands| {
        let mut v = sp.lock().unwrap();
        for i in 0..N_WIDGETS {
            let src = cmds.spawn(Health { current: i as f32, max: 100.0 }).id();
            let mut ec = cmds.spawn(BindText {
                source: src,
                comp,
                field: 0,
                field2: NO_FIELD,
                template: TemplateId::Value,
            });
            ec.insert(UiTextBuffer::default());
            v.push(src);
        }
    });
    let srcs = sources.lock().unwrap().clone();

    (world, schedule, srcs)
}

/// Enqueues a bump of every source so the next `schedule.run` is dirty (the
/// in-schedule mutator applies them, every widget reformats). The enqueue is a
/// `Vec::extend` into the retained `MutQueue` — its own steady-state allocation
/// is part of BOTH paths (pair + baseline both run `mutator_system`), so it
/// cancels out of the delta.
fn touch_all(world: &mut EcsMaster, srcs: &[Entity]) {
    let q = &mut world.resource_mut::<MutQueue>().pending;
    for (i, &s) in srcs.iter().enumerate() {
        q.push((s, Health { current: (i + 1) as f32, max: 100.0 }));
    }
}

// ───────────────────────── tests ───────────────────────────────────────────

#[test]
fn bind_still_frame_zero_own_allocations() {
    let _g = lock_driver();

    // Pair path: warm to high-water (touch once so every sink formats + the
    // scratch buffers grow to steady capacity, then settle on still frames).
    let (mut pw, mut ps, psrcs) = build_bound_world(true);
    touch_all(&mut pw, &psrcs);
    for _ in 0..6 {
        ps.run(&mut pw);
    }
    // Now still frames: no source changed → discovery false, apply early-returns.
    let pair = count_allocs(ps.census, || {
        for _ in 0..8 {
            ps.run(&mut pw);
        }
    });

    // Baseline path: identical shape + warm (same one-time touch).
    let (mut bw, mut bs, bsrcs) = build_bound_world(false);
    touch_all(&mut bw, &bsrcs);
    for _ in 0..6 {
        bs.run(&mut bw);
    }
    let base = count_allocs(bs.census, || {
        for _ in 0..8 {
            bs.run(&mut bw);
        }
    });

    assert_baseline_counted(base);
    assert!(
        pair <= base,
        "still-frame bind pair must not allocate beyond the executor baseline \
         (pair={pair}, baseline={base}, delta={})",
        pair as isize - base as isize
    );
}

#[test]
fn bind_dirty_frame_zero_own_allocations_steady_state() {
    let _g = lock_driver();

    // Pair path warmed to high-water with repeated dirty frames so every retained
    // scratch buffer reaches steady capacity.
    let (mut pw, mut ps, psrcs) = build_bound_world(true);
    for _ in 0..6 {
        touch_all(&mut pw, &psrcs);
        ps.run(&mut pw);
    }
    let pair = count_allocs(ps.census, || {
        for _ in 0..8 {
            touch_all(&mut pw, &psrcs);
            ps.run(&mut pw);
        }
    });

    // Baseline path: identical shape + the SAME per-frame touch_all source bumps
    // (so the command-queue churn cancels out of the delta).
    let (mut bw, mut bs, bsrcs) = build_bound_world(false);
    for _ in 0..6 {
        touch_all(&mut bw, &bsrcs);
        bs.run(&mut bw);
    }
    let base = count_allocs(bs.census, || {
        for _ in 0..8 {
            touch_all(&mut bw, &bsrcs);
            bs.run(&mut bw);
        }
    });

    assert_baseline_counted(base);
    assert!(
        pair <= base,
        "dirty-frame bind pair (over {N_WIDGETS} widgets) must allocate nothing of \
         its own beyond the baseline (pair={pair}, baseline={base}, delta={})",
        pair as isize - base as isize
    );
}

/// Non-vacuity: the subtraction means something only while the baseline's window counted the
/// executor's own per-frame allocation (`install`'s scope state, one per frame on the driver). A
/// window that counted nothing would pass every delta.
fn assert_baseline_counted(base: usize) {
    assert!(
        base > 0,
        "the baseline window counted nothing, so the subtraction has no subtrahend and the \
         delta would be vacuous"
    );
}

// ───────────────────────── why the shape ───────────────────────────────────
//
// Until 2026-10-11 the baseline was `[mutator_system, noop_normal, noop_exclusive]`, a `Query`
// system in discovery's slot, and this header called both shapes "one normal + one exclusive
// system". `ui_bind_discovery` has taken `&mut EcsMaster` since it landed (P4), so the executor
// ran the whole pair inline on the driver and handed the pool nothing, while the baseline handed
// it `noop_normal` every frame and paid that task's 4096-byte chunk. MEASURED at `be2a3939` (dev
// host, msvc, release, `--test-threads=4`, every counted acquisition logged with its thread, 160
// runs): every window without a first touch in it read pair 8 / baseline 16 on still frames and
// pair 64 / baseline 72 on dirty ones. So the gate passed a pair that allocated one object of its
// own every frame: an 8-byte `Box` planted at the top of `ui_bind_discovery`, or in
// `ui_bind_apply` past its still-frame return, left both tests green in 40 of 40 runs. Now both
// shapes hand the pool nothing per frame, a clean window reads the same on both (8 / 8, 64 / 64),
// and either plant reds 40 of 40 runs (the apply plant in the dirty test, the one that reaches it).
//
// ───────────────────────── what a window counts ────────────────────────────
//
// A window counts exactly the threads that run the schedule it measures: the driver (the test
// thread, which dispatches and runs all three exclusive systems) and the workers of that
// schedule's own pool, which no frame of either shape reaches today but which stay counted so a
// bind system that turns parallel is still measured. It counts them from the moment it opens, so
// each of those threads must be done with its FIRST TOUCHES before any window opens: the
// allocations a thread makes once in its life, at a moment its OS scheduler picks.
//
// Until 2026-10-11 a window counted every thread of the process and assumed that six warm frames
// had covered every first touch. In the same 160 runs a window held one in 30 of 320 test runs, 18
// of them a pair window: a pool worker's 2304-byte crossbeam-epoch `Local` and std's 30-byte
// UTF-16 copy of its name (Windows thread start); crossbeam-epoch's once-per-process collector
// (its 640-byte `Global` and 2072-byte queue sentinel) when that worker was the process's first
// thread to pin; and the OTHER test's thread starting up (the 76-byte UTF-16 copy of
// `bind_still_frame_zero_own_allocations`) before it blocked on `ARM_LOCK`. A baseline window
// also held the driver's own `Local`, registered by its first probe of a worker's deque while it
// joined a `noop_normal` task. Behind the old slack none of them could red. With only the shape
// corrected, the all-thread census red 10 times in 230 runs at full width ("pair=66,
// baseline=64" and the like; 0 in 230 under 4- and 2-CPU masks), and a first touch in a baseline
// window would raise the subtrahend and hide a real +1: the class `zero_alloc.rs` fixed on the
// same date, and round 11 in `boyko_render/tests/ui_no_realloc.rs` (libtest's own threads).
// Since: 0 red in 560 release, 100 debug and 100 `force_alloc_panic` runs over four shapes, and
// each of 300 instrumented test runs read exactly the driver's frames on both sides, nothing else.
//
// So [`lock_driver`] registers the driver's `Local` before any census pool exists (the process's
// first pin, so the collector's one-time objects come with it); [`census_pool`] enrols each pool's
// workers in the pool's own census at a boot barrier and waits until each has gone idle again, its
// first steal sweep behind it; and [`count_allocs`] counts the members of one census only, and
// reds when a `Local` is registered while its window is open. The construction is
// `zero_alloc.rs`'s, stated independently here.

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
/// the injector itself, before a worker wakes, never probes, and [`census_pool`]'s barrier is
/// such a join. So without this the driver's `Local` could land in a pool's exact count, or in
/// any window of a schedule that hands its pool a task.
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
/// Nothing here hands the pool a task beyond the barrier's `WORKERS + 1`, so both schedules reach
/// their windows after the same number of injector pushes, which keeps the injector's block
/// allocation (one per 63 pushes, crossbeam-deque's `Injector::push`) out of the windows.
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
