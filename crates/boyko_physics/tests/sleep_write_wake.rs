//! Lane SL-1b (ruling 25; the owner's answer to O-1, ruling 27: "a velocity or impulse written into
//! a sleeping body wakes its island"; design `D:/tmp/phys-orch/sl/design_r2.md` §3.5, cut
//! `sl/cut.md` §3.3): a velocity written into a sleeping body wakes it on the next step.
//!
//! Since SL-1 a resting row's velocity is exactly zero: the settle stores `+0.0` in every
//! component, and a frozen row is never solved. So a resting row whose gathered velocity has a
//! non-zero component was written from outside the solver, and `IslandSleep::begin_step` wakes it
//! before its island's count is compared ("The write-wake"). Before SL-1b such a row stayed frozen
//! on the write's step; a write above the sleep threshold woke it one step later through
//! `end_step`'s energy, and a write below the threshold never woke it at all.
//!
//! # Tests
//!
//! | id | test | asserts | red before SL-1b | kills |
//! |---|---|---|---|---|
//! | T-SL3 | `a_velocity_written_into_a_sleeping_body_is_solved_on_the_next_step` | a lone 4-box column, frozen; six variants, each writing ONE velocity component into its top box: 0.5 m/s along `x`, `y` or `z` (linear) or 0.5 rad/s about `x`, `y` or `z` (angular): on the next step every row of the column is awake, the top box moved, one write wake and no contact wake | frozen on the write's step | M-SL5 (ω ignored, angular), M-SL8 (the post-solve snapshot), no write test (and, in debug, the capture assert), a write test blind to any one of the six components (that component's variant alone, round-1 F1) |
//! | T-SL4 | `a_sub_threshold_write_wakes_a_sleeping_body` | a lone box, frozen; 5 mm/s written (`|v|² = 2.5e-5`, below the threshold): awake on the next step, one write wake | never woken | no write test |
//! | T-SL5 | `a_write_right_after_the_island_goes_to_sleep_is_kept` | a lone box; 0.5 m/s written right after the step that puts it to sleep: on the next step it is awake and solved from that velocity | frozen there, solved one step later | no write test |
//! | T-SL6 | `a_soft_body_reaction_wakes_a_sleeping_rigid_body` | a soft cube dropped on a frozen box (`add_physics_soft(.., true)`): the step after the first reaction lands on the frozen box, the box is awake | the recorded gap (`plugin.rs`): frozen with the reaction stored | no write test |
//! | T-SL6b | `a_rigid_body_under_a_resting_soft_load_never_freezes_with_a_velocity` | the same box under the soft cube for the whole load window: every frozen step enters the solve with zero velocity (SL-INV), a frozen step that receives a reaction is never followed by another frozen step, and the box settled and was write-woken at least once | the frozen box stores the reaction (SL-INV broken) | no write test |
//! | T-SL8 | `off_and_sets_agree_under_writes` | every scene above, `Off` against `Sets` at one and eight workers: poses, latch fingerprint, awake bits and the seven sleep counters equal on every step | — | a mode leak |
//!
//! T-SL3 to T-SL5 run the real `add_physics_colored_solve` schedule, T-SL6 and T-SL6b the coupled
//! soft pipeline (`add_physics_soft::<DefaultRigidSolver>(.., true)`), with sleeping on in both
//! sleep-skip modes at one and eight workers. No write changes a manifold count, so a wake can
//! only come from the write test: a count change would wake through wake-on-contact-change on the
//! trunk too (design §3.5, O1). The written body has no neighbour within reach of its write but
//! the box under it (T-SL3) or the floor, and that contact is a touching one, which a velocity
//! cannot remove: an approach only widens a point's speculative margin, and a separating write
//! leaves the fixed margin, which keeps a touch.
//!
//! The documented soft cycle (`IslandSleep::begin_step`, "The write-wake"): soft bodies never
//! sleep, so a rigid island under a resting soft load settles after the debounce, the next non-zero
//! reaction wakes it, and it is solved again.
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
use boyko_physics::plugin::{add_physics_colored_solve, add_physics_soft};
use boyko_physics::resources::{
    BroadphaseKind, BroadphaseSelectMode, IslandSleep, PhysicsConfig, SleepSkip, SolverScratch,
};
use boyko_physics::sdf_query::SdfField;
use boyko_physics::soft::SoftBody;
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
/// The sleep-skip modes and worker counts every scene runs.
const ARMS: [(SleepSkip, usize); 4] =
    [(SleepSkip::Off, 1), (SleepSkip::Off, 8), (SleepSkip::Sets, 1), (SleepSkip::Sets, 8)];
