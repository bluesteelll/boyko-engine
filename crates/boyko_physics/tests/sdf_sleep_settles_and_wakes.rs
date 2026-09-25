//! L10 C1a (`docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md`, "Gates", C1a):
//! a box pile resting on an SDF field with sleeping on — the gap `IslandSleep::begin_step`
//! records under "Not covered" ("no gate covers SDF + sleeping").
//!
//! # Scene
//!
//! The SDF world (`add_physics_sdf::<DefaultRigidSolver>`, the default world's colored solve)
//! on a one-worker pool: an SDF floor (a box field whose top face is `y = 0`) and a tower of
//! three unit cubes spawned exactly touching it and each other. `sleeping` is set on
//! explicitly, so the gate does not depend on the shipped default (it is off on this tree);
//! the other sleep knobs are the defaults.
//!
//! # Checks
//!
//! 1. **The pile freezes**, on some step `K <= SETTLE_LIMIT` that is printed: every pile row is
//!    frozen (`IslandSleep::is_row_awake` false) after step `K`.
//! 2. **The rest pose is the sleeping-off pose**: at step `K + HOLD` every cube's position is
//!    within the box-pile ε ([`BOX_PILE_EPS`]) of the same cube's in a sleeping-off twin run for
//!    the same number of steps.
//! 3. **A field edit wakes the pile on the next step**: the floor is lowered by
//!    [`FLOOR_DROP`], and on the first step after the edit every pile row is awake. The bottom
//!    cube loses its field contact, so its island's manifold count changes, which is the
//!    wake-on-contact-change `begin_step` runs.
//!
//! RED-first (C1a's log): SDF manifolds filed under no island (the SDF-sentinel manifold
//! skipped by `ConstraintGraph::flatten_islands`) — the island's count no longer sees the field
//! contact, the edit changes no count, and check 3 is red.
//!
//! Spins a `boyko_threadpool` (intractable under Miri), so `cfg(not(miri))`. The scene runs on
//! its own thread under a watchdog: a panic inside a scheduler worker does not propagate out of
//! `Schedule::run` today, it blocks the caller.

#![cfg(not(miri))]

use std::panic;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
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
use boyko_physics::plugin::add_physics_sdf;
use boyko_physics::resources::{IslandSleep, PhysicsConfig};
use boyko_physics::sdf_query::SdfField;
use boyko_physics::solver::DefaultRigidSolver;

use boyko_sdf_math::{SdfEdit, sdf_op};

/// The fixed step.
const DT: f32 = 1.0 / 60.0;
/// Cubes in the tower.
const PILE: usize = 3;
/// A cube's half-extent.
const HALF: f32 = 0.5;
/// The floor field's half-extent (its top face is `y = 0`).
const FLOOR_HALF: f32 = 50.0;
/// Upper bound on the settle loop. Exceeding it is the gate's red for check 1.
const SETTLE_LIMIT: usize = 600;
/// Steps both worlds run past the freeze before their poses are compared.
const HOLD: usize = 60;
/// The box-pile ε: A7-R1's creep bound for a resting box pile (`CREEP_BOUND_M` in
/// `sleep_settles_box_piles.rs`), the same 1 cm as the colored solver's sleeping-on-versus-off
/// rest gate (`rest_state_with_sleeping_equals_without_to_epsilon`).
const BOX_PILE_EPS: f32 = 0.01;
/// How far the field edit lowers the floor: the bottom cube is left hanging, so its field
/// contact vanishes on the next narrowphase.
const FLOOR_DROP: f32 = 0.5;
/// Wall-clock liveness guard for one scene, far above an honest run; NOT a measurement.
const SCENE_TIMEOUT: Duration = Duration::from_secs(if cfg!(debug_assertions) { 300 } else { 120 });

