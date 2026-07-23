//! [`Scope`] — borrow-erased fork/join over the pool.
//!
//! Plan §4.5. `Scope::spawn` accepts a closure with lifetime `'scope`;
//! we transmute the lifetime to `'static` for the duration of the task
//! body. The `'scope` correctness is upheld by [`Scope::drop`]: it
//! blocks (via work-stealing, not parking) until every spawned task has
//! completed, so no body outlives the `'scope` borrow.
//!
//! ## Work-stealing wait (plan §4.5.5)
//!
//! `Scope::Drop` does NOT call `std::thread::park()` unconditionally.
//! Instead, it polls the local injector (if on a worker), the global
//! injector, and sibling stealers. Without this, nested scopes can
//! deadlock when every worker is itself blocked inside its own
//! `Scope::Drop`.
//!
//! Note (plan §4.5.5 / Round 3 W-NEW-2): we do NOT drain the calling
//! worker's own Chase-Lev deque from `Scope::Drop`. That deque is owned
//! by `worker_main` on the worker's stack and is not accessible here.
//! Sufficient progress is guaranteed via the injectors and sibling
//! steals.

use core::marker::PhantomData;
use core::ptr::{self, NonNull};
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::time::Duration;

use crossbeam_deque::{Steal, Worker};
use crossbeam_utils::{Backoff, CachePadded};

use crate::sync::{AtomicPtr, AtomicUsize, Ordering, Thread};
use crate::thread_pool::{PoolInner, TaskHandle};
use crate::tls;
use crate::worker::{push_task, unpark_one_idle};

/// Shared state between [`Scope`] and its spawned tasks.
///
/// Allocated on the heap so that `Scope::spawn`'s closure can hold a
/// raw pointer to it that remains stable across `Scope` moves (the
/// `Scope` itself is small and `!Unpin` via the `Box<ScopeShared>`
/// indirection).
#[repr(C)]
pub(crate) struct ScopeShared {
    /// Count of outstanding spawned tasks. Increments on spawn, decrements
    /// on task completion (success or panic). `Scope::Drop` waits until
    /// this reaches zero.
    ///
    /// `CachePadded` because workers all decrement this on completion;
    /// the dispatcher's `Drop` thread reads it. Without padding, the
    /// completion traffic would false-share with neighbouring atomics.
    pub(crate) pending: CachePadded<AtomicUsize>,

    /// First panic payload observed by a task; consumed by `Scope::Drop` and re-raised via
    /// `resume_unwind`. Null means no task panicked.
    ///
    /// A `Box<dyn Any + Send>` is a FAT pointer and cannot live in an atomic, so the payload is
    /// boxed once more: the cell holds `*mut Box<dyn Any + Send>`, a thin pointer.
    ///
    /// 2026-07 audit: this was a `Mutex<Option<Box<dyn Any + Send>>>` documented as "cold-path
    /// only (panics are rare)". The WRITE is indeed panic-only, but `Scope::drop` read the slot
    /// through an unconditional `lock()` on every scope teardown and `ScopeShared::new` built a
    /// fresh `Mutex` per scope — and the parallel scheduler creates a scope per system run. The
    /// CAS-once slot keeps the same protocol (first panic wins, later payloads are dropped) with
    /// a null load on the path that does not panic.
    pub(crate) panic_payload: AtomicPtr<Box<dyn Any + Send + 'static>>,

    /// Thread to unpark when `pending` reaches zero. Captured at
    /// `Scope::new` time (typically `std::thread::current()` inside the
    /// surrounding `install`/`scope` frame).
    pub(crate) waker: Thread,
}

impl ScopeShared {
    /// Initialises the panic-payload slot to "no panic" — a null store, no allocation.
    #[inline]
    pub(crate) fn new(waker: Thread) -> Self {
        Self {
            pending: CachePadded::new(AtomicUsize::new(0)),
            panic_payload: AtomicPtr::new(ptr::null_mut()),
            waker,
        }
    }

