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
//! - **TLS `LANE_DEPOSIT.wid`** (TPN13) — populated on worker entry, set to
//!   [`WORKER_ID_DISPATCHER`] when [`ThreadPool::install`] runs on the calling
//!   thread, [`WORKER_ID_UNATTACHED`] otherwise.
//! - **TLS `IN_SYSTEM_RUN`** (ALLOC1/ALLOC6) — set by the scheduler's RAII
//!   guard around `System::run_unsafe`; consumed by context-restricted
//!   debug assertions in the ECS crate (event send/read, `Time` access;
//!   originally the retired shared Arena's `allocate_*`, Wave 2 Step 7c).
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
//! - `worker_main` with the 3-source poll loop (own deque → global injector →
//!   sibling steal → backoff/park). A worker's OWN spawns land on its own
//!   REGISTERED deque, whose `Stealer` every sibling scans. The owner end is
//!   FIFO, so the worker takes its OLDEST entry first and a fresh spawn is
//!   served behind what is already queued. That deque is also where a batch
//!   stolen from any other source is parked between tasks.
//! - Public TLS helpers `current_worker_id`,
//!   `current_worker_id_or_dispatcher_lane`.
//!
//! Subsequent waves (Schedule, par_iter, ECS Send/Sync gate, etc.) live in
//! `boyko_ecs`. This crate has no dependency on the ECS.

mod block;
mod scope;
pub(crate) mod sync;
mod task;
mod thread_pool;
mod tls;
mod worker;

// =========================================================================
// KE16 M2w — the Tree-Borrows negative control is MIRI-ONLY.
//
// `tb-neg-m2w` does not identify a configuration — it falsifies a gate. It is
// a deliberate-UB arm whose purpose is to BE REPORTED, and it exists so that
// M2w's positive result — "the shipped `task::scoped::run_scoped` executed
// under Tree Borrows with the chunk freed inside a completer's open release
// window and no UB was reported" — is known to be falsifiable, which a green
// with no producible red is not.
//
// A native build of it would put deliberate UB into every scoped spawn of an
// ordinary artifact, and `--all-features` is the configuration that would do
// that without anyone asking for it. So it does not compile.
//
// EXIT CONDITION 9 ASSERTS THIS MESSAGE, NOT THE EXIT CODE. Before the feature
// row existed, `cargo check --features tb-neg-m2w` already failed — because
// cargo did not know the feature — which satisfied a "must FAIL" criterion
// while the arm it gates did not exist. A condition whose pass criterion is an
// exit code cannot tell a refusal from an absence.
// =========================================================================

#[cfg(all(feature = "tb-neg-m2w", not(miri)))]
compile_error!(
    "`tb-neg-m2w` is the KE16 M2w DELIBERATE-UB negative control and builds ONLY under Miri. \
     `cfg(miri)` is unset in this build, so the arm would compile into a native artifact whose \
     every scoped task commits a Tree-Borrows violation on purpose: `run_scoped_neg` holds a \
     protector over chunk memory across the release RMW that authorises `Scope::drop`'s \
     `free_all` to deallocate that chunk. Run it under `cargo miri test`, through \
     `scripts/tb_neg_gate.ps1`, or drop the feature. THIS refusal is the failure exit condition 9 \
     requires; cargo's own `none of the selected packages contains these features` is NOT — that \
     one means the feature row is missing from `crates/boyko_threadpool/Cargo.toml` and the arm \
     does not exist at all"
);

pub use scope::Scope;
pub use thread_pool::{MAX_WORKERS, PoolInner, ThreadPool, ThreadPoolBuilder, WorkerHandle};
pub use tls::{
    InSystemRunGuard, WORKER_ID_DISPATCHER, WORKER_ID_UNATTACHED, current_worker_id,
    current_worker_id_or_dispatcher_lane, is_in_system_run, try_with_active_pool,
};

