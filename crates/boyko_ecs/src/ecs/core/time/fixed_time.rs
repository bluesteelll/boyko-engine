//! [`FixedTime`] — the fixed-timestep clock + accumulator resource
//! (Phase 20 plan D2/D3/D4/D9).
//!
//! Fixed-schedule systems read this via `Res<FixedTime>`; its
//! [`delta`](FixedTime::delta) IS the timestep. The
//! [`overstep`](FixedTime::overstep) field is THE accumulator driven by
//! [`fixed_advance`](super::fixed_advance), and
//! [`overstep_fraction`](FixedTime::overstep_fraction) is THE interpolation
//! alpha read by Main-schedule render/interpolation systems after the fixed
//! loop has settled (plan D9).

use std::sync::OnceLock;
use std::time::Duration;

use crate::ecs::core::resources::register_new;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::identifiers::primitives::ResourceId;

/// Default fixed timestep: exactly 64 Hz = 15 625 000 ns (plan D4 — lossless
/// in f32/f64 and free of refresh-rate beat patterns).
pub(crate) const DEFAULT_FIXED_TIMESTEP: Duration = Duration::from_nanos(15_625_000);

/// The fixed-timestep clock: timestep + overstep accumulator + alpha
/// (plan D2).
///
/// Driven exclusively by [`fixed_advance`](super::fixed_advance) (the
/// `pub(crate)` [`accumulate`](FixedTime::accumulate) /
/// [`expend`](FixedTime::expend) pair); systems read it, and may reconfigure
/// the timestep via `ResMut<FixedTime>` ([`set_timestep`](FixedTime::set_timestep)
/// takes effect on the NEXT fixed loop — the running loop keeps its
/// entry-time snapshot, plan ★M3).
///
/// # Invariant
///
/// After each fixed loop, `overstep < timestep` (the loop expends until the
/// accumulator is below one step). `elapsed` is the exact sum of expended
/// timesteps — the determinism witness (plan D3).
pub struct FixedTime {
    /// The fixed timestep. Setter-validated `> 0`.
    timestep: Duration,
    /// Cached `f32` seconds of `timestep` — the per-system
    /// [`delta_secs`](FixedTime::delta_secs) read.
    timestep_secs: f32,
    /// THE accumulator (plan D3). Invariant: `< timestep` after each fixed
    /// loop. Never discarded by the engine;
    /// [`discard_overstep`](FixedTime::discard_overstep) is the explicit
    /// escape hatch (plan D4).
    overstep: Duration,
    /// Sum of expended timesteps (the determinism witness).
    elapsed: Duration,
    /// Substep count of the most recent fixed loop, written by
    /// [`fixed_advance`](super::fixed_advance) after the loop. Apps without a
    /// fixed schedule read a permanent `0` (plan Q5).
    steps_this_frame: u32,
}

impl FixedTime {
    /// Creates a fixed clock with the given timestep.
    ///
    /// # Panics
    ///
    /// Panics if `timestep` is zero.
    pub fn new(timestep: Duration) -> Self {
        if timestep.is_zero() {
            zero_timestep_panic();
        }
        Self {
            timestep,
            timestep_secs: timestep.as_secs_f32(),
            overstep: Duration::ZERO,
            elapsed: Duration::ZERO,
            steps_this_frame: 0,
        }
    }

    /// Creates a fixed clock running at `hz` steps per second.
    ///
    /// `from_hz(64.0)` yields exactly 15 625 000 ns (the engine default).
    ///
    /// # Panics
    ///
    /// Panics if `hz` is not finite or not strictly positive, or so large
    /// (above ~1e9 Hz) that the timestep rounds below `Duration`'s 1 ns
    /// resolution.
    pub fn from_hz(hz: f64) -> Self {
        Self::new(timestep_from_hz(hz))
    }

    /// The fixed timestep.
    #[inline]
    pub fn timestep(&self) -> Duration {
        self.timestep
    }

