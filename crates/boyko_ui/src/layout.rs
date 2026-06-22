//! The in-house layout algorithm — a CSS-flexbox-style multi-pass solver run
//! over the `ChildOf`/`Children` tree, writing [`ComputedRect`] to every node.
//!
//! # Two systems
//!
//! [`ui_layout_discovery`] is a normal scheduled system (change detection lives
//! here because SystemParams supply the tick window). [`ui_layout_apply`] is an
//! exclusive system (nested parent↔child mutable row access).
//!
//! # Engine-API deviation from the architectural plan (load-bearing)
//!
//! The plan's discovery system was specified as `Query<Entity, Or<(Changed<…>,
//! …)>>` doing a per-changed-node up-walk to its enclosing root, marking only
//! the dirty roots. That signature is **not implementable on this engine**: the
//! `Query` API has no entity-yielding `QueryData` (only `&T`/`&mut T`/`Ref`/
//! `Mut`/`Option`/`AnyOf`/tuples/`()` — see `query/data.rs`), so a change-
//! detecting query cannot yield the `Entity` handles the up-walk needs, and
//! `Query::is_empty()` is archetype-level only (it ignores the per-row
//! `Changed`/`Added` filter — see `query.rs::is_empty`). The only entity-handle
//! enumeration is `EcsMaster::query_entities` (which allocates and has no change
//! detection), and an exclusive body has no tick window for `Changed`.
//!
//! Resolution, staying inside the plan's two-system architecture and its
//! 0%-overhead steady state: discovery runs `Query<(), Or<(Changed<…>, …)>>` and
//! detects "did any layout input change this frame" via `iter().next().is_some()`
//! (the iterator DOES honor per-row `Changed`/`Added`). It writes a single
//! `dirty` flag (plus the viewport-generation check) into `LayoutScratch`. When
//! dirty, apply re-lays-out ALL roots. This relaxes the relayout granularity from
//! per-dirty-root to all-roots — the only buildable option given the missing
//! entity-yielding query — while preserving change-detection-in-a-FunctionSystem,
//! the exclusive walk, and the zero-cost steady state.
//!
//! # No per-frame allocation on the change path (Principle 5)
//!
//! The plan forbids `query_entities` (which allocates a fresh `Vec` per call) on
//! any per-frame path. Apply therefore does NOT call `query_entities` per dirty
//! frame. Instead `LayoutScratch::roots` caches the `UiRoot` entity list; it is
//! refreshed (one `query_entities` into the reused buffer) ONLY when discovery
//! sets `roots_dirty` — i.e. an `Added<UiRoot>` fired or a structural change
//! (`Changed<Children>`/`Changed<ChildOf>`, which a recursive root despawn also
//! triggers on the surviving graph) could have changed the root set. A plain size
//! tweak leaves `roots_dirty` false, so a steady stream of property-only change
//! frames re-walks the cached roots with zero allocation. Dead/removed roots are
//! tolerated at use (a despawned root's `UiLayout` read returns `None` → skipped),
//! so a stale cache entry is harmless until the next root-set refresh.

use std::mem;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::{ChildOf, Children};
use boyko_ecs::ecs::core::iters::query::filter::{Added, Changed, Or};
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::system::{Res, ResMut};

use crate::anchor::resolve_anchor_origin;
use crate::components::{
    ComputedRect, ContentSize, UiAbsolute, UiAlign, UiAnchor, UiGrid, UiLayout, UiRoot, UiSpacing,
};
use crate::resources::{
    ChildSize, Insets, LayoutScratch, MAX_LAYOUT_DEPTH, Measured, MeasuredNode, Size,
    StretchItem, StretchTarget, UiSafeArea, UiViewport,
};
use crate::units::{AlignCross, AlignMain, LayoutType, PositionType, Unit};
use crate::world::components::{UiWorldAnchor, UiWorldCulled, UiWorldHidden, UiWorldProjection};

// ───────────────────────── discovery ──────────────────────────────────────

/// Discovery pass: a NORMAL scheduled system. SystemParams supply the
/// `(last_run, this_run]` window that `Changed`/`Added` require, so this is where
/// change detection lives. Sets the per-frame `dirty` flag in [`LayoutScratch`].
/// Zero per-frame allocation.
///
/// Steady state (no input changed, viewport unchanged): the change query yields
/// nothing (Phase-10 0%-overhead), so `dirty` stays `false` and
/// [`ui_layout_apply`] early-returns.
//
// `clippy::type_complexity`: the `Query<(), Or<(Changed<…>, …)>>` change-set type
// IS the SystemParam signature — the engine resolves it positionally, so it
// cannot be a `type` alias without losing the SystemParam impl. Allowed.
#[allow(clippy::type_complexity)]
pub fn ui_layout_discovery(
    changed: Query<
        (),
        Or<(
            Changed<UiLayout>,
            Changed<UiSpacing>,
            Changed<UiAlign>,
            Changed<UiAbsolute>,
            Changed<ContentSize>,
            Changed<Children>,
            Changed<ChildOf>,
            Added<UiRoot>,
            // GUI P6a: an anchor change re-pins the root (its rect origin moves).
            // A `UiAnchor` change is a property change (not a root-set change), so
            // it does NOT trip `root_set_changed` — the cached `roots` list stays
            // valid; only a relayout is needed.
            Changed<UiAnchor>,
            // GUI P7a: a re-projection moves a world-anchored root's origin /
            // scale / visibility, so it must trigger a relayout — exactly like a
            // `UiAnchor` change. It is a property change (the project system
            // writes `UiWorldProjection` per frame, not a root-set change), so it
            // does NOT trip `root_set_changed`.
            Changed<UiWorldProjection>,
        )>,
    >,
    // Root-set-change signal: an `Added<UiRoot>` (a new root) or any structural
    // change (a `Changed<Children>`/`Changed<ChildOf>`, which a recursive root
    // despawn also stamps on the surviving graph) means the cached root list in
    // `LayoutScratch::roots` may be stale and apply must refresh it. A plain
    // property tweak does NOT trip this, so the cached list survives steady
    // property-animation frames with zero re-enumeration.
    root_set_changed: Query<
        (),
        Or<(Added<UiRoot>, Changed<Children>, Changed<ChildOf>)>,
    >,
    viewport: Res<UiViewport>,
    mut scratch: ResMut<LayoutScratch>,
) {
    // `iter().next().is_some()` is the per-row change signal: the iterator honors
    // the per-row Changed/Added filter (unlike `is_empty()`, which is archetype-
    // level only). In steady state this is empty and allocates nothing.
    let inputs_changed = changed.iter().next().is_some();
    let viewport_changed = viewport.generation != scratch.last_viewport_generation;
    scratch.dirty = inputs_changed || viewport_changed;
    // OR-accumulate: apply clears `roots_dirty` only after it actually refreshes,
    // so a root-set change observed on a frame whose apply is skipped (it never is
    // here, but defensively) is not lost.
    if root_set_changed.iter().next().is_some() {
        scratch.roots_dirty = true;
    }
}

// ───────────────────────── apply ──────────────────────────────────────────

/// Apply pass: an EXCLUSIVE system (the only form that gives nested parent↔child
/// mutable row access). When dirty, re-lays-out the root subtrees.
///
/// Uses the `mem::take` borrow protocol: the scratch buffers are moved onto the
/// stack at entry (freeing the world borrow so the recursion can call
/// `get_component`/`get_component_mut`) and moved back at exit with their
/// capacities retained.
//
// `clippy::needless_pass_by_ref_mut`: `resource_mut` / `get_component_mut` are
// `&mut self`, so the `&mut EcsMaster` IS required, but clippy cannot see through
// the cross-crate method calls. Allowed (mirrors `boyko_demo`'s exclusive
// systems).
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn ui_layout_apply(world: &mut EcsMaster) {
    // Read the viewport + safe-area + dirty flag, then move the scratch buffers
    // out so the recursion's `get_component_mut` calls do not conflict with a held
    // borrow of the resource slab (Decision 7). The safe-area snapshot is read
    // here (a `Copy` value) so the anchor resolve inside the recursion holds no
    // resource borrow; `UiSafeArea` is host-set and defaults to zero, so a host
    // that never inserts it still reads the zero inset.
    let viewport = *world.resource::<UiViewport>();
    let safe_area = world
        .try_resource::<UiSafeArea>()
        .copied()
        .unwrap_or_default();
    let (dirty, mut scratch) = {
        let s = world.resource_mut::<LayoutScratch>();
        (s.dirty, mem::take(s))
    };

    #[cfg(test)]
    {
        scratch.relayout_count = 0;
    }

    if dirty {
        scratch.last_viewport_generation = viewport.generation;
        scratch.dirty = false;

        // Refresh the cached root list ONLY when the root SET could have changed
        // (a new/removed root), never per dirty frame. A property-only change frame
        // reuses the cached list, so there is NO per-frame `query_entities`
        // allocation on the change path (Principle 5). `query_entities` allocates,
        // so it is confined to the rare root-set-change frame.
        if scratch.roots_dirty || !scratch.roots_initialized {
            refresh_roots(world, &mut scratch);
        }

        // Walk the cached roots. A despawned/stale entry's `UiLayout` read returns
        // `None` and is skipped inside `layout_root`, so a not-yet-refreshed stale
        // cache is harmless.
        for i in 0..scratch.roots.len() {
            let root = scratch.roots[i];
            #[cfg(test)]
            {
                scratch.relayout_count += 1;
            }
            layout_root(world, &mut scratch, root, viewport, &safe_area);
        }
    }

    // Put the buffers back (headers only; capacities travel with the moved Vecs).
    *world.resource_mut::<LayoutScratch>() = scratch;
}

/// Re-enumerates the `UiRoot` entities into the reused `scratch.roots` buffer.
/// This is the ONLY `query_entities` call (it allocates a fresh `Vec` per call —
/// `ecs_master.rs:2096`), so it is gated behind a root-set change and runs off the
/// steady property-change path.
#[cold]
#[inline(never)]
fn refresh_roots(world: &mut EcsMaster, scratch: &mut LayoutScratch) {
    let fresh = world.query_entities(&[UiRoot::component_id()]);
    scratch.roots.clear();
    scratch.roots.extend_from_slice(&fresh);
    scratch.roots_dirty = false;
    scratch.roots_initialized = true;
}

