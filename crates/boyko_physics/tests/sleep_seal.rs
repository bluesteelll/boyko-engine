//! Lane SL's seal (ruling 25; design `D:/tmp/phys-orch/sl/design_r2.md` §3.4, cut `sl/cut.md`
//! §3.0 W4a and §3.2): on the first step after a settle, a manifold count change is re-keyed only
//! when nothing outside the solver changed; every external change still wakes the island.
//!
//! The settle zeroes a sleeping island's velocities, which moves its velocity-dependent
//! speculative margins to their minimum, so the first comparison after it can see a count the
//! zero itself changed. That step is sealed iff the rows are the same entities in the same order
//! exactly one gather later, the narrowphase configuration and the SDF edits are the settle's, and
//! every watched row (each member of the settled island and each row that is no island member)
//! gathered bit-equal to the settle's stash (`IslandSleep::begin_step`, "The settle and the
//! seal"). Each test below makes ONE external change on that step and requires the trunk's wake;
//! each kills the mutation that drops the leg of the seal that change trips.
//!
//! # Tests
//!
//! | id | test | change on the first step after the settle | kills |
//! |---|---|---|---|
//! | T-SL11 | `a_support_despawned_on_the_first_step_after_a_settle_wakes_the_pile` | the column's bottom box despawned | (guard) |
//! | T-SL11s | `a_static_spawned_into_a_settled_pile_on_the_first_step_after_a_settle_wakes_it` | a static box spawned overlapping the top box, appended after every watched row | M-SL9a (rows-stable ignored) |
//! | T-SL12 | `a_static_support_moved_on_the_first_step_after_a_settle_wakes_the_pile` | the static floor lowered by 0.5 m | M-SL9b (non-members not stashed) |
//! | T-SL13 | `a_member_teleported_on_the_first_step_after_a_settle_wakes_its_island` | the top box lifted by 1 m | M-SL9c (members' poses ignored) |
//! | T-SL14 | `a_contact_config_change_on_the_first_step_after_a_settle_is_not_rekeyed` | `speculative_distance = 0`, which drops a 10 mm lateral speculative pair | M-SL9d (epoch ignored) |
//! | T-SL15 | `a_frozen_island_written_on_another_islands_first_post_settle_step_still_wakes` | island B, frozen long ago, has its top box lifted on island A's sealed step | M-SL10 (re-key any latched row) |
//! | T-SL16 | `an_sdf_edit_under_a_settled_box_on_the_first_step_after_a_settle_wakes_it` | the SDF floor lowered by 0.5 m | M-SL9e (SDF edits out of the epoch) |
//! | T-SL22 | `a_static_rotated_into_a_settled_pile_on_the_first_step_after_a_settle_wakes_it` | a static plank beside the top box turned 90° about `y`, into it | `seal_equal` without `rotation` |
//! | T-SL23 | `a_static_resized_into_a_settled_pile_on_the_first_step_after_a_settle_wakes_it` | a static box beside the top box grown along `x`, into it | `seal_equal` without `shape` |
//! | T-SL24 | `a_static_slid_along_z_into_a_settled_pile_on_the_first_step_after_a_settle_wakes_it` | a static box beside the top box moved along `-z`, into it | `seal_equal` blind to a vector's `z` |
//! | T-SL25 | `a_static_given_a_velocity_toward_a_settled_pile_on_the_first_step_after_a_settle_wakes_it` | a static box 50 mm from the top box given 6 m/s toward it: its speculative margin reaches the box | `seal_equal` without `linear_velocity` |
//! | T-SL26 | `a_static_given_a_spin_beside_a_settled_pile_on_the_first_step_after_a_settle_wakes_it` | the same box given 30 rad/s about `z`: its corners' speculative margin reaches the box | `seal_equal` without `angular_velocity` |
//! | T-SL27 | `a_support_turned_sensor_on_the_first_step_after_a_settle_wakes_the_pile` | the static floor, the last row, gains `Sensor`: its contact becomes an overlap report, and the walk order is kept | `seal_equal` without `is_sensor` |
//! | T-SL28 | `a_support_parked_on_the_sdf_floor_on_the_first_step_after_a_settle_wakes_the_pile` | the bottom box on the SDF floor loses `Simulated`: its field contact goes | `seal_equal` without `simulated` |
//! | T-SL29 | `a_support_made_immovable_on_the_first_step_after_a_settle_wakes_the_pile` | the bottom box's inverse mass set to 0: it leaves the island, whose count changes | `seal_equal` without `inv_mass` and `inv_inertia_local` (each alone is the other's derivative, `seal_equal`'s docs) |
//! | T-SL8a | `off_and_sets_agree_through_every_seal_event` | every scene above, `Off` against `Sets` at W1 and W8: equal on every step | a mode leak |
//!
//! T-SL22 to T-SL29 close round 1's F2: each pins, end to end, one field of `seal_equal` that can
//! change a manifold count, and each is red under that field's warning-free removal. The unit
//! T-SL21 (`resources_tests.rs`) pins every compared field and both derived exclusions directly;
//! the fields no scene can reach (`kinematic`, `restitution`, `friction`) are named in
//! `seal_equal`'s docs.
//!
//! Every scene runs the real colored schedule (`add_physics_colored_solve`, or `add_physics_sdf`
//! for T-SL16 and T-SL28) with sleeping on, in both sleep-skip modes at one and eight workers.
//!
//! Spins real thread pools (intractable under Miri), so `cfg(not(miri))`. Every scene runs on its
//! own thread under a watchdog: a panic inside a scheduler worker blocks `Schedule::run`.

