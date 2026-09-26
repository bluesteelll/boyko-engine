//! L10 C3b (`docs/physics/perf-campaign/levers/L10-sleeping/`, design 04 "Gates" +
//! 06 §7 + 08 §7 and the rulings in `levers/00-RULINGS.md`): the frozen-pair skip
//! (`SleepSkip::Sets`) reproduces the oracle (`SleepSkip::Off`) bit for bit, compared on EVERY
//! step.
//!
//! Every scene is built twice — identical bodies, identical configuration, sleeping on in both —
//! and the two worlds differ only in `PhysicsConfig::sleep_skip`. They are stepped in lockstep,
//! every script edit is applied to both before the step it names, and after each step the
//! observation below must be equal:
//!
//! * every `RigidBody` bit; every row's sleep latch (asleep, debounce, island key) through
//!   `IslandSleep::latch_fingerprint`, every row's awake bit, and `contact_wakes`;
//! * the logical views: `ContactPairs::pairs`, `Manifolds::manifolds` as bytes (152 B each, no
//!   implicit padding) and `sensor_overlaps`;
//! * the islands: `n_islands`, `island_of` per row, `island_len`, `island(i)` resolved by value
//!   and through `position_of` (as a set, against `Off`'s index list), `max_island_constraints`,
//!   `is_island_frozen` per id;
//! * `WarmSeedStats` (the carry fields logical, design 06 E6′);
//! * the hysteresis table's lookup for every logical box pair (not its bytes);
//! * the warm seed the next step would find for every point of every logical manifold
//!   (`ColoredSoftStepSolver::for_each_warm_seed`: stream records ⊎ kept records);
//! * the pair tags (`Manifolds::for_each_pair_tag`, over the LOGICAL pairs): a collided slot
//!   byte-equal to `Off`'s; a held slot — or a pair the tree withheld, which has no slot (L10
//!   C3c) — must carry the held-skip tag, and its kept copy must equal `Off`'s tag on every bit
//!   but `HIT` and `SEPHIT` when `Off`'s tag carries kept state, and be absent otherwise
//!   (design 08 §7).
//!
//! On top of the compare, every tree step of either world is checked against the pair-set
//! oracle (L10 C3c): a probe system between the broadphase and the narrowphase compares the
//! logical pairs (the stream ⊎ the pairs the tree withholds) with `all_pairs_into` over the
//! snapshot both read.
//!
//! # Scenes
//!
//! * the lane's sleeping-on fixture scenes (`docs/measurements/2026-09-23-l10-sleeping/`): the
//!   rest pile (`R-S`, `R-on`: the default configuration) and the Jolt pile at a 0.5 gap (`J-Son`
//!   / `J-A-on`: cfg-A; `J-D-on`: the default configuration), at W ∈ {1, 8};
//! * S1, the rest pile, and S2, the Jolt pile, × {Tree (its sleeper set withholds the held
//!   pairs since L10 C3c), Grid with the parallel emit} × the parallel narrowphase ×
//!   `contact_reuse` ∈ {on, off} × W ∈ {1, 8}; S1 also on AllPairs, and at W ∈ {2, 4, 16} in
//!   release. On a Tree cell the `Sets` world must withhold pairs (a void otherwise), and at
//!   W = 8 a step whose stream is empty must dispatch no narrowphase chunk though its logical
//!   pairs would (design 06 N13, "Tree all-held dispatch = 0");
//! * S3, the adversarial arms (each its own test, named for what it drives);
//! * S4, a pile on an SDF floor with a mid-run field edit and a kernel toggle, on the default
//!   configuration and on the Tree with its brute path off (the SDF pipeline's tree seam);
//! * S5, a coupled soft body landing on a held pile (Grid forced).
//!
//! # Mid-step writes (L10 D9b, `levers/L10-sleeping/10-DESIGN-D9B.md`)
//!
//! Two writers run in every world of every arm: `mid_early` between the broadphase and the
//! narrowphase, `mid_late` between the last manifold producer and the solve — the places a
//! gameplay system with no ordering against the physics block can land. A script queues writes in
//! them ([`MidWrites`]); each records what it saw before its own write.
//!
//! * Suite A (`d9b_defer_*`): deferral equivalence. A world M takes a write inside step `at`'s
//!   window, a world B takes the same write at the next step boundary, and the two must match
//!   after every step; a control C, which takes the write before step `at`, must differ from B,
//!   or the arm is void.
//! * Suite B (`s3_mid_step_*`, `s4_mid_step_*`): the lockstep compare above under window writes.
//!
//! The rig also runs the pipelines without `IslandSleep` (the reference solver), where the
//! observation keeps the bodies, the views and the soft particles.
//!
//! # Anti-vacuity
//!
//! Each test states what it must observe (held rows, restores by rule, cross copies, restore
//! lookups, …) from `SleepSets::stats` / `SleepSets::rule_counts` of the `Sets` world; a run in
//! which nothing it is about happened fails as void.
//!
//! The piles are the runner's 15-layer pyramid (1240 bodies) in a release build and a 6-layer one
//! in a debug build, where two lockstep worlds of the full pile would not finish in a reasonable
//! sweep.
//!
//! Spins real thread pools (intractable under Miri), so `cfg(not(miri))`.

#![cfg(not(miri))]

use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_macros::{Component, Resource};
use boyko_sdf_math::{SdfEdit, sdf_op};
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, Kinematic, RigidBody, RigidBodyMass, Sensor, Simulated,
};
use boyko_physics::manifold::{BodyIndex, Manifold, SDF_SENTINEL};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::{
    add_physics_sdf, add_physics_soft, add_physics_soft_colored, add_physics_systems,
};
use boyko_physics::broadphase_tree::{BroadphaseTree, TreeDiag, all_pairs_into};
use boyko_physics::resources::{
    BroadphaseKind, BroadphaseSelectMode, ConstraintGraph, ContactPairs, IslandSleep, Manifolds,
    PairTagProbe, PhysicsConfig, SdfNarrowphaseKernel, SleepSkip, SolverScratch,
};
use boyko_physics::sdf_query::SdfField;
use boyko_physics::sleep_sets::{SleepRuleCounts, SleepSets, SleepSkipStats};
use boyko_physics::soft::SoftBody;
use boyko_physics::solver::{
    ColoredSoftStepSolver, DefaultRigidSolver, SoftStepSolver, WarmSeedStats,
};
use boyko_physics::systems::physics_gather;

// ── Constants ──────────────────────────────────────────────────────────────────────────────

/// The fixed step (60 Hz).
const DT: f32 = 1.0 / 60.0;
/// The pyramid's box pitch (Jolt's `cBoxSize`).
const BOX_SIZE: f32 = 2.0;
/// A pyramid box's half-extent.
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
/// A pyramid box's inverse mass (unit density, side 2: m = 8).
const BOX_INV_MASS: f32 = 0.125;
/// Its inverse inertia, uniform on the diagonal.
const BOX_INV_INERTIA: f32 = 0.1875;
/// The piles' floor half-extents.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
/// The pyramids' friction (the runner's `rest` and `jolt` scenes).
const FRICTION: f32 = 0.5;
/// The Jolt pile's spawn gap.
const JOLT_GAP: f32 = 0.5;
/// The HELD_SKIP tag (design 06 B1, `0x0C00`).
const HELD_SKIP: u16 = 0x0C00;
/// The tag's reuse-record bit (L9, bit 8).
const TAG_REC: u16 = 1 << 8;
/// The tag's separated bit (L9, bit 9).
const TAG_SEP: u16 = 1 << 9;

/// The pyramids' layers: the runner's 15 in release, 6 in debug.
const fn pyramid_height() -> i32 {
    if cfg!(debug_assertions) { 6 } else { 15 }
}

/// The fixture scenes' step count: the fixtures' 600 in release, fewer in debug.
const fn fixture_steps() -> usize {
    if cfg!(debug_assertions) { 300 } else { 600 }
}

// ── Bodies ─────────────────────────────────────────────────────────────────────────────────

/// A marker for the archetype created BEFORE the pile's: a body spawned into it takes a row
/// ahead of every pile row, so every pile row moves by one on that gather (a monotone Rows step
/// with no jumper).
#[derive(Component, Clone, Copy, Debug, Default)]
struct Front {
    _tag: u8,
}

/// Which archetype a body goes into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arch {
    /// The main archetype (the pile, the floor, most scene bodies).
    Main,
    /// The archetype ahead of it ([`Front`]).
    Front,
    /// The sensor archetype (after the main one).
    Sensor,
}

/// One body to spawn.
#[derive(Clone, Copy, Debug)]
struct Spec {
    position: Vec3,
    rotation: Quat,
    velocity: Vec3,
    shape: ColliderShape,
    /// `0` for an immovable body.
    inv_mass: f32,
    inv_inertia: f32,
    friction: f32,
    arch: Arch,
    kinematic: bool,
    /// Whether a dynamic body is enabled as `Simulated` (a cleared bit parks it).
    simulated: bool,
}

impl Spec {
    /// A dynamic cube of half-extent `half` at `position`, unit density.
    fn cube(position: Vec3, half: f32) -> Self {
        let m = 8.0 * half * half * half;
        Self {
            position,
            rotation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            shape: ColliderShape::Box { half_extents: Vec3::new(half, half, half) },
            inv_mass: 1.0 / m,
            inv_inertia: 1.0 / (m * (2.0 * half * 2.0 * half) / 6.0),
            friction: FRICTION,
            arch: Arch::Main,
            kinematic: false,
            simulated: true,
        }
    }

    /// A pyramid box.
    fn pile_box(position: Vec3) -> Self {
        Self {
            inv_mass: BOX_INV_MASS,
            inv_inertia: BOX_INV_INERTIA,
            ..Self::cube(position, HALF_BOX)
        }
    }

    /// A dynamic sphere of radius `r`.
    fn ball(position: Vec3, r: f32) -> Self {
        let m = 4.0 / 3.0 * std::f32::consts::PI * r * r * r;
        Self {
            shape: ColliderShape::Sphere { radius: r },
            inv_mass: 1.0 / m,
            inv_inertia: 1.0 / (0.4 * m * r * r),
            ..Self::cube(position, r)
        }
    }

    /// An immovable box of half-extents `half` at `position`.
    fn wall(position: Vec3, half: Vec3) -> Self {
        Self {
            shape: ColliderShape::Box { half_extents: half },
            inv_mass: 0.0,
            inv_inertia: 0.0,
            ..Self::cube(position, 1.0)
        }
    }

    /// The piles' floor.
    fn floor() -> Self {
        Self::wall(Vec3::new(0.0, -1.0, 0.0), FLOOR_HALF_EXTENTS)
    }

    fn at(self, arch: Arch) -> Self {
        Self { arch, ..self }
    }

    fn turned(self, rotation: Quat) -> Self {
        Self { rotation, ..self }
    }

    fn moving(self, velocity: Vec3) -> Self {
        Self { velocity, ..self }
    }
}

/// A rotation of `angle` radians about +Y.
fn yaw(angle: f32) -> Quat {
    let (s, c) = (0.5 * angle).sin_cos();
    Quat::new(0.0, s, 0.0, c)
}

/// Views a `#[repr(C)]` POD value as its bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, initialised `#[repr(C)]` `T` borrowed for the returned slice's
    // lifetime; the slice covers exactly its `size_of::<T>()` bytes, read-only, which is the
    // layout the component pool stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// The three archetypes, created in this order so the front one sorts first.
#[derive(Clone, Copy)]
struct Archetypes {
    front: ArchetypeId,
    main: ArchetypeId,
    sensor: ArchetypeId,
}

impl Archetypes {
    fn create(world: &mut EcsMaster) -> Self {
        let base = [RigidBody::component_id(), RigidBodyMass::component_id(), Collider::component_id()];
        let front = world.create_archetype(&[base[0], base[1], base[2], Front::component_id()]);
        let main = world.create_archetype(&base);
        let sensor = world.create_archetype(&[base[0], base[1], base[2], Sensor::component_id()]);
        Self { front, main, sensor }
    }
}

/// Spawns `spec` into `world`.
fn spawn(world: &mut EcsMaster, arch: Archetypes, spec: &Spec) -> Entity {
    let body = RigidBody {
        position: spec.position,
        linear_velocity: spec.velocity,
        rotation: spec.rotation,
        angular_velocity: Vec3::ZERO,
    };
    let i = spec.inv_inertia;
    let mass = RigidBodyMass {
        inv_inertia: if spec.inv_mass == 0.0 { Mat3::ZERO } else { Mat3::from_diagonal(Vec3::new(i, i, i)) },
        inv_mass: spec.inv_mass,
        restitution: 0.0,
        friction: spec.friction,
    };
    let collider = Collider { shape: spec.shape, layer: 1, mask: 1 };
    let base: [(_, &[u8]); 3] = [
        (RigidBody::component_id(), as_bytes(&body)),
        (RigidBodyMass::component_id(), as_bytes(&mass)),
        (Collider::component_id(), as_bytes(&collider)),
    ];
    let front = Front::default();
    let sensor = Sensor;
    let created = match spec.arch {
        Arch::Main => world.create_entity(arch.main, &base),
        Arch::Front => world.create_entity(
            arch.front,
            &[base[0], base[1], base[2], (Front::component_id(), as_bytes(&front))],
        ),
        Arch::Sensor => world.create_entity(
            arch.sensor,
            &[base[0], base[1], base[2], (Sensor::component_id(), as_bytes(&sensor))],
        ),
    };
    let e = created.expect("construction: every archetype accepts its columns");
    if spec.inv_mass != 0.0 && spec.simulated {
        world.enable::<Simulated>(e);
    }
    if spec.kinematic {
        world.enable::<Kinematic>(e);
    }
    e
}

/// The rest pile (gap 0) or the Jolt pile (`gap`), `height` layers, Jolt's placement order,
/// optionally offset by `(ox, oz)`.
fn pyramid(height: i32, gap: f32, ox: f32, oz: f32) -> Vec<Spec> {
    let mut out = Vec::new();
    for i in 0..height {
        let lo = i / 2;
        let hi = height - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                out.push(Spec::pile_box(Vec3::new(
                    ox - (height as f32) + BOX_SIZE * j as f32 + odd,
                    1.0 + (BOX_SIZE + gap) * i as f32,
                    oz - (height as f32) + BOX_SIZE * k as f32 + odd,
                )));
            }
        }
    }
    out
}

/// A tower of `n` cubes of half-extent `half` at `(x, z)`, each exactly touching the one below
/// (the floor's top face is `y = 0`), so it settles at once and freezes after the debounce.
fn tower(n: usize, x: f32, z: f32, half: f32) -> Vec<Spec> {
    (0..n).map(|i| Spec::cube(Vec3::new(x, half + 2.0 * half * i as f32, z), half)).collect()
}

// ── The rig ────────────────────────────────────────────────────────────────────────────────

/// Which pipeline a scene runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pipeline {
    /// `add_physics_systems::<DefaultRigidSolver>`: the default world.
    Default,
    /// `add_physics_sdf::<DefaultRigidSolver>`, with a floor field.
    Sdf,
    /// `add_physics_soft::<DefaultRigidSolver>(.., coupling = true)`.
    SoftCoupled,
    /// `add_physics_soft::<DefaultRigidSolver>(.., coupling = false)`.
    Soft,
    /// `add_physics_soft_colored::<DefaultRigidSolver>`.
    SoftColored,
    /// `add_physics_soft::<SoftStepSolver>(.., coupling = true)`: the reference broadphase and
    /// solve, no `IslandSleep`; sleeping off only.
    SoftCoupledRef,
}

/// The configuration knobs a variant sets on top of the pipeline's defaults.
#[derive(Clone, Copy, Debug)]
struct Variant {
    workers: usize,
    /// `None` keeps the default broadphase.
    kind: Option<BroadphaseKind>,
    parallel_bp: Option<bool>,
    parallel_np: Option<bool>,
    parallel_solve: Option<bool>,
    simd_solve: Option<bool>,
    reuse: Option<bool>,
    /// The tree's brute threshold (`Some(0)`: every step is a tree-path step).
    brute_max_rows: Option<u32>,
}

impl Variant {
    /// The default configuration at `workers` workers.
    const fn default_cfg(workers: usize) -> Self {
        Self {
            workers,
            kind: None,
            parallel_bp: None,
            parallel_np: None,
            parallel_solve: None,
            simd_solve: None,
            reuse: None,
            brute_max_rows: None,
        }
    }

    /// The runner's cfg-A at `workers` workers: AllPairs, the three parallel flags on iff W > 1,
    /// the scalar solve.
    const fn cfg_a(workers: usize) -> Self {
        let p = workers > 1;
        Self {
            workers,
            kind: Some(BroadphaseKind::AllPairs),
            parallel_bp: Some(p),
            parallel_np: Some(p),
            parallel_solve: Some(p),
            simd_solve: Some(false),
            reuse: None,
            brute_max_rows: None,
        }
    }

    /// The design's matrix cell: `kind`, the parallel narrowphase on, `reuse`, W.
    const fn cell(kind: BroadphaseKind, reuse: bool, workers: usize) -> Self {
        Self {
            workers,
            kind: Some(kind),
            parallel_bp: Some(true),
            parallel_np: Some(true),
            parallel_solve: None,
            simd_solve: None,
            reuse: Some(reuse),
            brute_max_rows: None,
        }
    }

    fn apply(&self, cfg: &mut PhysicsConfig) {
        if let Some(kind) = self.kind {
            cfg.broadphase_select = BroadphaseSelectMode::Manual;
            cfg.broadphase = kind;
        }
        if let Some(v) = self.parallel_bp {
            cfg.parallel_broadphase = v;
        }
        if let Some(v) = self.parallel_np {
            cfg.parallel_narrowphase = v;
        }
        if let Some(v) = self.parallel_solve {
            cfg.parallel_solve = v;
        }
        if let Some(v) = self.simd_solve {
            cfg.simd_solve = v;
        }
        if let Some(v) = self.reuse {
            cfg.contact_reuse = v;
        }
    }
}