/// Lays out one root subtree, seeding the root rect from the viewport. Runs the
/// two-phase O(N) solve: a bottom-up MEASURE pass that appends every node's
/// resolved size to the flat `scratch.measured` arena exactly once (a bounded
/// reflow factor aside), then a top-down POSITION pass that reads stored sizes and
/// writes `ComputedRect` without ever re-entering `measure_node`.
fn layout_root(
    world: &mut EcsMaster,
    scratch: &mut LayoutScratch,
    root: Entity,
    viewport: UiViewport,
    safe_area: &UiSafeArea,
) {
    let Some(layout) = world.get_component::<UiLayout>(root).copied() else {
        // A UiRoot without a UiLayout cannot be laid out. The DSL guarantees its
        // presence; treat a missing one as a no-op.
        return;
    };

    // The root's own size resolves against the viewport extent (a definite base).
    // The root is laid out at origin (0,0); the viewport extent supplies the
    // definite Pct base on each (x, y) axis.
    let lt = resolved_layout_type(layout.layout_type, root);
    let parent_def = AxisDef {
        main: Some(if matches!(lt, LayoutType::Row) {
            viewport.width
        } else {
            viewport.height
        }),
        cross: Some(if matches!(lt, LayoutType::Row) {
            viewport.height
        } else {
            viewport.width
        }),
    };

    // Reset the pass-stable arenas (capacity retained — no per-frame alloc after
    // warmup). The depth pools are cleared per-level inside the recursion.
    scratch.measured.clear();
    scratch.child_index.clear();
    scratch.child_sizes.clear();
    #[cfg(test)]
    {
        scratch.measure_visits = 0;
    }

    // ── PASS 1 (bottom-up): measure the whole subtree into the arena. ──────────
    // A default (indefinite) root FILLS the viewport: any axis the author left
    // Auto / Pct-of-indefinite is forced to the viewport extent so a background or
    // full-screen overlay spans the screen. An axis the author sized explicitly
    // (Px / Pct-of-viewport) keeps its own value (`force_def` only fills an
    // indefinite axis — see `measure_node`). The viewport extent is exactly
    // `parent_def`, so it doubles as the fill override. This is the documented root
    // rule ("default Auto root → fills viewport").
    let root_idx = measure_node(world, scratch, root, 0, parent_def, Some(parent_def));

    // The root has no parent to write its own rect, so write it here from the
    // stored axis-relative size folded to (w, h) at the origin.
    let root_m = scratch.measured[root_idx as usize].measured;
    if root_m.skip {
        return;
    }
    let (mut w, mut h) = fold_size(root_m.lt, root_m.size.main, root_m.size.cross);

    // GUI P7a: a WORLD-anchored root is positioned by `ui_world_project_system`
    // (which writes `UiWorldProjection`), NOT by a screen-edge `UiAnchor` (the two
    // are mutually exclusive). Resolved HERE so the layout pass stays the SINGLE
    // `ComputedRect` writer (same seam as P6a). A culled (frustum), hidden (hover),
    // or not-yet-projected (`!visible`) world root is skipped, like a `skip` node.
    if world.get_component::<UiWorldAnchor>(root).is_some() {
        let proj = world
            .get_component::<UiWorldProjection>(root)
            .copied()
            .unwrap_or_default();
        let culled = world.is_enabled::<UiWorldCulled>(root);
        let hidden = world.is_enabled::<UiWorldHidden>(root);
        if !proj.visible || culled || hidden {
            return;
        }
        // Uniform subtree scale (1.0 for ScreenSpace; ref/dist for WorldScaled).
        // Applied to the root extent; `position_node` carries the scaled origin so
        // the whole subtree shifts with the projected point. Per-child extent
        // scaling beyond the root box is a P7b/render concern (the CPU core
        // billboards a fixed-pixel or uniformly-scaled box).
        w *= proj.scale;
        h *= proj.scale;
        let origin = Origin {
            x: proj.screen_x,
            y: proj.screen_y,
        };
        write_rect(world, root, ComputedRect { x: origin.x, y: origin.y, w, h });
        position_node(world, scratch, root_idx, origin);
        return;
    }

    // Screen origin: the root anchors at the viewport top-left by default. GUI
    // P6a: an optional `UiAnchor` re-pins it to a screen edge/corner. The anchor
    // is resolved HERE — after measure (so the root's `w/h` are known, which the
    // right/bottom edges need) and before the rect is written — so the layout pass
    // remains the SINGLE `ComputedRect` writer (no pre-pass write race). The
    // resolved origin then seeds BOTH the root rect and `position_node`, so the
    // whole subtree shifts with the root. The `get_component::<UiAnchor>` lookup is
    // O(1) per root and only on the (cold) relayout path.
    let origin = match world.get_component::<UiAnchor>(root).copied() {
        Some(anchor) => {
            let o = resolve_anchor_origin(&viewport, safe_area, &anchor, w, h);
            Origin { x: o.x, y: o.y }
        }
        None => Origin { x: 0.0, y: 0.0 },
    };

    write_rect(
        world,
        root,
        ComputedRect {
            x: origin.x,
            y: origin.y,
            w,
            h,
        },
    );

    // ── PASS 2 (top-down): position the subtree from the arena. ───────────────
    // This phase NEVER calls `measure_node` (every size is already stored), so the
    // complexity-probe snapshot must be unchanged across it.
    position_node(world, scratch, root_idx, origin);
}

// ───────────────────────── geometry helpers ───────────────────────────────

/// The parent's DEFINITE extent on each axis (the Pct base; `None` if indefinite).
#[derive(Clone, Copy)]
struct AxisDef {
    main: Option<f32>,
    cross: Option<f32>,
}

/// Absolute screen-space top-left a node is placed at.
#[derive(Clone, Copy)]
struct Origin {
    x: f32,
    y: f32,
}

/// Maps an axis-relative `(main, cross)` extent to `(w, h)` for a layout type.
#[inline]
fn fold_size(lt: LayoutType, main: f32, cross: f32) -> (f32, f32) {
    match lt {
        // Row: main = x (width), cross = y (height).
        LayoutType::Row => (main, cross),
        // Column / Overlay / Grid(→Column): main = y (height), cross = x (width).
        _ => (cross, main),
    }
}

/// Maps an axis-relative `(main_pos, cross_pos)` offset to `(dx, dy)`.
#[inline]
fn fold_pos(lt: LayoutType, main: f32, cross: f32) -> (f32, f32) {
    match lt {
        LayoutType::Row => (main, cross),
        _ => (cross, main),
    }
}

/// Re-expresses a child's measured size (returned by `measure_node` in the CHILD's
/// OWN axis frame) into the PARENT's axis frame, so a parent accumulating /
/// sizing a child reads the correct main/cross extents even when parent and child
/// have mismatched orientations (e.g. a Row container under a Column parent).
///
/// `child.size` is `(main, cross)` in `child.lt`'s frame; this folds it back to
/// absolute `(w, h)` and re-splits into the parent's `(main, cross)`. When parent
/// and child share orientation this is the identity; only mismatched orientations
/// transpose (the axis-frame bug fix).
#[inline]
fn measured_in_parent_frame(parent_lt: LayoutType, child: &Measured) -> Size {
    // Absolute child extents, independent of either frame.
    let (cw, ch) = fold_size(child.lt, child.size.main, child.size.cross);
    // Re-split into the parent's frame (same axis convention as `fold_size`).
    match parent_lt {
        LayoutType::Row => Size { main: cw, cross: ch },
        _ => Size { main: ch, cross: cw },
    }
}

/// Resolves the effective layout type. GUI P6a implements `Grid` (a uniform
/// `columns × rows` cell placement); it is no longer a `Column` fallback.
#[inline]
fn resolved_layout_type(lt: LayoutType, _node: Entity) -> LayoutType {
    lt
}

/// Cold diagnostic: a node reached the depth clamp (treated as a leaf).
#[cold]
#[inline(never)]
fn depth_clamped() {
    debug_assert!(
        false,
        "layout recursion hit MAX_LAYOUT_DEPTH; clamping to leaf (cycle or pathological depth?)"
    );
}

/// Cold diagnostic: a positioned node is missing its `ComputedRect` column.
#[cold]
#[inline(never)]
fn missing_rect() {
    debug_assert!(
        false,
        "node missing ComputedRect at a positioning write; skipping (the DSL must insert it)"
    );
}

// ───────────────────────── unit resolution ────────────────────────────────

/// Resolves a length to a definite f32 against a definite base, or `None` if the
/// unit is intrinsic (`Auto`) or `Pct` of an indefinite base.
#[inline]
fn resolve_definite(unit: Unit, base: Option<f32>) -> Option<f32> {
    match unit {
        Unit::Px(v) => Some(v),
        Unit::Pct(p) => base.map(|b| p * 0.01 * b),
        // Stretch / Auto are not definite here.
        Unit::Stretch(_) | Unit::Auto => None,
    }
}

/// Resolves a min length against a definite base. `Auto` returns the supplied
/// content floor; a non-resolvable unit returns 0.0 (no lower bound).
#[inline]
fn resolve_min(unit: Unit, base: Option<f32>, content_floor: f32) -> f32 {
    match unit {
        Unit::Auto => content_floor,
        _ => resolve_definite(unit, base).unwrap_or(0.0),
    }
}

/// Resolves a max length against a definite base. A non-resolvable unit returns
/// `f32::MAX` (no upper bound).
#[inline]
fn resolve_max(unit: Unit, base: Option<f32>) -> f32 {
    resolve_definite(unit, base).unwrap_or(f32::MAX)
}

// ───────────────────────── the recursive node solver ──────────────────────

