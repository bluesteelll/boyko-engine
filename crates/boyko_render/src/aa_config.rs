//! AA campaign — the ECS-native anti-aliasing config the USER sets, plus the cold policy
//! that maps it to the render driver's pass selection.
//!
//! Principle 0: ECS-native — [`AaConfig`] is a `#[derive(Resource)]` singleton (the cold
//! owner-set config, NOT a side `std::Vec`/`HashMap`), and [`ResolvedAa`] is its derived
//! companion Resource written by the cold [`resolve_aa_policy`] system. This mirrors the
//! SSAO substrate exactly ([`SsaoConfig`](crate::ssao_config::SsaoConfig) +
//! [`ResolvedSsao`](crate::ssao_config::ResolvedSsao) + `resolve_ssao_policy`), which in turn
//! mirrors the lighting StrategyPolicy substrate.
//!
//! # Capability is structural (no redundant `enabled: bool`)
//!
//! Whether an AA pass runs is keyed off the [`AaMode`] enum, NOT a separate flag —
//! [`AaMode::Off`] IS "disabled". This is the capability-is-structural principle (mirrors
//! [`SsaoQuality::Off`](crate::ssao_config::SsaoQuality::Off)). [`AaConfig::enabled`] is a
//! derived predicate (`mode != Off`), not stored state.
//!
//! # The 0%-gate
//!
//! [`AaConfig::default`] is [`AaMode::Off`] — byte-identical to today (no post-process AA
//! pass; the present-blit samples the deferred resolve's `lit` target directly). The render
//! driver maps `Off` to "no AA activation" (present samples `lit`, no `aa_out` target, no AA
//! pass recorded) — the byte-identity anchor for the golden gates.
//!
//! # Extensibility
//!
//! Stage 1 landed `Off` + `Fxaa`. Stage 2 added `Smaa` (3-pass morphological AA). Stage 3
//! adds `Ssaa` (2× ordered-grid supersampling) — a purely additive change; each new mode
//! plugs into the SAME framework (a post-process pass at the resolve→present seam writing
//! the shared `aa_out` target the present samples). Unlike `Fxaa`/`Smaa`, `Ssaa` is
//! **boot-fixed, host-authoritative**: the render scale is decided once at
//! `WindowHost::boot` (device-capability probe: `max_image_dimension_2d` + VRAM estimate,
//! degrading to `Off` on failure — never a panic) and the per-frame read site in
//! `boyko_app::runner` LOCKS the mode (`ssaa_armed ⇒ force Ssaa`, `!ssaa_armed ⇒ any Ssaa
//! degrades to Off`). This crate only carries the enum word; it cannot see the boot
//! resolution.

use boyko_macros::Resource;

use boyko_ecs::ecs::core::system::{Res, ResMut};

// ---- AaMode (the owner-set knob; capability is structural) ---------------------------

/// The anti-aliasing technique the owner sets on [`AaConfig`]. `#[repr(u32)]` so it can be
/// forwarded to the backend as a stable mode word.
///
/// [`Off`](AaMode::Off) is the structural "disabled" state (the capability-is-structural
/// principle): the render driver gates the whole post-process AA pass on `mode != Off`, so
/// there is NO redundant `enabled: bool` — exactly as
/// [`SsaoQuality`](crate::ssao_config::SsaoQuality) keys off its `Off` variant.
///
/// Stage 1 implemented `Off` + `Fxaa`; Stage 2 added `Smaa`; Stage 3 added `Ssaa`
/// (boot-fixed, host-authoritative — see `boyko_app::host::WindowHost` for the arming
/// probe). Stage 4 adds `Taa` — camera-reprojection temporal supersampling, live-toggleable
/// like `Fxaa`/`Smaa`. **v1 caveat**: only the raster mesh path is sub-pixel jittered (see
/// [`crate::taa_jitter`] for the C1 rationale — the SDF marcher stays un-jittered, so
/// SDF-marched pixels are temporally stable but un-supersampled); the temporal resolve is
/// landed OFF-byte-identical and
/// converged-static-validated, but in-motion quality (ghosting, disocclusion) is
/// owner-gated, not yet visually blessed.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AaMode {
    /// No post-process AA — the 0%-gate (the present-blit samples `lit` directly). The
    /// DEFAULT, so a world that never inserts a non-default [`AaConfig`] is byte-identical
    /// to today.
    #[default]
    Off,
    /// FXAA (Fast Approximate Anti-Aliasing) — a single-pass luma-edge post-process
    /// (`shaders/fxaa.fs.hlsl`). Cheap spatial AA, no history / motion vectors / jitter.
    Fxaa,
    /// SMAA 1x (Enhanced Subpixel Morphological Antialiasing, PRESET_HIGH) — a 3-pass
    /// morphological post-process (edge detection → blending-weight calculation →
    /// neighborhood blending; `shaders/smaa_{edge,weight,blend}.fs.hlsl`). Sharper
    /// diagonal/corner edges than FXAA at a higher per-frame cost (3 passes + 2 LUT
    /// reads vs FXAA's 1 pass).
    Smaa,
    /// 2× ordered-grid supersampling — the whole deferred pipeline renders at 2× per axis
    /// (4× pixels), then a linear-light box downsample (`shaders/ssaa_downsample.fs.hlsl`)
    /// resolves into a native-size `aa_out` the present-blit samples 1:1. Quality-reference
    /// AA (resolves geometry + shading + texture aliasing, not just post-process luma
    /// edges). **Render-scaled and boot-fixed**: the 2× resolution is committed at
    /// `WindowHost::boot` behind a device-capability probe (degrades to `Off`, never
    /// panics); this mode cannot be toggled live like `Fxaa`/`Smaa` — changing it requires
    /// a re-boot with a different `EnginePlugins::with_ssaa_scale`.
    Ssaa,
    /// TAA (Temporal Anti-Aliasing) — camera-reprojection temporal supersampling: a per-frame
    /// sub-pixel jitter of the raster mesh vertex push (`crate::taa_jitter`), accumulated
    /// through a color-history ring reprojected by the camera's motion (`crate::motion_cam`)
    /// and resolved with a variance-clipped, luma-weighted blend
    /// (`boyko_rhi_vulkan::shaders::taa_resolve`). Live-toggleable like `Fxaa`/`Smaa` (native
    /// resolution — no render-scale commitment, unlike `Ssaa`). **v1 scope**: only the raster
    /// mesh path is jittered/supersampled (SDF-marched pixels stay stable but un-supersampled
    /// — C1); in-motion quality is owner-gated (not yet visually blessed for motion).
    Taa,
}

