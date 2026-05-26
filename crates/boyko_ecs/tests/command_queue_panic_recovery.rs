//! Phase 8d Step 8 — `CommandQueue::apply` panic-recovery acceptance tests.
//!
//! Locks down the Bevy-mirror semantics that ship in Step 6's
//! [`CommandQueue::apply`] + [`RawCommandQueue::apply_or_drop_queued`] +
//! [`consume_and_drop_glue`]. Each test isolates one invariant:
//!
//! 1. **`command_queue_panic_skips_panicker_runs_rest_on_redrive`** — W3'
//!    cursor-before-apply. A panicking command is excluded from the
//!    recovery range; survivors run on the next `apply` call; the panicker
//!    is NEVER re-applied.
//!
//! 2. **`command_queue_no_step_0_5_prepend`** — C2' Step 0.5 deletion.
//!    `panic_recovery` is OPAQUE between apply calls. The success-path
//!    drain MUST NOT touch recovery; only the catch_unwind Err branch
//!    interacts with it.
//!
//! 3. **`command_queue_drop_runs_pending_commands_without_world`** — the
//!    `Drop` glue runs each pending command's `Drop` exactly once when the
//!    queue is dropped without `apply`. `Command::apply` does NOT run on
//!    this path (the world is unavailable).
//!
//! All tests use atomic counters for tracking; the `__test_*` helpers on
//! [`CommandQueue`] (added in Step 8 alongside this file) expose the
//! otherwise crate-private `new` / `push` / `apply` and observers for
//! `bytes` / `panic_recovery` length. Component-slot range 580..=590 per
//! the Step 8 spec to avoid collisions with prior phases.

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::commands::command_queue::CommandQueue;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;

// ── Test serialization ───────────────────────────────────────────────────────
//
// The per-command tracking counters below are `static AtomicUsize` —
// PROCESS-GLOBAL. Rust's default test harness runs tests in parallel
// threads, so two tests touching the same `A_APPLY` counter race with
// each other (one's `reset_all` wipes the other's increment).
//
// The fix: every test starts by acquiring `TEST_MUTEX`, which serializes
// the test bodies even though they live in the same binary. `acquire()`
// is panic-tolerant — a prior test's panic poisons the mutex, but we
// unwrap-or-recover so subsequent tests still run.
static TEST_MUTEX: Mutex<()> = Mutex::new(());

