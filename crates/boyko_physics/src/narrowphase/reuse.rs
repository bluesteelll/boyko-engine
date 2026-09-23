//! Contact reuse (L9, `docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md`).
//!
//! Two things live here: the per-row orientation frame (D2), and the reuse record with the
//! functions that build, check and refresh it (L9b, D4–D8).
//!
//! # The frame fill (D2, ruling O3)
//!
//! [`fill_row_frames`] runs on the calling thread at the entry of each narrowphase path
//! (`narrowphase_serial_with`, and `dispatch::prepare` before any chunk runs), so it runs once per
//! step and for every direct caller of either path. It writes the frame of every BOX row and skips
//! every other row, because no pair reads a non-box row through a frame. A frame's `radius` is
//! written only on a step that requested contact reuse, its one reader. The fill writes nothing
//! and returns `None` in two cases, and the step's box pairs then build their two frames per pair
//! with the same [`RowFrame::axes_of`]:
//!
//! * the step has fewer candidate pairs than half its rows. The fill costs one conversion per box
//!   row, and the pairs save at most two per pair, so on such a sparse step the fill would cost more
//!   than it saves (ruling O3: the fill skips the rows no pair reads);
//! * the step has more rows than the column's reserve (the stage ceiling's rule in
//!   `narrowphase/dispatch.rs`: a resize past it panics).
//!
//! # Why a frame gives today's bits (Lemma L9-L2, second half)
//!
//! [`RowFrame::axes_of`] is the axis computation `Obb::new` performed per pair, moved: the three
//! columns of `Mat3::from_quat(rotation)`. The fill and the per-pair path call it on the same
//! gathered rotation, so `Obb::from_frame` receives the bits `Obb::new` computed. The radius is
//! `half_extents.length()`, the function the per-pair path calls. The arithmetic is IEEE `f32` add
//! and multiply only, and Rust does not contract them into FMA.
//!
//! # Contact reuse (L9b)
//!
//! A touching box pair whose relative pose moved less than τ_eff since its last full collision
//! reuses that collision's contact features. [`PhysicsConfig::contact_reuse`] turns it on; it is
//! off by default, and then no function below runs.
//!
//! * **Who (D3, D8).** Both shapes boxes, neither body a sensor, τ_eff positive, and the pair slow
//!   ([`is_fast`] false). Every other pair takes today's path.
//! * **The record (D5, D9).** A slow pair whose full collision produced a contact stores a
//!   [`ReuseRecord`] at its stream slot ([`build`]): the pose of the smaller body S in the frame of
//!   the larger body F, the reference face and the kept incident points in the incident body's
//!   frame (a face contact), or the two edge axes (an edge contact). Its tag carries `REC`.
//! * **The criterion (D4).** The next step joins the record through the pair carry and keeps it
//!   iff the shapes are bitwise the record's and [`criterion`] bounds the relative motion by
//!   τ_eff, measured in F's frame and scaled by S's extent.
//! * **The refresh (D5, D6).** A kept record is refreshed from the current poses
//!   ([`refresh`]): each face point is carried with the incident body and its separation re-measured
//!   against the reference face, a point that lifted off is dropped; an edge record re-evaluates its
//!   edge axis exactly as the SAT would and takes today's edge contact on it.
//! * **A miss (D7)** — no record, the shapes changed, the criterion failed, or the edge axis
//!   degenerated — runs today's full collision, builds a new record from it and emits the REFRESH of
//!   that record, never the raw collision. So a slow pair's output is a pure function of (record,
//!   poses), which is lemma L9-L1: equal poses give equal output bits, whatever the history.
//!
//! # The records survive a row-order flip (ruling W2)
//!
//! A record names its bodies by role only where the roles matter: the shape pin (`half_a`,
//! `half_b`), which body is the reference face's (`REF_IS_B`), which body is F on a bitwise radius
//! tie (`F_IS_B`) and an edge record's axis pair. [`ReuseRecord::flipped`] exchanges exactly those,
//! so a pair whose two rows swapped order keeps its record; the pose is F's and S's, the incident
//! points are the incident body's, and a face feature id names a face and a corner of physical
//! bodies.
//!
//! ZERO `unsafe`; every column is a kernel `ScratchColumn` owned by
//! [`Manifolds`](crate::resources::Manifolds) (principle 0), grown in place, no per-step heap
//! allocation.
//!
//! [`PhysicsConfig::contact_reuse`]: crate::resources::PhysicsConfig::contact_reuse

use boyko_ecs::ecs::core::component::scratch::ScratchColumn;

use crate::components::ColliderShape;
use crate::manifold::{BodyIndex, ContactPoint, Manifold};
use crate::math::{MAX_CONTACT_POINTS, Mat3, Quat, Vec3};
use crate::narrowphase::box_box::{
    BoxBoxContact, EdgeRefresh, FeatureRef, Obb, contact_feature, refresh_edge,
};
use crate::narrowphase::carry::PairTag;
use crate::resources::BodyState;

/// The fraction of the smaller bounding radius, and of the thinnest half-extent, that bounds τ_eff
/// (D4, ruling O1): 5 %, so the deepest-point bound `2·τ_eff` stays a tenth of the thinnest box's
/// half-thickness.
pub(crate) const TAU_EFF_FRACTION: f32 = 0.05;

/// A box row's orientation resolved into world axes for one step (D2): `axes[i]` is the world
/// direction of the box's local axis `i`, column `i` of `Mat3::from_quat(rotation)`, and `radius`
/// its bounding radius `|half_extents|`.
///
/// 40 B, `#[repr(C)]`, POD. The radius is written only on a step that requested contact reuse, its
/// one reader (the choice of the larger body F and τ_eff, D4).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RowFrame {
    /// The world directions of local axes x, y and z.
    pub(crate) axes: [Vec3; 3],
    /// The box's bounding radius, `half_extents.length()`.
    pub(crate) radius: f32,
}

const _: () = assert!(size_of::<RowFrame>() == 40);

impl RowFrame {
    /// The value of a slot the fill has not written this step (a grown slot, a non-box row). No
    /// pair reads it: only a box-box pair reads a frame, and only its own two box rows'.
    pub(crate) const UNWRITTEN: Self = Self { axes: [Vec3::ZERO; 3], radius: 0.0 };

    /// The world axes of a body with orientation `rotation`: the columns of `Mat3::from_quat`.
    #[inline]
    pub(crate) fn axes_of(rotation: Quat) -> [Vec3; 3] {
        let r = Mat3::from_quat(rotation);
        // Column `i` of R is the world direction of local axis `i`. With row-major storage,
        // column `i` is `(rows[0][i], rows[1][i], rows[2][i])`.
        [
            Vec3::new(r.rows[0].x, r.rows[1].x, r.rows[2].x),
            Vec3::new(r.rows[0].y, r.rows[1].y, r.rows[2].y),
            Vec3::new(r.rows[0].z, r.rows[1].z, r.rows[2].z),
        ]
    }
}

/// Writes the frame of every box row of `bodies` into `frames` and returns the column's read slice,
/// one slot per row; or writes nothing and returns `None` when the step's box pairs build their
/// frames per pair instead (module docs, "The frame fill"). `n_pairs` is the step's candidate pair
/// count; `radius` is whether the step requested contact reuse, the radius's one reader.
///
/// Serial, on the calling thread, before any pair of the step is collided; the returned slice is
/// read-only while the pairs run, on any thread.
pub(crate) fn fill_row_frames<'a>(
    frames: &'a mut ScratchColumn<RowFrame>,
    bodies: &[BodyState],
    n_pairs: usize,
    radius: bool,
) -> Option<&'a [RowFrame]> {
    let rows = bodies.len();
    if n_pairs.saturating_mul(2) < rows || rows > frames.capacity() {
        return None;
    }
    {
        // The view publishes its length on drop, before the read slice below is taken.
        let mut view = frames.build_view();
        view.resize(rows, RowFrame::UNWRITTEN);
        for (frame, body) in view.as_mut_slice().iter_mut().zip(bodies) {
            if let ColliderShape::Box { half_extents } = body.shape {
                frame.axes = RowFrame::axes_of(body.rotation);
                if radius {
                    frame.radius = half_extents.length();
                }
            }
        }
    }
    Some(frames.as_read_slice())
}