/// Runs `scene` on its own thread and fails if it has not finished within [`SCENE_TIMEOUT`];
/// a panic on the scene thread is re-raised with its own payload.
fn under_watchdog<T, F>(what: &str, scene: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let handle = thread::Builder::new()
        .name("l10-sdf-scene".to_string())
        .spawn(move || {
            let outcome = scene();
            done_tx.send(()).ok();
            outcome
        })
        .expect("harness: the scene thread must spawn");
    if let Err(mpsc::RecvTimeoutError::Timeout) = done_rx.recv_timeout(SCENE_TIMEOUT) {
        panic!(
            "watchdog: the scene '{what}' did not finish within {SCENE_TIMEOUT:?}: a panic inside \
             a scheduler worker blocks `Schedule::run`, and its message is printed above"
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
    // slice's lifetime; the slice covers exactly its `size_of::<T>()` bytes, read-only, which
    // is the layout the component pool stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// The floor field with its top face at `y = top`.
fn floor(top: f32) -> SdfField {
    SdfField::from_edits(&[SdfEdit::box_shape(
        [0.0, top - FLOOR_HALF, 0.0],
        [FLOOR_HALF, FLOOR_HALF, FLOOR_HALF],
        sdf_op::UNION,
        0.0,
    )])
}

/// One unit cube resting at height `y` (its centre), at rest.
fn spawn_cube(world: &mut EcsMaster, y: f32) {
    let body = RigidBody {
        position: Vec3::new(0.0, y, 0.0),
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.0,
        friction: 0.5,
    };
    let collider = Collider {
        shape: ColliderShape::Box { half_extents: Vec3::new(HALF, HALF, HALF) },
        layer: 1,
        mask: 1,
    };
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
    world.enable::<Simulated>(e);
}

/// The SDF world with the tower spawned and `sleeping` set.
fn build(sleeping: bool) -> (EcsMaster, Schedule) {
    let mut world = EcsMaster::new();
    for i in 0..PILE {
        spawn_cube(&mut world, HALF + 2.0 * HALF * i as f32);
    }
    let mut builder = ScheduleBuilder::new(serial_pool());
    let _keys = add_physics_sdf::<DefaultRigidSolver>(&mut builder, &mut world);
    *world.resource_mut::<SdfField>() = floor(0.0);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    let schedule = builder.build(&mut world);
    world.resource_mut::<PhysicsConfig>().sleeping = sleeping;
    (world, schedule)
}

/// Every cube's position, in spawn (= row) order.
fn positions(world: &mut EcsMaster) -> Vec<Vec3> {
    let q = world.query::<&RigidBody, ()>();
    q.iter().map(|b| b.position).collect()
}

/// Pile rows frozen after the last step (rows are spawn order: one archetype, no static body).
fn frozen_rows(world: &EcsMaster) -> usize {
    let sleep = world.resource::<IslandSleep>();
    (0..PILE).filter(|&r| !sleep.is_row_awake(r)).count()
}

/// What the scene measured.
struct Outcome {
    /// The step after which every pile row was first frozen.
    freeze_step: Option<usize>,
    /// Positions at step `freeze_step + HOLD`, sleeping on.
    on: Vec<Vec3>,
    /// Positions at the same step, the sleeping-off twin.
    off: Vec<Vec3>,
    /// Pile rows awake on the first step after the field edit.
    awake_after_edit: usize,
    /// `IslandSleep::contact_wakes` across that step.
    contact_wakes: (u64, u64),
}

fn scene() -> Outcome {
    let (mut world, mut schedule) = build(true);
    let mut freeze_step = None;
    for step in 1..=SETTLE_LIMIT {
        schedule.run(&mut world);
        if frozen_rows(&world) == PILE {
            freeze_step = Some(step);
            break;
        }
    }
    let Some(k) = freeze_step else {
        return Outcome {
            freeze_step,
            on: Vec::new(),
            off: Vec::new(),
            awake_after_edit: 0,
            contact_wakes: (0, 0),
        };
    };
    for _ in 0..HOLD {
        schedule.run(&mut world);
    }
    let on = positions(&mut world);

    let (mut twin, mut twin_schedule) = build(false);
    for _ in 0..k + HOLD {
        twin_schedule.run(&mut twin);
    }
    let off = positions(&mut twin);

    let before = world.resource::<IslandSleep>().contact_wakes();
    *world.resource_mut::<SdfField>() = floor(-FLOOR_DROP);
    schedule.run(&mut world);
    let awake_after_edit = PILE - frozen_rows(&world);
    let after = world.resource::<IslandSleep>().contact_wakes();
    Outcome { freeze_step, on, off, awake_after_edit, contact_wakes: (before, after) }
}

#[test]
fn an_sdf_pile_freezes_rests_where_sleeping_off_rests_and_wakes_on_a_field_edit() {
    let o = under_watchdog("sdf pile", scene);
    let k = o.freeze_step.unwrap_or_else(|| {
        panic!(
            "check 1: the {PILE}-cube tower on the SDF floor did not freeze within {SETTLE_LIMIT} \
             steps with sleeping on"
        )
    });
    println!("SDF pile: every row frozen after step {k} (limit {SETTLE_LIMIT})");

    let mut worst = 0.0f32;
    for (i, (a, b)) in o.on.iter().zip(&o.off).enumerate() {
        let d = *a - *b;
        let dist = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
        println!(
            "SDF pile: cube {i} at step {}: on ({:.6}, {:.6}, {:.6}) off ({:.6}, {:.6}, {:.6}) |d| {dist:.3e} m",
            k + HOLD,
            a.x,
            a.y,
            a.z,
            b.x,
            b.y,
            b.z
        );
        worst = worst.max(dist);
    }
    assert_eq!(o.on.len(), PILE, "construction: every cube is read back");
    assert!(
        worst < BOX_PILE_EPS,
        "check 2: the frozen pile's rest pose is {worst} m from the sleeping-off twin's, past the \
         box-pile ε {BOX_PILE_EPS} m"
    );

    println!(
        "SDF pile: the floor lowered by {FLOOR_DROP} m; {} of {PILE} rows awake on the next step; \
         contact wakes {} -> {}",
        o.awake_after_edit, o.contact_wakes.0, o.contact_wakes.1
    );
    assert_eq!(
        o.awake_after_edit, PILE,
        "check 3: a field edit that takes the pile's support away must wake every pile row on the \
         first step after it"
    );
}
