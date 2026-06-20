//! Layout resources — the viewport seed and the reused per-frame scratch.
//!
//! Both are engine-owned `Resource`s (Principle-0 legitimate storage), not side
//! `std::Vec`/`HashMap` stores. [`LayoutScratch`] is strictly frame-transient: it
//! holds only `Entity` handles + POD work items, is reset every frame, and never
//! caches per-node durable layout state across frames.

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_macros::Resource;

use crate::components::UiAlign;
use crate::units::LayoutType;

/// Maximum layout recursion depth = depth-pool length = cycle/pathological-depth
/// guard. Real UI trees are shallow; a tree deeper than this is clamped to a leaf
/// (debug-asserted).
pub(crate) const MAX_LAYOUT_DEPTH: usize = 128;

/// Screen-space root seed. Set by the host (window/swapchain) on surface create
/// and resize.
///
/// `scale_factor` folds DPI (logical = physical / scale); P1 layout is
/// logical-px, P5a applies `scale_factor` at upload. The host bumps `generation`
/// on every resize so the layout pass can detect a resize without a per-row tick
/// (resources have no `Changed` semantics).
#[derive(Resource, Clone, Copy, Debug)]
pub struct UiViewport {
    /// Logical width in pixels.
    pub width: f32,
    /// Logical height in pixels.
    pub height: f32,
    /// DPI scale factor (logical = physical / scale).
    pub scale_factor: f32,
    /// Bumped by the host on every resize; a mismatch vs the last-acted-on value
    /// marks all roots dirty.
    pub generation: u32,
}

impl Default for UiViewport {
    #[inline]
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            scale_factor: 1.0,
            generation: 0,
        }
    }
}

/// Which container-relative slot a stretch participant fills during the main-axis
/// freeze loop.
#[derive(Clone, Copy)]
pub(crate) enum StretchTarget {
    /// The relative child at this index in the sorted flow order.
    Child(u32),
    /// The gap that follows the child at this index. RESERVED for parent-applied
    /// stretch gaps (a `Stretch` `row_gap`/`column_gap`); P1 resolves gaps as
    /// fixed lengths, so this variant is not yet constructed.
    #[allow(dead_code)]
    GapAfter(u32),
}

/// The resolved final main + cross size of a relative child, computed once during
/// the measure phase (Pass B/D.5) and read back by the positioning passes without
/// re-measuring. POD; lives in the pass-stable flat [`LayoutScratch::child_sizes`]
/// arena, ranged per parent by `MeasuredNode::size_lo..size_hi`.
#[derive(Clone, Copy, Default)]
pub(crate) struct ChildSize {
    /// Final main-axis extent (freeze-computed for stretch items, else resolved).
    pub main: f32,
    /// Final cross-axis extent (honors `AlignCross::Stretch` / a Stretch cross).
    pub cross: f32,
}

/// A main/cross size pair (the algorithm works in axis-relative coordinates and
/// folds to x/y/w/h only when writing).
#[derive(Clone, Copy)]
pub(crate) struct Size {
    pub main: f32,
    pub cross: f32,
}

/// Resolved spacing insets in axis-relative coordinates.
#[derive(Clone, Copy)]
pub(crate) struct Insets {
    pub main_before: f32,
    pub main_after: f32,
    pub cross_before: f32,
    pub cross_after: f32,
    pub gap: f32,
}

impl Insets {
    pub const ZERO: Self = Self {
        main_before: 0.0,
        main_after: 0.0,
        cross_before: 0.0,
        cross_after: 0.0,
        gap: 0.0,
    };

    #[inline]
    pub fn main_total(&self) -> f32 {
        self.main_before + self.main_after
    }
    #[inline]
    pub fn cross_total(&self) -> f32 {
        self.cross_before + self.cross_after
    }
}

/// The output of the measure phase (Passes A–D) for one node: its resolved size,
/// content box, axis-relative insets, partition boundary, layout type, align, and
/// skip flag. The positioning phase consumes this from the arena without
/// re-measuring.
#[derive(Clone, Copy)]
pub(crate) struct Measured {
    pub lt: LayoutType,
    pub align: UiAlign,
    pub size: Size,
    pub content_main: f32,
    pub content_cross: f32,
    pub insets: Insets,
    pub relative_count: usize,
    /// `true` if this flow container had ANY main-axis stretch participant (the
    /// `stretch_pool[depth]` was non-empty). Stored because the freeze pool is
    /// depth-transient and gone by the position pass, which needs this exact gate
    /// to suppress AlignMain distribution when stretch consumed the free space.
    pub has_stretch: bool,
    /// `true` if the node has no `UiLayout` (an unlayoutable node — skip it).
    pub skip: bool,
}

