//! Miri coverage for the apply walk's cursor guard (`CursorSync` in
//! `src/ecs/core/commands/command_queue.rs`), under BOTH borrow models.
//!
//! Run via (UG-08 recipe, `crates/boyko_symcensus/ug15/RECIPES.md`):
//! ```bash
//! # Tree Borrows leg
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-strict-provenance" \
//!   cargo +nightly-x86_64-pc-windows-msvc miri test \
//!   --config 'target."cfg(windows)".rustflags=["-Zrandomize-layout"]' \
//!   -p boyko-ecs --test miri_command_queue_cursor
//! # Stacked Borrows leg: the same with `-Zmiri-tree-borrows` removed
//! MIRIFLAGS="-Zmiri-strict-provenance" cargo +nightly-x86_64-pc-windows-msvc miri test …
//! ```
//!
//! # What this target isolates
//!
//! The walk keeps its read position in a stack-local cursor and relies on a
//! scope guard to publish that local into the queue's persistent `cursor`
//! field on every exit — normal completion, unwind out of a panicking
//! `Command::apply`, and the drop-only walk a dropped queue runs. Until the
//! fix this target was written for, the guard reached the local through a
//! `&raw const` pointer taken BEFORE the loop wrote the local directly.
//! Under Stacked Borrows a raw pointer to a local carries its own tag, and a
//! direct write to the local pops it (`wip/stacked-borrows.md`, "Accessing
//! memory", write rule), so the guard's read was UB on every walk that
//! reached at least one command. Tree Borrows accepts the same code, because
//! a raw pointer taken directly to a local inherits the local's tag
//! (R. Jung, "From Stacked Borrows to Tree Borrows", 2023-06-02: the
//! `let ptr = addr_of_mut!(x); x = 1; ptr.read();` example). The project
//! requires both legs green (UG-08), so this target runs under both, and
//! each test below drives the guard through one of its three exits with the
//! local cursor written at least twice before the guard reads it. The guard
//! now reaches the local only through a `&mut` borrow it holds.
//!
//! The four tests that surfaced the defect (`miri_pool_growth`'s churn test
//! and three `miri_phase22` tests) reach the guard only through a full
//! `EcsMaster` system run; this target reaches it with a bare queue and
//! no-op commands, so a regression is caught in one small binary.

#![cfg(miri)]

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::commands::command_queue::CommandQueue;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;

/// A no-op command that counts its applies and its drops. The counters are
/// per-test statics, so the tests stay independent under the parallel
/// harness.
struct Counted {
    applied: &'static AtomicUsize,
    dropped: &'static AtomicUsize,
}

impl Command for Counted {
    fn apply(self, _world: &mut EcsMaster) {
        self.applied.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

/// A command whose `apply` panics, so the guard's `Drop` runs during unwind.
struct Panics {
    dropped: &'static AtomicUsize,
}

impl Command for Panics {
    fn apply(self, _world: &mut EcsMaster) {
        panic!("miri_command_queue_cursor: deliberate panic inside Command::apply");
    }
}

impl Drop for Panics {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

/// Normal completion: the walk writes the local cursor for each of two
/// commands (header advance in the loop, payload advance in the glue), then
/// the success path drops the guard explicitly and the guard reads the local.
#[test]
fn miri_cursor_guard_success_exit() {
    static APPLIED: AtomicUsize = AtomicUsize::new(0);
    static DROPPED: AtomicUsize = AtomicUsize::new(0);

    let mut world = EcsMaster::new();
    let mut queue = CommandQueue::__test_new();
    queue.__test_push(Counted { applied: &APPLIED, dropped: &DROPPED });
    queue.__test_push(Counted { applied: &APPLIED, dropped: &DROPPED });
    queue.__test_apply(&mut world);

    assert_eq!(APPLIED.load(Ordering::Relaxed), 2, "both commands applied");
    assert_eq!(DROPPED.load(Ordering::Relaxed), 2, "both commands dropped once");
    assert_eq!(queue.__test_bytes_len(), 0, "the success path drains the arena");
}

/// Unwind exit: the guard's `Drop` runs while a panic from the second command
/// unwinds through the walk, and the survivor range the recovery computes is
/// only right if the guard published the local cursor (already advanced past
/// the panicker by the glue). The redrive then applies the survivor.
#[test]
fn miri_cursor_guard_unwind_exit() {
    static APPLIED: AtomicUsize = AtomicUsize::new(0);
    static DROPPED: AtomicUsize = AtomicUsize::new(0);
    static PANICKER_DROPPED: AtomicUsize = AtomicUsize::new(0);

    let mut world = EcsMaster::new();
    let mut queue = CommandQueue::__test_new();
    queue.__test_push(Counted { applied: &APPLIED, dropped: &DROPPED });
    queue.__test_push(Panics { dropped: &PANICKER_DROPPED });
    queue.__test_push(Counted { applied: &APPLIED, dropped: &DROPPED });

    let outcome = panic::catch_unwind(AssertUnwindSafe(|| queue.__test_apply(&mut world)));
    assert!(outcome.is_err(), "the panicker's panic propagates out of apply");
    assert_eq!(APPLIED.load(Ordering::Relaxed), 1, "only the command before the panicker ran");
    assert_eq!(PANICKER_DROPPED.load(Ordering::Relaxed), 1, "the panicker dropped once, on unwind");
    assert!(queue.__test_bytes_len() > 0, "the survivor was re-absorbed into the arena");
    assert_eq!(queue.__test_recovery_len(), 0, "top-level recovery is re-absorbed in the same call");

    queue.__test_apply(&mut world);
    assert_eq!(APPLIED.load(Ordering::Relaxed), 2, "the redrive applied the survivor");
    assert_eq!(DROPPED.load(Ordering::Relaxed), 2, "each counted command dropped once");
    assert_eq!(PANICKER_DROPPED.load(Ordering::Relaxed), 1, "the panicker is never redriven");
    assert_eq!(queue.__test_bytes_len(), 0, "the redrive drains the arena");
}

/// Drop-only exit: a queue dropped with un-applied commands runs the same
/// walk with no world, and the guard's `Drop` reads the local on the way out.
#[test]
fn miri_cursor_guard_drop_only_exit() {
    static APPLIED: AtomicUsize = AtomicUsize::new(0);
    static DROPPED: AtomicUsize = AtomicUsize::new(0);

    {
        let mut queue = CommandQueue::__test_new();
        queue.__test_push(Counted { applied: &APPLIED, dropped: &DROPPED });
        queue.__test_push(Counted { applied: &APPLIED, dropped: &DROPPED });
    }

    assert_eq!(APPLIED.load(Ordering::Relaxed), 0, "the drop-only walk never applies");
    assert_eq!(DROPPED.load(Ordering::Relaxed), 2, "each pending command dropped once");
}
