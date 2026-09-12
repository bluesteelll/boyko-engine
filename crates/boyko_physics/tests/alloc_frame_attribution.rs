//! **Attribution.** The census ([`alloc_frame_census`](../alloc_frame_census.rs))
//! answers *how many* heap acquisitions a steady-state frame makes; a number
//! nobody can act on is not an answer. This binary answers *whose*, and asserts
//! every attribution claim so a future tree that breaks one reds here instead
//! of silently invalidating the prose.
//!
//! # Tree
//!
//! Written for and measured on `merge/ke16-into-ecsnative` @ `ca582e72`
//! (`feat/ecs-native-storage` with the shipped KE16 thread pool merged in),
//! 2026-09-10, `stable-x86_64-pc-windows-gnu` rustc 1.98.1, release and debug;
//! section G and the non-`Local` capture added and run 2026-09-11 at
//! `d11962a9` — `ca582e72` plus one commit that touches only the physics bench,
//! its off-by-default `bench-alloc` feature and `docs/OPEN-QUESTIONS.md`, so no
//! code on any path this binary runs.
//! A first draft of this file was written against `feat/ecs-native-storage` @
//! `ad0ebea4`, whose pool PREDATES KE16 Stage 3b; its measured shape is kept
//! below as the BEFORE side, because the before/after of Stage 3b was never
//! recorded anywhere else.
//!
//! # Method — five, and each is named at its own site
//!
//! `Schedule::run` exposes **no per-stage seam**: it is one `pool.install`
//! frame whose body is a private executor loop, and a probe system cannot be
//! ordered between two physics stages from outside `boyko_ecs` (`SystemKey`'s
//! module is `pub(crate)`; `PhysicsStageKeys` hands out bare `usize`s). So:
//!
//! 0. **Layout classes** — every acquisition is charged, inside the global
//!    allocator and without allocating, to `scope` (`Box<ScopeShared>`),
//!    `chunk` (a Stage 3b `ScopeBlock` chunk), `inj` (a crossbeam `Injector`
//!    block) or `OTHER`. This is what makes the attribution COMPLETE rather than
//!    sampled: every row below is decomposed, and `OTHER` is what is left.
//! 1. **Primitive pricing** ([`a_dispatch_primitives`]) — `install` / `scope` /
//!    `spawn` / `spawn_batch` called directly, outside any ECS, and every class
//!    count PREDICTED from the threadpool's published layout receipt before it
//!    is compared.
//! 2. **Per-system windows** ([`b_per_system_windows`]) — eight systems, each
//!    snapshotting the counter around its own body, serialised by a shared
//!    `ResMut` so the windows cannot overlap. **This changes behaviour** (the
//!    systems no longer run concurrently); the arm is re-run with plain systems
//!    and the two frame totals are printed side by side — they are equal.
//! 3. **Differential attribution** ([`c_app_subsystem_deltas`],
//!    [`d_physics_deltas`]) — one feature toggled at a time; the physics arms on
//!    ONE warmed world between interleaved A/B/A windows.
//! 4. **Call sites and stages by backtrace** ([`e_call_sites`]) — a capture mode
//!    inside the allocator, `BOYKO_ALLOC_TRACE=1`, NEVER on for any assertion in
//!    this file or in the census gate. Each capture is also charged to a STAGE —
//!    the innermost `FunctionSystem<…>` on its stack, else the executor step —
//!    which is the per-stage seam the schedule does not have. Filters
//!    (`realloc`-only, `OTHER`-only, non-epoch-`Local` `OTHER`) are what make
//!    one-in-thousands sites findable at all.
//! 5. **An `OTHER`-layout log** ([`g_rare_other_layouts`]) — `(frame, size,
//!    align)` of every `OTHER` acquisition, written into a fixed array of
//!    atomics from inside the allocator: no backtrace, so it names the rare
//!    objects under UNDISTURBED timing, which a capture cannot promise (a
//!    capture is slow enough to move a worker's first steal).
//!
//! # What it found (AFTER = `ca582e72`; BEFORE = `ad0ebea4`)
//!
//! **Every steady-state acquisition in every scene is one of four objects,
//! and three of them belong to `boyko_threadpool`.**
//!
//! * **A scope frame is ONE `Box<ScopeShared>` (256 B), plus ONE 4 KiB
//!   `ScopeBlock` chunk the moment it spawns**, plus a doubling chunk each time
//!   its cells overflow (predicted exactly at k = 256 / 257 / 1024 spawns: 1 / 2 /
//!   3 chunks). A spawn itself allocates nothing. BEFORE, the same frame cost
//!   FOUR plus one per spawn: the box, three allocations of a scratch
//!   `crossbeam_deque::Worker` built on every `Scope::drop`, and a heap cell per
//!   task. The frame cost does not move with the pool's width (1 / 2 / 4 / 8).
//! * **An App frame of n systems is therefore 2 whatever n is** (BEFORE: n + 4):
//!   one install frame, one chunk the n cells share, and 1/63 of an injector
//!   block per dispatcher-side push. The eight instrumented system bodies
//!   allocate 0; the executor's residual over `scope + chunk + injector` is 0.
//! * **The event lane, change detection and the whole query path are free**:
//!   +0.000 over an identical 4-system baseline. A `par_iter` fan-out is exactly
//!   +1 scope and +1 chunk (BEFORE: +6).
//! * **A parallel physics step is `1 + passes x colours` scope frames, on
//!   EVERY frame, exactly** (asserted per frame: `scope - 1` is a multiple of
//!   `substeps x (1 + relax)`, and the quotient is the dispatched colour count —
//!   10 on the warmed 1240-body pile over the census's fixed window, so 121 a
//!   step there, and 9 once the pile settles further), with ~1.7 chunks per
//!   scope. **Quote the step as a range, never as 331.8:** over 4,352 steady
//!   steps (the adjudication run, 2026-09-11, reproduced the same day) it read
//!   302..339 per step, with 256-step block means ~307..332 (331.8 in the first
//!   block — the census window itself — 307.5 in the last) on 109..121 scope
//!   frames and 193..217 chunks; 331.797 is the census window's figure, which
//!   is deterministic and therefore repeats to three decimals, and is not the
//!   steady state. By stage, on the one step the backtrace trace charged
//!   (section E, `BOYKO_ALLOC_TRACE=1`): `physics_solve_colored` 99.5 %,
//!   `Schedule::run`'s install frame 0.5 %, every other physics system 0 — and
//!   that step was a WARM-UP step — the 50th of a fresh pile, after 48 serial
//!   warm-up steps and one parallel one — whose own numbers are **145 scope
//!   frames (1 + 12 passes x 12 dispatched colours) and 386 acquisitions**
//!   (re-run 2026-09-11: 386 of 386 captured; the solve's 384 = 144 scope
//!   boxes + 240 chunk grows, the install frame's 2 = 1 + 1). The percentage
//!   belongs to that step. In steady state the install frame is still the same
//!   1 scope + 1 chunk and the solve owns everything else. Worker-side pushes land in the
//!   worker's own Chase-Lev ring and allocate nothing (0.125 injector blocks a
//!   step; BEFORE, 38). `parallel_broadphase` is +0.000 — inert below its body
//!   floor.
//! * **`OTHER` in steady state is exactly one site**: crossbeam-epoch's
//!   `Collector::register`, a pool thread's thread-local `LocalHandle` created on
//!   its first steal (36 of 36 `OTHER` captures across 24 fresh Apps). Once per
//!   thread and front-loaded ([`f_rare_other_is_first_touch`]). **But not
//!   front-loaded into warm-up**: the layout log ([`g_rare_other_layouts`],
//!   2026-09-11, eight runs — five release, three debug — 832 fresh 2-worker
//!   Apps in the census's `S0, 2 systems` shape) put 660 of 1056 epoch `Local`s
//!   at frame 64 or later — INSIDE the census's steady window — never more than
//!   2 per App (one per thread) and never 2 in one frame. So the census gate's
//!   `OTHER` allowance is load-bearing, not decorative. The same log found ONE
//!   other object, 13 times in those 832 Apps, at most once per App and always
//!   before frame 64: 30 bytes, align 2. The non-`Local` capture (`FILTER_RARE`)
//!   caught it on a release run — `std::sys::pal::windows::to_u16s::inner` under
//!   `std::sys::thread::windows::Thread::new::thread_start`: the UTF-16 copy of
//!   the 14-character worker name `boyko-worker-N` (15 units with the NUL) that
//!   std hands `SetThreadDescription` as a new thread starts. A pool worker
//!   that finished BOOTING after setup — `ThreadPoolBuilder::build` does not
//!   wait for its threads to start. In a DEBUG build
//!   the colored pipeline adds one more per step: `debug_assert_coloring`'s
//!   scratch `Vec` in `physics_build_graph` (release: none, asserted by the same
//!   filtered trace coming back empty).
//! * **`Commands` churn was the ONLY growable, and it was UNBOUNDED — fixed by
//!   EM2′.** Before the fix, over 2 048 frames of a flat 4 160-entity
//!   population the recycled-id free list grew by +131 072 on +131 072 despawns
//!   and the id-slot store by the same; not one id was reused. The realloc-only
//!   trace named `CommandQueue::apply` -> `EcsMaster::delete_entity` ->
//!   `RawVec::grow_one`, and the control settled whose defect it was: the SAME
//!   churn through the dispatcher's own `create_entity` / `delete_entity`
//!   recycled perfectly (32 704 despawns left the slot store at 64). Identical
//!   on both trees — the pool was not involved. EM2′ lets `Commands::spawn`
//!   claim from the free list (now a claimable stack on a `VmColumn`), and C6
//!   is inverted to pin the fix: over the same 2 048 frames the free list stays
//!   at one frame's despawns and the slot store does not move.
//!
//! # Coverage boundary — the Rust heap, and only the Rust heap
//!
//! The counter is a `#[global_allocator]` and sees exactly what passes through
//! `GlobalAlloc`. The ECS's column storage does not: a `ComponentPool` (and so
//! every `ScratchColumn` the physics step runs on) reserves with
//! `VirtualAlloc(MEM_RESERVE)` (`mmap` on Unix) and grows by
//! `VmReservation::commit` -> `VirtualAlloc(MEM_COMMIT)`
//! (`boyko_ecs/src/ecs/memory/vm.rs`), and neither is counted. So "0 from every
//! physics buffer", and every other FREE verdict here, means **zero heap
//! acquisitions, not zero memory growth** — a column committing one more
//! granule a frame would still read 0. Flat committed memory is a separate
//! claim that needs a commit-side counter; this binary does not make it.
//!
//! # How it runs
//!
//! `harness = false` (see `Cargo.toml`), for the census's reason: the counter is
//! process-global, and libtest kept a second thread beside the test that
//! allocates on its own schedule — its "has been running for over 60 seconds"
//! notice was measured in a census frame, and it fires on any run that crosses
//! a minute, whatever the machine's load. [`main`] is the whole runner and keeps libtest's command-line
//! contract (name filters, `--list`, `--ignored`, `--nocapture`, exit 101); it
//! runs nothing under Miri, as the `#![cfg(not(miri))]` it replaces did.
//!
//! ```text
//! cargo test -p boyko-physics --release --test alloc_frame_attribution -- --nocapture
//! ```
//!
//! ⚠ Counts only. No wall-clock figure is produced anywhere in this binary.

// No `#![cfg(not(miri))]`: a `harness = false` target must have a `main` in
// every configuration, and a crate-level `cfg` would strip it. `main` returns
// before measuring anything under Miri instead.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::events::event_registry::register_event;
use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;
use boyko_ecs::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use boyko_ecs::ecs::core::iters::query::Changed;
use boyko_ecs::ecs::core::system::Entities;
use boyko_ecs::prelude::*;
use boyko_macros::{Bundle, Component, Resource};

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::{add_physics_colored_solve, add_physics_systems};
use boyko_physics::resources::{Manifolds, PhysicsConfig};
use boyko_physics::solver::SoftStepSolver;

// ═══════════════════════════════════════════════════════════════════════════
// Report output — libtest's capture contract, without libtest
// ═══════════════════════════════════════════════════════════════════════════
//
// Stated independently of the census's copy, like the class predicates below:
// a shared module would let one binary's edit change the other's behaviour.

/// `true` streams the report to stdout as it is produced (`--nocapture`, or
/// `RUST_TEST_NOCAPTURE`); `false` holds it in [`HELD`] and [`main`] prints it
/// only on failure or with `--show-output`, as libtest's capture did.
static STREAM: AtomicBool = AtomicBool::new(false);
/// The held report, reserved by [`main`] before the first window so that
/// appending does not grow it during the run.
static HELD: Mutex<String> = Mutex::new(String::new());
/// Larger than the report of a default run; a `BOYKO_ALLOC_TRACE=1` run may
/// outgrow it, which only its (never-asserted) trace tables would see.
const HELD_RESERVE: usize = 1 << 20;

/// The sink behind [`say!`] and [`say_inline!`].
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

