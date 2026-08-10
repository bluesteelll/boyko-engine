//! Profiling rung 9 — the rejection sampler that puts the CPU and GPU axes on one line.
//!
//! # What was actually missing, measured before this rung was written
//!
//! `docs/diagnostics/profiling/02-GPU.md`'s D14 specifies a v1 artifact field
//! `cpu_gpu_offset = UNCORRELATED`, and `00-GOAL-TARGETS.md`'s "Where did the frame go?" row
//! repeats it. **It was never written.** A repo-wide grep for `cpu_gpu_offset` before this rung
//! returned four documentation lines and one module-doc mention in
//! [`crate::profiling::artifact`] — and no field, no key, no writer. So tier 1's *declaration* was
//! specified and shipped as prose only: a reader of the artifact could not tell that the GPU
//! numbers were on an unrelated axis, because nothing said so.
//!
//! This rung ships both halves. The field exists now in every artifact, and it says either why
//! there is no correlation or what the correlation is.
//!
//! # Why the offset cannot be inferred from what the engine already measures
//!
//! Every GPU duration in this tree comes from a query pool: the GPU wrote a counter value when it
//! reached a pipeline stage. Two such values subtract cleanly *because they are on one axis*.
//! Relating either of them to a CPU instant needs a third thing — a reading of the device counter
//! taken at a moment the CPU can also name — and nothing in a command buffer can provide it: by
//! the time the host can read a query result, the moment it describes is long past, separated by
//! submission, scheduling and fence latency that is itself unmeasured.
//!
//! `VK_EXT_calibrated_timestamps` provides exactly the missing thing, and nothing more. Khronos
//! states the problem it solves in the extension's own motivation: core timestamps *"cannot be
//! compared even across separate submits within the same run of an application, as power
//! management events can reset the timer."*
//!
//! # The sampler, and why one sample is not a calibration
//!
//! [`crate::profiling::correlate`] brackets each device-clock read between two CPU clock reads.
//! The device value lies somewhere inside that bracket; the bracket's WIDTH is therefore the
//! uncertainty in placing it on the CPU axis, and it is a number this code measures rather than
//! assumes.
//!
//! A single bracket is worthless because it cannot distinguish a fast sample from a preempted
//! one — a scheduler slice landing between the two CPU reads widens the bracket by a millisecond
//! and leaves no other trace. D14 tier 2's protocol is therefore a **rejection sampler**: take
//! [`CORRELATE_PROBES`] probes, find the narrowest bracket, and discard every probe wider than
//! `min × 3/2`. What survives is the set that was not interrupted.
//!
//! Two decisions inside that protocol are this module's, not the corpus's, and both are stated
//! because either could reasonably have gone the other way:
//!
//! * **The offset is the MEDIAN of the accepted probes, not the minimum-bracket probe's.** Keeping
//!   only the narrowest uses one sample out of thirty-two and is moved by whatever noise that one
//!   sample carried. The median uses every probe that survived the rejection and is not moved by
//!   one that squeaked under the threshold.
//! * **The published bound is the WORST accepted bracket, not the median's own.** A bound
//!   describing only the sample that happened to land in the middle would understate the spread
//!   the median was computed from.
//!
//! # What this module does NOT claim
//!
//! * **It is not a frequency calibration.** Two free-running counters drift, and one offset
//!   describes one instant. That is why [`Correlated::drift_ns`] exists: the sampler runs a second
//!   time at the end of the window and publishes how far the two axes moved apart over
//!   [`Correlated::span_ns`], instead of asserting that they did not.
//! * **It does not survive a clock epoch break.** A suspend invalidates the CPU tick axis
//!   entirely, and an offset measured on either side of one is meaningless. The sampler reads
//!   `clock_epoch()` before and after and refuses ([`Uncorrelated::EpochBreak`]) rather than
//!   averaging across it.
//! * **It says nothing about a queue's timestamp validity.** `timestampValidBits` masking, and the
//!   wrap that implies, are the seam's contract (`boyko_rhi::DeviceClockSample::device_ticks`) and
//!   are unchanged here.

use boyko_rhi::api::RhiApi;
use boyko_rhi::device::RhiDevice;
use boyko_rhi::DeviceClockSample;

use boyko_diag::clock;

