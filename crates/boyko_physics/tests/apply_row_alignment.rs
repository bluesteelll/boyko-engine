//! Defect A5: `physics_apply` must write a solved row back into the body it was gathered
//! from.
//!
//! Before the A5 fix, `physics_gather` walked `Query<(Ref<RigidBody>, &RigidBodyMass,
//! &Collider, …)>` and `physics_apply` walked `Query<Mut<RigidBody>>`, and the two were
//! paired by WALK POSITION alone. Any entity that carried `RigidBody` but lacked something
//! the gather requires — a `Collider`, a `RigidBodyMass` — was walked by apply and never
//! gathered. Query walks visit matched archetypes in ascending archetype id, i.e. in
//! creation order, so such an entity shifted every body walked after it by one row: body
//! `i` received body `i + 1`'s solved position and velocity. The fix gives every
//! position-pairing stage the same filter type, `BodyQuery` (`boyko_physics::body_set`).
//!
//! # What each test does
//!
//! Four dynamic spheres and a static floor live in one archetype; a STRAY entity with
//! `RigidBody` but without a gather requirement lives in another, created BEFORE the
//! bodies' archetype so that it is walked first. Two independent readings of the damage:
//!
//! * **Direct.** After a step, every gathered body's live `RigidBody::position` must equal
//!   the `SolverScratch` row it was gathered into. This is the invariant `physics_apply`'s
//!   own doc comment claims.
//! * **End to end.** The same scene WITHOUT the stray entity is the control. The stray is
//!   not gathered, so the solver sees an identical world and is bit-deterministic: every
//!   gathered body must end at exactly the same position in both runs.
//!
//! The stray entity itself must come out byte-identical to how it was spawned — it is not
//! simulated and (for the collider-less kind) has `inv_mass == 0`, so `physics_integrate`
//! skips it and `physics_apply` is the only writer left that could touch it.
//!
//! A third reading needs no physics step at all: `the_apply_walks_are_exactly_the_gather_walk_*`
//! (V1a, V1b) runs one probe system per shipped stage signature (`BodyQuery<BodyGatherData>`,
//! `BodyQuery<BodyApplyData>`, `BodyQuery<BodySoftApplyData>`) and compares the entities they
//! visit, while the file's `carrier_walk` records every `RigidBody` carrier and so pins where
//! the stray sits in a walk that does not filter it.
//!
//! # Both build profiles, two different symptoms
//!
//! In a debug build the desync was caught by `physics_apply`'s own
//! `debug_assert!(row == bodies.len())` — but it fires INSIDE a scheduler worker, and a
//! worker panic does not propagate out of `Schedule::run` today: it blocks the caller
//! forever. Every scene therefore runs on its own thread under a watchdog, so the test
//! fails with a message instead of the binary hanging; libtest captures the worker's own
//! panic into the failing test's output, so it is quoted there. In a release build there is
//! no assert and the misassignment was SILENT, which is why the positional assertions above
//! exist at all.
//!
//! One case read differently in the two profiles before the fix and says so in its own doc
//! comment: a stray walked LAST left the gathered rows aligned by luck (the trailing rows
//! fell off the end of the snapshot and were skipped), so it was red only in debug, on the
//! row-count assert. That asymmetry was the defect's shape — the damage depended on
//! archetype creation order, which no caller controls.
//!
//! Device-free; every scene spins up a `boyko_threadpool`, which is intractable under
//! Miri.

use std::panic;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::{Commands, ResMut};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, EntityId};
use boyko_macros::{Component, Resource};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::body_set::{BodyApplyData, BodyGatherData, BodyQuery, BodySoftApplyData};
use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{PhysicsConfig, SolverScratch};

// ── Scene constants ──────────────────────────────────────────────────────────

/// The fixed step (60 Hz).
const DT: f32 = 1.0 / 60.0;
/// Steps of the plain scenes: long enough for the spheres to fall, hit the floor and
/// resolve contacts, so a shifted row is a gross position error rather than a rounding
/// one.
const STEPS: usize = 40;
/// Steps before the mid-run `Collider` removal.
const STEPS_BEFORE_REMOVAL: usize = 20;
/// Steps after the mid-run `Collider` removal.
const STEPS_AFTER_REMOVAL: usize = 40;
/// A static floor box whose top face is `y = 0`.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(20.0, 0.5, 20.0);

/// Wall-clock budget for one scene. NOT a performance measurement: it is the liveness
/// guard described in the module header, sized far above any honest run of at most 60
/// steps of a five-body scene so that only a hang can reach it.
const SCENE_TIMEOUT: Duration = Duration::from_secs(if cfg!(debug_assertions) { 120 } else { 60 });

