//! `physics_zones_count_exactly`: every physics profiling zone records exactly its structural
//! count on every step, and every counter records exactly the quantity it names — each checked
//! against a value recomputed from the world's public state, not from the solver's own columns.
//!
//! # What is asserted, per step
//!
//! Armed, on the shared scene (`support/profiling_harness.rs`: one wide color and two narrow ones,
//! a 4-worker pool, `parallel_solve` and `parallel_narrowphase` on), with the shipped 4 substeps
//! and 2 relax passes. After each
//! step the profiler is folded and each zone's lifetime accumulator is diffed against its value
//! before the step:
//!
//! | zone | samples per step |
//! |---|---|
//! | `phys_solve_build`, `phys_restitution`, `phys_store`, `phys_write_back` | 1 |
//! | `phys_gravity`, `phys_warm_apply`, `phys_pass_biased`, `phys_integrate` | substeps = 4 |
//! | `phys_pass_relax` | substeps × relax = 8 |
//! | `phys_color_wide` / `phys_color_narrow` | (wide / narrow colors) × 12 sweeps |
//! | `phys_sleep_begin` / `phys_sleep_freeze` / `phys_sleep_end` | 0 / 0 / 0 sleeping off; 1 / 2 / 1 on |
//! | `phys_np_dispatch` / `phys_np_compact` / `phys_np_axis_commit` | 1 each when the recomputed narrowphase chunk count is at least 2, else 0 |
//! | `phys_bp_verify` / `phys_bp_build` / `phys_bp_query` / `phys_bp_assemble` | 1 each on a tree-path step, 0 on the AllPairs step |
//! | each of the ten step counters | 1, with the step's value as its total |
//! | `phys_bp_queried` / `phys_bp_members` / `phys_bp_rebuilds` | tree path: (1, N − members) / (1, [step ≥ 2]) / (1, [step == 2]); AllPairs: (0, 0) |
//! | `phys_sb_bodies` / `phys_sb_pa` / `phys_sb_pb` / `phys_sb_pc` (W8S) | 1 each |
//! | `phys_wave_ramp` / `_tail` / `_inflight` / `_lanes` (W8S) | 1 each, the sum over the step's dispatched colour waves (wide colors × 12 sweeps); the in-flight and lane sums each in `[waves, waves × (W + 1)]` |
//! | `phys_wave_overflow` / `phys_color_scopes` / `phys_color_tasks` (W8S) | (1, 0) / (1, the waves) / (1, the tasks the solver's cut walk makes, recomputed) |
//! | `phys_route_worker` + `phys_route_external` (W8S) | 1 each, summing to the waves |
//! | the ten `phys_hist_*` counters (W8S) | 1 each, the colours and slots per bin, recomputed |
//! | `phys_s6_graph_hit` / `phys_s6_pb_hit` (W8S) | 1 each, 0 or 1, the P-b hit never without the graph hit, both 0 on the first armed step |
//! | `phys_np_wave_*` (W8S) | 1 each when the narrowphase dispatched; in-flight and lanes in `[1, W + 1]`, overflow 0 |
//! | `phys_wave_join` / `_helped` / `_first_ramp` / `_first_tail` / `_pass_ramp` (W8S review) | 1 each; join ≤ tail, first tail ≤ tail, first ramp ≤ pass ramp ≤ ramp, helped ≤ the solve scopes, and no helped wave means a ramp of 0 |
//! | `phys_np_wave_join` / `phys_np_route_worker` (W8S review) | 1 each when the narrowphase dispatched; join ≤ its tail, route 0 or 1 |
//! | every system of the schedule (its `SystemSpan`) | 1 |
//!
//! The W8S dispatch counters are recomputed from the same graph and manifolds: a colour
//! dispatches when `parallel_solve` is on, the pool has two workers and the step's widest colour
//! reaches `WIDE_COLOR_MIN_SLOTS` (the scene's wide colour, on every step), and its task count is
//! [`expected_color_tasks`], this test's copy of the solver's cut walk. The S6 counter must hit on
//! some step of the run, or its 0 would be the only value ever checked.
//!
//! The first [`STEPS_OFF`] steps run with sleeping off, the parity configuration; summed over
//! them the counts are the plan's literals — build 3, gravity = warm = integrate = biased = 12,
//! relax 24. The next [`STEPS_ON`] steps run with sleeping on, so the three sleep zones are
//! counted too; no island can freeze that early (the debounce is 60 steps), and the test asserts
//! every dynamic row awake, so every manifold is still solved and the recomputation holds. The
//! next [`STEPS_BRUTE`] step keeps the Tree but raises `brute_max_rows` above the row count (the
//! brute loop), and the last [`STEPS_ALLPAIRS`] step switches the broadphase to `AllPairs`; on
//! both every tree zone and counter must read zero.
//!
//! The tree counters are pinned to the scene's structure, not read back from the tree's own
//! `diag()`: the floor is the scene's one static, so it is pending at step 1 and admitted at
//! step 2 (rent 2 ≥ 1 + 1/4), after which it is the one member; every other row is queried.
//!
//! # The independent recomputation
//!
//! A color's class and slot count come from the `ConstraintGraph` resource and the `Manifolds`
//! resource — the colors' manifold lists and each manifold's point count — not from the solver's
//! `color_offsets`, which is what the zones themselves read. Wide means at least
//! `WIDE_COLOR_MIN_SLOTS` slots, the solver's inline floor. The scene must show at least one
//! wide and one narrow color on every step, or a bug that classes every color the same way would
//! pass on the other side's zero — and a DIFFERENT number of each, or a bug that swapped the two
//! classes would leave both counts unchanged. (Measured: on a scene with one of each, inverting
//! the class predicate in `solve_all_colors` passed this test.) The narrowphase counters are recomputed from `Manifolds` and
//! `ContactPairs`, the broadphase counter from `ContactPairs`. The three narrowphase class counters
//! (L9: reused, separated-axis hits, full) are the narrowphase's public `pair_classes`, which must
//! close over the step: their non-box count equals the one recomputed here from the pairs' shapes,
//! and the four classes sum to the pairs. The harness keeps the shipped contact reuse, on since L9
//! C4, and some step must reuse a record, so the reused counter is checked against a live count.
//!
//! The narrowphase's chunk count is recomputed from the pair count, the pool's worker count and the
//! exported `NP_*` constants — with the `lanes < 2` term, as the engine's rule has it — and the
//! scene must dispatch on every step (at least two chunks), or the three L5 zones would be checked
//! against zero. `Manifolds::narrowphase_dispatches` must rise by one per step as well.
//!
//! Also asserted: the physics zones' ids are distinct from each other and from every system's,
//! so no two rows are one row; and the store reports no dropped sample.
//!
//! A step's counter rows and their relations are checked together and reported together: a
//! mutation that silences several counters names every one of them in one run.
//!
//! Under a profile whose tier folds the zones, the correct reading is zero everywhere, and that
//! is what is asserted instead.
//!
//! # Shown red (2026-09-19, msvc, debug), each on the assertion it targets
//!
//! Measured on the harness scene as it stood then, nine rows deep (90 floor bodies); the figures
//! are that scene's. L5 grew it to twenty rows, so today's counts differ.
//!
//! * the class predicate inverted at the color zone in `solve_all_colors`: `phys_color_wide`
//!   24 spans against 12 on step 0;
//! * the relax-pass zone deleted: `phys_pass_relax` 0 against 8;
//! * the class predicate inverted in the slot counters only: `phys_slots_wide` 72 against 360;
//! * `phys_np_points` reporting the manifold count: 108 against 432;
//! * the broadphase counter deleted: `phys_bp_pairs` (0, 0) against (1, 135);
//! * the zone around the frozen-row capture deleted: `phys_sleep_freeze` 1 against 2 on step 3,
//!   the first sleeping-on step.
//!
//! # How to run
//!
//! ```text
//! cargo test -p boyko-physics --test profiling_zone_counts
//! ```
//!
//! `harness = false`: the binary's only test runs alone in its process (see the harness module
//! for why). Counts, not timings: it needs no quiet machine.