/// D14 tier 2's probe count: *"32 probes at arm"*.
///
/// Thirty-two brackets cost microseconds in total — each is two `rdtsc` reads around one driver
/// call that issues no GPU work — so the count is set by what makes the rejection meaningful
/// rather than by what it costs. Below about a dozen a single unlucky slice can be the minimum.
pub const CORRELATE_PROBES: usize = 32;

/// The acceptance threshold's numerator: a probe is kept when
/// `bracket <= min_bracket * ACCEPT_NUM / ACCEPT_DEN`.
///
/// D14 spells it `min_deviation × 3/2`. Expressed as an integer ratio rather than `1.5` so the
/// comparison is exact — a float threshold would make acceptance depend on rounding at the
/// boundary, and the boundary is where a marginal probe sits by definition.
pub const ACCEPT_NUM: u64 = 3;
/// The acceptance threshold's denominator. See [`ACCEPT_NUM`].
pub const ACCEPT_DEN: u64 = 2;

/// Why a run produced no offset.
///
/// Each variant is REACHABLE and each is separately produced by a test in this module. A refusal
/// nothing can trigger is a refusal that says nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Uncorrelated {
    /// The device cannot sample its own clock from the host — `VK_EXT_calibrated_timestamps` is
    /// absent, or it is present without `VK_TIME_DOMAIN_DEVICE_EXT`.
    ///
    /// **This is D14 tier 1**, and it is the ordinary case on a device that simply does not have
    /// the extension. Not an error.
    Unsupported,
    /// Every probe failed at the seam, or every bracket was degenerate (the CPU counter did not
    /// advance across the call).
    NoProbeSurvived,
    /// `clock_epoch()` moved between the first and last probe: the CPU tick axis was reset
    /// underneath the run.
    EpochBreak,
    /// `ticks_per_ns` is not a usable scale (non-finite or non-positive), so CPU ticks cannot be
    /// turned into nanoseconds at all.
    CpuUnscaled,
    /// `timestampPeriod` is not a usable scale, so device ticks cannot be turned into
    /// nanoseconds. `DeviceCaps::timestamps_usable` gates this upstream; the check is repeated
    /// here because this function is also called with values a test chose.
    DeviceUnscaled,
}

impl Uncorrelated {
    /// The wire word, rendered inside `UNCORRELATED(...)`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "UNSUPPORTED",
            Self::NoProbeSurvived => "NO_PROBE_SURVIVED",
            Self::EpochBreak => "EPOCH_BREAK",
            Self::CpuUnscaled => "CPU_UNSCALED",
            Self::DeviceUnscaled => "DEVICE_UNSCALED",
        }
    }

    /// Parses [`Self::as_str`] back. `None` for anything else — an unknown reason is not silently
    /// mapped onto a known one.
    #[must_use]
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "UNSUPPORTED" => Some(Self::Unsupported),
            "NO_PROBE_SURVIVED" => Some(Self::NoProbeSurvived),
            "EPOCH_BREAK" => Some(Self::EpochBreak),
            "CPU_UNSCALED" => Some(Self::CpuUnscaled),
            "DEVICE_UNSCALED" => Some(Self::DeviceUnscaled),
            _ => None,
        }
    }
}

/// A measured relation between the CPU tick axis and the device tick axis.
///
/// # The direction of the offset, said once and plainly
///
/// `cpu_ns = device_ns + offset_ns`.
///
/// So a GPU zone whose begin stamp is `d` device ticks sits at CPU nanosecond
/// `d * timestamp_period + offset_ns`. The offset is large and its sign is arbitrary — both axes
/// count from unrelated origins — which is precisely why it is a measured number and not one
/// anybody could sanity-check by eye.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Correlated {
    /// `cpu_ns - device_ns` at the accepted probes' median, nanoseconds.
    pub offset_ns: i64,
    /// The WORST accepted bracket, nanoseconds — how far apart the two clock reads that straddle
    /// the device read could have been.
    pub bracket_ns: u64,
    /// The worst `maxDeviation` the driver reported across the accepted probes, nanoseconds.
    ///
    /// Carried separately from [`Self::bracket_ns`] rather than folded into one figure, because a
    /// combined bound hides which term produced it. The campaign has already paid for that once,
    /// in the contrast band's terms.
    pub driver_ns: u64,
    /// How many probes survived the rejection.
    pub accepted: u32,
    /// How many were discarded — a wide bracket, a backwards bracket, or a seam error.
    pub rejected: u32,
    /// `clock_epoch()` during the run. A consumer comparing two correlations must refuse a pair
    /// whose epochs differ.
    pub epoch: u32,
    /// How much the offset moved between the arm-time correlation and a later one, nanoseconds;
    /// `0` when only one correlation was taken.
    ///
    /// This is the term that stops a single offset from being an unstated assumption about
    /// frequency stability. Signed: the two counters can drift either way.
    pub drift_ns: i64,
    /// The wall time [`Self::drift_ns`] accumulated over, nanoseconds; `0` when there is no drift
    /// measurement. Without it the drift is a number with no rate.
    pub span_ns: u64,
}

