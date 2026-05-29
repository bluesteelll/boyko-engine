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
//! [`Schedule::run`] reads or writes it. Two fields cross the worker /
//! dispatcher boundary:
//!
//! * `completion_queue` (MPSC `ArrayQueue`) — workers `push`, dispatcher
//!   `pop` inside `apply_window_drain`.
//! * `pending_apply` (`AtomicUsize`) — workers `fetch_add(1, Release)` on
//!   body completion; dispatcher `load(Acquire)` to evaluate the
//!   apply-window gate.
//!
//! The split is documented per-field below; Round 3 O-NEW-2 audit verified
//! that `pred_remaining` is dispatcher-sole-mutator (no worker access),
//! which is why it stays a plain `Box<[u16]>` rather than `Box<[AtomicU16]>`.
//!
//! [`Schedule`]: super::schedule::Schedule
//! [`Schedule::run`]: super::schedule::Schedule::run

use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_queue::ArrayQueue;
use crossbeam_utils::CachePadded;
use fixedbitset::FixedBitSet;

use crate::ecs::core::schedule::conflict_graph::{ConflictGraph, SystemIndex};

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

    /// MPSC completion queue. Workers push their `SystemIndex` on body
    /// completion; the dispatcher pops inside `apply_window_drain`.
    /// `ArrayQueue` is lock-free and bounded — capacity is
    /// `max(system_count, 1)` so the worker's `push` is infallible under
    /// SCH6 (each system completes at most once per frame).
    ///
    /// Crosses the worker/dispatcher boundary; `ArrayQueue: Send + Sync`.
    pub(crate) completion_queue: ArrayQueue<SystemIndex>,

    /// Outstanding apply count.
    ///
    /// * Workers `fetch_add(1, Release)` after `completion_queue.push`,
    ///   publishing the pushed `SystemIndex` together with every byte the
    ///   system body wrote.
    /// * The dispatcher `load(Acquire)` to evaluate the apply-window gate
    ///   (`pending == running.count_ones()`); the Acquire synchronises-with
    ///   every worker's Release, guaranteeing the dispatcher sees all
    ///   body-side writes before calling `apply`.
    /// * After draining `target` completions, the dispatcher does
    ///   `fetch_sub(target, Relaxed)` — Relaxed is fine because the
    ///   dispatcher's own subsequent operations are sequenced behind a
    ///   `&mut self` borrow.
    ///
    /// `CachePadded` so the cross-thread traffic does not false-share with
    /// the adjacent bitsets.
    pub(crate) pending_apply: CachePadded<AtomicUsize>,

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

        // ArrayQueue panics on capacity 0; guard the empty-schedule case.
        let completion_queue = ArrayQueue::new(system_count.max(1));

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
            completion_queue,
            pending_apply: CachePadded::new(AtomicUsize::new(0)),
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

        debug_assert!(
            self.completion_queue.is_empty(),
            "invariant SCH6: completion_queue must drain across frames"
        );
        debug_assert_eq!(
            self.pending_apply.load(Ordering::Relaxed),
            0,
            "invariant SCH6: pending_apply must hit zero before frame boundary"
        );
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
}
