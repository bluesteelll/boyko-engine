//! Defect A5, second instance: `physics_soft_rigid_apply` must land the soft-rigid reaction
//! on the body it was computed for.
//!
//! The coupled soft step writes its reaction into `SoftRigidReaction::dv_lin` / `dv_ang`,
//! indexed by GATHER row, and `physics_soft_rigid_apply` pairs those rows with its own walk by
//! position. Before the A5 fix that walk was `Query<Mut<RigidBody>>`, so an entity carrying
//! `RigidBody` outside the body set, walked before the bodies, received row 0's reaction and
//! shifted every later body by one row. The fix gives the stage `BodyQuery<BodySoftApplyData>`
//! (`boyko_physics::body_set`), the gather's own row selection.
//!
//! # Scene
//!
//! The real `add_physics_soft::<SoftStepSolver>(.., coupling = true)` schedule with gravity
//! off (as in `soft_body_sp2::pipeline::coupling_pipeline_forces_grid_and_moves_body`): one
//! light dynamic sphere at the origin and one soft particle just above it moving down, three
//! steps. The STRAY is `{RigidBody, RigidBodyMass}` (`inv_mass` 0, `Simulated` clear, no
//! `Collider`), and its archetype is created BEFORE the sphere's, so an unfiltered walk visits
//! it first. The control is the same scene without the stray. The stray is not gathered, so
//! the solver sees an identical world in both runs.
//!
//! **The scene ASSERTS that walk order before it steps** (`carrier_walk`), because every
//! assertion below rests on it and none of them reads it. `coupled_scene` has exactly one
//! body-set member, so there is no "the bodies behind it shift" signal to fall back on: with
//! the stray walked LAST it falls off the end of the one-row snapshot and is skipped, and the
//! whole file is green on unfixed code. Reordering the two `create_archetype` calls is a
//! one-line edit, so the premise is checked rather than argued (round-3 review W1).
//!
//! A debug build of a regression trips the soft apply's row-count `debug_assert!` inside a
//! scheduler worker, which blocks `Schedule::run` today instead of propagating, so every
//! scene runs on its own thread under a watchdog.
//!
//! Device-free; every scene spins up a `boyko_threadpool`, which is intractable under Miri.

use std::panic;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::data::Mut;
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::Resource;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_soft;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::sdf_query::SdfField;
use boyko_physics::soft::SoftBody;
use boyko_physics::solver::SoftStepSolver;

/// The fixed step (60 Hz).
const DT: f32 = 1.0 / 60.0;
/// Steps per scene (as in the SP2 pipeline gate).
const STEPS: usize = 3;
/// The soft particle's start height: it penetrates the sphere of radius 0.6 at the origin.
const PARTICLE_Y: f32 = 0.55;
/// The soft particle's start velocity along `y`: moving down into the sphere.
const PARTICLE_VEL_Y: f32 = -2.0;

/// Wall-clock budget for one scene. NOT a performance measurement: a liveness guard sized
/// far above an honest three-step run, so only a hang reaches it.
const SCENE_TIMEOUT: Duration = Duration::from_secs(if cfg!(debug_assertions) { 120 } else { 60 });

/// Runs `scene` on its own thread and fails the test if it has not finished within
/// [`SCENE_TIMEOUT`], instead of blocking the process (a worker panic does not propagate
/// out of `Schedule::run` today). A panic on the scene thread is re-raised with its payload.
fn under_watchdog<T, F>(what: &str, scene: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let handle = thread::Builder::new()
        .name("a5-soft-scene".to_string())
        .spawn(move || {
            let outcome = scene();
            done_tx.send(()).ok();
            outcome
        })
        .expect("harness: the scene thread must spawn");
    if let Err(mpsc::RecvTimeoutError::Timeout) = done_rx.recv_timeout(SCENE_TIMEOUT) {
        panic!(
            "watchdog: the scene '{what}' did not finish within {SCENE_TIMEOUT:?}. A panic inside \
             a scheduler worker does not propagate out of `Schedule::run` today, it blocks the \
             caller, so this is what the soft apply's row-count `debug_assert!` looks like from \
             the outside; the worker's own message is printed in this test's captured output, \
             above this line."
        );
    }
    match handle.join() {
        Ok(outcome) => outcome,
        Err(payload) => panic::resume_unwind(payload),
    }
}

