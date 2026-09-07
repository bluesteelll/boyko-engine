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
//! - `worker_main` with the 4-source poll loop (local injector → own deque →
//!   global injector → sibling steal → backoff/park). The own deque is the
//!   source the earlier wording omitted: it is where a batch stolen from any
//!   other source is parked between tasks. A worker's OWN spawns do not land
//!   there today — they go to `injector_local[wid]`, which no sibling polls
//!   (KE16 defect A); the A axis of that tournament is what moves them.
//!   Under `ke16-a1` / `ke16-a1-fifo` they land on the own deque and under
//!   `ke16-a3` in the global injector, and then the loop is 3-source: the first
//!   stage is compiled out with the queue it drained (own deque → global
//!   injector → sibling steal → backoff/park).
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
// KE16 tournament — mutual exclusion of the candidate features.
//
// One `compile_error!` per illegal pair, each naming both features and the
// axis, so a mis-specified `--features` line fails to BUILD instead of
// producing a number for a configuration nobody meant to measure
// (`docs/threadpool/KE16-DESIGN.md` §4). `ke16-a5` implies `ke16-a2` through
// Cargo, so that one pair is legal and is absent from the list. The whole
// block is deleted with the features once the verdict lands.
// =========================================================================

#[cfg(all(feature = "ke16-a1", feature = "ke16-a1-fifo"))]
compile_error!(
    "KE16 axis A: `ke16-a1` and `ke16-a1-fifo` are mutually exclusive (one placement candidate \
     per build)"
);
#[cfg(all(feature = "ke16-a1", feature = "ke16-a2"))]
compile_error!("KE16 axis A: `ke16-a1` and `ke16-a2` are mutually exclusive");
#[cfg(all(feature = "ke16-a1", feature = "ke16-a3"))]
compile_error!("KE16 axis A: `ke16-a1` and `ke16-a3` are mutually exclusive");
#[cfg(all(feature = "ke16-a1-fifo", feature = "ke16-a2"))]
compile_error!("KE16 axis A: `ke16-a1-fifo` and `ke16-a2` are mutually exclusive");
#[cfg(all(feature = "ke16-a1-fifo", feature = "ke16-a3"))]
compile_error!("KE16 axis A: `ke16-a1-fifo` and `ke16-a3` are mutually exclusive");
#[cfg(all(feature = "ke16-a2", feature = "ke16-a3"))]
compile_error!("KE16 axis A: `ke16-a2` and `ke16-a3` are mutually exclusive");
#[cfg(all(feature = "ke16-a5", feature = "ke16-a1"))]
compile_error!(
    "KE16 axis A: `ke16-a5` and `ke16-a1` are mutually exclusive (A5 places into `injector_local`, \
     which A1 never feeds)"
);
#[cfg(all(feature = "ke16-a5", feature = "ke16-a1-fifo"))]
compile_error!("KE16 axis A: `ke16-a5` and `ke16-a1-fifo` are mutually exclusive");
#[cfg(all(feature = "ke16-a5", feature = "ke16-a3"))]
compile_error!("KE16 axis A: `ke16-a5` and `ke16-a3` are mutually exclusive");

#[cfg(all(feature = "ke16-b1", feature = "ke16-b3"))]
compile_error!(
    "KE16 axis B: `ke16-b1` and `ke16-b3` are mutually exclusive (they differ only in the external \
     joiner's policy)"
);
#[cfg(all(
    any(feature = "ke16-b1", feature = "ke16-b3"),
    not(any(feature = "ke16-a1", feature = "ke16-a1-fifo"))
))]
compile_error!(
    "KE16 axis B: `ke16-b1` / `ke16-b3` require `ke16-a1` or `ke16-a1-fifo` — the worker joiner \
     must be able to REACH its own deque, and only the A1 arms publish that deque's address into \
     this thread's TLS (`WorkerDequeDeposit`, `worker.rs`). THE MISSING THING IS THE DEPOSIT, NOT \
     THE DEQUE: every arm builds `worker_count` deques and registers their `Stealer`s \
     unconditionally (`thread_pool.rs`), and each worker is handed one by move, so `WorkerLane` \
     under A2/A3/A5 simply carries a `wid` with no `deque()` accessor. A2/A3/A5 therefore leave \
     the joiner at B0 — a REACHABILITY gap, not a structural impossibility. An earlier wording of \
     this message said the A1 arms are what GIVE the joiner a registered deque; that was false, \
     and it is what made axis B read as closed by construction"
);

