//! Box-vs-box (OBB-OBB) contact generation (P2 W4) — the heavy convex generator.
//!
//! The standard separating-axis (SAT) + reference-face clip + bounded-point
//! reduction pipeline, with the feature-id stability machinery a resting box
//! stack needs (P2 W3 precondition):
//!
//! 1. **15-axis SAT**: the 3 face axes of each box plus the 9 edge-edge cross
//!    products. The axis of LEAST penetration is the contact axis; a positive gap
//!    on ANY axis means the boxes are separated (no contact).
//! 2. **Reference-face clip** (min axis is a face axis): the reference face is the
//!    one on that axis; the incident face is the other box's most anti-parallel
//!    face; the incident polygon is Sutherland-Hodgman-clipped against the
//!    reference face's 4 side planes, and points below the reference face are kept.
//! 3. **Edge-edge** (min axis is a cross product): a single contact at the closest
//!    points of the two contacting edges.
//! 4. **Deterministic ≤4-point reduction**: keep the deepest point plus the three
//!    that maximize the contact-patch spread, ties broken by lowest incident-vertex
//!    index — a pure function of the clipped polygon, so the selection is
//!    reproducible (no FP-tie nondeterminism).
//! 5. **Reference-axis hysteresis**: bias toward last frame's reference axis to
//!    stop the min axis (hence the feature ids) from flickering under FP noise on a
//!    near-parallel resting stack.
//!
//! ZERO `unsafe`, no heap allocation (fixed-size stack buffers), deterministic.

use crate::manifold::{BodyIndex, ContactPoint, Manifold};
use crate::math::{Mat3, Quat, Vec3};

use super::{feature_edge_edge, feature_face_face};

/// Penetration ratio within which the current best SAT axis is considered "no
/// better" than last frame's, so the hysteresis keeps last frame's axis (P2 W4 —
/// the resting-stack feature-id flicker guard). `1.05` = a 5 % bias toward
/// stability (matches the W3 plan).
const HYSTERESIS_RATIO: f32 = 1.05;

/// A small absolute slop added when comparing SAT penetrations so two genuinely
/// co-equal axes (a perfectly axis-aligned resting pair) do not ping-pong on the
/// last bit of FP noise even without a stored last axis.
const SAT_EPS: f32 = 1.0e-5;

/// Alignment-comparison slop for [`most_aligned_face`]: a later axis must beat the
/// current best `|axis·dir|` by more than this to be picked, so a sub-epsilon FP
/// drift near a 45° bisector (where two faces are near-equally aligned) does not
/// flip the incident/reference face — which would change every clipped vertex's
/// feature id (a one-frame whole-manifold warm-start miss). Within the slop the
/// LOWEST axis index wins (a deterministic, pure-function tie-break).
const FACE_ALIGN_EPS: f32 = 1.0e-4;

/// Number of edge-edge cross-product axes (3 × 3 axis pairs).
const EDGE_AXES: usize = 9;

/// Total SAT axes: 3 A-face + 3 B-face + 9 edge-edge.
const SAT_AXES: usize = 6 + EDGE_AXES;

/// An oriented box resolved into world frame for the SAT test (P2 W4).
///
/// `axes[i]` is the world-space unit direction of the box's local axis `i`
/// (the rows of `Rᵀ` / columns of `R`), `half[i]` its half-extent along that axis.
#[derive(Clone, Copy, Debug)]
struct Obb {
    /// World center.
    center: Vec3,
    /// World-space unit axis directions (local x, y, z).
    axes: [Vec3; 3],
    /// Half-extents along each local axis.
    half: [f32; 3],
}

impl Obb {
    /// Builds the world OBB from a body's center, orientation, and local
    /// half-extents.
    #[inline]
    fn new(center: Vec3, rotation: Quat, half_extents: Vec3) -> Self {
        let r = Mat3::from_quat(rotation);
        // Column `i` of R is the world direction of local axis `i`. With row-major
        // storage, column `i` is `(rows[0][i], rows[1][i], rows[2][i])`.
        let axes = [
            Vec3::new(r.rows[0].x, r.rows[1].x, r.rows[2].x),
            Vec3::new(r.rows[0].y, r.rows[1].y, r.rows[2].y),
            Vec3::new(r.rows[0].z, r.rows[1].z, r.rows[2].z),
        ];
        Self {
            center,
            axes,
            half: [half_extents.x, half_extents.y, half_extents.z],
        }
    }

    /// The projection radius (support half-width) of the box onto unit `axis`:
    /// `Σ half[i] · |axis · axes[i]|`.
    #[inline]
    fn projection_radius(&self, axis: Vec3) -> f32 {
        self.half[0] * axis.dot(self.axes[0]).abs()
            + self.half[1] * axis.dot(self.axes[1]).abs()
            + self.half[2] * axis.dot(self.axes[2]).abs()
    }
}

