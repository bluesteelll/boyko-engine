//! SR phase B — `RegionGrain::is_valid` and `ColoredSoftStepSolver::set_region_grain`, the one
//! public gate on the region's grain (tester round 1: no test in the lane asserts that a refusal
//! happens; every other test passes a valid grain and `assert!`s the `true`).
//!
//! The grain decides how many blocks every entry of the region's table gets, and the region asserts
//! `n_blocks <= 8 * P` at publish, so an out-of-range term that slipped past `set_region_grain`
//! would panic inside a solving step (a zero floor divides by zero in the cut walks). The gate is
//! the only thing between the parity runner's `--sr-*` flags and that panic.
//!
//! * `every_term_out_of_range_is_refused`: each of the six terms at 0 and both factors at 9.
//! * `the_edges_of_the_range_are_accepted`: 1 for every term, and 8 / `u32::MAX` at the other end.
//! * `a_refused_grain_changes_nothing`: after a refusal the next step still cuts with the grain in
//!   force (the widest item and the published count equal the step before the refusal).
//!
//! Red-first (each mutation run on the tree that added this file): dropping the `is_valid` early
//! return in `set_region_grain` reds the first test (accepted) and the third (its refusal assert);
//! installing the grain before refusing it reds the third alone (the refused zero fill floor
//! divides by zero in the next step's fill cut); changing `body_bpp`'s
//! `<= REGION_MAX_BLOCKS_PER_PARTICIPANT` to `<` reds the second at 8, and the third, whose `FINE`
//! grain is then refused at construction.

#![cfg(not(miri))]

use boyko_physics::components::ColliderShape;
use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};
use boyko_physics::solver::ColoredSoftStepSolver;
use boyko_physics::solver::colored::RegionGrain;
use boyko_threadpool::ThreadPoolBuilder;

/// Every term at its smallest legal value.
const FLOOR: RegionGrain =
    RegionGrain { wide_floor: 1, colour_min_points: 1, max_bpp: 1, body_bpp: 1, body_rows: 1, fill_points: 1 };

/// Every term at its largest legal value.
const CEILING: RegionGrain = RegionGrain {
    wide_floor: u32::MAX,
    colour_min_points: u32::MAX,
    max_bpp: 8,
    body_bpp: 8,
    body_rows: u32::MAX,
    fill_points: u32::MAX,
};

/// A grain that cuts the test pile into several blocks per entry (every floor at 1, factors at 8).
const FINE: RegionGrain =
    RegionGrain { wide_floor: 1, colour_min_points: 1, max_bpp: 8, body_bpp: 8, body_rows: 8, fill_points: 1 };

#[test]
fn every_term_out_of_range_is_refused() {
    let bad: [(&str, RegionGrain); 8] = [
        ("wide_floor 0", RegionGrain { wide_floor: 0, ..FINE }),
        ("colour_min_points 0", RegionGrain { colour_min_points: 0, ..FINE }),
        ("max_bpp 0", RegionGrain { max_bpp: 0, ..FINE }),
        ("max_bpp 9", RegionGrain { max_bpp: 9, ..FINE }),
        ("body_bpp 0", RegionGrain { body_bpp: 0, ..FINE }),
        ("body_bpp 9", RegionGrain { body_bpp: 9, ..FINE }),
        ("body_rows 0", RegionGrain { body_rows: 0, ..FINE }),
        ("fill_points 0", RegionGrain { fill_points: 0, ..FINE }),
    ];
    for (what, grain) in bad {
        assert!(!grain.is_valid(), "{what}: `is_valid` must be false");
        let mut solver = ColoredSoftStepSolver::default();
        assert!(!solver.set_region_grain(grain), "{what}: `set_region_grain` must refuse it");
    }
}

#[test]
fn the_edges_of_the_range_are_accepted() {
    for (what, grain) in [
        ("DEFAULT", RegionGrain::DEFAULT),
        ("FLOOR", FLOOR),
        ("CEILING", CEILING),
        ("FINE", FINE),
    ] {
        assert!(grain.is_valid(), "{what}: `is_valid` must be true");
        let mut solver = ColoredSoftStepSolver::default();
        assert!(solver.set_region_grain(grain), "{what}: `set_region_grain` must accept it");
    }
}

