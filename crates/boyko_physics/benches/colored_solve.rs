//! Phase O5 Gate 9 — colored-vs-default solve A/B at ~1k / 10k contacts.
//!
//! Measures one full solver step (build + substep loop + warm store + write-back)
//! for the colored [`ColoredSoftStepSolver`] (color-order sweep over the solver's
//! contact columns — the per-point `ContactColumns` SoA in O5, the AoSoA
//! `CohortColumns` since L11 C2 — single-threaded in O5) against the reference
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
//!
//! # The O5 rows are SCALAR on purpose (2026-09-18)
//!
//! `simd_solve` defaults to `true` since 2026-09-18. The `solve_step` rows measured
//! the O5 premise — the scalar colored sweep against the reference — and their
//! figures (1.059× at ~1k contacts, 1.131× at ~10k, `colored_solve_plus_graph`) are
//! that comparison, so [`config`] pins `simd_solve: false` to keep their meaning. The
//! `default_world` row is the colored solve + graph build at `PhysicsConfig::default()`
//! (the O7 cohort kernel on); it is reported, not gated. The SIMD A/B is
//! `o7_simd_ab`.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, ConstraintGraph, FrictionModel, PhysicsConfig, SolverScratch};
use boyko_physics::solver::{ColoredSoftStepSolver, RigidSolver, SoftStepSolver};
use boyko_threadpool::ThreadPoolBuilder;

// boyko_physics does not re-export RigidBody/Collider through a bench-friendly POD
// builder, so build BodyState via from_columns with the public component structs.
use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass};

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
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius: 0.5 },
        layer: 1,
        mask: 1,
    };
    BodyState::from_columns(&body, &mass, &collider, false, true, false)
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
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius: 50.0 },
        layer: 1,
        mask: 1,
    };
    BodyState::from_columns(&body, &mass, &collider, false, false, false)
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

/// The O5 rows' config: the SCALAR colored solve, whatever the default (see the
/// module docs).
fn config() -> PhysicsConfig {
    PhysicsConfig {
        dt: 1.0 / 60.0,
        simd_solve: false,
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
                scratch.set_bodies(&bodies);
                scratch.touched.reset(scratch.bodies().len());
                // Warm so the warm-start cache is at steady state.
                for _ in 0..4 {
                    scratch.touched.reset(scratch.bodies().len());
                    solver.solve(&cfg, &manifolds, &mut scratch);
                }
                b.iter(|| {
                    scratch.touched.reset(scratch.bodies().len());
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
                scratch.set_bodies(&bodies);
                scratch.touched.reset(scratch.bodies().len());
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
                scratch.set_bodies(&bodies);
                for _ in 0..4 {
                    graph.build(&manifolds, scratch.bodies().len(), &is_dynamic);
                    scratch.touched.reset(scratch.bodies().len());
                    solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
                }
                b.iter(|| {
                    graph.build(black_box(&manifolds), scratch.bodies().len(), &is_dynamic);
                    scratch.touched.reset(scratch.bodies().len());
                    solver.solve_colored(black_box(&cfg), black_box(&manifolds), &graph, &mut scratch);
                });
            },
        );

        // ── The default world: `colored_solve_plus_graph` at
        //    `PhysicsConfig::default()` (the O7 cohort kernel on). Reported, not gated.
        group.bench_with_input(
            BenchmarkId::new("default_world", n_contacts),
            &n_contacts,
            |b, &_n| {
                let cfg = PhysicsConfig { dt: 1.0 / 60.0, ..PhysicsConfig::default() };
                let inv_mass: Vec<f32> = bodies.iter().map(|bb| bb.inv_mass).collect();
                let is_dynamic = move |row: u32| (row as usize) < inv_mass.len() && inv_mass[row as usize] != 0.0;
                let mut graph = ConstraintGraph::with_capacity(bodies.len());
                let mut solver = ColoredSoftStepSolver::default();
                let mut scratch = SolverScratch::with_capacity(bodies.len());
                scratch.set_bodies(&bodies);
                for _ in 0..4 {
                    graph.build(&manifolds, scratch.bodies().len(), &is_dynamic);
                    scratch.touched.reset(scratch.bodies().len());
                    solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
                }
                b.iter(|| {
                    graph.build(black_box(&manifolds), scratch.bodies().len(), &is_dynamic);
                    scratch.touched.reset(scratch.bodies().len());
                    solver.solve_colored(black_box(&cfg), black_box(&manifolds), &graph, &mut scratch);
                });
            },
        );
    }
    group.finish();
}

