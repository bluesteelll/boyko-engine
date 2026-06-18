//! Phase O6 Gate 10 — PARALLEL colored-solve scaling A/B (the production-ready
//! perf bar).
//!
//! Measures one full colored solver step (build + substep loop + warm store +
//! write-back) under the O6 parallel per-color dispatch (`parallel_solve = true`),
//! driven INSIDE a `ThreadPool::install` frame so `solve_colored` finds the ambient
//! pool, at worker counts {1, 2, 4, 8}, against the single-threaded colored solve
//! (`parallel_solve = false`, the O5 baseline). Both run the IDENTICAL warmed
//! resting contact set, so the bench isolates the dispatch scaling.
//!
//! # The plan's bar (`docs/OPTIMIZATION-PLAN-PHYSICS.md` Phase O6 "production-ready
//! when"): `criterion >= 0.6x linear to 4 workers on a 10k-body pyramid` (a big
//! single-island case). With the O5 single-thread step time `t1`, the 4-worker step
//! `t4` meets the bar when `t1 / t4 >= 0.6 * 4 = 2.4` (>= 60% of ideal 4x). The
//! table below reports `t1` and the {1,2,4,8}-worker times so the speedups are
//! read off directly; the bar is a >= 2.4x speedup at 4 workers.
//!
//! # Threshold sweep (the W1 `MIN_PARALLEL_SLOTS_PER_COLOR` knob)
//!
//! `MIN_PARALLEL_SLOTS_PER_COLOR` is a COMPILE-TIME const, so it cannot be swept
//! from a runtime bench. Its EFFECT — "small-color worlds don't pay a scope-alloc /
//! dispatch tax; large-color worlds scale" — is measured here by varying the SCENE
//! at the fixed threshold:
//!   * `small_color` (every color < 256 → all colors solved INLINE under
//!     `parallel_solve = true`): parallel must be >= the single-threaded colored
//!     solve (NO regression — no `pool.scope` is issued, so the only delta is the
//!     `try_with_active_pool` lookup). This is the "no scope tax" bar.
//!   * `pyramid_10k` (the shared base/island colors >> 256 → real dispatch): the
//!     >= 2.4x-at-4-workers scaling bar.
//!
//! Anti-vacuity: every scene asserts `> 0` contacts AND `> 1` color, and (for the
//! pyramid) that its widest color exceeds the threshold so a real `pool.scope`
//! dispatch occurs across `> 1` worker.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

use boyko_physics::components::{BodyType, Collider, ColliderShape, RigidBody, RigidBodyMass};
use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};
use boyko_physics::solver::ColoredSoftStepSolver;
use boyko_threadpool::ThreadPoolBuilder;

/// The colored solver's W1 inline-vs-dispatch threshold (a private const in the
/// crate; mirrored here only for the bench's anti-vacuity witness). Keep in sync
/// with `colored.rs::MIN_PARALLEL_SLOTS_PER_COLOR`.
const MIN_PARALLEL_SLOTS_PER_COLOR: u32 = 256;

fn dyn_sphere(position: Vec3) -> BodyState {
    let body = RigidBody { position, linear_velocity: Vec3::ZERO, rotation: Quat::IDENTITY, angular_velocity: Vec3::ZERO };
    let mass = RigidBodyMass { inv_inertia: Mat3::IDENTITY, inv_mass: 1.0, restitution: 0.0, friction: 0.5, body_type: BodyType::Dynamic };
    let collider = Collider { shape: ColliderShape::Sphere { radius: 0.5 }, layer: 1, mask: 1 };
    BodyState::from_columns(&body, &mass, &collider)
}

fn static_floor() -> BodyState {
    let body = RigidBody { position: Vec3::new(0.0, -50.0, 0.0), linear_velocity: Vec3::ZERO, rotation: Quat::IDENTITY, angular_velocity: Vec3::ZERO };
    let mass = RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass: 0.0, restitution: 0.0, friction: 0.5, body_type: BodyType::Static };
    let collider = Collider { shape: ColliderShape::Sphere { radius: 50.0 }, layer: 1, mask: 1 };
    BodyState::from_columns(&body, &mass, &collider)
}

fn manifold(a: u32, b: u32, normal: Vec3, anchor: Vec3) -> Manifold {
    let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
    m.normal = normal;
    m.points[0] = ContactPoint { anchor_a: anchor, anchor_b: anchor, separation: -0.05, feature_id: 0 };
    m.count = 1;
    m
}

