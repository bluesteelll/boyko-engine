//! BodyType → EnableTag-bit refactor: the toggle / capability-state gate suite.
//!
//! These tests pin the NEW behaviors the refactor introduces (the byte-identity
//! determinism oracle lives in `bodytype_determinism_golden.rs`; the P2 physics
//! gates in `softstep.rs`; the pose-sync split in `scene_sync_s5.rs`). They assert
//! the capability+state model the plan specifies:
//!
//! - (a) a body with `Simulated` OFF is GATHERED but NOT integrated (freezes like
//!   the old Static); flipping `Simulated` ON resumes simulation next step.
//! - (b) the flip is O(1) — NO archetype migration: `archetype_count()` is
//!   unchanged across the toggle, and the entity stays in the same query result
//!   (a bitset tag is never in any archetype signature mask).
//! - (c) `SolverScratch.bodies()[row].simulated` reflects the toggled bit per row
//!   in gather order (the `IsEnabled<Simulated>` capture is order-preserving).
//! - (d) a Kinematic body (`Kinematic` SET, `Simulated` CLEAR, `inv_mass == 0`) is
//!   gathered with `kinematic == true` and its pose stays frozen.
//! - (e) an entity LACKING the dynamic-physics columns (`Collider`-only, no
//!   `RigidBody`/`RigidBodyMass`) is structurally skipped by `physics_gather`
//!   (never entered into `SolverScratch.bodies()`).
//! - (f) pipeline `physics_integrate` (NoopSolver, non-`SolverOwned`): a
//!   `Simulated`-OFF dynamic does NOT gravity-integrate; a `Simulated`-ON dynamic
//!   does.
//! - (h) parked dynamic (`RigidBody` + `inv_mass != 0` + `Simulated` OFF): NOT
//!   synced from `Transform`, NOT written out, pose bit-stable across N steps.
//!
//! Spins up `boyko_threadpool` (intractable under Miri — the pool is loom+Miri
//! proven in the ECS Phase-9 series), so `cfg(not(miri))`. The pure datum / gather
//! / integrate paths are covered Miri-clean by `bodytype_bits_toggle_miri.rs`.

#![cfg(not(miri))]

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_macros::Bundle;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    Collider, ColliderShape, Kinematic, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{PhysicsConfig, SolverScratch};
use boyko_physics::solver::NoopSolver;

// ── Helpers (mirror scene_sync_s5.rs / colored_acceptance_o5.rs) ──────────────

/// Views a `#[repr(C)]` POD value as its raw bytes for the `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; we view its `size_of::<T>()`
    // bytes read-only for the borrow — the exact layout the pool stores.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A deterministic single-threaded pool (the IM-2 determinism precondition).
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// A `Collider`-only body (no physics dynamics) — used for the structural-skip
/// gate (e). It is a DISTINCT archetype from `RigidBodyBundle`.
#[derive(Bundle)]
struct ColliderOnly {
    collider: Collider,
}

fn unit_sphere() -> Collider {
    Collider {
        shape: ColliderShape::Sphere { radius: 0.5 },
        layer: 1,
        mask: 1,
    }
}

/// A dynamic-capable `RigidBodyMass` (unit inverse mass + identity inertia).
fn dynamic_mass() -> RigidBodyMass {
    RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.0,
        friction: 0.5,
    }
}

/// A `RigidBody` at `position`, at rest (no velocity / spin).
fn body_at(position: Vec3) -> RigidBody {
    RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    }
}

/// Spawns a `RigidBodyBundle` body and returns its entity (no bits set yet).
fn spawn_rigid(
    world: &mut EcsMaster,
    body: RigidBody,
    mass: RigidBodyMass,
    collider: Collider,
) -> Entity {
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
        .expect("invariant: RigidBodyBundle archetype accepts the three columns")
}

/// Builds the Foundation-integrator pipeline (NoopSolver does NOT own
/// integration, so `physics_integrate` runs — the path gate (a)/(f)/(h) exercise).
fn build_pipeline(world: &mut EcsMaster, dt: f32) -> Schedule {
    let mut builder = ScheduleBuilder::new(serial_pool());
    let _keys = add_physics_systems::<NoopSolver>(&mut builder, world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(dt)));
    builder.build(world)
}

const DT: f32 = 1.0 / 60.0;
const GRAVITY: Vec3 = Vec3::new(0.0, -10.0, 0.0);

// ── Gate (a): Simulated OFF freezes; flipping ON resumes ──────────────────────