/// One node's measure result in the flat DFS-order arena
/// ([`LayoutScratch::measured`]). POD `Copy`. Children are appended to the arena
/// BEFORE their parent (post-order append), so a parent's contiguous child run is
/// recorded as a half-open `[child_lo, child_hi)` range into the
/// [`LayoutScratch::child_index`] side-lane (flow order: relative first, absolute
/// tail). The position pass walks parents top-down and reads children by that
/// range — it never re-enters `measure_node`. Frame-transient: the whole arena is
/// `clear()`-reset every `layout_root`.
#[derive(Clone, Copy)]
pub(crate) struct MeasuredNode {
    /// The entity this slot measures (the position pass writes this node's
    /// children's rects against it).
    pub entity: Entity,
    /// The Pass A–D result: resolved size, content box, axis-relative insets,
    /// partition boundary, layout type, align, skip flag.
    pub measured: Measured,
    /// Half-open range into [`LayoutScratch::child_index`] of THIS node's children
    /// (flow order: relative first, absolute tail). `child_lo == child_hi` = leaf.
    pub child_lo: u32,
    pub child_hi: u32,
    /// Half-open range into [`LayoutScratch::child_sizes`] of THIS node's resolved
    /// relative-child sizes (the former per-depth `child_size_pool`, now
    /// pass-stable). `size_lo == size_hi` for an Overlay/leaf container.
    pub size_lo: u32,
    pub size_hi: u32,
}

/// One participant in a container's main-axis stretch freeze loop. POD.
#[derive(Clone, Copy)]
pub(crate) struct StretchItem {
    /// What this item resizes (a child or a gap).
    pub target: StretchTarget,
    /// Flex-grow factor.
    pub factor: f32,
    /// Lower clamp (Auto-min resolved to measured content main before freeze).
    pub min: f32,
    /// Upper clamp (`f32::MAX` sentinel if unbounded).
    pub max: f32,
    /// `factor * free / sum` for the current round (the "measured" share).
    pub base_share: f32,
    /// `clamp(base_share, min, max)` — the resolved size once frozen.
    pub computed: f32,
    /// Whether this item has been frozen (clamped and removed from the pool).
    pub frozen: bool,
}

/// Reused per-frame scratch (a `Resource` — engine storage, allocated ONCE at
/// setup via [`LayoutScratch::with_seeds`], capacity persists, only
/// `clear()`/index-reset per frame).
///
/// `Default` is fully EMPTY so it is a valid `mem::take` target (the apply
/// system moves the buffers onto its stack for the recursion, then moves them
/// back, freeing the world borrow without dropping any capacity).
#[derive(Resource)]
pub struct LayoutScratch {
    /// Depth-indexed child working sets. `child_pool[d]` is the sorted child list
    /// for the container being laid out at recursion depth `d`. Each inner `Vec`
    /// is reused across frames (cleared on entry, capacity retained). Depth-
    /// indexing is required because `layout_node` recurses while its own working
    /// set is live — a shared buffer would be clobbered by the recursion.
    pub(crate) child_pool: Vec<Vec<Entity>>,
    /// Depth-indexed stretch working sets (same depth-isolation rationale).
    pub(crate) stretch_pool: Vec<Vec<StretchItem>>,
    /// Depth-indexed accumulator of a node's children's ARENA indices, in flow
    /// order (relative first, absolute tail), filled as Pass B/B.5 measure each
    /// child. Depth-indexed for the same reason as `child_pool`: it is live across
    /// the `depth+1` child recursion (which pushes grandchildren into the next
    /// level's lane). After all children are measured it is copied CONTIGUOUSLY
    /// into the pass-stable `child_index` arena, so a parent's child run is a clean
    /// `[child_lo, child_hi)` range despite post-order arena append.
    pub(crate) child_idx_pool: Vec<Vec<u32>>,
    /// The flat DFS-order measure arena: every node's [`MeasuredNode`] in
    /// post-order append order (children before parent), produced by the bottom-up
    /// measure pass and read by the top-down position pass. This — together with
    /// `child_index` + `child_sizes` — is the pass-stable memo that closes the
    /// exponential re-entry: each node is measured ONCE (a small bounded reflow
    /// factor aside), and the position pass never re-enters `measure_node`. Reused
    /// across frames; `clear()`-reset per `layout_root` (capacity retained).
    pub(crate) measured: Vec<MeasuredNode>,
    /// Side-lane of child arena indices (into `measured`), in flow order, ranged
    /// per parent by `MeasuredNode::child_lo..child_hi`. Lets the position pass get
    /// a parent's contiguous child run despite post-order arena append. Reused;
    /// `clear()`-reset per `layout_root`.
    pub(crate) child_index: Vec<u32>,
    /// Pass-stable per-child resolved sizes, ranged per parent by
    /// `MeasuredNode::size_lo..size_hi`. Replaces the per-depth `child_size_pool`:
    /// promoting it to a flat pass-stable arena is exactly what stops
    /// `resolve_child_sizes`/positioning from re-entering `measure_node`. Each
    /// entry is the resolved size of the relative child at the matching flow index;
    /// absolute children occupy the parent's `child_index` tail and are sized from
    /// their own arena `measured` entry. Frame-transient POD, reused across frames.
    pub(crate) child_sizes: Vec<ChildSize>,
    /// Cached root entity list (the `UiRoot`-tagged entities). Refreshed only when
    /// `roots_dirty` is set (an `Added<UiRoot>` or a structural change that could
    /// have removed a root), NEVER per dirty frame — this replaces a per-frame
    /// `query_entities` allocation on the change path with a steady-state read of a
    /// reused buffer. Dead/removed entries are skipped at use (a despawned root's
    /// `UiLayout` read returns `None`). Frame-transient POD reused across frames.
    pub(crate) roots: Vec<Entity>,
    /// Set by discovery when an `Added<UiRoot>` or a structural change is observed,
    /// telling apply to refresh the cached `roots` list before relaying. Cleared by
    /// apply after the refresh.
    pub(crate) roots_dirty: bool,
    /// Whether apply has ever populated `roots` (so the very first dirty frame
    /// refreshes even if no `Added<UiRoot>` was caught in the discovery window —
    /// e.g. roots spawned before the systems were first scheduled).
    pub(crate) roots_initialized: bool,
    /// Last viewport generation the layout pass acted on (resize detection).
    pub(crate) last_viewport_generation: u32,
    /// Discovery's per-frame "any layout input changed this frame" flag, written
    /// by `ui_layout_discovery` and consumed (then cleared) by `ui_layout_apply`.
    pub(crate) dirty: bool,
    /// `#[cfg(test)]` relayout counter (number of roots relaid this frame). A
    /// production hook the tests read; layout never branches on it.
    #[cfg(test)]
    pub relayout_count: u32,
    /// `#[cfg(test)]` complexity probe: the number of `measure_node` ENTRIES this
    /// `layout_root` pass (reset at pass entry, before Pass 1). The linear-scaling
    /// tests assert the genuine `O(N * stretch_nesting_depth)` envelope —
    /// `measure_visits` stays at least `N` and well under the exponential
    /// `(3*branching)^depth` it replaced (for shallow flexible nesting the factor
    /// is the typical `~3x`, but it can grow linearly with stretch nesting depth,
    /// bounded by `MAX_LAYOUT_DEPTH`). Snapshotting it before Pass 2 and asserting
    /// it is unchanged after also proves the position pass never re-enters
    /// `measure_node`. Layout never branches on it.
    #[cfg(test)]
    pub measure_visits: u32,
}

