//! [`ThreadPool`] — the public face of the work-stealing pool — and
//! [`PoolInner`], the worker-shared state it wraps.
//!
//! Layout follows plan §4.2. Hot atomics live in [`CachePadded`] cells to
//! avoid false sharing on push/wake paths. Each worker exposes a
//! [`Stealer`] in a global registry, plus a local [`Injector`] that other
//! workers / the dispatcher can target for cache-friendly enqueuing
//! (plan §2.7 / Round 2 C2).
//!
//! ## Phase 9.3b — the handle/inner split (decision E)
//!
//! Before 9.3b every worker held an owned `Arc<ThreadPool>`, so the user's
//! handle could never reach refcount 0 while workers lived → `Drop` (the
//! sole writer of `shutdown`) was dead code and the threads + allocation
//! leaked to process exit. We break the cycle the way rayon does:
//!
//! - [`PoolInner`] holds every field workers / [`Scope`] cross-reference;
//!   workers hold `Arc<PoolInner>`.
//! - [`ThreadPool`] is the user-facing handle: `{ inner: Arc<PoolInner>,
//!   join_handles }`. Workers never see the handle, so dropping the last
//!   `Arc<ThreadPool>` runs [`ThreadPool::drop`] exactly once (decision E
//!   keeps `PoolInner` behind `Arc` only — never borrowed `&mut` while a
//!   worker is alive — so no Tree-Borrows protected-tag hazard, unlike the
//!   rejected self-pointer decision D).

use core::ptr::NonNull;
use std::sync::Arc;
// `thread::{Builder, JoinHandle}` are std-only (loom has no equivalent);
// `Thread` here is the std thread handle stored per worker. The scope waker's
// `Thread` (and `current()` for it) routes through `crate::sync` so that the
// loom M1 model observes the real park/unpark happens-before — see the two
// `crate::sync::thread::current()` call sites below.
use std::thread::{self, JoinHandle, Thread};

use crossbeam_deque::{Injector, Stealer, Worker};
use crossbeam_utils::CachePadded;

use crate::scope::{Scope, ScopeShared};
use crate::sync::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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
/// upholds the 'static erasure via [`Scope`]'s `Drop` blocking contract
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

/// Per-worker control block. Wrapped in [`CachePadded`] by the pool so that
/// idle/unpark traffic on worker `i` doesn't false-share with worker `j`.
#[repr(C)]
pub struct WorkerHandle {
    /// `std::thread::Thread` handle for `unpark`. Stable for the lifetime
    /// of the worker (set once at spawn).
    pub(crate) thread: Thread,
}

impl WorkerHandle {
    #[inline]
    pub(crate) fn new(thread: Thread) -> Self {
        Self { thread }
    }
}

/// Internal record kept alongside each spawned worker so that
/// [`ThreadPool::drop`] / [`ThreadPool::join`] can join cleanly. Not exposed.
struct WorkerJoin {
    handle: JoinHandle<()>,
}

/// Worker-shared state of the pool (plan §4.1; decision E).
///
/// This is the type behind `Arc<PoolInner>` that worker threads, [`Scope`],
/// and the ambient-pool TLS all reference. It is `pub` so that it can appear
/// in the public [`try_with_active_pool`](crate::try_with_active_pool)
/// signature, but **opaque**: it has no public fields and exposes only the
/// methods `par_iter`/`par_chunk` need ([`num_threads`](Self::num_threads),
/// [`scope`](Self::scope), [`install`](Self::install)).
///
/// `PoolInner` is **never borrowed `&mut`** anywhere — it lives behind `Arc`
/// and is dropped only via `Arc`'s internal `&mut` at refcount 0, i.e. AFTER
/// every worker has joined. No `&mut PoolInner` protector ever spans a worker
/// access, which is what keeps the cross-thread shared `&PoolInner` sound
/// under Tree Borrows (same reasoning as the Phase 9.2 `NonNull` fix).
#[repr(C)]
pub struct PoolInner {
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

