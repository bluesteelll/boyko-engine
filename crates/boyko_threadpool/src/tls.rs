//! Thread-local state for the pool.
//!
//! Phase 9 plan §2.7 (allocation discipline ALLOC1/ALLOC6) and §2.8 (event
//! lane TLS EVT1) both rely on per-thread flags maintained here. The ECS
//! crate consumes these helpers without depending on internal pool types.
//!
//! Phase 9.3b (decision E): the active-pool TLS pointer is a
//! `*const PoolInner` (the worker-shared state behind `Arc<PoolInner>`),
//! NOT a `*const ThreadPool` (the user-facing handle). A worker holds only
//! `Arc<PoolInner>` and must never resurrect the handle, so it deposits the
//! `PoolInner` pointer. `PoolInner` is opaque `pub`; consumers
//! (`par_iter`/`par_chunk`) only call its `num_threads()`/`scope()`.

use core::cell::Cell;
use core::ptr;

use crate::thread_pool::PoolInner;

/// Sentinel for the dispatcher thread (the calling thread inside
/// [`ThreadPool::install`]). Distinct from worker ids so that EVT1's lane
/// router can place dispatcher writes on an extra lane (`worker_count`).
///
/// [`ThreadPool::install`]: crate::ThreadPool::install
pub const WORKER_ID_DISPATCHER: u32 = u32::MAX - 1;

/// Sentinel for "no associated worker / dispatcher". Default state for
/// threads that never entered an install scope.
pub const WORKER_ID_UNATTACHED: u32 = u32::MAX;

thread_local! {
    /// Active pool pointer for ambient `par_iter` dispatch. Set by
    /// [`ThreadPool::install`](crate::ThreadPool::install) entry / cleared on
    /// exit. Worker threads also have this set on `worker_main` entry. Null
    /// when no pool is attached. Points at the shared [`PoolInner`], never the
    /// handle (decision E).
    pub(crate) static ACTIVE_POOL: Cell<*const PoolInner> = const { Cell::new(ptr::null()) };

    /// Current worker id.
    /// - `0..MAX_WORKERS-1` — running on worker `N`.
    /// - [`WORKER_ID_DISPATCHER`] — running on the dispatcher thread inside
    ///   `ThreadPool::install`.
    /// - [`WORKER_ID_UNATTACHED`] — not on a worker, not in an install scope.
    pub(crate) static CURRENT_WORKER_ID: Cell<u32> = const { Cell::new(WORKER_ID_UNATTACHED) };

    /// Allocation discipline guard (ALLOC1). Set true by the worker's
    /// run-system RAII guard; the ECS crate's context-restricted paths
    /// `debug_assert!` it (or its negation). Reset by
    /// [`InSystemRunGuard::drop`].
    pub(crate) static IN_SYSTEM_RUN: Cell<bool> = const { Cell::new(false) };
}

/// Returns the current worker id, or [`WORKER_ID_DISPATCHER`] /
/// [`WORKER_ID_UNATTACHED`] sentinel when not on a worker.
#[inline]
pub fn current_worker_id() -> u32 {
    CURRENT_WORKER_ID.with(|c| c.get())
}

/// Returns the lane index for `EventDispatcher::send_event`:
/// - The worker's own id when on a worker (`0..worker_count-1`).
/// - `worker_count` when on the dispatcher (an extra lane reserved at
///   `EventConfig::default_for(worker_count + 1)` time — see plan §2.8 EVT1).
/// - `0` when unattached (default lane for non-scheduler call sites).
///
/// `worker_count` MUST be passed by the caller because the TLS doesn't know
/// the pool's worker count (and we deliberately avoid a TLS pool pointer
/// dereference on the event hot path).
#[inline]
pub fn current_worker_id_or_dispatcher_lane(worker_count: u32) -> u32 {
    let id = current_worker_id();
    if id == WORKER_ID_DISPATCHER {
        worker_count
    } else if id == WORKER_ID_UNATTACHED {
        0
    } else {
        id
    }
}

/// Returns `true` when the current thread is executing inside a system body
/// (between [`InSystemRunGuard::enter`] and the guard's drop).
#[inline]
pub fn is_in_system_run() -> bool {
    IN_SYSTEM_RUN.with(|c| c.get())
}

/// Set the worker id for the current thread. Called once on `worker_main`
/// entry; not intended for user code.
#[inline]
pub(crate) fn set_current_worker_id(id: u32) {
    CURRENT_WORKER_ID.with(|c| c.set(id));
}

