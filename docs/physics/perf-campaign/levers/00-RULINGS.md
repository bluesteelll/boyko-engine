# Lever designs after P0 — orchestrator rulings (2026-09-19)

Each lever folder holds its research, its design (rev 1) and the architecture review of rev 1. This file
records what was decided on each review. The P0 measurement behind all three is
`docs/measurements/2026-09-19-physics-p0/ANALYSIS.md`; the campaign rulings are `../00-RULINGS.md`.

## L5 — parallel narrowphase (with L4 and L2 as the lane's first commits): APPROVED with changes

The review found no blocking remark and no determinism hole. Closed by ruling; the implementing developer
applies these to rev 1:
- **W1:** the algorithm matches D4's inline conditions, including `lanes < 2`; a narrowphase twin of
  G-L4-1 (1 worker, flag on: `narrowphase_dispatches` delta = 0) is added, with the mutation "drop the lanes
  term" required red; the runner's copy of `chunk_count` carries the same term.
- **W2:** the J-P1 runner row is retired in C1; ω₁ comes from a zero-work spawn/join microbench from then
  on (the campaign rulings allow it).
- **W3:** the compared axis-table state is `(key, axis)` per slot plus `occupied`, hashed by field — never
  raw bytes (the entry has padding). The lemmas speak of "table state".
- **Optional, all adopted:** O1 (open the dispatch zone after the `c < 2` decision), O2 (assert the pool
  workers share one MXCSR with each other and the default; no spawner-side FTZ mutation), O3 (the
  load-clear / grow non-vacuity items get a crate-internal observable), O4 (`--expect-pose` from C0/C1
  on), O5 (do not reach for the scratch-id fallback; the tiling assert stays), O6 (the broadphase order
  assert becomes strict `<`), O7 (the census citations and the debug S1c pin move too).
- **Open question 1, the +1 scope per step (D8):** accepted. A `pool.scope` costs one boxed `ScopeShared`
  and a block chunk; the solver already pays 133 per step and the census pins them exactly. The narrowphase
  adds one, pinned in the same census, until the allocator campaign removes scope allocation for all of
  them (KC-05, rung D-M2). This is a ruled exception to "no allocation on the hot path", not a precedent
  for new per-entity allocation.
- **Open question 2:** a parallel compaction is built only under the campaign build-if rule (≥ 5 % of T(W)
  and above the SE bar), not at the plan's 1 %.
- **Open question 3:** L2 is exempt from the end-to-end T(W) gate, because no measured row runs `Auto`; its
  gates are the structural ones in the plan.

## Broadphase redesign (`BroadphaseKind::Tree`): REVISE (rev 2) before implementation

No blocking remark; the exactness and determinism core holds (only the exact AllPairs set is bit-identical,
because the pair count sizes the axis cache). Five Important remarks are substantive enough for a revision:
- **W1:** the sleep hint is gated on `cfg.sleeping` and on the mask covering the current N; a structural
  gate `sleeper_rebuilds == 0` on a sleeping-off J run, with "hint not gated" required red.
- **W2:** a complete trigger list for the persistent static and sleeper sets; the cursor stamp; upper-bound
  gates (`static_rebuilds == 1` over 600 J steps; a bound after R-S freezes); rebuild costs at 1,240 / 10k /
  100k; the `row_identity_churn` arms in the gates; and a decision whether the bit+class verify is the sole
  correctness authority (then the remap trigger is dropped and M5 re-specified). Churn must not dissolve
  the sets in a spawn-heavy game.
- **W3:** the parallel query wave (C2) is gated on its own (serial tree against parallel tree, same binary,
  W=8, SE gate) and built only if it clears the build-if rule at that point; the design's own figures put it
  near 1 % of T(8), so expect it to be deferred.
- **W4:** negative radii in G1 (both negative, mixed), so M2 can go red.
- **W5:** per-path structural expectations for every new zone and counter, in the plan and in the runner's
  `check_step`.