#[path = "support/profiling_harness.rs"]
mod harness;

use boyko_diag::profiling_abi::{ZoneHandle, zone_id};
use boyko_ecs::ecs::core::profiling::SYSTEM_ZONES_COMPILED;
use boyko_physics::narrowphase::{NP_CHUNKS_PER_LANE, NP_MAX_CHUNKS, NP_MIN_PAIRS_PER_CHUNK};
use boyko_physics::profiling::{
    COUNTER_ZONE_COUNT, COUNTER_ZONES, HIST_BINS, PHYS_BP_ASSEMBLE, PHYS_BP_BUILD,
    PHYS_BP_MEMBERS, PHYS_BP_PAIRS, PHYS_BP_QUERIED, PHYS_BP_QUERY, PHYS_BP_REBUILDS,
    PHYS_BP_VERIFY, PHYS_COLOR_NARROW, PHYS_COLOR_SCOPES, PHYS_COLOR_TASKS, PHYS_COLOR_WIDE,
    PHYS_GRAVITY, PHYS_HIST_COLORS_GE256, PHYS_HIST_COLORS_LT32, PHYS_HIST_COLORS_LT64,
    PHYS_HIST_COLORS_LT128, PHYS_HIST_COLORS_LT256, PHYS_HIST_SLOTS_GE256, PHYS_HIST_SLOTS_LT32,
    PHYS_HIST_SLOTS_LT64, PHYS_HIST_SLOTS_LT128, PHYS_HIST_SLOTS_LT256, PHYS_INTEGRATE,
    PHYS_NP_AXIS_COMMIT, PHYS_NP_CHUNKS, PHYS_NP_COMPACT, PHYS_NP_DISPATCH, PHYS_NP_FULL,
    PHYS_NP_MANIFOLDS, PHYS_NP_PAIRS, PHYS_NP_POINTS, PHYS_NP_REUSED, PHYS_NP_SEP_HITS,
    PHYS_NP_ROUTE_WORKER, PHYS_NP_WAVE_INFLIGHT, PHYS_NP_WAVE_JOIN, PHYS_NP_WAVE_LANES,
    PHYS_NP_WAVE_OVERFLOW, PHYS_NP_WAVE_RAMP, PHYS_NP_WAVE_TAIL, PHYS_PASS_BIASED,
    PHYS_PASS_RELAX, PHYS_RESTITUTION, PHYS_ROUTE_EXTERNAL, PHYS_ROUTE_WORKER, PHYS_S6_GRAPH_HIT,
    PHYS_S6_PB_HIT, PHYS_SB_BODIES, PHYS_SB_PA, PHYS_SB_PB, PHYS_SB_PC, PHYS_SLEEP_BEGIN,
    PHYS_SLEEP_CLASSIFY, PHYS_SLEEP_END, PHYS_SLEEP_FREEZE, PHYS_SLEEP_HELD, PHYS_SLOTS_NARROW,
    PHYS_SLOTS_WIDE, PHYS_SOLVE_BUILD, PHYS_STORE, PHYS_WARM_APPLY, PHYS_WAVE_FIRST_RAMP,
    PHYS_WAVE_FIRST_TAIL, PHYS_WAVE_HELPED, PHYS_WAVE_INFLIGHT, PHYS_WAVE_JOIN, PHYS_WAVE_LANES,
    PHYS_WAVE_OVERFLOW, PHYS_WAVE_PASS_RAMP, PHYS_WAVE_RAMP, PHYS_WAVE_TAIL, PHYS_WRITE_BACK,
    SPAN_ZONE_COUNT, SPAN_ZONES, WIDE_COLOR_MIN_SLOTS, ZONES_COMPILED, hist_bin,
};
use boyko_physics::broadphase_tree::BroadphaseTree;
use boyko_physics::components::ColliderShape;
use boyko_physics::resources::{
    BroadphaseKind, ConstraintGraph, ContactPairs, IslandSleep, Manifolds, PhysicsConfig,
    SolverScratch,
};

