# Physics window 8b: synthesis (results-analyst, 2026-09-29)

The window ran on 2026-09-29 from 17:21 to 21:09 on the owner's workstation (8 physical / 16 logical cores, windows-msvc). The driver finished with exit 0, "complete", and its counts with exit 0.

## 1. Headline

Window 8b ran 2026-09-29 17:21–21:09: 894 timed processes, 0 invalid, 0 voided passes, and 26 of 786 slots dropped because the desktop and agent sessions were busy. It settles four of its six questions; the other two rest on one protocol reading.

- **S4 holds and stays.** At W8 on J-T the trunk (16191fda) is 0.1705 ms (8.58 %) faster than its S4-off parent, STRONG, and no W is slower. The claim rests on the pre-registered span route, because the block's own 0.060 ms rung was not seen.
- **S7's partial form fails and does not merge.** Capping lanes at the physical cores leaves W8 unchanged but makes W16 slower, STRONG: +0.1400 ms (+6.83 %) on J-T and +0.6579 ms (+12.63 %) on J-A. That is the opposite of the prediction, and R_16 = 0.0525 ms was demonstrated in the same block.
- **F3 does not become the default.** Its kd order cuts the J query by 16.88 % (R1, STRONG), but the kd build eats the gain (R3 not claimed), so LeafList stays the default.
- **The tree thresholds need re-ruling before tree C4.** The G4 block moves the tree/all-pairs crossover down from 144/152 to 128–136.
- **C1b and C6 are kept.** Sleeping on the tree broadphase (C1b) shows no resolved awake cost and cuts the settled pile by 96.5–99.1 %. DM1's C6 is kept.
- **S1 inputs.** The ω_b v2 re-bench puts the S1 region's net barrier at 0.35–1.07 µs per stage at 8 participants. That is about today's dispatch cost per wave (0.91–0.95 µs). Parking costs about 6 µs per stage after any serial stretch longer than 5–20 µs. On the design's own formula, S1's gain over an S4-pattern interim (S2i + S3i) straddles the 5 % build-if; which side depends on how the interim is priced.
- **The K = 2 reading.** F3's R2, four disarmed J-Son-T product rows and the gap-20/80 park price are NOT CLAIMED by the pre-registered letter. The only reason is that a contaminated pass kept K = 2. If a K = 2 pass may gate, all of them are STRONG. Of the decisions, only F3's branch label moves with that ruling.
- **Jolt.** 8b has no Jolt row, so the standing against Jolt 5.6 is not re-measured.

## Method

**Sources.** Every number in this file was computed by a synthesis script run on `raw/`. The scripts and their outputs are in `analysis/synth/`:

| script | output | what it covers |
|---|---|---|
| `synthlib.py` | library | shared parsers and statistics |
| `s0_validity.py` | `s0_validity.txt` | processes, validity, clean, slots, K, idle waits, R4, receipts, binaries |
| `s1_runner_claims.py` | `s1_runner_claims.txt` / `.json` | every runner claim under both K readings, the trunk walls, the cross-block check |
| `s2_micro.py` | `s2_micro.txt` | ω_b v2, the park rule, participation, ω(W, gap), v1 continuity |
| `s3_g4_dm1.py` | `s3_g4_dm1.txt` | G4 thresholds and R3d, the DM1 gate |
| `s4_s1_arith.py` | `s4_s1_arith.txt` | S1 arithmetic on 8b's own inputs |
| `s5_blockdrift.py` | `s5_blockdrift.txt` | block-level drift |
| `s6_extras.py` | `s6_extras.txt` | SPLIT shares, c_q, L10 pieces, paired T/P, trunk scaling, L_wide at W8/W16 |

**Parsers.** The window's own: `tools/lib/driver.py` (`load_csv`, `parse_summary`) and `tools/micro8b.py` (`parse`, `process_rules`).

**Agreement with the group sections.** The synthesis reproduces every group verdict and the numbers they share, with two exceptions:
- The K reading is applied uniformly across the window (see below).
- The L10 share at W8 is 0.52 % here, measured against the disarmed OFF step. `jsondm1.md`'s 0.50 % uses the armed OFF step.