    /// Publishes `payload` as THE panic of this scope, keeping the first one seen.
    ///
    /// Called only from the `Err` arm of a task body's `catch_unwind`. A racing second panic
    /// loses the CAS and drops its own payload, matching the previous `Mutex` protocol
    /// ("first wins; subsequent payloads dropped") without a lock.
    #[cold]
    #[inline(never)]
    pub(crate) fn capture_panic(&self, payload: Box<dyn Any + Send + 'static>) {
        let raw = Box::into_raw(Box::new(payload));
        if self
            .panic_payload
            .compare_exchange(ptr::null_mut(), raw, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            // SAFETY: `raw` was minted from `Box::into_raw` on the line above and the failed CAS
            //   means it was never published, so this thread is still its unique owner and no
            //   other thread can observe it. Reclaiming it here is the "later panics are
            //   dropped" half of the protocol.
            drop(unsafe { Box::from_raw(raw) });
        }
    }

    /// Takes the captured panic payload, leaving the slot empty.
    ///
    /// `Acquire` pairs with [`capture_panic`](Self::capture_panic)'s release half, so the
    /// payload's contents are visible to the taker.
    #[inline]
    pub(crate) fn take_panic(&self) -> Option<Box<dyn Any + Send + 'static>> {
        let raw = self.panic_payload.swap(ptr::null_mut(), Ordering::Acquire);
        if raw.is_null() {
            return None;
        }
        // SAFETY: a non-null pointer here was published by exactly one `capture_panic` CAS, and
        //   the `swap` that produced it atomically removed it from the cell — so this thread is
        //   now its unique owner. The pointer came from `Box::into_raw(Box::new(..))`, matching
        //   this `Box::from_raw`.
        Some(*unsafe { Box::from_raw(raw) })
    }

    /// Register one outstanding task. Called by [`Scope::spawn`] before the
    /// task body is enqueued.
    ///
    /// `AcqRel` so that the increment is ordered against the matching
    /// `complete_task` decrements on worker threads (the join wait reads the
    /// resulting count with `Acquire` in [`Self::is_drained`]).
    #[inline]
    pub(crate) fn register_task(&self) {
        self.pending.fetch_add(1, Ordering::AcqRel);
    }

    /// Mark one spawned task complete. Called from the task-body wrapper after
    /// the body has run (and any panic payload has been stored).
    ///
    /// ORDER IS LOAD-BEARING (Phase 9.2 Candidate U): `waker.unpark()` happens
    /// BEFORE `pending.fetch_sub`. While this task has not yet decremented,
    /// `pending >= 1`, so `Scope::drop`'s join cannot have observed zero and
    /// therefore cannot have freed the allocation — the `self.waker` read is
    /// sound. After the `fetch_sub`, this thread performs NO further access to
    /// `*self`; that decrement is its last byte-access to the allocation, so the
    /// joiner may deallocate the instant it observes zero (the single free site
    /// is `Scope::drop`). Multi-drain-safe: the free is tied to scope END, never
    /// to an intermediate wave's `pending -> 0` (the ECS executor drains
    /// `pending` to zero once per dispatch wave).
    ///
    /// The unpark is UNCONDITIONAL (no `prev == 1` gate): learning we are last
    /// would require reading `pending` after the sub — too late. An unconditional
    /// pre-decrement unpark is a cheap token store when the dispatcher runs; a
    /// spurious wake when it is parked is harmless (it re-checks `pending`). The
    /// rare lost-wakeup window is covered by the `park_timeout` backstops.
    ///
    /// `AcqRel` on the decrement is unchanged (loom-proven M1).
    #[inline]
    pub(crate) fn complete_task(&self) {
        self.waker.unpark();
        self.pending.fetch_sub(1, Ordering::AcqRel);
    }

    /// Returns `true` once every spawned task has completed. Polled by the
    /// work-stealing join wait in [`join_workers_until_drained`].
    ///
    /// `Acquire` pairs with the `AcqRel` decrement in [`Self::complete_task`]
    /// so that, when this observes zero, the completing tasks' writes are
    /// visible to the joiner.
    #[inline]
    pub(crate) fn is_drained(&self) -> bool {
        self.pending.load(Ordering::Acquire) == 0
    }
}

