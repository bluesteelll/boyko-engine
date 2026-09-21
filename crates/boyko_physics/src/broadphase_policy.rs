//! P3 — the cold StrategyPolicy substrate for the broadphase (the physics
//! analogue of the lighting P1 policy; `docs/ARCHITECTURE-HYBRID-PERF.md`
//! Part 3.3 + P3).
//!
//! Principle 0: ECS-native — a [`PhysicsStats`] Resource (the cold cost-model
//! carrier, NOT an ad-hoc `static`) plus the cold [`select_broadphase`] policy
//! system; NO side store, NO `dyn`. The policy auto-selects the EXISTING
//! [`PhysicsConfig::broadphase`](crate::resources::PhysicsConfig) gate
//! (AllPairs ↔ Grid) from the live active-body count.
//!
//! # Per-domain, not cross-crate
//!
//! Physics owns its own stats + policy (render has its own `LightStats` from P1):
//! a single cross-crate `SceneStats` would break crate layering — physics and
//! render do not depend on each other. This is the physics-broadphase instance.
//!
//! # Cold by construction (zero hot-path cost)
//!
//! [`select_broadphase`] runs at the gather→broadphase boundary —
//! `.after(physics_gather)` (so the active-body count is fresh: the gather has
//! just refilled [`SolverScratch`](crate::resources::SolverScratch)) and
//! `.before(physics_broadphase)` (so THIS frame's decision feeds the broadphase
//! build, no one-frame staleness). It is the SINGLE owner of every field it writes
//! ([`PhysicsStats::active_body_count`], [`PhysicsStats::broadphase_band`], and, in
//! [`Auto`](crate::resources::BroadphaseSelectMode::Auto) mode,
//! [`PhysicsConfig::broadphase`](crate::resources::PhysicsConfig)) — the Part 2.2
//! write discipline (one producer per field, so no write-write conflict). The hot
//! broadphase loop never reads [`PhysicsStats`].
//!
//! # Why the snapshot count IS the active-body count
//!
//! The broadphase tests EVERY gathered body pairwise — the AllPairs arm runs a
//! `for i in 0..n { for j in (i + 1)..n }` over `SolverScratch::bodies()` with NO
//! per-body simulated/dynamic filter (the feasibility predicate is the only
//! culler, applied per CANDIDATE pair, not per body). So the body count that
//! drives the O(n²) cost — the situation key — is exactly `bodies().len()`. The
//! policy reads the SAME slice the broadphase iterates, so it cannot drift from /
//! double-count the set the broadphase actually pays for.
//!
//! # Result transparency (the P3 0%-result gate)
//!
//! The AllPairs and Grid arms are RESULT-EQUIVALENT: the Grid build emits the SAME
//! feasibility-filtered, `(min, max)`-sorted candidate set as AllPairs (the O2
//! 0%-correctness gate, asserted by the existing `production_grid_equals_all_pairs`
//! test). So flipping `broadphase` in Auto mode changes WHICH broadphase runs,
//! never a physics result bit — the narrowphase input (the candidate pairs) is
//! identical either way.
//!
//! # Hysteresis (anti-thrash)
//!
//! The selector is banded: Grid switches ON at `count >= `[`GRID_HI`] and OFF at
//! `count <= `[`GRID_LO`], keeping the current side ([`PhysicsStats::broadphase_band`])
//! in the band `(LO, HI)`. A single threshold would flip every frame for a body
//! count oscillating across it (Part 2.4); the band absorbs that. The Grid CSR
//! buffers are PREALLOCATED at scene-load
//! ([`BroadphaseGrid::with_capacity`](crate::resources::BroadphaseGrid::with_capacity)),
//! so an AllPairs→Grid flip is a FILL (clear + refill, capacity reused), never a
//! `Vec::new`/grow on the frame path (Principle 5; Part 2.4 transition cost).

use boyko_macros::Resource;

use boyko_ecs::ecs::core::system::{Res, ResMut};

use crate::resources::{BroadphaseKind, BroadphaseSelectMode, PhysicsConfig, SolverScratch};

// ---- banded thresholds, calibrated on the size-disparity crossover (P0b §8) -----------

