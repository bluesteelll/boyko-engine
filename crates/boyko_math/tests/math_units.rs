//! Unit + convention tests for the `boyko_math` vocabulary (std-lib S1).
//!
//! Coverage goals:
//! - `Vec2` / `Vec3` / `Vec4`: construction, ops, exact-`sqrt` length, normalize
//!   (incl. zero-guard + bit-determinism), dot/cross, abs/clamp, projections.
//! - `Quat`: identity, normalize (zero-guard to identity), Hamilton product,
//!   rotate / inverse_rotate, conjugate, integrate.
//! - `Mat3` (ROW-major): identity/zero, from_rows, mul_vec, from_diagonal,
//!   from_quat (matches `Quat::rotate`), transpose (involution), determinant,
//!   inverse (round-trip + singular), matrix product (associativity, identity).
//! - `Mat4` (COLUMN-major): identity, from_cols, mul_vec4, mul_mat4, projections.
//! - `Affine3A`: identity, TRS construction, transform_point / transform_vector,
//!   compose (`mul`), inverse round-trip.
//! - The single row-major <-> column-major convention boundary:
//!   `Affine3A::to_mat4` / `Mat4::from_affine` must transpose the linear part and
//!   place the translation in the last column, and the resulting `Mat4` must
//!   transform a homogeneous point identically to the affine.
//!
//! Bit-determinism: the lifted `Vec3`/`Quat`/`Mat3` normalize is asserted to be
//! `to_bits`-exactly `len_sq.sqrt().recip()` (NOT `rsqrt`), and a re-run check
//! confirms the new `Mat4`/`Affine3A` ops are reproducible bit-for-bit.

