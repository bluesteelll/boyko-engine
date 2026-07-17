//! TAA rung C1 — the ECS-native TAA tunable config the author sets, plus the cold policy that
//! maps its live scalars onto the existing [`ResolvedTaa`] UBO carrier.
//!
//! Principle 0: ECS-native — [`TaaConfig`] is the author-set `#[derive(Resource)]` singleton
//! (the cold config, NOT a side `std::Vec`/`HashMap`) and [`ResolvedTaa`] is its derived UBO
//! companion, mirroring the shadow-denoise substrate exactly:
//! [`ShadowDenoiseConfig`](crate::shadow_denoise_config::ShadowDenoiseConfig) (the author
//! config) + [`ResolvedShadowDenoise`](crate::shadow_denoise_config::ResolvedShadowDenoise) (the
//! derived UBO) + [`resolve_shadow_denoise_policy`](crate::shadow_denoise_config::resolve_shadow_denoise_policy)
//! (the cold single-writer).
//!
//! # `ResolvedTaa` already existed with no writer
//!
//! [`ResolvedTaa`](crate::aa_config::ResolvedTaa) (`aa_config.rs`) shipped with its three live
//! fields carrying the shipped v1 tuning as HARDCODED literals in its `Default` impl — no
//! `World` policy ever wrote it. [`resolve_taa_policy`] is that missing single writer,
//! completing the substrate the same way [`resolve_shadow_denoise_policy`] completes
//! [`ResolvedShadowDenoise`].
//!
//! # Full knob surface now, wired incrementally (clean-architecture-first-time)
//!
//! [`TaaConfig`] declares every tunable the shipped `taa_resolve.comp.hlsl` algorithm has a
//! decision point for, each defaulting to the CURRENTLY SHIPPED behaviour (sourced from
//! `taa_resolve.comp.hlsl` itself, [`ResolvedTaa`]'s prior hardcoded defaults, or — for
//! [`TaaConfig::depth_tol`], which has no shipped TAA-side constant — the sibling temporal-
//! denoise substrate [`ShadowDenoiseConfig::disocclusion_depth_tol`](crate::shadow_denoise_config::ShadowDenoiseConfig::disocclusion_depth_tol)
//! this shader's own module doc says it is "Modeled on"). Declaring the full surface up front
//! avoids an interim struct shape a later rung would have to widen; **only
//! [`TaaConfig::jitter_scope`] is WIRED this rung** — every other field is read by
//! [`resolve_taa_policy`] (the three fields [`ResolvedTaa`] already carries) or carried inert
//! (declared, not yet consumed by any pass). Each field's doc states which.

use boyko_macros::Resource;

use boyko_ecs::ecs::core::system::{Res, ResMut};

use crate::aa_config::ResolvedTaa;

// ---- the knob enums (capability is structural: the shipped behaviour is each enum's #[default]) --

/// The sub-pixel jitter sequence [`TaaConfig::jitter`] selects. Only
/// [`Halton23`](Self::Halton23) is wired — the shipped 8-tap
/// [`HALTON_8`](crate::taa_jitter::HALTON_8) table. `R2`/`Off` are declared for the full knob
/// surface but read by no resolve this rung — a future rung would wire them into
/// [`crate::taa_jitter`]'s table selection.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum JitterSequence {
    /// The shipped 8-tap Halton(2,3) table ([`HALTON_8`](crate::taa_jitter::HALTON_8)). The
    /// DEFAULT — today's only implemented sequence.
    #[default]
    Halton23,
    /// A quasi-random R2 low-discrepancy sequence — declared, NOT wired this rung.
    R2,
    /// No jitter (a diagnostic mode: temporal accumulation over a static sample grid) —
    /// declared, NOT wired this rung.
    Off,
}

/// Which camera surfaces [`TaaConfig`]'s sub-pixel jitter perturbs. Capability is structural
/// (mirrors [`AaMode`](crate::aa_config::AaMode)'s `Off`-keyed gate): the CHOICE of scope, not a
/// bool, decides which host producer runs.
///
/// **WIRED this rung**: `boyko_app::runner`'s frame loop reads
/// [`TaaConfig::basis_shear_enabled`] to decide whether to shear the b5 camera basis via
/// [`composite_perspective_from_view_sheared`](crate::composite_perspective_from_view_sheared).
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum JitterScope {
    /// Shipped v1 (the C1 cut — see [`crate::taa_jitter`]'s module doc): ONLY the raster mesh
    /// vertex push is jittered
    /// ([`gbuffer_push_from_view_jittered`](crate::gbuffer_push_from_view_jittered)); the b5
    /// marcher/resolve/SSAO/CSM/froxel-shared camera basis stays UNJITTERED. The DEFAULT — a
    /// world that never opts in renders byte-identically to today's raster-only jitter.
    #[default]
    RasterOnly,
    /// Rung C1 (this rung): the SAME `(jx, jy)` jitter ALSO shears the b5 camera forward basis,
    /// so the SDF marcher supersamples too — I2 (shared sub-pixel position) now holds across
    /// BOTH legs, not just the raster mesh. See `docs/TAA-PLAN.md` Decision 1 for the
    /// derivation [`composite_perspective_from_view_sheared`](crate::composite_perspective_from_view_sheared)
    /// implements.
    RasterAndBasis,
}

