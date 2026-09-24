//! P3 — the cold broadphase density-policy (`select_broadphase`) gates.
//!
//! Three property groups (mirroring the P1 lighting-policy + the existing
//! `broadphase_grid.rs` AllPairs↔Grid equivalence methodology):
//!
//! 1. **Manual = 0%-gate**: in the default `Manual` select mode the policy only
//!    writes `PhysicsStats.active_body_count`; it NEVER changes
//!    `PhysicsConfig.broadphase` (the kind stays exactly as configured).
//! 2. **Auto banded hysteresis**: in `Auto` mode the policy selects `Grid` at/above
//!    `GRID_HI`, `AllPairs` at/below `GRID_LO`, and HOLDS the current side strictly
//!    inside the band — in BOTH directions (no thrash).
//! 3. **Result transparency**: the AllPairs and Grid arms produce the IDENTICAL
//!    candidate pair set (and thus the identical narrowphase manifold input) on the
//!    same scene — so the Auto selection changes which broadphase runs, never a
//!    physics result bit. This is the P3 0%-result gate (reuses the
//!    `production_grid_equals_all_pairs` methodology).
//! 4. **G-L2-1**: on the Jolt pyramid (1240 boxes plus the slab), `Auto` keeps
//!    AllPairs — the scene P0b §8 measured Grid 2.46× slower on.
//!
//! Every scene size here is derived from `GRID_LO` / `GRID_HI`, so since the band was
//! calibrated to the measured size-disparity crossover (2,700 / 3,000, P0b §8) the
//! lattices hold about 3k spheres. That is intractable under Miri, so every test
//! carries a `miri-slow` ignore; the `banded` hysteresis itself stays covered under
//! Miri by `broadphase_policy.rs`'s unit tests.

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::manifold::{BodyIndex, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{BroadphaseKind, ContactPairs, Manifolds, PhysicsConfig};
use boyko_physics::solver::NoopSolver;
use boyko_physics::{BroadphaseSelectMode, GRID_HI, GRID_LO, PhysicsStats};

// ── spawn + world helpers (mirrors broadphase_grid.rs) ───────────────────────

/// Views a `#[repr(C)]` POD as raw bytes for the `create_entity` spawn path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; we view its `size_of::<T>()`
    // bytes as a read-only slice bounded by the borrow — the exact layout the
    // component pool stores (mirrors `broadphase_grid.rs::as_bytes`).
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn spawn_sphere(world: &mut EcsMaster, position: Vec3, radius: f32) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.5,
        friction: 0.3,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius },
        layer: 1,
        mask: 1,
    };
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

/// A dense-ish lattice of `n` spheres with some overlaps (so a non-empty pair set)
/// plus deterministic positions. Zero gravity + zero velocity (set on the world)
/// keep `physics_integrate` a position no-op so the broadphase sees a stable scene.
fn lattice_world(n: usize) -> (EcsMaster, Schedule) {
    let mut world = EcsMaster::new();
    let side = (n as f64).cbrt().ceil().max(1.0) as usize;
    let spacing = 0.9_f32; // sub-diameter (radius 0.5 → diameter 1.0) ⇒ real overlaps
    let mut i = 0usize;
    'outer: for z in 0..side {
        for y in 0..side {
            for x in 0..side {
                if i >= n {
                    break 'outer;
                }
                let p = Vec3::new(x as f32 * spacing, y as f32 * spacing, z as f32 * spacing);
                spawn_sphere(&mut world, p, 0.5);
                i += 1;
            }
        }
    }

    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let _ = add_physics_systems::<NoopSolver>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(1.0 / 64.0)));
    // Zero gravity ⇒ integrate is a no-op on position, so the broadphase sees the
    // authored lattice unchanged each run.
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;
    let schedule = builder.build(&mut world);
    (world, schedule)
}

// ── (1) Manual = 0%-gate ─────────────────────────────────────────────────────