/// Body identities carried in [`BodyId`].
const B1: u32 = 1;
const B2: u32 = 2;
const B3: u32 = 3;
const FLOOR: u32 = 4;
/// The entity that carries `RigidBody` and is never gathered.
const STRAY: u32 = 9;

// ── Test-only component ──────────────────────────────────────────────────────

/// Stable identity of a body, so assertions follow entities rather than rows.
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct BodyId {
    id: u32,
}

// ── Probes: the shipped stage signatures, run without the physics schedule ────────

/// The entities each probe visited, in walk order.
#[derive(Resource, Default)]
struct ProbeLog {
    gather: Vec<EntityId>,
    apply: Vec<EntityId>,
    soft: Vec<EntityId>,
}

/// Walks exactly what `physics_gather` walks: its first parameter's type.
fn probe_gather(query: BodyQuery<BodyGatherData>, mut log: ResMut<ProbeLog>) {
    for (entity, _) in query.iter_entities() {
        log.gather.push(entity);
    }
}

/// Walks exactly what `physics_apply` walks. The `Mut` guard is never dereferenced
/// mutably, so no change tick moves.
fn probe_apply(mut query: BodyQuery<BodyApplyData>, mut log: ResMut<ProbeLog>) {
    for (entity, _body) in query.iter_entities_mut() {
        log.apply.push(entity);
    }
}

/// Walks exactly what `physics_soft_rigid_apply` walks; same care as [`probe_apply`].
fn probe_soft(mut query: BodyQuery<BodySoftApplyData>, mut log: ResMut<ProbeLog>) {
    for (entity, _body) in query.iter_entities_mut() {
        log.soft.push(entity);
    }
}

/// The three shipped stage walks as body ids.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ShippedWalks {
    gather: Vec<u32>,
    apply: Vec<u32>,
    soft: Vec<u32>,
}

/// Every walk a scene records before it steps.
#[derive(Clone, Debug)]
struct Walks {
    /// The test's own gather-shaped query: row `i` of the solver is element `i`.
    gather: Vec<u32>,
    /// Every `RigidBody` carrier in ascending archetype order (not a shipped walk).
    carrier: Vec<u32>,
    /// What the shipped stage signatures visit.
    shipped: ShippedWalks,
}

// ── Watchdog ─────────────────────────────────────────────────────────────────────

/// Runs `scene` on its own thread and fails the test if it has not finished within
/// [`SCENE_TIMEOUT`], instead of blocking the process.
///
/// A panic raised inside a scheduler worker does not propagate out of `Schedule::run`
/// today — it blocks the caller — and a regression of this fix trips `physics_apply`'s
/// row-count `debug_assert!` in a debug build. A panic on the scene thread itself is
/// re-raised here with its original payload, so an ordinary failed assertion still reports
/// its own message.
fn under_watchdog<T, F>(what: &str, scene: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let handle = thread::Builder::new()
        .name("a5-scene".to_string())
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
             caller, so this is what `physics_apply`'s gather/apply row-count `debug_assert!` \
             looks like from the outside; the worker's own message is printed in this test's \
             captured output, above this line."
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

/// Bit-exact vector equality (the solver is run-to-run bit-deterministic; the control
/// comparison depends on that, and `control_a_scene_without_a_stray_body_is_reproducible`
/// is the guard for it).
fn bits_eq(a: Vec3, b: Vec3) -> bool {
    a.x.to_bits() == b.x.to_bits()
        && a.y.to_bits() == b.y.to_bits()
        && a.z.to_bits() == b.z.to_bits()
}

/// Which gather requirement the stray entity lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stray {
    /// `{RigidBody, RigidBodyMass, BodyId}` — no `Collider`. The engine's documented
    /// "immovable contact surface" shape (`inv_mass == 0`, `Simulated` clear) minus its
    /// collider: a body a caller forgot to give one.
    NoCollider,
    /// `{RigidBody, Collider, BodyId}` — no `RigidBodyMass`.
    NoMass,
}

impl Stray {
    fn component_ids(self) -> [boyko_ecs::ecs::identifiers::primitives::ComponentId; 3] {
        match self {
            Self::NoCollider => [
                RigidBody::component_id(),
                RigidBodyMass::component_id(),
                BodyId::component_id(),
            ],
            Self::NoMass => [
                RigidBody::component_id(),
                Collider::component_id(),
                BodyId::component_id(),
            ],
        }
    }
}

