//! B1: a frozen island must keep its warm-start impulses, so that the step that wakes it
//! is warm-started like a step of a pile that never slept.
//!
//! # The defect, as read from the code
//!
//! A manifold whose island is frozen is never pushed into the colored solver's columns
//! (`ColoredSoftStepSolver::build_columns`, the O8 solve skip). Without a carry,
//! `store_and_swap` rebuilds the warm table from those columns alone, and
//! `WarmStartTable::rebuild` zeroes every slot first. So after the first frozen step the
//! island's converged impulses are gone, and the step that wakes it seeds every contact point
//! from zero: a loaded pile restarts cold. Box2D v3 keeps a sleeping island's contacts,
//! impulses included, in its sleeping solver set.
//!
//! What these tests require of the store is a CARRY. On every step a manifold is frozen, its
//! points' entries are read from the previous table under the rows that table was keyed by, and
//! re-inserted under this gather's rows into a table sized for the solved and the carried
//! points together. A miss drops the entry, as on the solved path: a pair whose rows cannot be
//! translated, or a point whose feature id differs from the last solved step's.
//!
//! # What is measured
//!
//! * Warm-start HITS per contact point on the wake step: [`WarmSeedStats::point_hits`] over
//!   [`WarmSeedStats::points`], the points whose read key found an entry in the previous
//!   step's table. (`WarmSeedStats::manifolds` counts pushed manifolds whether or not any of
//!   their lookups hit, so it cannot tell a warm wake from a cold one.)
//! * The carry itself on every frozen step: [`WarmSeedStats::carry_hits`] over
//!   [`WarmSeedStats::carry_points`], the frozen points whose entry was re-inserted.
//! * The pile's velocities and full state after the wake step, against the never-slept twin.
//!
//! # Tests
//!
//! | test | asserts | pre-fix reading (instrument only, msvc, debug = release) |
//! |---|---|---|
//! | `a_woken_pile_is_warm_started_at_every_point_like_its_never_slept_twin` | wake-step hits = points, as the twin's | 0 of 20 (twin: 20 of 20) |
//! | `a_woken_pile_does_not_sag_faster_than_its_never_slept_twin` | wake-step fall within the at-rest speed of the twin's | 4.5394e-2 m/s against 1e-2 |
//! | `a_frozen_pile_carries_every_warm_entry_on_every_frozen_step` | carry hits = frozen points on each of the 31 frozen steps | 0 of 20 |
//! | `a_woken_pile_steps_exactly_as_its_twin_stepped_from_the_frozen_state` | wake-step state = the twin's state after the sleeper's first frozen step, as bits | differs at box 0, field 0 |
//! | `a_frozen_island_whose_row_moves_while_held_carries_every_warm_entry` | the carry through a row move while frozen | 0 of 24 |
//! | `frozen_islands_carried_beside_a_parallel_solve_are_bit_identical_across_worker_counts` | W = 1 and W = 8 bit-identical on every step | anti-vacuity red: nothing carried |
//!
//! Each carry mutation was run against all six (2026-09-19, msvc, debug), and each is red in
//! at least one, on its B1 assertion with every control green:
//!
//! * the carry deleted: all six;
//! * the carry reading with the store key instead of the read key: the row-move test only, on
//!   the move step (20 of 24). With no row move the two keys are equal;
//! * every carried normal impulse scaled by 0.5: the velocity and exactness tests;
//! * scaled by 0.9999: the exactness test only;
//! * the table sized for the solved points alone: the determinism test, through the store's
//!   load `debug_assert!` (a debug-only red: in release the load stays below 1);
//! * scaled by 0.9999 only on a pool of more than one thread: the determinism test only.
//!
//! # The twin
//!
//! Two worlds, the same scene spawned in the same order, stepped in lockstep:
//!
//! * the **sleeper** runs with sleeping on and the default threshold and debounce; it
//!   settles, freezes, is held frozen for [`HOLD_STEPS`] steps, and is then woken with
//!   `IslandSleep::wake_all`;
//! * the **twin** runs with sleeping on and a threshold of `0`. A speed² is never below
//!   `0`, so its debounce never ticks and it never sleeps, but it takes the same
//!   sleeping-on solve path as the sleeper.
//!
//! Before the sleeper's first frozen step nothing distinguishes the two worlds, and the
//! scenario asserts that they are bit-identical on every one of those steps. So the twin
//! is exactly what the sleeper would have been had it never slept, and on the wake step
//! it is the control: it must show `point_hits == points`, which proves the assertion
//! under test can pass on this scene at this step.
//!
//! The scenario also checks that a zero hit count cannot come from anything but the lost
//! impulses: neither warm cursor records a `Reset`, and every pushed manifold's lookup
//! resolved to rows of the previous gather (`translated == manifolds`).
//!
//! # The velocity check
//!
//! A pile woken cold starts its wake step from zero contact impulse, so gravity pulls it
//! down before the soft contacts catch it: it sags. The velocity test compares the pile's
//! largest downward box velocity on the wake step with the twin's on the same step, with a
//! tolerance of [`at_rest_speed`], the speed below which the engine itself calls a body at
//! rest (`sqrt(DEFAULT_SLEEP_THRESHOLD)`, the threshold being on speed²). The pile was at
//! rest by that criterion when it froze, so a warm wake must not make it fall faster than
//! its twin by more than that.
//!
//! Its control is the twin's own freeze step. The sleeper froze in exactly the state the
//! twin held one step before the sleeper's first frozen step, and the twin stepped that
//! state warm-started. So the twin's velocities after that step are a warm wake of the
//! sleeper's frozen state, and they must pass the same bound against the twin's wake-step
//! velocities, which proves the bound admits a warm wake. The exactness test asks for more:
//! that the sleeper's wake step IS that step, bit for bit.
//!
//! # Scenes
//!
//! All three run through the real `add_physics_colored_solve` schedule, dt = 1/60, gravity
//! (0, -9.81, 0), sleeping on, on a static floor box with its top face at `y = 0`. Nothing
//! else in the configuration is set, except the parallel dispatch in the third.
//!
//! * **The column** (the twin tests and the carry test): [`COLUMN_HEIGHT`] unit-half-extent
//!   boxes exactly touching (gap 0), on a serial pool. A column rather than a pyramid: a
//!   pyramid's knife-edge lateral pairs make and break contact at rest, and a manifold that
//!   appears on the wake step has no warm entry to find in EITHER world, which would make the
//!   twin's `point_hits == points` fail for a reason that has nothing to do with sleep. The
//!   column's contacts are all load-bearing face contacts, and the lowest carries the whole
//!   column's weight.
//! * **The row move**: the column, a lone box on the floor, and a static box far above the
//!   floor that touches nothing, on a serial pool. Deleting the static box while everything
//!   is frozen swap-moves the lone box from the last row into row 1 (see [`run_row_move`]).
//! * **The crowd** (determinism): twelve columns of heights 1 to 3, which freeze on different
//!   steps, then [`CROWD_SINGLES`] single boxes spawned beside them once they are frozen, on a
//!   1-worker and an 8-worker pool with `parallel_solve` on. Stepped in lockstep and compared
//!   after every step: every box's full state and the warm-seed statistics.
//!
//! In the crowd some frozen points miss on the step their island freezes. The columns are
//! still settling there, a point's feature id can differ from the last solved step's, and the
//! never-slept counterpart of that step misses the same key. The crowd therefore reports those
//! misses, and asserts instead that the wake step reads exactly what the last held step
//! carried.
//!
//! Device-free. The schedule spins up a `boyko_threadpool`, which is intractable under
//! Miri. Every scenario runs on its own thread under a watchdog: a panic inside a scheduler
//! worker does not propagate out of `Schedule::run` today, it blocks the caller.

