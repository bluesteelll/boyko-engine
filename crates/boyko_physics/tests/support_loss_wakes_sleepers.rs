//! Defect A4: losing a SUPPORT must wake the body that rested on it.
//!
//! A body latched asleep on top of another body keeps its pose restored every step and is
//! never flagged touched, and `Changed<RigidBody>` is deliberately not a wake condition.
//! Before the A4 fix nothing in the sleep latch looked at a removed row's contact partners,
//! so when the body underneath went away the upper body stayed frozen in mid-air forever.
//! Measured on the tree without the fix (`d552be05`, branch `merge/ke16-into-ecsnative`):
//! the upper body dropped by exactly 0 m over 30 steps and its row was awake on 0 of them,
//! for a despawn (with and without a row move), for a `RigidBody` removal and for a pose
//! write 10 m away. The fix (wake-on-contact-change, `IslandSleep::begin_step`) unlatches a
//! latched row whose island's manifold count changed.
//!
//! # Scene
//!
//! A static floor box (top face at `y = 0`), a support sphere S under an upper sphere U, a
//! bystander stack B1 under B2, and a lone sphere L resting on the floor far from
//! everything. Every body carries [`BodyId`], so every assertion follows entities, not
//! rows. The real `add_physics_colored_solve` schedule runs with sleeping on and
//! `sleep_frames = 8`; the pile is stepped until `IslandSleep::is_row_awake` is false for
//! every tracked body.
//!
//! The box-pile scene of `e_*` is a static floor box, a support cube S and an upper cube U
//! resting on it, with L9 contact reuse forced on. It exists because L9's ruling W2 needs a
//! box pile: contact reuse touches box pairs only, so no sphere scene here can go red on it.
//!
//! # Cases
//!
//! | test | change applied | before A4 | after A4 |
//! |---|---|---|---|
//! | `a_deleting_*` | `delete_entity` of S, with S the last row (no row moves) and with S in row 1 (L swap-moves) | RED | GREEN |
//! | `a_the_wake_lands_*` (I1) | `delete_entity` of S (last row); U must be awake on the FIRST step after it | not run | GREEN |
//! | `b_*` | `Commands::remove::<RigidBody>()` on S (S leaves both the gather and the apply walk) | RED | GREEN |
//! | `d_*` | S's `RigidBody::position` written 10 m away (a support loss with no removal) | RED | GREEN |
//! | `e_*` (L9 ruling W2) | box pile, contact reuse on: S lowered by τ_eff / 2, so U's record refreshes with every point lifted (D6) and the pair has no manifold | not run | GREEN |
//! | `c_*` | `disable::<Simulated>()` on S (S stays in the gather, parked) | GREEN guard | GREEN guard |
//! | `control_deleting_a_body_*` | `delete_entity` of L (last row), which touches only the static floor: nobody may wake | GREEN | GREEN |
//! | `control_deleting_a_lone_body_*` (I2) | `delete_entity` of L from row 1: S swap-moves and the island ids renumber; nobody may wake | not run | GREEN |
//! | `attribution_*` | the same change followed by `IslandSleep::wake_all()` | GREEN | GREEN |
//!
//! The `attribution_*` tests are the anti-vacuity evidence for the red ones: the SAME
//! assertion, on the SAME scene, passes as soon as a wake happens, so the red is the
//! missing wake and not an assertion that can never hold.
//!
//! Disabling `Simulated` is NOT a support loss in this engine's model — a parked body with
//! `inv_mass != 0` stays an island member and a collider (on the box floor this scene uses;
//! the SDF-field case is not covered, see `IslandSleep::begin_step`), and
//! [`Simulated`](boyko_physics::components::Simulated) documents that its pose is frozen
//! in place while the solve still applies impulses to it. `c_*` guards that behaviour: the
//! parked support must keep carrying U. It runs with an explicit `wake_all` so that U is
//! demonstrably awake while it is being carried — without one the guard would pass
//! vacuously, out of the very freeze the other tests are about.
//!
//! Removing `Collider` from the support is a DIFFERENT defect (it desyncs the gather and
//! apply walks) and lives in `apply_row_alignment.rs`.
//!
//! Device-free. Every scene spins up a `boyko_threadpool`, which is intractable under
//! Miri, and runs on its own thread under a watchdog: a panic inside a scheduler worker
//! does not propagate out of `Schedule::run` today, it blocks the caller, so a test that
//! trips an invariant inside a system would otherwise hang the whole binary.

use std::panic;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_macros::Component;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
#[cfg(not(miri))]
use boyko_physics::resources::PairClasses;
use boyko_physics::resources::{
    ConstraintGraph, IslandSleep, Manifolds, PhysicsConfig, SleepSkip, SolverScratch,
};

// ── Scene constants ──────────────────────────────────────────────────────────

/// The fixed step (60 Hz), as in `sleeping_pipeline_o8.rs`.
const DT: f32 = 1.0 / 60.0;
/// The brief's debounce.
const SLEEP_FRAMES: u16 = 8;
/// The latch is checked only after this many steps.
const MIN_SETTLE_STEPS: usize = 120;
/// Upper bound on the settle loop; exceeding it is a harness failure, not the defect.
const MAX_SETTLE_STEPS: usize = 600;
/// Steps run after the change.
const POST_STEPS: usize = 30;
/// The upper body must drop by more than this in `POST_STEPS` steps (free fall covers
/// about 1.23 m in 0.5 s; U lands on the floor after 1 m).
const MIN_DROP: f32 = 0.5;
/// How far U may drift while a PARKED support still carries it (the `c_*` guard). An
/// order of magnitude below [`MIN_DROP`], so "still carried" and "fell" cannot be
/// confused.
const MAX_CARRIED_DRIFT: f32 = 0.05;
/// A static floor box whose top face is `y = 0` and which spans `x, z` in `[-20, 20]`.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(20.0, 0.5, 20.0);

