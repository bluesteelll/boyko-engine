//! `CommandQueue` — type-erased, packed byte-arena for deferred commands.
//!
//! Phase 8d Step 6 / plan §10. The queue stores each command as
//! `[CommandMeta (8 B)][command bytes (sizeof::<C>())]`, all writes via
//! `ptr::write_unaligned` and all reads via `ptr::read_unaligned`. No
//! `Packed<T>` wrapper, no `&` / `&mut` reference creation into the byte
//! slots (invariant **CQ-PACK1**).
//!
//! The apply machinery uses a raw-pointer twin [`RawCommandQueue`] minted
//! via `&raw mut` from `&mut CommandQueue` (invariant **C3** — no
//! intermediate reference creation; Tree Borrows compatible).

// Step 6 deliverable: the queue and its raw twin. Step 7 (the `Commands`
// SystemParam) is the first non-test consumer of `push` / `apply`; the
// in-file unit tests below already exercise the full path. Suppress
// dead-code on the lib build until Step 7 wires the public consumer.
#![allow(dead_code)]

use std::mem::{self, MaybeUninit};
use std::panic::AssertUnwindSafe;
use std::ptr::NonNull;

use crate::ecs::core::commands::command::{
    Command, CommandMeta, ConsumeAndDropFn, consume_and_drop_glue,
};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

/// Byte offset of a command's payload from the start of its slot in the
/// queue's byte arena. Equal to `mem::size_of::<CommandMeta>()` — replaces
/// the rejected `Packed<T>` wrapper (C4 FIX).
///
/// All reads / writes at this offset use `read_unaligned` /
/// `write_unaligned`; reference creation into the byte slot is forbidden
/// by invariant **CQ-PACK1**.
const COMMAND_PAYLOAD_OFFSET: usize = mem::size_of::<CommandMeta>();

/// Type-erased, packed, byte-arena command queue.
///
/// # Layout (invariants CQ1, CQ2)
///
/// * `bytes: Vec<MaybeUninit<u8>>` — packed commands. `MaybeUninit<u8>`
///   permits commands with internal padding (Bevy PR #6391).
/// * `cursor: usize` — apply-time read position. `bytes.len()` at rest;
///   advances during apply.
/// * `panic_recovery: Vec<MaybeUninit<u8>>` — un-run tail of a panicked
///   apply. **OPAQUE between applies** (C2' FIX — Round 2's "Step 0.5"
///   prepend was wrong); recovery is re-absorbed into `bytes` only inside
///   the `catch_unwind` Err branch when `start == 0` (top-level), matching
///   Bevy exactly.
///
/// # Size (O2)
///
/// `2 × Vec` headers (24 B each on 64-bit) + 1 × `usize` cursor = **56 B**.
/// Stack-resident per system; fits in one cache line.
///
/// # Send / Sync (CQ-SEND1 / W1')
///
/// `CommandQueue: Send` via explicit `unsafe impl` — every command stored
/// in the byte arena satisfies `Command: Send + 'static`, so the bytes are
/// transitively Send. `Sync` is NOT implemented — `&CommandQueue` does not
/// allow concurrent push / apply; the per-system ownership (CQ5) gives
/// single-writer access in Phase 8d.
pub struct CommandQueue {
    pub(crate) bytes: Vec<MaybeUninit<u8>>,
    pub(crate) cursor: usize,
    pub(crate) panic_recovery: Vec<MaybeUninit<u8>>,
}

// SAFETY (CQ-SEND1, W1'):
//   Every command enqueued via `push<C>` satisfies `C: Command: Send + 'static`
//   (the bound is on the push entry point). The byte arena therefore holds
//   only Send bytes. The auto-trait `Send` is NOT derived because
//   `Vec<MaybeUninit<u8>>` is Send for any T:Send and MaybeUninit<u8> is
//   Send, but we state the explicit impl to document the load-bearing
//   `Command: Send + 'static` invariant.
//
//   Mirrors Bevy's `unsafe impl Send for CommandQueue`.
unsafe impl Send for CommandQueue {}

// Intentionally NOT `Sync` — `&CommandQueue` does not permit safe
// concurrent access. Phase 8d gives single-writer ownership via per-system
// queues (CQ5); Phase 9's scheduler is the next layer of arbitration.

impl CommandQueue {
    /// Constructs a fresh, empty queue.
    ///
    /// `Vec::new()` is allocation-free (`Vec` defers heap allocation until
    /// the first push). The 56 B stack layout is materialised in place at
    /// the system's `State` field.
    pub(crate) const fn new() -> Self {
        Self {
            bytes: Vec::new(),
            cursor: 0,
            panic_recovery: Vec::new(),
        }
    }