/// Where the stray archetype sits in the walk relative to the gathered bodies' archetype.
/// Query walks visit matched archetypes in ascending archetype id, so this is just
/// creation order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Walk {
    /// Created BEFORE the bodies' archetype: walked first by an unfiltered walk.
    StrayFirst,
    /// Created AFTER it: walked last.
    StrayLast,
}

/// What to spawn into the gathered archetype.
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

/// A unit-mass sphere of radius 0.5.
fn ball(id: u32, position: Vec3) -> Spec {
    Spec {
        id,
        position,
        shape: ColliderShape::Sphere { radius: 0.5 },
        inv_mass: 1.0,
    }
}

struct Harness {
    world: EcsMaster,
    physics: Schedule,
    /// `{RigidBody, RigidBodyMass, Collider, BodyId}`: every GATHERED body lives here.
    plain: ArchetypeId,
    /// The stray archetype, when the scene has one.
    stray_arch: Option<(Stray, ArchetypeId)>,
    /// Every entity this harness spawned, with its body id.
    spawned: Vec<(EntityId, u32)>,
}

impl Harness {
    /// The real `add_physics_colored_solve` schedule with sleeping OFF, so every dynamic
    /// row is solved and flagged touched on every step and `physics_apply` writes it back.
    ///
    /// `stray` decides whether a stray archetype exists and whether it is created before
    /// or after the gathered bodies' archetype.
    fn new(stray: Option<(Stray, Walk)>) -> Self {
        let mut world = EcsMaster::new();
        let stray_before = matches!(stray, Some((_, Walk::StrayFirst)));
        let mut stray_arch = None;
        if let Some((kind, _)) = stray
            && stray_before
        {
            stray_arch = Some((kind, world.create_archetype(&kind.component_ids())));
        }
        let plain = world.create_archetype(&[
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
            BodyId::component_id(),
        ]);
        if let Some((kind, _)) = stray
            && !stray_before
        {
            stray_arch = Some((kind, world.create_archetype(&kind.component_ids())));
        }
        let mut builder = ScheduleBuilder::new(serial_pool());
        let _keys = add_physics_colored_solve(&mut builder, &mut world);
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        let physics = builder.build(&mut world);
        {
            let cfg = world.resource_mut::<PhysicsConfig>();
            cfg.sleeping = false;
        }
        Self {
            world,
            physics,
            plain,
            stray_arch,
            spawned: Vec::new(),
        }
    }

    /// Spawns a gathered body: it carries every column the gather requires.
    fn spawn_body(&mut self, spec: Spec) -> Entity {
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
        self.spawned.push((entity.id(), spec.id));
        entity
    }

    /// Spawns the stray entity: `RigidBody` plus exactly one of the gather's other two
    /// requirements. `Simulated` is left CLEAR and `inv_mass` is zero, so
    /// `physics_integrate` skips it and `physics_apply` is the only writer that could
    /// still touch it.
    fn spawn_stray(&mut self) -> (Entity, RigidBody) {
        let (kind, arch) = self
            .stray_arch
            .expect("construction: this scene has no stray archetype");
        let body = RigidBody {
            position: Vec3::new(7.0, 5.0, 0.0),
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        };
        let mass = RigidBodyMass {
            inv_inertia: Mat3::ZERO,
            inv_mass: 0.0,
            restitution: 0.0,
            friction: 0.5,
        };
        let collider = Collider {
            shape: ColliderShape::Sphere { radius: 0.5 },
            layer: 1,
            mask: 1,
        };
        let id = BodyId { id: STRAY };
        let columns: [(_, &[u8]); 3] = match kind {
            Stray::NoCollider => [
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (BodyId::component_id(), as_bytes(&id)),
            ],
            Stray::NoMass => [
                (RigidBody::component_id(), as_bytes(&body)),
                (Collider::component_id(), as_bytes(&collider)),
                (BodyId::component_id(), as_bytes(&id)),
            ],
        };
        let entity = self
            .world
            .create_entity(arch, &columns)
            .expect("construction: the stray archetype accepts every column");
        self.spawned.push((entity.id(), STRAY));
        (entity, body)
    }

    fn step(&mut self) {
        self.physics.run(&mut self.world);
    }

    fn run(&mut self, steps: usize) {
        for _ in 0..steps {
            self.step();
        }
    }

    /// Body ids in the GATHER's walk order: row `i` of the solver is element `i`.
    fn gather_walk(&mut self) -> Vec<u32> {
        let q = self
            .world
            .query::<(&RigidBody, &RigidBodyMass, &Collider, &BodyId), ()>();
        q.iter().map(|(_, _, _, id)| id.id).collect()
    }

