//! GUI P7b — the cross-drift guard (test 9): `boyko_scene::camera::camera_ray`
//! must agree with the SDF marcher's host ray-gen
//! `boyko_rhi_vulkan::compute::composite_pixel_ray` for a matching perspective
//! camera.
//!
//! This is a PURE-CPU test: `composite_pixel_ray` is a HOST function (the shader
//! ray-gen mirror), so NO `VulkanContext` / GPU device is booted. It runs on any
//! machine, including CI without a GPU.
//!
//! # Why here
//!
//! `boyko_render` is the only crate that may name BOTH `boyko_scene::camera`
//! (`camera_ray`, `ViewUniform`) AND `boyko_rhi_vulkan::compute`
//! (`composite_pixel_ray`, `CompositeCamera`) — the low-level backend never
//! depends upward on the scene crate.
//!
//! # The agreement
//!
//! `host_camera_from_view(&view, w, h)` builds the marcher camera with
//! `aspect = w/h` (mirroring the production bridge in
//! `camera_drives_render_gpu.rs:270-280`). The marcher folds `+0.5` into the pixel
//! center, so `camera_ray` is fed `px+0.5`/`py+0.5`. For the center + 4 corners +
//! one off-center off-axis pixel in EACH quadrant, the origin and direction must
//! match within EPS (so a y-flip sign error or an aspect w/h-vs-h/w error cannot
//! hide behind symmetry).
//!
//! EPS = 1e-6: a forward perspective camera never produces a near-zero
//! pre-normalize `dir`, so `Vec3::normalize`'s zero-guard branch is never taken on
//! these pixels — it computes the identical sqrt/divide the marcher does. The only
//! divergence is f32 rounding in the basis-combine, well below 1e-6.

use boyko_math::{Affine3A, Quat, Vec3};
use boyko_rhi_vulkan::compute::{CompositeCamera, composite_pixel_ray};
use boyko_scene::camera::{Projection, ViewUniform, camera_ray};

/// EPS for the origin/direction agreement (W4 — justified in the module docs).
const EPS: f32 = 1.0e-6;

/// The host-mirror [`CompositeCamera`] for `view`, with `aspect = w/h` — verbatim
/// the production bridge `host_camera_from_view` (camera_drives_render_gpu.rs:270).
fn host_camera_from_view(view: &ViewUniform, w: u32, h: u32) -> CompositeCamera {
    let tan_half_fov = (view.fov_y * 0.5).tan();
    CompositeCamera::Perspective {
        eye: [view.camera_pos.x, view.camera_pos.y, view.camera_pos.z],
        forward: [view.cam_forward.x, view.cam_forward.y, view.cam_forward.z],
        right: [view.cam_right.x, view.cam_right.y, view.cam_right.z],
        up: [view.cam_up.x, view.cam_up.y, view.cam_up.z],
        tan_half_fov,
        aspect: (w as f32) / (h as f32),
    }
}

/// A unit quaternion for a rotation of `angle` (radians) about a UNIT `axis`:
/// `q = (axis * sin(θ/2), cos(θ/2))`.
fn quat_axis_angle(axis: Vec3, angle: f32) -> Quat {
    let half = angle * 0.5;
    let s = half.sin();
    Quat::new(axis.x * s, axis.y * s, axis.z * s, half.cos())
}

/// A forward perspective view (rotated camera, off-origin eye) so the basis lanes
/// are NON-trivial — a y-flip / aspect bug cannot hide behind a canonical basis.
fn rotated_forward_view(w: u32, h: u32) -> ViewUniform {
    let eye = Vec3::new(1.5, -2.0, 4.0);
    // A small yaw+pitch so right/up/forward are all off-axis (still orthonormal).
    let rot = quat_axis_angle(Vec3::new(0.0, 1.0, 0.0), 0.3)
        .mul(quat_axis_angle(Vec3::new(1.0, 0.0, 0.0), -0.2));
    let global = Affine3A::from_translation_rotation_scale(eye, rot, Vec3::ONE);
    let projection = Projection::Perspective {
        fov_y: core::f32::consts::FRAC_PI_3, // 60°
        aspect: (w as f32) / (h as f32),
        near: 0.1,
        far: 1000.0,
    };
    ViewUniform::from_camera(global, projection)
}

#[track_caller]
fn approx3(a: [f32; 3], b: Vec3, what: &str) {
    let d = ((a[0] - b.x).powi(2) + (a[1] - b.y).powi(2) + (a[2] - b.z).powi(2)).sqrt();
    assert!(d <= EPS, "{what}: marcher {a:?} vs camera_ray {b:?} (|Δ|={d})");
}

/// The pixel set: center + 4 corners + 1 off-center off-axis pixel in EACH of the
/// four quadrants. Distinct off-axis points in every quadrant defeat symmetry.
fn pixel_set(w: u32, h: u32) -> Vec<(u32, u32)> {
    vec![
        (w / 2, h / 2),               // center
        (0, 0),                       // TL corner
        (w - 1, 0),                   // TR corner
        (0, h - 1),                   // BL corner
        (w - 1, h - 1),               // BR corner
        (w / 4, h / 4),               // Q-TL off-axis
        (3 * w / 4, h / 4),           // Q-TR off-axis
        (w / 4, 3 * h / 4),           // Q-BL off-axis
        (3 * w / 4, 3 * h / 4),       // Q-BR off-axis
        (5 * w / 8, 3 * h / 8),       // an extra asymmetric off-axis sample
    ]
}

#[test]
fn camera_ray_matches_marcher_composite_pixel_ray() {
    let (w, h) = (1280u32, 720u32);
    let view = rotated_forward_view(w, h);
    let cam = host_camera_from_view(&view, w, h);

    for (px, py) in pixel_set(w, h) {
        let (ro_m, rd_m) = composite_pixel_ray(px, py, w, h, cam);
        // +0.5 to hit the marcher's pixel CENTER (it folds +0.5 in itself).
        let r = camera_ray(&view, px as f32 + 0.5, py as f32 + 0.5, w as f32, h as f32);

        approx3(ro_m, r.origin, &format!("origin at pixel ({px},{py})"));
        approx3(rd_m, r.dir, &format!("dir at pixel ({px},{py})"));
    }
}

/// A second aspect (square viewport) so an aspect = w/h vs h/w transposition (which
/// is identity at 1:1) cannot pass test 1 by accident — here it is 16:9 above and
/// a different non-square below; both must agree.
#[test]
fn camera_ray_matches_marcher_at_a_second_aspect() {
    let (w, h) = (800u32, 1000u32); // portrait (h > w): aspect < 1, distinct from 16:9
    let view = rotated_forward_view(w, h);
    let cam = host_camera_from_view(&view, w, h);

    for (px, py) in pixel_set(w, h) {
        let (ro_m, rd_m) = composite_pixel_ray(px, py, w, h, cam);
        let r = camera_ray(&view, px as f32 + 0.5, py as f32 + 0.5, w as f32, h as f32);
        approx3(ro_m, r.origin, &format!("portrait origin at ({px},{py})"));
        approx3(rd_m, r.dir, &format!("portrait dir at ({px},{py})"));
    }
}
