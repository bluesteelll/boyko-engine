//! VG R3 piece 1 step P1-2 (docs/VG-R3-P1-PYRAMID-PLAN.md §4) — the ONE site that turns the
//! `boyko_render::hzb` oracle into the plain scalars the backend allocates the depth pyramid from.
//!
//! # Why this lives in the host and nowhere else
//!
//! `boyko_render` depends on `boyko_rhi_vulkan`, not the reverse, so the RHI-side owner cannot
//! name [`HzbLayout`]. The plan's resolution is that the RHI stores DERIVED SCALARS and derives
//! nothing: `prev_pow2`, `msb` and `max(1, base >> k)` exist once in the tree, in
//! [`boyko_render::hzb`]. `boyko_app` is the only crate that links both, so it is where the oracle
//! is called and the scalars are handed over — one implementation, no cross-crate formula to
//! drift.
//!
//! # Degrade, never panic
//!
//! An extent the oracle refuses ([`HzbLayoutError`](boyko_render::hzb::HzbLayoutError): a zero
//! axis, or one past `MAX_HZB_EXTENT`) yields `None` — the same disarmed state
//! [`HzbMode::Off`](boyko_render::HzbMode::Off) produces, so a degenerate frame builds no pyramid
//! instead of taking the process down.

use boyko_render::HzbConfig;
use boyko_render::hzb::HzbLayout;
use boyko_rhi_vulkan::present::{HzbPlan, MAX_HZB_LEVELS};

/// The pyramid plan for a `width × height` source, or `None` when the pyramid is disarmed.
///
/// `config` is what `World::try_resource::<HzbConfig>()` yielded: `None` (a host that never
/// composed [`HzbPlugin`](boyko_render::HzbPlugin)) and `Some(Off)` (the default) are the SAME
/// answer — no plan, hence no image, no views, no build passes. Only
/// [`HzbMode::Build`](boyko_render::HzbMode::Build) produces a plan.
///
/// The returned [`HzbPlan`] carries the oracle's own `levels()` and `level_extent(k)` verbatim:
/// level 0 is `prev_pow2` of EACH source axis (not the source extent), every later level is
/// `max(1, base >> k)`, and the chain runs to `1 × 1`.
pub(crate) fn hzb_plan_for(config: Option<HzbConfig>, width: u32, height: u32) -> Option<HzbPlan> {
    if !config.is_some_and(|c| c.enabled()) {
        return None;
    }
    // `Err` is the degrade path, not a bug: a zero axis (a minimized frame that still reached
    // here) or an extent past `MAX_HZB_EXTENT` has no pyramid, and refusing to build one is the
    // 0%-gate rather than a panic.
    let layout = HzbLayout::new(width, height).ok()?;

    let levels = layout.levels();
    // Structurally unreachable, and cheap enough to state anyway: `HzbLayout::new` refuses any
    // axis past `MAX_HZB_EXTENT`, whose level count IS `boyko_render::hzb::MAX_HZB_LEVELS`, and
    // that constant is pinned equal to the RHI's array capacity by a `const _: () = assert!(..)`
    // in `boyko_render::hzb`. Both premises would have to break together for this to fire.
    debug_assert!(
        (levels as usize) <= MAX_HZB_LEVELS,
        "invariant: the oracle's level count fits the backend's inline per-level capacity"
    );

    let mut level_extent = [[0u32; 2]; MAX_HZB_LEVELS];
    for (level, slot) in level_extent.iter_mut().enumerate().take(levels as usize) {
        *slot = layout.level_extent(level as u32);
    }
    Some(HzbPlan { levels, level_extent })
}

#[cfg(test)]
mod tests {
    use boyko_render::HzbMode;

    use super::*;

    /// The five extents plan §5's G3 gate names — odd, non-square, terminal and real — plus the
    /// square power of two every existing golden pin uses (where a base-map or clamp bug cannot
    /// fire, which is exactly why the other five are here).
    const EXTENTS: [(u32, u32); 6] =
        [(7, 3), (8, 16), (1, 1), (511, 1023), (1920, 1080), (512, 512)];

    fn build_config() -> Option<HzbConfig> {
        Some(HzbConfig { mode: HzbMode::Build })
    }

