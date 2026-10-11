//! Lane SL (ruling 25; design `D:/tmp/phys-orch/sl/design_r2.md` §3, cut `sl/cut.md` §3.2): a
//! sleeping body's velocity is zero.
//!
//! Before SL a frozen body kept the velocity its island was last solved with (measured up to
//! 2.4 mm/s on J-T, ruling 24's C3b anomaly): the freeze capture restored it every frozen step and
//! the write-back skipped the row, so the component held it for the whole sleep and a later wake
//! launched the body from it. Jolt, Rapier and Box2D zero the velocity inside the step that puts
//! the body to sleep. SL does the same: `end_step` marks the rows of an island that goes to sleep
//! `REST_SETTLED`, the settle zeroes them in the post-solve snapshot `physics_apply` writes back,
//! and the next `begin_step` freezes the island. The seal keeps that zero from waking its own
//! island: on the first step after a settle a manifold count change is re-keyed when nothing
//! outside the solver changed (`IslandSleep::begin_step`, "The settle and the seal").
//!
//! # Tests
//!
//! | id | test | asserts | kills |
//! |---|---|---|---|
//! | T-SL1 | `a_pile_that_falls_asleep_has_zero_velocity` | on every frozen step every box's component velocity is `0.0` and its position is the frozen one | M-SL1 (no settle call), M-SL7 (settle before the write-back) |
//! | T-SL7 | `a_latched_row_in_an_island_still_solved_keeps_its_velocity_and_latch` | rows that latched while their island still had an unlatched member stay latched, awake and solved, with the solver's velocity | M-SL2 (zero every latched row), M-SL4 (SL-1b: write test on `asleep` rows) |
//! | T-SL9 | `the_settle_is_bit_identical_across_worker_counts` | W in {1, 2, 4, 8}: identical poses, latches and counters through a freeze, a hold, a velocity write, a seal break and a re-freeze | a worker-count leak |
//! | T-SL10p | `a_pile_whose_first_attempt_flickers_freezes_on_it` (release) | J-T, gap 0.5, contact reuse off, box 581's spawn `z` one ulp down (`b581zm`): the pile freezes at its first attempt, no contact wake, at least one re-key, no seal break | M-SL9 (`sealed = false`), M-SL11 (stash before the zero) |
//!
//! T-SL1, T-SL7 run the real `add_physics_colored_solve` schedule with sleeping on in both
//! sleep-skip modes (`Off` and `Sets`) at one and eight workers.
//!
//! RED-first (C2's commit message): T-SL1 on the parent (SL-2a) reads a non-zero frozen velocity
//! on every arm; T-SL10p reads the trunk's rise.
//!
//! T-SL10p's seed is the cut's first fallback (C-7), not the design's `b38xp`: `b38xp` rises on the
//! parent exactly as the CS sweep recorded (step 146, 1240 contact wakes, frozen at 206), but under
//! SL its first attempt changes no count at all (the zero removed the flicker by itself), so the
//! seal is never offered a change and the test could not see M-SL9. `b581zm` rises on the parent
//! too (the sweep: step 146, 1240 wakes, frozen at 206) and under SL its first attempt's count
//! change is re-keyed (1240 rows). The 16-seed reading is in the lane's C2 record.
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
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{
    BroadphaseKind, BroadphaseSelectMode, IslandSleep, PhysicsConfig, SleepSkip, SolverScratch,
};

// ── Scene constants ──────────────────────────────────────────────────────────

/// The fixed step (60 Hz).
const DT: f32 = 1.0 / 60.0;
/// Every box's half-extent (Jolt's `cHalfBoxSize`).
const HALF: f32 = 1.0;
/// A unit-density cube of half-extent 1: m = 8.
const BOX_INV_MASS: f32 = 0.125;
/// `8 / 12 · (2² + 2²)` per axis, inverted.
const BOX_INV_INERTIA: f32 = 0.1875;
/// The static floor's half-extents; its top face is `y = 0`.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
/// The sleep-skip modes and worker counts every guard runs.
const ARMS: [(SleepSkip, usize); 4] =
    [(SleepSkip::Off, 1), (SleepSkip::Off, 8), (SleepSkip::Sets, 1), (SleepSkip::Sets, 8)];
