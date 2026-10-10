//! KE16 App-1 — the SOFT-BODY colored solve, dispatched from a REGISTERED WORKER.
//!
//! ## The gap this file closes
//!
//! App-1 (`docs/threadpool/KE16-DESIGN-APP.md` §1) rewrote the lane count at three physics sites
//! to `lanes = pool.num_threads()`, dropping the `+ 1` that counted the caller as an extra lane.
//! Its whole rationale is about WHO calls `pool.scope`: on the production route the caller is one
//! of the W workers, so the extra lane never existed; the `+ 1` described the bench route, where
//! an unregistered thread joins and helps.
//!
//! Two of the three sites are covered on that production-shaped route:
//! `solver/colored.rs` by `ke16_app1_solve_in_system_bit_identity.rs` (a scheduled system) and
//! `resources.rs` by `broadphase_grid.rs`'s production-world test. The THIRD —
//! `soft/colored.rs`'s `lanes = pool.num_threads()` — is exercised only from the test thread:
//! `soft_colored_sp4.rs`'s `{1, N}` oracle runs its steps inside `pool.install`, so the caller of
//! `pool.scope` is the EXTERNAL joiner, the one caller shape whose lane arithmetic App-1 says is
//! measured rather than counted. That is the route App-1 did NOT change and the one this file
//! does not re-measure.
//!
//! ## The route, and why it is `pool.spawn` rather than a scheduled system
//!
//! The property under test is "the colored soft solve dispatches correctly when
//! `try_with_active_pool` resolves on a registered worker". `pool.spawn` puts the whole step on a
//! worker, whose `worker_main` sets `ACTIVE_POOL` (`boyko_threadpool`'s `worker.rs`), so the
//! solve's `try_with_active_pool` sees the pool exactly as it does under `Schedule::run` — and
//! the join is the task's own, with no external joiner anywhere in the picture (the shape
//! `KE16-DESIGN.md` §8 uses for its own worker-route Miri gates).
//!
//! It is deliberately NOT a scheduled system: the step needs a WORLD, and building one means
//! creating entities, which routes through `EcsMaster::spawn_one` -> `drain_deferred_hook_queue`,
//! whose SAFETY-7 tripwire asserts `!is_in_system_run()`. Building a fixture world inside a
//! system body would fire that assert — legitimately, since hooks must not drain inside the
//! allocation-discipline window — and the failure would say nothing about App-1. The alternative,
//! handing a pre-built world to the body through a raw pointer, buys the guard bracket at the
//! price of an `unsafe impl Send` this file does not need.
//!
//! ## What is asserted
//!
//! 1. [`soft_colored_from_a_worker_runs_on_a_registered_worker_and_dispatches`] — the two
//!    anti-vacuity conditions, on their own, so a failure of the oracle below cannot be read as a
//!    fixture problem: the step ran on an id `< MAX_WORKERS` (not the dispatcher, not unattached)
//!    and at least one color crossed `MIN_PARALLEL_SLOTS_PER_COLOR` and dispatched a `pool.scope`.
//! 2. [`soft_colored_from_a_worker_is_bit_identical_at_one_and_four_workers`] — the `{1, N}`
//!    oracle on that route: the position bits are identical at W=1, at W=4 and against the
//!    pool-free inline fallback. The lane count sizes the chunk count and nothing else, so a
//!    chunk-count change is a pure perf knob; if `lanes` ever became a per-lane INDEX, W=1 with a
//!    helping caller is where it would read out of bounds.
//!
//! Both tests hold in the default build and under every KE16 candidate: they gate the VALUE the
//! route produces, not how many lanes it reaches (that is the occupancy harness's job).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::yield_now;
use std::time::{Duration, Instant};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::into_system::IntoSystem;
use boyko_threadpool::{MAX_WORKERS, ThreadPoolBuilder, current_worker_id};

use boyko_physics::math::Vec3;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::sdf_query::SdfField;
use boyko_physics::soft::{SoftBody, SoftColorScratch, physics_soft_step_colored};

/// Cloth width. A 40x40 grid is ~3120 structural edges; a regular grid 4-colors them, so the
/// widest distance color holds ~780 slots — well above `MIN_PARALLEL_SLOTS_PER_COLOR` (256), so
/// the parallel dispatch fires instead of the inline fallback. Same width as
/// `soft_colored_sp4.rs`'s oracle, so the two files' bits are directly comparable.
const CLOTH_W: usize = 40;

