//! Retention tier C (D22) — log-linear histograms over the whole session, opt-in per zone.
//!
//! # What tier C is for, and what it is explicitly NOT for
//!
//! Tier A (the 121-frame ring) answers *"what happened in the last two seconds"*. Tier B
//! ([`lifetime`](super::lifetime)) answers *"what has this zone cost all session"* in four numbers.
//! Neither can answer *"what does the tail look like"* — a p99 over a session — because the ring
//! has thrown the frames away and four accumulators cannot hold a shape.
//!
//! **`resolve` does NOT consume histograms, and that is a decision rather than an omission.** The
//! bucket geometry below is chosen against this box's own measured decidability floor (4.7–14.3 %),
//! and a bucket is the same order wide. A contrast resolved through a histogram would be a
//! comparison of two quantisation grids; the window reducer's exact per-frame values are what
//! `resolve` reads.
//!
//! # The geometry, and a correction to the corpus's own description of it
//!
//! 192 buckets, 3 mantissa bits, `u16` counters — 384 B of buckets plus `total` and `count`, so
//! **400 B per slot**, exactly as specified. What is corrected is the *width*:
//!
//! > the corpus: *"3 mantissa bits ⇒ 6.25 % bucket width"*
//!
//! With 3 mantissa bits there are **8 buckets per octave**, so a bucket spans `2^e / 8` inside the
//! octave `[2^e, 2^(e+1))` — **12.5 % relative at the bottom of the octave and 6.25 % at the top**.
//! 6.25 % is the *half*-width, which is the error of reporting a bucket's MIDPOINT. This
//! implementation never reports a midpoint ([`HistView`] yields **edges**), so the figure a reader
//! should hold is the one that bounds an edge pair: **≤ 12.5 % relative**. The corpus's conclusion
//! is untouched — a bucket is still the same order as the floor, and `resolve` still must not read
//! one — only the number attached to it is made honest.
//!
//! # Bucket layout
//!
//! ```text
//! v in 0..16      bucket = v                      exact; 16 linear buckets
//! v >= 16         e = floor(log2 v)  (>= 4)
//!                 m = (v >> (e - 3)) & 7          the 3 bits below the leading one
//!                 bucket = 16 + (e - 4) * 8 + m
//! ```
//!
//! The linear head matters: a span of 3 ticks is 3 ticks, and a log grid over the first octave
//! would quantise the cheapest zones — the ones whose cost is being argued about — hardest.
//!
//! The top bucket (191) is **open-ended**: its upper edge is `u64::MAX`, not `2^26`. A closed top
//! bucket would report an upper edge a sample exceeded, which is the one thing an edge pair must
//! never do. `2^26` ticks is ~22 ms at 3 GHz — past a frame, so a span landing there is already an
//! outlier the ring's `max` describes better.

/// Buckets per slot. 24 octave-groups of 8, plus the 16-entry linear head, minus the overlap the
/// head covers — the arithmetic is in [`bucket_of`] and pinned by the tests below.
pub const HIST_BUCKETS: usize = 192;

/// Mantissa bits — buckets per octave is `1 << HIST_MANTISSA_BITS`.
pub const HIST_MANTISSA_BITS: u32 = 3;

/// Buckets in the exact linear head, covering `0..LINEAR_HEAD`.
const LINEAR_HEAD: u64 = 16;

/// Sub-buckets per octave.
const SUB: u32 = 1 << HIST_MANTISSA_BITS;

/// One zone's session histogram. **400 B**, and the width is asserted rather than asserted-in-prose.
///
/// `u16` counters: a bucket saturates at 65 535 samples, which for a once-per-frame zone is ~18
/// minutes in one bucket. Saturation is **counted** (`hist_saturations` in the drop counters), never
/// silent — a bucket that stops climbing while `count` keeps climbing is a shape that has quietly
/// stopped being a distribution.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HistSlot {
    /// Per-bucket sample counts.
    pub buckets: [u16; HIST_BUCKETS],
    /// Σ of every value fed, exact and independent of the quantisation.
    pub total: u64,
    /// Samples fed, exact — including any whose bucket saturated.
    pub count: u64,
}

