//! GUI P7a — world-space / diegetic UI, the CPU-testable core (HEADLESS, NO GPU).
//!
//! Every gate here exercises the REAL implementation:
//! [`project_world_to_screen`] (the pure math core), [`ui_world_project_system`]
//! (the per-anchor projection + frustum-cull), [`ui_world_visibility_system`]
//! (the hover-driven show/hide), and the `layout_root` world-anchor seam.
//!
//! The gates, 1:1 with the P7a spec:
//!
//! 1. A `WorldPos` anchor projects to the HAND-COMPUTED screen pixel for a known
//!    camera `view_proj` + viewport (the load-bearing gate — the expected NDC→px
//!    is computed independently from the `perspective_rh` formula, NOT by calling
//!    the function under test).
//! 2. An `EntityAnchor` tracks a MOVING target (move `Transform` → propagate →
//!    project → screen origin follows).
//! 3. Behind-camera (`w <= 0`) + off-screen anchors are CULLED (`UiWorldCulled`
//!    set, `UiWorldProjection.visible` clear); an on-screen anchor is not.
//! 4. `WorldScaled` size shrinks with distance; `ScreenSpace` is constant
//!    (billboard) across distances.
//! 5. Distance fade is monotonically non-increasing in distance, within `[0, 1]`.
//! 6. Show/hide: setting `HoveredWorldEntity` enables that entity's anchor subtree
//!    (`UiWorldHidden` clear) + disables the prior; `None` hides all.
//! 7. 0%-gate: with no `UiWorldAnchor` the project system does zero work; a static
//!    anchor + still camera does NO re-projection (Changed-gated).
//!
//! # The deterministic frame vehicle
//!
//! Two harnesses. The math gates (1, 3, 4, 5) call `project_world_to_screen`
//! and/or drive `ui_world_project_system` once via `run_system` and read the
//! `UiWorldProjection` / `UiWorldCulled` results — no tick window needed (the
//! project system is not value-gated for the FIRST projection). The Changed-gate
//! (gate 7's static-camera half) and the moving-target gate (gate 2) need a real
//! `Schedule` so the `(last_run, this_run]` window survives across frames; that
//! harness wires `propagate_transforms` → `ui_world_project_system` →
//! `ui_layout_discovery` → `ui_layout_apply` in the documented order.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_math::{Mat4, Vec3};

use boyko_scene::{GlobalTransform, Transform, ViewUniform, propagate_transforms};

use boyko_ui::components::{ComputedRect, UiLayout, UiRoot};
use boyko_ui::layout::{ui_layout_apply, ui_layout_discovery};
use boyko_ui::resources::{LayoutScratch, UiSafeArea, UiViewport};
use boyko_ui::units::Unit;
use boyko_ui::world::{
    HoveredWorldEntity, UiWorldAnchor, UiWorldCulled, UiWorldHidden, UiWorldHoverState,
    UiWorldProjection, UiWorldScratch, WorldScaleMode, WorldTarget, project_world_to_screen,
    ui_world_project_system, ui_world_visibility_system,
};

// ════════════════════════════════════════════════════════════════════════════
// Shared constants + hand math (the ground truth — NOT the code under test)
// ════════════════════════════════════════════════════════════════════════════

const VP_W: f32 = 1600.0;
const VP_H: f32 = 900.0;
const ASPECT: f32 = VP_W / VP_H; // 16:9
const FOV_Y: f32 = core::f32::consts::FRAC_PI_2; // 90°  → tan(45°)=1 → f=1
const NEAR: f32 = 0.1;
const FAR: f32 = 100.0;

/// The camera-at-origin `view_proj`. The camera sits at the world origin with an
/// identity rotation (forward = −Z, the canonical pose), so `view = global⁻¹ =
/// IDENTITY` and `view_proj = proj · view = proj`. This is what the gate-1 hand
/// math projects through.
fn origin_view_proj() -> Mat4 {
    // view = identity (origin camera) ⇒ view_proj = perspective.
    Mat4::perspective_rh(FOV_Y, ASPECT, NEAR, FAR)
}

/// Hand-computes the expected screen pixel for `world` through the SAME clip-space
/// convention the impl documents — but built from the raw `perspective_rh`
/// formula, NOT by calling `project_world_to_screen`. This is the independent
/// oracle for the load-bearing gate.
///
/// `perspective_rh(fov_y, aspect, near, far)` (column-major), with
/// `f = 1/tan(fov_y/2)` and `nf = 1/(near-far)`, has nonzero entries
/// `col0 = (f/aspect,0,0,0)`, `col1 = (0,f,0,0)`, `col2 = (0,0,far·nf,-1)`,
/// `col3 = (0,0,near·far·nf,0)`. For `world=(x,y,z,1)` the clip-space result is
/// `clip.x = (f/aspect)·x`, `clip.y = f·y`, `clip.z = far·nf·z + near·far·nf`,
/// `clip.w = -z`.
fn hand_project(world: [f32; 3]) -> (f32, f32, f32, f32) {
    let f = (FOV_Y * 0.5).tan().recip();
    let nf = (NEAR - FAR).recip();
    let (x, y, z) = (world[0], world[1], world[2]);
    let clip_x = (f / ASPECT) * x;
    let clip_y = f * y;
    let clip_z = FAR * nf * z + NEAR * FAR * nf;
    let clip_w = -z;
    let inv_w = 1.0 / clip_w;
    let ndc_x = clip_x * inv_w;
    let ndc_y = clip_y * inv_w;
    let ndc_z = clip_z * inv_w;
    let screen_x = (ndc_x * 0.5 + 0.5) * VP_W;
    let screen_y = (1.0 - (ndc_y * 0.5 + 0.5)) * VP_H; // +y-down flip
    (screen_x, screen_y, clip_w, ndc_z)
}

