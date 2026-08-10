//! VG R3 piece 4 rung P4-4 — the ECS-native **CONSUMER** knob for the two-phase HZB occlusion
//! decision.
//!
//! Principle 0: ECS-native — [`OcclusionConfig`] is a `#[derive(Resource)]` singleton (the cold
//! owner-set config, NOT a side `std::Vec`/`HashMap`), mirroring
//! [`HzbConfig`](crate::hzb_config::HzbConfig) exactly: one enum knob, a structural predicate, no
//! stored flag.
//!
//! # Two knobs, two audiences
//!
//! [`HzbMode`](crate::hzb_config::HzbMode) answers *"does the engine maintain a depth pyramid"* —
//! the PRODUCER. This one answers *"does the owner want the occlusion decision"* — the CONSUMER.
//! They are separate types because they are separate questions, and because piece 4's host
//! disjunct lets EITHER of them ask for the pyramid
//! (`boyko_app::hzb_plan::hzb_plan_for`): a `TwoPhase` consumer on an `Off` producer still gets a
//! pyramid, because the alternative is a silently-dead knob.
//!
//! # Capability is structural (no redundant `enabled: bool`)
//!
//! Whether the engine performs the decision is keyed off the [`OcclusionMode`] enum, NOT a
//! separate flag — [`OcclusionMode::Off`] IS "disabled". [`OcclusionConfig::enabled`] is a derived
//! predicate (`mode != Off`), not stored state, exactly as
//! [`HzbConfig::enabled`](crate::hzb_config::HzbConfig::enabled) is.
//!
//! # The 0%-gate, and why the default is `Off`
//!
//! [`OcclusionConfig::default`] is [`OcclusionMode::Off`] — byte-identical to a world that never
//! inserts the Resource: `GBufferScene::path_vb_occlusion_split()` is false through its FIRST
//! conjunct, no late pass is declared or recorded, no marked instance is ever tested. In order of
//! weight:
//!
//! 1. **Error asymmetry.** The split's failure mode is DELETED GEOMETRY; its upside is bounded by
//!    the early raster's share of a frame. [`OcclusionCulling`](crate::occlusion_marker) itself is
//!    opt-in for that reason.
//! 2. **On this corpus the benefit is provably zero and the cost is not.** On a converged static
//!    scene the late scope correctly draws nothing (the campaign's plan D12 fixed point), so a
//!    default that costs on every static scene and pays on none is not a default.
//! 3. It is the 0%-gate every sibling config anchors on, so composing
//!    [`OcclusionPlugin`](crate::occlusion_plugin::OcclusionPlugin) unconditionally leaves every
//!    committed golden pin byte-identical BY CONSTRUCTION rather than by measurement.
//!
//! # ⚠️ `Off` is not an allocation-backed disarm
//!
//! It suppresses the DECISION, the late passes and the second/third descriptor-set *binding*. The
//! late buffers, `hzb_null`, the widened cull layout and the sets are still minted on every
//! VisibilityBuffer boot — that is a boot-time allocation question, tracked separately, and this
//! knob does not claim to close it.

use boyko_macros::Resource;

// ---- OcclusionMode (the owner-set knob; capability is structural) --------------------

/// Whether the engine performs the two-phase HZB occlusion decision on instances carrying
/// [`OcclusionCulling`](crate::occlusion_marker::OcclusionCulling). `#[repr(u32)]` so the
/// discriminant is a stable arm word (the [`HzbMode`](crate::hzb_config::HzbMode) rule).
///
/// Exactly two variants, and that is permanent, for [`HzbMode`](crate::hzb_config::HzbMode)'s own
/// reason: this is the CONSUMER knob. There is no quality dimension — the decision is ONE
/// conservative min-over-footprint predicate, so there is no cheaper or better version of it, only
/// "performed" or "not" — and the diagnostic verdict overrides (defer nothing / defer everything)
/// are a different AXIS living in `boyko_app::OcclusionForce`, not a third variant here. A verdict
/// override is not an answer to "does the owner want the decision".
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OcclusionMode {
    /// No decision — the 0%-gate and the DEFAULT: `path_vb_occlusion_split()` is false, no late
    /// passes are declared or recorded, no marked instance is ever tested, and the frame is
    /// byte-identical to a world that never inserts this Resource.
    ///
    /// ⚠️ `Off` means **do not TEST**, never *do not GATHER*: the per-frame marked-instance count
    /// stays the gather's own number, so the counter means one thing regardless of this knob. The
    /// cost of a marked-but-disarmed world is one `u32` read.
    #[default]
    Off,
    /// The shipping decision: the early phase tests every marked instance against the PREVIOUS
    /// frame's pyramid and defers the rejected ones; the late phase re-tests them after the
    /// pyramid has been rebuilt from the early scope's depth.
    ///
    /// Needs a pyramid, and gets one either from
    /// [`HzbMode::Build`](crate::hzb_config::HzbMode::Build) or — since piece 4 — from this
    /// variant alone, through the host's plan disjunct.
    TwoPhase,
}

