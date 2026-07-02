//! World→screen projection (GUI P7a) — the pure math core + the project system.
//!
//! [`project_world_to_screen`] is the load-bearing, allocation-free,
//! branch-light per-anchor math (one `Mat4·Vec4` + one divide + a handful of
//! compares). [`ui_world_project_system`] drives it over every
//! [`UiWorldAnchor`] root, writing the result into [`UiWorldProjection`]
//! set-if-changed (the Changed-gate) and flipping the frustum-cull
//! [`UiWorldCulled`] bit O(1).
//!
//! # Clip-space convention (the hand-computed gate's ground truth)
//!
//! The engine's [`Mat4`] is COLUMN-MAJOR; [`Mat4::perspective_rh`] is
//! right-handed with depth `[0, 1]` (the Vulkan/WGSL convention) and clip
//! `w = -view_z`. Behind-camera ⇒ `w <= 0`. NDC `x, y ∈ [-1, 1]` map to logical
//! pixels with the y-flip that matches `ComputedRect` / `UiViewport` (+y DOWN):
//!
//! ```text
//! screen_x = (ndc.x * 0.5 + 0.5) * vp_w
//! screen_y = (1.0 - (ndc.y * 0.5 + 0.5)) * vp_h
//! ```

use std::mem;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_math::mat::Mat4;
use boyko_math::vec::Vec4;
use boyko_scene::camera::ViewUniform;
use boyko_scene::transform::GlobalTransform;

use crate::resources::UiViewport;
use crate::world::components::{
    UiWorldAnchor, UiWorldCulled, UiWorldProjection, WorldScaleMode, WorldTarget,
};
use crate::world::pick::UiWorldScratch;

/// The off-screen cull margin (NDC units): an anchor whose POINT is within this
/// margin past an edge still projects (so a partly-off-screen label survives a
/// little). The cull is anchor-POINT-based, not subtree-AABB-based — a large
/// subtree whose anchor leaves the frustum is culled wholesale (a documented P7a
/// limitation; true subtree-AABB culling is deferred to the P7b render pass).
const CULL_MARGIN_NDC: f32 = 0.1;

/// The clip-`w` epsilon below which an anchor is treated as behind the camera.
/// `w = -view_z`; `w <= EPS` ⇒ at or behind the eye plane (avoids a divide that
/// would explode `1/w`).
const CLIP_W_EPS: f32 = 1.0e-4;

/// The minimum eye-distance used for `WorldScaled` / fade, clamping away a
/// divide-by-zero when the anchor coincides with the eye.
const MIN_DIST: f32 = 1.0e-4;

/// The result of projecting one world point through a camera.
///
/// `screen_{x,y}` are logical pixels (+x right, +y down — the `ComputedRect`
/// basis); `clip_w` is the perspective denominator (`= -view_z`, the forward
/// depth); `visible` is `false` when the point is behind the camera or off-screen
/// (past the [`CULL_MARGIN_NDC`] margin).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectedPoint {
    /// Screen-space x, logical px (+x right).
    pub x: f32,
    /// Screen-space y, logical px (+y down).
    pub y: f32,
    /// Clip-space `w` = the perspective denominator (`-view_z`, forward depth).
    pub clip_w: f32,
    /// NDC z (`[0, 1]` inside the frustum).
    pub ndc_z: f32,
    /// `false` when behind the camera or off-screen.
    pub visible: bool,
}

