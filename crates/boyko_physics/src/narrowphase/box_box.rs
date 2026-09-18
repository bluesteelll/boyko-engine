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
//!    Every emitted vertex carries a feature id built from the features that
//!    created it, so no two points of one manifold share a warm-start key (A7a —
//!    see [`clip_against_plane`] and
//!    [`feature_face_clip`](super::feature_face_clip)).
//! 3. **Edge-edge** (min axis is a cross product): a single contact at the closest
//!    points of the two contacting edges.
//! 4. **Deterministic ≤4-point reduction**: keep the deepest point plus the three
//!    that maximize the contact-patch spread, ties broken by the lowest
//!    [`ClipVertex::tie_ord`] (the pre-A7a ordinal, retained so the reduction's
//!    order is unchanged by the relabelling) — a pure function of the clipped
//!    polygon, so the selection is reproducible (no FP-tie nondeterminism).
//! 5. **Reference-axis hysteresis**: bias toward last frame's reference axis to
//!    stop the min axis (hence the feature ids) from flickering under FP noise on a
//!    near-parallel resting stack.
//!
//! ZERO `unsafe`, no heap allocation (fixed-size stack buffers), deterministic.

use crate::manifold::{BodyIndex, ContactPoint, Manifold};
use crate::math::{Mat3, Quat, Vec3};

use super::{feature_edge_edge, feature_face_clip, feature_face_face};

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

/// The `in_edge` label of an edge a clip pass created: `PLANE_EDGE_BASE + plane`.
/// The gap at `4..8` is deliberate — bit 3 alone says "this edge lies in a
/// reference side plane", which is what a reader of a dumped id needs.
const PLANE_EDGE_BASE: u8 = 8;

/// A clipped contact vertex carried through Sutherland-Hodgman (P2 W4 / A7a).
#[derive(Clone, Copy)]
struct ClipVertex {
    /// World position.
    pos: Vec3,
    /// TODAY'S ordinal, retained ONLY so [`reduce_points`]' tie-breaks stay
    /// bit-identical to the committed behaviour. NOT an identity — for an
    /// intersection it is still `min(prev, cur)`, which is exactly why it cannot
    /// be a warm key (A7a). Never read for a feature id.
    tie_ord: u8,
    /// Identity of the polygon edge ENTERING this vertex, in a 4-bit edge space:
    /// `0..4` = the incident face's ring edge `k` (ring position `k` → ring
    /// position `(k + 1) % 4` — positions in the face's 4-vertex ring, not the
    /// box corner indices `0..8` that `tie_ord` holds); `PLANE_EDGE_BASE + p` =
    /// the edge a clip against reference side plane `p` created. It names the
    /// edge an intersection is cut ON, so a vertex the clip creates can be named
    /// by its two parent features.
    in_edge: u8,
    /// The feature id this vertex carries into the manifold — injective over one
    /// manifold's points (see [`feature_face_clip`]).
    feature_id: u32,
}

