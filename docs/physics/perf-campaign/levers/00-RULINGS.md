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

**T(W) gate measured 2026-09-21** (window 3, `docs/measurements/2026-09-21-physics-window3/analysis.md` §3; the
2026-09-21 block of `docs/MEASUREMENT-QUEUE.md` §10). On the shipped default row (`--cfg default`: simd on,
AllPairs under `Manual`, sleeping off) C4 `de06b6c9` against C2 `aac562a7` reads **−2.435 ms at W=8 (−31.3 %)**,
claimed under range, IQR and SE (bars 15.7 / 5.3 / 3.0 %), and the same-binary A/B (`--parallel-np off` against
on, W=8) reads **+2.626 ms (+49.1 %)**, claimed under all three (36.3 / 9.6 / 7.0 %) — L5's rule (realized
ΔT(8) ≥ 1.33 ms, claimed) is cleared by 2×, at the top of the 2.4–2.6 ms prediction. At W = 2 / 4 / 16: −13.6 /
−25.5 / −32.0 %, claimed under IQR and SE (the min–max reading is inflated by one contaminated pass-0 process per
cell). One pose `0x32d5e235342b4143` across C2, C4 and every W (77/77 timed processes plus the untimed 500-step
gate with its 501-step red control); equal knobs across the two binaries (`--parallel-np off` against C2, W=8)
+2.4 %, not claimed. **At W=1 C4 reads +1.34 % (+0.142 ms) over C2, claimed under SE only** (bars 4.5 / 1.5 /
1.0 %; K = 5 and 6 against the design's K = 12 at W=1): a one-worker pool takes the serial loop by design
(`one_worker_parallel_narrowphase_runs_the_serial_loop`), so this is a C3 + C4 binary difference, not the flag,
and whether it is "a claimed regression at any W" turns on which spread the claim rule means — `../00-RULINGS.md`
names SE, §10's RESULT block keeps range / IQR with SE beside them and calls the question OPEN. That is the
orchestrator's call; a same-binary `--parallel-np off` W=1 row at K = 12 (about 2.5 minutes) settles it. Not
exercised by the window: the design's own row for the rule (J-A, cfg-A, at C2 and C4), the armed rows (I(W),
E(W) at W = 2, 4, 16), the R rows and the J-C canary on C4 — those gate rows stay open.

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

**G4 / G5 measured 2026-09-22 (window 4, `docs/measurements/2026-09-22-broadphase-tree/analysis.md`; the 2026-09-22
block of `docs/MEASUREMENT-QUEUE.md` §11; C3 binary `a46b8287`, K=6, Jolt v5.6.0 only). Every directional gate passes:
the Tree is claimed faster than AllPairs on the same binary at every W on J cfg-A (−8.6 … −24.4 %), on the shipped
default row (−14.5 % at W=1, −32.6 % at W=8), on R (−11.5 / −22.5 %) and on all 12 churn arms, S16 unchanged, one
pose per scene, structure clean, canary seen; the headline Δbp(8) = 1.488 ms clears the 1.03 ms bar; `T-D-tree` at
W=8 reads **3.699 ms = 1.440× Jolt v5.6.0** (0.918× at W=1; a projection until the default ships). **But the stop
rules FIRED** — G4 rule 1 `J snapshot` 0.443 ms > 0.30, the G5 tree-span gate 0.476 / 0.466 ms > 0.36 / 0.35, rule 6
`compaction` at every m and rule 9 `high jumper` at 100k — all on one quantity, the J-scale query at 334 ns per row
against the design's 100–150 — **so C4 (the default flip) is DEFERRED by the recipe's letter until the query-cost
investigation of `analysis.md` §8 has run and the J snapshot cell and the armed rows are re-taken.** Constants:
`TREE_BRUTE_MAX_ROWS` = 64 (derived, unchanged); `AUTO_TREE_LO/HI` 64 / 256 by the recipe's grid rule (126 / 140 by
L2's procedure), committed only after the refinement run between 64 and 256; `ADMIT_BUILD_RATIO` and C2 (D6)
DEFERRED — the recipe's `c_build` formula contains c_q and must be corrected first, and C2 is built iff the
post-investigation t_q(J, W=1) ≥ 0.235 ms on the default row (0.316 on cfg-A) at E = 0.68. The investigation can
change the flip's size, not its sign, unless it finds the excess is work the design forbids. ⚠ *2026-09-25, ruling after window 7 wave 2 (orchestrator): **the tree thresholds are `TREE_BRUTE_MAX_ROWS` = 144 and `AUTO_TREE_LO/HI` = 144 / 152**, by the recipe's rule (`g4_g5_recipe.md:131-132`), on both families (the L2 log-log procedure beside it reads 150 / 135) — a code change for the tree lane, not made by the record. This is the refinement run the constants above waited for (`docs/measurements/2026-09-25-physics-window7/wave2/`, queue section 15.5): an instrument of the trunk `93b2615b` with only `G4_SIZES` widened (one line), `all_pairs` against the shipped LeafList `tree` at 12 sizes between 64 and 256, K = 3. Both families are monotone: `all_pairs` is not claimed slower up to 144 (all_pairs/tree 1.0076 uniform, 0.9929 disparity, neither claimed) and the tree is claimed faster from 152 on (1.0482 / 1.0340, all three readings). The log-log crossovers are 142.6 / 145.4, up from window 4's 111 / 138, against `c3b/design.md:244`'s "the crossovers move down"; the same-binary RowWalk bridge puts most of that rise in a same-kernel term (all_pairs/tree_rowwalk 11–18 % below window 4), not in F1.*

