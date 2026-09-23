//! The implicit packed 8-wide BVH (`04-DESIGN-REV2.md`, D2): build, query and kill.
//!
//! # Layout
//!
//! One [`ScratchColumn`] of [`Node8`], level 0 (the leaves) first and the root last. Node `i` of
//! level `L ≥ 1` has children `8·i .. 8·i + 8` of level `L − 1`, so there is no child pointer:
//! `level_start[L] + i` addresses the node and `level_start[L − 1] + 8·i + k` its lane `k`. A
//! leaf node holds up to eight rows in Morton order; the leaf **slot** of a row is
//! `8 · leaf_node + lane`, and a slot is what a member record remembers.
//!
//! * An internal lane holds its child's AABB (`min_x, min_y, min_z, max_x, max_y, max_z`), the
//!   union of the padded leaf boxes below it. An empty lane holds `+inf` mins and `-inf` maxes,
//!   so no query box meets it.
//! * A leaf lane holds `x, y, z, r` and the row's bits. An empty or killed lane holds
//!   `x = +inf` and row [`NO_LANE_ROW`]; the exact test on it is always false, because every
//!   query is a Normal row whose bound is finite.
//!
//! # Kill
//!
//! Killing a lane rewrites the leaf only. The boxes above it keep covering the dead lane, which
//! costs a box hit now and then and never a wrong answer. A rebuild (admission or compaction)
//! drops the dead lanes.
//!
//! # Build
//!
//! The rows are sorted by the Morton code of their position quantised to a 10-bit grid over
//! the rows' bounding box (an LSD radix sort over the 30 code bits in three 10-bit passes,
//! stable, so equal codes keep row order), then packed eight to a leaf. Each level above is
//! the union of the level below. Every step of the build reads only the rows it is given, so a
//! build is a pure function of the row list and the tree is the same on every machine — not
//! that the pair set depends on it: the assembly canonicalises the order.

use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

use super::kernel::{QueryBox, box_mask, leaf_mask, padded_radius};

#[cfg(feature = "bp-query-counts")]
use super::counts::{QueryCounts, QueryProbe, TreeShape, shape_of};

/// Index of the `min_x` row of an internal node's `p`.
pub(crate) const MIN_X: usize = 0;
/// Index of the `min_y` row of an internal node's `p`.
pub(crate) const MIN_Y: usize = 1;
/// Index of the `min_z` row of an internal node's `p`.
pub(crate) const MIN_Z: usize = 2;
/// Index of the `max_x` row of an internal node's `p`.
pub(crate) const MAX_X: usize = 3;
/// Index of the `max_y` row of an internal node's `p`.
pub(crate) const MAX_Y: usize = 4;
/// Index of the `max_z` row of an internal node's `p`.
pub(crate) const MAX_Z: usize = 5;
/// Index of the `x` row of a leaf node's `p`.
pub(crate) const LEAF_X: usize = 0;
/// Index of the `y` row of a leaf node's `p`.
pub(crate) const LEAF_Y: usize = 1;
/// Index of the `z` row of a leaf node's `p`.
pub(crate) const LEAF_Z: usize = 2;
/// Index of the `r` row of a leaf node's `p`.
pub(crate) const LEAF_R: usize = 3;
/// Index of the row-bits row of a leaf node's `p` (a `u32` carried as `f32` bits, never used
/// in arithmetic).
pub(crate) const LEAF_ROW: usize = 4;

/// The row bits of an empty or killed leaf lane. Rows stay below `2^24`, so it is never a row.
pub(crate) const NO_LANE_ROW: u32 = u32::MAX;

/// Lanes per node.
pub(crate) const LANES: usize = 8;

/// The most levels a tree can have: `8^13 > 2^24` rows, the row bound of this crate.
pub(crate) const MAX_LEVELS: usize = 13;

/// The traversal stack's capacity: at most seven pushes net per internal level plus the root
/// (`7 · (MAX_LEVELS − 1) + 1 = 85`).
const STACK: usize = 96;

/// Bits of a Morton axis: a 1024-cell grid per axis, 30 code bits in all.
const MORTON_BITS: u32 = 10;

/// Radix digit width of the Morton sort, and so its histogram size.
const RADIX_BITS: u32 = 10;

/// One node of the tree: six rows of eight lanes, 192 B, three cache lines, 32-aligned so the
/// AVX2 kernel loads each row aligned.
#[repr(C, align(32))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Node8 {
    /// See the module docs for the meaning of each row.
    pub(crate) p: [[f32; 8]; 6],
}

