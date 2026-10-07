//! FM C0 — the friction lever's gates, written on the per-point base before the lever exists.
//!
//! Lever FM (friction per manifold: two tangent rows and one twist row at the patch centre, Jolt
//! 5.6's and Rapier's model) changes values. This file fixes, before any of its code lands, what
//! it must not move and how its friction must behave, so that FM is judged against bounds and
//! pins written on the per-point solver that ships today.
//!
//! # The count-1 scene hashes (G-P5's fixtures)
//!
//! FM changes only manifolds of two or more points: a one-point contact has no patch, so its rows
//! are the per-point rows bit for bit. Three scenes whose every manifold holds exactly one point
//! are therefore pinned here at the shipped default (V2's speculative contacts, contact reuse on,
//! `simd_solve` on), and FM may never move them (`fm/design_r2.md` §9.3):
//!
//! | scene | fixture |
//! |---|---|
//! | sphere pile | 64 spheres (r 0.5, `inv_mass` 1, μ 0.5) in four 4 × 4 layers on the floor (μ 0.5). Same-layer neighbours touch exactly; each odd layer is offset by (0.5, 0.5) and starts 0.145 m above its seat, so it drops, and its outer row overhangs the base and rolls off |
//! | sphere-SDF | 8 spheres (μ 0.3) on the SDF half-space `y <= 0`, launched at (2, 0, 0.25·i) m/s: they slide, then friction brings them to rolling. The SDF contact takes the body's own μ |
//! | sphere-on-box | 8 touching spheres (μ 0.4) at rest on the top face of a static box (half (20, 0.5, 6), μ 0.4) rotated by R_y(30°)·R_z(15°): they roll about 14.5 m down the 40 m face in 4 s |
//!
//! Each runs 240 steps at dt 1/60 with sleeping off, on four cells: W ∈ {1, 8} × `simd_solve` ∈
//! {off, on}. The hash is FNV-1a 64 over every `RigidBody`'s 13 `f32` words in walk order, taken
//! after every step, as in `default_world_worker_invariance.rs`. Every scene asserts its premises
//! before its pin, so a red names what broke:
//!
//! * **P1** every manifold of every step holds exactly one point (the step, the pair and the count
//!   are named);
//! * **P2** every dynamic body holds a manifold on at least half of the steps;
//! * **P3** friction reaches the hash: the scene's μ = 0 twin (W 1, `simd_solve` on) ends on a
//!   different final hash, so a pin that friction cannot move is a red, not a green;
//! * **P4** the four cells' per-step hashes are identical; a divergence is reported by its first
//!   step. A W 1 / W 8 or `simd_solve` off / on difference is an engine identity defect, never a
//!   per-cell pin.
//!
//! The W 8 cells are a configuration identity, not a parallel-path test: 64 bodies do not reach the
//! colour dispatch floor (256 slots, `default_world_worker_invariance.rs`) or the narrowphase chunk
//! floor, so a W 8 step runs the serial paths. A count-1 scene that dispatches (about 600 resting
//! spheres) would make G-P5's W 8 cell exercise them; that is FM C1's to add if wanted.
//!
//! **Pin rule.** Each pin is the final step's hash, the same value in debug and release. FM must not
//! move it. Otherwise it follows A7-R1's re-pin rule (`sleep_settles_box_piles.rs`): only a
//! value-changing lever that changes one-point contact values re-pins it, in the commit that makes
//! the change, from this test's own `final hash` line, naming the lever and the old value. In a
//! commit that claims bit identity a moved pin is a defect, never a re-pin.
//!
//! # Harness
//!
//! Bodies are spawned with `EcsMaster::spawn_batch` of [`RigidBodyBundle`] (no `unsafe` in this
//! file); a dynamic body is then enabled as [`Simulated`], a static one is not. The world is the
//! shipped default, `add_physics_systems::<DefaultRigidSolver>` (the colored solve), or
//! `add_physics_sdf::<DefaultRigidSolver>` for the SDF scene, on a `ThreadPoolBuilder` pool of the
//! cell's W. After the build, [`PhysicsConfig`] gets the scene's gravity, the cell's `simd_solve`
//! and `sleeping = false`, written explicitly (lever ruling L8 W1; it is also the default). Every
//! other field keeps its default. The per-arm configuration is the one [`Arm`] parameter, so an arm
//! that later needs one more flag is one hunk in [`Sim::new`].
//!
//! The gather walks the single `RigidBodyBundle` archetype in spawn order, so a manifold's row
//! index is the body's spawn index; [`Sim::new`] asserts that the walk reproduces the spawn
//! positions bit for bit before any step, which is what makes that mapping a checked fact.
//!
//! # Legs
//!
//! Every test runs in both profiles in the ordinary `cargo test -p boyko-physics --test
//! friction_manifold` run. The file spins thread pools, which is intractable under Miri, so the
//! whole file is `cfg(not(miri))`; no test in it carries an ignore attribute.