impl OcclusionMode {
    /// The ARTIFACT spelling — `"off"` / `"two_phase"`.
    ///
    /// One table, because two artifacts already print this mode and a second spelling would be a
    /// second text that can disagree with the first: the `BOYKO_VB_PROBE` dump's `[host]` table,
    /// and the retired `BOYKO_VB_BENCH` summary's `VB-P4 regime … mode=[…]` line (profiling rung 7
    /// deleted that printer; the artifact header carries the same census). Both exist because rung
    /// P4-4 made the arming a LIVE Resource, so "which configuration produced this capture?" has
    /// to be answerable from the artifact rather than from a knob that was read once at boot.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            OcclusionMode::Off => "off",
            OcclusionMode::TwoPhase => "two_phase",
        }
    }

    /// Every mode, for exhaustive iteration in tests and in artifact readers. The array's LENGTH
    /// is the arity, so a variant left out of it fails the tests below rather than going untested.
    pub const ALL: [OcclusionMode; 2] = [OcclusionMode::Off, OcclusionMode::TwoPhase];
}

// ---- OcclusionConfig (the owner-set Resource — mirrors HzbConfig) --------------------

/// The global occlusion-decision arming config — a `World`-singleton Resource the owner sets, the
/// consumer-side analogue of [`HzbConfig`](crate::hzb_config::HzbConfig). Carries ONLY the
/// [`OcclusionMode`] knob: enablement is structural (`mode != Off`), so there is no separate flag.
///
/// Read **live, per frame**, beside `HzbConfig` — it does not join
/// [`RenderPathFrozenConsumers`](crate::render_path_config::RenderPathFrozenConsumers). That
/// carrier exists because a live SSAO/DDGI flip can make the light header ask the shade to combine
/// a term whose pass was never armed. The occlusion split has no header term: its arming is ONE
/// predicate that the declarator, the recorder and the shader all read from ONE folded word,
/// computed after the flip, in the same frame, from the same scene.
#[derive(Resource, Clone, Copy, Debug)]
pub struct OcclusionConfig {
    /// The owner-set decision arming. [`Off`](OcclusionMode::Off) (the default) ⇒ no split.
    pub mode: OcclusionMode,
}

impl Default for OcclusionConfig {
    #[inline]
    fn default() -> Self {
        // Off == today (the 0%-gate anchor): a default world performs no occlusion decision.
        Self { mode: OcclusionMode::Off }
    }
}

impl OcclusionConfig {
    /// Whether the two-phase decision runs — the structural predicate `mode != Off` (NOT stored
    /// state). True ⇒ the split's conjuncts are evaluated and, when they hold, the late passes are
    /// declared and recorded; false ⇒ the 0%-gate.
    #[inline]
    pub const fn enabled(&self) -> bool {
        !matches!(self.mode, OcclusionMode::Off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_off_the_zero_gate() {
        let cfg = OcclusionConfig::default();
        assert_eq!(cfg.mode, OcclusionMode::Off);
        assert!(!cfg.enabled(), "the default config is the 0%-gate (no occlusion decision)");
    }

    #[test]
    fn default_mode_is_off() {
        // The SECOND route into `Off`, pinned for `HzbConfig`'s own reason: `OcclusionConfig::
        // default` is a hand-written impl naming `OcclusionMode::Off` literally, while
        // `OcclusionMode::default()` comes from the `#[default]` attribute. Neither derives from
        // the other, so neither test implies the other.
        assert_eq!(OcclusionMode::default(), OcclusionMode::Off);
    }

    #[test]
    fn enabled_agrees_with_the_discriminant_on_every_variant() {
        // Capability is structural: `enabled()` must be exactly "the discriminant is not Off's",
        // checked against the `#[repr(u32)]` word rather than against a restatement of the same
        // `matches!` the impl uses.
        for mode in [OcclusionMode::Off, OcclusionMode::TwoPhase] {
            let expected = mode as u32 != OcclusionMode::Off as u32;
            assert_eq!(
                OcclusionConfig { mode }.enabled(),
                expected,
                "{mode:?}: enabled() must track the discriminant"
            );
        }
    }

    /// The artifact spelling is total, unique, and indexable by the `#[repr(u32)]` discriminant —
    /// the three properties a reader of a `[host] occ_mode` field depends on.
    #[test]
    fn the_artifact_spelling_is_total_and_unique() {
        assert_eq!(OcclusionMode::ALL.len(), 2, "ALL must list every mode");
        for (i, a) in OcclusionMode::ALL.iter().enumerate() {
            assert_eq!(*a as u32, i as u32, "{a:?} must sit at its own discriminant in ALL");
            assert!(!a.as_str().is_empty(), "{a:?} has no word");
            for b in &OcclusionMode::ALL[i + 1..] {
                assert_ne!(a.as_str(), b.as_str(), "{a:?} and {b:?} share a word");
            }
        }
    }

    /// A WILDCARD-FREE match over the enum, so adding a variant fails to COMPILE here rather than
    /// silently inheriting whichever arm a `_` would have swallowed. The bodies restate the
    /// intended answer for each variant; the compile error is the actual gate.
    #[test]
    fn every_variant_states_its_own_answer_without_a_wildcard() {
        for mode in [OcclusionMode::Off, OcclusionMode::TwoPhase] {
            let want = match mode {
                OcclusionMode::Off => false,
                OcclusionMode::TwoPhase => true,
            };
            assert_eq!(OcclusionConfig { mode }.enabled(), want, "{mode:?}");
        }
    }
}
