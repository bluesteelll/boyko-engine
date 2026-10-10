//! P3 — the cold broadphase density-policy (`select_broadphase`) gates.
//!
//! Six property groups (mirroring the P1 lighting-policy + the existing
//! `broadphase_grid.rs` AllPairs↔Grid equivalence methodology):
//!
//! 1. **Manual = 0%-gate**: in the default `Manual` select mode the policy only
//!    writes `PhysicsStats.active_body_count`; it NEVER changes
//!    `PhysicsConfig.broadphase` (the kind stays exactly as configured).
//! 2. **Auto banded hysteresis** (the tree broadphase's C4): in `Auto` mode the policy
//!    selects `Tree` at/above `AUTO_TREE_HI`, `AllPairs` at/below `AUTO_TREE_LO`, and
//!    HOLDS the current side strictly inside the band — in BOTH directions (no thrash).
//! 3. **Result transparency**: the AllPairs, Grid and Tree arms produce the IDENTICAL
//!    candidate pair set (and thus the identical narrowphase manifold input) on the
//!    same scene — so the Auto selection changes which broadphase runs, never a
//!    physics result bit. This is the P3 0%-result gate (reuses the
//!    `production_grid_equals_all_pairs` methodology).
//! 4. **G-L2-1**: on the Jolt pyramid (1240 boxes plus the slab), `Auto` never selects
//!    Grid — the scene P0b §8 measured Grid 2.46× slower on; it selects the Tree.
//! 5. **The coupling pin** (design 04 D7): the soft↔rigid coupling path forces Grid, its
//!    prerequisite, and pins `Manual`, so Auto can never overwrite the Grid there.
//! 6. **The default kind** (the tree broadphase's C4): `Tree`, under `Manual`, on the default
//!    `LeafList` query kernel; asserted without a world, so it also runs under Miri.
//!
//! The Auto sizes are derived from `AUTO_TREE_LO` / `AUTO_TREE_HI` (about 130 spheres); the
//! transparency and Manual sizes reach `GRID_HI` (3,000 spheres). A physics step through a
//! thread-pool schedule is intractable under Miri at either size, so every world-building
//! test carries a `miri-slow` ignore; the `banded` hysteresis itself stays covered under
//! Miri by `broadphase_policy.rs`'s unit tests.

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::broadphase_policy::{AUTO_TREE_HI, AUTO_TREE_LO};
use boyko_physics::broadphase_tree::{QueryKernel, TREE_BRUTE_MAX_ROWS};
use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::manifold::{BodyIndex, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::{add_physics_soft, add_physics_systems};
use boyko_physics::resources::{BroadphaseKind, ContactPairs, Manifolds, PhysicsConfig};
use boyko_physics::solver::{NoopSolver, SoftStepSolver};
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
    ignore = "miri-slow: a physics step over AUTO_TREE_HI + 8 (144) spheres with an O(n^2) AllPairs \
              broadphase, through a boyko_threadpool schedule"
)]
fn manual_mode_only_counts_never_changes_kind() {
    // At or above AUTO_TREE_HI so an Auto policy WOULD flip to the Tree — proving Manual's
    // hold is the gate, not a too-small scene.
    let n = (AUTO_TREE_HI as usize) + 8;
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
    // …but the kind was NOT touched, even though n > AUTO_TREE_HI.
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
        "Manual mode leaves a Grid-configured world on Grid (count 4 <= AUTO_TREE_LO would select \
         AllPairs in Auto)"
    );
}

// ── (2) Auto banded hysteresis (both directions) ─────────────────────────────

#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: a physics step over AUTO_TREE_HI + 8 (144) spheres through a \
              boyko_threadpool schedule"
)]
fn auto_selects_tree_above_hi() {
    let n = (AUTO_TREE_HI as usize) + 8;
    let (mut world, mut schedule) = lattice_world(n);
    world.resource_mut::<PhysicsConfig>().broadphase_select = BroadphaseSelectMode::Auto;
    // Start from AllPairs (the cold-start band).
    world.resource_mut::<PhysicsConfig>().broadphase = BroadphaseKind::AllPairs;

    schedule.run(&mut world);

    assert_eq!(
        world.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::Tree,
        "count >= AUTO_TREE_HI ⇒ Auto selects the Tree"
    );
    assert!(world.resource::<PhysicsStats>().broadphase_band, "the band latched ON");
}

