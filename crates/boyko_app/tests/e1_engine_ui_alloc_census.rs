//! **E1 — UG-03's engine scene (the engine design's G-ALLOC): what a steady frame of the headless
//! `EnginePlugins` + UI app allocates, by class.**
//!
//! # What is measured
//!
//! App (b) of rung B2 (`b2_common::app_b`: `EnginePlugins` plus today's UI plugin composition, the
//! committed five-node UI fixture, seven device-inert meshes, a W = 4 pool) under a counting
//! `#[global_allocator]` with UG-03's classes. The run is `finish()`, warm-up frames 0..=9, then
//! the **steady window, frames 10..=19** (the design's "10..20", read as a half-open range), every
//! frame `update_with_delta(16 ms)`. **OTHER is the data path**: every steady acquisition that is
//! not a scope frame, a `ScopeBlock` chunk or an injector block. Its target is 0.
//!
//! # The classifier is COPIED from `boyko_physics/tests/alloc_frame_census.rs`, not extracted
//!
//! Extraction would be a code motion in a file whose lines the documents anchor, and it would
//! confound the census's trunk re-pin with an instrument change (B2 decision D6). The copy is held
//! to the pool three ways: the `CHUNK0` / `CHUNK_ALIGN` literals are asserted equal to
//! `boyko_threadpool::__layout_receipt`; the behavioural class control prices raw `pool.install`
//! frames (empty → scope 1 chunk 0; one spawn → scope 1 chunk 1, OTHER bounded by one first touch
//! per thread); and the positive controls prove the counter is live and sees worker threads.
//!
//! **One class is added: `EPOCH`**, `align == 128 && size == 2304` — crossbeam-epoch's per-thread
//! `Local`, registered on a thread's FIRST peer steal (the census names it the only first-touch
//! `OTHER` site its trace found, and measures it at 2304 B). The census leaves it in `OTHER` and
//! budgets `2 × W` for it inside a 256-frame window. E1's window is 10 frames, where that budget
//! would hide a data-path regression of up to 8 acquisitions — so E1 takes first touch OUT of the
//! window instead of budgeting for it: see "First touch" below.
//!
//! # First touch — forced before the window, and verified
//!
//! The `EPOCH` count is taken from a snapshot made BEFORE the app's pool exists, so it counts
//! exactly the app's workers. After `finish()` and before frame 0, [`force_first_touch`] drives the
//! app's own pool until that count reaches one `Local` per worker (bounded rounds; RED if it
//! cannot), and the run then asserts it is EXACTLY one per worker. MEASURED on this tree: all four
//! workers register while the app is composed (1–2 after the pool is built, 4 after composition),
//! so the forcing takes 0 rounds today; it is there so a tree that stops touching every worker
//! before frame 0 still takes first touch out of the window rather than into it, and
//! [`epoch_class_matches_a_fresh_pool`] proves on a fresh pool that it does (one `Local` per worker
//! within two rounds, none after). A worker that has registered its `Local` has also booted (std's
//! `SetThreadDescription` name copy happens at thread start), so no per-thread first-touch object
//! remains to land in the window. The gate then asserts `EPOCH == 0` in the steady window and
//! budgets **no** first-touch allowance on `OTHER`: `OTHER ≤ other_per_frame × 10`, exactly.
//!
//! # Anti-vacuity
//!
//! * **UI nodes > 0**: entities carrying `UiLayout` (every node `spawn_ui_tree` lowers carries one)
//!   read after frame 9, plus at least one `UiRoot`; and the fixture's node count is exact.
//! * **Mesh instances > 0 on EVERY steady frame**: `MeshRenderScratch::instance_count()` read after
//!   each frame, so the window cannot be green over frames that gathered nothing.
//! * The counter is live inside the window (the census's nested-snapshot probe frame).
//!
//! # Pins and their rule
//!
//! [`pin`] is the census's `Pin` shape. Pins are recorded at B2 from the first run and confirmed by
//! two more (counts only) as **ceilings that only fall**: a lower reading on a later rung lowers the
//! pin; an upward move is RED, re-measured and attributed, never widened (G-ALLOC "budget only
//! decreases"; UG-03 P9).
//!
//! # Red-first (test-only knobs, no engine source reads them)
//!
//! * `BOYKO_B2_CANARY=e1_other` adds a Main system that makes a fresh `Vec` and pushes into it
//!   every frame: RED on OTHER.
//! * `BOYKO_B2_CANARY=e1_no_mesh` spawns no mesh: RED on the mesh anti-vacuity.
//! * `BOYKO_B2_CANARY=e1_no_ui` composes `UiPlugin` without a document: RED on the UI anti-vacuity.
//!
//! # What E1 does not see
//!
//! The runner, the device frame and the VisibilityBuffer path — those are E1v at HO2. And, like the
//! census, only the Rust heap: `ComponentPool` / `ScratchColumn` growth commits pages through
//! `VirtualAlloc`, which this counter does not see (G-RES records committed column bytes instead).
//!
//! # How to run
//!
//! ```text
//! cargo test -p boyko-app --test e1_engine_ui_alloc_census -- --nocapture
//! ```
//!
//! `harness = false`, with the census's libtest command-line contract (`--list`, filters,
//! `--exact`, `--skip`, `--nocapture` / `RUST_TEST_NOCAPTURE`, `--show-output`, exit 101 on
//! failure). Counts only; no wall-clock figure is produced anywhere in this binary.

