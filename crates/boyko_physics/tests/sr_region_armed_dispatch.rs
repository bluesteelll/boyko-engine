//! `armed_region_dispatches_follow_the_profiler_arm`: the solve region runs its `ARMED = true`
//! monomorph exactly when the profiler is armed — MD2, the SR-B close test's carry-over
//! (orchestrator ruling 32).
//!
//! # Why the gate exists
//!
//! `solve_region` picks the region's monomorph once per region from
//! `zone_enabled!(PHYS_REGION_OPENS)`. An `ARMED = true` region on a disarmed step pushes no
//! sample (every zone it opens is gated again) and changes no simulation bit, so a selector that
//! always answers `true` — SR-B's surviving mutation MD2, `core::hint::black_box(true)` in place of
//! the `zone_enabled!` read — passed every gate there was, the disarmed codegen receipt included:
//! the receipt proves what the `ARMED = false` monomorph contains, and under MD2 both monomorphs
//! are still built and referenced. The lane record is
//! `docs/physics/perf-campaign/levers/scaling/03-SR-POLICY-RECORD.md`.
//!
//! The witness is `boyko_physics::solver::colored::armed_region_dispatches`, a process-wide count
//! that exists only under the `region-armed-counts` feature. The solver's `item_begin` hook adds
//! one on item 0, and the region protocol calls that hook only from an `ARMED = true` monomorph
//! (`boyko_threadpool::RegionStages::item_begin`), so the count follows the monomorph that ran,
//! not the boolean that chose it.
//!
//! # What is asserted
//!
//! The shared scene (`support/profiling_harness.rs`: the real `add_physics_colored_solve` schedule
//! on a 4-worker pool, `parallel_solve` on, a wide floor colour), one world. Per step, `a` is the
//! rise of the `ARMED = true` count and `r` the rise of the solver's `region_dispatches`.
//!
//! 1. **Disarmed**: [`DISARMED_STEPS`] steps while nothing in the process has ever armed
//!    (asserted). On every step `a == 0`; on every step from [`WARM_STEPS`] on `r == 1`, so the
//!    disarmed clause is not vacuous (the floor static is admitted at step 2, harness docs).
//! 2. **Armed**: [`ARMED_STEPS`] steps after `arm_profiler`, each folded. On every step `r == 1`
//!    and `a == r`.
//!
//! Under a profile whose tier folds the physics zones, `zone_enabled!` reads `false` while armed
//! too, so every region runs `ARMED = false`, and the armed half asserts `a == 0` instead.
//!
//! # What it catches
//!
//! * MD2 (the selector reads `black_box(true)`) and MD (the selector is the literal `true`): a
//!   disarmed step runs `ARMED = true`, `a = 1`.
//! * MA (the two arms' const arguments swapped) and the half swap (both arms `true`): the same
//!   disarmed red; under MA an armed step also reads `a = 0` against `r = 1`.
//! * The increment deleted (the counter's own anti-vacuity): armed step 0 reads `a = 0`, `r = 1`.
//!
//! # Shown red (sr-policy C1, 2026-10-15: each mutation alone in the lane's tree, then reverted)
//!
//! * MD2, debug and release; MD, MA and the half swap, debug: `disarmed step 0: a = 1, r = 1: a
//!   disarmed step dispatched the ARMED = true region monomorph`.
//! * The increment deleted, debug: `armed step 0: a = 0, r = 1: the armed step's regions did not
//!   run the ARMED = true monomorph`.
//!
//! # How to run
//!
//! ```text
//! cargo test -p boyko-physics --features region-armed-counts --test sr_region_armed_dispatch -- --nocapture
//! ```
//!
//! `required-features` (see `Cargo.toml`): without the feature cargo refuses the named target
//! rather than running nothing. `harness = false`: the arm mask is process-wide, so the test is the
//! only thing in its process (see the harness module). Counts, not timings: it needs no quiet
//! machine.

// `dead_code`: the harness serves every profiling validation binary, and this one calls only its
// runner, the scene, a step, the arm and the fold.
#[path = "support/profiling_harness.rs"]
#[allow(dead_code)]
mod harness;

use boyko_diag::profiling_abi::any_armed;
use boyko_physics::profiling::ZONES_COMPILED;
use boyko_physics::solver::ColoredSoftStepSolver;
use boyko_physics::solver::colored::armed_region_dispatches;

use harness::{Scene, run_single_test};

/// The name libtest would list this test under.
const TEST_NAME: &str = "armed_region_dispatches_follow_the_profiler_arm";
/// Steps of the disarmed phase.
const DISARMED_STEPS: usize = 12;
/// Disarmed steps before every step must open a region: the floor static is pending at step 1 and
/// admitted at step 2, so from step 4 every step has the wide floor colour.
const WARM_STEPS: usize = 4;
/// Steps of the armed phase.
const ARMED_STEPS: usize = 8;

fn main() {
    run_single_test(TEST_NAME, armed_region_dispatches_follow_the_profiler_arm);
}

/// The solve regions the scene's solver has run since it was built.
fn region_dispatches(scene: &Scene) -> u64 {
    scene.world.resource::<ColoredSoftStepSolver>().region_dispatches()
}

/// One step, as `(a, r)`: the `ARMED = true` regions it ran and the regions it ran.
fn counted_step(scene: &mut Scene) -> (u64, u64) {
    let (armed_before, regions_before) = (armed_region_dispatches(), region_dispatches(scene));
    scene.step();
    (armed_region_dispatches() - armed_before, region_dispatches(scene) - regions_before)
}

fn armed_region_dispatches_follow_the_profiler_arm() {
    // ── 1. Disarmed ──
    assert!(!any_armed(), "the disarmed phase must start before anything in the process armed");
    let mut scene = Scene::spawn();
    let (mut disarmed_regions, mut armed_regions, mut armed_count) = (0u64, 0u64, 0u64);
    for step in 0..DISARMED_STEPS {
        let (a, r) = counted_step(&mut scene);
        disarmed_regions += r;
        assert_eq!(
            a, 0,
            "disarmed step {step}: a = {a}, r = {r}: a disarmed step dispatched the ARMED = true \
             region monomorph"
        );
        if step >= WARM_STEPS {
            assert_eq!(
                r, 1,
                "disarmed step {step}: a = {a}, r = {r}: the step did not open exactly one region, \
                 so the disarmed clause proves nothing"
            );
        }
    }
    assert!(!any_armed(), "nothing armed during the disarmed phase");

    // ── 2. Armed ──
    scene.arm_profiler();
    for step in 0..ARMED_STEPS {
        let (a, r) = counted_step(&mut scene);
        scene.fold();
        (armed_regions, armed_count) = (armed_regions + r, armed_count + a);
        assert_eq!(r, 1, "armed step {step}: a = {a}, r = {r}: the step did not open exactly one region");
        let want = if ZONES_COMPILED { r } else { 0 };
        assert_eq!(
            a, want,
            "armed step {step}: a = {a}, r = {r}: the armed step's regions did not run the \
             ARMED = {ZONES_COMPILED} monomorph"
        );
    }
    println!(
        "{TEST_NAME}: {DISARMED_STEPS} disarmed steps ran {disarmed_regions} regions, 0 of them \
         ARMED = true; {ARMED_STEPS} armed steps ran {armed_regions} regions, {armed_count} of them \
         ARMED = true"
    );
    if !ZONES_COMPILED {
        println!(
            "{TEST_NAME}: this profile's tier folds the physics zones, so an armed step runs the \
             ARMED = false monomorph; the armed phase asserted a = 0"
        );
    }
}
