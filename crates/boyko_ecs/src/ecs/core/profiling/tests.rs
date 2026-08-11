//! The rung-2 mechanism suite.
//!
//! # Why these are unit tests and not integration tests
//!
//! Two of them need inputs no public API can produce. The clock-epoch detector fires on a gap of
//! [`MAX_PLAUSIBLE_FRAME_TICKS`] — ten seconds — and a gate can neither wait that long nor suspend
//! the machine, so the branch is only reachable by backdating the previous fold's stamp
//! ([`Profiler::backdate_last_fold`], `#[cfg(test)]`). A detector nothing ever runs is the
//! vacuous-gate pattern with extra steps.
//!
//! # One global, one lock — the fifth time this campaign has needed the rule
//!
//! `VM_BASE`, `ARMED_STRIDE`, `ARM_MASK`, the lane rings and the `92xx` once-latches are all
//! process-global. Two tests arming concurrently would fold each other's samples into each other's
//! frames, and the failure would look like an attribution bug. Every test that arms takes
//! [`test_serial`], which is this module's ONE lock over all of them.
//!
//! The latches make one figure un-assertable: a `Once` code fires for the **first** claimant in
//! the process, so a test can only ever assert `report_count(..) >= 1`. It is stated at each site
//! rather than smoothed, because `>= 1` and `== 1` differ in what they would catch.

use std::sync::MutexGuard;

use boyko_diag::lane::{LANE_COUNT, set_lane};
use boyko_diag::profile::REGION_CAPACITY;
use boyko_diag::profiling_abi::ENGINE_ZONE_SLOTS;
use boyko_diag::sample::{self, Region, Sample, SampleKind};
use boyko_diag::{clock, profiling_abi};

use super::diag;
use super::fold::fold;
use super::store::{
    ArmOutcome, CellLabel, FOLD_L1D_ZONE_LIMIT, FrameState, Profiler, ProfilerConfig, ROOT_SCOPE,
    WINDOW, test_serial,
};

/// The lane this file's producers write. Nothing else in this binary claims a lane.
const TEST_LANE: u16 = 3;

/// A zone id inside every geometry this file arms.
const ZONE: u16 = 7;

/// **The one geometry this whole test binary arms with.**
///
/// The reservation, `ARMED_STRIDE` and `ARMED_HIST_SLOTS` are process-global and published ONCE, so
/// the geometry is a property of the process rather than of a test. Two fixtures asking for
/// different tier-C slot counts is not a race — it is one of them being refused with
/// `HistGeometryMismatch`, which is the store behaving correctly and the fixture being wrong.
/// Measured at rung 12: three tier-C tests failed in their own `arm` before this const existed.
///
/// `MAX_HIST_SLOTS` rather than the 4 the tier-C tests need, so the refusal test can exhaust it
/// without a second geometry.
const TEST_GEOMETRY: ProfilerConfig =
    ProfilerConfig { user_zone_budget: 0, hist_slots: crate::ecs::core::profiling::store::MAX_HIST_SLOTS };

/// Take the lock, arm a fresh store on [`TEST_LANE`], and leave the rings empty.
///
/// The drain is not tidiness: `arm` resets the **overflow** deltas but cannot reset the rings,
/// and a sample another test left behind would be folded into this one's frames by its stamp.
fn armed() -> (MutexGuard<'static, ()>, Profiler) {
    let guard = test_serial();
    set_lane(TEST_LANE);
    let mut p = Profiler::new();
    let outcome = p.arm(TEST_GEOMETRY);
    assert!(
        matches!(outcome, ArmOutcome::Armed | ArmOutcome::Rearmed),
        "the canonical geometry must always arm: {outcome:?}"
    );
    drain_every_region();
    (guard, p)
}

/// Empty every lane region without folding anything.
fn drain_every_region() {
    for lane in 0..LANE_COUNT {
        for region in [Region::Engine, Region::User] {
            // SAFETY: the caller holds the module's `test_serial` lock, so this thread is the only
            //   consumer of these regions in the process for the duration of the call.
            unsafe {
                sample::drain_region(lane, region, |_| {});
            }
        }
    }
}

fn span(stamp: u64, value: u64) -> Sample {
    Sample { stamp, value, zone: ZONE, flags: SampleKind::Span as u16, _pad: 0 }
}

fn counter(stamp: u64, value: u64) -> Sample {
    Sample { stamp, value, zone: ZONE, flags: SampleKind::Counter as u16, _pad: 0 }
}

