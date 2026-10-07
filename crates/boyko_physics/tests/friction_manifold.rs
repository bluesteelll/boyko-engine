//! FM C0 — the friction lever's gates, written on the per-point base before the lever exists.
//!
//! Lever FM (friction per manifold: two tangent rows and one twist row at the patch centre, Jolt
//! 5.6's and Rapier's model) changes values. This file fixes, before any of its code lands, what
//! it must not move and how its friction must behave, so that FM is judged against bounds and
//! pins written on the per-point solver that ships today.
//!
//! # The friction behaviour gates (G-P1, G-P2, G-P3, G-P7)
//!
//! Each gate is one box on the static floor, run on the shipped default world (the colored solve,
//! W 1, `simd_solve` on). The floor's friction always equals the box's: the solver combines
//! friction as `max(a, b)` (`solver/colored.rs`, `solver/soft_step.rs`), so a floor at any other μ
//! would silently be the scene's μ. Bodies spawn touching (centre height = half-height), so V2's
//! speculative contact exists on step 1. Tilted gravity is `9.81·(sin θ, −cos θ, 0)` with
//! `tan θ` given, computed in `f64` and rounded once to `f32`; downhill is +x on a level floor, so a
//! yaw stays exactly about the contact normal. Every bound below was fixed in the design
//! (`fm/design_r2.md` §8 C0) or in the cut (`fm-c0/cut.md` §1.1) before any run of these scenes.
//!
//! | gate | scene | (component) bound |
//! |---|---|---|
//! | **G-P1** incline threshold | box half (0.5, 0.2, 0.5), 1 kg, spawned at (−40, 0.2, 0); yaw 0° and 30°; μ ∈ {0.2, 0.5, 0.8}; `tan θ = k·μ`; dt 1/120, 480 frames. 18 cells; tipping needs `tan θ > 2.5`, the largest is 1.6 | **stick** (k 0.95): creep `\|(x, z)(480) − (x, z)(240)\| < 5 mm`. **slide** (k 1.05): `x(480) − x(0) ∈ [0.5, 1.5]·½·a·(4 s)²`. **accel** (k 2): `(v_x(480) − v_x(240)) / 2 s` within ±5 % of `a`. Here `a = 9.81·(sin θ − μ cos θ)` |
//! | **G-P2** twist stop | box half (1, 0.25, 1), 8 kg, μ 0.2, at rest with ω₀ = (0, 2, 0) rad/s; dt 1/60, 300 steps | **stop**: the first step `s*` at whose end `\|ω\| < 1e-3` lies in [25, 33]. **hold**: `\|ω\| < 1e-3` at the end of each of the 60 steps after `s*`. **drift**: `\|(x, z)(s* + 60) − (x, z)(0)\| ≤ 1 mm` |
//! | **G-P3** yaw while sliding (Jolt #983) | box half (0.5, 0.25, 0.5), 1 kg, μ 0.3, yaw 30°, v₀ = (3, 0, 0) m/s; dt 1/60, 180 steps | **heading**: `ψ = atan2(−e.z, e.x)` with `e = R·x̂`, `\|ψ(180) − ψ(0)\| ≤ 0.5°`. **lateral**: `\|z(180) − z(0)\| ≤ 1 cm`. **stop distance**: `x(180) − x(0)` within ±5 % of `v₀² / (2μg)` = 1.529 m |
//! | **G-P7** single-box slide | one J-T box (half 1, `inv_mass` 0.125), μ 0.2, at rest; gravity = S-SLIDE's literal bits (`tan θ = ½`, `benches/jolt_parity_pyramid/dyn_spec.rs`); dt 1/60, 70 steps | **acceleration**: `(v_x(70) − v_x(10)) / 1 s` within ±3 % of `9.81·(sin θ − 0.2 cos θ)` = 2.632 m/s² |
//!
//! Derivations and estimator choices:
//!
//! * **G-P2's stop band** is the analytic per-corner model: four corners at `√2` from the axis,
//!   each carrying `mg/4`, give a friction torque `μmg√2` and a stop after `ω₀·I_y / (μmg√2)` =
//!   0.4805 s = 28.8 steps. Lever ruling L8 Review OQ1: the base registers inside [25, 33], and a
//!   base reading outside it is a finding, not a band change.
//! * **G-P2's drift bound** (design OQ1): in exact Coulomb the four corner forces of a symmetric
//!   box spinning in place cancel pairwise, so the ideal drift is 0. Per-point Gauss-Seidel sees
//!   each corner after the previous one's update and can break that symmetry, so the per-point
//!   base may drift; FM's tangent rows act at the patch centre, on the spin axis, and are
//!   predicted to pass.
//! * **G-P7's estimator is the velocity difference**, not the displacement: under symplectic Euler
//!   the displacement form `2·(x(70) − x(10) − v_x(10)·1 s)` reads `a` high by `+1/N` for `N`
//!   integration steps (+0.42 % at 240 substeps, +1.67 % at 60 steps), a bias inside a ±3 % band;
//!   the velocity difference is exact under constant acceleration. The displacement form is
//!   printed beside it as a receipt.
//! * **G-P1's windows**: stick and accel read the second half of the run (the first half settles
//!   the contact); slide reads the whole run from rest.
//!
//! Every gate also asserts its scene's premises, and a premise red is a STOP (the scene does not
//! stand: a fixture or engine finding), never a bound edit: every body finite on every step; the
//! box holds a manifold on every step of its gate's window; G-P2's box-floor manifold holds 4
//! points at step 1 (the analytic's 4 corners); G-P3's box has stopped (`|v(180)| < 1e-3`);
//! G-P7's box stays within 1° of its spawn orientation (no tumble; tipping needs `tan θ > 1`).
//!
//! Every gate also runs its scenes on the reference [`SoftStepSolver`] and prints `"<gate>
//! reference receipt (SoftStepSolver)"` lines. The reference is a receipt, never asserted (lever
//! ruling L8 W4): if it misses a bound that is reported, and the colored bound is never adjusted.
//!
//! # How a per-point arm that misses its bound is recorded (design OQ2)
//!
//! A gate is a set of (cell, component) pairs, each named `"<cell>: <component>"`. The bounds were
//! fixed before the per-point base first ran, and that run splits the pairs:
//!
//! * pairs the base passes form the plain test `<gate>_per_point`;
//! * pairs the base misses would form `<gate>_per_point_characterization`, a test that passes by
//!   asserting each listed pair still misses its bound on the side the base run recorded, with the
//!   live value in its message, never skipped by an ignore attribute and never `should_panic`. A
//!   test exists only for a non-empty set.
//!
//! **The base run met every pair** (msvc, trunk `b46cf0ec`, 2026-10-07, debug and release printing
//! identical values), so every pair is a plain gate and no characterization exists. That includes
//! G-P3, which the design expected the per-point base to fail (the Jolt #983 class): it read a
//! heading change of 0.00035° and a lateral drift of 2.7 µm. The base's readings, every reference
//! receipt among them, are this file's `--nocapture` lines.
//!
//! At FM C1 the FM arm is a plain gate against the same bounds. A per-point pair that misses later
//! is a change in the per-point solver, reported as such. The bound is never edited to follow a
//! reading.
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
//! Bodies are spawned with `EcsMaster::spawn_batch` of [`RigidBodyBundle`], the safe bundle path,
//! so the file needs no raw byte view; a dynamic body is then enabled as [`Simulated`], a static
//! one is not. The world is the shipped default, `add_physics_systems::<DefaultRigidSolver>` (the
//! colored solve), `add_physics_systems::<SoftStepSolver>` for a reference receipt, or
//! `add_physics_sdf::<DefaultRigidSolver>` for the SDF scene, on a `ThreadPoolBuilder` pool of the
//! cell's W (W 1 for every G-P scene). After the build, [`PhysicsConfig`] gets the scene's gravity, the cell's `simd_solve`
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

