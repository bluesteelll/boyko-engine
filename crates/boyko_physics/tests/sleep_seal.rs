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
//! | T-SL8a | `off_and_sets_agree_through_every_seal_event` | every scene above, `Off` against `Sets` at W1 and W8: equal on every step | a mode leak |
//!
//! Every scene runs the real colored schedule (`add_physics_colored_solve`, or `add_physics_sdf`
//! for T-SL16) with sleeping on, in both sleep-skip modes at one and eight workers.
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

use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass, Simulated};
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
}

const CHANGES: [Change; 7] = [
    Change::DespawnSupport,
    Change::SpawnStatic,
    Change::LowerFloor,
    Change::LiftTop,
    Change::SpeculativeOff,
    Change::LiftOtherIsland,
    Change::LowerSdfFloor,
];

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
    /// Every step's record, for T-SL8a.
    records: Vec<StepRecord>,
}

fn scene(change: Change, mode: SleepSkip, workers: usize) -> Outcome {
    let sdf = change == Change::LowerSdfFloor;
    let mut h = Harness::new(mode, workers, sdf);
    let floor = if sdf { None } else { Some(h.floor()) };
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
    };
    let rows_before = h.world.resource::<SolverScratch>().bodies_len();
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
    let concerned: Vec<Entity> =
        if change == Change::LiftOtherIsland { other.clone() } else { watched.clone() };
    let woke = h.all_awake(&concerned);
    let pile_frozen = h.all_frozen(&watched);
    for _ in 0..AFTER {
        h.step();
        records.push(h.record());
    }
    let drop = y0 - h.body(moved).position.y;
    Outcome { settle_step, before, after, woke, pile_frozen, drop, appended, records }
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