impl AaMode {
    /// The stable mode word forwarded to the backend (the `#[repr(u32)]` discriminant).
    /// `Off => 0`, `Fxaa => 1`, `Smaa => 2`, `Ssaa => 3`, `Taa => 4`.
    #[inline]
    pub const fn as_word(self) -> u32 {
        self as u32
    }
}

// ---- AaConfig (the owner-set Resource — mirrors SsaoConfig) ---------------------------

/// The global anti-aliasing config — a `World`-singleton Resource the owner sets, the AA
/// analogue of [`SsaoConfig`](crate::ssao_config::SsaoConfig). Carries ONLY the [`AaMode`]
/// knob: enablement is structural (`mode != Off`), so there is no separate flag.
///
/// `#[derive(Resource)]` via [`boyko_macros::Resource`] (the same derive path `SsaoConfig`
/// uses).
#[derive(Resource, Clone, Copy, Debug)]
pub struct AaConfig {
    /// The owner-set AA technique. [`Off`](AaMode::Off) (the default) ⇒ no pass.
    pub mode: AaMode,
}

impl Default for AaConfig {
    #[inline]
    fn default() -> Self {
        // Off == today (the 0%-gate anchor): a default world runs no post-process AA pass.
        Self { mode: AaMode::Off }
    }
}

impl AaConfig {
    /// Whether a post-process AA pass runs — the structural predicate `mode != Off` (NOT
    /// stored state). True ⇒ the `aa_out` target + the AA pass are wired; false ⇒ the
    /// 0%-gate (present samples `lit`). Mirrors reading the mode in the SSAO policy rather
    /// than a redundant `bool`.
    #[inline]
    pub const fn enabled(&self) -> bool {
        !matches!(self.mode, AaMode::Off)
    }
}

// ---- ResolvedAa (the derived carrier — mirrors ResolvedSsao) --------------------------

/// The derived AA selection the render driver reads — the AA analogue of
/// [`ResolvedSsao`](crate::ssao_config::ResolvedSsao). [`resolve_aa_policy`] is its SINGLE
/// writer (the one-producer-per-field write discipline), recomputing it from [`AaConfig`]
/// each policy run. `#[repr(C)]` for a stable layout.
///
/// The render driver reads `mode` to (1) decide whether to build the `aa_out` target + the
/// AA activation (`mode != Off`) and (2) select which post-process pass to record.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct ResolvedAa {
    /// The resolved AA technique the driver enacts (`Off` ⇒ no pass — the 0%-gate).
    pub mode: AaMode,
}

impl Default for ResolvedAa {
    #[inline]
    fn default() -> Self {
        // The resolve of the default `AaConfig` (Off) — the 0%-gate, so a never-run policy
        // already reads the no-pass selection.
        resolve_aa(&AaConfig::default())
    }
}

