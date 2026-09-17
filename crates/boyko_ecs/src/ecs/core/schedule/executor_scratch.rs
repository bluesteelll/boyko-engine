//! [`ExecutorScratch`] — per-frame mutable executor state.
//!
//! See Phase 9 plan §5.2 / §7.4.1 / §11.1. Wave 5 Step 11 lands the real
//! field set behind the Wave 4 placeholder. The scratch is owned by the
//! [`Schedule`] and reset between frames; its lifetime equals the
//! schedule's.
//!
//! # Mutator discipline
//!
//! Almost every field is **dispatcher-owned** — only the thread calling
//! [`Schedule::run`] reads or writes it. Two pieces of state cross the
//! worker / dispatcher boundary, and they live in a SEPARATE heap allocation
//! ([`CompletionChannel`], Phase 9.3c) reached only through a bare
//! [`NonNull`] so their bytes do not sit inside the `Schedule` allocation
//! covered by the dispatcher's `&mut self` Tree-Borrows protector:
//!
//! * `CompletionChannel::queue` (MPSC `ArrayQueue`) — workers `push`,
//!   dispatcher `pop` inside `apply_window_drain`.
//! * `CompletionChannel::hot.pending` (`AtomicUsize`) — workers
//!   `fetch_add(1, Release)` on body completion; dispatcher `load(Acquire)`
//!   to evaluate the apply-window gate.
//! * `CompletionChannel::hot.panicked` (`AtomicU32`) — a panicking system's
//!   [`SystemRunGuard`] claims it first-wins; the dispatcher reads it once per
//!   round and re-arms it once per frame in [`reset_for_frame`].
//!
//! [`reset_for_frame`]: ExecutorScratch::reset_for_frame
//!
//! The split is documented per-field below; Round 3 O-NEW-2 audit verified
//! that `pred_remaining` is dispatcher-sole-mutator (no worker access),
//! which is why it stays a plain `Box<[u16]>` rather than `Box<[AtomicU16]>`.
//!
//! [`Schedule`]: super::schedule::Schedule
//! [`Schedule::run`]: super::schedule::Schedule::run

use core::marker::PhantomData;
use core::ptr::NonNull;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use boyko_threadpool::InSystemRunGuard;
use crossbeam_queue::ArrayQueue;
use crossbeam_utils::CachePadded;
use fixedbitset::FixedBitSet;

use crate::ecs::core::schedule::conflict_graph::{ConflictGraph, SystemIndex};
use crate::ecs::core::schedule::schedule::completion_queue_overflow;

/// `panicked`'s "no system has panicked this run" sentinel.
///
/// Safe as a sentinel because [`SystemIndex`] is a `u16` newtype: a schedule
/// would have to hold `u32::MAX` systems for a real index to collide, and
/// `ExecutorScratch::new` allocates a `FixedBitSet`, a `[u16]` and an
/// `ArrayQueue` each of `system_count` — `ArrayQueue::new(u32::MAX as usize)`
/// fails long first. `panicked_claim`'s `debug_assert!` gates the mint side;
/// the bound itself is enforced by allocation.
pub(crate) const NO_PANICKED_SYSTEM: u32 = u32::MAX;

/// Cross-thread completion state, heap-allocated so its bytes live OUTSIDE the
/// `Schedule` allocation. `ExecutorScratch` owns it as a bare
/// [`NonNull`] (constructed via `Box::into_raw`, freed via `Box::from_raw` in
/// `Drop`) — NOT a `Box` field, because a `Box` place asserts `Unique`/noalias
/// on its pointee under `&mut self`, which would re-pollute this allocation's
/// Tree-Borrows tag tree. With a bare `NonNull`, the only lineage that reaches
/// these bytes is the non-retagging `NonNull::as_ptr` one shared by the
/// dispatcher, the workers, and the reset asserts (Phase 9.3c). This is the
/// exact relocation `boyko_threadpool::scope::ScopeShared` uses (Phase 9.2).
///
/// `pub(crate)` only because it appears in the `pub(crate)` signatures of
/// `ExecutorScratch::completion` and `CompletionCell::new`; its fields stay
/// private (reached solely through `CompletionCell`'s accessors within this
/// module).
pub(crate) struct CompletionChannel {
    /// MPSC completion queue. Workers `push` their `SystemIndex` on body
    /// completion; the dispatcher `pop`s in `apply_window_drain`. Capacity
    /// `max(system_count, 1)` ⇒ infallible push under SCH6 (one completion per
    /// system per frame). `ArrayQueue: Send + Sync`; its internal head/tail are
    /// crossbeam-`CachePadded`, so they do not false-share with `hot`.
    queue: ArrayQueue<SystemIndex>,
    /// The two words every dispatcher round reads. `CachePadded` so these
    /// cross-thread atomics share no cache line with `queue`'s indices.
    hot: CachePadded<CompletionHot>,
}

