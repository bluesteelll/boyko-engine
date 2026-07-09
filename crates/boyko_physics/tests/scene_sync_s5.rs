//! std-lib S5 gate suite — physics ⇄ `Transform` pose sync + `Sensor`.
//!
//! Covers the five S5 gates:
//!
//! 1. DYNAMIC body → `Transform`: a Dynamic root, stepped under gravity / its own
//!    velocity, has its integrated `RigidBody` pose copied OUT to `Transform` by
//!    `sync_body_to_transform` (`.after(apply)`), then `propagate_transforms`
//!    composes that into `GlobalTransform`.
//! 2. KINEMATIC / STATIC `Transform` → body: a gameplay-authored `Transform` is
//!    copied IN to `RigidBody` by `sync_transform_to_body` (`.before(integrate)`),
//!    so the body pose follows the `Transform`.
//! 3. (bit-determinism is asserted by the FULL `cargo test -p boyko-physics`
//!    determinism suite, unchanged by S5 — see the report.)
//! 4. SENSOR: a sensor-marked overlapping body REPORTS the overlap
//!    (`Manifolds::sensor_overlaps`) but the solver never resolves it (velocity
//!    bit-unchanged); a non-sensor overlap DOES resolve (velocity changes).
//! 5. NO PARALLEL POSE: after a frame, `Transform` and `RigidBody` agree for a
//!    Dynamic body — one pose datum, no third store.
//!
//! All schedules are HAND-BUILT (a `ScheduleBuilder` wired by
//! `add_physics_systems_with_scene_sync`, then a single-threaded pool), mirroring
//! the existing `physics_seam.rs` idiom; the per-frame `propagate_transforms` is
//! driven directly (it is a plain `&mut EcsMaster` function, not a `SystemParam`).

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_macros::Bundle;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    Collider, ColliderShape, Kinematic, RigidBody, RigidBodyMass, Sensor, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems_with_scene_sync;
use boyko_physics::resources::{Manifolds, PhysicsConfig};
use boyko_physics::solver::{NoopSolver, RigidSolver, SoftStepSolver};

use boyko_scene::propagation::propagate_transforms;
use boyko_scene::transform::{GlobalTransform, Transform};

// ── Test-local bundles (Transform alongside the physics columns) ──────────────

/// A rigid body that also carries the engine's spatial pose columns
/// (`Transform` + `GlobalTransform`) so the S5 sync has both sides to copy
/// between. A named struct because the `Bundle` derive rejects tuple bundles.
#[derive(Bundle)]
struct SyncedBody {
    body: RigidBody,
    mass: RigidBodyMass,
    collider: Collider,
    transform: Transform,
    global: GlobalTransform,
}

/// As [`SyncedBody`] but additionally tagged as a [`Sensor`] (a distinct
/// archetype — the sensor membership bit).
#[derive(Bundle)]
struct SyncedSensorBody {
    body: RigidBody,
    mass: RigidBodyMass,
    collider: Collider,
    sensor: Sensor,
    transform: Transform,
    global: GlobalTransform,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Views a `#[repr(C)]` POD value as its raw bytes for the `create_entity` spawn
/// path (the physics components carry enums, so are not `bytemuck::Pod`).
///
/// # Safety
/// `T` is a `#[repr(C)]` component whose byte image is a valid serialization for
/// the pool registered under `T::component_id()` (holds for every component used
/// here — all `#[repr(C)]`, stored by raw byte copy).
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; we view its `size_of::<T>()` bytes read-only
    // for the borrow. `T` is `#[repr(C)]`, matching the pool's stored layout. The
    // slice borrows `value`, so it cannot outlive it.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A deterministic single-threaded pool (the IM-2 determinism precondition).
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// Local replacement for the deleted `BodyType` enum — the test's body-role intent
/// (Decision 2). It maps to the inverse mass plus the `Simulated` / `Kinematic`
/// EnableTag bits the spawn helpers set: `Dynamic` ⇒ `inv_mass = 1`, `Simulated`
/// SET; `Static` / `Kinematic` ⇒ `inv_mass = 0`, `Simulated` CLEAR; `Kinematic`
/// additionally sets the `Kinematic` bit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    Static,
    Kinematic,
    Dynamic,
}

