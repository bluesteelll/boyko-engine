//! Matrix primitives: [`Mat3`] (row-major) and [`Mat4`] (column-major).
//!
//! [`Mat3`] is **lifted verbatim** from the physics foundation and is
//! **row-major** (`rows[i]` is row `i`): `from_quat`/`mul`/`transpose`/`mul_vec`
//! are instruction-identical, so the migrated physics is bit-for-bit unchanged.
//!
//! [`Mat4`] is **new** and **column-major** to match the WGSL `mat4x4`
//! convention (so a `Mat4` uploads directly to a GPU uniform). The row-major ↔
//! column-major convention boundary is crossed in EXACTLY ONE place:
//! [`Mat4::from_affine`] / [`crate::affine::Affine3A::to_mat4`]. The new code
//! obeys the same no-FMA / exact-sqrt discipline as the lifted code.

use crate::quat::Quat;
use crate::vec::{Vec3, Vec4};

use std::ops::Mul;

/// A 3×3 matrix, **row-major** (`rows[i]` is row `i`, so `rows[i].x` is the
/// element at row `i`, column `0`).
///
/// `#[repr(C)]`. It holds a body's **inverse inertia tensor** in the physics
/// rigid-body mass column: a symmetric 3×3 that maps an angular impulse/velocity
/// to its angular response (`Δω = I⁻¹ · τ`). The solver applies it via
/// [`mul_vec`](Self::mul_vec). It is also the linear part of an
/// [`Affine3A`](crate::affine::Affine3A) (reusing these row-major ops verbatim).
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

    /// Constructs a matrix whose three **columns** are `c0`, `c1`, `c2`
    /// (left to right).
    ///
    /// `Mat3` is row-major, so the columns are stored transposed: row `i` is
    /// `(c0[i], c1[i], c2[i])`. This makes column `j` selectable by
    /// [`mul_vec`](Self::mul_vec) on the `j`-th unit axis
    /// (`from_columns(c0, c1, c2).mul_vec((1, 0, 0)) == c0`, etc.). The camera
    /// basis convention `ViewUniform::from_camera` reads is column-major
    /// (a local axis maps to a stored column), so a look-at basis is assembled
    /// here as columns rather than rows.
    #[inline]
    pub const fn from_columns(c0: Vec3, c1: Vec3, c2: Vec3) -> Self {
        Self::from_rows(
            Vec3::new(c0.x, c1.x, c2.x),
            Vec3::new(c0.y, c1.y, c2.y),
            Vec3::new(c0.z, c1.z, c2.z),
        )
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
    /// `q` is assumed unit (orientations are kept normalized); a non-unit `q`
    /// scales the result by `|q|²`.
    #[inline]
    pub fn from_quat(q: Quat) -> Self {
        let (x, y, z, w) = (q.x, q.y, q.z, q.w);
        let (xx, yy, zz) = (x * x, y * y, z * z);
        let (xy, xz, yz) = (x * y, x * z, y * z);
        let (wx, wy, wz) = (w * x, w * y, w * z);
        Self::from_rows(
            Vec3::new(1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy)),
            Vec3::new(2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx)),
            Vec3::new(2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy)),
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

    /// Builds a row-major linear part from a rotation `q` and a per-axis
    /// (non-uniform) `scale`: `R · diag(scale)`.
    ///
    /// Used by [`Affine3A::from_translation_rotation_scale`](crate::affine::Affine3A::from_translation_rotation_scale).
    /// Scaling the COLUMNS of `R` (i.e. `R · diag(s)`) scales the basis vectors,
    /// which for a row-major matrix means multiplying column `j` by `scale[j]`.
    /// No FMA: each element is a single `*`.
    #[inline]
    pub fn from_quat_scale(q: Quat, scale: Vec3) -> Self {
        let r = Self::from_quat(q);
        Self::from_rows(
            Vec3::new(
                r.rows[0].x * scale.x,
                r.rows[0].y * scale.y,
                r.rows[0].z * scale.z,
            ),
            Vec3::new(
                r.rows[1].x * scale.x,
                r.rows[1].y * scale.y,
                r.rows[1].z * scale.z,
            ),
            Vec3::new(
                r.rows[2].x * scale.x,
                r.rows[2].y * scale.y,
                r.rows[2].z * scale.z,
            ),
        )
    }

    /// The determinant of this row-major 3×3.
    ///
    /// Used by [`inverse`](Self::inverse) (general affine inverse). No FMA.
    #[inline]
    pub fn determinant(self) -> f32 {
        let r = &self.rows;
        r[0].x * (r[1].y * r[2].z - r[1].z * r[2].y)
            - r[0].y * (r[1].x * r[2].z - r[1].z * r[2].x)
            + r[0].z * (r[1].x * r[2].y - r[1].y * r[2].x)
    }

    /// The general inverse `self⁻¹` via the adjugate / determinant, or `None`
    /// when `self` is (near) singular.
    ///
    /// This is the GENERAL inverse (handles non-uniform scale / shear), used by
    /// [`Affine3A::inverse`](crate::affine::Affine3A::inverse). For a pure
    /// rotation, [`transpose`](Self::transpose) is the cheaper exact inverse.
    /// Bit-determinism is not load-bearing here (new code, not on the physics
    /// path), but it still obeys the no-FMA discipline.
    #[inline]
    pub fn inverse(self) -> Option<Self> {
        let r = &self.rows;
        // Cofactor matrix (the adjugate is its transpose).
        let c00 = r[1].y * r[2].z - r[1].z * r[2].y;
        let c01 = r[1].z * r[2].x - r[1].x * r[2].z;
        let c02 = r[1].x * r[2].y - r[1].y * r[2].x;
        let det = r[0].x * c00 + r[0].y * c01 + r[0].z * c02;
        if det == 0.0 {
            return None;
        }
        let inv_det = det.recip();
        let c10 = r[0].z * r[2].y - r[0].y * r[2].z;
        let c11 = r[0].x * r[2].z - r[0].z * r[2].x;
        let c12 = r[0].y * r[2].x - r[0].x * r[2].y;
        let c20 = r[0].y * r[1].z - r[0].z * r[1].y;
        let c21 = r[0].z * r[1].x - r[0].x * r[1].z;
        let c22 = r[0].x * r[1].y - r[0].y * r[1].x;
        // inv = adjugate / det = transpose(cofactor) * inv_det.
        Some(Self::from_rows(
            Vec3::new(c00 * inv_det, c10 * inv_det, c20 * inv_det),
            Vec3::new(c01 * inv_det, c11 * inv_det, c21 * inv_det),
            Vec3::new(c02 * inv_det, c12 * inv_det, c22 * inv_det),
        ))
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
    /// The default matrix is [`Mat3::IDENTITY`].
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// A 4×4 matrix, **column-major** (`cols[i]` is column `i`) — the WGSL `mat4x4`
/// convention, so a `Mat4` uploads directly to a GPU uniform.
///
/// NEW code (not on the physics determinism path). The ONLY place the engine
/// crosses the row-major ↔ column-major boundary is [`from_affine`](Self::from_affine).
/// All arithmetic obeys the no-FMA discipline.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    /// The four columns, left to right.
    pub cols: [Vec4; 4],
}

