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
//!   freed the box AFTER the body, `run_scoped` frees the cell BEFORE it. Same
//!   count, same thread, no instruction delta — only the block size and the
//!   timing changed.
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
//! SIBLING activation ([`catch_unwind`]) that has RETURNED before the activation
//! performing the release is entered. That is the same substitution Phase 9.2
//! made one level up (`join_workers_until_drained(inner, shared: *const
//! ScopeShared)`) and 2026-09-05 made one level down
//! (`ScopeShared::complete_task(shared: *const Self)`), and the same one rayon
//! documents on `unsafe fn set(this: *const Self)` — "This function operates on
//! `*const Self` instead of `&self` to allow it to become dangling during this
//! call."
//!
//! It is unwritable-around rather than upheld by care: the only call target the
//! queues can hold is `unsafe fn(*const ())`, whose entire argument list is POD,
//! so there is no reference-shaped parameter through which a body's environment
//! could reach the releasing frame.
//!
//! # Allocation, and what happens to a task that never runs
//!
//! One `alloc` per spawn and one `dealloc` per task, on the same threads as the
//! `Box` they replace: the spawner allocates, the thread that runs the task
//! frees. What moved is WHERE inside the run: the cell is freed as soon as the
//! body has been read out of it, BEFORE the body is invoked, so by the time the
//! release RMW commits the payload allocation does not exist at all — which is
//! stronger than "no protector covers it".
//!
//! A task that is DROPPED WITHOUT RUNNING (a queue torn down with work still in
//! it) must still run the user's destructors and still free its cell; the boxed
//! closure got that from its drop glue. It is restored here by [`Task`]'s own
//! `Drop`, which calls the cell's `drop_unrun` thunk — one fn pointer stored at
//! offset 0 of every cell, read only on that cold path, so the hot path keeps
//! its single indirection.

use core::alloc::Layout;
use core::mem::ManuallyDrop;
use core::ptr;
use std::alloc::{alloc, dealloc, handle_alloc_error};
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::scope::ScopeShared;

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
    /// place and frees the cell. Never called on a task that ran.
    drop_unrun: TaskFn,
}

/// Payload of a task spawned into a [`Scope`](crate::Scope).
#[repr(C)]
struct ScopedCell<F> {
    head: CellHead,
    /// The scope's shared state, as an ADDRESS. Copied out by value before the
    /// body runs; never dereferenced through a reference stored anywhere.
    shared: *const ScopeShared,
    body: F,
}

// Pins `__layout_receipt::SCOPED_CELL_HEADER`, the number the allocation
// receipts predict a chunk count from. TWO asserts rather than one, and the
// second is not decoration: the first alone would still hold if `body` were
// deleted, and it is the second that says the body is IN the cell at its own
// size. Drift on either side is a build failure.
const _: () = assert!(size_of::<ScopedCell<()>>() == crate::__layout_receipt::SCOPED_CELL_HEADER);
const _: () =
    assert!(size_of::<ScopedCell<[u8; 64]>>() == crate::__layout_receipt::SCOPED_CELL_HEADER + 64);

// And a THIRD pin, on the alignment, which neither size pin implies. A receipt
// buckets an allocation on the PAIR `(size, align)`; pinning size alone leaves
// the other half of the discriminant free to move, and a cell class that moved
// would leave the receipt counting over an EMPTY bucket — vacuously green.
// `#[repr(C, align(16))]` on the type above is the concrete edit that does it:
// both sizes stay 16 and 80, and the real allocations move to align 16.
const _: () = assert!(align_of::<ScopedCell<()>>() == crate::__layout_receipt::SCOPED_CELL_ALIGN);

/// Payload of a fire-and-forget task (`ThreadPool::spawn`). No scope, no
/// completion accounting, no panic capture — an unwind out of the body reaches
/// [`worker::run_task`](crate::worker::run_task) and aborts.
#[repr(C)]
struct DetachedCell<F> {
    head: CellHead,
    body: F,
}

// Pins `__layout_receipt::DETACHED_CELL_HEADER`, doubly, for the reason stated
// over `ScopedCell` above: this cell's header is the thunk alone, and a single
// assert would not notice a cell that had stopped carrying its body.
const _: () =
    assert!(size_of::<DetachedCell<()>>() == crate::__layout_receipt::DETACHED_CELL_HEADER);