/// Wall-clock budget for one scene. NOT a performance measurement: it is the liveness
/// guard described in the module header, sized far above any honest run of ~660 steps of
/// a six-body scene so that only a hang can reach it.
const SCENE_TIMEOUT: Duration = Duration::from_secs(if cfg!(debug_assertions) { 300 } else { 120 });

/// Body identities carried in [`BodyId`].
const FLOOR: u32 = 0;
const SUPPORT: u32 = 1;
const UPPER: u32 = 2;
const BY1: u32 = 3;
const BY2: u32 = 4;
const LONE: u32 = 5;
/// Every dynamic body the settle loop waits on.
const TRACKED: [u32; 5] = [SUPPORT, UPPER, BY1, BY2, LONE];

// ── Test-only component ──────────────────────────────────────────────────────

/// Stable identity of a body, so assertions follow entities rather than rows.
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct BodyId {
    id: u32,
}

// ── Watchdog ─────────────────────────────────────────────────────────────────

/// Runs `scene` on its own thread and fails the test if it has not finished within
/// [`SCENE_TIMEOUT`], instead of blocking the process.
///
/// A panic raised inside a scheduler worker does not propagate out of `Schedule::run`
/// today — it blocks the caller — so without this every such panic costs the whole test
/// binary. A panic on the scene thread itself is re-raised here with its original payload,
/// so an ordinary failed assertion still reports its own message.
fn under_watchdog<T, F>(what: &str, scene: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let handle = thread::Builder::new()
        .name("a4-scene".to_string())
        .spawn(move || {
            let outcome = scene();
            // Sent only on the success path: a panic drops the sender instead, which the
            // receiver sees as a disconnect and the join below re-raises.
            done_tx.send(()).ok();
            outcome
        })
        .expect("harness: the scene thread must spawn");
    if let Err(mpsc::RecvTimeoutError::Timeout) = done_rx.recv_timeout(SCENE_TIMEOUT) {
        panic!(
            "watchdog: the scene '{what}' did not finish within {SCENE_TIMEOUT:?}. A panic inside \
             a scheduler worker does not propagate out of `Schedule::run` today, it blocks the \
             caller, so this is what an invariant tripped inside a physics system looks like; the \
             worker's own message is printed above this one."
        );
    }
    match handle.join() {
        Ok(outcome) => outcome,
        Err(payload) => panic::resume_unwind(payload),
    }
}

// ── Harness ──────────────────────────────────────────────────────────────────

/// Views a `#[repr(C)]` POD value as its bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, initialised `#[repr(C)]` `T` borrowed for the returned
    // slice's lifetime; the slice covers exactly its `size_of::<T>()` bytes read-only —
    // the layout the component pool stores (mirrors `sleeping_pipeline_o8::as_bytes`).
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A single-threaded pool (deterministic).
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// What to spawn.
#[derive(Clone, Copy)]
struct Spec {
    id: u32,
    position: Vec3,
    shape: ColliderShape,
    inv_mass: f32,
}

/// The static floor box (top face at `y = 0`).
fn floor() -> Spec {
    Spec {
        id: FLOOR,
        position: Vec3::new(0.0, -0.5, 0.0),
        shape: ColliderShape::Box {
            half_extents: FLOOR_HALF_EXTENTS,
        },
        inv_mass: 0.0,
    }
}

/// A unit-mass sphere of radius 0.5 at rest.
fn ball(id: u32, position: Vec3) -> Spec {
    Spec {
        id,
        position,
        shape: ColliderShape::Sphere { radius: 0.5 },
        inv_mass: 1.0,
    }
}

/// Bit-exact vector equality (the solver is run-to-run bit-deterministic).
fn bits_eq(a: Vec3, b: Vec3) -> bool {
    a.x.to_bits() == b.x.to_bits()
        && a.y.to_bits() == b.y.to_bits()
        && a.z.to_bits() == b.z.to_bits()
}

struct Harness {
    world: EcsMaster,
    physics: Schedule,
    /// `{RigidBody, RigidBodyMass, Collider, BodyId}`: every body lives here.
    plain: ArchetypeId,
}

impl Harness {
    /// The colored physics schedule of `sleeping_pipeline_o8.rs` with sleeping on and
    /// `sleep_frames = 8`.
    fn new() -> Self {
        let mut world = EcsMaster::new();
        let plain = world.create_archetype(&[
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
            BodyId::component_id(),
        ]);
        let mut builder = ScheduleBuilder::new(serial_pool());
        let _keys = add_physics_colored_solve(&mut builder, &mut world);
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        let physics = builder.build(&mut world);
        {
            let cfg = world.resource_mut::<PhysicsConfig>();
            cfg.sleeping = true;
            cfg.sleep_frames = SLEEP_FRAMES;
        }
        Self {
            world,
            physics,
            plain,
        }
    }

    fn spawn(&mut self, spec: Spec) -> Entity {
        let body = RigidBody {
            position: spec.position,
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        };
        let mass = RigidBodyMass {
            inv_inertia: if spec.inv_mass == 0.0 {
                Mat3::ZERO
            } else {
                Mat3::IDENTITY
            },
            inv_mass: spec.inv_mass,
            restitution: 0.0,
            friction: 0.5,
        };
        let collider = Collider {
            shape: spec.shape,
            layer: 1,
            mask: 1,
        };
        let id = BodyId { id: spec.id };
        let entity = self
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
        self.world.enable::<Simulated>(entity);
        entity
    }

    fn step(&mut self) {
        self.physics.run(&mut self.world);
    }

    /// Body ids in the gather's walk order (row `i` of the solver is element `i`).
    fn walk_ids(&mut self) -> Vec<u32> {
        let q = self
            .world
            .query::<(&RigidBody, &RigidBodyMass, &Collider, &BodyId), ()>();
        q.iter().map(|(_, _, _, id)| id.id).collect()
    }