    /// Returns `true` when no command bytes are queued.
    ///
    /// Mirrors the empty-queue early-out condition in [`Self::apply`]
    /// (command_queue.rs:215): `panic_recovery` is OPAQUE and does not affect
    /// emptiness — at rest it is always empty in single-level use (the Err
    /// branch re-absorbs recovery into `bytes` in the same call). Used by
    /// `EcsMaster::drain_deferred_hook_queue` to loop until the deferred queue
    /// is quiescent (Phase 14a, plan §8 P2).
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Pushes `cmd: C` into the queue.
    ///
    /// # Layout written
    ///
    /// ```text
    /// [CommandMeta (8 B)][C payload (sizeof::<C>())]
    /// ```
    ///
    /// Both segments written via `write_unaligned` into successive byte
    /// offsets — no `Packed<T>` wrapper, no reference creation into
    /// unaligned memory (invariant **CQ-PACK1**).
    ///
    /// # Cost (D1 target: <= 20 ns)
    ///
    /// * `bytes.reserve(meta + cmd size)` — amortised, cold on first growth.
    /// * Two unaligned writes.
    /// * `bytes.set_len(...)`.
    pub(crate) fn push<C: Command>(&mut self, cmd: C) {
        let meta = CommandMeta {
            consume_and_drop: consume_and_drop_glue::<C> as ConsumeAndDropFn,
        };
        let cmd_size = mem::size_of::<C>();
        let total = COMMAND_PAYLOAD_OFFSET + cmd_size;
        let old_len = self.bytes.len();
        self.bytes.reserve(total);
        debug_assert!(
            self.bytes.capacity() >= old_len + total,
            "invariant: reserve must guarantee capacity >= old_len + total"
        );

        // SAFETY (CQ1, CQ2, CQ-PACK1):
        //   - `bytes.capacity() >= old_len + total` post-`reserve` (debug-asserted above).
        //   - `MaybeUninit<u8>` is byte-pattern-agnostic — writes to uninit
        //     slots are sound.
        //   - Both writes are `write_unaligned`; the byte slot has no
        //     required alignment. We NEVER construct `&` or `&mut`
        //     references into the byte slot (CQ-PACK1).
        //   - `set_len(old_len + total)` reflects the bytes we just
        //     initialised; subsequent `read_unaligned` from those offsets
        //     is sound (CQ2).
        //   - `cmd: C` is moved by value into `write_unaligned`'s argument
        //     slot, then bitwise-copied into the queue's bytes. The queue's
        //     bytes are the new logical owner; no local `Drop` runs
        //     (`write_unaligned` is a bitwise copy that does NOT invoke
        //     `Drop` on the destination, and the destination was
        //     `MaybeUninit` anyway).
        unsafe {
            let base = self.bytes.as_mut_ptr().add(old_len);
            std::ptr::write_unaligned(base as *mut CommandMeta, meta);
            std::ptr::write_unaligned(base.add(COMMAND_PAYLOAD_OFFSET) as *mut C, cmd);
            self.bytes.set_len(old_len + total);
        }
    }

    /// Mints a [`RawCommandQueue`] borrowed from `self` for the duration
    /// of the returned struct's use. No intermediate `&` / `&mut`
    /// references to the inner fields are created — Tree Borrows OK (C3 FIX).
    fn raw(&mut self) -> RawCommandQueue {
        // SAFETY (C3):
        //   - `&raw mut self.bytes` (and friends) mint raw pointers from
        //     the live `&mut self` WITHOUT creating intermediate references
        //     (no `&mut self.bytes` is materialised here).
        //   - The pointers are guaranteed non-null because they were
        //     derived from a live `&mut`-borrowed struct.
        //   - For the duration of any subsequent call on the returned
        //     `RawCommandQueue`, the caller MUST NOT touch `self` directly;
        //     the raw twin is the sole accessor. Enforced by the call
        //     sites in `apply` / `Drop` below.
        unsafe {
            RawCommandQueue {
                bytes: NonNull::new_unchecked(&raw mut self.bytes),
                cursor: NonNull::new_unchecked(&raw mut self.cursor),
                panic_recovery: NonNull::new_unchecked(&raw mut self.panic_recovery),
            }
        }
    }

