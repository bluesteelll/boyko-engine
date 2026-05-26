//! [`ThreadPool`] — the public face of the work-stealing pool.
//!
//! Layout follows plan §4.2. Hot atomics live in [`CachePadded`] cells to
//! avoid false sharing on push/wake paths. Each worker exposes a
//! [`Stealer`] in a global registry, plus a local [`Injector`] that other
//! workers / the dispatcher can target for cache-friendly enqueuing
//! (plan §2.7 / Round 2 C2).

use core::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle, Thread};

use crossbeam_deque::{Injector, Stealer, Worker};
use crossbeam_utils::CachePadded;

use crate::scope::{Scope, ScopeShared};
use crate::tls;
use crate::worker::worker_main;

/// Maximum supported worker count.
///
/// The idle bitset is a single `AtomicU64`; bit `i` represents worker `i`.
/// 64 workers covers every consumer CPU and most server CPUs at the time
/// of writing. Larger bitsets require a multi-word implementation
/// (deferred — see plan §4.6 future work).
pub const MAX_WORKERS: usize = 64;

/// One unit of work submitted to the pool. `body` is the actual closure;
/// it is heap-allocated so that the deque entries remain word-sized.
///
/// The `Send` bound on `body` is sufficient at the type level; the pool
/// upholds the 'static erasure via [`Scope::Drop`]'s blocking contract
/// (plan §4.5.6).
pub struct TaskHandle {
    pub(crate) body: Box<dyn FnOnce() + Send + 'static>,
}

impl TaskHandle {
    /// Wrap a closure into a heap-allocated task handle. Used by
    /// [`Scope::spawn`] and [`ThreadPool::spawn`].
    #[inline]
    pub(crate) fn new(body: Box<dyn FnOnce() + Send + 'static>) -> Self {
        Self { body }
    }

    /// Consume the handle and run the closure.
    #[inline]
    pub(crate) fn run(self) {
        (self.body)();
    }
}

/// Per-worker control block. Cache-line padded so that idle/unpark traffic
/// on worker `i` doesn't false-share with worker `j`.
#[repr(C)]
pub struct WorkerHandle {
    /// `std::thread::Thread` handle for `unpark`. Stable for the lifetime
    /// of the worker (set once at spawn).
    pub(crate) thread: Thread,

    /// Park-counter / sticky-unpark coordination state. Reserved for
    /// future loom-tested optimizations; the production wake-up protocol
    /// today relies on the OS's sticky `park`/`unpark` semantics plus the
    /// `idle` bitset on the pool.
    pub(crate) park_state: CachePadded<AtomicU64>,
}

impl WorkerHandle {
    #[inline]
    pub(crate) fn new(thread: Thread) -> Self {
        Self {
            thread,
            park_state: CachePadded::new(AtomicU64::new(0)),
        }
    }
}

/// Internal record kept alongside each spawned worker so that
/// [`ThreadPool::drop`] can join cleanly. Not exposed.
struct WorkerJoin {
    handle: JoinHandle<()>,
}

/// Custom Chase-Lev work-stealing thread pool.
///
/// See the crate docs for the Wave 1 feature set. Construct via
/// [`ThreadPoolBuilder::build`]; the pool is wrapped in an [`Arc`] so that
/// worker threads can hold a reference for their entire lifetime.
#[repr(C)]
pub struct ThreadPool {
    /// Global injector. The dispatcher (or any non-worker thread) pushes
    /// here; workers drain it in stage 2 of `worker_main`.
    pub(crate) injector_global: CachePadded<Injector<TaskHandle>>,

    /// Per-worker local injectors. A worker pushes inner-spawn tasks to
    /// `injector_local[worker_id]` for cache locality; siblings still see
    /// them via the local-injector poll in stage 1.5 of `worker_main`.
    pub(crate) injector_local: Arc<[CachePadded<Injector<TaskHandle>>]>,

