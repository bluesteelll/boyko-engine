//! SR phase B — the solve region's Tree Borrows leg: a small pile, solved in a region on a
//! 2-worker pool with every grain term lowered, so every stage kind is cut into blocks that the
//! two participants run at once (the cut's §2.6; critique r1 W1 and critique W2: a leg whose
//! stages all ran on one participant would see none of the region's row-partitioned writes).
//!
//! # What runs
//!
//! Twenty dynamic spheres on a static floor, four of them carrying a sphere: two colours, the
//! wider one sixteen groups in two cohorts. With `wide_floor` 1, `colour_min_points` 1 and
//! `body_rows` 8, the gravity and integrate entries cut the 21 rows into three blocks, both
//! colours' sweeps into several, and the wide colour's warm start into its two cohorts. Every row's
//! restitution is 0, so no row can bounce (the store's in-region condition once it joins the
//! region).
//!
//! # The per-kind guard (critique W2, staged per commit)
//!
//! Under Miri the solver counts, per stage kind, the blocks the helpers ran and the most blocks an
//! entry of the kind had (`ColoredSoftStepSolver::region_kind_tally`, `cfg(miri)` only — natively
//! the tally does not exist). Each kind this commit's region runs — gravity, the warm start, the
//! biased and relax sweeps, integrate — must have had an entry of at least two blocks AND a helper
//! block. A mutation is read only after this guard is green: a Tree Borrows green on a run whose
//! helpers ran no block of a kind says nothing about that kind's writes.
//!
//! Natively the same scene runs (the pool route, no tally) and its bits must equal the serial
//! solve's, so the file is never an empty target.
//!
//! # Run (Tree Borrows; the pool cannot run under Stacked Borrows, phase A's finding)
//!
//! ```text
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
//!   cargo +nightly-x86_64-pc-windows-msvc miri test -p boyko-physics \
//!   --test sr_region_pile_miri -- --test-threads=1
//! ```
//!
//! `-Zmiri-ignore-leaks` for the reason `colored_rigid_scratch_miri.rs` gives (crossbeam-epoch's
//! at-exit residue from the pool's teardown). The Stacked Borrows leg of the region's physics
//! stages is the pool-free route in the crate's unit tests (`sr_region_sb_route_*`).

use boyko_physics::components::ColliderShape;
use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};
use boyko_physics::solver::ColoredSoftStepSolver;
use boyko_physics::solver::colored::RegionGrain;
use boyko_threadpool::ThreadPoolBuilder;

