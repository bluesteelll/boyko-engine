//! Component bundles for spawning sim entities (plan §6.2).
//!
//! `#[derive(Bundle)]` requires a NAMED struct (tuple/unit/generic bundles are
//! rejected by the derive — see `boyko_ecs/tests/bundle_compile_fail/`). Each
//! bundle is the full component set of one entity archetype.

use boyko_macros::Bundle;

use crate::render::instance::GpuInstance;
use crate::sim::components::{ParticleTag, Position, Velocity};

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