/// Steps per run. Enough for the cloth to move (the snapshot is non-trivial) and for a chunking
/// difference to diverge if the solve were chunk-count-dependent.
const STEPS: usize = 4;

/// Upper bound on how long the worker task may take before this file calls it a hang rather than
/// a slow box. Four steps of a 40x40 cloth are milliseconds; the margin is for a loaded machine.
const TASK_DEADLINE: Duration = Duration::from_secs(60);

// ── Fixture (the shapes of `soft_colored_sp4.rs`, kept identical on purpose) ──────────────────

/// A `w x w` particle grid (a cloth) with structural distance edges (right + down neighbours) and
/// the top row pinned (`inv_mass == 0`).
fn grid_cloth(w: usize) -> SoftBody {
    let mut positions = Vec::with_capacity(w * w);
    let mut inv_masses = Vec::with_capacity(w * w);
    for y in 0..w {
        for x in 0..w {
            positions.push([x as f32 * 0.1, 2.0 - y as f32 * 0.1, 0.0]);
            inv_masses.push(if y == 0 { 0.0 } else { 1.0 });
        }
    }
    let idx = |x: usize, y: usize| (y * w + x) as u32;
    let mut edges = Vec::new();
    for y in 0..w {
        for x in 0..w {
            if x + 1 < w {
                edges.push((idx(x, y), idx(x + 1, y)));
            }
            if y + 1 < w {
                edges.push((idx(x, y), idx(x, y + 1)));
            }
        }
    }
    SoftBody::from_mesh(&positions, &inv_masses, &edges, None, 1.0e-7, 0.0)
        .expect("invariant: the grid cloth is well-formed")
}

/// Gravity ON (so the cloth moves) and the colored soft path selected.
fn install(world: &mut EcsMaster) {
    world.insert_resource(PhysicsConfig {
        dt: 1.0 / 60.0,
        substeps: 2,
        gravity: Vec3::new(0.0, -9.81, 0.0),
        soft_body: true,
        soft_body_colored: true,
        ..PhysicsConfig::default()
    });
    world.insert_resource(SdfField::default());
    world.insert_resource(SoftColorScratch::default());
}

fn spawn_soft(world: &mut EcsMaster, body: SoftBody) {
    let arch = world.create_archetype(&[SoftBody::component_id()]);
    world.spawn_one(arch, body).expect("invariant: {SoftBody} archetype accepts a SoftBody");
}

fn pos_bits(world: &mut EcsMaster) -> Vec<(u32, u32, u32)> {
    let q = world.query::<&SoftBody, ()>();
    let mut it = q.iter();
    let body = it.next().expect("invariant: one soft body was spawned");
    (0..body.pos_x.len())
        .map(|i| (body.pos_x[i].to_bits(), body.pos_y[i].to_bits(), body.pos_z[i].to_bits()))
        .collect()
}

/// Builds a fresh cloth world and runs `STEPS` colored soft steps on the CALLING thread.
/// Returns the position bits and the parallel-dispatch count of the last step.
fn run_colored_here() -> (Vec<(u32, u32, u32)>, usize) {
    let mut world = EcsMaster::new();
    install(&mut world);
    spawn_soft(&mut world, grid_cloth(CLOTH_W));
    let mut sys = IntoSystem::into_system(physics_soft_step_colored);
    for s in 0..STEPS {
        if s == STEPS - 1
            && let Some(sc) = world.try_resource_mut::<SoftColorScratch>()
        {
            sc.reset_parallel_counter();
        }
        world.run_system_once(&mut sys);
    }
    let count = world
        .try_resource::<SoftColorScratch>()
        .expect("invariant: SoftColorScratch was inserted")
        .parallel_color_count();
    (pos_bits(&mut world), count)
}

/// What one worker-route run reports back.
struct Run {
    bits: OnceLock<Vec<(u32, u32, u32)>>,
    /// Colors that crossed the threshold and dispatched a `pool.scope` in the last step.
    parallel_colors: AtomicUsize,
    /// The route receipt: the worker id the step ran on.
    worker_id: AtomicU32,
    done: AtomicBool,
}

