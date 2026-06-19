//! Physics O11 SP1 micro-bench: the single-threaded XPBD soft-body step
//! (`physics_soft_step`) over {64, 512} particles.
//!
//! This is the SP4-parallel BASELINE: SP1 is distance-constraint-only and
//! single-threaded, so this reports the per-step cost of predict → one
//! Gauss-Seidel distance-constraint pass → one-sided SDF collide → velocity
//! update on a grid-of-particles cloth fixture, driven through the real
//! `SystemParam` fetch via `run_system_once` (the kernel in isolation, excluding
//! the `Schedule::run` dispatch tax). It is NOT a pass/fail gate — SP1 is the
//! correctness foundation; the numbers establish the baseline the SP4 parallel
//! coloring is measured against.
//!
//! ```text
//! cargo bench -p boyko-physics --bench soft_step
//! ```

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::into_system::IntoSystem;

use boyko_physics::math::Vec3;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::sdf_query::SdfField;
use boyko_physics::soft::SoftBody;
use boyko_physics::soft::physics_soft_step;

use boyko_sdf_math::{SdfEdit, sdf_op};

/// A `w × h` grid of particles in the xz plane (a cloth sheet) at height `y`,
/// spacing `1.0`, braced by structural + both face-diagonal edges per cell — a
/// realistic distance-constraint topology (≈ 4 constraints per interior particle).
fn grid_cloth(w: usize, h: usize, y: f32) -> (Vec<[f32; 3]>, Vec<(u32, u32)>) {
    let idx = |x: usize, z: usize| (z * w + x) as u32;
    let mut positions = Vec::with_capacity(w * h);
    for z in 0..h {
        for x in 0..w {
            positions.push([x as f32 - w as f32 * 0.5, y, z as f32 - h as f32 * 0.5]);
        }
    }
    let mut edges = Vec::new();
    for z in 0..h {
        for x in 0..w {
            if x + 1 < w {
                edges.push((idx(x, z), idx(x + 1, z))); // +x structural
            }
            if z + 1 < h {
                edges.push((idx(x, z), idx(x, z + 1))); // +z structural
            }
            if x + 1 < w && z + 1 < h {
                edges.push((idx(x, z), idx(x + 1, z + 1))); // diagonal
                edges.push((idx(x + 1, z), idx(x, z + 1))); // anti-diagonal
            }
        }
    }
    (positions, edges)
}

/// An SDF box floor (top face at y = 0) so the SDF-collide branch is exercised.
fn sdf_floor() -> SdfField {
    let half = 50.0_f32;
    SdfField::from_edits(&[SdfEdit::box_shape(
        [0.0, -half, 0.0],
        [half, half, half],
        sdf_op::UNION,
        0.0,
    )])
}

/// Builds a world holding one cloth `SoftBody` of `~side²` particles plus the
/// soft config + SDF floor.
fn setup(side: usize) -> EcsMaster {
    let (positions, edges) = grid_cloth(side, side, 2.0);
    let n = positions.len();
    let inv_masses = vec![1.0_f32; n];
    let body = SoftBody::from_mesh(&positions, &inv_masses, &edges, None, 1.0e-7, 0.1)
        .expect("cloth grid is well-formed");

    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[SoftBody::component_id()]);
    world.spawn_one(arch, body).expect("spawn cloth");
    world.insert_resource(PhysicsConfig {
        dt: 1.0 / 60.0,
        substeps: 8,
        gravity: Vec3::new(0.0, -9.81, 0.0),
        soft_body: true,
        ..PhysicsConfig::default()
    });
    world.insert_resource(sdf_floor());
    world
}

fn bench_soft_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("physics_soft_step");
    // 8×8 = 64 particles, 23×23 = 529 ≈ 512 particles.
    for &(side, particles) in &[(8usize, 64usize), (23usize, 529usize)] {
        group.throughput(Throughput::Elements(particles as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(particles),
            &side,
            |b, &side| {
                let mut world = setup(side);
                // Build the system ONCE (its `Marker` type is unnameable, so it is
                // held by `let mut` in the closure scope rather than returned). Warm:
                // settle into a representative steady state + prime the query-state
                // caches (the kernel's per-step loop counts are state-independent).
                let mut sys = IntoSystem::into_system(physics_soft_step);
                for _ in 0..30 {
                    world.run_system_once(&mut sys);
                }
                b.iter(|| {
                    world.run_system_once(black_box(&mut sys));
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_soft_step);
criterion_main!(benches);