use harness::{Scene, WORKERS, run_single_test};

/// The name libtest would list this test under.
const TEST_NAME: &str = "physics_zones_count_exactly";
/// Steps with sleeping off: the plan's "3 steps".
const STEPS_OFF: usize = 3;
/// Steps with sleeping on after them, for the sleep zones.
const STEPS_ON: usize = 2;
/// One step on the Tree's brute path (`brute_max_rows` raised above the row count): the tree
/// zones and counters read zero there too — the "Tree with N ≤ brute" row of the design's
/// per-path table (W5).
const STEPS_BRUTE: usize = 1;
/// One last step on the `AllPairs` broadphase: the tree zones and counters read zero.
const STEPS_ALLPAIRS: usize = 1;
/// Every step.
const STEPS: usize = STEPS_OFF + STEPS_ON + STEPS_BRUTE + STEPS_ALLPAIRS;

fn main() {
    run_single_test(TEST_NAME, physics_zones_count_exactly);
}

/// The solver's colour cut constants (`solver/colored.rs`: `CHUNKS_PER_WORKER`,
/// `MIN_SLOTS_PER_CHUNK`, `COHORT`), copied: the task count below is a second derivation, not a
/// read of the solver.
const COLOR_CHUNKS_PER_WORKER: usize = 6;
const COLOR_MIN_SLOTS_PER_CHUNK: usize = 64;
const COLOR_COHORT: usize = 8;

/// One step's structure, recomputed from the world after the step.
#[derive(Debug)]
struct StepShape {
    wide_colors: u64,
    narrow_colors: u64,
    wide_slots: u64,
    narrow_slots: u64,
    pairs: u64,
    manifolds: u64,
    points: u64,
    /// W8S: the tasks one sweep's wide colours cut into.
    wide_tasks: u64,
    /// W8S: colours per histogram bin.
    hist_colors: [u64; HIST_BINS],
    /// W8S: their slots.
    hist_slots: [u64; HIST_BINS],
}