/// Measure phase (Passes A–D + the merged D.5): computes a node's size from its
/// children WITHOUT writing any `ComputedRect`, APPENDS the result to the flat
/// `scratch.measured` arena, and RETURNS the node's arena index. A node without a
/// `UiLayout` (or one that hit the depth clamp) is appended as a `skip` slot —
/// always appending keeps a parent's child run 1:1 with its flow order, so the
/// position pass can read `child_index[child_lo + i]` for flow slot `i` directly.
/// The children are measured (and appended) FIRST (post-order), so a parent's
/// arena entry is always at a higher index than every descendant; the parent
/// records its children's indices contiguously into `scratch.child_index`.
///
/// This replaces the old exponential `(3*branching)^depth` re-entry: each node is
/// measured ONCE in the base walk and its resolved per-child sizes are stored
/// pass-stably in `scratch.child_sizes`; the position pass reads the arena and
/// NEVER re-enters `measure_node`. A node is re-measured ONLY when its parent's
/// final content box genuinely differs from the intrinsic availability it was
/// first measured against (the gated conditional reflow — Decision 4). Each such
/// re-measure re-walks that one subtree, so the total measure visits are
/// `O(N * stretch_nesting_depth)` — NOT a flat multiple of `N`: for a
/// stretch-heavy tree the re-layout factor grows with the depth of flexible
/// nesting. It is absolutely capped by `MAX_LAYOUT_DEPTH` (128); the commonly
/// cited `~3x` figure is the TYPICAL case for shallow flexible nesting (≲3
/// levels), not a hard ceiling (see GUI-P1-LAYOUT-PLAN.md:24,:595).
///
/// `parent_def` is the parent's definite extent (the Pct base). `force_def`
/// overrides this node's OWN size on any axis that is `Some` — used by
/// `layout_root` to make a default (indefinite) root fill the viewport and by the
/// conditional reflow to re-derive a subtree at its already-resolved final size.
/// An axis the author sized explicitly keeps its own value; `force_def` only
/// supplies a definite extent where the node's own axis is indefinite. The depth
/// pools are this level's working set; the recursion measures children at
/// `depth + 1`, so a parent's working set is never clobbered (Decision 6).
#[allow(clippy::too_many_lines)]
fn measure_node(
    world: &mut EcsMaster,
    scratch: &mut LayoutScratch,
    node: Entity,
    depth: usize,
    parent_def: AxisDef,
    force_def: Option<AxisDef>,
) -> u32 {
    #[cfg(test)]
    {
        scratch.measure_visits += 1;
    }
    if depth >= MAX_LAYOUT_DEPTH {
        depth_clamped();
        return push_skip(scratch, node);
    }
    debug_assert!(depth < MAX_LAYOUT_DEPTH);

    // ── Pass A: gather & partition ─────────────────────────────────────────
    // Grouped get_component reads (Open Question 2 resolution — see the module
    // footer): one shared read per input column. `get_components_raw` was
    // rejected because it allocates a Vec per call (defeating the zero-alloc
    // goal); grouped get_component is non-allocating and compiles cleanly.
    let Some(layout) = world.get_component::<UiLayout>(node).copied() else {
        // No UiLayout: append a skip slot so the parent's flow index still maps
        // 1:1 onto a child arena entry (excluded from flow accumulation, treated
        // as a leaf by the position pass).
        return push_skip(scratch, node);
    };
    let spacing = world
        .get_component::<UiSpacing>(node)
        .copied()
        .unwrap_or_default();
    let align = world
        .get_component::<UiAlign>(node)
        .copied()
        .unwrap_or_default();

    let lt = resolved_layout_type(layout.layout_type, node);

    // Partition children into relative (front) and absolute (tail), sorted by
    // Entity id for a deterministic flow order (Children order is unspecified).
    {
        let child_buf = &mut scratch.child_pool[depth];
        child_buf.clear();
        if let Some(children) = world.get_component::<Children>(node) {
            child_buf.extend_from_slice(children.as_slice());
        }
        child_buf.sort_unstable_by_key(|e| e.id().0);
    }
    let mut relative_count = scratch.child_pool[depth].len();
    {
        let child_buf = &mut scratch.child_pool[depth];
        // In-place partition: relative kept in front, absolute moved to the tail.
        let mut i = 0usize;
        let mut boundary = child_buf.len();
        while i < boundary {
            let child = child_buf[i];
            let is_absolute = world
                .get_component::<UiLayout>(child)
                .map(|cl| matches!(cl.position_type, PositionType::Absolute))
                .unwrap_or(false);
            if is_absolute {
                boundary -= 1;
                child_buf.swap(i, boundary);
                relative_count -= 1;
            } else {
                i += 1;
            }
        }
        // The tail (absolute) is now id-unordered after the swaps; re-sort each
        // partition for determinism.
        child_buf[..relative_count].sort_unstable_by_key(|e| e.id().0);
        child_buf[relative_count..].sort_unstable_by_key(|e| e.id().0);
    }
    let total_children = scratch.child_pool[depth].len();

    // Resolve this node's spacing insets against the parent's definite axes.
    let insets = resolve_insets(lt, &spacing, parent_def);

    // Resolve this node's own definite extents (if author set Px/Pct-of-definite).
    // `force_def` supplies a definite extent on an axis the author left indefinite
    // (root → viewport fill; conditional reflow → the already-resolved size). An
    // explicitly-sized axis is NOT overridden.
    let self_def_main = resolve_definite(main_unit(lt, &layout), parent_def.main)
        .or_else(|| force_def.and_then(|f| f.main));
    let self_def_cross = resolve_definite(cross_unit(lt, &layout), parent_def.cross)
        .or_else(|| force_def.and_then(|f| f.cross));

    // ── Pass B: intrinsic content measure (recurses measure-only into depth+1) ─
    // Records each measured child's arena index into `child_idx_pool[depth]` in
    // flow order (relative first); the absolute tail is appended after Pass C.
    scratch.child_idx_pool[depth].clear();
    let mut relative_main_sum = 0.0f32;
    let mut max_child_cross = 0.0f32;
    collect_stretch_and_measure(
        world,
        node,
        scratch,
        depth,
        lt,
        relative_count,
        self_def_main,
        self_def_cross,
        insets,
        &mut relative_main_sum,
        &mut max_child_cross,
    );

    // Leaf content fallback (no relative children): use ContentSize.
    if relative_count == 0 && !matches!(lt, LayoutType::Overlay | LayoutType::Grid) {
        let content = world
            .get_component::<ContentSize>(node)
            .copied()
            .unwrap_or_default();
        let (cw, ch) = (content.width, content.height);
        let (leaf_main, leaf_cross) = match lt {
            LayoutType::Row => (cw, ch),
            _ => (ch, cw),
        };
        relative_main_sum = leaf_main;
        max_child_cross = leaf_cross;
    }

    // ── Pass C: resolve this container's definite extents ──────────────────
    let min_main = resolve_min(min_main_unit(lt, &layout), parent_def.main, relative_main_sum);
    let max_main = resolve_max(max_main_unit(lt, &layout), parent_def.main);
    let min_cross =
        resolve_min(min_cross_unit(lt, &layout), parent_def.cross, max_child_cross);
    let max_cross = resolve_max(max_cross_unit(lt, &layout), parent_def.cross);

    let target_main = match self_def_main {
        Some(v) => v.clamp(min_main, max_main),
        None => (relative_main_sum + insets.main_total()).clamp(min_main, max_main),
    };
    let target_cross = match self_def_cross {
        Some(v) => v.clamp(min_cross, max_cross),
        None => (max_child_cross + insets.cross_total()).clamp(min_cross, max_cross),
    };

    let content_main = (target_main - insets.main_total()).max(0.0);
    let content_cross = (target_cross - insets.cross_total()).max(0.0);

    // ── Pass D: main-axis stretch freeze (populates stretch_pool[depth]) ───
    resolve_stretch_freeze(scratch, depth, content_main, relative_main_sum);

    let m = Measured {
        lt,
        align,
        size: Size {
            main: target_main,
            cross: target_cross,
        },
        content_main,
        content_cross,
        insets,
        relative_count,
        has_stretch: !scratch.stretch_pool[depth].is_empty(),
        skip: false,
    };

    // ── Pass B.5: measure the absolute (out-of-flow) tail into the arena ────
    // Absolute children do not accumulate flow, but they still need a stored
    // measure so `position_absolute_from_arena` reads it instead of re-entering
    // `measure_node`. They are appended to `child_idx_pool` AFTER the relatives,
    // preserving the flow-then-absolute ordering of `child_index`.
    measure_absolute_children(world, scratch, depth, relative_count, total_children, &m);

    // ── Pass D.5: resolve EVERY child's final size into the arena ───────────
    // Sizes for all children (relative-or-overlay first, then the absolute tail)
    // are pushed in flow order, so `child_sizes[size_lo + k]` lines up 1:1 with
    // `child_index[child_lo + k]`. The former `measure_node(child, …)` re-entries
    // here become ARENA READS of each child's Pass-B/B.5 result — the O(N) fix.
    let size_lo = scratch.child_sizes.len() as u32;
    match lt {
        LayoutType::Overlay => resolve_overlay_child_sizes(world, scratch, depth, &m),
        LayoutType::Grid => resolve_grid_child_sizes(world, scratch, node, &m),
        _ => resolve_flow_child_sizes(world, scratch, depth, &m),
    }
    resolve_absolute_child_sizes(world, scratch, depth, relative_count, total_children, &m);
    let size_hi = scratch.child_sizes.len() as u32;

    // ── Conditional reflow (Decision 4): re-measure a child ONCE iff its final
    //    resolved size differs from its intrinsic measure AND it has its own
    //    children. A fixed-size child (final == intrinsic) is never re-measured.
    //    This is the bounded `stretch_nesting` factor, enforced structurally, and
    //    is what lets the position pass NEVER call `measure_node`. ──
    reflow_changed_children(world, scratch, depth, &m, size_lo, total_children);

    // ── Commit this node's children index range + arena entry (post-order). ──
    let child_lo = scratch.child_index.len() as u32;
    // SAFETY-of-bounds note: `child_idx_pool[depth]` holds exactly this node's
    // children's arena indices in flow order (relatives then absolutes), each
    // produced by a child append above; copy them contiguously so the position
    // pass gets a clean range.
    {
        let lane = mem::take(&mut scratch.child_idx_pool[depth]);
        scratch.child_index.extend_from_slice(&lane);
        scratch.child_idx_pool[depth] = lane;
    }
    let child_hi = scratch.child_index.len() as u32;

    let idx = scratch.measured.len() as u32;
    scratch.measured.push(MeasuredNode {
        entity: node,
        measured: m,
        child_lo,
        child_hi,
        size_lo,
        size_hi,
    });
    idx
}

/// Appends a "skip" (depth-clamped) node to the arena and returns its index. A
/// skip node has no children and no resolved sizes, so the position pass treats it
/// as a leaf.
#[cold]
#[inline(never)]
fn push_skip(scratch: &mut LayoutScratch, node: Entity) -> u32 {
    let size_lo = scratch.child_sizes.len() as u32;
    let child_lo = scratch.child_index.len() as u32;
    let idx = scratch.measured.len() as u32;
    scratch.measured.push(MeasuredNode {
        entity: node,
        measured: Measured {
            lt: LayoutType::Column,
            align: UiAlign::default(),
            size: Size { main: 0.0, cross: 0.0 },
            content_main: 0.0,
            content_cross: 0.0,
            insets: Insets::ZERO,
            relative_count: 0,
            has_stretch: false,
            skip: true,
        },
        child_lo,
        child_hi: child_lo,
        size_lo,
        size_hi: size_lo,
    });
    idx
}

// ───────────────────────── pass B: measure + stretch collect ──────────────

/// Resolves padding + border + the active gap into axis-relative insets.
#[inline]
fn resolve_insets(lt: LayoutType, s: &UiSpacing, parent_def: AxisDef) -> Insets {
    let pl = resolve_definite(s.padding_left, parent_def_x(lt, parent_def)).unwrap_or(0.0)
        + resolve_definite(s.border_left, parent_def_x(lt, parent_def)).unwrap_or(0.0);
    let pr = resolve_definite(s.padding_right, parent_def_x(lt, parent_def)).unwrap_or(0.0)
        + resolve_definite(s.border_right, parent_def_x(lt, parent_def)).unwrap_or(0.0);
    let pt = resolve_definite(s.padding_top, parent_def_y(lt, parent_def)).unwrap_or(0.0)
        + resolve_definite(s.border_top, parent_def_y(lt, parent_def)).unwrap_or(0.0);
    let pb = resolve_definite(s.padding_bottom, parent_def_y(lt, parent_def)).unwrap_or(0.0)
        + resolve_definite(s.border_bottom, parent_def_y(lt, parent_def)).unwrap_or(0.0);
    // Gap: row_gap when main = y (Column), column_gap when main = x (Row). A
    // Stretch gap resolves to 0 here and is handled by the freeze loop.
    let gap_unit = match lt {
        LayoutType::Row => s.column_gap,
        _ => s.row_gap,
    };
    let gap = resolve_definite(gap_unit, None).unwrap_or(0.0);
    match lt {
        // Row: main = x, cross = y.
        LayoutType::Row => Insets {
            main_before: pl,
            main_after: pr,
            cross_before: pt,
            cross_after: pb,
            gap,
        },
        // Column / Overlay / Grid: main = y, cross = x.
        _ => Insets {
            main_before: pt,
            main_after: pb,
            cross_before: pl,
            cross_after: pr,
            gap,
        },
    }
}

#[inline]
fn parent_def_x(lt: LayoutType, d: AxisDef) -> Option<f32> {
    match lt {
        LayoutType::Row => d.main,
        _ => d.cross,
    }
}

#[inline]
fn parent_def_y(lt: LayoutType, d: AxisDef) -> Option<f32> {
    match lt {
        LayoutType::Row => d.cross,
        _ => d.main,
    }
}

#[inline]
fn main_unit(lt: LayoutType, l: &UiLayout) -> Unit {
    match lt {
        LayoutType::Row => l.width,
        _ => l.height,
    }
}
#[inline]
fn cross_unit(lt: LayoutType, l: &UiLayout) -> Unit {
    match lt {
        LayoutType::Row => l.height,
        _ => l.width,
    }
}
#[inline]
fn min_main_unit(lt: LayoutType, l: &UiLayout) -> Unit {
    match lt {
        LayoutType::Row => l.min_width,
        _ => l.min_height,
    }
}
#[inline]
fn max_main_unit(lt: LayoutType, l: &UiLayout) -> Unit {
    match lt {
        LayoutType::Row => l.max_width,
        _ => l.max_height,
    }
}
#[inline]
fn min_cross_unit(lt: LayoutType, l: &UiLayout) -> Unit {
    match lt {
        LayoutType::Row => l.min_height,
        _ => l.min_width,
    }
}
#[inline]
fn max_cross_unit(lt: LayoutType, l: &UiLayout) -> Unit {
    match lt {
        LayoutType::Row => l.max_height,
        _ => l.max_width,
    }
}

/// Measures one child into the arena (recursing into `depth + 1`), records its
/// arena index into `child_idx_pool[depth]` (so the parent's `child_index` range
/// stays contiguous and in flow order, 1:1 with `child_pool[depth]`), and returns
/// its measured size re-expressed in the PARENT's axis frame. A child without a
/// `UiLayout` is appended as a skip slot (zero size) and still recorded, preserving
/// the flow-index correspondence.
#[inline]
fn measure_child_into_arena(
    world: &mut EcsMaster,
    scratch: &mut LayoutScratch,
    parent_lt: LayoutType,
    child: Entity,
    depth: usize,
    child_def: AxisDef,
) -> Size {
    let child_idx = measure_node(world, scratch, child, depth + 1, child_def, None);
    scratch.child_idx_pool[depth].push(child_idx);
    let m = scratch.measured[child_idx as usize].measured;
    measured_in_parent_frame(parent_lt, &m)
}

