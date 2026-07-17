//! TAA raster-only sub-pixel jitter (the C1-scoped v1 DEFAULT: marcher/SDF jitter is opt-in —
//! see the module docs on [`crate::view::marcher_view_proj_rows_jittered`]).
//!
//! # Why raster-only BY DEFAULT (C1)
//!
//! The b5 UBO `cam_forward` is O1-normalized and shared, RAW, by deferred PBR / SSAO / shadow
//! CSM-cascade-select / froxel light-slice view-z reconstruction — passes that run regardless
//! of the AA mode. Perturbing it to jitter the SDF marcher would inject a per-frame,
//! Halton-phase-correlated wobble into all of those UNLESS the perturbation is EXACTLY the
//! shear [`crate::view::composite_perspective_from_view_sheared`] applies (algebraically
//! absorbed by `generate_ray`'s own linear NDC formula — see that fn's doc for the derivation),
//! which is why v1 defaults to raster-only (`TaaConfig::jitter_scope ==
//! JitterScope::RasterOnly`) rather than jittering the shared basis unconditionally: an
//! explicit opt-in keeps every consumer's default behaviour byte-unchanged. This module's
//! [`NdcJitter`] is shared by BOTH the raster mesh push and (when opted in) the b5 shear, so a
//! world that never sets `jitter_scope = RasterAndBasis` renders exactly as before —
//! SDF-marched pixels stay temporally stable but un-supersampled.
//!
//! # The opt-in lift (`JitterScope::RasterAndBasis`, `crate::taa_config`)
//!
//! Setting `TaaConfig::jitter_scope = JitterScope::RasterAndBasis` makes `boyko_app::runner`
//! pass this module's `(jx, jy)` into
//! [`composite_perspective_from_view_sheared`](crate::view::composite_perspective_from_view_sheared)
//! as well, so the SDF marcher (and every other b5-consuming pass) supersamples too — a pure
//! HOST-side data perturbation of the SAME push-constant struct, touching zero shaders / `.spv`
//! artifacts. See `docs/TAA-PLAN.md` Decision 1 for the full derivation.
//!
//! # Principle 0 / ECS-native
//!
//! [`JitterState`] is a `#[derive(Resource)]` singleton — the frame-to-frame Halton phase lives
//! in the engine's own storage, not a host-side `static mut`. [`crate::aa_plugin::AaPlugin`]
//! inserts the default (`phase = 0, armed = false`).
//!
//! # OFF byte-identity (provable, not merely tested)
//!
//! [`ndc_jitter`] returns exactly `NdcJitter { jx: 0.0, jy: 0.0 }` when `!state.armed` — a
//! structural skip, not a `* 0.0` multiply. Every consumer (`view::*_jittered`) adds this
//! offset via `row0 += jx * row3; row1 += jy * row3`, so a `{0.0, 0.0}` jitter is an exact
//! additive zero: `AaMode::Off` (the default) is byte-identical to the pre-TAA render.

use boyko_macros::Resource;

/// 8-phase Halton(2,3) sub-pixel offsets, in PIXELS (range `[-0.5, +0.5]`) — the standard
/// low-discrepancy table (`halton(i, base) - 0.5`, indices 1..=8, the Bevy-shipped values).
///
/// NB: a finite Halton prefix is NOT exactly zero-mean — the y-column sums to `0.0`, but the
/// x-column sums to `-0.4375` (mean `-0.0547 px`), so the converged-static image carries a
/// ~0.055 px horizontal bias (imperceptible, and it matches every industry TAA using this
/// table). The bias is pinned by the `halton_8_x_sum_is_bounded` unit test; re-centering the
/// x-column would remove it at the cost of diverging from the reference sequence.
pub const HALTON_8: [[f32; 2]; 8] = [
    [0.0, -0.16667],
    [-0.25, 0.16667],
    [0.25, -0.38889],
    [-0.375, -0.05556],
    [0.125, 0.27778],
    [-0.125, -0.27778],
    [0.375, 0.05556],
    [-0.4375, 0.38889],
];

/// The owner-invisible per-frame jitter phase — a `#[derive(Resource)]` singleton (Principle 0).
/// Advanced once per frame by [`advance_jitter`], ONLY while TAA is armed; a TAA-off frame
/// leaves `phase` frozen and sets `armed = false`, so [`ndc_jitter`] returns the exact-zero
/// offset (the structural OFF skip).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct JitterState {
    /// `frame_count % 8` — the index into [`HALTON_8`] for the CURRENT frame (post-advance).
    pub phase: u32,
    /// `true` iff `AaMode::Taa` was the resolved mode this frame — the structural OFF-skip
    /// flag [`ndc_jitter`] reads.
    pub armed: bool,
}

/// The final-flipped-NDC jitter offset — the SINGLE source of truth
/// [`crate::view::marcher_view_proj_rows_jittered`] /
/// [`crate::view::gbuffer_push_from_view_jittered`] read. `{0.0, 0.0}` exactly when the
/// producing [`JitterState`] is `!armed`, so the projection is byte-identical to the
/// non-jittered path.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NdcJitter {
    /// Add-to-final-NDC.x (`row0 += jx * row3`).
    pub jx: f32,
    /// Add-to-final-NDC.y (`row1 += jy * row3`).
    pub jy: f32,
}

