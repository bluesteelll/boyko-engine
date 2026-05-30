//! ECS components for the simulation (plan §6.1).
//!
//! Each component is its own SoA column inside the archetype; the engine stores
//! columns contiguously, which is exactly what the zero-copy GPU upload relies
//! on (plan D2). All components are `#[repr(C)]` so their byte layout is stable;
//! the ones that are uploaded ([`Position`], and `GpuInstance` in
//! [`crate::render::instance`]) are additionally `bytemuck::Pod`.
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
/// A true ZST is rejected by the component pool (`component_pool.rs` debug-asserts
/// `size > 0`), so this is a 1-byte tag rather than an empty struct. It marks an
/// entity as belonging to the Particles mode for despawn-on-exit in later waves;
/// for the Wave-3 MVP it simply tags every spawned particle.
///
/// `Pod` is derived (it is a `#[repr(C)]` single `u8`, trivially Pod) so the
/// startup spawn path can hand its bytes to `create_entity` via
/// `bytemuck::bytes_of` with no `unsafe` (see `app::spawn_particles`).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Pod, Zeroable)]
pub struct ParticleTag(pub u8);

/// Mode-membership marker for boid entities (plan D16 / §9 G3 / Wave 5).
///
/// The Boids-mode analogue of [`ParticleTag`]: a 1-byte tag (a true ZST is
/// rejected by the component pool) carried by every boid so the despawn-on-exit
/// system can find them via `query_entities(&[BoidTag::component_id()])`. `Pod`
/// for the same `bytemuck::bytes_of` spawn path.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Pod, Zeroable)]
pub struct BoidTag(pub u8);

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
/// The Physics-mode analogue of [`ParticleTag`]/[`BoidTag`]: a 1-byte tag (a true
/// ZST is rejected by the component pool) carried by every ball so the
/// despawn-on-exit system can find them via
/// `query_entities(&[BallTag::component_id()])`. `Pod` for the same
/// `bytemuck::bytes_of` spawn path.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Pod, Zeroable)]
pub struct BallTag(pub u8);

