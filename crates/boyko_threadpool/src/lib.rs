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
mod thread_pool;
mod tls;
mod worker;

pub use scope::Scope;
pub use thread_pool::{MAX_WORKERS, TaskHandle, ThreadPool, ThreadPoolBuilder, WorkerHandle};
pub use tls::{
    InSystemRunGuard, WORKER_ID_DISPATCHER, WORKER_ID_UNATTACHED, current_worker_id,
    current_worker_id_or_dispatcher_lane, is_in_system_run, try_with_active_pool,
};