    /// Body ids of every entity carrying `RigidBody`, in ascending archetype order.
    ///
    /// This is NOT a shipped walk: it is what an unfiltered `Query<Mut<RigidBody>>` would
    /// visit, and it only pins where the stray sits relative to the bodies. The shipped
    /// walks come from [`Self::shipped_walks`].
    fn carrier_walk(&mut self) -> Vec<u32> {
        let q = self.world.query::<(&RigidBody, &BodyId), ()>();
        q.iter().map(|(_, id)| id.id).collect()
    }

    /// Runs the three probe systems once, in a one-off schedule, and maps what they
    /// visited to body ids. Never runs the physics schedule.
    fn shipped_walks(&mut self) -> ShippedWalks {
        self.world.insert_resource(ProbeLog::default());
        let mut builder = ScheduleBuilder::new(serial_pool());
        builder.add_system(probe_gather);
        builder.add_system(probe_apply);
        builder.add_system(probe_soft);
        let mut probes = builder.build(&mut self.world);
        probes.run(&mut self.world);
        let log = self.world.resource::<ProbeLog>();
        let to_ids = |walked: &[EntityId]| -> Vec<u32> {
            walked
                .iter()
                .map(|entity| {
                    self.spawned
                        .iter()
                        .find(|(spawned, _)| spawned == entity)
                        .map(|&(_, id)| id)
                        .unwrap_or_else(|| {
                            panic!("harness: a probe visited {entity:?}, which this harness did not spawn")
                        })
                })
                .collect()
        };
        ShippedWalks {
            gather: to_ids(&log.gather),
            apply: to_ids(&log.apply),
            soft: to_ids(&log.soft),
        }
    }

    /// Live position of each gathered body, in gather-walk order.
    fn gathered_positions(&mut self) -> Vec<(u32, Vec3)> {
        let q = self
            .world
            .query::<(&RigidBody, &RigidBodyMass, &Collider, &BodyId), ()>();
        q.iter().map(|(b, _, _, id)| (id.id, b.position)).collect()
    }

    /// Rows where the live component disagrees with the solver snapshot it was gathered
    /// into, as `(row, body id, live position, snapshot position)`. Valid right after a
    /// step and before any structural change.
    ///
    /// Empty is the invariant `physics_apply` documents. It is not vacuously empty: the
    /// snapshot advances every step (the colored solver owns integration and writes the
    /// integrated pose into `SolverScratch`), so a body whose component was NOT written
    /// back, or was written back from another row, shows up here.
    fn row_mismatches(&mut self) -> Vec<(usize, u32, Vec3, Vec3)> {
        let walked = self.gathered_positions();
        let snapshot: Vec<Vec3> = self
            .world
            .resource::<SolverScratch>()
            .bodies()
            .iter()
            .map(|b| b.position)
            .collect();
        let mut out = Vec::new();
        for (row, (id, live)) in walked.iter().copied().enumerate() {
            match snapshot.get(row) {
                Some(&snap) if bits_eq(live, snap) => {}
                Some(&snap) => out.push((row, id, live, snap)),
                None => out.push((row, id, live, Vec3::new(f32::NAN, f32::NAN, f32::NAN))),
            }
        }
        out
    }

    fn body(&self, entity: Entity) -> RigidBody {
        *self
            .world
            .get_component::<RigidBody>(entity)
            .expect("harness: a tracked body is live")
    }

    /// Removes `Collider` from `entity` through `Commands`, applied by a one-system
    /// schedule BETWEEN physics steps (so the removal is not itself a mid-pipeline
    /// structural change).
    fn remove_collider(&mut self, entity: Entity) {
        let mut builder = ScheduleBuilder::new(serial_pool());
        builder.add_system(move |mut commands: Commands| {
            commands.entity(entity).remove::<Collider>();
        });
        let mut schedule = builder.build(&mut self.world);
        schedule.run(&mut self.world);
    }
}

// ── Scenes ───────────────────────────────────────────────────────────────────

/// The stray entity as spawned and as it came out.
#[derive(Debug)]
struct StrayState {
    kind: Stray,
    spawned: RigidBody,
    finished: RigidBody,
}

#[derive(Debug)]
struct SceneResult {
    /// The walks recorded before the scene stepped (after the removal, for the removal
    /// scene).
    walks: Walks,
    /// Final position of every gathered body, in gather-walk order.
    gathered: Vec<(u32, Vec3)>,
    /// Gathered rows whose live pose disagrees with their solver row after the last step.
    row_mismatches: Vec<(usize, u32, Vec3, Vec3)>,
    stray: Option<StrayState>,
}