/// Upper bound on a settle loop; a liveness bound, not a measurement.
const SETTLE_LIMIT: usize = 400;
/// Steps a frozen pile is held and read.
const HOLD: usize = 30;
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
        .name("sl-scene".to_string())
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

/// Whether both velocities of `b` are `0.0` component-wise (`-0.0` reads as zero).
fn at_rest(b: &RigidBody) -> bool {
    [b.linear_velocity, b.angular_velocity]
        .iter()
        .all(|v| v.x == 0.0 && v.y == 0.0 && v.z == 0.0)
}

/// `|v|² + |ω|²` of `b`.
fn speed_sq(b: &RigidBody) -> f32 {
    b.linear_velocity.length_squared() + b.angular_velocity.length_squared()
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

/// FNV-1a 64 over `words`, continuing `h`.
fn fnv(h: u64, words: impl IntoIterator<Item = u64>) -> u64 {
    words.into_iter().fold(h, |h, w| {
        w.to_le_bytes()
            .iter()
            .fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
    })
}

/// The sleep counters, in a fixed order.
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

/// One world on the real colored schedule with sleeping on.
struct Harness {
    world: EcsMaster,
    physics: Schedule,
    /// `{RigidBody, RigidBodyMass, Collider, BodyId}`: every body lives here, so a spawn appends.
    plain: ArchetypeId,
    next_id: u32,
}

impl Harness {
    /// The colored schedule over `workers` workers, sleeping on in sleep-skip mode `mode`, the
    /// default sleep threshold and debounce, the tree broadphase, dt = [`DT`].
    fn new(mode: SleepSkip, workers: usize) -> Self {
        let mut world = EcsMaster::new();
        let plain = world.create_archetype(&[
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
            BodyId::component_id(),
        ]);
        let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(workers).build());
        let _keys = add_physics_colored_solve(&mut builder, &mut world);
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        let physics = builder.build(&mut world);
        {
            let cfg = world.resource_mut::<PhysicsConfig>();
            cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
            cfg.dt = DT;
            cfg.sleeping = true;
            cfg.sleep_skip = mode;
            cfg.broadphase_select = BroadphaseSelectMode::Manual;
            cfg.broadphase = BroadphaseKind::Tree;
        }
        Self { world, physics, plain, next_id: 0 }
    }

    /// Spawns a box at rest; `dynamic` boxes are unit-density half-extent-1 cubes enabled as
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

    /// The static floor (top face at `y = 0`).
    fn floor(&mut self) -> Entity {
        self.spawn(Vec3::new(0.0, -FLOOR_HALF_EXTENTS.y, 0.0), FLOOR_HALF_EXTENTS, false)
    }

    /// A column of `height` exactly touching boxes on the floor at `(x, z)`, bottom first.
    fn column(&mut self, x: f32, z: f32, height: usize) -> Vec<Entity> {
        (0..height)
            .map(|i| {
                let p = Vec3::new(x, HALF + 2.0 * HALF * i as f32, z);
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

    /// Whether every body of `bodies` was frozen (not awake) on the last step.
    fn frozen(&mut self, bodies: &[Entity]) -> bool {
        let rows: Vec<usize> = bodies.iter().map(|&e| self.row_of(e)).collect();
        let sleep = self.sleep();
        rows.iter().all(|&r| !sleep.is_row_awake(r))
    }

    /// One step's record for the worker-count comparison: every body's state in walk order, the
    /// latch fingerprint, the awake bits and the sleep counters, hashed.
    fn record(&mut self) -> u64 {
        let poses: Vec<[u32; 13]> = {
            let q = self.world.query::<(&RigidBody, &BodyId), ()>();
            q.iter().map(|(b, _)| body_bits(b)).collect()
        };
        let n = self.world.resource::<SolverScratch>().bodies_len();
        let sleep = self.sleep();
        let h = fnv(
            0xcbf2_9ce4_8422_2325,
            poses.iter().flat_map(|p| p.iter().map(|&w| u64::from(w))),
        );
        let h = fnv(h, [sleep.latch_fingerprint()]);
        let h = fnv(h, (0..n).map(|r| u64::from(sleep.is_row_awake(r))));
        fnv(h, counters(sleep))
    }
}

// ── T-SL1 ────────────────────────────────────────────────────────────────────

/// What T-SL1 read on one arm.
#[derive(Debug)]
struct ZeroReading {
    /// The first step after which the whole column was frozen.
    freeze_step: usize,
    /// Frozen steps read.
    frozen_steps: usize,
    /// The largest `|v|² + |ω|²` of a column box's component on a frozen step.
    frozen_speed_sq_max: f32,
    /// Frozen steps on which a box's component velocity was not zero.
    nonzero_steps: usize,
    /// Frozen steps on which a box's position differed from its first frozen position.
    moved_steps: usize,
}

fn zero_scene(mode: SleepSkip, workers: usize) -> ZeroReading {
    let mut h = Harness::new(mode, workers);
    h.floor();
    let column = h.column(0.0, 0.0, 4);
    let mut freeze_step = None;
    for step in 1..=SETTLE_LIMIT {
        h.step();
        if h.frozen(&column) {
            freeze_step = Some(step);
            break;
        }
    }
    let freeze_step = freeze_step
        .unwrap_or_else(|| panic!("T-SL1: the column did not freeze within {SETTLE_LIMIT} steps"));
    let frozen_at: Vec<Vec3> = column.iter().map(|&e| h.body(e).position).collect();
    let mut reading = ZeroReading {
        freeze_step,
        frozen_steps: 0,
        frozen_speed_sq_max: 0.0,
        nonzero_steps: 0,
        moved_steps: 0,
    };
    for k in 0..=HOLD {
        if k > 0 {
            h.step();
        }
        assert!(h.frozen(&column), "T-SL1: the column woke {k} steps into its hold");
        reading.frozen_steps += 1;
        let bodies: Vec<RigidBody> = column.iter().map(|&e| h.body(e)).collect();
        if !bodies.iter().all(at_rest) {
            reading.nonzero_steps += 1;
        }
        for b in &bodies {
            reading.frozen_speed_sq_max = reading.frozen_speed_sq_max.max(speed_sq(b));
        }
        let moved = bodies.iter().zip(&frozen_at).any(|(b, p)| {
            b.position.x.to_bits() != p.x.to_bits()
                || b.position.y.to_bits() != p.y.to_bits()
                || b.position.z.to_bits() != p.z.to_bits()
        });
        reading.moved_steps += usize::from(moved);
    }
    reading
}

/// T-SL1 (M-SL1, M-SL7): a resting 4-box column on the floor freezes, and on every frozen step
/// every box's component velocity is zero and its position is the one it froze at.
#[test]
fn a_pile_that_falls_asleep_has_zero_velocity() {
    for (mode, workers) in ARMS {
        let what = format!("T-SL1, {mode:?}, W{workers}");
        let r = under_watchdog(&what, move || zero_scene(mode, workers));
        println!("{what}: {r:?}");
        assert!(
            r.frozen_steps > HOLD,
            "{what}: anti-vacuity: the frozen hold after step {} was read",
            r.freeze_step
        );
        assert_eq!(
            (r.nonzero_steps, r.moved_steps),
            (0, 0),
            "{what}: (frozen steps with a non-zero component velocity, frozen steps with a moved \
             position) over {} frozen steps; the largest frozen |v|² + |ω|² was {:e}: a sleeping \
             body must keep a zero velocity",
            r.frozen_steps,
            r.frozen_speed_sq_max
        );
    }
}

// ── T-SL7 ────────────────────────────────────────────────────────────────────

/// The step after which the joiner B is spawned beside column A.
const JOIN_AFTER: usize = 20;

/// What T-SL7 read on one arm.
#[derive(Debug)]
struct LatchReading {
    /// The steps before the first settle on which A's rows were all latched while every one of
    /// them was awake: the C2 window.
    window: Vec<usize>,
    /// Steps of the window on which an A row's component velocity was bit-zero.
    zeroed_steps: usize,
    /// The first step on which `rows_put_to_sleep` rose, and by how much.
    settle: Option<(usize, u64)>,
    /// `write_wakes` after the last step.
    write_wakes: u64,
    /// The first step after which A and B were all frozen.
    freeze_step: Option<usize>,
}

fn latch_scene(mode: SleepSkip, workers: usize) -> LatchReading {
    let mut h = Harness::new(mode, workers);
    h.floor();
    let a = h.column(0.0, 0.0, 2);
    for _ in 0..JOIN_AFTER {
        h.step();
    }
    // B rests on the floor, touching A's bottom box face to face.
    let b = h.spawn(Vec3::new(2.0 * HALF, HALF, 0.0), Vec3::new(HALF, HALF, HALF), true);
    let mut all = a.clone();
    all.push(b);
    let mut reading = LatchReading {
        window: Vec::new(),
        zeroed_steps: 0,
        settle: None,
        write_wakes: 0,
        freeze_step: None,
    };
    for step in JOIN_AFTER + 1..=SETTLE_LIMIT {
        let put = h.sleep().rows_put_to_sleep();
        h.step();
        let rows: Vec<usize> = a.iter().map(|&e| h.row_of(e)).collect();
        let sleep = h.sleep();
        let latched = rows.iter().all(|&r| sleep.is_row_latched(r));
        let awake = rows.iter().all(|&r| sleep.is_row_awake(r));
        let now = sleep.rows_put_to_sleep();
        if reading.settle.is_none() && now > put {
            reading.settle = Some((step, now - put));
        }
        if reading.settle.is_none() && latched && awake {
            reading.window.push(step);
            let zeroed = a.iter().any(|&e| {
                let bits = body_bits(&h.body(e));
                bits[3..6].iter().chain(&bits[10..13]).all(|&w| w == 0)
            });
            reading.zeroed_steps += usize::from(zeroed);
        }
        if h.frozen(&all) {
            reading.freeze_step = Some(step);
            break;
        }
    }
    reading.write_wakes = h.sleep().write_wakes();
    reading
}

/// T-SL7 (M-SL2; M-SL4 with SL-1b): column A rests from step 0; joiner B is spawned at rest
/// beside it after [`JOIN_AFTER`] steps, so their island keeps A's debounce and A latches about
/// that many steps before B (cut W4b). Over that window A's rows stay latched, awake and solved
/// until the merged island goes to sleep: nothing zeroes them or wakes them, and their component
/// velocities are the solver's, never bit-zero.
#[test]
fn a_latched_row_in_an_island_still_solved_keeps_its_velocity_and_latch() {
    for (mode, workers) in ARMS {
        let what = format!("T-SL7, {mode:?}, W{workers}");
        let r = under_watchdog(&what, move || latch_scene(mode, workers));
        println!(
            "{what}: window {:?} ({} steps), zeroed steps {}, first settle (step, rows) {:?}, \
             write_wakes {}, freeze step {:?}",
            r.window.first().zip(r.window.last()),
            r.window.len(),
            r.zeroed_steps,
            r.settle,
            r.write_wakes,
            r.freeze_step
        );
        assert!(
            !r.window.is_empty(),
            "{what}: anti-vacuity: A never latched while its island was still solved, so the test \
             cannot see a latched row of an awake island"
        );
        let (first, last) = (r.window[0], r.window[r.window.len() - 1]);
        assert!(
            r.window.len() == last - first + 1 && r.settle.map(|s| s.0) == Some(last + 1),
            "{what}: A must stay latched and awake on every step from its latch to the step its \
             island goes to sleep (window {first}..={last}, {} steps; first settle {:?})",
            r.window.len(),
            r.settle
        );
        assert_eq!(
            (r.zeroed_steps, r.write_wakes, r.settle.map(|s| s.1)),
            (0, 0, Some(3)),
            "{what}: (window steps with an A row's velocity bit-zero, write_wakes, rows the first \
             settle put to sleep): A is solved, never zeroed or woken, until A and B settle together"
        );
        assert_eq!(r.freeze_step, r.settle.map(|s| s.0 + 1), "{what}: A and B freeze after their settle");
    }
}

// ── T-SL9 ────────────────────────────────────────────────────────────────────

/// The worker counts T-SL9 compares.
const WORKERS: [usize; 4] = [1, 2, 4, 8];

/// What T-SL9 recorded on one worker count.
#[derive(Debug, PartialEq, Eq)]
struct WorkerTrace {
    /// One record per step ([`Harness::record`]).
    records: Vec<u64>,
    /// The steps before which each event was applied: the velocity write, the far spawn.
    events: Vec<usize>,
    /// The counters after the last step.
    counters: [u64; 7],
}

fn worker_scene(workers: usize) -> WorkerTrace {
    let mut h = Harness::new(SleepSkip::Sets, workers);
    h.floor();
    let first = h.column(0.0, 0.0, 3);
    let second = h.column(8.0, 0.0, 2);
    let mut all = first.clone();
    all.extend(&second);
    let mut trace = WorkerTrace { records: Vec::new(), events: Vec::new(), counters: [0; 7] };
    // Phase 1: both columns freeze, then hold.
    let mut held = 0;
    let mut step = 0;
    while held < 10 {
        step += 1;
        assert!(step <= SETTLE_LIMIT, "T-SL9 W{workers}: no freeze within {SETTLE_LIMIT} steps");
        h.step();
        trace.records.push(h.record());
        if h.frozen(&all) {
            held += 1;
        }
    }
    // Phase 2: a velocity written into the first column's frozen top box.
    let top = first[2];
    h.world
        .get_component_mut::<RigidBody>(top)
        .expect("harness: the top box is live")
        .linear_velocity = Vec3::new(0.5, 0.0, 0.0);
    trace.events.push(step);
    // Phase 3: on the first step after the next settle, a far static box is spawned (a seal
    // break: the rows move), then everything re-freezes.
    let mut spawned = false;
    let mut refrozen = 0;
    while refrozen < 10 {
        step += 1;
        assert!(step <= 4 * SETTLE_LIMIT, "T-SL9 W{workers}: no re-freeze");
        let put = h.sleep().rows_put_to_sleep();
        h.step();
        trace.records.push(h.record());
        if !spawned && h.sleep().rows_put_to_sleep() > put {
            h.spawn(Vec3::new(-30.0, 5.0, 0.0), Vec3::new(HALF, HALF, HALF), false);
            spawned = true;
            trace.events.push(step);
        }
        if spawned && h.frozen(&all) {
            refrozen += 1;
        } else {
            refrozen = 0;
        }
    }
    trace.counters = counters(h.sleep());
    trace
}

/// T-SL9: the settle, the seal and the rest state are serial and read W-invariant inputs, so W in
/// {1, 2, 4, 8} is bit-identical through a freeze, a hold, a velocity write into a frozen body, a
/// seal break and a re-freeze.
#[test]
fn the_settle_is_bit_identical_across_worker_counts() {
    let traces: Vec<WorkerTrace> = WORKERS
        .iter()
        .map(|&w| under_watchdog(&format!("T-SL9 W{w}"), move || worker_scene(w)))
        .collect();
    let base = &traces[0];
    println!(
        "T-SL9: {} steps, events before steps {:?}, counters (contact_wakes, write_wakes, \
         rows_put_to_sleep, settles, settle_rekeys, seal_held, seal_breaks) {:?}",
        base.records.len(),
        base.events,
        base.counters
    );
    let [_, _, put, settles, _, _, breaks] = base.counters;
    assert!(
        put > 0 && settles >= 2 && breaks >= 1,
        "T-SL9: anti-vacuity: the run must settle at least twice and break a seal \
         (rows_put_to_sleep {put}, settles {settles}, seal_breaks {breaks})"
    );
    for (w, t) in WORKERS.iter().zip(&traces).skip(1) {
        let first = base.records.iter().zip(&t.records).position(|(a, b)| a != b);
        assert!(
            t == base,
            "T-SL9: W{w} differs from W1: first differing step {:?} (step count {} vs {}), events \
             {:?} vs {:?}, counters {:?} vs {:?}",
            first.map(|s| s + 1),
            t.records.len(),
            base.records.len(),
            t.events,
            base.events,
            t.counters,
            base.counters
        );
    }
}

// ── T-SL10p ──────────────────────────────────────────────────────────────────

/// Jolt's `cBoxSize`: the pitch between neighbouring boxes in a layer.
const BOX_SIZE: f32 = 2.0;
/// J-T's layer gap.
const JT_GAP: f32 = 0.5;
/// Jolt's pyramid height.
const PYRAMID_HEIGHT: i32 = 15;
/// Dynamic boxes in the pyramid.
const PYRAMID_BODIES: usize = 1240;
/// Jolt's `PyramidScene` friction.
const JT_FRICTION: f32 = 0.2;
/// Steps of the J-T row.
const JT_STEPS: usize = 500;
/// The perturbed box (`b581zm`, `cs/proto/sleep_sweep/PREREG.md`: layer 2, j 13, k 5, z = -5.0),
/// its `z` stepped one ulp DOWN in its f32 bits (the CS hook's `-1`; for a negative component the
/// magnitude shrinks).
const SEED_BOX: usize = 581;

/// Every box's spawn lattice point at layer gap `gap`, in spawn order: Jolt's placement loop, as
/// `v2_speculative_fidelity.rs`'s `lattice` builds it.
fn lattice(gap: f32) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(PYRAMID_BODIES);
    for i in 0..PYRAMID_HEIGHT {
        let lo = i / 2;
        let hi = PYRAMID_HEIGHT - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { HALF } else { 0.0 };
                out.push(Vec3::new(
                    -(PYRAMID_HEIGHT as f32) + BOX_SIZE * j as f32 + odd,
                    1.0 + (BOX_SIZE + gap) * i as f32,
                    -(PYRAMID_HEIGHT as f32) + BOX_SIZE * k as f32 + odd,
                ));
            }
        }
    }
    out
}