/// Views a `#[repr(C)]` POD value as its bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, initialised `#[repr(C)]` `T` borrowed for the returned
    // slice's lifetime; the slice covers exactly its `size_of::<T>()` bytes read-only, the
    // layout the component pool stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// What the carrier probe visited, in walk order.
#[derive(Resource, Default)]
struct CarrierLog {
    walked: Vec<EntityId>,
}

/// Walks what the PRE-FIX `physics_soft_rigid_apply` walked: every `RigidBody` carrier, in
/// ascending archetype order. Not a shipped walk — it only pins where the stray sits. The
/// `Mut` guard is never dereferenced mutably, so no change tick moves.
fn probe_carrier(mut query: Query<Mut<RigidBody>>, mut log: ResMut<CarrierLog>) {
    for (entity, _body) in query.iter_entities_mut() {
        log.walked.push(entity);
    }
}

/// Runs [`probe_carrier`] once in a one-off schedule and returns the carrier walk.
fn carrier_walk(world: &mut EcsMaster) -> Vec<EntityId> {
    world.insert_resource(CarrierLog::default());
    let mut builder = ScheduleBuilder::new(serial_pool());
    builder.add_system(probe_carrier);
    let mut probe = builder.build(world);
    probe.run(world);
    world.resource::<CarrierLog>().walked.clone()
}

/// Bit-exact equality of every `RigidBody` field.
fn body_bits_eq(a: &RigidBody, b: &RigidBody) -> bool {
    let v = |p: Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    let q = |r: Quat| [r.x.to_bits(), r.y.to_bits(), r.z.to_bits(), r.w.to_bits()];
    v(a.position) == v(b.position)
        && v(a.linear_velocity) == v(b.linear_velocity)
        && q(a.rotation) == q(b.rotation)
        && v(a.angular_velocity) == v(b.angular_velocity)
}

/// What one run observed.
#[derive(Debug)]
struct CoupledRun {
    sphere_before: RigidBody,
    sphere_after: RigidBody,
    /// `(spawned, finished)` for the stray, when the run has one.
    stray: Option<(RigidBody, RigidBody)>,
}

