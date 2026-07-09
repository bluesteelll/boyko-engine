//! Cursor-ray pick + depth-test occlusion (GUI P7b) — the exclusive system that
//! turns the cursor into a 3D pick and re-derives every world-anchor root's
//! occlusion bit.
//!
//! [`ui_world_pick_system`] builds the cursor ray from the S3 [`ViewUniform`] (via
//! [`camera_ray`]), ray-tests every [`UiPickable`] + `GlobalTransform` scene
//! entity, writes the NEAREST hit into [`HoveredWorldEntity`] (the seam
//! [`ui_world_visibility_system`](super::visibility::ui_world_visibility_system)
//! consumes), and ALSO casts an eye→anchor ray per root to flip the
//! [`UiWorldOccluded`] bit (a CPU PROXY against the SAME bounds the pick tests —
//! NOT a GPU depth-buffer test).
//!
//! # C2 — no stale set bit
//!
//! The occlusion pass ALWAYS walks every [`UiWorldAnchor`] root every run and
//! re-derives its [`UiWorldOccluded`] bit UNCONDITIONALLY (enable if occluded,
//! disable otherwise). On EVERY path — cursor-inactive, `depth_test == false`,
//! culled, dangling target, occluded, clear — the bit is written. No code path
//! leaves a set bit un-revisited (the gate-5 eviction class: a pickable that
//! despawned after setting a bit, or a cursor that left the window, can never
//! strand a root permanently occluded).
//!
//! Principle 0: the pick bound is a first-class ECS component ([`UiPickable`]) on
//! the engine's own storage; the per-frame root/pickable/bound buffers live in the
//! retained [`UiWorldScratch`] resource (cleared-then-refilled, so the steady state
//! allocates nothing — the sibling `UiInteractionScratch` / `UiBarScratch`
//! discipline), NOT a parallel persistent data store.

use std::mem;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_input::PhysicalInput;
use boyko_macros::Resource;
use boyko_math::{Ray, Vec3, ray_aabb, ray_sphere};
use boyko_scene::camera::{ViewUniform, camera_ray};
use boyko_scene::transform::GlobalTransform;

use crate::resources::UiViewport;
use crate::world::components::{
    HoveredWorldEntity, UiPickShape, UiPickable, UiWorldAnchor, UiWorldOccluded, UiWorldProjection,
    WorldTarget,
};
use crate::world::project::resolve_anchor_point;

/// The RELATIVE (scale-invariant) occlusion bias: a root is occluded iff a nearer
/// surface is hit at `t < dA * (1.0 - REL_BIAS)`, where `dA` is the eye→anchor
/// distance. A FIXED world-unit bias is meaningless at a different world scale
/// (1e-3 units is huge in a millimeter scene, nothing in a kilometer scene); the
/// relative form is always ~0.1% of the label range, so a label coincident with a
/// surface does not flicker (O2).
const REL_BIAS: f32 = 1.0e-3;

/// The pick `layer_mask`: which layers the cursor picks (ANDed against each
/// [`UiPickable::layers`]). `u32::MAX` = "pick all layers" — the YAGNI default
/// keeps the still path identical to "no masking"; a per-layer use case adds a
/// `Resource` later (the plan's open question 1).
const PICK_LAYER_MASK: u32 = u32::MAX;

/// A pickable's bound pre-transformed into WORLD space ONCE per frame, so the pick
/// pass (step 5) and the occlusion pass (step 6) ray-test IDENTICAL geometry — the
/// single place the `GlobalTransform` → world-bound math lives.
///
/// The shape stays in the entity's frame except for the uniform scale `s` folded
/// into `radius`/`half_extents`; `center` is the world translation. An `Aabb`
/// stays axis-aligned in WORLD (a documented approximation — true OBB picking is
/// out of scope, §6).
#[derive(Clone, Copy)]
struct PickBound {
    /// The pickable scene entity (the id written to [`HoveredWorldEntity`] / used
    /// for occlusion self-exclusion).
    entity: Entity,
    /// World-space center (the `GlobalTransform` translation).
    center: Vec3,
    /// The bound shape with the uniform scale `s` already folded in.
    shape: WorldShape,
}

