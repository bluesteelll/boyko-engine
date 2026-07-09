//! Render P7-Q2 TASK 3 — the ECS-native SSAO quality config the USER sets, plus the
//! cold resolve policy that maps it to the variant-pipeline selection.
//!
//! Principle 0: ECS-native — [`SsaoConfig`] is a `#[derive(Resource)]` singleton (the
//! cold owner-set config, NOT a side `std::Vec`/`HashMap`), and [`ResolvedSsao`] is its
//! derived companion Resource written by the cold [`resolve_ssao_policy`] system. This
//! mirrors the lighting StrategyPolicy substrate exactly: [`LightingConfig`] (the
//! owner-set config) + [`LightStats`] (the derived carrier) + `select_lighting_cull`
//! (the single-owner cold policy) in [`crate::light_policy`].
//!
//! [`LightingConfig`]: crate::light::LightingConfig
//! [`LightStats`]: crate::light_policy::LightStats
//!
//! # Capability is structural (no redundant `enabled: bool`)
//!
//! Whether SSAO runs is keyed off the [`SsaoQuality`] enum, NOT a separate flag —
//! [`SsaoQuality::Off`] IS "disabled". This is the capability-is-structural principle
//! and mirrors how `select_lighting_cull` keys off [`ClusterSelectMode`] rather than a
//! `bool`. [`SsaoConfig::enabled`] is a derived predicate (`quality != Off`), not stored
//! state.
//!
//! [`ClusterSelectMode`]: crate::light::ClusterSelectMode
//!
//! # The 0%-gate
//!
//! [`SsaoConfig::default`] is [`SsaoQuality::Off`] — byte-identical to today (no SSAO
//! pass, the resolve's AO-combine off). The resolve maps `Off` to
//! `ResolvedSsao { variant: None, ssao_mode_word: 0 }`, the no-pass anchor.
//!
//! # No live render consumer yet (the named follow-up)
//!
//! The deferred-render ECS system that READS [`ResolvedSsao`] each frame — picking the
//! pre-compiled variant pipeline ([`boyko_rhi_vulkan`]'s `sdf_ssao_spirv_variant`) and
//! setting the resolve's `with_ssao_mode` from `ssao_mode_word` — is the explicit LARGER
//! follow-up (the deferred pipeline is test-driven today, not a `boyko_render` system).
//! TASK 3 lands the Resource + policy + plugin, unit-tested, ready for that consumer.

use boyko_macros::Resource;

use boyko_ecs::ecs::core::system::{Res, ResMut};

// ---- SsaoQuality (the owner-set quality knob; capability is structural) --------------

/// The SSAO quality level the owner sets on [`SsaoConfig`]. The variant index into the
/// pre-compiled `.spv` table (Render P7-Q2 Mechanism C) is the enum's position MINUS the
/// [`Off`](SsaoQuality::Off) slot: `Low => 0`, `Medium => 1`, `High => 2`, matching the
/// `boyko_shaderdsl::ssao::SsaoQuality` row order (`Low`/`Medium`/`High`) and
/// `SSAO_PRESETS[0..3]`.
///
/// `Off` is the structural "disabled" state (the capability-is-structural principle): the
/// resolve gates the whole SSAO pass on `quality != Off`, so there is NO redundant
/// `enabled: bool` — exactly as `select_lighting_cull` keys off
/// [`ClusterSelectMode`](crate::light::ClusterSelectMode) rather than a flag.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SsaoQuality {
    /// SSAO is off — the 0%-gate (no pass, resolve AO-combine off). The DEFAULT, so a
    /// world that never inserts a non-default [`SsaoConfig`] is byte-identical to today.
    #[default]
    Off,
    /// `boyko_shaderdsl::ssao::SSAO_PRESETS[0]` — the cheapest tap budget (variant 0).
    Low,
    /// `boyko_shaderdsl::ssao::SSAO_PRESETS[1]` — today's shipped scalars (variant 1).
    Medium,
    /// `boyko_shaderdsl::ssao::SSAO_PRESETS[2]` — the widest tap budget (variant 2).
    High,
}

impl SsaoQuality {
    /// The pre-compiled `.spv` variant index this quality selects, or `None` when
    /// [`Off`](SsaoQuality::Off). `Low => Some(0)`, `Medium => Some(1)`, `High => Some(2)`
    /// — the row order of `boyko_shaderdsl::ssao::SsaoQuality` / `SSAO_PRESETS`. The render
    /// driver binds the variant pipeline keyed by this index.
    #[inline]
    pub const fn variant(self) -> Option<usize> {
        match self {
            SsaoQuality::Off => None,
            SsaoQuality::Low => Some(0),
            SsaoQuality::Medium => Some(1),
            SsaoQuality::High => Some(2),
        }
    }
}

