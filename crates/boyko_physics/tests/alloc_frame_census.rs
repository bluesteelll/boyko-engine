//! **The frame allocation census, and its gate.** How many heap allocations
//! does a steady-state frame actually make — and which object is each one?
//!
//! # Why this binary exists
//!
//! The tree ships some twenty counting test binaries (the phase-1 survey
//! counted twenty on `05fcbd1d`; KE16 adds `block_allocation_receipts.rs`).
//! Every one of them either
//! bypasses `Schedule::run` (an isolated helper, `run_system_once`, a direct
//! solver call) or drives `Schedule::run` and then **subtracts it as a
//! baseline**. So twenty gates each prove "zero in my window" and the frame
//! total is unmeasured. This binary measures the total instead of subtracting
//! it: it installs its own counting `#[global_allocator]` and reports what a
//! whole frame costs, in three regimes.
//!
//! # Why it lives in `boyko_physics/tests/`
//!
//! It has to reach `boyko_physics` (the rigid pile is the largest real scene
//! the tree can drive headless) AND `boyko_ecs` (the `App` frame funnel, the
//! executor, `Commands`, events, `par_iter`). `boyko_app` does not depend on
//! `boyko_physics`, and `boyko_ecs` cannot depend on it either. This crate's
//! dev graph is the only one that spans both.
//!
//! # Three regimes, never one number
//!
//! * **SETUP** — world build + spawn + the first schedule run. Allocations here
//!   are expected; the number is reported, never asserted.
//! * **WARM-UP** — the frames where amortised growth is still happening. `K` is
//!   *derived*, not assumed: `K` is one past the LAST frame in the whole run
//!   whose allocation count exceeded the steady window's own maximum.
//! * **STEADY STATE** — mean, min and **max** over the last
//!   [`STEADY_FRAMES`] frames. The max is the number that matters: one frame in
//!   two hundred that allocates is a hitch, and a mean hides it.
//!
//! Allocations, reallocations, deallocations and bytes are counted separately.
//! A realloc is the signature of a `Vec` growing and is named on its own row.
//!
//! # Anti-vacuity (both halves)
//!
//! A zero from a schedule that ran nothing is this repository's most-catalogued
//! failure. Two independent guards, and neither is optional:
//!
//! 1. **The counter is LIVE.** [`counter_is_live`] proves all four counters
//!    (alloc / realloc / dealloc / bytes) move. Then EVERY scene ends with a
//!    probe frame that makes a deliberate 1 MiB allocation *inside the measured
//!    window* and asserts the window saw it. And [`worker_thread_allocations_are_counted`]
//!    proves the counter is process-global by allocating from a SYSTEM BODY
//!    running on a worker thread.
//! 2. **The scene is DOING something.** Every scene asserts a scene-side
//!    invariant: bodies moved and contacts were generated (S1), entities were
//!    spawned and despawned (S2), events were delivered and `Changed` matched
//!    (S3), exactly N systems ran exactly once per frame (S0).
//!
//! # What it measured — two trees, side by side (2026-09-10, release)
//!
//! Recorded here because the harness is the thing that produced them, and a
//! number without its instrument is not a measurement. Counts are heap
//! ACQUISITIONS (`alloc` + `realloc`) per steady-state frame, mean / MAX over
//! the 256-frame window.
//!
//! * **BEFORE** — `feat/ecs-native-storage` @ `ad0ebea4` (`D:/wt/ecsnative`), a
//!   thread pool that PREDATES KE16 Stage 3b: one heap cell per spawned task,
//!   and a scratch `crossbeam_deque::Worker` built on every `Scope::drop`.
//! * **AFTER** — `merge/ke16-into-ecsnative` @ `ca582e72` (`D:/wt/joltab`), the
//!   same ECS and physics code with the shipped KE16 pool merged in: scoped
//!   cells are emplaced in the per-scope `ScopeBlock`. Re-confirmed 2026-09-11
//!   at `d11962a9`, which adds only the physics bench's off-by-default
//!   `bench-alloc` feature and a doc — no code on any path this binary runs. The two trees differ in
//!   `boyko_threadpool` and in comments/dead `KE16_SPAWN_BATCH` arms around the
//!   dispatch sites — the dispatch SHAPE (what spawns where) is unchanged.
//!
//! ```text
//! scene                                   BEFORE (ad0ebea4)   AFTER (ca582e72)
//! S0  0 systems (control)                     0.000 / 0          0.000 / 0
//! S0  1 system                                5.016 / 6          2.016 / 3
//! S0  2 systems                               6.031 / 7          2.031 / 3 (6)
//! S0  4 systems                               8.062 / 9          2.066 / 3 (4)
//! S0  8 systems                              12.125 / 13         2.125 / 3
//! S0  16 systems                             20.254 / 21         2.254 / 3
//! S0b 4 Main + 4 Fixed, 1 substep            16.125 / 17         4.125 / 5 (6)
//! S2  churn + par_iter                       14.098 / 16         4.039 / 5
//! S3  query + event loop                      8.066 / 9          2.062 / 3 (4)
//! S1a pile, default pipeline, serial         11.109 / 12         2.109 / 3
//! S1b pile, colored, parallel OFF            12.125 / 13         2.125 / 3
//! S1c pile, colored, parallel ON (W=4)     2670.98  / 2724     331.80  / 339   (*)
//! ```
//!
//! **(\*) S1c's row is a FIXED WINDOW, not the pile's steady state.** Both
//! columns are driven steps 64..320 of a freshly built pile, and that window is
//! deterministic — the colored solve is bit-identical for any worker count and
//! the dispatch decision reads only the contact set — which is the whole reason
//! nine runs agreed to three decimals. A longer run does not stay there: the
//! pile keeps settling and the per-step count moves in whole dispatched
//! colours. Over 4,352 steady steps (the adjudication run, 2026-09-11,
//! reproduced the same day with this file's scene extended to 17 x 256 steady
//! steps) S1c read **302..339 per step, with 256-step block means ~307..332**
//! (331.8 in the first block — which IS the census window — 331.4 in the
//! second, 307.5 in the last; 316.1 over all 4,352), on 109..121 scope frames
//! (9 or 10 dispatched colours) and 193..217 chunks, `OTHER` 0, `realloc` 0.
//! No step of it exceeded the census window's own maxima (121 / 217 / 339).
//! Quote S1c as that range; 331.797 is the census window's figure and nothing
//! more. The debug pile (height 10) does the same over the same 4,352 steps:
//! 147..196 per step, 73..97 scope frames (6..8 colours), one chunk each,
//! block means 171.2..181.3.
//!
//! (AFTER is identical to three decimals across nine release runs — five on
//! 2026-09-10, four on 2026-09-11; S1c read 331.797 / 339 in every one, the
//! fixed window above — except
//! where a once-per-thread `OTHER` acquisition, below, happened to land inside
//! an App scene's window and added 1..3 to its MAX and a few thousandths to its
//! mean: S0 n=2 read 2.031..2.043 with max 3..6, S0 n=4 max 3..4, S0b
//! 4.125..4.133 with max 5..6, S2 4.039..4.047, S3 max 3..4. A parenthesised
//! MAX is that worst run. The dispatch classes never moved in any run.)
//!
//! **S2 after EM2′ (same tree plus the entity-id recycling fix, 2026-09-11,
//! release and debug alike): 4.031 / 5, `realloc` 0.** The 0.008 it lost is
//! exactly the two free-list reallocs the AFTER column's window carried
//! (2 / 256 frames); the dispatch classes are unchanged (2 scope + 2 chunk +
//! 0.031 injector).
//!
//! **The delta, attributed.** A scope frame (`pool.install` or a nested
//! `pool.scope`) cost `4 + (one cell per spawn)` BEFORE — a `Box<ScopeShared>`
//! plus the three allocations of the scratch deque, plus a cell per task. AFTER
//! it costs `1 + (one 4 KiB chunk if it spawns anything, doubling as cells
//! overflow)`: the scratch deque is gone (`join_on_worker` / `join_external`
//! build none) and the cells share the chunk. So an App frame went from `n + 4`
//! to a flat 2 whatever `n` is. S1c's 8.05x drop (over the fixed window) is the
//! same two terms over the window's 121 scope frames a step, plus a third: a
//! push made by a WORKER now lands in that
//! worker's own Chase-Lev ring (`place_task` -> `push_on_lane_no_wake`), which
//! allocates nothing, where BEFORE it went through the injector and cost a
//! 1520-byte block per 63 pushes (38 a traced step; AFTER, 0.125).
//!
//! # What an AFTER frame is made of — attributed COMPLETELY, by layout class
//!
//! Every acquisition is charged to one of four classes from its `Layout` alone
//! (see [`C_SCOPE`]); `alloc_frame_attribution.rs` checks each class against the
//! primitive that produces it and names every call site with backtraces.
//!
//! ```text
//! scene   scope/frame   chunk/frame        injector   OTHER            realloc
//! S0 n>=1   1 exact      1 exact            n/63       0 (first touch)  0
//! S0b       2 exact      2 exact            8/63       0 (first touch)  0
//! S2        2 exact      2 exact            2/63       0 (first touch)  0 (was 2, EM2′)
//! S3        1 exact      1 exact            4/63       0 (first touch)  0
//! S1a/S1b   1 exact      1 exact            ~7/63      0                0
//! S1c     121 (window) 205..217 (1.74/scope) 0.125     0                0
//! S1c long 109..121    193..217              —         0                0
//! ```
//!
//! * `scope` = the `Box<ScopeShared>` of `Schedule::run`'s install frame, of each
//!   `par_iter` fan-out, and of each dispatched colour in each of the solver's
//!   12 colour passes: `1 + 12 x` the dispatched colours, asserted on every
//!   steady frame. On the 1240-body pile that is `1 + 12 x 10 = 121` on every
//!   frame of the census window, and `1 + 12 x 9 = 109` once the pile settles
//!   further (the long-run row). Per stage: `Schedule::run`'s install frame owns
//!   1 scope + 1 chunk of every step and `physics_solve_colored` owns every
//!   other dispatch object — in the census window 120 scopes + ~209.7 chunks of
//!   the 331.8. ⚠ The backtrace trace that charged `physics_solve_colored`
//!   **99.5 %** was taken on a WARM-UP step, and the percentage belongs to that
//!   step's own numbers: **145 scope frames (1 + 12 passes x 12 dispatched
//!   colours) and 386 acquisitions**, of which the install frame's 2 are the
//!   other 0.5 % (the attribution binary's section E, `BOYKO_ALLOC_TRACE=1`:
//!   the 50th step of a fresh pile, after 48 serial warm-up steps and one
//!   parallel one; re-run 2026-09-11, 386 of 386 captured, the solve's 384 =
//!   144 scope boxes + 240 chunk grows).
//!   Every other physics system, the event lane, change detection and every
//!   system body own ZERO heap acquisitions (see "Coverage boundary").
//! * `OTHER` = everything that is not a dispatch object, and in steady state it
//!   is exactly one site: crossbeam-epoch's `Collector::register`, the
//!   thread-local `LocalHandle` a pool thread creates on its FIRST steal
//!   (2304 B, align 128). Once per thread, front-loaded, absent from the second
//!   half of a 4096-frame run — a setup cost that lands in a frame.
//! * `realloc` = ZERO in every scene since EM2′. It used to be
//!   `EntityMaster::free_entity_ids` growing under `CommandQueue::apply` ->
//!   `EcsMaster::delete_entity`, and that growth was unbounded: `Commands::spawn`
//!   reserved fresh ids through the worker counter and never popped the list the
//!   despawns filled, so on a flat population the list AND the id-slot store
//!   grew one entry per despawn forever. EM2′ made the free list a claimable
//!   stack that `Commands::spawn` pops with one `fetch_sub`, so the churn now
//!   reuses the ids it despawns: at rest the stack holds one frame's 64
//!   despawns and the slot store does not move.
//!
//! In a DEBUG build S1b and S1c carry one extra `OTHER` per step: the
//! `cfg!(debug_assertions)`-gated `debug_assert_coloring` scratch in
//! `ConstraintGraph::build` (the allocation `constraint_graph_o4_world.rs`
//! already tolerates). The debug S1c scene is the height-10 pile: 85..=97
//! scope frames (7 or 8 dispatched colours) in the census window and 73..=97
//! over 4,352 steps, one chunk each, MAX 196 — pinned with the release pin's
//! shape: the long run's envelope, no upward headroom.
//!
//! # The gate
//!
//! [`gate`] pins every scene's steady-state envelope at what it measured — per
//! CLASS, not as one total, because a total is exactly what a regression hides
//! under: the mutation that made each S0 probe system allocate one `Vec` per
//! frame took the 1- and 2-system frames to MAX 5 against a total pin of 7 —
//! GREEN on the total — and was caught only by their `OTHER` pins (256 and 513
//! over the window against a budget of 4). Re-run on the final file
//! 2026-09-11 with ONE `Box<u64>` per frame in `probe0` alone: six violations
//! (every S0 row with n >= 1 and S0b, 256..257 `OTHER` against 4), every MAX pin
//! still green. Headroom is zero wherever a count is
//! structural and is justified at each non-zero pin (see [`pins`]).
//!
//! S1c's number is DATA-DEPENDENT: its warm-up spans 290..387 as the pile
//! collapses, and after it the count moves in whole dispatched colours as the
//! contact set settles (302..339 per step over 4,352 steps). Quote it as a
//! range, never as a figure. Its pins are that long run's envelope with NO
//! upward headroom — scope 109..=121, chunk 193..=217, dispatch MAX 339:
//!
//! * **Downward** they keep what the long run reached (one colour below the
//!   census window), so a pile that settles further does not red.
//! * **Upward** they keep nothing. The first form of this pin allowed one
//!   dispatched colour either way, and no upward colour was ever observed in
//!   4,352 steady steps — while that one-colour allowance was exactly the size
//!   of a real regression: ONE extra `pool.scope` fan-out per colour pass
//!   (+12 scope frames and +12 chunks a step, 331.8 -> 355.8, MAX 363) sat
//!   inside it and stayed GREEN (adjudication, 2026-09-11). The per-frame
//!   structural assertion cannot see it either: `scope - 1 = 132` is still a
//!   multiple of the 12 passes, it just reads as eleven colours.
//! * **Why not pin EXACTLY against the dispatched colour count.** The count is
//!   not observable from outside `boyko_physics/src`: whether a colour is
//!   dispatched is decided from private constants (`MIN_PARALLEL_SLOTS_PER_COLOR`,
//!   `MIN_SLOTS_PER_CHUNK`, `CHUNKS_PER_WORKER`) over `ContactColumns`' private
//!   CSR. Recomputing it here would be a replica of the code under test, not an
//!   observation of it — the replica drifts with the solver and the gate then
//!   checks the replica.
//!
//! # Coverage boundary — the Rust heap, and only the Rust heap
//!
//! The counter is a `#[global_allocator]`, so it sees exactly what passes
//! through `GlobalAlloc`. The ECS's column storage does not: a `ComponentPool`
//! (and so every `ScratchColumn`, which is one) reserves its address range with
//! `VirtualAlloc(MEM_RESERVE)` (`mmap` on Unix) and grows by
//! `VmReservation::commit` -> `VirtualAlloc(MEM_COMMIT)`
//! (`boyko_ecs/src/ecs/memory/vm.rs`), and neither call is counted. So every
//! zero here — "every physics buffer is free", "the event lane is free" —
//! means **zero heap acquisitions, not zero memory growth**: a column that
//! committed one more granule every frame would read 0 in this census. Whether
//! committed memory is flat across the steady window is a separate question
//! that needs a commit-side counter, and this binary does not answer it.
//!
//! # Open
//!
//! * **One `OTHER` object is unnamed.** On 2026-09-11 the S0 n=2 window carried
//!   ONE frame of six acquisitions: 1 scope + 1 chunk + 1 injector block + 3
//!   `OTHER`, 10 888 bytes, which leaves 5 016 bytes of `OTHER` = two 2304-byte
//!   epoch `Local`s + 408 bytes. The 408 is not the 30-byte thread-boot object
//!   the attribution binary names, and 832 fresh Apps in the same shape under
//!   its layout log never reproduced a two-`OTHER` frame at all. It is inside
//!   the per-thread allowance (3 of 4) and is not a rate — the attribution
//!   binary's section F asserts a long run's second half carries none — but
//!   what it IS remains unmeasured.
//! * **The `Commands` id leak — FIXED by EM2′, and this census can no longer
//!   see it.** The leak used to surface here as S2's pinned `realloc` of 2 (the
//!   free list's `Vec` doubling). The recycled stack now lives on a `VmColumn`,
//!   which grows by `VirtualAlloc(MEM_COMMIT)` — outside this counter (see
//!   "Coverage boundary"). So S2's `realloc` pin of 0 would stay GREEN if the
//!   deferred route stopped recycling again: the growth would move entirely
//!   into uncounted commits. The gates that DO see that regression count ids,
//!   not bytes: `boyko_ecs/tests/em_deferred_recycle.rs` (T-RED-1: the free
//!   list stays <= one frame's despawns, the slot store and its commit frontier
//!   do not move, and each frame's spawns ARE the previous window's despawns)
//!   and the attribution binary's C6 (the same two lengths over 2048 frames of
//!   this churn).
//!
//! # How to run
//!
//! ```text
//! cargo test -p boyko-physics --release --test alloc_frame_census -- --nocapture
//! ```
//!
//! The target is `harness = false` (see `Cargo.toml`) and this file's [`main`]
//! is its whole runner. That is what makes the counter see ONLY the census:
//! under libtest the test ran on a spawned thread while libtest's main thread
//! stayed alive beside it, and that thread allocates on its own schedule — its
//! "has been running for over 60 seconds" notice was measured as 2 `OTHER` in
//! one frame of a long run. The census takes ~17..24 s in release and ~34 s in
//! debug today, under the line, but a loaded machine crosses it; with no
//! harness there is no second thread to allocate, at any duration. `main`
//! keeps the libtest contract a runner relies on — `running 1 test`, name
//! filters and `--exact` / `--skip`, `--list`, `--ignored` (runs nothing),
//! `--nocapture` / `RUST_TEST_NOCAPTURE` (stream the report; otherwise it is
//! held and printed only on failure or with `--show-output`), and exit code
//! 101 on failure. Under Miri the binary is built and runs nothing, as the
//! `#![cfg(not(miri))]` it replaces did.
//!
//! Release is the regime the question is about; a debug build carries
//! `debug_assert!` scratch allocations the shipped frame does not have, and the
//! scenes are scaled down under `debug_assertions` so a dev-profile sweep still
//! terminates. The gate is pinned for BOTH profiles (see [`RELEASE`]), so the
//! workspace's own debug `cargo test --workspace --all-targets --no-fail-fast`
//! runs it rather than skipping it.
//!
//! ⚠ This binary reports **counts, not timings**. It does not need a quiet
//! machine and no wall-clock number is produced anywhere in it.