fn dyn_sphere(position: Vec3, lin: Vec3) -> BodyState {
    BodyState {
        inv_inertia: Mat3::from_diagonal(Vec3::new(1.5, 1.2, 1.3)),
        inv_inertia_local: Mat3::from_diagonal(Vec3::new(1.5, 1.2, 1.3)),
        position,
        linear_velocity: lin,
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

fn static_body(position: Vec3) -> BodyState {
    BodyState {
        inv_inertia: Mat3::ZERO,
        inv_inertia_local: Mat3::ZERO,
        position,
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

fn manifold(a: u32, b: u32, normal: Vec3, sep: f32, anchor: Vec3, points: u8) -> Manifold {
    let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
    m.normal = normal;
    for p in 0..usize::from(points) {
        let offset = Vec3::new(0.1 * p as f32, 0.0, 0.0);
        m.points[p] = ContactPoint {
            anchor_a: anchor + offset,
            anchor_b: anchor + offset,
            separation: sep,
            feature_id: p as u32,
        };
    }
    m.count = points;
    m
}

/// Sixteen spheres on the floor (the last row), the first four carrying one sphere each.
fn pile() -> (Vec<BodyState>, Vec<Manifold>) {
    const BOTTOMS: u32 = 16;
    const TOPS: u32 = 4;
    let mut bodies = Vec::new();
    for i in 0..BOTTOMS {
        bodies.push(dyn_sphere(Vec3::new(i as f32 * 3.0, 0.6, 0.0), Vec3::new(0.0, -0.5, 0.1)));
    }
    for i in 0..TOPS {
        bodies.push(dyn_sphere(Vec3::new(i as f32 * 3.0, 2.4, 0.0), Vec3::new(0.1, -0.5, 0.0)));
    }
    let floor = BOTTOMS + TOPS;
    bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));
    let mut manifolds = Vec::new();
    for i in 0..BOTTOMS {
        let anchor = Vec3::new(i as f32 * 3.0, 0.0, 0.0);
        manifolds.push(manifold(i, floor, Vec3::new(0.0, -1.0, 0.0), -0.2, anchor, 1 + (i % 2) as u8));
    }
    for i in 0..TOPS {
        let anchor = Vec3::new(i as f32 * 3.0, 1.5, 0.0);
        manifolds.push(manifold(i, BOTTOMS + i, Vec3::new(0.0, 1.0, 0.0), -0.1, anchor, 2));
    }
    manifolds.sort_by_key(|m| (m.body_a.0, m.body_b.0));
    (bodies, manifolds)
}

fn build_graph(bodies: &[BodyState], manifolds: &[Manifold]) -> ConstraintGraph {
    let mut g = ConstraintGraph::with_capacity(bodies.len());
    let inv_mass: Vec<f32> = bodies.iter().map(|b| b.inv_mass).collect();
    g.build(manifolds, bodies.len(), move |row| {
        (row as usize) < inv_mass.len() && inv_mass[row as usize] != 0.0
    });
    g
}

/// The stage kinds the region runs at this commit, each held to the per-kind guard under Miri.
#[cfg(miri)]
const GUARDED_KINDS: [&str; 6] = ["gravity", "warm", "biased", "integrate", "relax", "fill"];

/// Every grain term at its floor.
const LOWERED: RegionGrain =
    RegionGrain { wide_floor: 1, colour_min_points: 1, max_bpp: 6, body_bpp: 8, body_rows: 8, fill_points: 1 };

/// The pile's state bits after `steps` steps: through a region on a 2-worker pool (`region`), or
/// the serial solve. Returns the bits and the solver.
fn solve(region: bool, steps: usize, simd_solve: bool) -> (Vec<u32>, ColoredSoftStepSolver) {
    let (bodies, manifolds) = pile();
    let graph = build_graph(&bodies, &manifolds);
    let config = PhysicsConfig { dt: 1.0 / 60.0, parallel_solve: region, simd_solve, ..PhysicsConfig::default() };
    let mut solver = ColoredSoftStepSolver::default();
    solver.set_region(true);
    assert!(solver.set_region_grain(LOWERED), "construction: the lowered grain is valid");
    let mut scratch = SolverScratch::with_capacity(bodies.len());
    scratch.set_bodies(&bodies);
    let step = |scratch: &mut SolverScratch, solver: &mut ColoredSoftStepSolver| {
        for _ in 0..steps {
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored(&config, &manifolds, &graph, scratch);
        }
    };
    if region {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        pool.install(|_| step(&mut scratch, &mut solver));
    } else {
        step(&mut scratch, &mut solver);
    }
    let bits = scratch
        .bodies()
        .iter()
        .flat_map(|b| {
            [
                b.position.x,
                b.position.y,
                b.position.z,
                b.linear_velocity.x,
                b.linear_velocity.y,
                b.linear_velocity.z,
                b.angular_velocity.x,
                b.angular_velocity.y,
                b.angular_velocity.z,
            ]
        })
        .map(f32::to_bits)
        .collect();
    (bits, solver)
}

/// The region's stages on a 2-worker pool: every kind multi-block and helper-run (Miri), the
/// bits equal to the serial solve's (every build).
#[test]
fn region_pile_stages_run_on_both_participants() {
    for simd_solve in [false, true] {
        let (serial, _) = solve(false, 2, simd_solve);
        let (region, solver) = solve(true, 2, simd_solve);
        assert_eq!(solver.region_dispatches(), 2, "simd {simd_solve}: a region per step (the premise)");
        assert_eq!(region, serial, "simd {simd_solve}: the region's bits are the serial solve's");
        #[cfg(miri)]
        {
            let tally = solver.region_kind_tally();
            for name in GUARDED_KINDS {
                let k = tally.iter().find(|k| k.kind == name).expect("the tally names every kind it ran");
                let (helper, max_blocks) = (k.helper_blocks, k.max_blocks);
                assert!(
                    max_blocks >= 2 && helper > 0,
                    "simd {simd_solve}: the per-kind guard: `{name}` had at most {max_blocks} \
                     blocks and its helpers ran {helper} — a stage that ran on one participant \
                     cannot show its writes are disjoint"
                );
            }
        }
    }
}
