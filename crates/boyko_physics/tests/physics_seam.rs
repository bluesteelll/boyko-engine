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
use boyko_physics::math::{MAX_CONTACT_POINTS, Vec2};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{ContactPairs, PhysicsConfig, SolverScratch};
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

/// A dynamic unit-mass body at `position` with `linear_velocity`, a unit circle
/// collider.
fn dynamic_body(position: Vec2, linear_velocity: Vec2) -> (RigidBody, RigidBodyMass, Collider) {
    let body = RigidBody {
        position,
        linear_velocity,
        rotation: 0.0,
        angular_velocity: 0.0,
    };
    let mass = RigidBodyMass {
        inv_mass: 1.0,
        inv_inertia: 1.0,
        restitution: 0.5,
        friction: 0.3,
        body_type: BodyType::Dynamic,
    };
    let collider = Collider {
        shape: ColliderShape::Circle { radius: 0.5 },
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
    // Size pinned to <= 128 B (2 cache lines) — a hard const-assert in the crate;
    // re-checked here at runtime for documentation.
    assert!(size_of::<Manifold>() <= 128, "Manifold must be <= 128 B");
    assert_eq!(MAX_CONTACT_POINTS, 2, "2D foundation has 2 contact points");

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
    let (body, mass, collider) = dynamic_body(Vec2::new(1.0, 2.0), Vec2::new(3.0, 4.0));
    spawn_body(&mut world, body, mass, collider);

    // Query the hot + cold columns back and confirm the values survived.
    let q = world.query::<(&RigidBody, &RigidBodyMass), ()>();
    let mut seen = 0;
    for (b, m) in q.iter() {
        assert_eq!(b.position, Vec2::new(1.0, 2.0));
        assert_eq!(b.linear_velocity, Vec2::new(3.0, 4.0));
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
        let (b, m, c) = dynamic_body(Vec2::new(i as f32 * 100.0, 0.0), Vec2::ZERO);
        spawn_body(&mut world, b, m, c);
    }

    let dt = 1.0 / 64.0;
    let mut schedule = build_schedule::<NoopSolver>(&mut world, dt);
    // Use a known gravity so the integration is checkable.
    world.resource_mut::<PhysicsConfig>().gravity = Vec2::new(0.0, -10.0);

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
            scratch.bodies[i].linear_velocity = Vec2::ZERO;
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
    let (b, m, c) = dynamic_body(Vec2::ZERO, Vec2::new(50.0, -50.0));
    spawn_body(&mut world, b, m, c);

    let dt = 1.0 / 64.0;
    let mut schedule = build_schedule::<DummySolver>(&mut world, dt);
    // Disable gravity so the only velocity writer is the dummy solver.
    world.resource_mut::<PhysicsConfig>().gravity = Vec2::ZERO;

    schedule.run(&mut world);

    let q = world.query::<&RigidBody, ()>();
    let body = q.iter().next().expect("one body");
    assert_eq!(
        body.linear_velocity,
        Vec2::ZERO,
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
            Vec2::new(0.0, 0.0),
            Vec2::new(0.4, 0.0),
            Vec2::new(0.0, 0.4),
            Vec2::new(0.8, 0.2),
            Vec2::new(5.0, 5.0), // isolated; pairs with none
        ];
        for &p in &positions {
            let (b, m, c) = dynamic_body(p, Vec2::ZERO);
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
    assert!(a.iter().all(|&(lo, hi)| lo <= hi), "each pair is (min, max)");
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
