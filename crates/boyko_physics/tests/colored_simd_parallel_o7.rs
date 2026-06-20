//! Phase O7 — the {parallel × simd × N-worker} bit-identity + in-span gather gate
//! (the W1 CRITICAL-path test for the C1 cross-worker READ-race fix).
//!
//! The committed O7 review found no test combined `parallel_solve == true` +
//! `simd_solve == true` + a real `ThreadPool` — the exact path where the per-rank
//! gather could read a CONCURRENTLY-RUNNING worker's slots (a value-benign but real
//! data race: the discarded lane's slot lay in the next worker's `[chunk_start,
//! chunk_end)` span, which that worker writes every rank). The C1 fix clamps every
//! gathered slot to the LANE'S OWN GROUP last slot (an absent lane to `chunk_start`),
//! so every read provably stays in the worker's own span.
//!
//! This file drives a LARGE, RAGGED, MULTI-COHORT colored scene through a real
//! pool at `{parallel=false, simd=true}`, `{parallel=true, 1 worker, simd=true}`,
//! and `{parallel=true, N workers, simd=true}`, asserting ALL are bit-identical to
//! the `{parallel=false, simd=false}` scalar oracle. A pure bit-equality assert can
//! NOT catch the race on its own (the foreign value is `blendv`-discarded), so the
//! kernel carries a `debug_assert!` that every gathered slot lies in `[chunk_start,
//! chunk_end)`; this test runs in DEBUG (the default `cargo test` profile), so the
//! N-worker arm fires that assertion if ANY lane reads out of span.
//!
//! The scene: ~300 dynamic spheres on ONE shared static floor (≈300 body-disjoint
//! width-1 groups in a single color ⇒ ≫ MIN_PARALLEL_SLOTS_PER_COLOR=256 slots and
//! ≫ 9 manifold-groups, so cohort-snapping cuts a PARTIAL trailing cohort) plus
//! several width-4 box-box manifolds (RAGGED widths, distinct dynamic pairs ⇒ same
//! color) so a narrow (width-1) and a wide (width-4) group meet at cohort/task
//! boundaries.
//!
//! Gated `#[cfg(not(miri))]` (spins the threadpool; Miri-intractable int-to-ptr)
//! and `#[cfg(target_feature = "avx2")]` (the SIMD solve only widens under +avx2;
//! a non-AVX2 build routes both arms through the scalar oracle, making the property
//! vacuous — the always-compiled lib `simd_solve_width_only_matches_scalar_step`
//! covers that build).

#![cfg(all(not(miri), target_arch = "x86_64", target_feature = "avx2"))]

use boyko_physics::components::{BodyType, ColliderShape};
use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{MAX_CONTACT_POINTS, Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};
use boyko_physics::solver::ColoredSoftStepSolver;
use boyko_threadpool::ThreadPoolBuilder;

// ── Scene helpers ─────────────────────────────────────────────────────────────

fn dyn_sphere(position: Vec3, lin: Vec3, ang: Vec3) -> BodyState {
    BodyState {
        // A non-trivial diagonal inertia so the angular term is non-vacuous.
        inv_inertia: Mat3::from_diagonal(Vec3::new(1.5, 1.2, 1.3)),
        inv_inertia_local: Mat3::from_diagonal(Vec3::new(1.5, 1.2, 1.3)),
        position,
        linear_velocity: lin,
        angular_velocity: ang,
        rotation: Quat::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.0,
        friction: 0.7,
        body_type: BodyType::Dynamic,
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
        shape: ColliderShape::Sphere { radius: 1.0 },
    }
}

fn manifold(a: u32, b: u32, normal: Vec3, sep: f32, anchor: Vec3) -> Manifold {
    let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
    m.normal = normal;
    m.points[0] = ContactPoint { anchor_a: anchor, anchor_b: anchor, separation: sep, feature_id: 0 };
    m.count = 1;
    m
}

/// A `n`-point (≤ MAX_CONTACT_POINTS) box-box manifold: one body pair, one group.
fn box_manifold(a: u32, b: u32, normal: Vec3, sep: f32, anchor: Vec3, n: u8) -> Manifold {
    let n = (n as usize).min(MAX_CONTACT_POINTS);
    let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
    m.normal = normal;
    for (p, slot) in m.points.iter_mut().take(n).enumerate() {
        let off = Vec3::new(p as f32 * 0.1, 0.0, p as f32 * 0.1);
        *slot = ContactPoint {
            anchor_a: anchor + off,
            anchor_b: anchor + off,
            separation: sep,
            feature_id: p as u32,
        };
    }
    m.count = n as u8;
    m
}

/// `n_floor` dynamic spheres over ONE shared static floor + `n_box` width-4 box
/// pairs. All dynamic rows are pairwise distinct, so the floor-spheres and the box
/// pairs are body-disjoint and land in a single large RAGGED color.
fn ragged_multicohort_scene(n_floor: u32, n_box: u32) -> Vec<BodyState> {
    let mut bodies = Vec::new();
    for i in 0..n_floor {
        bodies.push(dyn_sphere(
            Vec3::new(i as f32 * 3.0, 0.6, 0.0),
            Vec3::new(0.2 * (i as f32 + 1.0), -1.0, 0.1),
            Vec3::new(0.05, -0.1, 0.2),
        ));
    }
    // Box pairs (2 dynamic bodies each).
    for j in 0..n_box {
        let base_x = -5.0 - j as f32 * 6.0;
        bodies.push(dyn_sphere(
            Vec3::new(base_x, 10.0, 0.0),
            Vec3::new(1.0, 0.0, -0.3),
            Vec3::new(0.1, 0.2, -0.15),
        ));
        bodies.push(dyn_sphere(
            Vec3::new(base_x + 2.0, 10.0, 0.0),
            Vec3::new(-1.0, 0.0, 0.3),
            Vec3::new(-0.2, 0.05, 0.1),
        ));
    }
    // The shared static floor (last row).
    bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));
    bodies
}