    /// Sets the fixed timestep. Takes effect on the NEXT fixed loop: a
    /// running [`fixed_advance`](super::fixed_advance) keeps the snapshot it
    /// took at loop entry (plan ★M3), so a mid-loop change cannot void the
    /// substep bound.
    ///
    /// # Panics
    ///
    /// Panics if `d` is zero.
    #[inline]
    pub fn set_timestep(&mut self, d: Duration) {
        if d.is_zero() {
            zero_timestep_panic();
        }
        self.timestep = d;
        self.timestep_secs = d.as_secs_f32();
    }

    /// The per-substep delta — identical to [`timestep`](FixedTime::timestep)
    /// (the fixed-schedule mirror of `Time::delta`).
    #[inline]
    pub fn delta(&self) -> Duration {
        self.timestep
    }

    /// The per-substep delta as `f32` seconds (cached).
    #[inline]
    pub fn delta_secs(&self) -> f32 {
        self.timestep_secs
    }

    /// The unexpended accumulator remainder. `< timestep` after each fixed
    /// loop.
    #[inline]
    pub fn overstep(&self) -> Duration {
        self.overstep
    }

    /// THE interpolation alpha: `overstep / timestep`, in `[0, 1)` (plan D9).
    ///
    /// Read from Main-schedule systems AFTER the fixed loop (the frame-driver
    /// order guarantees the loop has settled before Main runs). A mid-catch-up
    /// read from a FIXED-schedule system can still observe `overstep >=
    /// timestep` (the loop has not finished expending yet); the value then
    /// saturates at the upper edge of the half-open range instead of
    /// exceeding it.
    #[inline]
    pub fn overstep_fraction(&self) -> f32 {
        let f = (self.overstep.as_secs_f64() / self.timestep.as_secs_f64()) as f32;
        // `overstep < timestep` — the POST-LOOP invariant — makes the f64
        // ratio strictly < 1.0, but f64 → f32 rounding at the upper edge can
        // land exactly on 1.0 (e.g. timestep 1 s, overstep 1 s − 1 ns), and a
        // mid-catch-up read from a fixed-schedule system can see the ratio at
        // or above 1.0 outright. Both cases pin to the documented half-open
        // range.
        if f >= 1.0 { 1.0_f32.next_down() } else { f }
    }

    /// Discards the accumulated overstep — the explicit escape hatch for
    /// teleports / long-pause resumes (plan D4). The engine itself never
    /// drops accumulated time.
    #[inline]
    pub fn discard_overstep(&mut self) {
        self.overstep = Duration::ZERO;
    }

    /// Exact sum of expended timesteps (the determinism witness).
    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Substep count of the most recent fixed loop. A permanent `0` when no
    /// fixed schedule exists (the loop never runs — plan Q5).
    #[inline]
    pub fn steps_this_frame(&self) -> u32 {
        self.steps_this_frame
    }

    /// Adds one frame's virtual delta to the accumulator. Driven only by
    /// [`fixed_advance`](super::fixed_advance).
    #[inline]
    pub(crate) fn accumulate(&mut self, delta: Duration) {
        self.overstep += delta;
    }

    /// Attempts to expend one timestep from the accumulator: on success
    /// subtracts `ts` from `overstep`, adds it to `elapsed`, and returns
    /// `true`; returns `false` once `overstep < ts`.
    ///
    /// ★M3 (binding): `ts` is the CALLER's loop-entry snapshot of the
    /// timestep, NOT `self.timestep` — so a `ResMut<FixedTime>` system
    /// calling [`set_timestep`](FixedTime::set_timestep) mid-loop cannot
    /// change the amount the running loop expends per step (which would void
    /// the substep bound and the next-frame-effect promise).
    #[inline]
    pub(crate) fn expend(&mut self, ts: Duration) -> bool {
        debug_assert!(!ts.is_zero(), "invariant: the timestep snapshot is > 0");
        if self.overstep >= ts {
            self.overstep -= ts;
            self.elapsed += ts;
            true
        } else {
            false
        }
    }

    /// Records the substep count of the loop that just finished. Driven only
    /// by [`fixed_advance`](super::fixed_advance), after the loop.
    #[inline]
    pub(crate) fn set_steps_this_frame(&mut self, steps: u32) {
        self.steps_this_frame = steps;
    }
}

