//! **V2's overlap-only rule (`speculative_distance = 0` AND `speculative_velocity_cap = 0`) is
//! the pre-V2 engine, bit for bit — on every pair type.**
//!
//! V2 (owner V2a / V2b, 2026-09-30) changes the contact rule on every pair type the narrowphase
//! has: box-box, sphere-sphere, sphere-box in both row orders, sphere and box against the SDF
//! field, and the sensor and kinematic cases that ride on them; the solvers gain a speculative
//! branch. Its promise is that `PhysicsConfig::speculative_distance = 0` together with
//! `speculative_velocity_cap = 0` reproduces the rule from before V2 exactly; the distance at 0
//! alone does not, since the approach-velocity margin (default cap 0.5 m) still keeps and solves
//! speculative points. The pinned box piles (`default_world_pyramid_determinism.rs`, A7-R1,
//! GOLDEN, the parity runner's pose gates) witness that for boxes; this file is the other half: one
//! small scene per pair type, run through the real schedule, with every step's poses, the solver's
//! manifold stream and the sensor-overlap stream folded into one FNV-1a 64 hash per scene, pinned.
//!
//! The pins were read on the pre-V2 parent (`16191fda`, msvc) and never move: every later commit
//! sets both `speculative_distance = 0` and `speculative_velocity_cap = 0` explicitly and must
//! keep them. A red here is a leak of the new rule into the overlap-only rule, never a re-pin.
//!
//! Each scene runs on the colored solver at one worker and at eight (the {1, N} identity makes
//! them one pin), and the box-stack scene also on the reference `SoftStepSolver`. Every scene
//! asserts that its pair type actually produced contacts (or overlaps), so a scene whose bodies
//! stopped touching cannot keep its pin by pinning nothing.

#![cfg(not(miri))]

use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, Kinematic, RigidBody, RigidBodyMass, Sensor, Simulated,
};
use boyko_physics::manifold::{Manifold, SDF_SENTINEL};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::{add_physics_sdf, add_physics_systems};
use boyko_physics::resources::{
    Manifolds, PhysicsConfig, SdfNarrowphaseKernel, SolverScratch,
};
use boyko_physics::sdf_query::SdfField;
use boyko_physics::solver::{DefaultRigidSolver, SoftStepSolver};

use boyko_sdf_math::{SdfEdit, sdf_op};

/// Fixed timestep.
const DT: f32 = 1.0 / 60.0;
/// Steps per scene.
const STEPS: usize = 120;
/// FNV-1a 64 offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64 prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// How a body takes part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// `Simulated`, `inv_mass > 0`.
    Dynamic,
    /// Neither bit, `inv_mass == 0`.
    Static,
    /// `Kinematic` set, `inv_mass == 0`: its velocity feeds the contacts, its pose stays.
    Kinematic,
    /// A static `Sensor`.
    StaticSensor,
    /// A dynamic `Sensor`.
    DynamicSensor,
}

/// One body of a scene.
#[derive(Clone, Copy, Debug)]
struct Spec {
    shape: ColliderShape,
    position: Vec3,
    velocity: Vec3,
    rotation: Quat,
    restitution: f32,
    role: Role,
}

impl Spec {
    fn sphere(radius: f32, position: Vec3, role: Role) -> Self {
        Self {
            shape: ColliderShape::Sphere { radius },
            position,
            velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            restitution: 0.0,
            role,
        }
    }

    fn cuboid(half: Vec3, position: Vec3, role: Role) -> Self {
        Self {
            shape: ColliderShape::Box { half_extents: half },
            position,
            velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            restitution: 0.0,
            role,
        }
    }

    fn moving(mut self, velocity: Vec3) -> Self {
        self.velocity = velocity;
        self
    }

    fn turned(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    fn bouncy(mut self, restitution: f32) -> Self {
        self.restitution = restitution;
        self
    }
}

/// Which pipeline a scene runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pipe {
    /// `add_physics_systems::<DefaultRigidSolver>`: the colored solve.
    Colored,
    /// `add_physics_systems::<SoftStepSolver>`: the reference solve.
    Reference,
    /// `add_physics_sdf::<DefaultRigidSolver>` on an SDF floor, with this box kernel.
    ColoredSdf(SdfNarrowphaseKernel),
}

/// What a scene's streams held over the run, for its anti-vacuity check.
#[derive(Clone, Copy, Debug, Default)]
struct Seen {
    sphere_sphere: u64,
    sphere_box: u64,
    box_sphere: u64,
    box_box: u64,
    sdf_sphere: u64,
    sdf_box: u64,
    kinematic: u64,
    sensor_overlaps: u64,
}

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its `size_of::<T>()` bytes
    // read-only for the duration of the borrow, which is the exact layout the component pool
    // stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Spawns `spec` into the plain or the sensor archetype.