/// The tree's pair-set oracle (L10 C3c, module docs): runs between the broadphase and the
/// narrowphase, on the snapshot both read. A step of another kind is not compared: it withholds
/// nothing, and the lockstep compare with `Off` covers its pair set.
#[derive(Resource)]
struct PairOracle {
    /// All-pairs' set of the step, rebuilt in place.
    oracle: ContactPairs,
    /// Tree steps compared.
    compared: u64,
    /// Tree steps compared while the sleeper set held a row.
    with_sleepers: u64,
    /// Tree steps whose logical pairs differed from all-pairs'.
    mismatches: u64,
}

impl Default for PairOracle {
    fn default() -> Self {
        Self { oracle: ContactPairs::with_capacity(0), compared: 0, with_sleepers: 0, mismatches: 0 }
    }
}

// `clippy::needless_pass_by_value`: `Res` / `ResMut` are by-value `SystemParam`s.
#[allow(clippy::needless_pass_by_value)]
fn oracle_pairs(
    scratch: Res<SolverScratch>,
    cfg: Res<PhysicsConfig>,
    tree: Res<BroadphaseTree>,
    pairs: Res<ContactPairs>,
    mut probe: ResMut<PairOracle>,
) {
    if cfg.broadphase != BroadphaseKind::Tree {
        return;
    }
    let probe = &mut *probe;
    all_pairs_into(scratch.bodies(), &mut probe.oracle);
    probe.compared += 1;
    probe.with_sleepers += u64::from(tree.sleeper_members() > 0);
    probe.mismatches += u64::from(probe.oracle.pairs() != pairs.pairs());
}

/// Writes a script queues for the middle of a step (review W1 of L10 C3c; design 10, Tests A).
/// Each writer drains its own queue — [`mid_early`] between the broadphase and the narrowphase,
/// [`mid_late`] between the last manifold producer and the solve — where a gameplay system with no
/// ordering against the physics block can run. Both writers are registered in every world, so
/// every world of an arm runs the same schedule graph; a writer with nothing queued only records
/// what it saw.
#[derive(Default)]
struct MidWrites {
    /// A configuration write, taken by the next run.
    cfg: Option<fn(&mut PhysicsConfig)>,
    /// A field assigned wholesale, then taken.
    field: Option<SdfField>,
    /// One `IslandSleep::wake_all()`, then cleared.
    wake: bool,
    /// Runs that performed a write.
    applied: u32,
    /// What this writer read on its latest run, BEFORE its own write: what a stage after it that
    /// read the resources live would see, an earlier writer's write included.
    seen: Option<(PhysicsConfig, Option<SdfField>)>,
}

impl MidWrites {
    /// Records what the writer sees, then performs its queued writes.
    fn drain(&mut self, cfg: &mut PhysicsConfig, field: Option<&mut SdfField>, sleep: Option<&mut IslandSleep>) {
        self.seen = Some((*cfg, field.as_deref().copied()));
        let mut wrote = false;
        if let Some(write) = self.cfg.take() {
            write(cfg);
            wrote = true;
        }
        if let Some(value) = self.field.take() {
            *field.expect("script: a field write needs an SdfField") = value;
            wrote = true;
        }
        if std::mem::take(&mut self.wake) {
            sleep.expect("script: a wake needs IslandSleep").wake_all();
            wrote = true;
        }
        self.applied += u32::from(wrote);
    }
}

/// The early writer's queue.
#[derive(Resource, Default)]
struct MidEarly(MidWrites);

/// The late writer's queue.
#[derive(Resource, Default)]
struct MidLate(MidWrites);

// `clippy::needless_pass_by_value`: `Res` / `ResMut` are by-value `SystemParam`s.
#[allow(clippy::needless_pass_by_value)]
fn mid_early(
    mut cfg: ResMut<PhysicsConfig>,
    mut field: Option<ResMut<SdfField>>,
    mut sleep: Option<ResMut<IslandSleep>>,
    mut mid: ResMut<MidEarly>,
) {
    mid.0.drain(&mut cfg, field.as_deref_mut(), sleep.as_deref_mut());
}

// `clippy::needless_pass_by_value`: `Res` / `ResMut` are by-value `SystemParam`s.
#[allow(clippy::needless_pass_by_value)]
fn mid_late(
    mut cfg: ResMut<PhysicsConfig>,
    mut field: Option<ResMut<SdfField>>,
    mut sleep: Option<ResMut<IslandSleep>>,
    mut mid: ResMut<MidLate>,
) {
    mid.0.drain(&mut cfg, field.as_deref_mut(), sleep.as_deref_mut());
}

/// Where a mid-step write lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// [`mid_early`]: after the broadphase, before the narrowphase.
    Early,
    /// [`mid_late`]: after the last manifold producer, before the solve.
    Late,
}

/// One world of a lockstep pair.
struct Rig {
    world: EcsMaster,
    schedule: Schedule,
    arch: Archetypes,
    /// Every body spawned, in spawn order (a despawned one stays, its slot stale).
    bodies: Vec<Entity>,
    /// The world's own mode: `Off` for the oracle, `Sets` for the world under test (`Off` with
    /// sleeping off).
    mode: SleepSkip,
}

impl Rig {
    fn new(pipeline: Pipeline, variant: Variant, mode: SleepSkip, specs: &[Spec]) -> Self {
        Self::with_mode(pipeline, variant, Some(mode), specs)
    }

    /// A world with sleeping on in `mode`, or off for `None`.
    fn with_mode(pipeline: Pipeline, variant: Variant, mode: Option<SleepSkip>, specs: &[Spec]) -> Self {
        let mut world = EcsMaster::new();
        let arch = Archetypes::create(&mut world);
        let bodies = specs.iter().map(|s| spawn(&mut world, arch, s)).collect();
        let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(variant.workers).build());
        let keys = match pipeline {
            Pipeline::Default => add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world),
            Pipeline::Sdf => {
                let keys = add_physics_sdf::<DefaultRigidSolver>(&mut builder, &mut world);
                *world.resource_mut::<SdfField>() = sdf_floor(0.0);
                keys
            }
            Pipeline::SoftCoupled => add_physics_soft::<DefaultRigidSolver>(&mut builder, &mut world, true),
            Pipeline::Soft => add_physics_soft::<DefaultRigidSolver>(&mut builder, &mut world, false),
            Pipeline::SoftColored => add_physics_soft_colored::<DefaultRigidSolver>(&mut builder, &mut world),
            Pipeline::SoftCoupledRef => add_physics_soft::<SoftStepSolver>(&mut builder, &mut world, true),
        };
        world.insert_resource(PairOracle::default());
        {
            // `PhysicsStageKeys` carries each stage's `SystemKey` inner index; the type is not
            // nameable here but its field is public (the tree scenes' probe does the same).
            let probe = builder.add_system(oracle_pairs);
            let mut after = probe.key();
            after.0 = keys.broadphase;
            let mut before = probe.key();
            before.0 = keys.narrowphase;
            probe.after(after).before(before);
        }
        world.insert_resource(MidEarly::default());
        world.insert_resource(MidLate::default());
        {
            let writer = builder.add_system(mid_early);
            let mut after = writer.key();
            after.0 = keys.broadphase;
            let mut before = writer.key();
            before.0 = keys.narrowphase;
            writer.after(after).before(before);
        }
        {
            // After the last manifold producer (`physics_build_graph` shares no resource with
            // it, so it is not ordered against it), before the solve.
            let writer = builder.add_system(mid_late);
            let mut after = writer.key();
            after.0 = keys.narrowphase_sdf.unwrap_or(keys.narrowphase);
            let mut before = writer.key();
            before.0 = keys.solve;
            writer.after(after).before(before);
        }
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        let schedule = builder.build(&mut world);
        {
            let cfg = world.resource_mut::<PhysicsConfig>();
            variant.apply(cfg);
            cfg.sleeping = mode.is_some();
            if let Some(mode) = mode {
                cfg.sleep_skip = mode;
            }
        }
        if let Some(rows) = variant.brute_max_rows {
            world.resource_mut::<BroadphaseTree>().set_brute_max_rows(rows);
        }
        Self { world, schedule, arch, bodies, mode: mode.unwrap_or(SleepSkip::Off) }
    }

    /// The queue of the writer at `place`.
    fn mid(&mut self, place: Place) -> &mut MidWrites {
        match place {
            Place::Early => &mut self.world.resource_mut::<MidEarly>().0,
            Place::Late => &mut self.world.resource_mut::<MidLate>().0,
        }
    }

    /// The queue of the writer at `place`, read.
    fn mid_ref(&self, place: Place) -> &MidWrites {
        match place {
            Place::Early => &self.world.resource::<MidEarly>().0,
            Place::Late => &self.world.resource::<MidLate>().0,
        }
    }

    fn step(&mut self) {
        self.schedule.run(&mut self.world);
    }

    fn cfg(&mut self) -> &mut PhysicsConfig {
        self.world.resource_mut::<PhysicsConfig>()
    }

    fn body_mut(&mut self, k: usize) -> impl std::ops::DerefMut<Target = RigidBody> + '_ {
        self.world.get_component_mut::<RigidBody>(self.bodies[k]).expect("script: a tracked body is live")
    }

    fn spawn(&mut self, spec: &Spec) -> usize {
        let e = spawn(&mut self.world, self.arch, spec);
        self.bodies.push(e);
        self.bodies.len() - 1
    }

    fn despawn(&mut self, k: usize) {
        assert!(self.world.delete_entity(self.bodies[k]), "script: a tracked body is live");
    }
}

/// The SDF floor field with its top face at `y = top`.
fn sdf_floor(top: f32) -> SdfField {
    SdfField::from_edits(&[SdfEdit::box_shape([0.0, top - 50.0, 0.0], [50.0, 50.0, 50.0], sdf_op::UNION, 0.0)])
}

// ── The observation ────────────────────────────────────────────────────────────────────────

/// Everything the gate compares after a step (module docs). The sleep fields are `None` / empty
/// in a world without `IslandSleep`, the graph fields in one without a `ConstraintGraph`, and the
/// warm fields in one without the colored solver.
#[derive(Debug, PartialEq)]
struct Obs {
    bodies: Vec<[u32; 13]>,
    latch: Option<u64>,
    awake: Vec<bool>,
    contact_wakes: Option<u64>,
    pairs: Vec<(u32, u32)>,
    manifolds: Vec<[u8; 152]>,
    sensors: Vec<[u8; 152]>,
    n_islands: Option<u32>,
    island_of: Vec<u32>,
    island_len: Vec<u32>,
    /// Per island: the manifolds' positions in the logical view, sorted.
    island_positions: Vec<Vec<usize>>,
    /// Per island: the manifolds by value, sorted by bytes.
    island_values: Vec<Vec<[u8; 152]>>,
    max_island: Option<u32>,
    frozen: Vec<bool>,
    warm: Option<WarmSeedStats>,
    axes: Vec<Option<usize>>,
    /// `(key, fid, seed bits)`, sorted.
    seeds: Vec<(u64, u16, Option<[u32; 3]>)>,
    tags: Vec<PairTagProbe>,
    /// Every soft particle's position and velocity bits, bodies in query order (design 10, Tests
    /// A); empty without soft bodies.
    soft: Vec<[u32; 6]>,
}

fn manifold_bytes(m: &Manifold) -> [u8; 152] {
    let mut out = [0u8; 152];
    out.copy_from_slice(as_bytes(m));
    out
}

fn observe(world: &mut EcsMaster) -> Obs {
    let soft = {
        let q = world.query::<&SoftBody, ()>();
        q.iter()
            .flat_map(|b| {
                (0..b.pos_x.len()).map(move |i| {
                    [
                        b.pos_x[i].to_bits(),
                        b.pos_y[i].to_bits(),
                        b.pos_z[i].to_bits(),
                        b.vel_x[i].to_bits(),
                        b.vel_y[i].to_bits(),
                        b.vel_z[i].to_bits(),
                    ]
                })
            })
            .collect()
    };
    let bodies = {
        let q = world.query::<&RigidBody, ()>();
        q.iter()
            .map(|b| {
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
            })
            .collect()
    };
    let world = &*world;
    let rows = world.resource::<SolverScratch>().bodies_len();
    let bodies_state = world.resource::<SolverScratch>().bodies();
    let sleep = world.contains_resource::<IslandSleep>().then(|| world.resource::<IslandSleep>());
    let graph = world.contains_resource::<ConstraintGraph>().then(|| world.resource::<ConstraintGraph>());
    let solver = (world.contains_resource::<ColoredSoftStepSolver>() && world.contains_resource::<SleepSets>())
        .then(|| (world.resource::<ColoredSoftStepSolver>(), world.resource::<SleepSets>()));
    let pairs_res = world.resource::<ContactPairs>();
    let m = world.resource::<Manifolds>();
    let pairs: Vec<(u32, u32)> = pairs_res.pairs().iter().map(|&(a, b)| (a.0, b.0)).collect();
    let manifolds: Vec<[u8; 152]> = m.manifolds().iter().map(|m| manifold_bytes(&m)).collect();
    let n_islands = graph.map_or(0, ConstraintGraph::n_islands);
    let island_positions = graph.map_or_else(Vec::new, |graph| (0..n_islands)
        .map(|i| {
            let mut v: Vec<usize> = graph
                .island(i)
                .iter()
                .map(|h| m.position_of(h).expect("a handle of an island resolves"))
                .collect();
            v.sort_unstable();
            v
        })
        .collect());
    let island_values = graph.map_or_else(Vec::new, |graph| (0..n_islands)
        .map(|i| {
            let mut v: Vec<[u8; 152]> = graph
                .island(i)
                .iter()
                .map(|h| manifold_bytes(&m.get(h).expect("a handle of an island resolves")))
                .collect();
            v.sort_unstable();
            v
        })
        .collect());
    let is_box = |r: u32| matches!(bodies_state[r as usize].shape, ColliderShape::Box { .. });
    let axes = pairs
        .iter()
        .map(|&(a, b)| (is_box(a) && is_box(b)).then(|| m.box_axis_cache.get(BodyIndex(a), BodyIndex(b))).flatten())
        .collect();
    let mut seeds = Vec::new();
    if let Some((solver, sets)) = solver {
        solver.for_each_warm_seed(m, sets, |key, fid, seed| {
            seeds.push((key, fid, seed.map(|s| s.map(f32::to_bits))));
        });
    }
    seeds.sort_unstable();
    let mut tags = Vec::new();
    m.for_each_pair_tag(pairs_res, |p| tags.push(p));
    Obs {
        bodies,
        latch: sleep.map(IslandSleep::latch_fingerprint),
        awake: sleep.map_or_else(Vec::new, |sleep| (0..rows).map(|r| sleep.is_row_awake(r)).collect()),
        contact_wakes: sleep.map(IslandSleep::contact_wakes),
        pairs,
        manifolds,
        sensors: m.sensor_overlaps().iter().map(manifold_bytes).collect(),
        n_islands: graph.map(ConstraintGraph::n_islands),
        island_of: graph.map_or_else(Vec::new, |graph| (0..rows as u32).map(|r| graph.island_of(r)).collect()),
        island_len: graph.map_or_else(Vec::new, |graph| (0..n_islands).map(|i| graph.island_len(i)).collect()),
        island_positions,
        island_values,
        max_island: graph.map(ConstraintGraph::max_island_constraints),
        frozen: sleep.map_or_else(Vec::new, |sleep| (0..n_islands).map(|i| sleep.is_island_frozen(i)).collect()),
        warm: solver.map(|(solver, _)| solver.warm_seed_stats()),
        axes,
        seeds,
        tags,
        soft,
    }
}

/// The first difference between `off` and `sets`, named, or `None`.
fn diff(off: &Obs, sets: &Obs) -> Option<String> {
    diff_named(off, sets, ("Off", "Sets"), true)
}

/// The first difference between `off` and `sets` (named `names`), or `None`. With `lockstep` the
/// tags compare by the held-slot rule (module docs), `off` being the oracle; without it every field
/// compares exactly (two worlds in the same mode, design 10 Tests A).
fn diff_named(off: &Obs, sets: &Obs, names: (&str, &str), lockstep: bool) -> Option<String> {
    let (na, nb) = names;
    macro_rules! list {
        ($f:ident) => {
            if off.$f != sets.$f {
                return Some(first_diff(stringify!($f), names, &off.$f, &sets.$f));
            }
        };
    }
    macro_rules! scalar {
        ($f:ident) => {
            if off.$f != sets.$f {
                return Some(format!("{}: {na} {:?}, {nb} {:?}", stringify!($f), off.$f, sets.$f));
            }
        };
    }
    list!(bodies);
    scalar!(latch);
    list!(awake);
    scalar!(contact_wakes);
    list!(pairs);
    list!(manifolds);
    list!(sensors);
    scalar!(n_islands);
    list!(island_of);
    list!(island_len);
    list!(island_positions);
    list!(island_values);
    scalar!(max_island);
    list!(frozen);
    list!(axes);
    list!(seeds);
    scalar!(warm);
    list!(soft);
    if !lockstep {
        list!(tags);
        return None;
    }
    if off.tags.len() != sets.tags.len() {
        return Some(format!("tags: {} Off slots, {} Sets slots", off.tags.len(), sets.tags.len()));
    }
    for (k, (o, s)) in off.tags.iter().zip(&sets.tags).enumerate() {
        assert!(!o.held, "the Off world holds nothing");
        let ok = (o.a, o.b) == (s.a, s.b)
            && if s.held {
                s.slot == HELD_SKIP && s.invariant == if o.kept_class { o.invariant } else { None }
            } else {
                s.slot == o.slot
            };
        if !ok {
            return Some(format!("tag of stream slot {k}: Off {o:?}, Sets {s:?}"));
        }
    }
    None
}

/// The first index where two lists differ, with both elements, or their lengths.
fn first_diff<T: std::fmt::Debug + PartialEq>(name: &str, names: (&str, &str), a: &[T], b: &[T]) -> String {
    let (na, nb) = names;
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => format!("{name}[{i}] of {}/{}: {na} {:?}, {nb} {:?}", a.len(), b.len(), a[i], b[i]),
        None => format!("{name}: lengths {na} {}, {nb} {}", a.len(), b.len()),
    }
}

