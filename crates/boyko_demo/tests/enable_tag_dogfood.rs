//! Headless dogfood for the enable-bit (bitset) tag backend (EnableTag Wave 6 /
//! Step 11).
//!
//! Drives the real native [`SimRunner`] (thread pool + schedule + transition
//! pass) with NO window and proves the [`Frozen`] enable-bit tag works on a live
//! workload:
//!
//! * after the first step, the particle cloud is partitioned by the `Frozen`
//!   per-row bit into a non-empty frozen subset + the rest, summing to the full
//!   population (the O(1) toggle ran in `freeze_pulse`);
//! * the typed `Query<&Position, Enabled<Frozen>>` / `Disabled<Frozen>` filters
//!   select exactly those two subsets (the query-filter side of the dogfood);
//! * frozen particles do NOT move across further steps (the integrator's
//!   `Disabled<Frozen>` filter skips them) while the non-frozen ones advance.
//!
//! If this passes, the enable-bit toggle + `Enabled`/`Disabled` query compose
//! correctly through the public API in a real fixed-timestep frame loop.
//!
//! # Miri
//!
//! `#![cfg(not(miri))]`: drives `Schedule::run` (worker dispatch via
//! `Scope::spawn`, the Phase-9.1 Tree-Borrows deferral), like `tests/sim_smoke.rs`
//! and `tests/mode_switch.rs`.
#![cfg(not(miri))]

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Disabled, Enabled};
use boyko_threadpool::ThreadPoolBuilder;

use boyko_demo::sim::components::{Frozen, ParticleTag, Position};
use boyko_demo::sim::modes::PARTICLE_COUNT;
use boyko_demo::sim::resources::{InputState, SimParams};
use boyko_demo::sim::runner::SimRunner;

/// One engine fixed step (64 Hz, Phase 20) as an f32 display delta. A power-of-
/// two fraction, so `from_secs_f32` converts it EXACTLY to 15,625,000 ns — each
/// `step()` call expends exactly one substep with zero remainder.
const FIXED_DT: f32 = 1.0 / 64.0;

/// Builds a headless world + native runner sized for the particle population.
fn setup() -> (EcsMaster, SimRunner) {
    let mut world = EcsMaster::with_capacity(PARTICLE_COUNT, 2);
    world.insert_resource(InputState::default());
    world.insert_resource(SimParams::default());

    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let runner = SimRunner::new(pool, &mut world);
    (world, runner)
}

/// After one step the cloud is partitioned by the `Frozen` per-row bit: a
/// non-empty frozen subset plus the rest, summing to the full population. The
/// typed `Enabled<Frozen>` / `Disabled<Frozen>` filters select exactly those
/// subsets — the query-filter side of the dogfood.
#[test]
fn freeze_pulse_partitions_cloud_by_enable_bit() {
    let (mut world, mut runner) = setup();

    let steps = runner.step(&mut world, FIXED_DT);
    assert!(steps > 0, "the runner must run at least one fixed step");

    // Ground truth via the per-entity `is_enabled` probe.
    let particles = world.query_entities(&[ParticleTag::component_id()]);
    assert_eq!(
        particles.len(),
        PARTICLE_COUNT,
        "on_enter(Particles) spawns the full particle set on frame 1"
    );
    let frozen_probe = particles
        .iter()
        .filter(|&&e| world.is_enabled::<Frozen>(e))
        .count();
    assert!(
        frozen_probe > 0,
        "freeze_pulse must freeze a non-empty subset of particles"
    );
    assert!(
        frozen_probe < PARTICLE_COUNT,
        "freeze_pulse must leave most particles unfrozen"
    );

    // The typed query filters must agree with the per-entity probe.
    let enabled_rows = world.query::<&Position, Enabled<Frozen>>().iter().count();
    let disabled_rows = world.query::<&Position, Disabled<Frozen>>().iter().count();
    assert_eq!(
        enabled_rows, frozen_probe,
        "Enabled<Frozen> must visit exactly the frozen rows"
    );
    assert_eq!(
        enabled_rows + disabled_rows,
        PARTICLE_COUNT,
        "Enabled<Frozen> and Disabled<Frozen> must partition the whole cloud"
    );
}

/// The integrator's `Disabled<Frozen>` filter skips frozen particles: across
/// further steps a frozen particle holds its position, while the non-frozen
/// cloud advances. This is the dogfood's payoff — a per-row enable bit gating a
/// real `par_iter_mut` system body.
#[test]
fn frozen_particles_do_not_move() {
    let (mut world, mut runner) = setup();

    // Step once to spawn + run the first freeze_pulse + integrate.
    runner.step(&mut world, FIXED_DT);

    // Capture the frozen subset's entities + positions. `freeze_pulse` re-applies
    // the SAME stride each step, so this set is stable across the steps below.
    let particles = world.query_entities(&[ParticleTag::component_id()]);
    let frozen: Vec<_> = particles
        .iter()
        .copied()
        .filter(|&e| world.is_enabled::<Frozen>(e))
        .collect();
    assert!(!frozen.is_empty(), "expected a non-empty frozen subset");

    let frozen_before: Vec<Position> = frozen
        .iter()
        .map(|&e| {
            *world
                .get_component::<Position>(e)
                .expect("a live particle has a Position")
        })
        .collect();

    // Pick a non-frozen witness to prove the integrator IS running (so the
    // frozen-stay assertion is not vacuously true on a stalled schedule).
    let mover = particles
        .iter()
        .copied()
        .find(|&e| !world.is_enabled::<Frozen>(e))
        .expect("expected at least one non-frozen particle");
    let mover_before = *world
        .get_component::<Position>(mover)
        .expect("a live particle has a Position");

    // Advance several more fixed steps.
    for _ in 0..16 {
        runner.step(&mut world, FIXED_DT);
    }

    // Every frozen particle must be byte-for-byte at its captured position: the
    // `Disabled<Frozen>` filter skipped it every step.
    for (&e, before) in frozen.iter().zip(&frozen_before) {
        let after = *world
            .get_component::<Position>(e)
            .expect("the frozen particle is still live");
        assert_eq!(
            after.x, before.x,
            "a frozen particle's x must not move (integrator skipped it)"
        );
        assert_eq!(
            after.y, before.y,
            "a frozen particle's y must not move (integrator skipped it)"
        );
    }

    // The non-frozen witness must have advanced — the integrator ran.
    let mover_after = *world
        .get_component::<Position>(mover)
        .expect("the non-frozen particle is still live");
    assert!(
        (mover_after.x - mover_before.x).abs() > 1e-4
            || (mover_after.y - mover_before.y).abs() > 1e-4,
        "a non-frozen particle must move — proves the integrator ran"
    );
}