fn dyn_sphere(position: Vec3) -> BodyState {
    BodyState {
        inv_inertia: Mat3::from_diagonal(Vec3::new(1.5, 1.2, 1.3)),
        inv_inertia_local: Mat3::from_diagonal(Vec3::new(1.5, 1.2, 1.3)),
        position,
        linear_velocity: Vec3::new(0.0, -0.5, 0.1),
        angular_velocity: Vec3::new(0.0, 0.1, 0.0),
        rotation: Quat::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.0,
        friction: 0.5,
        simulated: true,
        kinematic: false,
        is_sensor: false,
        bp_margin: 0.0,
        shape: ColliderShape::Sphere { radius: 1.0 },
    }
}

fn floor_body() -> BodyState {
    BodyState {
        inv_inertia: Mat3::ZERO,
        inv_inertia_local: Mat3::ZERO,
        position: Vec3::new(0.0, -1.0, 0.0),
        linear_velocity: Vec3::ZERO,
        angular_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        inv_mass: 0.0,
        restitution: 0.0,
        friction: 0.5,
        simulated: false,
        kinematic: false,
        is_sensor: false,
        bp_margin: 0.0,
        shape: ColliderShape::Sphere { radius: 1.0 },
    }
}

/// Twenty spheres on one floor body: one colour of twenty one-or-two-point manifolds.
fn pile() -> (Vec<BodyState>, Vec<Manifold>) {
    let n = 20u32;
    let mut bodies: Vec<BodyState> = (0..n).map(|i| dyn_sphere(Vec3::new(i as f32 * 3.0, 0.6, 0.0))).collect();
    bodies.push(floor_body());
    let mut manifolds = Vec::new();
    for i in 0..n {
        let mut m = Manifold::new(BodyIndex(i), BodyIndex(n));
        m.normal = Vec3::new(0.0, -1.0, 0.0);
        let points = 1 + (i % 2) as u8;
        for p in 0..usize::from(points) {
            let anchor = Vec3::new(i as f32 * 3.0 + 0.1 * p as f32, 0.0, 0.0);
            m.points[p] = ContactPoint { anchor_a: anchor, anchor_b: anchor, separation: -0.2, feature_id: p as u32 };
        }
        m.count = points;
        manifolds.push(m);
    }
    manifolds.sort_by_key(|m| (m.body_a.0, m.body_b.0));
    (bodies, manifolds)
}

fn graph(bodies: &[BodyState], manifolds: &[Manifold]) -> ConstraintGraph {
    let mut g = ConstraintGraph::with_capacity(bodies.len());
    let inv_mass: Vec<f32> = bodies.iter().map(|b| b.inv_mass).collect();
    g.build(manifolds, bodies.len(), move |row| (row as usize) < inv_mass.len() && inv_mass[row as usize] != 0.0);
    g
}

#[test]
fn a_refused_grain_changes_nothing() {
    let (bodies, manifolds) = pile();
    let graph = graph(&bodies, &manifolds);
    let config = PhysicsConfig { dt: 1.0 / 60.0, parallel_solve: true, ..PhysicsConfig::default() };
    let mut solver = ColoredSoftStepSolver::default();
    assert!(solver.set_region_grain(FINE), "construction: FINE is valid");
    let mut scratch = SolverScratch::with_capacity(bodies.len());
    scratch.set_bodies(&bodies);
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let (first, second) = pool.install(|_| {
        scratch.touched.reset(scratch.bodies().len());
        solver.solve_colored(&config, &manifolds, &graph, &mut scratch);
        let first = solver.last_region_report();
        // An out-of-range grain: a zero fill floor would divide by zero in the next step's cuts if it
        // were installed.
        assert!(!solver.set_region_grain(RegionGrain { fill_points: 0, ..FINE }), "the refusal under test");
        scratch.touched.reset(scratch.bodies().len());
        solver.solve_colored(&config, &manifolds, &graph, &mut scratch);
        (first, solver.last_region_report())
    });
    assert_eq!(solver.region_dispatches(), 2, "premise: a region opened on both steps");
    assert!(first.max_blocks >= 2, "premise: FINE cuts the pile into several blocks ({first:?})");
    assert_eq!(second.max_blocks, first.max_blocks, "the widest item after the refusal is the one before it");
    assert_eq!(second.published, first.published, "the published item count after the refusal is the one before it");
}
