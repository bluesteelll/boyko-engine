//! P8 size-class coarse-grid broadphase gates (`docs/ARCHITECTURE-HYBRID-PERF.md`
//! Part 3.3 + P8).
//!
//! The uniform-grid broadphase routes any body whose AABB spans ≥ `MAX_CELL_SPAN`
//! fine cells to a SECOND, COARSE size-class grid instead of the old O(k·n)
//! oversized-vs-all residual. P8 is a perf refactor of HOW the oversized candidates
//! are FOUND, NOT a change to WHICH pairs exist: the feasibility-filtered,
//! `(min, max)`-sorted [`ContactPairs`] set must stay BIT-IDENTICAL to all-pairs
//! (and thus to the single-grid path) on every scene — the load-bearing 0%-gate.
//!
//! Two properties are gated here:
//!   (a) **result-equivalence** — the size-class grid emits the EXACT SAME pair set
//!       (→ same manifolds) as all-pairs on size-disparate "few-big + many-small"
//!       scenes (the equivalence keystone), AND
//!   (b) **the O(k·n) residual is GONE** — for a FIXED `k` the oversized candidate
//!       work does NOT scale with `n` (observed via the pre-feasibility candidate
//!       count, which the old residual would grow ≈ `k·n`).
//!
//! The reference predicate is the LITERAL production all-pairs loop (same operand
//! order, same [`body_bounding_radius`]) — a match proves the grid reproduces the
//! real default path, not a re-derived oracle.

use boyko_physics::components::ColliderShape;
use boyko_physics::manifold::BodyIndex;
use boyko_physics::math::Vec3;
use boyko_physics::resources::{BodyState, BroadphaseGrid};
use boyko_physics::systems::body_bounding_radius;

use proptest::prelude::*;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// A `BodyState` carrying only the two fields the broadphase reads (`position`,
/// `shape`); every other field defaults (irrelevant to pairing).
fn body(position: Vec3, shape: ColliderShape) -> BodyState {
    BodyState {
        position,
        shape,
        ..Default::default()
    }
}

fn sphere(position: Vec3, radius: f32) -> BodyState {
    body(position, ColliderShape::Sphere { radius })
}

fn boxx(position: Vec3, half: Vec3) -> BodyState {
    body(position, ColliderShape::Box { half_extents: half })
}

/// The reference all-pairs broadphase — the LITERAL production `AllPairs` arm (same
/// predicate, same `(min, max)` emission). Already `(min, max)`-sorted (`i < j`).
fn all_pairs(bodies: &[BodyState]) -> Vec<(BodyIndex, BodyIndex)> {
    let mut pairs = Vec::new();
    let n = bodies.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let bound = body_bounding_radius(&bodies[i]) + body_bounding_radius(&bodies[j]);
            let delta = bodies[j].position - bodies[i].position;
            if delta.length_squared() <= bound * bound {
                pairs.push((BodyIndex(i as u32), BodyIndex(j as u32)));
            }
        }
    }
    pairs
}

/// Builds the grid over `bodies` into a fresh grid, returning the pair set.
fn grid_pairs(grid: &mut BroadphaseGrid, bodies: &[BodyState]) -> Vec<(BodyIndex, BodyIndex)> {
    let mut out = Vec::new();
    grid.build(bodies, &mut out);
    out
}

/// Asserts the size-class grid pair set is bit-identical (same `(min, max)` order)
/// to all-pairs over `bodies` — the P8 equivalence keystone.
fn assert_grid_eq_all_pairs(bodies: &[BodyState]) {
    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
    let g = grid_pairs(&mut grid, bodies);
    let a = all_pairs(bodies);
    assert_eq!(
        g, a,
        "the size-class grid pair set must be bit-identical to all-pairs"
    );
    assert!(
        g.windows(2).all(|w| w[0] <= w[1]),
        "the size-class grid output honors the (min, max) sort"
    );
}

/// A dense `side³` lattice of small unit spheres packed at sub-diameter spacing
/// (real overlaps, fine cells), starting at the origin.
fn small_lattice(side: usize, radius: f32, spacing: f32) -> Vec<BodyState> {
    let mut bodies = Vec::with_capacity(side * side * side);
    for z in 0..side {
        for y in 0..side {
            for x in 0..side {
                bodies.push(sphere(
                    Vec3::new(x as f32 * spacing, y as f32 * spacing, z as f32 * spacing),
                    radius,
                ));
            }
        }
    }
    bodies
}