/// A [`UiPickShape`] resolved to WORLD units (the uniform scale applied).
#[derive(Clone, Copy)]
enum WorldShape {
    /// A world-radius sphere.
    Sphere(f32),
    /// A world-half-extents axis-aligned box.
    Aabb(Vec3),
}

/// Reused per-frame scratch for the world-UI systems (a `Resource` — engine
/// storage, allocated once, capacity retained). Frame-transient: every buffer is
/// `clear()`-then-refilled so the steady-state path allocates nothing (Principle
/// 1/5, the `UiInteractionScratch` / `UiBarScratch` discipline).
///
/// Shared by [`ui_world_pick_system`] (all four buffers), the project system, and
/// the visibility system (both reuse only `roots` + `arch_ids`). `Default` is
/// fully EMPTY so it is a valid `mem::take` target: each exclusive body moves the
/// buffers onto its stack for the world-mutating loop, then moves them back with
/// capacity retained (the `&mut world` calls in the loop cannot coexist with a held
/// resource borrow — the same borrow protocol `ui_bar_apply` uses).
///
/// Principle 0: these are transient per-frame query/snapshot buffers, NOT a
/// parallel persistent per-entity store — every durable datum stays a component on
/// the engine's own storage.
#[derive(Resource, Default)]
pub struct UiWorldScratch {
    /// Retained `UiWorldAnchor` root-query buffer (the roots project / pick /
    /// visibility all iterate). Refilled via `query_entities_buf`. `pub(crate)` so
    /// the sibling `project` / `visibility` modules reuse the same buffer.
    pub(crate) roots: Vec<Entity>,
    /// Retained `UiPickable` entity-query buffer for [`collect_bounds`]. Refilled
    /// via `query_entities_buf`. Only the pick system uses it.
    pickables: Vec<Entity>,
    /// Retained world-space [`PickBound`] snapshot (one per pickable), built ONCE
    /// per pick frame and ray-tested by both the pick and occlusion passes. Only
    /// the pick system uses it.
    bounds: Vec<PickBound>,
    /// Retained archetype-id scratch backing every `query_entities_buf` call above
    /// (alloc-free archetype walk). `pub(crate)` so the sibling modules reuse it.
    pub(crate) arch_ids: Vec<ArchetypeId>,
}

/// The conservative uniform scale `s = max(‖col_0‖, ‖col_1‖, ‖col_2‖)` of the
/// row-major `Mat3 { rows: [Vec3; 3] }` linear part (the COLUMN norms — NOT a
/// max-abs element, W3).
///
/// A uniform-scaled target has equal column norms (`debug_assert!`ed within eps,
/// the uniform-scale contract, mirroring `debug_assert_camera_rigid`); a
/// non-uniform-scaled target conservatively uses the largest (the bound never
/// shrinks below the true shape on any axis).
#[inline]
fn uniform_scale(gt: &GlobalTransform) -> f32 {
    let r = &gt.affine().matrix3.rows;
    // Column i of the row-major matrix = (rows[0][i], rows[1][i], rows[2][i]).
    let c0 = Vec3::new(r[0].x, r[1].x, r[2].x).length();
    let c1 = Vec3::new(r[0].y, r[1].y, r[2].y).length();
    let c2 = Vec3::new(r[0].z, r[1].z, r[2].z).length();
    debug_assert!(
        within_eps(c0, c1) && within_eps(c0, c2),
        "UiPickable target must be uniform-scaled (column norms {c0}, {c1}, {c2})"
    );
    c0.max(c1).max(c2)
}

/// `true` when `a` and `b` agree to a relative-plus-absolute eps (the uniform-scale
/// contract check). Tolerant near zero (a degenerate zero-scale target).
#[inline]
fn within_eps(a: f32, b: f32) -> bool {
    const EPS: f32 = 1.0e-3;
    (a - b).abs() <= EPS * (1.0 + a.abs())
}