/// The classification of the SAT axis of least penetration.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SatClass {
    /// A face axis of box A (axis index `0..3`).
    FaceA(usize),
    /// A face axis of box B (axis index `0..3`).
    FaceB(usize),
    /// An edge-edge cross product (A-axis `a`, B-axis `b`).
    Edge { a: usize, b: usize },
}

/// The result of the SAT query: the least-penetration axis (world, oriented A→B),
/// its penetration depth, its classification, and the canonical SAT-axis index
/// (`0..15`) for the hysteresis store.
#[derive(Clone, Copy, Debug)]
struct SatResult {
    /// The contact axis, world-frame, oriented from A toward B.
    axis: Vec3,
    /// Penetration depth along that axis (`≥ 0`; the boxes overlap by this much).
    /// Carried for diagnostics / the least-penetration unit test; the contact
    /// generators recompute per-point separations from the clipped geometry.
    #[allow(dead_code)]
    depth: f32,
    /// The geometric classification used to pick the contact-generation path.
    class: SatClass,
    /// The canonical SAT-axis index `0..SAT_AXES` (face A 0..3, face B 3..6,
    /// edge-edge 6..15) — the value persisted for the reference-axis hysteresis.
    index: usize,
}

/// One SAT axis candidate, evaluated for overlap.
#[derive(Clone, Copy)]
struct AxisCandidate {
    /// The (normalized) world axis, oriented A→B.
    axis: Vec3,
    /// Penetration depth (`> 0` = overlapping); `< 0` here means separated and is
    /// reported up as an immediate "no contact".
    depth: f32,
    /// Classification.
    class: SatClass,
    /// Canonical index `0..SAT_AXES`.
    index: usize,
}

/// Evaluates one SAT axis: returns the signed penetration (overlap of the two
/// projection radii minus the center separation along the axis) oriented so the
/// returned axis runs A→B, or `None` for a degenerate (near-zero) axis to skip.
#[inline]
fn eval_axis(a: &Obb, b: &Obb, raw_axis: Vec3, class: SatClass, index: usize) -> Option<AxisCandidate> {
    let len_sq = raw_axis.length_squared();
    if len_sq < 1.0e-8 {
        // Near-parallel edge pair (the cross product collapses): this axis carries
        // no separating information; skip it (the standard SAT degeneracy guard).
        return None;
    }
    let axis = raw_axis * len_sq.sqrt().recip();
    let center_delta = b.center - a.center;
    let separation = center_delta.dot(axis);
    // Penetration = (rA + rB) − |centerDelta · axis|. Orient the axis A→B (so a
    // positive `separation` keeps the axis; a negative one flips it).
    let overlap = a.projection_radius(axis) + b.projection_radius(axis) - separation.abs();
    let oriented = if separation < 0.0 { axis * -1.0 } else { axis };
    Some(AxisCandidate {
        axis: oriented,
        depth: overlap,
        class,
        index,
    })
}

/// Runs the 15-axis SAT and returns the least-penetration axis, or `None` if the
/// boxes are separated on any axis (P2 W4).
///
/// `last_axis` is last frame's chosen SAT-axis index (for the same body pair), or
/// `None` on a cold contact. When the current best axis is no deeper than
/// `HYSTERESIS_RATIO ×` last frame's axis penetration, last frame's axis is kept —
/// biasing toward a stable reference so the feature ids do not flicker on a
/// resting near-parallel stack.
//
// `clippy::needless_range_loop`: `i` is simultaneously the canonical SAT-axis
// index (stored in the candidate + used for the hysteresis), the `axes[i]`
// selector, and the `SatClass` payload — three roles a bare `enumerate()` over one
// array cannot carry, so the explicit index is the correct, readable form.
#[allow(clippy::needless_range_loop)]
fn sat(a: &Obb, b: &Obb, last_axis: Option<usize>) -> Option<SatResult> {
    // All 15 candidate axes in canonical order: A-face 0..3, B-face 3..6,
    // edge-edge 6..15 (a-major: (a0×b0, a0×b1, a0×b2, a1×b0, …)).
    let mut candidates: [Option<AxisCandidate>; SAT_AXES] = [None; SAT_AXES];
    for i in 0..3 {
        candidates[i] = eval_axis(a, b, a.axes[i], SatClass::FaceA(i), i);
    }
    for i in 0..3 {
        candidates[3 + i] = eval_axis(a, b, b.axes[i], SatClass::FaceB(i), 3 + i);
    }
    let mut k = 6;
    for ea in 0..3 {
        for eb in 0..3 {
            let axis = a.axes[ea].cross(b.axes[eb]);
            candidates[k] = eval_axis(a, b, axis, SatClass::Edge { a: ea, b: eb }, k);
            k += 1;
        }
    }

    // Any axis with non-overlap (depth < 0) ⇒ the boxes are separated.
    for cand in candidates.iter().flatten() {
        if cand.depth < 0.0 {
            return None;
        }
    }

    // Face axes are preferred over edge axes at equal depth (a face contact is
    // more stable than an edge contact), and ties break by LOWEST canonical index
    // — both make the min-axis selection a deterministic pure function of the
    // geometry, never FP-tie-order dependent.
    let mut best: Option<AxisCandidate> = None;
    for cand in candidates.iter().flatten() {
        best = Some(match best {
            None => *cand,
            Some(cur) => {
                let cand_is_edge = matches!(cand.class, SatClass::Edge { .. });
                let cur_is_edge = matches!(cur.class, SatClass::Edge { .. });
                // Strictly shallower wins; within SAT_EPS prefer a face axis, then
                // the lower canonical index (deterministic tie-break).
                if cand.depth < cur.depth - SAT_EPS {
                    *cand
                } else if cand.depth > cur.depth + SAT_EPS {
                    cur
                } else if !cand_is_edge && cur_is_edge {
                    *cand
                } else if cand_is_edge && !cur_is_edge {
                    cur
                } else if cand.index < cur.index {
                    *cand
                } else {
                    cur
                }
            }
        });
    }
    let best = best?;

    // Reference-axis hysteresis: if last frame's axis is still overlapping and the
    // current best is no deeper than HYSTERESIS_RATIO × last frame's depth, KEEP
    // last frame's axis (so the reference face — hence the feature ids — does not
    // flip on FP noise across a resting near-parallel pair).
    let chosen = match last_axis.and_then(|idx| candidates.get(idx).copied().flatten()) {
        Some(last) if last.index != best.index && best.depth >= last.depth / HYSTERESIS_RATIO => last,
        _ => best,
    };

    Some(SatResult {
        axis: chosen.axis,
        depth: chosen.depth,
        class: chosen.class,
        index: chosen.index,
    })
}