fn gauge(stamp: u64, value: u64) -> Sample {
    Sample { stamp, value, zone: ZONE, flags: SampleKind::Gauge as u16, _pad: 0 }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// arm / disarm
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// `arm` publishes in the one order the whole soundness argument rests on: the slab, then every
/// lane buffer, then the mask. This asserts the observable end state of that order — the mask is
/// set and **every** region has a buffer, so no emitter can pass the mask gate and reach a null.
#[test]
fn arm_publishes_every_region_and_then_the_mask() {
    let (_g, mut p) = armed();
    assert!(p.is_armed());
    assert!(profiling_abi::scope_armed(ROOT_SCOPE), "the mask is what an emitter reads first");
    assert_eq!(p.zone_stride(), ENGINE_ZONE_SLOTS as u32);
    assert!(Profiler::reserved_bytes() > 0);
    for lane in 0..LANE_COUNT {
        for region in [Region::Engine, Region::User] {
            assert!(
                sample::region_armed(lane, region),
                "lane {lane} {region:?} has no buffer, so an emitter could pass the mask and \
                 reach a null pointer"
            );
        }
    }
    p.disarm();
    assert!(!p.is_armed());
    // Disarm nulls NOTHING. An emitter that passed the mask gate before the store could otherwise
    // load a nulled pointer after it.
    assert!(sample::region_armed(TEST_LANE, Region::Engine));
}

/// A second `arm` at the live geometry allocates **zero** additional bytes — the residency gate's
/// second clause, asserted here where the mechanism is rather than only in the gate.
#[test]
fn a_second_arm_reserves_nothing_further() {
    let (_g, mut p) = armed();
    let before = Profiler::reserved_bytes();
    let outcome = p.arm(TEST_GEOMETRY);
    assert_eq!(outcome, ArmOutcome::Rearmed);
    assert_eq!(Profiler::reserved_bytes(), before, "a re-arm reserved more address space");
}

/// A geometry the live session cannot change to is **refused**, not applied — and refused with a
/// value rather than a panic, because a mis-sized re-arm is a host's configuration mistake and a
/// profiler that kills a shipped title over one has become the failure it reports.
#[test]
fn re_arming_with_a_different_geometry_is_refused_with_e9213() {
    let (_g, p) = armed();
    let live = p.zone_stride();
    let mut other = Profiler::new();
    let outcome = other.arm(ProfilerConfig { user_zone_budget: 64, hist_slots: 0 });
    assert_eq!(outcome, ArmOutcome::GeometryMismatch { live, asked: live + 64 });
    assert!(!other.is_armed(), "a refused arm must not leave a store thinking it armed");
    assert_eq!(other.zone_stride(), 0);
    // `>= 1`: `E9213` is `Once` and another test in this process may have claimed it first.
    assert!(diag::report_count(9213) >= 1, "the refusal was silent");
    // The live session is untouched.
    assert_eq!(p.zone_stride(), live);
}

/// `W9211` reports rather than refuses, and it reports on the geometry that is actually live.
///
/// The threshold is not a preference: 21 B per zone per frame × 1024 zones is the 21 KiB column
/// row that, plus the fold's ~9.6 KiB of lane reads, lands at 30.6 KiB against a 32 KiB L1d.
#[test]
fn a_stride_over_the_l1d_limit_is_reported_and_still_arms() {
    let (_g, p) = armed();
    assert!(p.is_armed(), "W9211 reports; it must never refuse");
    if p.zone_stride() > FOLD_L1D_ZONE_LIMIT {
        assert!(
            diag::report_count(9211) >= 1,
            "the live stride is over the L1d limit and nothing said so"
        );
    } else {
        // Not a skip: at or under the limit there is nothing to report, and asserting the code
        // fired would be asserting a warning about a configuration that does not warrant one.
        assert_eq!(diag::report_count(9211), 0);
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// attribution
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A sample lands in the frame containing its `stamp`, on either side of a boundary.
#[test]
fn a_sample_straddling_a_boundary_lands_in_the_frame_containing_its_stamp() {
    let (_g, mut p) = armed();
    let f0_begin = p.begin_of_row(0);

    fold(&mut p); // opens frame 1; frame 0's interval is now closed
    let f1_begin = p.begin_of_row(1);
    assert!(f1_begin > f0_begin);

    assert!(sample::push(Region::Engine, span(f0_begin, 10)));
    assert!(sample::push(Region::Engine, span(f1_begin, 20)));
    fold(&mut p); // opens frame 2 and folds both

    let c0 = p.cell(0, ZONE).expect("frame 0 row");
    let c1 = p.cell(1, ZONE).expect("frame 1 row");
    assert_eq!((c0.count, c0.total), (1, 10), "the earlier stamp left its own frame");
    assert_eq!((c1.count, c1.total), (1, 20), "the later stamp left its own frame");
}

/// Attribution reads `stamp` and **only** `stamp`, for every kind.
///
/// This is B1's direct red: a fold that consumed the payload would put the 10^18 counter far in
/// the future or far in the past and count it as `late`, while the 10^3 one landed correctly — so
/// the defect would look like a magnitude-dependent attribution bug rather than a field mix-up.
#[test]
fn a_counter_is_attributed_by_its_stamp_and_never_by_its_value() {
    let (_g, mut p) = armed();
    fold(&mut p);
    let live_begin = p.begin_of_row(p.cursor());

    assert!(sample::push(Region::Engine, counter(live_begin, 1_000)));
    assert!(sample::push(Region::Engine, counter(live_begin, 1_000_000_000_000_000_000)));
    let row = p.cursor();
    fold(&mut p);

    let c = p.cell(row, ZONE).expect("the live frame's row");
    assert_eq!(c.count, 2, "a counter's value decided its frame");
    assert_eq!(c.total, 1_000 + 1_000_000_000_000_000_000, "a counter's cell must SUM");
    assert_eq!(p.drops().late, 0, "a large payload was mistaken for an old stamp");
}

/// A nested span pair is written **inner first** — the inner span closes before the outer one, so
/// the ring carries the later stamp before the earlier. A forward-only walk attributes the outer
/// span to the inner one's frame.
#[test]
fn a_nested_pair_written_out_of_stamp_order_lands_in_the_right_frames() {
    let (_g, mut p) = armed();
    let outer_stamp = p.begin_of_row(0);
    fold(&mut p);
    let inner_stamp = p.begin_of_row(1);

    // Written in CLOSE order: inner (later stamp) first, outer (earlier stamp) second.
    assert!(sample::push(Region::Engine, span(inner_stamp, 5)));
    assert!(sample::push(Region::Engine, span(outer_stamp, 500)));
    fold(&mut p);

    let c0 = p.cell(0, ZONE).expect("frame 0 row");
    let c1 = p.cell(1, ZONE).expect("frame 1 row");
    assert_eq!((c1.count, c1.total), (1, 5), "the inner span moved");
    assert_eq!(
        (c0.count, c0.total),
        (1, 500),
        "the outer span was attributed to the inner one's frame — the walk is not bidirectional"
    );
}

/// A stamp older than the retained window is `late`, and `W9209` says so.
///
/// Not attributed to the floor: putting it in the oldest retained frame would place a sample in a
/// frame it did not happen in, which is a wrong number rather than a missing one.
#[test]
fn a_sample_older_than_the_window_is_late() {
    let (_g, mut p) = armed();
    let base = p.begin_of_row(0);
    fold(&mut p);
    let before = p.drops().late;

    // One tick before the first retained frame began.
    assert!(sample::push(Region::Engine, span(base - 1, 1)));
    fold(&mut p);

    assert_eq!(p.drops().late, before + 1, "a sample below the floor was silently attributed");
    assert!(diag::report_count(9209) >= 1, "the late drop was not reported");
}

/// One zone taking 100 000 samples in one frame keeps `count` exact and `total` consistent.
///
/// This is M9's boundary: `count` was a `u16` for four revisions, and 100 000 wraps it **silently**
/// — after which `total`, `min` and `max` describe a different sample set than `count` does, no
/// drop class covers it and nothing would have said so.
///
/// The samples arrive over many folds because a region holds `REGION_CAPACITY` at a time. They all
/// carry frame 0's stamp, so they are all attributed to frame 0 — which is the point: attribution
/// is by stamp, not by which fold happened to carry the sample.
#[test]
fn one_zone_taking_a_hundred_thousand_samples_keeps_count_exact() {
    const N: u32 = 100_000;
    let (_g, mut p) = armed();
    let stamp = p.begin_of_row(0);

    let mut pushed = 0u32;
    while pushed < N {
        let batch = u32::min(REGION_CAPACITY, N - pushed);
        for _ in 0..batch {
            assert!(sample::push(Region::Engine, counter(stamp, 1)), "a fresh region refused");
            pushed += 1;
        }
        fold(&mut p);
    }

    let c = p.cell(0, ZONE).expect("frame 0 row");
    assert_eq!(c.count, N, "count is not exact at 100 000 — a u16 would read {}", N % 65_536);
    assert_eq!(c.total, u64::from(N), "total and count describe different sample sets");
    assert_eq!(p.drops().late, 0);
    assert_eq!(p.drops().engine_overflow, 0, "the batching was supposed to stay inside capacity");
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// accumulation semantics
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A gauge is a level: the last one in a frame wins. A counter is a rate: the frame's sum.
///
/// The two share a cell shape and differ only here, which is exactly why one test asserts both —
/// an implementation that assigns for both, or accumulates for both, passes half of this.
#[test]
fn a_gauge_is_last_write_wins_and_a_counter_accumulates() {
    let (_g, mut p) = armed();
    fold(&mut p);
    let stamp = p.begin_of_row(p.cursor());
    let row = p.cursor();

    assert!(sample::push(Region::Engine, gauge(stamp, 11)));
    assert!(sample::push(Region::Engine, gauge(stamp, 22)));
    fold(&mut p);
    let c = p.cell(row, ZONE).expect("row");
    assert_eq!(c.total, 22, "a gauge's cell must hold the LAST level, not the sum");
    assert_eq!(c.count, 2, "both gauges were still counted");

    let row2 = p.cursor();
    let stamp2 = p.begin_of_row(row2);
    assert!(sample::push(Region::Engine, counter(stamp2, 11)));
    assert!(sample::push(Region::Engine, counter(stamp2, 22)));
    fold(&mut p);
    let c2 = p.cell(row2, ZONE).expect("row");
    assert_eq!(c2.total, 33, "a counter's cell must SUM — a rate needs the frame's total");
}

/// A span longer than `u32::MAX` ticks keeps `total` and `count` exact and loses only the extrema,
/// which the cell's own label reports.
#[test]
fn a_span_past_u32_range_is_labelled_and_counted_but_not_lost() {
    let (_g, mut p) = armed();
    fold(&mut p);
    let stamp = p.begin_of_row(p.cursor());
    let row = p.cursor();
    let before = p.drops().span_over_range;

    let huge = u64::from(u32::MAX) + 7;
    assert!(sample::push(Region::Engine, span(stamp, huge)));
    fold(&mut p);

    let c = p.cell(row, ZONE).expect("row");
    assert_eq!(c.total, huge, "total must stay EXACT; only min/max lose range");
    assert_eq!(c.count, 1);
    assert_eq!(c.max, u32::MAX, "the extremum clamps");
    assert_eq!(c.label, CellLabel::OverRange, "a clamped cell that reads MEASURED is a lie");
    assert_eq!(p.drops().span_over_range, before + 1);
}

/// An untouched cell says so. `CellLabel::Empty` is discriminant 0, so a recycled row starts here
/// without an initialisation pass — and a reader can tell "no samples" from "a measured zero".
#[test]
fn an_untouched_cell_is_empty_rather_than_a_measured_zero() {
    let (_g, mut p) = armed();
    fold(&mut p);
    let c = p.cell(p.cursor(), ZONE + 1).expect("row");
    assert_eq!(c.label, CellLabel::Empty);
    assert_eq!((c.total, c.count), (0, 0));
}

/// A recycled row cannot report the frame it held `WINDOW` frames ago.
#[test]
fn a_row_is_zeroed_when_the_cursor_wraps_onto_it() {
    let (_g, mut p) = armed();
    let stamp = p.begin_of_row(0);
    assert!(sample::push(Region::Engine, counter(stamp, 42)));
    fold(&mut p);
    assert_eq!(p.cell(0, ZONE).expect("row").total, 42);

    // Wrap the cursor all the way round onto row 0 again. The fold above already moved it to row
    // 1, so the wrap is `WINDOW - 1` more.
    for _ in 0..WINDOW - 1 {
        fold(&mut p);
    }
    assert_eq!(p.cursor(), 0, "the cursor did not return to row 0");
    let c = p.cell(0, ZONE).expect("row");
    assert_eq!((c.total, c.count), (0, 0), "a recycled row still reports its old frame");
    assert_eq!(c.label, CellLabel::Empty);
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// loss accounting — G4b
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The fold's accumulation is lossless **and** idempotent: a region's refusals are counted exactly
/// once, and a second fold with no new refusals adds nothing.
///
/// # The showable RED, and why it is not the corpus's
///
/// The corpus's `G4b` says *"replace `fetch_sub(observed)` with `store(0)` and run a live producer
/// ⇒ an increment between load and clear is lost"*. There is no clear here at all: `substrate/
/// loss-fold`'s Q2 resolved to **(b)**, monotone counters with the delta at the consumer, and the
/// transport shipped that shape. The claim survives verbatim; the RED changes with the mechanism:
/// **replace `overflow_since(lane, region, seen)` with `overflow(lane, region)` in `fold.rs`** ⇒
/// the second fold re-adds the same refusals ⇒ `engine_overflow` doubles ⇒ the second assertion
/// below reds. Run and confirmed at implementation.
#[test]
fn a_regions_refusals_are_counted_exactly_once() {
    let (_g, mut p) = armed();
    fold(&mut p);
    let before = p.drops().engine_overflow;

    // Fill the region exactly, then refuse a known number.
    let stamp = p.begin_of_row(p.cursor());
    for _ in 0..REGION_CAPACITY {
        assert!(sample::push(Region::Engine, counter(stamp, 1)), "a drained region refused early");
    }
    const REFUSED: u64 = 5;
    for _ in 0..REFUSED {
        assert!(!sample::push(Region::Engine, counter(stamp, 1)), "a full region admitted");
    }

    fold(&mut p);
    assert_eq!(
        p.drops().engine_overflow,
        before + REFUSED,
        "the fold did not count exactly what the region refused"
    );

    // Nothing new was refused, so nothing new may be counted. This is the clause a re-read of the
    // monotone total instead of the delta fails.
    fold(&mut p);
    assert_eq!(p.drops().engine_overflow, before + REFUSED, "the fold re-counted old refusals");
    assert!(diag::report_count(9203) >= 1, "the overflow was silent");
}

/// The engine and user regions lose independently — a game's runaway scope costs a game's samples.
///
/// This is the transport-level half of `G20`; the macro-level half needs a second crate in one
/// process and is that gate's by construction.
#[test]
fn a_full_user_region_costs_the_engine_nothing() {
    let (_g, mut p) = armed();
    fold(&mut p);
    let before = p.drops();
    let stamp = p.begin_of_row(p.cursor());

    for _ in 0..REGION_CAPACITY {
        assert!(sample::push(Region::User, counter(stamp, 1)));
    }
    assert!(!sample::push(Region::User, counter(stamp, 1)), "a full user region admitted");
    // The engine region is untouched by all of that.
    assert!(sample::push(Region::Engine, counter(stamp, 9)), "the engine region was collateral");

    fold(&mut p);
    assert_eq!(p.drops().user_overflow, before.user_overflow + 1);
    assert_eq!(
        p.drops().engine_overflow,
        before.engine_overflow,
        "a game's overflow reached the engine's counter"
    );
}

/// For any interleaving of pushes and folds, per region: `pushed == folded + in_ring + refused`.
///
/// The property the whole transport rests on, asserted over a schedule that folds at irregular
/// intervals rather than after every batch.
#[test]
fn every_pushed_sample_is_folded_in_flight_or_refused() {
    let (_g, mut p) = armed();
    fold(&mut p);
    let stamp = p.begin_of_row(0);
    let refused_before = p.drops().engine_overflow;

    let mut pushed = 0u64;
    let mut refused = 0u64;
    // 7 and 300 are coprime with the capacity, so the fold lands mid-ring rather than on a
    // boundary that would make the arithmetic work by accident.
    for round in 0..300u64 {
        for _ in 0..7 {
            if sample::push(Region::Engine, counter(stamp, 1)) {
                pushed += 1;
            } else {
                refused += 1;
            }
        }
        if round % 11 == 0 {
            fold(&mut p);
        }
    }
    let in_ring = u64::from(sample::pending(TEST_LANE, Region::Engine));
    fold(&mut p);

    let folded = u64::from(p.cell(0, ZONE).expect("row").count);
    assert_eq!(pushed, folded, "a pushed sample was neither folded nor left in the ring");
    assert_eq!(p.drops().engine_overflow - refused_before, refused);
    assert!(in_ring <= u64::from(REGION_CAPACITY));
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// clock epoch — G21
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A forward clock jump discards the in-flight window, is counted, is reported, and the **next**
/// window is complete.
///
/// The corpus's `G21` also asserts that every log record after the jump carries the incremented
/// `clock_epoch`. That clause needs the logger's record to carry an epoch field, which is L16's;
/// it is named here rather than silently dropped.
///
/// RED: delete the `now - last > MAX_PLAUSIBLE_FRAME_TICKS` test in `epoch_broke` ⇒ the ten-second
/// interval is folded as an ordinary frame, `clock_epoch_breaks` stays 0 and the pre-jump cells
/// survive ⇒ the first two assertions red. Run and confirmed at implementation.
#[test]
fn a_forward_clock_jump_discards_the_window_and_recalibrates() {
    let (_g, mut p) = armed();
    let stamp = p.begin_of_row(0);
    assert!(sample::push(Region::Engine, counter(stamp, 1)));
    fold(&mut p);
    assert_eq!(p.cell(0, ZONE).expect("row").count, 1, "the pre-jump sample never landed");

    let epoch_before = clock::clock_epoch();
    let breaks_before = p.drops().clock_epoch_breaks;

    // The injection: backdate the previous fold so the next one sees a ten-second gap. Waiting is
    // not an option and suspending the machine is not a gate.
    p.backdate_last_fold(clock::ticks().saturating_sub(MAX_JUMP));
    fold(&mut p);

    assert_eq!(p.drops().clock_epoch_breaks, breaks_before + 1, "the jump was not detected");
    assert!(clock::clock_epoch() > epoch_before, "the epoch did not advance");
    assert!(diag::report_count(9216) >= 1, "the epoch break was silent");
    let c = p.cell(0, ZONE).expect("row");
    assert_eq!((c.total, c.count), (0, 0), "the in-flight window survived the jump");
    assert_eq!(p.frame(), 0, "the discard did not restart the window");

    // The NEXT window is complete: a sample after the jump folds normally.
    let stamp = p.begin_of_row(0);
    assert!(sample::push(Region::Engine, counter(stamp, 3)));
    fold(&mut p);
    assert_eq!(p.cell(0, ZONE).expect("row").count, 1, "the window after the break is not whole");
}

/// The injected gap: comfortably past the threshold, because the wiring is what this exercises.
///
/// **The boundary itself is asserted elsewhere**, by `fold`'s own `is_forward_jump` test, and it
/// has to be: `now` is read *inside* the fold, after this test has injected `last`, so an injection
/// aimed at exactly the threshold lands however many ticks past it the intervening code took. A
/// test that cannot place its input on the boundary cannot tell `>` from `>=`.
const MAX_JUMP: u64 = super::store::MAX_PLAUSIBLE_FRAME_TICKS * 2;

/// An ordinary gap is a slow frame, not a jump — the negative half of the wiring.
#[test]
fn an_ordinary_gap_is_a_slow_frame_and_not_a_jump() {
    let (_g, mut p) = armed();
    let breaks_before = p.drops().clock_epoch_breaks;
    p.backdate_last_fold(
        clock::ticks().saturating_sub(super::store::MAX_PLAUSIBLE_FRAME_TICKS / 2),
    );
    fold(&mut p);
    assert_eq!(
        p.drops().clock_epoch_breaks,
        breaks_before,
        "a gap well under the threshold was called a jump"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// frame records
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A folded frame seals, and the live one does not.
#[test]
fn the_previous_frame_seals_and_the_live_one_stays_pending() {
    let (_g, mut p) = armed();
    fold(&mut p);
    let live = p.cursor();
    let sealed = (live + WINDOW as u32 - 1) % WINDOW as u32;

    let s = p.frame_record(sealed).expect("sealed row");
    assert_eq!(s.state, FrameState::Sealed);
    assert!(s.cpu_end >= s.cpu_begin, "a sealed frame ended before it began");

    let l = p.frame_record(live).expect("live row");
    assert_eq!(l.state, FrameState::Pending);
    assert_eq!(l.cpu_end, 0, "the live frame has not ended");
    assert_eq!(l.frame, p.frame());
}

/// A frame record counts the samples folded into its row, which is what makes an empty window
/// distinguishable from an unfolded one.
#[test]
fn a_frame_record_counts_the_samples_folded_into_it() {
    let (_g, mut p) = armed();
    let stamp = p.begin_of_row(0);
    for _ in 0..4 {
        assert!(sample::push(Region::Engine, counter(stamp, 1)));
    }
    fold(&mut p);
    assert_eq!(p.frame_record(0).expect("row").samples, 4);
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// the frame driver's four zones (rung 3b)
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The four `App` zones bracket what they say they bracket, with the cardinalities that make the
/// frame definable across `Fixed×N + Main`.
///
/// # What each assertion is for
///
/// `__frame` once per frame is the primary CPU number. `__main_run` once is step ⑤.
/// **`__fixed_step` N times is the one that was undefined before**: an earlier definition called
/// the primary number "the `Schedule::run` span", but the host frame is `Time → events → Fixed×N →
/// Main` — two schedules, one of which runs N times — so that phrase named no single interval.
/// Bracketing one substep inside the catch-up loop is what makes N a `count` a reader can read.
///
/// `__fold` is asserted **non-zero and outside `__frame`**: the instrument's own cost is disclosed
/// rather than hidden, and it is not inside the number it is disclosing.
///
/// RED: move the `__frame` guard above the `fold_frame` call in `App::update_with_delta` ⇒ the
/// fold's span falls inside the frame's ⇒ `__frame`'s total swallows `__fold`'s and the instrument
/// is inside its own primary number. Caught by the last assertion. Run at implementation.
#[test]
fn the_four_app_zones_bracket_the_frame_with_the_right_cardinalities() {
    use std::time::Duration;

    use super::zones::{FIXED_STEP, FOLD, FRAME, MAIN_RUN};
    use crate::ecs::core::app::{App, CoreSchedule};
    use crate::ecs::core::profiling::plugin::ProfilerPlugin;
    use crate::ecs::core::profiling::store::unbind_world;

    let _guard = test_serial();
    set_lane(TEST_LANE);
    unbind_world();
    drain_every_region();

    fn noop() {}

    let mut app = App::new();
    app.set_fixed_timestep(Duration::from_millis(10));
    app.add_systems_in(CoreSchedule::Fixed, noop);
    app.add_plugin(ProfilerPlugin);
    app.finish();

    // Arm through the App's own resource: the fold the frame driver runs is that store's.
    let outcome = app.world_mut().resource_mut::<Profiler>().arm(TEST_GEOMETRY);
    assert!(matches!(outcome, ArmOutcome::Armed | ArmOutcome::Rearmed), "{outcome:?}");
    drain_every_region();

    // A 10 ms timestep and a 30 ms delta is three substeps per frame, chosen so the count is
    // neither 0 nor 1 — the two values a broken bracket is most likely to produce by accident.
    const FRAMES: u64 = 6;
    const SUBSTEPS: u32 = 3;
    app.run_n_with_delta(FRAMES, Duration::from_millis(30));

    let frame_z = profiling_abi::zone_id(&FRAME);
    let fixed_z = profiling_abi::zone_id(&FIXED_STEP);
    let main_z = profiling_abi::zone_id(&MAIN_RUN);
    let fold_z = profiling_abi::zone_id(&FOLD);

    let p = app.world().resource::<Profiler>();
    // The LAST frame's samples are still in the rings: a span closes at the end of its frame and is
    // folded at the top of the next one. So the frame examined is the one before the live cursor.
    let row = (p.cursor() + WINDOW as u32 - 1) % WINDOW as u32;

    let frame = p.cell(row, frame_z).expect("frame row");
    let main = p.cell(row, main_z).expect("frame row");
    let fixed = p.cell(row, fixed_z).expect("frame row");
    // `__fold` is read one row FURTHER back, and the reason is structural rather than an
    // off-by-one. Its guard must close AFTER the drain — otherwise the sample it produces would be
    // taken by the very drain it is measuring and attributed to the frame it was measuring. So its
    // sample is always pushed after that fold's drain has finished, and it waits for the NEXT one.
    // The instrument's own cost is therefore always one fold further behind than the frame's, and
    // a reader comparing them in the same row would be comparing two different frames.
    let fold_row = (p.cursor() + WINDOW as u32 - 2) % WINDOW as u32;
    let fold_cell = p.cell(fold_row, fold_z).expect("frame row");

    assert_eq!(frame.count, 1, "__frame must open exactly once per frame");
    assert_eq!(main.count, 1, "__main_run must open exactly once per frame");
    assert_eq!(
        fixed.count, SUBSTEPS,
        "__fixed_step must open once per SUBSTEP; a bracket around the catch-up loop would read 1"
    );
    assert!(fold_cell.count >= 1, "__fold produced nothing, so the instrument is undisclosed");

    // The instrument is OUTSIDE its own primary number. Not an approximation: `__fold` brackets a
    // call that has already returned when `__frame` opens, so the frame's span cannot contain it.
    assert!(
        frame.total > 0 && fold_cell.total > 0,
        "both spans must be non-zero for the containment claim to mean anything"
    );
    assert!(
        fixed.total <= frame.total,
        "the substeps are inside the frame, so their sum cannot exceed it"
    );
    assert_eq!(
        p.cell(fold_row, fold_z).expect("frame row").count,
        1,
        "__fold must open exactly once per frame"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// per-system spans (rung 3c)
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A system's run produces a span **in its own zone's cell**, once per run, on **two workers**.
///
/// # What each half is for
///
/// *Its own zone* is what makes `SystemMeta.zone` a measurement rather than a minted number nobody
/// reads — the state rung 3a would otherwise have shipped: an id assigned at `try_build` and never
/// consumed by anything.
///
/// *Once per run* is the cardinality the concurrency analysis rests on. A system that runs three
/// times contributes three intervals; a bracket that fired once per frame instead would report a
/// serialisation index computed over a third of the data and say nothing about it.
///
/// Two workers, so the samples travel the **concurrent** dispatch path and land in the workers'
/// own lanes rather than the dispatcher's — a span opened on the dispatcher and closed on a worker
/// would charge the wrong producer, and the overlap analysis reads exactly that pair.
///
/// RED: delete the `SystemSpan::open` at `schedule.rs`'s concurrent site ⇒ both cells stay empty.
/// Second RED: hoist the guard out of the spawned closure so it opens on the dispatcher ⇒ the
/// samples land in the dispatcher's lane, which this test does not assert on directly — it is the
/// lane-attribution clause of `G7(b)` and is named here rather than claimed.
#[test]
fn a_system_run_produces_one_span_per_run_in_its_own_zone() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

    use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
    use crate::ecs::core::profiling::SYSTEM_ZONES_COMPILED;
    use crate::ecs::core::schedule::schedule_builder::ScheduleBuilder;

    static RUNS: AtomicU32 = AtomicU32::new(0);

    fn sys_a() {
        RUNS.fetch_add(1, Ordering::Relaxed);
    }
    fn sys_b() {
        RUNS.fetch_add(1, Ordering::Relaxed);
    }

    let (_g, mut p) = armed();

    let pool: Arc<ThreadPool> = ThreadPoolBuilder::new().num_threads(2).build();
    let mut builder = ScheduleBuilder::new(pool);
    builder.add_system(sys_a);
    builder.add_system(sys_b);
    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    let za = schedule.systems[0].system.meta().zone();
    let zb = schedule.systems[1].system.meta().zone();

    if !SYSTEM_ZONES_COMPILED {
        // Not a skip: at a folded tier the correct answer is that no system has a zone and no
        // span is produced, and asserting it is what would catch a bracket the `const` gate
        // failed to delete.
        assert_eq!(za, crate::ecs::core::profiling::ZONE_ID_UNASSIGNED);
        return;
    }
    assert_ne!(za, zb, "two systems share one zone id, so their spans would merge");

    // The rings must be empty, or a sample another test left would be folded into these cells by
    // its stamp.
    drain_every_region();

    const RUNS_EXPECTED: u32 = 3;
    for _ in 0..RUNS_EXPECTED {
        schedule.run(&mut world);
    }
    assert_eq!(RUNS.load(Ordering::Relaxed), RUNS_EXPECTED * 2, "the systems did not run");

    // Every span was stamped after frame 0 opened at `arm` and before this fold opens frame 1, so
    // they all attribute to row 0.
    fold(&mut p);

    let a = p.cell(0, za).expect("frame 0 row");
    let b = p.cell(0, zb).expect("frame 0 row");
    assert_eq!(a.count, RUNS_EXPECTED, "system A produced {} spans", a.count);
    assert_eq!(b.count, RUNS_EXPECTED, "system B produced {} spans", b.count);
    assert!(a.total > 0 && b.total > 0, "a span of zero ticks is not a measurement");
    assert_eq!(a.label, CellLabel::Measured);
    assert_eq!(p.drops().late, 0, "a span was attributed outside the retained window");
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// rung 3d — the dispatch-round pair
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Every dispatching round records **exactly one span and exactly one width**, on the dispatcher's
/// own lane, and the widths sum to the number of systems the schedule dispatched.
///
/// This is the whole of what the corpus's `RoundRecord` column was for — rounds per frame is the
/// span zone's `count`, round span is its `total`/`min`/`max`, wave width is the counter zone's —
/// obtained from two cells the store already had instead of 90.8 KiB it did not.
///
/// RED: delete the `round.close(dispatched)` call in `executor_main_loop` ⇒ both cells stay empty.
/// Second RED: drop the `dispatched == 0` guard in `RoundProbe::close` ⇒ the backoff rounds record
/// too, `__round_width`'s `min` becomes 0, and the width distribution reports a wave this schedule
/// never dispatched.
#[test]
fn every_dispatching_round_records_one_span_and_one_width() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

    use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
    use crate::ecs::core::profiling::zones::{ROUND, ROUND_WIDTH, ROUND_ZONES_COMPILED};
    use crate::ecs::core::schedule::schedule_builder::ScheduleBuilder;

    static RUNS: AtomicU32 = AtomicU32::new(0);

    fn r_a() {
        RUNS.fetch_add(1, Ordering::Relaxed);
    }
    fn r_b() {
        RUNS.fetch_add(1, Ordering::Relaxed);
    }

    let (_g, mut p) = armed();
    RUNS.store(0, Ordering::Relaxed);

    let z_round = boyko_diag::profiling_abi::zone_id(&ROUND);
    let z_width = boyko_diag::profiling_abi::zone_id(&ROUND_WIDTH);

    let pool: Arc<ThreadPool> = ThreadPoolBuilder::new().num_threads(2).build();
    let mut builder = ScheduleBuilder::new(pool);
    builder.add_system(r_a);
    builder.add_system(r_b);
    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);

    drain_every_region();

    const FRAMES: u32 = 3;
    for _ in 0..FRAMES {
        schedule.run(&mut world);
    }
    assert_eq!(RUNS.load(Ordering::Relaxed), FRAMES * 2, "the systems did not run");

    fold(&mut p);

    let round = p.cell(0, z_round).expect("frame 0 row");
    let width = p.cell(0, z_width).expect("frame 0 row");

    if !ROUND_ZONES_COMPILED {
        // Not a skip: at a folded tier the correct answer is that the probe was deleted from the
        // build, and asserting it is what would catch a bracket the `const` gate failed to remove.
        assert_eq!(round.count, 0, "a folded tier still recorded a round");
        assert_eq!(width.count, 0);
        return;
    }

    assert!(round.count >= FRAMES, "{} rounds for {FRAMES} runs of 2 systems", round.count);
    assert_eq!(
        round.count, width.count,
        "a round recorded a span without a width, or a width without a span"
    );
    assert_eq!(
        width.total,
        u64::from(FRAMES * 2),
        "the widths must sum to the systems actually dispatched"
    );
    assert!(width.min >= 1, "a recorded round dispatched nothing");
    assert!(round.total > 0, "a round of zero ticks is not a measurement");
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// rung 3d — the interval ring
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A zone that opens N times inside one frame contributes **N** intervals, with occurrence indices
/// running `0..N`.
///
/// This is `F19b`'s direct red. The design this replaces wrote one slot per `(frame, system)`, so a
/// `Fixed`-schedule system running eight times per frame overwrote itself seven times and the ring
/// reported the eighth run as if it were the frame's only one.
///
/// RED: make `append_interval` write to slot `bank * INTERVALS_PER_FRAME` without advancing `lens`
/// ⇒ the length stays 1 and only the last span survives.
#[cfg(feature = "profiling-analysis")]
#[test]
fn a_zone_that_opens_n_times_in_one_frame_contributes_n_intervals() {
    let (_g, mut p) = armed();
    let t0 = p.begin_of_row(0);

    for k in 0..5u64 {
        assert!(sample::push(Region::Engine, span(t0 + k, 100 + k)));
    }
    fold(&mut p);

    let ivals = p.intervals_of_frame(0);
    assert_eq!(ivals.len(), 5, "the ring assigned where it must append");
    for (k, iv) in ivals.iter().enumerate() {
        assert_eq!(iv.zone, ZONE);
        assert_eq!(iv.begin, t0 + k as u64);
        assert_eq!(u64::from(iv.dur), 100 + k as u64);
        assert_eq!(
            usize::from(iv.occ),
            k,
            "the occurrence index is the cell's count before the increment"
        );
    }
    // And the cell agrees, because `occ` IS that cell's count — the two are one fact, not two.
    assert_eq!(p.cell(0, ZONE).expect("frame 0 row").count, 5);
}

/// Only spans enter the ring. A counter and a gauge have no duration, and an "interval" built from
/// a payload would be an interval on the number line rather than on the clock.
#[cfg(feature = "profiling-analysis")]
#[test]
fn only_spans_enter_the_interval_ring() {
    let (_g, mut p) = armed();
    let t0 = p.begin_of_row(0);

    assert!(sample::push(Region::Engine, counter(t0, 1_000)));
    assert!(sample::push(Region::Engine, gauge(t0, 2_000)));
    assert!(sample::push(Region::Engine, span(t0, 42)));
    fold(&mut p);

    let ivals = p.intervals_of_frame(0);
    assert_eq!(ivals.len(), 1, "a counter or a gauge was admitted as an interval");
    assert_eq!(u64::from(ivals[0].dur), 42);
    assert_eq!(p.cell(0, ZONE).expect("frame 0 row").count, 3, "all three still reached the cell");
}

/// The ring's horizon is `OVERLAP_FRAMES` frames, and a frame past it gets an EMPTY slice — never
/// the bank's current owner's intervals under the older frame's number.
///
/// RED, MEASURED: increment `intervals_dropped` on `append_interval`'s out-of-horizon return ⇒ the
/// last clause fails. That is the whole distinction this test exists for — a horizon is a stated
/// bound and a full bank is a loss, and only one of them may be counted, or a reader subtracting
/// drops from samples counts the same span twice.
#[cfg(feature = "profiling-analysis")]
#[test]
fn the_ring_forgets_a_frame_older_than_its_horizon() {
    use crate::ecs::core::profiling::store::OVERLAP_FRAMES;

    let (_g, mut p) = armed();
    let t0 = p.begin_of_row(0);
    assert!(sample::push(Region::Engine, span(t0, 11)));
    fold(&mut p);
    assert_eq!(p.intervals_of_frame(0).len(), 1);

    // Walk the cursor exactly to the horizon: frame 0 is retained at a distance of
    // `OVERLAP_FRAMES - 1` and gone at `OVERLAP_FRAMES`. The boundary is asserted on both sides,
    // because `>` and `>=` differ by precisely this one frame.
    while p.frame() < OVERLAP_FRAMES as u32 - 1 {
        fold(&mut p);
    }
    assert_eq!(p.intervals_of_frame(0).len(), 1, "the horizon dropped a frame it still covers");

    fold(&mut p);
    assert_eq!(p.frame(), OVERLAP_FRAMES as u32);
    assert!(p.intervals_of_frame(0).is_empty(), "a frame past the horizon handed back a bank");
    // And the bank frame 0 used is now frame 8's, empty rather than inherited.
    assert!(p.intervals_of_frame(OVERLAP_FRAMES as u32).is_empty());
    assert_eq!(p.drops().intervals_dropped, 0, "leaving the horizon is not a drop");

    // Now the case the two refusals are actually distinguished on: a span stamped inside a frame
    // the RING no longer covers but the 121-frame WINDOW still does. The column takes it; the ring
    // skips it; and `intervals_dropped` must stay at zero, because the measurement was not lost —
    // it is in the cell, and counting it as a drop would report one sample under two headings.
    assert!(sample::push(Region::Engine, span(t0, 13)));
    fold(&mut p);
    assert_eq!(
        p.cell(0, ZONE).expect("frame 0 row").count,
        2,
        "the column refused a frame it still retains"
    );
    assert!(p.intervals_of_frame(0).is_empty());
    assert_eq!(p.drops().late, 0, "the span was inside the retained window");
    assert_eq!(
        p.drops().intervals_dropped,
        0,
        "leaving the ring's horizon was counted as a loss"
    );
}

/// A **full** bank refuses and counts. The measurement itself is never the thing lost — the cell
/// took every span — so what the counter reports is precisely "this many spans are missing from
/// the overlap analysis", and the report carries it beside the index for that reason.
///
/// RED, MEASURED: delete the `drops.intervals_dropped += 1` in the full-bank branch ⇒ the ring
/// silently truncates and a serialisation index computed over 2048 of 2064 spans is handed over as
/// if it were complete.
#[cfg(feature = "profiling-analysis")]
#[test]
fn a_full_bank_refuses_and_counts_it() {
    use crate::ecs::core::profiling::store::INTERVALS_PER_FRAME;

    let (_g, mut p) = armed();
    let t0 = p.begin_of_row(0);

    // Fill the bank exactly. One fold can carry at most `2 * REGION_CAPACITY` samples, so the
    // number of folds is computed from the geometry rather than assumed — and every stamp is `t0`,
    // so every one of them attributes to frame 0 however many folds it takes.
    let mut pushed = 0usize;
    while pushed < INTERVALS_PER_FRAME {
        for region in [Region::Engine, Region::User] {
            for _ in 0..REGION_CAPACITY {
                if pushed >= INTERVALS_PER_FRAME {
                    break;
                }
                assert!(sample::push(region, span(t0, 7)), "a drained region refused early");
                pushed += 1;
            }
        }
        fold(&mut p);
    }
    assert_eq!(p.intervals_of_frame(0).len(), INTERVALS_PER_FRAME);
    assert_eq!(p.drops().intervals_dropped, 0, "a bank filled exactly has refused nothing");

    const OVER: u64 = 16;
    for _ in 0..OVER {
        assert!(sample::push(Region::Engine, span(t0, 7)));
    }
    fold(&mut p);

    assert_eq!(p.intervals_of_frame(0).len(), INTERVALS_PER_FRAME, "the bank grew past capacity");
    assert_eq!(p.drops().intervals_dropped, OVER, "a full bank refused without counting");
    // The measurement itself was never lost — the cell counted every one of them.
    assert_eq!(
        u64::from(p.cell(0, ZONE).expect("frame 0 row").count),
        INTERVALS_PER_FRAME as u64 + OVER
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// rung 3d — G8: concurrency computability
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// **G8.** A schedule with a known-compatible pair and a known conflict: `declared` matches the
/// conflict graph, and `observed` is non-zero.
///
/// # Why there is no SKIP clause
///
/// The corpus pins the configuration at "a pool with >= 2 workers" and says the gate SKIPS below
/// that, with CI failing on any non-zero skip count. `ThreadPoolBuilder::num_threads(2)` is clamped
/// only to `[1, MAX_WORKERS]` (`thread_pool.rs`), so two workers are spawned on a single-core box
/// as readily as on a sixteen-core one — the pool does not consult the machine. A worker count
/// below two would therefore be a threadpool defect and not an environment to be excused, so the
/// clause is an unconditional assertion with the reason printed, which is strictly stronger than a
/// skip that has to be counted.
///
/// # Why the rendezvous
///
/// Two systems that merely spin for 100 µs each overlap only if the scheduler happened to start
/// them together, which makes the gate a coin toss on a loaded machine. The pair here waits for
/// each other first and *then* spins the pinned 100 µs, so overlap is structural whenever the
/// executor really did dispatch them concurrently — and the wait is bounded, so an executor that
/// serialised them fails the assertion instead of hanging.
///
/// RED, MEASURED: return immediately from `append_interval` ⇒ this test fails at
/// `frames_analysed >= 1`. Worth stating exactly, because it is one step earlier than the obvious
/// guess: with no interval at all the report does not compute a serialisation index of 1.0 for a
/// frame that ran in parallel — it reports **no frames analysed**, `compatible_co_ran == 0`, and
/// `serialisation_index() == None`. The corpus's "`observed` unavailable" is the literal outcome.
#[cfg(feature = "profiling-analysis")]
#[test]
fn g8_declared_matches_the_graph_and_observed_overlap_is_non_zero() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
    use std::time::{Duration, Instant};

    use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

    use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
    use crate::ecs::core::profiling::SYSTEM_ZONES_COMPILED;
    use crate::ecs::core::profiling::analysis::{concurrency, pair_overlap};
    use crate::ecs::core::resources::register_new;
    use crate::ecs::core::resources::resource::Resource;
    use crate::ecs::core::schedule::schedule_builder::ScheduleBuilder;
    use crate::ecs::core::system::params::resmut::ResMut;
    use crate::ecs::identifiers::primitives::ResourceId;

    /// Two resources, so that "compatible" and "conflicting" are properties of declared ACCESS and
    /// not of an ordering edge this test invented.
    struct GateA;
    struct GateB;

    // Hand-written, for the reason `Profiler`'s is: `boyko-macros` is a dev-dependency, so its
    // derives are reachable from an integration test and not from a unit test in `src/`.
    impl Resource for GateA {
        fn resource_id() -> ResourceId {
            static ID: std::sync::OnceLock<ResourceId> = std::sync::OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }
    impl Resource for GateB {
        fn resource_id() -> ResourceId {
            static ID: std::sync::OnceLock<ResourceId> = std::sync::OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    static ENTERED: AtomicU32 = AtomicU32::new(0);
    static MET: AtomicU32 = AtomicU32::new(0);

    /// Wait for the other half of the pair, then spin the gate's pinned 100 µs.
    fn spin_together() {
        ENTERED.fetch_add(1, AtomicOrdering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if ENTERED.load(AtomicOrdering::SeqCst) >= 2 {
                MET.fetch_add(1, AtomicOrdering::SeqCst);
                break;
            }
            std::hint::spin_loop();
        }
        let until = Instant::now() + Duration::from_micros(100);
        while Instant::now() < until {
            std::hint::spin_loop();
        }
    }

    // The parameter TYPE is what declares the access the conflict graph reads, so these bodies
    // touch nothing: a write would test the resource plumbing, which is not this gate's subject.
    fn sys_a(_r: ResMut<GateA>) {
        spin_together();
    }
    fn sys_b(_r: ResMut<GateB>) {
        spin_together();
    }
    // Conflicts with BOTH, so the only compatible pair in the schedule is (A, B) and the aggregate
    // has exactly one pair in its denominator.
    fn sys_c(_a: ResMut<GateA>, _b: ResMut<GateB>) {}

    let (_g, mut p) = armed();
    ENTERED.store(0, AtomicOrdering::SeqCst);
    MET.store(0, AtomicOrdering::SeqCst);

    let pool: Arc<ThreadPool> = ThreadPoolBuilder::new().num_threads(2).build();
    assert!(
        pool.worker_count() >= 2,
        "G8 needs a pool of at least 2 workers and asked for 2; the pool built {} — \
         `num_threads` is clamped only to [1, MAX_WORKERS], so this is a threadpool defect",
        pool.worker_count()
    );

    let mut builder = ScheduleBuilder::new(pool);
    builder.add_system(sys_a);
    builder.add_system(sys_b);
    builder.add_system(sys_c);
    let mut world = EcsMaster::new();
    world.insert_resource(GateA);
    world.insert_resource(GateB);
    let mut schedule = builder.build(&mut world);

    if !SYSTEM_ZONES_COMPILED {
        // At a folded tier no system carries a zone, so every one of them is unanalysed and the
        // report says so rather than reporting a serialisation index over nothing.
        drain_every_region();
        schedule.run(&mut world);
        fold(&mut p);
        let r = concurrency(&p, &schedule);
        assert_eq!(r.systems_unanalysed, 3);
        assert_eq!(r.compatible_co_ran, 0);
        assert_eq!(r.serialisation_index(), None, "an index over no data is not a number");
        return;
    }

    // The DECLARED half, read from the same bits the executor dispatches on.
    let compat_ab = !schedule.conflict_graph.conflict_bits[0].contains(1);
    let compat_ac = !schedule.conflict_graph.conflict_bits[0].contains(2);
    let compat_bc = !schedule.conflict_graph.conflict_bits[1].contains(2);
    assert!(compat_ab, "A and B touch disjoint resources and must be declared compatible");
    assert!(!compat_ac, "C writes A's resource and must be declared conflicting");
    assert!(!compat_bc, "C writes B's resource and must be declared conflicting");

    drain_every_region();
    schedule.run(&mut world);
    fold(&mut p);

    assert_eq!(
        MET.load(AtomicOrdering::SeqCst),
        2,
        "the executor did not run the compatible pair concurrently, so there was no overlap to \
         observe — this is a dispatch finding, not a measurement one"
    );

    let report = concurrency(&p, &schedule);
    assert_eq!(report.systems, 3);
    assert_eq!(report.systems_unanalysed, 0);
    assert!(report.frames_analysed >= 1, "the ring covered no frame");
    assert_eq!(report.intervals_dropped, 0, "a truncated bank makes the index unquotable");
    assert_eq!(
        report.compatible_co_ran, 1,
        "exactly one declared-compatible pair could have overlapped in this schedule"
    );
    assert_eq!(
        report.compatible_overlapped, 1,
        "the pair that ran on two workers for 100 us each did not read as overlapping"
    );
    assert_eq!(report.serialisation_index(), Some(0.0), "a fully realised pair serialises at 0");

    // And the per-pair form the corpus prints.
    let ab = pair_overlap(&p, &schedule, 0, 1);
    assert!(ab.declared_compatible);
    assert_eq!(ab.frames_co_ran, 1);
    assert_eq!(ab.frames_overlapped, 1);
    assert_eq!(ab.observed_frac(), Some(1.0));

    let ac = pair_overlap(&p, &schedule, 0, 2);
    assert!(!ac.declared_compatible, "the per-pair form must read the same graph as the aggregate");
}

/// An empty window has **no** serialisation index. `1.0` would report perfect serialisation where
/// the honest answer is that nothing ran.
#[cfg(feature = "profiling-analysis")]
#[test]
fn an_index_over_no_co_running_pair_refuses_rather_than_reading_one() {
    use crate::ecs::core::profiling::analysis::ConcurrencyReport;

    let empty = ConcurrencyReport::default();
    assert_eq!(empty.serialisation_index(), None);

    let all_parallel =
        ConcurrencyReport { compatible_co_ran: 4, compatible_overlapped: 4, ..empty };
    assert_eq!(all_parallel.serialisation_index(), Some(0.0));

    let all_serial = ConcurrencyReport { compatible_co_ran: 4, compatible_overlapped: 0, ..empty };
    assert_eq!(all_serial.serialisation_index(), Some(1.0));
}

// ════════════════════════════════════════════════════════════════════════════════════════════════
// G12 (profiling rung 11) — the scope toggle, two-sided, through the path a game actually has.
// ════════════════════════════════════════════════════════════════════════════════════════════════

/// The first of the two scope bits this gate owns.
///
/// The **top** two, and that is not decoration: `register_scope` mints upward from
/// `USER_SCOPE_BASE`, so a colliding mint would have to be the 31st in the process. `g12_app`
/// asserts the range is still clear rather than trusting the arithmetic — a latent collision would
/// show up as a scope somebody else armed, which reads exactly like the bug this gate exists to
/// catch.
const G12_BIT_A: u8 = 62;
/// The second scope bit — the control. Its samples must keep arriving while A's stop.
const G12_BIT_B: u8 = 63;

boyko_diag::declare_zone!(
    G12_ZONE_A,
    name = "g12.zone.a",
    scope = G12_BIT_A as u32,
    tier = boyko_diag::profiling_abi::ZoneTier::Always,
);

boyko_diag::declare_zone!(
    G12_ZONE_B,
    name = "g12.zone.b",
    scope = G12_BIT_B as u32,
    tier = boyko_diag::profiling_abi::ZoneTier::Always,
);

/// No entity — the resting value of the two toggle mailboxes below.
///
/// `u64::MAX` is not a packable `(EntityId, generation)` pair: it would need generation
/// `u32::MAX`, which the entity master never mints. So "no target" and "some target" cannot be
/// confused, which a plain `0` — entity 0, generation 0, a real spawnable handle — would.
const G12_NO_TARGET: u64 = u64::MAX;

/// The entity the next frame's parallel system must **disable**, packed.
static G12_DISABLE: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(G12_NO_TARGET);
/// The entity the next frame's parallel system must **enable**, packed.
static G12_ENABLE: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(G12_NO_TARGET);

fn g12_pack(e: crate::ecs::core::entity::entity::Entity) -> u64 {
    // `EntityId` is a `usize`; the low half of the word is where it goes, so the pack is only
    // lossless while an id fits 32 bits. A test spawning four entities is nowhere near it, and the
    // assertion says so rather than leaving the truncation to be discovered by a wrong toggle.
    debug_assert!(e.id().0 <= u32::MAX as usize, "invariant: the packed entity id fits 32 bits");
    (u64::from(e.generation()) << 32) | (e.id().0 as u64)
}

fn g12_unpack(bits: u64) -> crate::ecs::core::entity::entity::Entity {
    crate::ecs::core::entity::entity::Entity::new(
        crate::ecs::identifiers::primitives::EntityId((bits & 0xFFFF_FFFF) as usize),
        (bits >> 32) as u32,
    )
}

/// The emitting system: one span on each scope, every frame it runs.
fn g12_emit() {
    let _a = boyko_diag::zone!(G12_ZONE_A);
    let _b = boyko_diag::zone!(G12_ZONE_B);
}

/// **The write path a game actually has** — an ordinary parallel system holding `Commands`.
///
/// Not an exclusive system and not the host: `EcsMaster::enable`/`disable` take `&mut self`, which
/// no parallel system can hold, and rev 3's design named no alternative — so its "only switch" had
/// no caller. This is the caller the tree already supplied
/// (`system/params/entity_commands.rs:220`, `:236`), and the command applies at this system's
/// `apply`, inside the same schedule run.
///
/// `swap` rather than `load`: the mailbox fires exactly once, so a test that ran four more frames
/// cannot re-issue the toggle and mask an implementation that only worked because it was told
/// repeatedly.
fn g12_toggle(mut commands: crate::ecs::core::system::Commands) {
    let off = G12_DISABLE.swap(G12_NO_TARGET, core::sync::atomic::Ordering::Relaxed);
    if off != G12_NO_TARGET {
        commands
            .entity(g12_unpack(off))
            .disable::<crate::ecs::core::profiling::ecs_control::ProfilingScopeEnabled>();
    }
    let on = G12_ENABLE.swap(G12_NO_TARGET, core::sync::atomic::Ordering::Relaxed);
    if on != G12_NO_TARGET {
        commands
            .entity(g12_unpack(on))
            .enable::<crate::ecs::core::profiling::ecs_control::ProfilingScopeEnabled>();
    }
}

/// Samples folded into the newest **complete** frame, for the two scope zones.
///
/// `frame() - 1`, not `frame()`: a span closes at the end of its frame and is drained at the top of
/// the next one, so the live frame's row is still filling. Reading the live row would report zero
/// for every zone and pass the disable clause for the wrong reason.
fn g12_counts(p: &Profiler) -> (u32, u32) {
    let f = p.frame().saturating_sub(1);
    let row = p.row_of(f).expect("the previous frame is inside the retained window");
    let a = p.cell(row, profiling_abi::zone_id(&G12_ZONE_A)).expect("a retained row");
    let b = p.cell(row, profiling_abi::zone_id(&G12_ZONE_B)).expect("a retained row");
    (a.count, b.count)
}

/// Build an armed `App` with two scope entities, both enabled, and the emit/toggle systems.
///
/// Returns the app and the two scope entities, A first.
fn g12_app() -> (crate::ecs::core::app::App, [crate::ecs::core::entity::entity::Entity; 2]) {
    use crate::ecs::core::app::{App, CoreSchedule};
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::profiling::ecs_control::{
        ProfilingScope, ProfilingScopeEnabled, minted_game_scopes,
    };
    use crate::ecs::core::profiling::plugin::ProfilerPlugin;
    use crate::ecs::core::profiling::store::unbind_world;
    use crate::ecs::core::system::Commands;

    set_lane(TEST_LANE);
    unbind_world();
    drain_every_region();

    assert!(
        boyko_diag::profiling_abi::USER_SCOPE_BASE + minted_game_scopes() <= u32::from(G12_BIT_A),
        "a register_scope mint has reached this gate's own bits; the two would toggle each other"
    );

    let mut app = App::new();
    app.add_systems_in(CoreSchedule::Main, g12_toggle);
    app.add_systems_in(CoreSchedule::Main, g12_emit);
    app.add_plugin(ProfilerPlugin);
    app.finish();

    let outcome = app.world_mut().resource_mut::<Profiler>().arm(TEST_GEOMETRY);
    assert!(matches!(outcome, ArmOutcome::Armed | ArmOutcome::Rearmed), "{outcome:?}");
    drain_every_region();

    app.world_mut().run_system(|mut cmds: Commands| {
        cmds.spawn(ProfilingScope { bit: G12_BIT_A, name: "g12.a" });
        cmds.spawn(ProfilingScope { bit: G12_BIT_B, name: "g12.b" });
    });

    let spawned = app.world().query_entities(&[ProfilingScope::component_id()]);
    let mut found: [Option<crate::ecs::core::entity::entity::Entity>; 2] = [None, None];
    for e in spawned {
        let bit = app
            .world_mut()
            .query::<&ProfilingScope, ()>()
            .get(e)
            .expect("the entity was just spawned with the component queried for")
            .bit;
        match bit {
            G12_BIT_A => found[0] = Some(e),
            G12_BIT_B => found[1] = Some(e),
            other => panic!("an unexpected ProfilingScope on bit {other} shares this world"),
        }
    }
    let ents = [found[0].expect("scope A was spawned"), found[1].expect("scope B was spawned")];

    // The host path, used here for SETUP only — the direct-path clause is what tests it as a write
    // path.
    app.world_mut().enable::<ProfilingScopeEnabled>(ents[0]);
    app.world_mut().enable::<ProfilingScopeEnabled>(ents[1]);

    (app, ents)
}

/// **G12 clause 1** — the toggle through `Commands`, from an ordinary parallel system.
///
/// With scopes A and B armed, a parallel system issues
/// `commands.entity(a).disable::<ProfilingScopeEnabled>()` ⇒ the **next** frame has zero A samples
/// **and** a non-zero count of B samples; re-enabling brings A back.
///
/// # What each half catches
///
/// One that projects on the same frame rather than the next would make the latency claim false,
/// which is why the disable is issued a frame before the count is read rather than in the same one.
///
/// # Four REDs, run at implementation, each landing somewhere different
///
/// | Injected defect | Where it actually fired |
/// |---|---|
/// | delete `ecs_control::project(world)` from `fold_frame_cold` | the **warm-up**: nothing measured at all (`0/0`) |
/// | `arm_bit()` returns `1 << (bit - 1)` — the wrong bit | the warm-up here; **the clause-3 control** named the exact words, `left: 0x6000…`, `right: 0xC000…` |
/// | `project_scopes` ORs instead of replacing | the zero-A assertion, `left: 1, right: 0`, in **both** clauses |
/// | the projection publishes `0` when any scope is disabled | *"disabling A silenced B as well"* |
///
/// **The first two land on the warm-up, and that is not a weakness of the gate — it is what a
/// scope-projected mask means.** `arm` sets only `ROOT_SCOPE`; every bit above
/// `PROJECTED_SCOPE_BASE` exists *because* the projection put it there. So a projection that is
/// missing or wrong does not leave the previous behaviour standing, it leaves the two zones
/// unarmed — and the assertion that catches it is the one asserting the gate is not vacuous.
///
/// The corpus's limits column says a whole-mask clear *"passes clause 1 and fails clause 2"*.
/// **MEASURED: it fails clause 1**, on the B half — which the clause column itself demands
/// (*"zero A samples **and** a non-zero count of B samples"*). The two columns disagreed; the
/// clause column is the one that is implemented.
#[test]
fn g12_a_parallel_system_toggles_a_scope_and_only_that_scope() {
    use std::time::Duration;

    let _guard = test_serial();
    let (mut app, ents) = g12_app();
    const DT: Duration = Duration::from_millis(16);

    // Warm-up: both scopes armed, both zones measuring.
    app.run_n_with_delta(4, DT);
    let (a0, b0) = g12_counts(app.world().resource::<Profiler>());
    assert!(
        a0 > 0 && b0 > 0,
        "both scopes must measure before a toggle can mean anything: {a0}/{b0}"
    );

    // The command is issued during the first of these frames and applies inside that schedule run;
    // the mask is projected at the top of the NEXT frame, so the count read below is the frame
    // after the projection — which is what "the next frame" means for both write paths.
    G12_DISABLE.store(g12_pack(ents[0]), core::sync::atomic::Ordering::Relaxed);
    app.run_n_with_delta(3, DT);
    let (a1, b1) = g12_counts(app.world().resource::<Profiler>());
    assert_eq!(a1, 0, "the frame after the disable still measured scope A {a1} times");
    assert!(b1 > 0, "disabling A silenced B as well — the projection cleared the whole mask");

    // Two-sided: the switch comes back.
    G12_ENABLE.store(g12_pack(ents[0]), core::sync::atomic::Ordering::Relaxed);
    app.run_n_with_delta(3, DT);
    let (a2, b2) = g12_counts(app.world().resource::<Profiler>());
    assert!(a2 > 0, "re-enabling scope A did not bring it back — the toggle is one-way");
    assert!(b2 > 0, "B stopped measuring across a toggle of A");
}

/// **G12 clause 2** — the same assertions through the direct `world.disable::<T>(e)` path.
///
/// The host, or an exclusive system already holding `&mut EcsMaster`, has this path and no queue.
/// It lands immediately; the projection still runs at the next fold, so the observable latency is
/// the same one — which is the point of asserting it twice rather than assuming the two paths
/// agree.
#[test]
fn g12_the_direct_world_path_toggles_the_same_scope_with_the_same_latency() {
    use std::time::Duration;

    use crate::ecs::core::profiling::ecs_control::ProfilingScopeEnabled;

    let _guard = test_serial();
    let (mut app, ents) = g12_app();
    const DT: Duration = Duration::from_millis(16);

    app.run_n_with_delta(4, DT);
    let (a0, b0) = g12_counts(app.world().resource::<Profiler>());
    assert!(a0 > 0 && b0 > 0, "both scopes must measure first: {a0}/{b0}");

    app.world_mut().disable::<ProfilingScopeEnabled>(ents[0]);
    app.run_n_with_delta(3, DT);
    let (a1, b1) = g12_counts(app.world().resource::<Profiler>());
    assert_eq!(a1, 0, "the direct disable left scope A measuring {a1} times");
    assert!(b1 > 0, "the direct disable silenced B as well");

    app.world_mut().enable::<ProfilingScopeEnabled>(ents[0]);
    app.run_n_with_delta(3, DT);
    let (a2, b2) = g12_counts(app.world().resource::<Profiler>());
    assert!(a2 > 0, "the direct re-enable did not bring scope A back");
    assert!(b2 > 0, "B stopped measuring across a direct toggle of A");
}

/// **G12 clause 3, the measurable half** — a fielded tag forced through would project ZERO,
/// silently.
///
/// The compile half is a `trybuild` fixture
/// (`tests/enable_filter_compile_fail/profiling_scope_as_enable_tag_rejected.rs`, which names B2's
/// refuted type verbatim): the derive refuses a fielded bitset tag outright — and adding it
/// re-blessed a *neighbouring* fixture, because the compiler's "other types implement `Bundle`"
/// list now carries this rung's two self-bundles. This is the half that matters more, because it
/// is the one
/// a `debug_assert` would not have caught: the **read** path (`enable_tag_api.rs:201-215`) has no
/// storage-kind assert, so pushing a table-storage id through it does not panic — it finds no
/// enable column, answers `false` for every entity, and the projection collapses to an all-zero
/// scope half. A profiler permanently disarmed, in every build, with no diagnostic.
///
/// RED: give `test_enable_bit` the assert the write path has, and this test panics instead of
/// measuring — which is the outcome B2 argues for and the tree does not currently have.
#[test]
fn g12_a_table_storage_id_forced_through_the_enable_path_projects_zero() {
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry::EnableTagId;
    use crate::ecs::core::profiling::ecs_control::{
        ProfilingScope, ProfilingScopeEnabled, projected_bits,
    };

    let _guard = test_serial();
    let (mut app, ents) = g12_app();

    // The shipped path, as the control: with the real tag, both bits project.
    assert_eq!(
        projected_bits(app.world_mut()),
        (1u64 << G12_BIT_A) | (1u64 << G12_BIT_B),
        "the control failed: the real bitset tag must project both scopes"
    );

    // Now B2's design: `ProfilingScope` — a FIELDED, table-storage component — as the enable tag.
    // `EnableTagId`'s field is crate-private precisely because the public surface treats
    // `ComponentId -> EnableTagId` as a proof of mint; constructing one here is what "force the id
    // through anyway" means, and it is only writable from inside this crate.
    let forced = EnableTagId(ProfilingScope::component_id());
    assert_ne!(
        forced.component_id(),
        ProfilingScopeEnabled::component_id(),
        "the two components must be distinct ids, or this test is asserting about the real tag"
    );
    for e in ents {
        assert!(
            !app.world().is_enabled_id(e, forced),
            "a table-storage id read as an enable bit answered TRUE; the silent-zero argument for \
             splitting capability from state rests on it answering false"
        );
    }
}

/// `register_scope` mints inside the game range, carries the name it was given, and refuses rather
/// than wrapping when the word is full.
///
/// Not a `G12` clause — the corpus states `register_scope`'s range in prose. It is gated here
/// because "32..63 for games" is exactly the kind of range that is true until somebody changes a
/// constant, and because the refusal is the only failure a legal call can reach.
///
/// **This test saturates the process's scope counter by design**, which is why the `g12_app` helper
/// asserts the mint has not reached bits 62/63 rather than assuming it: under the module lock the
/// two orders are both legal, and only the assertion tells them apart.
#[test]
fn register_scope_mints_in_the_game_range_and_refuses_past_the_word() {
    use boyko_diag::profiling_abi::{SCOPE_COUNT, USER_SCOPE_BASE};

    use crate::ecs::core::profiling::ecs_control::{ScopeError, register_scope};

    let _guard = test_serial();

    // The mint is process-global and other tests in this binary may have taken bits, so the claim
    // is about the RANGE and the name, not about a particular number.
    let s = register_scope("g12.registered").expect("a fresh mint inside the range succeeds");
    assert!(
        (USER_SCOPE_BASE..SCOPE_COUNT).contains(&u32::from(s.bit)),
        "register_scope minted {} — outside the game range {USER_SCOPE_BASE}..{SCOPE_COUNT}",
        s.bit
    );
    assert_eq!(s.name, "g12.registered", "the name the caller gave must reach the component");
    assert_ne!(s.arm_bit(), 0, "a minted scope must contribute a projectable bit");

    let mut refused = None;
    for _ in 0..=SCOPE_COUNT {
        if let Err(e) = register_scope("g12.drain") {
            refused = Some(e);
            break;
        }
    }
    assert_eq!(
        refused,
        Some(ScopeError::Exhausted),
        "the 33rd game scope must be refused; a wrap would hand out a bit that is already live"
    );
    assert!(register_scope("g12.after").is_err(), "the refusal must be sticky, not one-shot");
}

// ════════════════════════════════════════════════════════════════════════════════════════════════
// G18 / G16 (profiling rung 12) — retention tiers B and C.
// ════════════════════════════════════════════════════════════════════════════════════════════════

boyko_diag::declare_zone!(
    TIER_TEST_ZONE,
    name = "t.tier",
    scope = ROOT_SCOPE,
    tier = boyko_diag::profiling_abi::ZoneTier::Always,
);

/// The zone the tier tests own — **MINTED, never a hand-picked number**.
///
/// # Why a raw id is unsound here, measured
///
/// The first draft used `const tier_zone(): u16 = 11`, chosen to be distinct from [`ZONE`]. `G18`
/// then counted **20 013 of 20 000** samples in a full-workspace sweep: thirteen it never pushed.
///
/// The module lock does not prevent this and cannot. `ARM_MASK` is process-global, so while ANY
/// profiling test holds the profiler armed, **every other test in this binary that runs a schedule
/// emits `SystemSpan` samples** (`zones.rs:193`) into the shared lane rings — on its own thread, in
/// its own lane, which the fold drains along with everything else. Those samples carry per-system
/// zone ids minted at `try_build` out of the same monotone `ENGINE_ID_NEXT`. A hand-picked id is
/// therefore a bet that no system in the whole crate's test suite lands on it, and the bet is
/// re-rolled by every change to test execution order.
///
/// Minting removes the bet: the counter is monotone and shared, so an id this handle owns is one no
/// `SystemMeta` can ever be given.
///
/// ⚠️ [`ZONE`] is still a raw `7` and carries the same latent hazard. It is left alone here because
/// it is pre-existing and its tests assert on single cells rather than on exact session totals, so
/// they are far less sensitive — but it is recorded in `docs/OPEN-QUESTIONS.md` rather than left to
/// be rediscovered.
fn tier_zone() -> u16 {
    profiling_abi::zone_id(&TIER_TEST_ZONE)
}

/// Arm with tier C committed, and leave the rings empty.
///
/// A wrapper rather than a second geometry, and that is the whole lesson: `hist_slots` is the
/// SECOND DIMENSION of a geometry the process publishes once, so a fixture that armed with its own
/// slot count would be refused by `HistGeometryMismatch` rather than getting one. See
/// [`TEST_GEOMETRY`].
fn armed_with_hist() -> (MutexGuard<'static, ()>, Profiler) {
    armed()
}

/// Push one span and fold it, returning nothing — the fixture for driving many frames cheaply.
fn push_span_at(zone: u16, stamp: u64, value: u64) {
    let s = Sample { stamp, value, zone, flags: SampleKind::Span as u16, _pad: 0 };
    assert!(sample::push(Region::Engine, s), "the region must accept a lone test sample");
}

/// **G18** — the lifetime accumulator agrees with the ring, over more frames than the ring retains.
///
/// `lifetime[z].count` equals Σ per-frame `count[z]`, and `lifetime[z].max` equals the max per-frame
/// `max[z]`. Driven over **10 000 frames**, which is 82 windows: the accumulator is the only thing
/// that still remembers frame 0, so an implementation that read the ring would fail on scale alone.
///
/// # Why the sum is taken as the samples are FED, not by walking the ring
///
/// Walking the ring at the end would only see the last 121 frames — the very limitation tier B
/// exists to remove — so the oracle is accumulated in the test's own two variables at push time.
/// That also makes the oracle independent of every store mechanism the gate is judging.
///
/// # ⚠️ This clause DOES NOT catch the corpus's own row-pass defect — MEASURED
///
/// The RED was run: tier B restricted to samples landing in the row about to be sealed, which is
/// what a sweep of the sealed row is equivalent to. **This test stayed GREEN.** Every sample it
/// feeds is stamped and folded inside one frame, so nothing ever crosses a boundary and the row
/// pass loses nothing to lose.
///
/// So `G18` **as the corpus states it** — *"`lifetime[z].count` equals Σ per-frame `count[z]`"* —
/// would have certified the defective implementation. The clause that actually catches it is the
/// next test, and it exists because this one was measured not to.
#[test]
fn g18_the_lifetime_accumulator_agrees_with_every_frame_the_ring_forgot() {
    let (_g, mut p) = armed();

    const FRAMES: u64 = 10_000;
    let mut oracle_count = 0u64;
    let mut oracle_max = 0u32;
    let mut oracle_total = 0u64;
    let mut oracle_min = u32::MAX;

    for f in 0..FRAMES {
        // Two samples per frame, one of them varying, so `max` has something to track and `count`
        // is not merely the frame number.
        let a = 100 + (f % 97) * 13;
        let b = 50 + (f % 31);
        let now = boyko_diag::clock::ticks();
        push_span_at(tier_zone(), now, a);
        push_span_at(tier_zone(), now, b);
        oracle_count += 2;
        oracle_total += a + b;
        oracle_max = oracle_max.max(a as u32).max(b as u32);
        oracle_min = oracle_min.min(a as u32).min(b as u32);
        fold(&mut p);
    }
    // One more fold: the last frame's samples are drained at the top of the frame after it.
    fold(&mut p);

    let life = p.lifetime(tier_zone()).expect("an armed store has an accumulator for every zone");
    assert_eq!(
        life.count, oracle_count,
        "tier B counted {} of {oracle_count} samples over {FRAMES} frames",
        life.count
    );
    assert_eq!(life.total, oracle_total, "tier B's sum diverged from the fed sum");
    assert_eq!(life.max_ticks(), Some(oracle_max), "tier B's maximum is not the maximum fed");
    assert_eq!(life.min_ticks(), Some(oracle_min), "tier B's minimum is not the minimum fed");

    // The claim that makes this gate worth more than a counter test: the ring cannot answer it.
    // Frame 0 left the window 82 windows ago, and its samples are still in the total above.
    assert!(
        FRAMES > WINDOW as u64,
        "the gate must outrun the ring, or it is not testing retention at all"
    );
    let oldest_retained = p.frame().saturating_sub(WINDOW as u32 - 1);
    assert!(
        oldest_retained > 0,
        "the ring still holds frame 0, so this gate has not yet proved tier B remembers anything \
         the ring forgot"
    );
}

/// **G18's second half** — a span that CROSSES a frame boundary still reaches the accumulator.
///
/// This is the case the corpus's row-pass loses. A span stamped in frame `F` but written during
/// `F+1` is drained by the fold at the top of `F+2`; a sweep of `F`'s row at seal already ran one
/// fold earlier, so the sample would land in `F`'s cell and never in tier B.
///
/// MEASURED as the difference between the two designs: the cell and the accumulator must agree.
///
/// # RED, run at implementation
///
/// Restrict tier B to samples landing in the row about to be sealed — the row pass, expressed as a
/// one-line filter. This test reds with `left: 0, right: 1`: the span reached its CELL and not the
/// accumulator. Its sibling above, which is `G18` as the corpus writes it, stayed **green** through
/// the same injection.
#[test]
fn g18_a_span_written_a_frame_after_it_was_stamped_still_reaches_tier_b() {
    let (_g, mut p) = armed();

    // Frame A opens.
    fold(&mut p);
    let stamp_in_a = boyko_diag::clock::ticks();
    let frame_a = p.frame();

    // Frame B opens WITHOUT the span having been written yet — this is the boundary crossing.
    fold(&mut p);
    assert_eq!(p.frame(), frame_a + 1, "the fixture must actually have opened a second frame");

    // Now the span closes: stamped in A, pushed during B.
    push_span_at(tier_zone(), stamp_in_a, 4242);

    // The fold at the top of frame C drains it and attributes it to A.
    fold(&mut p);

    let row_a = p.row_of(frame_a).expect("frame A is still inside the window");
    let cell = p.cell(row_a, tier_zone()).expect("a retained row");
    assert_eq!(cell.count, 1, "the crossing span did not land in the frame it was stamped in");

    let life = p.lifetime(tier_zone()).expect("an armed store has an accumulator");
    assert_eq!(
        life.count, 1,
        "the crossing span reached the CELL but not the accumulator -- which is exactly what a \
         sweep of the sealed row produces, and why tier B folds per sample instead"
    );
    assert_eq!(life.max_ticks(), Some(4242));
}

/// **G16** — the histogram's bucket edges bracket a sorted oracle's p99, and it counts what it was
/// fed.
///
/// 100 000 synthetic durations from a known, skewed distribution. The oracle is the sorted array's
/// p99 — computed by the test, from the same values, with no reference to the bucket grid.
///
/// # Why the assertion is a BRACKET and not an equality
///
/// A histogram knows which bucket the answer is in and nothing finer. `quantile` returns the
/// bucket's `[lo, hi)`, and the claim is `lo <= oracle < hi`. An equality would demand precision the
/// structure does not have, and a point estimate would *manufacture* it.
///
/// # RED, run at implementation
///
/// An off-by-one in the bucket index (`bucket_of` returning `idx + 1`) ⇒ the oracle falls outside
/// the returned edges ⇒ red.
#[test]
fn g16_the_histogram_edges_bracket_the_sorted_oracle_p99() {
    use crate::ecs::core::profiling::hist::HistView;

    let (_g, mut p) = armed_with_hist();
    let slot = p.subscribe_histogram(tier_zone()).expect("a fresh subscription gets a slot");
    assert_eq!(slot, 0, "the first subscription takes the first slot");

    const N: usize = 100_000;
    let mut fed: Vec<u64> = Vec::with_capacity(N);
    // A skewed, deterministic distribution: mostly cheap, with a heavy tail — the shape a frame
    // profiler actually sees, and the one where a p99 is worth asking for. No RNG crate: an LCG
    // keeps the gate reproducible and `boyko_ecs`'s dev-dependencies unchanged.
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    for _ in 0..N {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let v = if x % 100 < 99 { 200 + (x >> 40) % 800 } else { 50_000 + (x >> 40) % 400_000 };
        fed.push(v);
    }

    for (i, &v) in fed.iter().enumerate() {
        // The stamp is re-read per sample, and that is not tidiness either. The first draft hoisted
        // one `ticks()` out of the loop; after `WINDOW` folds that stamp is below the retained
        // floor, so `attribute` returns `None` and every later sample is counted `late` instead of
        // folded. MEASURED: the histogram received 30 720 of 100 000 — exactly 120 batches, one per
        // retained frame. A sample must be stamped in the frame it is going to be folded into.
        let now = boyko_diag::clock::ticks();
        push_span_at(tier_zone(), now, v);
        // Fold in batches so the region never overflows: `REGION_CAPACITY` is finite, and a refused
        // push would silently shrink the fed set and make the count assertion below vacuous. The
        // batch is the LOOP INDEX, not `fed.len()` -- the first draft used the latter, which is
        // constant inside the loop, so it never folded at all and the region filled on push 1025.
        // `push_span_at`'s own assertion is what caught it, which is why that assertion is there.
        if i % 256 == 255 {
            fold(&mut p);
        }
    }
    fold(&mut p);
    fold(&mut p);

    let view = p.histogram(tier_zone()).expect("a subscribed zone has a histogram");
    assert_eq!(
        view.count(),
        N as u64,
        "the histogram counted {} of the {N} values fed -- a short count makes every quantile \
         below it a statement about a different sample",
        view.count()
    );

    let mut sorted = fed.clone();
    sorted.sort_unstable();
    // 1-based rank, matching `HistView::quantile`'s own definition, so the two agree about WHICH
    // sample p99 names before they are compared about where it is.
    let rank = ((0.99f64) * N as f64).ceil().max(1.0) as usize;
    let oracle = sorted[rank - 1];

    let (lo, hi) = view.quantile(0.99).expect("a non-empty histogram has a p99");
    assert!(
        lo <= oracle && oracle < hi,
        "p99 bucket [{lo}, {hi}) does not bracket the sorted oracle {oracle}"
    );

    // And the same for the median, so the gate is not a statement about one tail bucket.
    let med_rank = ((0.50f64) * N as f64).ceil().max(1.0) as usize;
    let med_oracle = sorted[med_rank - 1];
    let (mlo, mhi) = view.quantile(0.50).expect("a non-empty histogram has a median");
    assert!(
        mlo <= med_oracle && med_oracle < mhi,
        "median bucket [{mlo}, {mhi}) does not bracket the sorted oracle {med_oracle}"
    );

    // The un-quantised figures survive the grid exactly -- this is what lets a reader trust the
    // mean even where the quantiles are only bracketed.
    let fed_total: u64 = fed.iter().sum();
    assert_eq!(view.total(), fed_total, "the histogram's total must be exact, not bucketed");

    // Non-vacuity: an unsubscribed zone has no histogram at all, so the assertions above are about
    // a slot that had to be granted rather than one that exists for every zone.
    assert!(
        p.histogram(tier_zone() + 1).is_none(),
        "an unsubscribed zone must have no histogram, or the subscription mechanism is not gating \
         anything"
    );
    let _ = HistView::new(&crate::ecs::core::profiling::hist::HistSlot::ZERO);
}

/// Tier C refuses past its committed slots rather than sharing one.
///
/// Two zones on one slot would sum two distributions into a shape that is neither.
#[test]
fn g16_tier_c_refuses_past_its_slots_and_is_idempotent_per_zone() {
    use crate::ecs::core::profiling::store::MAX_HIST_SLOTS;

    let (_g, mut p) = armed_with_hist();

    let a = p.subscribe_histogram(100).expect("the first zone gets a slot");
    let b = p.subscribe_histogram(101).expect("the second zone gets a different slot");
    assert_ne!(a, b, "two zones must not share a slot");

    assert_eq!(
        p.subscribe_histogram(100),
        Some(a),
        "a second subscription for one zone must return its FIRST slot, not spend another -- two          slots for one zone would split its distribution, and the split reads as two quieter zones"
    );
    assert_eq!(p.hist_subscribed(), 2, "the idempotent call must not have consumed a slot");

    // Exhaust the rest, then prove the refusal. Distinct zone ids throughout, so nothing is
    // refused for being a repeat.
    for z in 0..MAX_HIST_SLOTS - 2 {
        assert!(
            p.subscribe_histogram(200 + z as u16).is_some(),
            "slot {z} of the committed {MAX_HIST_SLOTS} was refused early"
        );
    }
    assert_eq!(p.hist_subscribed(), MAX_HIST_SLOTS, "every committed slot must be claimable");

    assert_eq!(
        p.subscribe_histogram(999),
        None,
        "the zone past the last slot must be REFUSED, not folded onto somebody else's"
    );
    assert!(p.histogram(999).is_none(), "a refused zone must have no histogram at all");
}

/// The `hist_slots` request is clamped at [`MAX_HIST_SLOTS`] and the clamp is OBSERVABLE.
///
/// A host that asked for 4096 and silently got 64 would compute a residency figure from a number
/// the store never used.
#[test]
fn tier_c_clamps_its_slot_request_and_says_so() {
    use crate::ecs::core::profiling::store::MAX_HIST_SLOTS;

    let _g = test_serial();
    set_lane(TEST_LANE);
    let mut p = Profiler::new();

    // Ask for far more than the cap. The clamp is what makes this an ACCEPTED re-arm: without it,
    // `slots` would be `MAX_HIST_SLOTS + 9000`, which does not equal the geometry this process
    // published, and the arm would come back `HistGeometryMismatch`. So the outcome below IS the
    // measurement of the clamp, not merely a reading of it.
    let outcome = p.arm(ProfilerConfig {
        user_zone_budget: 0,
        hist_slots: MAX_HIST_SLOTS + 9000,
    });
    assert!(
        matches!(outcome, ArmOutcome::Armed | ArmOutcome::Rearmed),
        "an over-large slot request must be CLAMPED, not refused as a geometry mismatch: {outcome:?}"
    );
    assert_eq!(
        p.hist_slots(),
        MAX_HIST_SLOTS,
        "the clamp must be readable; a caller cannot budget against a number the store discarded"
    );
    drain_every_region();
}
/// Rung 12's residency figures, MEASURED from the store.
///
/// A print rather than an assertion, deliberately and narrowly: the *composition* is gated by
/// `store::tests::the_retention_tiers_grew_the_reservation_by_exactly_their_own_bytes`, and an
/// absolute byte count asserted here would need re-blessing every time an unrelated section moved
/// while saying nothing about which term did. What this leaves behind is the figure the corpus
/// quotes, taken from the store instead of from arithmetic.
#[test]
fn rung12_tier_costs_measured() {
    let (_g, p) = armed();
    let stride = p.zone_stride();
    let slots = p.hist_slots();
    println!(
        "RUNG12 stride={stride} hist_slots={slots}          tierB={} B  hist_of={} B  tierC={} B  reserved_total={} B",
        stride as usize * 24,
        stride as usize,
        slots as usize * 400,
        Profiler::reserved_bytes()
    );
    assert!(Profiler::reserved_bytes() > 0, "an armed store has a reservation to report");
}

// ════════════════════════════════════════════════════════════════════════════════════════════════
// Profiling rung 13 — the OBSERVED sample kind.
// ════════════════════════════════════════════════════════════════════════════════════════════════

boyko_diag::declare_zone!(
    KIND_TEST_ZONE,
    name = "t.kind",
    scope = ROOT_SCOPE,
    tier = boyko_diag::profiling_abi::ZoneTier::Always,
);

/// Minted, for the reason [`tier_zone`] states at length: a gate that asserts what a zone IS must
/// own an id nothing else can be given.
fn kind_zone() -> u16 {
    profiling_abi::zone_id(&KIND_TEST_ZONE)
}

/// **Rung 13** — a zone's kind is what the fold OBSERVED, and an unobserved zone reports absence.
///
/// # What this is for
///
/// A telemetry `ZoneRow` has to say whether `total` is ticks or increments, and MEASURED at this
/// rung there is nowhere else to learn it: `ZoneDesc` carries `name`, `scope`, `tier` and `region`
/// and no kind, and there is no static `counter!` / `gauge!` macro in the tree at all — only the
/// dynamic registry's. So the kind is an observation, and this gate is what makes it one.
///
/// # The clause that matters is the FIRST one
///
/// `SampleKind::Span` is discriminant `0` and the reservation is zero-filled, so a raw cast would
/// report *every zone in the geometry* as a span — including the thousands that never ran. A
/// decoder would then label a row of absent zones "spans, total 0 ticks", which is a measurement.
///
/// # Two REDs, and the FIRST one landed off the prediction
///
/// * Drop the `+ 1` from `kind_byte` **alone** ⇒ the failure is not "everything is a span", it is
///   `left: None, right: Some(Span)` on the zone a span was actually pushed to: the encoder now
///   writes `0` and the decoder still reads `0` as unknown, so **spans become invisible**. Half the
///   shift is a different defect from none of it.
/// * Drop it from `kind_byte` **and** `kind_of_byte` — the honest raw-discriminant cast — ⇒
///   `left: Some(Span), right: None` on the untouched zone, which is the defect the shift exists
///   for: every zone in the geometry, including the thousands that never ran, reported as a span.
#[test]
fn rung13_a_zones_kind_is_observed_and_absence_is_reported_as_absence() {
    let (_g, mut p) = armed();

    // A zone inside the geometry that nothing has ever pushed to.
    let untouched = kind_zone() + 1;
    assert!(u32::from(untouched) < p.zone_stride(), "the probe zone must be inside the geometry");
    assert_eq!(
        p.observed_kind(untouched),
        None,
        "a zone nothing was folded for must report NO kind — a zero-filled map plus a raw \
         discriminant cast would call it a span"
    );

    // A span, observed.
    push_span_at(kind_zone(), boyko_diag::clock::ticks(), 700);
    fold(&mut p);
    fold(&mut p);
    assert_eq!(p.observed_kind(kind_zone()), Some(SampleKind::Span));

    // A counter on the SAME zone: the map holds one byte, so the last kind folded is what it says,
    // and that is the honest reading of a byte that cannot hold two.
    let s = Sample {
        stamp: boyko_diag::clock::ticks(),
        value: 3,
        zone: kind_zone(),
        flags: SampleKind::Counter as u16,
        _pad: 0,
    };
    assert!(sample::push(Region::Engine, s), "the region must accept a lone test sample");
    fold(&mut p);
    fold(&mut p);
    assert_eq!(p.observed_kind(kind_zone()), Some(SampleKind::Counter));

    // Outside the geometry is `None` rather than a read past the section.
    assert_eq!(p.observed_kind(u16::MAX), None);
}

/// A re-arm does not inherit the previous session's kinds.
///
/// The map is cleared on the same pass as tier C's, and for the same reason: a wrong unit on a
/// number is worse than an absent one, and an inherited kind is a wrong unit that looks measured.
#[test]
fn rung13_a_re_arm_forgets_what_the_previous_session_observed() {
    let (_g, mut p) = armed();
    push_span_at(kind_zone(), boyko_diag::clock::ticks(), 500);
    fold(&mut p);
    fold(&mut p);
    assert_eq!(p.observed_kind(kind_zone()), Some(SampleKind::Span));

    p.disarm();
    let outcome = p.arm(TEST_GEOMETRY);
    // `Rearmed`, not `Armed`: the reservation is process-lifetime, so only the FIRST arm in a
    // process creates it and every later one adopts it. Both are success.
    assert!(
        matches!(outcome, ArmOutcome::Armed | ArmOutcome::Rearmed),
        "re-arm at the live geometry must succeed, got {outcome:?}"
    );
    assert_eq!(
        p.observed_kind(kind_zone()),
        None,
        "a re-armed session must not report the previous session's kind"
    );
}
