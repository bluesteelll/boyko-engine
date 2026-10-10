//! Defect A5: the body set, the one row selection `physics_gather`, `physics_apply` and
//! `physics_soft_rigid_apply` share (`boyko_physics::body_set`).
//!
//! # What each group checks
//!
//! * `check_selections_*` (V4, V4b): the comparator the wire-up check uses can fail, names
//!   the stage and the differing components, and accepts equal selections.
//! * `the_shipped_selections_*` (V5): `body_walker_selections` reads each stage's own data
//!   alias and the shared filter, and the three shipped selections agree.
//! * `the_selection_masks_predict_*` (V6) and the P1 proptest: the masks are what the
//!   kernel's own query walks select, in the same order for all three stages, including
//!   after incremental archetype-cache updates.
//! * `a_seam_solver_*` (V7a, V7b): an out-of-crate `RigidSolver` writing through either
//!   public scratch surface reaches its own bodies and never a `RigidBody` carrier outside the
//!   body set. The scene runs `probe_carrier` before it steps and asserts the stray sits at
//!   walk index 0: that placement IS the defect signal both tests read, and with the stray
//!   walked last (one `create_archetype` line apart) it falls off the end of the 2-row
//!   snapshot and both tests pass on unfixed release code (round-3 review W1).
//! * `body_set_check_passes_on_*` (V8): every public `add_physics_*` entry point runs the
//!   wire-up check and still builds (the duplicate `RigidBodyMass`/`Collider` read
//!   declarations cause no access conflict).
//!
//! V4, V4b and V5 touch no thread pool and run under Miri. Everything else builds a
//! schedule on a `boyko_threadpool`, which is intractable under Miri. V7 steps the physics
//! schedule, so it runs under a watchdog: a debug-build regression trips `physics_apply`'s
//! row-count `debug_assert!` inside a scheduler worker, which blocks `Schedule::run` today
//! instead of propagating.

use std::panic;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_mask::ComponentMask;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::data::Mut;
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::{Commands, ResMut};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId, EntityId};
use boyko_macros::{Component, Resource};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};
use proptest::prelude::*;

use boyko_physics::body_set::{
    BodyApplyData, BodyGatherData, BodyQuery, BodySoftApplyData, StageSelection,
    body_walker_selections, check_selections,
};
use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Sensor, Simulated,
};
use boyko_physics::manifold::Manifold;
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::{
    add_physics_colored, add_physics_colored_solve, add_physics_sdf, add_physics_soft,
    add_physics_soft_colored, add_physics_systems, add_physics_systems_with_scene_sync,
};
use boyko_physics::resources::{BodyState, PhysicsConfig, SolverScratch};
use boyko_physics::sdf_query::SdfField;
use boyko_physics::solver::{RigidSolver, SoftStepSolver};

// ── Shared helpers ───────────────────────────────────────────────────────────

/// The fixed step (60 Hz).
const DT: f32 = 1.0 / 60.0;

/// Wall-clock budget for one V7 scene. NOT a performance measurement: a liveness guard
/// sized far above an honest two-step run, so only a hang reaches it.
const SCENE_TIMEOUT: Duration = Duration::from_secs(if cfg!(debug_assertions) { 120 } else { 60 });

/// Runs `scene` on its own thread and fails the test if it has not finished within
/// [`SCENE_TIMEOUT`] (a worker panic does not propagate out of `Schedule::run` today). A
/// panic on the scene thread is re-raised with its payload.
fn under_watchdog<T, F>(what: &str, scene: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let handle = thread::Builder::new()
        .name("a5-body-set".to_string())
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
             caller, so this is what `physics_apply`'s row-count `debug_assert!` looks like from \
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
    // SAFETY: `value` is a live, initialised `#[repr(C)]` (or zero-sized) `T` borrowed for the
    // returned slice's lifetime; the slice covers exactly its `size_of::<T>()` bytes
    // read-only, the layout the component pool stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

fn mask_of(ids: &[ComponentId]) -> ComponentMask {
    ComponentMask::from_components(ids)
}

fn body_set_ids() -> [ComponentId; 3] {
    [
        RigidBody::component_id(),
        RigidBodyMass::component_id(),
        Collider::component_id(),
    ]
}

fn a_rigid_body(position: Vec3) -> RigidBody {
    RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    }
}

