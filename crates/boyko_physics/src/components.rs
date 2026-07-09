//! Physics components — ordinary `#[derive(Component)]` CPU-archetype columns
//! with a Phase-10-ready hot/cold layout (plan D5).
//!
//! The split is deliberate (D5): [`RigidBody`] is the HOT integrator state in
//! its own SoA column, so the integrate loop streams only those bytes;
//! [`RigidBodyMass`] is the COLD mass/material in a SEPARATE column so it never
//! pollutes the integrate cache lines. [`Collider`] is a zero-`dyn` tagged
//! union. [`Contact`] is the gameplay-facing queryable snapshot — the ONLY
//! place an `EntityId` appears in the physics data (plan IM-1); the solve reads
//! the dense [`Manifolds`](crate::resources::Manifolds) resource buffer instead.

use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::{Bundle, Component};

use crate::manifold::Manifold;
use crate::math::{Mat3, Quat, Vec3};

/// HOT integrator state of a rigid body (plan D5) — its own SoA column.
///
/// Contains ONLY the fields the integrate loop touches every step, so
/// `Query<&mut RigidBody>` streams a tight, cache-dense column. Mass/material
/// lives in the separate [`RigidBodyMass`] column.
///
/// `Default` is derived: `Vec3` fields default to zero and `rotation` defaults
/// to [`Quat::IDENTITY`] (the `Quat` `Default` impl), so a default body has a
/// valid identity orientation, not an invalid all-zero quaternion.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct RigidBody {
    /// World position (center of mass), in world units.
    pub position: Vec3,
    /// Linear velocity, in world units per second.
    pub linear_velocity: Vec3,
    /// Orientation (unit quaternion).
    pub rotation: Quat,
    /// Angular velocity (world-frame axis-angle rate), in radians per second.
    pub angular_velocity: Vec3,
}

/// Runtime on/off of dynamic simulation for a body that HAS a [`RigidBody`]
/// (the capability+state model — Decision 1).
///
/// An EnableTag bit (`#[component(storage = "bitset")]`): O(1) toggle via the
/// kernel `enable::<Simulated>` / `disable::<Simulated>` API, with NO archetype
/// migration and NO row move — so flipping it NEVER reorders the physics gather
/// (Encoding A is preserved bit-for-bit). It is a zero-sized tag with no column;
/// the per-row bit is read non-filteringly through
/// [`IsEnabled<Simulated>`](boyko_ecs::ecs::core::iters::query::IsEnabled).
///
/// Semantics: a body whose `Simulated` bit is SET integrates under gravity and
/// is advanced by the solver (when `inv_mass != 0`). A body whose bit is CLEAR
/// is "parked": its pose is frozen-in-place (not integrated), though if it still
/// has `inv_mass != 0` the coloring/solve may apply impulses to it (Avian's
/// "dummy SolverBody for a disabled dynamic" freeze semantics — Decision 4).
///
/// REPLACES the old `BodyType` enum's `Dynamic` discrimination. A permanent
/// (collision-only) static body simply does not carry a [`RigidBody`] (Decision
/// 5 — structural skip); an immovable contact surface carries `RigidBody` with
/// `inv_mass == 0` and `Simulated` CLEAR.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[component(storage = "bitset")]
pub struct Simulated;

/// Marks a body as kinematic (moved by external control only — Decision 1).
///
/// An EnableTag bit, like [`Simulated`]. Captured at gather into
/// [`BodyState::kinematic`](crate::resources::BodyState::kinematic) and read by
/// the one-sided contact response; the body's externally-set velocity feeds the
/// response but its pose is NOT advanced by the solver (kinematic MOTION is an
/// intentional deferral, not built yet — same status as the old
/// `BodyType::Kinematic`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[component(storage = "bitset")]
pub struct Kinematic;

/// COLD mass / material properties of a rigid body (plan D5) — a SEPARATE
/// column from [`RigidBody`].
///
/// Read by the solve, never by the integrate loop, so keeping it out of
/// [`RigidBody`] keeps the hot integrate path's cache lines clean (D5). Stores
/// inverse mass and the inverse inertia TENSOR (so a static body is simply
/// `inv_mass == 0` / `inv_inertia == Mat3::ZERO`, no branch).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct RigidBodyMass {
    /// Inverse inertia TENSOR (`Mat3::ZERO` = infinite inertia). Maps an
    /// angular impulse/torque to the body's angular response (`Δω = I⁻¹ · τ`).
    ///
    /// **Convention (locks the contract for the Phase-10 solver):** this is the
    /// **world-space** inverse inertia tensor, so `Δω = inv_inertia · τ_world`
    /// pairs directly with [`RigidBody`]'s `angular_velocity` and the world-frame
    /// [`Quat::integrate`](crate::math::Quat::integrate) (which premultiplies a
    /// world-frame `ω`). A solver that keeps a body-space tensor must rotate it
    /// into world space (`R · I⁻¹_body · Rᵀ`) before applying torques here.
    pub inv_inertia: Mat3,
    /// Inverse mass (`0` = infinite mass / immovable).
    pub inv_mass: f32,
    /// Coefficient of restitution `e ∈ [0, 1]` (1 = perfectly elastic).
    pub restitution: f32,
    /// Coulomb friction coefficient.
    pub friction: f32,
}

