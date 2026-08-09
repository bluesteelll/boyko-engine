//! Profiling rung 7 — the window reducer: retired GPU pairs in, [`ZoneRow`]s out.
//!
//! [`super::artifact`] is the file; this is what fills it. It takes the `PairResult` slices the
//! `GpuZoneRecorder` hands its retire sink, accumulates per zone across a window of frames, and
//! reduces to the median / mean / p95 / offsets the artifact carries.
//!
//! # It has no console form, and that is the point
//!
//! The corpus states the reducer *"has no console form at all. Every value it produces goes into
//! the TOML artifact or the binary stream"*, and says why: **that is what lets rung 7 delete the
//! stdout measurement channel.** A reducer with a `print` would leave the channel alive under
//! another name.
//!
//! # THE STATISTICS ARE THE SHIPPED ONES, DELIBERATELY
//!
//! [`stats_ns`] is `runner.rs`'s `vb_bench_stats_ns`, conventions and all: mean over the raw
//! samples; median as the average of the two central values on an even count; p95 as
//! `sorted[(n * 0.95) as usize]` clamped to the last index. That is not laziness — rung 7a's whole
//! justification for writing figures at one decimal was that they stay **directly comparable with
//! the printed lines**, and a different median convention makes them incomparable for a reason
//! that has nothing to do with the channel. When rung 8 changes a convention it changes it for
//! both, or the comparison it licenses is between two instruments rather than two channels.
//!
//! # Offsets are per FRAME before they are reduced
//!
//! A zone's begin offset is measured from **its own frame's** base — the earliest measured begin in
//! that frame — and only then folded across frames. Reducing the raw `begin_ticks` instead would
//! reduce the GPU clock itself, which drifts across a window and means nothing. The end offset is
//! formed per frame from that frame's two halves for the same reason `runner.rs` states at its own
//! fold: *"adding the two published MEDIANS afterwards is not a time any frame had"*.
//!
//! # Allocation
//!
//! The per-zone sample vectors are `Vec`s, allocated once per zone at first sight and grown to the
//! window's length. This is off-frame, bench-only host code that runs after a frame's results are
//! read — not the recorder and not a hot path — and the alternative, a fixed `[f64; WINDOW]` per
//! possible zone, is 128 zones × 121 frames of `f64` reserved to hold a handful of rows.

use boyko_rhi_vulkan::present::gpu_zone::{GpuLabel, PairResult};

use super::artifact::{LabelCensus, ZoneLabel, ZoneRow};

/// One zone's samples across the window.
struct ZoneAccum {
    /// The zone id, `family base + pass slot`.
    zone: u16,
    /// The WORST label this zone showed in any frame of the window.
    ///
    /// Worst rather than last: a window in which one frame tore is a window whose numbers are
    /// suspect, and a reader that saw only the final frame's label would be told otherwise.
    worst: ZoneLabel,
    /// Measured durations, ns.
    dur_ns: Vec<f64>,
    /// Measured begin offsets from each frame's own base, ns.
    begin_off_ns: Vec<f64>,
    /// Measured end offsets, formed per frame.
    end_off_ns: Vec<f64>,
}

/// How bad a label is. `Measured` is best; anything else means the numbers are not measurements.
fn severity(l: ZoneLabel) -> u8 {
    match l {
        ZoneLabel::Measured => 0,
        ZoneLabel::NotBracketed => 1,
        ZoneLabel::Lost => 2,
        ZoneLabel::Torn => 3,
    }
}

/// Maps the recorder's label onto the artifact's. Two enums because they belong to two crates and
/// one of them is the file format; the mapping is total and lives here alone.
fn label_of(l: GpuLabel) -> ZoneLabel {
    match l {
        GpuLabel::Measured => ZoneLabel::Measured,
        GpuLabel::NotBracketed => ZoneLabel::NotBracketed,
        GpuLabel::Lost => ZoneLabel::Lost,
        GpuLabel::Torn => ZoneLabel::Torn,
    }
}

/// Accumulates a window of retired frames and reduces it to the artifact's rows.
pub struct WindowReducer {
    zones: Vec<ZoneAccum>,
    census: LabelCensus,
    /// Nanoseconds per GPU tick (`VkPhysicalDeviceLimits::timestampPeriod`).
    period_ns: f64,
    /// Frames folded in, whatever they contained.
    frames: u32,
}