/// A big single-ISLAND pyramid: `base` rows wide at the bottom, narrowing by one
/// per level to the apex. Every sphere rests on the two below it (a connected
/// triangular lattice → ONE island) plus the bottom row on the shared floor. The
/// contact graph is dense and multi-color (the shared-floor color + the inter-row
/// colors each hold many slots, well above the threshold), so the parallel dispatch
/// fans across workers. Returns `(bodies, manifolds)`. `base = 141` yields
/// 141·142/2 = 10011 dynamic bodies — the plan's ~10k pyramid.
fn pyramid_scene(base: u32) -> (Vec<BodyState>, Vec<Manifold>) {
    // Row r (0 = bottom) has `base - r` spheres; precompute each row's first row-id.
    let mut row_first = Vec::with_capacity(base as usize + 1);
    let mut acc = 0u32;
    for r in 0..base {
        row_first.push(acc);
        acc += base - r;
    }
    row_first.push(acc);
    let n_dyn = acc;

    let mut bodies: Vec<BodyState> = Vec::with_capacity(n_dyn as usize + 1);
    for r in 0..base {
        let count = base - r;
        let y = 0.5 + r as f32 * 0.86; // ~vertical pitch of stacked unit spheres
        for i in 0..count {
            // Center each row; offset alternate rows by half a sphere (brick stack).
            let x = (i as f32 - (count as f32 - 1.0) * 0.5) * 1.02 + (r as f32 * 0.5);
            bodies.push(dyn_sphere(Vec3::new(x, y, 0.0)));
        }
    }
    let floor_row = n_dyn;
    bodies.push(static_floor());

    let id = |r: u32, i: u32| row_first[r as usize] + i;
    let mut manifolds = Vec::new();
    for r in 0..base {
        let count = base - r;
        for i in 0..count {
            let me = id(r, i);
            if r == 0 {
                manifolds.push(manifold(me, floor_row, Vec3::new(0.0, -1.0, 0.0), bodies[me as usize].position));
            }
            // Lateral neighbour in the same row.
            if i + 1 < count {
                manifolds.push(manifold(me, id(r, i + 1), Vec3::new(1.0, 0.0, 0.0), bodies[me as usize].position));
            }
            // Rest on the two supports in the row below (brick pattern).
            if r + 1 < base {
                let above_count = base - (r + 1);
                // sphere `i` of row r+1 sits on `i` and `i+1` of row r.
                if i < above_count {
                    let up = id(r + 1, i);
                    manifolds.push(manifold(me, up, Vec3::new(0.0, 1.0, 0.0), bodies[me as usize].position));
                }
                if i >= 1 && (i - 1) < above_count {
                    let up = id(r + 1, i - 1);
                    manifolds.push(manifold(me, up, Vec3::new(0.0, 1.0, 0.0), bodies[me as usize].position));
                }
            }
        }
    }
    (bodies, manifolds)
}

/// A SMALL-color world: a single short stack chain (every color far below the
/// threshold), so under `parallel_solve = true` every color is solved INLINE.
fn small_color_scene(n: u32) -> (Vec<BodyState>, Vec<Manifold>) {
    let mut bodies: Vec<BodyState> = (0..n)
        .map(|i| dyn_sphere(Vec3::new(0.0, 0.5 + i as f32 * 0.99, 0.0)))
        .collect();
    let floor_row = n;
    bodies.push(static_floor());
    let mut manifolds = Vec::new();
    manifolds.push(manifold(0, floor_row, Vec3::new(0.0, -1.0, 0.0), bodies[0].position));
    for i in 1..n {
        manifolds.push(manifold(i - 1, i, Vec3::new(0.0, 1.0, 0.0), bodies[i as usize].position));
    }
    (bodies, manifolds)
}

fn config(parallel: bool) -> PhysicsConfig {
    PhysicsConfig { dt: 1.0 / 60.0, parallel_solve: parallel, ..PhysicsConfig::default() }
}

fn build_graph(bodies: &[BodyState], manifolds: &[Manifold]) -> ConstraintGraph {
    let mut g = ConstraintGraph::with_capacity(bodies.len());
    let inv_mass: Vec<f32> = bodies.iter().map(|b| b.inv_mass).collect();
    g.build(manifolds, bodies.len(), move |row| {
        (row as usize) < inv_mass.len() && inv_mass[row as usize] != 0.0
    });
    g
}

/// Widest color span (in manifolds) for the partition — the anti-vacuity witness
/// that a real `pool.scope` dispatch occurs. The bench scenes use SINGLE-point
/// manifolds, so a color's manifold count equals its contact-POINT (slot) count,
/// the unit the W1 threshold is measured in.
fn widest_color(graph: &ConstraintGraph) -> u32 {
    (0..graph.n_colors())
        .map(|c| graph.color(c).len() as u32)
        .max()
        .unwrap_or(0)
}

/// Times one warmed colored solve step at `workers` worker count (worker count 0 =
/// the single-threaded `parallel_solve = false` baseline, run with NO pool).
fn bench_one(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    label: &str,
    n_contacts: usize,
    bodies: &[BodyState],
    manifolds: &[Manifold],
    workers: usize,
) {
    let id = if workers == 0 {
        BenchmarkId::new(format!("{label}/single_O5"), n_contacts)
    } else {
        BenchmarkId::new(format!("{label}/parallel_{workers}w"), n_contacts)
    };
    group.bench_with_input(id, &n_contacts, |b, &_n| {
        let parallel = workers != 0;
        let cfg = config(parallel);
        let graph = build_graph(bodies, manifolds);
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.bodies = bodies.to_vec();
        scratch.touched.reset(scratch.bodies.len());

        if parallel {
            let pool = ThreadPoolBuilder::new().num_threads(workers).build();
            pool.install(|_scope| {
                for _ in 0..4 {
                    scratch.touched.reset(scratch.bodies.len());
                    solver.solve_colored(&cfg, manifolds, &graph, &mut scratch);
                }
                b.iter(|| {
                    scratch.touched.reset(scratch.bodies.len());
                    solver.solve_colored(black_box(&cfg), black_box(manifolds), black_box(&graph), &mut scratch);
                });
            });
        } else {
            for _ in 0..4 {
                scratch.touched.reset(scratch.bodies.len());
                solver.solve_colored(&cfg, manifolds, &graph, &mut scratch);
            }
            b.iter(|| {
                scratch.touched.reset(scratch.bodies.len());
                solver.solve_colored(black_box(&cfg), black_box(manifolds), black_box(&graph), &mut scratch);
            });
        }
    });
}