// No `#![cfg(not(miri))]`: a `harness = false` target must have a `main` in every configuration.
// An integration-test target: compiled out of every shipping build. `Mutex` holds the report.
#![allow(clippy::disallowed_types)]

mod b2_common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::ThreadId;

use boyko_ecs::App;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_macros::{Bundle, Component};
use boyko_render::MeshRenderScratch;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};
use boyko_ui::components::{UiLayout, UiRoot};

use b2_common::{
    FRAME, MESHES, THREADS, UI_FIXTURE, app_b, assert_fixture_parses_clean, canary, new_app,
};

// ═══════════════════════════════════════════════════════════════════════════
// Report output — libtest's capture contract, without libtest (census copy)
// ═══════════════════════════════════════════════════════════════════════════

static STREAM: AtomicBool = AtomicBool::new(false);
static HELD: Mutex<String> = Mutex::new(String::new());
const HELD_RESERVE: usize = 1 << 20;

fn emit(args: std::fmt::Arguments<'_>, newline: bool) {
    if STREAM.load(Ordering::Relaxed) {
        if newline {
            println!("{args}");
        } else {
            print!("{args}");
        }
    } else {
        let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        // Writing into a `String` cannot fail.
        let _ = std::fmt::Write::write_fmt(&mut *held, args);
        if newline {
            held.push('\n');
        }
    }
}

fn dump_held() {
    let mut held = match HELD.try_lock() {
        Ok(h) => h,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return,
    };
    if !held.is_empty() {
        print!("{held}");
        held.clear();
    }
}

macro_rules! say {
    () => {
        emit(format_args!(""), true)
    };
    ($($arg:tt)*) => {
        emit(format_args!($($arg)*), true)
    };
}

macro_rules! say_inline {
    ($($arg:tt)*) => {
        emit(format_args!($($arg)*), false)
    };
}

// ═══════════════════════════════════════════════════════════════════════════
// The counting global allocator (census copy, plus the EPOCH class and an OTHER log)
// ═══════════════════════════════════════════════════════════════════════════

static N_ALLOC: AtomicU64 = AtomicU64::new(0);
static N_REALLOC: AtomicU64 = AtomicU64::new(0);
static N_DEALLOC: AtomicU64 = AtomicU64::new(0);
static B_ALLOC: AtomicU64 = AtomicU64::new(0);
static B_REALLOC: AtomicU64 = AtomicU64::new(0);
static B_DEALLOC: AtomicU64 = AtomicU64::new(0);

// The census's four classes (`alloc_frame_census.rs`, "Layout classes"), predicates unchanged.
static C_SCOPE: AtomicU64 = AtomicU64::new(0);
static C_CHUNK: AtomicU64 = AtomicU64::new(0);
static C_INJ: AtomicU64 = AtomicU64::new(0);
static C_OTHER: AtomicU64 = AtomicU64::new(0);
/// E1's fifth class: crossbeam-epoch's per-thread `Local` (see the header).
static C_EPOCH: AtomicU64 = AtomicU64::new(0);

const CHUNK0: usize = 4096;
const CHUNK_ALIGN: usize = 64;
const INJECTOR_BLOCK_BYTES: usize = 8 + 63 * 24;
const SCOPE_SHARED_BYTES: usize = 256;
const SCOPE_SHARED_ALIGN: usize = 128;
/// crossbeam-epoch's `Local`: a `CachePadded` epoch (align 128) beside a 64-slot deferred bag.
/// The census measured it at 2304 B ("two 2304-byte epoch `Local`s", its "Open" section);
/// [`epoch_class_matches_a_fresh_pool`] re-measures it.
const EPOCH_LOCAL_BYTES: usize = 2304;
const EPOCH_LOCAL_ALIGN: usize = 128;

/// The OTHER log: `(frame << 48) | (align_log2 << 40) | size`, one per OTHER acquisition while
/// [`LOG_ARMED`] is set, dropped when full. Static and fixed-size, so logging never allocates.
const OTHER_LOG_CAP: usize = 512;
static OTHER_LOG: [AtomicU64; OTHER_LOG_CAP] = [const { AtomicU64::new(0) }; OTHER_LOG_CAP];
static OTHER_LOG_LEN: AtomicUsize = AtomicUsize::new(0);
static LOG_ARMED: AtomicBool = AtomicBool::new(false);
/// The frame index the driver is in (for the OTHER log).
static CUR_FRAME: AtomicU64 = AtomicU64::new(0);

