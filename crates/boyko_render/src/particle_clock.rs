//! Particles P0 (D6 / plan §The clock) — the subsystem's OWN fixed-rate clock.
//!
//! [`ParticleClock`] is a `#[derive(Resource)]` singleton owning `timestep`, `accumulator`,
//! `steps` and `dropped_steps`. It is advanced once per RENDERED frame from
//! [`Time::delta_secs`](boyko_ecs::ecs::core::time::Time::delta_secs) — which the engine has
//! already clamped to `Time::max_delta`, scaled by `relative_speed`, and zeroed while paused — so
//! pausing the game pauses particles and slow-motion slows them, both free.
//!
//! # Why the subsystem owns a clock instead of using `FixedTime` (D6/M4)
//!
//! `App`'s `fixed_builder` is created LAZILY on the first `*_in(CoreSchedule::Fixed, …)` call, and
//! `event_policy_cfg: None` auto-resolves at `finish` to "`WaitForFixed` **iff a Fixed schedule was
//! configured**". A rendering plugin that registered anything on `Fixed` would therefore flip
//! **every event type in the process** from `EveryFrame` to `WaitForFixed` — at 200 fps against a
//! 64 Hz step, two frames in three — silently changing input, UI and collision event delivery.
//! Patching the policy back is worse than the disease: it would then override the resolution a
//! user's OWN later Fixed schedule should have produced, and it would make plugin order
//! load-bearing. Owning the clock removes the coupling entirely (plan invariant 3, *subsystem
//! containment*; the gate is `tests/particle_containment.rs`).
//!
//! Everything the fixed rate was ever needed FOR is preserved: a constant `dt`, so `damping =
//! exp2(-drag · timestep)` and the rotation multiplier `(cos ω·timestep, sin ω·timestep)` stay
//! host-precomputable per effect (which is what deletes `exp2` and all trig from the GPU), and a
//! bounded tunneling distance for rung P1's SDF collision.
//!
//! # ONE number, clamped ONCE, on the host (M3)
//!
//! [`steps`](ParticleClock::steps) is used by `particle_tick_emitters` to advance every emitter
//! (`dt = steps · timestep`) **and** is pushed verbatim to the `particle_sim` shader as its loop
//! bound. The shader's own `min(pc.steps, PARTICLE_SUBSTEP_CEILING)` survives solely as the hang
//! guard against a corrupt push constant (`robustBufferAccess` is OFF — an unbounded device loop is
//! a GPU-hang class); it can never bind, because the host already clamped. Rev 2 had the host
//! advancing UNCLAMPED while the shader clamped — the same "a value with two homes" defect the
//! plan's reversal ledger names as the shape the defects keep arriving in.
//!
//! # The excess is DROPPED, not carried
//!
//! Carrying `raw_steps - steps` would build a backlog that never drains under sustained
//! slow-motion, converting a transient into a permanent lag. Dropping is exactly what
//! `Time::max_delta` already does for the frame delta ("the single death-spiral guard"); this is
//! the same policy at the subsystem's own rate, and it is COUNTED in
//! [`dropped_steps`](ParticleClock::dropped_steps).
//!
//! # Trade-off, stated (M6)
//!
//! Above the step rate most frames step ZERO times — at 200 fps with a 64 Hz timestep, two in
//! three — so particles visibly move at 64 Hz against a 200 Hz camera. **P0 accepts fixed-rate
//! stepping without render-time interpolation.** Rung P2b adds it, riding
//! [`overstep_fraction`](ParticleClock::overstep_fraction).

use boyko_macros::Resource;

use crate::particle::PARTICLE_SUBSTEP_CEILING;

/// The default particle step rate, matching the engine's own fixed default (64 Hz) so a project
/// that never touches the knob steps particles and gameplay physics at the same cadence.
///
/// `1/64` is exactly representable in binary floating point, so the derived timestep is exact and
/// `n · timestep` accumulates no rate drift.
pub const PARTICLE_DEFAULT_HZ: f32 = 64.0;

