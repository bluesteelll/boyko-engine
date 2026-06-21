//! Phase O6 Gate 6 — BOUNDED scope allocation in the PARALLEL colored solve (the
//! W1 min-work-threshold fix).
//!
//! The O6 review's MAJOR was that a naive per-color `pool.scope` allocated once per
//! color per pass (~`substeps × (1 + relax) × n_colors` boxed frames/closures every
//! step — a data-dependent per-step alloc that fails the campaign's zero-per-step
//! gate). The fix: `MIN_PARALLEL_SLOTS_PER_COLOR` — a color below the threshold is
//! solved INLINE (no scope), so `pool.scope` is reached ONLY for the FEW large
//! colors that amortize the dispatch. This file proves it with a counting
//! `#[global_allocator]`, on TWO worlds driven through a real `ThreadPool`:
//!
//! 1. **Small-color world** (every color below the threshold): a warmed parallel
//!    step allocates ≈ 0 — NO `pool.scope` is issued (everything inline), so the
//!    parallel path's per-step alloc matches the single-threaded path. This is the
//!    "small-color worlds don't pay a scope tax" guarantee.
//! 2. **Dense / large-color world** (the shared-floor color far above the
//!    threshold): a warmed parallel step's scope allocation is BOUNDED to a small
//!    constant proportional to `passes × #colors-above-threshold` — NOT `~12 ×
//!    n_colors`. We bound it well below what one-scope-per-color would cost, and
//!    assert it does NOT grow with the (large) total color count (the bound is the
//!    same whether the dense scene has dozens or hundreds of small colors, because
//!    only the ONE huge floor-color crosses the threshold).
//!
//! The solver's OWN scratch stays zero-per-step-alloc in BOTH worlds (the W2
//! capacity-reuse guarantee); the only residual is the threshold-bounded
//! `pool.scope` dispatch cost — the justified parallelism overhead (the same
//! per-dispatch cost class as `Query::par_iter`).
//!
//! Gated `#[cfg(not(miri))]`: spins up the threadpool (Miri-intractable int-to-ptr,
//! Phase 9.1-9.3) and the counting-allocator wrapper is a known Miri harness
//! artifact in std shutdown.

#![cfg(not(miri))]

use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};
use boyko_physics::solver::ColoredSoftStepSolver;
use boyko_physics::components::{BodyType, ColliderShape};
use boyko_threadpool::ThreadPoolBuilder;

// ── Scene helpers (mirror the lib test fixtures) ──────────────────────────────

fn dyn_sphere(position: Vec3) -> BodyState {
    BodyState {
        inv_inertia: Mat3::ZERO,
        inv_inertia_local: Mat3::ZERO,
        position,
        linear_velocity: Vec3::ZERO,
        angular_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.0,
        friction: 0.5,
        body_type: BodyType::Dynamic,
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
        body_type: BodyType::Static,
        is_sensor: false,
        shape: ColliderShape::Sphere { radius: 1.0 },
    }
}

fn manifold(a: u32, b: u32, normal: Vec3, sep: f32, anchor: Vec3) -> Manifold {
    let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
    m.normal = normal;
    m.points[0] = ContactPoint {
        anchor_a: anchor,
        anchor_b: anchor,
        separation: sep,
        feature_id: 0,
    };
    m.count = 1;
    m
}

/// `n` dynamic spheres in a tight overlapping line over one shared static floor.
fn dense_scene(n: usize) -> Vec<BodyState> {
    let mut bodies: Vec<BodyState> = (0..n)
        .map(|i| dyn_sphere(Vec3::new(i as f32 * 1.5, 1.0, 0.0)))
        .collect();
    bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));
    bodies
}

/// Each adjacent dynamic pair + each dynamic-vs-floor contact (manifold order).
fn dense_manifolds(bodies: &[BodyState]) -> Vec<Manifold> {
    let n = bodies.len() - 1;
    let floor = n as u32;
    let mut out = Vec::new();
    for a in 0..n {
        out.push(manifold(a as u32, floor, Vec3::new(0.0, -1.0, 0.0), -0.2, bodies[a].position));
        if a + 1 < n {
            let pa = bodies[a].position;
            let pb = bodies[a + 1].position;
            let delta = pb - pa;
            let dist = delta.length();
            if dist > 1e-6 {
                let normal = delta * dist.recip();
                out.push(manifold(a as u32, (a + 1) as u32, normal, dist - 2.0, pa + normal));
            }
        }
    }
    out
}