/// Identifies the box face (its outward LOCAL axis index + sign) whose outward
/// world normal is MOST aligned with `dir` (P2 W4). Returns `(axis, positive,
/// world_normal)`.
#[inline]
fn most_aligned_face(obb: &Obb, dir: Vec3) -> (usize, bool, Vec3) {
    let mut best_axis = 0usize;
    let mut best_dot = f32::MIN;
    let mut best_sign = 1.0f32;
    for i in 0..3 {
        let d = obb.axes[i].dot(dir);
        let aligned = d.abs();
        // The face whose OUTWARD normal (±axis) is most aligned with `dir`. A later
        // axis only wins if it is more aligned by MORE than `FACE_ALIGN_EPS`; a
        // near-tie (within the slop, e.g. a 45° bisector under FP noise) keeps the
        // earlier (lower-index) axis. This is a pure function of the inputs (same
        // geometry → same pick), so it preserves determinism while killing the
        // sub-epsilon incident-face flicker that would change every feature id.
        if aligned > best_dot + FACE_ALIGN_EPS {
            best_dot = aligned;
            best_axis = i;
            best_sign = if d >= 0.0 { 1.0 } else { -1.0 };
        }
    }
    let positive = best_sign > 0.0;
    (best_axis, positive, obb.axes[best_axis] * best_sign)
}

/// The 4 world-space vertices of `obb`'s face on local `axis`/`positive`, in a
/// consistent winding, along with their local-vertex (corner) indices (P2 W4).
#[inline]
fn face_vertices(obb: &Obb, axis: usize, positive: bool) -> [(Vec3, usize); 4] {
    // The two in-plane axes (the ones that are NOT the face axis).
    let (u, v) = match axis {
        0 => (1usize, 2usize),
        1 => (0usize, 2usize),
        _ => (0usize, 1usize),
    };
    let sign = if positive { 1.0 } else { -1.0 };
    let face_center = obb.center + obb.axes[axis] * (obb.half[axis] * sign);
    let eu = obb.axes[u] * obb.half[u];
    let ev = obb.axes[v] * obb.half[v];
    // Four corners in (−,−),(+,−),(+,+),(−,+) winding around the face. Recover the
    // corner index (bit per axis) so a clipped vertex can carry its incident id.
    let su = [-1.0, 1.0, 1.0, -1.0];
    let sv = [-1.0, -1.0, 1.0, 1.0];
    let mut out = [(Vec3::ZERO, 0usize); 4];
    for c in 0..4 {
        let pos = face_center + eu * su[c] + ev * sv[c];
        // Corner index bits: face axis bit set per `positive`, u bit per su sign,
        // v bit per sv sign.
        let mut idx = 0usize;
        if positive {
            idx |= 1 << axis;
        }
        if su[c] > 0.0 {
            idx |= 1 << u;
        }
        if sv[c] > 0.0 {
            idx |= 1 << v;
        }
        out[c] = (pos, idx);
    }
    out
}

/// A clipped contact vertex carried through Sutherland-Hodgman (P2 W4).
#[derive(Clone, Copy)]
struct ClipVertex {
    /// World position.
    pos: Vec3,
    /// Incident-face source corner index (`0..8`) — its feature identity. An
    /// interpolated vertex inherits the lower-index endpoint's corner so the id is
    /// deterministic.
    incident_vtx: usize,
}