**Statistics (window 8's `lib8`).**
- A cell is the median over K of the per-process values.
- The spreads are, each relative to the median: i = the IQR (inclusive quartiles), s = 1.2533·SD/√K, r = min–max.
- For B against A, a flag is set iff |B/A − 1| > 2·hypot(A_x, B_x).

**Ruling 1.**
- CLAIMED iff i AND s flag pooled (K = 9) AND in each of the three passes, with one sign throughout.
- STRONG iff r also flags, pooled and in every pass.
- Flags are printed in i/s/r order. `s7.md` and `s4split.md` print r/i/s.

**Per-process values.**
- Runner rows: the mean of `wall_ns` over the window.
- F3 armed spans: the median over steps [100,500), as pre-registered.
- ω_b v2: (region@72 − region@36)/36, taken within one process.

**Per manifold.** Every J-T/J-A process in S7-AB and S4-AB has the same pose and the same manifold sequence, so ratios per manifold equal the wall ratios. Absolute ns per manifold are given where they are used.

**Slot rule (windows 7/8).** Take the original if it is valid and clean, else its re-run if that is valid and clean, else drop the slot. Clean means both receipts ≤ 5 %, the witness ≤ 2 %, and no build process. Placement receipts are recorded and never used.

**The window-wide reading of "every clean block" is LETTER.**
- Every non-voided pass gates. No pass was voided.
- A pass-cell with K < 3 sets no flag. This is window 7's "No claim when either side has K < 3", which `lib8.cmp_` implements; ruling 1 never lifted it.
- The F3 reconciliation (D2) found this to be the pre-registered reading, so the synthesis applies it to every block. Two group headlines that used K2 are relabelled here: J-Son-T's disarmed product rows, and ω's PARK-20/80.
- K2 means a K = 2 pass-cell gates on its own i and s. It needs an orchestrator ruling (PC-8b-1).

The comparisons whose verdict differs between the two readings:
- re-derived here: R2 (TD t_q W1), TD t_b W1, the four disarmed J-Son-T G/F rows (W1 and W8), and the four PARK-20/80 cells;
- added by the F3 reconciliation: TA t_q and t_b at W1, and the two TR bridges.

**Appendices, by reference.** Each is the corrected, verified version.
- `analysis/s7.md` — S7-AB.
- `analysis/s4split.md` — S4-AB and SPLIT.
- `analysis/omega.md` — ω_b v2, ω(W, gap), v1 continuity.
- `analysis/f3.md` — F3 and F3-G4, as corrected after verification (R2 read by the letter; 16 pass-cells with K = 2).
- `analysis/jsondm1.md` — J-Son-T and DM1.

## 2. Validity (`s0_validity.txt`, `s5_blockdrift.txt`)

**What holds.**
- **Scale.** The window lasted 228.3 min: 894 timed processes (786 originals and 108 re-runs) plus 20 warm-ups, across 9 blocks and 21 passes. No block was skipped; the cutoff was 21:36.
- **Validity.** 0 of 894 processes are invalid, and my check agrees with the driver's flag on all 894. The check covered:
  - exit, sha256 against the pin, 500 CSV steps;
  - `void_steps` 0, the expected pose matched, workers;
  - the SUMMARY window mean;
  - the `micro8b` rules;
  - DM1's FIFO present mode and zone n = 220;
  - all 60 criterion ids.
- **Voids.** Passes voided: 0. Voided processes: 0. R4 pose compares: 171 of 171 equal. Park-gap void cells: 0 of 8. No block's void rule fired: VOID = none.
- **Placement receipts.** 914 of 914 launched processes carry the main-thread cycle fraction and 867 carry `main_share_top_est`. They were used for nothing.

**What weakens the window.**

1. **Contamination.** 134 of 894 timed processes were unclean. The witness's top "other" process was `claude.exe` for 99 of them, `browser.exe` for 17 and `Telegram.exe` for 12. Of 108 re-runs, 82 were used, and 26 of 786 slots were dropped:

   | block | dropped slots | pass-cells with K = 2 |
   |---|---|---|
   | F3 | 16 | 16 of 81 |
   | ω-v1-cont | 4 | all 4 cells (single pass) |
   | ω-v2 | 2 | 2 |
   | J-Son-T | 2 | 2 |
   | DM1 | 2 | — |

   These K = 2 cells are what make R2, the disarmed J-Son-T product rows and the park price depend on the K reading.
2. **The whole F3 block ran slow (post hoc).** F3-JT-leaflist and S4-AB's S4-JT#tip use the same binary and an identical effective config; the SUMMARYs differ only in the explicit kernel flag. Yet F3 reads slower at every W:

   | W | 1 | 2 | 4 | 8 | 16 |
   |---|---|---|---|---|---|
   | F3 / S4-AB | 1.0417 | 1.0521 | 1.0779 | 1.1066 | 1.0824 |

   - At every W, F3's fastest process is slower than S4-AB's slowest.
   - S7-AB and SPLIT agree with S4-AB within 0.75 % on the same file, so the drift is specific to F3.
   - The receipts of the used processes (medians) show a busier machine: machine-busy 1.70 % in F3 and 1.90 % in J-Son-T, against 0.59 % in S4-AB; witness 0.44 % in F3 against 0.23 % in S4-AB.
   - Inside F3, a process's relative wall does not track its witness (Spearman ρ −0.004, n = 167). No per-process receipt can see this slowdown.
   - Every F3 verdict is a within-block A/B, so no verdict is affected. F3's absolute values (R1's t_q, the per-W walls) are not comparable across blocks. R1 passed anyway.