// No `#![cfg(not(miri))]`: a `harness = false` target must have a `main` in
// every configuration, and a crate-level `cfg` would strip it. `main` returns
// before measuring anything under Miri instead.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::ThreadId;
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

/// `true` streams the report to stdout as it is produced (`--nocapture`, or
/// `RUST_TEST_NOCAPTURE`); `false` holds it in [`HELD`] and [`main`] prints it
/// only on failure or with `--show-output` — what libtest's capture did, so a
/// passing census does not flood a workspace `cargo test`.
static STREAM: AtomicBool = AtomicBool::new(false);
/// The held report. [`main`] reserves [`HELD_RESERVE`] bytes before the first
/// measured window, so appending never grows it during the run; every append
/// is outside a window regardless (see [`report`]).
static HELD: Mutex<String> = Mutex::new(String::new());
/// Larger than the whole report.
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
// The counting global allocator
// ═══════════════════════════════════════════════════════════════════════════

/// Process-global, always-on counters. Process-global rather than
/// thread-local **by design**: a frame's allocations are not all made on the
/// driving thread — a system body runs on a worker, and
/// [`worker_thread_allocations_are_counted`] pins that this counter sees them.
/// A thread-local counter — the kind most of the tree's zero-alloc gates use —
/// is blind to the half of the frame that is not the dispatcher.
static N_ALLOC: AtomicU64 = AtomicU64::new(0);
static N_REALLOC: AtomicU64 = AtomicU64::new(0);
static N_DEALLOC: AtomicU64 = AtomicU64::new(0);
static B_ALLOC: AtomicU64 = AtomicU64::new(0);
static B_REALLOC: AtomicU64 = AtomicU64::new(0);
static B_DEALLOC: AtomicU64 = AtomicU64::new(0);

