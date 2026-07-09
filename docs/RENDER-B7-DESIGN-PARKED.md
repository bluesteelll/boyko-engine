# Render B7 — Lipschitz/bound fold-pruning (DESIGNED + PARKED, measurement-deferred)

> Status: **DEFERRED by decision** (architect + internal-critic both recommend defer; orchestrator concurs — measurement-driven, not doctrine). The design below is adversarially reviewed and implementation-ready; build it the day a profile shows the `sdf_edit_list_x8` fold ALU is a measured bottleneck AND an n ≫ 16 (256–4096-edit) workload exists. Until then it is plausibly **net-negative** (see Defer rationale).

## Defer rationale (why not now)
1. `MAX_SDF_EDITS = 16` — the 256–4096-edit regime the prune targets does not exist yet; at n=16 the per-edit overhead dominates the saving.
2. Exact-prune is bit-safe **only for hard-union** edits (smooth-union saturates to `acc` only at a ULP-wide IEEE band boundary that isn't analytically expressible; intersect/subtract have no finite per-edit *upper* bound, so a far point can still dominate).
3. The skip predicate `dlo > acc` fires only for edits *far* from the query `p`; the physics narrowphase that calls `sdf_edit_list_x8` queries *near* contact surfaces by construction (small `acc`) → the skip almost never fires → overhead paid, saving rarely captured → plausibly net-negative on the exact target workload.
4. O1 (in-solver AVX2) + O2 (uniform-grid broadphase) moved the physics bottleneck off the scalar/all-pairs inner loop; the fold is likely no longer hot. Profile before any code.

## Core design (adversarially reviewed, ready to implement)
- **D1 — per-edit, running-`acc`-dependent EARLY-SKIP inside the fold.** Not a static cull, not a reorder, no new SSBO/binding. The skip test runs against the live `acc` at edit i≥1; each edit is either folded exactly as today or skipped (acc untouched). The threshold is the running accumulator, so a precomputed spatial structure is the wrong direction.
- **D2 — prune HARD UNION ONLY (the per-op exact-identity analysis).** Containment lemma: skip an edit only if `combine(acc,d,op,k).to_bits() == acc.to_bits()`.
  - UNION hard (k≤0): `min(acc,d) == acc` to the bit iff `d > acc` *strictly*. Skip iff `dlo > acc` (since `dlo ≤ d`). **YES.**
  - UNION smooth (k>0): saturates only when `hh` rounds to exactly 1.0 — a ULP-wide shell not analytically expressible. **NO** (folds verbatim).
  - INTERSECT: needs an *upper* bound on `d`; none exists for far `p` (`d→∞`). **NO.**
  - SUBTRACT: technically `dlo > -acc` works but fires ~never near surfaces; scoped out of v1.
- **D3 — sqrt-free L∞ lower bound `dlo` (cheaper than the exact primitive it gates).** Sphere: `max(|dx|,|dy|,|dz|) − r ≤ length − r`. Box: `max(|dx|−hx,|dy|−hy,|dz|−hz) ≤ sd_box`. Reuses already-loaded center/params; no new memory traffic. (`fl(L∞) ≤ fl(L2)` holds by round-to-nearest monotonicity — critic-verified.)
- **D4 — x8 lockstep via containment.** Per-lane predicate; skip the edit only if all 8 lanes AND (movemask==0xFF); when lanes disagree, fold for all (the would-skip lanes have `d>acc` ⇒ `min==acc` bit-exact). x8 == scalar per-lane regardless.
- **D5 — GPU stays the verbatim frozen oracle.** A per-pixel skip can't elide a subgroup edit-iteration (neighboring pixels have different `acc`) → net-negative + divergence. A GPU B7 needs a subgroup-uniform tile bound (P4b `TileBound` pattern) — separate feature, out of scope.
- **D6 — 0%-gate by compile-time symbol selection.** `sdf_edit_list_x8_pruned` is a SEPARATE function; the current `sdf_edit_list_x8` stays verbatim (the OFF symbol → byte-identical machine code). A `const B7_ENABLED`/cfg selects.
- **Frozen-region resolution:** the frozen contract is about the VALUE, not the source text. B7 v1 touches ONLY `boyko_physics/src/sdf_simd.rs` (`sdf_edit_list_x8`, a non-oracle already diffed vs the scalar). The GPU `sdf()` + the CPU scalar `sdf_edit_list` (the golden oracle + physics source) stay verbatim. A future GPU/scalar B7 would require all three (GPU/scalar/x8) to change in lockstep and re-pass the bit-exact gate.
- **Fold-order pin:** edit 0 is the un-skippable hard seed; a skipped hard-union edit is an identity transition (`min==acc`), so survivors are a literal subsequence → bit-identical accumulation. ∎

## Gate list (when built)
Bit-exact proptest `sdf_edit_list_x8_pruned == sdf_edit_list` (scalar) over hard-union far edits (skip-firing) + mixed op-lists (assert non-union never pruned) + f32::MAX/inf-adjacent/±0 in the prune path; anti-vacuity (skip_count>0 AND pruned==unpruned); OFF-build asm byte-identity; `cpu_gpu_sdf_agreement` still green; Miri (scalar prune); criterion A/B across n∈{4,16,64,256} × skip-fractions incl. the **near-contact** distribution (the load-bearing bench); a `debug_assert!(dlo <= edit_distance)` tripwire.

## Pre-build gating questions (answer before option-a code)
1. Profile post-O1/O2: is the `sdf_edit_list_x8` fold ALU a measured bottleneck, or is the time in broadphase/solver/the gradient stencil?
2. Does an n ≫ 16 (256–4096-edit) scene exist / is it imminent?
3. Measured **near-contact** skip-fire rate (should be ~0; bench the realistic distribution, not synthetic all-far).
