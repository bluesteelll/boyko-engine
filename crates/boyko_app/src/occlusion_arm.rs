//! VG R3 piece 4 rung P4-4 — the ONE site that turns the owner's `OcclusionConfig` and the
//! diagnostic `OcclusionForce` into the plain per-frame value the backend arms the split from.
//!
//! # Why this lives in the host and nowhere else
//!
//! `boyko_render` depends on `boyko_rhi_vulkan`, not the reverse, so the RHI-side owner cannot
//! name [`OcclusionMode`]. `boyko_app` is the only crate that links both — the same layering
//! [`crate::hzb_plan`] resolves the pyramid plan with, and this module is deliberately its twin:
//! one pure function, no `World`, no device, unit-tested without a GPU.
//!
//! # Presence is the arming
//!
//! The result is `Option<VbOcclusionArm>`, not a `(bool, u32)` pair, so "armed" and "which verdict
//! is forced" cannot be carried as two fields that disagree. `None` is the default `Off` and the
//! absent-Resource case alike — the treatment [`crate::hzb_plan::hzb_plan_for`] gives an absent
//! `HzbConfig`.
//!
//! # `Force` without `TwoPhase` is inert, and it is inert HERE
//!
//! A `KeepAll` on an `Off` config yields `None`, so the force word never reaches the scene at all.
//! That is one of two independent reasons the combination changes no pixel — the other being the
//! host fold, which OR-s the FORCE bits only on a frame that takes the split — and having both is
//! deliberate: this one is a property of a pure function a unit test can enumerate.

// `OcclusionMode` is NOT imported here: this fn reads the mode only through
// `OcclusionConfig::enabled()`, so the type name appears in a doc link and in the tests, and
// rustc counts neither as a use. The tests import it themselves.
use boyko_render::OcclusionConfig;
use boyko_rhi_vulkan::present::VbOcclusionArm;

use crate::occlusion_force::OcclusionForce;

/// This frame's occlusion arming, or `None` when the decision is disarmed.
///
/// `config` is what `World::try_resource::<OcclusionConfig>()` yielded: `None` (a host that never
/// composed [`OcclusionPlugin`](boyko_render::OcclusionPlugin)) and `Some(Off)` (the default) are
/// the SAME answer — no arm, hence no split, no late passes, no marked instance tested. Only
/// [`OcclusionMode::TwoPhase`](boyko_render::OcclusionMode::TwoPhase) produces an arm.
///
/// `force` is the diagnostic override, `OcclusionForce::None` on every world that does not insert
/// the Resource. It is carried INSIDE the arm because it is meaningless without one.
pub(crate) fn occlusion_arm_for(
    config: Option<OcclusionConfig>,
    force: OcclusionForce,
) -> Option<VbOcclusionArm> {
    if !config.is_some_and(|c| c.enabled()) {
        return None;
    }
    let force_flags = force.flags();
    // The host half of the shader's own contract: FORCE_KEEP (defer nothing) and FORCE_LATE (defer
    // everything marked) are opposite controls, and the resolution of "both" would be whichever
    // branch the module tests first. `OcclusionForce::flags` cannot produce both — it is a total
    // match naming at most one constant per variant — so this states the property at the boundary
    // where a future second producer of the word would have to meet it.
    debug_assert!(
        force_flags.count_ones() <= 1,
        "invariant: an occlusion arm forces at most one verdict"
    );
    Some(VbOcclusionArm { force_flags })
}

#[cfg(test)]
mod tests {
    use boyko_render::OcclusionMode;
    use boyko_rhi_vulkan::present::{VB_CULL_OCC_FORCE_KEEP, VB_CULL_OCC_FORCE_LATE};

    use super::*;

    /// The whole 2 × 3 product, enumerated: the mode decides PRESENCE and the force decides the
    /// PAYLOAD, and neither decides the other. Written as a table rather than as two loops so a
    /// wrong cell is readable as a wrong cell.
    #[test]
    fn the_mode_decides_presence_and_the_force_decides_the_payload() {
        let cases: [(OcclusionMode, OcclusionForce, Option<u32>); 6] = [
            (OcclusionMode::Off, OcclusionForce::None, None),
            (OcclusionMode::Off, OcclusionForce::KeepAll, None),
            (OcclusionMode::Off, OcclusionForce::DeferAll, None),
            (OcclusionMode::TwoPhase, OcclusionForce::None, Some(0)),
            (OcclusionMode::TwoPhase, OcclusionForce::KeepAll, Some(VB_CULL_OCC_FORCE_KEEP)),
            (OcclusionMode::TwoPhase, OcclusionForce::DeferAll, Some(VB_CULL_OCC_FORCE_LATE)),
        ];
        for (mode, force, want) in cases {
            let got = occlusion_arm_for(Some(OcclusionConfig { mode }), force);
            assert_eq!(
                got.map(|a| a.force_flags),
                want,
                "({mode:?}, {force:?}) must arm {want:?}"
            );
        }
    }

    /// The two disarmed routes agree, for [`crate::hzb_plan`]'s own reason: a host that never
    /// composed the plugin and a host carrying the default must produce the same frame, because
    /// the arm's PRESENCE is what the backend keys the split on.
    #[test]
    fn absent_and_default_are_the_same_answer() {
        for force in OcclusionForce::ALL {
            assert!(
                occlusion_arm_for(None, force).is_none(),
                "{force:?}: an absent config must not arm"
            );
            assert!(
                occlusion_arm_for(Some(OcclusionConfig::default()), force).is_none(),
                "{force:?}: the default (Off) config must not arm"
            );
        }
    }

    /// A forced verdict on a DISARMED config reaches nothing — the inertness claim as a property
    /// of this function, independent of the host fold that also enforces it.
    #[test]
    fn a_force_without_an_arm_is_inert() {
        for force in [OcclusionForce::KeepAll, OcclusionForce::DeferAll] {
            assert_eq!(
                occlusion_arm_for(Some(OcclusionConfig { mode: OcclusionMode::Off }), force),
                None,
                "{force:?} on an Off config must not smuggle a force word onto the scene"
            );
        }
    }

    /// Never both bits: the shader's contradiction, refused at the host boundary for every
    /// regime rather than for the ones a caller happened to try.
    #[test]
    fn no_arm_ever_carries_both_force_bits() {
        for force in OcclusionForce::ALL {
            let arm = occlusion_arm_for(
                Some(OcclusionConfig { mode: OcclusionMode::TwoPhase }),
                force,
            )
            .expect("TwoPhase always arms");
            assert_ne!(
                arm.force_flags & (VB_CULL_OCC_FORCE_KEEP | VB_CULL_OCC_FORCE_LATE),
                VB_CULL_OCC_FORCE_KEEP | VB_CULL_OCC_FORCE_LATE,
                "{force:?}: FORCE_KEEP and FORCE_LATE are opposite controls"
            );
        }
    }
}