/// The subsystem's own fixed-rate clock (D6) — a `World`-singleton Resource.
///
/// Does **not** read `FixedTime` and does **not** cause a `CoreSchedule::Fixed` schedule to exist;
/// see the module doc for why that distinction is load-bearing for every OTHER subsystem in the
/// process.
///
/// Field order is hot-first: `timestep` / `accumulator` / `steps` are read or written on every
/// frame's advance, `dropped_steps` is a diagnostic only a hitch touches. 16 B, one quarter of a
/// cache line.
#[derive(Resource, Clone, Copy, Debug)]
pub struct ParticleClock {
    /// Seconds per particle substep — the constant `dt` every host-precomputed effect parameter is
    /// baked against.
    timestep: f32,
    /// Unconsumed virtual time, always in `[0, timestep)` after an advance.
    accumulator: f32,
    /// Substeps to run THIS frame — the one number `particle_tick_emitters` and the sim's push
    /// constant share (M3).
    steps: u32,
    /// Monotonic count of substeps DROPPED by the [`PARTICLE_SUBSTEP_CEILING`] clamp — the
    /// diagnostic that makes the time-dropping policy auditable instead of silent.
    dropped_steps: u32,
}

impl ParticleClock {
    /// Builds a clock stepping at `hz` substeps per second.
    ///
    /// # Panics (debug)
    ///
    /// `debug_assert!`s `hz.is_finite() && hz > 0.0`. A non-positive or non-finite rate would make
    /// `timestep` non-finite and every subsequent `accumulator / timestep` meaningless; in release
    /// the value falls back to [`PARTICLE_DEFAULT_HZ`] rather than poisoning the clock, because a
    /// mis-set knob must not be able to hang the device loop.
    #[inline]
    pub fn from_hz(hz: f32) -> Self {
        debug_assert!(
            hz.is_finite() && hz > 0.0,
            "invariant: the particle step rate must be finite and positive (got {hz})"
        );
        let hz = if hz.is_finite() && hz > 0.0 { hz } else { PARTICLE_DEFAULT_HZ };
        Self { timestep: 1.0 / hz, accumulator: 0.0, steps: 0, dropped_steps: 0 }
    }

    /// Seconds per substep — the constant `dt` the host bakes `damping` and the rotation
    /// multiplier against (D6).
    #[inline]
    pub const fn timestep(&self) -> f32 {
        self.timestep
    }

    /// Substeps to run THIS frame, already clamped to [`PARTICLE_SUBSTEP_CEILING`] (M3).
    ///
    /// THE one number: `particle_tick_emitters` advances every emitter by `steps · timestep` and
    /// the sim receives this exact value as its loop bound. Zero on a frame above the step rate
    /// (the common case at high refresh — M6): particles hold position, and the sim still rebuilds
    /// the alive list and the render records.
    #[inline]
    pub const fn steps(&self) -> u32 {
        self.steps
    }

    /// Monotonic count of substeps dropped by the ceiling clamp since boot.
    ///
    /// Non-zero only on a frame whose virtual delta exceeded `PARTICLE_SUBSTEP_CEILING · timestep`
    /// — reachable in practice only through `Time::relative_speed`, which is public and validated
    /// `finite && >= 0` (speed 8.0 on the stock 250 ms `max_delta` asks for 128 substeps at 64 Hz).
    /// On such a frame particles age at `ceiling / raw` of wall-clock; the alternative, an
    /// unbounded device loop, is a GPU-hang class.
    #[inline]
    pub const fn dropped_steps(&self) -> u32 {
        self.dropped_steps
    }

    /// The unconsumed fraction of a substep, in `[0, 1)` — rung P2b's render-time interpolation
    /// factor (M6), the subsystem's own equivalent of `FixedTime::overstep_fraction()`.
    ///
    /// Unused at P0: the VS reads the sim's positions directly, so a frame that steps zero times
    /// draws the previous substep's state verbatim.
    #[inline]
    pub fn overstep_fraction(&self) -> f32 {
        self.accumulator / self.timestep
    }

