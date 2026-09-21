//! Convex narrowphase contact generators (P2 W4) — sphere-box and box-box.
//!
//! These are pure CPU geometry: they read two [`BodyState`](crate::resources::BodyState)
//! snapshots and emit a [`Manifold`](crate::manifold::Manifold) (the universal
//! contact currency). ZERO `unsafe`, no allocation (the manifold is a
//! fixed-capacity POD), single-threaded, deterministic. The sphere-sphere
//! generator stays inline in [`physics_narrowphase`](crate::systems::physics_narrowphase);
//! the box generators live here because they are the heavy correctness surface.
//!
//! The one module here that is not a generator is `dispatch`, the parallel narrowphase (L5):
//! it runs the same per-pair collision over contiguous chunks of the candidate pairs on the
//! ambient pool's workers and joins their output back in pair order. Its `unsafe` is the three
//! per-row writes of a chunk into rows no other chunk owns; its public surface is the three
//! chunking constants re-exported below.
//!
//! # OBB convention
//!
//! A [`ColliderShape::Box`](crate::components::ColliderShape::Box) is an ORIENTED
//! box: the world box is the body's
//! [`rotation`](crate::components::RigidBody::rotation) applied to the LOCAL
//! `half_extents` about the body position. The generators transform into a box's
//! local frame via [`Quat::inverse_rotate`](crate::math::Quat::inverse_rotate)
//! and back via [`Quat::rotate`](crate::math::Quat::rotate).
//!
//! # Normal orientation
//!
//! Every generator emits the manifold `normal` pointing from body A toward body B
//! (the sphere-sphere `(posB − posA)` convention), so the solver's sign handling
//! is uniform across shape pairs.
//!
//! # Feature ids (warm-start identity, P2 W3/W4)
//!
//! A contact point's `feature_id` is a stable tag for cross-frame warm-start
//! matching. The three box contact classes are DISJOINT via high-bit tags so a
//! transition between classes (e.g. face-face → edge-edge as boxes tip) is a
//! genuine warm-start MISS rather than an alias to a different contact:
//!
//! - **face-face** (bit 15 CLEAR): [`feature_face_face`] packs the reference-face
//!   index (`0..6`) and the incident-vertex index (`0..8`).
//! - **edge-edge** (bit 15 SET, bit 14 CLEAR): [`feature_edge_edge`] packs the
//!   two edge-axis indices.
//! - **vertex-face** (bit 15 SET, bit 14 SET): [`feature_vertex_face`] tags a
//!   single deepest vertex (the sphere-box / box-corner case).
//!
//! The id is derived from the CLIPPED-FEATURE identity, not the raw SAT axis
//! index, so it does not flip when the SAT axis flips under FP noise — that, plus
//! the reference-axis hysteresis in [`box_box`], is what holds a resting box stack.
//!
//! # Why a face-face manifold needs TWO id encoders (A7a)
//!
//! Only the incident face's ORIGINAL corners are named by a corner index. A
//! Sutherland-Hodgman intersection is born on an edge and has no corner of its
//! own, and naming it after one of its two endpoints is not an identity: a
//! quarter-overlap face contact clips all four corners away and every surviving
//! point inherits the same endpoint, so one manifold carries one id four times.
//! `warm_start::pack` then packs equal keys and the open-addressed insert
//! overwrites all but one of them. Measured on a resting height-15 pile: 1926 of
//! 5044 manifolds (38.2 %) and 3682 of 12817 contact points (28.7 %) are exposed
//! to it.
//!
//! An intersection is therefore named by the two features that CREATED it — the
//! polygon edge it was cut on and the reference-face side plane that cut it —
//! via [`feature_face_clip`], the shape Box2D Lite's four-field `FeaturePair`,
//! Box2D v3's `B2_MAKE_ID` and Box3D's `b3MakeFeaturePair` all take. Both
//! encoders live in the face-face class (bit 15 CLEAR) and are separated from
//! each other by bit 13, so a corner and an intersection never alias while a
//! class transition still misses warm-start.

pub mod axis_cache;
pub mod box_box;
pub(crate) mod dispatch;
pub mod sphere_box;

pub use dispatch::{NP_CHUNKS_PER_LANE, NP_MAX_CHUNKS, NP_MIN_PAIRS_PER_CHUNK};

/// High bit (bit 15): SET for the non-face-face classes (edge-edge, vertex-face),
/// CLEAR for face-face. Keeps the three classes' feature ids disjoint so a
/// class transition correctly misses warm-start.
const TAG_NON_FACE: u32 = 0x8000;

/// Bit 14: within the `TAG_NON_FACE` region, SET selects vertex-face, CLEAR
/// selects edge-edge.
const TAG_VERTEX_FACE: u32 = 0x4000;