/// A `RigidBodyMass` with the given body kind (unit mass for dynamic, the static
/// `inv_mass == 0` for static/kinematic so the integrator and solver both treat
/// it as immovable). The runtime `Simulated` / `Kinematic` bits are applied by the
/// spawn helpers (the field is gone — Decision 2/3).
fn mass_for(kind: BodyKind) -> RigidBodyMass {
    let inv_mass = match kind {
        BodyKind::Dynamic => 1.0,
        // Static / kinematic bodies are immovable (the solver/integrate skip them).
        BodyKind::Static | BodyKind::Kinematic => 0.0,
    };
    let inv_inertia = if inv_mass == 0.0 {
        Mat3::ZERO
    } else {
        Mat3::IDENTITY
    };
    RigidBodyMass {
        inv_inertia,
        inv_mass,
        restitution: 0.5,
        friction: 0.3,
    }
}

/// A unit-sphere collider (radius 0.5), layer/mask 1.
fn unit_sphere() -> Collider {
    Collider {
        shape: ColliderShape::Sphere { radius: 0.5 },
        layer: 1,
        mask: 1,
    }
}

/// Applies the EnableTag bits for `kind` to a freshly spawned body (Decision 3/6):
/// `Dynamic` ⇒ `Simulated` SET; `Kinematic` ⇒ `Kinematic` SET (and `Simulated`
/// CLEAR); `Static` ⇒ neither. The bit, not a field, now carries the body role.
fn apply_kind_bits(world: &mut EcsMaster, e: Entity, kind: BodyKind) {
    match kind {
        BodyKind::Dynamic => world.enable::<Simulated>(e),
        BodyKind::Kinematic => world.enable::<Kinematic>(e),
        BodyKind::Static => {}
    }
}

/// Spawns a [`SyncedBody`] and returns its entity handle, applying `kind`'s
/// EnableTag bits.
fn spawn_synced(
    world: &mut EcsMaster,
    body: RigidBody,
    mass: RigidBodyMass,
    collider: Collider,
    transform: Transform,
    kind: BodyKind,
) -> Entity {
    let archetype = world.bundle_archetype_id_for::<SyncedBody>();
    let global = GlobalTransform::default();
    let e = world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
                (Transform::component_id(), as_bytes(&transform)),
                (GlobalTransform::component_id(), as_bytes(&global)),
            ],
        )
        .expect("invariant: SyncedBody archetype accepts its five columns");
    apply_kind_bits(world, e, kind);
    e
}

/// Spawns a [`SyncedSensorBody`] and returns its entity handle, applying `kind`'s
/// EnableTag bits.
fn spawn_synced_sensor(
    world: &mut EcsMaster,
    body: RigidBody,
    mass: RigidBodyMass,
    collider: Collider,
    transform: Transform,
    kind: BodyKind,
) -> Entity {
    let archetype = world.bundle_archetype_id_for::<SyncedSensorBody>();
    let global = GlobalTransform::default();
    let sensor = Sensor;
    let e = world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
                (Sensor::component_id(), as_bytes(&sensor)),
                (Transform::component_id(), as_bytes(&transform)),
                (GlobalTransform::component_id(), as_bytes(&global)),
            ],
        )
        .expect("invariant: SyncedSensorBody archetype accepts its six columns");
    apply_kind_bits(world, e, kind);
    e
}

/// Builds a fixed schedule with the physics pipeline + S5 scene sync wired for
/// solver `S`, and installs the test `FixedTime`.
fn build_sync_schedule<S: RigidSolver + Default>(world: &mut EcsMaster, dt: f32) -> Schedule {
    let mut builder = ScheduleBuilder::new(serial_pool());
    let _keys = add_physics_systems_with_scene_sync::<S>(&mut builder, world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(dt)));
    builder.build(world)
}

// ── Gate 1: DYNAMIC body → Transform → GlobalTransform ────────────────────────