/// Clips the polygon `poly` (`len` vertices) against the half-space `{ x : (x −
/// plane_point) · plane_normal ≤ 0 }` (keep the side the normal points AWAY from),
/// writing the result into `out` and returning its length (Sutherland-Hodgman, P2
/// W4). At most `len + 1` vertices are produced.
fn clip_against_plane(
    poly: &[ClipVertex],
    plane_point: Vec3,
    plane_normal: Vec3,
    out: &mut [ClipVertex],
) -> usize {
    let n = poly.len();
    if n == 0 {
        return 0;
    }
    let mut count = 0usize;
    let dist = |p: Vec3| (p - plane_point).dot(plane_normal);
    let mut prev = poly[n - 1];
    let mut prev_d = dist(prev.pos);
    for &cur in poly.iter() {
        let cur_d = dist(cur.pos);
        let prev_in = prev_d <= 0.0;
        let cur_in = cur_d <= 0.0;
        if cur_in {
            if !prev_in {
                // Entering: emit the intersection, then the current vertex.
                let t = prev_d / (prev_d - cur_d);
                out[count] = ClipVertex {
                    pos: prev.pos + (cur.pos - prev.pos) * t,
                    // Inherit the lower corner index for a deterministic id.
                    incident_vtx: prev.incident_vtx.min(cur.incident_vtx),
                };
                count += 1;
            }
            out[count] = cur;
            count += 1;
        } else if prev_in {
            // Leaving: emit only the intersection.
            let t = prev_d / (prev_d - cur_d);
            out[count] = ClipVertex {
                pos: prev.pos + (cur.pos - prev.pos) * t,
                incident_vtx: prev.incident_vtx.min(cur.incident_vtx),
            };
            count += 1;
        }
        prev = cur;
        prev_d = cur_d;
    }
    count
}

/// A scored candidate contact point after clipping (P2 W4 reduction input).
#[derive(Clone, Copy)]
struct ScoredPoint {
    /// World contact point (on the reference face, projected from the incident
    /// clipped vertex).
    pos: Vec3,
    /// Signed separation along the contact normal (negative = penetrating). Only
    /// penetrating points are kept.
    separation: f32,
    /// Incident-face source corner index (for the feature id + the tie-break).
    incident_vtx: usize,
}

/// Reduces a clipped point set to at most 4 contacts: the DEEPEST point plus the
/// up-to-3 that maximize the contact-patch spread, ties broken by the LOWEST
/// incident-vertex index — a pure function of the input (P2 W4).
///
/// `normal` is the contact normal (A→B); it defines the plane the patch lives in,
/// so the "two points off the diameter, one per side" split is measured by the
/// signed area along `normal` (`(edge × rel) · normal`) — a well-defined,
/// rotation-stable side test, not a fragile per-component sum.
///
/// Writes the kept points into `out` and returns the count (`≤ 4`).
fn reduce_points(points: &[ScoredPoint], normal: Vec3, out: &mut [ScoredPoint; 4]) -> usize {
    let n = points.len();
    if n == 0 {
        return 0;
    }
    if n <= 4 {
        for (i, &p) in points.iter().enumerate() {
            out[i] = p;
        }
        return n;
    }

    // 1) The deepest point (lowest separation); ties → lowest incident_vtx.
    let mut deepest = 0usize;
    for i in 1..n {
        let p = points[i];
        let d = points[deepest];
        if p.separation < d.separation
            || (p.separation == d.separation && p.incident_vtx < d.incident_vtx)
        {
            deepest = i;
        }
    }

    let mut chosen = [deepest, deepest, deepest, deepest];
    let mut chosen_len = 1usize;

    // 2) The point farthest from the deepest (one diameter of the patch).
    let base = points[deepest].pos;
    let mut far = deepest;
    let mut far_d2 = -1.0f32;
    for i in 0..n {
        let d2 = (points[i].pos - base).length_squared();
        let cur = points[far];
        if d2 > far_d2 || (d2 == far_d2 && points[i].incident_vtx < cur.incident_vtx) {
            far_d2 = d2;
            far = i;
        }
    }
    if far != deepest {
        chosen[chosen_len] = far;
        chosen_len += 1;
    }

    // 3) + 4) The two points maximizing the signed area of the quad (the widest
    // spread off the deepest↔far diameter, one on each side). The diameter is
    // `(base → points[far])`; score each remaining point by its perpendicular
    // distance off that line, keeping the most positive and most negative.
    if chosen_len >= 2 {
        let edge = points[far].pos - base;
        let mut best_pos = -1.0f32;
        let mut best_pos_i = usize::MAX;
        let mut best_neg = -1.0f32;
        let mut best_neg_i = usize::MAX;
        for i in 0..n {
            if i == deepest || i == far {
                continue;
            }
            // Signed area (×2) of the triangle (base, far, points[i]) measured in
            // the contact plane: `(edge × rel) · normal`. Its sign is the side of
            // the deepest↔far diameter the point lies on; its magnitude is the
            // perpendicular spread. Projecting onto `normal` (not summing the raw
            // components) makes the side test geometrically correct for ANY
            // contact orientation and deterministic.
            let rel = points[i].pos - base;
            let cross = edge.cross(rel);
            let signed_area = cross.dot(normal);
            let area = signed_area.abs();
            if signed_area >= 0.0 {
                if area > best_pos
                    || (area == best_pos
                        && best_pos_i != usize::MAX
                        && points[i].incident_vtx < points[best_pos_i].incident_vtx)
                {
                    best_pos = area;
                    best_pos_i = i;
                }
            } else if area > best_neg
                || (area == best_neg
                    && best_neg_i != usize::MAX
                    && points[i].incident_vtx < points[best_neg_i].incident_vtx)
            {
                best_neg = area;
                best_neg_i = i;
            }
        }
        if best_pos_i != usize::MAX && chosen_len < 4 {
            chosen[chosen_len] = best_pos_i;
            chosen_len += 1;
        }
        if best_neg_i != usize::MAX && chosen_len < 4 {
            chosen[chosen_len] = best_neg_i;
            chosen_len += 1;
        }
    }

    for i in 0..chosen_len {
        out[i] = points[chosen[i]];
    }
    chosen_len
}

