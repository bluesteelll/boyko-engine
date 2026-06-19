//! Physics O11 SP2 micro-bench (observation, NOT a pass/fail gate): the cost of the
//! two new SP2 hot paths.
//!
//! 1. `project_volume`: a tet-meshed soft cube stepped through the uncoupled
//!    `physics_soft_step` — the per-step cost now INCLUDES the volume sweep
//!    (`0..tet_count()` of `project_volume`) on top of the SP1 distance sweep. The
//!    A/B is the same cube WITHOUT tets (distance-only) vs WITH tets, so the delta
//!    is the volume-constraint cost.
//! 2. The coupled step: `physics_soft_step_coupled` (the per-particle
//!    `deepest_contact` broadphase-grid query + the D7 reaction) + the post-apply
//!    `physics_soft_rigid_apply`, against a small rigid snapshot. This is the cost of
//!    the coupling query per particle.
//!
//! Driven through the real `SystemParam` fetch via `run_system_once` (the kernel in
//! isolation, excluding the `Schedule::run` dispatch tax), mirroring `soft_step.rs`.
//!
//! ```text
//! cargo bench -p boyko-physics --bench soft_step_sp2
//! ```

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::into_system::IntoSystem;

use boyko_physics::components::{BodyType, ColliderShape, RigidBody};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, BroadphaseGrid, PhysicsConfig, SolverScratch};
use boyko_physics::sdf_query::SdfField;
use boyko_physics::soft::{
    SoftBody, SoftRigidReaction, physics_soft_rigid_apply, physics_soft_step,
    physics_soft_step_coupled,
};

/// A `w × h` grid of particles in the xz plane braced by structural + both face
/// diagonals (the `soft_step.rs` cloth topology).
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
                edges.push((idx(x, z), idx(x + 1, z)));
            }
            if z + 1 < h {
                edges.push((idx(x, z), idx(x, z + 1)));
            }
            if x + 1 < w && z + 1 < h {
                edges.push((idx(x, z), idx(x + 1, z + 1)));
                edges.push((idx(x + 1, z), idx(x, z + 1)));
            }
        }
    }
    (positions, edges)
}

/// `(positions, edges, tets)` for a volumetric lattice fixture.
type Lattice = (Vec<[f32; 3]>, Vec<(u32, u32)>, Vec<(u32, u32, u32, u32)>);

/// A `side × side × side` lattice of particles (spacing 1.0), every grid cell split
/// into 5 tets — a realistic SOLID volumetric body (≈ 5 tets / 24 distance edges per
/// cell). Returns `(positions, edges, tets)`.
fn tet_lattice(side: usize) -> Lattice {
    let n = side;
    let idx = |x: usize, y: usize, z: usize| ((z * n + y) * n + x) as u32;
    let mut positions = Vec::with_capacity(n * n * n);
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                positions.push([x as f32, y as f32 + 2.0, z as f32]);
            }
        }
    }
    let mut edges = Vec::new();
    let mut tets = Vec::new();
    // Each unit cell (8 corners) → 12 surface + 12 face-diagonal edges (dedup is not
    // needed for a bench fixture) + 5 tets.
    for z in 0..n - 1 {
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let c = [
                    idx(x, y, z),
                    idx(x + 1, y, z),
                    idx(x, y + 1, z),
                    idx(x + 1, y + 1, z),
                    idx(x, y, z + 1),
                    idx(x + 1, y, z + 1),
                    idx(x, y + 1, z + 1),
                    idx(x + 1, y + 1, z + 1),
                ];
                // Cube corners indexed by (x,y,z) bits: c[0..8] maps bit0=x, bit1=y,
                // bit2=z. Add the 5-tet split (the canonical decomposition).
                let map = |i: usize| c[i];
                for &(a, b, cc, d) in &[
                    (0usize, 1, 2, 4),
                    (3, 1, 2, 7),
                    (5, 1, 4, 7),
                    (6, 2, 4, 7),
                    (1, 2, 4, 7),
                ] {
                    tets.push((map(a), map(b), map(cc), map(d)));
                }
                // A few structural edges per cell (enough to brace; not deduped).
                edges.push((c[0], c[1]));
                edges.push((c[0], c[2]));
                edges.push((c[0], c[4]));
                edges.push((c[1], c[7]));
                edges.push((c[2], c[7]));
                edges.push((c[4], c[7]));
            }
        }
    }
    (positions, edges, tets)
}

fn sdf_floor() -> SdfField {
    SdfField::default()
}

/// A dynamic-sphere `BodyState` snapshot row.
fn sphere_state(position: Vec3, radius: f32, inv_mass: f32) -> BodyState {
    let inv_inertia = if inv_mass > 0.0 {
        let s = inv_mass * 5.0 / (2.0 * radius * radius);
        Mat3::from_diagonal(Vec3::new(s, s, s))
    } else {
        Mat3::ZERO
    };
    BodyState {
        inv_inertia,
        inv_inertia_local: inv_inertia,
        position,
        linear_velocity: Vec3::ZERO,
        angular_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        inv_mass,
        restitution: 0.0,
        friction: 0.0,
        body_type: BodyType::Dynamic,
        shape: ColliderShape::Sphere { radius },
    }
}