    /// Advances the clock by one rendered frame's virtual delta and computes this frame's
    /// [`steps`](Self::steps) — EXACTLY the plan's normative block:
    ///
    /// ```text
    /// accumulator  += delta_secs
    /// raw_steps     = floor(accumulator / timestep)
    /// steps         = min(raw_steps, PARTICLE_SUBSTEP_CEILING)   // THE clamp, once, here
    /// dropped_steps += raw_steps - steps
    /// accumulator  -= raw_steps * timestep                       // DROP the excess
    /// ```
    ///
    /// `delta_secs` is `Time::delta_secs()` — already `min(raw, max_delta)`-clamped, speed-scaled
    /// and zero while paused, so this fn inherits the engine's inflow guard for free and needs no
    /// second one.
    ///
    /// Note the drain subtracts `raw_steps`, not `steps`: that is what makes the ceiling a
    /// TIME-DROPPING clamp rather than a backlog (see the module doc).
    ///
    /// # Robustness
    ///
    /// A non-finite `delta_secs` (only reachable by writing `Time` behind its own validation)
    /// trips a `debug_assert!` and, in release, drains the accumulator to zero and steps zero
    /// times rather than propagating a NaN into the push constant that bounds a device loop.
    pub fn advance(&mut self, delta_secs: f32) {
        debug_assert!(
            delta_secs.is_finite() && delta_secs >= 0.0,
            "invariant: Time::delta_secs() is finite and non-negative (got {delta_secs})"
        );

        self.accumulator += delta_secs;

        // `f32 as u32` saturates at u32::MAX and maps NaN to 0 (Rust's defined float→int cast), so
        // a pathological accumulator cannot wrap the step count into a small number.
        let raw_steps = (self.accumulator / self.timestep).floor() as u32;
        let steps = if raw_steps > PARTICLE_SUBSTEP_CEILING { PARTICLE_SUBSTEP_CEILING } else { raw_steps };
        self.dropped_steps = self.dropped_steps.saturating_add(raw_steps - steps);

        // The remainder float rounding leaves behind. Keeping it ONLY when it lands strictly
        // inside `[0, timestep)` is what makes `overstep_fraction()`'s documented range true by
        // construction; anything outside is dropped to zero, which is the same policy the ceiling
        // clamp applies and is bounded by one substep.
        let remainder = self.accumulator - raw_steps as f32 * self.timestep;
        self.accumulator =
            if remainder > 0.0 && remainder < self.timestep { remainder } else { 0.0 };

        self.steps = steps;

        debug_assert!(
            self.steps <= PARTICLE_SUBSTEP_CEILING,
            "invariant: pc.steps <= PARTICLE_SUBSTEP_CEILING before the push (M3)"
        );
        debug_assert!(
            self.accumulator >= 0.0 && self.accumulator < self.timestep,
            "invariant: the accumulator stays in [0, timestep) so overstep_fraction() is in [0, 1)"
        );
    }
}