/// FNV-1a 64 over a list of manifolds' bytes.
fn manifolds_hash(manifolds: &[[u8; 152]]) -> u64 {
    manifolds.iter().flatten().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

// ── The lockstep driver ────────────────────────────────────────────────────────────────────

/// What the `Sets` world did over a run (the anti-vacuity evidence).
#[derive(Debug, Default)]
struct Evidence {
    steps: usize,
    /// Per step: the stats after the step.
    stats: Vec<SleepSkipStats>,
    /// Per step: rows frozen (not awake) after the step, dynamic ones only.
    frozen_rows: Vec<usize>,
    /// Per step: dynamic rows.
    dynamic_rows: Vec<usize>,
    /// Per step: held pairs whose kept tag carries a reuse record (`REC`).
    held_rec: Vec<usize>,
    /// Per step: held pairs whose kept tag carries a separating axis (`SEP`).
    held_sep: Vec<usize>,
    /// Per step: logical positions held this step whose occupant one step earlier was a
    /// computed box pair on a pre-read (non-Identity) frame — which commits its axis, whatever
    /// its hint — with an axis other than the held pair's. A held skip that left the slot's
    /// commit unwritten would re-key the held pair with that stale axis (design 08 N27). The
    /// position is the stream slot on every kind but the Tree, which withholds held pairs (L10
    /// C3c); arm 10 runs off the Tree.
    stale_slots: Vec<usize>,
    /// Per step: the `Sets` world's logical pairs, and of those the ones the tree withheld (L10
    /// C3c, design 04 T3), from its pair-tag probes.
    logical: Vec<usize>,
    /// See [`logical`](Self::logical).
    withheld: Vec<usize>,
    /// Per step: the `Sets` world's narrowphase dispatches (`0` or `1`).
    np_dispatches: Vec<u64>,
    /// Per step: the `Sets` world's tree counters and its sleeper-set size.
    tree: Vec<TreeDiag>,
    /// See [`tree`](Self::tree).
    sleepers: Vec<u64>,
    /// The `Sets` world's tree steps the pair-set oracle compared, and of those the ones with a
    /// live sleeper set ([`PairOracle`]).
    oracle_compared: u64,
    /// See [`oracle_compared`](Self::oracle_compared).
    oracle_with_sleepers: u64,
    /// The per-rule counts at the end.
    rules: SleepRuleCounts,
    /// Per step: the per-rule counts after the step.
    rules_at: Vec<SleepRuleCounts>,
    /// Per step: the `Off` world's reuse hits (`Manifolds::pair_classes().reused`).
    off_reused: Vec<u64>,
    /// Per step: the `Off` world's logical pair count (`Manifolds::pair_classes().pairs`).
    off_pairs: Vec<u64>,
    /// Per step: a hash of the `Off` world's logical manifolds' bytes.
    off_manifolds: Vec<u64>,
}

impl Evidence {
    fn held_steps(&self) -> usize {
        self.stats.iter().filter(|s| s.held_rows > 0).count()
    }

    fn sum(&self, f: impl Fn(&SleepSkipStats) -> u32) -> u64 {
        self.stats.iter().map(|s| u64::from(f(s))).sum()
    }
}

/// A script edit, applied to both worlds before a step.
type Script<'a> = dyn FnMut(usize, &mut Rig) + 'a;

/// Runs `specs` twice — `Off` and `Sets` — in lockstep for `steps` steps, `script(step, rig)`
/// applied to each world before step `step` (0-based), and compares every step. Panics on the
/// first difference with the step and the field.
fn lockstep(
    label: &str,
    pipeline: Pipeline,
    variant: Variant,
    specs: &[Spec],
    steps: usize,
    script: &mut Script<'_>,
) -> Evidence {
    lockstep_probed(label, pipeline, variant, specs, steps, script, &mut |_, _, _| {})
}

/// A probe run after every lockstep step on `(step, off, sets)`.
type Probe<'a> = dyn FnMut(usize, &Rig, &Rig) + 'a;

/// [`lockstep`] with `probe(step, off, sets)` run after every step and its compare.
fn lockstep_probed(
    label: &str,
    pipeline: Pipeline,
    variant: Variant,
    specs: &[Spec],
    steps: usize,
    script: &mut Script<'_>,
    probe: &mut Probe<'_>,
) -> Evidence {
    let mut off = Rig::new(pipeline, variant, SleepSkip::Off, specs);
    let mut sets = Rig::new(pipeline, variant, SleepSkip::Sets, specs);
    let mut ev = Evidence { steps, ..Evidence::default() };
    // The previous step's `Off` axes and `Sets` held flags by slot, and whether it pre-read.
    let mut prev: Option<(Vec<Option<usize>>, Vec<bool>, bool)> = None;
    for step in 0..steps {
        script(step, &mut off);
        script(step, &mut sets);
        let pre_reads = sets.world.resource::<Manifolds>().box_axis_cache.prefetched_frames();
        let dispatches = sets.world.resource::<Manifolds>().narrowphase_dispatches();
        off.step();
        sets.step();
        let pre_read = sets.world.resource::<Manifolds>().box_axis_cache.prefetched_frames() > pre_reads;
        ev.np_dispatches.push(sets.world.resource::<Manifolds>().narrowphase_dispatches() - dispatches);
        let tree = sets.world.resource::<BroadphaseTree>();
        ev.tree.push(tree.diag());
        ev.sleepers.push(tree.sleeper_members());
        for (name, rig) in [("Off", &off), ("Sets", &sets)] {
            let oracle = rig.world.resource::<PairOracle>();
            assert_eq!(
                oracle.mismatches, 0,
                "{label} ({name}) step {step}: the tree's logical pairs differ from all-pairs'"
            );
        }
        let oracle = sets.world.resource::<PairOracle>();
        (ev.oracle_compared, ev.oracle_with_sleepers) = (oracle.compared, oracle.with_sleepers);
        let (o, s) = (observe(&mut off.world), observe(&mut sets.world));
        if let Some(why) = diff(&o, &s) {
            let st = sets.world.resource::<SleepSets>();
            panic!(
                "{label}: Sets diverges from Off after step {step}: {why}\n  Sets stats {:?}\n  rules {:?}",
                st.stats(),
                st.rule_counts()
            );
        }
        let held_with = |bit: u16| s.tags.iter().filter(|t| t.held && t.invariant.is_some_and(|i| i & bit != 0)).count();
        ev.stale_slots.push(match &prev {
            Some((axes, held, true)) => s
                .tags
                .iter()
                .enumerate()
                .filter(|&(k, t)| {
                    t.held
                        && held.get(k) == Some(&false)
                        && matches!((axes.get(k), o.axes.get(k)), (Some(Some(x)), Some(Some(y))) if x != y)
                })
                .count(),
            _ => 0,
        });
        prev = Some((o.axes.clone(), s.tags.iter().map(|t| t.held).collect(), pre_read));
        ev.logical.push(s.tags.len());
        ev.withheld.push(s.tags.iter().filter(|t| t.withheld).count());
        ev.held_rec.push(held_with(TAG_REC));
        ev.held_sep.push(held_with(TAG_SEP));
        let st = sets.world.resource::<SleepSets>();
        ev.stats.push(st.stats());
        let scratch = sets.world.resource::<SolverScratch>();
        let sleep = sets.world.resource::<IslandSleep>();
        let dynamic: Vec<usize> =
            (0..scratch.bodies_len()).filter(|&r| scratch.bodies()[r].inv_mass != 0.0).collect();
        ev.frozen_rows.push(dynamic.iter().filter(|&&r| !sleep.is_row_awake(r)).count());
        ev.dynamic_rows.push(dynamic.len());
        ev.rules_at.push(st.rule_counts());
        let classes = off.world.resource::<Manifolds>().pair_classes();
        ev.off_reused.push(classes.reused);
        ev.off_pairs.push(classes.pairs);
        ev.off_manifolds.push(manifolds_hash(&o.manifolds));
        probe(step, &off, &sets);
    }
    ev.rules = sets.world.resource::<SleepSets>().rule_counts();
    if std::env::var_os("L10_ARM_TRACE").is_some() {
        for (name, rig) in [("Off", &off), ("Sets", &sets)] {
            let c = &rig.world.resource::<Manifolds>().box_axis_cache;
            println!(
                "{label} {name}: axis table load clears {}, grows {}, pre-read frames {}",
                c.load_clears(),
                c.grows(),
                c.prefetched_frames()
            );
        }
    }
    ev
}

/// A script that does nothing.
fn quiet(_: usize, _: &mut Rig) {}

/// The design's S1 anti-vacuity: held rows equal the dynamic rows on at least 90 % of the steps
/// on which every dynamic row is frozen.
fn assert_all_held_when_frozen(label: &str, ev: &Evidence) {
    let frozen: Vec<usize> =
        (0..ev.steps).filter(|&k| ev.dynamic_rows[k] > 0 && ev.frozen_rows[k] == ev.dynamic_rows[k]).collect();
    assert!(!frozen.is_empty(), "{label}: anti-vacuity: no step froze every dynamic row");
    let held = frozen.iter().filter(|&&k| ev.stats[k].held_rows as usize == ev.dynamic_rows[k]).count();
    assert!(
        held * 10 >= frozen.len() * 9,
        "{label}: anti-vacuity: every dynamic row was held on {held} of the {} all-frozen steps (< 90 %)",
        frozen.len()
    );
    println!("{label}: {} all-frozen steps, all held on {held}; rules {:?}", frozen.len(), ev.rules);
}

/// L10 C3c's anti-vacuity on a Tree cell (design 04 T3, 06 N13): the `Sets` world withheld pairs
/// — its sleeper set was live — and the pair-set oracle compared its tree steps, those with a
/// live sleeper set among them; and at W ≥ 2 every step whose stream was empty dispatched no
/// narrowphase chunk, and at least one such step held two chunks' worth of logical pairs (so a
/// chunk count read from the logical view would have dispatched there).
fn assert_tree_withholds(label: &str, ev: &Evidence, workers: usize) {
    let steps = (0..ev.steps).filter(|&k| ev.withheld[k] > 0).count();
    let most = ev.withheld.iter().copied().max().unwrap_or(0);
    assert!(steps > 0, "{label}: void: the tree withheld no pair");
    assert!(
        ev.oracle_compared == ev.steps as u64 && ev.oracle_with_sleepers > 0,
        "{label}: void: the pair-set oracle compared {} of {} steps, {} with a live sleeper set",
        ev.oracle_compared,
        ev.steps,
        ev.oracle_with_sleepers
    );
    let mut all_held = 0usize;
    if workers >= 2 {
        let two_chunks = 2 * boyko_physics::narrowphase::NP_MIN_PAIRS_PER_CHUNK;
        for k in 0..ev.steps {
            if ev.withheld[k] == ev.logical[k] {
                assert_eq!(
                    ev.np_dispatches[k], 0,
                    "{label} step {k}: an empty stream dispatched the narrowphase ({} logical pairs)",
                    ev.logical[k]
                );
                all_held += usize::from(ev.logical[k] >= two_chunks);
            }
        }
        assert!(
            all_held > 0,
            "{label}: void: no step had an empty stream beside {two_chunks} logical pairs"
        );
    }
    let last = ev.tree.last().copied().unwrap_or_default();
    println!(
        "{label}: withheld pairs on {steps} steps (at most {most}); empty-stream W≥2 steps {all_held}; \
         sleepers at the end {}; tree {last:?}",
        ev.sleepers.last().copied().unwrap_or(0)
    );
}

// ── The fixture scenes ─────────────────────────────────────────────────────────────────────

fn rest_pile() -> Vec<Spec> {
    let mut v = vec![Spec::floor()];
    v.extend(pyramid(pyramid_height(), 0.0, 0.0, 0.0));
    v
}

fn jolt_pile() -> Vec<Spec> {
    let mut v = vec![Spec::floor()];
    v.extend(pyramid(pyramid_height(), JOLT_GAP, 0.0, 0.0));
    v
}

/// `R-S` / `R-on`: the rest pile, the default configuration, at W ∈ {1, 8}.
#[test]
fn fixture_rest_pile_default_cfg() {
    for w in [1, 8] {
        let label = format!("R-S W{w}");
        let ev = lockstep(&label, Pipeline::Default, Variant::default_cfg(w), &rest_pile(), fixture_steps(), &mut quiet);
        assert_all_held_when_frozen(&label, &ev);
    }
}

/// `J-Son` / `J-A-on`: the Jolt pile, cfg-A, at W ∈ {1, 8}.
#[test]
fn fixture_jolt_pile_cfg_a() {
    for w in [1, 8] {
        let label = format!("J-Son W{w}");
        let ev = lockstep(&label, Pipeline::Default, Variant::cfg_a(w), &jolt_pile(), fixture_steps(), &mut quiet);
        assert_all_held_when_frozen(&label, &ev);
    }
}

/// `J-D-on`: the Jolt pile, the default configuration, at W ∈ {1, 8}.
#[test]
fn fixture_jolt_pile_default_cfg() {
    for w in [1, 8] {
        let label = format!("J-D-on W{w}");
        let ev = lockstep(&label, Pipeline::Default, Variant::default_cfg(w), &jolt_pile(), fixture_steps(), &mut quiet);
        assert_all_held_when_frozen(&label, &ev);
    }
}

// ── S1 / S2: the piles across the design's matrix ──────────────────────────────────────────

/// The steps of a matrix run: past the freeze, then held.
const fn matrix_steps() -> usize {
    if cfg!(debug_assertions) { 260 } else { 400 }
}

/// S1: the rest pile × {Tree, Grid-parallel, AllPairs} × `contact_reuse` ∈ {on, off} × W ∈ {1, 8},
/// the parallel narrowphase on.
#[test]
fn s1_rest_pile_matrix() {
    for kind in [BroadphaseKind::Tree, BroadphaseKind::Grid, BroadphaseKind::AllPairs] {
        for reuse in [false, true] {
            for w in [1, 8] {
                let label = format!("S1 {kind:?} reuse {reuse} W{w}");
                let variant = Variant::cell(kind, reuse, w);
                let ev = lockstep(&label, Pipeline::Default, variant, &rest_pile(), matrix_steps(), &mut quiet);
                assert_all_held_when_frozen(&label, &ev);
                if kind == BroadphaseKind::Tree {
                    assert_tree_withholds(&label, &ev, w);
                }
            }
        }
    }
}

/// S1 at W ∈ {2, 4, 16} (release only: the sweep is long in a debug build).
#[test]
#[cfg(not(debug_assertions))]
fn s1_rest_pile_more_workers() {
    for w in [2, 4, 16] {
        for kind in [BroadphaseKind::Tree, BroadphaseKind::Grid] {
            let label = format!("S1 {kind:?} W{w}");
            let variant = Variant::cell(kind, true, w);
            let ev = lockstep(&label, Pipeline::Default, variant, &rest_pile(), matrix_steps(), &mut quiet);
            assert_all_held_when_frozen(&label, &ev);
            if kind == BroadphaseKind::Tree {
                assert_tree_withholds(&label, &ev, w);
            }
        }
    }
}

/// S1 on the Tree with `brute_max_rows = 0`: every step a tree-path step, the design's second
/// Tree arm beside the default one (L10 C3c: its sleeper set withholds the held pairs).
#[test]
fn s1_rest_pile_tree_path_every_step() {
    for w in [1, 8] {
        let label = format!("S1 Tree brute 0 W{w}");
        let variant = Variant { brute_max_rows: Some(0), ..Variant::cell(BroadphaseKind::Tree, true, w) };
        let ev = lockstep(&label, Pipeline::Default, variant, &rest_pile(), matrix_steps(), &mut quiet);
        assert_all_held_when_frozen(&label, &ev);
        assert_tree_withholds(&label, &ev, w);
    }
}

/// S2: the Jolt pile × {Tree, Grid-parallel} × `contact_reuse` ∈ {on, off} × W ∈ {1, 8}.
#[test]
fn s2_jolt_pile_matrix() {
    let steps = if cfg!(debug_assertions) { 300 } else { 1000 };
    for kind in [BroadphaseKind::Tree, BroadphaseKind::Grid] {
        for reuse in [false, true] {
            for w in [1, 8] {
                let label = format!("S2 {kind:?} reuse {reuse} W{w}");
                let variant = Variant::cell(kind, reuse, w);
                let ev = lockstep(&label, Pipeline::Default, variant, &jolt_pile(), steps, &mut quiet);
                assert_all_held_when_frozen(&label, &ev);
                if kind == BroadphaseKind::Tree {
                    assert_tree_withholds(&label, &ev, w);
                }
            }
        }
    }
}

// ── S3: the adversarial arms ───────────────────────────────────────────────────────────────

/// The small scene most arms start from: the floor and three towers of three unit cubes, far
/// enough apart to be three islands. The towers' cubes are indices `1 + 3 t + level`.
fn towers() -> Vec<Spec> {
    let mut v = vec![Spec::floor()];
    for x in [-6.0, 0.0, 6.0] {
        v.extend(tower(3, x, 0.0, 0.5));
    }
    v
}

/// The step by which the towers are held (they settle and freeze well before it).
const HELD_BY: usize = 150;
/// An arm's step count.
const ARM_STEPS: usize = 330;

/// The first step on which `ev` held a row, or a void.
fn first_held(label: &str, ev: &Evidence) -> usize {
    ev.stats
        .iter()
        .position(|s| s.held_rows > 0)
        .unwrap_or_else(|| panic!("{label}: void: nothing was ever held"))
}

/// Asserts the arm's scene was held before its first event at [`HELD_BY`]: every dynamic row
/// held on the step before it.
fn assert_held_before_events(label: &str, ev: &Evidence) {
    let first = first_held(label, ev);
    let k = HELD_BY - 1;
    assert!(
        ev.stats[k].held_rows as usize == ev.dynamic_rows[k],
        "{label}: void: {} of {} dynamic rows held on the step before the events (first held at {first})",
        ev.stats[k].held_rows,
        ev.dynamic_rows[k]
    );
}

/// The variants every arm runs on: the Tree with the brute path off at W = 1, the Grid with its
/// parallel emit at W = 8, AllPairs at W = 1 — `contact_reuse` on — and the Tree again with
/// `contact_reuse` off. The Grid's tree also has its brute path off: an arm that switches the
/// Grid to the Tree (the toggles arm, T6) then runs the tree path, whose sleeper set the switch
/// back must dissolve; no other arm runs the tree there.
fn arm_variants() -> [Variant; 4] {
    let tree = Variant { brute_max_rows: Some(0), ..Variant::cell(BroadphaseKind::Tree, true, 1) };
    [
        tree,
        Variant { brute_max_rows: Some(0), ..Variant::cell(BroadphaseKind::Grid, true, 8) },
        Variant::cell(BroadphaseKind::AllPairs, true, 1),
        Variant { reuse: Some(false), ..tree },
    ]
}

/// Runs an arm on every variant of [`arm_variants`], then `check`s each run's evidence.
fn arm(label: &str, specs: &[Spec], steps: usize, script: &mut Script<'_>, check: &dyn Fn(&str, &Evidence)) {
    for v in arm_variants() {
        let label = format!(
            "{label} [{:?} reuse {:?} W{}]",
            v.kind.expect("an arm names its kind"),
            v.reuse,
            v.workers
        );
        let ev = lockstep(&label, Pipeline::Default, v, specs, steps, script);
        if std::env::var_os("L10_ARM_TRACE").is_some() {
            for (k, st) in ev.stats.iter().enumerate() {
                if k % 10 == 0 || st.restored > 0 || st.moved_in > 0 || st.flushes > 0 {
                    println!("{label} step {k}: frozen {}/{} {st:?}", ev.frozen_rows[k], ev.dynamic_rows[k]);
                }
            }
        }
        println!("{label}: {} steps, {} with a held row; rules {:?}", ev.steps, ev.held_steps(), ev.rules);
        check(&label, &ev);
    }
}

/// D1 (R3): a held member's pose written by a user system — a teleport of 1 mm, then a velocity
/// write on another tower — restores its island; both re-freeze and move in again (M1).
#[test]
fn s3_member_teleport_and_velocity_write_restore() {
    arm(
        "S3 teleport/velocity",
        &towers(),
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY {
                rig.body_mut(3).position.x += 1.0e-3;
            }
            if step == HELD_BY + 40 {
                rig.body_mut(5).linear_velocity.z = 1.0e-4;
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.d1_member >= 2, "{label}: void: D1 restored {} times", ev.rules.d1_member);
            assert!(ev.sum(|s| s.moved_in) >= 5, "{label}: void: the islands did not move back in");
            // Design 06 §7's anti-vacuity: the solve searched the restore warm source after a
            // read-side miss, and found the restored records there.
            assert!(
                ev.rules.restore_rec_searches > 0 && ev.rules.restore_rec_hits > 0,
                "{label}: void: restore_rec searched {} times, hit {}",
                ev.rules.restore_rec_searches,
                ev.rules.restore_rec_hits
            );
        },
    );
}