// =========================================================================
// KE16 tournament — every axis's arm is in this tree.
//
// The base pass shipped the A axis and the items that ship regardless (W-a,
// App-6, App-8, App-11); `ke16-b1` / `ke16-b3` landed in `src/scope.rs`,
// `ke16-w-gate` / `ke16-w-count` in `src/worker.rs`, `src/scope.rs` and
// `src/sync.rs`, and `ke16-c-batch` / `ke16-w-fanout` (`Scope::spawn_batch`,
// `worker::push_task_no_wake`, `worker::wake_up_to`, and the production
// `par_*` / physics callers) with the C axis.
//
// Each axis carried a `compile_error!` refusing its own feature until its arm
// was here, and deleted that refusal in the commit that landed the arm. The
// refusal was not tidiness: a feature whose arm is missing produces a binary
// identical to the default build while `KE16_A/B/W/C` still report the
// candidate's token, so `KE16_EXPECT` certifies the run and a default-build
// number is filed against the candidate — the failure the witness exists to
// forbid ("features not actually enabled cannot produce a number",
// `docs/threadpool/KE16-DESIGN.md` §4), and quiet in the one direction that is
// never caught downstream: the number is plausible. The sharpest case was
// `ke16-w-gate` while only HALF of W-b was here (the <=1 push gate without the
// thief-residue cascade): the binary was neither the default nor the candidate.
// A new arm that lands in halves reinstates its own refusal for the same reason.
// =========================================================================
// KE16 witness — which tournament configuration this artifact actually is.
//
// The benches and the two red-first gates print `ke16_variant()` on entry and
// call `ke16_check_expected_variant()`, so "the features were not actually
// enabled" cannot silently produce a number (`KE16-DESIGN.md` §4,
// `KE16-DESIGN-MEASUREMENT.md` §5). Deleted with the features.
// =========================================================================

/// Placement axis (defect A) of this build: `"a0"` (today's `injector_local`
/// placement), `"a1"`, `"a1f"`, `"a2"`, `"a3"` or `"a5"`.
#[cfg(not(any(
    feature = "ke16-a1",
    feature = "ke16-a1-fifo",
    feature = "ke16-a2",
    feature = "ke16-a3"
)))]
pub const KE16_A: &str = "a0";
/// Placement axis (defect A) of this build. See the `a0` arm.
#[cfg(feature = "ke16-a1")]
pub const KE16_A: &str = "a1";
/// Placement axis (defect A) of this build. See the `a0` arm.
#[cfg(feature = "ke16-a1-fifo")]
pub const KE16_A: &str = "a1f";
/// Placement axis (defect A) of this build. See the `a0` arm.
#[cfg(all(feature = "ke16-a2", not(feature = "ke16-a5")))]
pub const KE16_A: &str = "a2";
/// Placement axis (defect A) of this build. See the `a0` arm.
#[cfg(feature = "ke16-a3")]
pub const KE16_A: &str = "a3";
/// Placement axis (defect A) of this build. See the `a0` arm.
#[cfg(feature = "ke16-a5")]
pub const KE16_A: &str = "a5";

/// Joiner axis (defect B) of this build: `"b0"` (today's `scratch` joiner),
/// `"b1"` or `"b3"`.
#[cfg(not(any(feature = "ke16-b1", feature = "ke16-b3")))]
pub const KE16_B: &str = "b0";
/// Joiner axis (defect B) of this build. See the `b0` arm.
#[cfg(feature = "ke16-b1")]
pub const KE16_B: &str = "b1";
/// Joiner axis (defect B) of this build. See the `b0` arm.
#[cfg(feature = "ke16-b3")]
pub const KE16_B: &str = "b3";

/// Wake / completion axis of this build: `"w0"` (wake on every push,
/// unconditional completion unpark), `"wg"`, `"wc"` or `"wgc"`.
#[cfg(not(any(feature = "ke16-w-gate", feature = "ke16-w-count")))]
pub const KE16_W: &str = "w0";
/// Wake / completion axis of this build. See the `w0` arm.
#[cfg(all(feature = "ke16-w-gate", not(feature = "ke16-w-count")))]
pub const KE16_W: &str = "wg";
/// Wake / completion axis of this build. See the `w0` arm.
#[cfg(all(feature = "ke16-w-count", not(feature = "ke16-w-gate")))]
pub const KE16_W: &str = "wc";
/// Wake / completion axis of this build. See the `w0` arm.
#[cfg(all(feature = "ke16-w-gate", feature = "ke16-w-count"))]
pub const KE16_W: &str = "wgc";

/// Batch-spawn axis of this build: `"c0"` (per-task spawn), `"c1"`
/// (`spawn_batch`) or `"c1f"` (`spawn_batch` + the wake fan-out).
///
/// ⚠ **`c1` / `c1f` describe the wave's ACCOUNTING under `ke16-a5`; its WAKE
/// is A5's on every push for which A5 had a bit to claim.** A5's placement
/// wakes the sibling whose idle bit it claimed and reports that no further
/// decision is due, so the wave's one wake decision — `c1`'s saving, and the
/// sole site of `c1f`'s fan-out — is skipped for exactly those waves
/// (`crate::Scope::wake_for_wave`). When the idle mask was empty or the claim
/// CAS lost, the decision IS taken and finds nothing, at the cost of one fence
/// plus one load of the contended `idle` line per wave. The `pending` saving
/// applies throughout. The design declares the axes composable
/// (`KE16-DESIGN.md` §4) and
/// defines C by that decision; until that is resolved, no `c1`/`c1f` row may be
/// filed from a build whose `KE16_A` is `"a5"`.
#[cfg(not(feature = "ke16-c-batch"))]
pub const KE16_C: &str = "c0";
/// Batch-spawn axis of this build. See the `c0` arm.
#[cfg(all(feature = "ke16-c-batch", not(feature = "ke16-w-fanout")))]
pub const KE16_C: &str = "c1";
/// Batch-spawn axis of this build. See the `c0` arm.
#[cfg(feature = "ke16-w-fanout")]
pub const KE16_C: &str = "c1f";

