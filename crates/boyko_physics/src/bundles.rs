//! Object-category physics bundle presets (standard-library Phase S6).
//!
//! These mix `boyko_scene`'s spatial / render-capability components with this
//! crate's physics components. The layering is sound: `boyko_physics` depends on
//! `boyko_scene` (a downward edge), so naming scene types here is cycle-free; the
//! reverse (scene naming physics types) would cycle, which is why these bundles
//! live here, not in `boyko_scene`.
//!
//! Both are named `#[derive(Bundle)]` structs (the derive rejects tuples /
//! generics), so a repeated spawn hits the per-impl Phase-8.5 static bundle cache.
//!
//! # The GPU instance + per-frame enable bit are NOT bundle fields
//!
//! As with the scene bundles, neither `RenderEnabled` (a bitset tag) nor the dense
//! `Gpu3dInstance` column is a field here — the render layer attaches them after
//! spawning. A spawned [`DynamicBody`] becomes drawable by additionally inserting
//! a `Gpu3dInstance` and enabling `RenderEnabled` on the entity.

use boyko_macros::Bundle;
use boyko_scene::{GlobalTransform, MaterialHandle, MeshHandle, Transform, Visibility};

use crate::components::{Collider, RigidBody, RigidBodyMass, Sensor};

/// A fully simulated, drawable dynamic body (arity 8).
///
/// The complete authoring set for "a physics object you can see": the spatial
/// pose ([`Transform`] / [`GlobalTransform`]), the render handles ([`MeshHandle`] /
/// [`MaterialHandle`] / [`Visibility`]), and the physics columns ([`RigidBody`] /
/// [`RigidBodyMass`] / [`Collider`]). The pose is kept in ONE datum — physics
/// writes [`RigidBody`], `sync_body_to_transform` mirrors it into [`Transform`],
/// and `propagate_transforms` derives [`GlobalTransform`] (Principle 0, no
/// parallel pose store).
#[derive(Bundle)]
pub struct DynamicBody {
    /// Local pose (designer-facing); mirrored from the body each step.
    pub transform: Transform,
    /// Cached world pose, filled by `propagate_transforms`.
    pub global: GlobalTransform,
    /// Mesh asset handle.
    pub mesh: MeshHandle,
    /// Material asset handle.
    pub material: MaterialHandle,
    /// HOT integrator state.
    pub body: RigidBody,
    /// COLD mass / material.
    pub mass: RigidBodyMass,
    /// Collision shape + filter.
    pub collider: Collider,
    /// Persisted authoring visibility.
    pub visibility: Visibility,
}

/// A trigger / sensor volume: a placed collider that reports overlaps without
/// blocking (arity 4).
///
/// The [`Sensor`] marker makes the solver SKIP contact resolution for any pair
/// touching this body — overlaps are reported through the per-step sensor signal
/// instead, so the resolved (non-sensor) contact set is byte-identical to a world
/// that never minted this id. No [`RigidBody`] / [`RigidBodyMass`]: a trigger is
/// not integrated.
#[derive(Bundle)]
pub struct Trigger {
    /// Local pose (designer-facing).
    pub transform: Transform,
    /// Cached world pose, filled by `propagate_transforms`.
    pub global: GlobalTransform,
    /// Collision shape + filter (the trigger volume).
    pub collider: Collider,
    /// The sensor marker (overlap-reporting, non-blocking).
    pub sensor: Sensor,
}