    /// Per-worker handles (thread handle for `unpark`).
    pub(crate) workers: Arc<[CachePadded<WorkerHandle>]>,

    /// Idle bitset. Bit `i` is 1 iff worker `i` is parked or about to
    /// park. Lock-free via `fetch_or` / `fetch_and` / `compare_exchange`.
    pub(crate) idle: CachePadded<AtomicU64>,

    /// Rotating start offset for `unpark_one_idle`'s set-bit search. A plain
    /// `Relaxed` counter (order does not matter — it only spreads which parked
    /// worker is woken across successive wakes) that removes the systematic
    /// lowest-bit wake bias. `CachePadded` so the wake-path RMW does not
    /// false-share with the `idle` bitset it sits beside.
    pub(crate) wake_rotor: CachePadded<AtomicU64>,

    /// Counter of active install/scope frames. Used by `Drop` to assert
    /// no scope outlives the pool (drop while `> 0` is a contract bug).
    pub(crate) active_scopes: CachePadded<AtomicUsize>,

    /// Shutdown flag. Set by [`ThreadPool::drop`] before unparking all
    /// workers.
    pub(crate) shutdown: CachePadded<AtomicBool>,

    /// Worker count (cold; written once at construction). Lives here (O1) so
    /// a worker holding `&PoolInner` can read it.
    pub(crate) worker_count: u32,
}

impl PoolInner {
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

    /// Push a task onto the pool from outside any scope. Backing
    /// implementation of [`ThreadPool::spawn`].
    pub(crate) fn spawn<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let task = TaskHandle::new(Box::new(f));
        crate::worker::push_task(self, task);
    }

    /// Backing implementation of [`ThreadPool::install`]. Runs on the
    /// calling thread; sets the ambient-pool + worker-id TLS for the
    /// duration (restored on return *and* on unwind via `InstallGuard`).
    pub fn install<'scope, F, R>(&'scope self, f: F) -> R
    where
        F: FnOnce(&Scope<'scope>) -> R + Send,
        R: Send,
    {
        self.active_scopes.fetch_add(1, Ordering::AcqRel);

        let prev_pool = tls::swap_active_pool(self as *const PoolInner);
        let prev_worker_id = {
            let cur = tls::current_worker_id();
            tls::set_current_worker_id(tls::WORKER_ID_DISPATCHER);
            cur
        };

        // O4 (Phase 9.3b): now that `ThreadPool::drop` actually runs and
        // joins, a panic propagating out of `f`/`drop(scope)` must NOT leave
        // `active_scopes > 0` or dirty TLS (that would trip the Drop
        // debug-assert on unwind). The guard restores both on the normal and
        // the unwinding path.
        let _frame = InstallGuard {
            inner: self,
            prev_pool: Some(prev_pool),
            prev_worker_id: Some(prev_worker_id),
        };

        // `crate::sync::thread::current()` so the captured waker `Thread`
        // matches `ScopeShared.waker`'s type under both backends (loom routes
        // park/unpark through its own `Thread`).
        let shared = Box::new(ScopeShared::new(crate::sync::thread::current()));
        // SAFETY (scope lifetime erasure to '_):
        //   The scope is dropped before this function returns; Drop
        //   blocks until every spawned task has completed (or has
        //   panicked and been captured into `shared.panic_payload`).
        //   No task body outlives the scope.
        let scope = Scope::new(self, shared);

        let result = f(&scope);
        // Explicit drop so the order is unambiguous: scope drains first,
        // then `_frame` restores TLS + decrements `active_scopes`. (If `f`
        // panicked we never reach here; `scope` and `_frame` are dropped by
        // the unwinder in reverse declaration order — `scope` first, then
        // `_frame` — which is the same order as this manual sequence.)
        drop(scope);

        result
    }

    /// Backing implementation of [`ThreadPool::scope`]. Cheaper than
    /// [`install`](Self::install): it does not touch the ambient-pool /
    /// worker-id TLS (the caller is assumed to already be inside an
    /// `install` frame).
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

        // O4: decrement `active_scopes` on the unwinding path too. No TLS to
        // restore here (`scope` never swapped it), so both `prev_*` are None.
        let _frame = InstallGuard {
            inner: self,
            prev_pool: None,
            prev_worker_id: None,
        };

        // See `install`: shimmed `current()` so the waker `Thread` type matches
        // `ScopeShared.waker` under the loom backend.
        let shared = Box::new(ScopeShared::new(crate::sync::thread::current()));
        let scope = Scope::new(self, shared);

        let result = f(&scope);
        drop(scope);

        result
    }
}