    /// Apply-then-drop all queued commands.
    ///
    /// # Semantics (C2' / Bevy mirror — Phase 12.5 Opt-A1 hoist)
    ///
    /// 1. Empty-bytes early-out (D1 target: <= 3 ns). `panic_recovery` is
    ///    OPAQUE — its emptiness does not affect the early-out; recovery
    ///    only matters inside the `catch_unwind` Err branch.
    /// 2. Snapshot `start = cursor` (always 0 in current single-level use)
    ///    and `stop_snapshot = bytes.len()`.
    /// 3. Walk commands from `start` to `stop_snapshot` inside a SINGLE
    ///    `catch_unwind` (Phase 12.5 Opt-A1 — hoisted out of the per-command
    ///    loop; ~5-10 ns/cmd saved). Per-command dispatch is via
    ///    [`RawCommandQueue::apply_or_drop_queued_no_catch`].
    /// 4. On panic during any command's `apply`:
    ///    * The cursor was already advanced PAST the panicker by
    ///      [`consume_and_drop_glue`] before the panic (W3').
    ///    * The remaining range `[local_cursor..bytes.len()]` is captured
    ///      into `panic_recovery` by [`RawCommandQueue::handle_panic_recovery`].
    ///    * If `start == 0` (top-level), `panic_recovery` is appended back
    ///      into `bytes` in the SAME panicking call (Bevy mirror) so the
    ///      next `apply` walks the survivors.
    ///    * `resume_unwind` propagates the original panic.
    /// 5. On success:
    ///    * If `bytes.len() > stop_snapshot`, command-during-apply pushes
    ///      occurred (Q-A1.1 case 4 fix). The new bytes at
    ///      `[stop_snapshot..)` are compacted down to `[start..)` via
    ///      `ptr::copy` (overlap-tolerant). The previous implementation
    ///      called `set_len(start)` and silently discarded those pushes.
    pub(crate) fn apply(&mut self, world: &mut EcsMaster) {
        // Empty-queue early-out (D1 target: <= 3 ns).
        //
        // Bevy invariant: `panic_recovery` non-empty implies `bytes`
        // non-empty at top-level. The Err branch always re-absorbs recovery
        // into bytes in the same call, so an empty-bytes / non-empty-
        // recovery state cannot persist between applies in single-level
        // (Phase 8d) use.
        debug_assert!(
            self.panic_recovery.is_empty() || !self.bytes.is_empty(),
            "invariant (Bevy): panic_recovery non-empty implies bytes non-empty",
        );
        if self.bytes.is_empty() {
            return;
        }

        // Mint the raw twin; release the safe `&mut self` for the
        // duration of the walk (C3 FIX).
        let mut raw = self.raw();
        let world_ptr = NonNull::from(&mut *world);

        // Phase 12.5 Opt-A1: a SINGLE catch_unwind wraps the entire
        // per-queue walk. Panic-recovery is centralised inside
        // `handle_panic_recovery` (called from the Err branch only).
        //
        // SAFETY (C3, CQ4):
        //   - `raw` was derived from `&mut self` via `&raw mut` — no
        //     intermediate `&` / `&mut` references on the struct's fields
        //     were created, so Tree Borrows is satisfied.
        //   - For the duration of `apply_or_drop_queued_no_catch`, we do
        //     NOT touch `self`'s fields directly; `raw` is the sole accessor.
        //   - `world_ptr` is minted via `NonNull::from(&mut *world)`;
        //     the pointee is uniquely borrowed (we hold `&mut EcsMaster`
        //     exclusively).
        //   - APP4 forbids re-entry into `run_system_once` /
        //     `run_closure_once` from inside any command's `apply` body —
        //     the borrow checker enforces this at the API boundary.
        let walk = AssertUnwindSafe(|| unsafe {
            raw.apply_or_drop_queued_no_catch(Some(world_ptr));
        });

        if let Err(payload) = std::panic::catch_unwind(walk) {
            // SAFETY: exclusive access invariant identical to the success
            //   path. `start == 0` for single-level Phase 8d use; recovery
            //   re-absorbs into bytes before resume.
            unsafe { raw.handle_panic_recovery(0) };
            std::panic::resume_unwind(payload);
        }
    }

    /// Phase 14a (plan §8 P2 / SAFETY-5 / SAFETY-6): drain
    /// `world.deferred_hook_queue` with full [`Self::apply`] semantics — the
    /// `raw()` mint + a single `catch_unwind` + `handle_panic_recovery(0)`
    /// survivor re-absorb — taking the world as [`NonNull<EcsMaster>`] instead
    /// of `&mut`, because the deferred queue is a **field** of `world`.
    ///
    /// An associated fn (NOT a `&mut self` method) by design: a `&mut self`
    /// receiver on the queue field plus the `&mut *world` each `cmd.apply`
    /// forms would alias (the W5-statement hazard). Reaching the queue through
    /// `world` instead keeps `raw()` / [`RawCommandQueue`] private to this file
    /// and avoids the field-alias entirely.
    ///
    /// # Safety
    ///
    /// * `world` is a valid, exclusively-borrowed [`EcsMaster`] (the caller
    ///   holds `&mut EcsMaster` and minted this `NonNull` from it; SAFETY-4
    ///   window — `IN_SYSTEM_RUN == false`).
    /// * All queue access (`bytes` / `cursor` / `panic_recovery`) is threaded
    ///   through the raw twin's `NonNull` (see [`CursorSync`]'s discipline); a
    ///   transient `&mut *world` is formed ONLY
    ///   per `cmd.apply`, never while a `&mut`-into-the-queue is live. The
    ///   bytes `Vec`'s heap buffer is a separate allocation from `EcsMaster`,
    ///   so the byte walk never aliases `&mut *world`; the in-`EcsMaster`
    ///   cursor / recovery writes are sequenced through raw pointers, never
    ///   simultaneous with `&mut *world` (SAFETY-5).
    pub(crate) unsafe fn apply_via_raw_twin(world: NonNull<EcsMaster>) {
        // SAFETY: transient `&mut` to the queue field, used only to mint the
        //   raw twin; dropped immediately after. The world is exclusively
        //   borrowed per the fn contract.
        let mut twin = unsafe { (*world.as_ptr()).deferred_hook_queue.raw() };

        // Single catch — identical shape to `apply` (:244-250). The catch
        // lives HERE (not in the catch-free `apply_or_drop_queued_no_catch`),
        // so SAFETY-6's "the drain has its own single catch_unwind" holds.
        //
        // SAFETY (C3, CQ4, SAFETY-5):
        //   - `twin` was derived from a live `&mut self`(queue) via `&raw mut`
        //     (no intermediate reference on the fields); Tree Borrows OK.
        //   - The walk forms a transient `&mut *world` per command; no
        //     `&mut`-into-the-queue is live across it (the twin reads the
        //     queue's heap buffers, a separate allocation).
        let walk = AssertUnwindSafe(|| unsafe {
            twin.apply_or_drop_queued_no_catch(Some(world));
        });

        if let Err(payload) = std::panic::catch_unwind(walk) {
            // SAFETY: same exclusive-access invariant as the walk; `start == 0`
            //   re-absorbs survivors into the SAME `bytes` (mirrors `apply`'s
            //   :248 + `handle_panic_recovery` :560-566) before re-raising.
            unsafe { twin.handle_panic_recovery(0) };
            std::panic::resume_unwind(payload);
        }
    }
}

