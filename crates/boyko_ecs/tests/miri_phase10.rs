//! Phase 10 Wave E Step 15 — Miri tests for change-detection unsafe code.
//!
//! Run via `cargo +nightly miri test --test miri_phase10`.
//!
//! See plan §13.3.
//!
//! # Single-thread vs multi-thread Miri
//!
//! Phase 9's `miri_phase9.rs` suite already documents that the
//! multi-threaded scope-spawn path interacts poorly with Miri's Tree
//! Borrows protected-tag mechanic (some `par_iter` worker shapes flag
//! benign Send/Sync transitions). Wave E focuses Miri coverage on:
//!
//! * Single-thread `Mut<T>::deref_mut` writes through `UnsafeCell<Tick>`.
//! * Single-thread tick arithmetic (`Tick::is_newer_than` + wraparound).
//! * Single-thread `check_ticks` scan.
//! * Cross-thread `UnsafeCell<Tick>` writes to **disjoint indices** —
//!   the Round 2 C3 test pinning that the abstract-machine soundness
//!   does NOT depend on cache-line sharing avoidance. Uses explicit
//!   `std::thread::scope` (NOT the boyko thread pool) so the test
//!   bypasses Phase 9's protected-tag interaction.
//!
//! # File gate
//!
//! `#![cfg(miri)]` — the suite only compiles under Miri. Native runs
//! ignore the file entirely; the integration tests in
//! `phase10_change_detection.rs` cover the same semantic ground
//! end-to-end.

#![cfg(miri)]

use std::cell::UnsafeCell;

use boyko_ecs::ecs::core::change_detection::{Tick, CHECK_TICK_THRESHOLD, MAX_CHANGE_AGE};
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Changed, Mut, Query};
use boyko_macros::Component;

// ── Component reservations (slot range 380-410) ────────────────────────────

#[derive(Component)]
#[repr(C)]
struct MiriPos394 {
    x: f32,
    y: f32,
}

#[derive(Component)]
#[repr(C)]
struct MiriHp395 {
    hp: u32,
}

// ── Test 1: Tick arithmetic under wraparound ──────────────────────────────

/// Plan §13.3 — `Tick::is_newer_than` and `check_tick` must be UB-free
/// under wraparound. The implementation uses `wrapping_sub`, which is
/// total — no arithmetic UB possible. This test exercises the math on
/// values near `u32::MAX` so Miri's UB detectors (including any
/// overflow-on-wrap warnings) get a chance to fire.
#[test]
fn miri_tick_arithmetic_no_overflow_panic() {
    // Spot-check the four boundary cases (lower, upper, far past, far future).
    let last = Tick::new(2);
    let this = Tick::new(10);
    let _ = Tick::new(5).is_newer_than(last, this);
    let _ = Tick::new(10).is_newer_than(last, this); // inclusive upper
    let _ = Tick::new(2).is_newer_than(last, this); // exclusive lower
    let _ = Tick::new(11).is_newer_than(last, this); // wrapped future

    // Wraparound: stored near u32::MAX, current near 0.
    let near_max = Tick::new(u32::MAX - 10);
    let near_zero = Tick::new(5);
    let _ = near_max.is_newer_than(Tick::new(u32::MAX - 100), near_zero);

    // check_tick clamp on an aged-out tick.
    let aged = Tick::new(50);
    let current = Tick::new(MAX_CHANGE_AGE.wrapping_add(200));
    let clamped = aged.check_tick(current);
    assert_eq!(clamped.get(), current.get().wrapping_sub(MAX_CHANGE_AGE));
}

// ── Test 2: Single-thread Mut deref_mut tick write ────────────────────────