/// Three falling spheres over a static floor, plus (optionally) a stray entity carrying
/// `RigidBody` outside the gather's shape — built, with every walk recorded, and NOT
/// stepped.
///
/// The first gathered row is a DYNAMIC body on purpose: only a simulated dynamic row is
/// flagged touched, so row 0's state is actually written somewhere. With the floor first,
/// a stray walked first would receive an untouched row and the damage would hide.
fn build_plain_scene(
    stray: Option<(Stray, Walk)>,
) -> (Harness, Option<(Stray, Entity, RigidBody)>, Walks) {
    let mut h = Harness::new(stray);
    h.spawn_body(ball(B1, Vec3::new(-2.0, 2.0, 0.0)));
    h.spawn_body(ball(B2, Vec3::new(0.0, 3.0, 0.0)));
    h.spawn_body(ball(B3, Vec3::new(2.0, 4.0, 0.0)));
    h.spawn_body(floor());
    let stray_entity = stray.map(|(kind, _)| {
        let (entity, spawned) = h.spawn_stray();
        (kind, entity, spawned)
    });

    let gather = h.gather_walk();
    assert_eq!(
        gather,
        vec![B1, B2, B3, FLOOR],
        "construction: the gather must walk the three spheres then the floor"
    );
    let carrier = h.carrier_walk();
    let (expected_carrier, premise) = match stray {
        None => (
            vec![B1, B2, B3, FLOOR],
            "with no stray, the `RigidBody` carriers are exactly the bodies",
        ),
        Some((_, Walk::StrayFirst)) => (
            vec![STRAY, B1, B2, B3, FLOOR],
            "the stray carries `RigidBody` and sits before the bodies in ascending archetype order",
        ),
        Some((_, Walk::StrayLast)) => (
            vec![B1, B2, B3, FLOOR, STRAY],
            "the stray carries `RigidBody` and sits after the bodies in ascending archetype order",
        ),
    };
    assert_eq!(
        carrier, expected_carrier,
        "construction: {premise} - this is the premise the whole file rests on"
    );
    let shipped = h.shipped_walks();
    (
        h,
        stray_entity,
        Walks {
            gather,
            carrier,
            shipped,
        },
    )
}

/// [`build_plain_scene`], stepped [`STEPS`] times.
fn plain_scene(stray: Option<(Stray, Walk)>) -> SceneResult {
    let (mut h, stray_entity, walks) = build_plain_scene(stray);

    h.run(STEPS);

    SceneResult {
        walks,
        gathered: h.gathered_positions(),
        row_mismatches: h.row_mismatches(),
        stray: stray_entity.map(|(kind, entity, spawned)| StrayState {
            kind,
            spawned,
            finished: h.body(entity),
        }),
    }
}

/// The same three spheres, with `Collider` removed from B3 after `steps_before` steps —
/// built, with every walk recorded after the removal, and not stepped any further.
/// `walk` decides whether the archetype B3 migrates INTO was created before the bodies'
/// archetype (so B3 is walked first afterwards by an unfiltered walk) or after it (walked
/// last). In these scenes B3 plays the stray's role.
fn build_removal_scene(walk: Walk, steps_before: usize) -> (Harness, Walks) {
    let mut h = Harness::new(Some((Stray::NoCollider, walk)));
    h.spawn_body(ball(B1, Vec3::new(-2.0, 2.0, 0.0)));
    h.spawn_body(ball(B2, Vec3::new(0.0, 3.0, 0.0)));
    let b3 = h.spawn_body(ball(B3, Vec3::new(2.0, 4.0, 0.0)));
    h.spawn_body(floor());
    assert_eq!(
        h.gather_walk(),
        vec![B1, B2, B3, FLOOR],
        "construction: the gather walk before the removal"
    );

    h.run(steps_before);
    h.remove_collider(b3);

    let gather = h.gather_walk();
    assert_eq!(
        gather,
        vec![B1, B2, FLOOR],
        "construction: B3 must leave the gather when its `Collider` goes, and the floor must \
         swap-move into its row"
    );
    let carrier = h.carrier_walk();
    let (expected_carrier, where_) = match walk {
        Walk::StrayFirst => (vec![B3, B1, B2, FLOOR], "before"),
        Walk::StrayLast => (vec![B1, B2, FLOOR, B3], "after"),
    };
    assert_eq!(
        carrier, expected_carrier,
        "construction: B3 now carries `RigidBody` without `Collider` and sits {where_} the \
         bodies in ascending archetype order: the removal must migrate it into the archetype \
         this scene pre-created ({walk:?}); if the ECS minted a fresh archetype instead, this \
         scene is not testing what it claims"
    );
    let shipped = h.shipped_walks();
    (
        h,
        Walks {
            gather,
            carrier,
            shipped,
        },
    )
}