/// Tight epsilon for the hand-computed pixel: both sides run the identical f32 ops
/// (`perspective_rh` cols, `mul_vec4` linear combination, one divide), so they
/// agree to a few ULPs.
const EPS_PX: f32 = 1.0e-3;

#[track_caller]
fn approx(a: f32, b: f32, eps: f32, what: &str) {
    assert!((a - b).abs() <= eps, "{what}: expected {b}, got {a} (|Δ|={})", (a - b).abs());
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 1 — WorldPos anchor → HAND-COMPUTED screen pixel (THE load-bearing gate)
// ════════════════════════════════════════════════════════════════════════════

/// Centered point directly in front of the camera lands at the viewport center.
#[test]
fn gate1_pure_centered_point_is_viewport_center() {
    let vp = origin_view_proj();
    let pp = project_world_to_screen(&vp, [0.0, 0.0, -10.0], VP_W, VP_H);
    approx(pp.x, VP_W * 0.5, EPS_PX, "centered screen_x");
    approx(pp.y, VP_H * 0.5, EPS_PX, "centered screen_y");
    assert!(pp.visible, "a centered in-front point is visible");
    approx(pp.clip_w, 10.0, EPS_PX, "clip_w == -z == 10");
}

/// Several off-center in-front points match the independent hand oracle EXACTLY
/// (tight eps). The y-flip and the aspect division are both exercised.
#[test]
fn gate1_pure_offcenter_matches_hand_oracle() {
    let vp = origin_view_proj();
    // A spread of points at varying depth, lateral, and vertical offset, all in
    // front of the camera (z < 0).
    let pts = [
        [3.0, 2.0, -10.0],
        [-4.0, 1.5, -8.0],
        [2.0, -3.0, -12.0],
        [-1.0, -2.5, -5.0],
        [6.0, 0.0, -20.0],
    ];
    for p in pts {
        let pp = project_world_to_screen(&vp, p, VP_W, VP_H);
        let (ex, ey, ew, ez) = hand_project(p);
        approx(pp.x, ex, EPS_PX, &format!("screen_x for {p:?}"));
        approx(pp.y, ey, EPS_PX, &format!("screen_y for {p:?}"));
        approx(pp.clip_w, ew, EPS_PX, &format!("clip_w for {p:?}"));
        approx(pp.ndc_z, ez, EPS_PX, &format!("ndc_z for {p:?}"));
        assert!(pp.visible, "in-front, on-screen point {p:?} is visible");
    }
}

/// A hand-derived NON-trivial pixel asserted against a LITERAL expected value
/// (no shared formula at all), so a sign / flip error in either the impl or the
/// `hand_project` oracle cannot hide. Point `(3, 2, -10)` with f=1, aspect=16/9:
/// ndc_x = (1/(16/9))·3 / 10 = (9/16)·0.3 = 0.16875;
/// ndc_y = 1·2 / 10 = 0.2;
/// screen_x = (0.16875·0.5 + 0.5)·1600 = 0.584375·1600 = 935.0;
/// screen_y = (1 − (0.2·0.5 + 0.5))·900 = (1 − 0.6)·900 = 360.0.
#[test]
fn gate1_pure_literal_pixel() {
    let vp = origin_view_proj();
    let pp = project_world_to_screen(&vp, [3.0, 2.0, -10.0], VP_W, VP_H);
    approx(pp.x, 935.0, 2.0e-2, "literal screen_x for (3,2,-10)");
    approx(pp.y, 360.0, 2.0e-2, "literal screen_y for (3,2,-10)");
}

/// The full system path: a spawned `WorldPos` anchor, driven by
/// `ui_world_project_system`, writes the HAND-COMPUTED pixel into its
/// `UiWorldProjection` (and is not culled). This exercises the resource read +
/// the offset add + the `set_if_neq` write, not just the pure function.
#[test]
fn gate1_system_worldpos_writes_hand_pixel() {
    let mut h = ProjOnce::new();
    // offset is added to the target before projection; pick a target+offset that
    // sums to a known off-center in-front point.
    let target = [1.0, -1.0, -8.0];
    let offset = [2.0, 3.0, -2.0]; // → projected point (3, 2, -10)
    let anchor = h.spawn_world_anchor(WorldTarget::WorldPos(target), offset, WorldScaleMode::ScreenSpace);
    h.run();

    let proj = h.projection(anchor);
    let (ex, ey, _ew, _ez) = hand_project([3.0, 2.0, -10.0]);
    approx(proj.screen_x, ex, EPS_PX, "system screen_x");
    approx(proj.screen_y, ey, EPS_PX, "system screen_y");
    assert!(proj.visible, "in-front on-screen anchor is visible");
    assert!(!h.world.is_enabled::<UiWorldCulled>(anchor), "visible anchor is not culled");
    approx(proj.scale, 1.0, EPS_PX, "ScreenSpace scale is 1.0");
    approx(proj.fade, 1.0, EPS_PX, "fade off ⇒ 1.0");
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 2 — EntityAnchor tracks a MOVING target
// ════════════════════════════════════════════════════════════════════════════

/// Moving the tracked entity's `Transform` (then propagating + projecting) moves
/// the anchor's projected screen origin to the new world point's hand-computed
/// pixel. Driven through a real schedule so propagation + projection run in order.
#[test]
fn gate2_entity_anchor_follows_moving_target() {
    let mut h = SchedHarness::new();

    // The tracked scene entity, initially at (0,0,-10) (viewport center).
    let target = h.spawn_target(Vec3::new(0.0, 0.0, -10.0));
    // A world-anchored UI root tracking it, no offset.
    let anchor = h.spawn_entity_anchor(target, [0.0, 0.0, 0.0]);

    h.run(); // propagate + project + layout
    let p0 = h.projection(anchor);
    let (cx, cy, _, _) = hand_project([0.0, 0.0, -10.0]);
    approx(p0.screen_x, cx, 1.0e-2, "initial screen_x (center)");
    approx(p0.screen_y, cy, 1.0e-2, "initial screen_y (center)");

    // Move the target up-and-right in world space; bump the tick so the edit lands
    // in the next propagate window, then re-run.
    h.tick();
    h.set_target_translation(target, Vec3::new(3.0, 2.0, -10.0));
    h.run();

    let p1 = h.projection(anchor);
    let (ex, ey, _, _) = hand_project([3.0, 2.0, -10.0]);
    approx(p1.screen_x, ex, 1.0e-2, "moved screen_x follows target");
    approx(p1.screen_y, ey, 1.0e-2, "moved screen_y follows target");
    // It genuinely MOVED (not a stale read).
    assert!((p1.screen_x - p0.screen_x).abs() > 1.0, "screen origin actually moved in x");
    assert!((p1.screen_y - p0.screen_y).abs() > 1.0, "screen origin actually moved in y");

    // And the laid-out rect origin tracks the projection (the layout seam). The
    // project system writes `Changed<UiWorldProjection>` on the frame the value
    // moves; `ui_layout_discovery` consuming that same-frame in-schedule write
    // lands the recomputed `ComputedRect` one frame later — the engine's standard
    // one-frame change-propagation seam between two systems in one schedule (the
    // documented `ui_text_measure → layout` / p6a `run_settled` behavior, not a
    // P7a defect). One settle frame is enough.
    h.run();
    let rect = h.rect(anchor);
    approx(rect.x, ex, 1.0e-2, "ComputedRect.x tracks the projected origin");
    approx(rect.y, ey, 1.0e-2, "ComputedRect.y tracks the projected origin");
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 3 — behind-camera + off-screen are CULLED; on-screen is not
// ════════════════════════════════════════════════════════════════════════════

/// A point BEHIND the camera (positive z, looking down −Z) has `clip_w <= 0` and
/// is culled (pure function).
#[test]
fn gate3_pure_behind_camera_culled() {
    let vp = origin_view_proj();
    let pp = project_world_to_screen(&vp, [0.0, 0.0, 5.0], VP_W, VP_H);
    assert!(!pp.visible, "a point behind the camera is not visible");
    assert!(pp.clip_w <= 0.0, "clip_w = -z = -5 <= 0 behind the camera");

    // Exactly at the eye plane (z = 0 ⇒ w = 0) is also culled (the w<=EPS guard).
    let at_eye = project_world_to_screen(&vp, [0.0, 0.0, 0.0], VP_W, VP_H);
    assert!(!at_eye.visible, "a point at the eye plane (w==0) is culled");
}

/// A far-off-axis in-front point lands past the NDC margin and is off-screen.
#[test]
fn gate3_pure_offscreen_culled() {
    let vp = origin_view_proj();
    // At z=-1, ndc_x = (9/16)·x. To exceed the +1.1 margin: x > 1.1·16/9 ≈ 1.956.
    let far_right = project_world_to_screen(&vp, [10.0, 0.0, -1.0], VP_W, VP_H);
    assert!(!far_right.visible, "a far-right point is off-screen");
    // ndc_y to exceed margin at z=-1: y > 1.1.
    let far_up = project_world_to_screen(&vp, [0.0, 10.0, -1.0], VP_W, VP_H);
    assert!(!far_up.visible, "a far-up point is off-screen");
}

/// The SYSTEM path: a behind-camera anchor sets `UiWorldCulled` + clears
/// `visible`; an on-screen one clears the cull bit. And the cull bit FLIPS back
/// when the anchor comes back on-screen (re-run).
#[test]
fn gate3_system_cull_bit_tracks_visibility() {
    let mut h = ProjOnce::new();
    let behind = h.spawn_world_anchor(WorldTarget::WorldPos([0.0, 0.0, 5.0]), [0.0, 0.0, 0.0], WorldScaleMode::ScreenSpace);
    let front = h.spawn_world_anchor(WorldTarget::WorldPos([0.0, 0.0, -10.0]), [0.0, 0.0, 0.0], WorldScaleMode::ScreenSpace);
    let off = h.spawn_world_anchor(WorldTarget::WorldPos([10.0, 0.0, -1.0]), [0.0, 0.0, 0.0], WorldScaleMode::ScreenSpace);
    h.run();

    assert!(h.world.is_enabled::<UiWorldCulled>(behind), "behind-camera anchor is culled");
    assert!(!h.projection(behind).visible, "behind-camera projection is invisible");
    assert!(h.world.is_enabled::<UiWorldCulled>(off), "off-screen anchor is culled");
    assert!(!h.projection(off).visible, "off-screen projection is invisible");
    assert!(!h.world.is_enabled::<UiWorldCulled>(front), "on-screen anchor is NOT culled");
    assert!(h.projection(front).visible, "on-screen projection is visible");

    // Bring the behind anchor in front; the cull bit must clear on re-projection.
    if let Some(mut a) = h.world.get_component_mut::<UiWorldAnchor>(behind) {
        a.target = WorldTarget::WorldPos([0.0, 0.0, -10.0]);
    }
    h.run();
    assert!(!h.world.is_enabled::<UiWorldCulled>(behind), "cull bit clears once back on-screen");
    assert!(h.projection(behind).visible, "now-visible after coming on-screen");
}

/// A culled (or hidden) world root is SKIPPED by the layout pass — the
/// `layout_root` world-anchor seam. We verify the rect is NOT written to the
/// projected origin while culled, and IS once visible.
#[test]
fn gate3_layout_skips_culled_root() {
    let mut h = SchedHarness::new();
    // A target behind the camera ⇒ the anchor is culled; the layout skips it.
    let target = h.spawn_target(Vec3::new(0.0, 0.0, 5.0));
    let anchor = h.spawn_entity_anchor(target, [0.0, 0.0, 0.0]);
    h.run();
    assert!(h.world.is_enabled::<UiWorldCulled>(anchor), "behind-camera root is culled");
    assert!(!h.projection(anchor).visible, "culled root projection invisible");
    // The rect stays at its spawn default (origin), NOT moved to a projected px.
    let rect_culled = h.rect(anchor);
    // Move the target in front; the layout now positions it at the projected px.
    h.tick();
    h.set_target_translation(target, Vec3::new(0.0, 0.0, -10.0));
    h.run();
    assert!(!h.world.is_enabled::<UiWorldCulled>(anchor), "now on-screen");
    // One settle frame for the project→layout one-frame seam (see gate 2).
    h.run();
    let rect_vis = h.rect(anchor);
    let (cx, cy, _, _) = hand_project([0.0, 0.0, -10.0]);
    approx(rect_vis.x, cx, 1.0e-2, "visible root rect at projected center x");
    approx(rect_vis.y, cy, 1.0e-2, "visible root rect at projected center y");
    // The culled-frame rect did not already equal the projected position.
    assert!(
        (rect_culled.x - cx).abs() > 1.0 || (rect_culled.y - cy).abs() > 1.0,
        "while culled the rect was not at the projected center"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 4 — WorldScaled shrinks with distance; ScreenSpace is constant
// ════════════════════════════════════════════════════════════════════════════

/// `WorldScaled` scale = ref_distance / eye_distance — strictly decreasing as the
/// anchor recedes; `ScreenSpace` scale stays 1.0 at every distance (billboard).
#[test]
fn gate4_worldscaled_shrinks_screenspace_constant() {
    let depths = [-5.0_f32, -10.0, -20.0, -40.0];

    // ScreenSpace: scale == 1.0 at every depth.
    {
        let mut h = ProjOnce::new();
        let mut anchors = Vec::new();
        for &z in &depths {
            anchors.push(h.spawn_world_anchor(
                WorldTarget::WorldPos([0.0, 0.0, z]),
                [0.0, 0.0, 0.0],
                WorldScaleMode::ScreenSpace,
            ));
        }
        h.run();
        for (a, &z) in anchors.iter().zip(&depths) {
            approx(h.projection(*a).scale, 1.0, EPS_PX, &format!("ScreenSpace scale constant at z={z}"));
        }
    }

    // WorldScaled: scale strictly decreasing in distance, and equal to
    // ref_distance/|z| (camera at origin ⇒ eye-distance == |z| on the −Z axis).
    {
        let mut h = ProjOnce::new();
        let ref_dist = 10.0_f32;
        let mut anchors = Vec::new();
        for &z in &depths {
            let mut a = UiWorldAnchor {
                target: WorldTarget::WorldPos([0.0, 0.0, z]),
                offset: [0.0, 0.0, 0.0],
                scale_mode: WorldScaleMode::WorldScaled,
                ref_distance: ref_dist,
                ..Default::default()
            };
            a.billboard = true;
            anchors.push(h.spawn_anchor_value(a));
        }
        h.run();

        let mut prev = f32::INFINITY;
        for (a, &z) in anchors.iter().zip(&depths) {
            let s = h.projection(*a).scale;
            let expected = ref_dist / z.abs();
            approx(s, expected, 1.0e-3, &format!("WorldScaled scale == ref/dist at z={z}"));
            assert!(s < prev, "WorldScaled scale strictly decreases with distance (z={z}: {s} !< {prev})");
            assert!(s > 0.0 && s.is_finite(), "scale finite > 0");
            prev = s;
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 5 — distance fade is monotonic non-increasing in [0, 1]
// ════════════════════════════════════════════════════════════════════════════

/// The fade factor is in `[0, 1]`, equals 1.0 at/below `fade_near`, 0.0 at/beyond
/// `fade_far`, and is non-increasing across a sweep of distances.
#[test]
fn gate5_fade_monotonic_in_unit_interval() {
    let mut h = ProjOnce::new();
    let fade_near = 5.0_f32;
    let fade_far = 50.0_f32;
    // Depths spanning below-near, within, and beyond-far (camera at origin ⇒
    // eye-distance == |z| on the −Z axis).
    let depths = [-2.0_f32, -5.0, -10.0, -25.0, -50.0, -80.0];
    let mut anchors = Vec::new();
    for &z in &depths {
        let a = UiWorldAnchor {
            target: WorldTarget::WorldPos([0.0, 0.0, z]),
            fade_by_distance: true,
            fade_near,
            fade_far,
            ..Default::default()
        };
        anchors.push(h.spawn_anchor_value(a));
    }
    h.run();

    let mut prev = f32::INFINITY;
    for (a, &z) in anchors.iter().zip(&depths) {
        let fade = h.projection(*a).fade;
        let dist = z.abs();
        assert!((0.0..=1.0).contains(&fade), "fade in [0,1] at dist={dist}: {fade}");
        assert!(fade <= prev + 1.0e-6, "fade non-increasing in distance (dist={dist}: {fade} > {prev})");
        prev = fade;
    }

    // Endpoints: at/below near ⇒ 1.0; at/beyond far ⇒ 0.0.
    approx(h.projection(anchors[0]).fade, 1.0, 1.0e-4, "fade==1 below fade_near");
    approx(h.projection(anchors[1]).fade, 1.0, 1.0e-4, "fade==1 at fade_near");
    approx(h.projection(*anchors.last().unwrap()).fade, 0.0, 1.0e-4, "fade==0 beyond fade_far");

    // A mid-distance fade matches the hand formula
    // saturate((far - dist)/(far - near)). dist=25: (50-25)/45 = 0.5555…
    let mid = h.projection(anchors[3]).fade;
    approx(mid, (fade_far - 25.0) / (fade_far - fade_near), 1.0e-4, "fade lerp at dist=25");
}

/// Fade OFF ⇒ factor is always 1.0 regardless of distance.
#[test]
fn gate5_fade_off_is_unity() {
    let mut h = ProjOnce::new();
    let a = h.spawn_world_anchor(WorldTarget::WorldPos([0.0, 0.0, -90.0]), [0.0, 0.0, 0.0], WorldScaleMode::ScreenSpace);
    h.run();
    approx(h.projection(a).fade, 1.0, EPS_PX, "fade off ⇒ unity even far away");
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 6 — show/hide via HoveredWorldEntity (EnableTag O(1))
// ════════════════════════════════════════════════════════════════════════════

/// Setting `HoveredWorldEntity(Some(e))` shows the root tracking `e` (clears
/// `UiWorldHidden`) and hides every other entity-tracking root (sets the bit);
/// changing the hover swaps them; `None` hides all.
#[test]
fn gate6_hover_shows_one_hides_the_rest() {
    let mut h = VisHarness::new();

    let e_a = h.spawn_scene_entity();
    let e_b = h.spawn_scene_entity();
    let e_c = h.spawn_scene_entity();
    let anc_a = h.spawn_entity_anchor(e_a);
    let anc_b = h.spawn_entity_anchor(e_b);
    let anc_c = h.spawn_entity_anchor(e_c);
    // A fixed WorldPos anchor is NOT hover-driven; it must be left untouched.
    let fixed = h.spawn_world_pos_anchor([0.0, 0.0, -10.0]);

    // First run with None hovered ⇒ every entity-tracking root hidden, fixed left.
    h.set_hover(None);
    h.run();
    assert!(h.world.is_enabled::<UiWorldHidden>(anc_a), "None hides A");
    assert!(h.world.is_enabled::<UiWorldHidden>(anc_b), "None hides B");
    assert!(h.world.is_enabled::<UiWorldHidden>(anc_c), "None hides C");
    assert!(!h.world.is_enabled::<UiWorldHidden>(fixed), "fixed WorldPos root never hover-hidden");

    // Hover B ⇒ B shown, A & C hidden.
    h.set_hover(Some(e_b));
    h.run();
    assert!(!h.world.is_enabled::<UiWorldHidden>(anc_b), "hovering B shows B");
    assert!(h.world.is_enabled::<UiWorldHidden>(anc_a), "A hidden while B hovered");
    assert!(h.world.is_enabled::<UiWorldHidden>(anc_c), "C hidden while B hovered");
    assert!(!h.world.is_enabled::<UiWorldHidden>(fixed), "fixed still untouched");

    // Switch hover to A ⇒ A shown, B hidden again (the prior un-hidden one).
    h.set_hover(Some(e_a));
    h.run();
    assert!(!h.world.is_enabled::<UiWorldHidden>(anc_a), "hovering A shows A");
    assert!(h.world.is_enabled::<UiWorldHidden>(anc_b), "prior-hovered B is re-hidden");
    assert!(h.world.is_enabled::<UiWorldHidden>(anc_c), "C still hidden");

    // None again ⇒ all hidden.
    h.set_hover(None);
    h.run();
    assert!(h.world.is_enabled::<UiWorldHidden>(anc_a), "None re-hides A");
    assert!(h.world.is_enabled::<UiWorldHidden>(anc_b), "None re-hides B");
    assert!(h.world.is_enabled::<UiWorldHidden>(anc_c), "None re-hides C");
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 7 — 0%-overhead gates
// ════════════════════════════════════════════════════════════════════════════

/// No `UiWorldAnchor` in the world ⇒ the project system writes NOTHING. We assert
/// it leaves the (unrelated) `ViewUniform` and a probe entity untouched, and that
/// `query_entities(&[UiWorldAnchor])` is empty so the per-root loop never runs.
#[test]
fn gate7_no_anchor_zero_work() {
    let mut h = ProjOnce::new();
    // A non-anchor entity (just a UiRoot) — the project system must not touch it.
    let plain = h.spawn_plain_root();

    // The anchor set is empty.
    assert!(
        h.world.query_entities(&[UiWorldAnchor::component_id()]).is_empty(),
        "no UiWorldAnchor entities ⇒ empty anchor set ⇒ the per-root loop body never runs"
    );

    // Running the system is a no-op w.r.t. components: the plain root has no
    // UiWorldProjection at all.
    h.run();
    assert!(
        h.world.get_component::<UiWorldProjection>(plain).is_none(),
        "the project system inserts no projection on a non-anchor entity"
    );
}

/// A STATIC anchor + a STILL camera does NO re-projection: across steady frames
/// `UiWorldProjection`'s changed-tick stops advancing once the value settles
/// (the `set_if_neq` Changed-gate). Driven through a real schedule so the tick
/// window is preserved.
#[test]
fn gate7_static_anchor_still_camera_no_reprojection() {
    let mut h = SchedHarness::new();
    let anchor = h.spawn_world_pos_anchor([0.0, 0.0, -10.0]);

    // Warm: run a few frames to let everything settle (the first frame writes the
    // projection; thereafter the value is identical).
    h.run();
    h.run();
    let settled = h.projection_changed_tick(anchor);
    // The gate is only meaningful if the projection was actually written at least
    // once (a non-vacuous changed tick). A WorldPos anchor in front of the camera
    // is visible, so the first projection writes a real value.
    assert!(settled.is_some(), "the projection was written at least once (gate is non-vacuous)");
    assert!(h.projection(anchor).visible, "the static anchor is visible (a real, settled projection)");

    // Several MORE still frames must not re-bump the changed tick (the value is
    // identical ⇒ set_if_neq suppresses the write).
    for i in 0..5 {
        h.run();
        let t = h.projection_changed_tick(anchor);
        assert_eq!(
            t, settled,
            "frame {i}: a static anchor + still camera must NOT re-write UiWorldProjection (Changed-gate)"
        );
    }
}

/// The Changed-gate at the pure-value level: re-projecting an IDENTICAL static
/// anchor leaves its `UiWorldProjection` byte-identical (so `set_if_neq` writes
/// nothing). This is the cull-tag-independent half of the 0%-claim.
#[test]
fn gate7_reprojection_is_value_stable() {
    let mut h = ProjOnce::new();
    let a = h.spawn_world_anchor(WorldTarget::WorldPos([2.0, 1.0, -7.0]), [0.0, 0.0, 0.0], WorldScaleMode::ScreenSpace);
    h.run();
    let first = h.projection(a);
    h.run();
    let second = h.projection(a);
    assert_eq!(first, second, "re-projecting an identical static anchor yields a byte-identical projection");
}

// ════════════════════════════════════════════════════════════════════════════
// Harness: ProjOnce — project system driven once per `run` via `run_system`
// ════════════════════════════════════════════════════════════════════════════

/// A world with `ViewUniform` + `UiViewport` seeded for the camera-at-origin view,
/// driving `ui_world_project_system` once per `run` (no tick window needed for the
/// FIRST projection / cull assertions).
struct ProjOnce {
    world: EcsMaster,
}

impl ProjOnce {
    fn new() -> Self {
        let mut world = EcsMaster::new();
        world.insert_resource(ViewUniform {
            view_proj: origin_view_proj(),
            inv_view: Mat4::IDENTITY,
            camera_pos: boyko_math::Vec4::new(0.0, 0.0, 0.0, 1.0),
            cam_forward: boyko_math::Vec4::new(0.0, 0.0, -1.0, 0.0),
            cam_right: boyko_math::Vec4::new(1.0, 0.0, 0.0, 0.0),
            cam_up: boyko_math::Vec4::new(0.0, 1.0, 0.0, 0.0),
            fov_y: FOV_Y,
            aspect: ASPECT,
            near: NEAR,
            far: FAR,
        });
        world.insert_resource(UiViewport {
            width: VP_W,
            height: VP_H,
            scale_factor: 1.0,
            generation: 0,
        });
        world.insert_resource(UiWorldScratch::default());
        Self { world }
    }

    fn run(&mut self) {
        self.world.run_system(ui_world_project_system);
    }

    fn spawn_anchor_value(&mut self, anchor: UiWorldAnchor) -> Entity {
        spawn_anchor(&mut self.world, anchor)
    }

    fn spawn_world_anchor(&mut self, target: WorldTarget, offset: [f32; 3], scale_mode: WorldScaleMode) -> Entity {
        let anchor = UiWorldAnchor {
            target,
            offset,
            scale_mode,
            ..Default::default()
        };
        spawn_anchor(&mut self.world, anchor)
    }

    fn spawn_plain_root(&mut self) -> Entity {
        spawn_via_commands(&mut self.world, move |cmds| {
            let mut ec = cmds.spawn(UiRoot);
            ec.insert(definite_root_layout());
            ec.insert(ComputedRect::default());
            ec.id()
        })
    }

    fn projection(&self, e: Entity) -> UiWorldProjection {
        *self.world.get_component::<UiWorldProjection>(e).expect("anchor has a UiWorldProjection")
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Harness: SchedHarness — propagate → project → layout in a real Schedule
// ════════════════════════════════════════════════════════════════════════════

/// A world wiring `propagate_transforms` → `ui_world_project_system` →
/// `ui_layout_discovery` → `ui_layout_apply` in the documented order, run as a
/// real `Schedule` (so the `(last_run, this_run]` change windows survive frames).
/// `resolve_active_camera` is NOT in the schedule — the `ViewUniform` is set
/// directly to the camera-at-origin view (a still camera, the Changed-gate
/// vehicle), exactly the gates_camera "set the view directly" pattern.
struct SchedHarness {
    world: EcsMaster,
    schedule: Schedule,
    ticker: Schedule,
}

/// A no-op exclusive system whose only effect is advancing the world change tick
/// (the `gates_determinism::Scene::ticker` precedent).
fn noop_exclusive(_world: &mut EcsMaster) {}

impl SchedHarness {
    fn new() -> Self {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let mut world = EcsMaster::new();
        // Camera-at-origin view (still).
        world.insert_resource(ViewUniform {
            view_proj: origin_view_proj(),
            inv_view: Mat4::IDENTITY,
            camera_pos: boyko_math::Vec4::new(0.0, 0.0, 0.0, 1.0),
            cam_forward: boyko_math::Vec4::new(0.0, 0.0, -1.0, 0.0),
            cam_right: boyko_math::Vec4::new(1.0, 0.0, 0.0, 0.0),
            cam_up: boyko_math::Vec4::new(0.0, 1.0, 0.0, 0.0),
            fov_y: FOV_Y,
            aspect: ASPECT,
            near: NEAR,
            far: FAR,
        });
        world.insert_resource(UiViewport { width: VP_W, height: VP_H, scale_factor: 1.0, generation: 0 });
        world.insert_resource(UiSafeArea::default());
        world.insert_resource(LayoutScratch::with_seeds());
        world.insert_resource(UiWorldScratch::default());
        world.insert_resource(boyko_scene::TransformPropagationScratch::default());

        let mut builder = ScheduleBuilder::new(pool);
        let prop = builder.add_system(propagate_transforms).key();
        let proj = builder.add_system(ui_world_project_system).after(prop).key();
        let disc = builder.add_system(ui_layout_discovery).after(proj).key();
        builder.add_system(ui_layout_apply).after(disc);
        let schedule = builder.build(&mut world);

        // A SEPARATE ticker schedule (a noop exclusive system) advances the world
        // change tick WITHOUT running propagation — the `gates_determinism::Scene`
        // precedent. `propagate_transforms` dirty-scans `Changed<Transform>` over
        // its own recorded `last_run` (a half-open `(last_run, this_run]` window),
        // and an `Added<Transform>` written at Tick::ZERO is NOT `> last_run==0`.
        // Lifting the tick off ZERO before any spawn (while propagation's
        // `last_run` stays 0) makes the spawn's Added land strictly inside the
        // first propagate window. The main schedule must NOT pre-run for the same
        // reason: running propagation first would bump its `last_run` past the
        // later spawn tick.
        let mut tbuilder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
        tbuilder.add_system(noop_exclusive);
        let mut ticker = tbuilder.build(&mut world);
        ticker.run(&mut world); // lift the tick off ZERO before any spawn

        Self { world, schedule, ticker }
    }

    /// Advances the world change tick (without running the pipeline) so a
    /// post-run structural/Transform edit lands in the next run's window.
    fn tick(&mut self) {
        self.ticker.run(&mut self.world);
    }

    fn run(&mut self) {
        self.schedule.run(&mut self.world);
    }

    /// Spawns a scene entity with a `Transform` at `t` + a `GlobalTransform`
    /// (propagation recomputes the global from the local each run).
    fn spawn_target(&mut self, t: Vec3) -> Entity {
        spawn_via_commands(&mut self.world, move |cmds| {
            let mut ec = cmds.spawn(Transform::from_translation(t));
            ec.insert(GlobalTransform::default());
            ec.id()
        })
    }

    fn set_target_translation(&mut self, e: Entity, t: Vec3) {
        if let Some(mut tr) = self.world.get_component_mut::<Transform>(e) {
            tr.translation = t;
        }
    }

    fn spawn_entity_anchor(&mut self, target: Entity, offset: [f32; 3]) -> Entity {
        let anchor = UiWorldAnchor {
            target: WorldTarget::EntityAnchor(target),
            offset,
            ..Default::default()
        };
        spawn_anchor(&mut self.world, anchor)
    }

    fn spawn_world_pos_anchor(&mut self, p: [f32; 3]) -> Entity {
        let anchor = UiWorldAnchor { target: WorldTarget::WorldPos(p), ..Default::default() };
        spawn_anchor(&mut self.world, anchor)
    }

    fn projection(&self, e: Entity) -> UiWorldProjection {
        *self.world.get_component::<UiWorldProjection>(e).expect("anchor has a UiWorldProjection")
    }

    fn projection_changed_tick(&self, e: Entity) -> Option<u32> {
        self.world
            .get_component_changed_tick(e, UiWorldProjection::component_id())
            .map(|t| t.get())
    }

    fn rect(&self, e: Entity) -> ComputedRect {
        *self.world.get_component::<ComputedRect>(e).expect("anchor root has a ComputedRect")
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Harness: VisHarness — drives ui_world_visibility_system from HoveredWorldEntity
// ════════════════════════════════════════════════════════════════════════════

/// A world driving `ui_world_visibility_system` per `run`. The visibility system
/// reads `HoveredWorldEntity` + its private `UiWorldHoverState` Changed-gate, so
/// both resources are seeded; `run` invokes the system once (a hover CHANGE each
/// `set_hover` makes the gate fire).
struct VisHarness {
    world: EcsMaster,
}

impl VisHarness {
    fn new() -> Self {
        let mut world = EcsMaster::new();
        world.insert_resource(HoveredWorldEntity::default());
        world.insert_resource(UiWorldHoverState::default());
        world.insert_resource(UiWorldScratch::default());
        Self { world }
    }

    fn run(&mut self) {
        self.world.run_system(ui_world_visibility_system);
    }

    fn set_hover(&mut self, e: Option<Entity>) {
        *self.world.resource_mut::<HoveredWorldEntity>() = HoveredWorldEntity(e);
    }

    /// A bare scene entity to serve as a hover target id.
    fn spawn_scene_entity(&mut self) -> Entity {
        spawn_via_commands(&mut self.world, move |cmds| {
            cmds.spawn(Transform::from_translation(Vec3::ZERO)).id()
        })
    }

    fn spawn_entity_anchor(&mut self, target: Entity) -> Entity {
        let anchor = UiWorldAnchor {
            target: WorldTarget::EntityAnchor(target),
            ..Default::default()
        };
        spawn_anchor(&mut self.world, anchor)
    }

    fn spawn_world_pos_anchor(&mut self, p: [f32; 3]) -> Entity {
        let anchor = UiWorldAnchor { target: WorldTarget::WorldPos(p), ..Default::default() };
        spawn_anchor(&mut self.world, anchor)
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Shared spawn helpers (the Phase-11/19 Arc<Mutex<..>> probe pattern)
// ════════════════════════════════════════════════════════════════════════════

/// A definite-size root layout so `layout_root` measures a real box (and the
/// projected origin seeds a non-degenerate rect).
fn definite_root_layout() -> UiLayout {
    UiLayout {
        width: Unit::Px(120.0),
        height: Unit::Px(40.0),
        ..Default::default()
    }
}

/// Spawns a world-anchored UI ROOT carrying `anchor` (+ a definite `UiLayout`,
/// `UiRoot`, `ComputedRect`). `UiWorldAnchor`'s `#[require(UiWorldProjection)]`
/// auto-inserts the projection column.
fn spawn_anchor(world: &mut EcsMaster, anchor: UiWorldAnchor) -> Entity {
    spawn_via_commands(world, move |cmds| {
        let mut ec = cmds.spawn(anchor);
        ec.insert(definite_root_layout());
        ec.insert(UiRoot);
        ec.insert(ComputedRect::default());
        ec.id()
    })
}

/// The established `Commands` + `Arc<Mutex<Option<Entity>>>` probe: spawn inside a
/// one-shot system, harvest the handle after the apply window.
fn spawn_via_commands<F>(world: &mut EcsMaster, f: F) -> Entity
where
    F: FnOnce(&mut Commands) -> Entity + Send + Sync + 'static,
{
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    let f = Mutex::new(Some(f));
    world.run_system(move |mut cmds: Commands| {
        let f = f.lock().unwrap().take().expect("spawn closure runs once");
        let e = f(&mut cmds);
        *probe.lock().unwrap() = Some(e);
    });
    let e = sink.lock().unwrap().expect("spawned handle");
    assert!(world.has_entity(e), "spawned entity is live after apply");
    e
}
