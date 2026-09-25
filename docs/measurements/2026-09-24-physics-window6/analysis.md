P1 CONTINUE L10 (Off′(1) = 1.467 ms over [264,1000) and 1.304 ms over the all-frozen tail, against a 0.584 ms stop bar); P2 BUILD L9 C4 ((a) band [1.077, 1.666] ms, CLEAR; (b) ΔT(1) = 1.845 ms against a 0.900 ms bar, PASS; Δt_np(1) = 1.725 ms); P3 C3b SHIPS and F3 IS TAKEN UP (t_q(J,1) = 0.210–0.235 ms and c_q = 170–190 ns, over 150 ns; attribution A refuted; the C2 call is flagged and not robust); P4 PASS (no FAIL, canary SEEN; G9 still owes J-A at W 2/4/16 and its K=12 letter)

# Window 6 reduction: L10 pre-C0, L9 C0 → C4, C3b query cost, L11 G9 remainder, extras and bridges, recomputed from `win6/raw/`

results-analyst, 2026-09-24.

**Inputs:**
- Window 6: `runs.jsonl` (553 records), every `run.csv` and `stdout.txt`, `rows6.json`, `plan.md`, `SHA256SUMS`, `COMMIT.txt`, the fixtures, `wait_log.txt`, `progress.txt`, `WINDOW_DONE` (`exit 0 complete`).
- Bridges, each recomputed from that window's own raw files with the same slot rule:
  - window 5 `d9f1cf70…/win5/raw` (its protocol set only);
  - window 4b `win4b/raw`;
  - window 4 `win4/raw` (`T-A-tree-armed`);
  - window 3's Jolt v5.6.0 cells, from the `mean_ms` values in `D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/runs.jsonl`.
- Window 4's G4 j100 cell (0.4430) is cited from `2026-09-22-broadphase-tree/analysis.md:134`. It is the only number I did not recompute.

**Method:**
- My script recomputes every window mean, span, counter and frozen step from the per-step CSVs. It rebuilds the class-bench band from the SUMMARY JSON with the formula in `narrowphase_classes.rs:746-755`, and re-parses the criterion estimates. It re-derives "clean" from the receipts and imports nothing from `reduce6.py`.
- **A cell** is the median over K; r = min–max, i = IQR, s = 1.2533·SD/√K, each relative to the median.
- **Claim rule:** a difference is claimed iff |B/A − 1| > 2·hypot(sA, sB), under both r and s. Against a constant bar, the claim uses the cell's own spread.
- **Span readings:** A = the per-process mean; B = the per-process median over [100,500) or over the row's window. B is the convention windows 4b and 5 used for their gates.
- git was read-only.

## 0. Selection, input checks, and the driver's reduction

**Selection:**
- 553 records = 529 processes (24 warm-ups, 444 originals, 61 re-runs) + 24 pass markers. There are 0 voided passes, and `binaries_after` reads "all match".
- **436 of 444 slots are used, 53 of them re-runs. 8 slots are dropped**, because both attempts had an after-receipt over 5 %. Each of these cells is therefore K=5:
  - `C3b-TA-armed` par W1
  - `C3b-TD-tree` tip W1
  - `G9-S16` g9p W1
  - `G9-RS` g9p W8
  - `G9-JAs-mid` g9p W2
  - `G9-R` g9p W1
  - `G9-JAs-mid` g9t W16
  - `B2-JAs` w4bt W8
- My recomputed "clean" matches the driver's flag in 529 of 529 processes. The 69 contaminations were all flagged by the after-receipt: `claude.exe` topped 67 of them, and no build or lane process was present.

**Input checks (all hold):**
- Every window mean equals `mean_ms` (|Δ| ≤ 1e-9 ms) and `window_mean_ns`, with full step counts.
- **Structure: 0 problems in 436 processes.** Every process has:
  - exit 0 and 0 void steps;
  - `expect_pose: match`, with the hash equal to its fixture (J500 `0x32d5e235342b4143`, JAon500 `0x30c5438bc6ad9ffa`, Offp-J1000 `0x3db47fae414b655c`, Son-J1000 `0xcc2a5400c66eecce`, R1100 `0x87e561d20589d4a5`, Ron1100 `0xc8bbe34cf6a8afc6`, RS800 `0x2a2b7926a48aab00`, RSOffp800 `0xb7f1e9e8f91f75ab`, S16-300 `0x8877dbb1192e9b92`);
  - `workers` = `pool_workers` = W, `msvc`, and the armed flag its row expects;
  - drops 0 and dispatcher-solve 0, with no ring traffic;
  - the row's broadphase kind;
  - TreeDiag `static_rebuilds 1, members 1, evictions 0` on tree rows (L10 Off′ included) and all zero on AllPairs rows;
  - mask `0xffff`, and an exe sha256 and commit that match.