/// [`build_removal_scene`] after [`STEPS_BEFORE_REMOVAL`] steps, stepped
/// [`STEPS_AFTER_REMOVAL`] more times.
fn removal_scene(walk: Walk) -> SceneResult {
    let (mut h, walks) = build_removal_scene(walk, STEPS_BEFORE_REMOVAL);

    h.run(STEPS_AFTER_REMOVAL);

    SceneResult {
        walks,
        gathered: h.gathered_positions(),
        row_mismatches: h.row_mismatches(),
        stray: None,
    }
}

// ── Comparison helpers ───────────────────────────────────────────────────────

/// Fails unless every gathered body ends at bit-exactly the position it has in `control`.
fn assert_matches_control(what: &str, control: &SceneResult, actual: &SceneResult) {
    assert_eq!(
        control.gathered.len(),
        actual.gathered.len(),
        "construction: {what}: the control and the scene must gather the same body count \
         (control walk {:?}, scene walk {:?})",
        control.walks.gather,
        actual.walks.gather
    );
    let differing: Vec<(u32, Vec3, u32, Vec3)> = control
        .gathered
        .iter()
        .zip(&actual.gathered)
        .filter(|((_, c), (_, a))| !bits_eq(*c, *a))
        .map(|(&(cid, c), &(aid, a))| (cid, c, aid, a))
        .collect();
    assert!(
        differing.is_empty(),
        "{what}: `physics_apply` pairs its walk with the gather walk by POSITION, so a body \
         that is walked by apply and not gathered shifts every body behind it. Gather walk {:?}, \
         shipped apply walk {:?}, carrier walk {:?}. Each gathered body must end where the \
         stray-free control put it, since the stray is not gathered and the solver is \
         bit-deterministic. Differing (control id, control position, scene id, scene \
         position): {:?}. Rows disagreeing with their own solver snapshot after the last step: \
         {:?}.",
        actual.walks.gather,
        actual.walks.shipped.apply,
        actual.walks.carrier,
        differing,
        actual.row_mismatches
    );
}

// ── The shipped walks, read without stepping ─────────────────────────────────

/// V1a: in every plain scene shape (each stray kind walked first and last, and no stray),
/// every shipped write-back walk is exactly the shipped gather walk, while the unfiltered
/// carrier walk still contains the stray. No scene here steps physics, so this reads the
/// stage signatures alone.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds the real physics schedule and runs probe schedules on a \
              boyko_threadpool; thread spawning is intractable under Miri"
)]
fn the_apply_walks_are_exactly_the_gather_walk_in_the_plain_scenes() {
    let report = under_watchdog("shipped walks of the plain scenes", || {
        let mut rows: Vec<(String, Walks, Option<u32>)> = Vec::new();
        for stray in [
            Some((Stray::NoCollider, Walk::StrayFirst)),
            Some((Stray::NoMass, Walk::StrayFirst)),
            Some((Stray::NoCollider, Walk::StrayLast)),
            Some((Stray::NoMass, Walk::StrayLast)),
            None,
        ] {
            let (_h, _stray, walks) = build_plain_scene(stray);
            rows.push((format!("plain {stray:?}"), walks, stray.map(|_| STRAY)));
        }
        rows
    });
    assert_shipped_walks_match_the_gather(&report);
}

/// V1b: after `Collider` is removed from B3 (migrating it into an archetype walked first or
/// last), every shipped write-back walk is exactly the shipped gather walk `[B1, B2, FLOOR]`,
/// while the unfiltered carrier walk still contains B3.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds the real physics schedule and runs probe schedules on a \
              boyko_threadpool; thread spawning is intractable under Miri"
)]
fn the_apply_walks_are_exactly_the_gather_walk_after_a_collider_removal() {
    let report = under_watchdog("shipped walks after a collider removal", || {
        let mut rows: Vec<(String, Walks, Option<u32>)> = Vec::new();
        for walk in [Walk::StrayFirst, Walk::StrayLast] {
            let (_h, walks) = build_removal_scene(walk, 0);
            rows.push((format!("removal {walk:?}, 0 steps before"), walks, Some(B3)));
        }
        rows
    });
    for (scene, walks, _) in &report {
        assert_eq!(
            walks.shipped.gather,
            vec![B1, B2, FLOOR],
            "{scene}: the shipped gather must walk B1, B2 and the floor once B3 has lost its \
             `Collider`"
        );
    }
    assert_shipped_walks_match_the_gather(&report);
}

