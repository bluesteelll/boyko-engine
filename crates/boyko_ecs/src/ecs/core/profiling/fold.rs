//! The fold: drain every lane region, attribute each sample to the frame its `stamp` falls in,
//! and accumulate into the frame-major columns.
//!
//! # Where it runs, and why not in a system
//!
//! At the **top of `App::update_with_delta`**, before step ①. That is the single funnel both
//! entry points share: the windowed host calls `app.update_with_delta(dt)` directly and never
//! touches `App::update`, so a fold placed in the latter would never run in the one configuration
//! that has a GPU channel — lanes would fill, `overflow` would climb, and no frame would ever
//! seal, with every gate green.
//!
//! It is also what puts the instrument **outside its own primary number**: the fold is not inside
//! `__frame`, so the frame time it reports does not include the cost of reporting it.
//!
//! # Attribution reads `stamp` and only `stamp`
//!
//! For all three kinds, before the kind dispatch. An earlier design consumed a field whose meaning
//! varied by kind — a timestamp for spans, a payload for counters and gauges — so counters landed
//! in `late` and large gauge values truncated a region's fold. The rule here is structural rather
//! than ordering-dependent: **no field whose meaning varies by kind may be read before the
//! `match`**, and `stamp` is the only field whose meaning does not.
//!
//! # The walk is bidirectional, and that is not an optimisation
//!
//! A region is **not** TSC-monotone. A `Span` stamps at OPEN and is written at CLOSE, so a nested
//! pair writes the inner span (later stamp) before the outer (earlier stamp). A forward-only walk
//! would attribute the outer span to the inner one's frame. The walk keeps the previous sample's
//! frame as a hint and moves **both ways** from it, bounded by the retained window; a stamp older
//! than the window is `late`. The common case — consecutive samples in the same frame — costs one
//! comparison.
//!
//! # This rung drains the whole region rather than stopping at the cut
//!
//! The corpus's step 3 stops a region at the first sample whose `stamp >= cut` and defers the
//! rest, which costs a long outer span one extra fold. That stop exists because the corpus's
//! ordering opens the new frame **after** the drain, so a sample past the cut belongs to a frame
//! that does not exist yet.
//!
//! This fold opens the frame **first**, so the live frame is open with `cpu_begin == cut` and a
//! sample past the cut is attributed to it by the same walk, on the same rule. Nothing is deferred
//! and nothing is attributed differently — a sample lands in the frame containing its stamp either
//! way. What changes is that a slot is freed one fold earlier, which is strictly less overflow
//! pressure on the region. **The deferred-outer-span cost the corpus states does not exist here**,
//! and this note is why.

use boyko_diag::lane::LANE_COUNT;
use boyko_diag::loss::{self, DiagFlag, LossClass};
use boyko_diag::sample::{Region, Sample, SampleKind};
use boyko_diag::{clock, profiling_abi, sample};

use crate::ecs::core::profiling::diag;
#[cfg(feature = "profiling-analysis")]
use crate::ecs::core::profiling::store::{
    INTERVALS_PER_FRAME, Interval, IntervalRing, OVERLAP_FRAMES,
};
use crate::ecs::core::profiling::hist;
use crate::ecs::core::profiling::store::{
    CellLabel, Columns, DropCounters, FRAME_FLAG_CLOCK_UNCALIBRATED, FrameRecord, FrameState,
    MAX_PLAUSIBLE_FRAME_TICKS, Profiler, Tiers, WINDOW,
};