impl Drop for ScopeShared {
    /// Reclaims an uncollected panic payload.
    ///
    /// `Scope::drop` normally hands the payload off via [`take_panic`](ScopeShared::take_panic)
    /// before freeing the allocation, so this fires only when a `ScopeShared` is dropped without
    /// that hand-off — the `loom_exports` test wrapper constructs one directly. Without it a
    /// captured payload would leak; the previous `Mutex<Option<..>>` got this for free from
    /// `Option`'s drop glue, and dropping to a raw pointer gives up that guarantee unless it is
    /// restated here.
    #[inline]
    fn drop(&mut self) {
        let raw = *self.panic_payload.get_mut();
        if !raw.is_null() {
            // SAFETY: `&mut self` proves exclusive access. A non-null value here was published
            //   by one `capture_panic` CAS and never taken, so this is its unique owner; the
            //   pointer came from `Box::into_raw(Box::new(..))`, matching this `Box::from_raw`.
            drop(unsafe { Box::from_raw(raw) });
        }
    }
}

/// Send-marked raw pointer to `ScopeShared` used as a closure capture in
/// `Scope::spawn`. Raw pointers are `!Send` by default; we wrap them so
/// they can cross thread boundaries inside a task body.
///
/// The inner field is **private** so that closures cannot capture it
/// directly (Rust 2021+ disjoint-capture would otherwise see the inner
/// `*const ScopeShared` and reject the closure as `!Send`); access goes
/// through [`SharedPtr::as_ref`] which only operates on `&self`.
#[derive(Copy, Clone)]
struct SharedPtr {
    ptr: *const ScopeShared,
}

impl SharedPtr {
    #[inline]
    fn new(ptr: *const ScopeShared) -> Self {
        Self { ptr }
    }

    /// Borrow the pointee.
    ///
    /// # Safety
    /// The pointee must outlive `'a`. In `Scope::spawn` this is upheld
    /// because the Scope's `NonNull<ScopeShared>` allocation outlives every
    /// task body (Scope::Drop waits for completion before the single free).
    #[inline]
    unsafe fn as_ref<'a>(&self) -> &'a ScopeShared {
        // SAFETY: forwarded to the caller; see method doc.
        unsafe { &*self.ptr }
    }
}

// SAFETY:
//   The pointer references a `ScopeShared` heap allocation owned by the
//   originating `Scope`. The Scope's `Drop` blocks until every task that
//   captured the pointer has completed (via the work-stealing wait in
//   `join_workers_until_drained`), so the pointee outlives every use of
//   the pointer on any worker thread. The pointee type (`ScopeShared`)
//   contains only `Sync` interior — `AtomicUsize`, `Mutex`, `Thread` —
//   so concurrent access through the pointer is sound.
unsafe impl Send for SharedPtr {}

/// Fork/join scope over a [`ThreadPool`](crate::ThreadPool).
///
/// Constructed by [`ThreadPool::install`] or [`ThreadPool::scope`].
/// Spawn child tasks via [`Scope::spawn`]; the scope blocks at drop time
/// until every spawned task has completed.
///
/// [`ThreadPool::install`]: crate::ThreadPool::install
/// [`ThreadPool::scope`]: crate::ThreadPool::scope
pub struct Scope<'scope> {
    /// The pool's worker-shared state this scope spawns into. Borrowed for
    /// `'scope` (Phase 9.3b decision E: `&PoolInner`, not the handle — the
    /// handle keeps `inner` alive across the `install`/`scope` frame that
    /// borrows it, so this reference is valid for `'scope`).
    inner: &'scope PoolInner,
    /// Owned `ScopeShared` held as a raw `NonNull` (Phase 9.2 Candidate U —
    /// the Tree-Borrows-protector fix). `NonNull::as_ptr` is a `Copy` that
    /// copies the pointer WITHOUT retagging the pointee, so `Scope::drop`'s
    /// `&mut self` protector covers only this 8-byte field, never the heap
    /// allocation that worker threads concurrently write `pending` into.
    /// Created by `Box::into_raw` in [`Scope::new`], freed by `Box::from_raw`
    /// in [`Scope::drop`] (the single free site).
    pub(crate) shared: NonNull<ScopeShared>,
    /// `PhantomData<&'scope mut &'scope ()>` makes the scope invariant in
    /// `'scope`, which is what we want — `'scope` is a borrow window, not
    /// a covariant lifetime.
    _phantom: PhantomData<&'scope mut &'scope ()>,
}

