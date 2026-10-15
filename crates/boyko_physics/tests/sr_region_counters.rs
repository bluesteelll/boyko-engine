//! SR phase B — the solve region's counts, read unarmed (the cut's C2 gate, critique W3: the
//! solver keeps its last region's report, `ColoredSoftStepSolver::last_region_report`, so no
//! profiler and no second bound world is needed).
//!
//! # What is asserted
//!
//! * **One region per region step** (`region_dispatches` rises by one per step at W ≥ 2), and
//!   its published and inline item counts and its largest published entry equal this file's
//!   replica of the table builder, recomputed from the graph and the manifolds the step was
//!   handed — at W 2, 3, 4, 8 and 16, with `simd_solve` on and off, under the default grain and
//!   under `max_bpp` 2 and 4 and a lowered grain. The largest entry is at most the per-participant
//!   cap times the participants.
//! * **The helpers run blocks** of a J-T-sized pile at W8 and W16 (`helper_blocks > 0`): on EVERY
//!   step where each of the W participants has a usable CPU (`usable_cpus() >= W`: the 16-CPU dev
//!   host, at both widths), and on at least one step within [`OVERSUBSCRIBED_HORIZON_STEPS`] where
//!   they outnumber the CPUs (a 4-vCPU runner, at both widths). A region step lasts about a
//!   millisecond, and a helper that is not scheduled for a whole region runs no block of it (the
//!   phase A finding). With more participants than CPUs, and a runner's steal time on top, the
//!   caller can run every block of a step inline before any helper gets a CPU, and no width the
//!   process can read rules that out; a pool whose helpers never join still reds there. Every test
//!   of this file runs one at a time ([`serial`]), so the binary's own pools never take its
//!   helpers' CPUs. MEASURED 2026-10-11 on the 16-CPU dev host, release, under a 4-CPU affinity
//!   mask (`start /affinity F`) with busy threads under the same mask standing in for steal time.
//!   Before this split and [`serial`], at `--test-threads=4` (a 4-vCPU runner's libtest width):
//!   10 of 10 green with no busy thread, 3 of 10 red with one, 7 of 10 with two and 10 of 10 with
//!   three, each "W8/W16 step N: no helper ran a block" (the red of `windows-latest`'s release leg
//!   in CI runs 5 and 6, at W8 step 0 and step 5). One test at a time already cleared two busy
//!   threads (10 of 10 green, no helper-less step in 1,000), but not three (8 of 10 red on the
//!   every-step check). After: 20 of 20 green at `--test-threads=4` with zero, one, two, three and
//!   four busy threads, and 20 of 20 at full width with the every-step check. With three and four
//!   the gate stepped past [`HELPER_STEPS`] in one run each, the first after the busy threads
//!   started, at both widths: the first step with a helper block was step 11 and step 17.
//! * **The shipped policy's advance axis** (sr-policy S1): on the same pile at W8 and W16,
//!   `region_helper_advances` reads 0 on every region step, because the shipped `SolvePolicy`'s
//!   orchestrator publishes every item. The advances identity of `RegionReport::advances`
//!   (`advances == published + 1`) is asserted on every step, in release too. The helpers must
//!   have run blocks on some step at each width (the helper gate's rule above), or "no helper
//!   advanced" would prove nothing. This is the first behavioural physics catch of a shipped policy
//!   that takes the finisher axis (SR-B's close-test mutation MB). Shown red with the alias set to
//!   `WithAdvance<V2Policy, true>` (sr-policy R-ADV).
//! * **The scope count** of a warmed region step at W4, under a thread-local counting
//!   allocator: two allocations (the scope's shared frame and its first task chunk) for the one
//!   scope the step opens, the region's — the fill is the region's stage 0 (SR commit 7) and SR's
//!   flip retired S4's setup scope and the per-colour scopes — on twelve warmed steps.
//!   Some steps add one allocation of the pool's own (its deque and epoch bookkeeping: the same
//!   scene on the retired per-colour scopes read 28 and 29 alternating), so a step may read one
//!   more, and the fewest must read exactly two: a per-step allocation in the region path lifts
//!   them all.
//!
//! Spins real thread pools (intractable under Miri), so `cfg(not(miri))`.

