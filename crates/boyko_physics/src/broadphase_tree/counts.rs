//! The query-cost investigation's counters (commit C3b of the tree broadphase; the investigation
//! is section 8 of `docs/measurements/2026-09-22-broadphase-tree/analysis.md`): what one query
//! of the packed 8-wide BVH does, and the shape of the tree it walks.
//!
//! Compiled only under the non-default `bp-query-counts` feature, which nothing in the workspace
//! enables. Without it this module does not exist and neither does any counting statement in
//! `bvh.rs`, so the default build's query is the uncounted kernel (the C3b receipt compares the
//! default build's code before and after the feature was added).
//!
//! # What is counted
//!
//! * Per query of one tree ([`QueryCounts`]), by `PackedBvh8::query` itself: the 8-wide box
//!   tests (internal nodes visited), the 8-wide exact tests (leaf nodes visited), the lanes the
//!   box tests passed (the children descended into), the live lanes of the visited leaves (the
//!   leaf candidates), how many of those a per-lane box test would keep, and the lanes the exact
//!   test accepted. The kernel has no per-lane box test — its leaf test is the exact
//!   sphere-bound test on eight lanes at once — so `leaf_box_hits` prices what such a test would
//!   save, not work the kernel does.
//! * Per queried row ([`RowQueryCounts`]): its walk of the active and of the static tree, and
//!   the partners each walk put into the row's segment — the pairs the query pass emits.
//! * Per tree ([`TreeShape`]): the levels, the leaf occupancy, the internal nodes' child counts,
//!   and per level the children's boxes against the box of the node that holds them, by surface
//!   area and by volume.
//! * Per leaf-list pass ([`LeafListCounts`], the C3b fix F1): the collection walks' box tests,
//!   the candidates collected, the prefilter chunks, the exact tests kept and the partners
//!   emitted — counted by the pass itself, in this feature's build and the crate's test build.
//!   [`BroadphaseTree::leaf_list_counts`] reads the last pass's; [`BroadphaseTree::query_stage`]
//!   re-runs a step's query stage under either kernel, so a driver can compare the two kernels'
//!   bytes (G-LL1) on scenes the crate's own tests do not hold.
//!
//! # How a pass is counted
//!
//! [`BroadphaseTree::count_query_pass`] replays the query pass of the last tree-path step: the
//! same rows in the same order, against the same trees, through the same kernel, with the
//! same filter. It checks every row's emitted partner count against the segment that step
//! wrote, so a replay that walked anything other than what the step walked fails there. The
//! replay exists because the step has no per-row storage to hand out: a `Vec` or a new scratch
//! column in the step would be a heap site or a column id this feature does not own, while a
//! query is a pure function of the tree and the query row.
//!
//! The sleeper tree of L10 C3c is walked for that check and not reported: without the
//! sleep-skip it is empty, and every count here is C3b's. With the sleep-skip, a step whose
//! query was followed by a change to the sleeper set — L10's release (T4) after the query, or a
//! brute or other-kind step that dissolved the set (T6) — cannot be replayed: the replay misses
//! the partners the step found in the killed lanes, and the check above panics.

use core::sync::atomic::{AtomicU32, Ordering};

use super::bvh::{
    LANES, LEAF_R, LEAF_ROW, LEAF_X, LEAF_Y, LEAF_Z, MAX_LEVELS, MAX_X, MAX_Y, MAX_Z, MIN_X, MIN_Y,
    MIN_Z, NO_LANE_ROW, Node8,
};
use super::kernel::{QueryBox, padded_radius};
use super::{BroadphaseTree, KIND_WIDE, QueryKernel};

pub use super::LeafListCounts;

/// The most levels a [`TreeShape`] describes (`8^13 > 2^24` rows).
pub const TREE_MAX_LEVELS: usize = MAX_LEVELS;

/// One query's walk of one tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryCounts {
    /// 8-wide box tests: the internal nodes visited.
    pub box_tests: u32,
    /// 8-wide exact tests: the leaf nodes visited.
    pub leaf_tests: u32,
    /// Internal lanes whose box met the query box: the children descended into.
    pub descents: u32,
    /// Live lanes of the visited leaves: the rows the exact test decided.
    pub leaf_candidates: u32,
    /// Leaf candidates whose own padded box meets the query box.
    pub leaf_box_hits: u32,
    /// Lanes the exact test accepted: the calls of the query's `accept`, the query row itself
    /// and the partners the caller's filter drops included.
    pub exact_hits: u32,
}