/// Prints the held report, if any, and empties it.
fn dump_held() {
    // `try_lock`: this also runs from the panic hook, on whatever thread
    // panicked; a report that cannot be taken there is dumped by `main` later.
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

/// `println!` for the report — streamed or held, per [`STREAM`].
macro_rules! say {
    () => {
        emit(format_args!(""), true)
    };
    ($($arg:tt)*) => {
        emit(format_args!($($arg)*), true)
    };
}

/// `print!` for the report — streamed or held, per [`STREAM`].
macro_rules! say_inline {
    ($($arg:tt)*) => {
        emit(format_args!($($arg)*), false)
    };
}

// ═══════════════════════════════════════════════════════════════════════════
// The counting global allocator: layout classes, plus an opt-in capture mode
// ═══════════════════════════════════════════════════════════════════════════

/// Process-global counters, always on. Same shape as the census's, and
/// process-global for the same reason: half of a frame's allocations are made
/// on worker threads, which a thread-local counter cannot see.
static N_ALLOC: AtomicU64 = AtomicU64::new(0);
static N_REALLOC: AtomicU64 = AtomicU64::new(0);
static N_DEALLOC: AtomicU64 = AtomicU64::new(0);
static B_ALLOC: AtomicU64 = AtomicU64::new(0);
static B_REALLOC: AtomicU64 = AtomicU64::new(0);

// ── Layout classes — method 0, and the one every other method is checked by ──
//
// Every fresh acquisition is charged to exactly one class from its `Layout`
// alone: no backtrace, no allocation, two compares. The predicates are the
// census gate's, stated independently here on purpose (a shared module would
// let one binary's edit move the other's pins), and each is positively
// controlled in section A by pricing the primitive that produces it:
//
// * SCOPE — `align == 128 && size == 256`: the `Box<ScopeShared>` of every
//   `pool.install` / `pool.scope` (`thread_pool.rs`). `ScopeShared` is
//   `#[repr(C)]`, leads with a `CachePadded<AtomicUsize>` and carries three
//   8-byte words behind it: 152 bytes, rounded to the alignment, 256.
// * CHUNK — `align == 64 && size.is_power_of_two() && size >= 4096`: a Stage 3b
//   `ScopeBlock` chunk (`block.rs`'s `grow`, the only producer of that shape).
// * INJ — `size == 1520 && align == 8`: a `crossbeam_deque::Injector` block,
//   `8 + 63 * (16-byte Task + 8-byte state)`.
// * OTHER — everything else, INCLUDING an over-aligned allocation that is not
//   exactly a `ScopeShared` (crossbeam-epoch's per-thread `Local` carries a
//   `CachePadded` epoch and is align 128 too; it must not be counted as a scope).
static C_SCOPE: AtomicU64 = AtomicU64::new(0);
static C_CHUNK: AtomicU64 = AtomicU64::new(0);
static C_INJ: AtomicU64 = AtomicU64::new(0);
static C_OTHER: AtomicU64 = AtomicU64::new(0);

const CHUNK0: usize = 4096;
const CHUNK_ALIGN: usize = 64;
const INJECTOR_BLOCK_BYTES: usize = 8 + 63 * 24;
const SCOPE_SHARED_BYTES: usize = 256;
const SCOPE_SHARED_ALIGN: usize = 128;
/// Pushes per `Injector` block (`crossbeam_deque`'s `BLOCK_CAP`).
const INJECTOR_BLOCK_CAP: f64 = 63.0;

/// The layout class of one fresh acquisition.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Scope,
    Chunk,
    Inj,
    Other,
}

#[inline]
fn classify(layout: Layout) -> Class {
    let (size, align) = (layout.size(), layout.align());
    let (class, counter) = if align == SCOPE_SHARED_ALIGN && size == SCOPE_SHARED_BYTES {
        (Class::Scope, &C_SCOPE)
    } else if align == CHUNK_ALIGN && size.is_power_of_two() && size >= CHUNK0 {
        (Class::Chunk, &C_CHUNK)
    } else if size == INJECTOR_BLOCK_BYTES && align == 8 {
        (Class::Inj, &C_INJ)
    } else {
        (Class::Other, &C_OTHER)
    };
    counter.fetch_add(1, Ordering::Relaxed);
    if class == Class::Other && OTHER_LOG_ON.load(Ordering::Relaxed) {
        log_other(size, align);
    }
    class
}

// ── The OTHER-layout log: WHAT a rare OTHER acquisition is, without a backtrace ──
//
// The census saw, about one window in seven, a 2-worker App frame carrying THREE
// `OTHER` acquisitions where the per-thread first touch explains two, and the
// backtrace table never caught the third: capture is slow enough to move a
// worker's first steal, and the event is timing-dependent. This log records the
// `(frame, size, align)` of every `OTHER` acquisition into a fixed array of
// atomics — no allocation, no symbolisation, two relaxed RMWs — so the rare
// object is at least NAMED by its layout under undisturbed timing, and the
// size-filtered capture ([`FILTER_RARE`]) can then be aimed at exactly it.

/// Entries the log holds; later ones are counted in [`OTHER_LOG_LEN`] but dropped.
const OTHER_LOG_CAP: usize = 512;
/// Armed only by section G, never during any other window.
static OTHER_LOG_ON: AtomicBool = AtomicBool::new(false);
/// The frame index the driver is currently in, stamped into every entry.
static OTHER_LOG_FRAME: AtomicU64 = AtomicU64::new(0);
static OTHER_LOG_LEN: AtomicUsize = AtomicUsize::new(0);
/// `frame << 40 | size << 8 | log2(align)`.
static OTHER_LOG: [AtomicU64; OTHER_LOG_CAP] = [const { AtomicU64::new(0) }; OTHER_LOG_CAP];

#[cold]
#[inline(never)]
fn log_other(size: usize, align: usize) {
    let i = OTHER_LOG_LEN.fetch_add(1, Ordering::Relaxed);
    if i < OTHER_LOG_CAP {
        let frame = OTHER_LOG_FRAME.load(Ordering::Relaxed);
        OTHER_LOG[i].store(
            (frame << 40) | ((size as u64 & 0xFFFF_FFFF) << 8) | align.trailing_zeros() as u64,
            Ordering::Relaxed,
        );
    }
}

/// crossbeam-epoch's per-thread `Local` — the one `OTHER` object the backtrace
/// table names (`Collector::register`, 2304 B, align 128).
const EPOCH_LOCAL_BYTES: usize = 2304;
/// std's `to_u16s` copy of a worker's name, made in `thread_start` for
/// `SetThreadDescription`: `boyko-worker-N` is 14 characters, plus the NUL, as
/// `u16`s (`ThreadPoolBuilder`'s default prefix; single-digit worker index).
const THREAD_NAME_UTF16_BYTES: usize = 2 * ("boyko-worker-0".len() + 1);

/// Capture arming. `false` for every measured window and for the whole run
/// unless `BOYKO_ALLOC_TRACE=1`.
static TRACE_ON: AtomicBool = AtomicBool::new(false);
/// Remaining captures. Bounded so a scene that allocates freely cannot turn a
/// diagnostic into an unbounded symbolisation run.
static TRACE_BUDGET: AtomicU64 = AtomicU64::new(0);
/// Allocations seen by an armed window that passed its filter, whether or not
/// they were captured — the denominator the site table's coverage is quoted
/// against.
static TRACE_SEEN: AtomicU64 = AtomicU64::new(0);
/// What an armed window captures. A filter is what makes a RARE site findable
/// at all: a `realloc`, or an `OTHER`-class acquisition, is one event in
/// thousands, and a fresh-allocation budget big enough to reach one would
/// symbolise for minutes.
static TRACE_FILTER: AtomicU8 = AtomicU8::new(FILTER_ALL);
/// Every fresh acquisition and every `realloc`.
const FILTER_ALL: u8 = 0;
/// `realloc` only — the `Vec`-growth signature.
const FILTER_REALLOC: u8 = 1;
/// Fresh acquisitions of the `OTHER` class only — everything that is not a
/// dispatch object.
const FILTER_OTHER: u8 = 2;
/// `OTHER`-class acquisitions that are NOT crossbeam-epoch's per-thread `Local`
/// — the capture is paid only on the rare object itself, so arming it barely
/// moves the timing the object depends on.
const FILTER_RARE: u8 = 3;

thread_local! {
    /// Re-entrancy guard. Capturing a backtrace ALLOCATES; without this the
    /// first capture would recurse into itself forever.
    static IN_TRACE: Cell<bool> = const { Cell::new(false) };
}

/// Aggregated capture sites: `(folded key, hits, bytes)`. A sorted `Vec` of
/// pairs rather than a map — the workspace bans `HashMap`, and the table is
/// short enough that a linear scan is the whole structure.
static SITES: Mutex<Vec<(String, u64, u64)>> = Mutex::new(Vec::new());
/// The same captures tallied by STAGE — the innermost system (or executor
/// step) on the captured stack: `(stage, hits, bytes)`.
static STAGES: Mutex<Vec<(String, u64, u64)>> = Mutex::new(Vec::new());
/// The first capture, verbatim and unfolded, so a reader can judge for himself
/// how much the folding threw away and whether symbols resolved at all.
static FIRST_RAW: Mutex<String> = Mutex::new(String::new());

/// Delegates to `System` and records. `realloc` is counted on its own axis: it
/// is the signature of a `Vec` growing, which is the shape the arena exists to
/// remove, and the deliverable has to tell it apart from a fresh acquisition.
struct CountingAlloc;

// SAFETY: pure delegation to `System`; every layout / pointer contract is
// forwarded unchanged. The counters are `static` atomics and never allocate.
// The capture path CAN allocate, which is exactly why it is behind a
// thread-local re-entrancy guard — the nested allocation is counted but not
// captured, so recursion terminates at depth one.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        N_ALLOC.fetch_add(1, Ordering::Relaxed);
        B_ALLOC.fetch_add(layout.size() as u64, Ordering::Relaxed);
        let class = classify(layout);
        if TRACE_ON.load(Ordering::Relaxed) {
            fresh_capture(class, layout.size() as u64);
        }
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        N_ALLOC.fetch_add(1, Ordering::Relaxed);
        B_ALLOC.fetch_add(layout.size() as u64, Ordering::Relaxed);
        let class = classify(layout);
        if TRACE_ON.load(Ordering::Relaxed) {
            fresh_capture(class, layout.size() as u64);
        }
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        N_REALLOC.fetch_add(1, Ordering::Relaxed);
        B_REALLOC.fetch_add(new_size as u64, Ordering::Relaxed);
        if TRACE_ON.load(Ordering::Relaxed) {
            let f = TRACE_FILTER.load(Ordering::Relaxed);
            if f == FILTER_ALL || f == FILTER_REALLOC {
                capture(new_size as u64);
            }
        }
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        N_DEALLOC.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

/// How many frames of a capture are kept in a folded key.
const KEEP_FRAMES: usize = 10;

/// Applies the armed filter to one fresh acquisition.
#[cold]
#[inline(never)]
fn fresh_capture(class: Class, size: u64) {
    let f = TRACE_FILTER.load(Ordering::Relaxed);
    if f == FILTER_ALL
        || (f == FILTER_OTHER && class == Class::Other)
        || (f == FILTER_RARE && class == Class::Other && size != EPOCH_LOCAL_BYTES as u64)
    {
        capture(size);
    }
}

/// Records one allocation's backtrace into [`SITES`] and [`STAGES`].
///
/// `#[cold]` and `#[inline(never)]`: it is off in every measured window and in
/// the whole shipping-shaped run, so it must not cost the hot path a byte of
/// I-cache.
#[cold]
#[inline(never)]
fn capture(size: u64) {
    // `try_with`, not `with`: this runs inside the global allocator, and an
    // allocation made during a thread's TLS teardown would panic on `with`.
    // A capture lost to teardown is a lost sample, not a failed run.
    let _ = IN_TRACE.try_with(|guard| {
        if guard.get() {
            return;
        }
        TRACE_SEEN.fetch_add(1, Ordering::Relaxed);
        if TRACE_BUDGET.load(Ordering::Relaxed) == 0 {
            return;
        }
        TRACE_BUDGET.fetch_sub(1, Ordering::Relaxed);
        guard.set(true);

        let raw = std::backtrace::Backtrace::force_capture().to_string();
        let key = fold(&raw);
        let stage = stage_of(&raw);
        if let Ok(mut first) = FIRST_RAW.lock()
            && first.is_empty()
        {
            *first = raw;
        }
        if let Ok(mut sites) = SITES.lock() {
            match sites.iter_mut().find(|(k, _, _)| *k == key) {
                Some(entry) => {
                    entry.1 += 1;
                    entry.2 += size;
                }
                None => sites.push((key, 1, size)),
            }
        }
        if let Ok(mut stages) = STAGES.lock() {
            match stages.iter_mut().find(|(k, _, _)| *k == stage) {
                Some(entry) => {
                    entry.1 += 1;
                    entry.2 += size;
                }
                None => stages.push((stage, 1, size)),
            }
        }

        guard.set(false);
    });
}

/// The STAGE a captured stack belongs to: the innermost ECS system on it, else
/// the innermost executor / frame-funnel step.
///
/// This is the per-stage seam the `Schedule` does not expose. `Schedule::run`
/// is one `pool.install` whose body is a private executor loop; `SystemKey`'s
/// module is `pub(crate)`, so a probe system cannot be ORDERED between two
/// physics stages from outside `boyko_ecs` (`PhysicsStageKeys` hands out bare
/// `usize`s that nothing outside the kernel can turn back into a key). The
/// stack, however, names the system: every system body runs under
/// `FunctionSystem<path::to::system_fn, …>::run_unsafe`, so the innermost such
/// frame IS the stage. Lines are scanned innermost-first.
fn stage_of(raw: &str) -> String {
    for line in raw.lines() {
        if let Some(at) = line.find("FunctionSystem<") {
            let rest = &line[at + "FunctionSystem<".len()..];
            let path = rest.split([',', '>']).next().unwrap_or(rest);
            let name = path.rsplit("::").next().unwrap_or(path);
            return format!("system `{name}`");
        }
        if line.contains("CommandQueue") {
            return "executor: CommandQueue apply (deferred Commands)".to_string();
        }
        if line.contains("Schedule>::run") {
            return "executor: Schedule::run (install frame / dispatch)".to_string();
        }
        if line.contains("App>::update") {
            return "App::update_with_delta, outside Schedule::run".to_string();
        }
        if line.contains("worker_main") || line.contains("thread_start") {
            return "pool worker, outside any system (steal / park / TLS)".to_string();
        }
    }
    "unattributed (no system, executor or worker frame on the stack)".to_string()
}

