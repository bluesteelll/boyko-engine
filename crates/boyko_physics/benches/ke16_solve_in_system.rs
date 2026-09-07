//! KE16 — the PHYSICS-level measurement harness for threadpool defect A.
//!
//! ## What this bench exists to measure, and why the existing ones cannot
//!
//! `benches/parallel_solve.rs` (O6 Gate 10) and `benches/colored_solve.rs` (O5
//! Gate 9) both drive `ColoredSoftStepSolver::solve_colored` **from the bench
//! thread** — `colored_solve.rs` with no pool at all, `parallel_solve.rs` from
//! inside `pool.install(|_scope| ...)` on the bench thread itself
//! (`parallel_solve.rs:198-199`). The bench thread is not a registered worker,
//! so `install` labels it `WORKER_ID_DISPATCHER` and every task the solver's
//! `pool.scope` pushes lands in `injector_global`, which every worker polls.
//! **Both existing benches therefore measure the HEALTHY dispatch path only.**
//!
//! In production the same call runs from `physics_solve_colored`, registered
//! with `builder.add_system(...)` (`plugin.rs`), and the parallel scheduler
//! runs every concurrent system body ON A WORKER. From a worker, `push_task`
//! routes to `injector_local[wid]`, which no sibling ever polls, and the
//! joining worker drains it into a private unregistered `scratch` deque and
//! runs it inline. **The shipping configuration is the path no bench covers.**
//!
//! This bench runs the IDENTICAL warmed solve over the IDENTICAL scene down
//! both routes and reports the ratio:
//!
//!   * `bench_thread_install` — `pool.install` from the bench thread (what O6
//!     measures). W workers PLUS the bench thread, which work-steals while the
//!     scope is open: a W+1 lane route.
//!   * `bench_thread_install_Wminus1` — the same shape over a `num_threads(W-1)`
//!     pool, so W-1 workers plus the helping bench thread is exactly W lanes.
//!     This is the reference the acceptance line uses when the shipped joiner
//!     policy keeps the external helper, because comparing the scheduled route
//!     against a W+1 lane route would grant it a structural allowance of up to
//!     (W+1)/W — 6.25 % at W=16, the width of the effect under study
//!     (`KE16-DESIGN-APP.md` §11).
//!   * `in_scheduled_system` — one concurrent system, `Schedule::run` (what
//!     ships);
//!   * `empty_schedule_control` — a schedule holding one empty system, so the
//!     scheduler's own per-run cost is subtracted rather than attributed to the
//!     solver;
//!   * `single_threaded_O5` — no pool at all: the bar both parallel routes must
//!     beat to have bought anything.
//!
//! ## Occupancy is NOT instrumented here
//!
//! The ECS-level harness (`boyko_ecs/benches/ke16_par_iter_in_system.rs`)
//! carries a max-in-flight counter because the body is the bench's own. Here
//! the body is `solve_colored`'s per-color inner loop, inside
//! `boyko_physics/src/`, and this harness may not modify `src/`. So the physics
//! consumer is measured on wall-clock alone; the occupancy evidence lives in
//! the ECS harness, where the same pool mechanism is exercised.
//!
//! The scene, and the `dyn_sphere` / `static_floor` / `manifold` /
//! `pyramid_scene` / `build_graph` / `widest_color` helpers, are lifted
//! verbatim from `benches/parallel_solve.rs` so the two benches are comparable
//! row for row; a shared bench module would be preferable but benches are
//! separate crates and this harness may not add a `src/` module to do it.

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::thread::available_parallelism;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass};
use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};
use boyko_physics::solver::ColoredSoftStepSolver;
use boyko_threadpool::{
    MAX_WORKERS, ThreadPool, ThreadPoolBuilder, current_worker_id, ke16_check_expected_variant,
};

/// The colored solver's W1 inline-vs-dispatch threshold (a private const in the
/// crate; mirrored here only for the anti-vacuity witness). Keep in sync with
/// `colored.rs::MIN_PARALLEL_SLOTS_PER_COLOR`.
const MIN_PARALLEL_SLOTS_PER_COLOR: u32 = 256;

