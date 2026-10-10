//! Defect A — ROW-KEYED physics state under structural change, RED-FIRST gates.
//!
//! Source: `docs/unification/checkpoint-2026-09-11/latent-defects.md`, section A. That
//! verdict was reached by reading code only; this file is the reproduction.
//!
//! # The mechanism
//!
//! A body's solver row is its position in `physics_gather`'s walk over the matching
//! archetypes. A despawn swap-removes (the archetype's last row moves into the hole), a
//! spawn appends, and a component insert / remove migrates the body to another archetype
//! (so every row of an archetype walked later shifts by one). The per-row state that
//! survives between steps is NOT re-keyed when that happens:
//!
//! - `IslandSleep`'s latch (`asleep` / `below_count`) is only resized
//!   (`IslandSleep::sync_rows`), so a body that lands in another body's row inherits that
//!   body's sleep latch;
//! - the warm-start table is keyed by `(row_a, row_b, feature_id)`, so a body that lands
//!   in another body's row reads that body's stored contact impulses.
//!
//! # What each test pins
//!
//! | test | case |
//! |---|---|
//! | `a1_delete_then_spawn_*` | a NEW body appended into a row whose latch is set hangs mid-air |
//! | `a1_spawn_then_delete_*` | the same, with the new body swap-moved into the deleted row |
//! | `a1c_slow_existing_body_*` | an OLD body at the apex of a throw (speed below the sleep threshold) swap-moved into a latched row hangs; `Ref<RigidBody>::is_added` does not see it |
//! | `a1c_control_*` | the same body falls when the same delete does not move its row (GREEN today: proves the red above is the row move) |
//! | `a3_*` | a component remove in the archetype walked first shifts every later row: an awake faller freezes for a step, and latched bodies wake |
//! | `a2_*` | deleting an unrelated awake body wakes a latched pile |
//! | `warm_start_*` | a cube survivor moved into a deleted cube carrier's row reads the carrier's four warm-start impulses and leaves the step with a velocity far outside resting noise |
//!
//! Every assertion follows ENTITIES, not rows: each body carries a [`BodyId`], the test's
//! row map walks the same archetypes in the same order as the gather, and
//! [`Harness::assert_walk_matches_gather`] checks that map against the solver's own
//! `SolverScratch::bodies()` snapshot before the structural change is made.
//!
//! Device-free; drives the real colored physics `Schedule`. Spins up `boyko_threadpool`
//! (intractable under Miri), so `cfg(not(miri))` like `sleeping_pipeline_o8.rs`.

#![cfg(not(miri))]

use std::sync::Arc;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{Query, Ref};
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::{Commands, ResMut};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_macros::{Component, Resource};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{
    DEFAULT_SLEEP_THRESHOLD, IslandSleep, PhysicsConfig, SolverScratch,
};
use boyko_physics::solver::ColoredSoftStepSolver;

// ── Scene constants ──────────────────────────────────────────────────────────

/// The fixed step (60 Hz), as in `sleeping_pipeline_o8.rs`.
const DT: f32 = 1.0 / 60.0;
/// Brisk debounce so a pile latches within the step budget (the brief's value).
const SLEEP_FRAMES: u16 = 8;
/// The verdict's settle budget: the latch is checked only after this many steps.
const MIN_SETTLE_STEPS: usize = 120;
/// Upper bound on the settle loop; exceeding it is a harness failure, not the defect.
const MAX_SETTLE_STEPS: usize = 600;
/// A static floor box whose top face is `y = 0` and which spans `x, z` in `[-20, 20]`.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(20.0, 0.5, 20.0);

/// Body identities carried in [`BodyId`].
const FLOOR: u32 = 0;
const P1: u32 = 1;
const P2: u32 = 2;
const P3: u32 = 3;
const P4: u32 = 4;
/// The pile: P1 under P2 at `x = 0`, P3 under P4 at `x = 2`.
const PILE: [u32; 4] = [P1, P2, P3, P4];
/// A body that falls forever beside the floor (always awake).
const FALLER: u32 = 5;
/// The body spawned after the pile has latched (A1).
const NEWCOMER: u32 = 6;
/// The body thrown upward at spawn so it is at its apex when the row move happens (A1c).
const APEX: u32 = 7;
/// A body spawned in the same window as the A1c row move (the probe's anti-vacuity).
const SENTINEL: u32 = 8;
/// A resting body spawned after the apex body in the A1c control, so the delete moves it
/// instead of the apex body.
const TAIL: u32 = 9;
/// The body whose component remove shifts the rows (A3).
const MIGRANT: u32 = 10;
/// The warm-start scene: the body carrying the tower, and the lone survivor.
const CARRIER: u32 = 20;
const SURVIVOR: u32 = 21;
/// First id of the ten tower spheres on the carrier.
const TOWER_BASE: u32 = 30;

