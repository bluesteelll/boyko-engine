//! VG R3 piece 4 rung P4-4 — the **DIAGNOSTIC** verdict override for the occlusion decision.
//!
//! This is not an owner knob and must never become one. It exists so a fixture can hold every
//! mechanism constant and vary ONE push-constant bit — the `[vb_occ_mixed*]` pin ladder, whose
//! `keep → mixed` step is a DECISION contrast precisely because `keep` runs the full machinery
//! with the verdict forced off.
//!
//! # Why it lives here and not beside `OcclusionConfig`
//!
//! `boyko_render::OcclusionMode` answers ONE question — does the owner want the occlusion
//! decision — and has exactly two variants permanently, the rule
//! `boyko_render::HzbMode` states for itself. A verdict override is not an answer to that
//! question; it is a second axis. Two axes, two types, composed as `TwoPhase × Force{None,
//! KeepAll, DeferAll}`, and `Force` without `TwoPhase` is INERT by the existing host fold: the
//! FORCE bits are OR-ed only on a frame that takes the split, and the cull module tests them only
//! inside its armed guard.
//!
//! # Where this replaced a boot-time env read
//!
//! Until this rung the regime was `std::env::var("BOYKO_VG_OCC_FORCE")`, read once inside
//! `GpuSceneBundles::boot` — shipping code, with a boot panic in it, while the ARMING beside it
//! was an ECS-derived per-frame predicate. Two sources of truth for one decision, and nothing
//! checked them against each other. The decode moved out to the fixtures
//! (`boyko_app/tests/occ_fixture`), which is the only place that needs it, and the panic moved
//! with it.
//!
//! The boot read's other rationale — *"a knob that can change mid-run makes 'which regime produced
//! this capture?' unanswerable from the artifact"* — is answered by RECORDING, never by asserting
//! constancy: [`VbRecordProbe::occ_flags`](boyko_rhi_vulkan::present::VbRecordProbe::occ_flags) is
//! stamped from the word the recorder PUSHED, `VbProbeContext` carries the host's independent
//! view beside it, and the bench summary reports the SET of distinct regime words it observed.

use boyko_macros::Resource;
use boyko_rhi_vulkan::present::{VB_CULL_OCC_FORCE_KEEP, VB_CULL_OCC_FORCE_LATE};

/// The artifact/env spelling of [`OcclusionForce::None`].
pub const OCC_FORCE_WORD_NONE: &str = "none";
/// The artifact/env spelling of [`OcclusionForce::KeepAll`].
pub const OCC_FORCE_WORD_KEEP: &str = "keep";
/// The artifact/env spelling of [`OcclusionForce::DeferAll`].
pub const OCC_FORCE_WORD_LATE: &str = "late";

/// A verdict override for measurement and gating — a `World`-singleton Resource, `None` by
/// default and by absence alike.
///
/// **NOT an owner knob** (see the module doc). It is inserted by fixtures, read through
/// `try_resource` — so a host that never inserts it gets [`None`](OcclusionForce::None), exactly
/// as an absent `HzbConfig` means "no pyramid" — and it is INERT unless
/// `boyko_render::OcclusionConfig` armed the split.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OcclusionForce {
    /// No override: the early phase decides from the pyramid. The shipping regime, and the only
    /// one any non-fixture world is ever in.
    #[default]
    None,
    /// `VB_CULL_OCC_FORCE_KEEP` — the early phase defers NOTHING, so the split runs its full
    /// machinery (second raster scope, late cull dispatch, both extra descriptor sets) over an
    /// empty candidate set. The one-variable baseline of the DECISION: `[vb_occ_mixed_keep]`.
    KeepAll,
    /// `VB_CULL_OCC_FORCE_LATE` — the early phase defers EVERY marked instance regardless of the
    /// pyramid. The only regime in which a converged STATIC scene reaches a nonzero late-survivor
    /// count, hence the only one in which the late raster path is exercised at all:
    /// `[vb_occ_mixed_late]`.
    DeferAll,
}

