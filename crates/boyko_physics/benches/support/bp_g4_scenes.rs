//! The G4 scenes of the tree broadphase, shared by the bench that times them
//! (`benches/broadphase.rs`, the `bp_g4_*` groups) and the C3b counting driver that counts their
//! queries (`tests/bp_query_counts.rs`), so both read the same bodies: a body set is built here
//! once, never transcribed.
//!
//! Moved verbatim out of `benches/broadphase.rs` (C3b); only the visibility changed.

use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_physics::broadphase_tree::BroadphaseTree;
use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{
    BodyState, BroadphaseKind, BroadphaseSelectMode, PhysicsConfig, SolverScratch,
};
use boyko_threadpool::ThreadPoolBuilder;


/// Steps of J the snapshot row is taken after: the pile has landed and its contact set is the
/// measured J set (P0 counts 9.5k pairs from step 100 on).
pub(crate) const J_SNAPSHOT_STEPS: usize = 100;
/// Untimed steps before a `tree` row is timed: with the rent rule at 1/4 a still static is
/// admitted on the third step, so the fourth is the steady state.
pub(crate) const TREE_WARM_STEPS: usize = 3;

// Jolt's pyramid, as `jolt_parity_pyramid.rs` transcribes it.
pub(crate) const BOX_SIZE: f32 = 2.0;
pub(crate) const HALF_BOX: f32 = 0.5 * BOX_SIZE;
pub(crate) const JOLT_SEPARATION: f32 = 0.5;
const JOLT_FRICTION: f32 = 0.2;
const PYRAMID_HEIGHT: i32 = 15;
const PYRAMID_BODIES: usize = 1_240;
pub(crate) const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
pub(crate) const BOX_INV_MASS: f32 = 0.125;
const BOX_INV_INERTIA: f32 = 0.1875;
const DT: f32 = 1.0 / 60.0;

/// Builds a `BodyState` carrying only the broadphase-relevant fields.
pub(crate) fn sphere(position: Vec3, radius: f32) -> BodyState {
    BodyState {
        position,
        shape: ColliderShape::Sphere { radius },
        ..Default::default()
    }
}

/// A size-disparity scene: `n` typical small bodies spread across a wide box plus
/// a few much-larger giants (radius >> the typical median). With the cell-size
/// floor DECOUPLED from `max_radius` (O2 W1), the grid resolves the many small
/// bodies into fine cells and routes the giants to the oversized hatch — so it
/// must NOT degrade to all-pairs here. With the old `2·max_radius` floor a single
/// giant would force giant cells, clustering every small body into a few coarse
/// cells (all-pairs within them). This bench is the criterion for that fix.
pub(crate) fn disparity_scene(n: usize) -> Vec<BodyState> {
    let mut bodies = Vec::with_capacity(n + 4);
    // The typical many: small spheres on a tight cubic lattice (sub-diameter
    // spacing → real overlaps). Packed densely so `cbrt(n)` is large and the
    // extent stays bounded → the median floor (`2·0.5 = 1.0`) dominates the cell
    // size and the cells stay fine. This is the decoupled-floor win condition: a
    // single giant no longer coarsens the whole grid.
    let side = (n as f64).cbrt().ceil() as usize;
    let spacing = 0.9_f32;
    let mut i = 0usize;
    'outer: for z in 0..side {
        for y in 0..side {
            for x in 0..side {
                if i >= n {
                    break 'outer;
                }
                let p = Vec3::new(x as f32 * spacing, y as f32 * spacing, z as f32 * spacing);
                bodies.push(sphere(p, 0.5));
                i += 1;
            }
        }
    }
    // The few giants (radius >> median 0.5): diameter 50 ≫ a fine cell → each
    // spans far more than MAX_CELL_SPAN cells → routed to the oversized hatch.
    // Placed inside the lattice so they overlap many small bodies (real pairs).
    let span = side as f32 * spacing;
    for k in 0..4 {
        let f = k as f32;
        bodies.push(sphere(Vec3::new(span * 0.25 + f, span * 0.5, span * 0.5 - f), 25.0));
    }
    bodies
}

/// A deterministic, moderately dense scene of `n` unit spheres on a cubic lattice
/// with a sub-cell jitter, scaled so neighbors overlap (a real candidate set).
pub(crate) fn scene(n: usize) -> Vec<BodyState> {
    let side = (n as f64).cbrt().ceil() as usize;
    // Spacing < 2·radius so adjacent lattice cells overlap → many real pairs.
    let spacing = 0.9_f32;
    let radius = 0.5_f32;
    let mut bodies = Vec::with_capacity(n);
    let mut i = 0usize;
    'outer: for z in 0..side {
        for y in 0..side {
            for x in 0..side {
                if i >= n {
                    break 'outer;
                }
                let t = i as f32;
                let jitter = Vec3::new((t * 0.13).sin() * 0.1, (t * 0.27).cos() * 0.1, 0.0);
                let p = Vec3::new(x as f32 * spacing, y as f32 * spacing, z as f32 * spacing) + jitter;
                bodies.push(sphere(p, radius));
                i += 1;
            }
        }
    }
    bodies
}

