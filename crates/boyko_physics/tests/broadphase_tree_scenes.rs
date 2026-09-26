//! G2 of the tree broadphase design (`docs/physics/perf-campaign/levers/broadphase/04-DESIGN-REV2.md`):
//! the scene oracle on every step plus the structural bounds, on the real colored schedule with
//! the Tree forced (`brute_max_rows = 0`), and the pose bytes of an AllPairs twin world.
//!
//! # What is asserted
//!
//! A probe system between the broadphase and the narrowphase recomputes the pair set with
//! [`all_pairs_into`] on the very snapshot the broadphase read and counts the steps on which the
//! Tree's `ContactPairs` differs — asserted zero, with the probe's step count asserted equal to
//! the steps run (a probe that never ran cannot pass). Every dynamic body's `RigidBody` bits are
//! compared with an AllPairs world stepped in lockstep, after every step.
//!
//! | scene | structural assertions (sleeping off) |
//! |---|---|
//! | J (Jolt's pyramid, gap 0.5), [`PYRAMID_STEPS`] steps | `static_rebuilds == 1` from step 2; `sleeper_rebuilds == 0`, `hint_candidates == 0`; evictions, translations, wide and excluded rows all 0; `members == 1` from step 2 |
//! | R (the touching pyramid, gap 0) | the same |
//! | S16 (the 16-box tower) | the oracle and the pose bytes |
//! | churn (`row_identity_churn`'s scene): 6 arms, settle [`CHURN_SETTLE`], then [`CHURN_STEPS`] churn steps | every arm: Δ`static_rebuilds` == 0 and Δ`evictions` == 0; non-stable arms Δ`translations` == 40, stable 0; `members == 1` on every step |
//!
//! | J-Son, R-S (sleeping on, L10's `Sets`: the sleeper set, commit C5 built as L10 C3c) | the oracle over the LOGICAL pairs (the stream ⊎ the withheld pairs) and the pose bytes of a sleeping-on AllPairs twin on every step; `static_rebuilds == 1` from step 2; translations, wide and excluded rows and locator resets 0; A — the first step with every dynamic row held — exists and is ≤ 300 (a void is red; debug's 10-layer rest pile ≤ 600, [`R_S_ROW`]); after A, Δ`sleeper_rebuilds` ≤ 1 and Δ`evictions` == 0; the total `sleeper_rebuilds` pinned; at the end every box is a sleeper and every pair withheld. Then the toggle-off script: sleeping off with no row change dissolves the sleeper set on that step, and on the steps after it Δ`hint_candidates`, Δ`sleeper_rebuilds` and Δ`evictions` are 0 and the floor stays in S. |
//!
//! The sleeping-on churn arms of the design's G2 table are not built here: L10's plan for C3c
//! names the J-Son and R-S rows (its T7) only.
//!
//! # G-TH1: the brute threshold's band (window 7 wave 2, Q3; `levers/broadphase/06-DESIGN-F3.md`)
//!
//! The one rig here at the DEFAULT `brute_max_rows` ([`TREE_BRUTE_MAX_ROWS`]): ten three-layer
//! rest piles (141 rows) and a script that spawns one box every [`BAND_PERIOD`] steps up to 150
//! rows and despawns back, twice, so the row count crosses the threshold both ways, with sleeping
//! off and on. On every step the oracle and the pose bytes of the AllPairs twin hold, and the
//! path derived from the row count against `brute_max_rows()` is the path the step took — a
//! tree-path step answers leaf-list leaves or holds sleepers, a brute step moves no counter and
//! holds no sleeper (T6). Void unless brute and tree steps both ran, the crossings happened, and
//! (sleeping on) a descent crossed with a live sleeper set. Shown red: at the provisional 64 (no
//! brute step); with `all_pairs_into`'s inner loop reversed (the pose: the stream's order reaches
//! it); with `n < brute_max_rows` in `step_hinted` (the step at 144 rows takes the tree path).
//!
//! # G-F3-8: the kd leaf order (`levers/broadphase/06-DESIGN-F3.md`)
//!
//! Every Tree rig above — J, R, S16, the churn arms, J-Son and R-S — runs again with the Tree's
//! query kernel set to `QueryKernel::LeafListKd`, the active tree built in the kd median-split
//! leaf order: the oracle and the AllPairs twin's pose bytes on every step, as on the default
//! kernel, and the receipt `kd_order_builds` equal to the tree-path steps (0 on the default
//! kernel's rigs). The kd order moves the query stream's segments, never `ContactPairs`, so the
//! pose is the twin's. Shown red: with the rev scatter of the assembly walking the Q rows in slot
//! order (an order-only leak: the pair set is kept), the kd rigs' oracle — which compares the
//! pair lists in order — fails at the first step with a static member.
//!
//! # Scene size per profile
//!
//! Release runs Jolt's full pyramid (height 15, 1 240 boxes) for 600 steps, as the design
//! specifies. Debug runs height 10 (385 boxes) for 200 steps, so the file stays in the ordinary
//! debug run — the same split `default_world_pyramid_determinism.rs` makes, for the same reason:
//! the oracle is O(n²) and an unoptimised 1 240-box world is minutes. Every assertion above is
//! scale-free.
//!
//! # Shown red (the mutations of the design's table)
//!
//! * M5s, the cursor stamp removed: `translations` 0 ≠ 40 on the churn arms.
//! * M5r, rev 1's rule (rebuild the set on every `Rows` step): Δ`static_rebuilds` == 0 fails.
//! * M3, rev entries dropped: the oracle differs from the admission step on.
//!
//! Spins real thread pools, so `cfg(not(miri))`.

#![cfg(not(miri))]

use std::sync::Arc;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::{Commands, Res, ResMut};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_macros::{Component, Resource};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::broadphase_tree::{
    BroadphaseTree, QueryKernel, TREE_BRUTE_MAX_ROWS, TreeDiag, all_pairs_into,
};
use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{BroadphaseKind, ContactPairs, PhysicsConfig, SolverScratch};

// ── Scene constants (Jolt `PyramidScene.h`, as the parity runner transcribes them) ──

const BOX_SIZE: f32 = 2.0;
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
const JOLT_SEPARATION: f32 = 0.5;
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
const JOLT_FRICTION: f32 = 0.2;
const REST_FRICTION: f32 = 0.5;
const DT: f32 = 1.0 / 60.0;
const BOX_INV_MASS: f32 = 0.125;
const BOX_INV_INERTIA: f32 = 0.1875;
const TOWER_BODIES: usize = 16;