const _: () = assert!(size_of::<HistSlot>() == 400, "D22 pins tier C at 400 B per slot");

impl HistSlot {
    /// The all-zero slot. A committed, zero-filled reservation section is already this.
    pub const ZERO: HistSlot = HistSlot { buckets: [0; HIST_BUCKETS], total: 0, count: 0 };
}

/// The bucket `v` falls in.
///
/// Total, saturating at the open-ended top bucket. `#[inline]` because it runs once per folded
/// sample for a subscribed zone, inside the fold's inner loop.
#[inline]
#[must_use]
pub fn bucket_of(v: u64) -> usize {
    if v < LINEAR_HEAD {
        return v as usize;
    }
    // `v >= 16` so `leading_zeros() <= 59` and `e >= 4`; the shift below is therefore `>= 1`.
    let e = 63 - v.leading_zeros();
    let m = ((v >> (e - HIST_MANTISSA_BITS)) & u64::from(SUB - 1)) as usize;
    let idx = LINEAR_HEAD as usize + (e - 4) as usize * SUB as usize + m;
    if idx >= HIST_BUCKETS { HIST_BUCKETS - 1 } else { idx }
}

/// The half-open value range `[lo, hi)` bucket `b` covers.
///
/// The top bucket's `hi` is `u64::MAX` because it is open-ended — see the module docs. A closed
/// upper edge there would be an edge a sample had exceeded.
#[must_use]
pub fn bucket_edges(b: usize) -> (u64, u64) {
    debug_assert!(b < HIST_BUCKETS, "invariant: a bucket index is inside the slot");
    if (b as u64) < LINEAR_HEAD {
        return (b as u64, b as u64 + 1);
    }
    if b == HIST_BUCKETS - 1 {
        let e = 4 + (b as u32 - LINEAR_HEAD as u32) / SUB;
        let m = (b as u64 - LINEAR_HEAD) % u64::from(SUB);
        return ((SUB as u64 + m) << (e - HIST_MANTISSA_BITS), u64::MAX);
    }
    let e = 4 + (b as u32 - LINEAR_HEAD as u32) / SUB;
    let m = (b as u64 - LINEAR_HEAD) % u64::from(SUB);
    let shift = e - HIST_MANTISSA_BITS;
    ((SUB as u64 + m) << shift, (SUB as u64 + m + 1) << shift)
}

/// A read view over one zone's histogram (D25's tier-C surface).
///
/// Borrowed rather than copied: a `HistSlot` is 400 B and a reader wants one quantile, not a copy.
#[derive(Clone, Copy)]
pub struct HistView<'a> {
    slot: &'a HistSlot,
}

impl<'a> HistView<'a> {
    /// Wrap a slot.
    #[must_use]
    pub fn new(slot: &'a HistSlot) -> HistView<'a> {
        HistView { slot }
    }

    /// Samples fed into this histogram, exact.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.slot.count
    }

