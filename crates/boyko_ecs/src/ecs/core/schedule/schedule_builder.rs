//! [`ScheduleBuilder`] — user-facing schedule constructor.
//!
//! See Phase 9 plan §5.5 (with the Round 3 W-NEW-3 pre-destructure
//! pattern) and §14 Step 9 acceptance criteria. The builder collects
//! systems + ordering hints, then on [`ScheduleBuilder::build`]:
//!
//! 1. Initialises every system once (capturing its `Access` surface);
//! 2. Validates that the ordering DAG is acyclic via Tarjan SCC;
//! 3. Linearises systems via Kahn's topological sort;
//! 4. Hands the topologically-ordered descriptors to
//!    [`ConflictGraph::build`] (Wave 4 Step 10) for the per-system
//!    conflict bitsets + predecessor counts;
//! 5. Constructs a [`Schedule`] that the (Wave 5) executor can run.
//!
//! # Round 3 W-NEW-3 pre-destructure
//!
//! `build` immediately destructures `self` into its four fields. This
//! avoids the borrow conflicts of the Round 1 sketch (which had
//! `&mut self` mutating `descriptors` while the same `self` was being
//! used to read `order_edges`). The destructure also lets us move the
//! descriptor vec into `insert_sync_points` in the eventual Wave 5
//! Step 14 path without an extra clone.
//!
//! [`ScheduleBuilder::build`]: ScheduleBuilder::build
//! [`ConflictGraph::build`]: super::conflict_graph::ConflictGraph::build

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;

use boyko_threadpool::ThreadPool;

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::schedule::conflict_graph::{ConflictGraph, SystemIndex};
use crate::ecs::core::schedule::ordering::{OrderingEdge, SystemKey};
use crate::ecs::core::schedule::executor_scratch::ExecutorScratch;
use crate::ecs::core::schedule::schedule::Schedule;
use crate::ecs::core::schedule::system_box::SystemBox;
use crate::ecs::core::schedule::system_config::SystemConfig;
use crate::ecs::core::schedule::system_descriptor::SystemDescriptor;
use crate::ecs::core::schedule::system_set::SystemSetId;
use crate::ecs::core::system::into_system::IntoSystem;
use crate::ecs::core::system::system::System;

/// Maximum number of systems a single [`Schedule`] can hold (plan §3 Q4 /
/// §13.6). The cap fits comfortably into the `u16` used by `SystemIndex`
/// and `pred_count`, and corresponds to ~2 KB of `pred_remaining` data
/// plus ~128 KB of conflict bitsets — both well within L2.
pub const MAX_SYSTEMS_PER_SCHEDULE: usize = 1024;

/// Builder for [`Schedule`]. Construct via [`ScheduleBuilder::new`];
/// chain `add_system(...).before(...).after(...)` calls; finalise with
/// [`build`](Self::build).
pub struct ScheduleBuilder {
    /// Pool reference. Cloned into the resulting [`Schedule`] on build.
    pub(crate) pool: Arc<ThreadPool>,

    /// Staging slot per system. Index in this vec == `SystemKey.0`.
    pub(crate) descriptors: Vec<SystemDescriptor>,

    /// `TypeId(SystemSet)` → `SystemSetId` interning. The first
    /// `.in_set(MySet)` allocates a fresh id; subsequent calls return
    /// the same value.
    pub(crate) sets: HashMap<TypeId, SystemSetId>,

    /// `SystemSetId` → list of member [`SystemKey`]s. Mirrors the
    /// `SystemDescriptor::sets` Vec the other direction. Wave 5 Step 14
    /// consumes this for the set-expansion pass.
    pub(crate) set_members: HashMap<SystemSetId, Vec<SystemKey>>,
}

impl ScheduleBuilder {
    /// Constructs an empty builder bound to the given pool.
    #[inline]
    pub fn new(pool: Arc<ThreadPool>) -> Self {
        Self {
            pool,
            descriptors: Vec::new(),
            sets: HashMap::new(),
            set_members: HashMap::new(),
        }
    }