/// RAII guard that restores the install/scope frame state (decrement
/// `active_scopes`, restore the ambient-pool + worker-id TLS) on BOTH the
/// normal return and the unwinding path (O4).
///
/// For [`PoolInner::scope`] there is no TLS to restore, so `prev_pool` /
/// `prev_worker_id` are `None`; only the `active_scopes` decrement runs.
struct InstallGuard<'a> {
    inner: &'a PoolInner,
    prev_pool: Option<*const PoolInner>,
    prev_worker_id: Option<u32>,
}

impl Drop for InstallGuard<'_> {
    #[inline]
    fn drop(&mut self) {
        if let Some(id) = self.prev_worker_id {
            tls::set_current_worker_id(id);
        }
        if let Some(p) = self.prev_pool {
            tls::swap_active_pool(p);
        }
        self.inner.active_scopes.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Custom Chase-Lev work-stealing thread pool — the user-facing handle.
///
/// See the crate docs for the Wave 1 feature set. Construct via
/// [`ThreadPoolBuilder::build`], which returns an `Arc<ThreadPool>`. Workers
/// hold `Arc<PoolInner>` (the inner state), NOT this handle, so dropping the
/// last `Arc<ThreadPool>` runs [`ThreadPool::drop`] — which sets the shutdown
/// flag, unparks every worker, and joins them — exactly once (plan §6).
///
/// # Shutdown contract
///
/// The handle (or its last clone) must **never** be dropped from inside an
/// [`install`](Self::install) / [`scope`](Self::scope) frame on the same
/// thread (O5): [`Drop`] now blocks on `join`, so self-joining the dispatcher
/// thread would deadlock. In practice this cannot happen — `install`/`scope`
/// borrow `&self`, so the handle is still owned by the caller for the whole
/// frame.
#[repr(C)]
pub struct ThreadPool {
    /// Worker-shared state. One `Arc` deref reaches every hot-path field.
    pub(crate) inner: Arc<PoolInner>,

    /// Join handles for the worker threads, owned by the handle ALONE
    /// (workers cannot reach them, so they cannot double-join). `Option` so
    /// that an explicit [`join`](Self::join) and [`Drop`] are
    /// join-exactly-once via `take()`. The `Mutex` is touched only on the
    /// cold teardown path; never on a hot path.
    // Shutdown plumbing: the only `lock()` sites are `ThreadPool::join` and
    // `ThreadPool::drop`, both once-per-process teardown. Workers cannot reach
    // this field at all.
    #[allow(clippy::disallowed_types)]
    join_handles: std::sync::Mutex<Option<Vec<WorkerJoin>>>,
}

impl ThreadPool {
    /// Returns the number of worker threads in the pool.
    #[inline]
    pub fn worker_count(&self) -> u32 {
        self.inner.worker_count()
    }

    /// Convenience alias mirroring the plan's `num_threads()` naming.
    #[inline]
    pub fn num_threads(&self) -> usize {
        self.inner.num_threads()
    }

    /// Read the active pool for the current thread (the one set by the
    /// most recent [`ThreadPool::install`] on this thread). Returns
    /// `None` when no pool is attached.
    ///
    /// Wave 6 / `par_iter` uses this to grab the ambient pool without an
    /// explicit reference parameter on the iterator. The pointer is to the
    /// shared [`PoolInner`] (decision E), never the handle.
    #[inline]
    pub fn current_pool() -> Option<NonNull<PoolInner>> {
        let p = tls::active_pool_ptr();
        // SAFETY: `tls::active_pool_ptr` returns either a null pointer or a
        // pointer that was deposited by `install`/`scope`/`worker_main` while
        // a live `PoolInner` (behind `Arc`) was reachable. The caller receives
        // a `NonNull` wrapper but the dereference is the caller's
        // responsibility; the `NonNull` itself is sound to construct from a
        // non-null pointer.
        NonNull::new(p as *mut PoolInner)
    }

    /// Push a task onto the pool from outside any scope. Intended for
    /// fire-and-forget work that does not require join semantics. The
    /// 'static bound is the absence of any borrowing; for scoped work see
    /// [`ThreadPool::install`] / [`ThreadPool::scope`].
    #[inline]
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.inner.spawn(f);
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
    #[inline]
    pub fn install<'scope, F, R>(&'scope self, f: F) -> R
    where
        F: FnOnce(&Scope<'scope>) -> R + Send,
        R: Send,
    {
        self.inner.install(f)
    }

    /// Re-entrant scope creation. Cheaper than [`install`](Self::install)
    /// because it does not touch the active-pool / worker-id TLS — the
    /// caller is assumed to already be inside an `install` frame
    /// (typically a worker task body or the dispatcher running through
    /// `Schedule::run`).
    ///
    /// Used by `Query::par_iter` (Wave 6).
    #[inline]
    pub fn scope<'scope, F, R>(&'scope self, f: F) -> R
    where
        F: FnOnce(&Scope<'scope>) -> R + Send,
        R: Send,
    {
        self.inner.scope(f)
    }

    /// Explicitly shut the pool down and join every worker thread, blocking
    /// until they exit. Idempotent with [`Drop`]: the join handles are taken
    /// once, so calling `join()` and then dropping the handle (or vice
    /// versa) joins exactly once and never double-joins.
    ///
    /// Must not be called from inside an `install`/`scope` frame on the same
    /// thread (O5) — it would self-join the dispatcher and deadlock.
    pub fn join(&self) {
        let handles = self
            .join_handles
            .lock()
            .expect("invariant: join_handles mutex never poisoned by us")
            .take();
        if let Some(handles) = handles {
            self.shutdown_and_join(handles);
        }
    }

    /// Shared teardown body for [`join`](Self::join) and [`Drop`]. The
    /// caller has already `take()`n the join handles, so this runs for
    /// exactly one of `{join(), Drop}` — the take-once discipline guarantees
    /// no double-join.
    fn shutdown_and_join(&self, handles: Vec<WorkerJoin>) {
        // Publish shutdown to every worker BEFORE unparking — workers
        // re-check this flag after wakeup. Release pairs with the worker's
        // Acquire load in `worker_main`.
        self.inner.shutdown.store(true, Ordering::Release);

        for w in self.inner.workers.iter() {
            w.thread.unpark();
        }

        for j in handles {
            // We deliberately ignore the result — a panicking worker has
            // already been observed via the scope's panic_payload path;
            // join here would only surface "thread aborted unexpectedly".
            let _ = j.handle.join();
        }
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        // Take the join handles once. If an explicit `join()` already took
        // them, this is a no-op (join-exactly-once).
        let handles = self
            .join_handles
            .lock()
            .expect("invariant: join_handles mutex never poisoned by us")
            .take();
        let Some(handles) = handles else { return };

        debug_assert_eq!(
            self.inner.active_scopes.load(Ordering::Acquire),
            0,
            "ThreadPool dropped with active scopes still in flight"
        );

        self.shutdown_and_join(handles);
    }
}

// SAFETY (Send/Sync for ThreadPool / PoolInner):
//   Every field is either trivially Send/Sync (Arc, atomics, Mutex,
//   plain `u32`) or a `CachePadded<...>` wrapper around such a type.
//   `Injector<TaskHandle>`, `Stealer<TaskHandle>`, and `Worker<TaskHandle>`
//   are Send/Sync per crossbeam-deque's public contracts (verified in
//   crossbeam-deque 0.8 docs). `TaskHandle::body` is `Box<dyn FnOnce + Send
//   + 'static>` so the queues' element types are Send. No interior
//   `!Send`/`!Sync` field exists.
//
//   `PoolInner` MUST be Send + Sync because the `Arc<PoolInner>` is shared
//   between the spawning thread and every worker thread (and the ambient-pool
//   TLS hands out `&PoolInner` cross-thread). `ThreadPool` MUST be Send +
//   Sync because `Schedule` (Wave 3+) borrows `&ThreadPool` for `install`
//   from arbitrary system contexts; its `Arc<PoolInner>` + `Mutex<Option<…>>`
//   fields are both Send + Sync.
//
//   The auto-derive already gives us Send/Sync for both structs (no raw
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
    /// the canonical handle; workers hold their own `Arc<PoolInner>` clones
    /// internally so that the inner state stays alive at least until every
    /// worker exits, but they do NOT hold the handle (so [`ThreadPool::drop`]
    /// runs when the last `Arc<ThreadPool>` is dropped — plan §6).
    // Pool bootstrap: the `Mutex`es here are the one-shot `Arc<PoolInner>`
    // publication handshake (dropped once every worker has read it) and the
    // teardown-only `join_handles` slot. Runs once at engine boot, never
    // per-frame.
    #[allow(clippy::disallowed_types)]
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
        // populate `inner.workers`, which means we must spawn the threads,
        // capture their handles, and let them learn their own
        // `Arc<PoolInner>`. The classical trick: spawn threads that block on
        // a one-shot handshake; the parent constructs the `Arc<PoolInner>`,
        // then publishes it. We use a `Once`-style handshake via
        // `Arc<Mutex<Option<Arc<PoolInner>>>>` + `Condvar`; the Condvar
        // notify happens-after the `*guard = Some(...)` store and
        // happens-before each worker's wake, so the published `Arc<PoolInner>`
        // is visible to every worker without any extra atomic ordering.
        //
        // Alternative considered: pre-create `WorkerHandle`s with bogus
        // Thread handles and patch them later. Rejected because Thread
        // doesn't expose a no-op constructor.
        let bootstrap: Arc<std::sync::Mutex<Option<Arc<PoolInner>>>> =
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
                    // Wait for the parent to publish the Arc<PoolInner>.
                    let inner = {
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
                    worker_main(inner, worker_id as u32, deque);
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

        let inner = Arc::new(PoolInner {
            injector_global: CachePadded::new(Injector::new()),
            injector_local,
            stealers,
            workers,
            idle: CachePadded::new(AtomicU64::new(0)),
            wake_rotor: CachePadded::new(AtomicU64::new(0)),
            active_scopes: CachePadded::new(AtomicUsize::new(0)),
            shutdown: CachePadded::new(AtomicBool::new(false)),
            worker_count: worker_count as u32,
        });

        // Publish the inner state to the waiting workers.
        {
            let mut guard = bootstrap
                .lock()
                .expect("invariant: bootstrap mutex never poisoned");
            *guard = Some(Arc::clone(&inner));
        }
        bootstrap_cvar.notify_all();

        // Touch the affinity setting so the compiler doesn't warn about
        // unused field once the no-op stub matures into real pinning.
        if self.affinity {
            // Wave 1 deliberate no-op (plan §4 Q7); see ThreadPoolBuilder
            // doc-comment.
        }

        Arc::new(ThreadPool {
            inner,
            join_handles: std::sync::Mutex::new(Some(join_handles)),
        })
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