// ── Test-only components and resources ───────────────────────────────────────

/// Stable identity of a body, so assertions follow entities rather than rows.
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct BodyId {
    id: u32,
}

/// A data component whose removal migrates a body out of the archetype walked first.
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct Marker {
    _tag: u32,
}

/// The ids whose `RigidBody` a `Ref<RigidBody>::is_added` read reported as added on the
/// last probe run — the exact signal the verdict's partial fix proposes to key on.
#[derive(Resource, Default)]
struct AddedProbe {
    added: Vec<u32>,
}

/// Records which bodies `Ref<RigidBody>::is_added` reports this run. Runs in its own
/// schedule immediately before every physics step, so its `last_run` is the previous
/// step — the same window a pre-`begin_step` pass would see.
//
// `clippy::needless_pass_by_value`: `Query` / `ResMut` are by-value `SystemParam`s,
// the same false positive the physics systems carry.
#[allow(clippy::needless_pass_by_value)]
fn record_added_bodies(query: Query<(Ref<RigidBody>, &BodyId)>, mut probe: ResMut<AddedProbe>) {
    probe.added.clear();
    for (body, id) in query.iter() {
        if body.is_added() {
            probe.added.push(id.id);
        }
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

/// A single-threaded pool (deterministic, IM-2 precondition).
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// What to spawn.
#[derive(Clone, Copy)]
struct Spec {
    id: u32,
    position: Vec3,
    velocity: Vec3,
    shape: ColliderShape,
    inv_mass: f32,
}

/// The static floor box (top face at `y = 0`).
fn floor() -> Spec {
    Spec {
        id: FLOOR,
        position: Vec3::new(0.0, -0.5, 0.0),
        velocity: Vec3::ZERO,
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
        velocity: Vec3::ZERO,
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
    probe: Schedule,
    /// `{RigidBody, RigidBodyMass, Collider, BodyId, Marker}` — created FIRST, so the
    /// gather walks it first.
    marked: ArchetypeId,
    /// `{RigidBody, RigidBodyMass, Collider, BodyId}`.
    plain: ArchetypeId,
}

impl Harness {
    /// The colored physics schedule of `sleeping_pipeline_o8.rs` (sleeping switched by
    /// `sleeping`, `sleep_frames = 8`), plus the `is_added` probe schedule.
    fn new(sleeping: bool) -> Self {
        let mut world = EcsMaster::new();
        let marked = world.create_archetype(&[
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
            BodyId::component_id(),
            Marker::component_id(),
        ]);
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
            cfg.sleeping = sleeping;
            cfg.sleep_frames = SLEEP_FRAMES;
        }

        world.insert_resource(AddedProbe::default());
        let mut probe_builder = ScheduleBuilder::new(serial_pool());
        probe_builder.add_system(record_added_bodies);
        let probe = probe_builder.build(&mut world);

        Self {
            world,
            physics,
            probe,
            marked,
            plain,
        }
    }

    fn spawn(&mut self, spec: Spec, marked: bool) -> Entity {
        let body = RigidBody {
            position: spec.position,
            linear_velocity: spec.velocity,
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
        let marker = Marker { _tag: 0 };
        let created = if marked {
            self.world.create_entity(
                self.marked,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                    (BodyId::component_id(), as_bytes(&id)),
                    (Marker::component_id(), as_bytes(&marker)),
                ],
            )
        } else {
            self.world.create_entity(
                self.plain,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                    (BodyId::component_id(), as_bytes(&id)),
                ],
            )
        };
        let entity = created.expect("construction: the body archetype accepts every column");
        self.world.enable::<Simulated>(entity);
        entity
    }

    /// Two stacks of two spheres on the floor: P1 under P2 at `x = 0`, P3 under P4 at
    /// `x = 2`. Returns the entities in `PILE` order.
    fn spawn_pile(&mut self) -> [Entity; 4] {
        [
            self.spawn(ball(P1, Vec3::new(0.0, 0.5, 0.0)), false),
            self.spawn(ball(P2, Vec3::new(0.0, 1.5, 0.0)), false),
            self.spawn(ball(P3, Vec3::new(2.0, 0.5, 0.0)), false),
            self.spawn(ball(P4, Vec3::new(2.0, 1.5, 0.0)), false),
        ]
    }

    /// One frame: the `is_added` probe, then the physics schedule.
    fn step(&mut self) {
        self.probe.run(&mut self.world);
        self.physics.run(&mut self.world);
    }

    fn steps(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
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

    fn reported_added(&self, id: u32) -> bool {
        self.world.resource::<AddedProbe>().added.contains(&id)
    }

    /// Checks the test's row map against the solver's own snapshot: after a step, the
    /// gather row `i` holds exactly the pose of the body the walk puts at `i`. Call right
    /// after a step and before any structural change.
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

    /// Steps at least `MIN_SETTLE_STEPS`, then until every body in `ids` has a row that
    /// was NOT awake on the last step (its island frozen). Returns the step count.
    fn settle_until_latched(&mut self, ids: &[u32]) -> usize {
        for step in 1..=MAX_SETTLE_STEPS {
            self.step();
            if step >= MIN_SETTLE_STEPS && ids.iter().all(|&id| !self.awake(id)) {
                return step;
            }
        }
        panic!("harness: bodies {ids:?} did not latch asleep within {MAX_SETTLE_STEPS} steps");
    }

    /// Removes `Marker` from `entity` through `Commands` (a real component remove, applied
    /// by a one-system schedule between physics steps).
    fn remove_marker(&mut self, entity: Entity) {
        let mut builder = ScheduleBuilder::new(serial_pool());
        builder.add_system(move |mut commands: Commands| {
            commands.entity(entity).remove::<Marker>();
        });
        let mut schedule = builder.build(&mut self.world);
        schedule.run(&mut self.world);
    }
}

// ── A1: a new body inherits a latch and hangs mid-air ────────────────────────

/// What the A1 scenes observed.
struct NewcomerOutcome {
    y_spawn: f32,
    y_after_30: f32,
    row: usize,
    awake_on_first_step: bool,
    reported_added: bool,
    settle_steps: usize,
}

/// Floor + latched pile; then (`delete_first`) delete P2 and spawn E, or spawn E and
/// delete P2; then 30 steps. P2 is at row 2, never the last row.
fn newcomer_scene(delete_first: bool) -> NewcomerOutcome {
    let mut h = Harness::new(true);
    h.spawn(floor(), false);
    let pile = h.spawn_pile();
    let settle_steps = h.settle_until_latched(&PILE);
    h.assert_walk_matches_gather();
    assert_eq!(
        h.walk_ids(),
        vec![FLOOR, P1, P2, P3, P4],
        "construction: walk order before the change"
    );

    let e_spec = ball(NEWCOMER, Vec3::new(5.0, 4.0, 0.0));
    let e = if delete_first {
        assert!(
            h.world.delete_entity(pile[1]),
            "construction: P2 must be despawnable"
        );
        h.spawn(e_spec, false)
    } else {
        let e = h.spawn(e_spec, false);
        assert!(
            h.world.delete_entity(pile[1]),
            "construction: P2 must be despawnable"
        );
        e
    };
    let expected_walk = if delete_first {
        // P4 swap-moved into P2's row; E appended at P4's old row (row count unchanged).
        vec![FLOOR, P1, P4, P3, NEWCOMER]
    } else {
        // E appended at row 5, then swap-moved into P2's row.
        vec![FLOOR, P1, NEWCOMER, P3, P4]
    };
    assert_eq!(
        h.walk_ids(),
        expected_walk,
        "construction: walk order after the change"
    );

    let y_spawn = h.body(e).position.y;
    h.step();
    let row = h.row_of(NEWCOMER);
    let awake_on_first_step = h.awake(NEWCOMER);
    let reported_added = h.reported_added(NEWCOMER);
    h.steps(29);
    NewcomerOutcome {
        y_spawn,
        y_after_30: h.body(e).position.y,
        row,
        awake_on_first_step,
        reported_added,
        settle_steps,
    }
}

fn assert_newcomer_falls(o: &NewcomerOutcome, order: &str) {
    assert!(
        o.y_after_30 < 3.0,
        "A1 ({order}): body E spawned at rest at y = {} far from everything, in row {} of a latched \
         pile's former member, must fall under gravity; after 30 steps E.y = {} (expected < 3.0). \
         E's row awake on its first step: {}. Ref<RigidBody>::is_added saw E: {}. Pile latched \
         after {} steps.",
        o.y_spawn,
        o.row,
        o.y_after_30,
        o.awake_on_first_step,
        o.reported_added,
        o.settle_steps
    );
}

/// A1, delete then spawn: P4 swap-moves into P2's row, and E is appended at P4's old row,
/// whose latch is still set; the row count is unchanged, so `sync_rows` does nothing.
#[test]
fn a1_delete_then_spawn_new_body_is_not_frozen_by_a_stale_latch() {
    let o = newcomer_scene(true);
    assert_newcomer_falls(&o, "delete then spawn");
}

/// A1, spawn then delete: E is appended, then swap-moved into P2's latched row.
#[test]
fn a1_spawn_then_delete_new_body_is_not_frozen_by_a_stale_latch() {
    let o = newcomer_scene(false);
    assert_newcomer_falls(&o, "spawn then delete");
}

// ── A1c: an OLD, slow body moved into a latched row ──────────────────────────

/// Steps from spawn to the apex of the vertical throw.
const APEX_STEPS: usize = 240;

struct ApexOutcome {
    apex_y: f32,
    y_after_30: f32,
    apex_speed_sq: f32,
    row_before: usize,
    row_after: usize,
    awake_on_first_step: bool,
    apex_reported_added: bool,
}

/// Floor + pile + body C thrown straight up at spawn so that after `APEX_STEPS` steps it
/// is at its apex (speed below the sleep threshold) with its own latch clear, far from
/// everything. Then P2 (row 2) is deleted and a sentinel is spawned, and 30 steps run.
///
/// `with_tail == false`: C is the last row, so the delete swap-moves C into P2's latched
/// row (the defect case). `with_tail == true`: a resting body T is spawned after C, so
/// the SAME delete moves T instead and C keeps its own row (the control).
fn apex_scene(with_tail: bool) -> ApexOutcome {
    let mut h = Harness::new(true);
    h.spawn(floor(), false);
    let pile = h.spawn_pile();

    // The per-substep gravity increment exactly as the solver forms it, times the
    // substep count to the apex, so the upward speed crosses zero at step APEX_STEPS.
    let (gravity_y, substeps) = {
        let cfg = h.world.resource::<PhysicsConfig>();
        (cfg.gravity.y, cfg.substeps.max(1) as usize)
    };
    let dt = h.world.resource::<FixedTime>().delta_secs();
    let dv_substep = -gravity_y * (dt / substeps as f32);
    let v0 = dv_substep * (APEX_STEPS * substeps) as f32;
    let apex_body = h.spawn(
        Spec {
            velocity: Vec3::new(0.0, v0, 0.0),
            ..ball(APEX, Vec3::new(-5.0, 4.0, 0.0))
        },
        false,
    );
    if with_tail {
        h.spawn(ball(TAIL, Vec3::new(-10.0, 0.5, 0.0)), false);
    }

    h.steps(APEX_STEPS);
    h.assert_walk_matches_gather();
    let mut latched = PILE.to_vec();
    if with_tail {
        latched.push(TAIL);
    }
    for &id in &latched {
        assert!(
            !h.awake(id),
            "construction: body {id} must be latched asleep by step {APEX_STEPS}"
        );
    }
    assert!(
        h.awake(APEX),
        "construction: the apex body's own latch must be clear at its apex"
    );
    let c = h.body(apex_body);
    let apex_speed_sq =
        c.linear_velocity.dot(c.linear_velocity) + c.angular_velocity.dot(c.angular_velocity);
    assert!(
        apex_speed_sq < DEFAULT_SLEEP_THRESHOLD,
        "construction: the apex body must be slower than the sleep threshold at the move \
         (speed^2 = {apex_speed_sq}, threshold {DEFAULT_SLEEP_THRESHOLD})"
    );
    let row_before = h.row_of(APEX);

    assert!(
        h.world.delete_entity(pile[1]),
        "construction: P2 must be despawnable"
    );
    h.spawn(ball(SENTINEL, Vec3::new(0.0, 4.0, -50.0)), false);
    let expected_walk = if with_tail {
        vec![FLOOR, P1, TAIL, P3, P4, APEX, SENTINEL]
    } else {
        vec![FLOOR, P1, APEX, P3, P4, SENTINEL]
    };
    assert_eq!(
        h.walk_ids(),
        expected_walk,
        "construction: walk order after the delete"
    );

    let apex_y = h.body(apex_body).position.y;
    h.step();
    let row_after = h.row_of(APEX);
    let awake_on_first_step = h.awake(APEX);
    let apex_reported_added = h.reported_added(APEX);
    // Anti-vacuity for the probe: the body spawned in the same window IS reported.
    assert!(
        h.reported_added(SENTINEL),
        "construction: the is_added probe must see the sentinel spawned in this window"
    );
    h.steps(29);
    ApexOutcome {
        apex_y,
        y_after_30: h.body(apex_body).position.y,
        apex_speed_sq,
        row_before,
        row_after,
        awake_on_first_step,
        apex_reported_added,
    }
}

fn assert_apex_body_falls(o: &ApexOutcome, case: &str) {
    assert!(
        o.y_after_30 < o.apex_y - 1.0,
        "A1c ({case}): body C, spawned {APEX_STEPS} steps earlier and at the apex of its throw \
         (speed^2 = {} < threshold {DEFAULT_SLEEP_THRESHOLD}), moved from row {} to row {}, must fall \
         ~1.24 m in 30 steps; apex y = {}, y after 30 steps = {}. C's row awake on the first step \
         after the move: {}. Ref<RigidBody>::is_added saw C: {}.",
        o.apex_speed_sq,
        o.row_before,
        o.row_after,
        o.apex_y,
        o.y_after_30,
        o.awake_on_first_step,
        o.apex_reported_added
    );
}

/// A1c: a body that is NOT newly added, swap-moved into a latched row while slower than
/// the sleep threshold and with no awake island neighbour, hangs indefinitely. A fix
/// keyed only on `Ref<RigidBody>::is_added` does not cover it: the construction asserts
/// the probe sees the sentinel spawned in the same window, and the message reports what it
/// saw for C.
#[test]
fn a1c_slow_existing_body_moved_into_a_latched_row_is_not_frozen() {
    let o = apex_scene(false);
    assert!(
        !o.apex_reported_added,
        "construction: C must not be reported added (it was spawned {APEX_STEPS} steps earlier)"
    );
    assert_apex_body_falls(&o, "swap-moved into P2's latched row");
}

/// Control for A1c (GREEN today): the identical delete, but a resting tail body is the
/// last row, so the tail moves and C keeps its own row. C falls — so the A1c red is the
/// row move, not "a slow body mid-air sleeps anyway".
#[test]
fn a1c_control_same_body_falls_when_the_delete_does_not_move_its_row() {
    let o = apex_scene(true);
    assert_eq!(
        o.row_before, o.row_after,
        "construction: C keeps its row in the control"
    );
    assert_apex_body_falls(&o, "control, row unchanged");
}

// ── A3: a cross-archetype row shift ──────────────────────────────────────────

struct ShiftOutcome {
    faller_y_before: f32,
    faller_y_after: f32,
    faller_row_before: usize,
    faller_row_after: usize,
    woken: Vec<u32>,
    migrant_still_simulated: bool,
}

/// Archetype `marked` (walked first) holds M resting on the floor; archetype `plain` holds
/// the floor, the pile and an awake faller D. Once M and the pile are latched, `Marker`
/// is removed from M: M migrates to the end of `plain`, and every `plain` row shifts down
/// by one — P1 lands in the static floor's row (latch never set), D lands in P4's row
/// (latch set), M lands in D's row (latch clear). One step is taken.
fn cross_archetype_shift_scene() -> ShiftOutcome {
    let mut h = Harness::new(true);
    let migrant = h.spawn(ball(MIGRANT, Vec3::new(-4.0, 0.5, 0.0)), true);
    h.spawn(floor(), false);
    h.spawn_pile();
    let faller = h.spawn(ball(FALLER, Vec3::new(50.0, 4.0, 0.0)), false);
    assert_eq!(
        h.walk_ids(),
        vec![MIGRANT, FLOOR, P1, P2, P3, P4, FALLER],
        "construction: the marked archetype must be walked first"
    );

    let mut latched = vec![MIGRANT];
    latched.extend_from_slice(&PILE);
    h.settle_until_latched(&latched);
    h.assert_walk_matches_gather();
    assert!(
        h.awake(FALLER),
        "construction: the faller must be awake before the shift"
    );
    let faller_row_before = h.row_of(FALLER);

    h.remove_marker(migrant);
    assert_eq!(
        h.walk_ids(),
        vec![FLOOR, P1, P2, P3, P4, FALLER, MIGRANT],
        "construction: the remove must migrate M to the end of the later archetype"
    );
    let migrant_still_simulated = h.world.is_enabled::<Simulated>(migrant);

    let faller_y_before = h.body(faller).position.y;
    h.step();
    let faller_y_after = h.body(faller).position.y;
    let faller_row_after = h.row_of(FALLER);
    let woken = latched.iter().copied().filter(|&id| h.awake(id)).collect();
    ShiftOutcome {
        faller_y_before,
        faller_y_after,
        faller_row_before,
        faller_row_after,
        woken,
        migrant_still_simulated,
    }
}

/// A3: an awake falling body must not freeze for a step because a component remove in
/// an earlier-walked archetype shifted it into a latched row.
#[test]
fn a3_component_remove_in_first_archetype_does_not_freeze_an_awake_faller() {
    let o = cross_archetype_shift_scene();
    assert!(
        o.faller_y_after < o.faller_y_before,
        "A3: awake faller D (row {} -> {} after M's component remove) must keep falling on the next \
         step; y before = {}, y after = {}. Latched bodies woken by the same shift: {:?}. M still \
         Simulated after migration: {}.",
        o.faller_row_before,
        o.faller_row_after,
        o.faller_y_before,
        o.faller_y_after,
        o.woken,
        o.migrant_still_simulated
    );
}

/// A3: latched bodies must not wake because a component remove in an earlier-walked
/// archetype shifted them into rows whose latch is clear.
#[test]
fn a3_component_remove_in_first_archetype_does_not_wake_latched_bodies() {
    let o = cross_archetype_shift_scene();
    assert!(
        o.woken.is_empty(),
        "A3: bodies latched asleep before M's component remove must still be asleep one step later; \
         awake: {:?} (ids: M = {MIGRANT}, P1..P4 = 1..4). Faller y before/after the step: {} / {}. \
         M still Simulated after migration: {}.",
        o.woken,
        o.faller_y_before,
        o.faller_y_after,
        o.migrant_still_simulated
    );
}

// ── A2: an unrelated despawn wakes a latched pile ────────────────────────────

/// A2: an awake faller D is spawned before the pile; once the pile is latched D is
/// deleted, so P4 swap-moves into D's row (latch clear). After one step every pile body
/// must still be asleep.
#[test]
fn a2_deleting_an_awake_body_does_not_wake_a_latched_pile() {
    let mut h = Harness::new(true);
    h.spawn(floor(), false);
    let faller = h.spawn(ball(FALLER, Vec3::new(50.0, 4.0, 0.0)), false);
    h.spawn_pile();
    assert_eq!(
        h.walk_ids(),
        vec![FLOOR, FALLER, P1, P2, P3, P4],
        "construction: walk order"
    );
    let settle_steps = h.settle_until_latched(&PILE);
    h.assert_walk_matches_gather();
    assert!(
        h.awake(FALLER),
        "construction: the faller must be awake before the delete"
    );

    assert!(
        h.world.delete_entity(faller),
        "construction: the faller must be despawnable"
    );
    assert_eq!(
        h.walk_ids(),
        vec![FLOOR, P4, P1, P2, P3],
        "construction: P4 swap-moved into the faller's row"
    );
    h.step();
    let woken: Vec<u32> = PILE.iter().copied().filter(|&id| h.awake(id)).collect();
    assert!(
        woken.is_empty(),
        "A2: deleting an unrelated awake body must not wake a pile latched asleep after {settle_steps} \
         steps; pile bodies awake on the next step: {woken:?} (P4 now in row {})",
        h.row_of(P4)
    );
}

// ── Warm start: a survivor reads the deleted body's impulse ──────────────────

/// Settle steps before the delete (the 11-sphere tower needs longer than the pile).
const WARM_SETTLE_STEPS: usize = 300;
/// Control-run steps before the delete whose step-to-step variation defines the noise.
const WARM_NOISE_WINDOW: usize = 60;
/// Steps recorded after the delete.
const WARM_POST_STEPS: usize = 20;

/// A unit-mass unit cube at rest (a four-point face-face manifold on the floor).
fn cube(id: u32, position: Vec3) -> Spec {
    Spec {
        shape: ColliderShape::Box {
            half_extents: Vec3::new(0.5, 0.5, 0.5),
        },
        ..ball(id, position)
    }
}

struct WarmRun {
    /// B's state after each of the last `WARM_NOISE_WINDOW` settle steps.
    pre: Vec<RigidBody>,
    /// B's state after each step following the delete; `[0]` is the next step.
    post: Vec<RigidBody>,
    row_before: usize,
    row_after: usize,
}

/// Static floor F; cube carrier A with a ten-sphere tower on it; lone cube survivor B
/// resting on F far away. `moved == false` (control): walk F, B, tower, A — deleting A
/// (the last row) moves nobody. `moved == true`: walk F, A, tower, B — deleting A
/// swap-moves B into A's row, whose floor contact keys `(0, 1, feature)` carry A's
/// stored impulses. Sleeping is OFF so the latch cannot freeze B and mask the kick.
/// `warm_start == false` replaces the solver with
/// `ColoredSoftStepSolver::with_warm_start(false)` (attribution).
fn warm_start_scene(moved: bool, warm_start: bool) -> WarmRun {
    let mut h = Harness::new(false);
    if !warm_start {
        h.world
            .insert_resource(ColoredSoftStepSolver::with_warm_start(false));
    }
    h.spawn(floor(), false);
    let survivor_spec = cube(SURVIVOR, Vec3::new(15.0, 0.5, 15.0));
    let carrier_spec = cube(CARRIER, Vec3::new(0.0, 0.5, 0.0));
    let survivor;
    let carrier;
    if moved {
        carrier = h.spawn(carrier_spec, false);
        for k in 0..10u32 {
            h.spawn(
                ball(TOWER_BASE + k, Vec3::new(0.0, 1.5 + k as f32, 0.0)),
                false,
            );
        }
        survivor = h.spawn(survivor_spec, false);
    } else {
        survivor = h.spawn(survivor_spec, false);
        for k in 0..10u32 {
            h.spawn(
                ball(TOWER_BASE + k, Vec3::new(0.0, 1.5 + k as f32, 0.0)),
                false,
            );
        }
        carrier = h.spawn(carrier_spec, false);
    }

    let mut pre = Vec::with_capacity(WARM_NOISE_WINDOW);
    for step in 1..=WARM_SETTLE_STEPS {
        h.step();
        if step > WARM_SETTLE_STEPS - WARM_NOISE_WINDOW {
            pre.push(h.body(survivor));
        }
    }
    h.assert_walk_matches_gather();
    let row_before = h.row_of(SURVIVOR);
    assert!(
        h.world.delete_entity(carrier),
        "construction: the carrier must be despawnable"
    );
    let row_after = h.row_of(SURVIVOR);
    if moved {
        assert_eq!(
            (row_before, row_after),
            (12, 1),
            "construction: B must move into A's row"
        );
    } else {
        assert_eq!(
            (row_before, row_after),
            (1, 1),
            "construction: B must keep its row"
        );
    }

    let mut post = Vec::with_capacity(WARM_POST_STEPS);
    for _ in 0..WARM_POST_STEPS {
        h.step();
        post.push(h.body(survivor));
    }
    WarmRun {
        pre,
        post,
        row_before,
        row_after,
    }
}

/// Largest step-to-step change of `field` over a series of states.
fn max_step_change(series: &[RigidBody], field: impl Fn(&RigidBody) -> Vec3) -> f32 {
    series
        .windows(2)
        .map(|w| (field(&w[1]) - field(&w[0])).length())
        .fold(0.0f32, f32::max)
}

fn rigid_bits_eq(a: &RigidBody, b: &RigidBody) -> bool {
    bits_eq(a.position, b.position)
        && bits_eq(a.linear_velocity, b.linear_velocity)
        && bits_eq(a.angular_velocity, b.angular_velocity)
        && a.rotation == b.rotation
}

/// Control-vs-moved comparison of B on the step after the delete.
#[derive(Debug)]
struct WarmMetrics {
    warm_start: bool,
    /// B's state right before the delete is bit-identical in both runs.
    pre_delete_bits_equal: bool,
    /// `|v_moved - v_control|` on the next step.
    v_deviation: f32,
    /// Control's largest step-to-step `|dv|` (last settle window + post window).
    v_noise: f32,
    /// `|p_moved - p_control|` on the next step.
    p_deviation: f32,
    /// Control's largest step-to-step `|dp|` (last settle window + post window).
    p_noise: f32,
    /// Largest `|p_moved - p_control|` over the later post-delete steps.
    p_deviation_later: f32,
    control_next: RigidBody,
    moved_next: RigidBody,
    moved_rows: (usize, usize),
}

fn warm_metrics(warm_start: bool) -> WarmMetrics {
    let control = warm_start_scene(false, warm_start);
    let moved = warm_start_scene(true, warm_start);
    let control_series: Vec<RigidBody> = control.pre.iter().chain(&control.post).copied().collect();
    WarmMetrics {
        warm_start,
        pre_delete_bits_equal: rigid_bits_eq(
            control.pre.last().expect("window is non-empty"),
            moved.pre.last().expect("window is non-empty"),
        ),
        v_deviation: (moved.post[0].linear_velocity - control.post[0].linear_velocity).length(),
        v_noise: max_step_change(&control_series, |b| b.linear_velocity),
        p_deviation: (moved.post[0].position - control.post[0].position).length(),
        p_noise: max_step_change(&control_series, |b| b.position),
        p_deviation_later: moved
            .post
            .iter()
            .zip(&control.post)
            .skip(1)
            .map(|(m, c)| (m.position - c.position).length())
            .fold(0.0f32, f32::max),
        control_next: control.post[0],
        moved_next: moved.post[0],
        moved_rows: (moved.row_before, moved.row_after),
    }
}

fn print_warm_metrics(m: &WarmMetrics) {
    eprintln!(
        "warm_start[cube, warm_start={}]: pre-delete bit-equal {}; |dv| {:e} m/s vs v-noise {:e} m/s \
         (ratio {:e}); |dp| {:e} m vs p-noise {:e} m; later |dp| max {:e} m; moved rows {:?}",
        m.warm_start,
        m.pre_delete_bits_equal,
        m.v_deviation,
        m.v_noise,
        m.v_deviation / m.v_noise,
        m.p_deviation,
        m.p_noise,
        m.p_deviation_later,
        m.moved_rows
    );
    eprintln!("    control next = {:?}", m.control_next);
    eprintln!("    moved   next = {:?}", m.moved_next);
}

/// Warm start: cube survivor B, swap-moved into deleted cube carrier A's row, must leave
/// the next step with the control run's velocity within the control's resting noise (the
/// largest step-to-step `|dv|` of the control's B over its last settle window and its
/// post-delete steps). The deviation / noise ratio is printed (`--nocapture`).
///
/// Cubes, not spheres: a lone sphere's one floor contact has its lever arm parallel to
/// the normal, and the soft solve's `-mass_coeff * m_eff * (vn + bias) - impulse_coeff *
/// lambda` then leaves a velocity independent of the seed `lambda` (`SoftCoefficients`:
/// `a2 * a3 + a3 == 1`), while the relax pass (`-m_eff * vn`) zeroes `vn` exactly.
/// Measured on d5782d43: a sphere survivor that reads the carrier's impulse ends the
/// step bit-identical to the control (position and velocity), so a sphere version of
/// this test cannot fail. A cube's four-point manifold couples the points through the
/// angular terms, and the wrong seed survives.
#[test]
fn warm_start_survivor_moved_into_a_deleted_row_matches_control_velocity() {
    let warm = warm_metrics(true);
    let cold = warm_metrics(false);
    print_warm_metrics(&warm);
    print_warm_metrics(&cold);

    assert!(
        warm.pre_delete_bits_equal && cold.pre_delete_bits_equal,
        "construction: B's state before the delete must be bit-identical in the control and moved runs"
    );
    assert!(
        cold.v_deviation == 0.0 && cold.p_deviation == 0.0,
        "construction: with warm starting OFF the row move alone must not change B's next step \
         (|dv| {:e}, |dp| {:e}) - otherwise the deviation below is not attributable to warm start",
        cold.v_deviation,
        cold.p_deviation
    );
    assert!(
        warm.v_deviation <= warm.v_noise,
        "warm start: cube survivor B, swap-moved from row {} into deleted carrier A's row {}, must leave \
         the next step with the control's velocity within resting noise; |v_moved - v_control| = {:e} m/s \
         vs resting noise {:e} m/s (ratio {:e}); |p_moved - p_control| = {:e} m vs position noise {:e} m",
        warm.moved_rows.0,
        warm.moved_rows.1,
        warm.v_deviation,
        warm.v_noise,
        warm.v_deviation / warm.v_noise,
        warm.p_deviation,
        warm.p_noise
    );
}
