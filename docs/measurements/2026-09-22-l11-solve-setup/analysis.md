**T(1) J-As: tip 8.697 ms against parent 10.693 ms, Delta -1.996 ms (-18.7 %), claimed under SE and IQR (not under the pooled min-max bar; the two cells' min-max ranges are disjoint). T(8) J-As: tip 4.240 against parent 5.342, Delta -1.103 ms (-20.6 %), claimed under SE (ranges disjoint); against Jolt v5.6.0's 2.569 ms the tip is 1.650x (+65.0 %, claimed under all three readings), and at W=1 0.885x its 9.828 ms (-11.5 %, IQR and SE). G9 gates: T(1) PASS (-1.996 <= -0.81), T(8) PASS (-1.103 <= -0.61), solve_build PASS (-0.573 / -0.542 ms at W=1 / 8 <= -0.29), store PASS (-0.141 / -0.134 <= -0.11); wide colours and cfg-A not slower at any W (faster, claimed under SE). warm_apply is C3's gate and is not measurable on this tip.**

# Window 4b reduction: L11 C2 (`f8873aae`) against its parent C0 (`146a1125`) under G9 of `02-DESIGN-REV1.md` - recomputed from `win4b/raw/`

results-analyst, 2026-09-22. Inputs: `win4b/raw/runs.jsonl` (457 records), every per-process `run.csv` under `raw/main-p{0,1}/`, `raw/armed-p{0,1}/`, `raw/canary-p0/`, `raw/manifest.json`, `raw/window_log.txt`, `raw/wait_log.txt`, `bin/SHA256SUMS`, `logs/01_build_parent.log`, `logs/03_build_tip.log`; for the bridge and the Jolt reference, window 3's own `D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/` (`runs.jsonl`, the `JOLT56-T` per-frame CSVs under `pass-0{0,1}/`, `receipts/JOLT56-T_receipt_W1/receipt_discrete_th1.csv`). Reduction: `win4b/tools/analyze_win4b.py` -> `win4b/analyst/reduction.json` and `analyst/tables_analyst.md`; nothing imported from the tester's `reduce.py` (its slot rule was read to state where it coincides with mine, below). I did not read numbers off `window_report.md`, `raw/analysis.json` or `raw/tables.md`; where a number below coincides with the tester's, that is a reproduction, not a copy. Statistic and claim rule as ruled (rows.json, restating the P0/window-3 ruling): a cell = the MEDIAN over K separate processes of the process's window mean (ms/step); spreads = min-max range, IQR (inclusive quartiles) and the median's SE (1.2533 sample-SD / sqrt K), each as % of the median; tip against parent is claimed iff |median_tip/median_parent - 1| > 2 * hypot(spread_parent, spread_tip), read three ways (range / IQR / SE). G9 names the SE form as its gate; the min-max reading is printed beside it as rows.json asks. Git was used read-only in `D:/wt/lighttable` (ancestry between the parent and window 3's trees); nothing outside `win4b/` was written; no tree, target or git state was touched.

## 0. Selection, input checks, what the window report gets right, and two protocol notes

**Selection (the protocol's rule).** 457 records = 5 untimed warm-ups (one per pass) + 410 originals + 42 re-runs. Per (block, pass, seq) slot the original is used when valid and uncontaminated (both 5-s receipts <= 5 % busy and no build process, the driver's own flags), else its re-run. **410 processes used, 0 slots dropped**: all 42 contaminated originals (42 after-receipts over 5 %, 0 before-receipts, 0 build processes) have a clean re-run. The tester's rule prefers the re-run when both attempts are clean; a re-run exists only after a contaminated original, so the two rules pick the same 410 processes (checked: 0 slots differ). K = 12 in every cell (J-C: K = 1 per binary, as specified). 0 invalid attempts, 0 non-zero exits.

**Checks on the inputs (all hold):**
- Every window mean recomputed from the per-step file equals the driver's `mean_ms` exactly (max |delta| 0) and the runner's own `window_mean_ns` to 9.3e-10 ns. Every window holds exactly its row's step count (500 for J rows, 500 of 1100 for R, 500 of 800 for R-S, 300 for S16).
- Poses: every used process prints its row's expected hash (`0x32d5e235342b4143` J-As / J-A / J-As-a / J-C, `0x87e561d20589d4a5` R, `0x2a2b7926a48aab00` R-S, `0x8877dbb1192e9b92` S16); `expect_pose` = `match` in all 410 (every process, parent and tip, carried `--expect-pose` against the pose-gate file); 0 mismatches. The untimed pose gate (`gate/`) additionally holds the 501-step red control (exit 4, mismatch).
- `void_steps` 0, `drops_total` 0, disarmed ring traffic 0 in every process; `pool_workers` = W in all 410; affinity mask read back `0xffff` x410 (P-none); `target_env msvc`, zone tier `dev`; 1,240 bodies; the armed flag matches the row list in every process (R, R-S, S16, J-As-a, J-C armed; J-As, J-A disarmed).
- Binaries: `bin/SHA256SUMS` re-verified 2/2 (`runner_parent.exe` 8dfd0143..., `runner_tip.exe` 29dbd993...); every used process carries its binary's hash. Build logs: `Compiling boyko-physics v0.1.0 (C:\Users\flint\...\scratchpad\win4b\parent_tree\crates\boyko_physics)` for the parent (47.07 s, cold) and `(D:\wt\lighttable\crates\boyko_physics)` for the tip (44.27 s), both `Finished parity profile`.
- Printed config is field-equal between the two binaries in every row/W (14 row-W pairs checked). cfg `as` prints `simd_solve true`, `parallel_solve` / `parallel_broadphase` / `parallel_narrowphase` = (W > 1), AllPairs / Manual, sleeping off; cfg `a` the same with `simd_solve false`; R and R-S print the default (`parallel_solve true`, `parallel_narrowphase true`, `parallel_broadphase false`; R-S `sleeping true`).
- Counters are identical on both binaries at every W: J rows 4,524.246 manifolds / 9,561.196 pairs per step over [0,500) (4,519.3 over [100,500)), 10.79 colours, 9.11 wide, 109.32 waves, 16,838 wide slots, 16,888 points; R 6,662.3 manifolds, 16 colours, 168 waves; R-S 6,675 manifolds all frozen (0 awake, 0 waves); S16 16 manifolds. The design's denominator 4,524.2 is the [0,500) count, and it is the same before and after, as the lever's bit-identity requires.
- Ancestry (read-only): window 3's `de06b6c9` (L5 C4) is an ancestor of the parent `146a1125`; between them the only `boyko_physics` source commits are `8af0e3b9` (A1b, the row-identity key) and `146a1125` itself (L11 C0, tests + the J-As runner row). The tip is exactly C0 + `691891c4` (C1) + `f8873aae` (C2).
- Receipts: 499 distinct 5-s receipts: median 1.44 % busy, p90 4.81 %, max 12.35 %, **42 over 5 %** (4 in main-p0, 37 in main-p1, 1 in armed-p1, 0 in armed-p0 and canary), **0 with a build process**; no build process during any process. Idle rule before every one of the five blocks: three consecutive quiet 60-s polls (cpu10 0.7-4.6 %, 0 build and 0 lane-target processes), `High performance`, AC. During-process witness (`others_busy_pct`) over the 410 used processes: median 0.33 %, p95 3.48 %, max 8.68 %, 5 over 5 %. Background by CPU-seconds: `claude.exe` 90.5 s, `Telegram.exe` 85.2 s, `browser.exe` 54.7 s, everything else < 7 s each - main-p1 (08:55-09:57) carried nearly all of it; main-p0 (08:20-08:52), the armed block (09:59-10:09) and the canary (10:12) were quiet.