// ── Layout classes: WHICH object each fresh acquisition is, with no backtrace ──
//
// Every fresh acquisition (`alloc` / `alloc_zeroed`; a `realloc` keeps its own
// axis above) is charged to exactly ONE of four classes, decided from its
// `Layout` alone. That is what lets the GATE attribute every steady-state frame
// completely without allocating, without symbolising, and without a
// diagnostic mode: the classifier is two compares on the allocation path, and
// the class the engine does NOT produce in steady state — [`C_OTHER`] — is
// pinned on its own.
//
// The predicates are exact for this tree and each one is backed by a site:
//
// * [`C_SCOPE`] — `align == 128 && size == 256`. The `Box<ScopeShared>` that
//   every `pool.install` and every nested `pool.scope` makes
//   (`boyko_threadpool/src/thread_pool.rs`, `install` and `scope`).
//   `ScopeShared` is `#[repr(C)]` and leads with a `CachePadded<AtomicUsize>`
//   (align 128 on x86_64 — the same fact
//   `boyko_threadpool/tests/block_allocation_receipts.rs` rests its chunk
//   predicate on), followed by three 8-byte words: 152 bytes, rounded up to the
//   alignment, 256. So this counter IS the number of scope frames opened. The
//   SIZE is part of the predicate on purpose: crossbeam-epoch's per-thread
//   `Local` is over-aligned too (it carries a `CachePadded` epoch) and is
//   allocated the first time a thread pins — a one-off per thread that must
//   land in `OTHER`, where it is visible, and not be counted as a scope.
// * [`C_CHUNK`] — `align == 64 && size.is_power_of_two() && size >= 4096`. A
//   Stage 3b `ScopeBlock` chunk (`boyko_threadpool/src/block.rs`, `grow`: every
//   chunk is `(CHUNK0 << e, CHUNK_ALIGN)` with `CHUNK0 = 4096`,
//   `CHUNK_ALIGN = 64`, `debug_assert`ed at the producer). A scope that spawns
//   at least one task takes at least one; a scope whose cells overflow 4 KiB
//   takes an 8 KiB one next, and so on.
// * [`C_INJ`] — `size == 1520 && align == 8`. A `crossbeam_deque::Injector`
//   block: a `next` pointer plus `BLOCK_CAP = 63` slots of `{ Task (16 B),
//   state (8 B) }` = `8 + 63 * 24` (crossbeam-deque 0.8.8, `deque.rs`
//   `const BLOCK_CAP`). Every scoped push lands in an injector, which takes a
//   block per 63 pushes and frees it when the head leaves it.
// * [`C_OTHER`] — everything else. In a steady-state frame of a scene that
//   neither grows a world nor calls a system that allocates, this is the class
//   that must be ZERO, and the gate pins it per scene.
//
// ⚠ The predicates are an ARGUMENT, not a type fact, so each has a positive
// control in the gate: a scene that spawns tasks must show `C_CHUNK > 0`, a
// scene that spawns more than 63 tasks over its window must show `C_INJ > 0`,
// and every frame must show `C_SCOPE >= 1`. If crossbeam changes its block
// size or `ScopeShared` loses its padding, the class empties, `C_OTHER` fills,
// and the gate reds on BOTH — never green from an emptied bucket.
static C_SCOPE: AtomicU64 = AtomicU64::new(0);
static C_CHUNK: AtomicU64 = AtomicU64::new(0);
static C_INJ: AtomicU64 = AtomicU64::new(0);
static C_OTHER: AtomicU64 = AtomicU64::new(0);

/// `ScopeBlock`'s base chunk size (`block.rs`'s `CHUNK0`), stated as a literal
/// and cross-checked against the crate's published receipt in
/// [`class_predicates_match_the_threadpool_receipt`].
const CHUNK0: usize = 4096;
/// `ScopeBlock`'s chunk alignment (`block.rs`'s `CHUNK_ALIGN`).
const CHUNK_ALIGN: usize = 64;
/// `crossbeam_deque::Injector`'s block size for a 16-byte `Task`.
const INJECTOR_BLOCK_BYTES: usize = 8 + 63 * 24;
/// `size_of::<ScopeShared>()` and its alignment (see [`C_SCOPE`]).
const SCOPE_SHARED_BYTES: usize = 256;
const SCOPE_SHARED_ALIGN: usize = 128;

/// Charges one fresh acquisition to its class. Two compares and one relaxed
/// RMW; allocates nothing, so it is safe inside the global allocator.
#[inline]
fn classify(layout: Layout) {
    let (size, align) = (layout.size(), layout.align());
    let class = if align == SCOPE_SHARED_ALIGN && size == SCOPE_SHARED_BYTES {
        &C_SCOPE
    } else if align == CHUNK_ALIGN && size.is_power_of_two() && size >= CHUNK0 {
        &C_CHUNK
    } else if size == INJECTOR_BLOCK_BYTES && align == 8 {
        &C_INJ
    } else {
        &C_OTHER
    };
    class.fetch_add(1, Ordering::Relaxed);
}

/// Delegates every request to `System` and records it. `realloc` is counted on
/// its own axis rather than folded into acquisitions: it is the signature of a
/// `Vec` growing, which is exactly the shape the arena exists to remove.
struct CountingAlloc;

// SAFETY: pure delegation to `System` with relaxed counter side effects; every
// layout / pointer contract is forwarded unchanged, and the counters never
// allocate (they are `static` atomics), so no re-entrancy is possible.
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

/// One reading of the six counters.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Snap {
    alloc: u64,
    realloc: u64,
    dealloc: u64,
    alloc_bytes: u64,
    realloc_bytes: u64,
    dealloc_bytes: u64,
    /// Fresh acquisitions by layout class; `scope + chunk + inj + other == alloc`.
    scope: u64,
    chunk: u64,
    inj: u64,
    other: u64,
}

