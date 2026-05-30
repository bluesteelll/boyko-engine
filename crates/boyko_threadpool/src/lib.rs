//! # boyko_threadpool
//!
//! Custom Chase-Lev work-stealing thread pool — the foundation of the
//! Phase 9 parallel scheduler for `boyko-engine`. Built directly on top of
//! `crossbeam_deque::{Worker, Stealer, Injector}` primitives; everything
//! above (worker threads, parking, scope, panic propagation, install API)
//! is hand-rolled to fit the engine's contracts.
//!
//! ## Scope of this crate
//!
//! This crate is **not** a general-purpose thread pool. It exists to support
//! the `boyko_ecs` scheduler (Wave 3+) and the `Query::par_iter` fork-join
//! driver (Wave 6). API design choices are dictated by the scheduler's needs:
//!
//! - **`install`** sets up TLS bookkeeping for the calling dispatcher thread
//!   and runs a closure that may spawn tasks into the pool.
//! - **`scope`** is the same shape as `install` but lighter — designed to be
//!   called from inside a worker (or from inside another `install`) without
//!   re-entering the dispatcher TLS bookkeeping. `par_iter` uses `scope`.
//! - **`Scope::Drop`** blocks via *work-stealing* (the rayon pattern), not
//!   plain parking, so that nested scopes cannot deadlock when every worker
//!   is itself inside a `scope` waiting for inner tasks.
//!
//! ## Phase 9 contracts (TPN1..TPN13, ALLOC1..ALLOC6, EVT1..EVT4)
//!
//! See `docs/PHASE-9-PARALLEL-SCHEDULER-PLAN.md` §2 for the full invariant
//! list. The crate ships:
//!
//! - **TLS `CURRENT_WORKER_ID`** (TPN13) — populated on worker entry, set to
//!   [`WORKER_ID_DISPATCHER`] when [`ThreadPool::install`] runs on the calling
//!   thread, [`WORKER_ID_UNATTACHED`] otherwise.
//! - **TLS `IN_SYSTEM_RUN`** (ALLOC1/ALLOC6) — set by the scheduler's RAII
//!   guard around `System::run_unsafe`; consumed by `Arena::allocate_*`
//!   debug assertions in the ECS crate (Wave 2 Step 7c).
//! - **TLS `ACTIVE_POOL`** — current pool pointer for ambient `par_iter`
//!   dispatch (consumed by `Query::par_iter` in Wave 6).
//! - **Idle bitset** (TPN6/TPN7) — `AtomicU64`-backed; supports up to
//!   [`MAX_WORKERS`] worker threads. Lock-free push/wake protocol with a
//!   load-bearing re-poll after `mark_idle` (see §13.4.1 Race C).
//!
//! ## What this Wave 1 ships
//!
//! - `ThreadPool` struct + builder API (no worker affinity yet — see Wave 7).
//! - `Scope::spawn` with `'scope` lifetime erasure (SAFETY: `Scope::Drop`
//!   blocks until pending tasks complete, even on panic).
//! - `worker_main` with the 4-source poll loop (local injector → global
//!   injector → sibling steal → backoff/park).
//! - Public TLS helpers `current_worker_id`,
//!   `current_worker_id_or_dispatcher_lane`.
//!
//! Subsequent waves (Schedule, par_iter, ECS Send/Sync gate, etc.) live in
//! `boyko_ecs`. This crate has no dependency on the ECS.

mod scope;
pub(crate) mod sync;
mod thread_pool;
mod tls;
mod worker;

pub use scope::Scope;
pub use thread_pool::{MAX_WORKERS, TaskHandle, ThreadPool, ThreadPoolBuilder, WorkerHandle};
pub use tls::{
    InSystemRunGuard, WORKER_ID_DISPATCHER, WORKER_ID_UNATTACHED, current_worker_id,
    current_worker_id_or_dispatcher_lane, is_in_system_run, try_with_active_pool,
};

/// Phase 9.1 loom test surface (test-only; `#[cfg(loom)]`, never in the shipped
/// artifact).
///
/// The loom models in `tests/loom_pool.rs` (`#![cfg(loom)]`) are an external
/// integration crate that cannot reach the crate-internal (`pub(crate)`)
/// synchronization primitives, and a `pub use` of a `pub(crate)` item is
/// rejected (E0364/E0365). To honor C1 — the models must drive the *real*
/// production methods, not copies — this module exposes thin `pub` shim wrappers
/// that forward to the unchanged `pub(crate)` items. Each wrapper is a single
/// call to the production method, so loom still observes the real
/// `AcqRel`/`Acquire`/`Release` orderings of `scope.rs` / `worker.rs`.
///
/// The whole module is gated by `cfg(loom)`: the normal (non-loom) build never
/// compiles it and the production declarations are left exactly as shipped
/// (`pub(crate)`, unchanged) — byte-identical native codegen (§6
/// zero-native-cost). No new symbol leaks into the normal public API.
#[cfg(loom)]
pub mod loom_exports {
    use crate::scope::ScopeShared;

    /// The loom-shimmed synchronization surface for the models, taken straight
    /// from `loom`. Re-exporting through `crate::sync` is impossible because its
    /// items are `pub(crate)` (a `pub use` of them is E0365); going to `loom`
    /// directly yields the **identical** `Atomic*` / `Thread` types that
    /// `crate::sync` aliases under `--cfg loom`, so the model and the production
    /// methods still share one loom atomic / waker instance (C1 preserved).
    pub mod sync {
        pub use loom::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering, fence};
        pub use loom::thread::{self, Thread};
    }

    /// Opaque `pub` handle wrapping the real `pub(crate) ScopeShared`, so the
    /// external loom model can construct and drive it. Every method below is a
    /// one-line forward to the production method — loom sees the real orderings.
    pub struct LoomScopeShared(ScopeShared);

    impl LoomScopeShared {
        /// Construct over the real `ScopeShared::new` (loom `Thread` waker).
        #[inline]
        pub fn new(waker: crate::sync::Thread) -> Self {
            Self(ScopeShared::new(waker))
        }

        /// Forwards to the real [`ScopeShared::register_task`] (`fetch_add`,
        /// `AcqRel`).
        #[inline]
        pub fn register_task(&self) {
            self.0.register_task();
        }

        /// Forwards to the real [`ScopeShared::complete_task`] (`fetch_sub`
        /// `AcqRel` + `prev==1` unpark — the lost-wakeup-critical branch).
        #[inline]
        pub fn complete_task(&self) {
            self.0.complete_task();
        }

        /// Forwards to the real [`ScopeShared::is_drained`] (`load`, `Acquire`).
        #[inline]
        pub fn is_drained(&self) -> bool {
            self.0.is_drained()
        }
    }

    /// Forwards to the real [`crate::worker::mark_idle`] (`fetch_or`,
    /// `Release`).
    #[inline]
    pub fn mark_idle(idle: &crate::sync::AtomicU64, worker_id: u32) {
        crate::worker::mark_idle(idle, worker_id);
    }

    /// Forwards to the real [`crate::worker::unmark_idle`] (`fetch_and`,
    /// `Release`).
    #[inline]
    pub fn unmark_idle(idle: &crate::sync::AtomicU64, worker_id: u32) {
        crate::worker::unmark_idle(idle, worker_id);
    }
}