#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: a physics step over GRID_HI + 8 (3008) spheres with an O(n^2) AllPairs \
              broadphase, through a boyko_threadpool schedule"
)]
fn manual_mode_only_counts_never_changes_kind() {
    // Above GRID_HI so an Auto policy WOULD flip to Grid — proving Manual's
    // hold is the gate, not a too-small scene.
    let n = (GRID_HI as usize) + 8;
    let (mut world, mut schedule) = lattice_world(n);
    // Manual is the default; assert it and pin AllPairs.
    assert_eq!(
        world.resource::<PhysicsConfig>().broadphase_select,
        BroadphaseSelectMode::Manual,
        "default select mode is Manual (the 0%-gate)"
    );
    world.resource_mut::<PhysicsConfig>().broadphase = BroadphaseKind::AllPairs;

    schedule.run(&mut world);

    // The count was written…
    assert_eq!(
        world.resource::<PhysicsStats>().active_body_count as usize,
        n,
        "the policy counts every gathered body"
    );
    // …but the kind was NOT touched, even though n > GRID_HI.
    assert_eq!(
        world.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::AllPairs,
        "Manual mode must not override the configured broadphase kind"
    );

    // Symmetric: a Manual world configured to Grid stays Grid.
    let (mut world2, mut schedule2) = lattice_world(4);
    world2.resource_mut::<PhysicsConfig>().broadphase = BroadphaseKind::Grid;
    schedule2.run(&mut world2);
    assert_eq!(
        world2.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::Grid,
        "Manual mode leaves a Grid-configured world on Grid (count 4 < GRID_LO would flip in Auto)"
    );
}

// ── (2) Auto banded hysteresis (both directions) ─────────────────────────────

#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: a physics step over GRID_HI + 8 (3008) spheres with an O(n^2) AllPairs \
              broadphase, through a boyko_threadpool schedule"
)]
fn auto_selects_grid_above_hi() {
    let n = (GRID_HI as usize) + 8;
    let (mut world, mut schedule) = lattice_world(n);
    world.resource_mut::<PhysicsConfig>().broadphase_select = BroadphaseSelectMode::Auto;
    // Start from AllPairs (the cold-start band).
    world.resource_mut::<PhysicsConfig>().broadphase = BroadphaseKind::AllPairs;

    schedule.run(&mut world);

    assert_eq!(
        world.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::Grid,
        "count >= GRID_HI ⇒ Auto selects Grid"
    );
    assert!(world.resource::<PhysicsStats>().broadphase_band, "the band latched ON");
}

#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: a physics step over GRID_LO - 8 (2692) spheres with an O(n^2) AllPairs \
              broadphase, through a boyko_threadpool schedule"
)]
fn auto_selects_all_pairs_below_lo() {
    let n = (GRID_LO as usize).saturating_sub(8).max(1);
    let (mut world, mut schedule) = lattice_world(n);
    world.resource_mut::<PhysicsConfig>().broadphase_select = BroadphaseSelectMode::Auto;
    // Start the band ON (Grid) so we prove the DOWN transition, not just a no-op.
    world.resource_mut::<PhysicsConfig>().broadphase = BroadphaseKind::Grid;
    world.resource_mut::<PhysicsStats>().broadphase_band = true;

    schedule.run(&mut world);

    assert_eq!(
        world.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::AllPairs,
        "count <= GRID_LO ⇒ Auto selects AllPairs"
    );
    assert!(!world.resource::<PhysicsStats>().broadphase_band, "the band latched OFF");
}

#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: two physics steps over (GRID_LO + GRID_HI) / 2 (2850) spheres, each \
              through a boyko_threadpool schedule"
)]
fn auto_holds_band_inside_dead_zone_both_directions() {
    // A body count strictly inside (GRID_LO, GRID_HI): the band must HOLD whatever
    // side it started on (the anti-thrash dead zone).
    let mid = (GRID_LO as usize + GRID_HI as usize) / 2;
    assert!(
        (mid as u32) > GRID_LO && (mid as u32) < GRID_HI,
        "test fixture: mid must lie strictly inside the band"
    );

    // Was-ON holds ON.
    let (mut world_on, mut sched_on) = lattice_world(mid);
    world_on.resource_mut::<PhysicsConfig>().broadphase_select = BroadphaseSelectMode::Auto;
    world_on.resource_mut::<PhysicsConfig>().broadphase = BroadphaseKind::Grid;
    world_on.resource_mut::<PhysicsStats>().broadphase_band = true;
    sched_on.run(&mut world_on);
    assert_eq!(
        world_on.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::Grid,
        "inside the band a was-Grid world stays Grid (hysteresis hold)"
    );
    assert!(world_on.resource::<PhysicsStats>().broadphase_band);

    // Was-OFF holds OFF.
    let (mut world_off, mut sched_off) = lattice_world(mid);
    world_off.resource_mut::<PhysicsConfig>().broadphase_select = BroadphaseSelectMode::Auto;
    world_off.resource_mut::<PhysicsConfig>().broadphase = BroadphaseKind::AllPairs;
    world_off.resource_mut::<PhysicsStats>().broadphase_band = false;
    sched_off.run(&mut world_off);
    assert_eq!(
        world_off.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::AllPairs,
        "inside the band a was-AllPairs world stays AllPairs (hysteresis hold)"
    );
    assert!(!world_off.resource::<PhysicsStats>().broadphase_band);
}