impl Snap {
    /// Reads all six counters.
    ///
    /// `SeqCst` on the read side, `Relaxed` on the increment side: the real
    /// happens-before edge is the pool's own join (a `Schedule::run` returns
    /// only after every spawned task has completed), so this read is ordered
    /// after every worker increment that belongs to the window regardless.
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
            other: C_OTHER.load(Ordering::SeqCst),
        }
    }

    /// `self - base`, componentwise.
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
            other: self.other - base.other,
        }
    }

    /// Total heap ACQUISITIONS: a realloc is an acquisition too.
    fn acquisitions(self) -> u64 {
        self.alloc + self.realloc
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Regimes
// ═══════════════════════════════════════════════════════════════════════════

/// Frames given to warm-up before the steady window opens. `K` is measured
/// inside this budget, never assumed to equal it.
const WARM_BUDGET: usize = 64;
/// The steady window. The brief's floor is 200; 256 keeps a power of two.
const STEADY_FRAMES: usize = 256;
/// Total driven frames per scene.
const TOTAL_FRAMES: usize = WARM_BUDGET + STEADY_FRAMES;
/// The deliberate allocation the in-window liveness probe makes. Large enough
/// that no scene's own byte traffic can be mistaken for it.
const PROBE_BYTES: usize = 1 << 20;

/// Drives `step` for [`TOTAL_FRAMES`] frames, recording the six-counter delta
/// of each one into `out` (which MUST already have the capacity, so the
/// recording itself never allocates inside the run).
fn drive<F: FnMut()>(mut step: F, out: &mut Vec<Snap>) {
    assert!(
        out.capacity() >= TOTAL_FRAMES,
        "sample buffer must be pre-sized or the harness allocates inside its own window"
    );
    out.clear();
    for _ in 0..TOTAL_FRAMES {
        let before = Snap::now();
        step();
        out.push(Snap::now().since(before));
    }
}

/// Runs two more frames: a plain control, then one that makes a deliberate
/// [`PROBE_BYTES`] allocation INSIDE the measured window. Asserts the window saw
/// it — this is what makes every zero above meaningful.
///
/// # Why the assertion is on a NESTED snapshot, not on the frame-to-frame delta
///
/// The first form of this probe compared `live.alloc` against `control.alloc`
/// and FAILED on a dev-profile run at `control=15, live=15` — not because the
/// counter was dead, but because the control frame happened to carry the
/// periodic +1 injector block, so the frame's own jitter exactly cancelled the
/// probe. A liveness check that a scene's own noise can cancel is not a
/// liveness check. The counts are therefore asserted on a snapshot pair taken
/// AROUND the deliberate allocation, still inside the open window, which no
/// frame jitter can move; the window TOTAL is separately asserted to contain
/// the probe's bytes, which is the claim about the window itself.
fn liveness_probe<F: FnMut()>(label: &str, samples: &[Snap], mut step: F) -> (Snap, Snap) {
    let a = Snap::now();
    step();
    let control = Snap::now().since(a);

    let window_open = Snap::now();
    step();

    let inner_open = Snap::now();
    let mut v: Vec<u8> = Vec::with_capacity(PROBE_BYTES);
    v.resize(PROBE_BYTES, 0xA5);
    black_box(&v);
    let sum = v[0] as u64 + v[PROBE_BYTES - 1] as u64;
    drop(v);
    let inner = Snap::now().since(inner_open);

    let live = Snap::now().since(window_open);

    assert_eq!(
        sum,
        0xA5 * 2,
        "the probe buffer must actually have been written"
    );
    assert!(
        inner.alloc >= 1,
        "{label}: ANTI-VACUITY FAILED — the alloc counter did not see a deliberate allocation made inside the measured window (saw {})",
        inner.alloc
    );
    assert!(
        inner.dealloc >= 1,
        "{label}: ANTI-VACUITY FAILED — the dealloc counter did not see the probe's free (saw {})",
        inner.dealloc
    );
    assert!(
        inner.alloc_bytes >= PROBE_BYTES as u64,
        "{label}: ANTI-VACUITY FAILED — the byte counter did not see {PROBE_BYTES} probe bytes (saw {})",
        inner.alloc_bytes
    );
    // And the FRAME TOTAL really accumulated it. The floor is the steady
    // window's own MINIMUM frame, not the adjacent control frame: an earlier
    // form compared against the control and failed on a release run at
    // `control=5584, live=1052640` — the control frame had caught the churn
    // scene's occasional 262 KB spike, so the scene's own jitter, not a dead
    // counter, decided the assertion. A floor taken from the measured minimum
    // cannot be moved by that jitter, and the claim survives: a frame carrying
    // the probe must cost at least the cheapest frame plus the probe.
    let steady_min_bytes =
        Stat::of(&samples[WARM_BUDGET..], |s| s.alloc_bytes + s.realloc_bytes).min;
    assert!(
        live.alloc_bytes + live.realloc_bytes >= steady_min_bytes + PROBE_BYTES as u64,
        "{label}: ANTI-VACUITY FAILED — the WINDOW total did not grow by the probe's {PROBE_BYTES} bytes (steady min={steady_min_bytes}, live={})",
        live.alloc_bytes + live.realloc_bytes
    );
    (control, live)
}

/// The per-counter summary of one window.
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

/// `K` — where warm-up ends, DERIVED: one past the last frame in the whole run
/// whose acquisition count exceeded the steady window's own maximum. If nothing
/// in the run exceeds it, `K = 0` (the scene was born steady).
fn settle_index(samples: &[Snap], steady_max: u64) -> usize {
    let mut k = 0usize;
    for (i, s) in samples.iter().enumerate() {
        if s.acquisitions() > steady_max {
            k = i + 1;
        }
    }
    k
}

/// A frequency table over the steady window's acquisition counts. No `HashMap`
/// (workspace-banned); a sorted `Vec` of pairs is the whole structure.
fn histogram(samples: &[Snap]) -> Vec<(u64, usize)> {
    let mut h: Vec<(u64, usize)> = Vec::with_capacity(16);
    for s in samples {
        let v = s.acquisitions();
        match h.iter_mut().find(|(k, _)| *k == v) {
            Some((_, c)) => *c += 1,
            None => h.push((v, 1)),
        }
    }
    h.sort_unstable_by_key(|(k, _)| *k);
    h
}

/// One line of the closing summary table.
struct Row {
    label: String,
    k: usize,
    mean: f64,
    min: u64,
    max: u64,
    realloc_max: u64,
    bytes_mean: f64,
    setup_alloc: u64,
    /// Per-frame MAX of every acquisition that is NOT in the `OTHER` class —
    /// the dispatch objects plus `realloc`s.
    dispatch_max: u64,
    /// `realloc`s over the whole steady window.
    realloc_sum: u64,
    /// Per-class steady-state figures: `(mean, min, max, sum)`.
    scope: (f64, u64, u64, u64),
    chunk: (f64, u64, u64, u64),
    inj: (f64, u64, u64, u64),
    other: (f64, u64, u64, u64),
}

/// Prints the three-regime report and returns its summary row. Every `say!`
/// here is OUTSIDE every measured window (formatting allocates).
fn report(
    label: &str,
    note: &str,
    setup: Snap,
    samples: &[Snap],
    control: Snap,
    live: Snap,
) -> Row {
    let steady = &samples[WARM_BUDGET..];
    let acq = Stat::of(steady, |s| s.acquisitions());
    let alloc = Stat::of(steady, |s| s.alloc);
    let realloc = Stat::of(steady, |s| s.realloc);
    let dealloc = Stat::of(steady, |s| s.dealloc);
    let bytes = Stat::of(steady, |s| s.alloc_bytes + s.realloc_bytes);
    let c_scope = Stat::of(steady, |s| s.scope);
    let c_chunk = Stat::of(steady, |s| s.chunk);
    let c_inj = Stat::of(steady, |s| s.inj);
    let c_other = Stat::of(steady, |s| s.other);
    let dispatch = Stat::of(steady, |s| s.acquisitions() - s.other);
    let k = settle_index(samples, acq.max);

    say!("\n╔══════════════════════════════════════════════════════════════════════");
    say!("║ {label}");
    say!("║ {note}");
    say!("╠══ SETUP (world build + spawn + first run; expected, not asserted) ═══");
    say!(
        "║   alloc={:<8} realloc={:<6} dealloc={:<8} bytes_acquired={}",
        setup.alloc,
        setup.realloc,
        setup.dealloc,
        setup.alloc_bytes + setup.realloc_bytes
    );
    say!("╠══ WARM-UP (K derived, not assumed) ══════════════════════════════════");
    say!("║   K = {k}   (last frame above the steady max, +1; budget was {WARM_BUDGET})");
    let show = k.clamp(8, 24);
    say_inline!("║   acquisitions/frame, frames 0..{show}: ");
    for s in &samples[..show] {
        say_inline!("{} ", s.acquisitions());
    }
    say!();
    if k > 0 {
        let pre = Stat::of(&samples[..k], |s| s.acquisitions());
        say!(
            "║   warm-up total = {} acquisitions over {k} frame(s), peak {}",
            pre.sum, pre.max
        );
        // WHAT the warm-up frames above the steady max were, by class — so a
        // late K is attributable instead of merely reported.
        let mut over = Snap::default();
        let mut n_over = 0usize;
        for s in samples[..k].iter().filter(|s| s.acquisitions() > acq.max) {
            over.alloc += s.alloc;
            over.realloc += s.realloc;
            over.scope += s.scope;
            over.chunk += s.chunk;
            over.inj += s.inj;
            over.other += s.other;
            n_over += 1;
        }
        say!(
            "║   the {n_over} warm-up frame(s) above the steady max, by class: scope={} chunk={}              injector={} OTHER={} realloc={}",
            over.scope, over.chunk, over.inj, over.other, over.realloc
        );
    }
    // ⚠ There USED to be an assertion here, `K <= WARM_BUDGET`, and it could not
    // fail. `K` is one past the last frame ABOVE THE STEADY MAX, and no frame of
    // the steady window is above its own max, so `K <= WARM_BUDGET` holds by
    // construction for every input — a guard that was green from its own
    // definition (found 2026-09-10 while porting the harness to the KE16 tree).
    // `K` is reported, never asserted. What guards the steady window instead is
    // what a contaminated window would actually move: the per-frame CLASS pins in
    // the gate below (a scope or chunk count off its structural value on any one
    // frame), the drift line above, and the per-scene MAX.
    say!(
        "╠══ STEADY STATE (n = {}) ══════════════════════════════════════════════",
        acq.n
    );
    say!(
        "║   acquisitions  mean={:>9.3}  min={:<6} MAX={:<6}",
        acq.mean(),
        acq.min,
        acq.max
    );
    say!(
        "║     · alloc     mean={:>9.3}  min={:<6} MAX={:<6}",
        alloc.mean(),
        alloc.min,
        alloc.max
    );
    say!(
        "║     · realloc   mean={:>9.3}  min={:<6} MAX={:<6}   <- Vec growth",
        realloc.mean(),
        realloc.min,
        realloc.max
    );
    say!(
        "║   deallocations mean={:>9.3}  min={:<6} MAX={:<6}",
        dealloc.mean(),
        dealloc.min,
        dealloc.max
    );
    say!(
        "║   bytes acquired mean={:>8.1}  min={:<8} MAX={:<8}",
        bytes.mean(),
        bytes.min,
        bytes.max
    );
    say_inline!("║   histogram (acquisitions -> frames): ");
    for (v, c) in histogram(steady) {
        say_inline!("{v}->{c}  ");
    }
    say!();
    say!(
        "║   BY CLASS (mean / min / MAX per frame):  scope {:.3}/{}/{}   chunk {:.3}/{}/{}            injector {:.3}/{}/{}   OTHER {:.3}/{}/{}",
        c_scope.mean(),
        c_scope.min,
        c_scope.max,
        c_chunk.mean(),
        c_chunk.min,
        c_chunk.max,
        c_inj.mean(),
        c_inj.min,
        c_inj.max,
        c_other.mean(),
        c_other.min,
        c_other.max
    );
    // A trend inside the window is the other way a steady window can be
    // contaminated, and `K` cannot see it: halves are compared, never asserted
    // here (the pinned MAX is the assertion).
    let half = steady.len() / 2;
    let h1 = Stat::of(&steady[..half], |s| s.acquisitions());
    let h2 = Stat::of(&steady[half..], |s| s.acquisitions());
    say!(
        "║   drift: first half mean {:.3} max {}  |  second half mean {:.3} max {}",
        h1.mean(),
        h1.max,
        h2.mean(),
        h2.max
    );
    say!("╠══ ANTI-VACUITY: counter live INSIDE this scene's window ═════════════");
    say!(
        "║   control frame: alloc={} bytes={}   |   probe frame: alloc={} bytes={} (+{} MiB)",
        control.alloc,
        control.alloc_bytes,
        live.alloc,
        live.alloc_bytes,
        PROBE_BYTES >> 20
    );
    say!("╚══════════════════════════════════════════════════════════════════════");

    Row {
        label: label.to_string(),
        k,
        mean: acq.mean(),
        min: acq.min,
        max: acq.max,
        realloc_max: realloc.max,
        bytes_mean: bytes.mean(),
        setup_alloc: setup.alloc + setup.realloc,
        dispatch_max: dispatch.max,
        realloc_sum: realloc.sum,
        scope: (c_scope.mean(), c_scope.min, c_scope.max, c_scope.sum),
        chunk: (c_chunk.mean(), c_chunk.min, c_chunk.max, c_chunk.sum),
        inj: (c_inj.mean(), c_inj.min, c_inj.max, c_inj.sum),
        other: (c_other.mean(), c_other.min, c_other.max, c_other.sum),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Shared fixtures
// ═══════════════════════════════════════════════════════════════════════════

/// Payload row. 4096 of them, so `par_iter` is above
/// `MIN_ARCHETYPE_FOR_PARALLEL` (1024) and actually fans out.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Payload {
    v: u32,
}

#[derive(Bundle)]
struct PayloadBundle {
    p: Payload,
}

/// The churn marker: carries the generation it was spawned in, so ONE system
/// can despawn the previous generation and spawn the next without needing an
/// ordering edge between two systems.
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
    despawned: u64,
}

#[derive(Resource, Default)]
struct EventTally {
    sent: u64,
    received: u64,
    changed_matches: u64,
}

/// Rows seeded into the payload archetype.
const ROWS: usize = 4096;
/// Entities spawned AND despawned every churn frame.
const CHURN_PER_FRAME: usize = 64;
/// Events sent every frame in S3.
const EVENTS_PER_FRAME: u32 = 32;

fn pool(workers: usize) -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(workers).build()
}

fn seed_payload(world: &mut EcsMaster, rows: usize) {
    world
        .spawn_batch((0..rows as u32).map(|v| PayloadBundle { p: Payload { v } }))
        .expect("seed payload rows");
}

// ── S0 probe systems: sixteen DISTINCT fn items ─────────────────────────────
//
// Distinct `fn` items, not an array of fn pointers: every element of such an
// array has the SAME type, and a schedule that keyed systems by type would
// silently collapse sixteen registrations into one — a vacuous measurement
// that reads as a low number. Each one bumps its own counter so the scene can
// PROVE that exactly `n` systems ran exactly once per frame.
static PROBE_HITS: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];
static FIXED_HITS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

macro_rules! probe_systems {
    ($($idx:literal => $name:ident),* $(,)?) => {
        $(
            fn $name(q: Query<&Payload>) {
                PROBE_HITS[$idx].fetch_add(1, Ordering::Relaxed);
                black_box(q.iter().count());
            }
        )*
    };
}
probe_systems!(
    0 => probe0, 1 => probe1, 2 => probe2, 3 => probe3,
    4 => probe4, 5 => probe5, 6 => probe6, 7 => probe7,
    8 => probe8, 9 => probe9, 10 => probe10, 11 => probe11,
    12 => probe12, 13 => probe13, 14 => probe14, 15 => probe15,
);

macro_rules! fixed_systems {
    ($($idx:literal => $name:ident),* $(,)?) => {
        $(
            fn $name(q: Query<&Payload>) {
                FIXED_HITS[$idx].fetch_add(1, Ordering::Relaxed);
                black_box(q.iter().count());
            }
        )*
    };
}
fixed_systems!(0 => fixed0, 1 => fixed1, 2 => fixed2, 3 => fixed3);