    /// Σ of every value fed, exact and un-quantised.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.slot.total
    }

    /// One bucket's count.
    #[must_use]
    pub fn bucket(&self, b: usize) -> u16 {
        self.slot.buckets.get(b).copied().unwrap_or(0)
    }

    /// The **bucket edges** bracketing quantile `q`, or `None` on an empty histogram.
    ///
    /// # Edges, never a point estimate — this is the whole API decision
    ///
    /// A histogram knows which bucket the answer is in and nothing finer. Returning a midpoint, or
    /// an interpolated value, invents precision the data does not have and reads as a measurement.
    /// Returning `[lo, hi)` says exactly what is known: the true quantile is somewhere in here.
    /// `G16` asserts the ORACLE FALLS INSIDE this pair, which is a claim a point estimate could not
    /// even express.
    ///
    /// Empty is `None` rather than `(0, 0)`, for the reason the whole corpus repeats: a structural
    /// zero is indistinguishable from a measured one.
    #[must_use]
    pub fn quantile(&self, q: f64) -> Option<(u64, u64)> {
        debug_assert!((0.0..=1.0).contains(&q), "invariant: a quantile is in 0..=1");
        if self.slot.count == 0 {
            return None;
        }
        // The rank of the sample being asked for, 1-based, so `q = 1.0` names the last sample and
        // `q = 0.0` names the first. `ceil` rather than `round`: p99 must not be allowed to land
        // below the 99th percentile by a rounding rule nobody stated.
        let rank = (q * self.slot.count as f64).ceil().max(1.0) as u64;
        let mut seen = 0u64;
        for b in 0..HIST_BUCKETS {
            seen += u64::from(self.slot.buckets[b]);
            if seen >= rank {
                return Some(bucket_edges(b));
            }
        }
        // Reachable only when buckets saturated and their sum no longer reaches `count` — the
        // condition `hist_saturations` exists to report. The top bucket is the honest answer: the
        // missing mass is at the top, because that is the only place saturation hides it.
        Some(bucket_edges(HIST_BUCKETS - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every value lands in a bucket whose edges contain it. The property the whole tier rests on.
    #[test]
    fn every_value_is_inside_the_edges_of_its_own_bucket() {
        // Dense over the linear head and the first octaves, then sparse and wide.
        for v in 0..4096u64 {
            let b = bucket_of(v);
            let (lo, hi) = bucket_edges(b);
            assert!(lo <= v && v < hi, "value {v} fell in bucket {b} = [{lo}, {hi})");
        }
        for shift in 0..64u32 {
            for delta in [0u64, 1, 3] {
                let x = (1u64 << shift).saturating_add(delta);
                let b = bucket_of(x);
                let (lo, hi) = bucket_edges(b);
                assert!(lo <= x && x < hi, "value {x} fell in bucket {b} = [{lo}, {hi})");
            }
        }
    }

    /// The buckets tile the value space with no gap and no overlap.
    ///
    /// A gap would make some value land in a bucket whose edges exclude it; an overlap would make
    /// two buckets claim one value and the quantile walk double-count it.
    #[test]
    fn the_buckets_are_contiguous_and_cover_everything() {
        let (lo0, _) = bucket_edges(0);
        assert_eq!(lo0, 0, "the grid must start at zero");
        for b in 1..HIST_BUCKETS {
            let (_, prev_hi) = bucket_edges(b - 1);
            let (lo, _) = bucket_edges(b);
            assert_eq!(lo, prev_hi, "bucket {b} does not begin where {} ended", b - 1);
        }
        let (_, top_hi) = bucket_edges(HIST_BUCKETS - 1);
        assert_eq!(top_hi, u64::MAX, "the top bucket must be open-ended");
    }

    /// Monotone: a larger value never lands in a lower bucket.
    #[test]
    fn the_mapping_is_monotone() {
        let mut last = 0usize;
        for v in 0..100_000u64 {
            let b = bucket_of(v);
            assert!(b >= last, "value {v} went backwards, {last} -> {b}");
            last = b;
        }
    }

    /// The corrected width claim, asserted rather than left in prose: a bucket is at most 12.5 %
    /// of its own lower edge, and 6.25 % is the HALF-width the corpus quoted.
    #[test]
    fn a_bucket_is_at_most_one_eighth_of_its_lower_edge() {
        for b in (LINEAR_HEAD as usize)..(HIST_BUCKETS - 1) {
            let (lo, hi) = bucket_edges(b);
            let width = hi - lo;
            assert!(
                width * 8 <= lo,
                "bucket {b} = [{lo}, {hi}) is wider than an eighth of its lower edge"
            );
        }
    }

    /// An empty histogram refuses rather than answering zero.
    #[test]
    fn an_empty_histogram_has_no_quantile() {
        let slot = HistSlot::ZERO;
        assert!(HistView::new(&slot).quantile(0.99).is_none());
    }
}