**The headline with the Tree, 2026-09-23 (window 5, `docs/measurements/2026-09-23-combined-tree-l11/analysis.md` §1; the
2026-09-23 block of `docs/MEASUREMENT-QUEUE.md` §13; tip `cbd86a65` = the Tree C1+C3 + L11 C0–C3 + A1b + A3, K=6, Jolt
v5.6.0's window-3 cells only, never re-run).** `--cfg default --broadphase tree` reads 7.106 ms at W=1 = **0.723× Jolt
v5.6.0** (claimed under IQR and SE; min-max fails on Jolt's own window-3 cell, which holds one 11.364 ms process) and
2.716 ms at W=8 = **1.057×, not claimed either way** — the two cannot be told apart at eight workers; 0.807× / 0.926× at
W = 2 / 4. The same binary's shipped AllPairs default is 0.894× / 1.672× at W = 1 / 8, and the Tree beats it on that
binary at every W (−19.1 % / −36.8 % at W = 1 / 8, all three readings). Per manifold the Tree row is 1.38× / 1.96× Jolt
(boyko carries 1.88× fewer manifolds). The cross-window W=8 ratios carry that window's bridge-2 qualification (its
parent's W=8 cell sat +11.5 % above window 4b's on unchanged solver code). By the arithmetic of `analysis.md` §1 the
query's excess over the design (334 ns per row against 100–150: 0.225–0.287 ms at W=8) is larger than the whole
+0.147 ms W=8 gap to Jolt. **C4 (the default flip) stays DEFERRED:** no Tree row in window 5 was armed, so neither the
G4 `J snapshot` cell nor the G5 Tree span was re-taken, the query-cost investigation of window 4's `analysis.md` §8 has
not run, and the stop rules of 2026-09-22 stand. ⚠ *2026-09-24, ruling after window 6 (orchestrator): **the runner gains `--bp-kernel` before F3's window.** Window 6 timed C3b's query kernels across two binaries, its parent and its tip, because the runner has no kernel switch (`c3b/design.md:253`, `:387`), and the tip's armed W=1 t_q read 0.2102 ms in one block and 0.2354 ms in a later block of the same window, with the cause unknown (window 6 `analysis.md` §3.2, §7 item 3; FOLLOW-UP item 8, which leaves the flag to the orchestrator). So `benches/jolt_parity_pyramid.rs` gains `--bp-kernel`, which selects the query kernel inside one binary, before the quiet window of F3 (the reserve kd median-split leaf order), so that F3's armed A/B is timed same-binary. The flag changes no default and no pose.*

**C3b SHIPS and F3 is TAKEN UP, 2026-09-24 (window 6, `docs/measurements/2026-09-24-physics-window6/analysis.md` §3; the
2026-09-24 block of `docs/MEASUREMENT-QUEUE.md` §14.3; C3b's parent `6dd1f916` (query RowWalk) against its tip `983480a9`
(LeafList), interleaved, K=6).** On the armed `T-A-tree-armed` row the query t_q falls 0.4044 → 0.2102 ms at W=1 (0.520×)
and 0.4126 → 0.2129 ms at W=8 (0.516×), claimed under all three readings, and criterion on the tip reads LeafList/RowWalk
0.330 on J: the kernel ships. The G5 Tree span (0.271 / 0.276 ms against 0.36 / 0.35) and G4 rule 1 (`J snapshot` 0.1446
≤ 0.30 ms) are now cleared, and Δbp(8) = 1.698 ms against the 1.03 bar. The same armed cell re-run in a later block of
the same window reads 0.2354 ms at W=1 (+11.96 %, claimed; W=8 reproduces), so t_q > 0.21 ms is claimed there and
attribution A is refuted; c_q = 170–190 ns per queried row is above 150 ns in both blocks, so F3, the reserve kd
median-split leaf order (`c3b/design.md:168-172`), is taken up. **C2 is flagged, not built:** its letter (t_q(J, W=1) ≥
0.235 ms) is not robust — 0.2102 in one block (−10.5 %, claimed below) and 0.2354 in the other (on the bar, not claimed)
— and D6 fires in both blocks (6.9 / 7.3 % of T(8)); the analysis recommends deciding it after F3's re-time. The default
flip (C4) is not decided by this line: rules 6, 8 and 9 remain (`c3b/design.md:237-241`). ⚠ *2026-09-25, ruling after window 7 wave 2 (orchestrator): **C2 is NOT BUILT.** The design's letter row is the default row armed (`c3b/design.md:231`, `:372`), and there C3b's tip reads t_q(J, W=1) = 0.2079 / 0.2078 / 0.2078 ms in two separated blocks and pooled (K = 6 / 6 / 12), −11.5 % below 0.235, claimed below under min–max and SE in each (`docs/measurements/2026-09-25-physics-window7/wave2/`, queue section 15.5); c_q = 167.6 ns per queried row. The cfg-A row window 6 read is recorded as context: 0.2459 / 0.2486 / 0.2471, on the bar (claimed above under SE only). Window 6's +11.96 % block shift is neither a launch-layout term (padded against plain +0.36 % pooled, not claimed) nor a within-run block term (+1.12 %, not claimed); between the windows the cfg-A query moved on both kernels (the parent 0.4044 → 0.4938 ms on the same binary and row), while the kernel ratio held (0.520 → 0.500) and the default row did not move (0.2043 in window 7 P4, 0.2078 here).*

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

## Per-contact parity (owner, 2026-09-19): L9 and L11 designed and CLOSED by ruling

The owner set the goal: parity with Jolt per contact, not only per step. Per manifold at W=1 the solve is
already at parity (boyko with simd 1.24 µs against Jolt 1.30–1.49 µs); the gap is collision detection
(broadphase 0.465 + narrowphase 0.695 µs per manifold) and the serial solve setup (0.425 µs). Two new designs,
each with research and review (no blocking remark in either):

**L9 — contact reuse** (`L9-contact-reuse/`): L9a exact (early SAT exit + a cached separating axis, bit-identical);
L9b reuses a touching box pair's features within a tolerance and refreshes separation from the current poses
(value-changing, owner-approved). No "no contact" caching and no speculative margin (it would inflate the
manifold count). Rulings:
- **Build-if (review OQ1):** the owner's per-contact goal overrides the W=8 build-if that §9 applied; L9 is
  built and gated on its W=1 and per-contact gains (L9a gets a realized-gain rule on J-D, O6).
- **W1:** the axis-hint table stays keyed for hit pairs on every step where the key set changes (prefetched,
  cleared, grown) — the tag carries the axis through the row-translated join — so hysteresis survives and
  L10's E5 mirror stays equal to Off.
- **W2:** the spurious-wake path is CLOSED: records survive a row-order flip (the pose check is by the larger
  and smaller body; only `REF_IS_B` flips); a box-pile arm moving the support by less than τ_eff joins the A4
  tests so "all points lifted ⇒ no manifold ⇒ wake" can go red.
- Optional O1–O7 adopted (τ_eff also clamped by the smallest half-extent; record memory stated; the
  `row_frames` fill skips unread rows or runs in the parallel phase; the gate fixes — slab as body B, M-c3
  re-specified, tolerance scaled by the incident body's extent; tags written for every pair kind every step;
  one tag layout shared with L10). The Rapier description is corrected (parry keeps local normals, cos 1°,
  1 mm). `pairs_prev` is stamped with the gather sequence (OQ4).
- **C0 reading and the C4 decision (2026-09-24, window 6; `docs/measurements/2026-09-24-physics-window6/`, queue section 14.2; trunk `4db26681` = L9 C0–C3, reuse switchable in one binary, K=6): BUILD C4.** (a) The class-bench band (`02-DESIGN-REV1.md:380`) is CLEAR: low 1.077 ms, high 1.666, the low end claimed above 1.0 under all three readings (touching 435.6 ns per pair; the separated class is already post-L9a, so the band prices L9b). (b) The realized-gain rule (`:407-410`) passes: J-A reuse off against on at W=1 over [100,500), ΔT(1) = 1.845 ms (18.161 → 16.315, −10.2 %), claimed under all three readings, against a bar of 0.6 × 1.499 = 0.900 ms (2.05×); the armed Δt_np(1) is 1.725 ms. G-L9b-6 had passed (h_J = 0.9897), and G-TW's other off/on rows (J-D and R at W 1/8, J-A at W 2/4/16) are all claimed faster, with no claimed regression at any W. C4 (`contact_reuse = true`, τ 1 mm; `:161-165`, `:384`) is the only value-changing commit: under each file's own rule it re-pins the J/R/J-Son/R-S pose fixtures (J500 `0x32d5e235342b4143` → `0x30c5438bc6ad9ffa`, R1100 → `0xc8bbe34cf6a8afc6`), `PINNED_FINAL_HASH`, `A7_R1_D_MAX_BITS` (the reuse-off run stays at `0x3a3c_3896`), A7-R1's docs, A7-R2's freeze step, G2/G7/G8, H8's manifold count and the box-pile goldens, and it reaches the tree lane's J-pose pins and u/phys-l10's lane-base fixtures; from then on cross-window bridges use reuse-off rows. Still owed on the C4 binary: G-TW's canary, J-D and R at W 2/4/16, the armed W8 per-contact table and the formal A/B.
- **G-TW on the C4 binary: PASS, so C4 can merge (2026-09-25, window 7; `docs/measurements/2026-09-25-physics-window7/`, queue section 15.2; `989ca0f0` = trunk `6c40b3e6` + C4, both arms from one binary via `--contact-reuse`, K=6).** The realized-gain rule (`02-DESIGN-REV1.md:410`) passes on the C4 binary: J-A reuse off against on at W=1 over [100,500), ΔT(1) = 1.937 ms (18.413 → 16.475, −10.5 %), claimed under all three readings, 2.15× the 0.900 ms bar. No on/off ratio is claimed slower at any W (J-A, J-D and R at W 1/2/4/8/16, over [0,500) or [600,1100), and the J [0,100) witness). The `on` rows read one hash per row across W, and the `off` rows read C0's (`:414`). The armed per-contact table (`:413`) reads the narrowphase at 2.611 → 0.751 ms at W=1, 0.168 µs per manifold. Two limits are recorded, not waived. First, the canary (`:412`) is seen under the design's SE claim rule (`:407`: a rise of +5.38 % against an SE bar of 2.40 %) but not under the window's two-spread rule (min–max bar 10.96 %), the same split window 6 accepted for G9's parent canary. Second, the window's browser-loaded pass 0 left G-TW's min–max bars at 26–73 % for W ≥ 2, where the quiet pass alone reads every row faster. Whether the canary is re-read alone in a quiet window (about 3 minutes) is the orchestrator's call (window 7 `analysis.md` FOLLOW-UP 1). Recorded for the lane: narrow colours rise ×6.5 with reuse on (+0.15 ms, claimed). ⚠ *2026-09-25, ruling after window 7 wave 2 (orchestrator): **G-TW's resolution is DEMONSTRATED, and the canary under SE above stands.** On the same C4 binary and G-TW's own J-A `on` row at W=1 (unarmed, K=6, one quiet block; `docs/measurements/2026-09-25-physics-window7/wave2/`, queue section 15.5), four canary rungs at 0.5 / 1 / 1.5 / 2 × R (R = 7.542 %, G-TW's own two-spread bar in window 7) raise the step +3.64 / +7.44 / +11.20 / +14.87 % (rise/injected 1.01–1.05) against min–max bars of 2.6–2.7 %, and every rung is SEEN under all three readings. The smallest rung seen with every larger one seen is **0.5 R = 3.77 % of the step**. This covers the W=1 J-A gate only; G-TW's W ≥ 2 bars (26–73 % in window 7) are not re-measured.*

**L11 — the solve setup per contact** (`L11-solve-setup/`): warm impulses stored per manifold (no per-point hash
probe/insert), cohort-shaped constraint blocks instead of the 26-column push, SIMD warm apply; bit-identical;
predicted setup 0.425 → 0.10–0.21 µs per manifold. Rulings:
- **W1:** `plan` names the whole equal-key run; the P-c seed and the D3 carry go through the single lookup
  routine D2 specifies; the mutation "fill/carry scan only the last equal-key record" is recorded red.
