//! O4 criterion bench (Gate 9): the cost of the pure
//! [`ConstraintGraph::build`](boyko_physics::ConstraintGraph) partition
//! (union-find islands + greedy coloring) at 1k / 10k / 100k bodies with REALISTIC
//! contact density (a settled-stack-like graph: each body contacts a few
//! neighbours). The build is `O(contacts · colors_touched)` so the per-build cost
//! should scale ~linearly in the manifold count.
//!
//! The bench drives the SAME pure `build()` the production `physics_build_graph`
//! stage calls (it touches only `Vec` scratch), with a WARMED graph so the
//! steady-state, capacity-reused path is measured (matching the runtime hot path),
//! not first-build growth.
//!
//! Anti-vacuity: every scene is asserted to have `> 0` manifolds and `> 1` color
//! before timing, so the bench never reports the cost of a no-op.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

use boyko_physics::manifold::{BodyIndex, Manifold};
use boyko_physics::resources::ConstraintGraph;

/// Builds a realistic-density contact graph over `n_bodies` dynamic bodies: a grid
/// of small "stacks" where each body contacts its vertical neighbour and a couple
/// of lateral neighbours — a connected, multi-color graph (the chromatic load of a
/// resting pile). Returns the manifold list and the dynamic-body count.
///
/// Density: ~3 manifolds per body (each body is body_a of up to 3 edges to higher
/// rows). This yields one big island and a color count > 1 (neighbours share
/// bodies), exercising the greedy first-fit's color search.
fn realistic_graph(n_bodies: u32) -> Vec<Manifold> {
    let mut manifolds = Vec::with_capacity(n_bodies as usize * 3);
    // Treat rows as a 1D chain of "columns" of height H; connect each body to the
    // next in its column (vertical contact) and to the same level in the next
    // column (lateral contact) — a 2D-lattice contact graph (one island, several
    // colors).
    const H: u32 = 16; // column height
    for row in 0..n_bodies {
        let next = row + 1;
        // Vertical neighbour (within the same column).
        if next < n_bodies && next % H != 0 {
            manifolds.push(Manifold::new(BodyIndex(row), BodyIndex(next)));
        }
        // Lateral neighbour (the same height in the next column).
        let lateral = row + H;
        if lateral < n_bodies {
            manifolds.push(Manifold::new(BodyIndex(row), BodyIndex(lateral)));
        }
        // A diagonal brace (the next column, one level up) — adds chromatic load.
        let diag = row + H + 1;
        if diag < n_bodies && (row + 1) % H != 0 {
            manifolds.push(Manifold::new(BodyIndex(row), BodyIndex(diag)));
        }
    }
    manifolds
}

fn bench_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("ConstraintGraph::build");

    for &n_bodies in &[1_000u32, 10_000, 100_000] {
        let manifolds = realistic_graph(n_bodies);
        let n = n_bodies as usize;
        let is_dynamic = move |row: u32| row < n_bodies; // every row dynamic

        // Anti-vacuity + warm the graph: a real partition with > 1 color.
        let mut warm = ConstraintGraph::with_capacity(n);
        warm.build(&manifolds, n, is_dynamic);
        assert!(!manifolds.is_empty(), "scene must have > 0 manifolds");
        assert!(
            warm.n_colors() > 1,
            "dense scene must have > 1 color (got {})",
            warm.n_colors()
        );
        assert!(warm.n_islands() >= 1, "dense scene must have >= 1 island");

        group.throughput(Throughput::Elements(manifolds.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(n_bodies),
            &n_bodies,
            |b, &_n_bodies| {
                // Reuse the warmed graph: steady-state, capacity-reused build (the
                // production hot path), not first-build growth.
                b.iter(|| {
                    warm.build(black_box(&manifolds), n, is_dynamic);
                    black_box(warm.n_colors());
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_build);
criterion_main!(benches);