/// Highest worker id the scheduled solve system ever ran on — half of the route receipt.
///
/// A `fetch_max` rather than a store: what matters is not WHICH worker took the body but whether
/// any run of it was off-pool. The two sentinels (`WORKER_ID_DISPATCHER = u32::MAX - 1`,
/// `WORKER_ID_UNATTACHED = u32::MAX`) are the largest `u32`s, so a max is exactly that test.
/// A body that ran on the dispatcher would push the solver's chunks to `injector_global` and
/// measure the HEALTHY route under the shipping route's name
/// (`KE16-DESIGN-MEASUREMENT.md` §5 shape 2).
static SYSTEM_WORKER_ID: AtomicU32 = AtomicU32::new(0);

/// Runs of the scheduled solve system — the OTHER half of the route receipt.
///
/// Worker 0 is a legal id, so the id alone cannot separate "ran on worker 0" from "never ran".
/// It does NOT detect a filtered-out row: the receipt is evaluated inside the row's own routine,
/// and criterion never invokes a routine whose id the filter rejected (nor under `--list`), so a
/// filtered row yields no reading and takes no receipt. What the count separates is the shape
/// where the row WAS measured and the solve system still never ran — a schedule built without
/// the system, or a `run_if` that skipped it — all of which leave `SYSTEM_WORKER_ID` at the
/// legal id 0. The "never ran" sentinel cannot live in `SYSTEM_WORKER_ID` itself, because every
/// value outside `[0, MAX_WORKERS)` is ABOVE the legal ids and would survive the `fetch_max`
/// fold the off-pool half needs, so existence is counted apart.
static SYSTEM_BODY_RUNS: AtomicUsize = AtomicUsize::new(0);

/// Panics unless the scheduled system ran at all, and unless every recorded run of it was on a
/// registered worker.
fn assert_system_route_receipt() {
    let runs = SYSTEM_BODY_RUNS.load(Ordering::SeqCst);
    assert!(
        runs > 0,
        "the `in_scheduled_system` route was measured but recorded NO run of the solve system: \
         the schedule never dispatched it, so the number this row produced is not a measurement \
         of this route"
    );
    let observed = SYSTEM_WORKER_ID.load(Ordering::SeqCst);
    assert!(
        (observed as usize) < MAX_WORKERS,
        "the `in_scheduled_system` route recorded worker id {observed} over {runs} runs (>= \
         MAX_WORKERS = {MAX_WORKERS}): the solve system ran off-pool, so its `pool.scope` \
         reached injector_global and this row is the bench-thread route measured twice"
    );
}

// ── Scene (verbatim from benches/parallel_solve.rs) ──────────────────────────

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
    let collider = Collider { shape: ColliderShape::Sphere { radius: 0.5 }, layer: 1, mask: 1 };
    BodyState::from_columns(&body, &mass, &collider, false, true, false)
}

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
    let collider = Collider { shape: ColliderShape::Sphere { radius: 50.0 }, layer: 1, mask: 1 };
    BodyState::from_columns(&body, &mass, &collider, false, false, false)
}

fn manifold(a: u32, b: u32, normal: Vec3, anchor: Vec3) -> Manifold {
    let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
    m.normal = normal;
    m.points[0] = ContactPoint { anchor_a: anchor, anchor_b: anchor, separation: -0.05, feature_id: 0 };
    m.count = 1;
    m
}

