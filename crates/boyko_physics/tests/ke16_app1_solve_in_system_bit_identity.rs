//! KE16 App-1 — the colored solve is bit-identical on the route that SHIPS.
//!
//! ## Why this file exists
//!
//! App-1 (`docs/threadpool/KE16-DESIGN-APP.md` §1) changed the lane count at
//! `solver/colored.rs`, `soft/colored.rs` and `resources.rs` from
//! `pool.num_threads() + 1` to `pool.num_threads()`, which changes the number of chunks a
//! dispatched color is cut into (64 / 96 / 96 at W=16 instead of 68 / 102 / 102). The design's
//! justification for making that change without re-blessing any oracle is one sentence: *"Bit-
//! identity is chunk-count- and chunk-shape-independent, so the {1, N} oracles do not move"*.
//!
//! Every existing {1, N} oracle drives the solve **from the test thread** inside
//! `pool.install(...)` — `tests/colored_simd_parallel_o7.rs:227-228`,
//! `tests/colored_rigid_scratch_determinism.rs`, the `--lib` oracles. The test thread is not a
//! registered worker, so `install` labels it `WORKER_ID_DISPATCHER` and every chunk the solver
//! spawns lands in `injector_global`. **That is the healthy dispatch route, and it is not the
//! route physics ships on**: `physics_solve_colored` is a scheduled system, and the parallel
//! scheduler runs concurrent system bodies ON A WORKER, where the chunks go to
//! `injector_local[wid]` and the joining worker drains them into a private scratch deque and
//! runs them inline (KE16 defect A).
//!
//! So the property App-1 leans on — the values do not depend on how the color is cut, or on
//! which lane cuts it — has never been checked on the shipping route. This file checks it, at
//! several worker counts, because the lane count IS the chunk count and W is what App-1 changed
//! it to.
//!
//! ## What it asserts, and what it does NOT
//!
//! It asserts VALUES, never timing or occupancy. A run in which defect A serialises the whole
//! wave onto one worker passes this file, and should: the point is that the value contract holds
//! whatever the placement does. Occupancy is gated by `boyko_ecs/tests/ke16_occupancy_gate.rs`
//! and measured by the KE16 benches.
//!
//! ## The receipt, and why it is a separate test
//!
//! A body that ran on the dispatcher would push its chunks to `injector_global` and re-measure
//! the healthy route under this file's name — `KE16-DESIGN-MEASUREMENT.md` §5 shape 2, "healthy
//! route twice". [`solve_in_scheduled_system_runs_on_a_registered_worker`] is that instrument
//! check, kept apart so a failure says "the harness lost the worker route" rather than "the
//! solver moved".
//!
//! ## Scene
//!
//! `N_FLOOR` dynamic spheres resting on ONE shared static floor. Every constraint pairs a
//! distinct dynamic body with the same STATIC row, and the graph colors by dynamic-body conflict
//! only, so all `N_FLOOR` constraints land in a single color of `N_FLOOR` width-1 groups —
//! `N_FLOOR` slots, far above the solver's `MIN_PARALLEL_SLOTS_PER_COLOR` inline threshold, so
//! the `pool.scope` dispatch is genuinely taken. [`floor_scene_color_exceeds_the_parallel_
//! dispatch_threshold`] is the anti-vacuity witness for that claim.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_physics::components::ColliderShape;
use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};
use boyko_physics::solver::ColoredSoftStepSolver;
use boyko_threadpool::{MAX_WORKERS, ThreadPoolBuilder, current_worker_id};

/// The colored solver's inline-vs-dispatch threshold (a private const in the crate; mirrored
/// here for the anti-vacuity witness only). Keep in sync with
/// `solver/colored.rs::MIN_PARALLEL_SLOTS_PER_COLOR`.
const MIN_PARALLEL_SLOTS_PER_COLOR: usize = 256;

/// Dynamic spheres on the shared floor. 300 > 256 slots, so the single color crosses the
/// dispatch threshold with margin even if the constant is nudged.
const N_FLOOR: u32 = 300;

/// Solve steps per run. Four is enough for the bodies to leave rest (the anti-vacuity check
/// below compares against the unsolved snapshot) and keeps a nine-configuration file fast.
const STEPS: usize = 4;

// ── Scene ────────────────────────────────────────────────────────────────────

fn dyn_sphere(position: Vec3, lin: Vec3, ang: Vec3) -> BodyState {
    BodyState {
        // A non-trivial diagonal inertia so the angular term is not vacuous.
        inv_inertia: Mat3::from_diagonal(Vec3::new(1.5, 1.2, 1.3)),
        inv_inertia_local: Mat3::from_diagonal(Vec3::new(1.5, 1.2, 1.3)),
        position,
        linear_velocity: lin,
        angular_velocity: ang,
        rotation: Quat::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.0,
        friction: 0.7,
        simulated: true,
        kinematic: false,
        is_sensor: false,
        shape: ColliderShape::Sphere { radius: 1.0 },
    }
}

