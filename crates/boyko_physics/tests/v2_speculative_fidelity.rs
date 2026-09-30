//! **V2's fidelity gate: Jolt's pyramid must stand after its landing.** The J-T scene (Jolt's
//! `PyramidScene`, height 15, 1240 boxes, 0.5 m layer gap, friction 0.2, restitution 0) run for 500
//! steps through the real schedule at one worker, eight trajectories per cell, in four cells:
//! contact reuse on and off, times sleeping off and on.
//!
//! # Why this gate exists
//!
//! The pose-hash gates compare runs with each other and with pinned bytes; none of them can see
//! whether the pile holds. On the pre-V2 contact rule it does not: a contact existed only on
//! overlap, the layers land at 3–12.5 m/s, most boxes of a layer land on one to three of their four
//! supports, get kicked sideways, and the outer ring slides off (F0 verdict: 0 of 32 trajectories
//! hold, the top box 475 mm off, 28.8°; Jolt 5.6 holds with 0 boxes past 0.5 m, 51 mm, 0.54°). V2
//! (owner V2a, V2b = 20 mm, 2026-09-30) makes a contact point exist while its separation is at most
//! `PhysicsConfig::speculative_distance`, and solves a positive separation as a speculative bias.
//!
//! # The trajectories
//!
//! The pile is chaotic, so one trajectory proves little. A trajectory is the scene with one box's
//! spawn `x` nudged by one ulp (`f32::next_up`), the F0 ensemble's seeds: none, and boxes 0, 200,
//! 400, 600, 800, 1000 and 1239 in spawn order.
//!
//! # The rule
//!
//! Per trajectory, each box's horizontal drift from its spawn lattice point at step 500 (the F0
//! `pile.py` metric). **A cell passes iff all eight trajectories have no box past 0.5 m and a max
//! drift under 0.5 m.** The gate passes iff every cell does. Every trajectory's row is printed
//! (boxes past 0.5 m and 0.1 m, max drift, the top box's offset, the max rotation, the pose hash)
//! before the verdict, pass or fail.
//!
//! # The anchors (the test builds J-T, not a look-alike)
//!
//! A cell's unperturbed trajectory ends in the pose hash of the parity runner's fixture for the
//! same configuration — FNV-1a 64 over every dynamic box's `RigidBody` as 13 little-endian `f32`,
//! in spawn order, the runner's `pose_bytes`. A mismatch means this file no longer builds the scene
//! the runner builds, which is a defect of the test, not a finding about the contact rule.
//!
//! # How to run it
//!
//! Release only in practice (32 runs of a 1240-box pile): the device-free `-- --ignored` leg,
//! `cargo test -p boyko-physics --release --locked --test v2_speculative_fidelity -- --ignored
//! --nocapture`. The four cells are four tests, so libtest runs them side by side.

#![cfg(not(miri))]

use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{BroadphaseKind, BroadphaseSelectMode, PhysicsConfig};

/// Jolt's `cBoxSize`: the pitch between neighbouring boxes in a layer.
const BOX_SIZE: f32 = 2.0;
/// Jolt's `cHalfBoxSize`.
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
/// Jolt's `cBoxSeparation`, the J-T gap.
const GAP: f32 = 0.5;
/// Jolt's pyramid height.
const PYRAMID_HEIGHT: i32 = 15;
/// Dynamic boxes in the pyramid.
const PYRAMID_BODIES: usize = 1240;
/// Jolt's floor half-extents.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
/// Jolt's `PyramidScene` friction.
const FRICTION: f32 = 0.2;
/// Jolt's `cDeltaTime`.
const DT: f32 = 1.0 / 60.0;
/// Unit-density cube of half-extent 1: m = 8.
const BOX_INV_MASS: f32 = 0.125;
/// `8 / 12 · (2² + 2²)` per axis, inverted.
const BOX_INV_INERTIA: f32 = 0.1875;
/// Steps per trajectory: the runner's J-T row.
const STEPS: usize = 500;
/// The largest horizontal drift a standing pile may show, in metres.
const DRIFT_LIMIT_M: f64 = 0.5;

