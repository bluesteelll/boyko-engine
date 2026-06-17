//! Per-contact constraint math for the TGS-Soft solver (P2 W2) — pure functions,
//! no allocation, unit-testable in isolation.
//!
//! These are the genuinely-new 3D pieces the [`SoftStepSolver`](super::SoftStepSolver)
//! builds on: the degeneracy-safe contact [`tangent_basis`], the angular-aware
//! normal [`effective_mass`], and the [`BodyEffective`] per-substep view that lets
//! the solve read/write a static body and a dynamic body through one uniform
//! branchless path (a static body has `inv_mass == 0` and `inv_inertia ==
//! Mat3::ZERO`, so it contributes `0` to the effective mass and ignores any
//! applied impulse).

use crate::math::{Mat3, Vec3};

/// Builds an orthonormal tangent basis `(t1, t2)` for the unit contact normal
/// `n` (P2 W2 — the friction plane).
///
/// Returns two unit vectors that, with `n`, form a right-handed orthonormal
/// frame: `t1 ⟂ n`, `t2 = n × t1` (already unit because `n` and `t1` are unit
/// and orthogonal). The friction impulse lives in the `(t1, t2)` plane.
///
/// # Degeneracy guard
///
/// The naive `t1 = n × ẑ` is the ZERO vector when `n ≈ ±ẑ` (a vertical floor
/// normal — the common resting case), which would collapse the friction plane.
/// This picks the seed axis away from `n`: cross with `ẑ` unless `n` is nearly
/// parallel to `ẑ` (`|n.z| ≥ 0.999`), in which case it crosses with `x̂` instead,
/// guaranteeing a non-zero `t1` for every unit `n`.
#[inline]
pub fn tangent_basis(n: Vec3) -> (Vec3, Vec3) {
    // Seed away from `n`: `n × ẑ` is non-degenerate unless `n ∥ ẑ`, where `n × x̂`
    // is (the two seeds are never both parallel to `n`).
    let seed = if n.z.abs() < 0.999 {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    let t1 = n.cross(seed).normalize();
    // `t2 = n × t1` is unit: `n ⟂ t1` and both are unit ⇒ `|n × t1| = 1`.
    let t2 = n.cross(t1);
    (t1, t2)
}

/// A per-substep solver view of one body — the uniform read/write surface the
/// constraint solve drives (P2 W2).
///
/// Carries the inverse mass, the WORLD inverse inertia (re-rotated each substep
/// from the body's local tensor + current orientation), and the live linear /
/// angular velocity. A static or `inv_mass == 0` body has `inv_mass == 0.0` and
/// `inv_inertia == Mat3::ZERO`, so [`apply_impulse`](Self::apply_impulse) is a
/// branchless no-op for it and [`effective_mass`] sees a zero contribution — no
/// per-body `if static` branch in the hot loop.
#[derive(Clone, Copy, Debug)]
pub struct BodyEffective {
    /// Inverse mass (`0` = immovable).
    pub inv_mass: f32,
    /// WORLD inverse inertia tensor `R · I⁻¹_local · Rᵀ` for the current
    /// orientation (refreshed per substep by the solver).
    pub inv_inertia: Mat3,
    /// Linear velocity (mutated by [`apply_impulse`](Self::apply_impulse)).
    pub linear_velocity: Vec3,
    /// Angular velocity (mutated by [`apply_impulse`](Self::apply_impulse)).
    pub angular_velocity: Vec3,
}

impl BodyEffective {
    /// The velocity of the material point at world offset `r` from the center of
    /// mass: `v + ω × r`.
    #[inline]
    pub fn point_velocity(&self, r: Vec3) -> Vec3 {
        self.linear_velocity + self.angular_velocity.cross(r)
    }

    /// Applies a world impulse `p` at the offset `r` from the center of mass:
    /// `v += inv_mass · p`, `ω += I⁻¹ · (r × p)`.
    ///
    /// Branchless for a static body: `inv_mass == 0` and `inv_inertia ==
    /// Mat3::ZERO` make both updates the zero vector, so an immovable body
    /// ignores the impulse without a guard.
    #[inline]
    pub fn apply_impulse(&mut self, r: Vec3, p: Vec3) {
        self.linear_velocity = self.linear_velocity + p * self.inv_mass;
        self.angular_velocity = self.angular_velocity + self.inv_inertia.mul_vec(r.cross(p));
    }
}

/// The effective mass of a contact constraint along the unit direction `dir`,
/// accounting for both bodies' linear AND angular response (P2 W2).
///
/// Returns `mEff = 1 / k` where
///
/// ```text
/// rdA = rA × dir;  rdB = rB × dir
/// k   = invMassA + invMassB
///     + dir · ((I⁻¹_world_A · rdA) × rA)
///     + dir · ((I⁻¹_world_B · rdB) × rB)
/// ```
///
/// (`0` when `k ≤ 0`, e.g. two static bodies — no constraint response). This is
/// the standard sequential-impulse denominator used for both the normal solve
/// (`dir = n`) and each friction-tangent solve (`dir = t1` / `dir = t2`); a
/// static body contributes `0` to `k` through its zero `inv_mass` / `inv_inertia`.
#[inline]
pub fn effective_mass(
    dir: Vec3,
    ra: Vec3,
    rb: Vec3,
    body_a: &BodyEffective,
    body_b: &BodyEffective,
) -> f32 {
    // Angular term for one body: `dir · ((I⁻¹ · (r × dir)) × r)`.
    let angular = |inv_inertia: Mat3, r: Vec3| {
        let rd = r.cross(dir);
        dir.dot(inv_inertia.mul_vec(rd).cross(r))
    };
    let k = body_a.inv_mass
        + body_b.inv_mass
        + angular(body_a.inv_inertia, ra)
        + angular(body_b.inv_inertia, rb);
    if k > 0.0 { 1.0 / k } else { 0.0 }
}

#[cfg(test)]
mod tests {
    //! Thread-free unit tests for the genuinely-new 3D solver math (P2 W2).
    //!
    //! These drive `tangent_basis`, `effective_mass`, and the `BodyEffective`
    //! point-velocity / apply-impulse pair purely through `core` f32 arithmetic —
    //! NO threadpool, NO schedule — so they run both as native `cargo test` and
    //! under `cargo +nightly miri test --lib` (the schedule-driven W2 acceptance
    //! tests in `tests/softstep.rs` spawn a worker thread and are native-only).
    //! W2 has ZERO `unsafe`, so Miri is only asserting the math touches no UB.

    use super::*;

    /// A dynamic body view at rest with isotropic inverse inertia `i` on the
    /// diagonal and the given inverse mass.
    fn dyn_body(inv_mass: f32, i: f32) -> BodyEffective {
        BodyEffective {
            inv_mass,
            inv_inertia: Mat3::from_diagonal(Vec3::new(i, i, i)),
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
        }
    }

    /// An immovable (static) body view: zero inverse mass + zero inertia.
    fn static_body() -> BodyEffective {
        BodyEffective {
            inv_mass: 0.0,
            inv_inertia: Mat3::ZERO,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
        }
    }

    #[test]
    fn tangent_basis_is_orthonormal_for_floor_normal() {
        // The +z floor normal is the degeneracy case `cross(n, z) == 0`.
        let n = Vec3::new(0.0, 0.0, 1.0);
        let (t1, t2) = tangent_basis(n);
        assert!((t1.length() - 1.0).abs() < 1e-6, "t1 unit, got {}", t1.length());
        assert!((t2.length() - 1.0).abs() < 1e-6, "t2 unit, got {}", t2.length());
        assert!(t1.dot(n).abs() < 1e-6, "t1 ⟂ n");
        assert!(t2.dot(n).abs() < 1e-6, "t2 ⟂ n");
        assert!(t1.dot(t2).abs() < 1e-6, "t1 ⟂ t2");
    }

    #[test]
    fn tangent_basis_is_right_handed() {
        // `t2 = n × t1`, so `n · (t1 × t2) > 0` (a right-handed frame).
        for &n in &[
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(1.0, 2.0, 3.0).normalize(),
        ] {
            let (t1, t2) = tangent_basis(n);
            assert!(
                n.dot(t1.cross(t2)) > 0.0,
                "right-handed frame for n={n:?}, triple={}",
                n.dot(t1.cross(t2))
            );
        }
    }

    #[test]
    fn effective_mass_two_unit_point_masses_is_half() {
        // Two equal unit-mass bodies (inv_mass 1, no angular contribution: the
        // anchor is along the normal so `r × n == 0`). `k = 1 + 1 = 2`,
        // `mEff = 0.5`.
        let a = dyn_body(1.0, 0.0);
        let b = dyn_body(1.0, 0.0);
        let n = Vec3::new(1.0, 0.0, 0.0);
        let r = Vec3::new(1.0, 0.0, 0.0); // parallel to n ⇒ zero angular term
        let m = effective_mass(n, r, r, &a, &b);
        assert!((m - 0.5).abs() < 1e-6, "mEff for two unit masses: {m}");
    }

    #[test]
    fn effective_mass_one_sided_against_static_is_dynamic_mass() {
        // A dynamic unit mass against a static body: the static side contributes
        // 0 to `k`, so `k = 1`, `mEff = 1`.
        let dyn_ = dyn_body(1.0, 0.0);
        let stat = static_body();
        let n = Vec3::new(0.0, 1.0, 0.0);
        let r = Vec3::new(0.0, 1.0, 0.0);
        let m = effective_mass(n, r, r, &dyn_, &stat);
        assert!((m - 1.0).abs() < 1e-6, "one-sided mEff: {m}");
    }

    #[test]
    fn effective_mass_two_static_bodies_is_zero() {
        // No constraint response between two immovable bodies (`k <= 0`).
        let a = static_body();
        let b = static_body();
        let n = Vec3::new(1.0, 0.0, 0.0);
        let r = Vec3::new(0.0, 1.0, 0.0);
        assert_eq!(effective_mass(n, r, r, &a, &b), 0.0);
    }

    #[test]
    fn effective_mass_angular_term_lowers_effective_mass() {
        // A lever-arm anchor perpendicular to the normal engages the angular term,
        // so `k` grows and `mEff` drops below the pure-linear `0.5`.
        let a = dyn_body(1.0, 1.0);
        let b = dyn_body(1.0, 1.0);
        let n = Vec3::new(0.0, 1.0, 0.0);
        let r = Vec3::new(1.0, 0.0, 0.0); // ⟂ n ⇒ non-zero `r × n`
        let m = effective_mass(n, r, r, &a, &b);
        assert!(m < 0.5, "angular term must lower mEff below 0.5, got {m}");
        assert!(m > 0.0, "mEff still positive, got {m}");
    }

    #[test]
    fn point_velocity_includes_angular_contribution() {
        // v + ω × r: ω about +z, r along +x ⇒ tangential velocity along +y.
        let b = BodyEffective {
            inv_mass: 1.0,
            inv_inertia: Mat3::ZERO,
            linear_velocity: Vec3::new(0.0, 0.0, 0.0),
            angular_velocity: Vec3::new(0.0, 0.0, 2.0),
        };
        let v = b.point_velocity(Vec3::new(1.0, 0.0, 0.0));
        assert!((v.y - 2.0).abs() < 1e-6, "ω×r gives +2 along y, got {v:?}");
        assert!(v.x.abs() < 1e-6 && v.z.abs() < 1e-6);
    }

    #[test]
    fn apply_impulse_static_body_is_noop() {
        // inv_mass == 0 + inv_inertia == ZERO ⇒ an impulse moves nothing.
        let mut b = static_body();
        b.apply_impulse(Vec3::new(0.0, 1.0, 0.0), Vec3::new(10.0, 20.0, 30.0));
        assert_eq!(b.linear_velocity, Vec3::ZERO, "static linear unchanged");
        assert_eq!(b.angular_velocity, Vec3::ZERO, "static angular unchanged");
    }

    #[test]
    fn apply_impulse_dynamic_body_updates_linear_and_angular() {
        // Pure linear at the center of mass (r == 0 ⇒ no torque): v += inv_mass·p.
        let mut b = dyn_body(0.5, 1.0);
        b.apply_impulse(Vec3::ZERO, Vec3::new(4.0, 0.0, 0.0));
        assert!((b.linear_velocity.x - 2.0).abs() < 1e-6, "v += 0.5·4 = 2");
        assert_eq!(b.angular_velocity, Vec3::ZERO, "r == 0 ⇒ no torque");

        // Off-center impulse: r along +x, p along +y ⇒ torque about +z.
        let mut c = dyn_body(1.0, 1.0);
        c.apply_impulse(Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
        assert!(c.angular_velocity.z > 0.0, "torque spins about +z, got {:?}", c.angular_velocity);
    }
}
