//! HW-RT Rung 3a Step 1 — the ECS-native shadow-denoise config the author sets, plus the
//! cold resolve policy that packs its live-tunable edge-stop scalars into a std140 UBO.
//!
//! Principle 0: ECS-native — [`ShadowDenoiseConfig`] is the author-set `#[derive(Resource)]`
//! singleton (the cold config, NOT a side `std::Vec`/`HashMap`) and [`ResolvedShadowDenoise`]
//! is its derived companion Resource written by the cold [`resolve_shadow_denoise_policy`].
//! This mirrors the ray-shadow substrate exactly:
//! [`RayShadowConfig`](crate::ray_shadow_config::RayShadowConfig) (the author config) +
//! [`ResolvedRayShadow`](crate::ray_shadow_config::ResolvedRayShadow) (the derived UBO) +
//! [`resolve_ray_shadow_system`](crate::ray_shadow_config::resolve_ray_shadow_system) (the
//! cold single-writer), and the SSAO substrate
//! ([`SsaoConfig`](crate::ssao_config::SsaoConfig) +
//! [`resolve_ssao_policy`](crate::ssao_config::resolve_ssao_policy)).
//!
//! # Capability is structural (no redundant `enabled: bool`)
//!
//! Whether the denoise pass runs is keyed off the [`ShadowDenoiseMode`] enum, NOT a separate
//! flag — [`ShadowDenoiseMode::None`] IS "disabled". This is the capability-is-structural
//! principle and mirrors how [`SsaoConfig`](crate::ssao_config::SsaoConfig) keys off
//! [`SsaoQuality`](crate::ssao_config::SsaoQuality) rather than a `bool`.
//! [`ShadowDenoiseConfig::enabled`] is a derived predicate (`mode == Spatial`), not stored
//! state.
//!
//! # The 0%-gate (byte-identical default)
//!
//! [`ShadowDenoiseConfig::default`] is [`ShadowDenoiseMode::None`] — the resolve traces the
//! Vogel cone inline (byte-identical to today, no a-trous pass). This is Step 1 of the Rung
//! 3a track: a PURE config module + plugin, no render/shader/framegraph change. The a-trous
//! filter that READS [`ResolvedShadowDenoise`] lands in the later steps.

use boyko_macros::Resource;

use boyko_ecs::ecs::core::system::{Res, ResMut};

// ---- ShadowDenoiseMode (the author-set knob; capability is structural) ----------------

/// Spatial denoise mode for the RT soft mesh-shadow visibility. Capability-is-structural:
/// `None` (default) = no denoise pass, the resolve traces inline (byte-identical to today).
///
/// `None` is the structural "disabled" state (the capability-is-structural principle): the
/// resolve gates the whole denoise pass on `mode != None`, so there is NO redundant
/// `enabled: bool` — exactly as [`SsaoQuality`](crate::ssao_config::SsaoQuality) keys the
/// SSAO pass off `Off` rather than a flag.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ShadowDenoiseMode {
    /// No denoise — inline Vogel trace in the resolve (0%-gate, byte-identical default). The
    /// DEFAULT, so a world that never inserts a non-default [`ShadowDenoiseConfig`] renders
    /// byte-identically to today.
    #[default]
    None,
    /// Single-frame edge-avoiding a-trous spatial filter over the traced visibility.
    Spatial,
}

// ---- constants ------------------------------------------------------------------------

/// Max a-trous iterations (bounds the ping-pong dispatch count). The spatial reach grows as
/// `2^levels`, so this caps both the dispatch count and the worst-case filter footprint.
pub const MAX_ATROUS_LEVELS: u32 = 5;

// ---- ShadowDenoiseConfig (the author-set Resource — mirrors RayShadowConfig) ----------

/// Author-facing shadow-denoise tuning. `Copy`; `Default` = `None` (byte-identical 0%-gate).
///
/// `#[derive(Resource)]` via [`boyko_macros::Resource`] (the same derive path
/// [`RayShadowConfig`](crate::ray_shadow_config::RayShadowConfig) uses). Enablement is
/// structural (`mode != None`), so there is no separate flag.
#[derive(Resource, Clone, Copy, Debug)]
pub struct ShadowDenoiseConfig {
    /// `None` (default) => inline trace, byte-identical. `Spatial` => a-trous filter path.
    pub mode: ShadowDenoiseMode,
    /// A-trous dispatch count (spatial reach ~ 2^levels). ON-default 3; clamped
    /// `1..=MAX_ATROUS_LEVELS` by [`clamped_levels`](ShadowDenoiseConfig::clamped_levels).
    pub levels: u32,
    /// Depth (linear view-Z) edge-stop sigma. ON-default 1.0.
    pub sigma_z: f32,
    /// Normal edge-stop exponent. ON-default 128.0.
    pub sigma_n: f32,
}