impl QueryCounts {
    /// 8-wide tests of either kind.
    #[inline]
    pub const fn tests(&self) -> u32 {
        self.box_tests + self.leaf_tests
    }

    /// Field-by-field sum.
    #[inline]
    pub const fn plus(&self, other: &Self) -> Self {
        Self {
            box_tests: self.box_tests + other.box_tests,
            leaf_tests: self.leaf_tests + other.leaf_tests,
            descents: self.descents + other.descents,
            leaf_candidates: self.leaf_candidates + other.leaf_candidates,
            leaf_box_hits: self.leaf_box_hits + other.leaf_box_hits,
            exact_hits: self.exact_hits + other.exact_hits,
        }
    }

    /// An internal node's box test with result `mask`.
    #[inline]
    pub(crate) fn note_internal(&mut self, mask: u32) {
        self.box_tests += 1;
        self.descents += mask.count_ones();
    }

    /// A leaf node's exact test against the query box `q` with result `mask`.
    #[inline]
    pub(crate) fn note_leaf(&mut self, node: &Node8, q: &QueryBox, mask: u32) {
        self.leaf_tests += 1;
        self.exact_hits += mask.count_ones();
        for k in 0..LANES {
            if let Some((lo, hi)) = leaf_lane_box(node, k) {
                self.leaf_candidates += 1;
                let meets = q.lo[0] <= hi[0]
                    && q.hi[0] >= lo[0]
                    && q.lo[1] <= hi[1]
                    && q.hi[1] >= lo[1]
                    && q.lo[2] <= hi[2]
                    && q.hi[2] >= lo[2];
                self.leaf_box_hits += u32::from(meets);
            }
        }
    }
}

/// One queried row of a pass: its walks of the active and of the static tree, and the partners
/// each walk put into the row's segment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowQueryCounts {
    /// The queried row.
    pub row: u32,
    /// The walk of the active tree (the Q rows).
    pub active: QueryCounts,
    /// The walk of the static tree (the members of S).
    pub statics: QueryCounts,
    /// Active-tree partners kept by the `partner > row` rule.
    pub emitted_active: u32,
    /// Static-tree partners (all kept).
    pub emitted_statics: u32,
}

impl RowQueryCounts {
    /// Both walks, summed.
    #[inline]
    pub const fn both(&self) -> QueryCounts {
        self.active.plus(&self.statics)
    }

    /// The row's segment length: the pairs the query pass emitted for it.
    #[inline]
    pub const fn emitted(&self) -> u32 {
        self.emitted_active + self.emitted_statics
    }
}

/// What a replayed pass covered, and the rest of the step's pair set beside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryPassTotals {
    /// Q rows replayed (the active tree's leaves).
    pub rows: u32,
    /// Partners the Q rows emitted.
    pub emitted: u64,
    /// Wide rows: an exact O(N) loop, not a tree query, so not replayed.
    pub wide_rows: u32,
    /// Partners the Wide rows emitted, read from the step's records.
    pub wide_emitted: u64,
    /// Entries of the static pair list, which the assembly places without a query.
    pub static_pairs: u64,
}

impl QueryPassTotals {
    /// The step's stream length as the three sources add up to it (the pairs L10's sleeper set
    /// withholds, T3, are not in the stream).
    #[inline]
    pub const fn pairs(&self) -> u64 {
        self.emitted + self.wide_emitted + self.static_pairs
    }
}

/// One level of a tree: its nodes against the children each holds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LevelShape {
    /// Nodes of the level.
    pub nodes: u32,
    /// Their children: the live lanes of a leaf node, the non-empty lanes of an internal node.
    pub children: u32,
    /// Sum over the level's nodes of their children's box surface areas. A leaf node's children
    /// are its live rows' padded boxes, the boxes its parent lane is the union of.
    pub child_area: f64,
    /// Sum over the level's nodes of the surface area of the union of their children's boxes.
    pub node_area: f64,
    /// Mean over the nodes with a child of `child area / node area`.
    pub area_ratio_mean: f64,
    /// Largest `child area / node area` of the level.
    pub area_ratio_max: f64,
    /// Sum over the level's nodes of their children's box volumes.
    pub child_volume: f64,
    /// Sum over the level's nodes of the volume of the union of their children's boxes.
    pub node_volume: f64,
}