use boyko_math::{Affine3A, Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

/// Tolerance for derived float comparisons (rotations, products).
const EPS: f32 = 1e-5;

fn close(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps
}

fn vec3_close(a: Vec3, b: Vec3, eps: f32) -> bool {
    close(a.x, b.x, eps) && close(a.y, b.y, eps) && close(a.z, b.z, eps)
}

// ----------------------------------------------------------------------------
// Vec2
// ----------------------------------------------------------------------------

#[test]
fn vec2_constants_and_new() {
    assert_eq!(Vec2::ZERO, Vec2::new(0.0, 0.0), "ZERO is (0,0)");
    assert_eq!(Vec2::ONE, Vec2::new(1.0, 1.0), "ONE is (1,1)");
    let v = Vec2::new(2.0, -3.0);
    assert_eq!((v.x, v.y), (2.0, -3.0), "new stores components verbatim");
}

#[test]
fn vec2_add_sub_scale() {
    let a = Vec2::new(1.0, 2.0);
    let b = Vec2::new(3.0, -1.0);
    assert_eq!(a + b, Vec2::new(4.0, 1.0), "componentwise add");
    assert_eq!(a - b, Vec2::new(-2.0, 3.0), "componentwise sub");
    assert_eq!(a * 2.0, Vec2::new(2.0, 4.0), "scalar mul");
}

#[test]
fn vec2_dot_and_cross() {
    let a = Vec2::new(1.0, 0.0);
    let b = Vec2::new(0.0, 1.0);
    assert_eq!(a.dot(b), 0.0, "orthogonal dot is zero");
    assert_eq!(a.dot(a), 1.0, "self dot is length_squared");
    // 2D cross is the signed z of the 3D cross: x cross y = +1.
    assert_eq!(a.cross(b), 1.0, "x cross y = +1");
    assert_eq!(b.cross(a), -1.0, "cross is anti-commutative");
}

#[test]
fn vec2_length_is_exact_sqrt() {
    let v = Vec2::new(3.0, 4.0);
    assert_eq!(v.length_squared(), 25.0, "3^2 + 4^2 = 25");
    // length is EXACTLY length_squared().sqrt() (no rsqrt approximation).
    assert_eq!(
        v.length().to_bits(),
        25.0_f32.sqrt().to_bits(),
        "length is exact sqrt of length_squared (bit-identical)"
    );
    assert_eq!(v.length(), 5.0, "|(3,4)| = 5");
}

#[test]
fn vec2_normalize_unit_and_bit_exact() {
    let v = Vec2::new(3.0, 4.0);
    let n = v.normalize();
    assert!(close(n.length(), 1.0, EPS), "normalized is unit length");
    // Bit-determinism: normalize is literally len_sq.sqrt().recip(), NOT rsqrt.
    let inv = 25.0_f32.sqrt().recip();
    assert_eq!(n.x.to_bits(), (3.0_f32 * inv).to_bits(), "x bit-exact");
    assert_eq!(n.y.to_bits(), (4.0_f32 * inv).to_bits(), "y bit-exact");
}

#[test]
fn vec2_normalize_zero_guard() {
    assert_eq!(
        Vec2::ZERO.normalize(),
        Vec2::ZERO,
        "zero-length normalize returns ZERO (no NaN)"
    );
}

#[test]
fn vec2_abs_and_componentwise_and_finite() {
    let v = Vec2::new(-2.0, 3.0);
    assert_eq!(v.abs(), Vec2::new(2.0, 3.0), "per-component abs");
    assert_eq!(
        v.componentwise_mul(Vec2::new(2.0, -1.0)),
        Vec2::new(-4.0, -3.0),
        "Hadamard product"
    );
    assert!(v.is_finite(), "finite vector");
    assert!(
        !Vec2::new(f32::NAN, 0.0).is_finite(),
        "NaN makes it non-finite"
    );
    assert!(
        !Vec2::new(0.0, f32::INFINITY).is_finite(),
        "Inf makes it non-finite"
    );
}

// ----------------------------------------------------------------------------
// Vec3
// ----------------------------------------------------------------------------

#[test]
fn vec3_constants_and_new() {
    assert_eq!(Vec3::ZERO, Vec3::new(0.0, 0.0, 0.0), "ZERO");
    assert_eq!(Vec3::ONE, Vec3::new(1.0, 1.0, 1.0), "ONE");
}

#[test]
fn vec3_add_sub_scale() {
    let a = Vec3::new(1.0, 2.0, 3.0);
    let b = Vec3::new(-1.0, 0.5, 4.0);
    assert_eq!(a + b, Vec3::new(0.0, 2.5, 7.0), "add");
    assert_eq!(a - b, Vec3::new(2.0, 1.5, -1.0), "sub");
    assert_eq!(a * 2.0, Vec3::new(2.0, 4.0, 6.0), "scale");
}

#[test]
fn vec3_dot_and_cross_right_handed() {
    let x = Vec3::new(1.0, 0.0, 0.0);
    let y = Vec3::new(0.0, 1.0, 0.0);
    let z = Vec3::new(0.0, 0.0, 1.0);
    assert_eq!(x.dot(y), 0.0, "orthogonal");
    assert_eq!(x.dot(x), 1.0, "self dot");
    assert_eq!(x.cross(y), z, "x cross y = z (right-handed)");
    assert_eq!(y.cross(z), x, "y cross z = x");
    assert_eq!(z.cross(x), y, "z cross x = y");
    assert_eq!(y.cross(x), z * -1.0, "anti-commutative");
}

#[test]
fn vec3_length_is_exact_sqrt() {
    let v = Vec3::new(1.0, 2.0, 2.0);
    assert_eq!(v.length_squared(), 9.0, "1+4+4 = 9");
    assert_eq!(
        v.length().to_bits(),
        9.0_f32.sqrt().to_bits(),
        "length = exact sqrt(length_squared)"
    );
    assert_eq!(v.length(), 3.0, "|(1,2,2)| = 3");
}

#[test]
fn vec3_normalize_unit_and_bit_exact() {
    let v = Vec3::new(0.0, 3.0, 4.0);
    let n = v.normalize();
    assert!(close(n.length(), 1.0, EPS), "unit length");
    let inv = 25.0_f32.sqrt().recip();
    assert_eq!(n.x.to_bits(), (0.0_f32 * inv).to_bits(), "x bit-exact");
    assert_eq!(n.y.to_bits(), (3.0_f32 * inv).to_bits(), "y bit-exact");
    assert_eq!(n.z.to_bits(), (4.0_f32 * inv).to_bits(), "z bit-exact");
}

#[test]
fn vec3_normalize_zero_guard() {
    assert_eq!(
        Vec3::ZERO.normalize(),
        Vec3::ZERO,
        "zero normalize -> ZERO (no NaN)"
    );
    // A sub-MIN_POSITIVE-squared vector is treated as zero (the narrowphase
    // coincident-body guard).
    let tiny = Vec3::new(1e-30, 0.0, 0.0);
    assert_eq!(tiny.normalize(), Vec3::ZERO, "tiny normalize -> ZERO");
}

#[test]
fn vec3_axis_reads_component() {
    let v = Vec3::new(7.0, 8.0, 9.0);
    assert_eq!(v.axis(0), 7.0, "axis 0 = x");
    assert_eq!(v.axis(1), 8.0, "axis 1 = y");
    assert_eq!(v.axis(2), 9.0, "axis 2 = z");
}

#[test]
#[should_panic(expected = "axis index must be 0..3")]
fn vec3_axis_out_of_range_panics_in_debug() {
    // debug_assert!-guarded; the test binary is built in debug so the panic
    // fires. (In release this debug_assert is compiled out by design.)
    let v = Vec3::new(1.0, 2.0, 3.0);
    let _ = v.axis(3);
}

#[test]
fn vec3_abs_clamp_componentwise_xy() {
    let v = Vec3::new(-2.0, 3.0, -4.0);
    assert_eq!(v.abs(), Vec3::new(2.0, 3.0, 4.0), "abs");
    assert_eq!(
        v.componentwise_mul(Vec3::new(2.0, 2.0, 2.0)),
        Vec3::new(-4.0, 6.0, -8.0),
        "Hadamard"
    );
    let limit = Vec3::new(1.0, 1.0, 1.0);
    assert_eq!(
        v.clamp_symmetric(limit),
        Vec3::new(-1.0, 1.0, -1.0),
        "clamp into [-1,1] box"
    );
    assert_eq!(v.xy(), Vec2::new(-2.0, 3.0), "xy projection drops z");
}

#[test]
fn vec3_is_finite() {
    assert!(Vec3::new(1.0, 2.0, 3.0).is_finite(), "finite");
    assert!(
        !Vec3::new(0.0, f32::NAN, 0.0).is_finite(),
        "NaN non-finite"
    );
    assert!(
        !Vec3::new(0.0, 0.0, f32::NEG_INFINITY).is_finite(),
        "Inf non-finite"
    );
}

// ----------------------------------------------------------------------------
// Vec4
// ----------------------------------------------------------------------------

#[test]
fn vec4_align_and_size() {
    // The GPU/std140 lane contract: exactly 16 bytes, 16-aligned.
    assert_eq!(core::mem::size_of::<Vec4>(), 16, "Vec4 is 16 bytes");
    assert_eq!(core::mem::align_of::<Vec4>(), 16, "Vec4 is 16-aligned");
}

#[test]
fn vec4_construct_and_projections() {
    let v = Vec4::new(1.0, 2.0, 3.0, 4.0);
    assert_eq!(v.xyz(), Vec3::new(1.0, 2.0, 3.0), "xyz drops w");
    assert_eq!(
        Vec4::from_vec3(Vec3::new(5.0, 6.0, 7.0), 1.0),
        Vec4::new(5.0, 6.0, 7.0, 1.0),
        "from_vec3 appends w"
    );
    assert_eq!(Vec4::ZERO, Vec4::new(0.0, 0.0, 0.0, 0.0), "ZERO");
}

#[test]
fn vec4_ops_and_length() {
    let a = Vec4::new(1.0, 2.0, 3.0, 4.0);
    let b = Vec4::new(4.0, 3.0, 2.0, 1.0);
    assert_eq!(a + b, Vec4::new(5.0, 5.0, 5.0, 5.0), "add");
    assert_eq!(a - b, Vec4::new(-3.0, -1.0, 1.0, 3.0), "sub");
    assert_eq!(a * 2.0, Vec4::new(2.0, 4.0, 6.0, 8.0), "scale");
    assert_eq!(a.dot(b), 4.0 + 6.0 + 6.0 + 4.0, "dot = 20");
    let u = Vec4::new(1.0, 0.0, 0.0, 0.0);
    assert_eq!(u.length_squared(), 1.0, "length_squared");
    assert_eq!(
        Vec4::new(2.0, 0.0, 0.0, 0.0).length().to_bits(),
        4.0_f32.sqrt().to_bits(),
        "length is exact sqrt"
    );
}

// ----------------------------------------------------------------------------
// Quat
// ----------------------------------------------------------------------------

#[test]
fn quat_identity_and_default() {
    assert_eq!(Quat::IDENTITY, Quat::new(0.0, 0.0, 0.0, 1.0), "identity");
    assert_eq!(Quat::default(), Quat::IDENTITY, "default is identity");
}

#[test]
fn quat_identity_rotate_is_noop() {
    let v = Vec3::new(1.0, -2.0, 3.0);
    assert_eq!(Quat::IDENTITY.rotate(v), v, "identity rotation is no-op");
}

#[test]
fn quat_normalize_unit_and_bit_exact() {
    let q = Quat::new(0.0, 0.0, 2.0, 0.0);
    let n = q.normalize();
    let len = (n.x * n.x + n.y * n.y + n.z * n.z + n.w * n.w).sqrt();
    assert!(close(len, 1.0, EPS), "unit quaternion");
    // Bit-determinism: len_sq.sqrt().recip(), NOT rsqrt.
    let inv = 4.0_f32.sqrt().recip();
    assert_eq!(n.z.to_bits(), (2.0_f32 * inv).to_bits(), "z bit-exact");
}

#[test]
fn quat_normalize_zero_guard_to_identity() {
    assert_eq!(
        Quat::new(0.0, 0.0, 0.0, 0.0).normalize(),
        Quat::IDENTITY,
        "all-zero normalize -> IDENTITY (a valid rotation, not NaN)"
    );
}

#[test]
fn quat_hamilton_identity_is_neutral() {
    let q = Quat::new(0.1, 0.2, 0.3, 0.9).normalize();
    // Hamilton product with identity (both operand orders) leaves q unchanged.
    let li = Quat::IDENTITY.mul(q);
    let ri = q.mul(Quat::IDENTITY);
    assert!(close(li.x, q.x, EPS) && close(li.w, q.w, EPS), "id * q = q");
    assert!(close(ri.x, q.x, EPS) && close(ri.w, q.w, EPS), "q * id = q");
    // The `Mul` operator agrees with the named `mul`.
    let op = q * Quat::IDENTITY;
    assert_eq!(op, ri, "operator * agrees with named mul");
}

#[test]
fn quat_rotate_known_z90() {
    let half = std::f32::consts::FRAC_PI_4; // theta/2 for 90deg
    let q = Quat::new(0.0, 0.0, half.sin(), half.cos());
    let r = q.rotate(Vec3::new(1.0, 0.0, 0.0));
    assert!(vec3_close(r, Vec3::new(0.0, 1.0, 0.0), EPS), "+x -> +y: {r:?}");
}

#[test]
fn quat_conjugate_and_inverse_rotate_undo() {
    let q = Quat::new(0.1, -0.3, 0.2, 0.9).normalize();
    let c = q.conjugate();
    assert_eq!(c.x, -q.x, "conj negates x");
    assert_eq!(c.w, q.w, "conj keeps w");
    // inverse_rotate undoes rotate for a unit quaternion.
    let v = Vec3::new(1.0, -2.0, 3.0);
    let round = q.inverse_rotate(q.rotate(v));
    assert!(vec3_close(round, v, EPS), "inverse_rotate(rotate(v)) = v: {round:?}");
}

#[test]
fn quat_integrate_advances_and_stays_unit() {
    let omega = Vec3::new(0.0, 0.0, 1.0); // 1 rad/s about +z
    let q = Quat::IDENTITY.integrate(omega, 0.1);
    let len = (q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w).sqrt();
    assert!(close(len, 1.0, EPS), "integrate re-normalizes to unit");
    let r = q.rotate(Vec3::new(1.0, 0.0, 0.0));
    assert!(r.y > 0.0, "+x rotates toward +y under +z omega: {r:?}");
    assert!(r.z.abs() < EPS, "rotation stays in xy-plane");
}

#[test]
fn quat_integrate_is_bit_deterministic_across_runs() {
    // The integrate operation order is fixed; the same inputs give the same bits.
    let omega = Vec3::new(0.7, -0.5, 0.9);
    let a = Quat::IDENTITY.integrate(omega, 1.0 / 64.0);
    let b = Quat::IDENTITY.integrate(omega, 1.0 / 64.0);
    assert_eq!(a.x.to_bits(), b.x.to_bits(), "x bit-stable");
    assert_eq!(a.y.to_bits(), b.y.to_bits(), "y bit-stable");
    assert_eq!(a.z.to_bits(), b.z.to_bits(), "z bit-stable");
    assert_eq!(a.w.to_bits(), b.w.to_bits(), "w bit-stable");
}

// ----------------------------------------------------------------------------
// Mat3 (ROW-major)
// ----------------------------------------------------------------------------

#[test]
fn mat3_identity_zero_default() {
    let v = Vec3::new(5.0, -7.0, 11.0);
    assert_eq!(Mat3::IDENTITY.mul_vec(v), v, "identity * v = v");
    assert_eq!(Mat3::ZERO.mul_vec(v), Vec3::ZERO, "zero * v = 0");
    assert_eq!(Mat3::default(), Mat3::IDENTITY, "default is identity");
}

#[test]
fn mat3_mul_vec_is_row_major() {
    // rows[i] is row i, so output component i is rows[i] . v.
    let m = Mat3::from_rows(
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(4.0, 5.0, 6.0),
        Vec3::new(7.0, 8.0, 9.0),
    );
    let v = Vec3::new(1.0, 0.0, 0.0);
    // ROW-major: column 0 of the matrix = (rows[0].x, rows[1].x, rows[2].x).
    assert_eq!(
        m.mul_vec(v),
        Vec3::new(1.0, 4.0, 7.0),
        "mul_vec(e0) selects column 0 (row-major)"
    );
}

#[test]
fn mat3_from_diagonal_scales() {
    let d = Vec3::new(2.0, 3.0, 4.0);
    let m = Mat3::from_diagonal(d);
    let v = Vec3::new(5.0, 6.0, 7.0);
    assert_eq!(
        m.mul_vec(v),
        Vec3::new(10.0, 18.0, 28.0),
        "diag(d) scales per-axis"
    );
}

#[test]
fn mat3_from_quat_identity_is_identity() {
    assert_eq!(
        Mat3::from_quat(Quat::IDENTITY),
        Mat3::IDENTITY,
        "from_quat(identity) = IDENTITY"
    );
}

#[test]
fn mat3_from_quat_matches_rotate() {
    // The matrix form must reproduce the quaternion rotation for several pairs.
    let quats = [
        Quat::IDENTITY,
        Quat::new(0.0, 0.0, 0.3826834, 0.9238795), // 45deg about +z
        Quat::new(0.5, 0.5, 0.5, 0.5),             // 120deg about (1,1,1)
        Quat::new(0.1, -0.2, 0.3, 0.9).normalize(),
    ];
    let vecs = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(1.0, -2.0, 3.0),
    ];
    for &q in &quats {
        let m = Mat3::from_quat(q);
        for &v in &vecs {
            assert!(
                vec3_close(m.mul_vec(v), q.rotate(v), EPS),
                "from_quat(q).mul_vec(v) == q.rotate(v) for q={q:?} v={v:?}"
            );
        }
    }
}