/// What T-SL10p read.
#[derive(Debug)]
struct JtReading {
    /// The steps (`step + 1`, the runner's `first_frozen_step` convention) on which
    /// `contact_wakes` rose.
    contact_wake_rises: Vec<usize>,
    /// `[step + 1, rows]` of every step on which `rows_put_to_sleep` rose.
    settle_steps: Vec<(usize, u64)>,
    /// The first step (`step + 1`) after which no dynamic body was awake.
    first_frozen_step: Option<usize>,
    /// Steps after `first_frozen_step` on which a dynamic body was awake.
    awake_after_freeze: usize,
    /// The sleep counters after the run.
    counters: [u64; 7],
}

fn jt_scene() -> JtReading {
    let lattice = lattice(JT_GAP);
    assert_eq!(lattice.len(), PYRAMID_BODIES, "construction: Jolt's placement loop");
    let mut world = EcsMaster::new();
    // The runner's spawn: the floor, then the pyramid, every body in the RigidBodyBundle
    // archetype; the floor is not enabled as Simulated.
    let floor = RigidBody {
        position: Vec3::new(0.0, -1.0, 0.0),
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let archetype = world.bundle_archetype_id_for::<boyko_physics::components::RigidBodyBundle>();
    let spawn = |world: &mut EcsMaster, body: RigidBody, mass: RigidBodyMass, half: Vec3| {
        let collider = Collider { shape: ColliderShape::Box { half_extents: half }, layer: 1, mask: 1 };
        world
            .create_entity(
                archetype,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                ],
            )
            .expect("construction: the RigidBodyBundle archetype accepts the three columns")
    };
    spawn(
        &mut world,
        floor,
        RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass: 0.0, restitution: 0.0, friction: JT_FRICTION },
        FLOOR_HALF_EXTENTS,
    );
    for (k, &p) in lattice.iter().enumerate() {
        let mut position = p;
        if k == SEED_BOX {
            let before = position.z;
            // The CS hook's semantics: the f32 bits step by -1.
            position.z = f32::from_bits(before.to_bits() - 1);
            println!(
                "T-SL10p: b581zm: box {k} z {before:e} ({:#010x}) -> {:e} ({:#010x})",
                before.to_bits(),
                position.z,
                position.z.to_bits()
            );
        }
        let body = RigidBody {
            position,
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        };
        let mass = RigidBodyMass {
            inv_inertia: Mat3::from_diagonal(Vec3::new(
                BOX_INV_INERTIA,
                BOX_INV_INERTIA,
                BOX_INV_INERTIA,
            )),
            inv_mass: BOX_INV_MASS,
            restitution: 0.0,
            friction: JT_FRICTION,
        };
        let e = spawn(&mut world, body, mass, Vec3::new(HALF, HALF, HALF));
        world.enable::<Simulated>(e);
    }
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
    let _keys = add_physics_colored_solve(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    {
        // The runner's `--cfg default --broadphase tree --sleeping on --contact-reuse off`.
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
        cfg.dt = DT;
        cfg.sleeping = true;
        cfg.contact_reuse = false;
        cfg.broadphase_select = BroadphaseSelectMode::Manual;
        cfg.broadphase = BroadphaseKind::Tree;
    }
    let mut schedule = builder.build(&mut world);
    let mut reading = JtReading {
        contact_wake_rises: Vec::new(),
        settle_steps: Vec::new(),
        first_frozen_step: None,
        awake_after_freeze: 0,
        counters: [0; 7],
    };
    for step in 0..JT_STEPS {
        let before = counters(world.resource::<IslandSleep>());
        schedule.run(&mut world);
        let sleep = world.resource::<IslandSleep>();
        let now = counters(sleep);
        if now[0] > before[0] {
            reading.contact_wake_rises.push(step + 1);
        }
        if now[2] > before[2] {
            reading.settle_steps.push((step + 1, now[2] - before[2]));
        }
        let awake = world
            .resource::<SolverScratch>()
            .bodies()
            .iter()
            .enumerate()
            .any(|(r, b)| b.inv_mass != 0.0 && sleep.is_row_awake(r));
        match (awake, reading.first_frozen_step) {
            (false, None) => reading.first_frozen_step = Some(step + 1),
            (true, Some(_)) => reading.awake_after_freeze += 1,
            _ => {}
        }
    }
    reading.counters = counters(world.resource::<IslandSleep>());
    reading
}