- The class bench printed a SUMMARY in 6 of 6 processes, and its printed band equals my recomputation. Criterion printed 8 estimates in 6 of 6.
- **Frozen step:** the runner's `first_frozen_step` equals the CSV's last-awake step + 1 in every sleeping process (the CSV's `awake` is post-step). Off′ freezes at 274, J-Son at 265, R-S at 248 and R-S Off′ at 250.
- **Receipts:** 620 in all; median 0.85 %, p90 5.13 %, max 17.21 %, 69 over 5 %. The during-process witness over the used set: median 0.26 %, p95 3.87 %, max 5.07 %.
- **Git ancestry:**
  - `0ca312bd`→`cbd86a65` is L11 C3.
  - `cbd86a65`→`6dd1f916` changes only plugin wiring (`plugin.rs`, `lib.rs`, `body_set.rs`) and small ECS query edits. There is no broadphase, narrowphase or solver change.
  - `6dd1f916`→`983480a9` is C3b.
  - `983480a9`→`4db26681` is exactly L9 C0–C3.

**Against the driver's `raw/tables.md`:** every cell, spread, ratio and claim flag reproduces. My disagreements:
1. **P3, "attribution A REFUTED (ratio > 0.55 or t_q > 0.21)".** In the P3 block neither condition holds under the rule as written (`c3b/design.md:222`: "> 0.21 ms …, claimed under SE"). The ratio is 0.520, and t_q = 0.2102 is +0.10 % against a 2·s bar of 0.48 %, so it is not claimed. The refutation does hold, but from the P5g block: 0.2354, +12.1 %, Y/Y/Y. The driver printed that cell as a "same-block repeat" without comparing; the two blocks differ by +11.96 %, claimed (§3.2).
2. **P3, "C2 not built (t_q < 0.235)".** That is true only in the P3 block. In P5g, t_q = 0.2354 ≥ 0.235 by the letter, though not claimed above.
3. **G9 per-stage spans:** the driver reads A, while the gate convention is B. The values differ (e.g. warm_apply on R at W1: A 0.7532→0.4597, B 0.7395→0.4488), but no verdict changes.
4. **The driver omits the S16 `store` reading** (+20 ns under B). It is not a FAIL.
5. **The parent's canary:** it is not seen under this window's two-spread rule, but it is seen under G9's own SE form.

## 1. P1: the L10 pre-C0 refutation

**Bar** (`L10-sleeping/08-DESIGN-REV2.3.md:221`; `07-REVIEW-OF-REV2.2.md:162`; `09-REVIEW-OF-REV2.3.md:22`; plan.md:111-126): if Off′(1) < 0.584 ms (= 0.6×0.59 + 0.23), stop and escalate before C0.

| b4db, W=1, armed, K=6 | [264,1000) | all-frozen tail | ffs |
|---|---|---|---|
| **L10-Offp** (Off′: Tree, reuse on, L11, L5, sleeping) | **1.4670** [1.4566-1.4826] (1.77/0.88/0.34) | **1.3043** [1.2935-1.3218] (2.16/0.81/0.40) | 274 |
| L10-Son (AllPairs, reuse off) | 4.6973 [4.6759-4.7127] (0.78/0.37/0.15) | 4.6970 | 265 |

- **Arithmetic:** Off′ exceeds the bar by +0.883 ms (+151 %) over the window and by +0.720 ms (+123 %) over the tail. The smallest process is 2.2–2.5× the bar, and both readings are claimed above it under r/i/s.
- **Verdict: the refutation does NOT fire. Continue L10.**
- **The two readings disagree by 0.163 ms (−11.1 %).** The literal window includes the ten awake steps [264,274), at 13.3 ms each: solve 0.381 against 0.220, wide colours 0.148 against 0.
- The design's Off′ is the "all frozen" tail (`06-DESIGN-REV2.2.md:253`), so **1.304 ms is the matching number**. Both readings sit inside the design's 0.82–1.59 band.
- Per logical manifold: 0.292 µs, against the design's 0.18–0.35.
- **Off′ against J-Son:** −3.230 ms, Y/Y/Y. Of that, bp is −1.722 (the Tree) and np −1.677 (L9b). **That drop is the Tree plus L9b, not L10.**

**Off′(1) tail spans (A) against the budget (`06-DESIGN-REV2.2.md:255-262`) and the rev-2 floor (`04-DESIGN-REV2.md:338-348`):**

| term | measured | design Off′ | floor | removed by |
|---|---|---|---|---|
| bp | **0.2520** (query 0.1949, assemble 0.0280, build 0.0235, verify 0.0054) | 0.09–0.24 (0.012 over the top) | verify 0.003–0.005 | **C3c** (Tree seam): 0.247 |
| np | **0.6438** | 0.48–0.80 | 0 | **C3b**: 0.644 |
| graph | **0.1160** | 0.12 | 0.010–0.020 | **C3b**: 0.096–0.106 |
| solve | **0.2196** (store 0.080, integrate 0.067, solve_build 0.038, sleep 0.024, gravity 0.007) | 0.07–0.30 | 0.015–0.030 | **C3a** (already on u/phys-l10 as `3c059ca6`) plus C3b's frozen stream: 0.190–0.205 |
| remainder | **0.0687** (gather 0.027, g 0.037, apply 0.005) | 0.064–0.134 | stays | none (needs U6/X-5) |
| new cost: A1.3 | — | — | +0.010–0.025 | C3b |

- **Ceiling at W=1:**
  - Measured-basis floor = 0.109–0.149 ms.
  - **L10's gain ceiling on the J-Son tail = 1.155–1.195 ms**, 89–92 % of Off′. It splits as C3b ≈ 0.715–0.740, C3a ≈ 0.190–0.205 and C3c ≈ 0.247.
  - The realized-gain gate is 0.35 ms (`:267`), so the ceiling clears it 3.3×.
- **Ceiling at W=8:** Off′(8) = 0.817 (tail 0.780; the design said 0.42–0.93), so Δ(8) = 0.60–0.70 ms against a 0.14 ms gate.
- **What the ceiling excludes:**
  - L10 cannot recover today's J-Son 4.70 → Off′ 1.30. That needs the Tree as default (the tree lane's C4) plus L9 C4.
  - C3c's 0.247 exists only with the Tree; on AllPairs the bp stays at 1.975 ms.