/// Folds a rendered backtrace to the first [`KEEP_FRAMES`] frames that are not
/// the instrument itself.
///
/// The skipped prefix is the capture machinery and the allocator shim — frames
/// that are identical for EVERY sample and would make every key equal on its
/// first line. Everything after the first kept frame is kept verbatim,
/// including `std` frames, because "which `Vec` grew" is often only legible
/// from the `alloc::raw_vec` frame above the caller.
fn fold(raw: &str) -> String {
    /// Substrings that mark a frame as belonging to the instrument.
    const INSTRUMENT: [&str; 7] = [
        "backtrace",
        "CountingAlloc",
        "alloc_frame_attribution::capture",
        "alloc_frame_attribution::fresh_capture",
        "alloc_frame_attribution::fold",
        "__rust_alloc",
        "__rg_",
    ];
    let mut out = String::with_capacity(512);
    let mut kept = 0usize;
    let mut started = false;
    for line in raw.lines() {
        let t = line.trim();
        // Frame lines are `N: symbol`; the `at path:line` continuations are
        // dropped, since a release build rarely has them and their presence
        // would make two builds of the same tree produce two keys.
        let Some((idx, sym)) = t.split_once(": ") else {
            continue;
        };
        if !idx.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        // ⚠ The skip list is EXACTLY the instrument, and nothing heuristic.
        // The first form also skipped any frame containing `alloc::` or
        // `core::` on the theory that those were std plumbing — and every path
        // in this engine contains `ecs::core::`, so it silently discarded
        // `Schedule::run` and `App::update_with_delta` and folded three
        // different sites onto one meaningless key. A prefix filter that can
        // eat the answer is worse than no filter: the real instrument frames
        // are named, so name them.
        let is_instrument = INSTRUMENT.iter().any(|m| sym.contains(m));
        if !started {
            if is_instrument {
                continue;
            }
            started = true;
        }
        out.push_str(sym);
        out.push('\n');
        kept += 1;
        if kept >= KEEP_FRAMES {
            break;
        }
    }
    if out.is_empty() {
        out.push_str("<no symbolisable frame>\n");
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Measuring
// ═══════════════════════════════════════════════════════════════════════════

/// One reading of the counters.
#[derive(Clone, Copy, Default)]
struct Snap {
    alloc: u64,
    realloc: u64,
    bytes: u64,
    /// Bytes requested by `realloc` ALONE, on its own axis: the churn probe has
    /// to say how big the growing buffer is, and a window total cannot.
    realloc_bytes: u64,
    /// Fresh acquisitions by class; `scope + chunk + inj + other == alloc`.
    scope: u64,
    chunk: u64,
    inj: u64,
    other: u64,
}

impl Snap {
    fn now() -> Self {
        Snap {
            alloc: N_ALLOC.load(Ordering::SeqCst),
            realloc: N_REALLOC.load(Ordering::SeqCst),
            bytes: B_ALLOC.load(Ordering::SeqCst) + B_REALLOC.load(Ordering::SeqCst),
            realloc_bytes: B_REALLOC.load(Ordering::SeqCst),
            scope: C_SCOPE.load(Ordering::SeqCst),
            chunk: C_CHUNK.load(Ordering::SeqCst),
            inj: C_INJ.load(Ordering::SeqCst),
            other: C_OTHER.load(Ordering::SeqCst),
        }
    }
    fn since(self, base: Snap) -> Snap {
        Snap {
            alloc: self.alloc - base.alloc,
            realloc: self.realloc - base.realloc,
            bytes: self.bytes - base.bytes,
            realloc_bytes: self.realloc_bytes - base.realloc_bytes,
            scope: self.scope - base.scope,
            chunk: self.chunk - base.chunk,
            inj: self.inj - base.inj,
            other: self.other - base.other,
        }
    }
    /// Heap ACQUISITIONS: a realloc is an acquisition too.
    fn acq(self) -> u64 {
        self.alloc + self.realloc
    }
}

/// The summary of one measured window.
#[derive(Clone, Copy, Default)]
struct Meas {
    mean: f64,
    min: u64,
    max: u64,
    realloc_max: u64,
    bytes_mean: f64,
    /// Per-class means over the window.
    scope: f64,
    chunk: f64,
    inj: f64,
    other: f64,
    /// The worst single frame's `OTHER` count.
    other_max: u64,
}

/// Warms `warm` repetitions, then records `n` per-repetition readings.
///
/// The sample buffer is pre-sized: a `Vec` growing mid-window would be an
/// allocation the instrument itself made inside its own measurement.
fn frames<F: FnMut()>(warm: usize, n: usize, mut f: F) -> Vec<Snap> {
    for _ in 0..warm {
        f();
    }
    let mut out: Vec<Snap> = Vec::with_capacity(n);
    for _ in 0..n {
        let before = Snap::now();
        f();
        out.push(Snap::now().since(before));
    }
    out
}

/// Summarises a run of per-repetition readings.
fn summarise(v: &[Snap]) -> Meas {
    let n = v.len() as f64;
    let mean_of = |pick: fn(&Snap) -> u64| v.iter().map(pick).sum::<u64>() as f64 / n;
    Meas {
        mean: mean_of(|s| s.acq()),
        min: v.iter().map(|s| s.acq()).min().expect("invariant: n >= 1"),
        max: v.iter().map(|s| s.acq()).max().expect("invariant: n >= 1"),
        realloc_max: v
            .iter()
            .map(|s| s.realloc)
            .max()
            .expect("invariant: n >= 1"),
        bytes_mean: mean_of(|s| s.bytes),
        scope: mean_of(|s| s.scope),
        chunk: mean_of(|s| s.chunk),
        inj: mean_of(|s| s.inj),
        other: mean_of(|s| s.other),
        other_max: v.iter().map(|s| s.other).max().expect("invariant: n >= 1"),
    }
}

/// Warms `warm` repetitions, then measures `n` and summarises them.
fn window<F: FnMut()>(warm: usize, n: usize, f: F) -> Meas {
    summarise(&frames(warm, n, f))
}

/// Warm-up repetitions for the cheap primitive arms.
const WARM: usize = 32;
/// Measured repetitions for the cheap primitive arms.
const REPS: usize = 128;
/// Warm-up frames for a physics arm (the pile must reach its resting shape).
const PHYS_WARM: usize = 48;
/// Measured frames per physics window. Deliberately modest: this is a count,
/// and the machine it runs on is a workstation with other work on it.
const PHYS_REPS: usize = 24;

/// One row of the closing attribution table.
struct Row {
    section: &'static str,
    what: String,
    m: Meas,
    verdict: &'static str,
}

/// The classes the deliverable asks every site to be sorted into.
mod verdict {
    /// Genuinely transient scratch: amortised, bounded, and not this engine's
    /// to remove.
    pub const TRANSIENT: &str = "transient / amortised — bounded, queue-internal";
    /// Structural per-dispatch allocation that should not exist at all.
    pub const SHOULD_NOT: &str = "SHOULD NOT ALLOCATE — per-scope dispatch object";
    /// Measured to cost nothing; listed so the zero is on the record.
    pub const FREE: &str = "free (measured zero)";
}

fn push(rows: &mut Vec<Row>, section: &'static str, what: String, m: Meas, verdict: &'static str) {
    say!(
        "  {:<58} mean={:>9.3} min={:<5} max={:<5} | scope={:>7.3} chunk={:>7.3} inj={:>6.3} \
         OTHER={:>6.3} (max {}) realloc_max={} bytes={:>9.0}",
        what,
        m.mean,
        m.min,
        m.max,
        m.scope,
        m.chunk,
        m.inj,
        m.other,
        m.other_max,
        m.realloc_max,
        m.bytes_mean
    );
    rows.push(Row {
        section,
        what,
        m,
        verdict,
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// Fixtures shared with the census (deliberately duplicated, see below)
// ═══════════════════════════════════════════════════════════════════════════
//
// The pyramid builder and the payload components are copied from
// `alloc_frame_census.rs` rather than hoisted into a `tests/common/` module.
// A shared module would make one binary's edit able to move the OTHER binary's
// pinned gate numbers without touching the file that carries them, which is the
// exact shape this repository catalogues as "the gate moved and nobody saw it".
// The duplication is checked by both binaries measuring the same scene and
// agreeing.

/// Payload row. 4096 of them, so `par_iter` clears `MIN_ARCHETYPE_FOR_PARALLEL`.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Payload {
    v: u32,
}

#[derive(Bundle)]
struct PayloadBundle {
    p: Payload,
}

/// The churn marker, carrying the generation it was spawned in.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Doomed {
    wave: u32,
}

#[derive(Bundle)]
struct DoomedBundle {
    d: Doomed,
}

#[derive(Resource, Default)]
struct Churn {
    generation: u32,
    spawned: u64,
}

#[derive(Resource, Default)]
struct Tally {
    hits: u64,
    sent: u64,
    received: u64,
    changed: u64,
}

/// Rows seeded into the payload archetype.
const ROWS: usize = 4096;
/// Entities spawned and despawned every churn frame.
const CHURN_PER_FRAME: usize = 64;
/// Events sent every frame in the event arm.
const EVENTS_PER_FRAME: u32 = 32;

fn pool(workers: usize) -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(workers).build()
}

fn seed_payload(world: &mut EcsMaster, rows: usize) {
    world
        .spawn_batch((0..rows as u32).map(|v| PayloadBundle { p: Payload { v } }))
        .expect("seed payload rows");
}

// ═══════════════════════════════════════════════════════════════════════════
// A — primitive pricing: what a dispatch object costs, outside any ECS
// ═══════════════════════════════════════════════════════════════════════════

/// `ScopeBlock` chunks a scope needs to hold `k` cells of `cell` bytes: chunk
/// `i` is `CHUNK0 << i` bytes (`block.rs`'s `grow`, `e = max(i, min_e)`), and a
/// cell never straddles two chunks. Derived from the published layout receipt,
/// never from a measurement of the thing it predicts.
fn chunks_for(k: usize, cell: usize) -> u64 {
    let mut held = 0usize;
    let mut n = 0u64;
    while held < k {
        held += (CHUNK0 << n) / cell;
        n += 1;
    }
    n
}

/// A pool whose threads have each been through their first steal.
///
/// A fresh pool's worker threads finish booting ASYNCHRONOUSLY: a worker's
/// first-touch registrations (section F, and the `OTHER`-only trace table)
/// happen whenever that thread first gets work, and the process-global counter
/// charges them to whichever window is open at the time. Four hundred spawning
/// frames give every lane work many times over before anything is priced.
fn warmed_pool(workers: usize) -> Arc<ThreadPool> {
    let p = pool(workers);
    for _ in 0..400 {
        p.install(|s| {
            for _ in 0..4 * workers {
                s.spawn(|| {
                    black_box(0u32);
                });
            }
        });
    }
    p
}

/// The `OTHER` allowance of a primitive window: at most two acquisitions in
/// the WHOLE window and never two in one frame. That is the shape of a late
/// first touch landing inside the window (section F tests that it is not a
/// rate); anything a primitive allocated on every call would show `REPS` of
/// them.
fn first_touch_only(m: &Meas) -> bool {
    m.other * REPS as f64 <= 2.0 + 1e-9 && m.other_max <= 1
}

/// What the two primitive prices came out at, for the sections that build on
/// them.
struct Prim {
    /// An install frame that spawns nothing: one `ScopeShared`.
    empty_frame: f64,
    /// An install frame that spawns at least one task (and at most one chunk's
    /// worth): one `ScopeShared` plus one 4 KiB chunk.
    spawning_frame: f64,
}

/// Prices the threadpool's public primitives directly. This is what turns the
/// census's measured per-frame figures into named objects: it needs no
/// assumption about the executor, because it never enters one.
fn a_dispatch_primitives(rows: &mut Vec<Row>) -> Prim {
    say!("\n── A. DISPATCH PRIMITIVES (threadpool called directly, no ECS) ──");

    assert_eq!(CHUNK0, boyko_threadpool::__layout_receipt::CHUNK0);
    assert_eq!(CHUNK_ALIGN, boyko_threadpool::__layout_receipt::CHUNK_ALIGN);

    let p4 = warmed_pool(4);

    // An `install` frame with an empty body.
    let empty = window(WARM, REPS, || p4.install(|_| ()));
    push(
        rows,
        "A",
        "pool.install(|_| ()) — the bare install frame".to_string(),
        empty,
        verdict::SHOULD_NOT,
    );
    // ⚠ THE PRIOR THIS REPLACES. On the pre-Stage-3b pool (`feat/ecs-native-
    // storage` @ `ad0ebea4`) this same frame measured FOUR: the box plus three
    // allocations of a scratch `crossbeam_deque::Worker` that
    // `join_workers_until_drained` built on every `Scope::drop`. KE16's join
    // (`join_on_worker` / `join_external`) builds no deque, so the three are gone
    // and the frame is the box alone — asserted by CLASS, so a new allocation in
    // the frame reds here even if it happened to keep the total at one.
    assert!(
        (empty.scope - 1.0).abs() < 0.01
            && empty.chunk == 0.0
            && empty.inj == 0.0
            && first_touch_only(&empty)
            && empty.realloc_max == 0,
        "A: an empty install frame measured scope={:.3} chunk={:.3} inj={:.3} OTHER={:.3} — it \
         must be exactly one `Box<ScopeShared>` and nothing else",
        empty.scope,
        empty.chunk,
        empty.inj,
        empty.other
    );

    // The spawn sweep. The body is a zero-sized closure, so a scoped cell is
    // exactly the published `SCOPED_CELL_HEADER`; the chunk count at every `k`
    // is PREDICTED from the layout receipt and then compared.
    let body = || {
        black_box(0u32);
    };
    let cell = boyko_threadpool::__layout_receipt::SCOPED_CELL_HEADER + size_of_val(&body);
    assert_eq!(
        cell % boyko_threadpool::__layout_receipt::SCOPED_CELL_ALIGN,
        0,
        "A: the cell stride derivation assumes a header-aligned zero-sized body"
    );
    say!(
        "  (a scoped cell here is {cell} bytes; a 4 KiB chunk holds {})",
        CHUNK0 / cell
    );
    let mut spawning_frame = 0.0f64;
    for k in [0usize, 1, 2, 4, 8, 16, 32, 64, 128, 256, 257, 512, 1024] {
        let m = window(WARM, REPS, || {
            p4.install(|scope| {
                for _ in 0..k {
                    scope.spawn(body);
                }
            })
        });
        if k == 1 {
            spawning_frame = m.mean;
        }
        let want_chunks = chunks_for(k, cell) as f64;
        let want_inj = k as f64 / INJECTOR_BLOCK_CAP;
        push(
            rows,
            "A",
            format!("pool.install + {k} × Scope::spawn (predict {want_chunks} chunk(s))"),
            m,
            verdict::SHOULD_NOT,
        );
        assert!(
            (m.scope - 1.0).abs() < 0.01,
            "A: k={k}: {:.3} scope boxes per install frame, not one",
            m.scope
        );
        assert!(
            (m.chunk - want_chunks).abs() < 0.01,
            "A: k={k}: {:.3} ScopeBlock chunks per frame; the layout receipt predicts \
             {want_chunks} for {k} cells of {cell} bytes — Stage 3b's growth rule or its cell \
             stride has moved",
            m.chunk
        );
        // Every push from the dispatcher lands in the global injector, which takes
        // one block per `BLOCK_CAP` pushes — so the block rate is the push rate
        // over 63, and a figure off by more than one block per window's frame
        // means a push route has changed.
        assert!(
            (m.inj - want_inj).abs() < 1.0,
            "A: k={k}: {:.3} injector blocks per frame against {want_inj:.3} predicted from \
             one block per 63 pushes",
            m.inj
        );
        assert!(
            first_touch_only(&m),
            "A: k={k}: {:.3} OTHER-class acquisitions per frame — a spawn has begun allocating \
             something that is neither a cell chunk nor a queue block",
            m.other
        );
    }

    // `spawn_batch` — measured so the claim that it allocates what N spawns
    // allocate is a measurement and not a reading of the source.
    let batch16 = window(WARM, REPS, || {
        p4.install(|scope| {
            scope.spawn_batch(16, (0..16).map(|_| body));
        })
    });
    push(
        rows,
        "A",
        "pool.install + Scope::spawn_batch(16) — one wave".to_string(),
        batch16,
        verdict::SHOULD_NOT,
    );
    assert!(
        (batch16.scope - 1.0).abs() < 0.01 && (batch16.chunk - 1.0).abs() < 0.01,
        "A: spawn_batch(16) measured scope={:.3} chunk={:.3}; it must cost what 16 spawns cost \
         — one box and one chunk",
        batch16.scope,
        batch16.chunk
    );

    // ── Is the frame cost per-JOIN or per-WORKER? ──
    //
    // Both objects belong to the scope, not to the pool, so neither may move
    // with the pool's width. A figure that grew with the width would be a
    // per-worker structure and a different fix.
    let mut width_points: Vec<(usize, Meas)> = Vec::with_capacity(4);
    for w in [1usize, 2, 4, 8] {
        let p = warmed_pool(w);
        let m = window(WARM, REPS, || p.install(|s| s.spawn(body)));
        width_points.push((w, m));
        push(
            rows,
            "A",
            format!("pool.install + 1 spawn on a {w}-worker pool"),
            m,
            verdict::SHOULD_NOT,
        );
    }
    for (w, m) in &width_points {
        assert!(
            (m.scope - 1.0).abs() < 0.01 && (m.chunk - 1.0).abs() < 0.01,
            "A: on a {w}-worker pool an install frame with one spawn measured scope={:.3} \
             chunk={:.3} — the frame cost moved with the pool's width",
            m.scope,
            m.chunk
        );
    }

    // A nested `scope` inside an install frame — the shape a system body takes
    // when it calls `par_iter` or the colored solver's `pool.scope`. An EMPTY
    // nested scope takes a box and no chunk (the block is lazy); a nested scope
    // that spawns takes both. That equality is what makes the physics figure
    // predictable from the scope count in section D.
    let nested_empty = window(WARM, REPS, || {
        p4.install(|_| {
            p4.scope(|_| ());
        })
    });
    push(
        rows,
        "A",
        "pool.install { pool.scope(|_| ()) } — an empty nested scope".to_string(),
        nested_empty,
        verdict::SHOULD_NOT,
    );
    let nested_one = window(WARM, REPS, || {
        p4.install(|_| {
            p4.scope(|s| s.spawn(body));
        })
    });
    push(
        rows,
        "A",
        "pool.install { pool.scope(|s| 1 spawn) } — a spawning nested scope".to_string(),
        nested_one,
        verdict::SHOULD_NOT,
    );
    assert!(
        (nested_empty.scope - 2.0).abs() < 0.01 && nested_empty.chunk == 0.0,
        "A: an install frame holding one EMPTY nested scope measured scope={:.3} chunk={:.3}, not \
         two boxes and no chunk",
        nested_empty.scope,
        nested_empty.chunk
    );
    assert!(
        (nested_one.scope - 2.0).abs() < 0.01 && (nested_one.chunk - 1.0).abs() < 0.01,
        "A: an install frame holding one SPAWNING nested scope measured scope={:.3} \
         chunk={:.3}, not two boxes and one chunk",
        nested_one.scope,
        nested_one.chunk
    );

    say!(
        "  ⇒ a scope frame is ONE `Box<ScopeShared>` (256 B), plus ONE 4 KiB `ScopeBlock` chunk \
         the moment it spawns, plus a doubling chunk each time its cells overflow; a spawn \
         itself costs no allocation, only 1/63 of an injector block when it is pushed from \
         outside the pool. Measured: empty frame {:.3}, frame with one spawn {spawning_frame:.3}.",
        empty.mean
    );

    Prim {
        empty_frame: empty.mean,
        spawning_frame,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// B — per-system windows inside a real Schedule
// ═══════════════════════════════════════════════════════════════════════════

/// Per-system acquisition totals, indexed by the wrapper's own index.
static SYS_ACQ: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
/// Per-system call counts, so a system that silently did not run is visible.
static SYS_RUNS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];

/// Eight instrumented systems, each snapshotting the global counter around its
/// own body.
///
/// Every one takes `ResMut<Tally>`. That is load-bearing and not a convenience:
/// the counter is process-global, so two bodies running CONCURRENTLY would each
/// measure the other and both readings would be garbage. A shared `ResMut` is a
/// write-write conflict, so the schedule's conflict graph serialises them — the
/// windows cannot overlap by construction rather than by hoping the pool
/// happened to run them in sequence.
macro_rules! instrumented {
    ($($idx:literal => $name:ident),* $(,)?) => {
        $(
            fn $name(mut t: ResMut<Tally>, q: Query<&Payload>) {
                let before = Snap::now();
                t.hits += 1;
                black_box(q.iter().count());
                let d = Snap::now().since(before);
                SYS_ACQ[$idx].fetch_add(d.acq(), Ordering::Relaxed);
                SYS_RUNS[$idx].fetch_add(1, Ordering::Relaxed);
            }
        )*
    };
}
instrumented!(
    0 => wrapped0, 1 => wrapped1, 2 => wrapped2, 3 => wrapped3,
    4 => wrapped4, 5 => wrapped5, 6 => wrapped6, 7 => wrapped7,
);

/// The same eight bodies WITHOUT the instrument and without the shared
/// `ResMut`, so the wrapper's own cost and the serialisation's own cost are
/// measured rather than assumed.
macro_rules! plain {
    ($($name:ident),* $(,)?) => {
        $(
            fn $name(q: Query<&Payload>) {
                black_box(q.iter().count());
            }
        )*
    };
}
plain!(
    plain0, plain1, plain2, plain3, plain4, plain5, plain6, plain7
);

/// Charges each system's own body, then charges the remainder to dispatch.
fn b_per_system_windows(rows: &mut Vec<Row>, prim: &Prim) {
    say!("\n── B. PER-SYSTEM WINDOWS (8 systems, conflict-serialised) ──");
    const N: usize = 8;
    const WORKERS: usize = 4;

    for s in SYS_ACQ.iter() {
        s.store(0, Ordering::Relaxed);
    }
    for s in SYS_RUNS.iter() {
        s.store(0, Ordering::Relaxed);
    }

    let mut app = App::with_pool(pool(WORKERS));
    seed_payload(app.world_mut(), ROWS);
    app.world_mut().insert_resource(Tally::default());
    app.add_systems(wrapped0);
    app.add_systems(wrapped1);
    app.add_systems(wrapped2);
    app.add_systems(wrapped3);
    app.add_systems(wrapped4);
    app.add_systems(wrapped5);
    app.add_systems(wrapped6);
    app.add_systems(wrapped7);
    app.finish();

    let runs_before: Vec<u64> = SYS_RUNS.iter().map(|s| s.load(Ordering::Relaxed)).collect();
    let acq_before: Vec<u64> = SYS_ACQ.iter().map(|s| s.load(Ordering::Relaxed)).collect();
    let instrumented = window(WARM, REPS, || {
        app.update_with_delta(Duration::from_millis(16))
    });
    let runs: Vec<u64> = SYS_RUNS
        .iter()
        .zip(runs_before.iter())
        .map(|(s, b)| s.load(Ordering::Relaxed) - b)
        .collect();
    let body: Vec<u64> = SYS_ACQ
        .iter()
        .zip(acq_before.iter())
        .map(|(s, b)| s.load(Ordering::Relaxed) - b)
        .collect();

    // ── The plain control, same shape, no instrument, no shared ResMut ──
    let mut app2 = App::with_pool(pool(WORKERS));
    seed_payload(app2.world_mut(), ROWS);
    app2.add_systems(plain0);
    app2.add_systems(plain1);
    app2.add_systems(plain2);
    app2.add_systems(plain3);
    app2.add_systems(plain4);
    app2.add_systems(plain5);
    app2.add_systems(plain6);
    app2.add_systems(plain7);
    app2.finish();
    let plain = window(WARM, REPS, || {
        app2.update_with_delta(Duration::from_millis(16))
    });

    push(
        rows,
        "B",
        format!("App frame, {N} plain concurrent systems (control)"),
        plain,
        verdict::SHOULD_NOT,
    );
    push(
        rows,
        "B",
        format!("App frame, {N} instrumented + conflict-serialised systems"),
        instrumented,
        verdict::SHOULD_NOT,
    );

    // The per-system counters span the WHOLE `window` call — its warm-up
    // repetitions as well as its measured ones — because they are read once on
    // either side of it. `window` runs `WARM + REPS` frames, so that is the
    // divisor; using `REPS` inflated every body figure by 25% and the run-count
    // assertion below is what caught it.
    const DRIVEN: u64 = (WARM + REPS) as u64;
    let total_body: u64 = body.iter().sum();
    for (i, (b, r)) in body.iter().zip(runs.iter()).enumerate() {
        assert_eq!(
            *r, DRIVEN,
            "B: system {i} ran {r} times over {DRIVEN} driven frames — the per-system window \
             is not one-per-frame and its total cannot be divided by the frame count"
        );
        say!(
            "  system {i:>2}: body acquisitions over {DRIVEN} frames = {b}  ({:.4}/frame)",
            *b as f64 / DRIVEN as f64
        );
    }
    say!(
        "  ⇒ ALL EIGHT BODIES TOGETHER: {total_body} acquisitions over {DRIVEN} frames \
         ({:.4}/frame) against a frame total of {:.3}",
        total_body as f64 / DRIVEN as f64,
        instrumented.mean
    );

    // ── The attribution claim ──
    //
    // If the bodies cost nothing, then everything the frame costs is dispatch,
    // and the primitive prices from section A account for it exactly.
    let bodies_per_frame = total_body as f64 / DRIVEN as f64;
    assert!(
        bodies_per_frame < 0.5,
        "B: the eight system BODIES cost {bodies_per_frame:.3} acquisitions a frame — the \
         census's conclusion that the whole frame cost is dispatch no longer holds, and the \
         attribution below is charging dispatch for work a body did"
    );

    // ── The dispatch accounting, by CLASS rather than by a fitted line ──
    //
    // Section A priced an install frame that spawns as one `ScopeShared` plus
    // one chunk, with each dispatcher-side push costing 1/63 of an injector
    // block. If the executor allocates nothing of its own, the frame's classes
    // are exactly that and its OTHER class is empty.
    let predicted = 1.0 + 1.0 + N as f64 / INJECTOR_BLOCK_CAP;
    let residual = plain.mean - (plain.scope + plain.chunk + plain.inj);
    say!(
        "  ⇒ DISPATCH ACCOUNTING: measured {:.3} = scope {:.3} + chunk {:.3} + injector {:.3} + \
         residual {residual:.3} (predicted from section A: {predicted:.3}; an A-frame with one \
         spawn was {:.3})",
        plain.mean, plain.scope, plain.chunk, plain.inj, prim.spawning_frame
    );
    // THE ATTRIBUTION CLOSES HERE. One install frame and one chunk account for
    // the whole frame to within the periodic injector block, so the ECS
    // executor — the conflict walk, the ready queue, the completion cell, the
    // deferred-command drain — allocates NOTHING of its own per run, and the
    // `n` systems it spawns cost no allocation each: their cells share the one
    // chunk. On the pre-3b pool the same frame was `4 + n`.
    assert!(
        (plain.scope - 1.0).abs() < 0.01 && (plain.chunk - 1.0).abs() < 0.01,
        "B: an 8-system App frame measured scope={:.3} chunk={:.3}, not the one install frame \
         and one chunk section A prices it at",
        plain.scope,
        plain.chunk
    );
    assert!(
        residual.abs() < 0.05,
        "B: the frame's residual over `scope + chunk + injector` is {residual:.3} — something in \
         the executor or the App funnel has begun allocating per frame, and the attribution \
         table is now incomplete"
    );

    push(
        rows,
        "B",
        format!("··· of which: 8 system BODIES ({bodies_per_frame:.4}/frame)"),
        Meas {
            mean: bodies_per_frame,
            ..Meas::default()
        },
        verdict::FREE,
    );
    push(
        rows,
        "B",
        "··· of which: executor residual over scope+chunk+injector".to_string(),
        Meas {
            mean: residual,
            other: plain.other,
            other_max: plain.other_max,
            ..Meas::default()
        },
        verdict::FREE,
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// C — App subsystem differentials: one feature at a time, everything held
// ═══════════════════════════════════════════════════════════════════════════

/// A hand-written `Event` impl; only one lane-carrying type is needed here.
#[derive(Clone, Copy)]
struct NoParticipants;
impl Participants for NoParticipants {
    fn participant_count() -> usize {
        0
    }
    fn participant_info() -> &'static [ParticipantInfo] {
        &[]
    }
}

#[derive(Clone, Copy)]
struct NoParameters;
impl Parameters for NoParameters {}

/// The event carried by the event-lane arm.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ping {
    value: u32,
}

impl Event for Ping {
    type Participants = NoParticipants;
    type Parameters = NoParameters;
    fn event_id() -> u64 {
        201
    }
    fn event_name() -> &'static str {
        "Ping"
    }
    fn new(_: NoParticipants, _: NoParameters) -> Self {
        Ping { value: 0 }
    }
    fn participants(&self) -> &NoParticipants {
        unimplemented!()
    }
    fn participants_mut(&mut self) -> &mut NoParticipants {
        unimplemented!()
    }
    fn parameters(&self) -> &NoParameters {
        unimplemented!()
    }
    fn parameters_mut(&mut self) -> &mut NoParameters {
        unimplemented!()
    }
}

fn send_pings(mut w: EventWriter<Ping>, mut t: ResMut<Tally>) {
    for i in 0..EVENTS_PER_FRAME {
        w.send(Ping { value: i })
            .expect("send within lane capacity");
        t.sent += 1;
    }
}

fn read_pings(mut r: EventReader<Ping>, mut t: ResMut<Tally>) {
    let mut n = 0u64;
    for e in r.read() {
        black_box(e.value);
        n += 1;
    }
    t.received += n;
}

fn touch_payload(mut q: Query<Mut<Payload>>) {
    for mut p in q.iter_mut() {
        p.v = p.v.wrapping_add(1);
    }
}

fn count_changed(q: Query<&Payload, Changed<Payload>>, mut t: ResMut<Tally>) {
    t.changed += q.iter().count() as u64;
}

/// The `par_iter` fan-out: one nested scope plus one cell per chunk, per call.
fn par_fanout(q: Query<&Payload>) {
    static SUM: AtomicU64 = AtomicU64::new(0);
    q.par_iter().for_each(|p: &Payload| {
        SUM.fetch_add(p.v as u64, Ordering::Relaxed);
    });
}

/// Spawns and despawns one generation per frame through `Commands`.
fn churn(mut cmds: Commands, q: Query<&Doomed>, ents: Entities, mut state: ResMut<Churn>) {
    let generation = state.generation;
    for (id, d) in q.iter_entities() {
        if d.wave != generation
            && let Some(e) = ents.get(id)
        {
            cmds.despawn(e);
        }
    }
    for _ in 0..CHURN_PER_FRAME {
        cmds.spawn(DoomedBundle {
            d: Doomed { wave: generation },
        });
    }
    state.spawned += CHURN_PER_FRAME as u64;
    state.generation = generation.wrapping_add(1);
}

/// Two no-op filler systems, so every arm below carries the SAME system count
/// and the delta is the feature rather than the `n` in `n + 4`.
fn filler0(q: Query<&Payload>) {
    black_box(q.iter().count());
}
fn filler1(q: Query<&Payload>) {
    black_box(q.iter().count());
}

/// Every arm here has exactly four Main systems and two workers. Only what the
/// systems DO changes, so each row's delta from the baseline is that feature's
/// per-frame allocation and nothing else.
fn c_app_subsystem_deltas(rows: &mut Vec<Row>) -> f64 {
    say!("\n── C. APP SUBSYSTEM DELTAS (4 Main systems everywhere, W=2) ──");
    const WORKERS: usize = 2;
    register_event::<Ping>(201);

    // Baseline: four systems that only read.
    let mut base_app = App::with_pool(pool(WORKERS));
    seed_payload(base_app.world_mut(), ROWS);
    base_app.add_systems(plain0);
    base_app.add_systems(plain1);
    base_app.add_systems(filler0);
    base_app.add_systems(filler1);
    base_app.finish();
    let base = window(WARM, REPS, || {
        base_app.update_with_delta(Duration::from_millis(16))
    });
    push(
        rows,
        "C",
        "baseline: 4 read-only systems".to_string(),
        base,
        verdict::SHOULD_NOT,
    );

    // Events: send 32, read 32, every frame, with the EveryFrame swap policy so
    // `update_events` (App step ③) is entered on every single frame.
    let mut ev_app = App::with_pool(pool(WORKERS));
    seed_payload(ev_app.world_mut(), ROWS);
    ev_app
        .world_mut()
        .preregister_event::<Ping>(
            EventConfig::default_for(WORKERS as u32 + 1).expect("event config for W+1 lanes"),
        )
        .expect("preregister Ping");
    ev_app.world_mut().insert_resource(Tally::default());
    ev_app.set_event_update_policy(EventUpdatePolicy::EveryFrame);
    ev_app.add_systems(send_pings);
    ev_app.add_systems(read_pings);
    ev_app.add_systems(filler0);
    ev_app.add_systems(filler1);
    ev_app.finish();
    let before_ev = {
        let t = ev_app.world().resource::<Tally>();
        (t.sent, t.received)
    };
    let ev = window(WARM, REPS, || {
        ev_app.update_with_delta(Duration::from_millis(16))
    });
    let (sent, received) = {
        let t = ev_app.world().resource::<Tally>();
        (t.sent - before_ev.0, t.received - before_ev.1)
    };
    assert!(
        sent >= REPS as u64 * EVENTS_PER_FRAME as u64 && received >= sent - EVENTS_PER_FRAME as u64,
        "C: ANTI-VACUITY — {received} of {sent} events reached a reader; the event lane is \
         not delivering, so its measured zero is a zero over nothing"
    );
    push(
        rows,
        "C",
        format!("+ event lane: {EVENTS_PER_FRAME}/frame sent+read, EveryFrame swap"),
        ev,
        verdict::FREE,
    );

    // Change detection: mutate all 4096 rows through `Mut<T>` and run a
    // `Changed<T>` query over them.
    let mut cd_app = App::with_pool(pool(WORKERS));
    seed_payload(cd_app.world_mut(), ROWS);
    cd_app.world_mut().insert_resource(Tally::default());
    cd_app.add_systems(touch_payload);
    cd_app.add_systems(count_changed);
    cd_app.add_systems(filler0);
    cd_app.add_systems(filler1);
    cd_app.finish();
    let before_cd = cd_app.world().resource::<Tally>().changed;
    let cd = window(WARM, REPS, || {
        cd_app.update_with_delta(Duration::from_millis(16))
    });
    let changed = cd_app.world().resource::<Tally>().changed - before_cd;
    assert!(
        changed >= (REPS as u64 - 1) * ROWS as u64,
        "C: ANTI-VACUITY — Changed<Payload> matched {changed} rows over {REPS} frames; change \
         detection is not seeing the mutation"
    );
    push(
        rows,
        "C",
        format!("+ change detection: {ROWS} Mut<T> writes + a Changed<T> query"),
        cd,
        verdict::FREE,
    );

    // `par_iter`: one fan-out over the 4096-row archetype.
    let mut pi_app = App::with_pool(pool(WORKERS));
    seed_payload(pi_app.world_mut(), ROWS);
    pi_app.add_systems(par_fanout);
    pi_app.add_systems(plain1);
    pi_app.add_systems(filler0);
    pi_app.add_systems(filler1);
    pi_app.finish();
    let pi = window(WARM, REPS, || {
        pi_app.update_with_delta(Duration::from_millis(16))
    });
    push(
        rows,
        "C",
        format!("+ one Query::par_iter fan-out over {ROWS} rows"),
        pi,
        verdict::SHOULD_NOT,
    );

    // `Commands`: spawn and despawn a generation per frame.
    let mut ch_app = App::with_pool(pool(WORKERS));
    seed_payload(ch_app.world_mut(), ROWS);
    ch_app.world_mut().insert_resource(Churn::default());
    ch_app.add_systems(churn);
    ch_app.add_systems(plain1);
    ch_app.add_systems(filler0);
    ch_app.add_systems(filler1);
    ch_app.finish();
    ch_app.update_with_delta(Duration::from_millis(16));
    ch_app.update_with_delta(Duration::from_millis(16));
    let pop_before = ch_app.world().entity_count();
    let spawned_before = ch_app.world().resource::<Churn>().spawned;
    let ch = window(WARM, REPS, || {
        ch_app.update_with_delta(Duration::from_millis(16))
    });
    let spawned = ch_app.world().resource::<Churn>().spawned - spawned_before;
    let pop_after = ch_app.world().entity_count();
    assert!(
        spawned >= REPS as u64 * CHURN_PER_FRAME as u64,
        "C: ANTI-VACUITY — the churn system did not spawn every frame ({spawned})"
    );
    assert!(
        pop_after.abs_diff(pop_before) <= 2 * CHURN_PER_FRAME,
        "C: the churn population drifted {pop_before} -> {pop_after}; not a steady state, so \
         its delta prices growth rather than churn"
    );
    push(
        rows,
        "C",
        format!("+ Commands: {CHURN_PER_FRAME} spawns + {CHURN_PER_FRAME} despawns/frame"),
        ch,
        verdict::FREE,
    );

    say!(
        "  ⇒ DELTAS from the 4-system baseline ({:.3}): events {:+.3}, change-detect {:+.3}, \
         par_iter {:+.3}, Commands churn {:+.3}",
        base.mean,
        ev.mean - base.mean,
        cd.mean - base.mean,
        pi.mean - base.mean,
        ch.mean - base.mean
    );

    // ── C6: does the churn's id bookkeeping stay FLAT on a flat population? ──
    //
    // Before EM2′ the churn arm was the only one in the whole census with a
    // non-zero steady-state `realloc`: `Commands::spawn` never reused an id, so
    // the recycled-id free list and the id-slot store grew one entry per
    // despawn forever. The fix moved the free list onto a `VmColumn`, which
    // this counter cannot see — so the `realloc` quarters below read zero
    // whether or not ids are recycled, and the gate is the two LENGTHS: a
    // recycling deferred route keeps the free list at one frame's despawns and
    // the slot store exactly where it was.
    const QUARTER: usize = 512;
    let mut quarters = [(0u64, 0u64); 4];
    let recycled_before = ch_app.world().recycled_entity_count();
    let slots_before = ch_app.world().entity_master().capacity();
    for q in &mut quarters {
        let before = Snap::now();
        for _ in 0..QUARTER {
            ch_app.update_with_delta(Duration::from_millis(16));
        }
        let d = Snap::now().since(before);
        *q = (d.realloc, d.realloc_bytes);
    }
    let pop_end = ch_app.world().entity_count();
    let recycled_after = ch_app.world().recycled_entity_count();
    let slots_after = ch_app.world().entity_master().capacity();
    say_inline!("  ⇒ churn realloc over 4 × {QUARTER} further frames: ");
    for (calls, bytes) in &quarters {
        say_inline!("{calls} call(s)/{bytes} B  ");
    }
    say!("; population {pop_after} -> {pop_end}");

    // ── The growable, measured by its length rather than its bytes ──
    //
    // The only push-per-despawn structure on the `DespawnCommand` path is the
    // recycled-id free list, and it has a public length
    // (`recycled_entity_count`), as the slot store does (`capacity`). Both are
    // counts of ids, so they see the leak class whether the storage behind them
    // is a heap `Vec` or an uncounted `VirtualAlloc` column.
    let frames = 4 * QUARTER;
    let despawns = frames * CHURN_PER_FRAME;
    say!(
        "  ⇒ over the SAME {frames} frames, {despawns} despawns: recycled-id free list \
         {recycled_before} -> {recycled_after} ({:+}), id-slot store {slots_before} -> \
         {slots_after} ({:+}), live population {pop_after} -> {pop_end}",
        recycled_after as i64 - recycled_before as i64,
        slots_after as i64 - slots_before as i64
    );
    assert!(
        recycled_after <= recycled_before + CHURN_PER_FRAME,
        "C6: the recycled-id free list went {recycled_before} -> {recycled_after} over \
         {despawns} despawns on a flat population — the deferred `Commands` route is not \
         reusing the ids it despawns (EM2′ regressed: the list should hold at most one \
         frame's {CHURN_PER_FRAME} despawns)"
    );
    assert_eq!(
        slots_after, slots_before,
        "C6: the id-slot store grew {slots_before} -> {slots_after} over {despawns} despawns on \
         a flat population — `Commands::spawn` is minting fresh ids instead of claiming the \
         recycled ones (EM2′ regressed)"
    );
    assert!(
        pop_end.abs_diff(pop_after) <= 2 * CHURN_PER_FRAME,
        "C6: the churn population drifted {pop_after} -> {pop_end} over the long run; the \
         quarters are not comparable"
    );

    // ── C7: the control — the DIRECT route recycles on the same churn ──
    //
    // Before EM2′ this control is what said whose defect the leak was: the
    // dispatcher's own `create_entity` popped the free list while
    // `Commands::spawn` reserved through the worker counter, so "the engine
    // never recycles" and "the deferred path never recycles" had different
    // fixes. Since EM2′ both routes pop the same recycled stack; the control
    // stays so that a C6 red can still be told apart from a regression of the
    // stack itself (which would red here too).
    let mut direct = EcsMaster::new();
    let arch = direct.bundle_archetype_id_for::<DoomedBundle>();
    let d_recycled_before = direct.recycled_entity_count();
    let d_slots_before = direct.entity_master().capacity();
    let mut live: Vec<Entity> = Vec::with_capacity(CHURN_PER_FRAME);
    for round in 0..512u32 {
        for e in live.drain(..) {
            assert!(direct.delete_entity(e), "C7: a live entity must despawn");
        }
        for _ in 0..CHURN_PER_FRAME {
            let bytes = round.to_ne_bytes();
            let e = direct
                .create_entity(arch, &[(Doomed::component_id(), &bytes)])
                .expect("C7: the Doomed archetype accepts its one column");
            live.push(e);
        }
    }
    let d_direct_despawns = 512 * CHURN_PER_FRAME - CHURN_PER_FRAME;
    say!(
        "  ⇒ CONTROL, the same churn through the DISPATCHER's own create_entity/delete_entity \
         ({d_direct_despawns} despawns): recycled-id free list {d_recycled_before} -> {}, \
         id-slot store {d_slots_before} -> {}",
        direct.recycled_entity_count(),
        direct.entity_master().capacity()
    );
    assert!(
        direct.entity_master().capacity() <= d_slots_before + 2 * CHURN_PER_FRAME,
        "C7: the DIRECT churn route grew the id-slot store, from {d_slots_before} to {} over \
         {d_direct_despawns} despawns — the recycled stack itself is not recycling, on the \
         dispatcher route as well as the deferred one",
        direct.entity_master().capacity()
    );

    // The rate must not RISE. Before EM2′ the calls fell (a doubling `Vec`
    // makes each successive growth twice as far apart) while the SIZE of each
    // growth doubled — bounded in call count and NOT in bytes, which is why the
    // length asserts above, not this one, are the leak's gate. Since EM2′ the
    // quarters read zero; a `realloc` rate that appears and holds would be a
    // NEW heap growable on the churn path.
    let early = quarters[0].0 + quarters[1].0;
    let late = quarters[2].0 + quarters[3].0;
    assert!(
        late <= early.max(2),
        "C6: the churn scene's `realloc` rate did not fall across a long steady run \
         ({early} calls in the first half, {late} in the second) — a growable is growing at a \
         constant rate on a FLAT population, which is the one shape the arena exists to \
         remove, and it is unbounded"
    );

    // ── The attribution claims ──
    assert!(
        (ev.mean - base.mean).abs() < 0.6,
        "C: the event lane cost {:+.3} a frame over an identical 4-system baseline; the \
         census concluded App steps ⓪–③ are free and this contradicts it",
        ev.mean - base.mean
    );
    assert!(
        (ch.mean - base.mean).abs() < 0.6,
        "C: the Commands churn cost {:+.3} a frame over an identical 4-system baseline; since          EM2′ the deferred spawn/despawn route reuses its ids and the recycled stack is not on          the heap, so a non-zero delta is a NEW heap growable on the churn path",
        ch.mean - base.mean
    );
    assert!(
        (cd.mean - base.mean).abs() < 0.6,
        "C: change detection cost {:+.3} a frame; the census concluded {ROWS} Mut<T> writes \
         and a Changed<T> query are free and this contradicts it",
        cd.mean - base.mean
    );
    assert!(
        (pi.scope - base.scope - 1.0).abs() < 0.01 && pi.chunk - base.chunk >= 0.99,
        "C: a `par_iter` fan-out added scope {:+.3} / chunk {:+.3} — it should add exactly its \
         own nested scope and at least one chunk, so either it did not fan out or the dispatch \
         shape has changed",
        pi.scope - base.scope,
        pi.chunk - base.chunk
    );

    base.mean
}

// ═══════════════════════════════════════════════════════════════════════════
// D — the physics step, by differential toggle on ONE warmed world
// ═══════════════════════════════════════════════════════════════════════════

const BOX_SIZE: f32 = 2.0;
const BOX_SEPARATION: f32 = 0.5;
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
const DT: f32 = 1.0 / 60.0;

/// Jolt's `cPyramidHeight`, scaled down in a debug build.
const fn pyramid_height() -> i32 {
    if cfg!(debug_assertions) { 10 } else { 15 }
}

fn spawn_body(
    world: &mut EcsMaster,
    body: RigidBody,
    mass: RigidBodyMass,
    collider: Collider,
    simulated: bool,
) -> Entity {
    fn as_bytes<T>(value: &T) -> &[u8] {
        // SAFETY: `T` is a `#[repr(C)]` POD physics component with no padding
        // invariants of its own; the slice is exactly `size_of::<T>()` bytes
        // read from a live `&T` and cannot outlive that borrow.
        unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
    }
    let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
    let e = world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("invariant: RigidBodyBundle archetype accepts the three columns");
    if simulated {
        world.enable::<Simulated>(e);
    }
    e
}

/// Builds Jolt's pyramid, index for index.
fn spawn_jolt_pyramid(world: &mut EcsMaster) -> usize {
    spawn_body(
        world,
        RigidBody {
            position: Vec3::new(0.0, -1.0, 0.0),
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        },
        RigidBodyMass {
            inv_inertia: Mat3::ZERO,
            inv_mass: 0.0,
            restitution: 0.0,
            friction: 0.5,
        },
        Collider {
            shape: ColliderShape::Box {
                half_extents: FLOOR_HALF_EXTENTS,
            },
            layer: 1,
            mask: 1,
        },
        false,
    );

    let height = pyramid_height();
    let mut n = 0usize;
    for i in 0..height {
        let lo = i / 2;
        let hi = height - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                let position = Vec3::new(
                    -(height as f32) + BOX_SIZE * j as f32 + odd,
                    1.0 + (BOX_SIZE + BOX_SEPARATION) * i as f32,
                    -(height as f32) + BOX_SIZE * k as f32 + odd,
                );
                spawn_body(
                    world,
                    RigidBody {
                        position,
                        linear_velocity: Vec3::ZERO,
                        rotation: Quat::IDENTITY,
                        angular_velocity: Vec3::ZERO,
                    },
                    RigidBodyMass {
                        inv_inertia: Mat3::from_diagonal(Vec3::new(0.1875, 0.1875, 0.1875)),
                        inv_mass: 0.125,
                        restitution: 0.0,
                        friction: 0.5,
                    },
                    Collider {
                        shape: ColliderShape::Box {
                            half_extents: Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
                        },
                        layer: 1,
                        mask: 1,
                    },
                    true,
                );
                n += 1;
            }
        }
    }
    n
}