    /// Per-worker stealers. Index `i` is the [`Stealer`] for worker `i`'s
    /// Chase-Lev deque. Workers steal from siblings in randomized order.
    pub(crate) stealers: Arc<[Stealer<TaskHandle>]>,

    /// Per-worker handles (thread handle + park state).
    pub(crate) workers: Arc<[CachePadded<WorkerHandle>]>,

    /// Idle bitset. Bit `i` is 1 iff worker `i` is parked or about to
    /// park. Lock-free via `fetch_or` / `fetch_and` / `compare_exchange`.
    pub(crate) idle: CachePadded<AtomicU64>,

    /// Counter of active install/scope frames. Used by `Drop` to assert
    /// no scope outlives the pool (drop while `> 0` is a contract bug).
    pub(crate) active_scopes: CachePadded<AtomicUsize>,

    /// Shutdown flag. Set in `Drop` before unparking all workers.
    pub(crate) shutdown: CachePadded<AtomicBool>,

    /// Worker count (cold; written once at construction).
    worker_count: u32,

    /// Join handles for the worker threads. Wrapped in a `Mutex` only for
    /// the cold drop path; never touched on hot paths.
    join_handles: std::sync::Mutex<Vec<WorkerJoin>>,
}

impl ThreadPool {
    /// Returns the number of worker threads in the pool.
    #[inline]
    pub fn worker_count(&self) -> u32 {
        self.worker_count
    }

    /// Convenience alias mirroring the plan's `num_threads()` naming.
    #[inline]
    pub fn num_threads(&self) -> usize {
        self.worker_count as usize
    }

    /// Read the active pool for the current thread (the one set by the
    /// most recent [`ThreadPool::install`] on this thread). Returns
    /// `None` when no pool is attached.
    ///
    /// Wave 6 / `par_iter` uses this to grab the ambient pool without an
    /// explicit reference parameter on the iterator.
    #[inline]
    pub fn current_pool() -> Option<NonNull<ThreadPool>> {
        let p = tls::active_pool_ptr();
        // SAFETY: `tls::active_pool_ptr` returns either a null pointer or
        // a pointer that was deposited by `install`/`scope` while a live
        // `&ThreadPool` was on the stack. The caller receives a `NonNull`
        // wrapper but the dereference is the caller's responsibility; the
        // `NonNull` itself is sound to construct from a non-null pointer.
        NonNull::new(p as *mut ThreadPool)
    }