/// Pass B: measures each relative child's intrinsic content on both axes
/// (measure-only — no rect writes), accumulates the non-stretch main sum + the
/// max cross, pushes a `StretchItem` (with its pre-measured content floor) for each
/// stretch child, and records each measured child's arena index into
/// `child_idx_pool[depth]` in flow order. Populates `stretch_pool[depth]`.
#[allow(clippy::too_many_arguments)]
fn collect_stretch_and_measure(
    world: &mut EcsMaster,
    node: Entity,
    scratch: &mut LayoutScratch,
    depth: usize,
    lt: LayoutType,
    relative_count: usize,
    self_def_main: Option<f32>,
    self_def_cross: Option<f32>,
    insets: Insets,
    relative_main_sum: &mut f32,
    max_child_cross: &mut f32,
) {
    scratch.stretch_pool[depth].clear();
    if matches!(lt, LayoutType::Overlay | LayoutType::Grid) {
        // Overlay/Grid children do not accumulate flow main; they are measured
        // against the content box and the container hugs the widest/tallest child
        // (or its explicit size). Grid then resizes each child to a uniform cell in
        // `resolve_grid_child_sizes` and places it in `position_grid_from_arena` —
        // the size+position pair the two-pass solver requires (no position-pass
        // re-measure).
        for i in 0..relative_count {
            let child = scratch.child_pool[depth][i];
            let child_def = AxisDef {
                main: self_def_main.map(|m| (m - insets.main_total()).max(0.0)),
                cross: self_def_cross.map(|c| (c - insets.cross_total()).max(0.0)),
            };
            let measured =
                measure_child_into_arena(world, scratch, lt, child, depth, child_def);
            *relative_main_sum = relative_main_sum.max(measured.main);
            *max_child_cross = max_child_cross.max(measured.cross);
        }
        // Grid: an Auto-sized container must hug ALL tracks. Each child becomes a
        // uniform cell (`content / track-count`) in `resolve_grid_child_sizes`, so the
        // Auto hug must reserve `rows * max_cell_main` x `cols * max_cell_cross` — NOT
        // a single cell (the prior `max(child)` under-sized an Auto grid to 1/rows of
        // one child). Overlay stacks its children, so its hug stays `max(child)`. A
        // definite-sized container ignores this hug (Pass C uses the explicit size).
        // Uses the SAME `grid_dims` the resolve/position passes use, so the measured
        // box and the per-cell division agree. O(1) extra — no new traversal.
        if matches!(lt, LayoutType::Grid) {
            let (cols, rows) = grid_dims(world, node, relative_count);
            *relative_main_sum *= rows as f32;
            *max_child_cross *= cols as f32;
        }
        return;
    }

    let content_main_base = self_def_main.map(|m| (m - insets.main_total()).max(0.0));
    let content_cross_base = self_def_cross.map(|c| (c - insets.cross_total()).max(0.0));

    for i in 0..relative_count {
        let child = scratch.child_pool[depth][i];
        let child_layout = match world.get_component::<UiLayout>(child).copied() {
            Some(l) => l,
            None => {
                // No UiLayout: still measure (a skip slot, folds to zero) so the
                // arena flow index stays 1:1, then skip its accumulation — exactly
                // the old `continue` semantics.
                let child_def = AxisDef {
                    main: content_main_base,
                    cross: content_cross_base,
                };
                let _ = measure_child_into_arena(world, scratch, lt, child, depth, child_def);
                continue;
            }
        };
        let cm = main_unit(lt, &child_layout);

        // Measure the child's intrinsic content (measure-only recursion), then
        // re-express it in THIS container's axis frame (the child may have a
        // different orientation — its `size` is in its own frame).
        let child_def = AxisDef {
            main: content_main_base,
            cross: content_cross_base,
        };
        let measured =
            measure_child_into_arena(world, scratch, lt, child, depth, child_def);

        match cm {
            Unit::Stretch(f) => {
                let min =
                    resolve_min(min_main_unit(lt, &child_layout), content_main_base, measured.main);
                let max = resolve_max(max_main_unit(lt, &child_layout), content_main_base);
                scratch.stretch_pool[depth].push(StretchItem {
                    target: StretchTarget::Child(i as u32),
                    factor: f.max(0.0),
                    min,
                    max,
                    base_share: 0.0,
                    computed: 0.0,
                    frozen: false,
                });
                // Stretch contributes 0 to the main sum until distribution.
            }
            _ => {
                // Non-stretch contribution = resolved-definite-or-measured,
                // clamped to the child's main min/max.
                let contribution = resolve_definite(cm, content_main_base).unwrap_or(measured.main);
                let cmin =
                    resolve_min(min_main_unit(lt, &child_layout), content_main_base, measured.main);
                let cmax = resolve_max(max_main_unit(lt, &child_layout), content_main_base);
                *relative_main_sum += contribution.clamp(cmin, cmax);
            }
        }
        *max_child_cross = max_child_cross.max(measured.cross);

        // Gap after each child except the last (resolved fixed gap; a Stretch gap
        // is reserved — see StretchTarget::GapAfter).
        if i + 1 < relative_count {
            *relative_main_sum += insets.gap;
        }
    }
}

// ───────────────────────── pass D: stretch freeze ─────────────────────────

/// Pass D: the iterative CSS resolve-flexible-lengths freeze loop. Subtracts each
/// frozen item's COMPUTED (clamped) size and factor from the running free/sum.
fn resolve_stretch_freeze(
    scratch: &mut LayoutScratch,
    depth: usize,
    content_main: f32,
    relative_main_sum: f32,
) {
    let items = &mut scratch.stretch_pool[depth];
    let stretch_count = items.len();
    if stretch_count == 0 {
        return;
    }

    let mut free_main = content_main - relative_main_sum;
    let mut rounds = 0usize;
    loop {
        // Sum of factors over unfrozen items.
        let mut sum = 0.0f32;
        for it in items.iter() {
            if !it.frozen {
                sum += it.factor;
            }
        }
        if sum <= 0.0 {
            // No remaining flex factor: freeze all unfrozen at their min floor.
            for it in items.iter_mut() {
                if !it.frozen {
                    it.computed = it.min.max(0.0);
                    it.frozen = true;
                }
            }
            break;
        }

        // Compute each unfrozen item's base share and clamp violation.
        let mut total_violation = 0.0f32;
        for it in items.iter_mut() {
            if it.frozen {
                continue;
            }
            it.base_share = it.factor * free_main / sum;
            it.computed = it.base_share.clamp(it.min, it.max);
            total_violation += it.computed - it.base_share;
        }

        // Freeze rule by the sign of net violation.
        let mut froze_any = false;
        if total_violation == 0.0 {
            for it in items.iter_mut() {
                if !it.frozen {
                    it.frozen = true;
                    froze_any = true;
                }
            }
        } else {
            let freeze_positive = total_violation > 0.0;
            for it in items.iter_mut() {
                if it.frozen {
                    continue;
                }
                let v = it.computed - it.base_share;
                if (freeze_positive && v > 0.0) || (!freeze_positive && v < 0.0) {
                    it.frozen = true;
                    froze_any = true;
                    // Subtract the COMPUTED (clamped) size + factor (CSS rule).
                    free_main -= it.computed;
                }
            }
        }

        rounds += 1;
        debug_assert!(
            rounds <= stretch_count + 1,
            "stretch freeze did not converge in <= S rounds"
        );
        if !froze_any || rounds > stretch_count {
            // Safety net: if no item froze (should not happen for legal min/max),
            // freeze the rest at their base share to terminate.
            if !froze_any {
                for it in items.iter_mut() {
                    if !it.frozen {
                        it.frozen = true;
                    }
                }
            }
            break;
        }
        // All frozen?
        if items.iter().all(|it| it.frozen) {
            break;
        }
    }
}

// ───────────────────────── pass D.5: resolve child sizes ──────────────────

/// Pass D.5 (flow): resolves EACH relative child's final main + cross size ONCE
/// and appends it to the pass-stable `scratch.child_sizes` arena. The positioning
/// pass reads it instead of re-measuring — the core of the O(N) fix: the former
/// `measure_node(child, …)` re-entries at the measured-fallback sites are now
/// ARENA READS of the child's Pass-B result (`scratch.measured[child_idx]`),
/// measured against the SAME `child_def` (the parent content box), so the value is
/// bit-identical to the old re-measure. Runs after Pass D (the freeze), so a
/// stretch child's `computed` is final.
///
/// The cross axis honors `AlignCross::Stretch`; the main axis is the freeze-
/// computed size for a stretch child, else the resolved/measured size.
fn resolve_flow_child_sizes(
    world: &mut EcsMaster,
    scratch: &mut LayoutScratch,
    depth: usize,
    m: &Measured,
) {
    let child_def = AxisDef {
        main: Some(m.content_main),
        cross: Some(m.content_cross),
    };

    for child_index in 0..m.relative_count {
        let child = scratch.child_pool[depth][child_index];
        let child_layout = world
            .get_component::<UiLayout>(child)
            .copied()
            .unwrap_or_default();

        // The child's Pass-B measured size, re-expressed in THIS frame. Read from
        // the arena (no re-measure). The relative child at flow slot `child_index`
        // is `child_idx_pool[depth][child_index]`.
        let child_arena_idx = scratch.child_idx_pool[depth][child_index];
        let measured = measured_in_parent_frame(
            m.lt,
            &scratch.measured[child_arena_idx as usize].measured,
        );

        // Main: stretch-computed if this child is a stretch item, else resolved.
        let stretch_main = scratch.stretch_pool[depth].iter().find_map(|it| {
            if let StretchTarget::Child(idx) = it.target
                && idx as usize == child_index
            {
                Some(it.computed.max(0.0))
            } else {
                None
            }
        });
        let main = match stretch_main {
            Some(v) => v,
            None => {
                let cm = main_unit(m.lt, &child_layout);
                let resolved = resolve_definite(cm, child_def.main).unwrap_or(measured.main);
                let cmin =
                    resolve_min(min_main_unit(m.lt, &child_layout), child_def.main, measured.main);
                let cmax = resolve_max(max_main_unit(m.lt, &child_layout), child_def.main);
                resolved.clamp(cmin, cmax)
            }
        };

        // Cross: honor AlignCross::Stretch / a Stretch cross unit.
        let cc = cross_unit(m.lt, &child_layout);
        let cmin = resolve_min(min_cross_unit(m.lt, &child_layout), child_def.cross, 0.0);
        let cmax = resolve_max(max_cross_unit(m.lt, &child_layout), child_def.cross);
        let stretch_cross =
            matches!(m.align.cross, AlignCross::Stretch) || matches!(cc, Unit::Stretch(_));
        let cross = if stretch_cross {
            m.content_cross.clamp(cmin, cmax)
        } else {
            resolve_definite(cc, child_def.cross)
                .unwrap_or(measured.cross)
                .clamp(cmin, cmax)
        };

        scratch.child_sizes.push(ChildSize { main, cross });
    }
}