/// A built, warmed physics world plus the schedule that steps it.
struct Pile {
    world: EcsMaster,
    schedule: Schedule,
    bodies: usize,
}

/// Builds a colored-solve pile and warms it to its resting shape.
fn build_pile(workers: usize, colored: bool, substeps: u32, relax: u32) -> Pile {
    let mut world = EcsMaster::new();
    let bodies = spawn_jolt_pyramid(&mut world);
    let mut builder = ScheduleBuilder::new(pool(workers));
    if colored {
        let _ = add_physics_colored_solve(&mut builder, &mut world);
    } else {
        let _ = add_physics_systems::<SoftStepSolver>(&mut builder, &mut world);
    }
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
        cfg.dt = DT;
        cfg.substeps = substeps;
        cfg.relax_iterations = relax;
        cfg.parallel_solve = false;
        cfg.parallel_broadphase = false;
        cfg.sleeping = false;
    }
    let schedule = builder.build(&mut world);
    Pile {
        world,
        schedule,
        bodies,
    }
}

impl Pile {
    fn step(&mut self) {
        self.schedule.run(&mut self.world);
    }
    fn set_parallel_solve(&mut self, on: bool) {
        self.world.resource_mut::<PhysicsConfig>().parallel_solve = on;
    }
    fn set_parallel_broadphase(&mut self, on: bool) {
        self.world
            .resource_mut::<PhysicsConfig>()
            .parallel_broadphase = on;
    }
    /// Both loop counts at once. `PhysicsConfig` is read at the top of every
    /// step, so this takes effect on the next `step()` without a rebuild.
    fn set_passes(&mut self, substeps: u32, relax: u32) {
        let cfg = self.world.resource_mut::<PhysicsConfig>();
        cfg.substeps = substeps;
        cfg.relax_iterations = relax;
    }
    fn contacts(&self) -> usize {
        self.world.resource::<Manifolds>().manifolds().len()
    }
}

