//! Jolt-parity pyramid: the SAME SCENE, run through boyko's FULL physics step, so
//! the two engines' parallel scaling can be compared on this machine instead of
//! against a published number measured on someone else's.
//!
//! # What is identical, exactly
//!
//! Every geometric and integration parameter is transcribed from Jolt's own
//! `PerformanceTest/PyramidScene.h` and `PerformanceTest.cpp` (v5.3.0):
//!
//! * floor — a STATIC box with half-extents `(50, 1, 50)` centred at `(0, -1, 0)`;
//! * boxes — half-extents `(1, 1, 1)`, no convex radius, `cBoxSize = 2.0`,
//!   `cBoxSeparation = 0.5`, `cPyramidHeight = 15`;
//! * the placement loop, index for index, including the odd-layer half-box offset;
//! * **1240 dynamic bodies** + 1 static floor;
//! * gravity `(0, -9.81, 0)` — already both engines' default, so this is a
//!   coincidence worth stating rather than an alignment;
//! * `dt = 1/60`, one step per `Update` (Jolt's `cDeltaTime`, `collision_steps = 1`);
//! * sleeping DISABLED, which is what Jolt's scene forces (`mAllowSleeping = false`)
//!   to keep the large island awake — and is boyko's default anyway.
//!
//! # ⚠ What CANNOT be made identical, and why the metric is a RATIO
//!
//! The two solvers are different families. Jolt runs 10 velocity + 2 position
//! iterations inside one collision step; boyko runs 4 TGS-soft substeps each with
//! 1 + 2 relaxation sweeps. There is no assignment of one engine's iteration
//! counts to the other's that means the same thing, so an absolute
//! steps-per-second comparison would be comparing two different amounts of work
//! and calling it a speed difference.
//!
//! Therefore the comparable quantity is **parallel scaling** — `T(1) / T(N)` — and
//! efficiency per core derived from it. That ratio divides out each engine's own
//! per-step constant, so it survives the iteration-count mismatch that an absolute
//! number does not. Both engines are built for the same ISA baseline (AVX2 / FMA /
//! LZCNT / TZCNT / F16C, no AVX-512; Jolt prints its set at startup and boyko's
//! worktree pins `-C target-cpu=x86-64-v3`).
//!
//! # ⚠ And what this measures about boyko specifically
//!
//! The colored solve and the parallel dispatch are OPT-IN and ship OFF
//! (`PhysicsConfig::{colored, parallel_solve, parallel_broadphase}` all default
//! `false`). This bench turns them on. That is the configuration a game would
//! choose for a pile like this, but it is not what a default world runs, and the
//! numbers must never be quoted as "boyko's default physics".

#![allow(clippy::missing_const_for_thread_local)]

// Alloc A/B: opt-in low-variance allocator for A/B signal extraction, the same
// arm `bench_bevy_vs_boyko/benches/comparison_v2.rs` carries (Phase X.E).
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc, which
// is far more deterministic and exposes structural signals the system heap
// masks (the documented ±20-30% variance source). Here the question is whether
// the heap traffic of a parallel step costs time: a gap that grows with W would
// mean cross-thread frees limit scaling, a flat gap only the malloc/free
// instructions. Measured 2026-09-10: no gap at any W. On this tree (thread-pool
// Stage 3b present) a step makes ~332 allocations at W=8, same-thread, not the
// ~2.7k stealer-freed cells of the pre-3b pool the diagnostic was written for;
// docs/OPEN-QUESTIONS.md records both. See docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::PhysicsConfig;

// ── Jolt PyramidScene.h constants, transcribed ───────────────────────────────

/// `cBoxSize` — the pitch between neighbouring boxes in a layer.
const BOX_SIZE: f32 = 2.0;
/// `cBoxSeparation` — the vertical gap added on top of `cBoxSize` per layer.
const BOX_SEPARATION: f32 = 0.5;
/// `cHalfBoxSize` — the box half-extent on every axis.
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
/// `cPyramidHeight` — the number of layers.
const PYRAMID_HEIGHT: i32 = 15;

/// The floor's half-extents, from Jolt's `BoxShape(Vec3(50, 1, 50))`.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);

/// Jolt's `cDeltaTime`.
const DT: f32 = 1.0 / 60.0;