fn static_floor() -> BodyState {
    BodyState {
        inv_inertia: Mat3::ZERO,
        inv_inertia_local: Mat3::ZERO,
        position: Vec3::new(0.0, -1.0, 0.0),
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

/// `N_FLOOR` dynamic spheres (rows `0..N_FLOOR`) plus the shared static floor (last row).
fn floor_scene() -> Vec<BodyState> {
    let mut bodies = Vec::with_capacity(N_FLOOR as usize + 1);
    for i in 0..N_FLOOR {
        bodies.push(dyn_sphere(
            Vec3::new(i as f32 * 3.0, 0.6, 0.0),
            Vec3::new(0.2 * (i as f32 + 1.0), -1.0, 0.1),
            Vec3::new(0.05, -0.1, 0.2),
        ));
    }
    bodies.push(static_floor());
    bodies
}

/// One width-1 manifold per dynamic sphere against the shared floor row. Rebuilt from the
/// CURRENT body positions each step, exactly as the existing O7 oracle does, so the sequence is
/// a function of the body state alone.
fn floor_manifolds(bodies: &[BodyState]) -> Vec<Manifold> {
    let floor_row = (bodies.len() - 1) as u32;
    let mut out = Vec::with_capacity(N_FLOOR as usize);
    for i in 0..N_FLOOR {
        let mut m = Manifold::new(BodyIndex(i), BodyIndex(floor_row));
        m.normal = Vec3::new(0.0, -1.0, 0.0);
        m.points[0] = ContactPoint {
            anchor_a: bodies[i as usize].position,
            anchor_b: bodies[i as usize].position,
            separation: -0.2,
            feature_id: 0,
        };
        m.count = 1;
        out.push(m);
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

/// Bit pattern of every body's integrated state — the comparison currency of the {1, N} oracles
/// (`tests/colored_simd_parallel_o7.rs:182-200`). `to_bits` rather than `==` so a NaN or a
/// negative zero is a difference, not a coincidence.
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

/// `STEPS` colored solve steps over a fresh copy of the scene, with `parallel_solve` as given.
/// Runs on the CALLING thread; used for the serial oracle, which needs no pool at all.
fn solve_here(parallel_solve: bool) -> Vec<u32> {
    let cfg = PhysicsConfig { dt: 1.0 / 60.0, parallel_solve, ..PhysicsConfig::default() };
    let mut solver = ColoredSoftStepSolver::default();
    let bodies = floor_scene();
    let mut scratch = SolverScratch::with_capacity(bodies.len());
    scratch.set_bodies(&bodies);
    for _ in 0..STEPS {
        let manifolds = floor_manifolds(scratch.bodies());
        let graph = build_graph(scratch.bodies(), &manifolds);
        scratch.touched.reset(scratch.bodies().len());
        solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
    }
    snapshot_bits(&scratch)
}

/// The same `STEPS` steps with `parallel_solve = true`, run inside a CONCURRENT scheduled system
/// on a `workers`-wide pool — the route physics ships on.
///
/// Everything the solve needs is built inside the system body from the captured immutable scene,
/// so the body owns its state outright: no leak, no raw pointer, no `unsafe`. `Schedule::run`
/// joins every dispatched system before it returns, so the `OnceLock` is filled by the time this
/// function reads it.
///
/// Returns the body snapshot and the worker id the body ran on (the route receipt).
fn solve_in_scheduled_system(workers: usize) -> (Vec<u32>, u32) {
    let out: Arc<OnceLock<Vec<u32>>> = Arc::new(OnceLock::new());
    let route = Arc::new(AtomicU32::new(u32::MAX));

    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));

    let out_in = Arc::clone(&out);
    let route_in = Arc::clone(&route);
    builder.add_system(move || {
        // Taken BEFORE the solve so the route is recorded even if the solve panics.
        route_in.store(current_worker_id(), Ordering::SeqCst);

        let cfg =
            PhysicsConfig { dt: 1.0 / 60.0, parallel_solve: true, ..PhysicsConfig::default() };
        let mut solver = ColoredSoftStepSolver::default();
        let bodies = floor_scene();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.set_bodies(&bodies);
        for _ in 0..STEPS {
            let manifolds = floor_manifolds(scratch.bodies());
            let graph = build_graph(scratch.bodies(), &manifolds);
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
        }
        let _ = out_in.set(snapshot_bits(&scratch));
    });
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let bits = out
        .get()
        .cloned()
        .expect("invariant: Schedule::run joins its systems, so the system body has completed");
    (bits, route.load(Ordering::SeqCst))
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Anti-vacuity for the whole file: the scene's single color must be wide enough that the solver
/// DISPATCHES it instead of solving it inline. If it were below the threshold every "parallel"
/// arm below would silently be the inline scalar path and would agree with the oracle for the
/// wrong reason.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds a 301-body constraint graph; the property is architecture-independent \
              and is covered natively"
)]
fn floor_scene_color_exceeds_the_parallel_dispatch_threshold() {
    let bodies = floor_scene();
    let manifolds = floor_manifolds(&bodies);
    let graph = build_graph(&bodies, &manifolds);

    // The solver's threshold is measured in SLOTS (contact points), not in groups
    // (`solver/colored.rs:2598-2599`: `color_slots = span.1 - span.0`). Every manifold in this
    // scene carries exactly one point, so the color's group count IS its slot count — asserted
    // rather than assumed, because the equality is what makes the count below comparable to the
    // threshold.
    assert!(
        manifolds.iter().all(|m| m.count == 1),
        "every floor manifold must be width-1 for the group count to equal the slot count the \
         solver's dispatch threshold is measured in"
    );

    let widest = (0..graph.n_colors()).map(|c| graph.color(c).len()).max().unwrap_or(0);
    println!(
        "[ke16 app-1] colors={} widest_color_groups={widest} (= slots, width-1 manifolds) \
         threshold={MIN_PARALLEL_SLOTS_PER_COLOR}",
        graph.n_colors()
    );
    assert!(
        widest > MIN_PARALLEL_SLOTS_PER_COLOR,
        "the floor scene's widest color holds {widest} slots, which is not above the solver's \
         inline threshold ({MIN_PARALLEL_SLOTS_PER_COLOR}): every parallel arm in this file \
         would take the inline path and agree with the serial oracle without ever dispatching a \
         chunk"
    );
}

/// The route receipt, on its own: the solve system must run on a REGISTERED WORKER.
///
/// If it ran on the dispatcher (`WORKER_ID_DISPATCHER = u32::MAX - 1`) or unattached
/// (`WORKER_ID_UNATTACHED = u32::MAX`), the chunks would reach `injector_global` and the two
/// bit-identity tests below would be re-measuring the dispatcher route the existing oracles
/// already cover (`KE16-DESIGN-MEASUREMENT.md` §5 shape 2).
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: spins a real OS thread pool and runs the parallel scheduler"
)]
fn solve_in_scheduled_system_runs_on_a_registered_worker() {
    let (_bits, route) = solve_in_scheduled_system(4);

    assert!(
        (route as usize) < MAX_WORKERS,
        "the solve system recorded worker id {route} (>= MAX_WORKERS = {MAX_WORKERS}): it ran \
         off-pool, so this file is exercising the dispatcher route, not the scheduled route it \
         exists to cover"
    );
}