/// Miri-only: how many times `ScopeShared::complete_task`'s release probe has
/// fired since process start — one per `pending -> 0` transition.
///
/// The WEAKER of the gate's two armed-ness observations. It proves the probe was
/// CALLED, which catches a probe that was deleted, `cfg`'d away, or moved off
/// the `prev == 1` path. It does NOT prove the probe still opens a window: the
/// KE16 tester defeated it by keeping this very `fetch_add` and deleting only
/// the yield loop, which left the counter full, every census green, and the gate
/// deciding nothing. `miri_frees_inside_a_release_window` is the observation
/// that mutation cannot pass; both are asserted, because they fail differently
/// and their messages point at different mistakes.
///
/// `cfg(miri)`-only: the shipped artifact has neither the counter nor this
/// function.
#[cfg(miri)]
#[must_use]
pub fn miri_release_probe_firings() -> usize {
    scope::MIRI_RELEASE_PROBE_FIRINGS.load(core::sync::atomic::Ordering::SeqCst)
}

/// Miri-only: how many `Scope::drop` frees landed while a completer of that same
/// allocation was still inside its post-decrement release window.
///
/// THE GATE'S ARMED-NESS OBSERVATION — the only one that checks the property
/// instead of a proxy for it. `tests/miri_scope_completion_protector.rs` decides
/// whether a free can race a live Tree-Borrows protector; that verdict is
/// meaningful only if the run actually produced the overlap, and neither of the
/// test's schedule censuses (`bodies`, `windows`) can see it, because the
/// overlap happens after the decrement where no body observes anything.
///
/// It exists because NO CONSTANT CAN CERTIFY THE SAME THING. A compile-time
/// floor on the probe's yield count was tried and refuted twice over: the
/// threshold was measured at 4 rather than the 2 that was asserted, AND
/// armed-ness turned out not to be monotonic in the count at all — two
/// reconstructions differing by one MIR statement put a DISARMED point at 8
/// yields between armed points at 7 and 10, because the mechanism is
/// round-robin alignment rather than duration. This counter observes the
/// alignment's RESULT and so is indifferent to how it was obtained.
///
/// `cfg(miri)`-only.
#[cfg(miri)]
#[must_use]
pub fn miri_frees_inside_a_release_window() -> usize {
    scope::MIRI_FREES_INSIDE_A_RELEASE_WINDOW.load(core::sync::atomic::Ordering::SeqCst)
}

/// Miri-only: how many `Scope::drop` CHUNK frees landed while a completer of
/// that same scope was still inside its post-decrement release window.
///
/// The KE16 M2w observation. It is the block's counterpart to
/// [`miri_frees_inside_a_release_window`] and NOT a second reading of it:
/// that one observes the `ScopeShared` box's `Box::from_raw`, this one observes
/// `ScopeBlock::free_all`, which reclaims the chunks the scope's task cells
/// live in. Since Stage 3b those cells outlive the release RMW, so their
/// storage is freed inside the same window the scope's own allocation is — and
/// that is a NEW free, in a class no completer holds a pointer into, which is
/// why it gets its own column rather than sharing one whose `>= 1` could be
/// satisfied by the other event entirely.
///
/// Counted once per `Scope::drop`, and only for a scope whose block actually
/// grew a chunk.
///
/// `cfg(miri)`-only.
#[cfg(miri)]
#[must_use]
pub fn miri_block_frees_inside_a_release_window() -> usize {
    scope::MIRI_BLOCK_FREES_INSIDE_A_RELEASE_WINDOW.load(core::sync::atomic::Ordering::SeqCst)
}

