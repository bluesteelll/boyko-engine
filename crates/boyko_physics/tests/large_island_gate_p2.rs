//! P2 large-island threshold gate (docs/ARCHITECTURE-HYBRID-PERF.md Part 3.3 / P2).
//!
//! The colored-parallel constraint solve is keyed on island size, not on a build-time
//! flag alone: even with `PhysicsConfig::parallel_solve` opted in, a step whose
//! LARGEST island holds fewer than
//! [`LARGE_ISLAND_CONSTRAINTS`](boyko_physics::resources::LARGE_ISLAND_CONSTRAINTS)
//! manifolds runs the byte-identical SINGLE-THREADED colored solve (the per-color
//! `pool.scope` dispatch cannot amortize on so small a parallel unit), and a step
//! at/above the threshold takes the colored-parallel path. The selection is a single
//! scalar compare on a count [`ConstraintGraph::build`] already folds in (zero extra
//! pass).
//!
//! This file is the P2 gate's tester suite. It proves three properties:
//!
//! 1. **`max_island_constraints` is exact** (Gate A, Miri-safe pure `build`): the
//!    largest-island manifold count matches a hand-computed reference on single-big-
//!    island, many-small-island, empty, and threshold-boundary scenes.
//! 2. **The gate SELECTS the path by island size** (Gate B, `cfg(not(miri))`): with
//!    `parallel_solve == true`, a SMALL-island scene issues ZERO `pool.scope`
//!    allocations (forced single-threaded by the gate) while a LARGE-island scene
//!    issues a non-zero, bounded count (let through to the parallel dispatch) — the
//!    island size, and ONLY the island size, flips the observable.
//! 3. **Both gated paths are BIT-IDENTICAL** (Gate C, `cfg(not(miri))`, the
//!    load-bearing equivalence): a small-island scene's gated single-threaded result
//!    bit-equals the `parallel_solve == false` reference, and a large-island scene's
//!    gated-parallel result bit-equals its single-threaded reference. The gate
//!    changes only WHERE the colored solve runs, never the bits (the {1, N}-worker
//!    property). The colored solve math is untouched — only the selection of which
//!    solve path runs.
//!
//! Gate A is the Miri-compatible pure-`build` path (only `Vec` scratch); Gates B/C
//! spin the threadpool (Miri-intractable int-to-ptr, Phase 9.1-9.3) and so are
//! `cfg(not(miri))`.

use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, ConstraintGraph, LARGE_ISLAND_CONSTRAINTS};
use boyko_physics::components::ColliderShape;

// ── Scene helpers (mirror the lib + colored_parallel_alloc_o6 fixtures) ───────────

fn dyn_sphere(position: Vec3) -> BodyState {
    BodyState {
        inv_inertia: Mat3::from_diagonal(Vec3::new(1.5, 1.2, 1.3)),
        inv_inertia_local: Mat3::from_diagonal(Vec3::new(1.5, 1.2, 1.3)),
        position,
        linear_velocity: Vec3::ZERO,
        angular_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.0,
        friction: 0.5,
        simulated: true,
        kinematic: false,
        is_sensor: false,
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
        shape: ColliderShape::Sphere { radius: 1.0 },
    }
}

fn edge(a: u32, b: u32) -> Manifold {
    Manifold::new(BodyIndex(a), BodyIndex(b))
}

/// The dynamic predicate the production stage uses: a row is dynamic iff in range
/// with non-zero inverse mass.
fn is_dynamic_of(bodies: &[BodyState]) -> impl Fn(u32) -> bool + '_ {
    move |row: u32| (row as usize) < bodies.len() && bodies[row as usize].inv_mass != 0.0
}

// ── Gate A: max_island_constraints is exact (Miri-safe pure build) ────────────────

/// One connected chain of `n` dynamic bodies (`0-1-2-…-(n-1)`) is ONE island with
/// `n - 1` manifolds — so `max_island_constraints` must be exactly `n - 1`.
#[test]
fn max_island_constraints_single_chain_island() {
    let n = 300u32;
    let bodies: Vec<BodyState> =
        (0..n).map(|i| dyn_sphere(Vec3::new(i as f32, 0.0, 0.0))).collect();
    let manifolds: Vec<Manifold> = (0..n - 1).map(|i| edge(i, i + 1)).collect();

    let mut g = ConstraintGraph::with_capacity(bodies.len());
    g.build(&manifolds, bodies.len(), is_dynamic_of(&bodies));

    assert_eq!(g.n_islands(), 1, "a connected chain is one island");
    assert_eq!(
        g.max_island_constraints(),
        n - 1,
        "the single island holds all {} manifolds",
        n - 1
    );
}