/// The step's contact-reuse parameters (D8, D11; ruling W1) and the pair carry's parity, `Copy`,
/// shared by the serial loop and every chunk.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ReuseStep {
    /// Whether L9b runs this step: [`PhysicsConfig::contact_reuse`], and the pair carry can hold
    /// a record for every candidate pair.
    ///
    /// [`PhysicsConfig::contact_reuse`]: crate::resources::PhysicsConfig::contact_reuse
    pub(crate) on: bool,
    /// τ, the reuse distance in metres (`PhysicsConfig::contact_reuse_distance`).
    pub(crate) tau: f32,
    /// The step's `dt²`, for the fast predicate (D8).
    pub(crate) dt2: f32,
    /// Whether the hysteresis table's key set changed this step (rows moved, a Reset, a clear or
    /// a grow), so a hit re-keys its pair's entry with its record's axis (ruling W1).
    pub(crate) rekey: bool,
    /// The step's parity (`PairCarry::open`): the records it writes carry it, the ones it reads
    /// the other.
    pub(crate) parity: bool,
}

impl ReuseStep {
    /// Contact reuse off: every pair takes today's path.
    pub(crate) const OFF: Self = Self { on: false, tau: 0.0, dt2: 0.0, rekey: false, parity: false };

    /// The parameters of a step with `dt`, the configuration's switch and distance `tau`, and
    /// whether the hysteresis table's key set changed.
    #[inline]
    pub(crate) fn new(on: bool, tau: f32, dt: f32, rekey: bool) -> Self {
        debug_assert!(
            !on || (tau.is_finite() && tau >= 0.0),
            "invariant: PhysicsConfig::contact_reuse_distance is finite and >= 0"
        );
        Self { on, tau, dt2: dt * dt, rekey, parity: false }
    }
}

/// What a pair's join found in the previous step: its tag in the current roles, and its record
/// when the tag says it wrote one and this step reuses (L9 D9).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Prev<'a> {
    /// The previous step's tag, flipped into the current roles.
    pub(crate) tag: PairTag,
    /// The previous step's record for the pair, as stored (in the previous roles), iff the tag
    /// carries `REC` and this step reuses.
    pub(crate) record: Option<&'a ReuseRecord>,
    /// Whether the pair's two rows swapped order since: the record must be [flipped]
    /// (ReuseRecord::flipped) before it is read.
    pub(crate) flipped: bool,
}

impl Prev<'static> {
    /// No join: no tag, no record.
    pub(crate) const NONE: Self =
        Self { tag: PairTag::NONE, record: None, flipped: false };
}

/// A slow box pair's reuse record (D5, D9): what its last full collision found, in a form the
/// next steps can refresh from the current poses.
///
/// 128 B, two cache lines, POD, every byte a named field (the two pads are written zero).
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReuseRecord {
    /// S's centre in F's frame at the last full collision.
    d_ref: Vec3,
    /// Kept features: `1..=4` incident points (face), `1` (edge).
    count: u8,
    /// [`REF_IS_B`] | [`REF_NEG`] | [`EDGE`] | [`F_IS_B`] | [`PARITY`].
    flags: u8,
    /// The reference face's local axis `0..3` (face), or `ea << 2 | eb` (edge).
    feat: u8,
    /// Zero.
    _p0: u8,
    /// `q_F* ⊗ q_S` at the last full collision.
    q_ref: Quat,
    /// Body A's half-extents, compared bitwise.
    half_a: Vec3,
    /// Body B's half-extents, compared bitwise.
    half_b: Vec3,
    /// The kept incident points in the incident body's frame (face).
    lp: [Vec3; MAX_CONTACT_POINTS],
    /// The kept points' feature ids, carried unchanged (face).
    feature: [u32; MAX_CONTACT_POINTS],
    /// Zero.
    _p1: [u32; 2],
}

const _: () = assert!(size_of::<ReuseRecord>() == 128 && align_of::<ReuseRecord>() == 16);

/// Record flag: the reference face is body B's.
const REF_IS_B: u8 = 1 << 0;
/// Record flag: the reference face is the NEGATIVE side of its local axis.
const REF_NEG: u8 = 1 << 1;
/// Record flag: an edge-edge record.
const EDGE: u8 = 1 << 2;
/// Record flag: F is body B (decides a bitwise radius tie).
const F_IS_B: u8 = 1 << 3;
/// Record flag: the parity of the step that wrote the record (a debug check that a `REC` tag's
/// record was written in the step before).
const PARITY: u8 = 1 << 7;

impl ReuseRecord {
    /// A record no step wrote: the fill of a grown slot. No pair reads it (`REC` ⇒ written in the
    /// same step).
    pub(crate) const UNWRITTEN: Self = Self {
        d_ref: Vec3::ZERO,
        count: 0,
        flags: 0,
        feat: 0,
        _p0: 0,
        q_ref: Quat::IDENTITY,
        half_a: Vec3::ZERO,
        half_b: Vec3::ZERO,
        lp: [Vec3::ZERO; MAX_CONTACT_POINTS],
        feature: [0; MAX_CONTACT_POINTS],
        _p1: [0; 2],
    };

    /// This record as seen by the pair with bodies A and B exchanged (ruling W2): the shape pin,
    /// the reference side, F's side on a tie and an edge record's axis pair exchange; the pose,
    /// the reference face and the incident points belong to physical bodies and do not.
    #[inline]
    pub(crate) fn flipped(self) -> Self {
        let mut r = self;
        core::mem::swap(&mut r.half_a, &mut r.half_b);
        r.flags ^= REF_IS_B | F_IS_B;
        if r.flags & EDGE != 0 {
            r.feat = ((r.feat & 3) << 2) | (r.feat >> 2);
        }
        r
    }

    /// Whether this is an edge-edge record.
    #[inline]
    pub(crate) fn is_edge(&self) -> bool {
        self.flags & EDGE != 0
    }

    /// The parity of the step that wrote it.
    #[inline]
    pub(crate) fn parity(&self) -> bool {
        self.flags & PARITY != 0
    }

    /// This record, marked as written by a step of parity `parity`.
    #[inline]
    pub(crate) fn with_parity(mut self, parity: bool) -> Self {
        self.flags = (self.flags & !PARITY) | if parity { PARITY } else { 0 };
        self
    }

    /// Every field as `u32` words, for fingerprints and bitwise comparisons (the two pads
    /// included).
    pub(crate) fn words(&self) -> [u32; 32] {
        let mut w = [0u32; 32];
        let v = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
        w[0..3].copy_from_slice(&v(self.d_ref));
        w[3] = u32::from_le_bytes([self.count, self.flags, self.feat, self._p0]);
        w[4..8].copy_from_slice(&[self.q_ref.x, self.q_ref.y, self.q_ref.z, self.q_ref.w].map(f32::to_bits));
        w[8..11].copy_from_slice(&v(self.half_a));
        w[11..14].copy_from_slice(&v(self.half_b));
        for (i, p) in self.lp.iter().enumerate() {
            w[14 + 3 * i..17 + 3 * i].copy_from_slice(&v(*p));
        }
        w[26..30].copy_from_slice(&self.feature);
        w[30..32].copy_from_slice(&self._p1);
        w
    }
}

/// Whether two vectors hold the same bits.
#[inline]
fn same_bits(x: Vec3, y: Vec3) -> bool {
    x.x.to_bits() == y.x.to_bits() && x.y.to_bits() == y.y.to_bits() && x.z.to_bits() == y.z.to_bits()
}

/// `v` in the frame whose world axes are `axes`: `Rᵀ v`.
#[inline]
fn to_frame(axes: &[Vec3; 3], v: Vec3) -> Vec3 {
    Vec3::new(axes[0].dot(v), axes[1].dot(v), axes[2].dot(v))
}

/// `v` given in the frame whose world axes are `axes`, in world: `R v`.
#[inline]
fn from_frame(axes: &[Vec3; 3], v: Vec3) -> Vec3 {
    axes[0] * v.x + axes[1] * v.y + axes[2] * v.z
}

/// A box pair's reuse geometry for one step: both bounding radii and τ_eff (D4, ruling O1).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PairGeom {
    /// Body A's bounding radius.
    pub(crate) ra: f32,
    /// Body B's bounding radius.
    pub(crate) rb: f32,
    /// `min(τ, 5 % of the smaller radius, 5 % of the thinnest half-extent of either box)`.
    pub(crate) tau_eff: f32,
}

impl PairGeom {
    /// The geometry of the box pair `(oa, ob)` with bounding radii `ra`, `rb` under reuse distance
    /// `tau`.
    #[inline]
    pub(crate) fn new(oa: &Obb, ob: &Obb, ra: f32, rb: f32, tau: f32) -> Self {
        let h_min = oa.half[0]
            .min(oa.half[1])
            .min(oa.half[2])
            .min(ob.half[0])
            .min(ob.half[1])
            .min(ob.half[2]);
        let tau_eff = tau.min(TAU_EFF_FRACTION * ra.min(rb)).min(TAU_EFF_FRACTION * h_min);
        Self { ra, rb, tau_eff }
    }