## 2. P2: L9, the C0 reading and the C4 decision

### 2.1 (a) The class-bench band (`L9-contact-reuse/02-DESIGN-REV1.md:380`; plan.md:128-149)

- **Rule:** FIRES iff high < 1.0; AMBIGUOUS iff low < 1.0 ≤ high; CLEAR iff low ≥ 1.0.
- **Per-class costs on J (ns/pair, K=6):**

  | class | cost |
  |---|---|
  | separated | 45.24 [44.94-48.35] |
  | **touching** | **435.56** [434.09-440.89] (1.56/0.39/0.28) |
  | stream | 232.69 |

  - Counts: N_sep = 5,044, N_touch = 4,515.
  - Rest: separated 52.41, touching 456.09. Rain, fast: 208.45.
- **Band, per process, median over K:**
  - low **1.0771** [1.0729-1.1125] (3.68/0.60/0.71);
  - high **1.6659**.
  - low = 5,044×(45.24−60) + 0.80×4,515×(435.56−100) − 0.06 = −0.074 + 1.212 − 0.060 = 1.077 ms;
  - high = 0.052 + 1.645 − 0.030 = 1.666 ms.
- **Verdict (a): CLEAR.** The low end is claimed above 1.0 under r, i and s (+7.7 % against 2r = 7.4 % and 2s = 1.4 %). All six per-process lows are ≥ 1.073.
- **Caveat: on 4db26681 the separated class is post-L9a** (45 ns, against the design's pre-L9a 150–220 ns). Its term is −0.074..+0.052 ms, so the band prices essentially L9b alone.
- **L9a is already realized** (cross-binary):
  - J-A at W1: g9t (pre-L9) 18.929 → b4db reuse-off 18.131 = −0.797 ms, Y/Y/Y.
  - np span: 3.076 → 2.409 = −0.667 ms.
  - Total L9 np: 3.076 → 0.685 = **2.39 ms**, against the design's Δt_np(1) of 1.4–2.5.

### 2.2 (b) Same-binary off/on and the realized-gain rule (`02-DESIGN-REV1.md:407-410`)

- **Prediction:** 0.9897 × 4,515 × (435.56 − [100, 60]) ns = **[1.4994, 1.6782] ms**.
- **Bar:** 0.6 × 1.4994 = **0.8997 ms** (0.896–0.914 across t_touch's min and max).

| J-A, b4db | off | on | ΔT | effect; bars r/i/s; claimed |
|---|---|---|---|---|
| W1 [0,500) | 18.1313 | 16.5068 | +1.6245 | −8.96 %; 2.3/0.8/0.4; Y/Y/Y |
| **W1 [100,500)** | **18.1605** [18.1324-18.3111] | **16.3152** [16.2494-16.3513] | **+1.8453** | **−10.16 %; Y/Y/Y**; worst pairing +1.781 |
| W1 [0,100) (witness) | 18.0178 | 17.2815 | +0.736 | Y/Y/Y (faster) |
| W8 [0,500) / [100,500) | 6.0419 / 6.0539 | 5.8998 / 5.8750 | +0.142 / +0.179 | n/Y/Y |

- **Armed twins at W1, [100,500):**
  - wall 18.1846 → 16.3245, ΔT = 1.860 ms, Y/Y/Y;
  - **np span Δt_np(1) = 2.4093 → 0.6846 = 1.7247 ms** under A (1.741 under B), Y/Y/Y;
  - solve −0.110 ms (fewer manifolds, 4,519.26 → 4,467.67), bp unchanged;
  - reused 4,457.5 and full 46.7 per step, so h ≥ 0.9896.
- **Verdict (b): PASS.** ΔT(1) = 1.845 ms, claimed, is **2.05× the bar**. The worst pairing passes too.
  - Against the prediction: 1.23× its low end. Δt_np is 1.15× the low end and 1.03× the high end.
- **Per contact:**
  - np on: 0.153 µs per manifold (design 0.14–0.39, `:328`) and 71.7 ns per pair (budget 41–139 ns).
  - J-A pre-L9 → on is −12.8 %, against the design's −7..−13 % (`:318`).
  - At W8 the gain (−0.14 to −0.22 ms) sits just below the design's −0.26..−0.55 (`:321`). No gate is attached to that line.
- **G-TW's other L9b rows** (P5d/e/f; on/off; all Y/Y/Y):

  | row | W | effect |
  |---|---|---|
  | J-D | 1 / 8 | −19.57 % / −4.86 % |
  | R | 1 / 8 | −27.16 % / −6.09 % |
  | J-A | 2 / 4 / 16 | −7.13 / −5.28 / −2.06 % |

  - The [0,100) witness is faster or not claimed at every W.
  - **There is no claimed regression at any W**, so the blocking rule does not fire.
- **Poses:**
  - off rows = C0 fixtures (gate 10 of 10);
  - on rows: J `0x30c5438bc6ad9ffa` across W 1/2/4/8/16 and cfg a/default;
  - R `0xc8bbe34cf6a8afc6` at W 1/8.

### 2.3 The C4 decision

**BUILD C4.** Every input says go:
- (a) is CLEAR;
- (b) passes;
- G-L9b-6 already passed (h_J = 0.9897 ≥ 0.7, `…/l9/h_tau.md:3`);
- G-TW found no regression anywhere.

**What C4 changes** (`02-DESIGN-REV1.md:384`; D11 at `:161-165`):
- `PhysicsConfig::contact_reuse` becomes `true` by default; τ stays 1 mm.
- "The only commit that moves values; re-pins under each file's own rule." `--contact-reuse off` must still reproduce the C0 hashes.

**Pins it moves** (`:439-447`, plus `…/l9/c3.md:14`):
1. **The runner's pose fixtures.**
   - J default/cfg-a changes `0x32d5e235342b4143` → `0x30c5438bc6ad9ffa`; R `0x87e561d2…` → `0xc8bbe34c…`; J-Son and R-S move too.
   - Every consumer of `0x32d5…` moves as well, unless it sets reuse off explicitly: the tree lane's kept J-pose pins (`c3b/design.md` §7) and every future window's J500 fixture.
   - Across C4, cross-window bridges must use reuse-off rows.
2. **`default_world_pyramid_determinism.rs` `PINNED_FINAL_HASH`** (release and debug) and **`A7_R1_D_MAX_BITS`**. A7-R1's reuse-off run must still read `0x3a3c_3896` exactly.
3. **A7-R1's D_max docs** (`sleep_settles_box_piles.rs:64-72`, `:96-127`), under its bands: < 2 mm confirms, 2–10 mm is acceptable, ≥ 10 mm STOPs.
4. **A7-R2's freeze step** (`sleeping_pipeline.rs:23-25`).
5. G2/G7 counts and G8's freeze step.
6. **H8's manifold count** (4,519.26 → 4,467.67 on J; 6,662 → 6,914 on R, +3.8 %) and every per-manifold denominator.
7. Box-pile goldens. Tests of the exact narrowphase set `contact_reuse: false` explicitly.

**C4's own gates:**
- G-L9b-7 plus `contact_reuse_bounds.rs`;
- `frozen_island_warm_start` ×6, `support_loss_wakes_sleepers`, `sleeping_pipeline`;
- FEATURE_MAP and SYSTEMS list `reuse.rs` and `carry.rs`.

**Still owed to G-TW after C4:**
- the canary on the L9 binary;
- J-D and R at W 2/4/16;
- the armed W8 per-contact table;
- the formal A/B on the C4 binary. This window's A/B ran on C3's switch, which is the same code path.

## 3. P3: the C3b query cost (par 6dd1f916 RowWalk against tip 983480a9 LeafList)

**Rules:**
- `c3b/design.md:215-226`: `:222` "> 0.21 ms, claimed under SE", `:226` "into the band, ≤ 0.186 claimed";
- the actions at `:365-376`, F3 at `:168-172`, C2 at `:228-236`;
- the span limits at `treebp/g4_g5_recipe.md:208`;
- plan.md:151-170.

c_q = t_q / 1,240 queried rows.

### 3.1 The P3 block (09:40-09:56)

| W | par t_q | tip t_q | tip/par | tip c_q | span par→tip (limit) |
|---|---|---|---|---|---|
| 1 | 0.4044 K=5 | **0.2102** [0.2095-0.2120] (1.18/0.60/0.24) | **0.520**, Y/Y/Y | **169.5 ns** | 0.4637 → **0.2714** (0.36): PASS |
| 8 | 0.4126 | **0.2129** | **0.516**, Y/Y/Y | 171.7 ns | 0.4738 → **0.2758** (0.35): PASS |

- **Step effect, tip against par:**
  - armed T: −1.0 % (W1, n/n/Y) and −5.0 % (W8, n/Y/Y);
  - T-D tree: 6.946 → 6.814 (−1.9 %, n/n/Y) and 2.618 → 2.431 (−7.2 %, n/Y/Y).
- **Criterion, same binary** (P5c), LeafList/RowWalk, all < 1 and claimed:

  | case | ratio |
  |---|---|
  | **j100** | **0.330** [0.3289-0.3311] |
  | 1240 | 0.330 |
  | uniform | 0.525 |
  | disparity | 0.457 |

### 3.2 The same cell in the P5g block (11:20-11:31) does not reproduce at W=1

- `G5-TA-tree-a` on the tip is the same binary with the same arguments as `C3b-TA-armed`. The command lines differ only in the output paths (648 against 654 characters; `analyst_argdiff.py`), and the config is identical.
- **t_q at W1 = 0.2354** [0.2325-0.2399] (c_q 189.8 ns), against 0.2102. That is **+11.96 %, Y/Y/Y**; the span moves +13.5 %, Y/Y/Y. W8 reproduces (0.2094, −1.7 %).
- **It is not within-block noise.** The within-block ranges are 1.2 % and 3.1 %, and the two blocks' value ranges do not overlap.
- Both blocks ran the W=1 process on the same logical CPU (core 10).
- P5g was otherwise faster: its clock read 133.4–133.9 % against 130.0 %, with np −5..−7 % and T −1.6 %. **Only the query span moved, only at W=1, and against the machine's direction.**
- Untested candidate: an address-layout effect of the launch context (path lengths are constant within a block; Mytkowicz et al. 2009).
- **Pooled K=12:** 0.2222 [0.2095-0.2399] (13.65/11.19/2.15), c_q 179.2 ns.

### 3.3 The action table applied

| reading | P3 block | P5g | pooled | action |
|---|---|---|---|---|
| LeafList/RowWalk < 1 (J cell and armed rows) | 0.330 same-binary; 0.520/0.516 cross-binary | — | — | **SHIPS** (robust) |
| ratio ≥ 1 | no | — | — | not rejected |
| > 0.55, or t_q > 0.21 claimed | n/n/n (+0.10 %) | **+12.1 %, Y/Y/Y** | +5.8 %, SE only | **attribution A REFUTED.** The measured c_q (170–190 ns) matches attribution B's disparity-calibrated range (172–192, `:354`), while the bench ratio (0.330) matches A's model (0.334–0.350) |
| t_q ≥ 0.235 → build C2 | −10.5 %, claimed below | **0.2354, on the bar (not claimed)** | −5.4 %, SE only | **not robust** |
| D6 re-read (`broadphase/04-DESIGN-REV2.md:204-214`) | trigger 0.157 at T(8) 2.431 | trigger 0.146 at T(8) 2.247 | — | **fires in both.** Saving = t_q(8)·(1−1/(8·0.681)) − 0.0065 = 0.167 / 0.164 ms = 6.9 / 7.3 % of T(8); C2's own gate needs 68–73 % of it realized. **Flagged** |
| c_q > 150 ns → F3 | **169.5**; +13.0 % over 0.186, Y/Y/Y | **189.8**, Y/Y/Y | 179.2 | **F3 TAKEN UP** (robust) |
| into the band | no (claimed above) | no | no | — |
| tree span ≤ 0.36/0.35 | 0.271/0.276 | 0.308/0.267 | 0.290/0.275 | **PASS** (robust) |

- **What F3 is:** a kd median-split leaf order replacing `morton_sort`. It splits top-down on the widest centroid axis, gives the left part a multiple of 8 rows, and breaks ties by row index.
- **Its model:** 554 instructions and 6.5 mispredicts per J row, a ratio of 0.254–0.270 against F1+F2's 0.334–0.350.
- **Its bar:** "reserve; only if the re-time reads c_q > 150 ns". It reads 169.5–189.8 ns.
- **Its risk:** it changes the build (which must stay at 15–25 µs; measured 23.5 µs) and the slot and stream order. `ContactPairs` and the pose must not move.
- **Arithmetic only:** t_q × 0.726–0.808 = 0.153–0.170 ms (P3 basis) or 0.171–0.190 (P5g basis).
- **In situ against the bench:** the runner's LeafList span is 1.88–2.13× the bench pass, while RowWalk's is 1.06×. So the bench's 3× gain becomes 1.7–1.9× in situ, and F3 and C2 must be judged in the runner.

## 4. P4: the L11 G9 remainder (g9p against g9t; `L11-solve-setup/02-DESIGN-REV1.md:359-371`; window 5 analysis.md:227)

| row@W | parent | tip | effect | claimed r/i/s |
|---|---|---|---|---|
| J-A@1 | 18.9490 | 18.9285 | −0.11 % | n/n/n |
| J-A@8 | 6.1700 | 6.1279 | −0.68 % | n/n/n |
| R@1 | 11.3311 K=5 | 10.9177 | −3.65 % | n/Y/Y |
| R@8 | 5.7155 | 5.4469 | −4.70 % | n/n/Y |
| R-S@1 | 6.0832 | 6.0552 | −0.46 % | n/n/n |
| R-S@8 | 3.3061 K=5 | 3.2761 | −0.91 % | n/n/n |
| S16@1 | 0.0825 K=5 | 0.0822 | −0.32 % | n/n/n |
| J-As@2 | 6.5329 K=5 | 6.2901 | −3.72 % | n/Y/Y |
| J-As@4 | 5.2088 | 4.9884 | −4.23 % | n/Y/Y |
| J-As@16 | 4.8798 | 4.6192 K=5 | −5.34 % | **Y/Y/Y** (faster) |
| J-As-a@1 | 8.9108 | 8.5486 | −4.07 % | n/n/Y |
| J-C@1 | 9.3803 | 8.9966 | −4.09 % | n/n/Y |

- **No row is claimed slower, under either rule. There is no FAIL.**
- **Stages (reading B):**
  - **warm_apply:** J-As-a W1 **0.5278 → 0.2979 (−0.2299 ms, Y/Y/Y)**, which passes the −0.19 bar again. R W1 is −0.291 and R W8 −0.301 ms, both Y/Y/Y.
  - **Wide colours are not claimed slower:** −4.65 % on J-As-a, −1.70 % (R W1), +0.69 % (R W8), all n/n/n.
  - **solve_build and store** are not claimed, except R-S store −11 % (the tip is faster). **S16's store goes 60 → 80 ns** (B, Y/Y/Y; under A +20.7 %, n/Y/Y). That is two timer quanta and 0.02 % of the step, which itself is not claimed. It is a reading, not a FAIL.
- **Canary:**
  - Its span reads 1.0008× the injection on the tip and 1.0006× on the parent, with every process within 0.42 %.
  - Step rise: tip +0.4481 ms (105 % of the injection), **Y/Y/Y**; parent +0.4694 (106 %), n/n/Y.
  - Wall − canary closes to within 0.27–0.36 % (n/n/n).
  - **SEEN:** on the tip fully, on the parent under G9's own SE form (`:360`).
- **Is G9 closed? Not formally.** Everything run passes, but J-A at W 2/4/16 is not run (plan.md:219), and the K=12 letter is not met (every cell is K=6 or 5).

## 5. P5: the extras

| item | reading | verdict |
|---|---|---|
| P5a bridge 2 (w4bt against g9p, J-As) | W1 +0.16 % (n/n/n); **W8 +1.75 % (+0.078 ms), n/n/Y** | **HOLDS.** The line merge's W8 cost is ≤ 1.75 %; window 5's +11.5 % was mostly machine state |
| P5b L10 spans | Off′(8) 0.817 (tail 0.780); J-Son(8) 2.799. **R-S Off′** 1.594 (W1) / 0.966 (W8): bp 0.245, np 0.835, graph 0.188, solve 0.266. R-S 5.778 / 3.121 | Reading. **The R gate is 0.6×(Off′_RS − 0.25) = 0.806 / 0.429 ms** |
| P5g G5 on the tip, same binary | **Δbp(8) = 1.698 (A) / 1.702 (B) ms, against a ≥ 1.03 bar** (`g4_g5_recipe.md:205`); W1 1.808. T-D tree against AllPairs −22.6 % / −44.2 %, and T-A −10.3 % / −30.6 %, all Y/Y/Y. Span 0.308/0.267 | **PASS** |
| P5d J-D off/on | −19.6 % / −4.9 %, Y/Y/Y | PASS |
| P5e R off/on | −27.2 % / −6.1 %, Y/Y/Y; manifolds +3.8 % | PASS (the manifold rise is a C4 re-measure item) |
| P5f J-A at W 2/4/16 | −7.1 / −5.3 / −2.1 %, Y/Y/Y | PASS |
| P5c G4 criterion | j100 0.1446 against RowWalk 0.4388 = **0.330**; uniform 0.525 (model 0.38–0.39); disparity 0.457. Rule 1: 0.1446 ≤ 0.30 | **PASS** (none ≥ 1, J ≤ 0.55, uniform and disparity within [0.30, 0.60]) |
| Reading: the tip's tree default row against Jolt v5.6.0 (window 3: 9.8282 / 2.5693, recomputed) | W1 0.693× (P3 block) / 0.662× (P5g), n/Y/Y. **W8 0.946× (P3, n/n/n) / 0.874× (P5g, n/Y/Y)**; the AllPairs default is 1.566×, Y/Y/Y | Reading. At or below Jolt at W8, but not claimed under min-max |

## 6. The bridges

**Same-binary:**

| bridge | earlier | window 6 | effect | claimed r/i/s | holds? |
|---|---|---|---|---|---|
| w5 J-As parent@1 → B2 g9p | 8.8591 | 8.7571 | −1.15 % | n/n/n | yes |
| w5 J-As parent@8 → B2 g9p | 4.7257 | 4.5288 | −4.17 % | n/Y/Y | yes, under the window rule |
| w5 J-As-a parent@1 / tip@1 | 8.8211 / 8.4727 | 8.9108 / 8.5486 | +1.02 / +0.89 % | n/n/n | yes |
| w5 warm_apply(B) parent / tip | 0.5285 / 0.2960 | 0.5278 / 0.2979 | −0.13 / +0.64 % | n/n/n | yes: **the gate reproduces** (−0.2325 → −0.2299) |
| w4b J-As tip@1 → B2 w4bt | 8.6973 | 8.7433 | +0.53 % | n/n/n | yes |
| w4b J-As tip@8 → B2 w4bt | 4.2399 | 4.4508 | +4.97 % | n/n/Y | yes, under the window rule |

**Window 5's tip default row, across near-identical code:**

| bridge | earlier | window 6 | effect | claimed | holds? |
|---|---|---|---|---|---|
| HL-D-tree@1 → par | 7.1062 | 6.9458 | −2.26 % | n/n/n | yes |
| HL-D-tree@8 → par | 2.7160 | 2.6182 | −3.60 % | n/n/n | yes |
| HL-D-allpairs@1 → tip | 8.7823 | 8.4040 | −4.31 % | n/n/Y | yes |
| HL-D-allpairs@8 → tip | 4.2957 | 4.0237 | −6.33 % | n/n/Y | yes |

**Window 4 (cross-binary):**

| bridge | earlier | window 6 | effect | claimed | holds? |
|---|---|---|---|---|---|
| t_q(B)@1 → par | 0.4140 | 0.4044 | **−2.32 %** | **Y/Y/Y** | **FAILS** |
| t_q(B)@8 → par | 0.4110 | 0.4126 | +0.37 % | n/n/Y | yes |
| RowWalk j100 | 0.4430 | 0.4388 | −0.95 % | n/·/Y | yes |

**Window 4b on the g9p binary (readings, not bridges):**
- J-A +0.09/+4.4 %, R +1.5/+9.0 %, R-S +2.6/+6.2 %, S16 +3.1 %, J-As W2/4/16 +5.0/+5.8/+12.0 %.
- None is claimed under the window rule. They are consistent with bridge 2 plus a ≈+5 % window term at W8.

**In-window duplicates:**
- The tip's armed-tree t_q: **+11.96 % at W1 (Y/Y/Y), FAILS**; −1.7 % at W8.
- The tip's T-D tree: −4.5 % at W1; **−7.6 % at W8 (Y/Y/Y), FAILS**.

**What that invalidates:**
1. **No same-binary cross-window bridge fails under the window rule.** At W=1 they all hold within 1.2 %.
2. **At W=8, SE alone sees a ±4–6 % window term.** That qualifies every cross-window W8 ratio: tree/Jolt 0.874–0.946 here, window 5's 1.057, and the L11 chain at W8.
3. **Window 4's RowWalk t_q at W1 fails (−2.3 %).** Stop quoting 333.9 ns/row as today's parent cost; the par reads 326.1 (W1) / 332.7 (W8). C3b's calibration shifts by about 2 %, which is negligible.
4. **The in-window duplicates invalidate:**
   - single-block absolute-threshold decisions (P3's 0.21/0.235/0.186 lines and D6) — ship, F3 and the span PASS survive, C2 does not;
   - single-block absolute T(8) headlines.
   - They do **not** invalidate interleaved within-block comparisons.

## 7. What this window cannot claim

1. **L10's own gain.** C0–C3a are untimed; §1's split by commit is arithmetic.
2. **L9's G-TW on the C4 binary.** Not run: the canary on the L9 binary, J-D and R at W 2/4/16, the armed W8 table, and L9a's C0-vs-C2 rows.
3. **A same-binary armed C3b A/B.** The runner has no `--bp-kernel`, and the cause of the W1 shift is unknown.
4. **G9's full letter:** J-A at W 2/4/16 and K=12 are not run.
5. **The Tree as the shipped default, and any re-run Jolt.** Every Jolt ratio is cross-window against window 3.

## FOLLOW-UP WORK ITEMS

**P1: CONTINUE L10**
1. **u/phys-l10 C3b** (worktree `D:/wt/merge`, HEAD `3c059ca6`).
   - It adds Sets (`06-DESIGN-REV2.2.md:318`) and rev 2.3's D-F/D-G/D-H/A3′/B1′/S8b (`08-DESIGN-REV2.3.md:222-226`).
   - Value change: none; the Off rows must equal Son-J1000 `0xcc2a…` and RS800 `0x2a2b…`.
   - Gates: the S3 arms 4b/7/8, the comparator, and the C3b rehearsal Δ ≥ 0.35 ms.
   - Targets: np 0.644 → 0, graph 0.116 → 0.01–0.02.
2. **u/phys-l10 C3c,** the Tree seam (bp C5 T1–T6).
   - Value change: none. Target: bp 0.252 → 0.005.
   - It interlocks with the tree lane's default flip.
3. **After L9 C4 lands,** u/phys-l10 merges it and **re-records its lane-base fixtures** (`b984a2af`; the design already expects this, `06-DESIGN-REV2.2.md:308`).
4. **Quiet window:** L10's G-TW after C3b/C3c.
   - Gates: J-Son ≥ 0.35 ms (W1) / 0.14 (W8); **R-S ≥ 0.806 / 0.429 ms**.
   - Ceilings to beat: 1.155–1.195 ms / 0.60–0.70 ms.
   - Plus C1b's queued timing.

**P2: BUILD L9 C4**
5. **u/phys-l9 C4:** `contact_reuse = true` (`02-DESIGN-REV1.md:161-165`, `:384`).
   - **Pins it moves:**
     - the J/R/J-Son/R-S pose fixtures (J500 → `0x30c5438bc6ad9ffa`, R1100 → `0xc8bbe34cf6a8afc6`);
     - `PINNED_FINAL_HASH` (release and debug) and `A7_R1_D_MAX_BITS` (the reuse-off run stays at `0x3a3c_3896`);
     - A7-R1's docs, A7-R2's freeze step, G2/G7/G8;
     - H8 (J 4,519.26 → 4,467.67; R 6,662 → 6,914) and the per-manifold denominators;
     - box-pile goldens;
     - the tree lane's J-pose pins.
   - **Gates:**
     - G-L9b-7 plus `contact_reuse_bounds.rs`, and the A7-R1 bands (A7-R2 must freeze);
     - `frozen_island_warm_start` ×6, `support_loss_wakes_sleepers`, `sleeping_pipeline`;
     - the `--workspace --all-targets --no-fail-fast` gates;
     - FEATURE_MAP and SYSTEMS updates.
6. **Quiet window:** G-TW on the C4 binary.
   - Off/on on J-A, J-D and R at every W, the canary on the L9 binary, and the armed W8 table: about 15 minutes.
   - From here on, bridges use reuse-off rows.

**P3: SHIP C3b, TAKE UP F3**
7. **New branch off integ/unified** (for example `u/treebp-f3`; lock set `broadphase_tree/**` and the benches): **F3**, the kd median-split leaf order.
   - What moves: **G-LL3's count pins** (`c3b/design.md:208`: J 1953/5663/155/6216/15575/9564, uniform, disparity), the stream order, and the build span (must stay at 15–25 µs).
   - What must stay: the pose, GOLDEN `0x1575_326A_EB80_3052`, TreeDiag, the Vec census and UG-02.
8. **The runner's `--bp-kernel` flag** (`c3b/design.md:253`, `:387`; the orchestrator's call), before F3's window.
9. **C2 is flagged, not built.** Decide it after F3's re-time, when D6 is predicted near its edge (5.0–6.1 % of T(8)).
10. **The tree lane's C4 default flip** (`broadphase/04-DESIGN-REV2.md:216-220`).
    - Rule 1 and the span gate are now cleared.
    - Rules 6, 8 and 9 remain (`c3b/design.md:237-241`).
    - It moves every default-row timing, but not the pose.
11. **Quiet window:**
    - (a) the F3 A/B (armed W1/8 plus G4);
    - (b) the W1 t_q block-shift test: two separated blocks plus a padded-path variant, for a pooled K=12 figure before C2;
    - (c) the owed `TREE_BRUTE_MAX_ROWS` / `AUTO_TREE_LO/HI` 64–256 refinement (`c3b/design.md:244`).

**P4: PASS, G9 not formally closed**
12. **Quiet window, about 6 minutes:** J-A at W 2/4/16, g9p against g9t, K=6.
13. **Ruling:** is K=12 (L11 `:360`) required? If so, +6 processes per cell, about 30 minutes.
14. No code follow-up: L11 C3 moves no pins, and the S16 `store` reading needs no action.

**Bridges and protocol**
15. Read absolute-bar decisions (C2, band entries, the Jolt headline) from two separated blocks, or pooled at K=12, and carry the ±5 % W8 window term when the comparison crosses windows. This is the orchestrator's call.

## 8. Open points and files

1. C2: the letter is unresolved and D6 fires. Recommend F3 first.
2. The W1 query-span block shift is the window's largest unexplained number. It moves C2, but no ship or refute decision.
3. L9 C4's re-pins reach two other lanes (the tree lane's pose pins and u/phys-l10's fixtures). Sequence the merges.
4. G9's K=12 letter against the window protocol's K=6.
5. Every verdict is the same under SE alone, except the parent canary's wall rise.

Files are in `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/7dc37fc3-7b1e-46f1-89da-ccd0dba7c788/scratchpad/win6/`:
- `analysis.md` — **not written: the harness blocks report files for this role; the text above is its content**
- `tools/analyze6.py`
- `analyst_argdiff.py`
- `analyst/tables.txt`
- `analyst/reduction.json`
- `analyst_run.log`