/// Bit 13: within the face-face class (bits 15 and 14 CLEAR), SET selects a
/// CLIPPED intersection ([`feature_face_clip`]), CLEAR an original incident
/// corner ([`feature_face_face`]).
const TAG_FACE_CLIP: u32 = 0x2000;

/// Packs a face-face contact's feature id from the reference-face index and the
/// index of an ORIGINAL incident-face corner (P2 W3/W4).
///
/// `ref_face ∈ 0..6` (the 6 box faces), `incident_vtx ∈ 0..8` (the incident
/// face's own corner, the one that survived the clip). A vertex the clip
/// CREATED has no corner of its own and is named by [`feature_face_clip`]
/// instead. The high bit stays CLEAR (face-face class). The id is a pure
/// function of the clipped feature identity, so it is stable as long as the same
/// reference face keeps the same incident corner — independent of the raw SAT
/// min-axis numbering.
#[inline]
pub fn feature_face_face(ref_face: u32, incident_vtx: u32) -> u32 {
    debug_assert!(ref_face < 6, "invariant: a box has 6 faces (ref_face < 6)");
    debug_assert!(incident_vtx < 8, "invariant: a box has 8 vertices (incident_vtx < 8)");
    // 3 bits ref_face (0..6), 3 bits incident_vtx (0..8): max 0b101_111 = 0x2F,
    // below bit 13 — surviving corners are the low region of the face-face class.
    (ref_face << 3) | incident_vtx
}

/// Packs a CLIPPED face-face contact vertex's feature id from the two features
/// that created it: the polygon edge it was cut on and the reference-face side
/// plane that cut it (A7a).
///
/// `ref_face ∈ 0..6`, `cut_edge ∈ 0..12` (the incident face's ring edges `0..4`,
/// or `8 + q` for an edge lying in reference side plane `q` — an edge a previous
/// clip pass created), `plane ∈ 0..4` (the reference side plane index).
///
/// Layout, high → low: bit 13 `TAG_FACE_CLIP` | `ref_face` 3 bits << 6 |
/// `cut_edge` 4 bits << 2 | `plane` 2 bits. The maximum is `0x216F`, so bits 15
/// and 14 stay CLEAR — this is still the face-face class, disjoint from
/// edge-edge and vertex-face — and the id fits the 16-bit feature field of
/// [`warm_start::pack`](crate::solver::warm_start::pack).
///
/// # Injectivity within one manifold
///
/// The three arguments are the whole identity, and each is pinned:
///
/// - `ref_face` is a constant of the manifold (one reference face clips the
///   whole incident polygon), so it never separates two points and never lets
///   two manifolds of different reference faces share an id.
/// - `plane` is clipped exactly once per manifold — the four side planes run in
///   a fixed order — so two intersections cut by different planes differ here.
/// - `cut_edge` separates the (at most two) intersections one plane creates: a
///   convex polygon crosses a plane at most twice, on two DISTINCT edges, and
///   the working ring holds at most one edge per label (each original ring edge
///   survives as at most one segment, and a clip against plane `q` adds at most
///   one new edge, labelled `8 + q` — none when the plane does not cut).
///
/// Distinct intersections therefore carry distinct ids under every clipping
/// order, and bit 13 keeps them disjoint from the surviving corners' ids —
/// which is the whole of A7a: every point of a face manifold gets its own
/// warm-start key.
#[inline]
pub fn feature_face_clip(ref_face: u32, cut_edge: u32, plane: u32) -> u32 {
    debug_assert!(ref_face < 6, "invariant: a box has 6 faces (ref_face < 6)");
    debug_assert!(cut_edge < 12, "invariant: 4 incident ring edges + 4 clip-plane edges (cut_edge < 12)");
    debug_assert!(plane < 4, "invariant: a reference face has 4 side planes (plane < 4)");
    TAG_FACE_CLIP | (ref_face << 6) | (cut_edge << 2) | plane
}

/// Packs an edge-edge contact's feature id from the two crossed edge-axis indices
/// (P2 W4).
///
/// `edge_a` / `edge_b ∈ 0..3` (which local axis of each box the contacting edge
/// runs along). The high bit is SET (non-face-face) and bit 14 CLEAR (edge-edge),
/// so this never collides with a face-face or vertex-face id.
#[inline]
pub fn feature_edge_edge(edge_a: u32, edge_b: u32) -> u32 {
    debug_assert!(edge_a < 3 && edge_b < 3, "invariant: edge axes are 0..3");
    TAG_NON_FACE | (edge_a << 4) | edge_b
}

/// Packs a vertex-face contact's feature id from the single deepest vertex index
/// (P2 W4 — the sphere-box / box-corner single-point case).
///
/// `vtx ∈ 0..8`. The high bit and bit 14 are both SET (vertex-face), so this is
/// disjoint from both other classes.
#[inline]
pub fn feature_vertex_face(vtx: u32) -> u32 {
    debug_assert!(vtx < 8, "invariant: a box has 8 vertices (vtx < 8)");
    TAG_NON_FACE | TAG_VERTEX_FACE | vtx
}