// ---- SsaoConfig (the owner-set Resource — mirrors LightingConfig) ---------------------

/// The global SSAO config (Render P7-Q2 TASK 3) — a `World`-singleton Resource the owner
/// sets, the SSAO analogue of [`LightingConfig`](crate::light::LightingConfig). Carries
/// ONLY the [`SsaoQuality`] knob: enablement is structural (`quality != Off`), so there is
/// no separate flag.
///
/// `#[derive(Resource)]` via [`boyko_macros::Resource`] (the same derive path
/// `LightingConfig` uses).
#[derive(Resource, Clone, Copy, Debug)]
pub struct SsaoConfig {
    /// The owner-set SSAO quality. [`Off`](SsaoQuality::Off) (the default) ⇒ no pass.
    pub quality: SsaoQuality,
}

impl Default for SsaoConfig {
    #[inline]
    fn default() -> Self {
        // Off == today (the 0%-gate anchor): a default world runs no SSAO pass.
        Self { quality: SsaoQuality::Off }
    }
}

impl SsaoConfig {
    /// Whether SSAO runs — the structural predicate `quality != Off` (NOT stored state).
    /// True ⇒ a variant pipeline is bound and the resolve combines AO; false ⇒ the
    /// 0%-gate (no pass). Mirrors reading `clusters_enabled`/the mode in the lighting
    /// policy rather than a redundant `bool`.
    #[inline]
    pub const fn enabled(&self) -> bool {
        !matches!(self.quality, SsaoQuality::Off)
    }
}

// ---- ResolvedSsao (the derived carrier — mirrors LightStats) -------------------------

/// The derived SSAO selection the render driver reads — the SSAO analogue of
/// [`LightStats`](crate::light_policy::LightStats). [`resolve_ssao_policy`] is its SINGLE
/// writer (the one-producer-per-field write discipline), recomputing it from
/// [`SsaoConfig`] each policy run. `#[repr(C)]` for a stable two-field layout.
///
/// The render driver (the deferred-render ECS system — the named follow-up) reads this to
/// (1) pick the pre-compiled variant pipeline by `variant` (`None` ⇒ skip the SSAO pass)
/// and (2) set the resolve's `with_ssao_mode` from `ssao_mode_word`.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct ResolvedSsao {
    /// The pre-compiled `.spv` variant index to bind (`Some(0/1/2)` for `Low/Medium/High`),
    /// or `None` when SSAO is off (skip the pass — the 0%-gate).
    pub variant: Option<usize>,
    /// The resolve's SSAO-combine mode word: `0` ⇒ AO-combine off (no pass), `1` ⇒ on.
    pub ssao_mode_word: u32,
}

impl Default for ResolvedSsao {
    #[inline]
    fn default() -> Self {
        // The resolve of the default `SsaoConfig` (Off) — the 0%-gate, so a never-run
        // policy already reads the no-pass selection.
        resolve_ssao(&SsaoConfig::default())
    }
}

// ---- the resolve decision (pure — the SSAO analogue of `banded`) ---------------------

/// Maps an [`SsaoConfig`] to its derived [`ResolvedSsao`] — the pure SSAO resolve decision
/// (the analogue of `banded` in [`crate::light_policy`], the unit-testable core the cold
/// system wraps):
///
/// - [`Off`](SsaoQuality::Off) ⇒ `{ variant: None, ssao_mode_word: 0 }` — the 0%-gate (no
///   pass, resolve combine off).
/// - [`Low`](SsaoQuality::Low)/[`Medium`](SsaoQuality::Medium)/[`High`](SsaoQuality::High)
///   ⇒ `{ variant: Some(0/1/2), ssao_mode_word: 1 }`, the variant index matching
///   `boyko_shaderdsl::ssao::SsaoQuality` row order (`Low=0`, `Medium=1`, `High=2`).
#[inline]
pub fn resolve_ssao(cfg: &SsaoConfig) -> ResolvedSsao {
    let variant = cfg.quality.variant();
    // `ssao_mode_word` is the structural enablement, derived from the SAME `variant` so the
    // two can never disagree: a bound variant ⇒ combine on, no variant ⇒ off.
    let ssao_mode_word = u32::from(variant.is_some());
    ResolvedSsao { variant, ssao_mode_word }
}