/// Ray-tests a world-space [`PickBound`] and returns the nearest non-negative `t`
/// (or `None` on a miss / degenerate ray — the W2 guard lives in `ray_*`).
#[inline]
fn intersect(ray: Ray, bound: &PickBound) -> Option<f32> {
    match bound.shape {
        WorldShape::Sphere(radius) => ray_sphere(ray, bound.center, radius),
        WorldShape::Aabb(half_extents) => ray_aabb(ray, bound.center, half_extents),
    }
}

/// The nearest positive-`t` hit of `ray` over `bounds`, excluding the bound whose
/// entity equals `exclude` (the anchor's own `EntityAnchor` target — O1
/// self-exclusion). Returns `(t, entity)` of the nearest hit, or `None`.
///
/// Shared by the PICK pass (no exclusion) and the OCCLUSION pass (self-exclusion),
/// so both walk the SAME pre-transformed bounds with one selection rule.
#[inline]
fn nearest_hit(ray: Ray, bounds: &[PickBound], exclude: Option<Entity>) -> Option<(f32, Entity)> {
    let mut best: Option<(f32, Entity)> = None;
    for bound in bounds {
        if Some(bound.entity) == exclude {
            continue;
        }
        if let Some(t) = intersect(ray, bound) {
            let take = match best {
                None => true,
                Some((best_t, _)) => t < best_t,
            };
            if take {
                best = Some((t, bound.entity));
            }
        }
    }
    best
}

/// Writes `value` into [`HoveredWorldEntity`] only when it changes (the
/// `set_if_neq` discipline the world systems share). The downstream visibility
/// system has its OWN Changed-gate, so the gate here keeps the resource's
/// change-tick clean on a still pick.
#[inline]
fn set_hovered_if_changed(world: &mut EcsMaster, value: Option<Entity>) {
    let r = world.resource_mut::<HoveredWorldEntity>();
    if r.0 != value {
        r.0 = value;
    }
}