impl Default for LayoutScratch {
    #[inline]
    fn default() -> Self {
        // Default == fully EMPTY (a valid mem::take target).
        Self {
            child_pool: Vec::new(),
            stretch_pool: Vec::new(),
            child_idx_pool: Vec::new(),
            measured: Vec::new(),
            child_index: Vec::new(),
            child_sizes: Vec::new(),
            roots: Vec::new(),
            roots_dirty: false,
            roots_initialized: false,
            last_viewport_generation: 0,
            dirty: false,
            #[cfg(test)]
            relayout_count: 0,
            #[cfg(test)]
            measure_visits: 0,
        }
    }
}

impl LayoutScratch {
    /// Builds a scratch with every depth-pool buffer seeded to a modest capacity
    /// so the first frames after a new high-water container do not reallocate.
    ///
    /// The host calls this ONCE at setup and inserts the result as a `Resource`.
    pub fn with_seeds() -> Self {
        const SEED_ROOTS: usize = 8; // typical max screen-space roots
        const SEED_FANOUT: usize = 32; // typical max children per container
        const SEED_STRETCH: usize = 16; // typical max stretch items per container
        // High-water seed for the flat pass-stable arenas (total nodes laid out in
        // one pass). A larger tree triggers one amortized Vec growth on its first
        // frame, then the capacity is retained (the documented growth exception,
        // identical to today's child_pool).
        const SEED_NODES: usize = 256;
        let mut child_pool = Vec::with_capacity(MAX_LAYOUT_DEPTH);
        let mut stretch_pool = Vec::with_capacity(MAX_LAYOUT_DEPTH);
        let mut child_idx_pool = Vec::with_capacity(MAX_LAYOUT_DEPTH);
        for _ in 0..MAX_LAYOUT_DEPTH {
            child_pool.push(Vec::with_capacity(SEED_FANOUT));
            stretch_pool.push(Vec::with_capacity(SEED_STRETCH));
            child_idx_pool.push(Vec::with_capacity(SEED_FANOUT));
        }
        Self {
            child_pool,
            stretch_pool,
            child_idx_pool,
            measured: Vec::with_capacity(SEED_NODES),
            child_index: Vec::with_capacity(SEED_NODES),
            child_sizes: Vec::with_capacity(SEED_NODES),
            roots: Vec::with_capacity(SEED_ROOTS),
            roots_dirty: false,
            roots_initialized: false,
            last_viewport_generation: 0,
            dirty: false,
            #[cfg(test)]
            relayout_count: 0,
            #[cfg(test)]
            measure_visits: 0,
        }
    }
}
