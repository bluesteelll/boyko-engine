**HEADLINE (window 4, 2026-09-22, C3 binary a46b8287, K=6): `T-D-tree` at W=8 = 3.699 ms against Jolt v5.6.0's 2.569 ms = 1.440x (+44.0 %, claimed under IQR and SE, not under min-max); T(1) = 9.026 ms against 9.828 = 0.918x (-8.2 %, IQR and SE); realized Delta-bp(8) on the armed rows = 1.488 ms against the 1.03 ms bar: PASS (1.44x the bar, 0.83-0.87x the design's 1.72-1.80). C4 verdict: STOP-RULE FIRED - the flip of the default is DEFERRED by the recipe's letter (G4 rule 1 `J snapshot` 0.443 ms > 0.30; G5 gate `tree span` 0.476 / 0.466 ms > 0.36 / 0.35; also G4 rules 6 `compaction` at m = 1240 / 10k / 100k and 9 `high jumper` at 100k). Every directional gate passes (Tree claimed faster than AllPairs on J cfg-A at all five W, on the default row at W = 1 and 8, on R at W = 1 and 8, on all 12 churn arms; S16 no regression; one pose per scene; structure clean; canary seen), so the investigation can change the flip's size, not its sign - unless it finds the excess is work the design forbids. `TREE_BRUTE_MAX_ROWS` = 64 (derived, unchanged); `AUTO_TREE_LO / HI` = 64 / 256 by the recipe's grid rule (126 / 140 by L2's procedure; the grid is too coarse, the recipe's own refinement run is owed); `ADMIT_BUILD_RATIO`: DEFERRED (its denominator is the fired quantity, and the recipe's `c_build` term contains the query cost, so the formula as written reads 1.40 - see section 6); C2 (D6): DEFERRED (t_q = 0.414 ms makes the 5 % trigger fire on both gated rows for any E >= 0.32; at the design's t_q it does not).**

# Window 4 reduction: the Tree broadphase (C3) - G4 bench and build-if, G5 end to end - recomputed from `win4/raw/`

results-analyst, 2026-09-22. Inputs: `win4/raw/runs.jsonl` (159 records), every per-process `run.csv` and `pose.bin` under `raw/pass-00/` and `raw/pass-01/`, `raw/g4/runs.jsonl` (166 records), every `raw/g4/<group>/<arm>/<param>/<baseline>/estimates.json` (117 cells x K) and `raw/g4/logs/*.stderr.txt` (the structural receipts), `raw/window_log.txt`, `raw/wait_log.txt`, `raw/g4/wait_log.txt`, `wait_log.txt`, `bin/SHA256SUMS`, `logs/01..04` (commit, builds, pose gate); for the Jolt v5.6.0 and window-3 `D-L5` cells, `D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/runs.jsonl` (`mean_ms` per process, same selection rule) and `.../raw/receipts/JOLT56-T_receipt_W1/receipt_discrete_th1.csv` (Jolt's manifold count) - reused, never re-run, per the owner's ruling; no v5.3.0 row anywhere. Reduction: `win4/tools/analyze_win4.py` -> `win4/analyst/reduction.json`, `win4/analyst/tables.txt`; nothing imported from the tester's scripts, and I did not read numbers off `window_report.md`, `raw/g4/g4_reduction.json`, `raw/g5_reduction.json` or `logs/09,10,13`. Where a number below coincides with the tester's it is a reproduction, not a copy. Design references are read from `D:/wt/mq-de06b6c9/docs/physics/perf-campaign/levers/broadphase/04-DESIGN-REV2.md` (D3.5, D6, "Expected gain", G4/G5) and the recipe `scratchpad/treebp/g4_g5_recipe.md`; git was used read-only. Nothing outside `win4/` was written; no tree, target or git state was touched.

**Statistic (the recipe 1.1 = the window-3 ruling).** A cell is the MEDIAN over K separate processes (G5: the [0,500) window mean, ms/step; G4: criterion's `median.point_estimate`); spreads = min-max range, IQR (inclusive/linear quartiles) and the median's SE (1.2533 * sample SD / sqrt K), each as % of the median; B against A is claimed iff |median_B / median_A - 1| > 2 * hypot(spread_A, spread_B), printed under every reading as r / i / s. G4 cells carry range and SE (the recipe's "both spreads"; K=3 has no IQR worth printing; the K=6 churn row prints all three). Effects are B/A - 1 with A the AllPairs (or the reference) side.

## 0. Selection, input checks, and the run record

**G5 selection.** 159 records = 2 untimed warm-ups (`T-D-tree@W8`, one per pass) + 156 originals + 1 re-run. Per (pass, seq) slot the original is used when exit 0, valid and uncontaminated, else its re-run: **156 processes used, 0 slots excluded, K = 6 in all 26 cells.** One original was invalid - pass 1 seq 75 `T-D-allpairs@W1` (`raw/pass-01/075_r2_T-D-allpairs_W1/`), pose `0xf6e397d168e0e8b8` against the row's `0x32d5e235342b4143`; its re-run (`..._rerun`, 10:44:00) hashed the row's pose and is the used process. That event is a finding in its own right (section 9, point 1). 0 non-zero exits, 0 contaminated attempts.

**G5 input checks (all hold).** Every window mean recomputed from the 500 `wall_ns` values equals the driver's `mean_ms` and the runner's `window_mean_ns` exactly (max |delta| 0); all 156 files hold steps 0..499. Pose: one hash per (scene, cfg) - `jolt/a` and `jolt/default` both `0x32d5e235342b4143` (the cfg-A rows hash the same 500-step pose as the default rows, so `simd_solve`/`parallel_solve`/`parallel_narrowphase` are bit-transparent on J, as the design's determinism argument requires; the recipe expected a separate cfg-A hash and the pose gate `logs/04` shows the same equality), `rest/default` `0xee2a67a98434919a`, `s16/default` `0x71313833f6a8e645`; the `pose.bin` files are one identical file per scene (sha256 `eff361e1bdf7...` for J = window 3's `D-L5` pose file, `89f084c2816b...` R, `82c192b5aa86...` S16); `expect_pose` = `match` in the 72 processes that carried a same-pass twin reference, `none` in 84. `void_steps` 0 and `first_void` null in all 156; `pool_workers` = W and `dispatcher` 1 everywhere; mask `0xffff` x156 (P-none); `target_env msvc`, `debug_assertions false`, zone tier `dev`; `tree_brute_max_rows 64`, `broadphase_select Manual` in every SUMMARY; the exe sha256 `19456ab4...` in all 156 = `bin/SHA256SUMS` (re-hashed by my script: matches). Counts: J 1240 bodies, final manifolds 4515 / pairs 9559 in all 120 J processes; R 6670 / 9570 in 24; S16 16 / 31 in 12. TreeDiag gate (recipe 2.2 item 2): on every J and R `tree` process `static_rebuilds 1, members 1`, evictions / translations / patches / wide / excluded / sleeper_rebuilds / hint_candidates / locator_resets 0; on every `allpairs` and `s16` process every field 0 - no violation. Knobs: `--cfg a` prints `simd_solve false`, `parallel_solve` = (W > 1), `parallel_narrowphase` = (W > 1); `--cfg default` prints all three `true` (as window 3's `D-L5`).

**G5 receipts.** 314 5-s receipts (before + after, all timed attempts): median 1.00 %, p90 1.84 %, max 3.97 %, **0 over 5 %**, 0 with a build process. During-process witness `others_busy_pct` over the 156 used: median 0.29 %, p95 0.88 %, max 6.49 % - one process over 3 %: pass 1 seq 6 `R-allpairs@W8` (`raw/pass-01/006_r0_R-allpairs_W8`, 7.440 ms, a `python.exe` 16036 at 3.75 CPU-s during its 3.75-s wall; bracketing receipts 0.90 / 0.78 %, so the protocol used it; it is that cell's maximum, section 2.3). `waited_s` 0 in every record. Idle rule (`raw/wait_log.txt`): pass 0 three quiet polls (cpu10 2.06 / 1.20 / 0.95 %, 0 build, 0 lane-target processes), idle 10:16:05; pass 1 (1.83 / 1.65 / 1.38 %), idle 10:31:07; opening 10-s receipts 1.18 % and 0.69 % (`raw/window_log.txt`). Timed 10:16:16-10:44:10 +03:00; `window_state.json`: power scheme High performance, 245 -> 241 processes, `binaries_after: all match`, `status: complete`. Before that, the driver's first launch (08:18) was stopped by the tester at 08:29:54 during its idle wait because a second timed window (`scratchpad/win4b`) was running - nothing timed (`wait_log.txt`, `raw/launch1_aborted_0819/`); the standalone idle receipt of 08:04-08:17 saw 8 build processes on poll 3.

**G4 selection.** 166 records = 156 originals + 10 re-runs; per (block, k, seq, cell) the same rule: **156 used, 0 excluded.** The 10 re-runs replace originals whose *after* receipt exceeded 5 % (`contaminated_after`, all 10; `claude.exe` in every top-5): `bp_g4_uniform` k2 (the whole group re-run as `k2r`), and nine churn cells - k1 of `burst_migrate/on_tree`, `first_archetype_spawn/on_tree`, `archetype_shift/on_allpairs`, `swap_churn/on_tree`, `stable/on_tree`, `stable/on_allpairs`, `first_archetype_spawn/off_tree`, `first_archetype_spawn/off_allpairs`, and k6 of `burst_migrate/on_allpairs` (each `k<k>r`). Every used process's `estimates.json` exists under its baseline directory (117 cells x K read, none missing). Exes: `broadphase-595045019352a67f.exe` `efb84636...` and `row_identity_churn-ec1df0bd330016d8.exe` `d8ba73fc...` = `bin/SHA256SUMS`; exit 0 x166; mask `0xffff`; `build_procs_during` empty and `voided_during` false in all 166. 332 receipts: median 0.61 %, p90 1.79 %, max 9.54 %, 10 over 5 % (the 10 above; 0 before-receipts); witness over the 156 used: median 0.22 %, p95 2.64 %, max 3.12 %; 11 records waited 25 s for a clean before-receipt. Timed 03:48:22-07:54:32 after the standalone idle receipt of 03:40-03:42 (3 polls) and the driver's own 6-poll wait (poll 3 cpu10 6.27 %, `claude.exe`); three more driver waits of 3 polls each (`raw/g4/wait_log.txt`); no build process in any poll or receipt.

**G4 structural receipts.** All 78 receipt keys (the `bp_g4_*/<n>` rows/pairs/members lines, the 21 maintenance `timed step` lines with their `(oracle ok)`, the 3 `high_jumper` mover lines, the 24 churn `tree over the receipt` lines) are byte-identical across every process of every cell, and equal the recipe's dry-run table on every row it lists: `scene/1240` rows 1241 pairs 9570 members 1 rebuilds 1; `scene/j100` pairs 9564; `scene/10000` / `100000` pairs 83559 / 840494; `uniform/100000` 293501, `disparity/100000` 574763 (members 0); `stable` pairs 9464 / 83149 / 867834; `high_jumper` mover with 9 higher-row partners, translations +1, patches +18, evictions 0, rebuilds 0 at every m; `compaction` evictions +1, rebuilds +1, members m/2; `admission_of_64` rebuilds +1, members m + 64; `shift_translation` translations +1, patches 0; every churn `*_tree` arm translations +4 (stable 0), evictions 0, static_rebuilds 1, members 1. The receipts of the two disparity families carry n + 4 rows (the giants), e.g. `disparity/17` rows 21 - the crossover below is read on the nominal n.

**Builds (`logs/02`, `logs/03`, `logs/01_commit.txt`).** Commit `a46b828797b5e4ff9881598fcc8c4a7e82dcd6ae`, C1 `ecbfe416` an ancestor. Parity runner: `Dirty boyko-physics v0.1.0 (D:\wt\mq-de06b6c9\crates\boyko_physics): the file crates\boyko_physics\src\broadphase_tree\mod.rs has changed` then `Compiling boyko-physics v0.1.0 (D:\wt\mq-de06b6c9\crates\boyko_physics)`, `Finished parity profile [optimized]`, `-C opt-level=3 -C lto=fat ... -C target-cpu=x86-64-v3`, executable `D:/wt/_targets/mq-de06b6c9\parity\deps\jolt_parity_pyramid-39550833a2ead4ba.exe` -> `bin/runner_c3.exe`. Benches: `Compiling boyko-physics v0.1.0 (D:\wt\mq-de06b6c9\crates\boyko_physics)`, `Finished bench profile [optimized]`, the two exes above. The lane's tree was built, not the caller's.

**Against the window report.** Every G4 cell and every G5 cell, spread and comparison I checked reproduces from the raw files (the report's 0.4429 / 0.1607 / 1.966 / 24.27 / 5.239 ms stop-rule values, 17.889 / 5.023 / 3.699 / 2.569 ms headline cells, 0.4756 / 0.4662 ms spans, 1.517 / 1.553 / 1.484 / 1.488 ms Delta-bp readings). Two things the report does not say that the raw files do: the compaction arm's timed step re-queries m/2 rows (section 4.1, rule 6), and the invalid W=1 process diverges from step 439 in the solver, not the broadphase (section 9.1).

## 1. The headline table: boyko against Jolt v5.6.0 (the only reference; window-3 cells, K=6)

Cells are median over K [min-max] ms/step; the ratio is boyko / Jolt; bars r / i / s in %; claimed r/i/s. Jolt v5.6.0 recomputed from window 3's `raw/runs.jsonl` with the same selection rule (137 used of 169 timed, 1 excluded slot): W1 9.828 [9.518-11.364] (18.78 / 3.31 / 3.50 %), W2 5.770 [5.703-5.818] (1.99 / 0.87 / 0.38), W4 3.581 [3.519-3.663] (4.02 / 2.36 / 0.83), W8 2.569 [2.501-3.124] (24.26 / 3.09 / 4.76), W16 2.388 [2.337-2.465] (5.32 / 3.42 / 1.12) - identical to window 3's own reduction.

| W | `T-D-tree` (default + Tree) | `T-D-allpairs` (default, = D-L5 re-taken) | `T-A-tree` (cfg-A + Tree) | Jolt v5.6.0 | T-D-tree / Jolt | T-D-allpairs / Jolt | T-A-tree / Jolt |
|---|---|---|---|---|---|---|---|
| 1 | **9.026** [8.918-9.092] (1.93 / 0.97 / 0.38) | 10.554 [10.464-10.738] (2.60 / 1.05 / 0.49) | 17.889 [17.791-18.108] (1.77 / 0.48 / 0.32) | 9.828 [9.518-11.364] | **0.918** (-8.2 %; 37.8 / 6.9 / 7.0; n/Y/Y) | 1.074 (+7.4 %; n/Y/Y) | 1.820 (+82.0 %; Y/Y/Y) |
| 2 | - | - | 10.588 [10.511-10.692] (1.70 / 0.76 / 0.32) | 5.770 | - | - | 1.835 (Y/Y/Y) |
| 4 | - | - | 6.874 [6.826-7.087] (3.79 / 1.37 / 0.73) | 3.581 | - | - | 1.919 (Y/Y/Y) |
| 8 | **3.699** [3.688-3.771] (2.23 / 0.79 / 0.44) | 5.491 [5.327-5.549] (4.05 / 2.15 / 0.84) | 5.023 [5.001-5.081] (1.60 / 0.51 / 0.29) | 2.569 [2.501-3.124] | **1.440** (+44.0 %; 48.7 / 6.4 / 9.6; n/Y/Y) | 2.137 (+113.7 %; Y/Y/Y) | 1.955 (+95.5 %; Y/Y/Y) |
| 16 | - | - | 4.926 [4.869-5.173] (6.17 / 0.91 / 1.15) | 2.388 | - | - | 2.062 (+106.2 %; Y/Y/Y) |

Values by process (sorted): T-D-tree W1 8.918, 8.976, 9.001, 9.052, 9.075, 9.092; W8 3.688, 3.689, 3.698, 3.699, 3.728, 3.771. T-D-allpairs W1 10.464, 10.515, 10.552, 10.555, 10.662, 10.738; W8 5.327, 5.353, 5.486, 5.495, 5.508, 5.549. T-A-tree W1 17.791, 17.825, 17.863, 17.914, 17.922, 18.108; W2 10.511, 10.553, 10.587, 10.589, 10.659, 10.692; W4 6.826, 6.836, 6.860, 6.887, 6.952, 7.087; W8 5.001, 5.011, 5.014, 5.033, 5.039, 5.081; W16 4.869, 4.884, 4.924, 4.927, 4.943, 5.173. (Files: `raw/pass-0{0,1}/<seq>_r<round>_<row>_W<W>/run.csv`.)

- **The two min-max "n" readings are Jolt's, not boyko's.** The v5.6.0 W1 cell carries one 11.364 ms process and the W8 cell one 3.124 ms process (window 3's pass-0 contamination); the boyko cells have ranges of 1.6-2.2 %. Sensitivity, not the ruled statistic: without those two Jolt processes, W8 reads 2.545 [2.501-2.594] (range 3.7 %) and the ratio 1.453 (+45.3 %, bar 8.6 % under range: claimed under all three); W1 reads 9.783 [9.518-10.073] (5.7 %) and the ratio 0.923 (-7.7 %; bar 12.0 %: still not claimed under range).
- **Per manifold per step over [100,500), each side's own count** (boyko 4,519.2575 manifolds/step in every J process; Jolt v5.6.0 8,489.0 from window 3's untimed `-receipt` CSV): W1 boyko 1.997 us against Jolt 1.158 us = **1.72x**; W8 0.818 against 0.303 = **2.70x**. The raw W1 win (0.918x) is the 1.88x smaller contact set, as window 3 said of `D-L5`.
- **Sub-windows** [0,100) / [100,500): T-D-tree 8.966 / 9.034 (W1), 3.697 / 3.699 (W8); the tree rows are flat across the window (the J tail is stable from step 2).
- **Scaling.** T-D-tree T(1)/T(8) = **2.440** (T-D-allpairs 1.922; window 3's D-L5 2.008; Jolt v5.6.0 3.825). T-A-tree T(1)/T(W) = 1.690, 2.602, 3.561, 3.632 at W = 2, 4, 8, 16 (T-A-allpairs 1.595, 2.294, 2.947, 3.017); T(16) against T(8): tree -1.94 % (n/n/n), allpairs -2.35 % (n/n/Y). The Tree is serial, so the step's W-scaling improves because a serial 0.47 ms replaced a serial 1.95 ms (section 2.5): the broadphase is 12.6 % of T-D-tree(8) and 9.3 % of T-A-tree(8), against 29.4 % of T-A-allpairs(8).
- **What the span leaves on the table at W=8.** If the tree span were at the design's 0.15-0.23 ms instead of the measured 0.466, T-D-tree(8) would read 3.38-3.46 ms = 1.32-1.35x Jolt; at a zero-cost broadphase 3.23 ms = 1.26x. The remaining 26 % is downstream of `ContactPairs` and outside this window's claims.

## 2. G5: the tree against AllPairs, row by row (same binary, `--broadphase`; effect = tree / allpairs - 1)

### 2.1 J cfg-A, every W (`T-A-tree` vs `T-A-allpairs`)

| W | allpairs | tree | ratio | effect | delta ms | bars r / i / s % | claimed |
|---|---|---|---|---|---|---|---|
| 1 | 19.571 [19.444-19.697] (1.30 / 0.34 / 0.22) | 17.889 [17.791-18.108] (1.77 / 0.48 / 0.32) | 0.914 | **-8.60 %** | -1.682 | 4.39 / 1.18 / 0.78 | **Y/Y/Y** |
| 2 | 12.270 [12.156-12.292] (1.11 / 0.80 / 0.26) | 10.588 [10.511-10.692] (1.70 / 0.76 / 0.32) | 0.863 | -13.71 % | -1.682 | 4.07 / 2.19 / 0.83 | Y/Y/Y |
| 4 | 8.531 [8.508-8.736] (2.67 / 1.46 / 0.59) | 6.874 [6.826-7.087] (3.79 / 1.37 / 0.73) | 0.806 | -19.42 % | -1.657 | 9.28 / 4.00 / 1.88 | Y/Y/Y |
| 8 | 6.642 [6.582-6.740] (2.39 / 1.26 / 0.47) | 5.023 [5.001-5.081] (1.60 / 0.51 / 0.29) | 0.756 | **-24.37 %** | **-1.619** | 5.75 / 2.72 / 1.11 | **Y/Y/Y** |
| 16 | 6.486 [6.438-6.583] (2.23 / 0.90 / 0.42) | 4.926 [4.869-5.173] (6.17 / 0.91 / 1.15) | 0.759 | -24.06 % | -1.561 | 13.12 / 2.56 / 2.45 | Y/Y/Y |

The step-level delta is 1.56-1.68 ms at every W (the design's table: 1.86-1.94 at W=1, ~1.8 at 2/4/16, 1.72-1.80 at 8); the shortfall against the design is the tree span's excess over its predicted 0.15-0.24 ms (section 2.5).

### 2.2 The default row (`T-D-*`, the row C4 would ship) and the bridge to window 3

| W | T-D-allpairs | T-D-tree | ratio | effect | delta ms | bars r / i / s % | claimed |
|---|---|---|---|---|---|---|---|
| 1 | 10.554 [10.464-10.738] | 9.026 [8.918-9.092] | 0.855 | **-14.47 %** | -1.528 | 6.47 / 2.86 / 1.23 | **Y/Y/Y** |
| 8 | 5.491 [5.327-5.549] | 3.699 [3.688-3.771] | 0.674 | **-32.63 %** | -1.792 | 9.26 / 4.59 / 1.91 | **Y/Y/Y** |

The bridge: `T-D-allpairs` on this binary against window 3's `D-L5` (de06b6c9, whose physics code this lane carries; window 3 recomputed: W1 10.738 [10.684-10.793] (1.01 / 0.73 / 0.23), W8 5.347 [5.307-5.651] (6.43 / 1.70 / 1.24)):

| W | D-L5 (win3) | T-D-allpairs (win4) | effect | delta ms | bars r / i / s % | claimed |
|---|---|---|---|---|---|---|
| 1 | 10.738 | 10.554 | **-1.72 %** | -0.184 | 5.57 / 2.57 / 1.08 | **n / n / Y** |
| 8 | 5.347 | 5.491 | +2.68 % | +0.143 | 15.20 / 5.49 / 3.00 | n / n / n |

No claimed shift under range or IQR at either W; under SE a -1.7 % shift at W=1 (bar 1.08 %) - the same magnitude and the same SE-only reading window 3 found on its own W=1 cells (its section 3), and the two binaries differ by the whole C3 commit (code layout), so it cannot be attributed. Per pass, T-D-allpairs W1: pass 0 10.515, 10.662, 10.738; pass 1 10.464, 10.552, 10.555. The T-D-tree against D-L5: -15.94 % (W1) and -30.83 % (W8), Y/Y/Y - the same numbers as the in-window comparison, to the bridge's shift.

### 2.3 R (`--scene rest`, default cfg)

| W | R-allpairs | R-tree | ratio | effect | delta ms | bars r / i / s % | claimed |
|---|---|---|---|---|---|---|---|
| 1 | 13.906 [13.671-14.053] (2.75 / 0.32 / 0.46) | 12.308 [12.222-12.704] (3.92 / 1.17 / 0.75) | 0.885 | **-11.49 %** | -1.598 | 9.57 / 2.43 / 1.75 | **Y/Y/Y** |
| 8 | 6.847 [6.694-7.440] (10.90 / 2.65 / 2.00) | 5.310 [5.086-5.481] (7.46 / 5.38 / 1.65) | 0.775 | **-22.46 %** | -1.538 | 26.41 / 12.00 / 5.19 | **n / Y / Y** |

The W=8 range reading is the one 7.440 ms `R-allpairs` process (pass 1 seq 6, the 6.49 % witness; section 0) plus R-tree's own 7.5 % range (5.086, 5.129, 5.221, 5.398, 5.451, 5.481 - two pass-0 and one pass-1 process 5-7 % above the rest, clean receipts). Sensitivity without the 7.440 process: R-allpairs W8 6.814 [6.694-6.998] (4.47 %), ratio 0.779 (-22.1 %), bar 17.4 % under range: claimed under all three. R's manifolds 6,673.4 per step over [100,500); one pose `0xee2a67a98434919a` x24.

### 2.4 S16 (17 rows, the brute path in both arms)

`S16-allpairs` 0.04500 [0.04447-0.04586] ms (3.01 / 1.59 / 0.60), `S16-tree` 0.04483 [0.04450-0.04581] (3.08 / 1.83 / 0.65): tree / allpairs 0.996, **-0.38 %**, bars 8.61 / 4.84 / 1.77: n/n/n - no claimed regression (and no claimed gain). TreeDiag all zeros in both arms (the state is untouched below `brute_max_rows`).

### 2.5 The armed rows: the tree span, Delta-bp, t_q, structure (`--arm-profiler`, cfg-A)

Per process the median over steps [100,500) of each per-step column, then the cell median over K (K=6; `raw/pass-0{0,1}/<seq>_r<r>_T-A-{tree,allpairs}-armed_W{1,8}/run.csv`). The mean over [0,500) is given beside it where the gate could be read either way.

| span (ms) | T-A-tree-armed W1 | T-A-tree-armed W8 | T-A-allpairs-armed W1 | T-A-allpairs-armed W8 |
|---|---|---|---|---|
| `phys_bp_verify` | 0.0078 [0.0075-0.0082] | 0.0066 [0.0065-0.0067] | 0 | 0 |
| `phys_bp_build` | 0.0267 [0.0255-0.0273] | 0.0228 [0.0217-0.0229] | 0 | 0 |
| `phys_bp_query` | **0.4140** [0.4135-0.4175] (range 0.97 %, SE 0.20 %) | **0.4110** [0.4099-0.4127] | 0 | 0 |
| `phys_bp_assemble` | 0.0264 [0.0254-0.0268] | 0.0261 [0.0259-0.0266] | 0 | 0 |
| **sigma of the four** | **0.4756** [0.4727-0.4795] (1.41 / 0.28 %); mean[0,500) 0.5162 | **0.4662** [0.4643-0.4687] (0.95 / 0.16 %); mean 0.4697 | 0 | 0 |
| `sys_physics_broadphase` | 0.4762 [0.4734-0.4801]; mean 0.5167 | 0.4665 [0.4645-0.4689]; mean 0.4700 | **2.0291** [1.9550-2.0659] (5.46 / 1.08 %); mean 2.0341 | **1.9543** [1.9475-1.9588] (0.58 / 0.10 %); mean 1.9541 |

Values of sigma by process: W1 0.4727, 0.4734, 0.4755, 0.4756, 0.4781, 0.4795; W8 0.4643, 0.4659, 0.4661, 0.4664, 0.4672, 0.4687. The AllPairs span reproduces P0b's armed "bp before" (design table: 2.100 at W1, 1.945 at W8) within -3.4 % / +0.5 %.

- **Tree span gate (2.3): FAIL at both W.** 0.4756 > 0.36 ms (1.32x) at W=1; 0.4662 > 0.35 ms (1.33x) at W=8. The query is 87-88 % of it: 0.414 ms = 334 ns per queried row = 43 ns per emitted pair (9,559), against the design's c_q 100-150 ns/row (t_q 0.124-0.186 ms). Verify, build and assemble are at or under their arithmetic (2.5-5 / 15-25 / 16-27 us predicted; 7.8 / 26.7 / 26.4 us measured at W=1 - verify is 1.6-3x its band, 5 us in absolute terms).
- **Delta-bp gate (2.3): PASS.** `physics_broadphase` allpairs - tree: **W8 1.4878 ms** (median reading; 1.4841 by the [0,500) mean) against the bar **1.03 ms** - 1.44x the bar and 0.83-0.87x the design's 1.72-1.80; W1 1.5529 (median) / 1.5174 (mean). The span ratio tree / allpairs is 0.239 at W8 (-76.1 %, bars 2.23 / 0.42 / 0.37, Y/Y/Y) and 0.235 at W1 (Y/Y/Y). The armed step deltas (allpairs-armed - tree-armed) are 1.685 (W1) / 1.676 (W8) ms, 0.13-0.19 ms more than the span deltas.
- **Arming cost:** T-A-tree-armed vs T-A-tree -0.10 % at both W (n/n/n); T-A-allpairs-armed vs T-A-allpairs -0.08 % (W1) / +0.79 % (W8), n/n/n.
- **Structure gate (2.3): PASS.** In all 12 tree-armed processes `span_n_sets` = {(1,1,1,1)}, `phys_bp_queried + phys_bp_members` = 1241 on every step (queried 1241 / members 0 at steps 0-1, 1240 / 1 from step 2), `phys_bp_rebuilds` total 1 per process, `void` 0 on every step, `first_void` null, `phys_bp_pairs` 9559 at step 499; the allpairs-armed processes read (0,0,0,0) and 0 members (the brute path).
- **t_q (recipe 1.3): 0.4140 ms at J, W=1** (0.4110 at W=8); the 10k upper bound from the bench: `bp_g4_scene/tree/10000` 4.807 ms (J-like density), `bp_g4_uniform/tree/10000` 3.022 ms.

### 2.6 The canary (`T-C-tree`, `--canary-frac 0.05 --canary-ref-ns <T of the same pass's T-A-tree>`)

`canary_ns` per process 889,545-905,374 (W1) and 250,030-254,059 (W8) = 5 % of the reference T. The canary's own span `sys_parity_canary_ns` reads it within -1.4..+0.6 % (W1 median 0.8925 ms [0.8898-0.9057]; W8 0.2516 [0.2503-0.2543]). The step rises against `T-A-tree` by **+4.89 % (+0.875 ms, 0.97-0.98 of the canary; bars 4.37 / 1.31 / 0.79; Y/Y/Y) at W=1** and **+6.30 % (+0.316 ms, 1.25-1.27 of the canary; bars 28.10 / 3.62 / 5.60; n/Y/Y) at W=8** (the T-C-tree W8 cell holds one 6.039 ms process, pass 1 seq 55; the other five read 5.294-5.402). P0 read its canary the same way (+5.06 / +4.81 % against 5.00 / 4.98 injected, Y/Y/Y): **seen at both W**, under min-max only at W=1. The four tree spans inside the canary processes read as in the armed rows (query 0.4141 / 0.4097 ms).

## 3. G4: the criterion benches (`bench` profile)

### 3.1 Pair finding (K=3; median [min-max] ms; range / SE %; `raw/g4/<group>/<arm>/<n>/k{1,2,3}[r]/estimates.json`)

| family, n | all_pairs | grid_w1 | grid_w8 | tree | tree / all_pairs (claimed r/s) | tree / grid_w1 |
|---|---|---|---|---|---|---|
| uniform 17 | 0.000182 [0.000182-0.000183] | 0.00310 | 0.00310 | 0.00177 [0.00176-0.00181] | 9.73 (+873 %, Y/Y) | 0.570 |
| uniform 64 | 0.00277 [0.00276-0.00277] | 0.0477 | 0.0476 | 0.00447 [0.00446-0.00447] | 1.616 (+61.6 %, Y/Y) | 0.094 |
| uniform 128 | 0.01174 [0.01172-0.01174] | 0.0491 | 0.0490 | 0.01032 [0.01030-0.01034] | **0.879 (-12.1 %, Y/Y)** | 0.210 |
| uniform 256 | 0.04393 | 0.1178 | 0.1177 | 0.02200 [0.02187-0.02206] | 0.501 (Y/Y) | 0.187 |
| uniform 1000 | 0.6343 | 0.6651 | 0.6661 | 0.1531 [0.1510-0.1557] | 0.241 (Y/Y) | 0.230 |
| uniform 10000 | 75.67 | 8.069 | 3.440 | 3.022 [3.020-3.028] | 0.040 (Y/Y) | 0.374 (grid_w8 0.878) |
| uniform 100000 | 12,018 [11,754-16,879] | 89.69 | 36.41 | 39.85 [39.72-40.06] | 0.003 (Y/Y) | 0.444 (grid_w8 **1.094**) |
| disparity 17 | 0.000290 | 0.00729 | 0.00731 | 0.00232 | 8.01 (Y/Y) | 0.319 |
| disparity 64 | 0.00350 [0.00346-0.00350] | 0.0526 | 0.0526 | 0.00700 [0.00699-0.00702] | 1.999 (+99.9 %, Y/Y) | 0.133 |
| disparity 128 | 0.01389 [0.01376-0.01392] | 0.1756 | 0.1762 | 0.01468 [0.01461-0.01479] | **1.057 (+5.7 %, bars 3.32 / 1.25, Y/Y)** | 0.084 |
| disparity 256 | 0.04844 | 0.3513 | 0.3518 | 0.03140 [0.03127-0.03156] | **0.648 (-35.2 %, Y/Y)** | 0.089 |
| disparity 1000 | 0.6547 | 2.439 | 2.445 | 0.2306 | 0.352 (Y/Y) | 0.095 |
| disparity 10000 | 75.94 | 15.93 | 8.649 | 3.557 | 0.047 (Y/Y) | 0.223 (grid_w8 0.411) |
| disparity 100000 | 14,853 [12,240-15,282] | 139.3 | 83.61 | 45.73 [45.58-46.81] | 0.003 (Y/Y) | 0.328 (grid_w8 0.547) |
| scene 1240 | 1.8485 [1.8480-1.8531] | 5.411 | 5.411 | **0.4415** [0.4414-0.4419] (0.11 / 0.04) | 0.239 (-76.1 %, Y/Y) | 0.082 |
| scene j100 | 1.8538 | 4.981 | 4.982 | **0.4430** [0.4424-0.4440] (0.36 / 0.13) | 0.239 (Y/Y) | 0.089 |
| scene 10000 | 123.39 | 36.79 | 14.52 | 4.807 [4.803-4.814] | 0.039 (Y/Y) | 0.131 (grid_w8 0.331) |
| scene 100000 | 17,633 [17,550-22,192] | 336.3 | 128.1 | 59.34 [59.18-59.61] | 0.003 (Y/Y) | 0.176 (grid_w8 0.463) |

Per row of the tree's whole step: uniform 153 / 302 / 398 ns at 1k / 10k / 100k; scene 356 / 481 / 593 ns at 1240 / 10k / 100k - the dense scene (7.7 pairs per row) costs 2.3x the uniform lattice (2.4-2.9 pairs per row) per row, and 46-71 ns per emitted pair on the scene family. The design's c_q (100-150 / 150-200 / 250 ns per row) is met by the uniform family within 1.0-1.6x and exceeded by the scene family 2.4-3.6x. `grid_w8` is below `grid_w1` only from 10k (its `MIN_PARALLEL_BODIES`), and beats the serial tree at uniform 100k by 9 % (tree / grid_w8 1.094) - the W=1 rule does not read it.

### 3.2 Maintenance (K=3; ms; `raw/g4/bp_g4_maintenance/<arm>/<m>/k*/estimates.json`)

| m | stable | admission_from_empty | admission_of_64 | eviction_filter | shift_translation | compaction | high_jumper |
|---|---|---|---|---|---|---|---|
| 1240 | 0.03269 [0.03233-0.03345] | 0.4729 [0.4675-0.4832] | 0.06917 [0.06903-0.07099] | 0.04618 [0.04610-0.04649] | 0.05542 [0.05513-0.05643] | 0.2069 [0.2061-0.2070] | 0.05991 [0.05961-0.06001] |
| 10000 | 0.2784 [0.2741-0.2804] | 4.838 [4.826-4.968] | 0.5302 [0.5291-0.5330] | 0.3893 [0.3885-0.3899] | 0.4681 [0.4678-0.4704] | 2.355 [2.354-2.356] | 0.5053 [0.5053-0.5069] |
| 100000 | 4.368 [4.279-4.457] | 62.54 [61.43-63.31] | 9.725 [9.418-9.801] | 6.080 [5.977-6.120] | 9.196 [9.157-9.856] | 30.35 [30.25-30.42] | 9.607 [9.549-9.620] |

### 3.3 Churn (`row_identity_churn`, K=6, ms per churn step; `raw/g4/row_identity_churn/<arm>/<label>/k{1..6}[r]/estimates.json`)

| arm | sleeping | allpairs | tree | tree / allpairs (effect; bars r/i/s; claimed) | tree - tree(stable) | allpairs - allpairs(stable) | difference of the two |
|---|---|---|---|---|---|---|---|
| stable | off | 11.590 [11.506-11.682] | 9.606 [9.553-9.739] | 0.829 (-17.1 %; 4.92 / 1.38 / 0.90; Y/Y/Y) | 0 | 0 | 0 |
| swap_churn | off | 11.883 | 9.769 [9.733-9.940] | 0.822 (-17.8 %; 4.97 / 2.24 / 0.99; Y/Y/Y) | +0.163 (vs stable +1.70 %, n/n/Y) | +0.294 | -0.130 |
| archetype_shift | off | 11.582 | 9.694 [9.656-9.951] | 0.837 (-16.3 %; 6.84 / 3.86 / 1.51; Y/Y/Y) | +0.088 (+0.92 %, n/n/n) | -0.008 | +0.096 |
| first_archetype_spawn | off | 11.859 | 9.874 [9.754-9.987] | 0.833 (-16.7 %; 5.88 / 2.93 / 1.15; Y/Y/Y) | +0.268 (+2.79 %, n/Y/Y) | +0.270 | -0.002 |
| burst_despawn | off | 12.001 [11.758-12.396] | 9.653 [9.612-10.216] | 0.804 (-19.6 %; 16.43 / 3.08 / 3.04; Y/Y/Y) | **+0.047** (+0.49 %, n/n/n) | +0.412 | -0.365 |
| burst_migrate | off | 11.945 [11.833-12.645] | 9.755 [9.714-10.548] | 0.817 (-18.3 %; 21.84 / 3.82 / 4.27; **n**/Y/Y) | +0.149 (+1.55 %, n/n/n) | +0.356 | -0.207 |
| stable | on | 5.739 [5.724-5.791] | 4.043 [4.040-4.058] | 0.704 (-29.6 %; 2.53 / 0.71 / 0.46; Y/Y/Y) | 0 | 0 | 0 |
| swap_churn | on | 5.803 [5.782-6.270] | 4.120 [4.104-4.167] | 0.710 (-29.0 %; 17.11 / 1.69 / 3.45; Y/Y/Y) | +0.077 (+1.90 %, n/Y/Y) | +0.063 | +0.014 |
| archetype_shift | on | 5.994 | 4.183 [4.141-4.589] | 0.698 (-30.2 %; 22.66 / 3.72 / 4.41; Y/Y/Y) | +0.140 (+3.46 %, n/Y/n) | +0.255 | -0.115 |
| first_archetype_spawn | on | 5.830 [5.812-6.348] | 4.150 [4.114-4.176] | 0.712 (-28.8 %; 18.65 / 2.27 / 3.76; Y/Y/Y) | +0.107 (+2.65 %, n/Y/Y) | +0.090 | +0.017 |
| burst_despawn | on | 5.814 [5.791-6.330] | 4.130 [4.113-4.452] | 0.710 (-29.0 %; 24.77 / 1.73 / 4.98; Y/Y/Y) | +0.087 (+2.14 %, n/Y/n) | +0.074 | +0.012 |
| burst_migrate | on | 5.832 [5.820-6.464] | 4.190 [4.164-4.203] | 0.718 (-28.2 %; 22.19 / 0.64 / 4.56; Y/Y/Y) | +0.147 (+3.63 %, **Y/Y/Y**) | +0.092 | +0.054 |

The wide ranges on the sleeping-on allpairs cells (8-11 %) and on `burst_*` are one process per cell 8-11 % above the rest (k1 in five of them, before the tester minimized the desktop app at 05:58; clean bracketing receipts), which is why several claims hold under IQR and SE but not min-max.

## 4. The gates, with the arithmetic

### 4.1 Recipe 1.2 - the G4 stop rules (medians, ms; "- stable" = the same m's `stable` cell subtracted)

| # | rule | cell | value | limit | value / limit | verdict |
|---|---|---|---|---|---|---|
| 1 | J snapshot | `bp_g4_scene/tree/j100` | **0.4429** (0.4424 / 0.4429 / 0.4440) | 0.30 | **1.48** | **FIRED** |
| 2 | Tree vs Grid (W=1, n > 64) | tree / grid_w1: uniform 0.210, 0.187, 0.230, 0.374, 0.444; disparity 0.084, 0.089, 0.095, 0.223, 0.328; scene 0.082, 0.131, 0.176 | - | tree slower, claimed | tree faster everywhere, Y/Y | held |
| 3 | stable | `stable/1240` / 10k / 100k | 0.0327 / 0.2784 / 4.368 | 0.058 / 0.46 / 4.6 | 0.56 / 0.61 / **0.95** | held |
| 4 | translation | `shift_translation - stable` | 0.0227 / 0.1897 / 4.828 | 0.062 / 0.50 / 5.0 | 0.37 / 0.38 / **0.97** | held |
| 5 | eviction | `eviction_filter - stable` | 0.0135 / 0.1108 / 1.712 | 0.048 / 0.38 / 3.8 | 0.28 / 0.29 / 0.45 | held |
| 6 | compaction | `compaction - eviction_filter` | **0.1607 / 1.9658 / 24.270** | 0.050 / 0.40 / 6.0 | **3.21 / 4.91 / 4.05** | **FIRED x3** |
| 7 | admission of 64 | `admission_of_64 - stable` | 0.0365 / 0.2518 / 5.357 | 0.120 / 0.80 / 9.8 | 0.30 / 0.31 / 0.55 | held |
| 8 | from empty | `admission_from_empty - stable` | 0.4402 / 4.560 / 58.17 | 0.46 / 4.8 / 60 | **0.96 / 0.95 / 0.97** | held (at the limit) |
| 9 | high jumper (W1) | `high_jumper - stable` | 0.0272 / 0.2269 / **5.239** | 0.062 / 0.50 / 5.0 | 0.44 / 0.45 / **1.05** | **FIRED at 100k** |

- **Rule 1.** 1.48x the limit; the design's arithmetic for the J step is 0.16-0.24 ms. The same quantity measured end to end in the runner is 0.476 / 0.466 ms (section 2.5), of which 0.414 is the query; the bench's `scene/1240` (0.4415) and `j100` (0.4430) agree with the runner's sigma to 7 %.
- **Rule 6, what the timed step contains.** The compaction arm (`crates/boyko_physics/benches/broadphase.rs`, `MaintArm::Compaction`) teleports the members below half, takes an untimed step (h - 1 = 619 / 4999 / 49999 evictions, "dead below live: no compaction yet"), teleports one more and times *that* step: one eviction, the filter, the compaction - **and the queries of the h = 620 / 5000 / 50000 evicted rows, which are Q rows on that step** (D3.2: an evicted row "becomes Q"; a still static row is pending, i.e. "Q this step"). The subtraction of `eviction_filter` (whose timed step carries about a dozen pending rows) does not remove them. Per live leaf the value reads 259 / 393 / 485 ns - the per-row query cost (334 ns at J), not the design's radix-only c_build (12-20 ns/row). Removing (h - 1) queries at the measured per-row cost (334 ns from t_q at J; 302 and 398 ns from the uniform 10k / 100k tree steps as bounds) leaves **-0.046 / +0.455 / +4.35 ms** against the design's compaction band 0.015-0.025 / 0.12-0.20 / 2-3 ms: inside it at 1240, 2-4x above at 10k and 1.4-2.2x at 100k, with the query estimate's own uncertainty (+-10 % of 1.5 / 20 ms = +-0.15 / +-2 ms) covering most of the 100k excess. So the rule fired on the recipe's formula; whether the compaction itself is over its band is not separable in this bench. What the investigation must measure is in section 8.
- **Rule 8** sits at 95-97 % of its limit at every m: `admission_from_empty - stable` = m x (c_build + c_q) + merge by D3.5's own definition (the admission queries every pending row), so it carries the same per-row query cost as rule 1 (355 / 456 / 582 ns per row against the design's 112-170 / 162-220 / 270-280).
- **Rule 9** fires at 100k only, by 4.8 % (the difference's min-max 5.09-5.34, all above 5.0). The jumper is the translation plus an 18-entry patch; the translation itself (rule 4) is at 97 % of its limit at 100k (the list pass over 867,834 entries: 4.83 ms = 5.6 ns per entry against c_list 1.5-2.5 plus N x 4-6 ns), and at 37-38 % at 1240 / 10k. A memory-bound list pass at 14 MB of entries, marginal, not a J-scale finding.
- **Rules 2, 3, 4, 5, 7 hold**, rules 3 and 4 at the limit at 100k, rule 7 at 0.30-0.55.

### 4.2 Recipe 1.1 - the churn claim rules (section 3.3)

- **"On every arm `*_tree` faster than `*_allpairs`, claimed":** 12 of 12 arms under IQR and SE (effects -16.3..-19.6 % sleeping off, -28.2..-30.2 % on); 11 of 12 under min-max, the exception `burst_migrate/sleeping_off` (-18.3 %, range bar 21.8 %: one 12.645 allpairs and one 10.548 tree process). **PASS under SE and IQR; under the recipe's "both spreads" reading one arm short.**
- **"`tree(arm) - tree(stable)` <= 0.05 ms, or not claimed above that":** by value, 1 of 10 non-stable arms (`burst_despawn/off`, +0.047 ms); the other nine read +0.077..+0.268 ms. Read as "not claimed above": under min-max none of the ten is claimed; under SE five are (swap_churn/off +0.163, first_archetype_spawn/off +0.268, swap_churn/on +0.077, first_archetype_spawn/on +0.107, burst_migrate/on +0.147); under all three readings one (`burst_migrate/on`, +0.147 ms, +3.63 %). **Under range: PASS; under SE: 5 arms above 0.05 ms and claimed.** Two facts bound what this rule can see: (a) the AllPairs arm pays the same churn - `allpairs(arm) - allpairs(stable)` is +0.06..+0.41 ms on the same arms - and the difference of the two (the last column, the tree's own churn overhead) is +0.096 (archetype_shift/off), +0.054 (burst_migrate/on) and <= +0.017 or negative on the other eight; (b) 0.05 ms is 0.5 % of a 9.6 ms step (1.2 % of 4.0 ms), below the SE bars of 0.9-4.6 %, so K=6 cannot resolve the rule's threshold either way. The rule's intent (a translation step costs <= 0.05 ms over a stable step) is better read on the maintenance bench: `shift_translation - stable` = 0.0227 ms at 1240 (rule 4, 0.37x its limit).

### 4.3 Recipe 2.3 - the G5 gates

| gate | rule | measured | verdict |
|---|---|---|---|
| headline Delta | T-A-tree vs T-A-allpairs at W=8 claimed faster; Delta-bp(8) >= 1.03 ms on the armed rows | -24.37 % (-1.619 ms), Y/Y/Y; Delta-bp(8) = 1.488 ms (median [100,500)) / 1.484 (mean [0,500)) | **PASS** (1.44x the bar; 0.83-0.87x the design's 1.72-1.80) |
| every W | claimed faster at W = 1, 2, 4, 16 | -8.60 / -13.71 / -19.42 / -24.06 %, Y/Y/Y at each | **PASS** |
| the default row | T-D-tree vs T-D-allpairs claimed faster at W = 1, 8; T-D-allpairs reproduces D-L5 (10.738 / 5.347) with no claimed shift; T-D-tree(8) against Jolt v5.6.0 2.569 | -14.47 / -32.63 %, Y/Y/Y; bridge -1.72 % (n/n/Y) / +2.68 % (n/n/n); **3.699 ms = 1.440x Jolt** (+44.0 %, n/Y/Y) | **PASS** (the bridge: no claimed shift under range or IQR; SE-only at W=1, section 2.2) |
| the tree span | sigma of the four `phys_bp_*` spans, median over [100,500), <= 0.36 (W1) / 0.35 (W8) ms, else stop | **0.4756 / 0.4662 ms** (1.32x / 1.33x) | **FAIL - stop** |
| structure | `void_steps == 0`, `check_step` green (four spans once, queried + members == N, no Wide/Excluded), `first_void == null` on every armed process | all 12 tree-armed processes: (1,1,1,1), 1241, void 0, first_void null | **PASS** |
| canary | the span reads F*T and the step rises by it | span within -1.4..+0.6 % of canary_ns; rise +0.875 ms = 0.98x at W1 (Y/Y/Y), +0.316 ms = 1.26x at W8 (n/Y/Y) | **PASS (seen)**, at W8 not under min-max |
| S16 | no claimed regression | -0.38 %, n/n/n | **PASS** |
| R | claimed faster at W = 1, 8 | -11.49 % (Y/Y/Y), -22.46 % (n/Y/Y) | **PASS under IQR and SE**; W8 not under min-max (one 7.440 ms process) |
| pose | `expect_pose: match` against the twin; one hash per (scene, cfg) across kinds and W | 72 match / 84 none / 0 mismatch among the used; one hash per scene (J's shared by both cfgs); the invalid original re-run | **PASS** on the used set; **the invalid original is a finding** (section 9.1) |

## 5. What cannot be claimed (the recipe's "What it cannot claim", and this window's own)

- **No W scaling of the Tree**: it is serial by design; the improved T(1)/T(8) of the tree rows (3.56 against 2.95) is the removal of a serial 1.95 ms span, not parallelism. C2 is deferred (section 7).
- **Nothing downstream of `ContactPairs`**: the pair set is identical (9,559 final pairs, one pose) and no stage below the broadphase was armed apart from the four spans; the 0.13-0.19 ms by which the step deltas exceed the span deltas is inside the step bars and is not attributed.
- **The C5 sleeping floor**: no `--sleeping` row was run; the churn `sleeping_on` cells are `row_identity_churn`'s scene, not J-Son.
- **Jolt parity**: a projection until C4 ships the default; the T-D-tree numbers are the same-binary flag, not the shipped default.
- **Under min-max**: T-D-tree / Jolt at W = 1 and 8 (Jolt's two outliers), R at W=8, the canary at W=8, `burst_migrate/off` on the churn row, and the bridge's W=1 SE-only shift are not claimed under all three readings; every other claim above is.
- **The design's Delta of 1.72-1.80 ms at W=8 is not reproduced** (1.488 measured); the 1.03 bar is.
- **The default cfg's own tree span** was not armed (the armed rows are cfg-A); the 12.6 % share of T-D-tree(8) assumes the cfg-A span.

## 6. The C4 constants (recipe 1.4)

### 6.1 `TREE_BRUTE_MAX_ROWS` - derived: **64** (unchanged from C1's provisional 64)

Rule: the largest n in {17, 64, 128, 256, 1k} at which `all_pairs/n` is not claimed slower than `tree/n`, the smaller of the uniform and disparity answers. all_pairs / tree (claimed under range / SE, either): uniform 0.103 (n), 0.619 (n), **1.137 (+13.7 %, bars 0.86 / 0.32: Y)**, 1.997 (Y), 4.143 (Y) -> 64; disparity 0.125 (n), 0.500 (n), **0.946 (-5.4 %, bars 3.32 / 1.25: not slower)**, 1.543 (Y), 2.839 (Y) -> 128; both families monotone. min(64, 128) = **64**. The crossover lies between grid points in both families (log-log interpolation of the ratio: 111 uniform, 138 disparity), so the recipe's own remedy applies to C4: add sizes between 64 and 256 to `G4_SIZES` and re-run the two families (K=3 each, about 10 min per process = 1 h). Sensitivity to the investigation: the value moves only if the tree's cost at n <= 64 falls by 38 % (uniform) / 50 % (disparity), or all_pairs' rises; 64 is the conservative side (running the brute loop up to 64 rows costs nothing today and at most the fix's margin after).

### 6.2 `AUTO_TREE_LO` / `AUTO_TREE_HI` - derived by the recipe's grid rule: **64 / 256**; by L2's procedure **126 / 140**; the band needs the refinement run

The recipe: HI = the smallest n at which `tree` is claimed faster than `all_pairs` (uniform 128, disparity 256), LO = the largest n at which `all_pairs` is not claimed slower (64, 128); the wider band, LO >= `TREE_BRUTE_MAX_ROWS`: **LO 64, HI 256**. L2's actual procedure for `GRID_LO/HI` (`broadphase_policy.rs`: HI = the larger family's log-log crossover to two significant figures, LO = a 10 % dead band under it) gives **HI 140, LO 126** (0.9 x HI, as L2's 2,700 = 0.9 x 3,000; the uniform crossover 111 sits below the band, as L2's uniform crossover 1,109 sat below its band). The price of the wide band, stated as L2 did: inside 128 <= n < 256 Auto holds AllPairs where the tree is claimed faster on uniform by 12-50 % (0.0014-0.022 ms per step at those n); inside 64 < n < 128 Auto holds AllPairs where it is faster on both families. Neither band is exercised by any timed row (the default ships `broadphase_select: Manual`, window 3's finding stands), so the choice costs nothing on J, R or S16. Recommendation: commit the recipe's 64 / 256 only after the refinement run, or L2's 126 / 140 with the two crossovers cited; the investigation bears on both the way it bears on 6.1.

### 6.3 `ADMIT_BUILD_RATIO` - **DEFERRED**; the recipe's formula as written reads 1.40 and cannot read the design's quantity

The recipe: (c_build + k * c_list) / c_q with c_build = (`admission_from_empty/m` - stable) / m, c_list = (`eviction_filter/m` - stable) / |L|, k = |L| / m, c_q = t_q(J) / 1240 = **333.8 ns/row** (check: uniform/tree/10000 / 10000 = 302.2 ns/row).

| m | L | c_build (recipe) ns/row | c_list ns/entry | k | numerator ns/row | ratio / c_q(J) | / c_q(10k check) | / the design's mid c_q |
|---|---|---|---|---|---|---|---|---|
| 1240 | 9,464 | 355.0 | 1.425 | 7.63 | 365.9 | 1.096 | 1.211 | 2.93 |
| **10000** | 83,149 | 456.0 | 1.333 | 8.31 | 467.1 | **1.399 -> (7, 5)** | 1.546 -> (11, 7) | 2.67 |
| 100000 | 867,834 | 581.7 | 1.973 | 8.68 | 598.8 | 1.794 | 1.982 | 2.40 |

The design expects 0.14-0.24. The formula gives >= 1 by construction: D3.5 defines admission as "rebuild X's tree over members u pending (radix only) + query each pending row + merge", so `admission_from_empty - stable` = m x (c_build + c_q) + merge and the recipe's "c_build" is c_build + c_q (+ merge) - it contains the denominator. The rent rule (D3.5) needs the radix-only c_build: admit when the queries the pending rows have paid (rent, in units of c_q) cover the admission's cost beyond the pending rows' own queries, (num/den) x (members + pending) x c_q = (c_build + k c_list)(members + pending). c_list is measured cleanly (1.33-1.97 ns/entry, design 1.5-2.5). The radix c_build is bounded from `admission_of_64` (its timed step = a radix rebuild over m + 64 + 64 queries + a merge): (a64 - stable - 64 x c_q(J)) / (m + 64) <= **11.6 / 22.9 / 53.3 ns/row** (<= 1.3 / 11.9 / 36.2 if the merge is one list pass) - consistent with the design's 12-20 (20-30 at 100k). With that bound: (c_build + k c_list) / c_q(J) <= **0.067 / 0.102 / 0.211** at 1240 / 10k / 100k (0.036 / 0.069 / 0.160 net of the merge), i.e. at m = 10k **(1, 8) or below** against today's (1, 4) - but only because the measured c_q is 2.2-3.3x the design's; with the design's c_q the same numerators give 0.18 / 0.19 / 0.28, inside its band. The constant is a ratio of two costs of which the denominator is the quantity under investigation (rule 1): **DEFERRED**. What the investigation must add is a c_build arm whose timed step is a radix rebuild alone (or the four spans on the `admission_of_64` timed step), and the recipe's formula corrected to use it.

## 7. The C2 decision (recipe 1.3, design D6)

t_q at J, W=1: **0.4140 ms** [0.4135-0.4175] (section 2.5); 334 ns per queried row; 0.411 at W=8. D6: C2 is built only if t_q x (1 - 1/(8E)) - omega(8) >= 5 % of the measured T(8) on a gated row. omega(8) = 6.54 us (the L5 design's colour-wave measurement, `levers/L5-narrowphase/02-DESIGN-REV1.md`); E is unmeasured for a query wave (the narrowphase's E(8) = 0.681 is the only in-tree proxy).

| gated row | T(8) | 5 % | saving at E = 1 | at E = 0.681 | at E = 0.5 | t_q at which the trigger fires (E = 0.681) | E at which it fires (omega = 6.5 us) |
|---|---|---|---|---|---|---|---|
| T-A-tree (cfg-A) | 5.023 | 0.251 | 0.356 (7.1 %) | 0.331 (**6.6 %**) | 0.304 (6.0 %) | >= 0.316 ms | >= 0.32 |
| T-D-tree (default) | 3.699 | 0.185 | 0.356 (9.6 %) | 0.331 (**8.9 %**) | 0.304 (8.2 %) | >= 0.235 ms | >= 0.23 |

At the measured t_q the D6 trigger **fires on both gated rows for any plausible E** (>= 0.23-0.32); at the design's t_q (0.124-0.186 ms, c_q 100-150 ns/row) the saving is 0.09-0.15 ms = 1.8-3.0 % of T-A-tree(8) and 2.4-4.0 % of T-D-tree(8), below 5 % on both - the design's own "deferred". The decision therefore turns on the investigation's t_q: **DEFERRED**, with the rule stated: **build C2 iff the post-investigation t_q(J, W=1) >= 0.235 ms on the default row (0.316 on cfg-A) at E = 0.68**; the 10k bound (4.8 ms on the J-like scene) is D6's second trigger and needs an end-to-end row of >= 8k bodies that does not exist.

## 8. One quantity behind the fired rules, and what the investigation must measure

The J-scale query costs 334 ns per row (43 ns per emitted pair) where the design's arithmetic has 100-150 ns per row; verify, build, assemble, c_list and the radix c_build are at or near their bands. That single excess explains, by arithmetic: rule 1 (0.443 vs 0.30: 0.414 of query + 0.03 of the rest), the tree-span FAIL (0.476 / 0.466 vs 0.36 / 0.35), rule 8 at 95-97 % of its limit (m x c_q inside the admission), rule 6 (h x c_q inside the compaction arm's timed step), the Delta-bp shortfall against the design (1.49 vs 1.72-1.80: the 0.24-0.32 ms the span is over), the recipe's `ADMIT_BUILD_RATIO` reading >= 1, and the D6 trigger firing. Rule 9 at 100k is separate and marginal (the list pass at 868k entries). None of it bears on the sign of any tree-vs-allpairs comparison: the tree beats AllPairs by 4.2x on the J snapshot at this cost.

The investigation, before C4 commits the constants and the flip (each item names its number and file):
1. **Counts per query on `bp_g4_scene/tree/j100`** (one untimed process, a counting build of the query kernel): 8-wide node tests, leaf candidates, exact tests and emitted pairs per query, against the design's "~10-17 8-wide tests per query" and 7.7 pairs per row; the same on `bp_g4_uniform/tree/1000` (153 ns/row, where the arithmetic holds) and `bp_g4_disparity/tree/1000` (231 ns/row). If the J counts are 3-4x the design's, the excess is the tree's shape on a dense pyramid (Morton order on 1240 unit boxes in a 50 x 50 slab: leaf occupancy, level count, node overlap); if the counts match, it is the per-test cost (the AVX2 path is compiled in - `-C target-cpu=x86-64-v3` - but whether the exact test and the emission are the scalar fallback is what a per-test cycle count shows).
2. **The four spans on the compaction arm's timed step** (`phys_bp_verify`, which holds the maintenance and the compaction, against `phys_bp_query`, which holds the h re-queries) at m = 1240 / 10k / 100k, or the arm rebuilt so that its timed step admits the half back untimed before the compaction. Expected: query ~ h x c_q (0.21 / 1.5 / 20 ms), compaction within 0.015-0.025 / 0.12-0.20 / 2-3 ms. Rule 6 is then re-read on the compaction span alone.
3. **A radix-only c_build** (a build arm with no queries, or `admission_of_64`'s spans) so that `ADMIT_BUILD_RATIO` = (c_build + k c_list) / c_q is computed from the quantities D3.5 names; today's bound says <= 0.07-0.10 at J / 10k with the measured c_q.
4. **The translation list pass at 100k** (rule 4 at 97 %, rule 9 at 105 %): the per-entry cost of the pass over 867,834 entries (5.6 ns/entry against c_list 1.5-2.5 + N x 4-6 ns) - a prefetch or layout question, not a J-scale one.
5. After 1-3: re-run the J snapshot cell and the armed rows (K=6, ~15 min) and re-read rule 1, the tree-span gate, D6 and the two constants; re-run uniform and disparity with the added sizes (6.1).

## 9. Open questions for the orchestrator, and files

1. **A W=1 determinism event in the shipped default, not in the tree.** `raw/pass-01/075_r2_T-D-allpairs_W1/` (`--cfg default --broadphase allpairs --workers 1`, 10:43:11, receipts 0.85 / 0.68 %, witness 0.22 %) hashed `0xf6e397d168e0e8b8`; 42,845 of 64,480 pose bytes differ from its twin. Against any matching J process its per-step CSV is identical through step 438 and **diverges at step 439**: `top_y` 28.990158 against 28.990156, then 58 of the remaining 61 steps differ in `top_y` and 48 in the manifold count; **the pair count never differs** (9,559 on every step of both) and the final counts match (4515 / 9559). The pair set is AllPairs' brute loop, so the broadphase is not the site; the divergence is numerical, late, and in a process whose SUMMARY prints `pool_workers 1, dispatcher 1` with `simd_solve`, `parallel_solve` and `parallel_narrowphase` all true - the one-worker-pool path window 3 already flagged for `D-L5`'s W=1 (+1.34 % SE-only) and which memory records as the "fixed the race, not the class" pattern. Rate: 1 of 13 default-cfg J W=1 processes this window (7 allpairs incl. the re-run, 6 tree), 0 of 30 cfg-A W=1 (whose flags are off at W=1), 0 of 12 default W=8, 0 of 24 R, 0 of the 25 untimed gate runs. One event sizes nothing; the cheapest next measurement is untimed: `T-D-allpairs@W1` and `T-D-tree@W1` at K >= 30 each with `--expect-pose` (about 5 s per process), then the same with `--parallel-solve off` / `--parallel-np off` to name the flag. It bears on the flip in one way: the default C4 would ship carries this path with either broadphase.
2. **The claim-rule reading is still stated two ways in the tree** (window 3 section 6.1): under SE the bridge shifts -1.7 % at W=1 and five churn arms are above 0.05 ms; under range and IQR neither. Every verdict above is printed under all three.
3. **Min-max casualties this window are few and named**: Jolt's two window-3 outliers (11.364, 3.124), `R-allpairs@W8`'s 7.440 (a witness of 6.49 % with clean receipts: the 5-s receipt cannot see a 3.75-s burst inside a 3.75-s process), `T-C-tree@W8`'s 6.039, and one k1 process in five sleeping-on churn cells. A during-process witness threshold as a gate is the same protocol question window 3 raised.
4. **The G4 grid is too coarse for the two crossovers** (64-128 uniform, 128-256 disparity); the recipe's refinement run is owed before `AUTO_TREE_LO/HI` is committed.
5. **Two defects in the recipe's arithmetic**, both fixable before the re-run: the `compaction - eviction_filter` subtraction (rule 6) and the `c_build` definition (1.4), for the reasons in 4.1 and 6.3.
6. **The tester's deviations** (the aborted first launch at 08:29 under a concurrent window; the desktop app minimized at 05:58 and restored twice) left no timed process in doubt: 0 build processes and 0 contaminated receipts in G5, 10 re-run slots in G4 all replaced by clean re-runs.

**Files.** This reduction: `win4/analysis.md`. Script and data: `win4/tools/analyze_win4.py`, `win4/analyst/reduction.json` (every cell, per-process value, span, comparison, gate, constant and receipt figure), `win4/analyst/tables.txt` (the script's printed tables). Everything quoted above is in `win4/raw/` (`runs.jsonl`, `pass-00/`, `pass-01/`, `g4/runs.jsonl`, `g4/<group>/<arm>/<param>/k*/estimates.json`, `g4/logs/`, `g4/wait_log.txt`, `window_log.txt`, `wait_log.txt`), `win4/wait_log.txt`, `win4/bin/SHA256SUMS`, `win4/logs/01-04`, or, for the Jolt and D-L5 cells, in `D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/runs.jsonl` and `.../raw/receipts/JOLT56-T_receipt_W1/receipt_discrete_th1.csv`. No file outside `win4/` was written; no tree, target or git state was touched.
