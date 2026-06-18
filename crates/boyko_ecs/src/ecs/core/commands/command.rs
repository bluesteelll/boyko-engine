//! The `Command` trait — the unit of deferred world mutation.
//!
//! Phase 8d Step 6 / plan §11. The trait is intentionally minimal: a
//! `Command` is a value-typed payload that, when flushed by
//! `super::CommandQueue::apply`, consumes itself (`self` by value) and
//! mutates the world via exclusive `&mut EcsMaster`.
//!
//! # `Send + 'static`
//!
//! The bound makes the byte arena `Send` (invariant **CQ-SEND1**) and
//! guarantees no borrowed references survive into the queue's storage.
//! Both are required for Phase 9's scheduler.
//!
//! # Why no `Sync`
//!
//! Single-writer queue ownership (per-system, invariant **CQ5**) means
//! `&Command` never needs to cross threads. `Sync` would be additional
//! cognitive load for no benefit at this phase.

// Step 6 deliverable: trait + glue fnptr. The fnptr type alias and the
// glue function become alive in Step 7 (via `CommandQueue::push<C>` call
// sites). Suppress dead-code warnings until then; the allow follows the
// project convention established in `iters/query/iter.rs`.
#![allow(dead_code)]

use std::mem::{self, MaybeUninit};
use std::ptr::{self, NonNull};

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

/// A deferred world mutation.
///
/// Commands are enqueued via `Commands::spawn(...)` / `Commands::despawn(...)`
/// / `Commands::add(cmd)` (Step 7) and flushed by
/// `super::CommandQueue::apply` after the system body returns.
///
/// # Contract — invariants CQ4, CQ7
///
/// * **CQ4** — `apply` is invoked at most once per command instance. The
///   queue's apply loop calls it from inside
///   `super::CommandQueue::apply`; the panic-recovery path SKIPS the
///   panicker on redrive (W3' RESOLUTION).
/// * **CQ7** — `apply` receives exclusive `&mut EcsMaster`. APP4 forbids
///   re-entry into `EcsMaster::run_system_once` / `run_closure_once` from
///   within `apply`.
///
/// # Drop safety
///
/// If `apply` is never called (the queue is dropped with un-flushed
/// commands), the per-type drop glue (`consume_and_drop_glue`) is
/// invoked with `world = None` so the command's [`Drop`] impl runs once
/// and only once.
pub trait Command: Send + 'static {
    /// Performs the deferred mutation, consuming `self`.
    fn apply(self, world: &mut EcsMaster);
}

/// Single-fnptr dispatch entry stored alongside each command's payload in
/// the queue's byte arena (invariant **CQ2** — 8 bytes on x86_64).
///
/// `#[repr(C)]` so the layout is predictable across compilers and the
/// `write_unaligned` / `read_unaligned` round-trip is sound.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CommandMeta {
    /// Type-erased dispatch fnptr — see [`consume_and_drop_glue`].
    pub(crate) consume_and_drop: ConsumeAndDropFn,
}

/// Type-erased glue fnptr — calls a concrete `Command::apply` (when
/// `world` is `Some`) or drops the command in-place (when `world` is
/// `None`).
///
/// The `cursor` parameter is advanced PAST the command's bytes BEFORE
/// `Command::apply` runs (invariant **W3'**). On panic mid-apply, the
/// recovery loop sees `cursor` already past the panicker and excludes it
/// from the recovery range.
///
/// # Safety contract
///
/// See [`consume_and_drop_glue`] — every invocation must satisfy the
/// preconditions documented there.
pub(crate) type ConsumeAndDropFn = unsafe fn(
    value: *mut MaybeUninit<u8>,
    world: Option<NonNull<EcsMaster>>,
    cursor: &mut usize,
);

/// Per-type drop / apply glue. Erased through [`ConsumeAndDropFn`] and
/// stored in [`CommandMeta::consume_and_drop`].
///
/// # Safety
///
/// The caller (always [`super::command_queue::RawCommandQueue::apply_or_drop_queued`])
/// MUST satisfy:
///
/// * `value` points at `mem::size_of::<C>()` bytes that form a valid `C`
///   — i.e. bytes were written by [`super::CommandQueue::push::<C>`] via
///   `ptr::write_unaligned::<C>` and have not been read out since.
/// * The caller holds exclusive access to those bytes for the call's
///   duration (no concurrent reader, no other glue invocation).
/// * If `world` is `Some`, the [`NonNull<EcsMaster>`] points at a live
///   master that the caller holds exclusively (`&mut EcsMaster` reborrowed
///   into NonNull via `NonNull::from`).
/// * `cursor` is non-null, points at the apply loop's local cursor, and
///   may be mutated.
///
/// **CQ-PACK1** — we use `ptr::read_unaligned` on `value as *mut C`. We
/// NEVER construct `&C` or `&mut C` into the unaligned byte slot.
///
/// # W3' — cursor advance discipline
///
/// `*cursor += mem::size_of::<C>()` runs UNCONDITIONALLY BEFORE
/// `cmd.apply(world)`. On panic mid-`apply`, the local `cmd: C` was
/// already moved out of `value` by `ptr::read_unaligned` on entry; the
/// unwind path drops `cmd` exactly once via local drop-elaboration. The
/// byte slot at `value` is left logically uninitialized (its contents
/// were moved into `cmd`); since `cursor` is already past it, the
/// recovery loop excludes it.
///
/// Net effect:
///
/// * No double-drop — `cmd` drops once on the unwind path; the byte slot
///   is never re-processed.
/// * No leak on panic — the panicker drops cleanly via local unwind;
///   survivors `[cursor..bytes.len()]` drop via the recovery redrive.
pub(crate) unsafe fn consume_and_drop_glue<C: Command>(
    value: *mut MaybeUninit<u8>,
    world: Option<NonNull<EcsMaster>>,
    cursor: &mut usize,
) {
    debug_assert!(!value.is_null(), "consume_and_drop_glue: value pointer null");

    // SAFETY (CQ-PACK1):
    //   - `ptr::read_unaligned` does not require alignment.
    //   - It does NOT create an intermediate reference (`&C` or `&mut C`)
    //     into the byte slot — critical for Tree Borrows.
    //   - The caller guarantees `value` points at `sizeof::<C>()` bytes
    //     forming a valid `C` (CQ2 — written by `push<C>::write_unaligned`).
    //   - After this read, the byte slot is logically uninitialized; the
    //     queue's cursor (advanced below) guarantees no further reads.
    let cmd: C = unsafe { ptr::read_unaligned(value as *mut C) };

    // W3' — advance the caller's cursor PAST this command's bytes BEFORE
    // running `cmd.apply`. The meta header was already advanced by the
    // apply loop; we own the payload-size advance. If `cmd.apply` panics
    // below, the caller's `local_cursor` is already past us — recovery
    // excludes us.
    *cursor += mem::size_of::<C>();

    match world {
        Some(world_ptr) => {
            // SAFETY (CQ7):
            //   - The caller (CommandQueue::apply via RawCommandQueue)
            //     holds `&mut EcsMaster` exclusively for the call's
            //     duration; APP4 forbids re-entry from within `cmd.apply`.
            //   - `world_ptr` was minted via `NonNull::from(&mut *world)`
            //     — the pointee is live and uniquely borrowed.
            let world: &mut EcsMaster = unsafe { &mut *world_ptr.as_ptr() };
            cmd.apply(world);
        }
        None => {
            // Drop-only path (CommandQueue::Drop with un-flushed commands).
            // `cmd` falls out of scope; its Drop runs exactly once.
            drop(cmd);
        }
    }
}