3. **Idle waits.** 18 of 21 passes met the idle rule at the 130.4–130.5 s floor. The exceptions were S7-AB p1 (250.5 s), ω-v2 p0 (190.5 s) and ω-v2 p2 (490.5 s). In total the idle waits took 54.7 min.
4. **Uniformly slow processes with clean receipts** (windows 7 and 8) recurred on S4-AB's PARENT at W16 (`s4split.md` §4). They broke S4's W16 claims, which have no pre-registered bar. Ruling 1 keeps such processes, and no receipt identifies them. The placement receipt reads the process's creating thread, not the thread that runs the step.
5. **Rows narrower than ruling 1's list.** S4-AB has no in-block ladder at W8/W16 and no rungs at W1/2/4. Its one W8 rung was not seen, so S4's claim rests on the span route.
6. **Deviations in the ω_b v2 bench:**
   - the calibrated block work is 568.6–577.6 ns, not the specified ~0.7 µs;
   - the 36-stage configs always run before the 72-stage ones;
   - on the worker route at P8 b = 4P, the first helper arrives at 7.5 µs at 72 stages against 0.8–0.9 µs at 36. That can bias the slope by up to 183–186 ns a stage; the external route shows 8–11 ns.
7. **ω-v1-cont is underpowered.** It ran last, on a busy desktop, with K = 2.
8. **The DM1 gate is weak on three zones.** It covers 1920×1080 with FIFO present mode only, and the grow frame is not measured. On VB_SHADE, VB_PRODUCE_NET and GBUF_DEFERRED_RESOLVE the bands are 21.9–25.4 % of the zone.

## 3. Pre-registered verdicts, all blocks

The window is [0,500) unless named. Flags are i/s/r, pooled. The verdict is the LETTER reading; the K2 reading is shown where it differs.

### S7-AB

P = trunk 16191fda, T = a3adc827 (S7 partial). n = 9 per cell, split 3/3/3 over the passes.

| claim | pre-registered bar | verdict | strength | key numbers (ms) |
|---|---|---|---|---|
| C1-JT | T(16) not claimed slower than T(8) | **FAILS**: T(16) claimed slower (REFUTED) | STRONG | T(8) 1.8112 [1.8003–1.8536]; T(16) 2.1902 [2.1816–2.2341]; +20.92 % (+0.3790); YYY pooled and in every pass |
| C1-JA | same, on J-A | **FAILS** (REFUTED) | STRONG | 5.3040 → 5.8658; +10.59 % (+0.5617); YYY everywhere |
| C2-JT | T(8) not claimed slower than P(8) | **HOLDS** (no resolved difference) | — | 1.8152 → 1.8112; −0.22 %; nnn everywhere; pooled bars i 2.01 / s 1.16 % |
| C2-JA | same, on J-A | **HOLDS** | — | 5.3081 → 5.3040; −0.08 %; nnn; pooled bars i 0.69 / s 0.57 % |
| C3-JT | P(16) − T(16) against a bar of 0.105 ms (predicted: T faster by 0.166–0.175) | **REFUTED**: T claimed slower | STRONG | P(16) 2.0502 [2.0393–2.0659]; T(16) 2.1902; +6.83 % (+0.1400); YYY everywhere |
| R_8 | ladder8 on P at W8 | **R_8 = 0.060 ms** | — | F0.5 +0.0337 not seen (fails passes 1 and 2); F1 +0.0660, F1.5 +0.0911 and F2 +0.1257 seen (F2 STRONG) |
| R_16 | ladder16 on P at W16 | **R_16 = 0.0525 ms** | F1–F2 STRONG | +0.0545 / +0.1139 / +0.1599 / +0.2146, all seen |
| J-A rung, W16, 0.105 | recommended row | **NOT SEEN** | — | +0.1095; pass 0 nnn |
| W1 pair (optional, Q10) | reported | not claimed | — | 4.4167 → 4.4551; +0.87 %; nnn |
| voids | pose, engagement, receipt-free process | **none** | — | the pre-flight engagement receipt passed (PREP.md, untimed) |

### S4-AB and SPLIT

A = PARENT (S4 off), B = TIP (trunk). n = 9/9, split 3/3/3.

