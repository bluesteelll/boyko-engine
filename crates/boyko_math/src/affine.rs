//! Packed affine transform [`Affine3A`] — a linear 3×3 (carrying non-uniform
//! scale / shear) plus a translation.
//!
//! NEW code (not on the physics determinism path) but it still obeys the no-FMA
//! / exact-sqrt discipline. `matrix3` reuses the **row-major** [`Mat3`] ops
//! verbatim, so the affine path adds no new linear-algebra code and no new
//! transpose-bug surface. The row-major ↔ column-major boundary is crossed in
//! EXACTLY ONE place: [`Affine3A::to_mat4`] / [`Mat4::from_affine`].

use crate::mat::{Mat3, Mat4};
use crate::quat::Quat;
use crate::vec::{Vec3, Vec4};

/// A packed affine transform: a row-major linear part (`matrix3`) plus a
/// `translation`.
///
/// `#[repr(C, align(16))]`. Payload is `Mat3` (3 × `Vec3` = 36 B) + `Vec3`
/// (12 B) = 48 B, rounded to a 16-aligned 48 B (already a multiple of 16). It is
/// the cached world pose behind a `GlobalTransform`. `transform_point` /
/// `transform_vector` / `mul` all reuse the row-major [`Mat3`] ops, so there is
/// no convention split on the affine path.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine3A {
    /// The linear part: rotation, scale, and shear, **row-major** (see [`Mat3`]).
    pub matrix3: Mat3,
    /// The translation.
    pub translation: Vec3,
}

impl Affine3A {
    /// The identity transform (identity linear part, zero translation).
    pub const IDENTITY: Self = Self {
        matrix3: Mat3::IDENTITY,
        translation: Vec3::ZERO,
    };

    /// Builds an affine from a translation, rotation, and per-axis scale
    /// (`T · R · S` applied to a point: scale, then rotate, then translate).
    ///
    /// The linear part is `R · diag(scale)` via
    /// [`Mat3::from_quat_scale`](Mat3::from_quat_scale) (row-major). No FMA.
    #[inline]
    pub fn from_translation_rotation_scale(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self {
            matrix3: Mat3::from_quat_scale(rotation, scale),
            translation,
        }
    }

    /// The RIGID camera **world** transform looking from `eye` at `target` with
    /// world `up` hint — NOT the view matrix (that is `self.inverse()`).
    ///
    /// Right-handed, in the basis convention `ViewUniform::from_camera` consumes:
    /// local `right = +X`, `up = +Y`, `forward = -Z`, stored as the COLUMNS of
    /// `matrix3`. Column 2 (camera `+Z`) is `normalize(eye - target)` (so the
    /// camera's `-Z` points AT `target`); column 0 is `normalize(cross(up, +Z))`;
    /// column 1 is `cross(+Z, right)`. The ordered triple `(right, true_up, back)`
    /// is right-handed (`cross(right, true_up) ≈ back`, `det(matrix3) ≈ +1`).
    ///
    /// Degenerate guards (never `NaN`):
    /// - `eye == target` (zero `back`) → substitute `back = +Z` (a valid default).
    /// - `up ∥ back` (the pole, zero `right`) → swap ONLY the source `up` to a
    ///   fallback axis orthogonal to `back` (the world axis least aligned with
    ///   `back`), then re-derive `right = cross(fallback_up, back)`. The cross
    ///   ORDER is never reordered, so the chirality (and thus `det ≈ +1`) is
    ///   identical to the nominal case.
    #[inline]
    pub fn look_at_rh(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        // Threshold below which a length-squared counts as degenerate. Matches
        // the zero-length guard scale used by `Vec3::normalize`.
        const EPS_SQ: f32 = 1.0e-12;

        // Column 2 (camera +Z) = direction FROM target TO eye, so -Z points AT
        // target. Guard the eye==target case with a valid default +Z.
        let back_raw = eye - target;
        let back = if back_raw.length_squared() < EPS_SQ {
            Vec3::new(0.0, 0.0, 1.0)
        } else {
            back_raw.normalize()
        };

        // Column 0 (camera +X) = cross(up, back). If up ∥ back the right axis is
        // degenerate (the pole): swap ONLY the source up to a fallback axis that
        // is least aligned with `back`, then reuse the SAME cross order so the
        // basis chirality is preserved.
        let right_raw = up.cross(back);
        let right = if right_raw.length_squared() < EPS_SQ {
            // Pick the world axis least aligned with `back` so the cross is
            // well-conditioned: if `back` is closest to ±X use +Y, else use +X.
            let fallback_up = if back.x.abs() <= back.y.abs() && back.x.abs() <= back.z.abs() {
                Vec3::new(1.0, 0.0, 0.0)
            } else {
                Vec3::new(0.0, 1.0, 0.0)
            };
            fallback_up.cross(back).normalize()
        } else {
            right_raw.normalize()
        };

        // Column 1 (camera +Y). `right` and `back` are unit and orthogonal, so
        // this is already unit; normalize defensively against accumulated drift.
        let true_up = back.cross(right).normalize();

        Self {
            matrix3: Mat3::from_columns(right, true_up, back),
            translation: eye,
        }
    }