#![cfg(not(miri))]
// clippy 1.98.0 false positive: this file's `thread_local!` initialiser already uses the
// `const { … }` form the lint asks for. See this crate's lib.rs for the full account.
#![allow(clippy::missing_const_for_thread_local)]

use boyko_physics::components::ColliderShape;
use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};
use boyko_physics::solver::ColoredSoftStepSolver;
use boyko_physics::solver::colored::RegionGrain;
use boyko_threadpool::ThreadPoolBuilder;

// ── The scene ────────────────────────────────────────────────────────────────

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
        simulated: true,
        kinematic: false,
        is_sensor: false,
        bp_margin: 0.0,
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
        bp_margin: 0.0,
        shape: ColliderShape::Sphere { radius: 1.0 },
    }
}

fn manifold(a: u32, b: u32, normal: Vec3, sep: f32, anchor: Vec3, points: u8) -> Manifold {
    let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
    m.normal = normal;
    for p in 0..usize::from(points) {
        let offset = Vec3::new(0.1 * p as f32, 0.0, 0.0);
        m.points[p] = ContactPoint {
            anchor_a: anchor + offset,
            anchor_b: anchor + offset,
            separation: sep,
            feature_id: p as u32,
        };
    }
    m.count = points;
    m
}

/// `bottoms` spheres on a shared static floor (the last row), the first `tops` of them carrying a
/// sphere each. The floor contacts of the carried spheres and their top contacts conflict, so the
/// graph has at least two colours: the floor contacts, most of them in one wide colour, and the
/// top contacts beside them. Contacts carry one to four points by their index.
fn pile(bottoms: u32, tops: u32) -> (Vec<BodyState>, Vec<Manifold>) {
    let mut bodies = Vec::new();
    for i in 0..bottoms {
        bodies.push(dyn_sphere(Vec3::new(i as f32 * 3.0, 0.6, 0.0), Vec3::new(0.0, -0.5, 0.1)));
    }
    for i in 0..tops {
        bodies.push(dyn_sphere(Vec3::new(i as f32 * 3.0, 2.4, 0.0), Vec3::new(0.1, -0.5, 0.0)));
    }
    let floor = bottoms + tops;
    bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));
    let mut manifolds = Vec::new();
    let down = Vec3::new(0.0, -1.0, 0.0);
    for i in 0..bottoms {
        let anchor = Vec3::new(i as f32 * 3.0, 0.0, 0.0);
        manifolds.push(manifold(i, floor, down, -0.2, anchor, 1 + (i % 4) as u8));
    }
    for i in 0..tops {
        let anchor = Vec3::new(i as f32 * 3.0, 1.5, 0.0);
        manifolds.push(manifold(i, bottoms + i, Vec3::new(0.0, 1.0, 0.0), -0.1, anchor, 1 + (i % 3) as u8));
    }
    manifolds.sort_by_key(|m| (m.body_a.0, m.body_b.0));
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

// ── The replica of the region's table builder ──────────────────────────────────

/// The solver's COHORT width (`solver/colored.rs`).
const COHORT: usize = 8;

/// What the replica predicts for one step's region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Predicted {
    published: u32,
    inline: u32,
    max_blocks: u32,
}

/// The colour cut's walk: groups holding `groups` points each, cut into at most `n_target` blocks
/// by point quota, snapped to cohorts under `simd`.
fn colour_blocks(groups: &[u32], n_target: usize, simd: bool) -> usize {
    let n = groups.len();
    let total: usize = groups.iter().map(|&g| g as usize).sum();
    let target = total.div_ceil(n_target).max(1);
    let step = if simd { COHORT } else { 1 };
    let mut start = vec![0usize];
    for &g in groups {
        start.push(start.last().copied().unwrap_or(0) + g as usize);
    }
    let (mut lo, mut blocks) = (0usize, 0usize);
    while lo < n {
        let mut hi = (lo + step).min(n);
        while hi < n && start[hi] - start[lo] < target {
            hi = (hi + step).min(n);
        }
        blocks += 1;
        lo = hi;
    }
    blocks
}