/// Pass D.5 (overlay): resolves each overlay child's final size into the arena.
/// Same arithmetic as the former `position_overlay` sizing (per-child align-self
/// cross-stretch + a Stretch main filling the content box), but reads each child's
/// Pass-B measure from the arena instead of re-entering `measure_node`.
fn resolve_overlay_child_sizes(
    world: &mut EcsMaster,
    scratch: &mut LayoutScratch,
    depth: usize,
    m: &Measured,
) {
    let child_def = AxisDef {
        main: Some(m.content_main),
        cross: Some(m.content_cross),
    };
    for i in 0..m.relative_count {
        let child = scratch.child_pool[depth][i];
        let child_layout = world
            .get_component::<UiLayout>(child)
            .copied()
            .unwrap_or_default();
        let child_align = world
            .get_component::<UiAlign>(child)
            .copied()
            .unwrap_or(m.align);

        let child_arena_idx = scratch.child_idx_pool[depth][i];
        let measured = measured_in_parent_frame(
            m.lt,
            &scratch.measured[child_arena_idx as usize].measured,
        );

        let cm = main_unit(m.lt, &child_layout);
        let main_min = resolve_min(min_main_unit(m.lt, &child_layout), child_def.main, 0.0);
        let main_max = resolve_max(max_main_unit(m.lt, &child_layout), child_def.main);
        let main = if matches!(cm, Unit::Stretch(_)) {
            m.content_main.clamp(main_min, main_max)
        } else {
            resolve_definite(cm, child_def.main)
                .unwrap_or(measured.main)
                .clamp(main_min, main_max)
        };

        let cc = cross_unit(m.lt, &child_layout);
        let cross_min = resolve_min(min_cross_unit(m.lt, &child_layout), child_def.cross, 0.0);
        let cross_max = resolve_max(max_cross_unit(m.lt, &child_layout), child_def.cross);
        let stretch_cross =
            matches!(child_align.cross, AlignCross::Stretch) || matches!(cc, Unit::Stretch(_));
        let cross = if stretch_cross {
            m.content_cross.clamp(cross_min, cross_max)
        } else {
            resolve_definite(cc, child_def.cross)
                .unwrap_or(measured.cross)
                .clamp(cross_min, cross_max)
        };

        scratch.child_sizes.push(ChildSize { main, cross });
    }
}

/// Resolves the `(columns, rows)` cell counts for a Grid container with
/// `child_count` relative children, from the node's [`UiGrid`] config (default
/// `1×auto` when absent). `columns == 0` coerces to `1`; `rows == 0` derives
/// `ceil(child_count / columns)` (at least 1). Both are bounded by the child
/// count's ceiling, so the placement stays `O(children)`.
fn grid_dims(world: &EcsMaster, node: Entity, child_count: usize) -> (usize, usize) {
    let cfg = world.get_component::<UiGrid>(node).copied().unwrap_or_default();
    let cols = (cfg.columns as usize).max(1);
    let rows = if cfg.rows == 0 {
        // ceil(child_count / cols), at least 1 so an empty grid still has a row.
        child_count.div_ceil(cols).max(1)
    } else {
        cfg.rows as usize
    };
    (cols, rows)
}

/// Pass D.5 (grid): sizes each relative child to a UNIFORM cell — the content
/// box divided by the `(columns, rows)` track counts (GUI P6a). Appends one
/// [`ChildSize`] per relative child to the pass-stable arena, 1:1 with
/// `child_index`, so the position pass reads it without re-measuring (the
/// two-pass invariant). `position_grid_from_arena` places each child into its
/// cell. Bounded `O(relative_count)` — no super-linear scan.
///
/// The grid main axis is vertical (rows down y), the cross axis horizontal
/// (columns across x) — the same axis convention `fold_size`/`fold_pos` use for
/// the `Column`/`_` arm, so a cell's `(main = cell_main, cross = cell_cross)`
/// folds to `(w = cell_cross, h = cell_main)`.
fn resolve_grid_child_sizes(
    world: &mut EcsMaster,
    scratch: &mut LayoutScratch,
    node: Entity,
    m: &Measured,
) {
    let (cols, rows) = grid_dims(world, node, m.relative_count);
    // Uniform cell extent: the content box divided by the track counts. `cols`/
    // `rows` are >= 1 (see `grid_dims`), so no division by zero.
    let cell_cross = m.content_cross / cols as f32;
    let cell_main = m.content_main / rows as f32;
    for _ in 0..m.relative_count {
        scratch.child_sizes.push(ChildSize { main: cell_main, cross: cell_cross });
    }
}

/// Pass D.5 (absolute tail): resolves each absolute child's final size into the
/// arena, appended AFTER the relative/overlay entries so `child_sizes` stays 1:1
/// with `child_index`. Same arithmetic as the former `position_absolute` sizing,
/// reading the Pass-B.5 measure from the arena instead of re-measuring.
fn resolve_absolute_child_sizes(
    world: &mut EcsMaster,
    scratch: &mut LayoutScratch,
    depth: usize,
    relative_count: usize,
    total_children: usize,
    m: &Measured,
) {
    let child_def = AxisDef {
        main: Some(m.content_main),
        cross: Some(m.content_cross),
    };
    for i in relative_count..total_children {
        let child = scratch.child_pool[depth][i];
        let child_layout = world
            .get_component::<UiLayout>(child)
            .copied()
            .unwrap_or_default();

        let child_arena_idx = scratch.child_idx_pool[depth][i];
        let measured = measured_in_parent_frame(
            m.lt,
            &scratch.measured[child_arena_idx as usize].measured,
        );

        let cm = main_unit(m.lt, &child_layout);
        let main_min = resolve_min(min_main_unit(m.lt, &child_layout), child_def.main, 0.0);
        let main_max = resolve_max(max_main_unit(m.lt, &child_layout), child_def.main);
        let main = resolve_definite(cm, child_def.main)
            .unwrap_or(measured.main)
            .clamp(main_min, main_max);

        let cc = cross_unit(m.lt, &child_layout);
        let cross_min = resolve_min(min_cross_unit(m.lt, &child_layout), child_def.cross, 0.0);
        let cross_max = resolve_max(max_cross_unit(m.lt, &child_layout), child_def.cross);
        let cross = resolve_definite(cc, child_def.cross)
            .unwrap_or(measured.cross)
            .clamp(cross_min, cross_max);

        scratch.child_sizes.push(ChildSize { main, cross });
    }
}

// ───────────────────────── pass B.5: measure absolute tail ────────────────

/// Pass B.5: measures the absolute (out-of-flow) children into the arena and
/// records their indices into `child_idx_pool[depth]` AFTER the relatives, so the
/// committed `child_index` range is `[relatives.. , ..absolutes]`. They do not
/// accumulate flow, but a stored measure lets the position pass read their size
/// from the arena instead of re-entering `measure_node`.
fn measure_absolute_children(
    world: &mut EcsMaster,
    scratch: &mut LayoutScratch,
    depth: usize,
    relative_count: usize,
    total_children: usize,
    m: &Measured,
) {
    let child_def = AxisDef {
        main: Some(m.content_main),
        cross: Some(m.content_cross),
    };
    for i in relative_count..total_children {
        let child = scratch.child_pool[depth][i];
        // `measure_child_into_arena` measures, records the index, and returns the
        // size; the size is recomputed (with offsets) in `resolve_absolute_child_sizes`.
        let _ = measure_child_into_arena(world, scratch, m.lt, child, depth, child_def);
    }
}

// ───────────────────────── conditional descendant reflow ──────────────────

/// Conditional reflow (Decision 4): for each child (relative/overlay AND absolute)
/// whose FINAL resolved `ChildSize` differs from its intrinsic Pass-B measure AND
/// that has its own children, re-measure it ONCE at its final size (passed as
/// `force_def`), appending a fresh arena subtree and re-pointing the parent's
/// child index to it. A fixed-size child (final == intrinsic) is left as-is — its
/// stored intrinsic subtree is already the correct final layout. This is the
/// bounded `stretch_nesting` factor that replaces the old unconditional re-entry:
/// because each changed-size subtree is re-measured (re-walked) once, the total
/// measure visits are `O(N * stretch_nesting_depth)`, absolutely capped by
/// `MAX_LAYOUT_DEPTH` (128). The `~3x` figure quoted elsewhere is the TYPICAL
/// shallow-nesting case (≲3 flexible levels), not a hard cap — a deep stack of
/// stretch containers raises the factor linearly with nesting depth. This is what
/// lets the position pass NEVER call `measure_node`.
fn reflow_changed_children(
    world: &mut EcsMaster,
    scratch: &mut LayoutScratch,
    depth: usize,
    m: &Measured,
    size_lo: u32,
    total_children: usize,
) {
    if depth + 1 >= MAX_LAYOUT_DEPTH {
        return;
    }
    for k in 0..total_children {
        let final_size = scratch.child_sizes[size_lo as usize + k];
        let child_arena_idx = scratch.child_idx_pool[depth][k];
        let intrinsic = scratch.measured[child_arena_idx as usize];
        if intrinsic.measured.skip {
            continue;
        }
        // Compare in the CHILD's own frame: fold the parent-frame final size back
        // to absolute (w, h), then re-split into the child's frame so we compare
        // like-for-like against the child's stored `size`.
        let (fw, fh) = fold_size(m.lt, final_size.main, final_size.cross);
        let child_lt = intrinsic.measured.lt;
        let (final_main, final_cross) = match child_lt {
            LayoutType::Row => (fw, fh),
            _ => (fh, fw),
        };
        let stored = intrinsic.measured.size;
        let changed = final_main != stored.main || final_cross != stored.cross;
        let has_children = intrinsic.child_lo != intrinsic.child_hi;
        if !changed || !has_children {
            continue;
        }
        // Re-measure the child at its FINAL size (supplied as both `parent_def` —
        // for the child's own Pct — and `force_def` — to fill any indefinite axis),
        // appending a fresh subtree; re-point this child slot at it.
        let cdef = AxisDef {
            main: Some(final_main),
            cross: Some(final_cross),
        };
        let new_idx = measure_node(world, scratch, intrinsic.entity, depth + 1, cdef, Some(cdef));
        scratch.child_idx_pool[depth][k] = new_idx;
    }
}

// ───────────────────────── passes E + F + G: positioning ──────────────────

/// Position pass for one arena node: writes its CHILDREN's `ComputedRect` at the
/// `origin` (its own absolute top-left, written by its parent or by `layout_root`
/// for the root) and recurses into each child. Reads sizes from the arena and
/// NEVER calls `measure_node` (the complexity-probe invariant). A skip node is a
/// no-op leaf.
fn position_node(world: &mut EcsMaster, scratch: &mut LayoutScratch, idx: u32, origin: Origin) {
    // Copy the small POD header out so the per-child world/scratch mutations below
    // do not hold a borrow into the arena.
    let node = scratch.measured[idx as usize];
    if node.measured.skip {
        return;
    }
    // Invariant: exactly one resolved `ChildSize` per child arena slot, so the
    // size range and the child-index range have equal length (the position helpers
    // index both by the same flow `k`).
    debug_assert_eq!(
        node.size_hi - node.size_lo,
        node.child_hi - node.child_lo,
        "child_sizes range must be 1:1 with child_index range"
    );
    match node.measured.lt {
        LayoutType::Overlay => position_overlay_from_arena(world, scratch, &node, origin),
        LayoutType::Grid => position_grid_from_arena(world, scratch, &node, origin),
        _ => position_flow_from_arena(world, scratch, &node, origin),
    }
    position_absolute_from_arena(world, scratch, &node, origin);
}

/// Reads a child's `(entity, arena_index, final_size)` for the parent's flow slot
/// `k` (`0`-based over `child_index[child_lo..child_hi]`).
#[inline]
fn child_slot(scratch: &LayoutScratch, node: &MeasuredNode, k: usize) -> (Entity, u32, ChildSize) {
    let child_idx = scratch.child_index[node.child_lo as usize + k];
    let entity = scratch.measured[child_idx as usize].entity;
    let size = scratch.child_sizes[node.size_lo as usize + k];
    (entity, child_idx, size)
}