impl Default for FixedTime {
    /// 64 Hz — exactly 15 625 000 ns per step.
    fn default() -> Self {
        Self::new(DEFAULT_FIXED_TIMESTEP)
    }
}

// Hand-implemented rather than `#[derive(Resource)]`: `boyko-macros` is a
// dev-dependency of `boyko-ecs`, so its derives are unavailable in normal
// builds. Mirrors EXACTLY what the derive expands to (same as `AppExit`).
impl Resource for FixedTime {
    #[inline]
    fn resource_id() -> ResourceId {
        static ID: OnceLock<ResourceId> = OnceLock::new();
        *ID.get_or_init(|| ResourceId(register_new::<Self>()))
    }
}

/// Converts a frequency to its exact integer-nanosecond timestep. Shared by
/// [`FixedTime::from_hz`] and `App::set_fixed_hz`.
///
/// # Panics
///
/// Panics if `hz` is not finite or not strictly positive, or so large (above
/// ~1e9 Hz) that the timestep rounds below `Duration`'s 1 ns resolution.
pub(crate) fn timestep_from_hz(hz: f64) -> Duration {
    if !(hz.is_finite() && hz > 0.0) {
        invalid_hz_panic(hz);
    }
    let ts = Duration::from_secs_f64(hz.recip());
    // ~1e9 Hz is the resolution ceiling: above it `recip()` rounds below 1 ns
    // and the timestep collapses to zero — fail in the frequency domain here
    // rather than with the downstream (confusing) zero-timestep message.
    if ts.is_zero() {
        excessive_hz_panic(hz);
    }
    ts
}

/// Cold panic for a zero timestep, kept out of the constructor/setter bodies.
#[cold]
#[inline(never)]
fn zero_timestep_panic() -> ! {
    panic!("FixedTime requires a non-zero timestep (a zero step would make the fixed loop unbounded)");
}

/// Cold panic for an invalid frequency, kept out of `from_hz`.
#[cold]
#[inline(never)]
fn invalid_hz_panic(hz: f64) -> ! {
    panic!("FixedTime::from_hz requires a finite, strictly positive frequency (got {hz})");
}