    /// Push a task onto the pool from outside any scope. Intended for
    /// fire-and-forget work that does not require join semantics. The
    /// 'static bound is the absence of any borrowing; for scoped work see
    /// [`ThreadPool::install`] / [`ThreadPool::scope`].
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let task = TaskHandle::new(Box::new(f));
        crate::worker::push_task(self, task);
    }

    /// Block the calling thread, running `f` to completion. The closure
    /// receives a [`Scope`] that can spawn child tasks; the function
    /// returns only after every spawned task has completed (work-stealing
    /// while waiting — see [`Scope::drop`](Scope)).
    ///
    /// Sets the calling thread's TLS pool pointer for the duration so that
    /// nested `par_iter` calls can find the ambient pool, and sets the
    /// worker-id sentinel to [`WORKER_ID_DISPATCHER`].
    ///
    /// [`WORKER_ID_DISPATCHER`]: crate::WORKER_ID_DISPATCHER
    pub fn install<'scope, F, R>(&'scope self, f: F) -> R
    where
        F: FnOnce(&Scope<'scope>) -> R + Send,
        R: Send,
    {
        self.active_scopes.fetch_add(1, Ordering::AcqRel);

        let prev_pool = tls::swap_active_pool(self as *const _);
        let prev_worker_id = {
            let cur = tls::current_worker_id();
            tls::set_current_worker_id(tls::WORKER_ID_DISPATCHER);
            cur
        };

        let shared = Box::new(ScopeShared::new(thread::current()));
        // SAFETY (scope lifetime erasure to '_):
        //   The scope is dropped before this function returns; Drop
        //   blocks until every spawned task has completed (or has
        //   panicked and been captured into `shared.panic_payload`).
        //   No task body outlives the scope.
        let scope = Scope::new(self, shared);

        let result = f(&scope);
        // Explicit drop so the order is unambiguous: scope drains first,
        // TLS restored afterwards. Otherwise the compiler is free to drop
        // `scope` after `result` is moved, which would still be correct
        // but harder to reason about.
        drop(scope);

        tls::set_current_worker_id(prev_worker_id);
        tls::swap_active_pool(prev_pool);

        self.active_scopes.fetch_sub(1, Ordering::AcqRel);
        result
    }

    /// Re-entrant scope creation. Cheaper than [`install`](Self::install)
    /// because it does not touch the active-pool / worker-id TLS — the
    /// caller is assumed to already be inside an `install` frame
    /// (typically a worker task body or the dispatcher running through
    /// `Schedule::run`).
    ///
    /// Used by `Query::par_iter` (Wave 6).
    pub fn scope<'scope, F, R>(&'scope self, f: F) -> R
    where
        F: FnOnce(&Scope<'scope>) -> R + Send,
        R: Send,
    {
        debug_assert!(
            !tls::active_pool_ptr().is_null(),
            "ThreadPool::scope called without an active pool; use install instead"
        );

        self.active_scopes.fetch_add(1, Ordering::AcqRel);

        let shared = Box::new(ScopeShared::new(thread::current()));
        let scope = Scope::new(self, shared);

        let result = f(&scope);
        drop(scope);

        self.active_scopes.fetch_sub(1, Ordering::AcqRel);
        result
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        debug_assert_eq!(
            self.active_scopes.load(Ordering::Acquire),
            0,
            "ThreadPool dropped with active scopes still in flight"
        );

        // Publish shutdown to every worker BEFORE unparking — workers
        // re-check this flag after wakeup. Release pairs with the worker's
        // Acquire load in `worker_main`.
        self.shutdown.store(true, Ordering::Release);

        for w in self.workers.iter() {
            w.thread.unpark();
        }

        // Join every worker. We acquire the lock once and drain.
        let mut guard = self
            .join_handles
            .lock()
            .expect("invariant: join_handles mutex never poisoned by us");
        for j in guard.drain(..) {
            // We deliberately ignore the result — a panicking worker has
            // already been observed via the scope's panic_payload path;
            // join here would only surface "thread aborted unexpectedly".
            let _ = j.handle.join();
        }
    }
}

// SAFETY (Send/Sync for ThreadPool):
//   Every field is either trivially Send/Sync (Arc, atomics, Mutex,
//   plain `u32`) or a `CachePadded<...>` wrapper around such a type.
//   `Injector<TaskHandle>`, `Stealer<TaskHandle>`, and `Worker<TaskHandle>`
//   are Send/Sync per crossbeam-deque's public contracts (verified in
//   crossbeam-deque 0.8 docs). `TaskHandle::body` is `Box<dyn FnOnce + Send
//   + 'static>` so the queues' element types are Send. No interior
//   `!Send`/`!Sync` field exists.
//
//   ThreadPool itself MUST be Send + Sync because (a) the
//   `Arc<ThreadPool>` is shared between the spawning thread and every
//   worker thread, and (b) `Schedule` (Wave 3+) borrows `&ThreadPool` for
//   `install` from arbitrary system contexts.
//
//   The auto-derive already gives us Send/Sync for this struct (no raw
//   pointers / no UnsafeCell directly held), so this comment is purely
//   documentary — we do NOT write `unsafe impl`.

/// Fluent builder for [`ThreadPool`]. Defaults: thread count =
/// `std::thread::available_parallelism()`, no affinity, no custom stack
/// size, name prefix `"boyko-worker"`.
pub struct ThreadPoolBuilder {
    num_threads: Option<usize>,
    affinity: bool,
    stack_size: Option<usize>,
    thread_name_prefix: Option<String>,
}