/// D2: a ball rolled into a held tower restores it (the scan finds a stream pair from a held
/// row to a row that does not rest) (M7).
#[test]
fn s3_projectile_restores_by_the_d2_scan() {
    arm(
        "S3 projectile",
        &towers(),
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY {
                rig.spawn(&Spec::ball(Vec3::new(-3.0, 0.3, 0.0), 0.3).moving(Vec3::new(-6.0, 0.0, 0.0)));
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.d2_scan >= 1, "{label}: void: the D2 scan restored nothing");
        },
    );
}

/// D8: `sleep_threshold` lowered below the held islands' energy unlatches their rows at the next
/// `end_step`; the next broadphase restores them (M8).
#[test]
fn s3_sleep_threshold_change_restores_by_d8() {
    arm(
        "S3 sleep_threshold",
        &towers(),
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY {
                rig.cfg().sleep_threshold = -1.0;
            }
            if step == HELD_BY + 5 {
                rig.cfg().sleep_threshold = PhysicsConfig::default().sleep_threshold;
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.d8_latch >= 1, "{label}: void: D8 restored nothing");
        },
    );
}

/// D6: `wake_all` flushes every held island.
#[test]
fn s3_wake_all_flushes() {
    arm(
        "S3 wake_all",
        &towers(),
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY {
                rig.world.resource_mut::<IslandSleep>().wake_all();
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.d6_wake_all >= 1, "{label}: void: no D6 flush");
        },
    );
}

/// The mode transitions (design 04 D9, 08 D-H): sleeping on → off → on, then the `Sets` world's
/// mode `Sets → Off → Sets`, each while islands are held (M19, N30, W4-M); then a kind switch
/// away from the tree and back (T6, M-T6).
///
/// On the Tree the sleeping toggle is also the tree design's toggle-off script (its ruling W2, as
/// L10's T1 recasts it): the step that turns sleeping off dissolves the sleeper set — the hint is
/// off without the sleep-skip — and while sleeping stays off no row is a candidate, no set is
/// rebuilt and no member is evicted (the static set keeps its floor). A hint read regardless of
/// the step's mode would see the stale classification and evict the floor. The kind switch
/// dissolves the sleeper set and empties the withheld list on the first step of the other kind.
#[test]
fn s3_sleeping_and_mode_toggles() {
    arm(
        "S3 toggles",
        &towers(),
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY {
                rig.cfg().sleeping = false;
            }
            if step == HELD_BY + 3 {
                rig.cfg().sleeping = true;
            }
            if step == HELD_BY + 120 {
                rig.cfg().sleep_skip = SleepSkip::Off;
            }
            if step == HELD_BY + 123 {
                let mode = rig.mode;
                rig.cfg().sleep_skip = mode;
            }
            // A kind switch and back while held (design 04 D9's last row: the held set is
            // unchanged, the narrowphase skip keys off `HELD`, not off the broadphase).
            if step == HELD_BY + 150 || step == HELD_BY + 160 {
                let cfg = rig.cfg();
                cfg.broadphase = match cfg.broadphase {
                    BroadphaseKind::Grid => BroadphaseKind::Tree,
                    _ => BroadphaseKind::Grid,
                };
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.dh_mode >= 2, "{label}: void: {} D-H flushes", ev.rules.dh_mode);
            let switch_away = if label.contains("[Tree") {
                // The toggle-off script: sleeping off on steps HELD_BY .. HELD_BY + 3.
                assert!(ev.sleepers[HELD_BY - 1] > 0, "{label}: void: no sleeper before the toggle");
                assert!(
                    ev.sleepers[HELD_BY] == 0 && ev.withheld[HELD_BY] == 0,
                    "{label}: the toggle-off step keeps {} sleepers, {} withheld pairs",
                    ev.sleepers[HELD_BY],
                    ev.withheld[HELD_BY]
                );
                let (a, b) = (ev.tree[HELD_BY], ev.tree[HELD_BY + 2]);
                assert_eq!(
                    (b.hint_candidates - a.hint_candidates, b.sleeper_rebuilds - a.sleeper_rebuilds, b.evictions - a.evictions),
                    (0, 0, 0),
                    "{label}: with sleeping off: Δcandidates, Δsleeper rebuilds, Δevictions"
                );
                assert!(ev.sleepers[HELD_BY + 1..HELD_BY + 3].iter().all(|&z| z == 0), "{label}: a sleeper while sleeping is off");
                // The static set keeps its members across the toggle (|S| = members − |Z|).
                let statics = |k: usize| ev.tree[k].members - ev.sleepers[k];
                assert!(
                    (HELD_BY..HELD_BY + 3).all(|k| statics(k) == statics(HELD_BY - 1)),
                    "{label}: the static set changed across the toggle: {:?}",
                    (HELD_BY - 1..HELD_BY + 3).map(statics).collect::<Vec<_>>()
                );
                Some(HELD_BY + 150)
            } else if label.contains("[Grid") {
                Some(HELD_BY + 160)
            } else {
                None
            };
            // T6: the first step of the other kind dissolves the sleeper set the tree held.
            if let Some(k) = switch_away {
                assert!(ev.sleepers[k - 1] > 0, "{label}: void: no sleeper before the kind switch at step {k}");
                assert!(
                    ev.sleepers[k] == 0 && ev.withheld[k] == 0,
                    "{label}: the kind switch at step {k} keeps {} sleepers, {} withheld pairs",
                    ev.sleepers[k],
                    ev.withheld[k]
                );
            }
        },
    );
}

/// Review W1 of L10 C3c (design 04 D9: every stage reads the mode the broadphase recorded, never
/// the configuration): `sleeping` and the mode written by a system between the broadphase and the
/// narrowphase ([`mid_early`]). Sleeping on → off on a step whose broadphase held the towers: the
/// narrowphase has skipped their pairs, so a solve that picked its arm from the configuration
/// would integrate the held rows at their raw inverse mass with no contacts, where `Off` keeps the
/// towers' contacts. Then off → on, and the mode `Sets → Off → Sets`, the same way. Each write
/// takes effect on the next step, whose broadphase flushes (D-H).
#[test]
fn s3_mid_step_config_writes_take_effect_on_the_next_step() {
    arm(
        "S3 mid-step writes",
        &towers(),
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            // The world's own mode, restored by a write that captures nothing.
            let restore: fn(&mut PhysicsConfig) = match rig.mode {
                SleepSkip::Off => |c| c.sleep_skip = SleepSkip::Off,
                SleepSkip::Sets => |c| c.sleep_skip = SleepSkip::Sets,
            };
            let mid = rig.mid(Place::Early);
            match step {
                s if s == HELD_BY => mid.cfg = Some(|c| c.sleeping = false),
                s if s == HELD_BY + 3 => mid.cfg = Some(|c| c.sleeping = true),
                s if s == HELD_BY + 120 => mid.cfg = Some(|c| c.sleep_skip = SleepSkip::Off),
                s if s == HELD_BY + 123 => mid.cfg = Some(restore),
                _ => {}
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            // Each "off" write landed mid-step: its step's broadphase held every dynamic row, and
            // the next step's broadphase flushed them.
            for k in [HELD_BY, HELD_BY + 120] {
                assert!(
                    ev.stats[k].held_rows as usize == ev.dynamic_rows[k] && ev.stats[k + 1].flushes == 1,
                    "{label}: void: the write at step {k} did not land mid-step on a held step: {:?} then {:?}",
                    ev.stats[k],
                    ev.stats[k + 1]
                );
            }
            assert!(ev.rules.dh_mode >= 2, "{label}: void: {} D-H flushes", ev.rules.dh_mode);
        },
    );
}

/// Design 04 T2 (M15; ruling W1 of rev 2.1's Invariant V): a static box standing beside a held
/// ball, the two bounding spheres overlapping but the shapes 0.1 apart — a sphere–box pair with
/// no kept state, so the box anchors nothing and D1a cannot see it — is turned 45° in place:
/// position and radius unchanged, so the tree's bit compare keeps it, and only T2's `RESTING`
/// term takes it out of the static set. Its pair with the ball then returns to the stream, where
/// the D2 scan restores the ball (its partner does not rest), and the turned corner now reaches
/// the ball, as under `Off`. With the term dropped, the pair stays withheld and the ball never
/// learns of the corner.
#[test]
fn s3_static_turned_in_place_beside_a_held_ball() {
    let mut specs = towers();
    specs.push(Spec::ball(Vec3::new(12.0, 0.3, 0.0), 0.3));
    let slab = specs.len();
    // Half-extent 0.4 at x = 12.8: its face at 12.4, 0.1 clear of the ball (12.3); turned 45°
    // about +Y its edge reaches 12.8 − 0.4·√2 ≈ 12.234, inside the ball.
    specs.push(Spec::wall(Vec3::new(12.8, 0.4, 0.0), Vec3::new(0.4, 0.4, 0.4)));
    arm(
        "S3 static turned in place",
        &specs,
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY {
                rig.body_mut(slab).rotation = yaw(std::f32::consts::FRAC_PI_4);
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.d2_scan >= 1, "{label}: void: the turned slab restored nothing");
            if label.contains("[Tree") {
                assert!(ev.withheld[HELD_BY - 1] > 0, "{label}: void: nothing withheld before the turn");
            }
        },
    );
}

/// A Rows step while held: a body spawned into the archetype ahead of the pile moves every pile
/// row by one (the mirror re-keys the moved held pairs, design 06 D-C), and — on a later gather
/// — a member turned by a user write restores its island on a Rows step, whose restore keys must
/// be the previous step's rows (M20′, N17).
#[test]
fn s3_rows_step_restore() {
    arm(
        "S3 Rows restore",
        &towers(),
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY {
                rig.spawn(&Spec::wall(Vec3::new(30.0, 5.0, 30.0), Vec3::new(0.5, 0.5, 0.5)).at(Arch::Front));
            }
            if step == HELD_BY + 30 {
                rig.spawn(&Spec::wall(Vec3::new(-30.0, 5.0, 30.0), Vec3::new(0.5, 0.5, 0.5)).at(Arch::Front));
                rig.body_mut(3).rotation = yaw(1.0e-3);
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.d1_member >= 1, "{label}: void: the Rows-step turn restored nothing");
            assert!(ev.rules.mirrored > 0, "{label}: void: no held key was mirrored on a Rows step");
        },
    );
}

/// Arm 1 and the jumper arms (design 06 arm 1; ruling W2): a swap-remove despawn of the body
/// ahead of the towers moves the last tower's top cube to its row — a jumper whose row now
/// orders before every other member's, so its pairs flip. Its island restores (D1), and its
/// restored pairs are found through L9's join with the jumper search (W2-M, N3′, N4, M17).
#[test]
fn s3_jumper_flip_restore() {
    let mut specs = vec![Spec::wall(Vec3::new(30.0, 5.0, -30.0), Vec3::new(0.5, 0.5, 0.5))];
    specs.extend(towers());
    arm(
        "S3 jumper",
        &specs,
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY {
                rig.despawn(0);
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.d1_member >= 1, "{label}: void: the jumper restored nothing");
        },
    );
}

/// Arm 11: a held member is despawned (its row vanishes; the restore sources skip the entries
/// that name it) (N31).
#[test]
fn s3_held_member_despawned() {
    arm(
        "S3 member despawn",
        &towers(),
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY {
                rig.despawn(2);
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.d1_member >= 1, "{label}: void: the despawn restored nothing");
        },
    );
}

/// A small rest pyramid (`height` layers, exactly touching, pitch 2) at `(ox, oz)`: side-by-side
/// neighbours touch (contact pairs, and with `contact_reuse` on, reuse records), diagonal ones
/// are candidates the SAT separates (SEP pairs).
fn small_pyramid(height: i32, ox: f32, oz: f32) -> Vec<Spec> {
    pyramid(height, 0.0, ox, oz)
}

/// Design 06 D-C and ruling W3 of rev 2 (M10, N9, N10): the hysteresis table cleared for its
/// load on a Rows step while a pile with held reuse-record pairs and held separated pairs is
/// held. Fifteen static bodies spawned into the archetype ahead of the pile on every step move
/// every pile row, so every held box key is re-keyed each step (the mirror's moved pairs, as `Off`'s
/// computes re-key them) and the table's live occupancy climbs past half its slots, which clears
/// it on a Rows step; the mirror re-keys every held pair that re-keys after that clear, and no
/// separated pair.
#[test]
fn s3_load_clear_on_a_rows_step_mirrors_every_held_key() {
    let mut specs = vec![Spec::floor()];
    specs.extend(small_pyramid(3, 0.0, 0.0));
    arm(
        "S3 grow-clear",
        &specs,
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            // Fifteen rows ahead per step, more than the pile's row span: every re-keyed held
            // pair lands on a key no pair held one step ago, so the occupancy climbs.
            if (HELD_BY..HELD_BY + 40).contains(&step) {
                for i in 0..15 {
                    let x = 30.0 + 2.0 * i as f32;
                    let z = 30.0 + 2.0 * (step - HELD_BY) as f32;
                    rig.spawn(&Spec::wall(Vec3::new(x, 5.0, z), Vec3::new(0.5, 0.5, 0.5)).at(Arch::Front));
                }
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.mirror_clears >= 1, "{label}: void: no mirror pass on a cleared table");
            assert!(ev.rules.mirrored > 0, "{label}: void: no key mirrored");
            let k = HELD_BY - 1;
            assert!(ev.held_sep[k] > 0, "{label}: void: no separated pair held");
            if label.contains("reuse Some(true)") {
                assert!(ev.held_rec[k] > 0, "{label}: void: no reuse-record pair held");
                // Design 06 §7's anti-vacuity: a held REC pair that emitted no manifold (a slow
                // pair separated inside τ keeps its record without a contact).
                assert!(ev.rules.rec_without_manifold > 0, "{label}: void: no REC-without-manifold pair held");
            }
        },
    );
}

/// Design 04 D6, review O7: the logical manifold view interleaves the stream and the held store
/// by ordinal — a ball rolling on the floor between two held towers in row order puts its stream
/// manifold between their kept ones.
#[test]
fn s3_mixed_view_interleaves_stream_and_kept() {
    let mut specs = vec![Spec::floor()];
    specs.extend(tower(3, -6.0, 0.0, 0.5));
    specs.push(Spec::ball(Vec3::new(0.0, 0.3, -20.0), 0.3).moving(Vec3::new(0.0, 0.0, 4.0)));
    specs.extend(tower(3, 6.0, 0.0, 0.5));
    arm(
        "S3 mixed view",
        &specs,
        ARM_STEPS,
        &mut quiet,
        &|label, ev| {
            let k = HELD_BY - 1;
            assert_eq!(
                ev.stats[k].held_rows, 6,
                "{label}: void: both towers must be held while the ball rolls"
            );
            assert!(ev.frozen_rows[k] < ev.dynamic_rows[k], "{label}: void: the ball froze");
        },
    );
}

/// Arm 5 (design 06 §7, N16): enough tombstoned records that compaction fires. Seventy cubes in
/// a line, a separated pair between neighbours, each its own island, settle at staggered steps;
/// `wake_all` restores them all; they re-freeze and move in again past the compaction.
#[test]
fn s3_compaction_of_dead_records() {
    let mut specs = vec![Spec::floor()];
    for i in 0..70 {
        let lift = 0.002 * (i % 7) as f32;
        specs.push(Spec::cube(Vec3::new(-40.0 + 1.1 * i as f32, 0.5 + lift, 5.0), 0.5));
    }
    arm(
        "S3 compaction",
        &specs,
        ARM_STEPS + 120,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY + 60 {
                rig.world.resource_mut::<IslandSleep>().wake_all();
            }
        },
        &|label, ev| {
            assert!(ev.stats[HELD_BY + 59].held_islands >= 64, "{label}: void: fewer than 64 records held");
            assert!(ev.rules.compactions >= 1, "{label}: void: no compaction");
            assert!(ev.rules.cross_copies > 0, "{label}: void: no cross pair was copied");
        },
    );
}

