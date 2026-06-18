//! W5 acceptance tests for CPU SDF-collision queries (`SdfField` + the C1
//! sentinel solver path).
//!
//! These prove the body-vs-SDF narrowphase + the C1 sentinel solve:
//!
//! - `sample_sdf` unit tests pin the physics wrapper + `Vec3` ↔ `[f32; 3]`
//!   conversion against hand-computed sphere / box / union / subtract / smooth-min
//!   distances and gradient directions (the leaf is already golden-tested; this
//!   asserts the wrapper does not corrupt the boundary).
//! - `sphere_vs_sdf_box_manifold` pins the emitted manifold (normal ≈ field
//!   gradient, separation ≈ d − radius, `body_b == SDF_SENTINEL`).
//! - `sdf_collision_resolves` is the seam-swap analog: a sphere dropped onto an
//!   SDF box "floor" RESTS on it under `SoftStepSolver` and falls through under
//!   `NoopSolver`.
//! - `box_on_sdf_incline` is the SDF variant of `box_box_friction_3d`: a box on an
//!   inclined SDF plane holds under static friction below the cone limit and slides
//!   above it (the SDF contact feeds the 2-DOF cone correctly).
//! - `sdf_solver_is_deterministic` runs an SDF scene twice and asserts the result
//!   is bit-identical (the SDF narrowphase + sentinel path is deterministic).
//!
//! The full-pipeline tests drive a real `Schedule` (a `num_threads(1)` pool, the
//! IM-2 determinism precondition). The `cpu_gpu_sdf_agreement` three-way
//! conformance gate is INTENTIONALLY ABSENT here — it needs the GPU + boyko_rhi
//! and is a separate GPU-tester gate; this crate stays graphics-free.

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    BodyType, Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass,
};
use boyko_physics::manifold::SDF_SENTINEL;
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_sdf;
use boyko_physics::resources::{Manifolds, PhysicsConfig};
use boyko_physics::sdf_query::{SdfField, sample_sdf};
use boyko_physics::solver::{NoopSolver, RigidSolver, SoftStepSolver};

use boyko_sdf_math::{SdfEdit, sdf_op};

// ── Test helpers (mirror `softstep.rs`) ──────────────────────────────────────

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
///
/// # Safety
///
/// `T` must be a `#[repr(C)]` value whose bytes are a valid serialization for the
/// pool registered under `T::component_id()` (holds for the physics components).
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; we view its `size_of::<T>()` bytes as a
    // read-only slice for the borrow's duration. `T` is `#[repr(C)]` so the byte
    // layout matches what the pool stores; the slice borrows `value` and cannot
    // outlive it.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A single-threaded pool (deterministic, IM-2 precondition).
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// Spawns one rigid body via the raw `create_entity` path.
fn spawn_body(world: &mut EcsMaster, body: RigidBody, mass: RigidBodyMass, collider: Collider) {
    let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
    world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("invariant: RigidBodyBundle archetype accepts the three columns");
}

/// A sphere body at `position` with the given velocity, radius, mass-inverse,
/// restitution, friction, and body type.
#[allow(clippy::too_many_arguments)]
fn sphere(
    position: Vec3,
    velocity: Vec3,
    radius: f32,
    inv_mass: f32,
    restitution: f32,
    friction: f32,
    body_type: BodyType,
) -> (RigidBody, RigidBodyMass, Collider) {
    let body = RigidBody {
        position,
        linear_velocity: velocity,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass,
        restitution,
        friction,
        body_type,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius },
        layer: 1,
        mask: 1,
    };
    (body, mass, collider)
}

/// A box (OBB) body at `position`/`rotation` with the given half-extents, mass-
/// inverse, friction, and body type (no initial velocity, zero restitution).
#[allow(clippy::too_many_arguments)]
fn box_body(
    position: Vec3,
    rotation: Quat,
    half_extents: Vec3,
    inv_mass: f32,
    restitution: f32,
    friction: f32,
    body_type: BodyType,
) -> (RigidBody, RigidBodyMass, Collider) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass,
        restitution,
        friction,
        body_type,
    };
    let collider = Collider {
        shape: ColliderShape::Box { half_extents },
        layer: 1,
        mask: 1,
    };
    (body, mass, collider)
}

