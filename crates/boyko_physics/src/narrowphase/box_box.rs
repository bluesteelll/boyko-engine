//! Box-vs-box (OBB-OBB) contact generation (P2 W4) — the heavy convex generator.
//!
//! The standard separating-axis (SAT) + reference-face clip + bounded-point
//! reduction pipeline, with the feature-id stability machinery a resting box
//! stack needs (P2 W3 precondition):
//!
//! 1. **15-axis SAT**: the 3 face axes of each box plus the 9 edge-edge cross
//!    products. A positive gap on ANY axis means the boxes are separated (no
//!    contact). Otherwise the shallowest face axis and the shallowest edge axis are
//!    found separately, and the face is the contact axis unless the edge is
//!    shallower than the face's REALIZED clipped patch by more than
//!    [`FACE_AXIS_PREFERENCE`], or the face realizes no patch at all — Box3D's
//!    rule (A7b), with two older differences at the face path's boundary (see the
//!    constant). The patch is built only for a pair whose SAT answer is the edge.
//! 2. **Reference-face clip** (the contact axis is a face axis): the reference face is the
//!    one on that axis; the incident face is the other box's most anti-parallel
//!    face; the incident polygon is Sutherland-Hodgman-clipped against the
//!    reference face's 4 side planes, and points below the reference face are kept.
//!    Every emitted vertex carries a feature id built from the features that
//!    created it, so no two points of one manifold share a warm-start key (A7a —
//!    see [`clip_against_plane`] and
//!    [`feature_face_clip`](super::feature_face_clip)).
//! 3. **Edge-edge** (the contact axis is a cross product, per item 1, or the chosen
//!    face realizes no patch and no best-face patch was built — see item 5): a single
//!    contact at the closest points of the two contacting edges.
//! 4. **Deterministic ≤4-point reduction**: keep the deepest point plus the three
//!    that maximize the contact-patch spread, ties broken by the lowest
//!    [`ClipVertex::tie_ord`] (the pre-A7a ordinal, retained so the reduction's
//!    order is unchanged by the relabelling) — a pure function of the clipped
//!    polygon, so the selection is reproducible (no FP-tie nondeterminism).
//! 5. **Reference-axis hysteresis**: bias toward last frame's reference axis to
//!    stop the contact axis (hence the feature ids) from flickering under FP noise
//!    on a near-parallel resting stack. Face↔face and edge↔edge holds are as
//!    before; an edge hint never holds a pair whose chosen axis is a face (A7b). A
//!    held face that realizes no patch yields to the best face's patch when item 1
//!    already built it, as Box3D's full query does when a cached feature fails.
//!
//! Whether a pair has a manifold is a function of the two poses alone: past an
//! overlapping SAT, every path ends in a face patch or an edge contact, except a
//! reference face with a zero in-plane extent (no contact) — and "no edge axis at all",
//! which two orthonormal frames never produce. The hint picks which contact, never
//! whether there is one.
//!
//! # The exact fast path (L9a, `levers/L9-contact-reuse/02-DESIGN-REV1.md`)
//!
//! Two changes that move no bit (Lemma L9-L2):
//!
//! * **The SAT returns at the first separating axis** in canonical order ([`sat`]). The
//!   pair was separated iff ANY axis had a negative depth and [`eval_axis`] is pure, so
//!   stopping at the first one changes no answer; an overlapping pair still evaluates all
//!   fifteen axes, in the same order, as before. The axis it stopped on is reported
//!   ([`BoxBoxOutcome::Separated`]), which is what commit C2 carries as the pair's cached
//!   separating axis.
//! * **The box frame is an input** ([`Obb::from_frame`]): the narrowphase reads each box
//!   row's axes from the per-step frame column (`narrowphase/reuse.rs`) instead of
//!   converting the row's quaternion per pair. [`Obb::new`] is the same computation
//!   through [`RowFrame::axes_of`], so both give the same bits.
//!
//! The crate's narrowphase enters through [`box_box_classify`]; [`box_box_contact`] keeps
//! its signature as a wrapper over it. The pre-L9 bodies of `Obb::new`, the SAT and
//! `box_box_contact` stay as the test oracle `pre_l9` (gate G-L9a-1).
//!
//! # Contact reuse (L9b, `narrowphase/reuse.rs`)
//!
//! Two small entries serve the reuse records, and neither changes a contact: [`contact_feature`]
//! names the feature a contact was built on — the reference face `face_contact` picked, re-derived
//! from the contact's axis and normal with the same [`most_aligned_face`], or the edge pair — and
//! [`refresh_edge`] re-evaluates an edge record's axis exactly as [`sat`] evaluates its candidate
//! of that index and builds today's edge contact on it.
//!
//! ZERO `unsafe`, no heap allocation (fixed-size stack buffers), deterministic.

use crate::manifold::{BodyIndex, ContactPoint, Manifold};
use crate::math::{Quat, Vec3};

use super::reuse::RowFrame;
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

/// How much shallower an edge-edge axis must be than the best face axis's REALIZED
/// contact patch before it replaces the face (A7b). `SAT_EPS` keeps its role for ties
/// WITHIN a class.
///
/// **Box3D's rule, as its live `convex_manifold.c` ships it** (`B3_LINEAR_SLOP = 0.005`
/// m): it always builds the face contact first and records `clipSeparation`, the minimum
/// separation over the clipped face points; it switches to the edge contact only if
/// `edgeSeparation > clipSeparation + linearSlop`, or if the face contact has no points.
/// Here that reads `edge.depth < patch_depth − FACE_AXIS_PREFERENCE`, with `patch_depth`
/// the deepest kept point's penetration (the reduction always keeps the deepest point).
///
/// Two differences from Box3D predate S5 and remain, both in what counts as a face patch.
/// Box3D treats a clip left with fewer than 3 vertices as no face contact
/// (`convex_manifold.c:1092-1096`); here only an EMPTY clip does. Box3D keeps speculative
/// points, clipped points above the reference face up to its speculative distance (:1128);
/// here only points with `separation <= 0` are kept. So a clip that degenerates to a
/// segment — diagonal neighbours touching along an edge — is a 1-2 point face patch here
/// and an edge contact in Box3D; and a touch whose clipped points all lie just above the
/// reference face is an empty patch here (the edge fallback), where Box3D keeps a
/// speculative face patch if the clip holds three or more vertices.
///
/// **Why the patch and not the face axis's SAT depth.** Every clipped point at depth `s`
/// lies in both boxes, and the point above it on the reference face lies in the reference
/// box, so the overlap along any axis at angle `φ` from the face normal is at least
/// `s·cos φ`. An edge axis that nearly duplicates the face normal — `A.x × B.z` on a nearly
/// aligned face pair — can therefore never read more than `s·(1 − cos φ)` shallower than
/// the patch, whatever the lever arm, and is never taken on a face patch. The SAT face
/// depth has no such bound: it is set by the incident box's deepest vertex even when that
/// vertex overhangs the reference face, so it differs from the duplicate edge axis by about
/// `θ·Δc_lateral` for a relative tilt `θ`. On a resting pile a 4 → 1 support flip moves the
/// contact normal by ~1e-5..2e-5 rad, which is the RELATIVE tilt that sets that difference
/// (A7 C2 probe, 2026-09-18, a height-7 pile's support flips at steps 601-602, msvc
/// release: e.g. pair (11, 53)'s normal `x` 1.34e-4 → 1.18e-4 across its 4 → 1 flip; the
/// 1e-4..2e-4 rad in the same dump are the ABSOLUTE tilt from world `y`, which sets no
/// depth difference). Over a quarter overlap's `|Δc_lateral|` ≈ 1.4 m that is
/// ~1.4e-5..2.8e-5 m — beyond the old `SAT_EPS` face-preference window, so the pair fell
/// to the 1-point edge path on jitter (A7b).
///
/// **What the value buys, and what it costs.** On a face patch the window only has to
/// absorb FP: `s·cos φ ≤ edge.depth` holds exactly in real arithmetic, and the two sides
/// are computed along different paths (projection radii against clipped points), each off
/// by about an ulp of the centre coordinates — ~2e-6 m at the pile's 30 m. The value
/// itself matters for GENUINE edge-edge contacts (crossed edges, `φ` far from 0): a face
/// patch up to this much deeper than the edge axis still wins, which keeps a contact on the
/// multi-point face path through the tipping transition instead of flickering to one
/// point. The face path's per-point separations are realized depths — each point is
/// pushed out by its own penetration along the face normal, never by the SAT face depth —
/// so a larger value does not overshoot; it resolves more crossed-edge contacts along a
/// face normal that is `φ` off the minimum-translation axis, through a push-out up to this
/// much deeper than the edge axis requires.
///
/// **On a resting pile the value does nothing.** Over steps 600-3000 of A7-R1's height-15
/// pile (msvc release, 2026-09-18) the SAT answered an edge 3 531 050 times, and the
/// comparison against the realized patch chose that edge 0 times; the edge was taken 264
/// times, every one of them a face that realized no patch. What moved the pile off the edge
/// path is comparing against the realized patch at all, not this number.
///
/// Metres, like the rest of the engine (gravity −9.81, `CREEP_BOUND_M`); Box2D scales its
/// slop by `b2_lengthUnitsPerMeter`, so if a length unit is ever introduced this constant
/// joins it.
const FACE_AXIS_PREFERENCE: f32 = 0.005;

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
pub(crate) struct Obb {
    /// World center.
    pub(crate) center: Vec3,
    /// World-space unit axis directions (local x, y, z).
    pub(crate) axes: [Vec3; 3],
    /// Half-extents along each local axis.
    pub(crate) half: [f32; 3],
}

impl Obb {
    /// Builds the world OBB from a body's center, orientation, and local
    /// half-extents, with the axes of [`RowFrame::axes_of`]`(rotation)`.
    #[inline]
    pub(crate) fn new(center: Vec3, rotation: Quat, half_extents: Vec3) -> Self {
        Self::from_axes(center, RowFrame::axes_of(rotation), half_extents)
    }

    /// Builds the world OBB from a body's center, its step's orientation frame (L9 D2) and its
    /// local half-extents. Given a frame of `RowFrame::axes_of(rotation)` it is
    /// [`new`](Self::new), bit for bit.
    #[inline]
    pub(crate) fn from_frame(center: Vec3, frame: &RowFrame, half_extents: Vec3) -> Self {
        Self::from_axes(center, frame.axes, half_extents)
    }

