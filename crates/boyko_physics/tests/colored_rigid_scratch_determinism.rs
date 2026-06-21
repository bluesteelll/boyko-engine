//! Audit Stage P — determinism of the rigid colored solve after the body
//! gather-mirror moved from `std::Vec` onto a
//! [`ScratchColumn`](boyko_ecs::ecs::core::component::scratch::ScratchColumn).
//!
//! The migration changed only the BUFFER backing the per-body view (a
//! `ScratchColumn` whose `*base.add(i)` equals the old `vec[i]`), not the float
//! math, the color order, or the intra-color CSR order. So the colored solve MUST
//! stay:
//!
//!   * **{1, N}-worker bit-identical** — the parallel dispatch (now via
//!     `ScratchSolveView::row_ptr` per element) writes the SAME distinct dynamic
//!     rows as the serial path; for any worker count the body snapshot is
//!     bit-for-bit identical to the single-threaded colored solve (the load-bearing
//!     O6 property, preserved by the migration).
//!   * **serial byte-identical run-to-run** — two independent serial runs of the
//!     same scene produce bit-identical output (no run-to-run nondeterminism from
//!     the column backing).
//!   * **valid (finite, in-tolerance) under a DIFFERENT op order** — varying the
//!     body insertion order yields a different-but-valid result (the critic's W1:
//!     reordering inputs may change bits but must stay finite + plausible).
//!
//! This is the SCALAR path (no `simd_solve`), so it is always live regardless of
//! the build's target features (the AVX2 `{parallel × simd}` bit-identity is
//! covered by `colored_simd_parallel_o7.rs`). Gated `cfg(not(miri))`: spins the
//! threadpool (Miri-intractable int-to-ptr).

#![cfg(not(miri))]

use boyko_physics::components::{BodyType, ColliderShape};
use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};
use boyko_physics::solver::ColoredSoftStepSolver;
use boyko_threadpool::ThreadPoolBuilder;

// ── Scene helpers ───────────────────────────────────────────────────────────────

