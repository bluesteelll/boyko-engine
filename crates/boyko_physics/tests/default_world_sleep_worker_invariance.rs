//! L10 C1a (`docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md`, "Gates", C1a):
//! the default physics world with sleeping on is worker-count invariant, bit for bit.
//!
//! The default world (`add_physics_systems::<DefaultRigidSolver>`, default `PhysicsConfig` with
//! `sleeping` set on explicitly, so the gate does not depend on the shipped default, which is
//! off on this tree) runs the rest pile — the jolt parity runner's `rest` scene, an exactly
//! touching box pyramid on a static floor — for [`STEPS`] steps on pools of 1, 2, 4, 8 and 16
//! workers. The FNV-1a 64 of every `RigidBody` bit is taken after every step, and the per-step
//! sequence must be identical across the five runs: the pool size may change where a colour is
//! solved or a pair is collided, never which island freezes, when, or the numbers it freezes
//! with.
//!
//! The pyramid is 15 layers (1240 boxes, the runner's scene) in a release build and
//! [`DEBUG_HEIGHT`] layers in a debug build, where the full pile would not finish in a
//! reasonable sweep (the frame census scales its pyramid the same way).
//!
//! # Anti-vacuity
//!
//! Every run must observe a frozen step: a step after which some dynamic row is frozen
//! (`IslandSleep::is_row_awake` false). Invariance over runs in which nothing ever froze would
//! say nothing about the frozen path. RED-first (C1a's log): `sleep_frames = u16::MAX` never
//! latches a row, and every run fails "a frozen step was observed".
//!
//! Spins real thread pools (intractable under Miri), so `cfg(not(miri))`.

#![cfg(not(miri))]

use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{IslandSleep, PhysicsConfig};
use boyko_physics::solver::DefaultRigidSolver;

/// Steps each run takes.
const STEPS: usize = 400;
/// The worker counts compared.
const WORKERS: [usize; 5] = [1, 2, 4, 8, 16];
/// The fixed step.
const DT: f32 = 1.0 / 60.0;
/// The pitch between neighbouring boxes in a layer (Jolt's `cBoxSize`).
const BOX_SIZE: f32 = 2.0;
/// Every box's half-extent.
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
/// The floor's half-extents.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
/// The `rest` scene's friction.
const FRICTION: f32 = 0.5;
/// A unit-density cube of half-extent 1: m = 8.
const BOX_INV_MASS: f32 = 0.125;
/// Its inverse inertia, uniform on the diagonal.
const BOX_INV_INERTIA: f32 = 0.1875;
/// The pyramid's layers in a debug build.
const DEBUG_HEIGHT: i32 = 8;

/// The pyramid's layers: the runner's 15 in release, [`DEBUG_HEIGHT`] in debug.
const fn pyramid_height() -> i32 {
    if cfg!(debug_assertions) { DEBUG_HEIGHT } else { 15 }
}

/// Views a `#[repr(C)]` POD value as its bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, initialised `#[repr(C)]` `T` borrowed for the returned
    // slice's lifetime; the slice covers exactly its `size_of::<T>()` bytes, read-only, which
    // is the layout the component pool stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// One box at rest; a dynamic one is enabled as `Simulated`, the floor is not.
fn spawn_box(world: &mut EcsMaster, position: Vec3, dynamic: bool) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let (mass, half_extents) = if dynamic {
        (
            RigidBodyMass {
                inv_inertia: Mat3::from_diagonal(Vec3::new(
                    BOX_INV_INERTIA,
                    BOX_INV_INERTIA,
                    BOX_INV_INERTIA,
                )),
                inv_mass: BOX_INV_MASS,
                restitution: 0.0,
                friction: FRICTION,
            },
            Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
        )
    } else {
        (
            RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass: 0.0, restitution: 0.0, friction: FRICTION },
            FLOOR_HALF_EXTENTS,
        )
    };
    let collider = Collider { shape: ColliderShape::Box { half_extents }, layer: 1, mask: 1 };
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
        .expect("construction: the RigidBodyBundle archetype accepts the three columns");
    if dynamic {
        world.enable::<Simulated>(e);
    }
}

/// The floor, then the exactly touching pyramid in Jolt's placement order. Returns the number
/// of rows (the floor included).
fn spawn_rest_pile(world: &mut EcsMaster) -> usize {
    spawn_box(world, Vec3::new(0.0, -1.0, 0.0), false);
    let height = pyramid_height();
    let mut rows = 1;
    for i in 0..height {
        let lo = i / 2;
        let hi = height - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                let position = Vec3::new(
                    -(height as f32) + BOX_SIZE * j as f32 + odd,
                    1.0 + BOX_SIZE * i as f32,
                    -(height as f32) + BOX_SIZE * k as f32 + odd,
                );
                spawn_box(world, position, true);
                rows += 1;
            }
        }
    }
    rows
}

/// FNV-1a 64 over every `RigidBody`'s thirteen `f32`s, as bits, in query order.
fn pose_hash(world: &mut EcsMaster) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let q = world.query::<&RigidBody, ()>();
    for b in q.iter() {
        let words = [
            b.position.x,
            b.position.y,
            b.position.z,
            b.linear_velocity.x,
            b.linear_velocity.y,
            b.linear_velocity.z,
            b.rotation.x,
            b.rotation.y,
            b.rotation.z,
            b.rotation.w,
            b.angular_velocity.x,
            b.angular_velocity.y,
            b.angular_velocity.z,
        ];
        for w in words {
            for byte in w.to_bits().to_le_bytes() {
                h ^= u64::from(byte);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    h
}

/// One run: the per-step pose hashes, and the steps after which some row was frozen.
fn run(workers: usize) -> (Vec<u64>, usize) {
    let mut world = EcsMaster::new();
    let rows = spawn_rest_pile(&mut world);
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(workers).build());
    add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    let mut schedule = builder.build(&mut world);
    world.resource_mut::<PhysicsConfig>().sleeping = true;
    let mut hashes = Vec::with_capacity(STEPS);
    let mut frozen_steps = 0;
    for _ in 0..STEPS {
        schedule.run(&mut world);
        hashes.push(pose_hash(&mut world));
        let sleep = world.resource::<IslandSleep>();
        frozen_steps += usize::from((0..rows).any(|r| !sleep.is_row_awake(r)));
    }
    (hashes, frozen_steps)
}

#[test]
fn the_default_world_with_sleeping_on_is_worker_count_invariant() {
    let (reference, frozen_1) = run(WORKERS[0]);
    println!(
        "W=1: pyramid height {}, {STEPS} steps, final pose hash {:#018x}, a row frozen on {frozen_1} steps",
        pyramid_height(),
        reference[STEPS - 1]
    );
    assert!(
        frozen_1 > 0,
        "anti-vacuity: at W=1 a frozen step was observed on none of {STEPS} steps"
    );
    for &w in &WORKERS[1..] {
        let (hashes, frozen) = run(w);
        println!("W={w}: final pose hash {:#018x}, a row frozen on {frozen} steps", hashes[STEPS - 1]);
        assert!(frozen > 0, "anti-vacuity: at W={w} a frozen step was observed on none of {STEPS} steps");
        if let Some(step) = reference.iter().zip(&hashes).position(|(a, b)| a != b) {
            panic!(
                "W={w} diverges from W=1 at step {}: pose hash {:#018x} against {:#018x}",
                step + 1,
                hashes[step],
                reference[step]
            );
        }
        assert_eq!(frozen, frozen_1, "W={w}: the frozen steps differ from W=1's");
    }
}