/// Generates the box-box contact between OBB body A and OBB body B, or `None` when
/// they do not overlap (P2 W4). `last_axis` is the previous frame's chosen SAT-axis
/// index for this body pair (hysteresis); the returned manifold carries the new
/// axis index out-of-band via [`BoxBoxContact::reference_axis`].
//
// `clippy::too_many_arguments`: a convex-convex generator genuinely needs both
// bodies' (center, rotation, half-extents) plus the two row indices and the
// hysteresis axis. Grouping them into a struct would just shuffle the same data
// across the call boundary (the caller has them as separate `BodyState` fields);
// the flat signature mirrors `sphere_box_contact`.
#[allow(clippy::too_many_arguments)]
pub fn box_box_contact(
    body_a: BodyIndex,
    body_b: BodyIndex,
    a_center: Vec3,
    a_rotation: Quat,
    a_half: Vec3,
    b_center: Vec3,
    b_rotation: Quat,
    b_half: Vec3,
    last_axis: Option<usize>,
) -> Option<BoxBoxContact> {
    let a = Obb::new(a_center, a_rotation, a_half);
    let b = Obb::new(b_center, b_rotation, b_half);

    let sat = sat(&a, &b, last_axis)?;

    let manifold = match sat.class {
        SatClass::FaceA(_) | SatClass::FaceB(_) => face_contact(&a, &b, &sat, body_a, body_b),
        SatClass::Edge { a: ea, b: eb } => {
            edge_contact(&a, &b, &sat, ea, eb, body_a, body_b)
        }
    };

    manifold.map(|m| BoxBoxContact {
        manifold: m,
        reference_axis: sat.index,
    })
}

/// The box-box result: the contact manifold plus the chosen SAT-axis index to
/// persist for next frame's hysteresis (P2 W4).
pub struct BoxBoxContact {
    /// The contact manifold (normal runs A→B, up to 4 points).
    pub manifold: Manifold,
    /// The canonical SAT-axis index (`0..15`) chosen this frame — the caller
    /// stores it keyed by the body pair to bias next frame's SAT selection.
    pub reference_axis: usize,
}