#[test]
fn mat3_transpose_involution_and_swaps() {
    let m = Mat3::from_rows(
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(4.0, 5.0, 6.0),
        Vec3::new(7.0, 8.0, 9.0),
    );
    assert_eq!(m.transpose().transpose(), m, "transpose is an involution");
    let t = m.transpose();
    assert_eq!(t.rows[0], Vec3::new(1.0, 4.0, 7.0), "row 0 = old column 0");
    assert_eq!(t.rows[1], Vec3::new(2.0, 5.0, 8.0), "row 1 = old column 1");
    assert_eq!(t.rows[2], Vec3::new(3.0, 6.0, 9.0), "row 2 = old column 2");
}

#[test]
fn mat3_mul_identity_and_associativity() {
    let a = Mat3::from_rows(
        Vec3::new(1.0, 2.0, 0.0),
        Vec3::new(0.0, 1.0, 3.0),
        Vec3::new(4.0, 0.0, 1.0),
    );
    let b = Mat3::from_rows(
        Vec3::new(2.0, 0.0, 1.0),
        Vec3::new(1.0, 3.0, 0.0),
        Vec3::new(0.0, 1.0, 2.0),
    );
    assert_eq!(a * Mat3::IDENTITY, a, "a * I = a");
    assert_eq!(Mat3::IDENTITY * a, a, "I * a = a");
    let v = Vec3::new(5.0, -3.0, 2.0);
    assert!(
        vec3_close((a * b).mul_vec(v), a.mul_vec(b.mul_vec(v)), EPS),
        "(a*b).v == a.(b.v)"
    );
}