/// Miri-only: how many completers could not record their release window because
/// every slot was already taken.
///
/// NOT a third armed-ness observation — it is what stops the other two from
/// being read wrong. A completer that finds no slot loses an observation and can
/// never invent one, so the gate stays sound; but the loss reappears downstream
/// as `overlaps = 0`, under an assert whose message sends the reader off to
/// re-tune the probe's yield count against an observation that was never taken.
/// A red with the WRONG DIAGNOSIS is the failure mode this repository keeps
/// cataloguing. The gate prints this delta and asserts it is zero, so a slot
/// shortage reports itself as a slot shortage.
///
/// `cfg(miri)`-only.
#[cfg(miri)]
#[must_use]
pub fn miri_window_slot_exhaustions() -> usize {
    scope::MIRI_WINDOW_SLOT_EXHAUSTIONS.load(core::sync::atomic::Ordering::SeqCst)
}

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

    /// KE16 W-d′ — the count-gated wake target type, re-exported for M1c.
    ///
    /// Under loom this is the counting newtype over `loom::thread::Thread`
    /// (`crate::sync`), which the model OWNS: production never constructs one
    /// (`PoolInner::joiner_wake_target` is null under loom), so the target M1c
    /// hands to [`LoomScopeShared::new_worker_joined`] is the model's own
    /// `Box<WakeHandle>`, outliving the completers that unpark through it.
    pub use crate::sync::WakeHandle;

    /// Opaque `pub` handle wrapping the real `pub(crate) ScopeShared`, so the
    /// external loom model can construct and drive it. Every method below is a
    /// one-line forward to the production method — loom sees the real orderings.
    pub struct LoomScopeShared(ScopeShared);

    impl LoomScopeShared {
        /// Construct over the real `ScopeShared::new` (loom `Thread` waker) with
        /// NO count-gate target — the external-joiner shape, which is what
        /// M1/M2/M3 model.
        #[inline]
        pub fn new(waker: crate::sync::Thread) -> Self {
            Self(ScopeShared::new(waker, core::ptr::null()))
        }

        /// KE16 W-d′ (M1c): construct over the real `ScopeShared::new` with a
        /// count-gate target, the shape a scope opened FROM A WORKER of the
        /// scope's own pool has.
        ///
        /// # Safety
        /// `target` must point at a `WakeHandle` that outlives every task
        /// registered on this scope — in the model, a `Box<WakeHandle>` owned by
        /// the `loom::model` closure and dropped after the joins. Production's
        /// equivalent invariant is that the pointee is `PoolInner`-owned
        /// (`ScopeShared::joiner_wake`).
        #[inline]
        pub unsafe fn new_worker_joined(
            waker: crate::sync::Thread,
            target: *const WakeHandle,
        ) -> Self {
            Self(ScopeShared::new(waker, target))
        }

        /// Forwards to the real [`ScopeShared::register_task`] (`fetch_add`,
        /// `AcqRel`).
        #[inline]
        pub fn register_task(&self) {
            self.0.register_task();
        }

        /// Forwards to the real [`ScopeShared::complete_task`]. Which arm runs
        /// is decided by the scope's wake TARGET, not by the model and not by
        /// the build: a NULL target — the external-joiner shape — takes Phase
        /// 9.2 Candidate U's unconditional `waker.unpark()` BEFORE the
        /// `fetch_sub` `AcqRel`, because that waker lives in the allocation the
        /// decrement releases; a NON-NULL one takes the `fetch_sub` first and
        /// unparks the target only on `prev == 1` (KE16 W-d′), which it can
        /// afford because that target is owned by the pool rather than by the
        /// scope. M1c drives the second, which is why its target has to be real
        /// enough to count.
        ///
        /// The production function is an associated function over
        /// `*const ScopeShared` (the KE16 protector-lifetime fix), so this
        /// forward coerces its own `&self` to an address. The model is
        /// unaffected: loom checks orderings, not Tree-Borrows protectors, and
        /// the `LoomScopeShared` is owned by the model's closure rather than
        /// freed by a racing joiner.
        #[inline]
        pub fn complete_task(&self) {
            // SAFETY: `&self.0` is a live `ScopeShared` owned by this wrapper,
            //   which the model keeps alive across every completer (it is a
            //   `loom::sync::Arc<LoomScopeShared>` in M1/M1c). The caller —
            //   always a model thread that popped exactly one toy task —
            //   completes each registration at most once, which is the
            //   function's one obligation.
            unsafe { ScopeShared::complete_task(&self.0) };
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
    /// `Release`), previous-word return included.
    ///
    /// M1 / M2 / M2b / M2c ignore it: their parked thread plays `worker_main`'s
    /// role, and `worker_main` never needs to know whether its bit was claimed,
    /// because every one of its post-`mark_idle` exits re-polls every source.
    /// M4 does need it — "was the bit still mine?" is the discriminator of its
    /// lost-wake oracle, and reading it off the RMW rather than off a preceding
    /// load is what makes the answer exact (a claim cannot slip between them).
    /// It is the same question the KE16 B joiner asks at every one of its
    /// post-`mark_idle` exits.
    #[inline]
    pub fn unmark_idle(idle: &crate::sync::AtomicU64, worker_id: u32) -> u64 {
        crate::worker::unmark_idle(idle, worker_id)
    }

    /// Forwards to the real [`crate::worker::publish_fence`] — the producer's
    /// half of the wake protocol's SB litmus (KE16 W-a). M2 drives THIS, not a
    /// model-local `fence`, so the model and production cannot drift apart
    /// (C1); its calibration copy deletes the call and must go red.
    #[inline]
    pub fn publish_fence() {
        crate::worker::publish_fence();
    }
}

/// The layout facts this crate's allocation receipts are stated over.
///
/// `#[doc(hidden)] pub` because the receipt binaries in `tests/` are external
/// crates: they cannot read a `pub(crate)` constant, and a number they carried
/// themselves would be a number nothing checks. Hidden because it is an
/// instrument, not API.
///
/// EVERY CONSTANT HERE IS AN INDEPENDENT LITERAL, and that is the whole design.
/// A receipt that computed its expectation from the type under test would
/// assert `x == x` and survive any change to either. Each literal is instead
/// pinned to the real type by a `const _: () = assert!(..)` NEXT TO THAT TYPE —
/// the two cell headers in `task/scoped.rs` and `task/detached.rs`, the three
/// chunk facts in `block.rs` — so drift between a number here and the thing it
/// describes is a build failure.
///
/// The cell headers carry THREE pins each rather than one. `size_of::<Cell<()>>()
/// == HEADER` alone would still hold with the `body` field deleted; the second
/// pin, at a 64-byte body, is what says the body is IN the cell at its own size.
/// The third pins the ALIGNMENT, because a receipt's bucket is the pair
/// `(size, align)` and the two size pins leave that pair half-open — an added
/// `#[repr(align(16))]` keeps both of them green while emptying the bucket.
#[doc(hidden)]
pub mod __layout_receipt {
    /// `size_of::<ScopedCell<F>>() - size_of::<F>()`: the `CellHead` thunk plus
    /// the scope's `ScopeShared` address. Pinned in `task/scoped.rs`.
    pub const SCOPED_CELL_HEADER: usize = 16;

    /// `align_of::<ScopedCell<F>>()` for every `F` whose own alignment does not
    /// exceed it — the cell class's alignment floor, set by the thunk and the
    /// `ScopeShared` address. Pinned in `task/scoped.rs`.
    ///
    /// A receipt buckets an allocation on the PAIR `(size, align)`, so this
    /// half needs its own literal: with size pinned alone, a cell class free to
    /// move to another alignment moves out of the bucket a receipt counts over,
    /// and "zero allocations in the cell class" comes back green from an empty
    /// bucket rather than from a clean run.
    pub const SCOPED_CELL_ALIGN: usize = 8;

    /// `size_of::<DetachedCell<F>>() - size_of::<F>()`: the `CellHead` thunk
    /// alone, because a fire-and-forget task has no scope to name. Pinned in
    /// `task/detached.rs`.
    pub const DETACHED_CELL_HEADER: usize = 8;

    /// `align_of::<DetachedCell<F>>()` under the same condition and for the
    /// same reason as [`SCOPED_CELL_ALIGN`]: the thunk alone sets it. Pinned in
    /// `task/detached.rs`.
    pub const DETACHED_CELL_ALIGN: usize = 8;

    /// The block allocator's base chunk capacity — every chunk is `CHUNK0 << e`
    /// bytes for some `e`. Pinned in `block.rs`.
    pub const CHUNK0: usize = 4096;

    /// Every chunk's alignment. With the power-of-two size above it is the
    /// bucket predicate a chunk-class receipt matches on. Pinned in `block.rs`.
    pub const CHUNK_ALIGN: usize = 64;

    /// The chunk table's length. Pinned in `block.rs`.
    pub const MAX_CHUNKS: usize = 32;
}