impl LevelShape {
    /// The level's overlap figure: summed children's surface area over summed node surface
    /// area.
    #[inline]
    pub fn area_ratio(&self) -> f64 {
        if self.node_area > 0.0 { self.child_area / self.node_area } else { 0.0 }
    }

    /// Summed children's volume over summed node volume. Children lie inside the node's box, so
    /// the ratio exceeds 1 only if some children overlap.
    #[inline]
    pub fn volume_ratio(&self) -> f64 {
        if self.node_volume > 0.0 { self.child_volume / self.node_volume } else { 0.0 }
    }
}

/// One tree's shape after its last build (and any kills since).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeShape {
    /// Levels; `0` for an empty tree.
    pub levels: u32,
    /// Leaf slots of the last build, live and killed.
    pub leaf_slots: u32,
    /// Live lanes.
    pub live: u32,
    /// `leaf_occupancy[k]`: leaf nodes with `k` live lanes.
    pub leaf_occupancy: [u32; LANES + 1],
    /// `child_counts[k]`: internal nodes with `k` non-empty lanes.
    pub child_counts: [u32; LANES + 1],
    /// `level[L]`, `L < levels`: level `L`'s nodes against their children (level 0 is the
    /// leaves, over their rows).
    pub level: [LevelShape; TREE_MAX_LEVELS],
}

/// The last query's counts, published by `PackedBvh8::query` and read back by its caller.
///
/// Atomic only because the tree lives in a `Resource`, which must be `Sync`. The thread that
/// runs a query is the thread that reads its counts back, after the query returns, so program
/// order already orders the load after the store and `Relaxed` is exact: nothing else is
/// published through these.
#[derive(Debug)]
pub(crate) struct QueryProbe {
    box_tests: AtomicU32,
    leaf_tests: AtomicU32,
    descents: AtomicU32,
    leaf_candidates: AtomicU32,
    leaf_box_hits: AtomicU32,
    exact_hits: AtomicU32,
}

impl QueryProbe {
    /// A probe holding zero counts.
    pub(crate) const fn new() -> Self {
        Self {
            box_tests: AtomicU32::new(0),
            leaf_tests: AtomicU32::new(0),
            descents: AtomicU32::new(0),
            leaf_candidates: AtomicU32::new(0),
            leaf_box_hits: AtomicU32::new(0),
            exact_hits: AtomicU32::new(0),
        }
    }

    /// Records `c` as the last query's counts.
    #[inline]
    pub(crate) fn publish(&self, c: QueryCounts) {
        self.box_tests.store(c.box_tests, Ordering::Relaxed);
        self.leaf_tests.store(c.leaf_tests, Ordering::Relaxed);
        self.descents.store(c.descents, Ordering::Relaxed);
        self.leaf_candidates.store(c.leaf_candidates, Ordering::Relaxed);
        self.leaf_box_hits.store(c.leaf_box_hits, Ordering::Relaxed);
        self.exact_hits.store(c.exact_hits, Ordering::Relaxed);
    }

    /// The last query's counts.
    #[inline]
    pub(crate) fn last(&self) -> QueryCounts {
        QueryCounts {
            box_tests: self.box_tests.load(Ordering::Relaxed),
            leaf_tests: self.leaf_tests.load(Ordering::Relaxed),
            descents: self.descents.load(Ordering::Relaxed),
            leaf_candidates: self.leaf_candidates.load(Ordering::Relaxed),
            leaf_box_hits: self.leaf_box_hits.load(Ordering::Relaxed),
            exact_hits: self.exact_hits.load(Ordering::Relaxed),
        }
    }
}