- Optional O1–O3 adopted (the allocation wording; a per-row fallback for non-finite rows instead of a
  whole-step AllPairs; the scratch-id cohort placement as the review computed it).

## L10 — sleeping on by default, and the frozen-pair skip: REVISE (rev 2) after the broadphase rev 2

- **B1, the blocking remark — ruling:** the public views stay LOGICAL. `ContactPairs`, `Manifolds`,
  `ConstraintGraph`'s island queries, `IslandSleep::is_island_frozen` and `WarmSeedStats` report sleeping
  contacts and islands exactly as they do with sleeping off, the way Box2D v3 and Rapier expose sleeping
  contacts. The internal solve consumes only the awake stream; the logical views are provided WITHOUT
  copying the sleeping store every step (iterate the awake stream and the sleeping store together, or keep
  a logical index). No test is loosened: the suites that read these views keep their assertions.
- **W1:** the coloring/write-guard predicate pair (`contact.rs:20-40`) stays identical by construction, or
  the invariant is enforced in release.
- **W2:** every runtime transition is defined (sleeping on/off, each mode change), "resting" requires the
  stream cursor, and the runtime toggles join S3.
- **W3:** the mirrored axis `set`s for clean islands run after `begin_frame`, and a Rows-step grow-clear
  while an island is clean joins S3.
- **W4:** S1–S3 run under both broadphase kinds with the parallel emit, plus an SDF pile with a mid-run
  field edit and a coupled soft body.
- **W5:** a census scene that wakes, re-freezes and churns rows after warm-up, and a mutation in the restore
  path.
- Optional O1–O5 adopted.
- Because L10's frozen-pair skip in the broadphase is the broadphase design's C5, rev 2 of L10 is written
  against the broadphase rev 2.

## Rev 2 (2026-09-19): both designs CLOSED by ruling

Both second reviews found no blocking remark (broadphase: 2 Important; L10: 3 Important), each with an
unambiguous "needed". The designs are closed; the implementing lane folds these in as rev 2.1 before its
first commit. Order of lanes: L5 (approved above) → broadphase → L10.

**Broadphase rev 2** (`broadphase/04-DESIGN-REV2.md`, review `05-REVIEW-OF-REV2.md`):
- **W1 (the patch rule cascades on a high jumper that is the min endpoint of ≥ 2 entries):** the keep test
  must not keep an entry with a non-monotone endpoint (jumper rows identified from `prev_row` during the
  verify); D3.5 states the true bound; a G4 maintenance arm at 10k / 100k moves one member to the top row
  with ≥ 2 higher-row partners under the same 2× stop rule; a unit test pins `a` for that case.
- **W2:** anchor A is defined over the dynamic rows (or "Q is empty") and its existence is asserted
  (A ≤ 300); a toggle-off script (sleeping on until frozen, then off with no row change) asserts
  Δ`hint_candidates` == 0 and Δ`sleeper_rebuilds` == 0, with "drop the `cfg.sleeping` term" shown red; a void
  in a cargo test is a red, never a pass.
- Optional remarks adopted where cheap.

**L10 rev 2** (`L10-sleeping/04-DESIGN-REV2.md`, review `05-REVIEW-OF-REV2.md`):
- **W1:** the Tree's hint is PRE_HELD minus the prologue restore list (or `release` runs for every restore on
  a Tree step); a debug assertion of Invariant V over `withheld` after the kind arm; the mutation
  "prologue-restored rows left in the hint".
- **W2:** restored runs are looked up by a key translation does not change (the t−1 ordinal through
  `prev_row`, or local-index pairs), or every pair of a jumper-restored record is recomputed; the A2.5
  filter runs before the A2.4 merge; the mutation "restored run searched by translated ordinal".
- **W3:** the classify cost is bounded (R3 only for rows a record, a CAND island, the D2 scan or the SL list
  can reach; skip the pass when nothing is held or CAND; or a narrower compare for static rows); a
  static-heavy "not claimed slower" arm (J plus 10k statics).
- Optional remarks adopted where cheap.