/// Runs [`run_colored_here`] inside a task on a `workers`-thread pool and waits for it from the
/// test thread WITHOUT joining the pool: `pool.spawn` + a flag, so the test thread never becomes
/// a helper and the whole solve — dispatch, chunks and join — belongs to the worker.
fn run_colored_on_a_worker(workers: usize) -> (Vec<(u32, u32, u32)>, usize, u32) {
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let out = Arc::new(Run {
        bits: OnceLock::new(),
        parallel_colors: AtomicUsize::new(0),
        worker_id: AtomicU32::new(u32::MAX),
        done: AtomicBool::new(false),
    });

    let sink = Arc::clone(&out);
    pool.spawn(move || {
        // Recorded BEFORE the solve, so the route is known even if the solve panics.
        sink.worker_id.store(current_worker_id(), Ordering::SeqCst);
        let (bits, count) = run_colored_here();
        sink.parallel_colors.store(count, Ordering::SeqCst);
        let _ = sink.bits.set(bits);
        sink.done.store(true, Ordering::Release);
    });

    let deadline = Instant::now() + TASK_DEADLINE;
    while !out.done.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "the worker-routed soft step did not finish within {TASK_DEADLINE:?} on a \
             {workers}-worker pool: the nested dispatch did not complete"
        );
        yield_now();
    }

    let bits = out.bits.get().cloned().expect("invariant: `done` is set after `bits`");
    (bits, out.parallel_colors.load(Ordering::SeqCst), out.worker_id.load(Ordering::SeqCst))
}

// ── Tests ────────────────────────────────────────────────────────────────────────────────────

/// Anti-vacuity for this file, asserted on its own: the step ran on a REGISTERED WORKER and it
/// genuinely dispatched.
///
/// If the id were `WORKER_ID_DISPATCHER` (`u32::MAX - 1`) or `WORKER_ID_UNATTACHED` (`u32::MAX`)
/// this file would be re-measuring the external-joiner route that `soft_colored_sp4.rs` already
/// covers (`KE16-DESIGN-MEASUREMENT.md` §5 shape 2), and if no color crossed the threshold the
/// oracle below would agree with the serial path for the wrong reason.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: builds a real OS thread pool and steps a 1600-particle cloth"
)]
fn soft_colored_from_a_worker_runs_on_a_registered_worker_and_dispatches() {
    let (_bits, parallel_colors, worker_id) = run_colored_on_a_worker(4);
    println!(
        "[ke16 app-1 soft] workers=4 worker_id={worker_id} parallel_colors={parallel_colors}"
    );

    assert!(
        (worker_id as usize) < MAX_WORKERS,
        "the soft step recorded worker id {worker_id} (>= MAX_WORKERS = {MAX_WORKERS}): it ran \
         off-pool, so this file is exercising the external-joiner route rather than the \
         registered-worker route App-1's lane count is about"
    );
    assert!(
        parallel_colors >= 1,
        "no color crossed MIN_PARALLEL_SLOTS_PER_COLOR: the cloth took the inline fallback and \
         `soft/colored.rs`'s lane arithmetic was never reached"
    );
}

/// The `{1, N}` oracle on the worker route: the colored soft solve dispatched from a registered
/// worker produces the same position bits at W=1, at W=4, and with no pool at all.
///
/// `lanes` sizes only the chunk COUNT (`n_chunks = clamp(lanes * CHUNKS_PER_WORKER, 1, total)`),
/// and the solve is chunk-count- and chunk-shape-independent, so all three must agree bit for
/// bit. A divergence means either the partition became value-visible or `lanes` reached something
/// other than the chunk count.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: three 1600-particle cloth runs, two of them across a real OS thread pool"
)]
fn soft_colored_from_a_worker_is_bit_identical_at_one_and_four_workers() {
    let (bits_1, colors_1, _) = run_colored_on_a_worker(1);
    let (bits_4, colors_4, _) = run_colored_on_a_worker(4);
    let (bits_inline, _) = run_colored_here();

    assert!(
        colors_1 >= 1 && colors_4 >= 1,
        "anti-vacuity: both worker-routed runs must dispatch at least one color \
         (W=1 -> {colors_1}, W=4 -> {colors_4})"
    );
    assert_ne!(
        bits_inline,
        {
            let mut still = EcsMaster::new();
            install(&mut still);
            spawn_soft(&mut still, grid_cloth(CLOTH_W));
            pos_bits(&mut still)
        },
        "the cloth did not move over {STEPS} steps: the snapshot is the seed state and would \
         match any solve, correct or not"
    );

    assert_eq!(
        bits_1, bits_4,
        "the worker-routed colored soft solve is not bit-identical between a 1-worker and a \
         4-worker pool: the chunk count became value-visible"
    );
    assert_eq!(
        bits_4, bits_inline,
        "the worker-routed colored soft solve diverges from the pool-free inline fallback: the \
         parallel partition is not value-neutral on the registered-worker route"
    );
}
