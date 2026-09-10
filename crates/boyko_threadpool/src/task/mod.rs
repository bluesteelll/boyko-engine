//! The pool's queue element — [`Task`] — and the heap cells it points at.
//!
//! # The shape, and why it is this one
//!
//! A task is a raw pointer plus a MONOMORPHIZED function pointer, both held in
//! the element (rayon's `JobRef`, verbatim):
//!
//! ```text
//! Task { payload: *const (), execute: unsafe fn(*const ()) }      // 16 B
//! ```
//!
//! It replaces `TaskHandle { body: Box<dyn FnOnce() + Send + 'static> }` at the
//! same 16 bytes and one fewer dependent load: a call through a trait object is
//! `element -> vtable -> slot -> target`, a call through this one is
//! `element -> target`. Every queue moves the same number of bytes as before,
//! so `steal_batch`'s `read_volatile`/`write_volatile` traffic is unchanged.
//!
//! # THE FILE LAYOUT, which is a visibility decision
//!
//! Three files, and the split is what keeps D3 (`block.rs`'s header) a
//! type-system fact rather than a convention:
//!
//! * `mod.rs` — [`Task`], its two constructors, its `Drop`, and the shared
//!   [`CellHead`] prefix. Nothing here knows a cell's body type.
//! * `scoped.rs` — `ScopedCell`, `run_scoped`, `drop_unrun_scoped`.
//! * `detached.rs` — `DetachedCell`, `run_detached`, `drop_unrun_detached`,
//!   and `alloc_cell`, whose only remaining caller is
//!   [`Task::new_detached`].
//!
//! Both cell types are `pub(in crate::task)` — spelled `pub(super)` at their
//! declarations, which is the same bound from one level down: `mod.rs` names
//! them because its constructors build them, and NOTHING outside this module
//! can. Not `pub(crate)`, deliberately. That bound is
//! the whole of D3 — an erased `*const ()` out of a `ScopeBlock` is inert
//! until someone casts it back to a cell type, and widening either type's
//! visibility is a type-system-visible edit rather than a style change.
//!
//! # THE LEDGER, and it is not a pure win
//!
//! MEASURED on release, non-test objects (`rustc -O -C opt-level=3 --emit asm`,
//! instructions counted per symbol), by a method whose positive control — the
//! same symbol plus one `asm!` `nop` — resolves exactly one instruction:
//!
//! * DISPATCH is instruction-NEUTRAL. The removed vtable dereference was
//!   already folded into the call's memory operand (`callq *24(%rdx)`, whose
//!   target is loaded from a SECOND object, the closure's vtable on its own
//!   `.rdata` line); the new call is a tail `jmpq *%rax` off the element's own
//!   second word. What this removes is one LEVEL of the call's dependency
//!   chain, not an instruction.
//! * SPAWN costs +3 instructions and +8 payload bytes per task: 22 -> 25
//!   instructions, cell 16 -> 24 B for an 8-byte body. The extra store is the
//!   `drop_unrun` thunk, which buys back the teardown the `Box` glue gave for
//!   free (see the teardown clause below).
//! * The `dealloc` did not vanish, it RELOCATED: `Box<dyn FnOnce>::call_once`
//!   freed the box AFTER the body, `run_detached` frees the cell BEFORE it.
//!   Same count, same thread, no instruction delta — only the block size and
//!   the timing changed. This bullet is Stage 1's ledger and its SCOPED half is
//!   superseded: Stage 3b took the `dealloc` out of `run_scoped` altogether,
//!   so for a scoped task there is no per-task free left to relocate (see the
//!   allocation clause below).
//!
//! So the honest sentence is: Stage 1 pays 3 instructions and 8 bytes at spawn
//! to remove one dependency level at dispatch. The point of the change is
//! neither of those numbers — it is the obligation below.
//!
//! # THE REASON THIS TYPE EXISTS, which is not the load
//!
//! Tree Borrows installs a *protector* on every reference-typed value that
//! arrives as a function ARGUMENT — including references nested inside a
//! by-value aggregate, which is exactly how a closure's captured `&T` gets one —
//! and that protector is live for the callee's WHOLE ACTIVATION, to the closing
//! brace, whether or not a byte is touched. Hence the obligation:
//!
//! > at the instant the release RMW commits (`ScopeShared::complete_task`'s
//! > `pending.fetch_sub`, which is what authorises `Scope::drop` to free), NO
//! > thread may hold a live activation of any function that received, as an
//! > argument, a reference into memory that the reclamation reclaims.
//!
//! The `Box<dyn FnOnce()>` shape violated it STRUCTURALLY. `Scope::prepare`
//! built `move || { catch_unwind(AssertUnwindSafe(f)); ...; complete_task(shared) }`
//! and boxed it, so the user's body `f` — and every `&T` inside it — was a
//! by-value argument of the SAME `call_once` activation that published "you may
//! free". Nothing about the wrapper's ordering could fix that: the protector
//! expires at the wrapper's closing brace, and the release is before it.
//!
//! Here the body is instead READ OUT of the cell by value and passed to a
//! SIBLING activation ([`catch_unwind`](std::panic::catch_unwind)) that has
//! RETURNED before the activation performing the release is entered. That is
//! the same substitution Phase 9.2 made one level up
//! (`join_workers_until_drained(inner, shared: *const ScopeShared)`) and
//! 2026-09-05 made one level down (`ScopeShared::complete_task(shared: *const
//! Self)`), and the same one rayon documents on `unsafe fn set(this: *const
//! Self)` — "This function operates on `*const Self` instead of `&self` to
//! allow it to become dangling during this call."
//!
//! It is unwritable-around rather than upheld by care: the only call target the
//! queues can hold is `unsafe fn(*const ())`, whose entire argument list is POD,
//! so there is no reference-shaped parameter through which a body's environment
//! could reach the releasing frame.
//!
//! # Allocation, and what happens to a task that never runs
//!
//! The two kinds stopped paying alike in Stage 3b, and only the DETACHED one
//! still looks like the `Box` it replaced:
//!
//! * DETACHED (`ThreadPool::spawn`) — one `alloc` per spawn and one `dealloc`
//!   per task, on the same threads as the `Box`: the spawner allocates
//!   (`alloc_cell`), the thread that runs the task frees, and inside the run
//!   the cell is freed as soon as the body has been read out of it, before the
//!   body is invoked.
//! * SCOPED (`Scope::spawn`) — no per-task allocation at all. The cell is
//!   emplaced in the scope's own `ScopeBlock` ([`crate::block`]) and reclaimed
//!   with every other cell of that scope by ONE `free_all` in `Scope::drop`,
//!   the first thing after the join returns. One `alloc` per chunk replaces one
//!   per task; the peak payment moves from tasks IN FLIGHT to tasks SPAWNED.
//!
//! ## THE CLAUSE THE SCOPED PATH LOST, named as a downgrade
//!
//! Until Stage 3b this section read: the cell "is freed as soon as the body has
//! been read out of it, BEFORE the body is invoked, so by the time the release
//! RMW commits the payload allocation does not exist at all — which is stronger
//! than 'no protector covers it'". For the scoped kind that is now FALSE, and
//! not by a little: the cell necessarily OUTLIVES the release RMW, because the
//! reclamation that frees it is `Scope::drop`'s and that RMW is what authorises
//! `Scope::drop` to proceed.
//!
//! The replacement is weaker in FORM, which is why it is written as a loss
//! rather than reworded into a win. "The allocation does not exist" needs
//! nothing from the code that runs afterwards and survives any future
//! tightening of Tree Borrows; what stands in its place is the `ScopeShared`
//! clause, verbatim — the one `complete_task` has always rested on:
//!
//! > no protector covers the cell, because `run_scoped` holds only
//! > `*const ScopedCell<F>` and forms no reference into it — `shared` is a
//! > place read and `body` leaves by `ptr::read` through `addr_of!`.
//!
//! So the cell's argument is now the same SHAPE as the scope allocation's, and
//! it is decided the same way: by a reduction that leaves exactly one function
//! able to hold a protector across the release (`run_scoped`'s SAFETY block)
//! plus an execution gate over that one function (KE16 M2w).
//!
//! A task that is DROPPED WITHOUT RUNNING (a queue torn down with work still in
//! it) must still run the user's destructors; the boxed closure got that from
//! its drop glue. It is restored here by [`Task`]'s own `Drop`, which calls the
//! cell's `drop_unrun` thunk — one fn pointer stored at offset 0 of every cell,
//! read only on that cold path, so the hot path keeps its single indirection.
//! The thunk frees a DETACHED cell and does not free a SCOPED one: an unrun
//! scoped task is still counted in `pending`, so its scope's join has not
//! returned, so `free_all` has not run and the chunk it sits in is live.