#[inline]
fn classify(layout: Layout) {
    let (size, align) = (layout.size(), layout.align());
    let class = if align == SCOPE_SHARED_ALIGN && size == SCOPE_SHARED_BYTES {
        &C_SCOPE
    } else if align == CHUNK_ALIGN && size.is_power_of_two() && size >= CHUNK0 {
        &C_CHUNK
    } else if size == INJECTOR_BLOCK_BYTES && align == 8 {
        &C_INJ
    } else if align == EPOCH_LOCAL_ALIGN && size == EPOCH_LOCAL_BYTES {
        &C_EPOCH
    } else {
        if LOG_ARMED.load(Ordering::Relaxed) {
            let i = OTHER_LOG_LEN.fetch_add(1, Ordering::Relaxed);
            if i < OTHER_LOG_CAP {
                let frame = CUR_FRAME.load(Ordering::Relaxed) & 0xFFFF;
                let word = (frame << 48)
                    | (u64::from(align.trailing_zeros()) << 40)
                    | (size as u64 & 0xFF_FFFF_FFFF);
                OTHER_LOG[i].store(word, Ordering::Relaxed);
            }
        }
        &C_OTHER
    };
    class.fetch_add(1, Ordering::Relaxed);
}

struct CountingAlloc;

// SAFETY: pure delegation to `System` with relaxed counter side effects; every layout / pointer
// contract is forwarded unchanged, and the counters and the OTHER log never allocate (they are
// `static` atomics), so no re-entrancy is possible.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        N_ALLOC.fetch_add(1, Ordering::Relaxed);
        B_ALLOC.fetch_add(layout.size() as u64, Ordering::Relaxed);
        classify(layout);
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        N_ALLOC.fetch_add(1, Ordering::Relaxed);
        B_ALLOC.fetch_add(layout.size() as u64, Ordering::Relaxed);
        classify(layout);
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        N_REALLOC.fetch_add(1, Ordering::Relaxed);
        B_REALLOC.fetch_add(new_size as u64, Ordering::Relaxed);
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        N_DEALLOC.fetch_add(1, Ordering::Relaxed);
        B_DEALLOC.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Snap {
    alloc: u64,
    realloc: u64,
    dealloc: u64,
    alloc_bytes: u64,
    realloc_bytes: u64,
    dealloc_bytes: u64,
    /// `scope + chunk + inj + epoch + other == alloc`.
    scope: u64,
    chunk: u64,
    inj: u64,
    epoch: u64,
    other: u64,
}

impl Snap {
    /// `SeqCst` on the read side, `Relaxed` on the increment side: the happens-before edge is the
    /// pool's own join (a `Schedule::run` returns only after every spawned task has completed).
    fn now() -> Self {
        Snap {
            alloc: N_ALLOC.load(Ordering::SeqCst),
            realloc: N_REALLOC.load(Ordering::SeqCst),
            dealloc: N_DEALLOC.load(Ordering::SeqCst),
            alloc_bytes: B_ALLOC.load(Ordering::SeqCst),
            realloc_bytes: B_REALLOC.load(Ordering::SeqCst),
            dealloc_bytes: B_DEALLOC.load(Ordering::SeqCst),
            scope: C_SCOPE.load(Ordering::SeqCst),
            chunk: C_CHUNK.load(Ordering::SeqCst),
            inj: C_INJ.load(Ordering::SeqCst),
            epoch: C_EPOCH.load(Ordering::SeqCst),
            other: C_OTHER.load(Ordering::SeqCst),
        }
    }

    fn since(self, base: Snap) -> Snap {
        Snap {
            alloc: self.alloc - base.alloc,
            realloc: self.realloc - base.realloc,
            dealloc: self.dealloc - base.dealloc,
            alloc_bytes: self.alloc_bytes - base.alloc_bytes,
            realloc_bytes: self.realloc_bytes - base.realloc_bytes,
            dealloc_bytes: self.dealloc_bytes - base.dealloc_bytes,
            scope: self.scope - base.scope,
            chunk: self.chunk - base.chunk,
            inj: self.inj - base.inj,
            epoch: self.epoch - base.epoch,
            other: self.other - base.other,
        }
    }