/// The measured size-disparity crossover of the two broadphases, in bodies: Grid is faster
/// than AllPairs above it. `[MEASURED 2026-09-19, P0b §8, bench profile, K=1]` —
/// `benches/broadphase.rs`'s `broadphase_disparity` group, one process: t_grid / t_all_pairs =
/// 3.908 at 1,000 bodies and 0.220 at 10,000, so the log-log interpolation crosses 1 at 2,978
/// (`docs/measurements/2026-09-19-physics-p0/ANALYSIS.md` §8). The uniform lattice crosses
/// at 1,109, but the Jolt pyramid (1240 boxes on a 50 × 50 slab) behaves like the disparity
/// family — Grid is 2.46× slower there at W=1 — so the disparity crossover is the one the
/// band must clear. Only [`GRID_HI`]'s const-assert reads it.
const DISPARITY_CROSSOVER_BODIES: u32 = 2_978;

/// Banded LOW edge: in [`Auto`](crate::resources::BroadphaseSelectMode::Auto) mode the
/// broadphase switches to [`AllPairs`](crate::resources::BroadphaseKind::AllPairs) when the
/// live active-body count drops to `<= GRID_LO`.
///
/// `[MEASURED 2026-09-19, P0b §8, bench profile, K=1]` — a 10 % dead band under
/// [`GRID_HI`]. A size-disparity scene held on Grid inside the band loses at most about 1.13×
/// on the broadphase (the §8 disparity slope, interpolated at 2,700 bodies). The price, stated:
/// on UNIFORM scenes between the uniform crossover (1,109) and [`GRID_HI`], Auto keeps
/// AllPairs where Grid would be faster — by up to about 2.7× at 3,000 bodies (the §8 uniform
/// slope, interpolated). Two measured scene families cannot calibrate a disparity classifier;
/// the better policy belongs to the broadphase redesign.
pub const GRID_LO: u32 = 2_700;

/// Banded HIGH edge: in [`Auto`](crate::resources::BroadphaseSelectMode::Auto) mode the
/// broadphase switches to [`Grid`](crate::resources::BroadphaseKind::Grid) when the live
/// active-body count rises to `>= GRID_HI`.
///
/// `[MEASURED 2026-09-19, P0b §8, bench profile, K=1]` — the size-disparity crossover
/// (2,978 bodies) to two significant figures, which is all the precision one process in the
/// `bench` profile supports. `GRID_LO < GRID_HI` is the hysteresis gap that prevents boundary
/// thrash. The provisional 96 / 192 it replaces sat 6–31× below both crossovers, and Auto
/// switched the Jolt pyramid to Grid at +3.07 ms per step (gated by
/// `auto_keeps_all_pairs_on_the_jolt_pyramid`).
pub const GRID_HI: u32 = 3_000;

const _: () = assert!(GRID_LO < GRID_HI, "hysteresis: the OFF edge must sit below the ON edge");
const _: () = assert!(
    GRID_HI >= DISPARITY_CROSSOVER_BODIES,
    "GRID_HI must not sit below the measured size-disparity crossover (2,978 bodies, P0b §8): \
     Auto would pick Grid where it measured slower"
);

// ---- PhysicsStats (the cold cost-model carrier) --------------------------------------

/// The cold cost-model input for the broadphase StrategyPolicy (P3) — a Principle-0
/// Resource, read/written ONCE per policy run, NEVER per row.
///
/// [`select_broadphase`] is the SINGLE owner of both fields (the write discipline of
/// Part 2.2 — one producer per field, so no write-write conflict and no race). It is
/// `#[repr(C)]` so its `u32` + `bool` fields have a stable layout.
///
/// - `active_body_count`: the live broadphase body count
///   ([`SolverScratch::bodies`](crate::resources::SolverScratch)`.len()`) — the situation
///   key for the `BroadphaseKind` crossover. This IS the set the broadphase tests pairwise
///   (it applies no per-body filter), so it is the exact cost driver.
/// - `broadphase_band`: the current side of the banded Grid selector (the named hysteresis
///   carrier — Part 2.4; NOT an ad-hoc `static`). `true` ⇒ the band currently selects Grid,
///   `false` ⇒ AllPairs. Meaningful only in
///   [`Auto`](crate::resources::BroadphaseSelectMode::Auto) mode.
#[derive(Resource)]
#[repr(C)]
pub struct PhysicsStats {
    /// Live broadphase body count (the `BroadphaseKind` situation key).
    pub active_body_count: u32,
    /// Current side of the banded Grid selector (hysteresis carrier): `true` ⇒ Grid.
    pub broadphase_band: bool,
}

impl Default for PhysicsStats {
    #[inline]
    fn default() -> Self {
        // Band starts OFF (AllPairs) — matches `PhysicsConfig::broadphase`'s default
        // `BroadphaseKind::AllPairs`, so the first Auto evaluation below `GRID_HI` keeps
        // the kind on AllPairs (the 0%-gate anchor carries over to Auto's cold start).
        Self { active_body_count: 0, broadphase_band: false }
    }
}

