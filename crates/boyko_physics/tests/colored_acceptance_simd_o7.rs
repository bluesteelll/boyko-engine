//! Phase O7 — acceptance on the SIMD SOLVE PATH (Gate 5).
//!
//! The O5 acceptance suite (`colored_acceptance_o5.rs`) drives the colored solve
//! with the DEFAULT config (`simd_solve == false`), i.e. the SCALAR-colored (O6)
//! path. The O7 bit-exactness tests prove `simd_solve == scalar` byte-for-byte, so
//! physical validity follows by transitivity — but the spec's Gate 5 wants the
//! colored acceptance gates exercised DIRECTLY on the live SIMD path. This file
//! re-runs the penetration + sphere-stack + box-stack acceptance scenes through the
//! REAL physics `Schedule` with `simd_solve == true` AND `parallel_solve == true`
//! over a multi-worker pool (the production O7 path), asserting the SAME tolerance /
//! inequality invariants the O5 gates assert.
//!
//! Gated `#[cfg(target_feature = "avx2")]`: only an +avx2 build widens the solve, so
//! only there is this non-vacuous (a non-AVX2 build routes both to the scalar oracle
//! and the O5 suite already covers it). `#[cfg(not(miri))]`: spins the threadpool.

#![cfg(all(not(miri), target_arch = "x86_64", target_feature = "avx2"))]

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{Manifolds, PhysicsConfig};

// ── Test helpers (mirror `colored_acceptance_o5.rs`) ──────────────────────────

fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; we view its `size_of::<T>()`
    // bytes as a read-only slice bounded by the borrow — the exact layout the pool
    // stores (mirrors `colored_acceptance_o5::as_bytes`).
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A 4-worker pool: the SIMD-solve scenes run under genuine cross-worker dispatch
/// so Gate 5 validates the production `{parallel + simd}` path, not just inline.
fn worker_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(4).build()
}

fn spawn_body(world: &mut EcsMaster, body: RigidBody, mass: RigidBodyMass, collider: Collider) {
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
        .expect("invariant: RigidBodyBundle archetype accepts the three columns");
    world.enable::<Simulated>(e);
}

#[allow(clippy::too_many_arguments)]
fn sphere(
    position: Vec3,
    velocity: Vec3,
    radius: f32,
    inv_mass: f32,
    restitution: f32,
    friction: f32,
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
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius },
        layer: 1,
        mask: 1,
    };
    (body, mass, collider)
}

fn box_body(
    position: Vec3,
    half_extents: Vec3,
    inv_mass: f32,
    friction: f32,
) -> (RigidBody, RigidBodyMass, Collider) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass,
        restitution: 0.0,
        friction,
    };
    let collider = Collider {
        shape: ColliderShape::Box { half_extents },
        layer: 1,
        mask: 1,
    };
    (body, mass, collider)
}

/// Builds the COLORED physics schedule with `simd_solve` + `parallel_solve` ON,
/// stamps the fixed timestep, and returns the schedule.
fn build_simd_schedule(world: &mut EcsMaster, dt: f32) -> Schedule {
    let mut builder = ScheduleBuilder::new(worker_pool());
    let _keys = add_physics_colored_solve(&mut builder, world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(dt)));
    builder.build(world)
}

/// Enables the O7 SIMD solve + parallel dispatch on the world's config.
fn enable_simd_solve(world: &mut EcsMaster, gravity: Vec3) {
    let cfg = world.resource_mut::<PhysicsConfig>();
    cfg.gravity = gravity;
    cfg.simd_solve = true;
    cfg.parallel_solve = true;
}

fn all_bodies(world: &mut EcsMaster) -> Vec<RigidBody> {
    let q = world.query::<&RigidBody, ()>();
    q.iter().copied().collect()
}

// ── Gate 5: penetration resolves on the SIMD path ─────────────────────────────

#[test]
fn simd_solve_resolves_penetration() {
    let mut world = EcsMaster::new();
    let (ab, am, ac) = sphere(Vec3::ZERO, Vec3::ZERO, 0.5, 1.0, 0.0, 0.0);
    spawn_body(&mut world, ab, am, ac);
    let (bb, bm, bc) =
        sphere(Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, 0.5, 1.0, 0.0, 0.0);
    spawn_body(&mut world, bb, bm, bc);

    let dt = 1.0 / 60.0;
    let mut schedule = build_simd_schedule(&mut world, dt);
    enable_simd_solve(&mut world, Vec3::ZERO);

    for _ in 0..120 {
        schedule.run(&mut world);
    }

    let bodies = all_bodies(&mut world);
    assert_eq!(bodies.len(), 2);
    let dist = (bodies[1].position - bodies[0].position).length();
    assert!(
        dist >= 1.0 - 1e-2,
        "SIMD colored solve must separate spheres to >= radius sum, got dist {dist}"
    );
    let com_x = 0.5 * (bodies[0].position.x + bodies[1].position.x);
    assert!((com_x - 0.25).abs() < 1e-3, "COM drifted under SIMD colored solve: com_x {com_x}");
}

// ── Gate 5: restitution bounces on the SIMD path ──────────────────────────────