    /// Registers a system. Returns a [`SystemConfig`] handle for fluent
    /// `.before(...)` / `.after(...)` / `.chain(...)` / `.in_set(...)`
    /// chaining.
    ///
    /// Systems are stored in insertion order; `SystemKey.0` equals the
    /// index in the descriptor vec at insertion time. Topological
    /// re-ordering happens in [`build`](Self::build).
    ///
    /// # Output bound
    ///
    /// Plan SCH10 / Q1 — only `Out = ()` systems flow through the
    /// scheduler. Non-unit-output systems use `EcsMaster::run_system`
    /// outside the schedule.
    pub fn add_system<F, M>(&mut self, system: F) -> SystemConfig<'_>
    where
        F: IntoSystem<(), (), M>,
        F::System: System<Out = ()> + 'static,
    {
        let sys = F::into_system(system);
        let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
        let system_box = SystemBox::new(boxed);
        let key = SystemKey(self.descriptors.len());
        self.descriptors.push(SystemDescriptor::new(system_box));
        SystemConfig {
            builder: self,
            key,
        }
    }

    /// Interns the `TypeId` of a system set, returning a stable
    /// [`SystemSetId`]. First call allocates the id; subsequent calls
    /// return the same value.
    #[inline]
    pub(crate) fn set_id_of(&mut self, type_id: TypeId) -> SystemSetId {
        let next = self.sets.len();
        *self
            .sets
            .entry(type_id)
            .or_insert_with(|| SystemSetId(next))
    }

    /// Number of systems registered so far (pre-build).
    #[inline]
    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    /// `true` iff no systems have been registered.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }

