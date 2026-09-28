//! S7 (the scaling plan's partial W16 lever, `docs/physics/perf-campaign/levers/scaling/01-DESIGN.md`
//! §6.7 and §13): the colored solve caps the threads one colour wave runs on, and nothing else.
//!
//! * [`the_cap_engages_where_the_pool_is_wider`]: at W16 under `AtMost(8)` the waves run capped
//!   on every step whose colours dispatch (T-S7-1's engagement half; its lanes bound needs the
//!   profiler, so it is `s7_lane_cap_armed`, a binary of its own).
//! * [`the_cap_is_inert_at_or_below_the_core_count`] (T-S7-2): `AtMost(8)` at W 2/4/8 and
//!   `Uncapped` at W16 never cap, while their colours do dispatch.
//! * [`capped_waves_are_bit_identical`] (T-S7-3): the final pose at W 1/2/4/8/16 under
//!   `Uncapped`, `AtMost(2)`, `AtMost(3)` and `AtMost(8)` equals the serial solve's, on J-T (the
//!   cohort-snapped walk) and on J-A (the scalar walk).
//! * [`the_engine_reads_a_physical_core_count`] (T-S7-4): the host receipt — physical, logical
//!   and source, printed — with a positive count, and at most 4096 when the OS answered.
//! * [`the_default_cap_engages_exactly_where_the_pool_exceeds_the_physical_cores`] (review W2):
//!   the shipped default, `PhysicalCores`, caps at W = 2 × physical (at most 64) and not at
//!   W = physical, on any host: the expectation is the formula `max(physical, 2) < W` over the
//!   engine's own read, never a constant, and this is the only gate on the default's resolution.
//!
//! Every test here pins its lane cap except the last, so none depends on the host's core count.

#[path = "support/s7_pyramid.rs"]
mod pyramid;

use std::num::NonZeroU16;

use boyko_physics::solver::{LaneCap, TopologySource};

use pyramid::{Pile, Row};

/// Steps per run: past the first contacts, short enough for a debug build of a W16 pool.
const STEPS: usize = 24;

fn at_most(n: u16) -> LaneCap {
    LaneCap::AtMost(NonZeroU16::new(n).expect("a positive cap"))
}

/// `STEPS` steps of `row` at `workers` under `cap`: the final pose hash, the steps that ran
/// capped, and the steps whose solve setup dispatched (a step whose colours dispatch too).
fn run(workers: usize, row: Row, parallel: bool, cap: LaneCap) -> (u64, u64, u64) {
    let mut pile = Pile::new(workers, row, parallel, cap);
    for _ in 0..STEPS {
        pile.step();
    }
    let s = pile.solver();
    (pile.pose_hash(), s.lane_capped_steps(), s.setup_dispatches())
}

#[test]
fn the_cap_engages_where_the_pool_is_wider() {
    let (_, capped, dispatched) = run(16, Row::JT, true, at_most(8));
    println!("S7 W16 AtMost(8): {capped} capped steps, {dispatched} setup dispatches of {STEPS}");
    assert!(dispatched > 0, "anti-vacuity: the pyramid's solve never dispatched at W16");
    assert!(
        capped >= dispatched && capped <= STEPS as u64,
        "S7: at W16 under AtMost(8) every step whose colours dispatch runs capped: {capped} capped, \
         {dispatched} setup dispatches, {STEPS} steps"
    );
}

#[test]
fn the_cap_is_inert_at_or_below_the_core_count() {
    for (workers, cap) in [(2, at_most(8)), (4, at_most(8)), (8, at_most(8)), (16, LaneCap::Uncapped)] {
        let (_, capped, dispatched) = run(workers, Row::JT, true, cap);
        assert!(dispatched > 0, "anti-vacuity: W{workers} never dispatched");
        assert_eq!(capped, 0, "S7: W{workers} under {cap:?} must never cap ({dispatched} dispatching steps)");
    }
}

#[test]
fn capped_waves_are_bit_identical() {
    let mut engaged = 0u64;
    for row in [Row::JT, Row::JA] {
        let (serial, _, _) = run(1, row, false, LaneCap::Uncapped);
        for workers in [1usize, 2, 4, 8, 16] {
            for cap in [LaneCap::Uncapped, at_most(2), at_most(3), at_most(8)] {
                let (hash, capped, _) = run(workers, row, true, cap);
                engaged += capped;
                assert_eq!(
                    hash, serial,
                    "S7: {row:?} at W{workers} under {cap:?} ({capped} capped steps): the pose must equal the \
                     serial solve's, bit for bit"
                );
            }
        }
    }
    assert!(engaged > 0, "anti-vacuity: no run ran a capped wave");
}

#[test]
fn the_engine_reads_a_physical_core_count() {
    let pile = Pile::new(2, Row::JT, true, LaneCap::PhysicalCores);
    let s = pile.solver();
    let (physical, logical, source) = (s.physical_cores(), s.logical_cores(), s.topology_source());
    println!("S7 host receipt: physical {physical}, logical {logical}, source {source:?}");
    assert!(physical >= 1 && logical >= 1, "S7: the read reports positive counts: {physical} / {logical}");
    if source == TopologySource::Os {
        assert!(physical <= 4096, "S7: an OS count past 4096 physical cores is a parse defect: {physical}");
    }
}

#[test]
fn the_default_cap_engages_exactly_where_the_pool_exceeds_the_physical_cores() {
    let physical = Pile::new(1, Row::JT, true, LaneCap::PhysicalCores).solver().physical_cores() as usize;
    let cap = physical.max(2);
    let mut dispatched_any = false;
    for workers in [physical.max(2), (2 * physical).clamp(2, 64)] {
        let (_, capped, dispatched) = run(workers, Row::JT, true, LaneCap::PhysicalCores);
        dispatched_any |= dispatched > 0;
        let expected = dispatched > 0 && cap < workers;
        println!("S7 default at W{workers} (physical {physical}): {capped} capped steps, {dispatched} dispatching");
        assert_eq!(
            capped > 0,
            expected,
            "S7: the default cap (PhysicalCores, physical {physical}) must engage exactly where the pool is \
             wider than max(physical, 2) = {cap}: W{workers}, {capped} capped steps"
        );
    }
    assert!(dispatched_any, "anti-vacuity: the pyramid never dispatched");
}