impl ResolvedAa {
    /// Whether a post-process AA pass runs (`mode != Off`) — the same structural predicate
    /// as [`AaConfig::enabled`], read off the derived carrier the driver holds.
    #[inline]
    pub const fn enabled(&self) -> bool {
        !matches!(self.mode, AaMode::Off)
    }
}

// ---- the resolve decision (pure — the AA analogue of `resolve_ssao`) ------------------

/// Maps an [`AaConfig`] to its derived [`ResolvedAa`] — the pure AA resolve decision (the
/// unit-testable core the cold system wraps). An identity forward of the mode: the SSAA
/// device-capability degrade (an extent the device cannot allocate → `Off`) happens
/// host-side at `WindowHost::boot`, BEFORE this crate ever sees a mode word — this crate
/// cannot see the boot resolution, so the seam here stays identity. The doc'd degrade-seam
/// intent still mirrors how `resolve_ssao` centralises the SSAO variant decision, for any
/// future ECS-visible degrade this crate CAN decide.
#[inline]
pub fn resolve_aa(cfg: &AaConfig) -> ResolvedAa {
    ResolvedAa { mode: cfg.mode }
}

// ---- the cold StrategyPolicy system (mirrors `resolve_ssao_policy`) --------------------

/// The cold AA resolve policy — reads [`AaConfig`] and writes the derived [`ResolvedAa`],
/// the AA analogue of `resolve_ssao_policy`. It is the SINGLE owner of [`ResolvedAa`] (the
/// one-producer write discipline) and runs at the gather/setup boundary, scheduled BEFORE
/// the render point so the fresh selection feeds the SAME frame.
///
/// Cold by construction (zero hot-path cost): a single branchless map run once per frame; the
/// per-pixel AA cost lives entirely in the pre-compiled post-process shader, never here.
//
// `clippy::needless_pass_by_value`: `Res`/`ResMut` are by-value `SystemParam`s read/written
// through reborrows — the same false-positive `resolve_ssao_policy` carries.
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_aa_policy(cfg: Res<AaConfig>, mut resolved: ResMut<ResolvedAa>) {
    *resolved = resolve_aa(&cfg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_off_the_zero_gate() {
        let cfg = AaConfig::default();
        assert_eq!(cfg.mode, AaMode::Off);
        assert!(!cfg.enabled(), "the default config is the 0%-gate (no AA pass)");
    }

    #[test]
    fn enabled_is_structural_mode_not_off() {
        // Capability is structural: every non-Off mode is enabled.
        assert!(AaConfig { mode: AaMode::Fxaa }.enabled(), "Fxaa must be enabled (mode != Off)");
        assert!(AaConfig { mode: AaMode::Smaa }.enabled(), "Smaa must be enabled (mode != Off)");
        assert!(AaConfig { mode: AaMode::Ssaa }.enabled(), "Ssaa must be enabled (mode != Off)");
        assert!(AaConfig { mode: AaMode::Taa }.enabled(), "Taa must be enabled (mode != Off)");
        assert!(!AaConfig { mode: AaMode::Off }.enabled(), "Off is the disabled state");
    }

    #[test]
    fn mode_word_is_the_repr_discriminant() {
        // The backend forwards the `#[repr(u32)]` discriminant as a stable mode word.
        assert_eq!(AaMode::Off.as_word(), 0);
        assert_eq!(AaMode::Fxaa.as_word(), 1);
        assert_eq!(AaMode::Smaa.as_word(), 2);
        assert_eq!(AaMode::Ssaa.as_word(), 3);
        assert_eq!(AaMode::Taa.as_word(), 4);
    }

    #[test]
    fn resolve_forwards_the_mode() {
        assert_eq!(resolve_aa(&AaConfig { mode: AaMode::Off }), ResolvedAa { mode: AaMode::Off });
        assert_eq!(resolve_aa(&AaConfig { mode: AaMode::Fxaa }), ResolvedAa { mode: AaMode::Fxaa });
        assert_eq!(resolve_aa(&AaConfig { mode: AaMode::Smaa }), ResolvedAa { mode: AaMode::Smaa });
        assert_eq!(resolve_aa(&AaConfig { mode: AaMode::Ssaa }), ResolvedAa { mode: AaMode::Ssaa });
        assert_eq!(resolve_aa(&AaConfig { mode: AaMode::Taa }), ResolvedAa { mode: AaMode::Taa });
    }

    #[test]
    fn default_resolved_matches_resolving_the_default_config() {
        // The `ResolvedAa::default` shortcut must equal resolving a default `AaConfig`, so a
        // never-run policy already carries the no-pass selection.
        assert_eq!(ResolvedAa::default(), resolve_aa(&AaConfig::default()));
        assert!(!ResolvedAa::default().enabled());
    }
}
