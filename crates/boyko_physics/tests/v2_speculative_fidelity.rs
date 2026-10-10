//! **V2's fidelity gate: Jolt's pyramid must stand after its landing, at every drop height.** The
//! J-T scene (Jolt's `PyramidScene`, height 15, 1240 boxes, friction 0.2, restitution 0) run for
//! 500 steps through the real schedule at one worker, over the gap sweep of the owner's ruling
//! (2026-09-30, item 9): the layer gap — the drop each layer lands from — at 0.5, 0.75, 0.9, 0.95,
//! 1.0, 1.02, 1.05, 1.1, 1.25, 1.5 and 2.0 m, with contact reuse on and off.
//!
//! # Why this gate exists
//!
//! The pose-hash gates compare runs with each other and with pinned bytes; none of them can see
//! whether the pile holds. On the pre-V2 contact rule it does not: a contact existed only on
//! overlap, the layers land at 3–12.5 m/s, most boxes of a layer land on one to three of their four
//! supports, get kicked sideways, and the outer ring slides off (F0 verdict: 0 of 32 trajectories
//! hold at gap 0.5, the top box 475 mm off, 28.8°; Jolt 5.6 holds with 0 boxes past 0.5 m, 51 mm,
//! 0.54°). V2 (owner V2a, V2b = 20 mm) keeps a contact point while its separation is at most
//! `d + min(cap, approach · h)` — the speculative distance plus the approach-velocity margin — and
//! solves a positive separation as a speculative bias. The fixed distance alone only moves the
//! landing boundary (F0f); with the velocity term the pile holds at every gap (F0g: 88 of 88
//! runs; 35 of 88 without the term; Jolt 5.6 37 of 44).
//!
//! # The runs
//!
//! The pile is chaotic, so one trajectory proves little: at gaps 0.5 and 1.0 each arm also runs
//! the scene with box 600's spawn `x` nudged by one ulp (`f32::next_up`), one of F0g's seeds. Every
//! other gap runs the unperturbed scene. Sleeping is off in the sweep (the runner's J-T row); a
//! third arm runs gap 0.5 with sleeping on, contact reuse on and off.
//!
//! # The rule
//!
//! Per run, each box's horizontal drift from its spawn lattice point at step 500 (the F0 `pile.py`
//! metric). **A run holds iff no box has drifted past 0.5 m and the max drift is under 0.5 m; an
//! arm passes iff every run holds.** Every run's row is printed (boxes past 0.5 m and 0.1 m, max
//! drift, the top box's offset, the max rotation, the pose hash) before the verdict, pass or fail.
//!
//! # The red control
//!
//! A gate that holds on the shipped rule could also be holding for a reason that is not the
//! velocity term. So one run is required to FAIL: gap 1.5, contact reuse on, the velocity term
//! capped at 1 mm (F0g: 19 boxes past 0.5 m, 9.4 m max drift). If it holds, the gate cannot see
//! the term it certifies.
//!
//! # The anchors (the test builds J-T, not a look-alike)
//!
//! At gap 0.5, the unperturbed runs end in the pose hash of the parity runner's fixture for the
//! same configuration — FNV-1a 64 over every dynamic box's `RigidBody` as 13 little-endian `f32`,
//! in spawn order, the runner's `pose_bytes`. A mismatch means this file no longer builds the scene
//! the runner builds, which is a defect of the test, not a finding about the contact rule.
//!
//! # How to run it
//!
//! Release only in practice (29 runs of a 1240-box pile): the device-free `-- --ignored` leg,
//! `cargo test -p boyko-physics --release --locked --test v2_speculative_fidelity -- --ignored
//! --nocapture`. The four arms are four tests, so libtest runs them side by side.

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
/// Jolt's `cBoxSeparation`: J-T's gap, the one the runner's fixtures anchor.
const GAP: f32 = 0.5;
/// The owner's gap sweep (ruling 2026-09-30, item 9; F0g's gaps).
const GAPS: [f32; 11] = [0.5, 0.75, 0.9, 0.95, 1.0, 1.02, 1.05, 1.1, 1.25, 1.5, 2.0];
/// The gaps that also run the one-ulp seed.
const SEEDED_GAPS: [f32; 2] = [0.5, 1.0];
/// The one-ulp seed: box 600's spawn `x` nudged by one ulp (one of F0g's seeds).
const SEED: usize = 600;
/// The red control's gap and velocity-term cap (F0g's control: 19 boxes past 0.5 m).
const CONTROL_GAP: f32 = 1.5;
const CONTROL_CAP: f32 = 0.001;
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