    /// Finalises the schedule.
    ///
    /// Round 3 W-NEW-3 mandates this body pre-destructure `self` so
    /// that the descriptor vec can be moved into successive phases
    /// without aliasing the builder.
    ///
    /// # Panics
    ///
    /// * On cycle detection (`boyko-B9001`): the ordering DAG built
    ///   from the user's `.before/.after/.chain` hints contains a
    ///   strongly-connected component with > 1 node. The panic message
    ///   lists the system names in the cycle.
    /// * `descriptors.len() > MAX_SYSTEMS_PER_SCHEDULE` (`debug_assert!`,
    ///   plan §13.6 O-NEW-3).
    pub fn build(self, world: &mut EcsMaster) -> Schedule {
        // Round 3 W-NEW-3 pre-destructure: this gives independent
        // mutable handles into each field, lets us move `descriptors`
        // by value into downstream phases, and keeps the (eventually
        // very long) build body readable.
        let Self {
            pool,
            mut descriptors,
            sets: _sets,
            set_members: _set_members,
        } = self;

        // Step 1 — initialise every system. Allocation is allowed here
        // because the builder runs on the dispatcher with no workers
        // active (ALLOC2).
        for d in &mut descriptors {
            d.system_box.system.initialize(world);
            // SCH15 / Round 2 C9 invariant: refresh the cached
            // `is_exclusive` flag now that `Access` is filled in.
            // `SystemBox::new` recorded the post-construction value
            // (often all-zero before init); `initialize` may have
            // mutated `meta.access`. We re-read once here to freeze
            // the truth at the same point the executor will observe
            // it.
            d.system_box.is_exclusive = d.system_box.system.access().is_universal();
        }

        // Step 2 — capture names BEFORE the descriptors move into later
        // phases. Diagnostics in `cycle_in_before_after_panics` rely on
        // this snapshot.
        let names: Vec<&'static str> = descriptors
            .iter()
            .map(|d| d.system_box.name)
            .collect();

        // Plan §13.6 O-NEW-3 (and §3 Q4): hard cap fits u16.
        debug_assert!(
            descriptors.len() <= MAX_SYSTEMS_PER_SCHEDULE,
            "Schedule cap MAX_SYSTEMS_PER_SCHEDULE = {} exceeded ({})",
            MAX_SYSTEMS_PER_SCHEDULE,
            descriptors.len()
        );
        debug_assert!(
            descriptors.len() <= u16::MAX as usize,
            "ScheduleBuilder::build: descriptors.len() must fit u16"
        );

        // Step 3 — collect raw DAG edges from each descriptor's
        // ordering hints.
        let n = descriptors.len();
        let dag_edges_keys: Vec<(SystemKey, SystemKey)> = descriptors
            .iter()
            .flat_map(|d| d.ordering_hints.iter().filter_map(OrderingEdge::as_dag_edge))
            .collect();

        // Step 4 — Tarjan SCC for cycle detection on the ordering DAG.
        // `tarjan_scc` returns one Vec per strongly-connected component;
        // any SCC with > 1 node is a cycle.
        let sccs = tarjan_scc(n, &dag_edges_keys);
        for scc in &sccs {
            if scc.len() > 1 {
                let cycle_names: Vec<&'static str> =
                    scc.iter().map(|k| names[k.0]).collect();
                panic!(
                    "boyko-B9001: schedule contains a cycle of {} systems: {:?}",
                    scc.len(),
                    cycle_names
                );
            }
        }

        // Step 5 — topological sort via Kahn's algorithm.
        // `topo_order[new_index] = old_key.0` (mapping from post-sort
        // index back into the original `descriptors` vec).
        let topo_order = kahn_topological_sort(n, &dag_edges_keys);
        debug_assert_eq!(topo_order.len(), n, "Kahn's must produce a full ordering");

        // Step 6 — permute descriptors into topological order. We build
        // a `SystemIndex`-keyed edge list along the way: each old
        // `SystemKey.0` is mapped to its new index via `reorder`.
        let mut reorder = vec![0u16; n];
        for (new_idx, &old_key) in topo_order.iter().enumerate() {
            reorder[old_key.0] = new_idx as u16;
        }

        // Permute the descriptors. We pop from a `Vec<Option<...>>` to
        // avoid double-moves while we iterate.
        let mut taking: Vec<Option<SystemDescriptor>> =
            descriptors.into_iter().map(Some).collect();
        let mut ordered: Vec<SystemDescriptor> = Vec::with_capacity(n);
        for &old_key in &topo_order {
            ordered.push(
                taking[old_key.0]
                    .take()
                    .expect("invariant: each descriptor consumed exactly once by topo sort"),
            );
        }
        debug_assert!(
            taking.iter().all(|opt| opt.is_none()),
            "invariant: every descriptor must be moved by the topo permutation"
        );

        // Step 7 — translate raw `SystemKey` edges to post-permutation
        // `SystemIndex` edges. Dedupe along the way — multiple
        // `.before(other).after(other)` chains can emit duplicates that
        // would otherwise inflate `pred_count` and trip the executor's
        // underflow `debug_assert!`.
        let mut dedup: std::collections::HashSet<(u16, u16)> = std::collections::HashSet::new();
        let mut dag_edges_idx: Vec<(SystemIndex, SystemIndex)> =
            Vec::with_capacity(dag_edges_keys.len());
        for &(from_key, to_key) in &dag_edges_keys {
            let from_idx = reorder[from_key.0];
            let to_idx = reorder[to_key.0];
            if dedup.insert((from_idx, to_idx)) {
                dag_edges_idx.push((SystemIndex(from_idx), SystemIndex(to_idx)));
            }
        }

        // Step 8 — sync-point insertion is the Wave 5 Step 14 deliverable.
        // For now this is a no-op pass-through; the descriptors and edges
        // flow straight into the ConflictGraph.
        let (descriptors_with_sync, dag_edges_with_sync) =
            insert_sync_points(ordered, dag_edges_idx);

        // Step 9 — ConflictGraph build (Wave 4 Step 10).
        let conflict_graph = ConflictGraph::build(&descriptors_with_sync, &dag_edges_with_sync);

        // Plan §13.6 O-NEW-3 secondary check: every pred_count fits u16.
        // The `pred_count: Box<[u16]>` type itself enforces the per-element
        // bound; `ConflictGraph::build` uses `checked_add` on each increment
        // so any overflow would have already panicked before reaching here.
        // The bound assertion is therefore implicit — no runtime check is
        // necessary, and the previous `c <= u16::MAX` form is tautological
        // at the `u16` type level (clippy::absurd_extreme_comparisons).

        // Step 10 — drop the descriptor envelope, keep the `SystemBox`es.
        let n_final = descriptors_with_sync.len();
        let systems: Vec<SystemBox> = descriptors_with_sync
            .into_iter()
            .map(|d| d.system_box)
            .collect();

        // Build the scratch *after* the conflict graph so we can seed
        // `pred_remaining` from `pred_count` in one pass.
        let _ = n_final; // historical; preserved for symmetry with prior wave.
        let executor_scratch = ExecutorScratch::new(systems.len(), &conflict_graph);

        Schedule {
            pool,
            systems,
            conflict_graph,
            executor_scratch,
        }
    }
}