/// Clips the polygon `poly` (`len` vertices) against the half-space `{ x : (x −
/// plane_point) · plane_normal ≤ 0 }` (keep the side the normal points AWAY from),
/// writing the result into `out` and returning its length (Sutherland-Hodgman, P2
/// W4). At most `len + 1` vertices are produced.
///
/// `ref_face` and `plane` (the reference side-plane index `0..4`) name the cut,
/// so every emitted vertex carries an identity rather than an inherited ordinal.
/// The rule is keyed by EMISSION, not by control-flow case, because the entering
/// branch emits TWO vertices:
///
/// | case | emission | vertex | `in_edge` | `feature_id` |
/// |---|---|---|---|---|
/// | entering | 1 of 2 | the intersection | `PLANE_EDGE_BASE + plane` | `feature_face_clip(ref_face, cur.in_edge, plane)` |
/// | entering | 2 of 2 | `cur` | unchanged | unchanged |
/// | inside | 1 of 1 | `cur` | unchanged | unchanged |
/// | leaving | 1 of 1 | the intersection | `cur.in_edge` | `feature_face_clip(ref_face, cur.in_edge, plane)` |
///
/// Both intersections lie on the edge `prev → cur`, whose label is `cur.in_edge`
/// — that is the edge they are cut on, hence the id's second feature. Their own
/// `in_edge` differs: the entering intersection is reached along the new boundary
/// segment lying IN `plane`, while the leaving one is still reached along the
/// original edge.
fn clip_against_plane(
    poly: &[ClipVertex],
    plane_point: Vec3,
    plane_normal: Vec3,
    ref_face: u32,
    plane: u32,
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
                    tie_ord: prev.tie_ord.min(cur.tie_ord),
                    in_edge: PLANE_EDGE_BASE + plane as u8,
                    feature_id: feature_face_clip(ref_face, cur.in_edge as u32, plane),
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
                tie_ord: prev.tie_ord.min(cur.tie_ord),
                in_edge: cur.in_edge,
                feature_id: feature_face_clip(ref_face, cur.in_edge as u32, plane),
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
    /// [`ClipVertex::tie_ord`] — the reduction's tie-break key, and nothing else.
    /// It is carried SEPARATELY from `feature_id` so that, for a given clipped
    /// point set, A7a only relabels: the reduction's induced order is a function
    /// of this field alone, so it is byte-identical to the behaviour before A7a.
    tie_ord: usize,
    /// The point's warm-start identity, carried straight into the manifold.
    feature_id: u32,
}