/// Raw-pointer twin of [`CommandQueue`]. Apply-time machinery uses this
/// shape to avoid Tree Borrows UB (C3 FIX).
///
/// # Send / Sync
///
/// `RawCommandQueue: !Send + !Sync` (contains raw `NonNull` pointers; the
/// auto-traits are NOT derived for raw pointers). INTENTIONAL —
/// `RawCommandQueue` is a transient stack value inside one `apply` call;
/// it must never cross threads. Phase 9's scheduler arbitrates at the
/// `CommandQueue` (safe wrapper) level, not the raw twin.
#[derive(Clone, Copy)]
struct RawCommandQueue {
    bytes: NonNull<Vec<MaybeUninit<u8>>>,
    cursor: NonNull<usize>,
    panic_recovery: NonNull<Vec<MaybeUninit<u8>>>,
}

/// Phase 12.6 hybrid-cursor scope guard.
///
/// Mirrors Bevy's `command_queue.rs` local-cursor pattern: the hot loop in
/// [`RawCommandQueue::apply_or_drop_queued_no_catch`] reads / writes a
/// stack-local `local_cursor` (cheap register access; no per-iteration
/// `NonNull::as_ref` dereference on a heap-resident cursor field). This
/// guard's `Drop` writes the local back into the queue's persistent
/// cursor on EITHER normal completion OR unwind — so the Phase 12.5
/// Opt-A1 panic-recovery semantics survive:
///
///   * `handle_panic_recovery` reads `*self.cursor` to identify the
///     survivor range `[cursor..bytes.len())`. The guard's `Drop` must
///     fire BEFORE `handle_panic_recovery` so the queue's cursor reflects
///     `consume_and_drop_glue`'s W3' advance past the panicker.
///   * Rust drops locals in reverse declaration order; since the guard is
///     declared AFTER `local_cursor` inside the apply function, it drops
///     FIRST on unwind, while `local_cursor` is still alive on the stack.
///   * The outer `catch_unwind` in [`CommandQueue::apply`] runs
///     `handle_panic_recovery` only on the Err branch, AFTER unwinding
///     has dropped the guard.
struct CursorSync {
    /// Persistent queue cursor (heap-resident inside `CommandQueue::cursor`).
    cursor_ptr: NonNull<usize>,
    /// Pointer to the stack-local `local_cursor` in the apply walk.
    /// Valid for as long as the apply walk's stack frame is live; the
    /// guard's drop position guarantees the frame is still live when
    /// this read fires.
    local_ptr: *const usize,
}

impl Drop for CursorSync {
    #[inline]
    fn drop(&mut self) {
        // SAFETY:
        //   - `self.local_ptr` points at a `usize` stack-local in the
        //     apply walk's frame. The guard is declared AFTER that local
        //     so Rust's reverse-declaration drop order guarantees the
        //     local is still alive when this read runs (true for both
        //     normal exit and panic unwinding).
        //   - `self.cursor_ptr` is the queue's persistent cursor field.
        //     The apply walk holds exclusive access for the duration of
        //     the call; no other reader/writer is touching that slot.
        //   - The write is one `usize` store; no surrounding state needs
        //     synchronisation here (single-threaded queue access).
        unsafe {
            *self.cursor_ptr.as_ptr() = *self.local_ptr;
        }
    }
}