fn a_mass(inv_mass: f32) -> RigidBodyMass {
    RigidBodyMass {
        inv_inertia: if inv_mass == 0.0 {
            Mat3::ZERO
        } else {
            Mat3::IDENTITY
        },
        inv_mass,
        restitution: 0.0,
        friction: 0.5,
    }
}

fn a_collider() -> Collider {
    Collider {
        shape: ColliderShape::Sphere { radius: 0.5 },
        layer: 1,
        mask: 1,
    }
}

/// A test-local identity column, so every archetype below is this file's own.
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct Tag {
    id: u32,
}

/// Marker components whose insert or remove migrates an entity between archetypes (P1).
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct M0 {
    _tag: u32,
}
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct M1 {
    _tag: u32,
}
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct M2 {
    _tag: u32,
}

fn marker_id(k: usize) -> ComponentId {
    match k {
        0 => M0::component_id(),
        1 => M1::component_id(),
        _ => M2::component_id(),
    }
}

/// One archetype this file creates: which of the body columns it carries, plus markers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shape {
    mass: bool,
    collider: bool,
    sensor: bool,
    markers: [bool; 3],
}

impl Shape {
    const BODY: Self = Self {
        mass: true,
        collider: true,
        sensor: false,
        markers: [false; 3],
    };
    const NO_COLLIDER: Self = Self {
        mass: true,
        collider: false,
        sensor: false,
        markers: [false; 3],
    };
    const NO_MASS: Self = Self {
        mass: false,
        collider: true,
        sensor: false,
        markers: [false; 3],
    };

    fn ids(self) -> Vec<ComponentId> {
        let mut ids = vec![RigidBody::component_id(), Tag::component_id()];
        if self.mass {
            ids.push(RigidBodyMass::component_id());
        }
        if self.collider {
            ids.push(Collider::component_id());
        }
        if self.sensor {
            ids.push(Sensor::component_id());
        }
        for (k, &on) in self.markers.iter().enumerate() {
            if on {
                ids.push(marker_id(k));
            }
        }
        ids
    }

    fn is_member(self) -> bool {
        self.mass && self.collider
    }
}

/// Creates one entity of `shape` in `arch`.
fn spawn_shape(world: &mut EcsMaster, arch: ArchetypeId, shape: Shape, id: u32) -> Entity {
    let body = a_rigid_body(Vec3::new(id as f32 * 3.0, 50.0, 0.0));
    let mass = a_mass(1.0);
    let collider = a_collider();
    let tag = Tag { id };
    let sensor = Sensor;
    let (m0, m1, m2) = (M0 { _tag: 0 }, M1 { _tag: 0 }, M2 { _tag: 0 });
    let marker_bytes = [as_bytes(&m0), as_bytes(&m1), as_bytes(&m2)];
    let mut columns: Vec<(ComponentId, &[u8])> = vec![
        (RigidBody::component_id(), as_bytes(&body)),
        (Tag::component_id(), as_bytes(&tag)),
    ];
    if shape.mass {
        columns.push((RigidBodyMass::component_id(), as_bytes(&mass)));
    }
    if shape.collider {
        columns.push((Collider::component_id(), as_bytes(&collider)));
    }
    if shape.sensor {
        columns.push((Sensor::component_id(), as_bytes(&sensor)));
    }
    for (k, &on) in shape.markers.iter().enumerate() {
        if on {
            columns.push((marker_id(k), marker_bytes[k]));
        }
    }
    world
        .create_entity(arch, &columns)
        .expect("construction: the archetype accepts every column of its shape")
}

// ── Probes: the shipped stage signatures plus an unfiltered carrier walk ──────────

#[derive(Resource, Default)]
struct ProbeLog {
    gather: Vec<EntityId>,
    apply: Vec<EntityId>,
    soft: Vec<EntityId>,
    carrier: Vec<EntityId>,
}

impl ProbeLog {
    fn clear(&mut self) {
        self.gather.clear();
        self.apply.clear();
        self.soft.clear();
        self.carrier.clear();
    }
}

