//! 3D math primitives for the physics foundation (plan §3.5/§3.6, "2D→3D type
//! swap").
//!
//! [`Vec3`] is the single vector type the whole crate is built on; the 2D→3D
//! transition was deliberately a localized type swap here — only this module's
//! vector/orientation types and the manifold's `MAX_CONTACT_POINTS` const
//! change, so no pipeline/seam shape breaks (plan §3.6). The genuinely-3D math
//! ([`Quat`] orientation, [`Mat3`] inertia tensor) lives here too: these are
//! real new math, not aliases.

use std::ops::{Add, Mul, Sub};

/// Maximum contact points a single [`Manifold`](crate::manifold::Manifold)
/// stores (plan §3.6).
///
/// `4` is the 3D convex-convex contact-manifold maximum (the face-clipped
/// quad of a box-box / hull-hull contact; Box2D's 2D limit was `2`, the 3D
/// equivalent is `4`). It is a `const` so the `points` array and every loop
/// over it follow automatically.
pub const MAX_CONTACT_POINTS: usize = 4;

/// A 3D vector / point in world units.
///
/// `#[repr(C)]` so the byte layout is stable (it rides inside the POD
/// [`Manifold`](crate::manifold::Manifold) currency and the
/// [`RigidBody`](crate::components::RigidBody) component column).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
}

impl Vec3 {
    /// The zero vector.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// Constructs a vector from its components.
    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Dot product `self · other`.
    #[inline]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Cross product `self × other` (right-handed).
    ///
    /// Needed for the 3D angular terms (`r × impulse`, `ω × r`) the quaternion
    /// integration and a future Phase-10 solver build on.
    #[inline]
    pub fn cross(self, other: Self) -> Self {
        Self {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    /// Squared length (`self · self`). Cheaper than [`length`](Self::length)
    /// when only a comparison is needed — avoids the `sqrt`.
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    /// Euclidean length `|self|`.
    #[inline]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    /// Returns `self` scaled to unit length, or [`Vec3::ZERO`] when `self` is
    /// (near) zero-length.
    ///
    /// Guards against a divide-by-zero on a degenerate vector (the
    /// coincident-body case in narrowphase) rather than producing `NaN`.
    #[inline]
    pub fn normalize(self) -> Self {
        let len_sq = self.length_squared();
        if len_sq <= f32::MIN_POSITIVE {
            return Self::ZERO;
        }
        let inv_len = len_sq.sqrt().recip();
        self * inv_len
    }
}

impl Add for Vec3 {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}

impl Sub for Vec3 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z * rhs,
        }
    }
}

/// A unit quaternion encoding a 3D orientation (plan §3.5 — genuine 3D math).
///
/// `#[repr(C)]` with the components in `(x, y, z, w)` order (vector part first,
/// scalar `w` last — the GPU/glTF convention, so a later GPU mirror is a direct
/// byte copy). A unit quaternion `q = (x, y, z, w)` with
/// `w = cos(θ/2)`, `(x, y, z) = sin(θ/2)·axis` rotates a vector by the angle `θ`
/// about `axis`. The crate keeps orientations normalized (integration
/// re-normalizes each step).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    /// X component of the vector part (`sin(θ/2)·axis.x`).
    pub x: f32,
    /// Y component of the vector part (`sin(θ/2)·axis.y`).
    pub y: f32,
    /// Z component of the vector part (`sin(θ/2)·axis.z`).
    pub z: f32,
    /// Scalar part (`cos(θ/2)`).
    pub w: f32,
}

impl Quat {
    /// The identity orientation (no rotation).
    pub const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    /// Constructs a quaternion from its raw components (`x, y, z` vector part,
    /// `w` scalar part). Does NOT normalize — callers that need a unit
    /// quaternion call [`normalize`](Self::normalize).
    #[inline]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// Returns `self` scaled to unit length, or [`Quat::IDENTITY`] when `self`
    /// is (near) zero-length.
    ///
    /// Guards against a divide-by-zero on a degenerate quaternion (e.g. a
    /// hand-built all-zero value) rather than producing `NaN`.
    #[inline]
    pub fn normalize(self) -> Self {
        let len_sq = self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w;
        if len_sq <= f32::MIN_POSITIVE {
            return Self::IDENTITY;
        }
        let inv_len = len_sq.sqrt().recip();
        Self {
            x: self.x * inv_len,
            y: self.y * inv_len,
            z: self.z * inv_len,
            w: self.w * inv_len,
        }
    }