/// Run one fold.
///
/// The disarmed cost is **one `.bss` load and one predicted branch** — the mask read happens
/// before any resource is touched, which is what makes "off costs address space, not resident
/// memory" true at the call site rather than only at the emitter.
pub fn fold(profiler: &mut Profiler) {
    if !profiler.is_armed() {
        return;
    }
    debug_assert!(
        !boyko_threadpool::is_in_system_run(),
        "invariant: the fold runs between schedule runs, with no workers in flight"
    );

    // Destructive, and this is its single consumer: a second caller of `take_raised` would clear
    // bits this one never saw.
    let raised = loss::take_raised();
    diag::report_raised(raised);
    let uncalibrated = raised & DiagFlag::ClockUncalibrated.as_bits() != 0;

    let now = clock::ticks();
    let drops_at_entry = profiler.drops().total();

    if epoch_broke(profiler, now) {
        // The window is gone; the frame opened below starts a clean one. Nothing else in this
        // fold is skipped — the lanes still hold samples, and refusing to drain them would turn a
        // discarded window into a stuck region.
        profiler.discard_window();
        reopen_after_discard(profiler, now, uncalibrated);
        return;
    }

    let prev_row = profiler.cursor();
    open_next_frame(profiler, now, uncalibrated);

    // The begins are copied into L1 once rather than read from the reservation per sample: the
    // walk touches them on every sample, and 121 `u64` is under a page.
    let mut begins = [0u64; WINDOW];
    for (row, slot) in begins.iter_mut().enumerate() {
        *slot = profiler.begin_of_row(row as u32);
    }

    let Some(cols) = profiler.columns_for_fold() else {
        debug_assert!(false, "invariant: an armed store has columns");
        return;
    };
    let Some(tiers) = profiler.tiers_for_fold() else {
        debug_assert!(false, "invariant: an armed store has retention tiers");
        return;
    };
    let stride = profiler.zone_stride();
    let live = profiler.frame();
    let floor = live + 1 - u32::min(live + 1, WINDOW as u32);

    // Derived once, beside the columns and for the same reason: the ring's base is a constant for
    // the whole drain, and re-deriving it per sample would put a load and an add in the inner loop
    // to recompute a value that cannot change.
    #[cfg(feature = "profiling-analysis")]
    let ring = profiler.interval_ring();

    // Per-row sample tallies, applied to the frame records once at the end. Accumulating them in
    // the records themselves would put a 32 B record write in the inner loop, on a line the
    // columns are not on.
    let mut tally = [0u32; WINDOW];

    let state = profiler.fold_state();
    for lane in 0..LANE_COUNT {
        for (i, region) in [Region::Engine, Region::User].into_iter().enumerate() {
            // SAFETY: this is the region's single consumer. The fold runs on the dispatcher/host
            //   thread between schedule runs, holding `&mut Profiler`; the kernel's resource
            //   borrow rules hand out exactly one, and there is no other caller of `drain_region`
            //   in the engine. That borrow IS the exclusivity the contract asks for.
            unsafe {
                sample::drain_region(lane, region, |s| {
                    match attribute(&begins, live, floor, *state.walk_hint, s.stamp) {
                        Some(f) => {
                            *state.walk_hint = f;
                            let row = (f % WINDOW as u32) as usize;
                            tally[row] = tally[row].saturating_add(1);
                            let occ = accumulate(&cols, &tiers, stride, row, &s, state.drops);
                            #[cfg(feature = "profiling-analysis")]
                            if let (Some(ring), Some(occ)) = (ring, occ) {
                                append_interval(
                                    ring,
                                    live,
                                    f,
                                    &s,
                                    occ,
                                    state.interval_len,
                                    state.drops,
                                );
                            }
                            #[cfg(not(feature = "profiling-analysis"))]
                            let _ = occ;
                        }
                        None => state.drops.late += 1,
                    }
                });
            }

            // Q2(b): the counter is monotone and the delta lives here, in the consumer. Nothing
            // clears the producer's cell, so there is no window for a clear to race an increment.
            let seen = &mut state.overflow_seen[lane as usize][i];
            let delta = sample::overflow_since(lane, region, seen);
            match region {
                Region::Engine => state.drops.engine_overflow += delta,
                Region::User => state.drops.user_overflow += delta,
            }
        }
    }

    // The substrate's un-laned row: samples from threads the lane topology could not place. The
    // same monotone-plus-delta shape, through the substrate's own accessor.
    let cell = loss::cell_at_row(loss::ROW_UNLANED, LossClass::Unclaimed);
    state.drops.unclaimed += loss::delta_since(cell, state.unclaimed_seen).count;

    // The borrow ends at this last read; NLL is what releases `profiler` for the writes below.
    let drops = *state.drops;

    apply_tally(profiler, &tally);
    seal(profiler, prev_row, now, (drops.total() - drops_at_entry) as u32);

    if drops.engine_overflow + drops.user_overflow > 0 {
        diag::report_overflow(drops.engine_overflow, drops.user_overflow);
    }
    if drops.unclaimed > 0 {
        diag::report_lane_exhausted();
    }
    if drops.late > 0 {
        diag::report_late(drops.late);
    }
}