impl Correlated {
    /// D14's `max_deviation_ns`: the bound **at the instant the offset was sampled**.
    ///
    /// A method, not a field. The two terms it maxes are both stored, and storing their maximum
    /// as well would be a third value obliged to agree with two others — this tree has already
    /// measured what that costs. One derivation, one truth.
    ///
    /// ⚠️ **This is not the error at an arbitrary point in the window.** See
    /// [`Self::deviation_at_ns`], and the measured numbers in its doc, before using this one to
    /// place a zone that was recorded seconds after the sample.
    #[must_use]
    pub const fn max_deviation_ns(&self) -> u64 {
        if self.bracket_ns > self.driver_ns {
            self.bracket_ns
        } else {
            self.driver_ns
        }
    }

    /// The bound `elapsed_ns` after the correlation was taken — the sampling bound plus the share
    /// of the measured drift accumulated by then.
    ///
    /// # The factor of twenty-seven thousand
    ///
    /// MEASURED, first real 240-frame window on this box (`schema_version = 8`, VB×Mesh, 2026-08-10):
    ///
    /// ```text
    /// cpu_gpu_bracket_ns = 11          <- the sampling bound
    /// cpu_gpu_drift_ns   = 299_776     <- over
    /// cpu_gpu_span_ns    = 1_731_151_495   (= 173 ppm)
    /// ```
    ///
    /// The two axes moved **300 microseconds** apart over 1.73 seconds while the sampling bound was
    /// **11 nanoseconds**. A consumer that placed a zone from the end of that window using
    /// [`Self::max_deviation_ns`] alone would have published a bound four orders of magnitude too
    /// tight — and 300 µs is ~2 % of a 60 Hz frame, so it is not a rounding matter either.
    ///
    /// That is why the drift is measured rather than assumed negligible, and why this
    /// interpolation lives here instead of in each caller: there is one place to get it wrong.
    ///
    /// Linear in `elapsed_ns`, which is a MODEL and not a measurement — two counters can drift
    /// non-uniformly. With one drift observation it is the only defensible interpolation, and it
    /// degrades to [`Self::max_deviation_ns`] when the drift was never measured
    /// ([`Self::span_ns`] `== 0`).
    #[must_use]
    pub fn deviation_at_ns(&self, elapsed_ns: u64) -> u64 {
        let base = self.max_deviation_ns();
        if self.span_ns == 0 {
            return base;
        }
        let rate = self.drift_ns.unsigned_abs() as f64 / self.span_ns as f64;
        base.saturating_add((rate * elapsed_ns as f64).ceil() as u64)
    }
}

/// The artifact's `cpu_gpu_offset`: a measured relation, or the reason there is none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Correlation {
    /// The axes are related, with a measured bound.
    Correlated(Correlated),
    /// They are not, and this is why.
    Uncorrelated(Uncorrelated),
}

impl Correlation {
    /// The `cpu_gpu_offset` value as it appears in the artifact.
    ///
    /// One key holds both cases, and the case is legible from the value itself: a refusal renders
    /// as `UNCORRELATED(<REASON>)` — keeping D14's own literal greppable — and a measurement
    /// renders as a decimal integer. Two keys, one for the status and one for the number, would be
    /// two values obliged to agree about which of them is meaningful.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Correlated(c) => c.offset_ns.to_string(),
            Self::Uncorrelated(u) => format!("UNCORRELATED({})", u.as_str()),
        }
    }

    /// The refusal reason, if this is one. `None` when correlated.
    #[must_use]
    pub const fn refusal(&self) -> Option<Uncorrelated> {
        match self {
            Self::Correlated(_) => None,
            Self::Uncorrelated(u) => Some(*u),
        }
    }
}

