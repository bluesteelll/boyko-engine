//! VG R3 piece 1 step P1-1 — the ECS-native ARMING config for the hierarchical-Z depth
//! pyramid.
//!
//! Principle 0: ECS-native — [`HzbConfig`] is a `#[derive(Resource)]` singleton (the cold
//! owner-set config, NOT a side `std::Vec`/`HashMap`), mirroring
//! [`AaConfig`](crate::aa_config::AaConfig) / [`SsaoConfig`](crate::ssao_config::SsaoConfig)
//! exactly — one enum knob, a structural predicate, no stored flag.
//!
//! # Read by nothing, deliberately
//!
//! In piece 1 the pyramid is built and read by NOTHING (see
//! `docs/VG-R3-P1-PYRAMID-PLAN.md` §1). This step lands only the knob, ahead of the image,
//! the pipelines and the build passes, so each later step is a small diff against a config
//! surface that is already reviewed and already defaulted to the 0%-gate.
//!
//! # Capability is structural (no redundant `enabled: bool`)
//!
//! Whether the engine maintains a pyramid is keyed off the [`HzbMode`] enum, NOT a separate
//! flag — [`HzbMode::Off`] IS "disabled". [`HzbConfig::enabled`] is a derived predicate
//! (`mode != Off`), not stored state, exactly as
//! [`AaConfig::enabled`](crate::aa_config::AaConfig::enabled) and
//! [`SsaoConfig::enabled`](crate::ssao_config::SsaoConfig::enabled) are.
//!
//! # The 0%-gate
//!
//! [`HzbConfig::default`] is [`HzbMode::Off`] — byte-identical to today: no image, no views,
//! no pipelines, no descriptor sets, no passes, and zero barriers on a ResId that is still
//! declared. A world that never inserts a non-default [`HzbConfig`] renders exactly what it
//! renders now.
//!
//! # Why there is no `ResolvedHzb` derived carrier
//!
//! [`SsaoConfig`](crate::ssao_config::SsaoConfig) needs
//! [`ResolvedSsao`](crate::ssao_config::ResolvedSsao) because quality → variant-index is a
//! REAL map (three `.spv` variants, a header mode word, an à-trous pass count) that must be
//! derived once and read consistently. Here the map would be the identity: the only thing
//! downstream needs is "pyramid, or no pyramid". A derived carrier restating one bit would be
//! a second source of truth for that bit and nothing else, so the render driver reads
//! [`HzbConfig::enabled`] directly and there is no policy system to schedule (hence
//! [`HzbPlugin`](crate::hzb_plugin::HzbPlugin) registers none).
//!
//! # Why this config does NOT join `RenderPathFrozenConsumers`
//!
//! [`RenderPathFrozenConsumers`](crate::render_path_config::RenderPathFrozenConsumers) exists
//! because a runtime SSAO/DDGI flip can make the light header tell the shade to combine a term
//! whose pass was never armed — a live config drifting away from the boot-shaped framegraph.
//! The pyramid cannot drift that way: it is read by nothing, and its arming is captured at
//! `GBufferTargets::create` time ONTO THE TARGETS (the `AaArm::from_scene` /
//! `TargetsProfile::from_scene` precedent), so the recorder keys off the targets it was handed,
//! never off the live config. A frozen-consumer field would be a snapshot of a value no
//! recorder reads.
//!
//! # Two variants, permanently
//!
//! [`HzbMode`] answers "does the engine maintain a depth pyramid", not "how good is it". The
//! occlusion CONSUMER (the cull's late pass, the per-instance verdict) is a separate config in
//! pieces 3/4 of the decomposition — not a third variant here. Keeping the producer knob and
//! the consumer knob distinct is what lets the pyramid be armed and gated against
//! [`crate::hzb`]'s host oracle while nothing consumes it.

use boyko_macros::Resource;

// ---- HzbMode (the owner-set knob; capability is structural) --------------------------