    fn row_of(&mut self, id: u32) -> usize {
        self.walk_ids()
            .iter()
            .position(|&x| x == id)
            .unwrap_or_else(|| panic!("harness: body {id} is not in the gather walk"))
    }

    /// Whether body `id`'s row was awake (solved + integrated) on the last step.
    fn awake(&mut self, id: u32) -> bool {
        let row = self.row_of(id);
        self.world.resource::<IslandSleep>().is_row_awake(row)
    }

    fn body(&self, entity: Entity) -> RigidBody {
        *self
            .world
            .get_component::<RigidBody>(entity)
            .expect("harness: a tracked body is live")
    }

    /// Checks the test's row map against the solver's own snapshot: after a step, gather
    /// row `i` holds exactly the pose of the body the walk puts at `i`. Call right after a
    /// step and before any structural change.
    fn assert_walk_matches_gather(&mut self) {
        let walked: Vec<(u32, Vec3)> = {
            let q = self
                .world
                .query::<(&RigidBody, &RigidBodyMass, &Collider, &BodyId), ()>();
            q.iter().map(|(b, _, _, id)| (id.id, b.position)).collect()
        };
        let gathered: Vec<Vec3> = self
            .world
            .resource::<SolverScratch>()
            .bodies()
            .iter()
            .map(|b| b.position)
            .collect();
        assert_eq!(
            walked.len(),
            gathered.len(),
            "harness: the test's walk and the gather disagree on the row count"
        );
        for (row, ((id, walked_pos), gathered_pos)) in walked.iter().zip(&gathered).enumerate() {
            assert!(
                bits_eq(*walked_pos, *gathered_pos),
                "harness: walk row {row} is body {id} at {walked_pos:?}, but gather row {row} is at \
                 {gathered_pos:?} - the test's row map is not the solver's"
            );
        }
    }

