//! ECS components for the simulation (plan §6.1).
//!
//! Each component is its own SoA column inside the archetype; the engine stores
//! columns contiguously, which is exactly what the zero-copy GPU upload relies
//! on (plan D2). Data components are `#[repr(C)]` so their byte layout is
//! stable; the ones that are uploaded ([`Position`], and `GpuInstance` in
//! [`crate::render::instance`]) are additionally `bytemuck::Pod`. The mode
//! markers ([`ParticleTag`], [`BoidTag`], [`BallTag`]) are real ZST tags
//! (Phase 22): tick-only pools, zero bytes per row, no layout to pin.
//!
//! The boyko `Component` derive is a pure marker — it adds no fields and only
//! assigns a lazily-allocated `ComponentId` — so it coexists with `Pod`
//! (plan §9 G1).

use bytemuck::{Pod, Zeroable};

use boyko_macros::Component;

/// Entity position in world units (plan §5.3: the world is the box
/// `[-WORLD_HALF_EXTENT, WORLD_HALF_EXTENT]^2`).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Pod, Zeroable)]
pub struct Position {
    /// X coordinate in world units.
    pub x: f32,
    /// Y coordinate in world units.
    pub y: f32,
}

/// Entity velocity in world units per second.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Pod, Zeroable)]
pub struct Velocity {
    /// X velocity in world units per second.
    pub x: f32,
    /// Y velocity in world units per second.
    pub y: f32,
}

/// Mode-membership marker for particle entities (plan D16 / §9 G3).
///
/// A real ZST tag (Phase 22): the engine stores it in a tick-only pool — no
/// data column, no bytes per row. It marks an entity as belonging to the
/// Particles mode for despawn-on-exit. The direct `create_entity` spawn path
/// contributes a 0-length byte slice for it (`(tag_id, &[])`); no `Pod`
/// ceremony is needed because there is no payload to serialize.
#[derive(Component, Clone, Copy, Debug)]
pub struct ParticleTag;

/// Mode-membership marker for boid entities (plan D16 / §9 G3 / Wave 5).
///
/// The Boids-mode analogue of [`ParticleTag`]: a real ZST tag (Phase 22)
/// carried by every boid so the despawn-on-exit system can find them via
/// `query_entities(&[BoidTag::component_id()])`.
#[derive(Component, Clone, Copy, Debug)]
pub struct BoidTag;

/// Per-ball collision radius in world units (plan §6.1 / D13 / Wave 6).
///
/// Its own SoA column, read by the physics broad/narrow phases. `Pod` (a
/// `#[repr(C)]` single `f32`, trivially Pod) so the direct ball spawn path can
/// hand its bytes to `create_entity` via `bytemuck::bytes_of` with no `unsafe`.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Pod, Zeroable)]
pub struct Radius(pub f32);

/// Mode-membership marker for physics-ball entities (plan D16 / §9 G3 / Wave 6).
///
/// The Physics-mode analogue of [`ParticleTag`]/[`BoidTag`]: a real ZST tag
/// (Phase 22) carried by every ball so the despawn-on-exit system can find
/// them via `query_entities(&[BallTag::component_id()])`, and so the physics
/// systems can scope their queries with `With<BallTag>`.
#[derive(Component, Clone, Copy, Debug)]
pub struct BallTag;

#[cfg(test)]
mod tests {
    use super::*;

    /// Phase 22: the mode markers are real ZSTs (tick-only pools). A
    /// regression to a sized payload would silently reintroduce per-row bytes
    /// and break the `(tag_id, &[])` contract every direct spawn path
    /// (`app::ParticleSpawner::spawn_one`, `modes::scatter_spawn`,
    /// `modes::spawn_balls`) relies on.
    #[test]
    fn mode_tags_are_zero_sized() {
        assert_eq!(size_of::<ParticleTag>(), 0, "ParticleTag must be a ZST");
        assert_eq!(size_of::<BoidTag>(), 0, "BoidTag must be a ZST");
        assert_eq!(size_of::<BallTag>(), 0, "BallTag must be a ZST");
    }
}

