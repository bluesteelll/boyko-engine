//! Phase O5 Gate 9 — colored-vs-default solve A/B at ~1k / 10k contacts.
//!
//! Measures one full solver step (build + substep loop + warm store + write-back)
//! for the colored [`ColoredSoftStepSolver`] (color-order sweep over the SoA
//! `ContactColumns`, single-threaded in O5) against the reference
//! [`SoftStepSolver`] (manifold-order Gauss-Seidel), on the IDENTICAL warmed
//! resting-stack contact set. Both are driven DIRECTLY (no schedule, no
//! threadpool) so the bench isolates the solver, not the ECS dispatch.
//!
//! O5 expectation: single-threaded colored is roughly COMPARABLE to the reference
//! (the parallel/SIMD wins land in O6/O7); this bench just confirms NO pathological
//! regression and that the zero-alloc build shows as low/no allocator jitter (a
//! tight variance band). The colored path additionally pays for the per-step
//! `ConstraintGraph::build` (partition), reported as its own line so the solve-only
//! cost is comparable.
//!
//! Anti-vacuity: every scene asserts `> 0` contacts AND `> 1` color before timing,
//! so the bench never reports a no-op.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};
use boyko_physics::solver::{ColoredSoftStepSolver, RigidSolver, SoftStepSolver};

// boyko_physics does not re-export RigidBody/Collider through a bench-friendly POD
// builder, so build BodyState via from_columns with the public component structs.
use boyko_physics::components::{BodyType, Collider, ColliderShape, RigidBody, RigidBodyMass};

/// A dynamic sphere body state at `position`.
fn dyn_sphere(position: Vec3) -> BodyState {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.0,
        friction: 0.5,
        body_type: BodyType::Dynamic,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius: 0.5 },
        layer: 1,
        mask: 1,
    };
    BodyState::from_columns(&body, &mass, &collider)
}

/// A static floor sphere.
fn static_floor() -> BodyState {
    let body = RigidBody {
        position: Vec3::new(0.0, -50.0, 0.0),
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::ZERO,
        inv_mass: 0.0,
        restitution: 0.0,
        friction: 0.5,
        body_type: BodyType::Static,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius: 50.0 },
        layer: 1,
        mask: 1,
    };
    BodyState::from_columns(&body, &mass, &collider)
}

/// A single-point penetrating manifold between rows `a` and `b`.
fn manifold(a: u32, b: u32, normal: Vec3, anchor: Vec3) -> Manifold {
    let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
    m.normal = normal;
    m.points[0] = ContactPoint {
        anchor_a: anchor,
        anchor_b: anchor,
        separation: -0.05,
        feature_id: 0,
    };
    m.count = 1;
    m
}

/// Builds a resting-pile scene with ~`n_contacts` contacts: a grid of short sphere
/// columns over one shared static floor. Each dynamic sphere contacts the one
/// below (vertical) and its lateral neighbour — a connected, multi-color graph
/// (the chromatic load of a resting pile). Returns `(bodies, manifolds)`.
fn pile_scene(n_columns: u32, height: u32) -> (Vec<BodyState>, Vec<Manifold>) {
    let n_dyn = (n_columns * height) as usize;
    let mut bodies: Vec<BodyState> = Vec::with_capacity(n_dyn + 1);
    for col in 0..n_columns {
        for h in 0..height {
            let x = col as f32 * 1.05;
            let y = 0.5 + h as f32 * 0.99;
            bodies.push(dyn_sphere(Vec3::new(x, y, 0.0)));
        }
    }
    let floor_row = n_dyn as u32;
    bodies.push(static_floor());

    let mut manifolds = Vec::new();
    let row_of = |col: u32, h: u32| col * height + h;
    for col in 0..n_columns {
        for h in 0..height {
            let r = row_of(col, h);
            if h == 0 {
                // Bottom of each column on the floor.
                manifolds.push(manifold(r, floor_row, Vec3::new(0.0, -1.0, 0.0), bodies[r as usize].position));
            } else {
                // On the sphere below (A = lower, B = upper, normal +y).
                let below = row_of(col, h - 1);
                manifolds.push(manifold(below, r, Vec3::new(0.0, 1.0, 0.0), bodies[r as usize].position));
            }
            // Lateral contact to the next column at the same height.
            if col + 1 < n_columns {
                let right = row_of(col + 1, h);
                manifolds.push(manifold(r, right, Vec3::new(1.0, 0.0, 0.0), bodies[r as usize].position));
            }
        }
    }
    (bodies, manifolds)
}

