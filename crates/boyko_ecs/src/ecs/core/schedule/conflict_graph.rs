//! [`ConflictGraph`] — per-system conflict bitsets + DAG predecessor
//! counts consumed by the executor (Wave 5 Step 12).
//!
//! See Phase 9 plan §5.3 + §7.1 + §7.4. Wave 4 Step 10 builds the
//! immutable graph during [`ScheduleBuilder::build`]; the executor reads
//! it on every dispatch round without further mutation.
//!
//! # What the bitsets encode
//!
//! `conflict_bits[i]` is a `FixedBitSet` of length `n` (one bit per
//! system). Bit `j` is set iff systems `i` and `j` must serialise — that
//! is, either:
//!
//! * Their declared `Access` surfaces conflict
//!   ([`Access::conflicts_with`]); or
//! * The user added an ordering hint between them
//!   (`.before` / `.after` / `.chain`). Ordered systems share a conflict
//!   bit because the downstream cannot run alongside the upstream
//!   anyway — bundling the predicate into one bitset lets the executor
//!   answer "can I dispatch sys `i` now?" with a single SIMD scan
//!   against the `running` bitset.
//!
//! # `pred_count` covers ordering edges only
//!
//! `pred_count[i]` is the **in-degree of system `i` in the ordering DAG**
//! (not the conflict graph). Conflict edges are commutative — they say
//! "these two cannot run together" — but they impose no order. The
//! executor uses `pred_remaining` (Wave 5 Step 11) to track when a
//! system's *ordered* predecessors have completed; conflict bits gate
//! only the *concurrent* dispatch decision against the `running` set.
//!
//! [`ScheduleBuilder::build`]: super::schedule_builder::ScheduleBuilder::build
//! [`Access::conflicts_with`]: crate::ecs::core::system::access::Access::conflicts_with

use fixedbitset::FixedBitSet;

use crate::ecs::core::schedule::system_descriptor::SystemDescriptor;

/// Stable index assigned to each system after topological sorting.
///
/// `u16` is wide enough for `MAX_SYSTEMS_PER_SCHEDULE = 1024` with three
/// orders of magnitude of headroom; the executor's `pred_remaining: Box<[u16]>`
/// (plan §7.4.1, Round 3 O-NEW-2) shares the type so an in-degree count
/// cannot overflow without first overflowing `SystemIndex` itself.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub(crate) struct SystemIndex(pub u16);

/// Immutable per-frame execution prerequisites.
///
/// Built once by [`ConflictGraph::build`]; consumed read-only by the
/// executor (Wave 5 Step 12). Field order is `pred_count → successors →
/// conflict_bits` — `pred_count` is hottest (the executor consults it
/// at frame start to seed the ready queue), `successors` walked once per
/// completion, `conflict_bits` scanned via SIMD `bitset_intersects` on
/// every dispatch attempt.
///
/// # Dead-code allowance
///
/// `successors` and `conflict_bits` are written by `build` (Wave 4 Step
/// 10) but read only by the Wave 5 Step 12 executor. The lint is
/// silenced here rather than crate-wide so the wiring intent stays
/// visible at this checkpoint.
#[allow(dead_code)]
pub(crate) struct ConflictGraph {
    /// In-degree of each system in the ordering DAG. Used to seed
    /// `ExecutorScratch::pred_remaining` at frame start (plan §7.4.2).
    pub(crate) pred_count: Box<[u16]>,

    /// Outgoing DAG edges per system. When system `i` completes the
    /// executor decrements `pred_remaining[s]` for each `s` in
    /// `successors[i]`.
    pub(crate) successors: Box<[Box<[SystemIndex]>]>,

    /// `conflict_bits[i]` has bit `j` set iff `i` and `j` cannot run
    /// concurrently (access conflict OR ordering edge between them).
    pub(crate) conflict_bits: Box<[FixedBitSet]>,
}