#[test]
fn mat3_determinant_known_values() {
    assert_eq!(Mat3::IDENTITY.determinant(), 1.0, "det(I) = 1");
    let d = Mat3::from_diagonal(Vec3::new(2.0, 3.0, 4.0));
    assert_eq!(d.determinant(), 24.0, "det(diag) = product");
    // A singular matrix (two equal rows) has determinant 0.
    let sing = Mat3::from_rows(
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(4.0, 5.0, 6.0),
    );
    assert_eq!(sing.determinant(), 0.0, "singular det = 0");
}

#[test]
fn mat3_inverse_round_trip() {
    let m = Mat3::from_rows(
        Vec3::new(1.0, 2.0, 0.0),
        Vec3::new(0.0, 1.0, 3.0),
        Vec3::new(4.0, 0.0, 1.0),
    );
    let inv = m.inverse().expect("non-singular");
    let prod = m * inv;
    assert!(
        vec3_close(prod.rows[0], Mat3::IDENTITY.rows[0], EPS)
            && vec3_close(prod.rows[1], Mat3::IDENTITY.rows[1], EPS)
            && vec3_close(prod.rows[2], Mat3::IDENTITY.rows[2], EPS),
        "m * m^-1 = I: {prod:?}"
    );
}

#[test]
fn mat3_inverse_singular_is_none() {
    let sing = Mat3::from_rows(
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(2.0, 4.0, 6.0), // 2x row 0
        Vec3::new(0.0, 0.0, 0.0),
    );
    assert!(sing.inverse().is_none(), "singular inverse is None");
}