/// Phase O7 Gate 9 — the SIMD A/B: `{scalar-colored}` (O6, `simd_solve = false`)
/// vs `{simd_solve}` (O7) on the SAME warmed contact set, BOTH single-threaded and
/// parallel.
///
/// The bar (`PHYSICS-O7-DESIGN.md` Bench B / the plan's O7 production-ready gate) is
/// `{simd_solve} / {scalar-colored} >= 1.8x` on the solve, AND
/// `{parallel + simd} >= {parallel scalar}` (no cohort-snapping width-starvation).
/// On a non-AVX2 build `simd_solve` routes to the scalar oracle ⇒ the two lines
/// coincide.
///
/// ⚠ The remedy for that is NOT `RUSTFLAGS="-C target-feature=+avx2"`, which this
/// comment used to prescribe. `RUSTFLAGS` REPLACES `target.<triple>.rustflags`, so it
/// deletes both the repository's `-C target-cpu=x86-64-v3` baseline and this box's
/// linker flags — and the baseline already carries AVX2, so an ordinary
/// `cargo bench` has the SIMD arm live. The wrong recipe fails by making both arms
/// scalar, which reads as "the switch does nothing" rather than as an error.
fn bench_simd_ab(c: &mut Criterion) {
    let mut group = c.benchmark_group("o7_simd_ab");
    group.sample_size(30);

    // ~10k contacts: a dense pile so a color carries many cohorts (≫ 8 groups).
    let (bodies, manifolds) = pile_scene(128, 40);
    let n_contacts = manifolds.len();
    let graph = build_graph(&bodies, &manifolds);
    assert!(n_contacts > 0, "scene must have > 0 contacts");
    assert!(graph.n_colors() > 1, "dense scene must have > 1 color (got {})", graph.n_colors());
    // Anti-vacuity: a color must be wide enough that the cohort kernel fires (> 8
    // groups in at least one color, i.e. a full cohort + a partial).
    let widest = (0..graph.n_colors()).map(|c| graph.color(c).len()).max().unwrap_or(0);
    assert!(widest > 8, "widest color must exceed one cohort (8 groups), got {widest}");

    group.throughput(Throughput::Elements(n_contacts as u64));

    // `(label, simd_solve, parallel, workers, substeps, relax)`. The default
    // (substeps=4, relax=2 ⇒ 12 solve sweeps/step) is the production whole-step A/B;
    // the `solve_dominant` pair (substeps=8, relax=4 ⇒ 40 sweeps/step) amplifies the
    // WIDENED-solve fraction so the speedup converges toward the kernel's true ratio
    // (the design's Bench A), isolating it from the un-widened scalar surround
    // (build_columns / warm_apply / integrate / restitution / store) — the Amdahl
    // ceiling of the whole-step line.
    // `(label, simd_solve, simd_o1, parallel, workers, substeps, relax)`. `simd_o1`
    // widens O1's integrate/inertia surround too, so the `prod` pair isolates the
    // SOLVE speedup from O1's un-widened scalar surround in the whole-step number.
    let variants: &[(&str, bool, bool, bool, usize, u32, u32)] = &[
        ("scalar_colored_single", false, false, false, 0, 4, 2),
        ("simd_solve_single", true, false, false, 0, 4, 2),
        ("scalar_colored_parallel_4w", false, false, true, 4, 4, 2),
        ("simd_solve_parallel_4w", true, false, true, 4, 4, 2),
        ("scalar_colored_single_solvedom", false, false, false, 0, 8, 4),
        ("simd_solve_single_solvedom", true, false, false, 0, 8, 4),
        ("scalar_colored_single_prod", false, true, false, 0, 4, 2),
        ("simd_solve_single_prod", true, true, false, 0, 4, 2),
    ];

    for &(label, simd_solve, simd_o1, parallel, workers, substeps, relax_iterations) in variants {
        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            simd: simd_o1,
            simd_solve,
            parallel_solve: parallel,
            substeps,
            relax_iterations,
            ..PhysicsConfig::default()
        };
        group.bench_with_input(BenchmarkId::new(label, n_contacts), &n_contacts, |b, &_n| {
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(bodies.len());
            scratch.set_bodies(&bodies);
            scratch.touched.reset(scratch.bodies().len());

            let warm_and_time =
                |b: &mut criterion::Bencher<'_>, solver: &mut ColoredSoftStepSolver, scratch: &mut SolverScratch| {
                    for _ in 0..4 {
                        scratch.touched.reset(scratch.bodies().len());
                        solver.solve_colored(&cfg, &manifolds, &graph, scratch);
                    }
                    b.iter(|| {
                        scratch.touched.reset(scratch.bodies().len());
                        solver.solve_colored(black_box(&cfg), black_box(&manifolds), black_box(&graph), scratch);
                    });
                };

            if parallel {
                let pool = ThreadPoolBuilder::new().num_threads(workers).build();
                pool.install(|_scope| warm_and_time(b, &mut solver, &mut scratch));
            } else {
                warm_and_time(b, &mut solver, &mut scratch);
            }
        });
    }
    group.finish();
}