fn spawn(world: &mut EcsMaster, spec: &Spec) {
    let dynamic = matches!(spec.role, Role::Dynamic | Role::DynamicSensor);
    let body = RigidBody {
        position: spec.position,
        linear_velocity: spec.velocity,
        rotation: spec.rotation,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: if dynamic { Mat3::from_diagonal(Vec3::new(1.0, 1.0, 1.0)) } else { Mat3::ZERO },
        inv_mass: if dynamic { 1.0 } else { 0.0 },
        restitution: spec.restitution,
        friction: 0.4,
    };
    let collider = Collider { shape: spec.shape, layer: 1, mask: 1 };
    let base = [RigidBody::component_id(), RigidBodyMass::component_id(), Collider::component_id()];
    let columns = [
        (base[0], as_bytes(&body)),
        (base[1], as_bytes(&mass)),
        (base[2], as_bytes(&collider)),
    ];
    let sensor = Sensor;
    let e = if matches!(spec.role, Role::StaticSensor | Role::DynamicSensor) {
        let archetype = world.create_archetype(&[base[0], base[1], base[2], Sensor::component_id()]);
        world.create_entity(
            archetype,
            &[columns[0], columns[1], columns[2], (Sensor::component_id(), as_bytes(&sensor))],
        )
    } else {
        let archetype = world.create_archetype(&base);
        world.create_entity(archetype, &columns)
    }
    .expect("construction: every archetype accepts its columns");
    if dynamic {
        world.enable::<Simulated>(e);
    }
    if spec.role == Role::Kinematic {
        world.enable::<Kinematic>(e);
    }
}

/// An SDF floor whose top face is `y = 0`.
fn sdf_floor() -> SdfField {
    SdfField::from_edits(&[SdfEdit::box_shape([0.0, -20.0, 0.0], [20.0, 20.0, 20.0], sdf_op::UNION, 0.0)])
}

/// Folds `v`'s bits into `h`.
fn fold(h: u64, v: u32) -> u64 {
    v.to_le_bytes().iter().fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(FNV_PRIME))
}

/// Folds one manifold stream: its length, then each manifold's rows, normal and points.
fn fold_stream(mut h: u64, stream: &[Manifold]) -> u64 {
    h = fold(h, stream.len() as u32);
    for m in stream {
        h = fold(h, m.body_a.0);
        h = fold(h, m.body_b.0);
        h = fold(h, u32::from(m.count));
        for c in [m.normal.x, m.normal.y, m.normal.z] {
            h = fold(h, c.to_bits());
        }
        for p in &m.points[..usize::from(m.count)] {
            h = fold(h, p.separation.to_bits());
            h = fold(h, p.feature_id);
            for c in [p.anchor_a.x, p.anchor_a.y, p.anchor_a.z, p.anchor_b.x, p.anchor_b.y, p.anchor_b.z] {
                h = fold(h, c.to_bits());
            }
        }
    }
    h
}

/// Counts what the step's streams hold, by the gathered rows' shapes and bits.
fn census(world: &EcsMaster, seen: &mut Seen) {
    let bodies = world.resource::<SolverScratch>().bodies();
    let manifolds = world.resource::<Manifolds>();
    for m in manifolds.solver_manifolds() {
        let a = &bodies[m.body_a.0 as usize];
        if m.body_b == SDF_SENTINEL {
            match a.shape {
                ColliderShape::Sphere { .. } => seen.sdf_sphere += 1,
                ColliderShape::Box { .. } => seen.sdf_box += 1,
            }
            continue;
        }
        let b = &bodies[m.body_b.0 as usize];
        match (a.shape, b.shape) {
            (ColliderShape::Sphere { .. }, ColliderShape::Sphere { .. }) => seen.sphere_sphere += 1,
            (ColliderShape::Sphere { .. }, ColliderShape::Box { .. }) => seen.sphere_box += 1,
            (ColliderShape::Box { .. }, ColliderShape::Sphere { .. }) => seen.box_sphere += 1,
            (ColliderShape::Box { .. }, ColliderShape::Box { .. }) => seen.box_box += 1,
        }
        seen.kinematic += u64::from(a.kinematic || b.kinematic);
    }
    seen.sensor_overlaps += manifolds.sensor_overlaps().len() as u64;
}