/// Turns a batch of probes into a [`Correlation`]. **Pure**: no clock, no device, no I/O.
///
/// Everything about this rung that can be wrong without a GPU is wrong in here, which is why it is
/// separated from the collection loop above it. `failures` is how many probes the seam refused —
/// they cannot appear in `probes`, but they must appear in the rejected count or the artifact
/// would report thirty-two accepted out of thirty-two attempted while a third of them errored.
///
/// `epoch_before` / `epoch_after` bracket the whole collection loop.
#[must_use]
pub fn resolve(
    probes: &[DeviceClockSample],
    failures: u32,
    ticks_per_ns: f64,
    period_ns: f64,
    epoch_before: u32,
    epoch_after: u32,
) -> Correlation {
    if epoch_before != epoch_after {
        return Correlation::Uncorrelated(Uncorrelated::EpochBreak);
    }
    if !ticks_per_ns.is_finite() || ticks_per_ns <= 0.0 {
        return Correlation::Uncorrelated(Uncorrelated::CpuUnscaled);
    }
    if !period_ns.is_finite() || period_ns <= 0.0 {
        return Correlation::Uncorrelated(Uncorrelated::DeviceUnscaled);
    }

    // Pass 1 — bracket widths, in CPU TICKS. The narrowest is the threshold's basis, so it is
    // found before anything is converted: comparing in ticks keeps the ordering exact, and a
    // conversion applied to every probe before the minimum is known would round the basis itself.
    let mut widths = [0u64; CORRELATE_PROBES];
    let mut n = 0usize;
    let mut rejected = failures;
    for p in probes.iter().take(CORRELATE_PROBES) {
        // A bracket whose end does not follow its start is not a narrow bracket, it is a broken
        // one: the counter was read out of order (a migration between cores on a non-invariant
        // TSC, or a test that constructed nonsense). `checked_sub` makes that a rejection rather
        // than a `u64` wrap to something astronomically wide — which would also be rejected, but
        // only by accident, and would poison the minimum if it wrapped to something small.
        match p.cpu_ticks_after.checked_sub(p.cpu_ticks_before) {
            Some(w) => {
                widths[n] = w;
                n += 1;
            }
            None => rejected += 1,
        }
    }
    if n == 0 {
        return Correlation::Uncorrelated(Uncorrelated::NoProbeSurvived);
    }

    let min_width = widths[..n].iter().copied().min().unwrap_or(0);
    // Integer, and saturating. `min_width * 3` cannot realistically overflow a `u64` bracket, but
    // the saturation costs one instruction on a path taken thirty-two times per process and
    // removes the need to reason about it at all.
    let threshold = min_width.saturating_mul(ACCEPT_NUM) / ACCEPT_DEN;

    // Pass 2 — accept, and convert only what was accepted.
    let mut offsets = [0i64; CORRELATE_PROBES];
    let mut accepted = 0usize;
    let mut bracket_ns: u64 = 0;
    let mut driver_ns: u64 = 0;
    for (p, w) in probes.iter().take(CORRELATE_PROBES).zip(widths[..n].iter()) {
        if *w > threshold {
            rejected += 1;
            continue;
        }
        // The device value lies inside the bracket; the midpoint is the estimator, and it is
        // chosen HERE rather than at the seam so that the seam keeps reporting what it observed.
        let cpu_mid_ticks = p.cpu_ticks_before + w / 2;
        // `ticks / ticks_per_ns`, matching `boyko_diag::clock`'s own scale direction (its round-
        // trip test reconstructs ns as `(c1 - c0) as f64 / ticks_per_ns()`).
        //
        // Precision: an absolute tick count on a machine up for weeks is ~1e16, where an `f64`
        // step is 2 ticks — under a nanosecond at any plausible clock rate, and three orders below
        // the brackets this sampler accepts. Stated rather than assumed, because the alternative
        // (differencing against an arbitrary base first) would buy nothing measurable.
        let cpu_ns = cpu_mid_ticks as f64 / ticks_per_ns;
        let device_ns = p.device_ticks as f64 * period_ns;
        offsets[accepted] = (cpu_ns - device_ns).round() as i64;
        accepted += 1;

        // A BOUND is rounded up. Rounding a bound down publishes a tighter claim than was
        // measured, which is the one direction that is never harmless.
        let w_ns = (*w as f64 / ticks_per_ns).ceil() as u64;
        bracket_ns = bracket_ns.max(w_ns);
        driver_ns = driver_ns.max(p.driver_max_deviation_ns);
    }
    // Unreachable by construction — the minimum always satisfies `min <= min * 3 / 2` — but the
    // arithmetic that makes it unreachable is integer division, and an empty accepted set would
    // otherwise index nothing and publish an offset of zero.
    if accepted == 0 {
        return Correlation::Uncorrelated(Uncorrelated::NoProbeSurvived);
    }

    let acc = &mut offsets[..accepted];
    acc.sort_unstable();
    // The lower median for an even count, not the mean of the two middles. Both are defensible;
    // the lower median is an OBSERVED probe's offset, and the mean of two is a value no probe
    // reported — the same reason the campaign's floor uses a lattice node rather than an average.
    let offset_ns = acc[(accepted - 1) / 2];

    Correlation::Correlated(Correlated {
        offset_ns,
        bracket_ns,
        driver_ns,
        accepted: accepted as u32,
        rejected,
        epoch: epoch_after,
        // Filled by `with_drift` when a second correlation is taken; a single run states zero for
        // both, which `span_ns == 0` marks as "not measured" rather than "no drift".
        drift_ns: 0,
        span_ns: 0,
    })
}