impl<'scope> Scope<'scope> {
    #[inline]
    pub(crate) fn new(inner: &'scope PoolInner, shared: Box<ScopeShared>) -> Self {
        // Cold path: runs single-threaded before any task is spawned, so the
        // conversion never races a worker. `Box::into_raw` hands ownership of
        // the allocation to this `NonNull`; `Scope::drop` reclaims it.
        let shared = NonNull::new(Box::into_raw(shared))
            .expect("invariant: Box::into_raw never yields null");
        Self {
            inner,
            shared,
            _phantom: PhantomData,
        }
    }

    /// Spawn a child task. The closure may borrow data with lifetime
    /// `'scope`; the scope blocks at drop until the task completes, so
    /// the borrow remains valid.
    ///
    /// # Panic semantics
    /// A panic inside `f` is captured into the scope's panic payload and
    /// re-raised on the calling thread when the scope drops. The first
    /// panic wins; subsequent panics are dropped.
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'scope,
    {
        // SAFETY: `self.shared` points to the live `ScopeShared` allocation
        //   owned by this `Scope` (created in `new`, freed only by `drop`).
        //   `spawn` runs on the owner thread before the task is enqueued, so
        //   this shared reborrow does not race any worker.
        unsafe { self.shared.as_ref() }.register_task();

        // Raw pointer to ScopeShared — stable for the lifetime of the
        // allocation and therefore for the lifetime of every spawned task
        // (Scope::Drop blocks until they all complete before the single free).
        //
        // The raw pointer needs a Send wrapper because closures can't
        // capture `*const T` (it's `!Send`); the safety obligation is
        // documented on `SharedPtr` below. `NonNull::as_ptr` copies the
        // pointer without retagging the pointee (no protector over the
        // allocation — the Phase 9.2 Candidate U TB fix).
        let shared_ptr = SharedPtr::new(self.shared.as_ptr() as *const ScopeShared);

        // Wrap the user body so that:
        //   - We catch_unwind to keep panics inside the worker.
        //   - We store the first panic payload.
        //   - We unpark-then-decrement `pending` on completion (Candidate U).
        let wrapped = move || {
            let result = catch_unwind(AssertUnwindSafe(f));
            // SAFETY: `shared_ptr` names the live `ScopeShared` allocation
            //   (created in `Scope::new`, freed only by `Scope::drop` after the
            //   join). This is a transient shared reborrow on the worker thread;
            //   it MUST NOT outlive the `complete_task` call below — after that
            //   call's `pending.fetch_sub` the dispatcher may free the box.
            let shared = unsafe { shared_ptr.as_ref() };

            if let Err(payload) = result {
                shared.capture_panic(payload);
            }
            // - Multiple panics: first wins; subsequent payloads dropped (the losing CAS in
            //   `capture_panic` reclaims its own box).
            // - No poisoning to handle: the slot is an atomic, so a panicking task cannot leave
            //   it in a degraded state the way a `Mutex` guard could.

            // Last action: unpark-then-decrement. After this the worker
            // performs NO further access to the allocation (there is NO
            // worker-side `Box::from_raw` in Candidate U — the box is freed
            // solely by `Scope::drop`). `shared` MUST NOT be dereferenced after
            // this line.
            shared.complete_task();
        };

        // Box the body so the deque entry stays word-sized.
        let body_scoped: Box<dyn FnOnce() + Send + 'scope> = Box::new(wrapped);

        // SAFETY (lifetime erasure 'scope -> 'static):
        //   The closure body borrows data with lifetime 'scope. We
        //   transmute the trait object's lifetime to 'static so that it
        //   can be stored in `TaskHandle` (whose body type is
        //   `Box<dyn FnOnce() + Send + 'static>`). The transmute is
        //   sound because:
        //     - `Scope::drop` blocks until `pending == 0` via the
        //       work-stealing wait (`join_workers_until_drained`).
        //     - The blocking happens BEFORE any 'scope borrow can
        //       expire (Scope::drop runs while `'scope` is still live;
        //       the user's `install`/`scope` call frame still holds the
        //       borrow).
        //     - Even when the calling thread panics, Drop runs during
        //       unwinding (Rust's stack-unwinding semantics).
        //   The only edge case is `std::process::abort` / SIGKILL: if
        //   the process is terminated mid-task, workers may continue
        //   accessing freed stack frames in the brief window before the
        //   kernel reclaims memory. This is observable only at the
        //   language level; no real program can observe the UB because
        //   the process is gone. Same edge case as rayon's `scope`.
        let body_static: Box<dyn FnOnce() + Send + 'static> =
            unsafe { core::mem::transmute(body_scoped) };

        push_task(self.inner, TaskHandle::new(body_static));
    }
}