/// A big single-ISLAND pyramid: `base` rows wide at the bottom, narrowing by
/// one per level. `base = 141` yields 141·142/2 = 10011 dynamic bodies — the
/// O6 plan's ~10k pyramid, whose widest color is far above the W1 threshold, so
/// a real `pool.scope` dispatch occurs.
fn pyramid_scene(base: u32) -> (Vec<BodyState>, Vec<Manifold>) {
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
        let y = 0.5 + r as f32 * 0.86;
        for i in 0..count {
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
                manifolds.push(manifold(
                    me,
                    floor_row,
                    Vec3::new(0.0, -1.0, 0.0),
                    bodies[me as usize].position,
                ));
            }
            if i + 1 < count {
                manifolds.push(manifold(
                    me,
                    id(r, i + 1),
                    Vec3::new(1.0, 0.0, 0.0),
                    bodies[me as usize].position,
                ));
            }
            if r + 1 < base {
                let above_count = base - (r + 1);
                if i < above_count {
                    let up = id(r + 1, i);
                    manifolds.push(manifold(
                        me,
                        up,
                        Vec3::new(0.0, 1.0, 0.0),
                        bodies[me as usize].position,
                    ));
                }
                if i >= 1 && (i - 1) < above_count {
                    let up = id(r + 1, i - 1);
                    manifolds.push(manifold(
                        me,
                        up,
                        Vec3::new(0.0, 1.0, 0.0),
                        bodies[me as usize].position,
                    ));
                }
            }
        }
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

fn widest_color(graph: &ConstraintGraph) -> u32 {
    (0..graph.n_colors()).map(|c| graph.color(c).len() as u32).max().unwrap_or(0)
}

// ── The state one solve step needs, in one owned box ─────────────────────────

/// Everything `solve_colored` mutates or reads, owned together so it can be
/// handed to a scheduled system as a single capture.
struct SolveState {
    cfg: PhysicsConfig,
    manifolds: Vec<Manifold>,
    graph: ConstraintGraph,
    solver: ColoredSoftStepSolver,
    scratch: SolverScratch,
}

impl SolveState {
    fn new(bodies: &[BodyState], manifolds: Vec<Manifold>, parallel: bool) -> Self {
        let graph = build_graph(bodies, &manifolds);
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.set_bodies(bodies);
        scratch.touched.reset(bodies.len());
        Self {
            cfg: PhysicsConfig { dt: 1.0 / 60.0, parallel_solve: parallel, ..PhysicsConfig::default() },
            manifolds,
            graph,
            solver: ColoredSoftStepSolver::default(),
            scratch,
        }
    }

    /// One full colored solve step, identical on both routes.
    #[inline(never)]
    fn step(&mut self) {
        let n = self.scratch.bodies().len();
        self.scratch.touched.reset(n);
        self.solver.solve_colored(
            black_box(&self.cfg),
            black_box(&self.manifolds),
            black_box(&self.graph),
            &mut self.scratch,
        );
    }
}

/// A `'static` handle to a leaked [`SolveState`], so one scheduled system can
/// own it for the whole bench.
///
/// The scheduler requires a system body to be `Send + Sync + 'static`. The
/// state is leaked (the bench process owns it until exit) and reached through a
/// raw pointer, because `SolverScratch` is not required to be `Sync` and the
/// bound would otherwise be unsatisfiable for a plain `&'static mut`.
struct StateHandle(*mut SolveState);

// SAFETY: the pointee is a `Box::leak`ed `SolveState`, so it is live for the
//   remainder of the process and is never freed or moved. Exactly ONE
//   `StateHandle` is ever made per `SolveState` (constructed in
//   `spawn_solve_schedule` from a fresh `Box`), and it is moved into exactly
//   one system closure. A `Schedule` runs each of its systems at most once per
//   `run`, and `Schedule::run` joins every dispatched system before returning,
//   so the only `&mut SolveState` in existence at any instant is the one the
//   system body creates, and it is dropped before `run` returns. The bench
//   thread does not touch the state between runs. Therefore no aliasing `&mut`
//   and no cross-thread data race can be formed through this handle.
unsafe impl Send for StateHandle {}
// SAFETY: as above — `&StateHandle` exposes no operation at all (the field is
//   private and only the owning closure dereferences it), so sharing the
//   handle across threads cannot produce a race; the mutation discipline is
//   the one argued for `Send`.
unsafe impl Sync for StateHandle {}