const _: () = assert!(size_of::<Node8>() == 192 && align_of::<Node8>() == 32);

impl Node8 {
    /// An internal node with eight empty lanes.
    const EMPTY_INTERNAL: Self = Self {
        p: [
            [f32::INFINITY; 8],
            [f32::INFINITY; 8],
            [f32::INFINITY; 8],
            [f32::NEG_INFINITY; 8],
            [f32::NEG_INFINITY; 8],
            [f32::NEG_INFINITY; 8],
        ],
    };

    /// A leaf node with eight empty lanes.
    const EMPTY_LEAF: Self = Self {
        p: [
            [f32::INFINITY; 8],
            [0.0; 8],
            [0.0; 8],
            [0.0; 8],
            [f32::from_bits(NO_LANE_ROW); 8],
            [0.0; 8],
        ],
    };
}

/// A row the build reads: its bits and its row. 32 B.
#[repr(C, align(32))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Item {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
    pub(crate) r: f32,
    pub(crate) row: u32,
    pub(crate) _pad: [u32; 3],
}

const _: () = assert!(size_of::<Item>() == 32 && align_of::<Item>() == 32);

impl Item {
    /// An item for `row` at `(x, y, z)` with bounding radius `r`.
    #[inline]
    pub(crate) const fn new(x: f32, y: f32, z: f32, r: f32, row: u32) -> Self {
        Self { x, y, z, r, row, _pad: [0; 3] }
    }
}

/// One leaf lane's contents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Leaf {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
    pub(crate) r: f32,
    /// The row, or [`NO_LANE_ROW`] for an empty or killed lane.
    pub(crate) row: u32,
}

/// The tree.
pub(crate) struct PackedBvh8 {
    /// Level 0 first, root last.
    nodes: ScratchColumn<Node8>,
    /// `level_start[L]` is the first node of level `L`; `level_start[levels]` the node count.
    level_start: [u32; MAX_LEVELS + 1],
    /// Levels in the tree; `0` for an empty tree.
    levels: u8,
    /// Leaf slots in use: live plus killed lanes. Slots `0 .. leaves` are the items given to
    /// the last build, in Morton order.
    leaves: u32,
    /// Killed lanes among them.
    dead: u32,
    /// The last query's counts (`bp-query-counts` only).
    #[cfg(feature = "bp-query-counts")]
    probe: QueryProbe,
}

impl PackedBvh8 {
    /// An empty tree on the column `id`, whose layout must already be registered as [`Node8`].
    pub(crate) fn new(id: ComponentId, reserve_nodes: usize) -> Self {
        Self {
            nodes: ScratchColumn::new(id, reserve_nodes),
            level_start: [0; MAX_LEVELS + 1],
            levels: 0,
            leaves: 0,
            dead: 0,
            #[cfg(feature = "bp-query-counts")]
            probe: QueryProbe::new(),
        }
    }

    /// Leaf slots in use by the last build (live and killed).
    #[inline]
    pub(crate) fn leaves(&self) -> u32 {
        self.leaves
    }

    /// Killed lanes since the last build.
    #[inline]
    pub(crate) fn dead(&self) -> u32 {
        self.dead
    }

    /// Live lanes.
    #[inline]
    pub(crate) fn live(&self) -> u32 {
        self.leaves - self.dead
    }

    /// The contents of leaf slot `slot`.
    #[inline]
    pub(crate) fn leaf(&self, slot: u32) -> Leaf {
        debug_assert!(slot < self.leaves, "invariant: leaf slot {slot} is past the build");
        let node = &self.nodes.as_read_slice()[(slot as usize) / LANES];
        let k = (slot as usize) % LANES;
        Leaf {
            x: node.p[LEAF_X][k],
            y: node.p[LEAF_Y][k],
            z: node.p[LEAF_Z][k],
            r: node.p[LEAF_R][k],
            row: node.p[LEAF_ROW][k].to_bits(),
        }
    }

    /// The row in leaf slot `slot`, or [`NO_LANE_ROW`] when the lane is killed.
    #[inline]
    pub(crate) fn leaf_row(&self, slot: u32) -> u32 {
        debug_assert!(slot < self.leaves, "invariant: leaf slot {slot} is past the build");
        self.nodes.as_read_slice()[(slot as usize) / LANES].p[LEAF_ROW][(slot as usize) % LANES]
            .to_bits()
    }