impl OcclusionForce {
    /// The push-constant bits this regime contributes to `GBufferScene::vb_occ_flags`.
    ///
    /// `0`, or exactly ONE of the two FORCE bits — never both, which is a property of this
    /// function's shape (a total match over three variants, each naming at most one constant)
    /// rather than of a caller's discipline. The two are opposite controls and the resolution of
    /// "both" would be whichever branch the shader tests first.
    #[inline]
    pub const fn flags(self) -> u32 {
        match self {
            OcclusionForce::None => 0,
            OcclusionForce::KeepAll => VB_CULL_OCC_FORCE_KEEP,
            OcclusionForce::DeferAll => VB_CULL_OCC_FORCE_LATE,
        }
    }

    /// The artifact spelling — `"none"` / `"keep"` / `"late"`.
    ///
    /// ONE table serves both directions: the probe dump prints this, the fixtures' env decode
    /// ([`Self::from_word`]) reads it, and `[vb_occ_mixed_keep.env]`'s `BOYKO_VG_OCC_FORCE`
    /// value is literally one of these words. A second spelling table would be a second text that
    /// can disagree with the pin file.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            OcclusionForce::None => OCC_FORCE_WORD_NONE,
            OcclusionForce::KeepAll => OCC_FORCE_WORD_KEEP,
            OcclusionForce::DeferAll => OCC_FORCE_WORD_LATE,
        }
    }

    /// The inverse of [`Self::as_str`]: `None` for a word that is not a regime.
    ///
    /// Returns an `Option` rather than defaulting, because "not a regime" must be LOUD at the
    /// caller. A typo'd `BOYKO_VG_OCC_FORCE` that silently rendered the default regime while the
    /// operator believed it forced one is how a control gets reported as green — the reason the
    /// deleted boot decode panicked, kept here as the caller's obligation.
    #[inline]
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            OCC_FORCE_WORD_NONE => Some(OcclusionForce::None),
            OCC_FORCE_WORD_KEEP => Some(OcclusionForce::KeepAll),
            OCC_FORCE_WORD_LATE => Some(OcclusionForce::DeferAll),
            _ => Option::None,
        }
    }

    /// This regime's index in [`Self::ALL`] — `0..3`, dense and total.
    ///
    /// It exists so an observer can accumulate the SET of regimes it saw as a bitmask (`1 << slot`)
    /// with no allocation and no scan: the bench summary's `VB-P4 regime observed=[…]` line does
    /// exactly that, once per timed frame, on the frame loop. The
    /// `VbTimedPass::slot()` precedent (retired at profiling rung 7, its ids now `gpu_zone`'s
    /// `ZONE_VB_*` constants) — a table-driven index, pinned by a bijection test, so no
    /// call site re-derives one.
    #[inline]
    pub const fn slot(self) -> u8 {
        match self {
            OcclusionForce::None => 0,
            OcclusionForce::KeepAll => 1,
            OcclusionForce::DeferAll => 2,
        }
    }

    /// Every regime, for exhaustive iteration in tests and in the fixtures' error messages. The
    /// array's LENGTH is the arity, so a new variant that is not listed here fails the round-trip
    /// test below rather than being silently untested.
    pub const ALL: [OcclusionForce; 3] =
        [OcclusionForce::None, OcclusionForce::KeepAll, OcclusionForce::DeferAll];
}