impl RawCommandQueue {
    /// Walk the queue; apply or drop each command in turn. **Catch-free**
    /// inner — the outer caller wraps the entire walk in ONE `catch_unwind`
    /// (Phase 12.5 Opt-A1 / §4).
    ///
    /// # Safety
    ///
    /// * The caller (always `CommandQueue::apply` or `CommandQueue::Drop`)
    ///   holds exclusive access to the underlying [`CommandQueue`] for the
    ///   call's duration — no other reader/writer touches the struct.
    /// * The `world` `NonNull`, if `Some`, points at a live [`EcsMaster`]
    ///   that the caller holds exclusively via `&mut`.
    /// * Re-entry from inside a command's `apply` body is forbidden by
    ///   APP4 (the command would need to call `run_system_once`, which
    ///   the borrow checker rejects since the caller already holds
    ///   `&mut EcsMaster`).
    /// * On entry, `cursor <= bytes.len()`.
    /// * On panic, the outer caller MUST invoke
    ///   [`Self::handle_panic_recovery`] before `resume_unwind` to capture
    ///   survivors. The function does NOT catch internally.
    unsafe fn apply_or_drop_queued_no_catch(&mut self, world: Option<NonNull<EcsMaster>>) {
        // SAFETY (raw NonNull → temporary `&` / `&mut` per access):
        //   The caller's exclusive access invariant guarantees no aliased
        //   reader / writer exists. Each `as_ref` / `as_mut` call below
        //   produces a short-lived reference whose lifetime does NOT
        //   overlap any other reference into the same field.
        let start = unsafe { *self.cursor.as_ref() };
        let stop_snapshot = unsafe { self.bytes.as_ref().len() };
        debug_assert!(start <= stop_snapshot, "invariant: cursor <= bytes.len()");

        // Phase 12.6 — hybrid cursor pattern (mirrors Bevy's `command_queue.rs:240`):
        //
        // The hot loop reads / writes a stack-local `local_cursor` (cheap
        // register access). A scope-guard `CursorSync` writes the local
        // back into `*self.cursor` on EITHER normal completion OR unwind
        // (the guard's `Drop` impl fires during stack unwinding too).
        //
        // Why this preserves Phase 12.5 Opt-A1 panic-recovery semantics:
        //
        //   * `consume_and_drop_glue` advances `*cursor_ref += sizeof::<C>()`
        //     BEFORE `cmd.apply` runs (W3' discipline). The `cursor_ref`
        //     passed in is `&mut local_cursor` — the advance lands on the
        //     stack-local.
        //   * On panic mid-apply, unwind drops `_guard` BEFORE dropping
        //     `local_cursor` (declared after `local_cursor` ⇒ dropped
        //     before in LIFO order). The guard's `Drop` reads
        //     `*self.local_ptr` (the up-to-date local) and writes it into
        //     `*self.cursor_ptr` (the queue's persistent cursor) — exactly
        //     what `handle_panic_recovery` needs to identify the survivor
        //     range `[local_cursor..bytes.len())`.
        //   * On normal completion, the loop exits with `local_cursor ==
        //     stop_snapshot`; the success-path block writes
        //     `*self.cursor.as_mut() = start` overriding the guard's
        //     write, which is fine — both happen under exclusive access.
        //
        // The loop bound stays the LOCAL `stop_snapshot`, so commands
        // enqueued by command-during-apply (pushing past `stop_snapshot`)
        // are NOT re-entered into the current walk (Q-A1.1 case 4 fix
        // happens in the post-loop compaction block below).
        let mut local_cursor: usize = start;

        // SAFETY: `local_cursor` lives on this stack frame until the
        //   function returns or unwinds. The guard reads its value via
        //   raw pointer in its `Drop` impl; since the guard is declared
        //   AFTER `local_cursor`, Rust's reverse-declaration drop order
        //   fires the guard's Drop FIRST during unwind, while
        //   `local_cursor` is still alive on the stack. The `cursor_ptr`
        //   is `self.cursor` — a `NonNull<usize>` valid for the duration
        //   of the function call (the underlying field lives on the heap
        //   via the queue's owner, and we hold exclusive access).
        let _guard = CursorSync {
            cursor_ptr: self.cursor,
            local_ptr: &raw const local_cursor,
        };

        while local_cursor < stop_snapshot {
            // Read meta at the current cursor.
            //
            // SAFETY (CQ2):
            //   - The bytes at `local_cursor` were populated by
            //     `CommandQueue::push<C>`, which wrote a `CommandMeta` via
            //     `write_unaligned`.
            //   - `read_unaligned` requires no alignment and creates no
            //     intermediate reference.
            //   - `local_cursor + COMMAND_PAYLOAD_OFFSET <= stop_snapshot`
            //     holds because every pushed command writes a full
            //     `meta + payload` block.
            let meta = unsafe {
                self.bytes
                    .as_mut()
                    .as_mut_ptr()
                    .add(local_cursor)
                    .cast::<CommandMeta>()
                    .read_unaligned()
            };

            // Advance the local cursor past the meta header. The guard
            // will sync this to `*self.cursor` on Drop.
            local_cursor += COMMAND_PAYLOAD_OFFSET;

            // Pointer to the command's payload bytes.
            //
            // SAFETY:
            //   - `local_cursor < bytes.len()` (`push` wrote the payload
            //     immediately after the meta header).
            //   - The resulting pointer is for `consume_and_drop_glue` to
            //     `read_unaligned::<C>` from; no reference is created here.
            let cmd_ptr = unsafe { self.bytes.as_mut().as_mut_ptr().add(local_cursor) };

            // Pass `&mut local_cursor` to the glue — when
            // `consume_and_drop_glue` advances `*cursor += sizeof::<C>()`
            // (W3'), it advances our stack-local. The guard's Drop
            // mirrors that advance into `*self.cursor` on either normal
            // exit or unwind.
            //
            // SAFETY (CQ-PACK1 + CQ7):
            //   - The byte slot at `cmd_ptr` was populated by
            //     `push<C>::write_unaligned` and has not been read out
            //     since.
            //   - We hold exclusive access to the bytes (caller's
            //     invariant). `&mut local_cursor` is a unique borrow of
            //     the stack-local — no other reference into it lives
            //     across this call.
            //   - `world` is a live exclusive `&mut EcsMaster` if Some.
            //   - W3' (`consume_and_drop_glue`) advances the cursor by
            //     `sizeof::<C>()` BEFORE running `cmd.apply`. On panic,
            //     the stack-local is already past the panicker; the
            //     guard's `Drop` syncs it into `*self.cursor` before the
            //     outer `catch_unwind` invokes `handle_panic_recovery`,
            //     which reads `*self.cursor` to identify the survivor
            //     range and excludes the panicker (its bytes were moved
            //     into `cmd: C` by `ptr::read_unaligned` and dropped via
            //     local unwind).
            unsafe {
                (meta.consume_and_drop)(cmd_ptr, world, &mut local_cursor);
            }
        }

        // Reached the success path: drop the guard NOW so it cannot
        // overwrite the `*self.cursor = start` reset below. (The guard
        // exists to sync `local_cursor` into `*self.cursor` on unwind;
        // on normal completion the success-path block writes `start`
        // directly, and the guard's last-second write of
        // `local_cursor == stop_snapshot` would clobber it.)
        drop(_guard);

        // Success path. Three sub-cases:
        //   (A) `bytes.len() == stop_snapshot` — no command-during-apply
        //       pushes; shrink to `start` (drop the applied range).
        //   (B) `bytes.len() > stop_snapshot` — command-during-apply pushes
        //       happened (Q-A1.1 case 4). The previous implementation
        //       called `set_len(start)` and SILENTLY DISCARDED the new
        //       bytes — a latent bug. Phase 12.5 Opt-A1 fixes this: copy
        //       the new bytes down to `[start..)` via `ptr::copy`
        //       (overlap-tolerant when `start + delta > stop_snapshot`).
        //
        // SAFETY:
        //   - Bytes `0..start` were valid before.
        //   - Bytes `[start..stop_snapshot)` have all been drained by
        //     `consume_and_drop_glue` (their `cmd: C` locals moved out
        //     and either applied or dropped); the slots are logically
        //     uninitialized.
        //   - Bytes `[stop_snapshot..new_stop)` are valid
        //     `[CommandMeta][payload]` entries written by `push<C>` during
        //     the walk.
        //   - `ptr::copy` is overlap-tolerant; both ranges live inside
        //     the same `Vec<MaybeUninit<u8>>` allocation.
        //   - `set_len` reflects the new logical length post-compaction.
        unsafe {
            let bytes = self.bytes.as_mut();
            let new_stop = bytes.len();
            if new_stop > stop_snapshot {
                let delta = new_stop - stop_snapshot;
                let base = bytes.as_mut_ptr();
                // SAFETY: source `[stop_snapshot..new_stop)` and dest
                //   `[start..start + delta)` both lie inside the same
                //   allocation. They may overlap when `start + delta >
                //   stop_snapshot`; `ptr::copy` handles overlap.
                std::ptr::copy(
                    base.add(stop_snapshot),
                    base.add(start),
                    delta,
                );
                bytes.set_len(start + delta);
            } else {
                bytes.set_len(start);
            }
            *self.cursor.as_mut() = start;
        }
    }