    /// Rewrites the row of the live lane `slot` (a row translation). The lane's bits are
    /// unchanged, so no box above it moves.
    #[inline]
    pub(crate) fn set_leaf_row(&mut self, slot: u32, row: u32) {
        debug_assert!(slot < self.leaves, "invariant: leaf slot {slot} is past the build");
        let mut view = self.nodes.build_view();
        let node = &mut view.as_mut_slice()[(slot as usize) / LANES];
        debug_assert!(
            node.p[LEAF_ROW][(slot as usize) % LANES].to_bits() != NO_LANE_ROW,
            "invariant: a killed lane is never re-rowed"
        );
        node.p[LEAF_ROW][(slot as usize) % LANES] = f32::from_bits(row);
    }

    /// Kills the live lane `slot`: its exact test becomes false and it is dropped by the next
    /// build.
    #[inline]
    pub(crate) fn kill(&mut self, slot: u32) {
        debug_assert!(slot < self.leaves, "invariant: leaf slot {slot} is past the build");
        let mut view = self.nodes.build_view();
        let node = &mut view.as_mut_slice()[(slot as usize) / LANES];
        let k = (slot as usize) % LANES;
        debug_assert!(
            node.p[LEAF_ROW][k].to_bits() != NO_LANE_ROW,
            "invariant: a lane is killed at most once"
        );
        node.p[LEAF_X][k] = f32::INFINITY;
        node.p[LEAF_ROW][k] = f32::from_bits(NO_LANE_ROW);
        self.dead += 1;
        debug_assert!(self.dead <= self.leaves, "invariant: dead ≤ leaves");
    }

    /// Rebuilds the tree over `items` (every one a Normal row), with `keys_a` / `keys_b` as the
    /// radix sort's ping-pong buffers. After the build, leaf slot `s` holds the `s`-th item in
    /// Morton order and no lane is dead.
    pub(crate) fn build(
        &mut self,
        items: &[Item],
        keys_a: &mut ScratchColumn<u64>,
        keys_b: &mut ScratchColumn<u64>,
    ) {
        let n = items.len();
        debug_assert!(n < 1 << 24, "invariant: rows stay below 2^24");
        self.dead = 0;
        self.leaves = n as u32;
        if n == 0 {
            self.nodes.build_view().clear();
            self.levels = 0;
            return;
        }

        // Level geometry.
        let mut count = [0u32; MAX_LEVELS];
        count[0] = n.div_ceil(LANES) as u32;
        let mut levels = 1usize;
        while count[levels - 1] > 1 {
            debug_assert!(levels < MAX_LEVELS, "invariant: 13 levels cover 2^24 rows");
            count[levels] = (count[levels - 1] as usize).div_ceil(LANES) as u32;
            levels += 1;
        }
        let mut start = 0u32;
        for (level, &c) in count.iter().enumerate().take(levels) {
            self.level_start[level] = start;
            start += c;
        }
        self.level_start[levels] = start;
        self.levels = levels as u8;
        let total = start as usize;

        // Morton order of the items.
        let order = morton_sort(items, keys_a, keys_b);

        let mut nodes = self.nodes.build_view();
        nodes.clear();
        nodes.resize(total, Node8::EMPTY_INTERNAL);
        let nodes = nodes.as_mut_slice();
        for node in nodes.iter_mut().take(count[0] as usize) {
            *node = Node8::EMPTY_LEAF;
        }
        for (slot, &key) in order.iter().enumerate() {
            let item = &items[(key & 0xffff_ffff) as usize];
            let node = &mut nodes[slot / LANES];
            let k = slot % LANES;
            node.p[LEAF_X][k] = item.x;
            node.p[LEAF_Y][k] = item.y;
            node.p[LEAF_Z][k] = item.z;
            node.p[LEAF_R][k] = item.r;
            node.p[LEAF_ROW][k] = f32::from_bits(item.row);
        }

        // Each level above is the union of the level below.
        for level in 1..levels {
            let child_start = self.level_start[level - 1] as usize;
            let child_count = count[level - 1] as usize;
            let (below, above) = nodes.split_at_mut(self.level_start[level] as usize);
            let children = &below[child_start..child_start + child_count];
            for (i, node) in above.iter_mut().take(count[level] as usize).enumerate() {
                for k in 0..LANES {
                    let c = i * LANES + k;
                    if c >= child_count {
                        break;
                    }
                    let (lo, hi) = if level == 1 {
                        leaf_bounds(&children[c])
                    } else {
                        internal_bounds(&children[c])
                    };
                    node.p[MIN_X][k] = lo[0];
                    node.p[MIN_Y][k] = lo[1];
                    node.p[MIN_Z][k] = lo[2];
                    node.p[MAX_X][k] = hi[0];
                    node.p[MAX_Y][k] = hi[1];
                    node.p[MAX_Z][k] = hi[2];
                }
            }
        }
    }