/// Whether the engine maintains a hierarchical-Z depth pyramid. `#[repr(u32)]` so it can be
/// forwarded to the backend / a dump manifest as a stable arm word.
///
/// [`Off`](HzbMode::Off) is the structural "disabled" state (the capability-is-structural
/// principle): the render driver gates the whole build chain on `mode != Off`, so there is NO
/// redundant `enabled: bool` — exactly as [`AaMode`](crate::aa_config::AaMode) and
/// [`SsaoQuality`](crate::ssao_config::SsaoQuality) key off their `Off` variants.
///
/// Exactly two variants, and that is permanent: this enum is the PRODUCER knob. A quality
/// dimension would be a lie (the pyramid is a `min` reduce — there is no cheaper or better
/// version of it, only "present" or "absent"), and the occlusion consumer is a separate later
/// config, not a third variant.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum HzbMode {
    /// No pyramid — the 0%-gate: no image, no per-mip views, no descriptor sets, no build
    /// passes, zero barriers, and not one recorded command. The DEFAULT, so a world that never
    /// inserts a non-default [`HzbConfig`] is byte-identical to today.
    ///
    /// The ONE thing this arm does not suppress (VG R3 piece 1 step P1-4) is the `hzb_build`
    /// bind-group LAYOUT and PIPELINE: the backend mints those unconditionally at boot, so that
    /// the pyramid's arming is a SINGLE predicate living on the targets (`HzbTargets` exists iff
    /// the plan does) rather than a second one here that could disagree with it. A boot-time
    /// `VkDescriptorSetLayout` + `VkPipeline` dispatched by nothing changes no rendered pixel.
    #[default]
    Off,
    /// Build the pyramid every frame from the depth attachment (`R32_SFLOAT`, a real Vulkan
    /// mip chain, level 0 at `prev_pow2` of each source axis, the reverse-Z `min` reduce —
    /// [`crate::hzb`] is the host oracle the built result is gated against). In piece 1 the
    /// built pyramid is read by nothing; it costs the build dispatches and its VRAM, and
    /// changes no rendered pixel.
    Build,
}

// ---- HzbConfig (the owner-set Resource — mirrors AaConfig) ---------------------------

/// The global depth-pyramid arming config — a `World`-singleton Resource the owner sets, the
/// HZB analogue of [`AaConfig`](crate::aa_config::AaConfig). Carries ONLY the [`HzbMode`]
/// knob: enablement is structural (`mode != Off`), so there is no separate flag.
///
/// `#[derive(Resource)]` via [`boyko_macros::Resource`] (the same derive path
/// `AaConfig`/`SsaoConfig` use). There is no derived companion Resource — see the module doc
/// for why a `ResolvedHzb` would restate one bit.
#[derive(Resource, Clone, Copy, Debug)]
pub struct HzbConfig {
    /// The owner-set pyramid arming. [`Off`](HzbMode::Off) (the default) ⇒ nothing is built.
    pub mode: HzbMode,
}

impl Default for HzbConfig {
    #[inline]
    fn default() -> Self {
        // Off == today (the 0%-gate anchor): a default world builds no pyramid.
        Self { mode: HzbMode::Off }
    }
}

impl HzbConfig {
    /// Whether the pyramid is built — the structural predicate `mode != Off` (NOT stored
    /// state). True ⇒ the image, its per-mip views and the build passes exist; false ⇒ the
    /// 0%-gate. The render driver reads THIS rather than a derived carrier (see the module
    /// doc).
    #[inline]
    pub const fn enabled(&self) -> bool {
        !matches!(self.mode, HzbMode::Off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_off_the_zero_gate() {
        let cfg = HzbConfig::default();
        assert_eq!(cfg.mode, HzbMode::Off);
        assert!(!cfg.enabled(), "the default config is the 0%-gate (no pyramid is built)");
    }

    #[test]
    fn default_mode_is_off() {
        // There are TWO independent routes to `Off`, and this pins the one the test above does
        // not: `HzbConfig::default` is the hand-written impl, which names `HzbMode::Off`
        // literally and never calls `HzbMode::default()`, while `HzbMode::default()` comes from
        // the `#[default]` attribute. Neither derives from the other, so neither test implies
        // the other. `HzbMode::default()` has no production caller today; it is pinned so that
        // the day one appears — or the day `HzbConfig` switches to `#[derive(Default)]` and the
        // attribute becomes load-bearing — it is already anchored to the 0%-gate.
        assert_eq!(HzbMode::default(), HzbMode::Off);
    }

    #[test]
    fn enabled_agrees_with_the_discriminant_on_every_variant() {
        // Capability is structural: `enabled()` must be exactly "the discriminant is not
        // Off's", checked against the `#[repr(u32)]` word rather than against a restatement
        // of the same `matches!` the impl uses.
        for mode in [HzbMode::Off, HzbMode::Build] {
            let expected = mode as u32 != HzbMode::Off as u32;
            assert_eq!(
                HzbConfig { mode }.enabled(),
                expected,
                "{mode:?}: enabled() must track the discriminant"
            );
        }
    }
}