/// Pyramid layers: Jolt's 15 in release, 10 in debug (see the module docs).
#[cfg(not(debug_assertions))]
const PYRAMID_HEIGHT: i32 = 15;
#[cfg(debug_assertions)]
const PYRAMID_HEIGHT: i32 = 10;

/// Steps of the pyramid runs.
#[cfg(not(debug_assertions))]
const PYRAMID_STEPS: usize = 600;
#[cfg(debug_assertions)]
const PYRAMID_STEPS: usize = 200;

/// Steps of the tower run.
const TOWER_STEPS: usize = 300;

/// Settle steps before a churn window (sleeping off), as the bench has it.
const CHURN_SETTLE: usize = 30;
/// Churn steps per arm.
const CHURN_STEPS: u64 = 40;

// ── The probe ─────────────────────────────────────────────────────────────────

/// The oracle beside the broadphase's output: runs between the two, on the snapshot both read.
#[derive(Resource)]
struct Probe {
    oracle: ContactPairs,
    steps: u64,
    mismatches: u64,
    first_mismatch: Option<u64>,
}

impl Default for Probe {
    fn default() -> Self {
        Self { oracle: ContactPairs::with_capacity(0), steps: 0, mismatches: 0, first_mismatch: None }
    }
}

// `clippy::needless_pass_by_value`: `Res` / `ResMut` are by-value `SystemParam`s.
#[allow(clippy::needless_pass_by_value)]
fn probe_pairs(scratch: Res<SolverScratch>, pairs: Res<ContactPairs>, mut probe: ResMut<Probe>) {
    let probe = &mut *probe;
    all_pairs_into(scratch.bodies(), &mut probe.oracle);
    if probe.oracle.pairs() != pairs.pairs() {
        probe.mismatches += 1;
        probe.first_mismatch.get_or_insert(probe.steps);
    }
    probe.steps += 1;
}

// ── Worlds ────────────────────────────────────────────────────────────────────

/// Views a `#[repr(C)]` POD as its bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, initialised `#[repr(C)]` `T` borrowed for the returned slice's
    // lifetime; the slice covers exactly its `size_of::<T>()` bytes read-only, the layout the
    // component pool stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn spawn_box(world: &mut EcsMaster, position: Vec3, friction: f32, dynamic: bool) -> Entity {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let (mass, half_extents) = if dynamic {
        (
            RigidBodyMass {
                inv_inertia: Mat3::from_diagonal(Vec3::new(BOX_INV_INERTIA, BOX_INV_INERTIA, BOX_INV_INERTIA)),
                inv_mass: BOX_INV_MASS,
                restitution: 0.0,
                friction,
            },
            Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
        )
    } else {
        (RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass: 0.0, restitution: 0.0, friction }, FLOOR_HALF_EXTENTS)
    };
    let collider = Collider { shape: ColliderShape::Box { half_extents }, layer: 1, mask: 1 };
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
    if dynamic {
        world.enable::<Simulated>(e);
    }
    e
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SceneKind {
    Jolt,
    Rest,
    S16,
}

impl SceneKind {
    fn name(self) -> &'static str {
        match self {
            Self::Jolt => "J",
            Self::Rest => "R",
            Self::S16 => "S16",
        }
    }

    fn gap(self) -> f32 {
        match self {
            Self::Jolt => JOLT_SEPARATION,
            Self::Rest | Self::S16 => 0.0,
        }
    }

    fn friction(self) -> f32 {
        match self {
            Self::Jolt => JOLT_FRICTION,
            Self::Rest | Self::S16 => REST_FRICTION,
        }
    }
}

/// The floor and the scene's dynamic boxes, in the parity runner's spawn order.
fn spawn_scene(world: &mut EcsMaster, scene: SceneKind) -> Vec<Entity> {
    let friction = scene.friction();
    let gap = scene.gap();
    spawn_box(world, Vec3::new(0.0, -1.0, 0.0), friction, false);
    let mut boxes = Vec::new();
    match scene {
        SceneKind::Jolt | SceneKind::Rest => {
            for i in 0..PYRAMID_HEIGHT {
                let lo = i / 2;
                let hi = PYRAMID_HEIGHT - (i + 1) / 2;
                for j in lo..hi {
                    for k in lo..hi {
                        let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                        let position = Vec3::new(
                            -(PYRAMID_HEIGHT as f32) + BOX_SIZE * j as f32 + odd,
                            1.0 + (BOX_SIZE + gap) * i as f32,
                            -(PYRAMID_HEIGHT as f32) + BOX_SIZE * k as f32 + odd,
                        );
                        boxes.push(spawn_box(world, position, friction, true));
                    }
                }
            }
        }
        SceneKind::S16 => {
            for i in 0..TOWER_BODIES {
                let position = Vec3::new(0.0, 1.0 + (BOX_SIZE + gap) * i as f32, 0.0);
                boxes.push(spawn_box(world, position, friction, true));
            }
        }
    }
    boxes
}

/// One world on the real colored schedule with the probe wired between the broadphase and the
/// narrowphase.
struct Rig {
    world: EcsMaster,
    physics: Schedule,
    boxes: Vec<Entity>,
}

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// Wires the colored schedule with the probe, `kind` as the broadphase, sleeping off; the Tree
/// is forced onto its tree path.
fn wire(world: &mut EcsMaster, kind: BroadphaseKind) -> Schedule {
    wire_sleeping(world, kind, false)
}

/// [`wire`] with sleeping `sleeping` (its default sleep-skip mode, `Sets`).
fn wire_sleeping(world: &mut EcsMaster, kind: BroadphaseKind, sleeping: bool) -> Schedule {
    let mut builder = ScheduleBuilder::new(serial_pool());
    let keys = add_physics_colored_solve(&mut builder, world);
    world.insert_resource(Probe::default());
    {
        let probe = builder.add_system(probe_pairs);
        // `PhysicsStageKeys` carries each stage's `SystemKey` inner index; the type is not
        // nameable here but its field is public (the parity runner's canary does the same).
        let mut after = probe.key();
        after.0 = keys.broadphase;
        let mut before = probe.key();
        before.0 = keys.narrowphase;
        probe.after(after).before(before);
    }
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
        cfg.dt = DT;
        cfg.sleeping = sleeping;
        cfg.broadphase = kind;
    }
    world.resource_mut::<BroadphaseTree>().set_brute_max_rows(0);
    builder.build(world)
}