/// Many DISJOINT dynamic pairs `(0-1) (2-3) …` are many islands of ONE manifold each
/// — so the LARGEST island holds exactly 1 manifold, regardless of how many islands.
#[test]
fn max_island_constraints_many_tiny_islands() {
    let pairs = 500u32;
    let n = pairs * 2;
    let bodies: Vec<BodyState> =
        (0..n).map(|i| dyn_sphere(Vec3::new(i as f32, 0.0, 0.0))).collect();
    let manifolds: Vec<Manifold> = (0..pairs).map(|p| edge(p * 2, p * 2 + 1)).collect();

    let mut g = ConstraintGraph::with_capacity(bodies.len());
    g.build(&manifolds, bodies.len(), is_dynamic_of(&bodies));

    assert_eq!(g.n_islands(), pairs, "each disjoint pair is its own island");
    assert_eq!(
        g.max_island_constraints(),
        1,
        "every island holds exactly one manifold, so the max is 1 regardless of island count"
    );
}

/// A scene mixing one big island (a star over a shared dynamic hub) with several tiny
/// islands: the max must track the BIG island's manifold count, not the island count.
#[test]
fn max_island_constraints_tracks_the_largest_not_the_count() {
    // Big island: a star — hub row 0 contacts rows 1..=big_edges (big_edges manifolds,
    // all in one island because the hub is dynamic and connects them).
    let big_edges = 400u32;
    let hub_rows = big_edges + 1; // rows 0..=big_edges
    // Small islands: disjoint dynamic pairs appended after the star's rows.
    let small_pairs = 50u32;
    let total = hub_rows + small_pairs * 2;

    let bodies: Vec<BodyState> =
        (0..total).map(|i| dyn_sphere(Vec3::new(i as f32, 0.0, 0.0))).collect();

    let mut manifolds: Vec<Manifold> = (1..=big_edges).map(|r| edge(0, r)).collect();
    for p in 0..small_pairs {
        let a = hub_rows + p * 2;
        manifolds.push(edge(a, a + 1));
    }

    let mut g = ConstraintGraph::with_capacity(bodies.len());
    g.build(&manifolds, bodies.len(), is_dynamic_of(&bodies));

    assert_eq!(
        g.n_islands(),
        1 + small_pairs,
        "one star island + {small_pairs} pair islands"
    );
    assert_eq!(
        g.max_island_constraints(),
        big_edges,
        "the max tracks the star island's {big_edges} manifolds, not the {} total islands",
        1 + small_pairs
    );
}

/// Empty / island-less partitions report `0` (the loop body never runs).
#[test]
fn max_island_constraints_empty_is_zero() {
    let mut g = ConstraintGraph::with_capacity(0);
    let no_bodies: [BodyState; 0] = [];
    g.build(&[], 0, is_dynamic_of(&no_bodies));
    assert_eq!(g.n_islands(), 0);
    assert_eq!(g.max_island_constraints(), 0, "no islands → max is 0");

    // A static-static degenerate contact files under NO island → still 0.
    let statics = [static_body(Vec3::ZERO), static_body(Vec3::new(0.5, 0.0, 0.0))];
    let mut g2 = ConstraintGraph::with_capacity(2);
    g2.build(&[edge(0, 1)], 2, is_dynamic_of(&statics));
    assert_eq!(g2.n_islands(), 0, "two static bodies → no island");
    assert_eq!(g2.max_island_constraints(), 0, "the static-static edge is in no island");
}