    /// Ids of every body that shares a manifold with at least one live point with body
    /// `id` on the last step, ascending. Valid only right after a step and before any
    /// structural change (manifolds are keyed by that step's rows).
    fn contact_ids(&mut self, id: u32) -> Vec<u32> {
        let walk = self.walk_ids();
        let row = walk
            .iter()
            .position(|&x| x == id)
            .unwrap_or_else(|| panic!("harness: body {id} is not in the gather walk"))
            as u32;
        let mut out: Vec<u32> = self
            .world
            .resource::<Manifolds>()
            .manifolds()
            .iter()
            .filter(|m| m.count > 0)
            .filter_map(|m| {
                let (a, b) = (m.body_a.0, m.body_b.0);
                let other = if a == row {
                    b
                } else if b == row {
                    a
                } else {
                    return None;
                };
                walk.get(other as usize).copied()
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Steps at least `MIN_SETTLE_STEPS`, then until every body in `ids` has a row that
    /// was NOT awake on the last step. Returns the step count.
    fn settle_until_latched(&mut self, ids: &[u32]) -> usize {
        for step in 1..=MAX_SETTLE_STEPS {
            self.step();
            if step >= MIN_SETTLE_STEPS && ids.iter().all(|&id| !self.awake(id)) {
                return step;
            }
        }
        panic!("harness: bodies {ids:?} did not latch asleep within {MAX_SETTLE_STEPS} steps");
    }

    /// Removes `RigidBody` from `entity` through `Commands`, applied by a one-system
    /// schedule between physics steps.
    fn remove_rigid_body(&mut self, entity: Entity) {
        let mut builder = ScheduleBuilder::new(serial_pool());
        builder.add_system(move |mut commands: Commands| {
            commands.entity(entity).remove::<RigidBody>();
        });
        let mut schedule = builder.build(&mut self.world);
        schedule.run(&mut self.world);
    }
}

// ── The support-loss scene ───────────────────────────────────────────────────

/// What is done to the support S.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    DeleteEntity,
    RemoveRigidBody,
    DisableSimulated,
    TeleportSupport,
}

/// Where S sits in the archetype.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    /// S is the archetype's last row: removing it moves no other row.
    SupportLast,
    /// S is row 1: removing it swap-moves the last row (L) into row 1.
    SupportFirst,
}

#[derive(Debug)]
struct Outcome {
    change: Change,
    order: Order,
    explicit_wake: bool,
    settle_steps: usize,
    upper_contacts_before: Vec<u32>,
    walk_before: Vec<u32>,
    walk_after: Vec<u32>,
    upper_y_before: f32,
    upper_y_after: f32,
    upper_awake_first_step: bool,
    upper_awake_steps: usize,
    upper_contacts_after_first_step: Vec<u32>,
    bystanders_woken_first_step: Vec<u32>,
}

impl Outcome {
    /// How far U fell, in metres: positive is downwards.
    fn dropped(&self) -> f32 {
        self.upper_y_before - self.upper_y_after
    }
}

/// Floor + S under U + B1 under B2 + lone L; settle until all are latched; apply
/// `change` to S; optionally `wake_all`; run `POST_STEPS` steps.
fn support_scene(change: Change, order: Order, explicit_wake: bool) -> Outcome {
    let mut h = Harness::new();
    h.spawn(floor());
    let (support, upper) = match order {
        Order::SupportLast => {
            h.spawn(ball(LONE, Vec3::new(-6.0, 0.5, 0.0)));
            h.spawn(ball(BY1, Vec3::new(3.0, 0.5, 0.0)));
            h.spawn(ball(BY2, Vec3::new(3.0, 1.5, 0.0)));
            let upper = h.spawn(ball(UPPER, Vec3::new(0.0, 1.5, 0.0)));
            let support = h.spawn(ball(SUPPORT, Vec3::new(0.0, 0.5, 0.0)));
            (support, upper)
        }
        Order::SupportFirst => {
            let support = h.spawn(ball(SUPPORT, Vec3::new(0.0, 0.5, 0.0)));
            let upper = h.spawn(ball(UPPER, Vec3::new(0.0, 1.5, 0.0)));
            h.spawn(ball(BY1, Vec3::new(3.0, 0.5, 0.0)));
            h.spawn(ball(BY2, Vec3::new(3.0, 1.5, 0.0)));
            h.spawn(ball(LONE, Vec3::new(-6.0, 0.5, 0.0)));
            (support, upper)
        }
    };
    let walk_before = h.walk_ids();
    let expected_before = match order {
        Order::SupportLast => vec![FLOOR, LONE, BY1, BY2, UPPER, SUPPORT],
        Order::SupportFirst => vec![FLOOR, SUPPORT, UPPER, BY1, BY2, LONE],
    };
    assert_eq!(
        walk_before, expected_before,
        "construction: walk order before the change"
    );

    let settle_steps = h.settle_until_latched(&TRACKED);
    h.assert_walk_matches_gather();
    let upper_contacts_before = h.contact_ids(UPPER);
    assert_eq!(
        upper_contacts_before,
        vec![SUPPORT],
        "construction: U must rest on S and touch nothing else before the change"
    );
    let upper_y_before = h.body(upper).position.y;

    match change {
        Change::DeleteEntity => assert!(
            h.world.delete_entity(support),
            "construction: S must be despawnable"
        ),
        Change::RemoveRigidBody => h.remove_rigid_body(support),
        Change::DisableSimulated => h.world.disable::<Simulated>(support),
        Change::TeleportSupport => {
            let mut body = h
                .world
                .get_component_mut::<RigidBody>(support)
                .expect("construction: S is live");
            body.position = Vec3::new(0.0, 0.5, 10.0);
        }
    }
    let walk_after = h.walk_ids();
    let expected_after = match (change, order) {
        (Change::DisableSimulated | Change::TeleportSupport, _) => expected_before.clone(),
        (_, Order::SupportLast) => vec![FLOOR, LONE, BY1, BY2, UPPER],
        (_, Order::SupportFirst) => vec![FLOOR, LONE, UPPER, BY1, BY2],
    };
    assert_eq!(
        walk_after, expected_after,
        "construction: walk order after the change"
    );
    if explicit_wake {
        h.world.resource_mut::<IslandSleep>().wake_all();
    }

    h.step();
    let upper_awake_first_step = h.awake(UPPER);
    let upper_contacts_after_first_step = h.contact_ids(UPPER);
    let bystanders_woken_first_step: Vec<u32> = [BY1, BY2, LONE]
        .into_iter()
        .filter(|&id| h.awake(id))
        .collect();
    let mut upper_awake_steps = usize::from(upper_awake_first_step);
    for _ in 1..POST_STEPS {
        h.step();
        upper_awake_steps += usize::from(h.awake(UPPER));
    }

    Outcome {
        change,
        order,
        explicit_wake,
        settle_steps,
        upper_contacts_before,
        walk_before,
        walk_after,
        upper_y_before,
        upper_y_after: h.body(upper).position.y,
        upper_awake_first_step,
        upper_awake_steps,
        upper_contacts_after_first_step,
        bystanders_woken_first_step,
    }
}

/// Runs `support_scene` under the watchdog.
fn scene(change: Change, order: Order, explicit_wake: bool) -> Outcome {
    let what = format!("{change:?} / {order:?} / explicit wake_all: {explicit_wake}");
    under_watchdog(&what, move || support_scene(change, order, explicit_wake))
}

fn assert_upper_fell(o: &Outcome) {
    assert!(
        o.dropped() > MIN_DROP,
        "support loss ({:?}, {:?}, explicit wake_all: {}): body U, latched asleep on support S \
         after {} steps (U's contacts before: {:?}), must drop more than {MIN_DROP} m in \
         {POST_STEPS} steps; y before = {}, y after = {} (drop {}). U's row awake on the first step \
         after the change: {}; awake on {}/{POST_STEPS} steps; U's contacts after that step: {:?}; \
         bystanders (B1 = {BY1}, B2 = {BY2}, L = {LONE}) awake on that step: {:?}; walk before {:?}, \
         after {:?}.",
        o.change,
        o.order,
        o.explicit_wake,
        o.settle_steps,
        o.upper_contacts_before,
        o.upper_y_before,
        o.upper_y_after,
        o.dropped(),
        o.upper_awake_first_step,
        o.upper_awake_steps,
        o.upper_contacts_after_first_step,
        o.bystanders_woken_first_step,
        o.walk_before,
        o.walk_after
    );
}

// ── (a) delete the support ───────────────────────────────────────────────────

/// (a) S deleted while it is the last row, so no other row moves.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              660 solver steps are intractable under Miri"
)]
fn a_deleting_the_support_wakes_the_body_it_carried() {
    assert_upper_fell(&scene(Change::DeleteEntity, Order::SupportLast, false));
}

/// (a) S deleted from row 1, so L swap-moves into S's row (the row identity map carries
/// L's latch).
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              660 solver steps are intractable under Miri"
)]
fn a_deleting_the_support_with_a_row_move_wakes_the_body_it_carried() {
    assert_upper_fell(&scene(Change::DeleteEntity, Order::SupportFirst, false));
}

/// I1 (a): the wake lands on the FIRST step after S is deleted, not merely within the 30
/// steps the drop assertion allows.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              660 solver steps are intractable under Miri"
)]
fn a_the_wake_lands_on_the_first_step_after_the_support_is_deleted() {
    let o = scene(Change::DeleteEntity, Order::SupportLast, false);
    assert!(
        o.upper_awake_first_step,
        "a wake one step late passes the 30-step drop but not this: U's row must be awake on the \
         first step after S is deleted; awake on {}/{POST_STEPS} steps, drop {} m, U's contacts \
         before {:?} and after that step {:?}, walk before {:?}, after {:?}",
        o.upper_awake_steps,
        o.dropped(),
        o.upper_contacts_before,
        o.upper_contacts_after_first_step,
        o.walk_before,
        o.walk_after
    );
}

// ── (b) the support leaves the physics query ─────────────────────────────────