impl ConflictGraph {
    /// Builds the conflict graph from `descriptors` plus a flat list of
    /// directed ordering edges `(from → to)`. `dag_edges` is expected to
    /// already be deduped — duplicate entries inflate `pred_count` and
    /// trigger the underflow `debug_assert!` in the executor.
    ///
    /// # Panics in debug
    ///
    /// `debug_assert!`s that every edge endpoint is in range
    /// `0..descriptors.len()`. Out-of-range edges indicate a builder
    /// bug; release builds elide the check.
    ///
    /// # Complexity
    ///
    /// O(N² / w) for the pairwise access scan, where `w` is the
    /// bitset block width. Acceptable as a one-shot build cost; see plan
    /// §7.1 for the breakdown.
    pub(crate) fn build(
        descriptors: &[SystemDescriptor],
        dag_edges: &[(SystemIndex, SystemIndex)],
    ) -> Self {
        let n = descriptors.len();

        // Allocate the conflict bitsets up-front; each is `n` bits wide.
        let mut conflict_bits: Vec<FixedBitSet> = (0..n)
            .map(|_| FixedBitSet::with_capacity(n))
            .collect();

        // Pairwise access conflict — symmetric, half-triangle iteration.
        for i in 0..n {
            let access_i = descriptors[i].system_box.system.access();
            for j in 0..i {
                let access_j = descriptors[j].system_box.system.access();
                if access_i.conflicts_with(access_j) {
                    conflict_bits[i].insert(j);
                    conflict_bits[j].insert(i);
                }
            }
        }

        // Per-system successor lists + in-degree counters. Both are
        // populated from `dag_edges`.
        let mut successors_buf: Vec<Vec<SystemIndex>> = vec![Vec::new(); n];
        let mut pred_count_buf: Vec<u16> = vec![0u16; n];

        for &(from, to) in dag_edges {
            let from_idx = from.0 as usize;
            let to_idx = to.0 as usize;
            debug_assert!(
                from_idx < n,
                "ConflictGraph::build: dag edge `from` out of range ({} >= {})",
                from_idx,
                n,
            );
            debug_assert!(
                to_idx < n,
                "ConflictGraph::build: dag edge `to` out of range ({} >= {})",
                to_idx,
                n,
            );

            successors_buf[from_idx].push(to);
            pred_count_buf[to_idx] = pred_count_buf[to_idx]
                .checked_add(1)
                .expect("invariant: pred_count must fit u16 (MAX_SYSTEMS_PER_SCHEDULE bound)");

            // Ordered systems also share a conflict bit — the downstream
            // cannot run alongside the upstream regardless of access.
            conflict_bits[from_idx].insert(to_idx);
            conflict_bits[to_idx].insert(from_idx);
        }

        // Post-condition (plan §13.6): conflict bits are symmetric. Cheap
        // assertion — one bit-flip equality per pair we touched.
        #[cfg(debug_assertions)]
        for i in 0..n {
            for j in 0..i {
                debug_assert_eq!(
                    conflict_bits[i].contains(j),
                    conflict_bits[j].contains(i),
                    "ConflictGraph::build: asymmetric conflict bit at ({i}, {j})"
                );
            }
        }

        let successors: Box<[Box<[SystemIndex]>]> = successors_buf
            .into_iter()
            .map(Vec::into_boxed_slice)
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Self {
            pred_count: pred_count_buf.into_boxed_slice(),
            successors,
            conflict_bits: conflict_bits.into_boxed_slice(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
    use crate::ecs::core::schedule::system_box::SystemBox;
    use crate::ecs::core::system::access::Access;
    use crate::ecs::core::system::system::System;
    use crate::ecs::core::system::system_meta::SystemMeta;
    use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
    use crate::ecs::identifiers::primitives::{ComponentId, ResourceId};

    /// Test-only `System` impl whose `Access` can be driven by tests
    /// without dragging in the full `SystemParam` pipeline.
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

    fn descriptor_with_access(name: &'static str, access: Access) -> SystemDescriptor {
        let mut meta = SystemMeta::for_testing(name);
        meta.access = access;
        let sys = ProbeSystem { meta };
        let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
        SystemDescriptor::new(SystemBox::new(boxed))
    }

    /// Two systems that both write the same resource share a conflict
    /// bit in each other's bitset.
    #[test]
    fn two_conflicting_systems_have_conflict_bit() {
        let mut access_a = Access::new();
        access_a.add_resource_write(ResourceId(0));
        let mut access_b = Access::new();
        access_b.add_resource_write(ResourceId(0));

        let descs = vec![
            descriptor_with_access("a", access_a),
            descriptor_with_access("b", access_b),
        ];

        let graph = ConflictGraph::build(&descs, &[]);
        assert!(graph.conflict_bits[0].contains(1), "0 must list 1 as conflict");
        assert!(graph.conflict_bits[1].contains(0), "1 must list 0 as conflict");
        assert_eq!(graph.pred_count[0], 0, "no ordering edges means pred_count 0");
        assert_eq!(graph.pred_count[1], 0);
    }

    /// Two disjoint-access systems have no conflict bit set.
    #[test]
    fn disjoint_access_systems_no_conflict_bit() {
        let mut access_a = Access::new();
        access_a.add_resource_read(ResourceId(0));
        let mut access_b = Access::new();
        access_b.add_component_write(ComponentId(5));

        let descs = vec![
            descriptor_with_access("a", access_a),
            descriptor_with_access("b", access_b),
        ];

        let graph = ConflictGraph::build(&descs, &[]);
        assert!(!graph.conflict_bits[0].contains(1));
        assert!(!graph.conflict_bits[1].contains(0));
    }

    /// `pred_count[i] == 0` for every system in a graph with no ordering
    /// edges (independent of conflict status — conflicts don't count as
    /// predecessors).
    #[test]
    fn pred_count_zero_for_no_deps() {
        let mut conflict_access = Access::new();
        conflict_access.add_resource_write(ResourceId(7));
        let descs = vec![
            descriptor_with_access("a", conflict_access),
            descriptor_with_access("b", {
                let mut a = Access::new();
                a.add_resource_write(ResourceId(7));
                a
            }),
            descriptor_with_access("c", Access::new()),
        ];

        let graph = ConflictGraph::build(&descs, &[]);
        for &pc in graph.pred_count.iter() {
            assert_eq!(pc, 0, "no DAG edges -> every pred_count must be 0");
        }
    }

    /// `pred_count[i]` matches the in-degree from `dag_edges`.
    #[test]
    fn pred_count_matches_in_degree() {
        let descs = vec![
            descriptor_with_access("a", Access::new()),
            descriptor_with_access("b", Access::new()),
            descriptor_with_access("c", Access::new()),
            descriptor_with_access("d", Access::new()),
        ];

        // Graph: a -> b, a -> c, b -> c, c -> d
        let edges = vec![
            (SystemIndex(0), SystemIndex(1)),
            (SystemIndex(0), SystemIndex(2)),
            (SystemIndex(1), SystemIndex(2)),
            (SystemIndex(2), SystemIndex(3)),
        ];

        let graph = ConflictGraph::build(&descs, &edges);
        assert_eq!(graph.pred_count[0], 0, "a has no predecessors");
        assert_eq!(graph.pred_count[1], 1, "b has 1 predecessor (a)");
        assert_eq!(graph.pred_count[2], 2, "c has 2 predecessors (a, b)");
        assert_eq!(graph.pred_count[3], 1, "d has 1 predecessor (c)");

        // Ordering edges also set conflict bits.
        assert!(graph.conflict_bits[0].contains(1));
        assert!(graph.conflict_bits[1].contains(0));
        assert!(graph.conflict_bits[2].contains(3));
        assert!(graph.conflict_bits[3].contains(2));

        // Successor lists mirror the edge list.
        assert_eq!(graph.successors[0].len(), 2);
        assert_eq!(graph.successors[2].as_ref(), &[SystemIndex(3)][..]);
    }
}