impl BroadphaseTree {
    /// Replays the query pass of the last tree-path step and calls `sink` once per queried row,
    /// in the step's order (the active tree's leaves, Morton order): the row's walk of the
    /// active tree with the `partner > row` filter, then of the static tree (then, unreported,
    /// of the sleeper tree). Returns what the pass covered beside the rest of the step's pair
    /// set, so a caller can check [`QueryPassTotals::pairs`] against the step's output.
    ///
    /// Read-only: the trees, the records and the pair list are the step's. A brute step
    /// (`N ≤ brute_max_rows`) touches none of them while the sleeper set is empty (it dissolves
    /// the set otherwise; module docs), so after one the replay is still of the last tree-path
    /// step.
    ///
    /// # Panics
    ///
    /// If a replayed row's emitted partner count differs from the segment the step wrote for it:
    /// the replay would then not have walked what the step walked.
    pub fn count_query_pass(&self, mut sink: impl FnMut(RowQueryCounts)) -> QueryPassTotals {
        let recs = self.rec[usize::from(self.cur)].as_read_slice();
        let mut totals = QueryPassTotals::default();
        for slot in 0..self.active.leaves() {
            let leaf = self.active.leaf(slot);
            let row = leaf.row;
            let mut emitted_active = 0u32;
            self.active.query(leaf.x, leaf.y, leaf.z, leaf.r, |t| {
                emitted_active += u32::from(t > row);
            });
            let active = self.active.last_query();
            let mut emitted_statics = 0u32;
            self.statics.query(leaf.x, leaf.y, leaf.z, leaf.r, |_| {
                emitted_statics += 1;
            });
            let statics = self.statics.last_query();
            // L10 C3c: the sleeper tree's partners belong to the segment too; its walk is not
            // reported (an empty sleeper tree, every step without the sleep-skip, emits none).
            let mut emitted_sleepers = 0u32;
            self.sleepers.query(leaf.x, leaf.y, leaf.z, leaf.r, |_| {
                emitted_sleepers += 1;
            });
            let rec = &recs[row as usize];
            assert_eq!(
                emitted_active + emitted_statics + emitted_sleepers,
                rec.nrev + rec.nfwd,
                "row {row}: the replay emits the segment the step wrote"
            );
            totals.rows += 1;
            totals.emitted += u64::from(emitted_active + emitted_statics + emitted_sleepers);
            sink(RowQueryCounts { row, active, statics, emitted_active, emitted_statics });
        }
        for rec in recs.iter().filter(|rec| rec.kind() == KIND_WIDE) {
            totals.wide_rows += 1;
            totals.wide_emitted += u64::from(rec.nrev + rec.nfwd);
        }
        totals.static_pairs = self.ss.as_read_slice().len() as u64;
        totals
    }

    /// What the last leaf-list pass did: the pass of the last tree-path step under
    /// [`QueryKernel::LeafList`], or of the last [`query_stage`](Self::query_stage) re-run under
    /// it.
    pub fn leaf_list_counts(&self) -> LeafListCounts {
        self.ll_counts
    }

    /// Re-runs the query stage of the last tree-path step under `kernel` and returns its bytes,
    /// borrowed from the tree: the stream (every Q and Wide row's segment) and every row's
    /// `(seg, nrev, nfwd)`. The trees and the records' bits are the step's and the stage reads
    /// nothing else, so the re-run is the step's stage under the other kernel; the selected
    /// kernel and the structural counters are restored.
    ///
    /// Allocates nothing: a driver that compares two kernels' stages copies what it keeps, as
    /// [`count_query_pass`](Self::count_query_pass) hands its rows to a sink (module docs, "How
    /// a pass is counted").
    pub fn query_stage(
        &mut self,
        kernel: QueryKernel,
    ) -> (&[u32], impl Iterator<Item = (u32, u32, u32)> + '_) {
        let n = self.rec[usize::from(self.cur)].as_read_slice().len();
        let (kernel_before, diag_before) = (self.kernel, self.diag);
        self.kernel = kernel;
        self.query_all(n);
        self.kernel = kernel_before;
        self.diag = diag_before;
        let records = self.rec[usize::from(self.cur)]
            .as_read_slice()
            .iter()
            .map(|r| (r.seg, r.nrev, r.nfwd));
        (self.aux.as_read_slice(), records)
    }

    /// The active tree's shape (the Q rows of the last tree-path step).
    pub fn active_shape(&self) -> TreeShape {
        self.active.shape()
    }

    /// The static tree's shape (the members of S, with the lanes killed since its last build).
    pub fn static_shape(&self) -> TreeShape {
        self.statics.shape()
    }
}

