//! Phase O8 — IM-1 desync gate under topology CHURN, through the REAL physics
//! `Schedule` (gate 6 of the tester's deferred-gate list).
//!
//! The dev's in-module sanity tests drive `solve_colored_sleeping` directly; this
//! file is the only one that exercises the FULL pipeline with sleeping ON — the
//! [`physics_gather`] → solve → [`physics_apply`] chain whose `physics_apply`
//! desync `debug_assert!` ("no structural change between gather and apply") must
//! NEVER fire while bodies are spawned / despawned MID-SLEEP (merging + splitting
//! islands).
//!
//! The row-keyed sleep model must survive island renumbering: a spawn / despawn
//! between schedule runs changes the live row count AND the island ids, but
//! [`IslandSleep::begin_step`] re-sizes the per-row latch to the fresh row count and
//! re-derives the freeze decision from the (stable) rows. If the model carried a
//! volatile-id latch, a renumbered scene could mis-flag rows touched and trip the
//! apply-side desync assert. This test asserts it does not — over many spawn/despawn
//! cycles with sleeping ON.
//!
//! Tests run in the DEBUG profile, so the `debug_assert!` is LIVE: a desync would
//! PANIC and fail the test (not silently pass). The assert is the gate.
//!
//! Spins up `boyko_threadpool` (intractable under Miri — the pool is loom+Miri
//! proven in the ECS Phase-9 series), so it is `cfg(not(miri))`. The pure sleeping
//! path is covered Miri-clean by the `colored.rs` lib tests.

#![cfg(not(miri))]

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::PhysicsConfig;

// ── Test helpers (mirror `colored_acceptance_o5.rs`) ─────────────────────────

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; we view its `size_of::<T>()` bytes
    // as a read-only slice bounded by the borrow — the exact layout the pool stores
    // (mirrors `colored_acceptance_o5::as_bytes`).
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A single-threaded pool (deterministic, IM-2 precondition).
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

fn spawn_body(
    world: &mut EcsMaster,
    body: RigidBody,
    mass: RigidBodyMass,
    collider: Collider,
) -> Entity {
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
    // Decision 3/6: enable `Simulated` on every spawned body (byte-identical to the
    // old `BodyType` — a static body, inv_mass == 0, stays gated off; no kinematic
    // body is spawned here).
    world.enable::<Simulated>(e);
    e
}

#[allow(clippy::too_many_arguments)]
fn sphere(
    position: Vec3,
    radius: f32,
    inv_mass: f32,
) -> (RigidBody, RigidBodyMass, Collider) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: if inv_mass == 0.0 { Mat3::ZERO } else { Mat3::IDENTITY },
        inv_mass,
        restitution: 0.0,
        friction: 0.5,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius },
        layer: 1,
        mask: 1,
    };
    (body, mass, collider)
}

/// Builds the COLORED physics schedule with sleeping ENABLED and stamps the fixed
/// timestep.
fn build_sleeping_schedule(world: &mut EcsMaster, dt: f32) -> Schedule {
    let mut builder = ScheduleBuilder::new(serial_pool());
    let _keys = add_physics_colored_solve(&mut builder, world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(dt)));
    let schedule = builder.build(world);
    world.resource_mut::<PhysicsConfig>().sleeping = true;
    // Brisk debounce so the pile actually latches asleep within the step budget.
    world.resource_mut::<PhysicsConfig>().sleep_frames = 8;
    schedule
}

/// All live body positions in archetype-row order (the gather/apply walk order).
fn body_ys(world: &mut EcsMaster) -> Vec<f32> {
    let q = world.query::<&RigidBody, ()>();
    q.iter().map(|b| b.position.y).collect()
}