impl Default for ThreadPoolBuilder {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadPoolBuilder {
    /// Construct a builder with all options unset.
    #[inline]
    pub fn new() -> Self {
        Self {
            num_threads: None,
            affinity: false,
            stack_size: None,
            thread_name_prefix: None,
        }
    }

    /// Override the number of worker threads. Clamped to `[1, MAX_WORKERS]`
    /// at `build()` time.
    #[inline]
    pub fn num_threads(mut self, n: usize) -> Self {
        self.num_threads = Some(n);
        self
    }

    /// Toggle worker CPU affinity. **Wave 1: no-op stub.** Real pinning
    /// will land in a later Wave (plan §4 Q7 — currently default off).
    #[inline]
    pub fn pin_workers(mut self, on: bool) -> Self {
        self.affinity = on;
        self
    }

    /// Override the worker thread stack size in bytes.
    #[inline]
    pub fn stack_size(mut self, bytes: usize) -> Self {
        self.stack_size = Some(bytes);
        self
    }

    /// Set the thread name prefix. Default `"boyko-worker"`. Workers are
    /// named `<prefix>-<id>`.
    #[inline]
    pub fn thread_name_prefix(mut self, s: impl Into<String>) -> Self {
        self.thread_name_prefix = Some(s.into());
        self
    }

    /// Spawn worker threads and return the pool. The returned [`Arc`] is
    /// the canonical handle — workers hold their own copies internally so
    /// that the pool stays alive at least until every worker exits.
    pub fn build(self) -> Arc<ThreadPool> {
        let requested = self.num_threads.unwrap_or_else(default_worker_count);
        let worker_count = requested.clamp(1, MAX_WORKERS);

        // Crossbeam workers must be constructed up-front so that we can
        // publish the corresponding stealers into a shared registry before
        // any worker_main runs. We hand the `Worker<TaskHandle>` to the
        // worker thread by move (it's not Sync).
        let mut deques: Vec<Worker<TaskHandle>> = Vec::with_capacity(worker_count);
        let mut stealers: Vec<Stealer<TaskHandle>> = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let w: Worker<TaskHandle> = Worker::new_fifo();
            stealers.push(w.stealer());
            deques.push(w);
        }
        let stealers: Arc<[Stealer<TaskHandle>]> = stealers.into();

        let mut injector_local_vec: Vec<CachePadded<Injector<TaskHandle>>> =
            Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            injector_local_vec.push(CachePadded::new(Injector::new()));
        }
        let injector_local: Arc<[CachePadded<Injector<TaskHandle>>]> = injector_local_vec.into();

        // Workers are created lazily — we need their `Thread` handles to
        // populate `pool.workers`, which means we must spawn the threads,
        // capture their handles, and somehow let them learn their own
        // `Arc<ThreadPool>`. The classical trick: spawn threads that block
        // on a one-shot channel; the parent constructs the pool, then
        // sends the Arc through the channels. We use a simpler shape
        // because every worker just needs `pool.clone()` after construction
        // — defer worker_main by sending the pool through a dedicated
        // `crossbeam_deque` channel? — overkill. We use a `Once`-style
        // handshake via `Arc<Mutex<Option<Arc<ThreadPool>>>>`.
        //
        // Alternative considered: pre-create `WorkerHandle`s with bogus
        // Thread handles and patch them later. Rejected because Thread
        // doesn't expose a no-op constructor.
        let bootstrap: Arc<std::sync::Mutex<Option<Arc<ThreadPool>>>> =
            Arc::new(std::sync::Mutex::new(None));
        let bootstrap_cvar: Arc<std::sync::Condvar> = Arc::new(std::sync::Condvar::new());