/// Whether the gap since the previous fold is a clock jump rather than a slow frame.
///
/// Charges the epoch break and recalibrates as a side effect, because both belong to the
/// detection and splitting them would let a caller detect without recording.
#[inline]
fn epoch_broke(profiler: &mut Profiler, now: u64) -> bool {
    let state = profiler.fold_state();
    let last = *state.last_fold;
    *state.last_fold = now;
    if !is_forward_jump(now, last) {
        return false;
    }
    state.drops.clock_epoch_breaks += 1;
    break_epoch(now);
    true
}

/// The detector's whole decision, as a pure function of its two inputs.
///
/// Split out so the **boundary** is assertable. Through `fold` it is not: `now` is read inside the
/// fold, after the test has injected `last`, so an injection aimed at exactly the threshold lands
/// however many ticks past it the intervening code took. A test that cannot place its input on the
/// boundary cannot tell `>` from `>=`, which is the one thing a threshold gets wrong.
///
/// `last == 0` is "no previous fold": the first fold of a session has nothing to compare against,
/// and treating that absence as a jump would discard the window at every arm.
#[inline]
#[must_use]
const fn is_forward_jump(now: u64, last: u64) -> bool {
    last != 0 && now.wrapping_sub(last) > MAX_PLAUSIBLE_FRAME_TICKS
}

/// The cold half of the detector: publish the epoch bump, report it and re-probe the scale.
#[cold]
#[inline(never)]
fn break_epoch(now: u64) {
    clock::note_forward_jump(now);
    diag::report_epoch_break();
    clock::calibrate();
}

/// Open frame 0 of a fresh window after a discard.
fn reopen_after_discard(profiler: &mut Profiler, now: u64, uncalibrated: bool) {
    let epoch = clock::clock_epoch();
    let state = profiler.fold_state();
    *state.cursor = 0;
    *state.frame = 0;
    *state.epoch = epoch;
    *state.walk_hint = 0;
    // The whole ring, not one bank: a discard throws away the window, and seven banks still
    // holding the discarded epoch's intervals would be read as the new epoch's.
    #[cfg(feature = "profiling-analysis")]
    {
        *state.interval_len = [0; OVERLAP_FRAMES];
    }
    profiler.write_frame(0, open_record(0, now, epoch, uncalibrated));
    profiler.write_begin(0, now);
}

/// Advance the cursor, recycle the row it lands on and open the next frame in it.
fn open_next_frame(profiler: &mut Profiler, now: u64, uncalibrated: bool) {
    let epoch = clock::clock_epoch();
    let state = profiler.fold_state();
    *state.cursor = (*state.cursor + 1) % WINDOW as u32;
    *state.frame += 1;
    *state.epoch = epoch;
    let (row, frame) = (*state.cursor, *state.frame);

    // The bank this frame claims is the one frame `n - OVERLAP_FRAMES` left behind. Emptying it
    // here is the ring's whole recycle, and it is the same rule as `zero_row`'s: a slot is
    // recycled when the frame that owns it opens, never lazily on read.
    #[cfg(feature = "profiling-analysis")]
    {
        state.interval_len[frame as usize % OVERLAP_FRAMES] = 0;
    }

    // The recycle is what stops a row from reporting the frame it held `WINDOW` frames ago.
    profiler.zero_row(row);
    profiler.write_frame(row, open_record(frame, now, epoch, uncalibrated));
    profiler.write_begin(row, now);
}

/// A freshly opened frame's record.
#[inline]
fn open_record(frame: u32, now: u64, epoch: u32, uncalibrated: bool) -> FrameRecord {
    FrameRecord {
        frame,
        drops: 0,
        cpu_begin: now,
        cpu_end: 0,
        samples: 0,
        clock_epoch: epoch as u16,
        state: FrameState::Pending,
        flags: if uncalibrated { FRAME_FLAG_CLOCK_UNCALIBRATED } else { 0 },
    }
}

/// Add this fold's per-row sample tallies to the frame records.
fn apply_tally(profiler: &mut Profiler, tally: &[u32; WINDOW]) {
    for (row, added) in tally.iter().copied().enumerate() {
        if added == 0 {
            continue;
        }
        let row = row as u32;
        let Some(mut rec) = profiler.frame_record(row) else { continue };
        rec.samples = rec.samples.saturating_add(added);
        profiler.write_frame(row, rec);
    }
}