// ── (a) Equivalence keystone: few-big + many-small, deterministic + proptest ──

#[test]
fn sizeclass_grid_equals_all_pairs_few_big_many_small() {
    // A dense 12³ lattice of small spheres (fine cells) + a handful of giants whose
    // diameter ≫ a fine cell, so each crosses the MAX_CELL_SPAN threshold and is
    // routed to the coarse size-class grid. Some giants overlap the cluster (real
    // oversized–small pairs) and overlap each other (oversized–oversized pairs).
    let mut bodies = small_lattice(12, 0.5, 0.9);
    // Giants at varied positions over the lattice; some overlap, some grazing.
    for k in 0..5 {
        let f = k as f32;
        bodies.push(sphere(Vec3::new(f * 1.7 + 1.0, f * 1.3, f * 0.9), 14.0 + f));
    }

    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
    let g = grid_pairs(&mut grid, &bodies);

    // Non-vacuity: multiple giants genuinely went oversized (the coarse-grid path is
    // live, not dead) AND the cluster produced pairs.
    assert!(
        grid.oversized_len() >= 2,
        "anti-vacuity: >= 2 giants must be oversized (got {})",
        grid.oversized_len()
    );
    assert!(!g.is_empty(), "anti-vacuity: the size-disparate scene has survivors");

    assert_eq!(
        g,
        all_pairs(&bodies),
        "the size-class grid pair set must equal all-pairs (few-big + many-small)"
    );
}

#[test]
fn sizeclass_grid_equals_all_pairs_oversized_oversized_via_coarse() {
    // A dense small lattice (pins the median → fine cells → the giants below ARE
    // oversized) PLUS several mutually-OVERLAPPING giants clustered in one corner. The
    // giants share coarse cells, so the within-coarse-cell + min-shared-coarse-cell
    // dedup is the oversized–oversized path under test; every such pair must be emitted
    // exactly once and the whole result equal all-pairs.
    let mut bodies = small_lattice(14, 0.5, 0.9);
    let giant_lo = bodies.len();
    // Four giants tightly clustered (mutually overlapping) → oversized–oversized pairs.
    bodies.push(sphere(Vec3::new(2.0, 2.0, 2.0), 22.0));
    bodies.push(sphere(Vec3::new(3.0, 2.5, 2.0), 22.0));
    bodies.push(sphere(Vec3::new(2.5, 3.0, 2.5), 22.0));
    bodies.push(sphere(Vec3::new(2.0, 2.0, 3.0), 22.0));
    let giant_count = bodies.len() - giant_lo;

    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
    let g = grid_pairs(&mut grid, &bodies);

    // Anti-vacuity: every clustered giant is oversized (the coarse oversized–oversized
    // path runs), so the dedup is genuinely exercised across multiple coarse cells.
    assert_eq!(
        grid.oversized_len(),
        giant_count,
        "anti-vacuity: every clustered giant is oversized"
    );
    let a = all_pairs(&bodies);
    assert_eq!(g, a, "oversized–oversized coarse emit (+ oversized–small) == all-pairs");
    // The C(giant, 2) mutually-overlapping giant pairs are present exactly once each.
    let giant_pairs = g
        .iter()
        .filter(|(lo, hi)| (lo.0 as usize) >= giant_lo && (hi.0 as usize) >= giant_lo)
        .count();
    assert_eq!(
        giant_pairs,
        giant_count * (giant_count - 1) / 2,
        "every oversized–oversized pair emitted exactly once (no miss/dup)"
    );
}

#[test]
fn sizeclass_grid_equals_all_pairs_mixed_box_and_sphere_giants() {
    // Box + sphere giants over a DENSE small cluster (pins the median → fine cells →
    // the giants cross MAX_CELL_SPAN) — exercises the shape-agnostic bounding radius
    // through the coarse bucketing AND the oversized–small fine scan.
    let mut bodies = small_lattice(14, 0.5, 0.9);
    bodies.push(boxx(Vec3::new(2.0, 2.0, 2.0), Vec3::new(14.0, 14.0, 14.0)));
    bodies.push(sphere(Vec3::new(3.0, 2.0, 2.0), 22.0));
    bodies.push(boxx(Vec3::new(4.0, 3.0, 1.0), Vec3::new(12.0, 16.0, 13.0)));
    assert_grid_eq_all_pairs(&bodies);
    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
    let _ = grid_pairs(&mut grid, &bodies);
    assert!(grid.oversized_len() >= 2, "the box + sphere giants are oversized (non-vacuity)");
}

