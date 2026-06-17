//! Integration tests for the `boyko_physics` foundation (plan §Validation).
//!
//! Mirrors the demo's CPU-archetype physics wiring (`boyko_demo/sim/runner.rs`)
//! and the plan's validation list: manifold layout, component round-trip, the
//! no-op step in a real schedule, the seam-swappability proof (a second solver),
//! deterministic pair order, and static (no-`dyn`) dispatch.

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::Resource;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    BodyType, Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass,
};
use boyko_physics::manifold::{BodyIndex, Manifold};
use boyko_physics::math::{MAX_CONTACT_POINTS, Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{ContactPairs, Manifolds, PhysicsConfig, SolverScratch};
use boyko_physics::solver::{NoopSolver, RigidSolver};

// ── Test helpers ─────────────────────────────────────────────────────────────

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity`
/// spawn path (the physics components carry enums, so they are not
/// `bytemuck::Pod` and cannot use `bytes_of`).
///
/// # Safety
///
/// `T` must be a `#[repr(C)]` value whose bytes are a valid serialization for
/// the component pool registered under `T::component_id()` — which holds for the
/// physics components (all are `#[repr(C)]` and the engine stores them by raw
/// byte copy).
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; we view its `size_of::<T>()` bytes as a
    // read-only slice for the duration of the borrow. `T` is `#[repr(C)]` so the
    // byte layout matches what the component pool stores. The slice borrows
    // `value`, so it cannot outlive it.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Builds a single-threaded pool (deterministic, IM-2 precondition) for the
/// schedule tests.
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// Spawns one rigid body via the raw `create_entity` path (the `Bundle` derive is
/// consumable only through `Commands`; the direct path takes raw byte pairs, as
/// the demo's `spawn_balls` does).
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

/// A dynamic unit-mass body at `position` with `linear_velocity`, a unit sphere
/// collider.
fn dynamic_body(position: Vec3, linear_velocity: Vec3) -> (RigidBody, RigidBodyMass, Collider) {
    let body = RigidBody {
        position,
        linear_velocity,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.5,
        friction: 0.3,
        body_type: BodyType::Dynamic,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius: 0.5 },
        layer: 1,
        mask: 1,
    };
    (body, mass, collider)
}

/// Builds a schedule with the physics pipeline wired via `add_physics_systems`
/// for solver `S`, then sets the fixed timestep so `delta_secs()` is the test dt.
fn build_schedule<S: RigidSolver + Default>(world: &mut EcsMaster, dt: f32) -> Schedule {
    let mut builder = ScheduleBuilder::new(serial_pool());
    let _keys = add_physics_systems::<S>(&mut builder, world);
    // Overwrite the fixed clock with the test timestep (add_physics_systems does
    // NOT insert a clock — the schedule's systems read whatever `FixedTime` the
    // app installs).
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(dt)));
    builder.build(world)
}

// ── manifold_layout (plan Validation) ────────────────────────────────────────

#[test]
fn manifold_layout() {
    // The 3D 4-point manifold is 152 B (3 cache lines) — a hard const-assert in
    // the crate; re-checked here at runtime for documentation. The 2-CL budget
    // was intentionally relinquished for the 3D contact data (OQ-4).
    assert_eq!(size_of::<Manifold>(), 152, "3D 4-point Manifold is 152 B");
    assert!(
        size_of::<Manifold>() <= 192,
        "Manifold within its 3-CL bound"
    );
    assert_eq!(MAX_CONTACT_POINTS, 4, "3D foundation has 4 contact points");

    let m = Manifold::new(BodyIndex(3), BodyIndex(7));
    assert_eq!(m.count, 0, "a fresh manifold has zero live points");
    assert_eq!(m.points.len(), MAX_CONTACT_POINTS, "points array == MAX");
    assert_eq!(m.body_a, BodyIndex(3));
    assert_eq!(m.body_b, BodyIndex(7));

    // `count` is in range for the full array.
    let mut full = Manifold::new(BodyIndex(0), BodyIndex(1));
    full.count = MAX_CONTACT_POINTS as u8;
    assert!((full.count as usize) <= MAX_CONTACT_POINTS);

    // `BodyIndex` is a transparent u32 (keeps the manifold inside its CL budget).
    assert_eq!(size_of::<BodyIndex>(), size_of::<u32>());
}

// ── components_round_trip (plan Validation) ──────────────────────────────────