/// A Dynamic root spawned at a pose, stepped under gravity with the Foundation
/// integrator (`NoopSolver` does NOT own integration, so `physics_integrate`
/// runs), has its integrated `RigidBody.position` copied OUT to `Transform` by
/// `sync_body_to_transform`, which then composes into `GlobalTransform`.
#[test]
fn dynamic_body_pose_flows_to_transform_and_global() {
    let mut world = EcsMaster::new();

    let start = Vec3::new(7.0, 3.0, -2.0);
    let body = RigidBody {
        position: start,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let entity = spawn_synced(
        &mut world,
        body,
        mass_for(BodyKind::Dynamic),
        unit_sphere(),
        Transform::from_translation(start),
        BodyKind::Dynamic,
    );

    let dt = 1.0 / 64.0;
    let mut schedule = build_sync_schedule::<NoopSolver>(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -10.0, 0.0);

    const FRAMES: u32 = 8;
    for _ in 0..FRAMES {
        schedule.run(&mut world);
    }

    // The body integrated downward under gravity.
    let rb = *world
        .get_component::<RigidBody>(entity)
        .expect("body lives");
    assert!(
        rb.position.y < start.y,
        "Dynamic body fell under gravity: rb.y = {} (start {})",
        rb.position.y,
        start.y
    );

    // `sync_body_to_transform` copied the integrated pose OUT, bit-exact.
    let t = *world
        .get_component::<Transform>(entity)
        .expect("transform lives");
    assert_eq!(
        t.translation, rb.position,
        "Transform.translation bit-equals the integrated RigidBody.position (sync ran last)"
    );
    assert_eq!(
        t.rotation, rb.rotation,
        "Transform.rotation bit-equals the integrated RigidBody.rotation"
    );
    // Scale is never touched by the sync.
    assert_eq!(
        t.scale,
        Vec3::ONE,
        "Transform.scale is left as authored (sync does not write it)"
    );

    // Propagate → GlobalTransform reflects the moved Transform.
    propagate_transforms(&mut world);
    let g = *world
        .get_component::<GlobalTransform>(entity)
        .expect("global lives");
    assert_eq!(
        g.translation(),
        t.translation,
        "GlobalTransform.translation reflects the synced Transform (root compose)"
    );
}

// ── Gate 2: KINEMATIC / STATIC Transform → body ───────────────────────────────

/// A Kinematic body's gameplay-authored `Transform` is copied IN to `RigidBody`
/// by `sync_transform_to_body` (runs `.before(integrate)`), so after a step the
/// body pose follows the authored `Transform` (and gravity does NOT move it).
#[test]
fn kinematic_transform_drives_body() {
    let mut world = EcsMaster::new();

    // Spawn with a STALE RigidBody pose; the authored Transform is the truth.
    let authored = Vec3::new(2.0, 9.0, 4.0);
    let authored_rot = Quat::new(0.0, 0.0, 0.382_683_43, 0.923_879_5); // ~45° about Z
    let stale_body = RigidBody {
        position: Vec3::new(-100.0, -100.0, -100.0),
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let entity = spawn_synced(
        &mut world,
        stale_body,
        mass_for(BodyKind::Kinematic),
        unit_sphere(),
        Transform {
            translation: authored,
            rotation: authored_rot,
            scale: Vec3::ONE,
        },
        BodyKind::Kinematic,
    );

    let dt = 1.0 / 64.0;
    let mut schedule = build_sync_schedule::<NoopSolver>(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -10.0, 0.0);

    schedule.run(&mut world);

    let rb = *world
        .get_component::<RigidBody>(entity)
        .expect("body lives");
    assert_eq!(
        rb.position, authored,
        "Kinematic RigidBody.position followed the authored Transform (sync ran first)"
    );
    assert_eq!(
        rb.rotation, authored_rot,
        "Kinematic RigidBody.rotation followed the authored Transform"
    );
    // The Transform itself is unchanged (gameplay owns it; nothing wrote it back).
    let t = *world
        .get_component::<Transform>(entity)
        .expect("transform lives");
    assert_eq!(
        t.translation, authored,
        "Kinematic Transform stays gameplay-authored (no body→transform write)"
    );
}

/// A Static body's `Transform` likewise drives the `RigidBody` pose (the
/// `sync_transform_to_body` path runs for every non-Dynamic body).
#[test]
fn static_transform_drives_body() {
    let mut world = EcsMaster::new();

    let authored = Vec3::new(-5.0, 0.5, 11.0);
    let stale_body = RigidBody {
        position: Vec3::ZERO,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let entity = spawn_synced(
        &mut world,
        stale_body,
        mass_for(BodyKind::Static),
        unit_sphere(),
        Transform::from_translation(authored),
        BodyKind::Static,
    );

    let dt = 1.0 / 64.0;
    let mut schedule = build_sync_schedule::<NoopSolver>(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -10.0, 0.0);

    schedule.run(&mut world);

    let rb = *world
        .get_component::<RigidBody>(entity)
        .expect("body lives");
    assert_eq!(
        rb.position, authored,
        "Static RigidBody.position followed the authored Transform"
    );
    assert!(
        rb.position.y > 0.0,
        "Static body did NOT fall under gravity (gameplay-authored, integrate skips it)"
    );
}

// ── Gate 4: SENSOR reports without resolving; non-sensor resolves ─────────────

/// Steps a single frame of the real (resolving) `SoftStepSolver` over two deeply
/// overlapping spheres, returning the dynamic body's post-step linear velocity
/// plus the (manifold count, sensor-overlap count). `sensor` decides whether the
/// SECOND (static wall) body carries the `Sensor` marker.
fn run_overlap_frame(sensor: bool) -> (Vec3, usize, usize) {
    let mut world = EcsMaster::new();

    // Dynamic body moving toward the wall (no gravity, so the ONLY velocity
    // writer is the contact solve).
    let approach = Vec3::new(5.0, 0.0, 0.0);
    let dyn_body = RigidBody {
        position: Vec3::ZERO,
        linear_velocity: approach,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let dyn_entity = spawn_synced(
        &mut world,
        dyn_body,
        mass_for(BodyKind::Dynamic),
        unit_sphere(),
        Transform::from_translation(Vec3::ZERO),
        BodyKind::Dynamic,
    );

    // A deeply overlapping static body at x = 0.4 (center distance 0.4 < 1.0 sum
    // → penetrating), optionally a sensor.
    let wall_pos = Vec3::new(0.4, 0.0, 0.0);
    let wall_body = RigidBody {
        position: wall_pos,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let wall_mass = mass_for(BodyKind::Static);
    let wall_tf = Transform::from_translation(wall_pos);
    if sensor {
        spawn_synced_sensor(&mut world, wall_body, wall_mass, unit_sphere(), wall_tf, BodyKind::Static);
    } else {
        spawn_synced(&mut world, wall_body, wall_mass, unit_sphere(), wall_tf, BodyKind::Static);
    }

    let dt = 1.0 / 64.0;
    // The real TGS solver RESOLVES contacts (changes velocity); it owns
    // integration, so `physics_integrate` is gated off and gravity rides inside
    // the substep loop (set to zero below to isolate the contact impulse).
    let mut schedule = build_sync_schedule::<SoftStepSolver>(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;

    schedule.run(&mut world);

    let rb = *world
        .get_component::<RigidBody>(dyn_entity)
        .expect("dynamic body lives");
    let manifolds = world.resource::<Manifolds>();
    (
        rb.linear_velocity,
        manifolds.manifolds.len(),
        manifolds.sensor_overlaps.len(),
    )
}

/// A non-sensor overlapping pair RESOLVES: the solver emits a manifold and the
/// dynamic body's approach velocity changes (an impulse opposed the approach).
#[test]
fn non_sensor_overlap_resolves_velocity() {
    let approach = Vec3::new(5.0, 0.0, 0.0);
    let (vel, manifold_count, sensor_count) = run_overlap_frame(false);

    assert!(
        manifold_count >= 1,
        "a non-sensor overlap produces a solver manifold (got {manifold_count})"
    );
    assert_eq!(
        sensor_count, 0,
        "a non-sensor world reports zero sensor overlaps"
    );
    assert_ne!(
        vel, approach,
        "the solver resolved the contact — the dynamic body's velocity changed"
    );
    // The impulse opposed the +X approach (the wall pushes back along −X).
    assert!(
        vel.x < approach.x,
        "the resolved velocity decelerated along the contact normal: {} -> {}",
        approach.x,
        vel.x
    );
}

/// A sensor-marked overlapping pair REPORTS the overlap but does NOT resolve it:
/// the solver buffer stays empty (so no impulse), the sensor-overlap buffer is
/// populated, and the dynamic body's velocity is bit-unchanged (it passes
/// through).
#[test]
fn sensor_overlap_reports_without_resolving() {
    let approach = Vec3::new(5.0, 0.0, 0.0);
    let (vel, manifold_count, sensor_count) = run_overlap_frame(true);

    assert_eq!(
        manifold_count, 0,
        "a sensor pair never enters the solver manifold buffer (got {manifold_count})"
    );
    assert!(
        sensor_count >= 1,
        "the sensor overlap is REPORTED in sensor_overlaps (got {sensor_count})"
    );
    assert_eq!(
        vel, approach,
        "the dynamic body passes through the sensor — velocity bit-unchanged"
    );
}

/// Cross-check: with everything else identical, ONLY the `Sensor` marker decides
/// whether the velocity changes — the sensor passes through, the non-sensor does
/// not. (One assert per behavior; this binds the two arms to the same scene.)
#[test]
fn sensor_marker_is_the_only_difference() {
    let (sensor_vel, _, sensor_overlaps) = run_overlap_frame(true);
    let (solid_vel, solid_manifolds, _) = run_overlap_frame(false);

    assert_ne!(
        sensor_vel, solid_vel,
        "the same overlap resolves WITHOUT the marker and passes through WITH it"
    );
    assert!(
        sensor_overlaps >= 1 && solid_manifolds >= 1,
        "each arm produced its own signal (sensor report vs solver manifold)"
    );
}

// ── Gate 5: NO PARALLEL POSE (one datum, no third store) ──────────────────────

/// After a frame, a Dynamic body's `Transform` and `RigidBody` agree bit-for-bit:
/// the world pose lives in ONE datum copied one-directionally, not two writers
/// fighting over a third store.
#[test]
fn no_parallel_pose_transform_and_body_agree() {
    let mut world = EcsMaster::new();

    let start = Vec3::new(1.0, 10.0, -3.0);
    let body = RigidBody {
        position: start,
        linear_velocity: Vec3::new(2.0, 0.0, -1.0),
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::new(0.0, 0.0, 1.0),
    };
    let entity = spawn_synced(
        &mut world,
        body,
        mass_for(BodyKind::Dynamic),
        unit_sphere(),
        Transform::from_translation(start),
        BodyKind::Dynamic,
    );

    let dt = 1.0 / 64.0;
    let mut schedule = build_sync_schedule::<NoopSolver>(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);

    const FRAMES: u32 = 5;
    for _ in 0..FRAMES {
        schedule.run(&mut world);
    }

    let rb = *world
        .get_component::<RigidBody>(entity)
        .expect("body lives");
    let t = *world
        .get_component::<Transform>(entity)
        .expect("transform lives");

    // The single source of truth: the two columns hold the SAME pose, exactly.
    assert_eq!(
        t.translation, rb.position,
        "Transform and RigidBody translation agree (one pose datum, no parallel store)"
    );
    assert_eq!(
        t.rotation, rb.rotation,
        "Transform and RigidBody rotation agree (one pose datum)"
    );

    // And the moved pose is genuinely the integrated one (not a stale default).
    assert!(
        rb.position.y < start.y && rb.position != start,
        "the body actually moved (the agreement is not a no-op)"
    );
}

// ── Determinism cross-check: scene sync does not perturb the solve ────────────

/// Running the FULL pipeline with scene sync wired produces the bit-identical
/// `RigidBody` pose to running it WITHOUT scene sync (the sync wraps around the
/// solve and copies plain fields — it never touches the integrate/solve floats).
/// This is the local witness for the HARD bit-determinism gate; the full
/// determinism suite (`cargo test -p boyko-physics`) is the authoritative check.
#[test]
fn scene_sync_does_not_perturb_solve_bit_identical() {
    /// Steps `frames` of a falling dynamic body and returns its `RigidBody`.
    /// `with_sync` selects the sync-wrapped pipeline vs the plain one.
    fn run(with_sync: bool, frames: u32) -> RigidBody {
        let mut world = EcsMaster::new();
        let start = Vec3::new(0.0, 50.0, 0.0);
        let body = RigidBody {
            position: start,
            linear_velocity: Vec3::new(1.0, 0.0, 2.0),
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::new(0.5, 0.0, 0.0),
        };
        let entity = spawn_synced(
            &mut world,
            body,
            mass_for(BodyKind::Dynamic),
            unit_sphere(),
            Transform::from_translation(start),
            BodyKind::Dynamic,
        );

        let dt = 1.0 / 64.0;
        let mut builder = ScheduleBuilder::new(serial_pool());
        if with_sync {
            let _ = add_physics_systems_with_scene_sync::<SoftStepSolver>(&mut builder, &mut world);
        } else {
            let _ = boyko_physics::plugin::add_physics_systems::<SoftStepSolver>(
                &mut builder,
                &mut world,
            );
        }
        world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(dt)));
        let mut schedule = builder.build(&mut world);
        world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -10.0, 0.0);

        for _ in 0..frames {
            schedule.run(&mut world);
        }
        *world.get_component::<RigidBody>(entity).expect("body lives")
    }

    const FRAMES: u32 = 16;
    let with = run(true, FRAMES);
    let without = run(false, FRAMES);

    assert_eq!(
        with, without,
        "scene sync does NOT shift any solve float — the RigidBody pose is bit-identical"
    );
}

/// Adding a `Sensor` body to a scene must not perturb the NON-sensor solve
/// result: the dynamic body's resolved velocity against a solid wall is
/// bit-identical whether or not a (disjoint, non-overlapping) sensor body also
/// exists in the world.
#[test]
fn adding_sensor_does_not_perturb_nonsensor_solve() {
    /// Resolves the dynamic-vs-solid-wall contact, optionally also spawning a
    /// FAR-AWAY sensor body (disjoint — it never overlaps anything), and returns
    /// the dynamic body's post-step velocity.
    fn run(extra_sensor: bool) -> Vec3 {
        let mut world = EcsMaster::new();

        let approach = Vec3::new(5.0, 0.0, 0.0);
        let dyn_entity = spawn_synced(
            &mut world,
            RigidBody {
                position: Vec3::ZERO,
                linear_velocity: approach,
                rotation: Quat::IDENTITY,
                angular_velocity: Vec3::ZERO,
            },
            mass_for(BodyKind::Dynamic),
            unit_sphere(),
            Transform::from_translation(Vec3::ZERO),
            BodyKind::Dynamic,
        );
        // The solid wall the dynamic body actually hits.
        let wall_pos = Vec3::new(0.4, 0.0, 0.0);
        spawn_synced(
            &mut world,
            RigidBody {
                position: wall_pos,
                linear_velocity: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                angular_velocity: Vec3::ZERO,
            },
            mass_for(BodyKind::Static),
            unit_sphere(),
            Transform::from_translation(wall_pos),
            BodyKind::Static,
        );
        if extra_sensor {
            // A sensor body FAR away — it overlaps nothing, so it adds a sensor
            // overlap signal of zero and must not touch the solid solve.
            let far = Vec3::new(1000.0, 1000.0, 1000.0);
            spawn_synced_sensor(
                &mut world,
                RigidBody {
                    position: far,
                    linear_velocity: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    angular_velocity: Vec3::ZERO,
                },
                mass_for(BodyKind::Static),
                unit_sphere(),
                Transform::from_translation(far),
                BodyKind::Static,
            );
        }

        let dt = 1.0 / 64.0;
        let mut schedule = build_sync_schedule::<SoftStepSolver>(&mut world, dt);
        world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;
        schedule.run(&mut world);

        world
            .get_component::<RigidBody>(dyn_entity)
            .expect("dynamic body lives")
            .linear_velocity
    }

    let plain = run(false);
    let with_sensor = run(true);
    assert_eq!(
        plain, with_sensor,
        "a disjoint sensor body does not perturb the non-sensor solve (bit-identical)"
    );
}
