//! 3D math primitives for the physics foundation (plan §3.5/§3.6, "2D→3D type
//! swap").
//!
//! [`Vec3`] is the single vector type the whole crate is built on; the 2D→3D
//! transition was deliberately a localized type swap here — only this module's
//! vector/orientation types and the manifold's `MAX_CONTACT_POINTS` const
//! change, so no pipeline/seam shape breaks (plan §3.6). The genuinely-3D math
//! ([`Quat`] orientation, [`Mat3`] inertia tensor) lives here too: these are
//! real new math, not aliases.
//!
//! # S1 migration (bit-deterministic)
//!
//! As of the standard-library S1 phase, [`Vec3`], [`Quat`], and [`Mat3`] are
//! **no longer defined here** — they were lifted **verbatim** (algorithm- and
//! instruction-identical) into the shared [`boyko_math`] leaf crate and are
//! **re-exported** below. The physics public API and every `crate::math::{..}`
//! call site are therefore unchanged, and the migrated math is **bit-for-bit
//! identical** to the former in-crate definitions: `normalize` is still
//! `len_sq.sqrt().recip()` (exact `sqrt`, NOT `rsqrt`), the [`Quat::integrate`]
//! order is unchanged, and no FMA/`mul_add`/fast-math was introduced. The
//! physics determinism test suite (the `.to_bits()`-equality O9 SDF / signed-zero
//! cases) is the safety net and stays green with zero edits. `MAX_CONTACT_POINTS`
//! is physics-specific and remains defined here.

pub use boyko_math::{Mat3, Quat, Vec3};

