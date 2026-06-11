//! Headless smoke test for the simulation (plan §11.3 `sim_smoke.rs`).
//!
//! Proves the sim is real and window-independent: it builds an `EcsMaster` +
//! the native `SimRunner` (thread pool + schedule), runs K fixed steps, and
//! asserts that positions advanced and stayed finite and in-box. No window, no
//! wgpu surface — only the ECS + scheduler path runs.
//!
//! This is the MVP's "the headline is honest" check: if this passes, the
//! `Schedule::run` + `par_iter_mut` integration works on its own, and the demo
//! binary merely renders the column the sim produces.
//!
//! # Wave 5 update — the runner owns startup spawning
//!
//! Through Wave 4 these tests spawned a fixed-N particle set BY HAND, then built
//! the runner. Wave 5 moved startup population into the mode state machine:
//! `SimRunner::new` registers `Mode::Particles` as the initial state, so the
//! first `Schedule::run` fires the synthesized `on_enter(Particles)` transition
//! (Phase 17 D7), which spawns `modes::PARTICLE_COUNT` particles via the runner
//! itself (plan §10 W5). A test that also spawned by hand would double-populate.
//!
//! So these tests no longer spawn directly: they build the runner, step once to
//! let `on_enter(Particles)` populate, and assert against `PARTICLE_COUNT`. This
//! is the faithful headless mirror of what the binary does on frame 1.

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_demo::render::instance::GpuInstance;
use boyko_demo::sim::components::Position;
use boyko_demo::sim::modes::PARTICLE_COUNT;
use boyko_demo::sim::resources::{InputState, SimParams};
use boyko_demo::sim::runner::SimRunner;

/// One engine fixed step (64 Hz, Phase 20) as an f32 display delta. A power-
/// of-two fraction, so from_secs_f32 converts it EXACTLY to 15,625,000 ns -
/// each step() call below expends exactly one substep with zero remainder.
const FIXED_DT: f32 = 1.0 / 64.0;

/// World half-extent the runner clamps positions to (mirrors the demo's
/// `WORLD_HALF_EXTENT`). Particles must stay within this box (walls bounce).
const BOUND: f32 = 100.0;

/// Builds a headless world with the sim resources, sized for the particle
/// population the runner's `on_enter(Particles)` spawns on the first step.
fn setup_world() -> EcsMaster {
    let mut world = EcsMaster::with_capacity(PARTICLE_COUNT, 2);
    world.insert_resource(InputState::default());
    world.insert_resource(SimParams::default());
    world
}

/// Collects every particle position via the direct query API (the same
/// `for_each_chunk` path the renderer's upload uses).
fn collect_positions(world: &mut EcsMaster) -> Vec<Position> {
    let mut out = Vec::new();
    world
        .query::<&Position, ()>()
        .for_each_chunk(|slice: &[Position]| out.extend_from_slice(slice));
    out
}

/// The runner drives a real multi-threaded `Schedule`; after the initial
/// `on_enter(Particles)` spawn and several fixed steps every particle must have
/// moved and remain finite and in-box.
#[test]
fn sim_advances_positions_and_stays_finite() {
    const STEPS: usize = 30;

    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let mut world = setup_world();
    let mut runner = SimRunner::new(pool, &mut world);

    // Step once: the synthesized on_enter(Particles) spawns the cloud, and the
    // particle integrator runs the same frame.
    let first = runner.step(&mut world, FIXED_DT);
    assert!(first > 0, "the runner must run at least one fixed step");

    let before = collect_positions(&mut world);
    assert_eq!(
        before.len(),
        PARTICLE_COUNT,
        "on_enter(Particles) must spawn the full particle set on the first step"
    );

    // Drive more display time so the accumulator runs several more fixed steps.
    for _ in 0..STEPS {
        runner.step(&mut world, FIXED_DT);
    }

    let after = collect_positions(&mut world);
    assert_eq!(
        after.len(),
        PARTICLE_COUNT,
        "particle count must be stable across steps"
    );

    // Every position must be finite and inside the world box (walls bounce).
    for p in &after {
        assert!(
            p.x.is_finite() && p.y.is_finite(),
            "position went non-finite: {p:?}"
        );
        assert!(
            p.x >= -BOUND - 1.0 && p.x <= BOUND + 1.0,
            "x escaped the world box: {}",
            p.x
        );
        assert!(
            p.y >= -BOUND - 1.0 && p.y <= BOUND + 1.0,
            "y escaped the world box: {}",
            p.y
        );
    }

    // Most particles must have moved between the first step and the last —
    // proves integration ran (not a no-op schedule). Random spawn velocities are
    // nonzero, so effectively all move; require a strong majority to be robust
    // against any that happen to bounce back near their start.
    let moved = before
        .iter()
        .zip(&after)
        .filter(|(b, a)| (b.x - a.x).abs() > 1e-4 || (b.y - a.y).abs() > 1e-4)
        .count();
    assert!(
        moved > PARTICLE_COUNT / 2,
        "integration must move most particles ({moved} of {PARTICLE_COUNT} moved)"
    );
}

/// `sync_gpu_instance` must populate the `GpuInstance` column from the sim
/// state — after stepping, the GPU mirror's position matches `Position`. This
/// is the zero-copy upload's correctness precondition: the column the renderer
/// reads holds live data.
#[test]
fn gpu_instance_mirrors_position_after_step() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = setup_world();
    let mut runner = SimRunner::new(pool, &mut world);

    // One step: on_enter(Particles) spawns, integrator moves, sync_gpu_instance
    // writes the GpuInstance column — all in dependency order in the schedule.
    let steps = runner.step(&mut world, FIXED_DT);
    assert!(steps > 0, "expected at least one fixed step");

    // Collect (Position, GpuInstance) pairs; the sync system must have written
    // each GpuInstance.pos to match its Position.
    let mut mismatches = 0usize;
    let mut seen = 0usize;
    world
        .query::<(&Position, &GpuInstance), ()>()
        .for_each_chunk(|(positions, gpus): (&[Position], &[GpuInstance])| {
            for (p, g) in positions.iter().zip(gpus) {
                seen += 1;
                if (g.pos[0] - p.x).abs() > 1e-4 || (g.pos[1] - p.y).abs() > 1e-4 {
                    mismatches += 1;
                }
            }
        });

    assert_eq!(
        seen, PARTICLE_COUNT,
        "every spawned particle must appear in the joined query"
    );
    assert_eq!(
        mismatches, 0,
        "sync_gpu_instance must mirror Position into GpuInstance.pos"
    );
}