/// Builds a world with a volume body (tets) or a distance-only body, plus the soft
/// config + SDF floor.
fn setup_volume(side: usize, with_tets: bool) -> EcsMaster {
    let (positions, edges, tets) = tet_lattice(side);
    let n = positions.len();
    let inv_masses = vec![1.0_f32; n];
    let body = if with_tets {
        SoftBody::from_tet_mesh(
            &positions, &inv_masses, &edges, &tets, None, None, 1.0e-5, 0.0, 0.1,
        )
        .expect("tet lattice is well-formed")
    } else {
        SoftBody::from_mesh(&positions, &inv_masses, &edges, None, 1.0e-5, 0.1)
            .expect("distance lattice is well-formed")
    };
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[SoftBody::component_id()]);
    world.spawn_one(arch, body).expect("spawn volume body");
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

/// Builds a world for the coupled step: a cloth sheet over a small field of dynamic
/// rigid spheres (so the per-particle `deepest_contact` query has candidates).
fn setup_coupled(side: usize) -> EcsMaster {
    let (positions, edges) = grid_cloth(side, side, 0.5);
    let n = positions.len();
    let inv_masses = vec![1.0_f32; n];
    let body = SoftBody::from_mesh(&positions, &inv_masses, &edges, None, 1.0e-7, 0.1)
        .expect("cloth grid is well-formed");
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[SoftBody::component_id()]);
    world.spawn_one(arch, body).expect("spawn cloth");

    // A field of dynamic rigid spheres beneath the cloth (one per ~4 particles) so
    // the coupling query resolves real contacts.
    let mut bodies = Vec::new();
    let rb_arch = world.create_archetype(&[RigidBody::component_id()]);
    let span = side as f32;
    let count = (side / 2).max(1);
    for i in 0..count {
        let t = i as f32 / count as f32;
        let pos = Vec3::new((t - 0.5) * span, 0.0, (t - 0.5) * span);
        bodies.push(sphere_state(pos, 0.4, 4.0));
        world
            .spawn_one(
                rb_arch,
                RigidBody {
                    position: pos,
                    linear_velocity: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    angular_velocity: Vec3::ZERO,
                },
            )
            .expect("spawn rigid body");
    }

    world.insert_resource(PhysicsConfig {
        dt: 1.0 / 60.0,
        substeps: 8,
        gravity: Vec3::new(0.0, -9.81, 0.0),
        soft_body: true,
        soft_rigid_coupling: true,
        ..PhysicsConfig::default()
    });
    world.insert_resource(sdf_floor());

    // Build the broadphase grid + the scratch snapshot + the reaction sink (what the
    // pipeline produces on the Grid arm).
    let mut grid = BroadphaseGrid::with_capacity(bodies.len().max(1));
    let mut out = Vec::new();
    grid.build(&bodies, &mut out);
    let mut scratch = SolverScratch::with_capacity(bodies.len().max(1));
    scratch.bodies = bodies.clone();
    world.insert_resource(scratch);
    world.insert_resource(grid);
    world.insert_resource(SoftRigidReaction::with_capacity(bodies.len().max(1)));
    world
}

fn bench_volume_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("soft_step_sp2/project_volume");
    // 4³ = 64, 6³ = 216 particles (a small + a medium solid lattice).
    for &side in &[4usize, 6usize] {
        let particles = side * side * side;
        for &with_tets in &[false, true] {
            let label = if with_tets { "with_tets" } else { "distance_only" };
            group.bench_with_input(
                BenchmarkId::new(label, particles),
                &(side, with_tets),
                |b, &(side, with_tets)| {
                    let mut world = setup_volume(side, with_tets);
                    let mut sys = IntoSystem::into_system(physics_soft_step);
                    for _ in 0..20 {
                        world.run_system_once(&mut sys);
                    }
                    b.iter(|| {
                        world.run_system_once(black_box(&mut sys));
                    });
                },
            );
        }
    }
    group.finish();
}

fn bench_coupled_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("soft_step_sp2/coupled");
    // 8×8 = 64, 16×16 = 256 cloth particles over a field of rigid spheres.
    for &side in &[8usize, 16usize] {
        let particles = side * side;
        group.bench_with_input(
            BenchmarkId::from_parameter(particles),
            &side,
            |b, &side| {
                let mut world = setup_coupled(side);
                let mut step = IntoSystem::into_system(physics_soft_step_coupled);
                let mut apply = IntoSystem::into_system(physics_soft_rigid_apply);
                for _ in 0..20 {
                    world.run_system_once(&mut step);
                    world.run_system_once(&mut apply);
                }
                b.iter(|| {
                    world.run_system_once(black_box(&mut step));
                    world.run_system_once(black_box(&mut apply));
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_volume_step, bench_coupled_step);
criterion_main!(benches);
