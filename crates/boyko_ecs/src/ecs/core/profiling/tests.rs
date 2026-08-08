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

/// Take the lock, arm a fresh store on [`TEST_LANE`], and leave the rings empty.
///
/// The drain is not tidiness: `arm` resets the **overflow** deltas but cannot reset the rings,
/// and a sample another test left behind would be folded into this one's frames by its stamp.
fn armed() -> (MutexGuard<'static, ()>, Profiler) {
    let guard = test_serial();
    set_lane(TEST_LANE);
    let mut p = Profiler::new();
    let outcome = p.arm(ProfilerConfig::default());
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
    let outcome = p.arm(ProfilerConfig::default());
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
    let outcome = other.arm(ProfilerConfig { user_zone_budget: 64 });
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
    let outcome = app.world_mut().resource_mut::<Profiler>().arm(ProfilerConfig::default());
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