/// The two words every dispatcher round reads, in ONE cache line.
///
/// `pending` keeps offset 0 of the padded block, so no existing access
/// changes. `panicked` is written only by a panicking system's guard and read
/// once per round — putting it here means the round's existing `Acquire` load
/// of `pending` has already brought it into L1, so the cancel check costs no
/// second line. The line is already contended by completion traffic; a
/// read-mostly neighbour adds no new sharing.
#[repr(C)]
struct CompletionHot {
    /// Outstanding apply count. Workers `fetch_add(1, Release)` after `push`;
    /// the dispatcher `load(Acquire)` to gate the apply window and
    /// `fetch_sub(n, Relaxed)` after draining.
    pending: AtomicUsize,
    /// `NO_PANICKED_SYSTEM` = none. Claimed first-wins by a panicking system's
    /// `SystemRunGuard::drop`; read once per dispatcher round; cleared ONLY by
    /// `reset_for_frame` (Decision 3b — a cancel-path clear would re-arm the
    /// "a second round mis-fires" hazard).
    panicked: AtomicU32,
    // No explicit tail padding: the `CachePadded` wrapper already owns the
    // false-sharing story, and a hand-written `[u8; 4]` only encodes a 64-bit
    // assumption into a `const` assert, which is a BUILD ERROR rather than a
    // test failure on a 32-bit target (this crate keeps a wasm32 arm).
}

// Target-independent, and the one the claim actually makes: `panicked` sits
// immediately after `pending`, and the pair is inside one cache line. Reds on
// every target if the declaration order is swapped.
const _: () =
    assert!(core::mem::offset_of!(CompletionHot, panicked) == core::mem::size_of::<AtomicUsize>());
const _: () = assert!(core::mem::size_of::<CompletionHot>() <= 64);

// The byte-exact pin, gated the way this tree already gates layout pins
// (`archetype.rs`, `entity_inland.rs`).
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::offset_of!(CompletionHot, panicked) == 8);

/// `Copy` read-only handle on a [`CompletionChannel`].
///
/// Carries a [`NonNull`] that `as_ptr(self)`-copies WITHOUT retagging the
/// pointee (the Phase 9.2 primitive). The accessor takes `self` **by value**
/// (the cell is `Copy`) so no `&self` reborrow retags the carried pointer — the
/// same C1 rationale as [`UnsafeEcsCell`]. Read-only: every write to the
/// channel goes through the channel's own interior mutability, so the cell
/// never forms a `&mut`/`*mut` to the pointee, and `PhantomData<&'a
/// CompletionChannel>` (a SHARED marker, deliberately weaker than
/// `UnsafeEcsCell`'s) is the correct variance.
///
/// [`UnsafeEcsCell`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell
#[derive(Clone, Copy)]
pub(crate) struct CompletionCell<'a> {
    ptr: NonNull<CompletionChannel>,
    _marker: PhantomData<&'a CompletionChannel>,
}

impl<'a> CompletionCell<'a> {
    /// Mints a cell from the owning `NonNull`.
    ///
    /// # Safety
    /// The pointee must outlive `'a` (the caller picks `'a`). The pointer must
    /// be the live, `Box::into_raw`-derived `CompletionChannel` owned by an
    /// `ExecutorScratch` that is not dropped/moved for `'a`.
    #[inline]
    pub(crate) unsafe fn new(ptr: NonNull<CompletionChannel>) -> Self {
        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Returns a shared reference to the channel.
    ///
    /// # Safety
    /// Upholds the `new` contract: the pointee is live for `'a`. The by-value
    /// receiver consumes the `Copy` cell, so no `&self` retag occurs; the
    /// returned `&CompletionChannel` permits only interior-mutable access
    /// (`queue.push`/`pop`, `pending.fetch_add`/`load`/`fetch_sub`), which is
    /// the channel's designed MPSC contract.
    #[inline]
    fn channel(self) -> &'a CompletionChannel {
        // SAFETY: `NonNull::as_ptr` copies the address without retagging the
        //   pointee (Phase 9.2 primitive); the pointee is live for `'a` per
        //   `new`'s contract. By-value `self` (Copy) means no `&self` reborrow
        //   downgrades the pointer to SharedReadOnly before the deref.
        unsafe { &*self.ptr.as_ptr() }
    }

    /// Worker completion push (interior-mutable `&self` op on the queue).
    /// Returns the index back in `Err` if the queue is full (unreachable under
    /// SCH6: capacity `>= system_count`, one push per system per frame).
    #[inline]
    pub(crate) fn push(self, idx: SystemIndex) -> Result<(), SystemIndex> {
        self.channel().queue.push(idx)
    }

    /// Dispatcher completion pop (interior-mutable `&self` op on the queue).
    #[inline]
    pub(crate) fn pop(self) -> Option<SystemIndex> {
        self.channel().queue.pop()
    }