    /// Whether F is body B, deciding a bitwise radius tie by `tie`.
    #[inline]
    fn f_is_b(&self, tie: bool) -> bool {
        if self.ra == self.rb { tie } else { self.rb > self.ra }
    }
}

/// The fast predicate (D8): whether the pair's relative motion over one step can exceed half of
/// τ_eff, `3·dt²·(|v_S − v_F|² + |ω_F|²·|c_S − c_F|² + |ω_S − ω_F|²·(r_S + τ_eff)²) > (τ_eff/2)²`,
/// with F the larger body (body A on a tie). A NaN motion is fast. A pure function of the two
/// `BodyState`s, so equal states class a pair alike.
#[inline]
pub(crate) fn is_fast(ba: &BodyState, bb: &BodyState, g: &PairGeom, dt2: f32) -> bool {
    let (f, s, r_s) = if g.f_is_b(false) { (bb, ba, g.ra) } else { (ba, bb, g.rb) };
    let dv = s.linear_velocity - f.linear_velocity;
    let dw = s.angular_velocity - f.angular_velocity;
    let arm = r_s + g.tau_eff;
    let motion = dv.length_squared()
        + f.angular_velocity.length_squared() * (s.position - f.position).length_squared()
        + dw.length_squared() * (arm * arm);
    let half = 0.5 * g.tau_eff;
    let step = 3.0 * dt2 * motion;
    step > half * half || step.is_nan()
}

/// S's pose in F's frame: `(R_Fᵀ (c_S − c_F), q_F* ⊗ q_S)`. [`build`] and [`criterion`] both call
/// it, so a record checked on the poses it was built on reads `Δd = 0` and `s = 0` exactly.
#[inline]
fn relative_pose(f: &Obb, s: &Obb, qf: Quat, qs: Quat) -> (Vec3, Quat) {
    (to_frame(&f.axes, s.center - f.center), qf.conjugate().mul(qs))
}

/// The reuse criterion (D4, lemma L9-L3): the shapes are bitwise the record's and
/// `2·(|Δd|² + 4 s² (r_S + τ_eff)²) ≤ τ_eff²`, with `Δd` S's displacement in F's frame since the
/// record and `s = |vec(q_ref* ⊗ q)| = sin(Δθ/2)`. Since `(x + y)² ≤ 2(x² + y²)` it implies
/// `|Δd| + 2 s (r_S + τ_eff) ≤ τ_eff`: every point of S, and every point of F within
/// `r_S + τ_eff` of S's centre, moved by at most τ_eff relative to the other body.
///
/// `r` is in the current roles. F is the larger body, and on a bitwise radius tie the record's.
#[inline]
pub(crate) fn criterion(
    r: &ReuseRecord,
    oa: &Obb,
    ob: &Obb,
    qa: Quat,
    qb: Quat,
    g: &PairGeom,
) -> bool {
    if !same_bits(r.half_a, half_vec(oa)) || !same_bits(r.half_b, half_vec(ob)) {
        return false;
    }
    let f_is_b = g.f_is_b(r.flags & F_IS_B != 0);
    let (d, q, r_s) = if f_is_b {
        let (d, q) = relative_pose(ob, oa, qb, qa);
        (d, q, g.ra)
    } else {
        let (d, q) = relative_pose(oa, ob, qa, qb);
        (d, q, g.rb)
    };
    let dd = d - r.d_ref;
    let rel = r.q_ref.conjugate().mul(q);
    let s2 = rel.x * rel.x + rel.y * rel.y + rel.z * rel.z;
    let arm = r_s + g.tau_eff;
    2.0 * (dd.length_squared() + 4.0 * s2 * (arm * arm)) <= g.tau_eff * g.tau_eff
}

/// The half-extents of `obb` as a vector.
#[inline]
fn half_vec(obb: &Obb) -> Vec3 {
    Vec3::new(obb.half[0], obb.half[1], obb.half[2])
}

/// Builds the record of a slow pair's full collision `c` on the boxes `(oa, ob)` with orientations
/// `(qa, qb)` (D5, D9), marked with the step's `parity`. F is the larger body, body A on a tie.
///
/// Out of line: it runs once per slow miss, and the steady-state loop is the hit path.
#[inline(never)]
pub(crate) fn build(
    c: &BoxBoxContact,
    oa: &Obb,
    ob: &Obb,
    qa: Quat,
    qb: Quat,
    g: &PairGeom,
    parity: bool,
) -> ReuseRecord {
    let f_is_b = g.f_is_b(false);
    let (d_ref, q_ref) =
        if f_is_b { relative_pose(ob, oa, qb, qa) } else { relative_pose(oa, ob, qa, qb) };
    let mut r = ReuseRecord {
        d_ref,
        q_ref,
        half_a: half_vec(oa),
        half_b: half_vec(ob),
        flags: if f_is_b { F_IS_B } else { 0 },
        ..ReuseRecord::UNWRITTEN
    };
    let m = &c.manifold;
    match contact_feature(oa, ob, c) {
        FeatureRef::Face { ref_is_b, axis, positive } => {
            let inc = if ref_is_b { oa } else { ob };
            let count = usize::from(m.count);
            debug_assert!(
                (1..=MAX_CONTACT_POINTS).contains(&count),
                "invariant: a face contact keeps 1..=4 points"
            );
            for (i, p) in m.points[..count].iter().enumerate() {
                // The incident anchor is the clipped incident point itself (`face_contact`).
                let world = if ref_is_b { p.anchor_a } else { p.anchor_b };
                r.lp[i] = to_frame(&inc.axes, world - inc.center);
                r.feature[i] = p.feature_id;
            }
            r.count = m.count;
            r.feat = axis;
            r.flags |= if ref_is_b { REF_IS_B } else { 0 } | if positive { 0 } else { REF_NEG };
        }
        FeatureRef::Edge { ea, eb } => {
            r.count = 1;
            r.feat = (ea << 2) | eb;
            r.flags |= EDGE;
            r.feature[0] = m.points[0].feature_id;
        }
    }
    r.with_parity(parity)
}

/// What [`refresh`] made of a record on the current poses.
pub(crate) enum Refreshed {
    /// The refreshed manifold; a face record whose every point lifted off leaves `count == 0`
    /// (D6: no manifold, the record kept).
    Contact(Manifold),
    /// An edge record whose edge axis now separates the boxes, on this canonical axis: separated
    /// exactly, as the SAT would find (L9a).
    Separated(u8),
    /// An edge record whose edge axis degenerated: the pair misses.
    Degenerate,
}

/// Refreshes the record `r` (in the current roles) on the boxes `(oa, ob)` of the pair `(a, b)`
/// (D5, D6).
///
/// * **Face:** each kept point is carried with the incident body, `w = (c_I − c_R) + R_I·lp`, its
///   separation re-measured against the reference face, `sep = w·n − h` with
///   `n = ±R.axes[i]` (the bits `face_contact` names its reference normal with) and `h` the
///   reference half-extent; a point with `sep > 0` is dropped, as `face_contact` drops it (a NaN
///   is dropped too). The anchors are `p_I = c_R + w` and `p_R = p_I − n·sep`, and the normal
///   runs A→B. For an original incident corner `sep` is its exact distance to the current
///   reference plane.
/// * **Edge:** the edge axis re-evaluated as the SAT evaluates it, then today's edge contact.
#[inline]
pub(crate) fn refresh(r: &ReuseRecord, oa: &Obb, ob: &Obb, a: BodyIndex, b: BodyIndex) -> Refreshed {
    if r.is_edge() {
        let (ea, eb) = (usize::from(r.feat >> 2), usize::from(r.feat & 3));
        return match refresh_edge(oa, ob, ea, eb, a, b) {
            EdgeRefresh::Contact(m) => Refreshed::Contact(m),
            EdgeRefresh::Separated(axis) => Refreshed::Separated(axis),
            EdgeRefresh::Degenerate => Refreshed::Degenerate,
        };
    }
    Refreshed::Contact(refresh_face(r, oa, ob, a, b))
}