/// Straddles the gate boundary: a chain of exactly `LARGE_ISLAND_CONSTRAINTS + 1`
/// bodies has `LARGE_ISLAND_CONSTRAINTS` manifolds (== threshold → gate ON), and one
/// fewer body is one BELOW the threshold (gate OFF). Proves the count is exact at the
/// decision point.
#[test]
fn max_island_constraints_at_the_threshold_boundary() {
    // == threshold: a chain of (T + 1) bodies has T edges.
    let t = LARGE_ISLAND_CONSTRAINTS;
    let at_bodies: Vec<BodyState> =
        (0..=t).map(|i| dyn_sphere(Vec3::new(i as f32, 0.0, 0.0))).collect();
    let at_manifolds: Vec<Manifold> = (0..t).map(|i| edge(i, i + 1)).collect();
    let mut g_at = ConstraintGraph::with_capacity(at_bodies.len());
    g_at.build(&at_manifolds, at_bodies.len(), is_dynamic_of(&at_bodies));
    assert_eq!(g_at.max_island_constraints(), t, "T-edge chain hits the threshold exactly");
    assert!(
        g_at.max_island_constraints() >= LARGE_ISLAND_CONSTRAINTS,
        "at-threshold scene is GATE-ON (parallel path eligible)"
    );

    // One below: a chain of T bodies has T-1 edges.
    let below_bodies: Vec<BodyState> =
        (0..t).map(|i| dyn_sphere(Vec3::new(i as f32, 0.0, 0.0))).collect();
    let below_manifolds: Vec<Manifold> = (0..t - 1).map(|i| edge(i, i + 1)).collect();
    let mut g_below = ConstraintGraph::with_capacity(below_bodies.len());
    g_below.build(&below_manifolds, below_bodies.len(), is_dynamic_of(&below_bodies));
    assert_eq!(g_below.max_island_constraints(), t - 1, "(T-1)-edge chain is one below");
    assert!(
        g_below.max_island_constraints() < LARGE_ISLAND_CONSTRAINTS,
        "one-below scene is GATE-OFF (forced single-threaded)"
    );
}

// ── Gates B + C: path selection + bit-equivalence under a real pool ───────────────

#[cfg(not(miri))]
mod pool_gates {
    use super::*;
    use boyko_physics::resources::{PhysicsConfig, SolverScratch};
    use boyko_physics::solver::ColoredSoftStepSolver;
    use boyko_threadpool::ThreadPoolBuilder;

    fn manifold(a: u32, b: u32, normal: Vec3, sep: f32, anchor: Vec3) -> Manifold {
        let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
        m.normal = normal;
        m.points[0] = ContactPoint { anchor_a: anchor, anchor_b: anchor, separation: sep, feature_id: 0 };
        m.count = 1;
        m
    }

