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
change the flip's size, not its sign, unless it finds the excess is work the design forbids.

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
not run, and the stop rules of 2026-09-22 stand.

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
flip (C4) is not decided by this line: rules 6, 8 and 9 remain (`c3b/design.md:237-241`).

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
- **G9's remainder on C3 (2026-09-24, window 6; `docs/measurements/2026-09-24-physics-window6/`, queue section 14.4):** window 5's own two binaries (`0ca312bd` against `cbd86a65`), K=6 interleaved, **PASS**: no row is claimed slower under either rule — J-A −0.11 % / −0.68 % at W = 1 / 8, R −3.65 % / −4.70 %, R-S −0.46 % / −0.91 %, S16 −0.32 %, J-As at W 2/4/16 −3.72 / −4.23 / −5.34 %, J-As-a −4.07 %; warm_apply on `J-As-a` at W=1 re-reads 0.5278 → 0.2979 ms (−0.2299, claimed; window 5: −0.2325); wide colours are not claimed slower; S16's store 60 → 80 ns is two timer quanta on an unclaimed step, a reading, not a FAIL. The canary is SEEN (its span 1.0008× / 1.0006× the injection; the step rise claimed on the tip, and under G9's own SE form on the parent). **G9 is not formally closed:** J-A at W 2/4/16 was not run, and the design's K=12 is not met (every cell K = 6 or 5); whether K=12 is required is an open ruling. Bridge 2 holds: window 4b's tip against window 5's parent on `J-As` is +0.16 % at W=1 and +1.75 % at W=8 (SE only), so window 5's +11.5 % at W=8 was mostly machine state.
- **L7 RETIRED (2026-09-23, after C3), closing the design's instruction** (`L11-solve-setup/02-DESIGN-REV1.md` D7, "Recommend retiring L7 after C3", and its interaction list, "L7: retire it after C3 (D7)"). L7 — `warm_start_apply` per wide colour, in parallel (`01-PLAN-REV1.md`, row L7) — is not built and leaves the lever set. D7 prices its dispatch at 4 × 9.11 waves × ω(8) 6.54 µs ≈ 0.24 ms, and C3 has now measured the whole warm apply it would split at 0.296 ms (W=1) / 0.304 ms (W=8) (window 5, armed, reading B). The plan's size rule (t_warm(8) ≥ 3 % of T(8)) is met at 6.7 % (0.304 of 4.510 ms), but the plan's own formula t_warm(1)·(1 − 1/(E·W)) − 4·waves·ω nets it to at most 0.296 × 7/8 − 0.24 ≈ 0.02 ms at W=8 even at E = 1 — arithmetic, not measured. Its prerequisite L6 was never built (`docs/physics/perf-campaign/00-RULINGS.md`, "Rulings after P0", 2026-09-19: "L6 is not built (8.2 % < 10 %); L7 stays gated behind L6"). No L7 code, lever directory or gate exists, so nothing is removed; a per-colour parallel warm pass comes back only as a new design with its own measurement.
- **L12 is commissioned** (the effective-mass cache per inertia epoch: 5 of 12 sweeps compute, 7 load;
  bit-identical) as its own design after L11.

**Lane order (ruled, replaces the order above):** L5 (in flight) → then, in parallel because their files are
disjoint, the tree broadphase and L11 → L9 (after L5, which owns the narrowphase system) → L10 (rev 2.2: it
adopts L11's per-manifold warm storage and L9's tag layout) → L8 (friction per patch, on L11's block layout)
→ L12. Each lane is gated end to end and per contact under the P0 protocol.

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