impl WindowReducer {
    /// A reducer for a device whose timestamps advance `period_ns` per tick.
    #[must_use]
    pub fn new(period_ns: f64) -> WindowReducer {
        WindowReducer { zones: Vec::new(), census: LabelCensus::default(), period_ns, frames: 0 }
    }

    /// Frames folded in so far.
    #[must_use]
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// Folds one retired frame's pairs.
    pub fn observe_frame(&mut self, pairs: &[PairResult]) {
        self.frames += 1;
        // THIS frame's base: the earliest measured begin in it. A frame with no measured pair has
        // no base and contributes no offsets — which is different from contributing an offset of
        // zero, and the difference is the whole reason the label travels beside the numbers.
        let base = pairs
            .iter()
            .filter(|p| matches!(p.label, GpuLabel::Measured))
            .map(|p| p.begin_ticks)
            .min();

        for p in pairs {
            let label = label_of(p.label);
            match label {
                ZoneLabel::Measured => self.census.measured += 1,
                ZoneLabel::NotBracketed => self.census.not_bracketed += 1,
                ZoneLabel::Lost => self.census.lost += 1,
                ZoneLabel::Torn => self.census.torn += 1,
            }

            let idx = match self.zones.iter().position(|z| z.zone == p.zone) {
                Some(i) => i,
                None => {
                    self.zones.push(ZoneAccum {
                        zone: p.zone,
                        worst: label,
                        dur_ns: Vec::new(),
                        begin_off_ns: Vec::new(),
                        end_off_ns: Vec::new(),
                    });
                    self.zones.len() - 1
                }
            };
            let acc = &mut self.zones[idx];
            if severity(label) > severity(acc.worst) {
                acc.worst = label;
            }
            let (Some(base), ZoneLabel::Measured) = (base, label) else { continue };
            // `begin_ticks` is masked to the device's valid bits and monotone within a frame, so
            // the subtraction cannot underflow for the pair that IS the base or any pair after it.
            let begin_off = (p.begin_ticks.saturating_sub(base) as f64) * self.period_ns;
            let dur = (p.dur_ticks as f64) * self.period_ns;
            acc.dur_ns.push(dur);
            acc.begin_off_ns.push(begin_off);
            acc.end_off_ns.push(begin_off + dur);
        }
    }

    /// Reduces the window.
    ///
    /// Rows come out sorted by zone id so the artifact is deterministic: two runs of the same
    /// configuration differ in their numbers, never in their row order, which is what lets a reader
    /// diff two files line for line.
    #[must_use]
    pub fn finish(mut self) -> (Vec<ZoneRow>, LabelCensus) {
        self.zones.sort_unstable_by_key(|z| z.zone);
        let rows = self
            .zones
            .iter()
            .map(|z| {
                let (median_ns, mean_ns, p95_ns) = stats_ns(&z.dur_ns);
                let (begin_off_ns, _, _) = stats_ns(&z.begin_off_ns);
                let (end_off_ns, _, _) = stats_ns(&z.end_off_ns);
                ZoneRow {
                    zone: z.zone,
                    label: z.worst,
                    // The count of MEASURED samples, not of frames: a row saying `n = 30` when
                    // three of the thirty were torn would be claiming thirty measurements.
                    n: z.dur_ns.len() as u32,
                    median_ns,
                    mean_ns,
                    p95_ns,
                    begin_off_ns,
                    end_off_ns,
                }
            })
            .collect();
        (rows, self.census)
    }
}