/// Runs `specs` on `pipe` at `workers` for [`STEPS`] steps: the hash of every step's poses, solver
/// manifolds and sensor overlaps, and what the streams held.
fn run(specs: &[Spec], pipe: Pipe, workers: usize) -> (u64, Seen) {
    let mut world = EcsMaster::new();
    for spec in specs {
        spawn(&mut world, spec);
    }
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(workers).build());
    match pipe {
        Pipe::Colored => {
            add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
        }
        Pipe::Reference => {
            add_physics_systems::<SoftStepSolver>(&mut builder, &mut world);
        }
        Pipe::ColoredSdf(_) => {
            add_physics_sdf::<DefaultRigidSolver>(&mut builder, &mut world);
            *world.resource_mut::<SdfField>() = sdf_floor();
        }
    }
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    let mut schedule = builder.build(&mut world);
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.dt = DT;
        // The rule this file pins: V2 at `d = 0` with the approach-velocity margin off (the
        // overlap-only rule), set explicitly so the pins do not follow the defaults (20 mm and a
        // 0.5 m cap from V2's value-changing commit on).
        cfg.speculative_distance = 0.0;
        cfg.speculative_velocity_cap = 0.0;
        if let Pipe::ColoredSdf(kernel) = pipe {
            cfg.sdf_narrowphase = kernel;
        }
    }
    let mut h = FNV_OFFSET;
    let mut seen = Seen::default();
    for _ in 0..STEPS {
        schedule.run(&mut world);
        let q = world.query::<&RigidBody, ()>();
        for b in q.iter() {
            for c in [
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
                h = fold(h, c.to_bits());
            }
        }
        let manifolds = world.resource::<Manifolds>();
        h = fold_stream(h, manifolds.solver_manifolds());
        h = fold_stream(h, manifolds.sensor_overlaps());
        census(&world, &mut seen);
    }
    (h, seen)
}

/// Runs `specs` on `pipe` at one worker and at eight (colored pipelines), asserts the two agree
/// and equal `pin`, and returns what the streams held.
fn assert_pinned(name: &str, specs: &[Spec], pipe: Pipe, pin: u64) -> Seen {
    let (h1, seen) = run(specs, pipe, 1);
    let mut hashes = vec![("W1", h1)];
    if pipe != Pipe::Reference {
        let (h8, _) = run(specs, pipe, 8);
        hashes.push(("W8", h8));
    }
    println!("{name} ({pipe:?}): {hashes:x?}, {seen:?}");
    for (label, h) in &hashes {
        assert_eq!(
            *h, pin,
            "{name} ({pipe:?}) {label}: hash {h:#018x}, pinned {pin:#018x} — the scene's poses, \
             manifolds or overlaps moved under the overlap-only rule (speculative_distance = 0, \
             speculative_velocity_cap = 0), which must reproduce the pre-V2 rule exactly; a leak \
             of the new rule, never a re-pin"
        );
    }
    seen
}

/// A static floor box with its top face at `y = 0`.
fn floor() -> Spec {
    Spec::cuboid(Vec3::new(10.0, 1.0, 10.0), Vec3::new(0.0, -1.0, 0.0), Role::Static)
}

/// A small rotation about the axis `(x, y, z)` (normalised here) by `angle` radians.
fn about(x: f32, y: f32, z: f32, angle: f32) -> Quat {
    let axis = Vec3::new(x, y, z).normalize();
    let (s, c) = (0.5 * angle).sin_cos();
    Quat::new(axis.x * s, axis.y * s, axis.z * s, c)
}

/// Sphere-sphere: a column dropped on a static sphere, one side impact, one bouncy sphere.
#[test]
fn sphere_sphere_at_d0_is_the_pre_v2_rule() {
    let specs = [
        Spec::sphere(2.0, Vec3::ZERO, Role::Static),
        Spec::sphere(0.5, Vec3::new(0.3, 3.0, 0.0), Role::Dynamic),
        Spec::sphere(0.5, Vec3::new(-0.2, 4.5, 0.1), Role::Dynamic).bouncy(0.5),
        Spec::sphere(0.5, Vec3::new(0.1, 6.0, -0.2), Role::Dynamic),
        Spec::sphere(0.5, Vec3::new(3.5, 2.2, 0.0), Role::Dynamic).moving(Vec3::new(-4.0, 0.0, 0.0)),
    ];
    let seen = assert_pinned("sphere-sphere", &specs, Pipe::Colored, PIN_SPHERE_SPHERE);
    assert!(seen.sphere_sphere > 0, "anti-vacuity: the scene made sphere-sphere contacts");
}