- Optional O1–O5 adopted (the SIMD warm-apply mask is `active ∧ movable`, with a −0.0 scene; lanes ≥ nlanes
  read no body row, with a Miri multi-thread case; the restitution predicate is `!(e <= 0.0)`; the test-port
  inventory lists what is deleted; the L10 mapping uses t−1's stream index).
- **OQ1/OQ2:** measure the C1 share and the kernel's gather/scatter share before the C2/C3 rewrites.
- **Ruling after C2 (2026-09-21, the census adjudication):** the S1c release pins moved with C2 — chunk 230 → 134,
  dispatch MAX 365 → 269, scope unmoved, the debug pins unmoved — because the colour task's cell shrank with the
  view it captures (280 → 120 B; 34 cells per chunk, not 14) and no colour scope needs a second block any more.
  This is the census's own re-pin case (the 17 × 256-step long run against a C1 control, the per-frame structural
  assertion, the attribution binary's A/B arm, then the envelope with no headroom either way), NOT a widening; the
  design's G7 sentence "scope/chunk pins unchanged (C1–C3)" is corrected in place. Row D of the attribution binary
  is not loosened: it pins `chunk == scope` per frame at W = 2/4/8 and keeps its strict lane-growth assertion where
  the model predicts growth — on the scalar cut walk (`simd_solve = false`) at W=8 against W=4 — because on the
  shipped kernel a colour spawns at most 32 tasks at any lane count (the ruling's "48 at W=8" was the ceiling
  `lanes × CHUNKS_PER_WORKER`, not the count the cohort-snapped cut walk produces).
- **G9 on C2 (2026-09-22, window 4b; `docs/measurements/2026-09-22-l11-solve-setup/`, queue section 12):** C1+C2 against C0 (`f8873aae` against `146a1125`), K=12 interleaved under the P0 protocol, passes every realized-gain gate the design set on this tip — T(1) `J-As` −1.996 ms (10.693 → 8.697; bar −0.81), T(8) −1.103 (5.342 → 4.240; bar −0.61), solve_build −0.573 / −0.542 at W=1 / 8 (bar −0.29), store −0.141 / −0.134 (bar −0.11); wide colours −27.7 % / −28.5 % and cfg-A faster at every W, so both "not claimed slower" gates hold; the setup 1.643 → 0.832 ms, serial at both W; 0.885× / 1.650× Jolt v5.6.0 at W=1 / 8 (1.69× / 3.07× per manifold); poses and counters bit-identical on both binaries. **C3 still owes the warm_apply gate (−0.19 ms): the tip reads −0.092 from C1's merge-join, 0.519 ms against C3's 0.15–0.30 target, and the design's setup band (0.44–0.93) is met at 0.832 only with C3 outstanding;** C4's in-binary A/B at W=8 stays conditional on D10 (the setup is 20 % of the tip's 4.24 ms step). Facts for the design, not reds: on the all-frozen `R-S` the tip's solve_build is +16 µs per step and its store, 0.128 ms, sits above the predicted 0.05–0.10.
- **G9 on C3 (2026-09-23, window 5; `docs/measurements/2026-09-23-combined-tree-l11/`, queue section 13):** C3 against its parent (`cbd86a65` against `0ca312bd`, the lane after the integration-line merge: C0–C2 + the tree broadphase, A1b, A3), K=6 interleaved under the P0 protocol, **passes the warm_apply gate**: armed `J-As-a` at W=1 0.5285 → 0.2960 ms, Δ −0.2325 against the −0.19 bar (the design's listed bar, already the 0.6× realized-gain form; not discounted a second time), margin 0.043, claimed under all three readings, the worst process pairing still −0.229; −0.2348 at W=8 (0.5391 → 0.3043). The tip's 0.296 ms sits inside the design's 0.15–0.30 target at its slow edge (0.74× the lower predicted Δ); the setup (build + warm + store) is 0.656 / 0.672 ms at W=1 / 8, inside the 0.44–0.93 band; wide colours are not claimed slower (kernel −0.077 / +0.0006 ms); solve_build, store, broadphase and narrowphase do not move; poses and counters bit-identical on both binaries. The step moves −0.232 / −0.216 ms on `J-As` at W=1 / 8, not claimed at K=6 (the design sets no separate step gate for C3; C1+C2 passed T(1)/T(8) in window 4b); cumulative C0 → C3, chained across windows 4b and 5: −2.23 / −1.32 ms. **Still open for C3: the rest of G9** (J-A, R, R-S, S16, W 2/4/16, K=12, the canary). C4 (D10, the parallel fill) stays conditional: solve_build(8) after C3 is 0.336 ms, 7.45 % of T(8) on `J-As`, above D10's 5 % trigger — recorded here, the build decision is not taken by this line.
- **G9's remainder on C3 (2026-09-24, window 6; `docs/measurements/2026-09-24-physics-window6/`, queue section 14.4):** window 5's own two binaries (`0ca312bd` against `cbd86a65`), K=6 interleaved, **PASS**: no row is claimed slower under either rule — J-A −0.11 % / −0.68 % at W = 1 / 8, R −3.65 % / −4.70 %, R-S −0.46 % / −0.91 %, S16 −0.32 %, J-As at W 2/4/16 −3.72 / −4.23 / −5.34 %, J-As-a −4.07 %; warm_apply on `J-As-a` at W=1 re-reads 0.5278 → 0.2979 ms (−0.2299, claimed; window 5: −0.2325); wide colours are not claimed slower; S16's store 60 → 80 ns is two timer quanta on an unclaimed step, a reading, not a FAIL. The canary is SEEN (its span 1.0008× / 1.0006× the injection; the step rise claimed on the tip, and under G9's own SE form on the parent). ~~**G9 is not formally closed:** J-A at W 2/4/16 was not run, and the design's K=12 is not met (every cell K = 6 or 5); whether K=12 is required is an open ruling.~~ ⚠ *2026-09-24, reconcile: G9 on C3 is CLOSED by ruling, in the bullet "G9 on C3 CLOSED by ruling" below (`:217-229`): K = 6 under the window protocol supersedes the letter "K=12", a cell read at K = 5 stands, and J-A at W 2/4/16 goes to the next quiet window as a record.* Bridge 2 holds: window 4b's tip against window 5's parent on `J-As` is +0.16 % at W=1 and +1.75 % at W=8 (SE only), so window 5's +11.5 % at W=8 was mostly machine state.
- **L7 RETIRED (2026-09-23, after C3), closing the design's instruction** (`L11-solve-setup/02-DESIGN-REV1.md` D7, "Recommend retiring L7 after C3", and its interaction list, "L7: retire it after C3 (D7)"). L7 — `warm_start_apply` per wide colour, in parallel (`01-PLAN-REV1.md`, row L7) — is not built and leaves the lever set. D7 prices its dispatch at 4 × 9.11 waves × ω(8) 6.54 µs ≈ 0.24 ms, and C3 has now measured the whole warm apply it would split at 0.296 ms (W=1) / 0.304 ms (W=8) (window 5, armed, reading B). The plan's size rule (t_warm(8) ≥ 3 % of T(8)) is met at 6.7 % (0.304 of 4.510 ms), but the plan's own formula t_warm(1)·(1 − 1/(E·W)) − 4·waves·ω nets it to at most 0.296 × 7/8 − 0.24 ≈ 0.02 ms at W=8 even at E = 1 — arithmetic, not measured. Its prerequisite L6 was never built (`docs/physics/perf-campaign/00-RULINGS.md`, "Rulings after P0", 2026-09-19: "L6 is not built (8.2 % < 10 %); L7 stays gated behind L6"). No L7 code, lever directory or gate exists, so nothing is removed; a per-colour parallel warm pass comes back only as a new design with its own measurement.
- **L12 is commissioned** (the effective-mass cache per inertia epoch: 5 of 12 sweeps compute, 7 load;
  bit-identical) as its own design after L11.
- **G9 on C3 CLOSED by ruling (orchestrator, 2026-09-24, after window 6).** Window 6 ran the rest of G9 on window 5's two
  binaries and found no row claimed slower and the canary seen (window 6 `analysis.md` §4; queue section 14.4). It left
  two items open (FOLLOW-UP items 12 and 13). The rulings:
  - **K.** G9 closes under the window protocol the timing windows were ruled on: K = 6 processes per cell, and a claim
    only above twice the combined spread under both min-max and SE (the window-3 protocol block of 2026-09-19, as
    window 6 states it in queue section 14). That protocol supersedes the letter "K=12 interleaved" of
    `L11-solve-setup/02-DESIGN-REV1.md:360`. A cell is read at K = 5 when one of its slots is dropped because the
    original and its one re-run both had an after-receipt over 5 %. That is the protocol's re-run-once rule working as
    written, and the claim rule applies to the five processes unchanged. So window 6's five such G9 cells stand: R@1,
    R-S@8, S16@1 and J-As@2 on the parent, J-As@16 on the tip (window 6 `analysis.md:26-27` and its §4 table).
  - **J-A at W 2/4/16.** The three G9 cells no window has run yet go to the next quiet window: `J-A` at W = 2, 4 and 16,
    g9p against g9t, K = 6, about six minutes. They are a record against the same "not claimed slower" gate, and C3
    moves no pin, so no code waits on them.
  - **Reconcile with `u/win6`** (added 2026-09-24 by the Phase B document step's revision). `u/win6` adds window 6's
    own line, "G9's remainder on C3", above this bullet, and it says "G9 is not formally closed … whether K=12 is
    required is an open ruling". This bullet is that ruling. When both branches have merged, that sentence is struck to a pointer here (the reconcile list in
    `docs/unification/UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md` §2, after the Phase B table). ⚠ *2026-09-24, reconcile: struck at the merge of `u/doc-phase-b` into `integ/unified` (`:213`).*
- **G9's J-A record: G9 on C3 stays CLOSED (2026-09-25, window 7; `docs/measurements/2026-09-25-physics-window7/`, queue section 15.3).** The ruling above sent three cells to the next quiet window: `J-A` at W = 2, 4 and 16, `0ca312bd` against `cbd86a65`, K=6. None is claimed slower: +0.14 %, +0.02 % and −0.02 % (bars 2.5–12.5 %, not claimed under any reading). Every G9 row now has a cell and none is claimed slower, so G9 is closed on the record as well as by ruling.

**Lane order (ruled, replaces the order above):** L5 (in flight) → then, in parallel because their files are
disjoint, the tree broadphase and L11 → L9 (after L5, which owns the narrowphase system) → L10 (rev 2.2: it
adopts L11's per-manifold warm storage and L9's tag layout) → L8 (friction per patch, on L11's block layout)
→ L12. Each lane is gated end to end and per contact under the P0 protocol.

**Per-contact parity against Jolt v5.6.0, re-read in one window (2026-09-25, window 7; `docs/measurements/2026-09-25-physics-window7/`, queue sections 15.1 and 15.4):**
- **The Jolt headline does NOT hold under its two-block rule.** The rule: claimed pooled AND in block A AND in block B, in the same direction (window 6 `analysis.md` FOLLOW-UP 15). The row is our default (trunk `93b2615b`, `--cfg default --broadphase tree`, reuse off) against Jolt v5.6.0, W=8, [0,500): block A reads 0.836 (not claimed under min–max), block B **0.872 (−12.8 %, claimed under all three readings)**, pooled 0.811 (SE only). It holds at no W. Block A ran with an agent session and then the owner's browser active, 5–24 % slower on every row, and a pooled min–max range spans that block shift: at W=2 both blocks claim ours faster and the pooled cell does not. Per manifold (each side's own count: ours 4,519.26, Jolt 8,489.0), Jolt is faster at every W: **1.18× at W=1 and 1.60× at W=8 in block B** (1.10× / 1.48× pooled). Scaling T(1)/T(8) in block B: ours 2.72, Jolt 3.83. The headline is re-taken after C4 merges, with both blocks quiet. ⚠ *2026-09-25, ruling after window 7 wave 2 (orchestrator): **the reading is "0.87x Jolt at W8 on the default row, reproduced in three clean blocks, NOT claimed under the window protocol"**, and the rule (claimed in every block under min–max AND SE, and pooled) is not changed after the data. Wave 2 (`docs/measurements/2026-09-25-physics-window7/wave2/`, queue section 15.5) added blocks C and D to A and B. ours/Jolt at W8, [0,500): A 0.8355 (n/Y/Y), **B 0.8721 (Y/Y/Y), C 0.8697 (n/Y/Y), D 0.8696 (n/Y/n)**, pooled A–D 0.8674 (n/n/Y): it does NOT hold. Block A ran with an agent session and then the owner's browser active (its browser pass reads a during-process witness median of 2.28 %, against 0.59–0.69 % in B, C and D). In C and D, min–max fails on one process each: in C a Jolt process at 2.7015 ms (+7.0 %) with quiet receipts (0.86 / 1.61 %); in D a process of ours at 3.1487 ms (+43.2 %) whose 5-s receipts pass (1.81 / 2.18 %) while its during-process witness reads 14.25 % (`msedge.exe`), which the protocol records but does not gate; that process also fails D's SE. At W16 our default row is at near parity (0.977–0.981 in B, C and D, claimed nowhere under min–max). Per manifold, each side's own count (ours 4,519.26, Jolt 8,489.0), Jolt is faster: 1.601× / 1.596× / 1.594× at W8 in B / C / D, 1.78–1.79× at W16.*
- **Where the per-manifold gap lives, from Jolt's own profiler.** Its profiled Distribution build costs +36 % at W=1 and +19 % at W=8, so only shares are read. About 65 % of Jolt's step is solve, and about 27 % is one FindCollisions job. That job's narrowphase is the body-pair cache: 8,488 of its 8,489 manifolds per frame go through "Add Constraint From Cached Manifold", which also builds the contact constraint, and the SAT runs once per frame. Stage by stage per manifold (block B):
  - **At W=1** our whole excess is the narrowphase: 537 against 263–301 ns, or np + setup + warm 666 against 273–321. Our solve is 186–224 ns cheaper and our broadphase is at parity. **C4 removes the excess** (np −380 ns; the W=1 ratio goes from about 1.18× to about 0.85×, arithmetic).
  - **At W=8** the excess is our five stages that do not parallelize: the broadphase query +45, solve_build +69, warm_apply +62, the graph +24 and the integrate group +23 ns per manifold, together 1.17 ms of a 2.18 ms step. Our narrowphase and colours are at or below Jolt, so C4 takes the ratio only to about 1.44×.
  - The stages have owners, some not yet assigned: the tree lane's C2 (after F3); L11 C4 (D10's parallel fill; solve_build is 14 % of T(8), over its 5 % trigger); and, with no lane yet, a warm apply that pays no extra dispatch, the awake-scene graph build and the integrate group (window 7 `analysis.md` § 6, FOLLOW-UP 2–6).

## L8 (friction per patch) and L12 (the effective-mass cache): designed and CLOSED by ruling (2026-09-19)

Both reviews found no blocking remark (L8: 5 Important; L12: 4 Important). Each design is closed; the implementing
lane folds these items in as rev 1.1 before its first commit.

**L8 — friction per patch** (`L8-patch-friction/`; value-changing, owner-approved D2). Two tangent rows with a
circular clamp at μΣλn and a twist row clamped at μΣd_iλn_i at the patch centre, on L11's block layout — the model
Jolt adopted in v5.6.0 (#2039), Box3D, Bepu and Rapier's default. `SoftStepSolver` stays the per-point Coulomb
reference. Rulings:
- **W1:** every row that GATES the lever runs with `--sleeping off` pinned explicitly; sleeping-on rows are
  witnesses. Before C0 a contact-set-robust pass rule is registered: the primary claim is the solve timed on one
  recorded contact stream in both binaries (`solve_colored` is public), with per-sweep cost normalised by points and
  by manifolds, each with its own SE. The end-to-end T(W) row is a secondary witness, because the lever moves the
  trajectory and so the contact set (D1 moved it 12 %).
- **W2:** the new counters stay off `WarmSeedStats` (a colored-solver-only accessor); G-S6 keeps its literal form,
  no diff in `soft_step.rs`.
- **W3:** a cohort in which no lane is multi-point skips the patch step uniformly (masked ops change no value, so
  G-P5 holds), and a sphere-pile row is gated "not claimed slower".
- **W4:** in every G-P gate the colored arm is the gate and the reference arm is a receipt; if the reference misses
  a bound it is reported, and the colored bound is never adjusted to match it.
- **W5:** pre-registered bands on the A7-R2, G8 and R-S freeze steps (confirm / report / STOP, as A7-R1).
- **O1:** `vn0` stays cold, outside the RankBlock, so the slot is L12's. O2–O7 adopted (the C1 digest and the
  multi-point pins join the red set; the FMA census covers `contact.rs`; `rc` blended by `present`; wording against
  L10's `SleepSkip::Off`; A7-R1 reported against the C0 base reading; G-P4's stop time is linear AND angular rest).
- **Review OQ1:** G-P2's band is registered at C0 from the base run, before any value change, and the registration
  is the gate; a base reading outside the analytic [25, 33] is a finding, not a band change.
- **Not in L8:** friction skipped in bias sweeps (Box2D main, Box3D) is a separate value change; it is a later
  candidate (L8b), priced on its own.

**L12 — effective masses cached per inertia epoch** (`L12-mass-cache/`; bit-identical). Rulings:
- **W1, the build-if:** the per-contact override written for L9 extends to L12 — the owner's goal is parity per
  contact and "squeeze maximum performance" — so the bar is the SE bar at W=1 per contact, not 5 % of T(W).
- **W2:** T1's control arm has the parent's kernel shape (no store, no column); rolling back means reverting C1,
  never a flag.
- **W3:** the C0 decision uses a bench-only upper-bound probe (masses not recomputed on load sweeps; a value-changing
  probe is allowed in a bench); if even that bound does not clear the SE bar, L12 closes at C0 without C1.
- **W4:** every W>1 arm asserts that dispatch happened on a scene that dispatches (widest colour ≥ 256 and
  n_chunks ≥ 2, or a scope/chunk delta > 0), and at least one cohort's compute and load sweeps run in different
  tasks.
- O1–O5 adopted (a control where inertia cannot change; G1 replays a recorded event trace; D6's coverage stated
  truly; a census of mutable access sites; c′_R = Rl_off/8; the reserve formula in `MASS_LANES_PER_*` terms).
- **Review OQs:** sleeping is pinned off for T1/T2; the per-contact denominator is solved (awake) manifolds, not
  L10's logical view; L12's layout is fixed after L8 closes (L8 owns the friction-mass choice); if T2 fails its
  claim with C1 landed, C1 is reverted.

## L10 rev 2.2 (re-based on L11, L9 and L5): REVISE → rev 2.3 CLOSED by ruling (2026-09-21)

`L10-sleeping/06-DESIGN-REV2.2.md` answers the three re-basing questions (the B1 carry and restore path on L11's
per-manifold records; coexistence with L9's tags and records; the frozen-pair skip inside L5's chunked emit). The
first review (`07`) was taken on the design's last text block only — the architect's answer arrived in two
messages and only the second was passed on; the complete text was restored from the agent transcript and
re-reviewed. That review (`07-REVIEW-OF-REV2.2.md`) found 1 blocking remark — the `row_frames` fill skips HELD rows
while sensor pairs with a HELD endpoint are still computed, so after a row shift a box sensor over a held pile reads
another body's frame — and 5 important ones (no W ≥ 2 sleeping-on census arm; sensor pairs in the kept unit; `dt`
missing from the sleep epoch although L9 reads it; REC admitted on a "exactly zero" quaternion claim that
`Quat::mul` does not satisfy; transitions into Off). Revision 2.3, a delta, is being written against those rulings.

