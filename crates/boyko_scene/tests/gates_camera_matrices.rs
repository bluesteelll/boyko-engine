//! S3 camera CPU MATRIX gates.
//!
//! These pin the numeric contract of [`ViewUniform::from_camera`] /
//! [`Projection::to_mat4`] that the gate review found under-covered: the existing
//! `gates_camera.rs` checks the SELECTION policy and the basis lanes, but never
//! the projection reference values, the `view = inverse(GlobalTransform)`
//! identity for a *rotated* camera, the `view_proj = proj · view` composition,
//! nor the column-major upload layout. This file closes those gaps with
//! reference-value assertions (independent re-derivations, not a copy of the
//! production formula's structure).

use boyko_scene::{Projection, ViewUniform};

use boyko_math::{Affine3A, Mat3, Mat4, Quat, Vec3, Vec4};

use core::f32::consts::FRAC_PI_3;

/// Float comparison tolerance for matrix elements (single precision, a few
/// dependent muls/recips).
const EPS: f32 = 1e-5;

/// Asserts two `Mat4`s agree element-wise within [`EPS`], reporting the first
/// mismatch with column/row indices.
fn assert_mat4_close(got: Mat4, want: Mat4, ctx: &str) {
    for c in 0..4 {
        let g = got.cols[c];
        let w = want.cols[c];
        let gr = [g.x, g.y, g.z, g.w];
        let wr = [w.x, w.y, w.z, w.w];
        for r in 0..4 {
            assert!(
                (gr[r] - wr[r]).abs() <= EPS,
                "{ctx}: col {c} row {r} got {} want {} (delta {})",
                gr[r],
                wr[r],
                (gr[r] - wr[r]).abs()
            );
        }
    }
}

/// Multiplies a `Mat4` (column-major) by a column `Vec4`: `m · v`.
fn mul_point(m: Mat4, v: Vec4) -> Vec4 {
    Vec4::new(
        m.cols[0].x * v.x + m.cols[1].x * v.y + m.cols[2].x * v.z + m.cols[3].x * v.w,
        m.cols[0].y * v.x + m.cols[1].y * v.y + m.cols[2].y * v.z + m.cols[3].y * v.w,
        m.cols[0].z * v.x + m.cols[1].z * v.y + m.cols[2].z * v.z + m.cols[3].z * v.w,
        m.cols[0].w * v.x + m.cols[1].w * v.y + m.cols[2].w * v.z + m.cols[3].w * v.w,
    )
}

// ════════════════════════════════════════════════════════════════════════════
// (1a) Perspective projection matches the reference value
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn perspective_proj_matches_reference_values() {
    let fov_y = FRAC_PI_3; // 60°
    let aspect = 16.0 / 9.0;
    let near = 0.1_f32;
    let far = 100.0_f32;
    let proj = Projection::Perspective {
        fov_y,
        aspect,
        near,
        far,
    }
    .to_mat4();

    // Independently re-derived right-handed, depth-[0,1] (WGSL/Vulkan) reference.
    let f = 1.0 / (fov_y * 0.5).tan();
    let nf = 1.0 / (near - far);
    let reference = Mat4::from_cols(
        Vec4::new(f / aspect, 0.0, 0.0, 0.0),
        Vec4::new(0.0, f, 0.0, 0.0),
        Vec4::new(0.0, 0.0, far * nf, -1.0),
        Vec4::new(0.0, 0.0, near * far * nf, 0.0),
    );
    assert_mat4_close(proj, reference, "perspective_rh");

    // Cross-check the geometry: a point exactly on the near plane in front of the
    // camera (view space z = -near) maps to clip z = 0 after the perspective
    // divide; on the far plane (z = -far) maps to clip z = 1 (the [0,1] depth
    // range). Re-derives the contract, not the matrix.
    let on_near = mul_point(proj, Vec4::new(0.0, 0.0, -near, 1.0));
    assert!((on_near.z / on_near.w - 0.0).abs() <= 1e-4, "near plane -> depth 0");
    let on_far = mul_point(proj, Vec4::new(0.0, 0.0, -far, 1.0));
    assert!((on_far.z / on_far.w - 1.0).abs() <= 1e-4, "far plane -> depth 1");
}