/// Sphere-box with the sphere on the lower row: spheres dropped on a floor and a dynamic box.
#[test]
fn sphere_box_at_d0_is_the_pre_v2_rule() {
    let specs = [
        Spec::sphere(0.5, Vec3::new(0.0, 2.0, 0.0), Role::Dynamic),
        Spec::sphere(0.4, Vec3::new(1.3, 3.5, 0.2), Role::Dynamic).bouncy(0.6),
        Spec::sphere(0.3, Vec3::new(-1.5, 1.0, 0.0), Role::Dynamic).moving(Vec3::new(2.0, -1.0, 0.0)),
        floor(),
        Spec::cuboid(Vec3::new(0.5, 0.5, 0.5), Vec3::new(1.2, 0.6, 0.0), Role::Dynamic),
    ];
    let seen = assert_pinned("sphere-box", &specs, Pipe::Colored, PIN_SPHERE_BOX);
    assert!(seen.sphere_box > 0, "anti-vacuity: the scene made sphere-box contacts");
}

/// Box-sphere (the flipped arm): the box on the lower row — a floor and boxes spawned before the
/// spheres, and a dynamic box landing on a static sphere spawned after it.
#[test]
fn box_sphere_at_d0_is_the_pre_v2_rule() {
    let specs = [
        floor(),
        Spec::cuboid(Vec3::new(0.5, 0.5, 0.5), Vec3::new(4.0, 2.4, 0.0), Role::Dynamic)
            .turned(about(1.0, 0.0, 1.0, 0.3)),
        Spec::sphere(0.5, Vec3::new(0.0, 1.5, 0.0), Role::Dynamic).bouncy(0.4),
        Spec::sphere(0.5, Vec3::new(-1.0, 3.0, 0.3), Role::Dynamic),
        Spec::sphere(0.8, Vec3::new(4.0, 0.8, 0.0), Role::Static),
    ];
    let seen = assert_pinned("box-sphere", &specs, Pipe::Colored, PIN_BOX_SPHERE);
    assert!(seen.box_sphere > 0, "anti-vacuity: the scene made box-sphere contacts");
}

/// Spheres on the SDF floor.
#[test]
fn sphere_sdf_at_d0_is_the_pre_v2_rule() {
    let specs = [
        Spec::sphere(0.5, Vec3::new(0.0, 1.5, 0.0), Role::Dynamic),
        Spec::sphere(0.5, Vec3::new(2.0, 0.6, 0.0), Role::Dynamic).moving(Vec3::new(3.0, -2.0, 0.0)),
        Spec::sphere(0.3, Vec3::new(-2.0, 2.5, 1.0), Role::Dynamic).bouncy(0.5),
    ];
    let seen = assert_pinned(
        "sphere-sdf",
        &specs,
        Pipe::ColoredSdf(SdfNarrowphaseKernel::Scalar),
        PIN_SPHERE_SDF,
    );
    assert!(seen.sdf_sphere > 0, "anti-vacuity: the scene made sphere-SDF contacts");
}

/// Tilted boxes on the SDF floor.
fn box_sdf_specs() -> [Spec; 3] {
    [
        Spec::cuboid(Vec3::new(0.5, 0.5, 0.5), Vec3::new(0.0, 1.5, 0.0), Role::Dynamic)
            .turned(about(1.0, 0.0, 0.5, 0.4)),
        Spec::cuboid(Vec3::new(0.6, 0.3, 0.4), Vec3::new(2.5, 1.0, 0.0), Role::Dynamic)
            .turned(about(0.0, 0.0, 1.0, 0.2))
            .moving(Vec3::new(-1.0, -3.0, 0.0)),
        Spec::cuboid(Vec3::new(0.5, 0.5, 0.5), Vec3::new(-2.0, 0.6, 1.0), Role::Dynamic),
    ]
}

/// Boxes on the SDF floor, the scalar kernel (the default).
#[test]
fn box_sdf_scalar_at_d0_is_the_pre_v2_rule() {
    let seen = assert_pinned(
        "box-sdf scalar",
        &box_sdf_specs(),
        Pipe::ColoredSdf(SdfNarrowphaseKernel::Scalar),
        PIN_BOX_SDF_SCALAR,
    );
    assert!(seen.sdf_box > 0, "anti-vacuity: the scene made box-SDF contacts");
}

/// Boxes on the SDF floor, the AVX2 kernel (O9).
#[test]
fn box_sdf_avx2_at_d0_is_the_pre_v2_rule() {
    let seen = assert_pinned(
        "box-sdf avx2",
        &box_sdf_specs(),
        Pipe::ColoredSdf(SdfNarrowphaseKernel::Avx2),
        PIN_BOX_SDF_AVX2,
    );
    assert!(seen.sdf_box > 0, "anti-vacuity: the scene made box-SDF contacts");
}