/// Arm 3 (design 06 B3, Invariant K; N6, N18, N28): tower X of four cubes and tower Y of two,
/// 1.1 apart — separated cross pairs between them, and with reuse on the reuse records the
/// touching cubes keep. Both move in together; then X's top cube (no cross pair) is nudged, so X
/// alone restores and moves back in while Y is held, copying the cross pairs from Y's record;
/// then a ball knocks Y, which restores alone (the D2 scan) and wakes, so X restores next (its
/// anchors move) and computes the cross pairs from the copy; finally both restore on one step.
#[test]
fn s3_cross_pairs_restored_x_then_y_then_both() {
    let mut specs = vec![Spec::floor()];
    specs.extend(tower(4, 0.0, 0.0, 0.5));
    specs.extend(tower(2, 1.1, 0.0, 0.5));
    arm(
        "S3 cross pairs",
        &specs,
        ARM_STEPS + 180,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY {
                rig.body_mut(4).position.x += 1.0e-4;
            }
            if step == HELD_BY + 100 {
                rig.spawn(&Spec::ball(Vec3::new(4.0, 0.3, 0.0), 0.3).moving(Vec3::new(-3.0, 0.0, 0.0)));
            }
            if step == HELD_BY + 330 {
                rig.body_mut(1).position.x -= 1.0e-4;
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.cross_copies > 0, "{label}: void: no cross pair was copied");
            assert!(ev.rules.restore_idx_hits > 0, "{label}: void: no restored pair found its kept state");
            assert!(ev.rules.d2_scan >= 1, "{label}: void: the ball restored nothing");
            assert!(ev.rules.d1a_anchor >= 1, "{label}: void: no anchor restored its island");
        },
    );
}

/// Review W2 of L10 C3c (design 06 B3, Invariant K): a move-in copies its cross pairs from a held
/// record restored on the same step, which it reads tombstoned, its row table translated out of
/// order. Tower X (four cubes) stands beside island Y — two cubes on the floor, `y0` 0.1 from X
/// and `yr` beyond it, bridged by `y1` resting across both — so `y0` and `y1` share separated
/// cross pairs with X's lower cubes, and `yr` pairs with nothing of X. Both move in; X's top cube
/// (no cross pair) is nudged, so X alone restores and, its latch still asleep, is a candidate on
/// the next step, whose despawn restores Y (D1):
///
/// * vanished: `yr`, whose row lies between `y0`'s and `y1`'s, is despawned, and an unrelated far
///   cube — the archetype's last row — takes its row: Y's members read `[y0, NONE, y1]`;
/// * jumper: an unrelated far cube spawned before Y is despawned, and `yr` — the last row — takes
///   its row: Y's members read `[y0, y1, jumper]`.
///
/// Either way the pre-W2 lookup, a binary search of Y's members, misses `y1`, which is present and
/// rests: X would move in without the separated pair on `y1` that `Off` still carries, so its held
/// slot's kept tag would differ from `Off`'s, and a later restore would compute the pair with no
/// state. The two layouts do not miss for the same reason:
///
/// * vanished: any binary search misses, one that returns on its first equal probe included: its
///   middle probe reads `NONE` (`u32::MAX`), which sends it to the half before `y1`;
/// * jumper: only std's `slice::binary_search` as it stands (rustc 1.98.1) is known to miss. It
///   runs a fixed number of halvings with no early exit on a match: it probes `y1` at index 1,
///   keeps halving, and ends on the jumper's row. A search that returns on its first equal probe
///   finds `y1` there. So if std ever returns to an early exit, this layout alone stops guarding
///   W2, and the vanished layout still does.
#[test]
fn s3_move_in_copies_from_a_record_restored_on_its_step() {
    let x_tower = tower(4, 0.0, 0.0, 0.5);
    // X spans x <= 0.5; y0 spans [0.6, 1.6] and yr [1.6, 2.6]. y1 spans [1.1, 2.1] one level up,
    // resting on both, so the three are one island through its two contacts. yr's centre is 2.1
    // from X's axis, past two bounding spheres' reach (2 x 0.866): no pair with X. y1's is 1.6
    // from X's second cube (a pair, 0.6 apart) and 1.89 from the others (none, the top cube
    // included).
    let y0 = Spec::cube(Vec3::new(1.1, 0.5, 0.0), 0.5);
    let yr = Spec::cube(Vec3::new(2.1, 0.5, 0.0), 0.5);
    let y1 = Spec::cube(Vec3::new(1.6, 1.5, 0.0), 0.5);
    let far = Spec::cube(Vec3::new(20.0, 0.5, 0.0), 0.5);
    // Body indices: the floor 0, X 1..=4 (its top cube 4), then each scene's four.
    let scenes: [(&str, [Spec; 4], usize); 2] = [
        ("vanished", [y0, yr, y1, far], 6),
        ("jumper", [far, y0, y1, yr], 5),
    ];
    for (name, tail, despawned) in scenes {
        let mut specs = vec![Spec::floor()];
        specs.extend(x_tower.iter().copied());
        specs.extend(tail);
        arm(
            &format!("S3 move-in from a restored record ({name})"),
            &specs,
            ARM_STEPS,
            &mut |step, rig: &mut Rig| {
                if step == HELD_BY {
                    rig.body_mut(4).position.x -= 1.0e-4;
                }
                if step == HELD_BY + 1 {
                    rig.despawn(despawned);
                }
            },
            &|label, ev| {
                assert_held_before_events(label, ev);
                // X, Y and the far cube: Y is ONE record, so its member run is the one the
                // despawn unsorts.
                let before = ev.stats[HELD_BY - 1];
                assert_eq!(before.held_islands, 3, "{label}: void: Y is not one held island: {before:?}");
                let k = HELD_BY + 1;
                assert!(
                    ev.stats[k].moved_in >= 1 && ev.stats[k].restored >= 1,
                    "{label}: void: step {k} did not both restore Y and move X in: {:?}",
                    ev.stats[k]
                );
                assert!(
                    ev.rules.cross_reads_dead > 0 && ev.rules.cross_copies > 0,
                    "{label}: void: no cross pair was looked up in a record restored on its step \
                     ({} lookups, {} copies)",
                    ev.rules.cross_reads_dead,
                    ev.rules.cross_copies
                );
            },
        );
    }
}

/// Arms 4 and 4b (design 06 D-E, 08 D-E′; N15, N23): the epoch's config inputs changed while
/// held — `contact_reuse` toggled, τ changed, then the rate halved (`dt`) — each flushes.
#[test]
fn s3_epoch_changes_flush() {
    arm(
        "S3 epoch",
        &towers(),
        ARM_STEPS + 60,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY {
                let reuse = rig.cfg().contact_reuse;
                rig.cfg().contact_reuse = !reuse;
            }
            if step == HELD_BY + 60 {
                rig.cfg().contact_reuse_distance *= 0.5;
            }
            if step == HELD_BY + 120 {
                rig.world.resource_mut::<FixedTime>().set_timestep(Duration::from_secs_f32(2.0 * DT));
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.d5_epoch >= 3, "{label}: void: {} epoch flushes", ev.rules.d5_epoch);
        },
    );
}

/// Arm 2 (design 06 B4, L9 ruling W2; N5): with contact reuse on, the support S of a held box
/// pile is lowered by half of τ_eff; the restore computes the pile's pairs from the kept reuse
/// records (hits), and U wakes exactly as under `Off`.
#[test]
fn s3_support_lowered_below_tau_eff_restores_from_the_kept_records() {
    let mut specs = vec![Spec::floor()];
    specs.extend(tower(2, 0.0, 0.0, 0.5));
    for v in arm_variants() {
        let v = Variant { reuse: Some(true), ..v };
        let label = format!("S3 support below tau [{:?} W{}]", v.kind, v.workers);
        let mut script = |step: usize, rig: &mut Rig| {
            if step == 0 {
                rig.cfg().contact_reuse_distance = 0.002;
            }
            if step == HELD_BY {
                rig.body_mut(1).position.y -= 0.001;
            }
        };
        let ev = lockstep(&label, Pipeline::Default, v, &specs, ARM_STEPS, &mut script);
        assert_held_before_events(&label, &ev);
        assert!(ev.rules.restore_idx_hits > 0, "{label}: void: no restored pair found its kept state");
        assert!(ev.held_rec[HELD_BY - 1] > 0, "{label}: void: no reuse-record pair held");
    }
}

/// Arm 8 (design 08 D-G; N21, N22): a static sensor over tower 0 and a kinematic sensor drifting
/// at 1 mm/s over tower 1, both overlapping their towers, and a small static sensor 5 cm beside
/// tower 0's bottom cube — a separated sensor pair, whose carried separating axis still separates
/// it every step. A sensor pair is never kept and its partner never anchors: the drifting trigger
/// neither refuses tower 1's move-in nor restores it (at most one move-in of the towers before
/// the ball). A ball then restores tower 0 while the sensors touch it, and the separated sensor
/// pair of the restored cube is still collided from its carry, as under `Off` (N22).
///
/// The drift is written by the script, one step's worth each step: a kinematic body's pose is
/// gameplay-owned, and the physics does not integrate its `linear_velocity`.
#[test]
fn s3_sensor_triggers_over_a_held_tower() {
    let mut specs = towers();
    let trigger = Spec::wall(Vec3::new(-6.0, 1.5, 0.0), Vec3::new(1.2, 1.2, 1.2)).at(Arch::Sensor);
    specs.push(trigger);
    let mut moving = Spec::wall(Vec3::new(0.0, 1.5, 0.0), Vec3::new(1.2, 1.2, 1.2)).at(Arch::Sensor);
    moving.kinematic = true;
    let drifting = specs.len();
    specs.push(moving);
    specs.push(Spec::wall(Vec3::new(-6.0, 0.5, -0.8), Vec3::new(0.25, 0.25, 0.25)).at(Arch::Sensor));
    arm(
        "S3 sensor triggers",
        &specs,
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            rig.body_mut(drifting).position.z = 1.0e-3 * DT * (step + 1) as f32;
            if step == HELD_BY + 60 {
                rig.spawn(&Spec::ball(Vec3::new(-3.0, 0.3, 0.0), 0.3).moving(Vec3::new(-6.0, 0.0, 0.0)));
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            let moved: u64 = ev.stats[..HELD_BY + 60].iter().map(|s| u64::from(s.moved_in)).sum();
            assert!(moved <= 3, "{label}: the triggers churned the towers: {moved} move-ins before the ball");
            assert!(ev.rules.d2_scan >= 1, "{label}: void: the ball restored nothing");
        },
    );
}

/// Arm 9 (ruling W3, design 08 B5′): reuse records that miss on every step and are settled —
/// the admission's `SETTLED` term — with the pair at rest. Gravity off; cube A at rest, cube B
/// above it turned about a general axis, its lowest corner one ulp into A's top face (the
/// shallowest contact the float grid holds, found ulp by ulp; the pair then separates at
/// ~5e-11 m/s, which is not exactly zero — no contact exists at a zero separation — but far
/// below the fast bound of τ_eff), τ = 1e-9 m (τ = 0 records nothing on this tree: a pair
/// takes the reuse path only with τ_eff > 0). The relative rotation's residual in `conj(q) ⊗ q`
/// is not zero for a general axis, so the criterion fails at bit-equal poses: a miss every step,
/// settled from the second. A count of zero such admissions is a void.
#[test]
fn s3_settled_reuse_misses_are_admitted() {
    let axis = Vec3::new(1.0, 2.0, 3.0);
    let axis = axis * (1.0 / axis.length());
    let (sn, cs) = (0.5f32 * 0.7).sin_cos();
    let turn = Quat::new(axis.x * sn, axis.y * sn, axis.z * sn, cs);
    let lowest = |t: Quat| {
        // The cube's lowest corner below its centre: Σ |R e_i · y| · h.
        let r = Mat3::from_quat(t);
        0.5 * (r.rows[1].x.abs() + r.rows[1].y.abs() + r.rows[1].z.abs())
    };
    let specs_at = |y: f32| {
        vec![
            Spec::cube(Vec3::new(0.0, 0.0, 0.0), 0.5),
            Spec::cube(Vec3::new(0.0, y, 0.0), 0.5).turned(turn),
        ]
    };
    let y0 = 0.5 + lowest(turn);
    let setup = |rig: &mut Rig| {
        rig.cfg().gravity = Vec3::ZERO;
        rig.cfg().contact_reuse = true;
        rig.cfg().contact_reuse_distance = 1.0e-9;
    };
    let touching = |y: f32| {
        let mut rig = Rig::new(Pipeline::Default, Variant::default_cfg(1), SleepSkip::Off, &specs_at(y));
        setup(&mut rig);
        (0..3).all(|_| {
            rig.step();
            !rig.world.resource::<Manifolds>().solver_manifolds().is_empty()
        })
    };
    // The shallowest contact the float grid allows: the highest height, ulp by ulp from above
    // the analytic touch, at which the corner still penetrates (a zero separation is not
    // reachable — contact is penetration, and the probe recorded in the lane's C3b notes shows
    // one ulp of it pushes the pair apart at ~5e-11 m/s, far below τ_eff's fast bound).
    let y = (-64..=8)
        .rev()
        .map(|k: i32| f32::from_bits((y0.to_bits() as i32 + k) as u32))
        .find(|&y| touching(y))
        .expect("construction: some height within 64 ulps below the touch is a contact");
    for v in arm_variants() {
        let v = Variant { reuse: Some(true), ..v };
        let label = format!("S3 arm 9 [{:?} W{}]", v.kind, v.workers);
        let mut script = |step: usize, rig: &mut Rig| {
            if step == 0 {
                setup(rig);
            }
            if step == HELD_BY {
                rig.body_mut(1).position.y += 1.0e-3;
            }
        };
        let ev = lockstep(&label, Pipeline::Default, v, &specs_at(y), ARM_STEPS, &mut script);
        println!("{label}: contact height {y}; rules {:?}", ev.rules);
        assert!(first_held(&label, &ev) < HELD_BY, "{label}: void: not held before the event");
        assert!(
            ev.rules.rec_settled_misses > 0,
            "{label}: void: no reuse record was admitted on a settled miss (HIT = 0, SETTLED = 1)"
        );
    }
}

/// Arm 7 (design 08 D-F; N19): a static box sensor beside a held cube whose frame it reads. The
/// cubes stand apart on the floor (one island each), yawed 0° and 45° in turn, so a row's last
/// filled frame is its neighbour's yaw. The sensor sits where a 45° cube's corner reaches and a
/// 0° cube's face does not. A body spawned ahead of the pile moves every row by one: the held
/// cube next to the sensor lands on a row whose frame slot was last filled for a 45° cube, and
/// only the fill of `SENSOR_NBR` rows keeps its own frame there.
#[test]
fn s3_sensor_reads_a_held_rows_own_frame_after_a_rows_step() {
    let mut specs = vec![Spec::floor()];
    for i in 0..8 {
        let turn = if i % 2 == 0 { Quat::IDENTITY } else { yaw(std::f32::consts::FRAC_PI_4) };
        specs.push(Spec::cube(Vec3::new(3.0 * i as f32, 0.5, 0.0), 0.5).turned(turn));
    }
    // Beside cube 2 (0°, x = 6): x ∈ [6.6, 7.0], clear of its face at 6.5, inside a 45° corner's
    // reach to 6.707.
    specs.push(Spec::wall(Vec3::new(6.8, 0.5, 0.0), Vec3::new(0.2, 0.2, 0.2)).at(Arch::Sensor));
    arm(
        "S3 sensor frame",
        &specs,
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY || step == HELD_BY + 40 {
                rig.spawn(&Spec::wall(Vec3::new(-30.0, 5.0, 30.0 + step as f32), Vec3::new(0.5, 0.5, 0.5)).at(Arch::Front));
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.stats[HELD_BY + 45].held_rows >= 8, "{label}: void: the cubes were not held across the Rows steps");
        },
    );
}

/// Arm 10 (design 08 §7; N27): at W = 8 with the parallel narrowphase, a despawn shifts the
/// stream so held pairs land on slots whose previous occupants — pairs of cubes sliding on ice
/// ahead of the pile in row order, awake and computed every step — committed a real axis. A held
/// slot's commit must be written (`AXIS_NONE`), or the commit pass re-keys its pair with a stale
/// axis. The two despawns are on consecutive steps: the first is a Rows step, whose pre-read
/// frame has every computed pair commit its axis whatever its hint, so the second step's held
/// slots inherit real axes (on an Identity step a settled pair commits `AXIS_NONE`, and a stale
/// `AXIS_NONE` is harmless).
#[test]
fn s3_despawn_shifts_the_stream_under_held_slots() {
    let mut specs = vec![Spec::floor()];
    let mut ice = Spec::wall(Vec3::new(60.0, -1.0, 0.0), Vec3::new(20.0, 1.0, 60.0)).at(Arch::Front);
    ice.friction = 0.0;
    specs.push(ice);
    for i in 0..30 {
        for j in 0..2 {
            let mut s = Spec::cube(Vec3::new(45.0 + 1.0 * j as f32, 0.5, -40.0 + 2.5 * i as f32), 0.5)
                .moving(Vec3::new(0.0, 0.0, 1.0))
                .at(Arch::Front);
            s.friction = 0.0;
            specs.push(s);
        }
    }
    specs.extend(small_pyramid(5, 0.0, 0.0));
    let first_slider = 2;
    let mut variants = vec![Variant::cell(BroadphaseKind::Grid, true, 8), Variant::cell(BroadphaseKind::Grid, false, 8)];
    variants.push(Variant::cell(BroadphaseKind::AllPairs, true, 8));
    for v in variants {
        let label = format!("S3 arm 10 [{:?} reuse {:?} W{}]", v.kind, v.reuse, v.workers);
        let mut script = |step: usize, rig: &mut Rig| {
            // The sliders' velocity is written every step (a user system steering them), so they
            // never rest and are never held: their pairs are collided, and commit a real axis,
            // on every step in both modes.
            let despawned = if step > HELD_BY + 1 { 2 } else { usize::from(step > HELD_BY) };
            for k in first_slider + despawned..first_slider + 60 {
                rig.body_mut(k).linear_velocity.z = 1.0 + 1.0e-4 * (step % 2) as f32;
            }
            if step == HELD_BY || step == HELD_BY + 1 {
                rig.despawn(first_slider + step - HELD_BY);
            }
        };
        let ev = lockstep(&label, Pipeline::Default, v, &specs, ARM_STEPS, &mut script);
        println!("{label}: rules {:?}", ev.rules);
        if std::env::var_os("L10_ARM_TRACE").is_some() {
            for (k, st) in ev.stats.iter().enumerate().step_by(10) {
                println!("{label} step {k}: frozen {}/{} {st:?}", ev.frozen_rows[k], ev.dynamic_rows[k]);
            }
        }
        let k = HELD_BY - 1;
        assert_eq!(ev.stats[k].held_rows, 55, "{label}: void: the pyramid alone must be held before the despawns");
        assert!(ev.stats[HELD_BY + 1].held_skipped_pairs > 0, "{label}: void: no held skip after the despawn");
        assert!(
            ev.stale_slots[HELD_BY + 1] > 0,
            "{label}: void: no held slot inherited a computed pair's real axis on the second despawn"
        );
    }
}

