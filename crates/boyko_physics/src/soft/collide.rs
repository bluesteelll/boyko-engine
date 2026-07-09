//! One-sided SDF collision for soft-body particles (Physics O11 SP1, plan D4).
//!
//! Each particle is a sphere of [`SoftBody::particle_radius`](crate::soft::SoftBody)
//! pushed OUT of the analytic [`SdfField`] surface. The query reuses the O9
//! [`sample_sdf`] evaluator (the SAME edit list the GPU renders, exact arithmetic,
//! zero readback), so the soft-body collision and the rigid SDF narrowphase fold
//! bit-identical field math.
//!
//! Sign convention (matches the rigid SDF path): `sample_sdf` returns a signed
//! distance (negative INSIDE a solid) and the OUTWARD unit field gradient. A
//! particle whose surface gap (`dist - radius`) is negative is penetrating and is
//! displaced along the outward normal by exactly the penetration depth. A
//! zero-gradient critical point yields `Vec3::ZERO`; the push is then skipped (a
//! deterministic no-op), never a `NaN`.

use crate::math::Vec3;
use crate::sdf_query::{SdfField, sample_sdf};

/// Resolves one particle against the SDF, returning the corrected position.
///
/// Returns `pos` unchanged when the particle is non-penetrating (`gap >= 0`) or
/// when the field gradient is degenerate (zero — a CSG-seam critical point with no
/// usable normal). Otherwise it pushes the particle center OUT along the outward
/// surface normal by the penetration depth `|gap|`, so the particle just touches
/// the surface.
///
/// One-sided: the field only ever PUSHES OUT (it never pulls a non-penetrating
/// particle to the surface), matching the rigid SDF narrowphase. Pinned particles
/// are skipped by the caller (this is called only for movable particles), so this
/// function needs no inverse-mass argument.
///
/// Determinism: exact `sqrt`/`divide` only — `sample_sdf` is the shared
/// rsqrt-free leaf, and the push is `pos - normal * gap` (no FMA).
#[inline]
pub fn collide_sdf(field: &SdfField, pos: Vec3, radius: f32) -> Vec3 {
    let (dist, normal) = sample_sdf(field, pos);
    let gap = dist - radius;
    if gap >= 0.0 {
        // Non-penetrating: one-sided field never pulls in.
        return pos;
    }
    if normal == Vec3::ZERO {
        // Zero-gradient critical point — no usable normal, deterministic no-push.
        return pos;
    }
    // `gap < 0` ⇒ push the center OUT by `normal * |gap|` (i.e. `- normal * gap`).
    pos - normal * gap
}