    /// `true` iff the completion queue is currently empty (SCH6 cross-frame
    /// drain assert).
    ///
    /// Used only in `debug_assert!` / test contexts (SCH6), which elide in
    /// release — hence `#[allow(dead_code)]` so the release lib stays clean.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn queue_is_empty(self) -> bool {
        self.channel().queue.is_empty()
    }

    /// Loads the outstanding-apply counter with `order` (dispatcher Acquire on
    /// the apply-window gate; Relaxed on the SCH6 asserts).
    #[inline]
    pub(crate) fn pending_load(self, order: Ordering) -> usize {
        self.channel().hot.pending.load(order)
    }

    /// Worker post-completion bump: `pending.fetch_add(1, order)` (Release).
    #[inline]
    pub(crate) fn pending_fetch_add(self, order: Ordering) -> usize {
        self.channel().hot.pending.fetch_add(1, order)
    }

    /// Dispatcher post-drain decrement: `pending.fetch_sub(n, order)` (Relaxed).
    #[inline]
    pub(crate) fn pending_fetch_sub(self, n: usize, order: Ordering) -> usize {
        self.channel().hot.pending.fetch_sub(n, order)
    }

    /// Reads the cancel flag — `NO_PANICKED_SYSTEM` when no system of this run
    /// has panicked.
    ///
    /// `Relaxed` is sufficient and the reason is an ordering, not a
    /// coincidence: the dispatcher performs this load immediately after its
    /// `Acquire` load of `pending`, and a panicking guard stores this flag
    /// BEFORE its `Release` `fetch_add` on `pending`. The `Acquire` therefore
    /// carries the edge for both words (Decision 4).
    #[inline]
    pub(crate) fn panicked_load(self, order: Ordering) -> u32 {
        self.channel().hot.panicked.load(order)
    }

    /// Claims the cancel flag for `idx`, first writer wins.
    ///
    /// Called only from a panicking [`SystemRunGuard`]'s `Drop`, before that
    /// guard publishes its completion. `Relaxed` on both arms: the edge is
    /// carried by the `Release` `fetch_add` sequenced after this call.
    #[inline]
    pub(crate) fn panicked_claim(self, idx: SystemIndex) {
        debug_assert!(
            u32::from(idx.0) < NO_PANICKED_SYSTEM,
            "invariant: a real SystemIndex must never equal the NO_PANICKED_SYSTEM sentinel"
        );
        let _ = self.channel().hot.panicked.compare_exchange(
            NO_PANICKED_SYSTEM,
            u32::from(idx.0),
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }

    /// Re-arms the cancel flag for a new run. `reset_for_frame` ONLY
    /// (Decision 3b).
    ///
    /// `Relaxed`: the store runs in the single-threaded window between frames
    /// (no worker is alive) and is sequenced-before every `scope.spawn` of the
    /// run that follows, on the same thread.
    #[inline]
    pub(crate) fn panicked_reset(self) {
        self.channel()
            .hot
            .panicked
            .store(NO_PANICKED_SYSTEM, Ordering::Relaxed);
    }
}

// SAFETY: the cell carries a `NonNull` to a `CompletionChannel` whose interior
// is entirely `Sync` (`ArrayQueue: Sync`, `CachePadded<CompletionHot>: Sync`,
// because `CompletionHot` is two atomics and nothing else).
// Concurrent access through `Copy`s of the cell is the channel's MPSC contract:
// many workers `push`/`fetch_add(Release)`, one dispatcher `pop`/`load(Acquire)`
// /`fetch_sub`. The allocation outlives every copy for `'a` (owned by the
// `ExecutorScratch` that minted the cell; `Scope::Drop` blocks every worker
// before the install frame — hence the cell's `'a` — ends). This is the
// read-only analogue of `boyko_threadpool::scope`'s shared-`Sync`-pointee
// argument; no `&mut`/aliasing-discipline contract is needed because the cell
// never yields a `&mut`.
unsafe impl<'a> Send for CompletionCell<'a> {}
unsafe impl<'a> Sync for CompletionCell<'a> {}