/// The tasks one dispatched colour spawns, its laid-out groups holding `groups` points each in
/// order: the solver's chunk count and cut walk (`solve_color_parallel`) over the copied
/// constants, SIMD cohort snapping included.
fn expected_color_tasks(groups: &[u32], lanes: usize, simd_solve: bool) -> u64 {
    let n_groups = groups.len();
    if n_groups == 0 {
        return 0;
    }
    let total: usize = groups.iter().map(|&g| g as usize).sum();
    let by_work = (total / COLOR_MIN_SLOTS_PER_CHUNK).max(1);
    let n_chunks = (lanes * COLOR_CHUNKS_PER_WORKER).min(by_work).clamp(1, n_groups);
    let target = total.div_ceil(n_chunks).max(1);
    let step = if simd_solve { COLOR_COHORT } else { 1 };
    let mut start = Vec::with_capacity(n_groups + 1);
    start.push(0usize);
    for &g in groups {
        start.push(start.last().copied().unwrap_or(0) + g as usize);
    }
    let (mut lo, mut tasks) = (0usize, 0u64);
    while lo < n_groups {
        let mut hi = (lo + step).min(n_groups);
        while hi < n_groups && start[hi] - start[lo] < target {
            hi = (hi + step).min(n_groups);
        }
        tasks += 1;
        lo = hi;
    }
    tasks
}

/// Recomputes this step's color classes and narrowphase output from the graph and manifold
/// resources — what the solve was handed, not what it built.
fn step_shape(scene: &Scene) -> StepShape {
    let graph = scene.world.resource::<ConstraintGraph>();
    let manifolds = scene.world.resource::<Manifolds>().solver_manifolds();
    let simd_solve = scene.world.resource::<PhysicsConfig>().simd_solve;
    let mut shape = StepShape {
        wide_colors: 0,
        narrow_colors: 0,
        wide_slots: 0,
        narrow_slots: 0,
        pairs: scene.world.resource::<ContactPairs>().pairs().len() as u64,
        manifolds: manifolds.len() as u64,
        points: manifolds.iter().map(|m| u64::from(m.count)).sum(),
        wide_tasks: 0,
        hist_colors: [0; HIST_BINS],
        hist_slots: [0; HIST_BINS],
    };
    for c in 0..graph.n_colors() {
        // The colour's laid-out groups: its manifolds in order, empty ones skipped (no island
        // of this scene freezes within its steps, asserted per step).
        let groups: Vec<u32> = graph
            .color(c)
            .iter()
            .map(|&mi| u32::from(manifolds[mi as usize].count))
            .filter(|&n| n != 0)
            .collect();
        let slots: u32 = groups.iter().sum();
        if slots >= WIDE_COLOR_MIN_SLOTS {
            shape.wide_colors += 1;
            shape.wide_slots += u64::from(slots);
            shape.wide_tasks += expected_color_tasks(&groups, WORKERS, simd_solve);
        } else {
            shape.narrow_colors += 1;
            shape.narrow_slots += u64::from(slots);
        }
        if let Some(bin) = hist_bin(slots) {
            shape.hist_colors[bin] += 1;
            shape.hist_slots[bin] += u64::from(slots);
        }
    }
    shape
}

/// `(count, total)` of each zone id's lifetime accumulator.
fn snapshot(scene: &Scene, ids: &[u16]) -> Vec<(u64, u64)> {
    ids.iter()
        .map(|&id| {
            let acc = scene.lifetime_of(id);
            (acc.count, acc.total)
        })
        .collect()
}

fn name_of(handle: &ZoneHandle) -> &'static str {
    handle.desc.name
}

/// The narrowphase's chunk count for `pairs` candidate pairs on `lanes` workers, or 0 for the
/// serial loop: a copy of the engine's rule from its exported constants, lanes term included.
fn expected_np_chunks(parallel_np: bool, pairs: u64, lanes: usize) -> u64 {
    if !parallel_np || lanes < 2 {
        return 0;
    }
    let pairs = usize::try_from(pairs).expect("a pair count fits usize");
    let chunks = (lanes * NP_CHUNKS_PER_LANE).min(pairs / NP_MIN_PAIRS_PER_CHUNK).min(NP_MAX_CHUNKS);
    if chunks < 2 { 0 } else { chunks as u64 }
}