const _: () = assert!(
    size_of::<DetachedCell<[u8; 64]>>() == crate::__layout_receipt::DETACHED_CELL_HEADER + 64
);
// The alignment half of this cell class's bucket, for the reason stated over
// `ScopedCell` above.
const _: () =
    assert!(align_of::<DetachedCell<()>>() == crate::__layout_receipt::DETACHED_CELL_ALIGN);

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
//   blocking-join argument that makes the erasure sound.
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
    /// # Safety
    ///
    /// * `shared` must point at a live `ScopeShared` whose `pending` ALREADY
    ///   counts this task, and the registration must not be released by anyone
    ///   else: the returned element's run function is what completes it, exactly
    ///   once. That is what keeps the allocation alive for the whole run — the
    ///   single free site (`Scope::drop`) frees only after a join that observed
    ///   `pending == 0`.
    /// * `body` may borrow for a lifetime shorter than `'static`, and this
    ///   erases it. The caller must guarantee that every borrow inside `body`
    ///   outlives the task's EXECUTION, not merely its push.
    pub(crate) unsafe fn new_scoped<F>(shared: *const ScopeShared, body: F) -> Self
    where
        F: FnOnce() + Send,
    {
        let cell = alloc_cell::<ScopedCell<F>>();
        // SAFETY: `alloc_cell` returned a fresh, uniquely owned allocation of
        //   exactly `size_of::<ScopedCell<F>>()` bytes at its alignment, and
        //   nothing has read or written it. `ptr::write` takes `*mut T` by
        //   value, so it forms no reference and initialises the whole cell in
        //   one store; `body` is moved in and is not dropped here.
        unsafe {
            ptr::write(
                cell,
                ScopedCell {
                    head: CellHead {
                        drop_unrun: drop_unrun_scoped::<F>,
                    },
                    shared,
                    body,
                },
            );
        }
        Self {
            payload: cell.cast::<()>(),
            execute: run_scoped::<F>,
        }
    }

    /// Build the task for a fire-and-forget body (`ThreadPool::spawn`).
    pub(crate) fn new_detached<F>(body: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let cell = alloc_cell::<DetachedCell<F>>();
        // SAFETY: as in `new_scoped` — a fresh, uniquely owned, correctly sized
        //   and aligned allocation, initialised in one `ptr::write` through a
        //   raw pointer.
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
        // The payload is consumed by `execute` (which frees the cell), so the
        // teardown thunk must NOT also fire — including on the unwinding path
        // out of a fire-and-forget body.
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
    /// Teardown of a task that never ran: drop the body, free the cell.
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
    /// debug-assert, and making it louder is a separate decision.
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

/// Allocate one uninitialised cell.
///
/// Never a zero-sized layout: every cell carries a `CellHead`.
///
/// # DO NOT DELETE THE `debug_assert!` BELOW WITHOUT RE-RUNNING THE W-d′ ARM
///
/// It is unfireable, and it was deleted once for exactly that reason (code
/// review, 2026-09-06). MEASURED consequence, same command, single variable:
/// `miri_scope_completion_protector` under `--features ke16-w-count,ke16-a2`
/// went from `ctx=W-d-prime firings=2/2 overlaps=2/2` (green) to
/// `firings=2/2 overlaps=0/2` — the anti-vacuity assert firing, deterministic
/// over three runs, because the arm's release window is a few instructions wide
/// in a DEBUG build and this assert is on the spawn path.
///
/// So the coupling is real but accidental, and it cuts both ways: the arm's
/// armedness rests on the instruction count of a function it does not mention.
/// The durable fix is to re-tune `MIRI_RELEASE_PROBE_YIELDS` against an
/// observation (the harness says so itself, in the message that fires), which
/// is the gate owner's call, not this function's. Until then this assert stays
/// and this paragraph is why.
#[inline]
fn alloc_cell<C>() -> *mut C {
    let layout = Layout::new::<C>();
    // Statically true — `C` is only ever `ScopedCell<F>` / `DetachedCell<F>`,
    // each `#[repr(C)]` with a `CellHead` first — so this documents `alloc`'s
    // precondition rather than testing it. See the header before deleting it.
    debug_assert!(
        layout.size() > 0,
        "invariant: every cell carries a CellHead"
    );
    // SAFETY: `layout` has non-zero size, which is `alloc`'s only precondition.
    let raw = unsafe { alloc(layout) };
    if raw.is_null() {
        handle_alloc_error(layout);
    }
    raw.cast::<C>()
}

/// Run a scoped body: THE function the protector obligation is stated over.
///
/// The activation order is the whole argument, and it is enforced by the
/// signature rather than by care:
///
/// 1. `shared` and `body` are PLACE-READS through a raw pointer. They form no
///    reference, so this frame holds no protector over the cell or the scope.
/// 2. The cell is freed here, before the body runs.
/// 3. `catch_unwind` receives the body BY VALUE, so the user's `&T`s are
///    protected for ITS activation — and that activation RETURNS at step 4's
///    semicolon, while this task's own registration is still counted in
///    `pending` and no join can have observed zero.
/// 4. `capture_panic` reborrows `*shared` on the cold path and returns.
/// 5. Only then does `complete_task` run the release RMW, with an argument list
///    of exactly one raw pointer.
///
/// # Safety
///
/// `ptr` must be the address of a live `ScopedCell<F>` minted by
/// [`Task::new_scoped`] with this same `F`, not yet run and not yet dropped, and
/// whose `shared` still counts this task in `pending`.
unsafe fn run_scoped<F: FnOnce()>(ptr: *const ()) {
    let cell = ptr.cast::<ScopedCell<F>>().cast_mut();

    // SAFETY: `cell` is live and fully initialised per this function's
    //   contract. Both are place-reads through a raw pointer: `shared` is a
    //   `Copy` field read, and `body` is moved OUT of the cell by
    //   `ptr::read` — after it the cell's `body` must never be read or dropped
    //   again, which is why the deallocation below is unconditional and why the
    //   element that named this cell was forgotten by `Task::run`.
    let shared: *const ScopeShared = unsafe { (*cell).shared };
    let body: F = unsafe { ptr::read(ptr::addr_of!((*cell).body)) };

    // SAFETY: the cell was allocated in `Task::new_scoped` with exactly this
    //   layout, its body has just been moved out (so no value in it is still
    //   owned by the allocation), and this is its single free site — `Task::run`
    //   forgot the element, so no `drop_unrun` will free it a second time.
    //   `dealloc`, not `Box::from_raw`: a `Box` would mint a `Unique` protector
    //   over the payload for the rest of this frame.
    unsafe { dealloc(cell.cast::<u8>(), Layout::new::<ScopedCell<F>>()) };

    // THE ACTIVATION THAT CARRIES THE USER'S REFERENCES, and it ends here. Every
    // `&T` nested inside `body` is protected for the duration of this call and
    // no longer: `catch_unwind` takes the aggregate by value, invokes it, and
    // returns before any line below runs.
    let result = catch_unwind(AssertUnwindSafe(body));

    if let Err(payload) = result {
        // SAFETY: the `ScopeShared` allocation is live — THIS task has not
        //   completed, so `pending` still counts it and no join can have
        //   observed zero, and `Scope::drop` is the only free site. The `&self`
        //   protector `capture_panic` installs therefore expires strictly before
        //   the release below.
        unsafe { (*shared).capture_panic(payload) };
    }

    // SAFETY: live for the reason above, and this is the allocation's LAST use
    //   on this thread. The reclamation this RMW authorises is `Scope::drop`'s:
    //   the `ScopeShared` allocation, plus — once the user's `scope` call
    //   returns — whatever the `'scope` frame owns. So the obligation is over
    //   the WHOLE activation stack below this frame, and there are exactly two,
    //   because a task runs either on a worker or inline on a joiner:
    //
    //   WORKER: `worker_main(Arc<PoolInner>, u32, Worker<Task>)` — every
    //   argument by value, no reference at all -> `worker::run_task(t: Task)`
    //   -> its `catch_unwind`, whose closure captures that POD element ->
    //   `Task::run(ManuallyDrop<Task>)` -> this frame, argument list `*const ()`.
    //
    //   JOINER (`Scope::drop` stealing a task of the very scope it waits on, the
    //   worst case): `Scope::drop(&mut self)` -> `join_workers_until_drained(
    //   &PoolInner, *const ScopeShared)` -> optionally `drain_scratch(
    //   &Worker<Task>)` -> the same `run_task` / `catch_unwind` / `Task::run`
    //   tail. Its three reference-typed arguments are each outside the
    //   reclaimed range: `&PoolInner` names `Arc`-owned state that outlives
    //   every scope; `&Worker<Task>` names a scratch deque that is a LOCAL of
    //   `join_workers_until_drained`, whose frame pops strictly after this join
    //   returns; and `&mut Scope` names the joiner's own `Scope`, which holds
    //   its `ScopeShared` as `NonNull` rather than `&`, so the allocation this
    //   RMW frees is not inside what that protector covers. `shared` itself
    //   crosses as a raw pointer — Phase 9.2 Candidate U, for this reason.
    //   (`try_steal_any` is NOT in either list: it returns its `Some(t)` and
    //   pops before `run_task` is entered.)
    //
    //   NOT live on either stack: any activation that received `body`, or a
    //   reference nested in it — `catch_unwind` and the body's own `call_once`
    //   both ended at their closing braces above, while `pending >= 1`.
    unsafe { ScopeShared::complete_task(shared) };
}

/// Run a fire-and-forget body.
///
/// No scope, no completion accounting: nothing here authorises any thread to
/// free anything, so the protector obligation is vacuous. The cell is still
/// freed before the body runs, which keeps the shape identical to
/// [`run_scoped`]'s and means a panicking body leaks nothing.
///
/// An unwind propagates to [`worker::run_task`](crate::worker::run_task), which
/// aborts (rayon's `spawn` policy, 2026-07 audit).
///
/// # Safety
///
/// `ptr` must be the address of a live `DetachedCell<F>` minted by
/// [`Task::new_detached`] with this same `F`, not yet run and not yet dropped.
unsafe fn run_detached<F: FnOnce()>(ptr: *const ()) {
    let cell = ptr.cast::<DetachedCell<F>>().cast_mut();

    // SAFETY: `cell` is live and fully initialised per this function's
    //   contract; `body` is moved out by a place-read through a raw pointer and
    //   the cell is not read again.
    let body: F = unsafe { ptr::read(ptr::addr_of!((*cell).body)) };

    // SAFETY: as in `run_scoped` — the layout is the constructor's, the body has
    //   been moved out, and this is the cell's single free site.
    unsafe { dealloc(cell.cast::<u8>(), Layout::new::<DetachedCell<F>>()) };

    body();
}

/// Teardown thunk for a scoped cell that never ran.
///
/// # Safety
///
/// `ptr` must be the address of a live `ScopedCell<F>` minted by
/// [`Task::new_scoped`] with this same `F`, not yet run and not yet dropped.
unsafe fn drop_unrun_scoped<F>(ptr: *const ()) {
    let cell = ptr.cast::<ScopedCell<F>>().cast_mut();
    // SAFETY: `cell` is live and its `body` is initialised and still owned by
    //   the allocation (the element never ran, so nothing read it out). The
    //   drop runs before the free, and the free uses the constructor's layout.
    unsafe {
        ptr::drop_in_place(ptr::addr_of_mut!((*cell).body));
        dealloc(cell.cast::<u8>(), Layout::new::<ScopedCell<F>>());
    }
}

/// Teardown thunk for a detached cell that never ran.
///
/// # Safety
///
/// `ptr` must be the address of a live `DetachedCell<F>` minted by
/// [`Task::new_detached`] with this same `F`, not yet run and not yet dropped.
unsafe fn drop_unrun_detached<F>(ptr: *const ()) {
    let cell = ptr.cast::<DetachedCell<F>>().cast_mut();
    // SAFETY: as in `drop_unrun_scoped` — a live cell whose body was never moved
    //   out, dropped in place and then freed with the constructor's layout.
    unsafe {
        ptr::drop_in_place(ptr::addr_of_mut!((*cell).body));
        dealloc(cell.cast::<u8>(), Layout::new::<DetachedCell<F>>());
    }
}


#[cfg(test)]
mod tests {
    //! Teardown coverage for the two `drop_unrun` thunks (KE16 finding W3).
    //!
    //! These live in `src/` rather than in `tests/` because [`Task`] is
    //! `pub(crate)`: an integration test cannot mint one, and a thunk/layout
    //! mismatch here is heap corruption that no pool-level test can see.
    //!
    //! The "still frees" half of the obligation is NOT asserted by a counter
    //! here — a counting global allocator would be process-global and these
    //! tests run in parallel with the crate's other unit tests. It is decided
    //! by running this module under Miri with leak checking ON (i.e. `MIRIFLAGS`
    //! WITHOUT `-Zmiri-ignore-leaks`), where a cell that is dropped but not
    //! freed is a reported leak and a cell freed twice is reported UB.

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crossbeam_deque::{Injector, Steal, Worker};

    use super::*;
    use crate::scope::ScopeShared;

    /// A body payload whose destructor is observable.
    ///
    /// Holds an `Arc` as well as the counter so the tests can check the two
    /// things a teardown owes separately: that `drop` RAN (the counter) and
    /// that what the body OWNED was released (`Arc::strong_count`).
    struct Witness {
        drops: Arc<AtomicUsize>,
        // Never read BY DESIGN, and that is what it measures: the test observes it
        // from outside through `Arc::strong_count`, so a teardown that ran `drop`
        // without releasing what the body OWNED still fails. Reading it here would
        // defeat the point — the field's whole job is to be dropped, not consulted.
        #[allow(dead_code)]
        held: Arc<u32>,
    }

    impl Drop for Witness {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// A live `ScopeShared` with one task registered, for the scoped arm.
    ///
    /// `Task::new_scoped`'s contract requires a live `ScopeShared` whose
    /// `pending` already counts the task. The unrun path deliberately does NOT
    /// complete that registration (see [`Task::drop`]'s doc), so this scope is
    /// dropped without a join, which is what a torn-down queue looks like.
    fn registered_scope() -> Box<ScopeShared> {
        let shared = Box::new(ScopeShared::new(std::thread::current(), ptr::null()));
        shared.register_task();
        shared
    }

    #[test]
    fn dropping_an_unrun_detached_task_runs_the_body_destructor() {
        let drops = Arc::new(AtomicUsize::new(0));
        let held = Arc::new(7u32);
        let w = Witness {
            drops: Arc::clone(&drops),
            held: Arc::clone(&held),
        };

        let task = Task::new_detached(move || {
            let _ = &w;
        });
        assert_eq!(
            Arc::strong_count(&held),
            2,
            "the body still owns its Arc while the task is alive"
        );

        drop(task);

        assert_eq!(
            drops.load(Ordering::Acquire),
            1,
            "a detached task dropped unrun must run the body's destructor exactly once"
        );
        assert_eq!(
            Arc::strong_count(&held),
            1,
            "the body's Arc clone must be released by the teardown thunk"
        );
    }

    #[test]
    fn dropping_an_unrun_scoped_task_runs_the_body_destructor() {
        let shared = registered_scope();
        let drops = Arc::new(AtomicUsize::new(0));
        let held = Arc::new(7u32);
        let w = Witness {
            drops: Arc::clone(&drops),
            held: Arc::clone(&held),
        };

        // SAFETY: `shared` is live for this whole function and its `pending`
        //   counts this task (`registered_scope`). The task is dropped, never
        //   run, so nothing dereferences `shared` through the element.
        let task = unsafe {
            Task::new_scoped(&*shared as *const ScopeShared, move || {
                let _ = &w;
            })
        };

        drop(task);

        assert_eq!(
            drops.load(Ordering::Acquire),
            1,
            "a scoped task dropped unrun must run the body's destructor exactly once"
        );
        assert_eq!(
            Arc::strong_count(&held),
            1,
            "the body's Arc clone must be released by the teardown thunk"
        );
    }

    #[test]
    fn a_detached_task_that_ran_does_not_also_run_the_teardown_thunk() {
        let drops = Arc::new(AtomicUsize::new(0));
        let ran = Arc::new(AtomicUsize::new(0));
        let w = Witness {
            drops: Arc::clone(&drops),
            held: Arc::new(0),
        };
        let ran_in = Arc::clone(&ran);

        let task = Task::new_detached(move || {
            let _ = &w;
            ran_in.fetch_add(1, Ordering::AcqRel);
        });
        task.run();

        assert_eq!(ran.load(Ordering::Acquire), 1, "the body must have run");
        assert_eq!(
            drops.load(Ordering::Acquire),
            1,
            "a task that RAN drops its body exactly once - the teardown thunk must not \
             also fire (double drop) and the body must not leak"
        );
    }

    #[test]
    fn a_scoped_task_that_ran_does_not_also_run_the_teardown_thunk() {
        let shared = registered_scope();
        let drops = Arc::new(AtomicUsize::new(0));
        let w = Witness {
            drops: Arc::clone(&drops),
            held: Arc::new(0),
        };

        // SAFETY: `shared` is live and counts this task; running the element is
        //   what completes that registration, exactly once.
        let task = unsafe {
            Task::new_scoped(&*shared as *const ScopeShared, move || {
                let _ = &w;
            })
        };
        task.run();

        assert_eq!(
            drops.load(Ordering::Acquire),
            1,
            "a scoped task that RAN drops its body exactly once"
        );
        assert!(
            shared.is_drained(),
            "running the element must complete the scope registration"
        );
    }

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
        // `DetachedCell` / `CellHead` are private to this module, so no other
        // file in the crate can name them.
        assert_eq!(
            core::mem::offset_of!(CellHead, drop_unrun),
            0,
            "the thunk is the first field of the shared prefix"
        );
        assert_eq!(
            core::mem::offset_of!(ScopedCell<()>, head),
            0,
            "a scoped cell begins with its head, which is what makes Task::drop's blind \
             offset-0 load well defined"
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