/// Reduces a clipped point set to at most 4 contacts: the DEEPEST point plus the
/// up-to-3 that maximize the contact-patch spread, ties broken by the LOWEST
/// [`ScoredPoint::tie_ord`] — a pure function of the input (P2 W4).
///
/// The tie-break reads `tie_ord` and NEVER `feature_id`: `tie_ord` is the
/// pre-A7a ordinal, so for a GIVEN clipped point set the induced order, and so
/// the kept points, are byte-identical to the behaviour before the ids became
/// injective. Reading `feature_id` here would re-order the reduction, because a
/// clipped vertex's id (bit 13 set) sorts above every corner's.
///
/// That is all the split preserves. The ids themselves are new wherever the old
/// ones collided, so warm-start keys, seeds and impulses change there, and with
/// them the trajectory, every later clipped point set, the points this function
/// keeps from it, and every downstream number. Measured on a resting height-15
/// pile at step 600: 12817 live contact points before A7a, 14605 after.
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

    // 1) The deepest point (lowest separation); ties → lowest tie_ord.
    let mut deepest = 0usize;
    for i in 1..n {
        let p = points[i];
        let d = points[deepest];
        if p.separation < d.separation
            || (p.separation == d.separation && p.tie_ord < d.tie_ord)
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
        if d2 > far_d2 || (d2 == far_d2 && points[i].tie_ord < cur.tie_ord) {
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
                        && points[i].tie_ord < points[best_pos_i].tie_ord)
                {
                    best_pos = area;
                    best_pos_i = i;
                }
            } else if area > best_neg
                || (area == best_neg
                    && best_neg_i != usize::MAX
                    && points[i].tie_ord < points[best_neg_i].tie_ord)
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

    // Seed the clip polygon with the incident face. An original corner keeps the
    // id it has always had — only vertices the clip CREATES are relabelled (A7a).
    // `face_vertices` winds the ring, so the edge ENTERING ring position `i` is
    // ring edge `i - 1`.
    let ref_face_id = ref_face_idx as u32;
    let mut buf_a = [ClipVertex {
        pos: Vec3::ZERO,
        tie_ord: 0,
        in_edge: 0,
        feature_id: 0,
    }; 8];
    let mut buf_b = buf_a;
    let mut poly_len = 4usize;
    for (i, &(pos, idx)) in inc_face.iter().enumerate() {
        buf_a[i] = ClipVertex {
            pos,
            tie_ord: idx as u8,
            in_edge: ((i + 3) % 4) as u8,
            feature_id: feature_face_face(ref_face_id, idx as u32),
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
        let new_len =
            clip_against_plane(&src[..poly_len], edge_mid, side_normal, ref_face_id, e as u32, dst);
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
        tie_ord: 0,
        feature_id: 0,
    }; 8];
    let mut scored_len = 0usize;
    for &cv in &src[..poly_len] {
        let separation = (cv.pos - ref_face_center).dot(ref_normal);
        if separation <= 0.0 {
            scored[scored_len] = ScoredPoint {
                pos: cv.pos,
                separation,
                tie_ord: cv.tie_ord as usize,
                feature_id: cv.feature_id,
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
        tie_ord: 0,
        feature_id: 0,
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
            feature_id: p.feature_id,
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

    /// `(ref_face, cut_edge, plane)` of a CLIPPED face-face feature id, or `None` for
    /// every other id. Every decode is re-encoded through [`feature_face_clip`] and must
    /// reproduce the id, so the A7-N1 floors that read it cannot drift from the layout.
    fn clip_fields(id: u32) -> Option<(u32, u32, u32)> {
        if id & super::super::TAG_NON_FACE != 0 || id & super::super::TAG_FACE_CLIP == 0 {
            return None;
        }
        let fields = ((id >> 6) & 0x7, (id >> 2) & 0xF, id & 0x3);
        assert_eq!(
            feature_face_clip(fields.0, fields.1, fields.2),
            id,
            "clip id {id:#x} does not re-encode to itself: `clip_fields` no longer matches \
             `feature_face_clip`'s layout"
        );
        Some(fields)
    }

    /// Scans one manifold's ids for A7-N1: every repeated id is pushed onto `duplicates`,
    /// described by `at`. Returns `(shared_edge, shared_plane)`: whether two CLIPPED points
    /// share `(ref_face, cut_edge)` — one polygon edge cut by two side planes, which only the
    /// `plane` field tells apart — and whether two share `(ref_face, plane)` — two edges cut
    /// by one plane, which only `cut_edge` tells apart. Both are read from the fields
    /// themselves, so a mutation that drops either field still counts here and reds on the
    /// duplicate instead.
    fn scan_ids(
        ids: &[u32],
        duplicates: &mut Vec<String>,
        at: impl Fn() -> String,
    ) -> (bool, bool) {
        let mut shared_edge = false;
        let mut shared_plane = false;
        for i in 1..ids.len() {
            for j in 0..i {
                if ids[i] == ids[j] {
                    duplicates.push(format!(
                        "{}: duplicate feature id {:#x} at points {j} and {i} (manifold ids \
                         {ids:x?})",
                        at(),
                        ids[i]
                    ));
                }
                if let (Some(a), Some(b)) = (clip_fields(ids[i]), clip_fields(ids[j])) {
                    shared_edge |= a.0 == b.0 && a.1 == b.1;
                    shared_plane |= a.0 == b.0 && a.2 == b.2;
                }
            }
        }
        (shared_edge, shared_plane)
    }

    /// A7-N1: EVERY point of EVERY box-box manifold carries a DISTINCT feature id — so no
    /// two points of one manifold pack the same `warm_start::pack(a, b, feature_id)` key
    /// and none of them loses its seed to the other on the table's open-addressed insert
    /// (A7a).
    ///
    /// A duplicate id is a property of the clip's LABELLING, so it is swept, not sampled,
    /// over two geometries. Each is the one that makes a field of [`feature_face_clip`]
    /// load-bearing, and each has a floor proving it still does:
    ///
    /// * **Offset × yaw.** The incident face slides from a near-full overlap out to the
    ///   pile's own quarter overlap at `(1, 1)`, so two incident edges are cut by ONE side
    ///   plane and only `cut_edge` tells their intersections apart. The yawed rows clip to
    ///   more than 4 candidates and carry the reduction through the relabelling. This
    ///   contains A7-R0's `(1, 1)` case at yaw 0, NOT its other three sign cases — A7-R0
    ///   is the only gate of those and is not redundant with this test.
    /// * **Corner-spanning.** The incident face is centred on the reference face, yawed
    ///   AND pitched, so one incident edge crosses two side planes beside a reference-face
    ///   corner and the pitch keeps both of its intersections among the deepest. Only
    ///   `plane` tells them apart. The first sweep never produces this (0 of its 1200
    ///   manifolds, measured 2026-09-18), so without this sweep a dropped `plane` term
    ///   stayed green here.
    ///
    /// The third field, `ref_face`, is constant over a manifold, so no sweep of this
    /// property can gate it; [`clipped_ids_of_different_reference_faces_never_coincide`]
    /// does.
    #[test]
    fn every_manifold_point_carries_a_distinct_feature_id() {
        // ── Sweep 1: offset × yaw ──
        //
        // 20 × 20 offsets per yaw, from 0.05 to the quarter overlap at 1.0 inclusive. The
        // contact is guaranteed BY CONSTRUCTION: the incident face's centre is the offset,
        // so for an offset in the CLOSED reference face the incident face's inscribed disc
        // (radius 1) overlaps the reference face with positive area — a quarter disc at
        // (1, 1) — and with the 1 mm overlap in y the boxes intersect, so no separating axis
        // exists. Past 1.0 that stops holding: at offset (1.75, 1.75), yaw 0.45, the
        // cross-product axis `a.y × b.x` separates the pair by 2 mm.
        const STEPS: usize = 20;
        const YAWS: [f32; 3] = [0.0, 0.2, 0.45];
        const OFFSET_LO: f32 = 0.05;
        const OFFSET_HI: f32 = 1.0;
        const OFFSET_SPAN: f32 = OFFSET_HI - OFFSET_LO;

        let mut duplicates: Vec<String> = Vec::new();
        let mut swept = 0usize;
        let mut four_point = 0usize;
        let mut one_plane_two_edges = 0usize;
        for &yaw in &YAWS {
            let half_angle = yaw * 0.5;
            let rot = Quat::new(0.0, half_angle.sin(), 0.0, half_angle.cos());
            for ix in 0..STEPS {
                for iz in 0..STEPS {
                    let dx = OFFSET_LO + OFFSET_SPAN * ix as f32 / (STEPS - 1) as f32;
                    let dz = OFFSET_LO + OFFSET_SPAN * iz as f32 / (STEPS - 1) as f32;
                    let contact = box_box_contact(
                        A,
                        B,
                        Vec3::ZERO,
                        Quat::IDENTITY,
                        Vec3::new(1.0, 1.0, 1.0),
                        Vec3::new(dx, 2.0 - 1.0e-3, dz),
                        rot,
                        Vec3::new(1.0, 1.0, 1.0),
                        None,
                    )
                    .unwrap_or_else(|| {
                        panic!(
                            "construction: offset ({dx}, {dz}) yaw {yaw} puts the incident face's \
                             centre on the reference face and overlaps by 1 mm in y, so the \
                             boxes intersect and no separating axis exists"
                        )
                    });
                    let m = contact.manifold;
                    let count = usize::from(m.count);
                    swept += 1;
                    if count == 4 {
                        four_point += 1;
                    }
                    let ids: Vec<u32> =
                        m.points[..count].iter().map(|p| p.feature_id).collect();
                    let (_, shared_plane) = scan_ids(&ids, &mut duplicates, || {
                        format!("offset ({dx}, {dz}) yaw {yaw}")
                    });
                    one_plane_two_edges += usize::from(shared_plane);
                }
            }
        }

        // ── Sweep 2: corner-spanning (yaw × pitch × offset) ──
        //
        // B is turned by yaw about y, then pitched about x, and placed so the centre of its
        // incident (local -y) face sits 1 mm below A's top face at (ox, oz). That point is
        // on B's surface and strictly inside A, so the boxes intersect BY CONSTRUCTION and
        // no separating axis exists. Offsets stay near the centre so every incident edge
        // runs past a reference-face corner.
        const CORNER_YAWS: usize = 8; // 0.05 ..= 0.40
        const CORNER_PITCHES: usize = 5; // 0.01 ..= 0.05
        const CORNER_OFFSETS: [f32; 3] = [-0.2, 0.0, 0.2];
        const DIP: f32 = 1.0e-3;

        let mut corner_swept = 0usize;
        let mut one_edge_two_planes = 0usize;
        for iy in 0..CORNER_YAWS {
            let yaw = 0.05 * (iy + 1) as f32;
            let turn = Quat::new(0.0, (yaw * 0.5).sin(), 0.0, (yaw * 0.5).cos());
            for ip in 0..CORNER_PITCHES {
                let pitch = 0.01 * (ip + 1) as f32;
                let tilt = Quat::new((pitch * 0.5).sin(), 0.0, 0.0, (pitch * 0.5).cos());
                let rot = turn * tilt;
                let incident_centre = rot.rotate(Vec3::new(0.0, -1.0, 0.0));
                for &ox in &CORNER_OFFSETS {
                    for &oz in &CORNER_OFFSETS {
                        let centre = Vec3::new(ox, 1.0 - DIP, oz) - incident_centre;
                        let contact = box_box_contact(
                            A,
                            B,
                            Vec3::ZERO,
                            Quat::IDENTITY,
                            Vec3::new(1.0, 1.0, 1.0),
                            centre,
                            rot,
                            Vec3::new(1.0, 1.0, 1.0),
                            None,
                        )
                        .unwrap_or_else(|| {
                            panic!(
                                "construction: yaw {yaw} pitch {pitch} offset ({ox}, {oz}) puts a \
                                 point of B's incident face 1 mm inside A, so the boxes intersect \
                                 and no separating axis exists"
                            )
                        });
                        let m = contact.manifold;
                        let ids: Vec<u32> = m.points[..usize::from(m.count)]
                            .iter()
                            .map(|p| p.feature_id)
                            .collect();
                        corner_swept += 1;
                        let (shared_edge, _) = scan_ids(&ids, &mut duplicates, || {
                            format!("yaw {yaw} pitch {pitch} offset ({ox}, {oz})")
                        });
                        one_edge_two_planes += usize::from(shared_edge);
                    }
                }
            }
        }

        assert_eq!(
            (swept, corner_swept),
            (
                STEPS * STEPS * YAWS.len(),
                CORNER_YAWS * CORNER_PITCHES * CORNER_OFFSETS.len() * CORNER_OFFSETS.len()
            ),
            "construction: both sweeps must visit every cell"
        );
        // The property first, so a dropped field reds with the duplicate it makes rather
        // than with a floor below.
        assert!(duplicates.is_empty(), "{}", duplicates.join("; "));

        // Anti-vacuity, one floor per sweep, each counting the configuration the sweep
        // exists for. A manifold of one point is distinct for free: the axis-aligned rows
        // alone clip every offset of sweep 1 to a proper rectangle, so one row's worth of
        // 4-point manifolds is the floor. The field floors are one yaw row's worth (sweep
        // 1) and one per yaw × pitch cell (sweep 2), against 1151 of 1200 and 215 of 360
        // measured (2026-09-18) — low enough to survive a narrowphase change that merely
        // moves which points the reduction keeps, and far above the 0 of a sweep that no
        // longer reaches its configuration.
        assert!(
            four_point >= STEPS * STEPS,
            "sweep 1 produced only {four_point} four-point manifolds out of {swept}: it is no \
             longer measuring the clipped face path the duplicate ids live on"
        );
        assert!(
            one_plane_two_edges >= STEPS * STEPS,
            "sweep 1: only {one_plane_two_edges} of {swept} manifolds keep two intersections cut \
             by one side plane on two edges, so it no longer shows that `cut_edge` is what \
             separates them"
        );
        assert!(
            one_edge_two_planes >= CORNER_YAWS * CORNER_PITCHES,
            "sweep 2: only {one_edge_two_planes} of {corner_swept} manifolds keep two \
             intersections of one edge cut by two side planes, so it no longer shows that \
             `plane` is what separates them"
        );
    }

    /// A7-N1 (third arm): a CLIPPED point's id never equals one from another reference
    /// face of the SAME pair — so when a pair's reference face changes, its old seeds miss
    /// instead of being handed to unrelated points. The corner ids get the same guarantee
    /// from [`feature_face_face`]'s own `ref_face` field.
    ///
    /// This is the field of [`feature_face_clip`] that A7-N1's within-manifold sweeps cannot
    /// gate: `ref_face` is constant over one manifold. So one pair `(A, B)` is put in face
    /// contact across A's `+x`, `+y` and `+z` faces with B turned about the contact normal
    /// only, which puts the reference face — whichever box owns it — on a different axis in
    /// each group BY CONSTRUCTION. The groups' clipped ids must be pairwise disjoint, and
    /// the anti-vacuity guard requires a `(cut_edge, plane)` label that occurs under two
    /// reference faces: on those ids `ref_face` is the only field left to tell them apart.
    #[test]
    fn clipped_ids_of_different_reference_faces_never_coincide() {
        const STEPS: usize = 5; // offsets 0.05 ..= 0.85 on the contact face
        const TURNS: [f32; 3] = [0.0, 0.2, 0.45];

        let mut groups: Vec<Vec<u32>> = Vec::with_capacity(3);
        for axis in 0..3 {
            let mut clipped: Vec<u32> = Vec::new();
            for &turn in &TURNS {
                let (s, c) = (turn * 0.5).sin_cos();
                for iu in 0..STEPS {
                    for iv in 0..STEPS {
                        let u = 0.05 + 0.2 * iu as f32;
                        let v = 0.05 + 0.2 * iv as f32;
                        // B's face along `axis` stays exactly on `axis` under a turn about it,
                        // and the incident face's centre sits on A's face 1 mm deep, so the
                        // contact holds by A7-N1 sweep 1's argument.
                        let (centre, rot) = match axis {
                            0 => (Vec3::new(2.0 - 1.0e-3, u, v), Quat::new(s, 0.0, 0.0, c)),
                            1 => (Vec3::new(u, 2.0 - 1.0e-3, v), Quat::new(0.0, s, 0.0, c)),
                            _ => (Vec3::new(u, v, 2.0 - 1.0e-3), Quat::new(0.0, 0.0, s, c)),
                        };
                        let m = box_box_contact(
                            A,
                            B,
                            Vec3::ZERO,
                            Quat::IDENTITY,
                            Vec3::new(1.0, 1.0, 1.0),
                            centre,
                            rot,
                            Vec3::new(1.0, 1.0, 1.0),
                            None,
                        )
                        .unwrap_or_else(|| {
                            panic!(
                                "construction: axis {axis} offset ({u}, {v}) turn {turn} overlaps \
                                 A's face by 1 mm with the incident face's centre on it"
                            )
                        })
                        .manifold;
                        for p in &m.points[..usize::from(m.count)] {
                            if clip_fields(p.feature_id).is_none() {
                                continue;
                            }
                            let n = [m.normal.x, m.normal.y, m.normal.z];
                            assert!(
                                n[axis] > 0.99,
                                "construction: a clipped face contact across A's +axis-{axis} \
                                 face must carry that axis as its normal, or its reference face \
                                 is not on axis {axis}; normal {:?}",
                                m.normal
                            );
                            if !clipped.contains(&p.feature_id) {
                                clipped.push(p.feature_id);
                            }
                        }
                    }
                }
            }
            assert!(
                !clipped.is_empty(),
                "construction: the face contacts across axis {axis} produced no clipped point, \
                 so there is nothing to compare"
            );
            groups.push(clipped);
        }

        let mut shared: Vec<String> = Vec::new();
        for i in 1..groups.len() {
            for j in 0..i {
                for &id in &groups[i] {
                    if groups[j].contains(&id) {
                        shared.push(format!(
                            "{id:#x} under the axis-{j} and axis-{i} reference faces"
                        ));
                    }
                }
            }
        }
        assert!(
            shared.is_empty(),
            "clipped feature ids repeat across reference faces of one pair: {}",
            shared.join(", ")
        );

        let label = |id: u32| {
            let (_, cut_edge, plane) =
                clip_fields(id).expect("invariant: every grouped id is a clip id");
            (cut_edge, plane)
        };
        let label_under_two_faces = (1..groups.len()).any(|i| {
            (0..i).any(|j| {
                groups[i]
                    .iter()
                    .any(|&a| groups[j].iter().any(|&b| label(a) == label(b)))
            })
        });
        assert!(
            label_under_two_faces,
            "no (cut_edge, plane) label occurs under two reference faces, so `ref_face` \
             separates nothing here and this arm gates nothing; groups {groups:x?}"
        );
    }

    /// A7-N1 (second arm): every tie-break in [`reduce_points`] reads `tie_ord` —
    /// the pre-A7a ordinal — and NEVER `feature_id`, so the injective ids are a
    /// pure relabelling and the reduction's kept set and order are byte-identical
    /// to the committed behaviour.
    ///
    /// The fixture is five coplanar candidates at one separation, so the ordinal
    /// decides at every stage (deepest, farthest, and the two spread picks), with
    /// the ordinal order and the feature-id order deliberately inverse. Reading
    /// `feature_id` instead picks a different deepest point — every clipped id has
    /// bit 13 set and therefore sorts above every corner id — and the whole kept
    /// set follows it.
    #[test]
    fn reduction_tie_break_reads_the_ordinal_not_the_feature_id() {
        let point = |x: f32, z: f32, tie_ord: usize, feature_id: u32| ScoredPoint {
            pos: Vec3::new(x, 0.0, z),
            separation: -0.5,
            tie_ord,
            feature_id,
        };
        let points = [
            point(0.0, 0.0, 7, 0x2001),
            point(2.0, 0.0, 5, 0x2002),
            point(0.0, 2.0, 3, 0x2003),
            point(2.0, 2.0, 1, 0x2004),
            point(1.0, 1.0, 0, 0x2005),
        ];
        assert!(
            points.iter().all(|p| p.separation == points[0].separation),
            "construction: one separation for all five, or the deepest pick never reaches a \
             tie-break and the test measures nothing"
        );
        let by_ord = (0..points.len())
            .min_by_key(|&i| points[i].tie_ord)
            .expect("invariant: the fixture is non-empty");
        let by_id = (0..points.len())
            .min_by_key(|&i| points[i].feature_id)
            .expect("invariant: the fixture is non-empty");
        assert_ne!(
            by_ord, by_id,
            "construction: the two keys must disagree on this fixture, or a tie-break reading the \
             wrong one would be invisible"
        );

        let mut out = [point(0.0, 0.0, 0, 0); 4];
        let kept = reduce_points(&points, Vec3::new(0.0, 1.0, 0.0), &mut out);
        let kept_ord: Vec<usize> = out[..kept].iter().map(|p| p.tie_ord).collect();
        let kept_ids: Vec<u32> = out[..kept].iter().map(|p| p.feature_id).collect();
        assert_eq!(
            kept_ord,
            vec![0usize, 1, 5, 3],
            "reduction order changed: kept ordinals {kept_ord:?} (feature ids {kept_ids:x?}); \
             committed behaviour keeps ordinals [0, 1, 5, 3]"
        );
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