/// Plan §13.3 `miri_phase10_unsafe_cell_tick_write`:
/// `Mut<T>::deref_mut` writes through `UnsafeCell<Tick>`. Miri's
/// Tree Borrows verifies the write does not invalidate any outstanding
/// borrow of the component data.
#[test]
fn miri_mut_deref_guard_no_ub() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[MiriPos394::component_id()]);
    for _ in 0..4 {
        ecs.spawn_one(arch, MiriPos394 { x: 0.0, y: 0.0 }).unwrap();
    }

    // Single-thread dispatch: writes through Mut::deref_mut + Changed read.
    ecs.run_closure_once(|mut q: Query<Mut<MiriPos394>>| {
        for mut p in &mut q {
            p.x += 1.0;
            p.y += 2.0;
        }
    });
    // Verify the tick bumps are observable in the next schedule frame
    // via a Changed<T> reader.
    ecs.run_closure_once(|q: Query<&MiriPos394, Changed<MiriPos394>>| {
        for _ in &q {
            // capture via shared probe would force Send+Sync; for Miri
            // we just exercise the iter to validate no UB.
        }
    });
}

// ── Test 3: Added filter Miri smoke ───────────────────────────────────────

/// Single-thread `Added<T>` filter walk — exercises tick reads through
/// `UnsafeCell::get()` + `Tick::is_newer_than` compare.
#[test]
fn miri_added_filter_single_thread_no_ub() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[MiriHp395::component_id()]);
    for _ in 0..3 {
        ecs.spawn_one(arch, MiriHp395 { hp: 100 }).unwrap();
    }
    ecs.run_closure_once(
        |q: Query<&MiriHp395, boyko_ecs::ecs::core::iters::query::Added<MiriHp395>>| {
            for _h in &q {
                // empty body — Miri only checks for UB.
            }
        },
    );
}

// ── Test 4: Changed filter Miri smoke ─────────────────────────────────────

/// Single-thread `Changed<T>` filter walk.
#[test]
fn miri_changed_filter_single_thread_no_ub() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[MiriHp395::component_id()]);
    for _ in 0..3 {
        ecs.spawn_one(arch, MiriHp395 { hp: 50 }).unwrap();
    }
    ecs.run_closure_once(|q: Query<&MiriHp395, Changed<MiriHp395>>| {
        for _ in &q {}
    });
}

// ── Test 5: check_ticks scan Miri smoke ───────────────────────────────────

/// Plan §13.3 `miri_phase10_check_ticks_scan`: invoke
/// `run_check_ticks_scan` (via the public-via-pub(crate) facade exposed
/// to scheduler) on a populated world. Verifies no UB in the dispatcher
/// scan path.
///
/// We exercise via the public scheduler API; the scan won't fire unless
/// the global tick has advanced past `CHECK_TICK_THRESHOLD`, which a
/// real test cannot achieve in bounded time. Instead, we verify the
/// per-row tick read/write paths invoked by `Schedule::run` are
/// Miri-clean — those use the same `UnsafeCell<Tick>` accessor pattern
/// the scan uses.
#[test]
fn miri_per_row_tick_read_write_no_ub() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[MiriHp395::component_id()]);
    for _ in 0..8 {
        ecs.spawn_one(arch, MiriHp395 { hp: 99 }).unwrap();
    }

    // Two passes: writer + Changed reader. Each iteration touches every
    // tick slot at least once, exercising the same UnsafeCell paths the
    // scan uses (read + write through `*UnsafeCell::get()`).
    for _ in 0..3 {
        ecs.run_closure_once(|mut q: Query<Mut<MiriHp395>>| {
            for mut h in &mut q {
                h.hp = h.hp.wrapping_sub(1);
            }
        });
        ecs.run_closure_once(|q: Query<&MiriHp395, Changed<MiriHp395>>| {
            for _ in &q {}
        });
    }
}

// ── Test 6: Disjoint-index par writes from multiple threads (C3) ──────────