/// The shape of the tree whose nodes are `nodes`, level `L` starting at `level_start[L]`.
pub(crate) fn shape_of(
    nodes: &[Node8],
    level_start: &[u32; MAX_LEVELS + 1],
    levels: usize,
    leaves: u32,
    dead: u32,
) -> TreeShape {
    let mut shape = TreeShape {
        levels: levels as u32,
        leaf_slots: leaves,
        live: leaves - dead,
        leaf_occupancy: [0; LANES + 1],
        child_counts: [0; LANES + 1],
        level: [LevelShape::default(); TREE_MAX_LEVELS],
    };
    for level in 0..levels {
        let first = level_start[level] as usize;
        let end = level_start[level + 1] as usize;
        let mut acc = LevelShape::default();
        let mut ratio_sum = 0.0f64;
        let mut ratio_nodes = 0u32;
        for node in &nodes[first..end] {
            let mut children = 0usize;
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            let mut child_area = 0.0f64;
            let mut child_volume = 0.0f64;
            for k in 0..LANES {
                let child = if level == 0 { leaf_lane_box(node, k) } else { internal_lane_box(node, k) };
                let Some((clo, chi)) = child else {
                    continue;
                };
                children += 1;
                child_area += area(clo, chi);
                child_volume += volume(clo, chi);
                for a in 0..3 {
                    lo[a] = lo[a].min(clo[a]);
                    hi[a] = hi[a].max(chi[a]);
                }
            }
            if level == 0 {
                shape.leaf_occupancy[children] += 1;
            } else {
                shape.child_counts[children] += 1;
            }
            acc.nodes += 1;
            acc.children += children as u32;
            if children == 0 {
                continue;
            }
            let node_area = area(lo, hi);
            acc.child_area += child_area;
            acc.node_area += node_area;
            acc.child_volume += child_volume;
            acc.node_volume += volume(lo, hi);
            if node_area > 0.0 {
                let ratio = child_area / node_area;
                ratio_sum += ratio;
                ratio_nodes += 1;
                acc.area_ratio_max = acc.area_ratio_max.max(ratio);
            }
        }
        if ratio_nodes > 0 {
            acc.area_ratio_mean = ratio_sum / f64::from(ratio_nodes);
        }
        shape.level[level] = acc;
    }
    shape
}

/// The padded box of a leaf node's lane `k`, or `None` for an empty or killed lane: the box
/// `leaf_bounds` unions into the parent lane.
#[inline]
fn leaf_lane_box(node: &Node8, k: usize) -> Option<([f32; 3], [f32; 3])> {
    if node.p[LEAF_ROW][k].to_bits() == NO_LANE_ROW {
        return None;
    }
    let (x, y, z, r) = (node.p[LEAF_X][k], node.p[LEAF_Y][k], node.p[LEAF_Z][k], node.p[LEAF_R][k]);
    let h = padded_radius(x, y, z, r);
    Some(([x - h, y - h, z - h], [x + h, y + h, z + h]))
}

/// The box of an internal node's lane `k`, or `None` for an empty lane (`+inf` mins, `-inf`
/// maxes).
#[inline]
fn internal_lane_box(node: &Node8, k: usize) -> Option<([f32; 3], [f32; 3])> {
    let lo = [node.p[MIN_X][k], node.p[MIN_Y][k], node.p[MIN_Z][k]];
    let hi = [node.p[MAX_X][k], node.p[MAX_Y][k], node.p[MAX_Z][k]];
    if lo[0] > hi[0] { None } else { Some((lo, hi)) }
}

/// Surface area of the box `[lo, hi]`, in `f64`.
#[inline]
fn area(lo: [f32; 3], hi: [f32; 3]) -> f64 {
    let (dx, dy, dz) = extents(lo, hi);
    2.0 * (dx * dy + dy * dz + dz * dx)
}

/// Volume of the box `[lo, hi]`, in `f64`.
#[inline]
fn volume(lo: [f32; 3], hi: [f32; 3]) -> f64 {
    let (dx, dy, dz) = extents(lo, hi);
    dx * dy * dz
}

#[inline]
fn extents(lo: [f32; 3], hi: [f32; 3]) -> (f64, f64, f64) {
    (
        f64::from(hi[0]) - f64::from(lo[0]),
        f64::from(hi[1]) - f64::from(lo[1]),
        f64::from(hi[2]) - f64::from(lo[2]),
    )
}