use core::mem::ManuallyDrop;
use core::ptr;

use crate::block::ScopeBlock;
use crate::scope::ScopeShared;

mod detached;
mod scoped;

use self::detached::{DetachedCell, alloc_cell, drop_unrun_detached, run_detached};
// `run_scoped` is NOT imported by name: `new_scoped` picks its run function
// between two module-qualified paths, one per side of the `tb-neg-m2w` cfg, so
// an import of either name would be unused in one configuration. The
// qualification is unconditional and identical in both, which is what keeps the
// cfg difference below the ONLY one the negative arm makes.
use self::scoped::{ScopedCell, drop_unrun_scoped};

/// The ONE call shape the pool's queues can hold. Entirely POD, which is what
/// makes the protector obligation above impossible to violate through it.
type TaskFn = unsafe fn(*const ());

/// The prefix EVERY payload cell begins with.
///
/// `#[repr(C)]` on the cells plus this being their first field is what lets
/// [`Task::drop`] read the thunk without knowing the body type: the cells share
/// a common initial sequence, so the load at offset 0 is well defined for all of
/// them.
#[repr(C)]
struct CellHead {
    /// Runs iff the element is dropped WITHOUT running: drops the body in
    /// place, and frees the cell if the cell is its own allocation. Never
    /// called on a task that ran.
    drop_unrun: TaskFn,
}