use std::ops::RangeInclusive;
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
use boyko_physics::solver::{DefaultRigidSolver, RigidSolver, SoftStepSolver};
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

    /// A dynamic box at rest.
    fn dynamic_box(
        position: Vec3,
        rotation: Quat,
        half_extents: Vec3,
        inv_mass: f32,
        friction: f32,
    ) -> Self {
        Self { inv_mass, ..Self::static_box(position, rotation, half_extents, friction) }
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

// ── The friction behaviour gates (G-P1, G-P2, G-P3, G-P7) ────────────────────

/// Gravity's magnitude in every G-P scene, in m/s² (the engine's default `(0, -9.81, 0)`).
const G: f64 = 9.81;
/// The default gravity, for the G-P scenes on a level floor.
const DOWN: Vec3 = Vec3::new(0.0, -9.81, 0.0);
/// The box's row in every G-P scene: the floor spawns first, at row 0.
const BOX_ROW: usize = 1;

/// A bound on one reading, fixed before any run.
#[derive(Clone, Copy, Debug)]
enum Bound {
    /// `value < hi`.
    Below(f64),
    /// `value <= hi`.
    AtMost(f64),
    /// `lo <= value <= hi`.
    Within(f64, f64),
}

/// The side of its bound a reading misses on.
#[derive(Clone, Copy, Debug)]
enum Side {
    Low,
    High,
}

/// One (cell, component) pair's reading against its bound.
#[derive(Clone, Debug)]
struct Reading {
    /// `"<cell>: <component>"`, unique within its gate.
    pair: String,
    value: f64,
    bound: Bound,
}

impl Reading {
    /// `None` when the reading meets its bound, else the side it misses on. `INFINITY` stands for
    /// "never happened" (no stop within the run) and misses high; a NaN misses high too.
    fn miss(&self) -> Option<Side> {
        let v = self.value;
        match self.bound {
            Bound::Below(hi) if v < hi => None,
            Bound::AtMost(hi) if v <= hi => None,
            Bound::Within(lo, hi) if lo <= v && v <= hi => None,
            Bound::Within(lo, _) if v < lo => Some(Side::Low),
            _ => Some(Side::High),
        }
    }

    /// The reading's verdict, for the printed record.
    fn verdict(&self) -> String {
        self.miss().map_or_else(|| "meets it".to_string(), |side| format!("misses {side:?}"))
    }
}

/// A G-P run's record of the box.
struct Trace {
    /// The box after every step; `states[0]` is the spawn state.
    states: Vec<RigidBody>,
    /// On every step, the largest point count of a manifold holding the box (0: none);
    /// `points[0]` = 0, no step ran.
    points: Vec<u8>,
    /// The first step after which some body was non-finite.
    first_non_finite: Option<usize>,
}

impl Trace {
    /// The premises every G-P gate shares: every body finite on every step, and the box holding a
    /// manifold on every step of `window`, without which friction is vacuous there.
    fn standing(&self, window: RangeInclusive<usize>) -> Result<(), String> {
        if let Some(step) = self.first_non_finite {
            return Err(format!("a body is non-finite after step {step}"));
        }
        match window.clone().find(|&step| self.points[step] == 0) {
            Some(step) => Err(format!(
                "the box holds no manifold on step {step} of its window {window:?}, so friction \
                 is vacuous there"
            )),
            None => Ok(()),
        }
    }

    /// The box after `step`.
    fn at(&self, step: usize) -> &RigidBody {
        &self.states[step]
    }
}

/// Runs `scene` for `steps` on solver `S` (W 1, `simd_solve` on) and records the box.
fn trace<S: RigidSolver + Default>(scene: &Scene, steps: usize) -> Trace {
    let mut sim = Sim::new::<S>(scene, DEFAULT_ARM);
    let mut states = Vec::with_capacity(steps + 1);
    let mut points = Vec::with_capacity(steps + 1);
    let mut first_non_finite = None;
    states.push(sim.bodies()[BOX_ROW]);
    points.push(0);
    for step in 1..=steps {
        sim.step();
        let bodies = sim.bodies();
        if first_non_finite.is_none()
            && !bodies.iter().all(|b| body_words(b).iter().all(|w| w.is_finite()))
        {
            first_non_finite = Some(step);
        }
        states.push(bodies[BOX_ROW]);
        let box_points = sim
            .manifolds()
            .iter()
            .filter(|m| m.body_a.0 as usize == BOX_ROW || m.body_b.0 as usize == BOX_ROW)
            .map(|m| m.count)
            .max()
            .unwrap_or(0);
        points.push(box_points);
    }
    Trace { states, points, first_non_finite }
}

/// Runs one G-P cell: the reference first, printed as a receipt and never asserted, then the
/// colored default, whose premises `read` asserts (a premise red is a STOP). Prints every colored
/// reading against its bound and returns them with the colored trace.
fn run_gate_cell<F>(
    gate: &str,
    cell: &str,
    scene: &Scene,
    steps: usize,
    read: F,
) -> (Vec<Reading>, Trace)
where
    F: Fn(&Trace) -> Result<Vec<Reading>, String>,
{
    match read(&trace::<SoftStepSolver>(scene, steps)) {
        Ok(readings) => {
            for r in readings {
                println!(
                    "{gate} reference receipt (SoftStepSolver): {} = {} (bound {:?}; {})",
                    r.pair,
                    r.value,
                    r.bound,
                    r.verdict()
                );
            }
        }
        Err(premise) => println!(
            "{gate} reference receipt (SoftStepSolver), {cell}: a premise fails: {premise}"
        ),
    }
    let colored = trace::<DefaultRigidSolver>(scene, steps);
    let readings = read(&colored).unwrap_or_else(|premise| {
        panic!(
            "{gate}, {cell}, premise: {premise}. The scene does not stand: a fixture or engine \
             finding. STOP; never edit a bound or the fixture to make it stand"
        )
    });
    for r in &readings {
        println!(
            "{gate}, per-point base: {} = {} (bound {:?}; {})",
            r.pair,
            r.value,
            r.bound,
            r.verdict()
        );
    }
    (readings, colored)
}

/// The plain per-point gate: every pair meets its bound.
fn assert_gate(gate: &str, readings: &[Reading]) {
    assert!(!readings.is_empty(), "harness: {gate} read no pair, so it would pass vacuously");
    let misses: Vec<String> = readings
        .iter()
        .filter(|r| r.miss().is_some())
        .map(|r| format!("{} = {} ({:?}; {})", r.pair, r.value, r.bound, r.verdict()))
        .collect();
    assert!(
        misses.is_empty(),
        "{gate}, per-point base: {} of {} pairs miss their bounds, which were fixed before any \
         run (fm/design_r2.md §8 C0): {}. The per-point base met every pair when this file was \
         written, so a miss is a change in the per-point solver; never edit a bound to follow a \
         reading",
        misses.len(),
        readings.len(),
        misses.join("; ")
    );
}

/// `tan θ`'s incline gravity, `9.81·(sin θ, −cos θ, 0)`, computed in `f64` and rounded once.
fn incline_gravity(tan: f64) -> Vec3 {
    let theta = tan.atan();
    Vec3::new((G * theta.sin()) as f32, (-G * theta.cos()) as f32, 0.0)
}

/// The analytic sliding acceleration down `tan θ` at friction `mu`: `9.81·(sin θ − μ cos θ)`.
fn incline_acceleration(mu: f64, tan: f64) -> f64 {
    let theta = tan.atan();
    G * (theta.sin() - mu * theta.cos())
}

/// A yaw of `degrees` about +y, built by hand as `(0, sin ψ/2, 0, cos ψ/2)`.
fn yaw(degrees: f64) -> Quat {
    let half = degrees.to_radians() / 2.0;
    Quat::new(0.0, half.sin() as f32, 0.0, half.cos() as f32)
}

/// `body` on the static floor, which carries the body's own friction.
fn on_floor(body: BodySpec, gravity: Vec3, dt: f32) -> Scene {
    Scene { bodies: vec![floor(body.friction), body], gravity, dt, ground: Ground::Bodies }
}

/// The horizontal distance between the box's centre after steps `from` and `to`.
fn horizontal_distance(t: &Trace, from: usize, to: usize) -> f64 {
    let (a, b) = (t.at(from).position, t.at(to).position);
    f64::from(b.x - a.x).hypot(f64::from(b.z - a.z))
}

/// The magnitude of `v`, in `f64`.
fn norm(v: Vec3) -> f64 {
    (f64::from(v.x).powi(2) + f64::from(v.y).powi(2) + f64::from(v.z).powi(2)).sqrt()
}

// G-P1: the incline threshold.

/// G-P1's friction coefficients.
const G_P1_MU: [f64; 3] = [0.2, 0.5, 0.8];
/// G-P1's yaws about +y, in degrees.
const G_P1_YAW_DEG: [f64; 2] = [0.0, 30.0];
/// G-P1's step, in seconds.
const G_P1_DT_S: f64 = 1.0 / 120.0;
/// G-P1's frames: 4 s.
const G_P1_FRAMES: usize = 480;
/// The frame that opens the stick and accel windows (the first half settles the contact).
const G_P1_HALF: usize = 240;
/// G-P1's box: tips only above `tan θ = 0.5 / 0.2 = 2.5`.
const G_P1_BOX_HALF: Vec3 = Vec3::new(0.5, 0.2, 0.5);
/// G-P1's spawn: far uphill, so the accel arm's 33.3 m at μ 0.8 stays on the floor.
const G_P1_SPAWN: Vec3 = Vec3::new(-40.0, 0.2, 0.0);
/// The stick arm's creep bound over frames 240-480, in metres.
const G_P1_CREEP_M: f64 = 0.005;
/// The slide arm's band, as multiples of `½·a·t²`.
const G_P1_SLIDE_BAND: [f64; 2] = [0.5, 1.5];
/// The accel arm's tolerance around the analytic `a`.
const G_P1_ACCEL_TOLERANCE: f64 = 0.05;

/// G-P1's three arms.
#[derive(Clone, Copy, Debug)]
enum Incline {
    /// `tan θ = 0.95μ`: inside the cone, the box must hold.
    Stick,
    /// `tan θ = 1.05μ`: just outside it, the box must slide by about `½·a·t²`.
    Slide,
    /// `tan θ = 2μ`: well outside it, the box must accelerate at `a`.
    Accel,
}

impl Incline {
    /// `tan θ / μ`.
    fn k(self) -> f64 {
        match self {
            Self::Stick => 0.95,
            Self::Slide => 1.05,
            Self::Accel => 2.0,
        }
    }
}

/// G-P1's scene for one cell of `arm`.
fn g_p1_scene(arm: Incline, mu: f64, yaw_deg: f64) -> Scene {
    let body = BodySpec::dynamic_box(G_P1_SPAWN, yaw(yaw_deg), G_P1_BOX_HALF, 1.0, mu as f32);
    on_floor(body, incline_gravity(arm.k() * mu), G_P1_DT_S as f32)
}

/// G-P1's readings for `arm`: one pair per (μ, yaw) cell.
fn g_p1_readings(arm: Incline) -> Vec<Reading> {
    let gate = format!("G-P1 {arm:?}");
    let mut out = Vec::new();
    for mu in G_P1_MU {
        for yaw_deg in G_P1_YAW_DEG {
            let cell = format!("μ {mu}, yaw {yaw_deg}°");
            let a = incline_acceleration(mu, arm.k() * mu);
            let scene = g_p1_scene(arm, mu, yaw_deg);
            let (readings, _) = run_gate_cell(&gate, &cell, &scene, G_P1_FRAMES, |t| match arm {
                Incline::Stick => {
                    t.standing(G_P1_HALF + 1..=G_P1_FRAMES)?;
                    Ok(vec![Reading {
                        pair: format!("{cell}: creep"),
                        value: horizontal_distance(t, G_P1_HALF, G_P1_FRAMES),
                        bound: Bound::Below(G_P1_CREEP_M),
                    }])
                }
                Incline::Slide => {
                    t.standing(1..=G_P1_FRAMES)?;
                    let time = G_P1_FRAMES as f64 * G_P1_DT_S;
                    let expected = 0.5 * a * time * time;
                    Ok(vec![Reading {
                        pair: format!("{cell}: displacement"),
                        value: f64::from(t.at(G_P1_FRAMES).position.x - t.at(0).position.x),
                        bound: Bound::Within(
                            G_P1_SLIDE_BAND[0] * expected,
                            G_P1_SLIDE_BAND[1] * expected,
                        ),
                    }])
                }
                Incline::Accel => {
                    t.standing(G_P1_HALF + 1..=G_P1_FRAMES)?;
                    let dv = t.at(G_P1_FRAMES).linear_velocity.x
                        - t.at(G_P1_HALF).linear_velocity.x;
                    Ok(vec![Reading {
                        pair: format!("{cell}: acceleration"),
                        value: f64::from(dv) / ((G_P1_FRAMES - G_P1_HALF) as f64 * G_P1_DT_S),
                        bound: Bound::Within(
                            a * (1.0 - G_P1_ACCEL_TOLERANCE),
                            a * (1.0 + G_P1_ACCEL_TOLERANCE),
                        ),
                    }])
                }
            });
            out.extend(readings);
        }
    }
    out
}

/// G-P1, stick arm: inside the friction cone (`tan θ = 0.95μ`) the box holds.
#[test]
fn g_p1_incline_stick_per_point() {
    assert_gate("G-P1 Stick", &g_p1_readings(Incline::Stick));
}

/// G-P1, slide arm: just outside the cone (`tan θ = 1.05μ`) the box slides about `½·a·t²`.
#[test]
fn g_p1_incline_slide_per_point() {
    assert_gate("G-P1 Slide", &g_p1_readings(Incline::Slide));
}

/// G-P1, accel arm: well outside the cone (`tan θ = 2μ`) the box accelerates at `a` ±5 %.
#[test]
fn g_p1_incline_accel_per_point() {
    assert_gate("G-P1 Accel", &g_p1_readings(Incline::Accel));
}

// G-P2: the twist stop.

/// G-P2's box: 2 m × 0.5 m × 2 m.
const G_P2_BOX_HALF: Vec3 = Vec3::new(1.0, 0.25, 1.0);
/// G-P2's inverse mass: 8 kg.
const G_P2_INV_MASS: f32 = 0.125;
/// G-P2's friction coefficient.
const G_P2_MU: f64 = 0.2;
/// G-P2's initial spin about +y, in rad/s.
const G_P2_OMEGA0: f32 = 2.0;
/// G-P2's step, in seconds.
const G_P2_DT_S: f64 = 1.0 / 60.0;
/// G-P2's run length, the last step a stop may be found on.
const G_P2_STEPS: usize = 300;
/// `|ω|` below this, in rad/s, is at rest.
const G_P2_REST: f64 = 1.0e-3;
/// The steps after the stop that must stay at rest.
const G_P2_HOLD_STEPS: usize = 60;
/// The stop step's band (analytic 28.8 steps).
const G_P2_STOP_BAND: [f64; 2] = [25.0, 33.0];
/// The COM drift bound, in metres (design OQ1).
const G_P2_DRIFT_M: f64 = 0.001;

/// G-P2's scene.
fn g_p2_scene() -> Scene {
    let mut body = BodySpec::dynamic_box(
        Vec3::new(0.0, G_P2_BOX_HALF.y, 0.0),
        Quat::IDENTITY,
        G_P2_BOX_HALF,
        G_P2_INV_MASS,
        G_P2_MU as f32,
    );
    body.angular_velocity = Vec3::new(0.0, G_P2_OMEGA0, 0.0);
    on_floor(body, DOWN, G_P2_DT_S as f32)
}

/// G-P2's readings: the stop step `s*` (`INFINITY` if none within the run), the largest `|ω|`
/// over the [`G_P2_HOLD_STEPS`] after it (`INFINITY` if they do not fit in the run), and the COM
/// drift at `s* + 60` (at the last step if that does not fit).
fn g_p2_readings() -> Vec<Reading> {
    let (readings, _) = run_gate_cell("G-P2", "twist", &g_p2_scene(), G_P2_STEPS, |t| {
        if t.points[1] != 4 {
            return Err(format!(
                "the box-floor manifold holds {} points at step 1, not the 4 corners the analytic \
                 stop assumes",
                t.points[1]
            ));
        }
        let spin = |step: usize| norm(t.at(step).angular_velocity);
        let stop = (1..=G_P2_STEPS).find(|&step| spin(step) < G_P2_REST);
        let held = stop.filter(|&s| s + G_P2_HOLD_STEPS <= G_P2_STEPS);
        let read_at = held.map_or(G_P2_STEPS, |s| s + G_P2_HOLD_STEPS);
        t.standing(1..=read_at)?;
        Ok(vec![
            Reading {
                pair: "twist: stop step".to_string(),
                value: stop.map_or(f64::INFINITY, |s| s as f64),
                bound: Bound::Within(G_P2_STOP_BAND[0], G_P2_STOP_BAND[1]),
            },
            Reading {
                pair: "twist: largest |ω| over the 60 steps after the stop".to_string(),
                value: held.map_or(f64::INFINITY, |s| {
                    (s + 1..=s + G_P2_HOLD_STEPS).map(spin).fold(0.0, f64::max)
                }),
                bound: Bound::Below(G_P2_REST),
            },
            Reading {
                pair: "twist: COM drift".to_string(),
                value: horizontal_distance(t, 0, read_at),
                bound: Bound::AtMost(G_P2_DRIFT_M),
            },
        ])
    });
    readings
}

/// G-P2, stop and hold: a box spun in place on the floor stops inside the analytic band and stays
/// stopped.
#[test]
fn g_p2_twist_stops_and_holds_per_point() {
    let readings: Vec<Reading> =
        g_p2_readings().into_iter().filter(|r| !r.pair.ends_with("COM drift")).collect();
    assert_gate("G-P2 stop", &readings);
}

/// G-P2, drift: the spinning box's centre does not wander (design OQ1).
#[test]
fn g_p2_twist_drift_per_point() {
    let readings: Vec<Reading> =
        g_p2_readings().into_iter().filter(|r| r.pair.ends_with("COM drift")).collect();
    assert_gate("G-P2 drift", &readings);
}

// G-P3: yaw while sliding.

/// G-P3's box.
const G_P3_BOX_HALF: Vec3 = Vec3::new(0.5, 0.25, 0.5);
/// G-P3's friction coefficient.
const G_P3_MU: f64 = 0.3;
/// G-P3's yaw, in degrees.
const G_P3_YAW_DEG: f64 = 30.0;
/// G-P3's launch speed along +x, in m/s.
const G_P3_V0: f64 = 3.0;
/// G-P3's step, in seconds.
const G_P3_DT_S: f64 = 1.0 / 60.0;
/// G-P3's run: the analytic stop is at 61.2 steps.
const G_P3_STEPS: usize = 180;
/// The heading-change bound, in degrees.
const G_P3_HEADING_DEG: f64 = 0.5;
/// The lateral-displacement bound, in metres.
const G_P3_LATERAL_M: f64 = 0.01;
/// The stop distance's tolerance around `v₀² / (2μg)`.
const G_P3_STOP_TOLERANCE: f64 = 0.05;
/// The premise's rest speed at the last step, in m/s.
const G_P3_REST: f64 = 1.0e-3;

/// G-P3's scene.
fn g_p3_scene() -> Scene {
    let mut body = BodySpec::dynamic_box(
        Vec3::new(0.0, G_P3_BOX_HALF.y, 0.0),
        yaw(G_P3_YAW_DEG),
        G_P3_BOX_HALF,
        1.0,
        G_P3_MU as f32,
    );
    body.linear_velocity = Vec3::new(G_P3_V0 as f32, 0.0, 0.0);
    on_floor(body, DOWN, G_P3_DT_S as f32)
}

/// The heading of a box's local +x, `atan2(−e.z, e.x)` with `e = R·x̂`, in degrees.
fn heading_deg(rotation: Quat) -> f64 {
    let e = rotation.rotate(Vec3::new(1.0, 0.0, 0.0));
    f64::from(-e.z).atan2(f64::from(e.x)).to_degrees()
}

/// G-P3's readings: the heading change, the lateral displacement and the stop distance.
fn g_p3_readings() -> Vec<Reading> {
    let (readings, _) = run_gate_cell("G-P3", "slide", &g_p3_scene(), G_P3_STEPS, |t| {
        t.standing(1..=G_P3_STEPS)?;
        let speed = norm(t.at(G_P3_STEPS).linear_velocity);
        if speed >= G_P3_REST {
            return Err(format!(
                "the box has not stopped by step {G_P3_STEPS}: |v| = {speed} m/s (premise < \
                 {G_P3_REST})"
            ));
        }
        let turn = heading_deg(t.at(G_P3_STEPS).rotation) - heading_deg(t.at(0).rotation);
        let expected = G_P3_V0 * G_P3_V0 / (2.0 * G_P3_MU * G);
        Ok(vec![
            Reading {
                pair: "slide: heading change (deg)".to_string(),
                value: ((turn + 540.0).rem_euclid(360.0) - 180.0).abs(),
                bound: Bound::AtMost(G_P3_HEADING_DEG),
            },
            Reading {
                pair: "slide: lateral displacement".to_string(),
                value: f64::from(t.at(G_P3_STEPS).position.z - t.at(0).position.z).abs(),
                bound: Bound::AtMost(G_P3_LATERAL_M),
            },
            Reading {
                pair: "slide: stop distance".to_string(),
                value: f64::from(t.at(G_P3_STEPS).position.x - t.at(0).position.x),
                bound: Bound::Within(
                    expected * (1.0 - G_P3_STOP_TOLERANCE),
                    expected * (1.0 + G_P3_STOP_TOLERANCE),
                ),
            },
        ])
    });
    readings
}

/// G-P3 (Jolt #983): a yawed box launched along +x slides straight and stops at `v₀² / (2μg)`.
#[test]
fn g_p3_yaw_while_sliding_per_point() {
    assert_gate("G-P3", &g_p3_readings());
}

// G-P7: the single-box slide.

/// S-SLIDE's gravity, `9.81·(1, −2, 0) / √5`, as the literal bits the dynamic parity scenes run
/// (`benches/jolt_parity_pyramid/dyn_spec.rs`, `SLIDE_GRAVITY_BITS`): `tan θ = ½`.
const G_P7_GRAVITY_BITS: [u32; 3] = [0x408c_63a9, 0xc10c_63a9, 0];
/// G-P7's friction coefficient (S-SLIDE's).
const G_P7_MU: f64 = 0.2;
/// The J-T box: half-extent 1.
const G_P7_BOX_HALF: Vec3 = Vec3::new(1.0, 1.0, 1.0);
/// The J-T box's inverse mass: 8 kg.
const G_P7_INV_MASS: f32 = 0.125;
/// G-P7's step, in seconds.
const G_P7_DT_S: f64 = 1.0 / 60.0;
/// The step that opens G-P7's window.
const G_P7_FROM: usize = 10;
/// G-P7's last step: a 60-step (1 s) window.
const G_P7_TO: usize = 70;
/// The acceleration's tolerance around the analytic value.
const G_P7_TOLERANCE: f64 = 0.03;
/// The premise's largest tilt from the spawn orientation, in degrees.
const G_P7_TILT_DEG: f64 = 1.0;

/// G-P7's scene.
fn g_p7_scene() -> Scene {
    let body = BodySpec::dynamic_box(
        Vec3::new(0.0, G_P7_BOX_HALF.y, 0.0),
        Quat::IDENTITY,
        G_P7_BOX_HALF,
        G_P7_INV_MASS,
        G_P7_MU as f32,
    );
    let [gx, gy, gz] = G_P7_GRAVITY_BITS.map(f32::from_bits);
    on_floor(body, Vec3::new(gx, gy, gz), G_P7_DT_S as f32)
}

/// The angle of `rotation` away from the identity, in degrees.
fn tilt_deg(rotation: Quat) -> f64 {
    let (x, y, z, w) = (
        f64::from(rotation.x),
        f64::from(rotation.y),
        f64::from(rotation.z),
        f64::from(rotation.w),
    );
    let cos_half = (w.abs() / (x * x + y * y + z * z + w * w).sqrt()).min(1.0);
    2.0 * cos_half.acos().to_degrees()
}

/// G-P7's reading, and the displacement-form receipt read from the same colored trace.
fn g_p7_reading() -> (Vec<Reading>, f64) {
    let window = (G_P7_TO - G_P7_FROM) as f64 * G_P7_DT_S;
    let theta = 0.5f64.atan();
    let expected = G * (theta.sin() - G_P7_MU * theta.cos());
    let (readings, t) = run_gate_cell("G-P7", "slide", &g_p7_scene(), G_P7_TO, |t| {
        t.standing(1..=G_P7_TO)?;
        let worst = (0..=G_P7_TO).map(|s| tilt_deg(t.at(s).rotation)).fold(0.0, f64::max);
        if worst > G_P7_TILT_DEG {
            return Err(format!(
                "the box tilted {worst}° from its spawn orientation (premise <= {G_P7_TILT_DEG}°)"
            ));
        }
        let dv = t.at(G_P7_TO).linear_velocity.x - t.at(G_P7_FROM).linear_velocity.x;
        Ok(vec![Reading {
            pair: "slide: acceleration".to_string(),
            value: f64::from(dv) / window,
            bound: Bound::Within(
                expected * (1.0 - G_P7_TOLERANCE),
                expected * (1.0 + G_P7_TOLERANCE),
            ),
        }])
    });
    let dx = f64::from(t.at(G_P7_TO).position.x - t.at(G_P7_FROM).position.x);
    let v_from = f64::from(t.at(G_P7_FROM).linear_velocity.x);
    (readings, 2.0 * (dx - v_from * window) / (window * window))
}

/// G-P7: one J-T box under S-SLIDE's incline gravity accelerates at `9.81·(sin θ − 0.2 cos θ)` =
/// 2.632 m/s² ±3 %.
#[test]
fn g_p7_single_box_slide_per_point() {
    let (readings, displacement_form) = g_p7_reading();
    println!(
        "G-P7, per-point base: displacement-form receipt 2·(x(70) − x(10) − v_x(10)·1 s) = \
         {displacement_form} m/s² (biased by +1/N under symplectic Euler; not the gate)"
    );
    assert_gate("G-P7", &readings);
}