/// Not red-first by construction: the selector before the tree broadphase's C4 (the Grid band
/// at 2,700 / 3,000) also selects AllPairs at `AUTO_TREE_LO - 8` spheres. What it proves is the
/// DOWN transition of the Tree band from a world started on the Tree with the band on.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: a physics step over AUTO_TREE_LO - 8 (120) spheres with an O(n^2) AllPairs \
              broadphase, through a boyko_threadpool schedule"
)]
fn auto_selects_all_pairs_below_lo() {
    let n = (AUTO_TREE_LO as usize).saturating_sub(8).max(1);
    let (mut world, mut schedule) = lattice_world(n);
    world.resource_mut::<PhysicsConfig>().broadphase_select = BroadphaseSelectMode::Auto;
    // Start the band ON (Tree) so we prove the DOWN transition, not just a no-op.
    world.resource_mut::<PhysicsConfig>().broadphase = BroadphaseKind::Tree;
    world.resource_mut::<PhysicsStats>().broadphase_band = true;

    schedule.run(&mut world);

    assert_eq!(
        world.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::AllPairs,
        "count <= AUTO_TREE_LO ⇒ Auto selects AllPairs"
    );
    assert!(!world.resource::<PhysicsStats>().broadphase_band, "the band latched OFF");
}

#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: two physics steps over (AUTO_TREE_LO + AUTO_TREE_HI) / 2 (132) spheres, \
              each through a boyko_threadpool schedule"
)]
fn auto_holds_band_inside_dead_zone_both_directions() {
    // A body count strictly inside (AUTO_TREE_LO, AUTO_TREE_HI): the band must HOLD whatever
    // side it started on (the anti-thrash dead zone).
    let mid = (AUTO_TREE_LO as usize + AUTO_TREE_HI as usize) / 2;
    assert!(
        (mid as u32) > AUTO_TREE_LO && (mid as u32) < AUTO_TREE_HI,
        "test fixture: mid must lie strictly inside the band"
    );

    // Was-ON holds ON.
    let (mut world_on, mut sched_on) = lattice_world(mid);
    world_on.resource_mut::<PhysicsConfig>().broadphase_select = BroadphaseSelectMode::Auto;
    world_on.resource_mut::<PhysicsConfig>().broadphase = BroadphaseKind::Tree;
    world_on.resource_mut::<PhysicsStats>().broadphase_band = true;
    sched_on.run(&mut world_on);
    assert_eq!(
        world_on.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::Tree,
        "inside the band a was-Tree world stays on the Tree (hysteresis hold)"
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

// ── (3) Result transparency: AllPairs == Grid == Tree ────────────────────────

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

/// Scenes at every edge either band has, and around the Tree's own brute threshold, prove the
/// selection is result-transparent at every density the Auto policy (or a Manual Grid) could
/// land on: the Grid and the Tree emit AllPairs' candidate set.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: twenty-seven physics steps at up to GRID_HI (3000) spheres, nine with an \
              O(n^2) AllPairs broadphase, through a boyko_threadpool schedule"
)]
fn all_pairs_equals_grid_and_tree_across_densities() {
    let t = TREE_BRUTE_MAX_ROWS as usize;
    let sizes = [
        1usize,
        t - 1,
        t,
        t + 1,
        AUTO_TREE_LO as usize,
        (AUTO_TREE_LO as usize + AUTO_TREE_HI as usize) / 2,
        AUTO_TREE_HI as usize,
        GRID_LO as usize,
        GRID_HI as usize,
    ];
    for n in sizes {
        let (ap_pairs, _) = run_pairs_and_manifolds(BroadphaseKind::AllPairs, n);
        assert!(n < 2 || !ap_pairs.is_empty(), "anti-vacuity: the lattice of {n} spheres has pairs");
        for kind in [BroadphaseKind::Grid, BroadphaseKind::Tree] {
            let (pairs, _) = run_pairs_and_manifolds(kind, n);
            assert_eq!(pairs, ap_pairs, "{kind:?} == AllPairs at n = {n} (result-transparent at every band)");
        }
    }
}

// ── (4) G-L2-1: Auto never selects Grid on the Jolt pyramid ──────────────────

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