/// The face half of [`refresh`].
#[inline]
fn refresh_face(r: &ReuseRecord, oa: &Obb, ob: &Obb, a: BodyIndex, b: BodyIndex) -> Manifold {
    debug_assert!(
        (1..=MAX_CONTACT_POINTS).contains(&usize::from(r.count)) && r.feat < 3,
        "invariant: a face record keeps 1..=4 points on a local axis 0..3"
    );
    let ref_is_b = r.flags & REF_IS_B != 0;
    let (rf, inc) = if ref_is_b { (ob, oa) } else { (oa, ob) };
    let i = usize::from(r.feat);
    let n = rf.axes[i] * if r.flags & REF_NEG != 0 { -1.0 } else { 1.0 };
    debug_assert!(
        (n.length_squared() - 1.0).abs() <= 1.0e-5,
        "invariant: a refreshed face normal is a unit rotation column"
    );
    let h = rf.half[i];
    let dc = inc.center - rf.center;
    let mut m = Manifold::new(a, b);
    m.normal = if ref_is_b { n * -1.0 } else { n };
    let mut kept = 0usize;
    for (lp, &feature_id) in r.lp[..usize::from(r.count)].iter().zip(&r.feature) {
        let w = dc + from_frame(&inc.axes, *lp);
        let separation = w.dot(n) - h;
        if separation <= 0.0 {
            let on_incident = rf.center + w;
            let on_reference = on_incident - n * separation;
            let (anchor_a, anchor_b) =
                if ref_is_b { (on_incident, on_reference) } else { (on_reference, on_incident) };
            m.points[kept] = ContactPoint { anchor_a, anchor_b, separation, feature_id };
            kept += 1;
        }
    }
    m.count = kept as u8;
    m
}

#[cfg(test)]
mod tests {
    //! L9b's unit gates (commit C3, `levers/L9-contact-reuse/02-DESIGN-REV1.md`, "Gates"):
    //!
    //! * **G-L9b-1** — the criterion's soundness, a proptest over box pairs resting on a box or a
    //!   slab (either one body A or body B, tilted so the record keeps some corners and not others,
    //!   small and thin boxes included), each perturbed by a random relative rotation about the
    //!   moving box's centre and a translation, both reaching well past τ_eff, under a random common
    //!   motion. On every hit: (a) the refreshed separation of an original incident corner is its
    //!   exact distance to the current reference plane; (b) the full collision's deepest point is
    //!   at most `2·τ_eff` deeper than the refresh's; (c) every corner of the smaller body moved by
    //!   at most τ_eff in the larger body's frame (lemma L9-L3). Mutations M-b1 (the criterion
    //!   without its rotation term), M-b3 (the refresh's normal carried by the incident body) and
    //!   M-b4 (no τ_eff clamp) must each turn it red. Two fixed cases run first and carry M-b1's
    //!   witness, so no random draw can leave the rotation term unexercised.
    //! * **G-L9b-2** — lemma L9-L1: after a forced miss (a teleport and back, a shape change and
    //!   back) the outputs of three steps at frozen poses are bitwise equal, and a record's first
    //!   step equals its next. M-b5 (a miss emitting the raw full collision) must turn it red.
    //! * **G-L9b-4** (units) — a box on a slab that is body B hits (M-b2, F = A always, turns it
    //!   red); a shape change misses; a sensor pair and a fast pair never record.

    #[cfg(not(miri))]
    use std::cell::Cell;

    #[cfg(not(miri))]
    use proptest::prelude::*;

    use super::*;
    use crate::components::{Collider, RigidBody, RigidBodyMass};
    #[cfg(not(miri))]
    use crate::narrowphase::box_box::{BoxBoxOutcome, box_box_classify};
    use crate::narrowphase::carry::CarryIn;
    use crate::resources::Manifolds;
    use crate::row_identity::RowRemap;
    use crate::systems::narrowphase_serial_with;

    /// The step of every test here.
    const DT: f32 = 1.0 / 60.0;
    /// Body A's row.
    const A: BodyIndex = BodyIndex(0);
    /// Body B's row.
    const B: BodyIndex = BodyIndex(1);

