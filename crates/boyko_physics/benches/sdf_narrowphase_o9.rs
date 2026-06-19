//! O9 A/B: the box-vs-SDF narrowphase (`box_sdf_manifold`) — scalar build vs the
//! `+avx2` batched-kernel build — at SDF edit counts {1, 2, 4, 8, 16}.
//!
//! `box_sdf_manifold` is private + cfg-gated (one arm per build), so the A/B is
//! TWO BUILDS of the SAME public entry, not a runtime flag:
//!
//! ```text
//! # scalar baseline (default build):
//! cargo bench -p boyko-physics --bench sdf_narrowphase_o9
//! # AVX2 (the batched kernel):
//! RUSTFLAGS="-C target-feature=+avx2" cargo bench -p boyko-physics --bench sdf_narrowphase_o9
//! ```
//!
//! The bench drives the `add_physics_sdf::<NoopSolver>` schedule over `N` box
//! bodies, each FULLY submerged in a multi-edit SDF field (so every corner
//! penetrates and each runs the gradient batch — the worst-case narrowphase). The
//! `NoopSolver` makes the solve a no-op, so the per-step cost is gather +
//! broadphase + `physics_narrowphase_sdf`. Gather + broadphase are byte-identical
//! in BOTH builds (no SIMD-gated code there), so the scalar-build → avx2-build
//! DELTA is purely the `box_sdf_manifold` kernel — that delta is the O9 speed-up.
//! (Per-box absolute time includes the shared overhead and is the conservative
//! upper bound; the kernel-only win is larger.)
//!
//! Anti-vacuity: every body penetrates (the narrowphase emits a manifold per body),
//! and the field has > 0 edits at every measured count.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    BodyType, Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_sdf;
use boyko_physics::sdf_query::SdfField;
use boyko_physics::solver::NoopSolver;

use boyko_sdf_math::{SdfEdit, sdf_op};

/// Deterministic splitmix64 — reproducible scene, no external dep.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn f32_in(&mut self, range: f32) -> f32 {
        let u = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        (u * 2.0 - 1.0) * range
    }
}

/// View a `#[repr(C)]` POD as bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; we view its `size_of::<T>()` bytes
    // read-only for the borrow. The slice borrows `value` and cannot outlive it.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

fn spawn_box(world: &mut EcsMaster, position: Vec3, rotation: Quat, half: Vec3) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.0,
        friction: 0.5,
        body_type: BodyType::Dynamic,
    };
    let collider = Collider {
        shape: ColliderShape::Box { half_extents: half },
        layer: 1,
        mask: 1,
    };
    let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
    world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("RigidBodyBundle archetype accepts the three columns");
}

/// A field of `n_edits` (>= 1) primitives whose UNION fully encloses the body cloud
/// (a huge first box + smaller spheres/boxes), so every body corner penetrates and
/// runs the gradient batch — the worst-case narrowphase work.
fn submerging_field(n_edits: usize) -> SdfField {
    let mut rng = Rng::new(0x0900_f1e1_d000_0009);
    let mut edits = Vec::with_capacity(n_edits);
    // Edit 0 seeds a large enclosing box (every body sits inside it).
    edits.push(SdfEdit::box_shape([0.0, 0.0, 0.0], [60.0, 60.0, 60.0], sdf_op::UNION, 0.0));
    for _ in 1..n_edits {
        let center = [rng.f32_in(40.0), rng.f32_in(40.0), rng.f32_in(40.0)];
        if rng.next_u64() & 1 == 0 {
            edits.push(SdfEdit::sphere(center, 1.0 + rng.f32_in(2.0).abs(), sdf_op::UNION, 0.2));
        } else {
            edits.push(SdfEdit::box_shape(
                center,
                [1.0 + rng.f32_in(2.0).abs(), 1.0 + rng.f32_in(2.0).abs(), 1.0 + rng.f32_in(2.0).abs()],
                sdf_op::UNION,
                0.0,
            ));
        }
    }
    SdfField::from_edits(&edits)
}

/// Builds a world of `n_bodies` random-posed boxes + the `n_edits` submerging field
/// + the `add_physics_sdf::<NoopSolver>` schedule, ready to `run`.
fn build_scene(n_bodies: usize, n_edits: usize) -> (EcsMaster, Schedule) {
    let mut world = EcsMaster::new();
    let mut rng = Rng::new(0x0900_b0d1_e500_0009);
    for _ in 0..n_bodies {
        let pos = Vec3::new(rng.f32_in(40.0), rng.f32_in(40.0), rng.f32_in(40.0));
        let rot = Quat::new(rng.f32_in(1.0), rng.f32_in(1.0), rng.f32_in(1.0), rng.f32_in(1.0))
            .normalize();
        let half = Vec3::new(
            0.3 + rng.f32_in(0.5).abs(),
            0.3 + rng.f32_in(0.5).abs(),
            0.3 + rng.f32_in(0.5).abs(),
        );
        spawn_box(&mut world, pos, rot, half);
    }
    let mut builder = ScheduleBuilder::new(serial_pool());
    let _keys = add_physics_sdf::<NoopSolver>(&mut builder, &mut world);
    *world.resource_mut::<SdfField>() = submerging_field(n_edits);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(1.0 / 60.0)));
    let schedule = builder.build(&mut world);
    (world, schedule)
}

fn bench_narrowphase(c: &mut Criterion) {
    const N_BODIES: usize = 4_000;
    let mut group = c.benchmark_group("o9_box_sdf_narrowphase");
    group.throughput(Throughput::Elements(N_BODIES as u64));

    for &n_edits in &[1usize, 2, 4, 8, 16] {
        group.bench_with_input(BenchmarkId::from_parameter(n_edits), &n_edits, |b, &n_edits| {
            let (mut world, mut schedule) = build_scene(N_BODIES, n_edits);
            // Warm one step (populate scratch, archetype caches) before timing.
            schedule.run(&mut world);
            b.iter(|| {
                schedule.run(black_box(&mut world));
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_narrowphase);
criterion_main!(benches);