/// A quaternion rotating by `angle` radians about +z.
fn quat_z(angle: f32) -> Quat {
    let half = 0.5 * angle;
    Quat::new(0.0, 0.0, half.sin(), half.cos())
}

/// Builds the SDF physics schedule for solver `S`, inserts the SDF field, and
/// stamps the fixed timestep.
fn build_sdf_schedule<S: RigidSolver + Default>(
    world: &mut EcsMaster,
    field: SdfField,
    dt: f32,
) -> Schedule {
    let mut builder = ScheduleBuilder::new(serial_pool());
    let _keys = add_physics_sdf::<S>(&mut builder, world);
    *world.resource_mut::<SdfField>() = field;
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(dt)));
    builder.build(world)
}

/// Reads back every body's `RigidBody` in query (= spawn) order.
fn all_bodies(world: &mut EcsMaster) -> Vec<RigidBody> {
    let q = world.query::<&RigidBody, ()>();
    q.iter().copied().collect()
}

/// An SDF "floor": a single box whose top face sits at `y = 0` (center at
/// `y = -half`, so a body falling in the +y half-space rests at its own radius).
fn sdf_floor() -> SdfField {
    let half = 50.0_f32;
    SdfField::from_edits(&[SdfEdit::box_shape(
        [0.0, -half, 0.0],
        [half, half, half],
        sdf_op::UNION,
        0.0,
    )])
}

// ── sample_sdf unit tests (pure-fn, Miri-tractable) ──────────────────────────

