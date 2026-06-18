//! Phase O5 acceptance gates (Gate 2 + Gate 3) — the COLORED solve path driven
//! through the REAL physics `Schedule` via
//! [`add_physics_colored_solve`](boyko_physics::add_physics_colored_solve).
//!
//! # Gate 2 — tolerance acceptance on the colored path
//!
//! The colored sweep reorders the per-substep Gauss-Seidel pass (color order, not
//! manifold order), so the converged float values DIFFER from the reference
//! [`SoftStepSolver`](boyko_physics::SoftStepSolver) — but the result must stay
//! PHYSICALLY VALID: penetration resolves, stacks settle and hold, friction holds
//! below the cone limit and slides above it, restitution bounces. These gates
//! assert TOLERANCES / INEQUALITIES (never a bit-baseline against the reference),
//! which absorb the converged-value change. They mirror the reference
//! `softstep.rs` gates one-for-one, swapping `add_physics_systems::<SoftStepSolver>`
//! for `add_physics_colored_solve`.
//!
//! # Gate 3 — 0%-gate (the reference is byte-untouched)
//!
//! The colored solve is a SEPARATE solver + stage (Decision 7). Adding it must not
//! perturb the default path: `colored_solve_run_to_run_bit_identical` proves the
//! colored solve is itself deterministic through the schedule, and the reference's
//! own determinism/static-body gates stay green in `softstep.rs` (unchanged). The
//! `add_physics_colored_solve` path registers `physics_solve_colored` IN PLACE of
//! the default `physics_solve_step` — confirmed via the stage keys.
//!
//! This file spins up `boyko_threadpool` (intractable under Miri — the pool is
//! loom+Miri proven in the ECS Phase-9 series), so it is `cfg(not(miri))`. The
//! pure colored-solve path is covered Miri-clean by the `colored.rs` lib tests.

#![cfg(not(miri))]

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    BodyType, Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{Manifolds, PhysicsConfig};

// ── Test helpers (mirror `softstep.rs`) ──────────────────────────────────────

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; we view its `size_of::<T>()`
    // bytes as a read-only slice bounded by the borrow — the exact layout the pool
    // stores (mirrors `softstep::as_bytes`).
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A single-threaded pool (deterministic, IM-2 precondition).
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

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

/// A quaternion rotating by `angle` radians about +z (the incline axis).
fn quat_z(angle: f32) -> Quat {
    let half = 0.5 * angle;
    Quat::new(0.0, 0.0, half.sin(), half.cos())
}

/// Builds the COLORED physics schedule and stamps the fixed timestep.
fn build_colored_schedule(world: &mut EcsMaster, dt: f32) -> Schedule {
    let mut builder = ScheduleBuilder::new(serial_pool());
    let _keys = add_physics_colored_solve(&mut builder, world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(dt)));
    builder.build(world)
}

fn all_bodies(world: &mut EcsMaster) -> Vec<RigidBody> {
    let q = world.query::<&RigidBody, ()>();
    q.iter().copied().collect()
}

fn set_body_velocity(world: &mut EcsMaster, row: usize, velocity: Vec3) {
    let mut q = world.query::<&mut RigidBody, ()>();
    for (i, body) in q.iter_mut().enumerate() {
        if i == row {
            body.linear_velocity = velocity;
        }
    }
}

// ── Gate 2: softstep_resolves_penetration (colored) ──────────────────────────