/// The F0 ensemble's seeds: `None` is the unperturbed scene; `Some(k)` nudges box `k`'s spawn `x`
/// by one ulp.
const SEEDS: [Option<usize>; 8] =
    [None, Some(0), Some(200), Some(400), Some(600), Some(800), Some(1000), Some(1239)];

/// The parity runner's J-T fixtures (`docs/measurements/2026-09-27-physics-window8/gate/fixtures/`
/// and window 8b's), by the configuration they were recorded with: `JT500` (`--cfg default
/// --broadphase tree --sleeping off`).
const ANCHOR_REUSE_ON_SLEEP_OFF: u64 = 0x30c5_438b_c6ad_9ffa;
/// `JToff500`: `JT500` with `--contact-reuse off`.
const ANCHOR_REUSE_OFF_SLEEP_OFF: u64 = 0x32d5_e235_342b_4143;
/// `JSonT500`: J-T with sleeping on (recorded as `--cfg a --broadphase tree --sleeping on`, which
/// the engine's scalar/SIMD and worker-count identities make the default configuration's bits;
/// this file's run on the parent `16191fda` is the measurement that they are).
const ANCHOR_REUSE_ON_SLEEP_ON: u64 = 0x3db4_7fae_414b_655c;
/// Window 6's `Son-J1000` (`--cfg a --sleeping --contact-reuse off`, 1000 steps; L10's pose gate
/// `G4f_off_Son-J1000`): the pile is frozen before step 500, so the 500-step pose is the 1000-step
/// one (read on the parent `16191fda` by this file's own run).
const ANCHOR_REUSE_OFF_SLEEP_ON: u64 = 0xcc2a_5400_c66e_ecce;

/// FNV-1a 64 offset basis (the runner's).
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64 prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// One cell of the gate.
#[derive(Clone, Copy, Debug)]
struct Cell {
    /// Its name in the report.
    name: &'static str,
    /// `PhysicsConfig::contact_reuse`.
    reuse: bool,
    /// `PhysicsConfig::sleeping`.
    sleeping: bool,
    /// The runner fixture the unperturbed trajectory must end in, when one exists.
    anchor: Option<u64>,
}

/// What one trajectory ended in.
#[derive(Clone, Copy, Debug)]
struct Trajectory {
    seed: Option<usize>,
    pose_hash: u64,
    past_half_m: usize,
    past_tenth_m: usize,
    max_drift_m: f64,
    top_offset_m: f64,
    max_rotation_deg: f64,
}

impl Trajectory {
    /// Whether this trajectory holds: no box past the limit, and the max drift under it.
    fn holds(&self) -> bool {
        self.past_half_m == 0 && self.max_drift_m < DRIFT_LIMIT_M
    }

    /// The report row.
    fn row(&self) -> String {
        let seed = self.seed.map_or_else(|| "none".to_owned(), |k| k.to_string());
        format!(
            "seed {seed:>4}: >0.5m {:3}  >0.1m {:4}  max {:9.1} mm  top {:8.1} mm  rot {:7.2} deg  \
             hash {:#018x}  {}",
            self.past_half_m,
            self.past_tenth_m,
            self.max_drift_m * 1e3,
            self.top_offset_m * 1e3,
            self.max_rotation_deg,
            self.pose_hash,
            if self.holds() { "holds" } else { "FAILS" }
        )
    }
}

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its `size_of::<T>()` bytes
    // read-only for the duration of the borrow, which is the exact layout the component pool
    // stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Spawns one box at rest into the `RigidBodyBundle` archetype, the runner's `spawn_box`: a
/// dynamic one is enabled as `Simulated`; the floor is not, which is how a static body is expressed.
fn spawn_box(world: &mut EcsMaster, position: Vec3, dynamic: bool) -> Entity {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let (mass, half_extents) = if dynamic {
        (
            RigidBodyMass {
                inv_inertia: Mat3::from_diagonal(Vec3::new(
                    BOX_INV_INERTIA,
                    BOX_INV_INERTIA,
                    BOX_INV_INERTIA,
                )),
                inv_mass: BOX_INV_MASS,
                restitution: 0.0,
                friction: FRICTION,
            },
            Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
        )
    } else {
        (
            RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass: 0.0, restitution: 0.0, friction: FRICTION },
            FLOOR_HALF_EXTENTS,
        )
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
        .expect("invariant: the RigidBodyBundle archetype accepts the three columns");
    if dynamic {
        world.enable::<Simulated>(e);
    }
    e
}