/// G-L2-1: on the Jolt pyramid (1240 boxes plus the slab), `Auto` never selects Grid. P0b §8
/// measured Grid 2.46× slower than AllPairs on this scene, which behaves like the
/// size-disparity family (crossover 2,978 bodies); at the provisional band (96/192) Auto
/// switched it to Grid at +3.07 ms per step (red-first then, as `auto_keeps_all_pairs_on_the_jolt_pyramid`).
/// Since the tree broadphase's C4 Auto's high side is the Tree, so the pyramid (above
/// `AUTO_TREE_HI`) selects the Tree; red-first on the Grid band, which kept it on AllPairs.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: one physics step over 1241 boxes with a box-box narrowphase, through a \
              boyko_threadpool schedule"
)]
fn auto_never_selects_grid_on_the_jolt_pyramid() {
    let (mut world, mut schedule) = jolt_pyramid_world();
    world.resource_mut::<PhysicsConfig>().broadphase_select = BroadphaseSelectMode::Auto;
    assert_eq!(
        world.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::Tree,
        "construction: Auto cold-starts from the default Tree"
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
        BroadphaseKind::Tree,
        "G-L2-1: Auto must select the Tree for the {count}-body Jolt pyramid (AUTO_TREE_HI = \
         {AUTO_TREE_HI}), never Grid, which P0b §8 measured 2.46× slower here"
    );
    assert!(
        world.resource::<PhysicsStats>().broadphase_band,
        "G-L2-1: the band must be ON at or above AUTO_TREE_HI = {AUTO_TREE_HI}"
    );
}

// ── (5) The coupling path pins Manual (design 04 D7) ─────────────────────────

/// The soft↔rigid coupling path forces Grid, the coupled step's prerequisite (`plugin.rs`), and
/// pins `broadphase_select` to `Manual` so an Auto default could never overwrite the Grid on the
/// first step. Without a mutation the pin's assertion cannot fail (the default is `Manual`); the
/// proof is the tree broadphase C4's red-first: the default mutated to `Auto` with the pin
/// removed fails here, and with the pin kept it stays green.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: one physics step of the soft pipeline with coupling, through a \
              boyko_threadpool schedule"
)]
fn coupling_path_pins_manual_select_and_grid() {
    let mut world = EcsMaster::new();
    for i in 0..4 {
        spawn_sphere(&mut world, Vec3::new(i as f32 * 0.9, 0.0, 0.0), 0.5);
    }
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let _ = add_physics_soft::<SoftStepSolver>(&mut builder, &mut world, true);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(1.0 / 60.0)));
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;
    let mut schedule = builder.build(&mut world);
    {
        let cfg = world.resource::<PhysicsConfig>();
        assert_eq!(cfg.broadphase_select, BroadphaseSelectMode::Manual, "the coupling path pins Manual");
        assert_eq!(cfg.broadphase, BroadphaseKind::Grid, "the coupling path forces Grid");
    }

    schedule.run(&mut world);

    assert_eq!(
        world.resource::<PhysicsConfig>().broadphase,
        BroadphaseKind::Grid,
        "the coupled world is still on Grid after a step (4 bodies would select AllPairs in Auto)"
    );
    assert_eq!(world.resource::<PhysicsStats>().active_body_count, 4, "non-vacuity: the policy ran");
}

// ── (6) The default kind (the tree broadphase's C4) ──────────────────────────

/// The tree broadphase's C4: the Tree is the default kind, both in `PhysicsConfig::default()` and
/// as the enum's own default, with the Manual select mode, the serial Grid emit untouched
/// (`parallel_broadphase` off, design 04 D6) and the Morton leaf-list kernel (ruling 4 of
/// 2026-09-29; the kd order was frozen and removed by the 2026-10-01 rulings, 8). No world is
/// built, so it runs under Miri.
#[test]
fn default_broadphase_is_the_tree() {
    let cfg = PhysicsConfig::default();
    assert_eq!(cfg.broadphase, BroadphaseKind::Tree, "the default broadphase kind");
    assert_eq!(BroadphaseKind::default(), BroadphaseKind::Tree, "the enum's own default");
    assert_eq!(cfg.broadphase_select, BroadphaseSelectMode::Manual, "the default select mode");
    assert!(!cfg.parallel_broadphase, "parallel_broadphase stays off (design 04 D6)");
    assert_eq!(QueryKernel::default(), QueryKernel::LeafList, "the default query kernel (ruling 4)");
}