/// Close the frame at `row`.
fn seal(profiler: &mut Profiler, row: u32, now: u64, drops: u32) {
    let Some(mut rec) = profiler.frame_record(row) else { return };
    rec.cpu_end = now;
    rec.drops = drops;
    // Every folded frame seals at this rung. The corpus's third state, `Partial`, is for a frame
    // whose GPU slot never retired, and there is no GPU channel here — so a frame that folded is a
    // frame that is complete, and saying so is not an assumption but the absence of the only
    // mechanism that could make it false.
    rec.state = FrameState::Sealed;
    profiler.write_frame(row, rec);
}

/// The absolute frame `stamp` belongs to, or `None` when it predates the retained window.
///
/// `hint` is the previous sample's answer — a hint and never a bound: the walk moves both ways
/// from it and re-derives the result, so a wrong hint costs steps and never an attribution.
#[inline]
fn attribute(
    begins: &[u64; WINDOW],
    live: u32,
    floor: u32,
    hint: u32,
    stamp: u64,
) -> Option<u32> {
    debug_assert!(floor <= live, "invariant: the window floor is at or below the live frame");
    let mut f = hint.clamp(floor, live);
    loop {
        if stamp < begins[(f % WINDOW as u32) as usize] {
            if f == floor {
                return None;
            }
            f -= 1;
        } else if f < live && stamp >= begins[((f + 1) % WINDOW as u32) as usize] {
            f += 1;
        } else {
            return Some(f);
        }
    }
}

/// Fold one sample into its cell.
///
/// The kind dispatch happens **after** attribution, on a field that has already been read — see
/// the module docs on why that order is a rule rather than a preference.
///
/// Returns the **occurrence index of a folded `Span`** within its `(frame, zone)` cell — the
/// cell's `count` as it stood *before* this sample's increment — and `None` for anything else: a
/// rejected sample, or a `Counter`/`Gauge`, neither of which is an interval. That number is the
/// interval ring's `occ`, and returning it here is what lets the ring carry no occurrence counter
/// of its own; the fold has the value in a register either way.
#[inline]
fn accumulate(
    cols: &Columns,
    tiers: &Tiers,
    stride: u32,
    row: usize,
    s: &Sample,
    drops: &mut DropCounters,
) -> Option<u32> {
    if u32::from(s.zone) >= stride {
        // A zone id past the armed geometry. Reachable only by arming a smaller stride than the
        // registry has already minted into, which `E9213` refuses on the arm path — so this is the
        // residual, and dropping the sample is the only attribution that is not a lie.
        drops.late += 1;
        return None;
    }
    let idx = row * stride as usize + s.zone as usize;
    debug_assert!(idx < cols.cells, "invariant: a folded cell is inside the columns");

    let Some(kind) = s.kind() else {
        // The reserved kind encoding. It has no producer at this rung — `ZoneGuard::drop` writes
        // `Span` and nothing else does — so a sample carrying it came from a writer this build
        // does not have. Guessing which kind it meant is how a decoder invents data.
        debug_assert!(false, "invariant: no writer at this rung emits the reserved sample kind");
        return None;
    };

    let (clamped, over) = if s.value > u64::from(u32::MAX) {
        (u32::MAX, true)
    } else {
        (s.value as u32, false)
    };

    // SAFETY: `idx < cells` (asserted above and bounded by the `stride` test), every column holds
    //   `cells` initialised elements inside the committed reservation, and this thread holds the
    //   store's `&mut` — clause (a) of `Profiler`'s `Send` impl. The five columns are disjoint
    //   sections, so no two of these accesses alias.
    let occurrence;
    unsafe {
        let count = cols.count.add(idx);
        let total = cols.total.add(idx);
        let min = cols.min.add(idx);
        let max = cols.max.add(idx);
        let label = cols.label.add(idx);

        let n = count.read();
        occurrence = n;
        match kind {
            // A counter's cell ACCUMULATES: a rate needs the frame's sum, and an assignment
            // cannot support one. A span's does too — the sum of its durations.
            SampleKind::Span | SampleKind::Counter => {
                total.write(total.read().wrapping_add(s.value));
            }
            // A gauge is a level: the last one in the frame wins. The running total a reader might
            // want belongs in the lifetime accumulator (rung 12), not here, where it would make
            // `total` mean two things.
            SampleKind::Gauge => total.write(s.value),
        }
        count.write(n.saturating_add(1));

        if n == 0 {
            min.write(clamped);
            max.write(clamped);
        } else {
            if clamped < min.read() {
                min.write(clamped);
            }
            if clamped > max.read() {
                max.write(clamped);
            }
        }

        if over {
            label.write(CellLabel::OverRange as u8);
        } else if label.read() == CellLabel::Empty as u8 {
            label.write(CellLabel::Measured as u8);
        }
    }

    // ── retention tiers B and C (D22, rung 12) ──────────────────────────────────────────────
    //
    // PER SAMPLE, not per sealed row, and the difference is a defect rather than a preference. A
    // span stamps at OPEN and is written at CLOSE, so one crossing a frame boundary is drained a
    // fold later than the frame it belongs to — after that frame's row was sealed. A pass over the
    // sealed row would therefore miss it, and it misses longest spans most often, which are the
    // ones tier B exists to total. Here the sample reaches both tiers exactly once, by construction.
    //
    // SAFETY: `s.zone < stride == tiers.zones` (tested at the top of this function, and both come
    //   from the same `arm`), so the accumulator and the map byte are inside their sections, which
    //   `Layout::new` sized at `zone_stride` elements each inside the committed reservation. `arm`
    //   seeded every accumulator to `LifetimeAcc::EMPTY` and zeroed the map before publishing the
    //   mask. The fold holds `&mut Profiler` — clause (a) of `Profiler`'s `Send` impl — so no other
    //   thread writes these. The three sections are disjoint from each other and from the columns.
    unsafe {
        debug_assert!((s.zone as usize) < tiers.zones, "invariant: a folded zone is inside the tiers");
        (*tiers.lifetime.add(s.zone as usize)).push(s.value, clamped);

        let mapped = tiers.hist_of.add(s.zone as usize).read();
        if mapped != 0 {
            let slot = &mut *tiers.hist.add(mapped as usize - 1);
            let b = hist::bucket_of(s.value);
            let cell = &mut slot.buckets[b];
            if *cell == u16::MAX {
                // A `u16` bucket at 65 535 is ~18 minutes of a once-per-frame zone in one bucket.
                // Counted rather than wrapped: a wrapped bucket makes the shape LOOK like a
                // different distribution, where a saturated one plus this counter says exactly what
                // is missing and where.
                drops.hist_saturations += 1;
            } else {
                *cell += 1;
            }
            // `total` and `count` stay EXACT regardless of the bucket, so a saturated histogram
            // still reports a true mean — the quantiles are what degrade, and only they.
            slot.total = slot.total.wrapping_add(s.value);
            slot.count += 1;
        }
    }

    if over && kind == SampleKind::Span {
        // The corpus's counter names spans, and it is kept literal: `total` and `count` stay exact
        // for every kind, and what a clamp costs is the extrema — which the cell's own
        // `OverRange` label reports for all three.
        drops.span_over_range += 1;
    }

    if kind == SampleKind::Span { Some(occurrence) } else { None }
}

