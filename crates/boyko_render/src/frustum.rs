//! VG rung R2c: camera-frustum planes and the conservative AABB rejection test.
//!
//! This module is the HOST half of the per-batch draw cull. The GPU half
//! (`vb_batch_cull.comp.hlsl`) does not re-derive anything: the six planes computed here are
//! PUSHED to it, so the shader and this oracle evaluate the identical numbers and a disagreement
//! between them is a shader bug rather than a math bug. That is the whole reason the extraction
//! lives on the host.
//!
//! # The convention, stated because getting it wrong is silent
//!
//! [`frustum_planes_from_view_proj`] takes the matrix in the SAME form
//! [`gbuffer_push_from_view_jittered`](crate::view::gbuffer_push_from_view_jittered) builds it —
//! `pv[row][col]`, math rows, `clip = pv · world`. (That function then serialises it COLUMN-major
//! into the push, which is HLSL's `float4x4` storage; the serialisation is a transposition, not a
//! different matrix, and it is not this module's concern.)
//!
//! Vulkan clip space is `-w ≤ x,y ≤ w` and `0 ≤ z ≤ w`. The six half-space inequalities are
//! therefore linear combinations of the matrix rows, and they hold **regardless of REVERSE-Z**:
//! this engine renders reverse-Z (`GREATER`, clear `0.0`), which swaps which physical plane maps
//! to `z = 0` versus `z = w`, but `0 ≤ z ≤ w` is true either way. The extraction needs no
//! reverse-Z special case, and adding one would be the bug.
//!
//! Planes are returned UNNORMALISED. Normalising costs a `sqrt` per plane and buys nothing here —
//! the rejection test compares a signed distance against zero, and scaling a plane by a positive
//! constant cannot change that sign. It would also introduce a division that a degenerate
//! (zero-normal) row turns into a NaN, and a NaN comparison in the test below reads as "not
//! outside", i.e. it would silently disarm the cull rather than fail loudly.

use crate::instance_model::InstanceModelCol;
use crate::mesh_draw::DrawBatch;

/// One frustum plane as `(a, b, c, d)` with the convention **inside ⇒ `a·x + b·y + c·z + d ≥ 0`**.
pub type Plane = [f32; 4];

/// The number of frustum planes: left, right, bottom, top, near, far — in that fixed order, which
/// `vb_batch_cull.comp.hlsl` mirrors.
pub const FRUSTUM_PLANE_COUNT: usize = 6;

