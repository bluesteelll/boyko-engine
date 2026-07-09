//! [`Time`] — the virtual frame clock resource (Phase 20 plan D2/D4/D11).
//!
//! Written exactly once per frame by the frame driver
//! ([`App::update_with_delta`] → [`Time::advance_with`]); read by systems via
//! `Res<Time>`. The virtual delta is the raw delta clamped to
//! [`max_delta`](Time::max_delta) (the single death-spiral guard, plan D4),
//! then scaled by [`relative_speed`](Time::relative_speed); a paused clock
//! yields a ZERO delta. The real (unclamped, unscaled) delta and its sum are
//! carried alongside — no separate `Time<Real>` resource (plan D2).
//!
//! [`App::update_with_delta`]: crate::ecs::core::app::app::App::update_with_delta

use std::sync::OnceLock;
use std::time::Duration;

use crate::ecs::core::resources::register_new;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::identifiers::primitives::ResourceId;

/// Default inflow clamp for the virtual delta (plan D4): a hitch longer than
/// this is truncated, bounding the fixed loop's worst-case substep count
/// (exactly 16 at the 64 Hz default timestep).
const DEFAULT_MAX_DELTA: Duration = Duration::from_millis(250);

/// The virtual frame clock — pausable, scalable, hitch-clamped (plan D2).
///
/// Frame (Main-schedule) systems read this via `Res<Time>`; fixed-schedule
/// systems read [`FixedTime`](super::FixedTime) instead. The driver advances
/// it once per frame with [`advance_with`](Time::advance_with).
///
/// # Determinism
///
/// On the default path (`relative_speed == 1.0`) the virtual delta is pure
/// integer-nanosecond arithmetic — bit-identical to the clamped raw delta
/// (plan ★m5). A non-1.0 speed routes through `Duration::mul_f64`, which is
/// IEEE-deterministic across platforms but not bit-identical to the unscaled
/// delta.
pub struct Time {
    /// Virtual delta this frame: `clamp(raw, max_delta) * relative_speed`;
    /// ZERO while paused.
    delta: Duration,
    /// Cached `f32` seconds of `delta` — the per-system read.
    delta_secs: f32,
    /// Sum of virtual deltas.
    elapsed: Duration,
    /// Raw delta this frame — unclamped, unscaled, ignores pause.
    real_delta: Duration,
    /// Wall-clock sum of raw deltas.
    real_elapsed: Duration,
    /// Inflow clamp (plan D4). Setter-validated `> 0`.
    max_delta: Duration,
    /// Virtual speed multiplier. Setter-validated finite and `>= 0`
    /// (`0.0` is a legal pause-alias).
    relative_speed: f32,
    /// Paused ⇒ the virtual delta is ZERO (real fields still advance).
    paused: bool,
}

impl Time {
    /// Virtual delta of the current frame (clamped, scaled; ZERO while
    /// paused).
    #[inline]
    pub fn delta(&self) -> Duration {
        self.delta
    }

    /// Virtual delta of the current frame as `f32` seconds (cached).
    #[inline]
    pub fn delta_secs(&self) -> f32 {
        self.delta_secs
    }

    /// Sum of all virtual deltas since construction.
    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Raw delta of the current frame — unclamped, unscaled, pause-blind.
    #[inline]
    pub fn real_delta(&self) -> Duration {
        self.real_delta
    }

    /// Wall-clock sum of raw deltas since construction.
    #[inline]
    pub fn real_elapsed(&self) -> Duration {
        self.real_elapsed
    }

    /// Pauses the virtual clock: subsequent frames see a ZERO virtual delta
    /// (and therefore zero fixed substeps) until [`unpause`](Time::unpause).
    /// Real time keeps advancing.
    #[inline]
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Resumes the virtual clock after a [`pause`](Time::pause).
    ///
    /// Note: pause does NOT accumulate a backlog — paused frames contribute
    /// ZERO virtual time, so no catch-up burst occurs on unpause.
    #[inline]
    pub fn unpause(&mut self) {
        self.paused = false;
    }

    /// `true` while the virtual clock is paused.
    #[inline]
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Current virtual speed multiplier (default `1.0`).
    #[inline]
    pub fn relative_speed(&self) -> f32 {
        self.relative_speed
    }

    /// Sets the virtual speed multiplier.
    ///
    /// `0.0` is legal (an alias for pause); `1.0` restores the pure
    /// integer-nanosecond default path (plan ★m5). Raising the speed raises
    /// the worst-case fixed-substep count proportionally (plan D4 trade-off).
    /// A huge finite speed (e.g. `f32::MAX`) passes this validation but can
    /// overflow the `Duration::mul_f64` inside
    /// [`advance_with`](Time::advance_with) and panic THERE on the next frame
    /// (Bevy-identical exposure).
    ///
    /// # Panics
    ///
    /// Panics if `s` is not finite or is negative.
    #[inline]
    pub fn set_relative_speed(&mut self, s: f32) {
        if !(s.is_finite() && s >= 0.0) {
            invalid_relative_speed_panic(s);
        }
        self.relative_speed = s;
    }

