//! Phase O8 Gate 9 — the sleeping headline win + the worst-case overhead.
//!
//! Three timings of one full colored solve step (build_columns + substep loop +
//! warm store + write-back) on the IDENTICAL ~10k-contact resting pile, driven
//! DIRECTLY (no schedule / threadpool) so the bench isolates the solver:
//!
//! 1. `all_awake_no_sleeping` — sleeping OFF: the pre-O8 colored solve cost (the
//!    baseline). Every island is solved + integrated every step.
//! 2. `mostly_settled_sleeping_on` — sleeping ON, the pile pre-warmed until every
//!    island latches asleep: the steady state SKIPS all solve + integrate (only the
//!    always-on gather + begin_step/end_step bookkeeping remains). THE HEADLINE WIN:
//!    this must be ≤ ~5% of (1).
//! 3. `fully_awake_sleeping_on` — sleeping ON but a scene that NEVER sleeps
//!    (`sleep_threshold = 0`, so no island ever drops below it): the worst case —
//!    the full solve PLUS the always-on sleeping bookkeeping. This must be only a
//!    SMALL overhead over (1) (no pathological regression).
//!
//! The one-time `IslandSleep` buffer grow (1024 → n rows) is warm-up, NOT per-step:
//! the slept/awake arms warm for several steps before `b.iter`, so the timed loop
//! measures the steady state.
//!
//! Anti-vacuity: the scene asserts `> 0` contacts AND `> 1` color before timing.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{
    BodyState, ConstraintGraph, IslandSleep, PhysicsConfig, SolverScratch,
};
use boyko_physics::solver::ColoredSoftStepSolver;

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
    BodyState::from_columns(&body, &mass, &collider, false)
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
    BodyState::from_columns(&body, &mass, &collider, false)
}

/// A single-point resting manifold between rows `a` and `b`.
fn manifold(a: u32, b: u32, normal: Vec3, anchor: Vec3) -> Manifold {
    let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
    m.normal = normal;
    m.points[0] = ContactPoint {
        anchor_a: anchor,
        anchor_b: anchor,
        separation: -0.001, // a shallow, resting contact (near-zero energy)
        feature_id: 0,
    };
    m.count = 1;
    m
}

/// A resting-pile scene (mirrors `colored_solve::pile_scene`): a grid of short
/// sphere columns over one shared static floor, with vertical + lateral contacts.
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
                manifolds.push(manifold(r, floor_row, Vec3::new(0.0, -1.0, 0.0), bodies[r as usize].position));
            } else {
                let below = row_of(col, h - 1);
                manifolds.push(manifold(below, r, Vec3::new(0.0, 1.0, 0.0), bodies[r as usize].position));
            }
            if col + 1 < n_columns {
                let right = row_of(col + 1, h);
                manifolds.push(manifold(r, right, Vec3::new(1.0, 0.0, 0.0), bodies[r as usize].position));
            }
        }
    }
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

fn bench_sleeping(c: &mut Criterion) {
    let mut group = c.benchmark_group("sleeping_step");

    // ~10k contacts (128 columns × 40 high ≈ 10k contacts) — the headline scale.
    for &(n_columns, height) in &[(32u32, 16u32), (128, 40)] {
        let (bodies, manifolds) = pile_scene(n_columns, height);
        let n_contacts = manifolds.len();
        let graph = build_graph(&bodies, &manifolds);
        assert!(n_contacts > 0, "scene must have > 0 contacts");
        assert!(graph.n_colors() > 1, "dense scene must have > 1 color (got {})", graph.n_colors());

        group.throughput(Throughput::Elements(n_contacts as u64));

        // ── (1) Baseline: sleeping OFF, every island solved every step ───────
        group.bench_with_input(
            BenchmarkId::new("all_awake_no_sleeping", n_contacts),
            &n_contacts,
            |b, &_n| {
                let cfg = PhysicsConfig {
                    dt: 1.0 / 60.0,
                    ..PhysicsConfig::default()
                };
                let mut solver = ColoredSoftStepSolver::default();
                let mut scratch = SolverScratch::with_capacity(bodies.len());
                scratch.set_bodies(&bodies);
                for _ in 0..4 {
                    scratch.touched.reset(scratch.bodies().len());
                    solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
                }
                b.iter(|| {
                    scratch.touched.reset(scratch.bodies().len());
                    solver.solve_colored(black_box(&cfg), black_box(&manifolds), black_box(&graph), &mut scratch);
                });
            },
        );

        // ── (2) THE HEADLINE WIN: sleeping ON + the pile pre-warmed asleep ───
        //    Warm for > sleep_frames steps so every island latches asleep; the
        //    timed step then SKIPS all solve + integrate (only gather-equivalent
        //    bookkeeping remains). The IslandSleep buffer grow is in the warm-up.
        group.bench_with_input(
            BenchmarkId::new("mostly_settled_sleeping_on", n_contacts),
            &n_contacts,
            |b, &_n| {
                let cfg = PhysicsConfig {
                    dt: 1.0 / 60.0,
                    sleeping: true,
                    sleep_frames: 4, // brisk so the warm-up sleeps the pile quickly
                    ..PhysicsConfig::default()
                };
                let mut solver = ColoredSoftStepSolver::default();
                let mut scratch = SolverScratch::with_capacity(bodies.len());
                scratch.set_bodies(&bodies);
                let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
                // Warm: settle + latch the whole pile asleep (and grow the buffers).
                for _ in 0..16 {
                    scratch.touched.reset(scratch.bodies().len());
                    solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
                }
                b.iter(|| {
                    scratch.touched.reset(scratch.bodies().len());
                    solver.solve_colored_sleeping(
                        black_box(&cfg),
                        black_box(&manifolds),
                        black_box(&graph),
                        &mut scratch,
                        &mut sleep,
                    );
                });
            },
        );

        // ── (3) WORST CASE: sleeping ON but never sleeps (threshold 0) ───────
        //    Full solve + integrate PLUS the always-on sleeping bookkeeping — the
        //    overhead this must NOT pathologically regress over (1).
        group.bench_with_input(
            BenchmarkId::new("fully_awake_sleeping_on", n_contacts),
            &n_contacts,
            |b, &_n| {
                let cfg = PhysicsConfig {
                    dt: 1.0 / 60.0,
                    sleeping: true,
                    sleep_threshold: 0.0, // nothing ever drops below ⇒ never sleeps
                    ..PhysicsConfig::default()
                };
                let mut solver = ColoredSoftStepSolver::default();
                let mut scratch = SolverScratch::with_capacity(bodies.len());
                scratch.set_bodies(&bodies);
                let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
                for _ in 0..4 {
                    scratch.touched.reset(scratch.bodies().len());
                    solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
                }
                b.iter(|| {
                    scratch.touched.reset(scratch.bodies().len());
                    solver.solve_colored_sleeping(
                        black_box(&cfg),
                        black_box(&manifolds),
                        black_box(&graph),
                        &mut scratch,
                        &mut sleep,
                    );
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_sleeping);
criterion_main!(benches);