fn probe_gather(query: BodyQuery<BodyGatherData>, mut log: ResMut<ProbeLog>) {
    for (entity, _) in query.iter_entities() {
        log.gather.push(entity);
    }
}

fn probe_apply(mut query: BodyQuery<BodyApplyData>, mut log: ResMut<ProbeLog>) {
    for (entity, _body) in query.iter_entities_mut() {
        log.apply.push(entity);
    }
}

fn probe_soft(mut query: BodyQuery<BodySoftApplyData>, mut log: ResMut<ProbeLog>) {
    for (entity, _body) in query.iter_entities_mut() {
        log.soft.push(entity);
    }
}

/// What an unfiltered write-back walk (the pre-fix `physics_apply`) would visit.
fn probe_carrier(mut query: Query<Mut<RigidBody>>, mut log: ResMut<ProbeLog>) {
    for (entity, _body) in query.iter_entities_mut() {
        log.carrier.push(entity);
    }
}

/// A probe schedule over `world`, built once so its query states update incrementally.
fn probe_schedule(world: &mut EcsMaster, with_carrier: bool) -> Schedule {
    world.insert_resource(ProbeLog::default());
    let mut builder = ScheduleBuilder::new(serial_pool());
    builder.add_system(probe_gather);
    builder.add_system(probe_apply);
    builder.add_system(probe_soft);
    if with_carrier {
        builder.add_system(probe_carrier);
    }
    builder.build(world)
}

// ── V4, V4b: the comparator ──────────────────────────────────────────────────

fn selection(
    stage: &'static str,
    include: ComponentMask,
    exclude: ComponentMask,
) -> StageSelection {
    StageSelection {
        stage,
        data_include: include,
        include,
        exclude,
    }
}

/// V4: a divergent include (in either direction) or exclude is rejected, the first
/// differing entry is named, and every entry is compared against entry 0.
#[test]
fn check_selections_rejects_a_divergent_include() {
    let [rb, mass, col] = body_set_ids();
    let empty = ComponentMask::new();
    let full = mask_of(&[rb, mass, col]);
    let partial = mask_of(&[rb, mass]);
    let sensor = mask_of(&[Sensor::component_id()]);

    let err = check_selections(&[
        selection("reference", full, empty),
        selection("stage", partial, empty),
    ])
    .expect_err("a stage that does not require `Collider` must be rejected");
    assert_eq!(
        (
            err.reference,
            err.stage,
            err.include_only_in_reference,
            err.include_only_in_stage,
            err.exclude_differs,
        ),
        ("reference", "stage", mask_of(&[col]), empty, false),
        "the reference requires `Collider` and the stage does not: (reference, stage, only in \
         reference, only in stage, exclude differs)"
    );
    let message = err.to_string();
    assert!(
        message.starts_with(
            "body set disagreement: `stage` does not select the rows `reference` gathers"
        ) && message.contains(&format!("{col}")),
        "the report must open with the documented sentence and name `Collider`'s id {col}: {message}"
    );

    let err = check_selections(&[
        selection("reference", partial, empty),
        selection("stage", full, empty),
    ])
    .expect_err("a stage that requires more than the reference must be rejected");
    assert_eq!(
        (
            err.include_only_in_reference,
            err.include_only_in_stage,
            err.exclude_differs
        ),
        (empty, mask_of(&[col]), false),
        "mirrored: (only in reference, only in stage, exclude differs)"
    );

    let err = check_selections(&[
        selection("reference", full, empty),
        selection("stage", full, sensor),
    ])
    .expect_err("an exclude-only difference must be rejected");
    assert_eq!(
        (
            err.include_only_in_reference,
            err.include_only_in_stage,
            err.exclude_differs
        ),
        (empty, empty, true),
        "exclude only: (only in reference, only in stage, exclude differs)"
    );

    // Every entry is compared with entry 0, not with its neighbour.
    for (entries, expected_stage) in [
        (
            [
                selection("reference", full, empty),
                selection("first", partial, empty),
                selection("second", partial, empty),
            ],
            "first",
        ),
        (
            [
                selection("reference", full, empty),
                selection("first", full, empty),
                selection("second", partial, empty),
            ],
            "second",
        ),
    ] {
        let err =
            check_selections(&entries).expect_err("a three-entry disagreement must be rejected");
        assert_eq!(
            (err.reference, err.stage),
            ("reference", expected_stage),
            "the first entry that differs from entry 0 is reported"
        );
    }
}

