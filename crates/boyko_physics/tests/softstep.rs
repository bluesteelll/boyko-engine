//! W2 acceptance tests for the in-house TGS-Soft solver (`SoftStepSolver`).
//!
//! These drive the full physics pipeline (gather → broadphase → narrowphase →
//! solve → apply) through a real `Schedule`, swapping the solver to prove the
//! seam is load-bearing, and assert the W2 contract: penetration resolves,
//! restitution bounces, friction resists tangential motion, the solve is
//! deterministic, and the C2 DYNAMIC-only integrate gate keeps static bodies
//! bit-identical across a step. A `tangent_basis` unit test pins the
//! degeneracy-safe friction frame (the `n ≈ ±z` floor-normal case).
//!
//! Warm-start is INTENTIONALLY ABSENT in W2 (impulses start at zero each frame);
//! that is W3.

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
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::solver::{NoopSolver, RigidSolver, SoftStepSolver};

// ── Test helpers (mirror `physics_seam.rs`) ──────────────────────────────────

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity`
/// spawn path.
///
/// # Safety
///
/// `T` must be a `#[repr(C)]` value whose bytes are a valid serialization for the
/// component pool registered under `T::component_id()` — which holds for the
/// physics components (all `#[repr(C)]`, stored by raw byte copy).
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

/// Builds the physics schedule for solver `S` and stamps the fixed timestep.
fn build_schedule<S: RigidSolver + Default>(world: &mut EcsMaster, dt: f32) -> Schedule {
    let mut builder = ScheduleBuilder::new(serial_pool());
    let _keys = add_physics_systems::<S>(&mut builder, world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(dt)));
    builder.build(world)
}

/// Reads back every body's `RigidBody` in query (= spawn) order.
fn all_bodies(world: &mut EcsMaster) -> Vec<RigidBody> {
    let q = world.query::<&RigidBody, ()>();
    q.iter().copied().collect()
}

/// Overwrites the `linear_velocity` of the body at query (= spawn) row `row` with
/// `velocity`, leaving every other field untouched.
///
/// Used by the friction tests to inject a one-shot tangential push AFTER the body
/// has settled into a steady resting contact (so the cone is measured at steady
/// normal load, not during the penetration-recovery transient).
fn set_body_velocity(world: &mut EcsMaster, row: usize, velocity: Vec3) {
    // `&mut T` (not `Mut<T>`) is the change-detection-free mutable query data, so
    // it is permitted through `EcsMaster::query` outside a system body.
    let mut q = world.query::<&mut RigidBody, ()>();
    for (i, body) in q.iter_mut().enumerate() {
        if i == row {
            body.linear_velocity = velocity;
        }
    }
}

// ── tangent_basis unit test (degeneracy-safe frame) ──────────────────────────

#[test]
fn tangent_basis_orthonormal_for_axes_and_oblique() {
    use boyko_physics::solver::contact::tangent_basis;

    let normals = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        // The KEY degeneracy case: a vertical floor normal `±z` where the naive
        // `cross(n, z)` is ZERO.
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.0, 0.0, -1.0),
        // A few oblique normals.
        Vec3::new(1.0, 1.0, 1.0).normalize(),
        Vec3::new(-2.0, 0.5, 3.0).normalize(),
        Vec3::new(0.0, 0.001, 0.999999).normalize(),
    ];

    for &n in &normals {
        let (t1, t2) = tangent_basis(n);
        // Both tangents are unit length (non-zero — the degeneracy guard).
        assert!(
            (t1.length() - 1.0).abs() < 1e-5,
            "t1 must be unit for n={n:?}, got len {}",
            t1.length()
        );
        assert!(
            (t2.length() - 1.0).abs() < 1e-5,
            "t2 must be unit for n={n:?}, got len {}",
            t2.length()
        );
        // Orthonormal frame: t1 ⟂ n, t2 ⟂ n, t1 ⟂ t2.
        assert!(t1.dot(n).abs() < 1e-5, "t1 ⟂ n failed for n={n:?}");
        assert!(t2.dot(n).abs() < 1e-5, "t2 ⟂ n failed for n={n:?}");
        assert!(t1.dot(t2).abs() < 1e-5, "t1 ⟂ t2 failed for n={n:?}");
    }
}

// ── softstep_resolves_penetration ────────────────────────────────────────────