/// One unit of work as the pool's queues carry it: a payload address and the
/// monomorphized function that consumes it.
///
/// `!Copy` and `Drop` deliberately — see the module header's teardown clause.
#[repr(C)]
pub(crate) struct Task {
    /// Address of a `ScopedCell<F>` / `DetachedCell<F>` whose body type is
    /// erased. Non-null; the type is `*const ()` rather than `NonNull` because
    /// `execute` already carries the niche `Option<Task>` needs.
    payload: *const (),
    /// The monomorphized run function for THIS payload's body type. Also the
    /// discriminant: there is no tag and no branch on dispatch.
    execute: TaskFn,
}

// The element must not grow: `Injector`/`Worker`/`Stealer` move it by
// `read_volatile`/`write_volatile`, and `steal_batch` moves up to 32 of them per
// steal. `TaskHandle`'s `Box<dyn FnOnce>` was a 16-byte fat pointer.
const _: () = assert!(size_of::<Task>() == 16);
const _: () = assert!(align_of::<Task>() == 8);
// `pop`/`steal` return `Option<Task>` by value on every poll of every source.
// The fn pointer is non-null, so the niche keeps that at 16 bytes too — the size
// `Option<TaskHandle>` had.
const _: () = assert!(size_of::<Option<Task>>() == 16);

