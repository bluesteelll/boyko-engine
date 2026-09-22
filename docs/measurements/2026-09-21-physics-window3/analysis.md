# Window 3 reduction: the measured default row (L4+L2, and L5) against the P0b bridge and Jolt - recomputed from `win3/raw/`

project-analyst, 2026-09-21. Inputs: `win3/raw/runs.jsonl` (171 records), every per-process CSV under `raw/pass-00/` and `raw/pass-01/`, the two Jolt `-receipt` CSVs under `raw/receipts/`, `raw/window_log.txt`, `raw/wait_log.txt`, `bin/SHA256SUMS`; for the bridge, P0b's own `docs/measurements/2026-09-19-physics-p0/p0b/raw/window/runs.jsonl` (`mean_ms` per process; its CSVs are tarred). Reduction: `(scratch)/analyst_win3/reduce.py` and `sens.py` -> `reduction.json`; nothing imported from the tester's scripts. I did not read numbers off `window_report.md`, `analysis.json` or `tables.md`; where a number below coincides with the tester's, that is a reproduction, not a copy. Statistic and claim rule as ruled: a cell = the MEDIAN over K separate processes of the [0,500) window mean (ms/step); spreads = min-max range, IQR (inclusive quartiles) and the median's SE (1.2533 SD / sqrt K), each as % of the median; B against A is claimed iff |median_B/median_A - 1| > 2 * hypot(spread_A, spread_B), read three ways (range / IQR / SE). Nothing outside `win3/` and the analyst's scratch directory was written; no tree, target or git state was touched (git was used read-only in `D:/wt/mq-de06b6c9` for ancestry, the plan text and to confirm that the three non-lever commits above the P0 tip touch only `docs/`).

## 0. Selection, input checks, and what the window report gets right