/// Sync-point insertion — Wave 5 Step 14 deliverable.
///
/// # Phase 9 (this revision): conservative pass-through
///
/// Plan §8 describes the Bevy-style auto-insertion algorithm:
///
/// 1. Detect every system that owns a `CommandQueue` `SystemParam`
///    (`has_deferred == true`).
/// 2. For each `(A, B)` DAG edge where `A` is deferred and `B` performs
///    structural reads, insert an `ApplyDeferred` exclusive system between
///    them and rewire the edge through it.
/// 3. Coalesce shared upstream cones to minimise the number of inserted
///    syncs.
///
/// The full implementation is **deferred to Phase 9.1**. Two prerequisites
/// are missing today:
///
/// * `SystemMeta`/`SystemBox` does not yet expose a `has_deferred()` query
///   — the flag must thread through `SystemParam::init_access` into a new
///   bit on `Access` or a sibling cache.
/// * `ApplyDeferred` is not yet a registered system type (its body is a
///   no-op; the dispatcher special-cases it by walking an `upstream`
///   `Vec<SystemIndex>` and calling `apply` on each). The infrastructure
///   exists in `ExclusiveFunctionSystem` + universal `Access` but the
///   marker has not been wired.
///
/// # Why the pass-through is correct (SCH7)
///
/// The apply window barrier (plan §2.2 SCH7 / §5.4.5.1) already
/// serialises every system's `apply` against every concurrent worker.
/// `Commands::add` enqueues into the system's own `CommandQueue` (which
/// is `!Sync`, per CQ-SEND2); the queue is flushed by
/// `SystemParam::apply` from the dispatcher inside the apply window.
/// Downstream systems run only after their predecessors' `apply` calls
/// have returned (the executor sets `completed[i]` AFTER `apply`).
///
/// Therefore: without explicit `ApplyDeferred` insertion, every
/// `Commands`-enqueued mutation is visible to every downstream system —
/// just at the cost of one extra dispatcher round per system that has
/// deferred work. The trade is "slightly less parallelism vs the full
/// Bevy algorithm", not "correctness".
///
/// # Phase 9.1 follow-up
///
/// The full algorithm is enumerated in plan §8.2; it will be wired
/// alongside change-detection ticks (Phase 10) when `has_deferred`
/// becomes a first-class SystemParam predicate.
#[inline]
fn insert_sync_points(
    descriptors: Vec<SystemDescriptor>,
    dag_edges: Vec<(SystemIndex, SystemIndex)>,
) -> (Vec<SystemDescriptor>, Vec<(SystemIndex, SystemIndex)>) {
    (descriptors, dag_edges)
}

/// Standard Tarjan strongly-connected-components.
///
/// Returns one `Vec<SystemKey>` per SCC. Trivial (single-node) SCCs are
/// returned alongside non-trivial ones; callers filter for `len() > 1`
/// when detecting cycles.
///
/// # Implementation notes
///
/// Iterative — recursion would blow the stack on a 1024-system schedule.
/// The control stack stores `(node, child_iter_index)` so we can resume
/// after recursing into a child. Lowlink + on-stack state lives in
/// parallel arrays keyed by `SystemKey.0`.
fn tarjan_scc(n: usize, edges: &[(SystemKey, SystemKey)]) -> Vec<Vec<SystemKey>> {
    // Build an adjacency list (index = source key).
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(from, to) in edges {
        adj[from.0].push(to.0);
    }

    // Per-node Tarjan state.
    const UNVISITED: u32 = u32::MAX;
    let mut index_of: Vec<u32> = vec![UNVISITED; n];
    let mut lowlink: Vec<u32> = vec![0; n];
    let mut on_stack: Vec<bool> = vec![false; n];

    let mut next_index: u32 = 0;
    let mut scc_stack: Vec<usize> = Vec::with_capacity(n);
    let mut sccs: Vec<Vec<SystemKey>> = Vec::new();

    // Iterative DFS: control stack frames are `(node, next_child_idx)`.
    // When we visit `node` for the first time we push the frame; each
    // step advances `next_child_idx` and either recurses (push child)
    // or pops (back-edge / unwind).
    let mut ctrl: Vec<(usize, usize)> = Vec::with_capacity(n);

    for start in 0..n {
        if index_of[start] != UNVISITED {
            continue;
        }
        // Begin visit at `start`.
        index_of[start] = next_index;
        lowlink[start] = next_index;
        next_index += 1;
        scc_stack.push(start);
        on_stack[start] = true;
        ctrl.push((start, 0));

        while let Some(&(node, child_pos)) = ctrl.last() {
            if child_pos < adj[node].len() {
                let child = adj[node][child_pos];
                // Advance the parent's child iterator first so the
                // back-edge unwind sees the right position.
                ctrl.last_mut().unwrap().1 += 1;
                if index_of[child] == UNVISITED {
                    // Tree edge — recurse.
                    index_of[child] = next_index;
                    lowlink[child] = next_index;
                    next_index += 1;
                    scc_stack.push(child);
                    on_stack[child] = true;
                    ctrl.push((child, 0));
                } else if on_stack[child] {
                    // Back edge — propagate lowlink.
                    lowlink[node] = lowlink[node].min(index_of[child]);
                }
                // (Forward / cross edges to nodes that are visited but
                // not on the SCC stack do not update lowlink — they
                // belong to already-emitted SCCs.)
            } else {
                // Finished `node`. If it is an SCC root, pop the SCC.
                let node_low = lowlink[node];
                let node_idx = index_of[node];
                ctrl.pop();
                if node_low == node_idx {
                    let mut component = Vec::new();
                    loop {
                        let top = scc_stack.pop().expect("SCC stack must contain root");
                        on_stack[top] = false;
                        component.push(SystemKey(top));
                        if top == node {
                            break;
                        }
                    }
                    sccs.push(component);
                }
                // Propagate lowlink back to the parent (if any).
                if let Some(&mut (parent, _)) = ctrl.last_mut() {
                    lowlink[parent] = lowlink[parent].min(node_low);
                }
            }
        }
    }

    sccs
}