impl<'scope> Drop for Scope<'scope> {
    fn drop(&mut self) {
        // `NonNull::as_ptr` is a `Copy` that copies the pointer WITHOUT
        // retagging the pointee, so this Drop's `&mut self` protector covers
        // only the 8-byte `shared` field, never the heap allocation that worker
        // threads write `pending` into (the Phase 9.2 Candidate U TB fix).
        let raw: *mut ScopeShared = self.shared.as_ptr();

        // SAFETY: `raw` is live for the whole join — it is freed only by the
        //   single `Box::from_raw` below, which runs after this returns. The
        //   join reborrows `*raw` only per-poll for one `Acquire` load, never
        //   forming a reference that spans a worker's `pending` write (the
        //   raw-pointer / NonNull design). The `*mut` coerces to the `*const`
        //   parameter.
        unsafe { join_workers_until_drained(self.inner, raw) };

        // The join returned ⇒ `pending == 0` (the final wave's decrement). No
        // worker will start a new `complete_task`, and every worker that ran
        // has completed its `fetch_sub` (its last access to the allocation),
        // which happens-before the join's `Acquire` load. The dispatcher is now
        // the sole owner.
        //
        // SAFETY: pre-free shared access through the raw pointer. `is_drained`
        //   is an `Acquire` load and `panic_payload` is an `AtomicPtr` (Sync). The
        //   payload is taken BEFORE the free so that no `*raw` access follows
        //   the deallocation.
        debug_assert!(
            unsafe { (*raw).is_drained() },
            "Scope::Drop returned with pending tasks still in flight"
        );
        // No lock on the common path: a scope whose tasks all completed reads a null pointer.
        let payload = unsafe { (*raw).take_panic() };

        // SAFETY (the single free site — Phase 9.2 Candidate U):
        //   - `raw` is the `Box::into_raw` address minted in `Scope::new`.
        //   - The dispatcher is the unique remaining owner: the join observed
        //     `pending == 0`, and every worker's last allocation access — its
        //     `pending.fetch_sub` — happens-before the join's `Acquire` load.
        //   - The payload was taken above, before this free; no `*raw` access
        //     follows. Reached once (Drop runs once), unconditionally, so the
        //     allocation is freed exactly once — no double-free, multi-drain-safe
        //     (the free is tied to scope END, never to an intermediate wave's
        //     `pending -> 0`).
        unsafe { drop(Box::from_raw(raw)) };

        // Re-raise OUTSIDE any `*raw` access (the payload is a moved-out stack
        // local that no longer aliases the freed allocation).
        if let Some(p) = payload {
            resume_unwind(p);
        }
    }
}