impl Default for RigidBodyMass {
    fn default() -> Self {
        // A unit-mass body with light bounce — the common spawn default. Whether
        // it actually simulates is the runtime `Simulated` bit (Decision 6),
        // defaulted ON by the spawn helpers, not a field here.
        // Identity inverse inertia is the unit-tensor placeholder until a real
        // shape-derived tensor is computed (Phase 10).
        Self {
            inv_inertia: Mat3::IDENTITY,
            inv_mass: 1.0,
            restitution: 0.5,
            friction: 0.3,
        }
    }
}

/// The collision shape of a [`Collider`] (plan D5).
///
/// A zero-`dyn` tagged union (`#[repr(C)]` enum) — no `Box<dyn Shape>`, no
/// vtable, no heap (principle 1). New 3D shapes extend the enum; a real
/// narrowphase matches on the variant.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColliderShape {
    /// A sphere of the given radius (world units).
    Sphere {
        /// Radius in world units.
        radius: f32,
    },
    /// An ORIENTED box (OBB) given by its half-extents in the body's LOCAL frame
    /// (world units). The world box is the body's
    /// [`RigidBody::rotation`](crate::components::RigidBody::rotation) applied to
    /// these local half-extents about [`RigidBody::position`].
    ///
    /// Renamed from `Aabb` in P2 W4 (OQ-3): the box-box / sphere-box generators
    /// treat it as an oriented box (the body carries a `Quat`), so "axis-aligned"
    /// was an active lie. The field name `half_extents` is unchanged.
    Box {
        /// Half-extents along each LOCAL axis (before the body rotation).
        half_extents: Vec3,
    },
}

impl Default for ColliderShape {
    fn default() -> Self {
        Self::Sphere { radius: 0.5 }
    }
}

/// The collider attached to a body (plan D5).
///
/// `layer` / `mask` are the broadphase filter: a pair `(a, b)` is a candidate
/// only when each body's `layer` is in the other's `mask` (foundation
/// broadphase does an unfiltered all-pairs; the fields are wired for the
/// Phase-10 broadphase).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Collider {
    /// The collision shape (zero-`dyn` tagged union).
    pub shape: ColliderShape,
    /// This collider's collision layer bitset.
    pub layer: u32,
    /// Which layers this collider collides against (bitset).
    pub mask: u32,
}

/// Marks a [`Collider`] as a SENSOR / trigger (std-lib S5).
///
/// A sensor collider still participates in broadphase and narrowphase — the
/// engine detects the overlap and records it — but the solver SKIPS contact
/// resolution for any pair where either body carries `Sensor`: no impulse is
/// applied, so neither body's velocity changes. This is the "trigger volume"
/// primitive (a region that reports who is inside it without physically blocking
/// them).
///
/// A `Sensor`-marked overlap is reported through
/// [`Manifolds::sensor_overlaps`](crate::resources::Manifolds::sensor_overlaps),
/// the per-step overlap signal, instead of the
/// [`Manifolds::manifolds`](crate::resources::Manifolds::manifolds) buffer the
/// solver consumes — so the resolved (non-sensor) contact set is byte-identical
/// to a world that never minted a `Sensor` id (the 0%-gate), and the solver's
/// bit-deterministic output is untouched.
///
/// A zero-sized marker (`#[derive(Component)]` over a unit struct), so attaching
/// it is a pure archetype membership bit — no per-body storage, no hot-loop
/// branch beyond the gathered `is_sensor` flag.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sensor;

/// Gameplay-facing contact snapshot (plan D5 / IM-1).
///
/// An OPTIONAL queryable component carrying a [`Manifold`] snapshot plus the
/// `other` body's stable [`EntityId`]. This is the **only** place `EntityId`
/// appears in the physics data — the solve itself reads the dense
/// [`Manifolds`](crate::resources::Manifolds) resource buffer (sequential), not
/// scattered `Contact` components.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug)]
pub struct Contact {
    /// The contact manifold (its `body_a`/`body_b` are per-step
    /// [`BodyIndex`](crate::manifold::BodyIndex) row indices, not entities).
    pub manifold: Manifold,
    /// The stable entity this body is in contact with (projected from the
    /// per-step row index via
    /// [`SolverScratch.entities`](crate::resources::SolverScratch)).
    pub other: EntityId,
}

/// The full component set for spawning a rigid body in one call (plan D5).
///
/// Bundles the hot [`RigidBody`], the cold [`RigidBodyMass`], and the
/// [`Collider`] so a user spawns a complete body with a single bundle. A named
/// struct because the `Bundle` derive rejects tuple/unit/generic bundles.
#[derive(Bundle)]
pub struct RigidBodyBundle {
    /// HOT integrator state.
    pub body: RigidBody,
    /// COLD mass / material.
    pub mass: RigidBodyMass,
    /// Collision shape + filter.
    pub collider: Collider,
}