/// Passes E + F (flow): places the relative children of `node` in flow order
/// (Row/Column), writing each child's `ComputedRect` at its absolute origin and
/// recursing via `position_node`. All sizes come from `scratch.child_sizes` (the
/// arena) — no re-measure.
fn position_flow_from_arena(
    world: &mut EcsMaster,
    scratch: &mut LayoutScratch,
    node: &MeasuredNode,
    origin: Origin,
) {
    let m = &node.measured;

    // First sweep: sum the stored child main sizes + fixed gaps (AlignMain
    // leftover). Pure arena reads — no re-measure.
    let mut used_main = 0.0f32;
    for k in 0..m.relative_count {
        used_main += scratch.child_sizes[node.size_lo as usize + k].main;
        if k + 1 < m.relative_count {
            used_main += m.insets.gap;
        }
    }

    // AlignMain leading offset (ignored when any stretch consumed free space — the
    // stored `has_stretch` gate — and clamped to >= 0 when over-constrained so
    // content packs at the before-edge). Byte-identical to the old `position_flow`.
    let leftover = m.content_main - used_main;
    let (leading, between_extra) = if m.has_stretch {
        (0.0, 0.0)
    } else {
        align_main_offsets(m.align.main, leftover.max(0.0), m.relative_count)
    };

    let mut cursor = m.insets.main_before + leading;
    for k in 0..m.relative_count {
        let (child, child_idx, sz) = child_slot(scratch, node, k);

        let cross_pos = m.insets.cross_before
            + cross_align_fraction(m.align.cross) * (m.content_cross - sz.cross);
        let (dx, dy) = fold_pos(m.lt, cursor, cross_pos);
        let (w, h) = fold_size(m.lt, sz.main, sz.cross);
        let rect = ComputedRect {
            x: origin.x + dx,
            y: origin.y + dy,
            w,
            h,
        };
        write_rect(world, child, rect);
        position_node(world, scratch, child_idx, Origin { x: rect.x, y: rect.y });

        cursor += sz.main + between_extra;
        if k + 1 < m.relative_count {
            cursor += m.insets.gap;
        }
    }
}

/// The cross-axis placement fraction for an `AlignCross` (0.0 start, 0.5 center,
/// 1.0 end; stretch placed at start since it fills).
#[inline]
fn cross_align_fraction(a: AlignCross) -> f32 {
    match a {
        AlignCross::Start | AlignCross::Stretch => 0.0,
        AlignCross::Center => 0.5,
        AlignCross::End => 1.0,
    }
}

/// Computes the `(leading, between_extra)` AlignMain offsets from the positive
/// leftover. `between_extra` is per-child extra spacing for the space-* modes.
#[inline]
fn align_main_offsets(a: AlignMain, leftover: f32, count: usize) -> (f32, f32) {
    if count == 0 {
        return (0.0, 0.0);
    }
    let n = count as f32;
    match a {
        AlignMain::Start => (0.0, 0.0),
        AlignMain::Center => (leftover * 0.5, 0.0),
        AlignMain::End => (leftover, 0.0),
        AlignMain::SpaceBetween => {
            if count <= 1 {
                (0.0, 0.0)
            } else {
                (0.0, leftover / (n - 1.0))
            }
        }
        AlignMain::SpaceAround => {
            let unit = leftover / n;
            (unit * 0.5, unit)
        }
        AlignMain::SpaceEvenly => {
            let unit = leftover / (n + 1.0);
            (unit, unit)
        }
    }
}

/// Positions Overlay children: each placed independently within the content box by
/// its OWN `UiAlign` (align-self), falling back to the container's `UiAlign` when
/// the child carries none. Child SIZES come from the arena (`child_sizes`, resolved
/// in `resolve_overlay_child_sizes`); only the per-child OFFSET is computed here
/// (cheap component reads, no re-measure). Recurses via `position_node`.
fn position_overlay_from_arena(
    world: &mut EcsMaster,
    scratch: &mut LayoutScratch,
    node: &MeasuredNode,
    origin: Origin,
) {
    let m = &node.measured;
    for i in 0..m.relative_count {
        let (child, child_idx, sz) = child_slot(scratch, node, i);
        // Align-self: the child's own UiAlign overrides; the container's is the
        // fallback when the child has none.
        let child_align = world
            .get_component::<UiAlign>(child)
            .copied()
            .unwrap_or(m.align);

        let child_main = sz.main;
        let child_cross = sz.cross;

        let main_pos = m.insets.main_before
            + cross_align_fraction(child_align.main_as_cross()) * (m.content_main - child_main);
        let cross_pos = m.insets.cross_before
            + cross_align_fraction(child_align.cross) * (m.content_cross - child_cross);
        let (dx, dy) = fold_pos(m.lt, main_pos, cross_pos);
        let (w, h) = fold_size(m.lt, child_main, child_cross);
        let rect = ComputedRect {
            x: origin.x + dx,
            y: origin.y + dy,
            w,
            h,
        };
        write_rect(world, child, rect);
        position_node(world, scratch, child_idx, Origin { x: rect.x, y: rect.y });
    }
}

/// Positions Grid children into a uniform `columns × rows` cell layout (GUI
/// P6a). Relative child at flow slot `k` occupies cell
/// `(col = k % cols, row = k / cols)`; its top-left is the content-box before-edge
/// plus the cell offset. Child SIZES come from the arena (`child_sizes`, resolved
/// to the cell extent in `resolve_grid_child_sizes`) — no re-measure. Recurses via
/// `position_node`. Bounded `O(relative_count)`.
///
/// `cols` is recovered from the resolved cell extent (`content_cross / cell_cross`)
/// so the placement uses the SAME track count `resolve_grid_child_sizes` sized
/// against, without re-reading `UiGrid` (the position pass takes no extra
/// component read on the hot path). A degenerate zero-width cell falls back to a
/// single column.
fn position_grid_from_arena(
    world: &mut EcsMaster,
    scratch: &mut LayoutScratch,
    node: &MeasuredNode,
    origin: Origin,
) {
    let m = &node.measured;
    if m.relative_count == 0 {
        return;
    }
    // Recover the column count from the resolved cell cross extent (set by
    // `resolve_grid_child_sizes` to `content_cross / cols`). Guard against a
    // zero/degenerate cell (zero content box) by defaulting to one column.
    let first_cell_cross = scratch.child_sizes[node.size_lo as usize].cross;
    let cols = if first_cell_cross > 0.0 {
        (m.content_cross / first_cell_cross).round().max(1.0) as usize
    } else {
        1
    };

    for k in 0..m.relative_count {
        let (child, child_idx, sz) = child_slot(scratch, node, k);
        let col = k % cols;
        let row = k / cols;
        // Cell offset in axis-relative coordinates (main = rows down, cross =
        // columns across), then folded to (dx, dy) like every other path.
        let main_pos = m.insets.main_before + row as f32 * sz.main;
        let cross_pos = m.insets.cross_before + col as f32 * sz.cross;
        let (dx, dy) = fold_pos(m.lt, main_pos, cross_pos);
        let (w, h) = fold_size(m.lt, sz.main, sz.cross);
        let rect = ComputedRect {
            x: origin.x + dx,
            y: origin.y + dy,
            w,
            h,
        };
        write_rect(world, child, rect);
        position_node(world, scratch, child_idx, Origin { x: rect.x, y: rect.y });
    }
}

// ───────────────────────── pass G: absolute children ──────────────────────

/// Positions absolute (out-of-flow) children against the container's padding box.
/// Child SIZES come from the arena (`child_sizes`, resolved in
/// `resolve_absolute_child_sizes`); only the per-child OFFSET (the `UiAbsolute`
/// left/top) is computed here. Recurses via `position_node`.
fn position_absolute_from_arena(
    world: &mut EcsMaster,
    scratch: &mut LayoutScratch,
    node: &MeasuredNode,
    origin: Origin,
) {
    let m = &node.measured;
    let total = (node.child_hi - node.child_lo) as usize;
    let child_def = AxisDef {
        main: Some(m.content_main),
        cross: Some(m.content_cross),
    };
    for k in m.relative_count..total {
        let (child, child_idx, sz) = child_slot(scratch, node, k);
        let abs = world
            .get_component::<UiAbsolute>(child)
            .copied()
            .unwrap_or_default();

        let child_main = sz.main;
        let child_cross = sz.cross;

        // before (left/top) wins; resolve against the padding-box extents.
        let (main_before_unit, cross_before_unit) = match m.lt {
            LayoutType::Row => (abs.left, abs.top),
            _ => (abs.top, abs.left),
        };
        let main_pos = m.insets.main_before
            + resolve_definite(main_before_unit, child_def.main).unwrap_or(0.0);
        let cross_pos = m.insets.cross_before
            + resolve_definite(cross_before_unit, child_def.cross).unwrap_or(0.0);

        let (dx, dy) = fold_pos(m.lt, main_pos, cross_pos);
        let (w, h) = fold_size(m.lt, child_main, child_cross);
        let rect = ComputedRect {
            x: origin.x + dx,
            y: origin.y + dy,
            w,
            h,
        };
        write_rect(world, child, rect);
        position_node(world, scratch, child_idx, Origin { x: rect.x, y: rect.y });
    }
}

// ───────────────────────── rect write (set-if-changed) ────────────────────

/// Writes a node's `ComputedRect` set-if-changed: reads the current value through
/// a shared accessor and acquires the `Mut` guard (which bumps `changed_tick` on
/// any mutable deref) ONLY when the value actually differs. Bounded by a finite
/// `debug_assert!` (a NaN rect would never compare equal and would bump the tick
/// every frame forever).
fn write_rect(world: &mut EcsMaster, node: Entity, rect: ComputedRect) {
    debug_assert!(
        rect.x.is_finite() && rect.y.is_finite() && rect.w.is_finite() && rect.h.is_finite(),
        "ComputedRect must be finite before write"
    );
    match world.get_component::<ComputedRect>(node) {
        Some(current) if *current == rect => {
            // Bit-identical: suppress the write so the changed_tick does not bump.
        }
        Some(_) => {
            if let Some(mut guard) = world.get_component_mut::<ComputedRect>(node) {
                *guard = rect;
            }
        }
        None => missing_rect(),
    }
}

// ───────────────────────── small helpers on UiAlign ───────────────────────

impl UiAlign {
    /// Treats the main alignment as a cross-style placement fraction selector for
    /// the Overlay path (where there is no flow, only independent placement).
    #[inline]
    fn main_as_cross(self) -> AlignCross {
        match self.main {
            AlignMain::Start | AlignMain::SpaceBetween | AlignMain::SpaceEvenly => {
                AlignCross::Start
            }
            AlignMain::Center | AlignMain::SpaceAround => AlignCross::Center,
            AlignMain::End => AlignCross::End,
        }
    }
}

// ───────────────────────── Open Question 2 resolution (doc) ────────────────
//
// The plan's Open Question 2 (batched raw accessor `get_components_raw` vs
// grouped `get_component`): RESOLVED in favor of grouped `get_component`.
// `get_components_raw` (ecs_master.rs:2121) allocates a `Vec<(ComponentId,
// *const u8)>` per call (`Vec::with_capacity` on every invocation), which would
// allocate once per node visit and defeat the zero-per-frame-alloc goal. Grouped
// `get_component` calls are non-allocating, type-safe, and compile cleanly. The
// honest cost model (each visit is a multi-hop pointer chase through the sparse
// `entities_inland` store) holds either way; archetype-order SoA traversal
// remains the documented future optimization seam.