// SAFETY:
//   `payload` addresses a cell whose body type is `Send` — both constructors
//   below require `F: Send`, and they are the only way to mint a `Task`. The
//   cell is written by the spawner BEFORE the push that publishes the element
//   and is never written again by anyone, so the only cross-thread hand-off is
//   the value itself; crossbeam's push is `Release` and its steal `Acquire`, so
//   the thread that consumes the element sees the fully written cell. `execute`
//   is a `'static` function pointer.
//
//   `Task` is deliberately NOT `Sync` and does not need to be: a queue element
//   is owned by exactly one thread at a time — moved into a queue, moved out of
//   it by one consumer, and consumed there.
//
//   NOT covered by this clause, and covered instead by `Scope::drop`: a
//   `ScopedCell<F>`'s `F` may borrow for `'scope` rather than `'static`, and
//   erasing it into `*const ()` erases that. See `Scope::prepare` for the
//   blocking-join argument that makes the erasure sound. The same argument is
//   what keeps the CELL alive for the whole run now that it lives in the
//   scope's block rather than in its own allocation.
unsafe impl Send for Task {}

impl Task {
    /// Build the task for a body spawned into a scope.
    ///
    /// `shared` is the scope's `ScopeShared` address, held BY VALUE in the cell:
    /// a stored `&ScopeShared` would be the material a call turns into a
    /// protector, which is the whole defect this representation removes.
    ///
    /// `F` is NOT bounded by `'static`. The lifetime disappears in the coercion
    /// of `run_scoped::<F>` to a `TaskFn` — that coercion IS the erasure, in
    /// place of the `mem::transmute` of a trait object it replaces — and the
    /// obligation it creates is this function's second safety clause.
    ///
    /// The cell is emplaced in `block` rather than allocated, and the payload
    /// word is `BlockPtr::erase`'s `*const ()`: the typed handle is consumed by
    /// the erasure, so nothing that outlives this call can name the cell as a
    /// `ScopedCell<F>` except the run/teardown thunks the element carries.
    ///
    /// # Safety
    ///
    /// * `shared` must point at a live `ScopeShared` whose `pending` ALREADY
    ///   counts this task, and the registration must not be released by anyone
    ///   else: the returned element's run function is what completes it, exactly
    ///   once. That is what keeps the allocation alive for the whole run — the
    ///   single free site (`Scope::drop`) frees only after a join that observed
    ///   `pending == 0`.
    /// * `block` must be the `ScopeBlock` of the scope `shared` names, and it
    ///   must not be `free_all`ed until this task has either run or been
    ///   dropped. The cell lives in `block`'s chunks, so `free_all` invalidates
    ///   it — and the clause above is what upholds this one, because
    ///   `free_all`'s single call site is `Scope::drop`, after the same join.
    ///   A task that is dropped unrun does NOT release its registration
    ///   (see [`Task::drop`]), so it cannot outlive that join either.
    /// * `body` may borrow for a lifetime shorter than `'static`, and this
    ///   erases it. The caller must guarantee that every borrow inside `body`
    ///   outlives the task's EXECUTION, not merely its push.
    pub(crate) unsafe fn new_scoped<F>(
        block: &ScopeBlock,
        shared: *const ScopeShared,
        body: F,
    ) -> Self
    where
        F: FnOnce() + Send,
    {
        // `emplace` allocates AND initialises through a `*mut T`, so no
        // reference into chunk memory is ever formed here — D2 in `block.rs`'s
        // header, and the reason this constructor needs no `unsafe` block of
        // its own where the allocating one did.
        let cell = block.emplace(ScopedCell {
            head: CellHead {
                drop_unrun: drop_unrun_scoped::<F>,
            },
            shared,
            body,
        });
        // THE ONE TEXTUAL DIFFERENCE THE KE16 M2w NEGATIVE ARM MAKES TO THIS
        // PATH, and it has to stay the only one: the arm's claim is that a UB
        // report is attributable to ONE protector over chunk memory
        // (`scoped::run_scoped_neg`'s reference argument), and an arm that
        // also changed the cell, the read-out or the completion would report
        // UB without saying which change earned it. `tb-neg-m2w` is refused
        // outside `cfg(miri)` in `lib.rs`, so the deliberate-UB value cannot
        // reach a native artifact.
        #[cfg(not(feature = "tb-neg-m2w"))]
        let execute: TaskFn = scoped::run_scoped::<F>;
        #[cfg(feature = "tb-neg-m2w")]
        let execute: TaskFn = scoped::run_scoped_neg::<F>;

        Self {
            payload: cell.erase(),
            execute,
        }
    }