/// Every box's spawn lattice point, in spawn order: Jolt's placement loop, index for index.
fn lattice() -> Vec<Vec3> {
    let mut out = Vec::with_capacity(PYRAMID_BODIES);
    for i in 0..PYRAMID_HEIGHT {
        let lo = i / 2;
        let hi = PYRAMID_HEIGHT - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                out.push(Vec3::new(
                    -(PYRAMID_HEIGHT as f32) + BOX_SIZE * j as f32 + odd,
                    1.0 + (BOX_SIZE + GAP) * i as f32,
                    -(PYRAMID_HEIGHT as f32) + BOX_SIZE * k as f32 + odd,
                ));
            }
        }
    }
    out
}

/// The floor, then the pyramid in spawn order, box `seed`'s `x` nudged by one ulp. Returns the
/// dynamic boxes in spawn order.
fn spawn_scene(world: &mut EcsMaster, lattice: &[Vec3], seed: Option<usize>) -> Vec<Entity> {
    spawn_box(world, Vec3::new(0.0, -1.0, 0.0), false);
    lattice
        .iter()
        .enumerate()
        .map(|(k, &p)| {
            let mut position = p;
            if seed == Some(k) {
                position.x = position.x.next_up();
            }
            spawn_box(world, position, true)
        })
        .collect()
}