/// The cohort cut's walk: cohorts holding `cohorts` points each, at most `n_target` ranges.
fn cohort_blocks(cohorts: &[u32], n_target: usize) -> usize {
    if cohorts.is_empty() {
        return 0;
    }
    let total: usize = cohorts.iter().map(|&p| p as usize).sum();
    let target = total.div_ceil(n_target).max(1);
    let (mut n, mut lo, mut acc) = (0usize, 0usize, 0usize);
    for (k, &p) in cohorts.iter().enumerate() {
        acc += p as usize;
        if acc >= target && n + 1 < n_target {
            n += 1;
            lo = k + 1;
            acc = 0;
        }
    }
    if lo < cohorts.len() {
        n += 1;
    }
    n
}

/// The body cut: whole 8-row groups over every row.
fn body_blocks(rows: usize, grain: RegionGrain, p: usize) -> usize {
    let unit = (grain.body_rows as usize).div_ceil(8) * 8;
    let n = (grain.body_bpp as usize * p).min(rows.div_ceil(unit)).max(1);
    let per = rows.div_ceil(n).div_ceil(8) * 8;
    rows.div_ceil(per)
}

/// Predicts one step's region from what the step was handed: the graph's colours and the
/// manifolds' point counts (no island freezes in these runs), the row count, `p` participants.
#[allow(clippy::too_many_arguments)]
fn predict(
    graph: &ConstraintGraph,
    manifolds: &[Manifold],
    rows: usize,
    p: usize,
    grain: RegionGrain,
    simd: bool,
    substeps: usize,
    relax: usize,
) -> Predicted {
    let mut out = Predicted { published: 0, inline: 0, max_blocks: 0 };
    let mut item = |blocks: usize, times: usize| {
        if blocks >= 2 {
            out.published += times as u32;
            out.max_blocks = out.max_blocks.max(blocks as u32);
        } else {
            out.inline += times as u32;
        }
    };
    let body = body_blocks(rows, grain, p);
    // Gravity and integrate, once per substep each.
    item(body, 2 * substeps);
    // Every colour's cohorts, in the layout's order: the fill's ranges cut them.
    let mut all_cohorts: Vec<u32> = Vec::new();
    for c in 0..graph.n_colors() {
        let groups: Vec<u32> =
            graph.color(c).iter().map(|&mi| u32::from(manifolds[mi as usize].count)).filter(|&n| n != 0).collect();
        let points: usize = groups.iter().map(|&g| g as usize).sum();
        let cohorts: Vec<u32> = groups.chunks(COHORT).map(|c| c.iter().sum()).collect();
        let wide = !groups.is_empty() && points >= grain.wide_floor as usize;
        let sweep = if groups.is_empty() {
            0
        } else if wide {
            let n_target = (grain.max_bpp as usize * p)
                .min(points / grain.colour_min_points as usize)
                .clamp(1, groups.len());
            colour_blocks(&groups, n_target, simd)
        } else {
            1
        };
        let warm = if sweep <= 1 { sweep } else { cohort_blocks(&cohorts, sweep.min(cohorts.len())) };
        all_cohorts.extend(&cohorts);
        // Per substep: the warm start, one biased sweep, `relax` relax sweeps.
        item(warm, substeps);
        item(sweep, substeps * (1 + relax));
    }
    // The fill (stage 0, once): S4's task count with the region grain's terms, S4's cut; under two
    // ranges it is one inline block.
    let points: usize = all_cohorts.iter().map(|&p| p as usize).sum();
    let tasks = (grain.max_bpp as usize * p).min(points / grain.fill_points as usize).min(all_cohorts.len()).min(32);
    let fill = if tasks >= 2 { cohort_blocks(&all_cohorts, tasks).max(1) } else { 1 };
    item(fill, 1);
    // The store (once, last): the fill's cohort ranges; these piles are warm and no row bounces.
    item(fill, 1);
    out
}

// ── The machine ──────────────────────────────────────────────────────────────

/// The region steps each width of the helper gate runs, and checks one by one where every
/// participant has a usable CPU.
const HELPER_STEPS: usize = 10;