/// The cursor-ray pick + depth-test occlusion system (GUI P7b), EXCLUSIVE.
///
/// Reads `PhysicalInput` + [`UiViewport`] + [`ViewUniform`] (snapshot-copied so no
/// resource borrow is held across the per-entity `&mut`-calls). If the cursor is
/// inactive (outside / unfocused) it writes `HoveredWorldEntity(None)`
/// (set-if-changed) AND walks EVERY [`UiWorldAnchor`] root to DISABLE its
/// [`UiWorldOccluded`] bit, then returns (C2: the clear-all path leaves no stale
/// set bit). Otherwise:
///
///  1. PICK: builds the cursor ray, ray-tests every [`UiPickable`] +
///     `GlobalTransform` entity (layer-masked), and writes the NEAREST positive-`t`
///     hit's entity into `HoveredWorldEntity` (set-if-changed); no hit → `None`.
///  2. OCCLUSION: walks EVERY root EVERY run and re-derives its `UiWorldOccluded`
///     bit UNCONDITIONALLY. A `depth_test == true` root casts an eye→anchor-point
///     ray and is ENABLED iff some [`UiPickable`] surface (excluding the anchor's
///     own `EntityAnchor` target — O1) is hit at `t < dA * (1 - REL_BIAS)`, else
///     DISABLED. A `depth_test == false` root is always DISABLED. Because the bit
///     is re-derived (enable XOR disable) for every root on every run, no path
///     leaves a set `UiWorldOccluded` bit un-revisited (C2).
///
/// Schedule: `.after(resolve_active_camera)` (fresh `ViewUniform`),
/// `.after(propagate_transforms)` (fresh `GlobalTransform`),
/// `.before(ui_world_visibility_system)` (it consumes `HoveredWorldEntity`).
//
// `clippy::needless_pass_by_ref_mut`: `query_entities_buf` / `get_component` /
// `resource` / `resource_mut` / `enable` / `disable` are `&mut self` engine
// methods clippy cannot see through. Mirrors `ui_world_project_system` /
// `ui_focus_system`.
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn ui_world_pick_system(world: &mut EcsMaster) {
    // Snapshot the shared inputs (Copy/Clone) so no resource borrow is held across
    // the per-entity `&mut`-calls (the P7a snapshot borrow protocol).
    let view = *world.resource::<ViewUniform>();
    let viewport = *world.resource::<UiViewport>();
    let physical = world.resource::<PhysicalInput>().clone();

    // Move the retained buffers out so the per-entity `&mut world` calls do not
    // conflict with a held resource borrow (the `mem::take` borrow protocol).
    let mut scratch = mem::take(world.resource_mut::<UiWorldScratch>());

    let cursor_active = physical.cursor_inside && physical.window_focused;

    if !cursor_active {
        // Cursor gone: clear the hover AND re-derive every root's occlusion bit to
        // "not occluded" (C2 — a bit set on a prior active frame must not survive).
        set_hovered_if_changed(world, None);
        clear_all_occlusion(world, &mut scratch.roots, &mut scratch.arch_ids);
        *world.resource_mut::<UiWorldScratch>() = scratch;
        return;
    }

    // Cursor → LOGICAL px (one narrow after the divide — the focus-system idiom).
    let scale = if viewport.scale_factor > 0.0 {
        viewport.scale_factor
    } else {
        1.0
    };
    debug_assert!(viewport.scale_factor > 0.0, "UiViewport.scale_factor must be > 0");
    let cursor = [
        (physical.cursor_pos[0] / scale as f64) as f32,
        (physical.cursor_pos[1] / scale as f64) as f32,
    ];

    let ray = camera_ray(&view, cursor[0], cursor[1], viewport.width, viewport.height);

    // Snapshot every pickable's WORLD bound ONCE into the retained `bounds` buffer
    // (the single transform-math site, reused by the pick selection AND every
    // occlusion ray — avoids re-querying / re-transforming per root). Cleared then
    // refilled, capacity retained (world UI is sparse, O(tens) pickables) — NOT a
    // persistent parallel store (Principle 0).
    collect_bounds(world, &mut scratch.pickables, &mut scratch.bounds, &mut scratch.arch_ids);

    // ── PICK: nearest positive-t hit over all bounds (no self-exclusion) ────────
    let hovered = nearest_hit(ray, &scratch.bounds, None).map(|(_, e)| e);
    set_hovered_if_changed(world, hovered);

    // ── OCCLUSION: re-derive EVERY root's bit unconditionally (C2) ──────────────
    // Refill the retained `roots` buffer (same query the project / visibility
    // systems drive); iterate by reference so `bounds` stays borrowable alongside.
    let eye = view.camera_pos.xyz();
    world.query_entities_buf(&[UiWorldAnchor::component_id()], &mut scratch.roots, &mut scratch.arch_ids);
    for &root in scratch.roots.iter() {
        let Some(anchor) = world.get_component::<UiWorldAnchor>(root).copied() else {
            continue;
        };

        // depth_test == false → always-on-top overlay, never occluded.
        if !anchor.depth_test {
            world.disable::<UiWorldOccluded>(root);
            continue;
        }

        // Already frustum-culled / behind-eye: re-derive the bit to "not occluded"
        // (a re-derivation to disable, NOT a skip) and avoid a meaningless ray.
        if let Some(proj) = world.get_component::<UiWorldProjection>(root)
            && !proj.visible
        {
            world.disable::<UiWorldOccluded>(root);
            continue;
        }

        // The anchor's final world point, via the shared resolver (so the pick
        // point and the occlusion ray can never drift). A dangling EntityAnchor →
        // not occluded (the project system already marks it invisible/culled).
        let Some(pt) = resolve_anchor_point(world, &anchor) else {
            world.disable::<UiWorldOccluded>(root);
            continue;
        };

        let anchor_pt = Vec3::new(pt[0], pt[1], pt[2]);
        let to_anchor = anchor_pt - eye;
        let dist_anchor = to_anchor.length();

        // O1: an EntityAnchor excludes its own tracked surface; a WorldPos anchor
        // has NO self-exclusion (any pickable in front of the fixed point occludes).
        let self_target = match anchor.target {
            WorldTarget::EntityAnchor(t) => Some(t),
            WorldTarget::WorldPos(_) => None,
        };

        // The eye→anchor ray. A zero `dist_anchor` (anchor AT the eye) makes the
        // dir degenerate → `ray_*` return None via the W2 guard → not occluded.
        let occ_ray = Ray::new(eye, to_anchor.normalize());
        let occluded = match nearest_hit(occ_ray, &scratch.bounds, self_target) {
            Some((t, _)) => t < dist_anchor * (1.0 - REL_BIAS),
            None => false,
        };

        if occluded {
            world.enable::<UiWorldOccluded>(root);
        } else {
            world.disable::<UiWorldOccluded>(root);
        }
    }

    // Put the retained buffers back with their (grown) capacity intact.
    *world.resource_mut::<UiWorldScratch>() = scratch;
}

