//! S35 camera-rig MATH matrix (STDLIB-S35-CAMERA-RIG-PLAN §7, `boyko_math` rows).
//!
//! Covers the three new look-at primitives:
//! - **W2** `Mat3::from_columns` — exact `rows` transpose layout AND the
//!   mechanical column-selection guard (`mul_vec(unit_axis_i) == c_i`).
//! - **C1** `Quat::from_mat3` — the EXACT algebraic inverse of `Mat3::from_quat`:
//!   an element-by-element directional round-trip on a NON-symmetric matrix
//!   (`m_90z`), a HAND-COMPUTED ground-truth rotate (NOT a round-trip), and a
//!   deterministic four-Shepperd-branch directional sweep (trace + near-180°
//!   about X/Y/Z) comparing the ROTATION ACTION on fixed vectors (q/−q double
//!   cover aware). No `proptest` dep in this crate, so the "property" case is a
//!   deterministic sampled loop (no RNG).
//! - `Affine3A::look_at_rh` — head-on basis (orthonormal + RIGHT-HANDED:
//!   `cross(right, true_up) ≈ back`, `det ≈ +1`), looks-AT-target projection to
//!   NDC origin, and the **W1** degenerate guards (`eye==target` AND the pole)
//!   staying finite with `det ≈ +1` (handedness preserved in the fallback).

use core::f32::consts::{FRAC_PI_2, PI};

use boyko_math::{Affine3A, Mat3, Mat4, Quat, Vec3, Vec4};

/// Tolerance for derived float comparisons (rotations, products, a `sqrt`).
const EPS: f32 = 1.0e-5;
/// Looser tolerance for the near-180° branches, where the f32 `sqrt` of a
/// near-`1+m_kk-…` operand and the `1/s` divide accumulate a few extra ULPs.
const EPS_180: f32 = 2.0e-4;

#[track_caller]
fn approx(a: f32, b: f32, eps: f32, what: &str) {
    assert!((a - b).abs() <= eps, "{what}: expected {b}, got {a} (|Δ|={})", (a - b).abs());
}

#[track_caller]
fn vec3_approx(a: Vec3, b: Vec3, eps: f32, what: &str) {
    assert!(
        (a.x - b.x).abs() <= eps && (a.y - b.y).abs() <= eps && (a.z - b.z).abs() <= eps,
        "{what}: expected {b:?}, got {a:?}"
    );
}

/// `true` when `a` matches `b` OR `-b` (the quaternion double cover) — used only
/// when comparing the ROTATION ACTION is impractical and the raw rotate is.
#[track_caller]
fn vec3_approx_signed(a: Vec3, b: Vec3, eps: f32, what: &str) {
    let pos = (a.x - b.x).abs() <= eps && (a.y - b.y).abs() <= eps && (a.z - b.z).abs() <= eps;
    let neg = (a.x + b.x).abs() <= eps && (a.y + b.y).abs() <= eps && (a.z + b.z).abs() <= eps;
    assert!(pos || neg, "{what}: expected ±{b:?}, got {a:?}");
}

#[track_caller]
fn mat3_approx(a: Mat3, b: Mat3, eps: f32, what: &str) {
    for r in 0..3 {
        vec3_approx(a.rows[r], b.rows[r], eps, &format!("{what} row {r}"));
    }
}

/// A unit-axis basis vector (`i == 0 → +X`, `1 → +Y`, `2 → +Z`).
fn unit_axis(i: usize) -> Vec3 {
    match i {
        0 => Vec3::new(1.0, 0.0, 0.0),
        1 => Vec3::new(0.0, 1.0, 0.0),
        _ => Vec3::new(0.0, 0.0, 1.0),
    }
}

/// A unit quaternion for a rotation of `angle` (rad) about (already-unit) `axis`.
fn quat_axis_angle(axis: Vec3, angle: f32) -> Quat {
    let h = angle * 0.5;
    let s = h.sin();
    Quat::new(axis.x * s, axis.y * s, axis.z * s, h.cos())
}

// ════════════════════════════════════════════════════════════════════════════
// W2 — Mat3::from_columns exact layout + column selection
// ════════════════════════════════════════════════════════════════════════════