**Against the window report.** Every cell median, min-max, per-stage span, gate delta and claim reading in the report reproduces from the raw files (34 cells, 17 comparisons, both armed rows, the canary, the receipt counts). Two protocol notes, neither of which changes a verdict:

1. **The gate bars.** G9 reads "Realized-gain gates, >= 0.6 x the lower predicted Delta: solve_build -0.29 ms; warm_apply -0.19 ms; store -0.11 ms; T(1) J-As -0.81 ms; T(8) -0.61 ms." Against the design's own targets table those listed numbers ARE the 0.6x products: solve_build 1.038 -> 0.25-0.55 ms gives a lower Delta of -0.488, x0.6 = -0.29; store 0.270 -> 0.04-0.08 gives -0.19, x0.6 = -0.11; warm_apply 0.614 -> 0.15-0.30 gives -0.314, x0.6 = -0.19; T(1) -1.35...-2.37, x0.6 = -0.81; T(8) -1.01...-1.57, x0.6 = -0.61 (each to the printed rounding). The task text's "-0.29 -> bar 0.174, -0.11 -> 0.066, -0.81 -> 0.486, -0.61 -> 0.366" applies the 0.6 a second time, and the window report gated against those doubly-discounted bars. This reduction gates against the design's bars (binding) and prints the task's beside them; every gate passes under both, with margin.
2. **The bridge at W=8 is knob-mismatched.** The design's window-3 comparison rows are `D-L5` (the shipped default at `de06b6c9`) and `J-A`. cfg `as` at W=8 prints `parallel_broadphase true`; the default prints `false`. At W=1 the two are the same code path (one worker: inline solve, serial narrowphase, no broadphase split), so the W=1 bridge is clean; the W=8 pairing is reported with the caveat (section 4).

## 1. Tip against parent: every row, every W

**Cells: median over K=12 [min-max] ms/step; spreads range / IQR / SE as % of the median; per-pass medians (pass 0 / pass 1).** Values by process are in `analyst/tables_analyst.md`.

| row @W | parent | r / i / s % | p0 / p1 | tip | r / i / s % | p0 / p1 |
|---|---|---|---|---|---|---|
| J-As @1 | **10.693** [10.438-11.620] | 11.05 / 2.83 / 1.31 | 10.687 / 10.818 | **8.697** [8.395-9.308] | 10.50 / 3.00 / 1.12 | 8.674 / 8.923 |
| J-As @2 | 7.659 [7.624-9.178] | 20.30 / 7.53 / 2.73 | 7.630 / 8.387 | 6.221 [6.169-7.062] | 14.36 / 8.02 / 1.83 | 6.176 / 6.469 |
| J-As @4 | 6.176 [6.042-7.646] | 25.97 / 8.34 / 3.06 | 6.063 / 6.631 | 4.923 [4.890-5.334] | 9.03 / 2.58 / 1.29 | 4.901 / 5.098 |
| J-As @8 | **5.342** [5.215-6.903] | 31.59 / 12.58 / 3.97 | 5.236 / 5.988 | **4.240** [4.204-5.004] | 18.88 / 4.33 / 2.37 | 4.212 / 4.400 |
| J-As @16 | 5.469 [5.344-6.737] | 25.47 / 16.36 / 3.59 | 5.384 / 6.335 | 4.357 [4.329-5.201] | 20.01 / 3.32 / 2.63 | 4.350 / 4.582 |
| J-A @1 | 19.582 [19.275-20.335] | 5.41 / 0.72 / 0.49 | 19.546 / 19.683 | 18.932 [18.597-19.426] | 4.38 / 1.06 / 0.42 | 18.947 / 18.868 |
| J-A @2 | 12.325 [12.169-13.745] | 12.78 / 3.80 / 1.74 | 12.236 / 12.961 | 11.465 [11.390-12.054] | 5.79 / 1.49 / 0.71 | 11.523 / 11.452 |
| J-A @4 | 8.529 [8.440-10.338] | 22.25 / 4.35 / 2.59 | 8.468 / 8.999 | 7.769 [7.652-8.675] | 13.17 / 4.97 / 1.69 | 7.669 / 8.077 |
| J-A @8 | 6.592 [6.552-7.992] | 21.84 / 7.05 / 2.96 | 6.561 / 7.241 | 5.910 [5.764-6.974] | 20.47 / 11.50 / 2.56 | 5.773 / 6.331 |
| J-A @16 | 6.663 [6.412-8.201] | 26.85 / 20.22 / 3.95 | 6.511 / 7.910 | 5.660 [5.590-7.046] | 25.71 / 11.99 / 3.44 | 5.638 / 6.037 |
| R @1 | 14.300 [13.744-15.712] | 13.76 / 7.03 / 1.72 | 13.968 / 15.034 | 11.161 [10.760-11.642] | 7.90 / 4.19 / 1.00 | 11.210 / 11.088 |
| R @8 | 6.781 [6.672-8.196] | 22.47 / 5.75 / 2.81 | 6.731 / 7.223 | 5.246 [5.104-6.354] | 23.83 / 14.04 / 3.14 | 5.111 / 5.574 |
| R-S @1 | 6.021 [5.950-7.127] | 19.54 / 6.87 / 2.13 | 6.010 / 6.422 | 5.928 [5.839-6.300] | 7.77 / 1.45 / 0.83 | 5.912 / 6.028 |
| R-S @8 | 3.234 [3.193-5.724] | 78.25 / 13.67 / 8.05 | 3.226 / 3.696 | 3.114 [3.088-3.457] | 11.86 / 0.46 / 1.43 | 3.114 / 3.114 |
| S16 @1 | 0.0810 [0.078-0.086] | 9.71 / 3.20 / 0.99 | 0.080 / 0.082 | 0.0800 [0.078-0.084] | 7.45 / 2.82 / 0.84 | 0.080 / 0.082 |
| J-As-a @1 (armed) | 10.658 [10.549-10.861] | 2.93 / 0.60 / 0.27 | 10.628 / 10.678 | 8.554 [8.384-8.960] | 6.74 / 1.81 / 0.75 | 8.539 / 8.606 |
| J-As-a @8 (armed) | 5.275 [5.231-5.311] | 1.52 / 0.98 / 0.20 | 5.273 / 5.276 | 4.242 [4.220-4.294] | 1.76 / 0.31 / 0.18 | 4.248 / 4.234 |

**Tip against parent: ratio, effect, Delta, the bars 2 * hypot(spread_parent, spread_tip) under range / IQR / SE, the claim under each, and the min-max relation.**

