//! Phase 9 §13.2 Send/Sync negative tests.
//!
//! Compile-time + smoke assertions that pin the Phase 9 multi-threaded
//! invariants:
//!
//! * Types that MUST be `Send + Sync` (worker thread crossings):
//!   - `EcsMaster` — workers receive `UnsafeEcsCell<'w>` copies derived
//!     from the dispatcher's `&mut EcsMaster`; the master itself crosses
//!     the dispatcher / worker boundary via the cell. (SEND1, plan §9.1.)
//!   - `UnsafeEcsCell<'w>` — `Copy` value handed to every worker spawn
//!     closure (SEND3, plan §2.4).
//!
//! * Types that MUST be `Send` but NOT `Sync`:
//!   - `CommandQueue` — Send because the byte arena holds `Send` payloads
//!     (CQ-SEND1), but NOT `Sync` because `&CommandQueue` does not permit
//!     concurrent push / apply. The `!Sync` half is the contract that
//!     compile-fails any `par_iter` body capturing `&mut Commands`
//!     (CQ-SEND2 — exercised via the trybuild test
//!     `par_iter_captures_commands_fails.rs`).
//!
//! The trait-bound checks are at runtime only in name — they are turned
//! into compile-time errors by the `#[test]` body that calls the
//! `assert_send` / `assert_sync` helpers with the type as a turbofish.
//! When a regression removes `Send` or `Sync`, this file stops compiling.

use boyko_ecs::ecs::core::commands::CommandQueue;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// Compile-time gate — fails to compile if `T: !Send`.
fn assert_send<T: Send>() {}

/// Compile-time gate — fails to compile if `T: !Send + Sync`.
fn assert_send_sync<T: Send + Sync>() {}

// ── Send + Sync (workers may freely cross with these) ───────────────────────

#[test]
fn ecs_master_is_send_sync() {
    // SEND1 — plan §9.1. The dispatcher hands `UnsafeEcsCell` copies to
    // workers, but the master itself logically crosses the dispatcher /
    // worker boundary via the cell's raw pointer. The `Send + Sync` impl
    // is on `EcsMaster` directly (see crates/boyko_ecs/src/ecs/core/
    // ecs_master/ecs_master.rs:1286).
    assert_send_sync::<EcsMaster>();
}

#[test]
fn unsafe_ecs_cell_is_send_sync() {
    // SEND3 — plan §2.4. The cell is `Copy`; `scope.spawn` captures a
    // copy by value. Its Send/Sync impls are stated on `UnsafeEcsCell<'w>`
    // for every `'w`.
    assert_send_sync::<UnsafeEcsCell<'static>>();
}

// ── Send-but-not-Sync (CommandQueue per CQ-SEND1 / CQ-SEND2) ────────────────

#[test]
fn command_queue_is_send() {
    // CQ-SEND1 — plan §2.4 / command_queue.rs:78. The byte-arena queue's
    // payloads are `Command: Send + 'static`, so the queue is hand-marked
    // Send. This is load-bearing for the future scheduler optimisation
    // where command flushes may be sharded across workers; today the
    // discipline is single-writer (the apply runs on the dispatcher).
    assert_send::<CommandQueue>();
}

/// `Commands<'s>` is the user-facing handle; `Commands<'s>: !Sync` is the
/// load-bearing contract that compile-fails any `par_iter` body capturing
/// `&mut Commands`. The compile-fail half is exercised by trybuild in
/// `tests/par_iter_captures_commands_fails.rs` (CQ-SEND2). Here we only
/// assert the positive Send + Sync claim of the queue itself — checking
/// `!Sync` directly requires negative-impl detection which Rust does not
/// expose to surface trait bounds. The trybuild test is the canonical
/// `!Sync` proof.
#[test]
fn command_queue_send_marker_pinned() {
    // Documentation marker — the function above already pins the Send
    // bound at compile time. Keeping this test as a named anchor for
    // future bisection (a regression that accidentally added Sync to
    // CommandQueue would still pass `assert_send`, but the trybuild
    // companion would fail to fail — i.e. the .stderr baseline would
    // diverge). The two tests together form a tight gate.
    assert_send::<CommandQueue>();
}