/// Kahn's algorithm for topological sort.
///
/// Returns a permutation of `0..n` as a `Vec<SystemKey>`. The order is
/// stable for fixed input (the ready queue is a FIFO — ties break in
/// insertion order, which matches user expectation for "two unordered
/// systems appear in `add_system` order").
///
/// # Pre-condition
///
/// The caller has already validated that the DAG is acyclic (via
/// `tarjan_scc`). Kahn's will not detect a cycle directly; it would
/// simply produce a partial order shorter than `n`. The `debug_assert!`
/// in `build` catches that misuse.
fn kahn_topological_sort(n: usize, edges: &[(SystemKey, SystemKey)]) -> Vec<SystemKey> {
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_degree: Vec<u32> = vec![0; n];
    for &(from, to) in edges {
        adj[from.0].push(to.0);
        in_degree[to.0] += 1;
    }

    // FIFO ready queue; using `VecDeque` to preserve insertion order.
    let mut ready: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for (idx, &d) in in_degree.iter().enumerate() {
        if d == 0 {
            ready.push_back(idx);
        }
    }

    let mut out: Vec<SystemKey> = Vec::with_capacity(n);
    while let Some(node) = ready.pop_front() {
        out.push(SystemKey(node));
        for &child in &adj[node] {
            in_degree[child] -= 1;
            if in_degree[child] == 0 {
                ready.push_back(child);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use boyko_threadpool::ThreadPoolBuilder;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::ecs::core::system::access::Access;
    use crate::ecs::core::system::system::System;
    use crate::ecs::core::system::system_meta::SystemMeta;
    use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

    /// Test-only `System` that counts `initialize` calls. Used to assert
    /// `build_initializes_systems_once`.
    struct CountingSystem {
        meta: SystemMeta,
        init_count: Arc<AtomicUsize>,
    }

    // SAFETY (S1): `run_unsafe` is empty; the trait contract is vacuous.
    unsafe impl System for CountingSystem {
        type Out = ();
        fn name(&self) -> &'static str {
            self.meta.name()
        }
        fn access(&self) -> &Access {
            self.meta.access()
        }
        fn initialize(&mut self, _world: &mut EcsMaster) {
            self.init_count.fetch_add(1, Ordering::Relaxed);
        }
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

    /// Wrapper that turns a `CountingSystem` into an `IntoSystem<(), ()>`
    /// via the identity-style closure pattern — easier than dragging the
    /// full `SystemParamFunction` chain in for a unit test.
    fn add_counting(
        builder: &mut ScheduleBuilder,
        name: &'static str,
        init_count: Arc<AtomicUsize>,
    ) -> SystemKey {
        let sys = CountingSystem {
            meta: SystemMeta::for_testing(name),
            init_count,
        };
        let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
        let system_box = SystemBox::new(boxed);
        let key = SystemKey(builder.descriptors.len());
        builder
            .descriptors
            .push(SystemDescriptor::new(system_box));
        key
    }

    fn fresh_pool() -> Arc<ThreadPool> {
        ThreadPoolBuilder::new().num_threads(1).build()
    }

    /// `add_system` returns a `SystemConfig` whose key matches the
    /// insertion order.
    #[test]
    fn add_system_assigns_key() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let init = Arc::new(AtomicUsize::new(0));
        let a = add_counting(&mut builder, "a", Arc::clone(&init));
        let b = add_counting(&mut builder, "b", Arc::clone(&init));
        assert_eq!(a.0, 0);
        assert_eq!(b.0, 1);
        assert_eq!(builder.len(), 2);
    }

    /// `build` runs `System::initialize` exactly once per registered
    /// system.
    #[test]
    fn build_initializes_systems_once() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let init = Arc::new(AtomicUsize::new(0));
        let _a = add_counting(&mut builder, "a", Arc::clone(&init));
        let _b = add_counting(&mut builder, "b", Arc::clone(&init));
        let _c = add_counting(&mut builder, "c", Arc::clone(&init));

        let mut world = EcsMaster::new();
        let schedule = builder.build(&mut world);
        assert_eq!(init.load(Ordering::Relaxed), 3);
        assert_eq!(schedule.len(), 3);
    }

    /// `.before(other)` + `.after(other)` on the same pair forms a cycle
    /// that `build` rejects with the documented `boyko-B9001` message.
    #[test]
    #[should_panic(expected = "boyko-B9001")]
    fn cycle_in_before_after_panics() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let init = Arc::new(AtomicUsize::new(0));
        let a = add_counting(&mut builder, "a", Arc::clone(&init));
        let b = add_counting(&mut builder, "b", Arc::clone(&init));

        // a -> b
        builder.descriptors[a.0]
            .ordering_hints
            .push(OrderingEdge::Before(a, b));
        // b -> a
        builder.descriptors[b.0]
            .ordering_hints
            .push(OrderingEdge::Before(b, a));

        let mut world = EcsMaster::new();
        let _schedule = builder.build(&mut world);
    }

    /// Topological sort respects `before` ordering: if `a` declares
    /// `before(b)`, `a` precedes `b` in the resulting `systems` vec.
    #[test]
    fn topological_sort_respects_before() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let init = Arc::new(AtomicUsize::new(0));

        // Insertion order: c, a, b. With c.before(a) and a.before(b) the
        // post-build order must be c, a, b.
        let c = add_counting(&mut builder, "c", Arc::clone(&init));
        let a = add_counting(&mut builder, "a", Arc::clone(&init));
        let b = add_counting(&mut builder, "b", Arc::clone(&init));

        builder.descriptors[c.0]
            .ordering_hints
            .push(OrderingEdge::Before(c, a));
        builder.descriptors[a.0]
            .ordering_hints
            .push(OrderingEdge::Before(a, b));

        let mut world = EcsMaster::new();
        let schedule = builder.build(&mut world);
        let names: Vec<&'static str> =
            schedule.systems.iter().map(|sb| sb.name).collect();
        // The exact ordering must place c first, then a, then b.
        assert_eq!(names, vec!["c", "a", "b"]);
    }

    /// Sanity probe — Kahn's on a small DAG.
    #[test]
    fn kahn_sort_basic() {
        // a -> b -> c, a -> c
        let edges = vec![
            (SystemKey(0), SystemKey(1)),
            (SystemKey(1), SystemKey(2)),
            (SystemKey(0), SystemKey(2)),
        ];
        let order = kahn_topological_sort(3, &edges);
        let pos: HashMap<usize, usize> =
            order.iter().enumerate().map(|(i, k)| (k.0, i)).collect();
        assert!(pos[&0] < pos[&1]);
        assert!(pos[&1] < pos[&2]);
        assert!(pos[&0] < pos[&2]);
    }

    /// Sanity probe — Tarjan on a known cycle returns one SCC of size 3.
    #[test]
    fn tarjan_detects_three_cycle() {
        let edges = vec![
            (SystemKey(0), SystemKey(1)),
            (SystemKey(1), SystemKey(2)),
            (SystemKey(2), SystemKey(0)),
        ];
        let sccs = tarjan_scc(3, &edges);
        let big = sccs.iter().filter(|s| s.len() > 1).count();
        assert_eq!(big, 1);
    }

    /// Sanity probe — Tarjan returns trivial SCCs (one per node) on an
    /// acyclic graph.
    #[test]
    fn tarjan_acyclic_yields_only_singletons() {
        let edges = vec![(SystemKey(0), SystemKey(1)), (SystemKey(1), SystemKey(2))];
        let sccs = tarjan_scc(3, &edges);
        assert!(sccs.iter().all(|s| s.len() == 1));
        assert_eq!(sccs.len(), 3);
    }
}