/// Where the participants outnumber the usable CPUs, the helper gate keeps stepping past
/// [`HELPER_STEPS`] until a helper has run a block, up to this many steps in all (about 0.4 s at
/// the 2 ms such a step took under four busy threads on 4 CPUs). MEASURED 2026-10-11 (module docs):
/// the first step with a helper block came at step 17 at the latest, so a pool whose helpers join
/// at all clears it with a wide margin, and one whose helpers never do reds once it is spent.
const OVERSUBSCRIBED_HORIZON_STEPS: usize = 200;

/// The logical CPUs this process may run on: `available_parallelism`, narrowed on Windows by the
/// process affinity mask, which std does not read there (MEASURED 2026-10-10, Rust 1.98.1, msvc,
/// 16-CPU dev host: under `start /affinity F` the mask is `0xf` and `available_parallelism` still
/// reads 16). On Linux std already reads the affinity mask and the cgroup quota. The same helper
/// as `boyko_threadpool`'s `ke16_nested_scope_occupancy`.
fn usable_cpus() -> usize {
    let available = std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
    #[cfg(windows)]
    {
        if let Some(mask_cpus) = affinity_cpus() {
            return available.min(mask_cpus);
        }
    }
    available
}

/// The CPU count of this process's affinity mask, or `None` if the call fails. A process spanning
/// several processor groups reports its current group's mask only; the machines this runs on have
/// at most 64 logical CPUs, one group.
#[cfg(windows)]
fn affinity_cpus() -> Option<usize> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn GetProcessAffinityMask(
            process: *mut std::ffi::c_void,
            process_mask: *mut usize,
            system_mask: *mut usize,
        ) -> i32;
    }
    let (mut process_mask, mut system_mask) = (0usize, 0usize);
    // SAFETY: `GetCurrentProcess` takes no argument and returns the current process's
    // pseudo-handle, which is always valid and needs no closing. `GetProcessAffinityMask` reads that
    // handle and writes one `DWORD_PTR` (`usize` on every Windows target) through each of its two
    // out-pointers, which point at live, aligned, writable `usize` locals for the whole call.
    let ok = unsafe {
        GetProcessAffinityMask(GetCurrentProcess(), &raw mut process_mask, &raw mut system_mask)
    };
    (ok != 0 && process_mask != 0).then(|| process_mask.count_ones() as usize)
}

/// Holds every test of this binary apart from the others: the guard lives for the whole test
/// body. Each test spins pools of up to 16 workers, and libtest runs as many tests at once as
/// there are CPUs, so without it the replica's pools take the CPUs the helper gate's helpers need
/// (MEASURED, module docs). A poisoned lock still serialises, so one red test does not red the
/// rest.
///
/// `clippy::disallowed_types`: a test-only gate between test bodies, taken once per test and never
/// on a measured path; the hot-path ban is about engine code.
#[allow(clippy::disallowed_types)]
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ── The gates ────────────────────────────────────────────────────────────────

fn cfg(simd_solve: bool) -> PhysicsConfig {
    PhysicsConfig { dt: 1.0 / 60.0, parallel_solve: true, simd_solve, ..PhysicsConfig::default() }
}