fn ragged_multicohort_manifolds(bodies: &[BodyState], n_floor: u32, n_box: u32) -> Vec<Manifold> {
    let floor_row = (bodies.len() - 1) as u32;
    let mut out = Vec::new();
    // Floor spheres: width-1 groups. Interleave the box manifolds so a narrow
    // (width-1) and a wide (width-4) group meet at cohort/task boundaries.
    for i in 0..n_floor {
        out.push(manifold(
            i,
            floor_row,
            Vec3::new(0.0, -1.0, 0.0),
            -0.2,
            Vec3::new(i as f32 * 3.0, 0.0, 0.0),
        ));
        // Insert a box manifold every ~7 floor groups (a non-multiple-of-8 stride
        // so wide groups straddle cohort boundaries within the color).
        if i % 7 == 6 {
            let j = i / 7;
            if j < n_box {
                let box_a = n_floor + 2 * j;
                let box_b = box_a + 1;
                out.push(box_manifold(
                    box_a,
                    box_b,
                    Vec3::new(1.0, 0.0, 0.0),
                    -0.3,
                    Vec3::new(-5.0 - j as f32 * 6.0 + 1.0, 10.0, 0.0),
                    4,
                ));
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

/// Runs the colored solve for `steps` over the ragged multi-cohort scene with the
/// given `(parallel_solve, simd_solve)` flags, inside a `workers`-wide pool install
/// frame so the parallel path finds the ambient pool. Returns the body snapshot
/// bits. Running in the default DEBUG profile keeps the kernel's in-span
/// `debug_assert!` live (the C1 structural guard).
fn run_in_pool(
    n_floor: u32,
    n_box: u32,
    steps: usize,
    parallel_solve: bool,
    simd_solve: bool,
    workers: usize,
) -> Vec<u32> {
    let cfg = PhysicsConfig {
        dt: 1.0 / 60.0,
        parallel_solve,
        simd_solve,
        ..PhysicsConfig::default()
    };
    let mut solver = ColoredSoftStepSolver::default();
    let bodies = ragged_multicohort_scene(n_floor, n_box);
    let mut scratch = SolverScratch::with_capacity(bodies.len());
    scratch.set_bodies(&bodies);
    scratch.touched.reset(scratch.bodies().len());

    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    pool.install(|_scope| {
        for _ in 0..steps {
            let manifolds = ragged_multicohort_manifolds(scratch.bodies(), n_floor, n_box);
            let graph = build_graph(scratch.bodies(), &manifolds);
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
        }
    });
    snapshot_bits(&scratch)
}

/// Test 3 (the W1 CRITICAL path): `{parallel × simd × N-worker}` must be
/// bit-identical to the `{parallel=false, simd=false}` scalar oracle, run in DEBUG
/// so the kernel's in-span gather `debug_assert!` (the C1 structural proof) fires
/// on any out-of-span read.
#[test]
fn simd_parallel_multiworker_is_bit_identical_and_in_span() {
    // ≈300 floor spheres ⇒ one color far above 256 slots / 9 groups; ~40 box pairs
    // give RAGGED widths straddling cohort boundaries.
    let (n_floor, n_box) = (300u32, 40u32);
    let steps = 6;

    // The scalar oracle (the 0%-gate reference): no SIMD, no parallel.
    let scalar = run_in_pool(n_floor, n_box, steps, false, false, 1);

    // {parallel=false, simd=true}: the inline cohort kernel over the whole color.
    let simd_inline = run_in_pool(n_floor, n_box, steps, false, true, 1);
    // {parallel=true, 1 worker, simd=true}: dispatched scope, cohort-snapped chunks.
    let simd_p1 = run_in_pool(n_floor, n_box, steps, true, true, 1);
    // {parallel=true, N workers, simd=true}: the CRITICAL cross-worker path — if any
    // lane gathered a foreign slot, the in-span debug_assert aborts this run.
    let simd_p2 = run_in_pool(n_floor, n_box, steps, true, true, 2);
    let simd_p4 = run_in_pool(n_floor, n_box, steps, true, true, 4);
    let simd_p8 = run_in_pool(n_floor, n_box, steps, true, true, 8);

    assert_eq!(scalar, simd_inline, "simd_solve inline must equal the scalar oracle");
    assert_eq!(scalar, simd_p1, "simd_solve + parallel (1 worker) must equal the scalar oracle");
    assert_eq!(scalar, simd_p2, "simd_solve + parallel (2 workers) must equal the scalar oracle");
    assert_eq!(scalar, simd_p4, "simd_solve + parallel (4 workers) must equal the scalar oracle");
    assert_eq!(scalar, simd_p8, "simd_solve + parallel (8 workers) must equal the scalar oracle");

    // Anti-vacuity: the scene must actually have solved (bodies moved off rest).
    let rest = {
        let mut s = SolverScratch::with_capacity((n_floor + 2 * n_box + 1) as usize);
        s.set_bodies(&ragged_multicohort_scene(n_floor, n_box));
        snapshot_bits(&s)
    };
    assert_ne!(scalar, rest, "the ragged multi-cohort scene must non-vacuously solve");
}
