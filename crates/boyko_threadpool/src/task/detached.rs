//! The fire-and-forget kind: its cell, the allocator that mints one, the
//! function that runs it, and the thunk that tears it down unrun.
//!
//! This is the kind Stage 3b did NOT change. A detached task has no scope, so
//! there is no block to emplace its cell in and no join to reclaim one behind:
//! it keeps the per-task `alloc`/`dealloc` pair, and `alloc_cell` lives here
//! because [`Task::new_detached`](super::Task::new_detached) is its only
//! remaining caller.

use core::alloc::Layout;
use core::ptr;
use std::alloc::{alloc, dealloc, handle_alloc_error};

use super::CellHead;

/// Payload of a fire-and-forget task (`ThreadPool::spawn`). No scope, no
/// completion accounting, no panic capture — an unwind out of the body reaches
/// [`worker::run_task`](crate::worker::run_task) and aborts.
#[repr(C)]
pub(super) struct DetachedCell<F> {
    pub(super) head: CellHead,
    pub(super) body: F,
}

// Pins `__layout_receipt::DETACHED_CELL_HEADER`, doubly, for the reason stated
// over `ScopedCell` in `scoped.rs`: this cell's header is the thunk alone, and
// a single assert would not notice a cell that had stopped carrying its body.
const _: () =
    assert!(size_of::<DetachedCell<()>>() == crate::__layout_receipt::DETACHED_CELL_HEADER);
const _: () = assert!(
    size_of::<DetachedCell<[u8; 64]>>() == crate::__layout_receipt::DETACHED_CELL_HEADER + 64
);
// The alignment half of this cell class's bucket, for the reason stated over
// `ScopedCell` in `scoped.rs`.
const _: () =
    assert!(align_of::<DetachedCell<()>>() == crate::__layout_receipt::DETACHED_CELL_ALIGN);