/// S4 (design 04 D10; M18): a tower on an SDF floor, held; the field replaced wholesale with the
/// floor 1 mm lower (the edit bits change; `gen` is no witness), which flushes; later the box
/// kernel toggled, which flushes again. On the default configuration and on the Tree with its
/// brute path off, where the SDF pipeline's broadphase keeps the sleeper set (L10 C3c): the
/// tower's two box–box pairs are withheld while it is held, and each flush dissolves the set.
#[test]
fn s4_sdf_pile_field_edit_and_kernel_toggle() {
    let specs = tower(3, 0.0, 0.0, 0.5);
    let tree = |w| Variant { brute_max_rows: Some(0), ..Variant::cell(BroadphaseKind::Tree, true, w) };
    for (kind, variant) in [1, 8].into_iter().flat_map(|w| [("default", Variant::default_cfg(w)), ("Tree", tree(w))]) {
        let w = variant.workers;
        let label = format!("S4 SDF [{kind}] W{w}");
        let mut script = |step: usize, rig: &mut Rig| {
            if step == HELD_BY {
                *rig.world.resource_mut::<SdfField>() = sdf_floor(-1.0e-3);
            }
            if step == HELD_BY + 100 {
                let k = rig.cfg().sdf_narrowphase;
                rig.cfg().sdf_narrowphase = match k {
                    SdfNarrowphaseKernel::Scalar => SdfNarrowphaseKernel::Avx2,
                    SdfNarrowphaseKernel::Avx2 => SdfNarrowphaseKernel::Scalar,
                };
            }
        };
        let ev = lockstep(&label, Pipeline::Sdf, variant, &specs, ARM_STEPS, &mut script);
        println!("{label}: rules {:?}", ev.rules);
        if std::env::var_os("L10_ARM_TRACE").is_some() {
            for (k, st) in ev.stats.iter().enumerate().step_by(10) {
                println!("{label} step {k}: frozen {}/{} {st:?}", ev.frozen_rows[k], ev.dynamic_rows[k]);
            }
        }
        assert!(ev.stats[HELD_BY - 1].held_rows == 3, "{label}: void: the tower was not held before the edit");
        assert!(ev.rules.d5_epoch >= 2, "{label}: void: {} epoch flushes", ev.rules.d5_epoch);
        if kind == "Tree" {
            // `workers = 1`: the empty-stream dispatch check needs two chunks' worth of logical
            // pairs, which a three-box tower never has.
            assert_tree_withholds(&label, &ev, 1);
            assert!(
                ev.sleepers[HELD_BY - 1] == 3 && ev.withheld[HELD_BY - 1] > 0,
                "{label}: void: before the edit the sleeper set holds {} rows, {} pairs withheld",
                ev.sleepers[HELD_BY - 1],
                ev.withheld[HELD_BY - 1]
            );
            assert!(
                ev.sleepers[HELD_BY] == 0 && ev.withheld[HELD_BY] == 0,
                "{label}: the edit's flush keeps {} sleepers, {} withheld pairs",
                ev.sleepers[HELD_BY],
                ev.withheld[HELD_BY]
            );
        }
    }
}

/// S5 (design 04 "Scenes"): a coupled soft cube dropped on a held tower (the Grid forced by the
/// coupling); its reaction lands on the tower's top cube, which restores the island (D1).
#[test]
fn s5_soft_body_reaction_restores_a_held_tower() {
    let specs = {
        let mut v = vec![Spec::floor()];
        v.extend(tower(3, 0.0, 0.0, 0.5));
        v
    };
    for w in [1, 8] {
        let label = format!("S5 soft W{w}");
        let mut script = |step: usize, rig: &mut Rig| {
            if step == 0 {
                *rig.world.resource_mut::<SdfField>() = sdf_floor(-5.0);
                rig.cfg().soft_body = true;
            }
            if step == HELD_BY {
                let half = 0.3f32;
                let mut positions = Vec::new();
                for i in 0..8u32 {
                    let s = |bit: u32| if i & bit != 0 { half } else { -half };
                    positions.push([s(1), 3.6 + s(2), s(4)]);
                }
                let mut edges = Vec::new();
                for a in 0..8u32 {
                    for b in (a + 1)..8u32 {
                        edges.push((a, b));
                    }
                }
                let body = SoftBody::from_mesh(&positions, &[1.0; 8], &edges, None, 1.0e-6, 0.05)
                    .expect("construction: a braced cube is a soft body");
                let arch = rig.world.create_archetype(&[SoftBody::component_id()]);
                rig.world.spawn_one(arch, body).expect("construction: the soft archetype accepts it");
            }
        };
        let ev = lockstep(&label, Pipeline::SoftCoupled, Variant::default_cfg(w), &specs, ARM_STEPS, &mut script);
        println!("{label}: rules {:?}", ev.rules);
        assert!(ev.stats[HELD_BY - 1].held_rows == 3, "{label}: void: the tower was not held before the drop");
        assert!(ev.rules.d1_member >= 1, "{label}: void: the reaction restored nothing");
    }
}

/// D3 (design 04 A2.1): the hysteresis table grown on an Identity step while the towers are
/// held. The worlds start with a small table (`Manifolds::with_capacity(16)`, 32 slots, which
/// the towers' nine pairs keep); a static slab parked far away is written to hang over the
/// towers — a pose write moves no row — and its bounding sphere pairs it with every tower cube,
/// so the logical pair count doubles past the table on a step whose rows did not move. Every
/// held island keeping a box-box manifold restores on that step (under `Off` its hints are gone
/// too).
#[test]
fn s3_identity_step_grow_restores_by_d3() {
    let mut specs = towers();
    specs.push(Spec::wall(Vec3::new(0.0, 40.0, 40.0), Vec3::new(8.0, 0.1, 3.0)));
    arm(
        "S3 D3 grow",
        &specs,
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            if step == 0 {
                rig.world.insert_resource(Manifolds::with_capacity(16));
            }
            if step == HELD_BY {
                rig.body_mut(10).position = Vec3::new(0.0, 5.0, 0.0);
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.d3_clear >= 1, "{label}: void: no D3 restore");
        },
    );
}

/// Members and anchors that change what a held island's inputs are: a parked member (its
/// `Simulated` bit cleared at spawn) re-enabled while held (D1), the static floor turned by
/// 1e-4 rad in place — position and radius unchanged, the case the tree's hint cannot see
/// (D1a; design 04 M15's scene at C3b) — and a kinematic platform drifting under a tower, which
/// never lets that tower rest; a degenerate box (a zero half-extent) beside a tower, which never
/// rests. The platform's drift is written by the script, one step's worth each step: a
/// kinematic body's pose is gameplay-owned, and the physics does not integrate its
/// `linear_velocity`.
#[test]
fn s3_parked_member_floor_turn_kinematic_platform_degenerate_box() {
    let mut specs = towers();
    // A lone cube that starts parked (index 10): it rests on the floor, so no contact impulse
    // ever moves it, and it freezes like the towers.
    let mut parked = Spec::cube(Vec3::new(15.0, 0.5, 0.0), 0.5);
    parked.simulated = false;
    specs.push(parked);
    let mut platform = Spec::wall(Vec3::new(-20.0, 0.25, 0.0), Vec3::new(2.0, 0.25, 2.0));
    platform.kinematic = true;
    let drifting = specs.len();
    specs.push(platform);
    specs.extend(tower(2, -20.0, 0.0, 0.5).into_iter().map(|mut s| {
        s.position.y += 0.5;
        s
    }));
    let mut flat = Spec::cube(Vec3::new(10.0, 0.25, 0.0), 0.5);
    flat.shape = ColliderShape::Box { half_extents: Vec3::new(0.5, 0.0, 0.5) };
    specs.push(flat);
    arm(
        "S3 parked/floor/kinematic/degenerate",
        &specs,
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            rig.body_mut(drifting).position.x = -20.0 + 1.0e-3 * DT * (step + 1) as f32;
            if step == HELD_BY {
                let e = rig.bodies[10];
                rig.world.enable::<Simulated>(e);
            }
            if step == HELD_BY + 60 {
                rig.body_mut(0).rotation = yaw(1.0e-4);
            }
        },
        &|label, ev| {
            assert!(ev.stats[HELD_BY - 1].held_rows >= 10, "{label}: void: the towers and the parked cube were not held");
            let most = ev.stats.iter().map(|s| s.held_rows).max().unwrap_or(0);
            assert!(most <= 10, "{label}: the platform's tower or the degenerate box was held ({most} rows)");
            assert!(ev.rules.d1_member >= 1, "{label}: void: re-enabling the parked member restored nothing");
            assert!(ev.rules.d1a_anchor >= 1, "{label}: void: turning the floor restored nothing");
        },
    );
}

/// A missed gather while held (design 04 S3's last scene; D7; 08 O-2, N32; M-R4):
/// `physics_gather` run once between two steps, so every row-keyed consumer — the latch, the
/// mask, L9's carry, the solver's warm cursor, L10's own — classifies the next step as a Reset.
/// `Off` wakes the towers on the latch's Reset and misses every carried join and warm record;
/// `Sets` flushes every record on its Reset arm, which has no previous rows and so leaves both
/// restore sources unstamped (empty to the solve and the narrowphase). The step before, a member
/// teleport restores tower 0 (D1), so the sources hold that restore's entries when the Reset
/// arrives: a Reset that left them visible would seed the woken tower from them.
#[test]
fn s3_missed_gather_flushes_by_d7() {
    arm(
        "S3 missed gather",
        &towers(),
        ARM_STEPS,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY {
                rig.body_mut(2).position.x += 1.0e-3;
            }
            if step == HELD_BY + 1 {
                rig.world.run_system(physics_gather);
            }
        },
        &|label, ev| {
            assert_held_before_events(label, ev);
            assert!(ev.rules.d1_member >= 1, "{label}: void: the teleport restored nothing");
            assert!(ev.rules.d7_cursor >= 1, "{label}: void: the missed gather flushed nothing (D7)");
            let reset = ev.stats[HELD_BY + 1];
            assert!(
                reset.flushes == 1 && reset.restored >= 2,
                "{label}: void: the Reset step did not flush the two held towers: {reset:?}"
            );
            let back: u32 = ev.stats[HELD_BY + 2..].iter().map(|s| s.moved_in).sum();
            assert!(back >= 1, "{label}: void: no tower moved back in after the Reset");
        },
    );
}

// ── Suite A: deferral equivalence (L10 D9b; design 10, Tests A) ────────────────────────────

/// A step-boundary action on one world.
type Act = Box<dyn Fn(&mut Rig)>;

/// One deferral arm's write in its three forms (design 10, Tests A): M queues it into a writer
/// during step `at`; B takes the same write directly before step `at + 1`; C, the control, takes
/// it before step `at`.
struct Defer {
    /// Queues M's write(s) before step `at`.
    m: Act,
    /// The writers M's write rides; each must have applied exactly one write in step `at`.
    places: Vec<Place>,
    /// B's write before step `at + 1`; `None` for a `dt` or transient write, which B does not take.
    b: Option<Act>,
    /// C's write before step `at`.
    c: Act,
    /// C's undo before step `at + 1` (the `dt` and transient cases).
    c_undo: Option<Act>,
    /// A boundary action M and B both take before step `at` (DA8's first wake).
    both: Option<Act>,
}

impl Defer {
    /// A configuration write at `place`.
    fn cfg(place: Place, write: fn(&mut PhysicsConfig)) -> Self {
        Self {
            m: Box::new(move |rig: &mut Rig| rig.mid(place).cfg = Some(write)),
            places: vec![place],
            b: Some(Box::new(move |rig: &mut Rig| write(rig.cfg()))),
            c: Box::new(move |rig: &mut Rig| write(rig.cfg())),
            c_undo: None,
            both: None,
        }
    }

    /// A field assigned wholesale at `place`.
    fn field(place: Place, make: fn() -> SdfField) -> Self {
        Self {
            m: Box::new(move |rig: &mut Rig| rig.mid(place).field = Some(make())),
            places: vec![place],
            b: Some(Box::new(move |rig: &mut Rig| *rig.world.resource_mut::<SdfField>() = make())),
            c: Box::new(move |rig: &mut Rig| *rig.world.resource_mut::<SdfField>() = make()),
            c_undo: None,
            both: None,
        }
    }

    /// One `IslandSleep::wake_all()` at `place`.
    fn wake(place: Place) -> Self {
        Self {
            m: Box::new(move |rig: &mut Rig| rig.mid(place).wake = true),
            places: vec![place],
            b: Some(Box::new(wake_all)),
            c: Box::new(wake_all),
            c_undo: None,
            both: None,
        }
    }

    /// `write`, which sets `PhysicsConfig::dt` to `dt`, at `place`. The gather stamps `dt` from the
    /// clock every step, so a boundary write is overwritten by design: B takes none, and C runs
    /// step `at` with the clock itself at `dt`.
    fn dt(place: Place, write: fn(&mut PhysicsConfig), dt: f32) -> Self {
        Self {
            m: Box::new(move |rig: &mut Rig| rig.mid(place).cfg = Some(write)),
            places: vec![place],
            b: None,
            c: Box::new(move |rig: &mut Rig| set_clock(rig, dt)),
            c_undo: Some(Box::new(|rig: &mut Rig| set_clock(rig, DT))),
            both: None,
        }
    }
}

/// One explicit wake request.
fn wake_all(rig: &mut Rig) {
    rig.world.resource_mut::<IslandSleep>().wake_all();
}

/// The fixed clock's step, which the gather stamps into `PhysicsConfig::dt`.
fn set_clock(rig: &mut Rig, dt: f32) {
    rig.world.resource_mut::<FixedTime>().set_timestep(Duration::from_secs_f32(dt));
}

/// What a deferral probe sees after a step: the three worlds and B's and C's observations.
struct Seen<'r> {
    step: usize,
    m: &'r Rig,
    b: &'r Rig,
    c: &'r Rig,
    ob: &'r Obs,
    oc: &'r Obs,
}

/// A probe run after every deferral step.
type DeferProbe<'a> = dyn FnMut(&Seen<'_>) + 'a;

/// A per-step setup every world of a deferral run takes (a scene's step-0 edits).
type Setup<'a> = dyn Fn(usize, &mut Rig) + 'a;

/// Design 10's deferral driver: three worlds of `specs` on `pipeline` / `variant` / `mode` (`None`
/// = sleeping off) — M takes `write` in step `at`'s window, B at the next boundary, C before step
/// `at` — run for `at + 11` steps. M must equal B after every step (exactly: the same mode), and
/// every writer carrying M's write must have applied one write in step `at`. Returns the first step
/// of `[at, at + 10]` on which C differs from B, with the difference, or `None`.
#[allow(clippy::too_many_arguments)]
fn deferral_run(
    label: &str,
    pipeline: Pipeline,
    variant: Variant,
    mode: Option<SleepSkip>,
    specs: &[Spec],
    at: usize,
    setup: &Setup<'_>,
    write: &Defer,
    probe: &mut DeferProbe<'_>,
) -> Option<(usize, String)> {
    let mut m = Rig::with_mode(pipeline, variant, mode, specs);
    let mut b = Rig::with_mode(pipeline, variant, mode, specs);
    let mut c = Rig::with_mode(pipeline, variant, mode, specs);
    let mut c_differs = None;
    for step in 0..=at + 10 {
        for rig in [&mut m, &mut b, &mut c] {
            setup(step, rig);
        }
        if step == at {
            if let Some(both) = &write.both {
                both(&mut m);
                both(&mut b);
            }
            (write.m)(&mut m);
            (write.c)(&mut c);
        }
        if step == at + 1 {
            if let Some(boundary) = &write.b {
                boundary(&mut b);
            }
            if let Some(undo) = &write.c_undo {
                undo(&mut c);
            }
        }
        m.step();
        b.step();
        c.step();
        let (om, ob, oc) = (observe(&mut m.world), observe(&mut b.world), observe(&mut c.world));
        if let Some(why) = diff_named(&om, &ob, ("M", "B"), false) {
            panic!(
                "{label}: M (the write in step {at}'s window) diverges from B (the same write at the \
                 next boundary) after step {step}: {why}"
            );
        }
        if step >= at && c_differs.is_none() {
            c_differs = diff_named(&ob, &oc, ("B", "C"), false).map(|why| (step, why));
        }
        if step == at {
            for &place in &write.places {
                let applied = m.mid_ref(place).applied;
                assert_eq!(applied, 1, "{label}: void: M's {place:?} writer applied {applied} writes in step {at}");
            }
        }
        probe(&Seen { step, m: &m, b: &b, c: &c, ob: &ob, oc: &oc });
    }
    c_differs
}

