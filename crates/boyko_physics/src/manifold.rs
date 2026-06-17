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

use crate::math::{MAX_CONTACT_POINTS, Vec2};

/// Dense per-step row index of a body in
/// [`SolverScratch.bodies`](crate::resources::SolverScratch) (plan IM-1).
///
/// `#[repr(transparent)]` over a `u32`: a body count never exceeds `u32::MAX` in
/// a single step, and `u32` (vs `usize`) keeps the [`Manifold`] inside its 2-CL
/// budget with headroom for the 3D `MAX_CONTACT_POINTS = 4` flip (MINOR-3).
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
    pub anchor_a: Vec2,
    /// Contact anchor on body B, in world units.
    pub anchor_b: Vec2,
    /// Signed separation along the manifold `normal`; negative = penetrating.
    pub separation: f32,
    /// Feature-pair tag for Phase-10 warm-start matching across frames.
    pub feature_id: u32,
}

/// The universal 2-point contact record produced by narrowphase and consumed by
/// the solver (plan D1, the "currency").
///
/// Fixed-capacity (no per-pair heap alloc — principle 5): the `points` array
/// holds [`MAX_CONTACT_POINTS`] slots, `count` says how many are live. The
/// `normal` lives once in the header (Box2D rationale) rather than per point.
/// Impulses are deliberately NOT here — they are the solver's warm-start state,
/// keeping the manifold a pure collision→solver currency.
///
/// Keyed by [`BodyIndex`] (IM-1), so the solver mutates the dense scratch rows
/// directly. The size is pinned `<= 128` B (2 cache lines) by the const-assert
/// below (MINOR-3).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Manifold {
    /// Contact points; only `points[..count]` are live.
    pub points: [ContactPoint; MAX_CONTACT_POINTS],
    /// Shared contact normal (unit, pointing from A toward B).
    pub normal: Vec2,
    /// Dense row index of body A (plan IM-1).
    pub body_a: BodyIndex,
    /// Dense row index of body B (plan IM-1).
    pub body_b: BodyIndex,
    /// Number of live points in `points` (`<= MAX_CONTACT_POINTS`).
    pub count: u8,
    /// Explicit tail padding to a `#[repr(C)]`-stable size; reserved.
    pub _pad: [u8; 3],
}

// MINOR-3: a hard layout guarantee, not a doc comment. With `BodyIndex(u32)`
// keys the 3D `MAX_CONTACT_POINTS = 4` flip still fits the 2-CL budget.
const _: () = assert!(size_of::<Manifold>() <= 128);

impl Manifold {
    /// Constructs an empty manifold between two bodies (zero contact points).
    ///
    /// Narrowphase fills `points` / `count` / `normal` as it detects contacts.
    #[inline]
    pub fn new(body_a: BodyIndex, body_b: BodyIndex) -> Self {
        Self {
            points: [ContactPoint::default(); MAX_CONTACT_POINTS],
            normal: Vec2::ZERO,
            body_a,
            body_b,
            count: 0,
            _pad: [0; 3],
        }
    }
}