/// Append one span to its frame's bank in the interval ring.
///
/// # Two refusals that are not the same thing, and only one of them is a loss
///
/// A frame outside the ring's [`OVERLAP_FRAMES`]-frame horizon has no bank, and this returns
/// without counting: the span's measurement is in its column cell regardless, and the horizon is a
/// stated bound of the analysis rather than a sample the instrument lost. Counting it would report
/// one sample under two headings — the double-count `substrate/loss-fold` exists to remove.
///
/// A **full** bank is a loss: that span is missing from the overlap analysis and nothing else
/// records the fact. It increments `intervals_dropped`, and the report carries the figure so a
/// serialisation index computed over a truncated bank is never handed over as if it were complete.
#[cfg(feature = "profiling-analysis")]
#[inline]
fn append_interval(
    ring: IntervalRing,
    live: u32,
    frame: u32,
    s: &Sample,
    occ: u32,
    lens: &mut [u32; OVERLAP_FRAMES],
    drops: &mut DropCounters,
) {
    if live.wrapping_sub(frame) >= OVERLAP_FRAMES as u32 {
        return;
    }
    let bank = frame as usize % OVERLAP_FRAMES;
    let len = lens[bank] as usize;
    if len >= INTERVALS_PER_FRAME {
        drops.intervals_dropped += 1;
        return;
    }

    let dur = if s.value > u64::from(u32::MAX) { u32::MAX } else { s.value as u32 };
    // A zone opening more than 65 535 times in one frame saturates the occurrence index rather
    // than wrapping it: a wrapped `occ` would name occurrence 0, and two intervals claiming to be
    // the same occurrence is a statement no reader can recover from. The cell's `count` stays
    // exact, so the number of occurrences is never the thing that is lost.
    let occ = u16::try_from(occ).unwrap_or(u16::MAX);

    // SAFETY: `bank < OVERLAP_FRAMES` (a modulus) and `len < INTERVALS_PER_FRAME` (tested just
    //   above), so the slot lies inside the ring section, which `Layout::new` sized at
    //   `OVERLAP_FRAMES * INTERVALS_PER_FRAME` `Interval`s inside the committed reservation. The
    //   fold holds `&mut Profiler` — clause (a) of `Profiler`'s `Send` impl — and is the ring's
    //   only writer, so no other thread can be writing this slot.
    unsafe {
        ring.base
            .add(bank * INTERVALS_PER_FRAME + len)
            .write(Interval { begin: s.stamp, dur, zone: s.zone, occ });
    }
    lens[bank] = lens[bank].saturating_add(1);
}