/// [`deferral_run`], void unless C differs from B in `[at, at + 10]`: the written value must reach
/// the trajectory.
#[allow(clippy::too_many_arguments)]
fn deferral(
    label: &str,
    pipeline: Pipeline,
    variant: Variant,
    mode: Option<SleepSkip>,
    specs: &[Spec],
    setup: &Setup<'_>,
    write: &Defer,
    probe: &mut DeferProbe<'_>,
) {
    let Some((step, why)) = deferral_run(label, pipeline, variant, mode, specs, AT, setup, write, probe) else {
        panic!(
            "{label}: void: the control C (the write before step {AT}) equals B at every step of \
             [{AT}, {}]: the written value never reaches the trajectory",
            AT + 10
        );
    };
    println!("{label}: M == B at every step; C differs from B at step {step}: {why}");
}

/// The step the deferral arms write at: the towers are held by then.
const AT: usize = HELD_BY;

/// The deferral arms' sleep modes: sleeping off, `Off` and `Sets`.
const MODES: [Option<SleepSkip>; 3] = [None, Some(SleepSkip::Off), Some(SleepSkip::Sets)];

/// A ball rolling at 2 m/s on the floor, far from the towers: its speed² of 4 keeps it awake in
/// every mode, so every mode has a body whose contact and integration read the inputs.
fn witness_ball() -> Spec {
    Spec::ball(Vec3::new(-40.0, 0.3, 20.0), 0.3).moving(Vec3::new(2.0, 0.0, 0.0))
}

/// [`towers`] and the witness ball.
fn towers_and_ball() -> Vec<Spec> {
    let mut v = towers();
    v.push(witness_ball());
    v
}

/// S4's tower on the SDF floor, and the witness ball rolling on the field.
fn sdf_tower_and_ball() -> Vec<Spec> {
    let mut v = tower(3, 0.0, 0.0, 0.5);
    v.push(witness_ball());
    v
}

/// S4's variants: the default configuration and the Tree with its brute path off, at W ∈ {1, 8}.
fn s4_variants() -> Vec<(&'static str, Variant)> {
    let tree = |w| Variant { brute_max_rows: Some(0), ..Variant::cell(BroadphaseKind::Tree, true, w) };
    [1, 8].into_iter().flat_map(|w| [("default", Variant::default_cfg(w)), ("Tree", tree(w))]).collect()
}

/// The soft scene (design 10, Tests A): the floor, one tower, the witness ball, and — spawned at
/// step 0 by [`soft_setup`] — S5's braced cube resting on the floor at (3, 0.3, 0), with the SDF
/// floor at `y = 0` and the soft pass on. On `Soft` and `SoftColored` the field is the particles'
/// only floor.
fn soft_scene() -> Vec<Spec> {
    let mut v = vec![Spec::floor()];
    v.extend(tower(3, 0.0, 0.0, 0.5));
    v.push(witness_ball());
    v
}

/// S5's braced soft cube of half-extent 0.3 centred at `centre`.
fn soft_cube(centre: [f32; 3]) -> SoftBody {
    let half = 0.3f32;
    let positions: Vec<[f32; 3]> = (0..8u32)
        .map(|i| {
            let s = |bit: u32| if i & bit != 0 { half } else { -half };
            [centre[0] + s(1), centre[1] + s(2), centre[2] + s(4)]
        })
        .collect();
    let edges: Vec<(u32, u32)> = (0..8u32).flat_map(|a| ((a + 1)..8u32).map(move |b| (a, b))).collect();
    SoftBody::from_mesh(&positions, &[1.0; 8], &edges, None, 1.0e-6, 0.05).expect("construction: a braced cube is a soft body")
}

/// The soft scene's step-0 edits. A light viscous damping (`soft_damping = 0.02`) settles the
/// cube on the field well before `at`: undamped, the stiff cube keeps bouncing on the field in the
/// uncoupled pipelines, where it is in the air at `at` and a field write does not reach it within
/// ten steps.
fn soft_setup(step: usize, rig: &mut Rig) {
    if step == 0 {
        *rig.world.resource_mut::<SdfField>() = sdf_floor(0.0);
        rig.cfg().soft_body = true;
        rig.cfg().soft_damping = 0.02;
        let arch = rig.world.create_archetype(&[SoftBody::component_id()]);
        rig.world.spawn_one(arch, soft_cube([3.0, 0.3, 0.0])).expect("construction: the soft archetype accepts it");
    }
}

/// A field's edits as bits (centre, params, kind, op), for a value comparison.
fn field_bits(field: &SdfField) -> Vec<[u32; 10]> {
    field
        .edits()
        .iter()
        .map(|e| {
            [
                e.center[0].to_bits(),
                e.center[1].to_bits(),
                e.center[2].to_bits(),
                e.center[3].to_bits(),
                e.params[0].to_bits(),
                e.params[1].to_bits(),
                e.params[2].to_bits(),
                e.params[3].to_bits(),
                e.kind,
                e.op,
            ]
        })
        .collect()
}

/// Whether the writer at `place` saw `field` on its latest run (before its own write).
fn saw_field(rig: &Rig, place: Place, field: &SdfField) -> bool {
    rig.mid_ref(place).seen.as_ref().and_then(|(_, f)| f.as_ref()).is_some_and(|f| field_bits(f) == field_bits(field))
}

/// Whether some logical manifold joins a sphere to the SDF field.
fn sphere_on_the_field(rig: &Rig) -> bool {
    let bodies = rig.world.resource::<SolverScratch>().bodies();
    rig.world
        .resource::<Manifolds>()
        .manifolds()
        .iter()
        .any(|m| m.body_b == SDF_SENTINEL && matches!(bodies[m.body_a.0 as usize].shape, ColliderShape::Sphere { .. }))
}

/// `rig`'s dynamic rows, and how many of them are frozen (not awake) this step.
fn frozen_of(rig: &Rig) -> (usize, usize) {
    let scratch = rig.world.resource::<SolverScratch>();
    let sleep = rig.world.resource::<IslandSleep>();
    let dynamic: Vec<usize> = (0..scratch.bodies_len()).filter(|&r| scratch.bodies()[r].inv_mass != 0.0).collect();
    (dynamic.iter().filter(|&&r| !sleep.is_row_awake(r)).count(), dynamic.len())
}

/// The run's label.
fn cell_label(arm: &str, pipeline: Pipeline, variant: &Variant, mode: Option<SleepSkip>) -> String {
    let mode = mode.map_or_else(|| "sleeping off".to_owned(), |m| format!("{m:?}"));
    format!("{arm} [{pipeline:?} {:?} reuse {:?} W{}] {mode}", variant.kind, variant.reuse, variant.workers)
}

/// DA1: an SDF edit in the Early window (the floor lowered 1 mm) lands at the next step, as if
/// written at the boundary, in every mode. Red before D9b: the SDF stage read the live field.
#[test]
fn d9b_defer_sdf_edit() {
    let write = Defer::field(Place::Early, || sdf_floor(-1.0e-3));
    for (_, variant) in s4_variants() {
        for mode in MODES {
            let label = cell_label("DA1 SDF edit", Pipeline::Sdf, &variant, mode);
            let mut on_field = false;
            deferral(&label, Pipeline::Sdf, variant, mode, &sdf_tower_and_ball(), &quiet, &write, &mut |s| {
                if s.step == AT - 1 {
                    on_field = sphere_on_the_field(s.b);
                }
            });
            assert!(on_field, "{label}: void: the ball has no field manifold at step {}", AT - 1);
        }
    }
}

/// DA2: an SDF edit made Early (the floor 5 m lower) and reverted Late is invisible — B takes no
/// write; C edits before `at` and reverts before `at + 1`. Red before D9b at `at`.
#[test]
fn d9b_defer_sdf_edit_reverted() {
    let write = Defer {
        m: Box::new(|rig: &mut Rig| {
            rig.mid(Place::Early).field = Some(sdf_floor(-5.0));
            rig.mid(Place::Late).field = Some(sdf_floor(0.0));
        }),
        places: vec![Place::Early, Place::Late],
        b: None,
        c: Box::new(|rig: &mut Rig| *rig.world.resource_mut::<SdfField>() = sdf_floor(-5.0)),
        c_undo: Some(Box::new(|rig: &mut Rig| *rig.world.resource_mut::<SdfField>() = sdf_floor(0.0))),
        both: None,
    };
    for (_, variant) in s4_variants() {
        for mode in MODES {
            let label = cell_label("DA2 SDF edit reverted", Pipeline::Sdf, &variant, mode);
            let mut saw = false;
            deferral(&label, Pipeline::Sdf, variant, mode, &sdf_tower_and_ball(), &quiet, &write, &mut |s| {
                if s.step == AT {
                    saw = saw_field(s.m, Place::Late, &sdf_floor(-5.0));
                }
            });
            assert!(saw, "{label}: void: the Late writer did not see the edited field at step {AT}");
        }
    }
}

/// DA3 / DA4's variants: the Grid and the Tree, contact reuse on, W ∈ {1, 8}.
fn reuse_variants() -> Vec<Variant> {
    [BroadphaseKind::Grid, BroadphaseKind::Tree]
        .into_iter()
        .flat_map(|kind| [1, 8].map(|w| Variant::cell(kind, true, w)))
        .collect()
}

/// DA3 / DA4's body: `write` Early on the towers and the ball, every mode; the `Off`-like worlds
/// (sleeping off, `Off`) must have reused a contact the step before (the write reaches a reuse).
fn defer_reuse_write(arm: &str, write: fn(&mut PhysicsConfig)) {
    let write = Defer::cfg(Place::Early, write);
    for variant in reuse_variants() {
        for mode in MODES {
            let label = cell_label(arm, Pipeline::Default, &variant, mode);
            let mut reused = 0;
            deferral(&label, Pipeline::Default, variant, mode, &towers_and_ball(), &quiet, &write, &mut |s| {
                if s.step == AT - 1 {
                    reused = s.b.world.resource::<Manifolds>().pair_classes().reused;
                }
            });
            if mode != Some(SleepSkip::Sets) {
                assert!(reused > 0, "{label}: void: no contact was reused at step {}", AT - 1);
            }
        }
    }
}

/// DA3: `contact_reuse` flipped Early. Red before D9b in the sleeping-off and `Off` modes (the
/// narrowphase read the live flag); `Sets` skips the held pairs and the ball's pair is no box pair,
/// so there the control carries the arm.
#[test]
fn d9b_defer_reuse() {
    defer_reuse_write("DA3 contact_reuse", |c| c.contact_reuse = !c.contact_reuse);
}

/// DA4: τ = 0 Early, as DA3.
#[test]
fn d9b_defer_tau() {
    defer_reuse_write("DA4 tau", |c| c.contact_reuse_distance = 0.0);
}

/// DA5: `dt` doubled — run 1 Early (it reaches the ball through the narrowphase and the solve),
/// run 2 Late (through the solve) — is invisible: the gather stamps `dt` at the next step, so B
/// takes no write, and C runs step `at` with the clock doubled. Red before D9b at `at`.
#[test]
fn d9b_defer_dt() {
    for place in [Place::Early, Place::Late] {
        let write = Defer::dt(place, |c| c.dt = 2.0 * DT, 2.0 * DT);
        for w in [1, 8] {
            let variant = Variant::default_cfg(w);
            for mode in MODES {
                let label = cell_label(&format!("DA5 dt {place:?}"), Pipeline::Default, &variant, mode);
                // Run 1's anti-vacuity: the Late writer saw the Early write, which a solve reading
                // the configuration live would take. Run 2's is its writer's `applied == 1`.
                let mut saw = place == Place::Late;
                deferral(&label, Pipeline::Default, variant, mode, &towers_and_ball(), &quiet, &write, &mut |s| {
                    if s.step == AT && place == Place::Early {
                        saw = s.m.mid_ref(Place::Late).seen.is_some_and(|(c, _)| c.dt == 2.0 * DT);
                    }
                });
                assert!(saw, "{label}: void: the Late writer did not see dt = 2 DT at step {AT}");
            }
        }
    }
}

/// DA6 (W1's witness): run 1 halves gravity Late (it reaches the rigid solve and the soft step),
/// run 2 raises the field 5 cm Late (the soft step alone), on every soft shape — the coupled, the
/// plain and the colored soft step, and the reference pipeline — in every mode the shape runs. C
/// must differ from B in the soft particles themselves. Red before D9b at `at`: the soft steps
/// read the live configuration and field.
#[test]
fn d9b_defer_soft() {
    let runs: [(&str, Defer); 2] = [
        ("gravity", Defer::cfg(Place::Late, |c| c.gravity.y *= 0.5)),
        ("field", Defer::field(Place::Late, || sdf_floor(0.05))),
    ];
    let shapes = [
        (Pipeline::SoftCoupled, &MODES[..]),
        (Pipeline::Soft, &MODES[..]),
        (Pipeline::SoftColored, &MODES[..]),
        (Pipeline::SoftCoupledRef, &MODES[..1]),
    ];
    for (run, write) in &runs {
        for (pipeline, modes) in shapes {
            for w in [1, 8] {
                let variant = Variant::default_cfg(w);
                for &mode in modes {
                    let label = cell_label(&format!("DA6 soft {run}"), pipeline, &variant, mode);
                    let mut soft_differs = false;
                    deferral(&label, pipeline, variant, mode, &soft_scene(), &soft_setup, write, &mut |s| {
                        soft_differs |= s.step >= AT && s.ob.soft != s.oc.soft;
                    });
                    assert!(soft_differs, "{label}: void: C equals B in the soft particles over [{AT}, {}]", AT + 10);
                }
            }
        }
    }
}

/// DA7 (C1): `wake_all()` Early, and again Late, while the towers are frozen. Red before D9b at
/// `at`: the solve served the live request.
#[test]
fn d9b_defer_wake_all() {
    for place in [Place::Early, Place::Late] {
        let write = Defer::wake(place);
        for variant in arm_variants() {
            for mode in [SleepSkip::Off, SleepSkip::Sets] {
                let label = cell_label(&format!("DA7 wake_all {place:?}"), Pipeline::Default, &variant, Some(mode));
                let mut frozen = (0, 1);
                deferral(&label, Pipeline::Default, variant, Some(mode), &towers(), &quiet, &write, &mut |s| {
                    if s.step == AT - 1 {
                        frozen = frozen_of(s.b);
                    }
                });
                assert_eq!(frozen.0, frozen.1, "{label}: void: frozen / dynamic rows at step {}", AT - 1);
            }
        }
    }
}

/// DA8: with `sleep_frames = 1`, M and B take a wake before `at`; M takes a second one in `at`'s
/// window, B before `at + 1`. A request raised after the latch must not be swallowed by one already
/// pending: the rows `end_step(at)` re-latches are woken again at `at + 1`. Red before D9b at
/// `at + 1` (M's second request is served with the first, at `at`).
#[test]
fn d9b_defer_wake_all_twice() {
    let write = Defer {
        m: Box::new(|rig: &mut Rig| rig.mid(Place::Early).wake = true),
        places: vec![Place::Early],
        b: Some(Box::new(wake_all)),
        c: Box::new(|rig: &mut Rig| {
            wake_all(rig);
            wake_all(rig);
        }),
        c_undo: None,
        both: Some(Box::new(wake_all)),
    };
    // A threshold two orders above the default, so the towers re-latch on the step a wake is
    // served, whatever the variant; with the default one the woken towers of some variants stay
    // above it for a step, and the arm is void.
    let setup = |step: usize, rig: &mut Rig| {
        if step == 0 {
            rig.cfg().sleep_frames = 1;
            rig.cfg().sleep_threshold = 1.0e-2;
        }
    };
    for variant in arm_variants() {
        for mode in [SleepSkip::Off, SleepSkip::Sets] {
            let label = cell_label("DA8 wake_all twice", Pipeline::Default, &variant, Some(mode));
            let mut relatched = (0, 1);
            deferral(&label, Pipeline::Default, variant, Some(mode), &towers(), &setup, &write, &mut |s| {
                // C took both wakes before `at` and none after: its rows frozen at `at + 1` are
                // the ones `end_step(at)` re-latched, as in B, whose latch after `at` is C's.
                if s.step == AT + 1 {
                    relatched = frozen_of(s.c);
                }
            });
            assert!(relatched.0 > 0, "{label}: void: no row re-latched at step {AT} ({relatched:?} frozen / dynamic)");
        }
    }
}

// ── DA9: every `PhysicsConfig` field, one at a time ────────────────────────────────────────

/// A field's class on a DA9 shape (design 10, Tests A, DA9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// Perturbed; M == B, and C ≠ B in at least one mode of the shape.
    Observable,
    /// Perturbed (it exercises the latch copy); M == B; no C ≠ B requirement. The reason is at
    /// the row.
    Unobservable,
    /// Not perturbed: it records the pipeline's wiring. The reason is at the row.
    Wiring,
}

/// One DA9 row.
struct Row {
    name: &'static str,
    write: fn(&mut PhysicsConfig),
    class: Class,
}

/// DA9's shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// The towers and the ball on the SDF pipeline.
    Sdf,
    /// The soft scene on the coupled soft pipeline.
    SoftCoupled,
    /// The soft scene on the colored soft pipeline.
    SoftColored,
}