/// Sensors: a static sensor box a dynamic box falls through, and a dynamic sensor sphere falling
/// onto the floor; the overlaps are reported, never solved.
#[test]
fn sensor_at_d0_is_the_pre_v2_rule() {
    let specs = [
        floor(),
        Spec::cuboid(Vec3::new(1.0, 0.25, 1.0), Vec3::new(0.0, 1.5, 0.0), Role::StaticSensor),
        Spec::cuboid(Vec3::new(0.4, 0.4, 0.4), Vec3::new(0.2, 3.0, 0.0), Role::Dynamic),
        Spec::sphere(0.4, Vec3::new(2.5, 1.2, 0.0), Role::DynamicSensor),
    ];
    let seen = assert_pinned("sensor", &specs, Pipe::Colored, PIN_SENSOR);
    assert!(seen.sensor_overlaps > 0, "anti-vacuity: the scene reported sensor overlaps");
}

/// A kinematic pusher: a kinematic box with a velocity against resting dynamic boxes.
#[test]
fn kinematic_at_d0_is_the_pre_v2_rule() {
    let specs = [
        floor(),
        Spec::cuboid(Vec3::new(0.5, 0.5, 0.5), Vec3::new(-1.0, 0.5, 0.0), Role::Kinematic)
            .moving(Vec3::new(1.5, 0.0, 0.0)),
        Spec::cuboid(Vec3::new(0.5, 0.5, 0.5), Vec3::new(0.0, 0.5, 0.0), Role::Dynamic),
        Spec::cuboid(Vec3::new(0.5, 0.5, 0.5), Vec3::new(0.2, 1.5, 0.0), Role::Dynamic),
    ];
    let seen = assert_pinned("kinematic", &specs, Pipe::Colored, PIN_KINEMATIC);
    assert!(seen.kinematic > 0, "anti-vacuity: the scene made contacts with the kinematic body");
}

/// A small tilted box stack.
fn box_stack_specs() -> [Spec; 4] {
    [
        floor(),
        Spec::cuboid(Vec3::new(0.5, 0.5, 0.5), Vec3::new(0.0, 0.55, 0.0), Role::Dynamic),
        Spec::cuboid(Vec3::new(0.5, 0.5, 0.5), Vec3::new(0.2, 1.7, 0.1), Role::Dynamic)
            .turned(about(0.3, 1.0, 0.2, 0.35)),
        Spec::cuboid(Vec3::new(0.4, 0.3, 0.6), Vec3::new(-0.1, 3.0, 0.0), Role::Dynamic)
            .turned(about(1.0, 0.0, 0.0, 0.15)),
    ]
}

/// The box stack on the colored solver.
#[test]
fn box_stack_colored_at_d0_is_the_pre_v2_rule() {
    let seen = assert_pinned("box-stack colored", &box_stack_specs(), Pipe::Colored, PIN_BOX_STACK_COLORED);
    assert!(seen.box_box > 0, "anti-vacuity: the scene made box-box contacts");
}

/// The box stack on the reference solver.
#[test]
fn box_stack_reference_at_d0_is_the_pre_v2_rule() {
    let seen =
        assert_pinned("box-stack reference", &box_stack_specs(), Pipe::Reference, PIN_BOX_STACK_REFERENCE);
    assert!(seen.box_box > 0, "anti-vacuity: the scene made box-box contacts");
}

/// Sphere-sphere pin.
const PIN_SPHERE_SPHERE: u64 = 0x516c_fc17_f467_4dc0;
/// Sphere-box pin.
const PIN_SPHERE_BOX: u64 = 0x7529_52df_f59a_3970;
/// Box-sphere pin.
const PIN_BOX_SPHERE: u64 = 0x9482_d692_9b66_a657;
/// Sphere-SDF pin.
const PIN_SPHERE_SDF: u64 = 0x6c25_08ac_26ba_dd94;
/// Box-SDF (scalar kernel) pin.
const PIN_BOX_SDF_SCALAR: u64 = 0x861a_7907_34be_9aa0;
/// Box-SDF (AVX2 kernel) pin.
const PIN_BOX_SDF_AVX2: u64 = 0x861a_7907_34be_9aa0;
/// Sensor pin.
const PIN_SENSOR: u64 = 0x870a_4b3f_4a0c_7133;
/// Kinematic pin.
const PIN_KINEMATIC: u64 = 0xcd65_248a_93ad_61a6;
/// Box stack, colored solver.
const PIN_BOX_STACK_COLORED: u64 = 0x5da7_9fa1_0ed4_04d1;
/// Box stack, reference solver.
const PIN_BOX_STACK_REFERENCE: u64 = 0x4415_2a12_6664_5a91;