    /// The threaded plan must reproduce the ORACLE, level for level — not a re-derivation that
    /// merely looks like it. Every `k < levels` is compared against `HzbLayout::level_extent(k)`
    /// on a layout built independently in the test.
    #[test]
    fn plan_reproduces_the_oracle_at_every_level() {
        for (w, h) in EXTENTS {
            let plan = hzb_plan_for(build_config(), w, h)
                .unwrap_or_else(|| panic!("{w}x{h}: an armed legal extent must produce a plan"));
            let layout = HzbLayout::new(w, h).expect("invariant: the fixture extents are legal");

            assert_eq!(plan.levels, layout.levels(), "{w}x{h}: level count");
            for level in 0..plan.levels {
                assert_eq!(
                    plan.extent_of(level),
                    layout.level_extent(level),
                    "{w}x{h}: level {level} extent"
                );
            }
        }
    }

    /// Level 0 is `prev_pow2` of each axis INDEPENDENTLY — the property the odd and non-square
    /// fixtures exist for. `7 × 3 → 4 × 2`, `511 × 1023 → 256 × 512`, `1920 × 1080 → 1024 × 1024`
    /// (both axes collapse to the same power of two from very different sources), `1 × 1 → 1 × 1`.
    /// Hand-computed, so a plan that merely agreed with a wrong oracle would still fail here.
    #[test]
    fn level_zero_is_prev_pow2_per_axis_by_hand() {
        let cases = [
            ((7u32, 3u32), [4u32, 2u32], 3u32),
            ((8, 16), [8, 16], 5),
            ((1, 1), [1, 1], 1),
            ((511, 1023), [256, 512], 10),
            ((1920, 1080), [1024, 1024], 11),
        ];
        for ((w, h), base, levels) in cases {
            let plan = hzb_plan_for(build_config(), w, h).expect("armed, legal extent");
            assert_eq!(plan.extent_of(0), base, "{w}x{h}: level 0 base");
            assert_eq!(plan.levels, levels, "{w}x{h}: level count");
        }
    }

    /// The chain terminates at `1 × 1` and each level halves with the `max(1, ..)` clamp — the
    /// clamp being what keeps a long axis halving after the short one has bottomed out (`7 × 3`
    /// reaches `Y == 1` at level 1 while `X` keeps going).
    #[test]
    fn every_chain_halves_with_the_clamp_and_terminates_at_one() {
        for (w, h) in EXTENTS {
            let plan = hzb_plan_for(build_config(), w, h).expect("armed, legal extent");
            for level in 1..plan.levels {
                let [pw, ph] = plan.extent_of(level - 1);
                assert_eq!(
                    plan.extent_of(level),
                    [(pw / 2).max(1), (ph / 2).max(1)],
                    "{w}x{h}: level {level} must be the clamped halving of {level_prev}",
                    level_prev = level - 1
                );
            }
            assert_eq!(
                plan.extent_of(plan.levels - 1),
                [1, 1],
                "{w}x{h}: the complete chain tops out at 1x1"
            );
        }
    }

    /// The 0%-gate, from BOTH routes into it: a world with no [`HzbConfig`] at all (a host that
    /// never composed the plugin) and a world carrying the default `Off`. Neither may produce a
    /// plan, because the plan's presence IS the backend's allocation request.
    #[test]
    fn off_and_absent_both_yield_no_plan() {
        for (w, h) in EXTENTS {
            assert!(hzb_plan_for(None, w, h).is_none(), "{w}x{h}: absent config must not arm");
            assert!(
                hzb_plan_for(Some(HzbConfig::default()), w, h).is_none(),
                "{w}x{h}: the default (Off) config must not arm"
            );
            assert!(
                hzb_plan_for(Some(HzbConfig { mode: HzbMode::Off }), w, h).is_none(),
                "{w}x{h}: an explicit Off must not arm"
            );
        }
    }

    /// An extent the oracle refuses degrades to the disarmed state instead of panicking: a zero
    /// axis (a minimized frame) and an axis past `MAX_HZB_EXTENT`. Both are armed requests, so
    /// this is not the `Off` path — it is the honest "this extent has no pyramid" answer.
    #[test]
    fn an_illegal_extent_degrades_to_no_plan() {
        let too_large = boyko_render::MAX_HZB_EXTENT + 1;
        for (w, h) in [(0, 1080), (1920, 0), (0, 0), (too_large, 8), (8, too_large)] {
            assert!(
                hzb_plan_for(build_config(), w, h).is_none(),
                "{w}x{h}: an extent the oracle refuses must degrade, not panic"
            );
        }
    }

    /// The entries past `levels` are padding, and reading one is a panic rather than a silent
    /// wrong extent — the guard that keeps `MAX_HZB_LEVELS` from being used as a span.
    #[test]
    #[should_panic(expected = "invariant: level is inside the pyramid the host planned")]
    fn reading_past_the_derived_level_count_panics() {
        let plan = hzb_plan_for(build_config(), 7, 3).expect("armed, legal extent");
        let _ = plan.extent_of(plan.levels);
    }
}