/// Block (with work stealing) until `shared.pending` is zero. Plan
/// §4.5.5.
///
/// This function is called from `Scope::Drop`. It steals work from any
/// stealable source (the calling worker's local injector if on a worker;
/// the global injector; any sibling stealer) and runs the stolen tasks
/// inline on the calling thread. When no work is stealable, it parks
/// with a short timeout — the timeout serves as a re-poll trigger; the
/// real wake-up arrives via `shared.waker.unpark()` from a completing
/// task (Phase 9.2 Candidate U: unpark precedes the decrement, so the
/// timeout is also the backstop for the rare lost-wakeup window).
///
/// # Safety
/// `shared` must point to a live `ScopeShared` allocation that remains
/// valid for the entire duration of this call. The caller (`Scope::drop`)
/// upholds this: the allocation is freed only by the single `Box::from_raw`
/// that runs after this function returns. The function reborrows `*shared`
/// only per-poll for one `Acquire` load (`is_drained`), never forming a
/// reference that spans a worker's `pending` write.
unsafe fn join_workers_until_drained(inner: &PoolInner, shared: *const ScopeShared) {
    let wid = tls::current_worker_id();
    let on_worker = (wid as usize) < inner.injector_local.len();
    let local_inj_idx = wid as usize;

    // A temporary deque to receive batches stolen from injectors/sibling
    // stealers. Not exposed; lives on the dispatcher's stack frame for
    // the duration of this wait.
    let scratch: Worker<TaskHandle> = Worker::new_fifo();

    let backoff = Backoff::new();

    loop {
        // Under Miri the scheduler is cooperative: a dispatcher that steals,
        // runs, and `continue`s without ever reaching the backoff/park branch
        // would starve the workers. Yield each iteration so Miri can advance
        // the other threads. Compiles to nothing natively (Phase 9.1 H2).
        #[cfg(miri)]
        std::thread::yield_now();

        // SAFETY: per the function contract, `shared` is live for this whole
        //   call; this is a transient `Acquire` load that does not span a
        //   worker write.
        if unsafe { (*shared).is_drained() } {
            return;
        }

        // 1. If we're on a worker, drain inner-spawn tasks targeted at us.
        // Every stolen task below runs through `worker::run_task`, NOT a bare
        // `t.run()` — 2026-07 audit finding. These queues carry BOTH scope-spawned
        // tasks (self-wrapped in `catch_unwind` at `Scope::spawn`, so the guard is
        // inert for them) AND fire-and-forget `ThreadPool::spawn` tasks, which are
        // wrapped NOWHERE. A bare unwind here escapes `Drop for Scope` and abandons
        // the join that the `'scope -> 'static` transmute below depends on, freeing
        // the caller's frame while spawned bodies still borrow it (UAF from safe
        // code). `run_task` applies the same abort-on-fire-and-forget-panic policy
        // the worker loop already applies to the identical task type.
        if on_worker
            && let Some(t) =
                drain_one(|| inner.injector_local[local_inj_idx].steal_batch_and_pop(&scratch))
        {
            crate::worker::run_task(t);
            drain_scratch(&scratch);
            backoff.reset();
            continue;
        }

        // 2. Global injector.
        if let Some(t) = drain_one(|| inner.injector_global.steal_batch_and_pop(&scratch)) {
            crate::worker::run_task(t);
            drain_scratch(&scratch);
            backoff.reset();
            continue;
        }

        // 3. Sibling steal — any worker, any deque.
        let stolen = try_steal_any(inner, &scratch);
        if let Some(t) = stolen {
            crate::worker::run_task(t);
            drain_scratch(&scratch);
            backoff.reset();
            continue;
        }

        // 4. Nothing to do. Either we exhaust backoff and park-with-
        //    timeout, or we snooze and loop.
        if backoff.is_completed() {
            // Wake one idle worker before parking: a sibling that just
            // pushed inner tasks into its own local injector may not
            // have raced through unpark_one_idle yet, but the work IS
            // visible — letting that worker grab it is also valid.
            unpark_one_idle(inner);
            std::thread::park_timeout(Duration::from_micros(50));
            backoff.reset();
        } else {
            backoff.snooze();
        }
    }
}

/// Drain anything left in our local scratch deque (we may have stolen a
/// batch where only the first task is returned and the rest live in
/// `scratch`). We run them all inline.
#[inline]
fn drain_scratch(scratch: &Worker<TaskHandle>) {
    while let Some(t) = scratch.pop() {
        // Same abort-guard reason as the steal sites above: a batch stolen from
        // `injector_global` can contain fire-and-forget tasks, and an unwind out
        // of here abandons the join (2026-07 audit finding).
        crate::worker::run_task(t);
    }
}

/// Try to steal a task from any sibling deque, ignoring nothing.
fn try_steal_any(inner: &PoolInner, scratch: &Worker<TaskHandle>) -> Option<TaskHandle> {
    for stealer in inner.stealers.iter() {
        if let Some(t) = drain_one(|| stealer.steal_batch_and_pop(scratch)) {
            return Some(t);
        }
    }
    None
}