/// The V1a/V1b predicate: for each `(scene, walks, excluded)`, the shipped gather is
/// non-empty and equals the gather-shaped query, both shipped apply walks equal it, and the
/// excluded body (if any) is in the carrier walk and nowhere else.
fn assert_shipped_walks_match_the_gather(report: &[(String, Walks, Option<u32>)]) {
    for (scene, walks, stray) in report {
        let shipped = &walks.shipped;
        assert!(
            !shipped.gather.is_empty(),
            "{scene}: anti-vacuity: the shipped gather walk must not be empty"
        );
        assert!(
            shipped.gather == walks.gather
                && shipped.apply == shipped.gather
                && shipped.soft == shipped.gather,
            "{scene}: `physics_apply` and `physics_soft_rigid_apply` must walk exactly the \
             entities `physics_gather` walks, in the same order; shipped gather {:?}, shipped \
             apply {:?}, shipped soft apply {:?}, the gather-shaped query {:?}",
            shipped.gather,
            shipped.apply,
            shipped.soft,
            walks.gather
        );
        match stray {
            Some(stray) => assert!(
                walks.carrier.contains(stray)
                    && !shipped.apply.contains(stray)
                    && walks.carrier.len() == shipped.apply.len() + 1,
                "{scene}: body {stray} carries `RigidBody` outside the body set, so the carrier \
                 walk must contain it and the shipped apply walk must not; carrier {:?}, shipped \
                 apply {:?}",
                walks.carrier,
                shipped.apply
            ),
            None => assert_eq!(
                walks.carrier, shipped.apply,
                "{scene}: with no stray every `RigidBody` carrier is a body-set member"
            ),
        }
    }
}

// ── A stray body walked before the gathered bodies ───────────────────────────

/// Direct reading: after a step, a gathered body's component must hold the row it was
/// gathered into.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              40 solver steps are intractable under Miri"
)]
fn gathered_bodies_hold_the_row_they_were_gathered_from_with_a_collider_less_body_first() {
    let scene = under_watchdog("collider-less stray walked first", || {
        plain_scene(Some((Stray::NoCollider, Walk::StrayFirst)))
    });
    assert!(
        scene.row_mismatches.is_empty(),
        "an entity with `RigidBody` but no `Collider`, walked before the bodies (carrier walk \
         {:?}, shipped apply walk {:?}, gather walk {:?}), shifts `physics_apply`'s row counter \
         if apply walks it: every gathered body then holds another row's solved state. `(row, \
         body id, live position, its own snapshot row)`: {:?}",
        scene.walks.carrier,
        scene.walks.shipped.apply,
        scene.walks.gather,
        scene.row_mismatches
    );
}

/// End-to-end reading: the same scene without the stray entity is the control.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              40 solver steps are intractable under Miri"
)]
fn gathered_bodies_match_the_stray_free_control_with_a_collider_less_body_first() {
    let control = under_watchdog("control for the collider-less stray", || plain_scene(None));
    let scene = under_watchdog("collider-less stray walked first", || {
        plain_scene(Some((Stray::NoCollider, Walk::StrayFirst)))
    });
    assert_matches_control(
        "a body with `RigidBody` and no `Collider`, walked first",
        &control,
        &scene,
    );
}

/// The same, for an entity that lacks `RigidBodyMass` instead of `Collider`: the gather
/// requires both, so either omission produces the same desync.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              40 solver steps are intractable under Miri"
)]
fn gathered_bodies_match_the_stray_free_control_with_a_mass_less_body_first() {
    let control = under_watchdog("control for the mass-less stray", || plain_scene(None));
    let scene = under_watchdog("mass-less stray walked first", || {
        plain_scene(Some((Stray::NoMass, Walk::StrayFirst)))
    });
    assert_matches_control(
        "a body with `RigidBody` and no `RigidBodyMass`, walked first",
        &control,
        &scene,
    );
}

/// The stray entity is not simulated, has no mass and is not gathered: nothing in the
/// pipeline may write it. Before the A5 fix `physics_apply` did, because its walk position
/// collided with a solved row.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              40 solver steps are intractable under Miri"
)]
fn a_collider_less_body_is_never_written_by_physics_apply() {
    let scene = under_watchdog("collider-less stray walked first", || {
        plain_scene(Some((Stray::NoCollider, Walk::StrayFirst)))
    });
    assert_stray_untouched(&scene);
}

/// V2: the same for a stray that lacks `RigidBodyMass` (so `physics_integrate`, which
/// requires `RigidBodyMass`, cannot reach it either).
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              40 solver steps are intractable under Miri"
)]
fn a_mass_less_body_is_never_written_by_physics_apply() {
    let scene = under_watchdog("mass-less stray walked first", || {
        plain_scene(Some((Stray::NoMass, Walk::StrayFirst)))
    });
    assert_stray_untouched(&scene);
}