/// The grains the replica is checked under.
fn grains() -> [(&'static str, RegionGrain); 4] {
    [
        ("default", RegionGrain::DEFAULT),
        ("bpp2", RegionGrain { max_bpp: 2, ..RegionGrain::DEFAULT }),
        ("bpp4", RegionGrain { max_bpp: 4, ..RegionGrain::DEFAULT }),
        (
            "lowered",
            RegionGrain { wide_floor: 1, colour_min_points: 1, max_bpp: 6, body_bpp: 8, body_rows: 8, fill_points: 1 },
        ),
    ]
}

#[test]
fn region_counts_equal_the_builder_replica() {
    let _serial = serial();
    let (bodies, manifolds) = pile(400, 40);
    let graph = build_graph(&bodies, &manifolds);
    assert!(graph.n_colors() >= 2, "non-vacuity: the pile has a wide colour and a narrow one");
    let config = cfg(true);
    let (substeps, relax) = (config.substeps as usize, config.relax_iterations as usize);
    let mut published_seen = 0u64;
    for simd in [true, false] {
        for (name, grain) in grains() {
            for workers in [2usize, 3, 4, 8, 16] {
                let mut solver = ColoredSoftStepSolver::default();
                assert!(solver.set_region_grain(grain), "{name}: a valid grain");
                let mut scratch = SolverScratch::with_capacity(bodies.len());
                scratch.set_bodies(&bodies);
                let pool = ThreadPoolBuilder::new().num_threads(workers).build();
                pool.install(|_| {
                    for step in 0..3 {
                        scratch.touched.reset(scratch.bodies().len());
                        let before = solver.region_dispatches();
                        solver.solve_colored(&cfg(simd), &manifolds, &graph, &mut scratch);
                        assert_eq!(
                            solver.region_dispatches() - before,
                            1,
                            "{name} simd {simd} W{workers} step {step}: one region per step"
                        );
                        let r = solver.last_region_report();
                        let want = predict(&graph, &manifolds, bodies.len(), workers, grain, simd, substeps, relax);
                        let got = Predicted { published: r.published, inline: r.inline, max_blocks: r.max_blocks };
                        assert_eq!(got, want, "{name} simd {simd} W{workers} step {step}: the region's counts");
                        let cap = grain.max_bpp.max(grain.body_bpp) * workers as u32;
                        assert!(
                            r.max_blocks <= cap,
                            "{name} W{workers}: an entry of {} blocks passes the cap {cap}",
                            r.max_blocks
                        );
                        published_seen += u64::from(r.published);
                    }
                });
            }
        }
    }
    assert!(published_seen > 0, "non-vacuity: some region published an item");
}

/// The name is the every-step check's, which runs wherever every participant has a usable CPU;
/// where they outnumber the CPUs the check is the oversubscribed one (module docs).
#[test]
fn helpers_run_blocks_on_every_step_at_w8_and_w16() {
    let _serial = serial();
    // A J-T-sized pile: 1,240 floor contacts, most of them in one wide colour.
    let (bodies, manifolds) = pile(1200, 40);
    let graph = build_graph(&bodies, &manifolds);
    let cpus = usable_cpus();
    for workers in [8usize, 16] {
        let every_step = cpus >= workers;
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.set_bodies(&bodies);
        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        let (steps, helped) = pool.install(|_| {
            let (mut step, mut helped) = (0usize, 0usize);
            while step < HELPER_STEPS || (!every_step && helped == 0 && step < OVERSUBSCRIBED_HORIZON_STEPS) {
                scratch.touched.reset(scratch.bodies().len());
                let before = solver.region_dispatches();
                solver.solve_colored(&cfg(true), &manifolds, &graph, &mut scratch);
                assert_eq!(solver.region_dispatches() - before, 1, "W{workers} step {step}: a region step");
                let r = solver.last_region_report();
                assert!(
                    !every_step || r.helper_blocks > 0,
                    "W{workers} step {step}: no helper ran a block of the region ({r:?}), with {cpus} usable \
                     CPUs for its {workers} participants"
                );
                helped += usize::from(r.helper_blocks > 0);
                step += 1;
            }
            (step, helped)
        });
        let check = if every_step { "every step" } else { "oversubscribed" };
        eprintln!("[SR] W{workers} on {cpus} usable CPUs ({check}): a helper ran blocks on {helped} of {steps} steps");
        assert!(
            helped > 0,
            "W{workers}: no helper ran a block of the region in {steps} steps on {cpus} usable CPUs; a pool's \
             helpers join a region even when its participants outnumber the CPUs"
        );
    }
}

/// sr-policy S1: the shipped `SolvePolicy` advances every item on participant 0, so
/// `region_helper_advances` reads 0 on every region step while the helpers run blocks (module
/// docs). Runs at least [`HELPER_STEPS`] steps per width, and on until a helper has run a block, up
/// to [`OVERSUBSCRIBED_HORIZON_STEPS`].
#[test]
fn the_shipped_policy_advances_every_item_on_participant_0() {
    let _serial = serial();
    let (bodies, manifolds) = pile(1200, 40);
    let graph = build_graph(&bodies, &manifolds);
    for workers in [8usize, 16] {
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.set_bodies(&bodies);
        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        let (steps, helped) = pool.install(|_| {
            let (mut step, mut helped) = (0usize, 0usize);
            while step < HELPER_STEPS || (helped == 0 && step < OVERSUBSCRIBED_HORIZON_STEPS) {
                scratch.touched.reset(scratch.bodies().len());
                let before = solver.region_dispatches();
                solver.solve_colored(&cfg(true), &manifolds, &graph, &mut scratch);
                assert_eq!(solver.region_dispatches() - before, 1, "W{workers} step {step}: a region step");
                let r = solver.last_region_report();
                assert_eq!(
                    solver.region_helper_advances(),
                    0,
                    "W{workers} step {step}: a helper published an item of the shipped policy's region ({r:?})"
                );
                assert_eq!(
                    r.advances,
                    u64::from(r.published) + 1,
                    "W{workers} step {step}: the advances identity, advances == published + 1 ({r:?})"
                );
                helped += usize::from(r.helper_blocks > 0);
                step += 1;
            }
            (step, helped)
        });
        eprintln!("[SR-POLICY] W{workers}: 0 helper advances on {steps} steps, a helper ran blocks on {helped}");
        assert!(
            helped > 0,
            "W{workers}: no helper ran a block of the region in {steps} steps, so 0 helper advances proves nothing"
        );
    }
}

#[test]
fn a_warmed_region_step_allocates_two_per_scope_it_opened() {
    let _serial = serial();
    let (bodies, manifolds) = pile(400, 40);
    let graph = build_graph(&bodies, &manifolds);
    let mut solver = ColoredSoftStepSolver::default();
    let mut scratch = SolverScratch::with_capacity(bodies.len());
    scratch.set_bodies(&bodies);
    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    // Per warmed step: (allocations, regions).
    let steps: Vec<(usize, u64)> = pool.install(|_| {
        for _ in 0..8 {
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored(&cfg(true), &manifolds, &graph, &mut scratch);
        }
        (0..12)
            .map(|_| {
                scratch.touched.reset(scratch.bodies().len());
                let r0 = solver.region_dispatches();
                let before = ALLOC.count();
                solver.solve_colored(&cfg(true), &manifolds, &graph, &mut scratch);
                let allocs = ALLOC.count().wrapping_sub(before);
                (allocs, solver.region_dispatches() - r0)
            })
            .collect()
    });
    eprintln!("[SR] warmed region steps at W4 (allocs, regions): {steps:?}");
    for &(allocs, regions) in &steps {
        assert_eq!(regions, 1, "non-vacuity: every warmed step opened its region");
        let scopes = 2 * regions as usize;
        // The pool's own deque and epoch bookkeeping adds one allocation on some steps, whatever
        // the solve does (measured on this scene on the retired per-colour scopes: 28 and 29
        // alternating).
        assert!(
            (scopes..=scopes + 1).contains(&allocs),
            "a warmed region step allocates the shared frame and the first task chunk of the one \
             scope it opens ({regions} region(s)), and at most one allocation of the pool's own: \
             {allocs}"
        );
    }
    let fewest = steps.iter().map(|&(a, r)| a - 2 * r as usize).min();
    assert_eq!(
        fewest,
        Some(0),
        "some warmed step must allocate exactly two per scope: a per-step allocation in the region \
         path would lift every step ({steps:?})"
    );
}

// ── Thread-local counting allocator (the pattern of `large_island_gate_p2.rs`) ─────

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

impl CountingAlloc {
    fn count(&self) -> usize {
        ALLOC_COUNT.with(Cell::get)
    }
}

#[inline]
fn bump_alloc_count() {
    let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
}

// SAFETY: every call forwards verbatim to the platform `System` allocator with the same layout;
//   the wrapper only bumps a thread-local counter (through `try_with`, which no-ops while TLS is
//   initialising, so it never re-enters the allocator). `dealloc` is an unchanged pass-through, so
//   the allocator contract is exactly `System`'s.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: forwarded verbatim to the system allocator (same layout).
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` / `layout` come from `System.alloc` above.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: forwarded verbatim; `ptr` / `layout` come from this allocator.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;
