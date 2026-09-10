//! The scoped kind: the cell a [`Scope`](crate::Scope) spawn emplaces in its
//! block, the function that runs it, and the thunk that tears it down unrun.
//!
//! Split out of `task.rs` in Stage 3b so that `ScopedCell`'s visibility is
//! `pub(in crate::task)` — nameable by `mod.rs`'s constructors and by nothing
//! else in the crate, which is D3 in `block.rs`'s header.

use core::ptr;
use std::panic::{AssertUnwindSafe, catch_unwind};

use super::CellHead;
use crate::scope::ScopeShared;

/// Payload of a task spawned into a [`Scope`](crate::Scope).
///
/// Lives in the scope's [`ScopeBlock`](crate::block::ScopeBlock), not in an
/// allocation of its own: [`Task::new_scoped`](super::Task::new_scoped)
/// emplaces it and `Scope::drop`'s `free_all` reclaims every cell of the scope
/// at once.
#[repr(C)]
pub(super) struct ScopedCell<F> {
    pub(super) head: CellHead,
    /// The scope's shared state, as an ADDRESS. Copied out by value before the
    /// body runs; never dereferenced through a reference stored anywhere.
    pub(super) shared: *const ScopeShared,
    pub(super) body: F,
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

/// Run a scoped body: THE function the protector obligation is stated over.
///
/// The activation order is the whole argument, and it is enforced by the
/// signature rather than by care:
///
/// 1. `shared` and `body` are PLACE-READS through a raw pointer. They form no
///    reference, so this frame holds no protector over the cell or the scope.
/// 2. `catch_unwind` receives the body BY VALUE, so the user's `&T`s are
///    protected for ITS activation — and that activation RETURNS at step 3's
///    semicolon, while this task's own registration is still counted in
///    `pending` and no join can have observed zero.
/// 3. `capture_panic` reborrows `*shared` on the cold path and returns.
/// 4. Only then does `complete_task` run the release RMW, with an argument list
///    of exactly one raw pointer.
///
/// The step this list USED to have between 1 and 2 — "the cell is freed here,
/// before the body runs" — is gone, because the cell is block memory and the
/// block is freed once, by `Scope::drop`. What that costs the argument is
/// stated in the module header's allocation clause, as a downgrade rather than
/// a rewording.
///
/// # Safety
///
/// `ptr` must be the address of a live `ScopedCell<F>` minted by
/// [`Task::new_scoped`](super::Task::new_scoped) with this same `F`, not yet run
/// and not yet dropped, and whose `shared` still counts this task in `pending`.
pub(super) unsafe fn run_scoped<F: FnOnce()>(ptr: *const ()) {
    let cell = ptr.cast::<ScopedCell<F>>().cast_mut();

    // SAFETY: `cell` is live and fully initialised per this function's
    //   contract. Both are place-reads through a raw pointer: `shared` is a
    //   `Copy` field read, and `body` is moved OUT of the cell by
    //   `ptr::read` — after it the cell's `body` must never be read or dropped
    //   again, which is why the element that named this cell was forgotten by
    //   `Task::run` (so no `drop_unrun_scoped` can run over the moved-out
    //   body) and why the storage goes back to the allocator wholesale, in
    //   `Scope::drop`'s `free_all`, rather than through any free on this path.
    let shared: *const ScopeShared = unsafe { (*cell).shared };
    let body: F = unsafe { ptr::read(ptr::addr_of!((*cell).body)) };

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
    //   the scope's BLOCK CHUNKS (`free_all`, the first thing after the join),
    //   then the `ScopeShared` allocation, plus — once the user's `scope` call
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
    //
    //   THE CELL IS NOW INSIDE THE RECLAIMED RANGE TOO (Stage 3b), and it is
    //   the FIRST thing this RMW authorises anyone to reclaim, because
    //   `free_all` precedes the `ScopeShared` free by placement. The same
    //   obligation therefore has to be discharged over chunk memory — and it
    //   reduces to THIS ONE FUNCTION, by enumerating every frame that can name
    //   a chunk address at the instant `free_all` runs (plan5 D5):
    //
    //     1. ANY WORKER. The only route a chunk address takes to a worker is
    //        `Task.payload`, typed `*const ()`, and the only frames that
    //        receive it are `Task::run` -> `execute(payload)`, i.e.
    //        `run_scoped::<F>` or `drop_unrun_scoped::<F>`. Nothing else can be
    //        stored in `execute`: `Task` is `pub(crate)` and its two
    //        constructors are the only mints.
    //     2. `drop_unrun_scoped` — VACUOUS, and that is a proof rather than a
    //        gap. An unrun scoped task's registration is still counted in
    //        `pending`, so the join has not returned, so `free_all` has not
    //        run. There is no free for a protector to span.
    //     3. THE SPAWNER SIDE (`ScopeBlock::emplace`, `Task::new_scoped`) —
    //        vacuous for the same reason: both run on the owner thread inside
    //        `Scope::spawn`, strictly before `Scope::drop`, and the one thread
    //        that can call `free_all` is the thread executing them.
    //     4. THE JOINER ITSELF. `free_all(&self)` reads `bases[i]` / `exps[i]`,
    //        which are frame memory and not chunk memory, and calls `dealloc`
    //        on raw pointers copied out of them. A joiner that ran a task
    //        inline did so through THIS function, whose frame popped before the
    //        join returned.
    //
    //   So the whole question is about this frame: does any reference it forms
    //   into the cell become a protected argument of a call that is live when
    //   the RMW below commits? It forms NONE — the place-read clause at the top
    //   is what says so — and that is the question KE16's M2w gate executes
    //   rather than assumes.
    unsafe { ScopeShared::complete_task(shared) };
}

// =========================================================================
// KE16 M2w — THE DELIBERATE COUNTEREXAMPLE.
//
// Everything under this banner is `#[cfg(feature = "tb-neg-m2w")]` and exists
// in order to BE REPORTED as Undefined Behaviour. `src/lib.rs` refuses the
// feature outside `cfg(miri)`, so it cannot reach a native artifact.
//
// Why it exists at all: M2w's positive arm asserts that the shipped
// `run_scoped` above executed under Tree Borrows with the chunk freed inside a
// completer's open release window and that no UB was reported. That assertion
// is worth nothing unless a configuration exists which WOULD have reported
// one — a green against which no red can be produced is a gate that cannot
// fail, which is this campaign's own catalogued failure shape. This is that
// configuration, and it is the reason Stage 3b's weakened argument (see the
// module header's "THE CLAUSE THE SCOPED PATH LOST") is checked rather than
// asserted.
// =========================================================================

// Keeps the SHIPPED `run_scoped` REFERENCED in this configuration, where the
// arm has taken over `Task::new_scoped`'s `execute` value and nothing else
// names it. The alternative is a `cfg_attr(…, allow(dead_code))` on the
// shipped function — a SECOND conditional edit to the shipped path, when the
// arm's entire claim is that it makes exactly one. The coercion is a
// compile-time fn-pointer instantiation and runs nothing.
#[cfg(feature = "tb-neg-m2w")]
const _: unsafe fn(*const ()) = run_scoped::<fn()>;

/// [`run_scoped`] plus exactly one protector over chunk memory: the KE16 M2w
/// NEGATIVE CONTROL.
///
/// # The delta, stated as the two textual changes it is
///
/// 1. the cell is bound as a REFERENCE — `let cell: &ScopedCell<F> = &*ptr.cast();`
///    — where [`run_scoped`] binds a `*mut ScopedCell<F>`;
/// 2. the release RMW is performed by [`finish_neg`], which takes that
///    reference as an ARGUMENT, so the protector it installs is live across
///    the RMW.
///
/// Everything else is the shipped text: the same `shared` read, the same
/// `ptr::read` body read-out through `addr_of!`, the same `catch_unwind`, the
/// same panic capture. That is deliberate — a ONE-PROTECTOR delta is what
/// makes the UB report ATTRIBUTABLE, and an arm that also moved the read-out
/// or the capture would report UB without saying which change earned it. (The
/// `shared` read loses its `unsafe` block, because a field read through a
/// reference needs none. That is a consequence of change 1, not a third
/// change.)
///
/// # Why this is exactly one protector, and over the right allocation
///
/// Tree Borrows installs a protector for a reference that is an ARGUMENT of a
/// live call, and it is a deallocation under a live protector that is
/// forbidden — a reference merely created and used installs nothing, which is
/// why the spelling here is the plainest one available rather than an evasive
/// one. `ScopeShared::complete_task` still takes `*const Self`, so the only
/// protected reference in this frame covers the CELL — chunk memory — and not
/// the scope's own box. The ordering then picks the allocation out for the
/// reader: `free_all` is the FIRST reclamation `Scope::drop` performs after
/// its join, ahead of `Box::from_raw` by placement, so the accessed tag in the
/// diagnostic is created in `block.rs` and names a chunk.
///
/// # Reading the result
///
/// A run of this arm that reports NO UB is a RED gate, not a pass: it means
/// the negative has stopped being negative, and the positive arm's green is no
/// longer known to be falsifiable.
///
/// # Safety
///
/// The same contract as [`run_scoped`] — `ptr` must address a live,
/// not-yet-run and not-yet-dropped `ScopedCell<F>` of this same `F`, whose
/// `shared` still counts this task in `pending` — plus one clause the shipped
/// function does not have: **this function is expected to commit UB even when
/// its contract is upheld.** That is its purpose, and it is why the feature
/// that compiles it does not build natively.
#[cfg(feature = "tb-neg-m2w")]
pub(super) unsafe fn run_scoped_neg<F: FnOnce()>(ptr: *const ()) {
    // CHANGE 1 OF 2: a reference where the shipped path holds a raw pointer.
    //
    // SAFETY: `ptr` is the address of a live, fully initialised
    //   `ScopedCell<F>` per this function's contract, so the reference is
    //   well formed, aligned and non-null. What it is NOT is harmless — this
    //   is the counterexample, and the argument list of `finish_neg` below is
    //   where the tag it mints becomes a protector that spans a free.
    let cell: &ScopedCell<F> = unsafe { &*ptr.cast() };

    let shared: *const ScopeShared = cell.shared;
    // SAFETY: the body is initialised and still owned by the cell — this task
    //   has not run — and `ptr::read` moves it OUT, after which the cell's
    //   `body` is never read or dropped again: the element that named the cell
    //   is forgotten by `Task::run`, so no `drop_unrun_scoped` follows.
    //   Identical to the shipped read-out.
    //
    // `explicit_auto_deref` is allowed rather than obeyed: clippy would have
    // this read `addr_of!(cell.body)`, which is the same operation but not the
    // same TEXT as the shipped `run_scoped`. This arm's whole claim is that it
    // differs from that function by two changes, and re-spelling the read-out
    // to satisfy a style lint would make it three — turning an attributable UB
    // report into one that has to be argued for.
    #[allow(clippy::explicit_auto_deref)]
    let body: F = unsafe { ptr::read(ptr::addr_of!((*cell).body)) };

    // The activation that carries the user's references, and it ends here —
    // the shipped clause, unchanged.
    let result = catch_unwind(AssertUnwindSafe(body));

    if let Err(payload) = result {
        // SAFETY: the `ScopeShared` allocation is live — THIS task has not
        //   completed, so `pending` still counts it and no join can have
        //   observed zero. The `&self` protector `capture_panic` installs
        //   expires strictly before the release below. Identical to the
        //   shipped capture.
        unsafe { (*shared).capture_panic(payload) };
    }

    // CHANGE 2 OF 2: the release runs inside a call that holds `cell`, so a
    // protector over chunk memory is live at the instant the RMW commits.
    finish_neg(cell, shared);
}

/// Performs the release RMW with a protector over the cell live across it.
///
/// `_cell` is unused BY DESIGN: it is here to be a protected argument for the
/// duration of the release below, and that protector is the whole of the
/// negative control. `#[inline(never)]` so the frame cannot be inlined away by
/// a codegen backend — Miri interprets MIR and never inlines, so the attribute
/// is a belt on the braces rather than the mechanism.
///
/// Not an `unsafe fn`, which is the plan's signature and is defensible here
/// rather than merely prescribed: the function is module-private, gated behind
/// the same feature as its single caller, and that caller establishes its
/// preconditions immediately above the call. Widening it into an `unsafe fn`
/// would move the one property this function is measured on — the protector
/// its argument list installs — no distance at all.
#[cfg(feature = "tb-neg-m2w")]
#[inline(never)]
fn finish_neg<F>(_cell: &ScopedCell<F>, shared: *const ScopeShared) {
    // SAFETY: `shared` addresses a live `ScopeShared` whose `pending` still
    //   counts this task — `run_scoped_neg`'s contract, and it is this
    //   function's only caller — so the RMW itself is the same one the shipped
    //   path performs and is sound on its own terms.
    //
    //   WHAT IS UNSOUND HERE, DELIBERATELY, IS THE ARGUMENT ABOVE. `_cell`'s
    //   protector covers chunk memory and is live for this entire activation,
    //   while this RMW is precisely what authorises `Scope::drop` to run
    //   `free_all` and deallocate that chunk. Tree Borrows must therefore
    //   report a deallocation through a protected tag, with the protected tag
    //   created in `run_scoped_neg` and the accessed tag created in
    //   `block.rs`. A run in which it reports nothing is a RED gate.
    unsafe { ScopeShared::complete_task(shared) };
}

/// Teardown thunk for a scoped cell that never ran.
///
/// Drops the body and frees NOTHING: since Stage 3b the cell is block memory,
/// and the block is freed once, by `Scope::drop`'s `free_all`. A `dealloc`
/// here would be a mismatched free of a bump interior — and it is not merely
/// forbidden but unreachable-by-order, because an unrun scoped task's
/// registration is still counted in `pending`, so the join `free_all` sits
/// behind has not returned.
///
/// # D2 IS QUALIFIED HERE, AND THIS PATH IS THE QUALIFICATION
///
/// `block.rs`'s header states D2 as "no caller can form a REFERENCE into chunk
/// memory … checked by the type system, not by a grep". **That is exact for
/// every route through `BlockPtr` — whose only exit is `erase(self) -> *const
/// ()` — and it is FALSE as an unqualified statement about the crate, because
/// of this function.** `ptr::drop_in_place` takes `*mut F` and forms nothing,
/// but the drop glue it invokes calls the user's `Drop::drop(&mut self)`: a
/// `&mut F` whose referent IS chunk memory, passed as an argument, hence a
/// genuine STRONG protector over a chunk for the whole of the user's
/// destructor. It is the only such protector in the crate, and it is here.
///
/// **The property still holds, and it holds by ORDERING rather than by
/// absence** — which is why the sentence is qualified instead of the fact
/// being dropped. A `Task` dropped unrun does not release its registration
/// (see [`Task::drop`](super::Task)), so `pending` still counts it, so
/// `Scope::drop`'s join has not returned, so `free_all` has not run: there is
/// no free for this protector to span, in any interleaving. The protector is
/// real; the deallocation it could conflict with is unreachable while it is
/// live.
///
/// Stating it the other way round — "no reference into chunk memory exists" —
/// is falsifiable in ten seconds by a reader who follows the drop glue, and
/// the whole soundness position after Stage 3b rests on these sentences being
/// exact.
///
/// # Safety
///
/// `ptr` must be the address of a live `ScopedCell<F>` minted by
/// [`Task::new_scoped`](super::Task::new_scoped) with this same `F`, not yet run
/// and not yet dropped.
pub(super) unsafe fn drop_unrun_scoped<F>(ptr: *const ()) {
    let cell = ptr.cast::<ScopedCell<F>>().cast_mut();
    // SAFETY: `cell` is live and its `body` is initialised and still owned by
    //   the cell (the element never ran, so nothing read it out). The
    //   `addr_of_mut!` place expression forms no reference into the cell —
    //   though the drop glue below does, and the doc's D2 clause states
    //   exactly why that `&mut F` protector cannot span a free — and it is
    //   the spelling that is correct for EVERY `F`: the body's offset is
    //   `align_up(16, align_of::<F>())`, not a constant 16, so any fixed
    //   byte-offset spelling is wrong for an over-aligned body — and AVX2
    //   (align 32) is this engine's stated ISA baseline.
    unsafe { ptr::drop_in_place(ptr::addr_of_mut!((*cell).body)) };
}

#[cfg(test)]
mod tests {
    //! Teardown coverage for `drop_unrun_scoped` (KE16 finding W3), and — since
    //! Stage 3b — for the block ownership the scoped cell moved into. See
    //! `super::super::teardown_support` for the Miri recipe these are decided
    //! under: leak checking is ON, so a test that does not free its own block
    //! is red.

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::Task;
    use super::super::teardown_support::Witness;
    use super::*;
    use crate::block::ScopeBlock;

