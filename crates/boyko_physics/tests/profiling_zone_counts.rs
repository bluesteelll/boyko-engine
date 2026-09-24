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
//! | every system of the schedule (its `SystemSpan`) | 1 |
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
//! the four classes sum to the pairs, and with contact reuse off (the harness) nothing is reused.
//!
//! The narrowphase's chunk count is recomputed from the pair count, the pool's worker count and the
//! exported `NP_*` constants — with the `lanes < 2` term, as the engine's rule has it — and the
//! scene must dispatch on every step (at least two chunks), or the three L5 zones would be checked
//! against zero. `Manifolds::narrowphase_dispatches` must rise by one per step as well.
//!
//! Also asserted: the physics zones' ids are distinct from each other and from every system's,
//! so no two rows are one row; and the store reports no dropped sample.
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
    COUNTER_ZONE_COUNT, COUNTER_ZONES, PHYS_BP_ASSEMBLE, PHYS_BP_BUILD, PHYS_BP_MEMBERS,
    PHYS_BP_PAIRS, PHYS_BP_QUERIED, PHYS_BP_QUERY, PHYS_BP_REBUILDS, PHYS_BP_VERIFY,
    PHYS_COLOR_NARROW, PHYS_COLOR_WIDE, PHYS_GRAVITY, PHYS_INTEGRATE, PHYS_NP_AXIS_COMMIT,
    PHYS_NP_CHUNKS, PHYS_NP_COMPACT, PHYS_NP_DISPATCH, PHYS_NP_FULL, PHYS_NP_MANIFOLDS,
    PHYS_NP_PAIRS, PHYS_NP_POINTS, PHYS_NP_REUSED, PHYS_NP_SEP_HITS, PHYS_PASS_BIASED,
    PHYS_PASS_RELAX, PHYS_RESTITUTION, PHYS_SLEEP_BEGIN, PHYS_SLEEP_CLASSIFY,
    PHYS_SLEEP_END, PHYS_SLEEP_FREEZE, PHYS_SLEEP_HELD, PHYS_SLOTS_NARROW, PHYS_SLOTS_WIDE,
    PHYS_SOLVE_BUILD,
    PHYS_STORE, PHYS_WARM_APPLY, PHYS_WRITE_BACK, SPAN_ZONE_COUNT, SPAN_ZONES,
    WIDE_COLOR_MIN_SLOTS, ZONES_COMPILED,
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
}

/// Recomputes this step's color classes and narrowphase output from the graph and manifold
/// resources — what the solve was handed, not what it built.
fn step_shape(scene: &Scene) -> StepShape {
    let graph = scene.world.resource::<ConstraintGraph>();
    let manifolds = scene.world.resource::<Manifolds>().solver_manifolds();
    let mut shape = StepShape {
        wide_colors: 0,
        narrow_colors: 0,
        wide_slots: 0,
        narrow_slots: 0,
        pairs: scene.world.resource::<ContactPairs>().pairs().len() as u64,
        manifolds: manifolds.len() as u64,
        points: manifolds.iter().map(|m| u64::from(m.count)).sum(),
    };
    for c in 0..graph.n_colors() {
        let slots: u32 = graph
            .color(c)
            .iter()
            .map(|&mi| u32::from(manifolds[mi as usize].count))
            .sum();
        if slots >= WIDE_COLOR_MIN_SLOTS {
            shape.wide_colors += 1;
            shape.wide_slots += u64::from(slots);
        } else {
            shape.narrow_colors += 1;
            shape.narrow_slots += u64::from(slots);
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
            (classes.pairs, classes.non_box, classes.reused),
            (shape.pairs, non_box, 0),
            "step {step}: the pair classes' pairs and non-box pairs are the step's, and with              contact reuse off nothing is reused ({classes:?})"
        );
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

        // (samples per step, value): the ten step counters sample once per step; the three
        // tree counters once per tree-path step and never otherwise.
        let expected_counters: [(&ZoneHandle, u64, u64); COUNTER_ZONE_COUNT] = [
            (&PHYS_SLOTS_WIDE, 1, shape.wide_slots),
            (&PHYS_SLOTS_NARROW, 1, shape.narrow_slots),
            (&PHYS_NP_PAIRS, 1, shape.pairs),
            (&PHYS_NP_MANIFOLDS, 1, shape.manifolds),
            (&PHYS_NP_POINTS, 1, shape.points),
            (&PHYS_BP_PAIRS, 1, shape.pairs),
            (&PHYS_NP_CHUNKS, 1, np_chunks),
            (&PHYS_BP_QUERIED, tp, bp_queried),
            (&PHYS_BP_MEMBERS, tp, bp_members),
            (&PHYS_BP_REBUILDS, tp, bp_rebuilds),
            (&PHYS_NP_REUSED, 1, classes.reused),
            (&PHYS_NP_SEP_HITS, 1, classes.sep_hits),
            (&PHYS_NP_FULL, 1, classes.full),
            // L10: once per sleeping step (the classification's); no island of this scene is
            // held within its steps.
            (&PHYS_SLEEP_HELD, sl, 0),
        ];
        for (k, &(handle, samples, value)) in expected_counters.iter().enumerate() {
            assert!(
                std::ptr::eq(handle, COUNTER_ZONES[k]),
                "the expectation table follows COUNTER_ZONES' order"
            );
            let count = counters_after[k].0 - counters_before[k].0;
            let total = counters_after[k].1 - counters_before[k].1;
            assert_eq!(
                (count, total),
                (samples * on, value * samples * on),
                "step {step}: counter `{}` recorded (samples, total) = ({count}, {total}), the \
                 step has ({samples}, {value})",
                name_of(handle)
            );
        }

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