#![cfg(not(miri))]

use std::panic;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{
    ConstraintGraph, DEFAULT_SLEEP_THRESHOLD, IslandSleep, Manifolds, PhysicsConfig, SolverScratch,
};
use boyko_physics::solver::{ColoredSoftStepSolver, WarmSeedStats};

// ── Scene constants ──────────────────────────────────────────────────────────

/// The fixed step (60 Hz).
const DT: f32 = 1.0 / 60.0;
/// The column's box half-extent.
const HALF_BOX: f32 = 1.0;
/// Boxes in the column.
const COLUMN_HEIGHT: usize = 5;
/// Steps the sleeper gets to freeze in. A liveness bound, not a measurement.
const SETTLE_LIMIT: usize = 1500;
/// Steps the sleeper is held frozen before the wake: enough that every warm entry the
/// island had before it froze has been through many table rebuilds.
const HOLD_STEPS: usize = 30;
/// Watchdog budget for one scenario.
const SCENARIO_TIMEOUT: Duration =
    Duration::from_secs(if cfg!(debug_assertions) { 300 } else { 120 });

// ── Harness ──────────────────────────────────────────────────────────────────

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

/// The speed below which the engine calls a body at rest: the default sleep threshold is on
/// speed² (`|v|² + |ω|²`), so this is its square root, 0.01 m/s.
fn at_rest_speed() -> f32 {
    DEFAULT_SLEEP_THRESHOLD.sqrt()
}