/// A dynamic-capable body with `Simulated` CLEAR does NOT integrate under gravity
/// (its pose is bit-stable across steps), then flipping `Simulated` ON makes it
/// fall on subsequent steps.
#[test]
fn simulated_off_freezes_dynamic_then_on_resumes() {
    let mut world = EcsMaster::new();
    let start = Vec3::new(0.0, 10.0, 0.0);
    let e = spawn_rigid(&mut world, body_at(start), dynamic_mass(), unit_sphere());
    // Spawn with the bit CLEAR — a parked dynamic.

    let mut schedule = build_pipeline(&mut world, DT);
    world.resource_mut::<PhysicsConfig>().gravity = GRAVITY;

    // Phase 1: Simulated OFF → frozen.
    for _ in 0..30 {
        schedule.run(&mut world);
    }
    let frozen = *world.get_component::<RigidBody>(e).expect("body lives");
    assert_eq!(
        frozen.position, start,
        "a Simulated-OFF dynamic body does not integrate (pose bit-stable, frozen like Static)"
    );

    // Phase 2: flip Simulated ON → it falls.
    world.enable::<Simulated>(e);
    for _ in 0..30 {
        schedule.run(&mut world);
    }
    let resumed = *world.get_component::<RigidBody>(e).expect("body lives");
    assert!(
        resumed.position.y < start.y,
        "after enabling Simulated the body resumes integrating (y {} < start {})",
        resumed.position.y,
        start.y
    );
}

// ── Gate (b): the flip is O(1) — no archetype migration ───────────────────────

/// Toggling `Simulated` on a live body changes neither `archetype_count()` nor the
/// body's query membership: a bitset tag is never in any archetype signature mask,
/// so the toggle is an in-place one-bit RMW (no migration, no row move).
#[test]
fn simulated_toggle_does_not_migrate_archetype() {
    let mut world = EcsMaster::new();
    let e = spawn_rigid(
        &mut world,
        body_at(Vec3::new(0.0, 5.0, 0.0)),
        dynamic_mass(),
        unit_sphere(),
    );

    let before = world.archetype_count();
    // The body is in the RigidBody query before the flip.
    assert_eq!(
        world.query::<&RigidBody, ()>().iter().count(),
        1,
        "the body is present in the RigidBody query before the flip"
    );

    world.enable::<Simulated>(e);
    assert_eq!(
        world.archetype_count(),
        before,
        "enabling a bitset tag creates NO new archetype (O(1) no-migration)"
    );

    world.disable::<Simulated>(e);
    assert_eq!(
        world.archetype_count(),
        before,
        "disabling a bitset tag creates NO new archetype (O(1) no-migration)"
    );
    // And the body still matches the same query, at the same single row.
    assert_eq!(
        world.query::<&RigidBody, ()>().iter().count(),
        1,
        "the body remains in the same query after the toggle (no row move / drop)"
    );
}

// ── Gate (c): the captured bit reflects the toggle per row in gather order ─────

/// After a step, `SolverScratch.bodies()` carries one row per gathered body in
/// gather (= spawn) order, and each row's `simulated` flag equals the body's
/// toggled `Simulated` bit (the `IsEnabled<Simulated>` capture is order-preserving
/// and never drops a row).
#[test]
fn gather_captures_simulated_bit_in_row_order() {
    let mut world = EcsMaster::new();
    // Spawn order: [ON, OFF, ON] — interleaved so a wrong mapping would surface.
    let e0 = spawn_rigid(&mut world, body_at(Vec3::new(0.0, 5.0, 0.0)), dynamic_mass(), unit_sphere());
    let _e1 = spawn_rigid(&mut world, body_at(Vec3::new(2.0, 5.0, 0.0)), dynamic_mass(), unit_sphere());
    let e2 = spawn_rigid(&mut world, body_at(Vec3::new(4.0, 5.0, 0.0)), dynamic_mass(), unit_sphere());
    world.enable::<Simulated>(e0);
    // e1 left OFF.
    world.enable::<Simulated>(e2);

    let mut schedule = build_pipeline(&mut world, DT);
    world.resource_mut::<PhysicsConfig>().gravity = GRAVITY;
    schedule.run(&mut world);

    let scratch = world.resource::<SolverScratch>();
    let bodies = scratch.bodies();
    assert_eq!(bodies.len(), 3, "all three bodies are gathered (non-filtering capture)");
    assert!(bodies[0].simulated, "row 0 (e0) captured Simulated ON");
    assert!(!bodies[1].simulated, "row 1 (e1) captured Simulated OFF");
    assert!(bodies[2].simulated, "row 2 (e2) captured Simulated ON");
}

// ── Gate (d): Kinematic body gathered, pose frozen ────────────────────────────

