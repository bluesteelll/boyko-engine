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
use crate::math::Vec2;

/// HOT integrator state of a rigid body (plan D5) — its own SoA column.
///
/// Contains ONLY the fields the integrate loop touches every step, so
/// `Query<&mut RigidBody>` streams a tight, cache-dense column. Mass/material
/// lives in the separate [`RigidBodyMass`] column.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct RigidBody {
    /// World position (center of mass), in world units.
    pub position: Vec2,
    /// Linear velocity, in world units per second.
    pub linear_velocity: Vec2,
    /// Orientation, in radians.
    pub rotation: f32,
    /// Angular velocity, in radians per second.
    pub angular_velocity: f32,
}

/// The simulation role of a body (plan D5).
///
/// `#[repr(u8)]` so it packs tightly inside the cold [`RigidBodyMass`] column.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BodyType {
    /// Immovable; infinite mass (`inv_mass == 0`). The default.
    #[default]
    Static,
    /// Moved by external control only; ignores forces but participates in
    /// collision against dynamic bodies.
    Kinematic,
    /// Fully simulated; responds to forces and impulses.
    Dynamic,
}

/// COLD mass / material properties of a rigid body (plan D5) — a SEPARATE
/// column from [`RigidBody`].
///
/// Read by the solve, never by the integrate loop, so keeping it out of
/// [`RigidBody`] keeps the hot integrate path's cache lines clean (D5). Stores
/// inverse mass/inertia (so a static body is simply `inv_mass == 0`, no branch).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct RigidBodyMass {
    /// Inverse mass (`0` = infinite mass / immovable).
    pub inv_mass: f32,
    /// Inverse rotational inertia (`0` = infinite inertia).
    pub inv_inertia: f32,
    /// Coefficient of restitution `e ∈ [0, 1]` (1 = perfectly elastic).
    pub restitution: f32,
    /// Coulomb friction coefficient.
    pub friction: f32,
    /// The body's simulation role.
    pub body_type: BodyType,
}

impl Default for RigidBodyMass {
    fn default() -> Self {
        // A unit-mass dynamic body with light bounce — the common spawn default.
        Self {
            inv_mass: 1.0,
            inv_inertia: 1.0,
            restitution: 0.5,
            friction: 0.3,
            body_type: BodyType::Dynamic,
        }
    }
}

/// The collision shape of a [`Collider`] (plan D5).
///
/// A zero-`dyn` tagged union (`#[repr(C)]` enum) — no `Box<dyn Shape>`, no
/// vtable, no heap (principle 1). New 2D shapes extend the enum; a real
/// narrowphase matches on the variant.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColliderShape {
    /// A circle of the given radius (world units).
    Circle {
        /// Radius in world units.
        radius: f32,
    },
    /// An axis-aligned box given by its half-extents (world units).
    Aabb {
        /// Half-extents along each axis.
        half_extents: Vec2,
    },
}

impl Default for ColliderShape {
    fn default() -> Self {
        Self::Circle { radius: 0.5 }
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
