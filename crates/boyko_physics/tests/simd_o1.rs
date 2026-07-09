//! O1 integration gate: the full physics pipeline with `PhysicsConfig.simd = true`
//! (the AVX2 in-solver `refresh_inertia` + gravity/position/quaternion integrate
//! kernels) is BIT-IDENTICAL to the same scene with `simd = false` (the scalar
//! oracle, the campaign 0%-gate).
//!
//! This drives the real `Schedule` (gather → broadphase → narrowphase → solve →
//! apply) through the `SoftStepSolver`, twice over a multi-body scene with active
//! rotation and resting contacts (so both the inertia refresh and the quaternion
//! integrate are exercised non-vacuously), and asserts every body's final
//! `RigidBody` matches bit-for-bit. The unit-level differential proptest in
//! `solver::simd` proves the kernels are bit-identical in isolation incl. tails;
//! this proves the wiring preserves that through the full step.
//!
//! On a non-AVX2 build the dispatcher runs scalar regardless of the flag, so the
//! test trivially holds (the flag is then a no-op); on an AVX2 build it proves the
//! SIMD path is a pure speed path with zero value change.

use std::sync::Arc;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::solver::SoftStepSolver;

/// Views a `#[repr(C)]` POD value's bytes for the raw `create_entity` spawn path.
///
/// # Safety
///
/// `T` is a live `#[repr(C)]` physics component (stored by raw byte copy); the
/// slice borrows `value` and cannot outlive it.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; `size_of::<T>()` bytes are viewed read-only
    // for the borrow's duration. `T` is `#[repr(C)]`, matching the pool layout.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A single-threaded pool (deterministic, IM-2 precondition).
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// Spawns one rigid body via the raw `create_entity` path.
fn spawn_body(world: &mut EcsMaster, body: RigidBody, mass: RigidBodyMass, collider: Collider) {
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

/// A sphere body with the given spin (initial angular velocity) so the quaternion
/// integrate + inertia refresh do real work every substep.
fn spinning_sphere(
    position: Vec3,
    spin: Vec3,
    radius: f32,
    inv_mass: f32,
) -> (RigidBody, RigidBodyMass, Collider) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: spin,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass,
        restitution: 0.0,
        friction: 0.5,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius },
        layer: 1,
        mask: 1,
    };
    (body, mass, collider)
}

/// Builds the physics schedule for `SoftStepSolver` and stamps the fixed timestep.
fn build_schedule(world: &mut EcsMaster, dt: f32) -> Schedule {
    let mut builder = ScheduleBuilder::new(serial_pool());
    let _keys = add_physics_systems::<SoftStepSolver>(&mut builder, world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(dt)));
    builder.build(world)
}

/// Reads back every body's `RigidBody` in query (= spawn) order.
fn all_bodies(world: &mut EcsMaster) -> Vec<RigidBody> {
    let q = world.query::<&RigidBody, ()>();
    q.iter().copied().collect()
}

/// Runs a scene of spinning, stacking spheres for `frames` steps with the SIMD
/// flag set to `simd`, returning every body's final `RigidBody`.
fn run_scene(simd: bool, frames: usize) -> Vec<RigidBody> {
    let mut world = EcsMaster::new();

    // A column of spinning dynamic spheres above a large static floor — the
    // spheres rotate (quaternion integrate + per-substep inertia refresh active)
    // AND settle into resting contact (the solve runs). 13 dynamic bodies forces
    // a partial AVX2 lane (13 = 8 + 5), so the scalar tail of the kernels is
    // exercised inside the real pipeline.
    let setup = [
        (Vec3::new(0.0, 1.0, 0.0), Vec3::new(1.0, 2.0, -0.5)),
        (Vec3::new(0.1, 2.0, 0.0), Vec3::new(-2.0, 0.5, 1.0)),
        (Vec3::new(-0.1, 3.0, 0.1), Vec3::new(0.5, -1.5, 2.0)),
        (Vec3::new(0.2, 4.0, -0.1), Vec3::new(3.0, 1.0, 0.0)),
        (Vec3::new(-0.2, 5.0, 0.0), Vec3::new(-1.0, -2.0, -1.0)),
        (Vec3::new(0.0, 6.0, 0.2), Vec3::new(2.0, 0.0, 1.5)),
        (Vec3::new(0.1, 7.0, -0.2), Vec3::new(-0.5, 3.0, -2.0)),
        (Vec3::new(-0.1, 8.0, 0.1), Vec3::new(1.5, -1.0, 0.5)),
        (Vec3::new(0.2, 9.0, 0.0), Vec3::new(0.0, 2.5, -1.5)),
        (Vec3::new(-0.2, 10.0, -0.1), Vec3::new(-2.5, 0.5, 1.0)),
        (Vec3::new(0.0, 11.0, 0.1), Vec3::new(1.0, -3.0, 0.0)),
        (Vec3::new(0.1, 12.0, -0.1), Vec3::new(-1.0, 1.0, 2.5)),
        (Vec3::new(-0.1, 13.0, 0.0), Vec3::new(2.0, -0.5, -1.0)),
    ];
    for &(pos, spin) in &setup {
        let (b, m, c) = spinning_sphere(pos, spin, 0.5, 1.0);
        spawn_body(&mut world, b, m, c);
    }
    // Static floor sphere.
    let (b, m, c) = spinning_sphere(
        Vec3::new(0.0, -10.0, 0.0),
        Vec3::ZERO,
        10.0,
        0.0,
    );
    spawn_body(&mut world, b, m, c);

    let dt = 1.0 / 60.0;
    let mut schedule = build_schedule(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);
    world.resource_mut::<PhysicsConfig>().simd = simd;

    for _ in 0..frames {
        schedule.run(&mut world);
    }
    all_bodies(&mut world)
}