// The two FORCE bits are DISJOINT single bits — asserted at compile time rather than in a test,
// because the property is about two constants this crate does not own and a change to either
// would otherwise be caught only by whichever gate happened to run.
const _: () = assert!(
    VB_CULL_OCC_FORCE_KEEP.count_ones() == 1,
    "VB_CULL_OCC_FORCE_KEEP must be a single bit"
);
const _: () = assert!(
    VB_CULL_OCC_FORCE_LATE.count_ones() == 1,
    "VB_CULL_OCC_FORCE_LATE must be a single bit"
);
const _: () = assert!(
    VB_CULL_OCC_FORCE_KEEP & VB_CULL_OCC_FORCE_LATE == 0,
    "the two FORCE bits are opposite controls and must not overlap"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_none_from_both_routes() {
        assert_eq!(OcclusionForce::default(), OcclusionForce::None);
        assert_eq!(OcclusionForce::default().flags(), 0, "the default regime pushes no FORCE bit");
    }

    /// `flags()` is `0` or exactly one bit, and the two nonzero regimes disagree — the property
    /// that makes "defer nothing" and "defer everything" distinguishable at the push constant.
    #[test]
    fn flags_are_zero_or_one_disjoint_bit() {
        assert_eq!(OcclusionForce::None.flags(), 0);
        for f in [OcclusionForce::KeepAll, OcclusionForce::DeferAll] {
            assert_eq!(f.flags().count_ones(), 1, "{f:?}: exactly one bit");
        }
        assert_ne!(
            OcclusionForce::KeepAll.flags(),
            OcclusionForce::DeferAll.flags(),
            "KeepAll and DeferAll are opposite controls; one word for both would make the pin \
             ladder's `keep -> late` step a no-op"
        );
        assert_eq!(
            OcclusionForce::KeepAll.flags() & OcclusionForce::DeferAll.flags(),
            0,
            "the two regimes must not share a bit"
        );
    }

    /// `as_str()` ↔ [`OcclusionForce::from_word`] round-trip over EVERY variant, plus the
    /// uniqueness of the words. This is the check that keeps the artifact's spelling and the env
    /// decode one table: if a variant is added and given a word that collides, the second half
    /// fires; if it is added and left out of `ALL`, the arity assert fires.
    #[test]
    fn every_regime_round_trips_through_its_word() {
        assert_eq!(OcclusionForce::ALL.len(), 3, "ALL must list every regime");
        for f in OcclusionForce::ALL {
            assert_eq!(
                OcclusionForce::from_word(f.as_str()),
                Some(f),
                "{f:?}: `{}` must decode back to itself",
                f.as_str()
            );
        }
        for (i, a) in OcclusionForce::ALL.iter().enumerate() {
            for b in &OcclusionForce::ALL[i + 1..] {
                assert_ne!(a.as_str(), b.as_str(), "{a:?} and {b:?} share a word");
            }
        }
    }

    /// A word that is not a regime decodes to nothing — including the plausible-looking ones a
    /// typo produces. The caller's obligation is to panic on this, not to default.
    #[test]
    fn a_non_regime_word_decodes_to_nothing() {
        for word in ["", "keep_all", "KEEP", "Late", "1", "true", "off"] {
            assert_eq!(
                OcclusionForce::from_word(word),
                None,
                "`{word}` is not a regime and must not decode to one"
            );
        }
    }

    /// `slot()` is a BIJECTION onto `0..ALL.len()` — the property the bench summary's regime
    /// bitmask rests on. A variant sharing a slot would make two regimes indistinguishable in the
    /// observed set, i.e. `n_distinct` would under-report a mid-run flip.
    #[test]
    fn slot_is_a_bijection_onto_the_index_range() {
        for (i, f) in OcclusionForce::ALL.iter().enumerate() {
            assert_eq!(
                f.slot() as usize,
                i,
                "{f:?}: slot() must be its own index in ALL (a dense, total map)"
            );
        }
        assert!(
            OcclusionForce::ALL.len() <= 8,
            "the observed-regime set is accumulated in a u8 bitmask; a fourth-plus regime past 8 \
             would need a wider word, not a wider mask"
        );
    }

    /// The pinned env values, verbatim from `goldens/PINS.toml`'s `[vb_occ_mixed_keep.env]` and
    /// `[vb_occ_mixed_late.env]`. Spelled as literals so a rename of either regime word reds HERE,
    /// where the pin file's meaning is, rather than on a GPU sweep hours later.
    #[test]
    fn the_pinned_env_values_decode_to_the_pinned_regimes() {
        assert_eq!(OcclusionForce::from_word("keep"), Some(OcclusionForce::KeepAll));
        assert_eq!(OcclusionForce::from_word("late"), Some(OcclusionForce::DeferAll));
    }
}