/// Builds a face-contact manifold via reference-face clip + reduction (P2 W4).
fn face_contact(
    a: &Obb,
    b: &Obb,
    sat: &SatResult,
    body_a: BodyIndex,
    body_b: BodyIndex,
) -> Option<Manifold> {
    // The SAT normal runs A→B. The reference box is the one OWNING the min face
    // axis; the incident box is the other.
    let a_is_reference = matches!(sat.class, SatClass::FaceA(_));
    let (reference, incident, ref_normal_dir) = if a_is_reference {
        // Reference face's outward normal points A→B (toward the incident box B).
        (a, b, sat.axis)
    } else {
        // Reference is B; its outward face points B→A = −(A→B).
        (b, a, sat.axis * -1.0)
    };

    // Reference face = the face whose outward normal is most aligned with the
    // A→B(or B→A) contact direction.
    let (ref_axis, ref_positive, ref_normal) = most_aligned_face(reference, ref_normal_dir);
    let ref_face_idx = ref_axis * 2 + usize::from(ref_positive);
    // Incident face = the other box's face MOST ANTI-PARALLEL to the reference
    // normal (its outward normal points back at the reference).
    let (inc_axis, inc_positive, _) = most_aligned_face(incident, ref_normal * -1.0);

    let ref_face = face_vertices(reference, ref_axis, ref_positive);
    let inc_face = face_vertices(incident, inc_axis, inc_positive);

    // Seed the clip polygon with the incident face (carrying corner ids).
    let mut buf_a = [ClipVertex {
        pos: Vec3::ZERO,
        incident_vtx: 0,
    }; 8];
    let mut buf_b = buf_a;
    let mut poly_len = 4usize;
    for (i, &(pos, idx)) in inc_face.iter().enumerate() {
        buf_a[i] = ClipVertex {
            pos,
            incident_vtx: idx,
        };
    }

    // The reference-face center + outward normal, for the side planes and the
    // below-face keep test.
    let ref_face_center = ref_face
        .iter()
        .fold(Vec3::ZERO, |acc, &(p, _)| acc + p)
        * 0.25;

    // Clip the incident polygon against the reference face's 4 side planes. Each
    // side plane's outward normal is `(edge midpoint − face center)` projected
    // into the face plane; keep the half-space on the inside.
    let (mut src, mut dst) = (&mut buf_a, &mut buf_b);
    for e in 0..4 {
        let v0 = ref_face[e].0;
        let v1 = ref_face[(e + 1) % 4].0;
        let edge_mid = (v0 + v1) * 0.5;
        let edge = v1 - v0;
        // Outward side-plane normal: perpendicular to the face edge AND to the ref
        // normal, pointing away from the face center.
        let mut side_normal = edge.cross(ref_normal).normalize();
        // Degeneracy guard (matching the module's `eval_axis` / `normalize` / sphere
        // r≤0 posture): a zero side normal means the reference face has a zero
        // in-plane half-extent (a zero-volume box face, e.g. `half_extents.x == 0`).
        // `clip_against_plane` against a zero normal keeps EVERY vertex (clips
        // nothing) → a malformed over-large manifold. `half_extents` is unvalidated
        // user data, so reject the degenerate reference face: no contact. The
        // debug_assert catches the upstream zero-extent collider in tests.
        debug_assert!(
            side_normal != Vec3::ZERO,
            "invariant: a non-degenerate reference face has non-zero in-plane extents"
        );
        if side_normal == Vec3::ZERO {
            return None;
        }
        if (edge_mid - ref_face_center).dot(side_normal) < 0.0 {
            side_normal = side_normal * -1.0;
        }
        let new_len = clip_against_plane(&src[..poly_len], edge_mid, side_normal, dst);
        core::mem::swap(&mut src, &mut dst);
        poly_len = new_len;
        if poly_len == 0 {
            return None;
        }
    }

    // Keep only vertices BELOW the reference face (penetrating), projecting each
    // onto the reference face for the contact anchor and computing its separation.
    let mut scored: [ScoredPoint; 8] = [ScoredPoint {
        pos: Vec3::ZERO,
        separation: 0.0,
        incident_vtx: 0,
    }; 8];
    let mut scored_len = 0usize;
    for &cv in &src[..poly_len] {
        let separation = (cv.pos - ref_face_center).dot(ref_normal);
        if separation <= 0.0 {
            scored[scored_len] = ScoredPoint {
                pos: cv.pos,
                separation,
                incident_vtx: cv.incident_vtx,
            };
            scored_len += 1;
        }
    }
    if scored_len == 0 {
        return None;
    }

    let mut reduced = [ScoredPoint {
        pos: Vec3::ZERO,
        separation: 0.0,
        incident_vtx: 0,
    }; 4];
    // The manifold normal runs A→B regardless of which box was the reference; it
    // also defines the contact plane the patch reduction measures spread in.
    let normal = sat.axis;
    let count = reduce_points(&scored[..scored_len], normal, &mut reduced);
    let mut manifold = Manifold::new(body_a, body_b);
    manifold.normal = normal;
    for (i, &p) in reduced[..count].iter().enumerate() {
        // Anchor on the reference face = project the incident point onto it; the
        // incident anchor is the clipped incident point itself.
        let on_reference = p.pos - ref_normal * p.separation;
        // anchor_a / anchor_b are tagged to the actual body A / B (the reference
        // may be B), so the solver's r-vectors come out on the right body.
        let (anchor_a, anchor_b) = if a_is_reference {
            (on_reference, p.pos)
        } else {
            (p.pos, on_reference)
        };
        manifold.points[i] = ContactPoint {
            anchor_a,
            anchor_b,
            separation: p.separation,
            feature_id: feature_face_face(ref_face_idx as u32, p.incident_vtx as u32),
        };
    }
    manifold.count = count as u8;
    debug_assert!(
        (manifold.count as usize) <= crate::math::MAX_CONTACT_POINTS,
        "invariant: box-box manifold count must not exceed MAX_CONTACT_POINTS"
    );
    Some(manifold)
}