/// Whether [`Scope::spawn_batch`] batches this wave (`ke16-c-batch`) or falls
/// back to one [`Scope::spawn`] per body.
///
/// A `const` rather than a `cfg` the callers repeat, because a caller in
/// ANOTHER crate cannot see this crate's features: a pass-through feature on
/// `boyko-ecs` / `boyko-physics` would leave the measured command
/// (`--features boyko-threadpool/ke16-c-batch`, `KE16-DESIGN.md` §4) enabling
/// the pool's arm while the callers still spawned per task — the witness would
/// say `c1` over a `c0` caller, which is the mislabelling the witness exists to
/// forbid. Reading it instead makes every caller follow THIS crate's feature.
///
/// It exists for the one caller shape that must do extra work to produce a
/// batch count — the physics dispatches, whose chunk cuts are data-dependent
/// and have to be walked once to be counted (`KE16-DESIGN-W.md` §4.2). Branching
/// on a `const` costs nothing: the un-batched build folds the count pass away
/// rather than paying it for a value it would not use. Deleted with the
/// features.
pub const KE16_SPAWN_BATCH: bool = cfg!(feature = "ke16-c-batch");

/// The tournament configuration of this build as `"{A}+{B}+{W}+{C}"`, e.g.
/// `"a0+b0+w0+c0"` for the default build.
///
/// Allocating is deliberate: this is called once on bench or gate entry, never
/// on a pool path.
pub fn ke16_variant() -> String {
    format!("{KE16_A}+{KE16_B}+{KE16_W}+{KE16_C}")
}

/// When `KE16_EXPECT` is set in the environment, panic unless it names exactly
/// this build; otherwise do nothing.
///
/// The measurement protocol sets `KE16_EXPECT` on every recorded run, so a
/// number can never come from a build whose `--features` line did not take
/// (`KE16-DESIGN-MEASUREMENT.md` §5 item 3). An unset variable is allowed for a
/// developer smoke run and its numbers are not recorded.
///
/// Emitting the banner is the CALLER's job, not this function's
/// (`KE16-DESIGN.md` §4 assigns the print to the benches and the gates): a
/// library source that writes to stdout reds `boyko-log`'s print census, whose
/// allowlist admits exactly one reason — a record that would otherwise be
/// invisible because the next statement ends the process — and a variant banner
/// is not it.
///
/// # Panics
/// If `KE16_EXPECT` is set and differs from [`ke16_variant`].
pub fn ke16_check_expected_variant() {
    let actual = ke16_variant();
    if let Ok(expected) = std::env::var("KE16_EXPECT")
        && expected != actual
    {
        panic!(
            "KE16_EXPECT mismatch: the environment asked for `{expected}` but this build is \
             `{actual}` — the `--features` line did not take; the run is void"
        );
    }
}

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
        /// is the build's, not the model's: without `ke16-w-count`, Phase 9.2
        /// Candidate U's unconditional `waker.unpark()` BEFORE the `fetch_sub`
        /// `AcqRel`; with it and a non-null target, the `fetch_sub` first and
        /// the target's `unpark` only on `prev == 1` (KE16 W-d′). M1c drives the
        /// second, which is why its target has to be real enough to count.
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
/// the two cell headers in `task.rs`, the three chunk facts in `block.rs` — so
/// drift between a number here and the thing it describes is a build failure.
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
    /// the scope's `ScopeShared` address. Pinned in `task.rs`.
    pub const SCOPED_CELL_HEADER: usize = 16;

    /// `align_of::<ScopedCell<F>>()` for every `F` whose own alignment does not
    /// exceed it — the cell class's alignment floor, set by the thunk and the
    /// `ScopeShared` address. Pinned in `task.rs`.
    ///
    /// A receipt buckets an allocation on the PAIR `(size, align)`, so this
    /// half needs its own literal: with size pinned alone, a cell class free to
    /// move to another alignment moves out of the bucket a receipt counts over,
    /// and "zero allocations in the cell class" comes back green from an empty
    /// bucket rather than from a clean run.
    pub const SCOPED_CELL_ALIGN: usize = 8;

    /// `size_of::<DetachedCell<F>>() - size_of::<F>()`: the `CellHead` thunk
    /// alone, because a fire-and-forget task has no scope to name. Pinned in
    /// `task.rs`.
    pub const DETACHED_CELL_HEADER: usize = 8;

    /// `align_of::<DetachedCell<F>>()` under the same condition and for the
    /// same reason as [`SCOPED_CELL_ALIGN`]: the thunk alone sets it. Pinned in
    /// `task.rs`.
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