/// Maps the current [`JitterState`] to this frame's [`NdcJitter`] at a `width × height` extent —
/// the pure, unit-testable core both raster jitter consumers read.
///
/// Returns the exact `{0.0, 0.0}` offset when `!state.armed` (the structural OFF-skip — NOT a
/// `* 0.0` multiply on a nonzero base, which the project's byte-identity discipline forbids as
/// unprovable). When armed, maps the pixel-space [`HALTON_8`] sample at `state.phase` into
/// final-NDC space: a `[-0.5, +0.5]`-pixel offset spans `2/width` (resp. `2/height`) of NDC.
#[inline]
pub fn ndc_jitter(state: &JitterState, width: u32, height: u32) -> NdcJitter {
    debug_assert!(state.phase < 8, "invariant: JitterState.phase indexes HALTON_8 (< 8)");
    if !state.armed {
        return NdcJitter::default();
    }
    debug_assert!(width > 0 && height > 0, "invariant: the composite extent is non-zero");
    let [px, py] = HALTON_8[state.phase as usize];
    NdcJitter { jx: 2.0 * px / width as f32, jy: 2.0 * py / height as f32 }
}

/// Advances the jitter phase once per frame (cold — a handful of scalar ops). `armed` is the
/// caller's resolved `AaMode::Taa` predicate for THIS frame:
/// * `armed == true`: `phase = (phase + 1) % 8` (cycles through [`HALTON_8`]).
/// * `armed == false`: `phase` is left unchanged — a re-arm resumes the cycle rather than
///   restarting it, and [`ndc_jitter`] ignores `phase` entirely while `!armed`.
///
/// Either way `state.armed` is set to the caller's predicate, so [`ndc_jitter`] reads the
/// CURRENT frame's arm state.
#[inline]
pub fn advance_jitter(state: &mut JitterState, armed: bool) {
    if armed {
        state.phase = (state.phase + 1) % HALTON_8.len() as u32;
    }
    state.armed = armed;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndc_jitter_is_exact_zero_when_disarmed() {
        let state = JitterState { phase: 3, armed: false };
        let j = ndc_jitter(&state, 1920, 1080);
        assert_eq!(j, NdcJitter::default());
        assert_eq!(j.jx, 0.0);
        assert_eq!(j.jy, 0.0);
    }

    #[test]
    fn ndc_jitter_is_nonzero_when_armed_at_a_nonzero_phase() {
        let state = JitterState { phase: 1, armed: true };
        let j = ndc_jitter(&state, 1920, 1080);
        assert_ne!(j, NdcJitter::default());
    }

    #[test]
    fn ndc_jitter_matches_the_pixel_to_ndc_mapping() {
        let state = JitterState { phase: 2, armed: true };
        let j = ndc_jitter(&state, 100, 200);
        let [px, py] = HALTON_8[2];
        assert!((j.jx - 2.0 * px / 100.0).abs() < f32::EPSILON);
        assert!((j.jy - 2.0 * py / 200.0).abs() < f32::EPSILON);
    }

    /// The 8-tap table is a LOW-DISCREPANCY Halton(2,3) sequence, not an exact-zero-sum
    /// construction: `sum_y == 0.0` exactly (the dedicated test below), but `sum_x == -0.4375`
    /// — bounded (never exceeds) the single largest-magnitude tap, `-0.4375` (the 8th x
    /// entry), because every OTHER x pairs off into an exact cancellation (`+0.25/-0.25`,
    /// `+0.125/-0.125`, `+0.375/-0.375`). This pins the known value (a regression guard) rather
    /// than asserting a stronger "centered" property the table does not actually satisfy.
    #[test]
    fn halton_8_x_sum_is_bounded_by_the_max_single_tap_and_pinned() {
        let sum_x: f32 = HALTON_8.iter().map(|[x, _]| x).sum();
        let max_abs = HALTON_8
            .iter()
            .flat_map(|p| p.iter())
            .fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(
            sum_x.abs() <= max_abs,
            "HALTON_8 x-sum ({sum_x}) must never exceed the max single-tap magnitude ({max_abs})"
        );
        assert!((sum_x - (-0.4375)).abs() < 1e-6, "pinned: HALTON_8's x-column sums to -0.4375");
    }

    /// `sum_y == 0.0` exactly is a genuine (non-approximate) property of this specific table —
    /// pinned so a future edit does not silently drift it.
    #[test]
    fn halton_8_y_column_sums_to_exact_zero() {
        let sum_y: f32 = HALTON_8.iter().map(|[_, y]| y).sum();
        assert_eq!(sum_y, 0.0, "HALTON_8's y-column is pinned to sum exactly to zero");
    }

    #[test]
    fn advance_jitter_cycles_phase_only_while_armed() {
        let mut state = JitterState::default();
        for expected in 1..=8u32 {
            advance_jitter(&mut state, true);
            assert_eq!(state.phase, expected % 8);
            assert!(state.armed);
        }
        // Disarming freezes phase but flips the flag.
        let frozen = state.phase;
        advance_jitter(&mut state, false);
        assert_eq!(state.phase, frozen, "phase must freeze while disarmed");
        assert!(!state.armed);
        // Re-arming resumes the cycle rather than restarting it.
        advance_jitter(&mut state, true);
        assert_eq!(state.phase, (frozen + 1) % 8);
    }

    #[test]
    fn default_jitter_state_is_the_zero_gate() {
        let state = JitterState::default();
        assert_eq!(state.phase, 0);
        assert!(!state.armed);
        assert_eq!(ndc_jitter(&state, 800, 600), NdcJitter::default());
    }
}
