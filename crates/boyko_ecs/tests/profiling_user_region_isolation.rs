//! **Profiling rung 10, `G20` (authorship half) — a runaway GAME scope drops zero engine samples.**
//!
//! # The half that needed a second crate, and how it got one
//!
//! `crates/boyko_ecs/src/ecs/core/profiling/tests.rs`'s
//! `a_full_user_region_costs_the_engine_nothing` is `G20`'s **transport** half: it pushes raw
//! `Sample`s into `Region::User` until the ring refuses, and shows the engine's ring untouched. Its
//! own doc says the remaining half *"needs a second crate in one process and is that gate's by
//! construction"* — because what that test cannot show is that a **game's own zone site** writes
//! the user region. It pushes with the region as an argument; the property is that no site gets to
//! pass one.
//!
//! An integration test is its own crate root. The `profiling_partition!(User)` below is therefore a
//! real crate-level partition, `RUNAWAY` below is a real static game zone, and the region its
//! samples land in is chosen by the same `crate::__BOYKO_ZONE_PARTITION` any game would write.
//!
//! # The RED this pins
//!
//! `[B3-fix]`: key the region on the MACRO rather than on the declaring crate — that is, have
//! `ZoneGuard`'s `Drop` push to `Region::Engine` because it came from a `declare_zone!` — and the
//! runaway loop below fills the engine's ring instead of the game's, so the engine's own samples
//! are refused and `engine_overflow` moves. The assertion that it does **not** move is the gate.
//!
//! # ⚠️ Its own binary, and single-threaded within it
//!
//! The lane rings are process-global. This file claims one lane, drains every region before it
//! starts, and asserts about counters no other test in the binary may move — so it is the only
//! test here, exactly as its two sibling gates are the only tests in theirs.

use boyko_diag::lane::set_lane;
use boyko_diag::profiling_abi::{ZoneTier, arm_scope, disarm_scope};
use boyko_diag::sample::{self, Region, Sample, SampleKind};
use boyko_ecs::ecs::core::profiling::{ArmOutcome, Profiler, ProfilerConfig, fold};

boyko_diag::profiling_partition!(User);

// A real static game zone, declared the way a game declares one. (A `//` comment and not a `///`
// one: a macro INVOCATION is not an item, so a doc comment here documents nothing and warns.)
boyko_diag::declare_zone!(
    RUNAWAY,
    name = "game.runaway",
    scope = 34,
    tier = ZoneTier::Always
);

/// The lane this file owns. Distinct from other test files' by convention; they are separate
/// processes, so the only requirement is that it is a real lane index.
const TEST_LANE: u16 = 5;

/// How many engine samples the engine emits while the game runs away. Every one must survive.
const ENGINE_SAMPLES: u32 = 64;

/// An engine-region sample, pushed with the region named explicitly — this file cannot *declare* an
/// engine zone (it is a `User` crate, which is the point), so the engine's traffic is spelled at
/// the transport.
fn engine_sample(stamp: u64, value: u64) -> Sample {
    Sample { stamp, value, zone: 3, flags: SampleKind::Counter as u16, _pad: 0 }
}

/// **`G20`, authorship half.** A game's own static zone site overflows the game's ring, and the
/// engine loses nothing.
#[test]
fn a_runaway_game_zone_costs_the_engine_nothing() {
    set_lane(TEST_LANE);
    let mut p = Profiler::new();
    let outcome = p.arm(ProfilerConfig::default());
    assert!(
        matches!(outcome, ArmOutcome::Armed | ArmOutcome::Rearmed),
        "the canonical geometry must arm: {outcome:?}"
    );
    // The rings survive an `arm`; a sample left by an earlier run would be folded into this run's
    // frames by its stamp. Drain before measuring anything.
    //
    // SAFETY: this is the only test in this binary and it runs on one thread, so this thread is
    //   the process's only consumer of these regions for the duration of the call.
    unsafe {
        for lane in 0..boyko_diag::lane::LANE_COUNT {
            for region in [Region::Engine, Region::User] {
                sample::drain_region(lane, region, |_| {});
            }
        }
    }
    fold(&mut p);
    let before = p.drops();

    arm_scope(34);

    // ---- The runaway. A game zone, opened and closed in a loop, past the ring's capacity. ----
    //
    // The loop is bounded rather than "until it overflows": an unbounded loop that never overflows
    // is a hang, and a hang is not a red.
    let mut opened = 0u64;
    for _ in 0..(boyko_diag::profile::REGION_CAPACITY * 4) {
        let g = boyko_diag::zone!(RUNAWAY);
        assert!(g.is_some(), "an armed zone on an armed scope must open");
        opened += 1;
        drop(g);
    }

    // ---- The engine's traffic, in the same window. ----
    // Stamped from the real clock, exactly as the game zone's guard stamps itself. `begin_of_row`
    // is `pub(crate)` and out of reach here — which is the better outcome anyway: both sides of
    // this comparison now go through the same clock the engine actually uses, so nothing about the
    // attribution is special-cased for the test.
    let mut engine_accepted = 0u32;
    for i in 0..ENGINE_SAMPLES {
        let stamp = boyko_diag::clock::ticks();
        if sample::push(Region::Engine, engine_sample(stamp, u64::from(i))) {
            engine_accepted += 1;
        }
    }

    fold(&mut p);
    let after = p.drops();

    println!(
        "G20 authorship half: {opened} game zone closes, {engine_accepted}/{ENGINE_SAMPLES} \
         engine samples accepted; user_overflow +{}, engine_overflow +{}",
        after.user_overflow - before.user_overflow,
        after.engine_overflow - before.engine_overflow,
    );

    assert!(
        after.user_overflow > before.user_overflow,
        "the runaway did not overflow the USER ring, so this run tested nothing. It closed \
         {opened} zones against a region capacity of {}.",
        boyko_diag::profile::REGION_CAPACITY
    );
    assert_eq!(
        engine_accepted, ENGINE_SAMPLES,
        "the engine's ring refused {} of its own samples while a GAME was running away",
        ENGINE_SAMPLES - engine_accepted
    );
    assert_eq!(
        after.engine_overflow, before.engine_overflow,
        "a game's overflow reached the ENGINE's counter. The region is supposed to be a property \
         of the declaring crate; if this fires, a game's static zone site is writing the engine's \
         ring and a mod can silence the engine's own measurements."
    );

    disarm_scope(34);
}
