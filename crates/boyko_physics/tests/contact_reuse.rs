//! G-L9b-4, the system arm (L9 C3, `docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md`):
//! a resting two-box stack on a static floor, contact reuse on, on the default pipeline. The first
//! step builds both contacts' records from the full collision; from the second step on, every
//! touching box pair reuses its record — 100 % hits — for as long as the stack rests.
//!
//! The narrowphase's own classes (`Manifolds::pair_classes`) are read after every step. Their
//! closure is asserted too, and the stack must be at rest in fact, not by a vacuous count: two
//! touching pairs every step, and a top box that has not fallen or drifted.

#![cfg(not(miri))]

use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{Manifolds, PhysicsConfig};
use boyko_physics::solver::DefaultRigidSolver;

/// Fixed timestep.
const DT: f32 = 1.0 / 60.0;
/// Steps the stack rests for.
const STEPS: usize = 120;

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its `size_of::<T>()` bytes
    // read-only for the duration of the borrow, which is the exact layout the component pool
    // stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Spawns a box of `half` extents at `position` into `archetype`, dynamic iff `dynamic`.
fn spawn(
    world: &mut EcsMaster,
    archetype: ArchetypeId,
    position: Vec3,
    half: Vec3,
    dynamic: bool,
) -> Entity {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: if dynamic { Mat3::from_diagonal(Vec3::new(6.0, 6.0, 6.0)) } else { Mat3::ZERO },
        inv_mass: if dynamic { 1.0 } else { 0.0 },
        restitution: 0.0,
        friction: 0.5,
    };
    let collider = Collider { shape: ColliderShape::Box { half_extents: half }, layer: 1, mask: 1 };
    let e = world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("construction: the body archetype accepts its columns");
    if dynamic {
        world.enable::<Simulated>(e);
    }
    e
}

#[test]
fn a_resting_two_box_stack_reuses_every_contact_after_one_step() {
    let mut world = EcsMaster::new();
    let archetype = world.create_archetype(&[
        RigidBody::component_id(),
        RigidBodyMass::component_id(),
        Collider::component_id(),
    ]);
    let unit = Vec3::new(0.5, 0.5, 0.5);
    spawn(&mut world, archetype, Vec3::new(0.0, -0.5, 0.0), Vec3::new(5.0, 0.5, 5.0), false);
    spawn(&mut world, archetype, Vec3::new(0.0, 0.5, 0.0), unit, true);
    let top = spawn(&mut world, archetype, Vec3::new(0.0, 1.5, 0.0), unit, true);
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
    add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    let mut physics = builder.build(&mut world);
    let cfg = world.resource_mut::<PhysicsConfig>();
    assert!(!cfg.contact_reuse, "L9 C3: contact reuse is off by default");
    cfg.contact_reuse = true;

    let mut rows = Vec::with_capacity(STEPS);
    for step in 0..STEPS {
        physics.run(&mut world);
        let c = world.resource::<Manifolds>().pair_classes();
        assert_eq!(
            c.full + c.reused + c.sep_hits + c.non_box,
            c.pairs,
            "step {step}: the pair classes close ({c:?})"
        );
        let touching = c.reused + c.full_contacts;
        assert_eq!(touching, 2, "step {step}: the floor-box and box-box contacts touch ({c:?})");
        rows.push((step, c.reused, c.full_contacts, c.records_built));
    }
    println!("G-L9b-4 stack (step, reused, full contacts, records built): {rows:?}");
    let (_, reused0, _, built0) = rows[0];
    assert_eq!((reused0, built0), (0, 2), "the first step builds both records: {:?}", rows[0]);
    for &(step, reused, full_contacts, _) in &rows[1..] {
        assert_eq!(
            (reused, full_contacts),
            (2, 0),
            "step {step}: every touching pair of a resting stack reuses its record"
        );
    }
    let y = world.get_component::<RigidBody>(top).expect("invariant: the top box is live").position.y;
    assert!(
        (y - 1.5).abs() < 0.01,
        "the stack must rest, or the reuse is not a resting stack's: top y {y}"
    );
}