    /// Phase 12.5 Opt-A1 (§4.4): centralised panic-recovery helper.
    ///
    /// Captures the un-walked range `[local_cursor..bytes.len())` into
    /// `panic_recovery`, shrinks `bytes` to `start`, resets `cursor` to
    /// `start`, and (when `start == 0`) re-absorbs recovery into bytes
    /// so the next `apply` walks the survivors (Bevy semantic).
    ///
    /// # Safety
    ///
    /// Same exclusive-access invariant as
    /// [`Self::apply_or_drop_queued_no_catch`]. Invoked from the Err
    /// branch of the outer `catch_unwind` only.
    #[cold]
    #[inline(never)]
    unsafe fn handle_panic_recovery(&mut self, start: usize) {
        // SAFETY: same exclusive-access invariant as the apply walk; we
        //   briefly materialise `&mut` references to the bytes /
        //   panic_recovery fields here.
        let bytes = unsafe { self.bytes.as_mut() };
        let recovery = unsafe { self.panic_recovery.as_mut() };
        let local_cursor = unsafe { *self.cursor.as_ref() };
        let current_stop = bytes.len();
        debug_assert!(
            local_cursor <= current_stop,
            "invariant: cursor advanced by glue stays within bytes",
        );

        // `*self.cursor` was advanced past the panicker by W3' inside
        // `consume_and_drop_glue` (which now mutates the queue's own cursor
        // field directly — see `apply_or_drop_queued_no_catch`). The
        // survivor range is everything from there to `current_stop`
        // (= `bytes.len()` AT panic time, which already includes any
        // commands the panicker pushed before panicking — Q-A1.1 case 1).
        //
        // `Vec<MaybeUninit<u8>>::extend_from_slice` requires `Copy`
        // elements; `MaybeUninit<u8>` IS `Copy`, so this is a memcpy.
        recovery.extend_from_slice(&bytes[local_cursor..current_stop]);

        // SAFETY: shrinking the Vec to `start`; bytes `0..start` remain
        //   valid; the dropped suffix's bytes were either applied (those
        //   at `[start..local_cursor)`, drained by
        //   `consume_and_drop_glue`) or moved into `recovery` above
        //   (those at `[local_cursor..current_stop)`).
        unsafe {
            bytes.set_len(start);
            *self.cursor.as_mut() = start;
        }

        if start == 0 {
            // Top-level apply panicked: re-absorb recovery into bytes in
            // the SAME panicking call so the next apply walks the
            // survivors (Bevy semantic). `append` moves recovery's
            // contents into bytes; recovery is empty after this.
            bytes.append(recovery);
        }
    }
}

impl Default for CommandQueue {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Step 8 acceptance-test helpers (doc-hidden, integration-test only)
// =============================================================================
//
// Phase 8d Step 8 (panic-recovery acceptance) lives in
// `crates/boyko_ecs/tests/command_queue_panic_recovery.rs`. The integration
// test cannot reach `pub(crate)` items, so the queue exposes a tightly
// scoped `#[doc(hidden)] pub` surface used ONLY by that test file:
//
// * [`CommandQueue::__test_new`] / [`__test_push`] / [`__test_apply`] —
//   re-export the otherwise crate-private constructors and dispatch.
// * [`CommandQueue::__test_recovery_len`] / [`__test_bytes_len`] — observe
//   recovery / bytes state across calls (Bevy-mirror C2' lock-down).
// * [`CommandQueue::__test_inject_recovery`] — populate `panic_recovery`
//   with a command's byte image so the C2' "Step 0.5 deletion" regression
//   test can prove that a clean `apply` (no panic) leaves recovery
//   untouched.
//
// These helpers are NOT part of the stable API. They are `#[doc(hidden)]`
// so they do not appear in rustdoc and are clearly marked as test-only by
// the `__test_` prefix.

impl CommandQueue {
    /// Test-only constructor — exposes [`CommandQueue::new`] to integration
    /// tests. See module note above.
    #[doc(hidden)]
    pub fn __test_new() -> Self {
        Self::new()
    }