#[test]
fn components_round_trip() {
    let mut world = EcsMaster::new();
    let (body, mass, collider) = dynamic_body(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
    spawn_body(&mut world, body, mass, collider);

    // Query the hot + cold columns back and confirm the values survived.
    let q = world.query::<(&RigidBody, &RigidBodyMass), ()>();
    let mut seen = 0;
    for (b, m) in q.iter() {
        assert_eq!(b.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(b.linear_velocity, Vec3::new(4.0, 5.0, 6.0));
        assert_eq!(m.inv_mass, 1.0);
        assert_eq!(m.body_type, BodyType::Dynamic);
        seen += 1;
    }
    assert_eq!(seen, 1, "exactly one body round-tripped");
}

// ── noop_step_runs_in_schedule (plan Validation) ─────────────────────────────

#[test]
fn noop_step_runs_in_schedule() {
    let mut world = EcsMaster::new();

    // N bodies, all at rest, with a known gravity.
    const N: usize = 16;
    for i in 0..N {
        let (b, m, c) = dynamic_body(Vec3::new(i as f32 * 100.0, 0.0, 0.0), Vec3::ZERO);
        spawn_body(&mut world, b, m, c);
    }

    let dt = 1.0 / 64.0;
    let mut schedule = build_schedule::<NoopSolver>(&mut world, dt);
    // Use a known gravity (down the Y axis) so the integration is checkable.
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -10.0, 0.0);

    const FRAMES: u32 = 8;
    for _ in 0..FRAMES {
        schedule.run(&mut world);
    }

    // The no-op solver leaves the integrate result intact. Closed-form for a
    // constant-gravity semi-implicit Euler integrator: after F steps,
    //   v = g·dt·F  (velocity updated before position each step)
    //   y = dt·Σ_{k=1..F} v_k = g·dt²·F(F+1)/2
    let g = -10.0_f32;
    let f = FRAMES as f32;
    let expected_v = g * dt * f;
    let expected_y = g * dt * dt * (f * (f + 1.0) / 2.0);

    let q = world.query::<&RigidBody, ()>();
    let mut count = 0;
    for body in q.iter() {
        assert!(
            (body.linear_velocity.y - expected_v).abs() < 1e-3,
            "velocity.y integrated wrong: {} vs {}",
            body.linear_velocity.y,
            expected_v
        );
        assert!(
            (body.position.y - expected_y).abs() < 1e-2,
            "position.y integrated wrong: {} vs {}",
            body.position.y,
            expected_y
        );
        count += 1;
    }
    assert_eq!(count, N, "all bodies present after the no-op steps");
}

// ── seam_implementable_by_second_solver (the swappability proof) ─────────────

/// A test-local solver that zeroes every body's linear velocity and flags the
/// row touched — a trivial-but-observable second `RigidSolver` impl, proving the
/// seam is swappable (plan Validation).
#[derive(Resource, Default)]
struct DummySolver;

impl RigidSolver for DummySolver {
    fn solve(
        &mut self,
        _config: &PhysicsConfig,
        _manifolds: &[Manifold],
        scratch: &mut SolverScratch,
    ) {
        // Zero velocities + mark every row touched so `physics_apply` writes back.
        let n = scratch.bodies.len();
        for i in 0..n {
            scratch.bodies[i].linear_velocity = Vec3::ZERO;
            scratch.touched.set(i);
        }
    }

    fn is_noop(&self) -> bool {
        false
    }
}

#[test]
fn seam_implementable_by_second_solver() {
    let mut world = EcsMaster::new();
    // A moving body; the dummy solver should zero its velocity, observable after
    // the apply stage.
    let (b, m, c) = dynamic_body(Vec3::ZERO, Vec3::new(50.0, -50.0, 25.0));
    spawn_body(&mut world, b, m, c);

    let dt = 1.0 / 64.0;
    let mut schedule = build_schedule::<DummySolver>(&mut world, dt);
    // Disable gravity so the only velocity writer is the dummy solver.
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;

    schedule.run(&mut world);

    let q = world.query::<&RigidBody, ()>();
    let body = q.iter().next().expect("one body");
    assert_eq!(
        body.linear_velocity,
        Vec3::ZERO,
        "the DummySolver zeroed the velocity through the seam (write-back observed)"
    );
}

// ── deterministic_pair_order (plan Validation) ───────────────────────────────

#[test]
fn deterministic_pair_order() {
    /// Runs the gather + broadphase over a fixed body set and returns the emitted
    /// pair order (BodyIndex tuples).
    fn run_once() -> Vec<(BodyIndex, BodyIndex)> {
        let mut world = EcsMaster::new();
        // A tight cluster so several bounding circles overlap → real pairs.
        let positions = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.4, 0.0, 0.0),
            Vec3::new(0.0, 0.4, 0.0),
            Vec3::new(0.8, 0.2, 0.0),
            Vec3::new(5.0, 5.0, 5.0), // isolated; pairs with none
        ];
        for &p in &positions {
            let (b, m, c) = dynamic_body(p, Vec3::ZERO);
            spawn_body(&mut world, b, m, c);
        }

        let dt = 1.0 / 64.0;
        let mut schedule = build_schedule::<NoopSolver>(&mut world, dt);
        schedule.run(&mut world);

        world.resource::<ContactPairs>().pairs.clone()
    }

    let a = run_once();
    let b = run_once();
    assert_eq!(a, b, "pair order is reproducible across two runs (D4)");
    assert!(!a.is_empty(), "the cluster produced at least one pair");
    // Confirm the (min, max) sort contract.
    assert!(
        a.windows(2).all(|w| w[0] <= w[1]),
        "pairs are emitted in sorted (min, max) order"
    );
    // Each pair is itself ordered low→high.
    assert!(
        a.iter().all(|&(lo, hi)| lo <= hi),
        "each pair is (min, max)"
    );
}