/// V4b: equal selections, an empty slice and a single entry are all accepted; only
/// `(include, exclude)` is compared, never `data_include`.
#[test]
fn check_selections_accepts_equal_selections() {
    let [rb, mass, col] = body_set_ids();
    let empty = ComponentMask::new();
    let full = mask_of(&[rb, mass, col]);
    let gather = StageSelection {
        stage: "gather",
        data_include: full,
        include: full,
        exclude: empty,
    };
    let apply = StageSelection {
        stage: "apply",
        data_include: mask_of(&[rb]),
        include: full,
        exclude: empty,
    };
    assert!(
        check_selections(&[gather, apply, apply]).is_ok(),
        "equal (include, exclude) must be accepted even though data_include differs"
    );
    assert!(check_selections(&[]).is_ok(), "an empty slice is Ok");
    assert!(check_selections(&[apply]).is_ok(), "a single entry is Ok");
}

// ── V5: the shipped selections ───────────────────────────────────────────────

/// V5: the three selections are read from the stage aliases and the shared filter.
#[test]
fn the_shipped_selections_are_read_from_the_stage_signatures() {
    let [rb, mass, col] = body_set_ids();
    let full = mask_of(&[rb, mass, col]);
    let rigid_body_only = mask_of(&[rb]);
    let selections = body_walker_selections(&mut EcsMaster::new());

    let names: Vec<&str> = selections.iter().map(|s| s.stage).collect();
    assert_eq!(
        names,
        [
            "physics_gather",
            "physics_apply",
            "physics_soft_rigid_apply"
        ],
        "the selections name the three stages in order"
    );
    let data: Vec<ComponentMask> = selections.iter().map(|s| s.data_include).collect();
    assert_eq!(
        data,
        [full, rigid_body_only, rigid_body_only],
        "data_include: the gather's data requires the three body columns, each write-back's \
         data only `RigidBody` (ids: RigidBody {rb}, RigidBodyMass {mass}, Collider {col})"
    );
    for s in &selections {
        assert_eq!(
            (s.include, s.exclude),
            (full, ComponentMask::new()),
            "`{}`: include must be the whole body set and exclude empty (ids: RigidBody {rb}, \
             RigidBodyMass {mass}, Collider {col})",
            s.stage
        );
    }
    assert!(
        check_selections(&selections).is_ok(),
        "the shipped selections must agree: {:?}",
        check_selections(&selections).err()
    );
}

// ── V6: the masks predict the kernel's own walks ─────────────────────────────