    /// Current inflow clamp (default 250 ms).
    #[inline]
    pub fn max_delta(&self) -> Duration {
        self.max_delta
    }

    /// Sets the inflow clamp applied to the raw delta BEFORE scaling
    /// (plan D4). Raising it raises the worst-case fixed-substep count
    /// proportionally.
    ///
    /// # Panics
    ///
    /// Panics if `d` is zero.
    #[inline]
    pub fn set_max_delta(&mut self, d: Duration) {
        if d.is_zero() {
            zero_max_delta_panic();
        }
        self.max_delta = d;
    }

    /// Advances the clock by one frame's raw delta (plan D11 / ★m5).
    ///
    /// Real fields take `raw` verbatim; the virtual delta is
    /// `min(raw, max_delta)` scaled by `relative_speed` (bypassing the float
    /// multiply entirely when the speed is exactly `1.0` — the default path
    /// stays pure integer-nanosecond), or ZERO while paused.
    ///
    /// # Contract
    ///
    /// This is the frame driver's entry (`App::update_with_delta` step ①, or
    /// a pool-less runner's own frame function). Call it exactly once per
    /// frame, OUTSIDE any system body — a system wanting to mutate the clock
    /// uses `ResMut<Time>` setters (`pause` / `set_relative_speed` / …), never
    /// a re-advance. Each `advance_with` must be paired with exactly one
    /// [`fixed_advance`](super::fixed_advance) call when a fixed loop exists
    /// (plan ★m6).
    pub fn advance_with(&mut self, raw: Duration) {
        // Phase 20 Q6 (the EventWriter precedent, inverted): catch
        // `ResMut<Time>`-driven re-advance from inside a scheduled system
        // body in debug builds.
        debug_assert!(
            !boyko_threadpool::is_in_system_run(),
            "Time::advance_with called inside a scheduled system body — the frame \
             driver advances the clock exactly once per frame; mutate the clock \
             from a system via ResMut<Time> setters instead",
        );
        debug_assert!(
            !self.max_delta.is_zero(),
            "invariant: max_delta > 0 (setter-validated)"
        );
        debug_assert!(
            self.relative_speed.is_finite() && self.relative_speed >= 0.0,
            "invariant: relative_speed is finite and non-negative (setter-validated)"
        );

        self.real_delta = raw;
        self.real_elapsed += raw;

        let clamped = raw.min(self.max_delta);
        let delta = if self.paused {
            Duration::ZERO
        } else if self.relative_speed == 1.0 {
            // ★m5: the default path is pure integer-ns — no float conversion.
            clamped
        } else {
            clamped.mul_f64(f64::from(self.relative_speed))
        };

        self.delta = delta;
        self.delta_secs = delta.as_secs_f32();
        self.elapsed += delta;
    }
}

impl Default for Time {
    /// Zeroed clock: `max_delta` 250 ms, `relative_speed` 1.0, unpaused.
    fn default() -> Self {
        Self {
            delta: Duration::ZERO,
            delta_secs: 0.0,
            elapsed: Duration::ZERO,
            real_delta: Duration::ZERO,
            real_elapsed: Duration::ZERO,
            max_delta: DEFAULT_MAX_DELTA,
            relative_speed: 1.0,
            paused: false,
        }
    }
}

// Hand-implemented rather than `#[derive(Resource)]`: `boyko-macros` is a
// dev-dependency of `boyko-ecs`, so its derives are unavailable in normal
// builds. Mirrors EXACTLY what the derive expands to (same as `AppExit`).
impl Resource for Time {
    #[inline]
    fn resource_id() -> ResourceId {
        static ID: OnceLock<ResourceId> = OnceLock::new();
        *ID.get_or_init(|| ResourceId(register_new::<Self>()))
    }
}

/// Cold panic for an invalid `relative_speed`, kept out of the setter body.
#[cold]
#[inline(never)]
fn invalid_relative_speed_panic(s: f32) -> ! {
    panic!("Time::set_relative_speed requires a finite, non-negative speed (got {s})");
}

