//! P1 — the cold StrategyPolicy substrate for lighting (the first instance of the
//! "cost-model policy layer" pattern; `docs/ARCHITECTURE-HYBRID-PERF.md` Part 2.2).
//!
//! Principle 0: ECS-native — a [`LightStats`] Resource (the cold cost-model carrier,
//! NOT an ad-hoc `static`) plus the cold [`select_lighting_cull`] policy system; NO
//! side store, NO `dyn`. The policy auto-selects the EXISTING
//! [`LightingConfig::clusters_enabled`](crate::light::LightingConfig) gate from the
//! live point/spot light count.
//!
//! # Per-domain, not cross-crate
//!
//! Render owns its own stats + policy (`physics` gets its own in P3): a single
//! cross-crate `SceneStats` would break crate layering — physics and render do not
//! depend on each other. This is the render-lighting instance only.
//!
//! # Cold by construction (zero hot-path cost)
//!
//! [`select_lighting_cull`] runs at the gather/setup boundary — scheduled BEFORE
//! `collect_lights` so the fresh decision feeds the header fold the SAME frame
//! (no one-frame staleness) — and it is the SINGLE owner of every field it writes
//! (`LightStats.point_spot_count`, `LightStats.cluster_band`, and, in
//! [`Auto`](crate::light::ClusterSelectMode::Auto) mode, `LightingConfig.clusters_enabled`).
//! The per-row resolve never reads [`LightStats`]; the only hot consumer is the header
//! word `collect_lights` already folds.
//!
//! # Hysteresis (anti-thrash)
//!
//! The selector is banded: clusters switch ON at `count >= `[`CLUSTER_HI`] and OFF at
//! `count <= `[`CLUSTER_LO`], keeping the current side ([`LightStats::cluster_band`]) in
//! the band `(LO, HI)`. A single threshold would flip every frame for a light count
//! oscillating across it (Part 2.4); the band absorbs that.

use boyko_macros::Resource;

use boyko_ecs::ecs::core::iters::query::{IsEnabled, Query, With};
use boyko_ecs::ecs::core::system::ResMut;

use crate::light::{ClusterSelectMode, LightEnabled, LightingConfig, PointLight, SpotLight};

// ---- banded thresholds (VB-P1d measured; see docs/VB-PERFORMANCE-TRACK.md) -----------