    /// Walks the tree for the Normal row at `(x, y, z)` with bounding radius `r`, calling
    /// `accept(row)` for every live lane whose exact test passes. The caller filters (its own
    /// row, the `row > query` rule of the active tree, the admission rule).
    ///
    /// Under `bp-query-counts` the walk is counted and the counts are published for
    /// `last_query`; without the feature no counting statement exists.
    #[inline]
    pub(crate) fn query(&self, x: f32, y: f32, z: f32, r: f32, mut accept: impl FnMut(u32)) {
        if self.levels == 0 {
            #[cfg(feature = "bp-query-counts")]
            self.probe.publish(QueryCounts::default());
            return;
        }
        let nodes = self.nodes.as_read_slice();
        let q = QueryBox::of(x, y, z, r);
        let root_level = usize::from(self.levels) - 1;
        // Entries pack `(level, index within level)`.
        let mut stack = [0u32; STACK];
        let mut depth = 1usize;
        stack[0] = pack(root_level, 0);
        #[cfg(feature = "bp-query-counts")]
        let mut counts = QueryCounts::default();
        while depth > 0 {
            depth -= 1;
            let (level, index) = unpack(stack[depth]);
            let node_index = self.level_start[level] as usize + index;
            debug_assert!(node_index < self.level_start[level + 1] as usize);
            // SAFETY: `index < count[level]` for every entry pushed (the root is index 0 of a
            // level with one node; a child index `8·i + k` is pushed only when the parent's lane
            // `k` holds a child, which `build` writes for `8·i + k < count[level − 1]` only —
            // an empty lane's `+inf / −inf` box meets no query box). So
            // `level_start[level] + index < level_start[level + 1] ≤ nodes.len()`.
            let node = unsafe { nodes.get_unchecked(node_index) };
            if level == 0 {
                let mut mask = leaf_mask(node, x, y, z, r);
                #[cfg(feature = "bp-query-counts")]
                counts.note_leaf(node, &q, mask);
                while mask != 0 {
                    let k = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    let row = node.p[LEAF_ROW][k].to_bits();
                    debug_assert_ne!(row, NO_LANE_ROW, "a killed lane's test is always false");
                    accept(row);
                }
            } else {
                let mut mask = box_mask(node, &q);
                #[cfg(feature = "bp-query-counts")]
                counts.note_internal(mask);
                while mask != 0 {
                    let k = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    debug_assert!(depth < STACK, "invariant: stack depth ≤ 7·levels + 1");
                    stack[depth] = pack(level - 1, index * LANES + k);
                    depth += 1;
                }
            }
        }
        #[cfg(feature = "bp-query-counts")]
        self.probe.publish(counts);
    }
}

/// The counting build's read side: the last query's counts and the tree's shape.
#[cfg(feature = "bp-query-counts")]
impl PackedBvh8 {
    /// The counts of the last [`query`](Self::query) on this tree.
    #[inline]
    pub(crate) fn last_query(&self) -> QueryCounts {
        self.probe.last()
    }

    /// The tree's shape: levels, occupancy, child counts and per-level box figures.
    pub(crate) fn shape(&self) -> TreeShape {
        shape_of(
            self.nodes.as_read_slice(),
            &self.level_start,
            usize::from(self.levels),
            self.leaves,
            self.dead,
        )
    }
}

/// Packs a traversal stack entry.
#[inline]
const fn pack(level: usize, index: usize) -> u32 {
    ((level as u32) << 28) | (index as u32)
}

/// Unpacks a traversal stack entry.
#[inline]
const fn unpack(entry: u32) -> (usize, usize) {
    ((entry >> 28) as usize, (entry & 0x0fff_ffff) as usize)
}

/// The union of a leaf node's live lanes' padded boxes.
#[inline]
fn leaf_bounds(node: &Node8) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for k in 0..LANES {
        if node.p[LEAF_ROW][k].to_bits() == NO_LANE_ROW {
            continue;
        }
        let (x, y, z, r) = (node.p[LEAF_X][k], node.p[LEAF_Y][k], node.p[LEAF_Z][k], node.p[LEAF_R][k]);
        let h = padded_radius(x, y, z, r);
        lo[0] = lo[0].min(x - h);
        lo[1] = lo[1].min(y - h);
        lo[2] = lo[2].min(z - h);
        hi[0] = hi[0].max(x + h);
        hi[1] = hi[1].max(y + h);
        hi[2] = hi[2].max(z + h);
    }
    (lo, hi)
}