/// Extracts the six clip-space frustum planes from a `clip = pv · world` matrix in math-row form.
///
/// Gribb–Hartmann: each clip inequality is a row combination.
///
/// | plane | inequality | row combination |
/// |---|---|---|
/// | left | `clip.x ≥ -clip.w` | `pv[3] + pv[0]` |
/// | right | `clip.x ≤ clip.w` | `pv[3] − pv[0]` |
/// | bottom | `clip.y ≥ -clip.w` | `pv[3] + pv[1]` |
/// | top | `clip.y ≤ clip.w` | `pv[3] − pv[1]` |
/// | near | `clip.z ≥ 0` | `pv[2]` |
/// | far | `clip.z ≤ clip.w` | `pv[3] − pv[2]` |
///
/// The last two are the Vulkan `[0, w]` depth range, NOT OpenGL's `[-w, w]`; using the OpenGL form
/// here would produce a near plane of `pv[3] + pv[2]`, which on a reverse-Z projection rejects
/// geometry in front of the camera.
#[must_use]
pub fn frustum_planes_from_view_proj(pv: &[[f32; 4]; 4]) -> [Plane; FRUSTUM_PLANE_COUNT] {
    let add = |a: [f32; 4], b: [f32; 4]| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
    let sub = |a: [f32; 4], b: [f32; 4]| [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
    [
        add(pv[3], pv[0]),
        sub(pv[3], pv[0]),
        add(pv[3], pv[1]),
        sub(pv[3], pv[1]),
        pv[2],
        sub(pv[3], pv[2]),
    ]
}

/// Extracts the six planes from the FIRST 64 BYTES OF THE RASTER PUSH — the same bytes the vertex
/// shader reads as its `view_proj`.
///
/// # Why decode the bytes instead of re-using the matrix
///
/// The cull must reject against the matrix the raster actually draws with, and the push is where
/// that matrix physically is. Taking the planes from a separately-computed `pv` would be correct
/// only for as long as nobody perturbs one without the other — and something already does:
/// [`gbuffer_push_from_view_jittered`](crate::view::gbuffer_push_from_view_jittered) offsets the
/// projection per frame for TAA. Byte provenance makes that question not arise.
///
/// The push stores the matrix COLUMN-major (HLSL's `float4x4` storage), i.e. element index
/// `col * 4 + row`, which this function inverts back to the math-row form
/// [`frustum_planes_from_view_proj`] documents.
#[must_use]
pub fn frustum_planes_from_push_bytes(bytes: &[u8; 64]) -> [Plane; FRUSTUM_PLANE_COUNT] {
    let mut pv = [[0.0f32; 4]; 4];
    for e in 0..16 {
        let mut w = [0u8; 4];
        w.copy_from_slice(&bytes[e * 4..e * 4 + 4]);
        // The inverse of the push's `out[(col * 4 + row) * 4]` serialisation.
        pv[e % 4][e / 4] = f32::from_le_bytes(w);
    }
    frustum_planes_from_view_proj(&pv)
}

/// The conservative rejection test: `true` iff the AABB `[min, max]` is **wholly in the negative
/// half-space of at least one plane**, and therefore certainly invisible.
///
/// # Why the error direction is one-way, by construction
///
/// For each plane, `n·c + d` is the centre's signed distance and `r = |a|·hx + |b|·hy + |c|·hz` is
/// the box's extent projected onto the plane normal, so `n·c + d + r` is the signed distance of the
/// box's FARTHEST corner along the normal. If that is still negative, every one of the eight
/// corners is outside — no approximation, an exact statement about the box.
///
/// The converse does NOT hold: a box straddling two planes' outsides without being wholly outside
/// either is reported visible. That is the intended direction. **A false "cull" would delete
/// geometry from the frame; a false "keep" costs one wasted draw**, so the test is deliberately
/// biased toward keeping. Callers must not "tighten" this into an exact frustum-box intersection
/// without re-deriving that guarantee.
///
/// A NaN in the box or the planes makes every comparison false, so the box is reported VISIBLE —
/// the same safe direction. See this module's header for why the planes are left unnormalised.
#[must_use]
pub fn aabb_outside_frustum(planes: &[Plane; FRUSTUM_PLANE_COUNT], min: [f32; 3], max: [f32; 3]) -> bool {
    let c = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, (min[2] + max[2]) * 0.5];
    let h = [(max[0] - min[0]) * 0.5, (max[1] - min[1]) * 0.5, (max[2] - min[2]) * 0.5];
    for p in planes {
        let dist = p[0] * c[0] + p[1] * c[1] + p[2] * c[2] + p[3];
        let radius = p[0].abs() * h[0] + p[1].abs() * h[1] + p[2].abs() * h[2];
        if dist + radius < 0.0 {
            return true;
        }
    }
    false
}