**Rev 2.3** (`L10-sleeping/08-DESIGN-REV2.3.md`, a delta to 06) closes every remark of `07` — the reviewer
verified each answer against the tree and the arithmetic (`09-REVIEW-OF-REV2.3.md`: 0 blocking, 4 important,
5 optional, 3 open questions). No blocking remark remains, so L10 is closed by ruling; the implementing developer
applies these to 06 + 08:
- **W1 (S8b's scope pin):** at W=4 the colored solver (`colored.rs:215`, one scope per wide colour per pass) and the
  Grid-parallel emit (`resources.rs:1982`, `:2074`) open scopes the formula `S8 + [chunk_count ≥ 2]` does not count, so
  the exact count is red on correct code. S8b pins in L5 C4's structural form on BOTH arms (`scope_delta −
  Δnp_dispatches − Δbp_dispatches ≡ 0 mod passes`, or the measured envelope, as G-L5-5 does); the Tree arm
  additionally asserts `widest < MIN` on every step and pins the exact count there. The 0-heap-beyond-scopes pin
  stays exact on both arms.
- **W2 (jumper pairs on the Restore route):** the Restore route reuses L9's `PairJoin` (the monotone cursor plus the
  `jumper_bits` binary search) over `restore_idx.keys`; a plain cursor is not an implementation of D1. Mutation:
  "jumper pair merged on the restore cursor" (L9 M-c2's twin), red via a swap-remove despawn of a held member's
  tail-row neighbour, joins the S3 table.
- **W3 (arm 9 is vacuous in a gravity pile):** with τ = 0 every pair with residual velocity is fast and never REC, so
  the arm exercised M3′ on zero REC pairs. Arm 9 is rebuilt at exactly-zero relative velocity (gravity off, bodies
  placed at rest) and counts REC pairs admitted with HIT = 0 ∧ SETTLED = 1; a count of 0 is a void, and a void is red.
- **W4 (`RowCls` on a Sets step with nothing held):** A1.3 rewrites every row's flags on every step L10 runs — the
  rev 2 W3 option "skip the pass when nothing is held or CAND" is withdrawn (a skipped pass would have to zero
  `row_cls`, which is the same pass with a constant). Mutation: "flags not rewritten on a nothing-held step", red on
  the step after any restore arm's wake.
- **O-1..O-5 adopted:** the Lemma 2 amendment says the held pair's slot is left to the mirror (D-C's argument, not
  Lemma 2's); N32 becomes a debug spec assert (`len == 0` on a Reset flush) because the described mutation cannot
  go red; the `& SENSOR` grep gate exempts (or routes through `sensor_pair`) the routing `debug_assert_eq!`; the
  `unsafe` delta is stated explicitly (0 new sites if Skip reuses L9's tag write and L5's commit write; otherwise
  each listed with the partition SAFETY); D-F records that its edge is under Grid (Tree already fills ~0 held rows
  through L9's ruled O3 mask) and the fill predicate's composition with L9's mask (AND) joins the Q2 cross-lane
  contract.
- **Open questions:** (1) D-F composes with L9's fill as a per-row predicate (AND); if L9 C1 lands a lazy fill in the
  parallel phase, the predicate moves into that fill's guard and D-F has no separate site — decided at L9 C1, recorded
  in Q2. (2) The sleeping-off solve arm keeps its signature and access set (zero cost when sleeping is off — the same
  bar as UG-15's "zero without mods"); the D-H flush with `m = Off` is drained by L10's own system before the solve,
  never by the solve. (3) Cross pairs are REC-without-manifold or SEP only (a manifold would have unioned the islands);
  B3 states it, since it is what makes `restore_rec` keys unique (A3′) and the per-record hit count well-defined.

Lane order stands: L5 (C1/C2 committed, C3/C4 in progress) → tree broadphase ∥ L11 → L9 → L10 (rev 2.3 + these
rulings) → L8 → L12. L10's implementation brief is 04 + 06 + 08 + this section; 05/07/09 are the reviews it answers.

**The pre-C0 refutation does NOT fire, 2026-09-24 — L10 continues (window 6,
`docs/measurements/2026-09-24-physics-window6/analysis.md` §1; the 2026-09-24 block of `docs/MEASUREMENT-QUEUE.md` §14.1;
lane base `4db26681`, K=6).** The armed Off′ span on J-Son at W=1 (`--cfg a --sleeping --broadphase tree
--contact-reuse on`) reads 1.467 ms over the literal window [264,1000) and 1.304 ms over the all-frozen tail (the design's
Off′; the pile freezes at step 274), against the stop bar of 0.584 ms (`08-DESIGN-REV2.3.md:221`), claimed above it
under all three readings; both sit inside the design's 0.82–1.59 band. Off′ is 3.230 ms under J-Son, and that drop is
the Tree plus L9b, not L10. By arithmetic, L10's own gain ceiling on the J-Son tail is 1.155–1.195 ms at W=1 (3.3× its
0.35 ms realized-gain gate) and 0.60–0.70 ms at W=8 (gate 0.14); the R-S gate is 0.806 / 0.429 ms at W = 1 / 8
(0.6 × (Off′_RS − 0.25), Off′_RS 1.594 / 0.966 ms).

## The W8+ scaling plan (`scaling/01-DESIGN.md`): eight rulings and the lane order (2026-09-26)

The design (rev 1, which answers its own architecture review in place) explains our T1/T8 of 2.72 against Jolt's 3.83
by two things at equal parallel efficiency (E(8) 0.62 against 0.60): our serial time is about twice Jolt's (1.20 ms
against about 0.58 ms), and our parallel work is 0.54× Jolt's. It ranks seven bit-identical levers, S1–S7, whose core
is a persistent solve region (S1) in place of about 109 colour scopes per step. The rulings are quoted as ruled in the
design's §9; what each means for a lane:
- **Tree C2 reopened on a W8 letter, as lever S5**, after F3's re-time. Its W1 letter (NOT BUILT, 2026-09-25) stands
  for W1; the W8 decision is taken on W8S's numbers.
- **The L6 fork letter is superseded.** The region primitive is decided on the S1 + S2 + S3 bundle, not on the colour
  loss alone, and L7 is revived as S2 on S1.
- **An armed-only `Relaxed` timestamp per task is admitted into the W8S instrument**, with its zero cost when disarmed
  proven by a codegen or census check. It is never a zone inside a worker task.
- **The headline metric is T(8) and T(16) per manifold, plus the identity terms.** T1/T8 is retired as an objective,
  because it falls whenever W1 gets faster.
- **The window-7 §5 re-read was queued for the W8S analysis**; ruling 10 of the next section closes it.
- **(OWNER VALUE, open) Spinning helpers for the region's length** — about 0.6–1.3 ms a step at W8, on up to seven
  cores — was put to the owner. Until it is answered, S1 carries both a spin-only and a spin-then-park variant.
- **L5 OQ1's per-step scope exception (the L5 section above) is extended** to S4's interim setup scope, which S1
  retires, and to S5's broadphase scope.
- **W8S's canary ladder and span gates are adopted** as the resolution protocol at W ≥ 2.

**The order of physics lanes** after L10b merges: (1) the W8S instrument commit (bit-identical, armed only), then the
W8S quiet window with Jolt in the same window; (2) S4, the parallel solve setup — cut and built before W8S, sized by
it, claimed only after the window; (3) tree F3 with `--bp-kernel` and the window-7 thresholds (`TREE_BRUTE_MAX_ROWS`
144, `AUTO_TREE_LO/HI` 144/152), then tree C4, then S5; (4) S1 + S2 + S3 (+ S7) as one kernel lane — research, design
review, loom models, the ω_b microbench; (5) S6 only if its hit-rate counter clears the build-if; (6) L10 C1b, with its
own quiet-window timing, where S1 must keep freeze capture/restore and write-back serial; (7) L8 after its rev-2
critique items are closed in the implementation, with its A/B in a quiet window; (8) L12, whose T1 shadow probe runs
in a quiet window and which is built only if the build-if holds.

Amended on 2026-09-27 (the next section): S6 is not built, and S7's partial form is promoted to right after S4.

## After window 8 (`docs/measurements/2026-09-27-physics-window8/`): the claim rule from 8b, S4 first, S6 not built, S7's partial form promoted, S1–S3 deferred (2026-09-27)

Window 8 ran on the instrument build `226bd99e`. Its standing, block A over [0, 500): ours/Jolt 5.6 is **0.449× at W1
(claimed)**, 0.795× at W8 and 0.866× at W16 (not claimed under min–max). Per manifold it is 0.816× / 1.456× / 1.567×,
the W16 reading claimed with Jolt cheaper. The W8 excess over T(1)/8 is 70 % serial: our serial chain is 1.31 ms
against Jolt's 0.63 ms. The inputs were the window's `analysis.md`, the instrument's review
(`review-instrument-226bd99e.md` in the same directory) and the rulings of 2026-09-26; the full text of the rulings is
the window README's "Rulings" section.
- **1. The claim rule from window 8b on**, registered before any 8b data. Two windows in a row lost a claim to single
  processes that ran uniformly 4–13 % slow with clean receipts, while the other pass resolved 0.030 ms. From 8b a
  cell has K = 9 over three passes (p0 reversed, p1, p2 reversed); a claim needs IQR AND SE in every clean block and
  pooled; min–max is reported beside it, and a claim that also passes it is STRONG. The resolution ladders are judged
  the same way. The driver records a per-process placement receipt (the main thread's share on its top logical CPU),
  which is never used to drop a process. Window 8's own verdicts are not changed retroactively.
- **2. S4 stays first.** Its bar stays the design's 0.060 ms (0.6 × 0.10); window 8 moved its expected gain to about
  0.19–0.22 ms, because the P-c fill's share of the setup is f = 0.82. The claim is taken in 8b's S4-AB block, with its
  own ladder, rung and zone canary.
- **3. S6 is NOT BUILT.** Its build-if (≥ 5 % of T(8)) fails at 3.5–3.8 % on the lower hit rate (J-T's). The design is
  frozen with this number, and reopens only if a later window's hit rate moves it over 5 %.
- **4. S7's partial form is PROMOTED** to a small lane right after S4, which touches the same solver files: the lanes
  capped at the physical cores on today's scope path (design §6.7). Its window claim is W16 not slower than W8 on
  J-T and J-A, with W8 unchanged. The engine reads the physical-core count itself, per ruling Q6 of the W8S lane's
  cut ("S7's lane owns an engine-side physical read").
- **5. S1 + S2 + S3: the decision is DEFERRED to window 8b.** ω_b as benched in window 8 reads 2.8–3.0 µs a stage at
  eight participants, against the design's 0.3–1.0; at that cost the bundle is about 5.5–6.8 % of T(8) (arith.). The
  bench could not decide it: its zero-work blocks make every participant contend at once, and its bench-only shared
  counter sits beside the publish word. Two inputs come first: B1 and B2 from the fixed instrument, and an ω_b v2
  re-bench — about 0.7 µs of real work per block, no shared `runs` RMW, padded claim and done words, blocks at 1× / 2× /
  4× P, P capped at 8, and the spin-then-park variant of the 2026-09-26 owner-value ruling. The ruling places ω_b v2 on
  `u/phys-w8s` after its verify; the S7 lane's spec carries it instead, as commit (1) of `u/phys-s7`, bench only.
- **6. Warm apply is the largest serial stage after setup** (0.304 ms at W8), and today reaches the parallel path only
  through S2, on S1. The S1 lane's design review prices an interim "S2i" (warm apply on today's scope path, the S4
  pattern) against S2-on-S1 with window-8 numbers, and the same for the integrate group (0.121 ms) and the graph
  (0.121 ms).
- **7. F3 → tree C4 → S5 is unchanged.** S5 is supported: the broadphase query is 10.5 % of T(8), 0.210 ms.
- **8. L10 C1b shows no measurable awake cost.** The settled pile reads −36 % at W8 and −49 % at W1, at cfg a. The
  product row, sleeping on the Tree broadphase, goes into 8b (J-Son-T at W 1/8 over [0, 100) / [100, 500) /
  [274, 500)). C1b keeps its place after the lanes that move no pins, because it moves every pose pin.
- **10. The Jolt re-read is closed.** Design §4.2 is CONFIRMED and window 7 §5's "Jolt overlaps serial with parallel"
  is REFUTED; Jolt's lighter profile costs 0–1 %, which closes window 7 FOLLOW-UP 10. This also closes the 2026-09-26
  ruling that queued the re-read.
- **9 and 11 are not lever rulings:** DM1 C6 is kept by its gate's letter and re-read in 8b with more power (render),
  and the orchestrator issues no tool call during a timed block beyond a progress read (window hygiene). Both are in
  the window README.

**Window 8b's contents**, assembled after the lanes merge and all under ruling 1: S4-AB (parent the w8s commit (3), tip
commit (4); an in-block ladder at W8/W16, the rung at W 1/2/4, the zone canary at 30/60 µs); the per-wave split rows on
the parent (J-T and J-T-a at W 1/8/16: B1, B2, N3, N6); ω_b v2 (both routes, participants 2/4/8, stages 36/72,
spin-only and spin-then-park); ω(W, gap) with the N4 participation receipt; the F3 rows (R1–R4 and the G4 block);
J-Son-T; the DM1 re-read. S7-AB joins it once the S7 lane lands.

Amended on 2026-09-29 (the next section): S7's partial form (ruling 4) is rejected and frozen, and the owner answered the
2026-09-26 owner-value question on spinning helpers.

## After window 8b: S7's partial form rejected and frozen, S4 kept, F3 not the default, the tree thresholds not reproduced, helpers spin (2026-09-29)

Window 8b ran on 2026-09-29 under ruling 1 of the previous section (K = 9 over three passes; a claim needs IQR AND SE,
and a claim that also passes min–max is STRONG): 894 timed processes, 0 invalid, 0 voided passes, and 26 of 786 slots
dropped because the desktop and agent sessions were busy. Its numbers are recorded in the window 8b measurement record
(to be committed). 8b carried no Jolt row, so the standing against Jolt 5.6 is not re-measured.
- **1. Owner value: helpers SPIN.** This answers the owner-value question of 2026-09-26. The S1 region, and any later
  region-style scheduler, keeps its helpers in active waiting between stages; spin-then-park is not the default, and
  laptop energy is accepted as the price. The park path of the ω_b v2 bench stays a measurement axis only.
- **2. S7's partial form (`u/phys-s7` commit (2), `a3adc827`) is REJECTED and frozen.** Capping the colour waves at the
  physical-core count leaves W8 unchanged and makes W16 slower, STRONG: +0.140 ms (+6.8 %) on J-T and +0.658 ms
  (+12.6 %) on J-A. C1 (W16 not slower than W8) fails STRONG on both rows and C3 (the predicted W16 gain) is refuted
  STRONG: the SMT lanes help inside the waves. The commit is not merged; it is kept under the annotated tag
  `phys/s7-partial-frozen`, and it returns only on a W16 A/B that shows a capped wave cheaper, e.g. inside S1 with S1's
  own W16 A/B. Commits (0) (the rulings) and (1) (ω_b v2) are merged on their own. PC-S7-7, the physical-core cap on
  S1's participants, is struck until S1 has its own W16 A/B.
- **3. S4: KEEP.** At W8 on J-T the trunk is 0.1705 ms (8.58 %) faster than its S4-off parent, STRONG, on the
  pre-registered span route; no W is claimed slower.
- **4. F3 (`LeafListKd`) is not the default.** R1 is STRONG (the kd order cuts the J query by 16.9 %), but the kd build
  eats the gain (22.9 → 50.9 µs at W1, 22.3 → 54.4 µs at W8) and R3 is not claimed. It is held opt-in until the tree C4
  decision; if C4 does not use it, it is frozen and removed.
- **5. The tree thresholds 144/152 are not reproduced**: the crossover reads 128/136. G4 is re-read at sizes 96–128
  before tree C4 wires `AUTO_TREE_LO/HI`, and the all-pairs slowdown against window 7 is checked first.
- **6. C1b (sleeping on by default) is supported.** No awake cost resolves at W1 or W8, and the settled pile is cut by
  96.5–99.1 %. C1b keeps its place in the queue (ruling 8 of the previous section).
- **7. DM1 C6: KEEP.** Window 8's +22 µs on `VB_EARLY_CULL` did not reproduce.
- **8. A pass-cell with K < 3 does not gate.** The letter stands: window 7's clause, imported by every later window.
  The driver changes for window 9: a dropped slot is re-run until the cell has K = 3 or the pass ends.
- **9. S1 stays undecided** until S2i and S3i are priced on 8b's inputs: a net stage barrier of 0.35–1.07 µs at P = 8, a
  park price of about +6 µs a stage after a serial gap of 20 µs or more, and a wake latency of 3.4–3.7 µs.
- **10. Owner (2026-09-29): "find out why we are slower than Jolt and solve it."** A diagnosis precedes any new lever:
  equal work first (manifold counts 4467 against 8489, iteration budgets), then the stage map, ours against Jolt, at
  W 1/8/16, then a ranked lever plan.

Amended on 2026-10-01 (the next section): tree C4 merges as the whole lane before ruling 5's G4 re-read, with its
thresholds marked PROVISIONAL until window 9b.

## After window 9a (2026-09-30): V2 ruled in (speculative contacts at 20 mm, K3, the approach-velocity margin), the SAT hysteresis sign fix dropped, tree C4 merged whole, the Rapier bar, the iteration budget, model routing

Window 9a ran on 2026-09-30 (19:13–22:16, 182.6 min) with the tip `u/phys-tree-c4` `50e31f1a` (the Tree the default
broadphase, value-neutral) against the parent, the trunk `3d9433ae` (AllPairs the default). Three blocks closed all
three passes: C4-AB with Jolt 5.6 in-block, C4-RAPIER (Rapier 0.36) and C4-G5. C4-BR aborted in its only pass (the
machine was never idle), and C4-G4 and C4-G4-kd never ran. There were 696 timed process records, 0 invalid and 0
voided passes, but a busy desktop left 30 of C4-AB's 102 pass-cells, all in pass 0, at K < 3, so almost nothing is
claimed by the letter; the one claim is Jolt's per-manifold lead at W16 (STRONG). Its numbers are recorded in the
window 9a measurement record (to be committed). Unclaimed, and at unequal fidelity (the pre-V2 pile deforms, 26 boxes
past 0.5 m and 2.18 m maximum drift, where Jolt and Rapier hold theirs), ours/Jolt 5.6 on the wall reads 0.435 / 0.506
/ 0.586 / 0.696 / 0.818 at W 1/2/4/8/16, and ours/Rapier's default (its simd8 build) 0.884 / 0.987 / 1.073 / 1.266 /
1.298.

The rulings were taken in two sittings. Those of 2026-09-30 came before the window, after the Jolt-gap diagnosis F0
(verdict: DEFECT yes, REGRESSION no) and on the Jolt-gap plan (PLAN rev 3): owner values, then the V2 flip rulings.
Those of 2026-10-01 answer the window's questions. The F0 verdict, the Jolt-gap plan and the V2 lane's implementation
report are not yet on the trunk. In rulings 3 and 6, S5 and S20 name the 5 mm and 20 mm margins, not the scaling lever
S5.

**2026-09-30 (owner values after F0, and the V2 flip rulings):**
- **1. Owner V2a: speculative contacts become our contact rule** (the Jolt / Box2D v3 form): a contact point is kept
  while its separation is ≤ d, and the solver treats s > 0 as a bias s/h with no push. Every contact pose pin moves
  once, re-derived by its own rule; the window gate fixtures (the JT500 / JA500 / JToff500 family) and the census
  counters are re-recorded.
- **2. Owner V2b: the default margin is 20 mm**, a `PhysicsConfig` value (Jolt's `mSpeculativeContactDistance` = 0.02,
  Box2D v3.1's `B2_SPECULATIVE_DISTANCE` = 0.02 m).
- **3. The production form is the F0 verdict's (a)–(f):** both kernels (scalar and the AVX2 cohort, branch-free
  select); the current separation from each body's accumulated step movement (Box2D v3's `s = s0 + dot(d, n)`),
  written in the existing integrate, with no new serial stage; every pair type (box-box's five sites, sphere-sphere,
  sphere-box, SDF); d/2 added to each body's broadphase radius at gather (the C4 / S5 kernels untouched); the SAT
  axis-hysteresis sign fix for negative depths (dropped by ruling 10 (a)); and a fidelity gate that can fail (J-T, 8
  one-ulp-seeded trajectories, reuse on AND off, 0 boxes > 0.5 m and a maximum drift < 0.5 m in 8/8; red on today's
  trunk; the `slow:` ignore class). K3 (the current-separation form applied to every point) was to be included only
  if F0e(ii) showed it tightens the pile; ruling 6 includes it.
- **4. The order is bugs first:** V2 lands on the trunk before any SR or S5 merge; those lanes rebase, re-run the M1
  census and re-pin, and window 9 (M7) measures the V2 trunk. C4 is value-neutral and keeps its own merge (window 9).
- **5. Owner, model routing: "both steps".** (a) Now: Sonnet 5.5 runs scouting and inventory, window preparation, the
  transcription of finished results into docs, and re-runs of existing gates with exit-code reports, each checked by
  an Opus verifier or a hard oracle; the architect, critic, code-reviewer, verifiers, results-analyst and the
  developer on kernel / unsafe work stay on Opus. (b) A judgment benchmark (≥ 25 historical cases with known answers,
  confident-wrong scored separately) decides the tester and part of the developer. `effort: low` on Opus is NOT used
  (the retrieval bench: 4 confident-wrong). Ruling 7 applies (b).
- **6. The F0e verdict (verified): K3 IS INCLUDED, default ON for every point.** K3 recomputes each contact point's
  separation after every substep's integrate, in Box2D v3's exact form
  `s = s0 + dot((dp_B + dq_B r_B − r_B) − (dp_A + dq_A r_A − r_A), n)`, and the kernel branches on the current s.
  - At gap 0.5: K3 alone holds 0/8 (not a fix by itself); S20 holds 8/8 with 65–92 boxes > 0.1 m; **K3 + S20 holds
    8/8 with 106–153 mm maximum drift and 2–6 boxes > 0.1 m** (Jolt 51 mm / 0), reuse on and off, at +7 % rows over
    S20 with the colour passes unchanged. The XS controls show that the gain comes from K3 reaching penetrating
    points. The V2 lane therefore ships S20 + K3 as the default; the const that selects "speculative points only"
    stays as a documented switch, off.
  - At the harsher drop, gap 1.0: S5 and S20 alone fail (layer 2 / layers 3–4 outer ring, flat deep landings — a
    different mechanism from the trunk's layer-8 grazing failure). **K3 + S20 holds 8/8 with reuse on and 0/8 with
    reuse off** (one layer-2 outer-ring box past 0.5 m in every trajectory).
  - The fidelity gate therefore has TWO arms: gap 0.5 (reuse on and off, 8/8) and gap 1.0 with reuse on (8/8). Gap
    1.0 with reuse off is recorded as a known limitation with its numbers, not gated, and goes to a follow-up
    diagnosis (F0f: does Jolt 5.6 hold at gap 1.0; the low-layer deep-landing mechanism). K3 + S5 is rejected (5/8, a
    layer-3 failure). Ruling 9 replaces the two arms with the gap sweep.
- **7. Model routing after the judgment benchmark** (47 historical cases: Sonnet 5.5 caught 45, missed 1, was
  confidently wrong on 1; Opus 5.5 46 / 0 / 1; tokens per task, Sonnet −13 %). The tester runs on Sonnet 5.5 from the
  next lane on, with the Opus code-reviewer still beside it. The developer stays on Opus for kernel / unsafe /
  numerics and uses Sonnet for mechanical work. The architect, critic, code-reviewer, results-analyst and verifiers
  stay on Opus. The running lanes (tree-c4, v2-spec) are not switched mid-flight. CLAUDE.md's "Model routing" section
  is rewritten in a doc commit with both measurements.
- **8. The SR cut's questions, answered by the orchestrator:**
  - Q1: Phase B (the physics half) starts only after V2 is on the trunk. Phase A (`region.rs`, its panic / epoch
    tests, loom M-R1..M-R13, the `Scope::new` debug guard, ω_b v3) may be implemented first.
  - Q2: the lock set extends to `profiling.rs`, `scope.rs`, the new physics test files, and V2's files once V2 has
    merged.
  - Q3: the critique's W8 is fixed by reserving the whole epoch range at region entry, gated by the native
    caught-poison test at P = 2/8/16 and loom M-R13 with the red mutation.
  - Q4: the merge bar is a FORMULA fixed now: SR merges iff W8 J-T ≥ 0.6 × SR's predicted Δ on the contact set of the
    trunk it lands on, with no W slower (W16 included); the value is computed at window prep from that trunk's census.
  - Q5: V2's per-step delta reset folds into SR's first gravity stage.
  - Q6, the policy defaults: the orchestrator pure-spins; helpers never park inside a region (the owner's "active
    waiting"): PAUSE, then `yield_now` between PAUSE bursts, never a sleep or park. PAUSE-only against PAUSE + yield
    is an ω_b v3 axis.
  - Q7: add `--scene pairs` for the one-wide-colour sweep arm.
  - Q8: build behind a switch; one final commit flips it, deletes the old path and re-pins the census once.
  - Q9: M5b's per-block owner receipt stays out of this lane.
  - Q10: V2's spec forbids a new serial stage for K3 (the per-body delta is written inside integrate). If V2
    nevertheless lands K3 as its own pass, SR carries it as one parallel per-substep stage; this is confirmed at the
    trunk sync.
  - Implementation timing: SR's implementation starts after window 9a (C4 + V2 + G4 + the Rapier / Jolt rows), so
    that its builds do not break the quiet window.
- **9. The approach-velocity margin (VMARGIN) is PART OF V2** (F0g, verified HOLDS).
  - Evidence: a FIXED margin only moves the partial-landing boundary (F0f). Over the gap sweep, K3 + S20 holds 35/88
    runs, Jolt 5.6 37/44, and **K3 + S20 + VMARGIN 88/88** (11 gaps {0.5, 0.75, 0.9, 0.95, 1.0, 1.02, 1.05, 1.1, 1.25,
    1.5, 2.0} × reuse on / off × 4 seeds), with a maximum drift of 56–427 mm (gap 0.5: 56–72 mm against Jolt's 49–52).
    It costs +3.5–5.6 % rows in the steady state with the colours unchanged; box 433: 786 → 24 mm.
  - **The rule:** `d_eff = d + min(cap, max(0, approach) * h)`, with h the step length, cap 0.5 m (never reached in
    F0g), gravity not included, and the velocities the two bodies' step-start velocities. It applies per POINT
    (linear + angular, `v(p) = v + w × (p − c)`, both bodies at the incident point) at the clip keep in `face_patch`
    and at the reuse `refresh_face` keep; per AXIS, with the conservative bound
    `−(v_B − v_A)·n + |w_A| R_A + |w_B| R_B` (R the circumradius), at the SAT early-out `separates` (which also covers
    `sep_still_holds`) and at `refresh_edge`; and as the same term on every other pair type (sphere-sphere,
    sphere-box, SDF).
  - **The broadphase:** each body's bounding radius grows by d/2 + min(cap, (|v| + |w| R) * h) inside
    `body_bounding_radius`, which every broadphase reads.
  - **The production form:** the velocities reach the contact sites through the call path (arguments / the pair
    record), with NO thread-local context; the prototype's per-thread context is not shipped.
  - **The fidelity gate becomes the gap SWEEP** (the 11 gaps above) × reuse on / off (no seed, plus one 1-ulp seed at
    gaps 0.5 and 1.0): every run must hold, and the gate is RED on the parent. A red control, "the velocity term
    capped at 1 mm → gap 1.5 with reuse on FAILS" (F0g measured 19 boxes, 9,447 mm), proves that the gate sees the
    term. d = 0 AND the velocity term off must still reproduce today's rule byte for byte (the d = 0 identity pins).
- **10. The V2 flip rulings** (the orchestrator, answering §2–§3 of the V2 lane's implementation report):
  - (a) **The SAT axis-hysteresis sign fix (spec item (e), ruling 3) is DROPPED from V2.** The rule stays
    `best.depth >= last.depth / 1.05` for every depth. With the fix and K3, A7-R1 with reuse off creeps 13.294 mm
    (the bound is 10 mm and is never relaxed); without it, 0.72 / 0.53 mm. The gap sweep holds 13/13 either way, and
    every F0 prototype that held ran without it. The C2 unit test `n2_a_separated_hint_holds_under_the_hysteresis` is
    inverted or removed with a note citing this ruling. Follow-up PC-V2-HYST (after V2): why the sign-correct
    hysteresis creeps with reuse off, with a test that can decide which form is right.
  - (b) G5 / G6 (premise P2), the G-L9b-7 tipping and the sleep-skip S3 voids go as §3 of the report plans. The
    overlap-only arm runs at d = 0 and cap = 0 against its old premise, plus a V2 twin that tests the V2 rule's own
    boundary. S3's cross-pairs / move-in rows settle longer or are re-placed so that the void guard has material (the
    run-against-run comparison unchanged). Grow-clear prefers a V2-native placement (the pair lifted past d_eff), else
    the overlap-only arm.
  - (c) The pins that move by their own rule: the pyramid `PINNED_FINAL_HASH` (the four new values in §3 of the
    report); the census S1c / S1e envelopes (195..=195, dispatch 391; debug 392), with the S1f d = 0 twin keeping the
    old pins; `bp_query_counts` (the overlap-only arms keep the model figures, and `j_snapshot` gains the (reuse,
    overlap-only) form); the fidelity anchors, from the C6 fixture recordings; the window fixtures, via the V2 lane's
    `tools/record_v2_fixtures.sh`; and the `_v2` pose-gate scripts.
  - (d) `alloc_frame_attribution` row D: the pinned "32 tasks at W = 8 fit one 4 KiB block" is a property of the
    pre-V2 contact set. The row is re-expressed as a derived expectation, with a red mutation: chunks == scope
    frames + the number of colour scopes whose task count at W = 8 exceeds one block's capacity (computed from the
    step's colour task counts). It must also prove that there is no per-step heap growth in the steady state (the second
    block is reused). SR later cuts the per-wave scopes to 3 per step.
  - (e) The G2 / G7 flicker-budget generator re-run is REQUIRED at the flip (critique W2b), with the budgets
    re-derived by the generator's own rule.
  - (f) Timing resumes after window 9a; the lane's builds must not run during the window.

**2026-10-01 (the owner's answers after window 9a, and the orchestrator's decisions):**
- **1. Owner: "beat Rapier" means Rapier's FASTEST settings against OUR fastest settings.** It is fair only under one
  fidelity bar for both: each engine's fastest configuration that still passes the SAME fidelity gate (the V2 gap
  sweep × reuse on / off, 0 boxes > 0.5 m and a maximum drift < 0.5 m in every run) on the J-T scene. Rapier's harness
  gains a gap flag and a configuration search (solver iterations, stabilisation, friction model, prediction
  distance, …) to find its fastest holding configuration; ours does likewise, via ruling 2.
- **2. Owner: the solver iteration budget may be a COMPILE-TIME parameter, with ADEQUATE values.** Today it is 4
  substeps × (1 biased + 2 relax) = 12 sweeps. The design: the budget as compile-time constants (const generics or a
  feature-selected preset, at zero runtime cost); the shipped default is the cheapest budget that passes the full
  fidelity bar (the gap sweep with reuse on and off, the pile tests, the existing solver oracles) with margin; values
  outside the validated set are refused at compile time. It lands after V2, where the fidelity gate lives.
- **3. Owner: codegen-units = 1 is acceptable for the shipped build profile if it measures faster.** Window 9b carries
  a cgu = 1 twin of our tip against the cgu = 16 parity profile; the profile changes only on a claimed gain.
- **4. Orchestrator: tree C4 merges now as the whole lane**: C4-1 the thresholds 128 / 136 (PROVISIONAL), C4-2 Auto
  opt-in with Manual the default, C4-3 the flip, C4-4 the record (`broadphase/07-C4-RECORD.md`). Window 9a met its
  merge rule, M7: J-D is 1.58–1.69 ms faster than the parent's default at every W, no W is slower, and G5 is recorded
  with every bar STRONG. C4-G4 was not measured, so ruling 5 of 2026-09-29 (G4 re-read at 96–128 before C4 wires
  `AUTO_TREE_LO/HI`) is not met at the merge; the flip reads only `TREE_BRUTE_MAX_ROWS`, and Auto is opt-in. The
  constants stay marked PROVISIONAL until window 9b's C4-G4 re-read.
- **5. Orchestrator: window 9b** = C4-BR + C4-G4 + C4-G4-kd (unmeasured in 9a) + V2's cost (V2-AB / SPAN / RES) + the
  cgu = 1 twin + Rapier's fastest holding configuration and Jolt 5.6 in-block, adjacent to our rows + the SR / S5 A/B
  when they exist. The desktop must be quiet: the owner's question 1 from 9a is still open (Telegram, the browser and
  other Claude sessions made C4-AB's pass 0 unusable).
