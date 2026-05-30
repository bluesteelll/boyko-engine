//! Headless smoke test for the Wave-3 simulation (plan §11.3 `sim_smoke.rs`).
//!
//! Proves the sim is real and window-independent: it builds an `EcsMaster` +
//! the native `SimRunner` (thread pool + schedule), spawns N particles, runs K
//! fixed steps, and asserts that positions advanced and stayed finite and
//! in-box. No window, no wgpu surface — only the ECS + scheduler path runs.
//!
//! This is the MVP's "the headline is honest" check: if this passes, the
//! `Schedule::run` + `par_iter_mut` integration works on its own, and the demo
//! binary merely renders the column the sim produces.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_demo::render::instance::GpuInstance;
use boyko_demo::sim::bundles::ParticleBundle;
use boyko_demo::sim::components::{ParticleTag, Position, Velocity};
use boyko_demo::sim::resources::{DeltaTime, InputState, SimParams};
use boyko_demo::sim::runner::{FIXED_DT, SimRunner};

/// World half-extent the runner clamps positions to (mirrors the demo's
/// `WORLD_HALF_EXTENT`). Particles must stay within this box (walls bounce).
const BOUND: f32 = 100.0;

/// Spawns `count` particles with deterministic, spread-out start states using
/// the direct `create_entity` path (the same path `app::spawn_particles` uses).
///
/// Deterministic (no RNG) so the assertions are stable across runs. Every
/// particle gets a non-zero velocity so integration must move it.
fn spawn(world: &mut EcsMaster, count: usize) {
    let archetype = world.bundle_archetype_id_for::<ParticleBundle>();
    let pos_id = Position::component_id();
    let vel_id = Velocity::component_id();
    let gpu_id = GpuInstance::component_id();
    let tag_id = ParticleTag::component_id();

    for i in 0..count {
        // Spread positions across the box; give each a distinct nonzero velocity.
        let f = i as f32;
        let x = (f * 0.7).rem_euclid(2.0 * BOUND) - BOUND;
        let y = (f * 1.3).rem_euclid(2.0 * BOUND) - BOUND;
        let pos = Position { x, y };
        let vel = Velocity { x: 10.0, y: -7.0 };
        let gpu = GpuInstance::new([x, y], 0.6, [80, 160, 255, 255]);
        let tag = ParticleTag(0);

        world
            .create_entity(
                archetype,
                &[
                    (pos_id, bytemuck::bytes_of(&pos)),
                    (vel_id, bytemuck::bytes_of(&vel)),
                    (gpu_id, bytemuck::bytes_of(&gpu)),
                    (tag_id, bytemuck::bytes_of(&tag)),
                ],
            )
            .expect("create_entity must succeed for the resolved archetype");
    }
}

/// Builds a headless world with the sim resources and `count` particles.
fn setup_world(count: usize) -> EcsMaster {
    let mut world = EcsMaster::with_capacity(count.max(1), 1);
    world.insert_resource(DeltaTime(FIXED_DT));
    world.insert_resource(InputState::default());
    world.insert_resource(SimParams::default());
    spawn(&mut world, count);
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

/// The runner drives a real multi-threaded `Schedule`; after several fixed
/// steps every particle must have moved and remain finite and in-box.
#[test]
fn sim_advances_positions_and_stays_finite() {
    const N: usize = 10_000;
    const STEPS: usize = 30;

    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let mut world = setup_world(N);
    let mut runner = SimRunner::new(pool, &mut world);

    let before = collect_positions(&mut world);
    assert_eq!(before.len(), N, "every spawned particle must be queryable");

    // Drive enough display time that the accumulator runs several fixed steps.
    let mut total_steps = 0u32;
    for _ in 0..STEPS {
        total_steps += runner.step(&mut world, FIXED_DT);
    }
    assert!(total_steps > 0, "the runner must have run at least one fixed step");

    let after = collect_positions(&mut world);
    assert_eq!(after.len(), N, "particle count must be stable across steps");

    // Every position must be finite and inside the world box (walls bounce).
    for p in &after {
        assert!(p.x.is_finite() && p.y.is_finite(), "position went non-finite: {p:?}");
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

    // At least one particle must have actually moved — proves integration ran
    // (not a no-op schedule). With a uniform nonzero velocity, effectively all
    // of them move; require a strong majority to be robust against any that
    // happen to bounce back to their start.
    let moved = before
        .iter()
        .zip(&after)
        .filter(|(b, a)| (b.x - a.x).abs() > 1e-4 || (b.y - a.y).abs() > 1e-4)
        .count();
    assert!(
        moved > N / 2,
        "integration must move most particles ({moved} of {N} moved)"
    );
}

/// `sync_gpu_instance` must populate the `GpuInstance` column from the sim
/// state — after stepping, the GPU mirror's position matches `Position`. This
/// is the zero-copy upload's correctness precondition: the column the renderer
/// reads holds live data.
#[test]
fn gpu_instance_mirrors_position_after_step() {
    const N: usize = 2_000;

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = setup_world(N);
    let mut runner = SimRunner::new(pool, &mut world);

    // One frame's worth of display time guarantees at least one fixed step.
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

    assert_eq!(seen, N, "every particle must appear in the joined query");
    assert_eq!(
        mismatches, 0,
        "sync_gpu_instance must mirror Position into GpuInstance.pos"
    );
}