/// Two windows of the same configuration as one row.
fn merge(a: &Meas, b: &Meas) -> Meas {
    Meas {
        mean: (a.mean + b.mean) / 2.0,
        min: a.min.min(b.min),
        max: a.max.max(b.max),
        realloc_max: a.realloc_max.max(b.realloc_max),
        bytes_mean: (a.bytes_mean + b.bytes_mean) / 2.0,
        scope: (a.scope + b.scope) / 2.0,
        chunk: (a.chunk + b.chunk) / 2.0,
        inj: (a.inj + b.inj) / 2.0,
        other: (a.other + b.other) / 2.0,
        other_max: a.other_max.max(b.other_max),
    }
}

/// The physics attribution. Every toggle here happens on ONE warmed world
/// between interleaved windows, so the pile's own contact-count drift is
/// present in BOTH arms of every comparison and cannot masquerade as the delta.
fn d_physics_deltas(rows: &mut Vec<Row>) -> (f64, f64) {
    say!("\n── D. PHYSICS DIFFERENTIALS (one warmed 1240-body pile, A/B/A) ──");
    const WORKERS: usize = 4;

    let mut pile = build_pile(WORKERS, true, 4, 2);
    for _ in 0..PHYS_WARM {
        pile.step();
    }
    let contacts = pile.contacts();
    assert!(
        contacts > 0,
        "D: ANTI-VACUITY — the narrowphase produced ZERO contacts, so every number below is a \
         number over an empty solve"
    );
    say!(
        "  scene: {} dynamic bodies, {contacts} contacts, W={WORKERS}",
        pile.bodies
    );

    // ── D1: parallel_solve, A/B/A on the same warmed world ──
    pile.set_parallel_solve(false);
    let off_a = window(4, PHYS_REPS, || pile.step());
    pile.set_parallel_solve(true);
    let on_a = window(4, PHYS_REPS, || pile.step());
    pile.set_parallel_solve(false);
    let off_b = window(4, PHYS_REPS, || pile.step());
    pile.set_parallel_solve(true);
    let on_b = window(4, PHYS_REPS, || pile.step());

    let off_mean = (off_a.mean + off_b.mean) / 2.0;
    let on_mean = (on_a.mean + on_b.mean) / 2.0;
    push(
        rows,
        "D",
        "physics step, parallel_solve OFF (2 windows, interleaved)".to_string(),
        merge(&off_a, &off_b),
        verdict::SHOULD_NOT,
    );
    push(
        rows,
        "D",
        "physics step, parallel_solve ON (2 windows, interleaved)".to_string(),
        merge(&on_a, &on_b),
        verdict::SHOULD_NOT,
    );
    say!(
        "  ⇒ parallel_solve costs {:+.1} acquisitions a step ({:.0}× the whole serial step); \
         drift between the two OFF windows was {:.3}, between the two ON windows {:.3}",
        on_mean - off_mean,
        on_mean / off_mean.max(1.0),
        (off_a.mean - off_b.mean).abs(),
        (on_a.mean - on_b.mean).abs()
    );
    assert!(
        (off_a.mean - off_b.mean).abs() < 2.0,
        "D: the two parallel_solve-OFF windows disagree by {:.3}; the world is drifting fast \
         enough that the A/B delta is not attributable to the flag",
        (off_a.mean - off_b.mean).abs()
    );
    assert!(
        on_mean > off_mean * 20.0,
        "D: parallel_solve ON ({on_mean:.1}) is not decisively above OFF ({off_mean:.1}) — \
         either the dispatch never engaged or its cost has been removed; in the first case \
         nothing below measures parallelism"
    );

    // ── D2: parallel_broadphase alone, same world ──
    pile.set_parallel_solve(false);
    pile.set_parallel_broadphase(true);
    let bp_on = window(4, PHYS_REPS, || pile.step());
    pile.set_parallel_broadphase(false);
    let bp_off = window(4, PHYS_REPS, || pile.step());
    push(
        rows,
        "D",
        "physics step, parallel_broadphase ON alone".to_string(),
        bp_on,
        verdict::FREE,
    );
    say!(
        "  ⇒ parallel_broadphase alone costs {:+.3} a step — it is INERT on this scene \
         (its body floor is above {} bodies)",
        bp_on.mean - bp_off.mean,
        pile.bodies
    );
    assert!(
        (bp_on.mean - bp_off.mean).abs() < 1.0,
        "D: parallel_broadphase moved the step by {:+.3}; the census attributed the whole \
         parallel budget to the SOLVE on the stated ground that the broadphase is below its \
         floor here, and that ground has moved",
        bp_on.mean - bp_off.mean
    );

    // ── D3: what UNIT the parallel budget is charged per ──
    //
    // ⚠ The first form of this arm divided by `substeps + relax` and measured a
    // per-unit cost that DOUBLED between its cheapest and its dearest point, so
    // it failed its own model check. The model was wrong, not the tree: the
    // relax loop in `solve_colored_inner` is NESTED INSIDE the substep loop
    // (`for _ in 0..substeps { … for _ in 0..relax_iterations { … } }`), so the
    // number of color passes is `substeps × (1 + relax)` — 12 at the defaults,
    // not 6. Under the right divisor the same four points are flat.
    //
    // The sweep runs on the SAME warmed world, interleaved, and returns to the
    // starting configuration at the end: `substeps` changes the trajectory, so
    // four separately-warmed piles would be four different physical states and
    // the "per pass" figure would be contaminated by how each one settled.
    fn passes_of(substeps: u32, relax: u32) -> u32 {
        substeps * (1 + relax)
    }
    pile.set_parallel_solve(true);
    let sweep = [(4u32, 2u32), (4, 0), (2, 0), (1, 0), (4, 2)];
    let mut pass_points: Vec<(u32, f64)> = Vec::with_capacity(sweep.len());
    for (substeps, relax) in sweep {
        pile.set_passes(substeps, relax);
        let per_frame = frames(6, PHYS_REPS, || pile.step());
        let m = summarise(&per_frame);
        let passes = passes_of(substeps, relax);
        pass_points.push((passes, m.mean));
        // THE STRUCTURAL CLAIM, per frame and exact: a step opens ONE install
        // frame plus one nested scope per DISPATCHED colour per pass, and every
        // pass walks the same colours (the graph is built once per step). So
        // `scope - 1` is a multiple of the pass count on EVERY frame, and the
        // quotient is the number of colours wide enough to dispatch.
        let mut colours: Vec<u64> = Vec::with_capacity(per_frame.len());
        for (i, f) in per_frame.iter().enumerate() {
            assert!(
                f.scope >= 1 && (f.scope - 1) % passes as u64 == 0,
                "D: substeps={substeps} relax={relax}: frame {i} opened {} scope frames, and \
                 {} is not a multiple of the {passes} colour passes — the solver no longer \
                 opens one scope per dispatched colour per pass",
                f.scope,
                f.scope.saturating_sub(1)
            );
            assert!(
                f.chunk >= f.scope,
                "D: frame {i}: {} chunks for {} scope frames — a dispatched colour spawns at \
                 least two chunks' worth of tasks by construction (a single-chunk colour runs \
                 inline), so every scope must hold at least one chunk",
                f.chunk,
                f.scope
            );
            colours.push((f.scope - 1) / passes as u64);
        }
        let (c_lo, c_hi) = (
            colours.iter().min().copied().unwrap_or(0),
            colours.iter().max().copied().unwrap_or(0),
        );
        say!(
            "  ⇒ substeps={substeps} relax={relax}: {passes} passes × {c_lo}..={c_hi} dispatched \
             colour(s) + 1 install frame = {:.1} scope frames/step, {:.2} chunks per scope",
            m.scope,
            m.chunk / m.scope.max(1.0)
        );
        push(
            rows,
            "D",
            format!(
                "physics step, parallel_solve ON, substeps={substeps} relax={relax} \
                 ({passes} color passes)"
            ),
            m,
            verdict::SHOULD_NOT,
        );
    }
    say_inline!("  ⇒ acquisitions PER COLOR PASS: ");
    for (passes, mean) in &pass_points {
        say_inline!("{passes}→{:.1}  ", mean / *passes as f64);
    }
    say!();

    // Drift control: the sweep opened and closed on the same configuration.
    let (open, close) = (pass_points[0].1, pass_points[4].1);
    say!(
        "  ⇒ drift control: the sweep opened at {open:.1} and closed at {close:.1} on the \
         SAME (4, 2) configuration ({:+.1}%)",
        (close - open) * 100.0 / open
    );
    assert!(
        (close - open).abs() < open * 0.25,
        "D: the sweep opened at {open:.1} and closed at {close:.1} on the same configuration — \
         the world drifted more than the effect being measured, so no row in this arm is \
         attributable to its own configuration"
    );

    let per_pass: Vec<f64> = pass_points.iter().map(|(p, m)| m / *p as f64).collect();
    let lo = per_pass.iter().cloned().fold(f64::MAX, f64::min);
    let hi = per_pass.iter().cloned().fold(0.0, f64::max);
    say!("  ⇒ per-color-pass cost spans {lo:.1} … {hi:.1} across the sweep");
    assert!(
        hi < lo * 1.6,
        "D: the per-COLOR-PASS cost spans {lo:.1}…{hi:.1} ({:.2}×) — the parallel budget is \
         not charged per color pass and the model above is wrong",
        hi / lo
    );

    // ── D4: how it scales with worker count ──
    //
    // The chunk count is `lanes * CHUNKS_PER_WORKER` capped by work, so if the
    // budget is one cell per chunk it is roughly linear in the worker count.
    let mut lane_points: Vec<(usize, f64)> = Vec::with_capacity(4);
    for w in [1usize, 2, 4] {
        let mut p = build_pile(w, true, 4, 2);
        for _ in 0..PHYS_WARM {
            p.step();
        }
        p.set_parallel_solve(true);
        let m = window(4, PHYS_REPS, || p.step());
        lane_points.push((w, m.mean));
        push(
            rows,
            "D",
            format!("physics step, parallel_solve ON, {w} worker(s)"),
            m,
            verdict::SHOULD_NOT,
        );
    }
    say!(
        "  ⇒ by worker count: {} → {:.0}, {} → {:.0}, {} → {:.0} acquisitions a step",
        lane_points[0].0,
        lane_points[0].1,
        lane_points[1].0,
        lane_points[1].1,
        lane_points[2].0,
        lane_points[2].1
    );
    assert!(
        lane_points[2].1 >= lane_points[0].1,
        "D: four workers ({:.1}) did not cost more than one ({:.1}); the per-chunk cell model \
         predicts the chunk count rises with the lane count and it did not",
        lane_points[2].1,
        lane_points[0].1
    );

    (off_mean, on_mean)
}