/// Registers exactly `n` of the sixteen probe systems.
macro_rules! add_probes {
    ($b:expr, $n:expr) => {{
        let n = $n;
        if n > 0 {
            $b.add_system(probe0);
        }
        if n > 1 {
            $b.add_system(probe1);
        }
        if n > 2 {
            $b.add_system(probe2);
        }
        if n > 3 {
            $b.add_system(probe3);
        }
        if n > 4 {
            $b.add_system(probe4);
        }
        if n > 5 {
            $b.add_system(probe5);
        }
        if n > 6 {
            $b.add_system(probe6);
        }
        if n > 7 {
            $b.add_system(probe7);
        }
        if n > 8 {
            $b.add_system(probe8);
        }
        if n > 9 {
            $b.add_system(probe9);
        }
        if n > 10 {
            $b.add_system(probe10);
        }
        if n > 11 {
            $b.add_system(probe11);
        }
        if n > 12 {
            $b.add_system(probe12);
        }
        if n > 13 {
            $b.add_system(probe13);
        }
        if n > 14 {
            $b.add_system(probe14);
        }
        if n > 15 {
            $b.add_system(probe15);
        }
    }};
}

// ═══════════════════════════════════════════════════════════════════════════
// Anti-vacuity 1a — the counter itself
// ═══════════════════════════════════════════════════════════════════════════

/// The layout classes are the objects they claim to be: pinned to the
/// threadpool's own published receipt, then POSITIVELY controlled by pricing the
/// two dispatch primitives directly, with no ECS in the way.
///
/// This is what stops a class from coming back green from an emptied bucket. If
/// `ScopeShared` changes size, if the chunk constants move, or if an `install`
/// frame starts allocating something new, one of the exact equalities below
/// reds here, before any scene prints a number.
fn class_predicates_match_the_threadpool_receipt() {
    assert_eq!(CHUNK0, boyko_threadpool::__layout_receipt::CHUNK0);
    assert_eq!(CHUNK_ALIGN, boyko_threadpool::__layout_receipt::CHUNK_ALIGN);

    const WORKERS: u64 = 2;
    const REPS: usize = 64;
    let p = pool(WORKERS as usize);
    // Warm the pool: the first frames on fresh threads carry one-off lazy
    // registrations (see the `OTHER` note in the gate) that are not the claim.
    // Several spawns per install so every lane gets work to steal.
    for _ in 0..400 {
        p.install(|s| {
            for _ in 0..4 * WORKERS {
                s.spawn(|| black_box(()));
            }
        });
    }

    // ⚠ The first form of this control priced ONE frame of each primitive and
    // demanded `OTHER == 0` on it, after 64 single-spawn warm-up installs. It
    // red on a release run on 2026-09-11 at `(scope, chunk, OTHER) = (1, 1, 1)`:
    // a worker's crossbeam-epoch `Local` — its first steal — landed on the
    // priced frame. The attribution binary's section G shows that is the
    // COMMON case, not a fluke: on fresh 2-worker Apps most first steals land
    // past frame 64. A single-frame exact-zero on a fresh pool is a coin toss,
    // so the control now prices `REPS` frames of each primitive, holds scope,
    // chunk and realloc EXACT on every frame, and bounds `OTHER` over both
    // windows by one first touch per pool thread — a primitive that allocated
    // on every call would show `REPS` of them, not two.
    let mut empty_other = 0u64;
    let mut one_other = 0u64;
    let mut one_inj = 0u64;
    for i in 0..REPS {
        let before = Snap::now();
        p.install(|_| ());
        let empty = Snap::now().since(before);
        assert_eq!(
            (empty.scope, empty.chunk, empty.inj, empty.realloc),
            (1, 0, 0, 0),
            "rep {i}: an EMPTY `pool.install` must be exactly one `Box<ScopeShared>` — the \
             scope class predicate (align {SCOPE_SHARED_ALIGN}, size {SCOPE_SHARED_BYTES}) no \
             longer matches it, or the install frame has started allocating a dispatch object"
        );
        empty_other += empty.other;

        let before = Snap::now();
        p.install(|s| s.spawn(|| black_box(())));
        let one = Snap::now().since(before);
        assert_eq!(
            (one.scope, one.chunk, one.realloc),
            (1, 1, 0),
            "rep {i}: an install frame with ONE spawn must be one `ScopeShared` plus one \
             {CHUNK0}-byte `ScopeBlock` chunk — the chunk class predicate no longer matches the \
             Stage 3b block"
        );
        one_other += one.other;
        one_inj += one.inj;
    }
    say!(
        "\n[class control] {REPS} empty installs: scope=1 chunk=0 on every frame, OTHER total {empty_other} \
         | {REPS} installs + 1 spawn: scope=1 chunk=1 on every frame, injector total {one_inj}, \
         OTHER total {one_other}"
    );
    assert!(
        empty_other + one_other <= WORKERS,
        "{} OTHER acquisitions over {} priced install frames on a warmed {WORKERS}-thread pool — \
         more than one first touch per thread, so an install frame is allocating something that \
         is neither a scope frame nor a chunk",
        empty_other + one_other,
        2 * REPS
    );
}

/// All four counters move. Runs FIRST (alphabetically it does not, but it is
/// asserted again from inside every scene, so ordering is irrelevant).
fn counter_is_live() {
    let before = Snap::now();
    let mut v: Vec<u8> = Vec::with_capacity(64);
    v.resize(64, 7);
    // `Vec::reserve` on a non-empty allocation routes through `Global::grow`,
    // which is `realloc` — this is what pins the realloc axis as live.
    v.reserve(1 << 16);
    black_box(&v);
    let len = v.len();
    drop(v);
    let d = Snap::now().since(before);

    assert_eq!(len, 64);
    assert!(d.alloc >= 1, "alloc counter is dead (saw {})", d.alloc);
    assert!(
        d.realloc >= 1,
        "realloc counter is dead — a Vec grew and the realloc axis did not move (saw {})",
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
        "\n[anti-vacuity] counter is live: alloc={} realloc={} dealloc={} \
         alloc_bytes={} realloc_bytes={}",
        d.alloc, d.realloc, d.dealloc, d.alloc_bytes, d.realloc_bytes
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Anti-vacuity 1b — the counter is PROCESS-GLOBAL, not dispatcher-local
// ═══════════════════════════════════════════════════════════════════════════

static WORKER_ALLOC_RAN_OFF_THREAD: AtomicBool = AtomicBool::new(false);

/// A system body that allocates on a WORKER thread is counted. This is the
/// property the tree's thread-local gates do not have, and it is why
/// the census can claim to cover the whole frame rather than the dispatcher's
/// half of it.
fn worker_thread_allocations_are_counted() {
    let main_id: ThreadId = std::thread::current().id();
    let mut app = App::with_pool(pool(2));
    seed_payload(app.world_mut(), 64);
    app.add_systems(move |q: Query<&Payload>| {
        if std::thread::current().id() != main_id {
            WORKER_ALLOC_RAN_OFF_THREAD.store(true, Ordering::Relaxed);
        }
        let mut v: Vec<u8> = Vec::with_capacity(PROBE_BYTES);
        v.resize(PROBE_BYTES, 1);
        black_box(&v);
        black_box(q.iter().count());
    });
    app.finish();
    app.update_with_delta(Duration::from_millis(16));

    let mut seen = 0u64;
    let mut seen_bytes = 0u64;
    for _ in 0..8 {
        let before = Snap::now();
        app.update_with_delta(Duration::from_millis(16));
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
        "\n[anti-vacuity] worker-thread allocations ARE counted: {seen} allocs / \
         {} MiB over 8 frames, system body confirmed off-thread",
        seen_bytes >> 20
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// S0 — the executor floor: what an App frame costs before any game logic
// ═══════════════════════════════════════════════════════════════════════════

/// Prices `App::update_with_delta` against the number of concurrent systems in
/// Main. This is the term the tree's other frame-driving gates SUBTRACT.
fn s0_executor_floor_per_system(rows: &mut Vec<Row>) {
    const WORKERS: usize = 2;
    let mut samples: Vec<Snap> = Vec::with_capacity(TOTAL_FRAMES);

    for n in [0usize, 1, 2, 4, 8, 16] {
        for h in PROBE_HITS.iter() {
            h.store(0, Ordering::Relaxed);
        }

        let setup_before = Snap::now();
        let mut app = App::with_pool(pool(WORKERS));
        seed_payload(app.world_mut(), ROWS);
        app.add_systems_cfg(|b| add_probes!(b, n));
        app.finish();
        app.update_with_delta(Duration::from_millis(16));
        let setup = Snap::now().since(setup_before);

        drive(
            || app.update_with_delta(Duration::from_millis(16)),
            &mut samples,
        );
        let (control, live) = liveness_probe("S0", &samples, || {
            app.update_with_delta(Duration::from_millis(16))
        });

        // ── Anti-vacuity: exactly `n` systems ran, exactly once per frame ──
        // (TOTAL_FRAMES driven + 1 setup frame + 2 probe frames.)
        let expected = (TOTAL_FRAMES + 3) as u64;
        for (i, h) in PROBE_HITS.iter().enumerate() {
            let hits = h.load(Ordering::Relaxed);
            if i < n {
                assert_eq!(
                    hits, expected,
                    "S0/n={n}: probe{i} ran {hits} times, expected {expected} — the \
                     schedule did not run every registered system exactly once per frame"
                );
            } else {
                assert_eq!(hits, 0, "S0/n={n}: probe{i} ran but was never registered");
            }
        }

        rows.push(report(
            &format!("S0 — executor floor: App frame, {n} concurrent system(s) in Main"),
            &format!(
                "{WORKERS} workers, {ROWS} rows, no Fixed schedule. Frame = \
                 fold + Time + check-ticks + update_events + Main Schedule::run."
            ),
            setup,
            &samples,
            control,
            live,
        ));
    }
}

/// The same floor with a Fixed schedule attached, driven at exactly one
/// substep per frame — the shape a real game has.
fn s0b_executor_floor_with_fixed_substep(rows: &mut Vec<Row>) {
    const WORKERS: usize = 2;
    /// Exactly one 64 Hz step.
    const STEP: Duration = Duration::from_nanos(15_625_000);
    let mut samples: Vec<Snap> = Vec::with_capacity(TOTAL_FRAMES);

    for h in PROBE_HITS.iter() {
        h.store(0, Ordering::Relaxed);
    }
    for h in FIXED_HITS.iter() {
        h.store(0, Ordering::Relaxed);
    }

    let setup_before = Snap::now();
    let mut app = App::with_pool(pool(WORKERS));
    seed_payload(app.world_mut(), ROWS);
    app.add_systems_cfg(|b| add_probes!(b, 4usize));
    app.add_systems_cfg_in(CoreSchedule::Fixed, |b| {
        b.add_system(fixed0);
        b.add_system(fixed1);
        b.add_system(fixed2);
        b.add_system(fixed3);
    });
    app.set_fixed_hz(64.0);
    app.finish();
    app.update_with_delta(STEP);
    let setup = Snap::now().since(setup_before);

    drive(|| app.update_with_delta(STEP), &mut samples);
    let (control, live) = liveness_probe("S0b", &samples, || app.update_with_delta(STEP));

    let expected = (TOTAL_FRAMES + 3) as u64;
    for (i, h) in PROBE_HITS.iter().enumerate().take(4) {
        assert_eq!(
            h.load(Ordering::Relaxed),
            expected,
            "S0b: Main probe{i} did not run once per frame"
        );
    }
    let fixed_hits = FIXED_HITS[0].load(Ordering::Relaxed);
    assert!(
        fixed_hits >= expected - 2 && fixed_hits <= expected + 2,
        "S0b: the Fixed schedule ran {fixed_hits} substeps over {expected} frames — the \
         scene is not the one-substep-per-frame shape it claims to be"
    );

    rows.push(report(
        "S0b — executor floor: App frame, 4 Main + 4 Fixed systems, 1 substep/frame",
        &format!("{WORKERS} workers, {ROWS} rows, 64 Hz fixed step, delta = exactly one step."),
        setup,
        &samples,
        control,
        live,
    ));
}

// ═══════════════════════════════════════════════════════════════════════════
// S1 — the rigid-body pile (Jolt-parity pyramid) through the real physics
//      Schedule. Its "frame" is one FIXED STEP: physics is registered on a
//      builder that needs `&mut EcsMaster` alongside it, which `App` does not
//      expose, so the schedule is driven directly — exactly as the shipped
//      bench and every physics acceptance suite drive it. The App funnel
//      around it is priced by S0/S0b.
// ═══════════════════════════════════════════════════════════════════════════

const BOX_SIZE: f32 = 2.0;
const BOX_SEPARATION: f32 = 0.5;
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
const DT: f32 = 1.0 / 60.0;

/// Jolt's `cPyramidHeight`. Scaled down in a debug build, where the shipped
/// scene would not terminate in a reasonable sweep.
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

/// Builds Jolt's pyramid, index for index. Returns every dynamic body's handle.
fn spawn_jolt_pyramid(world: &mut EcsMaster) -> Vec<Entity> {
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
    let mut out = Vec::with_capacity(1400);
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
                out.push(spawn_body(
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
                ));
            }
        }
    }
    out
}

/// One arm of S1.
fn run_pyramid_arm(
    rows: &mut Vec<Row>,
    label: &str,
    colored: bool,
    workers: usize,
    parallel: bool,
) {
    let mut samples: Vec<Snap> = Vec::with_capacity(TOTAL_FRAMES);

    let setup_before = Snap::now();
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
        cfg.parallel_solve = parallel;
        cfg.parallel_broadphase = parallel;
        cfg.sleeping = false;
    }
    let mut schedule = builder.build(&mut world);

    // Scene-side invariant, part 1: where the pile starts.
    let stride = bodies.len() / 8 + 1;
    let mut sample_ids: Vec<Entity> = Vec::with_capacity(16);
    let mut idx = 0usize;
    while idx < bodies.len() {
        sample_ids.push(bodies[idx]);
        idx += stride;
    }
    // `Vec<Entity>::iter()` resolves to the prelude's
    // `RelationshipSourceCollection::iter` (a trait method on the receiver beats an
    // autoderef to the slice), so it yields `Entity` BY VALUE, not `&Entity`.
    let start: Vec<Vec3> = sample_ids
        .iter()
        .map(|e| {
            world
                .get_component::<RigidBody>(e)
                .expect("body lives")
                .position
        })
        .collect();

    schedule.run(&mut world);
    let setup = Snap::now().since(setup_before);

    drive(|| schedule.run(&mut world), &mut samples);
    let (control, live) = liveness_probe(label, &samples, || schedule.run(&mut world));

    // ── The structural claim, per frame: a parallel step opens ONE install frame
    // plus one nested scope per dispatched colour per colour pass, and every pass
    // walks the same colours (the constraint graph is built once per step). So on
    // EVERY steady frame `scope - 1` is a multiple of the pass count
    // `substeps * (1 + relax_iterations)` — the relax loop is nested inside the
    // substep loop. A frame that breaks this has an allocation source the class
    // accounting does not know about.
    if parallel {
        let (substeps, relax) = {
            let cfg = world.resource::<PhysicsConfig>();
            (cfg.substeps as u64, cfg.relax_iterations as u64)
        };
        let passes = substeps * (1 + relax);
        for (i, f) in samples[WARM_BUDGET..].iter().enumerate() {
            assert!(
                f.scope > passes && (f.scope - 1) % passes == 0,
                "{label}: steady frame {i} opened {} scope frames; with {passes} colour passes \
                 (substeps {substeps} x (1 + relax {relax})) it must be 1 + {passes} x (dispatched \
                 colours >= 1)",
                f.scope
            );
        }
    }

    // ── Anti-vacuity: the scene is DOING something ──
    let contacts = world.resource::<Manifolds>().manifolds().len();
    assert!(
        contacts > 0,
        "{label}: ANTI-VACUITY FAILED — the narrowphase produced ZERO contacts, so the \
         solver had nothing to solve and every zero above is a zero over nothing"
    );
    let end: Vec<Vec3> = sample_ids
        .iter()
        .map(|e| {
            world
                .get_component::<RigidBody>(e)
                .expect("body lives")
                .position
        })
        .collect();
    let mut max_move = 0.0f32;
    for (a, b) in start.iter().zip(end.iter()) {
        let d = *b - *a;
        max_move = max_move.max((d.x * d.x + d.y * d.y + d.z * d.z).sqrt());
    }
    assert!(
        max_move > 1e-3,
        "{label}: ANTI-VACUITY FAILED — no sampled body moved (max {max_move}); the \
         integrate/solve stages did not advance the world"
    );

    rows.push(report(
        label,
        &format!(
            "{} dynamic bodies + 1 static floor, {workers} worker(s), sleeping OFF, \
             dt=1/60. Frame = ONE fixed step = one real physics `Schedule::run`. \
             Steady-state contacts = {contacts}; sampled bodies moved up to {max_move:.3} m.",
            bodies.len()
        ),
        setup,
        &samples,
        control,
        live,
    ));
}