impl Mat4 {
    /// The identity matrix.
    pub const IDENTITY: Self = Self {
        cols: [
            Vec4::new(1.0, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 1.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 1.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, 1.0),
        ],
    };

    /// Constructs a matrix from its four columns (left to right).
    #[inline]
    pub const fn from_cols(c0: Vec4, c1: Vec4, c2: Vec4, c3: Vec4) -> Self {
        Self {
            cols: [c0, c1, c2, c3],
        }
    }

    /// Matrix-vector product `self · v` (column-major: a linear combination of
    /// the columns weighted by `v`'s components).
    #[inline]
    pub fn mul_vec4(self, v: Vec4) -> Vec4 {
        // out = c0·v.x + c1·v.y + c2·v.z + c3·v.w  (separate adds, no FMA).
        self.cols[0] * v.x + self.cols[1] * v.y + self.cols[2] * v.z + self.cols[3] * v.w
    }

    /// Matrix product `self · rhs` (column-major). Each output column `j` is
    /// `self · rhs.cols[j]`.
    #[inline]
    pub fn mul_mat4(self, rhs: Self) -> Self {
        Self::from_cols(
            self.mul_vec4(rhs.cols[0]),
            self.mul_vec4(rhs.cols[1]),
            self.mul_vec4(rhs.cols[2]),
            self.mul_vec4(rhs.cols[3]),
        )
    }

    /// A right-handed perspective projection mapping to clip space with depth in
    /// `[0, 1]` (the WGSL/Vulkan convention).
    ///
    /// `fov_y` is the vertical field of view in radians, `aspect = width/height`.
    /// No FMA: each element is a single mul or sub.
    #[inline]
    pub fn perspective_rh(fov_y: f32, aspect: f32, near: f32, far: f32) -> Self {
        let f = (fov_y * 0.5).tan().recip();
        let nf = (near - far).recip();
        // Column-major: the projection's nonzero entries placed per WGSL.
        Self::from_cols(
            Vec4::new(f / aspect, 0.0, 0.0, 0.0),
            Vec4::new(0.0, f, 0.0, 0.0),
            Vec4::new(0.0, 0.0, far * nf, -1.0),
            Vec4::new(0.0, 0.0, near * far * nf, 0.0),
        )
    }

    /// A right-handed orthographic projection mapping to clip space with depth
    /// in `[0, 1]`.
    ///
    /// No FMA: each element is a single mul / sub / reciprocal.
    #[inline]
    pub fn orthographic_rh(
        left: f32,
        right: f32,
        bottom: f32,
        top: f32,
        near: f32,
        far: f32,
    ) -> Self {
        let rl = (right - left).recip();
        let tb = (top - bottom).recip();
        let nf = (near - far).recip();
        Self::from_cols(
            Vec4::new(2.0 * rl, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 2.0 * tb, 0.0, 0.0),
            Vec4::new(0.0, 0.0, nf, 0.0),
            Vec4::new(
                -(right + left) * rl,
                -(top + bottom) * tb,
                near * nf,
                1.0,
            ),
        )
    }
}

impl Mul for Mat4 {
    type Output = Self;

    /// Matrix product `self · rhs` (see [`mul_mat4`](Self::mul_mat4)).
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        self.mul_mat4(rhs)
    }
}

impl Default for Mat4 {
    /// The default matrix is [`Mat4::IDENTITY`].
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}