/// Projects a world point through `view_proj` to a screen-space pixel + a
/// visibility flag (the pure, allocation-free P7a math core).
///
/// `view_proj` is the COLUMN-MAJOR world→clip matrix (`proj · view`, exactly
/// [`ViewUniform::view_proj`]). `vp_w` / `vp_h` are the logical viewport extent.
/// See the module docs for the clip-space convention; the y-flip matches
/// `ComputedRect`'s +y-down basis.
///
/// Behind-camera (`clip_w <= CLIP_W_EPS`) returns `visible = false` with the
/// pixel coordinates left at the (meaningless) raw values; off-screen (NDC past
/// the margin on x/y, or z outside `[0, 1]`) likewise clears `visible` but the
/// pixel is still computed (a culled-but-positioned root can be un-culled without
/// re-projection).
#[inline]
pub fn project_world_to_screen(
    view_proj: &Mat4,
    world: [f32; 3],
    vp_w: f32,
    vp_h: f32,
) -> ProjectedPoint {
    let clip = view_proj.mul_vec4(Vec4::new(world[0], world[1], world[2], 1.0));

    // Behind-camera: w = -view_z; w <= 0 ⇒ at/behind the eye plane. Bail before
    // the divide so 1/w never explodes into a NaN/Inf pixel.
    if clip.w <= CLIP_W_EPS {
        return ProjectedPoint {
            x: 0.0,
            y: 0.0,
            clip_w: clip.w,
            ndc_z: 0.0,
            visible: false,
        };
    }

    let inv_w = 1.0 / clip.w;
    let ndc_x = clip.x * inv_w;
    let ndc_y = clip.y * inv_w;
    let ndc_z = clip.z * inv_w;

    // NDC → pixel (Vulkan top-left origin, +y down). The y-flip is REQUIRED:
    // NDC +y is up, `ComputedRect` / `UiViewport` +y is down.
    let screen_x = (ndc_x * 0.5 + 0.5) * vp_w;
    let screen_y = (1.0 - (ndc_y * 0.5 + 0.5)) * vp_h;

    // Off-screen cull: anchor POINT past the margin on x/y, or depth outside the
    // frustum. Folded to one bool (branch-light).
    let lo = -1.0 - CULL_MARGIN_NDC;
    let hi = 1.0 + CULL_MARGIN_NDC;
    let visible = ndc_x >= lo && ndc_x <= hi && ndc_y >= lo && ndc_y <= hi && (0.0..=1.0).contains(&ndc_z);

    ProjectedPoint {
        x: screen_x,
        y: screen_y,
        clip_w: clip.w,
        ndc_z,
        visible,
    }
}

/// Computes the `WorldScaled` uniform subtree scale for `anchor` at eye-distance
/// `dist`. `ScreenSpace` is constant `1.0`. Always finite and `> 0`.
#[inline]
fn resolve_scale(anchor: &UiWorldAnchor, dist: f32) -> f32 {
    match anchor.scale_mode {
        WorldScaleMode::ScreenSpace => 1.0,
        WorldScaleMode::WorldScaled => {
            let d = dist.max(MIN_DIST);
            // ref/dist: 1.0 at ref_distance, shrinks (→0) with distance, grows
            // (bounded by ref/MIN_DIST) closer. ref_distance is author-validated
            // finite > 0; guard MIN_DIST keeps the result finite.
            (anchor.ref_distance / d).max(MIN_DIST)
        }
    }
}

/// Computes the distance-fade factor for `anchor` at eye-distance `dist`:
/// `saturate((fade_far - dist) / (fade_far - fade_near))`, monotonically
/// non-increasing in `dist`, always in `[0, 1]`. `1.0` when fade is off.
#[inline]
fn resolve_fade(anchor: &UiWorldAnchor, dist: f32) -> f32 {
    if !anchor.fade_by_distance {
        return 1.0;
    }
    let span = anchor.fade_far - anchor.fade_near;
    // A non-positive span (fade_far <= fade_near, a misconfiguration) collapses
    // to a binary fade at fade_near rather than a NaN: full below, off at/above.
    if span <= 0.0 {
        return if dist < anchor.fade_near { 1.0 } else { 0.0 };
    }
    let t = (anchor.fade_far - dist) / span;
    t.clamp(0.0, 1.0)
}