/// Banded LOW edge: in [`Auto`](crate::light::ClusterSelectMode::Auto) mode the cluster
/// path switches OFF when the live point/spot light count drops to `<= CLUSTER_LO`.
///
/// `[MEASURED]` (VB-P1d, RTX 3060, `crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs`): the
/// froxel light-cull is O(clusters × lights) (`cluster_cull.hlsl` dispatches one thread per
/// froxel, each linearly scanning every light) and DOMINATES `froxel_total_ns` — it only
/// beats the flat all-lights scan above ~100 point/spot lights (measured break-even ≈ 103,
/// linearly interpolated between the N_ps=64 and N_ps=128 samples below). At 8 lights
/// clustering is ~43% SLOWER than flat. `CLUSTER_LO = 64` disarms where the flat scan clearly
/// wins (flat 95877 ns vs froxel 102720 ns at N_ps=64, +7% flat's favor). A future cull
/// optimization (the `uint local[256]` per-thread spill in `cluster_cull.hlsl`) would lower
/// this break-even and could tighten the band.
///
/// Measured `froxel_total_ns = cull_ns + shade_ns` vs `flat_shade_ns`, averaged over 100 timed
/// frames per config (froxel_shade stays ~25-30k ns regardless of N_ps — the clustering
/// payoff; flat_shade and froxel_cull both grow ~linearly with N_ps):
/// - N_ps=8:   flat 32799 | froxel 46816 (cull 19741 + shade 27075) — flat wins (+43%)
/// - N_ps=32:  flat 60815 | froxel 71999 (cull 42253 + shade 29747) — flat wins
/// - N_ps=64:  flat 95877 | froxel 102720 (cull 72748 + shade 29973) — flat wins (+7%)
/// - N_ps=128: flat 167322 | froxel 163039 (cull 134920 + shade 28119) — froxel wins (-2.6%)
/// - N_ps=256: flat 315044 | froxel 277662 (cull 252154 + shade 25508) — froxel wins (-12%)
/// - N_ps=512: flat 592015 | froxel 523370 (cull 498067 + shade 25303) — froxel wins (-12%)
///
/// ⚠️ `[REPRODUCIBILITY CAVEAT]` (added at VB-P1e rung H0, 2026-07-25). A re-measurement on the
/// SAME RTX 3060 reproduces the `N_ps` ≤ 128 rows within ~6% PER LEG, but **not** the high rows
/// sitting ABOVE this band: `N_ps=256` came out +21% (froxel) / +23% (flat) and `N_ps=512`
/// **+125% on the flat leg** / +55%
/// on the froxel leg, with a ~21% run-to-run spread at `N_ps=512` (the pass is stable WITHIN a
/// run — `BOYKO_VB_BENCH_FRAMES` 40 vs 220 differ by 0.13% — and unstable ACROSS runs; GPU
/// power/clock state is the leading suspect, not confirmed). Re-measured `flat / froxel_total`
/// (ns): `30888 / 44963` @8, `57586 / 67562` @32, `96587 / 96242` @64, `158622 / 173013` @128,
/// `387133 / 335179` @256, `1330623 / 810285` @512 — so froxel's margin moves -42.7→-45.6% @8,
/// -18.4→-17.3% @32, **-7.1→+0.4% @64**, **+2.6→-9.1% @128**, +11.9→+13.4% @256, +11.6→+39.1%
/// @512: NON-MONOTONIC exactly inside `[LO, HI]`, with BOTH determining rows flipping SIGN even
/// though both reproduce. They flip because a margin is a RATIO of two legs each holding only to
/// ~6%, and two such legs admit ~12.8% of movement in their ratio (1.06/0.94) — which covers the
/// +7.1% margin at 64 as well as the 2.6% one at 128. (The +7.1% is NOT itself below the ~6%
/// per-leg figure; it is the RATIO band that makes it unresolvable.)
///
/// ⚠️ **The re-measurement does not support [`CLUSTER_HI`].** Four of the six rows DO move toward
/// clustering (+1.1 / +7.5 / +1.6 / +27.5 points at `N_ps` 32 / 64 / 256 / 512) — the source of
/// the old "favours clustering MORE, so the constants stay conservative" reading. But `N_ps=128`
/// is one of the two that move the other way (`N_ps=8` is the other, -2.8), and 128 is both the
/// value `HI` takes and the row its "froxel already wins by ~2.6%" justification cites: there the
/// margin moves **11.6 points AGAINST** clustering, measuring the froxel leg **9.1% SLOWER** at
/// exactly the count `HI` arms it. Re-running the interpolation this doc used for ≈103 (linear in
/// the `flat - froxel` ns difference) over the re-measured rows moves the durable crossing to
/// **≈156** — bracketed by 128 at `-14391` ns and 256 at `+51954` ns, i.e. ABOVE `HI` rather than
/// below it; on percentage margins the same pair gives ≈180. That ≈156 leans on the
/// NON-reproducing `N_ps=256` row and is NOT a replacement constant, but the finding needs only
/// the reproducing `N_ps=128` row.
///
/// `CLUSTER_LO = 64` does survive: in the re-measured sweep, AT OR BELOW `N_ps=128`, the froxel
/// leg leads only inside a ~2.6-light-wide window (interpolated crossings at ≈63 and ≈65) and
/// only by 345 ns (0.4%), while below that window both sweeps agree flat wins by 17-46% (`N_ps`
/// 32 and 8) — far outside the ~12.8% ratio band. (Froxel leads again above the ≈156 crossing;
/// that is `HI`'s problem, not `LO`'s.) Re-tuning `HI` needs a repeated-run protocol with a
/// stated variance band, not another single sweep. Tracked as VB-P1f in
/// `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md`.
pub const CLUSTER_LO: u32 = 64;