/// The mask read that fronts the whole subsystem, exposed so `App` does not name
/// `boyko_diag::profiling_abi` itself.
#[inline]
#[must_use]
pub fn any_armed() -> bool {
    profiling_abi::any_armed()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn begins_of(xs: &[u64]) -> [u64; WINDOW] {
        let mut b = [0u64; WINDOW];
        for (i, v) in xs.iter().copied().enumerate() {
            b[i] = v;
        }
        b
    }

    /// The common case: a sample inside the live frame, found without moving.
    #[test]
    fn a_stamp_in_the_live_frame_is_attributed_to_it() {
        let b = begins_of(&[100, 200, 300]);
        assert_eq!(attribute(&b, 2, 0, 2, 350), Some(2));
        assert_eq!(attribute(&b, 2, 0, 2, 300), Some(2), "the begin belongs to its own frame");
    }

    /// The nesting case, which a forward-only walk gets wrong: an outer span stamped in an EARLIER
    /// frame is written after an inner one stamped later, so the hint points forward of the answer.
    #[test]
    fn the_walk_moves_backwards_from_a_hint_that_is_too_high() {
        let b = begins_of(&[100, 200, 300]);
        assert_eq!(attribute(&b, 2, 0, 2, 150), Some(0), "a forward-only walk would say 2");
        assert_eq!(attribute(&b, 2, 0, 2, 250), Some(1));
    }

    /// And forwards, from a stale hint left by the previous fold.
    #[test]
    fn the_walk_moves_forwards_from_a_hint_that_is_too_low() {
        let b = begins_of(&[100, 200, 300]);
        assert_eq!(attribute(&b, 2, 0, 0, 350), Some(2));
        assert_eq!(attribute(&b, 2, 0, 0, 200), Some(1));
    }

    /// A stamp older than the oldest retained frame is `late` — not silently attributed to the
    /// floor, which would put a sample in a frame it did not happen in.
    #[test]
    fn a_stamp_below_the_window_floor_is_late() {
        let b = begins_of(&[100, 200, 300]);
        assert_eq!(attribute(&b, 2, 0, 1, 99), None);
        // With the floor raised, a stamp inside frame 0 is equally gone.
        assert_eq!(attribute(&b, 2, 1, 1, 150), None);
    }

    /// The threshold, on the boundary. `>` and `>=` differ by exactly this one tick, and nothing
    /// driven through `fold` can place an input here — see [`is_forward_jump`]'s own docs.
    #[test]
    fn the_jump_threshold_is_exclusive_and_the_first_fold_is_never_one() {
        let t = 1_000_000_000_000u64;
        assert!(!is_forward_jump(t + MAX_PLAUSIBLE_FRAME_TICKS, t), "a gap AT the bound is a frame");
        assert!(is_forward_jump(t + MAX_PLAUSIBLE_FRAME_TICKS + 1, t), "one tick past is a jump");
        assert!(!is_forward_jump(u64::MAX, 0), "the first fold of a session has no gap to measure");
    }

    /// The hint is a hint: every entry point produces the same answer.
    #[test]
    fn the_answer_does_not_depend_on_the_hint() {
        let b = begins_of(&[10, 20, 30, 40, 50]);
        for stamp in [10u64, 15, 25, 33, 44, 60] {
            let expected = attribute(&b, 4, 0, 0, stamp);
            for hint in 0..=4u32 {
                assert_eq!(attribute(&b, 4, 0, hint, stamp), expected, "hint {hint} changed the answer");
            }
        }
    }
}