/// Resolves an anchor's FINAL world point (the target base + its `offset`), or
/// `None` for a dangling [`WorldTarget::EntityAnchor`] (despawned target or one
/// without a `GlobalTransform`).
///
/// The single source of truth for "where is this anchor in the world", shared by
/// [`ui_world_project_system`] (the projected point) and `ui_world_pick_system`'s
/// occlusion pass (the eye→anchor ray) so the two can never drift apart. A
/// `WorldPos` target is the fixed point; an `EntityAnchor` reads the tracked
/// entity's `GlobalTransform.translation` live. The caller decides what `None`
/// means (the project system marks the root invisible; the pick system clears the
/// occlusion bit).
#[inline]
pub(crate) fn resolve_anchor_point(
    world: &EcsMaster,
    anchor: &UiWorldAnchor,
) -> Option<[f32; 3]> {
    let base = match anchor.target {
        WorldTarget::WorldPos(p) => p,
        WorldTarget::EntityAnchor(target) => {
            let gt = world.get_component::<GlobalTransform>(target)?;
            let t = gt.translation();
            [t.x, t.y, t.z]
        }
    };
    Some([
        base[0] + anchor.offset[0],
        base[1] + anchor.offset[1],
        base[2] + anchor.offset[2],
    ])
}

/// Projects every [`UiWorldAnchor`] root to a screen seed (GUI P7a).
///
/// An EXCLUSIVE system (`&mut EcsMaster`): the engine's `Query` has no
/// entity-yielding `QueryData`, so the cull-tag flip — which needs the ROOT
/// `Entity` — cannot ride a normal query (the same constraint the layout pass
/// documents). It enumerates the world-anchor roots via `query_entities_buf`, reads
/// each anchor + the shared [`ViewUniform`] / [`UiViewport`], projects, then
/// writes [`UiWorldProjection`] set-if-changed and toggles [`UiWorldCulled`]
/// directly (O(1)).
///
/// Schedule it AFTER `resolve_active_camera` (fresh `ViewUniform`) and
/// `propagate_transforms` (fresh `GlobalTransform`), and BEFORE
/// `ui_layout_discovery` (so the same-frame relayout sees the new origin). The
/// ORDER is the host's responsibility (the P1/P5b discipline).
///
/// # Distance semantics (M1)
///
/// `WorldScaled` scale and the distance fade use the TRUE Euclidean eye-distance
/// (`|world - camera_pos|`), NOT the clip-`w` forward depth — so a laterally
/// off-center anchor at the same RANGE scales / fades the same as a centered one
/// (no lateral-parallax artifact). `clip_w` is used ONLY as the projection
/// denominator inside [`project_world_to_screen`].
///
/// # 0%-overhead
///
/// With no `UiWorldAnchor` in the world, the root query yields an empty set and
/// the system does no projection / no write. A still anchor + still camera
/// re-projects to a bit-identical [`UiWorldProjection`], so `set_if_neq`
/// suppresses the write and `Changed<UiWorldProjection>` stays clear (no
/// relayout). The root list rides the retained [`UiWorldScratch`] buffers, so a
/// steady frame allocates nothing.
//
// `clippy::needless_pass_by_ref_mut`: `query_entities_buf` / `get_component` /
// `get_component_mut` / `enable` / `disable` are reached through `&mut self`
// engine methods clippy cannot see through. Mirrors `ui_layout_apply` /
// `ui_dispatch_system`.
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn ui_world_project_system(world: &mut EcsMaster) {
    // The shared view + viewport (Copy snapshots so no resource borrow is held
    // across the per-root `get_component_mut`). Both default to a valid value
    // (identity view / zero viewport) before their producers first run.
    let view = *world.resource::<ViewUniform>();
    let viewport = *world.resource::<UiViewport>();

    // Camera eye for the TRUE distance (M1). `camera_pos.w` is free.
    let cam = view.camera_pos;
    let cam_xyz = [cam.x, cam.y, cam.z];

    // Aspect-mismatch guard (m4): if the camera aspect disagrees with the
    // viewport aspect, the projection lands at the wrong pixel. A host bug; trip
    // it in tests rather than ship a silent offset. `aspect == 0` (orthographic /
    // pre-resolve identity view) and a zero-extent viewport are skipped.
    debug_assert!(
        view.aspect == 0.0
            || viewport.width == 0.0
            || viewport.height == 0.0
            || (view.aspect - viewport.width / viewport.height).abs() < 1.0e-2,
        "ui_world_project: camera aspect {} != viewport aspect {} — the projection \
         will land at the wrong pixel (host must keep them in sync)",
        view.aspect,
        viewport.width / viewport.height,
    );

    // Move the retained root/arch buffers out so the per-root `&mut world` calls do
    // not conflict with a held resource borrow (the `mem::take` borrow protocol),
    // then refill the root list via the allocation-free `query_entities_buf`. World
    // roots are O(tens), off any per-entity hot path.
    let mut scratch = mem::take(world.resource_mut::<UiWorldScratch>());
    world.query_entities_buf(&[UiWorldAnchor::component_id()], &mut scratch.roots, &mut scratch.arch_ids);

    for &root in scratch.roots.iter() {
        let Some(anchor) = world.get_component::<UiWorldAnchor>(root).copied() else {
            continue;
        };

        // A root must not be both world-anchored and screen-edge-anchored
        // (mutually-exclusive root-positioning kinds).
        debug_assert!(
            world
                .get_component::<crate::components::UiAnchor>(root)
                .is_none(),
            "a UiWorldAnchor root must not also carry a screen-edge UiAnchor \
             (mutually-exclusive root-positioning kinds)"
        );

        // Resolve the world point: a fixed point, or a live entity's translation
        // (+ the anchor offset), via the shared helper. A dangling EntityAnchor
        // (despawned / no GlobalTransform) leaves the anchor invisible this frame
        // rather than projecting a stale point.
        let Some(point) = resolve_anchor_point(world, &anchor) else {
            mark_invisible(world, root);
            continue;
        };

        let pp = project_world_to_screen(&view.view_proj, point, viewport.width, viewport.height);

        // TRUE Euclidean eye-distance (M1) for scale + fade.
        let dx = point[0] - cam_xyz[0];
        let dy = point[1] - cam_xyz[1];
        let dz = point[2] - cam_xyz[2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();

        let scale = resolve_scale(&anchor, dist);
        let fade = resolve_fade(&anchor, dist);

        debug_assert!(
            scale.is_finite() && scale > 0.0 && (0.0..=1.0).contains(&fade),
            "ui_world_project: scale {scale} must be finite > 0 and fade {fade} in [0,1]"
        );

        let projection = UiWorldProjection {
            screen_x: pp.x,
            screen_y: pp.y,
            scale,
            fade,
            depth: pp.ndc_z,
            visible: pp.visible,
        };

        // Write set-if-changed (the Changed-gate). A missing column is skipped
        // (mirrors layout's missing-rect tolerance); `#[require(UiWorldProjection)]`
        // on `UiWorldAnchor` keeps the column present for an author-spawned anchor.
        if let Some(mut guard) = world.get_component_mut::<UiWorldProjection>(root) {
            guard.set_if_neq(projection);
        }

        // Frustum cull (Decision 3, M4): flip the project-owned UiWorldCulled bit
        // O(1). Visible ⇒ clear; off-screen / behind ⇒ set. Independent of the
        // hover-driven UiWorldHidden bit, so the two authorities never race.
        if pp.visible {
            world.disable::<UiWorldCulled>(root);
        } else {
            world.enable::<UiWorldCulled>(root);
        }
    }

    // Put the retained buffers back with their (grown) capacity intact.
    *world.resource_mut::<UiWorldScratch>() = scratch;
}

/// Marks `root`'s projection invisible + culled (a dangling `EntityAnchor`). The
/// cold off-path: a tracked target lost its `GlobalTransform`.
#[cold]
#[inline(never)]
fn mark_invisible(world: &mut EcsMaster, root: Entity) {
    if let Some(mut guard) = world.get_component_mut::<UiWorldProjection>(root) {
        let mut p = *guard;
        p.visible = false;
        guard.set_if_neq(p);
    }
    world.enable::<UiWorldCulled>(root);
}