#![cfg(not(miri))]

use std::time::Duration;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::manifold::{Manifold, SDF_SENTINEL};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::{add_physics_sdf, add_physics_systems};
use boyko_physics::resources::{Manifolds, PhysicsConfig};
use boyko_physics::sdf_query::SdfField;
use boyko_physics::solver::{DefaultRigidSolver, RigidSolver};
use boyko_sdf_math::{SdfEdit, sdf_op};

// ── Harness ──────────────────────────────────────────────────────────────────

/// The static floor's half-extents: a box whose top face is `y = 0`.
const FLOOR_HALF: Vec3 = Vec3::new(50.0, 1.0, 50.0);
/// The static floor's centre.
const FLOOR_CENTRE: Vec3 = Vec3::new(0.0, -1.0, 0.0);
/// The SDF floor's box: centre `(0, -50, 0)`, half-extents 50, so its top face is `y = 0`
/// (`sdf_collision.rs`'s incline field).
const SDF_FLOOR_HALF: f32 = 50.0;

/// One body of a scene, in spawn order.
#[derive(Clone, Copy, Debug)]
struct BodySpec {
    shape: ColliderShape,
    position: Vec3,
    rotation: Quat,
    linear_velocity: Vec3,
    angular_velocity: Vec3,
    /// `0` is a static body, spawned without [`Simulated`].
    inv_mass: f32,
    friction: f32,
}

impl BodySpec {
    /// A static box.
    fn static_box(position: Vec3, rotation: Quat, half_extents: Vec3, friction: f32) -> Self {
        Self {
            shape: ColliderShape::Box { half_extents },
            position,
            rotation,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            inv_mass: 0.0,
            friction,
        }
    }

    /// A dynamic sphere at rest.
    fn sphere(position: Vec3, radius: f32, inv_mass: f32, friction: f32) -> Self {
        Self {
            shape: ColliderShape::Sphere { radius },
            position,
            rotation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            inv_mass,
            friction,
        }
    }

    /// The components the body spawns with. Restitution is 0 everywhere in this file.
    fn bundle(&self) -> RigidBodyBundle {
        RigidBodyBundle {
            body: RigidBody {
                position: self.position,
                linear_velocity: self.linear_velocity,
                rotation: self.rotation,
                angular_velocity: self.angular_velocity,
            },
            mass: RigidBodyMass {
                inv_inertia: inv_inertia_of(self.shape, self.inv_mass),
                inv_mass: self.inv_mass,
                restitution: 0.0,
                friction: self.friction,
            },
            collider: Collider { shape: self.shape, layer: 1, mask: 1 },
        }
    }
}