/// The union of an internal node's lane boxes.
#[inline]
fn internal_bounds(node: &Node8) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for k in 0..LANES {
        lo[0] = lo[0].min(node.p[MIN_X][k]);
        lo[1] = lo[1].min(node.p[MIN_Y][k]);
        lo[2] = lo[2].min(node.p[MIN_Z][k]);
        hi[0] = hi[0].max(node.p[MAX_X][k]);
        hi[1] = hi[1].max(node.p[MAX_Y][k]);
        hi[2] = hi[2].max(node.p[MAX_Z][k]);
    }
    (lo, hi)
}

/// Spreads the low ten bits of `v` to every third bit.
#[inline]
const fn spread3(v: u32) -> u32 {
    let mut x = v & 0x3ff;
    x = (x | (x << 16)) & 0x0300_00ff;
    x = (x | (x << 8)) & 0x0300_f00f;
    x = (x | (x << 4)) & 0x030c_30c3;
    x = (x | (x << 2)) & 0x0924_9249;
    x
}

/// The 30-bit Morton code of a position quantised over `[lo, hi]` per axis.
#[inline]
pub(crate) fn morton(p: [f32; 3], lo: [f32; 3], scale: [f32; 3]) -> u32 {
    let cell = |k: usize| -> u32 {
        // `as u32` saturates, and a NaN (impossible for a Normal row) would map to 0.
        let c = ((p[k] - lo[k]) * scale[k]) as u32;
        c.min((1 << MORTON_BITS) - 1)
    };
    spread3(cell(0)) | (spread3(cell(1)) << 1) | (spread3(cell(2)) << 2)
}

/// Sorts `items` by Morton code, stable, returning the keys `(code << 32) | item index` in
/// sorted order (a slice of one of the two buffers).
fn morton_sort<'a>(
    items: &[Item],
    keys_a: &'a mut ScratchColumn<u64>,
    keys_b: &'a mut ScratchColumn<u64>,
) -> &'a [u64] {
    let n = items.len();
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for it in items {
        lo[0] = lo[0].min(it.x);
        lo[1] = lo[1].min(it.y);
        lo[2] = lo[2].min(it.z);
        hi[0] = hi[0].max(it.x);
        hi[1] = hi[1].max(it.y);
        hi[2] = hi[2].max(it.z);
    }
    let cells = ((1u32 << MORTON_BITS) - 1) as f32;
    let mut scale = [0.0f32; 3];
    for k in 0..3 {
        let span = hi[k] - lo[k];
        scale[k] = if span > 0.0 { cells / span } else { 0.0 };
    }

    {
        let mut a = keys_a.build_view();
        a.clear();
        for (i, it) in items.iter().enumerate() {
            let code = morton([it.x, it.y, it.z], lo, scale);
            a.push((u64::from(code) << 32) | i as u64);
        }
    }
    keys_b.build_view().resize(n, 0);

    // Three stable LSD passes over the code bits: a → b → a → b.
    let mut hist = [0u32; 1 << RADIX_BITS];
    let mut src_is_a = true;
    for pass in 0..3u32 {
        let shift = 32 + pass * RADIX_BITS;
        let (src, dst) = if src_is_a {
            (keys_a.as_read_slice(), keys_b.build_view())
        } else {
            (keys_b.as_read_slice(), keys_a.build_view())
        };
        let mut dst = dst;
        let dst = dst.as_mut_slice();
        hist.fill(0);
        for &k in src {
            hist[((k >> shift) & ((1 << RADIX_BITS) - 1)) as usize] += 1;
        }
        let mut sum = 0u32;
        for h in hist.iter_mut() {
            let c = *h;
            *h = sum;
            sum += c;
        }
        for &k in src {
            let d = ((k >> shift) & ((1 << RADIX_BITS) - 1)) as usize;
            dst[hist[d] as usize] = k;
            hist[d] += 1;
        }
        src_is_a = !src_is_a;
    }
    debug_assert!(!src_is_a, "three passes end in keys_b");
    let sorted = keys_b.as_read_slice();
    debug_assert!(sorted.windows(2).all(|w| (w[0] >> 32) <= (w[1] >> 32)), "Morton sort is sorted");
    sorted
}