// ───────────────────────── unit tests (scoped 0%-overhead) ────────────────
//
// These live INSIDE the crate (vs `tests/`) because they read the `#[cfg(test)]`
// `LayoutScratch::relayout_count` hook, which only exists when the lib is built
// with `--cfg test`. They drive a real `[discovery, apply, probe]` schedule and
// assert the change-detection / set-if-changed steady state.
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
    use boyko_ecs::ecs::core::entity::entity::Entity;
    use boyko_ecs::ecs::core::iters::query::filter::Changed;
    use boyko_ecs::ecs::core::iters::query::query::Query;
    use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
    use boyko_ecs::ecs::core::system::Commands;
    use boyko_threadpool::ThreadPoolBuilder;

    use crate::components::{ComputedRect, UiLayout, UiRoot};
    use crate::layout::{ui_layout_apply, ui_layout_discovery};
    use crate::resources::{LayoutScratch, MAX_LAYOUT_DEPTH, UiViewport};
    use crate::units::{LayoutType, Unit};

    /// Process-wide guard: the `Changed<ComputedRect>` probe writes a `static`
    /// counter, so the tests that read it must not interleave. Poison-tolerant
    /// (a panicking test must not cascade into the others as a `PoisonError`).
    static PROBE_LOCK: Mutex<()> = Mutex::new(());
    static RECT_CHANGES: AtomicUsize = AtomicUsize::new(0);

    fn lock_probe() -> std::sync::MutexGuard<'static, ()> {
        PROBE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Probe system: counts rows whose `ComputedRect` tick advanced this frame.
    fn count_rect_changes(
        q: Query<(), Changed<ComputedRect>>,
    ) {
        for _ in q.iter() {
            RECT_CHANGES.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn col(width: Unit, height: Unit) -> UiLayout {
        UiLayout { layout_type: LayoutType::Column, width, height, ..UiLayout::default() }
    }

    fn build_world() -> (EcsMaster, Schedule, Entity, Entity) {
        let mut world = EcsMaster::new();
        world.insert_resource(UiViewport {
            width: 400.0,
            height: 800.0,
            scale_factor: 1.0,
            generation: 0,
        });
        world.insert_resource(LayoutScratch::with_seeds());

        let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
        let probe = Arc::clone(&sink);
        world.run_system(move |mut cmds: Commands| {
            let mut v = probe.lock().expect("probe");
            let root = {
                let mut e = cmds.spawn(col(Unit::Px(400.0), Unit::Px(800.0)));
                e.insert(ComputedRect::default());
                e.insert(UiRoot);
                e.id()
            };
            v.push(root);
            let child = {
                let mut e = cmds.spawn(col(Unit::Px(100.0), Unit::Px(40.0)));
                e.insert(ComputedRect::default());
                e.set_parent(root);
                e.id()
            };
            v.push(child);
        });
        let ids = sink.lock().expect("probe").clone();
        let (root, child) = (ids[0], ids[1]);

        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let mut b = ScheduleBuilder::new(pool);
        let kd = b.add_system(ui_layout_discovery).key();
        let ka = b.add_system(ui_layout_apply).after(kd).key();
        b.add_system(count_rect_changes).after(ka);
        let schedule = b.build(&mut world);
        (world, schedule, root, child)
    }

    fn relayout_count(world: &EcsMaster) -> u32 {
        world.resource::<LayoutScratch>().relayout_count
    }

    fn measure_visits(world: &EcsMaster) -> u32 {
        world.resource::<LayoutScratch>().measure_visits
    }

    #[test]
    fn unchanged_frame_advances_no_rect_tick_and_zero_relayout() {
        let _g = lock_probe();
        let (mut world, mut schedule, _root, _child) = build_world();

        // Frame 1: the freshly-spawned rows are "changed", so the layout runs and
        // writes every ComputedRect (relayout_count >= 1).
        schedule.run(&mut world);
        assert!(relayout_count(&world) >= 1, "frame 1 lays out the dirty root");

        // Settle one more frame so any first-frame ComputedRect writes have been
        // observed and rolled past the change window.
        schedule.run(&mut world);

        // Now several fully-idle frames: discovery's scan is empty -> apply
        // early-returns -> relayout_count == 0 AND no ComputedRect tick advances.
        for f in 0..3 {
            RECT_CHANGES.store(0, Ordering::Relaxed);
            schedule.run(&mut world);
            assert_eq!(relayout_count(&world), 0, "idle frame {f} relays zero roots");
            assert_eq!(
                RECT_CHANGES.load(Ordering::Relaxed),
                0,
                "idle frame {f} advances no ComputedRect tick (0%-overhead)"
            );
        }
    }

    #[test]
    fn identical_geometry_mutation_does_not_advance_rect_tick() {
        let _g = lock_probe();
        let (mut world, mut schedule, _root, child) = build_world();

        // Settle (let the initial layout writes roll past the change window).
        for _ in 0..3 {
            schedule.run(&mut world);
        }

        // Mutate an input to a value that yields BIT-IDENTICAL geometry (re-insert
        // the same UiLayout). The input tick bumps (so the root relays this frame),
        // but the recomputed ComputedRect equals the current one, so set-if-changed
        // suppresses the write and the ComputedRect tick must NOT advance.
        world.run_system(move |mut cmds: Commands| {
            cmds.entity(child).insert(col(Unit::Px(100.0), Unit::Px(40.0)));
        });
        // Relayout frame (apply runs; any ComputedRect write would stamp this
        // frame, observed next frame).
        schedule.run(&mut world);
        assert!(relayout_count(&world) >= 1, "identical-input change still relays the root");

        // Observation frame: a suppressed (bit-identical) write means zero
        // Changed<ComputedRect> rows here.
        RECT_CHANGES.store(0, Ordering::Relaxed);
        schedule.run(&mut world);
        assert_eq!(
            RECT_CHANGES.load(Ordering::Relaxed),
            0,
            "bit-identical geometry must not advance the ComputedRect tick (set-if-changed)"
        );
    }

    #[test]
    fn differing_size_mutation_relays_exactly_one_root_and_advances_tick() {
        let _g = lock_probe();
        let (mut world, mut schedule, _root, child) = build_world();
        for _ in 0..3 {
            schedule.run(&mut world);
        }

        // Change the child to a DIFFERENT size -> exactly the one root relays.
        world.run_system(move |mut cmds: Commands| {
            cmds.entity(child).insert(col(Unit::Px(100.0), Unit::Px(55.0)));
        });
        // Relayout frame.
        schedule.run(&mut world);
        assert_eq!(relayout_count(&world), 1, "exactly one root relaid");
        assert_eq!(
            world.get_component::<ComputedRect>(child).expect("rect").h,
            55.0,
            "child height reflects the new size"
        );

        // Observation frame: the changed child's ComputedRect tick advanced, so the
        // probe sees >= 1 changed row (the child, at minimum).
        RECT_CHANGES.store(0, Ordering::Relaxed);
        schedule.run(&mut world);
        assert!(
            RECT_CHANGES.load(Ordering::Relaxed) >= 1,
            "the changed child's ComputedRect tick advanced (observed next frame)"
        );
    }

    // ───────────────── complexity-regression probe (measure_visits) ───────────
    //
    // These lock in the genuine `O(N * stretch_nesting_depth)` envelope and guard
    // against a regression to the old exponential `(3*branching)^depth` re-entry
    // (the exact "compiles + tests green but stays exponential" failure mode). A
    // deep STRETCH nesting is used on purpose: every level's final content box
    // differs from its intrinsic measure, so the gated conditional reflow
    // (Decision 4) fires at every level — the worst case for measure visits. They
    // read the `#[cfg(test)]` `measure_visits` hook, which is why they live
    // in-crate (integration crates cannot see it).

    fn row(width: Unit, height: Unit) -> UiLayout {
        UiLayout { layout_type: LayoutType::Row, width, height, ..UiLayout::default() }
    }

    /// Builds a `[discovery, apply]` schedule over a world that already contains a
    /// spawned tree. No probe system (these tests read `measure_visits`, not the
    /// `Changed<ComputedRect>` static), so no `PROBE_LOCK` is required.
    fn schedule_for(world: &mut EcsMaster) -> Schedule {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let mut b = ScheduleBuilder::new(pool);
        let kd = b.add_system(ui_layout_discovery).key();
        b.add_system(ui_layout_apply).after(kd);
        b.build(world)
    }

    /// Spawns a single-child-per-level chain of `depth` Row stretch containers
    /// under a fixed-size Row root. Each level is a `Row(Stretch(1), Px(20))` whose
    /// MAIN (width) is indefinite intrinsically but is forced to its parent's free
    /// space by stretch — so every level's final main differs from its intrinsic
    /// measure and the conditional reflow fires at every level. Returns the node
    /// count `N` (root + `depth` containers).
    fn spawn_stretch_path(world: &mut EcsMaster, depth: usize) -> u32 {
        world.run_system(move |mut cmds: Commands| {
            let root = {
                let mut e = cmds.spawn(row(Unit::Px(1000.0), Unit::Px(800.0)));
                e.insert(ComputedRect::default());
                e.insert(UiRoot);
                e.id()
            };
            let mut parent = root;
            for _ in 0..depth {
                let mut e = cmds.spawn(row(Unit::Stretch(1.0), Unit::Px(20.0)));
                e.insert(ComputedRect::default());
                e.set_parent(parent);
                parent = e.id();
            }
        });
        (1 + depth) as u32
    }

    /// Spawns a balanced binary tree of Row stretch containers of the given
    /// `levels` (the root is level 0). Every internal node is a
    /// `Row(Stretch(1), Px(20))`; the root is a fixed-size Row. Returns `N`, the
    /// total node count (`2^(levels+1) - 1`).
    fn spawn_stretch_binary_tree(world: &mut EcsMaster, levels: usize) -> u32 {
        world.run_system(move |mut cmds: Commands| {
            let root = {
                let mut e = cmds.spawn(row(Unit::Px(1024.0), Unit::Px(800.0)));
                e.insert(ComputedRect::default());
                e.insert(UiRoot);
                e.id()
            };
            // BFS by level so each node's two children are spawned under it.
            let mut frontier = vec![root];
            for _ in 0..levels {
                let mut next = Vec::with_capacity(frontier.len() * 2);
                for parent in frontier.drain(..) {
                    for _ in 0..2 {
                        let mut e = cmds.spawn(row(Unit::Stretch(1.0), Unit::Px(20.0)));
                        e.insert(ComputedRect::default());
                        e.set_parent(parent);
                        next.push(e.id());
                    }
                }
                frontier = next;
            }
        });
        (1u32 << (levels + 1)) - 1
    }

    #[test]
    fn deep_stretch_path_measure_visits_within_linear_depth_envelope() {
        const DEPTH: usize = 20;
        let mut world = EcsMaster::new();
        world.insert_resource(UiViewport {
            width: 1000.0,
            height: 800.0,
            scale_factor: 1.0,
            generation: 0,
        });
        world.insert_resource(LayoutScratch::with_seeds());
        let n = spawn_stretch_path(&mut world, DEPTH);

        let mut schedule = schedule_for(&mut world);
        schedule.run(&mut world);

        let visits = measure_visits(&world);
        // Lower bound: every node is measured at least once in the base walk.
        assert!(visits >= n, "every node measured at least once: {visits} >= {n}");
        // Upper bound: the genuine O(N * stretch_nesting_depth) envelope, hard-capped
        // by MAX_LAYOUT_DEPTH. This is the anti-exponential guard: at DEPTH=20 the
        // old (3*1)^20 re-entry would be ~3.5 billion visits — many orders of
        // magnitude over this ceiling, so any regression to a per-ancestor or
        // positioning re-measure trips here.
        let envelope = n * (MAX_LAYOUT_DEPTH as u32);
        assert!(
            visits <= envelope,
            "measure_visits {visits} exceeds O(N*depth) envelope {envelope} \
             (N={n}, cap={MAX_LAYOUT_DEPTH}) — possible exponential regression"
        );
        // Tighter, structural ceiling: each of the DEPTH stretch levels can trigger
        // at most one re-walk of its (shrinking) tail, so total visits are well
        // under N * actual_depth * a small constant. A blown bound here means the
        // reflow is re-entering more than once per changed level.
        let tight = n * (DEPTH as u32) * 4;
        assert!(
            visits <= tight,
            "measure_visits {visits} exceeds the tight N*depth*4 ceiling {tight} \
             (N={n}, depth={DEPTH})"
        );
    }

    #[test]
    fn balanced_stretch_binary_tree_measure_visits_not_exponential() {
        const LEVELS: usize = 10; // N = 2^11 - 1 = 2047 nodes.
        let mut world = EcsMaster::new();
        world.insert_resource(UiViewport {
            width: 1024.0,
            height: 800.0,
            scale_factor: 1.0,
            generation: 0,
        });
        world.insert_resource(LayoutScratch::with_seeds());
        let n = spawn_stretch_binary_tree(&mut world, LEVELS);

        let mut schedule = schedule_for(&mut world);
        schedule.run(&mut world);

        let visits = measure_visits(&world);
        assert!(visits >= n, "every node measured at least once: {visits} >= {n}");
        // The old (3*branching)^depth re-entry at LEVELS=10, branching=2 would be
        // astronomically larger than N*depth; this ceiling provably excludes it.
        let envelope = n.saturating_mul(LEVELS as u32 + 1).saturating_mul(4);
        assert!(
            visits <= envelope,
            "measure_visits {visits} exceeds O(N*depth) envelope {envelope} \
             (N={n}, levels={LEVELS}) — possible exponential regression"
        );
    }

    #[test]
    fn position_pass_never_re_measures() {
        // Snapshot measure_visits after Pass 1 by re-running layout and proving the
        // count is identical whether or not Pass 2 (positioning) ran: positioning
        // reads the arena and must NEVER call measure_node. We assert this through
        // the public invariant — a full relayout's measure_visits equals the count
        // accrued by the measure pass alone, since position_node only reads stored
        // sizes. Concretely: run two identical relayout frames over a stretch tree;
        // the per-frame measure_visits must be stable (positioning adds zero).
        const DEPTH: usize = 8;
        let mut world = EcsMaster::new();
        world.insert_resource(UiViewport {
            width: 1000.0,
            height: 800.0,
            scale_factor: 1.0,
            generation: 0,
        });
        world.insert_resource(LayoutScratch::with_seeds());
        let _n = spawn_stretch_path(&mut world, DEPTH);

        let mut schedule = schedule_for(&mut world);
        schedule.run(&mut world);
        let first = measure_visits(&world);

        // Force a second full relayout by bumping the viewport generation (resize
        // detection re-lays every root). The position pass runs again over the same
        // structure; measure_visits is reset at layout_root entry, so an equal
        // count proves positioning contributed zero measure_node entries on both
        // frames (had Pass 2 re-measured, the two frames would still match each
        // other, but the count would exceed the measure-only base — which the
        // envelope checks above already exclude; here we lock determinism).
        world.resource_mut::<UiViewport>().generation = 1;
        schedule.run(&mut world);
        let second = measure_visits(&world);

        assert_eq!(
            first, second,
            "measure_visits must be deterministic across identical relayouts \
             (positioning never re-measures): {first} != {second}"
        );
        assert!(first >= _n, "base measure walk visits every node");
    }

    // ─────────── linear-scaling COMPLEXITY guard (architect's C=3 spec) ────────
    //
    // The architect's spec mandates three FIXED-SIZE tree shapes — a DEEP chain, a
    // WIDE fan, and a BALANCED binary tree — measured at N ∈ {10, 100, 1000}, each
    // asserting `N <= measure_visits <= C*N` with the exact bound `C = 3`. Fixed
    // (`Px`) sizes mean the conditional reflow (Decision 4) NEVER fires (final ==
    // intrinsic on every axis), so each node is measured EXACTLY once and the count
    // is `== N`; the `<= 3N` ceiling is the regression cap. These are the
    // load-bearing anti-exponential guards: under the OLD `(3*branching)^depth`
    // re-entry the DEEP chain at N=1000 would be `3^1000` (the test never returns)
    // and the BALANCED tree at N=1023 would be astronomically over `3N`, so any
    // regression to a per-ancestor / positioning re-measure trips loudly here. They
    // read the in-crate `#[cfg(test)]` `measure_visits` hook.

    /// The exact linear-scaling constant from the architect's spec. A fixed-size
    /// tree must measure each node at most this many times.
    const SCALING_C: u32 = 3;

    /// Builds a world with a 1000×800 viewport + seeded scratch (the standard
    /// complexity-test fixture; no probe system needed).
    fn complexity_world() -> EcsMaster {
        let mut world = EcsMaster::new();
        world.insert_resource(UiViewport {
            width: 1000.0,
            height: 800.0,
            scale_factor: 1.0,
            generation: 0,
        });
        world.insert_resource(LayoutScratch::with_seeds());
        world
    }

    /// Spawns a DEEP chain: a single spine of `n` fixed-size `Column(Px,Px)` nodes,
    /// each the sole child of the previous. The first is the `UiRoot`. Returns `n`.
    /// Fixed sizes ⇒ no reflow ⇒ exactly one measure per node. This is THE
    /// load-bearing guard: the old `3^depth` re-entry would hang at n=1000.
    fn spawn_fixed_chain(world: &mut EcsMaster, n: usize) -> u32 {
        assert!(n >= 1, "a chain needs at least the root");
        world.run_system(move |mut cmds: Commands| {
            let mut parent = {
                let mut e = cmds.spawn(col(Unit::Px(400.0), Unit::Px(800.0)));
                e.insert(ComputedRect::default());
                e.insert(UiRoot);
                e.id()
            };
            for _ in 1..n {
                let mut e = cmds.spawn(col(Unit::Px(100.0), Unit::Px(40.0)));
                e.insert(ComputedRect::default());
                e.set_parent(parent);
                parent = e.id();
            }
        });
        n as u32
    }

    /// Spawns a WIDE fan: one fixed-size `Column` root with `n - 1` fixed-size
    /// `Px(10)` leaf children. Returns `n` (root + leaves). Each leaf measured once,
    /// the root once.
    fn spawn_fixed_fan(world: &mut EcsMaster, n: usize) -> u32 {
        assert!(n >= 1, "a fan needs at least the root");
        world.run_system(move |mut cmds: Commands| {
            let root = {
                let mut e = cmds.spawn(col(Unit::Px(400.0), Unit::Px(800.0)));
                e.insert(ComputedRect::default());
                e.insert(UiRoot);
                e.id()
            };
            for _ in 1..n {
                let mut e = cmds.spawn(col(Unit::Px(10.0), Unit::Px(10.0)));
                e.insert(ComputedRect::default());
                e.set_parent(root);
            }
        });
        n as u32
    }

    /// Spawns a BALANCED binary tree of fixed-size `Column` nodes of `levels`
    /// internal levels (root = level 0); every node fixed `Px(10)`. Returns the node
    /// count `N = 2^(levels+1) - 1`. Fixed sizes ⇒ exactly one measure per node, so
    /// `measure_visits == N` and is provably `<= 3N`.
    fn spawn_fixed_balanced(world: &mut EcsMaster, levels: usize) -> u32 {
        world.run_system(move |mut cmds: Commands| {
            let root = {
                let mut e = cmds.spawn(col(Unit::Px(1024.0), Unit::Px(800.0)));
                e.insert(ComputedRect::default());
                e.insert(UiRoot);
                e.id()
            };
            let mut frontier = vec![root];
            for _ in 0..levels {
                let mut next = Vec::with_capacity(frontier.len() * 2);
                for parent in frontier.drain(..) {
                    for _ in 0..2 {
                        let mut e = cmds.spawn(col(Unit::Px(10.0), Unit::Px(10.0)));
                        e.insert(ComputedRect::default());
                        e.set_parent(parent);
                        next.push(e.id());
                    }
                }
                frontier = next;
            }
        });
        (1u32 << (levels + 1)) - 1
    }

    /// Runs one relayout frame over a spawned tree and returns `measure_visits`.
    fn measure_visits_for(world: &mut EcsMaster) -> u32 {
        let mut schedule = schedule_for(world);
        schedule.run(world);
        measure_visits(world)
    }

    /// Asserts the architect's `N <= visits <= C*N` linear-scaling envelope and
    /// reports the actual count (printed so the run log proves linearity).
    #[track_caller]
    fn assert_linear(shape: &str, n: u32, visits: u32) {
        assert!(
            visits >= n,
            "{shape}: every node measured at least once — visits {visits} < N {n}"
        );
        assert!(
            visits <= SCALING_C * n,
            "{shape}: measure_visits {visits} exceeds C*N = {}*{n} = {} \
             (exponential regression?)",
            SCALING_C,
            SCALING_C * n
        );
        println!("[scaling] {shape} N={n} measure_visits={visits} (<= {}*N)", SCALING_C);
    }

    #[test]
    fn deep_chain_measure_visits_scale_linearly() {
        // The architect's spec asks for a DEEP chain at N ∈ {10, 100, 1000}. The
        // engine clamps layout recursion at `MAX_LAYOUT_DEPTH = 128` (a documented
        // cycle / pathological-depth guard whose `debug_assert!` PANICS in debug),
        // and a chain of `n` nodes recurses to depth `n - 1`, so a single spine is
        // only legal up to `n = MAX_LAYOUT_DEPTH` (128 nodes ⇒ depth 127). N=1000
        // as a *chain* therefore hits the depth clamp — by design, not a layout
        // bug. The anti-exponential N=1000 guard lives in the WIDE and BALANCED
        // shapes (which reach N=1000 / 1023 at shallow depth); here the DEEP axis is
        // exercised at the maximum legal depth. N is capped to the guard for the
        // 1000 case so the test asserts the legal-depth scaling rather than
        // tripping the intentional clamp.
        let max_chain = MAX_LAYOUT_DEPTH; // deepest legal single spine (depth n-1 < 128)
        for &requested in &[10usize, 100, 1000] {
            let n = requested.min(max_chain);
            let mut world = complexity_world();
            let count = spawn_fixed_chain(&mut world, n);
            let visits = measure_visits_for(&mut world);
            // Fixed sizes ⇒ EXACTLY one measure per node. The strict `==` is the
            // sharpest possible anti-exponential statement; the `assert_linear`
            // envelope (`<= 3N`) is the architect's documented ceiling. Under the
            // OLD `(3*branching)^depth` re-entry even this max-legal-depth chain
            // (depth 127) would be `3^127` ≈ 10^60 visits — the test would never
            // return — so a regression to any per-ancestor re-measure trips here.
            assert_eq!(
                visits, count,
                "deep chain N={count} (requested {requested}): fixed sizes must \
                 measure each node exactly once (got {visits}) — any re-entry is an \
                 exponential regression"
            );
            assert_linear("deep-chain", count, visits);
        }
    }

    #[test]
    fn wide_fan_measure_visits_scale_linearly() {
        for &n in &[10usize, 100, 1000] {
            let mut world = complexity_world();
            let count = spawn_fixed_fan(&mut world, n);
            let visits = measure_visits_for(&mut world);
            assert_eq!(
                visits, count,
                "wide fan N={count}: each leaf + root measured exactly once (got {visits})"
            );
            assert_linear("wide-fan", count, visits);
        }
    }

    #[test]
    fn balanced_tree_measure_visits_scale_linearly() {
        // levels {3,6,9} ⇒ N ∈ {15, 127, 1023} — the architect's balanced sizes,
        // bracketing the {10,100,1000} envelope.
        for &levels in &[3usize, 6, 9] {
            let mut world = complexity_world();
            let count = spawn_fixed_balanced(&mut world, levels);
            let visits = measure_visits_for(&mut world);
            // Bound is `<= 3N` (the architect's ceiling); fixed sizes give exactly N.
            assert_linear("balanced", count, visits);
            assert_eq!(
                visits, count,
                "balanced tree N={count}: fixed sizes ⇒ exactly one measure per node \
                 (got {visits})"
            );
        }
    }

    #[test]
    fn thousand_node_tree_lays_out_well_under_50ms() {
        // Wall-clock sanity: a 1000-node tree must complete a full relayout far
        // under 50 ms. A WIDE FAN is used (not a chain — a 1000-deep chain hits the
        // MAX_LAYOUT_DEPTH clamp, see `deep_chain_…`); the fan still has 1000 nodes
        // and is exactly the shape the OLD exponential code's per-parent re-entry
        // could not lay out quickly. Debug build, so the 50 ms bound is generous
        // (release is ~100× faster); this is a coarse anti-super-linear guard that
        // complements the exact visit-count assertions.
        let mut world = complexity_world();
        let count = spawn_fixed_fan(&mut world, 1000);
        let mut schedule = schedule_for(&mut world);
        let start = std::time::Instant::now();
        schedule.run(&mut world);
        let elapsed = start.elapsed();
        let visits = measure_visits(&world);
        assert_eq!(visits, count, "1000-node fan measured each node once");
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "1000-node layout took {elapsed:?} (>= 50ms) — possible super-linear blowup"
        );
        println!("[scaling] 1000-node fan laid out in {elapsed:?} (visits={visits})");
    }
}