    /// `n` dynamic spheres on ONE shared static floor → a single island of `n`
    /// manifolds (the floor is ground, so the dynamic rows do NOT connect to each
    /// other — but every dyn-vs-floor manifold files under that dynamic body's island;
    /// since the bodies are pairwise non-touching, this is `n` SINGLETON islands of one
    /// manifold each → `max_island_constraints == 1`). To get a LARGE island we instead
    /// chain the dynamic bodies together (each adjacent pair contacts), giving one
    /// island of `n - 1` dyn-dyn manifolds + `n` floor manifolds = a big connected
    /// island. We use the chained form for the large scene and the disjoint form for
    /// the small scene, so island SIZE (not body count) is the variable.
    fn chained_floor_scene(n: u32) -> (Vec<BodyState>, Vec<Manifold>) {
        let mut bodies = Vec::with_capacity(n as usize + 1);
        for i in 0..n {
            bodies.push(dyn_sphere(Vec3::new(i as f32 * 1.5, 0.6, 0.0)));
        }
        let floor = n;
        bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));

        let mut manifolds = Vec::with_capacity(2 * n as usize);
        for i in 0..n {
            // Dyn-vs-floor (ground attaches the body to its island, never merges).
            manifolds.push(manifold(
                i,
                floor,
                Vec3::new(0.0, -1.0, 0.0),
                -0.2,
                Vec3::new(i as f32 * 1.5, 0.0, 0.0),
            ));
            // Dyn-dyn chain link (merges adjacent bodies into ONE island).
            if i + 1 < n {
                manifolds.push(manifold(
                    i,
                    i + 1,
                    Vec3::new(1.0, 0.0, 0.0),
                    -0.1,
                    Vec3::new(i as f32 * 1.5 + 0.75, 0.6, 0.0),
                ));
            }
        }
        (bodies, manifolds)
    }

    /// `n` dynamic spheres, each in its OWN tiny 2-body island (pairs), well below the
    /// gate threshold — so the gate forces single-threaded regardless of body count.
    fn many_small_islands_scene(pairs: u32) -> (Vec<BodyState>, Vec<Manifold>) {
        let n = pairs * 2;
        let mut bodies = Vec::with_capacity(n as usize);
        for i in 0..n {
            // Space pairs far apart so only the intra-pair manifold exists; a wide gap
            // between pairs keeps each its own island.
            let pair = i / 2;
            let within = i % 2;
            bodies.push(dyn_sphere(Vec3::new(pair as f32 * 100.0 + within as f32, 0.6, 0.0)));
        }
        let mut manifolds = Vec::with_capacity(pairs as usize);
        for p in 0..pairs {
            manifolds.push(manifold(
                p * 2,
                p * 2 + 1,
                Vec3::new(1.0, 0.0, 0.0),
                -0.1,
                Vec3::new(p as f32 * 100.0 + 0.5, 0.6, 0.0),
            ));
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

    fn snapshot_bits(scratch: &SolverScratch) -> Vec<u32> {
        scratch
            .bodies()
            .iter()
            .flat_map(|b| {
                [
                    b.position.x.to_bits(),
                    b.position.y.to_bits(),
                    b.position.z.to_bits(),
                    b.linear_velocity.x.to_bits(),
                    b.linear_velocity.y.to_bits(),
                    b.linear_velocity.z.to_bits(),
                    b.angular_velocity.x.to_bits(),
                    b.angular_velocity.y.to_bits(),
                    b.angular_velocity.z.to_bits(),
                ]
            })
            .collect()
    }

    /// Runs `steps` of the SCALAR colored solve over `(bodies, manifolds)` at the given
    /// `parallel_solve` flag inside a `workers`-wide pool install frame. Returns the
    /// body snapshot bits.
    fn run(
        bodies: &[BodyState],
        manifolds: &[Manifold],
        steps: usize,
        parallel_solve: bool,
        workers: usize,
    ) -> Vec<u32> {
        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            parallel_solve,
            simd_solve: false,
            ..PhysicsConfig::default()
        };
        let graph = build_graph(bodies, manifolds);
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.set_bodies(bodies);
        scratch.touched.reset(scratch.bodies().len());

        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        pool.install(|_scope| {
            for _ in 0..steps {
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored(&cfg, manifolds, &graph, &mut scratch);
            }
        });
        snapshot_bits(&scratch)
    }

    /// Warms the solver on `(bodies, manifolds)` inside a pool with `parallel_solve`
    /// on, then measures the allocation count of ONE more warmed `solve_colored` step
    /// under the counting allocator. Returns `allocs_this_step`. A non-zero count means
    /// a `pool.scope` was dispatched (the gate let the parallel path run); zero means
    /// the gate forced single-threaded (no scope).
    fn warmed_parallel_step_allocs(
        bodies: &[BodyState],
        manifolds: &[Manifold],
        workers: usize,
    ) -> usize {
        let cfg =
            PhysicsConfig { dt: 1.0 / 60.0, parallel_solve: true, ..PhysicsConfig::default() };
        let graph = build_graph(bodies, manifolds);
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.set_bodies(bodies);
        scratch.touched.reset(scratch.bodies().len());

        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        pool.install(|_scope| {
            // Warm so every solver/scratch buffer reaches steady capacity.
            for _ in 0..8 {
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored(&cfg, manifolds, &graph, &mut scratch);
            }
            scratch.touched.reset(scratch.bodies().len());
            let before = ALLOC.count();
            solver.solve_colored(&cfg, manifolds, &graph, &mut scratch);
            ALLOC.count().wrapping_sub(before)
        })
    }

    // ── Gate B: island SIZE flips the path observable ─────────────────────────────

    #[test]
    fn small_island_scene_is_gated_to_single_thread() {
        // 200 disjoint 2-body islands: `max_island_constraints == 1`, far below the
        // gate threshold. With parallel_solve ON, the P2 gate must FORCE single-threaded
        // → ZERO `pool.scope` allocations on a warmed step.
        let (bodies, manifolds) = many_small_islands_scene(200);
        let g = build_graph(&bodies, &manifolds);
        assert_eq!(
            g.max_island_constraints(),
            1,
            "anti-vacuity: the small scene's largest island holds 1 manifold"
        );
        assert!(
            g.max_island_constraints() < LARGE_ISLAND_CONSTRAINTS,
            "the small scene is below the gate threshold"
        );

        let allocs = warmed_parallel_step_allocs(&bodies, &manifolds, 4);
        eprintln!("[P2 GateB] small max_island=1 parallel_solve=ON scope_allocs={allocs}");
        assert_eq!(
            allocs, 0,
            "a warmed parallel step on a SMALL-island scene must issue NO `pool.scope` \
             (the P2 gate forces single-threaded); got {allocs} allocs"
        );
    }

    #[test]
    fn large_island_scene_takes_the_parallel_path() {
        // One big chained island of ~2*n manifolds (n dyn-dyn + n dyn-floor),
        // comfortably above LARGE_ISLAND_CONSTRAINTS. With parallel_solve ON, the gate
        // lets it through → a real `pool.scope` dispatch → non-zero, BOUNDED allocs.
        let n = 600u32;
        let (bodies, manifolds) = chained_floor_scene(n);
        let g = build_graph(&bodies, &manifolds);
        assert!(
            g.max_island_constraints() >= LARGE_ISLAND_CONSTRAINTS,
            "anti-vacuity: the large scene's island ({}) must clear the gate threshold ({})",
            g.max_island_constraints(),
            LARGE_ISLAND_CONSTRAINTS
        );

        let allocs = warmed_parallel_step_allocs(&bodies, &manifolds, 4);
        eprintln!(
            "[P2 GateB] large max_island={} parallel_solve=ON scope_allocs={allocs}",
            g.max_island_constraints()
        );
        assert!(
            allocs > 0,
            "a warmed parallel step on a LARGE-island scene must dispatch a `pool.scope` \
             (the gate lets the parallel path run); got {allocs} allocs"
        );
    }

    // ── Gate C: both gated paths are BIT-IDENTICAL (the load-bearing equivalence) ─

    #[test]
    fn small_island_gated_path_is_bit_identical_to_serial() {
        // The gate forces the small scene single-threaded even with parallel_solve ON.
        // That gated result MUST bit-equal the explicit parallel_solve == false serial
        // run (the gate changes only WHERE the solve runs, never the bits).
        let (bodies, manifolds) = many_small_islands_scene(180);
        let steps = 6;

        let serial = run(&bodies, &manifolds, steps, false, 1);
        // parallel_solve ON but gate-forced single-threaded, at several worker counts.
        let gated_w1 = run(&bodies, &manifolds, steps, true, 1);
        let gated_w4 = run(&bodies, &manifolds, steps, true, 4);
        let gated_w8 = run(&bodies, &manifolds, steps, true, 8);

        assert_eq!(serial, gated_w1, "gated small scene (1w) must equal the serial bits");
        assert_eq!(serial, gated_w4, "gated small scene (4w) must equal the serial bits");
        assert_eq!(serial, gated_w8, "gated small scene (8w) must equal the serial bits");

        // Anti-vacuity: the scene actually solved (some body moved off its rest bits).
        let rest = run(&bodies, &manifolds, 0, false, 1);
        assert_ne!(rest, serial, "anti-vacuity: the solve must have moved the bodies");
    }

    #[test]
    fn large_island_gated_parallel_is_bit_identical_to_serial() {
        // The large scene is let through to the parallel dispatch. That parallel result
        // MUST bit-equal the single-threaded reference for any worker count (the
        // {1, N}-worker bit-identity property the gate preserves).
        let n = 400u32;
        let (bodies, manifolds) = chained_floor_scene(n);
        let g = build_graph(&bodies, &manifolds);
        assert!(
            g.max_island_constraints() >= LARGE_ISLAND_CONSTRAINTS,
            "anti-vacuity: the large scene must clear the gate so the parallel path runs"
        );
        let steps = 6;

        let serial = run(&bodies, &manifolds, steps, false, 1);
        let par_w1 = run(&bodies, &manifolds, steps, true, 1);
        let par_w2 = run(&bodies, &manifolds, steps, true, 2);
        let par_w4 = run(&bodies, &manifolds, steps, true, 4);
        let par_w8 = run(&bodies, &manifolds, steps, true, 8);

        assert_eq!(serial, par_w1, "gated-parallel large scene (1w) must equal serial bits");
        assert_eq!(serial, par_w2, "gated-parallel large scene (2w) must equal serial bits");
        assert_eq!(serial, par_w4, "gated-parallel large scene (4w) must equal serial bits");
        assert_eq!(serial, par_w8, "gated-parallel large scene (8w) must equal serial bits");
    }

    // ── Counting global allocator (mirrors colored_parallel_alloc_o6.rs) ──────────

    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
    }

    struct CountingAlloc;

    impl CountingAlloc {
        fn count(&self) -> usize {
            ALLOC_COUNT.with(|c| c.get())
        }
    }

    #[inline]
    fn bump_alloc_count() {
        let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
    }

    // SAFETY: every call forwards verbatim to the platform `System` allocator with the
    // same layout; the wrapper only bumps a thread-local counter (via `try_with` that
    // no-ops if TLS is mid-init, so it never re-enters the allocator). `dealloc` is an
    // unchanged pass-through, so the allocator contract is exactly `System`'s.
    unsafe impl GlobalAlloc for CountingAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            bump_alloc_count();
            // SAFETY: forwarded verbatim to the system allocator (same layout).
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: `ptr`/`layout` originate from `System.alloc` above.
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            bump_alloc_count();
            // SAFETY: `ptr`/`layout` originate from this allocator; `new_size` forwarded.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    #[global_allocator]
    static ALLOC: CountingAlloc = CountingAlloc;
}