/// One system's run obligation: restore the in-system TLS depth, then publish
/// the completion — on BOTH the normal and the unwinding path, in that order.
///
/// Stack-only; never escapes the spawned closure. The defect this type exists
/// to remove is that the publish used to be two *statements after the body*
/// rather than an *obligation of the frame*: a panicking system left `pending`
/// short of `running` forever, the apply window never opened, and the
/// dispatcher parked for the rest of the process.
///
/// # SP2 — exactly one publish per guard
///
/// `finish` publishes and disarms; `Drop` returns early when disarmed. The two
/// paths are mutually exclusive by the flag rather than by reading order. A
/// double publish would be the one way to overflow the completion queue, whose
/// capacity proof is `ArrayQueue::new(system_count.max(1))` × one publish per
/// system per frame × SCH6's between-frames emptiness.
///
/// # SP3 — the completion publish is NOT what delivers the payload
///
/// This guard publishes inside the task body, strictly before the pool's own
/// `catch_unwind` returns; the payload reaches the scope's slot in
/// `capture_panic`, which is sequenced before `complete_task`. So
/// `Scope::drop`'s join — not this accounting — is what closes delivery
/// (SCH-A6-1). Two edits would silently break that and neither is visible
/// here: moving the publish after the pool's catch, or shortening
/// `Scope::drop`'s join when a payload is already present.
pub(crate) struct SystemRunGuard<'a> {
    /// The channel this guard owes a completion to.
    completion: CompletionCell<'a>,
    /// The system whose run this guard brackets.
    idx: SystemIndex,
    /// The allocation-discipline guard (ALLOC1/ALLOC6), dropped FIRST inside
    /// this guard so the TLS depth is restored before the publish.
    alloc: Option<InSystemRunGuard>,
    /// `true` while the completion is still owed.
    armed: bool,
    /// The in-system depth at `enter`. An App-8-safe DELTA, not an absolute: a
    /// helping joiner may legitimately run a sibling system inline inside
    /// another system's body, so `depth == 0` is the wrong assertion.
    #[cfg(debug_assertions)]
    depth0: u32,
}

impl<'a> SystemRunGuard<'a> {
    /// Enters a system run: raises the in-system TLS depth and arms the
    /// completion obligation.
    #[inline]
    pub(crate) fn enter(completion: CompletionCell<'a>, idx: SystemIndex) -> Self {
        // Read BEFORE the guard raises the depth, so `finish`'s assert compares
        // the restored depth against the one this frame inherited.
        #[cfg(debug_assertions)]
        let depth0 = boyko_threadpool::system_run_depth();
        Self {
            completion,
            idx,
            alloc: Some(InSystemRunGuard::enter()),
            armed: true,
            #[cfg(debug_assertions)]
            depth0,
        }
    }

    /// The normal path: leave the system run, publish the completion, disarm.
    ///
    /// Taken by value; the (now disarmed) guard drops at the end of this body.
    #[inline]
    pub(crate) fn finish(mut self) {
        // SP1': the TLS depth is restored BEFORE the completion is published,
        // so a dispatcher that observes `pending == running` cannot still find
        // a worker inside a system body.
        self.alloc = None;
        #[cfg(debug_assertions)]
        debug_assert_eq!(
            boyko_threadpool::system_run_depth(),
            self.depth0,
            "invariant SP1': the in-system depth must be restored before the completion is \
             published"
        );
        debug_assert!(self.armed, "invariant SP2: a guard publishes exactly once");

        // NOT a cleanup pad. Byte-for-byte the publish this closure performed
        // before the guard existed. The `expect` is unreachable (SP2 ×
        // `ArrayQueue::new(system_count.max(1))` × SCH6); were it to fire,
        // `self.armed` is still true, so the unwind runs this guard's `Drop`,
        // whose retried push fails the same way and ends in
        // `completion_queue_overflow`'s `E1503` abort — the panic never reaches
        // the pool's `run_scoped`.
        self.completion
            .push(self.idx)
            .expect("invariant SCH6: completion_queue cap ≥ system_count");
        self.completion.pending_fetch_add(Ordering::Release);
        self.armed = false;
    }
}

impl Drop for SystemRunGuard<'_> {
    #[inline]
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        // The unwinding path. No `debug_assert` that can fire is reachable from
        // here, and that is a reachability claim rather than a placement rule:
        // `alloc = None` runs `InSystemRunGuard::drop`'s `depth > 0` assert,
        // which cannot fire because this guard's own `enter` raised it; the
        // push's capacity proof is SP2 × `ArrayQueue::new(system_count.max(1))`
        // × SCH6. A `debug_assert` that fired in a cleanup pad would abort.
        self.alloc = None;
        self.completion.panicked_claim(self.idx);

        // `expect` here would be an abort with the message "panic in a
        // destructor during cleanup", and a DROPPED completion would be the
        // hang this guard exists to remove — so the overflow gets its own
        // terminal site that says which invariant broke.
        if self.completion.push(self.idx).is_err() {
            completion_queue_overflow(self.idx);
        }
        self.completion.pending_fetch_add(Ordering::Release);
        self.armed = false;
    }
}

/// Per-frame executor scratch reused across [`Schedule::run`] calls.
///
/// See plan §5.2 / §7.4.1. Field order is shuffled vs the plan pseudocode
/// to keep the hot dispatcher bits (`running` / `completed` /
/// `pred_remaining`) on the prefix; the cache-padded atomic sits at the
/// tail so its line cannot false-share with any of the bitsets.
///
/// [`Schedule::run`]: super::schedule::Schedule::run
#[allow(dead_code)] // populated by Wave 5 Step 12 executor; fields read on hot path.
pub(crate) struct ExecutorScratch {
    /// Bit `i` is set iff system `i` is currently dispatched to a worker
    /// (or running on the dispatcher for an exclusive system). Cleared
    /// when the dispatcher pops the system's completion in
    /// `apply_window_drain`.
    ///
    /// **Dispatcher-owned** — workers never touch this. The bitset is
    /// `FixedBitSet`, which is not atomic; cross-thread observation would
    /// be UB without the apply-window barrier.
    pub(crate) running: FixedBitSet,