/// Allocate one uninitialised cell.
///
/// Never a zero-sized layout: every cell carries a `CellHead`.
///
/// # THE `debug_assert!` BELOW, AND THE RE-CHECK THIS COMMIT OWES
///
/// It is unfireable, and it was deleted once for exactly that reason (code
/// review, 2026-09-06). MEASURED consequence, same command, single variable:
/// `miri_scope_completion_protector` under `--features ke16-w-count,ke16-a2`
/// (the features are gone since Step App; the spellings are kept because they
/// are the exact key into `KE16-REJECTED.md`'s `Flag` column) went from
/// `ctx=W-d-prime firings=2/2
/// overlaps=2/2` (green) to `firings=2/2 overlaps=0/2` — the anti-vacuity
/// assert firing, deterministic over three runs, because the arm's release
/// window is a few instructions wide in a DEBUG build and this assert was on
/// the spawn path.
///
/// **THE COUPLING NO LONGER HAS THE FORM IT WAS MEASURED IN, and that is a
/// re-check owed rather than a coupling removed.** The measurement was taken
/// when the SCOPED kind's spawn path ran through this function; since Stage 3b
/// it does not — a scoped spawn emplaces its cell in the scope's block and
/// never reaches here, so the instructions this assert contributes are no
/// longer on the path whose window the W-d′ arm observes. Whether that arm is
/// still armed is therefore an open question ON THIS COMMIT, not an inherited
/// green: it is exit condition 5 of the plan and it is settled by re-running
/// the arm, not by reasoning from the old number.
///
/// The build it was measured in cannot be rebuilt either. That run selected the
/// KE16 W-d′ arm, which the tournament settled on and which now ships
/// unconditionally, together with A2, an axis-A candidate it rejected;
/// no `ke16-` candidate `cfg` survives in the sources, so the re-run is the
/// gate as it ships. That makes the old pair of numbers history, not a
/// baseline: neither half of it was taken on the configuration that ships.
///
/// Two things the tree already says about how to settle it. The cheap answer is
/// refused: `MIRI_RELEASE_PROBE_YIELDS`'s own doc records that ARMEDNESS IS NOT
/// MONOTONIC in that constant (disarmed at 8 between armed points at 7 and 10),
/// so "re-tune the constant" is not a fallback. And what certifies armedness is
/// an observation rather than an inequality —
/// `MIRI_FREES_INSIDE_A_RELEASE_WINDOW`, asserted by the gate — so a coupling
/// that has moved shows up as that count falling short and a LOUD red, not as a
/// green census over a defect.
///
/// The assert stays. It is `alloc`'s documented precondition, this paragraph is
/// the reason it is not deleted for being unfireable, and the detached kind
/// still spawns through here.
#[inline]
pub(super) fn alloc_cell<C>() -> *mut C {
    let layout = Layout::new::<C>();
    // Statically true — `C` is only ever `DetachedCell<F>`, `#[repr(C)]` with a
    // `CellHead` first — so this documents `alloc`'s precondition rather than
    // testing it. See the header before deleting it.
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

/// Run a fire-and-forget body.
///
/// No scope, no completion accounting: nothing here authorises any thread to
/// free anything, so the protector obligation is vacuous. The cell is still
/// freed before the body runs, which keeps the shape identical to the one
/// `run_scoped` had before Stage 3b and means a panicking body leaks nothing.
///
/// An unwind propagates to [`worker::run_task`](crate::worker::run_task), which
/// aborts (rayon's `spawn` policy, 2026-07 audit).
///
/// # Safety
///
/// `ptr` must be the address of a live `DetachedCell<F>` minted by
/// [`Task::new_detached`](super::Task::new_detached) with this same `F`, not yet
/// run and not yet dropped.
pub(super) unsafe fn run_detached<F: FnOnce()>(ptr: *const ()) {
    let cell = ptr.cast::<DetachedCell<F>>().cast_mut();

    // SAFETY: `cell` is live and fully initialised per this function's
    //   contract; `body` is moved out by a place-read through a raw pointer and
    //   the cell is not read again.
    let body: F = unsafe { ptr::read(ptr::addr_of!((*cell).body)) };

    // SAFETY: the layout is the constructor's (`alloc_cell::<DetachedCell<F>>`
    //   with this same `F`), the body has been moved out — so no value in the
    //   cell is still owned by the allocation — and this is the cell's single
    //   free site: `Task::run` forgot the element, so no `drop_unrun_detached`
    //   will free it a second time. `dealloc`, not `Box::from_raw`: a `Box`
    //   would mint a `Unique` protector over the payload for the rest of this
    //   frame.
    unsafe { dealloc(cell.cast::<u8>(), Layout::new::<DetachedCell<F>>()) };

    body();
}

/// Teardown thunk for a detached cell that never ran.
///
/// # Safety
///
/// `ptr` must be the address of a live `DetachedCell<F>` minted by
/// [`Task::new_detached`](super::Task::new_detached) with this same `F`, not yet
/// run and not yet dropped.
pub(super) unsafe fn drop_unrun_detached<F>(ptr: *const ()) {
    let cell = ptr.cast::<DetachedCell<F>>().cast_mut();
    // SAFETY: a live cell whose body was never moved out, dropped in place and
    //   then freed with the constructor's layout. This cell IS its own
    //   allocation — unlike the scoped one since Stage 3b — so the free belongs
    //   here and is this cell's only one.
    unsafe {
        ptr::drop_in_place(ptr::addr_of_mut!((*cell).body));
        dealloc(cell.cast::<u8>(), Layout::new::<DetachedCell<F>>());
    }
}

#[cfg(test)]
mod tests {
    //! Teardown coverage for `drop_unrun_detached` (KE16 finding W3). See
    //! `super::super::teardown_support` for the Miri recipe these are decided
    //! under — leak checking ON, which is what decides the "still frees" half.

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::Task;
    use super::super::teardown_support::Witness;

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
}