    /// A live `ScopeShared` with one task registered, for the scoped arm.
    ///
    /// `Task::new_scoped`'s contract requires a live `ScopeShared` whose
    /// `pending` already counts the task. The unrun path deliberately does NOT
    /// complete that registration (see `Task::drop`'s doc), so this scope is
    /// dropped without a join, which is what a torn-down queue looks like.
    fn registered_scope() -> Box<ScopeShared> {
        let shared = Box::new(ScopeShared::new(std::thread::current(), ptr::null()));
        shared.register_task();
        shared
    }

    #[test]
    fn dropping_an_unrun_scoped_task_runs_the_body_destructor() {
        let shared = registered_scope();
        // Declared before the element and freed after it, because the cell
        // lives INSIDE it: a block released first would leave the teardown
        // thunk reading freed memory, and a block never released is a leak the
        // Miri recipe reports.
        let block = ScopeBlock::new();
        let drops = Arc::new(AtomicUsize::new(0));
        let held = Arc::new(7u32);
        let w = Witness {
            drops: Arc::clone(&drops),
            held: Arc::clone(&held),
        };

        // SAFETY: `shared` is live for this whole function and its `pending`
        //   counts this task (`registered_scope`). The task is dropped, never
        //   run, so nothing dereferences `shared` through the element. `block`
        //   is the block of that same scope, it outlives the element, and it is
        //   not freed until the end of this function.
        let task = unsafe {
            Task::new_scoped(&block, &*shared as *const ScopeShared, move || {
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

        // SAFETY: the block's one emplaced value is already dead — the `drop`
        //   above ran `drop_unrun_scoped`, which dropped the body in place —
        //   and nothing can dereference the cell afterwards: the element that
        //   held the erased pointer was consumed by that same drop, and no
        //   `BlockPtr` was kept. Called once, on the thread that owns the
        //   block.
        unsafe { block.free_all() };
    }

    #[test]
    fn a_scoped_task_that_ran_does_not_also_run_the_teardown_thunk() {
        let shared = registered_scope();
        let block = ScopeBlock::new();
        let drops = Arc::new(AtomicUsize::new(0));
        let w = Witness {
            drops: Arc::clone(&drops),
            held: Arc::new(0),
        };

        // SAFETY: `shared` is live and counts this task; running the element is
        //   what completes that registration, exactly once. `block` holds the
        //   cell, outlives the element, and is freed only below — after the run
        //   has moved the body out of it.
        let task = unsafe {
            Task::new_scoped(&block, &*shared as *const ScopeShared, move || {
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

        // SAFETY: the block's one emplaced value is dead — `run_scoped` moved
        //   the body OUT of the cell by `ptr::read` and the cell is not read
        //   again — and the element that named it was forgotten by `Task::run`,
        //   so no erased copy survives to be dereferenced. Called once, on the
        //   thread that owns the block. This is the miniature of `Scope::drop`:
        //   the run completed the registration, which is what authorises the
        //   free.
        unsafe { block.free_all() };
    }
}