impl Default for ParticleClock {
    /// [`PARTICLE_DEFAULT_HZ`] (64 Hz), a drained accumulator, and zero steps — a world that has
    /// not yet rendered a frame asks the sim for no work.
    #[inline]
    fn default() -> Self {
        Self::from_hz(PARTICLE_DEFAULT_HZ)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One 64 Hz substep, exactly representable.
    const TS: f32 = 1.0 / 64.0;

    /// A deterministic LCG — the property tests need a wide, reproducible spread of deltas without
    /// pulling a third-party generator into this crate's dependency graph.
    struct Lcg(u64);

    impl Lcg {
        fn next_u32(&mut self) -> u32 {
            // Numerical Recipes' 64-bit LCG constants; only the high bits are used.
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 32) as u32
        }

        /// A value in `[lo, hi)`. 24 bits of mantissa, so `unit` is exact.
        fn next_f32(&mut self, lo: f32, hi: f32) -> f32 {
            let unit = (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32;
            lo + unit * (hi - lo)
        }
    }

    // ── Construction ────────────────────────────────────────────────────────

    #[test]
    fn default_is_the_engine_fixed_rate_with_a_drained_accumulator() {
        let clock = ParticleClock::default();
        assert_eq!(clock.timestep(), TS, "the default rate is 64 Hz, exactly representable");
        assert_eq!(clock.steps(), 0, "a world that has not rendered asks for no substeps");
        assert_eq!(clock.dropped_steps(), 0);
        assert_eq!(clock.overstep_fraction(), 0.0);
    }

    #[test]
    fn from_hz_inverts_the_rate() {
        assert_eq!(ParticleClock::from_hz(128.0).timestep(), 1.0 / 128.0);
        assert_eq!(ParticleClock::from_hz(30.0).timestep(), 1.0 / 30.0);
    }

    // ── The M3 clamp, with the plan's exact numbers (gate #10) ──────────────

    /// Gate #10 (M3), with the plan's exact numbers: a frame carrying **129** whole substeps of
    /// virtual time clamps to **64**, counts **+65** dropped, drains the accumulator to its
    /// fractional remainder, and reports the SAME 64 at both consumers (A1's `dt` and the sim's
    /// push constant read one field).
    ///
    /// 129 is the F27b worst-case bound `⌈(max_delta × speed + timestep) / timestep⌉` — at the
    /// stock 250 ms `max_delta` and `relative_speed = 8.0` that is `⌈(2.0 + 1/64) / (1/64)⌉ =
    /// 129`, the `+ timestep` term accounting for the sub-step remainder a frame may carry in.
    /// The delta is fed directly here because `advance` takes the virtual delta as a parameter:
    /// that is what makes the clamp unit-testable without a `Time`, a `World` or a frame loop.
    #[test]
    fn gate10_a_129_substep_frame_clamps_to_64_and_drops_65() {
        let mut clock = ParticleClock::from_hz(64.0);

        clock.advance(129.0 * TS);

        assert_eq!(clock.steps(), 64, "the ceiling is THE clamp, applied once, on the host");
        assert_eq!(clock.dropped_steps(), 65, "129 - 64 = 65 substeps of particle time dropped");
        assert!(
            clock.accumulator >= 0.0 && clock.accumulator < TS,
            "the accumulator drains to its fractional remainder, not to a 65-step backlog: {}",
            clock.accumulator
        );

        // ONE value, two consumers: A1's emitter advance and the sim's loop bound both read this
        // field. Asserting the derived `dt` here is what makes "one number" checkable rather than
        // merely stated.
        let dt = clock.steps() as f32 * clock.timestep();
        assert_eq!(dt, 64.0 * TS, "A1's dt is steps * timestep, from the SAME clamped steps");
        assert_eq!(clock.steps(), 64, "the push constant reads the same field, unmodified");
    }

    /// The stock-defaults arithmetic behind gate #10's scenario, recorded exactly: `max_delta`
    /// 250 ms scaled by `relative_speed = 8.0` is 2.0 s of virtual delta, which is `128` whole
    /// substeps at 64 Hz. 129 is the CEILING bound (one substep of slack for a carried remainder),
    /// not the floor value this specific frame produces — both exceed the 64 ceiling, which is the
    /// property M3 is about.
    #[test]
    fn stock_defaults_at_speed_8_on_a_250ms_hitch_ask_for_128_and_still_clamp() {
        let mut clock = ParticleClock::from_hz(64.0);

        // min(raw, max_delta) * relative_speed == 0.250 * 8.0
        clock.advance(0.250 * 8.0);

        assert_eq!(clock.steps(), 64, "128 requested, 64 granted");
        assert_eq!(clock.dropped_steps(), 64, "128 - 64 = 64 dropped");
    }

    /// The boundary itself: exactly `PARTICLE_SUBSTEP_CEILING` substeps of time drops NOTHING —
    /// the clamp is `min`, so the ceiling is attainable, not one short of it.
    #[test]
    fn exactly_the_ceiling_steps_drops_nothing() {
        let mut clock = ParticleClock::from_hz(64.0);

        clock.advance(PARTICLE_SUBSTEP_CEILING as f32 * TS);

        assert_eq!(clock.steps(), PARTICLE_SUBSTEP_CEILING);
        assert_eq!(clock.dropped_steps(), 0, "the ceiling is inclusive");
    }

    /// One substep past the ceiling drops exactly one.
    #[test]
    fn one_past_the_ceiling_drops_exactly_one() {
        let mut clock = ParticleClock::from_hz(64.0);

        clock.advance((PARTICLE_SUBSTEP_CEILING + 1) as f32 * TS);

        assert_eq!(clock.steps(), PARTICLE_SUBSTEP_CEILING);
        assert_eq!(clock.dropped_steps(), 1);
    }

    // ── Edge cases the plan names ───────────────────────────────────────────

    /// M6, the COMMON case above the step rate: at 200 fps against a 64 Hz timestep, two frames in
    /// three step ZERO times. Stated as a limitation rather than discovered as a bug — and pinned,
    /// so a future "fix" that makes every frame step at least once has to argue with this test.
    #[test]
    fn steps_zero_is_the_common_case_above_the_step_rate() {
        let mut clock = ParticleClock::from_hz(64.0);
        let frame = 1.0 / 200.0;

        let mut zero_frames = 0;
        let mut total_steps = 0u32;
        for _ in 0..300 {
            clock.advance(frame);
            if clock.steps() == 0 {
                zero_frames += 1;
            }
            total_steps += clock.steps();
        }

        // 300 frames at 200 fps is 1.5 s of virtual time == 96 substeps at 64 Hz (±1 for the f32
        // accumulation of a delta that is not exactly representable).
        assert!(
            total_steps.abs_diff(96) <= 1,
            "the rate is preserved: 1.5 s at 64 Hz is ~96 substeps, got {total_steps}"
        );
        // Every frame steps 0 or 1 times here (one 200 fps frame is well under one 64 Hz substep),
        // so the zero-frames count is the complement — roughly TWO IN THREE (M6).
        assert_eq!(zero_frames, 300 - total_steps, "the remaining frames step zero times (M6)");
        assert!(zero_frames > 300 / 2, "above the step rate, MOST frames step zero times");
        assert_eq!(clock.dropped_steps(), 0, "no time is dropped below the ceiling");
    }

    /// Pause (`relative_speed == 0`, or `Time::pause()`) yields a zero delta, and a zero delta
    /// yields zero steps and no accumulation — particles freeze with the game, for free.
    #[test]
    fn a_zero_delta_pause_steps_zero_times_forever() {
        let mut clock = ParticleClock::from_hz(64.0);
        // Bank a partial substep first, so "zero steps" is not trivially true from an empty clock.
        clock.advance(TS * 0.5);
        let banked = clock.accumulator;

        for _ in 0..1_000 {
            clock.advance(0.0);
            assert_eq!(clock.steps(), 0, "a paused frame asks the sim for no substeps");
        }

        assert_eq!(clock.accumulator, banked, "a paused frame neither consumes nor accrues time");
        assert_eq!(clock.dropped_steps(), 0);
    }

    /// The fractional carry is EXACT across frames: sub-substep deltas accumulate and fire a
    /// substep the moment they cross the timestep, losing nothing below the ceiling.
    #[test]
    fn sub_substep_deltas_accumulate_and_fire_exactly_once_at_the_crossing() {
        let mut clock = ParticleClock::from_hz(64.0);
        let quarter = TS / 4.0;

        for _ in 0..3 {
            clock.advance(quarter);
            assert_eq!(clock.steps(), 0, "three quarters of a substep is not a substep");
        }
        clock.advance(quarter);
        assert_eq!(clock.steps(), 1, "the fourth quarter completes the substep");
        assert_eq!(clock.overstep_fraction(), 0.0, "and drains the accumulator exactly");
    }

    // ── Properties (deterministic LCG over a wide input spread) ─────────────

    /// The plan's clock properties, over an arbitrary `(delta, timestep)` spread:
    ///
    /// 1. `steps == min(floor(acc_after_add / timestep), CEILING)` — recomputed from the observed
    ///    PRE-state, not from a restatement of the impl;
    /// 2. the accumulator never grows without bound (it stays in `[0, timestep)` after every
    ///    advance, so `overstep_fraction()` stays in `[0, 1)`);
    /// 3. `dropped_steps` is monotone non-decreasing;
    /// 4. `steps <= PARTICLE_SUBSTEP_CEILING`, always.
    #[test]
    fn clock_properties_hold_over_arbitrary_deltas_and_rates() {
        let mut rng = Lcg(0x5EED_1234_ABCD_0001);

        for case in 0..64 {
            // A wide rate spread: 8 Hz .. 1024 Hz.
            let hz = rng.next_f32(8.0, 1024.0);
            let mut clock = ParticleClock::from_hz(hz);
            let ts = clock.timestep();
            let mut prev_dropped = 0u32;

            for _ in 0..256 {
                // Deltas from "far below a substep" to "far past the ceiling", so both the
                // accumulate arm and the clamp arm are exercised in every case.
                let delta = rng.next_f32(0.0, 4.0 * PARTICLE_SUBSTEP_CEILING as f32 * ts);

                let acc_before = clock.accumulator;
                clock.advance(delta);

                let expected_raw = ((acc_before + delta) / ts).floor() as u32;
                let expected_steps = expected_raw.min(PARTICLE_SUBSTEP_CEILING);
                assert_eq!(
                    clock.steps(),
                    expected_steps,
                    "case {case}: steps must be min(floor(acc/timestep), CEILING)"
                );
                assert!(
                    clock.steps() <= PARTICLE_SUBSTEP_CEILING,
                    "case {case}: the ceiling is the host-side clamp"
                );
                assert!(
                    clock.accumulator >= 0.0 && clock.accumulator < ts,
                    "case {case}: the accumulator must not grow without bound (got {})",
                    clock.accumulator
                );
                let over = clock.overstep_fraction();
                assert!(
                    (0.0..1.0).contains(&over),
                    "case {case}: overstep_fraction must be in [0, 1) (got {over})"
                );
                assert!(
                    clock.dropped_steps() >= prev_dropped,
                    "case {case}: dropped_steps is monotone"
                );
                prev_dropped = clock.dropped_steps();
            }
        }
    }

    /// The rate-preservation property below the ceiling: over a script whose total virtual time
    /// never asks for more than the ceiling in any one frame, the substeps actually run equal
    /// `floor(total / timestep)` — nothing is lost, nothing is invented, and `dropped_steps` stays
    /// zero.
    #[test]
    fn below_the_ceiling_the_rate_is_preserved_exactly() {
        let mut rng = Lcg(0xC10C_C10C_0000_0007);
        let mut clock = ParticleClock::from_hz(64.0);

        let mut total_time = 0.0f32;
        let mut total_steps = 0u32;
        for _ in 0..2_000 {
            // At most 16 substeps per frame — comfortably under the 64 ceiling.
            let delta = rng.next_f32(0.0, 16.0 * TS);
            total_time += delta;
            clock.advance(delta);
            total_steps += clock.steps();
        }

        assert_eq!(clock.dropped_steps(), 0, "no frame reached the ceiling");
        // One substep of slack for the f32 accumulation of 2 000 deltas.
        let expected = (total_time / TS).floor() as u32;
        assert!(
            total_steps.abs_diff(expected) <= 1,
            "substeps run ({total_steps}) must track floor(total/timestep) ({expected})"
        );
    }
}