#[test]
fn mat3_from_quat_scale_is_rotation_times_scale() {
    let q = Quat::new(0.0, 0.0, 0.0, 1.0); // identity rotation
    let s = Vec3::new(2.0, 3.0, 4.0);
    let m = Mat3::from_quat_scale(q, s);
    // With identity rotation this is just diag(scale).
    assert_eq!(
        m.mul_vec(Vec3::new(1.0, 1.0, 1.0)),
        Vec3::new(2.0, 3.0, 4.0),
        "R=I gives diag(scale)"
    );
}

#[test]
fn mat3_inertia_round_trip_symmetric() {
    // R . I_local . R^T equals I_local at R = identity, and is symmetric for any R.
    let i_local = Mat3::from_diagonal(Vec3::new(2.0, 5.0, 11.0));
    let r_id = Mat3::from_quat(Quat::IDENTITY);
    assert_eq!(
        r_id * i_local * r_id.transpose(),
        i_local,
        "R=I -> world tensor = local tensor"
    );
    let q = Quat::new(0.2, -0.4, 0.5, 0.8).normalize();
    let r = Mat3::from_quat(q);
    let world = r * i_local * r.transpose();
    assert!(close(world.rows[0].y, world.rows[1].x, EPS), "M[0][1]=M[1][0]");
    assert!(close(world.rows[0].z, world.rows[2].x, EPS), "M[0][2]=M[2][0]");
    assert!(close(world.rows[1].z, world.rows[2].y, EPS), "M[1][2]=M[2][1]");
}