fn build_graph(bodies: &[BodyState], manifolds: &[Manifold]) -> ConstraintGraph {
    let mut g = ConstraintGraph::with_capacity(bodies.len());
    let inv_mass: Vec<f32> = bodies.iter().map(|b| b.inv_mass).collect();
    g.build(manifolds, bodies.len(), move |row| {
        (row as usize) < inv_mass.len() && inv_mass[row as usize] != 0.0
    });
    g
}

/// Warms a `ColoredSoftStepSolver` on the scene inside a pool, then measures the
/// allocation count of ONE more warmed `solve_colored` step under the counting
/// allocator. Returns `(allocs_this_step, n_colors)`.
///
/// The counting allocator is thread-local and the measurement runs on the
/// dispatcher thread (inside `pool.install`); `pool.scope`'s shared frame box and
/// the per-spawn closure boxes are allocated on THAT thread (`scope.spawn` boxes on
/// the calling thread), so the dispatcher's counter captures exactly the scope
/// dispatch allocation we are bounding.
fn warmed_parallel_step_allocs(n: usize, workers: usize) -> (usize, usize) {
    let cfg = PhysicsConfig { dt: 1.0 / 60.0, parallel_solve: true, ..PhysicsConfig::default() };
    let mut solver = ColoredSoftStepSolver::default();
    let mut scratch = SolverScratch::with_capacity(n + 1);
    scratch.set_bodies(&dense_scene(n));
    scratch.touched.reset(scratch.bodies().len());

    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let mut n_colors = 0usize;
    let allocs = pool.install(|_scope| {
        // Warm: several steps so every solver/graph/scratch buffer reaches steady
        // capacity (clear()+refill thereafter).
        for _ in 0..8 {
            let manifolds = dense_manifolds(scratch.bodies());
            let graph = build_graph(scratch.bodies(), &manifolds);
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
        }
        // Rebuild the manifolds + graph OUTSIDE the measurement window (they are the
        // narrowphase/partition work, not the colored solver's per-step alloc).
        let manifolds = dense_manifolds(scratch.bodies());
        let graph = build_graph(scratch.bodies(), &manifolds);
        n_colors = graph.n_colors() as usize;
        scratch.touched.reset(scratch.bodies().len());

        let before = ALLOC.count();
        solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
        ALLOC.count().wrapping_sub(before)
    });
    (allocs, n_colors)
}

// ── Gate 6a: small-color world pays ZERO scope tax (everything inline) ─────────

#[test]
fn small_color_world_pays_no_scope_alloc_tax() {
    // A SMALL-color world: only a handful of dynamic bodies, so every color's slot
    // count is far below MIN_PARALLEL_SLOTS_PER_COLOR (256) → `solve_color_parallel`
    // takes the INLINE branch for every color, issuing NO `pool.scope`. The warmed
    // parallel step must therefore allocate ZERO (the colored solver's scratch is
    // capacity-reused AND no scope is dispatched).
    let (allocs, n_colors) = warmed_parallel_step_allocs(8, 4);
    eprintln!("[O6 Gate6] small n=8 workers=4 n_colors={n_colors} scope_allocs={allocs}");
    // Anti-vacuity: a real multi-color partition (else the threshold path is untested).
    assert!(n_colors > 1, "anti-vacuity: small-color world must still have > 1 color (got {n_colors})");
    assert_eq!(
        allocs, 0,
        "a warmed PARALLEL step on a small-color world (every color < the threshold) \
         must allocate ZERO — no `pool.scope` is issued (all colors solved inline), \
         and the solver scratch is capacity-reused; got {allocs} allocs across {n_colors} colors"
    );
}

// ── Gate 6b: dense world's scope alloc is BOUNDED, not ~12×n_colors ────────────