// ── (3) Result transparency: AllPairs == Grid (pairs AND manifolds) ──────────

/// Runs the full physics step with a FIXED `kind` (Manual mode, so the policy does
/// not override it) and returns both the broadphase candidate pairs and the
/// narrowphase manifolds — the two are the solver's input the selection must not
/// perturb.
fn run_pairs_and_manifolds(
    kind: BroadphaseKind,
    n: usize,
) -> (Vec<(BodyIndex, BodyIndex)>, Vec<Manifold>) {
    let (mut world, mut schedule) = lattice_world(n);
    // Manual + an explicit kind ⇒ the policy counts but does not override.
    world.resource_mut::<PhysicsConfig>().broadphase_select = BroadphaseSelectMode::Manual;
    world.resource_mut::<PhysicsConfig>().broadphase = kind;
    schedule.run(&mut world);
    let pairs = world.resource::<ContactPairs>().pairs().iter().copied().collect::<Vec<_>>();
    let manifolds = world.resource::<Manifolds>().manifolds().iter().collect::<Vec<_>>();
    (pairs, manifolds)
}

#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: two physics steps over GRID_HI + 16 (3016) spheres, one with an O(n^2) \
              AllPairs broadphase, through a boyko_threadpool schedule"
)]
fn all_pairs_equals_grid_pairs_and_manifolds() {
    // A scene large enough to be a meaningful Grid build and to exceed GRID_HI, so it
    // is the regime where Auto would pick Grid (about 3k spheres since the band was
    // calibrated).
    let n = (GRID_HI as usize) + 16;

    let (ap_pairs, ap_manifolds) = run_pairs_and_manifolds(BroadphaseKind::AllPairs, n);
    let (grid_pairs, grid_manifolds) = run_pairs_and_manifolds(BroadphaseKind::Grid, n);

    assert_eq!(
        grid_pairs, ap_pairs,
        "P3 gate: the Grid broadphase candidate set is bit-identical to AllPairs (result-transparent)"
    );
    assert!(!ap_pairs.is_empty(), "the lattice produced candidate pairs (anti-vacuity)");

    // The narrowphase input the solver consumes must be identical too: same pairs ⇒
    // same per-pair manifolds in the same (min, max)-sorted order.
    assert_eq!(
        grid_manifolds.len(),
        ap_manifolds.len(),
        "the two broadphases feed the narrowphase the same number of manifolds"
    );
    assert!(!ap_manifolds.is_empty(), "the overlapping lattice produced manifolds (anti-vacuity)");
    for (g, a) in grid_manifolds.iter().zip(ap_manifolds.iter()) {
        assert_eq!(g.body_a, a.body_a, "manifold body_a must match");
        assert_eq!(g.body_b, a.body_b, "manifold body_b must match");
        assert_eq!(g.count, a.count, "manifold contact-point count must match");
    }
}

/// A scene that crosses BOTH bands proves the selection is result-transparent at
/// every density the Auto policy could land on.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: eight physics steps at up to GRID_HI (3000) spheres, each with an \
              O(n^2) AllPairs broadphase, through a boyko_threadpool schedule"
)]
fn all_pairs_equals_grid_across_densities() {
    for &n in &[1usize, GRID_LO as usize, (GRID_LO as usize + GRID_HI as usize) / 2, GRID_HI as usize] {
        let (ap_pairs, _) = run_pairs_and_manifolds(BroadphaseKind::AllPairs, n);
        let (grid_pairs, _) = run_pairs_and_manifolds(BroadphaseKind::Grid, n);
        assert_eq!(
            grid_pairs, ap_pairs,
            "Grid == AllPairs at n = {n} (result-transparent at every band)"
        );
    }
}

// ── (4) G-L2-1: Auto keeps AllPairs on the Jolt pyramid ──────────────────────