/// The local inverse inertia of a solid shape, by the formula the gather uses
/// (`BodyState::from_columns`). The gather derives it from the shape and never reads the authored
/// value; it is authored anyway so that the spawned components describe the body truthfully.
fn inv_inertia_of(shape: ColliderShape, inv_mass: f32) -> Mat3 {
    if inv_mass == 0.0 {
        return Mat3::ZERO;
    }
    match shape {
        ColliderShape::Sphere { radius } => {
            let inv = inv_mass * 5.0 / (2.0 * radius * radius);
            Mat3::from_diagonal(Vec3::new(inv, inv, inv))
        }
        ColliderShape::Box { half_extents } => {
            let (w, h, d) = (2.0 * half_extents.x, 2.0 * half_extents.y, 2.0 * half_extents.z);
            Mat3::from_diagonal(Vec3::new(
                12.0 * inv_mass / (h * h + d * d),
                12.0 * inv_mass / (w * w + d * d),
                12.0 * inv_mass / (w * w + h * h),
            ))
        }
    }
}

/// What a scene's bodies collide with besides each other.
#[derive(Clone, Copy, Debug)]
enum Ground {
    /// Nothing: the scene's own static bodies are its ground (`add_physics_systems`).
    Bodies,
    /// The SDF half-space `y <= 0` (`add_physics_sdf`).
    SdfFloor,
}

/// A scene: its bodies in spawn order, its gravity and its step.
#[derive(Clone, Debug)]
struct Scene {
    bodies: Vec<BodySpec>,
    gravity: Vec3,
    dt: f32,
    ground: Ground,
}

/// The configuration one run of a scene uses: the pool's worker count and `simd_solve`. Every
/// other [`PhysicsConfig`] field keeps its default, except `sleeping`, written off explicitly.
#[derive(Clone, Copy, Debug)]
struct Arm {
    workers: usize,
    simd_solve: bool,
}

/// The shipped default on one worker.
const DEFAULT_ARM: Arm = Arm { workers: 1, simd_solve: true };

/// A built world and its physics schedule.
struct Sim {
    world: EcsMaster,
    schedule: Schedule,
}

impl Sim {
    /// Spawns `scene`, builds solver `S`'s pipeline on a pool of `arm.workers` and applies `arm`.
    fn new<S: RigidSolver + Default>(scene: &Scene, arm: Arm) -> Self {
        let mut world = EcsMaster::new();
        let bundles: Vec<RigidBodyBundle> = scene.bodies.iter().map(BodySpec::bundle).collect();
        let entities = world
            .spawn_batch(bundles)
            .expect("construction: a scene's bodies fit one spawn batch");
        for (spec, &entity) in scene.bodies.iter().zip(&entities) {
            if spec.inv_mass != 0.0 {
                world.enable::<Simulated>(entity);
            }
        }
        let pool = ThreadPoolBuilder::new().num_threads(arm.workers).build();
        let mut builder = ScheduleBuilder::new(pool);
        match scene.ground {
            Ground::Bodies => {
                let _keys = add_physics_systems::<S>(&mut builder, &mut world);
            }
            Ground::SdfFloor => {
                let _keys = add_physics_sdf::<S>(&mut builder, &mut world);
                *world.resource_mut::<SdfField>() = SdfField::from_edits(&[SdfEdit::box_shape(
                    [0.0, -SDF_FLOOR_HALF, 0.0],
                    [SDF_FLOOR_HALF, SDF_FLOOR_HALF, SDF_FLOOR_HALF],
                    sdf_op::UNION,
                    0.0,
                )]);
            }
        }
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(scene.dt)));
        let schedule = builder.build(&mut world);
        {
            let cfg = world.resource_mut::<PhysicsConfig>();
            cfg.gravity = scene.gravity;
            cfg.simd_solve = arm.simd_solve;
            cfg.sleeping = false;
        }
        let mut sim = Self { world, schedule };
        let walked = sim.bodies();
        assert_eq!(
            walked.len(),
            scene.bodies.len(),
            "harness: the walk must hold every spawned body"
        );
        for (row, (body, spec)) in walked.iter().zip(&scene.bodies).enumerate() {
            assert!(
                body_words(body)
                    .iter()
                    .zip(body_words(&spec.bundle().body))
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "harness: walk row {row} is not spawn {row} ({body:?} against {spec:?}), so a \
                 manifold's row index would not name the body this file thinks it does"
            );
        }
        sim
    }

    /// One physics step.
    fn step(&mut self) {
        self.schedule.run(&mut self.world);
    }

    /// Every body, in walk (= spawn) order.
    fn bodies(&mut self) -> Vec<RigidBody> {
        let q = self.world.query::<&RigidBody, ()>();
        q.iter().copied().collect()
    }

    /// The last step's manifolds.
    fn manifolds(&self) -> Vec<Manifold> {
        self.world.resource::<Manifolds>().manifolds().iter().collect()
    }
}