/// A Kinematic body (`Kinematic` SET, `Simulated` CLEAR, `inv_mass == 0`) is
/// gathered with `kinematic == true` and `simulated == false`, and its pose stays
/// frozen across steps (kinematic MOTION is unbuilt; the bit is captured).
#[test]
fn kinematic_body_gathered_with_bit_and_pose_frozen() {
    let mut world = EcsMaster::new();
    let start = Vec3::new(1.0, 0.5, 0.0);
    let kin_mass = RigidBodyMass {
        inv_inertia: Mat3::ZERO,
        inv_mass: 0.0,
        restitution: 0.0,
        friction: 0.6,
    };
    let kin_body = RigidBody {
        position: start,
        linear_velocity: Vec3::new(-1.0, 0.0, 0.0),
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let e = spawn_rigid(&mut world, kin_body, kin_mass, unit_sphere());
    world.enable::<Kinematic>(e);

    let mut schedule = build_pipeline(&mut world, DT);
    world.resource_mut::<PhysicsConfig>().gravity = GRAVITY;
    schedule.run(&mut world);

    let scratch = world.resource::<SolverScratch>();
    let row = &scratch.bodies()[0];
    assert!(row.kinematic, "the Kinematic bit is captured at gather");
    assert!(!row.simulated, "a kinematic body has Simulated CLEAR");

    let rb = *world.get_component::<RigidBody>(e).expect("body lives");
    assert_eq!(
        rb.position, start,
        "the kinematic body's pose stays frozen (kinematic motion unbuilt; not integrated)"
    );
}

// ── Gate (e): an entity lacking the dynamic columns is structurally skipped ────

/// A `Collider`-only entity (no `RigidBody`/`RigidBodyMass`) never enters
/// `SolverScratch.bodies()`: the gather query requires the dynamics columns, so an
/// entity that lacks them is structurally skipped (capability = component
/// PRESENCE).
#[test]
fn collider_only_entity_is_structurally_skipped_by_gather() {
    let mut world = EcsMaster::new();

    // One real rigid body (gathered) + one collider-only entity (must be skipped).
    let rigid = spawn_rigid(
        &mut world,
        body_at(Vec3::new(0.0, 5.0, 0.0)),
        dynamic_mass(),
        unit_sphere(),
    );
    world.enable::<Simulated>(rigid);

    let collider_only_archetype = world.bundle_archetype_id_for::<ColliderOnly>();
    let collider = unit_sphere();
    let _co = world
        .create_entity(
            collider_only_archetype,
            &[(Collider::component_id(), as_bytes(&collider))],
        )
        .expect("invariant: ColliderOnly archetype accepts the one column");

    let mut schedule = build_pipeline(&mut world, DT);
    world.resource_mut::<PhysicsConfig>().gravity = GRAVITY;
    schedule.run(&mut world);

    let scratch = world.resource::<SolverScratch>();
    assert_eq!(
        scratch.bodies().len(),
        1,
        "only the RigidBody entity is gathered; the Collider-only entity is structurally skipped"
    );
}

// ── Gate (f): pipeline integrate gate (NoopSolver / non-SolverOwned) ──────────

/// In the pipeline path (NoopSolver, integrate runs), a `Simulated`-OFF dynamic
/// does NOT gravity-integrate while a `Simulated`-ON dynamic does — both in the
/// same world, so the gate is the only difference.
#[test]
fn pipeline_integrate_gates_on_simulated_bit() {
    let mut world = EcsMaster::new();
    let start = Vec3::new(0.0, 10.0, 0.0);
    let on = spawn_rigid(&mut world, body_at(start), dynamic_mass(), unit_sphere());
    let off = spawn_rigid(&mut world, body_at(start), dynamic_mass(), unit_sphere());
    world.enable::<Simulated>(on);
    // `off` left with Simulated CLEAR.

    let mut schedule = build_pipeline(&mut world, DT);
    world.resource_mut::<PhysicsConfig>().gravity = GRAVITY;
    for _ in 0..20 {
        schedule.run(&mut world);
    }

    let on_pos = world.get_component::<RigidBody>(on).expect("on lives").position;
    let off_pos = world.get_component::<RigidBody>(off).expect("off lives").position;
    assert!(
        on_pos.y < start.y,
        "the Simulated-ON dynamic integrated under gravity (y {} < {})",
        on_pos.y,
        start.y
    );
    assert_eq!(
        off_pos, start,
        "the Simulated-OFF dynamic did NOT integrate (pose frozen)"
    );
}

// ── Gate (h): parked dynamic — frozen, not synced in either direction ─────────

/// A parked dynamic (`RigidBody` + `inv_mass != 0` + `Simulated` OFF) keeps its
/// pose bit-stable across N steps: integrate skips it (gate a) and — exercised via
/// the integrate pipeline here — nothing else moves it. (The scene-sync freeze of a
/// parked dynamic is additionally covered in `scene_sync_s5.rs`.)
#[test]
fn parked_dynamic_pose_is_bit_stable() {
    let mut world = EcsMaster::new();
    let start = Vec3::new(3.0, 7.0, -2.0);
    let e = spawn_rigid(&mut world, body_at(start), dynamic_mass(), unit_sphere());
    // inv_mass != 0 (dynamic-capable) but Simulated OFF — a parked dynamic.

    let mut schedule = build_pipeline(&mut world, DT);
    world.resource_mut::<PhysicsConfig>().gravity = GRAVITY;
    for _ in 0..50 {
        schedule.run(&mut world);
    }

    let rb = *world.get_component::<RigidBody>(e).expect("body lives");
    assert_eq!(
        rb.position, start,
        "a parked dynamic (Simulated OFF, inv_mass != 0) is frozen in place"
    );
    assert_eq!(
        rb.linear_velocity,
        Vec3::ZERO,
        "a parked dynamic accrues no velocity (gravity is not integrated)"
    );
}