fn config() -> PhysicsConfig {
    PhysicsConfig {
        dt: 1.0 / 60.0,
        ..PhysicsConfig::default()
    }
}

fn build_graph(bodies: &[BodyState], manifolds: &[Manifold]) -> ConstraintGraph {
    let mut g = ConstraintGraph::with_capacity(bodies.len());
    let inv_mass: Vec<f32> = bodies.iter().map(|b| b.inv_mass).collect();
    g.build(manifolds, bodies.len(), move |row| {
        (row as usize) < inv_mass.len() && inv_mass[row as usize] != 0.0
    });
    g
}

fn bench_solve(c: &mut Criterion) {
    let mut group = c.benchmark_group("solve_step");

    // (columns, height) tuned to ~1k and ~10k contacts.
    for &(n_columns, height) in &[(32u32, 16u32), (128, 40)] {
        let (bodies, manifolds) = pile_scene(n_columns, height);
        let n_contacts = manifolds.len();
        let cfg = config();

        // Anti-vacuity: real contacts + a real multi-color partition.
        let graph = build_graph(&bodies, &manifolds);
        assert!(n_contacts > 0, "scene must have > 0 contacts");
        assert!(graph.n_colors() > 1, "dense scene must have > 1 color (got {})", graph.n_colors());

        group.throughput(Throughput::Elements(n_contacts as u64));

        // ── Reference: SoftStepSolver, manifold-order sweep ──────────────────
        group.bench_with_input(
            BenchmarkId::new("reference", n_contacts),
            &n_contacts,
            |b, &_n| {
                let mut solver = SoftStepSolver::default();
                let mut scratch = SolverScratch::with_capacity(bodies.len());
                scratch.bodies = bodies.clone();
                scratch.touched.reset(scratch.bodies.len());
                // Warm so the warm-start cache is at steady state.
                for _ in 0..4 {
                    scratch.touched.reset(scratch.bodies.len());
                    solver.solve(&cfg, &manifolds, &mut scratch);
                }
                b.iter(|| {
                    scratch.touched.reset(scratch.bodies.len());
                    solver.solve(black_box(&cfg), black_box(&manifolds), &mut scratch);
                });
            },
        );

        // ── Colored: ColoredSoftStepSolver, color-order sweep (solve only, the
        //    graph is prebuilt + reused — the partition cost is its own line) ──
        group.bench_with_input(
            BenchmarkId::new("colored_solve", n_contacts),
            &n_contacts,
            |b, &_n| {
                let graph = build_graph(&bodies, &manifolds);
                let mut solver = ColoredSoftStepSolver::default();
                let mut scratch = SolverScratch::with_capacity(bodies.len());
                scratch.bodies = bodies.clone();
                scratch.touched.reset(scratch.bodies.len());
                for _ in 0..4 {
                    scratch.touched.reset(scratch.bodies.len());
                    solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
                }
                b.iter(|| {
                    scratch.touched.reset(scratch.bodies.len());
                    solver.solve_colored(black_box(&cfg), black_box(&manifolds), black_box(&graph), &mut scratch);
                });
            },
        );

        // ── Colored + graph build: the full per-step colored cost (partition +
        //    solve) — what `physics_solve_colored` + `physics_build_graph` pay ─
        group.bench_with_input(
            BenchmarkId::new("colored_solve_plus_graph", n_contacts),
            &n_contacts,
            |b, &_n| {
                let inv_mass: Vec<f32> = bodies.iter().map(|bb| bb.inv_mass).collect();
                let is_dynamic = move |row: u32| (row as usize) < inv_mass.len() && inv_mass[row as usize] != 0.0;
                let mut graph = ConstraintGraph::with_capacity(bodies.len());
                let mut solver = ColoredSoftStepSolver::default();
                let mut scratch = SolverScratch::with_capacity(bodies.len());
                scratch.bodies = bodies.clone();
                for _ in 0..4 {
                    graph.build(&manifolds, scratch.bodies.len(), &is_dynamic);
                    scratch.touched.reset(scratch.bodies.len());
                    solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
                }
                b.iter(|| {
                    graph.build(black_box(&manifolds), scratch.bodies.len(), &is_dynamic);
                    scratch.touched.reset(scratch.bodies.len());
                    solver.solve_colored(black_box(&cfg), black_box(&manifolds), &graph, &mut scratch);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_solve);
criterion_main!(benches);