fn rig(scene: SceneKind, kind: BroadphaseKind) -> Rig {
    rig_sleeping(scene, kind, false)
}

fn rig_sleeping(scene: SceneKind, kind: BroadphaseKind, sleeping: bool) -> Rig {
    rig_kernel(scene, kind, sleeping, QueryKernel::default())
}

/// [`rig_sleeping`] with the Tree's query kernel `kernel` (G-F3-8).
fn rig_kernel(scene: SceneKind, kind: BroadphaseKind, sleeping: bool, kernel: QueryKernel) -> Rig {
    let mut world = EcsMaster::new();
    let boxes = spawn_scene(&mut world, scene);
    let physics = wire_sleeping(&mut world, kind, sleeping);
    world.resource_mut::<BroadphaseTree>().set_query_kernel(kernel);
    Rig { world, physics, boxes }
}

/// G-F3-8's receipt on a Tree rig after `tree_steps` tree-path steps under `kernel`: the kd order
/// ran on every one of them under `LeafListKd`, on none otherwise.
fn assert_kd_receipt(label: &str, d: &TreeDiag, kernel: QueryKernel, tree_steps: u64) {
    let want = if kernel == QueryKernel::LeafListKd { tree_steps } else { 0 };
    assert_eq!(d.kd_order_builds, want, "{label} ({kernel:?}): kd builds over {tree_steps} tree-path steps");
}

/// The label of a rig of `scene` under `kernel`: the scene's name, suffixed on the kd kernel.
fn kernel_label(scene: &str, kernel: QueryKernel) -> String {
    if kernel == QueryKernel::LeafListKd { format!("{scene}-kd") } else { scene.to_owned() }
}

impl Rig {
    fn step(&mut self) {
        self.physics.run(&mut self.world);
    }

    fn diag(&self) -> TreeDiag {
        self.world.resource::<BroadphaseTree>().diag()
    }

    fn probe(&self) -> (u64, u64, Option<u64>) {
        let p = self.world.resource::<Probe>();
        (p.steps, p.mismatches, p.first_mismatch)
    }

    fn pairs(&self) -> usize {
        self.world.resource::<ContactPairs>().pairs().len()
    }

    /// Every dynamic body's `RigidBody` as bits, in spawn order.
    fn pose_bits(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.boxes.len() * 13);
        for &e in &self.boxes {
            let b = self.world.get_component::<RigidBody>(e).expect("invariant: a spawned body is live");
            for f in [
                b.position.x,
                b.position.y,
                b.position.z,
                b.linear_velocity.x,
                b.linear_velocity.y,
                b.linear_velocity.z,
                b.rotation.x,
                b.rotation.y,
                b.rotation.z,
                b.rotation.w,
                b.angular_velocity.x,
                b.angular_velocity.y,
                b.angular_velocity.z,
            ] {
                out.push(f.to_bits());
            }
        }
        out
    }

    /// The pair stream as `u32`s: `a, b` per pair, in emit order.
    fn pair_words(&self) -> Vec<u32> {
        self.world.resource::<ContactPairs>().pairs().iter().flat_map(|(a, b)| [a.0, b.0]).collect()
    }
}