/// The `rows` storage of `from_columns(c0,c1,c2)` is the TRANSPOSE pattern
/// `[(c0.x,c1.x,c2.x),(c0.y,c1.y,c2.y),(c0.z,c1.z,c2.z)]`. A transpose-the-wrong
/// way (storing the columns AS rows) fails this.
#[test]
fn from_columns_stores_transpose_layout() {
    let c0 = Vec3::new(1.0, 2.0, 3.0);
    let c1 = Vec3::new(4.0, 5.0, 6.0);
    let c2 = Vec3::new(7.0, 8.0, 9.0);
    let m = Mat3::from_columns(c0, c1, c2);
    let want = Mat3::from_rows(
        Vec3::new(c0.x, c1.x, c2.x),
        Vec3::new(c0.y, c1.y, c2.y),
        Vec3::new(c0.z, c1.z, c2.z),
    );
    mat3_approx(m, want, 0.0, "from_columns rows must be the column-transpose layout");
}

/// The mechanical guard: `from_columns(c0,c1,c2).mul_vec(unit_axis_i) == c_i`.
/// This is what the look-at basis convention relies on (a local axis selects a
/// stored column). A transpose slip would return a ROW instead.
#[test]
fn from_columns_mul_vec_selects_each_column() {
    let cols = [
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(4.0, 5.0, 6.0),
        Vec3::new(7.0, 8.0, 9.0),
    ];
    let m = Mat3::from_columns(cols[0], cols[1], cols[2]);
    for (i, &col) in cols.iter().enumerate() {
        let got = m.mul_vec(unit_axis(i));
        vec3_approx(got, col, EPS, &format!("mul_vec(axis {i}) must select column {i}"));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// C1 — Quat::from_mat3 is the EXACT inverse of Mat3::from_quat
// ════════════════════════════════════════════════════════════════════════════

/// The matrix of a +90° rotation about +Z (NON-symmetric: `m01 = -1`, `m10 = +1`)
/// — derived from `Mat3::from_quat` so it is self-consistent with the convention
/// under test (not an independently-hand-typed matrix).
fn m_90z() -> Mat3 {
    Mat3::from_quat(quat_axis_angle(Vec3::new(0.0, 0.0, 1.0), FRAC_PI_2))
}

/// C1 directional round-trip on a NON-symmetric matrix: `from_quat(from_mat3(m))`
/// reproduces `m` ELEMENT-BY-ELEMENT. A transpose/sign slip in `from_mat3` yields
/// `q⁻¹` and the off-diagonals flip sign — caught here (this is NOT a
/// `m == mᵀ`-tolerant check; `m_90z` is asymmetric on purpose).
#[test]
fn from_mat3_round_trips_non_symmetric_matrix_elementwise() {
    let m = m_90z();
    // Sanity: the input really IS asymmetric (m01 == -m10), so a transposed
    // from_mat3 cannot pass by coincidence.
    approx(m.rows[0].y, -1.0, EPS, "m01 of the 90° z-rotation");
    approx(m.rows[1].x, 1.0, EPS, "m10 of the 90° z-rotation");

    let round = Mat3::from_quat(Quat::from_mat3(m));
    mat3_approx(round, m, EPS, "from_quat(from_mat3(m_90z)) must equal m_90z");
}

/// C1 ground-truth rotate (HAND-COMPUTED, NOT a round-trip): a +90° rotation
/// about +Z sends +X → +Y. The expected vector is derived from `m_90z * (1,0,0)`
/// so it is self-consistent with `Mat3::from_quat`'s active-rotation convention;
/// a transposed/inverted `from_mat3` would return `(0,-1,0)` and fail.
#[test]
fn from_mat3_rotate_matches_hand_computed_ground_truth() {
    let m = m_90z();
    let v = Vec3::new(1.0, 0.0, 0.0);

    // The ground truth: the matrix's own action on v (self-consistent, NOT a
    // quaternion round-trip). +90° about +Z maps (1,0,0) → (0,1,0).
    let expected = m.mul_vec(v);
    vec3_approx(expected, Vec3::new(0.0, 1.0, 0.0), EPS, "matrix action 90°z: +X → +Y");

    let q = Quat::from_mat3(m);
    let got = q.rotate(v);
    vec3_approx(got, expected, EPS, "Quat::from_mat3(m_90z).rotate(+X) must equal m_90z·(+X)");
}

/// C1 four-branch DIRECTIONAL sweep (deterministic, NO RNG). For each sampled
/// rotation we assert the ROTATION ACTION on several fixed vectors round-trips:
/// `from_mat3(from_quat(q)).rotate(v) ≈ q.rotate(v)` (action comparison is
/// q/−q double-cover immune). The sample set is constructed to HIT all four
/// Shepperd branches:
/// - the TRACE branch: many small/medium angles about arbitrary axes;
/// - the X/Y/Z-diagonal branches: 179° about each principal axis.
#[test]
fn from_mat3_four_branches_preserve_rotation_action() {
    // Fixed, non-axis-aligned probe vectors (so no rotation acts trivially).
    let probes = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.37, -0.81, 0.45),
        Vec3::new(-0.6, 0.2, 0.77),
    ];

    // A deterministic spread of axes (each normalized) and angles. The angles
    // span the trace branch (small/medium) explicitly.
    let axes = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(1.0, 0.0, 1.0),
        Vec3::new(0.0, 1.0, 1.0),
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new(2.0, -1.0, 3.0),
        Vec3::new(-1.0, 4.0, -2.0),
    ];
    let angles = [
        0.0_f32, 0.0872665, // 5°, the trace branch's hallmark
        0.5, 1.0, FRAC_PI_2, 2.0, 2.5,
    ];

    // ── trace + mixed-angle branch coverage ──────────────────────────────────
    for a in axes {
        let axis = a.normalize();
        for &ang in &angles {
            let q = quat_axis_angle(axis, ang);
            let m = Mat3::from_quat(q);
            let rebuilt = Quat::from_mat3(m);

            // Component double-cover check (informational; the load-bearing one
            // is the action below).
            let same = (rebuilt.x - q.x).abs() <= EPS_180
                && (rebuilt.y - q.y).abs() <= EPS_180
                && (rebuilt.z - q.z).abs() <= EPS_180
                && (rebuilt.w - q.w).abs() <= EPS_180;
            let neg = (rebuilt.x + q.x).abs() <= EPS_180
                && (rebuilt.y + q.y).abs() <= EPS_180
                && (rebuilt.z + q.z).abs() <= EPS_180
                && (rebuilt.w + q.w).abs() <= EPS_180;
            assert!(
                same || neg,
                "from_mat3(from_quat(q)) must be ±q (axis {axis:?}, angle {ang}): got {rebuilt:?} vs {q:?}"
            );

            for &v in &probes {
                vec3_approx(
                    rebuilt.rotate(v),
                    q.rotate(v),
                    EPS_180,
                    &format!("trace-sweep action (axis {axis:?}, angle {ang})"),
                );
            }
        }
    }

    // ── near-180° about each principal axis (x/y/z-diagonal branches) ─────────
    // 179° lands the trace ≈ -1, forcing the largest-diagonal branch selection.
    let near_pi = PI - 0.0174533; // 179°
    for i in 0..3 {
        let axis = unit_axis(i);
        let q = quat_axis_angle(axis, near_pi);
        let m = Mat3::from_quat(q);
        let rebuilt = Quat::from_mat3(m);

        for &v in &probes {
            vec3_approx(
                rebuilt.rotate(v),
                q.rotate(v),
                EPS_180,
                &format!("diagonal-branch action (axis {i}, 179°)"),
            );
        }
        // Also assert the rebuilt quaternion is ±q on the raw components, so the
        // chosen diagonal branch's exact off-diagonal SIGN pattern is exercised.
        vec3_approx_signed(
            Vec3::new(rebuilt.x, rebuilt.y, rebuilt.z),
            Vec3::new(q.x, q.y, q.z),
            EPS_180,
            &format!("diagonal-branch vector part (axis {i}, 179°)"),
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// look_at_rh — basis, projection, degenerate guards
// ════════════════════════════════════════════════════════════════════════════

/// Head-on: `eye=(0,0,5)`, `target=ORIGIN`, `up=+Y` →
/// `back ≈ (0,0,1)`, `right ≈ (1,0,0)`, `true_up ≈ (0,1,0)`; orthonormal AND
/// right-handed (`cross(right, true_up) ≈ back`, `det(matrix3) ≈ +1`).
#[test]
fn look_at_rh_head_on_basis_is_orthonormal_right_handed() {
    let eye = Vec3::new(0.0, 0.0, 5.0);
    let world = Affine3A::look_at_rh(eye, Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));

    // Columns are selectable by mul_vec on the unit axes (from_columns convention).
    let right = world.matrix3.mul_vec(unit_axis(0));
    let true_up = world.matrix3.mul_vec(unit_axis(1));
    let back = world.matrix3.mul_vec(unit_axis(2));

    vec3_approx(back, Vec3::new(0.0, 0.0, 1.0), EPS, "back (col 2) = +Z toward eye");
    vec3_approx(right, Vec3::new(1.0, 0.0, 0.0), EPS, "right (col 0) = +X");
    vec3_approx(true_up, Vec3::new(0.0, 1.0, 0.0), EPS, "true_up (col 1) = +Y");

    // Orthonormal.
    approx(right.length(), 1.0, EPS, "right is unit");
    approx(true_up.length(), 1.0, EPS, "true_up is unit");
    approx(back.length(), 1.0, EPS, "back is unit");
    approx(right.dot(true_up), 0.0, EPS, "right ⟂ true_up");
    approx(right.dot(back), 0.0, EPS, "right ⟂ back");
    approx(true_up.dot(back), 0.0, EPS, "true_up ⟂ back");

    // Right-handed.
    vec3_approx(right.cross(true_up), back, EPS, "cross(right, true_up) ≈ back");
    approx(world.matrix3.determinant(), 1.0, EPS, "det(matrix3) ≈ +1");

    // Eye is the translation.
    vec3_approx(world.translation, eye, EPS, "translation == eye");
}

/// `look_at_rh` looks AT the target: the derived VIEW (`look_at_rh.inverse()`,
/// the camera-world inverse) composed with a perspective projection maps the
/// `target` to NDC ≈ origin (screen center), i.e. clip `x/w ≈ 0` and `y/w ≈ 0`.
#[test]
fn look_at_rh_projects_target_to_ndc_center() {
    let eye = Vec3::new(3.0, 2.0, 4.0);
    let target = Vec3::new(-1.0, 0.5, 0.0);
    let world = Affine3A::look_at_rh(eye, target, Vec3::new(0.0, 1.0, 0.0));

    let view = world.inverse().expect("rigid look-at is invertible").to_mat4();
    // Build the projection directly in `boyko_math` (the crate under test cannot
    // depend on `boyko_scene::Projection`); this is the same matrix
    // `Projection::Perspective.to_mat4()` produces.
    let proj = Mat4::perspective_rh(FRAC_PI_2, 1.0, 0.1, 100.0);
    let view_proj = proj.mul_mat4(view);

    let clip = view_proj.mul_vec4(Vec4::from_vec3(target, 1.0));
    assert!(clip.w.abs() > EPS, "target is in front of the camera (w != 0): w={}", clip.w);
    approx(clip.x / clip.w, 0.0, 1.0e-4, "target projects to NDC x ≈ 0");
    approx(clip.y / clip.w, 0.0, 1.0e-4, "target projects to NDC y ≈ 0");
}

/// **W1** degenerate guard — `eye == target` (zero `back`): the result is
/// all-finite (no NaN/Inf) AND `det(matrix3) ≈ +1` (the fallback `back = +Z`
/// keeps the basis proper).
#[test]
fn look_at_rh_eye_equals_target_is_finite_and_right_handed() {
    let p = Vec3::new(2.0, -3.0, 1.0);
    let world = Affine3A::look_at_rh(p, p, Vec3::new(0.0, 1.0, 0.0));

    for r in 0..3 {
        assert!(world.matrix3.rows[r].is_finite(), "matrix3 row {r} must be finite");
    }
    assert!(world.translation.is_finite(), "translation must be finite");
    approx(world.matrix3.determinant(), 1.0, EPS, "eye==target fallback keeps det ≈ +1");
}

/// **W1** degenerate guard — the POLE (`up ∥ back`): eye directly above the
/// target with `up = +Y` (so `up ∥ (eye - target)`). The fallback swaps ONLY the
/// source `up`, reusing the SAME cross order, so the result is all-finite AND
/// `det(matrix3) ≈ +1` (chirality preserved).
#[test]
fn look_at_rh_pole_is_finite_and_right_handed() {
    let eye = Vec3::new(0.0, 5.0, 0.0);
    let world = Affine3A::look_at_rh(eye, Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));

    let right = world.matrix3.mul_vec(unit_axis(0));
    let true_up = world.matrix3.mul_vec(unit_axis(1));
    let back = world.matrix3.mul_vec(unit_axis(2));

    for v in [right, true_up, back, world.translation] {
        assert!(v.is_finite(), "pole-case basis/translation must be finite: {v:?}");
    }
    // back points from target toward eye, i.e. +Y here.
    vec3_approx(back, Vec3::new(0.0, 1.0, 0.0), EPS, "pole back = +Y (eye above target)");
    // Still orthonormal + right-handed.
    approx(right.length(), 1.0, EPS, "pole right is unit");
    approx(true_up.length(), 1.0, EPS, "pole true_up is unit");
    approx(right.dot(true_up), 0.0, EPS, "pole right ⟂ true_up");
    vec3_approx(right.cross(true_up), back, EPS, "pole cross(right, true_up) ≈ back");
    approx(world.matrix3.determinant(), 1.0, EPS, "pole fallback keeps det ≈ +1");
}