/// The neighborhood bound shape [`TaaConfig`]'s history clip evaluates against. Only
/// [`Variance`](Self::Variance) is wired — the shipped `mean ± γ·σ` AABB, Salvi-style
/// (`taa_resolve.comp.hlsl`'s `aabb_min`/`aabb_max`). Declared for the full knob surface; NOT
/// read by any resolve this rung.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ClampShape {
    /// No neighborhood bound — raw history, unclipped. Declared, NOT wired this rung.
    Off,
    /// The 3×3 neighborhood min/max box. Declared, NOT wired this rung.
    MinMax,
    /// `mean ± variance_gamma * sigma` (the shipped shape). The DEFAULT.
    #[default]
    Variance,
}

/// The color space [`TaaConfig`]'s neighborhood clamp is evaluated in. Only
/// [`Rgb`](Self::Rgb) is wired — the shipped resolve clips the raw LDR `lit` RGB directly, no
/// color-space transform (`taa_resolve.comp.hlsl`'s moments loop operates on `.rgb` texels
/// as-loaded). Declared, NOT wired this rung.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ClampSpace {
    /// Direct RGB (the shipped shape). The DEFAULT.
    #[default]
    Rgb,
    /// YCoCg (Karis/Salvi's decorrelated luma-chroma space) — declared, NOT wired this rung.
    YCoCg,
}

/// How [`TaaConfig`]'s out-of-bound history sample is pulled back into the neighborhood. Only
/// [`TowardCenter`](Self::TowardCenter) is wired — the shipped Karis/Lottes directional clip
/// (`taa_resolve.comp.hlsl`'s `clip_toward_aabb_center`, preserves hue/saturation rather than
/// shifting it). Declared, NOT wired this rung.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ClipMode {
    /// Per-channel clamp to `[aabb_min, aabb_max]` — can shift hue. Declared, NOT wired.
    Clamp,
    /// Pull back along the ray from the AABB center through the color (the shipped shape). The
    /// DEFAULT.
    #[default]
    TowardCenter,
}

/// The temporal feedback STRATEGY [`TaaConfig::default_blend`]/[`TaaConfig::min_blend`]
/// parameterize. Only [`ConfidenceAdaptive`](Self::ConfidenceAdaptive) is wired — the shipped
/// `clamp(1 / confidence, min_blend, default_blend)` ramp (`taa_resolve.comp.hlsl`'s
/// `blend_factor`). Declared, NOT wired this rung — [`resolve_taa_policy`] forwards
/// `default_blend`/`min_blend` regardless of this field's value.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BlendMode {
    /// `blend_factor = clamp(1 / confidence, min_blend, default_blend)` (the shipped shape). The
    /// DEFAULT.
    #[default]
    ConfidenceAdaptive,
    /// A fixed feedback weight, ignoring the accumulated-frame confidence counter. Declared, NOT
    /// wired this rung.
    Fixed,
}

/// The history reconstruction filter [`TaaConfig`] selects. Only
/// [`CatmullRom`](Self::CatmullRom) is wired — the shipped 16-tap separable bicubic
/// reconstruction (`taa_resolve.comp.hlsl`'s `sample_history_catmull_rom`). Declared, NOT wired
/// this rung.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum HistoryFilter {
    /// A single bilinear tap — cheaper, blurs faster under repeated accumulation. Declared, NOT
    /// wired this rung.
    Bilinear,
    /// The shipped 16-tap separable bicubic Catmull-Rom. The DEFAULT.
    #[default]
    CatmullRom,
}

/// The motion-vector source [`TaaConfig`]'s resolve reprojects with. Only
/// [`CameraOnly`](Self::CameraOnly) is wired — the shipped `gViewT` + shared camera basis +
/// `MotionCam.prev_view_proj` reconstruction (the C1 v1 scope; see `taa_resolve.comp.hlsl`'s
/// module doc). Declared, NOT wired this rung.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MvSource {
    /// Reproject through the camera-only ray (the shipped shape; exact for a moving camera over
    /// static geometry, and for a fully static scene). The DEFAULT.
    #[default]
    CameraOnly,
    /// A per-object motion-vector buffer (the `hwrt`-only mesh-shadow MV producer's future
    /// sibling). Declared, NOT wired this rung.
    PerObject,
}