**Selection (the protocol's rule).** 171 records = 2 untimed warm-ups + 138 originals + 31 re-runs. Per (pass, seq) slot the original is used when valid and uncontaminated, else its re-run. **137 processes used.** One slot excluded - `D-L4L2@W1` pass 0 seq 2: original after-receipt 6.33 %, its re-run after-receipt 22.5 % (the re-run's window mean, 10.496 ms, would have been that cell's minimum) - so that cell is **K=5**; the other 22 cells are K=6. 0 non-zero exits, 0 invalid attempts, 32 contaminated attempts (30 in pass 0, 2 in pass 1).

**Checks on the inputs (all hold):**
- Every window mean recomputed from the per-step file equals the driver's `mean_ms` (max |delta| 3.6e-15 ms) and, for boyko, the runner's own `window_mean_ns` (max |delta| 3.7e-9 ns); for Jolt it equals 1000 / printed steps-per-second per process (max |delta| 1.1e-7 ms). All 137 windows hold exactly 500 steps.
- One boyko pose, `0x32d5e235342b4143`, in all 77 boyko processes (4 rows, every W); `expect_pose` = `match` in the 47 processes that carried a same-pass reference, `none` in the other 30. Jolt: one hash per exe, `0xee15b89965ec747` x30 (v5.3.0) and `0xb8522b4e3fc62cfe` x30 (v5.6.0).
- void steps 0, disarmed ring traffic 0, drops none; `pool_workers` = W and Jolt threads = W in every process; affinity mask read back `0xffff` x137 (P-none); `target_env msvc`, zone tier `dev`, disarmed x77; 1240 bodies; Jolt banner `no_pair_cache=0, allow_sleep=0, receipt=0` x60.
- Binaries: `bin/SHA256SUMS` re-verified 5/5 OK (`runner_l4l2` 368d4104, `runner_tip` ef9325ef, `runner_l5` 26d17a10, Jolt v5.3.0 29b23ad1, v5.6.0 918fd2b7); every used process carries its row's hash.
- Printed knobs per cell (from the summaries): `D-L4L2` and `D-L5`: `simd_solve true, parallel_solve true, broadphase AllPairs, broadphase_select Manual, sleeping false`; `D-L5` adds `parallel_narrowphase true`, `D-L5-npoff` prints `false`. `J-A` (tip, cfg-A): `simd_solve false`, `parallel_solve false` at W=1 and `true` at W=8, AllPairs/Manual.
- Ancestry (read-only, `D:/wt/mq-de06b6c9`): `de06b6c9` (C4) <- `b8d9ab8f` (C3) <- `aac562a7` (L2 = C2) <- `caac7d06` (L4 = C1) <- `f236ebdd`; `dbd85977` (the P0 tip), S5 `8d656ad8`, KE16 `67563d3b`, B1 `aff98fe7`, L1 `00c07d0f`, `56c1e9e7` are all ancestors. Between the P0 tip and `aac562a7` the only code commits are L4 and L2 (the other three are docs), so `runner_l4l2` differs from `runner_tip` by L4 + L2 alone.
- Receipts: 203 distinct 5-s receipts in `runs.jsonl` (plus the two 10-s pass-opening receipts in `window_log.txt`: 2.57 % and 0.78 %): median 2.03 %, p90 6.59 %, max 22.5 %, **32 over 5 %**, 0 with a build process. Idle rule: pass 0 three quiet polls (cpu10 3.69 / 2.50 / 3.51 %, 0 build and 0 lane-target processes), idle 03:49:57; pass 1 (2.56 / 1.21 / 1.09 %), idle 04:30:25. During-process witness (`others_busy_pct`) over the 137 used processes: median 0.67 %, p95 1.80 %, max 5.1 %.

**Against the window report.** Every cell median, spread, comparison and claim reading in the report's sections 0, 4, 5 and 6 reproduces from the raw files. Two trivia, not corrections: the report's p90 receipt (6.55 %) differs from mine (6.59 %) by the quantile method; "1000/steps_per_s equals the per-frame mean to 4 decimals" is true per process, while at the cell level 1000/median(steps/s) and median(mean) differ in the 4th decimal at W=1 (15.3377 vs 15.3395) because K is even. The report's caveats stand: 13 used pass-0 processes sit > 5 % above their cell median with bracketing receipts under 5 %, which inflates the min-max range of 12 cells and is why several ratios are claimed under IQR and SE but not under min-max.

## 1. The measured default row: `D-L4L2` (aac562a7, `--cfg default` = L4 + L2, simd_solve on)

**T(W), median over K [min-max] ms/step; spreads range / IQR / SE as % of the median; values by process.**

| W | K | D-L4L2 | range / IQR / SE % | values (sorted) |
|---|---|---|---|---|
| 1 | 5 | **10.596** [10.549-10.759] | 1.98 / 0.13 / 0.43 | 10.549, 10.583, 10.596, 10.597, 10.759 |
| 2 | 6 | **9.097** [8.944-10.184] | 13.63 / 1.62 / 2.63 | 8.944, 8.979, 9.064, 9.130, 9.153, 10.184 |
| 4 | 6 | **8.280** [8.196-9.561] | 16.49 / 1.89 / 3.28 | 8.196, 8.208, 8.261, 8.299, 8.403, 9.561 |
| 8 | 6 | **7.783** [7.693-8.042] | 4.48 / 2.05 / 0.88 | 7.693, 7.744, 7.770, 7.795, 7.948, 8.042 |
| 16 | 6 | **8.161** [8.084-10.040] | 23.97 / 0.70 / 4.83 | 8.084, 8.150, 8.158, 8.163, 8.224, 10.040 |

Jolt, same window (K=6 everywhere):

| W | Jolt v5.3.0 | range / IQR / SE % | Jolt v5.6.0 | range / IQR / SE % |
|---|---|---|---|---|
| 1 | 15.339 [15.068-19.042] | 25.91 / 3.86 / 5.14 | 9.828 [9.518-11.364] | 18.78 / 3.31 / 3.50 |
| 2 | 8.571 [8.456-8.744] | 3.36 / 1.14 / 0.61 | 5.770 [5.703-5.818] | 1.99 / 0.87 / 0.38 |
| 4 | 5.168 [5.106-5.250] | 2.80 / 1.98 / 0.63 | 3.581 [3.519-3.663] | 4.02 / 2.36 / 0.83 |
| 8 | 3.583 [3.466-4.693] | 34.25 / 8.65 / 6.73 | 2.569 [2.501-3.124] | 24.26 / 3.09 / 4.76 |
| 16 | 3.164 [3.112-3.205] | 2.95 / 1.17 / 0.54 | 2.388 [2.337-2.465] | 5.32 / 3.42 / 1.12 |

**boyko / Jolt, raw (window means) and per manifold per step over [100,500) with each side's own receipted count.** Counts: boyko 4,519.2575 manifolds/step over [100,500) (4,544.2 over [0,100), 4,524.246 over [0,500), final 4,515; pairs 9,559.215, final 9,559), identical in all 77 boyko processes at every W. Jolt v5.3.0 (`-receipt`, untimed, W=1): 8,456.0025 over [100,500) (7,031.22 / 8,171.046 / final 8,456; 31,034.94 points); **v5.6.0, receipted for the first time**: 8,489.0 (7,042.23 / 8,199.646 / final 8,489; 31,111.96 points); 1,240 active bodies every frame; hashes equal the timed runs'. Jolt/boyko manifolds = 1.871 (v5.3.0) and 1.878 (v5.6.0).

| W | D-L4L2 / v5.3.0 raw (effect; bars r/i/s %; claimed) | D-L4L2 / v5.6.0 raw | per manifold, us: D-L4L2 / v5.3.0 / v5.6.0 | per-manifold ratio /v5.3.0 | /v5.6.0 |
|---|---|---|---|---|---|
| 1 | **0.691** (-30.9 %; 52.0 / 7.7 / 10.3; n/Y/Y) | **1.078** (+7.8 %; 37.8 / 6.6 / 7.1; n/Y/Y) | 2.346 / 1.812 / 1.144 | 1.295 | 2.049 |
| 2 | 1.061 (+6.1 %; 28.1 / 4.0 / 5.4; n/Y/Y) | 1.576 (+57.6 %; 27.5 / 3.7 / 5.3; Y/Y/Y) | 2.018 / 1.028 / 0.675 | 1.963 | 2.990 |
| 4 | 1.602 (+60.2 %; 33.4 / 5.5 / 6.7; Y/Y/Y) | 2.312 (+131.2 %; 33.9 / 6.1 / 6.8; Y/Y/Y) | 1.841 / 0.623 / 0.426 | 2.955 | 4.321 |
| 8 | **2.172** (+117.2 %; 69.1 / 17.8 / 13.6; Y/Y/Y) | **3.029** (+202.9 %; 49.4 / 7.4 / 9.7; Y/Y/Y) | 1.729 / 0.436 / 0.306 | 3.964 | 5.646 |
| 16 | 2.579 (+157.9 %; 48.3 / 2.7 / 9.7; Y/Y/Y) | 3.417 (+241.7 %; 49.1 / 7.0 / 9.9; Y/Y/Y) | 1.819 / 0.384 / 0.288 | 4.741 | 6.311 |

- Sub-windows (H1), D-L4L2 / v5.3.0 over [0,100) / [100,500): 0.706 / 0.692, 1.108 / 1.049, 1.698 / 1.579, 2.414 / 2.119, 2.831 / 2.534 (W = 1, 2, 4, 8, 16). Against v5.6.0: 1.013 / 1.091, 1.538 / 1.592, 2.359 / 2.301, 3.216 / 3.006, 3.715 / 3.360.
- **Scaling.** D-L4L2 T(1)/T(W) = 1.165, 1.280, **1.362**, 1.298 (W = 2, 4, 8, 16); **T(8)/T(16) = 0.954**, i.e. W=16 is +4.85 % (+0.378 ms) over W=8 - claimed under IQR only (bars 48.8 / 4.3 / 9.8 %; the W=16 cell carries one 10.040 ms pass-0 process). Step to step: W2/W1 -14.2 % (n/Y/Y), W4/W2 -9.0 % (n/Y/Y), W8/W4 -6.0 % (n/Y/n). Jolt v5.3.0: T(1)/T(8) = 4.282, T(8)/T(16) = 1.132 (W16 -11.7 %, n/n/n); v5.6.0: 3.825 and 1.076 (-7.0 %, n/n/n). For reference, cfg-A (`J-A`) in this window: T(1)/T(8) = 2.124.
- Jolt v5.6.0 against v5.3.0, this window: 0.641, 0.673, 0.693, 0.717, 0.755 (-35.9 / -32.7 / -30.7 / -28.3 / -24.5 %); claimed under IQR and SE at every W, under range at W = 2, 4, 16.
- Reading: the default row beats Jolt v5.3.0 only at W=1 and is behind at every W >= 2; it never beats v5.6.0. Per manifold it is behind both at every W, by 1.3x (v5.3.0, W=1) to 6.3x (v5.6.0, W=16): the raw W=1 win is the contact set (boyko 1.87x fewer manifolds), and the truth lies between the two readings, as P0b's H8 said. The row barely scales: 1.36x from one worker to eight against Jolt's 4.3x / 3.8x.

## 2. The bridge: `J-A` (tip ef9325ef, cfg-A, disarmed) in this window against P0b's `J-A-d1`

P0b's cells recomputed from its own `runs.jsonl` (258 used of 262; same selection rule; same binary ef9325ef): `J-A-d1@W1` 19.671 [19.079-19.835] (range 3.84 / IQR 0.50 / SE 0.70 %); `J-A-d1@W8` 9.162 [9.103-9.250] (1.61 / 0.17 / 0.27 %). The task's "10.9 % / 4.5 %" are P0b's smallest claimable same-row change under range, 2 * sqrt(2) * 3.84 and 2 * sqrt(2) * 1.61.

| cell | P0b (2026-09-19) | window 3 (2026-09-21) | effect | bars r / i / s % | claimed | medians inside the other window's min-max |
|---|---|---|---|---|---|---|
| J-A @W1 | 19.671 [19.079-19.835] | **19.483** [19.337-19.644] (1.57 / 0.65 / 0.29) | **-0.96 %** (-0.188 ms) | 8.3 / 1.6 / 1.5 | **n / n / n** | win3's median inside P0b's range: yes; P0b's median 0.14 % above win3's max |
| J-A @W8 | 9.162 [9.103-9.250] | **9.174** [9.070-9.243] (1.89 / 0.89 / 0.36) | **+0.13 %** (+0.012 ms) | 5.0 / 1.8 / 0.9 | **n / n / n** | yes / yes |
| Jolt v5.3.0 @W1 / 2 / 4 / 8 / 16 | 15.723 / 8.794 / 5.229 / 3.533 / 3.209 | 15.339 / 8.571 / 5.168 / 3.583 / 3.164 | -2.44 / -2.53 / -1.16 / +1.41 / -1.42 % | - | n/n/n, n/n/**Y**, n/n/n, n/n/n, n/n/n | - |
| Jolt v5.6.0 @W1 / 2 / 4 / 8 / 16 | 9.650 / 5.708 / 3.557 / 2.493 / 2.447 | 9.828 / 5.770 / 3.581 / 2.569 / 2.388 | +1.85 / +1.10 / +0.67 / +3.05 / -2.38 % | - | n/n/n, n/n/**Y**, n/n/n, n/n/n, n/n/n | - |

Pooled over both windows (K=12, same binary and cfg): J-A@W1 19.590 [19.079-19.835] (range 3.86 / IQR 1.05 / SE 0.39 %), J-A@W8 9.162 [9.070-9.250] (1.97 / 0.64 / 0.21 %).

**Verdict: the two windows are comparable.** The same binary under the same cfg reproduces at both W with no claimed shift under any reading, and its window-3 medians lie inside P0b's min-max. The ten Jolt cells reproduce within -2.5..+3.1 %, with no shift claimed under range or IQR; the two SE-only shifts at W=2 (-2.5 % v5.3.0, +1.1 % v5.6.0) are what SE bars of 1-2 % produce over ten comparisons and carry no sign pattern (five negative, five positive). So `D-L4L2` may be set beside P0b's other rows, with the bridge's own uncertainty (shift about 1 % at W=1 and 0.1 % at W=8; SE bars 1.5 % and 0.9 %) added to any cross-window claim. The cross-window comparison is also redundant where it matters: `J-A` was re-taken in this window, so the default row's distance from cfg-A is an in-window number:

- `D-L4L2` against `J-A`, in-window: **-45.6 % at W=1** (10.596 vs 19.483; bars 5.1 / 1.3 / 1.0 %, Y/Y/Y) and **-15.2 % at W=8** (7.783 vs 9.174; 9.7 / 4.5 / 1.9 %, Y/Y/Y). Cross-window against P0b's `J-A-d1`: -46.1 % (8.6 / 1.0 / 1.6, Y/Y/Y) and -15.1 % (9.5 / 4.1 / 1.8, Y/Y/Y) - the same number to within the bridge shift.
- Against P0b's **`J-B`** (armed; cfg-A + Grid + simd_solve; 14.206 [14.050-14.579] at W=1, 11.164 [11.091-11.299] at W=8): -25.4 % (bars 8.4 / 2.1 / 1.6, Y/Y/Y) and -30.3 % (9.7 / 4.5 / 1.9, Y/Y/Y). That is P0b's Grid cost (+3.07 / +3.09 ms) with the sign reversed, plus the remainder below.
- Against P0b's armed `J-A-a` (19.691 / 9.229): -46.2 % and -15.7 %, Y/Y/Y.
- **The derived "cfg-A + simd_solve" estimate is superseded.** P0b derived 11.04-11.14 ms at W=1 and 7.92-8.08 ms at W=8 (= J-B minus its Grid cost: 11.136 / 8.074). Measured `D-L4L2`: **10.596** (-4.0..-4.9 % against the estimate, 0.44-0.54 ms faster) and **7.783** (-1.7..-3.7 %, 0.14-0.30 ms faster); the estimate's W=8 band overlaps the cell's max (8.042) but not its median. Candidate reasons, none separately measured and all in the same direction: J-B was armed (P0b's A/A1 read +0.11 % / +0.74 %, not claimed), L4's inline path at W=1 and its whole-step gate term, and code layout (`runner_l4l2` differs from the tip by the L4 and L2 commits only). The configuration is the same one the estimate described: at W=1 `parallel_solve` on a one-worker pool solves inline (L4) exactly as cfg-A's `parallel_solve = W>1` did, and L2 has no effect under `Manual`. The queue's ratios for the estimate (0.70x v5.3.0 at W=1, 2.24-2.29x at W=8) become measured **0.691x and 2.172x**.

## 3. The L4+L5 T(W) gate: `D-L5` (de06b6c9, C4, default = L4 + L2 + L5) against `D-L4L2` (C2)

Rows exist: `D-L5` at W = 1, 2, 4, 8, 16 and `D-L5-npoff` (`--parallel-np off`, same binary) at W=8, all K=6; pose `0x32d5e235342b4143` in all 36 processes and `expect_pose: match` against the same-pass `D-L4L2` pose in every one of them (plus the untimed 500-step gate at W = 1, 8, 16, `logs/15`, with its 501-step red control exiting 4).

| W | D-L5 (K=6) | range / IQR / SE % | D-L5 vs D-L4L2: effect | delta ms | bars r / i / s % | claimed |
|---|---|---|---|---|---|---|
| 1 | **10.738** [10.684-10.793] | 1.01 / 0.73 / 0.23 | **+1.34 %** | +0.142 | 4.5 / 1.5 / 1.0 | **n / n / Y** |
| 2 | **7.864** [7.767-8.744] | 12.43 / 1.66 / 2.43 | -13.55 % | -1.232 | 36.9 / 4.6 / 7.2 | n / Y / Y |
| 4 | **6.173** [6.106-6.710] | 9.78 / 1.62 / 1.89 | -25.45 % | -2.107 | 38.3 / 5.0 / 7.6 | n / Y / Y |
| 8 | **5.347** [5.307-5.651] | 6.43 / 1.70 / 1.24 | **-31.29 %** | **-2.435** | 15.7 / 5.3 / 3.0 | **Y / Y / Y** |
| 16 | **5.552** [5.480-6.838] | 24.47 / 3.74 / 4.87 | -31.97 % | -2.609 | 68.5 / 7.6 / 13.7 | n / Y / Y |

D-L5 values by process: W1 10.684, 10.696, 10.701, 10.775, 10.776, 10.793; W2 7.767, 7.793, 7.807, 7.922, 7.928, 8.744; W4 6.106, 6.123, 6.171, 6.174, 6.255, 6.710; W8 5.307, 5.334, 5.345, 5.350, 5.454, 5.651; W16 5.480, 5.499, 5.511, 5.592, 5.749, 6.838. `D-L5-npoff@W8`: 7.973 [7.805-9.158] (16.97 / 4.51 / 3.30 %; 7.805, 7.839, 7.888, 8.058, 8.262, 9.158).

**The W=8 L5 delta against its prediction (2.4-2.6 ms in the queue; 2.22-2.37 ms of narrowphase on J-A in the L5 design; the design's pass rule: realized delta-T(8) >= 0.6 x 2.22 = 1.33 ms, claimed).**
- Same binary, flag off against on (`D-L5-npoff` vs `D-L5` at W=8): **+49.10 %, +2.626 ms** (5.347 -> 7.973); bars 36.3 / 9.6 / 7.0 % = 1.94 / 0.52 / 0.38 ms; **claimed under all three readings.** The parallel narrowphase saves 2.626 ms per step at W=8 - at the top of the 2.4-2.6 ms band (0.026 ms above it, one tenth of the SE bar) and above the design's 2.22-2.37 ms.
- Across binaries, C4 against C2 (`D-L5` vs `D-L4L2` at W=8): **-31.29 %, -2.435 ms**; bars 1.22 / 0.42 / 0.24 ms; claimed under all three; inside 2.4-2.6.
- The two differ by 0.19 ms, which is the binary drift at equal knobs: `D-L5-npoff` vs `D-L4L2` at W=8 reads **+2.44 % (+0.190 ms), not claimed under any reading** (bars 35.1 / 9.9 / 6.8 %).
- The prediction was made for cfg-A (simd_solve off); the narrowphase does not depend on `simd_solve`, so the absolute delta transfers, but the design's row for this rule was J-A, which was not run at C2/C4. The rule's 1.33 ms is cleared on the default row by 2x.
- Sensitivity (not the ruled statistic; one > 5 % pass-0 outlier dropped per cell): flag A/B +47.6 % (+2.543 ms, Y/Y/Y at bars 12.8 / 5.6 / 2.9 %); C4 vs C2 at W=8 -31.3 % (Y/Y/Y); at W = 2 / 4 / 16 -13.9 / -25.3 / -32.4 %, each Y/Y/Y (the ruled n/Y/Y at those W is the min-max inflation by one process).

**T(W) shape.** D-L5 T(1)/T(W) = 1.365, 1.740, **2.008**, 1.934; T(8)/T(16) = 0.963 (W=16 +3.82 %, +0.204 ms; bars 50.6 / 8.2 / 10.1 %; n/n/n). Step to step: W2/W1 -26.8 % (Y/Y/Y), W4/W2 -21.5 % (n/Y/Y), W8/W4 -13.4 % (n/Y/Y). Against Jolt v5.3.0: 0.700, 0.918, 1.194, 1.493, 1.755 (n/Y/Y at W = 1, 2, 4, 8; Y/Y/Y at 16); against v5.6.0: 1.093, 1.363, 1.724, 2.081, 2.324 (n/Y/Y at W=1, Y/Y/Y at W >= 2). Per manifold [100,500), us: 2.380, 1.751, 1.368, 1.186, 1.232 (D-L5 / v5.3.0 = 1.31, 1.70, 2.20, 2.72, 3.21; / v5.6.0 = 2.08, 2.59, 3.21, 3.87, 4.28). Sub-windows D-L5 / v5.3.0 [0,100) / [100,500): 0.713 / 0.702, 0.950 / 0.910, 1.283 / 1.173, 1.687 / 1.453, 1.947 / 1.717.

**W=1: +1.34 % (+0.142 ms), claimed under SE only (bar 0.98 %), not under IQR (1.49 %) or range (4.45 %).** What the raw files say about it:
- Per pass: pass 0 +0.23 % (D-L4L2 K=2, 10.671 [10.583-10.759] vs D-L5 K=3, 10.696 [10.684-10.701]; n/n/n); pass 1 **+1.70 %** (K=3 vs 3: 10.596 [10.549-10.597] vs 10.776 [10.775-10.793]; bars 1.0 / 0.5 / 0.4 %, Y/Y/Y). The pass-0 D-L5 processes are all end-of-pass re-runs (04:24-04:27); in pass 1 the two rows alternate at 04:33-04:41.
- Between passes each cell moved by as much as the effect: D-L5@W1 +0.75 % (10.696 -> 10.776), D-L4L2@W1 -0.70 % (10.671 -> 10.596). P0b's own note applies: resolving 1 % under SE needs K >= 12, and the L5 design's G-TW asked for K=12 at W=1; this window has K=5 and 6.
- It is a different-binary difference (C3 + C4 on top of C2; 925,613 differing bytes per the window report), not the flag: at W=1 the C4 code takes the serial loop, byte-identical to flag off (`crates/boyko_physics/src/resources.rs:309-318` at de06b6c9, gated by `one_worker_parallel_narrowphase_runs_the_serial_loop` in `crates/boyko_physics/tests/narrowphase_parallel_equivalence.rs:488`), so the only new per-step work is the `try_parallel` probe (`crates/boyko_physics/src/systems.rs:411-418`). The same-binary `runner_l5 --parallel-np off` at W=1 that would separate flag from binary was not in the row list and was not run.
- Reading under the two documents that state the claim rule: `00-RULINGS.md` ("Rulings after P0", in-tree since e2bcbcb5) makes SE the gate, under which this is a claimed +1.3 % regression at W=1 and G-TW's "a claimed regression at any W blocks" is triggered by the letter; the queue's section 10 RESULT block and this task's protocol keep range / IQR as the spread with SE supplementary, under which it is not claimed. The two documents disagree; that is the orchestrator's call, not mine. Either way the number is below what K=6 resolves, and 6 + 6 more W=1 processes (about 2.5 minutes) would settle it.

**G-TW as written in the L5 design, against what window 3 ran.** Satisfied: one pose hash per row across C2, C4 and every W (77/77 timed processes plus the untimed gate); the L5 rule (>= 1.33 ms claimed at W=8) by 2x on the default row; no claimed regression at any W >= 2 under any reading. Not exercised: the design's row for the rule (J-A, cfg-A, C2 vs C4); the R rows; armed rows, so I(W) before/after and E(W) at W = 2, 4, 16 are still not measured; the J-C canary on C4; K=12 at W=1. The window measured the shipped default instead of cfg-A, which is what the owner's question needs and what the queue's "measured row" asked for; the gate's own rows remain open.

## 4. The owner's question, from measured rows only: at W=1, is boyko's shipped default faster than Jolt?

"Shipped default" is `PhysicsConfig::default()` through `--cfg default`: at the lane tip `de06b6c9` (C4 landed 03:44) that is `D-L5` (L4 + L2 + L5); at `aac562a7` it was `D-L4L2`. Both are measured; both say the same thing.

| W=1 | boyko | Jolt | ratio | effect | bars r / i / s % | claimed |
|---|---|---|---|---|---|---|
| **D-L5 / v5.3.0** | 10.738 [10.684-10.793] | 15.339 [15.068-19.042] | **0.700** | **-30.0 %** (-4.60 ms) | 51.9 / 7.8 / 10.3 | **n / Y / Y** |
| D-L4L2 / v5.3.0 | 10.596 [10.549-10.759] | 15.339 | 0.691 | -30.9 % (-4.74 ms) | 52.0 / 7.7 / 10.3 | n / Y / Y |
| **D-L5 / v5.6.0** | 10.738 | 9.828 [9.518-11.364] | **1.093** | **+9.3 %** (+0.91 ms) | 37.6 / 6.8 / 7.0 | **n / Y / Y** |
| D-L4L2 / v5.6.0 | 10.596 | 9.828 | 1.078 | +7.8 % (+0.77 ms) | 37.8 / 6.6 / 7.1 | n / Y / Y |

- **Against Jolt v5.3.0: yes, faster, by 30 % - claimed under IQR and SE, not under min-max.** The min-max bar is 52 % because the v5.3.0 W=1 cell holds one 19.042 ms process (pass 0, 04:16:47, +24 % over its cell median, during-process witness 1.13 %, bracketing receipts 3.25 / 2.23 %) beside five at 15.068-15.798. Sensitivity, not the ruled statistic: without that one process Jolt reads 15.175 [15.068-15.798] and the ratio is 0.708 (D-L5) / 0.698 (D-L4L2), claimed under all three readings (bars 9.8 / 5.3 / 2.3 %).
- **Against Jolt v5.6.0: no - slower, by 9 % - claimed under IQR and SE, not under min-max** (the v5.6.0 W=1 cell carries one 11.364 ms process, +16 %). Sensitivity without it: 9.783 [9.518-10.073], ratio 1.098 / 1.083, still n/Y/Y (bars 11.5 / 4.4 / 2.4 %).
- **Per manifold per step over [100,500), each side's own count: boyko is slower than both.** D-L5 2.380 us against v5.3.0 1.812 us (**1.31x**) and v5.6.0 1.144 us (**2.08x**); D-L4L2 2.346 us (1.29x / 2.05x). The raw win over v5.3.0 is the contact set - boyko carries 4,519 manifolds per step where Jolt carries 8,456 (v5.3.0) / 8,489 (v5.6.0), 1.87x more - so the honest W=1 statement is "0.70x on the step, 1.31x per manifold, the truth between". The sub-windows agree: D-L5 / v5.3.0 = 0.713 over [0,100) and 0.702 over [100,500); / v5.6.0 = 1.023 and 1.107.
- For completeness at W=8 the shipped default is **1.49x v5.3.0** (n/Y/Y; the v5.3.0 W=8 cell has a 4.693 ms process, +31 %) and **2.08x v5.6.0** (Y/Y/Y); at W=16 1.76x and 2.32x (both Y/Y/Y). The W=1 advantage is the only place the default row is ahead of any Jolt.

## 5. Draft addendum for `docs/MEASUREMENT-QUEUE.md` section 10 (not applied; the orchestrator/owner edits)

**Edit 1 - the heading.** `## 10. Physics - the per-stage profile, and the Jolt parity row re-taken like for like (perf campaign P0) - TIMED 2026-09-19 (window 1 VOID; window 2 under the ruled protocol)` becomes `... TIMED 2026-09-19 (window 1 VOID; window 2 under the ruled protocol); **window 3 TIMED 2026-09-21 (the measured default row, L4+L2 and L5)**`.

**Edit 2 - the one line that is struck.** In the 2026-09-19 block's first bullet, replace `Queue a measured row before quoting this.` with: `**Measured 2026-09-21, window 3 (below): 10.596 ms at W=1 (0.69x v5.3.0) and 7.783 ms at W=8 (2.17x); the estimate was 4-5 % and 2-4 % high. Quote the measured row, not this estimate.**`

**Edit 3 - the two lever lines.** `L4 ... Built (caac7d06, on by default); untimed until the L4/L5 window.` -> `... timed in window 3 (2026-09-21): the default row (L4+L2, simd on) reads 10.596 / 7.783 ms at W=1 / 8.` and `L5 ... untimed until the window (C2 against C4).` -> `... timed in window 3: -2.435 ms at W=8 (C4 against C2) and 2.626 ms for the flag alone, claimed; predicted 2.4-2.6.`

**Edit 4 - the new block**, inserted after the 2026-09-19 block's `Receipts:` list and before `---`:

```markdown
**RESULT, 2026-09-21, window 3 (the measured default row).** One window, complete, under the 2026-09-19
protocol ruling (median over K separate processes of the [0,500) window mean; spread = min-max, IQR, with
the median's SE beside them; claimed iff |effect| > 2*hypot(spread_A, spread_B); 5-s receipts before and
after every process, > 5 % => re-run once at the end of the pass; 10-s receipt and three quiet 60-s polls
before each pass; no band void; P-none). Owner's workstation as on 09-19 (Ryzen 9 5900HS, 8C/16T, High
performance, AC; the owner's agent sessions and a browser open - see contamination). Timed 03:50:08-04:42:16
+03:00, after both running lanes finished (l5np HEAD `de06b6c9` at 03:44, lighttable `8e9cd328` at 03:17;
last lane build process seen 03:37). rustc 1.98.1 `host: x86_64-pc-windows-msvc`, no RUSTFLAGS,
`CARGO_INCREMENTAL=0`, cargo `parity` (inherits `release`: fat LTO, default CGU), zone tier `dev`, disarmed.
Trees, built in detached worktrees `D:/wt/mq-<sha>`: `aac562a7` (= L2 = C2; L4 `caac7d06` and L2 are the only
code commits above the P0 tip `dbd85977`) -> `runner_l4l2` sha256 `368d4104`; `de06b6c9` (L5 C4, on top of C3
`b8d9ab8f`) -> `runner_l5` `26d17a10`; the P0 tip `runner_tip` `ef9325ef` re-used for the bridge. Jolt
v5.3.0 Distribution `29b23ad1`, v5.6.0 `918fd2b7`, unchanged from 09-19, hashes re-verified. Ancestors true
for S5, KE16, B1, L1, `56c1e9e7`, `dbd85977`. `--cfg default` prints: colored, `simd_solve` on,
`parallel_solve` on, broadphase AllPairs under `broadphase_select` **Manual** (C2's Auto band is not
exercised by any row), sleeping off, substeps 4, relax 2; at `de06b6c9` also `parallel_narrowphase` on
(`--parallel-np off` for the same-binary A/B).

**Rows.** `D-L4L2` (default at aac562a7; W 1/2/4/8/16), `D-L5` (default at de06b6c9; W 1/2/4/8/16),
`D-L5-npoff` (W 8), `J-A` (tip, cfg-A, disarmed; W 1/8, the bridge), Jolt v5.3.0 and v5.6.0 timed
(`-s=Pyramid -q=Discrete -f`; W 1/2/4/8/16): 23 cells, 2 passes x 3 rounds, order interleaved by W with Jolt
and boyko alternating, pass 1 reversed, one untimed warm-up per pass; 138 slots, 31 re-runs, **K=6 in 22
cells and K=5 in `D-L4L2@W1`** (its original and its re-run were both contaminated). 0 non-zero exits.
Structural checks: one boyko pose `0x32d5e235342b4143` in all 77 boyko processes (every row, every W;
`--expect-pose` match in the 47 that carried a reference, plus the untimed 500-step gate at W 1/8/16 with
a 501-step red control), Jolt one hash per exe (`0xee15b89965ec747`, `0xb8522b4e3fc62cfe`); void 0, ring
traffic 0, drops 0; workers = W on both sides; mask `0xffff` everywhere.

**Contamination (the difference from 09-19).** 203 5-s receipts: median 2.03 % busy, p90 6.6 %, max
22.5 %, **32 over 5 %** (09-19: 2 of 266), 30 of them in pass 0 (03:50-04:28) - `claude.exe` sessions and
one browser burst, never a build. 13 used pass-0 processes sit 6-31 % above their cell median with
bracketing receipts under 5 % (during-process witness 0.5-5.1 %); the medians absorb one such process per
cell, the min-max ranges of 12 cells do not (6-34 %). Pass 1 (04:30-04:42) was quiet (2 contaminations).
Where a claim below holds under IQR and SE but not min-max, that is the reason.

| W | boyko default L4+L2 (`D-L4L2`) | boyko default +L5 (`D-L5`) | J-A bridge (cfg-A) | Jolt v5.3.0 | Jolt v5.6.0 | D-L4L2 / v5.3.0 | D-L5 / v5.3.0 | D-L5 / v5.6.0 |
|---|---|---|---|---|---|---|---|---|
| 1 | 10.596 [10.549-10.759] (K=5) | 10.738 [10.684-10.793] | 19.483 [19.337-19.644] | 15.339 [15.068-19.042] | 9.828 [9.518-11.364] | 0.691 | **0.700** | 1.093 |
| 2 | 9.097 [8.944-10.184] | 7.864 [7.767-8.744] | - | 8.571 [8.456-8.744] | 5.770 [5.703-5.818] | 1.061 | 0.918 | 1.363 |
| 4 | 8.280 [8.196-9.561] | 6.173 [6.106-6.710] | - | 5.168 [5.106-5.250] | 3.581 [3.519-3.663] | 1.602 | 1.194 | 1.724 |
| 8 | 7.783 [7.693-8.042] | 5.347 [5.307-5.651] | 9.174 [9.070-9.243] | 3.583 [3.466-4.693] | 2.569 [2.501-3.124] | 2.172 | **1.493** | 2.081 |
| 16 | 8.161 [8.084-10.040] | 5.552 [5.480-6.838] | - | 3.164 [3.112-3.205] | 2.388 [2.337-2.465] | 2.579 | 1.755 | 2.324 |

- **The bridge holds.** J-A re-taken: 19.483 (W=1) and 9.174 ms (W=8) against 09-19's 19.671 and 9.162:
  -0.96 % and +0.13 %, not claimed under any reading (bars 8.3 / 1.6 / 1.5 and 5.0 / 1.8 / 0.9 %); the
  Jolt cells reproduce within -2.5..+3.1 %, none claimed under range or IQR. This window's rows may be set
  beside the 09-19 rows.
- **The measured default row replaces the derived "cfg-A + simd_solve" estimate.** D-L4L2 against J-A,
  in-window: -45.6 % at W=1 and -15.2 % at W=8, claimed under all three readings. Against the estimate
  (11.0-11.1 / 7.9-8.1 ms) the row reads 10.596 / 7.783: 4-5 % and 2-4 % faster than derived. Against 09-19's
  armed J-B (Grid + simd): -25.4 % / -30.3 %, claimed - Grid's +3.07 / +3.09 ms with the sign reversed.
- **L4+L2 against Jolt.** 0.691x v5.3.0 at W=1 (-30.9 %; claimed under IQR and SE, not min-max) and 1.078x
  v5.6.0 (+7.8 %; IQR, SE); at W=8 2.172x and 3.029x (all three). Per manifold per step over [100,500), each
  side's own count (boyko 4,519.3; v5.3.0 8,456.0; **v5.6.0 8,489.0, receipted for the first time**):
  1.29, 1.96, 2.95, 3.96, 4.74 against v5.3.0 and 2.05, 2.99, 4.32, 5.65, 6.31 against v5.6.0 at W = 1-16.
  H1 sub-windows [0,100) / [100,500) against v5.3.0: 0.706 / 0.692 at W=1, 2.414 / 2.119 at W=8.
  Scaling T(1)/T(8) = 1.362 (Jolt 4.282 / 3.825); T(16) against T(8) +4.9 %, claimed under IQR only.
- **L5 (C4 against C2), the W3 gate on the default row.** At W=8: **-2.435 ms (-31.3 %)**, claimed under all
  three (bars 15.7 / 5.3 / 3.0 %); same binary, `--parallel-np off` against on: **+2.626 ms (+49.1 %)**,
  claimed under all three (36.3 / 9.6 / 7.0 %) - the prediction was 2.4-2.6 ms, the design's pass rule
  >= 1.33 ms. At W = 2 / 4 / 16: -13.6 / -25.5 / -32.0 %, claimed under IQR and SE. Equal knobs across the
  two binaries (`D-L5-npoff` against `D-L4L2` at W=8): +2.4 %, not claimed. D-L5 T(1)/T(8) = 2.008; T(16)
  against T(8) +3.8 %, not claimed. **At W=1 D-L5 reads +1.34 % over D-L4L2 (+0.142 ms): claimed under SE
  only** (bars 4.5 / 1.5 / 1.0 %); +0.2 % in pass 0 and +1.7 % in pass 1; different binaries, and the C4
  code takes the serial loop on a one-worker pool, so this is not the flag - a same-binary `--parallel-np off`
  W=1 row at K=12 prices it and was not run.
- **The owner's question (W=1, shipped default `de06b6c9`): faster than Jolt v5.3.0 by 30 % (0.700x),
  claimed under IQR and SE, not under min-max (one 19.042 ms Jolt process; without it 0.708x under all
  three); slower than v5.6.0 by 9 % (1.093x), IQR and SE. Per manifold: 1.31x v5.3.0 and 2.08x v5.6.0
  - slower than both; the step-level win is the 1.87x smaller contact set.**

**Not claimed / not measured.** D-L5 vs D-L4L2 at W=1 beyond SE; boyko W16 against W8 (D-L5 +3.8 %,
n/n/n; D-L4L2 IQR only); Jolt W16 against W8 (-11.7 % / -7.0 %, n/n/n); the binary drift at W=8 (+2.4 %);
boyko / v5.3.0 at W = 1, 2, 4, 8 and boyko / v5.6.0 at W=1 under min-max; v5.6.0 / v5.3.0 at W = 1, 8 under
range. Not run: armed rows (no I(W), E(W) or spans for C2/C4), the J-C canary on C4, the R rows, J-A at
C2/C4 (the design's own row for the L5 rule), an Auto-broadphase row (no runner flag), J-P1 (retired at L4).
The claim-rule question (range / IQR / SE) is still OPEN in this block while `00-RULINGS.md`'s post-P0
ruling names SE; every claim above is printed under all three.

Receipts: `docs/measurements/2026-09-21-physics-window3/`: `build_report.md`, `window_report.md`,
`analysis.md` (this reduction), `rows.json`, `rows_l5.json`, `wait_log.txt` (the lane flag poll),
`lanes_done.flag`, `bin/SHA256SUMS`, `logs/`, `raw/runs.jsonl`, `raw/manifest.json`,
`raw/window_state.json`, `raw/window_log.txt`, `raw/wait_log.txt`, `raw/pass-0{0,1}/`, `raw/receipts/`,
`raw/analysis.json`, `raw/tables.md`, `tools/`. `dry/` and `test/` are rehearsals, not measurements. Leave
out `__pycache__/` and the exes.
```

## 6. Open points for the orchestrator, and files

1. **Which spread the claim rule means is stated two ways in the tree.** `docs/physics/perf-campaign/00-RULINGS.md` ("Rulings after P0", landed with the P0 result in `e2bcbcb5`) rules the median's SE as the gate with range and IQR as context; the queue's section 10 RESULT block (same commit) and this task's protocol keep range / IQR as the spread with SE supplementary and call the question OPEN. Under SE, D-L5's +1.34 % at W=1 is a claimed regression and the L5 design's "a claimed regression at any W blocks" fires by the letter; under range / IQR it does not. Every other conclusion in this reduction is the same under both.
2. **The cheapest next measurement is the W=1 flag A/B on `runner_l5`** (`--parallel-np off` against on, K=12 each per the design's own K for W=1, about 2.5 minutes at ~5.3 s per process plus receipts). It separates the flag (predicted 0: the serial loop at one worker) from the C3+C4 binary, which is what the +1.34 % needs. J-A at C2/C4 and the armed rows (I(W), E(W) at W = 2, 4, 16) remain the design's open gate rows; the canary on C4 was not seen in this window.
3. **The min-max reading is the casualty of contamination, not the medians.** 12 cells carry one pass-0 process 6-31 % above the median with clean bracketing receipts; the 5-s receipt cannot see a burst inside a 2-10 s process. If min-max stays a ruled reading, a during-process witness threshold (P0b's p95 of 1.44 %, or this window's 1.80 %) would need to become a gate rather than a witness; that is a protocol change and not mine to make.
4. `D-L4L2@W1` is K=5 (both attempts of one slot contaminated). Its spread is the tightest in the window (range 1.98 %), so nothing above depends on it, but the block should say K=5 where it quotes the cell, as it does.
5. C2's Auto band (`GRID_LO`/`GRID_HI` 2,700 / 3,000) is not exercised by any row: the default ships `broadphase_select: Manual`. The build report flagged this; it stands.

**Files.** This reduction: `win3/analysis.md`. Scripts and intermediate data: `(scratch)/analyst_win3/reduce.py`, `sens.py`, `reduction.json` (every cell, comparison, per-process value, witness, receipt figure, the P0b cells and the bridge). Everything quoted above is in `win3/raw/` (`runs.jsonl`, `pass-00/`, `pass-01/`, `receipts/`, `window_log.txt`, `wait_log.txt`) or, for the bridge, in `docs/measurements/2026-09-19-physics-p0/p0b/raw/window/runs.jsonl`. No file outside `win3/` and my scratch directory was written; no tree, target or git state was touched.
