//! S7 T-S7-1, the lanes bound: at W16 under `LaneCap::AtMost(8)` — an 8-core machine's default —
//! a colour wave runs on at most 8 threads, and its chunks are exactly the uncapped walk's.
//!
//! Read through the W8S instrument, which is the only observer of the threads a wave ran on: per
//! step, `phys_wave_lanes` (the distinct threads that ran a stamped task, summed over the step's
//! solve waves) is at most `8 × phys_color_scopes`, plus `W + 1` for the setup wave, which S7 does
//! not cap; and `phys_color_tasks` equals the tasks the solver's cut walk makes at W16,
//! recomputed here from the constraint graph and the manifolds (a second derivation, as
//! `profiling_zone_counts` does it) — a capped wave runs the uncapped walk's chunks, each once.
//! The cap must also be what ran: `lane_capped_steps` rises on every step whose colours
//! dispatched.
//!
//! **Why a binary of its own.** The profiler's arm mask, lane rings, zone accumulators and world
//! bind are process-wide (`support/profiling_harness.rs`), so this binary holds exactly one test
//! and binds exactly one world. It is `harness = true` like every auto-discovered test; with a
//! single test nothing else in the process pushes a sample. The body runs on its own thread under
//! a watchdog, so a hung schedule fails rather than stalls the run.
//!
//! **It can fail.** A cap bounds a wave's lanes only where the wave has more chunks than the cap,
//! and the test requires that on every measured step (`tasks > cap × scopes`), or the bound would
//! hold uncapped too. In release the pile is Jolt's height 15 and a wave carries about 23 chunks
//! (window 8's 23.4), so the cap is 8. A debug build scales the pile to height 10, whose waves carry
//! about 6.6 chunks — fewer than 8, so a bound at 8 could not fail there — and the cap is 4. With
//! the cap removed (`LaneCap::Uncapped`) the bound reds on a host with more logical CPUs than the
//! cap, where the Chase-Lev pool lets every idle worker steal a chunk (window 8 read 15.39 lanes a
//! wave at W16 on the owner's 8-core / 16-thread host); on a smaller host it holds either way, and
//! the engagement and task assertions still gate.
//!
//! Under a profile whose tier folds the physics zones, every counter reads zero, and that is what
//! is asserted instead.

#[path = "support/s7_pyramid.rs"]
mod pyramid;

use std::num::NonZeroU16;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use boyko_diag::profiling_abi::zone_id;
use boyko_ecs::ecs::core::profiling::{ArmOutcome, Profiler, ProfilerConfig, bind_world, fold_frame};
use boyko_physics::profiling::{
    PHYS_COLOR_SCOPES, PHYS_COLOR_TASKS, PHYS_WAVE_LANES, WIDE_COLOR_MIN_SLOTS, ZONES_COMPILED,
};
use boyko_physics::resources::{ConstraintGraph, Manifolds, PhysicsConfig};
use boyko_physics::solver::LaneCap;

use pyramid::{Pile, Row};

/// The pool width and the cap under test (module docs, "It can fail").
const WORKERS: usize = 16;
const CAP: u16 = if cfg!(debug_assertions) { 4 } else { 8 };
/// Unarmed steps first (the pile's first contacts), then the measured ones.
const WARM: usize = 4;
const MEASURED: usize = 6;
/// The solver's colour cut constants (`solver/colored.rs`), copied: the task count below is a
/// second derivation, not a read of the solver.
const COLOR_CHUNKS_PER_WORKER: usize = 6;
const COLOR_MIN_SLOTS_PER_CHUNK: usize = 64;
const COLOR_COHORT: usize = 8;

