//! Audit Stage P — the rigid colored solve's Miri-TB soundness oracle after the
//! `std::Vec` gather-mirrors (P1: the body gather-mirror; P2: the contact
//! `ContactColumns` SoA) were migrated onto the engine's own
//! [`ScratchColumn`](boyko_ecs::ecs::core::component::scratch::ScratchColumn).
//!
//! The migration's load-bearing claim is STRUCTURAL on BOTH paths: the rigid
//! colored worker no longer forms a whole-buffer reborrow of either the bodies
//! (`ColorSolvePtrs::bodies()` `from_raw_parts_mut`, P1) OR the contact columns
//! (`ColorSolvePtrs::columns()` `&mut *self.cols` whole-struct reborrow, P2 —
//! DELETED). A worker now reaches each body row through a `ScratchSolveView`'s
//! per-element `row_ptr` and each contact column element through a
//! `ContactSolveView`'s per-element `base + index` accessor, so the SP4/rigid-class
//! whole-buffer reborrow is un-typeable on the worker path. That claim is
//! VALUE-BENIGN (the per-element writes target the same rows as before), so only a
//! borrow-model checker can witness it — a snapshot/bit test cannot.
//!
//! This file drives a scene whose SINGLE color crosses
//! `MIN_PARALLEL_SLOTS_PER_COLOR (256)`, so `solve_color_parallel` takes the
//! PARALLEL branch and the `ColorSolvePtrs` + per-element `row_ptr` writes ACTUALLY
//! execute across real worker threads under Tree-Borrows:
//!
//!   * **(a) pool-free** — no ambient pool ⇒ every color dispatches INLINE on the
//!     calling thread (the serial `solve_color` over the `ScratchSolveView`). This
//!     witnesses the SERIAL row-ptr access path (the shared `body_ref` / `body_mut`
//!     derefs) with zero threadpool involvement.
//!   * **(b) multi-worker** — a 2-worker `ThreadPool` wrapped around the solve via
//!     `pool.install`, on a scene whose widest color is ~300 slots (≫ 256), so the
//!     `pool.scope` dispatch fires and the `ScratchSolveView::row_ptr` writes run
//!     concurrently. A SHARED static floor row (`inv_mass == 0`) is referenced by
//!     every group but written by NONE (the `is_dynamic_row` guard) — the
//!     pinned-write witness: many workers read it, none writes it.
//!
//! Run (the load-bearing command):
//! ```text
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation" \
//!   cargo +nightly-x86_64-pc-windows-gnu miri test -p boyko-physics \
//!   --test colored_rigid_scratch_miri -- --test-threads=1
//! ```
//!
//! If the threadpool's worker-steal path trips the PRE-EXISTING crossbeam-deque
//! retag under Miri (a known executor over-approximation, NOT a Stage-P defect),
//! it surfaces in `(b)`; the `(a)` inline test still witnesses the row-ptr access
//! with ZERO threadpool involvement, so the migration's unsafe is covered either
//! way.

use boyko_physics::components::{BodyType, ColliderShape};
use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};
use boyko_physics::solver::ColoredSoftStepSolver;

// ── Scene helpers ───────────────────────────────────────────────────────────────