| claim | pre-registered bar | verdict | strength | key numbers (ms) |
|---|---|---|---|---|
| S4-W8 gain, J-T | gain ≥ 0.060 ms, claimed (wall with R_8, or span with the zone canary) | **CLAIMED** (span route) | STRONG | span `phys_solve_build` 0.3038 → 0.1144 (−0.1894, −62.34 %), YYY everywhere; wall also STRONG, 1.9859 [1.9780–2.0244] → 1.8154 [1.7937–1.8472] (−0.1705, −8.58 %) |
| R_8 in-block (S4-rung, 0.060) | rise claimed | **NOT CLAIMED** | — | +0.0475 (+2.62 %); pooled nYn; p0 nnn, p1 YYY, p2 YYn |
| zone canary N30000 (span) | rise claimed | **CLAIMED** | STRONG | +0.0303 for 0.030 injected |
| zone canary N60000 (span) | rise claimed | **CLAIMED** | STRONG | +0.0604 for 0.060 injected |
| no-slower, W1 | TIP not claimed slower | **HOLDS** (no resolved difference) | — | 4.4217 → 4.4500; +0.64 %; nnn; pooled bars i 4.45 / s 1.30 % |
| no-slower, W2 | TIP not claimed slower | **HOLDS** (TIP claimed faster) | — | 3.0807 → 2.9998; −2.62 %; YYn |
| no-slower, W4 | TIP not claimed slower | **HOLDS** (TIP claimed faster) | — | 2.3529 → 2.1981; −6.58 %; YYn (passes YYY) |
| no-slower, W16 | TIP not claimed slower | **HOLDS** (no resolved difference) | — | 2.2203 [2.2104–2.4401] → 2.0502; −7.66 %; p2 nYn |
| poses equal | every pose equals the fixture | **HOLDS** | — | every runner process matched its expected pose |
| SPLIT: B1, B2, pass ramps, N6 | measured, no bar | **MEASURED** | — | see §4.7 |

### ω-v2 and ω-v1-cont

Units are ns. n = 9 per cell, except the gap rows, which have n = 8 (K 2/3/3).

| claim | pre-registered bar | verdict | strength | key numbers |
|---|---|---|---|---|
| void rule | work in [500, 1000]; `exactly_once`; `lost_wakeups` 0; a park row at gap > 0 with no park on every rep | **NO VOID** | — | work 568.6–577.6; every gap-20/80 park cell parked 252 or 504 times a region; 0 of 8 cells void |
| gap-0 park rows | reported | reported | — | 35 of 36 cells took no park; worker P8 b = 4P at 72 stages parked 2–3 times a region |
| S1 input ω_b2(8, 4P) | none (it is an input) | **MEASURED** | — | slope: worker spin 3353 [3247–3608], park 3369 [3192–3692]; external spin 3144 [2922–3647], park 3183 [3003–3694] |
| participation | is the barrier paid by all P? | gap 0: **all P**; park at gap 20: **not all P** | — | gap 0: active 8, all-active reps 998–1000 of 1000. Park at gap 20: active 7–7.5, all-active reps median 294–493 |
| PARK-0 | park costs what spin costs | P8: 6 of 6 **NOT CLAIMED**. P2: 6 of 6 NOT CLAIMED. P4 b = 1P (both routes) and worker b = 2P: **CLAIMED, park cheaper**. Other P4 cells: NOT CLAIMED | — | at P8, park − spin is −175 to +39 |
| PARK-20 (P8, b = 4P) | park − spin, ruling 1 | **NOT CLAIMED** (K2: CLAIMED) | K2: STRONG | worker 23135 → 29157 (+6022, +26.0 %); external 22726 → 28985 (+6258, +27.5 %); only pass 0 has K = 2 |
| PARK-80 | same | **NOT CLAIMED** (K2: CLAIMED) | K2: STRONG | worker +6261 (+7.5 %); external +6174 (+7.4 %) |
| N4: ω(80) − ω(0) | ruling 1 | **CLAIMED** in all four cells | STRONG except W8 worker | W8 worker +2000, W8 external +6200, W16 worker +10200, W16 external +17000 |
| AW: ω(80) − ω(5) | ruling 1 | **CLAIMED** ×4 | STRONG ×4 | +6300 / +6600 / +9600 / +15600 |
| CONT-v1 (optional) | 8b against window 8 | **NOT CLAIMED** (K = 2, underpowered) | — | P8 slope: worker 2994.4 (window 8 published 3000); external 3194.4 (2825) |

### F3 and F3-G4

Tip 16191fda; A = leaflist, B = leaflist-kd. F3-G4 keeps its own K = 3 in one pass.