/// The tasks one dispatched colour spawns at `lanes` workers, its laid-out groups holding
/// `groups` points each in order (`profiling_zone_counts`'s copy of the cut walk).
fn expected_color_tasks(groups: &[u32], lanes: usize, simd_solve: bool) -> u64 {
    let n_groups = groups.len();
    if n_groups == 0 {
        return 0;
    }
    let total: usize = groups.iter().map(|&g| g as usize).sum();
    let by_work = (total / COLOR_MIN_SLOTS_PER_CHUNK).max(1);
    let n_chunks = (lanes * COLOR_CHUNKS_PER_WORKER).min(by_work).clamp(1, n_groups);
    if n_chunks < 2 {
        return 0;
    }
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

/// The wide colours' tasks of one sweep, recomputed from the graph and the manifolds.
fn sweep_tasks(pile: &Pile) -> u64 {
    let graph = pile.world.resource::<ConstraintGraph>();
    let manifolds = pile.world.resource::<Manifolds>().solver_manifolds();
    let simd_solve = pile.world.resource::<PhysicsConfig>().simd_solve;
    let mut tasks = 0;
    for c in 0..graph.n_colors() {
        let groups: Vec<u32> = graph
            .color(c)
            .iter()
            .map(|&mi| u32::from(manifolds[mi as usize].count))
            .filter(|&n| n != 0)
            .collect();
        if groups.iter().sum::<u32>() >= WIDE_COLOR_MIN_SLOTS {
            tasks += expected_color_tasks(&groups, WORKERS, simd_solve);
        }
    }
    tasks
}

fn lanes_bound_holds_and_the_chunks_are_the_uncapped_walks() {
    let cap = LaneCap::AtMost(NonZeroU16::new(CAP).expect("a positive cap"));
    let mut pile = Pile::new(WORKERS, Row::JT, true, cap);
    for _ in 0..WARM {
        pile.step();
    }
    bind_world(pile.world.world_id().get()).expect("this binary binds exactly one world");
    pile.world.insert_resource(Profiler::new());
    assert_eq!(pile.world.resource_mut::<Profiler>().arm(ProfilerConfig::default()), ArmOutcome::Armed);
    let (substeps, relax) = {
        let cfg = pile.world.resource::<PhysicsConfig>();
        (u64::from(cfg.substeps), u64::from(cfg.relax_iterations))
    };
    let sweeps = substeps * (1 + relax);
    let ids = [zone_id(&PHYS_WAVE_LANES), zone_id(&PHYS_COLOR_SCOPES), zone_id(&PHYS_COLOR_TASKS)];
    let totals = |pile: &Pile| ids.map(|id| pile.world.resource::<Profiler>().lifetime(id).map_or(0, |a| a.total));
    let mut dispatching = 0u64;
    for step in 0..MEASURED {
        let before = totals(&pile);
        let (capped0, setup0) = (pile.solver().lane_capped_steps(), pile.solver().setup_dispatches());
        pile.step();
        fold_frame(&mut pile.world);
        let after = totals(&pile);
        let [lanes, scopes, tasks] = [0, 1, 2].map(|i| after[i] - before[i]);
        let capped = pile.solver().lane_capped_steps() - capped0;
        let setup = pile.solver().setup_dispatches() - setup0;
        let want_tasks = sweep_tasks(&pile) * sweeps;
        println!(
            "step {step}: lanes {lanes}, colour scopes {scopes}, tasks {tasks} (recomputed {want_tasks}), \
             setup dispatched {setup}, capped {capped}"
        );
        if !ZONES_COMPILED {
            assert_eq!([lanes, scopes, tasks], [0, 0, 0], "step {step}: a folded tier records nothing");
            continue;
        }
        assert!(
            scopes == 0 || tasks > u64::from(CAP) * scopes,
            "anti-vacuity, step {step}: {tasks} chunks over {scopes} colour scopes are not more than {CAP} a \
             wave, so the lanes bound could not fail"
        );
        let bound = u64::from(CAP) * scopes + (WORKERS as u64 + 1) * setup;
        assert!(
            lanes <= bound,
            "step {step}: the waves ran on {lanes} lanes, above {CAP} per colour scope ({scopes}) plus {} for \
             the setup wave ({setup}) = {bound}: the cap did not engage",
            WORKERS + 1
        );
        assert_eq!(
            tasks, want_tasks,
            "step {step}: the capped waves ran {tasks} chunks, the uncapped walk at W{WORKERS} cuts {want_tasks}"
        );
        if scopes > 0 {
            dispatching += 1;
            assert_eq!(capped, 1, "step {step}: a step whose colours dispatched at W{WORKERS} ran capped");
        }
    }
    assert!(
        dispatching > 0 || !ZONES_COMPILED,
        "anti-vacuity: no measured step dispatched a colour wave"
    );
    assert!(pile.solver().lane_capped_steps() > 0, "S7: the cap never engaged");
}

#[test]
fn the_cap_engages_at_w16_on_an_8_core_machine() {
    let timeout = Duration::from_secs(if cfg!(debug_assertions) { 600 } else { 180 });
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("s7-armed".into())
        .spawn(move || {
            // The fold's own span is pushed from this thread: give it a lane, as the profiling
            // harness does.
            let _ = boyko_diag::lane::claim_lane();
            let outcome = std::panic::catch_unwind(lanes_bound_holds_and_the_chunks_are_the_uncapped_walks);
            let _ = tx.send(outcome.is_ok());
        })
        .expect("the test thread spawns");
    match rx.recv_timeout(timeout) {
        Ok(passed) => assert!(passed, "S7 T-S7-1 failed (its panic message is above)"),
        Err(_) => panic!("S7 T-S7-1: no result within {timeout:?}; a hung schedule reads as a failure"),
    }
}