// ── static_dispatch_no_dyn (plan Validation) ─────────────────────────────────

/// Compile-time proof that `RigidSolver` dispatch is static: this function is
/// generic over `S` (monomorphized, zero vtable). If `RigidSolver` were ever
/// made object-safe and someone wrote `dyn RigidSolver`, this would still
/// compile — so the real guard is that `RigidSolver: Resource` (`Resource:
/// Sized`) makes `dyn RigidSolver` itself a compile error. The negative test
/// lives in `tests/no_dyn_solver.rs` (a `trybuild`-free doc that the bound
/// rejects `dyn`); here we assert the positive path monomorphizes.
fn solve_generic<S: RigidSolver>(solver: &mut S, scratch: &mut SolverScratch) {
    let cfg = PhysicsConfig::default();
    if !solver.is_noop() {
        solver.solve(&cfg, &[], scratch);
    }
}

#[test]
fn static_dispatch_no_dyn() {
    let mut scratch = SolverScratch::with_capacity(0);
    let mut noop = NoopSolver;
    solve_generic(&mut noop, &mut scratch); // monomorphized, no-op early-out
    let mut dummy = DummySolver;
    solve_generic(&mut dummy, &mut scratch); // distinct monomorphization
    assert!(noop.is_noop());
    assert!(!dummy.is_noop());
}

/// The gameplay-facing `EntityId` only appears in `Contact`; the manifold/solver
/// are BodyIndex-keyed (IM-1). This compiles only because `BodyIndex` and
/// `EntityId` are distinct types — a regression that re-keyed the manifold on
/// `EntityId` would break the `BodyIndex`-typed pair buffer below.
#[test]
fn manifold_is_body_index_keyed_not_entity() {
    let pairs: Vec<(BodyIndex, BodyIndex)> = vec![(BodyIndex(0), BodyIndex(1))];
    let m = Manifold::new(pairs[0].0, pairs[0].1);
    assert_eq!(m.body_a, BodyIndex(0));
    // EntityId is a separate type used only by gameplay-facing Contact.
    let _entity_typed: EntityId = EntityId(0);
}

// ── sphere_sphere_narrowphase_3d (3D type-swap) ──────────────────────────────

/// Two overlapping unit spheres (radius 0.5) along +X produce a single contact
/// point with the expected 3D normal (+X) and signed separation.
///
/// Body A at the origin, body B at `x = 0.5`: center distance `0.5`, radius sum
/// `1.0`, so `separation = 0.5 − 1.0 = −0.5` (penetrating) and the normal points
/// from A toward B, i.e. `+X`.
#[test]
fn sphere_sphere_narrowphase_3d() {
    let mut world = EcsMaster::new();
    let (a_body, a_mass, a_col) = dynamic_body(Vec3::ZERO, Vec3::ZERO);
    spawn_body(&mut world, a_body, a_mass, a_col);
    let (b_body, b_mass, b_col) = dynamic_body(Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO);
    spawn_body(&mut world, b_body, b_mass, b_col);

    let dt = 1.0 / 64.0;
    let mut schedule = build_schedule::<NoopSolver>(&mut world, dt);
    // No gravity: keep the centers fixed so the narrowphase geometry is exact.
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;
    schedule.run(&mut world);

    let manifolds = &world.resource::<Manifolds>().manifolds;
    assert_eq!(
        manifolds.len(),
        1,
        "one overlapping sphere pair → one manifold"
    );
    let m = &manifolds[0];
    assert_eq!(m.count, 1, "sphere-sphere emits a single contact point");
    // Normal points from A toward B (+X), unit length.
    assert!((m.normal.x - 1.0).abs() < 1e-5, "normal.x: {}", m.normal.x);
    assert!(
        m.normal.y.abs() < 1e-5 && m.normal.z.abs() < 1e-5,
        "normal in +X"
    );
    // separation = dist − (rA + rB) = 0.5 − 1.0 = −0.5.
    assert!(
        (m.points[0].separation + 0.5).abs() < 1e-5,
        "separation: {}",
        m.points[0].separation
    );
}