impl Default for ShadowDenoiseConfig {
    /// `None` (the 0%-gate anchor): a default world runs no denoise pass and is
    /// byte-identical to today. The ON-defaults (`levels 3`, `sigma_z 1.0`, `sigma_n 128.0`)
    /// are carried so a bare `mode` flip to `Spatial` is a sensible starting tune.
    #[inline]
    fn default() -> Self {
        Self { mode: ShadowDenoiseMode::None, levels: 3, sigma_z: 1.0, sigma_n: 128.0 }
    }
}

impl ShadowDenoiseConfig {
    /// `true` iff the spatial denoise path is active (whole-pass capability gate) — the
    /// structural predicate `mode == Spatial` (NOT stored state). Mirrors reading the mode
    /// in [`SsaoConfig::enabled`](crate::ssao_config::SsaoConfig::enabled) rather than a
    /// redundant `bool`.
    #[inline]
    pub const fn enabled(&self) -> bool {
        matches!(self.mode, ShadowDenoiseMode::Spatial)
    }

    /// The a-trous level count clamped to the valid range (never 0, never > MAX). Bounds the
    /// ping-pong dispatch count regardless of the author-set [`levels`](ShadowDenoiseConfig::levels).
    #[inline]
    pub fn clamped_levels(&self) -> u32 {
        self.levels.clamp(1, MAX_ATROUS_LEVELS)
    }
}

// ---- ResolvedShadowDenoise (the derived UBO mirror) -----------------------------------

/// The packed UBO the a-trous filter reads: std140 vec4, 16 B. Edge-stop weight scalars
/// (no loop-bound impact => UBO, live-tunable per the Rung-1 perf split — the loop bound
/// `levels` drives the dispatch count on the host, NOT this UBO).
///
/// `#[repr(C)]` for a stable GPU-ready layout — the field ORDER + TYPES byte-mirror the
/// a-trous cbuffer (sigma_z @0, sigma_n @4, pad @8/@12). `#[derive(Resource)]` (the same
/// derive path [`ResolvedRayShadow`](crate::ray_shadow_config::ResolvedRayShadow) uses) so
/// the plugin inserts it as a `World` singleton and the cold policy writes it via `ResMut`.
#[repr(C)]
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ResolvedShadowDenoise {
    /// Depth (linear view-Z) edge-stop sigma. Offset 0.
    pub sigma_z: f32,
    /// Normal edge-stop exponent. Offset 4.
    pub sigma_n: f32,
    /// std140 vec4 tail padding. Offset 8.
    pub _pad0: f32,
    /// std140 vec4 tail padding. Offset 12.
    pub _pad1: f32,
}

// Layout pin: 4 × 4 = 16 B = one std140 vec4 slot. A change is a deliberate decision (the
// a-trous filter's cbuffer reads this stride).
const _: () = assert!(core::mem::size_of::<ResolvedShadowDenoise>() == 16);

/// The byte size of the host-coherent a-trous edge-stop UBO — `size_of::<ResolvedShadowDenoise>()`
/// (16 B). Hosts size their UBO slots from THIS constant (single source — no hand-copied `16`).
/// Mirrors [`RESOLVED_RAY_SHADOW_BYTES`](crate::ray_shadow_config::RESOLVED_RAY_SHADOW_BYTES).
pub const RESOLVED_SHADOW_DENOISE_BYTES: usize = core::mem::size_of::<ResolvedShadowDenoise>();

impl Default for ResolvedShadowDenoise {
    /// The resolve of the default [`ShadowDenoiseConfig`] — so a never-run policy (frame 0)
    /// already carries the correct edge-stop scalars.
    #[inline]
    fn default() -> Self {
        resolve_shadow_denoise(&ShadowDenoiseConfig::default())
    }
}

// ---- the resolve decision (pure — the unit-testable policy) ----------------------------