/// Builds a world + a one-system schedule whose single concurrent system
/// performs `state.step()` on a worker thread.
fn spawn_solve_schedule(
    pool: &Arc<ThreadPool>,
    state: Box<SolveState>,
) -> (EcsMaster, Schedule) {
    let handle = StateHandle(Box::leak(state) as *mut SolveState);
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(Arc::clone(pool));
    builder.add_system(move || {
        // The route receipt, taken BEFORE the solve so it is recorded even if the solve panics.
        SYSTEM_WORKER_ID.fetch_max(current_worker_id(), Ordering::AcqRel);
        SYSTEM_BODY_RUNS.fetch_add(1, Ordering::AcqRel);
        // Edition-2024 closures capture the narrowest PLACE they use, so naming
        // `handle.0` directly would capture the bare `*mut SolveState` field —
        // which is neither `Send` nor `Sync`, and the `unsafe impl`s on the
        // wrapper would never be consulted. Binding the whole struct first is
        // what makes the capture the wrapper.
        let handle = &handle;
        // SAFETY: see the `unsafe impl Send for StateHandle` block — the
        //   schedule runs this system body once per `run` and joins it before
        //   returning, so this is the only live reference to the pointee.
        let state: &mut SolveState = unsafe { &mut *handle.0 };
        state.step();
    });
    let schedule = builder.build(&mut world);
    (world, schedule)
}

/// A world + schedule holding one EMPTY concurrent system — the control that
/// prices `Schedule::run`'s own dispatch so it is not charged to the solver.
fn spawn_empty_schedule(pool: &Arc<ThreadPool>) -> (EcsMaster, Schedule) {
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(Arc::clone(pool));
    builder.add_system(|| {});
    let schedule = builder.build(&mut world);
    (world, schedule)
}

// ── Benches ─────────────────────────────────────────────────────────────────