#[test]
fn dense_world_scope_alloc_is_bounded_to_large_colors() {
    // A DENSE world: 600 dynamic bodies in a packed line over one shared floor. The
    // shared-floor color holds ~600 slots (≫ 256 → a real `pool.scope` dispatch);
    // the adjacent-pair colors are each tiny (< 256 → inline). So scopes are issued
    // ONLY for the FEW large colors, on each of the `substeps × (1 + relax)` passes.
    //
    // With the defaults (substeps = 4, relax = 2 → 12 passes) and ~1 color above the
    // threshold, the per-step scope count is ~12 — each scope allocating a small
    // bounded constant (a boxed shared frame + ≤ worker_count boxed spawn closures).
    // The bound is INDEPENDENT of n_colors (only the floor-color crosses the
    // threshold), which is the whole point of the W1 fix.
    let workers = 4;
    let (allocs, n_colors) = warmed_parallel_step_allocs(600, workers);
    eprintln!("[O6 Gate6] dense n=600  workers={workers}  n_colors={n_colors}  scope_allocs={allocs}");

    // Anti-vacuity: a genuinely multi-color dense partition.
    assert!(
        n_colors > 2,
        "anti-vacuity: dense world must have many colors (got {n_colors})"
    );

    // Per-step passes = substeps × (1 + relax). Per pass, the FEW large color(s)
    // each issue a `pool.scope`, each allocating a small bounded constant (1 shared
    // frame + ≤ (workers + 1) × CHUNKS_PER_WORKER spawn closures + internal
    // bookkeeping). The dense packed line has exactly ONE color above the threshold
    // (the shared floor), so the per-step scope-alloc count is ~`passes × per_scope`
    // — a small constant INDEPENDENT of the total color count. We bound it generously
    // (the O6 work-balanced chunking emits up to (workers+1) × CHUNKS_PER_WORKER
    // work-balanced chunks so the work-stealing pool equalizes the lanes; the
    // measured per-scope footprint is ~closures + frame + completion-channel/worker
    // bookkeeping), and the load-bearing assertion below is the n_colors-INDEPENDENCE
    // check, not this absolute number.
    let passes = 4 * (1 + 2); // substeps × (1 + relax_iterations) with the defaults
    // 1 frame + ≤ (workers+1) × CHUNKS_PER_WORKER closures + per-closure/pool
    // bookkeeping. CHUNKS_PER_WORKER (= 6) is a private const mirrored here only for
    // this bound; keep in sync with `colored.rs::CHUNKS_PER_WORKER`.
    let chunks_per_worker = 6;
    let per_scope_cap = 16 + (workers + 1) * chunks_per_worker * 5; // generous
    let large_colors_cap = 2; // the dense floor-line crosses the threshold in ~1 color
    let bound = passes * per_scope_cap * large_colors_cap;

    assert!(
        allocs <= bound,
        "dense-world warmed PARALLEL step allocs = {allocs} (expected <= {bound} = {passes} passes \
         × {per_scope_cap} per-scope × {large_colors_cap} large-colors-cap). A leak here that \
         scaled with n_colors ({n_colors}) would be ONE-SCOPE-PER-COLOR — the bug the W1 \
         threshold fixes; the bound is independent of n_colors."
    );

    // THE load-bearing property of the W1 fix: the scope-alloc count does NOT grow
    // with n_colors. Measure a SECOND dense world with DOUBLE the bodies (hence more
    // total colors AND a wider floor-color) and assert the scope-alloc count is
    // ESSENTIALLY UNCHANGED — it tracks #large-colors × passes (still ~1 large
    // color), NOT total n_colors. If the alloc were one-scope-per-color, doubling
    // the bodies (≈ doubling n_colors) would roughly double the count; instead it
    // must stay within a tiny tolerance of the smaller world's count.
    let (allocs_bigger, n_colors_bigger) = warmed_parallel_step_allocs(1200, workers);
    eprintln!("[O6 Gate6] dense n=1200 workers={workers} n_colors={n_colors_bigger} scope_allocs={allocs_bigger}");
    assert!(
        n_colors_bigger >= n_colors,
        "the bigger dense world should have >= as many colors ({n_colors_bigger} vs {n_colors})"
    );
    // n_colors-independence: a per-color leak would scale the alloc with n_colors;
    // the threshold-bounded path keeps it within `large_colors_cap × passes ×
    // per_scope` of the smaller world regardless of how many SMALL colors exist.
    let delta = allocs_bigger.abs_diff(allocs);
    assert!(
        delta <= passes * per_scope_cap,
        "doubling the body count (n_colors {n_colors} → {n_colors_bigger}) changed the scope-alloc \
         count by {delta} (allocs {allocs} → {allocs_bigger}); a one-scope-per-color leak would \
         scale it with n_colors. The W1 threshold guarantees the alloc tracks #large-colors × \
         passes (~constant), NOT total n_colors — the delta must stay within one large-color's \
         {passes}-pass budget ({})", passes * per_scope_cap
    );
    assert!(
        allocs_bigger <= bound,
        "the bigger dense world's scope-alloc ({allocs_bigger}) must still be within the same \
         n_colors-independent bound {bound}"
    );
}

// ── Counting global allocator (mirrors colored_solve_zero_alloc_o5.rs) ─────────

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