fn dyn_sphere(position: Vec3, lin: Vec3) -> BodyState {
    BodyState {
        inv_inertia: Mat3::from_diagonal(Vec3::new(1.5, 1.2, 1.3)),
        inv_inertia_local: Mat3::from_diagonal(Vec3::new(1.5, 1.2, 1.3)),
        position,
        linear_velocity: lin,
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
    m.points[0] = ContactPoint { anchor_a: anchor, anchor_b: anchor, separation: sep, feature_id: 0 };
    m.count = 1;
    m
}

/// `n` dynamic spheres each contacting ONE shared static floor (the last row).
/// The dynamic rows are pairwise distinct, so the greedy colorer puts all `n`
/// floor contacts in a single color of span `n`. With `n >= 256` that color
/// crosses `MIN_PARALLEL_SLOTS_PER_COLOR`, forcing the `pool.scope` dispatch — the
/// minimal scene that exercises the parallel `ScratchSolveView::row_ptr` path. The
/// shared floor (`inv_mass == 0`) is the pinned-write witness: every group reads it,
/// none writes it.
fn shared_floor_scene(n: u32) -> (Vec<BodyState>, Vec<Manifold>) {
    let mut bodies = Vec::with_capacity(n as usize + 1);
    for i in 0..n {
        bodies.push(dyn_sphere(
            Vec3::new(i as f32 * 3.0, 0.6, 0.0),
            Vec3::new(0.2 * (i as f32 + 1.0), -1.0, 0.1),
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

fn pos_sum(scratch: &SolverScratch) -> f64 {
    scratch
        .bodies()
        .iter()
        .map(|b| b.position.x as f64 + b.position.y as f64 + b.position.z as f64)
        .sum()
}

fn cfg(parallel: bool) -> PhysicsConfig {
    PhysicsConfig {
        dt: 1.0 / 60.0,
        parallel_solve: parallel,
        // Keep the SIMD solve OFF: the AVX2 kernel is `cfg(target_feature="avx2")`
        // and Miri runs the scalar fallback anyway; the row-ptr access path is the
        // same surface in both. The serial scalar `solve_color` over the view is
        // exactly what Miri checks here.
        simd_solve: false,
        ..PhysicsConfig::default()
    }
}

// ── (a) pool-free — the INLINE row-ptr access path under Miri ─────────────────────

/// Pool-free: with no ambient pool, every color dispatches INLINE on the calling
/// thread, so the serial `solve_color` over the `ScratchSolveView` (the shared
/// `body_ref` / `body_mut` / `body_copy` derefs) runs under Miri-TB. A tiny scene
/// keeps it fast.
#[test]
fn miri_inline_rowptr_access_clean() {
    let (bodies, manifolds) = shared_floor_scene(6);
    let graph = build_graph(&bodies, &manifolds);
    let mut solver = ColoredSoftStepSolver::default();
    let mut scratch = SolverScratch::with_capacity(bodies.len());
    scratch.set_bodies(&bodies);
    scratch.touched.reset(scratch.bodies().len());

    for _ in 0..3 {
        scratch.touched.reset(scratch.bodies().len());
        solver.solve_colored(&cfg(false), &manifolds, &graph, &mut scratch);
    }

    // The shared static floor is byte-frozen (the `is_dynamic_row` write guard):
    // its velocity stays zero and its position stays at its spawn point.
    let floor_spawn = Vec3::new(0.0, -1.0, 0.0);
    let floor = scratch.bodies()[6];
    assert_eq!(floor.linear_velocity, Vec3::ZERO, "shared static floor must stay frozen");
    assert_eq!(floor.angular_velocity, Vec3::ZERO, "shared static floor must stay frozen (angular)");
    assert_eq!(floor.position, floor_spawn, "shared static floor position must stay put");
    assert!(pos_sum(&scratch).is_finite(), "the inline colored solve produced finite state");
}

// ── (b) multi-worker — the ScratchSolveView::row_ptr writes under Tree-Borrows ────

/// THE load-bearing Miri-TB gate: a `pool.install`-driven colored solve on a scene
/// whose single color CROSSES `MIN_PARALLEL_SLOTS_PER_COLOR`, so
/// `solve_color_parallel` takes the PARALLEL branch and the per-element
/// `ScratchSolveView::row_ptr` writes (via the `ColorSolvePtrs.bodies` solve view)
/// ACTUALLY execute across worker threads. Miri-TB asserts the concurrent
/// per-element accesses are UB-clean (no data race, no aliasing violation, no OOB) —
/// the deleted whole-`&mut [BodyEffective]` reborrow is gone, so the rigid SP4-class
/// race surface no longer exists.
#[test]
fn miri_multiworker_rowptr_aliasing_clean() {
    use boyko_threadpool::ThreadPoolBuilder;

    // n = 300 disjoint dynamic spheres on one shared floor → a single color of span
    // 300 > 256, so the parallel dispatch fires across 2 workers with a shared
    // pinned (static-floor) witness. 301 bodies — small enough for Miri, large enough
    // to cross the threshold and chunk across the lanes.
    let (bodies, manifolds) = shared_floor_scene(300);
    let graph = build_graph(&bodies, &manifolds);
    let mut solver = ColoredSoftStepSolver::default();
    let mut scratch = SolverScratch::with_capacity(bodies.len());
    scratch.set_bodies(&bodies);
    scratch.touched.reset(scratch.bodies().len());

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    pool.install(|_scope| {
        // A single step drives every parallel unsafe path once under TB.
        scratch.touched.reset(scratch.bodies().len());
        solver.solve_colored(&cfg(true), &manifolds, &graph, &mut scratch);
    });

    // The shared static floor was read by every worker, written by none.
    let floor_spawn = Vec3::new(0.0, -1.0, 0.0);
    let floor = scratch.bodies()[300];
    assert_eq!(floor.linear_velocity, Vec3::ZERO, "shared static floor must stay frozen across workers");
    assert_eq!(floor.angular_velocity, Vec3::ZERO, "shared static floor must stay frozen across workers (angular)");
    assert_eq!(floor.position, floor_spawn, "shared static floor position must stay put");
    assert!(pos_sum(&scratch).is_finite(), "the multi-worker colored solve produced finite state");
}