    /// Test-only push — exposes [`CommandQueue::push`] to integration tests.
    #[doc(hidden)]
    pub fn __test_push<C: Command>(&mut self, cmd: C) {
        self.push(cmd);
    }

    /// Test-only apply — exposes [`CommandQueue::apply`] to integration tests.
    #[doc(hidden)]
    pub fn __test_apply(&mut self, world: &mut EcsMaster) {
        self.apply(world);
    }

    /// Test-only observer — byte length of the live queue (bytes field).
    #[doc(hidden)]
    pub fn __test_bytes_len(&self) -> usize {
        self.bytes.len()
    }

    /// Test-only observer — byte length of `panic_recovery`.
    ///
    /// At rest (between top-level applies in single-level Phase 8d use)
    /// this is always 0 — the catch_unwind Err branch in
    /// [`RawCommandQueue::apply_or_drop_queued`] always re-absorbs recovery
    /// into `bytes` in the same panicking call (Bevy-mirror C2' semantic).
    /// Non-zero values are reachable only via the [`__test_inject_recovery`]
    /// helper, which exists specifically to lock down the "Step 0.5
    /// deletion" regression test.
    ///
    /// [`__test_inject_recovery`]: CommandQueue::__test_inject_recovery
    #[doc(hidden)]
    pub fn __test_recovery_len(&self) -> usize {
        self.panic_recovery.len()
    }

    /// Test-only injection: writes a `[CommandMeta][C payload]` slot into
    /// `panic_recovery` instead of `bytes`. Mirrors [`CommandQueue::push`]'s
    /// unaligned-write layout exactly — same `CommandMeta`, same
    /// `write_unaligned` discipline — but targets the recovery buffer so
    /// the C2' regression test can assert that a subsequent clean `apply`
    /// (Ok branch, no panic) does NOT touch the recovery buffer.
    ///
    /// Used by `tests/command_queue_panic_recovery.rs` only.
    #[doc(hidden)]
    pub fn __test_inject_recovery<C: Command>(&mut self, cmd: C) {
        let meta = CommandMeta {
            consume_and_drop: consume_and_drop_glue::<C> as ConsumeAndDropFn,
        };
        let cmd_size = mem::size_of::<C>();
        let total = COMMAND_PAYLOAD_OFFSET + cmd_size;
        let old_len = self.panic_recovery.len();
        self.panic_recovery.reserve(total);

        // SAFETY (CQ1, CQ2, CQ-PACK1): mirrors `push`'s safety contract,
        // but the destination is `panic_recovery` instead of `bytes`:
        //   - `panic_recovery.capacity() >= old_len + total` post-`reserve`.
        //   - `MaybeUninit<u8>` is byte-pattern-agnostic.
        //   - Both writes are `write_unaligned`; the byte slot has no
        //     required alignment and we never construct `&`/`&mut`
        //     references into the slot (CQ-PACK1).
        //   - `set_len(old_len + total)` makes the bytes reachable by a
        //     subsequent `Drop` walk if the test never calls `apply`.
        //   - `cmd: C` is bit-copied into the recovery buffer; the queue's
        //     bytes are the new logical owner.
        unsafe {
            let base = self.panic_recovery.as_mut_ptr().add(old_len);
            std::ptr::write_unaligned(base as *mut CommandMeta, meta);
            std::ptr::write_unaligned(base.add(COMMAND_PAYLOAD_OFFSET) as *mut C, cmd);
            self.panic_recovery.set_len(old_len + total);
        }
    }
}

impl Drop for CommandQueue {
    /// Drop-glue walk for any un-flushed commands (plan §10.5 + Phase
    /// 12.5 Opt-A1 §4.3).
    ///
    /// Invokes each pending command's `consume_and_drop_glue` with
    /// `world = None`, so the per-type `Drop` impl runs. `panic_recovery`
    /// is drained the same way.
    ///
    /// # I4 — two catch-wrapped walks
    ///
    /// Each walk runs inside its OWN `catch_unwind` (panics in command
    /// `Drop` impls during teardown are swallowed — propagating across
    /// Drop is itself UB). The two walks are independent: a panic during
    /// the `bytes` walk does not abort the subsequent `panic_recovery`
    /// walk.
    fn drop(&mut self) {
        if !self.bytes.is_empty() {
            let mut raw = self.raw();
            // SAFETY (CQ4 + drop-only path):
            //   - We hold exclusive `&mut self` via the `Drop` invocation
            //     contract.
            //   - `world = None` selects the drop-only path in
            //     `consume_and_drop_glue` (the cmd's Drop runs in-place;
            //     no world access).
            let walk = AssertUnwindSafe(|| unsafe {
                raw.apply_or_drop_queued_no_catch(None);
            });
            let _ = std::panic::catch_unwind(walk);
        }
        if !self.panic_recovery.is_empty() {
            // Move recovery into bytes so the walk machinery can iterate
            // it; `apply_or_drop_queued_no_catch` reads from `self.bytes`
            // only.
            let mut recovery = mem::take(&mut self.panic_recovery);
            self.bytes.append(&mut recovery);
            let mut raw = self.raw();
            // SAFETY: same as above — drop-only path under exclusive
            //   `&mut self`.
            let walk = AssertUnwindSafe(|| unsafe {
                raw.apply_or_drop_queued_no_catch(None);
            });
            let _ = std::panic::catch_unwind(walk);
        }
    }
}

// =============================================================================
// Smoke tests — Phase 8d Step 6 (slots 560..=565 per the task spec)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Test command: increments a static atomic counter via `apply` and
    /// another via `Drop`. Verifies apply / drop paths independently.
    static APPLY_COUNTER: AtomicUsize = AtomicUsize::new(0);
    static DROP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct CounterCommand {
        delta: usize,
    }