#![cfg(not(miri))]

use std::panic;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_macros::Component;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyMass, Sensor, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::{add_physics_colored_solve, add_physics_sdf};
use boyko_physics::resources::{
    BroadphaseKind, BroadphaseSelectMode, IslandSleep, PhysicsConfig, SleepSkip, SolverScratch,
};
use boyko_physics::sdf_query::SdfField;
use boyko_physics::solver::DefaultRigidSolver;

use boyko_sdf_math::{SdfEdit, sdf_op};

// ── Scene constants ──────────────────────────────────────────────────────────

/// The fixed step (60 Hz).
const DT: f32 = 1.0 / 60.0;
/// Every box's half-extent.
const HALF: f32 = 1.0;
/// A unit-density cube of half-extent 1: m = 8.
const BOX_INV_MASS: f32 = 0.125;
/// `8 / 12 · (2² + 2²)` per axis, inverted.
const BOX_INV_INERTIA: f32 = 0.1875;
/// The static floor's half-extents; its top face is `y = 0`.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
/// The lateral gap of T-SL14's speculative pair: inside the default 20 mm speculative distance.
const SPECULATIVE_GAP: f32 = 0.01;
/// The half-extent of T-SL23 to T-SL26's static bystander cube.
const BYSTANDER_HALF: f32 = 0.25;
/// How deep T-SL22 to T-SL24's change pushes the bystander into the top box (T-SL11s's overlap).
const OVERLAP: f32 = 0.15;
/// The gap between T-SL25 and T-SL26's bystander and the top box: beyond the default 20 mm
/// speculative distance, so the pair has no manifold at rest, and inside the reach the written
/// velocity adds (`min(cap, approach · dt)`: 0.1 m for T-SL25, 0.125 m at T-SL26's leading
/// corners).
const REACH_GAP: f32 = 0.05;
/// T-SL25's write into the bystander: toward the top box, m/s.
const BYSTANDER_SPEED: Vec3 = Vec3::new(-6.0, 0.0, 0.0);
/// T-SL26's write into the bystander: about `z`, rad/s, so its corners on the `+y` side close on
/// the top box at 7.5 m/s.
const BYSTANDER_SPIN: Vec3 = Vec3::new(0.0, 0.0, 30.0);
/// The sleep-skip modes and worker counts every scene runs.
const ARMS: [(SleepSkip, usize); 4] =
    [(SleepSkip::Off, 1), (SleepSkip::Off, 8), (SleepSkip::Sets, 1), (SleepSkip::Sets, 8)];
/// Upper bound on the wait for the settle the event follows; a liveness bound.
const SETTLE_LIMIT: usize = 400;
/// Steps run after the event.
const AFTER: usize = 30;
/// A body that falls must drop more than this over [`AFTER`] steps (free fall covers about 1.2 m
/// in 0.5 s).
const MIN_DROP: f32 = 0.3;
/// Watchdog budget for one scene.
const SCENE_TIMEOUT: Duration = Duration::from_secs(if cfg!(debug_assertions) { 600 } else { 300 });

// ── Harness ──────────────────────────────────────────────────────────────────

/// Stable identity of a body, so assertions follow entities, not rows.
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct BodyId {
    id: u32,
}

/// Runs `scene` on its own thread and fails if it has not finished within [`SCENE_TIMEOUT`]; a
/// panic on the scene thread is re-raised with its own payload.
fn under_watchdog<T, F>(what: &str, scene: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let handle = thread::Builder::new()
        .name("sl-seal-scene".to_string())
        .spawn(move || {
            let outcome = scene();
            done_tx.send(()).ok();
            outcome
        })
        .expect("harness: the scene thread must spawn");
    if let Err(mpsc::RecvTimeoutError::Timeout) = done_rx.recv_timeout(SCENE_TIMEOUT) {
        panic!(
            "watchdog: the scene '{what}' did not finish within {SCENE_TIMEOUT:?}: a panic inside a \
             scheduler worker blocks `Schedule::run`, and its message is printed above"
        );
    }
    match handle.join() {
        Ok(outcome) => outcome,
        Err(payload) => panic::resume_unwind(payload),
    }
}

/// Views a `#[repr(C)]` POD value as its bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, initialised `#[repr(C)]` `T` borrowed for the returned slice's
    // lifetime; the slice covers exactly its `size_of::<T>()` bytes read-only, the layout the
    // component pool stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Every field of a `RigidBody`, as bits.
fn body_bits(b: &RigidBody) -> [u32; 13] {
    [
        b.position.x.to_bits(),
        b.position.y.to_bits(),
        b.position.z.to_bits(),
        b.linear_velocity.x.to_bits(),
        b.linear_velocity.y.to_bits(),
        b.linear_velocity.z.to_bits(),
        b.rotation.x.to_bits(),
        b.rotation.y.to_bits(),
        b.rotation.z.to_bits(),
        b.rotation.w.to_bits(),
        b.angular_velocity.x.to_bits(),
        b.angular_velocity.y.to_bits(),
        b.angular_velocity.z.to_bits(),
    ]
}