#[test]
fn sample_sdf_sphere_distance_and_gradient() {
    // A unit SDF sphere (radius 1) at the origin. A point at (2, 0, 0) is exactly
    // 1 unit OUTSIDE (distance 2 − 1 = 1); the gradient points radially outward.
    let field = SdfField::from_edits(&[SdfEdit::sphere([0.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0)]);
    let (d, n) = sample_sdf(&field, Vec3::new(2.0, 0.0, 0.0));
    assert!((d - 1.0).abs() < 1e-3, "sphere distance: {d}");
    // Outward radial gradient ≈ +x (a unit vector).
    assert!((n.length() - 1.0).abs() < 1e-3, "gradient must be unit: {}", n.length());
    assert!(n.x > 0.99, "gradient points +x: {n:?}");
    assert!(n.y.abs() < 1e-2 && n.z.abs() < 1e-2, "gradient is radial: {n:?}");

    // A point INSIDE the sphere has a negative distance.
    let (d_in, _) = sample_sdf(&field, Vec3::new(0.25, 0.0, 0.0));
    assert!(d_in < 0.0, "inside the sphere is negative: {d_in}");
    assert!((d_in - (-0.75)).abs() < 1e-3, "inside distance 0.25 − 1 = −0.75: {d_in}");
}

#[test]
fn sample_sdf_box_distance_and_gradient() {
    // An SDF box centered at the origin with half-extents (1, 1, 1). A point at
    // (2, 0, 0) is 1 unit outside the +x face; the gradient points +x.
    let field = SdfField::from_edits(&[SdfEdit::box_shape(
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        sdf_op::UNION,
        0.0,
    )]);
    let (d, n) = sample_sdf(&field, Vec3::new(2.0, 0.0, 0.0));
    assert!((d - 1.0).abs() < 1e-3, "box face distance: {d}");
    assert!(n.x > 0.99 && n.y.abs() < 1e-2 && n.z.abs() < 1e-2, "+x face normal: {n:?}");

    // The top face (+y) of the same box: a point at (0, 1.5, 0) is 0.5 above it.
    let (d_top, n_top) = sample_sdf(&field, Vec3::new(0.0, 1.5, 0.0));
    assert!((d_top - 0.5).abs() < 1e-3, "top face distance: {d_top}");
    assert!(n_top.y > 0.99, "+y face normal: {n_top:?}");
}

#[test]
fn sample_sdf_union_takes_nearer_surface() {
    // Union of two spheres: the field is the MIN of the two distances. A point
    // between them samples the nearer surface.
    let field = SdfField::from_edits(&[
        SdfEdit::sphere([-2.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0),
        SdfEdit::sphere([2.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0),
    ]);
    // At (1, 0, 0): distance to the right sphere is |1 − 2| − 1 = 0 (on its
    // surface); to the left is 3 − 1 = 2. The union takes the nearer (0).
    let (d, _) = sample_sdf(&field, Vec3::new(1.0, 0.0, 0.0));
    assert!(d.abs() < 1e-3, "union takes the nearer surface (0): {d}");
}

#[test]
fn sample_sdf_subtract_carves_a_cavity() {
    // A large sphere with a smaller one SUBTRACTED carves a cavity: the point at
    // the small sphere's center, which was deep INSIDE the big sphere, becomes
    // OUTSIDE (positive distance) after the subtraction.
    let field = SdfField::from_edits(&[
        SdfEdit::sphere([0.0, 0.0, 0.0], 2.0, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.0, 0.0, 0.0], 1.0, sdf_op::SUBTRACT, 0.0),
    ]);
    // The center was inside the big sphere (−2) but is now inside the carved
    // cavity → outside the solid (distance ≈ +1, the carved radius).
    let (d, _) = sample_sdf(&field, Vec3::new(0.0, 0.0, 0.0));
    assert!(d > 0.0, "subtraction carves the center out of the solid: {d}");
    assert!((d - 1.0).abs() < 1e-3, "carved distance ≈ +1: {d}");
}

#[test]
fn sample_sdf_smooth_min_rounds_below_hard_min() {
    // A smooth-union (smoothness > 0) blends two surfaces: at the seam the smooth
    // field dips BELOW the hard min (the polynomial smooth-min subtracts a bulge).
    let hard = SdfField::from_edits(&[
        SdfEdit::sphere([-1.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0),
        SdfEdit::sphere([1.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0),
    ]);
    let smooth = SdfField::from_edits(&[
        SdfEdit::sphere([-1.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0),
        SdfEdit::sphere([1.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.5),
    ]);
    // At the seam (origin, equidistant from both centers) the smooth field is
    // strictly less than the hard min (the blend pulls material in).
    let (d_hard, _) = sample_sdf(&hard, Vec3::new(0.0, 0.0, 0.0));
    let (d_smooth, _) = sample_sdf(&smooth, Vec3::new(0.0, 0.0, 0.0));
    assert!(
        d_smooth < d_hard,
        "smooth-min must round below the hard min at the seam: smooth {d_smooth}, hard {d_hard}"
    );
}

#[test]
fn sample_sdf_empty_field_is_far() {
    // An empty field samples to +far everywhere (no edits) — no body collides.
    let field = SdfField::default();
    assert!(field.is_empty());
    let (d, _) = sample_sdf(&field, Vec3::new(1.0, 2.0, 3.0));
    assert!(d > 1.0e8, "empty field is +far: {d}");
}

// ── sphere_vs_sdf_box_manifold ───────────────────────────────────────────────

#[test]
fn sphere_vs_sdf_box_manifold() {
    // A sphere overlapping an SDF box floor (top at y = 0) produces ONE manifold:
    // normal ≈ −gradient (the solver's A → surface convention, so −y for a floor),
    // separation ≈ d − radius, body_b == SDF_SENTINEL, body_a == the sphere's dense
    // row (0, the only body).
    let mut world = EcsMaster::new();
    let r = 0.5_f32;
    // Sphere center at y = 0.4 ⇒ d (above the top face) = 0.4, separation =
    // 0.4 − 0.5 = −0.1 (penetrating).
    let (sb, sm, sc) = sphere(
        Vec3::new(0.0, 0.4, 0.0),
        Vec3::ZERO,
        r,
        1.0,
        0.0,
        0.0,
        BodyType::Dynamic,
    );
    spawn_body(&mut world, sb, sm, sc);

    let dt = 1.0 / 120.0;
    // Use NoopSolver so the bodies do not move before we inspect the FIRST step's
    // manifold (the narrowphase still runs and appends the SDF contact).
    let mut schedule = build_sdf_schedule::<NoopSolver>(&mut world, sdf_floor(), dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;
    schedule.run(&mut world);

    let manifolds = &world.resource::<Manifolds>().manifolds;
    assert_eq!(manifolds.len(), 1, "exactly one body-vs-SDF manifold");
    let m = manifolds[0];
    assert_eq!(m.body_b, SDF_SENTINEL, "SDF manifold keys body_b == SDF_SENTINEL");
    assert_eq!(m.body_a.0, 0, "body_a is the sphere's dense row");
    assert_eq!(m.count, 1, "a sphere emits a single contact point");
    // Normal ≈ −y (the A → surface convention: the box top-face gradient is +y, the
    // manifold normal is its negation so the one-sided push ejects A upward).
    assert!(m.normal.y < -0.99, "SDF manifold normal is A→surface (−y): {:?}", m.normal);
    // Separation ≈ d − radius = 0.4 − 0.5 = −0.1.
    assert!(
        (m.points[0].separation - (-0.1)).abs() < 1e-2,
        "separation ≈ d − radius (−0.1): {}",
        m.points[0].separation
    );
}

// ── sdf_collision_resolves (seam-swap analog) ────────────────────────────────

#[test]
fn sdf_collision_resolves() {
    // A dynamic sphere dropped onto an SDF box "floor" (top at y = 0) under gravity
    // must REST on it (settle near y ≈ radius) under SoftStepSolver, but fall
    // THROUGH it under NoopSolver (which only integrates). Proves the SDF contact
    // is load-bearing through the C1 sentinel solve.
    fn final_y<S: RigidSolver + Default>() -> (f32, usize) {
        let mut world = EcsMaster::new();
        let r = 0.5_f32;
        // Spawn ABOVE the floor with a gap so it falls in (rest height ≈ r).
        let (sb, sm, sc) = sphere(
            Vec3::new(0.0, 2.0, 0.0),
            Vec3::ZERO,
            r,
            1.0,
            0.0,
            0.5,
            BodyType::Dynamic,
        );
        spawn_body(&mut world, sb, sm, sc);

        let dt = 1.0 / 60.0;
        let mut schedule = build_sdf_schedule::<S>(&mut world, sdf_floor(), dt);
        world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);

        let mut total_contacts = 0usize;
        for _ in 0..240 {
            schedule.run(&mut world);
            total_contacts += world.resource::<Manifolds>().manifolds.len();
        }
        (all_bodies(&mut world)[0].position.y, total_contacts)
    }

    let (y_soft, contacts_soft) = final_y::<SoftStepSolver>();
    let (y_noop, _) = final_y::<NoopSolver>();

    // Non-vacuity: the resting sphere must be in genuine SDF contact (else the body
    // is frozen and the resolve is never exercised).
    assert!(
        contacts_soft >= 100,
        "the sphere must rest in sustained SDF contact (else vacuous): {contacts_soft}"
    );
    // SoftStep: rests near the surface (radius above the y = 0 top face), NOT
    // sunk far below and NOT launched.
    assert!(
        y_soft > 0.3 && y_soft < 0.7,
        "SoftStep rests the sphere on the SDF floor (y ≈ 0.5): {y_soft}"
    );
    // Noop: falls straight through (no contact resolve) — far below the surface.
    assert!(
        y_noop < -5.0,
        "NoopSolver lets the sphere fall through the SDF floor: {y_noop}"
    );
}

// ── box_on_sdf_incline (SDF variant of box_box_friction_3d) ───────────────────

/// Drops a box onto an SDF box floor tilted by `incline` about +z, lets it SETTLE,
/// then returns its horizontal travel along the slope after `frames` of gravity-
/// driven sliding under the given `friction`. A high-friction box should creep far
/// less than a low-friction one (static friction holds it below the cone limit).
fn box_sdf_incline_slide(friction: f32, incline: f32, frames: usize) -> (f32, usize) {
    let mut world = EcsMaster::new();
    // The leaf's `sd_box` is axis-aligned (no per-edit rotation), so the incline is
    // realized as a large AXIS-ALIGNED SDF floor (top face at y = 0) under a GRAVITY
    // tilted by `incline` about +z. A box resting flat on the floor then feels a
    // tangential gravity component along the surface — the exact incline-friction
    // scenario, with the SDF contact normal staying +y (the floor's actual normal).
    let half = 50.0_f32;
    let field = SdfField::from_edits(&[SdfEdit::box_shape(
        [0.0, -half, 0.0],
        [half, half, half],
        sdf_op::UNION,
        0.0,
    )]);

    let he = Vec3::new(0.5, 0.5, 0.5);
    // Spawn the box just above the floor so it falls in and settles flat.
    let (bb, bm, bc) = box_body(
        Vec3::new(0.0, 1.0, 0.0),
        Quat::IDENTITY,
        he,
        1.0,
        0.0,
        friction,
        BodyType::Dynamic,
    );
    spawn_body(&mut world, bb, bm, bc);

    let dt = 1.0 / 120.0;
    let mut schedule = build_sdf_schedule::<SoftStepSolver>(&mut world, field, dt);
    // Tilt gravity by `incline` about +z: g = R_z(incline) · (0, -9.81, 0), giving a
    // tangential component along +x proportional to sin(incline) — the slope force.
    let g = quat_z(incline).rotate(Vec3::new(0.0, -9.81, 0.0));
    world.resource_mut::<PhysicsConfig>().gravity = g;

    // Settle the box flat onto the floor under the tilted gravity.
    for _ in 0..240 {
        schedule.run(&mut world);
    }
    let settled_x = all_bodies(&mut world)[0].position.x;

    // Measure the slide over the next `frames` (the static/kinetic friction outcome).
    let mut total_contacts = 0usize;
    for _ in 0..frames {
        schedule.run(&mut world);
        total_contacts += world.resource::<Manifolds>().manifolds.len();
    }
    let final_x = all_bodies(&mut world)[0].position.x;
    (final_x - settled_x, total_contacts)
}

#[test]
fn box_on_sdf_incline() {
    // The SDF variant of `box_box_friction_3d`: a box resting on an SDF floor under
    // a tilted gravity (the incline). HIGH friction holds it (the cone resists the
    // tangential slope force below its limit); LOW friction lets it slide. Proves
    // the SDF contact feeds the 2-DOF friction cone correctly.
    let incline = 0.3_f32; // ~17° slope.
    let frames = 240usize;

    let (slide_high, contacts_high) = box_sdf_incline_slide(1.0, incline, frames);
    let (slide_low, contacts_low) = box_sdf_incline_slide(0.0, incline, frames);

    // Non-vacuity: the box must be in genuine SDF contact while sliding.
    assert!(
        contacts_high >= frames && contacts_low >= frames,
        "the box must contact the SDF floor every frame (else vacuous): high {contacts_high}, low {contacts_low}"
    );
    // Friction resists the slope: a high-µ box travels much less far than a
    // frictionless one (which accelerates down the slope under tangential gravity).
    assert!(
        slide_high.abs() < slide_low.abs(),
        "friction must resist the incline slide: high-µ Δx {slide_high} vs low-µ Δx {slide_low}"
    );
    // The frictionless box actually slid down the slope (sanity: tangential gravity
    // moves it along +x — the slope's downhill direction for R_z(+incline)·g).
    assert!(
        slide_low.abs() > 0.1,
        "the frictionless box should slide down the incline: Δx {slide_low}"
    );
}

// ── sdf_solver_is_deterministic ──────────────────────────────────────────────

#[test]
fn sdf_solver_is_deterministic() {
    // The SDF narrowphase + the C1 sentinel solve must be bit-reproducible: the same
    // SDF scene, spawned in the same order through a num_threads(1) pool, run twice
    // IN THIS PROCESS, ends bit-identical. Guards the sentinel body-fetch + the
    // `pack_sdf` warm-start key path against hidden run-to-run nondeterminism.
    fn run_once() -> Vec<RigidBody> {
        let mut world = EcsMaster::new();
        // A small cluster of dynamic spheres + a box, all dropped onto an SDF floor.
        let setup = [
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.3, 1.4, 0.1),
            Vec3::new(-0.2, 1.7, -0.1),
        ];
        for &pos in &setup {
            let (b, m, c) = sphere(pos, Vec3::ZERO, 0.5, 1.0, 0.3, 0.5, BodyType::Dynamic);
            spawn_body(&mut world, b, m, c);
        }
        let (bb, bm, bc) = box_body(
            Vec3::new(1.0, 1.2, 0.0),
            quat_z(0.2),
            Vec3::new(0.5, 0.5, 0.5),
            1.0,
            0.3,
            0.5,
            BodyType::Dynamic,
        );
        spawn_body(&mut world, bb, bm, bc);

        let dt = 1.0 / 60.0;
        let mut schedule = build_sdf_schedule::<SoftStepSolver>(&mut world, sdf_floor(), dt);
        world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);
        for _ in 0..60 {
            schedule.run(&mut world);
        }
        all_bodies(&mut world)
    }

    let a = run_once();
    let b = run_once();
    assert_eq!(a.len(), b.len());
    for (i, (ba, bb)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(ba.position.x.to_bits(), bb.position.x.to_bits(), "body {i} pos.x");
        assert_eq!(ba.position.y.to_bits(), bb.position.y.to_bits(), "body {i} pos.y");
        assert_eq!(ba.position.z.to_bits(), bb.position.z.to_bits(), "body {i} pos.z");
        assert_eq!(ba.linear_velocity.x.to_bits(), bb.linear_velocity.x.to_bits(), "body {i} vel.x");
        assert_eq!(ba.linear_velocity.y.to_bits(), bb.linear_velocity.y.to_bits(), "body {i} vel.y");
        assert_eq!(ba.linear_velocity.z.to_bits(), bb.linear_velocity.z.to_bits(), "body {i} vel.z");
        assert_eq!(ba.rotation.x.to_bits(), bb.rotation.x.to_bits(), "body {i} rot.x");
        assert_eq!(ba.rotation.y.to_bits(), bb.rotation.y.to_bits(), "body {i} rot.y");
        assert_eq!(ba.rotation.z.to_bits(), bb.rotation.z.to_bits(), "body {i} rot.z");
        assert_eq!(ba.rotation.w.to_bits(), bb.rotation.w.to_bits(), "body {i} rot.w");
    }
}

// ── critical-point degeneracy (C1: zero gradient ⇒ ZERO normal, never NaN) ─────

#[test]
fn sdf_critical_point_emits_no_contact_and_stays_finite() {
    // A sphere body whose center coincides EXACTLY with an SDF primitive center under
    // deep penetration is a FIELD CRITICAL POINT: the central-difference gradient is
    // symmetric and folds to [0, 0, 0]. Before the C1 guard, the leaf normalized that
    // to [NaN, NaN, NaN]; `NaN < eps²` is `false`, so the narrowphase seam-skip never
    // fired and a NaN-normal contact poisoned the solver (breaking determinism, since
    // NaN != NaN). With the guard the gradient arrives as Vec3::ZERO, the skip fires,
    // and NO contact is emitted that frame.
    //
    // Field: a single SDF sphere (radius 2) at the origin. A radius-0.5 body at the
    // origin has center d = −2 (separation = −2.5, deeply penetrating) and a zero
    // gradient — the exact case the guard must survive.
    fn critical_field() -> SdfField {
        SdfField::from_edits(&[SdfEdit::sphere([0.0, 0.0, 0.0], 2.0, sdf_op::UNION, 0.0)])
    }

    // Part 1: the narrowphase emits NO manifold for the center-at-critical-point body.
    {
        let mut world = EcsMaster::new();
        let (sb, sm, sc) = sphere(
            Vec3::ZERO,
            Vec3::ZERO,
            0.5,
            1.0,
            0.0,
            0.5,
            BodyType::Dynamic,
        );
        spawn_body(&mut world, sb, sm, sc);

        let dt = 1.0 / 120.0;
        // NoopSolver: keep the body fixed at the origin so the narrowphase samples the
        // exact critical point on the FIRST step.
        let mut schedule = build_sdf_schedule::<NoopSolver>(&mut world, critical_field(), dt);
        world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;
        schedule.run(&mut world);

        let manifolds = &world.resource::<Manifolds>().manifolds;
        assert_eq!(
            manifolds.len(),
            0,
            "a center-at-critical-point body (zero gradient) emits no SDF contact (the \
             C1 skip fires on a ZERO normal, not a NaN one): {manifolds:?}"
        );
    }

    // Part 2: a real scene CONTAINING such a degenerate body stays finite and
    // deterministic over N steps under SoftStepSolver (a NaN normal would poison the
    // solver and break the bit-for-bit re-run).
    fn run_once() -> Vec<RigidBody> {
        let mut world = EcsMaster::new();
        // The degenerate body (center at the SDF sphere center) ...
        let (db, dm, dc) = sphere(Vec3::ZERO, Vec3::ZERO, 0.5, 1.0, 0.0, 0.5, BodyType::Dynamic);
        spawn_body(&mut world, db, dm, dc);
        // ... plus an ordinary body that resolves normally (so the solver runs).
        let (sb, sm, sc) = sphere(
            Vec3::new(3.0, 1.5, 0.0),
            Vec3::ZERO,
            0.5,
            1.0,
            0.0,
            0.5,
            BodyType::Dynamic,
        );
        spawn_body(&mut world, sb, sm, sc);

        let dt = 1.0 / 60.0;
        let mut schedule = build_sdf_schedule::<SoftStepSolver>(&mut world, critical_field(), dt);
        world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);
        for _ in 0..120 {
            schedule.run(&mut world);
        }
        all_bodies(&mut world)
    }

    let a = run_once();
    // Every body component stays finite — no NaN/Inf leaked from the critical point.
    for (i, body) in a.iter().enumerate() {
        for (name, v) in [
            ("position", body.position),
            ("linear_velocity", body.linear_velocity),
            ("angular_velocity", body.angular_velocity),
        ] {
            assert!(
                v.x.is_finite() && v.y.is_finite() && v.z.is_finite(),
                "body {i} {name} must stay finite (no NaN from the critical point): {v:?}"
            );
        }
        let q = body.rotation;
        assert!(
            q.x.is_finite() && q.y.is_finite() && q.z.is_finite() && q.w.is_finite(),
            "body {i} rotation must stay finite: {q:?}"
        );
    }

    // ... and the run is bit-for-bit deterministic (a NaN would make `a != b`).
    let b = run_once();
    assert_eq!(a.len(), b.len());
    for (i, (ba, bb)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(ba.position.x.to_bits(), bb.position.x.to_bits(), "body {i} pos.x");
        assert_eq!(ba.position.y.to_bits(), bb.position.y.to_bits(), "body {i} pos.y");
        assert_eq!(ba.position.z.to_bits(), bb.position.z.to_bits(), "body {i} pos.z");
        assert_eq!(ba.linear_velocity.x.to_bits(), bb.linear_velocity.x.to_bits(), "body {i} vel.x");
        assert_eq!(ba.linear_velocity.y.to_bits(), bb.linear_velocity.y.to_bits(), "body {i} vel.y");
        assert_eq!(ba.linear_velocity.z.to_bits(), bb.linear_velocity.z.to_bits(), "body {i} vel.z");
    }
}