/// The rung-R2c HOST ORACLE: the batch's `instanceCount` after the cull — `0` when the batch's
/// world AABB is wholly outside the frustum, its full instance count otherwise.
///
/// `vb_batch_cull.comp.hlsl` computes exactly this from the same pushed planes and the same
/// transfer-filled AABB, so this function is what a drawn-set equality test compares against.
/// A batch whose bounds are unavailable (`None` — mesh not `Loaded`, or the C0 zero-vertex
/// sentinel) is KEPT: absence of bounds is not evidence of invisibility.
#[must_use]
pub fn batch_instance_count_after_cull(
    planes: &[Plane; FRUSTUM_PLANE_COUNT],
    batch: &DrawBatch,
    ring: &[InstanceModelCol],
    mesh_aabb: Option<([f32; 3], [f32; 3])>,
) -> u32 {
    let Some(aabb) = mesh_aabb.and_then(|a| crate::csm_caster::batch_world_aabb(batch, ring, a))
    else {
        return batch.instance_count;
    };
    if aabb_outside_frustum(planes, aabb.0, aabb.1) { 0 } else { batch.instance_count }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain Vulkan-style perspective `proj · view` with the camera at the origin looking down
    /// `-z`, in the math-row form this module documents. Built by hand rather than through
    /// `ViewUniform` so the test pins the CONVENTION, not another function's output.
    fn perspective_at_origin() -> [[f32; 4]; 4] {
        // f = 1/tan(fovY/2) with fovY = 90°, aspect 1, near 0.1, far 100 (standard, NOT reverse-Z:
        // the extraction must not depend on which depth convention is in use, and using the plain
        // one here means a reverse-Z-specific bug cannot hide behind a reverse-Z fixture).
        let (f, n, fa) = (1.0f32, 0.1f32, 100.0f32);
        [
            [f, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, fa / (n - fa), (fa * n) / (n - fa)],
            [0.0, 0.0, -1.0, 0.0],
        ]
    }

    /// Brute-force oracle: is EVERY corner of the box outside the same plane? This is the
    /// definition [`aabb_outside_frustum`] claims to compute in closed form.
    fn every_corner_outside_some_plane(
        planes: &[Plane; FRUSTUM_PLANE_COUNT],
        min: [f32; 3],
        max: [f32; 3],
    ) -> bool {
        planes.iter().any(|p| {
            (0..8).all(|i| {
                let x = if i & 1 == 0 { min[0] } else { max[0] };
                let y = if i & 2 == 0 { min[1] } else { max[1] };
                let z = if i & 4 == 0 { min[2] } else { max[2] };
                p[0] * x + p[1] * y + p[2] * z + p[3] < 0.0
            })
        })
    }

    /// The closed form must agree with the 8-corner definition. Swept over a grid that straddles
    /// every plane, so the agreement is not a coincidence of one placement.
    #[test]
    fn closed_form_matches_the_eight_corner_definition() {
        let planes = frustum_planes_from_view_proj(&perspective_at_origin());
        let mut checked = 0usize;
        for xi in -4..=4 {
            for yi in -4..=4 {
                for zi in -6..=2 {
                    let c = [xi as f32 * 3.0, yi as f32 * 3.0, zi as f32 * 4.0];
                    for &hh in &[0.25f32, 1.0, 4.0] {
                        let min = [c[0] - hh, c[1] - hh, c[2] - hh];
                        let max = [c[0] + hh, c[1] + hh, c[2] + hh];
                        assert_eq!(
                            aabb_outside_frustum(&planes, min, max),
                            every_corner_outside_some_plane(&planes, min, max),
                            "closed form disagrees with the corner definition at centre {c:?} half {hh}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 2000, "sweep degenerated to {checked} cases");
    }

    /// SENSITIVITY: the sweep above is worthless if it never sees both answers. Pin that both
    /// occur, so "they agree" is not "they are both always false".
    #[test]
    fn the_sweep_covers_both_verdicts() {
        let planes = frustum_planes_from_view_proj(&perspective_at_origin());
        // Straight ahead, inside the 90° cone.
        assert!(!aabb_outside_frustum(&planes, [-0.5, -0.5, -5.5], [0.5, 0.5, -4.5]));
        // Far off to the left, well outside the left plane.
        assert!(aabb_outside_frustum(&planes, [99.0, -0.5, -5.5], [100.0, 0.5, -4.5]));
        // BEHIND the camera — the case an OpenGL-style near plane (`pv[3] + pv[2]`) would get
        // wrong on this matrix.
        assert!(aabb_outside_frustum(&planes, [-0.5, -0.5, 4.5], [0.5, 0.5, 5.5]));
    }

    /// The one-way guarantee, stated as a test: a box CONTAINING a point inside the frustum is
    /// never rejected. This is the property that makes the cull safe to arm.
    #[test]
    fn a_box_touching_the_frustum_is_never_culled() {
        let planes = frustum_planes_from_view_proj(&perspective_at_origin());
        // A huge box centred far outside, but stretched so it swallows the view volume.
        assert!(!aabb_outside_frustum(&planes, [-500.0, -500.0, -500.0], [500.0, 500.0, 500.0]));
        // A sliver that pokes in from the left, its centre outside the left plane.
        assert!(!aabb_outside_frustum(&planes, [-40.0, -0.1, -10.1], [-0.1, 0.1, -9.9]));
    }

    /// An unbounded sentinel box (the rung-R2c0 `VbBatchDesc::UNBOUNDED` corners) must survive
    /// every plane — that is what makes an unfilled descriptor conservative rather than fatal.
    #[test]
    fn the_unbounded_sentinel_is_never_culled() {
        let planes = frustum_planes_from_view_proj(&perspective_at_origin());
        let u = 1.0e30f32;
        assert!(!aabb_outside_frustum(&planes, [-u, -u, -u], [u, u, u]));
    }

    /// The byte decode must invert the push's serialisation EXACTLY. Serialised here with the
    /// literal loop `gbuffer_push_from_view_jittered` uses, so a transposed decode — the one bug
    /// this function can have, and one that produces a plausible-looking wrong frustum rather than
    /// a crash — fails here instead of silently culling the wrong half of the scene.
    #[test]
    fn the_push_decode_inverts_the_column_major_serialisation() {
        let pv = perspective_at_origin();
        let mut bytes = [0u8; 64];
        for col in 0..4 {
            for row in 0..4 {
                let b = pv[row][col].to_le_bytes();
                bytes[(col * 4 + row) * 4..(col * 4 + row) * 4 + 4].copy_from_slice(&b);
            }
        }
        assert_eq!(
            frustum_planes_from_push_bytes(&bytes),
            frustum_planes_from_view_proj(&pv),
            "the push decode is not the inverse of the push serialisation"
        );

        // SENSITIVITY: a TRANSPOSED decode must NOT agree, or the assertion above is satisfied by
        // a symmetric fixture rather than by a correct decode.
        let mut transposed = [[0.0f32; 4]; 4];
        for r in 0..4 {
            for c in 0..4 {
                transposed[r][c] = pv[c][r];
            }
        }
        assert_ne!(
            frustum_planes_from_view_proj(&transposed),
            frustum_planes_from_view_proj(&pv),
            "the fixture matrix is symmetric, so it cannot detect a transposed decode — pick another"
        );
    }

    /// THE ORACLE, END TO END: a batch behind the camera is culled to `0`, one in front keeps its
    /// full instance count.
    ///
    /// This is the assertion the goldens CANNOT make. Every pinned scene is entirely on-screen, so
    /// a cull that rejects nothing is byte-identical to a correct one — "9 pins unchanged" is
    /// evidence the cull breaks nothing, and no evidence at all that it ever culls. This test is
    /// the other half, and it runs the whole host path: mesh-local AABB → `batch_world_aabb`'s
    /// Arvo fold through the instance ring → the plane test.
    #[test]
    fn the_oracle_culls_a_batch_behind_the_camera_and_keeps_one_in_front() {
        let planes = frustum_planes_from_view_proj(&perspective_at_origin());
        // A unit cube at the origin in model space.
        let local = ([-0.5f32, -0.5, -0.5], [0.5f32, 0.5, 0.5]);
        // Two instances: translation only, identity linear part.
        let at = |z: f32| InstanceModelCol {
            rows: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, z]],
        };
        // ring[0] sits in front of the camera (-z), ring[1] behind it (+z).
        let ring = [at(-6.0), at(6.0)];
        let batch = |base: u32| DrawBatch {
            mesh_id: 0,
            index_count: 36,
            index_type: boyko_rhi::IndexType::Uint16,
            base_instance: base,
            instance_count: 1,
        };

        assert_eq!(
            batch_instance_count_after_cull(&planes, &batch(0), &ring, Some(local)),
            1,
            "a batch in front of the camera must keep every instance"
        );
        assert_eq!(
            batch_instance_count_after_cull(&planes, &batch(1), &ring, Some(local)),
            0,
            "a batch wholly behind the camera must cull to zero — if this is 1 the cull is armed \
             but inert, which every golden would happily accept"
        );
        // Bounds unavailable ⇒ KEEP. Absence of bounds is not evidence of invisibility, and this
        // is the path a not-yet-`Loaded` mesh takes every frame it is streaming in.
        assert_eq!(
            batch_instance_count_after_cull(&planes, &batch(1), &ring, None),
            1,
            "a batch with no bounds must be KEPT — the streaming path would otherwise pop"
        );
    }

    /// A NaN must read as VISIBLE, not as culled. `NaN < 0.0` is false, so the early-out never
    /// fires — asserted rather than assumed, because the opposite would delete geometry.
    #[test]
    fn a_nan_box_is_kept_not_culled() {
        let planes = frustum_planes_from_view_proj(&perspective_at_origin());
        let n = f32::NAN;
        assert!(!aabb_outside_frustum(&planes, [n, n, n], [n, n, n]));
        assert!(!aabb_outside_frustum(&planes, [99.0, n, -5.5], [100.0, n, -4.5]));
    }
}