/// The sleep counters, in a fixed order: `contact_wakes`, `write_wakes`, `rows_put_to_sleep`,
/// `settles`, `settle_rekeys`, `seal_held`, `seal_breaks`.
fn counters(sleep: &IslandSleep) -> [u64; 7] {
    [
        sleep.contact_wakes(),
        sleep.write_wakes(),
        sleep.rows_put_to_sleep(),
        sleep.settles(),
        sleep.settle_rekeys(),
        sleep.seal_held(),
        sleep.seal_breaks(),
    ]
}

/// The SDF floor field with its top face at `y = top`.
fn sdf_floor(top: f32) -> SdfField {
    SdfField::from_edits(&[SdfEdit::box_shape(
        [0.0, top - FLOOR_HALF_EXTENTS.y, 0.0],
        [FLOOR_HALF_EXTENTS.x, FLOOR_HALF_EXTENTS.y, FLOOR_HALF_EXTENTS.z],
        sdf_op::UNION,
        0.0,
    )])
}

/// One world on the real colored schedule with sleeping on.
struct Harness {
    world: EcsMaster,
    physics: Schedule,
    /// `{RigidBody, RigidBodyMass, Collider, BodyId}`: every body lives here, so a spawn appends.
    plain: ArchetypeId,
    next_id: u32,
}

impl Harness {
    /// The colored schedule (the SDF pipeline with an SDF floor at `y = 0` when `sdf`) over
    /// `workers` workers, sleeping on in sleep-skip mode `mode`, the tree broadphase, dt = [`DT`].
    fn new(mode: SleepSkip, workers: usize, sdf: bool) -> Self {
        let mut world = EcsMaster::new();
        let plain = world.create_archetype(&[
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
            BodyId::component_id(),
        ]);
        let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(workers).build());
        if sdf {
            let _keys = add_physics_sdf::<DefaultRigidSolver>(&mut builder, &mut world);
            *world.resource_mut::<SdfField>() = sdf_floor(0.0);
        } else {
            let _keys = add_physics_colored_solve(&mut builder, &mut world);
        }
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        let physics = builder.build(&mut world);
        {
            let cfg = world.resource_mut::<PhysicsConfig>();
            cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
            cfg.dt = DT;
            cfg.sleeping = true;
            cfg.sleep_skip = mode;
            if !sdf {
                cfg.broadphase_select = BroadphaseSelectMode::Manual;
                cfg.broadphase = BroadphaseKind::Tree;
            }
        }
        Self { world, physics, plain, next_id: 0 }
    }

    /// Spawns a box at rest; a `dynamic` one is a unit-density half-extent-1 cube enabled as
    /// `Simulated`, a static one carries `half_extents` and zero inverse mass.
    fn spawn(&mut self, position: Vec3, half_extents: Vec3, dynamic: bool) -> Entity {
        let body = RigidBody {
            position,
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        };
        let mass = if dynamic {
            RigidBodyMass {
                inv_inertia: Mat3::from_diagonal(Vec3::new(
                    BOX_INV_INERTIA,
                    BOX_INV_INERTIA,
                    BOX_INV_INERTIA,
                )),
                inv_mass: BOX_INV_MASS,
                restitution: 0.0,
                friction: 0.5,
            }
        } else {
            RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass: 0.0, restitution: 0.0, friction: 0.5 }
        };
        let collider = Collider { shape: ColliderShape::Box { half_extents }, layer: 1, mask: 1 };
        let id = BodyId { id: self.next_id };
        self.next_id += 1;
        let e = self
            .world
            .create_entity(
                self.plain,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                    (BodyId::component_id(), as_bytes(&id)),
                ],
            )
            .expect("construction: the body archetype accepts every column");
        if dynamic {
            self.world.enable::<Simulated>(e);
        }
        e
    }

    /// The static floor box (top face at `y = 0`).
    fn floor(&mut self) -> Entity {
        self.spawn(Vec3::new(0.0, -FLOOR_HALF_EXTENTS.y, 0.0), FLOOR_HALF_EXTENTS, false)
    }

    /// A column of `height` exactly touching cubes on the floor at `(x, 0)`, bottom first.
    fn column(&mut self, x: f32, height: usize) -> Vec<Entity> {
        (0..height)
            .map(|i| {
                let p = Vec3::new(x, HALF + 2.0 * HALF * i as f32, 0.0);
                self.spawn(p, Vec3::new(HALF, HALF, HALF), true)
            })
            .collect()
    }

    fn step(&mut self) {
        self.physics.run(&mut self.world);
    }

    fn sleep(&self) -> &IslandSleep {
        self.world.resource::<IslandSleep>()
    }

    fn body(&self, e: Entity) -> RigidBody {
        *self.world.get_component::<RigidBody>(e).expect("harness: a tracked body is live")
    }

    fn body_mut(&mut self, e: Entity) -> impl std::ops::DerefMut<Target = RigidBody> + '_ {
        self.world.get_component_mut::<RigidBody>(e).expect("harness: a tracked body is live")
    }

    /// Body ids in the gather's walk order: row `i` of the solver is element `i`.
    fn walk_ids(&mut self) -> Vec<u32> {
        let q = self.world.query::<(&RigidBody, &RigidBodyMass, &Collider, &BodyId), ()>();
        q.iter().map(|(_, _, _, id)| id.id).collect()
    }

    /// The gather row of body `e`.
    fn row_of(&mut self, e: Entity) -> usize {
        let id = self
            .world
            .get_component::<BodyId>(e)
            .expect("harness: a tracked body carries an id")
            .id;
        self.walk_ids()
            .iter()
            .position(|&x| x == id)
            .unwrap_or_else(|| panic!("harness: body {id} is not in the gather walk"))
    }

    /// Whether every body of `bodies` was awake on the last step.
    fn all_awake(&mut self, bodies: &[Entity]) -> bool {
        let rows: Vec<usize> = bodies.iter().map(|&e| self.row_of(e)).collect();
        let sleep = self.sleep();
        rows.iter().all(|&r| sleep.is_row_awake(r))
    }

    /// Whether every body of `bodies` was frozen on the last step.
    fn all_frozen(&mut self, bodies: &[Entity]) -> bool {
        let rows: Vec<usize> = bodies.iter().map(|&e| self.row_of(e)).collect();
        let sleep = self.sleep();
        rows.iter().all(|&r| !sleep.is_row_awake(r))
    }

    /// One step's record for the mode comparison: every body's state in walk order, the latch
    /// fingerprint, the awake bits and the sleep counters.
    fn record(&mut self) -> StepRecord {
        let poses: Vec<[u32; 13]> = {
            let q = self.world.query::<(&RigidBody, &BodyId), ()>();
            q.iter().map(|(b, _)| body_bits(b)).collect()
        };
        let n = self.world.resource::<SolverScratch>().bodies_len();
        let sleep = self.sleep();
        StepRecord {
            poses,
            fingerprint: sleep.latch_fingerprint(),
            awake: (0..n).map(|r| sleep.is_row_awake(r)).collect(),
            counters: counters(sleep),
        }
    }
}