/// Builds a single-point edge-edge contact at the closest points of the two
/// contacting edges (P2 W4).
fn edge_contact(
    a: &Obb,
    b: &Obb,
    sat: &SatResult,
    ea: usize,
    eb: usize,
    body_a: BodyIndex,
    body_b: BodyIndex,
) -> Option<Manifold> {
    // The contact axis runs A→B; the contacting edge of each box is the one along
    // local axis `ea` / `eb`, offset to the side facing the other box.
    let normal = sat.axis;

    // A's contacting edge: its support point along +normal (most toward B), with
    // the edge running along `a.axes[ea]`.
    let pa = support_edge_point(a, ea, normal);
    // B's contacting edge: its support point along −normal (most toward A).
    let pb = support_edge_point(b, eb, normal * -1.0);

    let da = a.axes[ea];
    let db = b.axes[eb];
    let (ca, cb) = closest_points_on_segments(pa, da, a.half[ea], pb, db, b.half[eb]);

    let separation = (cb - ca).dot(normal);
    let mut manifold = Manifold::new(body_a, body_b);
    manifold.normal = normal;
    manifold.points[0] = ContactPoint {
        anchor_a: ca,
        anchor_b: cb,
        separation,
        feature_id: feature_edge_edge(ea as u32, eb as u32),
    };
    manifold.count = 1;
    Some(manifold)
}

/// The center of the box edge running along local axis `edge_axis` that is most
/// extreme along `dir` (the contacting edge), in world space (P2 W4).
#[inline]
fn support_edge_point(obb: &Obb, edge_axis: usize, dir: Vec3) -> Vec3 {
    let mut p = obb.center;
    for i in 0..3 {
        if i == edge_axis {
            continue;
        }
        let s = if obb.axes[i].dot(dir) >= 0.0 { 1.0 } else { -1.0 };
        p = p + obb.axes[i] * (obb.half[i] * s);
    }
    p
}

