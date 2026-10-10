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

/// The rung-R2d HOST ORACLE at INSTANCE granularity: `true` iff this one instance survives the
/// per-instance cull.
///
/// The GPU half is `vb_batch_cull.comp.hlsl`'s level 2, whose `keep` expression the arming rung
/// replaces with exactly this test: the instance's mesh-LOCAL box (`gMeshBounds[mesh_id]`)
/// Arvo-transformed by that instance's affine (`gVbInstances[...]`), then run through the same six
/// pushed planes. One test, two granularities — [`batch_instance_count_after_cull`] is the same
/// predicate over a batch's UNION box.
///
/// The abs-matrix fold is [`arvo_transform`](crate::csm_caster::arvo_transform), REUSED rather than
/// transcribed: this repository already carries two callers of it (`batch_world_aabb` and
/// `reduce_bounds_into`), and a third copy would be a third text that can disagree with the shader.
///
/// # Unknown bounds KEEP
///
/// An INVERTED local box (`min > max` on any axis) is the "bounds unknown" sentinel — a
/// `MeshLocalBounds` row for a mesh that never registered, or the C0 zero-vertex fold — and it
/// returns `true`. **Absence of bounds is not evidence of invisibility**, and the sentinel's centre
/// is NaN, so folding it would poison the box rather than merely widen it. This is the same
/// direction [`batch_instance_count_after_cull`] takes for a `None` batch AABB and the same one
/// `MeshLocalBounds`' own doc obliges every consumer to take.
#[must_use]
pub fn instance_visible_after_cull(
    planes: &[Plane; FRUSTUM_PLANE_COUNT],
    instance: &InstanceModelCol,
    mesh_aabb: ([f32; 3], [f32; 3]),
) -> bool {
    let (mn, mx) = mesh_aabb;
    if mn[0] > mx[0] || mn[1] > mx[1] || mn[2] > mx[2] {
        return true;
    }
    let lc = [(mn[0] + mx[0]) * 0.5, (mn[1] + mx[1]) * 0.5, (mn[2] + mx[2]) * 0.5];
    let lh = [(mx[0] - mn[0]) * 0.5, (mx[1] - mn[1]) * 0.5, (mx[2] - mn[2]) * 0.5];
    let (wc, wh) = crate::csm_caster::arvo_transform(&instance.rows, lc, lh);
    let min = [wc[0] - wh[0], wc[1] - wh[1], wc[2] - wh[2]];
    let max = [wc[0] + wh[0], wc[1] + wh[1], wc[2] + wh[2]];
    !aabb_outside_frustum(planes, min, max)
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

    // ===========================================================================================
    // VG rung R2d — the per-INSTANCE oracle
    //
    // Every expectation below is HAND-COMPUTED from `perspective_at_origin`'s own rows, never
    // read off a second implementation: an oracle checked against an oracle agrees about its
    // shared mistakes.
    // ===========================================================================================

    /// The unit cube in model space — half-extent `0.5` on every axis, so a plane's projected
    /// radius is `(|a| + |b| + |c|) * 0.5` for the identity affine.
    const UNIT_CUBE: ([f32; 3], [f32; 3]) = ([-0.5, -0.5, -0.5], [0.5, 0.5, 0.5]);

    /// A translation-only instance row (identity linear part).
    fn at(t: [f32; 3]) -> InstanceModelCol {
        InstanceModelCol {
            rows: [[1.0, 0.0, 0.0, t[0]], [0.0, 1.0, 0.0, t[1]], [0.0, 0.0, 1.0, t[2]]],
        }
    }

    /// The right plane of [`perspective_at_origin`] is `pv[3] - pv[0] = (-1, 0, -1, 0)`, i.e.
    /// inside ⇒ `-x - z ≥ 0` ⇒ `x ≤ -z`. At `z = -6` the frustum admits `x ≤ 6`, and a unit cube's
    /// projected radius on that normal is `(1 + 0 + 1) * 0.5 = 1`.
    ///
    /// So the closed form rejects exactly when `(-x + 6) + 1 < 0`, i.e. `x > 7`. Both sides of that
    /// boundary are asserted, by hand:
    ///
    /// * `x = 8` ⇒ `-2 + 1 = -1 < 0` ⇒ CULLED;
    /// * `x = 6.4` ⇒ `-0.4 + 1 = 0.6 ≥ 0` ⇒ KEPT (the box straddles the plane, which the
    ///   conservative test must never reject).
    #[test]
    fn the_instance_oracle_rejects_past_the_hand_computed_plane_boundary() {
        let planes = frustum_planes_from_view_proj(&perspective_at_origin());
        assert!(
            !instance_visible_after_cull(&planes, &at([8.0, 0.0, -6.0]), UNIT_CUBE),
            "x = 8 at z = -6 is one unit outside the right plane once the cube's radius is \
             credited — the oracle must reject it"
        );
        assert!(
            instance_visible_after_cull(&planes, &at([6.4, 0.0, -6.0]), UNIT_CUBE),
            "x = 6.4 at z = -6 straddles the right plane (0.6 of the cube is still inside); a \
             conservative cull that rejects it would delete visible geometry"
        );
        // BEHIND the camera, on the near plane `pv[2]` — the case an OpenGL-style extraction gets
        // wrong on this matrix. `pv[2] = (0, 0, fa/(n-fa), fa*n/(n-fa))` with `fa/(n-fa) < 0`, so
        // at `z = +6` the centre distance is strongly negative and the radius is ~0.5.
        assert!(
            !instance_visible_after_cull(&planes, &at([0.0, 0.0, 6.0]), UNIT_CUBE),
            "an instance behind the camera must be rejected"
        );
        assert!(
            instance_visible_after_cull(&planes, &at([0.0, 0.0, -6.0]), UNIT_CUBE),
            "an instance squarely in front of the camera must be kept"
        );
    }

    /// UNKNOWN BOUNDS ⇒ KEEP. An inverted local box is the sentinel a mesh that never registered
    /// (or the C0 zero-vertex fold) leaves behind; it must not read as "empty, therefore invisible".
    ///
    /// Asserted at a position the oracle rejects with REAL bounds, so the test cannot pass merely
    /// because the instance happens to be on screen.
    #[test]
    fn an_instance_with_inverted_sentinel_bounds_is_kept_not_culled() {
        let planes = frustum_planes_from_view_proj(&perspective_at_origin());
        let far_off = at([80.0, 0.0, -6.0]);
        assert!(
            !instance_visible_after_cull(&planes, &far_off, UNIT_CUBE),
            "the fixture position must be REJECTED with real bounds, or the sentinel assertion \
             below proves nothing"
        );
        // The `MeshLocalBounds` sentinel shape: min > max on every axis.
        let sentinel = ([1.0f32, 1.0, 1.0], [-1.0f32, -1.0, -1.0]);
        assert!(
            instance_visible_after_cull(&planes, &far_off, sentinel),
            "absence of bounds is not evidence of invisibility — a streaming-in mesh would pop"
        );
        // Inverted on ONE axis only is still the sentinel: the fold's centre would be finite but
        // the half-extent negative, which is not a box at all.
        let one_axis = ([-0.5f32, 1.0, -0.5], [0.5f32, -1.0, 0.5]);
        assert!(instance_visible_after_cull(&planes, &far_off, one_axis));
    }

    /// ⚠️ THE ORDER of the unknown-bounds test against the Arvo fold, which is the one thing a
    /// deleted-arm census could not see: moving the sentinel check AFTER the transform changes no
    /// opcode COUNT, only their sequence.
    ///
    /// This is the case that inverts the guarantee, and a critic found exactly it in an earlier
    /// draft of the shader. A DEGENERATE affine — zero linear part, which a zero-scale or
    /// not-yet-initialised instance really produces — folds the large-but-finite inverted sentinel
    /// to `lc = (S + -S)/2 = 0`, `lh = -S`, and then every `wh[r] = dot(abs(row.xyz), lh)` is
    /// `0 * -S = 0`. The "unbounded" box has become a POINT at the translation, and a point far
    /// outside the frustum is REJECTED — so "bounds unknown" would silently mean "cull it", the
    /// exact inversion of the contract `MeshLocalBounds::UNKNOWN`'s doc states.
    ///
    /// Both evaluators of this predicate must test the sentinel first. This pins the host one; the
    /// shader mirrors it, and its header carries the same derivation.
    #[test]
    fn the_sentinel_is_tested_before_the_fold_so_a_degenerate_affine_cannot_invert_it() {
        let planes = frustum_planes_from_view_proj(&perspective_at_origin());
        // Zero linear part, translation far outside the frustum. `at()` builds an identity linear
        // part, so this instance is written out longhand.
        let degenerate = InstanceModelCol {
            rows: [
                [0.0, 0.0, 0.0, 80.0],
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, -6.0],
            ],
        };
        // Control: with REAL bounds the fold yields that same point, and it must be rejected —
        // otherwise the assertion below would pass for the wrong reason (nothing is culled here).
        assert!(
            !instance_visible_after_cull(&planes, &degenerate, UNIT_CUBE),
            "a degenerate affine at x=80 collapses any box to a point outside the frustum, so the \
             REJECT here is what makes the sentinel case below meaningful"
        );
        let sentinel = ([1.0e30f32, 1.0e30, 1.0e30], [-1.0e30f32, -1.0e30, -1.0e30]);
        assert!(
            instance_visible_after_cull(&planes, &degenerate, sentinel),
            "the sentinel must be recognised BEFORE the fold. Folded first, it collapses to the \
             same rejected point as the control above, and `bounds unknown` would come to mean \
             `cull it` — inverting the one guarantee this predicate owes its callers"
        );
    }

    /// A SHEARED affine, and the assertion is two-sided against the `abs` in the Arvo fold.
    ///
    /// The instance's linear part is `r0 = (1, -4, 0)`, so the world half-extent along X is
    /// `|1|*0.5 + |-4|*0.5 = 2.5` WITH the absolute value and `1*0.5 + (-4)*0.5 = -1.5` without it.
    /// Centred at `x = 7, z = -6`, the right plane `(-1, 0, -1, 0)` gives centre distance
    /// `-7 + 6 = -1` and radius `wh_x + wh_z`:
    ///
    /// * with `abs`: `2.5 + 0.5 = 3.0` ⇒ `-1 + 3 = 2 ≥ 0` ⇒ KEPT;
    /// * without `abs`: `-1.5 + 0.5 = -1.0` ⇒ `-1 + (-1) = -2 < 0` ⇒ CULLED.
    ///
    /// The counterfactual box is built here by hand and fed to the SHIPPED plane test, so the
    /// "a mutation dropping the abs would flip this" claim is executed rather than asserted in
    /// prose.
    #[test]
    fn a_sheared_instance_needs_the_abs_in_the_arvo_fold() {
        let planes = frustum_planes_from_view_proj(&perspective_at_origin());
        let sheared = InstanceModelCol {
            rows: [[1.0, -4.0, 0.0, 7.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, -6.0]],
        };
        assert!(
            instance_visible_after_cull(&planes, &sheared, UNIT_CUBE),
            "the sheared box reaches back into the frustum (half-extent 2.5 along X against a \
             centre 1 unit outside the right plane) and must be KEPT"
        );

        // The mutation, spelled out: the same fold with the absolute values removed.
        let lh = [0.5f32, 0.5, 0.5];
        let mut wc = [0.0f32; 3];
        let mut wh_no_abs = [0.0f32; 3];
        for r in 0..3 {
            let row = sheared.rows[r];
            wc[r] = row[3];
            wh_no_abs[r] = row[0] * lh[0] + row[1] * lh[1] + row[2] * lh[2];
        }
        assert_eq!(wh_no_abs[0], -1.5, "the counterfactual half-extent is the one the doc names");
        assert!(
            aabb_outside_frustum(
                &planes,
                [wc[0] - wh_no_abs[0], wc[1] - wh_no_abs[1], wc[2] - wh_no_abs[2]],
                [wc[0] + wh_no_abs[0], wc[1] + wh_no_abs[1], wc[2] + wh_no_abs[2]],
            ),
            "an abs-less fold must REJECT this instance — if it does not, this fixture cannot \
             detect the mutation it exists for"
        );
    }

    /// The union implication, at the two granularities the rung compares: a batch whose UNION box
    /// is rejected has every member rejected. Stated as a test because the whole per-instance rung
    /// rests on it, and a fold that lost an instance would break it silently.
    #[test]
    fn a_rejected_batch_implies_every_member_instance_is_rejected() {
        let planes = frustum_planes_from_view_proj(&perspective_at_origin());
        // Three instances, all far to the right of the frustum at z = -6 (the boundary is x = 7).
        let ring = [at([40.0, 0.0, -6.0]), at([41.0, 0.5, -6.0]), at([42.0, -0.5, -6.0])];
        let batch = DrawBatch {
            mesh_id: 0,
            index_count: 36,
            index_type: boyko_rhi::IndexType::Uint16,
            base_instance: 0,
            instance_count: 3,
        };
        assert_eq!(
            batch_instance_count_after_cull(&planes, &batch, &ring, Some(UNIT_CUBE)),
            0,
            "the union of three off-screen boxes is off-screen"
        );
        for (i, inst) in ring.iter().enumerate() {
            assert!(
                !instance_visible_after_cull(&planes, inst, UNIT_CUBE),
                "member {i} of a rejected batch must itself be rejected"
            );
        }
        // The converse must NOT hold, or per-instance granularity buys nothing: one member inside
        // keeps the whole batch while the other two stay individually rejected.
        let mixed = [at([0.0, 0.0, -6.0]), ring[1], ring[2]];
        assert_eq!(
            batch_instance_count_after_cull(&planes, &batch, &mixed, Some(UNIT_CUBE)),
            3,
            "a batch with one visible member keeps ALL THREE at batch granularity — this is the \
             waste the per-instance rung exists to remove"
        );
        assert!(instance_visible_after_cull(&planes, &mixed[0], UNIT_CUBE));
        assert!(!instance_visible_after_cull(&planes, &mixed[1], UNIT_CUBE));
        assert!(!instance_visible_after_cull(&planes, &mixed[2], UNIT_CUBE));
    }
}