/// (b) `RigidBody` removed from S: S leaves both the gather and the apply walks.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              660 solver steps are intractable under Miri"
)]
fn b_removing_the_supports_rigid_body_wakes_the_body_it_carried() {
    assert_upper_fell(&scene(Change::RemoveRigidBody, Order::SupportLast, false));
}

// ── (d) move the support away ────────────────────────────────────────────────

/// (d) S's `RigidBody::position` written 10 m away along `z`, onto the floor. A support
/// loss with no structural change at all: no row moves and no component is removed, so a
/// fix keyed only on despawn or removal leaves this one red.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              660 solver steps are intractable under Miri"
)]
fn d_teleporting_the_support_away_wakes_the_body_it_carried() {
    assert_upper_fell(&scene(Change::TeleportSupport, Order::SupportLast, false));
}

// ── (e) a box pile: the support lowered by less than τ_eff, contact reuse on ─────

/// Half-extent of the box-pile scene's cubes.
#[cfg(not(miri))]
const CUBE_HALF: f32 = 0.5;
/// The contact-reuse distance τ of the box-pile scene: 2 mm, one of G-L9b-6's readout
/// distances. Both box pairs (cube on cube, cube on the 20 m floor) have τ_eff = τ, since the
/// radius and half-extent clamps are 5 % of 0.866 m and of 0.5 m.
///
/// Not the 1 mm default. The settled pile's S-U points sit up to 0.45 mm deep (read on this
/// scene, 2026-09-23), so a drop must exceed that to lift them all. A hit allows at most
/// |Δd| = τ_eff/√2, counted from the pose its record was built at: 0.71 mm at τ = 1 mm. That
/// leaves a window too thin on both sides.
#[cfg(not(miri))]
const BOX_PILE_REUSE_DISTANCE: f32 = 0.002;
/// How far (e) lowers the box support: half of τ_eff. That is below τ_eff, as ruling W2 asks.
/// It is deeper than the pile's resting S-U penetration, which the test asserts as a premise.
/// And it leaves 0.41 mm of the criterion's τ_eff/√2 for the drift since the record was built.
#[cfg(not(miri))]
const BOX_SUPPORT_DROP: f32 = 0.5 * BOX_PILE_REUSE_DISTANCE;

/// A unit-mass cube of half-extent [`CUBE_HALF`] at rest.
#[cfg(not(miri))]
fn cube(id: u32, position: Vec3) -> Spec {
    Spec {
        id,
        position,
        shape: ColliderShape::Box {
            half_extents: Vec3::new(CUBE_HALF, CUBE_HALF, CUBE_HALF),
        },
        inv_mass: 1.0,
    }
}

/// What the box-pile scene observed.
#[cfg(not(miri))]
#[derive(Debug)]
struct BoxPileOutcome {
    settle_steps: usize,
    /// The narrowphase's pair classes on the last settle step and on the step after the drop.
    classes_latched: PairClasses,
    classes_after_drop: PairClasses,
    /// The deepest penetration of the S-U manifold's points on the last settle step.
    upper_depth_latched: f32,
    upper_y_before: f32,
    upper_awake_first_step: bool,
    upper_contacts_after_first_step: Vec<u32>,
    upper_awake_steps: usize,
    upper_y_after: f32,
    upper_contacts_after: Vec<u32>,
}

/// Floor + cube S under cube U, contact reuse forced on, L10's sleep-skip `mode`; settle until
/// both are latched; lower S by [`BOX_SUPPORT_DROP`]; run `POST_STEPS` steps.
#[cfg(not(miri))]
fn box_pile_scene(mode: SleepSkip) -> BoxPileOutcome {
    let mut h = Harness::new();
    {
        let cfg = h.world.resource_mut::<PhysicsConfig>();
        cfg.contact_reuse = true;
        cfg.contact_reuse_distance = BOX_PILE_REUSE_DISTANCE;
        cfg.sleep_skip = mode;
    }
    h.spawn(floor());
    let support = h.spawn(cube(SUPPORT, Vec3::new(0.0, CUBE_HALF, 0.0)));
    let upper = h.spawn(cube(UPPER, Vec3::new(0.0, 3.0 * CUBE_HALF, 0.0)));
    assert_eq!(
        h.walk_ids(),
        vec![FLOOR, SUPPORT, UPPER],
        "construction: walk order of the box pile"
    );

    let settle_steps = h.settle_until_latched(&[SUPPORT, UPPER]);
    h.assert_walk_matches_gather();
    let classes_latched = h.world.resource::<Manifolds>().pair_classes();
    assert_eq!(
        h.contact_ids(UPPER),
        vec![SUPPORT],
        "construction: U must rest on S and touch nothing else before the drop"
    );
    assert_eq!(
        h.contact_ids(SUPPORT),
        vec![FLOOR, UPPER],
        "construction: S must rest on the floor and carry U before the drop"
    );
    let upper_y_before = h.body(upper).position.y;
    let upper_depth_latched = {
        let pair = [h.row_of(SUPPORT) as u32, h.row_of(UPPER) as u32];
        h.world
            .resource::<Manifolds>()
            .manifolds()
            .iter()
            .filter(|m| pair.contains(&m.body_a.0) && pair.contains(&m.body_b.0))
            .flat_map(|m| m.points.into_iter().take(usize::from(m.count)))
            .fold(0.0f32, |d, p| d.max(-p.separation))
    };

    {
        let mut body = h
            .world
            .get_component_mut::<RigidBody>(support)
            .expect("construction: S is live");
        body.position.y -= BOX_SUPPORT_DROP;
    }
    assert_eq!(
        h.walk_ids(),
        vec![FLOOR, SUPPORT, UPPER],
        "construction: a pose write moves no row"
    );

    h.step();
    let classes_after_drop = h.world.resource::<Manifolds>().pair_classes();
    let upper_awake_first_step = h.awake(UPPER);
    let upper_contacts_after_first_step = h.contact_ids(UPPER);
    let mut upper_awake_steps = usize::from(upper_awake_first_step);
    for _ in 1..POST_STEPS {
        h.step();
        upper_awake_steps += usize::from(h.awake(UPPER));
    }

    BoxPileOutcome {
        settle_steps,
        classes_latched,
        classes_after_drop,
        upper_depth_latched,
        upper_y_before,
        upper_awake_first_step,
        upper_contacts_after_first_step,
        upper_awake_steps,
        upper_y_after: h.body(upper).position.y,
        upper_contacts_after: h.contact_ids(UPPER),
    }
}