    impl Command for CounterCommand {
        fn apply(self, _world: &mut EcsMaster) {
            APPLY_COUNTER.fetch_add(self.delta, Ordering::Relaxed);
            // `self` falls out of scope here; its Drop runs next.
        }
    }

    impl Drop for CounterCommand {
        fn drop(&mut self) {
            DROP_COUNTER.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Resets the counters between tests. Each test reads-then-resets so
    /// the global state does not leak across tests, but parallel test
    /// execution could interleave; the test runner is typically
    /// `--test-threads=1`-safe enough for this module. We hedge by
    /// reading deltas inside each test rather than asserting absolute
    /// values across the suite.
    fn reset_counters() -> (usize, usize) {
        let a = APPLY_COUNTER.swap(0, Ordering::Relaxed);
        let d = DROP_COUNTER.swap(0, Ordering::Relaxed);
        (a, d)
    }

    #[test]
    fn empty_apply_is_noop() {
        let _ = reset_counters();
        let mut q = CommandQueue::new();
        let mut world = EcsMaster::new();
        q.apply(&mut world);
        let (a, d) = reset_counters();
        assert_eq!(a, 0, "no commands ⇒ no apply");
        assert_eq!(d, 0, "no commands ⇒ no drop");
    }

    #[test]
    fn push_then_apply_runs_command() {
        let _ = reset_counters();
        let mut q = CommandQueue::new();
        q.push(CounterCommand { delta: 7 });
        let mut world = EcsMaster::new();
        q.apply(&mut world);
        let (a, d) = reset_counters();
        assert_eq!(a, 7, "apply ran once with delta=7");
        assert_eq!(d, 1, "drop ran once after apply");
        // Bytes should be drained to len=0 post-apply.
        assert_eq!(q.bytes.len(), 0, "bytes drained after apply");
        assert_eq!(q.cursor, 0, "cursor reset to start after apply");
    }

    #[test]
    fn push_many_then_apply_runs_each_command_once() {
        let _ = reset_counters();
        let mut q = CommandQueue::new();
        for i in 1..=10 {
            q.push(CounterCommand { delta: i });
        }
        let mut world = EcsMaster::new();
        q.apply(&mut world);
        let (a, d) = reset_counters();
        // 1 + 2 + ... + 10 = 55.
        assert_eq!(a, 55, "sum of deltas applied");
        assert_eq!(d, 10, "each command dropped exactly once");
        assert_eq!(q.bytes.len(), 0);
    }

    #[test]
    fn drop_runs_drop_glue_on_unapplied_commands() {
        let _ = reset_counters();
        {
            let mut q = CommandQueue::new();
            q.push(CounterCommand { delta: 42 });
            q.push(CounterCommand { delta: 100 });
            // Don't call apply — let the queue's Drop handle it.
        }
        let (a, d) = reset_counters();
        assert_eq!(a, 0, "drop-only path does NOT apply");
        assert_eq!(d, 2, "drop ran once per unapplied command");
    }

    #[test]
    fn capacity_not_shrunk_after_apply() {
        let _ = reset_counters();
        let mut q = CommandQueue::new();
        for i in 1..=50 {
            q.push(CounterCommand { delta: i });
        }
        let peak_capacity = q.bytes.capacity();
        let mut world = EcsMaster::new();
        q.apply(&mut world);
        let (_, _) = reset_counters();
        // bytes is drained (len=0) but capacity is retained (W3 policy).
        assert_eq!(q.bytes.len(), 0);
        assert_eq!(
            q.bytes.capacity(),
            peak_capacity,
            "W3: capacity retained across apply for reuse"
        );
    }

    #[test]
    fn push_preserves_layout_offset_constants() {
        // Sanity: COMMAND_PAYLOAD_OFFSET must equal CommandMeta size.
        assert_eq!(
            COMMAND_PAYLOAD_OFFSET,
            mem::size_of::<CommandMeta>(),
            "payload offset must equal meta size"
        );
        // For a zero-sized command, the slot is exactly the meta header.
        struct ZeroCmd;
        impl Command for ZeroCmd {
            fn apply(self, _world: &mut EcsMaster) {}
        }
        let mut q = CommandQueue::new();
        let before = q.bytes.len();
        q.push(ZeroCmd);
        let after = q.bytes.len();
        assert_eq!(
            after - before,
            COMMAND_PAYLOAD_OFFSET + mem::size_of::<ZeroCmd>(),
            "push must write meta + payload contiguously",
        );
    }
}