// ----------------------------------------------------------------------------
// Mat4 (COLUMN-major)
// ----------------------------------------------------------------------------

#[test]
fn mat4_align_and_identity() {
    assert_eq!(core::mem::align_of::<Mat4>(), 16, "Mat4 is 16-aligned");
    let v = Vec4::new(1.0, 2.0, 3.0, 4.0);
    assert_eq!(Mat4::IDENTITY.mul_vec4(v), v, "I * v = v");
    assert_eq!(Mat4::default(), Mat4::IDENTITY, "default is identity");
}

#[test]
fn mat4_mul_vec4_is_column_major() {
    // out = c0*x + c1*y + c2*z + c3*w. Build columns so cols[1] is distinctive.
    let m = Mat4::from_cols(
        Vec4::new(1.0, 0.0, 0.0, 0.0),
        Vec4::new(5.0, 6.0, 7.0, 8.0), // column 1
        Vec4::new(0.0, 0.0, 1.0, 0.0),
        Vec4::new(0.0, 0.0, 0.0, 1.0),
    );
    // v = e1 selects column 1.
    assert_eq!(
        m.mul_vec4(Vec4::new(0.0, 1.0, 0.0, 0.0)),
        Vec4::new(5.0, 6.0, 7.0, 8.0),
        "mul_vec4(e1) selects column 1 (column-major)"
    );
}