/// The App-1 property at the default-shaped pool: the dispatched, worker-routed solve produces
/// the same bits as the single-threaded solve.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: spins a real OS thread pool and runs the parallel scheduler"
)]
fn solve_in_scheduled_system_is_bit_identical_to_the_serial_oracle() {
    let serial = solve_here(false);
    let (in_system, _route) = solve_in_scheduled_system(4);

    assert_eq!(
        serial, in_system,
        "the colored solve dispatched from inside a scheduled system on a 4-worker pool did not \
         reproduce the single-threaded bits. App-1 changed the chunk count at this site \
         (`lanes = num_threads()`, no longer `+ 1`), and the design permits that ONLY because \
         the values are chunk-count- and chunk-shape-independent"
    );
}

/// The same property across worker counts. This is the App-1-sensitive sweep: `lanes` IS the
/// worker count, so each arm cuts the color into a DIFFERENT number of chunks
/// (`lanes * CHUNKS_PER_WORKER`, clamped to the group count) and every arm must still land on
/// the same bits.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: spins four real OS thread pools and runs the parallel scheduler on each"
)]
fn solve_in_scheduled_system_is_bit_identical_at_1_2_4_and_8_workers() {
    let serial = solve_here(false);

    for workers in [1usize, 2, 4, 8] {
        let (bits, route) = solve_in_scheduled_system(workers);
        assert!(
            (route as usize) < MAX_WORKERS,
            "at {workers} workers the solve system ran off-pool (recorded id {route}); the \
             reading below would be the dispatcher route under this test's name"
        );
        assert_eq!(
            serial, bits,
            "the scheduled-system solve on a {workers}-worker pool did not reproduce the \
             single-threaded bits: the result depends on the chunk count, which App-1 changes"
        );
    }
}

/// The scene must actually solve. A scene that never moves would make every equality above hold
/// on the untouched initial state.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs four colored solve steps over a 301-body scene"
)]
fn floor_scene_non_vacuously_solves() {
    let bodies = floor_scene();
    let mut rest = SolverScratch::with_capacity(bodies.len());
    rest.set_bodies(&bodies);

    let solved = solve_here(false);

    assert_ne!(
        snapshot_bits(&rest),
        solved,
        "the floor scene's bodies are bit-identical before and after {STEPS} solve steps, so \
         every bit-identity assertion in this file would hold over the unsolved state"
    );
}