/// V6: in a world holding both kinds of stray, bodies and a sensor body, each probe visits
/// exactly the entities whose archetype contains its selection's `include`, the three
/// shipped walks are identical, and the unfiltered carrier walk visits strictly more.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs a probe schedule on a boyko_threadpool; thread spawning is \
              intractable under Miri"
)]
fn the_selection_masks_predict_the_kernels_own_walks() {
    let mut world = EcsMaster::new();
    let sensor_body = Shape {
        sensor: true,
        ..Shape::BODY
    };
    let mut spawned: Vec<(EntityId, ComponentMask)> = Vec::new();
    let mut next_id = 0u32;
    // The collider-less stray's archetype is created first, so it is walked first.
    for (shape, count) in [
        (Shape::NO_COLLIDER, 2),
        (Shape::NO_MASS, 2),
        (Shape::BODY, 3),
        (sensor_body, 1),
    ] {
        let arch = world.create_archetype(&shape.ids());
        for _ in 0..count {
            let entity = spawn_shape(&mut world, arch, shape, next_id);
            next_id += 1;
            spawned.push((entity.id(), mask_of(&shape.ids())));
        }
    }

    let selections = body_walker_selections(&mut world);
    let mut probes = probe_schedule(&mut world, true);
    probes.run(&mut world);
    let log = world.resource::<ProbeLog>();

    let predict = |include: &ComponentMask| -> Vec<EntityId> {
        let mut out: Vec<EntityId> = spawned
            .iter()
            .filter(|(_, mask)| include.is_subset(mask))
            .map(|&(id, _)| id)
            .collect();
        out.sort_unstable();
        out
    };
    let sorted = |walk: &[EntityId]| -> Vec<EntityId> {
        let mut out = walk.to_vec();
        out.sort_unstable();
        out
    };

    // (a) each walk is exactly its selection's prediction.
    for (selection, walk) in selections.iter().zip([&log.gather, &log.apply, &log.soft]) {
        assert_eq!(
            sorted(walk),
            predict(&selection.include),
            "(a) `{}` must visit exactly the entities whose archetype contains its include mask",
            selection.stage
        );
    }
    let carrier_include = mask_of(&[RigidBody::component_id()]);
    assert_eq!(
        sorted(&log.carrier),
        predict(&carrier_include),
        "(a) the carrier probe must visit every `RigidBody` carrier"
    );
    assert_eq!(
        log.gather.len(),
        4,
        "anti-vacuity: the three bodies and the sensor body are body-set members"
    );

    // (b) the three shipped sequences are identical, order included.
    assert!(
        log.apply == log.gather && log.soft == log.gather,
        "(b) the shipped walks must be identical: gather {:?}, apply {:?}, soft {:?}",
        log.gather,
        log.apply,
        log.soft
    );

    // (c) the carrier walk visits the strays too.
    assert!(
        log.carrier.len() > log.apply.len(),
        "(c) the unfiltered carrier walk must visit strictly more entities than apply: carrier \
         {:?}, apply {:?}",
        log.carrier,
        log.apply
    );
}

// ── P1: order under incremental archetype-cache updates ──────────────────────

/// One archetype kind P1 may create.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Body,
    StrayNoCollider,
    StrayNoMass,
    MarkedBody(usize),
}

impl Kind {
    fn shape(self) -> Shape {
        match self {
            Self::Body => Shape::BODY,
            Self::StrayNoCollider => Shape::NO_COLLIDER,
            Self::StrayNoMass => Shape::NO_MASS,
            Self::MarkedBody(k) => {
                let mut markers = [false; 3];
                markers[k] = true;
                Shape {
                    markers,
                    ..Shape::BODY
                }
            }
        }
    }
}

/// One structural change P1 applies between probe runs: `(entity pick, op, marker)`.
/// `op` 0 despawns, 1 inserts the marker, 2 removes it.
type Op = (u32, u8, usize);

fn p1_case() -> impl Strategy<Value = (Vec<Kind>, Vec<usize>, Vec<Vec<Op>>)> {
    (0usize..=3).prop_flat_map(|markers| {
        let mut kinds = vec![Kind::Body, Kind::StrayNoCollider, Kind::StrayNoMass];
        kinds.extend((0..markers).map(Kind::MarkedBody));
        let n = kinds.len();
        (
            Just(kinds).prop_shuffle(),
            proptest::collection::vec(1usize..=8, n),
            proptest::collection::vec(
                proptest::collection::vec((any::<u32>(), 0u8..3, 0usize..3), 0..10),
                1..4,
            ),
        )
    })
}