    /// Build the task for a fire-and-forget body (`ThreadPool::spawn`).
    pub(crate) fn new_detached<F>(body: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let cell = alloc_cell::<DetachedCell<F>>();
        // SAFETY: `alloc_cell` returned a fresh, uniquely owned allocation of
        //   exactly `size_of::<DetachedCell<F>>()` bytes at its alignment, and
        //   nothing has read or written it. `ptr::write` takes `*mut T` by
        //   value, so it forms no reference and initialises the whole cell in
        //   one store; `body` is moved in and is not dropped here.
        unsafe {
            ptr::write(
                cell,
                DetachedCell {
                    head: CellHead {
                        drop_unrun: drop_unrun_detached::<F>,
                    },
                    body,
                },
            );
        }
        Self {
            payload: cell.cast::<()>(),
            execute: run_detached::<F>,
        }
    }

    /// Consume the element and run its body.
    ///
    /// Reached only through [`worker::run_task`](crate::worker::run_task),
    /// which wraps it in the abort backstop.
    #[inline]
    pub(crate) fn run(self) {
        // The payload is consumed by `execute`, so the teardown thunk must NOT
        // also fire — it would drop a body that has been moved out, and free a
        // detached cell twice — including on the unwinding path out of a
        // fire-and-forget body.
        let this = ManuallyDrop::new(self);
        // SAFETY: `payload` is the address `new_scoped`/`new_detached` minted
        //   and `execute` is the run function monomorphized for THAT cell's body
        //   type in the same constructor, so the pairing is established at the
        //   only sites that can mint a `Task` and cannot be mismatched. A `Task`
        //   is moved, never copied — it is `!Copy`, crossbeam moves elements
        //   bytewise, and the queues hand each element to exactly one consumer —
        //   so this runs at most once for the allocation.
        unsafe { (this.execute)(this.payload) };
    }
}

impl Drop for Task {
    /// Teardown of a task that never ran: drop the body, and free the cell if
    /// the cell owns its own allocation.
    ///
    /// This is what restores the `Box<dyn FnOnce>` drop glue the old element
    /// got for free. It fires when a queue holding work is dropped — a pool torn
    /// down with fire-and-forget tasks still enqueued — so a body holding an
    /// `Arc` clone, a file handle or a channel sender still releases it.
    ///
    /// Like that drop glue, it does NOT complete a scope registration: a
    /// SCOPED task reaching this path would leave its scope's join waiting
    /// forever, exactly as before this change. That state is already a contract
    /// violation caught by `ThreadPool::drop`'s `active_scopes == 0`
    /// debug-assert, and making it louder is a separate decision. It is also
    /// what makes the scoped thunk's non-free correct rather than a leak: the
    /// registration this path leaves outstanding is what keeps the scope's
    /// `free_all` from having run.
    ///
    /// `#[cold]`, not `#[inline]`: a task that RAN never reaches here
    /// ([`Task::run`] forgets the element), so every call is pool teardown with
    /// work still enqueued. Inlining the indirect call and its thunk into
    /// crossbeam's drop glue would buy nothing and cost L1i.
    #[cold]
    fn drop(&mut self) {
        // SAFETY: `payload` addresses a live cell (`run` takes the element by
        //   value and forgets it, so a dropped element is one that never ran).
        //   Every cell type begins with a `CellHead` at offset 0 under
        //   `#[repr(C)]`, so this reads a valid `TaskFn`; it is the thunk
        //   monomorphized for this cell's own body type by the constructor that
        //   wrote it.
        unsafe {
            let head = ptr::read(self.payload.cast::<CellHead>());
            (head.drop_unrun)(self.payload);
        }
    }
}