/// Snapshots every [`UiPickable`] + `GlobalTransform` entity into a world-space
/// [`PickBound`] (the single transform-math site), reusing the retained `pickables`
/// / `bounds` / `arch_ids` scratch (no fresh `Vec`). `bounds` is cleared then
/// refilled; a `UiPickable` without a `GlobalTransform`, or one whose layers do not
/// intersect [`PICK_LAYER_MASK`], is skipped (the former is a setup error →
/// `debug_assert!`).
#[allow(clippy::needless_pass_by_ref_mut)] // get_component is &mut-opaque to clippy
fn collect_bounds(
    world: &mut EcsMaster,
    pickables: &mut Vec<Entity>,
    bounds: &mut Vec<PickBound>,
    arch_ids: &mut Vec<ArchetypeId>,
) {
    world.query_entities_buf(&[UiPickable::component_id()], pickables, arch_ids);
    bounds.clear();
    for &e in pickables.iter() {
        let Some(pk) = world.get_component::<UiPickable>(e).copied() else {
            continue;
        };
        // Layer gate (the cursor's mask ANDed against the target's layers).
        if pk.layers & PICK_LAYER_MASK == 0 {
            continue;
        }
        let Some(gt) = world.get_component::<GlobalTransform>(e).copied() else {
            debug_assert!(false, "UiPickable {e:?} has no GlobalTransform (setup error)");
            continue;
        };

        let center = gt.translation();
        let s = uniform_scale(&gt);
        let shape = match pk.shape {
            UiPickShape::Sphere { radius } => WorldShape::Sphere(radius * s),
            UiPickShape::Aabb { half_extents } => WorldShape::Aabb(Vec3::new(
                half_extents[0] * s,
                half_extents[1] * s,
                half_extents[2] * s,
            )),
        };
        bounds.push(PickBound {
            entity: e,
            center,
            shape,
        });
    }
}

/// Disables (clears) the [`UiWorldOccluded`] bit on EVERY [`UiWorldAnchor`] root —
/// the cursor-inactive path's C2 guarantee (no bit set on a prior active frame can
/// survive the cursor leaving the window). O(R) ≈ tens roots; `disable` writes
/// nothing when the bit is already clear, so the inactive path stays cheap. Reuses
/// the retained `roots` / `arch_ids` scratch (no fresh `Vec`).
#[allow(clippy::needless_pass_by_ref_mut)] // enable/disable are &mut-opaque to clippy
fn clear_all_occlusion(world: &mut EcsMaster, roots: &mut Vec<Entity>, arch_ids: &mut Vec<ArchetypeId>) {
    world.query_entities_buf(&[UiWorldAnchor::component_id()], roots, arch_ids);
    for &root in roots.iter() {
        world.disable::<UiWorldOccluded>(root);
    }
}