/// Upper bound on a settle loop; a liveness bound, not a measurement.
const SETTLE_LIMIT: usize = 400;
/// Frozen steps held before a write, so the write lands past the first step after the settle.
const HOLD: usize = 5;
/// Steps run after a write (T-SL8 compares them too).
const AFTER: usize = 20;
/// The speed of T-SL3's linear writes (one axis each) and of T-SL5's lateral write, m/s.
const KICK: f32 = 0.5;
/// The rate of T-SL3's angular writes (one axis each), rad/s.
const SPIN: f32 = 0.5;
/// T-SL4's lateral write, m/s: `|v|² = 2.5e-5`, below the default threshold `1e-4`.
const NUDGE: Vec3 = Vec3::new(0.005, 0.0, 0.0);
/// The soft scenes' SDF floor top: far below the rigid floor, so the soft cube lands only on the
/// rigid box.
const SOFT_FIELD_TOP: f32 = -5.0;
/// The soft cube's centre height at its spawn: its lowest particles 0.3 m above the box's top face.
const SOFT_DROP_Y: f32 = 2.0 * HALF + 0.6;
/// Steps T-SL6b reads the box under the soft load, from the soft cube's spawn.
const LOAD_STEPS: usize = 600;
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
        .name("sl-write-scene".to_string())
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

/// Whether both velocities are component-wise `== 0.0` (SL-INV; `-0.0` included).
fn at_rest(linear: Vec3, angular: Vec3) -> bool {
    [linear.x, linear.y, linear.z, angular.x, angular.y, angular.z].iter().all(|&c| c == 0.0)
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

/// A braced soft cube of half-extent 0.3 centred at `centre` (the S5 cube of
/// `sleep_skip_bit_identity.rs`): eight 1 kg particles, every pair of corners an edge.
fn soft_cube(centre: Vec3) -> SoftBody {
    let half = 0.3f32;
    let positions: Vec<[f32; 3]> = (0..8u32)
        .map(|i| {
            let s = |bit: u32| if i & bit != 0 { half } else { -half };
            [centre.x + s(1), centre.y + s(2), centre.z + s(4)]
        })
        .collect();
    let edges: Vec<(u32, u32)> = (0..8u32).flat_map(|a| ((a + 1)..8u32).map(move |b| (a, b))).collect();
    SoftBody::from_mesh(&positions, &[1.0; 8], &edges, None, 1.0e-6, 0.05)
        .expect("construction: a braced cube is a soft body")
}

/// Which schedule a scene runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pipeline {
    /// `add_physics_colored_solve`, the tree broadphase.
    Colored,
    /// `add_physics_soft::<DefaultRigidSolver>(.., true)` with the SDF floor at
    /// [`SOFT_FIELD_TOP`]; the default broadphase selection.
    SoftCoupled,
}

/// One world with sleeping on.
struct Harness {
    world: EcsMaster,
    physics: Schedule,
    /// `{RigidBody, RigidBodyMass, Collider, BodyId}`: every rigid body lives here.
    plain: ArchetypeId,
    next_id: u32,
}