    /// Builds the world OBB from a center, world axes and local half-extents.
    #[inline]
    fn from_axes(center: Vec3, axes: [Vec3; 3], half_extents: Vec3) -> Self {
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

/// The classification of a SAT axis.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SatClass {
    /// A face axis of box A (axis index `0..3`).
    FaceA(usize),
    /// A face axis of box B (axis index `0..3`).
    FaceB(usize),
    /// An edge-edge cross product (A-axis `a`, B-axis `b`).
    Edge { a: usize, b: usize },
}

/// The result of the SAT query for an overlapping pair: the shallowest axis of each
/// class, and last frame's axis re-evaluated on this frame's poses. The face-versus-edge
/// choice and the hysteresis are made by [`box_box_classify`], because Box3D's rule compares
/// the edge against the face's REALIZED patch, which only the clip produces (A7b).
#[derive(Clone, Copy, Debug)]
struct SatResult {
    /// The shallowest face axis (canonical indices `0..6`, lower index on a tie within
    /// `SAT_EPS`).
    face: AxisCandidate,
    /// The shallowest edge-edge axis (canonical indices `6..15`, same tie rule), or `None`
    /// when every edge pair is parallel (the degeneracy guard skipped all nine).
    edge: Option<AxisCandidate>,
    /// Last frame's axis for this pair, re-evaluated here, or `None` on a cold contact or
    /// when that axis is degenerate this frame.
    hint: Option<AxisCandidate>,
}

/// One SAT axis candidate, evaluated for overlap.
#[derive(Clone, Copy, Debug)]
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

impl AxisCandidate {
    /// Whether this is an edge-edge cross-product axis (canonical index `6..15`).
    #[inline]
    fn is_edge(&self) -> bool {
        matches!(self.class, SatClass::Edge { .. })
    }
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

/// Why [`sat`] found no overlapping pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SatMiss {
    /// The first axis in canonical order (`0..15`) whose depth is negative: the boxes are
    /// separated on it (L9a (i)).
    Separated(u8),
    /// Every axis overlaps but no face axis survived the degeneracy guard, which only a
    /// degenerate rotation produces: no contact, as before S5.
    NoFaceAxis,
}

/// Whether an evaluated axis separates the boxes. A NaN depth compares false, as it did when
/// every axis was evaluated before the check.
#[inline]
fn separates(cand: Option<AxisCandidate>) -> bool {
    matches!(cand, Some(c) if c.depth < 0.0)
}

/// Whether canonical SAT axis `axis` (`0..15`) still separates the boxes (L9a (ii)): the pair's
/// separating axis of the previous step, evaluated first on this step's boxes.
///
/// The axis is built and evaluated exactly as [`sat`] builds and evaluates its candidate of the
/// same index — the same raw axis (a face column, or `a.axes[ea].cross(b.axes[eb])` for
/// `axis = 6 + 3·ea + eb`) through the same [`eval_axis`], judged by the same [`separates`] — so
/// `true` means the SAT has a negative candidate and reports the pair separated. `false` (the axis
/// overlaps now, or is degenerate) says nothing, and the SAT runs.
#[inline]
pub(crate) fn sep_still_holds(a: &Obb, b: &Obb, axis: u8) -> bool {
    let i = usize::from(axis);
    debug_assert!(i < SAT_AXES, "invariant: a carried separating axis is 0..15");
    let cand = if i < 3 {
        eval_axis(a, b, a.axes[i], SatClass::FaceA(i), i)
    } else if i < 6 {
        eval_axis(a, b, b.axes[i - 3], SatClass::FaceB(i - 3), i)
    } else {
        let (ea, eb) = ((i - 6) / 3, (i - 6) % 3);
        eval_axis(a, b, a.axes[ea].cross(b.axes[eb]), SatClass::Edge { a: ea, b: eb }, i)
    };
    separates(cand)
}

/// Runs the 15-axis SAT and returns the shallowest face axis, the shallowest edge
/// axis and last frame's axis re-evaluated, or why there is no overlap (P2 W4): the first
/// separating axis in canonical order, or no face axis at all.
///
/// `last_axis` is last frame's chosen SAT-axis index (for the same body pair), or
/// `None` on a cold contact; [`box_box_classify`] applies the hysteresis to it.
///
/// **The early exit (L9a (i)).** The axes are evaluated in canonical order and the SAT returns
/// at the first one with `depth < 0`. It used to evaluate all fifteen and then reject on any
/// negative one; the answer is "separated" exactly when some axis is negative either way, and
/// [`eval_axis`] is pure, so the early exit changes no result. An overlapping pair evaluates all
/// fifteen, in the same order, so every candidate below carries today's bits. The returned axis
/// is the FIRST negative one; commit C2 caches it, and any negative axis proves separation.
///
/// The two classes are selected in two passes rather than one loop, because a single
/// ε-window loop whose windows differ by class is order-dependent. Each pass is the
/// pre-S5 in-class rule: a candidate wins only if it is shallower by more than
/// `SAT_EPS`, otherwise the lower canonical index stays.
//
// `clippy::needless_range_loop`: `i` is simultaneously the canonical SAT-axis
// index (stored in the candidate + used for the hysteresis), the `axes[i]`
// selector, and the `SatClass` payload — three roles a bare `enumerate()` over one
// array cannot carry, so the explicit index is the correct, readable form.
#[allow(clippy::needless_range_loop)]
fn sat(a: &Obb, b: &Obb, last_axis: Option<usize>) -> Result<SatResult, SatMiss> {
    // All 15 candidate axes in canonical order: A-face 0..3, B-face 3..6,
    // edge-edge 6..15 (a-major: (a0×b0, a0×b1, a0×b2, a1×b0, …)).
    let mut candidates: [Option<AxisCandidate>; SAT_AXES] = [None; SAT_AXES];
    for i in 0..3 {
        candidates[i] = eval_axis(a, b, a.axes[i], SatClass::FaceA(i), i);
        if separates(candidates[i]) {
            return Err(SatMiss::Separated(i as u8));
        }
    }
    for i in 0..3 {
        candidates[3 + i] = eval_axis(a, b, b.axes[i], SatClass::FaceB(i), 3 + i);
        if separates(candidates[3 + i]) {
            return Err(SatMiss::Separated((3 + i) as u8));
        }
    }
    let mut k = 6;
    for ea in 0..3 {
        for eb in 0..3 {
            let axis = a.axes[ea].cross(b.axes[eb]);
            candidates[k] = eval_axis(a, b, axis, SatClass::Edge { a: ea, b: eb }, k);
            if separates(candidates[k]) {
                return Err(SatMiss::Separated(k as u8));
            }
            k += 1;
        }
    }

    let face = shallowest(&candidates[..6]);
    debug_assert!(
        face.is_some(),
        "invariant: a box's face axes are unit rotation columns (length² = 1), so the \
         degeneracy guard never skips all six"
    );
    // A degenerate rotation leaves no face axis: no contact, as before S5.
    let Some(face) = face else {
        return Err(SatMiss::NoFaceAxis);
    };

    Ok(SatResult {
        face,
        edge: shallowest(&candidates[6..]),
        hint: last_axis.and_then(|idx| candidates.get(idx).copied().flatten()),
    })
}

/// The shallowest candidate of one class, deterministic under FP ties: a candidate
/// replaces the running best only if it is shallower by more than `SAT_EPS`. Candidates
/// arrive in ascending canonical index, so on a tie the LOWER index stays — a pure
/// function of the geometry, never tie-order dependent.
#[inline]
fn shallowest(class: &[Option<AxisCandidate>]) -> Option<AxisCandidate> {
    let mut best: Option<AxisCandidate> = None;
    for cand in class.iter().flatten() {
        best = Some(match best {
            Some(cur) if cand.depth < cur.depth - SAT_EPS => *cand,
            Some(cur) => cur,
            None => *cand,
        });
    }
    best
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
///
/// A wrapper over [`box_box_classify`] on two [`Obb::new`] boxes: the crate's narrowphase
/// calls the classifier directly with the step's frames (L9 D2), and both give the same bits.
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
    match box_box_classify(&a, &b, body_a, body_b, last_axis) {
        BoxBoxOutcome::Contact(c) => Some(c),
        BoxBoxOutcome::Separated(_)
        | BoxBoxOutcome::StillSeparated(_)
        | BoxBoxOutcome::NoContact => None,
    }
}

/// What [`box_box_classify`] found for one box pair (L9a).
pub(crate) enum BoxBoxOutcome {
    /// The boxes touch: the manifold and the SAT axis to persist for the hysteresis.
    Contact(BoxBoxContact),
    /// The SAT separated the boxes on this canonical axis (`0..15`), the first negative one in
    /// canonical order.
    Separated(u8),
    /// The pair's carried separating axis (`0..15`) still separates the boxes, so the SAT did not
    /// run (L9a (ii), [`box_box_classify_carried`]).
    StillSeparated(u8),
    /// No contact for any other reason: no face axis (a degenerate rotation), a degenerate
    /// reference face, or a fallback with no edge axis.
    NoContact,
}

/// [`box_box_classify`] behind the pair's carried separating axis (L9a (ii), D1): when `sep_axis`
/// names an axis that still separates the boxes ([`sep_still_holds`]), the pair is separated and
/// neither the hint nor the SAT is consulted; otherwise the classifier runs on the hint `hint`
/// returns. Contact or no contact is the classifier's answer either way (lemma L9-L2), and the
/// narrowphase collides every box pair through this one function.
#[inline]
pub(crate) fn box_box_classify_carried(
    a: &Obb,
    b: &Obb,
    body_a: BodyIndex,
    body_b: BodyIndex,
    sep_axis: Option<u8>,
    hint: impl FnOnce() -> Option<usize>,
) -> BoxBoxOutcome {
    if let Some(axis) = sep_axis
        && sep_still_holds(a, b, axis)
    {
        return BoxBoxOutcome::StillSeparated(axis);
    }
    box_box_classify(a, b, body_a, body_b, hint())
}

/// Classifies the box pair `(a, b)` and, when they touch, generates its contact (P2 W4; L9a):
/// [`box_box_contact`]'s answer on the two given boxes, plus the separating axis when the SAT
/// rejects the pair. `last_axis` is the previous frame's chosen SAT-axis index for this body pair.
pub(crate) fn box_box_classify(
    a: &Obb,
    b: &Obb,
    body_a: BodyIndex,
    body_b: BodyIndex,
    last_axis: Option<usize>,
) -> BoxBoxOutcome {
    let sat = match sat(a, b, last_axis) {
        Ok(sat) => sat,
        Err(SatMiss::Separated(axis)) => return BoxBoxOutcome::Separated(axis),
        Err(SatMiss::NoFaceAxis) => return BoxBoxOutcome::NoContact,
    };

    // Face versus edge, Box3D's rule (A7b; see FACE_AXIS_PREFERENCE). The face's patch is
    // built here only when the SAT's own answer is the edge. Otherwise the edge cannot win —
    // the patch is never deeper than the face axis's SAT depth, and the preference exceeds
    // SAT_EPS — so the pair builds its one face contact below, exactly as before.
    let mut face_built: Option<Manifold> = None;
    let best = match sat.edge {
        Some(edge) if edge.depth < sat.face.depth - SAT_EPS => {
            match face_contact(a, b, &sat.face, body_a, body_b) {
                Ok(m) if edge.depth >= patch_depth(&m) - FACE_AXIS_PREFERENCE => {
                    face_built = Some(m);
                    sat.face
                }
                // The edge is shallower than the realized patch by more than the
                // preference, or the face realizes no patch. A degenerate reference face
                // lands here too: the edge is the answer this pair got before S5.
                _ => edge,
            }
        }
        _ => sat.face,
    };

    // Reference-axis hysteresis: if last frame's axis is still overlapping and the best
    // axis is no deeper than HYSTERESIS_RATIO × its depth, KEEP it, so the reference face —
    // hence the feature ids — does not flip on FP noise across a resting near-parallel
    // pair. Except that an edge hint never holds a pair whose best axis is a face: that is
    // how an edge chosen once on jitter kept a resting face pair on one point (A7b).
    let chosen = match sat.hint {
        Some(last)
            if last.index != best.index
                && best.depth >= last.depth / HYSTERESIS_RATIO
                && !(last.is_edge() && !best.is_edge()) =>
        {
            last
        }
        _ => best,
    };

    let (manifold, reference_axis) = match chosen.class {
        SatClass::Edge { a: ea, b: eb } => {
            let Some(m) = edge_contact(a, b, &chosen, ea, eb, body_a, body_b) else {
                return BoxBoxOutcome::NoContact;
            };
            (m, chosen.index)
        }
        SatClass::FaceA(_) | SatClass::FaceB(_) => match face_built {
            Some(m) if chosen.index == sat.face.index => (m, chosen.index),
            built => match face_contact(a, b, &chosen, body_a, body_b) {
                Ok(m) => (m, chosen.index),
                // A held face hint that realizes no patch yields to the best face's patch when
                // the choice above already built it: the answer the pair gets with no hint, as
                // Box3D re-runs its full query when a cached feature fails.
                Err(FaceMiss::Empty) => match built {
                    Some(m) => {
                        #[cfg(test)]
                        HELD_FACE_YIELDS.with(|n| n.set(n.get() + 1));
                        (m, sat.face.index)
                    }
                    None => {
                        return match edge_fallback(a, b, &sat, body_a, body_b) {
                            Some(c) => BoxBoxOutcome::Contact(c),
                            None => BoxBoxOutcome::NoContact,
                        };
                    }
                },
                Err(FaceMiss::Degenerate) => return BoxBoxOutcome::NoContact,
            },
        },
    };

    BoxBoxOutcome::Contact(BoxBoxContact {
        manifold,
        reference_axis,
    })
}

/// The deepest penetration over a face manifold's points, `−min(separation)` — Box3D's
/// `clipSeparation`, negated. The reduction always keeps the deepest clipped point, so the
/// minimum over the kept points is the minimum over the whole clipped patch.
#[inline]
fn patch_depth(m: &Manifold) -> f32 {
    m.points[..usize::from(m.count)]
        .iter()
        .fold(0.0f32, |deepest, p| deepest.max(-p.separation))
}

/// The contact feature a box-box contact was built on (L9b D5): what a reuse record stores to
/// refresh the contact without the SAT and the clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FeatureRef {
    /// A face contact: the reference face is `axis` (`0..3`) of body B iff `ref_is_b`, on its
    /// positive side iff `positive`; the other body is the incident one.
    Face {
        /// Whether the reference face is body B's.
        ref_is_b: bool,
        /// The reference face's local axis.
        axis: u8,
        /// Whether it is the positive side of that axis.
        positive: bool,
    },
    /// An edge-edge contact on `A.axes[ea] × B.axes[eb]`.
    Edge {
        /// Body A's edge axis.
        ea: u8,
        /// Body B's edge axis.
        eb: u8,
    },
}

/// The feature the contact `c` of the boxes `(a, b)` was built on (L9b D5).
///
/// `c.reference_axis` names the SAT candidate whose [`face_contact`] or [`edge_contact`] built the
/// manifold (every path of [`box_box_classify`] and [`edge_fallback`] reports that one). A face
/// contact's manifold normal is that candidate's oriented axis, so the reference face is the one
/// `face_contact` picked: [`most_aligned_face`] of the reference box along the normal (FaceA) or
/// its negation (FaceB), the same inputs and so the same face.
pub(crate) fn contact_feature(a: &Obb, b: &Obb, c: &BoxBoxContact) -> FeatureRef {
    let i = c.reference_axis;
    debug_assert!(i < SAT_AXES, "invariant: a chosen SAT axis is 0..15");
    if i < 6 {
        let ref_is_b = i >= 3;
        let (reference, dir) =
            if ref_is_b { (b, c.manifold.normal * -1.0) } else { (a, c.manifold.normal) };
        let (axis, positive, _) = most_aligned_face(reference, dir);
        FeatureRef::Face { ref_is_b, axis: axis as u8, positive }
    } else {
        FeatureRef::Edge { ea: ((i - 6) / 3) as u8, eb: ((i - 6) % 3) as u8 }
    }
}