/// Cold panic for a zero `max_delta`, kept out of the setter body.
#[cold]
#[inline(never)]
fn zero_max_delta_panic() -> ! {
    panic!("Time::set_max_delta requires a non-zero duration (a zero clamp would freeze virtual time)");
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── T1 — `Time::advance_with` table ──────────────────────────────────────

    /// Zero / first-frame delta: everything stays ZERO except nothing.
    #[test]
    fn advance_with_zero_raw_is_all_zero() {
        let mut t = Time::default();
        t.advance_with(Duration::ZERO);
        assert_eq!(t.delta(), Duration::ZERO);
        assert_eq!(t.delta_secs(), 0.0);
        assert_eq!(t.elapsed(), Duration::ZERO);
        assert_eq!(t.real_delta(), Duration::ZERO);
        assert_eq!(t.real_elapsed(), Duration::ZERO);
    }

    /// Default-path exactness (★m5): at speed 1.0 the virtual delta is
    /// bit-identical to the raw delta — including an odd nanosecond count
    /// no float round-trip could preserve in general.
    #[test]
    fn advance_with_default_path_is_integer_exact() {
        let mut t = Time::default();
        let raw = Duration::from_nanos(16_666_667);
        t.advance_with(raw);
        assert_eq!(t.delta(), raw);
        assert_eq!(t.elapsed(), raw);
    }

    /// Clamp edge: raw above `max_delta` is truncated to exactly `max_delta`;
    /// raw at exactly `max_delta` passes through unclamped.
    #[test]
    fn advance_with_clamps_to_max_delta() {
        let mut t = Time::default();
        t.advance_with(Duration::from_millis(400));
        assert_eq!(t.delta(), Duration::from_millis(250), "above the clamp: truncated");
        assert_eq!(t.real_delta(), Duration::from_millis(400), "real delta is unclamped");

        let mut t = Time::default();
        t.advance_with(Duration::from_millis(250));
        assert_eq!(t.delta(), Duration::from_millis(250), "at the clamp: passes through");
    }

    /// Scale: a non-1.0 speed multiplies the clamped delta.
    #[test]
    fn advance_with_scales_by_relative_speed() {
        let mut t = Time::default();
        t.set_relative_speed(2.0);
        t.advance_with(Duration::from_millis(100));
        assert_eq!(t.delta(), Duration::from_millis(200));
        assert_eq!(t.real_delta(), Duration::from_millis(100), "real delta is unscaled");

        // Scale applies AFTER the clamp (plan D4: clamp the raw inflow once).
        let mut t = Time::default();
        t.set_relative_speed(2.0);
        t.advance_with(Duration::from_millis(400));
        assert_eq!(t.delta(), Duration::from_millis(500), "clamp(400) = 250, then x2");
    }

    /// Speed 0.0 is the legal pause-alias: virtual delta ZERO.
    #[test]
    fn advance_with_speed_zero_is_pause_alias() {
        let mut t = Time::default();
        t.set_relative_speed(0.0);
        t.advance_with(Duration::from_millis(100));
        assert_eq!(t.delta(), Duration::ZERO);
        assert_eq!(t.real_delta(), Duration::from_millis(100));
    }

    /// Pause: virtual delta ZERO; real fields keep advancing.
    #[test]
    fn advance_with_paused_zeroes_virtual_delta() {
        let mut t = Time::default();
        t.pause();
        assert!(t.is_paused());
        t.advance_with(Duration::from_millis(100));
        assert_eq!(t.delta(), Duration::ZERO);
        assert_eq!(t.delta_secs(), 0.0);
        assert_eq!(t.elapsed(), Duration::ZERO);
        assert_eq!(t.real_delta(), Duration::from_millis(100));
        assert_eq!(t.real_elapsed(), Duration::from_millis(100));
    }

    /// Pause + scale: pause wins over any speed.
    #[test]
    fn advance_with_pause_wins_over_scale() {
        let mut t = Time::default();
        t.set_relative_speed(3.0);
        t.pause();
        t.advance_with(Duration::from_millis(100));
        assert_eq!(t.delta(), Duration::ZERO);
    }

    /// Real-vs-virtual divergence across a clamped + paused sequence.
    #[test]
    fn real_and_virtual_elapsed_diverge() {
        let mut t = Time::default();
        t.advance_with(Duration::from_millis(400)); // virtual 250, real 400
        t.pause();
        t.advance_with(Duration::from_millis(100)); // virtual 0, real 100
        t.unpause();
        t.advance_with(Duration::from_millis(10)); // virtual 10, real 10
        assert_eq!(t.elapsed(), Duration::from_millis(260));
        assert_eq!(t.real_elapsed(), Duration::from_millis(510));
    }

    /// Unpause resumes normal accumulation (no backlog burst).
    #[test]
    fn unpause_resumes_without_backlog() {
        let mut t = Time::default();
        t.pause();
        t.advance_with(Duration::from_millis(100));
        t.unpause();
        t.advance_with(Duration::from_millis(16));
        assert_eq!(t.elapsed(), Duration::from_millis(16));
    }

    // ── T1 — setter panics ────────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "finite, non-negative speed")]
    fn set_relative_speed_negative_panics() {
        Time::default().set_relative_speed(-1.0);
    }

    #[test]
    #[should_panic(expected = "finite, non-negative speed")]
    fn set_relative_speed_nan_panics() {
        Time::default().set_relative_speed(f32::NAN);
    }

    #[test]
    #[should_panic(expected = "finite, non-negative speed")]
    fn set_relative_speed_infinite_panics() {
        Time::default().set_relative_speed(f32::INFINITY);
    }

    #[test]
    #[should_panic(expected = "non-zero duration")]
    fn set_max_delta_zero_panics() {
        Time::default().set_max_delta(Duration::ZERO);
    }

    /// Defaults pin the plan's binding values.
    #[test]
    fn default_values_match_plan() {
        let t = Time::default();
        assert_eq!(t.max_delta(), Duration::from_millis(250));
        assert_eq!(t.relative_speed(), 1.0);
        assert!(!t.is_paused());
    }
}
