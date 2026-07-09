//! Convex narrowphase contact generators (P2 W4) — sphere-box and box-box.
//!
//! These are pure CPU geometry: they read two [`BodyState`](crate::resources::BodyState)
//! snapshots and emit a [`Manifold`](crate::manifold::Manifold) (the universal
//! contact currency). ZERO `unsafe`, no allocation (the manifold is a
//! fixed-capacity POD), single-threaded, deterministic. The sphere-sphere
//! generator stays inline in [`physics_narrowphase`](crate::systems::physics_narrowphase);
//! the box generators live here because they are the heavy correctness surface.
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

pub mod axis_cache;
pub mod box_box;
pub mod sphere_box;

/// High bit (bit 15): SET for the non-face-face classes (edge-edge, vertex-face),
/// CLEAR for face-face. Keeps the three classes' feature ids disjoint so a
/// class transition correctly misses warm-start.
const TAG_NON_FACE: u32 = 0x8000;

/// Bit 14: within the `TAG_NON_FACE` region, SET selects vertex-face, CLEAR
/// selects edge-edge.
const TAG_VERTEX_FACE: u32 = 0x4000;

/// Packs a face-face contact's feature id from the reference-face index and the
/// incident-face vertex index (P2 W3/W4).
///
/// `ref_face ∈ 0..6` (the 6 box faces), `incident_vtx ∈ 0..8` (the clipped
/// incident-face vertex). The high bit stays CLEAR (face-face class). The id is a
/// pure function of the clipped feature identity, so it is stable as long as the
/// same reference face clips the same incident vertex — independent of the raw
/// SAT min-axis numbering.
#[inline]
pub fn feature_face_face(ref_face: u32, incident_vtx: u32) -> u32 {
    debug_assert!(ref_face < 6, "invariant: a box has 6 faces (ref_face < 6)");
    debug_assert!(incident_vtx < 8, "invariant: a box has 8 vertices (incident_vtx < 8)");
    // 3 bits ref_face (0..6), 3 bits incident_vtx (0..8): max 0b101_101 = 0x2D,
    // well below bit 15 — the face-face class is the low region.
    (ref_face << 3) | incident_vtx
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
    use super::*;

    /// The three feature-id classes are pairwise DISJOINT across their full index
    /// ranges (no id produced by one class equals an id from another). This is the
    /// warm-start-miss-on-class-transition guarantee (P2 W4).
    #[test]
    fn feature_id_classes_are_disjoint() {
        let mut face_face = Vec::new();
        for rf in 0..6 {
            for v in 0..8 {
                face_face.push(feature_face_face(rf, v));
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

    /// Each class is internally injective (distinct inputs → distinct ids), so
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