/// Banded HIGH edge: in [`Auto`](crate::light::ClusterSelectMode::Auto) mode the cluster
/// path switches ON when the live point/spot light count rises to `>= CLUSTER_HI`.
///
/// `[MEASURED]` (VB-P1d, see [`CLUSTER_LO`]'s own doc for the full data table + provenance).
/// `CLUSTER_HI = 128` arms above the ≈103 break-even measured in THAT table (where froxel wins
/// by ~2.6% at N_ps=128, widening to ~-12% by N_ps=256/512), and the band `[64, 128]` straddles
/// that ≈103. `CLUSTER_LO < CLUSTER_HI` is the hysteresis gap that prevents boundary thrash.
///
/// ⚠️ **This is the constant the H0 re-measurement contradicts — read [`CLUSTER_LO`]'s
/// REPRODUCIBILITY CAVEAT before trusting the paragraph above.** The `N_ps=128` row that
/// determines `HI` reproduced to ~6% per leg and still flipped SIGN (froxel 9.1% SLOWER, not 2.6%
/// faster), which puts the re-measured crossing ABOVE 128 rather than below it. `HI` stays at 128
/// because VB-P1e does not re-tune the band (plan §1.4 consequence 3; the re-tune is VB-P1f), NOT
/// because the re-measurement supports it.
pub const CLUSTER_HI: u32 = 128;

const _: () = assert!(CLUSTER_LO < CLUSTER_HI, "hysteresis: the OFF edge must sit below the ON edge");

// ---- LightStats (the cold cost-model carrier) ----------------------------------------

/// The cold cost-model input for the lighting StrategyPolicy (P1) — a Principle-0
/// Resource, read ONCE per policy run, NEVER per row.
///
/// `select_lighting_cull` is the SINGLE owner of both fields (the write discipline of
/// Part 2.2 — one producer per field, so no write-write conflict and no race). It is
/// `#[repr(C)]` so its two `u32`-sized fields have a stable layout.
///
/// - `point_spot_count`: the live, ENABLED point/spot light count (honoring
///   [`IsEnabled<LightEnabled>`] exactly as `collect_lights` does) — the situation key
///   for the `clusters_enabled` crossover.
/// - `cluster_band`: the current side of the banded cluster selector (the named
///   hysteresis carrier — Part 2.4; NOT an ad-hoc `static`). `true` ⇒ the band currently
///   selects ON, `false` ⇒ OFF. Meaningful only in
///   [`Auto`](crate::light::ClusterSelectMode::Auto) mode.
#[derive(Resource)]
#[repr(C)]
pub struct LightStats {
    /// Live ENABLED point/spot light count (the `clusters_enabled` situation key).
    pub point_spot_count: u32,
    /// Current side of the banded cluster selector (hysteresis carrier).
    pub cluster_band: bool,
}

impl Default for LightStats {
    #[inline]
    fn default() -> Self {
        // Band starts OFF — matches `LightingConfig::clusters_enabled`'s `false` default
        // so the first Auto evaluation below `CLUSTER_HI` keeps the gate off (the 0%-gate
        // anchor carries over to Auto's cold start).
        Self { point_spot_count: 0, cluster_band: false }
    }
}

/// Banded-hysteresis decision: returns the new band side given the current side and the
/// situation value. Switches to `true` at `value >= hi`, to `false` at `value <= lo`,
/// and otherwise keeps `current` (the dead-band that absorbs boundary oscillation).
///
/// `debug_assert!(lo < hi)` — a degenerate band (`lo >= hi`) has no dead zone and would
/// thrash; the const-assert on [`CLUSTER_LO`]/[`CLUSTER_HI`] enforces this for the shipped
/// band, and this guards any future caller.
#[inline]
fn banded(current: bool, value: u32, lo: u32, hi: u32) -> bool {
    debug_assert!(lo < hi, "invariant: banded selector needs lo < hi for hysteresis");
    if value >= hi {
        true
    } else if value <= lo {
        false
    } else {
        current
    }
}

// ---- the cold StrategyPolicy system --------------------------------------------------