/// **Gate 6 — the IM-1 desync assert never fires under topology CHURN with sleeping
/// ON.** A resting stack settles + sleeps; then bodies are spawned (merge) and
/// despawned (split) BETWEEN schedule runs, repeatedly, while sleeping is on. The
/// `physics_apply` desync `debug_assert!` must never fire (the test would PANIC in
/// the debug profile). The row-keyed sleep model must survive island renumbering.
#[test]
fn im1_desync_never_fires_under_topology_churn_with_sleeping() {
    let mut world = EcsMaster::new();

    // A small resting stack: a static floor + two stacked dynamic spheres (radius
    // 0.5, centres at y = 0.5 and y = 1.5 over a floor top at y = 0).
    let (fb, fm, fc) = sphere(Vec3::new(0.0, -0.5, 0.0), 0.5, 0.0);
    spawn_body(&mut world, fb, fm, fc);
    let (b0, m0, c0) = sphere(Vec3::new(0.0, 0.5, 0.0), 0.5, 1.0);
    spawn_body(&mut world, b0, m0, c0);
    let (b1, m1, c1) = sphere(Vec3::new(0.0, 1.5, 0.0), 0.5, 1.0);
    spawn_body(&mut world, b1, m1, c1);

    let dt = 1.0 / 60.0;
    let mut schedule = build_sleeping_schedule(&mut world, dt);

    // Settle phase: step long enough that the pile latches asleep. If the desync
    // assert were going to fire on the steady (no-churn) sleeping path it would fire
    // here.
    for _ in 0..120 {
        schedule.run(&mut world);
    }
    let settled = body_ys(&mut world);
    assert!(
        settled.len() == 3,
        "expected the 3 settled bodies before churn, got {}",
        settled.len()
    );

    // Churn phase: repeatedly spawn (merge islands) + despawn (split islands)
    // dynamic bodies between schedule runs, with sleeping still ON. Each cycle:
    //   - spawn a faller above the pile (a new awake row → wake-on-merge),
    //   - run several steps (the new body joins / perturbs the islands),
    //   - despawn it (split),
    //   - run several more steps.
    // Throughout, `physics_apply`'s desync `debug_assert!` must not fire (a panic
    // here fails the test).
    for cycle in 0..12 {
        // Spawn a faller a little above the pile (alternating x so it sometimes
        // lands on the pile and sometimes beside it — both merge/split variants).
        let x = if cycle % 2 == 0 { 0.0 } else { 5.0 };
        let (sb, sm, sc) = sphere(Vec3::new(x, 4.0, 0.0), 0.5, 1.0);
        let faller = spawn_body(&mut world, sb, sm, sc);

        // Run with the new body present (island merge / new awake row).
        for _ in 0..15 {
            schedule.run(&mut world);
        }

        // Despawn the faller (island split / live row count drops).
        let removed = world.delete_entity(faller);
        assert!(removed, "the faller entity must be despawnable (cycle {cycle})");

        // Run again with the body gone (the per-row latch must re-key cleanly to the
        // new row count; no desync).
        for _ in 0..15 {
            schedule.run(&mut world);
        }

        // Sanity: the live body count returned to the pile size (floor + 2 stack).
        let ys = body_ys(&mut world);
        assert_eq!(
            ys.len(),
            3,
            "after despawn the live body count must return to 3 (cycle {cycle}), got {}",
            ys.len()
        );
    }

    // After all the churn the original pile is still resting near the floor (the
    // sleep machinery did not launch or sink it).
    let after = body_ys(&mut world);
    for y in &after {
        assert!(
            y.is_finite() && *y > -2.0 && *y < 10.0,
            "a body left a sane range after churn: y={y}"
        );
    }
}

/// A despawn that drops the live row count below the slept stack — the harder
/// direction of the desync assert (live rows < snapshot len would end the apply walk
/// with `row < len`). Despawn a SLEPT body and keep stepping; the assert must hold.
#[test]
fn despawning_a_slept_body_does_not_desync_apply() {
    let mut world = EcsMaster::new();
    let (fb, fm, fc) = sphere(Vec3::new(0.0, -0.5, 0.0), 0.5, 0.0);
    spawn_body(&mut world, fb, fm, fc);
    let (b0, m0, c0) = sphere(Vec3::new(0.0, 0.5, 0.0), 0.5, 1.0);
    let body0 = spawn_body(&mut world, b0, m0, c0);
    let (b1, m1, c1) = sphere(Vec3::new(3.0, 0.5, 0.0), 0.5, 1.0);
    spawn_body(&mut world, b1, m1, c1);

    let dt = 1.0 / 60.0;
    let mut schedule = build_sleeping_schedule(&mut world, dt);

    // Settle both dynamic bodies onto the floor; they latch asleep.
    for _ in 0..120 {
        schedule.run(&mut world);
    }
    assert_eq!(body_ys(&mut world).len(), 3, "3 settled bodies before despawn");

    // Despawn a now-slept body (live rows: 3 → 2). The next gather snapshots 2 rows,
    // the apply walks 2 — no desync, even though `IslandSleep` carried a latch for
    // the (now gone) higher row.
    assert!(world.delete_entity(body0), "the slept body must be despawnable");

    for _ in 0..60 {
        schedule.run(&mut world);
    }
    let ys = body_ys(&mut world);
    assert_eq!(ys.len(), 2, "after the despawn 2 bodies remain (floor + 1 dyn)");
    for y in &ys {
        assert!(y.is_finite(), "a body went non-finite after a slept-body despawn: y={y}");
    }
}