    /// Bit `i` is set iff system `i` has both run and applied this frame.
    /// Reset at the top of every `Schedule::run` via `reset_for_frame`.
    ///
    /// **Dispatcher-owned**.
    pub(crate) completed: FixedBitSet,

    /// Scratch buffer reused by `try_dispatch_ready` to accumulate ready
    /// systems without re-allocating per round. Plan §13.6 W7 post-condition
    /// (`running ∩ ready_scratch is empty`) is checked at end of dispatch.
    ///
    /// **Dispatcher-owned**.
    pub(crate) ready_scratch: FixedBitSet,

    /// Plain `u16` per system (Round 3 O-NEW-2). Initialised to
    /// `conflict_graph.pred_count[i]` at frame start; decremented
    /// non-atomically by the dispatcher on each predecessor completion in
    /// `apply_window_drain` (or in the exclusive-system branch of
    /// `try_dispatch_ready`). When a counter hits 0, system `i` becomes
    /// dispatchable.
    ///
    /// **Dispatcher-owned** — workers never touch this. The audit at plan
    /// §7.4.3 verified the discipline; saving the LOCK prefix on x86 is a
    /// minor speedup and clarifies "single-thread state" at the type level.
    pub(crate) pred_remaining: Box<[u16]>,

    /// Cross-thread completion state in its OWN heap allocation (Phase 9.3c TB
    /// hardening). A bare `NonNull` (NOT a `Box` field — see `CompletionChannel`
    /// docs). Allocated once in `new`, reused every frame, freed once in `Drop`.
    /// Accessed by dispatcher AND workers EXCLUSIVELY through a `CompletionCell`
    /// (or `NonNull::as_ref` for the single-threaded reset/asserts) — never
    /// through a reborrow that forms a `&CompletionChannel` to the pointee under
    /// `&mut self`.
    ///
    /// `pub(crate)` only so the dispatcher (`schedule.rs`) can READ the `Copy`
    /// `NonNull` value to mint a [`CompletionCell`] — the pointee is reached
    /// solely through that cell's non-retagging accessors, never by naming this
    /// field's pointee directly outside this module.
    pub(crate) completion: NonNull<CompletionChannel>,

    /// System count baked in at construction. Equal to
    /// `Schedule::systems.len()`. Stays consistent for the schedule's
    /// lifetime — Phase 9 does not support post-build mutation (SCH1).
    pub(crate) system_count: usize,

    /// Phase 16 — per-frame "conditions folded" memo. Bit `i` is set once
    /// system `i`'s own + gating-set conditions have been evaluated this
    /// frame, preventing a re-fold (which would advance a stateful
    /// condition's `Local` more than once — e.g. `run_once`). Reset in
    /// [`reset_for_frame`](Self::reset_for_frame). See `PHASE-16-PLAN.md`
    /// §3.3 / §7.3.
    ///
    /// **Dispatcher-owned** — touched only inside `evaluate_ready_conditions`.
    pub(crate) cond_evaluated: FixedBitSet,

    /// Phase 16 — per-frame set-condition memo flag. Bit `slot` is set once
    /// the set-condition row at that dense `slot` has run this frame; the
    /// result is cached in `set_cond_result[slot]`. A set condition gates
    /// every member, so it runs exactly ONCE per frame regardless of member
    /// count (§7.1). Zero-length when no set carries a condition.
    ///
    /// **Dispatcher-owned**.
    pub(crate) set_cond_evaluated: FixedBitSet,

    /// Phase 16 — per-frame set-condition result cache. Bit `slot` holds the
    /// `bool` verdict of the set-condition row at that dense `slot`, valid
    /// only while `set_cond_evaluated[slot]` is set. Reset each frame so the
    /// verdict is re-derived (§7.3).
    ///
    /// **Dispatcher-owned**.
    pub(crate) set_cond_result: FixedBitSet,

    /// Reusable scratch for the dispatcher-only systems accepted in a dispatch
    /// round. `try_dispatch_ready` `mem::take`s it, fills it, drains it, and
    /// restores the (now-empty) buffer to reuse the heap allocation next round —
    /// eliminating the per-round `Vec::new()` that was the executor's only
    /// hot-path allocation. Capacity ≤ `system_count`.
    ///
    /// **Dispatcher-owned**.
    pub(crate) exclusive_to_run: Vec<SystemIndex>,

    /// Reusable scratch for the concurrent systems spawned in a dispatch round.
    /// Same `mem::take`/refill/restore lifecycle as `exclusive_to_run`; the
    /// spawn loop consumes it by value and the drained buffer is restored for
    /// the next round. Capacity ≤ `system_count`.
    ///
    /// **Dispatcher-owned**.
    pub(crate) to_spawn: Vec<SystemIndex>,
}