/// (e) L9 contact reuse, ruling W2 (`docs/physics/perf-campaign/levers/00-RULINGS.md`): a
/// latched box pile whose support S is lowered by half of τ_eff must wake the box U it
/// carried. With contact reuse on, both box pairs are served from their records on that step
/// (the drop is inside what the criterion accepts). The drop is deeper than U's resting
/// penetration, so every point of U's record lifts: the refresh keeps none (design D6), the
/// pair has no manifold, the island's count changes, and U wakes. A refresh that kept the
/// lifted points, or a hit that kept its manifold, leaves U latched above S with the count
/// unchanged. This test turns red then; the sphere scenes above cannot, since contact reuse
/// touches box pairs only.
///
/// Contact reuse is set on here (the default since L9 C4, off before it), with its distance
/// set by [`BOX_PILE_REUSE_DISTANCE`], so the arm is the same before and after C4.
///
/// L10 C3b: the scene runs the sleep-skip `Off` — the oracle — because its premises read the
/// narrowphase's per-slot classes on the latched step, and under `Sets` the latched pile's
/// pairs are held, not collided, so their slots carry the held-skip tag by design (L10 design
/// 06 §7: slot tags are not claimed on held slots). [`e_sets_twin_matches_off`] runs the scene
/// under `Sets` and holds it to this arm's outcome.
/// `cfg(not(miri))` for the same reason as the scenes above: the real schedule on a thread
/// pool is intractable under Miri.
#[test]
#[cfg(not(miri))]
fn e_lowering_a_box_support_by_less_than_tau_eff_wakes_the_box_it_carried() {
    let o = under_watchdog(
        "box pile: S lowered by half of tau_eff, contact reuse on",
        || box_pile_scene(SleepSkip::Off),
    );
    assert!(
        o.upper_depth_latched < BOX_SUPPORT_DROP,
        "premise: U's deepest resting point on S must be shallower than the {BOX_SUPPORT_DROP} m \
         drop, or the drop lifts no point off and nothing is tested; it sits {} m deep",
        o.upper_depth_latched
    );
    println!(
        "(e) box pile: latched after {} steps, U {} m deep on S, drop {BOX_SUPPORT_DROP} m; pair \
         classes latched {:?}, after the drop {:?}; U awake on {}/{POST_STEPS} steps, y {} -> {}",
        o.settle_steps,
        o.upper_depth_latched,
        o.classes_latched,
        o.classes_after_drop,
        o.upper_awake_steps,
        o.upper_y_before,
        o.upper_y_after
    );
    let served_by_records = |c: &PairClasses| c.reused == 2 && c.full == 0;
    assert!(
        served_by_records(&o.classes_latched),
        "premise: on the step the pile latched (after {} steps), both box contacts (floor-S and \
         S-U) must be served from their records, or the pile is not on the reuse path this test \
         is about; pair classes {:?}",
        o.settle_steps,
        o.classes_latched
    );
    assert!(
        served_by_records(&o.classes_after_drop),
        "premise: on the step after S is lowered by {BOX_SUPPORT_DROP} m, both box pairs must \
         still hit their records, so that S-U losing its manifold is the refresh dropping lifted \
         points (D6) and not a full collision finding a gap; pair classes {:?}",
        o.classes_after_drop
    );
    assert!(
        o.upper_awake_first_step,
        "support moved by less than tau_eff, contact reuse on: U's row must be awake on the first \
         step after S is lowered by {BOX_SUPPORT_DROP} m, since every point of U's contact lifted \
         and the island's manifold count changed; U's contacts on that step: {:?} (S = {SUPPORT} \
         must not be among them); awake on {}/{POST_STEPS} steps; pair classes on that step {:?}",
        o.upper_contacts_after_first_step, o.upper_awake_steps, o.classes_after_drop
    );
    assert!(
        !o.upper_contacts_after_first_step.contains(&SUPPORT),
        "all of U's points lifted by {BOX_SUPPORT_DROP} m, so U and S must have no manifold on \
         the first step after the drop (D6); U's contacts on that step: {:?}",
        o.upper_contacts_after_first_step
    );
    assert!(
        o.upper_contacts_after == [SUPPORT]
            && (o.upper_y_after - o.upper_y_before).abs() <= MAX_CARRIED_DRIFT,
        "after the wake U must land back on S within {POST_STEPS} steps: U's contacts {:?}, \
         y before = {}, y after = {} (bound {MAX_CARRIED_DRIFT} m)",
        o.upper_contacts_after,
        o.upper_y_before,
        o.upper_y_after
    );
}