/// One step of a scene, as T-SL8a compares it.
#[derive(Debug, PartialEq, Eq)]
struct StepRecord {
    poses: Vec<[u32; 13]>,
    fingerprint: u64,
    awake: Vec<bool>,
    counters: [u64; 7],
}

// ── The scenes ───────────────────────────────────────────────────────────────

/// The external change a scene makes on the first step after its settle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    /// T-SL11: the column's bottom box despawned.
    DespawnSupport,
    /// T-SL11s: a static box spawned overlapping the top box, appended after every row.
    SpawnStatic,
    /// T-SL12: the static floor lowered by 0.5 m.
    LowerFloor,
    /// T-SL13: the top box lifted by 1 m.
    LiftTop,
    /// T-SL14: `speculative_distance` set to 0.
    SpeculativeOff,
    /// T-SL15: island B (frozen long ago) has its top box lifted by 1 m on island A's step.
    LiftOtherIsland,
    /// T-SL16: the SDF floor lowered by 0.5 m.
    LowerSdfFloor,
    /// T-SL22: a static plank beside the top box turned 90° about `y`, into it.
    RotateStatic,
    /// T-SL23: a static cube beside the top box grown along `x`, into it.
    GrowStatic,
    /// T-SL24: a static cube beside the top box moved along `-z`, into it.
    SlideStaticZ,
    /// T-SL25: a static cube [`REACH_GAP`] from the top box given [`BYSTANDER_SPEED`].
    SpeedStatic,
    /// T-SL26: the same cube given [`BYSTANDER_SPIN`].
    SpinStatic,
    /// T-SL27: the static floor, spawned last, gains [`Sensor`].
    SensorFloor,
    /// T-SL28: the bottom box on the SDF floor loses [`Simulated`].
    ParkSupport,
    /// T-SL29: the bottom box's inverse mass (and authored inertia) set to 0.
    DemoteSupport,
}

const CHANGES: [Change; 15] = [
    Change::DespawnSupport,
    Change::SpawnStatic,
    Change::LowerFloor,
    Change::LiftTop,
    Change::SpeculativeOff,
    Change::LiftOtherIsland,
    Change::LowerSdfFloor,
    Change::RotateStatic,
    Change::GrowStatic,
    Change::SlideStaticZ,
    Change::SpeedStatic,
    Change::SpinStatic,
    Change::SensorFloor,
    Change::ParkSupport,
    Change::DemoteSupport,
];

impl Change {
    /// Whether the scene runs the SDF pipeline on an SDF floor instead of a static floor box.
    fn sdf(self) -> bool {
        matches!(self, Change::LowerSdfFloor | Change::ParkSupport)
    }