#[cfg(test)]
mod teardown_support {
    //! Shared apparatus for the teardown coverage of the two `drop_unrun`
    //! thunks (KE16 finding W3), which lives in three modules — here, in
    //! `scoped::tests` and in `detached::tests` — because each thunk's tests
    //! belong beside the thunk.
    //!
    //! These live in `src/` rather than in `tests/` because [`super::Task`] is
    //! `pub(crate)`: an integration test cannot mint one, and a thunk/layout
    //! mismatch here is heap corruption that no pool-level test can see.
    //!
    //! The "still frees" half of the obligation is NOT asserted by a counter
    //! here — a counting global allocator would be process-global and these
    //! tests run in parallel with the crate's other unit tests. It is decided
    //! by running these modules under Miri with leak checking ON (i.e.
    //! `MIRIFLAGS` WITHOUT `-Zmiri-ignore-leaks`), where a cell that is dropped
    //! but not freed is a reported leak and a cell freed twice is reported UB.
    //! Since Stage 3b that recipe also decides the SCOPED arm's new obligation:
    //! its cell is block memory, so the test must free the block itself and a
    //! missing `free_all` is a reported leak.

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A body payload whose destructor is observable.
    ///
    /// Holds an `Arc` as well as the counter so the tests can check the two
    /// things a teardown owes separately: that `drop` RAN (the counter) and
    /// that what the body OWNED was released (`Arc::strong_count`).
    pub(super) struct Witness {
        pub(super) drops: Arc<AtomicUsize>,
        // Never read BY DESIGN, and that is what it measures: the test observes it
        // from outside through `Arc::strong_count`, so a teardown that ran `drop`
        // without releasing what the body OWNED still fails. Reading it here would
        // defeat the point — the field's whole job is to be dropped, not consulted.
        #[allow(dead_code)]
        pub(super) held: Arc<u32>,
    }

    impl Drop for Witness {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::AcqRel);
        }
    }
}

#[cfg(test)]
mod tests {
    //! What the ELEMENT owes, as opposed to what either thunk owes: that
    //! `Task::drop` fires for every element left in a torn-down queue, exactly
    //! once per body across the ran/unrun split, and that the blind offset-0
    //! load it does is well defined for both cell types.
    //!
    //! The per-thunk teardown tests are in `scoped::tests` / `detached::tests`;
    //! `teardown_support`'s header carries the Miri recipe all three are
    //! decided under.

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crossbeam_deque::{Injector, Steal, Worker};

    use super::teardown_support::Witness;
    use super::*;

    #[test]
    fn dropping_an_injector_of_unrun_tasks_drops_every_body() {
        const N: usize = 64;
        let drops = Arc::new(AtomicUsize::new(0));
        let held = Arc::new(7u32);

        let injector: Injector<Task> = Injector::new();
        for _ in 0..N {
            let w = Witness {
                drops: Arc::clone(&drops),
                held: Arc::clone(&held),
            };
            injector.push(Task::new_detached(move || {
                let _ = &w;
            }));
        }
        assert_eq!(
            Arc::strong_count(&held),
            N + 1,
            "every queued body holds its own clone"
        );

        drop(injector);

        assert_eq!(
            drops.load(Ordering::Acquire),
            N,
            "tearing down a queue that still holds work must run every body's destructor"
        );
        assert_eq!(
            Arc::strong_count(&held),
            1,
            "every queued body's Arc clone must be released"
        );
    }