/// FNV-1a 64 over every dynamic box's `RigidBody` as 13 little-endian `f32`, in spawn order: the
/// runner's `pose_bytes` hash.
fn pose_hash(world: &EcsMaster, boxes: &[Entity]) -> u64 {
    let mut h = FNV_OFFSET;
    for &e in boxes {
        let b = world.get_component::<RigidBody>(e).expect("invariant: a spawned box is live");
        let fields = [
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
        ];
        for f in fields {
            for byte in f.to_bits().to_le_bytes() {
                h = (h ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
            }
        }
    }
    h
}

/// Runs one trajectory of `cell` to step 500 at one worker and measures the pile.
fn run(cell: Cell, lattice: &[Vec3], seed: Option<usize>) -> Trajectory {
    let mut world = EcsMaster::new();
    let boxes = spawn_scene(&mut world, lattice, seed);
    assert_eq!(boxes.len(), PYRAMID_BODIES, "construction: J-T has {PYRAMID_BODIES} dynamic boxes");
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
    let keys = add_physics_colored_solve(&mut builder, &mut world);
    assert!(keys.build_graph.is_some(), "construction: the colored pipeline builds the graph");
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    {
        // The parity runner's `configure` for `--cfg default --broadphase tree --sleeping off|on`
        // (plus `--contact-reuse off` for a reuse-off cell).
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
        cfg.dt = DT;
        cfg.sleeping = cell.sleeping;
        cfg.contact_reuse = cell.reuse;
        cfg.broadphase_select = BroadphaseSelectMode::Manual;
        cfg.broadphase = BroadphaseKind::Tree;
    }
    let mut schedule = builder.build(&mut world);
    for _ in 0..STEPS {
        schedule.run(&mut world);
    }

    let hash = pose_hash(&world, &boxes);
    let mut past_half_m = 0;
    let mut past_tenth_m = 0;
    let mut max_drift_m = 0.0f64;
    let mut max_rotation_deg = 0.0f64;
    let mut top_offset_m = 0.0f64;
    for (k, (&e, p0)) in boxes.iter().zip(lattice).enumerate() {
        let b = world.get_component::<RigidBody>(e).expect("invariant: a spawned box is live");
        let dx = f64::from(b.position.x) - f64::from(p0.x);
        let dz = f64::from(b.position.z) - f64::from(p0.z);
        let drift = (dx * dx + dz * dz).sqrt();
        past_half_m += usize::from(drift > 0.5);
        past_tenth_m += usize::from(drift > 0.1);
        max_drift_m = max_drift_m.max(drift);
        let w = f64::from(b.rotation.w).abs().clamp(0.0, 1.0);
        max_rotation_deg = max_rotation_deg.max(2.0 * w.acos().to_degrees());
        if k + 1 == boxes.len() {
            top_offset_m = drift;
        }
    }
    Trajectory {
        seed,
        pose_hash: hash,
        past_half_m,
        past_tenth_m,
        max_drift_m,
        top_offset_m,
        max_rotation_deg,
    }
}

/// Runs a cell's eight trajectories, prints every row, checks the anchor, then the rule.
fn gate(cell: Cell) {
    let lattice = lattice();
    assert_eq!(lattice.len(), PYRAMID_BODIES, "construction: Jolt's placement loop");
    let runs: Vec<Trajectory> = SEEDS.iter().map(|&seed| run(cell, &lattice, seed)).collect();
    let rows: Vec<String> = runs.iter().map(Trajectory::row).collect();
    let held = runs.iter().filter(|t| t.holds()).count();
    println!(
        "V2 fidelity cell {} (contact_reuse {}, sleeping {}): {held}/{} hold\n  {}",
        cell.name,
        cell.reuse,
        cell.sleeping,
        runs.len(),
        rows.join("\n  ")
    );
    let unperturbed = runs[0];
    assert!(unperturbed.seed.is_none(), "construction: the first seed is the unperturbed scene");
    if let Some(anchor) = cell.anchor {
        assert_eq!(
            unperturbed.pose_hash, anchor,
            "cell {}: the unperturbed trajectory ends in {:#018x}, not the parity runner's fixture \
             {anchor:#018x} — this file no longer builds the scene the runner builds (a defect of \
             the test, not a finding about the contact rule)",
            cell.name, unperturbed.pose_hash
        );
    }
    assert!(
        held == runs.len(),
        "cell {}: {held}/{} trajectories hold (no box past {DRIFT_LIMIT_M} m, max drift under \
         {DRIFT_LIMIT_M} m); the pile does not stand:\n  {}",
        cell.name,
        runs.len(),
        rows.join("\n  ")
    );
}

#[test]
#[ignore = "deferred: V2 C5 - red by design (0/8 on the pre-V2 contact rule) until the lane's C5 makes speculative contacts the default; then `slow:` (32 runs of Jolt's 1240-box pyramid)"]
fn j_t_stands_contact_reuse_on_sleeping_off() {
    gate(Cell {
        name: "reuse-on/sleep-off",
        reuse: true,
        sleeping: false,
        anchor: Some(ANCHOR_REUSE_ON_SLEEP_OFF),
    });
}

#[test]
#[ignore = "deferred: V2 C5 - red by design (0/8 on the pre-V2 contact rule) until the lane's C5 makes speculative contacts the default; then `slow:` (32 runs of Jolt's 1240-box pyramid)"]
fn j_t_stands_contact_reuse_off_sleeping_off() {
    gate(Cell {
        name: "reuse-off/sleep-off",
        reuse: false,
        sleeping: false,
        anchor: Some(ANCHOR_REUSE_OFF_SLEEP_OFF),
    });
}

#[test]
#[ignore = "deferred: V2 C5 - red by design (0/8 on the pre-V2 contact rule) until the lane's C5 makes speculative contacts the default; then `slow:` (32 runs of Jolt's 1240-box pyramid)"]
fn j_t_stands_contact_reuse_on_sleeping_on() {
    gate(Cell {
        name: "reuse-on/sleep-on",
        reuse: true,
        sleeping: true,
        anchor: Some(ANCHOR_REUSE_ON_SLEEP_ON),
    });
}

#[test]
#[ignore = "deferred: V2 C5 - red by design (0/8 on the pre-V2 contact rule) until the lane's C5 makes speculative contacts the default; then `slow:` (32 runs of Jolt's 1240-box pyramid)"]
fn j_t_stands_contact_reuse_off_sleeping_on() {
    gate(Cell {
        name: "reuse-off/sleep-on",
        reuse: false,
        sleeping: true,
        anchor: Some(ANCHOR_REUSE_OFF_SLEEP_ON),
    });
}
