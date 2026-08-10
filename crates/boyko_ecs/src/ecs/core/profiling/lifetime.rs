//! Retention tier B (D22) — one 24-byte accumulator per zone, for the whole session.
//!
//! # Why the ring is not enough
//!
//! Tier A retains 121 frames, ~2 s. A question like *"how much has this system cost since the level
//! loaded"* is not answerable from it at any window size, because the answer is older than the
//! window by construction. Four numbers per zone answer it at `24 × Z` — 24 KiB at `Z = 1024`,
//! 168 KiB at this tree's `Z = 7168` — and they answer it exactly, not approximately.
//!
//! # `min` is `u32::MAX` on a zone that never ran, and that is REPORTED, never returned as a value
//!
//! A zero `min` would mean "the fastest run took 0 ticks", which is a measurement. `u32::MAX` is
//! the identity for the running minimum, so an untouched accumulator carries it — and
//! [`LifetimeAcc::min_ticks`] returns `None` for exactly that state. D22's data-structure block
//! names this ("*`min` on a zone that never received a sample stays `u32::MAX` and is REPORTED AS
//! 'no samples', never as a value*"), and it is the single most likely place for this tier to lie.
//!
//! # WHERE the fold happens — a deviation from the corpus, with the defect it avoids
//!
//! The corpus folds tier B *"in one sequential pass over the current frame's row, which the fold
//! just touched, so it is L1-warm"*. **That pass loses samples, and the ones it loses are the ones
//! this tier exists for.**
//!
//! A span STAMPS at open and is WRITTEN at close. A span that opens in frame `F` and closes in
//! `F+1` is drained by the fold at the top of `F+2` and attributed — correctly — to `F`, by the
//! bidirectional walk. But `F`'s row was swept into the accumulators one fold earlier, when `F` was
//! sealed. That sample increments `F`'s cell and never reaches the lifetime accumulator. The longer
//! a span is, the more likely it is to cross a frame boundary, so the row pass under-counts
//! **exactly the expensive zones**.
//!
//! What ships accumulates **per sample**, at the one site that already has the value in a register:
//! [`fold::accumulate`](super::fold). Every folded sample reaches tier B exactly once, so
//! `G18`'s "Σ per-frame count" identity holds by construction rather than by the window happening
//! not to contain a long span. The cost is four operations on one cache line per sample, inside
//! `__fold` and therefore disclosed (D16); the row pass would have cost one sequential walk of `Z`
//! cells per frame whether or not any zone ran, which at `Z = 7168` is not obviously cheaper.

/// One zone's whole-session accumulator. **24 B**, pinned.
///
/// `total` and `count` are `u64` because a session is long: at 1000 samples/frame and 60 Hz a `u32`
/// count wraps in 20 hours, and a wrapped count is a wrong answer rather than a missing one.
/// `min`/`max` are `u32` because they mirror the ring's clamped extrema — a value past `u32::MAX`
/// is already labelled `OverRange` in its cell and counted in `span_over_range`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LifetimeAcc {
    /// Σ of every value folded for this zone. `Span`: ticks. `Counter`: increments. `Gauge`: the
    /// sum of the levels seen, which is why [`mean_ticks`](Self::mean_ticks) is documented as
    /// meaningful only for spans.
    pub total: u64,
    /// Samples folded for this zone, all kinds.
    pub count: u64,
    /// Largest value seen, clamped to `u32::MAX`. `0` on a zone that never ran.
    pub max: u32,
    /// Smallest value seen, clamped to `u32::MAX`. **`u32::MAX` means "never ran"** — read it
    /// through [`min_ticks`](Self::min_ticks), which says so.
    pub min: u32,
}

const _: () = assert!(size_of::<LifetimeAcc>() == 24, "D22 pins tier B at 24 B per zone");

