//! Headless mode-switch test (plan §11.3 `mode_switch.rs` / Wave 5).
//!
//! Drives the [`Mode`] state machine through a real [`SimRunner`] (thread pool +
//! schedule + transition pass) with NO window, and asserts the end-to-end Wave-5
//! contract:
//!
//! * frame 1 (`on_enter(Particles)` synthesized, D7) spawns the particle set;
//! * queuing `NextState(Mode::Boids)` despawns the particle set and spawns the
//!   boid set on the transition frame (despawn-old `.before` spawn-new, H3);
//! * switching back to `Particles` reverses it.
//!
//! This is the real Phase-17 dogfood for the demo: if it passes, the gated
//! exclusive spawn/despawn systems + `in_state` sim systems + the auto-applied
//! transition all compose correctly through the public API.
//!
//! # Miri
//!
//! `#![cfg(not(miri))]`: drives `Schedule::run` (worker dispatch via
//! `Scope::spawn`, the Phase-9.1 Tree-Borrows deferral), like
//! `tests/phase17_states.rs` and `tests/sim_smoke.rs`.
#![cfg(not(miri))]

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_demo::sim::components::{BoidTag, ParticleTag};
use boyko_demo::sim::modes::{BOID_COUNT, Mode, PARTICLE_COUNT};
use boyko_demo::sim::resources::{InputState, SimParams};
use boyko_demo::sim::runner::SimRunner;

/// One engine fixed step (64 Hz, Phase 20) as an f32 display delta. A power-
/// of-two fraction, so from_secs_f32 converts it EXACTLY to 15,625,000 ns -
/// each step() call below expends exactly one substep with zero remainder.
const FIXED_DT: f32 = 1.0 / 64.0;

/// Number of live entities carrying `ParticleTag`.
fn particle_count(world: &EcsMaster) -> usize {
    world.query_entities(&[ParticleTag::component_id()]).len()
}

/// Number of live entities carrying `BoidTag`.
fn boid_count(world: &EcsMaster) -> usize {
    world.query_entities(&[BoidTag::component_id()]).len()
}

/// Builds a headless world + runner with the sim resources. The runner registers
/// the mode state and all gated systems; the boid-pipeline resources are
/// inserted by `SimRunner::new`.
fn setup() -> (EcsMaster, SimRunner) {
    // Capacity for the larger (particle) population; boids are fewer.
    let mut world = EcsMaster::with_capacity(PARTICLE_COUNT, 2);
    world.insert_resource(InputState::default());
    world.insert_resource(SimParams::default());

    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let runner = SimRunner::new(pool, &mut world);
    (world, runner)
}

/// Frame 1 enters `Particles` (synthesized initial transition, D7) and spawns the
/// particle set; no boids exist yet.
#[test]
fn initial_enter_spawns_particles() {
    let (mut world, mut runner) = setup();

    // Nothing has run yet: no entities.
    assert_eq!(particle_count(&world), 0, "no particles before the first step");
    assert_eq!(boid_count(&world), 0, "no boids before the first step");

    // One frame's display time -> at least one fixed step -> on_enter(Particles).
    let steps = runner.step(&mut world, FIXED_DT);
    assert!(steps > 0, "the runner must run at least one fixed step");

    assert_eq!(
        particle_count(&world),
        PARTICLE_COUNT,
        "on_enter(Particles) spawns the full particle set on frame 1"
    );
    assert_eq!(boid_count(&world), 0, "no boids in Particles mode");
    assert_eq!(*world.state::<Mode>(), Mode::Particles, "state is Particles");
}

/// Switching `Particles -> Boids` despawns the particles and spawns the boids on
/// the transition frame; switching back reverses it (plan §6.6 / H3).
#[test]
fn switch_particles_to_boids_and_back() {
    let (mut world, mut runner) = setup();

    // Frame 1: Particles spawned.
    runner.step(&mut world, FIXED_DT);
    assert_eq!(particle_count(&world), PARTICLE_COUNT, "frame 1: particles spawned");
    assert_eq!(boid_count(&world), 0);

    // Queue the switch and step: the transition pass applies Particles->Boids,
    // on_exit(Particles) despawns the particle set, on_enter(Boids) spawns the
    // boid set — all on this frame.
    world.set_next_state(Mode::Boids);
    runner.step(&mut world, FIXED_DT);

    assert_eq!(*world.state::<Mode>(), Mode::Boids, "state switched to Boids");
    assert_eq!(
        particle_count(&world),
        0,
        "on_exit(Particles) despawned every particle"
    );
    assert_eq!(
        boid_count(&world),
        BOID_COUNT,
        "on_enter(Boids) spawned the full boid set on the transition frame"
    );

    // Switch back: boids despawn, particles respawn.
    world.set_next_state(Mode::Particles);
    runner.step(&mut world, FIXED_DT);

    assert_eq!(*world.state::<Mode>(), Mode::Particles, "state switched back");
    assert_eq!(boid_count(&world), 0, "on_exit(Boids) despawned every boid");
    assert_eq!(
        particle_count(&world),
        PARTICLE_COUNT,
        "re-entering Particles respawns the particle set"
    );
}

/// After a switch to Boids, the boid sim runs: stepping several frames keeps the
/// boid population stable (no despawns/leaks) and the particle set stays empty.
/// Proves the `in_state(Boids)` sim systems run without disturbing membership.
#[test]
fn boids_sim_runs_without_membership_drift() {
    let (mut world, mut runner) = setup();

    runner.step(&mut world, FIXED_DT); // enter Particles
    world.set_next_state(Mode::Boids);
    runner.step(&mut world, FIXED_DT); // -> Boids

    assert_eq!(boid_count(&world), BOID_COUNT, "boids spawned");

    // Run several more frames in Boids mode.
    for _ in 0..10 {
        runner.step(&mut world, FIXED_DT);
    }

    assert_eq!(
        boid_count(&world),
        BOID_COUNT,
        "boid population is stable across sim frames (no leak/despawn)"
    );
    assert_eq!(particle_count(&world), 0, "no particles leak in during Boids mode");
    assert_eq!(*world.state::<Mode>(), Mode::Boids, "still in Boids mode");
}
