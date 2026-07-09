//! BYTE-IDENTITY determinism golden — the immovable oracle for the
//! BodyType-enum → EnableTag-bit (`Simulated`) refactor (Encoding A).
//!
//! # The contract (read before touching this file)
//!
//! This test pins a single 64-bit hash of the FULL final solver state of a fixed,
//! fully-deterministic mixed scene (Dynamic spheres + Dynamic boxes colliding and
//! settling, a Static floor, and one Kinematic body in contact with the dynamic
//! pile). The hash folds, in GATHER (= archetype-row = spawn) order, every body's
//! `position (x,y,z)`, `rotation quat (x,y,z,w)`, `linear_velocity (x,y,z)`, and
//! `angular_velocity (x,y,z)` — each `f32` via `f32::to_bits()` — through a
//! splitmix64-style mixer (no float compares; the bits are the value).
//!
//! ## Why this is the refactor oracle
//!
//! The planned refactor replaces the `RigidBodyMass.body_type: BodyType` enum
//! field with the `Simulated` EnableTag bit (toggle in place, NO archetype
//! migration, NO row move, NO sort). Encoding A's whole guarantee is BYTE
//! IDENTITY: an unchanged gather order ⇒ identical row indices ⇒ identical
//! contact/pair/manifold set+order ⇒ identical island/color/manifold solve order
//! ⇒ byte-identical final state. The EnableTag toggle is exactly the no-reorder
//! mechanism that preserves the gather order, so the refactor MUST reproduce the
//! IDENTICAL [`GOLDEN`] hash below.
//!
//! ## What the refactor MAY and MUST do
//!
//! - MAY adapt the test SETUP: how a body is declared Dynamic/Static/Kinematic
//!   may change from the `BodyType` struct-literal field to the new `Simulated`
//!   (and a kinematic) bit API. Update [`mixed_scene`] accordingly.
//! - MUST NOT change [`GOLDEN`]. A different hash after the refactor means the
//!   solve order or float-op order drifted (a determinism regression), which is
//!   precisely what Encoding A forbids. If the hash changes, the refactor — not
//!   this constant — is wrong.
//!
//! ## Scope (matches the existing determinism gates, softstep.rs:554-611)
//!
//! Single-threaded pool (the IM-2 precondition: serialized spawn fixes the gather
//! order). The hash is run-to-run stable IN-PROCESS for the same binary
//! (float-op order is fixed). It is NOT a cross-platform / cross-compiler baseline
//! — like every `to_bits()` gate in this crate, it asserts same-binary
//! reproducibility, which is what the refactor must preserve.
//!
//! Spins up `boyko_threadpool` (intractable under Miri — pool is loom+Miri proven
//! in the ECS Phase-9 series), so `cfg(not(miri))`.

#![cfg(not(miri))]

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    Collider, ColliderShape, Kinematic, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::PhysicsConfig;

// ── The golden oracle ────────────────────────────────────────────────────────

/// Hash of the full final solver state of [`mixed_scene`] after [`STEPS`] steps,
/// captured on the CURRENT (pre-refactor) `BodyType`-enum code.
///
/// **DO NOT CHANGE THIS VALUE in the EnableTag refactor.** See the module-level
/// contract: Encoding A requires the refactored code to reproduce this exact hash.
const GOLDEN: u64 = 0x1575_326A_EB80_3052;

/// Fixed step count — long enough to collide, transfer momentum, and begin to
/// settle so the state is a non-trivial function of the full solve order.
const STEPS: usize = 90;

/// Fixed timestep (60 Hz), stamped once into `FixedTime`.
const DT: f32 = 1.0 / 60.0;

// ── Test helpers (mirror `colored_acceptance_o5.rs`) ─────────────────────────

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; we view its `size_of::<T>()`
    // bytes as a read-only slice bounded by the borrow — the exact layout the pool
    // stores (mirrors `colored_acceptance_o5::as_bytes`).
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A single-threaded pool (deterministic, IM-2 precondition: serialized spawn).
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
    world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("invariant: RigidBodyBundle archetype accepts the three columns")
}

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
    rotation: Quat,
    half_extents: Vec3,
    inv_mass: f32,
    restitution: f32,
    friction: f32,
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