fn s1a_rigid_pile_default_pipeline_serial(rows: &mut Vec<Row>) {
    run_pyramid_arm(
        rows,
        "S1a — rigid pile, DEFAULT pipeline (add_physics_systems, serial pool)",
        false,
        1,
        false,
    );
}

/// The colored pipeline with the parallel switches OFF. Its only purpose is to
/// be the CONTROL for `S1c`: if the two arms measure the same number, the
/// parallel dispatch never engaged and `S1c`'s figure is about something else.
fn s1b_rigid_pile_colored_serial(rows: &mut Vec<Row>) {
    run_pyramid_arm(
        rows,
        "S1b — rigid pile, COLORED solve, parallel switches OFF (control for S1c)",
        true,
        4,
        false,
    );
}

fn s1c_rigid_pile_colored_parallel(rows: &mut Vec<Row>) {
    run_pyramid_arm(
        rows,
        "S1c — rigid pile, COLORED solve + parallel_solve/parallel_broadphase ON",
        true,
        4,
        true,
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// S2 — spawn / despawn churn + par_iter, through real App frames
// ═══════════════════════════════════════════════════════════════════════════

/// The `par_iter` fan-out system: one `Box<ScopeShared>` plus one heap cell per
/// chunk, per call, by construction.
fn par_fanout(q: Query<&Payload>) {
    static SUM: AtomicU64 = AtomicU64::new(0);
    q.par_iter().for_each(|p: &Payload| {
        SUM.fetch_add(p.v as u64, Ordering::Relaxed);
    });
}

/// Spawns `CHURN_PER_FRAME` entities and despawns the previous generation, in
/// ONE system — so no cross-system ordering edge is needed and the churn is
/// stationary from the second frame onward.
fn churn(mut cmds: Commands, q: Query<&Doomed>, ents: Entities, mut state: ResMut<Churn>) {
    let generation = state.generation;
    let mut killed = 0u64;
    for (id, d) in q.iter_entities() {
        if d.wave != generation
            && let Some(e) = ents.get(id)
        {
            cmds.despawn(e);
            killed += 1;
        }
    }
    for _ in 0..CHURN_PER_FRAME {
        cmds.spawn(DoomedBundle {
            d: Doomed { wave: generation },
        });
    }
    state.despawned += killed;
    state.spawned += CHURN_PER_FRAME as u64;
    state.generation = generation.wrapping_add(1);
}

fn s2_spawn_despawn_churn_and_par_iter(rows: &mut Vec<Row>) {
    const WORKERS: usize = 4;
    let mut samples: Vec<Snap> = Vec::with_capacity(TOTAL_FRAMES);

    let setup_before = Snap::now();
    let mut app = App::with_pool(pool(WORKERS));
    seed_payload(app.world_mut(), ROWS);
    app.world_mut().insert_resource(Churn::default());
    app.add_systems(par_fanout);
    app.add_systems(churn);
    app.finish();
    // Two frames so the churn reaches its stationary shape (generation g-1
    // exists to be despawned) before the setup snapshot closes.
    app.update_with_delta(Duration::from_millis(16));
    app.update_with_delta(Duration::from_millis(16));
    let setup = Snap::now().since(setup_before);

    let entities_before = app.world().entity_count();
    let churn_before = {
        let c = app.world().resource::<Churn>();
        (c.spawned, c.despawned)
    };

    drive(
        || app.update_with_delta(Duration::from_millis(16)),
        &mut samples,
    );
    let (control, live) = liveness_probe("S2", &samples, || {
        app.update_with_delta(Duration::from_millis(16))
    });

    // ── Anti-vacuity: entities really were spawned AND despawned ──
    let c = app.world().resource::<Churn>();
    let spawned = c.spawned - churn_before.0;
    let despawned = c.despawned - churn_before.1;
    let entities_after = app.world().entity_count();
    assert_eq!(
        spawned,
        (TOTAL_FRAMES + 2) as u64 * CHURN_PER_FRAME as u64,
        "S2: the spawn side of the churn did not run every frame"
    );
    assert!(
        despawned >= spawned - 2 * CHURN_PER_FRAME as u64,
        "S2: ANTI-VACUITY FAILED — {despawned} despawned against {spawned} spawned; the \
         churn is not stationary and the population is growing"
    );
    assert!(
        entities_after.abs_diff(entities_before) <= 2 * CHURN_PER_FRAME,
        "S2: population drifted {entities_before} -> {entities_after}; not a steady state"
    );

    rows.push(report(
        "S2 — spawn/despawn churn + par_iter, through App frames",
        &format!(
            "{WORKERS} workers, {ROWS} static rows + {CHURN_PER_FRAME} entities \
             spawned AND despawned via Commands every frame, plus one par_iter fan-out \
             over the {ROWS}-row archetype. Measured: {spawned} spawns / {despawned} \
             despawns across the run; population {entities_before} -> {entities_after}."
        ),
        setup,
        &samples,
        control,
        live,
    ));
}

// ═══════════════════════════════════════════════════════════════════════════
// S3 — the query-and-event loop: the App funnel WITH `update_events` doing work
// ═══════════════════════════════════════════════════════════════════════════

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

/// A hand-written `Event` impl (the `#[event]` macro path is exercised by the
/// kernel suites; this binary only needs one lane-carrying type).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ping {
    value: u32,
}

impl Event for Ping {
    type Participants = NoParticipants;
    type Parameters = NoParameters;
    fn event_id() -> u64 {
        200
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

fn send_pings(mut w: EventWriter<Ping>, mut tally: ResMut<EventTally>) {
    for i in 0..EVENTS_PER_FRAME {
        w.send(Ping { value: i })
            .expect("send within lane capacity");
        tally.sent += 1;
    }
}

fn read_pings(mut r: EventReader<Ping>, mut tally: ResMut<EventTally>) {
    let mut n = 0u64;
    for e in r.read() {
        black_box(e.value);
        n += 1;
    }
    tally.received += n;
}

fn touch_payload(mut q: Query<Mut<Payload>>) {
    for mut p in q.iter_mut() {
        p.v = p.v.wrapping_add(1);
    }
}

fn count_changed(q: Query<&Payload, Changed<Payload>>, mut tally: ResMut<EventTally>) {
    tally.changed_matches += q.iter().count() as u64;
}

fn s3_query_and_event_loop(rows: &mut Vec<Row>) {
    const WORKERS: usize = 2;
    register_event::<Ping>(200);
    let mut samples: Vec<Snap> = Vec::with_capacity(TOTAL_FRAMES);

    let setup_before = Snap::now();
    let mut app = App::with_pool(pool(WORKERS));
    seed_payload(app.world_mut(), ROWS);
    app.world_mut()
        .preregister_event::<Ping>(
            EventConfig::default_for(WORKERS as u32 + 1).expect("event config for W+1 lanes"),
        )
        .expect("preregister Ping");
    app.world_mut().insert_resource(EventTally::default());
    app.set_event_update_policy(EventUpdatePolicy::EveryFrame);
    app.add_systems(send_pings);
    app.add_systems(read_pings);
    app.add_systems(touch_payload);
    app.add_systems(count_changed);
    app.finish();
    for _ in 0..3 {
        app.update_with_delta(Duration::from_millis(16));
    }
    let setup = Snap::now().since(setup_before);

    let before = {
        let t = app.world().resource::<EventTally>();
        (t.sent, t.received, t.changed_matches)
    };

    drive(
        || app.update_with_delta(Duration::from_millis(16)),
        &mut samples,
    );
    let (control, live) = liveness_probe("S3", &samples, || {
        app.update_with_delta(Duration::from_millis(16))
    });

    // ── Anti-vacuity: events were actually delivered, Changed actually matched ──
    let t = app.world().resource::<EventTally>();
    let sent = t.sent - before.0;
    let received = t.received - before.1;
    let changed = t.changed_matches - before.2;
    let frames = (TOTAL_FRAMES + 2) as u64;
    assert_eq!(
        sent,
        frames * EVENTS_PER_FRAME as u64,
        "S3: the writer did not run every frame"
    );
    assert!(
        received >= (frames - 2) * EVENTS_PER_FRAME as u64,
        "S3: ANTI-VACUITY FAILED — only {received} of {sent} events reached a reader; \
         the event lane is not delivering and its per-frame cost is unmeasured"
    );
    assert!(
        changed >= (frames - 1) * ROWS as u64,
        "S3: ANTI-VACUITY FAILED — Changed<Payload> matched {changed} rows over {frames} \
         frames; change detection is not seeing the mutation"
    );

    rows.push(report(
        "S3 — query + event loop through App frames (EveryFrame swap)",
        &format!(
            "{WORKERS} workers, {ROWS} rows, 4 Main systems: send {EVENTS_PER_FRAME} \
             events, read them, mutate every row through Mut<T>, count Changed<T>. \
             Measured: {sent} sent / {received} received / {changed} Changed matches."
        ),
        setup,
        &samples,
        control,
        live,
    ));
}

// ═══════════════════════════════════════════════════════════════════════════
// THE GATE — every scene's steady-state envelope, pinned at what it IS
// ═══════════════════════════════════════════════════════════════════════════

/// The profile the pins were measured in. The two profiles differ in exactly
/// two places, both measured: a debug build's colored pipeline makes ONE extra
/// `OTHER` acquisition per step (the `cfg!(debug_assertions)`-gated
/// `debug_assert_coloring` scratch in `ConstraintGraph::build`, the same one
/// `constraint_graph_o4_world.rs` tolerates), and a debug build's pyramid is
/// height 10 instead of 15, so S1c is a different scene.
const RELEASE: bool = !cfg!(debug_assertions);

/// One scene's pinned steady-state envelope over the 256-frame window.
///
/// Pinned per CLASS rather than as one total, because one total is exactly
/// the number a regression can hide under: an App frame's MAX is 3 whether it
/// is `1 scope + 1 chunk + 1 injector block` or `1 + 1 + 1 new Vec`. The
/// classes cannot trade against each other.
struct Pin {
    /// Row label prefix (unique across the census).
    scene: &'static str,
    /// Worker threads in the scene's pool — the bound on first-touch `OTHER`.
    workers: u64,
    /// Scope frames on EVERY steady frame, inclusive range.
    scope: (u64, u64),
    /// `ScopeBlock` chunks on EVERY steady frame, inclusive range.
    chunk: (u64, u64),
    /// Per-frame MAX of every non-`OTHER` acquisition (dispatch objects plus
    /// `realloc`s).
    dispatch_max: u64,
    /// `OTHER` acquisitions the scene makes on every frame BY DESIGN (0, except
    /// the debug-only coloring scratch).
    other_per_frame: u64,
    /// `realloc`s over the whole steady window.
    realloc_sum: u64,
}

impl Pin {
    /// The first-touch allowance: crossbeam-epoch's `Collector::register`,
    /// the ONLY `OTHER` site `BOYKO_ALLOC_TRACE=1` finds in these scenes (36 of
    /// 36 captures across 24 fresh Apps), fires once per pool thread on its
    /// first steal — which can land in any frame, and on a fresh 2-worker App
    /// lands in the steady window more often than not (660 of 1056, the
    /// attribution binary's section G). Two per thread rather than one, because
    /// a thread can also finish BOOTING after setup (std's 30-byte UTF-16 copy
    /// of the worker name for `SetThreadDescription`, 13 Apps in 832, captured
    /// by stack) and because one census window measured THREE on a 2-worker
    /// pool whose third object is still unnamed (see "Open" in the header).
    /// Measured worst: 3 against this allowance of 4.
    fn first_touch(&self) -> u64 {
        2 * self.workers
    }
    /// The steady-state MAX this scene may reach on any single frame.
    fn total_max(&self) -> u64 {
        self.dispatch_max + self.other_per_frame + self.first_touch()
    }
}

/// The pins. Every number is the MEASURED steady-state figure on
/// `merge/ke16-into-ecsnative` @ `ca582e72`, 2026-09-10, re-confirmed
/// 2026-09-11 (nine release and five debug runs in all,
/// `stable-x86_64-pc-windows-gnu` rustc 1.98.1); the header lists
/// the per-scene measurements they were read from. Headroom is ZERO wherever
/// the count is structural, and each non-zero headroom says why in one line.
/// S1c is the one pin read from a longer run than its own window (the
/// 4,352-step adjudication run, 2026-09-11) — and only DOWNWARD: upward it is
/// the window's own maximum.
fn pins() -> [Pin; 12] {
    // An App frame: one install frame (a `ScopeShared` + one chunk) and at most
    // one injector block — the block arrives once per 63 dispatcher-side pushes,
    // so its per-frame max is 1 and it is already inside the measured 3.
    const fn app(scene: &'static str, scope: u64, workers: u64) -> Pin {
        Pin {
            scene,
            workers,
            scope: (scope, scope),
            chunk: (scope, scope),
            dispatch_max: 2 * scope + 1,
            other_per_frame: 0,
            realloc_sum: 0,
        }
    }
    let colored_other = if RELEASE { 0 } else { 1 };
    [
        // No system, no install frame: `Schedule::run` returns at its
        // `systems.is_empty()` guard.
        Pin {
            scene: "S0 — executor floor: App frame, 0 concurrent",
            workers: 2,
            scope: (0, 0),
            chunk: (0, 0),
            dispatch_max: 0,
            other_per_frame: 0,
            realloc_sum: 0,
        },
        app("S0 — executor floor: App frame, 1 concurrent", 1, 2),
        app("S0 — executor floor: App frame, 2 concurrent", 1, 2),
        app("S0 — executor floor: App frame, 4 concurrent", 1, 2),
        app("S0 — executor floor: App frame, 8 concurrent", 1, 2),
        app("S0 — executor floor: App frame, 16 concurrent", 1, 2),
        // Two `Schedule::run`s a frame (Fixed + Main).
        app("S0b", 2, 2),
        // Install frame + the `par_iter` fan-out's nested scope: `app`'s shape
        // exactly — 2 scope + 2 chunk + at most 1 injector block = dispatch MAX
        // 5, `realloc` 0. Re-derived 2026-09-11 after EM2′ from the frame
        // arithmetic, not by narrowing a measurement — and the measurement
        // agrees in both profiles (release and debug: 4.031 / 5, `realloc` 0):
        //   * `realloc` 0 is STRUCTURAL. The only growable this scene ever had
        //     was `EntityMaster::free_entity_ids`, a `Vec` that took 64 despawns
        //     a frame and was never popped by `Commands::spawn` (pinned at 2 per
        //     window, across its 8192 and 16384 doublings). Since EM2′ the
        //     recycled ids live on a `VmColumn` (no heap at all), and each
        //     frame's 64 spawns claim the previous window's 64 despawns, so the
        //     stack holds <= 64 entries at rest — inside its first commit slab
        //     (4096 entries) for the life of the scene.
        //   * dispatch MAX 5, no headroom: the former +1 covered the free-list
        //     doubling landing in the same frame as the injector block. That
        //     event no longer exists.
        // ⚠ This pin is BLIND to a regression of the leak itself: the stack is
        // `VirtualAlloc`-backed, so a deferred route that stopped recycling
        // would grow it (and the id-slot store) in uncounted commits and stay
        // green here. `em_deferred_recycle.rs` (T-RED-1) and the attribution
        // binary's C6 count the ids instead — see "Open" in the header.
        app("S2", 2, 4),
        app("S3", 1, 2),
        app("S1a", 1, 1),
        Pin {
            other_per_frame: colored_other,
            ..app("S1b", 1, 4)
        },
        if RELEASE {
            // 1240 bodies. The census window measures 121 scope frames on every
            // frame (1 + 12 passes x 10 dispatched colours) and 205..=217
            // chunks, dispatch MAX 339; the 4,352-step adjudication run
            // (reproduced 2026-09-11) reached 109 scope frames (9 colours) and
            // 193 chunks as the pile settled, and NEVER went above 121 / 217 /
            // 339. So the envelope is the long
            // run's, with ZERO upward headroom: the one-colour allowance this
            // pin used to carry upward (133 / 241 / 375) was exactly the size of
            // one extra fan-out per colour pass, and that regression stayed
            // green inside it (see "The gate" in the header). An upward colour
            // is a red to re-measure, never a pin to widen.
            Pin {
                scene: "S1c",
                workers: 4,
                scope: (121 - 12, 121),
                chunk: (193, 217),
                dispatch_max: 339,
                other_per_frame: 0,
                realloc_sum: 0,
            }
        } else {
            // 385 bodies (height 10): 85..=97 scope frames (7 or 8 dispatched
            // colours) with exactly one chunk each in the census window,
            // dispatch MAX 195; a 4,352-step debug run (2026-09-11) reached 73
            // (6 colours) and never went above 97 / 97 / 195. The release pin's
            // shape: the long run's envelope, ZERO upward headroom.
            Pin {
                scene: "S1c",
                workers: 4,
                scope: (85 - 12, 97),
                chunk: (85 - 12, 97),
                dispatch_max: 195,
                other_per_frame: 1,
                realloc_sum: 0,
            }
        },
    ]
}

/// Evaluates every pin against its row. Collects EVERY violation before
/// failing, so one red run names all of them rather than the first.
fn gate(rows: &[Row]) {
    let pins = pins();
    let mut violations: Vec<String> = Vec::with_capacity(16);
    let mut covered = 0usize;
    say!(
        "\n══════════ GATE — pinned steady-state envelopes ({}) ══════════",
        if RELEASE { "release" } else { "debug" }
    );
    for pin in &pins {
        let matching: Vec<&Row> = rows
            .iter()
            .filter(|r| r.label.starts_with(pin.scene))
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "GATE: pin `{}` matched {} rows — every pin must name exactly one scene",
            pin.scene,
            matching.len()
        );
        let r = matching[0];
        covered += 1;
        let mut check = |ok: bool, what: String| {
            if !ok {
                violations.push(format!("{}: {what}", r.label));
            }
        };
        check(
            r.max <= pin.total_max(),
            format!("steady MAX {} > pinned {}", r.max, pin.total_max()),
        );
        check(
            r.dispatch_max <= pin.dispatch_max,
            format!(
                "dispatch MAX {} > pinned {}",
                r.dispatch_max, pin.dispatch_max
            ),
        );
        check(
            r.scope.1 >= pin.scope.0 && r.scope.2 <= pin.scope.1,
            format!(
                "scope frames {}..={} outside pinned {}..={}",
                r.scope.1, r.scope.2, pin.scope.0, pin.scope.1
            ),
        );
        check(
            r.chunk.1 >= pin.chunk.0 && r.chunk.2 <= pin.chunk.1,
            format!(
                "chunks {}..={} outside pinned {}..={}",
                r.chunk.1, r.chunk.2, pin.chunk.0, pin.chunk.1
            ),
        );
        check(
            r.inj.2 <= 1,
            format!(
                "{} injector blocks in one frame; the per-63-pushes block can arrive at most once",
                r.inj.2
            ),
        );
        let other_budget = pin.other_per_frame * STEADY_FRAMES as u64 + pin.first_touch();
        check(
            r.other.3 <= other_budget,
            format!(
                "{} OTHER acquisitions over the window > budget {other_budget}",
                r.other.3
            ),
        );
        check(
            r.realloc_sum <= pin.realloc_sum,
            format!(
                "{} reallocs over the window > pinned {}",
                r.realloc_sum, pin.realloc_sum
            ),
        );
        say!(
            "{:<66} MAX {:>4} <= {:<4} dispatch {:>4} <= {:<4} scope {:>3}..={:<3} in {:>3}..={:<3} chunk {:>3}..={:<3} in {:>3}..={:<3} OTHER {:>3} <= {:<3} realloc {} <= {}",
            r.label,
            r.max,
            pin.total_max(),
            r.dispatch_max,
            pin.dispatch_max,
            r.scope.1,
            r.scope.2,
            pin.scope.0,
            pin.scope.1,
            r.chunk.1,
            r.chunk.2,
            pin.chunk.0,
            pin.chunk.1,
            r.other.3,
            other_budget,
            r.realloc_sum,
            pin.realloc_sum
        );
    }
    assert_eq!(
        covered,
        rows.len(),
        "GATE: {} scenes ran and {covered} were pinned — an unpinned scene is an ungated one",
        rows.len()
    );
    // Positive controls on the classes themselves (see `C_SCOPE`'s note): if a
    // class predicate stopped matching its object, the class would read ZERO
    // and every "<=" above would be green from an emptied bucket.
    let s16 = rows
        .iter()
        .find(|r| {
            r.label
                .starts_with("S0 — executor floor: App frame, 16 concurrent")
        })
        .expect("the 16-system row");
    if s16.inj.3 == 0 {
        violations.push(format!(
            "POSITIVE CONTROL: the 16-system scene pushed {} tasks over its window and the \
             injector class saw NO block — its predicate ({INJECTOR_BLOCK_BYTES} B, align 8) no \
             longer matches crossbeam's block",
            16 * STEADY_FRAMES
        ));
    }
    if violations.is_empty() {
        say!("GATE: GREEN — {covered} scenes, every class inside its pin");
    } else {
        for v in &violations {
            say!("GATE VIOLATION: {v}");
        }
        panic!(
            "GATE: {} violation(s) of the pinned steady-state allocation envelope — see the \
             GATE lines above. If the change is intended, re-measure and re-pin WITH the new \
             numbers in the header; do not widen a pin to make a red go away.",
            violations.len()
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// The single entry point
// ═══════════════════════════════════════════════════════════════════════════

/// The whole census, every scene in sequence, on the thread that calls it.
///
/// ONE entry point, not a stylistic choice: this binary's counter is
/// process-global, so two scenes running at once would each measure the other's
/// allocations and report a number that is not wrong by a little. It used to be
/// the single `#[test]` of a libtest binary for exactly that reason; since the
/// target became `harness = false` (see [`main`]) nothing else runs in the
/// process at all. The two anti-vacuity probes run FIRST, so a dead counter
/// fails the run before any scene prints a number.
fn frame_allocation_census() {
    counter_is_live();
    class_predicates_match_the_threadpool_receipt();
    worker_thread_allocations_are_counted();

    let mut rows: Vec<Row> = Vec::with_capacity(16);
    s0_executor_floor_per_system(&mut rows);
    s0b_executor_floor_with_fixed_substep(&mut rows);
    s2_spawn_despawn_churn_and_par_iter(&mut rows);
    s3_query_and_event_loop(&mut rows);
    s1a_rigid_pile_default_pipeline_serial(&mut rows);
    s1b_rigid_pile_colored_serial(&mut rows);
    s1c_rigid_pile_colored_parallel(&mut rows);

    // ── Anti-vacuity across arms: the parallel dispatch really did engage ──
    //
    // `PhysicsConfig::parallel_solve = true` is a REQUEST, not an event: the
    // colored solver still takes the inline path unless the widest color clears
    // its own slot floor, and `parallel_broadphase` is inert below its body
    // floor. Two arms differing only in those two flags is the only statement
    // that distinguishes "parallel dispatch is what costs this" from "the flag
    // was set". If they measure the same, S1c's number is about the colored
    // pipeline and NOT about parallelism, and must not be quoted as the latter.
    let serial_arm = rows
        .iter()
        .find(|r| r.label.starts_with("S1b"))
        .expect("S1b row");
    let parallel_arm = rows
        .iter()
        .find(|r| r.label.starts_with("S1c"))
        .expect("S1c row");
    let engaged = parallel_arm.mean > serial_arm.mean;
    say!(
        "
[anti-vacuity] parallel dispatch engaged: {} — colored-serial {:.3} vs colored-parallel {:.3} acquisitions/step ({:.1}x)",
        if engaged { "YES" } else { "NO" },
        serial_arm.mean,
        parallel_arm.mean,
        parallel_arm.mean / serial_arm.mean.max(1.0),
    );
    assert!(
        engaged,
        "ANTI-VACUITY FAILED — the colored-parallel arm ({:.3}) did not acquire more than the colored-serial arm ({:.3}) on the same {}-body scene, so the parallel dispatch never engaged and neither arm measures it",
        parallel_arm.mean,
        serial_arm.mean,
        pyramid_height(),
    );

    say!(
        "

══════════ SUMMARY — heap acquisitions per steady-state frame ══════════"
    );
    say!(
        "{:<66} {:>4} {:>10} {:>7} {:>7} {:>8} {:>12} {:>8}",
        "scene", "K", "mean", "min", "MAX", "realloc", "bytes/frame", "setup"
    );
    for r in &rows {
        say!(
            "{:<66} {:>4} {:>10.3} {:>7} {:>7} {:>8} {:>12.0} {:>8}",
            r.label, r.k, r.mean, r.min, r.max, r.realloc_max, r.bytes_mean, r.setup_alloc
        );
    }
    say!("═══════════════════════════════════════════════════════════════════════");
    say!(
        "
══════════ BY CLASS — steady-state mean / MAX per frame ══════════"
    );
    say!(
        "{:<66} {:>14} {:>14} {:>14} {:>14}",
        "scene", "scope", "chunk", "injector", "OTHER"
    );
    for r in &rows {
        say!(
            "{:<66} {:>8.3} / {:<3} {:>8.3} / {:<3} {:>8.3} / {:<3} {:>8.3} / {:<3}",
            r.label,
            r.scope.0,
            r.scope.2,
            r.chunk.0,
            r.chunk.2,
            r.inj.0,
            r.inj.2,
            r.other.0,
            r.other.2
        );
    }
    say!("═══════════════════════════════════════════════════════════════════════");
    say!(
        "Counts only. No wall-clock figure is produced anywhere in this binary, and none may be derived from it."
    );

    gate(&rows);
}

// ═══════════════════════════════════════════════════════════════════════════
// The runner — `harness = false`, with libtest's command-line contract
// ═══════════════════════════════════════════════════════════════════════════

/// The name libtest listed and filtered this census under while it was a
/// `#[test]`; kept so every existing filter and log search still finds it.
const TEST_NAME: &str = "frame_allocation_census";

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

/// Runs the census ONLY, on the main thread, with nothing else in the process.
///
/// The point of `harness = false` (see "How to run" in the header): the
/// counter is process-global, and libtest kept a second thread alive beside the
/// test that allocates on its own schedule — the "has been running for over 60
/// seconds" notice among it. A runner of our own has no such thread. Everything
/// `main` allocates (the command line, the report reservation, the panic hook)
/// is allocated before [`frame_allocation_census`] opens its first window, and
/// the summary lines after it has closed its last.
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
        // the line a reader checks, and it must not read as a pass of the census.
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
    let passed = std::panic::catch_unwind(frame_allocation_census).is_ok();
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