#[inline]
fn drain_one<F>(mut f: F) -> Option<TaskHandle>
where
    F: FnMut() -> Steal<TaskHandle>,
{
    loop {
        match f() {
            Steal::Success(t) => return Some(t),
            Steal::Empty => return None,
            Steal::Retry => {
                // Miri-only cooperative yield in this unbounded steal-retry
                // loop (Phase 9.1 H2). Byte-identical native: compiles away.
                #[cfg(miri)]
                std::thread::yield_now();
                continue;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ThreadPoolBuilder;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, AtomicUsize};

    #[test]
    fn scope_drain_with_no_tasks_is_noop() {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        pool.install(|_scope| {
            // no-op
        });
    }

    #[test]
    fn scope_spawn_can_borrow_stack_data() {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let counter = AtomicU32::new(0);
        pool.install(|scope| {
            for _ in 0..32 {
                scope.spawn(|| {
                    counter.fetch_add(1, Ordering::Relaxed);
                });
            }
        });
        assert_eq!(counter.load(Ordering::Acquire), 32);
    }

    #[test]
    fn scope_propagates_panic() {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let arc_pool = pool;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            arc_pool.install(|scope| {
                scope.spawn(|| panic!("planned"));
            });
        }));
        assert!(result.is_err(), "panic should propagate out of install");
    }

    #[test]
    fn nested_scope_does_not_deadlock() {
        let pool = ThreadPoolBuilder::new().num_threads(4).build();
        let counter = Arc::new(AtomicU32::new(0));
        let pool_for_outer = Arc::clone(&pool);

        pool.install(|outer| {
            for _ in 0..4 {
                let c = Arc::clone(&counter);
                let inner_pool = Arc::clone(&pool_for_outer);
                outer.spawn(move || {
                    inner_pool.scope(|inner| {
                        for _ in 0..8 {
                            let c2 = Arc::clone(&c);
                            inner.spawn(move || {
                                c2.fetch_add(1, Ordering::Relaxed);
                            });
                        }
                    });
                });
            }
        });

        assert_eq!(counter.load(Ordering::Acquire), 32);
    }

    /// Phase 9.2 Candidate U multi-drain regression (§11.4).
    ///
    /// Drives a SINGLE scope through several waves where `ScopeShared.pending`
    /// returns toward zero between waves *before* the scope drops — the pattern
    /// the deleted `free_state` handshake mis-freed on (it elected a freer on
    /// every `pending -> 0`, double-freeing on wave 2). Candidate U ties the
    /// single `Box::from_raw` to `Scope::drop` (scope END) alone, so repeated
    /// intermediate drains are harmless.
    ///
    /// Each wave's tasks are made to finish before the next wave spawns by
    /// spinning a bounded number of yields until the wave's `done` counter
    /// reaches `PER_WAVE` — by which point those tasks have called
    /// `complete_task` (unpark + `fetch_sub`), returning `pending` toward 0.
    /// If the scope leaked or double-freed, the native allocator (and the
    /// stress-test Drop accounting / Miri under the orchestrator) catch it.
    ///
    /// `done` is an `Arc<AtomicUsize>` (heap, `'static`-capable) rather than a
    /// per-wave stack local: `scope.spawn` requires the body to outlive
    /// `'scope`, so a fresh borrow created inside the `install` closure cannot
    /// be captured by spawned tasks — the `Arc` clone is the correct in-scope
    /// approximation of the executor's between-wave drive.
    #[test]
    fn scope_multi_drain_frees_once() {
        const WAVES: usize = 8;
        const PER_WAVE: usize = 4;

        let pool = ThreadPoolBuilder::new().num_threads(4).build();
        let done = Arc::new(AtomicUsize::new(0));
        let done_main = Arc::clone(&done);

        pool.install(move |scope| {
            for wave in 0..WAVES {
                for _ in 0..PER_WAVE {
                    let d = Arc::clone(&done_main);
                    scope.spawn(move || {
                        d.fetch_add(1, Ordering::Relaxed);
                    });
                }

                // Let THIS wave drain (its tasks reach `complete_task`, driving
                // `pending` back toward 0) before spawning the next wave, so the
                // scope sees several `pending -> 0` transitions over its life.
                // Bounded yields — never an unbounded spin — so a stuck wave
                // fails fast instead of hanging.
                let target = (wave + 1) * PER_WAVE;
                let mut spins = 0u32;
                while done_main.load(Ordering::Acquire) < target && spins < 10_000_000 {
                    std::thread::yield_now();
                    spins += 1;
                }
            }
        });
        // <-- the ONLY free, here at Scope::drop. A per-wave free would have
        //     double-freed above.

        assert_eq!(
            done.load(Ordering::Acquire),
            WAVES * PER_WAVE,
            "every wave's tasks must have run exactly once"
        );
    }
}