/// Cold panic for a frequency whose timestep rounds below `Duration`'s 1 ns
/// resolution, kept out of `timestep_from_hz`.
#[cold]
#[inline(never)]
fn excessive_hz_panic(hz: f64) -> ! {
    panic!(
        "FixedTime::from_hz: {hz} Hz rounds the timestep below Duration's 1 ns resolution \
         (the maximum representable frequency is ~1e9 Hz)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── T2 — `FixedTime` table ────────────────────────────────────────────────

    /// `from_hz(64.0)` is EXACTLY 15 625 000 ns (binding, plan D4), and the
    /// default matches it.
    #[test]
    fn from_hz_64_is_exact_default() {
        assert_eq!(FixedTime::from_hz(64.0).timestep(), Duration::from_nanos(15_625_000));
        assert_eq!(FixedTime::default().timestep(), Duration::from_nanos(15_625_000));
        assert_eq!(FixedTime::default().delta_secs(), 0.015625);
    }

    /// `delta()` mirrors the timestep; the accumulator starts empty.
    #[test]
    fn fresh_clock_state() {
        let ft = FixedTime::default();
        assert_eq!(ft.delta(), ft.timestep());
        assert_eq!(ft.overstep(), Duration::ZERO);
        assert_eq!(ft.elapsed(), Duration::ZERO);
        assert_eq!(ft.steps_this_frame(), 0);
        assert_eq!(ft.overstep_fraction(), 0.0);
    }

    /// Expend sequence: 40 ms at 64 Hz expends twice, then refuses; the
    /// remainder and the elapsed sum are integer-exact.
    #[test]
    fn expend_sequence_is_exact() {
        let mut ft = FixedTime::default();
        let ts = ft.timestep();
        ft.accumulate(Duration::from_millis(40));
        assert!(ft.expend(ts));
        assert!(ft.expend(ts));
        assert!(!ft.expend(ts), "40 ms holds exactly two 15.625 ms steps");
        assert_eq!(ft.overstep(), Duration::from_micros(8_750));
        assert_eq!(ft.elapsed(), Duration::from_micros(31_250));
    }

    /// Expend at an exact boundary leaves a ZERO remainder.
    #[test]
    fn expend_exact_boundary() {
        let mut ft = FixedTime::default();
        let ts = ft.timestep();
        ft.accumulate(ts);
        assert!(ft.expend(ts));
        assert!(!ft.expend(ts));
        assert_eq!(ft.overstep(), Duration::ZERO);
    }

    /// `overstep_fraction` stays in `[0, 1)` across the accumulator range,
    /// including the 1-ns-below-timestep upper edge.
    #[test]
    fn overstep_fraction_half_open_range() {
        let mut ft = FixedTime::default();
        ft.accumulate(Duration::from_micros(8_750));
        let f = ft.overstep_fraction();
        assert!((f - 0.56).abs() < 1e-6, "8.75 / 15.625 = 0.56 (got {f})");

        // Upper edge: a 1 s timestep with overstep = 1 s − 1 ns rounds to 1.0
        // in f32 without the pin; the documented range is half-open.
        let mut ft = FixedTime::new(Duration::from_secs(1));
        ft.accumulate(Duration::from_secs(1) - Duration::from_nanos(1));
        let f = ft.overstep_fraction();
        assert!((0.0..1.0).contains(&f), "alpha must stay in [0, 1) (got {f})");
    }

    /// `discard_overstep` empties the accumulator and nothing else.
    #[test]
    fn discard_overstep_clears_accumulator() {
        let mut ft = FixedTime::default();
        let ts = ft.timestep();
        ft.accumulate(Duration::from_millis(40));
        assert!(ft.expend(ts));
        ft.discard_overstep();
        assert_eq!(ft.overstep(), Duration::ZERO);
        assert_eq!(ft.elapsed(), ts, "elapsed keeps the already-expended step");
    }

    /// ★M3 unit form: `expend(ts)` uses the caller's snapshot, not the
    /// (possibly re-set) live timestep.
    #[test]
    fn expend_uses_caller_snapshot_not_live_timestep() {
        let mut ft = FixedTime::default();
        let snapshot = ft.timestep();
        ft.accumulate(Duration::from_millis(20));
        ft.set_timestep(Duration::from_millis(1)); // live timestep shrinks…
        assert!(ft.expend(snapshot));
        assert!(!ft.expend(snapshot), "…but the snapshot still governs the expend");
        assert_eq!(ft.elapsed(), snapshot);
        assert_eq!(ft.timestep(), Duration::from_millis(1), "the new timestep is staged for the next loop");
    }

    /// `set_timestep` refreshes the cached seconds.
    #[test]
    fn set_timestep_updates_cached_secs() {
        let mut ft = FixedTime::default();
        ft.set_timestep(Duration::from_millis(10));
        assert_eq!(ft.delta_secs(), 0.01);
        assert_eq!(ft.delta(), Duration::from_millis(10));
    }

    // ── T2 — constructor / setter panics ─────────────────────────────────────

    #[test]
    #[should_panic(expected = "non-zero timestep")]
    fn new_zero_timestep_panics() {
        let _ = FixedTime::new(Duration::ZERO);
    }

    #[test]
    #[should_panic(expected = "non-zero timestep")]
    fn set_timestep_zero_panics() {
        FixedTime::default().set_timestep(Duration::ZERO);
    }

    #[test]
    #[should_panic(expected = "finite, strictly positive frequency")]
    fn from_hz_zero_panics() {
        let _ = FixedTime::from_hz(0.0);
    }

    #[test]
    #[should_panic(expected = "finite, strictly positive frequency")]
    fn from_hz_nan_panics() {
        let _ = FixedTime::from_hz(f64::NAN);
    }

    /// A frequency above the ~1e9 Hz resolution ceiling fails in the
    /// frequency domain, not with the downstream zero-timestep message.
    #[test]
    #[should_panic(expected = "below Duration's 1 ns resolution")]
    fn from_hz_huge_panics_in_frequency_domain() {
        let _ = FixedTime::from_hz(1e10);
    }
}