    fn acquisitions(self) -> u64 {
        self.alloc + self.realloc
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Regimes
// ═══════════════════════════════════════════════════════════════════════════

/// Frames 0..=9: warm-up (the design's "10..20" steady window starts at frame 10).
const WARM_BUDGET: usize = 10;
/// Frames 10..=19: the steady window.
const STEADY_FRAMES: usize = 10;
const TOTAL_FRAMES: usize = WARM_BUDGET + STEADY_FRAMES;
/// The deliberate allocation the in-window liveness probe makes.
const PROBE_BYTES: usize = 1 << 20;
/// The UI fixture's node count (`b2_common/app_b.ui`: root, title, button, track, fill).
const FIXTURE_NODES: usize = 5;
/// Rounds [`force_first_touch`] may take before it is RED.
const FORCE_ROUNDS: usize = 4096;

struct Stat {
    min: u64,
    max: u64,
    sum: u64,
    n: usize,
}

impl Stat {
    fn of(samples: &[Snap], pick: fn(&Snap) -> u64) -> Self {
        let mut min = u64::MAX;
        let mut max = 0u64;
        let mut sum = 0u64;
        for s in samples {
            let v = pick(s);
            min = min.min(v);
            max = max.max(v);
            sum += v;
        }
        Stat {
            min,
            max,
            sum,
            n: samples.len(),
        }
    }
    fn mean(&self) -> f64 {
        self.sum as f64 / self.n as f64
    }
}

/// `K`: one past the last frame of the whole run above the steady max (census definition).
fn settle_index(samples: &[Snap], steady_max: u64) -> usize {
    let mut k = 0usize;
    for (i, s) in samples.iter().enumerate() {
        if s.acquisitions() > steady_max {
            k = i + 1;
        }
    }
    k
}

// ═══════════════════════════════════════════════════════════════════════════
// Controls (census copies)
// ═══════════════════════════════════════════════════════════════════════════

fn pool(workers: usize) -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(workers).build()
}

/// The copied chunk predicate's literals are the pool's published receipt, and raw install
/// frames price exactly as the classes claim (census `class_predicates_match_the_threadpool_receipt`).
fn class_predicates_match_the_threadpool_receipt() {
    const _: () = assert!(CHUNK0 == boyko_threadpool::__layout_receipt::CHUNK0);
    const _: () = assert!(CHUNK_ALIGN == boyko_threadpool::__layout_receipt::CHUNK_ALIGN);

    const WORKERS: u64 = 2;
    const REPS: usize = 64;
    let p = pool(WORKERS as usize);
    for _ in 0..400 {
        p.install(|s| {
            for _ in 0..4 * WORKERS {
                s.spawn(|| black_box(()));
            }
        });
    }
    let mut empty_other = 0u64;
    let mut one_other = 0u64;
    for i in 0..REPS {
        let before = Snap::now();
        p.install(|_| ());
        let empty = Snap::now().since(before);
        assert_eq!(
            (empty.scope, empty.chunk, empty.inj, empty.realloc),
            (1, 0, 0, 0),
            "rep {i}: an EMPTY `pool.install` must be exactly one `Box<ScopeShared>`"
        );
        empty_other += empty.other + empty.epoch;

        let before = Snap::now();
        p.install(|s| s.spawn(|| black_box(())));
        let one = Snap::now().since(before);
        assert_eq!(
            (one.scope, one.chunk, one.realloc),
            (1, 1, 0),
            "rep {i}: an install with ONE spawn must be one `ScopeShared` plus one {CHUNK0}-byte chunk"
        );
        one_other += one.other + one.epoch;
    }
    say!(
        "\n[class control] {REPS} empty installs: scope=1 chunk=0 on every frame, OTHER+EPOCH total \
         {empty_other} | {REPS} installs + 1 spawn: scope=1 chunk=1 on every frame, OTHER+EPOCH total {one_other}"
    );
    assert!(
        empty_other + one_other <= WORKERS,
        "{} OTHER+EPOCH acquisitions over {} priced install frames on a warmed {WORKERS}-thread pool",
        empty_other + one_other,
        2 * REPS
    );
}

fn counter_is_live() {
    let before = Snap::now();
    let mut v: Vec<u8> = Vec::with_capacity(64);
    v.resize(64, 7);
    v.reserve(1 << 16);
    black_box(&v);
    let len = v.len();
    drop(v);
    let d = Snap::now().since(before);
    assert_eq!(len, 64);
    assert!(d.alloc >= 1, "alloc counter is dead (saw {})", d.alloc);
    assert!(
        d.realloc >= 1,
        "realloc counter is dead (saw {})",
        d.realloc
    );
    assert!(
        d.dealloc >= 1,
        "dealloc counter is dead (saw {})",
        d.dealloc
    );
    assert!(
        d.alloc_bytes >= 64,
        "alloc byte counter is dead (saw {})",
        d.alloc_bytes
    );
    assert!(
        d.realloc_bytes >= 1 << 16,
        "realloc byte counter is dead (saw {})",
        d.realloc_bytes
    );
    say!(
        "\n[anti-vacuity] counter is live: alloc={} realloc={} dealloc={} alloc_bytes={} realloc_bytes={}",
        d.alloc,
        d.realloc,
        d.dealloc,
        d.alloc_bytes,
        d.realloc_bytes
    );
}

static WORKER_ALLOC_RAN_OFF_THREAD: AtomicBool = AtomicBool::new(false);

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct WorkerProbeRow {
    v: u32,
}

#[derive(Bundle)]
struct WorkerProbeBundle {
    r: WorkerProbeRow,
}

/// A system body that allocates on a worker thread is counted (the counter is process-global).
fn worker_thread_allocations_are_counted() {
    let main_id: ThreadId = std::thread::current().id();
    let mut app = App::with_pool(pool(2));
    app.world_mut()
        .spawn_batch((0..64u32).map(|v| WorkerProbeBundle {
            r: WorkerProbeRow { v },
        }))
        .expect("probe rows");
    app.add_systems(move |q: Query<&WorkerProbeRow>| {
        if std::thread::current().id() != main_id {
            WORKER_ALLOC_RAN_OFF_THREAD.store(true, Ordering::Relaxed);
        }
        let mut v: Vec<u8> = Vec::with_capacity(PROBE_BYTES);
        v.resize(PROBE_BYTES, 1);
        black_box(&v);
        black_box(q.iter().count());
    });
    app.finish();
    app.update_with_delta(FRAME);
    let mut seen = 0u64;
    let mut seen_bytes = 0u64;
    for _ in 0..8 {
        let before = Snap::now();
        app.update_with_delta(FRAME);
        let d = Snap::now().since(before);
        seen += d.alloc;
        seen_bytes += d.alloc_bytes;
    }
    assert!(
        WORKER_ALLOC_RAN_OFF_THREAD.load(Ordering::Relaxed),
        "the system body never ran off the driving thread — this probe cannot make its claim"
    );
    assert!(
        seen >= 8,
        "the counter missed worker-thread allocations (saw {seen} over 8 frames)"
    );
    assert!(
        seen_bytes >= 8 * PROBE_BYTES as u64,
        "the counter missed worker-thread BYTES (saw {seen_bytes})"
    );
    say!(
        "\n[anti-vacuity] worker-thread allocations ARE counted: {seen} allocs / {} MiB over 8 frames",
        seen_bytes >> 20
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// First touch
// ═══════════════════════════════════════════════════════════════════════════

/// One round: a leader task (from the injector) opens a nested scope on its worker, pushes
/// `8 × workers` children into that worker's local deque, and spins until other threads have run
/// all of them — so every idle worker takes at least one by a PEER STEAL, which is the event that
/// registers its epoch `Local`.
fn first_touch_round(p: &ThreadPool, workers: usize) {
    let children = 8 * workers;
    p.install(|s| {
        s.spawn(move || {
            let done = AtomicUsize::new(0);
            let leader = std::thread::current().id();
            let taken_by_others = AtomicUsize::new(0);
            p.scope(|inner| {
                for _ in 0..children {
                    let done = &done;
                    let taken_by_others = &taken_by_others;
                    inner.spawn(move || {
                        if std::thread::current().id() != leader {
                            taken_by_others.fetch_add(1, Ordering::Relaxed);
                        }
                        // A little work so a single thief cannot drain the deque alone.
                        let mut x = 0u64;
                        for i in 0..20_000u64 {
                            x = black_box(x.wrapping_add(i));
                        }
                        done.fetch_add(1, Ordering::Release);
                    });
                }
                // Spin (do not join) so the children can only leave this deque by theft; bounded
                // so a pool that never steals cannot hang the test — the join below then runs the
                // rest on this thread.
                let mut spins = 0u64;
                while done.load(Ordering::Acquire) < children && spins < 50_000_000 {
                    std::hint::spin_loop();
                    spins += 1;
                }
            });
            black_box(taken_by_others.load(Ordering::Relaxed));
        });
    });
}

/// Drives `p` until the EPOCH class has counted `workers` `Local`s since `base` — a snapshot taken
/// BEFORE `p`'s threads existed, so the count is exactly theirs — or RED after [`FORCE_ROUNDS`].
/// Returns the rounds taken (0 when every worker had already registered).
fn force_first_touch(p: &ThreadPool, workers: usize, base: Snap) -> usize {
    for round in 0..=FORCE_ROUNDS {
        if Snap::now().since(base).epoch >= workers as u64 {
            return round;
        }
        first_touch_round(p, workers);
    }
    panic!(
        "E1: {FORCE_ROUNDS} forcing rounds did not register one epoch Local per worker ({} of {workers}) — \
         a worker's first touch can still land in the steady window, where it would hide an OTHER \
         regression",
        Snap::now().since(base).epoch
    );
}

/// The EPOCH predicate names a real object: forcing a FRESH W-worker pool registers exactly W
/// `Local`s of exactly [`EPOCH_LOCAL_BYTES`], and nothing classified EPOCH afterwards.
fn epoch_class_matches_a_fresh_pool() {
    const W: usize = 3;
    let base = Snap::now();
    let p = pool(W);
    let rounds = force_first_touch(&p, W, base);
    let after = Snap::now().since(base);
    assert_eq!(
        after.epoch, W as u64,
        "a fresh {W}-worker pool registered {} epoch Locals ({EPOCH_LOCAL_BYTES} B, align \
         {EPOCH_LOCAL_ALIGN}) — the EPOCH predicate does not match one object per worker",
        after.epoch
    );
    let more = Snap::now();
    for _ in 0..16 {
        first_touch_round(&p, W);
    }
    let again = Snap::now().since(more);
    assert_eq!(
        again.epoch, 0,
        "a forced pool registered {} more epoch Locals",
        again.epoch
    );
    say!(
        "\n[epoch control] a fresh {W}-worker pool: {} epoch Local(s) of {EPOCH_LOCAL_BYTES} B after \
         {rounds} forcing round(s), 0 more over 16 further rounds",
        after.epoch
    );
    drop(p);
}

// ═══════════════════════════════════════════════════════════════════════════
// The scene
// ═══════════════════════════════════════════════════════════════════════════

/// `e1_other`: a fresh `Vec` + push every frame — the plan's canary shape, verbatim.
// The lint's `vec![..]` rewrite would be a different canary than the one plan 03 names.
#[allow(clippy::vec_init_then_push)]
fn b2_canary_alloc() {
    let mut v: Vec<u8> = Vec::new();
    v.push(0);
    black_box(&v);
}

/// One steady-window summary line.
struct Row {
    k: usize,
    mean: f64,
    max: u64,
    dispatch_max: u64,
    realloc_sum: u64,
    scope: (u64, u64),
    chunk: (u64, u64),
    inj_max: u64,
    epoch_sum: u64,
    other_sum: u64,
    other_max: u64,
}

/// E1's pinned envelope (the census's `Pin` shape, with no first-touch term — see the header).
struct Pin {
    /// Scope frames on EVERY steady frame, inclusive range.
    scope: (u64, u64),
    /// Chunks on EVERY steady frame, inclusive range.
    chunk: (u64, u64),
    /// Per-frame MAX of every non-OTHER acquisition (dispatch objects plus `realloc`s).
    dispatch_max: u64,
    /// OTHER acquisitions every frame makes BY DESIGN.
    other_per_frame: u64,
    /// `realloc`s over the whole steady window.
    realloc_sum: u64,
}

const RELEASE: bool = !cfg!(debug_assertions);

/// The pins, measured on `u/b2` @ `c1e9f1db` (`stable-x86_64-pc-windows-msvc`, rustc 1.98.1),
/// 2026-09-23, three runs per profile, every run identical frame for frame: a steady frame is one
/// Main install frame and one Fixed substep (a 16 ms frame against the 15.625 ms step carries
/// 0.375 ms a frame, so by arithmetic the first two-substep frame is frame 41, past this window and
/// its two probe frames), each a `ScopeShared` plus one
/// `ScopeBlock` chunk — scope 2, chunk 2 on every steady frame — plus an injector block on every
/// other frame; MAX 5 = dispatch MAX 5; OTHER 0; realloc 0. Debug and release read the same, so one
/// pin set serves both. Headroom is zero both ways: every count here is structural, so a higher
/// reading is a regression to attribute and a lower one re-derives the pin down.
fn pin() -> Pin {
    Pin {
        scope: (2, 2),
        chunk: (2, 2),
        dispatch_max: 5,
        other_per_frame: 0,
        realloc_sum: 0,
    }
}

fn e1_census() {
    counter_is_live();
    class_predicates_match_the_threadpool_receipt();
    worker_thread_allocations_are_counted();
    epoch_class_matches_a_fresh_pool();
    assert_fixture_parses_clean();

    let canary = canary();
    let ui_path = if canary.as_deref() == Some("e1_no_ui") {
        None
    } else {
        Some(UI_FIXTURE)
    };
    let meshes = if canary.as_deref() == Some("e1_no_mesh") {
        0
    } else {
        MESHES
    };
    if let Some(c) = &canary {
        say!("\nCANARY ARMED: {c}");
    }

    // `setup_before` is taken BEFORE the app's pool exists, so every epoch `Local` its workers
    // register — during composition, `finish()` or the forcing below — is counted from here.
    let setup_before = Snap::now();
    let mut app = new_app();
    let after_pool = Snap::now().since(setup_before).epoch;
    app_b(&mut app, ui_path, meshes);
    if canary.as_deref() == Some("e1_other") {
        app.add_systems(b2_canary_alloc);
    }
    let after_compose = Snap::now().since(setup_before).epoch;
    app.finish();
    let setup = Snap::now().since(setup_before);

    // First touch, forced before frame 0 on the app's own pool.
    let pool = Arc::clone(app.pool());
    let force_open = Snap::now();
    let rounds = force_first_touch(&pool, THREADS, setup_before);
    let forced = Snap::now().since(force_open);
    let total_epoch = Snap::now().since(setup_before).epoch;
    say!(
        "\n[first touch] epoch Locals registered since before the app's pool: {after_pool} after the \
         pool, {after_compose} after composition, {} after finish(), {total_epoch} after {rounds} \
         forcing round(s) (OTHER during forcing: {})",
        setup.epoch,
        forced.other
    );
    assert_eq!(
        total_epoch, THREADS as u64,
        "E1: {total_epoch} epoch Locals registered on a {THREADS}-worker app — one per worker is the \
         claim the forcing step rests on"
    );

    let mut samples: Vec<Snap> = Vec::with_capacity(TOTAL_FRAMES);
    let mut instances: Vec<usize> = Vec::with_capacity(TOTAL_FRAMES);
    let mut ui_nodes = 0usize;
    let mut ui_roots = 0usize;
    LOG_ARMED.store(true, Ordering::SeqCst);
    for f in 0..TOTAL_FRAMES {
        CUR_FRAME.store(f as u64, Ordering::SeqCst);
        let before = Snap::now();
        app.update_with_delta(FRAME);
        samples.push(Snap::now().since(before));
        instances.push(app.world().resource::<MeshRenderScratch>().instance_count());
        if f + 1 == WARM_BUDGET {
            // Outside every measured window (the query allocates its result).
            ui_nodes = app
                .world()
                .query_entities(&[UiLayout::component_id()])
                .len();
            ui_roots = app.world().query_entities(&[UiRoot::component_id()]).len();
        }
    }
    LOG_ARMED.store(false, Ordering::SeqCst);

    // In-window liveness (census `liveness_probe`, nested snapshot).
    let window_open = Snap::now();
    app.update_with_delta(FRAME);
    let inner_open = Snap::now();
    let mut v: Vec<u8> = Vec::with_capacity(PROBE_BYTES);
    v.resize(PROBE_BYTES, 0xA5);
    black_box(&v);
    drop(v);
    let inner = Snap::now().since(inner_open);
    let live = Snap::now().since(window_open);
    assert!(
        inner.alloc >= 1 && inner.alloc_bytes >= PROBE_BYTES as u64,
        "ANTI-VACUITY: the counter missed the in-window probe"
    );
    assert!(
        live.alloc_bytes >= PROBE_BYTES as u64,
        "ANTI-VACUITY: the window total missed the probe's bytes"
    );

    let steady = &samples[WARM_BUDGET..];
    let acq = Stat::of(steady, |s| s.acquisitions());
    let row = Row {
        k: settle_index(&samples, acq.max),
        mean: acq.mean(),
        max: acq.max,
        dispatch_max: Stat::of(steady, |s| s.acquisitions() - s.other).max,
        realloc_sum: Stat::of(steady, |s| s.realloc).sum,
        scope: {
            let s = Stat::of(steady, |s| s.scope);
            (s.min, s.max)
        },
        chunk: {
            let s = Stat::of(steady, |s| s.chunk);
            (s.min, s.max)
        },
        inj_max: Stat::of(steady, |s| s.inj).max,
        epoch_sum: Stat::of(steady, |s| s.epoch).sum,
        other_sum: Stat::of(steady, |s| s.other).sum,
        other_max: Stat::of(steady, |s| s.other).max,
    };

    say!("\n╔══════════════════════════════════════════════════════════════════════");
    say!(
        "║ E1 — headless EnginePlugins + UI (app (b)), W = {THREADS}, {} build",
        if RELEASE { "release" } else { "debug" }
    );
    say!(
        "║ SETUP (compose + finish): alloc={} realloc={} bytes={}",
        setup.alloc,
        setup.realloc,
        setup.alloc_bytes + setup.realloc_bytes
    );
    say!("║ per frame: acq realloc bytes | scope chunk injector epoch OTHER | mesh instances");
    for (i, (s, n)) in samples.iter().zip(&instances).enumerate() {
        say!(
            "║   frame {i:>2}{} {:>6} {:>4} {:>9} | {:>4} {:>4} {:>3} {:>3} {:>5} | {n}",
            if i >= WARM_BUDGET { "*" } else { " " },
            s.acquisitions(),
            s.realloc,
            s.alloc_bytes + s.realloc_bytes,
            s.scope,
            s.chunk,
            s.inj,
            s.epoch,
            s.other
        );
    }
    say!(
        "║ (* = steady window, frames {WARM_BUDGET}..={})",
        TOTAL_FRAMES - 1
    );
    say!("║ K = {} (last frame above the steady max, +1)", row.k);
    say!(
        "║ steady: acquisitions mean {:.3} MAX {} | dispatch MAX {} | scope {}..={} chunk {}..={} \
         injector MAX {} | EPOCH {} | OTHER sum {} MAX {} | realloc sum {}",
        row.mean,
        row.max,
        row.dispatch_max,
        row.scope.0,
        row.scope.1,
        row.chunk.0,
        row.chunk.1,
        row.inj_max,
        row.epoch_sum,
        row.other_sum,
        row.other_max,
        row.realloc_sum
    );
    let logged = OTHER_LOG_LEN.load(Ordering::SeqCst).min(OTHER_LOG_CAP);
    say_inline!("║ OTHER objects in the steady window (frame: size/align):");
    let mut any = false;
    for slot in OTHER_LOG.iter().take(logged) {
        let w = slot.load(Ordering::SeqCst);
        let frame = (w >> 48) as usize;
        if frame >= WARM_BUDGET {
            say_inline!(
                " {frame}:{}/{}",
                w & 0xFF_FFFF_FFFF,
                1u64 << ((w >> 40) & 0xFF)
            );
            any = true;
        }
    }
    say!("{}", if any { "" } else { " none" });
    say!(
        "║ UI nodes (UiLayout) after frame {}: {ui_nodes}, UiRoot: {ui_roots}",
        WARM_BUDGET - 1
    );
    say!("║ in-window probe: +{} MiB seen", live.alloc_bytes >> 20);
    say!("╚══════════════════════════════════════════════════════════════════════");
    say!(
        "E1 OTHER: {:.3} per steady frame (target 0)",
        row.other_sum as f64 / STEADY_FRAMES as f64
    );

    gate(&row, &instances[WARM_BUDGET..], ui_nodes, ui_roots);
}

fn gate(row: &Row, steady_instances: &[usize], ui_nodes: usize, ui_roots: usize) {
    let p = pin();
    let mut violations: Vec<String> = Vec::with_capacity(16);
    let mut check = |ok: bool, what: String| {
        if !ok {
            violations.push(what);
        }
    };
    // Anti-vacuity first.
    check(
        ui_nodes > 0 && ui_roots >= 1,
        format!(
            "ANTI-VACUITY: {ui_nodes} UI node(s), {ui_roots} UiRoot(s) after frame {} — the app ran no UI",
            WARM_BUDGET - 1
        ),
    );
    check(
        ui_nodes == 0 || ui_nodes == FIXTURE_NODES,
        format!("ANTI-VACUITY: {ui_nodes} UI nodes, the fixture lowers {FIXTURE_NODES}"),
    );
    for (i, n) in steady_instances.iter().enumerate() {
        check(
            *n > 0,
            format!(
                "ANTI-VACUITY: steady frame {} gathered {n} mesh instances",
                WARM_BUDGET + i
            ),
        );
    }
    // The envelope.
    check(
        row.epoch_sum == 0,
        format!(
            "{} epoch Local(s) registered inside the steady window — first touch was not forced out",
            row.epoch_sum
        ),
    );
    check(
        row.dispatch_max <= p.dispatch_max,
        format!(
            "dispatch MAX {} > pinned {}",
            row.dispatch_max, p.dispatch_max
        ),
    );
    check(
        row.scope.0 >= p.scope.0 && row.scope.1 <= p.scope.1,
        format!(
            "scope frames {}..={} outside pinned {}..={}",
            row.scope.0, row.scope.1, p.scope.0, p.scope.1
        ),
    );
    check(
        row.chunk.0 >= p.chunk.0 && row.chunk.1 <= p.chunk.1,
        format!(
            "chunks {}..={} outside pinned {}..={}",
            row.chunk.0, row.chunk.1, p.chunk.0, p.chunk.1
        ),
    );
    check(
        row.inj_max <= 1,
        format!("{} injector blocks in one frame", row.inj_max),
    );
    let other_budget = p.other_per_frame * STEADY_FRAMES as u64;
    check(
        row.other_sum <= other_budget,
        format!(
            "{} OTHER acquisitions over the window > budget {other_budget}",
            row.other_sum
        ),
    );
    check(
        row.max <= p.dispatch_max.saturating_add(p.other_per_frame),
        format!(
            "steady MAX {} > pinned {}",
            row.max,
            p.dispatch_max.saturating_add(p.other_per_frame)
        ),
    );
    check(
        row.realloc_sum <= p.realloc_sum,
        format!(
            "{} reallocs over the window > pinned {}",
            row.realloc_sum, p.realloc_sum
        ),
    );
    say!(
        "GATE E1 ({}): MAX {} dispatch {} <= {} scope {}..={} in {}..={} chunk {}..={} in {}..={} OTHER {} <= {} \
         EPOCH {} realloc {} <= {}",
        if RELEASE { "release" } else { "debug" },
        row.max,
        row.dispatch_max,
        p.dispatch_max,
        row.scope.0,
        row.scope.1,
        p.scope.0,
        p.scope.1,
        row.chunk.0,
        row.chunk.1,
        p.chunk.0,
        p.chunk.1,
        row.other_sum,
        other_budget,
        row.epoch_sum,
        row.realloc_sum,
        p.realloc_sum
    );
    if violations.is_empty() {
        say!("GATE: GREEN — E1, every class inside its pin");
    } else {
        for v in &violations {
            say!("GATE VIOLATION: {v}");
        }
        panic!(
            "GATE: {} violation(s) of E1's pinned envelope — see the GATE lines above. Pins only fall: \
             an upward move is re-measured and attributed, never widened.",
            violations.len()
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// The runner — `harness = false`, with libtest's command-line contract (census copy)
// ═══════════════════════════════════════════════════════════════════════════

const TEST_NAME: &str = "e1_engine_ui_alloc_census";

#[derive(Default)]
struct Cli {
    list: bool,
    ignored_only: bool,
    exact: bool,
    nocapture: bool,
    show_output: bool,
    filters: Vec<String>,
    skips: Vec<String>,
}

impl Cli {
    fn from_env() -> Self {
        let mut cli = Cli::default();
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--list" => cli.list = true,
                "--ignored" => cli.ignored_only = true,
                "--exact" => cli.exact = true,
                "--nocapture" | "--no-capture" => cli.nocapture = true,
                "--show-output" => cli.show_output = true,
                "--skip" => cli.skips.extend(args.next()),
                "--test-threads" | "--color" | "--format" | "--logfile" | "--shuffle-seed"
                | "-Z" => {
                    let _ = args.next();
                }
                flag if flag.starts_with('-') => {}
                filter => cli.filters.push(filter.to_owned()),
            }
        }
        cli.nocapture |= std::env::var_os("RUST_TEST_NOCAPTURE").is_some_and(|v| v != "0");
        cli
    }

    fn selects(&self, name: &str) -> bool {
        let hit = |pat: &String| {
            if self.exact {
                name == pat
            } else {
                name.contains(pat.as_str())
            }
        };
        !self.ignored_only
            && !self.skips.iter().any(hit)
            && (self.filters.is_empty() || self.filters.iter().any(hit))
    }
}

fn main() {
    let cli = Cli::from_env();
    if cli.list {
        if cli.selects(TEST_NAME) {
            println!("{TEST_NAME}: test");
        }
        return;
    }
    if cfg!(miri) || !cli.selects(TEST_NAME) {
        if cfg!(miri) {
            println!(
                "{TEST_NAME}: not run under Miri — it counts the native allocator over whole app frames"
            );
        }
        println!(
            "\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out\n"
        );
        return;
    }
    STREAM.store(cli.nocapture, Ordering::Relaxed);
    HELD.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .reserve(HELD_RESERVE);
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        dump_held();
        default_hook(info);
    }));
    println!("\nrunning 1 test");
    let passed = std::panic::catch_unwind(e1_census).is_ok();
    if cli.show_output || !passed {
        dump_held();
    }
    if passed {
        println!(
            "test {TEST_NAME} ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
        );
    } else {
        println!(
            "test {TEST_NAME} ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n"
        );
        std::process::exit(101);
    }
}