impl Harness {
    /// `pipeline` over `workers` workers, sleeping on in sleep-skip mode `mode`, the default
    /// sleep threshold and debounce, dt = [`DT`].
    fn new(pipeline: Pipeline, mode: SleepSkip, workers: usize) -> Self {
        let mut world = EcsMaster::new();
        let plain = world.create_archetype(&[
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
            BodyId::component_id(),
        ]);
        let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(workers).build());
        match pipeline {
            Pipeline::Colored => {
                let _keys = add_physics_colored_solve(&mut builder, &mut world);
            }
            Pipeline::SoftCoupled => {
                let _keys = add_physics_soft::<DefaultRigidSolver>(&mut builder, &mut world, true);
                *world.resource_mut::<SdfField>() = sdf_floor(SOFT_FIELD_TOP);
            }
        }
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        let physics = builder.build(&mut world);
        {
            let cfg = world.resource_mut::<PhysicsConfig>();
            cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
            cfg.dt = DT;
            cfg.sleeping = true;
            cfg.sleep_skip = mode;
            match pipeline {
                Pipeline::Colored => {
                    cfg.broadphase_select = BroadphaseSelectMode::Manual;
                    cfg.broadphase = BroadphaseKind::Tree;
                }
                Pipeline::SoftCoupled => cfg.soft_body = true,
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

    /// A column of `height` exactly touching cubes on the floor at `x = 0`, bottom first.
    fn column(&mut self, height: usize) -> Vec<Entity> {
        (0..height)
            .map(|i| {
                let p = Vec3::new(0.0, HALF + 2.0 * HALF * i as f32, 0.0);
                self.spawn(p, Vec3::new(HALF, HALF, HALF), true)
            })
            .collect()
    }

    /// Spawns the soft cube centred at `centre`.
    fn spawn_soft(&mut self, centre: Vec3) {
        let arch = self.world.create_archetype(&[SoftBody::component_id()]);
        self.world
            .spawn_one(arch, soft_cube(centre))
            .expect("construction: the soft archetype accepts a soft body");
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

    /// Steps until every body of `bodies` is frozen, then [`HOLD`] more frozen steps; every step
    /// is recorded. Panics past [`SETTLE_LIMIT`].
    fn freeze(&mut self, bodies: &[Entity], what: &str, records: &mut Vec<StepRecord>) {
        let mut held = 0;
        for _ in 0..SETTLE_LIMIT {
            self.step();
            records.push(self.record());
            if self.all_frozen(bodies) {
                held += 1;
                if held > HOLD {
                    return;
                }
            } else {
                held = 0;
            }
        }
        panic!("{what}: construction: no freeze within {SETTLE_LIMIT} steps");
    }
}

/// One step of a scene, as T-SL8 compares it.
#[derive(Debug, PartialEq, Eq)]
struct StepRecord {
    poses: Vec<[u32; 13]>,
    fingerprint: u64,
    awake: Vec<bool>,
    counters: [u64; 7],
}

// ── The write scenes (T-SL3, T-SL4, T-SL5) ───────────────────────────────────

/// A world axis: the one velocity component a T-SL3 write sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    /// The vector `s` along this axis, zero in the other two components.
    fn times(self, s: f32) -> Vec3 {
        match self {
            Axis::X => Vec3::new(s, 0.0, 0.0),
            Axis::Y => Vec3::new(0.0, s, 0.0),
            Axis::Z => Vec3::new(0.0, 0.0, s),
        }
    }

    /// This axis's component of `v`.
    fn of(self, v: Vec3) -> f32 {
        match self {
            Axis::X => v.x,
            Axis::Y => v.y,
            Axis::Z => v.z,
        }
    }
}

/// What a write scene writes, and when.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Write {
    /// T-SL3, linear: [`KICK`] along one axis into the frozen top box of a 4-box column.
    Kick(Axis),
    /// T-SL3, angular: [`SPIN`] about one axis into the frozen top box of a 4-box column.
    Spin(Axis),
    /// T-SL4: [`NUDGE`] into a frozen lone box.
    Nudge,
    /// T-SL5: [`KICK`] along `+x` into a lone box right after the step that puts it to sleep.
    AfterSettle,
}

/// T-SL3's six writes: one per velocity component the write test reads, each alone.
const AXIS_WRITES: [Write; 6] = [
    Write::Kick(Axis::X),
    Write::Kick(Axis::Y),
    Write::Kick(Axis::Z),
    Write::Spin(Axis::X),
    Write::Spin(Axis::Y),
    Write::Spin(Axis::Z),
];

/// Every write scene, as T-SL8 compares them.
const WRITES: [Write; 8] = [
    AXIS_WRITES[0],
    AXIS_WRITES[1],
    AXIS_WRITES[2],
    AXIS_WRITES[3],
    AXIS_WRITES[4],
    AXIS_WRITES[5],
    Write::Nudge,
    Write::AfterSettle,
];

/// What one write scene observed.
#[derive(Debug)]
struct WriteOutcome {
    /// The written body before the write (its component) and after the next step.
    before: RigidBody,
    after: RigidBody,
    /// Whether every row of the written body's island was awake on the step after the write, and
    /// whether the written row was.
    island_awake: bool,
    written_awake: bool,
    /// The counters right before and right after the step after the write.
    counters_before: [u64; 7],
    counters_after: [u64; 7],
    /// T-SL5: the box was frozen on no step before the write (the write lands right after its
    /// settle, before its first frozen step).
    never_frozen_before: bool,
    /// Every step's record, for T-SL8.
    records: Vec<StepRecord>,
}

fn write_scene(write: Write, mode: SleepSkip, workers: usize) -> WriteOutcome {
    let what = format!("{write:?} {mode:?} W{workers}");
    let mut h = Harness::new(Pipeline::Colored, mode, workers);
    h.floor();
    let island = h.column(if matches!(write, Write::Kick(_) | Write::Spin(_)) { 4 } else { 1 });
    let top = *island.last().expect("construction: a column has a top box");
    let mut records = Vec::new();
    let mut never_frozen_before = true;
    if write == Write::AfterSettle {
        let mut settled = false;
        for _ in 0..SETTLE_LIMIT {
            let put = h.sleep().rows_put_to_sleep();
            h.step();
            records.push(h.record());
            never_frozen_before &= !h.all_frozen(&island);
            if h.sleep().rows_put_to_sleep() > put {
                settled = true;
                break;
            }
        }
        assert!(settled, "{what}: construction: the box never went to sleep within {SETTLE_LIMIT} steps");
    } else {
        h.freeze(&island, &what, &mut records);
    }
    let before = h.body(top);
    {
        let mut b = h.body_mut(top);
        match write {
            Write::Kick(axis) => b.linear_velocity = axis.times(KICK),
            Write::Spin(axis) => b.angular_velocity = axis.times(SPIN),
            Write::AfterSettle => b.linear_velocity = Axis::X.times(KICK),
            Write::Nudge => b.linear_velocity = NUDGE,
        }
    }
    let counters_before = counters(h.sleep());
    h.step();
    records.push(h.record());
    let after = h.body(top);
    let island_awake = h.all_awake(&island);
    let written_awake = h.all_awake(&[top]);
    let counters_after = counters(h.sleep());
    for _ in 0..AFTER {
        h.step();
        records.push(h.record());
    }
    WriteOutcome {
        before,
        after,
        island_awake,
        written_awake,
        counters_before,
        counters_after,
        never_frozen_before,
        records,
    }
}

/// The write and contact wake rises on the step after the write.
fn wake_rises(o: &WriteOutcome) -> (u64, u64) {
    (
        o.counters_after[1] - o.counters_before[1],
        o.counters_after[0] - o.counters_before[0],
    )
}

/// T-SL3: 0.5 m/s along one axis (linear) or 0.5 rad/s about one axis (angular), one velocity
/// component per variant, all six of them, written into the frozen top box of a lone 4-box column
/// is solved on the next step: the column is awake, the top box moved (its position along the
/// written axis, or its rotation), one write wake, no contact wake. Red before SL-1b: the column
/// stays frozen on that step and `end_step`'s energy wakes it one step later. Every cell runs
/// before the verdict, so a write test blind to one component reds exactly that component's
/// variant (round-1 F1).
#[test]
fn a_velocity_written_into_a_sleeping_body_is_solved_on_the_next_step() {
    let mut failed = Vec::new();
    for write in AXIS_WRITES {
        for (mode, workers) in ARMS {
            let what = format!("T-SL3 {write:?} {mode:?} W{workers}");
            let o = under_watchdog(&what, move || write_scene(write, mode, workers));
            let moved = match write {
                Write::Kick(axis) => {
                    axis.of(o.after.position).to_bits() != axis.of(o.before.position).to_bits()
                }
                _ => {
                    let (a, b) = (o.after.rotation, o.before.rotation);
                    [a.x, a.y, a.z, a.w].map(f32::to_bits) != [b.x, b.y, b.z, b.w].map(f32::to_bits)
                }
            };
            println!(
                "{what}: column awake {}, top moved {moved}, (write, contact) wake rises {:?}",
                o.island_awake,
                wake_rises(&o)
            );
            if !(o.island_awake && moved && wake_rises(&o) == (1, 0)) {
                failed.push(format!(
                    "{what}: column awake {}, top moved {moved}, (write, contact) wake rises {:?} \
                     (want (1, 0)); top before {:?}, after {:?}",
                    o.island_awake,
                    wake_rises(&o),
                    o.before,
                    o.after
                ));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "the write must wake the column and be solved on the next step; {} of {} cells failed:\n{}",
        failed.len(),
        AXIS_WRITES.len() * ARMS.len(),
        failed.join("\n")
    );
}

/// T-SL4: 5 mm/s written into a frozen lone box (`|v|² = 2.5e-5`, below the sleep threshold)
/// wakes it on the next step. Red before SL-1b: `end_step`'s energy never crosses the threshold,
/// so the box stays frozen with the written velocity stored.
#[test]
fn a_sub_threshold_write_wakes_a_sleeping_body() {
    let threshold = PhysicsConfig::default().sleep_threshold;
    assert!(
        NUDGE.dot(NUDGE) < threshold,
        "construction: the nudge is below the default sleep threshold {threshold}"
    );
    for (mode, workers) in ARMS {
        let what = format!("T-SL4 {mode:?} W{workers}");
        let o = under_watchdog(&what, move || write_scene(Write::Nudge, mode, workers));
        println!("{what}: awake {}, (write, contact) wake rises {:?}", o.written_awake, wake_rises(&o));
        assert!(
            o.written_awake && wake_rises(&o) == (1, 0),
            "{what}: a sub-threshold write must wake the box on the next step: awake {}, (write, \
             contact) wake rises {:?} (want (1, 0)); box after {:?}",
            o.written_awake,
            wake_rises(&o),
            o.after
        );
    }
}

/// T-SL5: 0.5 m/s written into a lone box right after the step that puts it to sleep (its settle
/// zeroed it; it has not been frozen yet) is kept: on the next step the box is awake and solved
/// from that velocity (it moved along `+x`, and friction slowed it below the written speed). Red
/// before SL-1b: the box freezes on that step and is solved one step later.
#[test]
fn a_write_right_after_the_island_goes_to_sleep_is_kept() {
    for (mode, workers) in ARMS {
        let what = format!("T-SL5 {mode:?} W{workers}");
        let o = under_watchdog(&what, move || write_scene(Write::AfterSettle, mode, workers));
        assert!(
            o.never_frozen_before && at_rest(o.before.linear_velocity, o.before.angular_velocity),
            "{what}: construction: the write lands right after the settle, on a zeroed box never \
             frozen before (frozen before {}, box {:?})",
            !o.never_frozen_before,
            o.before
        );
        let vx = o.after.linear_velocity.x;
        let solved = o.after.position.x > o.before.position.x && vx > 0.0 && vx < KICK;
        println!(
            "{what}: awake {}, solved {solved} (x {} -> {}, v.x {vx}), (write, contact) wake rises {:?}",
            o.written_awake,
            o.before.position.x,
            o.after.position.x,
            wake_rises(&o)
        );
        assert!(
            o.written_awake && solved && wake_rises(&o) == (1, 0),
            "{what}: the write right after the settle must be solved on the next step: awake {}, \
             solved {solved}, (write, contact) wake rises {:?} (want (1, 0)); box before {:?}, after \
             {:?}",
            o.written_awake,
            wake_rises(&o),
            o.before,
            o.after
        );
    }
}

// ── The soft scenes (T-SL6, T-SL6b) ──────────────────────────────────────────

/// One step of the soft scene, as T-SL6 and T-SL6b read it.
#[derive(Clone, Copy, Debug)]
struct LoadStep {
    /// The box's row was frozen on this step.
    frozen: bool,
    /// The box's velocity as it entered this step's solve (the solver's snapshot after the step:
    /// a frozen row's is restored to its gathered state).
    entered: (Vec3, Vec3),
    /// The box's component velocity after this step (after the soft reaction's post-apply write).
    after: (Vec3, Vec3),
    /// `write_wakes` after this step.
    write_wakes: u64,
    /// `rows_put_to_sleep` after this step.
    put_to_sleep: u64,
}

/// What the soft scene observed.
#[derive(Debug)]
struct SoftOutcome {
    /// Every step from the soft cube's spawn, [`LOAD_STEPS`] of them.
    load: Vec<LoadStep>,
    /// Every step's record, for T-SL8.
    records: Vec<StepRecord>,
}

/// A box frozen on the floor; then the soft cube spawned [`SOFT_DROP_Y`] above it, and
/// [`LOAD_STEPS`] steps read.
fn soft_scene(mode: SleepSkip, workers: usize) -> SoftOutcome {
    let what = format!("soft {mode:?} W{workers}");
    let mut h = Harness::new(Pipeline::SoftCoupled, mode, workers);
    h.floor();
    let island = h.column(1);
    let bx = island[0];
    let mut records = Vec::new();
    h.freeze(&island, &what, &mut records);
    h.spawn_soft(Vec3::new(0.0, SOFT_DROP_Y, 0.0));
    let row = h.row_of(bx);
    let mut load = Vec::with_capacity(LOAD_STEPS);
    for _ in 0..LOAD_STEPS {
        h.step();
        records.push(h.record());
        let snap = h.world.resource::<SolverScratch>().bodies()[row];
        let b = h.body(bx);
        let sleep = h.sleep();
        load.push(LoadStep {
            frozen: !sleep.is_row_awake(row),
            entered: (snap.linear_velocity, snap.angular_velocity),
            after: (b.linear_velocity, b.angular_velocity),
            write_wakes: sleep.write_wakes(),
            put_to_sleep: sleep.rows_put_to_sleep(),
        });
    }
    SoftOutcome { load, records }
}

/// The first step on which the frozen box received a reaction (its component velocity left zero
/// after a frozen step).
fn first_reaction(load: &[LoadStep]) -> Option<usize> {
    load.iter().position(|s| s.frozen && !at_rest(s.after.0, s.after.1))
}

/// T-SL6: a soft cube dropped on a frozen box (`add_physics_soft(.., true)`): the step after the
/// first reaction lands on the frozen box, the box is awake and a write wake rose. Red before
/// SL-1b (the gap `plugin.rs` recorded): the box stays frozen with the reaction stored.
#[test]
fn a_soft_body_reaction_wakes_a_sleeping_rigid_body() {
    for (mode, workers) in ARMS {
        let what = format!("T-SL6 {mode:?} W{workers}");
        let o = under_watchdog(&what, move || soft_scene(mode, workers));
        let r = first_reaction(&o.load)
            .unwrap_or_else(|| panic!("{what}: void: no reaction ever landed on the frozen box"));
        let next = o.load.get(r + 1).unwrap_or_else(|| panic!("{what}: void: the reaction landed on the last step"));
        println!(
            "{what}: first reaction after load step {r} ({:?}); next step frozen {}, write wakes {} -> {}",
            o.load[r].after,
            next.frozen,
            o.load[r].write_wakes,
            next.write_wakes
        );
        assert!(
            !next.frozen && next.write_wakes > o.load[r].write_wakes,
            "{what}: the soft reaction landed on the frozen box after load step {r} ({:?}); on the \
             next step the box must be awake (frozen {}) and write-woken (write wakes {} -> {})",
            o.load[r].after,
            next.frozen,
            o.load[r].write_wakes,
            next.write_wakes
        );
    }
}

/// T-SL6b: the box under the resting soft cube for the whole load window. Every frozen step enters
/// the solve with zero velocity (SL-INV), and a frozen step whose reaction left the box's
/// velocity non-zero is never followed by another frozen step (the documented cycle: the reaction
/// wakes the box on the next step). Anti-vacuity: the box settled at least once under the load and
/// a reaction write-woke it at least once. Red before SL-1b: the frozen box stores the reaction,
/// so it enters a frozen step with a velocity.
#[test]
fn a_rigid_body_under_a_resting_soft_load_never_freezes_with_a_velocity() {
    for (mode, workers) in ARMS {
        let what = format!("T-SL6b {mode:?} W{workers}");
        let o = under_watchdog(&what, move || soft_scene(mode, workers));
        let load = &o.load;
        let reaction = first_reaction(load).unwrap_or_else(|| panic!("{what}: void: no reaction ever landed"));
        let window = &load[reaction..];
        let settles = window.last().map_or(0, |s| s.put_to_sleep) - load[reaction].put_to_sleep;
        let writes = window.last().map_or(0, |s| s.write_wakes) - load[reaction].write_wakes;
        let frozen_steps = window.iter().filter(|s| s.frozen).count();
        println!(
            "{what}: window from load step {reaction}: {} steps, {frozen_steps} frozen, settles +{settles}, \
             write wakes +{writes}",
            window.len()
        );
        for (k, s) in load.iter().enumerate() {
            assert!(
                !s.frozen || at_rest(s.entered.0, s.entered.1),
                "{what}: SL-INV: the box entered frozen load step {k} with velocity {:?}",
                s.entered
            );
        }
        for (k, w) in load.windows(2).enumerate() {
            assert!(
                !(w[0].frozen && !at_rest(w[0].after.0, w[0].after.1) && w[1].frozen),
                "{what}: the cycle: a reaction {:?} landed on the box frozen at load step {k}, and it \
                 was frozen again on the next step",
                w[0].after
            );
        }
        assert!(
            settles >= 1 && writes >= 1,
            "{what}: void: under the load the box must settle and be write-woken at least once \
             (settles +{settles}, write wakes +{writes})"
        );
    }
}

// ── T-SL8 ────────────────────────────────────────────────────────────────────

/// The first step on which two record lists differ, or their lengths.
fn first_difference(a: &[StepRecord], b: &[StepRecord]) -> Option<(usize, String)> {
    if let Some(k) = a.iter().zip(b).position(|(x, y)| x != y) {
        return Some((k + 1, format!("Off {:?}\nSets {:?}", a[k], b[k])));
    }
    (a.len() != b.len()).then(|| (a.len().min(b.len()), format!("lengths {} vs {}", a.len(), b.len())))
}

/// T-SL8: under writes into sleeping bodies `Off` and `Sets` agree on every step — every body's
/// state, the latch fingerprint, every awake bit and the seven sleep counters — at one and eight
/// workers, in every scene above. A written held row fails `Sets`' resting test (R3), so its
/// island is restored before the narrowphase and the write test reads the same inputs in both
/// modes.
#[test]
fn off_and_sets_agree_under_writes() {
    for workers in [1, 8] {
        for write in WRITES {
            let what = format!("T-SL8 {write:?} W{workers}");
            let off = under_watchdog(&what, move || write_scene(write, SleepSkip::Off, workers));
            let sets = under_watchdog(&what, move || write_scene(write, SleepSkip::Sets, workers));
            let diff = first_difference(&off.records, &sets.records);
            assert!(diff.is_none(), "{what}: Off and Sets differ after step {:?}", diff);
            println!("{what}: {} steps equal", off.records.len());
        }
        let what = format!("T-SL8 soft W{workers}");
        let off = under_watchdog(&what, move || soft_scene(SleepSkip::Off, workers));
        let sets = under_watchdog(&what, move || soft_scene(SleepSkip::Sets, workers));
        let diff = first_difference(&off.records, &sets.records);
        assert!(diff.is_none(), "{what}: Off and Sets differ after step {:?}", diff);
        println!("{what}: {} steps equal", off.records.len());
    }
}
