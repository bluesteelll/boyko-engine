//! GUI P7b — `boyko_scene::camera::camera_ray` unit tests (HEADLESS, NO GPU).
//!
//! Test matrix (GUI-P7B-PLAN §4, camera_ray 6-8):
//!  6. ROUND-TRIP camera_ray ↔ project (THE correctness anchor): project a world
//!     point in front of a perspective camera to a pixel, then assert the ray from
//!     that pixel passes through the point (closest point on ray ≈ point, t > 0).
//!  7. center pixel (px=w/2, py=h/2) → dir ≈ cam_forward.
//!  8. fov_y == 0 ortho arm: no panic / no NaN, finite ray.
//!
//! # Dependency note (test 6)
//!
//! `project_world_to_screen` lives in `boyko_ui`, which DEPENDS on `boyko_scene`;
//! calling it here would invert the dep edge. The clean option is to replicate the
//! projection math (NDC = clip/clip_w, y-flip, px = (ndc*0.5+0.5)*extent) directly
//! from `view.view_proj` in this test — it is the SAME formula `project_world_to_
//! screen` documents (`project.rs:91-113`), and the round-trip is exercised against
//! `camera_ray` (the function under test), so this is a true inverse check without
//! the dep cycle.

use boyko_math::{Affine3A, Quat, Vec3, Vec4};
use boyko_scene::camera::{Projection, ViewUniform, camera_ray};

const VP_W: f32 = 1280.0;
const VP_H: f32 = 720.0;
const FOV_Y: f32 = core::f32::consts::FRAC_PI_3; // 60°
const NEAR: f32 = 0.1;
const FAR: f32 = 1000.0;

/// A forward perspective view: camera at `eye`, looking down -z (identity
/// rotation), 60° fov, 16:9-ish aspect. Built through the real `from_camera` so
/// the view_proj + basis lanes are exactly what the engine produces.
fn forward_view(eye: Vec3) -> ViewUniform {
    let global = Affine3A::from_translation_rotation_scale(eye, Quat::IDENTITY, Vec3::ONE);
    let projection = Projection::Perspective {
        fov_y: FOV_Y,
        aspect: VP_W / VP_H,
        near: NEAR,
        far: FAR,
    };
    ViewUniform::from_camera(global, projection)
}

/// Projects a world point through a COLUMN-MAJOR `view_proj` to a logical pixel +
/// the clip-w (forward depth). Replicates `project_world_to_screen`'s formula
/// (the documented inverse of `camera_ray`) to avoid the boyko_ui dep cycle.
fn project_to_pixel(view: &ViewUniform, world: Vec3) -> (f32, f32, f32) {
    let clip = view
        .view_proj
        .mul_vec4(Vec4::new(world.x, world.y, world.z, 1.0));
    let inv_w = 1.0 / clip.w;
    let ndc_x = clip.x * inv_w;
    let ndc_y = clip.y * inv_w;
    let px = (ndc_x * 0.5 + 0.5) * VP_W;
    let py = (1.0 - (ndc_y * 0.5 + 0.5)) * VP_H; // +y-down flip
    (px, py, clip.w)
}

/// The perpendicular distance from `point` to the ray, and the parameter `t` of
/// the closest point. `dir` is unit, so `t = (point - origin)·dir`.
fn closest_on_ray(origin: Vec3, dir: Vec3, point: Vec3) -> (f32, f32) {
    let to_point = point - origin;
    let t = to_point.dot(dir);
    let closest = origin + dir * t;
    let perp = (point - closest).length();
    (t, perp)
}

// ════════════════════════════════════════════════════════════════════════════
// 6. ROUND-TRIP — the correctness anchor
// ════════════════════════════════════════════════════════════════════════════