    /// Hamilton product `self * rhs` — composes two rotations (apply `rhs`
    /// first, then `self`, when used to [`rotate`](Self::rotate) a vector).
    ///
    /// Identical to the [`Mul`] operator (`self * rhs`); provided as a named
    /// method per the foundation's math surface so call sites read explicitly.
    // `clippy::should_implement_trait`: the `Mul` operator IS implemented below
    // (delegating here); this inherent `mul` is the named-method form the
    // crate's math surface intentionally exposes alongside the operator.
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn mul(self, rhs: Self) -> Self {
        // Standard Hamilton product for q1 = self, q2 = rhs:
        //   w = w1·w2 − x1·x2 − y1·y2 − z1·z2
        //   x = w1·x2 + x1·w2 + y1·z2 − z1·y2
        //   y = w1·y2 − x1·z2 + y1·w2 + z1·x2
        //   z = w1·z2 + x1·y2 − y1·x2 + z1·w2
        Self {
            x: self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            y: self.w * rhs.y - self.x * rhs.z + self.y * rhs.w + self.z * rhs.x,
            z: self.w * rhs.z + self.x * rhs.y - self.y * rhs.x + self.z * rhs.w,
            w: self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
        }
    }

    /// Rotates the vector `v` by this orientation, `v' = q · v · q⁻¹`.
    ///
    /// Uses the standard expanded form (two cross products) so a unit `self`
    /// rotates `v` without forming the conjugate explicitly:
    /// `v' = v + 2·w·(u × v) + 2·(u × (u × v))`, where `u = (x, y, z)`.
    #[inline]
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let u = Vec3::new(self.x, self.y, self.z);
        let t = u.cross(v) * 2.0;
        v + t * self.w + u.cross(t)
    }

    /// Advances this orientation by `angular_velocity` (world-frame, rad/s) over
    /// `dt` and re-normalizes (plan §3.5 — first-order quaternion integration).
    ///
    /// Computes `q_next = normalize(q + ½·ω̂·q·dt)`, where `ω̂` is the pure
    /// quaternion `(ω.x, ω.y, ω.z, 0)`. This is the standard first-order
    /// integration of the quaternion kinematic equation `q̇ = ½·ω̂·q`; the
    /// final `normalize` corrects the small length drift the linear step
    /// introduces. Determinism: the operation order is fixed (build `ω̂`, one
    /// Hamilton product, scale, add, normalize), so the float result is
    /// reproducible across runs for the same inputs.
    #[inline]
    pub fn integrate(self, angular_velocity: Vec3, dt: f32) -> Self {
        let omega = Self::new(
            angular_velocity.x,
            angular_velocity.y,
            angular_velocity.z,
            0.0,
        );
        let half_dt = 0.5 * dt;
        let delta = omega.mul(self);
        Self {
            x: self.x + delta.x * half_dt,
            y: self.y + delta.y * half_dt,
            z: self.z + delta.z * half_dt,
            w: self.w + delta.w * half_dt,
        }
        .normalize()
    }
}

impl Default for Quat {
    /// The default orientation is [`Quat::IDENTITY`] (not all-zero, which is not
    /// a valid rotation).
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mul for Quat {
    type Output = Self;

    /// Quaternion composition via the Hamilton product (see [`Quat::mul`]).
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        Quat::mul(self, rhs)
    }
}

/// A 3×3 matrix, **row-major** (`rows[i]` is row `i`, so `rows[i].x` is the
/// element at row `i`, column `0`).
///
/// `#[repr(C)]`. It holds a body's **inverse inertia tensor** in
/// [`RigidBodyMass`](crate::components::RigidBodyMass): a symmetric 3×3 that
/// maps an angular impulse/velocity to its angular response (`Δω = I⁻¹ · τ`).
/// A future Phase-10 solver applies it via [`mul_vec`](Self::mul_vec).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3 {
    /// The three rows, top to bottom.
    pub rows: [Vec3; 3],
}

impl Mat3 {
    /// The zero matrix (zero inverse inertia = infinite inertia, i.e. a body
    /// that does not respond to torque).
    pub const ZERO: Self = Self {
        rows: [Vec3::ZERO, Vec3::ZERO, Vec3::ZERO],
    };

    /// The identity matrix.
    pub const IDENTITY: Self = Self {
        rows: [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ],
    };

    /// Constructs a matrix from its three rows (top to bottom).
    #[inline]
    pub const fn from_rows(r0: Vec3, r1: Vec3, r2: Vec3) -> Self {
        Self { rows: [r0, r1, r2] }
    }

    /// Matrix-vector product `self · v` (each output component is a row · `v`).
    #[inline]
    pub fn mul_vec(self, v: Vec3) -> Vec3 {
        Vec3::new(
            self.rows[0].dot(v),
            self.rows[1].dot(v),
            self.rows[2].dot(v),
        )
    }
}

impl Default for Mat3 {
    /// The default inverse inertia tensor is [`Mat3::IDENTITY`].
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

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
        assert_eq!(
            m.mul_vec(Vec3::new(1.0, 1.0, 1.0)),
            Vec3::new(2.0, 3.0, 4.0)
        );
    }
}