/// Applies `ops` through `Commands` in a one-off schedule, updating the model.
fn apply_ops(world: &mut EcsMaster, live: &mut Vec<(Entity, Shape)>, ops: &[Op]) {
    let mut deferred: Vec<(Entity, u8, usize)> = Vec::new();
    for &(pick, op, k) in ops {
        if live.is_empty() {
            break;
        }
        let index = pick as usize % live.len();
        let (entity, shape) = live[index];
        match op {
            0 => {
                assert!(
                    world.delete_entity(entity),
                    "construction: a live entity despawns"
                );
                live.swap_remove(index);
                deferred.retain(|&(pending, _, _)| pending != entity);
            }
            _ => {
                let mut next = shape;
                next.markers[k] = op == 1;
                live[index].1 = next;
                deferred.push((entity, op, k));
            }
        }
    }
    if deferred.is_empty() {
        return;
    }
    let mut builder = ScheduleBuilder::new(serial_pool());
    builder.add_system(move |mut commands: Commands| {
        for &(entity, op, k) in &deferred {
            let mut e = commands.entity(entity);
            match (op, k) {
                (1, 0) => {
                    e.insert(M0 { _tag: 0 });
                }
                (1, 1) => {
                    e.insert(M1 { _tag: 0 });
                }
                (1, _) => {
                    e.insert(M2 { _tag: 0 });
                }
                (_, 0) => {
                    e.remove::<M0>();
                }
                (_, 1) => {
                    e.remove::<M1>();
                }
                (_, _) => {
                    e.remove::<M2>();
                }
            }
        }
    });
    let mut schedule = builder.build(world);
    schedule.run(world);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// P1: random archetype creation order, then rounds of random despawns and marker
    /// migrations between runs of ONE kept probe schedule (so the query states update
    /// incrementally): after every run the three shipped walks are the same sequence, and
    /// it is exactly the body-set members.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "miri-slow: runs probe schedules on a boyko_threadpool for 32 cases; thread \
                  spawning is intractable under Miri"
    )]
    fn selection_order_is_independent_of_archetype_creation_order(case in p1_case()) {
        let (kinds, counts, rounds) = case;
        let mut world = EcsMaster::new();
        let mut live: Vec<(Entity, Shape)> = Vec::new();
        let mut next_id = 0u32;
        for (kind, &count) in kinds.iter().zip(&counts) {
            let shape = kind.shape();
            let arch = world.create_archetype(&shape.ids());
            for _ in 0..count {
                let entity = spawn_shape(&mut world, arch, shape, next_id);
                next_id += 1;
                live.push((entity, shape));
            }
        }
        let mut probes = probe_schedule(&mut world, false);

        for round in 0..=rounds.len() {
            if round > 0 {
                apply_ops(&mut world, &mut live, &rounds[round - 1]);
            }
            world.resource_mut::<ProbeLog>().clear();
            probes.run(&mut world);
            let log = world.resource::<ProbeLog>();
            prop_assert!(
                log.apply == log.gather && log.soft == log.gather,
                "round {}: the shipped walks differ (creation order {:?}): gather {:?}, apply \
                 {:?}, soft {:?}",
                round, kinds, log.gather, log.apply, log.soft
            );
            let mut members: Vec<EntityId> = live
                .iter()
                .filter(|(_, shape)| shape.is_member())
                .map(|(entity, _)| entity.id())
                .collect();
            members.sort_unstable();
            let mut walked = log.gather.clone();
            walked.sort_unstable();
            prop_assert_eq!(
                walked, members,
                "round {}: the gather walk must be exactly the live body-set members", round
            );
        }
    }
}

// ── V7: an out-of-crate solver writing through the public scratch surfaces ────────

/// Writes `(1, 2, 3)` into every row through `bodies_mut()` and marks each row touched.
#[derive(Resource, Default)]
struct WritesThroughBodiesMut;

impl RigidSolver for WritesThroughBodiesMut {
    fn solve(
        &mut self,
        _config: &PhysicsConfig,
        _manifolds: &[Manifold],
        scratch: &mut SolverScratch,
    ) {
        let n = scratch.bodies().len();
        {
            let mut view = scratch.bodies_mut();
            for row in view.as_mut_slice() {
                row.linear_velocity = Vec3::new(1.0, 2.0, 3.0);
            }
        }
        for row in 0..n {
            scratch.touched.set(row);
        }
    }
}

/// Replaces the rows through `set_bodies()` with `(4, 5, 6)` velocities, then marks each
/// row touched (`set_bodies` resets the touched mask, so the marks come after it).
#[derive(Resource, Default)]
struct WritesThroughSetBodies;

impl RigidSolver for WritesThroughSetBodies {
    fn solve(
        &mut self,
        _config: &PhysicsConfig,
        _manifolds: &[Manifold],
        scratch: &mut SolverScratch,
    ) {
        let mut rows: Vec<BodyState> = scratch.bodies().to_vec();
        for row in &mut rows {
            row.linear_velocity = Vec3::new(4.0, 5.0, 6.0);
        }
        scratch.set_bodies(&rows);
        for row in 0..rows.len() {
            scratch.touched.set(row);
        }
    }
}