| claim | pre-registered bar | verdict | strength | key numbers |
|---|---|---|---|---|
| R1 | t_q(kd) at TD W1 ≤ 0.186 ms | **CLAIMED** | STRONG | 167.88 µs [164.17–169.45], n = 9; −9.74 % against the bar; c_q 135.4 ns (1240 rows a step) |
| R2 | t_q(kd)/t_q(leaflist) < 1 at W1 | **NOT CLAIMED** (K2: CLAIMED) | K2: STRONG | 201.98 [198.97–205.39], n = 8 (3/2/3) → 167.88, n = 9; −16.88 % (−34.10 µs); pooled YYY; p1 has K = 2 and sets no flag. The samples do not overlap |
| R3a | t_qb(kd) < leaflist at W1, claimed | **NOT CLAIMED** | — | 225.26 → 218.81 (−2.86 %); pooled nYn (i bar 4.55 %) |
| R3b | t_qb not claimed slower at W8 | holds | — | 230.62 → 229.22 (−0.60 %); nnn |
| R3c | JT/RT walls not claimed slower at W 1/2/4/8/16 | holds (10 of 10 cells) | — | ratios 0.9879–1.0246; all nnn pooled; n 7–9 |
| R3d | scene tree_kd not claimed slower at 10k and 100k | holds (kd claimed faster) | STRONG | 0.8800 / 0.9080 |
| **R3** | a AND b AND c AND d | **NOT CLAIMED** (fails on R3a) | — | — |
| R4 | kd poses equal leaflist poses; a mismatch voids | **HOLDS** (not VOID) | — | 171 of 171 compares equal |
| TH 144/152 on `tree` | the recipe reproduces LO 144 / HI 152 | **NOT REPRODUCED**: the 144 leg is REFUTED in both families | STRONG | all_pairs/tree at 144: uniform 1.1294, disparity 1.1003 (YYY); at 152: 1.1971 / 1.1432 |
| the recipe on this block | — | recorded | — | disparity 128/136. Uniform: HI 128 with LO below the grid (ruling 1), or 128/136 (window 7's rule) |
| tree_kd crossover | recorded | recorded | — | 128/136 in both families |

### J-Son-T and DM1

J-Son-T: tip, cfg a, tree; A = sleeping off, B = sleeping on. DM1: ruling 9's ABBA ×2 gate, one pass.

| claim | pre-registered bar | verdict | strength | key numbers (ms per step) |
|---|---|---|---|---|
| AW [0,100): W1, W1-a, W8, W8-a | ON not claimed slower | **HOLDS** ×4 | — | +0.17 % / +0.11 % / +0.65 % / +0.32 %; the pooled i bar alone is 1.65 % at W1 and 12.93 % at W8 |
| G [100,500), armed (W1-a, W8-a) | ON claimed faster | **CLAIMED** | STRONG | 14.7674 → 6.5397 (−55.71 %); 3.9420 → 1.7813 (−54.81 %); n 9/9 |
| G [100,500), disarmed (W1, W8) | ON claimed faster | **NOT CLAIMED** (K2: CLAIMED) | K2: STRONG | 14.7501 → 6.5164 (−55.82 %); 3.8260 → 1.7127 (−55.24 %); OFF n = 8 (2/3/3) |
| F [274,500), armed (W1-a, W8-a) | ON claimed faster | **CLAIMED** | STRONG | 14.6551 → 0.1277 (−99.13 %); 3.9985 → 0.1334 (−96.66 %) |
| F [274,500), disarmed (W1, W8) | ON claimed faster | **NOT CLAIMED** (K2: CLAIMED) | K2: STRONG | 14.6145 → 0.1254 (−99.14 %); 3.8594 → 0.1337 (−96.54 %) |
| DM1 C6, per gated zone | median(B−A) ≤ 0 or inside the band; VB_EARLY_CULL above its band drops C6 | **B ≤ A in every gated zone** (reading R1) | — | see the list below |

DM1 per-zone results, median(B−A) in µs with the band in brackets:
- idle: VB_EARLY_CULL −6.94 (15.22); VB_RUN −6.14 (17.41); VB_SHADE −23.04 (71.17); VB_PRODUCE_NET −24.06 (72.19); GBUF_DEFERRED_RESOLVE −87.81 (130.05).
- 100-row edit: VB_EARLY_CULL +2.42 (inside 23.01); VB_RUN +2.05 (inside 23.04); VB_SHADE −24.58; VB_PRODUCE_NET −25.60.

## 4. Decisions implied

### 4.1 S7 does NOT merge

Applied literally, the cut's Outcome paragraph (`s7-omega2/cut.md:438-440`) gives this result:
- "C1 and C2 on both rows → merges": not met.
- "C1 fails with C3 claimed": does not apply, because C3 is refuted.
- **"C1 fails without C3 → report, do not merge on C1"**: this branch applies. a3adc827 stays off the trunk.

Beyond the letter, this is a W16 regression, not a missing effect:
- T is slower than P in 9 of 9 paired rounds at W16 on both rows (median T/P 1.0690 on J-T, 1.1245 on J-A). At W8, T/P is 0.9985 on J-T and 1.0011 on J-A.
- The design gate "no row may be claimed slower at W16" (`01-DESIGN.md` §6.9) would also fail.
- `LaneCap::PhysicalCores` is the default, so merging would ship the regression to every host with W greater than its physical core count.
- The trunk (P) is itself claimed STRONG slower at W16 than at W8 on J-T (+12.95 %), but not on J-A (−1.89 %, not claimed). S7 turns J-A's "W16 not slower" into "W16 slower".
- The cause is not measured. The three candidates are listed in `s7.md` §4.

This also bears on S1. PC-S7-7 plans S1's participant count as min(W, cores) − 1, but 8b measured that capping at the physical cores on today's path makes W16 slower. S1 must not inherit that cap without its own W16 A/B.

### 4.2 S4: KEEP

S4 is already on the trunk (16191fda). It holds its window claim: gain ≥ 0.060 ms at W8 on J-T, no W claimed slower, poses equal. The design's O2 fallback is not triggered.

- **Realised at W8:** 0.1705 ms on the wall and 0.1894 ms on the span. The P-c fill goes 0.2615 → 0.0706 ms (−0.1909). Per manifold on [100,500): 434.9 → 397.0 ns.
- **Resolution:** R_8 was not demonstrated in this block. Post hoc, the same TIP file resolves R_8 = 0.060 ms in S7-AB.

### 4.3 F3, the thresholds, and what tree C4 wires

1. **The default does not flip under any reading.** `QueryKernel` stays `LeafList`.
   - By the letter, R1 is claimed, R2 is NOT CLAIMED and R3 is not claimed. So window_cmds' **"not R2" branch** applies: F3 is rejected. The code either stays behind `--bp-kernel leaflist-kd` or is removed, which is the orchestrator's call.
   - Under a K2 ruling the branch is "R2 without R3", and F3 stays opt-in.
   - Either way the build is the problem. It grows from 22.87 to 50.92 µs at W1 and from 22.29 to 54.37 µs at W8, which swallows the query gains of −34.10 µs (W1) and −32.29 µs (W8).
2. **Thresholds: 144/152 do not survive, so the constants must be re-ruled.**
   - The recipe gives disparity 128/136.
   - Uniform gives 128/136 under window 7's rule. Under ruling 1 its LO falls below the grid, so G4 needs a re-read at sizes 96–128.
   - Post hoc (`f3.md` post hoc 5): the move comes from all_pairs being slower in this binary than in window 7's instrument, while tree barely moved. If that is a regression in the Tree's own brute path, lowering the constants would answer the wrong thing, so check it before re-ruling.
3. **Tree C4 wiring:**
   - Auto's high side goes to Tree with the default LeafList (Morton) kernel.
   - LeafListKd is never selected.
   - AUTO_TREE_LO/HI take the re-ruled values, not 144/152.
   - R4 shows F3 moved no pose, so wiring kd later needs no pose re-pin.

### 4.4 L10 C1b (sleeping on by default): supported, and keeps ruling 8's place

- **No awake cost resolves** at W1 or W8, disarmed or armed.
- **The product rows on the tree are faster.** The armed twins are CLAIMED STRONG, and the disarmed rows are CLAIMED STRONG under K2. No row reads the other way.
- **The pile freezes at step 274 in every ON process.** The settled-pile floor on [274,500) is 0.1254 ms at W1 and 0.1337 ms at W8, which removes window 8's AllPairs floor.
- **The awake bound at W8 is weak:** the pooled i bar alone is 12.93 %. The armed spans put the L10 serial pieces at 21.16 µs per step at W8 (0.52 % of the OFF step) and 19.60 µs at W1 (0.12 %).
- **C1b moves every pose pin,** so it stays after the lanes that move no pins.

### 4.5 DM1 C6: KEEP, no revert commit

- VB_EARLY_CULL does not stay above its band in either row: −6.94 µs (−0.88 %) idle, and +2.42 µs with the edit, inside a 23.01 µs band. Window 8's +22.3 µs did not reproduce.
- VB_EARLY_CULL and VB_RUN now carry bands of 1.9–2.9 % of the zone.
- The other three gated zones still cannot see a regression below 21.9–25.4 %.

### 4.6 The S1 inputs, and what they imply

The decision is the orchestrator's (ruling 5). Everything below is arithmetic unless it says "claimed".

**The inputs.**

- **ω_b2 at P = 8.** The pre-registered raw slope at b = 4P is **3144–3369 ns**, and it contains 4 × ~572 ns of block work. Net of that work, per stage:

  | blocks per participant | net ω_b |
  |---|---|
  | 1P | 0.35–0.42 µs |
  | 2P | 0.50–0.68 µs |
  | 4P | 0.86–1.07 µs |

  The colour waves carry 23.36 tasks per scope, which is **2.92 blocks per participant**, so their relevant w is about 0.50–1.07 µs. Today's dispatch cost per colour wave (ramp + join) is 0.907 µs on PARENT and 0.949 µs on TIP. At colour-wave grain, a region barrier costs about what today's scope dispatch costs.
- **Park price.** Nothing is measurable at back-to-back stages: PARK-0 is not claimed in any of the 6 P8 cells. After a 20–80 µs serial stretch, parking costs **+6.0 to +6.3 µs a stage**. That result is NOT CLAIMED by the letter only because of the K guard in pass 0, and STRONG under K2. At gap 20 the median parked region ran on 7 of the 8 participants.
- **Wake latency.** The pool parks before 20 µs of idleness: ω(80) − ω(20) is not claimed in any cell. After that, the first helper arrives 3.4–3.7 µs after the scope opens, against 0.3–0.9 µs when the pool is awake. Over the gap-5 baseline, the wake costs +6.3 / +6.6 µs at W8 (worker / external) and +9.6 / +15.6 µs at W16, all STRONG.
- **Recruitment.** SPLIT's B1 recruitment is 4.1 µs a step, so opening the region before P-a is **not worth building**.

**The bundle against the 5 % build-if.** This uses window 8 §5's formula with 8b's inputs; its constants are the design's (`s4_s1_arith.txt`).

| w plugged in | PARENT, T(8) 1.9859 ms | TIP, T(8) 1.8154 ms |
|---|---|---|
| raw slope (the pre-registered input read literally; not like-for-like, because it double-counts the block work) | 3.3–4.9 % | 3.0–4.8 % |
| net w, window 8's colour-wave term | 19.6–24.8 % | 20.9–26.6 % |
| net w, colour-wave term capped by SPLIT | 12.6–17.7 % | 14.0–19.7 % |

Under SPLIT, S1 removes at most ramp + join − one recruitment. That is 83.0 µs a step on PARENT and 87.1 µs on TIP, and S1 pays 96 × w for it. **The colour-wave term alone is −0.020 to +0.054 ms.** Under any net pricing, S2 and S3 carry the bundle over 5 %; the colour waves do not.

**S1 against the S4-pattern interim** (S2i + S3i on today's scope path; W8 ruling 6):
- If the interim pays ω(8) per wave (window 8's pricing of S2i), S1's increment over S2i + S3i is **+4.2 to +10.1 %** of T(8).
- If the interim pays only the dispatch cost per wave, S1's increment is **−1.4 to +4.5 %**. There is a case for this pricing: the design's 0.745 = 1 − 1/(8E) (`scale_design.md` §6) already charges the in-wave efficiency, and ω(8) charges it again.
- Under the dispatch-only pricing, S1's own increment falls below the 5 % build-if at every b. So **the build decision turns on how S2i/S3i are priced**, which is the S1 design review's job.
- Robust across pricings: S2 + S3 are worth about 0.27–0.30 ms on S1, and about 0.28 ms on today's path at dispatch-only pricing. The inputs are warm apply 0.3081 ms and the integrate group 0.1236 ms, on TIP at W8. The region itself buys little on the colour waves.

### 4.7 SPLIT's pointers to S1, S2, S3, S5 and S7

PARENT, armed, [100,500), W8 unless named (`s6_extras.txt`, `s4_s1_arith.txt`).

**L_wide at W8 is 319.5 µs a step:**

| component | µs a step | lever |
|---|---|---|
| ramp | 60.8 | S1 |
| join | 26.3 | S1 |
| imbalance | 69.0 | none (block grain, which S1 keeps) |
| in-wave remainder | 163.4 (51.1 %) | none (not dispatch) |

- **S1:** its dispatch target is at most 83.0 µs a step.
- **S2 and S3:**
  - The 12 pass-first ramps average 2960 ns, 9.8× the other waves' 302 ns. Together they are 35.5 µs a step, 58 % of the ramp.
  - They are helpers re-woken after the serial warm-apply and integrate gaps; S2 and S3 turn those gaps into parallel stages.
  - On S1, any such gap left inside a region pays the ~6 µs park price unless the helpers spin.
- **The tail:** 72 % imbalance, 28 % join (69.0 / 26.3 µs a step). Every wave is helped.
- **S5:** the bp query is 0.2069 ms at W8 on TIP, unchanged by S4, so S5's target stands. F3's kd would shrink it only with a cheaper build.
- **S7:** from W8 to W16, L_wide grows by +331.0 µs a step, and 81 % of the growth (+269.3 µs) is inside the waves (SMT). That was S7's target; as built, S7 made W16 worse.

## 5. Standing against Jolt 5.6

**Not measured in 8b.** Neither rows file carries a Jolt binary (`s0_validity.txt`), so window 8's in-block method cannot be applied. A ratio against window 8's Jolt cells would be a cross-window comparison, which is not that method, so none is given.

Our trunk (16191fda, J-T, disarmed, S4-AB TIP, n = 9 each). S7-AB's reading of the same file agrees within 0.75 %.

| W | T(W), [0,500), ms | [100,500), ns per manifold |
|---|---|---|
| 1 | 4.4500 | 943.6 |
| 2 | 2.9998 | 643.4 |
| 4 | 2.1981 | 476.3 |
| 8 | 1.8154 | 397.0 |
| 16 | 2.0502 | 450.4 |

On the trunk, W16 is claimed STRONG slower than W8 on J-T (+12.95 %).

## 6. Open questions and follow-ups

### For the owner (values only)

1. **Q1 — spinning helpers in the S1 region (09-26 ruling 6, still open).** Spin-only keeps up to 7 cores busy for the region's length. Spin-then-park costs nothing measurable at back-to-back stages, and about 6 µs a stage after each serial stretch longer than 5–20 µs. Is spinning acceptable?
2. **Q2 — the desktop during quiet windows.** The other applications and agent sessions made 134 processes unclean, cost 26 slots, and coincided with a 4–11 % block-level slowdown in F3. Can future windows run with them closed?
3. **Q3 — DM1 power.** May the GPU clocks be locked, and is a 1440p or 2160p display available? Without that, three gated zones stay blind below about 22–25 %.

### For the orchestrator

- **PC-8b-1 (K guard).** Rule LETTER or K2 window-wide.
  - Affected: F3 R2 plus five F3 context rows, the J-Son-T disarmed G/F rows at W1/W8, and ω PARK-20/80.
  - For window 9, pre-register either K2 or a second re-run per slot.
  - Record that the cut's "pass-0-style" rule cannot fire as written.
- **PC-8b-2 (S7).**
  - Do not merge a3adc827.
  - PC-S7-3's condition (C3 claimed) did not arise. Even so, its core-id sample is the instrument that can separate the candidate causes; run it on T at W16 before redesigning the cap.
  - Strike the physical-core cap from S1's plan (PC-S7-7) until S1 has its own W16 A/B.
- **PC-8b-3 (S4).** KEEP. Future lever A/B blocks should carry an in-block ladder at W8/W16 and rungs at W1/2/4.
- **PC-8b-4 (F3 / G4).**
  - Decide whether the kd code is kept behind the switch or removed.
  - Re-read G4 at sizes 96–128 in steps of 8.
  - Before re-ruling TREE_BRUTE_MAX_ROWS and AUTO_TREE_LO/HI, bisect the all_pairs slowdown (same source; suspects are codegen and the `ContactPairs` writer).
  - Explain why the bench and the runner disagree at J.
- **PC-8b-5 (S1 design review).**
  - Price S2i/S3i both ways (ω(8) and dispatch-only), using SPLIT's split and the net ω_b at b = 2–4P, and decide S1 on its increment over S2i + S3i.
  - Rule on which reading of the pre-registered input (raw slope or net) meets the 5 % build-if.
- **PC-8b-6 (ω bench).** Fix three things: the calibration target (0.57 µs measured, not 0.7), the fixed 36-before-72 config order, and the worker-route recruitment asymmetry at P8, b = 4P.
- **PC-8b-7 (window 8's record).** Window 8's per-wave table was in TSC ticks, not ns (`s4split.md` post hoc 1). Its "162 % of ω" and "W16 growth mostly in the tail, ~0.16 ms" need correction notes where they are cited. Window 8's verdicts are unchanged.
- **PC-8b-8 (hygiene).**
  - Add one fixed sentinel row to every block, e.g. S4-JT#tip@W8. The F3 drift was visible only because F3-JT happened to equal S4-JT.
  - Keep agent sessions idle during timed blocks.
  - Make the placement receipt sample the thread that runs the step.
- **PC-8b-9 (DM1).** Power needs clock control, many more ABBA repeats, or a non-FIFO present-mode knob (`host.rs:226`). The grow frame stays NOT MEASURED.
- **PC-8b-10 (Jolt).** Run Jolt 5.6 in-block in the next window on the S4 trunk (plus C4 if it has landed) to restate the standing.
- **PC-8b-11 (C1b lane).** If a tighter awake bound at W8 is needed, use the armed L10 spans (0.52 % of the step) or a longer awake window. The [0,100) wall cell cannot resolve below about 13 %.