// SAFETY (Phase 9.3c): `ExecutorScratch` lost its auto-derived `Send`/`Sync`
// only because `completion: NonNull<CompletionChannel>` is conservatively
// `!Send`/`!Sync`. That `NonNull` is an OWNING pointer (`Box::into_raw`-derived,
// freed exactly once in `Drop`) to a `CompletionChannel` whose interior is
// `Send + Sync` (`ArrayQueue` + `CachePadded<CompletionHot>`, the latter two
// atomics and nothing else). It therefore
// behaves exactly as the `Box<CompletionChannel>` it stands in for would (which
// would be auto-`Send + Sync`); the bare `NonNull` is used ONLY to avoid the
// `Box`-place `Unique`-retag under Tree Borrows. Every other field is already
// `Send + Sync`, and the dispatcher is the sole owner. Restoring these impls
// keeps `Schedule: Send` so `ThreadPool::install`'s `F: Send` dispatcher
// closure (which captures `&mut Schedule`) type-checks exactly as before this
// phase — no new cross-thread sharing of `ExecutorScratch` is introduced.
unsafe impl Send for ExecutorScratch {}
unsafe impl Sync for ExecutorScratch {}

#[allow(dead_code)] // consumed by Wave 5 Step 12 executor.
impl ExecutorScratch {
    /// Allocates a scratch sized for `system_count` systems and seeds
    /// `pred_remaining` from the conflict graph's `pred_count`.
    ///
    /// `set_condition_count` (Phase 16) sizes the set-condition memo bitsets
    /// (`set_cond_evaluated` / `set_cond_result`); it is `0` for a schedule
    /// with no set-level `.run_if`, in which case those bitsets are
    /// zero-length and every memo `clear()` is a no-op.
    ///
    /// Called once per schedule from `ScheduleBuilder::build`. Subsequent
    /// frames reuse the same allocation via [`reset_for_frame`](Self::reset_for_frame).
    pub(crate) fn new(
        system_count: usize,
        set_condition_count: usize,
        conflict_graph: &ConflictGraph,
    ) -> Self {
        let running = FixedBitSet::with_capacity(system_count);
        let completed = FixedBitSet::with_capacity(system_count);
        let ready_scratch = FixedBitSet::with_capacity(system_count);

        // Seed `pred_remaining` from the baseline. Subsequent frames
        // restore the same baseline via `reset_for_frame`.
        let mut pred_remaining_vec: Vec<u16> = Vec::with_capacity(system_count);
        pred_remaining_vec.extend_from_slice(&conflict_graph.pred_count);
        let pred_remaining = pred_remaining_vec.into_boxed_slice();

        // Phase 9.3c: heap-allocate the cross-thread channel and own it as a
        // bare NonNull (Box::into_raw transfers ownership; Drop reclaims it).
        // ArrayQueue panics on capacity 0; guard the empty-schedule case.
        let completion_box = Box::new(CompletionChannel {
            queue: ArrayQueue::new(system_count.max(1)),
            hot: CachePadded::new(CompletionHot {
                pending: AtomicUsize::new(0),
                panicked: AtomicU32::new(NO_PANICKED_SYSTEM),
            }),
        });
        // SAFETY: `Box::into_raw` yields a non-null, properly-aligned, live
        //   pointer; ownership is transferred to `self.completion` and freed
        //   exactly once in `Drop for ExecutorScratch`.
        let completion = unsafe { NonNull::new_unchecked(Box::into_raw(completion_box)) };

        // Phase 16 — per-frame condition memos. `cond_evaluated` is sized by
        // system count (one bit per system); the set memos by row count.
        let cond_evaluated = FixedBitSet::with_capacity(system_count);
        let set_cond_evaluated = FixedBitSet::with_capacity(set_condition_count);
        let set_cond_result = FixedBitSet::with_capacity(set_condition_count);

        // Preallocate the per-round dispatch buffers once. Both are bounded by
        // the system count and reused (cleared, never reallocated) every round.
        let exclusive_to_run = Vec::with_capacity(system_count);
        let to_spawn = Vec::with_capacity(system_count);

        Self {
            running,
            completed,
            ready_scratch,
            pred_remaining,
            completion,
            system_count,
            cond_evaluated,
            set_cond_evaluated,
            set_cond_result,
            exclusive_to_run,
            to_spawn,
        }
    }

