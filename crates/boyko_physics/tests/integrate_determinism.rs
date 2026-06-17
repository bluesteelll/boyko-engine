//! Property test: `physics_integrate` is deterministic — two runs over the same
//! random input are bit-identical (plan Validation; review I1).
//!
//! The integrator runs on a MULTI-THREADED pool over MANY bodies, so the test
//! actually exercises the parallel `par_iter_mut` write-to-disjoint-rows path —
//! the real determinism risk IM-2 warns about. A single-threaded, single-body
//! run would be deterministic trivially (pure function, same input → same bits)
//! and would prove nothing. Here, if a future change ever made the parallel
//! integrate observe cross-row / shared mutable state, the two runs would diverge
//! and this test would fail. Cross-run determinism still rides the IM-2
//! deterministic-spawn precondition, which the fixed serialized spawn loop below
//! satisfies (identical archetype row order across runs).

use std::sync::Arc;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    BodyType, Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass,
};
use boyko_physics::math::Vec2;
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::solver::NoopSolver;

use proptest::prelude::*;

/// Raw byte view of a `#[repr(C)]` value for the direct `create_entity` path.
///
/// # Safety
/// `T` is `#[repr(C)]` and the slice borrows `value` (cannot outlive it).
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: viewing a live `#[repr(C)]` `T` as its `size_of::<T>()` bytes for
    // the borrow's duration; the slice is tied to `value`'s lifetime.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A multi-threaded pool — the point of I1 (a single thread can't expose a
/// parallel-scheduling-order nondeterminism).
fn mt_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(4).build()
}

/// Runs `frames` no-op physics steps over `n_bodies` bodies on a multi-threaded
/// pool, returning every body's final `(position, linear_velocity)` in
/// archetype-row (iteration) order.
///
/// Each body `i` gets a distinct initial state derived deterministically from
/// the base + `i`, so the parallel integrate writes many distinct rows (not N
/// identical ones). Spawning is a fixed serialized loop → identical archetype
/// rows across runs (the IM-2 precondition), so the returned vectors are
/// position-comparable between two runs.
fn run_integration(
    n_bodies: usize,
    base_pos: Vec2,
    base_vel: Vec2,
    gravity: Vec2,
    dt: f32,
    frames: u32,
) -> Vec<(Vec2, Vec2)> {
    let mut world = EcsMaster::new();
    let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
    for i in 0..n_bodies {
        let fi = i as f32;
        let body = RigidBody {
            // Distinct per-row initial state so the parallel write touches many
            // different rows with different arithmetic.
            position: Vec2::new(base_pos.x + fi, base_pos.y - fi * 0.5),
            linear_velocity: Vec2::new(base_vel.x + fi * 0.25, base_vel.y - fi * 0.125),
            rotation: fi * 0.01,
            angular_velocity: fi * 0.001,
        };
        let mass = RigidBodyMass {
            inv_mass: 1.0,
            inv_inertia: 1.0,
            restitution: 0.5,
            friction: 0.3,
            body_type: BodyType::Dynamic,
        };
        let collider = Collider {
            shape: ColliderShape::Circle { radius: 0.5 },
            layer: 1,
            mask: 1,
        };
        world
            .create_entity(
                archetype,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                ],
            )
            .expect("spawn ok");
    }

    let mut builder = ScheduleBuilder::new(mt_pool());
    let _ = add_physics_systems::<NoopSolver>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(dt)));
    let mut schedule = builder.build(&mut world);
    world.resource_mut::<PhysicsConfig>().gravity = gravity;

    for _ in 0..frames {
        schedule.run(&mut world);
    }

    let q = world.query::<&RigidBody, ()>();
    q.iter().map(|b| (b.position, b.linear_velocity)).collect()
}

proptest! {
    /// Two identical runs of the multi-threaded integrator over many bodies
    /// produce bit-identical state for EVERY body (parallel write-to-disjoint-
    /// rows is order-independent — a real, falsifiable determinism check).
    #[test]
    fn integrate_is_deterministic(
        px in -1000.0f32..1000.0,
        py in -1000.0f32..1000.0,
        vx in -500.0f32..500.0,
        vy in -500.0f32..500.0,
        gx in -50.0f32..50.0,
        gy in -50.0f32..50.0,
        n_bodies in 64usize..256,
        frames in 1u32..16,
    ) {
        let dt = 1.0 / 64.0;
        let a = run_integration(n_bodies, Vec2::new(px, py), Vec2::new(vx, vy), Vec2::new(gx, gy), dt, frames);
        let b = run_integration(n_bodies, Vec2::new(px, py), Vec2::new(vx, vy), Vec2::new(gx, gy), dt, frames);
        prop_assert_eq!(a.len(), n_bodies);
        prop_assert_eq!(a.len(), b.len());
        for (ra, rb) in a.iter().zip(b.iter()) {
            prop_assert_eq!(ra.0.x.to_bits(), rb.0.x.to_bits());
            prop_assert_eq!(ra.0.y.to_bits(), rb.0.y.to_bits());
            prop_assert_eq!(ra.1.x.to_bits(), rb.1.x.to_bits());
            prop_assert_eq!(ra.1.y.to_bits(), rb.1.y.to_bits());
        }
    }
}