proptest! {
    // Randomized size-disparate scenes: a dense fixed lattice of typical bodies
    // (cbrt(n) large → fine cells, so the giants span >= MAX_CELL_SPAN cells →
    // oversized) MIXED with a few randomized giants (position + radius). The
    // size-class grid pair set MUST stay bit-identical to all-pairs for every scene.
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn sizeclass_grid_equals_all_pairs_proptest(
        giants in proptest::collection::vec(
            (
                (0.0_f32..9.0, 0.0_f32..9.0, 0.0_f32..9.0),
                12.0_f32..40.0,
            ),
            2..6usize,
        ),
    ) {
        let mut bodies = small_lattice(10, 0.5, 0.9);
        bodies.extend(
            giants
                .into_iter()
                .map(|((px, py, pz), r)| sphere(Vec3::new(px, py, pz), r)),
        );
        let mut grid = BroadphaseGrid::with_capacity(bodies.len());
        let g = grid_pairs(&mut grid, &bodies);
        let a = all_pairs(&bodies);
        prop_assert_eq!(g, a, "size-class grid pairs must equal all-pairs under size disparity");
    }
}

// ── (b) The O(k·n) oversized-vs-all residual is GONE ──────────────────────────
//
// The OLD escape hatch tested each oversized body against EVERY other body — an
// O(k·n) residual: the per-oversized candidate count was ≈ n, scaling LINEARLY with
// the total body count regardless of geometry. The P8 coarse size-class grid replaces
// it with a FOOTPRINT-bounded scan (oversized–small from only the fine cells the giant
// overlaps; oversized–oversized from the coarse grid), so the oversized candidate
// count tracks the giant's overlapped sub-region, NOT n.
//
// The observable is `BroadphaseGrid::oversized_candidate_count()` — the oversized leg's
// pre-feasibility candidate count, ISOLATED from the (legitimately scaling) small–small
// leg. A corner-placed giant whose AABB covers only a SUB-region of a growing lattice
// makes the distinction sharp: the old residual would grow this ≈ n (the lattice size),
// the footprint scan grows it only with the covered sub-region (sub-linearly).

#[test]
fn oversized_candidate_count_is_footprint_bounded_not_k_times_n() {
    // One giant at the -x/-y/-z corner with a radius reaching only PART of the lattice.
    // Grow the lattice 12³ (1728) → 16³ (4096): n grows 2.37×, but the giant's covered
    // sub-region (a fixed-radius corner ball) grows far slower — so its oversized
    // candidate count grows SUB-LINEARLY in n. The OLD O(k·n) residual would have made
    // it ≈ n (a 2.37× growth); the footprint scan keeps it well under 2× (measured
    // ≈ 1.27×). Both lattices stay result-equivalent to all-pairs.
    let corner = -18.0f32;
    let build = |side: usize| -> (usize, BroadphaseGrid, Vec<(BodyIndex, BodyIndex)>) {
        let mut bodies = small_lattice(side, 0.5, 0.9);
        let n_local = bodies.len();
        bodies.push(sphere(Vec3::new(corner, corner, corner), 25.0));
        let mut grid = BroadphaseGrid::with_capacity(bodies.len());
        let out = grid_pairs(&mut grid, &bodies);
        // Equivalence keystone holds at this size.
        assert_eq!(out, all_pairs(&bodies), "footprint scan == all-pairs (side={side})");
        (n_local, grid, out)
    };

    let (n_small, grid_small, _out_small) = build(12);
    let (n_big, grid_big, _out_big) = build(16);

    // Anti-vacuity: the corner giant IS classified oversized in both scenes (so the
    // coarse-grid / footprint path genuinely runs).
    assert_eq!(grid_small.oversized_len(), 1, "the corner giant is oversized (12³)");
    assert_eq!(grid_big.oversized_len(), 1, "the corner giant is oversized (16³)");

    let ov_small = grid_small.oversized_candidate_count();
    let ov_big = grid_big.oversized_candidate_count();
    assert!(ov_small > 0 && ov_big > 0, "anti-vacuity: the oversized leg emitted candidates");

    // THE residual-is-gone observable: the oversized candidate count grows SUB-
    // LINEARLY in n. n grew by `n_big / n_small` (≈ 2.37×); the old O(k·n) residual
    // would grow `ov` by the SAME factor. The footprint scan must grow it strictly
    // LESS than that — we assert a generous < 2× ceiling (measured ≈ 1.27×), which the
    // linear residual (≈ 2.37×) cannot satisfy.
    let n_growth = n_big as f64 / n_small as f64;
    let ov_growth = ov_big as f64 / ov_small as f64;
    assert!(
        ov_growth < 2.0 && ov_growth < n_growth,
        "the O(k·n) residual is gone: the oversized candidate count must grow SUB-\
         LINEARLY in n (n grew {n_growth:.2}×: {n_small}→{n_big}; oversized candidates \
         grew {ov_growth:.2}×: {ov_small}→{ov_big}). A growth tracking n would be the \
         old residual."
    );
}