/// What [`refresh_edge`] found.
pub(crate) enum EdgeRefresh {
    /// The edge axis overlaps: today's edge contact on it.
    Contact(Manifold),
    /// The edge axis separates the boxes, on this canonical axis (`6..15`).
    Separated(u8),
    /// The edge pair is parallel now: no axis.
    Degenerate,
}

/// Re-evaluates the edge axis `A.axes[ea] × B.axes[eb]` of the boxes `(a, b)` and, when it
/// overlaps, builds today's edge contact on it (L9b D5, the refresh of an edge record).
///
/// The axis is built and evaluated exactly as [`sat`] builds and evaluates its candidate of the same
/// index, so a negative depth is a negative SAT candidate — the pair is separated exactly — and on
/// the poses the record was built on the contact is the full collision's, bit for bit.
#[inline]
pub(crate) fn refresh_edge(
    a: &Obb,
    b: &Obb,
    ea: usize,
    eb: usize,
    body_a: BodyIndex,
    body_b: BodyIndex,
) -> EdgeRefresh {
    debug_assert!(ea < 3 && eb < 3, "invariant: edge axes are 0..3");
    let index = 6 + 3 * ea + eb;
    let Some(cand) =
        eval_axis(a, b, a.axes[ea].cross(b.axes[eb]), SatClass::Edge { a: ea, b: eb }, index)
    else {
        return EdgeRefresh::Degenerate;
    };
    if cand.depth < 0.0 {
        return EdgeRefresh::Separated(index as u8);
    }
    match edge_contact(a, b, &cand, ea, eb, body_a, body_b) {
        Some(m) => EdgeRefresh::Contact(m),
        None => EdgeRefresh::Degenerate,
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only: how many times [`edge_fallback`] has run on this thread. A7-N11 reads it
    /// to prove the fallback branch is exercised; outside `cfg(test)` it does not exist, so
    /// the fallback carries no counting cost in a shipping build. Per-thread because the
    /// test harness runs each test on its own thread.
    ///
    /// Per-thread also means a count taken through the narrowphase SYSTEM sees only the pairs
    /// its own thread collided: with `parallel_narrowphase` on and a pool of two or more
    /// workers, the chunks run on other threads. A system-level reader of this counter or of
    /// [`HELD_FACE_YIELDS`] must run with one worker or with the flag off; the one reader today
    /// calls [`box_box_contact`] directly on the test thread.
    static FALLBACKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Test-only, like [`FALLBACKS`]: how many times a held face hint that realized no patch
    /// yielded to the best face's already-built patch in [`box_box_contact`].
    static HELD_FACE_YIELDS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The edge contact for a pair whose chosen face realizes no patch — the clip left nothing,
/// or no clipped point lies below the reference face — when no best-face patch was built to
/// yield to. Box3D takes the edge there ("face contact can be empty if it does not realize
/// the axis of minimum penetration"). Before S5 a chosen face that realized nothing gave no
/// manifold, but on the pile most of these pairs still had one then: the pre-S5 rule held
/// the edge hint that S5's class clause refuses a face-answered pair. Over steps 600-3000
/// of A7-R1's height-15 pile (msvc release, 2026-09-18) this ran 12 572 times; on 10 446 of
/// those calls the pre-S5 rule, given the same poses and hint, built an edge contact too,
/// and the other 2126 (under one a step) are manifolds S5 adds. A degenerate reference face
/// never reaches here — it stays "no contact", as its guard documents.
///
/// Cold and out of line: a face the SAT answered realizes a patch whenever the boxes truly
/// intersect, so this runs for FP-thin touches — 12 567 of those 12 572 calls were a
/// same-layer knife-edge pair whose best face realized nothing — and for a held face when
/// the best face's patch was not built (the other 5; Box3D would build that patch there).
/// The hint passes through, so edge↔edge hysteresis survives the fallback. Returns `None`
/// only when no edge axis exists, which needs every edge pair parallel: two orthonormal
/// frames never produce that, because each axis of one is parallel to at most one axis of
/// the other. Every edge axis that exists overlaps, or the SAT would have reported the pair
/// separated.
#[cold]
#[inline(never)]
fn edge_fallback(
    a: &Obb,
    b: &Obb,
    sat: &SatResult,
    body_a: BodyIndex,
    body_b: BodyIndex,
) -> Option<BoxBoxContact> {
    #[cfg(test)]
    FALLBACKS.with(|n| n.set(n.get() + 1));
    let best = sat.edge?;
    let chosen = match sat.hint {
        Some(last)
            if last.is_edge()
                && last.index != best.index
                && best.depth >= last.depth / HYSTERESIS_RATIO =>
        {
            last
        }
        _ => best,
    };
    let SatClass::Edge { a: ea, b: eb } = chosen.class else {
        unreachable!("invariant: the best edge and an edge hint are both edge-class axes")
    };
    edge_contact(a, b, &chosen, ea, eb, body_a, body_b).map(|manifold| BoxBoxContact {
        manifold,
        reference_axis: chosen.index,
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

/// Why [`face_contact`] produced no manifold. The two are kept apart because only an
/// empty patch hands the pair on — to the best face's built patch or to the edge path
/// (A7b); a degenerate face stays no contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FaceMiss {
    /// The clip left nothing, or no clipped point lies below the reference face: the face
    /// does not realize the contact.
    Empty,
    /// The reference face has a zero in-plane extent (a zero-volume collider face).
    Degenerate,
}

/// Builds a face-contact manifold via reference-face clip + reduction (P2 W4), on the
/// face axis `sat` (class `FaceA` / `FaceB`).
fn face_contact(
    a: &Obb,
    b: &Obb,
    sat: &AxisCandidate,
    body_a: BodyIndex,
    body_b: BodyIndex,
) -> Result<Manifold, FaceMiss> {
    // The SAT normal runs A→B. The reference box is the one OWNING the face
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
            return Err(FaceMiss::Degenerate);
        }
        if (edge_mid - ref_face_center).dot(side_normal) < 0.0 {
            side_normal = side_normal * -1.0;
        }
        let new_len =
            clip_against_plane(&src[..poly_len], edge_mid, side_normal, ref_face_id, e as u32, dst);
        core::mem::swap(&mut src, &mut dst);
        poly_len = new_len;
        if poly_len == 0 {
            return Err(FaceMiss::Empty);
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
        return Err(FaceMiss::Empty);
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
    Ok(manifold)
}

/// Builds a single-point edge-edge contact at the closest points of the two
/// contacting edges (P2 W4), on the edge axis `sat`.
fn edge_contact(
    a: &Obb,
    b: &Obb,
    sat: &AxisCandidate,
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

/// The pre-L9 bodies of `Obb::new`, the SAT and `box_box_contact` — their code verbatim, the
/// comments and the test-only yield counter dropped: the oracle of gate G-L9a-1 (`levers/L9-contact-reuse/02-DESIGN-REV1.md`, "The
/// pre-change body is kept as a `#[cfg(test)]` oracle"). The SAT evaluates all fifteen axes and
/// then rejects on any negative one, and the box axes are converted from the quaternion inline,
/// so a defect in the early exit or in the frame column cannot reach both sides of the gate.
/// Everything past the SAT is shared with the kernel, which L9a does not change.
#[cfg(test)]
mod pre_l9 {
    use super::*;
    use crate::math::Mat3;

    /// The pre-L9 `Obb::new`.
    pub(super) fn obb(center: Vec3, rotation: Quat, half_extents: Vec3) -> Obb {
        let r = Mat3::from_quat(rotation);
        // Column `i` of R is the world direction of local axis `i`. With row-major
        // storage, column `i` is `(rows[0][i], rows[1][i], rows[2][i])`.
        let axes = [
            Vec3::new(r.rows[0].x, r.rows[1].x, r.rows[2].x),
            Vec3::new(r.rows[0].y, r.rows[1].y, r.rows[2].y),
            Vec3::new(r.rows[0].z, r.rows[1].z, r.rows[2].z),
        ];
        Obb {
            center,
            axes,
            half: [half_extents.x, half_extents.y, half_extents.z],
        }
    }

    /// The pre-L9 SAT: all fifteen axes, then `None` on any negative depth.
    #[allow(clippy::needless_range_loop)]
    fn sat(a: &Obb, b: &Obb, last_axis: Option<usize>) -> Option<SatResult> {
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

        let face = shallowest(&candidates[..6]);
        debug_assert!(
            face.is_some(),
            "invariant: a box's face axes are unit rotation columns (length² = 1), so the \
             degeneracy guard never skips all six"
        );
        let face = face?;

        Some(SatResult {
            face,
            edge: shallowest(&candidates[6..]),
            hint: last_axis.and_then(|idx| candidates.get(idx).copied().flatten()),
        })
    }

    /// The pre-L9 `box_box_contact`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn box_box_contact(
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
        let a = obb(a_center, a_rotation, a_half);
        let b = obb(b_center, b_rotation, b_half);

        let sat = sat(&a, &b, last_axis)?;

        let mut face_built: Option<Manifold> = None;
        let best = match sat.edge {
            Some(edge) if edge.depth < sat.face.depth - SAT_EPS => {
                match face_contact(&a, &b, &sat.face, body_a, body_b) {
                    Ok(m) if edge.depth >= patch_depth(&m) - FACE_AXIS_PREFERENCE => {
                        face_built = Some(m);
                        sat.face
                    }
                    _ => edge,
                }
            }
            _ => sat.face,
        };

        let chosen = match sat.hint {
            Some(last)
                if last.index != best.index
                    && best.depth >= last.depth / HYSTERESIS_RATIO
                    && !(last.is_edge() && !best.is_edge()) =>
            {
                last
            }
            _ => best,
        };

        let (manifold, reference_axis) = match chosen.class {
            SatClass::Edge { a: ea, b: eb } => {
                (edge_contact(&a, &b, &chosen, ea, eb, body_a, body_b)?, chosen.index)
            }
            SatClass::FaceA(_) | SatClass::FaceB(_) => match face_built {
                Some(m) if chosen.index == sat.face.index => (m, chosen.index),
                built => match face_contact(&a, &b, &chosen, body_a, body_b) {
                    Ok(m) => (m, chosen.index),
                    Err(FaceMiss::Empty) => match built {
                        Some(m) => (m, sat.face.index),
                        None => return edge_fallback(&a, &b, &sat, body_a, body_b),
                    },
                    Err(FaceMiss::Degenerate) => return None,
                },
            },
        };

        Some(BoxBoxContact {
            manifold,
            reference_axis,
        })
    }
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

    /// The SAT's best face axis on a known overlap is the shallow axis.
    /// Boxes overlap deeply in x/z but barely in y → the best face is the y face.
    #[test]
    fn sat_picks_axis_of_least_penetration() {
        let a = Obb::new(Vec3::ZERO, Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0));
        // B overlaps A by 0.1 in y, fully in x and z.
        let b = Obb::new(Vec3::new(0.0, 1.9, 0.0), Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0));
        let s = sat(&a, &b, None).expect("overlap").face;
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

    // ── A7b: the face-versus-edge choice ─────────────────────────────────────────

    /// A rotation by `angle` radians about the unit `axis`.
    fn about(axis: Vec3, angle: f32) -> Quat {
        let (s, c) = (angle * 0.5).sin_cos();
        Quat::new(axis.x * s, axis.y * s, axis.z * s, c)
    }

    /// All 15 SAT candidates of `(a, b)` in canonical order, straight from [`eval_axis`]. The
    /// anti-vacuity guards read these, never the selection under test.
    fn candidates_of(a: &Obb, b: &Obb) -> [Option<AxisCandidate>; SAT_AXES] {
        let mut out = [None; SAT_AXES];
        for (i, (&axis_a, &axis_b)) in a.axes.iter().zip(&b.axes).enumerate() {
            out[i] = eval_axis(a, b, axis_a, SatClass::FaceA(i), i);
            out[3 + i] = eval_axis(a, b, axis_b, SatClass::FaceB(i), 3 + i);
        }
        for (ea, &axis_a) in a.axes.iter().enumerate() {
            for (eb, &axis_b) in b.axes.iter().enumerate() {
                let k = 6 + 3 * ea + eb;
                out[k] = eval_axis(a, b, axis_a.cross(axis_b), SatClass::Edge { a: ea, b: eb }, k);
            }
        }
        out
    }

    /// The shallowest candidate of one class by a plain minimum (the first on an exact tie):
    /// deliberately not the kernel's `SAT_EPS` tie rule, which it exists to check.
    fn plain_min(class: &[Option<AxisCandidate>]) -> Option<AxisCandidate> {
        class.iter().flatten().fold(None, |best: Option<AxisCandidate>, &c| match best {
            Some(b) if b.depth <= c.depth => Some(b),
            _ => Some(c),
        })
    }

    /// Whether a feature id is on the edge-edge path: bit 15 set, bit 14 clear.
    fn is_edge_id(id: u32) -> bool {
        id & super::super::TAG_NON_FACE != 0 && id & super::super::TAG_VERTEX_FACE == 0
    }

    /// The feature ids of a manifold's live points.
    fn ids_of(m: &Manifold) -> Vec<u32> {
        m.points[..usize::from(m.count)]
            .iter()
            .map(|p| p.feature_id)
            .collect()
    }

    /// A7-N9 (A7b, the kernel gate): a resting face pair never takes the edge path — cold,
    /// or with its own best edge axis as the hint.
    ///
    /// Box A (half-extent 1) sits at the origin and box B (half-extent 1) at `(sx, 2 − d,
    /// sz)`: the pile's four quarter overlaps `(±1, ±1)` and the full overlap `(0, 0)`,
    /// dipped `d` = 0.1 mm or 0.5 mm. One box is tilted about x AND z at once, by every pair
    /// from ±{0.05, 0.1, 0.2, 0.4} mrad — B in the first pass, A in the second — which spans
    /// the ~1e-5..2e-5 rad relative tilt a resting pile's support flips were measured at and
    /// reaches 20× past it. The tilt must be about both axes: about one axis alone every edge
    /// axis is an exact duplicate of a face axis or degenerate, the old rule already takes the
    /// face, and the pose exercises nothing. Only the MIXED-DIP poses exercise A7b: a quarter
    /// overlap whose tilt lowers the overlap region along one lateral axis and raises it
    /// along the other. Measured (msvc, 2026-09-18): exactly the tilts with
    /// `sign(tx·sz) = sign(tz·sx)`, 32 of every 64 on each of the four quarter offsets and
    /// in both passes — 512 of 1280 poses — and none at the full overlap, which is kept as
    /// the control. The count is printed per offset. Under Miri the sweep visits every 5th
    /// pose, 100 of whose 256 are such poses.
    ///
    /// * **Arm 1, no hint:** every pose yields a manifold and no point carries an edge-edge
    ///   id. Restoring the pre-S5 selection (the edge wins whenever its SAT depth is below
    ///   the face's by more than `SAT_EPS`) reds it: 1024 of the 2560 (pose, arm) cells,
    ///   measured 2026-09-18. Setting `FACE_AXIS_PREFERENCE` to `SAT_EPS` does NOT: against
    ///   the realized patch a duplicate edge axis is never shallower by more than FP noise
    ///   (see the constant), which is the point of the form.
    /// * **Arm 2, hint = the pose's best edge axis:** the same assertion. Deleting the
    ///   hysteresis clause (an edge hint never holds a face) reds it: 768 cells, measured.
    ///
    /// Anti-vacuity, computed from [`eval_axis`] independently of the selection: the poses
    /// where the best edge axis is shallower than the best face axis by more than `SAT_EPS`,
    /// which are the poses where the pre-S5 rule takes the edge.
    #[test]
    fn a_resting_face_pair_never_takes_the_edge_path() {
        const OFFSETS: [(f32, f32); 5] =
            [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0), (0.0, 0.0)];
        const DIPS: [f32; 2] = [1.0e-4, 5.0e-4];
        const TILTS: [f32; 8] = [-4.0e-4, -2.0e-4, -1.0e-4, -0.5e-4, 0.5e-4, 1.0e-4, 2.0e-4, 4.0e-4];
        // Under Miri, every 5th pose: the narrowphase Miri set has a budget of about twice its
        // 94 s, and strided this test takes 24 s there (measured 2026-09-18).
        const STRIDE: usize = if cfg!(miri) { 5 } else { 1 };
        // Half the poses measured at each stride (msvc, 2026-09-18): 512 of 1280 natively, and
        // 100 of 256 under the Miri stride; see the doc comment.
        const OLD_RULE_EDGE_FLOOR: usize = if cfg!(miri) { 50 } else { 256 };

        let x = Vec3::new(1.0, 0.0, 0.0);
        let z = Vec3::new(0.0, 0.0, 1.0);
        let mut failures: Vec<String> = Vec::new();
        let mut old_rule_edge = [0usize; OFFSETS.len()];
        let mut cell = 0usize;
        let mut swept = 0usize;
        for tilt_b in [true, false] {
            for (o, &(sx, sz)) in OFFSETS.iter().enumerate() {
                for &d in &DIPS {
                    for &tx in &TILTS {
                        for &tz in &TILTS {
                            cell += 1;
                            if !cell.is_multiple_of(STRIDE) {
                                continue;
                            }
                            let tilt = about(x, tx) * about(z, tz);
                            let (ar, br) = if tilt_b {
                                (Quat::IDENTITY, tilt)
                            } else {
                                (tilt, Quat::IDENTITY)
                            };
                            let bc = Vec3::new(sx, 2.0 - d, sz);
                            // Formatted only for a message: a `format!` per pose dominates
                            // the Miri run otherwise.
                            let pose = || {
                                format!(
                                    "offset ({sx}, {sz}) dip {d} tilt ({tx}, {tz}) about (x, z) \
                                     on {}",
                                    if tilt_b { "B" } else { "A" }
                                )
                            };
                            swept += 1;

                            let cands = candidates_of(
                                &Obb::new(Vec3::ZERO, ar, Vec3::ONE),
                                &Obb::new(bc, br, Vec3::ONE),
                            );
                            let face = plain_min(&cands[..6])
                                .expect("construction: a box's face axes are never degenerate");
                            let edge = plain_min(&cands[6..]).unwrap_or_else(|| {
                                panic!(
                                    "construction: {} tilts about two axes, so no edge pair is \
                                     parallel",
                                    pose()
                                )
                            });
                            if edge.depth < face.depth - SAT_EPS {
                                old_rule_edge[o] += 1;
                            }

                            for hint in [None, Some(edge.index)] {
                                let c = box_box_contact(
                                    A, B, Vec3::ZERO, ar, Vec3::ONE, bc, br, Vec3::ONE, hint,
                                )
                                .unwrap_or_else(|| {
                                    panic!(
                                        "construction: {} puts B's incident face centre on A's \
                                         closed top face {d} m deep, so the boxes intersect",
                                        pose()
                                    )
                                });
                                let ids = ids_of(&c.manifold);
                                if !ids.iter().any(|&id| is_edge_id(id)) {
                                    continue;
                                }
                                let taken = cands[c.reference_axis];
                                let taken_depth = taken.map_or(f32::NAN, |t| t.depth);
                                failures.push(match hint {
                                    None => format!(
                                        "{}: a resting face pair took the edge path — axis {:?} \
                                         (index {}) depth {taken_depth} vs best face {:?} depth \
                                         {}; ids {ids:x?}",
                                        pose(),
                                        taken.map(|t| t.class),
                                        c.reference_axis,
                                        face.class,
                                        face.depth
                                    ),
                                    Some(k) => format!(
                                        "{}: hint = edge axis {k}: hysteresis restored the edge \
                                         axis over the preferred face (axis {}, depth \
                                         {taken_depth} vs best face {:?} depth {}; ids {ids:x?})",
                                        pose(),
                                        c.reference_axis,
                                        face.class,
                                        face.depth
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }

        assert_eq!(
            swept,
            2 * OFFSETS.len() * DIPS.len() * TILTS.len() * TILTS.len() / STRIDE,
            "construction: the sweep must visit every pose its stride selects"
        );
        assert!(
            failures.is_empty(),
            "{} of {} (pose, arm) cells took the edge path: {}",
            failures.len(),
            2 * swept,
            failures.join("; ")
        );
        let total: usize = old_rule_edge.iter().sum();
        println!(
            "A7-N9: {total} of {swept} poses are ones the pre-S5 rule takes the edge on; per \
             offset {OFFSETS:?}: {old_rule_edge:?}"
        );
        assert!(
            total >= OLD_RULE_EDGE_FLOOR,
            "A7-N9 anti-vacuity: only {total} of {swept} poses have a best edge axis shallower \
             than the best face axis by more than SAT_EPS (floor {OLD_RULE_EDGE_FLOOR}; per \
             offset {old_rule_edge:?}), so the sweep no longer reaches the poses A7b lived on"
        );
    }

    /// A7-N10: crossed edges still take the edge path — the face preference is a preference,
    /// not a veto.
    ///
    /// Box A is turned 30° about z and box B 15° about y, then 30° about x; B's centre is at
    /// `(1, 2.79, −0.5)`. A's z-edge crosses B's x-edge, and they interpenetrate by ~4.4 cm
    /// along `A.z × B.x`, while A's +x face (the best face axis) realizes a 3-point patch
    /// ~9.1 cm deep. The edge axis is therefore shallower than the REALIZED patch by more
    /// than [`FACE_AXIS_PREFERENCE`], and the contact is the single edge-edge point.
    ///
    /// The anti-vacuity guard establishes that independently of the selection: it takes the
    /// best face and best edge from [`eval_axis`], clips the face with the shipped
    /// [`face_contact`], and requires a NON-empty patch deeper than the edge by more than
    /// Box3D's `B3_LINEAR_SLOP` — the value the rule is specified at, deliberately not the
    /// kernel constant, so that retuning the constant reds on behaviour rather than on this
    /// guard. A pose whose patch is empty would not do: the cold fallback rebuilds the same
    /// edge contact there, so a rule that takes the face unconditionally would pass. Here
    /// that rule — or `FACE_AXIS_PREFERENCE = 1.0` — returns the 3-point face patch instead,
    /// and the behavioural assertion reds on it.
    #[test]
    fn crossed_edges_still_take_the_edge_path() {
        /// Box3D's `B3_LINEAR_SLOP`, the tolerance its `convex_manifold.c` applies.
        const BOX3D_LINEAR_SLOP: f32 = 0.005;

        let x = Vec3::new(1.0, 0.0, 0.0);
        let y = Vec3::new(0.0, 1.0, 0.0);
        let z = Vec3::new(0.0, 0.0, 1.0);
        let deg = core::f32::consts::PI / 180.0;
        let ar = about(z, 30.0 * deg);
        let br = about(x, 30.0 * deg) * about(y, 15.0 * deg);
        let bc = Vec3::new(1.0, 2.79, -0.5);
        let (a, b) = (Obb::new(Vec3::ZERO, ar, Vec3::ONE), Obb::new(bc, br, Vec3::ONE));

        let cands = candidates_of(&a, &b);
        assert!(
            cands.iter().flatten().all(|c| c.depth >= 0.0),
            "construction: the pose must overlap on all 15 axes"
        );
        let face = plain_min(&cands[..6]).expect("construction: face axes are never degenerate");
        let edge = plain_min(&cands[6..]).expect("construction: the boxes are turned about \
                                                   different axes, so some edge pair crosses");
        let SatClass::Edge { a: ea, b: eb } = edge.class else {
            unreachable!("invariant: indices 6..15 are edge axes")
        };
        let patch = face_contact(&a, &b, &face, A, B).unwrap_or_else(|miss| {
            panic!(
                "construction: the best face {:?} must realize a patch here, or the cold fallback \
                 is what builds the edge and this test cannot tell a face-always rule apart; \
                 face_contact gave {miss:?}",
                face.class
            )
        });
        let patch_deepest = patch.points[..usize::from(patch.count)]
            .iter()
            .fold(0.0f32, |d, p| d.max(-p.separation));
        let gap = patch_deepest - edge.depth;
        println!(
            "A7-N10: best face {:?} SAT depth {}, realized patch {} points {patch_deepest} m deep; \
             best edge {:?} depth {}; edge shallower than the patch by {gap} m",
            face.class, face.depth, patch.count, edge.class, edge.depth
        );
        assert!(
            patch.count >= 2 && gap > BOX3D_LINEAR_SLOP,
            "construction: the realized face patch must hold several points ({}) and be deeper \
             than the best edge by more than Box3D's linear slop {BOX3D_LINEAR_SLOP} m (it is \
             {gap} m)",
            patch.count
        );

        let c = box_box_contact(A, B, Vec3::ZERO, ar, Vec3::ONE, bc, br, Vec3::ONE, None)
            .expect("construction: the pose overlaps on every axis");
        let ids = ids_of(&c.manifold);
        let expected = feature_edge_edge(ea as u32, eb as u32);
        if ids.iter().all(|&id| !is_edge_id(id)) {
            panic!(
                "crossed edges produced a face manifold of {} points (axis {}, ids {ids:x?}); the \
                 best edge {:?} is {gap} m shallower than the realized face patch, more than \
                 Box3D's linear slop {BOX3D_LINEAR_SLOP} m (FACE_AXIS_PREFERENCE is \
                 {FACE_AXIS_PREFERENCE} m)",
                c.manifold.count, c.reference_axis, edge.class
            );
        }
        assert!(
            ids == [expected] && c.reference_axis == edge.index,
            "crossed edges must give the one edge-edge point on axis {} ({:?}, id {expected:#x}); \
             got axis {} with ids {ids:x?}",
            edge.index,
            edge.class,
            c.reference_axis
        );
    }

    /// The pre-S5 contact rule, frozen verbatim from `08fe7b9f`: the global argmin with its
    /// `SAT_EPS` face window, then the hysteresis with no class clause, driving the shipped
    /// [`face_contact`] / [`edge_contact`] (a face that realizes no patch is no contact, as
    /// it was). A7-N11's oracle; `scalar_box_sdf_manifold` is the precedent for a frozen
    /// oracle in a test module. Returns the manifold and the axis index it was built on.
    #[allow(clippy::needless_range_loop)]
    fn pre_s5_contact(a: &Obb, b: &Obb, last_axis: Option<usize>) -> Option<(Manifold, usize)> {
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

        for cand in candidates.iter().flatten() {
            if cand.depth < 0.0 {
                return None;
            }
        }

        let mut best: Option<AxisCandidate> = None;
        for cand in candidates.iter().flatten() {
            best = Some(match best {
                None => *cand,
                Some(cur) => {
                    let cand_is_edge = matches!(cand.class, SatClass::Edge { .. });
                    let cur_is_edge = matches!(cur.class, SatClass::Edge { .. });
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

        let chosen = match last_axis.and_then(|idx| candidates.get(idx).copied().flatten()) {
            Some(last) if last.index != best.index && best.depth >= last.depth / HYSTERESIS_RATIO => last,
            _ => best,
        };

        let manifold = match chosen.class {
            SatClass::FaceA(_) | SatClass::FaceB(_) => face_contact(a, b, &chosen, A, B).ok(),
            SatClass::Edge { a: ea, b: eb } => edge_contact(a, b, &chosen, ea, eb, A, B),
        };
        manifold.map(|m| (m, chosen.index))
    }

    /// One pose of A7-N11.
    #[derive(Clone, Copy)]
    struct PosePair {
        a_centre: Vec3,
        a_rot: Quat,
        a_half: Vec3,
        b_centre: Vec3,
        b_rot: Quat,
        b_half: Vec3,
    }

    /// What one A7-N11 cell produced: S5's axis and the pre-S5 rule's (`None` = no manifold),
    /// and which out-of-line branch S5 took.
    struct CellOutcome {
        s5_axis: Option<usize>,
        oracle_axis: Option<usize>,
        fell_back: bool,
        yielded: bool,
    }

    /// A7-N11's per-cell bookkeeping: each `(pose, hint)` cell runs through the frozen
    /// oracle and through S5.
    #[derive(Default)]
    struct NeverLoses {
        visited: usize,
        oracle_some: usize,
        class_differs: usize,
        added: usize,
        fallbacks: usize,
        yields: usize,
        lost: Vec<String>,
        rescued: Vec<String>,
        /// The pose being swept: the first `(hint, S5 produced a manifold)` asked of it.
        pose_first: Option<(Option<usize>, bool)>,
        /// Cells of the pose being swept.
        pose_cells: usize,
        /// Poses asked with at least two hints.
        poses_compared: usize,
        /// Poses on which S5's answer to "is there a manifold" changed with the hint.
        hint_dependent: Vec<String>,
    }

    impl NeverLoses {
        /// Starts a pose: every cell until the next call is the same pose under another hint.
        fn begin_pose(&mut self) {
            self.pose_first = None;
            self.pose_cells = 0;
        }

        /// `pose` is formatted only for a message: a `format!` per cell dominates the Miri
        /// run otherwise.
        fn cell(
            &mut self,
            pose: impl Fn() -> String,
            p: PosePair,
            hint: Option<usize>,
        ) -> CellOutcome {
            self.visited += 1;
            let a = Obb::new(p.a_centre, p.a_rot, p.a_half);
            let b = Obb::new(p.b_centre, p.b_rot, p.b_half);
            let oracle_axis = pre_s5_contact(&a, &b, hint).map(|(_, axis)| axis);
            let fallbacks = FALLBACKS.with(std::cell::Cell::get);
            let yields = HELD_FACE_YIELDS.with(std::cell::Cell::get);
            let s5_axis = box_box_contact(
                A, B, p.a_centre, p.a_rot, p.a_half, p.b_centre, p.b_rot, p.b_half, hint,
            )
            .map(|c| c.reference_axis);
            let fell_back = FALLBACKS.with(std::cell::Cell::get) > fallbacks;
            let yielded = HELD_FACE_YIELDS.with(std::cell::Cell::get) > yields;
            self.fallbacks += usize::from(fell_back);
            self.yields += usize::from(yielded);
            self.oracle_some += usize::from(oracle_axis.is_some());
            match (oracle_axis, s5_axis) {
                (Some(old_axis), None) => self.lost.push(format!(
                    "{} hint {hint:?}: S5 produced no manifold where the pre-S5 rule produced {} \
                     contact (axis {old_axis})",
                    pose(),
                    if old_axis >= 6 { "an edge" } else { "a face" }
                )),
                (Some(old_axis), Some(new_axis)) => {
                    if (old_axis >= 6) != (new_axis >= 6) {
                        self.class_differs += 1;
                    }
                    if fell_back && old_axis >= 6 {
                        self.rescued.push(format!("{} hint {hint:?}", pose()));
                    }
                }
                (None, Some(_)) => self.added += 1,
                (None, None) => {}
            }
            self.pose_cells += 1;
            match self.pose_first {
                None => self.pose_first = Some((hint, s5_axis.is_some())),
                Some((first_hint, first)) => {
                    self.poses_compared += usize::from(self.pose_cells == 2);
                    if first != s5_axis.is_some() {
                        let says = |some: bool| if some { "a manifold" } else { "no manifold" };
                        self.hint_dependent.push(format!(
                            "{}: hint {first_hint:?} gives {}, hint {hint:?} gives {}",
                            pose(),
                            says(first),
                            says(s5_axis.is_some())
                        ));
                    }
                }
            }
            CellOutcome {
                s5_axis,
                oracle_axis,
                fell_back,
                yielded,
            }
        }
    }

    /// One recorded pose of A7-N11's families G and H, as plain arrays so the tables are
    /// `const`s, with the hint it was recorded under.
    struct PoseLiteral {
        a_centre: [f32; 3],
        a_rot: [f32; 4],
        a_half: [f32; 3],
        b_rot: [f32; 4],
        b_half: [f32; 3],
        b_centre: [f32; 3],
        hint: usize,
    }

    impl PoseLiteral {
        fn pair(&self) -> PosePair {
            let quat = |q: [f32; 4]| Quat::new(q[0], q[1], q[2], q[3]);
            let vec = |v: [f32; 3]| Vec3::new(v[0], v[1], v[2]);
            PosePair {
                a_centre: vec(self.a_centre),
                a_rot: quat(self.a_rot),
                a_half: vec(self.a_half),
                b_centre: vec(self.b_centre),
                b_rot: quat(self.b_rot),
                b_half: vec(self.b_half),
            }
        }
    }

    /// Family G of A7-N11: generic pairs on which the hysteresis holds a FACE hint whose
    /// patch is empty, while the pre-S5 rule answered with an edge contact. S5's best axis
    /// there is a face whose realized patch is within [`FACE_AXIS_PREFERENCE`] of the edge —
    /// a genuine crossing, whose face axis reads a SAT depth of 0.2..0.6 m against a patch of
    /// millimetres — and the hysteresis compares the hint against that SAT depth, so it holds
    /// a face hint the pre-S5 rule refused against the edge's depth. The held face realizes
    /// nothing, and S5 takes the best face's patch, which the choice already built — the
    /// answer the pair gets with no hint, as Box3D re-runs its full query when a cached
    /// feature fails. (Before that was adopted in review, the cold fallback built the edge
    /// contact here, and without either S5 returns no contact on each of them.)
    ///
    /// Found by a seeded random search, msvc release, 2026-09-18: xorshift64 from
    /// `0x9E37_79B9_7F4A_7C15`, 400 000 draws — half tilted up to 0.01 rad about a random
    /// axis, half arbitrarily rotated — with half-extents 0.3..1.8 m and centres in a 6 m
    /// cube, asked with every hint. Of 154 423 overlapping poses × 16 hints the held face
    /// realized nothing 13 times: every time on an arbitrarily rotated pair with a face hint,
    /// and every time where the pre-S5 rule had an edge contact. These are the nine whose hold
    /// clears the 1.05 ratio by more than 9 mm. The literals are the printed shortest
    /// round-trip values, so each reproduces its pose bit for bit.
    const HELD_FACE_POSES: [PoseLiteral; 9] = [
        PoseLiteral {
            a_centre: [0.0, 0.0, 0.0],
            a_rot: [-0.6648342, 0.17607373, 0.49660873, 0.529503],
            a_half: [1.5036271, 0.49778914, 0.653097],
            b_rot: [0.546594, 0.3655251, 0.48252043, 0.5786195],
            b_half: [1.1914029, 0.8799692, 1.0102658],
            b_centre: [-1.1032796, 1.5676521, -1.5316939],
            hint: 2,
        },
        PoseLiteral {
            a_centre: [0.0, 0.0, 0.0],
            a_rot: [0.5952509, -0.51643276, -0.23746298, 0.5679656],
            a_half: [1.7873447, 1.7234278, 0.9652575],
            b_rot: [-0.51186764, -0.08664073, -0.32949403, 0.7886182],
            b_half: [0.5224719, 1.1395481, 1.0437485],
            b_centre: [2.877087, -0.7414718, -0.6032413],
            hint: 2,
        },
        PoseLiteral {
            a_centre: [0.0, 0.0, 0.0],
            a_rot: [-0.13617334, -0.14611992, 0.19333825, 0.9605863],
            a_half: [1.6981032, 1.3720462, 1.5774915],
            b_rot: [0.19533393, -0.1699526, -0.22376809, 0.9396215],
            b_half: [0.47136924, 1.5411577, 0.70284706],
            b_centre: [1.4933656, 2.4795933, 2.8321445],
            hint: 2,
        },
        PoseLiteral {
            a_centre: [0.0, 0.0, 0.0],
            a_rot: [-0.11826542, -0.22589156, -0.078930974, 0.96371996],
            a_half: [1.4514737, 1.5332198, 1.3561597],
            b_rot: [-0.1273879, 0.40622297, -0.54217976, 0.72442836],
            b_half: [0.7923305, 1.1464846, 0.7390419],
            b_centre: [1.4141132, 1.7781404, 2.4172592],
            hint: 3,
        },
        PoseLiteral {
            a_centre: [0.0, 0.0, 0.0],
            a_rot: [-0.33984423, -0.29841387, 0.8477624, 0.2770452],
            a_half: [1.2417753, 0.97033834, 0.36877787],
            b_rot: [-0.0945314, -0.34941804, -0.9184993, 0.15915443],
            b_half: [0.37938005, 0.5825089, 1.6009762],
            b_centre: [0.14320636, 2.3151736, 0.36153924],
            hint: 4,
        },
        PoseLiteral {
            a_centre: [0.0, 0.0, 0.0],
            a_rot: [-0.15935925, 0.4907358, 0.28044337, 0.80940384],
            a_half: [0.9519472, 0.9570394, 1.2769477],
            b_rot: [0.0050486103, -0.020915672, -0.037814833, 0.9990531],
            b_half: [1.6334269, 1.7562027, 0.8177097],
            b_centre: [0.6311023, 2.968233, 1.8120396],
            hint: 5,
        },
        PoseLiteral {
            a_centre: [0.0, 0.0, 0.0],
            a_rot: [0.62574583, 0.63963133, -0.08548456, 0.43818536],
            a_half: [0.3299575, 0.67026836, 1.1001298],
            b_rot: [0.5439136, -0.55330473, -0.5499445, 0.3091487],
            b_half: [0.985748, 0.38174158, 1.7291548],
            b_centre: [1.9717201, 1.1541452, 0.36960268],
            hint: 1,
        },
        PoseLiteral {
            a_centre: [0.0, 0.0, 0.0],
            a_rot: [-0.0013517787, 0.14451209, 0.12155152, 0.982008],
            a_half: [1.5199089, 0.7582309, 0.8454623],
            b_rot: [0.29862013, 0.22358985, -0.30708984, 0.87551665],
            b_half: [1.3675344, 0.38682935, 0.50728595],
            b_centre: [-2.807338, 0.87298214, -0.263772],
            hint: 0,
        },
        PoseLiteral {
            a_centre: [0.0, 0.0, 0.0],
            a_rot: [-0.7331118, 0.3805939, 0.52216387, 0.21222678],
            a_half: [0.8348465, 1.4515908, 0.6626575],
            b_rot: [-0.12036794, 0.4965787, 0.5743793, 0.6395385],
            b_half: [1.3109138, 0.77814513, 0.499645],
            b_centre: [-0.14276308, 1.6186888, 1.3813058],
            hint: 5,
        },
    ];

    /// Family H of A7-N11: resting knife-edge pairs on which the SAT's own best face realizes
    /// no patch — same-layer diagonal neighbours of the height-15 pile, touching at an overlap
    /// of FP size, where every clipped point lies above the reference face. The cold fallback
    /// builds their edge contact. Before S5 such a pair had a manifold only while the
    /// hysteresis held an edge hint (the pre-S5 rule took the face and got nothing on a cold
    /// call), so its existence depended on the hint; S5 adds the manifold on a cold call.
    ///
    /// Printed by a temporary probe of A7-R1's scene (msvc release, 2026-09-18) with the hint
    /// each pair read on that step: over steps 600-3000 the fallback ran 12 572 times, 12 567
    /// of them on such a pair, and kept an edge hint over its own best edge 279 times. The
    /// first four here are such holds, one per pile pair — the fallback's hysteresis is what
    /// picks their axis, and it picks the axis the pre-S5 rule held — and the last two hold
    /// the best edge itself. Box A is at its pile position, so each literal reproduces its call
    /// bit for bit.
    const EMPTY_FACE_POSES: [PoseLiteral; 6] = [
        PoseLiteral {
            a_centre: [9.000141, 0.9990835, -13.000137],
            a_rot: [1.8768058e-5, 3.259257e-5, 2.9724286e-5, 1.0],
            a_half: [1.0, 1.0, 1.0],
            b_rot: [4.9517865e-5, 2.3405475e-5, 1.1294186e-6, 1.0],
            b_half: [1.0, 1.0, 1.0],
            b_centre: [11.000066, 0.9994316, -15.000288],
            hint: 13,
        },
        PoseLiteral {
            a_centre: [-5.0000453, 0.9978098, 2.9998622],
            a_rot: [1.243791e-5, -1.9552801e-6, -2.7462032e-5, 1.0],
            a_half: [1.0, 1.0, 1.0],
            b_rot: [5.7928326e-5, -5.415284e-5, -2.1504922e-5, 1.0],
            b_half: [1.0, 1.0, 1.0],
            b_centre: [-3.0001512, 0.99766785, 5.0000534],
            hint: 11,
        },
        PoseLiteral {
            a_centre: [-3.0001805, 0.9982773, 9.000466],
            a_rot: [-3.3563912e-5, 6.289356e-6, -2.415498e-5, 1.0],
            a_half: [1.0, 1.0, 1.0],
            b_rot: [2.4368242e-6, 4.8374848e-5, -3.9761864e-5, 1.0],
            b_half: [1.0, 1.0, 1.0],
            b_centre: [-1.000273, 0.9978106, 7.000306],
            hint: 11,
        },
        PoseLiteral {
            a_centre: [-11.000313, 0.99829894, 2.9996977],
            a_rot: [9.8534394e-5, -1.4946176e-5, 3.619122e-5, 1.0],
            a_half: [1.0, 1.0, 1.0],
            b_rot: [0.00010402989, 5.004693e-5, 2.7550572e-5, 1.0],
            b_half: [1.0, 1.0, 1.0],
            b_centre: [-9.000367, 0.99812645, 0.9996273],
            hint: 11,
        },
        PoseLiteral {
            a_centre: [-0.000291323, 18.996376, -4.0000224],
            a_rot: [3.4758283e-5, 1.4220808e-5, -7.350325e-6, 1.0],
            a_half: [1.0, 1.0, 1.0],
            b_rot: [2.245445e-5, -2.9725077e-6, -2.6801637e-5, 1.0],
            b_half: [1.0, 1.0, 1.0],
            b_centre: [1.9997394, 18.996225, -2.000021],
            hint: 7,
        },
        PoseLiteral {
            a_centre: [-9.000111, 0.9990819, -13.0004],
            a_rot: [1.0700427e-5, 1.7921036e-5, -3.0772883e-6, 1.0],
            a_half: [1.0, 1.0, 1.0],
            b_rot: [2.4418694e-5, 4.708176e-5, -2.1782123e-5, 1.0],
            b_half: [1.0, 1.0, 1.0],
            b_centre: [-7.0002055, 0.99913573, -15.0005245],
            hint: 11,
        },
    ];

    /// A7-N11: the face preference never loses a contact — wherever the pre-S5 rule produced
    /// a manifold, S5 produces one too, for every pose and every hint — and whether S5
    /// produces one never depends on the hint.
    ///
    /// **Family F** (the plan's): B (half-extent 1) over A's top face (half-extent 1, at the
    /// origin), centred at `(ox, 2 − overlap, oz)` with `ox, oz ∈ {0, 1, 1.5, 1.9, 1.99}` —
    /// full and quarter overlaps, B overhanging A's top-face edges and their corner regions,
    /// out to corner over corner on a 1 cm square — tilted about x and z at once by every
    /// pair from {0, ±1, ±4} mrad, dipped `overlap ∈ {0.1, 1, 4}` mm, and asked with every
    /// hint `{None} ∪ {0..15}`. Under Miri, every 97th (pose, hint) cell: 309 of 30 000,
    /// 82 of them ending on different classes (measured natively with the stride forced).
    /// Families G and H run unstrided there too; the whole narrowphase Miri set then took
    /// 157 s (msvc, 2026-09-18; 94 s before S5).
    /// F reaches the choice S5 changed (anti-vacuity 1) but NOT the fallback: it ran in none
    /// of F's 30 000 cells (measured 2026-09-18), nor in 58 800 cells with the offsets
    /// extended to 1.996 and 1.999. A face the SAT answers realizes a patch whenever the
    /// boxes truly intersect, so a near-resting pair never needs it.
    ///
    /// **Family G** ([`HELD_FACE_POSES`]) is a face held by the hysteresis whose patch is
    /// empty, and **family H** ([`EMPTY_FACE_POSES`]) a knife-edge pair whose best face
    /// realizes nothing; both are asked with every hint, also under Miri. At its recorded
    /// hint a G pose must take the best face's already-built patch, and an H pose must fall
    /// back to the edge and keep the edge axis the pre-S5 rule held — the edge↔edge
    /// hysteresis passes through the fallback.
    ///
    /// **Existence is a function of the poses alone** (the review of S5, W1). After the SAT
    /// overlaps, S5 returns no contact only for a degenerate reference face (a zero in-plane
    /// extent) or when no edge axis exists, which two orthonormal frames never produce; every
    /// other path ends in a face patch or an edge contact. So a box pair with non-zero
    /// extents has a manifold under every hint or under none — A4's Known behaviour 2 cannot
    /// occur for box pairs. Every pose asked with two or more hints is checked. Making the
    /// fallback run only when a hint is held (a cold pair whose best face realizes nothing
    /// gets no contact, as before S5) reds this assertion and nothing before it: the pre-S5
    /// rule lacks those contacts on the cold call too, so "never loses" cannot see it.
    ///
    /// The oracle is [`pre_s5_contact`]. Anti-vacuity: (1) some F cells end on a different
    /// axis class under the two rules; (2) in some cell the fallback ran (the test-only
    /// `FALLBACKS` counter moved) where the oracle had an edge contact, so the fallback is
    /// what kept it. Also printed: the cells where S5 has a manifold the oracle lacked.
    #[test]
    fn face_preference_never_loses_a_contact() {
        const OFFSETS: [f32; 5] = [0.0, 1.0, 1.5, 1.9, 1.99];
        const TILTS: [f32; 5] = [-4.0e-3, -1.0e-3, 0.0, 1.0e-3, 4.0e-3];
        const OVERLAPS: [f32; 3] = [1.0e-4, 1.0e-3, 4.0e-3];
        const HINTS: usize = 1 + SAT_AXES;
        const STRIDE: usize = if cfg!(miri) { 97 } else { 1 };

        let x = Vec3::new(1.0, 0.0, 0.0);
        let z = Vec3::new(0.0, 0.0, 1.0);
        let mut f = NeverLoses::default();
        let mut cell = 0usize;
        for &ox in &OFFSETS {
            for &oz in &OFFSETS {
                for &tx in &TILTS {
                    for &tz in &TILTS {
                        for &overlap in &OVERLAPS {
                            f.begin_pose();
                            let pair = PosePair {
                                a_centre: Vec3::ZERO,
                                a_rot: Quat::IDENTITY,
                                a_half: Vec3::ONE,
                                b_centre: Vec3::new(ox, 2.0 - overlap, oz),
                                b_rot: about(x, tx) * about(z, tz),
                                b_half: Vec3::ONE,
                            };
                            for h in 0..HINTS {
                                cell += 1;
                                if !cell.is_multiple_of(STRIDE) {
                                    continue;
                                }
                                let pose = || {
                                    format!(
                                        "F: offset ({ox}, {oz}) tilt ({tx}, {tz}) about (x, z) \
                                         overlap {overlap}"
                                    )
                                };
                                f.cell(pose, pair, h.checked_sub(1));
                            }
                        }
                    }
                }
            }
        }

        let mut g = NeverLoses::default();
        let mut g_wrong = Vec::new();
        for (i, p) in HELD_FACE_POSES.iter().enumerate() {
            g.begin_pose();
            for h in 0..HINTS {
                let hint = h.checked_sub(1);
                let out = g.cell(|| format!("G: pose {i}"), p.pair(), hint);
                if hint == Some(p.hint) && !(out.yielded && out.s5_axis.is_some_and(|ax| ax < 6)) {
                    g_wrong.push(format!(
                        "pose {i} hint {hint:?}: S5 axis {:?} (fell back: {}), pre-S5 axis {:?}",
                        out.s5_axis, out.fell_back, out.oracle_axis
                    ));
                }
            }
        }
        let mut hh = NeverLoses::default();
        let mut h_wrong = Vec::new();
        for (i, p) in EMPTY_FACE_POSES.iter().enumerate() {
            hh.begin_pose();
            for h in 0..HINTS {
                let hint = h.checked_sub(1);
                let out = hh.cell(|| format!("H: pose {i}"), p.pair(), hint);
                if hint == Some(p.hint)
                    && !(out.fell_back
                        && out.oracle_axis == Some(p.hint)
                        && out.s5_axis == Some(p.hint))
                {
                    h_wrong.push(format!(
                        "pose {i} hint {hint:?}: S5 axis {:?} (fell back: {}), pre-S5 axis {:?}",
                        out.s5_axis, out.fell_back, out.oracle_axis
                    ));
                }
            }
        }

        let families = [("F", &f), ("G", &g), ("H", &hh)];
        let visited: usize = families.iter().map(|(_, t)| t.visited).sum();
        let lost: Vec<&str> = families
            .iter()
            .flat_map(|(_, t)| &t.lost)
            .map(String::as_str)
            .collect();
        assert!(
            lost.is_empty(),
            "{} of {visited} (pose, hint) cells lost a contact: {}",
            lost.len(),
            lost.join("; ")
        );
        let hint_dependent: Vec<&str> = families
            .iter()
            .flat_map(|(_, t)| &t.hint_dependent)
            .map(String::as_str)
            .collect();
        assert!(
            hint_dependent.is_empty(),
            "whether a box pair with non-zero extents has a manifold changed with the axis hint \
             in {} (pose, hint) cells; it must depend on the poses alone: {}",
            hint_dependent.len(),
            hint_dependent.join("; ")
        );
        assert!(
            g_wrong.is_empty(),
            "family G: a held face hint that realizes no patch must yield to the best face's \
             already-built patch, not fall back to the edge: {}",
            g_wrong.join("; ")
        );
        assert!(
            h_wrong.is_empty(),
            "family H: where the best face realizes no patch, the cold fallback must build the \
             edge contact on the edge axis the hysteresis holds, the one the pre-S5 rule held: {}",
            h_wrong.join("; ")
        );
        for (name, t) in families {
            println!(
                "A7-N11 {name}: {} cells, the pre-S5 rule has a manifold in {}; the two rules end \
                 on different axis classes in {}; the fallback ran in {} and kept an edge contact \
                 the pre-S5 rule had in {}; a held face yielded to the built patch in {}; S5 adds \
                 a manifold the pre-S5 rule lacked in {}; {} poses asked with two or more hints",
                t.visited,
                t.oracle_some,
                t.class_differs,
                t.fallbacks,
                t.rescued.len(),
                t.yields,
                t.added,
                t.poses_compared
            );
        }
        assert!(
            f.class_differs > 0,
            "A7-N11 anti-vacuity (1): the two rules never end on different axis classes in {} \
             family-F cells, so F no longer reaches the choice S5 changed",
            f.visited
        );
        assert!(
            families.iter().any(|(_, t)| !t.rescued.is_empty()),
            "A7-N11 anti-vacuity (2): no cell has the fallback run where the pre-S5 rule had an \
             edge contact ({} fallbacks in {visited} cells), so this test does not exercise what \
             the fallback exists for",
            families.iter().map(|(_, t)| t.fallbacks).sum::<usize>()
        );
    }

    /// R5's degenerate clause (A7b): a CHOSEN reference face with a zero in-plane extent is
    /// no contact — it does not fall back to the edge — and the guard that says so is reached.
    ///
    /// Box A is flat (half-extents `(1, 0, 1)`) at the origin and box B a unit box at
    /// `(1.95, 0, 0)`. They overlap by 5 cm along x; the best face axis is A's x axis (index 0,
    /// the lower index of the x tie) and no edge axis is shallower, so the SAT's answer is the
    /// face and the reference is A's `+x` face, whose extent along y is zero. Its side planes
    /// degenerate. [`degenerate_zero_extent_box_no_malformed_manifold`] never reaches the guard:
    /// its degenerate face is the incident one.
    ///
    /// In a debug build the `debug_assert!` beside the guard fires first — it exists to catch
    /// a zero-extent collider upstream — so there, and under Miri, the test expects that
    /// panic. The guard's own behaviour is checked in release, where
    /// `cargo test --release -p boyko-physics` runs it: letting a degenerate face fall back to
    /// the edge, or deleting the guard (every vertex then survives the degenerate side plane),
    /// returns a manifold and reds it there.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(
            expected = "invariant: a non-degenerate reference face has non-zero in-plane extents"
        )
    )]
    fn a_degenerate_reference_face_is_no_contact() {
        let a_half = Vec3::new(1.0, 0.0, 1.0);
        let bc = Vec3::new(1.95, 0.0, 0.0);
        let (a, b) = (
            Obb::new(Vec3::ZERO, Quat::IDENTITY, a_half),
            Obb::new(bc, Quat::IDENTITY, Vec3::ONE),
        );
        let cands = candidates_of(&a, &b);
        assert!(
            cands.iter().flatten().all(|c| c.depth >= 0.0),
            "construction: the pair must overlap on every axis"
        );
        let face = plain_min(&cands[..6]).expect("construction: face axes are never degenerate");
        let edge = plain_min(&cands[6..])
            .expect("construction: A.y × B.z and A.z × B.y are unit axes along x");
        assert!(
            face.index == 0 && edge.depth >= face.depth - SAT_EPS,
            "construction: the SAT's answer must be A's x face (index 0); best face {:?} depth {}, \
             best edge {:?} depth {}",
            face.class,
            face.depth,
            edge.class,
            edge.depth
        );
        let result = box_box_contact(
            A,
            B,
            Vec3::ZERO,
            Quat::IDENTITY,
            a_half,
            bc,
            Quat::IDENTITY,
            Vec3::ONE,
            None,
        );
        if let Some(c) = result {
            panic!(
                "a degenerate reference face (A's +x face, zero extent along y) produced a manifold \
                 of {} points on axis {} (ids {:x?}): a zero-extent face is no contact — it neither \
                 falls back to the edge nor clips against a zero side normal",
                c.manifold.count,
                c.reference_axis,
                ids_of(&c.manifold)
            );
        }
    }

    // ── G-L9a-1: the exact fast path against the pre-L9 oracle ─────────────────────────────

    /// xorshift64*: the seeded draws of the G-L9a-1 arms.
    struct Draw(u64);

    impl Draw {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }

        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            let unit = (self.next() >> 40) as f32 / (1u64 << 24) as f32;
            lo + (hi - lo) * unit
        }

        fn int(&mut self, lo: i32, hi: i32) -> f32 {
            (lo + self.below((hi - lo + 1) as u64) as i32) as f32
        }

        fn rotation(&mut self) -> Quat {
            loop {
                let q = Quat::new(
                    self.range(-1.0, 1.0),
                    self.range(-1.0, 1.0),
                    self.range(-1.0, 1.0),
                    self.range(-1.0, 1.0),
                );
                let n2 = q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w;
                if (0.01..=1.0).contains(&n2) {
                    return q.normalize();
                }
            }
        }

        /// The identity or a half turn about a coordinate axis: `Mat3::from_quat` of each is
        /// exact (entries 0 and ±1), so an integer lattice stays exact under it.
        fn exact_rotation(&mut self) -> Quat {
            match self.below(4) {
                0 => Quat::IDENTITY,
                1 => Quat::new(1.0, 0.0, 0.0, 0.0),
                2 => Quat::new(0.0, 1.0, 0.0, 0.0),
                _ => Quat::new(0.0, 0.0, 1.0, 0.0),
            }
        }

        fn hint(&mut self) -> Option<usize> {
            let h = self.below(SAT_AXES as u64 + 1) as usize;
            (h < SAT_AXES).then_some(h)
        }

        /// Half-extents with each component zero one time in three.
        fn degenerate_half(&mut self) -> Vec3 {
            let mut h = Vec3::new(self.range(0.2, 1.5), self.range(0.2, 1.5), self.range(0.2, 1.5));
            if self.below(3) == 0 {
                h.x = 0.0;
            }
            if self.below(3) == 0 {
                h.y = 0.0;
            }
            if self.below(3) == 0 {
                h.z = 0.0;
            }
            h
        }
    }

    /// One pose of a G-L9a-1 case.
    #[derive(Clone, Copy, Debug)]
    struct L9aPose {
        ca: Vec3,
        qa: Quat,
        ha: Vec3,
        cb: Vec3,
        qb: Quat,
        hb: Vec3,
        hint: Option<usize>,
    }

    /// The step frame of a body with orientation `q` (the fill's axes; no pair reads the radius).
    fn frame_of(q: Quat) -> RowFrame {
        RowFrame { axes: RowFrame::axes_of(q), radius: 0.0 }
    }

    /// A contact as words: every manifold field and point slot, then the reference axis.
    fn contact_words(c: &BoxBoxContact) -> Vec<u32> {
        let m = &c.manifold;
        let mut w = vec![m.body_a.0, m.body_b.0, u32::from(m.count)];
        w.extend([m.normal.x, m.normal.y, m.normal.z].map(f32::to_bits));
        for p in &m.points {
            w.extend(
                [p.anchor_a.x, p.anchor_a.y, p.anchor_a.z, p.anchor_b.x, p.anchor_b.y, p.anchor_b.z]
                    .map(f32::to_bits),
            );
            w.extend([p.separation.to_bits(), p.feature_id]);
        }
        w.push(c.reference_axis as u32);
        w
    }

    /// What one G-L9a-1 case exercised.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum L9aSeen {
        Contact,
        /// A contact on a pose with an axis of depth exactly zero: the pair an early exit on
        /// `depth <= 0` would call separated.
        ContactAtZeroDepth,
        Separated,
        NoContact,
        /// Both sides panicked (a degenerate reference face's debug assertion).
        BothPanicked,
    }

    /// One G-L9a-1 case: [`box_box_classify`] on boxes built from the step's frames
    /// ([`Obb::from_frame`] of [`RowFrame::axes_of`]) and the public [`box_box_contact`] against the
    /// pre-L9 oracle — the manifold's words and the axis, or no contact; a separated answer must
    /// name the FIRST axis in canonical order whose depth is negative on the oracle's boxes.
    fn l9a_case(p: L9aPose) -> Result<L9aSeen, String> {
        let oracle = std::panic::catch_unwind(|| {
            pre_l9::box_box_contact(A, B, p.ca, p.qa, p.ha, p.cb, p.qb, p.hb, p.hint)
                .map(|c| contact_words(&c))
        });
        let kernel = std::panic::catch_unwind(|| {
            let a = Obb::from_frame(p.ca, &frame_of(p.qa), p.ha);
            let b = Obb::from_frame(p.cb, &frame_of(p.qb), p.hb);
            match box_box_classify(&a, &b, A, B, p.hint) {
                BoxBoxOutcome::Contact(c) => (Some(contact_words(&c)), None),
                BoxBoxOutcome::Separated(axis) => (None, Some(axis)),
                BoxBoxOutcome::StillSeparated(_) => {
                    panic!("box_box_classify reads no carried axis")
                }
                BoxBoxOutcome::NoContact => (None, None),
            }
        });
        let wrapper = std::panic::catch_unwind(|| {
            box_box_contact(A, B, p.ca, p.qa, p.ha, p.cb, p.qb, p.hb, p.hint)
                .map(|c| contact_words(&c))
        });
        let (oracle, (kernel, separated), wrapper) = match (oracle, kernel, wrapper) {
            (Ok(o), Ok(k), Ok(w)) => (o, k, w),
            (Err(_), Err(_), Err(_)) => return Ok(L9aSeen::BothPanicked),
            (o, k, w) => {
                return Err(format!(
                    "panic mismatch: oracle panicked {}, classify {}, box_box_contact {}; {p:?}",
                    o.is_err(),
                    k.is_err(),
                    w.is_err()
                ));
            }
        };
        if kernel != oracle || wrapper != oracle {
            return Err(format!(
                "outcome differs from the pre-L9 oracle: oracle {oracle:?}, classify {kernel:?} \
                 (separated on {separated:?}), box_box_contact {wrapper:?}; {p:?}"
            ));
        }
        let (a, b) = (pre_l9::obb(p.ca, p.qa, p.ha), pre_l9::obb(p.cb, p.qb, p.hb));
        let cands = candidates_of(&a, &b);
        let first_negative = cands.iter().position(|c| matches!(c, Some(c) if c.depth < 0.0));
        match separated {
            Some(axis) => {
                if first_negative != Some(usize::from(axis)) {
                    return Err(format!(
                        "separated on axis {axis}, but the first negative axis in canonical \
                         order is {first_negative:?}; {p:?}"
                    ));
                }
                Ok(L9aSeen::Separated)
            }
            None if first_negative.is_some() => Err(format!(
                "axis {first_negative:?} is negative but classify did not report the pair \
                 separated; {p:?}"
            )),
            None if kernel.is_some() => {
                let zero = cands.iter().flatten().any(|c| c.depth == 0.0);
                Ok(if zero { L9aSeen::ContactAtZeroDepth } else { L9aSeen::Contact })
            }
            None => Ok(L9aSeen::NoContact),
        }
    }

    /// G-L9a-1 (`levers/L9-contact-reuse/02-DESIGN-REV1.md`, commit C1): the SAT's early exit
    /// and the frame-built boxes give the pre-L9 kernel's answer bit for bit — the manifold, the
    /// axis to persist, or no contact — and a separated pair is reported on its first negative
    /// axis. Three arms: arbitrary poses; exactly touching integer lattices (identity or half-turn
    /// orientations, integer centres and half-extents, the offset along one axis exactly the sum
    /// of the half-extents, and every other offset an integer, so faces, edges and corners meet
    /// at depth exactly zero); and degenerate extents (a zero half-extent on some axes, where the
    /// kernel's reference-face guard may panic in debug and must then panic on both sides). Every
    /// case draws a random hint.
    ///
    /// Mutations recorded red (C1's red-first log): M-a1, the early exit on `depth <= 0.0`,
    /// calls the lattice's zero-depth contacts separated; M-a2, `RowFrame::axes_of` storing the rows of
    /// `Mat3::from_quat` instead of its columns, transposes every rotated box.
    #[test]
    #[cfg(not(miri))]
    fn l9a_classify_equals_the_pre_l9_kernel() {
        use proptest::prelude::*;
        use std::cell::Cell;

        let seen = Cell::new([0u64; 5]);
        let bump = |s: L9aSeen| {
            let mut counts = seen.get();
            counts[s as usize] += 1;
            seen.set(counts);
        };
        let config = ProptestConfig { cases: 2048, failure_persistence: None, ..ProptestConfig::default() };
        proptest!(config, |(seed in any::<u64>(), arm in 0u8..3)| {
            let mut d = Draw(seed | 1);
            let pose = match arm {
                0 => {
                    // A pile's coordinates (tens of metres), B within reach of A.
                    let ca = Vec3::new(d.range(-30.0, 30.0), d.range(0.0, 30.0), d.range(-30.0, 30.0));
                    L9aPose {
                        ca,
                        qa: d.rotation(),
                        ha: Vec3::new(d.range(0.2, 1.5), d.range(0.2, 1.5), d.range(0.2, 1.5)),
                        cb: ca + Vec3::new(d.range(-2.5, 2.5), d.range(-2.5, 2.5), d.range(-2.5, 2.5)),
                        qb: d.rotation(),
                        hb: Vec3::new(d.range(0.2, 1.5), d.range(0.2, 1.5), d.range(0.2, 1.5)),
                        hint: d.hint(),
                    }
                }
                1 => {
                    let ha = Vec3::new(d.int(1, 3), d.int(1, 3), d.int(1, 3));
                    let hb = Vec3::new(d.int(1, 3), d.int(1, 3), d.int(1, 3));
                    let sum = [ha.x + hb.x, ha.y + hb.y, ha.z + hb.z];
                    let touch = d.below(3) as usize;
                    let mut off = [0.0f32; 3];
                    for (i, o) in off.iter_mut().enumerate() {
                        let s = sum[i] as i32;
                        *o = if i == touch {
                            if d.below(2) == 0 { sum[i] } else { -sum[i] }
                        } else {
                            d.int(-s - 1, s + 1)
                        };
                    }
                    let ca = Vec3::new(d.int(-40, 40), d.int(0, 40), d.int(-40, 40));
                    L9aPose {
                        ca,
                        qa: d.exact_rotation(),
                        ha,
                        cb: ca + Vec3::new(off[0], off[1], off[2]),
                        qb: d.exact_rotation(),
                        hb,
                        hint: d.hint(),
                    }
                }
                _ => {
                    let ha = d.degenerate_half();
                    let hb = d.degenerate_half();
                    L9aPose {
                        ca: Vec3::new(d.range(-1.5, 1.5), d.range(-1.5, 1.5), d.range(-1.5, 1.5)),
                        qa: if d.below(2) == 0 { d.exact_rotation() } else { d.rotation() },
                        ha,
                        cb: Vec3::ZERO,
                        qb: if d.below(2) == 0 { d.exact_rotation() } else { d.rotation() },
                        hb,
                        hint: d.hint(),
                    }
                }
            };
            match l9a_case(pose) {
                Ok(s) => bump(s),
                Err(msg) => prop_assert!(false, "arm {}, seed {:#x}: {}", arm, seed, msg),
            }
        });
        let [contact, zero, separated, no_contact, panicked] = seen.get();
        println!(
            "G-L9a-1 coverage: contact {contact}, contact at zero depth {zero}, separated \
             {separated}, no contact {no_contact}, both panicked {panicked}"
        );
        assert!(contact > 0, "no case produced a contact");
        assert!(zero > 0, "no case produced a contact at depth exactly zero (the M-a1 witness)");
        assert!(separated > 0, "no case was separated");
    }

    // ── G-L9a-3: the carried separating axis against the pre-L9 oracle ───────────────────────

    /// The raw (unnormalised) axis of canonical index `axis` on boxes `a` and `b`: the face column,
    /// or the edge cross product before [`eval_axis`] normalises it.
    fn raw_axis(a: &Obb, b: &Obb, axis: usize) -> Vec3 {
        if axis < 3 {
            a.axes[axis]
        } else if axis < 6 {
            b.axes[axis - 3]
        } else {
            let (ea, eb) = ((axis - 6) / 3, (axis - 6) % 3);
            a.axes[ea].cross(b.axes[eb])
        }
    }

    /// The depth mutation M-a5 would test: the separation along the RAW axis, with no
    /// normalisation and no degeneracy guard. Mathematically it has the normalised depth's sign;
    /// in `f32` the two roundings can disagree at a knife edge.
    fn raw_depth(a: &Obb, b: &Obb, axis: usize) -> f32 {
        let raw = raw_axis(a, b, axis);
        let delta = b.center - a.center;
        a.projection_radius(raw) + b.projection_radius(raw) - delta.dot(raw).abs()
    }

    /// What one G-L9a-3 case exercised.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum L9a3Seen {
        /// The carried axis still separated the boxes: the SAT did not run.
        Hit,
        /// A carried axis that no longer separates, then the SAT separated the pair.
        StaleThenSeparated,
        /// A carried axis that no longer separates, then a contact.
        StaleThenContact,
        /// No carried axis.
        NoCarry,
        /// Both sides panicked (a degenerate reference face's debug assertion).
        BothPanicked,
    }

    /// Witnesses the two mutations need, counted per case.
    #[derive(Clone, Copy, Debug, Default)]
    struct L9a3Witness {
        /// A contact whose carried axis has depth exactly zero (M-a4 calls it separated).
        zero_depth_carried: u64,
        /// A contact whose carried axis has a non-negative normalised depth and a negative raw
        /// one (M-a5 calls it separated).
        raw_sign_flip_carried: u64,
    }

    /// One G-L9a-3 case: [`box_box_classify_carried`] on frame-built boxes with a carried axis
    /// `sep` against the pre-L9 oracle (which carries nothing) — the contact's words and axis, or
    /// no contact; a hit must name an axis whose depth is negative on the oracle's boxes.
    fn l9a3_case(p: L9aPose, sep: Option<u8>, w: &mut L9a3Witness) -> Result<L9a3Seen, String> {
        let oracle = std::panic::catch_unwind(|| {
            pre_l9::box_box_contact(A, B, p.ca, p.qa, p.ha, p.cb, p.qb, p.hb, p.hint)
                .map(|c| contact_words(&c))
        });
        let kernel = std::panic::catch_unwind(|| {
            let a = Obb::from_frame(p.ca, &frame_of(p.qa), p.ha);
            let b = Obb::from_frame(p.cb, &frame_of(p.qb), p.hb);
            match box_box_classify_carried(&a, &b, A, B, sep, || p.hint) {
                BoxBoxOutcome::Contact(c) => (Some(contact_words(&c)), None, false),
                BoxBoxOutcome::Separated(axis) => (None, Some(axis), false),
                BoxBoxOutcome::StillSeparated(axis) => (None, Some(axis), true),
                BoxBoxOutcome::NoContact => (None, None, false),
            }
        });
        let (oracle, (kernel, separated, hit)) = match (oracle, kernel) {
            (Ok(o), Ok(k)) => (o, k),
            (Err(_), Err(_)) => return Ok(L9a3Seen::BothPanicked),
            (o, k) => {
                return Err(format!(
                    "panic mismatch: oracle panicked {}, carried classify {}; sep {sep:?}, {p:?}",
                    o.is_err(),
                    k.is_err()
                ));
            }
        };
        if kernel != oracle {
            return Err(format!(
                "outcome differs from the pre-L9 oracle: oracle {oracle:?}, carried classify \
                 {kernel:?} (separated on {separated:?}, hit {hit}); sep {sep:?}, {p:?}"
            ));
        }
        let (a, b) = (pre_l9::obb(p.ca, p.qa, p.ha), pre_l9::obb(p.cb, p.qb, p.hb));
        let cands = candidates_of(&a, &b);
        if hit {
            let axis = separated.expect("invariant: a hit names its axis");
            if sep != Some(axis) || !matches!(cands[usize::from(axis)], Some(c) if c.depth < 0.0) {
                return Err(format!(
                    "a hit on axis {axis} (carried {sep:?}) whose oracle depth is {:?}; {p:?}",
                    cands[usize::from(axis)].map(|c| c.depth)
                ));
            }
            return Ok(L9a3Seen::Hit);
        }
        let Some(s) = sep else {
            return Ok(L9a3Seen::NoCarry);
        };
        let s = usize::from(s);
        if oracle.is_some() {
            let depth = cands[s].map(|c| c.depth);
            w.zero_depth_carried += u64::from(depth == Some(0.0));
            let flips = matches!(depth, Some(d) if d >= 0.0) && raw_depth(&a, &b, s) < 0.0;
            w.raw_sign_flip_carried += u64::from(flips);
            Ok(L9a3Seen::StaleThenContact)
        } else {
            Ok(L9a3Seen::StaleThenSeparated)
        }
    }

    /// The largest centre offset `t` along `dir` (from `lo`, which overlaps, towards `hi`, which
    /// does not) at which candidate `axis` of the oracle's boxes still has depth `>= 0`: a
    /// bisection over `f32`, so the pair lands on the knife edge of that axis.
    fn knife_edge(p: &L9aPose, dir: Vec3, axis: usize, mut lo: f32, mut hi: f32) -> f32 {
        let depth = |t: f32| {
            let a = pre_l9::obb(p.ca, p.qa, p.ha);
            let b = pre_l9::obb(p.ca + dir * t, p.qb, p.hb);
            candidates_of(&a, &b)[axis].map_or(f32::NAN, |c| c.depth)
        };
        for _ in 0..80 {
            let mid = 0.5 * (lo + hi);
            if mid == lo || mid == hi {
                break;
            }
            if depth(mid) >= 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// G-L9a-3 (`levers/L9-contact-reuse/02-DESIGN-REV1.md`, commit C2): a box pair's carried
    /// separating axis, arbitrary and mostly STALE, never changes the answer —
    /// [`box_box_classify_carried`] on frame-built boxes returns the pre-L9 kernel's contact bit
    /// for bit, or no contact, and a pair it reports separated without the SAT has a negative
    /// depth on the carried axis. Four arms, each with a random hint: arbitrary poses (the carried
    /// axis the pair's own first negative axis half the time, any axis or none otherwise);
    /// exactly touching integer lattices (the carried axis the touching face half the time);
    /// knife-edge face contacts on arbitrary orientations (the offset bisected in `f32` to the
    /// last one whose depth on the carried face is not negative); degenerate extents.
    ///
    /// Mutations recorded red (C2's red-first log): M-a4, the carried check accepting
    /// `depth <= 0`, calls the lattice's zero-depth contacts separated; M-a5, the carried check
    /// on the raw (unnormalised) axis, calls a knife-edge contact separated where the two
    /// roundings disagree in sign. Each has a witness counted here, so the gate cannot pass
    /// vacuously.
    #[test]
    #[cfg(not(miri))]
    fn l9a_carried_separating_axis_equals_the_pre_l9_kernel() {
        use proptest::prelude::*;
        use std::cell::Cell;

        let seen = Cell::new([0u64; 5]);
        let witness = Cell::new(L9a3Witness::default());
        let config = ProptestConfig { cases: 4096, failure_persistence: None, ..ProptestConfig::default() };
        proptest!(config, |(seed in any::<u64>(), arm in 0u8..4)| {
            let mut d = Draw(seed | 1);
            let random_sep = |d: &mut Draw| {
                let s = d.below(SAT_AXES as u64 + 1) as u8;
                (usize::from(s) < SAT_AXES).then_some(s)
            };
            let (pose, sep) = match arm {
                0 => {
                    let ca = Vec3::new(d.range(-30.0, 30.0), d.range(0.0, 30.0), d.range(-30.0, 30.0));
                    let pose = L9aPose {
                        ca,
                        qa: d.rotation(),
                        ha: Vec3::new(d.range(0.2, 1.5), d.range(0.2, 1.5), d.range(0.2, 1.5)),
                        cb: ca + Vec3::new(d.range(-2.5, 2.5), d.range(-2.5, 2.5), d.range(-2.5, 2.5)),
                        qb: d.rotation(),
                        hb: Vec3::new(d.range(0.2, 1.5), d.range(0.2, 1.5), d.range(0.2, 1.5)),
                        hint: d.hint(),
                    };
                    let first_negative = {
                        let (a, b) = (pre_l9::obb(pose.ca, pose.qa, pose.ha), pre_l9::obb(pose.cb, pose.qb, pose.hb));
                        candidates_of(&a, &b).iter().position(|c| matches!(c, Some(c) if c.depth < 0.0))
                    };
                    let sep = match first_negative {
                        Some(s) if d.below(2) == 0 => Some(s as u8),
                        _ => random_sep(&mut d),
                    };
                    (pose, sep)
                }
                1 => {
                    let ha = Vec3::new(d.int(1, 3), d.int(1, 3), d.int(1, 3));
                    let hb = Vec3::new(d.int(1, 3), d.int(1, 3), d.int(1, 3));
                    let sum = [ha.x + hb.x, ha.y + hb.y, ha.z + hb.z];
                    let touch = d.below(3) as usize;
                    let mut off = [0.0f32; 3];
                    for (i, o) in off.iter_mut().enumerate() {
                        let s = sum[i] as i32;
                        *o = if i == touch {
                            if d.below(2) == 0 { sum[i] } else { -sum[i] }
                        } else {
                            d.int(-s - 1, s + 1)
                        };
                    }
                    let ca = Vec3::new(d.int(-40, 40), d.int(0, 40), d.int(-40, 40));
                    let pose = L9aPose {
                        ca,
                        qa: d.exact_rotation(),
                        ha,
                        cb: ca + Vec3::new(off[0], off[1], off[2]),
                        qb: d.exact_rotation(),
                        hb,
                        hint: d.hint(),
                    };
                    // Under a half turn a box's face `touch` is still the world axis `touch`.
                    let sep = match d.below(4) {
                        0 => Some(touch as u8),
                        1 => Some(3 + touch as u8),
                        _ => random_sep(&mut d),
                    };
                    (pose, sep)
                }
                2 => {
                    let mut pose = L9aPose {
                        ca: Vec3::new(d.range(-30.0, 30.0), d.range(0.0, 30.0), d.range(-30.0, 30.0)),
                        qa: d.rotation(),
                        ha: Vec3::new(d.range(0.3, 1.5), d.range(0.3, 1.5), d.range(0.3, 1.5)),
                        cb: Vec3::ZERO,
                        qb: if d.below(2) == 0 { Quat::IDENTITY } else { d.rotation() },
                        hb: Vec3::new(d.range(0.3, 1.5), d.range(0.3, 1.5), d.range(0.3, 1.5)),
                        hint: d.hint(),
                    };
                    if d.below(2) == 0 {
                        pose.qb = pose.qa;
                    }
                    // B above A's face `i`, a little off its centre, pushed out along the face
                    // normal to the knife edge of that face's axis.
                    let a = pre_l9::obb(pose.ca, pose.qa, pose.ha);
                    let b0 = pre_l9::obb(Vec3::ZERO, pose.qb, pose.hb);
                    let i = d.below(3) as usize;
                    let (j, k) = ((i + 1) % 3, (i + 2) % 3);
                    let lateral = a.axes[j] * d.range(-0.3, 0.3) * a.half[j]
                        + a.axes[k] * d.range(-0.3, 0.3) * a.half[k];
                    let n = a.axes[i];
                    let reach = a.half[i] + b0.projection_radius(n);
                    let dir = n + lateral * (1.0 / reach);
                    let t = knife_edge(&pose, dir, i, 0.9 * reach, 1.1 * reach);
                    pose.cb = pose.ca + dir * t;
                    (pose, Some(i as u8))
                }
                _ => {
                    let ha = d.degenerate_half();
                    let hb = d.degenerate_half();
                    let pose = L9aPose {
                        ca: Vec3::new(d.range(-1.5, 1.5), d.range(-1.5, 1.5), d.range(-1.5, 1.5)),
                        qa: if d.below(2) == 0 { d.exact_rotation() } else { d.rotation() },
                        ha,
                        cb: Vec3::ZERO,
                        qb: if d.below(2) == 0 { d.exact_rotation() } else { d.rotation() },
                        hb,
                        hint: d.hint(),
                    };
                    (pose, random_sep(&mut d))
                }
            };
            let mut w = witness.get();
            match l9a3_case(pose, sep, &mut w) {
                Ok(s) => {
                    let mut counts = seen.get();
                    counts[s as usize] += 1;
                    seen.set(counts);
                    witness.set(w);
                }
                Err(msg) => prop_assert!(false, "arm {}, seed {:#x}: {}", arm, seed, msg),
            }
        });
        let [hit, stale_separated, stale_contact, no_carry, panicked] = seen.get();
        let w = witness.get();
        println!(
            "G-L9a-3 coverage: hit {hit}, stale then separated {stale_separated}, stale then \
             contact {stale_contact}, no carry {no_carry}, both panicked {panicked}; {w:?}"
        );
        assert!(hit > 0, "no carried axis still separated its pair");
        assert!(stale_separated > 0, "no stale carried axis fell back to a separated SAT");
        assert!(stale_contact > 0, "no stale carried axis fell back to a contact");
        assert!(w.zero_depth_carried > 0, "no contact carried an axis of depth exactly zero (the M-a4 witness)");
        assert!(
            w.raw_sign_flip_carried > 0,
            "no contact carried an axis whose raw depth is negative (the M-a5 witness)"
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