fn bench_ke16_solve_routes(c: &mut Criterion) {
    // The banner and the `KE16_EXPECT` check are TWO obligations (`KE16-DESIGN.md` §4): the check
    // is SILENT when the variable is unset, so without the banner the rows below would carry no
    // record of which build produced them, and the acceptance line this bench feeds would be
    // unattributable. It is printed HERE and not inside `ke16_check_expected_variant` because a
    // print from `crates/*/src/**.rs` reds `boyko-log`'s print census; the spelling matches the
    // pool crate's KE16 harness files, so one `grep "KE16 variant"` collects every row.
    println!("KE16 variant: {}", boyko_threadpool::ke16_variant());
    ke16_check_expected_variant();
    let workers = available_parallelism().map(|n| n.get()).unwrap_or(4);
    // The `bench_thread_install_Wminus1` reference pool. `ThreadPoolBuilder` clamps its argument
    // to `[1, MAX_WORKERS]`, so `max(1)` only spells out what the builder would do anyway on a
    // one-core box — where the row degenerates to the W-pool and its reading is not usable.
    let workers_minus_one = workers.saturating_sub(1).max(1);

    let (bodies, manifolds) = pyramid_scene(141);
    let n_contacts = manifolds.len();

    // Anti-vacuity, the same three witnesses `parallel_solve.rs` asserts: real
    // contacts, a real multi-color partition, and a color wide enough that the
    // solver's W1 gate takes the DISPATCH arm rather than solving inline. If
    // any fails, both routes would be measuring an inline solve and the ratio
    // would say nothing about the pool at all.
    let probe_graph = build_graph(&bodies, &manifolds);
    assert!(n_contacts > 0, "scene has no contacts");
    assert!(probe_graph.n_colors() > 1, "scene partitions into a single color");
    let widest = widest_color(&probe_graph);
    assert!(
        widest > MIN_PARALLEL_SLOTS_PER_COLOR,
        "widest color is {widest} slots, at or below the W1 threshold \
         ({MIN_PARALLEL_SLOTS_PER_COLOR}) — every color would solve inline and no `pool.scope` \
         dispatch would occur"
    );

    println!(
        "\n=== KE16 physics consumer (release={}) ===\n\
         workers={workers}  workers_minus_one={workers_minus_one}  available_parallelism={}\n\
         pyramid base=141: {} bodies, {n_contacts} contacts, {} colors, widest color {widest} slots\n",
        !cfg!(debug_assertions),
        available_parallelism().map(|n| n.get()).unwrap_or(0),
        bodies.len(),
        probe_graph.n_colors(),
    );

    let mut group = c.benchmark_group("ke16_solve_route");
    group.sample_size(20); // a 10k-body step is heavy; keep the wall-clock sane
    group.warm_up_time(Duration::from_secs(1));
    group.throughput(Throughput::Elements(n_contacts as u64));

    // ── Route 1: the O6 shape — `pool.install` on the BENCH THREAD (healthy).
    group.bench_with_input(
        BenchmarkId::new("bench_thread_install", n_contacts),
        &n_contacts,
        |b, _| {
            let pool = ThreadPoolBuilder::new().num_threads(workers).build();
            let mut state = SolveState::new(&bodies, manifolds.clone(), true);
            pool.install(|_scope| {
                for _ in 0..4 {
                    state.step();
                }
                b.iter(|| state.step());
            });
        },
    );

    // ── Reference: the W-LANE bench-thread route (App-10). `bench_thread_install` above runs
    //    W workers PLUS the bench thread, which helps while the scope is open, so it is a W+1
    //    lane route and beats a W-lane scheduled route structurally — by up to (W+1)/W, 6.25 % at
    //    W=16, which is the width of the effect under study. This row removes that allowance:
    //    W-1 workers plus the helping external joiner is W lanes, so the acceptance line compares
    //    W lanes against W lanes. (Which of the two rows IS the W-lane reference depends on the
    //    shipped joiner policy: if the external joiner parks instead of helping, this row is W-1
    //    lanes and `bench_thread_install` is the W-lane one — `KE16-DESIGN-APP.md` §11. The
    //    ratio between the two rows is itself the receipt for which of the two it was.)
    group.bench_with_input(
        BenchmarkId::new("bench_thread_install_Wminus1", n_contacts),
        &n_contacts,
        |b, _| {
            let pool = ThreadPoolBuilder::new().num_threads(workers_minus_one).build();
            let mut state = SolveState::new(&bodies, manifolds.clone(), true);
            pool.install(|_scope| {
                for _ in 0..4 {
                    state.step();
                }
                b.iter(|| state.step());
            });
        },
    );

    // ── Route 2: the SHIPPING shape — inside a scheduled system (defective).
    group.bench_with_input(
        BenchmarkId::new("in_scheduled_system", n_contacts),
        &n_contacts,
        |b, _| {
            let pool = ThreadPoolBuilder::new().num_threads(workers).build();
            let state = Box::new(SolveState::new(&bodies, manifolds.clone(), true));
            let (mut world, mut schedule) = spawn_solve_schedule(&pool, state);
            for _ in 0..4 {
                schedule.run(&mut world);
            }
            b.iter(|| schedule.run(black_box(&mut world)));
            // Once per `b.iter` batch, INSIDE the routine: the receipt is a property of the
            // route, reading an atomic inside the timed loop would price the instrument into
            // the number, and a row criterion filtered out is never invoked — so a row with no
            // reading takes no receipt.
            assert_system_route_receipt();
        },
    );

    // ── Control: the scheduler's own per-run cost, so route 2's overhead is
    //    priced rather than attributed to the solver.
    group.bench_with_input(
        BenchmarkId::new("empty_schedule_control", n_contacts),
        &n_contacts,
        |b, _| {
            let pool = ThreadPoolBuilder::new().num_threads(workers).build();
            let (mut world, mut schedule) = spawn_empty_schedule(&pool);
            for _ in 0..4 {
                schedule.run(&mut world);
            }
            b.iter(|| schedule.run(black_box(&mut world)));
        },
    );

    // ── Reference: the single-threaded O5 colored solve, no pool at all. The
    //    bar both parallel routes must beat to have bought anything.
    group.bench_with_input(
        BenchmarkId::new("single_threaded_O5", n_contacts),
        &n_contacts,
        |b, _| {
            let mut state = SolveState::new(&bodies, manifolds.clone(), false);
            for _ in 0..4 {
                state.step();
            }
            b.iter(|| state.step());
        },
    );

    group.finish();
}

criterion_group!(benches, bench_ke16_solve_routes);
criterion_main!(benches);