    /// Composes `self ∘ rhs` (apply `rhs` first, then `self`) — the
    /// parent-from-child composition used by transform propagation.
    ///
    /// `(self ∘ rhs)(p) = self.matrix3 · (rhs.matrix3 · p + rhs.t) + self.t`,
    /// so the linear part is the row-major [`Mat3`] product and the translation
    /// is `self.matrix3 · rhs.t + self.t`. Reuses the lifted row-major ops; no
    /// new linear algebra, no FMA.
    // `clippy::should_implement_trait`: affine composition is `∘` (parent ∘
    // child), NOT a commutative `Mul`; it is intentionally a named method per the
    // plan's `Affine3A::mul(self, Affine3A) -> Affine3A` surface (no `Mul`
    // operator, matching the named-method convention of `Quat::mul`).
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn mul(self, rhs: Self) -> Self {
        Self {
            matrix3: self.matrix3 * rhs.matrix3,
            translation: self.matrix3.mul_vec(rhs.translation) + self.translation,
        }
    }

    /// Transforms a point: `matrix3 · p + translation`.
    #[inline]
    pub fn transform_point(self, p: Vec3) -> Vec3 {
        self.matrix3.mul_vec(p) + self.translation
    }

    /// Transforms a direction (ignores translation): `matrix3 · v`.
    #[inline]
    pub fn transform_vector(self, v: Vec3) -> Vec3 {
        self.matrix3.mul_vec(v)
    }

    /// The GENERAL affine inverse, or `None` when the linear part is singular.
    ///
    /// Inverts the row-major `matrix3` via the general 3×3 inverse (handles
    /// non-uniform scale / shear), then `inv_t = -(matrix3⁻¹ · translation)`.
    /// For a rigid (orthonormal) camera transform the cheaper transpose form is
    /// preferred at the call site, but this general method exists for
    /// correctness on scaled/sheared transforms. No FMA.
    #[inline]
    pub fn inverse(self) -> Option<Self> {
        let inv_linear = self.matrix3.inverse()?;
        let inv_t = inv_linear.mul_vec(self.translation) * -1.0;
        Some(Self {
            matrix3: inv_linear,
            translation: inv_t,
        })
    }

    /// Converts this row-major affine into a **column-major** [`Mat4`].
    ///
    /// **This is the single row-major ↔ column-major convention boundary.** The
    /// row-major 3×3 is transposed into the upper-left of a column-major 4×4:
    /// column `j` of the `Mat4` is built from element `[*][j]` of the row-major
    /// `matrix3` (i.e. `cols[j] = (rows[0][j], rows[1][j], rows[2][j], 0)`), and
    /// the translation becomes the last column. No FMA (pure data shuffle).
    #[inline]
    pub fn to_mat4(self) -> Mat4 {
        let m = &self.matrix3.rows;
        Mat4::from_cols(
            Vec4::new(m[0].x, m[1].x, m[2].x, 0.0),
            Vec4::new(m[0].y, m[1].y, m[2].y, 0.0),
            Vec4::new(m[0].z, m[1].z, m[2].z, 0.0),
            Vec4::new(
                self.translation.x,
                self.translation.y,
                self.translation.z,
                1.0,
            ),
        )
    }
}

impl Default for Affine3A {
    /// The default transform is [`Affine3A::IDENTITY`].
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat4 {
    /// Builds a column-major [`Mat4`] from a row-major [`Affine3A`].
    ///
    /// The inverse of [`Affine3A::to_mat4`] direction — the single convention
    /// boundary. Delegates to [`Affine3A::to_mat4`] so the transpose-and-embed
    /// lives in exactly one place.
    #[inline]
    pub fn from_affine(affine: Affine3A) -> Self {
        affine.to_mat4()
    }
}