/// T-SL10p (M-SL9, M-SL11): J-T with contact reuse off and the `b581zm` seed, whose first latch
/// attempt flickers on the parent (a rise at step 146, 1240 contact wakes, frozen at 206;
/// `cs/proto/sleep_sweep/SWEEP.md`). Under the seal the attempt's count change is re-keyed: the
/// pile freezes on the step after its first settle and stays frozen, with no contact wake and no
/// seal break.
#[test]
#[cfg_attr(
    any(miri, debug_assertions),
    ignore = "slow: Jolt's 1240-box pyramid for 500 steps through the real schedule; release only, \
              intractable under Miri"
)]
fn a_pile_whose_first_attempt_flickers_freezes_on_it() {
    let r = under_watchdog("T-SL10p", jt_scene);
    println!(
        "T-SL10p (J-T, gap 0.5, reuse off, b581zm): contact_wake_rises {:?}, settle steps {:?}, \
         first frozen step {:?}, awake steps after it {}, counters (contact_wakes, write_wakes, \
         rows_put_to_sleep, settles, settle_rekeys, seal_held, seal_breaks) {:?}",
        r.contact_wake_rises,
        r.settle_steps,
        r.first_frozen_step,
        r.awake_after_freeze,
        r.counters
    );
    let [contact_wakes, _, _, _, settle_rekeys, _, seal_breaks] = r.counters;
    let first_settle = r.settle_steps.first().map(|s| s.0);
    assert!(
        first_settle.is_some() && r.first_frozen_step == first_settle.map(|s| s + 1),
        "T-SL10p: the pile must freeze on the step after its first settle (settle steps {:?}, \
         first frozen step {:?})",
        r.settle_steps,
        r.first_frozen_step
    );
    assert_eq!(
        (r.settle_steps.len(), r.awake_after_freeze, contact_wakes, seal_breaks),
        (1, 0, 0, 0),
        "T-SL10p: (settle events, awake steps after the freeze, contact_wakes, seal_breaks): the \
         first attempt holds"
    );
    assert!(
        settle_rekeys >= 1,
        "T-SL10p: anti-vacuity: the attempt's count changed and was re-keyed (settle_rekeys \
         {settle_rekeys}); without a re-key this seed no longer exercises the seal"
    );
}