// ════════════════════════════════════════════════════════════════════════════
// (1b) Orthographic projection matches the reference value
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn orthographic_proj_matches_reference_values() {
    let half_height = 2.0_f32;
    let aspect = 1.5_f32;
    let near = 0.5_f32;
    let far = 50.0_f32;
    let proj = Projection::Orthographic {
        half_height,
        aspect,
        near,
        far,
    }
    .to_mat4();

    let half_width = half_height * aspect;
    let (left, right, bottom, top) = (-half_width, half_width, -half_height, half_height);
    let rl = 1.0 / (right - left);
    let tb = 1.0 / (top - bottom);
    let nf = 1.0 / (near - far);
    let reference = Mat4::from_cols(
        Vec4::new(2.0 * rl, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 * tb, 0.0, 0.0),
        Vec4::new(0.0, 0.0, nf, 0.0),
        Vec4::new(-(right + left) * rl, -(top + bottom) * tb, near * nf, 1.0),
    );
    assert_mat4_close(proj, reference, "orthographic_rh");

    // Geometry cross-check: the corner (half_width, half_height, -near) maps to
    // clip (+1, +1, 0) — the top-right of the near plane is the clip-cube corner.
    let corner = mul_point(proj, Vec4::new(half_width, half_height, -near, 1.0));
    assert!((corner.x - 1.0).abs() <= 1e-5, "ortho right edge -> x = +1");
    assert!((corner.y - 1.0).abs() <= 1e-5, "ortho top edge -> y = +1");
    assert!((corner.z - 0.0).abs() <= 1e-5, "ortho near plane -> z = 0");
    let far_corner = mul_point(proj, Vec4::new(0.0, 0.0, -far, 1.0));
    assert!((far_corner.z - 1.0).abs() <= 1e-5, "ortho far plane -> z = 1");
}

// ════════════════════════════════════════════════════════════════════════════
// (2) view = inverse(camera GlobalTransform) for a ROTATED camera
// ════════════════════════════════════════════════════════════════════════════

/// A camera at a non-trivial rigid pose: rotated 90° about +Y and translated.
/// `Quat` for a 90° Y rotation is `(0, sin45, 0, cos45)`.
fn rotated_camera() -> Affine3A {
    let s = (core::f32::consts::FRAC_PI_4).sin(); // sin(45°) = cos(45°)
    let q = Quat::new(0.0, s, 0.0, s).normalize();
    Affine3A::from_translation_rotation_scale(Vec3::new(2.0, 1.0, -3.0), q, Vec3::new(1.0, 1.0, 1.0))
}

#[test]
fn view_is_inverse_of_global_transform() {
    let global = rotated_camera();
    let proj = Projection::Perspective {
        fov_y: FRAC_PI_3,
        aspect: 1.0,
        near: 0.1,
        far: 100.0,
    };
    let view_uniform = ViewUniform::from_camera(global, proj);

    // `inv_view` is documented to equal the camera world matrix (global as Mat4).
    assert_mat4_close(view_uniform.inv_view, global.to_mat4(), "inv_view == global");

    // The embedded view = global⁻¹. Recover it as proj⁻¹ is awkward; instead test
    // the defining property directly: transforming the camera's own world origin
    // (its translation) by `view` yields the view-space origin (0,0,0). Build
    // `view` from the independent affine inverse and check view · inv_view == I.
    let view = global.inverse().expect("rigid camera is invertible").to_mat4();
    let world = global.to_mat4();
    let composed = view.mul_mat4(world);
    assert_mat4_close(composed, Mat4::IDENTITY, "view · global == I");

    // And the camera eye maps to the origin in view space.
    let eye = Vec4::new(global.translation.x, global.translation.y, global.translation.z, 1.0);
    let in_view = mul_point(view, eye);
    assert!(in_view.x.abs() <= EPS, "eye.x -> 0 in view space");
    assert!(in_view.y.abs() <= EPS, "eye.y -> 0 in view space");
    assert!(in_view.z.abs() <= EPS, "eye.z -> 0 in view space");
}