// ═══════════════════════════════════════════════════════════════════════════
// E — call sites, by backtrace. Diagnostic only, never armed for an assertion.
// ═══════════════════════════════════════════════════════════════════════════

/// Whether the capture mode was asked for.
fn trace_requested() -> bool {
    std::env::var("BOYKO_ALLOC_TRACE")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Runs `f` with capture armed and prints the site distribution it produced.
///
/// The counts printed here are NOT per-frame figures and must never be quoted
/// as such: capture allocates (it renders a `Backtrace` into a `String`), so an
/// armed window's own totals include the instrument. Only the DISTRIBUTION over
/// sites is meaningful, which is exactly what the deliverable needs.
fn trace_window<F: FnMut()>(label: &str, budget: u64, filter: u8, mut f: F) {
    if let Ok(mut sites) = SITES.lock() {
        sites.clear();
    }
    if let Ok(mut stages) = STAGES.lock() {
        stages.clear();
    }
    if let Ok(mut first) = FIRST_RAW.lock() {
        first.clear();
    }
    TRACE_SEEN.store(0, Ordering::SeqCst);
    TRACE_BUDGET.store(budget, Ordering::SeqCst);
    TRACE_FILTER.store(filter, Ordering::SeqCst);
    TRACE_ON.store(true, Ordering::SeqCst);
    f();
    TRACE_ON.store(false, Ordering::SeqCst);
    TRACE_FILTER.store(FILTER_ALL, Ordering::SeqCst);

    let seen = TRACE_SEEN.load(Ordering::SeqCst);
    let mut sites = SITES.lock().expect("site table").clone();
    sites.sort_unstable_by_key(|a| std::cmp::Reverse(a.1));
    let captured: u64 = sites.iter().map(|(_, c, _)| *c).sum();

    say!("\n╔══ CALL SITES — {label}");
    say!(
        "║ {captured} of {seen} allocations captured ({} distinct sites); the rest are the \
         instrument's own and the budget's tail",
        sites.len()
    );
    let mut stages = STAGES.lock().expect("stage table").clone();
    stages.sort_unstable_by_key(|a| std::cmp::Reverse(a.1));
    say!("║ BY STAGE (the innermost system / executor step on each captured stack):");
    for (stage, hits, bytes) in &stages {
        say!(
            "║   {hits:>6} hits {bytes:>10} B  ({:>5.1}%)  {stage}",
            *hits as f64 * 100.0 / captured.max(1) as f64
        );
    }
    for (i, (key, hits, bytes)) in sites.iter().take(12).enumerate() {
        say!(
            "║\n║ #{i}  {hits} hits, {bytes} bytes ({:.1}% of captured)",
            *hits as f64 * 100.0 / captured.max(1) as f64
        );
        for line in key.lines().take(KEEP_FRAMES) {
            say!("║      {line}");
        }
    }
    if let Ok(first) = FIRST_RAW.lock()
        && !first.is_empty()
    {
        say!(
            "║\n║ ── the FIRST capture, verbatim and unfolded, so the folding above can be judged ──"
        );
        for line in first.lines().take(28) {
            say!("║   {line}");
        }
    }
    say!("╚══════════════════════════════════════════════════════════════════════");
}

/// The diagnostic run. Off unless `BOYKO_ALLOC_TRACE=1`.
fn e_call_sites() {
    if !trace_requested() {
        say!(
            "\n── E. CALL SITES: not armed. Re-run with BOYKO_ALLOC_TRACE=1 for the \
             backtrace-attributed site and stage tables. It is off by default because capture \
             ALLOCATES, so an armed window's own counts are the instrument's as well as the \
             frame's — it can diagnose, it must never gate."
        );
        return;
    }
    say!("\n── E. CALL SITES (BOYKO_ALLOC_TRACE=1; diagnostic, asserts nothing) ──");

    // (1) An 8-system App frame: the executor's whole per-frame budget.
    let mut app = App::with_pool(pool(4));
    seed_payload(app.world_mut(), ROWS);
    app.add_systems(plain0);
    app.add_systems(plain1);
    app.add_systems(plain2);
    app.add_systems(plain3);
    app.add_systems(plain4);
    app.add_systems(plain5);
    app.add_systems(plain6);
    app.add_systems(plain7);
    app.finish();
    for _ in 0..8 {
        app.update_with_delta(Duration::from_millis(16));
    }
    trace_window(
        "one App frame, 8 concurrent systems",
        256,
        FILTER_ALL,
        || {
            app.update_with_delta(Duration::from_millis(16));
        },
    );

    // (2) One physics step with the parallel solve on — per STAGE, so the step
    // is charged to the system that made each acquisition. ⚠ It is the 50th
    // step of a fresh pile, inside the census's 64-step warm-up (145 scope
    // frames, 12 dispatched colours, 386 acquisitions on 2026-09-11): quote its
    // stage percentages with ITS numbers, never against the steady window's.
    let mut pile = build_pile(4, true, 4, 2);
    for _ in 0..PHYS_WARM {
        pile.step();
    }
    pile.set_parallel_solve(true);
    pile.step();
    trace_window(
        "one physics step, parallel_solve ON, W=4",
        4096,
        FILTER_ALL,
        || {
            pile.step();
        },
    );

    // (3) One churn frame.
    let mut ch = App::with_pool(pool(4));
    seed_payload(ch.world_mut(), ROWS);
    ch.world_mut().insert_resource(Churn::default());
    ch.add_systems(churn);
    ch.add_systems(par_fanout);
    ch.finish();
    for _ in 0..16 {
        ch.update_with_delta(Duration::from_millis(16));
    }
    trace_window(
        "one churn + par_iter frame (Commands and the fan-out)",
        512,
        FILTER_ALL,
        || {
            ch.update_with_delta(Duration::from_millis(16));
        },
    );

    // (4) The churn scene's REALLOC sites. Before EM2′ this caught the one
    // growable in the whole census (the free list's `Vec`); since EM2′ the
    // stack lives on a `VmColumn` and the window is expected to capture none.
    trace_window(
        "the churn scene's REALLOC sites over 2048 frames (realloc-only filter)",
        64,
        FILTER_REALLOC,
        || {
            for _ in 0..2048 {
                ch.update_with_delta(Duration::from_millis(16));
            }
        },
    );

    // (5) The census's rare OTHER-class frames. They are one frame in a few
    // hundred and only on a FRESH pool, so this arms the OTHER-only filter
    // across the whole life of several fresh 4-system Apps — from the first
    // frame, which is where a per-thread first touch has to land.
    trace_window(
        "OTHER-class acquisitions across the frames of 24 fresh 2- and 4-system Apps (W=2)",
        256,
        FILTER_OTHER,
        || {
            for round in 0..24 {
                // Setup is NOT the question and would eat the budget: disarm
                // around the build, re-arm for the frames. `finish` plus the
                // first frame stay disarmed too — they are setup by the census's
                // own regime definition.
                TRACE_ON.store(false, Ordering::SeqCst);
                let mut a = App::with_pool(pool(2));
                seed_payload(a.world_mut(), ROWS);
                a.add_systems(plain0);
                a.add_systems(plain1);
                // Alternate the census's S0 n=2 shape (where a three-OTHER frame
                // was seen) with its n=4 shape.
                if round % 2 == 1 {
                    a.add_systems(filler0);
                    a.add_systems(filler1);
                }
                a.finish();
                a.update_with_delta(Duration::from_millis(16));
                TRACE_ON.store(true, Ordering::SeqCst);
                for _ in 0..320 {
                    a.update_with_delta(Duration::from_millis(16));
                }
                // Dropping the App joins its pool's threads — thread teardown is
                // not a frame either.
                TRACE_ON.store(false, Ordering::SeqCst);
                drop(a);
            }
            TRACE_ON.store(true, Ordering::SeqCst);
        },
    );

    // (6) The census's debug-profile OTHER: in a DEBUG build the colored-serial
    // pile (S1b) carries exactly one OTHER acquisition on every step, and the
    // release build carries none. Armed over sixteen warmed steps with the
    // OTHER-only filter, so a release run prints an empty table (which is the
    // release claim) and a debug run names the site.
    let mut serial = build_pile(4, true, 4, 2);
    for _ in 0..PHYS_WARM {
        serial.step();
    }
    trace_window(
        "OTHER-class acquisitions over 16 warmed COLORED-SERIAL steps (S1b's shape)",
        64,
        FILTER_OTHER,
        || {
            for _ in 0..16 {
                serial.step();
            }
        },
    );

    // (7) The rare non-Local OTHER that section G names by layout. FILTER_RARE
    // pays a capture ONLY on an OTHER that is not the 2304-byte epoch `Local`,
    // so the common first touch runs at full speed and the timing that decides
    // whether the rare object appears is barely disturbed — which is exactly
    // what the plain OTHER filter in (5) could not promise.
    trace_window(
        "non-Local OTHER acquisitions across the frames of G_ROUNDS (128) fresh 2-system Apps (W=2)",
        64,
        FILTER_RARE,
        || {
            for _ in 0..G_ROUNDS {
                TRACE_ON.store(false, Ordering::SeqCst);
                let mut a = g_fresh_app();
                TRACE_ON.store(true, Ordering::SeqCst);
                for _ in 0..(G_WARM + G_STEADY) {
                    a.update_with_delta(Duration::from_millis(16));
                }
                TRACE_ON.store(false, Ordering::SeqCst);
                drop(a);
            }
            TRACE_ON.store(true, Ordering::SeqCst);
        },
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// F — the rare OTHER frames, counted without a backtrace
// ═══════════════════════════════════════════════════════════════════════════

/// Are the census's rare `OTHER`-class frames a steady-state RATE or a
/// per-pool FIRST TOUCH?
///
/// The census saw a frame in a few hundred carry one to three `OTHER`
/// acquisitions, in warm-up and occasionally inside the steady window, and only
/// in App scenes on a freshly-built pool. The two readings have different
/// consequences for the gate: a rate means the pinned per-frame maximum must
/// carry it forever; a first touch means it is bounded by the pool's thread
/// count and a longer warm-up absorbs it. So: one App, one pool, a very long
/// run, and the `OTHER` total per 256-frame block. A rate is flat across the
/// blocks; a first touch front-loads and then stops.
fn f_rare_other_is_first_touch(rows: &mut Vec<Row>) {
    say!(
        "\n── F. RARE OTHER FRAMES — rate or first touch? (one 4-system App, W=2, 16 × 256 frames) ──"
    );
    const BLOCK: usize = 256;
    const BLOCKS: usize = 16;
    let mut totals = 0u64;
    let mut per_block = [0u64; BLOCKS];
    let mut a = App::with_pool(pool(2));
    seed_payload(a.world_mut(), ROWS);
    a.add_systems(plain0);
    a.add_systems(plain1);
    a.add_systems(filler0);
    a.add_systems(filler1);
    a.finish();
    for b in per_block.iter_mut() {
        let before = Snap::now();
        for _ in 0..BLOCK {
            a.update_with_delta(Duration::from_millis(16));
        }
        *b = Snap::now().since(before).other;
        totals += *b;
    }
    say_inline!("  OTHER per {BLOCK}-frame block: ");
    for b in &per_block {
        say_inline!("{b} ");
    }
    say!("  (total {totals} over {} frames)", BLOCK * BLOCKS);
    let late: u64 = per_block[BLOCKS / 2..].iter().sum();
    push(
        rows,
        "F",
        format!(
            "OTHER acquisitions in the LAST {} frames of one App",
            BLOCK * BLOCKS / 2
        ),
        Meas {
            mean: late as f64 / (BLOCK * BLOCKS / 2) as f64,
            other_max: late,
            ..Meas::default()
        },
        verdict::TRANSIENT,
    );
    // The claim this section exists to test: whatever the rare OTHER frames
    // are, they do not recur once the pool's threads have each been through
    // their first steal. If a late block carries any, it is a RATE, and the
    // census gate's OTHER headroom is not the one-off it is justified as.
    assert!(
        late <= 2,
        "F: the second half of a {}-frame run still carried {late} OTHER-class acquisitions — \
         the rare OTHER frames recur, so they are a steady-state rate and not a per-thread \
         first touch",
        BLOCK * BLOCKS
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// G — what the rare OTHER objects ARE, by layout, under undisturbed timing
// ═══════════════════════════════════════════════════════════════════════════

/// Fresh Apps section G drives.
const G_ROUNDS: u64 = 128;
/// The census's warm-up budget and steady window, so G's regimes are the
/// census's regimes.
const G_WARM: u64 = 64;
const G_STEADY: u64 = 256;

/// One fresh App in the census's `S0, 2 systems` shape (2 workers, 4096 rows) —
/// the shape the three-`OTHER` frame was seen in.
fn g_fresh_app() -> App {
    let mut a = App::with_pool(pool(2));
    seed_payload(a.world_mut(), ROWS);
    a.add_systems(plain0);
    a.add_systems(plain1);
    a.finish();
    // Setup, by the census's own regime definition: not logged.
    a.update_with_delta(Duration::from_millis(16));
    a
}

/// Names every `OTHER` acquisition in the frames of [`G_ROUNDS`] fresh Apps by
/// its layout, and counts the frames that carried two or more.
///
/// The claim this section tests is the census gate's `OTHER` allowance of two
/// per pool thread: whatever lands in `OTHER` after setup must be a per-thread
/// first touch, bounded per App, and not a rate.
fn g_rare_other_layouts() {
    say!(
        "\n── G. RARE OTHER OBJECTS BY LAYOUT ({G_ROUNDS} fresh 2-system Apps, W=2, {G_WARM} warm-up + {G_STEADY} steady frames each; log armed, no backtrace) ──"
    );
    for e in OTHER_LOG.iter() {
        e.store(0, Ordering::Relaxed);
    }
    OTHER_LOG_LEN.store(0, Ordering::SeqCst);
    let before_all = Snap::now().other;
    let mut frames_seen = 0u64;
    for round in 0..G_ROUNDS {
        let mut a = g_fresh_app();
        OTHER_LOG_ON.store(true, Ordering::SeqCst);
        for frame in 0..(G_WARM + G_STEADY) {
            OTHER_LOG_FRAME.store((round << 16) | frame, Ordering::SeqCst);
            a.update_with_delta(Duration::from_millis(16));
            frames_seen += 1;
        }
        // Dropping the App joins its threads — thread teardown is not a frame.
        OTHER_LOG_ON.store(false, Ordering::SeqCst);
        drop(a);
    }
    let len = OTHER_LOG_LEN.load(Ordering::SeqCst);
    let logged = len.min(OTHER_LOG_CAP);
    let entries: Vec<u64> = OTHER_LOG[..logged]
        .iter()
        .map(|e| e.load(Ordering::Relaxed))
        .collect();
    // Anti-vacuity: the log and the class counter must agree on how many
    // OTHERs there were (the class counter also saw the setup ones, which the
    // log was disarmed for, so it is an upper bound).
    let other_total = Snap::now().other - before_all;
    assert!(
        len as u64 <= other_total,
        "G: the log recorded {len} OTHER acquisitions but the class counter saw only {other_total}"
    );

    // By layout, split by regime.
    let mut by_layout: Vec<(u64, u64, u64, u64)> = Vec::with_capacity(16); // (size, align, warm, steady)
    for &e in &entries {
        let size = (e >> 8) & 0xFFFF_FFFF;
        let align = 1u64 << (e & 0xFF);
        let frame = (e >> 40) & 0xFFFF;
        let steady = frame >= G_WARM;
        match by_layout
            .iter_mut()
            .find(|(s, a, _, _)| *s == size && *a == align)
        {
            Some(row) => {
                if steady {
                    row.3 += 1
                } else {
                    row.2 += 1
                }
            }
            None => by_layout.push((size, align, u64::from(!steady), u64::from(steady))),
        }
    }
    by_layout.sort_unstable_by_key(|r| std::cmp::Reverse(r.2 + r.3));
    say!(
        "  {len} OTHER acquisitions logged over {frames_seen} frames ({} dropped past the log's capacity)",
        len.saturating_sub(OTHER_LOG_CAP)
    );
    say!("  {:>8} {:>6} {:>10} {:>10}", "size", "align", "warm-up", "steady");
    for (size, align, warm, steady) in &by_layout {
        let name = if *size == EPOCH_LOCAL_BYTES as u64 && *align == 128 {
            "  crossbeam-epoch `Local` (Collector::register, a thread's first pin)"
        } else if *size == THREAD_NAME_UTF16_BYTES as u64 && *align == 2 {
            "  std thread_start: UTF-16 worker name for SetThreadDescription (a worker booting after setup)"
        } else {
            "  ← NOT the epoch Local: aim BOYKO_ALLOC_TRACE=1 (FILTER_RARE) at it"
        };
        say!("  {size:>8} {align:>6} {warm:>10} {steady:>10}{name}");
    }

    // Per App: how many epoch Locals, and the worst frame's OTHER count.
    let mut worst_frame = 0u64;
    let mut frames_ge2 = 0u64;
    let mut max_locals_per_app = 0u64;
    let mut rare_per_app_max = 0u64;
    for round in 0..G_ROUNDS {
        let mine: Vec<u64> = entries
            .iter()
            .copied()
            .filter(|e| (e >> 56) == round)
            .collect();
        let locals = mine
            .iter()
            .filter(|e| ((*e >> 8) & 0xFFFF_FFFF) == EPOCH_LOCAL_BYTES as u64)
            .count() as u64;
        max_locals_per_app = max_locals_per_app.max(locals);
        rare_per_app_max = rare_per_app_max.max(mine.len() as u64 - locals);
        for frame in 0..(G_WARM + G_STEADY) {
            let n = mine
                .iter()
                .filter(|e| ((*e >> 40) & 0xFFFF) == frame)
                .count() as u64;
            worst_frame = worst_frame.max(n);
            if n >= 2 {
                frames_ge2 += 1;
            }
        }
    }
    say!(
        "  per App: epoch Locals max {max_locals_per_app} (the pool has 2 threads), non-Local OTHERs max {rare_per_app_max}; worst single frame {worst_frame} OTHER, frames carrying >= 2: {frames_ge2}"
    );
    assert!(
        len <= OTHER_LOG_CAP,
        "G: {len} OTHER acquisitions over {frames_seen} post-setup frames overflowed a {OTHER_LOG_CAP}-entry log — that is a RATE, not a per-thread first touch"
    );
    assert!(
        max_locals_per_app <= 2,
        "G: one App registered {max_locals_per_app} crossbeam-epoch Locals on a 2-thread pool — the first touch is not once per thread"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// The single entry point
// ═══════════════════════════════════════════════════════════════════════════

/// The whole attribution, every section in sequence, on the thread that calls
/// it. ONE entry point for the census's reason — the counter is process-global,
/// so two sections at once would each measure the other — and, since the target
/// became `harness = false` (see [`main`]), with nothing else in the process.
fn frame_allocation_attribution() {
    // ── Anti-vacuity: the counter moves on all three axes before anything is
    // attributed to it. A dead counter attributes everything to nothing.
    let before = Snap::now();
    let mut v: Vec<u8> = Vec::with_capacity(64);
    v.resize(64, 7);
    v.reserve(1 << 16);
    black_box(&v);
    drop(v);
    let d = Snap::now().since(before);
    assert!(d.alloc >= 1, "the alloc counter is dead (saw {})", d.alloc);
    assert!(
        d.realloc >= 1,
        "the realloc counter is dead (saw {})",
        d.realloc
    );
    assert!(
        d.bytes >= (1 << 16),
        "the byte counter is dead (saw {})",
        d.bytes
    );
    assert_eq!(
        d.alloc,
        d.scope + d.chunk + d.inj + d.other,
        "the four classes must partition every fresh acquisition"
    );
    say!(
        "[anti-vacuity] counter live: alloc={} realloc={} bytes={} (OTHER={})",
        d.alloc, d.realloc, d.bytes, d.other
    );

    let mut rows: Vec<Row> = Vec::with_capacity(64);
    let prim = a_dispatch_primitives(&mut rows);
    b_per_system_windows(&mut rows, &prim);
    let app_base = c_app_subsystem_deltas(&mut rows);
    let (phys_serial, phys_parallel) = d_physics_deltas(&mut rows);
    f_rare_other_is_first_touch(&mut rows);
    g_rare_other_layouts();
    e_call_sites();

    // ══ The deliverable ══
    say!("\n\n══════════ ATTRIBUTION — steady-state acquisitions per frame ══════════");
    say!(
        "{:<3} {:<66} {:>9} {:>5} {:>5} {:>8} {:>8} {:>7} {:>7} {:>10}  class",
        "§", "what", "mean", "min", "max", "scope", "chunk", "inj", "OTHER", "bytes"
    );
    for r in &rows {
        say!(
            "{:<3} {:<66} {:>9.3} {:>5} {:>5} {:>8.3} {:>8.3} {:>7.3} {:>7.3} {:>10.0}  {}",
            r.section,
            r.what,
            r.m.mean,
            r.m.min,
            r.m.max,
            r.m.scope,
            r.m.chunk,
            r.m.inj,
            r.m.other,
            r.m.bytes_mean,
            r.verdict
        );
    }
    say!("═══════════════════════════════════════════════════════════════════════");

    // ── The deliverable's third column, per SITE rather than per arm ──
    //
    // The rows above are what each configuration COSTS; this is what each
    // named site IS. The site names come from the layout classes (always on)
    // and the `BOYKO_ALLOC_TRACE=1` tables (diagnostic); the classification is
    // the judgement the brief asked for, written here so it travels with the
    // instrument that produced it.
    say!("\n══════════ CLASSIFICATION — every steady-state site, by what it is ══════════");
    for (site, class, why) in [
        (
            "Box<ScopeShared> — 1 per scope frame, 256 B (thread_pool.rs, install / scope)",
            verdict::SHOULD_NOT,
            "One per `Schedule::run` install frame AND one per nested `pool.scope` (a \
             `par_iter` fan-out, one dispatched colour of one solver pass). Freed at the join \
             and never reused. A pool-owned free list of scope frames — the joiner is always \
             the thread that frees it — takes it to zero; the colored solver's own comment \
             already names 'a true zero-alloc reusable-scope threadpool API' as a filed \
             follow-up.",
        ),
        (
            "ScopeBlock chunk — >= 1 per SPAWNING scope, 4 KiB then doubling (block.rs grow)",
            verdict::SHOULD_NOT,
            "Stage 3b collapsed one cell per task into one chunk per scope, and that is the \
             whole of the KE16 improvement: an App frame of n systems went from 4 + n to 2. \
             But the chunk is freed at every join (`free_all`) and the next scope allocates a \
             fresh one on the same thread. A per-joiner chunk cache (keep the high-water \
             chunk, reset the cursor) makes the steady state zero; the chunk table already \
             has the shape for it.",
        ),
        (
            "crossbeam Injector block — 1 per 63 pushes from OUTSIDE the pool, 1520 B",
            verdict::TRANSIENT,
            "Queue-internal: the injector takes a block per lap and frees it when the head \
             leaves it. It is the whole of every scene's periodic `max = mean + 1`. Pushes \
             made by a worker go to its own Chase-Lev ring and allocate nothing, which is why \
             the parallel physics step — 1500+ pushes, all from workers — shows 0.125 of a \
             block per step. Not this engine's to remove short of replacing the queue.",
        ),
        (
            "first touch per pool thread — OTHER class, a handful per FRESH pool (section F)",
            verdict::TRANSIENT,
            "Two objects, both per pool thread: crossbeam-epoch's `Local` on the thread's \
             first steal/pin (2304 B), and — when the thread finishes BOOTING after setup — \
             std's 30-byte UTF-16 copy of its name for `SetThreadDescription` (section G, \
             FILTER_RARE trace). Bounded by the pool's thread count and absent from a long \
             run's second half (section F), but NOT confined to warm-up: most first steals on \
             a fresh 2-worker App land past frame 64, inside the census's steady window. A \
             setup cost that lands in frames, not a frame cost — `ThreadPoolBuilder::build` \
             neither waits for its threads to start nor pre-registers them with the epoch \
             collector, and either would move both objects into setup.",
        ),
        (
            "the recycled-entity stack (CommandQueue::apply -> DespawnCommand) — EM2′",
            verdict::FREE,
            "Was the ONLY growable in the whole census, and unbounded: deferred despawns pushed \
             an id per despawn onto a `Vec` while deferred spawns reserved FRESH ids through the \
             worker counter (EM2 forbade a worker to pop the free list), so the list and the \
             id-slot store grew one entry per despawn forever on a flat population. EM2′ made \
             the list a claimable stack on a `VmColumn` that `Commands::spawn` pops with one \
             `fetch_sub`: the churn now reuses its ids (C6: the list stays at one frame's \
             despawns, the slot store does not move) and the `realloc` is gone. Coverage \
             boundary: the stack's storage is `VirtualAlloc`-backed, so this counter would NOT \
             see the leak come back — C6's lengths and `em_deferred_recycle.rs` would.",
        ),
        (
            "the event lane, change detection, query iteration, every physics buffer",
            verdict::FREE,
            "Measured zero HEAP acquisitions. `update_events` with the EveryFrame policy, 4096 \
             `Mut<T>` writes plus a `Changed<T>` query, and a 1240-body physics step all add \
             +0.000 over an identical no-op baseline. Every buffer in the step is \
             capacity-reused; the arena's premise holds everywhere except the dispatch objects. \
             Coverage boundary: the counter sees `GlobalAlloc` only, and the ECS columns grow by \
             `VmReservation::commit` (`VirtualAlloc(MEM_COMMIT)`), which it does not see — so \
             this is zero heap allocations, not zero memory growth.",
        ),
        (
            "PRE-3B ONLY: scratch Worker<Task> deque — 3 per scope frame (scope.rs, Scope::drop)",
            verdict::FREE,
            "Gone on the KE16 pool: `join_on_worker` / `join_external` build no deque. Listed \
             so the before/after is on the record — on `ad0ebea4` it was three quarters of \
             the fixed frame term.",
        ),
    ] {
        say!("\n  {site}\n    → {class}\n      {why}");
    }
    say!("\n═══════════════════════════════════════════════════════════════════════");
    say!(
        "A 4-system App frame costs {app_base:.3}; a serial physics step {phys_serial:.3}; the \
         SAME step with parallel_solve on costs {phys_parallel:.1}. An empty install frame is \
         {:.3} acquisitions and one that spawns is {:.3}, so every figure above is a count of \
         dispatch objects and not of game work.",
        prim.empty_frame, prim.spawning_frame
    );
    say!(
        "Counts only. No wall-clock figure is produced anywhere in this binary, and none may \
         be derived from it."
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// The runner — `harness = false`, with libtest's command-line contract
// ═══════════════════════════════════════════════════════════════════════════

/// The name libtest listed and filtered this binary under while it was a
/// `#[test]`; kept so every existing filter and log search still finds it.
const TEST_NAME: &str = "frame_allocation_attribution";

/// The subset of libtest's command line a runner actually sends. Anything else
/// that starts with `-` is accepted and ignored, as libtest's own no-op flags
/// (`--test-threads`, `--color`, `-q`, …) are for a one-test binary.
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
                // libtest's flags that take their value as the NEXT argument:
                // consume it, or it would be read as a name filter.
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

    /// libtest's selection rule for one non-ignored test: excluded by
    /// `--ignored`, by any `--skip` match, and by name filters none of which
    /// matches (substring, or equality under `--exact`).
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

/// Runs the attribution ONLY, on the main thread, with nothing else in the
/// process — see "How it runs" in the header. Everything `main` allocates is
/// allocated before [`frame_allocation_attribution`] opens its first window or
/// after it has closed its last.
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
                "{TEST_NAME}: not run under Miri — it counts the native allocator over \
                 multi-second scenes, which is not Miri's regime"
            );
        }
        // libtest's own summary for a filtered-out test: `running 0 tests` is
        // the line a reader checks, and it must not read as a pass.
        println!(
            "\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; \
             1 filtered out\n"
        );
        return;
    }

    STREAM.store(cli.nocapture, Ordering::Relaxed);
    HELD.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .reserve(HELD_RESERVE);
    // A failing assertion prints the report that led up to it before its own
    // message, as libtest's capture did.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        dump_held();
        default_hook(info);
    }));

    println!("\nrunning 1 test");
    let passed = std::panic::catch_unwind(frame_allocation_attribution).is_ok();
    if cli.show_output || !passed {
        dump_held();
    }
    if passed {
        println!(
            "test {TEST_NAME} ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; \
             0 measured; 0 filtered out\n"
        );
    } else {
        println!(
            "test {TEST_NAME} ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; \
             0 measured; 0 filtered out\n"
        );
        // libtest's exit code for a failed run.
        std::process::exit(101);
    }
}
