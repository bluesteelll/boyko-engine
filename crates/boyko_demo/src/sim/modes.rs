//! Mode (Particles / Boids) state machine and its spawn/despawn systems
//! (plan §6.6 / D15 / D16 / Wave 5).
//!
//! The demo's modes are a Phase-17 [`States`] type, [`Mode`]. A mode button
//! writes `NextState<Mode>` (the UI), `Schedule::run` auto-applies the
//! transition, and three classes of gated system react:
//!
//! * **Spawn-on-enter** (`.run_if(on_enter(Mode::X))`): an EXCLUSIVE
//!   `fn(&mut EcsMaster)` that populates the new mode's set via direct
//!   `create_entity` — no `Commands` 8192/call cap (plan §9 G5 / C1).
//! * **Despawn-on-exit** (`.run_if(on_exit(Mode::X))`): an EXCLUSIVE
//!   `fn(&mut EcsMaster)` that `query_entities(&[Tag::component_id()])` then
//!   `delete_entity` each — the despawn-by-tag path (plan D16 / §9 G7).
//! * **Per-mode sim** (`.run_if(in_state(Mode::X))`): the ordinary function
//!   systems for that mode (integration, forces, …).
//!
//! The STEP-0 gate (`tests/state_exclusive_smoke.rs`) proved the exclusive +
//! `.run_if(on_exit/on_enter)` combination compiles and fires exactly on the
//! transition frame, so this module uses the critic's RECOMMENDED default (C2)
//! rather than the `Commands::despawn` fallback.
//!
//! Why exclusive (C2): `query_entities` is `&self` and `delete_entity`/
//! `create_entity` are `&mut self`, so a body calling them directly on `world`
//! must be an exclusive system (universal access). It runs only when
//! `running == 0`, serializing the frame — acceptable because it fires only on
//! transition frames (gated by `on_enter`/`on_exit`).

use rand::Rng;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::state::states::States;

use crate::render::WORLD_HALF_EXTENT;
use crate::render::instance::GpuInstance;
use crate::sim::bundles::{BoidBundle, ParticleBundle};
use crate::sim::components::{BoidTag, ParticleTag, Position, Velocity};

/// The interactive simulation modes (plan §6.6). A Phase-17 state type: the UI
/// queues a switch via `NextState<Mode>` and the gated spawn/despawn/sim systems
/// react. `Physics` is added in Wave 6.
///
/// `Default` is `Particles` so the app can `insert_state(Mode::default())` and
/// the synthesized initial transition (Phase 17 D7) fires `on_enter(Particles)`
/// on frame 1 — which is where the startup population is spawned (plan §10 W5).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Mode {
    /// The interactive particle sandbox (Waves 3-4): gravity well + click-spawn.
    #[default]
    Particles,
    /// The boids/flocking mode (Wave 5): separation / alignment / cohesion.
    Boids,
}

impl States for Mode {}

/// Number of particles spawned when entering [`Mode::Particles`] (plan §1.2 MVP:
/// 100k+). Spawned via direct `create_entity` (C1), so the full population
/// exists on the transition frame with no per-frame apply delay.
pub const PARTICLE_COUNT: usize = 100_000;

/// Number of boids spawned when entering [`Mode::Boids`] (plan §6.5: "realistic
/// N ~tens of thousands"). Smaller than the particle count because each boid does
/// a 3x3 neighbor scan per step (O(n*k) vs the particles' O(n)).
pub const BOID_COUNT: usize = 30_000;

/// Initial speed range for spawned particles, in world units per second.
const PARTICLE_INITIAL_SPEED: f32 = 40.0;

/// Initial speed range for spawned boids, in world units per second. Boids start
/// with a real heading so the flock organizes immediately rather than from rest.
const BOID_INITIAL_SPEED: f32 = 40.0;

/// Pre-resolved ids for a one-archetype spawn path (plan §10 W5 / C1).
///
/// Resolving the archetype + component ids once (the archetype is registered on
/// first `bundle_archetype_id_for`) keeps the per-entity `create_entity` loop
/// free of registry lookups. Generic over the spawn closure's needs via the
/// explicit id fields rather than a bundle value, because the direct
/// `create_entity` path takes raw `(ComponentId, &[u8])` pairs (the `Bundle`
/// derive is only consumable through `Commands`).
struct SpawnIds {
    /// Target archetype (registered on resolve).
    archetype: boyko_ecs::ecs::identifiers::primitives::ArchetypeId,
    /// `Position` column id.
    pos: boyko_ecs::ecs::identifiers::primitives::ComponentId,
    /// `Velocity` column id.
    vel: boyko_ecs::ecs::identifiers::primitives::ComponentId,
    /// `GpuInstance` column id.
    gpu: boyko_ecs::ecs::identifiers::primitives::ComponentId,
    /// Mode-tag column id (`ParticleTag` or `BoidTag`).
    tag: boyko_ecs::ecs::identifiers::primitives::ComponentId,
}