/// FNV-1a over `words`, folded into `hash` (the witness the report quotes: one value over every
/// step of a run, so two worlds print the same value only if they agreed on every step).
fn fnv_fold(mut hash: u64, words: &[u32]) -> u64 {
    for w in words {
        for byte in w.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// The running witness hashes of one world: every step's pose bits and pair stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Witness {
    poses: u64,
    pairs: u64,
}

impl Witness {
    const START: Self = Self { poses: FNV_OFFSET, pairs: FNV_OFFSET };

    fn fold(&mut self, rig: &Rig) {
        self.poses = fnv_fold(self.poses, &rig.pose_bits());
        self.pairs = fnv_fold(self.pairs, &rig.pair_words());
    }
}

/// Prints and asserts the two worlds' witnesses over a run.
fn report_witness(scene: &str, steps: usize, tree: Witness, all: Witness) {
    println!(
        "{scene}: {steps} steps; pose hash Tree {:#018x} AllPairs {:#018x}; pair-stream hash Tree {:#018x} AllPairs {:#018x}",
        tree.poses, all.poses, tree.pairs, all.pairs
    );
    assert_eq!(tree, all, "{scene}: the running witnesses of the two worlds differ");
}

/// Runs `scene` on the Tree (query kernel `kernel`) and on AllPairs in lockstep for `steps`,
/// asserting the oracle and the pose bytes on every step; returns the Tree rig for the structural
/// assertions.
fn lockstep(scene: SceneKind, steps: usize, kernel: QueryKernel) -> Rig {
    let mut tree = rig_kernel(scene, BroadphaseKind::Tree, false, kernel);
    let mut all = rig(scene, BroadphaseKind::AllPairs);
    let (mut wt, mut wa) = (Witness::START, Witness::START);
    for step in 0..steps {
        tree.step();
        all.step();
        let (p_steps, mism, first) = tree.probe();
        assert_eq!(p_steps, step as u64 + 1, "{}: the probe runs once per step", scene.name());
        assert_eq!(mism, 0, "{}: the Tree's pairs differ from all-pairs' first at step {first:?}", scene.name());
        assert_eq!(tree.pairs(), all.pairs(), "{} step {step}: pair counts", scene.name());
        assert!(
            tree.pose_bits() == all.pose_bits(),
            "{} step {step}: the Tree's pose bytes differ from the AllPairs world's",
            scene.name()
        );
        wt.fold(&tree);
        wa.fold(&all);
    }
    assert!(tree.pairs() > 0, "{}: anti-vacuity: the scene has pairs", scene.name());
    let label = kernel_label(scene.name(), kernel);
    assert_kd_receipt(&label, &tree.diag(), kernel, steps as u64);
    report_witness(&label, steps, wt, wa);
    tree
}

/// The pyramid bounds of the G2 table, sleeping off, the Tree on query kernel `kernel`.
fn assert_pyramid_bounds(scene: SceneKind, steps: usize, kernel: QueryKernel) {
    let mut tree = rig_kernel(scene, BroadphaseKind::Tree, false, kernel);
    let mut all = rig(scene, BroadphaseKind::AllPairs);
    let (mut wt, mut wa) = (Witness::START, Witness::START);
    for step in 0..steps {
        tree.step();
        all.step();
        let d = tree.diag();
        let (_, mism, first) = tree.probe();
        assert_eq!(mism, 0, "{} step {step}: the oracle differs (first at {first:?})", scene.name());
        assert!(tree.pose_bits() == all.pose_bits(), "{} step {step}: pose bytes", scene.name());
        wt.fold(&tree);
        wa.fold(&all);
        let admitted = u64::from(step >= 2);
        assert_eq!(d.static_rebuilds, admitted, "{} step {step}: the floor is admitted at step 2", scene.name());
        assert_eq!(d.members, admitted, "{} step {step}: members", scene.name());
        assert_eq!(d.sleeper_rebuilds, 0);
        assert_eq!(d.hint_candidates, 0);
        assert_eq!(d.evictions, 0, "{} step {step}", scene.name());
        assert_eq!(d.translations, 0, "{} step {step}: a first gather with no member translates nothing", scene.name());
        assert_eq!(d.wide_rows, 0);
        assert_eq!(d.excluded_rows, 0);
        assert_eq!(d.locator_resets, 0);
    }
    let rows = tree.world.resource::<SolverScratch>().bodies_len();
    assert_eq!(rows, tree.boxes.len() + 1, "{}: the floor and every box are rows", scene.name());
    assert!(tree.pairs() > tree.boxes.len(), "{}: anti-vacuity: more pairs than boxes", scene.name());
    let label = kernel_label(scene.name(), kernel);
    assert_kd_receipt(&label, &tree.diag(), kernel, steps as u64);
    report_witness(&label, steps, wt, wa);
}

#[test]
fn g2_jolt_pyramid_matches_all_pairs_with_one_static_rebuild() {
    assert_pyramid_bounds(SceneKind::Jolt, PYRAMID_STEPS, QueryKernel::default());
}

#[test]
fn g2_rest_pyramid_matches_all_pairs_with_one_static_rebuild() {
    assert_pyramid_bounds(SceneKind::Rest, PYRAMID_STEPS, QueryKernel::default());
}

/// One sleeping-on row of G2 (L10 T7): its step count, its bound on A — the first step with
/// every dynamic row held — and its pinned total `sleeper_rebuilds` (deterministic: the
/// admissions and compactions of a scene with no row change).
struct SleepingRow {
    steps: usize,
    a_max: usize,
    sleeper_rebuilds: u64,
}

/// The release rows are the parity runner's J-Son and R-S piles (Jolt's 15 layers), with T7's
/// bound A ≤ 300 (measured: A = 265 and 249). Debug's 10-layer piles are other scenes: its rest
/// pile first holds every box at step 547 (measured), so its row bounds A at 600 and runs to
/// 700; the Jolt pile holds by step 183 and keeps the design's bound.
#[cfg(not(debug_assertions))]
const J_SON_ROW: SleepingRow = SleepingRow { steps: 600, a_max: 300, sleeper_rebuilds: 1 };
#[cfg(debug_assertions)]
const J_SON_ROW: SleepingRow = SleepingRow { steps: 360, a_max: 300, sleeper_rebuilds: 1 };
/// See [`J_SON_ROW`].
#[cfg(not(debug_assertions))]
const R_S_ROW: SleepingRow = SleepingRow { steps: 600, a_max: 300, sleeper_rebuilds: 1 };
#[cfg(debug_assertions)]
const R_S_ROW: SleepingRow = SleepingRow { steps: 700, a_max: 600, sleeper_rebuilds: 1 };

/// Steps of the toggle-off script after the run.
const TOGGLE_OFF_STEPS: usize = 6;

/// The sleeping-on rows of G2 (design C5 as L10 C3c builds it; L10 T7), the Tree on query kernel
/// `kernel`: see the module docs.
fn assert_sleeping_pyramid(scene: SceneKind, row: &SleepingRow, kernel: QueryKernel) {
    use boyko_physics::sleep_sets::SleepSets;
    let mut tree = rig_kernel(scene, BroadphaseKind::Tree, true, kernel);
    let mut all = rig_sleeping(scene, BroadphaseKind::AllPairs, true);
    let label = kernel_label(&format!("{}-sleeping", scene.name()), kernel);
    let dynamic = tree.boxes.len() as u32;
    let (mut wt, mut wa) = (Witness::START, Witness::START);
    let mut first_all_held: Option<(usize, TreeDiag)> = None;
    let mut withheld_steps = 0usize;
    for step in 0..row.steps {
        tree.step();
        all.step();
        let d = tree.diag();
        let (p_steps, mism, first) = tree.probe();
        assert_eq!(p_steps, step as u64 + 1, "{label}: the probe runs once per step");
        assert_eq!(mism, 0, "{label} step {step}: the logical pairs differ from all-pairs' (first at {first:?})");
        assert!(tree.pose_bits() == all.pose_bits(), "{label} step {step}: pose bytes differ from the AllPairs twin's");
        wt.fold(&tree);
        wa.fold(&all);
        assert_eq!(d.static_rebuilds, u64::from(step >= 2), "{label} step {step}: the floor is admitted at step 2, once");
        assert_eq!(
            (d.translations, d.wide_rows, d.excluded_rows, d.locator_resets),
            (0, 0, 0, 0),
            "{label} step {step}: no row change, no Wide or Excluded row, no Reset"
        );
        let stats = tree.world.resource::<SleepSets>().stats();
        withheld_steps += usize::from(stats.withheld_pairs > 0);
        if first_all_held.is_none() && stats.held_rows == dynamic {
            first_all_held = Some((step, d));
        }
    }
    let (a, at_a) = first_all_held.unwrap_or_else(|| panic!("{label}: void: no step held every dynamic row"));
    assert!(a <= row.a_max, "{label}: A = {a}: the pile was not all held by step {}", row.a_max);
    let end = tree.diag();
    assert!(end.sleeper_rebuilds - at_a.sleeper_rebuilds <= 1, "{label}: {} sleeper rebuilds after A", end.sleeper_rebuilds - at_a.sleeper_rebuilds);
    assert_eq!(end.evictions, at_a.evictions, "{label}: an eviction after A");
    let sleepers = tree.world.resource::<BroadphaseTree>().sleeper_members();
    let stats = tree.world.resource::<SleepSets>().stats();
    let logical = tree.pairs();
    assert_eq!(sleepers, u64::from(dynamic), "{label}: every box is a sleeper at the end");
    assert_eq!(stats.withheld_pairs as usize, logical, "{label}: every pair is withheld at the end (the stream is empty)");
    println!(
        "{label}: A = {a}; withheld on {withheld_steps} of {} steps; {logical} pairs withheld at the end; \
         tree at A {at_a:?}; at the end {end:?}",
        row.steps
    );
    assert_eq!(end.sleeper_rebuilds, row.sleeper_rebuilds, "{label}: the pinned sleeper rebuilds");

    // The toggle-off script (the tree design's ruling W2, as L10's T1 recasts it): sleeping off,
    // no row change.
    for rig in [&mut tree, &mut all] {
        rig.world.resource_mut::<PhysicsConfig>().sleeping = false;
    }
    let mut at_toggle = TreeDiag::default();
    for k in 0..TOGGLE_OFF_STEPS {
        tree.step();
        all.step();
        let (_, mism, first) = tree.probe();
        assert_eq!(mism, 0, "{label} toggle-off step {k}: the oracle differs (first at {first:?})");
        assert!(tree.pose_bits() == all.pose_bits(), "{label} toggle-off step {k}: pose bytes");
        wt.fold(&tree);
        wa.fold(&all);
        let d = tree.diag();
        let sleepers = tree.world.resource::<BroadphaseTree>().sleeper_members();
        assert_eq!(sleepers, 0, "{label} toggle-off step {k}: the sleeper set dissolves with sleeping off");
        assert_eq!(d.members, 1, "{label} toggle-off step {k}: the floor stays in S");
        if k == 0 {
            at_toggle = d;
        }
    }
    let d = tree.diag();
    assert_eq!(
        (
            d.hint_candidates - at_toggle.hint_candidates,
            d.sleeper_rebuilds - at_toggle.sleeper_rebuilds,
            d.evictions - at_toggle.evictions
        ),
        (0, 0, 0),
        "{label}: with sleeping off, Δcandidates, Δsleeper rebuilds, Δevictions after the toggle step"
    );
    assert_kd_receipt(&label, &d, kernel, (row.steps + TOGGLE_OFF_STEPS) as u64);
    report_witness(&label, row.steps + TOGGLE_OFF_STEPS, wt, wa);
}

#[test]
fn g2_jolt_pyramid_sleeping_on_withholds_the_held_pile() {
    assert_sleeping_pyramid(SceneKind::Jolt, &J_SON_ROW, QueryKernel::default());
}

#[test]
fn g2_rest_pyramid_sleeping_on_withholds_the_held_pile() {
    assert_sleeping_pyramid(SceneKind::Rest, &R_S_ROW, QueryKernel::default());
}

#[test]
fn g2_tower_matches_all_pairs_and_pose_bytes() {
    assert_tower(QueryKernel::default());
}

/// G2's tower row on query kernel `kernel`.
fn assert_tower(kernel: QueryKernel) {
    let tree = lockstep(SceneKind::S16, TOWER_STEPS, kernel);
    let d = tree.diag();
    assert_eq!(d.static_rebuilds, 1);
    assert_eq!(d.members, 1);
    assert_eq!(d.evictions, 0);
}

// ── G-F3-8: every Tree rig on the kd leaf order (module docs) ────────────────

#[test]
fn g_f3_8_jolt_pyramid_kd_matches_all_pairs() {
    assert_pyramid_bounds(SceneKind::Jolt, PYRAMID_STEPS, QueryKernel::LeafListKd);
}

#[test]
fn g_f3_8_rest_pyramid_kd_matches_all_pairs() {
    assert_pyramid_bounds(SceneKind::Rest, PYRAMID_STEPS, QueryKernel::LeafListKd);
}

#[test]
fn g_f3_8_tower_kd_matches_all_pairs() {
    assert_tower(QueryKernel::LeafListKd);
}

#[test]
fn g_f3_8_jolt_pyramid_sleeping_on_kd_matches_all_pairs() {
    assert_sleeping_pyramid(SceneKind::Jolt, &J_SON_ROW, QueryKernel::LeafListKd);
}

#[test]
fn g_f3_8_rest_pyramid_sleeping_on_kd_matches_all_pairs() {
    assert_sleeping_pyramid(SceneKind::Rest, &R_S_ROW, QueryKernel::LeafListKd);
}

#[test]
fn g_f3_8_churn_arms_kd_match_all_pairs() {
    assert_churn_arms(QueryKernel::LeafListKd);
}

// ── G-TH1: the brute threshold's band (module docs) ──────────────────────────

/// Rest piles of the band scene.
const BAND_PILES: usize = 10;
/// Layers of a band pile (9 + 4 + 1 boxes). Its boxes touch their neighbours, so a box has
/// several dynamic partners and the stream's order reaches the colouring.
const BAND_PILE_LAYERS: i32 = 3;
/// Boxes per band pile.
const BAND_PILE_BOXES: usize = 14;
/// Boxes the script spawns and despawns: 141 rows → 150 and back, across 144 ↔ 145.
const BAND_EXTRAS: usize = 9;
/// Steps between two row changes of the script.
const BAND_PERIOD: usize = 10;
/// Steps before the first climb; the sleeping-on arm's piles fall asleep in them.
const BAND_SETTLE: usize = 300;
/// Steps held at the top and at the bottom of a cycle.
const BAND_HOLD: usize = 30;
/// Climbs and descents.
const BAND_CYCLES: usize = 2;

/// The band scene on `kind`, sleeping `sleeping`, at the default brute threshold.
fn band_rig(kind: BroadphaseKind, sleeping: bool) -> Rig {
    let mut world = EcsMaster::new();
    spawn_box(&mut world, Vec3::new(0.0, -1.0, 0.0), REST_FRICTION, false);
    let mut boxes = Vec::with_capacity(BAND_PILES * BAND_PILE_BOXES + BAND_EXTRAS);
    for p in 0..BAND_PILES {
        let (ox, oz) = (-20.0 + 10.0 * (p % 5) as f32, -10.0 + 20.0 * (p / 5) as f32);
        for i in 0..BAND_PILE_LAYERS {
            let lo = i / 2;
            let hi = BAND_PILE_LAYERS - (i + 1) / 2;
            for j in lo..hi {
                for k in lo..hi {
                    let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                    let position = Vec3::new(
                        ox + BOX_SIZE * j as f32 + odd,
                        1.0 + BOX_SIZE * i as f32,
                        oz + BOX_SIZE * k as f32 + odd,
                    );
                    boxes.push(spawn_box(&mut world, position, REST_FRICTION, true));
                }
            }
        }
    }
    assert_eq!(boxes.len(), BAND_PILES * BAND_PILE_BOXES, "construction: ten piles of 14 boxes");
    let physics = wire_sleeping(&mut world, kind, sleeping);
    // G-TH1 is about the default threshold, which `wire_sleeping` sets to 0.
    world.resource_mut::<BroadphaseTree>().set_brute_max_rows(TREE_BRUTE_MAX_ROWS);
    Rig { world, physics, boxes }
}

/// The band script, one entry per step: `1` spawns the next extra box, `-1` despawns the last
/// one, `0` holds.
fn band_script() -> Vec<i8> {
    let mut script = vec![0i8; BAND_SETTLE];
    for _ in 0..BAND_CYCLES {
        for edit in [1i8, -1] {
            for _ in 0..BAND_EXTRAS {
                script.push(edit);
                script.extend(std::iter::repeat_n(0, BAND_PERIOD - 1));
            }
            script.extend(std::iter::repeat_n(0, BAND_HOLD));
        }
    }
    script
}

/// Applies one band-script entry to `rig`: an extra box resting on the floor clear of the piles
/// and of each other, or the despawn of the last one.
fn band_edit(rig: &mut Rig, edit: i8) {
    match edit {
        1 => {
            let k = rig.boxes.len() - BAND_PILES * BAND_PILE_BOXES;
            let position = Vec3::new(-20.0 + 5.0 * k as f32, 1.0, 30.0);
            let e = spawn_box(&mut rig.world, position, REST_FRICTION, true);
            rig.boxes.push(e);
        }
        -1 => {
            let e = rig.boxes.pop().expect("script: an extra box to despawn");
            assert!(rig.world.delete_entity(e), "script: the extra box is live");
        }
        _ => {}
    }
}

/// G-TH1 on the band scene, sleeping `sleeping` (module docs).
fn assert_threshold_band(sleeping: bool) {
    let label = if sleeping { "band, sleeping on" } else { "band, sleeping off" };
    let mut tree = band_rig(BroadphaseKind::Tree, sleeping);
    let mut all = band_rig(BroadphaseKind::AllPairs, sleeping);
    let threshold = tree.world.resource::<BroadphaseTree>().brute_max_rows() as usize;
    let script = band_script();
    let (mut wt, mut wa) = (Witness::START, Witness::START);
    let mut prev = tree.diag();
    let (mut prev_rows, mut prev_sleepers) = (0usize, 0u64);
    let (mut brute_steps, mut tree_steps, mut ups, mut downs, mut downs_held) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut first_sleeper = None;
    for (step, &edit) in script.iter().enumerate() {
        band_edit(&mut tree, edit);
        band_edit(&mut all, edit);
        tree.step();
        all.step();
        let (p_steps, mism, first) = tree.probe();
        assert_eq!(p_steps, step as u64 + 1, "{label}: the probe runs once per step");
        assert_eq!(mism, 0, "{label} step {step}: the Tree's pairs differ from all-pairs' (first at {first:?})");
        assert!(
            tree.pose_bits() == all.pose_bits(),
            "{label} step {step}: the Tree's pose bytes differ from the AllPairs twin's"
        );
        wt.fold(&tree);
        wa.fold(&all);

        let rows = tree.world.resource::<SolverScratch>().bodies_len();
        let d = tree.diag();
        let sleepers = tree.world.resource::<BroadphaseTree>().sleeper_members();
        // The Q rows and the sleepers are the dynamic boxes, never all empty: a tree-path step
        // either answers a leaf-list leaf or holds a sleeper; a brute step does neither (T6).
        let derived_tree = rows > threshold;
        let answered = d.leaf_list_leaves - prev.leaf_list_leaves;
        let took_tree = answered > 0 || sleepers > 0;
        assert_eq!(
            took_tree, derived_tree,
            "{label} step {step}: rows {rows} against brute_max_rows {threshold} derive the {} path, \
             but the step answered {answered} leaf-list leaves and holds {sleepers} sleepers",
            if derived_tree { "tree" } else { "brute" }
        );
        if derived_tree {
            tree_steps += 1;
        } else {
            brute_steps += 1;
            assert_eq!(
                TreeDiag { members: 0, ..d },
                TreeDiag { members: 0, ..prev },
                "{label} step {step}: the brute path moves no counter"
            );
        }
        ups += usize::from(prev_rows <= threshold && rows > threshold && step > 0);
        if prev_rows > threshold && rows <= threshold {
            downs += 1;
            downs_held += usize::from(prev_sleepers > 0);
        }
        if first_sleeper.is_none() && sleepers > 0 {
            first_sleeper = Some(step);
        }
        (prev, prev_rows, prev_sleepers) = (d, rows, sleepers);
    }
    assert!(
        brute_steps > 0,
        "{label}: void: the band scene ran no brute step (every step had rows > brute_max_rows = {threshold})"
    );
    assert!(
        tree_steps > 0,
        "{label}: void: the band scene ran no tree-path step (every step had rows <= brute_max_rows = {threshold})"
    );
    assert_eq!((ups, downs), (BAND_CYCLES, BAND_CYCLES), "{label}: the script crosses the threshold each way per cycle");
    if sleeping {
        assert!(
            downs_held > 0,
            "{label}: void: no descent crossed the threshold with a live sleeper set (first sleeper at step {first_sleeper:?})"
        );
    }
    println!(
        "{label}: {} steps, {brute_steps} brute, {tree_steps} tree-path; crossings up {ups}, down {downs} \
         ({downs_held} with a live sleeper set); first sleeper at step {first_sleeper:?}; tree {:?}",
        script.len(),
        tree.diag()
    );
    report_witness(label, script.len(), wt, wa);
}

/// G-TH1, sleeping off (module docs).
#[test]
fn tree_threshold_band_is_value_neutral() {
    assert_threshold_band(false);
}

/// G-TH1, sleeping on: the brute steps dissolve the sleeper set the tree-path steps built (T6).
#[test]
fn tree_threshold_band_is_value_neutral_sleeping_on() {
    assert_threshold_band(true);
}

// ── The churn scene (`benches/row_identity_churn.rs`, transcribed) ───────────

const MARKED_SPHERES: usize = 64;
const PLAIN_SPHERES: usize = 32;
const BURST: usize = 32;
const SPHERE_GRID: usize = 10;
const SPHERE_PITCH: f32 = 2.0;
const SPHERE_ORIGIN: f32 = 24.0;
const SPHERE_RADIUS: f32 = 0.5;

/// A data component whose presence puts a body in the MARKED archetype.
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct Marker {
    _tag: u32,
}

/// The `Marker` inserts and removes the next `marker_ops` run applies through `Commands`.
#[derive(Resource, Default)]
struct MarkerPlan {
    insert: Vec<Entity>,
    remove: Vec<Entity>,
}

#[allow(clippy::needless_pass_by_value)]
fn apply_marker_plan(mut commands: Commands, plan: Res<MarkerPlan>) {
    for &entity in &plan.insert {
        commands.entity(entity).insert(Marker { _tag: 0 });
    }
    for &entity in &plan.remove {
        commands.entity(entity).remove::<Marker>();
    }
}

fn slot_pose(slot: usize) -> Vec3 {
    Vec3::new(
        SPHERE_ORIGIN + SPHERE_PITCH * (slot % SPHERE_GRID) as f32,
        SPHERE_RADIUS,
        SPHERE_ORIGIN + SPHERE_PITCH * (slot / SPHERE_GRID) as f32,
    )
}

fn slot_of(position: Vec3) -> usize {
    let col = ((position.x - SPHERE_ORIGIN) / SPHERE_PITCH).round() as usize;
    let row = ((position.z - SPHERE_ORIGIN) / SPHERE_PITCH).round() as usize;
    row * SPHERE_GRID + col
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arm {
    Stable,
    SwapChurn,
    ArchetypeShift,
    FirstArchetypeSpawn,
    BurstDespawn,
    BurstMigrate,
}

impl Arm {
    const ALL: [Self; 6] = [
        Self::Stable,
        Self::SwapChurn,
        Self::ArchetypeShift,
        Self::FirstArchetypeSpawn,
        Self::BurstDespawn,
        Self::BurstMigrate,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::SwapChurn => "swap_churn",
            Self::ArchetypeShift => "archetype_shift",
            Self::FirstArchetypeSpawn => "first_archetype_spawn",
            Self::BurstDespawn => "burst_despawn",
            Self::BurstMigrate => "burst_migrate",
        }
    }
}

#[derive(Clone, Copy)]
enum Target {
    Marked,
    Plain,
}

struct Churn {
    world: EcsMaster,
    physics: Schedule,
    marker_ops: Schedule,
    marked: ArchetypeId,
    plain: ArchetypeId,
    slots: Vec<Entity>,
    cursor: usize,
    step: u64,
}

impl Churn {
    fn new(kernel: QueryKernel) -> Self {
        let mut world = EcsMaster::new();
        let marked = world.create_archetype(&[
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
            Marker::component_id(),
        ]);
        let plain = world.create_archetype(&[
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
        ]);
        let physics = wire(&mut world, BroadphaseKind::Tree);
        world.resource_mut::<BroadphaseTree>().set_query_kernel(kernel);
        world.insert_resource(MarkerPlan::default());
        let mut ops = ScheduleBuilder::new(serial_pool());
        ops.add_system(apply_marker_plan);
        let marker_ops = ops.build(&mut world);

        let mut churn = Self {
            world,
            physics,
            marker_ops,
            marked,
            plain,
            slots: Vec::with_capacity(MARKED_SPHERES + PLAIN_SPHERES),
            cursor: 0,
            step: 0,
        };
        for slot in 0..MARKED_SPHERES {
            let sphere = churn.spawn_sphere(slot, Target::Marked);
            churn.slots.push(sphere);
        }
        churn.spawn_floor_and_pyramid();
        for slot in MARKED_SPHERES..MARKED_SPHERES + PLAIN_SPHERES {
            let sphere = churn.spawn_sphere(slot, Target::Plain);
            churn.slots.push(sphere);
        }
        churn
    }

    fn spawn(&mut self, target: Target, body: RigidBody, mass: RigidBodyMass, collider: Collider, simulated: bool) -> Entity {
        let marker = Marker { _tag: 0 };
        let created = match target {
            Target::Marked => self.world.create_entity(
                self.marked,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                    (Marker::component_id(), as_bytes(&marker)),
                ],
            ),
            Target::Plain => self.world.create_entity(
                self.plain,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                ],
            ),
        };
        let entity = created.expect("construction: the body archetype accepts every column");
        if simulated {
            self.world.enable::<Simulated>(entity);
        }
        entity
    }

    fn spawn_sphere(&mut self, slot: usize, target: Target) -> Entity {
        self.spawn(
            target,
            RigidBody { position: slot_pose(slot), linear_velocity: Vec3::ZERO, rotation: Quat::IDENTITY, angular_velocity: Vec3::ZERO },
            RigidBodyMass { inv_inertia: Mat3::IDENTITY, inv_mass: 1.0, restitution: 0.0, friction: 0.5 },
            Collider { shape: ColliderShape::Sphere { radius: SPHERE_RADIUS }, layer: 1, mask: 1 },
            true,
        )
    }

    fn spawn_floor_and_pyramid(&mut self) {
        self.spawn(
            Target::Plain,
            RigidBody { position: Vec3::new(0.0, -1.0, 0.0), linear_velocity: Vec3::ZERO, rotation: Quat::IDENTITY, angular_velocity: Vec3::ZERO },
            RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass: 0.0, restitution: 0.0, friction: 0.5 },
            Collider { shape: ColliderShape::Box { half_extents: FLOOR_HALF_EXTENTS }, layer: 1, mask: 1 },
            false,
        );
        for i in 0..PYRAMID_HEIGHT {
            let lo = i / 2;
            let hi = PYRAMID_HEIGHT - (i + 1) / 2;
            for j in lo..hi {
                for k in lo..hi {
                    let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                    let position = Vec3::new(
                        -(PYRAMID_HEIGHT as f32) + BOX_SIZE * j as f32 + odd,
                        1.0 + (BOX_SIZE + JOLT_SEPARATION) * i as f32,
                        -(PYRAMID_HEIGHT as f32) + BOX_SIZE * k as f32 + odd,
                    );
                    self.spawn(
                        Target::Plain,
                        RigidBody { position, linear_velocity: Vec3::ZERO, rotation: Quat::IDENTITY, angular_velocity: Vec3::ZERO },
                        RigidBodyMass {
                            inv_inertia: Mat3::from_diagonal(Vec3::new(BOX_INV_INERTIA, BOX_INV_INERTIA, BOX_INV_INERTIA)),
                            inv_mass: BOX_INV_MASS,
                            restitution: 0.0,
                            friction: 0.5,
                        },
                        Collider { shape: ColliderShape::Box { half_extents: Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX) }, layer: 1, mask: 1 },
                        true,
                    );
                }
            }
        }
    }

    fn respawn(&mut self, slot: usize, target: Target) {
        let old = self.slots[slot];
        assert!(self.world.delete_entity(old), "construction: a churn sphere is live");
        self.slots[slot] = self.spawn_sphere(slot, target);
    }

    fn migrate(&mut self, insert: &[Entity], remove: &[Entity]) {
        {
            let plan = self.world.resource_mut::<MarkerPlan>();
            plan.insert.clear();
            plan.insert.extend_from_slice(insert);
            plan.remove.clear();
            plan.remove.extend_from_slice(remove);
        }
        self.marker_ops.run(&mut self.world);
    }

    /// Applies one step's structural change for `arm`, as the bench does.
    fn churn_step(&mut self, arm: Arm) {
        let even = self.step.is_multiple_of(2);
        self.step += 1;
        match arm {
            Arm::Stable => {}
            Arm::SwapChurn => {
                let slot = MARKED_SPHERES + self.cursor;
                self.respawn(slot, Target::Plain);
                self.cursor = (self.cursor + 1) % PLAIN_SPHERES;
            }
            Arm::ArchetypeShift => {
                let sphere = self.slots[0];
                if even {
                    self.migrate(&[], &[sphere]);
                } else {
                    self.migrate(&[sphere], &[]);
                }
            }
            Arm::FirstArchetypeSpawn => {
                let slot = MARKED_SPHERES + self.cursor;
                if even {
                    self.respawn(slot, Target::Marked);
                } else {
                    self.respawn(slot, Target::Plain);
                    self.cursor = (self.cursor + 1) % PLAIN_SPHERES;
                }
            }
            Arm::BurstDespawn => {
                let front: Vec<usize> = {
                    let walk = self.world.query::<(&RigidBody, &Marker), ()>();
                    walk.iter().take(BURST).map(|(body, _)| slot_of(body.position)).collect()
                };
                for &slot in &front {
                    assert!(self.world.delete_entity(self.slots[slot]), "construction: a churn sphere is live");
                }
                for &slot in &front {
                    self.slots[slot] = self.spawn_sphere(slot, Target::Marked);
                }
            }
            Arm::BurstMigrate => {
                let movers: Vec<Entity> = self.slots[MARKED_SPHERES..].to_vec();
                assert_eq!(movers.len(), BURST);
                if even {
                    self.migrate(&movers, &[]);
                } else {
                    self.migrate(&[], &movers);
                }
            }
        }
    }

    fn diag(&self) -> TreeDiag {
        self.world.resource::<BroadphaseTree>().diag()
    }

    fn probe(&self) -> (u64, u64, Option<u64>) {
        let p = self.world.resource::<Probe>();
        (p.steps, p.mismatches, p.first_mismatch)
    }

    fn maps_built(&self) -> u64 {
        self.world.resource::<SolverScratch>().row_remap_builds()
    }

    /// Folds this step's Tree pair stream and the probe's oracle stream into the two witnesses.
    fn fold_streams(&self, tree: &mut u64, oracle: &mut u64) {
        let words: Vec<u32> =
            self.world.resource::<ContactPairs>().pairs().iter().flat_map(|(a, b)| [a.0, b.0]).collect();
        *tree = fnv_fold(*tree, &words);
        let words: Vec<u32> =
            self.world.resource::<Probe>().oracle.pairs().iter().flat_map(|(a, b)| [a.0, b.0]).collect();
        *oracle = fnv_fold(*oracle, &words);
    }
}