/// For several world points in front of a perspective camera, projecting to a
/// pixel and casting `camera_ray` back from that pixel yields a ray whose closest
/// point to the world point coincides with it (perp ≈ 0) at a positive `t`.
#[test]
fn camera_ray_round_trips_through_projection() {
    let eye = Vec3::new(2.0, 1.0, 5.0);
    let view = forward_view(eye);

    // Points in front of the camera (the camera at z=5 looks toward -z, so points
    // must have a smaller z than the eye).
    let points = [
        Vec3::new(2.0, 1.0, 0.0),   // dead ahead
        Vec3::new(4.0, 2.0, -3.0),  // up-right and far
        Vec3::new(-1.0, -1.5, 1.0), // down-left, nearer
        Vec3::new(3.5, 0.0, -8.0),  // off to the side, far
    ];

    for p in points {
        let (px, py, clip_w) = project_to_pixel(&view, p);
        assert!(clip_w > 0.0, "the point {p:?} is in front of the camera (clip_w {clip_w} > 0)");

        let ray = camera_ray(&view, px, py, VP_W, VP_H);
        // Origin is the eye.
        assert!((ray.origin - eye).length() <= 1.0e-4, "ray origin is the eye for {p:?}");

        let (t, perp) = closest_on_ray(ray.origin, ray.dir, p);
        assert!(t > 0.0, "the point is ahead on the ray (t {t} > 0) for {p:?}");
        // The point lies on the ray to within a small tolerance (perp scales with
        // the eye→point range; use a relative-ish bound).
        let range = (p - eye).length();
        assert!(
            perp <= 1.0e-3 * range.max(1.0),
            "point {p:?} lies on the cast ray (perp {perp}, range {range})"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// 7. center pixel → dir ≈ cam_forward
// ════════════════════════════════════════════════════════════════════════════

/// The viewport-center pixel maps to a ray pointing along the camera forward.
#[test]
fn camera_ray_center_pixel_is_forward() {
    let view = forward_view(Vec3::new(0.0, 0.0, 5.0));
    let ray = camera_ray(&view, VP_W * 0.5, VP_H * 0.5, VP_W, VP_H);

    let fwd = view.cam_forward.xyz();
    let dot = ray.dir.dot(fwd);
    assert!(dot > 0.999_999, "center-pixel dir ≈ cam_forward (dot {dot})");
    // Sanity: forward is -z for the identity-rotation camera.
    assert!((fwd - Vec3::new(0.0, 0.0, -1.0)).length() <= 1.0e-5, "identity camera forward is -z");
}

/// An OFF-center pixel does NOT point along forward (proves test 7 is not vacuous
/// — the center mapping is special).
#[test]
fn camera_ray_offcenter_pixel_is_not_forward() {
    let view = forward_view(Vec3::ZERO);
    let ray = camera_ray(&view, VP_W * 0.25, VP_H * 0.25, VP_W, VP_H);
    let fwd = view.cam_forward.xyz();
    assert!(ray.dir.dot(fwd) < 0.999, "an off-center pixel deviates from forward");
    assert!(ray.dir.length() > 0.999 && ray.dir.length() < 1.001, "dir is unit-length");
}

// ════════════════════════════════════════════════════════════════════════════
// 8. ortho arm (fov_y == 0): finite, no panic / NaN
// ════════════════════════════════════════════════════════════════════════════

/// The ortho arm (the `fov_y == 0.0` sentinel) produces a finite, non-NaN ray
/// across the viewport. Documented-approximate (does not match the marcher's
/// fixed-constant ortho fixture), but must not panic or emit non-finite values.
#[test]
fn camera_ray_ortho_arm_is_finite() {
    // An identity ortho-ish view: forward -z, right +x, up +y, fov_y == 0.
    let view = ViewUniform {
        fov_y: 0.0,
        ..ViewUniform::IDENTITY
    };
    assert_eq!(view.fov_y, 0.0, "the ortho sentinel is set");

    for (px, py) in [
        (0.0, 0.0),
        (VP_W, VP_H),
        (VP_W * 0.5, VP_H * 0.5),
        (VP_W * 0.25, VP_H * 0.75),
    ] {
        let ray = camera_ray(&view, px, py, VP_W, VP_H);
        assert!(ray.origin.is_finite(), "ortho origin finite at ({px},{py}): {:?}", ray.origin);
        assert!(ray.dir.is_finite(), "ortho dir finite at ({px},{py}): {:?}", ray.dir);
        // The ortho dir is the camera forward (unit -z).
        assert!((ray.dir - Vec3::new(0.0, 0.0, -1.0)).length() <= 1.0e-5, "ortho dir is forward");
    }
}