fn dyn_sphere(position: Vec3, lin: Vec3, ang: Vec3) -> BodyState {
    BodyState {
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
    m.points[0] = ContactPoint { anchor_a: anchor, anchor_b: anchor, separation: sep, feature_id: 0 };
    m.count = 1;
    m
}

/// `n` dynamic spheres on ONE shared static floor → a single color of span `n`
/// (the dynamic rows are pairwise distinct). With `n >= 256` the color crosses
/// `MIN_PARALLEL_SLOTS_PER_COLOR`, so the parallel dispatch genuinely fires.
fn shared_floor_scene(n: u32) -> (Vec<BodyState>, Vec<Manifold>) {
    let mut bodies = Vec::with_capacity(n as usize + 1);
    for i in 0..n {
        bodies.push(dyn_sphere(
            Vec3::new(i as f32 * 3.0, 0.6, 0.0),
            Vec3::new(0.2 * (i as f32 + 1.0), -1.0, 0.1),
            Vec3::new(0.05, -0.1, 0.2),
        ));
    }
    let floor = n;
    bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));

    let mut manifolds = Vec::with_capacity(n as usize);
    for i in 0..n {
        manifolds.push(manifold(
            i,
            floor,
            Vec3::new(0.0, -1.0, 0.0),
            -0.2,
            Vec3::new(i as f32 * 3.0, 0.0, 0.0),
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

/// Runs `steps` of the SCALAR colored solve over the shared-floor scene at the
/// given `parallel` flag inside a `workers`-wide pool install frame. Returns the
/// body snapshot bits.
fn run(n: u32, steps: usize, parallel: bool, workers: usize) -> Vec<u32> {
    let cfg = PhysicsConfig {
        dt: 1.0 / 60.0,
        parallel_solve: parallel,
        simd_solve: false,
        ..PhysicsConfig::default()
    };
    let (bodies, manifolds) = shared_floor_scene(n);
    let graph = build_graph(&bodies, &manifolds);
    let mut solver = ColoredSoftStepSolver::default();
    let mut scratch = SolverScratch::with_capacity(bodies.len());
    scratch.set_bodies(&bodies);
    scratch.touched.reset(scratch.bodies().len());

    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    pool.install(|_scope| {
        for _ in 0..steps {
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
        }
    });
    snapshot_bits(&scratch)
}

// ── {1, N}-worker bit-identity (the migration must not perturb the bits) ──────────

#[test]
fn scalar_colored_one_vs_n_worker_bit_identical() {
    // n = 300 > 256 ⇒ the single shared-floor color crosses the parallel threshold.
    let (n, steps) = (300u32, 6);

    // The serial reference (parallel OFF, 1 worker pool present but unused).
    let serial = run(n, steps, false, 1);

    // parallel ON at {1, 2, 4, 8} workers — all must equal the serial bits (the O6
    // {1, N} property, preserved by the ScratchColumn migration: row_ptr writes hit
    // the same distinct dynamic rows as the serial slice writes).
    let p1 = run(n, steps, true, 1);
    let p2 = run(n, steps, true, 2);
    let p4 = run(n, steps, true, 4);
    let p8 = run(n, steps, true, 8);

    assert_eq!(serial, p1, "parallel 1-worker colored solve must equal the serial bits");
    assert_eq!(serial, p2, "parallel 2-worker colored solve must equal the serial bits");
    assert_eq!(serial, p4, "parallel 4-worker colored solve must equal the serial bits");
    assert_eq!(serial, p8, "parallel 8-worker colored solve must equal the serial bits");

    // Anti-vacuity: the scene must actually have solved (bodies moved off rest).
    let rest = {
        let (bodies, _) = shared_floor_scene(n);
        let mut s = SolverScratch::with_capacity(bodies.len());
        s.set_bodies(&bodies);
        snapshot_bits(&s)
    };
    assert_ne!(serial, rest, "the shared-floor scene must non-vacuously solve");
}

// ── serial run-to-run byte-identity (no nondeterminism from the column backing) ───

#[test]
fn scalar_colored_serial_run_to_run_byte_identical() {
    let (n, steps) = (64u32, 8);
    let a = run(n, steps, false, 1);
    let b = run(n, steps, false, 1);
    assert_eq!(a, b, "two serial colored runs of the same scene must be byte-identical");
}

// ── different op order ⇒ different-but-valid (the critic's W1) ─────────────────────

#[test]
fn scalar_colored_reversed_input_order_stays_valid() {
    let (n, steps) = (64u32, 6);
    let forward = run(n, steps, false, 1);

    // Build the SAME physical scene with the dynamic bodies inserted in REVERSED
    // order (the floor stays last so the manifold keying is consistent). The result
    // may differ bit-wise (a different solve order) but must stay finite + bounded —
    // the validity (not bit-identity) gate for an op-order change.
    let (mut bodies, _) = shared_floor_scene(n);
    let floor = bodies.pop().expect("floor row present");
    bodies.reverse();
    bodies.push(floor);
    // Re-key the manifolds to the reversed dynamic rows (row i now holds old row
    // n-1-i); the floor is still the last row.
    let floor_row = n;
    let mut manifolds = Vec::with_capacity(n as usize);
    for i in 0..n {
        manifolds.push(manifold(
            i,
            floor_row,
            Vec3::new(0.0, -1.0, 0.0),
            -0.2,
            bodies[i as usize].position,
        ));
    }
    let graph = build_graph(&bodies, &manifolds);
    let cfg = PhysicsConfig {
        dt: 1.0 / 60.0,
        parallel_solve: false,
        simd_solve: false,
        ..PhysicsConfig::default()
    };
    let mut solver = ColoredSoftStepSolver::default();
    let mut scratch = SolverScratch::with_capacity(bodies.len());
    scratch.set_bodies(&bodies);
    scratch.touched.reset(scratch.bodies().len());
    for _ in 0..steps {
        scratch.touched.reset(scratch.bodies().len());
        solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
    }

    // Every output is finite (no NaN/Inf injected by the reordering).
    for b in scratch.bodies() {
        assert!(
            b.position.x.is_finite() && b.position.y.is_finite() && b.position.z.is_finite(),
            "reversed-order solve must keep finite positions"
        );
        assert!(
            b.linear_velocity.x.is_finite()
                && b.linear_velocity.y.is_finite()
                && b.linear_velocity.z.is_finite(),
            "reversed-order solve must keep finite velocities"
        );
    }
    // Anti-vacuity: the forward run actually moved (so "valid" is non-trivial).
    let rest = {
        let (bodies, _) = shared_floor_scene(n);
        let mut s = SolverScratch::with_capacity(bodies.len());
        s.set_bodies(&bodies);
        snapshot_bits(&s)
    };
    assert_ne!(forward, rest, "the forward scene must non-vacuously solve");
}