/// The stray's position and velocity must come out bit-identical to how it was spawned.
fn assert_stray_untouched(scene: &SceneResult) {
    let stray = scene
        .stray
        .as_ref()
        .expect("construction: this scene spawns a stray entity");
    assert!(
        bits_eq(stray.spawned.position, stray.finished.position)
            && bits_eq(
                stray.spawned.linear_velocity,
                stray.finished.linear_velocity
            ),
        "the stray entity ({:?}) carries `RigidBody` but is not gathered, not `Simulated` and has \
         `inv_mass == 0` or no mass at all, so no physics stage may write it; carrier walk {:?}, \
         shipped apply walk {:?}, gather walk {:?}. Spawned {:?}, finished {:?}.",
        stray.kind,
        scene.walks.carrier,
        scene.walks.shipped.apply,
        scene.walks.gather,
        stray.spawned,
        stray.finished
    );
}

/// The same stray entity, walked LAST.
///
/// Before the A5 fix a RELEASE build was green here and that was the point: the trailing
/// rows fell off the end of the snapshot and were skipped, so the damage depended entirely
/// on archetype creation order — which no caller controls. A DEBUG build was red on
/// `physics_apply`'s own row-count `debug_assert!`, which fired for any stray regardless of
/// order. Both readings were the same defect; the fix makes it green in both.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              40 solver steps are intractable under Miri"
)]
fn gathered_bodies_match_the_stray_free_control_with_a_collider_less_body_last() {
    let control = under_watchdog("control for the trailing stray", || plain_scene(None));
    let scene = under_watchdog("collider-less stray walked last", || {
        plain_scene(Some((Stray::NoCollider, Walk::StrayLast)))
    });
    assert_matches_control(
        "a body with `RigidBody` and no `Collider`, walked last",
        &control,
        &scene,
    );
}

// ── The collider removed from a body mid-run ─────────────────────────────────

/// A body that loses its `Collider` between two steps leaves the gather but still carries
/// `RigidBody`. Where it lands must not decide whether the remaining bodies keep their own
/// state: the two runs differ only in whether the destination archetype existed before the
/// bodies' archetype, and they gather exactly the same bodies, so they must agree.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              60 solver steps are intractable under Miri"
)]
fn gathered_bodies_keep_their_state_when_a_collider_is_removed_mid_run() {
    let control = under_watchdog(
        "collider removed, body migrates to a trailing archetype",
        || removal_scene(Walk::StrayLast),
    );
    let scene = under_watchdog(
        "collider removed, body migrates to a leading archetype",
        || removal_scene(Walk::StrayFirst),
    );
    assert_matches_control(
        "a body whose `Collider` was removed mid-run, migrating into a leading archetype",
        &control,
        &scene,
    );
}

// ── Green controls ───────────────────────────────────────────────────────────

/// The comparison above is bit-exact, so it is worth nothing unless the scene is
/// bit-reproducible. Two runs of the stray-free scene must agree exactly.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              40 solver steps are intractable under Miri"
)]
fn control_a_scene_without_a_stray_body_is_reproducible() {
    let first = under_watchdog("control run 1", || plain_scene(None));
    let second = under_watchdog("control run 2", || plain_scene(None));
    assert_matches_control("two runs of the stray-free control", &first, &second);
    assert!(
        first.row_mismatches.is_empty() && second.row_mismatches.is_empty(),
        "the stray-free control must also satisfy the gather/apply row invariant; run 1: {:?}, \
         run 2: {:?}",
        first.row_mismatches,
        second.row_mismatches
    );
}

/// ...and unless the scene actually moves. A comparison between two frozen worlds would
/// pass no matter what `physics_apply` did.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool; thread spawning and \
              40 solver steps are intractable under Miri"
)]
fn control_a_scene_without_a_stray_body_actually_simulates() {
    let scene = under_watchdog("control run", || plain_scene(None));
    let spawned_y = [(B1, 2.0f32), (B2, 3.0f32), (B3, 4.0f32)];
    let stationary: Vec<(u32, Vec3)> = scene
        .gathered
        .iter()
        .copied()
        .filter(|(id, pos)| {
            spawned_y
                .iter()
                .any(|&(sid, sy)| sid == *id && pos.y >= sy - 0.5)
        })
        .collect();
    assert!(
        stationary.is_empty(),
        "after {STEPS} steps every sphere must have fallen at least 0.5 m towards the floor, or \
         the bit-exact control comparison would be comparing two frozen worlds; spheres that did \
         not: {stationary:?} (final positions {:?})",
        scene.gathered
    );
}