#[test]
fn colored_softstep_resolves_penetration() {
    // Two overlapping unit spheres (radius 0.5) along +X penetrate by 0.5. No
    // gravity, no restitution → they must separate and conserve momentum (COM
    // stays put). A tolerance gate — the colored sweep value change is absorbed.
    let mut world = EcsMaster::new();
    let (ab, am, ac) = sphere(Vec3::ZERO, Vec3::ZERO, 0.5, 1.0, 0.0, 0.0, BodyType::Dynamic);
    spawn_body(&mut world, ab, am, ac);
    let (bb, bm, bc) = sphere(
        Vec3::new(0.5, 0.0, 0.0),
        Vec3::ZERO,
        0.5,
        1.0,
        0.0,
        0.0,
        BodyType::Dynamic,
    );
    spawn_body(&mut world, bb, bm, bc);

    let dt = 1.0 / 60.0;
    let mut schedule = build_colored_schedule(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;

    for _ in 0..120 {
        schedule.run(&mut world);
    }

    let bodies = all_bodies(&mut world);
    assert_eq!(bodies.len(), 2);
    let dist = (bodies[1].position - bodies[0].position).length();
    assert!(
        dist >= 1.0 - 1e-2,
        "colored solve must separate spheres to >= radius sum, got dist {dist}"
    );
    let com_x = 0.5 * (bodies[0].position.x + bodies[1].position.x);
    assert!((com_x - 0.25).abs() < 1e-3, "COM drifted under colored solve: com_x {com_x}");
    assert!(bodies[0].position.x < 0.0, "A pushed -X: {:?}", bodies[0]);
    assert!(bodies[1].position.x > 0.5, "B pushed +X: {:?}", bodies[1]);
}

// ── Gate 2: restitution bounce vs no-bounce (colored) ─────────────────────────

#[test]
fn colored_restitution_bounce_vs_no_bounce() {
    fn run(restitution: f32) -> f32 {
        let mut world = EcsMaster::new();
        let (wb, wm, wc) = sphere(Vec3::ZERO, Vec3::ZERO, 0.5, 0.0, restitution, 0.0, BodyType::Static);
        spawn_body(&mut world, wb, wm, wc);
        let approach = -5.0_f32;
        let (mb, mm, mc) = sphere(
            Vec3::new(0.95, 0.0, 0.0),
            Vec3::new(approach, 0.0, 0.0),
            0.5,
            1.0,
            restitution,
            0.0,
            BodyType::Dynamic,
        );
        spawn_body(&mut world, mb, mm, mc);

        let dt = 1.0 / 120.0;
        let mut schedule = build_colored_schedule(&mut world, dt);
        world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;
        schedule.run(&mut world);

        all_bodies(&mut world)[1].linear_velocity.x
    }

    let v_no_bounce = run(0.0);
    let v_bounce = run(0.9);

    assert!(v_no_bounce > -0.5, "restitution 0 must not rebound under colored solve, vx {v_no_bounce}");
    assert!(v_bounce > 3.0, "restitution 0.9 must rebound (~4.5) under colored solve, vx {v_bounce}");
}

// ── Gate 2: restitution resting contact does not gain energy (colored) ────────

#[test]
fn colored_restitution_resting_contact_does_not_gain_energy() {
    // A dynamic sphere resting on a static sphere under gravity, restitution 0.5,
    // must SETTLE — no upward creep / energy gain (the RESTITUTION_THRESHOLD guard
    // is on the colored path too). Non-vacuous: a contact fires every frame.
    let mut world = EcsMaster::new();
    let (fb, fm, fc) = sphere(Vec3::ZERO, Vec3::ZERO, 0.5, 0.0, 0.5, 0.5, BodyType::Static);
    spawn_body(&mut world, fb, fm, fc);
    let (sb, sm, sc) = sphere(
        Vec3::new(0.0, 0.99, 0.0),
        Vec3::ZERO,
        0.5,
        1.0,
        0.5,
        0.5,
        BodyType::Dynamic,
    );
    spawn_body(&mut world, sb, sm, sc);

    let dt = 1.0 / 60.0;
    let mut schedule = build_colored_schedule(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);

    for _ in 0..30 {
        schedule.run(&mut world);
    }
    let settled_y = all_bodies(&mut world)[1].position.y;

    let mut max_y = settled_y;
    let mut max_speed_sq = 0.0_f32;
    let mut total_contacts = 0usize;
    for _ in 0..200 {
        schedule.run(&mut world);
        total_contacts += world.resource::<Manifolds>().manifolds.len();
        let b = all_bodies(&mut world)[1];
        max_y = max_y.max(b.position.y);
        max_speed_sq = max_speed_sq.max(b.linear_velocity.length_squared());
    }

    assert!(
        total_contacts >= 200,
        "resting pair must contact every frame (else the gate is vacuous): {total_contacts}"
    );
    assert!(max_y <= settled_y + 1e-2, "resting body crept upward under colored solve: settled {settled_y}, max {max_y}");
    assert!(max_speed_sq < 1.0, "resting body gained energy under colored solve: max speed^2 {max_speed_sq}");
}

// ── Gate 2: stacking_is_stable (colored, spheres) ─────────────────────────────

/// A vertical sphere stack on a static floor sphere (mirrors `softstep::spawn_sphere_stack`).
fn spawn_sphere_stack(world: &mut EcsMaster, n: usize, r: f32) {
    let floor_r = 50.0_f32;
    let (fb, fm, fc) = sphere(
        Vec3::new(0.0, -floor_r, 0.0),
        Vec3::ZERO,
        floor_r,
        0.0,
        0.0,
        0.6,
        BodyType::Static,
    );
    spawn_body(world, fb, fm, fc);
    let overlap = 0.01_f32;
    for i in 0..n {
        let y = r + (i as f32) * (2.0 * r - overlap) - overlap;
        let (sb, sm, sc) = sphere(Vec3::new(0.0, y, 0.0), Vec3::ZERO, r, 1.0, 0.0, 0.6, BodyType::Dynamic);
        spawn_body(world, sb, sm, sc);
    }
}

#[test]
fn colored_stacking_is_stable() {
    // A small sphere stack must HOLD under the colored solve — no jitter-apart, no
    // explosion, no sink-through. The warm-start (canonical-order IM-2b store)
    // seeds each contact so the resting stack stays supported.
    let n = 4usize;
    let r = 0.5_f32;
    let mut world = EcsMaster::new();
    spawn_sphere_stack(&mut world, n, r);

    let dt = 1.0 / 60.0;
    let mut schedule = build_colored_schedule(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);

    let rest_top_y = all_bodies(&mut world)[n].position.y;

    for _ in 0..30 {
        schedule.run(&mut world);
    }

    let mut max_top_y = f32::MIN;
    let mut min_top_y = f32::MAX;
    let mut max_pen = 0.0_f32;
    let mut max_speed_sq = 0.0_f32;
    let mut total_contacts = 0usize;

    for _ in 0..300 {
        schedule.run(&mut world);
        total_contacts += world.resource::<Manifolds>().manifolds.len();
        let bodies = all_bodies(&mut world);
        for b in &bodies {
            assert!(
                b.position.x.is_finite() && b.position.y.is_finite() && b.position.z.is_finite(),
                "colored stack produced a non-finite position: {b:?}"
            );
        }
        let top_y = bodies[n].position.y;
        max_top_y = max_top_y.max(top_y);
        min_top_y = min_top_y.min(top_y);
        max_speed_sq = max_speed_sq
            .max(bodies[1..=n].iter().map(|b| b.linear_velocity.length_squared()).fold(0.0, f32::max));
        for w in 1..n {
            let pen = (2.0 * r) - (bodies[w + 1].position.y - bodies[w].position.y);
            max_pen = max_pen.max(pen);
        }
    }

    assert!(
        total_contacts >= 300 * 2,
        "the colored stack must be in sustained multi-contact (else vacuous): {total_contacts}"
    );
    assert!(
        max_top_y <= rest_top_y + 0.25 * r,
        "colored stack drifted apart / launched: rest {rest_top_y}, max {max_top_y}"
    );
    assert!(
        min_top_y >= rest_top_y - 0.5 * r,
        "colored stack sank / collapsed: rest {rest_top_y}, min {min_top_y}"
    );
    assert!(max_pen < 0.25 * r, "colored inter-body penetration too deep: {max_pen}");
    assert!(max_speed_sq < 1.0, "colored stack jittering: max speed^2 {max_speed_sq}");
}

// ── Gate 2: stacking_is_stable (colored, boxes — multi-point manifolds) ────────

fn spawn_box_stack(world: &mut EcsMaster, n: usize, h: f32) {
    let floor_half = Vec3::new(20.0, 1.0, 20.0);
    let (fb, fm, fc) = box_body(
        Vec3::new(0.0, -floor_half.y, 0.0),
        Quat::IDENTITY,
        floor_half,
        0.0,
        0.0,
        0.8,
        BodyType::Static,
    );
    spawn_body(world, fb, fm, fc);
    let overlap = 0.01_f32;
    for i in 0..n {
        let y = h + (i as f32) * (2.0 * h - overlap) - overlap;
        let (bb, bm, bc) = box_body(
            Vec3::new(0.0, y, 0.0),
            Quat::IDENTITY,
            Vec3::new(h, h, h),
            1.0,
            0.0,
            0.8,
            BodyType::Dynamic,
        );
        spawn_body(world, bb, bm, bc);
    }
}

#[test]
fn colored_stacking_is_stable_boxes() {
    // A tower of dynamic boxes on a static box floor under the colored solve. Box
    // face-face contacts are multi-point (up to 4) → the colored single-group
    // invariant (a manifold's ≥2 points stay contiguous in one color span) is
    // exercised through the full pipeline, and the stack must HOLD.
    let n = 3usize;
    let h = 0.5_f32;
    let mut world = EcsMaster::new();
    spawn_box_stack(&mut world, n, h);

    let dt = 1.0 / 60.0;
    let mut schedule = build_colored_schedule(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);

    let rest_top_y = all_bodies(&mut world)[n].position.y;

    for _ in 0..30 {
        schedule.run(&mut world);
    }

    let mut max_top_y = f32::MIN;
    let mut min_top_y = f32::MAX;
    let mut max_pen = 0.0_f32;
    let mut max_speed_sq = 0.0_f32;
    let mut total_contacts = 0usize;
    let mut multi_point_frames = 0usize;

    for _ in 0..300 {
        schedule.run(&mut world);
        let manifolds = world.resource::<Manifolds>();
        total_contacts += manifolds.manifolds.len();
        if manifolds.manifolds.iter().any(|m| m.count >= 2) {
            multi_point_frames += 1;
        }
        let bodies = all_bodies(&mut world);
        for b in &bodies {
            assert!(
                b.position.x.is_finite() && b.position.y.is_finite() && b.position.z.is_finite(),
                "colored box stack non-finite position: {b:?}"
            );
        }
        let top_y = bodies[n].position.y;
        max_top_y = max_top_y.max(top_y);
        min_top_y = min_top_y.min(top_y);
        max_speed_sq = max_speed_sq
            .max(bodies[1..=n].iter().map(|b| b.linear_velocity.length_squared()).fold(0.0, f32::max));
        for w in 1..n {
            let pen = (2.0 * h) - (bodies[w + 1].position.y - bodies[w].position.y);
            max_pen = max_pen.max(pen);
        }
    }

    assert!(
        total_contacts >= 300 * 2,
        "colored box stack must be in sustained multi-contact (else vacuous): {total_contacts}"
    );
    assert!(
        multi_point_frames >= 300 - 5,
        "colored box face-face must be multi-point nearly every frame (clipper firing): {multi_point_frames}"
    );
    assert!(max_top_y <= rest_top_y + 0.25 * h, "colored box stack drifted: rest {rest_top_y}, max {max_top_y}");
    assert!(min_top_y >= rest_top_y - 0.5 * h, "colored box stack sank: rest {rest_top_y}, min {min_top_y}");
    assert!(max_pen < 0.25 * h, "colored inter-box penetration too deep: {max_pen}");
    assert!(max_speed_sq < 1.0, "colored box stack jittering: max speed^2 {max_speed_sq}");
}

// ── Gate 2: box_box_friction_3d (colored) ─────────────────────────────────────

/// Drops a dynamic box onto a STATIC box inclined by `incline` rad, lets it SETTLE,
/// then runs `frames` more and returns its in-plane (x, y) slide magnitude from the
/// settled position (mirrors `softstep::box_incline_slide`).
fn box_incline_slide(incline: f32, friction: f32, frames: usize) -> f32 {
    let mut world = EcsMaster::new();
    let rot = quat_z(incline);
    let floor_half = Vec3::new(20.0, 0.5, 20.0);
    let (fb, fm, fc) = box_body(Vec3::ZERO, rot, floor_half, 0.0, 0.0, friction, BodyType::Static);
    spawn_body(&mut world, fb, fm, fc);
    let surface_normal = Vec3::new(-incline.sin(), incline.cos(), 0.0);
    let box_half = Vec3::new(0.5, 0.5, 0.5);
    // Spawn the dynamic box just above the incline surface so it falls into contact.
    let spawn_pos = surface_normal * (floor_half.y + box_half.y + 0.05);
    let (bb, bm, bc) = box_body(spawn_pos, rot, box_half, 1.0, 0.0, friction, BodyType::Dynamic);
    spawn_body(&mut world, bb, bm, bc);

    let dt = 1.0 / 120.0;
    let mut schedule = build_colored_schedule(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);

    for _ in 0..240 {
        schedule.run(&mut world);
    }
    let settled = all_bodies(&mut world)[1].position;

    let mut total_contacts = 0usize;
    for _ in 0..frames {
        schedule.run(&mut world);
        total_contacts += world.resource::<Manifolds>().manifolds.len();
    }
    assert!(
        total_contacts >= 1,
        "the inclined box must contact the floor (else friction is vacuous): {total_contacts}"
    );
    let p = all_bodies(&mut world)[1].position;
    let dx = p.x - settled.x;
    let dy = p.y - settled.y;
    (dx * dx + dy * dy).sqrt()
}

#[test]
fn colored_box_box_friction_3d() {
    // A box resting on an INCLINED static box under the colored solve. Below the
    // cone limit (tan θ < µ) it holds; above it (tan θ > µ) it slides. Forces the
    // 2-DOF Coulomb cone onto a MULTI-POINT box face contact in the colored kernel.
    let incline = 0.3_f32; // tan(0.3) ≈ 0.309.
    let held = box_incline_slide(incline, 1.0, 240);
    let slid = box_incline_slide(incline, 0.05, 240);

    assert!(held < 0.05, "colored: static friction must hold the box (tan θ < µ): slid {held}");
    assert!(slid > 0.2, "colored: above the cone limit the box must slide downhill: slid {slid}");
    assert!(slid > held * 5.0, "colored: the slide must dominate the hold (non-vacuous cone): {slid} vs {held}");
}

// ── Gate 2: sphere friction resists tangential motion (colored) ───────────────

/// Settles a sphere on a static floor sphere, injects a tangential push, returns
/// the contact-point slip magnitude (mirrors `softstep::settle_then_push_contact_slip`).
fn settle_then_push_contact_slip(friction: f32, push: Vec3, frames: usize) -> f32 {
    let mut world = EcsMaster::new();
    let floor_r = 50.0_f32;
    let (fb, fm, fc) = sphere(
        Vec3::new(0.0, -floor_r, 0.0),
        Vec3::ZERO,
        floor_r,
        0.0,
        0.0,
        friction,
        BodyType::Static,
    );
    spawn_body(&mut world, fb, fm, fc);
    let r = 0.5_f32;
    let (sb, sm, sc) = sphere(Vec3::new(0.0, 1.0, 0.0), Vec3::ZERO, r, 1.0, 0.0, friction, BodyType::Dynamic);
    spawn_body(&mut world, sb, sm, sc);

    let dt = 1.0 / 120.0;
    let mut schedule = build_colored_schedule(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);

    for _ in 0..240 {
        schedule.run(&mut world);
    }
    set_body_velocity(&mut world, 1, push);
    let mut total_contacts = 0usize;
    for _ in 0..frames {
        schedule.run(&mut world);
        total_contacts += world.resource::<Manifolds>().manifolds.len();
    }
    assert!(total_contacts >= 1, "the pushed sphere must contact the floor (else friction is vacuous): {total_contacts}");
    let b = all_bodies(&mut world)[1];
    let slip = b.linear_velocity + b.angular_velocity.cross(Vec3::new(0.0, -r, 0.0));
    (slip.x * slip.x + slip.z * slip.z).sqrt()
}

#[test]
fn colored_sphere_friction_holds_below_limit_slips_above() {
    // The colored cone must arrest a sub-limit contact slip and saturate (keep
    // slipping) for an over-limit push — the same Coulomb signature as the
    // reference, validated as an inequality (not a bit-baseline).
    let slip_held = settle_then_push_contact_slip(1.0, Vec3::new(0.05, 0.0, 0.0), 120);
    assert!(
        slip_held < 1e-3,
        "colored static friction must hold a sub-limit push (|slip| ≈ 0), got {slip_held}"
    );
    let slip_slipping = settle_then_push_contact_slip(1.0, Vec3::new(4.0, 0.0, 0.0), 1);
    assert!(
        slip_slipping > 0.5,
        "colored over-limit push must keep slipping (kinetic friction), got {slip_slipping}"
    );
}

// ── Gate 3: the colored solve is run-to-run bit-identical THROUGH the schedule ─

#[test]
fn colored_solve_run_to_run_bit_identical_through_schedule() {
    // The whole colored pipeline (gather → broadphase → narrowphase →
    // build_graph → solve_colored → apply), run twice in this process with a
    // serialized spawn through a num_threads(1) pool, ends bit-identical — the
    // colored partition + sweep + canonical warm store carry no run-to-run
    // nondeterminism.
    fn run_once() -> Vec<RigidBody> {
        let mut world = EcsMaster::new();
        let setup = [
            (Vec3::new(0.0, 1.0, 0.0), 1.0, BodyType::Dynamic),
            (Vec3::new(0.3, 1.4, 0.1), 1.0, BodyType::Dynamic),
            (Vec3::new(-0.2, 1.7, -0.1), 1.0, BodyType::Dynamic),
            (Vec3::new(0.0, -10.0, 0.0), 0.0, BodyType::Static),
        ];
        for &(pos, inv_mass, body_type) in &setup {
            let radius = if body_type == BodyType::Static { 10.0 } else { 0.5 };
            let (b, m, c) = sphere(pos, Vec3::ZERO, radius, inv_mass, 0.3, 0.5, body_type);
            spawn_body(&mut world, b, m, c);
        }
        let dt = 1.0 / 60.0;
        let mut schedule = build_colored_schedule(&mut world, dt);
        for _ in 0..30 {
            schedule.run(&mut world);
        }
        all_bodies(&mut world)
    }

    let a = run_once();
    let b = run_once();
    assert_eq!(a.len(), b.len());
    // Anti-vacuity: the scene actually evolved.
    assert!(a.iter().any(|b| b.position.y.to_bits() != 0), "the scene evolved (anti-vacuity)");
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

// ── Gate 3: the colored path replaces (not augments) the default solve stage ───

#[test]
fn colored_solve_path_registers_build_graph_and_replaces_solve() {
    // The 0%-gate structural confirmation: `add_physics_colored_solve` registers
    // `physics_build_graph` (Some) and a `solve` stage (the colored one, in place
    // of the default generic solve) — the two solvers never both run (Decision 7).
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(serial_pool());
    let keys = add_physics_colored_solve(&mut builder, &mut world);
    assert!(
        keys.build_graph.is_some(),
        "the colored-solve path must register physics_build_graph"
    );
    // The solve stage exists and is ordered after build_graph (descriptor indices
    // are assigned in registration order; build_graph is registered before solve).
    assert!(
        keys.solve > keys.build_graph.unwrap(),
        "the colored solve stage must be registered after build_graph: solve {} build_graph {:?}",
        keys.solve,
        keys.build_graph
    );
}