/// `(stray spawned, stray finished, body velocities)` after two steps of
/// `add_physics_systems::<S>` with gravity off, a collider-less stray walked first and two
/// bodies.
fn seam_scene<S: RigidSolver + Default>() -> (RigidBody, RigidBody, Vec<Vec3>) {
    let mut world = EcsMaster::new();
    let stray_arch =
        world.create_archetype(&[RigidBody::component_id(), RigidBodyMass::component_id()]);
    let stray_body = a_rigid_body(Vec3::new(7.0, 5.0, 0.0));
    let stray_mass = a_mass(0.0);
    let stray = world
        .create_entity(
            stray_arch,
            &[
                (RigidBody::component_id(), as_bytes(&stray_body)),
                (RigidBodyMass::component_id(), as_bytes(&stray_mass)),
            ],
        )
        .expect("construction: the stray archetype accepts both columns");

    let body_arch = world.bundle_archetype_id_for::<RigidBodyBundle>();
    let mut bodies = Vec::new();
    for x in [-5.0f32, 5.0] {
        let body = a_rigid_body(Vec3::new(x, 1.0, 0.0));
        let mass = a_mass(1.0);
        let collider = a_collider();
        let entity = world
            .create_entity(
                body_arch,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                ],
            )
            .expect("construction: the body archetype accepts the three columns");
        world.enable::<Simulated>(entity);
        bodies.push(entity);
    }

    // The premise V7a and V7b rest on and never read: an unfiltered write-back walk visits
    // the stray at index 0, so the pre-fix `physics_apply` handed it row 0's solved state and
    // shifted the two bodies behind it. Creating the stray's archetype AFTER the bodies' is a
    // one-line edit that walks it last, drops it off the end of the 2-row snapshot, and greens
    // both tests on unfixed code in release (round-3 review W1).
    let mut probes = probe_schedule(&mut world, true);
    probes.run(&mut world);
    let walked = world.resource::<ProbeLog>().carrier.clone();
    let expected: Vec<EntityId> = std::iter::once(stray.id())
        .chain(bodies.iter().map(|e| e.id()))
        .collect();
    assert_eq!(
        walked, expected,
        "construction: an unfiltered `Query<Mut<RigidBody>>` must walk the collider-less stray \
         first and the two bodies behind it; it walked {walked:?}"
    );

    let mut builder = ScheduleBuilder::new(serial_pool());
    let _keys = add_physics_systems::<S>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    let mut schedule = builder.build(&mut world);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;
    for _ in 0..2 {
        schedule.run(&mut world);
    }

    let read = |entity: Entity| -> RigidBody {
        *world
            .get_component::<RigidBody>(entity)
            .expect("harness: a tracked entity is live")
    };
    let velocities = bodies.iter().map(|&e| read(e).linear_velocity).collect();
    (stray_body, read(stray), velocities)
}

fn body_bits_eq(a: &RigidBody, b: &RigidBody) -> bool {
    let v = |p: Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    let q = |r: Quat| [r.x.to_bits(), r.y.to_bits(), r.z.to_bits(), r.w.to_bits()];
    v(a.position) == v(b.position)
        && v(a.linear_velocity) == v(b.linear_velocity)
        && q(a.rotation) == q(b.rotation)
        && v(a.angular_velocity) == v(b.angular_velocity)
}

fn assert_seam_reached_its_bodies(
    what: &str,
    outcome: (RigidBody, RigidBody, Vec<Vec3>),
    expected: Vec3,
) {
    let (spawned, finished, velocities) = outcome;
    assert!(
        body_bits_eq(&spawned, &finished),
        "{what}: the collider-less stray walked first must not be written: spawned {spawned:?}, \
         finished {finished:?}"
    );
    assert!(
        velocities.len() == 2 && velocities.iter().all(|&v| v == expected),
        "{what}: every body must carry the velocity the solver wrote ({expected:?}): {velocities:?}"
    );
}