/// The closest points between two segments centered at `pa` / `pb` with unit
/// directions `da` / `db` and half-lengths `ha` / `hb` (P2 W4).
fn closest_points_on_segments(
    pa: Vec3,
    da: Vec3,
    ha: f32,
    pb: Vec3,
    db: Vec3,
    hb: f32,
) -> (Vec3, Vec3) {
    let r = pa - pb;
    let a = da.dot(da); // = 1 (unit)
    let e = db.dot(db); // = 1 (unit)
    let f = db.dot(r);
    let c = da.dot(r);
    let b = da.dot(db);
    let denom = a * e - b * b;
    // Parameter along A's segment.
    let s = if denom.abs() > 1.0e-8 {
        ((b * f - c * e) / denom).clamp(-ha, ha)
    } else {
        0.0 // parallel: pick the centers (the SAT degeneracy guard skips true
            // parallels, so this is a near-parallel fallback)
    };
    // Parameter along B's segment from A's chosen point.
    let t = ((b * s + f) / e).clamp(-hb, hb);
    let s = ((b * t - c) / a).clamp(-ha, ha);
    (pa + da * s, pb + db * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: BodyIndex = BodyIndex(0);
    const B: BodyIndex = BodyIndex(1);

    fn unit_box(center: Vec3) -> (Vec3, Quat, Vec3) {
        (center, Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0))
    }

    /// Two axis-aligned unit boxes stacked with a small overlap along +y: the SAT
    /// min axis is the y face, the normal runs A→B (+y), and a face contact yields
    /// up to 4 points all penetrating.
    #[test]
    fn stacked_boxes_face_contact_four_points() {
        let (ac, ar, ah) = unit_box(Vec3::ZERO);
        // B sits above A overlapping by 0.1 (centers 1.9 apart, half-sum 2.0).
        let (bc, br, bh) = unit_box(Vec3::new(0.0, 1.9, 0.0));
        let c = box_box_contact(A, B, ac, ar, ah, bc, br, bh, None)
            .expect("overlapping boxes must contact");
        let m = c.manifold;
        assert!(m.normal.y > 0.99, "normal must run A→B (+y): {:?}", m.normal);
        assert!(m.count >= 1, "a face contact must produce >= 1 point");
        assert!(m.count <= 4, "reduction caps at 4 points, got {}", m.count);
        for p in &m.points[..m.count as usize] {
            assert!(p.separation <= 1e-4, "kept points must be penetrating: {}", p.separation);
        }
    }

    /// Separated boxes produce no contact (SAT finds a separating axis).
    #[test]
    fn separated_boxes_no_contact() {
        let (ac, ar, ah) = unit_box(Vec3::ZERO);
        let (bc, br, bh) = unit_box(Vec3::new(0.0, 5.0, 0.0));
        assert!(
            box_box_contact(A, B, ac, ar, ah, bc, br, bh, None).is_none(),
            "boxes 5 apart must be separated"
        );
    }

    /// The SAT axis of least penetration on a known overlap is the shallow axis.
    /// Boxes overlap deeply in x/z but barely in y → the min axis is the y face.
    #[test]
    fn sat_picks_axis_of_least_penetration() {
        let a = Obb::new(Vec3::ZERO, Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0));
        // B overlaps A by 0.1 in y, fully in x and z.
        let b = Obb::new(Vec3::new(0.0, 1.9, 0.0), Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0));
        let s = sat(&a, &b, None).expect("overlap");
        // The shallow axis (depth 0.1) is a y-face axis.
        assert!(matches!(s.class, SatClass::FaceA(1) | SatClass::FaceB(1)), "class {:?}", s.class);
        assert!((s.depth - 0.1).abs() < 1e-4, "min depth {}", s.depth);
        assert!(s.axis.y.abs() > 0.99, "axis along y: {:?}", s.axis);
    }

    /// Sutherland-Hodgman clip + reduction never yields more than 4 points.
    #[test]
    fn clip_and_reduce_caps_at_four_points() {
        let (ac, ar, ah) = unit_box(Vec3::ZERO);
        // A slightly tilted box overlapping A's top face → a clipped quad.
        let half = 0.05_f32;
        let tilt = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let bc = Vec3::new(0.0, 1.9, 0.0);
        let c = box_box_contact(A, B, ac, ar, ah, bc, tilt, Vec3::new(1.0, 1.0, 1.0), None)
            .expect("tilted overlap must contact");
        assert!(c.manifold.count <= 4, "reduction caps at 4, got {}", c.manifold.count);
    }

    /// The ≤4-point reduction is deterministic: the same geometry yields the same
    /// points (same positions, same feature ids) across repeated calls.
    #[test]
    fn reduction_is_deterministic() {
        let (ac, ar, ah) = unit_box(Vec3::ZERO);
        let (bc, br, bh) = unit_box(Vec3::new(0.07, 1.9, -0.03));
        let run = || {
            box_box_contact(A, B, ac, ar, ah, bc, br, bh, None)
                .expect("overlap")
                .manifold
        };
        let m1 = run();
        let m2 = run();
        assert_eq!(m1.count, m2.count, "count must be deterministic");
        for i in 0..m1.count as usize {
            assert_eq!(
                m1.points[i].pos_bits(),
                m2.points[i].pos_bits(),
                "point {i} position must be bit-identical"
            );
            assert_eq!(
                m1.points[i].feature_id, m2.points[i].feature_id,
                "point {i} feature id must be deterministic"
            );
        }
    }

    /// A degenerate box with a ZERO in-plane half-extent (a zero-volume face)
    /// must not produce a malformed over-large manifold: the face generator's
    /// zero-side-normal guard rejects the degenerate reference face (returns
    /// `None`) instead of letting `clip_against_plane` keep every vertex.
    ///
    /// `half_extents = (1, 0, 1)` makes the y-faces zero-area; SAT picks one of
    /// them as the reference (shallowest), whose side planes are degenerate.
    /// Whatever the SAT outcome, the result must be a well-formed manifold
    /// (`count ≤ MAX_CONTACT_POINTS`) or `None` — never a blown-up point set.
    #[test]
    fn degenerate_zero_extent_box_no_malformed_manifold() {
        let (ac, ar, _) = unit_box(Vec3::ZERO);
        let a_half = Vec3::new(1.0, 1.0, 1.0);
        // B is a flat (zero y half-extent) box overlapping A's top region.
        let bc = Vec3::new(0.0, 0.95, 0.0);
        let b_half = Vec3::new(1.0, 0.0, 1.0);
        let result = box_box_contact(A, B, ac, ar, a_half, bc, Quat::IDENTITY, b_half, None);
        if let Some(c) = result {
            assert!(
                (c.manifold.count as usize) <= crate::math::MAX_CONTACT_POINTS,
                "degenerate box produced a malformed manifold: count {}",
                c.manifold.count
            );
        }
        // Either path (None or a capped manifold) is acceptable; the invariant is
        // simply that a degenerate face never yields an over-large point set.
    }

    /// A near-parallel resting pair keeps the SAME feature ids across a tiny
    /// perturbation when last frame's axis is fed back (the hysteresis guard).
    #[test]
    fn feature_id_stable_under_small_perturbation() {
        let (ac, ar, ah) = unit_box(Vec3::ZERO);
        let half = 0.001_f32; // a 0.002 rad tilt — FP-noise scale.
        let tilt = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let bc = Vec3::new(0.0, 1.9, 0.0);

        // Frame 1: cold (no last axis).
        let c1 = box_box_contact(A, B, ac, ar, ah, bc, Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0), None)
            .expect("overlap");
        // Frame 2: a tiny tilt, fed last frame's axis (hysteresis active).
        let c2 = box_box_contact(
            A,
            B,
            ac,
            ar,
            ah,
            bc,
            tilt,
            Vec3::new(1.0, 1.0, 1.0),
            Some(c1.reference_axis),
        )
        .expect("overlap");
        // The chosen reference axis must not flip under the noise-scale tilt.
        assert_eq!(
            c1.reference_axis, c2.reference_axis,
            "reference axis flickered under a noise-scale perturbation"
        );
    }

    impl ContactPoint {
        /// The bit pattern of the anchor positions (for bit-exact determinism
        /// assertions in tests).
        fn pos_bits(&self) -> (u32, u32, u32, u32, u32, u32) {
            (
                self.anchor_a.x.to_bits(),
                self.anchor_a.y.to_bits(),
                self.anchor_a.z.to_bits(),
                self.anchor_b.x.to_bits(),
                self.anchor_b.y.to_bits(),
                self.anchor_b.z.to_bits(),
            )
        }
    }
}