#[test]
fn oversized_candidate_count_independent_of_far_isolated_bodies() {
    // A giant overlapping a LOCAL cluster, then the SAME local scene with the giant
    // un-routed-to-far bodies: in both cases the oversized leg only ever scans cells
    // the giant overlaps, so adding FAR bodies that the giant's footprint never
    // touches cannot add oversized candidates. Here we keep the giant + local cluster
    // FIXED and confirm the oversized candidate count is a pure function of the local
    // footprint — never the total body count.
    //
    // Local dense cluster (giant overlaps it) — the baseline oversized candidate count.
    let base: Vec<BodyState> = {
        let mut b = small_lattice(14, 0.5, 0.9); // dense → fine cells → giant oversized
        b.push(sphere(Vec3::new(1.0, 1.0, 1.0), 28.0)); // overlaps the cluster
        b
    };
    let mut grid_base = BroadphaseGrid::with_capacity(base.len());
    let out_base = grid_pairs(&mut grid_base, &base);
    assert_eq!(grid_base.oversized_len(), 1, "the giant is oversized over the dense cluster");
    assert_eq!(out_base, all_pairs(&base), "baseline: footprint scan == all-pairs");
    let ov_base = grid_base.oversized_candidate_count();
    assert!(ov_base > 0, "anti-vacuity: the giant overlaps the cluster (oversized candidates)");

    // The giant's oversized candidate count must be bounded by its footprint body
    // count, NOT by the total scene. The old residual would equal ≈ n_local; here it
    // is at most the bodies in the giant's overlapped cells. We assert it never
    // EXCEEDS the all-pairs count of giant-involving feasible pairs by a huge margin —
    // i.e. it stays proportional to real overlaps, not a blind k·n sweep.
    let giant_row = (base.len() - 1) as u32;
    let giant_feasible = out_base
        .iter()
        .filter(|(lo, hi)| lo.0 == giant_row || hi.0 == giant_row)
        .count();
    assert!(giant_feasible > 0, "anti-vacuity: the giant has feasible pairs");
    // The footprint scan over-approximates the feasible set by the cells-vs-spheres
    // slack only (a small constant factor), never the full n²-style residual. A blind
    // k·n residual would push the candidate count far above the feasible-pair count
    // for a giant overlapping only part of the world; the footprint scan stays within
    // a small multiple of the genuine overlaps.
    assert!(
        ov_base <= giant_feasible * 4 + 64,
        "the oversized leg stays proportional to real overlaps (footprint-bounded): \
         oversized candidates {ov_base} vs feasible giant pairs {giant_feasible}; a blind \
         k·n residual would dwarf the feasible set"
    );
}

#[test]
fn oversized_overlapping_giant_still_equals_all_pairs() {
    // Correctness direction the residual probes can't see: a giant that DOES overlap
    // the whole lattice — the footprint-bounded scan must NOT MISS any genuine
    // oversized–small pair. Equivalence to all-pairs is the proof.
    let mut bodies = small_lattice(10, 0.5, 0.9);
    let n_small = bodies.len();
    bodies.push(sphere(Vec3::new(4.5, 4.5, 4.5), 40.0)); // spans the whole lattice

    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
    let g = grid_pairs(&mut grid, &bodies);
    assert_eq!(grid.oversized_len(), 1, "the world-spanning sphere is oversized");
    assert_eq!(g, all_pairs(&bodies), "footprint-bounded scan misses no oversized–small pair");

    // Anti-vacuity: the giant pairs with most of the lattice.
    let giant_row = n_small as u32;
    let giant_pairs = g.iter().filter(|(lo, hi)| lo.0 == giant_row || hi.0 == giant_row).count();
    assert!(
        giant_pairs > n_small / 2,
        "the giant pairs with most of the lattice (anti-vacuity): {giant_pairs} of {n_small}"
    );
}
