//! O2 broadphase A/B: the uniform-grid CSR broadphase ([`BroadphaseGrid::build`])
//! vs the shipped O(n²) all-pairs loop, at {100, 1k, 10k} bodies.
//!
//! Reports the measured crossover (the plan promises only "expect O(100s)", not a
//! number). Each scene is a moderately dense cluster so the candidate set is
//! non-trivial; every benched scene asserts `pairs.len() > 0` (anti-vacuity) so a
//! degenerate empty broadphase never reads as a win.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use boyko_physics::components::ColliderShape;
use boyko_physics::manifold::BodyIndex;
use boyko_physics::math::Vec3;
use boyko_physics::resources::{BodyState, BroadphaseGrid};
use boyko_physics::systems::body_bounding_radius;
use boyko_threadpool::ThreadPoolBuilder;

/// Builds a `BodyState` carrying only the broadphase-relevant fields.
fn sphere(position: Vec3, radius: f32) -> BodyState {
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
fn disparity_scene(n: usize) -> Vec<BodyState> {
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
fn scene(n: usize) -> Vec<BodyState> {
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

/// The all-pairs reference loop (the shipped `AllPairs` arm), into a reused `Vec`.
fn all_pairs(bodies: &[BodyState], out: &mut Vec<(BodyIndex, BodyIndex)>) {
    out.clear();
    let n = bodies.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let bound = body_bounding_radius(&bodies[i]) + body_bounding_radius(&bodies[j]);
            let delta = bodies[j].position - bodies[i].position;
            if delta.length_squared() <= bound * bound {
                out.push((BodyIndex(i as u32), BodyIndex(j as u32)));
            }
        }
    }
}

fn bench_broadphase(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadphase");
    for &n in &[100usize, 1_000, 10_000] {
        let bodies = scene(n);

        // Anti-vacuity: confirm the scene yields real pairs before benching.
        let mut probe = BroadphaseGrid::with_capacity(n);
        let mut probe_out = Vec::new();
        probe.build(&bodies, &mut probe_out);
        assert!(
            !probe_out.is_empty(),
            "benched scene (n={n}) must produce pairs (anti-vacuity)"
        );

        group.bench_with_input(BenchmarkId::new("all_pairs", n), &bodies, |b, bodies| {
            let mut out = Vec::new();
            b.iter(|| {
                all_pairs(black_box(bodies), &mut out);
                black_box(out.len());
            });
        });

        group.bench_with_input(BenchmarkId::new("grid", n), &bodies, |b, bodies| {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = Vec::new();
            // Warm the grid so the timed iterations measure the steady-state
            // (capacity-reused, alloc-free) build, not first-build growth.
            grid.build(bodies, &mut out);
            b.iter(|| {
                grid.build(black_box(bodies), &mut out);
                black_box(out.len());
            });
        });
    }
    group.finish();
}

/// O2 W1 size-disparity criterion: one batch of typical small bodies + a few
/// giants. The decoupled-floor grid must beat all-pairs at scale (where the old
/// `max_radius`-floored grid would have tied/lost by coarsening to all-pairs).
fn bench_disparity(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadphase_disparity");
    for &n in &[1_000usize, 10_000] {
        let bodies = disparity_scene(n);

        // Anti-vacuity: the giants pair with many small bodies.
        let mut probe = BroadphaseGrid::with_capacity(bodies.len());
        let mut probe_out = Vec::new();
        probe.build(&bodies, &mut probe_out);
        assert!(
            !probe_out.is_empty(),
            "disparity scene (n={n}) must produce pairs (anti-vacuity)"
        );
        assert!(
            probe.oversized_len() >= 2,
            "disparity scene (n={n}) must route >= 2 bodies oversized (got {})",
            probe.oversized_len()
        );

        group.bench_with_input(BenchmarkId::new("all_pairs", n), &bodies, |b, bodies| {
            let mut out = Vec::new();
            b.iter(|| {
                all_pairs(black_box(bodies), &mut out);
                black_box(out.len());
            });
        });

        group.bench_with_input(BenchmarkId::new("grid", n), &bodies, |b, bodies| {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = Vec::new();
            grid.build(bodies, &mut out);
            b.iter(|| {
                grid.build(black_box(bodies), &mut out);
                black_box(out.len());
            });
        });
    }
    group.finish();
}

/// O3 Gate 7: PARALLEL candidate-emit scaling. `BroadphaseGrid::build_parallel`
/// dispatched through a real `boyko_threadpool` at workers ∈ {1, 2, 4} on a DENSE
/// scene (n_cells ≈ n → a genuine multi-cell candidate set), at n ∈ {1k, 10k,
/// 100k}.
///
/// The headline gate is the speedup @100k: 4 workers vs 1 worker should reach a
/// ratio of at least 2.8x (the plan's Amdahl estimate is f ~ 0.04-0.10, i.e. a
/// ~3.08x ceiling at the serial CSR + final-sort fraction). Criterion reports each
/// lane's median; the 4-vs-1 ratio at 100k is read from the medians.
///
/// **What the `w1` row measures after KE16 App-1 (2026-09-02).** App-1 set
/// `lanes = pool.num_threads()` and re-aimed the previously dead `lanes < 2` guard,
/// so a ONE-worker pool no longer dispatches at all: `w1` is the O2 serial `build`
/// plus one `try_with_active_pool` probe, and `w1 / serial_o2` is therefore ≈ 1.00x
/// BY CONSTRUCTION. It is a receipt that the guard routes as documented, not a probe.
///
/// The W=1-vs-O2-serial SHAPED regression probe this doc used to claim for that row
/// is RETIRED, not moved: after App-1 the one-lane shaped path is unreachable through
/// `build_parallel` at any worker count, so the regression it guarded (a one-lane
/// caller paying Pass A + prefix-sum + per-chunk sort over the serial `build`) can no
/// longer occur. `crates/boyko_physics/tests/broadphase_grid.rs::
/// one_worker_build_parallel_takes_the_serial_fallback` is the gate on that routing,
/// with an allocation receipt; the shaped path is measured from `w2` upward.
///
/// Consequently the 4-vs-1 ratio's DENOMINATOR is now the serial path rather than the
/// one-lane shaped path. The serial path is the faster of the two, so the same 2.8x
/// line is now at least as strict as when it was calibrated — the threshold is not
/// loosened by App-1, but the number is against a different reference and the results
/// file records which.
///
/// Below MIN_PARALLEL_BODIES (= 4096) `build_parallel` takes the no-pool serial
/// shaped path regardless of the pool — so n=1k is a single-lane reference; the
/// scaling claim is read at 10k and (the gate) 100k where the dispatched branch
/// is live. The bench is DENSE + the parallel path is gated — both stated so a
/// degenerate scene can never read as a win (every scene asserts pairs > 0).
fn bench_parallel(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadphase_parallel");
    // The parallel-emit dispatch + final sort dominate the per-iter cost at scale;
    // a modest sample count keeps the 100k × 3-worker matrix wall-clock reasonable
    // while criterion still reports a stable median.
    group.sample_size(20);

    for &n in &[1_000usize, 10_000, 100_000] {
        let bodies = scene(n);

        // Anti-vacuity: confirm the scene yields real pairs before benching, and
        // report whether it is at/above the parallel-dispatch threshold (4096) so
        // the reader knows which n's actually exercise the dispatched branch.
        let mut probe = BroadphaseGrid::with_capacity(n);
        let mut probe_out = Vec::new();
        probe.build(&bodies, &mut probe_out);
        assert!(
            !probe_out.is_empty(),
            "parallel bench scene (n={n}) must produce pairs (anti-vacuity)"
        );

        // The O2 serial `build` baseline at this n. After App-1 this is also what
        // the `w1` row runs (the guard routes a one-worker pool here), so the two
        // medians read alike by construction; the pair is the routing receipt.
        group.bench_with_input(BenchmarkId::new("serial_o2", n), &bodies, |b, bodies| {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = Vec::new();
            grid.build(bodies, &mut out);
            b.iter(|| {
                grid.build(black_box(bodies), &mut out);
                black_box(out.len());
            });
        });

        for &workers in &[1usize, 2, 4] {
            let id = BenchmarkId::new(format!("w{workers}"), n);
            group.bench_with_input(id, &bodies, |b, bodies| {
                let pool = ThreadPoolBuilder::new().num_threads(workers).build();
                // Warm + time INSIDE one install frame so `try_with_active_pool`
                // finds the ambient pool every iteration (the dispatched branch at
                // workers >= 2; at workers = 1 the App-1 guard routes to the O2
                // serial `build`, which is what that row reports); warm-up grows
                // every scratch Vec so the timed builds are the steady-state,
                // capacity-reused (bounded-alloc) path.
                pool.install(|_scope| {
                    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
                    let mut out = Vec::new();
                    for _ in 0..3 {
                        grid.build_parallel(bodies, &mut out);
                    }
                    b.iter(|| {
                        grid.build_parallel(black_box(bodies), &mut out);
                        black_box(out.len());
                    });
                });
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench_broadphase, bench_disparity, bench_parallel);
criterion_main!(benches);