/// The cold lighting StrategyPolicy (P1) — counts live ENABLED point/spot lights and, in
/// [`Auto`](crate::light::ClusterSelectMode::Auto) mode, banded-selects
/// [`LightingConfig::clusters_enabled`](crate::light::LightingConfig).
///
/// Scheduled BEFORE `collect_lights` (so this frame's decision feeds the header fold —
/// no one-frame staleness). It is the SINGLE owner of the fields it writes (Part 2.2
/// write discipline):
///
/// 1. Counts entities with a `PointLight` OR a `SpotLight` whose
///    [`IsEnabled<LightEnabled>`] bit is set — mirroring how `collect_lights` decides a
///    light is enabled (a `LightEnabled`-disabled light is NOT counted). Writes the count
///    into [`LightStats::point_spot_count`].
/// 2. In [`Manual`](crate::light::ClusterSelectMode::Manual) (the default — the 0%-gate):
///    leaves `clusters_enabled` untouched (owner-controlled, byte-identical to pre-P1).
/// 3. In [`Auto`](crate::light::ClusterSelectMode::Auto): applies the
///    [`CLUSTER_LO`]/[`CLUSTER_HI`] `banded` hysteresis to the count and writes the
///    result to BOTH [`LightStats::cluster_band`] and `clusters_enabled`.
//
// `clippy::needless_pass_by_value`: `Query`/`ResMut` are by-value `SystemParam`s
// read/written through reborrows — the same false-positive `collect_lights` carries.
#[allow(clippy::needless_pass_by_value)]
pub fn select_lighting_cull(
    points: Query<IsEnabled<LightEnabled>, With<PointLight>>,
    spots: Query<IsEnabled<LightEnabled>, With<SpotLight>>,
    mut cfg: ResMut<LightingConfig>,
    mut stats: ResMut<LightStats>,
) {
    // Count ENABLED point + spot lights. `IsEnabled<LightEnabled>` is non-filtering, so a
    // never-toggled (seeded-enabled) row reads `true` and a disabled row reads `false` —
    // the SAME enabled-test `collect_lights` folds on. Each light kind is queried
    // separately (a `With<PointLight>`/`With<SpotLight>` structural filter); the
    // archetype signatures are disjoint, so the two counts never double-count a row.
    let count = points.iter().filter(|&en| en).count() + spots.iter().filter(|&en| en).count();
    let count = count as u32;
    stats.point_spot_count = count;

    // Manual is the 0%-gate: the policy never touches `clusters_enabled`.
    if cfg.cluster_select != ClusterSelectMode::Auto {
        return;
    }

    let band = banded(stats.cluster_band, count, CLUSTER_LO, CLUSTER_HI);
    stats.cluster_band = band;
    cfg.clusters_enabled = band;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banded_switches_on_at_hi_off_at_lo_and_holds_in_band() {
        // Below LO → OFF regardless of the previous side.
        assert!(!banded(true, CLUSTER_LO, CLUSTER_LO, CLUSTER_HI));
        assert!(!banded(false, 0, CLUSTER_LO, CLUSTER_HI));
        // At/above HI → ON regardless of the previous side.
        assert!(banded(false, CLUSTER_HI, CLUSTER_LO, CLUSTER_HI));
        assert!(banded(false, CLUSTER_HI + 100, CLUSTER_LO, CLUSTER_HI));
        // Strictly inside (LO, HI) → keep the current side (the dead band).
        let mid = CLUSTER_LO + 1;
        assert!(mid < CLUSTER_HI, "test fixture: mid must lie inside the band");
        assert!(banded(true, mid, CLUSTER_LO, CLUSTER_HI), "was-on stays on inside the band");
        assert!(!banded(false, mid, CLUSTER_LO, CLUSTER_HI), "was-off stays off inside the band");
    }

    #[test]
    fn light_stats_default_starts_band_off() {
        let s = LightStats::default();
        assert_eq!(s.point_spot_count, 0);
        assert!(!s.cluster_band, "the band cold-starts OFF (matches clusters_enabled's default)");
    }
}
