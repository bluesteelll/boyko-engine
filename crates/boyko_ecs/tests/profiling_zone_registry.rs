//! `G11`, engine half — the engine zone registry refuses **non-terminally**, counts the refusal,
//! and reports it.
//!
//! # Why this is its own binary, and it is not a style choice
//!
//! `NEXT_SLOT` is process-global and monotone: there is no way to exhaust the registry and then
//! give the slots back. Run inside the crate's own test binary, this file would leave every later
//! `declare_zone!` and every later `try_build` in that process unable to mint — nine hundred tests
//! downstream of one that "just checks a warning". An integration test is a separate process, so
//! the exhaustion is this file's alone.
//!
//! The same fact makes the ORDER inside this file load-bearing: the registry can be exhausted
//! exactly once, so everything that needs a live registry runs before the test that spends it, and
//! that ordering is enforced by putting all of it in one `#[test]` rather than by hoping `libtest`
//! schedules them in the order they are written.

use boyko_diag::profiling_abi::{ENGINE_ZONE_SLOTS, ZONE_ID_EXHAUSTED, mint_id, minted_zones};
use boyko_ecs::ecs::core::profiling::{ArmOutcome, Profiler, ProfilerConfig, fold, report_count};

/// Exhaust the registry and assert what survives it.
///
/// One test, five claims, in the one order the process permits.
///
/// # The showable RED
///
/// Make the refusal terminal — `panic!` instead of returning `ZONE_ID_EXHAUSTED` in
/// `boyko_diag::profiling_abi::mint_id` — and claim 2 dies with the process. That is precisely the
/// behaviour this rung reversed: a legal app with a thousand systems across three schedules would
/// have panicked at build time on a default-on feature, and a missing *measurement* is not a wrong
/// *answer*. Run and confirmed at implementation.
#[test]
fn the_registry_refuses_without_taking_the_process_down() {
    let mut profiler = Profiler::new();
    assert!(matches!(
        profiler.arm(ProfilerConfig::default()),
        ArmOutcome::Armed | ArmOutcome::Rearmed
    ));

    // ── 1. A fresh mint is assigned, and ids are dense and increasing ──
    let first = mint_id();
    let second = mint_id();
    assert_ne!(first, ZONE_ID_EXHAUSTED, "a registry with room refused");
    assert_eq!(second, first + 1, "the registry hands out a dense range");
    assert!(minted_zones() >= second, "occupancy did not follow the mints");

    // ── 2. Exhaustion is NON-TERMINAL: the loop below must return, not abort ──
    let mut exhausted_at = None;
    for i in 0..=ENGINE_ZONE_SLOTS {
        if mint_id() == ZONE_ID_EXHAUSTED {
            exhausted_at = Some(i);
            break;
        }
    }
    let exhausted_at = exhausted_at.expect("the registry never refused within its own capacity");
    assert!(exhausted_at <= ENGINE_ZONE_SLOTS);

    // ── 3. Every later mint gets the SAME answer, so a caller cannot be handed a duplicate ──
    for _ in 0..8 {
        assert_eq!(mint_id(), ZONE_ID_EXHAUSTED, "an exhausted registry minted an id");
    }

    // ── 4. Occupancy saturates rather than reporting a figure above the capacity ──
    assert_eq!(
        minted_zones(),
        ENGINE_ZONE_SLOTS as u16,
        "occupancy climbed past the capacity it is compared against"
    );

    // ── 5. Both conditions REACH A READER. The substrate is mute, so they were raised as sticky
    //       bits; the fold is what turns them into codes, and nothing else in this process calls
    //       `take_raised`.
    fold(&mut profiler);
    assert!(
        report_count(9201) >= 1,
        "the registry was exhausted and boyko-W9201 was never emitted"
    );
    assert!(
        report_count(9208) >= 1,
        "the registry crossed 90 % occupancy and boyko-W9208 was never emitted — which is the \
         whole point of that code: exhaustion must not be the first news of it"
    );
}