/// Runs the full protocol against a live device: [`CORRELATE_PROBES`] probes, then [`resolve`].
///
/// Calls `boyko_diag::clock::calibrate()` first. It is idempotent and CAS-guarded, and calling it
/// is strictly better than detecting an uncalibrated clock afterwards: `ticks_per_ns` on an
/// uncalibrated clock returns `1.0` — a value that is also the correct scale on the non-x86-64
/// backend — so "uncalibrated" and "one tick is one nanosecond" are indistinguishable at the read.
/// Ensuring beats detecting.
#[must_use]
pub fn correlate<A: RhiApi, D: RhiDevice<A>>(device: &D, period_ns: f64) -> Correlation {
    if !device.calibrated_timestamps_supported() {
        return Correlation::Uncorrelated(Uncorrelated::Unsupported);
    }
    clock::calibrate();

    let epoch_before = clock::clock_epoch();
    // A fixed array, not a `Vec`: thirty-two 32-byte samples is one kilobyte of stack, and this
    // runs on the boot path of a profiler whose whole point is not to perturb what it measures.
    let mut probes = [DeviceClockSample {
        cpu_ticks_before: 0,
        cpu_ticks_after: 0,
        device_ticks: 0,
        driver_max_deviation_ns: 0,
    }; CORRELATE_PROBES];
    let mut n = 0usize;
    let mut failures = 0u32;
    for _ in 0..CORRELATE_PROBES {
        match device.sample_device_clock() {
            Ok(s) => {
                probes[n] = s;
                n += 1;
            }
            // Counted, not aborted on. A driver that refuses one sample and serves the next is
            // still usable, and a run that stopped at the first refusal would report a smaller
            // `rejected` than it actually suffered.
            Err(_) => failures += 1,
        }
    }
    let epoch_after = clock::clock_epoch();

    resolve(
        &probes[..n],
        failures,
        clock::ticks_per_ns(),
        period_ns,
        epoch_before,
        epoch_after,
    )
}