/// The history-reset (disocclusion) test [`TaaConfig`] evaluates. Only
/// [`OffScreenOnly`](Self::OffScreenOnly) is wired — the shipped off-screen/behind-camera test
/// (`taa_resolve.comp.hlsl`'s `off_screen` check). Declared, NOT wired this rung —
/// [`TaaConfig::depth_tol`] stays unread while this field is `OffScreenOnly`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DisocclusionTest {
    /// Reset iff the reprojected UV is off-screen or behind the camera (the shipped shape). The
    /// DEFAULT.
    #[default]
    OffScreenOnly,
    /// ALSO reset on a reprojected-vs-current depth mismatch beyond [`TaaConfig::depth_tol`] —
    /// mirrors `shadow_temporal.comp.hlsl`'s `depth_swap` test. Declared, NOT wired this rung.
    OffScreenAndDepth,
}

/// A post-resolve sharpening pass [`TaaConfig`] may select. Only [`None`](Self::None) is wired
/// — the shipped resolve writes `aa_out` directly, no sharpen pass. Declared, NOT wired this
/// rung.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SharpenMode {
    /// No sharpen pass (the shipped shape). The DEFAULT.
    #[default]
    None,
    /// AMD RCAS-style contrast-adaptive sharpen. Declared, NOT wired this rung.
    Rcas,
}

// ---- TaaConfig (the author-set Resource — mirrors ShadowDenoiseConfig) ----------------------

/// The author-facing TAA tunable surface — a `World`-singleton Resource mirroring
/// [`ShadowDenoiseConfig`](crate::shadow_denoise_config::ShadowDenoiseConfig) /
/// [`SsaoConfig`](crate::ssao_config::SsaoConfig): the cold, owner-set config
/// [`resolve_taa_policy`] maps onto the derived [`ResolvedTaa`] UBO carrier every frame.
///
/// See the module doc for the "full surface now, wired incrementally" rationale — every field
/// below states its shipped source and whether it is read this rung.
///
/// `#[derive(Resource)]` via [`boyko_macros::Resource`] (the same derive path
/// [`ShadowDenoiseConfig`](crate::shadow_denoise_config::ShadowDenoiseConfig) uses).
#[derive(Resource, Clone, Copy, Debug)]
pub struct TaaConfig {
    /// The sub-pixel jitter sequence. Only [`JitterSequence::Halton23`] is wired (the shipped
    /// [`HALTON_8`](crate::taa_jitter::HALTON_8) table). Default `Halton23`.
    pub jitter: JitterSequence,
    /// The jitter cycle length. `8` mirrors [`HALTON_8`](crate::taa_jitter::HALTON_8)'s shipped
    /// length; NOT read by [`crate::taa_jitter::ndc_jitter`] this rung (the table length is a
    /// compile-time const there). Default `8`.
    pub jitter_samples: u32,
    /// Which camera surfaces the jitter perturbs — see [`JitterScope`]. **WIRED this rung**:
    /// [`basis_shear_enabled`](Self::basis_shear_enabled) gates the b5 camera-basis shear at the
    /// host call site (`boyko_app::runner`). Default [`JitterScope::RasterOnly`] (today's
    /// shipped raster-only jitter).
    pub jitter_scope: JitterScope,
    /// The neighborhood bound shape — see [`ClampShape`]. Default [`ClampShape::Variance`] (the
    /// shipped `mean ± γσ` AABB).
    pub clamp: ClampShape,
    /// The color space the clamp AABB is computed in — see [`ClampSpace`]. Default
    /// [`ClampSpace::Rgb`] (the shipped direct-RGB clip).
    pub clamp_space: ClampSpace,
    /// How an out-of-bound history sample is pulled back — see [`ClipMode`]. Default
    /// [`ClipMode::TowardCenter`] (the shipped Karis/Lottes directional clip).
    pub clip: ClipMode,
    /// The clip AABB half-width scale (`× σ`, Salvi-style). Forwarded into
    /// [`ResolvedTaa::variance_gamma`] by [`resolve_taa_policy`]. Shipped default `1.0`
    /// (`ResolvedTaa`'s prior hardcoded value).
    pub variance_gamma: f32,
    /// The feedback weight at confidence == 1 (just after a reset). Forwarded into
    /// [`ResolvedTaa::default_blend`] by [`resolve_taa_policy`]. Shipped default `0.1`.
    pub default_blend: f32,
    /// The steady-state feedback floor (confidence → ∞). Forwarded into
    /// [`ResolvedTaa::min_blend`] by [`resolve_taa_policy`]. Shipped default `0.015`.
    pub min_blend: f32,
    /// The blend STRATEGY [`default_blend`](Self::default_blend)/[`min_blend`](Self::min_blend)
    /// parameterize — see [`BlendMode`]. Default [`BlendMode::ConfidenceAdaptive`] (the shipped
    /// ramp).
    pub blend: BlendMode,
    /// Whether the blend is Karis inverse-tonemap luma-weighted (`w = 1 / (1 + luma)`,
    /// suppressing a single bright outlier tap from dominating the average). The shipped resolve
    /// always applies it. Default `true`.
    pub luma_weight: bool,
    /// The history reconstruction filter — see [`HistoryFilter`]. Default
    /// [`HistoryFilter::CatmullRom`] (the shipped 16-tap separable bicubic).
    pub history_filter: HistoryFilter,
    /// The motion-vector source — see [`MvSource`]. Default [`MvSource::CameraOnly`] (the
    /// shipped C1 v1 scope).
    pub mv_source: MvSource,
    /// The history-reset (disocclusion) test — see [`DisocclusionTest`]. Default
    /// [`DisocclusionTest::OffScreenOnly`] (the shipped off-screen/behind-camera test).
    pub disocclusion: DisocclusionTest,
    /// The relative depth-mismatch tolerance a future [`DisocclusionTest::OffScreenAndDepth`]
    /// would consume. UNREAD while [`disocclusion`](Self::disocclusion) stays `OffScreenOnly`.
    /// Default `0.02` — TAA's own resolve has no depth-tolerance constant to source from (it
    /// tests off-screen only); this is sourced instead from
    /// [`ShadowDenoiseConfig::disocclusion_depth_tol`](crate::shadow_denoise_config::ShadowDenoiseConfig::disocclusion_depth_tol)'s
    /// shipped default (`0.02`) — the sibling temporal-denoise substrate
    /// `taa_resolve.comp.hlsl`'s own module doc says this shader is "Modeled on"
    /// (`shadow_temporal.comp.hlsl`'s `depth_swap` test uses the identical relative-tolerance
    /// shape this field would parameterize).
    pub depth_tol: f32,
    /// A post-resolve sharpen pass — see [`SharpenMode`]. Default [`SharpenMode::None`] (the
    /// shipped resolve has no sharpen pass).
    pub sharpen: SharpenMode,
}

