//! The universal contact **currency** — [`Manifold`] (plan D1) — and its
//! per-step body addressing key [`BodyIndex`] (plan IM-1).
//!
//! A `Manifold` is a fixed-capacity, `#[repr(C)]`, `Copy` POD record produced by
//! narrowphase and consumed by the solver. It is the single type every collision
//! producer and every solver speaks: a real solver, an SDF-native narrowphase, or
//! an external (Rapier/Jolt) backend all slot in by reading/writing this shape
//! without reshaping the pipeline.
//!
//! # Addressing (IM-1 — authoritative)
//!
//! A manifold is keyed by [`BodyIndex`] (a `u32` dense row index into
//! [`SolverScratch.bodies`](crate::resources::SolverScratch)), **NOT** by
//! `EntityId`. The manifold is a per-step transient solver input, not a stable
//! cross-frame identity; the dense row index is what the gather→solve→apply
//! pipeline addresses (sequential, cache-friendly, no exclusive scatter). The
//! stable `EntityId` is projected out only for the gameplay-facing
//! [`Contact`](crate::components::Contact) component.

use crate::math::{MAX_CONTACT_POINTS, Vec3};

/// Dense per-step row index of a body in
/// [`SolverScratch.bodies`](crate::resources::SolverScratch) (plan IM-1).
///
/// `#[repr(transparent)]` over a `u32`: a body count never exceeds `u32::MAX` in
/// a single step, and `u32` (vs `usize`) keeps the [`Manifold`] compact (MINOR-3).
///
/// This is a transient, per-step index — it is only meaningful within one
/// physics step (gather assigns it, apply re-walks the same row order). It is
/// **not** an `EntityId` and must not be persisted across frames.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BodyIndex(pub u32);

/// A single contact point within a [`Manifold`] (plan D1).
///
/// `anchor_a` / `anchor_b` are the contact anchors on each body; `separation`
/// is the signed gap along the manifold `normal` (negative = penetrating);
/// `feature_id` is a stable feature pairing tag a Phase-10 solver uses for
/// warm-start matching across frames.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContactPoint {
    /// Contact anchor on body A, in world units.
    pub anchor_a: Vec3,
    /// Contact anchor on body B, in world units.
    pub anchor_b: Vec3,
    /// Signed separation along the manifold `normal`; negative = penetrating.
    pub separation: f32,
    /// Feature-pair tag for Phase-10 warm-start matching across frames.
    pub feature_id: u32,
}

/// The universal up-to-4-point contact record produced by narrowphase and
/// consumed by the solver (plan D1, the "currency").
///
/// Fixed-capacity (no per-pair heap alloc — principle 5): the `points` array
/// holds [`MAX_CONTACT_POINTS`] slots, `count` says how many are live. The
/// `normal` lives once in the header (Box2D rationale) rather than per point.
/// Impulses are deliberately NOT here — they are the solver's warm-start state,
/// keeping the manifold a pure collision→solver currency.
///
/// Keyed by [`BodyIndex`] (IM-1), so the solver mutates the dense scratch rows
/// directly. The size is pinned by the const-assert below.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Manifold {
    /// Contact points; only `points[..count]` are live.
    pub points: [ContactPoint; MAX_CONTACT_POINTS],
    /// Shared contact normal (unit, pointing from A toward B).
    pub normal: Vec3,
    /// Dense row index of body A (plan IM-1).
    pub body_a: BodyIndex,
    /// Dense row index of body B (plan IM-1).
    pub body_b: BodyIndex,
    /// Number of live points in `points` (`<= MAX_CONTACT_POINTS`).
    pub count: u8,
    /// Explicit tail padding to a `#[repr(C)]`-stable size; reserved.
    ///
    /// The struct's max field alignment is 4 (every field is `f32`/`u32`/`u8`),
    /// so the size must round up to a multiple of 4. The fields total
    /// `128 + 12 + 4 + 4 + 1 = 149` B; `_pad: [u8; 3]` brings it to a stable
    /// `152` B.
    pub _pad: [u8; 3],
}

// Layout guarantee (OQ-4 — a hard const-assert, not a doc comment).
//
// The 2D foundation held the manifold inside a 2-cache-line (128 B) budget. The
// 3D 4-point manifold genuinely needs ~152 B, spilling into a 3rd cache line:
//   points : [ContactPoint; 4] = 4 × (Vec3 12 + Vec3 12 + f32 4 + u32 4) = 128
//   normal : Vec3                                                        =  12
//   body_a : u32                                                         =   4
//   body_b : u32                                                         =   4
//   count  : u8                                                          =   1
//   _pad   : [u8; 3]                                                     =   3
//   -------------------------------------------------------------------- = 152
// The 2-CL budget is intentionally relinquished: the 3D contact data is the
// driving cost, and the bound is updated truthfully (OQ-4: "if it spills, the
// bound is updated truthfully") to 192 B (3 cache lines), which the real 152 B
// satisfies with headroom for future per-point growth.
const _: () = assert!(size_of::<Manifold>() == 152);
const _: () = assert!(size_of::<Manifold>() <= 192);

impl Manifold {
    /// Constructs an empty manifold between two bodies (zero contact points).
    ///
    /// Narrowphase fills `points` / `count` / `normal` as it detects contacts.
    #[inline]
    pub fn new(body_a: BodyIndex, body_b: BodyIndex) -> Self {
        Self {
            points: [ContactPoint::default(); MAX_CONTACT_POINTS],
            normal: Vec3::ZERO,
            body_a,
            body_b,
            count: 0,
            _pad: [0; 3],
        }
    }
}