/// V7a.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning is \
              intractable under Miri"
)]
fn a_seam_solver_writing_through_bodies_mut_reaches_its_bodies() {
    let outcome = under_watchdog(
        "seam solver via bodies_mut",
        seam_scene::<WritesThroughBodiesMut>,
    );
    assert_seam_reached_its_bodies("bodies_mut", outcome, Vec3::new(1.0, 2.0, 3.0));
}

/// V7b.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning is \
              intractable under Miri"
)]
fn a_seam_solver_writing_through_set_bodies_reaches_its_bodies() {
    let outcome = under_watchdog(
        "seam solver via set_bodies",
        seam_scene::<WritesThroughSetBodies>,
    );
    assert_seam_reached_its_bodies("set_bodies", outcome, Vec3::new(4.0, 5.0, 6.0));
}

// ── V8: every public entry point runs the wire-up check and still builds ─────────

/// Builds a world, wires it with `wire`, inserts the clock (and, when `sdf`, an
/// `SdfField`), and builds the schedule. The wire-up check panics inside `wire` on a
/// disagreement, and that panic is re-raised with this binary's component ids so the ids it
/// names can be read; `build` fails on an access conflict.
fn wire_and_build(sdf: bool, wire: impl FnOnce(&mut ScheduleBuilder, &mut EcsMaster)) {
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(serial_pool());
    let wired = panic::catch_unwind(panic::AssertUnwindSafe(|| wire(&mut builder, &mut world)));
    if let Err(payload) = wired {
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("<non-string panic payload>");
        panic!(
            "the wire-up panicked: {message} [component ids in this binary: RigidBody {}, \
             RigidBodyMass {}, Collider {}, Sensor {}]",
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
            Sensor::component_id()
        );
    }
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    if sdf {
        world.insert_resource(SdfField::default());
    }
    let _schedule = builder.build(&mut world);
    assert!(
        world.contains_resource::<SolverScratch>(),
        "construction: the entry point inserted the physics resources"
    );
}

/// V8.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds a schedule on a boyko_threadpool; intractable under Miri"
)]
fn body_set_check_passes_on_add_physics_systems() {
    wire_and_build(false, |b, w| {
        add_physics_systems::<SoftStepSolver>(b, w);
    });
}

/// V8.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds a schedule on a boyko_threadpool; intractable under Miri"
)]
fn body_set_check_passes_on_add_physics_systems_with_scene_sync() {
    wire_and_build(false, |b, w| {
        add_physics_systems_with_scene_sync::<SoftStepSolver>(b, w);
    });
}

/// V8.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds a schedule on a boyko_threadpool; intractable under Miri"
)]
fn body_set_check_passes_on_add_physics_colored() {
    wire_and_build(false, |b, w| {
        add_physics_colored::<SoftStepSolver>(b, w);
    });
}

/// V8.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds a schedule on a boyko_threadpool; intractable under Miri"
)]
fn body_set_check_passes_on_add_physics_colored_solve() {
    wire_and_build(false, |b, w| {
        add_physics_colored_solve(b, w);
    });
}

/// V8.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds a schedule on a boyko_threadpool; intractable under Miri"
)]
fn body_set_check_passes_on_add_physics_sdf() {
    wire_and_build(false, |b, w| {
        add_physics_sdf::<SoftStepSolver>(b, w);
    });
}

/// V8.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds a schedule on a boyko_threadpool; intractable under Miri"
)]
fn body_set_check_passes_on_add_physics_soft_uncoupled() {
    wire_and_build(false, |b, w| {
        add_physics_soft::<SoftStepSolver>(b, w, false);
    });
}

/// V8.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds a schedule on a boyko_threadpool; intractable under Miri"
)]
fn body_set_check_passes_on_add_physics_soft_coupled() {
    wire_and_build(true, |b, w| {
        add_physics_soft::<SoftStepSolver>(b, w, true);
    });
}

/// V8.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds a schedule on a boyko_threadpool; intractable under Miri"
)]
fn body_set_check_passes_on_add_physics_soft_colored() {
    wire_and_build(false, |b, w| {
        add_physics_soft_colored::<SoftStepSolver>(b, w);
    });
}
