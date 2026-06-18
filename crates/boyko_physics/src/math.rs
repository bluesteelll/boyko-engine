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

    /// `true` when every component is finite (no `NaN`, no `±Inf`).
    ///
    /// Used by the SDF narrowphase to reject a degenerate/non-finite field
    /// gradient (defense-in-depth alongside the zero-length seam-skip) before it
    /// can emit a `NaN`-normal contact that would poison the solver.
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
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

    /// Reads the component at axis `axis` (`0 = x`, `1 = y`, `2 = z`).
    ///
    /// Used by the box narrowphase (P2 W4) to address the principal axis a SAT
    /// face axis selects without a per-axis `match`.
    ///
    /// # Panics (debug only)
    ///
    /// `debug_assert!`s `axis < 3`; an out-of-range axis is a narrowphase bug.
    #[inline]
    pub fn axis(self, axis: usize) -> f32 {
        debug_assert!(axis < 3, "invariant: Vec3 axis index must be 0..3");
        match axis {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }

    /// Component-wise (Hadamard) product `(self.x·rhs.x, …)`.
    ///
    /// Used to scale a box's local half-extents by a per-axis sign vector when
    /// building OBB corners (P2 W4).
    #[inline]
    pub fn componentwise_mul(self, rhs: Self) -> Self {
        Self {
            x: self.x * rhs.x,
            y: self.y * rhs.y,
            z: self.z * rhs.z,
        }
    }

    /// Per-component absolute value `(|x|, |y|, |z|)`.
    #[inline]
    pub fn abs(self) -> Self {
        Self {
            x: self.x.abs(),
            y: self.y.abs(),
            z: self.z.abs(),
        }
    }

    /// Per-component clamp into `[-limit.x, limit.x] × …` (a symmetric box).
    ///
    /// The sphere-box closest-point uses this to clamp a sphere center expressed
    /// in a box's local frame to the box's half-extents (P2 W4).
    #[inline]
    pub fn clamp_symmetric(self, limit: Self) -> Self {
        Self {
            x: self.x.clamp(-limit.x, limit.x),
            y: self.y.clamp(-limit.y, limit.y),
            z: self.z.clamp(-limit.z, limit.z),
        }
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

    /// The conjugate `(−x, −y, −z, w)` — the INVERSE rotation for a unit
    /// quaternion (`q⁻¹ = q̄ / |q|²`, and `|q| = 1` for an orientation).
    #[inline]
    pub fn conjugate(self) -> Self {
        Self {
            x: -self.x,
            y: -self.y,
            z: -self.z,
            w: self.w,
        }
    }

    /// Rotates `v` by the INVERSE of this orientation (`v' = q⁻¹ · v · q`).
    ///
    /// For a unit `self` this is [`conjugate`](Self::conjugate)`.rotate(v)` — it
    /// maps a WORLD-frame vector into the body's LOCAL frame, which the box
    /// narrowphase (P2 W4) uses to express a sphere center / another box in an
    /// OBB's local axes.
    #[inline]
    pub fn inverse_rotate(self, v: Vec3) -> Vec3 {
        self.conjugate().rotate(v)
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

    /// A diagonal matrix `diag(d.x, d.y, d.z)`.
    ///
    /// The local inverse inertia of a principal-axis-aligned shape is diagonal
    /// (sphere, box); the gather builds it here, then rotates it into world
    /// space via `R · self · Rᵀ` (see [`from_quat`](Self::from_quat)).
    #[inline]
    pub const fn from_diagonal(d: Vec3) -> Self {
        Self::from_rows(
            Vec3::new(d.x, 0.0, 0.0),
            Vec3::new(0.0, d.y, 0.0),
            Vec3::new(0.0, 0.0, d.z),
        )
    }

    /// The row-major rotation matrix of the (unit) quaternion `q`.
    ///
    /// Built so that [`mul_vec`](Self::mul_vec) (per-row dot) reproduces
    /// [`Quat::rotate`](Quat::rotate): `Mat3::from_quat(q).mul_vec(v) ==
    /// q.rotate(v)` for a unit `q`. With `q = (x, y, z, w)` the standard
    /// rotation matrix is
    ///
    /// ```text
    /// | 1-2(y²+z²)   2(xy-wz)    2(xz+wy)  |
    /// | 2(xy+wz)    1-2(x²+z²)   2(yz-wx)  |
    /// | 2(xz-wy)     2(yz+wx)   1-2(x²+y²) |
    /// ```
    ///
    /// where `rows[i]` is row `i` (matching the module's row-major convention).
    /// `q` is assumed unit (the crate keeps orientations normalized); a
    /// non-unit `q` scales the result by `|q|²`.
    #[inline]
    pub fn from_quat(q: Quat) -> Self {
        let (x, y, z, w) = (q.x, q.y, q.z, q.w);
        let (xx, yy, zz) = (x * x, y * y, z * z);
        let (xy, xz, yz) = (x * y, x * z, y * z);
        let (wx, wy, wz) = (w * x, w * y, w * z);
        Self::from_rows(
            Vec3::new(
                1.0 - 2.0 * (yy + zz),
                2.0 * (xy - wz),
                2.0 * (xz + wy),
            ),
            Vec3::new(
                2.0 * (xy + wz),
                1.0 - 2.0 * (xx + zz),
                2.0 * (yz - wx),
            ),
            Vec3::new(
                2.0 * (xz - wy),
                2.0 * (yz + wx),
                1.0 - 2.0 * (xx + yy),
            ),
        )
    }

    /// The transpose `selfᵀ` (rows become columns).
    ///
    /// For a rotation matrix `R`, `Rᵀ == R⁻¹`; the world inverse inertia is
    /// `R · I⁻¹_local · Rᵀ`.
    #[inline]
    pub fn transpose(self) -> Self {
        let r = &self.rows;
        Self::from_rows(
            Vec3::new(r[0].x, r[1].x, r[2].x),
            Vec3::new(r[0].y, r[1].y, r[2].y),
            Vec3::new(r[0].z, r[1].z, r[2].z),
        )
    }
}

impl Mul for Mat3 {
    type Output = Self;

    /// Matrix product `self · rhs` (row-major: `out[i][j] = Σ_k a[i][k]·b[k][j]`).
    ///
    /// Each output row `i` is `self.rows[i]` left-multiplied by `rhs`: column
    /// `j` of the output is `self.rows[i] · rhs.column(j)`. Composing the
    /// world-inertia rotation `R · I⁻¹_local · Rᵀ` uses this directly.
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        // Each output row is `a` (a row of `self`) combined with `rhs`'s
        // columns; with row-major storage, column `j` of `rhs` is
        // `(rhs.rows[0][j], rhs.rows[1][j], rhs.rows[2][j])`.
        let row = |a: Vec3| {
            Vec3::new(
                a.x * rhs.rows[0].x + a.y * rhs.rows[1].x + a.z * rhs.rows[2].x,
                a.x * rhs.rows[0].y + a.y * rhs.rows[1].y + a.z * rhs.rows[2].y,
                a.x * rhs.rows[0].z + a.y * rhs.rows[1].z + a.z * rhs.rows[2].z,
            )
        };
        Self::from_rows(row(self.rows[0]), row(self.rows[1]), row(self.rows[2]))
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

    /// `from_diagonal` builds `diag(d)` — a diagonal scale.
    #[test]
    fn mat3_from_diagonal() {
        let d = Vec3::new(2.0, 3.0, 4.0);
        let m = Mat3::from_diagonal(d);
        let v = Vec3::new(5.0, 6.0, 7.0);
        assert_eq!(
            m.mul_vec(v),
            Vec3::new(d.x * v.x, d.y * v.y, d.z * v.z)
        );
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
        assert!((world.rows[0].y - world.rows[1].x).abs() < 1e-5, "M[0][1] == M[1][0]");
        assert!((world.rows[0].z - world.rows[2].x).abs() < 1e-5, "M[0][2] == M[2][0]");
        assert!((world.rows[1].z - world.rows[2].y).abs() < 1e-5, "M[1][2] == M[2][1]");
    }
}