fn acquire_test_lock() -> MutexGuard<'static, ()> {
    match TEST_MUTEX.lock() {
        Ok(g) => g,
        // Test 1 panics inside its body (Panicker::apply). When that
        // unwinds, the mutex becomes poisoned. We recover the guard so
        // Tests 2 and 3 still run — the prior test's state is reset by
        // `reset_all()` regardless.
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ── Per-command tracking counters ────────────────────────────────────────────
//
// Each `Counter*` static is the side-effect target for one of the test
// command types. The tests reset all counters before exercising the queue;
// the counters' final values are the test assertions.
//
// `Ordering::Relaxed` is sufficient — each `#[test]` runs as its own
// process-internal flow; the global counters are reset by the test's own
// preamble and read by the test's own assertion block. No cross-thread
// happens-before is needed.

static A_APPLY: AtomicUsize = AtomicUsize::new(0);
static A_DROP: AtomicUsize = AtomicUsize::new(0);

static B_APPLY: AtomicUsize = AtomicUsize::new(0);
static B_DROP: AtomicUsize = AtomicUsize::new(0);

static C_APPLY: AtomicUsize = AtomicUsize::new(0);
static C_DROP: AtomicUsize = AtomicUsize::new(0);

static PANICKER_APPLY_ENTERED: AtomicUsize = AtomicUsize::new(0);
static PANICKER_DROP: AtomicUsize = AtomicUsize::new(0);

static X_APPLY: AtomicUsize = AtomicUsize::new(0);
static X_DROP: AtomicUsize = AtomicUsize::new(0);

/// Resets every per-command counter to 0 before a test exercises the queue.
fn reset_all() {
    A_APPLY.store(0, Ordering::Relaxed);
    A_DROP.store(0, Ordering::Relaxed);
    B_APPLY.store(0, Ordering::Relaxed);
    B_DROP.store(0, Ordering::Relaxed);
    C_APPLY.store(0, Ordering::Relaxed);
    C_DROP.store(0, Ordering::Relaxed);
    PANICKER_APPLY_ENTERED.store(0, Ordering::Relaxed);
    PANICKER_DROP.store(0, Ordering::Relaxed);
    X_APPLY.store(0, Ordering::Relaxed);
    X_DROP.store(0, Ordering::Relaxed);
}

// ── No-op tracker commands ───────────────────────────────────────────────────
//
// Each `Tracker*` is a tiny `Command` that bumps its apply counter inside
// `apply` and its drop counter inside `Drop`. The two counters are observed
// separately so the tests can distinguish the apply path from the drop
// path (Test 3's headline assertion).

struct TrackerA;
struct TrackerB;
struct TrackerC;

impl Command for TrackerA {
    fn apply(self, _world: &mut EcsMaster) {
        A_APPLY.fetch_add(1, Ordering::Relaxed);
    }
}
impl Drop for TrackerA {
    fn drop(&mut self) {
        A_DROP.fetch_add(1, Ordering::Relaxed);
    }
}

impl Command for TrackerB {
    fn apply(self, _world: &mut EcsMaster) {
        B_APPLY.fetch_add(1, Ordering::Relaxed);
    }
}
impl Drop for TrackerB {
    fn drop(&mut self) {
        B_DROP.fetch_add(1, Ordering::Relaxed);
    }
}

impl Command for TrackerC {
    fn apply(self, _world: &mut EcsMaster) {
        C_APPLY.fetch_add(1, Ordering::Relaxed);
    }
}
impl Drop for TrackerC {
    fn drop(&mut self) {
        C_DROP.fetch_add(1, Ordering::Relaxed);
    }
}

/// A command whose `apply` deterministically panics. The "entered" counter
/// proves that `apply` was actually invoked (rather than skipped silently).
struct Panicker;

impl Command for Panicker {
    fn apply(self, _world: &mut EcsMaster) {
        PANICKER_APPLY_ENTERED.fetch_add(1, Ordering::Relaxed);
        panic!("Phase 8d Step 8 deliberate panic — Panicker::apply");
    }
}
impl Drop for Panicker {
    fn drop(&mut self) {
        PANICKER_DROP.fetch_add(1, Ordering::Relaxed);
    }
}

/// A command used only by the C2' regression test (Test 2). Kept distinct
/// from the `Tracker*` family so a stray invocation is unambiguous.
struct TrackerX;

impl Command for TrackerX {
    fn apply(self, _world: &mut EcsMaster) {
        X_APPLY.fetch_add(1, Ordering::Relaxed);
    }
}
impl Drop for TrackerX {
    fn drop(&mut self) {
        X_DROP.fetch_add(1, Ordering::Relaxed);
    }
}

// ── Test 1 — W3' headline: panicker skipped, survivors run on redrive ───────

/// Push `[A, PANICKER, B, C]`. The first `apply` runs A, then PANICKER
/// panics (caught by the queue's internal `catch_unwind` and re-raised);
/// the survivors `[B, C]` are captured into `panic_recovery` and (since
/// `start == 0`) re-absorbed into `bytes` in the same call. The second
/// `apply` walks `[B, C]` cleanly.
///
/// Assertions:
///
/// * `A_APPLY == 1` after the first `apply`.
/// * `PANICKER_APPLY_ENTERED == 1` — the panic actually fired.
/// * `B_APPLY == 0` and `C_APPLY == 0` after the first apply (survivors
///   deferred to the next call).
/// * `recovery_len == 0` after the first apply — Bevy-mirror C2' absorbs
///   recovery into `bytes` in the panicking call.
/// * `bytes_len > 0` after the first apply — survivors are queued for the
///   next walk.
/// * After the second `apply`: `B_APPLY == 1`, `C_APPLY == 1`,
///   `PANICKER_APPLY_ENTERED` still `== 1` (panicker NEVER re-runs —
///   W3' SKIP semantic).
#[test]
fn command_queue_panic_skips_panicker_runs_rest_on_redrive() {
    let _serial = acquire_test_lock();
    reset_all();

    let mut q = CommandQueue::__test_new();
    let mut world = EcsMaster::new();

    q.__test_push(TrackerA);
    q.__test_push(Panicker);
    q.__test_push(TrackerB);
    q.__test_push(TrackerC);

    // First apply — must propagate the panic. The queue catches inside
    // `apply` to perform recovery bookkeeping, then `resume_unwind`s.
    // `AssertUnwindSafe` is required because `&mut CommandQueue` /
    // `&mut EcsMaster` are not auto-`UnwindSafe`.
    let first = panic::catch_unwind(AssertUnwindSafe(|| {
        q.__test_apply(&mut world);
    }));
    assert!(first.is_err(), "first apply must propagate Panicker's panic");

    assert_eq!(
        A_APPLY.load(Ordering::Relaxed),
        1,
        "A ran before PANICKER",
    );
    assert_eq!(
        PANICKER_APPLY_ENTERED.load(Ordering::Relaxed),
        1,
        "PANICKER::apply was actually entered",
    );
    assert_eq!(
        B_APPLY.load(Ordering::Relaxed),
        0,
        "B did NOT run during the first apply (survivor)",
    );
    assert_eq!(
        C_APPLY.load(Ordering::Relaxed),
        0,
        "C did NOT run during the first apply (survivor)",
    );

    // After the panic, recovery has been re-absorbed into `bytes` (Bevy
    // mirror at start == 0); the next apply walks the survivors directly.
    assert_eq!(
        q.__test_recovery_len(),
        0,
        "panic_recovery re-absorbed into bytes at start==0 (Bevy mirror)",
    );
    assert!(
        q.__test_bytes_len() > 0,
        "bytes must hold the survivor tail [B, C] for the next apply",
    );

    // Second apply — survivors run cleanly. The panicker is NOT in the
    // recovery range (W3' cursor was advanced past it in
    // `consume_and_drop_glue` BEFORE the panic), so its counter stays at 1.
    q.__test_apply(&mut world);

    assert_eq!(
        B_APPLY.load(Ordering::Relaxed),
        1,
        "B ran on the redrive",
    );
    assert_eq!(
        C_APPLY.load(Ordering::Relaxed),
        1,
        "C ran on the redrive",
    );
    assert_eq!(
        PANICKER_APPLY_ENTERED.load(Ordering::Relaxed),
        1,
        "PANICKER::apply was NEVER re-invoked (W3' SKIP semantic)",
    );
    assert_eq!(
        q.__test_bytes_len(),
        0,
        "bytes drained after the redrive apply",
    );
    assert_eq!(
        q.__test_recovery_len(),
        0,
        "panic_recovery still empty after the clean redrive",
    );
}

// ── Test 2 — C2' regression: clean apply does NOT touch recovery ────────────

/// Inject TrackerX bytes directly into `panic_recovery` (the path the
/// Round 2 "Step 0.5 prepend" would have read). Push TrackerA into `bytes`.
/// Call `apply` — only A runs; X stays opaque in recovery.
///
/// This locks down the C2' deletion: a future refactor cannot accidentally
/// re-introduce a Step 0.5 prepend without this test exploding.
///
/// Assertions:
///
/// * After `apply`: `A_APPLY == 1` (A ran).
/// * `X_APPLY == 0` (X was in recovery, not in bytes; the success-path
///   drain DOES NOT touch recovery).
/// * `bytes_len == 0` (the success-path drain trims `bytes` to `start`).
/// * `recovery_len > 0` (recovery is still opaque — X's bytes are still
///   sitting there, untouched by the clean apply).
///
/// Note on cleanup: `TrackerX`'s Drop must still run exactly once — when
/// the queue is dropped at end-of-scope, the recovery-drain path in
/// [`CommandQueue::Drop`] walks `panic_recovery` and invokes each
/// command's `Drop` via the `world=None` glue path. The test asserts
/// `X_DROP == 1` AFTER the queue is dropped.
#[test]
fn command_queue_no_step_0_5_prepend() {
    let _serial = acquire_test_lock();
    reset_all();

    let mut q = CommandQueue::__test_new();
    let mut world = EcsMaster::new();

    // Pre-stage recovery with X. If Step 0.5 prepend were still alive, X
    // would dispatch on the next apply.
    q.__test_inject_recovery(TrackerX);
    assert!(
        q.__test_recovery_len() > 0,
        "test precondition: recovery is populated before apply",
    );

    // Now queue A in `bytes`. The apply walk reads from `bytes` only.
    q.__test_push(TrackerA);

    q.__test_apply(&mut world);

    assert_eq!(
        A_APPLY.load(Ordering::Relaxed),
        1,
        "A ran (it was in bytes)",
    );
    assert_eq!(
        X_APPLY.load(Ordering::Relaxed),
        0,
        "X did NOT run — recovery is OPAQUE between applies (C2' lock-down: \
         no Step 0.5 prepend)",
    );
    assert_eq!(
        q.__test_bytes_len(),
        0,
        "bytes drained after the clean apply",
    );
    assert!(
        q.__test_recovery_len() > 0,
        "recovery still holds X's bytes — apply never touched recovery on \
         the Ok path",
    );

    // X's Drop must still run via the queue's Drop glue (recovery branch).
    // The block scope below makes the timing explicit.
    drop(q);
    assert_eq!(
        X_DROP.load(Ordering::Relaxed),
        1,
        "X's Drop ran exactly once via CommandQueue::Drop's recovery walk",
    );
    assert_eq!(
        X_APPLY.load(Ordering::Relaxed),
        0,
        "X::apply was NEVER invoked — drop-only path uses world=None glue",
    );

    let _ = world;
}

// ── Test 3 — Drop glue runs pending commands without world ──────────────────

/// Push two trackers and let the queue go out of scope without calling
/// `apply`. The `Drop` impl on `CommandQueue` walks the bytes with
/// `world = None`, which dispatches `consume_and_drop_glue` on the
/// drop-only path (the per-type `Drop` runs in place; `Command::apply` is
/// NOT invoked).
///
/// Assertions:
///
/// * `A_DROP == 1`, `B_DROP == 1` (each command's `Drop` ran exactly once).
/// * `A_APPLY == 0`, `B_APPLY == 0` (the drop-only path bypasses `apply`).
#[test]
fn command_queue_drop_runs_pending_commands_without_world() {
    let _serial = acquire_test_lock();
    reset_all();

    {
        let mut q = CommandQueue::__test_new();
        q.__test_push(TrackerA);
        q.__test_push(TrackerB);
        // No apply — let the queue's `Drop` handle it.
    }

    assert_eq!(
        A_DROP.load(Ordering::Relaxed),
        1,
        "A's Drop ran exactly once via the queue's Drop glue",
    );
    assert_eq!(
        B_DROP.load(Ordering::Relaxed),
        1,
        "B's Drop ran exactly once via the queue's Drop glue",
    );
    assert_eq!(
        A_APPLY.load(Ordering::Relaxed),
        0,
        "A::apply did NOT run on the drop-only path",
    );
    assert_eq!(
        B_APPLY.load(Ordering::Relaxed),
        0,
        "B::apply did NOT run on the drop-only path",
    );
}