    /// Resets per-frame state at the top of [`Schedule::run`].
    ///
    /// * Clears `running`, `completed`, `ready_scratch`.
    /// * Restores `pred_remaining[i]` from `conflict_graph.pred_count[i]`.
    /// * Re-arms the cancel flag (`panicked` ← `NO_PANICKED_SYSTEM`) — the ONE
    ///   production site that clears it (Decision 3b).
    /// * `debug_assert!`s that the previous frame fully drained — both
    ///   `completion_queue` and `pending_apply` must be empty / zero.
    ///
    /// The completion queue and `pending_apply` are NOT cleared by this
    /// method — they MUST be empty across frames per SCH6 (every system
    /// completes exactly once per frame, and the apply window pops every
    /// completion before the loop exits).
    ///
    /// Cold-ish: runs once per frame, not per round. Tagging `#[cold]`
    /// would mislead — `Schedule::run` itself is the cold context.
    ///
    /// [`Schedule::run`]: super::schedule::Schedule::run
    pub(crate) fn reset_for_frame(&mut self, conflict_graph: &ConflictGraph) {
        self.running.clear();
        self.completed.clear();
        self.ready_scratch.clear();

        // Phase 16 — clear the per-frame condition memos so stateful
        // conditions are folded once next frame (§7.3). All three are
        // zero-length when the schedule carries no conditions, so the
        // clears are no-ops on the 0%-gate path.
        self.cond_evaluated.clear();
        self.set_cond_evaluated.clear();
        self.set_cond_result.clear();

        // Plain slice copy; both are `[u16]` of length `system_count`.
        // The `?` here is purely defensive — `ScheduleBuilder::build`
        // guarantees the lengths match.
        debug_assert_eq!(
            self.pred_remaining.len(),
            conflict_graph.pred_count.len(),
            "invariant SCH13: pred_remaining length must match conflict_graph.pred_count"
        );
        for (slot, &count) in self
            .pred_remaining
            .iter_mut()
            .zip(conflict_graph.pred_count.iter())
        {
            *slot = count;
        }

        // Decision 3b — re-arm the cancel flag, and NOWHERE else. The slot is
        // first-wins and sticky by construction, so without a per-frame clear
        // every later run of this schedule would cancel in round 1 and return
        // normally having dispatched nothing: a silent no-op frame, with no
        // symptom the caller can catch, and strictly worse than the hang this
        // design removes. This is the channel's one UNCONDITIONAL touch here —
        // one `Relaxed` `u32` store per `Schedule::run`, the same frame-path
        // cost class as the bitset `clear()`s above.
        //
        // SAFETY (Phase 9.3c, and now also on the release path): between frames
        //   no worker is alive — every `Schedule::run` joins via `Scope::Drop`
        //   before returning — so this store races nothing. Same non-retagging
        //   `as_ptr` lineage as every other access; no `Box` place is named.
        unsafe { CompletionCell::new(self.completion) }.panicked_reset();

        // The two SCH6 `as_ref()` reads stay INSIDE the `debug_assert!`s so
        // they elide in release; the `panicked` re-arm above is the one access
        // on the frame path.
        //
        // SAFETY (Phase 9.3c, both reads): between frames, no worker is alive
        //   (every `Schedule::run` joins all workers via `Scope::Drop` before
        //   returning), so reading the channel through the owning `NonNull`
        //   races nothing. `as_ref` goes through the non-retagging `as_ptr`
        //   lineage — the SAME lineage the dispatcher/workers use — so no
        //   foreign tag is introduced into the heap allocation; in particular
        //   no `Box` place (there is none) `Unique`-retags the pointee under
        //   `&mut self`.
        debug_assert!(
            unsafe { self.completion.as_ref() }.queue.is_empty(),
            "invariant SCH6: completion_queue must drain across frames"
        );
        debug_assert_eq!(
            unsafe { self.completion.as_ref() }
                .hot
                .pending
                .load(Ordering::Relaxed),
            0,
            "invariant SCH6: pending_apply must hit zero before frame boundary"
        );
    }
}