    #[test]
    fn dropping_a_worker_deque_of_unrun_tasks_drops_every_body() {
        const N: usize = 64;
        let drops = Arc::new(AtomicUsize::new(0));

        let deque: Worker<Task> = Worker::new_lifo();
        for _ in 0..N {
            let w = Witness {
                drops: Arc::clone(&drops),
                held: Arc::new(0),
            };
            deque.push(Task::new_detached(move || {
                let _ = &w;
            }));
        }

        drop(deque);

        assert_eq!(
            drops.load(Ordering::Acquire),
            N,
            "a per-worker deque torn down with work still in it must run every destructor"
        );
    }

    #[test]
    fn a_partly_drained_queue_drops_exactly_the_bodies_left_in_it() {
        const N: usize = 8;
        const DRAINED: usize = 3;
        let drops = Arc::new(AtomicUsize::new(0));
        let ran = Arc::new(AtomicUsize::new(0));

        let injector: Injector<Task> = Injector::new();
        for _ in 0..N {
            let w = Witness {
                drops: Arc::clone(&drops),
                held: Arc::new(0),
            };
            let ran_in = Arc::clone(&ran);
            injector.push(Task::new_detached(move || {
                let _ = &w;
                ran_in.fetch_add(1, Ordering::AcqRel);
            }));
        }

        let mut popped = 0usize;
        while popped < DRAINED {
            if let Steal::Success(t) = injector.steal() {
                t.run();
                popped += 1;
            }
        }

        assert_eq!(ran.load(Ordering::Acquire), DRAINED);
        assert_eq!(
            drops.load(Ordering::Acquire),
            DRAINED,
            "the bodies that ran dropped once each, through `run`"
        );

        drop(injector);

        assert_eq!(
            drops.load(Ordering::Acquire),
            N,
            "every body is dropped exactly once across the two paths - no double drop on \
             the drained ones, no leak on the remaining ones"
        );
    }

    #[test]
    fn a_zero_sized_body_still_round_trips_through_the_teardown_thunk() {
        // The degenerate cell: `DetachedCell<F>` is then exactly the `CellHead`,
        // which is what keeps `alloc_cell`'s layout non-zero. The thunk must
        // still free it.
        let task = Task::new_detached(|| {});
        assert_eq!(
            size_of::<Task>(),
            16,
            "the element does not grow for a ZST body"
        );
        drop(task);
    }

    #[test]
    fn the_cell_head_sits_at_offset_zero_in_every_cell() {
        // `Task::drop` reads `drop_unrun` from offset 0 of a `*const ()` whose
        // body type it does not know, and NOTHING pinned that offset: the two
        // size pins and the alignment pin over each cell all survive a reorder
        // to `{ body, head, shared }`, which under `#[repr(C)]` moves the thunk
        // to a non-zero offset and turns that blind load into a read of the
        // body's first word.
        //
        // Cheap to state, and it has to be stated HERE — `ScopedCell` /
        // `DetachedCell` / `CellHead` are `pub(in crate::task)` at most, so no
        // module outside `task` can name them, and this is the one module
        // inside it that can name BOTH cells.
        assert_eq!(
            core::mem::offset_of!(CellHead, drop_unrun),
            0,
            "the thunk is the first field of the shared prefix"
        );
        assert_eq!(
            core::mem::offset_of!(ScopedCell<()>, head),
            0,
            "a scoped cell begins with its head, which is what makes Task::drop's blind \
             offset load well defined"
        );
        assert_eq!(
            core::mem::offset_of!(ScopedCell<[u8; 64]>, head),
            0,
            "and it does so for a body of any size — the offset is not an accident of a ZST"
        );
        assert_eq!(
            core::mem::offset_of!(DetachedCell<()>, head),
            0,
            "a detached cell begins with its head, for the same reason"
        );
        assert_eq!(
            core::mem::offset_of!(DetachedCell<[u8; 64]>, head),
            0,
            "and for a body of any size"
        );
    }
}