/// (e) under L10's sleep-skip `Sets` (design 06 B4, "a support moved by less than τ_eff behaves
/// exactly as in Off"): the latched pile is HELD — every one of its three pairs skipped for a
/// held endpoint on the latched step — and lowering S restores it; the restore computes the
/// two box pairs from the kept reuse records, so on the step after the drop they hit exactly as
/// under `Off`, and every observable of the scene equals the `Off` run's.
#[test]
#[cfg(not(miri))]
fn e_sets_twin_matches_off() {
    let off = under_watchdog("box pile, sleep-skip Off", || box_pile_scene(SleepSkip::Off));
    let sets = under_watchdog("box pile, sleep-skip Sets", || box_pile_scene(SleepSkip::Sets));
    println!("(e) Sets twin: latched classes {:?}, after the drop {:?}", sets.classes_latched, sets.classes_after_drop);
    assert_eq!(
        (sets.classes_latched.held_skipped, sets.classes_latched.pairs),
        (3, 3),
        "anti-vacuity: under Sets the latched pile's three pairs must be held on the latched step"
    );
    assert_eq!(sets.classes_after_drop, off.classes_after_drop, "the step after the drop collides as Off does");
    assert_eq!(sets.settle_steps, off.settle_steps, "settle steps");
    assert_eq!(sets.upper_depth_latched.to_bits(), off.upper_depth_latched.to_bits(), "U's resting depth");
    assert_eq!(sets.upper_y_before.to_bits(), off.upper_y_before.to_bits(), "U's height before the drop");
    assert_eq!(sets.upper_awake_first_step, off.upper_awake_first_step, "U awake on the first step");
    assert_eq!(sets.upper_contacts_after_first_step, off.upper_contacts_after_first_step, "U's contacts then");
    assert_eq!(sets.upper_awake_steps, off.upper_awake_steps, "U's awake steps");
    assert_eq!(sets.upper_y_after.to_bits(), off.upper_y_after.to_bits(), "U's height after");
    assert_eq!(sets.upper_contacts_after, off.upper_contacts_after, "U's contacts after");
}

// ── (c) guard: a parked support is not a lost support ────────────────────────

/// (c) `Simulated` disabled on S: S stays in the gather, parked, and MUST keep carrying U.
///
/// This is the engine's documented model, not an oversight: `physics_build_graph` treats a
/// row as dynamic by `inv_mass` alone and the broadphase pairs every body, so a parked
/// support remains an island member and a collider.
/// [`Simulated`](boyko_physics::components::Simulated) says exactly that — the pose is
/// frozen in place while the solve may still apply impulses. Box2D's `b2Body_Disable`
/// makes the opposite choice (it destroys the contacts and wakes both bodies); a fix for
/// this defect must not silently import it.
///
/// The scene wakes everything explicitly first, so U is demonstrably awake while it is
/// being carried. Without that, a green here would be the freeze this file is about,
/// re-badged as a pass.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              660 solver steps are intractable under Miri"
)]
fn c_a_parked_support_keeps_carrying_the_body_it_holds() {
    let o = scene(Change::DisableSimulated, Order::SupportLast, true);
    assert!(
        o.upper_awake_first_step && o.upper_awake_steps > 0,
        "construction: after an explicit wake_all, U must actually be simulated, or this guard \
         would pass out of the freeze instead of out of the contact; awake on the first step: {}, \
         awake on {}/{POST_STEPS} steps",
        o.upper_awake_first_step,
        o.upper_awake_steps
    );
    assert!(
        o.dropped().abs() <= MAX_CARRIED_DRIFT
            && o.upper_contacts_after_first_step.contains(&SUPPORT),
        "parked support: `Simulated` off is documented as 'frozen in place', NOT as a support \
         loss, so U (awake on {}/{POST_STEPS} steps) must stay on S and move no more than \
         {MAX_CARRIED_DRIFT} m; y before = {}, y after = {} (drop {}); U's contacts after the \
         first step: {:?} (must contain S = {SUPPORT}); walk after the change: {:?}.",
        o.upper_awake_steps,
        o.upper_y_before,
        o.upper_y_after,
        o.dropped(),
        o.upper_contacts_after_first_step,
        o.walk_after
    );
}

// ── Attribution: the same changes with an explicit global wake ───────────────

/// (a) + `wake_all`: GREEN means (a)'s red is the missing wake alone.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              660 solver steps are intractable under Miri"
)]
fn attribution_a_delete_falls_after_an_explicit_wake_all() {
    assert_upper_fell(&scene(Change::DeleteEntity, Order::SupportLast, true));
}

/// (b, `RigidBody`) + `wake_all`.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              660 solver steps are intractable under Miri"
)]
fn attribution_b_remove_rigid_body_falls_after_an_explicit_wake_all() {
    assert_upper_fell(&scene(Change::RemoveRigidBody, Order::SupportLast, true));
}

/// (d) + `wake_all`.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              660 solver steps are intractable under Miri"
)]
fn attribution_d_teleport_falls_after_an_explicit_wake_all() {
    assert_upper_fell(&scene(Change::TeleportSupport, Order::SupportLast, true));
}

// ── Control: the removed body touched no dynamic body ────────────────────────

/// Where L sits in the archetype in the control scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoneOrder {
    /// L is the last row: deleting it moves no other row.
    LoneLast,
    /// L is row 1 and S the last row: deleting L swap-moves S into row 1, and the island
    /// ids renumber (L's singleton island was id 0).
    LoneFirst,
}

/// What the control scene observed.
#[derive(Debug)]
struct ControlOutcome {
    order: LoneOrder,
    settle_steps: usize,
    lone_contacts: Vec<u32>,
    walk_before: Vec<u32>,
    walk_after: Vec<u32>,
    /// U's island id on the last settle step and on the first step after the delete.
    upper_island_before: u32,
    upper_island_after: u32,
    /// `(step, body id)` for every watched body awake on any post-delete step.
    woken: Vec<(usize, u32)>,
    /// Watched bodies whose position changed by a single bit.
    moved: Vec<u32>,
}