#[test]
fn softstep_resolves_penetration() {
    // Two overlapping unit spheres (radius 0.5) along +X: centers 0.5 apart, so
    // they penetrate by 0.5. No gravity, no restitution → they must separate and
    // momentum is honored (symmetric push apart, the COM stays put).
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
    let mut schedule = build_schedule::<SoftStepSolver>(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;

    for _ in 0..120 {
        schedule.run(&mut world);
    }

    let bodies = all_bodies(&mut world);
    assert_eq!(bodies.len(), 2);
    let dist = (bodies[1].position - bodies[0].position).length();
    // Penetration resolved: centers at least the radius sum apart (within a
    // small soft-contact tolerance).
    assert!(
        dist >= 1.0 - 1e-2,
        "spheres must separate to >= radius sum, got dist {dist}"
    );
    // Momentum honored: equal-mass symmetric push keeps the COM at x = 0.25.
    let com_x = 0.5 * (bodies[0].position.x + bodies[1].position.x);
    assert!(
        (com_x - 0.25).abs() < 1e-3,
        "center of mass drifted: com_x {com_x}"
    );
    // They moved apart along +X (A left, B right).
    assert!(bodies[0].position.x < 0.0, "A pushed -X: {:?}", bodies[0]);
    assert!(bodies[1].position.x > 0.5, "B pushed +X: {:?}", bodies[1]);
}

#[test]
fn softstep_restitution_bounce_vs_no_bounce() {
    // A dynamic sphere approaching a heavy (near-static) sphere head-on along +X.
    // With restitution = 0 the approaching sphere does not rebound (separating
    // velocity ~ 0); with restitution near 1 it rebounds (separating velocity ≈
    // approach speed). Run a SINGLE step from a just-touching overlap so the
    // restitution pass operates on the captured approach velocity.
    fn run(restitution: f32) -> f32 {
        let mut world = EcsMaster::new();
        // Heavy "wall" sphere at the origin (tiny inv_mass so it barely moves).
        let (wb, wm, wc) = sphere(
            Vec3::ZERO,
            Vec3::ZERO,
            0.5,
            0.0, // static wall: inv_mass 0 (immovable)
            restitution,
            0.0,
            BodyType::Static,
        );
        spawn_body(&mut world, wb, wm, wc);
        // Mover overlapping slightly, travelling -X into the wall.
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
        let mut schedule = build_schedule::<SoftStepSolver>(&mut world, dt);
        world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;
        schedule.run(&mut world);

        let bodies = all_bodies(&mut world);
        // The mover is the dynamic body (query order = spawn order: wall, mover).
        bodies[1].linear_velocity.x
    }

    let v_no_bounce = run(0.0);
    let v_bounce = run(0.9);

    // No bounce: the mover's normal velocity is killed (≈ 0), not reversed.
    assert!(
        v_no_bounce > -0.5,
        "restitution 0 must not rebound, got vx {v_no_bounce}"
    );
    // Bounce: the mover rebounds along +X with ~restitution × approach speed.
    // The 5.0 m/s approach is well above RESTITUTION_THRESHOLD (1.0 m/s), so the
    // C1 threshold does not suppress this genuine impact.
    assert!(
        v_bounce > 3.0,
        "restitution 0.9 must rebound (~0.9 × 5 = 4.5), got vx {v_bounce}"
    );
}

// ── restitution_resting_contact_does_not_gain_energy (C1) ─────────────────────

#[test]
fn restitution_resting_contact_genuine_sphere_pair_does_not_gain_energy() {
    // C1 gate, GENUINE-CONTACT variant (tester addition; smallest resting pair).
    // This uses TWO r = 0.5 spheres resting in contact (bounding sum 1.0, centers
    // ~0.99 apart) — the minimal resting contact — so the restitution threshold
    // (C1) is genuinely exercised: a contact fires every frame. (Before the
    // BUG-W2-1 fix the broad/narrowphase hard-coded every radius to 0.5 and the
    // sibling's r = 50 floor was treated as r = 0.5, freezing its dynamic sphere;
    // that scene is now genuine too, but this minimal pair stays as the tightest
    // independent C1 witness.)
    //
    // The body must SETTLE under gravity with restitution = 0.5 and not creep /
    // jitter upward. Without the RESTITUTION_THRESHOLD guard, the per-frame
    // gravity-residual closing velocity (~|g|·dt ≈ 0.16 m/s, well below the 1.0
    // m/s threshold) would feed `v_target = -e·vn_initial > 0` and inject a small
    // separating impulse every frame, accumulating into upward drift.
    use boyko_physics::resources::Manifolds;
    let mut world = EcsMaster::new();
    // Static r = 0.5 sphere at the origin.
    let (fb, fm, fc) = sphere(Vec3::ZERO, Vec3::ZERO, 0.5, 0.0, 0.5, 0.5, BodyType::Static);
    spawn_body(&mut world, fb, fm, fc);
    // Dynamic r = 0.5 sphere resting just on top (centers ~1.0 apart, a hair
    // overlapping so the strict `separation < 0` narrowphase fires).
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
    let mut schedule = build_schedule::<SoftStepSolver>(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);

    // Settle.
    for _ in 0..30 {
        schedule.run(&mut world);
    }
    let settled_y = all_bodies(&mut world)[1].position.y;

    // Run many more frames; assert a real contact fires AND no upward creep.
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

    // Non-vacuous: a contact must actually exist (otherwise the body is frozen and
    // the C1 path never runs — the exact trap the large-floor sibling falls into).
    assert!(
        total_contacts >= 200,
        "the resting pair must be in contact every frame (else the C1 gate is vacuous): {total_contacts}"
    );
    // No upward creep / jitter (the threshold suppresses the gravity-residual
    // re-bounce).
    assert!(
        max_y <= settled_y + 1e-2,
        "resting body crept upward: settled_y {settled_y}, max_y {max_y}"
    );
    assert!(
        max_speed_sq < 1.0,
        "resting body gained energy: max speed^2 {max_speed_sq}"
    );
}

#[test]
fn restitution_resting_contact_does_not_gain_energy() {
    // C1 gate: a dynamic sphere resting on a static floor (a large static sphere)
    // under gravity with restitution = 0.5 must SETTLE — it must not creep or
    // jitter UPWARD over many frames. Without the restitution velocity threshold
    // the per-frame gravity-residual closing velocity (captured fresh in the gather
    // snapshot every frame) would feed `v_target = -e·vn_initial > 0`, injecting
    // energy every step and launching the resting body. With the threshold those
    // slow closing contacts get `e = 0` effectively, so the body rests.
    //
    // NON-VACUITY (BUG-W2-1 fix): now that broad/narrowphase read each body's real
    // radius (`body_bounding_radius` / `collider_radius` from `BodyState::shape`),
    // the r = 50 floor is sized 50 and the dynamic sphere genuinely rests on it —
    // a contact fires every frame, so the C1 restitution path is actually
    // exercised (the `total_contacts` assertion below guards against a regression
    // back to the frozen-body vacuous pass).
    //
    // FALL-INTO-CONTACT (BUG-W2-2 fix): the sphere spawns with a small GAP above
    // the floor (top surface at y = 0; sphere center at y = 1.0, so a ~0.5 gap to
    // its rest height ≈ 0.5). The solver — the SOLE integrator in owning mode —
    // makes it FALL under gravity into a stabilized resting contact, rather than
    // starting tangent (sep == 0, which the strict `sep < 0` narrowphase rejects,
    // freezing it). A long warm-up lets it settle before the energy window.
    use boyko_physics::resources::Manifolds;
    let mut world = EcsMaster::new();
    // Large static floor sphere, top surface at y = 0.
    let floor_r = 50.0_f32;
    let (fb, fm, fc) = sphere(
        Vec3::new(0.0, -floor_r, 0.0),
        Vec3::ZERO,
        floor_r,
        0.0,
        0.5,
        0.5,
        BodyType::Static,
    );
    spawn_body(&mut world, fb, fm, fc);
    // Dynamic sphere spawned ABOVE the floor with a gap so it falls in and settles
    // (rest height ≈ r = 0.5; spawn at y = 1.0 ⇒ falls ~0.5 into contact).
    let r = 0.5_f32;
    let spawn_y = 1.0_f32;
    let (sb, sm, sc) = sphere(
        Vec3::new(0.0, spawn_y, 0.0),
        Vec3::ZERO,
        r,
        1.0,
        0.5,
        0.5,
        BodyType::Dynamic,
    );
    spawn_body(&mut world, sb, sm, sc);

    let dt = 1.0 / 60.0;
    let mut schedule = build_schedule::<SoftStepSolver>(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);

    // Warm up: let the sphere FALL the gap and SETTLE into a stable resting
    // contact before sampling the settled height for the energy window.
    for _ in 0..120 {
        schedule.run(&mut world);
    }
    let settled_y = all_bodies(&mut world)[1].position.y;

    // Run MANY more frames; the height must not climb beyond the settled height
    // (no upward creep / jitter) and the vertical velocity must stay bounded
    // (no energy injection).
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

    // Non-vacuous: the resting sphere must be in contact with the large floor
    // every frame (else the body is frozen and the C1 restitution path never runs
    // — the exact trap BUG-W2-1's hard-coded radius caused).
    assert!(
        total_contacts >= 200,
        "the resting body must contact the floor every frame (else C1 is vacuous): {total_contacts}"
    );
    // No upward creep: the peak height stays at/below the settled height (small
    // tolerance for the soft-contact recovery wobble).
    assert!(
        max_y <= settled_y + 1e-2,
        "resting body crept upward: settled_y {settled_y}, max_y {max_y}"
    );
    // Bounded kinetic energy: a resting body's speed stays small (it does not
    // accelerate frame over frame). A re-bouncing body would blow past this.
    assert!(
        max_speed_sq < 1.0,
        "resting body gained energy: max speed^2 {max_speed_sq}"
    );
}

// ── seam_swap_noop_vs_softstep ───────────────────────────────────────────────

#[test]
fn seam_swap_noop_vs_softstep() {
    // The SAME overlapping-sphere scene under two solvers. Under NoopSolver the
    // spheres stay interpenetrating (integrate-only, no contact resolve); under
    // SoftStepSolver they separate. Proves the seam is load-bearing.
    fn final_distance<S: RigidSolver + Default>() -> f32 {
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
        let mut schedule = build_schedule::<S>(&mut world, dt);
        world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;
        for _ in 0..60 {
            schedule.run(&mut world);
        }
        let bodies = all_bodies(&mut world);
        (bodies[1].position - bodies[0].position).length()
    }

    let noop_dist = final_distance::<NoopSolver>();
    let softstep_dist = final_distance::<SoftStepSolver>();

    // Noop: zero gravity, zero initial velocity → no motion → still 0.5 apart
    // (interpenetrating).
    assert!(
        (noop_dist - 0.5).abs() < 1e-4,
        "NoopSolver leaves spheres interpenetrating, dist {noop_dist}"
    );
    // SoftStep: resolves the penetration → separated.
    assert!(
        softstep_dist >= 1.0 - 1e-2,
        "SoftStepSolver separates the spheres, dist {softstep_dist}"
    );
}

// ── solver_is_deterministic ──────────────────────────────────────────────────

#[test]
fn solver_is_deterministic() {
    // SCOPE: this proves same-binary float-op-order STABILITY under serialized
    // spawn — the same scene, spawned in the same order through a `num_threads(1)`
    // pool, run twice IN THIS PROCESS, ends bit-identical. It does NOT exercise the
    // real determinism hazard flagged in `systems.rs:29-35` (manifold / contact-
    // point order under PARALLEL spawn, where the `Relaxed` entity-id counter makes
    // dense-row order non-deterministic). It guards only that the solver's float op
    // sequence carries no hidden run-to-run nondeterminism (no atomics, no reduction
    // reorder, no rayon) for a fixed input ordering.
    fn run_once() -> Vec<RigidBody> {
        let mut world = EcsMaster::new();
        // A small cluster of overlapping dynamic spheres + a static floor sphere.
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
        let mut schedule = build_schedule::<SoftStepSolver>(&mut world, dt);
        for _ in 0..30 {
            schedule.run(&mut world);
        }
        all_bodies(&mut world)
    }

    let a = run_once();
    let b = run_once();
    assert_eq!(a.len(), b.len());
    for (i, (ba, bb)) in a.iter().zip(b.iter()).enumerate() {
        // Bit-identical: compare the raw f32 bits of every field.
        assert_eq!(
            ba.position.x.to_bits(),
            bb.position.x.to_bits(),
            "body {i} position.x differs"
        );
        assert_eq!(ba.position.y.to_bits(), bb.position.y.to_bits(), "body {i} pos.y");
        assert_eq!(ba.position.z.to_bits(), bb.position.z.to_bits(), "body {i} pos.z");
        assert_eq!(
            ba.linear_velocity.x.to_bits(),
            bb.linear_velocity.x.to_bits(),
            "body {i} vel.x"
        );
        assert_eq!(ba.linear_velocity.y.to_bits(), bb.linear_velocity.y.to_bits(), "body {i} vel.y");
        assert_eq!(ba.linear_velocity.z.to_bits(), bb.linear_velocity.z.to_bits(), "body {i} vel.z");
        assert_eq!(ba.rotation.x.to_bits(), bb.rotation.x.to_bits(), "body {i} rot.x");
        assert_eq!(ba.rotation.y.to_bits(), bb.rotation.y.to_bits(), "body {i} rot.y");
        assert_eq!(ba.rotation.z.to_bits(), bb.rotation.z.to_bits(), "body {i} rot.z");
        assert_eq!(ba.rotation.w.to_bits(), bb.rotation.w.to_bits(), "body {i} rot.w");
    }
}

// ── sphere_friction_on_static ────────────────────────────────────────────────

/// Drops a dynamic sphere onto a large static floor sphere, lets it SETTLE into a
/// steady resting contact, then injects a one-shot tangential push velocity and
/// measures the resulting horizontal travel `(Δx, Δz)` from the settled rest
/// position after `frames` steps.
///
/// The floor's top surface sits at `y = 0`; the sphere SPAWNS WITH A GAP (center
/// at `y ≈ 1.0`) and — under the solver's owned integration (BUG-W2-2 fix) — falls
/// under gravity into a stable resting contact during the settle phase (NO
/// tangential velocity). Only AFTER it has settled is the `push` injected, so the
/// friction cone is measured at STEADY normal load, not during the
/// penetration-recovery transient (which previously let a sub-limit push slip
/// ~0.036 before the cone stabilized). The returned displacement is relative to the
/// settled position, isolating the push-driven travel from the small settle drift.
///
/// NON-VACUITY (BUG-W2-1 / BUG-W2-2): the r = 50 floor is sized 50 (real-radius
/// broad/narrowphase) and the body genuinely falls onto and rests on it, so a
/// contact fires every measurement frame; the inner `total_contacts` assertion
/// guards every friction caller against a regression to a frozen / non-contacting
/// body that would let the friction asserts pass vacuously.
fn floor_slide_xz(friction: f32, push: Vec3, frames: usize) -> (f32, f32) {
    use boyko_physics::resources::Manifolds;
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
    // Spawn above the floor (rest height ≈ r = 0.5) so the body falls in cleanly.
    let (sb, sm, sc) = sphere(
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::ZERO,
        r,
        1.0,
        0.0,
        friction,
        BodyType::Dynamic,
    );
    spawn_body(&mut world, sb, sm, sc);

    let dt = 1.0 / 120.0;
    let mut schedule = build_schedule::<SoftStepSolver>(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);

    // Settle phase: fall the gap and stabilize the resting contact (no push) so the
    // normal load is at steady state before the tangential push is applied.
    for _ in 0..240 {
        schedule.run(&mut world);
    }
    let settled = all_bodies(&mut world)[1].position;

    // Inject the one-shot tangential push from the SETTLED state, then measure.
    set_body_velocity(&mut world, 1, push);
    let mut total_contacts = 0usize;
    for _ in 0..frames {
        schedule.run(&mut world);
        total_contacts += world.resource::<Manifolds>().manifolds.len();
    }
    // The sphere must be in real contact with the floor (else the friction cone is
    // never loaded and the tangential-motion asserts pass vacuously).
    assert!(
        total_contacts >= 1,
        "the sliding sphere must contact the floor (else friction is vacuous): {total_contacts}"
    );
    let p = all_bodies(&mut world)[1].position;
    // Displacement relative to the settled rest position (the push-driven travel).
    (p.x - settled.x, p.z - settled.z)
}

/// Drops a dynamic sphere onto a static floor, lets it SETTLE, injects a one-shot
/// tangential push of velocity `(push_x, 0, push_z)`, then returns the magnitude
/// of the contact-point SLIDING velocity at the bottom of the sphere after the
/// push, i.e. `|(v + ω × r).xz|` with `r = (0, -radius, 0)`.
///
/// This is the genuine STATIC-friction signature: a friction cone holds a
/// sub-limit push by driving the relative sliding velocity AT THE CONTACT to zero
/// (the body then rolls without slipping — its COM keeps translating because a
/// solid sphere has finite rotational inertia and W2 models no rolling
/// resistance). Asserting the COM is "held ≈ 0" is therefore physically wrong for
/// a sphere; the correct, non-vacuous static-hold witness is that the CONTACT
/// stops slipping. An over-limit push instead saturates the cone and keeps
/// slipping (kinetic friction), so this magnitude stays large.
fn settle_then_push_contact_slip(friction: f32, push: Vec3, frames: usize) -> f32 {
    use boyko_physics::resources::Manifolds;
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
    let (sb, sm, sc) = sphere(
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::ZERO,
        r,
        1.0,
        0.0,
        friction,
        BodyType::Dynamic,
    );
    spawn_body(&mut world, sb, sm, sc);

    let dt = 1.0 / 120.0;
    let mut schedule = build_schedule::<SoftStepSolver>(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);

    // Settle into a steady resting contact (no push) before applying the push.
    for _ in 0..240 {
        schedule.run(&mut world);
    }
    set_body_velocity(&mut world, 1, push);
    let mut total_contacts = 0usize;
    for _ in 0..frames {
        schedule.run(&mut world);
        total_contacts += world.resource::<Manifolds>().manifolds.len();
    }
    assert!(
        total_contacts >= 1,
        "the pushed sphere must contact the floor (else friction is vacuous): {total_contacts}"
    );
    let b = all_bodies(&mut world)[1];
    // Sliding velocity at the bottom contact point: v + ω × r, r = (0, -r, 0).
    let slip = b.linear_velocity + b.angular_velocity.cross(Vec3::new(0.0, -r, 0.0));
    (slip.x * slip.x + slip.z * slip.z).sqrt()
}

#[test]
fn sphere_friction_on_static() {
    // A dynamic sphere resting on a large static sphere (a floor proxy, since box
    // / plane contacts are W4) with a tangential push. High friction must resist
    // the tangential motion more than zero friction does (the cone is active).
    let (x_low_friction, _) = floor_slide_xz(0.0, Vec3::new(2.0, 0.0, 0.0), 120);
    let (x_high_friction, _) = floor_slide_xz(1.0, Vec3::new(2.0, 0.0, 0.0), 120);

    // With friction the sphere travels LESS far along +X (the cone resists the
    // tangential push); frictionless slides farther.
    assert!(
        x_high_friction < x_low_friction,
        "friction must resist sliding: high-µ x {x_high_friction} vs low-µ x {x_low_friction}"
    );
    // Frictionless still slid noticeably (sanity: the push did move it).
    assert!(
        x_low_friction > 0.1,
        "frictionless sphere should slide, x {x_low_friction}"
    );
}

#[test]
fn sphere_friction_static_holds_below_limit() {
    // Quantitative STATIC-friction bound, measured from a SETTLED resting contact
    // (BUG-W2-2 fix: the body falls in and stabilizes the normal load before the
    // push, so this is the steady-state cone, not the penetration-recovery
    // transient that previously let a sub-limit push appear to "slip").
    //
    // The static-hold witness is the CONTACT-POINT sliding velocity, NOT the COM
    // displacement: a solid sphere given a sub-limit push does not stay put — the
    // cone converts the push into ROLLING (the contact stops slipping while the COM
    // keeps translating, since W2 models no rolling resistance and a sphere has
    // finite rotational inertia). So "static friction holds" means the cone drives
    // the relative sliding velocity AT THE CONTACT to ≈ 0.
    //
    // Sub-limit push (0.05 m/s, well below µ·λn for the gravity-loaded normal
    // impulse): the cone arrests the contact slip almost immediately.
    let slip_held = settle_then_push_contact_slip(1.0, Vec3::new(0.05, 0.0, 0.0), 120);
    assert!(
        slip_held < 1e-3,
        "static friction must hold a sub-limit push (contact stops slipping, |slip| ≈ 0), got {slip_held}"
    );
    // Over-limit push (4.0 m/s): the cone saturates (kinetic friction) and the
    // contact keeps slipping — orders of magnitude above the held case, proving the
    // sub-limit assertion is the cone HOLDING, not a vacuous "everything is ≈ 0".
    let slip_slipping = settle_then_push_contact_slip(1.0, Vec3::new(4.0, 0.0, 0.0), 1);
    assert!(
        slip_slipping > 0.5,
        "an over-limit push must still be slipping at the contact (kinetic friction), got {slip_slipping}"
    );
}

#[test]
fn sphere_friction_2dof_cone_is_coupled() {
    // 2-DOF coupling: the friction cone clamps the 2D tangent-impulse MAGNITUDE to
    // µ·λn, NOT each tangent axis independently to µ·λn. To distinguish the cone
    // from two box clamps, push along BOTH tangents (normal is +y; tangents lie in
    // the x/z plane) with a diagonal (+x, +z) velocity, and compare against an
    // axis-aligned push of the SAME magnitude.
    //
    // Under a true 2D cone the resisted displacement magnitude is rotation-
    // invariant: a diagonal push of magnitude m is clamped to µ·λn just like an
    // axis-aligned push of magnitude m, so |displacement| matches (within
    // tolerance) and is split symmetrically across x and z. Two independent box
    // clamps would instead allow up to √2·µ·λn for a diagonal push (each axis
    // clamped to µ·λn separately), letting the diagonal case slide noticeably
    // FARTHER than the axis-aligned one.
    let speed = 4.0_f32;
    let diag = speed / 2.0_f32.sqrt();

    let (x_axis, z_axis) = floor_slide_xz(0.5, Vec3::new(speed, 0.0, 0.0), 120);
    let (x_diag, z_diag) = floor_slide_xz(0.5, Vec3::new(diag, 0.0, diag), 120);

    let axis_mag = (x_axis * x_axis + z_axis * z_axis).sqrt();
    let diag_mag = (x_diag * x_diag + z_diag * z_diag).sqrt();

    // Symmetric split: a (+x,+z) diagonal push resolves to equal x and z travel
    // (the coupled cone resists isotropically in the tangent plane).
    assert!(
        (x_diag - z_diag).abs() < 1e-2,
        "coupled cone must resist diagonal push symmetrically: x {x_diag}, z {z_diag}"
    );
    // Rotation invariance: the diagonal-push displacement magnitude matches the
    // axis-aligned one (NOT ~√2× larger, which two independent box clamps would
    // produce). A 25% band absorbs the soft-solve nonlinearity while still
    // excluding the box-clamp's ~41% excess.
    assert!(
        diag_mag <= axis_mag * 1.25,
        "diagonal push must be clamped by the 2D cone (not per-axis boxes): \
         diag |d| {diag_mag} vs axis |d| {axis_mag}"
    );
}

// ── static_body_unmoved_under_tgs (guards the C2 DYNAMIC-only gate) ───────────

#[test]
fn static_body_unmoved_under_tgs() {
    // A static body's `RigidBody` must be BIT-IDENTICAL before/after a SoftStep
    // step — the C2 gate integrates DYNAMIC bodies only, so a static floor must
    // not drift (and the pipeline's `physics_integrate` is gated off, so it does
    // not move the static body either).
    let mut world = EcsMaster::new();
    // A static floor sphere + a dynamic sphere that FALLS onto it (so a contact is
    // active and the solver actually runs on the static body's row). With the
    // real-radius narrowphase (BUG-W2-1 fix) the r = 10 floor's top surface is at
    // y = 0; the r = 0.5 sphere spawns ABOVE it at y = 1.0 (separation ≈ +0.5) and
    // — under the solver's owned integration (BUG-W2-2 fix) — falls under gravity
    // into a genuine resting contact (rest height ≈ 0.5). The dynamic row is then
    // touched (the `total_contacts` assert pins a real contact), so "static body
    // unmoved" is a non-vacuous claim AND the liveness assert below (the body
    // fell) actually holds.
    use boyko_physics::resources::Manifolds;
    let (fb, fm, fc) = sphere(
        Vec3::new(1.0, -10.0, 2.0),
        Vec3::ZERO,
        10.0,
        0.0,
        0.0,
        0.5,
        BodyType::Static,
    );
    let floor_before = fb;
    spawn_body(&mut world, fb, fm, fc);
    let spawn_y = 1.0_f32;
    let (db, dm, dc) = sphere(
        Vec3::new(1.0, spawn_y, 2.0),
        Vec3::ZERO,
        0.5,
        1.0,
        0.0,
        0.5,
        BodyType::Dynamic,
    );
    spawn_body(&mut world, db, dm, dc);

    let dt = 1.0 / 60.0;
    let mut schedule = build_schedule::<SoftStepSolver>(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);
    // Enough frames for the sphere to fall the gap and reach contact.
    let mut total_contacts = 0usize;
    for _ in 0..60 {
        schedule.run(&mut world);
        total_contacts += world.resource::<Manifolds>().manifolds.len();
    }

    // Non-vacuous: a contact must have fired on the static floor's row (otherwise
    // "static body unmoved" is trivially true for a body the solver never touched).
    assert!(
        total_contacts >= 1,
        "the dynamic sphere must contact the static floor (else 'unmoved' is vacuous): {total_contacts}"
    );

    let bodies = all_bodies(&mut world);
    let floor_after = bodies[0];
    // Bit-identical static body (no drift under the TGS step).
    assert_eq!(
        floor_after.position.x.to_bits(),
        floor_before.position.x.to_bits(),
        "static floor x drifted"
    );
    assert_eq!(
        floor_after.position.y.to_bits(),
        floor_before.position.y.to_bits(),
        "static floor y drifted (gravity leaked into a static body!)"
    );
    assert_eq!(
        floor_after.position.z.to_bits(),
        floor_before.position.z.to_bits(),
        "static floor z drifted"
    );
    assert_eq!(
        floor_after.linear_velocity.x.to_bits(),
        floor_before.linear_velocity.x.to_bits(),
        "static floor vx changed"
    );
    assert_eq!(
        floor_after.linear_velocity.y.to_bits(),
        floor_before.linear_velocity.y.to_bits(),
        "static floor vy changed (a gravity/contact impulse moved a static body!)"
    );
    assert_eq!(
        floor_after.rotation.w.to_bits(),
        floor_before.rotation.w.to_bits(),
        "static floor orientation changed"
    );
    // Liveness: the dynamic sphere actually FELL toward the floor under gravity
    // (it spawned at y = 1.0 with a gap and settled at the rest height ≈ 0.5, so
    // its final y is strictly below the spawn height). A frozen body — the
    // BUG-W2-2 failure — would still sit at y = 1.0.
    assert!(
        bodies[1].position.y < spawn_y - 0.1,
        "the dynamic sphere should have fallen toward the floor: spawn_y {spawn_y}, final y {}",
        bodies[1].position.y
    );
}

// ── free_dynamic_body_falls_under_owning_solver (BUG-W2-2 regression guard) ───

#[test]
fn free_dynamic_body_falls_under_owning_solver() {
    // BUG-W2-2 GUARD (tester addition, PERMANENT): a single dynamic sphere with NO
    // floor and NO contact must FALL under gravity when the SoftStepSolver owns
    // integration. This is the minimal, direct witness of the fix: the substep loop
    // integrates gravity (step 1) + position (step 5) for every dynamic body EVEN
    // with zero manifolds (no `manifolds.is_empty()` early-return), and `write_back`
    // touches the row so `physics_apply` writes the integrated state back. A
    // regression to the old early-return (or an untouched free row) would leave the
    // body FROZEN at its spawn position — the "froze in midair" symptom.
    //
    // The body is alone, so broadphase emits no pairs, narrowphase produces no
    // manifolds, and the solver's contact/friction/relax/restitution sweeps all
    // iterate an empty list. Only the gravity + position integration runs. After N
    // fixed steps a free body must have fallen by ≈ ½·g·T² (the discrete
    // semi-implicit Euler sum, which matches the closed form to better than 2 % at
    // this step count) and carry vy ≈ -g·T.
    let mut world = EcsMaster::new();
    let spawn = Vec3::new(0.0, 100.0, 0.0);
    let (b, m, c) = sphere(spawn, Vec3::ZERO, 0.5, 1.0, 0.0, 0.0, BodyType::Dynamic);
    spawn_body(&mut world, b, m, c);

    let dt = 1.0 / 60.0;
    let g = -9.81_f32;
    let mut schedule = build_schedule::<SoftStepSolver>(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, g, 0.0);

    // No contact ever exists for a lone body — assert that to prove the fall is
    // pure free-fall integration (not a contact response).
    use boyko_physics::resources::Manifolds;

    let steps = 60usize; // 1.0 s of simulation.
    let mut total_contacts = 0usize;
    for _ in 0..steps {
        schedule.run(&mut world);
        total_contacts += world.resource::<Manifolds>().manifolds.len();
    }

    let body = all_bodies(&mut world)[0];
    let total_t = dt * steps as f32;

    // No manifolds ever: this is genuine free fall, not a contact artifact.
    assert_eq!(
        total_contacts, 0,
        "a lone body must have NO contacts (free-fall, not contact response): {total_contacts}"
    );

    // The body actually fell (the BUG-W2-2 freeze would keep it at spawn.y).
    assert!(
        body.position.y < spawn.y - 1.0,
        "a free dynamic body must fall under the owning solver (BUG-W2-2): spawn.y {}, final y {}",
        spawn.y,
        body.position.y
    );

    // Quantitative free-fall: the drop matches ≈ ½·g·T² within 5 % (discrete
    // semi-implicit Euler over fixed steps; the analytic ½gT² is the reference).
    let drop = spawn.y - body.position.y;
    let expected_drop = 0.5 * (-g) * total_t * total_t; // +4.905 m over 1 s.
    let rel_err = (drop - expected_drop).abs() / expected_drop;
    assert!(
        rel_err < 0.05,
        "free-fall drop must match ½gT² within 5%: drop {drop}, expected {expected_drop}, rel_err {rel_err}"
    );

    // Velocity accumulated: vy ≈ -g·T (the integration ran every step, so the
    // velocity write-back is live — not a frozen zero).
    let expected_vy = g * total_t; // -9.81 m/s over 1 s.
    let vy_err = (body.linear_velocity.y - expected_vy).abs() / (-expected_vy);
    assert!(
        vy_err < 0.05,
        "free-fall velocity must match -gT within 5%: vy {}, expected {expected_vy}, vy_err {vy_err}",
        body.linear_velocity.y
    );
}