/// Maximum contact points a single [`Manifold`](crate::manifold::Manifold)
/// stores (plan §3.6).
///
/// `4` is the 3D convex-convex contact-manifold maximum (the face-clipped
/// quad of a box-box / hull-hull contact; Box2D's 2D limit was `2`, the 3D
/// equivalent is `4`). It is a `const` so the `points` array and every loop
/// over it follow automatically.
pub const MAX_CONTACT_POINTS: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;

    /// `dot` / `cross` follow the right-handed convention (`x × y = z`).
    #[test]
    fn vec3_dot_and_cross() {
        let x = Vec3::new(1.0, 0.0, 0.0);
        let y = Vec3::new(0.0, 1.0, 0.0);
        let z = Vec3::new(0.0, 0.0, 1.0);
        assert_eq!(x.dot(y), 0.0);
        assert_eq!(x.dot(x), 1.0);
        assert_eq!(x.cross(y), z);
        assert_eq!(y.cross(z), x);
        assert_eq!(z.cross(x), y);
        // Anti-commutative.
        assert_eq!(y.cross(x), z * -1.0);
    }

    /// `normalize` produces a unit vector and is zero-guarded.
    #[test]
    fn vec3_normalize() {
        let v = Vec3::new(3.0, 4.0, 0.0);
        let n = v.normalize();
        assert!((n.length() - 1.0).abs() < 1e-6);
        assert_eq!(Vec3::ZERO.normalize(), Vec3::ZERO);
    }

    /// The identity orientation leaves any vector unchanged.
    #[test]
    fn quat_identity_rotate_is_noop() {
        let v = Vec3::new(1.0, -2.0, 3.0);
        assert_eq!(Quat::IDENTITY.rotate(v), v);
    }

    /// `normalize` produces a unit quaternion and is zero-guarded to identity.
    #[test]
    fn quat_normalize() {
        let q = Quat::new(0.0, 0.0, 2.0, 0.0).normalize();
        let len = (q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w).sqrt();
        assert!((len - 1.0).abs() < 1e-6);
        assert_eq!(Quat::new(0.0, 0.0, 0.0, 0.0).normalize(), Quat::IDENTITY);
    }

    /// A 90° rotation about +z (built directly) maps +x toward +y.
    #[test]
    fn quat_rotate_known_angle() {
        let half = std::f32::consts::FRAC_PI_4; // θ/2 for θ = 90°
        let q = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let rotated = q.rotate(Vec3::new(1.0, 0.0, 0.0));
        assert!((rotated.x - 0.0).abs() < 1e-5);
        assert!((rotated.y - 1.0).abs() < 1e-5);
        assert!((rotated.z - 0.0).abs() < 1e-5);
    }

    /// `integrate` advances orientation: a +z angular velocity rotates a +x
    /// vector toward +y after a short step (sign + plane correct).
    #[test]
    fn quat_integrate_advances_orientation() {
        let omega = Vec3::new(0.0, 0.0, 1.0); // 1 rad/s about +z
        let dt = 0.1;
        let q = Quat::IDENTITY.integrate(omega, dt);
        // Still a unit quaternion.
        let len = (q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w).sqrt();
        assert!((len - 1.0).abs() < 1e-6);
        // +x rotates toward +y in the xy-plane (z stays ~0, y becomes positive).
        let rotated = q.rotate(Vec3::new(1.0, 0.0, 0.0));
        assert!(rotated.y > 0.0, "rotated.y should be positive: {rotated:?}");
        assert!(rotated.z.abs() < 1e-5, "rotation stays in xy-plane");
        assert!(rotated.x > 0.0, "small-angle: x still dominates");
    }

    /// Repeated `integrate` accumulates: 10 steps of 0.1 rad about +z ≈ 1 rad.
    #[test]
    fn quat_integrate_accumulates() {
        let omega = Vec3::new(0.0, 0.0, 1.0);
        let dt = 0.1;
        let mut q = Quat::IDENTITY;
        for _ in 0..10 {
            q = q.integrate(omega, dt);
        }
        let rotated = q.rotate(Vec3::new(1.0, 0.0, 0.0));
        // ~1 rad about +z: x ≈ cos(1) ≈ 0.54, y ≈ sin(1) ≈ 0.84 (first-order
        // integration drifts a little, so use a loose tolerance).
        assert!((rotated.x - 1.0_f32.cos()).abs() < 0.05);
        assert!((rotated.y - 1.0_f32.sin()).abs() < 0.05);
    }

    /// The identity matrix leaves any vector unchanged.
    #[test]
    fn mat3_identity_mul_vec() {
        let v = Vec3::new(5.0, -7.0, 11.0);
        assert_eq!(Mat3::IDENTITY.mul_vec(v), v);
        assert_eq!(Mat3::ZERO.mul_vec(v), Vec3::ZERO);
    }

    /// A non-trivial matrix multiplies as row · v.
    #[test]
    fn mat3_mul_vec() {
        let m = Mat3::from_rows(
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 3.0, 0.0),
            Vec3::new(0.0, 0.0, 4.0),
        );
        assert_eq!(m.mul_vec(Vec3::new(1.0, 1.0, 1.0)), Vec3::new(2.0, 3.0, 4.0));
    }

    /// `from_diagonal` builds `diag(d)` — a diagonal scale.
    #[test]
    fn mat3_from_diagonal() {
        let d = Vec3::new(2.0, 3.0, 4.0);
        let m = Mat3::from_diagonal(d);
        let v = Vec3::new(5.0, 6.0, 7.0);
        assert_eq!(m.mul_vec(v), Vec3::new(d.x * v.x, d.y * v.y, d.z * v.z));
    }

    /// `from_quat(IDENTITY)` is the identity matrix.
    #[test]
    fn mat3_from_quat_identity() {
        assert_eq!(Mat3::from_quat(Quat::IDENTITY), Mat3::IDENTITY);
    }

    /// A 90°-about-z quaternion yields the expected rotation matrix (+x → +y,
    /// +y → −x).
    #[test]
    fn mat3_from_quat_known_z90() {
        let half = std::f32::consts::FRAC_PI_4; // θ/2 for θ = 90°
        let q = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let m = Mat3::from_quat(q);
        // Expected R_z(90°) (row-major): [[0,-1,0],[1,0,0],[0,0,1]].
        let expected = Mat3::from_rows(
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        );
        for i in 0..3 {
            assert!((m.rows[i].x - expected.rows[i].x).abs() < 1e-6);
            assert!((m.rows[i].y - expected.rows[i].y).abs() < 1e-6);
            assert!((m.rows[i].z - expected.rows[i].z).abs() < 1e-6);
        }
    }

    /// `from_quat(q).mul_vec(v)` agrees with `q.rotate(v)` for several q, v —
    /// the matrix form must reproduce the quaternion rotation.
    #[test]
    fn mat3_from_quat_matches_rotate() {
        let quats = [
            Quat::IDENTITY,
            Quat::new(0.0, 0.0, 0.3826834, 0.9238795), // 45° about +z
            Quat::new(0.5, 0.5, 0.5, 0.5),             // 120° about (1,1,1)
            Quat::new(0.1, -0.2, 0.3, 0.9).normalize(),
        ];
        let vecs = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, -2.0, 3.0),
            Vec3::new(-4.0, 5.0, -6.0),
        ];
        for &q in &quats {
            let m = Mat3::from_quat(q);
            for &v in &vecs {
                let a = m.mul_vec(v);
                let b = q.rotate(v);
                assert!((a.x - b.x).abs() < 1e-5, "x: {a:?} vs {b:?}");
                assert!((a.y - b.y).abs() < 1e-5, "y: {a:?} vs {b:?}");
                assert!((a.z - b.z).abs() < 1e-5, "z: {a:?} vs {b:?}");
            }
        }
    }

    /// `transpose` is an involution (`m.transpose().transpose() == m`).
    #[test]
    fn mat3_transpose_involution() {
        let m = Mat3::from_rows(
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(4.0, 5.0, 6.0),
            Vec3::new(7.0, 8.0, 9.0),
        );
        assert_eq!(m.transpose().transpose(), m);
        // Transpose swaps off-diagonal elements.
        let t = m.transpose();
        assert_eq!(t.rows[0], Vec3::new(1.0, 4.0, 7.0));
        assert_eq!(t.rows[1], Vec3::new(2.0, 5.0, 8.0));
        assert_eq!(t.rows[2], Vec3::new(3.0, 6.0, 9.0));
    }

    /// The matrix product is associative with `mul_vec`:
    /// `(A·B).mul_vec(v) ≈ A.mul_vec(B.mul_vec(v))`.
    #[test]
    fn mat3_mul_agrees_with_mul_vec() {
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
        let v = Vec3::new(5.0, -3.0, 2.0);
        let lhs = (a * b).mul_vec(v);
        let rhs = a.mul_vec(b.mul_vec(v));
        assert!((lhs.x - rhs.x).abs() < 1e-5);
        assert!((lhs.y - rhs.y).abs() < 1e-5);
        assert!((lhs.z - rhs.z).abs() < 1e-5);
    }

    /// `IDENTITY` is the multiplicative identity for the matrix product.
    #[test]
    fn mat3_mul_identity() {
        let m = Mat3::from_rows(
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(4.0, 5.0, 6.0),
            Vec3::new(7.0, 8.0, 9.0),
        );
        assert_eq!(m * Mat3::IDENTITY, m);
        assert_eq!(Mat3::IDENTITY * m, m);
    }

    /// The inertia round-trip `R · I_local · Rᵀ` is symmetric, and equals
    /// `I_local` when `R == IDENTITY` (the gather's world-tensor construction).
    #[test]
    fn mat3_inertia_round_trip() {
        let i_local = Mat3::from_diagonal(Vec3::new(2.0, 5.0, 11.0));

        // R == IDENTITY ⇒ the world tensor equals the local tensor.
        let r_id = Mat3::from_quat(Quat::IDENTITY);
        let world_id = r_id * i_local * r_id.transpose();
        assert_eq!(world_id, i_local);

        // For any rotation R, R·I·Rᵀ is symmetric (I diagonal ⇒ symmetric).
        let q = Quat::new(0.2, -0.4, 0.5, 0.8).normalize();
        let r = Mat3::from_quat(q);
        let world = r * i_local * r.transpose();
        assert!(
            (world.rows[0].y - world.rows[1].x).abs() < 1e-5,
            "M[0][1] == M[1][0]"
        );
        assert!(
            (world.rows[0].z - world.rows[2].x).abs() < 1e-5,
            "M[0][2] == M[2][0]"
        );
        assert!(
            (world.rows[1].z - world.rows[2].y).abs() < 1e-5,
            "M[1][2] == M[2][1]"
        );
    }
}