/// The parity runner's V2 J-T fixtures (`docs/measurements/2026-09-30-v2-speculative/w8/`,
/// recorded at W1 by the flip's runner with `tools/record_v2_fixtures.sh`), by the configuration
/// they were recorded with: `JT500` (`--cfg default --broadphase tree --sleeping off`). Re-pinned at
/// V2's flip from the pre-V2 fixture `0x30c5_438b_c6ad_9ffa` (window 8's `JT500`, which the
/// overlap-only rule still reproduces); the four anchors below equal this file's own runs on the
/// flip's tree, so the two builders agree.
const ANCHOR_REUSE_ON_SLEEP_OFF: u64 = 0x441a_568e_91a4_f9c9;
/// `JToff500`: `JT500` with `--contact-reuse off` (pre-V2: `0x32d5_e235_342b_4143`).
const ANCHOR_REUSE_OFF_SLEEP_OFF: u64 = 0xbc09_a1fa_f7b4_13a8;
/// `JSonT500`: J-T with sleeping on (recorded as `--cfg a --broadphase tree --sleeping on`, which
/// the engine's scalar/SIMD and worker-count identities make the default configuration's bits;
/// this file's run is the measurement that they are). Pre-V2: `0x3db4_7fae_414b_655c`.
const ANCHOR_REUSE_ON_SLEEP_ON: u64 = 0x130c_76cb_6b46_3ab8;
/// `JSonToff500`: `JSonT500` with `--contact-reuse off`, 500 steps (V2's recorder adds the row; the
/// pre-V2 anchor was window 6's 1000-step `Son-J1000`, `0xcc2a_5400_c66e_ecce`, whose pile was frozen
/// before step 500).
const ANCHOR_REUSE_OFF_SLEEP_ON: u64 = 0xc671_8318_9439_4df6;

/// FNV-1a 64 offset basis (the runner's).
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64 prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// One run of the gate.
#[derive(Clone, Copy, Debug)]
struct Run {
    /// The layer gap, metres.
    gap: f32,
    /// `PhysicsConfig::contact_reuse`.
    reuse: bool,
    /// `PhysicsConfig::sleeping`.
    sleeping: bool,
    /// `None` is the unperturbed scene; `Some(k)` nudges box `k`'s spawn `x` by one ulp.
    seed: Option<usize>,
    /// `PhysicsConfig::speculative_velocity_cap` when `Some` (the red control); the default's
    /// otherwise.
    cap: Option<f32>,
}