/// `cBoxSize` of Jolt's `PyramidScene.h`: the pitch between neighbouring boxes in a layer.
const PYRAMID_BOX_SIZE: f32 = 2.0;
/// `cBoxSeparation`: the gap between layers of the `jolt` scene.
const PYRAMID_SEPARATION: f32 = 0.5;
/// `cPyramidHeight`: 15 layers, 1240 boxes.
const PYRAMID_HEIGHT: i32 = 15;
/// The pyramid's dynamic boxes.
const PYRAMID_BODIES: usize = 1240;

fn spawn_box(world: &mut EcsMaster, position: Vec3, half_extents: Vec3, dynamic: bool) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: if dynamic { Mat3::IDENTITY } else { Mat3::ZERO },
        inv_mass: if dynamic { 1.0 } else { 0.0 },
        restitution: 0.0,
        friction: 0.2,
    };
    let collider = Collider {
        shape: ColliderShape::Box { half_extents },
        layer: 1,
        mask: 1,
    };
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
    if dynamic {
        world.enable::<Simulated>(e);
    }
}

/// The `jolt` scene of `benches/jolt_parity_pyramid.rs`: Jolt's `PyramidScene.h` placement
/// loop, index for index, on a static (50, 1, 50) slab — the scene P0b §8 measured AllPairs
/// beating Grid on by 2.46× at W=1.
fn jolt_pyramid_world() -> (EcsMaster, Schedule) {
    let mut world = EcsMaster::new();
    spawn_box(&mut world, Vec3::new(0.0, -1.0, 0.0), Vec3::new(50.0, 1.0, 50.0), false);
    let half = 0.5 * PYRAMID_BOX_SIZE;
    let mut boxes = 0usize;
    for i in 0..PYRAMID_HEIGHT {
        let lo = i / 2;
        let hi = PYRAMID_HEIGHT - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { half } else { 0.0 };
                let position = Vec3::new(
                    -(PYRAMID_HEIGHT as f32) + PYRAMID_BOX_SIZE * j as f32 + odd,
                    1.0 + (PYRAMID_BOX_SIZE + PYRAMID_SEPARATION) * i as f32,
                    -(PYRAMID_HEIGHT as f32) + PYRAMID_BOX_SIZE * k as f32 + odd,
                );
                spawn_box(&mut world, position, Vec3::new(half, half, half), true);
                boxes += 1;
            }
        }
    }
    assert_eq!(boxes, PYRAMID_BODIES, "construction: the Jolt pyramid holds exactly 1240 boxes");

    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let _ = add_physics_systems::<NoopSolver>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(1.0 / 60.0)));
    let schedule = builder.build(&mut world);
    (world, schedule)
}

/// G-L2-1: on the Jolt pyramid (1240 boxes plus the slab), `Auto` keeps AllPairs. P0b §8
/// measured Grid 2.46× slower than AllPairs on this scene, which behaves like the
/// size-disparity family (crossover 2,978 bodies); at the provisional band (96/192) Auto
/// switched it to Grid at +3.07 ms per step. Red-first at 96/192.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: one physics step over 1241 boxes with an O(n^2) AllPairs broadphase and \
              a box-box narrowphase, through a boyko_threadpool schedule"
)]
fn auto_keeps_all_pairs_on_the_jolt_pyramid() {
    let (mut world, mut schedule) = jolt_pyramid_world();
    world.resource_mut::<PhysicsConfig>().broadphase_select = BroadphaseSelectMode::Auto;
    assert_eq!(
        world.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::AllPairs,
        "construction: Auto cold-starts from the default AllPairs"
    );

    schedule.run(&mut world);

    let count = world.resource::<PhysicsStats>().active_body_count as usize;
    assert_eq!(
        count,
        PYRAMID_BODIES + 1,
        "non-vacuity: the policy must have counted the whole scene (1240 boxes + the slab)"
    );
    assert_eq!(
        world.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::AllPairs,
        "G-L2-1: Auto selected Grid for the {count}-body Jolt pyramid, where P0b §8 measured \
         Grid 2.46× slower (GRID_LO = {GRID_LO}, GRID_HI = {GRID_HI})"
    );
    assert!(
        !world.resource::<PhysicsStats>().broadphase_band,
        "G-L2-1: the band must stay OFF below GRID_LO = {GRID_LO}"
    );
}