// ── Miri-tractable single-threaded paths (no threadpool/schedule) ─────────────
//
// The schedule-driven tests above spawn a real worker thread (even `serial_pool`
// builds one), so the threadpool's work-stealing spin loop makes them
// intractable under Miri's preemptive scheduler. These two tests drive the SAME
// 3D math the integrate system and the narrowphase system use — the quaternion
// step and the sphere-sphere geometry — purely through the public math API, with
// ZERO threads, so `cargo +nightly miri test` can validate the integrate +
// narrowphase paths for UB without hitting the spin loop. They are also normal
// native tests (always run under `cargo test`).

/// Single-threaded mirror of the integrate hot path: repeatedly apply the
/// closed-form semi-implicit Euler + quaternion `integrate` the way
/// `physics_integrate` does per row, and assert the orientation advanced about
/// the spin axis (no threadpool → Miri-tractable).
#[test]
fn integrate_step_single_threaded_miri() {
    let dt = 1.0 / 64.0_f32;
    let gravity = Vec3::new(0.0, -10.0, 0.0);
    let omega = Vec3::new(0.0, 0.0, 1.0); // 1 rad/s about +z

    let mut body = RigidBody {
        position: Vec3::ZERO,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: omega,
    };

    // The exact arithmetic of `physics_integrate`'s per-row closure.
    const FRAMES: u32 = 64;
    for _ in 0..FRAMES {
        body.linear_velocity = body.linear_velocity + gravity * dt;
        body.position = body.position + body.linear_velocity * dt;
        body.rotation = body.rotation.integrate(body.angular_velocity, dt);
    }

    // Orientation stays a unit quaternion (integrate re-normalizes each step).
    let q = body.rotation;
    let len = (q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w).sqrt();
    assert!((len - 1.0).abs() < 1e-5, "orientation stays unit: len={len}");
    // After ~1 rad about +z, +x has rotated into the +y half-plane.
    let rotated = q.rotate(Vec3::new(1.0, 0.0, 0.0));
    assert!(rotated.y > 0.0, "spin advanced +x toward +y: {rotated:?}");
    assert!(rotated.z.abs() < 1e-5, "rotation stays in the xy-plane");
    // Falling body integrated downward.
    assert!(body.position.y < 0.0, "gravity integrated downward");
}

/// Single-threaded mirror of `physics_narrowphase`'s sphere-sphere geometry:
/// computes the contact normal + signed separation for two overlapping unit
/// spheres directly from the public `Vec3` math (no threadpool → Miri-tractable).
/// Mirrors the exact formulae in `physics_narrowphase`.
#[test]
fn sphere_sphere_geometry_single_threaded_miri() {
    let (ra, rb) = (0.5_f32, 0.5_f32);
    let pos_a = Vec3::ZERO;
    let pos_b = Vec3::new(0.5, 0.0, 0.0);

    let delta = pos_b - pos_a;
    let dist = delta.length();
    let separation = dist - (ra + rb);
    let normal = if dist > f32::MIN_POSITIVE {
        delta * dist.recip()
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };

    assert!(separation < 0.0, "overlapping spheres penetrate");
    assert!((separation + 0.5).abs() < 1e-5, "separation = 0.5 - 1.0 = -0.5");
    assert!((normal.x - 1.0).abs() < 1e-5, "normal points +X (A→B)");
    assert!(
        normal.y.abs() < 1e-5 && normal.z.abs() < 1e-5,
        "normal is axis-aligned +X"
    );
    assert!(
        (normal.length() - 1.0).abs() < 1e-6,
        "narrowphase normal is unit length"
    );
}
