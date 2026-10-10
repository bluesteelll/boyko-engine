//! GATES for [`ViewUniform::from_camera`]'s degenerate-basis fallback — the host-side
//! closure of the first of the two device-NaN sources named in
//! `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §5.1.
//!
//! # The chain these gates pin
//!
//! A singular camera linear part makes `Mat3::mul_vec(axis).normalize()` return
//! [`Vec3::ZERO`] — a value that is **finite**, so it passes every host finiteness
//! assertion and is uploaded verbatim into the shared 80-byte camera block. On device,
//! `shaders/ray_gen.hlsli`'s perspective branch then computes
//! `dir = forward + right * sx + up * sy`, which for a zeroed basis is exactly `(0,0,0)`,
//! and `normalize(dir)` is `0/0` — undefined per GLSL.std.450, NaN in practice. That `rd`
//! reaches **twelve** translation units, `cluster_cull.hlsl` among them (it `#include`s
//! `ray_gen.hlsli` and calls `generate_ray` in both the flat and the hierarchical
//! froxel-AABB build).
//!
//! [`ray_gen_perspective_dir_mirror`] below is a bit-faithful Rust mirror of that shader
//! branch, written with the same raw `sqrt`-then-divide the shader's `normalize` and the
//! host oracle `composite_ray` both use — **no zero guard**, deliberately, because the
//! point of these gates is to observe what the shader would observe. It is the detector:
//! with the fallback removed from `from_camera`, [`the_degenerate_camera_no_longer_feeds_ray_gen_a_nan`]
//! goes RED on a NaN, which is the whole failure it exists to catch.

use boyko_math::{Affine3A, Mat3, Vec3};
use boyko_scene::{Projection, ViewUniform};

use core::f32::consts::FRAC_PI_3;

/// A square 60°-FOV perspective projection (matches `gates_camera.rs`'s own helper).
fn perspective(fov_y: f32) -> Projection {
    Projection::Perspective { fov_y, aspect: 1.0, near: 0.1, far: 100.0 }
}

/// A bit-faithful mirror of `shaders/ray_gen.hlsli`'s PERSPECTIVE `generate_ray` direction,
/// in the shader's own operation order, with the shader's own **unguarded** `normalize`
/// (raw `sqrt` then component divide — the same spelling
/// `boyko_rhi_vulkan::compute::composite_ray` uses so the host predicts the GPU bit-for-bit).
///
/// Returns `rd` for pixel-center NDC `(ndc_x, ndc_y)` under the published basis.
fn ray_gen_perspective_dir_mirror(
    view: &ViewUniform,
    ndc_x: f32,
    ndc_y: f32,
    tan_half_fov: f32,
    aspect: f32,
) -> [f32; 3] {
    let sx = ndc_x * aspect * tan_half_fov;
    let sy = ndc_y * tan_half_fov;
    let dir = [
        view.cam_forward.x + view.cam_right.x * sx + view.cam_up.x * sy,
        view.cam_forward.y + view.cam_right.y * sx + view.cam_up.y * sy,
        view.cam_forward.z + view.cam_right.z * sx + view.cam_up.z * sy,
    ];
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    [dir[0] / len, dir[1] / len, dir[2] / len]
}

/// A fully singular linear part: every local axis maps to the zero vector.
fn fully_singular() -> Affine3A {
    Affine3A { matrix3: Mat3::ZERO, translation: Vec3::new(1.0, 2.0, 3.0) }
}

/// A RANK-2 linear part: `right`(+X) and `up`(+Y) survive, `forward`(−Z) collapses to zero.
/// The case that makes a per-axis fallback unsafe and an all-or-nothing one mandatory.
fn zero_z_scale() -> Affine3A {
    Affine3A {
        matrix3: Mat3::from_diagonal(Vec3::new(1.0, 1.0, 0.0)),
        translation: Vec3::new(0.0, 0.0, 0.0),
    }
}

// ---- The fallback fires, and publishes a real basis ---------------------------------

#[test]
fn a_fully_singular_camera_publishes_the_canonical_basis_not_a_zero_one() {
    let view = ViewUniform::from_camera(fully_singular(), perspective(FRAC_PI_3));

    // The canonical triple — the same one `ViewUniform::IDENTITY` carries.
    assert_eq!([view.cam_right.x, view.cam_right.y, view.cam_right.z], [1.0, 0.0, 0.0]);
    assert_eq!([view.cam_up.x, view.cam_up.y, view.cam_up.z], [0.0, 1.0, 0.0]);
    assert_eq!([view.cam_forward.x, view.cam_forward.y, view.cam_forward.z], [0.0, 0.0, -1.0]);

    // The eye is still the camera's real translation — only the BASIS is substituted.
    assert_eq!(
        [view.camera_pos.x, view.camera_pos.y, view.camera_pos.z],
        [1.0, 2.0, 3.0],
        "the fallback replaces the basis, not the pose"
    );
}

#[test]
fn a_partially_singular_camera_falls_back_all_or_nothing() {
    let view = ViewUniform::from_camera(zero_z_scale(), perspective(FRAC_PI_3));

    // `forward` alone collapsed, but `right`/`up` are replaced too: a mixed triple is not
    // guaranteed to be a basis, and the substitution must keep the published one orthonormal.
    assert_eq!([view.cam_forward.x, view.cam_forward.y, view.cam_forward.z], [0.0, 0.0, -1.0]);
    assert_eq!([view.cam_right.x, view.cam_right.y, view.cam_right.z], [1.0, 0.0, 0.0]);
    assert_eq!([view.cam_up.x, view.cam_up.y, view.cam_up.z], [0.0, 1.0, 0.0]);
}