/// Floor + S under U + B1 under B2 + L; settle; delete L, whose only contact is the
/// static floor. `order` decides whether L is the last row (`LoneLast`: floor, S, U, B1, B2,
/// L) or row 1 (`LoneFirst`: floor, L, B1, B2, U, S).
fn control_scene(order: LoneOrder) -> ControlOutcome {
    let mut h = Harness::new();
    h.spawn(floor());
    let lone_position = Vec3::new(-6.0, 0.5, 0.0);
    let (s, u, b1, b2, lone) = match order {
        LoneOrder::LoneLast => {
            let s = h.spawn(ball(SUPPORT, Vec3::new(0.0, 0.5, 0.0)));
            let u = h.spawn(ball(UPPER, Vec3::new(0.0, 1.5, 0.0)));
            let b1 = h.spawn(ball(BY1, Vec3::new(3.0, 0.5, 0.0)));
            let b2 = h.spawn(ball(BY2, Vec3::new(3.0, 1.5, 0.0)));
            let lone = h.spawn(ball(LONE, lone_position));
            (s, u, b1, b2, lone)
        }
        LoneOrder::LoneFirst => {
            let lone = h.spawn(ball(LONE, lone_position));
            let b1 = h.spawn(ball(BY1, Vec3::new(3.0, 0.5, 0.0)));
            let b2 = h.spawn(ball(BY2, Vec3::new(3.0, 1.5, 0.0)));
            let u = h.spawn(ball(UPPER, Vec3::new(0.0, 1.5, 0.0)));
            let s = h.spawn(ball(SUPPORT, Vec3::new(0.0, 0.5, 0.0)));
            (s, u, b1, b2, lone)
        }
    };
    let walk_before = h.walk_ids();
    let (expected_before, expected_after) = match order {
        LoneOrder::LoneLast => (
            vec![FLOOR, SUPPORT, UPPER, BY1, BY2, LONE],
            vec![FLOOR, SUPPORT, UPPER, BY1, BY2],
        ),
        LoneOrder::LoneFirst => (
            vec![FLOOR, LONE, BY1, BY2, UPPER, SUPPORT],
            vec![FLOOR, SUPPORT, BY1, BY2, UPPER],
        ),
    };
    assert_eq!(
        walk_before, expected_before,
        "construction: walk order before the delete ({order:?})"
    );
    let settle_steps = h.settle_until_latched(&TRACKED);
    h.assert_walk_matches_gather();
    let lone_contacts = h.contact_ids(LONE);
    assert_eq!(
        lone_contacts,
        vec![FLOOR],
        "construction: L must touch only the static floor"
    );
    assert_eq!(
        h.contact_ids(UPPER),
        vec![SUPPORT],
        "construction: U must rest on S (the pile the control watches is a real contact pile)"
    );
    let watched = [(SUPPORT, s), (UPPER, u), (BY1, b1), (BY2, b2)];
    let poses_before: Vec<Vec3> = watched.iter().map(|&(_, e)| h.body(e).position).collect();
    let upper_island_before = {
        let row = h.row_of(UPPER) as u32;
        h.world.resource::<ConstraintGraph>().island_of(row)
    };

    assert!(
        h.world.delete_entity(lone),
        "construction: L must be despawnable"
    );
    let walk_after = h.walk_ids();
    assert_eq!(
        walk_after, expected_after,
        "construction: walk order after the delete ({order:?}; deleting the last row moves \
         nobody, deleting row 1 swap-moves the last row into it)"
    );

    let mut woken: Vec<(usize, u32)> = Vec::new();
    let mut upper_island_after = ConstraintGraph::NO_ISLAND;
    for step in 1..=POST_STEPS {
        h.step();
        if step == 1 {
            let row = h.row_of(UPPER) as u32;
            upper_island_after = h.world.resource::<ConstraintGraph>().island_of(row);
        }
        for &(id, _) in &watched {
            if h.awake(id) {
                woken.push((step, id));
            }
        }
    }
    let moved: Vec<u32> = watched
        .iter()
        .zip(&poses_before)
        .filter(|&(&(_, e), before)| !bits_eq(h.body(e).position, *before))
        .map(|(&(id, _), _)| id)
        .collect();
    ControlOutcome {
        order,
        settle_steps,
        lone_contacts,
        walk_before,
        walk_after,
        upper_island_before,
        upper_island_after,
        woken,
        moved,
    }
}

/// The no-wake assertion both controls share.
fn assert_nobody_woke(o: &ControlOutcome) {
    assert!(
        o.woken.is_empty() && o.moved.is_empty(),
        "control ({:?}): deleting L (contacts before: {:?}, no dynamic body) must wake nobody; \
         (step, id) awake within {POST_STEPS} steps: {:?}; bodies whose position changed: {:?}; \
         latched after {} steps; walk before {:?}, after {:?}; U's island id before {}, after {}.",
        o.order,
        o.lone_contacts,
        o.woken,
        o.moved,
        o.settle_steps,
        o.walk_before,
        o.walk_after,
        o.upper_island_before,
        o.upper_island_after
    );
}

/// No latched body may be awake on any of the next `POST_STEPS` steps, and none may move.
/// A fix that wakes everything on any removal turns this red — which is measured, not
/// assumed: adding `wake_all()` right after the delete made it fail with
/// `(step, id) awake within 30 steps: [(1, 1), (1, 2), (1, 3), (1, 4), ...]`.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              660 solver steps are intractable under Miri"
)]
fn control_deleting_a_body_that_touched_no_dynamic_body_wakes_nobody() {
    let o = under_watchdog("control: delete the lone body", || {
        control_scene(LoneOrder::LoneLast)
    });
    assert_nobody_woke(&o);
}

/// I2: the same delete, with L in row 1. S swap-moves into L's row and every island id
/// renumbers (L's singleton island was id 0), while no latched island's manifold count
/// changes: nobody may wake. A key built from the island id, or a key that does not follow
/// S across the row move, turns this red.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              660 solver steps are intractable under Miri"
)]
fn control_deleting_a_lone_body_that_moves_a_row_wakes_nobody() {
    let o = under_watchdog("control: delete the lone body from row 1", || {
        control_scene(LoneOrder::LoneFirst)
    });
    assert!(
        o.upper_island_before != o.upper_island_after
            && o.upper_island_after != ConstraintGraph::NO_ISLAND,
        "premise: the delete must renumber U's island, or this control does not test a \
         renumbering; U's island id before {}, after {}",
        o.upper_island_before,
        o.upper_island_after
    );
    assert_nobody_woke(&o);
}