/// Pure cold policy: config -> the packed edge-stop UBO. The PURE, unit-testable resolve (the
/// analogue of [`resolve_ray_shadow`](crate::ray_shadow_config::resolve_ray_shadow), the core
/// the cold system wraps). Carries only the runtime UBO scalars; the loop-bound `levels`
/// drives the host dispatch count and is NOT packed here. No allocation, no `World` access.
#[inline]
pub fn resolve_shadow_denoise(cfg: &ShadowDenoiseConfig) -> ResolvedShadowDenoise {
    ResolvedShadowDenoise { sigma_z: cfg.sigma_z, sigma_n: cfg.sigma_n, _pad0: 0.0, _pad1: 0.0 }
}

// ---- the cold single-writer system ----------------------------------------------------

/// Single writer of [`ResolvedShadowDenoise`] (cold, once/frame), mirrors
/// [`resolve_ray_shadow_system`](crate::ray_shadow_config::resolve_ray_shadow_system) /
/// [`resolve_ssao_policy`](crate::ssao_config::resolve_ssao_policy). Reads the author
/// [`ShadowDenoiseConfig`] and writes the derived UBO carrier (the one-producer-per-field
/// write discipline).
//
// `clippy::needless_pass_by_value`: `Res`/`ResMut` are by-value `SystemParam`s read/written
// through reborrows — the same false-positive `resolve_ray_shadow_system` carries.
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_shadow_denoise_policy(
    cfg: Res<ShadowDenoiseConfig>,
    mut out: ResMut<ResolvedShadowDenoise>,
) {
    *out = resolve_shadow_denoise(&cfg);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ShadowDenoiseConfig::default()` is the 0%-gate (`None`) with the ON-default tune.
    #[test]
    fn default_config_is_none_the_zero_gate() {
        let cfg = ShadowDenoiseConfig::default();
        assert_eq!(cfg.mode, ShadowDenoiseMode::None);
        assert_eq!(cfg.levels, 3);
        assert_eq!(cfg.sigma_z, 1.0);
        assert_eq!(cfg.sigma_n, 128.0);
        assert!(!cfg.enabled(), "the default config is the 0%-gate (no denoise pass)");
    }

    /// `ShadowDenoiseMode::default()` is `None` (the structural disabled state).
    #[test]
    fn mode_default_is_none() {
        assert_eq!(ShadowDenoiseMode::default(), ShadowDenoiseMode::None);
    }

    /// The resolve of the default packs the ON-default edge-stop scalars (padding zeroed).
    #[test]
    fn resolve_of_default_is_the_edge_stop_scalars() {
        let r = resolve_shadow_denoise(&ShadowDenoiseConfig::default());
        assert_eq!(
            r,
            ResolvedShadowDenoise { sigma_z: 1.0, sigma_n: 128.0, _pad0: 0.0, _pad1: 0.0 }
        );
        // The `Default` impl equals the resolve of the default config (the frame-0 seed).
        assert_eq!(r, ResolvedShadowDenoise::default());
    }

    /// The UBO layout pin (16 B — one std140 vec4 slot).
    #[test]
    fn resolved_shadow_denoise_is_16_bytes() {
        assert_eq!(core::mem::size_of::<ResolvedShadowDenoise>(), 16);
        assert_eq!(RESOLVED_SHADOW_DENOISE_BYTES, 16);
    }

    /// `clamped_levels` never yields 0 and never exceeds `MAX_ATROUS_LEVELS`.
    #[test]
    fn clamped_levels_bounds_the_dispatch_count() {
        let clamped = |levels: u32| ShadowDenoiseConfig { levels, ..Default::default() }.clamped_levels();
        assert_eq!(clamped(0), 1, "0 clamps up to 1 (never an empty ping-pong)");
        assert_eq!(clamped(99), MAX_ATROUS_LEVELS, "99 clamps down to MAX");
        assert_eq!(clamped(3), 3, "an in-range level passes through");
    }

    /// `enabled` is the structural predicate `mode == Spatial`.
    #[test]
    fn enabled_is_structural_mode_spatial() {
        assert!(ShadowDenoiseConfig { mode: ShadowDenoiseMode::Spatial, ..Default::default() }.enabled());
        assert!(!ShadowDenoiseConfig { mode: ShadowDenoiseMode::None, ..Default::default() }.enabled());
    }
}