/// The largest downward velocity (`-v.y`) over `velocities`. Negative when every box moves
/// up.
fn max_downward(velocities: &[Vec3]) -> f32 {
    velocities
        .iter()
        .map(|v| -v.y)
        .fold(f32::NEG_INFINITY, f32::max)
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

/// Spawns one box body at rest into the `RigidBodyBundle` archetype; a dynamic one
/// (`inv_mass != 0`) is enabled as `Simulated`.
fn spawn_box(
    world: &mut EcsMaster,
    position: Vec3,
    half_extents: Vec3,
    inv_mass: f32,
    inv_inertia: Mat3,
) -> Entity {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia,
        inv_mass,
        restitution: 0.0,
        friction: 0.5,
    };
    let collider = Collider {
        shape: ColliderShape::Box { half_extents },
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
    if inv_mass != 0.0 {
        world.enable::<Simulated>(e);
    }
    e
}

/// A unit-half-extent dynamic box: `inv_mass` 0.125, inverse inertia 0.1875 on the diagonal
/// (the column's boxes, and the determinism crowd's).
fn spawn_unit_box(world: &mut EcsMaster, centre: Vec3) -> Entity {
    spawn_box(
        world,
        centre,
        Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
        0.125,
        Mat3::from_diagonal(Vec3::new(0.1875, 0.1875, 0.1875)),
    )
}

/// One world: the column on its floor, through the real colored schedule.
struct Pile {
    world: EcsMaster,
    physics: Schedule,
    /// The column's boxes, bottom first.
    boxes: Vec<Entity>,
}

/// A world on the real `add_physics_colored_solve` schedule over `pool`, with sleeping on and
/// the given threshold, dt = [`DT`] and gravity (0, -9.81, 0); `parallel_solve` opts the
/// colored solve's parallel dispatch in. Nothing else in the configuration is touched.
fn colored_world(
    pool: Arc<ThreadPool>,
    sleep_threshold: f32,
    parallel_solve: bool,
) -> (EcsMaster, Schedule) {
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(pool);
    let _keys = add_physics_colored_solve(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    let physics = builder.build(&mut world);
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.sleeping = true;
        cfg.sleep_threshold = sleep_threshold;
        if parallel_solve {
            cfg.parallel_solve = true;
        }
        cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
        cfg.dt = DT;
    }
    (world, physics)
}

impl Pile {
    /// Builds the scene with sleeping on and the given sleep threshold.
    fn new(sleep_threshold: f32) -> Self {
        let (world, physics) = colored_world(serial_pool(), sleep_threshold, false);
        let mut pile = Self {
            world,
            physics,
            boxes: Vec::with_capacity(COLUMN_HEIGHT),
        };
        pile.spawn(
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(50.0, 1.0, 50.0),
            0.0,
            Mat3::ZERO,
        );
        for i in 0..COLUMN_HEIGHT {
            let centre = Vec3::new(0.0, HALF_BOX + 2.0 * HALF_BOX * i as f32, 0.0);
            let e = pile.spawn(
                centre,
                Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
                0.125,
                Mat3::from_diagonal(Vec3::new(0.1875, 0.1875, 0.1875)),
            );
            pile.boxes.push(e);
        }
        pile
    }

    fn spawn(
        &mut self,
        position: Vec3,
        half_extents: Vec3,
        inv_mass: f32,
        inv_inertia: Mat3,
    ) -> Entity {
        spawn_box(
            &mut self.world,
            position,
            half_extents,
            inv_mass,
            inv_inertia,
        )
    }

    fn step(&mut self) {
        self.physics.run(&mut self.world);
    }

    /// `(dynamic rows, of which awake)` on the last step.
    fn awake_rows(&self) -> (usize, usize) {
        let sleep = self.world.resource::<IslandSleep>();
        let mut dynamic = 0;
        let mut awake = 0;
        for (row, b) in self
            .world
            .resource::<SolverScratch>()
            .bodies()
            .iter()
            .enumerate()
        {
            if b.inv_mass != 0.0 {
                dynamic += 1;
                awake += usize::from(sleep.is_row_awake(row));
            }
        }
        (dynamic, awake)
    }

    /// Whether every dynamic row was frozen on the last step.
    fn frozen(&self) -> bool {
        let (dynamic, awake) = self.awake_rows();
        dynamic == COLUMN_HEIGHT && awake == 0
    }

    /// Whether every dynamic row was awake on the last step.
    fn all_awake(&self) -> bool {
        let (dynamic, awake) = self.awake_rows();
        dynamic == COLUMN_HEIGHT && awake == COLUMN_HEIGHT
    }

    fn stats(&self) -> WarmSeedStats {
        self.world
            .resource::<ColoredSoftStepSolver>()
            .warm_seed_stats()
    }

    fn body(&self, e: Entity) -> RigidBody {
        *self
            .world
            .get_component::<RigidBody>(e)
            .expect("harness: a column box is live")
    }

    /// Every box's full state, bottom first, as bits.
    fn state(&self) -> Vec<[u32; 13]> {
        self.boxes
            .iter()
            .map(|&e| body_bits(&self.body(e)))
            .collect()
    }

    /// Every box's linear velocity, bottom first.
    fn velocities(&self) -> Vec<Vec3> {
        self.boxes
            .iter()
            .map(|&e| self.body(e).linear_velocity)
            .collect()
    }
}

// ── Scenario ─────────────────────────────────────────────────────────────────

/// What the scenario observed.
#[derive(Debug)]
struct Observation {
    /// The sleeper's first frozen step.
    freeze_step: usize,
    /// The wake step (`freeze_step + HOLD_STEPS + 1`).
    wake_step: usize,
    /// The sleeper's warm-seed statistics on the step before its first frozen step.
    sleeper_before_freeze: WarmSeedStats,
    /// The sleeper's warm-seed statistics on the wake step.
    sleeper: WarmSeedStats,
    /// The twin's warm-seed statistics on the wake step.
    twin: WarmSeedStats,
    /// Each world's warm cursor `Reset` count just before the wake step, `(sleeper, twin)`.
    resets_before_wake: (u64, u64),
    /// Whether every sleeper row was awake on the wake step.
    sleeper_woke: bool,
    /// Whether every twin row was awake on every step of the scenario.
    twin_never_slept: bool,
    /// The sleeper's box velocities after the wake step, bottom first.
    sleeper_velocity: Vec<Vec3>,
    /// The twin's box velocities after the same step, bottom first.
    twin_velocity: Vec<Vec3>,
    /// The twin's box velocities after the sleeper's first frozen step: a warm-started step
    /// from the exact state the sleeper froze in (the twin's state one step before), so a
    /// warm wake of the sleeper. The velocity check's control.
    twin_velocity_at_freeze: Vec<Vec3>,
    /// The twin's warm-seed statistics on the sleeper's first frozen step.
    twin_at_freeze: WarmSeedStats,
    /// The twin's full box state after the sleeper's first frozen step, bottom first.
    twin_state_at_freeze: Vec<[u32; 13]>,
    /// The sleeper's full box state on its first frozen step (the state it froze in).
    sleeper_state_frozen: Vec<[u32; 13]>,
    /// The sleeper's full box state after the wake step.
    sleeper_state_at_wake: Vec<[u32; 13]>,
    /// The twin's full box state after the wake step.
    twin_state_at_wake: Vec<[u32; 13]>,
    /// The sleeper's warm-seed statistics on its first frozen step and each held step, in
    /// step order.
    sleeper_frozen_steps: Vec<WarmSeedStats>,
}

fn run_scenario() -> Observation {
    let mut sleeper = Pile::new(DEFAULT_SLEEP_THRESHOLD);
    let mut twin = Pile::new(0.0);
    let mut twin_never_slept = true;

    let mut freeze_step = None;
    let mut sleeper_before_freeze = WarmSeedStats::default();
    for step in 1..=SETTLE_LIMIT {
        sleeper.step();
        twin.step();
        twin_never_slept &= twin.all_awake();
        if sleeper.frozen() {
            freeze_step = Some(step);
            break;
        }
        assert_eq!(
            sleeper.state(),
            twin.state(),
            "the twin diverged from the sleeper at step {step}, before the sleeper froze: \
             the twin is not the sleeper's never-slept counterpart, so it cannot be the control"
        );
        sleeper_before_freeze = sleeper.stats();
    }
    let freeze_step = freeze_step.unwrap_or_else(|| {
        panic!(
            "the sleeper did not freeze within {SETTLE_LIMIT} steps ({:?} dynamic/awake rows on \
             the last step), so there is no frozen island to wake",
            sleeper.awake_rows()
        )
    });
    let twin_velocity_at_freeze = twin.velocities();
    let twin_at_freeze = twin.stats();
    let twin_state_at_freeze = twin.state();
    let sleeper_state_frozen = sleeper.state();
    let mut sleeper_frozen_steps = Vec::with_capacity(HOLD_STEPS + 1);
    sleeper_frozen_steps.push(sleeper.stats());

    for held in 1..=HOLD_STEPS {
        sleeper.step();
        twin.step();
        twin_never_slept &= twin.all_awake();
        assert!(
            sleeper.frozen(),
            "the sleeper woke {held} steps into the hold, before the explicit wake ({:?} \
             dynamic/awake rows)",
            sleeper.awake_rows()
        );
        assert!(
            sleeper.state() == sleeper_state_frozen,
            "the frozen sleeper moved {held} steps into the hold"
        );
        sleeper_frozen_steps.push(sleeper.stats());
    }

    let resets_before_wake = (sleeper.stats().remap_resets, twin.stats().remap_resets);
    sleeper.world.resource_mut::<IslandSleep>().wake_all();
    sleeper.step();
    twin.step();
    twin_never_slept &= twin.all_awake();

    Observation {
        freeze_step,
        wake_step: freeze_step + HOLD_STEPS + 1,
        sleeper_before_freeze,
        sleeper: sleeper.stats(),
        twin: twin.stats(),
        resets_before_wake,
        sleeper_woke: sleeper.all_awake(),
        twin_never_slept,
        sleeper_velocity: sleeper.velocities(),
        twin_velocity: twin.velocities(),
        twin_velocity_at_freeze,
        twin_at_freeze,
        twin_state_at_freeze,
        sleeper_state_frozen,
        sleeper_state_at_wake: sleeper.state(),
        twin_state_at_wake: twin.state(),
        sleeper_frozen_steps,
    }
}

fn under_watchdog<T, F>(what: &str, budget: Duration, scene: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let handle = thread::Builder::new()
        .name("b1-frozen-island".to_string())
        .spawn(move || {
            let outcome = scene();
            done_tx.send(()).ok();
            outcome
        })
        .expect("harness: the scene thread must spawn");
    if let Err(mpsc::RecvTimeoutError::Timeout) = done_rx.recv_timeout(budget) {
        panic!(
            "watchdog: the scene '{what}' did not finish within {budget:?}. A panic inside a \
             scheduler worker does not propagate out of `Schedule::run` today, it blocks the \
             caller; the worker's own message, if any, is printed above this one."
        );
    }
    match handle.join() {
        Ok(outcome) => outcome,
        Err(payload) => panic::resume_unwind(payload),
    }
}

/// Runs the scenario, prints what both worlds saw, and checks the preconditions that make
/// the wake step's numbers mean what the tests read them as.
fn observe(what: &'static str) -> Observation {
    let obs = under_watchdog(what, SCENARIO_TIMEOUT, run_scenario);
    let down = max_downward;
    println!(
        "{what}: froze at step {}, woken at step {}\n  \
         sleeper, step before the freeze: {:?}\n  \
         twin, freeze step:               {:?}\n  \
         sleeper, wake step:              {:?}  (point_hits {} of {} points)\n  \
         twin, wake step:                 {:?}  (point_hits {} of {} points)\n  \
         max downward velocity, wake step: sleeper {:e} m/s, twin {:e} m/s \
         (twin at the freeze step: {:e} m/s)\n  \
         sleeper velocity (bottom first): {:?}\n  \
         twin velocity (bottom first):    {:?}\n  \
         twin velocity at freeze:         {:?}\n  \
         sleeper (carry_points, carry_hits) on its {} frozen steps: {:?}\n  \
         sleeper wake-step state == twin state after the sleeper's first frozen step: {}",
        obs.freeze_step,
        obs.wake_step,
        obs.sleeper_before_freeze,
        obs.twin_at_freeze,
        obs.sleeper,
        obs.sleeper.point_hits,
        obs.sleeper.points,
        obs.twin,
        obs.twin.point_hits,
        obs.twin.points,
        down(&obs.sleeper_velocity),
        down(&obs.twin_velocity),
        down(&obs.twin_velocity_at_freeze),
        obs.sleeper_velocity,
        obs.twin_velocity,
        obs.twin_velocity_at_freeze,
        obs.sleeper_frozen_steps.len(),
        obs.sleeper_frozen_steps
            .iter()
            .map(|s| (s.carry_points, s.carry_hits))
            .collect::<Vec<_>>(),
        obs.sleeper_state_at_wake == obs.twin_state_at_freeze,
    );

    assert!(
        obs.twin_never_slept,
        "{what}: the twin (threshold 0) slept on some step, so it is not a never-slept control"
    );
    assert!(
        obs.sleeper_woke,
        "{what}: `wake_all` did not wake every sleeper row on the wake step"
    );
    assert!(
        obs.sleeper.points > 0 && obs.twin.points > 0,
        "{what}: a world pushed no contact point on the wake step (sleeper {}, twin {}), so a \
         hit count would be vacuous",
        obs.sleeper.points,
        obs.twin.points
    );
    for (who, stats, before) in [
        ("sleeper", obs.sleeper, obs.resets_before_wake.0),
        ("twin", obs.twin, obs.resets_before_wake.1),
    ] {
        assert_eq!(
            stats.remap_resets, before,
            "{what}: the {who}'s warm cursor recorded a Reset on the wake step, which would \
             miss every lookup for a reason other than the frozen island"
        );
        assert_eq!(
            stats.translated, stats.manifolds,
            "{what}: some of the {who}'s manifolds were not looked up under the previous \
             gather's rows on the wake step ({stats:?})"
        );
    }
    obs
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// B1, hits: the wake step of a frozen pile is warm-started at every contact point, as the
/// never-slept twin's same step is.
#[test]
fn a_woken_pile_is_warm_started_at_every_point_like_its_never_slept_twin() {
    let obs = observe("B1 hits");
    assert_eq!(
        obs.twin.point_hits, obs.twin.points,
        "control: the never-slept twin must be warm-started at every point on the wake step \
         ({:?}); if it is not, this scene cannot show a warm wake and the test below proves \
         nothing",
        obs.twin
    );
    assert_eq!(
        obs.sleeper.point_hits, obs.sleeper.points,
        "B1: the frozen pile lost its warm-start impulses while it slept. On the wake step \
         {} of its {} contact points found a warm entry; its never-slept twin found {} of {} \
         on the same step",
        obs.sleeper.point_hits, obs.sleeper.points, obs.twin.point_hits, obs.twin.points
    );
}

/// B1, velocity: the wake step of a frozen pile does not make it fall faster than its
/// never-slept twin on the same step by more than the engine's at-rest speed.
#[test]
fn a_woken_pile_does_not_sag_faster_than_its_never_slept_twin() {
    let obs = observe("B1 velocity");
    let bound = at_rest_speed();
    let twin = max_downward(&obs.twin_velocity);
    let warm_wake = max_downward(&obs.twin_velocity_at_freeze);
    let sleeper = max_downward(&obs.sleeper_velocity);
    assert!(
        warm_wake <= twin + bound,
        "control: a warm-started step from the sleeper's frozen state (the twin's first step \
         after it) falls at {warm_wake:e} m/s, more than the twin's wake-step {twin:e} m/s plus \
         the at-rest speed {bound:e} m/s; the bound does not admit a warm wake, so the check \
         below proves nothing"
    );
    assert!(
        sleeper <= twin + bound,
        "B1: the woken pile sagged. Its largest downward box velocity on the wake step is \
         {sleeper:e} m/s; its never-slept twin's on the same step is {twin:e} m/s, and the \
         tolerance is the at-rest speed {bound:e} m/s (a warm wake of the same frozen state: \
         {warm_wake:e} m/s)"
    );
}

/// B1, carry: on its first frozen step and on every held step, the frozen pile carries the
/// warm entry of every one of its live contact points into the next step's table.
///
/// This is the mechanism the two tests above observe only through its effect on the wake
/// step: a carry that drops entries part-way through the hold shows here on the step it
/// happens.
#[test]
fn a_frozen_pile_carries_every_warm_entry_on_every_frozen_step() {
    let obs = observe("B1 carry");
    let solved_before = obs.sleeper_before_freeze.points;
    for (k, s) in obs.sleeper_frozen_steps.iter().enumerate() {
        assert_eq!(
            (s.points, s.carry_points),
            (0, solved_before),
            "control: on frozen step {k} (0 = the first) the sleeper must solve no point and \
             count all {solved_before} points it solved before the freeze as frozen ({s:?}); \
             otherwise the carry count below has nothing to count"
        );
    }
    for (k, s) in obs.sleeper_frozen_steps.iter().enumerate() {
        assert_eq!(
            s.carry_hits, s.carry_points,
            "B1: frozen step {k} (0 = the first) carried {} of its {} frozen points' warm \
             entries into the next step's table; the rest were dropped, and the island wakes \
             without them ({s:?})",
            s.carry_hits, s.carry_points
        );
    }
}

/// The first box, and the first of its 13 state fields, at which two full box states differ.
fn first_difference(a: &[[u32; 13]], b: &[[u32; 13]]) -> Option<(usize, usize)> {
    a.iter().zip(b).enumerate().find_map(|(i, (x, y))| {
        x.iter()
            .zip(y)
            .position(|(p, q)| p != q)
            .map(|field| (i, field))
    })
}

/// B1, exactness: the carry keeps the frozen island's impulses bit for bit, so the step that
/// wakes the pile computes exactly the step its never-slept twin took from the same state.
///
/// The sleeper froze in the state the twin held after step `F - 1` (`F` the sleeper's first
/// frozen step; the scenario asserts the worlds bit-identical through `F - 1`), and the twin
/// stepped that state once, warm-started from step `F - 1`'s impulses. A carry that keeps
/// those impulses exactly hands the sleeper's wake step the same inputs. So the sleeper's
/// state after the wake step must equal the twin's state after step `F`, every field of every
/// box, as bits. The velocity bound above admits any wake within the at-rest speed; this one
/// admits none but the exact one, so a carry that scales, rounds or re-derives an impulse
/// fails it even where the bound passes.
#[test]
fn a_woken_pile_steps_exactly_as_its_twin_stepped_from_the_frozen_state() {
    let obs = observe("B1 exact");
    assert!(
        obs.twin_state_at_wake != obs.twin_state_at_freeze
            && obs.sleeper_state_at_wake != obs.sleeper_state_frozen,
        "control: the pile must still move at the bit level, the twin between the sleeper's \
         first frozen step and the wake step, and the sleeper across the wake step; otherwise \
         the equality below could hold for a pile that never moves"
    );
    assert!(
        obs.sleeper_state_at_wake == obs.twin_state_at_freeze,
        "B1: the woken pile did not step as its twin stepped from the same frozen state. First \
         difference (box bottom-first, field of 13: position, velocity, rotation, angular \
         velocity): {:?}; sleeper velocities after the wake step {:?}, twin velocities after \
         the sleeper's first frozen step {:?}",
        first_difference(&obs.sleeper_state_at_wake, &obs.twin_state_at_freeze),
        obs.sleeper_velocity,
        obs.twin_velocity_at_freeze
    );
}

// ── A row move while frozen ──────────────────────────────────────────────────

/// Where the row-move scene's lone box rests on the floor, clear of the column.
const MOVER_AT: Vec3 = Vec3::new(10.0, HALF_BOX, 10.0);
/// Steps held frozen before, and again after, the row move.
const MOVE_HOLD_STEPS: usize = 5;

/// What the row-move scenario observed.
#[derive(Debug)]
struct RowMoveObservation {
    /// The step on which the column and the lone box were both frozen.
    freeze_step: usize,
    /// Warm-seed statistics on the step before the freeze.
    before_freeze: WarmSeedStats,
    /// The lone box's gather row before and after the delete.
    mover_rows: (usize, usize),
    /// The warm cursor's `Reset` count before and after the move step.
    resets: (u64, u64),
    /// Warm-seed statistics on every frozen step, the move step included, in step order;
    /// `move_index` indexes the move step.
    frozen_steps: Vec<WarmSeedStats>,
    move_index: usize,
    /// Whether every dynamic row stayed frozen on every held step.
    stayed_frozen: bool,
    /// Warm-seed statistics on the wake step.
    wake: WarmSeedStats,
}

/// The gather row of the dynamic body resting at [`MOVER_AT`].
fn mover_row(world: &EcsMaster) -> usize {
    world
        .resource::<SolverScratch>()
        .bodies()
        .iter()
        .position(|b| {
            b.inv_mass != 0.0
                && (b.position.x - MOVER_AT.x).abs() < HALF_BOX
                && (b.position.z - MOVER_AT.z).abs() < HALF_BOX
        })
        .expect("harness: the lone box is in the gather")
}

/// Dynamic rows awake on the last step.
fn awake_dynamic_rows(world: &EcsMaster) -> usize {
    let sleep = world.resource::<IslandSleep>();
    world
        .resource::<SolverScratch>()
        .bodies()
        .iter()
        .enumerate()
        .filter(|(row, b)| b.inv_mass != 0.0 && sleep.is_row_awake(*row))
        .count()
}

/// Floor (row 0), a static box far above the floor that touches nothing (row 1), the column
/// (rows 2..=6), and a lone box resting on the floor (row 7). Once everything is frozen the
/// static box is deleted, so the lone box swap-moves from row 7 into row 1 while frozen. Its
/// only partner is the floor, row 0, which stays below it, so its one manifold keeps its pair
/// order and the warm cursor can translate it; the column's rows do not move. The static box
/// has no contact, so no entry of the previous table is keyed by row 1.
fn run_row_move() -> RowMoveObservation {
    let (mut world, mut physics) = colored_world(serial_pool(), DEFAULT_SLEEP_THRESHOLD, false);
    spawn_box(
        &mut world,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(50.0, 1.0, 50.0),
        0.0,
        Mat3::ZERO,
    );
    let far = spawn_box(
        &mut world,
        Vec3::new(-40.0, 20.0, -40.0),
        Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
        0.0,
        Mat3::ZERO,
    );
    for i in 0..COLUMN_HEIGHT {
        spawn_unit_box(
            &mut world,
            Vec3::new(0.0, HALF_BOX + 2.0 * HALF_BOX * i as f32, 0.0),
        );
    }
    spawn_unit_box(&mut world, MOVER_AT);
    let stats = |world: &EcsMaster| world.resource::<ColoredSoftStepSolver>().warm_seed_stats();

    let mut before_freeze = WarmSeedStats::default();
    let mut freeze_step = None;
    for step in 1..=SETTLE_LIMIT {
        physics.run(&mut world);
        if awake_dynamic_rows(&world) == 0 {
            freeze_step = Some(step);
            break;
        }
        before_freeze = stats(&world);
    }
    let freeze_step = freeze_step.unwrap_or_else(|| {
        panic!(
            "the row-move scene did not freeze within {SETTLE_LIMIT} steps ({} dynamic rows \
             awake)",
            awake_dynamic_rows(&world)
        )
    });
    let mut frozen_steps = vec![stats(&world)];
    let mut stayed_frozen = true;
    for _ in 0..MOVE_HOLD_STEPS {
        physics.run(&mut world);
        stayed_frozen &= awake_dynamic_rows(&world) == 0;
        frozen_steps.push(stats(&world));
    }

    let row_before = mover_row(&world);
    let resets_before = stats(&world).remap_resets;
    assert!(
        world.delete_entity(far),
        "construction: the far static box must be despawnable"
    );
    physics.run(&mut world);
    stayed_frozen &= awake_dynamic_rows(&world) == 0;
    let move_index = frozen_steps.len();
    frozen_steps.push(stats(&world));
    let mover_rows = (row_before, mover_row(&world));
    let resets = (resets_before, stats(&world).remap_resets);
    for _ in 0..MOVE_HOLD_STEPS {
        physics.run(&mut world);
        stayed_frozen &= awake_dynamic_rows(&world) == 0;
        frozen_steps.push(stats(&world));
    }

    world.resource_mut::<IslandSleep>().wake_all();
    physics.run(&mut world);
    RowMoveObservation {
        freeze_step,
        before_freeze,
        mover_rows,
        resets,
        frozen_steps,
        move_index,
        stayed_frozen,
        wake: stats(&world),
    }
}

/// B1, row move: a frozen island whose row moves while it is held still carries every warm
/// entry, translated to its new row, and wakes warm at every point.
///
/// The carry reads each entry under the rows the previous table was keyed by and stores it
/// under this gather's rows. With no row move the two keys are the same, so the column
/// scenario above cannot tell which one the carry reads; here the lone box's floor pair is
/// keyed (0, 7) in the previous table and (0, 1) in this step's.
#[test]
fn a_frozen_island_whose_row_moves_while_held_carries_every_warm_entry() {
    let obs = under_watchdog("B1 row move", SCENARIO_TIMEOUT, run_row_move);
    println!(
        "B1 row move: froze at step {}; lone box rows {:?}; resets {:?}; stayed frozen {}\n  \
         before the freeze: {:?}\n  \
         (carry_points, carry_hits) per frozen step, move step at index {}: {:?}\n  \
         wake step: {:?}",
        obs.freeze_step,
        obs.mover_rows,
        obs.resets,
        obs.stayed_frozen,
        obs.before_freeze,
        obs.move_index,
        obs.frozen_steps
            .iter()
            .map(|s| (s.carry_points, s.carry_hits))
            .collect::<Vec<_>>(),
        obs.wake
    );
    assert_eq!(
        obs.mover_rows,
        (7, 1),
        "construction: deleting the far static box must swap-move the lone box from row 7 into \
         row 1"
    );
    assert!(
        obs.stayed_frozen && obs.resets.0 == obs.resets.1,
        "control: nothing may wake and the warm cursor may not Reset across the move, or a miss \
         would have another cause ({obs:?})"
    );
    // The lone box and the column are separate islands and may freeze on different steps, so
    // the step before the whole scene froze may already carry some of its points.
    let live_before = obs.before_freeze.points + obs.before_freeze.carry_points;
    assert!(
        live_before > 0
            && obs
                .frozen_steps
                .iter()
                .all(|s| s.points == 0 && s.carry_points == live_before),
        "control: on every frozen step the scene must solve no point and count all \
         {live_before} live points it had before the freeze as frozen ({obs:?})"
    );
    for (k, s) in obs.frozen_steps.iter().enumerate() {
        assert_eq!(
            s.carry_hits, s.carry_points,
            "B1: frozen step {k} (the move step is {}) carried {} of its {} frozen points' warm \
             entries; the rest were dropped ({s:?})",
            obs.move_index, s.carry_hits, s.carry_points
        );
    }
    assert_eq!(
        obs.wake.point_hits, obs.wake.points,
        "B1: after the row move the woken scene found a warm entry at {} of its {} points",
        obs.wake.point_hits, obs.wake.points
    );
}

// ── Worker-count determinism ─────────────────────────────────────────────────

/// The worker counts whose runs must be bit-identical.
const CROWD_WORKERS: [usize; 2] = [1, 8];
/// Columns in the crowd's first group, of heights 1, 2, 3 in turn, so that its islands
/// freeze on different steps.
const CROWD_COLUMNS: usize = 12;
/// Single boxes in the crowd's second group, spawned resting on the floor once the first
/// group has frozen. Each has one four-point floor manifold, and floor manifolds of distinct
/// boxes are body-disjoint, so they share a color: 256 slots, the solver's parallel dispatch
/// floor, while the first group is frozen and carried.
///
/// 256 solved points is also a power of two on purpose. The solver starts from 2-slot warm
/// tables that only grow, and a table sized for the solved points alone would get
/// `2 · 256 = 512` slots; the carried points on top of them then load it past 0.5, which the
/// store's load `debug_assert!` reports. Sized for both sets, as the store is, it gets 1024.
const CROWD_SINGLES: usize = 64;
/// The colored solver's per-color dispatch floor (`MIN_PARALLEL_SLOTS_PER_COLOR`, private to
/// `solver/colored.rs`; mirrored here for the anti-vacuity witness only).
const MIN_PARALLEL_SLOTS_PER_COLOR: usize = 256;
/// Steps the crowd is held with every island frozen, before the spawn and before the wake.
const CROWD_HOLD_STEPS: usize = 10;
/// Steps run after the wake.
const CROWD_AFTER_WAKE_STEPS: usize = 60;

/// One world of the determinism scene, on a pool of `workers` threads, with the colored
/// solve's parallel dispatch opted in.
struct Crowd {
    world: EcsMaster,
    physics: Schedule,
    pool: Arc<ThreadPool>,
    boxes: Vec<Entity>,
}

impl Crowd {
    fn new(workers: usize) -> Self {
        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        let (mut world, physics) = colored_world(Arc::clone(&pool), DEFAULT_SLEEP_THRESHOLD, true);
        spawn_box(
            &mut world,
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(50.0, 1.0, 50.0),
            0.0,
            Mat3::ZERO,
        );
        let mut boxes = Vec::with_capacity(2 * CROWD_COLUMNS + CROWD_SINGLES);
        for c in 0..CROWD_COLUMNS {
            let x = -30.0 + 5.0 * c as f32;
            for layer in 0..=(c % 3) {
                let y = HALF_BOX + 2.0 * HALF_BOX * layer as f32;
                boxes.push(spawn_unit_box(&mut world, Vec3::new(x, y, -30.0)));
            }
        }
        Self {
            world,
            physics,
            pool,
            boxes,
        }
    }

    /// The second group: an 8 × 8 grid of single boxes resting on the floor, 2 m apart,
    /// clear of the columns.
    fn spawn_singles(&mut self) {
        for i in 0..CROWD_SINGLES {
            let x = -20.0 + 4.0 * (i % 8) as f32;
            let z = 4.0 * (i / 8) as f32;
            let e = spawn_unit_box(&mut self.world, Vec3::new(x, HALF_BOX, z));
            self.boxes.push(e);
        }
    }

    fn step(&mut self) {
        self.physics.run(&mut self.world);
    }

    fn stats(&self) -> WarmSeedStats {
        self.world
            .resource::<ColoredSoftStepSolver>()
            .warm_seed_stats()
    }

    /// Every box's full state, in spawn order, as bits.
    fn state(&self) -> Vec<[u32; 13]> {
        self.boxes
            .iter()
            .map(|&e| {
                body_bits(
                    self.world
                        .get_component::<RigidBody>(e)
                        .expect("harness: a crowd box is live"),
                )
            })
            .collect()
    }

    /// Dynamic rows awake on the last step.
    fn awake(&self) -> usize {
        awake_dynamic_rows(&self.world)
    }

    /// The widest color of the last step, counted in the contact points of the manifolds the
    /// solver pushed (those whose island was awake): the quantity the solver compares with
    /// its dispatch floor.
    fn widest_awake_color_slots(&self) -> usize {
        let graph = self.world.resource::<ConstraintGraph>();
        let manifolds = self.world.resource::<Manifolds>().solver_manifolds();
        let sleep = self.world.resource::<IslandSleep>();
        let bodies = self.world.resource::<SolverScratch>().bodies();
        (0..graph.n_colors())
            .map(|c| {
                graph
                    .color(c)
                    .iter()
                    .map(|&mi| {
                        let m = &manifolds[mi as usize];
                        let a = m.body_a.0 as usize;
                        let dynamic = if bodies[a].inv_mass != 0.0 {
                            a
                        } else {
                            m.body_b.0 as usize
                        };
                        if sleep.is_row_awake(dynamic) {
                            usize::from(m.count)
                        } else {
                            0
                        }
                    })
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0)
    }
}

/// What the determinism scenario observed, on the one-worker world (the others are asserted
/// bit-identical to it on every step).
#[derive(Debug)]
struct CrowdObservation {
    /// Thread counts of the compared pools, as built.
    pool_threads: Vec<usize>,
    /// The first step on which some column island was frozen.
    columns_first_froze: usize,
    /// The first step on which every column island was frozen.
    columns_all_froze: usize,
    /// The step on which the whole crowd, singles included, was frozen.
    crowd_froze: usize,
    /// Steps on which frozen points were carried while the widest pushed color was at or
    /// over the dispatch floor.
    carried_while_dispatching: usize,
    /// The most entries the carry re-inserted on such a step.
    most_carry_hits_while_dispatching: u32,
    /// Frozen steps on which the carry found fewer entries than it had frozen points (a
    /// point whose feature id differs from the last solved step's: a documented miss, which
    /// the never-slept twin of that step misses too). Reported, not asserted.
    steps_with_carry_misses: usize,
    /// The statistics of the last held step before the wake, and of the wake step.
    last_held: WarmSeedStats,
    wake: WarmSeedStats,
}

/// Steps every world once and asserts each is bit-identical to the first: every box's full
/// state and the warm-seed statistics.
fn step_all(worlds: &mut [Crowd], step: usize, phase: &str) {
    for w in worlds.iter_mut() {
        w.step();
    }
    let (reference, rest) = worlds.split_first().expect("harness: at least one world");
    let (state, stats) = (reference.state(), reference.stats());
    for other in rest {
        let other_state = other.state();
        assert!(
            other_state == state,
            "determinism: the {}-worker world diverged from the 1-worker world at step {step} \
             ({phase}); first difference (box in spawn order, field of 13): {:?}",
            other.pool.num_threads(),
            first_difference(&other_state, &state)
        );
        assert_eq!(
            other.stats(),
            stats,
            "determinism: the {}-worker world's warm-seed statistics diverged at step {step} \
             ({phase})",
            other.pool.num_threads()
        );
    }
}

fn run_crowd() -> CrowdObservation {
    let mut worlds: Vec<Crowd> = CROWD_WORKERS.iter().map(|&w| Crowd::new(w)).collect();
    let pool_threads = worlds.iter().map(|w| w.pool.num_threads()).collect();
    let column_boxes = worlds[0].boxes.len();
    let mut step = 0usize;
    let mut carried_while_dispatching = 0usize;
    let mut most_carry_hits_while_dispatching = 0u32;
    let mut steps_with_carry_misses = 0usize;
    let mut tally = |w: &Crowd| {
        let s = w.stats();
        if s.carry_points > 0 && w.widest_awake_color_slots() >= MIN_PARALLEL_SLOTS_PER_COLOR {
            carried_while_dispatching += 1;
            most_carry_hits_while_dispatching = most_carry_hits_while_dispatching.max(s.carry_hits);
        }
        steps_with_carry_misses += usize::from(s.carry_hits != s.carry_points);
    };

    // The columns settle and freeze, island by island.
    let mut columns_first_froze = None;
    let columns_all_froze = loop {
        step += 1;
        assert!(
            step <= SETTLE_LIMIT,
            "the columns did not all freeze within {SETTLE_LIMIT} steps ({} of {column_boxes} \
             boxes awake)",
            worlds[0].awake()
        );
        step_all(&mut worlds, step, "columns settling");
        tally(&worlds[0]);
        let awake = worlds[0].awake();
        if awake < column_boxes && columns_first_froze.is_none() {
            columns_first_froze = Some(step);
        }
        if awake == 0 {
            break step;
        }
    };
    for _ in 0..CROWD_HOLD_STEPS {
        step += 1;
        step_all(&mut worlds, step, "columns held");
        tally(&worlds[0]);
    }

    // The singles arrive while the columns are frozen, then settle and freeze in turn.
    for w in worlds.iter_mut() {
        w.spawn_singles();
    }
    let settle_from = step;
    let crowd_froze = loop {
        step += 1;
        assert!(
            step - settle_from <= SETTLE_LIMIT,
            "the singles did not all freeze within {SETTLE_LIMIT} steps ({} boxes awake)",
            worlds[0].awake()
        );
        step_all(&mut worlds, step, "singles settling");
        tally(&worlds[0]);
        if worlds[0].awake() == 0 {
            break step;
        }
    };
    for _ in 0..CROWD_HOLD_STEPS {
        step += 1;
        step_all(&mut worlds, step, "crowd held");
        tally(&worlds[0]);
    }
    let last_held = worlds[0].stats();

    // Everything wakes at once and runs on.
    for w in worlds.iter_mut() {
        w.world.resource_mut::<IslandSleep>().wake_all();
    }
    step += 1;
    step_all(&mut worlds, step, "wake step");
    let wake = worlds[0].stats();
    for _ in 0..CROWD_AFTER_WAKE_STEPS {
        step += 1;
        step_all(&mut worlds, step, "after the wake");
    }

    CrowdObservation {
        pool_threads,
        columns_first_froze: columns_first_froze.expect("invariant: set before every column froze"),
        columns_all_froze,
        crowd_froze,
        carried_while_dispatching,
        most_carry_hits_while_dispatching,
        steps_with_carry_misses,
        last_held,
        wake,
    }
}

/// B1, determinism: frozen islands carried while other islands are solved through the
/// parallel dispatch leave a 1-worker and an 8-worker world bit-identical on every step,
/// through islands freezing on different steps, a spawn while frozen, the hold and the wake.
///
/// The carry runs in the serial store from the serial freeze set, the manifold list and the
/// previous table, so nothing in it should see the worker count. This is the end-to-end check
/// on the real schedule, where the broadphase, the narrowphase and the solve all cut their
/// work by the pool's thread count.
#[test]
fn frozen_islands_carried_beside_a_parallel_solve_are_bit_identical_across_worker_counts() {
    let obs = under_watchdog("B1 determinism", SCENARIO_TIMEOUT, run_crowd);
    println!(
        "B1 determinism: {} steps carried with a feature-id miss (reported, not asserted); \
         {obs:?}",
        obs.steps_with_carry_misses
    );
    assert_eq!(
        obs.pool_threads,
        CROWD_WORKERS.to_vec(),
        "construction: the compared pools must have the requested thread counts"
    );
    assert!(
        obs.columns_first_froze < obs.columns_all_froze && obs.columns_all_froze < obs.crowd_froze,
        "construction: the column islands must freeze on different steps, and before the \
         singles ({obs:?})"
    );
    assert!(
        obs.carried_while_dispatching > 0 && obs.most_carry_hits_while_dispatching > 0,
        "anti-vacuity: no step carried a warm entry while a pushed color reached the solver's \
         dispatch floor of {MIN_PARALLEL_SLOTS_PER_COLOR} slots, so the worker counts were never \
         compared on a carry beside a parallel solve ({obs:?})"
    );
    assert!(
        obs.last_held.carry_hits > 0 && obs.wake.point_hits == obs.last_held.carry_hits,
        "anti-vacuity: the wake step must read exactly the entries the last held step carried, \
         and there must be some, or the compared steps never consumed what the carry kept \
         ({obs:?})"
    );
}