        let mut handles: Vec<JoinHandle<()>> = Vec::with_capacity(worker_count);
        let mut thread_handles: Vec<Thread> = Vec::with_capacity(worker_count);

        let name_prefix = self
            .thread_name_prefix
            .unwrap_or_else(|| "boyko-worker".to_string());
        let stack_size = self.stack_size;

        for (worker_id, deque) in deques.into_iter().enumerate() {
            let bootstrap_cl = Arc::clone(&bootstrap);
            let cvar_cl = Arc::clone(&bootstrap_cvar);
            let name = format!("{name_prefix}-{worker_id}");

            let mut builder = thread::Builder::new().name(name);
            if let Some(sz) = stack_size {
                builder = builder.stack_size(sz);
            }

            let join = builder
                .spawn(move || {
                    // Wait for the parent to publish the Arc<ThreadPool>.
                    let pool = {
                        let mut guard = bootstrap_cl
                            .lock()
                            .expect("invariant: bootstrap mutex never poisoned");
                        while guard.is_none() {
                            guard = cvar_cl
                                .wait(guard)
                                .expect("invariant: bootstrap condvar never poisoned");
                        }
                        Arc::clone(guard.as_ref().expect("invariant: just-checked Some"))
                    };
                    worker_main(pool, worker_id as u32, deque);
                })
                .expect("invariant: worker thread spawn must succeed");

            thread_handles.push(join.thread().clone());
            handles.push(join);
        }

        let workers: Arc<[CachePadded<WorkerHandle>]> = thread_handles
            .into_iter()
            .map(|t| CachePadded::new(WorkerHandle::new(t)))
            .collect::<Vec<_>>()
            .into();

        let join_handles: Vec<WorkerJoin> = handles
            .into_iter()
            .map(|h| WorkerJoin { handle: h })
            .collect();

        let pool = Arc::new(ThreadPool {
            injector_global: CachePadded::new(Injector::new()),
            injector_local,
            stealers,
            workers,
            idle: CachePadded::new(AtomicU64::new(0)),
            active_scopes: CachePadded::new(AtomicUsize::new(0)),
            shutdown: CachePadded::new(AtomicBool::new(false)),
            worker_count: worker_count as u32,
            join_handles: std::sync::Mutex::new(join_handles),
        });

        // Publish the pool to the waiting workers.
        {
            let mut guard = bootstrap
                .lock()
                .expect("invariant: bootstrap mutex never poisoned");
            *guard = Some(Arc::clone(&pool));
        }
        bootstrap_cvar.notify_all();

        // Touch the affinity setting so the compiler doesn't warn about
        // unused field once the no-op stub matures into real pinning.
        if self.affinity {
            // Wave 1 deliberate no-op (plan §4 Q7); see ThreadPoolBuilder
            // doc-comment.
        }

        pool
    }
}

/// Resolve the default worker count: prefer
/// `std::thread::available_parallelism`; fall back to 1.
#[cold]
fn default_worker_count() -> usize {
    match std::thread::available_parallelism() {
        Ok(n) => n.get(),
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn build_default_pool_succeeds() {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        assert_eq!(pool.worker_count(), 2);
        assert_eq!(pool.num_threads(), 2);
    }

    #[test]
    fn install_runs_closure() {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let v = pool.install(|_scope| 42);
        assert_eq!(v, 42);
    }

    #[test]
    fn spawn_100_tasks_via_scope_all_run() {
        let pool = ThreadPoolBuilder::new().num_threads(4).build();
        let counter = Arc::new(AtomicU32::new(0));

        pool.install(|scope| {
            for _ in 0..100 {
                let c = Arc::clone(&counter);
                scope.spawn(move || {
                    c.fetch_add(1, Ordering::Relaxed);
                });
            }
        });

        assert_eq!(counter.load(Ordering::Acquire), 100);
    }

    #[test]
    fn current_pool_is_none_outside_install() {
        assert!(ThreadPool::current_pool().is_none());
    }
}