/// `(median, mean, p95)`, ns — `runner.rs`'s `vb_bench_stats_ns`, convention for convention.
///
/// Returns zeros on an empty slice rather than asserting, because a zone the recorder never
/// measured is a normal outcome here: its row carries its label, and the label is what says the
/// numbers are not measurements. The shipped helper asserts instead because its caller could never
/// reach it empty.
#[must_use]
pub fn stats_ns(samples: &[f64]) -> (f64, f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let mut sorted: Vec<f64> = samples.to_vec();
    // `f64` is only `PartialOrd`; these are GPU timestamp deltas scaled by a finite period, so
    // `partial_cmp` cannot return `None` — say so rather than `unwrap()`.
    sorted.sort_unstable_by(|a, b| {
        a.partial_cmp(b).expect("invariant: GPU timestamp deltas are finite, never NaN")
    });
    let n = sorted.len();
    let median = if n % 2 == 1 { sorted[n / 2] } else { 0.5 * (sorted[n / 2 - 1] + sorted[n / 2]) };
    let p95 = sorted[((n as f64 * 0.95) as usize).min(n - 1)];
    (median, mean, p95)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The statistics agree with the shipped helper's conventions, including the even-count median
    /// and the p95 index — the property that keeps the artifact comparable with the printed lines.
    #[test]
    fn the_statistics_match_the_shipped_conventions() {
        // Even count: the median is the average of the two central values, not either of them.
        let (median, mean, p95) = stats_ns(&[10.0, 20.0, 30.0, 40.0]);
        assert!((median - 25.0).abs() < f64::EPSILON, "even-count median must average the centre");
        assert!((mean - 25.0).abs() < f64::EPSILON);
        // (4 * 0.95) as usize == 3 -> the last element.
        assert!((p95 - 40.0).abs() < f64::EPSILON);

        // Odd count: the middle element itself.
        let (median, _, _) = stats_ns(&[10.0, 20.0, 30.0]);
        assert!((median - 20.0).abs() < f64::EPSILON);
    }

    /// An empty zone reduces to zeros rather than panicking, and its ROW still carries a label.
    #[test]
    fn a_zone_with_no_measured_sample_reduces_to_zeros() {
        assert_eq!(stats_ns(&[]), (0.0, 0.0, 0.0));
    }

    /// Offsets are relative to each frame's OWN base, so a drifting GPU clock does not leak in.
    #[test]
    fn offsets_are_taken_from_each_frames_own_base() {
        let mut r = WindowReducer::new(1.0);
        // Two frames, same shape, bases a million ticks apart.
        for base in [1_000u64, 1_000_000u64] {
            r.observe_frame(&[
                PairResult { zone: 16, label: GpuLabel::Measured, begin_ticks: base, dur_ticks: 100 },
                PairResult {
                    zone: 17,
                    label: GpuLabel::Measured,
                    begin_ticks: base + 500,
                    dur_ticks: 50,
                },
            ]);
        }
        let (rows, census) = r.finish();
        assert_eq!(rows.len(), 2);
        assert_eq!(census.measured, 4);
        assert_eq!(rows[0].zone, 16);
        assert!(rows[0].begin_off_ns.abs() < f64::EPSILON, "the base zone's offset must be 0");
        assert!(
            (rows[1].begin_off_ns - 500.0).abs() < f64::EPSILON,
            "the second zone's offset must be 500 in BOTH frames, so its median is 500 — a \
             reducer that folded raw begin_ticks would report roughly half a million here"
        );
        assert!((rows[1].end_off_ns - 550.0).abs() < f64::EPSILON);
    }

    /// The worst label in the window survives to the row.
    #[test]
    fn one_torn_frame_makes_the_whole_window_torn_for_that_zone() {
        let mut r = WindowReducer::new(1.0);
        r.observe_frame(&[PairResult {
            zone: 16,
            label: GpuLabel::Measured,
            begin_ticks: 10,
            dur_ticks: 100,
        }]);
        r.observe_frame(&[PairResult {
            zone: 16,
            label: GpuLabel::Torn,
            begin_ticks: 0,
            dur_ticks: 0,
        }]);
        let (rows, census) = r.finish();
        assert_eq!(rows[0].label, ZoneLabel::Torn, "a window with a torn frame is not `Measured`");
        assert_eq!(rows[0].n, 1, "n counts MEASURED samples, not frames");
        assert_eq!(census.measured, 1);
        assert_eq!(census.torn, 1);
    }

    /// Rows come out sorted by zone id, so two runs differ in numbers and never in row order.
    #[test]
    fn rows_are_ordered_by_zone_id() {
        let mut r = WindowReducer::new(1.0);
        r.observe_frame(&[
            PairResult { zone: 32, label: GpuLabel::Measured, begin_ticks: 0, dur_ticks: 1 },
            PairResult { zone: 16, label: GpuLabel::Measured, begin_ticks: 0, dur_ticks: 1 },
        ]);
        let (rows, _) = r.finish();
        assert!(rows[0].zone < rows[1].zone, "rows must be sorted, they came out {rows:?}");
    }
}