fn physics_zones_count_exactly() {
    let mut scene = Scene::spawn();
    scene.arm_profiler();

    let (substeps, relax, parallel_np) = {
        let cfg = scene.world.resource::<PhysicsConfig>();
        (u64::from(cfg.substeps), u64::from(cfg.relax_iterations), cfg.parallel_narrowphase)
    };
    assert!(parallel_np, "the harness requests the parallel narrowphase");
    assert_eq!(
        (substeps, relax),
        (4, 2),
        "the plan's literal counts are for the shipped 4 substeps and 2 relax passes"
    );
    let sweeps = substeps * (1 + relax);

    let span_ids: Vec<u16> = SPAN_ZONES.iter().map(|&h| zone_id(h)).collect();
    let counter_ids: Vec<u16> = COUNTER_ZONES.iter().map(|&h| zone_id(h)).collect();
    let systems: Vec<(&'static str, u16)> = scene.physics.system_zones().collect();
    let system_ids: Vec<u16> = systems.iter().map(|&(_, id)| id).collect();

    // One row per zone: an id shared by two zones would merge their counts, and a sum of two
    // rows can match neither expectation or, worse, happen to match one.
    let mut all_ids: Vec<u16> =
        span_ids.iter().chain(&counter_ids).chain(&system_ids).copied().collect();
    all_ids.sort_unstable();
    assert!(
        all_ids.windows(2).all(|w| w[0] != w[1]),
        "two zones share an id: {all_ids:?}"
    );
    assert_eq!(
        systems.len(),
        if SYSTEM_ZONES_COMPILED { scene.physics.len() } else { 0 },
        "every system is listed once, or none under a folded tier: {systems:?}"
    );

    let mut literal = [0u64; 6]; // build, gravity, warm, integrate, biased, relax over STEPS_OFF
    let mut reused_total = 0u64;
    // W8S: the S6 graph hits over the run (some step must hit).
    let mut s6_hits = 0u64;
    let parallel_solve = scene.world.resource::<PhysicsConfig>().parallel_solve;
    assert!(parallel_solve, "the harness requests the parallel solve");

    for step in 0..STEPS {
        let sleeping = step >= STEPS_OFF;
        scene.set_sleeping(sleeping);
        // The path is derived from the configuration, the row count and `brute_max_rows()`,
        // never from the implementation: tree-path steps, then one Tree step with
        // `brute_max_rows` above N (the brute loop), then one `AllPairs` step.
        let tree_path = step < STEPS_OFF + STEPS_ON;
        if step == STEPS_OFF + STEPS_ON {
            scene.world.resource_mut::<BroadphaseTree>().set_brute_max_rows(u32::MAX);
        }
        if step >= STEPS_OFF + STEPS_ON + STEPS_BRUTE {
            scene.set_broadphase(BroadphaseKind::AllPairs);
        }
        {
            let cfg = scene.world.resource::<PhysicsConfig>().broadphase;
            let brute = scene.world.resource::<BroadphaseTree>().brute_max_rows();
            let rows = scene.world.resource::<SolverScratch>().bodies_len() as u32;
            let derived = cfg == BroadphaseKind::Tree && (step == 0 || rows > brute);
            assert_eq!(derived, tree_path, "step {step}: the path derived from cfg / N / brute_max_rows");
        }

        let spans_before = snapshot(&scene, &span_ids);
        let counters_before = snapshot(&scene, &counter_ids);
        let systems_before = snapshot(&scene, &system_ids);
        let dispatches_before = scene.world.resource::<Manifolds>().narrowphase_dispatches();
        scene.step();
        scene.fold();
        let spans_after = snapshot(&scene, &span_ids);
        let counters_after = snapshot(&scene, &counter_ids);
        let systems_after = snapshot(&scene, &system_ids);

        let shape = step_shape(&scene);
        println!("step {step} (sleeping {sleeping}): {shape:?}");
        assert_ne!(
            shape.wide_colors, shape.narrow_colors,
            "step {step}: equal class counts cannot show a swapped classification: {shape:?}"
        );
        assert!(
            shape.wide_colors >= 1 && shape.narrow_colors >= 1,
            "step {step}: the scene must show a wide AND a narrow color, or one class is \
             checked against zero: {shape:?}"
        );
        {
            let rows = scene.world.resource::<SolverScratch>().bodies();
            let dynamic = rows.iter().filter(|b| b.inv_mass != 0.0).count();
            assert_eq!(
                dynamic,
                scene.boxes.len(),
                "step {step}: every spawned box is one dynamic row"
            );
            if sleeping {
                let sleep = scene.world.resource::<IslandSleep>();
                let awake = (0..rows.len())
                    .filter(|&r| rows[r].inv_mass != 0.0 && sleep.is_row_awake(r))
                    .count();
                assert_eq!(
                    awake, dynamic,
                    "step {step}: a frozen row would skip manifolds the recomputation counts"
                );
            }
        }

        let np_chunks = expected_np_chunks(parallel_np, shape.pairs, WORKERS);
        assert!(
            np_chunks >= 2,
            "step {step}: the scene must dispatch the narrowphase, or its zones are checked \
             against zero: {} pairs make {np_chunks} chunks",
            shape.pairs
        );
        assert_eq!(
            scene.world.resource::<Manifolds>().narrowphase_dispatches() - dispatches_before,
            1,
            "step {step}: one narrowphase dispatch per step"
        );
        let np = u64::from(np_chunks >= 2);

        // L9's pair classes close over the step (their non-box count recomputed from the shapes).
        let classes = scene.world.resource::<Manifolds>().pair_classes();
        let non_box = {
            let rows = scene.world.resource::<SolverScratch>().bodies();
            let is_box = |r: u32| matches!(rows[r as usize].shape, ColliderShape::Box { .. });
            scene
                .world
                .resource::<ContactPairs>()
                .pairs()
                .iter()
                .filter(|&&(a, b)| !(is_box(a.0) && is_box(b.0)))
                .count() as u64
        };
        assert_eq!(
            (classes.pairs, classes.non_box),
            (shape.pairs, non_box),
            "step {step}: the pair classes' pairs and non-box pairs are the step's ({classes:?})"
        );
        reused_total += classes.reused;
        assert_eq!(
            classes.full + classes.reused + classes.sep_hits + classes.non_box,
            shape.pairs,
            "step {step}: the pair classes close over the pairs ({classes:?})"
        );
        assert!(classes.full > 0, "step {step}: no box pair ran the full collision ({classes:?})");

        // The tree broadphase's structure: the harness scene's one static (the floor) is pending
        // at step 1 and admitted at step 2 — one rebuild there, one member from then on, every
        // other row queried. Recomputed from the row count and the step, not from `diag()`.
        let rows = scene.rows();
        assert_eq!(rows, scene.boxes.len() as u64 + 1, "step {step}: the floor and every box are rows");
        let tp = u64::from(tree_path);
        let bp_members = tp * u64::from(step >= 2);
        let bp_rebuilds = tp * u64::from(step == 2);
        let bp_queried = tp * (rows - bp_members);

        let on = u64::from(ZONES_COMPILED);
        let sl = u64::from(sleeping);
        let expected_spans: [(&ZoneHandle, u64); SPAN_ZONE_COUNT] = [
            (&PHYS_SOLVE_BUILD, 1),
            (&PHYS_GRAVITY, substeps),
            (&PHYS_WARM_APPLY, substeps),
            (&PHYS_INTEGRATE, substeps),
            (&PHYS_PASS_BIASED, substeps),
            (&PHYS_PASS_RELAX, substeps * relax),
            (&PHYS_COLOR_WIDE, shape.wide_colors * sweeps),
            (&PHYS_COLOR_NARROW, shape.narrow_colors * sweeps),
            (&PHYS_RESTITUTION, 1),
            (&PHYS_STORE, 1),
            (&PHYS_WRITE_BACK, 1),
            (&PHYS_SLEEP_BEGIN, sl),
            (&PHYS_SLEEP_FREEZE, 2 * sl),
            (&PHYS_SLEEP_END, sl),
            // L10: the harness keeps the default sleep-skip mode (Sets), so the broadphase runs
            // the sleep-skip's prologue on every sleeping step.
            (&PHYS_SLEEP_CLASSIFY, sl),
            (&PHYS_NP_DISPATCH, np),
            (&PHYS_NP_COMPACT, np),
            (&PHYS_NP_AXIS_COMMIT, np),
            (&PHYS_BP_VERIFY, tp),
            (&PHYS_BP_BUILD, tp),
            (&PHYS_BP_QUERY, tp),
            (&PHYS_BP_ASSEMBLE, tp),
            (&PHYS_SB_BODIES, 1),
            (&PHYS_SB_PA, 1),
            (&PHYS_SB_PB, 1),
            (&PHYS_SB_PC, 1),
        ];
        for (k, &(handle, want)) in expected_spans.iter().enumerate() {
            assert!(
                std::ptr::eq(handle, SPAN_ZONES[k]),
                "the expectation table follows SPAN_ZONES' order"
            );
            let got = spans_after[k].0 - spans_before[k].0;
            assert_eq!(
                got,
                want * on,
                "step {step} (sleeping {sleeping}): `{}` recorded {got} spans, the structure \
                 has {} ({shape:?})",
                name_of(handle),
                want * on
            );
        }

        // W8S: every wide colour of every sweep dispatches (parallel_solve on, four workers, a
        // wide colour on every step); the in-flight and lane readings lie in [1, W + 1].
        let waves = shape.wide_colors * sweeps;
        let tasks = shape.wide_tasks * sweeps;
        let threads = WORKERS as u64 + 1;
        // (samples per step, value): the ten step counters sample once per step; the three
        // tree counters once per tree-path step and never otherwise; the W8S rows as the table
        // in the module docs says. `None` is a value checked below, or a timing.
        let expected_counters: [(&ZoneHandle, u64, Option<u64>); COUNTER_ZONE_COUNT] = [
            (&PHYS_SLOTS_WIDE, 1, Some(shape.wide_slots)),
            (&PHYS_SLOTS_NARROW, 1, Some(shape.narrow_slots)),
            (&PHYS_NP_PAIRS, 1, Some(shape.pairs)),
            (&PHYS_NP_MANIFOLDS, 1, Some(shape.manifolds)),
            (&PHYS_NP_POINTS, 1, Some(shape.points)),
            (&PHYS_BP_PAIRS, 1, Some(shape.pairs)),
            (&PHYS_NP_CHUNKS, 1, Some(np_chunks)),
            (&PHYS_BP_QUERIED, tp, Some(bp_queried)),
            (&PHYS_BP_MEMBERS, tp, Some(bp_members)),
            (&PHYS_BP_REBUILDS, tp, Some(bp_rebuilds)),
            (&PHYS_NP_REUSED, 1, Some(classes.reused)),
            (&PHYS_NP_SEP_HITS, 1, Some(classes.sep_hits)),
            (&PHYS_NP_FULL, 1, Some(classes.full)),
            // L10: once per sleeping step (the classification's); no island of this scene is
            // held within its steps.
            (&PHYS_SLEEP_HELD, sl, Some(0)),
            (&PHYS_WAVE_RAMP, 1, None),
            (&PHYS_WAVE_TAIL, 1, None),
            (&PHYS_WAVE_INFLIGHT, 1, None),
            (&PHYS_WAVE_LANES, 1, None),
            (&PHYS_WAVE_OVERFLOW, 1, Some(0)),
            (&PHYS_COLOR_SCOPES, 1, Some(waves)),
            (&PHYS_COLOR_TASKS, 1, Some(tasks)),
            (&PHYS_ROUTE_WORKER, 1, None),
            (&PHYS_ROUTE_EXTERNAL, 1, None),
            (&PHYS_HIST_COLORS_LT32, 1, Some(shape.hist_colors[0])),
            (&PHYS_HIST_COLORS_LT64, 1, Some(shape.hist_colors[1])),
            (&PHYS_HIST_COLORS_LT128, 1, Some(shape.hist_colors[2])),
            (&PHYS_HIST_COLORS_LT256, 1, Some(shape.hist_colors[3])),
            (&PHYS_HIST_COLORS_GE256, 1, Some(shape.hist_colors[4])),
            (&PHYS_HIST_SLOTS_LT32, 1, Some(shape.hist_slots[0])),
            (&PHYS_HIST_SLOTS_LT64, 1, Some(shape.hist_slots[1])),
            (&PHYS_HIST_SLOTS_LT128, 1, Some(shape.hist_slots[2])),
            (&PHYS_HIST_SLOTS_LT256, 1, Some(shape.hist_slots[3])),
            (&PHYS_HIST_SLOTS_GE256, 1, Some(shape.hist_slots[4])),
            (&PHYS_S6_GRAPH_HIT, 1, None),
            (&PHYS_S6_PB_HIT, 1, None),
            (&PHYS_NP_WAVE_RAMP, np, None),
            (&PHYS_NP_WAVE_TAIL, np, None),
            (&PHYS_NP_WAVE_INFLIGHT, np, None),
            (&PHYS_NP_WAVE_LANES, np, None),
            (&PHYS_NP_WAVE_OVERFLOW, np, Some(0)),
            (&PHYS_WAVE_JOIN, 1, None),
            (&PHYS_WAVE_HELPED, 1, None),
            (&PHYS_WAVE_FIRST_RAMP, 1, None),
            (&PHYS_WAVE_FIRST_TAIL, 1, None),
            (&PHYS_WAVE_PASS_RAMP, 1, None),
            (&PHYS_NP_WAVE_JOIN, np, None),
            (&PHYS_NP_ROUTE_WORKER, np, None),
        ];
        let mut totals = [0u64; COUNTER_ZONE_COUNT];
        // Every mismatch of the step, reported together (module docs).
        let mut wrong: Vec<String> = Vec::new();
        for (k, &(handle, samples, value)) in expected_counters.iter().enumerate() {
            assert!(
                std::ptr::eq(handle, COUNTER_ZONES[k]),
                "the expectation table follows COUNTER_ZONES' order"
            );
            let count = counters_after[k].0 - counters_before[k].0;
            let total = counters_after[k].1 - counters_before[k].1;
            totals[k] = total;
            if count != samples * on {
                wrong.push(format!(
                    "step {step}: counter `{}` recorded {count} samples, the step has {}",
                    name_of(handle),
                    samples * on
                ));
            }
            if let Some(value) = value
                && total != value * samples * on
            {
                wrong.push(format!(
                    "step {step}: counter `{}` totalled {total}, the step has {value} a sample \
                     ({samples} samples)",
                    name_of(handle)
                ));
            }
        }
        // The W8S rows whose value is a range or a relation (module docs' table).
        let total_of = |h: &ZoneHandle| {
            totals[COUNTER_ZONES.iter().position(|&z| std::ptr::eq(z, h)).expect("a listed counter")]
        };
        for (h, n) in [
            (&PHYS_WAVE_INFLIGHT, waves),
            (&PHYS_WAVE_LANES, waves),
            (&PHYS_NP_WAVE_INFLIGHT, np),
            (&PHYS_NP_WAVE_LANES, np),
        ] {
            let t = total_of(h);
            if !(n * on..=n * threads * on).contains(&t) {
                wrong.push(format!(
                    "step {step}: `{}` totalled {t} over {n} waves, outside [1, W + 1] a wave",
                    name_of(h)
                ));
            }
        }
        let scopes = total_of(&PHYS_ROUTE_WORKER) + total_of(&PHYS_ROUTE_EXTERNAL);
        if scopes != waves * on {
            wrong.push(format!(
                "step {step}: the route counters sum to {scopes}, the solve scopes are {}",
                waves * on
            ));
        }
        // The review's relations (B1, B2, N3, N6): a tail is its imbalance plus its join, one
        // wave's ramp and tail are within the step's sums, a helped wave is a solve scope, and a
        // step no helper reached reads a ramp of 0.
        let (ramp, tail) = (total_of(&PHYS_WAVE_RAMP), total_of(&PHYS_WAVE_TAIL));
        let (first_ramp, first_tail) =
            (total_of(&PHYS_WAVE_FIRST_RAMP), total_of(&PHYS_WAVE_FIRST_TAIL));
        let (pass_ramp, helped) = (total_of(&PHYS_WAVE_PASS_RAMP), total_of(&PHYS_WAVE_HELPED));
        let (np_join, np_tail) = (total_of(&PHYS_NP_WAVE_JOIN), total_of(&PHYS_NP_WAVE_TAIL));
        let relations = [
            (total_of(&PHYS_WAVE_JOIN) <= tail, "phys_wave_join ≤ phys_wave_tail"),
            (first_tail <= tail, "phys_wave_first_tail ≤ phys_wave_tail"),
            (first_ramp <= pass_ramp && pass_ramp <= ramp, "first ramp ≤ pass ramp ≤ ramp"),
            (helped <= scopes, "phys_wave_helped ≤ the solve scopes"),
            (helped > 0 || ramp == 0, "no helped wave, yet a ramp"),
            (np_join <= np_tail, "phys_np_wave_join ≤ phys_np_wave_tail"),
            (total_of(&PHYS_NP_ROUTE_WORKER) <= np * on, "phys_np_route_worker is 0 or 1"),
        ];
        for (holds, what) in relations {
            if !holds {
                wrong.push(format!(
                    "step {step}: {what} fails: join {}, tail {tail}, ramp {ramp}, first ramp \
                     {first_ramp}, first tail {first_tail}, pass ramp {pass_ramp}, helped {helped} \
                     of {scopes} scopes, np join {np_join}, np tail {np_tail}, np route {}",
                    total_of(&PHYS_WAVE_JOIN),
                    total_of(&PHYS_NP_ROUTE_WORKER)
                ));
            }
        }
        assert!(wrong.is_empty(), "{} counter check(s) failed:\n{}", wrong.len(), wrong.join("\n"));
        let (g, pb) = (total_of(&PHYS_S6_GRAPH_HIT), total_of(&PHYS_S6_PB_HIT));
        assert!(g <= 1 && pb <= g, "step {step}: S6 hits (graph {g}, P-b {pb}) are 0/1, P-b under graph");
        if step == 0 {
            assert_eq!(g, 0, "the first armed step has no witness to hit");
        }
        s6_hits += g;

        for (k, &(name, _)) in systems.iter().enumerate() {
            let got = systems_after[k].0 - systems_before[k].0;
            assert_eq!(got, 1, "step {step}: system `{name}` recorded {got} spans, it ran once");
        }

        if !sleeping {
            for (k, slot) in literal.iter_mut().enumerate() {
                *slot += spans_after[k].0 - spans_before[k].0;
            }
        }
    }

    // The plan's literals over the three sleeping-off steps, in SPAN_ZONES' first six rows:
    // build, gravity, warm, integrate, biased, relax.
    let want = if ZONES_COMPILED { [3, 12, 12, 12, 12, 24] } else { [0; 6] };
    assert_eq!(literal, want, "the plan's 3-step literals");
    assert!(
        reused_total > 0,
        "no step reused a contact record (contact reuse is on by default since L9 C4), so the \
         reused counter was only ever checked against zero"
    );
    assert!(
        s6_hits > 0 || !ZONES_COMPILED,
        "S6's graph counter never hit on a resting scene, so it was only ever checked against zero"
    );
    // The whole session since the arm, not a sum of diffs: nothing reached the store before the
    // first step or between the per-step snapshots.
    assert_eq!(
        scene.lifetime(&PHYS_SOLVE_BUILD).count,
        STEPS as u64 * u64::from(ZONES_COMPILED),
        "one build span per step since the arm"
    );
    assert_eq!(
        scene.lifetime(&PHYS_BP_VERIFY).count,
        (STEPS_OFF + STEPS_ON) as u64 * u64::from(ZONES_COMPILED),
        "one verify span per tree-path step since the arm, none on the AllPairs step"
    );

    let drops = scene.profiler().drops();
    assert_eq!(drops.total(), 0, "the store dropped samples, so a count may be short: {drops:?}");
}
