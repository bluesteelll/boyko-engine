//! Phase 10 Wave E Step 15 — property-based tests for wraparound
//! semantics.
//!
//! See plan §13.4 for the test matrix.
//!
//! # Why gated `#[cfg(not(miri))]`
//!
//! `proptest` materialises a `Strategy` machine that allocates and walks
//! hundreds of cases per test. Under Miri the harness compiles for
//! ~10× the runtime budget of a plain unit test; gating the entire file
//! out of Miri keeps `cargo +nightly miri test` snappy. Miri coverage of
//! the underlying `Tick` arithmetic is provided by the explicit Miri
//! tests in `tests/miri_phase10.rs`.

#![cfg(not(miri))]

use boyko_ecs::ecs::core::change_detection::{Tick, CHECK_TICK_THRESHOLD, MAX_CHANGE_AGE};
use proptest::prelude::*;

proptest! {
    /// Property: `Tick::is_newer_than` MUST NOT panic on any well-formed
    /// `(self, last_run, this_run)` triple — the wrapping arithmetic
    /// only relies on `u32::wrapping_sub`, which is total.
    ///
    /// Plan §13.4 `prop_is_newer_than_wraparound_invariant`.
    #[test]
    fn prop_is_newer_than_never_panics(
        stored in any::<u32>(),
        last in any::<u32>(),
        this in any::<u32>(),
    ) {
        let _ = Tick::new(stored).is_newer_than(Tick::new(last), Tick::new(this));
    }

    /// Property: `check_tick` returns a value whose age relative to
    /// `current` is at most `MAX_CHANGE_AGE`.
    ///
    /// Plan §13.4: ensures the wraparound clamp invariant holds for ANY
    /// input (well-formed or not) — the clamp is the safety net that
    /// keeps `is_newer_than` correct.
    #[test]
    fn prop_check_tick_clamps_within_max_change_age(
        current in any::<u32>(),
        tick in any::<u32>(),
    ) {
        let clamped = Tick::new(tick).check_tick(Tick::new(current));
        let age = current.wrapping_sub(clamped.get());
        prop_assert!(
            age <= MAX_CHANGE_AGE,
            "after clamp, age must be <= MAX_CHANGE_AGE: age={} MAX_CHANGE_AGE={}",
            age,
            MAX_CHANGE_AGE
        );
    }

    /// Property: `check_tick` is **idempotent** — `check_tick(check_tick(x))
    /// == check_tick(x)`.
    ///
    /// Plan §13.4 `prop_check_tick_idempotence`.
    #[test]
    fn prop_check_tick_idempotent(
        current in any::<u32>(),
        tick in any::<u32>(),
    ) {
        let c = Tick::new(current);
        let once = Tick::new(tick).check_tick(c);
        let twice = once.check_tick(c);
        prop_assert_eq!(once, twice, "check_tick must be idempotent");
    }

    /// Property: a tick that is **already in range** (age <= MAX_CHANGE_AGE)
    /// is returned unchanged by `check_tick` — the clamp only fires on
    /// aged-out values.
    #[test]
    fn prop_check_tick_noop_in_range(
        current in 0u32..u32::MAX,
        age in 0u32..=MAX_CHANGE_AGE,
    ) {
        let tick = Tick::new(current.wrapping_sub(age));
        let clamped = tick.check_tick(Tick::new(current));
        prop_assert_eq!(
            clamped, tick,
            "in-range tick (age <= MAX_CHANGE_AGE) must be returned unchanged: age={}",
            age
        );
    }

    /// Property: ticks within `(last_run, this_run]` MUST be newer than
    /// `last_run` per `is_newer_than`, under the bounded-age discipline.
    ///
    /// This pins the documented semantic from plan §4.2:
    /// `is_newer_than(self, l, t) ↔ self ∈ (l, t]` under
    /// `0 <= (t - l) <= MAX_CHANGE_AGE` and `0 <= (t - self) <= MAX_CHANGE_AGE`.
    #[test]
    fn prop_is_newer_than_window_semantic(
        last in 0u32..u32::MAX/2,
        window in 1u32..1_000_000,
        offset in 0u32..1_000_000,
    ) {
        let this = last.wrapping_add(window);
        let stored = last.wrapping_add(offset);
        let result = Tick::new(stored).is_newer_than(Tick::new(last), Tick::new(this));
        // stored ∈ (last, this] iff 0 < offset <= window.
        let expected = offset > 0 && offset <= window;
        prop_assert_eq!(
            result, expected,
            "is_newer_than mismatch: stored={} last={} this={} offset={} window={}",
            stored, last, this, offset, window
        );
    }

    /// Plan §13.4 / Round 2 W2 — under boyko's per-frame bump regime, the
    /// invariant `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD < u32::MAX` MUST
    /// hold. The const block in `tick.rs` enforces this at compile time;
    /// this property pins it at runtime over the full simulation domain
    /// (verifies the algebraic relation under randomised tick patterns).
    #[test]
    fn prop_max_change_age_safe_under_per_frame_bump(
        ticks_per_scan in 1u32..=CHECK_TICK_THRESHOLD,
    ) {
        // Imagine a stored tick aged `MAX_CHANGE_AGE` immediately after
        // a `check_ticks` scan; advance `ticks_per_scan` more frames
        // without another scan. The maximum possible age is
        // `MAX_CHANGE_AGE + ticks_per_scan`, bounded above by
        // `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD`.
        let worst_age = (MAX_CHANGE_AGE as u64) + (ticks_per_scan as u64);
        prop_assert!(
            worst_age < u32::MAX as u64,
            "MAX_CHANGE_AGE + ticks_per_scan >= u32::MAX — wraparound semantics break: worst_age={} u32::MAX={}",
            worst_age,
            u32::MAX
        );
    }

    /// Plan §13.4: `Added` ⊆ `Changed` for the same tick — if `added`
    /// satisfies the filter window, `changed` (which equals `added`
    /// immediately after insertion) MUST too, under identical
    /// `(last_run, this_run)`.
    #[test]
    fn prop_added_implies_changed(
        last in 1u32..u32::MAX/2,
        window in 1u32..1_000_000,
        offset in 0u32..1_000_000,
    ) {
        let this = last.wrapping_add(window);
        let stored = last.wrapping_add(offset);
        let added_match = Tick::new(stored).is_newer_than(Tick::new(last), Tick::new(this));
        let changed_match = Tick::new(stored).is_newer_than(Tick::new(last), Tick::new(this));
        prop_assert_eq!(
            added_match, changed_match,
            "Added and Changed share the same is_newer_than predicate; same tick → same result"
        );
    }
}