#[cfg(test)]
mod tests {
    // Test-oracle model: the std `HashSet` is the REFERENCE injectivity checker for the
    // bit-packed contact-feature ids. Compiled out of every shipping build; narrowphase
    // itself never builds a set.
    #![allow(clippy::disallowed_types)]

    use super::*;

    /// Every id of every encoder, in index order: `(face_face, face_clip,
    /// edge_edge, vertex_face)`. Exhaustive over each encoder's whole input
    /// range, so the disjointness check below is a proof rather than a sample.
    fn all_ids() -> (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>) {
        let mut face_face = Vec::new();
        for rf in 0..6 {
            for v in 0..8 {
                face_face.push(feature_face_face(rf, v));
            }
        }
        let mut face_clip = Vec::new();
        for rf in 0..6 {
            for e in 0..12 {
                for p in 0..4 {
                    face_clip.push(feature_face_clip(rf, e, p));
                }
            }
        }
        let mut edge_edge = Vec::new();
        for ea in 0..3 {
            for eb in 0..3 {
                edge_edge.push(feature_edge_edge(ea, eb));
            }
        }
        let mut vertex_face = Vec::new();
        for v in 0..8 {
            vertex_face.push(feature_vertex_face(v));
        }
        (face_face, face_clip, edge_edge, vertex_face)
    }

    /// The four feature-id encoders are pairwise DISJOINT across their full index
    /// ranges (no id produced by one equals an id from another). Across the three
    /// CLASSES this is the warm-start-miss-on-class-transition guarantee (P2 W4);
    /// between the two face-face encoders it is A7a's own guarantee — a surviving
    /// incident corner never aliases a vertex the clip created.
    #[test]
    fn feature_id_classes_are_disjoint() {
        let (face_face, face_clip, edge_edge, vertex_face) = all_ids();

        for &fc in &face_clip {
            assert!(fc & TAG_NON_FACE == 0, "face-clip must stay in the face-face class: {fc:#x}");
            assert!(fc & TAG_VERTEX_FACE == 0, "face-clip must stay in the face-face class: {fc:#x}");
            assert!(fc & TAG_FACE_CLIP != 0, "face-clip must set bit 13: {fc:#x}");
            assert!(fc <= 0x216F, "face-clip must fit the documented maximum: {fc:#x}");
            assert!(!face_face.contains(&fc), "face-clip aliases a face-face corner id: {fc:#x}");
        }
        for &ff in &face_face {
            assert!(ff & TAG_FACE_CLIP == 0, "a corner id must clear bit 13: {ff:#x}");
        }
        // Every id of every encoder fits the 16-bit warm-start key field.
        for &id in face_face.iter().chain(&face_clip).chain(&edge_edge).chain(&vertex_face) {
            assert!(id <= 0xFFFF, "feature id must fit warm_start::pack's 16-bit field: {id:#x}");
        }

        for &ff in &face_face {
            assert!(ff & TAG_NON_FACE == 0, "face-face must clear the high bit: {ff:#x}");
            assert!(!edge_edge.contains(&ff), "face-face aliases an edge-edge id: {ff:#x}");
            assert!(!vertex_face.contains(&ff), "face-face aliases a vertex-face id: {ff:#x}");
        }
        for &ee in &edge_edge {
            assert!(ee & TAG_NON_FACE != 0 && ee & TAG_VERTEX_FACE == 0, "edge-edge tag: {ee:#x}");
            assert!(!vertex_face.contains(&ee), "edge-edge aliases a vertex-face id: {ee:#x}");
        }
        for &vf in &vertex_face {
            assert!(vf & TAG_NON_FACE != 0 && vf & TAG_VERTEX_FACE != 0, "vertex-face tag: {vf:#x}");
        }
    }

    /// Each encoder is internally injective (distinct inputs → distinct ids), so
    /// distinct contact features warm-start independently.
    #[test]
    fn feature_id_is_injective_within_a_class() {
        let mut seen = std::collections::HashSet::new();
        for rf in 0..6 {
            for v in 0..8 {
                assert!(seen.insert(feature_face_face(rf, v)), "face-face id collision");
            }
        }
        seen.clear();
        for rf in 0..6 {
            for e in 0..12 {
                for p in 0..4 {
                    assert!(
                        seen.insert(feature_face_clip(rf, e, p)),
                        "face-clip id collision at (ref_face {rf}, cut_edge {e}, plane {p})"
                    );
                }
            }
        }
        seen.clear();
        for ea in 0..3 {
            for eb in 0..3 {
                assert!(seen.insert(feature_edge_edge(ea, eb)), "edge-edge id collision");
            }
        }
        seen.clear();
        for v in 0..8 {
            assert!(seen.insert(feature_vertex_face(v)), "vertex-face id collision");
        }
    }
}
