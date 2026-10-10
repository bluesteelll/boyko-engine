//! L6 check 5 — `boyko-W0501` fires when the query-type table crosses three quarters full.
//!
//! # Why this is its own integration test, and not a unit test beside the emitter
//!
//! `register_new()` mints from a **process-global** counter that is never recycled, and this test
//! burns 768 of its 1024 slots to reach the high-water mark. Run inside `boyko_ecs`'s lib-test
//! binary that would push every sibling test's `world.query::<D, F>()` towards the exhaustion
//! panic; run as its own integration binary it burns slots nothing else in the process wants.
//!
//! Its twin, `l6_query_table_exhaustion.rs`, is a **second** binary for the same reason at one
//! remove: it drives the counter past the cap, which is a one-way transition, so the two claims
//! cannot share a process whatever order the harness picks.
//!
//! # What this observes, and what it cannot
//!
//! It observes the real trigger — 768 genuine mints, the site's own arithmetic, the emission — and
//! that the record reached a destination. It cannot observe the rendered TEXT: no sink destination
//! is configured here, so the drain counts the record and writes it nowhere. The text is
//! `boyko_log`'s to gate, and it does, in `record.rs`'s walker tests.

use boyko_ecs::ecs::core::iters::query::query_type_registry::{MAX_QUERY_TYPES, register_new};
use boyko_log::level::Level;
use boyko_log::target::{LogTarget, set_target_level, target_stats};

/// `delivered + sync_routed`.
///
/// A test thread may or may not hold a diagnostics lane: with one the record lands in the ring and
/// the drain counts it as `delivered`, without one a `Warn` takes the synchronous channel and is
/// counted as `sync_routed` instead. Asserting only `delivered` would be green or red depending on
/// which harness thread ran the test, which is a coin flip dressed as a gate.
fn observed(id: boyko_log::TargetId) -> u64 {
    let s = target_stats(id);
    s.0 + s.3
}

#[test]
fn w0501_fires_when_the_query_table_crosses_three_quarters() {
    let id = <boyko_log::Query as LogTarget>::ID;
    set_target_level(id, Level::Trace);

    let high_water = MAX_QUERY_TYPES / 4 * 3;
    assert!(high_water > 0 && high_water < MAX_QUERY_TYPES, "the mark must be inside the table");

    // Mint up to one short of the mark and confirm SILENCE first. Without this leg the test would
    // pass for a site that warned on every mint, which is the failure mode a threshold has.
    for _ in 0..high_water - 1 {
        let _ = register_new();
    }
    for _ in 0..64 {
        if boyko_log::lifecycle::drain_once().is_some() {
            break;
        }
        std::thread::yield_now();
    }
    let before = observed(id);

    // The mint that crosses the line.
    let _ = register_new();
    for _ in 0..64 {
        if boyko_log::lifecycle::drain_once().is_some() {
            break;
        }
        std::thread::yield_now();
    }
    let after = observed(id);
    assert!(after > before, "boyko-W0501 never fired at {high_water} of {MAX_QUERY_TYPES}");

    // `RatePolicy::Once`, per site: the mark is crossed exactly once, so further mints are silent
    // even though the table keeps filling. The magnitude a reader wants is the occupancy, not a
    // line per slot.
    let at_mark = observed(id);
    for _ in 0..16 {
        let _ = register_new();
    }
    for _ in 0..64 {
        if boyko_log::lifecycle::drain_once().is_some() {
            break;
        }
        std::thread::yield_now();
    }
    assert_eq!(observed(id), at_mark, "the Once latch let a second W0501 through");
}