fn bench_pyramid_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_solve_pyramid_10k");
    group.sample_size(20); // a 10k-body step is heavy; keep the wall-clock sane

    let (bodies, manifolds) = pyramid_scene(141); // 10011 dynamic bodies
    let n_contacts = manifolds.len();
    let graph = build_graph(&bodies, &manifolds);

    // Anti-vacuity: real contacts, a real multi-color partition, AND a color above
    // the threshold (so `pool.scope` actually dispatches across > 1 worker).
    assert!(n_contacts > 0, "pyramid must have > 0 contacts");
    assert!(graph.n_colors() > 1, "pyramid must have > 1 color (got {})", graph.n_colors());
    let widest = widest_color(&graph);
    assert!(
        widest >= MIN_PARALLEL_SLOTS_PER_COLOR,
        "pyramid's widest color ({widest}) must exceed the threshold ({MIN_PARALLEL_SLOTS_PER_COLOR}) \
         so the parallel dispatch path is exercised"
    );

    group.throughput(Throughput::Elements(n_contacts as u64));
    // The O5 single-threaded baseline (worker 0), then the parallel sweep.
    for workers in [0usize, 1, 2, 4, 8] {
        bench_one(&mut group, "pyramid", n_contacts, &bodies, &manifolds, workers);
    }
    group.finish();
}

fn bench_threshold_effect(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_solve_threshold");
    group.sample_size(30);

    // SMALL-color world: every color < the threshold → all inline under
    // parallel_solve = true. The "no scope tax" bar: parallel >= single-threaded.
    let (bodies, manifolds) = small_color_scene(64);
    let n_contacts = manifolds.len();
    let graph = build_graph(&bodies, &manifolds);
    assert!(n_contacts > 0, "small-color scene must have > 0 contacts");
    assert!(graph.n_colors() > 1, "small-color scene must have > 1 color");
    let widest = widest_color(&graph);
    assert!(
        widest < MIN_PARALLEL_SLOTS_PER_COLOR,
        "small-color scene's widest color ({widest}) must stay BELOW the threshold \
         ({MIN_PARALLEL_SLOTS_PER_COLOR}) so every color is solved inline (the no-scope-tax case)"
    );
    group.throughput(Throughput::Elements(n_contacts as u64));
    for workers in [0usize, 1, 4] {
        bench_one(&mut group, "small_color", n_contacts, &bodies, &manifolds, workers);
    }

    // A LARGE-color world (a wide flat raft over the floor) where the shared-floor
    // color is far above the threshold → real dispatch; pair it with the small world
    // so the threshold's effect (tax vs no-tax) is visible side by side.
    let (bodies_l, manifolds_l) = large_color_raft(800);
    let n_l = manifolds_l.len();
    let graph_l = build_graph(&bodies_l, &manifolds_l);
    let widest_l = widest_color(&graph_l);
    assert!(
        widest_l >= MIN_PARALLEL_SLOTS_PER_COLOR,
        "large-color raft's widest color ({widest_l}) must exceed the threshold"
    );
    group.throughput(Throughput::Elements(n_l as u64));
    for workers in [0usize, 1, 4] {
        bench_one(&mut group, "large_color", n_l, &bodies_l, &manifolds_l, workers);
    }
    group.finish();
}

/// A wide flat raft: `n` dynamic spheres in a single line all resting on ONE shared
/// floor (the shared-floor color holds ~n slots → above the threshold). The
/// adjacent-pair colors are present too. A LARGE-color case for the threshold A/B.
fn large_color_raft(n: u32) -> (Vec<BodyState>, Vec<Manifold>) {
    let mut bodies: Vec<BodyState> = (0..n)
        .map(|i| dyn_sphere(Vec3::new(i as f32 * 0.98, 0.5, 0.0)))
        .collect();
    let floor_row = n;
    bodies.push(static_floor());
    let mut manifolds = Vec::new();
    for i in 0..n {
        manifolds.push(manifold(i, floor_row, Vec3::new(0.0, -1.0, 0.0), bodies[i as usize].position));
        if i + 1 < n {
            manifolds.push(manifold(i, i + 1, Vec3::new(1.0, 0.0, 0.0), bodies[i as usize].position));
        }
    }
    (bodies, manifolds)
}

criterion_group!(benches, bench_pyramid_scaling, bench_threshold_effect);
criterion_main!(benches);