/// Builds the production COLORED physics schedule and stamps the fixed timestep.
fn build_colored_schedule(world: &mut EcsMaster, dt: f32) -> Schedule {
    let mut builder = ScheduleBuilder::new(serial_pool());
    let _keys = add_physics_colored_solve(&mut builder, world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(dt)));
    builder.build(world)
}

/// Reads back every body's `RigidBody` in query (= gather = spawn) row order.
fn all_bodies(world: &mut EcsMaster) -> Vec<RigidBody> {
    let q = world.query::<&RigidBody, ()>();
    q.iter().copied().collect()
}

// ── The fixed scene ──────────────────────────────────────────────────────────

/// Spawns a FIXED mixed scene (Encoding-A coverage: Static + Dynamic + Kinematic
/// with ≥1 contact). Spawn order IS gather order, so the body sequence here pins
/// the determinism precondition.
///
/// Layout (all masses/positions/extents fixed):
/// - Body 0: Static floor box (`inv_mass 0`) at the origin — the ground.
/// - Bodies 1..4: three Dynamic spheres dropped at staggered x, slightly
///   overlapping in y so they collide with each other and the floor while falling.
/// - Bodies 4..6: two Dynamic boxes (one axis-aligned, one tilted) dropped onto
///   the floor — exercises the box-box / box-sphere manifold path with rotation.
/// - Body 6: one Kinematic body with a fixed leftward velocity, placed so the
///   dynamic pile contacts it (its velocity feeds the one-sided contact response;
///   its pose is NOT advanced — Kinematic motion is intentionally unbuilt today).
fn mixed_scene(world: &mut EcsMaster) {
    // The body-type SETUP now uses the EnableTag bits (Decision 1/6): an old
    // Dynamic body gets `Simulated` SET; the Static floor keeps it CLEAR (it is
    // gathered but never integrated — `is_dynamic_row(inv_mass==0)` is already
    // false); the Kinematic body keeps `Simulated` CLEAR and gets `Kinematic` SET.
    // The compound gate `simulated && is_dynamic_row(inv_mass)` reproduces the old
    // `BodyType::Dynamic && inv_mass != 0` truth value at every row, so the GOLDEN
    // hash is unchanged (Encoding A). The toggle is in place — NO archetype
    // migration, NO row move — so the gather order is byte-identical.

    // 1. Static floor — a wide thin box at y = 0. `Simulated` CLEAR.
    let (fb, fm, fc) = box_body(
        Vec3::new(0.0, 0.0, 0.0),
        Quat::IDENTITY,
        Vec3::new(10.0, 0.5, 10.0),
        0.0,
        0.0,
        0.6,
    );
    spawn_body(world, fb, fm, fc);

    // 2..4. Three Dynamic spheres (radius 0.5) dropped onto the floor at staggered
    // x so they pile and collide laterally. `Simulated` SET.
    for i in 0..3 {
        let x = -0.6 + 0.6 * i as f32;
        let y = 1.2 + 0.9 * i as f32;
        let (sb, sm, sc) = sphere(Vec3::new(x, y, 0.0), Vec3::ZERO, 0.5, 1.0, 0.2, 0.5);
        let e = spawn_body(world, sb, sm, sc);
        world.enable::<Simulated>(e);
    }

    // 5. Dynamic axis-aligned box dropped onto the floor. `Simulated` SET.
    let (b0b, b0m, b0c) = box_body(
        Vec3::new(1.5, 1.0, 0.3),
        Quat::IDENTITY,
        Vec3::new(0.4, 0.4, 0.4),
        1.0,
        0.0,
        0.5,
    );
    let e0 = spawn_body(world, b0b, b0m, b0c);
    world.enable::<Simulated>(e0);

    // 6. Dynamic tilted box dropped onto the floor (exercises rotation in the
    // manifold/solve path). `Simulated` SET.
    let (b1b, b1m, b1c) = box_body(
        Vec3::new(-1.5, 1.4, -0.3),
        quat_z(0.3),
        Vec3::new(0.4, 0.4, 0.4),
        1.0,
        0.0,
        0.5,
    );
    let e1 = spawn_body(world, b1b, b1m, b1c);
    world.enable::<Simulated>(e1);

    // 7. Kinematic body: a sphere with a fixed leftward velocity, positioned to
    // the right of the pile so the dynamic bodies contact it. Its velocity
    // participates in the one-sided contact response; its pose stays frozen.
    // `Simulated` CLEAR, `Kinematic` SET.
    let (kb, km, kc) = sphere(Vec3::new(1.1, 0.5, 0.0), Vec3::new(-1.0, 0.0, 0.0), 0.5, 0.0, 0.0, 0.6);
    let ke = spawn_body(world, kb, km, kc);
    world.enable::<Kinematic>(ke);
}