/// Builds and steps the coupled scene, with or without the stray.
fn coupled_scene(with_stray: bool) -> CoupledRun {
    let mut world = EcsMaster::new();

    // The stray's archetype first, so an unfiltered walk visits it before the sphere.
    let stray = with_stray.then(|| {
        let arch =
            world.create_archetype(&[RigidBody::component_id(), RigidBodyMass::component_id()]);
        let body = RigidBody {
            position: Vec3::new(7.0, 5.0, 0.0),
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        };
        let mass = RigidBodyMass {
            inv_inertia: Mat3::ZERO,
            inv_mass: 0.0,
            restitution: 0.3,
            friction: 0.5,
        };
        let entity = world
            .create_entity(
                arch,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                ],
            )
            .expect("construction: the stray archetype accepts both columns");
        (entity, body)
    });

    // A light dynamic sphere at the origin: the one body-set member.
    let sphere_arch = world.bundle_archetype_id_for::<RigidBodyBundle>();
    let body = RigidBody {
        position: Vec3::ZERO,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass: 4.0,
        restitution: 0.3,
        friction: 0.5,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius: 0.6 },
        layer: 1,
        mask: 1,
    };
    let sphere: Entity = world
        .create_entity(
            sphere_arch,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("construction: the body archetype accepts the three columns");
    world.enable::<Simulated>(sphere);

    // One soft particle just above the sphere, moving down into it.
    let positions = [[0.0_f32, PARTICLE_Y, 0.0]];
    let inv_masses = [1.0_f32];
    let edges: [(u32, u32); 0] = [];
    let mut soft = SoftBody::from_mesh(&positions, &inv_masses, &edges, None, 0.0, 0.1)
        .expect("construction: a lone particle is a well-formed soft body");
    soft.vel_y[0] = PARTICLE_VEL_Y;
    let soft_arch = world.create_archetype(&[SoftBody::component_id()]);
    world
        .spawn_one(soft_arch, soft)
        .expect("construction: the {SoftBody} archetype accepts a SoftBody");

    // The premise the whole file rests on: an unfiltered write-back walk visits the stray at
    // index 0, so the pre-fix stage gave it row 0's reaction. Without this the file's whole
    // defect signal is unasserted - move the stray's archetype after the sphere's and it is
    // walked LAST, falls off the end of the one-row snapshot, and every assertion below is
    // green on unfixed code (round-3 review W1).
    let walked = carrier_walk(&mut world);
    let expected: Vec<EntityId> = stray
        .iter()
        .map(|&(entity, _)| entity.id())
        .chain(std::iter::once(sphere.id()))
        .collect();
    assert_eq!(
        walked,
        expected,
        "construction: an unfiltered `Query<Mut<RigidBody>>` must walk {}, so the stray receives \
         row 0's reaction before the fix; it walked {walked:?}",
        if with_stray {
            "the stray and then the sphere"
        } else {
            "the sphere alone"
        }
    );

    let mut builder = ScheduleBuilder::new(serial_pool());
    let _keys = add_physics_soft::<SoftStepSolver>(&mut builder, &mut world, true);
    world.insert_resource(SdfField::default());
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    let mut schedule = builder.build(&mut world);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;

    let read = |world: &EcsMaster, entity: Entity| -> RigidBody {
        *world
            .get_component::<RigidBody>(entity)
            .expect("harness: a tracked entity is live")
    };
    let sphere_before = read(&world, sphere);
    for _ in 0..STEPS {
        schedule.run(&mut world);
    }
    CoupledRun {
        sphere_before,
        sphere_after: read(&world, sphere),
        stray: stray.map(|(entity, spawned)| (spawned, read(&world, entity))),
    }
}

/// V3: the reaction reaches the sphere, and the stray walked before it is never written.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real coupled physics schedule on a boyko_threadpool; thread \
              spawning is intractable under Miri"
)]
fn the_soft_rigid_reaction_reaches_the_body_it_was_computed_for() {
    let control = under_watchdog("coupled control (no stray)", || coupled_scene(false));
    let scene = under_watchdog("coupled scene with a leading stray", || coupled_scene(true));

    let dv = control.sphere_after.linear_velocity - control.sphere_before.linear_velocity;
    assert!(
        dv != Vec3::ZERO,
        "precondition: in the control run the sphere's velocity must change, i.e. a nonzero soft \
         reaction reached a gathered row; without it this test compares two untouched worlds. \
         Δv = {dv:?}"
    );

    let (spawned, finished) = scene
        .stray
        .expect("construction: this scene spawns a stray entity");
    assert!(
        body_bits_eq(&spawned, &finished),
        "(a) the stray carries `RigidBody` outside the body set (no `Collider`, `inv_mass` 0, not \
         `Simulated`), so `physics_soft_rigid_apply` must not write it: spawned {spawned:?}, \
         finished {finished:?}; the sphere's reaction in the control was Δv = {dv:?}"
    );
    assert!(
        body_bits_eq(&control.sphere_after, &scene.sphere_after),
        "(b) the sphere must receive exactly the reaction it receives without the stray: \
         control {:?}, with the stray {:?}",
        control.sphere_after,
        scene.sphere_after
    );
}

/// V3b: the bit-exact comparison above is worth nothing unless the coupled scene is
/// reproducible.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real coupled physics schedule on a boyko_threadpool; thread \
              spawning is intractable under Miri"
)]
fn control_the_coupled_scene_is_reproducible() {
    let first = under_watchdog("coupled control run 1", || coupled_scene(false));
    let second = under_watchdog("coupled control run 2", || coupled_scene(false));
    assert!(
        body_bits_eq(&first.sphere_after, &second.sphere_after),
        "two runs of the coupled control must end bit-identical: {:?} vs {:?}",
        first.sphere_after,
        second.sphere_after
    );
}