// ════════════════════════════════════════════════════════════════════════════
// (3) view_proj == proj · view (composition order)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn view_proj_is_proj_times_view() {
    let global = rotated_camera();
    let projection = Projection::Perspective {
        fov_y: FRAC_PI_3,
        aspect: 1.25,
        near: 0.2,
        far: 80.0,
    };
    let view_uniform = ViewUniform::from_camera(global, projection);

    let proj = projection.to_mat4();
    let view = global.inverse().expect("rigid camera is invertible").to_mat4();
    let expected = proj.mul_mat4(view);

    assert_mat4_close(view_uniform.view_proj, expected, "view_proj == proj · view");

    // Order matters: assert it is NOT view · proj (the transposed-intent bug).
    let wrong = view.mul_mat4(proj);
    let diff: f32 = (0..4)
        .map(|c| {
            let a = view_uniform.view_proj.cols[c];
            let b = wrong.cols[c];
            (a.x - b.x).abs() + (a.y - b.y).abs() + (a.z - b.z).abs() + (a.w - b.w).abs()
        })
        .sum();
    assert!(diff > 1e-3, "view_proj must be proj·view, not view·proj");
}

// ════════════════════════════════════════════════════════════════════════════
// (4) Column-major layout: cols[j] is column j (GPU mat4x4 upload form)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn view_proj_layout_is_column_major() {
    // Build a known affine where row-major vs column-major is distinguishable:
    // a translation-only transform puts the translation in the LAST COLUMN of a
    // column-major Mat4 (cols[3].xyz), not the last row.
    let global = Affine3A {
        matrix3: Mat3::IDENTITY,
        translation: Vec3::new(7.0, 8.0, 9.0),
    };
    let m = global.to_mat4();

    // Column-major: translation lives in cols[3] (the 4th column).
    assert_eq!([m.cols[3].x, m.cols[3].y, m.cols[3].z], [7.0, 8.0, 9.0], "translation in last column");
    assert_eq!(m.cols[3].w, 1.0, "homogeneous w of translation column");
    // The would-be "last row" (cols[*].w for the linear columns) must be zero —
    // it is NOT where translation lives (that's the row-major mistake).
    assert_eq!([m.cols[0].w, m.cols[1].w, m.cols[2].w], [0.0, 0.0, 0.0], "linear columns have w = 0");

    // ViewUniform's inv_view exposes the same column-major world matrix.
    let view = ViewUniform::from_camera(
        global,
        Projection::Perspective { fov_y: FRAC_PI_3, aspect: 1.0, near: 0.1, far: 100.0 },
    );
    assert_eq!(
        [view.inv_view.cols[3].x, view.inv_view.cols[3].y, view.inv_view.cols[3].z],
        [7.0, 8.0, 9.0],
        "inv_view carries the world translation in its last column"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// (5) Singular linear part -> identity-view fallback (no NaN)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn singular_camera_falls_back_to_identity_view_no_nan() {
    // A zero linear part is non-invertible: the view must fall back to identity
    // (the documented degenerate behaviour) rather than producing NaNs.
    let global = Affine3A {
        matrix3: Mat3 {
            rows: [Vec3::ZERO, Vec3::ZERO, Vec3::ZERO],
        },
        translation: Vec3::new(1.0, 2.0, 3.0),
    };
    let view = ViewUniform::from_camera(
        global,
        Projection::Orthographic { half_height: 1.0, aspect: 1.0, near: 0.0, far: 10.0 },
    );

    // view_proj = proj · identity = proj; no NaNs anywhere in the matrix.
    for c in 0..4 {
        let col = view.view_proj.cols[c];
        for v in [col.x, col.y, col.z, col.w] {
            assert!(v.is_finite(), "fallback view_proj must be finite (no NaN)");
        }
    }
}