// ── The stable 64-bit fold ───────────────────────────────────────────────────

/// splitmix64 finalizer — a full-avalanche mix of a single `u64`.
fn mix64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Folds one `f32` (by its raw bits, NOT its value) into the running hash.
///
/// Using `to_bits()` makes the fold order-fixed and exact: no float compare, and
/// `-0.0`/`NaN` payloads are preserved bit-for-bit — the property the golden pins.
fn fold_f32(state: u64, value: f32) -> u64 {
    mix64(state ^ value.to_bits() as u64)
}

/// Hashes the FULL final state of every body in gather (row) order.
///
/// Folds, per body, all 13 `f32` of the rigid-body state: `position{x,y,z}`,
/// `rotation{x,y,z,w}`, `linear_velocity{x,y,z}`, `angular_velocity{x,y,z}`. The
/// iteration order is gather order (`all_bodies` is a row-order query) — the
/// canonical order every other physics stage uses, so the hash is order-fixed.
fn state_hash(bodies: &[RigidBody]) -> u64 {
    // A non-zero seed so an empty body set is not the all-zero hash.
    let mut h = mix64(0x9E37_79B9_7F4A_7C15 ^ bodies.len() as u64);
    for b in bodies {
        h = fold_f32(h, b.position.x);
        h = fold_f32(h, b.position.y);
        h = fold_f32(h, b.position.z);
        h = fold_f32(h, b.rotation.x);
        h = fold_f32(h, b.rotation.y);
        h = fold_f32(h, b.rotation.z);
        h = fold_f32(h, b.rotation.w);
        h = fold_f32(h, b.linear_velocity.x);
        h = fold_f32(h, b.linear_velocity.y);
        h = fold_f32(h, b.linear_velocity.z);
        h = fold_f32(h, b.angular_velocity.x);
        h = fold_f32(h, b.angular_velocity.y);
        h = fold_f32(h, b.angular_velocity.z);
    }
    h
}

/// Runs the fixed scene for [`STEPS`] steps and returns the final state hash.
fn run_scene_hash() -> u64 {
    let mut world = EcsMaster::new();
    mixed_scene(&mut world);

    let mut schedule = build_colored_schedule(&mut world, DT);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -9.81, 0.0);

    for _ in 0..STEPS {
        schedule.run(&mut world);
    }
    state_hash(&all_bodies(&mut world))
}

// ── The gates ────────────────────────────────────────────────────────────────

/// The IMMOVABLE oracle: the fixed scene's final-state hash must equal [`GOLDEN`].
///
/// The EnableTag (`Simulated`-bit) refactor must reproduce this exact value — see
/// the module-level contract.
#[test]
fn bodytype_determinism_golden_hash_is_stable() {
    let actual = run_scene_hash();
    assert_eq!(
        actual, GOLDEN,
        "physics golden hash changed: got {actual:#018X}, expected {GOLDEN:#018X}. \
         If this is the EnableTag (Simulated-bit) refactor, the SOLVE ORDER drifted \
         (Encoding A is violated) — fix the refactor, NOT the GOLDEN constant. \
         If this is a deliberate solver-math change, re-capture the golden."
    );
}

/// Run-to-run determinism: the same scene hashed twice in-process is identical.
///
/// Guards the precondition the golden relies on (serialized spawn ⇒ fixed gather
/// order ⇒ fixed float-op order) independently of the hardcoded value.
#[test]
fn bodytype_determinism_run_to_run_identical() {
    let a = run_scene_hash();
    let b = run_scene_hash();
    assert_eq!(a, b, "physics solve is not run-to-run deterministic: {a:#018X} != {b:#018X}");
}