impl Default for TaaConfig {
    /// Every field defaults to the CURRENTLY SHIPPED behaviour — see each field's doc for its
    /// source. A world that never customizes [`TaaConfig`] resolves to
    /// [`ResolvedTaa::default`]'s prior hardcoded values (via [`resolve_taa`]) and never opts
    /// into the C1 basis shear (`jitter_scope == RasterOnly`).
    #[inline]
    fn default() -> Self {
        Self {
            jitter: JitterSequence::Halton23,
            jitter_samples: 8,
            jitter_scope: JitterScope::RasterOnly,
            clamp: ClampShape::Variance,
            clamp_space: ClampSpace::Rgb,
            clip: ClipMode::TowardCenter,
            variance_gamma: 1.0,
            default_blend: 0.1,
            min_blend: 0.015,
            blend: BlendMode::ConfidenceAdaptive,
            luma_weight: true,
            history_filter: HistoryFilter::CatmullRom,
            mv_source: MvSource::CameraOnly,
            disocclusion: DisocclusionTest::OffScreenOnly,
            depth_tol: 0.02,
            sharpen: SharpenMode::None,
        }
    }
}

impl TaaConfig {
    /// Whether the b5 camera-basis shear runs — the structural predicate
    /// `jitter_scope == RasterAndBasis` (NOT stored state), mirroring
    /// [`SsaoConfig::enabled`](crate::ssao_config::SsaoConfig::enabled)'s shape. The host call
    /// site (`boyko_app::runner`) reads this (ANDed with the frame's TAA-armed state) to decide
    /// whether to pass `Some(jitter)` or the structural-skip `None` into
    /// [`composite_perspective_from_view_sheared`](crate::composite_perspective_from_view_sheared).
    #[inline]
    pub const fn basis_shear_enabled(&self) -> bool {
        matches!(self.jitter_scope, JitterScope::RasterAndBasis)
    }
}

// ---- the resolve decision (pure — mirrors resolve_shadow_denoise) ---------------------------