| row @W | ratio | effect | Delta ms | bar_SE ms | bars r / i / s % | claimed r / i / s | min-max disjoint | Delta pass 0 / pass 1 |
|---|---|---|---|---|---|---|---|---|
| J-As @1 | **0.813** | **-18.67 %** | **-1.996** | 0.340 | 30.5 / 8.2 / 3.4 | **n / Y / Y** | yes (10.438 > 9.308) | -2.013 / -1.895 |
| J-As @2 | 0.812 | -18.77 % | -1.438 | 0.476 | 49.7 / 22.0 / 6.6 | n / n / Y | yes | -1.455 / -1.918 |
| J-As @4 | 0.797 | -20.29 % | -1.253 | 0.399 | 55.0 / 17.5 / 6.7 | n / Y / Y | yes | -1.161 / -1.533 |
| J-As @8 | **0.794** | **-20.64 %** | **-1.103** | 0.469 | 73.6 / 26.6 / 9.2 | **n / n / Y** | yes (5.215 > 5.004) | -1.025 / -1.588 |
| J-As @16 | 0.797 | -20.34 % | -1.112 | 0.454 | 64.8 / 33.4 / 8.9 | n / n / Y | yes | -1.034 / -1.753 |
| J-A @1 | 0.967 | -3.32 % | -0.650 | 0.251 | 13.9 / 2.6 / 1.3 | n / Y / Y | no | -0.598 / -0.815 |
| J-A @2 | 0.930 | -6.98 % | -0.860 | 0.459 | 28.1 / 8.2 / 3.8 | n / n / Y | yes | -0.713 / -1.509 |
| J-A @4 | 0.911 | -8.91 % | -0.760 | 0.514 | 51.7 / 13.2 / 6.2 | n / n / Y | no | -0.800 / -0.922 |
| J-A @8 | 0.897 | -10.35 % | -0.682 | 0.494 | 59.9 / 27.0 / 7.8 | n / n / Y | no | -0.788 / -0.910 |
| J-A @16 | 0.850 | -15.05 % | -1.003 | 0.654 | 74.4 / 47.0 / 10.5 | n / n / Y | no | -0.873 / -1.873 |
| R @1 | 0.781 | -21.95 % | -3.139 | 0.540 | 31.7 / 16.4 / 4.0 | n / Y / Y | yes | -2.757 / -3.946 |
| R @8 | 0.774 | -22.64 % | -1.535 | 0.504 | 65.5 / 30.3 / 8.4 | n / n / Y | yes | -1.620 / -1.649 |
| R-S @1 | 0.985 | -1.54 % | -0.093 | 0.275 | 42.1 / 14.1 / 4.6 | n / n / n | no | -0.098 / -0.394 |
| R-S @8 | 0.963 | -3.72 % | -0.120 | 0.528 | 158.3 / 27.4 / 16.4 | n / n / n | no | -0.112 / -0.583 |
| S16 @1 | 0.988 | -1.17 % | -0.001 | 0.002 | 24.5 / 8.5 / 2.6 | n / n / n | no | -0.001 / -0.000 |
| J-As-a @1 | 0.803 | -19.75 % | -2.105 | 0.141 | 14.7 / 3.8 / 1.6 | **Y / Y / Y** | yes | -2.089 / -2.071 |
| J-As-a @8 | 0.804 | -19.59 % | -1.033 | 0.026 | 4.7 / 2.1 / 0.5 | **Y / Y / Y** | yes | -1.026 / -1.042 |

- **The min-max reading is the casualty of pass 1's contamination, not of the data.** 37 of the 42 hot receipts fell in main-p1; every parent cell at W >= 2 sits 0.4-1.4 ms higher in pass 1 than in pass 0 (tip 0.1-0.4 ms), and the pooled ranges carry that. Pass 0 alone (K = 6 per side, the quiet pass) claims J-As at every W under **all three** readings (bars at W=1: 8.9 / 1.9 / 1.5 %; at W=8: 4.1 / 2.8 / 0.9 %), and R@W1 likewise; under the ruled pooled statistic the SE reading is what G9 names and it claims every J-As, J-A and R cell. In every J-As cell the two binaries' min-max ranges do not overlap at all (the parent's slowest-to-fastest process is above the tip's), which is a stronger statement than the 2 * hypot(range) bar can make.
- Sensitivity (not the ruled statistic): dropping the 5 used processes whose during-process witness exceeded 5 % moves no effect by more than 0.2 points and no claim reading. The [100,500) sub-window reads the same effects (-18.4 / -18.8 / -20.4 / -20.6 / -20.5 % for J-As at W = 1-16; -3.4 / -7.0 / -9.1 / -10.5 / -14.3 % for J-A), claimed under SE at every W.
- **Armed against disarmed, same binary** (J-As-a vs J-As): -0.33 % / -1.65 % at W=1 and -1.26 % / +0.05 % at W=8 (parent / tip), none claimed - the armed row is a fair proxy for the disarmed T and its tight spreads (SE 0.2-0.8 %) are the cleanest reading of the lever: **-19.75 % (-2.105 ms) at W=1 and -19.59 % (-1.033 ms) at W=8, claimed under all three readings.**
- **Scaling.** J-As T(1)/T(W) = 1.396 / 1.732 / 2.002 / 1.955 (parent) and 1.398 / 1.767 / 2.051 / 1.996 (tip) for W = 2 / 4 / 8 / 16: the lever is a near-constant ~20 % at every W, so the shape is unchanged; W=16 against W=8 is +2.4 % / +2.8 %, not claimed - neither binary scales beyond 8 workers. J-A: 1.589 / 2.296 / 2.971 / 2.939 (parent), 1.651 / 2.437 / 3.203 / 3.345 (tip).
- **R-S and S16 are not claimed either way** (R-S -1.5 % / -3.7 %, S16 -1.2 %; the R-S@W8 parent cell holds one 5.724 ms pass-1 process, +77 %). Pass 0 alone reads R-S@W8 at -3.5 % claimed under all three (K = 6), R-S@W1 -1.6 % under IQR only.

## 2. G9 as written: the realized-gain gates, wide colours, cfg-A