#[test]
fn mat4_mul_mat4_identity_and_compose() {
    let m = Mat4::from_cols(
        Vec4::new(2.0, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 3.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 4.0, 0.0),
        Vec4::new(1.0, 2.0, 3.0, 1.0),
    );
    assert_eq!(m.mul_mat4(Mat4::IDENTITY), m, "m * I = m");
    assert_eq!(Mat4::IDENTITY.mul_mat4(m), m, "I * m = m");
    // Operator * agrees with mul_mat4.
    assert_eq!(m * Mat4::IDENTITY, m.mul_mat4(Mat4::IDENTITY), "operator *");
    // Composition matches applying the two transforms in sequence to a vector.
    let n = Mat4::from_cols(
        Vec4::new(1.0, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 1.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0, 0.0),
        Vec4::new(10.0, 20.0, 30.0, 1.0),
    );
    let v = Vec4::new(1.0, 1.0, 1.0, 1.0);
    let composed = (m * n).mul_vec4(v);
    let sequential = m.mul_vec4(n.mul_vec4(v));
    assert!(
        close(composed.x, sequential.x, EPS)
            && close(composed.y, sequential.y, EPS)
            && close(composed.z, sequential.z, EPS)
            && close(composed.w, sequential.w, EPS),
        "(m*n).v == m.(n.v): {composed:?} vs {sequential:?}"
    );
}

#[test]
fn mat4_perspective_rh_maps_depth_zero_to_one() {
    // RH perspective, depth in [0,1]: a point at -near maps to clip z = 0,
    // a point at -far maps to clip z = far (w = far) i.e. ndc z = 1.
    let near = 0.1;
    let far = 100.0;
    let p = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, near, far);
    let at_near = p.mul_vec4(Vec4::new(0.0, 0.0, -near, 1.0));
    assert!(
        close(at_near.z / at_near.w, 0.0, 1e-4),
        "ndc z at near plane = 0: {}",
        at_near.z / at_near.w
    );
    let at_far = p.mul_vec4(Vec4::new(0.0, 0.0, -far, 1.0));
    assert!(
        close(at_far.z / at_far.w, 1.0, 1e-4),
        "ndc z at far plane = 1: {}",
        at_far.z / at_far.w
    );
}

#[test]
fn mat4_orthographic_rh_maps_corners() {
    let m = Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 0.0, 1.0);
    // The right plane x=1 maps to ndc x = +1, left x=-1 to -1.
    let right = m.mul_vec4(Vec4::new(1.0, 0.0, 0.0, 1.0));
    let left = m.mul_vec4(Vec4::new(-1.0, 0.0, 0.0, 1.0));
    assert!(close(right.x, 1.0, EPS), "x=1 -> ndc +1");
    assert!(close(left.x, -1.0, EPS), "x=-1 -> ndc -1");
    // near (z=0) -> ndc z = 0, far (z=-1) -> ndc z = 1.
    let near = m.mul_vec4(Vec4::new(0.0, 0.0, 0.0, 1.0));
    let far = m.mul_vec4(Vec4::new(0.0, 0.0, -1.0, 1.0));
    assert!(close(near.z, 0.0, EPS), "near z -> 0");
    assert!(close(far.z, 1.0, EPS), "far z -> 1");
}

// ----------------------------------------------------------------------------
// Affine3A + the row-major <-> column-major boundary
// ----------------------------------------------------------------------------

#[test]
fn affine_identity_default_and_transform() {
    assert_eq!(Affine3A::default(), Affine3A::IDENTITY, "default is identity");
    let p = Vec3::new(3.0, -4.0, 5.0);
    assert_eq!(Affine3A::IDENTITY.transform_point(p), p, "identity point");
    assert_eq!(Affine3A::IDENTITY.transform_vector(p), p, "identity vector");
}

#[test]
fn affine_trs_applies_scale_rotate_translate() {
    let t = Vec3::new(10.0, 20.0, 30.0);
    let r = Quat::IDENTITY;
    let s = Vec3::new(2.0, 3.0, 4.0);
    let a = Affine3A::from_translation_rotation_scale(t, r, s);
    // Point (1,1,1): scale -> (2,3,4), rotate (identity) -> same, translate.
    assert_eq!(
        a.transform_point(Vec3::new(1.0, 1.0, 1.0)),
        Vec3::new(12.0, 23.0, 34.0),
        "T.R.S point"
    );
    // transform_vector ignores translation.
    assert_eq!(
        a.transform_vector(Vec3::new(1.0, 1.0, 1.0)),
        Vec3::new(2.0, 3.0, 4.0),
        "vector ignores translation"
    );
}