/// Maps a [`TaaConfig`] onto the existing [`ResolvedTaa`] UBO carrier — the pure, unit-testable
/// resolve [`resolve_taa_policy`] wraps. Forwards ONLY the three fields [`ResolvedTaa`] has
/// ([`variance_gamma`](TaaConfig::variance_gamma), [`default_blend`](TaaConfig::default_blend),
/// [`min_blend`](TaaConfig::min_blend)); every other [`TaaConfig`] field is host-only or not yet
/// wired into any UBO (see each field's doc).
#[inline]
pub fn resolve_taa(cfg: &TaaConfig) -> ResolvedTaa {
    ResolvedTaa {
        default_blend: cfg.default_blend,
        min_blend: cfg.min_blend,
        variance_gamma: cfg.variance_gamma,
        _pad: 0.0,
    }
}

// ---- the cold single-writer system (mirrors resolve_shadow_denoise_policy) ------------------

/// Single writer of [`ResolvedTaa`] (cold, once/frame) — the missing policy the module doc
/// describes. Mirrors
/// [`resolve_shadow_denoise_policy`](crate::shadow_denoise_config::resolve_shadow_denoise_policy)
/// / [`resolve_ssao_policy`](crate::ssao_config::resolve_ssao_policy). Reads the author
/// [`TaaConfig`] and writes the derived UBO carrier (the one-producer-per-field write
/// discipline).
//
// `clippy::needless_pass_by_value`: `Res`/`ResMut` are by-value `SystemParam`s read/written
// through reborrows — the same false-positive `resolve_shadow_denoise_policy` carries.
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_taa_policy(cfg: Res<TaaConfig>, mut out: ResMut<ResolvedTaa>) {
    *out = resolve_taa(&cfg);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TaaConfig::default()` carries every shipped-behaviour default this module's docs claim.
    #[test]
    fn default_config_matches_shipped_constants() {
        let cfg = TaaConfig::default();
        assert_eq!(cfg.jitter, JitterSequence::Halton23);
        assert_eq!(cfg.jitter_samples, 8);
        assert_eq!(cfg.jitter_scope, JitterScope::RasterOnly);
        assert_eq!(cfg.clamp, ClampShape::Variance);
        assert_eq!(cfg.clamp_space, ClampSpace::Rgb);
        assert_eq!(cfg.clip, ClipMode::TowardCenter);
        assert_eq!(cfg.variance_gamma, 1.0);
        assert_eq!(cfg.default_blend, 0.1);
        assert_eq!(cfg.min_blend, 0.015);
        assert_eq!(cfg.blend, BlendMode::ConfidenceAdaptive);
        assert!(cfg.luma_weight);
        assert_eq!(cfg.history_filter, HistoryFilter::CatmullRom);
        assert_eq!(cfg.mv_source, MvSource::CameraOnly);
        assert_eq!(cfg.disocclusion, DisocclusionTest::OffScreenOnly);
        assert_eq!(cfg.depth_tol, 0.02);
        assert_eq!(cfg.sharpen, SharpenMode::None);
        assert!(
            !cfg.basis_shear_enabled(),
            "the default config is the 0%-gate (raster-only jitter, matching today)"
        );
    }

    /// Capability is structural: `basis_shear_enabled` keys ONLY off `jitter_scope`.
    #[test]
    fn basis_shear_enabled_is_structural_over_jitter_scope() {
        let raster_only = TaaConfig { jitter_scope: JitterScope::RasterOnly, ..TaaConfig::default() };
        assert!(!raster_only.basis_shear_enabled());

        let raster_and_basis =
            TaaConfig { jitter_scope: JitterScope::RasterAndBasis, ..TaaConfig::default() };
        assert!(raster_and_basis.basis_shear_enabled());
    }

    /// `resolve_taa` forwards exactly the three live [`ResolvedTaa`] fields, padding zeroed.
    #[test]
    fn resolve_taa_maps_the_three_live_fields() {
        let cfg = TaaConfig {
            default_blend: 0.2,
            min_blend: 0.03,
            variance_gamma: 1.5,
            ..TaaConfig::default()
        };
        assert_eq!(
            resolve_taa(&cfg),
            ResolvedTaa { default_blend: 0.2, min_blend: 0.03, variance_gamma: 1.5, _pad: 0.0 }
        );
    }

    /// The `ResolvedTaa::default` shortcut must equal resolving a default `TaaConfig`, so a
    /// never-run policy already carries the correct shipped scalars.
    #[test]
    fn default_resolved_matches_resolving_the_default_config() {
        assert_eq!(ResolvedTaa::default(), resolve_taa(&TaaConfig::default()));
    }
}