/// Makes every body dynamic, so the Tree keeps no persistent set over the scene.
pub(crate) fn make_dynamic(bodies: &mut [BodyState]) {
    for body in bodies {
        body.inv_mass = 1.0;
    }
}

/// Views a `#[repr(C)]` POD component as its bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, initialised `#[repr(C)]` POD component borrowed for the returned
    // slice's lifetime; the slice covers exactly its `size_of::<T>()` bytes, read-only, which is
    // the layout the component pool stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Spawns one box at rest into the `RigidBodyBundle` archetype, as the parity runner does: a
/// dynamic one is enabled as `Simulated`; the floor is not.
fn spawn_box(world: &mut EcsMaster, position: Vec3, dynamic: bool) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let (mass, half_extents) = if dynamic {
        (
            RigidBodyMass {
                inv_inertia: Mat3::from_diagonal(Vec3::new(
                    BOX_INV_INERTIA,
                    BOX_INV_INERTIA,
                    BOX_INV_INERTIA,
                )),
                inv_mass: BOX_INV_MASS,
                restitution: 0.0,
                friction: JOLT_FRICTION,
            },
            Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
        )
    } else {
        (
            RigidBodyMass {
                inv_inertia: Mat3::ZERO,
                inv_mass: 0.0,
                restitution: 0.0,
                friction: JOLT_FRICTION,
            },
            FLOOR_HALF_EXTENTS,
        )
    };
    let collider = Collider { shape: ColliderShape::Box { half_extents }, layer: 1, mask: 1 };
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
        .expect("construction: the RigidBodyBundle archetype accepts the three columns");
    if dynamic {
        world.enable::<Simulated>(e);
    }
}

/// The J snapshot: Jolt's pyramid on the real colored schedule with the Tree forced, sleeping
/// off, one worker, after [`J_SNAPSHOT_STEPS`] steps — the bodies the broadphase read on the
/// last of them. Contact reuse is left at its default, on since L9 C4.
pub(crate) fn j_snapshot() -> Vec<BodyState> {
    j_snapshot_with_reuse(None)
}

/// [`j_snapshot`] with [`PhysicsConfig::contact_reuse`] set to `contact_reuse` when it is `Some`
/// (`None` keeps the default). Contact reuse changes the trajectory of the
/// [`J_SNAPSHOT_STEPS`] steps, so the snapshot's bodies depend on it: `Some(false)` gives the
/// bodies of the exact narrowphase, which was the default before L9 C4.
pub(crate) fn j_snapshot_with_reuse(contact_reuse: Option<bool>) -> Vec<BodyState> {
    let mut world = EcsMaster::new();
    spawn_box(&mut world, Vec3::new(0.0, -1.0, 0.0), false);
    let mut boxes = 0usize;
    for i in 0..PYRAMID_HEIGHT {
        let lo = i / 2;
        let hi = PYRAMID_HEIGHT - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                let position = Vec3::new(
                    -(PYRAMID_HEIGHT as f32) + BOX_SIZE * j as f32 + odd,
                    1.0 + (BOX_SIZE + JOLT_SEPARATION) * i as f32,
                    -(PYRAMID_HEIGHT as f32) + BOX_SIZE * k as f32 + odd,
                );
                spawn_box(&mut world, position, true);
                boxes += 1;
            }
        }
    }
    assert_eq!(boxes, PYRAMID_BODIES, "construction: J holds exactly 1 240 boxes");
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
    let _keys = add_physics_colored_solve(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    let mut physics = builder.build(&mut world);
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
        cfg.dt = DT;
        cfg.sleeping = false;
        cfg.broadphase_select = BroadphaseSelectMode::Manual;
        cfg.broadphase = BroadphaseKind::Tree;
        if let Some(contact_reuse) = contact_reuse {
            cfg.contact_reuse = contact_reuse;
        }
    }
    world.resource_mut::<BroadphaseTree>().set_brute_max_rows(0);
    for _ in 0..J_SNAPSHOT_STEPS {
        physics.run(&mut world);
    }
    let d = world.resource::<BroadphaseTree>().diag();
    assert_eq!(d.static_rebuilds, 1, "the floor is admitted once (G2's J bound)");
    assert_eq!(d.members, 1, "the floor is the one member");
    let bodies = world.resource::<SolverScratch>().bodies().to_vec();
    assert_eq!(bodies.len(), PYRAMID_BODIES + 1, "construction: the snapshot holds J's rows");
    bodies
}