#[test]
fn affine_compose_matches_sequential_transform() {
    let outer = Affine3A::from_translation_rotation_scale(
        Vec3::new(1.0, 2.0, 3.0),
        Quat::new(0.0, 0.0, 0.3826834, 0.9238795), // 45deg z
        Vec3::new(1.5, 1.5, 1.5),
    );
    let inner = Affine3A::from_translation_rotation_scale(
        Vec3::new(-2.0, 0.5, 4.0),
        Quat::new(0.5, 0.5, 0.5, 0.5),
        Vec3::new(2.0, 1.0, 0.5),
    );
    let p = Vec3::new(1.0, -1.0, 2.0);
    // (outer . inner)(p) == outer(inner(p)).
    let composed = outer.mul(inner).transform_point(p);
    let sequential = outer.transform_point(inner.transform_point(p));
    assert!(
        vec3_close(composed, sequential, 1e-4),
        "compose matches sequential: {composed:?} vs {sequential:?}"
    );
}

#[test]
fn affine_inverse_round_trip() {
    let a = Affine3A::from_translation_rotation_scale(
        Vec3::new(5.0, -3.0, 2.0),
        Quat::new(0.1, -0.2, 0.3, 0.9).normalize(),
        Vec3::new(2.0, 0.5, 3.0),
    );
    let inv = a.inverse().expect("non-singular");
    let p = Vec3::new(7.0, 8.0, -9.0);
    let round = inv.transform_point(a.transform_point(p));
    assert!(
        vec3_close(round, p, 1e-3),
        "inv(a(p)) = p: {round:?} vs {p:?}"
    );
}

#[test]
fn affine_inverse_singular_is_none() {
    let a = Affine3A::from_translation_rotation_scale(
        Vec3::new(1.0, 2.0, 3.0),
        Quat::IDENTITY,
        Vec3::new(0.0, 1.0, 1.0), // zero x-scale -> singular linear part
    );
    assert!(a.inverse().is_none(), "singular affine inverse is None");
}

#[test]
fn affine_to_mat4_transposes_linear_and_embeds_translation() {
    // THE convention boundary: row-major Mat3 -> column-major Mat4.
    // cols[j] = (rows[0][j], rows[1][j], rows[2][j], 0); translation is col 3.
    let m3 = Mat3::from_rows(
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(4.0, 5.0, 6.0),
        Vec3::new(7.0, 8.0, 9.0),
    );
    let a = Affine3A {
        matrix3: m3,
        translation: Vec3::new(10.0, 11.0, 12.0),
    };
    let m4 = a.to_mat4();
    // Column 0 of the Mat4 = first COLUMN of the row-major Mat3 = (1,4,7).
    assert_eq!(m4.cols[0], Vec4::new(1.0, 4.0, 7.0, 0.0), "col0 = m3 col0");
    assert_eq!(m4.cols[1], Vec4::new(2.0, 5.0, 8.0, 0.0), "col1 = m3 col1");
    assert_eq!(m4.cols[2], Vec4::new(3.0, 6.0, 9.0, 0.0), "col2 = m3 col2");
    assert_eq!(
        m4.cols[3],
        Vec4::new(10.0, 11.0, 12.0, 1.0),
        "col3 = (translation, 1)"
    );
    // from_affine is the same operation.
    assert_eq!(Mat4::from_affine(a), m4, "from_affine == to_mat4");
}

#[test]
fn affine_to_mat4_transforms_point_identically() {
    // The Mat4 (applied to a homogeneous point) must reproduce the affine's
    // transform_point exactly — the conversion preserves geometry.
    let a = Affine3A::from_translation_rotation_scale(
        Vec3::new(3.0, -1.0, 4.0),
        Quat::new(0.2, 0.3, -0.4, 0.8).normalize(),
        Vec3::new(1.5, 2.0, 0.75),
    );
    let m4 = a.to_mat4();
    for &p in &[
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 2.0, -3.0),
        Vec3::new(-5.0, 4.0, 1.0),
    ] {
        let affine_pt = a.transform_point(p);
        let mat4_pt = m4.mul_vec4(Vec4::from_vec3(p, 1.0));
        assert!(
            close(mat4_pt.w, 1.0, EPS),
            "homogeneous w stays 1: {}",
            mat4_pt.w
        );
        assert!(
            vec3_close(mat4_pt.xyz(), affine_pt, 1e-4),
            "Mat4 point == affine point for p={p:?}: {:?} vs {affine_pt:?}",
            mat4_pt.xyz()
        );
    }
}

#[test]
fn affine_payload_layout_is_repr_c_aligned() {
    // 3 x Vec3 (Mat3, 36 B) + Vec3 (12 B) = 48 B, 16-aligned.
    assert_eq!(core::mem::align_of::<Affine3A>(), 16, "Affine3A 16-aligned");
    assert_eq!(core::mem::size_of::<Affine3A>(), 48, "Affine3A is 48 bytes");
}