    /// The static bystander a scene places beside the pile's top box (centre, half-extents),
    /// clear of every contact until the change.
    fn bystander(self) -> Option<(Vec3, Vec3)> {
        let cube = Vec3::new(BYSTANDER_HALF, BYSTANDER_HALF, BYSTANDER_HALF);
        // The top box spans `x, z` in `[-HALF, HALF]` and `y` in `[2 HALF, 4 HALF]`.
        let y = 3.0 * HALF;
        match self {
            // A plank whose long (`z`) half-extent lands along `x` after the turn, reaching
            // `OVERLAP` past the top box's `+x` face; before it, 1.3 m clear.
            Change::RotateStatic => {
                let plank = Vec3::new(0.05, BYSTANDER_HALF, 1.5);
                Some((Vec3::new(HALF + plank.z - OVERLAP, y, 0.0), plank))
            }
            // 0.75 m clear of the `+x` face.
            Change::GrowStatic => Some((Vec3::new(HALF + BYSTANDER_HALF + 0.75, y, 0.0), cube)),
            // 0.75 m clear of the `+z` face.
            Change::SlideStaticZ => Some((Vec3::new(0.0, y, HALF + BYSTANDER_HALF + 0.75), cube)),
            Change::SpeedStatic | Change::SpinStatic => {
                Some((Vec3::new(HALF + BYSTANDER_HALF + REACH_GAP, y, 0.0), cube))
            }
            _ => None,
        }
    }
}

/// What one scene observed.
#[derive(Debug)]
struct Outcome {
    /// The step after which the event's settle happened (the change is applied before the next).
    settle_step: usize,
    /// The counters right before and right after the first step after the settle.
    before: [u64; 7],
    after: [u64; 7],
    /// Whether every body the change concerns was awake on that step.
    woke: bool,
    /// Whether every body of the settled pile (A) was frozen on that step.
    pile_frozen: bool,
    /// How far the body the change leaves unsupported fell over the [`AFTER`] steps.
    drop: f32,
    /// T-SL11s's anti-vacuity: the row count rose by one and the last row is the spawned box.
    appended: Option<bool>,
    /// T-SL27's anti-vacuity: the floor's archetype move kept the gather's walk order, so only
    /// `is_sensor` can break the seal.
    order_kept: Option<bool>,
    /// Every step's record, for T-SL8a.
    records: Vec<StepRecord>,
}

