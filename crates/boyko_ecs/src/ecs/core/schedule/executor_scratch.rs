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
//! * `CompletionChannel::pending` (`AtomicUsize`) — workers
//!   `fetch_add(1, Release)` on body completion; dispatcher `load(Acquire)`
//!   to evaluate the apply-window gate.
//!
//! The split is documented per-field below; Round 3 O-NEW-2 audit verified
//! that `pred_remaining` is dispatcher-sole-mutator (no worker access),
//! which is why it stays a plain `Box<[u16]>` rather than `Box<[AtomicU16]>`.
//!
//! [`Schedule`]: super::schedule::Schedule
//! [`Schedule::run`]: super::schedule::Schedule::run

use core::marker::PhantomData;
use core::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_queue::ArrayQueue;
use crossbeam_utils::CachePadded;
use fixedbitset::FixedBitSet;

use crate::ecs::core::schedule::conflict_graph::{ConflictGraph, SystemIndex};

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
    /// crossbeam-`CachePadded`, so they do not false-share with `pending`.
    queue: ArrayQueue<SystemIndex>,
    /// Outstanding apply count. Workers `fetch_add(1, Release)` after `push`;
    /// the dispatcher `load(Acquire)` to gate the apply window and
    /// `fetch_sub(target, Relaxed)` after draining. `CachePadded` so this
    /// cross-thread atomic shares no cache line with `queue`'s indices.
    pending: CachePadded<AtomicUsize>,
}

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
        self.channel().pending.load(order)
    }

    /// Worker post-completion bump: `pending.fetch_add(1, order)` (Release).
    #[inline]
    pub(crate) fn pending_fetch_add(self, order: Ordering) -> usize {
        self.channel().pending.fetch_add(1, order)
    }

    /// Dispatcher post-drain decrement: `pending.fetch_sub(n, order)` (Relaxed).
    #[inline]
    pub(crate) fn pending_fetch_sub(self, n: usize, order: Ordering) -> usize {
        self.channel().pending.fetch_sub(n, order)
    }
}

// SAFETY: the cell carries a `NonNull` to a `CompletionChannel` whose interior
// is entirely `Sync` (`ArrayQueue: Sync`, `CachePadded<AtomicUsize>: Sync`).
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
}

// SAFETY (Phase 9.3c): `ExecutorScratch` lost its auto-derived `Send`/`Sync`
// only because `completion: NonNull<CompletionChannel>` is conservatively
// `!Send`/`!Sync`. That `NonNull` is an OWNING pointer (`Box::into_raw`-derived,
// freed exactly once in `Drop`) to a `CompletionChannel` whose interior is
// `Send + Sync` (`ArrayQueue` + `CachePadded<AtomicUsize>`). It therefore
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
            pending: CachePadded::new(AtomicUsize::new(0)),
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
        }
    }

    /// Resets per-frame state at the top of [`Schedule::run`].
    ///
    /// * Clears `running`, `completed`, `ready_scratch`.
    /// * Restores `pred_remaining[i]` from `conflict_graph.pred_count[i]`.
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

        // The `as_ref()` reads stay INSIDE the `debug_assert!`s so they elide
        // in release (no frame-path cost).
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