/// The 0%-gate: `simd = true` is BIT-IDENTICAL to `simd = false` over a multi-body
/// spinning + stacking scene (the SIMD inertia refresh + integrate are a pure
/// speed path — no value change).
#[test]
fn simd_on_equals_simd_off_bit_identical() {
    let off = run_scene(false, 40);
    let on = run_scene(true, 40);

    assert_eq!(off.len(), on.len(), "body count must match");
    // Anti-vacuity: the dynamic bodies must have actually moved + rotated (else a
    // frozen scene would pass trivially). Body 0 spins, so its rotation departs
    // from IDENTITY.
    assert_ne!(
        off[0].rotation.to_bits_tuple(),
        Quat::IDENTITY.to_bits_tuple(),
        "anti-vacuity: body 0 must have rotated under its spin"
    );

    for (i, (a, b)) in off.iter().zip(on.iter()).enumerate() {
        assert_eq!(a.position.x.to_bits(), b.position.x.to_bits(), "body {i} pos.x");
        assert_eq!(a.position.y.to_bits(), b.position.y.to_bits(), "body {i} pos.y");
        assert_eq!(a.position.z.to_bits(), b.position.z.to_bits(), "body {i} pos.z");
        assert_eq!(
            a.linear_velocity.x.to_bits(),
            b.linear_velocity.x.to_bits(),
            "body {i} vel.x"
        );
        assert_eq!(
            a.linear_velocity.y.to_bits(),
            b.linear_velocity.y.to_bits(),
            "body {i} vel.y"
        );
        assert_eq!(
            a.linear_velocity.z.to_bits(),
            b.linear_velocity.z.to_bits(),
            "body {i} vel.z"
        );
        assert_eq!(a.rotation.x.to_bits(), b.rotation.x.to_bits(), "body {i} rot.x");
        assert_eq!(a.rotation.y.to_bits(), b.rotation.y.to_bits(), "body {i} rot.y");
        assert_eq!(a.rotation.z.to_bits(), b.rotation.z.to_bits(), "body {i} rot.z");
        assert_eq!(a.rotation.w.to_bits(), b.rotation.w.to_bits(), "body {i} rot.w");
        assert_eq!(
            a.angular_velocity.x.to_bits(),
            b.angular_velocity.x.to_bits(),
            "body {i} avel.x"
        );
        assert_eq!(
            a.angular_velocity.y.to_bits(),
            b.angular_velocity.y.to_bits(),
            "body {i} avel.y"
        );
        assert_eq!(
            a.angular_velocity.z.to_bits(),
            b.angular_velocity.z.to_bits(),
            "body {i} avel.z"
        );
    }
}

/// With SIMD on, the solve stays run-to-run bit-deterministic (the SIMD path adds
/// no nondeterminism — same as `solver_is_deterministic` but on the SIMD path).
#[test]
fn simd_on_is_run_to_run_deterministic() {
    let a = run_scene(true, 40);
    let b = run_scene(true, 40);
    assert_eq!(a.len(), b.len());
    for (i, (ba, bb)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(ba.position.y.to_bits(), bb.position.y.to_bits(), "body {i} pos.y");
        assert_eq!(ba.rotation.x.to_bits(), bb.rotation.x.to_bits(), "body {i} rot.x");
        assert_eq!(ba.rotation.w.to_bits(), bb.rotation.w.to_bits(), "body {i} rot.w");
    }
}

/// Helper trait to compare a `Quat`'s raw bits as a tuple (the public `Quat` has
/// no bit-tuple accessor; this keeps the anti-vacuity check terse).
trait QuatBits {
    fn to_bits_tuple(&self) -> (u32, u32, u32, u32);
}

impl QuatBits for Quat {
    fn to_bits_tuple(&self) -> (u32, u32, u32, u32) {
        (
            self.x.to_bits(),
            self.y.to_bits(),
            self.z.to_bits(),
            self.w.to_bits(),
        )
    }
}
