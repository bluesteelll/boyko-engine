//! TAA history reset control — a `#[derive(Resource)]` singleton (Principle 0) tracking
//! whether the next frame's temporal-resolve accumulation must be discarded.
//!
//! # Why a reset flag (not re-deriving it from `AaMode` transitions in the shader)
//!
//! The `taa_hist` ring is `Option`-guarded and only ever populated with meaningful history
//! once TAA has been armed for at least one frame. On the FIRST armed frame (or after a
//! resize, which reallocates `taa_hist` at a new extent), the previous frame's history slot
//! is either absent or stale — the resolve must replace rather than blend (`blend_factor ==
//! 1.0`, mirroring the shadow-temporal denoiser's `I5` single-frame fallback / disocclusion
//! reset). Deriving this host-side (rather than re-detecting a "history changed shape" case
//! inside the shader) keeps the reset an explicit, testable host decision.

use boyko_macros::Resource;

/// Whether the resolve must discard `taa_hist` and fall back to the current single-frame
/// sample this frame (`blend_factor == 1.0`), plus the frame count since the last reset
/// (owner-diagnostic; not yet consumed by the resolve, but tracked from the start so a v1.1
/// convergence-quality metric has continuous history).
///
/// The host sets `reset = true` on: TAA's first armed frame (a mode transition into `Taa`),
/// and an extent change (a resize invalidates `taa_hist`'s allocated size). The consumer
/// (the resolve record site) reads and clears it every frame — a checked-and-cleared flag,
/// not a sticky one.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaaState {
    /// `true` ⇒ the next resolve must force `blend_factor == 1.0` (replace, not blend).
    pub reset: bool,
    /// Frames elapsed since the last reset (saturates rather than wraps — an owner diagnostic).
    pub frames_since_reset: u32,
}

impl TaaState {
    /// Marks the history invalid — the host calls this on a `Taa` mode transition or a resize.
    #[inline]
    pub fn mark_reset(&mut self) {
        self.reset = true;
        self.frames_since_reset = 0;
    }

    /// Advances one frame: returns whether THIS frame must reset (consuming the flag), and
    /// bumps the reset-age counter for the frame that follows.
    ///
    /// The single seam that both reads and clears the flag, mirroring
    /// [`MotionCamState::advance`](crate::motion_cam::MotionCamState::advance)'s
    /// consume-and-update shape.
    #[inline]
    #[must_use]
    pub fn advance(&mut self) -> bool {
        let reset_now = self.reset;
        self.reset = false;
        self.frames_since_reset = self.frames_since_reset.saturating_add(1);
        reset_now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_not_reset() {
        let state = TaaState::default();
        assert!(!state.reset);
        assert_eq!(state.frames_since_reset, 0);
    }

    #[test]
    fn mark_reset_arms_and_zeros_the_age() {
        let mut state = TaaState { reset: false, frames_since_reset: 42 };
        state.mark_reset();
        assert!(state.reset);
        assert_eq!(state.frames_since_reset, 0);
    }

    #[test]
    fn advance_consumes_the_flag_exactly_once() {
        let mut state = TaaState::default();
        state.mark_reset();
        assert!(state.advance(), "the reset frame must report true");
        assert!(!state.reset, "the flag is cleared after being consumed");
        assert_eq!(state.frames_since_reset, 1, "the reset frame itself counts as frame 1");
        assert!(!state.advance(), "the following frame must not re-report reset");
        assert_eq!(state.frames_since_reset, 2, "the second call must advance the age again");
    }

    #[test]
    fn frames_since_reset_counts_up_and_saturates() {
        let mut state = TaaState::default();
        for expected in 1..=8u32 {
            let _ = state.advance();
            assert_eq!(state.frames_since_reset, expected);
        }
        state.frames_since_reset = u32::MAX;
        let _ = state.advance();
        assert_eq!(state.frames_since_reset, u32::MAX, "advance must saturate, never wrap");
    }
}