/// Banded-hysteresis decision: returns the new band side given the current side and the
/// situation value. Switches to `true` at `value >= hi`, to `false` at `value <= lo`,
/// and otherwise keeps `current` (the dead-band that absorbs boundary oscillation).
///
/// `debug_assert!(lo < hi)` — a degenerate band (`lo >= hi`) has no dead zone and would
/// thrash; the const-assert on [`GRID_LO`]/[`GRID_HI`] enforces this for the shipped band,
/// and this guards any future caller.
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

/// The cold broadphase StrategyPolicy (P3) — counts the live active bodies and, in
/// [`Auto`](crate::resources::BroadphaseSelectMode::Auto) mode, banded-selects
/// [`PhysicsConfig::broadphase`](crate::resources::PhysicsConfig)
/// (AllPairs ↔ Grid).
///
/// Scheduled `.after(physics_gather)` (the body count is fresh) and
/// `.before(physics_broadphase)` (this frame's decision feeds the build — no one-frame
/// staleness). It is the SINGLE owner of the fields it writes (Part 2.2 write discipline):
///
/// 1. Reads the gathered body count from [`SolverScratch`] — the SAME slice the broadphase
///    iterates pairwise, with no per-body filter — and writes it into
///    [`PhysicsStats::active_body_count`].
/// 2. In [`Manual`](crate::resources::BroadphaseSelectMode::Manual) (the default — the
///    0%-gate): leaves [`PhysicsConfig::broadphase`](crate::resources::PhysicsConfig)
///    untouched (user-controlled, byte-identical to pre-P3).
/// 3. In [`Auto`](crate::resources::BroadphaseSelectMode::Auto): applies the
///    [`GRID_LO`]/[`GRID_HI`] [`banded`] hysteresis to the count and writes the result to
///    BOTH [`PhysicsStats::broadphase_band`] and `broadphase` (`true` ⇒ Grid, `false` ⇒
///    AllPairs). Because the two arms are result-equivalent, this is result-transparent.
//
// `clippy::needless_pass_by_value`: `Res<_>` / `ResMut<_>` are by-value `SystemParam`s
// read/written through reborrows — the same false-positive every physics system carries
// (see `physics_gather`).
#[allow(clippy::needless_pass_by_value)]
pub fn select_broadphase(
    scratch: Res<SolverScratch>,
    mut cfg: ResMut<PhysicsConfig>,
    mut stats: ResMut<PhysicsStats>,
) {
    // The cost driver is the gathered body count: the broadphase tests every row pairwise
    // (no per-body cull), so `bodies_len()` is exactly the O(n²) input. `as u32` is sound —
    // a single step never holds `u32::MAX` bodies (the `BodyIndex` row index is itself a
    // `u32`, so the snapshot count fits by construction).
    let count = scratch.bodies_len() as u32;
    stats.active_body_count = count;

    // Manual is the 0%-gate: the policy never touches `broadphase`.
    if cfg.broadphase_select != BroadphaseSelectMode::Auto {
        return;
    }

    let band = banded(stats.broadphase_band, count, GRID_LO, GRID_HI);
    stats.broadphase_band = band;
    cfg.broadphase = if band { BroadphaseKind::Grid } else { BroadphaseKind::AllPairs };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banded_switches_on_at_hi_off_at_lo_and_holds_in_band() {
        // Below LO → OFF regardless of the previous side.
        assert!(!banded(true, GRID_LO, GRID_LO, GRID_HI));
        assert!(!banded(false, 0, GRID_LO, GRID_HI));
        // At/above HI → ON regardless of the previous side.
        assert!(banded(false, GRID_HI, GRID_LO, GRID_HI));
        assert!(banded(false, GRID_HI + 1000, GRID_LO, GRID_HI));
        // Strictly inside (LO, HI) → keep the current side (the dead band).
        let mid = GRID_LO + 1;
        assert!(mid < GRID_HI, "test fixture: mid must lie inside the band");
        assert!(banded(true, mid, GRID_LO, GRID_HI), "was-on stays on inside the band");
        assert!(!banded(false, mid, GRID_LO, GRID_HI), "was-off stays off inside the band");
    }

    #[test]
    fn physics_stats_default_starts_band_off() {
        let s = PhysicsStats::default();
        assert_eq!(s.active_body_count, 0);
        assert!(!s.broadphase_band, "the band cold-starts OFF (matches broadphase's AllPairs default)");
    }
}