#[test]
fn simd_solve_restitution_bounces() {
    fn run(restitution: f32) -> f32 {
        let mut world = EcsMaster::new();
        let (wb, wm, wc) =
            sphere(Vec3::ZERO, Vec3::ZERO, 0.5, 0.0, restitution, 0.0);
        spawn_body(&mut world, wb, wm, wc);
        let (mb, mm, mc) = sphere(
            Vec3::new(0.95, 0.0, 0.0),
            Vec3::new(-5.0, 0.0, 0.0),
            0.5,
            1.0,
            restitution,
            0.0,
        );
        spawn_body(&mut world, mb, mm, mc);

        let dt = 1.0 / 120.0;
        let mut schedule = build_simd_schedule(&mut world, dt);
        enable_simd_solve(&mut world, Vec3::ZERO);
        schedule.run(&mut world);

        all_bodies(&mut world)[1].linear_velocity.x
    }

    let v_no_bounce = run(0.0);
    let v_bounce = run(0.9);
    assert!(v_no_bounce > -0.5, "restitution 0 must not rebound under SIMD solve, vx {v_no_bounce}");
    assert!(v_bounce > 3.0, "restitution 0.9 must rebound under SIMD solve, vx {v_bounce}");
}

// ── Gate 5: sphere stack holds on the SIMD path ───────────────────────────────

fn spawn_sphere_stack(world: &mut EcsMaster, n: usize, r: f32) {
    let floor_r = 50.0_f32;
    let (fb, fm, fc) =
        sphere(Vec3::new(0.0, -floor_r, 0.0), Vec3::ZERO, floor_r, 0.0, 0.0, 0.6);
    spawn_body(world, fb, fm, fc);
    let overlap = 0.01_f32;
    for i in 0..n {
        let y = r + (i as f32) * (2.0 * r - overlap) - overlap;
        let (sb, sm, sc) =
            sphere(Vec3::new(0.0, y, 0.0), Vec3::ZERO, r, 1.0, 0.0, 0.6);
        spawn_body(world, sb, sm, sc);
    }
}

#[test]
fn simd_solve_sphere_stack_is_stable() {
    let n = 4usize;
    let r = 0.5_f32;
    let mut world = EcsMaster::new();
    spawn_sphere_stack(&mut world, n, r);

    let dt = 1.0 / 60.0;
    let mut schedule = build_simd_schedule(&mut world, dt);
    enable_simd_solve(&mut world, Vec3::new(0.0, -9.81, 0.0));

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
                "SIMD stack produced a non-finite position: {b:?}"
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
        "the SIMD stack must be in sustained multi-contact (else vacuous): {total_contacts}"
    );
    assert!(max_top_y <= rest_top_y + 0.25 * r, "SIMD stack drifted apart: rest {rest_top_y}, max {max_top_y}");
    assert!(min_top_y >= rest_top_y - 0.5 * r, "SIMD stack sank: rest {rest_top_y}, min {min_top_y}");
    assert!(max_pen < 0.25 * r, "SIMD inter-body penetration too deep: {max_pen}");
    assert!(max_speed_sq < 1.0, "SIMD stack jittering: max speed^2 {max_speed_sq}");
}

// ── Gate 5: box stack holds on the SIMD path (multi-point manifolds) ───────────

fn spawn_box_stack(world: &mut EcsMaster, n: usize, h: f32) {
    let floor_half = Vec3::new(20.0, 1.0, 20.0);
    let (fb, fm, fc) =
        box_body(Vec3::new(0.0, -floor_half.y, 0.0), floor_half, 0.0, 0.8);
    spawn_body(world, fb, fm, fc);
    let overlap = 0.01_f32;
    for i in 0..n {
        let y = h + (i as f32) * (2.0 * h - overlap) - overlap;
        let (bb, bm, bc) =
            box_body(Vec3::new(0.0, y, 0.0), Vec3::new(h, h, h), 1.0, 0.8);
        spawn_body(world, bb, bm, bc);
    }
}

#[test]
fn simd_solve_box_stack_is_stable() {
    // Box face-face contacts are multi-point (up to 4) → the SIMD kernel's RAGGED
    // (width-4) ranks are exercised through the full pipeline; the stack must HOLD.
    let n = 3usize;
    let h = 0.5_f32;
    let mut world = EcsMaster::new();
    spawn_box_stack(&mut world, n, h);

    let dt = 1.0 / 60.0;
    let mut schedule = build_simd_schedule(&mut world, dt);
    enable_simd_solve(&mut world, Vec3::new(0.0, -9.81, 0.0));

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
                "SIMD box stack non-finite position: {b:?}"
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
        "SIMD box stack must be in sustained multi-contact (else vacuous): {total_contacts}"
    );
    assert!(
        multi_point_frames >= 300 - 5,
        "SIMD box face-face must be multi-point nearly every frame (ragged ranks firing): {multi_point_frames}"
    );
    assert!(max_top_y <= rest_top_y + 0.25 * h, "SIMD box stack drifted: rest {rest_top_y}, max {max_top_y}");
    assert!(min_top_y >= rest_top_y - 0.5 * h, "SIMD box stack sank: rest {rest_top_y}, min {min_top_y}");
    assert!(max_pen < 0.25 * h, "SIMD inter-box penetration too deep: {max_pen}");
    assert!(max_speed_sq < 1.0, "SIMD box stack jittering: max speed^2 {max_speed_sq}");
}