impl LifetimeAcc {
    /// The identity element: the state a committed, zero-filled section must be corrected to.
    ///
    /// ⚠️ **Not the all-zero bit pattern**, because `min` starts at `u32::MAX`. The section is
    /// zero-filled by the reservation, so `arm` seeds `min` explicitly — a tier B that skipped that
    /// step would report every zone's fastest run as 0 ticks, which is a *plausible* number and
    /// therefore the worst kind of wrong.
    pub const EMPTY: LifetimeAcc =
        LifetimeAcc { total: 0, count: 0, max: 0, min: u32::MAX };

    /// Whether this zone ever produced a sample.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The smallest value seen, or `None` when the zone never ran.
    #[must_use]
    pub fn min_ticks(&self) -> Option<u32> {
        if self.count == 0 { None } else { Some(self.min) }
    }

    /// The largest value seen, or `None` when the zone never ran.
    #[must_use]
    pub fn max_ticks(&self) -> Option<u32> {
        if self.count == 0 { None } else { Some(self.max) }
    }

    /// Mean value per sample, or `None` on an empty accumulator.
    ///
    /// Meaningful for `Span` and `Counter`. For a `Gauge` this is the mean of the levels *sampled*,
    /// which is a different quantity from the mean level over time and is not offered as one.
    #[must_use]
    pub fn mean_ticks(&self) -> Option<f64> {
        if self.count == 0 { None } else { Some(self.total as f64 / self.count as f64) }
    }

    /// Fold one sample in.
    ///
    /// `#[inline]`: this runs once per folded sample from the fold's inner loop, and the body is
    /// four operations on one line.
    #[inline]
    pub fn push(&mut self, value: u64, clamped: u32) {
        self.total = self.total.wrapping_add(value);
        self.count += 1;
        if clamped > self.max {
            self.max = clamped;
        }
        if clamped < self.min {
            self.min = clamped;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The empty state reports absence rather than a value.
    #[test]
    fn an_untouched_accumulator_has_no_minimum_to_report() {
        let a = LifetimeAcc::EMPTY;
        assert!(a.is_empty());
        assert_eq!(a.min_ticks(), None, "a zone that never ran has no fastest run");
        assert_eq!(a.max_ticks(), None);
        assert_eq!(a.mean_ticks(), None);
        assert_eq!(a.min, u32::MAX, "the identity for a running minimum is not zero");
    }

    /// One sample makes every reading defined, and `min == max` on a single sample.
    #[test]
    fn one_sample_defines_every_reading() {
        let mut a = LifetimeAcc::EMPTY;
        a.push(700, 700);
        assert_eq!(a.count, 1);
        assert_eq!(a.total, 700);
        assert_eq!(a.min_ticks(), Some(700));
        assert_eq!(a.max_ticks(), Some(700));
        assert_eq!(a.mean_ticks(), Some(700.0));
    }

    /// The extrema track both directions, and the order of arrival does not matter.
    #[test]
    fn the_extrema_do_not_depend_on_arrival_order() {
        let mut asc = LifetimeAcc::EMPTY;
        let mut desc = LifetimeAcc::EMPTY;
        for v in [3u64, 9, 1, 27, 5] {
            asc.push(v, v as u32);
        }
        for v in [5u64, 27, 1, 9, 3] {
            desc.push(v, v as u32);
        }
        assert_eq!(asc, desc);
        assert_eq!(asc.min_ticks(), Some(1));
        assert_eq!(asc.max_ticks(), Some(27));
        assert_eq!(asc.total, 45);
        assert_eq!(asc.count, 5);
    }

    /// A clamped sample contributes its EXACT value to `total` and its clamp to the extrema.
    ///
    /// The two fields answer different questions and the clamp belongs to only one of them: a sum
    /// that silently used `u32::MAX` would understate a session by however much the clamp ate.
    #[test]
    fn a_clamped_sample_keeps_its_exact_total() {
        let mut a = LifetimeAcc::EMPTY;
        let huge = u64::from(u32::MAX) + 1000;
        a.push(huge, u32::MAX);
        assert_eq!(a.total, huge, "the sum must carry the value, not the clamp");
        assert_eq!(a.max_ticks(), Some(u32::MAX));
    }
}