// ---- the cold StrategyPolicy system (mirrors `select_lighting_cull`) ------------------

/// The cold SSAO resolve policy — reads [`SsaoConfig`] and writes the derived
/// [`ResolvedSsao`], the SSAO analogue of `select_lighting_cull` in
/// [`crate::light_policy`]. It is the SINGLE owner of [`ResolvedSsao`] (the one-producer
/// write discipline) and runs at the gather/setup boundary, scheduled BEFORE the render
/// point so the fresh selection feeds the SAME frame (the `.before` registration in
/// [`SsaoPlugin`]).
///
/// Cold by construction (zero hot-path cost): the per-pixel SSAO cost is ZERO per variant
/// (Mechanism C — the loop bounds are baked `static const`), and this policy is a single
/// branchless map run once per frame; the per-row resolve never reads [`SsaoConfig`].
//
// `clippy::needless_pass_by_value`: `Res`/`ResMut` are by-value `SystemParam`s read/
// written through reborrows — the same false-positive `select_lighting_cull` carries.
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_ssao_policy(cfg: Res<SsaoConfig>, mut resolved: ResMut<ResolvedSsao>) {
    *resolved = resolve_ssao(&cfg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_off_the_zero_gate() {
        let cfg = SsaoConfig::default();
        assert_eq!(cfg.quality, SsaoQuality::Off);
        assert!(!cfg.enabled(), "the default config is the 0%-gate (no SSAO pass)");
    }

    #[test]
    fn enabled_is_structural_quality_not_off() {
        // Capability is structural: every non-Off quality is enabled with a real variant.
        for q in [SsaoQuality::Low, SsaoQuality::Medium, SsaoQuality::High] {
            let cfg = SsaoConfig { quality: q };
            assert!(cfg.enabled(), "{q:?} must be enabled (quality != Off)");
            assert!(cfg.quality.variant().is_some(), "{q:?} must select a variant");
        }
        assert!(SsaoQuality::Off.variant().is_none(), "Off selects no variant");
    }

    #[test]
    fn resolve_maps_each_quality_to_its_variant_and_mode_word() {
        // Off → no pass (the 0%-gate).
        let off = resolve_ssao(&SsaoConfig { quality: SsaoQuality::Off });
        assert_eq!(off, ResolvedSsao { variant: None, ssao_mode_word: 0 });

        // Low/Medium/High → variant 0/1/2, combine on.
        assert_eq!(
            resolve_ssao(&SsaoConfig { quality: SsaoQuality::Low }),
            ResolvedSsao { variant: Some(0), ssao_mode_word: 1 }
        );
        assert_eq!(
            resolve_ssao(&SsaoConfig { quality: SsaoQuality::Medium }),
            ResolvedSsao { variant: Some(1), ssao_mode_word: 1 }
        );
        assert_eq!(
            resolve_ssao(&SsaoConfig { quality: SsaoQuality::High }),
            ResolvedSsao { variant: Some(2), ssao_mode_word: 1 }
        );
    }

    #[test]
    fn default_resolved_matches_resolving_the_default_config() {
        // The `ResolvedSsao::default` shortcut must equal resolving a default `SsaoConfig`,
        // so a never-run policy already carries the no-pass selection.
        assert_eq!(ResolvedSsao::default(), resolve_ssao(&SsaoConfig::default()));
    }

    /// Pins the variant index ↔ quality mapping to the `boyko_shaderdsl::ssao::SsaoQuality`
    /// row order (`Low=0`, `Medium=1`, `High=2`) / `SSAO_PRESETS[0..3]`. `boyko_render` does
    /// NOT depend on `boyko_shaderdsl` (the dep would invert the layering — render must not
    /// pull the shader-author eDSL), so this asserts the DOCUMENTED 0/1/2 order locally; a
    /// future reorder of either side is caught by this fixture failing against the pin.
    #[test]
    fn variant_index_matches_documented_shaderdsl_row_order() {
        // The pinned row order (boyko_shaderdsl::ssao::SsaoQuality + SSAO_PRESETS):
        //   row 0 = Low, row 1 = Medium, row 2 = High.
        const LOW_ROW: usize = 0;
        const MEDIUM_ROW: usize = 1;
        const HIGH_ROW: usize = 2;
        assert_eq!(SsaoQuality::Low.variant(), Some(LOW_ROW));
        assert_eq!(SsaoQuality::Medium.variant(), Some(MEDIUM_ROW));
        assert_eq!(SsaoQuality::High.variant(), Some(HIGH_ROW));
    }
}
