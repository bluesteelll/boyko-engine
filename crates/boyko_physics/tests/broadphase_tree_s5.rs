//! S5 — the parallel tree query (`docs/physics/perf-campaign/levers/scaling/01-DESIGN.md` §6.5):
//! the world-level gates.
//!
//! * [`s5_switch_mirrors_into_the_tree`]: `PhysicsConfig::parallel_tree_query` reaches the tree
//!   broadphase on every Tree step, both ways, through the real schedule.
//!
//! Spins real thread pools (intractable under Miri), so `cfg(not(miri))`.

#![cfg(not(miri))]

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::broadphase_tree::BroadphaseTree;
use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{BroadphaseKind, PhysicsConfig};
use boyko_physics::solver::DefaultRigidSolver;

/// Fixed timestep.
const DT: f32 = 1.0 / 60.0;

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its `size_of::<T>()` bytes
    // read-only for the duration of the borrow, which is the exact layout the component pool
    // stores.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Spawns one box: dynamic with unit mass when `inv_mass` is 1, static when 0.
fn spawn_box(world: &mut EcsMaster, position: Vec3, half_extents: Vec3, inv_mass: f32) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        // A unit cube of mass 1 has I = 1/6 on the diagonal.
        inv_inertia: if inv_mass == 0.0 {
            Mat3::ZERO
        } else {
            Mat3::from_diagonal(Vec3::new(6.0, 6.0, 6.0))
        },
        inv_mass,
        restitution: 0.0,
        friction: 0.5,
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
        .expect("invariant: RigidBodyBundle archetype accepts the three columns");
    world.enable::<Simulated>(e);
}

/// A static floor and `grid × grid` two-box stacks (G3's scene at `grid = 13`), each box
/// overlapping what it rests on by 1 cm so every contact exists from the first frame.
fn spawn_stacks(world: &mut EcsMaster, grid: usize) {
    spawn_box(world, Vec3::new(0.0, -0.5, 0.0), Vec3::new(40.0, 0.5, 40.0), 0.0);
    let half = Vec3::new(0.5, 0.5, 0.5);
    let origin = -0.75 * (grid as f32 - 1.0);
    for i in 0..grid {
        for k in 0..grid {
            let x = origin + 1.5 * i as f32;
            let z = origin + 1.5 * k as f32;
            spawn_box(world, Vec3::new(x, 0.49, z), half, 1.0);
            spawn_box(world, Vec3::new(x, 1.48, z), half, 1.0);
        }
    }
}

/// The default world over `spawn_stacks(grid)` on a `workers`-wide pool.
fn default_world(workers: usize, grid: usize) -> (EcsMaster, Schedule) {
    let mut world = EcsMaster::new();
    spawn_stacks(&mut world, grid);
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(DT)));
    let schedule = builder.build(&mut world);
    (world, schedule)
}

/// The switch reaches the tree on every Tree step, both ways: a world whose configuration turns
/// it on reads it on in the tree after one step, and off again after the next.
#[test]
fn s5_switch_mirrors_into_the_tree() {
    let (mut world, mut schedule) = default_world(2, 2);
    assert_eq!(
        world.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::Tree,
        "construction: the default world runs the tree broadphase"
    );
    assert!(!world.resource::<BroadphaseTree>().parallel_query(), "construction: the tree's own default is off");
    for on in [true, false, true] {
        world.resource_mut::<PhysicsConfig>().parallel_tree_query = on;
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<BroadphaseTree>().parallel_query(),
            on,
            "S5: the tree did not read `parallel_tree_query = {on}` on the step after it was set"
        );
    }
}