fn scene(change: Change, mode: SleepSkip, workers: usize) -> Outcome {
    let sdf = change.sdf();
    let mut h = Harness::new(mode, workers, sdf);
    // T-SL27's floor is spawned after every other body: as the last row, its move into the
    // `Sensor` archetype (walked after this one) keeps the walk order.
    let floor_last = change == Change::SensorFloor;
    let floor = if sdf || floor_last { None } else { Some(h.floor()) };
    let mut records = Vec::new();
    // B, the other island of T-SL15, first: it freezes before A is spawned.
    let other = if change == Change::LiftOtherIsland { h.column(-10.0, 2) } else { Vec::new() };
    if change == Change::LiftOtherIsland {
        let mut frozen = false;
        for _ in 0..SETTLE_LIMIT {
            h.step();
            records.push(h.record());
            if h.all_frozen(&other) {
                frozen = true;
                break;
            }
        }
        assert!(frozen, "construction: island B freezes within {SETTLE_LIMIT} steps");
    }
    let pile = h.column(0.0, 2);
    let bystander = change.bystander().map(|(at, half)| h.spawn(at, half, false));
    let lone = if change == Change::SpeculativeOff {
        // A lone box beside the column's bottom box, 10 mm away: a speculative pair only.
        Some(h.spawn(
            Vec3::new(2.0 * HALF + SPECULATIVE_GAP, HALF, 0.0),
            Vec3::new(HALF, HALF, HALF),
            true,
        ))
    } else {
        None
    };
    let floor = if floor_last { Some(h.floor()) } else { floor };
    let mut watched = pile.clone();
    watched.extend(lone);
    // Wait for the pile's settle: the step on which `rows_put_to_sleep` rises while every pile
    // row is awake (B, already frozen, does not settle again).
    let mut settle_step = None;
    for step in 1..=SETTLE_LIMIT {
        let put = h.sleep().rows_put_to_sleep();
        h.step();
        records.push(h.record());
        if h.sleep().rows_put_to_sleep() > put && h.all_awake(&watched) {
            settle_step = Some(step);
            break;
        }
    }
    let settle_step =
        settle_step.unwrap_or_else(|| panic!("{change:?}: the pile did not settle within {SETTLE_LIMIT} steps"));
    if change == Change::LiftOtherIsland {
        assert!(h.all_frozen(&other), "construction: island B is frozen when A settles");
    }

    // The change, between the settle step and the next one.
    let (moved, spawned_at) = match change {
        Change::DespawnSupport => {
            assert!(h.world.delete_entity(pile[0]), "construction: the support is live");
            watched.retain(|&e| e != pile[0]);
            (pile[1], None)
        }
        Change::SpawnStatic => {
            // Overlapping the top box's +x face by 0.15 m.
            let at = Vec3::new(HALF + 0.1, 3.0 * HALF, 0.0);
            h.spawn(at, Vec3::new(0.25, 0.25, 0.25), false);
            (pile[1], Some(at))
        }
        Change::LowerFloor => {
            h.body_mut(floor.expect("construction: a box floor")).position.y -= 0.5;
            (pile[0], None)
        }
        Change::LiftTop | Change::LiftOtherIsland => {
            let top = if change == Change::LiftTop { pile[1] } else { other[1] };
            h.body_mut(top).position.y += 1.0;
            (top, None)
        }
        Change::SpeculativeOff => {
            h.world.resource_mut::<PhysicsConfig>().speculative_distance = 0.0;
            (pile[0], None)
        }
        Change::LowerSdfFloor => {
            *h.world.resource_mut::<SdfField>() = sdf_floor(-0.5);
            (pile[0], None)
        }
        Change::RotateStatic => {
            let s = std::f32::consts::FRAC_1_SQRT_2;
            h.body_mut(bystander.expect("construction: a plank")).rotation =
                Quat { x: 0.0, y: s, z: 0.0, w: s };
            (pile[1], None)
        }
        Change::GrowStatic => {
            let e = bystander.expect("construction: a bystander");
            // Its `-x` face lands `OVERLAP` inside the top box's `+x` face.
            let half_x = h.body(e).position.x - (HALF - OVERLAP);
            let half_extents = Vec3::new(half_x, BYSTANDER_HALF, BYSTANDER_HALF);
            let mut collider =
                h.world.get_component_mut::<Collider>(e).expect("harness: a tracked body is live");
            collider.shape = ColliderShape::Box { half_extents };
            (pile[1], None)
        }
        Change::SlideStaticZ => {
            h.body_mut(bystander.expect("construction: a bystander")).position.z -= 0.75 + OVERLAP;
            (pile[1], None)
        }
        Change::SpeedStatic => {
            let e = bystander.expect("construction: a bystander");
            h.body_mut(e).linear_velocity = BYSTANDER_SPEED;
            (pile[1], None)
        }
        Change::SpinStatic => {
            let e = bystander.expect("construction: a bystander");
            h.body_mut(e).angular_velocity = BYSTANDER_SPIN;
            (pile[1], None)
        }
        Change::SensorFloor => {
            let e = floor.expect("construction: a box floor");
            let sensor = Sensor::component_id();
            let outcome = h.world.add_component_by_id(e, sensor, &[]);
            assert!(
                h.world.has_component(e, sensor),
                "construction: the floor gains `Sensor` ({outcome:?})"
            );
            (pile[0], None)
        }
        Change::ParkSupport => {
            h.world.disable::<Simulated>(pile[0]);
            (pile[0], None)
        }
        Change::DemoteSupport => {
            let mut mass = h
                .world
                .get_component_mut::<RigidBodyMass>(pile[0])
                .expect("harness: a tracked body is live");
            mass.inv_mass = 0.0;
            mass.inv_inertia = Mat3::ZERO;
            (pile[0], None)
        }
    };
    let rows_before = h.world.resource::<SolverScratch>().bodies_len();
    let order_before = h.walk_ids();
    let before = counters(h.sleep());
    let y0 = h.body(moved).position.y;
    h.step();
    records.push(h.record());
    let after = counters(h.sleep());
    let appended = spawned_at.map(|at| {
        let bodies = h.world.resource::<SolverScratch>().bodies();
        let last = bodies.last().expect("construction: rows exist").position;
        bodies.len() == rows_before + 1
            && [last.x, last.y, last.z].map(f32::to_bits) == [at.x, at.y, at.z].map(f32::to_bits)
    });
    let order_kept = (change == Change::SensorFloor).then(|| h.walk_ids() == order_before);
    let concerned: Vec<Entity> =
        if change == Change::LiftOtherIsland { other.clone() } else { watched.clone() };
    let woke = h.all_awake(&concerned);
    let pile_frozen = h.all_frozen(&watched);
    for _ in 0..AFTER {
        h.step();
        records.push(h.record());
    }
    let drop = y0 - h.body(moved).position.y;
    Outcome { settle_step, before, after, woke, pile_frozen, drop, appended, order_kept, records }
}