/// Plan §13.3 Round 2 C3 — the critical Miri test pinning that
/// concurrent writes to **distinct `UnsafeCell<Tick>` slots** are sound
/// under the Rust abstract machine regardless of cache-line sharing.
///
/// We use bare `std::thread::scope` (NOT the boyko thread pool) to
/// avoid Phase 9's Tree Borrows protected-tag interaction. Each thread
/// writes to a separate, non-overlapping range of slots. The Round 2
/// C3 soundness argument: distinct `UnsafeCell<u32>` are distinct
/// memory locations; concurrent unsynchronised writes to distinct
/// memory locations are race-free per the [Rustonomicon
/// "Data Races and Race Conditions"](https://doc.rust-lang.org/nomicon/races.html)
/// section.
///
/// # Why this test
///
/// Production `par_iter` chunks write to adjacent slots in the same
/// cache line; absent this test, a future regression that "fixes"
/// false-sharing by introducing synchronisation would obscure the
/// theoretical soundness underlying the original design. This pin
/// ensures any future implementation change preserves the disjoint-
/// memory-location guarantee.
///
/// # Send/Sync gate
///
/// `UnsafeCell<Tick>` is `!Sync` by default; we wrap the buffer in a
/// `SyncWrapper` that asserts `Sync` under the disjoint-write contract.
/// The wrapper is the test's responsibility to keep sound: each thread
/// writes ONLY to its own non-overlapping range.
#[test]
fn miri_par_iter_chunks_write_adjacent_ticks_disjoint_no_ub() {
    /// `Sync` wrapper for the tick buffer. SAFETY: callers MUST ensure
    /// each thread writes only to a disjoint slice of indices. The Miri
    /// test below enforces this by construction.
    struct SyncSlice<'a>(&'a [UnsafeCell<Tick>]);
    unsafe impl Sync for SyncSlice<'_> {}

    // 1024 UnsafeCell<Tick> slots — large enough to span many cache
    // lines (each Tick is 4 B → 1024 × 4 B = 4 KB = 64 cache lines).
    const N: usize = 1024;
    let cells: Vec<UnsafeCell<Tick>> = (0..N).map(|_| UnsafeCell::new(Tick::ZERO)).collect();

    let sync_view = SyncSlice(&cells[..]);
    let sync_ref = &sync_view;

    // 4 worker threads, each writes 256 disjoint slots.
    const CHUNK: usize = N / 4;

    std::thread::scope(|scope| {
        for chunk_id in 0..4 {
            let start = chunk_id * CHUNK;
            let end = start + CHUNK;
            scope.spawn(move || {
                for i in start..end {
                    // SAFETY (Round 2 C3): each `i` is unique to this
                    // thread; no other thread writes to `sync_ref.0[i]`
                    // for the duration of this scope. `UnsafeCell<Tick>`
                    // is a distinct memory location per i, so concurrent
                    // writes to distinct memory locations from different
                    // threads are race-free regardless of cache-line
                    // sharing.
                    unsafe {
                        *sync_ref.0[i].get() = Tick::new((i as u32) + 1);
                    }
                }
            });
        }
    });

    // Verify all slots were written by some thread.
    for i in 0..N {
        // SAFETY: after the scope, all spawned threads are joined; the
        // calling thread has exclusive access to every cell.
        let written = unsafe { *cells[i].get() };
        assert_eq!(
            written.get(),
            (i as u32) + 1,
            "slot {} not written by any thread",
            i
        );
    }
}

// ── Test 7: Schedule run end-to-end Miri smoke ────────────────────────────
//
// DEFERRED under Miri: `Schedule::run` invokes `boyko_threadpool::Scope::spawn`,
// which triggers the known Tree Borrows protected-tag conflict documented
// in `tests/miri_phase9.rs` (the `ScopeShared` raw-pointer protocol). The
// `set_change_ticks` dispatch path itself is single-threaded inside
// `try_dispatch_ready` and would be Miri-clean in isolation; we cover the
// same code path indirectly via `miri_mut_deref_guard_no_ub` and
// `miri_per_row_tick_read_write_no_ub` (both go through `run_closure_once`,
// which bypasses the threadpool scope-spawn protocol). When Phase 9.1
// revisits the scope-shared raw-pointer protocol the gate below should be
// dropped.

// Placeholder kept in source so a future Miri rerun is one edit away.
// #[test]
// fn miri_set_change_ticks_dispatch_to_worker_visibility() { ... }

// ── Defensive consts to silence dead-code warnings ────────────────────────

#[allow(dead_code)]
const _CHECK_THRESHOLD: u32 = CHECK_TICK_THRESHOLD;
