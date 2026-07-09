//! Quaternion orientation [`Quat`].
//!
//! **Lifted verbatim** from the physics foundation. The integration order
//! (build `ω̂` → one Hamilton product → scale → add → normalize), the Hamilton
//! product term order, and `normalize`'s `len_sq.sqrt().recip()` (exact `sqrt`,
//! NOT `rsqrt`) are all instruction-identical, so the migrated physics is
//! bit-for-bit unchanged. No `mul_add`/FMA/fast-math anywhere.

use crate::mat::Mat3;
use crate::vec::Vec3;

use std::ops::Mul;

/// A unit quaternion encoding a 3D orientation.
///
/// `#[repr(C)]` with the components in `(x, y, z, w)` order (vector part first,
/// scalar `w` last — the GPU/glTF convention, so a GPU mirror is a direct byte
/// copy). A unit quaternion `q = (x, y, z, w)` with `w = cos(θ/2)`,
/// `(x, y, z) = sin(θ/2)·axis` rotates a vector by the angle `θ` about `axis`.
/// Orientations are kept normalized (integration re-normalizes each step).
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

    /// Constructs a unit quaternion from a PROPER orthonormal rotation matrix
    /// (`det ≈ +1`) — the algebraic inverse of [`Mat3::from_quat`].
    ///
    /// Uses Shepperd's largest-diagonal branch selection: the branch whose pivot
    /// (the trace `t = m00+m11+m22`, or a diagonal element) is largest is taken,
    /// so the `sqrt` operand is never near zero (avoiding the catastrophic
    /// cancellation of the naive `w = sqrt(1+t)/2` when `t ≈ -1`). The four
    /// branches invert exactly the sign layout `from_quat` writes (mat.rs):
    /// with the row/column index convention `m_ij ≡ m.rows[i].component(j)`,
    /// `from_quat` places `+w·k` on the LOWER-left of each off-diagonal pair, so
    /// `m21 - m12 = 4·w·x`, `m02 - m20 = 4·w·y`, `m10 - m01 = 4·w·z`, and the
    /// vector-vector sums `m21 + m12 = 4·y·z`, `m02 + m20 = 4·x·z`,
    /// `m10 + m01 = 4·x·y`. Bit-determinism: literal `sqrt` (NOT `rsqrt`),
    /// no FMA. The result is normalized (cheap insurance against input drift).
    ///
    /// DEBUG-asserts the input is orthonormal (unit, mutually-orthogonal rows,
    /// `det ≈ +1`). On a degenerate (non-rotation) input it returns a normalized
    /// best effort rather than `NaN`.
    #[inline]
    pub fn from_mat3(m: Mat3) -> Self {
        // m_ij = m.rows[i].component(j) (row i, column j) — row-major Mat3.
        let m00 = m.rows[0].x;
        let m01 = m.rows[0].y;
        let m02 = m.rows[0].z;
        let m10 = m.rows[1].x;
        let m11 = m.rows[1].y;
        let m12 = m.rows[1].z;
        let m20 = m.rows[2].x;
        let m21 = m.rows[2].y;
        let m22 = m.rows[2].z;

        // The input must be a proper rotation; the branch math assumes it.
        debug_assert!(
            (m.determinant() - 1.0).abs() < 1.0e-2,
            "Quat::from_mat3: input is not a proper rotation (det != +1)"
        );

        let trace = m00 + m11 + m22;
        let q = if trace > 0.0 {
            // Trace branch (w-dominant / small angle): s = 4w.
            let s = (trace + 1.0).sqrt() * 2.0;
            let inv_s = s.recip();
            Self::new(
                (m21 - m12) * inv_s,
                (m02 - m20) * inv_s,
                (m10 - m01) * inv_s,
                s * 0.25,
            )
        } else if m00 > m11 && m00 > m22 {
            // x-diagonal branch (near-180° about X): s = 4x.
            let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
            let inv_s = s.recip();
            Self::new(
                s * 0.25,
                (m01 + m10) * inv_s,
                (m02 + m20) * inv_s,
                (m21 - m12) * inv_s,
            )
        } else if m11 > m22 {
            // y-diagonal branch (near-180° about Y): s = 4y.
            let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
            let inv_s = s.recip();
            Self::new(
                (m01 + m10) * inv_s,
                s * 0.25,
                (m12 + m21) * inv_s,
                (m02 - m20) * inv_s,
            )
        } else {
            // z-diagonal branch (near-180° about Z): s = 4z.
            let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
            let inv_s = s.recip();
            Self::new(
                (m02 + m20) * inv_s,
                (m12 + m21) * inv_s,
                s * 0.25,
                (m10 - m01) * inv_s,
            )
        };
        q.normalize()
    }

    /// Returns `self` scaled to unit length, or [`Quat::IDENTITY`] when `self`
    /// is (near) zero-length.
    ///
    /// Guards against a divide-by-zero on a degenerate quaternion (e.g. a
    /// hand-built all-zero value) rather than producing `NaN`. Bit-determinism:
    /// literally `len_sq.sqrt().recip()` — NOT a hardware `rsqrt`.
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
    /// method per the math surface so call sites read explicitly.
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
    /// narrowphase uses to express a sphere center / another box in an OBB's
    /// local axes.
    #[inline]
    pub fn inverse_rotate(self, v: Vec3) -> Vec3 {
        self.conjugate().rotate(v)
    }

    /// Advances this orientation by `angular_velocity` (world-frame, rad/s) over
    /// `dt` and re-normalizes (first-order quaternion integration).
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
        let omega = Self::new(angular_velocity.x, angular_velocity.y, angular_velocity.z, 0.0);
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
