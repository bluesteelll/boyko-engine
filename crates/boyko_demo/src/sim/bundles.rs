//! Component bundles for spawning sim entities (plan §6.2).
//!
//! `#[derive(Bundle)]` requires a NAMED struct (tuple/unit/generic bundles are
//! rejected by the derive — see `boyko_ecs/tests/bundle_compile_fail/`). Each
//! bundle is the full component set of one entity archetype.

use boyko_macros::Bundle;

use crate::render::instance::GpuInstance;
use crate::sim::components::{BallTag, BoidTag, ParticleTag, Position, Radius, Velocity};

/// The archetype of a particle: position, velocity, its GPU mirror, and the
/// mode tag (plan §6.2). `GpuInstance` is carried in the same archetype as the
/// sim data so the upload reads the column directly with no AoS repack
/// (the headline zero-copy path, plan D2).
#[derive(Bundle)]
pub struct ParticleBundle {
    /// World position.
    pub pos: Position,
    /// World velocity.
    pub vel: Velocity,
    /// Per-instance GPU record, written each frame by `sync_gpu_instance`.
    pub gpu: GpuInstance,
    /// Mode-membership marker (plan D16).
    pub tag: ParticleTag,
}

/// The archetype of a boid (plan §6.2 / Wave 5): the same `(pos, vel, gpu)` hot
/// set as a particle but carrying [`BoidTag`] instead of `ParticleTag`, so it
/// lands in a distinct archetype and the per-mode despawn finds exactly its own
/// entities. `GpuInstance` shares the column with the sim data for the same
/// zero-copy upload (plan D2); `sync_gpu_instance` (mode-agnostic) renders it.
#[derive(Bundle)]
pub struct BoidBundle {
    /// World position.
    pub pos: Position,
    /// World velocity.
    pub vel: Velocity,
    /// Per-instance GPU record, written each frame by `sync_gpu_instance`.
    pub gpu: GpuInstance,
    /// Mode-membership marker (plan D16).
    pub tag: BoidTag,
}

/// The archetype of a physics ball (plan §6.2 / D13 / Wave 6): the `(pos, vel,
/// gpu)` hot set plus a per-ball [`Radius`] (the collision size) and a
/// [`BallTag`] so balls land in a distinct archetype the per-mode despawn finds.
/// `GpuInstance` shares the column with the sim data for the same zero-copy
/// upload (plan D2).
#[derive(Bundle)]
pub struct BallBundle {
    /// World position.
    pub pos: Position,
    /// World velocity.
    pub vel: Velocity,
    /// Collision radius in world units.
    pub radius: Radius,
    /// Per-instance GPU record, written each frame by the physics GPU sync.
    pub gpu: GpuInstance,
    /// Mode-membership marker (plan D16).
    pub tag: BallTag,
}
