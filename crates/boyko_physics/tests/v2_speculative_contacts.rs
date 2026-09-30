//! **V2's behaviour gates through the real schedule** (`levers/V2-speculative/01-DESIGN.md`):
//! each builds a small world with an explicit `speculative_distance`, so it pins what V2 does
//! whatever the default is.
//!
//! * **B1 (C3)** — a sphere pair whose surfaces are 10 mm apart is a candidate of every broadphase
//!   at `d = 20 mm` (each body's bounding sphere is inflated by `d / 2` at gather), and the
//!   narrowphase makes its one speculative point; at `d = 0` it is no candidate. Red before the
//!   margin: no candidate, so no manifold.

#![cfg(not(miri))]

use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{
    BroadphaseKind, BroadphaseSelectMode, ContactPairs, Manifolds, PhysicsConfig,
};
use boyko_physics::solver::DefaultRigidSolver;

/// Fixed timestep.
const DT: f32 = 1.0 / 60.0;
/// The owner's distance (V2b).
const D: f32 = 0.02;

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its `size_of::<T>()` bytes
    // read-only for the duration of the borrow, which is the exact layout the component pool
    // stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// One body: dynamic (`inv_mass` 1, `Simulated`) or static.
fn spawn(world: &mut EcsMaster, shape: ColliderShape, position: Vec3, velocity: Vec3, restitution: f32, dynamic: bool) {
    let body = RigidBody { position, linear_velocity: velocity, rotation: Quat::IDENTITY, angular_velocity: Vec3::ZERO };
    let mass = RigidBodyMass {
        inv_inertia: if dynamic { Mat3::from_diagonal(Vec3::new(1.0, 1.0, 1.0)) } else { Mat3::ZERO },
        inv_mass: if dynamic { 1.0 } else { 0.0 },
        restitution,
        friction: 0.5,
    };
    let collider = Collider { shape, layer: 1, mask: 1 };
    let base = [RigidBody::component_id(), RigidBodyMass::component_id(), Collider::component_id()];
    let archetype = world.create_archetype(&base);
    let e = world
        .create_entity(archetype, &[(base[0], as_bytes(&body)), (base[1], as_bytes(&mass)), (base[2], as_bytes(&collider))])
        .expect("construction: the archetype accepts the three columns");
    if dynamic {
        world.enable::<Simulated>(e);
    }
}

/// The colored pipeline at one worker, with `configure` applied to its configuration.
fn world(configure: impl FnOnce(&mut PhysicsConfig)) -> (EcsMaster, ScheduleBuilder) {
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
    add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.dt = DT;
        configure(cfg);
    }
    (world, builder)
}

/// Builds the schedule once the bodies are spawned.
fn build(mut world: EcsMaster, builder: ScheduleBuilder) -> (EcsMaster, Schedule) {
    let schedule = builder.build(&mut world);
    (world, schedule)
}

/// B1 (C3): two spheres of radius 0.5 whose surfaces are 10 mm apart, no gravity, on AllPairs,
/// Grid and Tree: at `d = 20 mm` every broadphase emits the pair and the narrowphase keeps one
/// speculative point with `s = +10 mm`; at `d = 0` there is no candidate.
#[test]
fn b1_every_broadphase_pairs_two_spheres_within_d() {
    for kind in [BroadphaseKind::AllPairs, BroadphaseKind::Grid, BroadphaseKind::Tree] {
        for d in [D, 0.0] {
            let (mut w, builder) = world(|cfg| {
                cfg.gravity = Vec3::ZERO;
                cfg.broadphase_select = BroadphaseSelectMode::Manual;
                cfg.broadphase = kind;
                cfg.speculative_distance = d;
            });
            let sphere = ColliderShape::Sphere { radius: 0.5 };
            spawn(&mut w, sphere, Vec3::new(0.0, 5.0, 0.0), Vec3::ZERO, 0.0, true);
            spawn(&mut w, sphere, Vec3::new(1.01, 5.0, 0.0), Vec3::ZERO, 0.0, true);
            let (mut w, mut schedule) = build(w, builder);
            schedule.run(&mut w);
            let pairs = w.resource::<ContactPairs>().pairs().len();
            let manifolds = w.resource::<Manifolds>().solver_manifolds().to_vec();
            if d > 0.0 {
                assert_eq!(pairs, 1, "B1 {kind:?}: the pair 10 mm apart is a candidate at d = {d}");
                assert_eq!(manifolds.len(), 1, "B1 {kind:?}: one manifold at d = {d}");
                let m = &manifolds[0];
                assert!(
                    m.count == 1 && (m.points[0].separation - 0.01).abs() < 1e-4 && m.points[0].separation > 0.0,
                    "B1 {kind:?}: one speculative point, s = +10 mm: {m:?}"
                );
            } else {
                assert_eq!(pairs, 0, "B1 {kind:?}: no candidate at d = 0");
                assert!(manifolds.is_empty(), "B1 {kind:?}: no manifold at d = 0");
            }
        }
    }
}