// ---- The detector: the shader's own arithmetic, on the published basis ---------------

/// The RED-mutation gate. Delete the fallback from `ViewUniform::from_camera` and this test
/// fails on a NaN `rd` — for BOTH degenerate shapes and at the pixel-CENTER ray, which is the
/// one ray a rank-2 basis kills even when the off-center ones survive.
#[test]
fn the_degenerate_camera_no_longer_feeds_ray_gen_a_nan() {
    let tan_half_fov = (FRAC_PI_3 * 0.5).tan();

    for (label, global) in [("fully singular", fully_singular()), ("zero-Z scale", zero_z_scale())] {
        let view = ViewUniform::from_camera(global, perspective(FRAC_PI_3));

        // The center pixel (ndc == 0) is the hard case: `sx == sy == 0`, so `dir` reduces to
        // `forward` alone and a zeroed forward yields `0/0` with nothing to mask it.
        for (ndc_x, ndc_y) in [(0.0_f32, 0.0_f32), (-1.0, -1.0), (1.0, -1.0), (0.5, 0.25)] {
            let rd = ray_gen_perspective_dir_mirror(&view, ndc_x, ndc_y, tan_half_fov, 1.0);
            assert!(
                rd[0].is_finite() && rd[1].is_finite() && rd[2].is_finite(),
                "{label}: ray_gen produced a non-finite rd {rd:?} at ndc ({ndc_x}, {ndc_y}) — \
                 the degenerate-basis fallback is not holding, and this NaN reaches every \
                 ray_gen.hlsli includer including cluster_cull.hlsl's froxel-AABB build"
            );
            let len_sq = rd[0] * rd[0] + rd[1] * rd[1] + rd[2] * rd[2];
            assert!(
                (len_sq - 1.0).abs() < 1e-5,
                "{label}: rd {rd:?} at ndc ({ndc_x}, {ndc_y}) is not unit (len² = {len_sq})"
            );
        }
    }
}

/// The same detector run against the UNGUARDED input, proving the assertion above is not
/// vacuous: a genuinely zeroed basis DOES make this mirror produce a NaN, so the test that
/// asserts finiteness is testing something.
#[test]
fn the_ray_gen_mirror_really_does_nan_on_a_zeroed_basis() {
    let mut zeroed = ViewUniform::IDENTITY;
    zeroed.cam_forward = boyko_math::Vec4::new(0.0, 0.0, 0.0, 0.0);
    zeroed.cam_right = boyko_math::Vec4::new(0.0, 0.0, 0.0, 0.0);
    zeroed.cam_up = boyko_math::Vec4::new(0.0, 0.0, 0.0, 0.0);

    let rd = ray_gen_perspective_dir_mirror(&zeroed, 0.0, 0.0, (FRAC_PI_3 * 0.5).tan(), 1.0);
    assert!(
        rd[0].is_nan() && rd[1].is_nan() && rd[2].is_nan(),
        "the detector must NaN on a zeroed basis, else the finiteness gate above is vacuous \
         (got {rd:?})"
    );
}

// ---- Golden neutrality: a valid camera is bit-unchanged ------------------------------

#[test]
fn a_well_formed_camera_basis_is_bit_identical_to_the_raw_normalize() {
    // A rotated + uniformly scaled camera — the general non-degenerate case. The fallback
    // branch must NOT be taken, so every published lane must equal the raw `mul_vec().normalize()`
    // the function computed before the guard existed.
    let linear = Mat3::from_columns(
        Vec3::new(0.6, 0.8, 0.0),
        Vec3::new(-0.8, 0.6, 0.0),
        Vec3::new(0.0, 0.0, 2.0),
    );
    let global = Affine3A { matrix3: linear, translation: Vec3::new(-4.0, 0.5, 7.25) };
    let view = ViewUniform::from_camera(global, perspective(FRAC_PI_3));

    let expect_right = linear.mul_vec(Vec3::new(1.0, 0.0, 0.0)).normalize();
    let expect_up = linear.mul_vec(Vec3::new(0.0, 1.0, 0.0)).normalize();
    let expect_forward = linear.mul_vec(Vec3::new(0.0, 0.0, -1.0)).normalize();

    // Bit equality, not epsilon: the guard is a branch, and a not-taken branch changes nothing.
    for (got, want, what) in [
        ([view.cam_right.x, view.cam_right.y, view.cam_right.z], expect_right, "right"),
        ([view.cam_up.x, view.cam_up.y, view.cam_up.z], expect_up, "up"),
        ([view.cam_forward.x, view.cam_forward.y, view.cam_forward.z], expect_forward, "forward"),
    ] {
        assert_eq!(got[0].to_bits(), want.x.to_bits(), "{what}.x moved");
        assert_eq!(got[1].to_bits(), want.y.to_bits(), "{what}.y moved");
        assert_eq!(got[2].to_bits(), want.z.to_bits(), "{what}.z moved");
    }
}

#[test]
fn the_identity_camera_is_bit_identical_to_the_identity_view_basis() {
    let view = ViewUniform::from_camera(Affine3A::IDENTITY, perspective(FRAC_PI_3));
    let want = ViewUniform::IDENTITY;
    assert_eq!(view.cam_right.x.to_bits(), want.cam_right.x.to_bits());
    assert_eq!(view.cam_up.y.to_bits(), want.cam_up.y.to_bits());
    assert_eq!(view.cam_forward.z.to_bits(), want.cam_forward.z.to_bits());
}