impl Drop for ExecutorScratch {
    /// Frees the `Box::into_raw`-leaked [`CompletionChannel`] exactly once.
    ///
    /// `ExecutorScratch` is owned by `Schedule`; this `Drop` runs when the
    /// schedule drops — after the last frame, with no worker alive (every
    /// `Schedule::run` joins all workers via `Scope::Drop` before returning).
    /// So the reclaimed allocation has no live cross-thread reference.
    fn drop(&mut self) {
        // SAFETY: `self.completion` was minted via `Box::into_raw(Box::new(..))`
        //   in `new` and never reassigned; this is the sole `Box::from_raw`, so
        //   the allocation is freed exactly once. No worker holds a
        //   `CompletionCell` into it at drop time (single-free-site discipline,
        //   mirrors `boyko_threadpool::scope::Scope::drop`).
        unsafe {
            drop(Box::from_raw(self.completion.as_ptr()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::schedule::conflict_graph::SystemIndex;
    use crate::ecs::core::schedule::system_box::SystemBox;
    use crate::ecs::core::schedule::system_descriptor::SystemDescriptor;
    use crate::ecs::core::system::access::Access;
    use crate::ecs::core::system::system::System;
    use crate::ecs::core::system::system_meta::SystemMeta;
    use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
    use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

    /// Minimal `System` impl for the scratch unit tests — empty body, no
    /// access. The conflict graph code does not consume `run_unsafe`, so
    /// the body is vacuous.
    struct ProbeSystem {
        meta: SystemMeta,
    }

    // SAFETY (S1): `run_unsafe` is empty; the trait contract is vacuous.
    unsafe impl System for ProbeSystem {
        type Out = ();
        fn name(&self) -> &'static str {
            self.meta.name()
        }
        fn access(&self) -> &Access {
            self.meta.access()
        }
        fn initialize(&mut self, _world: &mut EcsMaster) {}
        unsafe fn run_unsafe(&mut self, _world: UnsafeEcsCell<'_>) -> Self::Out {}
        fn meta(&self) -> &SystemMeta {
            &self.meta
        }
        fn set_change_ticks(
            &mut self,
            last_run: crate::ecs::core::change_detection::Tick,
            this_run: crate::ecs::core::change_detection::Tick,
        ) {
            self.meta.last_run = last_run;
            self.meta.this_run = this_run;
        }
        fn check_change_tick(&mut self, current: crate::ecs::core::change_detection::Tick) {
            self.meta.last_run = self.meta.last_run.check_tick(current);
            self.meta.this_run = self.meta.this_run.check_tick(current);
        }
    }

    fn descriptor(name: &'static str) -> SystemDescriptor {
        let sys = ProbeSystem {
            meta: SystemMeta::for_testing(name),
        };
        let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
        SystemDescriptor::new(SystemBox::new(boxed))
    }

    /// `new` allocates the bitsets to `system_count` and seeds
    /// `pred_remaining` from the conflict graph's baseline.
    #[test]
    fn new_seeds_pred_remaining_from_graph() {
        let descs = vec![descriptor("a"), descriptor("b"), descriptor("c")];
        // a -> c, b -> c
        let edges = vec![
            (SystemIndex(0), SystemIndex(2)),
            (SystemIndex(1), SystemIndex(2)),
        ];
        let graph = ConflictGraph::build(&descs, &edges);
        let scratch = ExecutorScratch::new(3, 0, &graph);
        assert_eq!(scratch.system_count, 3);
        assert_eq!(scratch.pred_remaining[0], 0);
        assert_eq!(scratch.pred_remaining[1], 0);
        assert_eq!(scratch.pred_remaining[2], 2);
        assert!(scratch.running.count_ones(..) == 0);
        assert!(scratch.completed.count_ones(..) == 0);
    }

    /// `reset_for_frame` restores `pred_remaining` to the graph baseline
    /// after manual mutation, and clears the running/completed bitsets.
    #[test]
    fn reset_restores_pred_remaining_and_clears_bitsets() {
        let descs = vec![descriptor("a"), descriptor("b")];
        let edges = vec![(SystemIndex(0), SystemIndex(1))];
        let graph = ConflictGraph::build(&descs, &edges);
        let mut scratch = ExecutorScratch::new(2, 0, &graph);

        scratch.pred_remaining[1] = 0;
        scratch.running.insert(0);
        scratch.completed.insert(0);
        scratch.ready_scratch.insert(1);

        scratch.reset_for_frame(&graph);

        assert_eq!(scratch.pred_remaining[0], 0);
        assert_eq!(scratch.pred_remaining[1], 1);
        assert!(!scratch.running.contains(0));
        assert!(!scratch.completed.contains(0));
        assert!(!scratch.ready_scratch.contains(1));
    }

    /// Empty schedule still allocates a usable scratch (ArrayQueue capacity
    /// must be > 0; the constructor uses `.max(1)`).
    #[test]
    fn empty_schedule_does_not_panic() {
        let descs: Vec<SystemDescriptor> = Vec::new();
        let graph = ConflictGraph::build(&descs, &[]);
        let scratch = ExecutorScratch::new(0, 0, &graph);
        assert_eq!(scratch.system_count, 0);
        assert_eq!(scratch.pred_remaining.len(), 0);
    }

    /// Phase 9.3c: `CompletionCell` is `Copy` and round-trips a `SystemIndex`
    /// through the shared channel (single-threaded). Two copies of the cell
    /// observe the same channel allocation.
    #[test]
    fn completion_cell_round_trips() {
        let descs = vec![descriptor("a")];
        let graph = ConflictGraph::build(&descs, &[]);
        let scratch = ExecutorScratch::new(1, 0, &graph);
        // SAFETY: `scratch` (hence the `Box::into_raw`-owned channel) outlives
        //   every cell use below; no worker exists in this single-threaded test.
        let cell = unsafe { CompletionCell::new(scratch.completion) };
        let copy_a = cell;
        let copy_b = cell;
        copy_a.channel().queue.push(SystemIndex(0)).expect("push");
        assert_eq!(copy_b.channel().queue.pop(), Some(SystemIndex(0)));
        assert_eq!(
            copy_a.channel() as *const CompletionChannel,
            copy_b.channel() as *const CompletionChannel,
            "Copy cells reference the same channel allocation"
        );
    }
}