/// FM C1 (`levers/FM-manifold-friction/`, design Q3, `fm_depth_ab`): `pile_scene(128, 40)`'s graph
/// with every manifold widened to `w ∈ {1, 2, 3, 4}` points — point `k` at the anchor plus an
/// in-plane offset of ±0.2 m, feature id `k`, separation −0.05 — under `friction_model` {PerPoint,
/// PerManifold}, on the single-threaded and the 4-worker `simd_solve` rows (`o7_simd_ab`'s). A
/// depth-1 layout runs the per-point code under either model (D16), so its two rows are one path.
///
/// Anti-vacuity before timing: at least 95 % of the layout's cohorts are of depth `w`
/// (`ColoredSoftStepSolver::layout_census`), and the per-manifold rows of `w ≥ 2` lay out patches.
/// `-- --test fm_depth_ab` runs each row once, untimed (criterion's test mode).
fn bench_fm_depth_ab(c: &mut Criterion) {
    let mut group = c.benchmark_group("fm_depth_ab");
    group.sample_size(30);
    let (bodies, base) = pile_scene(128, 40);
    for w in 1..=4u8 {
        let manifolds = widened(&base, w);
        let graph = build_graph(&bodies, &manifolds);
        let n_points = usize::from(w) * manifolds.len();
        group.throughput(Throughput::Elements(n_points as u64));
        for (model_label, model) in [("point", FrictionModel::PerPoint), ("manifold", FrictionModel::PerManifold)] {
            for (row, parallel, workers) in [("simd_solve_single", false, 0usize), ("simd_solve_parallel_4w", true, 4)] {
                let cfg = PhysicsConfig {
                    dt: 1.0 / 60.0,
                    simd_solve: true,
                    parallel_solve: parallel,
                    friction_model: model,
                    ..PhysicsConfig::default()
                };
                let id = BenchmarkId::new(format!("{row}/{model_label}/w{w}"), n_points);
                group.bench_with_input(id, &n_points, |b, &_n| {
                    let mut solver = ColoredSoftStepSolver::default();
                    let mut scratch = SolverScratch::with_capacity(bodies.len());
                    scratch.set_bodies(&bodies);
                    scratch.touched.reset(scratch.bodies().len());
                    let warm_and_time =
                        |b: &mut criterion::Bencher<'_>, solver: &mut ColoredSoftStepSolver, scratch: &mut SolverScratch| {
                            for _ in 0..4 {
                                scratch.touched.reset(scratch.bodies().len());
                                solver.solve_colored(&cfg, &manifolds, &graph, scratch);
                            }
                            let census = solver.layout_census();
                            assert!(
                                census.cohorts > 0
                                    && census.cohorts_by_depth[usize::from(w) - 1] * 100 >= census.cohorts * 95,
                                "fm_depth_ab anti-vacuity: at least 95 % of the cohorts must be of depth {w}: {census:?}"
                            );
                            assert!(
                                model == FrictionModel::PerPoint || w == 1 || census.patch_blocks > 0,
                                "fm_depth_ab anti-vacuity: the per-manifold rows of w >= 2 lay out patches: {census:?}"
                            );
                            b.iter(|| {
                                scratch.touched.reset(scratch.bodies().len());
                                solver.solve_colored(black_box(&cfg), black_box(&manifolds), black_box(&graph), scratch);
                            });
                        };
                    if parallel {
                        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
                        pool.install(|_scope| warm_and_time(b, &mut solver, &mut scratch));
                    } else {
                        warm_and_time(b, &mut solver, &mut scratch);
                    }
                });
            }
        }
    }
    group.finish();
}

/// `manifolds` with every manifold widened to `w` points (`fm_depth_ab`): point `k` at the first
/// point's anchor plus an in-plane offset of ±0.2 m along two axes perpendicular to the normal
/// (`pile_scene`'s normals are ±y and +x), feature id `k`, separation −0.05.
fn widened(manifolds: &[Manifold], w: u8) -> Vec<Manifold> {
    let offsets = [(0.2f32, 0.2f32), (-0.2, 0.2), (-0.2, -0.2), (0.2, -0.2)];
    manifolds
        .iter()
        .map(|m| {
            let n = m.normal;
            let (u, v) = if n.y.abs() > 0.5 {
                (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
            } else {
                (Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
            };
            let anchor = m.points[0].anchor_a;
            let mut out = *m;
            for (k, &(a, b)) in offsets.iter().enumerate().take(usize::from(w)) {
                let p = anchor + u * a + v * b;
                out.points[k] = ContactPoint { anchor_a: p, anchor_b: p, separation: -0.05, feature_id: k as u32 };
            }
            out.count = w;
            out
        })
        .collect()
}

criterion_group!(benches, bench_solve, bench_simd_ab, bench_fm_depth_ab);
criterion_main!(benches);