**The four gates.** Rule: the effect is claimed under G9's SE form AND Delta <= -bar. Bars: the design's (binding) and the task text's (doubly discounted) beside it. Stage deltas from the armed J-As-a rows, reading A (P0b's convention - per process the window mean over [0,500), then the median over K; the convention the targets were derived in), with reading B (the tester's: per process the median over steps [100,500), then the median over K) beside it.

| gate | row | parent -> tip (ms) | Delta ms | bar_SE ms | claimed r / i / s | lower predicted Delta | design bar (0.6x) | verdict | margin | task bar | verdict |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **T(1) J-As** | J-As @1 | 10.693 -> 8.697 | **-1.996** | 0.340 | n / Y / Y | -1.35 | **-0.81** | **PASS** | +1.186 | -0.486 | PASS |
| **T(8) J-As** | J-As @8 | 5.342 -> 4.240 | **-1.103** | 0.469 | n / n / Y | -1.01 | **-0.61** | **PASS** | +0.493 | -0.366 | PASS |
| **solve_build** | J-As-a @1, A | 0.864 -> 0.291 | **-0.573** | 0.014 | Y / Y / Y | -0.488 | **-0.29** | **PASS** | +0.283 | -0.174 | PASS |
| | J-As-a @8, A | 0.856 -> 0.314 | **-0.542** | 0.008 | Y / Y / Y | | -0.29 | PASS | +0.252 | -0.174 | PASS |
| | @1 / @8, B | 0.829 -> 0.275 / 0.839 -> 0.305 | -0.554 / -0.534 | 0.006 / 0.006 | Y/Y/Y both | | -0.29 | PASS | | | PASS |
| **store** | J-As-a @1, A | 0.163 -> 0.022 | **-0.141** | 0.028 | n / Y / Y | -0.19 | **-0.11** | **PASS** | +0.031 | -0.066 | PASS |
| | J-As-a @8, A | 0.160 -> 0.026 | **-0.134** | 0.006 | Y / Y / Y | | -0.11 | PASS | +0.024 | -0.066 | PASS |
| | @1 / @8, B | 0.157 -> 0.021 / 0.153 -> 0.025 | -0.136 / -0.128 | 0.025 / 0.004 | n/Y/Y, Y/Y/Y | | -0.11 | PASS | +0.026 / +0.018 | | PASS |
| warm_apply | J-As-a @1, A | 0.611 -> 0.519 | -0.092 | 0.002 | Y / Y / Y | (-0.314, C3) | (-0.19, **C3's**) | **not gated here** | | | |

- **T(1): -1.996 ms, 2.5x the design's bar and 1.5x the lower predicted Delta itself** (the design predicted T -1.35...-2.37 -> 8.67-9.79 ms; measured 8.697, at the fast edge of the band). Under the worst pairing of processes (the tip's slowest 9.308 against the parent's fastest 10.438) the delta is still -1.130 <= -0.81.
- **T(8): -1.103 ms, 1.8x the bar, inside the predicted -1.01...-1.57 band** (3.93-4.69 ms predicted, 4.240 measured). The worst pooled pairing (5.004 vs 5.215) is -0.211, which does not clear 0.61 - that pairing sets the tip's most contaminated pass-1 process (5.004, witness 6.8 %) against the parent's cleanest; in pass 0 alone the worst pairing is -0.985.
- **solve_build: 0.864 -> 0.291 ms (0.191 -> 0.064 us per manifold), -66 %**; the design's target band 0.25-0.55 ms holds the measured value at its fast end. The gate is cleared by 2x under either bar and under the worst process pairing (tip max 0.309 - parent min 0.859 = -0.550).
- **store: 0.163 -> 0.022 ms (0.036 -> 0.005 us per manifold), -87 %**; the design predicted 0.04-0.08 ms, the tip is BELOW that band. This is the tightest gate in absolute terms (margins +0.018...+0.031 ms), but the tip's per-process store values are 0.0211-0.0233 ms at W=1 (SE 0.0002 ms) and the parent's 0.1546-0.2929 (one process at 0.293, the other eleven 0.155-0.179; the outlier is a whole-process level, present under reading B too), so the worst pairing (0.0233 - 0.1546 = -0.131 at W=1; 0.0272 - 0.1451 = -0.118 at W=8) clears the design's 0.11 as well. The W=1 reading-A claim is n/Y/Y only because the parent's one 0.293 ms process makes the range bar 171 %; under IQR (14.7 %) and SE (17.0 %) the -86.8 % effect is claimed by a factor of five.
- **warm_apply is C3's gate (the 8-lane-wide apply) and C3 is not in this tip; it is reported, not gated:** 0.611 -> 0.519 ms at W=1 (-0.092, -15.0 %, claimed under all three) and 0.621 -> 0.532 at W=8 (-0.089). C1's merge-join warm start already moves it; C3's bar of -0.19 remains open for C3's own window.

**Wide colours (the kernel): the gate is "not claimed slower"; the -10...-25 % is claimed only if measured above SE.** Measured, J-As-a: **3.484 -> 2.520 ms at W=1 (-0.964 ms, -27.7 %; bars r / i / s 32.6 / 5.8 / 3.8 %; claimed n / Y / Y)** and **0.875 -> 0.625 ms at W=8 (-0.250 ms, -28.5 %; 9.5 / 3.7 / 1.1 %; Y / Y / Y)**. Per manifold 0.770 -> 0.557 us (W=1) and 0.193 -> 0.138 (W=8). Not slower: holds at both W; the gain is claimed above SE at both W, and it is larger than the design's -10...-25 % band and below its 2.68-3.22 ms target. Narrow colours move the same way (-27 % / -28 %, 16 -> 12 us and 18 -> 13 us; IQR and SE). The W=1 wide-colour range bar is set by two tip processes at 2.78 ms against ten at 2.39-2.54.

**cfg-A (scalar colours, `J-A`): not claimed slower at any W.** Holds at every W; the tip is faster at every W and the gain is claimed under SE at all five (W=1 under IQR too): -3.3 / -7.0 / -8.9 / -10.4 / -15.1 % (-0.650 / -0.860 / -0.760 / -0.682 / -1.003 ms). The design made no gain claim for cfg-A; the setup savings (build + store, about 0.7 ms) are what a scalar-colour step also pays.

**The canary (`J-C`, K = 1 per binary, a spin of 5 % of the step inserted as a system).** The span reads the spin on both binaries: parent 543,367 ns measured against 543,045 injected (+0.059 %), tip 428,595 against 428,103 (+0.115 %) - the profiler reads correctly on both. The wall rise: parent +0.494 ms for a 0.543 ms spin (J-C 11.153 against J-As-a@W1 10.658, outside that cell's min-max 10.549-10.861 - seen); tip +0.198 ms for a 0.428 ms spin (8.752 against 8.554, inside 8.384-8.960) - not resolvable at K = 1 against a cell whose range is 0.58 ms. The design asked for the canary to be seen; it is seen in the span on both binaries and in the wall on the parent.

## 3. The armed rows: per-stage us per manifold (the 4,524.2 denominator), and the R / R-S / S16 facts

**J-As-a, median over K=12 of per-process window means over [0,500) (reading A), ms and us per manifold, parent -> tip; the design's P0 baseline and L11 target beside them.**

| stage | W=1 parent -> tip, ms | us / manifold | Delta ms (effect) | claimed r/i/s | W=8 parent -> tip, ms | us / manifold | Delta ms (effect) | claimed | design: P0 ms / target ms |
|---|---|---|---|---|---|---|---|---|---|
| T (armed) | 10.658 -> 8.554 | 2.356 -> 1.891 | -2.105 (-19.8 %) | Y/Y/Y | 5.275 -> 4.242 | 1.166 -> 0.938 | -1.033 (-19.6 %) | Y/Y/Y | - |
| broadphase | 1.981 -> 1.902 | 0.438 -> 0.420 | -0.079 (-4.0 %) | n/Y/Y | 1.919 -> 1.915 | 0.424 -> 0.423 | -0.004 (-0.2 %) | n/n/Y | untouched |
| narrowphase | 3.006 -> 2.965 | 0.664 -> 0.655 | -0.040 (-1.3 %) | n/n/Y | 0.527 -> 0.522 | 0.117 -> 0.115 | -0.006 (-1.1 %) | n/n/Y | untouched |
| solve_colored (the solve span) | 5.235 -> 3.445 | 1.157 -> 0.762 | -1.790 (-34.2 %) | Y/Y/Y | 2.635 -> 1.615 | 0.583 -> 0.357 | -1.021 (-38.7 %) | Y/Y/Y | - |
| **solve_build** (setup) | **0.864 -> 0.291** | **0.191 -> 0.064** | **-0.573 (-66.3 %)** | Y/Y/Y | 0.856 -> 0.314 | 0.189 -> 0.069 | -0.542 (-63.3 %) | Y/Y/Y | 1.038 / 0.25-0.55 |
| **warm_apply** (C3 lever) | 0.611 -> 0.519 | 0.135 -> 0.115 | -0.092 (-15.0 %) | Y/Y/Y | 0.621 -> 0.532 | 0.137 -> 0.118 | -0.089 (-14.3 %) | Y/Y/Y | 0.614 / 0.15-0.30 (C3) |
| **store** | **0.163 -> 0.022** | **0.036 -> 0.005** | **-0.141 (-86.8 %)** | n/Y/Y | 0.160 -> 0.026 | 0.035 -> 0.006 | -0.134 (-83.8 %) | Y/Y/Y | 0.270 / 0.04-0.08 |
| **setup total** (build + warm + store) | **1.643 -> 0.832** | **0.363 -> 0.184** | **-0.811 (-49.3 %)** | Y/Y/Y | 1.638 -> 0.871 | 0.362 -> 0.193 | -0.767 (-46.8 %) | Y/Y/Y | 1.922 / 0.44-0.93 |
| **wide colours** (kernel) | **3.484 -> 2.520** | **0.770 -> 0.557** | **-0.964 (-27.7 %)** | n/Y/Y | 0.875 -> 0.625 | 0.193 -> 0.138 | -0.250 (-28.5 %) | Y/Y/Y | 3.576 / 2.68-3.22 |
| narrow colours | 0.016 -> 0.012 | 0.004 -> 0.003 | -0.004 (-27.3 %) | n/Y/Y | 0.018 -> 0.013 | 0.004 -> 0.003 | -0.005 (-28.1 %) | n/Y/Y | - |
| pass_biased / pass_relax | 1.183 -> 0.852 / 2.321 -> 1.682 | | -28.0 % / -27.6 % | n/Y/Y | 0.311 -> 0.227 / 0.587 -> 0.416 | | -27.2 % / -29.2 % | Y/Y/Y | - |
| restitution | 0.0058 -> 0.0033 | | -42 % | Y/Y/Y | 0.0061 -> 0.0034 | | -44 % | Y/Y/Y | - |
| integrate / gravity / write_back | 0.065 / 0.008 / 0.003 | | +0.2 / -3.8 / -3.1 %, none claimed above IQR | | 0.079 / 0.012 / 0.003 | | +3.2 / -1.9 / +1.3 % | | untouched |
| serial-in-solve sum (build+gravity+warm+integrate+restitution+store+write_back+sleep) | 1.724 -> 0.912 | 0.381 -> 0.201 | -0.813 (-47.1 %) | Y/Y/Y | 1.738 -> 0.973 | 0.384 -> 0.215 | -0.765 (-44.0 %) | Y/Y/Y | - |
| executor gap g | 0.237 -> 0.080 | | -0.156 | n/n/Y | 0.034 -> 0.033 | | -0.002 | n/Y/n | - |

Reading B (the tester's median-over-steps convention) gives the same picture with the stall tails removed: solve_build 0.829 -> 0.275 / 0.839 -> 0.305, store 0.157 -> 0.021 / 0.153 -> 0.025, warm 0.613 -> 0.519 / 0.625 -> 0.533, wide colours 3.472 -> 2.513 / 0.863 -> 0.619, solve span 5.214 -> 3.420 / 2.613 -> 1.602 (W=1 / W=8), every one claimed under SE and IQR, the W=8 stages under range too. The tester's per-stage numbers reproduce exactly under this reading.

- **The setup fell by half and the kernel by 28 %; the design's setup band (0.44-0.93 ms) holds the measured 0.832 ms only because warm_apply is still C3's 0.519.** With C3's predicted 0.15-0.30 the setup would sit at 0.46-0.61. The measured parent baselines (solve_build 0.864, warm 0.611, store 0.163, setup 1.643) are below the design's P0 J-B figures (1.038 / 0.614 / 0.270 / 1.922) - the P0 numbers came from a different binary (J-B, Grid + simd, the P0 tip) and the C0 parent already carries L4/L2/L5/A1b, so the realized deltas are measured against the actual parent, as G9 says, not against the P0 column.
- **Where the -2.1 ms at W=1 comes from (reading A): setup -0.811, wide colours -0.964, narrow -0.004, restitution -0.002, broadphase -0.079, narrowphase -0.040, executor gap -0.156, remainder (gather, build_graph, apply) -0.012; sum -2.07 against the T delta of -2.105.** At W=8: setup -0.767, wide -0.250, narrow -0.005, bp/np -0.010, gap -0.002, sum -1.03 against -1.033. The setup is serial at both W (0.83-0.87 ms on the tip, 1.64 on the parent, W-independent), as the design's "setup scaling at W=8 needs C4" says; the kernel gain is what scales.
- **Broadphase -4.0 % at W=1 (claimed under IQR and SE, not range) and narrowphase -1.3 % (SE only):** the lever does not touch either; the delta is the size of the collision stages' pass-1 drift and both are within 0.08 ms. Not claimed as a lever effect.

**R (rest scene, default cfg, window [600,1100), armed): the lever's largest absolute gain.** T 14.300 -> 11.161 ms at W=1 (-3.139, -22.0 %, n/Y/Y) and 6.781 -> 5.246 at W=8 (-1.535, -22.6 %, n/n/Y); 6,662 manifolds per step. Spans (reading A; per manifold divides by 6,662.3): solve_build 1.429 -> 0.463 (0.214 -> 0.070 us/m; -0.966 ms, SE only), store 0.415 -> 0.036 (0.062 -> 0.005; -0.378, SE only), warm_apply 0.845 -> 0.738 (-0.108, Y/Y/Y), wide colours 5.264 -> 3.678 (-1.586, -30.1 %, IQR and SE), solve span 8.161 -> 5.060 (-3.101, -38.0 %); at W=8 solve_build 1.270 -> 0.520, store 0.318 -> 0.046, wide 1.362 -> 0.934 (-31.4 %). **The parent's R-row setup spans are bimodal by process:** solve_build 1.19-1.43 ms in eight processes and 1.87-2.12 in four; store 0.24-0.31 in six and 0.51-0.68 in six (present under the step-median reading, so a whole-process level, not stalls); the tip's are tight (store 0.034-0.056, solve_build 0.42-0.53 with one 0.71). This is P0's "a process draws its own speed for its whole run" on the parent's per-point table and hash probe - the structures the lever deleted - and it is why the R setup deltas are claimed under SE only: their range and IQR bars are 100-275 %.

**R-S (everything frozen by step 300, window [300,800), armed): the tip is not claimed faster or slower on T (-1.5 % / -3.7 %), and it holds the window's only tip-slower stages, as facts for the design, not verdicts:**
- `solve_build` 0.0339 -> 0.0503 ms at W=1 (**+0.0164 ms, +48 %**, claimed under IQR and SE, not range) and 0.0345 -> 0.0513 at W=8 (+0.0168, +49 %, IQR and SE). On an all-frozen stream the tip's build does about 16 us more per step than the parent's - per manifold 0.0075 -> 0.0111 us over 6,675 frozen manifolds.
- narrow colours 0.0022 -> 0.0038 ms (+1.5 us, +68 %, claimed under all three at W=1; IQR and SE at W=8); there are no wide colours (0 waves).
- `store` 0.316 -> 0.127 ms at W=1 (-0.189, -60 %, SE only) and 0.259 -> 0.129 at W=8 (-0.131, not claimed - the parent's R-S store is bimodal 0.25 / 0.46-0.82 with a 1.74 ms process at W=8). **The tip's R-S store, 0.127-0.129 ms, is above the design's predicted 0.05-0.10 ms band for the B1 carry**; the parent's 0.316 / 0.259 sit at or below the design's "today" 0.32-0.37. The tip's store per process is tight (0.125-0.139).
- The solve span still falls: 0.449 -> 0.283 ms (-37 %, SE only) at W=1, 0.402 -> 0.287 at W=8 (not claimed). Sleep stages (begin / freeze / end) move by < 2 us, none claimed above IQR.

**S16 (16 manifolds, cfg-A, W=1):** T 0.0810 -> 0.0800 ms (-1.2 %, not claimed); solve_build 2.7 -> 1.3 us (-52 %, Y/Y/Y), store 0.4 -> 0.1 us (Y/Y/Y), warm 2.3 -> 2.0 us; narrow colours 44.4 -> 44.9 us (+1.2 %, not claimed). The fixed per-step cost of the setup fell by 2 us and the scalar kernel did not move.

## 4. The bridge to window 3, and the Jolt v5.6.0 reference

**Window 3's cells recomputed from its own `runs.jsonl`** (137 used of 171, same selection rule; all match `win3/analysis.md`): `J-A@W1` 19.483 [19.337-19.644] (range 1.57 / IQR 0.65 / SE 0.29 %), `J-A@W8` 9.174 [9.070-9.243] (1.89 / 0.89 / 0.36), `D-L5@W1` 10.738 [10.684-10.793] (1.01 / 0.73 / 0.23), `D-L5@W8` 5.347 [5.307-5.651] (6.43 / 1.70 / 1.24); Jolt v5.6.0 9.828 [9.518-11.364], 5.770, 3.581, 2.569 [2.501-3.124], 2.388 at W = 1-16, all K = 6.

| pairing | window 3 | this window (parent) | effect | bars r / i / s % | claimed | medians inside the other's min-max |
|---|---|---|---|---|---|---|
| **J-A @W1, equal knobs** (cfg-A at one worker, both trees) | 19.483 [19.337-19.644] | **19.582** [19.275-20.335] | **+0.51 %** (+0.100 ms) | 11.3 / 1.9 / 1.2 | **n / n / n** | yes / yes |
| **J-As @W1 vs D-L5 @W1** (same code path at one worker: inline solve, serial narrowphase; knobs print differently) | 10.738 [10.684-10.793] | **10.693** [10.438-11.620] | **-0.41 %** (-0.045 ms) | 22.2 / 5.8 / 2.7 | **n / n / n** | yes / yes |
| J-As @W8 vs D-L5 @W8 (KNOB MISMATCH: `parallel_broadphase` on in `as`, off in the default) | 5.347 [5.307-5.651] | 5.342 [5.215-6.903] | -0.09 % (-0.005 ms) | 64.5 / 25.4 / 8.3 | n / n / n | yes / yes |
| J-A @W8 vs window 3's J-A @W8 (L5's parallel narrowphase landed between the trees) | 9.174 [9.070-9.243] | 6.592 [6.552-7.992] | -28.2 % (-2.582 ms) | 43.8 / 14.2 / 6.0 | n / Y / Y | no / no |

**Verdict: the window is not suspect.** The equal-knob cell reproduces the day-old window to +0.5 % with no claimed shift under any reading and each median inside the other's min-max; the shipped-default W=1 cell (D-L5) reproduces the parent's J-As to -0.4 % the same way. The parent carries A1b and the two merges above `de06b6c9` on top of window 3's tree, so these are "equal knobs, different binaries" pairings of the class window 3 measured at +1.3 % (D-L5 vs D-L4L2, SE only) - and here they are inside 0.5 %. At W=8 the J-A pairing is not a bridge but a consistency check: the -2.58 ms it reads is L5's own gain (window 3 measured -2.435 ms for C4 against C2 and +2.626 ms for the flag alone), so the parent's J-A@W8 sits where L5 predicts it. The J-As/D-L5 @W8 pairing reproduces to -0.1 % despite the `parallel_broadphase` knob difference; that says the knob costs nothing measurable at W=8 under AllPairs, but it was not isolated, so it stays a caveat. Cross-window uncertainty to add to any claim that mixes the two windows: about 0.5 % at W=1 (SE bars 1.2-2.7 %).

**Jolt v5.6.0, the only reference (owner ruling), from window 3's cells; boyko / Jolt on the [0,500) window mean, and per manifold per step over [100,500) with each side's own receipted count (boyko 4,519.3; Jolt v5.6.0 8,489.0 from its `-receipt` run, recomputed from the receipt CSV; Jolt's [100,500) per-frame means recomputed from its six used per-frame files per W).**

| W | J-As parent / v5.6.0 | J-As tip / v5.6.0 (effect; bars r/i/s; claimed) | J-A tip / v5.6.0 | per manifold us: J-As tip / v5.6.0 | per-manifold ratio, tip | parent |
|---|---|---|---|---|---|---|
| **1** | 1.088 (+8.8 %; n/Y/Y) | **0.885** (**-11.5 %**; 43.0 / 8.9 / 7.3; **n / Y / Y**) | 1.926 (Y/Y/Y) | 1.938 / 1.1445 | **1.69x** | 2.07x |
| 2 | 1.327 | 1.078 (+7.8 %; 29.0 / 16.1 / 3.7; n/n/Y) | 1.987 | - | - | - |
| 4 | 1.724 | 1.374 (+37.4 %; Y/Y/Y) | 2.169 | - | - | - |
| **8** | 2.079 (Y/Y/Y) | **1.650** (**+65.0 %**; 61.5 / 10.6 / 10.6; **Y / Y / Y**) | 2.300 (Y/Y/Y) | 0.940 / 0.3063 | **3.07x** | 3.87x |
| 16 | 2.290 | 1.824 (+82.4 %; Y/Y/Y) | 2.370 | - | - | - |

- **T(1): the tip's J-As, 8.697 ms, is faster than Jolt v5.6.0's 9.828 by 11.5 % on the step, claimed under IQR and SE (not under min-max: the Jolt W=1 cell holds one 11.364 ms process, +16 %); over [100,500) 0.901x. The parent was slower than v5.6.0 by 8.8 %.** Per manifold the tip is 1.69x v5.6.0 (1.938 against 1.1445 us) - inside the design's predicted 1.68-1.90 band at its good edge; the per-step ratio 0.885 is below the design's predicted 0.90-1.01. The honest W=1 statement, as in window 3: 0.89x on the step, 1.69x per manifold, the truth between (boyko carries 1.88x fewer manifolds).
- **T(8): the tip's J-As, 4.240 ms, is 1.650x Jolt v5.6.0's 2.569 ms, claimed under all three readings (the parent was 2.079x); per manifold 3.07x.** Inside the design's predicted 1.58-1.88 per step and 2.95-3.52 per manifold. Against window 3's shipped default D-L5 at 2.081x, the lever has closed 0.43 of the W=8 ratio; T(1)/T(8) on the tip is 2.05 against v5.6.0's 3.82, and the remaining W=8 gap is the serial 0.87 ms setup, the 1.92 ms AllPairs broadphase and the 0.5 ms narrowphase, none of which this lever addresses.
- cfg-A (`J-A`) stays 1.9x-2.4x v5.6.0 at every W; it is not the shipped configuration and the design claims nothing for it beyond "not slower".

## 5. What cannot be claimed

- **Per-contact parity with v5.6.0.** 1.69x per manifold at W=1 and 3.07x at W=8; the design said even a zero setup leaves 1.77x, and the remainder is collision detection (broadphase 0.420 + narrowphase 0.655 us per manifold on the tip at W=1, 1.075 of the 1.891 total) owned by L5, the tree broadphase and L9.
- **A Jolt per-stage setup comparison.** Jolt was not re-run (owner ruling) and its setup sits inside FindCollisions in any case; only Jolt's window-3 T cells are used.
- **Setup scaling at W=8 without C4.** The setup is serial on both binaries: 1.64 ms (parent) and 0.83-0.87 ms (tip) at both W; the W=8 setup delta (-0.767) is the W=1 delta (-0.811) within the bars. C4's own in-binary A/B was not run.
- **The kernel gain is claimed only because it was measured above SE:** -27.7 % / -28.5 % wide colours at W=1 / W=8 (design: instruction-count estimate -10...-25 %, gated as "not slower"). It is claimed under IQR and SE at W=1 and all three at W=8; not under min-max at W=1 (two 2.78 ms tip processes).
- **C3's warm_apply gate (-0.19 ms) was not measured** - C3 is not in this tip; the -0.09 ms seen is C1's, reported as a fact.
- **The wide-colour and cfg-A "not slower" gates are passed by faster cells, not by no-change cells**; nothing here claims a tie anywhere.
- **Not claimed, per the readings:** every J-As, J-A and R effect under the pooled min-max reading (pass-1 contamination; pass 0 alone claims them under all three); J-A at W = 2-16 and J-As at W = 2, 8, 16 under IQR; R-S and S16 T either way; R's setup spans under range or IQR (the parent's bimodal processes); R-S's tip-slower solve_build and narrow colours as a defect (facts for the design: +16 us and +1.5 us per step on an all-frozen stream); W=16 against W=8 on either binary; broadphase / narrowphase deltas as lever effects (< 0.08 ms).
- **Not measured:** memory receipts (the design's ~2 MB committed-page shrink), C4, Jolt, the wall rise of the canary on the tip at K = 1, `parallel_broadphase` isolated at W=8, R-S at K beyond 12 (its T bars are 4.6 / 16.4 % under SE).
- **Old table bytes:** only values, poses, counters and spans are claimed; every pose and every counter is identical across the two binaries, as the lever's bit-identity requires.

## 6. Draft addendum for `docs/MEASUREMENT-QUEUE.md` and the L11 lane (not applied; the orchestrator/owner edits)

```markdown
**RESULT, 2026-09-22, window 4b (L11 C1+C2 against C0), G9 of 02-DESIGN-REV1.md.** One window, complete,
under the window-3 protocol block (median over K=12 separate processes of the window mean; spread = min-max,
IQR, the median's SE; claimed iff |effect| > 2*hypot(spread_A, spread_B), G9's SE form the gate; 5-s receipts
before and after every process, > 5 % or a build process => re-run once at the end of the pass; 10-s receipt
and three quiet 60-s polls before each block; P-none; every process ran with --expect-pose). Owner's
workstation (Ryzen 9 5900HS, 8C/16T, High performance, AC; the owner's agent sessions, Telegram and a browser
open - see contamination). Timed 08:20-10:12 +03:00. rustc 1.98.1 msvc, no RUSTFLAGS, CARGO_INCREMENTAL=0,
cargo `parity`, zone tier `dev`. Binaries: parent = C0 `146a1125` (git archive -> cold build; test-only +
the J-As runner row, no solver change) sha256 `8dfd0143`; tip = C2 `f8873aae` (lighttable, clean) `29dbd993`.
Poses per row bit-identical on both binaries in all 410 processes (J `0x32d5e235342b4143`, R
`0x87e561d20589d4a5`, R-S `0x2a2b7926a48aab00`, S16 `0x8877dbb1192e9b92`); counters identical; void 0, drops 0.

**Rows.** J-As and J-A at W 1/2/4/8/16, R and R-S at W 1/8, S16, armed J-As-a at W 1/8, J-C canary once
per binary: 34 cells, parent/tip interleaved inside each W group, 2 passes x 6 rounds, pass 1 reversed;
410 slots, 42 re-runs, K=12 everywhere, 0 dropped. Contamination: 499 receipts, median 1.4 %, 42 over 5 %
(37 in main pass 1), 0 build processes; pass 0, the armed block and the canary were quiet.

| W | J-As parent | J-As tip | Delta ms (effect; claimed r/i/s) | J-A parent -> tip | Jolt v5.6.0 (win 3) | tip / v5.6.0 |
|---|---|---|---|---|---|---|
| 1 | 10.693 [10.438-11.620] | **8.697** [8.395-9.308] | **-1.996 (-18.7 %; n/Y/Y; ranges disjoint)** | 19.582 -> 18.932 (-3.3 %, n/Y/Y) | 9.828 | **0.885** |
| 2 | 7.659 | 6.221 | -1.438 (-18.8 %; n/n/Y; disjoint) | 12.325 -> 11.465 (-7.0 %, SE) | 5.770 | 1.078 |
| 4 | 6.176 | 4.923 | -1.253 (-20.3 %; n/Y/Y; disjoint) | 8.529 -> 7.769 (-8.9 %, SE) | 3.581 | 1.374 |
| 8 | 5.342 [5.215-6.903] | **4.240** [4.204-5.004] | **-1.103 (-20.6 %; n/n/Y; disjoint)** | 6.592 -> 5.910 (-10.4 %, SE) | 2.569 | **1.650** |
| 16 | 5.469 | 4.357 | -1.112 (-20.3 %; n/n/Y; disjoint) | 6.663 -> 5.660 (-15.1 %, SE) | 2.388 | 1.824 |

R: 14.300 -> 11.161 (W=1, -3.139 ms, -22 %, n/Y/Y), 6.781 -> 5.246 (W=8, -1.535, -23 %, SE). R-S: -1.5 % /
-3.7 %, not claimed. S16: -1.2 %, not claimed. Armed J-As-a: -2.105 ms (W=1) and -1.033 (W=8), claimed under
all three (SE 0.2-0.8 %); armed vs disarmed not claimed on either binary.

- **G9 gates (design bars = 0.6 x the lower predicted Delta, as listed in the design): T(1) J-As -1.996 <=
  -0.81 PASS; T(8) -1.103 <= -0.61 PASS; solve_build -0.573 / -0.542 (W=1 / 8) <= -0.29 PASS; store -0.141 /
  -0.134 <= -0.11 PASS; wide colours not slower (faster: -27.7 % / -28.5 %, claimed above SE); cfg-A not
  slower at any W (faster at all five, SE). warm_apply -0.19 is C3's and is not measured on this tip (C1
  already reads -0.092).**
- **Per stage, J-As-a, us per manifold (4,524.2), parent -> tip at W=1 / W=8:** solve_build 0.191 -> 0.064 /
  0.189 -> 0.069; warm_apply 0.135 -> 0.115 / 0.137 -> 0.118; store 0.036 -> 0.005 / 0.035 -> 0.006;
  setup total 0.363 -> 0.184 / 0.362 -> 0.193 (1.643 -> 0.832 ms; serial at both W); wide colours 0.770 ->
  0.557 / 0.193 -> 0.138; solve span 1.157 -> 0.762 / 0.583 -> 0.357. Broadphase and narrowphase moved
  < 0.08 ms. Design targets: solve_build 0.25-0.55 ms (measured 0.291), store 0.04-0.08 (0.022, below),
  wide colours 2.68-3.22 (2.520, below), T(1) 8.67-9.79 (8.697), T(8) 3.93-4.69 (4.240).
- **The bridge holds:** the parent's J-A@W1 against window 3's J-A@W1 +0.5 % (n/n/n), its J-As@W1 against
  D-L5@W1 -0.4 % (n/n/n), medians inside each other's min-max; J-A@W8 sits -2.58 ms under window 3's, which
  is L5's measured gain.
- **Against Jolt v5.6.0 (window 3's cells, the only reference): at W=1 the tip is 0.885x (-11.5 %, IQR and
  SE; 1.69x per manifold); at W=8 1.650x (+65 %, all three; 3.07x per manifold). The parent was 1.088x and
  2.079x.**
- **Facts, not claims:** R-S (all frozen) is the only row with tip-slower stages - solve_build +16 us per step
  (+48 %, IQR and SE) and narrow colours +1.5 us; its tip store, 0.127-0.129 ms, is above the design's
  predicted 0.05-0.10. The parent's R-row setup spans are bimodal by process (store 0.24-0.31 / 0.51-0.68)
  where the tip's are tight. J-As W=16 is not faster than W=8 on either binary (+2.4 % / +2.8 %, not claimed).

**Not claimed / not measured.** Per-contact parity (1.69x / 3.07x per manifold); a Jolt setup comparison;
setup scaling at W=8 (serial 0.83-0.87 ms, C4 not run); C3's warm_apply gate; memory receipts; the pooled
min-max reading for the J and R rows (pass-1 contamination; pass 0 alone claims them under all three);
`parallel_broadphase` isolated at W=8.

Receipts: `docs/measurements/2026-09-22-physics-window4b/`: `window_report.md`, `analysis.md` (this
reduction), `rows.json`, `bin/SHA256SUMS`, `bin/COMMIT.txt`, `logs/`, `raw/` (`runs.jsonl`, `manifest.json`,
`window_state.json`, `window_log.txt`, `wait_log.txt`, `main-p{0,1}/`, `armed-p{0,1}/`, `canary-p0/`,
`analysis.json`, `tables.md`, `sensitivity.md`), `gate/` (the pose files and the red control), `tools/`,
`analyst/` (`reduction.json`, `tables_analyst.md`). `dry/` and `test/` are rehearsals. Leave out the exes
and `__pycache__/`.
```

Lane line for the L11 lane and the MEASUREMENT-QUEUE lever list: `L11 C1+C2 timed 2026-09-22, window 4b: J-As -1.996 ms at W=1 (10.693 -> 8.697) and -1.103 at W=8 (5.342 -> 4.240), setup 1.643 -> 0.832 ms, wide colours -28 %; all four G9 gates pass; 0.885x / 1.650x Jolt v5.6.0 at W=1 / W=8.`

## 7. Open points for the orchestrator, and files

1. **The gate bars in the task text are the design's bars discounted twice** (section 0, note 1). No verdict depends on it - every gate passes under both - but the record block should quote the design's -0.81 / -0.61 / -0.29 / -0.11 as the bars, not 0.486 / 0.366 / 0.174 / 0.066, or the next lever will inherit a 0.36x gate.
2. **R-S is the one row where the tip does measurably more work per step** (+16 us solve_build, +1.5 us narrow colours, on 6,675 frozen manifolds) and where the tip's store (0.128 ms) is above the design's predicted band. T is not claimed either way (-1.5 % / -3.7 %). Whether a frozen stream should pay the cohort build at all is a question for the L10 (sleeping) lane, which the design says maps its warm pieces onto these records; it is not a G9 red.
3. **The pooled min-max reading fails on 15 of 17 comparisons because of pass 1**, where the owner's sessions, Telegram and a browser ran (37 hot receipts in 62 minutes; the receipt sees the 5 s around a process, not the burst inside a 5-16 s process). Pass 0 alone claims every J-As cell under all three readings. If min-max stays a ruled reading, a during-process witness gate (this window's p95 is 3.5 %) is the protocol change window 3 already raised; the SE form G9 names is what this reduction gates on.
4. **The bridge at W=8 rests on a knob-mismatched pairing** (`parallel_broadphase`); a `--cfg default` J row on the parent at W=8, K=12, would make it clean (about 2 minutes). The W=1 bridge is clean and sufficient for the window's validity.
5. **C3's gate stays open.** warm_apply is 0.519 ms on the tip against C3's target 0.15-0.30; the design's setup band (0.44-0.93) is met at 0.832 only with C3 outstanding. C4 (setup scaling at W=8) is untouched: the setup is 0.83-0.87 ms at W=8, 20 % of the tip's 4.24 ms step.
6. **Memory receipts** (the design's ~2 MB committed-page shrink) were not taken; the row exists in the runner's protocol and is a 2-minute untimed leg.

**Files.** This reduction: `win4b/analysis.md`. Script and intermediate data: `win4b/tools/analyze_win4b.py` -> `win4b/analyst/reduction.json` (every cell, comparison, per-process value and span, receipt figure, the window-3 cells, the bridge, the Jolt reference), `win4b/analyst/tables_analyst.md`, `win4b/analyst/run.log`. Everything quoted above is in `win4b/raw/`, `win4b/bin/`, `win4b/gate/`, `win4b/logs/` or, for window 3, in `D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/`. No file outside `win4b/` was written; no tree, target or git state was touched; no commit was made.