/// Spawns `count` entities scattered across the world box with random initial
/// velocities, via the direct `create_entity` path (C1 / plan §9 G5).
///
/// `ids` carries the pre-resolved archetype + column ids; `tag_byte` is the
/// 1-byte tag value written into the mode-tag column. Component bytes come from
/// `bytemuck::bytes_of` (every component here is `Pod`), so the spawn is
/// `unsafe`-free. Stops early if the pool reports full mid-spawn (capacity), so a
/// failed spawn is a capacity condition, not a panic.
fn scatter_spawn(world: &mut EcsMaster, ids: &SpawnIds, count: usize, speed: f32, tag_byte: u8) {
    let mut rng = rand::rng();
    for _ in 0..count {
        let x = rng.random_range(-WORLD_HALF_EXTENT..WORLD_HALF_EXTENT);
        let y = rng.random_range(-WORLD_HALF_EXTENT..WORLD_HALF_EXTENT);
        let vx = rng.random_range(-speed..speed);
        let vy = rng.random_range(-speed..speed);
        let pos = Position { x, y };
        let vel = Velocity { x: vx, y: vy };
        // Seed a sane GpuInstance so the first frame before `sync_gpu_instance`
        // does not flash zeroed instances.
        let gpu = GpuInstance::new([x, y], 0.6, [80, 160, 255, 255]);
        let tag = tag_byte;
        let ok = world
            .create_entity(
                ids.archetype,
                &[
                    (ids.pos, bytemuck::bytes_of(&pos)),
                    (ids.vel, bytemuck::bytes_of(&vel)),
                    (ids.gpu, bytemuck::bytes_of(&gpu)),
                    (ids.tag, bytemuck::bytes_of(&tag)),
                ],
            )
            .is_ok();
        if !ok {
            // Pool full — stop; the population is whatever fit.
            break;
        }
    }
}

/// Despawns every entity carrying `tag_id` (plan D16 / §9 G7).
///
/// `query_entities(&[tag_id])` (a `&self` archetype scan) returns the live
/// entities of the mode; `delete_entity` removes each. Two-statement borrow
/// split (collect ids, then delete) because `query_entities` borrows `&self`
/// while `delete_entity` needs `&mut self`.
//
// `clippy::needless_pass_by_ref_mut`: `delete_entity` is `&mut self`, so the
// `&mut EcsMaster` IS required — but the lint cannot see through the cross-crate
// method call and false-positively suggests `&EcsMaster`, which would not
// compile. Allowed with this justification.
#[allow(clippy::needless_pass_by_ref_mut)]
fn despawn_tagged(
    world: &mut EcsMaster,
    tag_id: boyko_ecs::ecs::identifiers::primitives::ComponentId,
) {
    let ids = world.query_entities(&[tag_id]);
    for entity in ids {
        world.delete_entity(entity);
    }
}

/// Spawn-on-enter system for [`Mode::Particles`] (plan §6.6).
///
/// EXCLUSIVE `fn(&mut EcsMaster)` (universal access) gated
/// `.run_if(on_enter(Mode::Particles))` by the runner. Resolves the particle
/// archetype + ids, then scatter-spawns [`PARTICLE_COUNT`] particles directly.
/// Fires on frame 1 via the synthesized initial transition (D7) since
/// [`Mode::default`] is `Particles`.
pub fn spawn_particles(world: &mut EcsMaster) {
    let ids = SpawnIds {
        archetype: world.bundle_archetype_id_for::<ParticleBundle>(),
        pos: Position::component_id(),
        vel: Velocity::component_id(),
        gpu: GpuInstance::component_id(),
        tag: ParticleTag::component_id(),
    };
    scatter_spawn(world, &ids, PARTICLE_COUNT, PARTICLE_INITIAL_SPEED, 0);
}

/// Despawn-on-exit system for [`Mode::Particles`] (plan D16).
///
/// EXCLUSIVE `fn(&mut EcsMaster)` gated `.run_if(on_exit(Mode::Particles))`.
/// Removes every entity carrying [`ParticleTag`] (including click-spawned ones —
/// they share the tag) so the Boids mode starts from a clean world.
pub fn despawn_particles(world: &mut EcsMaster) {
    despawn_tagged(world, ParticleTag::component_id());
}

/// Spawn-on-enter system for [`Mode::Boids`] (plan §6.6 / Wave 5).
///
/// EXCLUSIVE `fn(&mut EcsMaster)` gated `.run_if(on_enter(Mode::Boids))`.
/// Scatter-spawns [`BOID_COUNT`] boids with random headings (so the flock
/// organizes immediately) into the boid archetype.
pub fn spawn_boids(world: &mut EcsMaster) {
    let ids = SpawnIds {
        archetype: world.bundle_archetype_id_for::<BoidBundle>(),
        pos: Position::component_id(),
        vel: Velocity::component_id(),
        gpu: GpuInstance::component_id(),
        tag: BoidTag::component_id(),
    };
    scatter_spawn(world, &ids, BOID_COUNT, BOID_INITIAL_SPEED, 0);
}

/// Despawn-on-exit system for [`Mode::Boids`] (plan D16).
///
/// EXCLUSIVE `fn(&mut EcsMaster)` gated `.run_if(on_exit(Mode::Boids))`.
/// Removes every entity carrying [`BoidTag`].
pub fn despawn_boids(world: &mut EcsMaster) {
    despawn_tagged(world, BoidTag::component_id());
}