/// Runs `change`'s scene on every arm and checks what every test of this file requires of it:
/// the change woke what it concerns by the trunk's rule — a contact wake, no re-key — and spent
/// the seal as a break.
fn trunk_wake_on_every_arm(change: Change, falls: bool) -> Vec<Outcome> {
    ARMS.iter()
        .map(|&(mode, workers)| {
            let what = format!("{change:?}, {mode:?}, W{workers}");
            let o = under_watchdog(&what, move || scene(change, mode, workers));
            let d = |i: usize| o.after[i] - o.before[i];
            println!(
                "{what}: settle at step {}; on the next step contact_wakes +{}, settle_rekeys +{}, \
                 seal_held +{}, seal_breaks +{}; woke {}; fell {} m over {AFTER} steps",
                o.settle_step,
                d(0),
                d(4),
                d(5),
                d(6),
                o.woke,
                o.drop
            );
            assert!(
                o.woke && d(0) > 0 && d(4) == 0,
                "{what}: the change must wake by the trunk's rule on the first step after the \
                 settle (woke {}, contact_wakes +{}, settle_rekeys +{})",
                o.woke,
                d(0),
                d(4)
            );
            assert!(
                d(6) == 1 && d(5) == 0,
                "{what}: the change must break the settle's seal (seal_breaks +{}, seal_held +{})",
                d(6),
                d(5)
            );
            if falls {
                assert!(
                    o.drop > MIN_DROP,
                    "{what}: the unsupported body must fall: it dropped {} m over {AFTER} steps",
                    o.drop
                );
            }
            o
        })
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// T-SL11: the column's support despawned on the first step after its settle: the top box wakes
/// on that step and falls.
#[test]
fn a_support_despawned_on_the_first_step_after_a_settle_wakes_the_pile() {
    trunk_wake_on_every_arm(Change::DespawnSupport, true);
}

/// T-SL11s (M-SL9a, cut W4a): a static box spawned overlapping the settled pile's top box on the
/// first step after the settle, into the same archetype, so it is appended after every watched
/// row: every stashed row still compares equal, and only the rows-stable test (the seal cursor's
/// `Identity`) can see the spawn. The new manifold changes the pile's count, which must wake it.
#[test]
fn a_static_spawned_into_a_settled_pile_on_the_first_step_after_a_settle_wakes_it() {
    for o in trunk_wake_on_every_arm(Change::SpawnStatic, false) {
        assert_eq!(
            o.appended,
            Some(true),
            "anti-vacuity: the spawned static must be the last row, one more than before"
        );
    }
}

/// T-SL12 (M-SL9b): the static floor lowered by 0.5 m on the first step after the pile's settle:
/// the floor is no island member but is watched, so the seal breaks, the pile wakes and falls.
#[test]
fn a_static_support_moved_on_the_first_step_after_a_settle_wakes_the_pile() {
    trunk_wake_on_every_arm(Change::LowerFloor, true);
}

/// T-SL13 (M-SL9c): the top box lifted by 1 m on the first step after the pile's settle: its
/// contacts vanish, the count changes, the seal (members' poses) breaks, and the box falls.
#[test]
fn a_member_teleported_on_the_first_step_after_a_settle_wakes_its_island() {
    trunk_wake_on_every_arm(Change::LiftTop, true);
}

/// T-SL14 (M-SL9d): `speculative_distance` set to 0 on the first step after the settle of a pile
/// that holds a 10 mm lateral speculative pair: the pair's manifold vanishes, the count changes,
/// and the epoch breaks the seal: a contact wake, no re-key.
#[test]
fn a_contact_config_change_on_the_first_step_after_a_settle_is_not_rekeyed() {
    trunk_wake_on_every_arm(Change::SpeculativeOff, false);
}

/// T-SL15 (M-SL10): island B froze long ago; island A settles at step N; on step N + 1 B's top
/// box is lifted by 1 m. B is no watched row, so A's seal holds on that step, but B's rows are
/// `REST_FROZEN`, so the trunk's rule applies to them: B wakes and its top box falls, while A
/// stays frozen.
#[test]
fn a_frozen_island_written_on_another_islands_first_post_settle_step_still_wakes() {
    for (mode, workers) in ARMS {
        let what = format!("T-SL15, {mode:?}, W{workers}");
        let o = under_watchdog(&what, move || scene(Change::LiftOtherIsland, mode, workers));
        let d = |i: usize| o.after[i] - o.before[i];
        println!(
            "{what}: A settled at step {}; on the next step contact_wakes +{}, settle_rekeys +{}, \
             seal_held +{}, seal_breaks +{}; B woke {}; A frozen {}; B's top box fell {} m",
            o.settle_step,
            d(0),
            d(4),
            d(5),
            d(6),
            o.woke,
            o.pile_frozen,
            o.drop
        );
        assert!(
            d(5) == 1 && d(6) == 0,
            "{what}: anti-vacuity: A's seal must hold on the step B is written (seal_held +{}, \
             seal_breaks +{}), or the re-key rule is never offered B's rows",
            d(5),
            d(6)
        );
        assert!(
            o.woke && d(0) > 0 && o.pile_frozen && o.drop > MIN_DROP,
            "{what}: B's rows are frozen rows, so its count change wakes it on a sealed step too \
             (B woke {}, contact_wakes +{}, A frozen {}, B's top box fell {} m)",
            o.woke,
            d(0),
            o.pile_frozen,
            o.drop
        );
    }
}

/// T-SL16 (M-SL9e): the SDF floor lowered by 0.5 m on the first step after the pile's settle:
/// the SDF edits are in the seal's epoch, so the seal breaks, the pile wakes and falls.
#[test]
fn an_sdf_edit_under_a_settled_box_on_the_first_step_after_a_settle_wakes_it() {
    trunk_wake_on_every_arm(Change::LowerSdfFloor, true);
}

/// T-SL22 (round-1 F2, `rotation`): a static plank beside the settled pile's top box, turned 90°
/// about `y` on the first step after the settle, so its long side reaches 0.15 m into the box. Its
/// position, velocities and shape are unchanged; only `rotation` tells the seal, and the new
/// manifold must wake the pile.
#[test]
fn a_static_rotated_into_a_settled_pile_on_the_first_step_after_a_settle_wakes_it() {
    trunk_wake_on_every_arm(Change::RotateStatic, false);
}

/// T-SL23 (round-1 F2, `shape`): a static cube beside the settled pile's top box, its half-extent
/// along `x` grown on the first step after the settle until it reaches 0.15 m into the box. Its
/// pose is unchanged; only `shape` tells the seal, and the new manifold must wake the pile.
#[test]
fn a_static_resized_into_a_settled_pile_on_the_first_step_after_a_settle_wakes_it() {
    trunk_wake_on_every_arm(Change::GrowStatic, false);
}

/// T-SL24 (round-1 F2, the `z` axis): a static cube beside the settled pile's top box, moved along
/// `-z` on the first step after the settle until it reaches 0.15 m into the box. Only its
/// position's `z` changed, so a seal blind to one axis of its vectors cannot see it; the new
/// manifold must wake the pile.
#[test]
fn a_static_slid_along_z_into_a_settled_pile_on_the_first_step_after_a_settle_wakes_it() {
    trunk_wake_on_every_arm(Change::SlideStaticZ, false);
}

/// T-SL25 (round-1 F2, a non-member's `linear_velocity`): a static cube 50 mm from the settled
/// pile's top box, beyond the 20 mm speculative distance, given 6 m/s toward the box on the first
/// step after the settle. A static is never integrated and the write test reads only resting
/// rows, but its velocity widens the pair's speculative margin to 0.12 m, so the pair gains a
/// manifold, and only the seal's `linear_velocity` sees the cause: the pile must wake.
#[test]
fn a_static_given_a_velocity_toward_a_settled_pile_on_the_first_step_after_a_settle_wakes_it() {
    trunk_wake_on_every_arm(Change::SpeedStatic, false);
}

/// T-SL26 (round-1 F2, a non-member's `angular_velocity`): the cube of T-SL25 given 30 rad/s about
/// `z` instead: its corners on the `+y` side close on the top box at 7.5 m/s, which widens their
/// speculative margin to 0.145 m, so the pair gains a manifold, and only the seal's
/// `angular_velocity` sees the cause: the pile must wake.
#[test]
fn a_static_given_a_spin_beside_a_settled_pile_on_the_first_step_after_a_settle_wakes_it() {
    trunk_wake_on_every_arm(Change::SpinStatic, false);
}

/// T-SL27 (round-1 F2, `is_sensor`): the static floor, spawned after every other body, gains
/// [`Sensor`] on the first step after the pile's settle. As the last row, its move into the
/// `Sensor` archetype keeps the gather's walk order (asserted), so the rows-stable test holds and
/// only the seal's `is_sensor` sees the change. The floor's contact becomes an overlap report: the
/// pile's count changes, the pile wakes and falls through the floor.
#[test]
fn a_support_turned_sensor_on_the_first_step_after_a_settle_wakes_the_pile() {
    for o in trunk_wake_on_every_arm(Change::SensorFloor, true) {
        assert_eq!(
            o.order_kept,
            Some(true),
            "anti-vacuity: the floor's archetype move must keep the walk order"
        );
    }
}

/// T-SL28 (round-1 F2, `simulated`): on the SDF floor, the pile's bottom box loses [`Simulated`]
/// on the first step after the settle. A parked body collides with no field, so its field contact
/// goes and the pile's count changes; it stays a member (non-zero inverse mass) and its velocity
/// is zero, so only the seal's `simulated` sees the change: the pile must wake (the recorded gap
/// of `IslandSleep::begin_step`'s "Not covered": a parked support loses its field contact).
#[test]
fn a_support_parked_on_the_sdf_floor_on_the_first_step_after_a_settle_wakes_the_pile() {
    trunk_wake_on_every_arm(Change::ParkSupport, false);
}

/// T-SL29 (round-1 F2, `inv_mass`): the pile's bottom box gets inverse mass 0 on the first step
/// after the settle: it leaves the island, so the top box's island loses the floor's manifold and
/// its count changes, and the seal sees the change through `inv_mass` and the `inv_inertia_local`
/// the gather derives from it. The top box must wake.
#[test]
fn a_support_made_immovable_on_the_first_step_after_a_settle_wakes_the_pile() {
    trunk_wake_on_every_arm(Change::DemoteSupport, false);
}

/// T-SL8a: the seal's inputs are mode-free, so `Off` and `Sets` agree on every step of every
/// scene above, at one and at eight workers: the poses, the latch fingerprint, the awake bits and
/// every sleep counter.
#[test]
fn off_and_sets_agree_through_every_seal_event() {
    for change in CHANGES {
        for workers in [1, 8] {
            let what = format!("T-SL8a, {change:?}, W{workers}");
            let off = under_watchdog(&what, move || scene(change, SleepSkip::Off, workers));
            let sets = under_watchdog(&what, move || scene(change, SleepSkip::Sets, workers));
            assert!(!off.records.is_empty(), "{what}: anti-vacuity: steps were recorded");
            let first = off.records.iter().zip(&sets.records).position(|(a, b)| a != b);
            assert!(
                first.is_none() && off.records.len() == sets.records.len(),
                "{what}: Off and Sets differ first at step {:?} ({} vs {} steps); counters there \
                 {:?} vs {:?}",
                first.map(|s| s + 1),
                off.records.len(),
                sets.records.len(),
                first.map(|s| off.records[s].counters),
                first.map(|s| sets.records[s].counters)
            );
        }
    }
}