/// Clear the worker id on the current thread back to
/// [`WORKER_ID_UNATTACHED`]. Reserved for use by code paths that must
/// detach a thread from the pool (e.g. test teardown); production
/// `install` restores the previous value via `set_current_worker_id` so
/// it does not call this directly.
#[inline]
#[allow(dead_code)]
pub(crate) fn clear_current_worker_id() {
    CURRENT_WORKER_ID.with(|c| c.set(WORKER_ID_UNATTACHED));
}

/// Replace the active-pool pointer; returns the previous value (so the
/// caller can restore it on exit — see `install`).
#[inline]
pub(crate) fn swap_active_pool(new: *const PoolInner) -> *const PoolInner {
    ACTIVE_POOL.with(|c| {
        let prev = c.get();
        c.set(new);
        prev
    })
}

/// Read the current active-pool pointer without modifying it.
#[inline]
pub(crate) fn active_pool_ptr() -> *const PoolInner {
    ACTIVE_POOL.with(|c| c.get())
}

/// Borrow the current active pool for the duration of `f`. Returns `None`
/// when no pool is attached to the current thread.
///
/// The borrow is bracketed by the closure call — the `&PoolInner` reference
/// MUST NOT escape `f` (the function signature prevents this at compile
/// time). Wave 6 `Query::par_iter` uses this to discover the ambient pool
/// without an explicit pool argument on every cursor.
#[inline]
pub fn try_with_active_pool<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&PoolInner) -> R,
{
    let p = active_pool_ptr();
    if p.is_null() {
        None
    } else {
        // SAFETY: `ACTIVE_POOL` is set by `ThreadPool::install`/`scope`
        //   immediately before `f(&scope)` runs and restored after
        //   `Scope::Drop` returns (which itself blocks until every spawned
        //   task completes). On a worker thread it is set at `worker_main`
        //   entry from the worker's own `Arc<PoolInner>` and lives until the
        //   worker returns (the handle joins every worker before dropping its
        //   `Arc<PoolInner>`, so the pointee outlives the deposit). Any thread
        //   that observes a non-null pointer is therefore inside a frame on
        //   the same thread (TLS is per-thread) whose `PoolInner` is live for
        //   the duration of this closure. `PoolInner` is reached only behind
        //   `Arc` and is never borrowed `&mut` (it is dropped via `Arc`'s
        //   internal `&mut` at refcount 0, after all workers join), so no
        //   `&mut` protector ever spans this shared `&PoolInner`. The closure
        //   cannot capture the borrow because of the `FnOnce(&PoolInner) -> R`
        //   signature; no aliasing escape.
        Some(f(unsafe { &*p }))
    }
}

/// RAII guard around a worker's system-body execution. Sets `IN_SYSTEM_RUN`
/// on entry, clears on drop. The ECS scheduler wraps `System::run_unsafe`
/// in `let _g = InSystemRunGuard::enter();` so context-restricted paths can
/// `debug_assert!` whether they run inside a system body (ALLOC6).
pub struct InSystemRunGuard {
    /// Prevents the guard from being constructible outside `enter`.
    _private: (),
}

impl InSystemRunGuard {
    /// Enter a system run. Panics in debug builds if a previous guard is
    /// still live on this thread (nested system runs are a contract
    /// violation under SCH7).
    #[inline]
    pub fn enter() -> Self {
        IN_SYSTEM_RUN.with(|c| {
            debug_assert!(!c.get(), "InSystemRunGuard nested; SCH7 violation");
            c.set(true);
        });
        Self { _private: () }
    }
}

impl Drop for InSystemRunGuard {
    #[inline]
    fn drop(&mut self) {
        IN_SYSTEM_RUN.with(|c| c.set(false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unattached_thread_sees_sentinel() {
        assert_eq!(current_worker_id(), WORKER_ID_UNATTACHED);
    }

    #[test]
    fn dispatcher_lane_for_unattached_is_zero() {
        // The "0" mapping for unattached threads is documented in EVT1.
        assert_eq!(current_worker_id_or_dispatcher_lane(8), 0);
    }

    #[test]
    fn in_system_run_guard_round_trip() {
        assert!(!is_in_system_run());
        {
            let _g = InSystemRunGuard::enter();
            assert!(is_in_system_run());
        }
        assert!(!is_in_system_run());
    }

    #[test]
    fn set_clear_worker_id_round_trip() {
        set_current_worker_id(3);
        assert_eq!(current_worker_id(), 3);
        // Lane for worker 3 with 8 workers => 3.
        assert_eq!(current_worker_id_or_dispatcher_lane(8), 3);
        clear_current_worker_id();
        assert_eq!(current_worker_id(), WORKER_ID_UNATTACHED);
    }
}