/// What one run ended in.
#[derive(Clone, Copy, Debug)]
struct Trajectory {
    run: Run,
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
        let r = self.run;
        let seed = r.seed.map_or_else(|| "none".to_owned(), |k| k.to_string());
        let cap = r.cap.map_or_else(String::new, |c| format!(" cap {c}"));
        format!(
            "gap {:4} reuse {:3} sleep {:3} seed {seed:>4}{cap}: >0.5m {:3}  >0.1m {:4}  max {:9.1} mm  \
             top {:8.1} mm  rot {:7.2} deg  hash {:#018x}  {}",
            r.gap,
            if r.reuse { "on" } else { "off" },
            if r.sleeping { "on" } else { "off" },
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

/// Every box's spawn lattice point at layer gap `gap`, in spawn order: Jolt's placement loop,
/// index for index.
fn lattice(gap: f32) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(PYRAMID_BODIES);
    for i in 0..PYRAMID_HEIGHT {
        let lo = i / 2;
        let hi = PYRAMID_HEIGHT - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
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

/// Runs `run` to step 500 at one worker and measures the pile.
fn run(run: Run) -> Trajectory {
    let lattice = lattice(run.gap);
    assert_eq!(lattice.len(), PYRAMID_BODIES, "construction: Jolt's placement loop");
    let mut world = EcsMaster::new();
    let boxes = spawn_scene(&mut world, &lattice, run.seed);
    assert_eq!(boxes.len(), PYRAMID_BODIES, "construction: J-T has {PYRAMID_BODIES} dynamic boxes");
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
    let keys = add_physics_colored_solve(&mut builder, &mut world);
    assert!(keys.build_graph.is_some(), "construction: the colored pipeline builds the graph");
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    {
        // The parity runner's `configure` for `--cfg default --broadphase tree --sleeping off|on`
        // (plus `--contact-reuse off` for a reuse-off run, `--gap G` for the gap, and
        // `--speculative-velocity-cap C` for the red control).
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
        cfg.dt = DT;
        cfg.sleeping = run.sleeping;
        cfg.contact_reuse = run.reuse;
        cfg.broadphase_select = BroadphaseSelectMode::Manual;
        cfg.broadphase = BroadphaseKind::Tree;
        if let Some(cap) = run.cap {
            cfg.speculative_velocity_cap = cap;
        }
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
    for (k, (&e, p0)) in boxes.iter().zip(&lattice).enumerate() {
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
        run,
        pose_hash: hash,
        past_half_m,
        past_tenth_m,
        max_drift_m,
        top_offset_m,
        max_rotation_deg,
    }
}

/// Runs `runs`, prints every row, checks each anchored run's pose hash, then the rule: every run
/// holds.
fn gate(name: &str, runs: &[(Run, Option<u64>)]) {
    let results: Vec<(Trajectory, Option<u64>)> = runs.iter().map(|&(r, anchor)| (run(r), anchor)).collect();
    let rows: Vec<String> = results.iter().map(|(t, _)| t.row()).collect();
    let held = results.iter().filter(|(t, _)| t.holds()).count();
    println!("V2 fidelity {name}: {held}/{} runs hold\n  {}", results.len(), rows.join("\n  "));
    for (t, anchor) in &results {
        if let Some(anchor) = *anchor {
            assert_eq!(
                t.pose_hash, anchor,
                "{name}: the run {:?} ends in {:#018x}, not the parity runner's fixture {anchor:#018x} \
                 — this file no longer builds the scene the runner builds (a defect of the test, not a \
                 finding about the contact rule)",
                t.run, t.pose_hash
            );
        }
    }
    assert!(
        held == results.len(),
        "{name}: {held}/{} runs hold (no box past {DRIFT_LIMIT_M} m, max drift under {DRIFT_LIMIT_M} \
         m); the pile does not stand:\n  {}",
        results.len(),
        rows.join("\n  ")
    );
}

/// The sweep's runs with contact reuse `reuse`: every gap unperturbed, plus the seed at the seeded
/// gaps; gap 0.5's unperturbed run anchored to `anchor`.
fn sweep(reuse: bool, anchor: u64) -> Vec<(Run, Option<u64>)> {
    let mut runs = Vec::with_capacity(GAPS.len() + SEEDED_GAPS.len());
    for gap in GAPS {
        let at = |seed| Run { gap, reuse, sleeping: false, seed, cap: None };
        runs.push((at(None), (gap == GAP).then_some(anchor)));
        if SEEDED_GAPS.contains(&gap) {
            runs.push((at(Some(SEED)), None));
        }
    }
    runs
}

#[ignore = "slow: the J-T gap sweep - 13 runs of Jolt's 1240-box pyramid for 500 steps; release, the device-free -- --ignored leg"]
#[test]
fn j_t_stands_at_every_gap_contact_reuse_on() {
    gate("gap sweep, contact reuse on", &sweep(true, ANCHOR_REUSE_ON_SLEEP_OFF));
}

#[ignore = "slow: the J-T gap sweep - 13 runs of Jolt's 1240-box pyramid for 500 steps; release, the device-free -- --ignored leg"]
#[test]
fn j_t_stands_at_every_gap_contact_reuse_off() {
    gate("gap sweep, contact reuse off", &sweep(false, ANCHOR_REUSE_OFF_SLEEP_OFF));
}

#[ignore = "slow: J-T with sleeping on - 2 runs of Jolt's 1240-box pyramid for 500 steps; release, the device-free -- --ignored leg"]
#[test]
fn j_t_stands_with_sleeping_on() {
    let at = |reuse| Run { gap: GAP, reuse, sleeping: true, seed: None, cap: None };
    gate(
        "gap 0.5, sleeping on",
        &[(at(true), Some(ANCHOR_REUSE_ON_SLEEP_ON)), (at(false), Some(ANCHOR_REUSE_OFF_SLEEP_ON))],
    );
}

/// The red control: gap 1.5, contact reuse on, the velocity term capped at 1 mm must NOT hold. A
/// hold here means the sweep's gaps hold for a reason other than the velocity term, and the gate
/// cannot see the term it certifies.
#[ignore = "slow: the fidelity gate's red control - 1 run of Jolt's 1240-box pyramid for 500 steps; release, the device-free -- --ignored leg"]
#[test]
fn red_control_the_velocity_term_capped_at_1_mm_fails_at_gap_1_5() {
    let t = run(Run { gap: CONTROL_GAP, reuse: true, sleeping: false, seed: None, cap: Some(CONTROL_CAP) });
    println!("V2 fidelity red control: {}", t.row());
    assert!(
        !t.holds(),
        "the red control holds: {} — the gate cannot tell the velocity term from its absence",
        t.row()
    );
}