/// Every `PhysicsConfig` field with its perturbation and its class on `shape`. The destructure is
/// exhaustive, so a field added to `PhysicsConfig` fails to compile until it is classified here,
/// and [`da9`] checks the rows against the fields `Debug` prints.
fn da9_rows(shape: Shape) -> Vec<Row> {
    use Class::{Observable as O, Unobservable as U, Wiring as W};
    let PhysicsConfig {
        gravity: _,
        dt: _,
        substeps: _,
        relax_iterations: _,
        contact_hertz: _,
        contact_damping: _,
        broadphase: _,
        broadphase_select: _,
        simd: _,
        simd_solve: _,
        sdf_narrowphase: _,
        parallel_broadphase: _,
        colored: _,
        parallel_solve: _,
        parallel_narrowphase: _,
        contact_reuse: _,
        contact_reuse_distance: _,
        sleeping: _,
        sleep_skip: _,
        sleep_threshold: _,
        sleep_frames: _,
        soft_body: _,
        soft_damping: _,
        soft_rest_clamp: _,
        soft_rigid_coupling: _,
        self_collision_iters: _,
        soft_body_colored: _,
        soft_self_collision_colored: _,
    } = PhysicsConfig::default();
    let soft = shape != Shape::Sdf;
    let coupled = shape == Shape::SoftCoupled;
    // `pick(yes, no)`: the class on the shapes `yes` holds for.
    let pick = |yes: bool, a: Class, b: Class| if yes { a } else { b };
    vec![
        Row { name: "gravity", write: |c| c.gravity.y *= 0.5, class: O },
        // The special case: the gather stamps `dt` (`Defer::dt`).
        Row { name: "dt", write: |c| c.dt = 2.0 * DT, class: O },
        Row { name: "substeps", write: |c| c.substeps += 1, class: O },
        Row { name: "relax_iterations", write: |c| c.relax_iterations += 1, class: O },
        Row { name: "contact_hertz", write: |c| c.contact_hertz *= 0.5, class: O },
        Row { name: "contact_damping", write: |c| c.contact_damping *= 0.5, class: O },
        // The coupling path requires the grid (`soft/solver.rs`): wiring there. Elsewhere every
        // kind emits the same pair set (the tree's `PairOracle` checks it).
        Row { name: "broadphase", write: |c| c.broadphase = BroadphaseKind::Tree, class: pick(coupled, W, U) },
        Row { name: "broadphase_select", write: |c| c.broadphase_select = BroadphaseSelectMode::Auto, class: pick(coupled, W, U) },
        // The bit-identity theorems: each changes cost, never a result bit.
        Row { name: "simd", write: |c| c.simd = !c.simd, class: U },
        Row { name: "simd_solve", write: |c| c.simd_solve = !c.simd_solve, class: U },
        // The AVX2 fold differs from the scalar one only in ±0 ties (`resources.rs`), which these
        // scenes do not reach.
        Row { name: "sdf_narrowphase", write: |c| c.sdf_narrowphase = SdfNarrowphaseKernel::Avx2, class: U },
        Row { name: "parallel_broadphase", write: |c| c.parallel_broadphase = !c.parallel_broadphase, class: U },
        // It records the schedule's shape and nothing reads it (`resources.rs`).
        Row { name: "colored", write: |c| c.colored = !c.colored, class: W },
        Row { name: "parallel_solve", write: |c| c.parallel_solve = !c.parallel_solve, class: U },
        Row { name: "parallel_narrowphase", write: |c| c.parallel_narrowphase = !c.parallel_narrowphase, class: U },
        Row { name: "contact_reuse", write: |c| c.contact_reuse = !c.contact_reuse, class: O },
        Row { name: "contact_reuse_distance", write: |c| c.contact_reuse_distance = 0.0, class: O },
        Row { name: "sleeping", write: |c| c.sleeping = !c.sleeping, class: O },
        // Off ≡ Sets (L10's invariant), and inert with sleeping off.
        Row { name: "sleep_skip", write: |c| c.sleep_skip = SleepSkip::Off, class: U },
        Row { name: "sleep_threshold", write: |c| c.sleep_threshold = 0.0, class: O },
        Row { name: "sleep_frames", write: |c| c.sleep_frames += 10, class: O },
        // No soft body on the SDF shape; the soft step is not registered there.
        Row { name: "soft_body", write: |c| c.soft_body = !c.soft_body, class: pick(soft, O, U) },
        Row { name: "soft_damping", write: |c| c.soft_damping = 0.5, class: pick(soft, O, U) },
        Row { name: "soft_rest_clamp", write: |c| c.soft_rest_clamp = !c.soft_rest_clamp, class: pick(soft, O, U) },
        // Read by the coupled step only.
        Row { name: "soft_rigid_coupling", write: |c| c.soft_rigid_coupling = !c.soft_rigid_coupling, class: pick(coupled, O, U) },
        // The cube's particles are 0.6 apart, past twice their radius (0.1): the self-collision
        // pass finds no pair to push (measured by DA9's first run).
        Row { name: "self_collision_iters", write: |c| c.self_collision_iters += 1, class: U },
        // Read by the colored soft step only.
        Row { name: "soft_body_colored", write: |c| c.soft_body_colored = !c.soft_body_colored, class: pick(shape == Shape::SoftColored, O, U) },
        // Self-collision is off in these scenes (`self_collision_iters` is 0).
        Row { name: "soft_self_collision_colored", write: |c| c.soft_self_collision_colored = !c.soft_self_collision_colored, class: U },
    ]
}

/// The top-level field names `Debug` prints for a struct value.
fn debug_field_names(printed: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut depth = 0u32;
    let mut start = None;
    for (i, ch) in printed.char_indices() {
        match ch {
            '{' | '(' | '[' => {
                depth += 1;
                start = None;
            }
            '}' | ')' | ']' => {
                depth -= 1;
                start = None;
            }
            ':' if depth == 1 && printed[i + 1..].starts_with(' ') => {
                if let Some(s) = start.take() {
                    names.push(&printed[s..i]);
                }
            }
            c if c.is_alphanumeric() || c == '_' => {
                start.get_or_insert(i);
            }
            _ => start = None,
        }
    }
    names
}

/// DA9 on `shape`: every perturbable field, one at a time, Early at `at`, sleeping off and `Sets`.
fn da9(shape: Shape) {
    let (pipeline, specs) = match shape {
        Shape::Sdf => (Pipeline::Sdf, sdf_tower_and_ball()),
        Shape::SoftCoupled => (Pipeline::SoftCoupled, soft_scene()),
        Shape::SoftColored => (Pipeline::SoftColored, soft_scene()),
    };
    let setup: &Setup<'_> = if shape == Shape::Sdf { &quiet } else { &soft_setup };
    let rows = da9_rows(shape);
    // Completeness against the type itself: `Debug` prints every field, once, at depth 1.
    let mut names: Vec<&str> = rows.iter().map(|r| r.name).collect();
    let printed = format!("{:?}", PhysicsConfig::default());
    let mut fields = debug_field_names(&printed);
    names.sort_unstable();
    fields.sort_unstable();
    assert_eq!(names, fields, "DA9: the rows are not PhysicsConfig's fields");
    let variant = Variant::default_cfg(1);
    let mut unobservable = Vec::new();
    for row in rows.iter().filter(|r| r.class != Class::Wiring) {
        let write = if row.name == "dt" { Defer::dt(Place::Early, row.write, 2.0 * DT) } else { Defer::cfg(Place::Early, row.write) };
        let mut seen = Vec::new();
        for mode in [None, Some(SleepSkip::Sets)] {
            let label = format!("DA9 {shape:?} {} {}", row.name, mode.map_or("sleeping off", |_| "Sets"));
            if let Some((step, _)) = deferral_run(&label, pipeline, variant, mode, &specs, AT, setup, &write, &mut |_| {}) {
                seen.push(format!("{} at {step}", mode.map_or("off", |_| "Sets")));
            }
        }
        match row.class {
            Class::Observable => assert!(
                !seen.is_empty(),
                "DA9 {shape:?}: {} is misclassified: C equals B in every mode, so it is not observable in this scene",
                row.name
            ),
            Class::Unobservable => {
                if seen.is_empty() {
                    unobservable.push(row.name);
                } else {
                    println!("DA9 {shape:?}: {} (classed unobservable) differs: {seen:?}", row.name);
                }
            }
            Class::Wiring => unreachable!("wiring rows are filtered"),
        }
    }
    println!("DA9 {shape:?}: every perturbed field deferred exactly; unobservable here: {unobservable:?}");
}

/// DA9 on the towers and the ball, SDF pipeline.
#[test]
fn d9b_defer_every_field_sdf() {
    da9(Shape::Sdf);
}

/// DA9 on the soft scene, coupled soft pipeline.
#[test]
fn d9b_defer_every_field_soft_coupled() {
    da9(Shape::SoftCoupled);
}

/// DA9 on the soft scene, colored soft pipeline.
#[test]
fn d9b_defer_every_field_soft_colored() {
    da9(Shape::SoftColored);
}

// ── Suite B: Off ≡ Sets under window writes (L10's invariant; design 10, Tests B) ──────────

/// Queues `write` into the writer at `place` of the world `rig` at `step == at`.
fn queue_cfg(rig: &mut Rig, place: Place, write: fn(&mut PhysicsConfig)) {
    rig.mid(place).cfg = Some(write);
}

/// LB1: an SDF edit (the floor lowered 1 mm) Early while the tower is held lands at the next
/// step: that step's epoch flushes, and `Off` recomputes the pile's field manifolds. Red before
/// D9b at `HELD_BY`: `Off`'s SDF stage read the new field while `Sets` kept the held manifolds.
#[test]
fn s4_mid_step_sdf_edit_lands_next_step() {
    for (kind, variant) in s4_variants() {
        let label = format!("LB1 [{kind}] W{}", variant.workers);
        let mut script = |step: usize, rig: &mut Rig| {
            if step == HELD_BY {
                rig.mid(Place::Early).field = Some(sdf_floor(-1.0e-3));
            }
        };
        let ev = lockstep(&label, Pipeline::Sdf, variant, &tower(3, 0.0, 0.0, 0.5), HELD_BY + 11, &mut script);
        let k = HELD_BY;
        assert_eq!(ev.stats[k].held_rows, 3, "{label}: void: the tower was not held at the write's step");
        assert!(
            ev.stats[k + 1].flushes == 1 && ev.rules_at[k + 1].d5_epoch == ev.rules_at[k].d5_epoch + 1,
            "{label}: void: the next step did not flush on the epoch: {:?}",
            ev.stats[k + 1]
        );
        assert_ne!(ev.off_manifolds[k + 1], ev.off_manifolds[k - 1], "{label}: void: Off's manifolds did not change");
    }
}

/// LB2: an SDF edit Early (the floor 5 m lower) reverted Late is invisible to both modes. Red
/// before D9b at `HELD_BY`, permanently: `Off`'s SDF stage saw the lowered floor.
#[test]
fn s4_mid_step_sdf_edit_reverted_is_invisible() {
    for (kind, variant) in s4_variants() {
        let label = format!("LB2 [{kind}] W{}", variant.workers);
        let mut script = |step: usize, rig: &mut Rig| {
            if step == HELD_BY {
                rig.mid(Place::Early).field = Some(sdf_floor(-5.0));
                rig.mid(Place::Late).field = Some(sdf_floor(0.0));
            }
        };
        let mut saw = false;
        let ev = lockstep_probed(&label, Pipeline::Sdf, variant, &tower(3, 0.0, 0.0, 0.5), HELD_BY + 11, &mut script, &mut |step, _, sets| {
            if step == HELD_BY {
                saw = saw_field(sets, Place::Late, &sdf_floor(-5.0));
            }
        });
        assert!(saw, "{label}: void: the Late writer did not see the edited field");
        let d5 = ev.rules_at[HELD_BY - 1].d5_epoch;
        assert!((HELD_BY..=HELD_BY + 10).all(|k| ev.rules_at[k].d5_epoch == d5), "{label}: the reverted edit changed the epoch");
        assert!(
            (HELD_BY..=HELD_BY + 10).all(|k| ev.frozen_rows[k] == ev.dynamic_rows[k]),
            "{label}: the pile did not stay latched through step {}",
            HELD_BY + 10
        );
    }
}

/// LB3 / LB4's body: `write` Early at `HELD_BY` on the towers, on the Tree and the Grid with
/// contact reuse on; the next step flushes and `Off` recomputes the reused pairs.
fn mid_step_reuse_write(arm: &str, write: fn(&mut PhysicsConfig)) {
    for variant in [BroadphaseKind::Tree, BroadphaseKind::Grid].into_iter().flat_map(|k| [1, 8].map(|w| Variant::cell(k, true, w))) {
        let label = format!("{arm} [{:?} W{}]", variant.kind, variant.workers);
        let mut script = |step: usize, rig: &mut Rig| {
            if step == HELD_BY {
                queue_cfg(rig, Place::Early, write);
            }
        };
        let ev = lockstep(&label, Pipeline::Default, variant, &towers(), HELD_BY + 11, &mut script);
        let k = HELD_BY;
        assert!(ev.off_reused[k - 1] > 0, "{label}: void: Off reused no contact before the write");
        assert_eq!(ev.stats[k + 1].flushes, 1, "{label}: void: the next step did not flush: {:?}", ev.stats[k + 1]);
        assert_ne!(ev.off_manifolds[k + 1], ev.off_manifolds[k - 1], "{label}: void: Off's manifolds did not change");
    }
}

/// LB3: `contact_reuse` flipped Early while the towers are held. Red before D9b at `HELD_BY`:
/// `Off` recomputed the frozen pairs with the flag the write set, while `Sets` skipped them.
#[test]
fn s3_mid_step_reuse_toggle_lands_next_step() {
    mid_step_reuse_write("LB3 contact_reuse", |c| c.contact_reuse = !c.contact_reuse);
}

/// LB4: τ = 0 Early, as LB3.
#[test]
fn s3_mid_step_reuse_distance_lands_next_step() {
    mid_step_reuse_write("LB4 tau", |c| c.contact_reuse_distance = 0.0);
}

/// LB5's written step: long enough that L9's `is_fast` classes the held towers' box pairs as fast
/// (checked by the reach world, below).
const DT_W: f32 = 1.0e3;

/// LB5: `dt = DT_W` Early, then `dt = DT` Late, at `HELD_BY`: the solve runs with `DT` on either
/// tree, so only the narrowphase's `is_fast` can see `DT_W`. Red before D9b at `HELD_BY`: `Off`'s
/// narrowphase classed the frozen box pairs as fast and ran their full collision, while `Sets`
/// skipped them.
///
/// Anti-vacuity, per run: (i) the Late writer saw `DT_W`; (ii) a reach world R (`Off`, idle
/// writers) run with the clock at `DT_W` on step `HELD_BY` loses a reuse among an unchanged or
/// larger pair set while every row is frozen (poses unchanged: the reuse criterion reads only
/// shapes and poses, so that is an `is_fast` flip); (iii) the epoch is unchanged at `HELD_BY + 1`.
#[test]
fn s3_mid_step_dt_write_is_invisible() {
    for variant in [BroadphaseKind::Tree, BroadphaseKind::Grid].into_iter().flat_map(|k| [1, 8].map(|w| Variant::cell(k, true, w))) {
        let label = format!("LB5 dt [{:?} W{}]", variant.kind, variant.workers);
        let mut script = |step: usize, rig: &mut Rig| {
            if step == HELD_BY {
                queue_cfg(rig, Place::Early, |c| c.dt = DT_W);
                queue_cfg(rig, Place::Late, |c| c.dt = DT);
            }
        };
        let mut saw = false;
        let ev = lockstep_probed(&label, Pipeline::Default, variant, &towers(), HELD_BY + 11, &mut script, &mut |step, _, sets| {
            if step == HELD_BY {
                saw = sets.mid_ref(Place::Late).seen.is_some_and(|(c, _)| c.dt == DT_W);
            }
        });
        assert!(saw, "{label}: void (i): the Late writer did not see dt = DT_W");
        assert_eq!(
            ev.rules_at[HELD_BY + 1].d5_epoch,
            ev.rules_at[HELD_BY].d5_epoch,
            "{label}: (iii) the restored dt changed the epoch"
        );
        let mut reach = Rig::new(Pipeline::Default, variant, SleepSkip::Off, &towers());
        let (mut frozen, mut pairs, mut reused) = ((0, 1), [0; 2], [0; 2]);
        for step in 0..=HELD_BY {
            if step == HELD_BY {
                set_clock(&mut reach, DT_W);
            }
            reach.step();
            if step + 1 >= HELD_BY {
                let c = reach.world.resource::<Manifolds>().pair_classes();
                (pairs[step + 1 - HELD_BY], reused[step + 1 - HELD_BY]) = (c.pairs, c.reused);
            }
            if step == HELD_BY - 1 {
                frozen = frozen_of(&reach);
            }
        }
        assert!(
            frozen.0 == frozen.1 && pairs[1] >= pairs[0] && reused[1] < reused[0],
            "{label}: void (ii): DT_W does not flip is_fast here: frozen {frozen:?}, pairs {pairs:?}, reused {reused:?}"
        );
    }
}

/// LB6 (C1): `wake_all()` while the towers are held, Early and (a second run) Late, lands at the
/// next step in both modes: its broadphase flushes (D6) and the solve clears every latch. Red
/// before D9b at `HELD_BY`: `Off`'s solve served the live request and solved the piles, while the
/// held rows stayed at inverse mass 0 under `Sets`.
#[test]
fn s3_mid_step_wake_all_lands_next_step() {
    for place in [Place::Early, Place::Late] {
        arm(
            &format!("LB6 wake_all {place:?}"),
            &towers(),
            HELD_BY + 11,
            &mut |step, rig: &mut Rig| {
                if step == HELD_BY {
                    rig.mid(place).wake = true;
                }
            },
            &|label, ev| {
                let k = HELD_BY;
                assert_eq!(ev.stats[k].held_rows as usize, ev.dynamic_rows[k], "{label}: void: not every row held at the write");
                assert!(
                    ev.stats[k + 1].flushes == 1 && ev.rules_at[k + 1].d6_wake_all == ev.rules_at[k].d6_wake_all + 1,
                    "{label}: void: the next step did not flush on the wake: {:?}",
                    ev.stats[k + 1]
                );
                assert_eq!(ev.frozen_rows[k + 1], 0, "{label}: void: rows stayed frozen after the wake");
            },
        );
    }
}

/// The control: `sleep_threshold = 0` Early. The solve reads it in `end_step`, which does not
/// depend on the mode, so this is green on either tree; the wake it causes lands in both worlds.
#[test]
fn s3_mid_step_sleep_threshold_is_mode_independent() {
    arm(
        "LB control sleep_threshold",
        &towers(),
        HELD_BY + 11,
        &mut |step, rig: &mut Rig| {
            if step == HELD_BY {
                queue_cfg(rig, Place::Early, |c| c.sleep_threshold = 0.0);
            }
        },
        &|label, ev| {
            assert!(
                (HELD_BY + 1..=HELD_BY + 3).any(|k| ev.frozen_rows[k] < ev.dynamic_rows[k]),
                "{label}: void: no wake within steps {}..={}",
                HELD_BY + 1,
                HELD_BY + 3
            );
        },
    );
}