/// Spawns one body through the raw `create_entity` path (the `Bundle` derive is
/// consumable only through `Commands`), mirroring the seam tests' fixture.
///
/// `simulated` gates whether the integrate / solve stages advance the row: the
/// floor is spawned WITHOUT it, which is how a static body is expressed here.
fn spawn_body(
    world: &mut EcsMaster,
    body: RigidBody,
    mass: RigidBodyMass,
    collider: Collider,
    simulated: bool,
) {
    // SAFETY-adjacent note: `as_bytes` reads a `#[repr(C)]` POD component as its
    // own bytes for the raw column write; the slice cannot outlive the value.
    fn as_bytes<T>(value: &T) -> &[u8] {
        unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
    }
    let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
    let e = world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("invariant: RigidBodyBundle archetype accepts the three columns");
    if simulated {
        world.enable::<Simulated>(e);
    }
}

/// Builds Jolt's pyramid, index for index. Returns the dynamic body count.
fn spawn_jolt_pyramid(world: &mut EcsMaster) -> usize {
    // Floor: static (inv_mass 0, zero inverse inertia), not `Simulated`.
    spawn_body(
        world,
        RigidBody {
            position: Vec3::new(0.0, -1.0, 0.0),
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        },
        RigidBodyMass {
            inv_inertia: Mat3::ZERO,
            inv_mass: 0.0,
            restitution: 0.0,
            friction: 0.5,
        },
        Collider {
            shape: ColliderShape::Box { half_extents: FLOOR_HALF_EXTENTS },
            layer: 1,
            mask: 1,
        },
        false,
    );

    let mut n = 0usize;
    for i in 0..PYRAMID_HEIGHT {
        let lo = i / 2;
        let hi = PYRAMID_HEIGHT - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                let position = Vec3::new(
                    -(PYRAMID_HEIGHT as f32) + BOX_SIZE * j as f32 + odd,
                    1.0 + (BOX_SIZE + BOX_SEPARATION) * i as f32,
                    -(PYRAMID_HEIGHT as f32) + BOX_SIZE * k as f32 + odd,
                );
                spawn_body(
                    world,
                    RigidBody {
                        position,
                        linear_velocity: Vec3::ZERO,
                        rotation: Quat::IDENTITY,
                        angular_velocity: Vec3::ZERO,
                    },
                    RigidBodyMass {
                        // A unit-density cube of half-extent 1: m = 8, and the
                        // inertia of a box is m/12 * (h^2 + w^2) per axis = 8/12 * 8.
                        // Inverse of that, uniform on the diagonal.
                        inv_inertia: Mat3::from_diagonal(Vec3::new(0.1875, 0.1875, 0.1875)),
                        inv_mass: 0.125,
                        restitution: 0.0,
                        friction: 0.5,
                    },
                    Collider {
                        shape: ColliderShape::Box {
                            half_extents: Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
                        },
                        layer: 1,
                        mask: 1,
                    },
                    true,
                );
                n += 1;
            }
        }
    }
    n
}

fn pool(workers: usize) -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(workers).build()
}

/// Builds a world with the pyramid and the FULL colored physics pipeline wired.
fn build(workers: usize) -> (EcsMaster, Schedule, usize) {
    let mut world = EcsMaster::new();
    let n = spawn_jolt_pyramid(&mut world);
    let mut builder = ScheduleBuilder::new(pool(workers));
    let _keys = add_physics_colored_solve(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(DT)));
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
        cfg.dt = DT;
        // Opt in to the parallel paths — see the header: these ship OFF.
        cfg.parallel_solve = workers > 1;
        cfg.parallel_broadphase = workers > 1;
        cfg.sleeping = false;
    }
    let schedule = builder.build(&mut world);
    (world, schedule, n)
}

fn bench_jolt_parity_pyramid(c: &mut Criterion) {
    let mut group = c.benchmark_group("jolt_parity_pyramid");
    group.sample_size(20);

    // Anti-vacuity: the transcribed loop must reproduce Jolt's body count exactly.
    // If this drifts, the two engines are no longer running the same scene and
    // every ratio below is comparing different work.
    {
        let (_w, _s, n) = build(1);
        assert_eq!(
            n, 1240,
            "the transcribed pyramid must hold exactly Jolt's 1240 dynamic bodies"
        );
    }

    for workers in [1usize, 2, 4, 8, 16] {
        group.bench_with_input(
            BenchmarkId::new("full_step", workers),
            &workers,
            |b, &w| {
                let (mut world, mut schedule, _n) = build(w);
                // Warm: let the pile settle into its steady contact set so the
                // timed region measures a representative step, not the first
                // frame's graph build.
                for _ in 0..20 {
                    schedule.run(&mut world);
                }
                b.iter(|| {
                    schedule.run(black_box(&mut world));
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_jolt_parity_pyramid);
criterion_main!(benches);