    /// xorshift64*: a seeded, dependency-free generator.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
        }

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

        fn unit(&mut self) -> f32 {
            (self.next() >> 40) as f32 / (1u64 << 24) as f32
        }

        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (hi - lo) * self.unit()
        }

        #[cfg(not(miri))]
        fn direction(&mut self) -> Vec3 {
            loop {
                let v = Vec3::new(self.range(-1.0, 1.0), self.range(-1.0, 1.0), self.range(-1.0, 1.0));
                let l = v.length_squared();
                if l > 1.0e-4 && l <= 1.0 {
                    return v * l.sqrt().recip();
                }
            }
        }
    }

    /// The rotation by `angle` about the unit `axis`.
    fn about(axis: Vec3, angle: f32) -> Quat {
        let (s, c) = (0.5 * angle).sin_cos();
        Quat::new(axis.x * s, axis.y * s, axis.z * s, c).normalize()
    }

    /// A box body at `position` with `rotation`, `half` extents and linear velocity `v`.
    fn box_body(position: Vec3, rotation: Quat, half: Vec3, v: Vec3, sensor: bool) -> BodyState {
        let body = RigidBody { position, linear_velocity: v, rotation, angular_velocity: Vec3::ZERO };
        let mass = RigidBodyMass {
            inv_inertia: Mat3::IDENTITY,
            inv_mass: 1.0,
            restitution: 0.0,
            friction: 0.5,
        };
        let collider = Collider { shape: ColliderShape::Box { half_extents: half }, layer: 1, mask: 1 };
        BodyState::from_columns(&body, &mass, &collider, sensor, true, false)
    }

    /// A manifold as words: every field and every point slot.
    fn manifold_words(m: &Manifold) -> Vec<u32> {
        let mut w = vec![m.body_a.0, m.body_b.0, u32::from(m.count)];
        w.extend([m.normal.x, m.normal.y, m.normal.z].map(f32::to_bits));
        for p in &m.points {
            w.extend(
                [p.anchor_a.x, p.anchor_a.y, p.anchor_a.z, p.anchor_b.x, p.anchor_b.y, p.anchor_b.z]
                    .map(f32::to_bits),
            );
            w.extend([p.separation.to_bits(), p.feature_id]);
        }
        w
    }

    /// What one narrowphase frame produced: the solver stream, each pair's tag, and each pair's
    /// record words with the parity bit masked (a record written in two steps differs there only).
    #[derive(Debug, PartialEq)]
    struct FrameOut {
        stream: Vec<Vec<u32>>,
        tags: Vec<PairTag>,
        records: Vec<Option<[u32; 32]>>,
    }

    /// One frame of the serial narrowphase with contact reuse forced on at distance `tau`: the pairs
    /// join the previous frame's (the same list) unless `first`.
    fn frame(
        m: &mut Manifolds,
        bodies: &[BodyState],
        pairs: &[(BodyIndex, BodyIndex)],
        first: bool,
        tau: f32,
    ) -> FrameOut {
        m.box_axis_cache.begin_frame(pairs.len());
        let reuse = ReuseStep::new(true, tau, DT, m.box_axis_cache.keys_changed());
        let carry =
            if first { CarryIn::NONE } else { CarryIn::new(RowRemap::Identity, pairs, &[]) };
        narrowphase_serial_with(m, bodies, pairs, false, carry.with_reuse(reuse));
        let tags = m.pair_carry.tags();
        let records = m.pair_carry.records();
        FrameOut {
            stream: m.manifolds().iter().map(manifold_words).collect(),
            tags: tags.to_vec(),
            records: tags
                .iter()
                .enumerate()
                .map(|(k, t)| {
                    t.has(PairTag::REC).then(|| {
                        let mut w = records[k].words();
                        w[3] &= !(u32::from(PARITY) << 8);
                        w
                    })
                })
                .collect(),
        }
    }

    // ── G-L9b-4: units ────────────────────────────────────────────────────────────────────

    /// A box resting 0.1 mm deep on a 100 m slab, 40 m off the slab's centre: the box is row 0
    /// (body A), the slab row 1 (body B, the larger body F).
    fn box_on_slab(offset: Vec3, rotation: Quat, half: Vec3, v: Vec3, sensor: bool) -> [BodyState; 2] {
        let slab = box_body(
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(50.0, 1.0, 50.0),
            Vec3::ZERO,
            false,
        );
        let b = box_body(Vec3::new(40.0, half.y - 1.0e-4, 5.0) + offset, rotation, half, v, sensor);
        [b, slab]
    }

    /// The tag the one pair of a box-on-slab frame wrote.
    fn tag_of(out: &FrameOut) -> PairTag {
        out.tags[0]
    }

    /// The unit box's half-extents.
    const UNIT: Vec3 = Vec3::new(1.0, 1.0, 1.0);

    /// G-L9b-4: measured in the larger body's frame, a box on a slab reuses its record through a
    /// small tilt. M-b2 (F = A always, here the box) must turn this red: in the box's frame the
    /// slab's centre, 40 m away, swings 4 mm for the 1e-4 rad tilt.
    #[test]
    fn a_box_on_a_slab_that_is_body_b_hits_with_the_slab_as_f() {
        let pairs = [(A, B)];
        let mut m = Manifolds::with_capacity(1);
        let at_rest = box_on_slab(Vec3::ZERO, Quat::IDENTITY, UNIT, Vec3::ZERO, false);
        let f0 = frame(&mut m, &at_rest, &pairs, true, 1.0e-3);
        assert!(
            tag_of(&f0).has(PairTag::REC) && !tag_of(&f0).has(PairTag::HIT),
            "the first frame builds the record: {:#06x}",
            f0.tags[0].bits()
        );
        // A 1e-4 rad tilt about the box's own centre and a 0.1 mm slide: in the slab's frame S
        // moved 0.1 mm and turned 1e-4 rad, 2·(|Δd|² + 4s²(r_S + τ)²) ≈ 7e-8 against τ² = 1e-6.
        let tilt = about(Vec3::new(0.0, 0.0, 1.0), 1.0e-4);
        let moved = box_on_slab(Vec3::new(1.0e-4, 0.0, 0.0), tilt, UNIT, Vec3::ZERO, false);
        let f1 = frame(&mut m, &moved, &pairs, false, 1.0e-3);
        assert!(
            tag_of(&f1).has(PairTag::REC | PairTag::HIT),
            "measured in the larger body's frame, the slab pair must reuse its record: {:#06x}",
            f1.tags[0].bits()
        );
        assert_eq!(f1.stream.len(), 1, "the reused record still emits the contact");
    }

    /// G-L9b-4: a box whose extents changed by an ulp at the same pose misses its record's shape
    /// pin and rebuilds; the rebuilt record then hits.
    #[test]
    fn a_shape_change_misses() {
        let pairs = [(A, B)];
        let mut m = Manifolds::with_capacity(1);
        frame(&mut m, &box_on_slab(Vec3::ZERO, Quat::IDENTITY, UNIT, Vec3::ZERO, false), &pairs, true, 1.0e-3);
        let grown = Vec3::new(1.0, 1.0, 1.0 + f32::EPSILON);
        let reshaped = box_on_slab(Vec3::ZERO, Quat::IDENTITY, grown, Vec3::ZERO, false);
        let f1 = frame(&mut m, &reshaped, &pairs, false, 1.0e-3);
        assert!(
            tag_of(&f1).has(PairTag::REC) && !tag_of(&f1).has(PairTag::HIT),
            "a reshaped box must miss and rebuild: {:#06x}",
            f1.tags[0].bits()
        );
        let f2 = frame(&mut m, &reshaped, &pairs, false, 1.0e-3);
        assert!(tag_of(&f2).has(PairTag::HIT), "control: the rebuilt record hits: {:#06x}", f2.tags[0].bits());
    }

    /// G-L9b-4: a sensor pair's overlap is emitted on today's path, frame after frame, and never
    /// recorded.
    #[test]
    fn a_sensor_pair_never_records() {
        let pairs = [(A, B)];
        let mut m = Manifolds::with_capacity(1);
        let bodies = box_on_slab(Vec3::ZERO, Quat::IDENTITY, UNIT, Vec3::ZERO, true);
        for first in [true, false, false] {
            let out = frame(&mut m, &bodies, &pairs, first, 1.0e-3);
            assert!(
                !tag_of(&out).has(PairTag::REC) && tag_of(&out).has(PairTag::PUSHED),
                "a sensor overlap is never recorded: {:#06x}",
                out.tags[0].bits()
            );
            assert_eq!(m.sensor_overlaps().len(), 1, "construction: the sensor overlaps the slab");
        }
    }

    /// G-L9b-4: a fast pair takes today's path, frame after frame, and never records; the same
    /// pair at a thousandth of the speed records.
    #[test]
    fn a_fast_pair_never_records() {
        let pairs = [(A, B)];
        // 1 m/s: 3·dt²·|v|² = 8.3e-4 against (τ/2)² = 2.5e-7.
        let v = Vec3::new(0.0, -1.0, 0.0);
        let mut m = Manifolds::with_capacity(1);
        let fast = box_on_slab(Vec3::ZERO, Quat::IDENTITY, UNIT, v, false);
        for first in [true, false, false] {
            let out = frame(&mut m, &fast, &pairs, first, 1.0e-3);
            assert!(
                !tag_of(&out).has(PairTag::REC) && tag_of(&out).has(PairTag::PUSHED),
                "a fast contact never records: {:#06x}",
                out.tags[0].bits()
            );
        }
        let slow_bodies = box_on_slab(Vec3::ZERO, Quat::IDENTITY, UNIT, v * 1.0e-3, false);
        let slow = frame(&mut m, &slow_bodies, &pairs, false, 1.0e-3);
        assert!(tag_of(&slow).has(PairTag::REC), "control: at 1 mm/s the pair records: {:#06x}", slow.tags[0].bits());
    }

    // ── G-L9b-2: lemma L9-L1 ──────────────────────────────────────────────────────────────

    /// A static slab and `n` boxes of random orientation packed above it, so most pairs touch;
    /// every pair `(i, j)`, `i < j`.
    #[cfg(not(miri))]
    fn pile(rng: &mut Rng, n: usize) -> (Vec<BodyState>, Vec<(BodyIndex, BodyIndex)>) {
        let mut bodies = vec![box_body(
            Vec3::new(0.0, -0.5, 0.0),
            Quat::IDENTITY,
            Vec3::new(4.0, 0.5, 4.0),
            Vec3::ZERO,
            false,
        )];
        for _ in 0..n {
            let q = Quat::new(rng.range(-0.3, 0.3), rng.range(-1.0, 1.0), rng.range(-0.3, 0.3), 1.0)
                .normalize();
            let p = Vec3::new(rng.range(-1.0, 1.0), rng.range(0.3, 1.6), rng.range(-1.0, 1.0));
            let h = Vec3::new(rng.range(0.3, 0.6), rng.range(0.3, 0.6), rng.range(0.3, 0.6));
            bodies.push(box_body(p, q, h, Vec3::ZERO, false));
        }
        let mut pairs = Vec::new();
        for i in 0..bodies.len() as u32 {
            for j in i + 1..bodies.len() as u32 {
                pairs.push((BodyIndex(i), BodyIndex(j)));
            }
        }
        (bodies, pairs)
    }

    /// Pairs whose raw full collision differs in some bit from the refresh of the record built on
    /// it (what a miss emits): the M-b5 witness, recomputed outside the narrowphase.
    #[cfg(not(miri))]
    fn raw_differs_from_refresh(bodies: &[BodyState], pairs: &[(BodyIndex, BodyIndex)], tau: f32) -> u64 {
        let mut n = 0;
        for &(a, b) in pairs {
            let (ba, bb) = (&bodies[a.0 as usize], &bodies[b.0 as usize]);
            let (ColliderShape::Box { half_extents: ha }, ColliderShape::Box { half_extents: hb }) =
                (ba.shape, bb.shape)
            else {
                continue;
            };
            let oa = Obb::new(ba.position, ba.rotation, ha);
            let ob = Obb::new(bb.position, bb.rotation, hb);
            if let BoxBoxOutcome::Contact(c) = box_box_classify(&oa, &ob, a, b, None) {
                let g = PairGeom::new(&oa, &ob, ha.length(), hb.length(), tau);
                let r = build(&c, &oa, &ob, ba.rotation, bb.rotation, &g, false);
                if let Refreshed::Contact(m) = refresh(&r, &oa, &ob, a, b) {
                    n += u64::from(manifold_words(&m) != manifold_words(&c.manifold));
                }
            }
        }
        n
    }

    /// What G-L9b-2 exercised.
    #[cfg(not(miri))]
    #[derive(Clone, Copy, Debug, Default)]
    struct FrozenSeen {
        /// Pairs whose raw full collision a miss must not emit (the M-b5 witness).
        raw_differs: u64,
        /// Hits in the frame after a record's first.
        hits: u64,
        /// Pairs of the moved body that missed and rebuilt after it came back.
        teleport_rebuilds: u64,
        /// Pairs of the reshaped body that missed and rebuilt after its shape came back.
        reshape_rebuilds: u64,
    }

    /// The pairs of `out` that rebuilt their record (`REC` without `HIT`) and touch `row`.
    #[cfg(not(miri))]
    fn rebuilds_of(out: &FrameOut, pairs: &[(BodyIndex, BodyIndex)], row: u32) -> u64 {
        out.tags
            .iter()
            .zip(pairs)
            .filter(|(t, (a, b))| {
                t.has(PairTag::REC) && !t.has(PairTag::HIT) && (a.0 == row || b.0 == row)
            })
            .count() as u64
    }

    /// G-L9b-2 (lemma L9-L1): at frozen poses a slow pair's output is the same bits step after
    /// step — from its record's first step, and after a forced miss of either kind: a body moved
    /// away and back (its pairs separate, then rebuild), and a body reshaped by an ulp and back (its
    /// pairs miss the shape pin twice). Three frames after each, stream and records bitwise equal.
    ///
    /// Mutation M-b5 — a miss emitting the raw full collision instead of its record's refresh — must
    /// turn this red: the raw collision and the refresh differ in some bit on the pairs the witness
    /// counts, so the miss frame and the hit after it differ.
    #[test]
    #[cfg(not(miri))]
    fn frozen_poses_give_the_same_bits_after_a_forced_miss() {
        let totals = Cell::new(FrozenSeen::default());
        let config = ProptestConfig { cases: 96, failure_persistence: None, ..ProptestConfig::default() };
        proptest!(config, |(seed in any::<u64>())| {
            let mut rng = Rng::new(seed);
            let n = 5 + rng.below(6) as usize;
            let (bodies, pairs) = pile(&mut rng, n);
            let tau = [2.5e-4, 5.0e-4, 1.0e-3, 2.0e-3][rng.below(4) as usize];
            let mut seen = totals.get();
            seen.raw_differs += raw_differs_from_refresh(&bodies, &pairs, tau);
            let mut m = Manifolds::with_capacity(pairs.len());

            let o0 = frame(&mut m, &bodies, &pairs, true, tau);
            let o1 = frame(&mut m, &bodies, &pairs, false, tau);
            let o2 = frame(&mut m, &bodies, &pairs, false, tau);
            seen.hits += o1.tags.iter().filter(|t| t.has(PairTag::HIT)).count() as u64;
            prop_assert_eq!(
                (&o0.stream, &o0.records), (&o1.stream, &o1.records),
                "seed {:#x}: a record's first frame and its hit", seed
            );
            prop_assert_eq!((&o1.stream, &o1.records), (&o2.stream, &o2.records), "seed {:#x}: two hits", seed);

            // A body moved 1 km away and back.
            let x = 1 + rng.below(n as u64) as usize;
            let mut away = bodies.clone();
            away[x].position = away[x].position + Vec3::new(1000.0, 0.0, 0.0);
            frame(&mut m, &away, &pairs, false, tau);
            let back = [
                frame(&mut m, &bodies, &pairs, false, tau),
                frame(&mut m, &bodies, &pairs, false, tau),
                frame(&mut m, &bodies, &pairs, false, tau),
            ];
            seen.teleport_rebuilds += rebuilds_of(&back[0], &pairs, x as u32);
            for w in back.windows(2) {
                prop_assert_eq!(
                    (&w[0].stream, &w[0].records), (&w[1].stream, &w[1].records),
                    "seed {:#x}: after a teleport and back", seed
                );
            }

            // A body reshaped by an ulp and back.
            let mut reshaped = bodies.clone();
            if let ColliderShape::Box { half_extents } = reshaped[x].shape {
                let grown = half_extents + Vec3::new(0.0, 0.0, half_extents.z * f32::EPSILON);
                reshaped[x].shape = ColliderShape::Box { half_extents: grown };
            }
            frame(&mut m, &reshaped, &pairs, false, tau);
            let restored = [
                frame(&mut m, &bodies, &pairs, false, tau),
                frame(&mut m, &bodies, &pairs, false, tau),
                frame(&mut m, &bodies, &pairs, false, tau),
            ];
            seen.reshape_rebuilds += rebuilds_of(&restored[0], &pairs, x as u32);
            for w in restored.windows(2) {
                prop_assert_eq!(
                    (&w[0].stream, &w[0].records), (&w[1].stream, &w[1].records),
                    "seed {:#x}: after a shape change and back", seed
                );
            }
            totals.set(seen);
        });
        let seen = totals.get();
        println!("G-L9b-2 coverage: {seen:?}");
        assert!(
            seen.raw_differs > 0,
            "no pair's raw collision differs from its record's refresh (M-b5 could not be seen): {seen:?}"
        );
        assert!(seen.hits > 0, "no record hit at frozen poses: {seen:?}");
        assert!(seen.teleport_rebuilds > 0, "no pair of the moved body rebuilt its record: {seen:?}");
        assert!(seen.reshape_rebuilds > 0, "no pair of the reshaped body rebuilt its record: {seen:?}");
    }

    // ── G-L9b-1: the criterion's soundness ────────────────────────────────────────────────

    /// A box pair at the record's poses `0` and at the perturbed poses `1`.
    #[cfg(not(miri))]
    #[derive(Clone, Copy, Debug)]
    struct SoundCase {
        ha: Vec3,
        hb: Vec3,
        pose0: [(Vec3, Quat); 2],
        pose1: [(Vec3, Quat); 2],
        tau: f32,
    }

    /// A box resting on a base box or slab — tilted so its lowest corner sits within a few τ of the
    /// base's top face — then moved relative to the base by a rotation about its own centre and a
    /// translation, in half the cases within what the criterion accepts and in the other half far
    /// past it, and the pair carried by a common motion. The base is body A or body B at random; one case in eight sits 20–60 m out on a slab.
    #[cfg(not(miri))]
    fn sound_case(rng: &mut Rng) -> SoundCase {
        let tau = [2.5e-4, 5.0e-4, 1.0e-3, 2.0e-3][rng.below(4) as usize];
        let far = rng.below(8) == 0;
        let slab = far || rng.below(3) == 0;
        let base_half = if slab {
            Vec3::new(rng.range(20.0, 60.0), rng.range(0.05, 1.0), rng.range(20.0, 60.0))
        } else {
            Vec3::new(rng.range(0.2, 1.5), rng.range(0.2, 1.5), rng.range(0.2, 1.5))
        };
        let top_half = match rng.below(4) {
            0 => Vec3::new(rng.range(0.002, 0.02), rng.range(0.002, 0.02), rng.range(0.002, 0.02)),
            1 => Vec3::new(rng.range(0.01, 0.6), rng.range(0.001, 0.02), rng.range(0.01, 0.6)),
            _ => Vec3::new(rng.range(0.2, 1.2), rng.range(0.2, 1.2), rng.range(0.2, 1.2)),
        };
        let g = about(rng.direction(), rng.range(0.0, core::f32::consts::PI));
        let base_c = Vec3::new(rng.range(-5.0, 5.0), rng.range(-5.0, 5.0), rng.range(-5.0, 5.0));
        // The top box in the base's frame: yawed about the base's y, then tilted about x.
        let lever = top_half.x.max(top_half.z);
        let phi = if rng.below(3) == 0 { 0.0 } else { rng.range(0.0, 3.0 * tau / lever) };
        let yaw = about(Vec3::new(0.0, 1.0, 0.0), rng.range(-3.2, 3.2));
        let local_q = about(Vec3::new(1.0, 0.0, 0.0), phi).mul(yaw);
        let reach = if far { (base_half.x - 1.0).max(0.0) } else { base_half.x.min(base_half.z) };
        let (ox, oz) = if far {
            (rng.range(0.7, 1.0) * reach, rng.range(-0.3, 0.3) * reach)
        } else {
            (rng.range(-1.0, 1.0) * reach, rng.range(-1.0, 1.0) * reach)
        };
        // The lowest corner's depth below the base's top face: a gap of up to τ, or up to 2τ deep.
        let axes = RowFrame::axes_of(local_q);
        let extent_y =
            top_half.x * axes[0].y.abs() + top_half.y * axes[1].y.abs() + top_half.z * axes[2].y.abs();
        let depth = rng.range(-tau, 2.0 * tau);
        let local_c = Vec3::new(ox, base_half.y + extent_y - depth, oz);
        let top0 = (base_c + g.rotate(local_c), g.mul(local_q));
        let base0 = (base_c, g);

        // The perturbation, in the base's frame: a rotation about the top box's centre (about a
        // horizontal axis half the time, where it tips the box) and a translation.
        let r_top = top_half.length();
        let h_min = top_half
            .x
            .min(top_half.y)
            .min(top_half.z)
            .min(base_half.x)
            .min(base_half.y)
            .min(base_half.z);
        let tau_eff = tau.min(TAU_EFF_FRACTION * r_top.min(base_half.length())).min(TAU_EFF_FRACTION * h_min);
        let axis = if rng.below(2) == 0 {
            if rng.below(2) == 0 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 0.0, 1.0) }
        } else {
            rng.direction()
        };
        // Half the cases stay inside what the criterion accepts (up to half of τ_eff from each
        // term), half reach twenty times past it in rotation and three times in translation.
        let (psi_max, shift_max) =
            if rng.below(2) == 0 { (0.5, 0.5) } else { (20.0, 3.0) };
        let psi = rng.unit() * psi_max * tau_eff / r_top;
        let shift = rng.direction() * (rng.unit() * shift_max * tau_eff);
        let top1_local = (local_c + shift, about(axis, psi).mul(local_q));
        // A common motion of both bodies.
        let h = about(rng.direction(), rng.range(0.0, 0.3));
        let t = rng.direction() * rng.range(0.0, 1.0);
        let base1 = (h.rotate(base_c) + t, h.mul(g));
        let top1 = (h.rotate(base_c + g.rotate(top1_local.0)) + t, h.mul(g.mul(top1_local.1)));

        if rng.below(2) == 0 {
            SoundCase { ha: base_half, hb: top_half, pose0: [base0, top0], pose1: [base1, top1], tau }
        } else {
            SoundCase { ha: top_half, hb: base_half, pose0: [top0, base0], pose1: [top1, base1], tau }
        }
    }

    /// G-L9b-1's two fixed rotation witnesses, so that M-b1's visibility does not rest on the
    /// random draw. Among the random cases, a hit whose rotation term reaches half of the
    /// criterion's budget comes up about 7 times in 4096 (`SoundSeen::rotation_bound`), so about one
    /// run in 2000 would read 0.
    ///
    /// A cube of half-extent 0.5 (body B, S) rests 0.5 mm deep on a 2 × 1 × 2 m base (body A, F:
    /// the larger radius, no tie) at τ = 1 mm, where τ_eff = τ. Between the poses S turns about the
    /// x axis through its own centre. Δd is then exactly 0, and the criterion's left side is its
    /// rotation term alone, `8 s² (r_S + τ_eff)²`, which each case sets to `k·τ_eff²`:
    ///
    /// * `k = 0.8`: a hit, and the rotation term is 0.8 of the budget (the witness counts from
    ///   0.5), so every run counts it in `rotation_bound`;
    /// * `k = 8`: a miss that the rotation term alone decides. Without that term (M-b1) it is a
    ///   hit on which the corners 0.707 m off the axis move `√(k/2)·0.707·τ_eff / (r_S + τ_eff)`
    ///   ≈ 1.63 τ_eff, so check (c) fails on every run.
    #[cfg(not(miri))]
    fn rotation_witnesses() -> [SoundCase; 2] {
        let tau = 1.0e-3;
        let (ha, hb) = (Vec3::new(1.0, 0.5, 1.0), Vec3::new(0.5, 0.5, 0.5));
        let base = (Vec3::ZERO, Quat::IDENTITY);
        let centre = Vec3::new(0.0, ha.y + hb.y - 0.5 * tau, 0.0);
        let arm = hb.length() + tau;
        [0.8f32, 8.0].map(|k| {
            let s = (k / 8.0).sqrt() * tau / arm;
            let turned = about(Vec3::new(1.0, 0.0, 0.0), 2.0 * s.asin());
            SoundCase {
                ha,
                hb,
                pose0: [base, (centre, Quat::IDENTITY)],
                pose1: [base, (centre, turned)],
                tau,
            }
        })
    }

    /// `v` as `f64`s.
    #[cfg(not(miri))]
    fn f64s(v: Vec3) -> [f64; 3] {
        [f64::from(v.x), f64::from(v.y), f64::from(v.z)]
    }

    #[cfg(not(miri))]
    fn dot64(a: [f64; 3], b: [f64; 3]) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[cfg(not(miri))]
    fn sub64(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    /// The world position of `obb`'s local point `x`, in `f64` from its `f32` frame.
    #[cfg(not(miri))]
    fn world64(obb: &Obb, x: [f64; 3]) -> [f64; 3] {
        let c = f64s(obb.center);
        let ax = obb.axes.map(f64s);
        [0, 1, 2].map(|i| c[i] + ax[0][i] * x[0] + ax[1][i] * x[1] + ax[2][i] * x[2])
    }

    /// `p` in `obb`'s frame, in `f64`.
    #[cfg(not(miri))]
    fn local64(obb: &Obb, p: [f64; 3]) -> [f64; 3] {
        let d = sub64(p, f64s(obb.center));
        obb.axes.map(|a| dot64(f64s(a), d))
    }

    /// The largest absolute coordinate of `v`.
    #[cfg(not(miri))]
    fn max_abs(v: Vec3) -> f64 {
        f64::from(v.x.abs().max(v.y.abs()).max(v.z.abs()))
    }

    /// The deepest penetration of a manifold's points, `max(0, −min separation)`.
    #[cfg(not(miri))]
    fn deepest(m: Option<&Manifold>) -> f64 {
        m.map_or(0.0, |m| {
            m.points[..usize::from(m.count)]
                .iter()
                .fold(0.0f64, |d, p| d.max(-f64::from(p.separation)))
        })
    }

    /// What G-L9b-1 exercised, summed over its cases.
    #[cfg(not(miri))]
    #[derive(Clone, Copy, Debug, Default)]
    struct SoundSeen {
        records: u64,
        hits: u64,
        misses: u64,
        /// Original incident corners whose refreshed separation was checked (a).
        corners: u64,
        /// Hits on which the full collision found a point deeper than the refresh's (b).
        unseen_deeper: u64,
        /// Hits on which the rotation term was at least half of the criterion's left side (M-b1).
        rotation_bound: u64,
        /// Hits on a pair whose thinnest half-extent is below 2 cm, where the clamp binds (M-b4).
        small: u64,
        /// Face hits whose incident body sits 20 m or more out (review O4's tolerance).
        far_incident: u64,
        /// Face hits on a record that kept fewer than four points.
        partial: u64,
        /// Hits on which the full collision switched to another feature, where (b) does not apply.
        switched: u64,
        /// The largest corner displacement over τ_eff seen on a hit (c).
        worst_ratio: f64,
    }

    /// One G-L9b-1 case: builds the record at poses 0 and, if the criterion keeps it at poses 1,
    /// checks (a), (b) and (c) of the module docs.
    #[cfg(not(miri))]
    fn sound_check(case: &SoundCase, seen: &mut SoundSeen) -> Result<(), String> {
        let obbs = |pose: &[(Vec3, Quat); 2]| {
            (Obb::new(pose[0].0, pose[0].1, case.ha), Obb::new(pose[1].0, pose[1].1, case.hb))
        };
        let (oa0, ob0) = obbs(&case.pose0);
        let (oa1, ob1) = obbs(&case.pose1);
        let (ra, rb) = (case.ha.length(), case.hb.length());
        let BoxBoxOutcome::Contact(c) = box_box_classify(&oa0, &ob0, A, B, None) else {
            return Ok(());
        };
        let g0 = PairGeom::new(&oa0, &ob0, ra, rb, case.tau);
        let record = build(&c, &oa0, &ob0, case.pose0[0].1, case.pose0[1].1, &g0, false);
        seen.records += 1;
        if !criterion(&record, &oa0, &ob0, case.pose0[0].1, case.pose0[1].1, &g0) {
            return Err("a record must pass the criterion on the poses it was built on (lemma L9-L1)".into());
        }
        let g1 = PairGeom::new(&oa1, &ob1, ra, rb, case.tau);
        if !criterion(&record, &oa1, &ob1, case.pose1[0].1, case.pose1[1].1, &g1) {
            seen.misses += 1;
            return Ok(());
        }
        seen.hits += 1;

        // The oracle τ_eff, in f64 from the extents.
        let h_min = [case.ha.x, case.ha.y, case.ha.z, case.hb.x, case.hb.y, case.hb.z]
            .map(f64::from)
            .into_iter()
            .fold(f64::INFINITY, f64::min);
        let tau_eff = f64::from(case.tau)
            .min(0.05 * f64::from(ra).min(f64::from(rb)))
            .min(0.05 * h_min);
        let scale = [oa1.center, ob1.center, oa0.center, ob0.center]
            .map(max_abs)
            .into_iter()
            .fold(0.0f64, f64::max)
            + f64::from(ra.max(rb));
        // f32 resolution at the pair's coordinates: a few ulps of its largest one.
        let noise = 8.0 * f64::from(f32::EPSILON) * scale.max(1.0);

        // (c) Lemma L9-L3: every corner of S moved by at most τ_eff in F's frame.
        let f_is_b = record.flags & F_IS_B != 0;
        let (f0, f1, s0, s1, hs) =
            if f_is_b { (&ob0, &ob1, &oa0, &oa1, case.ha) } else { (&oa0, &oa1, &ob0, &ob1, case.hb) };
        let mut worst = 0.0f64;
        for corner in 0..8u32 {
            let x = [0u32, 1, 2].map(|i| {
                let h = f64::from([hs.x, hs.y, hs.z][i as usize]);
                if (corner >> i) & 1 == 1 { h } else { -h }
            });
            let d = sub64(local64(f1, world64(s1, x)), local64(f0, world64(s0, x)));
            worst = worst.max(dot64(d, d).sqrt());
        }
        seen.worst_ratio = seen.worst_ratio.max(worst / tau_eff);
        if worst > tau_eff * (1.0 + 1.0e-4) + noise {
            return Err(format!(
                "(c) a corner of S moved {worst:e} m in F's frame on a hit, past τ_eff {tau_eff:e} \
                 + {noise:e}"
            ));
        }
        let (qf, qs) = if f_is_b {
            (case.pose1[1].1, case.pose1[0].1)
        } else {
            (case.pose1[0].1, case.pose1[1].1)
        };
        let rel = record.q_ref.conjugate().mul(qf.conjugate().mul(qs));
        let s2 = f64::from(rel.x * rel.x + rel.y * rel.y + rel.z * rel.z);
        let arm = f64::from(if f_is_b { ra } else { rb }) + f64::from(g1.tau_eff);
        if 8.0 * s2 * arm * arm >= 0.5 * f64::from(g1.tau_eff) * f64::from(g1.tau_eff) {
            seen.rotation_bound += 1;
        }
        if h_min < 0.02 {
            seen.small += 1;
        }

        // (a): the refresh at poses 1 against the exact corner distances.
        let refreshed = match refresh(&record, &oa1, &ob1, A, B) {
            Refreshed::Contact(m) => Some(m),
            Refreshed::Separated(_) | Refreshed::Degenerate => None,
        };
        if let Some(m) = &refreshed
            && !record.is_edge()
        {
            let ref_is_b = record.flags & REF_IS_B != 0;
            let (rf, inc) = if ref_is_b { (&ob1, &oa1) } else { (&oa1, &ob1) };
            if max_abs(inc.center) >= 20.0 {
                seen.far_incident += 1;
            }
            if usize::from(record.count) < MAX_CONTACT_POINTS {
                seen.partial += 1;
            }
            let i = usize::from(record.feat);
            let sign = if record.flags & REF_NEG != 0 { -1.0 } else { 1.0 };
            let n = f64s(rf.axes[i]).map(|x| x * sign);
            let r_inc = f64::from(if ref_is_b { ra } else { rb });
            let tol_a = 2.0e-6 * (max_abs(inc.center) + max_abs(rf.center) + r_inc).max(1.0);
            for p in &m.points[..usize::from(m.count)] {
                if p.feature_id & 0xE000 != 0 {
                    continue; // a clipped vertex, not an original corner
                }
                let vtx = p.feature_id & 7;
                let x = [0u32, 1, 2].map(|j| {
                    let h = f64::from(inc.half[j as usize]);
                    if (vtx >> j) & 1 == 1 { h } else { -h }
                });
                let exact = dot64(sub64(world64(inc, x), f64s(rf.center)), n) - f64::from(rf.half[i]);
                seen.corners += 1;
                if (f64::from(p.separation) - exact).abs() > tol_a {
                    return Err(format!(
                        "(a) corner {vtx}'s refreshed separation {:e} is not its distance {exact:e} \
                         to the current reference plane (tolerance {tol_a:e})",
                        p.separation
                    ));
                }
            }
        }

        // (b): the full collision's deepest point against the refresh's, when the full collision is
        // the same contact — no contact at all, or a face contact on the record's reference face.
        // A full collision that switched feature measures its separations along another normal,
        // where the two depths do not compare (`SoundSeen::switched`).
        let full = match box_box_classify(&oa1, &ob1, A, B, Some(c.reference_axis)) {
            BoxBoxOutcome::Contact(c1) => Some(c1),
            _ => None,
        };
        let same = full.as_ref().is_none_or(|c1| {
            contact_feature(&oa1, &ob1, c1) == contact_feature(&oa0, &ob0, &c)
        });
        if !same {
            seen.switched += 1;
            return Ok(());
        }
        let full = full.map(|c1| c1.manifold);
        let (d_full, d_ref) = (deepest(full.as_ref()), deepest(refreshed.as_ref()));
        if d_full > d_ref + 1.0e-6 {
            seen.unseen_deeper += 1;
        }
        let tol_b = 1.0e-5f64.max(noise);
        if d_full > d_ref + 2.0 * tau_eff + tol_b {
            return Err(format!(
                "(b) the full collision's deepest point {d_full:e} is more than 2·τ_eff \
                 ({tau_eff:e}) + {tol_b:e} deeper than the refresh's {d_ref:e}"
            ));
        }
        Ok(())
    }

    /// G-L9b-1: the criterion's soundness (module docs). 4096 random cases, after the two fixed
    /// rotation witnesses of [`rotation_witnesses`].
    ///
    /// Mutations this must turn red: M-b1, the criterion without `4 s² (r_S + τ_eff)²` (a rotation
    /// about S's centre moves no centre, so only (c) and (b) catch it; the fixed `k = 8` witness
    /// catches it on every run, before any random case); M-b3, the refresh measuring against the
    /// incident body's own face normal instead of the reference face's (a); M-b4, τ_eff unclamped
    /// (the small and thin boxes, (c)).
    #[test]
    #[cfg(not(miri))]
    fn the_criterion_bounds_what_a_hit_can_miss() {
        // The rotation witnesses: fixed, so a run cannot draw zero of them.
        let [bound, decided] = rotation_witnesses();
        let mut seen_bound = SoundSeen::default();
        if let Err(why) = sound_check(&bound, &mut seen_bound) {
            panic!("the fixed rotation-bound witness (k = 0.8): {why}; case {bound:?}");
        }
        assert!(
            seen_bound.hits == 1 && seen_bound.rotation_bound == 1,
            "the fixed rotation-bound witness must be a hit whose rotation term is at least half of \
             the criterion's budget: {seen_bound:?}"
        );
        let mut seen_decided = SoundSeen::default();
        if let Err(why) = sound_check(&decided, &mut seen_decided) {
            panic!("the fixed rotation-decided witness (k = 8): {why}; case {decided:?}");
        }
        assert!(
            seen_decided.records == 1 && seen_decided.misses == 1,
            "the fixed rotation-decided witness must build a record and miss it: {seen_decided:?}"
        );
        println!("G-L9b-1 fixed rotation witnesses: k = 0.8 {seen_bound:?}; k = 8 {seen_decided:?}");

        let totals = Cell::new(SoundSeen::default());
        let config = ProptestConfig { cases: 4096, failure_persistence: None, ..ProptestConfig::default() };
        proptest!(config, |(seed in any::<u64>())| {
            let mut rng = Rng::new(seed);
            let case = sound_case(&mut rng);
            let mut seen = totals.get();
            let verdict = sound_check(&case, &mut seen);
            totals.set(seen);
            prop_assert!(verdict.is_ok(), "seed {:#x}: {}; case {:?}", seed, verdict.unwrap_err(), case);
        });
        let seen = totals.get();
        println!("G-L9b-1 coverage: {seen:?}");
        assert!(seen.records >= 2048, "fewer than half the cases built a record: {seen:?}");
        assert!(seen.hits >= 512 && seen.misses >= 512, "the criterion must both keep and reject: {seen:?}");
        assert!(seen.corners > 0, "(a) checked no original corner: {seen:?}");
        assert!(seen.unseen_deeper > 0, "(b) never saw a deeper unseen feature: {seen:?}");
        // `seen.rotation_bound` is printed, not asserted: the fixed witnesses above carry M-b1's
        // visibility, since the random draw reads 0 about once in 2000 runs.
        assert!(seen.small > 0, "no hit on a box the clamp binds (M-b4 could not be seen): {seen:?}");
        assert!(seen.far_incident > 0, "no face hit with a far incident body (review O4): {seen:?}");
        assert!(seen.partial > 0, "no face hit on a record that kept fewer than four points: {seen:?}");
    }

}