/// A `RigidBody`'s 13 words, in the hash's order.
fn body_words(b: &RigidBody) -> [f32; 13] {
    [
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
    ]
}

/// FNV-1a 64 over every word of every body, in walk order.
fn state_hash(bodies: &[RigidBody]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for body in bodies {
        for word in body_words(body) {
            for byte in word.to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    hash
}

// ── The count-1 scenes (G-P5's fixtures) ─────────────────────────────────────

/// The count-1 scenes' step.
const COUNT1_DT: f32 = 1.0 / 60.0;
/// The count-1 scenes' length.
const COUNT1_STEPS: usize = 240;
/// The four cells every count-1 scene runs on; the first is the shipped default, whose final hash
/// is the pin and whose μ = 0 twin is P3's.
const COUNT1_CELLS: [Arm; 4] = [
    DEFAULT_ARM,
    Arm { workers: 1, simd_solve: false },
    Arm { workers: 8, simd_solve: false },
    Arm { workers: 8, simd_solve: true },
];
/// The count-1 spheres' radius.
const COUNT1_RADIUS: f32 = 0.5;

/// The sphere pile's final hash at the shipped default (V2), every cell (pin rule: module header).
/// Read at FM C0 on the trunk `b46cf0ec` (msvc, 2026-10-07), the same line in debug and release.
const PIN_COUNT1_SPHERE_PILE: u64 = 0x5b9c_1935_bee2_5490;
/// The sphere-SDF scene's final hash at the shipped default (V2), every cell; read with
/// [`PIN_COUNT1_SPHERE_PILE`].
const PIN_COUNT1_SPHERE_SDF: u64 = 0x9e53_aac5_6ed0_ca21;
/// The sphere-on-box scene's final hash at the shipped default (V2), every cell; read with
/// [`PIN_COUNT1_SPHERE_PILE`].
const PIN_COUNT1_SPHERE_ON_BOX: u64 = 0x26b4_d75e_212f_87a2;

/// `R = R_y(30°)·R_z(15°)`, the sphere-on-box incline's rotation, as `f32` bits: the Hamilton
/// product of `(0, sin 15°, 0, cos 15°)` and `(0, 0, sin 7.5°, cos 7.5°)`, which is
/// `(sin 15°·sin 7.5°, sin 15°·cos 7.5°, cos 15°·sin 7.5°, cos 15°·cos 7.5°)`, each component
/// computed in `f64` and rounded once (`fm-c0/dev/quat_bits.py`). Literal bits, so the pinned
/// fixture does not depend on a platform's `sin`.
const SPHERE_ON_BOX_ROTATION_BITS: [u32; 4] = [0x3d0a_5fb1, 0x3e83_61b5, 0x3e01_1ac1, 0x3f75_295a];

/// The static floor of μ `mu`.
fn floor(mu: f32) -> BodySpec {
    BodySpec::static_box(FLOOR_CENTRE, Quat::IDENTITY, FLOOR_HALF, mu)
}

/// The 64-sphere pile on the floor; every body has friction `mu` (0.5 at the pin).
fn sphere_pile(mu: f32) -> Scene {
    let mut bodies = vec![floor(mu)];
    for k in 0..4 {
        let odd = 0.5 * (k % 2) as f32;
        for i in 0..4 {
            for j in 0..4 {
                let centre = Vec3::new(
                    (i as f32 - 1.5) + odd,
                    0.5 + 0.9 * k as f32,
                    (j as f32 - 1.5) + odd,
                );
                bodies.push(BodySpec::sphere(centre, COUNT1_RADIUS, 1.0, mu));
            }
        }
    }
    Scene { bodies, gravity: Vec3::new(0.0, -9.81, 0.0), dt: COUNT1_DT, ground: Ground::Bodies }
}

/// Eight spheres launched across the SDF floor; every sphere has friction `mu` (0.3 at the pin).
fn sphere_sdf(mu: f32) -> Scene {
    let bodies = (0..8)
        .map(|i| {
            let mut sphere = BodySpec::sphere(
                Vec3::new(-7.0 + 2.0 * i as f32, COUNT1_RADIUS, 0.0),
                COUNT1_RADIUS,
                1.0,
                mu,
            );
            sphere.linear_velocity = Vec3::new(2.0, 0.0, 0.25 * i as f32);
            sphere
        })
        .collect();
    Scene { bodies, gravity: Vec3::new(0.0, -9.81, 0.0), dt: COUNT1_DT, ground: Ground::SdfFloor }
}

/// Eight touching spheres at rest on an inclined, yawed static box; every body has friction `mu`
/// (0.4 at the pin).
fn sphere_on_box(mu: f32) -> Scene {
    let [x, y, z, w] = SPHERE_ON_BOX_ROTATION_BITS.map(f32::from_bits);
    let rotation = Quat::new(x, y, z, w);
    let mut bodies =
        vec![BodySpec::static_box(Vec3::ZERO, rotation, Vec3::new(20.0, 0.5, 6.0), mu)];
    for i in 0..8 {
        let local = Vec3::new(12.0, 1.0, -3.5 + i as f32);
        bodies.push(BodySpec::sphere(rotation.rotate(local), COUNT1_RADIUS, 1.0, mu));
    }
    Scene { bodies, gravity: Vec3::new(0.0, -9.81, 0.0), dt: COUNT1_DT, ground: Ground::Bodies }
}

/// Runs `scene` on `arm` for [`COUNT1_STEPS`], asserting P1 on every step and P2 at the end, and
/// returns the state hash after every step.
fn run_count1(name: &str, scene: &Scene, arm: Arm) -> Vec<u64> {
    let mut sim = Sim::new::<DefaultRigidSolver>(scene, arm);
    let n = scene.bodies.len();
    let mut steps_in_contact = vec![0usize; n];
    let mut hashes = Vec::with_capacity(COUNT1_STEPS);
    for step in 1..=COUNT1_STEPS {
        sim.step();
        let mut touched = vec![false; n];
        for m in sim.manifolds() {
            assert_eq!(
                m.count,
                1,
                "{name}, {arm:?}, premise P1: step {step}, pair ({}, {}) holds count {}; every \
                 manifold of a count-1 scene must hold exactly one point, or the scene does not \
                 isolate one-point contacts. A narrowphase surprise: STOP, never reshape the \
                 fixture silently",
                m.body_a.0,
                if m.body_b == SDF_SENTINEL { "SDF".to_string() } else { m.body_b.0.to_string() },
                m.count
            );
            touched[m.body_a.0 as usize] = true;
            if m.body_b != SDF_SENTINEL {
                touched[m.body_b.0 as usize] = true;
            }
        }
        for (count, hit) in steps_in_contact.iter_mut().zip(touched) {
            *count += usize::from(hit);
        }
        let bodies = sim.bodies();
        assert!(
            bodies.iter().all(|b| body_words(b).iter().all(|w| w.is_finite())),
            "{name}, {arm:?}: a non-finite body state at step {step}"
        );
        hashes.push(state_hash(&bodies));
    }
    for (row, spec) in scene.bodies.iter().enumerate() {
        if spec.inv_mass != 0.0 {
            assert!(
                2 * steps_in_contact[row] >= COUNT1_STEPS,
                "{name}, {arm:?}, premise P2: body {row} held a manifold on {} of {COUNT1_STEPS} \
                 steps; every dynamic body must touch something on at least half of them, or the \
                 scene does not exercise its contacts",
                steps_in_contact[row]
            );
        }
    }
    hashes
}

/// The first step (1-based) on which two per-step hash traces differ, if any.
fn first_divergence(a: &[u64], b: &[u64]) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .or_else(|| (a.len() != b.len()).then_some(a.len().min(b.len())))
        .map(|i| i + 1)
}

/// Runs a count-1 scene on every cell and its μ = 0 twin, checks P1–P4, and compares the final
/// hash with `pin`.
fn assert_count1_scene(name: &str, scene: &Scene, twin: &Scene, pin: u64) {
    let cells: Vec<(Arm, Vec<u64>)> =
        COUNT1_CELLS.iter().map(|&arm| (arm, run_count1(name, scene, arm))).collect();
    let (base_arm, base) = &cells[0];
    for (arm, hashes) in &cells[1..] {
        if let Some(step) = first_divergence(base, hashes) {
            panic!(
                "{name}, premise P4: {arm:?} diverges from {base_arm:?} first at step {step} \
                 ({:#018x} against {:#018x}). The engine promises the same bits at every W and \
                 with `simd_solve` off and on: an existing identity defect. STOP and report; never \
                 pin per cell",
                hashes.get(step - 1).copied().unwrap_or(0),
                base.get(step - 1).copied().unwrap_or(0)
            );
        }
    }
    let final_hash = *base.last().expect("invariant: a count-1 run takes at least one step");
    let twin_final = *run_count1(&format!("{name} (μ = 0 twin)"), twin, DEFAULT_ARM)
        .last()
        .expect("invariant: a count-1 run takes at least one step");
    assert_ne!(
        twin_final, final_hash,
        "{name}, premise P3: the μ = 0 twin ends on the same final hash {final_hash:#018x}, so \
         friction does not reach this scene's pin and the fixture is vacuous for FM. STOP"
    );
    println!(
        "{name}: final hash {final_hash:#018x} on all {} cells (μ = 0 twin {twin_final:#018x})",
        cells.len()
    );
    assert_eq!(
        final_hash, pin,
        "{name}: final hash {final_hash:#018x} (pin {pin:#x}). FM must never move it; in a \
         commit that claims bit identity a change is a defect. Only a value-changing lever that \
         changes one-point contact values re-pins it, from this line, naming the lever and the \
         old value"
    );
}

/// G-P5's sphere pile: one-point sphere-sphere and sphere-floor contacts, identical on every cell,
/// pinned.
#[test]
fn count1_sphere_pile_hash_is_pinned_across_w_and_simd() {
    let scene = sphere_pile(0.5);
    assert_eq!(scene.bodies.len(), 65, "construction: a floor and 64 spheres");
    assert_count1_scene("count-1 sphere pile", &scene, &sphere_pile(0.0), PIN_COUNT1_SPHERE_PILE);
}

/// G-P5's sphere-SDF scene: one-point sphere-field contacts, identical on every cell, pinned.
#[test]
fn count1_sphere_sdf_hash_is_pinned_across_w_and_simd() {
    assert_count1_scene(
        "count-1 sphere-SDF",
        &sphere_sdf(0.3),
        &sphere_sdf(0.0),
        PIN_COUNT1_SPHERE_SDF,
    );
}

/// G-P5's sphere-on-box scene: one-point sphere-OBB-face and sphere-sphere contacts, identical on
/// every cell, pinned.
#[test]
fn count1_sphere_on_box_hash_is_pinned_across_w_and_simd() {
    assert_count1_scene(
        "count-1 sphere-on-box",
        &sphere_on_box(0.4),
        &sphere_on_box(0.0),
        PIN_COUNT1_SPHERE_ON_BOX,
    );
}