/// Folds a later correlation into an earlier one as a measured DRIFT.
///
/// D14 says *"recalibration each fold"*. This artifact has exactly one window and therefore
/// exactly one fold, so recalibrating per fold and correlating at arm are the same act — which
/// would leave the offset's validity across a window of hundreds of frames unstated. Taking a
/// second correlation at the end and publishing the difference states it: a reader can bound the
/// error anywhere in the window instead of trusting that two free-running counters held step.
///
/// Returns `early` unchanged when either side is a refusal, or when the two ran in different clock
/// epochs — a drift across an epoch break is not a drift, it is two unrelated numbers subtracted.
#[must_use]
pub fn with_drift(early: Correlation, late: Correlation, span_ns: u64) -> Correlation {
    let (Correlation::Correlated(a), Correlation::Correlated(b)) = (early, late) else {
        return early;
    };
    if a.epoch != b.epoch {
        return early;
    }
    Correlation::Correlated(Correlated {
        drift_ns: b.offset_ns.saturating_sub(a.offset_ns),
        span_ns,
        ..a
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a probe whose device read sits at `device_ticks` and whose bracket is `[b, b + w)`.
    fn probe(b: u64, w: u64, device_ticks: u64, driver: u64) -> DeviceClockSample {
        DeviceClockSample {
            cpu_ticks_before: b,
            cpu_ticks_after: b + w,
            device_ticks,
            driver_max_deviation_ns: driver,
        }
    }

    /// A clean batch resolves, and the offset has the documented direction.
    ///
    /// Constructed so the answer is known: `ticks_per_ns = 1.0` and `period_ns = 1.0` make ticks
    /// and nanoseconds interchangeable on both axes, so an offset of exactly `1_000` is what
    /// `cpu_ns = device_ns + offset_ns` demands. A `resolve` that subtracted the other way would
    /// report `-1_000` and fail here — which is the whole reason this test fixes the sign rather
    /// than asserting a magnitude.
    #[test]
    fn a_clean_batch_resolves_and_cpu_equals_device_plus_offset() {
        let probes: Vec<_> = (0..8)
            .map(|i| probe(11_000 + i * 100, 10, 10_000 + i * 100, 7))
            .collect();
        let r = resolve(&probes, 0, 1.0, 1.0, 3, 3);
        let Correlation::Correlated(c) = r else {
            panic!("a batch of eight identical-width brackets must resolve, got {r:?}");
        };
        // Each probe's midpoint is `11_000 + 100i + 5`; its device read is `10_000 + 100i`.
        assert_eq!(c.offset_ns, 1_005, "cpu_ns must equal device_ns + offset_ns");
        assert_eq!(c.accepted, 8);
        assert_eq!(c.rejected, 0);
        assert_eq!(c.epoch, 3);
        assert_eq!(c.bracket_ns, 10, "the bound is the WORST accepted bracket");
        assert_eq!(c.driver_ns, 7);
        assert_eq!(c.max_deviation_ns(), 10, "the max of the two terms");
    }

    /// **The rejection is the rung.** A preempted probe is discarded, and its offset does not move
    /// the answer.
    ///
    /// The RED this pins: delete the `if *w > threshold` arm — i.e. accept every probe — and the
    /// wide probe's offset (deliberately a thousand times off) joins the median's input set. This
    /// test asserts both halves, the count and the value, because a sampler that counted the
    /// rejection while still folding the sample in would pass a count-only assertion.
    #[test]
    fn a_preempted_probe_is_rejected_and_does_not_move_the_offset() {
        let mut probes: Vec<_> = (0..7)
            .map(|i| probe(11_000 + i * 100, 10, 10_000 + i * 100, 0))
            .collect();
        // One probe interrupted for 100_000 ticks, with a device read that would drag the median
        // far off if it were folded in.
        probes.push(probe(11_700, 100_000, 1, 0));
        let r = resolve(&probes, 0, 1.0, 1.0, 0, 0);
        let Correlation::Correlated(c) = r else {
            panic!("seven clean probes must still resolve, got {r:?}");
        };
        assert_eq!(c.accepted, 7, "the wide bracket must not be accepted");
        assert_eq!(c.rejected, 1);
        assert_eq!(
            c.offset_ns, 1_005,
            "a rejected probe must not contribute to the median"
        );
        assert_eq!(
            c.bracket_ns, 10,
            "the published bound must describe the ACCEPTED set only"
        );
    }

    /// The threshold is exactly `min * 3 / 2`, and it is inclusive.
    ///
    /// Both sides of the boundary are asserted in one test on purpose: a threshold off by one in
    /// either direction changes exactly one of these two numbers, and a test that pinned only the
    /// accepted side would stay green if the comparison became `>=`.
    #[test]
    fn the_acceptance_boundary_is_inclusive_at_min_times_three_halves() {
        // min = 10 ⇒ threshold = 15.
        let probes = [
            probe(1_000, 10, 100, 0),
            probe(2_000, 15, 100, 0), // exactly at the threshold — kept
            probe(3_000, 16, 100, 0), // one tick past it — dropped
        ];
        let r = resolve(&probes, 0, 1.0, 1.0, 0, 0);
        let Correlation::Correlated(c) = r else {
            panic!("expected a correlation, got {r:?}");
        };
        assert_eq!(c.accepted, 2, "the probe AT the threshold must be kept");
        assert_eq!(c.rejected, 1, "the probe one tick past it must not be");
    }

    /// Seam failures reach the rejected count.
    ///
    /// They cannot appear in `probes` — there is nothing to put there — so an implementation that
    /// simply ignored the parameter would report a perfect run. The RED: drop `failures` from the
    /// initial value of `rejected`.
    #[test]
    fn probes_the_seam_refused_are_counted_as_rejected() {
        let probes = [probe(1_000, 10, 100, 0), probe(2_000, 10, 100, 0)];
        let r = resolve(&probes, 5, 1.0, 1.0, 0, 0);
        let Correlation::Correlated(c) = r else {
            panic!("expected a correlation, got {r:?}");
        };
        assert_eq!(c.accepted, 2);
        assert_eq!(c.rejected, 5, "a refused sample is a rejected probe");
    }

    /// An epoch break refuses outright rather than averaging across it.
    #[test]
    fn an_epoch_break_refuses() {
        let probes = [probe(1_000, 10, 100, 0)];
        assert_eq!(
            resolve(&probes, 0, 1.0, 1.0, 4, 5).refusal(),
            Some(Uncorrelated::EpochBreak)
        );
    }

    /// Each remaining refusal is reachable. A variant nothing can produce is a variant that never
    /// tells a reader anything.
    #[test]
    fn every_refusal_is_producible() {
        let ok = [probe(1_000, 10, 100, 0)];
        assert_eq!(
            resolve(&ok, 0, f64::NAN, 1.0, 0, 0).refusal(),
            Some(Uncorrelated::CpuUnscaled)
        );
        assert_eq!(
            resolve(&ok, 0, 1.0, 0.0, 0, 0).refusal(),
            Some(Uncorrelated::DeviceUnscaled)
        );
        assert_eq!(
            resolve(&[], 32, 1.0, 1.0, 0, 0).refusal(),
            Some(Uncorrelated::NoProbeSurvived),
            "thirty-two seam failures and nothing collected"
        );
        // A backwards bracket is not a narrow one.
        let backwards = [DeviceClockSample {
            cpu_ticks_before: 2_000,
            cpu_ticks_after: 1_000,
            device_ticks: 100,
            driver_max_deviation_ns: 0,
        }];
        assert_eq!(
            resolve(&backwards, 0, 1.0, 1.0, 0, 0).refusal(),
            Some(Uncorrelated::NoProbeSurvived)
        );
    }

    /// The bound is rounded UP, never down.
    ///
    /// At `ticks_per_ns = 3.0` a 10-tick bracket is 3.33 ns; a bound that truncated would publish
    /// 3 and claim a tightness it did not measure.
    #[test]
    fn the_published_bracket_is_rounded_up() {
        let probes = [probe(1_000, 10, 100, 0)];
        let Correlation::Correlated(c) = resolve(&probes, 0, 3.0, 1.0, 0, 0) else {
            panic!("expected a correlation");
        };
        assert_eq!(c.bracket_ns, 4, "10 ticks / 3.0 = 3.33 ns, rounded up");
    }

    /// The wire form round-trips both cases, and the refusal keeps D14's own literal greppable.
    #[test]
    fn the_wire_form_carries_the_case_in_the_value() {
        let refused = Correlation::Uncorrelated(Uncorrelated::Unsupported);
        assert_eq!(refused.render(), "UNCORRELATED(UNSUPPORTED)");
        for u in [
            Uncorrelated::Unsupported,
            Uncorrelated::NoProbeSurvived,
            Uncorrelated::EpochBreak,
            Uncorrelated::CpuUnscaled,
            Uncorrelated::DeviceUnscaled,
        ] {
            assert_eq!(
                Uncorrelated::from_wire(u.as_str()),
                Some(u),
                "every reason must survive its own spelling"
            );
        }
        assert_eq!(
            Uncorrelated::from_wire("UNCORRELATED"),
            None,
            "an unknown word must not be mapped onto a known reason"
        );
        let c = Correlation::Correlated(Correlated {
            offset_ns: -42,
            bracket_ns: 1,
            driver_ns: 2,
            accepted: 3,
            rejected: 4,
            epoch: 5,
            drift_ns: 0,
            span_ns: 0,
        });
        assert_eq!(c.render(), "-42");
        assert_eq!(c.refusal(), None);
    }

    /// The bound GROWS with elapsed time, and it degrades to the sampling bound when the drift was
    /// never measured.
    ///
    /// The numbers are the ones the first real window produced (11 ns over a 173 ppm drift), so
    /// the assertion below is the factor this rung exists to stop a consumer from missing: at the
    /// window's end the honest bound is ~300 µs, not 11 ns. The RED: return `max_deviation_ns()`
    /// unconditionally.
    #[test]
    fn the_bound_grows_with_the_measured_drift() {
        let measured = Correlated {
            offset_ns: 0,
            bracket_ns: 11,
            driver_ns: 1,
            accepted: 15,
            rejected: 17,
            epoch: 0,
            drift_ns: 299_776,
            span_ns: 1_731_151_495,
        };
        assert_eq!(measured.max_deviation_ns(), 11, "the bound AT the sample");
        assert_eq!(
            measured.deviation_at_ns(0),
            11,
            "at the sampling instant the drift has contributed nothing"
        );
        let at_end = measured.deviation_at_ns(measured.span_ns);
        assert!(
            (299_780..=299_800).contains(&at_end),
            "a full span of 173 ppm drift must reach ~300 us, got {at_end} ns -- a consumer using \
             the 11 ns bound there would be wrong by four orders of magnitude"
        );
        // A negative drift is still a widening bound: the axes moved apart either way.
        let backwards = Correlated { drift_ns: -299_776, ..measured };
        assert_eq!(backwards.deviation_at_ns(measured.span_ns), at_end);
        // And with no drift measurement the bound is the sampling bound, unchanged.
        let unmeasured = Correlated { drift_ns: 0, span_ns: 0, ..measured };
        assert_eq!(unmeasured.deviation_at_ns(10_000_000_000), 11);
    }

    /// Drift is folded in only when both sides measured something in the same epoch.
    #[test]
    fn drift_needs_two_correlations_from_one_epoch() {
        let base = Correlated {
            offset_ns: 1_000,
            bracket_ns: 10,
            driver_ns: 0,
            accepted: 32,
            rejected: 0,
            epoch: 7,
            drift_ns: 0,
            span_ns: 0,
        };
        let late = Correlated { offset_ns: 1_250, ..base };
        let Correlation::Correlated(f) = with_drift(
            Correlation::Correlated(base),
            Correlation::Correlated(late),
            2_000_000_000,
        ) else {
            panic!("two correlations from one epoch must fold");
        };
        assert_eq!(f.drift_ns, 250);
        assert_eq!(f.span_ns, 2_000_000_000);
        assert_eq!(f.offset_ns, 1_000, "the EARLY offset is the published one");

        // A refusal on either side leaves the early value untouched, drift unclaimed.
        let refused = Correlation::Uncorrelated(Uncorrelated::Unsupported);
        let Correlation::Correlated(f) =
            with_drift(Correlation::Correlated(base), refused, 1_000)
        else {
            panic!("a refused second correlation must not destroy the first");
        };
        assert_eq!((f.drift_ns, f.span_ns), (0, 0));

        // And so does an epoch break between them.
        let other_epoch = Correlated { epoch: 8, offset_ns: 9_999, ..base };
        let Correlation::Correlated(f) = with_drift(
            Correlation::Correlated(base),
            Correlation::Correlated(other_epoch),
            1_000,
        ) else {
            panic!("expected the early correlation back");
        };
        assert_eq!(
            (f.drift_ns, f.span_ns),
            (0, 0),
            "a difference across an epoch break is not a drift"
        );
    }
}