/// G2, the churn arms with sleeping off: the oracle on every step, and the static set is never
/// dissolved by a row change — translated on every churn step, rebuilt on none.
#[test]
fn g2_churn_arms_translate_the_static_set_and_never_rebuild_it() {
    assert_churn_arms(QueryKernel::default());
}

/// G2's churn arms, the Tree on query kernel `kernel`.
fn assert_churn_arms(kernel: QueryKernel) {
    for arm in Arm::ALL {
        let mut churn = Churn::new(kernel);
        for _ in 0..CHURN_SETTLE {
            churn.physics.run(&mut churn.world);
        }
        let settled = churn.diag();
        assert_eq!(settled.static_rebuilds, 1, "{}: the floor was admitted during the settle", arm.name());
        assert_eq!(settled.members, 1, "{}: the floor is the one static", arm.name());
        let maps0 = churn.maps_built();
        let (mut w_tree, mut w_oracle) = (FNV_OFFSET, FNV_OFFSET);

        for k in 0..CHURN_STEPS {
            churn.churn_step(arm);
            churn.physics.run(&mut churn.world);
            churn.fold_streams(&mut w_tree, &mut w_oracle);
            let d = churn.diag();
            assert_eq!(d.members, 1, "{} churn step {k}: the floor stays a member", arm.name());
        }
        let d = churn.diag();
        let (steps, mism, first) = churn.probe();
        assert_eq!(steps, CHURN_SETTLE as u64 + CHURN_STEPS, "{}: the probe ran on every step", arm.name());
        assert_eq!(mism, 0, "{}: the Tree's pairs differ from all-pairs' (first at step {first:?})", arm.name());
        let expected_maps = if arm == Arm::Stable { 0 } else { CHURN_STEPS };
        assert_eq!(
            churn.maps_built() - maps0,
            expected_maps,
            "{}: anti-vacuity: every churn step changes the rows (maps built)",
            arm.name()
        );
        assert_eq!(d.static_rebuilds - settled.static_rebuilds, 0, "{}: no rebuild on a row change", arm.name());
        assert_eq!(d.evictions - settled.evictions, 0, "{}: no eviction on a row change", arm.name());
        assert_eq!(
            d.translations - settled.translations,
            expected_maps,
            "{}: one translation per Rows step with a member",
            arm.name()
        );
        assert_eq!(d.locator_resets, 0, "{}: the cursor is stamped every step", arm.name());
        assert_eq!(d.sleeper_rebuilds, 0);
        assert_eq!(d.hint_candidates, 0);
        assert_eq!(w_tree, w_oracle, "{}: the churn window's pair-stream witnesses differ", arm.name());
        assert_kd_receipt(arm.name(), &d, kernel, CHURN_SETTLE as u64 + CHURN_STEPS);
        println!(
            "{}: translations {} patches {} evictions {} static_rebuilds {}; churn-window pair-stream hash Tree {:#018x} oracle {:#018x}",
            arm.name(),
            d.translations,
            d.patches,
            d.evictions,
            d.static_rebuilds,
            w_tree,
            w_oracle
        );
    }
}
