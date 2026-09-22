# Window 3: L4+L2 (and L5, C4 landed) against the P0b bridge and Jolt -- TIMED 2026-09-21 03:50-04:42 +03:00

Tester report. Every timing below comes from `win3/raw/` (`raw/runs.jsonl` -> `raw/analysis.json` by
`tools/analyze_win3.py`; the tables are `raw/tables.md` from `tools/render_tables.py`). Hashes, poses and
build facts cite `win3/logs/`, `win3/bin/SHA256SUMS`, `win3/rows.json`, `win3/rows_l5.json`. Nothing under
`D:/wt/l5np`, `D:/wt/lighttable`, their targets, or any git ref was touched; the only new things on disk
outside `win3/` are the detached worktree `D:/wt/mq-de06b6c9` and its target dir `D:/wt/_targets/mq-de06b6c9`
(both may be removed by the owner).

Root: `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad/win3/` (`win3/` below).

## 0. Verdict in five lines (ruled statistic = median over K processes of the [0,500) window mean, ms/step)

| W | D-L4L2 (aac562a7 default) | **D-L5 (de06b6c9 default = L4+L2+L5)** | J-A bridge (tip cfg-A) | Jolt v5.3.0 | Jolt v5.6.0 | D-L5 / v5.3.0 | D-L5 / v5.6.0 |
|---|---|---|---|---|---|---|---|
| 1 | 10.596 [10.549-10.759] (K=5) | **10.738** [10.684-10.793] | 19.483 [19.337-19.644] | 15.339 [15.068-19.042] | 9.828 [9.518-11.364] | **0.700** | 1.093 |
| 2 | 9.097 [8.944-10.184] | **7.864** [7.767-8.744] | -- | 8.571 [8.456-8.744] | 5.770 [5.703-5.818] | 0.918 | 1.363 |
| 4 | 8.280 [8.196-9.561] | **6.173** [6.106-6.710] | -- | 5.168 [5.106-5.250] | 3.581 [3.519-3.663] | 1.194 | 1.724 |
| 8 | 7.783 [7.693-8.042] | **5.347** [5.307-5.651] | 9.174 [9.070-9.243] | 3.583 [3.466-4.693] | 2.569 [2.501-3.124] | **1.493** | 2.081 |
| 16 | 8.161 [8.084-10.040] | **5.552** [5.480-6.838] | -- | 3.164 [3.112-3.205] | 2.388 [2.337-2.465] | 1.755 | 2.324 |

- **The bridge holds.** J-A re-taken in this window: 19.483 ms (W=1) and 9.174 ms (W=8) against P0b's 19.671 /
  9.162 -- shifts of -0.96 % and +0.13 %, inside P0b's own min-max ([19.079-19.835], [9.103-9.250]). Every Jolt
  cell is within -2.5..+3.1 % of P0b's median. This window's boyko/Jolt ratios are comparable to P0b's table.
- **L5 at W=8: -31.3 % against D-L4L2 (7.783 -> 5.347 ms, -2.435 ms), claimed under all three readings**
  (bars range 15.7 %, IQR 5.3 %, SE 3.0 %). Same-binary A/B `--parallel-np off` at W=8: 7.973 ms, i.e. the
  parallel narrowphase saves **2.626 ms** (+49.1 % when off; claimed under all three). P0b predicted 2.4-2.6 ms.
- **The default (L4+L2, simd_solve on) against the P0b bridge:** -45.6 % at W=1 (19.483 -> 10.596) and -15.2 %
  at W=8 (9.174 -> 7.783), both claimed under all three readings. P0b's derived (not measured) estimate for
  "cfg-A + simd_solve" was 11.0-11.1 ms at W=1 and 7.9-8.1 ms at W=8; the measured default (which also
  carries L4 and L2, neither of which changes this row at W=1) reads 10.596 / 7.783.
- **boyko is faster than Jolt v5.3.0 at W=1** (0.700x, i.e. -30.0 %, claimed under IQR and SE but NOT under
  min-max: the Jolt W=1 cell carries one 19.042 ms outlier, see section 4) and 1.49x at W=8, 1.76x at W=16.
  Against v5.6.0: 1.09x at W=1 (claimed under IQR/SE only), 2.08x at W=8.
- **Scaling T(1)/T(W) at W=8:** D-L5 2.008 (D-L4L2 1.362, J-A 2.124, Jolt v5.3.0 4.282, v5.6.0 3.825). boyko
  W=16 against W=8: D-L5 +3.8 % (not claimed under any reading; bars 50.6 / 8.2 / 10.1 %), D-L4L2 +4.9 %
  (claimed under IQR only, bar 4.3 %; range 48.8 %, SE 9.8 %) -- both W=16 cells carry one pass-0 outlier.
  Jolt W16 vs W8: v5.3.0 -11.7 %, v5.6.0 -7.0 %, neither claimed under any reading (JOLT-T@W8 carries two
  pass-0 outliers).

Cost of the flag at W=1: D-L5 vs D-L4L2 +1.34 % (10.596 -> 10.738 ms), claimed under SE only (bar 0.98 %), not
under IQR (1.49 %) or range (4.45 %). Under L5 a one-worker pool stays on the serial loop with no scope, so
this is a different-binary difference (`runner_l5.exe` vs `runner_l4l2.exe`, 925,613 differing bytes), not
necessarily the flag; `D-L5-npoff` vs `D-L4L2` at W=8 (same knobs, different binaries) reads +2.44 %, not
claimed under any reading (bars 35.1 / 9.9 / 6.8 %).

## 1. The flag, the lanes, and the C4 runner

- **Step A.** `tools/wait_flag.py` polled every 60 s from 00:45:56; 179 polls logged in `win3/wait_log.txt`.
  **The flag was seen on poll 179 at 03:45:21** (`win3/lanes_done.flag`):
  `l5np_head=de06b6c9`, `lighttable_head=8e9cd328`, `written=2026-09-21T03:48+03:00`, plus the note
  "hwrtshadow lane (wf_67b0c49f-95d) is in read-only research/design stages; its later dev/test stages will
  build - receipts decide". The log also records the lanes' HEADs moving: l5np `aac562a7` -> `b8d9ab8f` (C3,
  seen 01:02) -> `de06b6c9` (C4, seen 03:44); lighttable `1c31aeac` -> `8e9cd328` (seen 03:17).
- **Step B, case "C4 landed".** `git -C D:/wt/l5np log --oneline -4` (read-only): `de06b6c9 perf(physics):
  parallel_narrowphase is on by default, and the census pins the one scope and one block its dispatch costs on
  every frame (L5 C4)` <- `b8d9ab8f` (L5 C3, dormant flag) <- `aac562a7` (L2) <- `caac7d06` (L4).
  - Build (`logs/13_build_runner_l5.log`, `tools/build_l5.sh`): `git -C D:/wt/l5np worktree add --detach
    D:/wt/mq-de06b6c9 de06b6c9` (new), then `cargo bench --no-run --profile parity -p boyko-physics --bench
    jolt_parity_pyramid` with `RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_INCREMENTAL=0
    CARGO_BUILD_JOBS=6 CARGO_TARGET_DIR=D:/wt/_targets/mq-de06b6c9`, `RUSTFLAGS` unset: 42.90 s, exit 0.
    Worktree clean (0 status lines); `Cargo.lock` byte-identical to `D:/wt/l5np/Cargo.lock`; no `__pycache__`
    written into any tree. D: free 75.7 GB before the build (the lanes had released their caches).
  - `bin/runner_l5.exe` sha256 `26d17a102814bccbd5f1d2ab2ddd696fd4f14c90ed10069d0d9ee3788901a8ab`, appended to
    `bin/SHA256SUMS` (`sha256sum -c`: 5/5 OK). 925,613 bytes differ from `runner_l4l2.exe`.
  - Usage text at `de06b6c9` (`logs/14_l5_usage_and_knobs.log`): the flag is spelled **`--parallel-np on|off`**
    (the task's spelling; confirmed against the binary, exit 2 on a bad flag). `--cfg default` prints
    `parallel_narrowphase: true` at W=1 and W=8 (all other knobs as at `aac562a7`: colored, simd_solve on,
    parallel_solve on, AllPairs under Manual, sleeping off, substeps 4, relax 2, `target_env msvc`);
    `--parallel-np off` prints `parallel_narrowphase: false`.
  - **Pose gate (`logs/15_l5_pose_500_gate.log`, untimed):** `runner_l5 --cfg default --steps 500 --pose-out ...
    --expect-pose dry/l4l2_default_W1.pose.bin` at W = 1, 8, 16: pose `0x32d5e235342b4143`, `expect_pose:
    match`, exit 0, void 0, ring traffic 0, 4515 manifolds / 9559 pairs, all three pose files sha256
    `eff361e1bdf7261ddee5237cfbc4dbc3ad79d3a743a94fb2ebdee39c832e390f` = `runner_l4l2`'s. **Bit-identical,
    as designed.** Red control: 501 steps at W=8 -> `0x9336b30a06a7d8af`, `mismatch ... first differing body
    Some(0)`, exit 4.
  - `win3/rows_l5.json` filled: `D-L5` (`--cfg default`, W in {1,2,4,8,16}) and `D-L5-npoff` (`--cfg default
    --parallel-np off`, W=8), both with `--expect-pose` against the same-W `D-L4L2` pose of the pass.

## 2. What ran

- Rows (`rows.json` + `rows_l5.json`): `D-L4L2` (`runner_l4l2.exe` `368d4104...`, `aac562a7`, W 1/2/4/8/16),
  `J-A` (`runner_tip.exe` `ef9325ef...`, `dbd85977`, `--cfg a`, W 1/8), `D-L5` and `D-L5-npoff`
  (`runner_l5.exe` `26d17a10...`, `de06b6c9`), `JOLT-T` (v5.3.0 `29b23ad1...`), `JOLT56-T` (v5.6.0
  `918fd2b7...`), Jolt W 1/2/4/8/16. **23 cells, 138 slots at K=6.** Binaries verified against the manifest and
  `bin/SHA256SUMS` before each pass and after the window (`all match`).
- Protocol as ruled: 2 passes x 3 rounds; pass-0 order W ascending with Jolt and boyko alternating inside a W
  group (`JOLT-T, D-L4L2, JOLT56-T, J-A, D-L5` at W=1; `JOLT-T, D-L4L2, J-A, JOLT56-T, D-L5, D-L5-npoff` at
  W=8), pass 1 the whole list reversed; one untimed `D-L4L2 W=8` warm-up per pass; 10-s receipt before each
  pass, 5-s receipt between processes; >5 % busy => re-run once at the end of the pass; no band void; P-none
  (mask read back `0xffff` on every process, no affinity call). 500 steps, window [0,500); sub-windows [0,100)
  and [100,500) recomputed from every per-step CSV. Tools: `tools/window3_run.py` (P0b's `window_run.py` with
  the rows from `rows.json`, same launch path, receipts and witnesses), `tools/wait_idle3.ps1` (P0's poller plus
  the "no process running from `D:/wt/_targets` or `D:/wt/mq-*`" condition), `tools/analyze_win3.py` (P0b's
  `stats`/`compare` verbatim). Rehearsed untimed under `win3/test/` first (its numbers are not measurements).
- **Idle rule (step C):** pass 0 -- 3 consecutive quiet polls at 03:47:47 / 03:48:47 / 03:49:47 (cpu10 3.69 /
  2.50 / 3.51 %, 0 build processes, 0 lane-target processes), idle at 03:49:57; pass 1 -- idle at 04:30:25
  (`raw/wait_log.txt`). No build process appeared in any receipt during the window.
- **Wall clock (`raw/window_state.json`, `analysis.json:wall_clock`):** window start 03:47:46, first timed
  process 03:50:18, pass 0 03:50:08 -> 04:28:09 (69 originals + 29 re-runs), pass 1 04:30:35 -> 04:41:58
  (69 originals + 2 re-runs), Jolt `-receipt` runs 04:42:03-04:42:16, end 04:42:16; status `complete`.
  Power scheme High performance, battery 100 % (AC), process count 249 -> 264.
- **Structural checks (all green, `analysis.json`):** 0 non-zero exits; every boyko process pose
  `0x32d5e235342b4143` (D-L4L2, J-A, D-L5, D-L5-npoff -- one pose across all four rows and every W); every Jolt
  process one hash per exe (`0xee15b89965ec747` v5.3.0, `0xb8522b4e3fc62cfe` v5.6.0 -- the expected 500-step
  hashes); `expect_pose` gate `match` wherever a same-pass reference existed (D-L5 at every W, D-L4L2 at W=1/8,
  D-L5-npoff), `none` elsewhere; void steps 0, drops 0, disarmed ring traffic 0 everywhere; `pool_workers` = W;
  Jolt printed threads = W; `target_env msvc`, zone tier `dev`; 500 steps in every window.

## 3. Receipts and contamination -- READ THIS BEFORE THE NUMBERS

- 205 distinct receipts (2 of 10 s): cpu_avg min 0.47 %, **median 2.03 %**, p90 6.55 %, **max 22.5 %**;
  **32 receipts over 5 %**, 0 with a build process. (P0b: 266 receipts, median 1.26 %, 2 over 5 %.)
- **32 contaminated attempts, 31 re-runs, 1 slot excluded** (`D-L4L2@W1` pass 0 seq 2: original after-receipt
  6.33 %, re-run after-receipt 22.5 %; that cell is **K=5**). All other 22 cells are K=6. 0 invalid attempts.
- **The contaminant is `claude.exe` (agent sessions -- PIDs 23404 and 4744 dominate, at 1.5-3.5 % of the
  machine each in bursts) and `browser.exe` (one 15.7 % burst at 04:07:45).** Not the lanes: the last lane build
  process was seen at 03:37:19 (poll 171) in `win3/wait_log.txt`, and the idle rule saw none. 30 of the 32 contaminated attempts (29 originals and 1 re-run) are
  in pass 0 (03:50-04:28); pass 1 (04:30-04:42) had 2.
- **My own monitoring contributed to three receipts and one timed process, and the protocol absorbed all of
  them:** a `python.exe` inspection in the 04:00:45 receipt (10.96 %), a `grep.exe` progress check in the
  04:19:31 receipt (7.48 %) that also overlapped the timed `D-L4L2@W4` pass-0 seq-57 process (`grep.exe`
  1.48 CPU-s in its `others_top5`), and a `bash.exe` in the 04:15:43 receipt; each of those processes was flagged
  and re-run, and none of them is in the used set. A `rust-analyzer.exe` burst (04:13:58, 12.85 %) is the LSP
  server, not launched by me. The 22.5 % receipt at 04:23:33 names no long-lived process (top `claude.exe`
  0.41 %): short-lived processes that ended inside the 5-s window are invisible to the Toolhelp walk.
- **What the gate did not catch (the witness, not a gate):** the during-process "CPU seconds of every other
  process" witness (`others_busy_pct`) has median 0.67 % and max 5.1 % over the 137 used processes (P0b: median
  0.74 %, p95 1.44 %, max 7.5 %). **Every used process that sits more than 5 % above its cell median is a
  pass-0 process from 03:51-04:22, and 12 of those 13 have an elevated witness (1.09-5.1 %, `claude.exe` on
  top; the exception is `D-L5@W8` p0s64 at 0.52 %)**, while their 5-s bracketing receipts read under 5 %: `JOLT-T@W1` p0s47 19.042 ms (+24 %), `JOLT56-T@W1` p0s3 11.364 (+16 %),
  `D-L4L2@W2` p0s7 10.184 (+12 %), `D-L5@W2` p0s9 8.744 (+11 %), `D-L4L2@W4` p0s11 9.561 (+15 %), `D-L5@W4`
  p0s59 6.710 (+9 %), `D-L5@W8` p0s64 5.651 (+6 %), `D-L5-npoff@W8` p0s19 9.158 (+15 %), `JOLT-T@W8` p0s14
  4.693 (+31 %) and p0s60 3.883 (+8 %), `JOLT56-T@W8` p0s17 3.124 (+22 %), `D-L4L2@W16` p0s21 10.040 (+23 %),
  `D-L5@W16` p0s23 6.838 (+23 %). The per-process values are listed in the cells table. **The ruled statistic
  (the median) is unaffected by one such process per cell; the min-max range is inflated in those 12 cells
  (6-34 %; `JOLT-T@W8` carries two), which is why several ratios are claimed under IQR and SE but not under
  min-max.** Pass-0 and pass-1
  medians are both printed per cell; e.g. `JOLT-T@W8` 3.883 (pass 0) / 3.481 (pass 1), `D-L5@W8` 5.350 / 5.334.
- **Supplementary, NOT the ruled statistic (`analysis.json:cells.*.supp_witness_filtered`,
  `supp_comparisons_witness_filtered`):** the same cells with processes whose witness exceeds 1.5 % (P0b's p95)
  dropped. It removes 1 process from 12 cells (two of them, `D-L4L2@W8` p0s38 and `JOLT56-T@W16` p0s68, are not
  >5 % outliers) and none from the others, and it does not catch five of the 13 outliers above (witness
  0.52-1.34 %), so I did not tune it further. Under it `D-L4L2 / v5.3.0` at W=4 and W=16 and `D-L5 / v5.3.0`
  at W=4 become claimed under min-max too (bars 7.5 / 6.8 / 7.4 %), and every claim in section 4 that already
  held under IQR and SE keeps its sign and magnitude (largest median moves: `JOLT-T@W1` 15.339 -> 15.175 and
  `D-L5-npoff@W8` 7.973 -> 7.888, both -1.1 %).
- `% Processor Performance` (PDH, machine total, median over processes): 129.2 (pass 0), 130.45 (pass 1).

## 4. Comparisons (effect = median_B/median_A - 1; claimed iff |effect| > 2*hypot(spread_A, spread_B))

| comparison | ratio | effect | bar range / IQR / SE (%) | claimed |
|---|---|---|---|---|
| D-L4L2 vs J-A @W1 (the default vs the P0b bridge) | 0.544 | -45.6 % | 5.1 / 1.3 / 1.1 | **Y/Y/Y** |
| D-L4L2 vs J-A @W8 | 0.848 | -15.2 % | 9.7 / 4.5 / 1.9 | **Y/Y/Y** |
| D-L5 vs J-A @W1 | 0.551 | -44.9 % | 3.8 / 2.0 / 0.8 | **Y/Y/Y** |
| D-L5 vs J-A @W8 | 0.583 | -41.7 % | 13.4 / 3.8 / 2.6 | **Y/Y/Y** |
| D-L5 vs D-L4L2 @W1 | 1.013 | +1.3 % | 4.5 / 1.5 / 1.0 | n/n/Y |
| D-L5 vs D-L4L2 @W2 | 0.865 | -13.6 % (-1.232 ms) | 36.9 / 4.6 / 7.2 | n/Y/Y |
| D-L5 vs D-L4L2 @W4 | 0.746 | -25.5 % (-2.107 ms) | 38.3 / 5.0 / 7.6 | n/Y/Y |
| **D-L5 vs D-L4L2 @W8** | **0.687** | **-31.3 % (-2.435 ms)** | 15.7 / 5.3 / 3.0 | **Y/Y/Y** |
| D-L5 vs D-L4L2 @W16 | 0.680 | -32.0 % (-2.609 ms) | 68.5 / 7.6 / 13.7 | n/Y/Y |
| **D-L5-npoff vs D-L5 @W8 (same binary, flag off)** | **1.491** | **+49.1 % (+2.626 ms)** | 36.3 / 9.6 / 7.0 | **Y/Y/Y** |
| D-L5-npoff vs D-L4L2 @W8 (same knobs, different binaries) | 1.024 | +2.4 % | 35.1 / 9.9 / 6.8 | n/n/n |
| D-L5 / Jolt v5.3.0 @W1 | 0.700 | -30.0 % | 51.9 / 7.9 / 10.3 | n/Y/Y |
| D-L5 / Jolt v5.3.0 @W2 | 0.918 | -8.3 % | 25.7 / 4.0 / 5.0 | n/Y/Y |
| D-L5 / Jolt v5.3.0 @W4 | 1.194 | +19.4 % | 20.4 / 5.1 / 4.0 | n/Y/Y |
| D-L5 / Jolt v5.3.0 @W8 | 1.493 | +49.3 % | 69.7 / 17.6 / 13.7 | n/Y/Y |
| D-L5 / Jolt v5.3.0 @W16 | 1.755 | +75.5 % | 49.3 / 7.8 / 9.8 | Y/Y/Y |
| D-L5 / Jolt v5.6.0 @W1 | 1.093 | +9.3 % | 37.6 / 6.8 / 7.0 | n/Y/Y |
| D-L5 / Jolt v5.6.0 @W8 | 2.081 | +108.1 % | 50.2 / 7.1 / 9.8 | Y/Y/Y |
| D-L4L2 / Jolt v5.3.0 @W1 | 0.691 | -30.9 % | 52.0 / 7.7 / 10.3 | n/Y/Y |
| D-L4L2 / Jolt v5.3.0 @W8 | 2.172 | +117.2 % | 69.1 / 17.8 / 13.6 | Y/Y/Y |
| J-A / Jolt v5.3.0 @W1 (P0b: 1.251) | 1.270 | +27.0 % | 51.9 / 7.8 / 10.3 | n/Y/Y |
| J-A / Jolt v5.3.0 @W8 (P0b: 2.593) | 2.561 | +156.1 % | 68.6 / 17.4 / 13.5 | Y/Y/Y |
| Jolt v5.6.0 vs v5.3.0 @W1..16 | 0.641 / 0.673 / 0.693 / 0.717 / 0.755 | -36 / -33 / -31 / -28 / -25 % | -- | n/Y/Y, Y/Y/Y, Y/Y/Y, n/Y/Y, Y/Y/Y |

Sub-windows, D-L4L2 / Jolt v5.3.0 over [0,100) / [100,500): 0.706 / 0.692 (W=1), 1.108 / 1.049 (W=2),
1.698 / 1.579 (W=4), 2.414 / 2.119 (W=8), 2.831 / 2.534 (W=16). The full sub-window table is in section 6.

**Bridge to P0b** (`analysis.json:bridge_to_p0b`): J-A@W1 19.483 vs 19.671 (-0.96 %; P0b's median is 0.14 %
above this window's max, and this window's median is inside P0b's own [19.079-19.835]); J-A@W8 9.174 vs 9.162 (+0.13 %, inside this
window's min-max). Jolt v5.3.0: -2.44 / -2.53 / -1.17 / +1.40 / -1.41 % at W=1..16; v5.6.0: +1.85 / +1.09 /
+0.69 / +3.06 / -2.39 %. No cell moved beyond its own IQR+SE resolution from P0b except by less than the
smallest claimable change P0b listed (J at W=1: 1.4 / 2.0 % IQR / SE).

## 5. Manifold / pair receipts, and time per manifold

- **boyko (per-step CSV, identical in every process of every boyko row and W):** manifolds mean 4544.2 over
  [0,100), **4519.257 over [100,500)**, 4524.246 over [0,500), final 4515; pairs mean 9559.215 over [100,500),
  final 9559. (P0b: 4519.3 / 4524.2 -- the same contact set, as the one pose implies.)
- **Jolt's own counts (untimed `-receipt` runs, W=1, 500 steps, `raw/receipts/`):** v5.3.0 manifolds mean
  7031.2 over [0,100), **8456.0 over [100,500)**, 8171.0 over [0,500), final 8456, points 31034.9 over [100,500),
  hash `0xee15b89965ec747` (= the timed hash), exit 0. **v5.6.0, receipted for the first time** (P0b used
  v5.3.0's count for it): 7042.2 / **8489.0** / 8199.6, final 8489, points 31112.0, hash `0xb8522b4e3fc62cfe`,
  exit 0. Active bodies 1240 on every frame, both exes.
- **Per manifold per step over [100,500), each side's own count (us):**

| W | boyko D-L5 us/manifold | boyko D-L4L2 | Jolt v5.3.0 | Jolt v5.6.0 | D-L5 / v5.3.0 | D-L5 / v5.6.0 |
|---|---|---|---|---|---|---|
| 1 | 2.380 | 2.346 | 1.812 | 1.144 | 1.31 | 2.08 |
| 2 | 1.750 | 2.018 | 1.028 | 0.675 | 1.70 | 2.59 |
| 4 | 1.368 | 1.841 | 0.623 | 0.426 | 2.20 | 3.21 |
| 8 | 1.186 | 1.729 | 0.436 | 0.306 | 2.72 | 3.88 |
| 16 | 1.232 | 1.819 | 0.384 | 0.288 | 3.21 | 4.28 |

(`analysis.json:per_manifold` holds the D-L4L2 column; the D-L5 column is `cells.D-L5@W*.sub.100_500.median`
x 1000 / 4519.257: 10.755, 7.911, 6.183, 5.361, 5.569 ms.) The raw and per-manifold ratios bracket the
truth as in P0b: Jolt's extra ~3.9k manifolds are its speculative lateral contacts.

## 6. Full tables (from `raw/tables.md`)

### Cells: window mean over [0,500), ms/step; median over K processes [min-max]; spread as % of the median

| row | W | K | median ms | min | max | range % | IQR % | SD % | SE(med) % | per-pass medians | values by process |
|---|---|---|---|---|---|---|---|---|---|---|---|
| D-L4L2 | 1 | 5 | **10.596** | 10.549 | 10.759 | 1.98 | 0.13 | 0.77 | 0.43 | 10.671 / 10.596 | 10.583, 10.759, 10.549, 10.596, 10.597 |
| D-L4L2 | 2 | 6 | **9.097** | 8.944 | 10.184 | 13.63 | 1.62 | 5.15 | 2.63 | 9.153 / 9.064 | 10.184, 8.944, 9.153, 8.979, 9.130, 9.064 |
| D-L4L2 | 4 | 6 | **8.280** | 8.196 | 9.561 | 16.49 | 1.89 | 6.41 | 3.28 | 8.261 / 8.299 | 9.562, 8.261, 8.207, 8.196, 8.300, 8.403 |
| D-L4L2 | 8 | 6 | **7.783** | 7.693 | 8.042 | 4.48 | 2.05 | 1.72 | 0.88 | 7.948 / 7.770 | 7.693, 7.948, 8.042, 7.744, 7.770, 7.795 |
| D-L4L2 | 16 | 6 | **8.161** | 8.084 | 10.040 | 23.97 | 0.70 | 9.44 | 4.83 | 8.224 / 8.150 | 10.040, 8.163, 8.224, 8.158, 8.084, 8.150 |
| J-A | 1 | 6 | **19.483** | 19.337 | 19.644 | 1.57 | 0.65 | 0.57 | 0.29 | 19.371 / 19.540 | 19.337, 19.483, 19.371, 19.483, 19.540, 19.644 |
| J-A | 8 | 6 | **9.174** | 9.070 | 9.243 | 1.89 | 0.89 | 0.70 | 0.36 | 9.124 / 9.217 | 9.069, 9.197, 9.124, 9.243, 9.217, 9.151 |
| D-L5 | 1 | 6 | **10.738** | 10.684 | 10.793 | 1.01 | 0.73 | 0.45 | 0.23 | 10.696 / 10.776 | 10.701, 10.696, 10.684, 10.775, 10.793, 10.776 |
| D-L5 | 2 | 6 | **7.864** | 7.767 | 8.744 | 12.43 | 1.66 | 4.76 | 2.43 | 7.807 / 7.922 | 8.745, 7.767, 7.807, 7.922, 7.928, 7.793 |
| D-L5 | 4 | 6 | **6.173** | 6.106 | 6.710 | 9.78 | 1.62 | 3.70 | 1.89 | 6.174 / 6.171 | 6.106, 6.174, 6.710, 6.123, 6.255, 6.171 |
| D-L5 | 8 | 6 | **5.347** | 5.307 | 5.651 | 6.43 | 1.70 | 2.42 | 1.24 | 5.350 / 5.334 | 5.350, 5.345, 5.651, 5.334, 5.307, 5.454 |
| D-L5 | 16 | 6 | **5.552** | 5.480 | 6.838 | 24.47 | 3.74 | 9.52 | 4.87 | 5.749 / 5.499 | 6.838, 5.592, 5.749, 5.479, 5.499, 5.511 |
| D-L5-npoff | 8 | 6 | **7.973** | 7.805 | 9.158 | 16.97 | 4.51 | 6.44 | 3.30 | 8.262 / 7.839 | 9.158, 8.262, 8.058, 7.839, 7.805, 7.888 |
| JOLT-T | 1 | 6 | **15.339** | 15.068 | 19.042 | 25.91 | 3.86 | 10.04 | 5.14 | 15.798 / 15.175 | 15.798, 15.068, 19.042, 15.120, 15.504, 15.175 |
| JOLT-T | 2 | 6 | **8.571** | 8.456 | 8.744 | 3.36 | 1.14 | 1.19 | 0.61 | 8.574 / 8.569 | 8.678, 8.456, 8.574, 8.549, 8.569, 8.744 |
| JOLT-T | 4 | 6 | **5.168** | 5.106 | 5.250 | 2.80 | 1.98 | 1.22 | 0.63 | 5.233 / 5.126 | 5.106, 5.233, 5.250, 5.126, 5.125, 5.210 |
| JOLT-T | 8 | 6 | **3.583** | 3.466 | 4.693 | 34.25 | 8.65 | 13.16 | 6.73 | 3.883 / 3.481 | 4.693, 3.566, 3.883, 3.481, 3.466, 3.599 |
| JOLT-T | 16 | 6 | **3.164** | 3.112 | 3.205 | 2.95 | 1.17 | 1.05 | 0.54 | 3.160 / 3.168 | 3.190, 3.143, 3.160, 3.168, 3.112, 3.205 |
| JOLT56-T | 1 | 6 | **9.828** | 9.518 | 11.364 | 18.78 | 3.31 | 6.84 | 3.50 | 9.669 / 9.873 | 11.364, 9.518, 9.669, 10.073, 9.783, 9.873 |
| JOLT56-T | 2 | 6 | **5.770** | 5.703 | 5.818 | 1.99 | 0.87 | 0.74 | 0.38 | 5.783 / 5.768 | 5.718, 5.783, 5.818, 5.768, 5.703, 5.773 |
| JOLT56-T | 4 | 6 | **3.581** | 3.519 | 3.663 | 4.02 | 2.36 | 1.62 | 0.83 | 3.542 / 3.653 | 3.519, 3.542, 3.584, 3.579, 3.663, 3.653 |
| JOLT56-T | 8 | 6 | **2.569** | 2.501 | 3.124 | 24.26 | 3.09 | 9.30 | 4.76 | 2.593 / 2.545 | 3.124, 2.593, 2.504, 2.501, 2.545, 2.594 |
| JOLT56-T | 16 | 6 | **2.388** | 2.337 | 2.465 | 5.32 | 3.42 | 2.18 | 1.12 | 2.445 / 2.343 | 2.400, 2.445, 2.465, 2.377, 2.344, 2.337 |

### Sub-windows [0,100) and [100,500), ms/step (median over K [min-max])

| row | W | [0,100) median | [0,100) min-max | [100,500) median | [100,500) min-max | [100,500) range % | [100,500) SE % |
|---|---|---|---|---|---|---|---|
| D-L4L2 | 1 | 10.416 | 10.343-10.717 | 10.600 | 10.567-10.851 | 2.68 | 0.61 |
| D-L4L2 | 2 | 8.958 | 8.846-10.140 | 9.120 | 8.967-10.195 | 13.47 | 2.59 |
| D-L4L2 | 4 | 8.117 | 8.009-9.378 | 8.321 | 8.228-9.607 | 16.58 | 3.28 |
| D-L4L2 | 8 | 7.626 | 7.564-7.858 | 7.815 | 7.716-8.143 | 5.46 | 1.02 |
| D-L4L2 | 16 | 8.008 | 7.864-9.643 | 8.220 | 8.120-10.139 | 24.57 | 4.94 |
| J-A | 1 | 19.200 | 18.992-19.617 | 19.553 | 19.424-19.651 | 1.16 | 0.24 |
| J-A | 8 | 9.003 | 8.965-9.244 | 9.202 | 9.096-9.284 | 2.05 | 0.37 |
| D-L5 | 1 | 10.514 | 10.361-10.981 | 10.755 | 10.725-10.831 | 0.98 | 0.18 |
| D-L5 | 2 | 7.682 | 7.607-8.719 | 7.911 | 7.807-8.751 | 11.93 | 2.34 |
| D-L5 | 4 | 6.133 | 6.093-6.280 | 6.183 | 6.092-6.817 | 11.74 | 2.24 |
| D-L5 | 8 | 5.327 | 5.230-6.773 | 5.361 | 5.295-5.380 | 1.58 | 0.29 |
| D-L5 | 16 | 5.509 | 5.452-6.958 | 5.569 | 5.476-6.808 | 23.93 | 4.73 |
| D-L5-npoff | 8 | 7.713 | 7.610-8.930 | 7.992 | 7.836-9.214 | 17.25 | 3.40 |
| JOLT-T | 1 | 14.751 | 14.520-17.629 | 15.319 | 15.205-19.546 | 28.34 | 5.75 |
| JOLT-T | 2 | 8.085 | 8.052-8.377 | 8.694 | 8.558-8.836 | 3.21 | 0.63 |
| JOLT-T | 4 | 4.780 | 4.713-4.796 | 5.269 | 5.204-5.364 | 3.03 | 0.71 |
| JOLT-T | 8 | 3.158 | 3.101-3.964 | 3.689 | 3.557-4.875 | 35.73 | 7.04 |
| JOLT-T | 16 | 2.829 | 2.759-2.855 | 3.244 | 3.200-3.300 | 3.07 | 0.55 |
| JOLT56-T | 1 | 10.279 | 9.956-11.274 | 9.716 | 9.409-11.387 | 20.36 | 3.81 |
| JOLT56-T | 2 | 5.825 | 5.714-6.076 | 5.730 | 5.691-5.773 | 1.42 | 0.35 |
| JOLT56-T | 4 | 3.440 | 3.426-3.513 | 3.617 | 3.543-3.715 | 4.76 | 0.94 |
| JOLT56-T | 8 | 2.371 | 2.297-2.782 | 2.600 | 2.547-3.210 | 25.49 | 5.08 |
| JOLT56-T | 16 | 2.156 | 2.126-2.200 | 2.447 | 2.388-2.532 | 5.89 | 1.21 |

### Jolt's own metric: printed steps/s (median over K [min-max]) and 1000/steps_per_s

| row | W | K | steps/s median | min | max | range % | SE % | ms from steps/s (median) | per-frame mean ms (cell median) | hash | threads |
|---|---|---|---|---|---|---|---|---|---|---|---|
| JOLT-T | 1 | 6 | 65.199 | 52.515 | 66.366 | 21.25 | 4.18 | 15.3395 | 15.3395 | 0xee15b89965ec747 | 1 |
| JOLT-T | 2 | 6 | 116.666 | 114.358 | 118.254 | 3.34 | 0.60 | 8.5715 | 8.5715 | 0xee15b89965ec747 | 2 |
| JOLT-T | 4 | 6 | 193.520 | 190.469 | 195.862 | 2.79 | 0.62 | 5.1678 | 5.1678 | 0xee15b89965ec747 | 4 |
| JOLT-T | 8 | 6 | 279.130 | 213.079 | 288.510 | 27.02 | 5.29 | 3.5826 | 3.5826 | 0xee15b89965ec747 | 8 |
| JOLT-T | 16 | 6 | 316.069 | 311.979 | 321.332 | 2.96 | 0.54 | 3.1639 | 3.1639 | 0xee15b89965ec747 | 16 |
| JOLT56-T | 1 | 6 | 101.750 | 87.994 | 105.061 | 16.77 | 3.09 | 9.8282 | 9.8282 | 0xb8522b4e3fc62cfe | 1 |
| JOLT56-T | 2 | 6 | 173.296 | 171.888 | 175.351 | 2.00 | 0.38 | 5.7705 | 5.7705 | 0xb8522b4e3fc62cfe | 2 |
| JOLT56-T | 4 | 6 | 279.217 | 272.967 | 284.142 | 4.00 | 0.82 | 3.5814 | 3.5814 | 0xb8522b4e3fc62cfe | 4 |
| JOLT56-T | 8 | 6 | 389.251 | 320.087 | 399.884 | 20.50 | 3.98 | 2.5693 | 2.5693 | 0xb8522b4e3fc62cfe | 8 |
| JOLT56-T | 16 | 6 | 418.688 | 405.754 | 427.813 | 5.27 | 1.11 | 2.3885 | 2.3885 | 0xb8522b4e3fc62cfe | 16 |

### boyko receipts per row (per-step CSV columns; the set of distinct per-process values)

| row | W | manifolds mean [0,100) | [100,500) | [0,500) | final | pairs mean [100,500) | final pairs | pose | expect_pose | pool_workers |
|---|---|---|---|---|---|---|---|---|---|---|
| D-L4L2 | 1 | [4544.2] | [4519.257] | [4524.246] | [4515.0] | [9559.215] | [9559.0] | 0x32d5e235342b4143 | match | [1] |
| D-L4L2 | 2 | [4544.2] | [4519.257] | [4524.246] | [4515.0] | [9559.215] | [9559.0] | 0x32d5e235342b4143 | none | [2] |
| D-L4L2 | 4 | [4544.2] | [4519.257] | [4524.246] | [4515.0] | [9559.215] | [9559.0] | 0x32d5e235342b4143 | none | [4] |
| D-L4L2 | 8 | [4544.2] | [4519.257] | [4524.246] | [4515.0] | [9559.215] | [9559.0] | 0x32d5e235342b4143 | match | [8] |
| D-L4L2 | 16 | [4544.2] | [4519.257] | [4524.246] | [4515.0] | [9559.215] | [9559.0] | 0x32d5e235342b4143 | none | [16] |
| J-A | 1 | [4544.2] | [4519.257] | [4524.246] | [4515.0] | [9559.215] | [9559.0] | 0x32d5e235342b4143 | none | [1] |
| J-A | 8 | [4544.2] | [4519.257] | [4524.246] | [4515.0] | [9559.215] | [9559.0] | 0x32d5e235342b4143 | none | [8] |
| D-L5 | 1 | [4544.2] | [4519.257] | [4524.246] | [4515.0] | [9559.215] | [9559.0] | 0x32d5e235342b4143 | match | [1] |
| D-L5 | 2 | [4544.2] | [4519.257] | [4524.246] | [4515.0] | [9559.215] | [9559.0] | 0x32d5e235342b4143 | match | [2] |
| D-L5 | 4 | [4544.2] | [4519.257] | [4524.246] | [4515.0] | [9559.215] | [9559.0] | 0x32d5e235342b4143 | match | [4] |
| D-L5 | 8 | [4544.2] | [4519.257] | [4524.246] | [4515.0] | [9559.215] | [9559.0] | 0x32d5e235342b4143 | match | [8] |
| D-L5 | 16 | [4544.2] | [4519.257] | [4524.246] | [4515.0] | [9559.215] | [9559.0] | 0x32d5e235342b4143 | match | [16] |
| D-L5-npoff | 8 | [4544.2] | [4519.257] | [4524.246] | [4515.0] | [9559.215] | [9559.0] | 0x32d5e235342b4143 | match | [8] |

### Jolt's own receipts (UNTIMED `-receipt` runs, W=1, 500 steps; the patch's counting ContactListener)

| exe | exit | frames | manifolds mean [0,100) | [100,500) | [0,500) | final | points mean [100,500) | active bodies min | hash |
|---|---|---|---|---|---|---|---|---|---|
| JOLT-T | 0 | 500 | 7031.2 | 8456.0 | 8171.0 | 8456.0 | 31034.9 | 1240.0 | 0xee15b89965ec747 |
| JOLT56-T | 0 | 500 | 7042.2 | 8489.0 | 8199.6 | 8489.0 | 31112.0 | 1240.0 | 0xb8522b4e3fc62cfe |

### Per manifold per step over [100,500), each side's own count (us)

| W | boyko manifolds | boyko us/manifold | Jolt 5.3.0 manifolds | Jolt 5.3.0 us/manifold | ratio b/J5.3 | Jolt 5.6.0 manifolds | Jolt 5.6.0 us/manifold | ratio b/J5.6 |
|---|---|---|---|---|---|---|---|---|
| 1 | 4519.3 | 2.346 | 8456.0 | 1.812 | 1.295 | 8489.0 | 1.144 | 2.049 |
| 2 | 4519.3 | 2.018 | 8456.0 | 1.028 | 1.963 | 8489.0 | 0.675 | 2.990 |
| 4 | 4519.3 | 1.841 | 8456.0 | 0.623 | 2.955 | 8489.0 | 0.426 | 4.321 |
| 8 | 4519.3 | 1.729 | 8456.0 | 0.436 | 3.964 | 8489.0 | 0.306 | 5.646 |
| 16 | 4519.3 | 1.819 | 8456.0 | 0.384 | 4.741 | 8489.0 | 0.288 | 6.311 |

### Comparisons (B against A; effect = median_B/median_A - 1; claimed iff |effect| > 2*hypot(spread_A, spread_B), three readings)

| comparison | ratio B/A | effect % | bar range % | bar IQR % | bar SE % | claimed range/IQR/SE |
|---|---|---|---|---|---|---|
| boyko D-L4L2 / Jolt v5.3.0 @W1 | 0.691 | -30.92 | 51.97 | 7.71 | 10.31 | n/Y/Y |
| boyko D-L4L2 / Jolt v5.6.0 @W1 | 1.078 | +7.82 | 37.78 | 6.62 | 7.05 | n/Y/Y |
| Jolt v5.6.0 vs v5.3.0 @W1 | 0.641 | -35.93 | 64.00 | 10.16 | 12.43 | n/Y/Y |
| D-L5 vs D-L4L2 @W1 | 1.013 | +1.34 | 4.45 | 1.49 | 0.98 | n/n/Y |
| boyko D-L5 / Jolt v5.3.0 @W1 | 0.700 | -30.00 | 51.86 | 7.85 | 10.28 | n/Y/Y |
| boyko D-L5 / Jolt v5.6.0 @W1 | 1.093 | +9.26 | 37.62 | 6.78 | 7.01 | n/Y/Y |
| boyko D-L4L2 / Jolt v5.3.0 @W2 | 1.061 | +6.13 | 28.07 | 3.96 | 5.41 | n/Y/Y |
| boyko D-L4L2 / Jolt v5.6.0 @W2 | 1.576 | +57.64 | 27.54 | 3.67 | 5.32 | Y/Y/Y |
| Jolt v5.6.0 vs v5.3.0 @W2 | 0.673 | -32.68 | 7.81 | 2.87 | 1.43 | Y/Y/Y |
| D-L5 vs D-L4L2 @W2 | 0.865 | -13.55 | 36.88 | 4.63 | 7.17 | n/Y/Y |
| boyko D-L5 / Jolt v5.3.0 @W2 | 0.918 | -8.25 | 25.74 | 4.03 | 5.02 | n/Y/Y |
| boyko D-L5 / Jolt v5.6.0 @W2 | 1.363 | +36.29 | 25.17 | 3.75 | 4.93 | Y/Y/Y |
| boyko D-L4L2 / Jolt v5.3.0 @W4 | 1.602 | +60.22 | 33.44 | 5.47 | 6.68 | Y/Y/Y |
| boyko D-L4L2 / Jolt v5.6.0 @W4 | 2.312 | +131.19 | 33.94 | 6.05 | 6.77 | Y/Y/Y |
| Jolt v5.6.0 vs v5.3.0 @W4 | 0.693 | -30.70 | 9.80 | 6.17 | 2.08 | Y/Y/Y |
| D-L5 vs D-L4L2 @W4 | 0.746 | -25.45 | 38.34 | 4.98 | 7.58 | n/Y/Y |
| boyko D-L5 / Jolt v5.3.0 @W4 | 1.194 | +19.45 | 20.35 | 5.11 | 3.98 | n/Y/Y |
| boyko D-L5 / Jolt v5.6.0 @W4 | 1.724 | +72.35 | 21.16 | 5.73 | 4.13 | Y/Y/Y |
| boyko D-L4L2 / Jolt v5.3.0 @W8 | 2.172 | +117.23 | 69.08 | 17.78 | 13.58 | Y/Y/Y |
| boyko D-L4L2 / Jolt v5.6.0 @W8 | 3.029 | +202.92 | 49.35 | 7.41 | 9.68 | Y/Y/Y |
| Jolt v5.6.0 vs v5.3.0 @W8 | 0.717 | -28.29 | 83.95 | 18.37 | 16.49 | n/Y/Y |
| D-L5 vs D-L4L2 @W8 | 0.687 | -31.29 | 15.67 | 5.33 | 3.04 | Y/Y/Y |
| boyko D-L5 / Jolt v5.3.0 @W8 | 1.493 | +49.26 | 69.69 | 17.63 | 13.69 | n/Y/Y |
| boyko D-L5 / Jolt v5.6.0 @W8 | 2.081 | +108.13 | 50.20 | 7.05 | 9.84 | Y/Y/Y |
| boyko D-L4L2 / Jolt v5.3.0 @W16 | 2.579 | +157.93 | 48.31 | 2.73 | 9.72 | Y/Y/Y |
| boyko D-L4L2 / Jolt v5.6.0 @W16 | 3.417 | +241.66 | 49.11 | 6.97 | 9.92 | Y/Y/Y |
| Jolt v5.6.0 vs v5.3.0 @W16 | 0.755 | -24.51 | 12.17 | 7.22 | 2.48 | Y/Y/Y |
| D-L5 vs D-L4L2 @W16 | 0.680 | -31.97 | 68.52 | 7.61 | 13.72 | n/Y/Y |
| boyko D-L5 / Jolt v5.3.0 @W16 | 1.755 | +75.47 | 49.30 | 7.84 | 9.80 | Y/Y/Y |
| boyko D-L5 / Jolt v5.6.0 @W16 | 2.324 | +132.44 | 50.09 | 10.13 | 10.00 | Y/Y/Y |
| D-L4L2 vs J-A (bridge) @W1 | 0.544 | -45.61 | 5.06 | 1.33 | 1.05 | Y/Y/Y |
| J-A / Jolt v5.3.0 @W1 | 1.270 | +27.01 | 51.91 | 7.82 | 10.29 | n/Y/Y |
| D-L4L2 vs J-A (bridge) @W8 | 0.848 | -15.17 | 9.73 | 4.47 | 1.90 | Y/Y/Y |
| J-A / Jolt v5.3.0 @W8 | 2.561 | +156.08 | 68.60 | 17.39 | 13.48 | Y/Y/Y |
| D-L5-npoff vs D-L5 @W8 | 1.491 | +49.10 | 36.29 | 9.63 | 7.04 | Y/Y/Y |
| D-L5-npoff vs D-L4L2 @W8 (same knobs, different binaries) | 1.024 | +2.44 | 35.10 | 9.90 | 6.82 | n/n/n |
| D-L5 vs J-A (bridge) @W1 | 0.551 | -44.88 | 3.75 | 1.96 | 0.75 | Y/Y/Y |
| D-L5 vs J-A (bridge) @W8 | 0.583 | -41.71 | 13.40 | 3.84 | 2.58 | Y/Y/Y |
| D-L4L2 W16 vs W8 | 1.049 | +4.85 | 48.78 | 4.33 | 9.82 | n/Y/n |
| D-L5 W16 vs W8 | 1.038 | +3.82 | 50.60 | 8.22 | 10.05 | n/n/n |
| JOLT-T W16 vs W8 | 0.883 | -11.69 | 68.75 | 17.46 | 13.51 | n/n/n |
| JOLT56-T W16 vs W8 | 0.930 | -7.04 | 49.68 | 9.21 | 9.78 | n/n/n |
| boyko D-L4L2 / Jolt v5.3.0 @W1 [0,100) | 0.706 | -29.39 | 42.76 | 24.85 | 9.78 | n/Y/Y |
| boyko D-L4L2 / Jolt v5.3.0 @W1 [100,500) | 0.692 | -30.80 | 56.93 | 4.50 | 11.56 | n/Y/Y |
| boyko D-L4L2 / Jolt v5.3.0 @W2 [0,100) | 1.108 | +10.80 | 30.00 | 3.44 | 5.94 | n/Y/Y |
| boyko D-L4L2 / Jolt v5.3.0 @W2 [100,500) | 1.049 | +4.90 | 27.69 | 4.92 | 5.33 | n/n/n |
| boyko D-L4L2 / Jolt v5.3.0 @W4 [0,100) | 1.698 | +69.82 | 33.92 | 4.10 | 6.62 | Y/Y/Y |
| boyko D-L4L2 / Jolt v5.3.0 @W4 [100,500) | 1.579 | +57.91 | 33.71 | 5.97 | 6.72 | Y/Y/Y |
| boyko D-L4L2 / Jolt v5.3.0 @W8 [0,100) | 2.414 | +141.44 | 55.17 | 4.49 | 10.98 | Y/Y/Y |
| boyko D-L4L2 / Jolt v5.3.0 @W8 [100,500) | 2.119 | +111.87 | 72.30 | 20.45 | 14.24 | Y/Y/Y |
| boyko D-L4L2 / Jolt v5.3.0 @W16 [0,100) | 2.831 | +183.06 | 44.95 | 5.25 | 8.85 | Y/Y/Y |
| boyko D-L4L2 / Jolt v5.3.0 @W16 [100,500) | 2.534 | +153.39 | 49.52 | 2.51 | 9.94 | Y/Y/Y |

### Bridge to P0b (this window's cell against P0b's median from MEASUREMENT-QUEUE section 10)

| cell | P0b median ms | win3 median ms | win3 min-max | shift % | win3 range % | win3 IQR % | win3 SE % | P0b median inside win3 min-max |
|---|---|---|---|---|---|---|---|---|
| J-A@W1 | 19.671 | 19.483 | 19.337-19.644 | -0.96 | 1.57 | 0.65 | 0.29 | n |
| J-A@W8 | 9.162 | 9.174 | 9.070-9.243 | +0.13 | 1.89 | 0.89 | 0.36 | Y |
| JOLT-T@W1 | 15.723 | 15.339 | 15.068-19.042 | -2.44 | 25.91 | 3.86 | 5.14 | Y |
| JOLT-T@W2 | 8.794 | 8.571 | 8.456-8.744 | -2.53 | 3.36 | 1.14 | 0.61 | n |
| JOLT-T@W4 | 5.229 | 5.168 | 5.106-5.250 | -1.17 | 2.80 | 1.98 | 0.63 | Y |
| JOLT-T@W8 | 3.533 | 3.583 | 3.466-4.693 | +1.40 | 34.25 | 8.65 | 6.73 | Y |
| JOLT-T@W16 | 3.209 | 3.164 | 3.112-3.205 | -1.41 | 2.95 | 1.17 | 0.54 | n |
| JOLT56-T@W1 | 9.650 | 9.828 | 9.518-11.364 | +1.85 | 18.78 | 3.31 | 3.50 | Y |
| JOLT56-T@W2 | 5.708 | 5.770 | 5.703-5.818 | +1.09 | 1.99 | 0.87 | 0.38 | Y |
| JOLT56-T@W4 | 3.557 | 3.581 | 3.519-3.663 | +0.69 | 4.02 | 2.36 | 0.83 | Y |
| JOLT56-T@W8 | 2.493 | 2.569 | 2.501-3.124 | +3.06 | 24.26 | 3.09 | 4.76 | n |
| JOLT56-T@W16 | 2.447 | 2.388 | 2.337-2.465 | -2.39 | 5.32 | 3.42 | 1.12 | Y |

### Scaling T(1)/T(W)

| row | W=1 | W=2 | W=4 | W=8 | W=16 |
|---|---|---|---|---|---|
| D-L4L2 | 1.000 | 1.165 | 1.280 | 1.362 | 1.298 |
| J-A | 1.000 | -- | -- | 2.124 | -- |
| JOLT-T | 1.000 | 1.790 | 2.968 | 4.282 | 4.848 |
| JOLT56-T | 1.000 | 1.703 | 2.744 | 3.825 | 4.115 |
| D-L5 | 1.000 | 1.365 | 1.740 | 2.008 | 1.934 |

### Receipts and witnesses

- receipts: 205 distinct (2 of 10 s); cpu_avg min 0.47 %, median 2.03 %, p90 6.553999999999999 %, max 22.5 %; over 5 %: 32; build processes seen in a receipt: 0
  - over 5 %: 2026-09-21T03:50:32.757+03:00 5.64 % top3 ['browser.exe', 'browser.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T03:51:08.762+03:00 6.33 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T03:52:00.228+03:00 5.26 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T03:52:36.244+03:00 6.73 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T03:53:11.275+03:00 10.08 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T03:53:54.815+03:00 5.93 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T03:54:37.462+03:00 8.72 % top3 ['claude.exe', 'claude.exe', 'browser.exe']
  - over 5 %: 2026-09-21T03:57:24.588+03:00 6.18 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T03:58:48.264+03:00 6.38 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T03:59:30.449+03:00 6.41 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T04:00:05.940+03:00 7.08 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T04:00:45.893+03:00 10.96 % top3 ['python.exe', 'claude.exe', 'claude.exe']
  - over 5 %: 2026-09-21T04:01:27.658+03:00 6.27 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T04:02:09.402+03:00 6.65 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T04:02:56.974+03:00 5.83 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T04:03:32.729+03:00 6.49 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T04:04:59.579+03:00 7.47 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T04:05:39.925+03:00 6.46 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T04:06:15.816+03:00 8.22 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T04:07:45.688+03:00 15.74 % top3 ['browser.exe', 'browser.exe', 'browser.exe']
  - over 5 %: 2026-09-21T04:12:31.659+03:00 16.97 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T04:13:05.299+03:00 10.81 % top3 ['claude.exe', 'claude.exe', 'Taskmgr.exe']
  - over 5 %: 2026-09-21T04:13:58.741+03:00 12.85 % top3 ['rust-analyzer.exe', 'Taskmgr.exe', 'claude.exe']
  - over 5 %: 2026-09-21T04:14:45.542+03:00 7.19 % top3 ['Taskmgr.exe', 'steam.exe', 'claude.exe']
  - over 5 %: 2026-09-21T04:15:43.835+03:00 16.27 % top3 ['claude.exe', 'Taskmgr.exe', 'bash.exe']
  - over 5 %: 2026-09-21T04:17:37.707+03:00 9.6 % top3 ['claude.exe', 'Taskmgr.exe', 'claude.exe']
  - over 5 %: 2026-09-21T04:18:13.098+03:00 8.4 % top3 ['claude.exe', 'Taskmgr.exe', 'steamwebhelper.exe']
  - over 5 %: 2026-09-21T04:19:31.194+03:00 7.48 % top3 ['grep.exe', 'Taskmgr.exe', 'claude.exe']
  - over 5 %: 2026-09-21T04:21:50.455+03:00 13.89 % top3 ['Taskmgr.exe', 'claude.exe', 'steamwebhelper.exe']
  - over 5 %: 2026-09-21T04:23:33.759+03:00 22.5 % top3 ['claude.exe', 'Taskmgr.exe', 'steamwebhelper.exe']
  - over 5 %: 2026-09-21T04:32:17.716+03:00 12.29 % top3 ['Taskmgr.exe', 'browser.exe', 'claude.exe']
  - over 5 %: 2026-09-21T04:34:56.649+03:00 5.54 % top3 ['claude.exe', 'Taskmgr.exe', 'claude.exe']
- top process in receipts (count): {'Taskmgr.exe': 110, 'claude.exe': 88, 'browser.exe': 4, 'python.exe': 1, 'rust-analyzer.exe': 1, 'grep.exe': 1}
- other processes' CPU during a used process: n 137, median 0.67 %, max 5.1 % of the machine; over 5 %: [['JOLT56-T', 8, 0, 17, 5.1, ['claude.exe', 'claude.exe', 'Taskmgr.exe']]]
- contaminated attempts: 32; re-runs: 31; excluded slots: 1; invalid attempts: 0; non-zero exits: []
  - contaminated: JOLT-T@W1 pass 0 seq 1 (original): before 1.95 % after 5.64 %; build before [] after []; top after [('browser.exe', 1.68), ('browser.exe', 1.0), ('Taskmgr.exe', 0.51)]; mean 18.8823 ms
  - contaminated: D-L4L2@W1 pass 0 seq 2 (original): before 4.31 % after 6.33 %; build before [] after []; top after [('claude.exe', 2.03), ('claude.exe', 1.11), ('Taskmgr.exe', 0.45)]; mean 11.8658 ms
  - contaminated: J-A@W1 pass 0 seq 4 (original): before 3.04 % after 5.26 %; build before [] after []; top after [('claude.exe', 1.68), ('claude.exe', 1.39), ('Taskmgr.exe', 0.51)]; mean 20.7893 ms
  - contaminated: D-L5@W1 pass 0 seq 5 (original): before 2.69 % after 6.73 %; build before [] after []; top after [('claude.exe', 3.01), ('claude.exe', 1.95), ('Taskmgr.exe', 0.41)]; mean 11.8928 ms
  - contaminated: JOLT-T@W2 pass 0 seq 6 (original): before 2.54 % after 10.08 %; build before [] after []; top after [('claude.exe', 1.6), ('claude.exe', 0.76), ('Taskmgr.exe', 0.49)]; mean 9.6520 ms
  - contaminated: JOLT56-T@W2 pass 0 seq 8 (original): before 1.72 % after 5.93 %; build before [] after []; top after [('claude.exe', 2.34), ('claude.exe', 1.37), ('Taskmgr.exe', 0.41)]; mean 6.5745 ms
  - contaminated: JOLT-T@W4 pass 0 seq 10 (original): before 2.45 % after 8.72 %; build before [] after []; top after [('claude.exe', 2.64), ('claude.exe', 1.8), ('browser.exe', 1.52)]; mean 6.2378 ms
  - contaminated: JOLT56-T@W4 pass 0 seq 12 (original): before 3.64 % after 6.18 %; build before [] after []; top after [('claude.exe', 2.91), ('claude.exe', 1.95), ('Taskmgr.exe', 0.41)]; mean 4.1818 ms
  - contaminated: D-L5@W4 pass 0 seq 13 (original): before 4.85 % after 6.38 %; build before [] after []; top after [('claude.exe', 3.11), ('claude.exe', 1.89), ('Taskmgr.exe', 0.43)]; mean 7.1209 ms
  - contaminated: D-L4L2@W8 pass 0 seq 15 (original): before 3.89 % after 6.41 %; build before [] after []; top after [('claude.exe', 3.05), ('claude.exe', 1.54), ('Taskmgr.exe', 0.51)]; mean 9.3743 ms
  - contaminated: J-A@W8 pass 0 seq 16 (original): before 2.47 % after 7.08 %; build before [] after []; top after [('claude.exe', 2.7), ('claude.exe', 1.78), ('Taskmgr.exe', 0.45)]; mean 10.8378 ms
  - contaminated: D-L5@W8 pass 0 seq 18 (original): before 3.24 % after 10.96 %; build before [] after []; top after [('python.exe', 3.67), ('claude.exe', 2.36), ('claude.exe', 1.23)]; mean 6.4868 ms
  - contaminated: JOLT-T@W16 pass 0 seq 20 (original): before 2.05 % after 6.27 %; build before [] after []; top after [('claude.exe', 2.11), ('claude.exe', 1.64), ('Taskmgr.exe', 0.43)]; mean 4.0555 ms
  - contaminated: JOLT56-T@W16 pass 0 seq 22 (original): before 4.23 % after 6.65 %; build before [] after []; top after [('claude.exe', 2.4), ('claude.exe', 1.56), ('Taskmgr.exe', 0.41)]; mean 3.1391 ms
  - contaminated: JOLT-T@W1 pass 0 seq 24 (original): before 3.2 % after 5.83 %; build before [] after []; top after [('claude.exe', 2.75), ('claude.exe', 1.66), ('Taskmgr.exe', 0.43)]; mean 18.0064 ms
  - contaminated: D-L4L2@W1 pass 0 seq 25 (original): before 1.86 % after 6.49 %; build before [] after []; top after [('claude.exe', 2.66), ('claude.exe', 2.05), ('Taskmgr.exe', 0.43)]; mean 11.3291 ms
  - contaminated: JOLT56-T@W1 pass 0 seq 26 (original): before 3.79 % after 7.47 %; build before [] after []; top after [('claude.exe', 2.4), ('claude.exe', 1.58), ('Taskmgr.exe', 0.49)]; mean 13.4220 ms
  - contaminated: J-A@W1 pass 0 seq 27 (original): before 3.95 % after 6.46 %; build before [] after []; top after [('claude.exe', 2.54), ('claude.exe', 1.78), ('Taskmgr.exe', 0.57)]; mean 20.5386 ms
  - contaminated: D-L5@W1 pass 0 seq 28 (original): before 2.96 % after 8.22 %; build before [] after []; top after [('claude.exe', 2.85), ('claude.exe', 2.15), ('Taskmgr.exe', 0.41)]; mean 11.6231 ms
  - contaminated: JOLT-T@W2 pass 0 seq 29 (original): before 2.74 % after 15.74 %; build before [] after []; top after [('browser.exe', 7.21), ('browser.exe', 1.41), ('browser.exe', 0.78)]; mean 19.3919 ms
  - contaminated: D-L4L2@W2 pass 0 seq 30 (original): before 3.27 % after 16.97 %; build before [] after []; top after [('claude.exe', 3.52), ('claude.exe', 2.93), ('Taskmgr.exe', 0.57)]; mean 11.2273 ms
  - contaminated: JOLT56-T@W2 pass 0 seq 31 (original): before 3.81 % after 10.81 %; build before [] after []; top after [('claude.exe', 2.62), ('claude.exe', 1.78), ('Taskmgr.exe', 0.51)]; mean 7.0596 ms
  - contaminated: D-L4L2@W4 pass 0 seq 34 (original): before 1.95 % after 12.85 %; build before [] after []; top after [('rust-analyzer.exe', 1.45), ('Taskmgr.exe', 0.29), ('claude.exe', 0.1)]; mean 13.4317 ms
  - contaminated: JOLT-T@W8 pass 0 seq 37 (original): before 1.24 % after 7.19 %; build before [] after []; top after [('Taskmgr.exe', 0.27), ('steam.exe', 0.16), ('claude.exe', 0.14)]; mean 3.5245 ms
  - contaminated: D-L5@W8 pass 0 seq 41 (original): before 0.65 % after 16.27 %; build before [] after []; top after [('claude.exe', 0.37), ('Taskmgr.exe', 0.31), ('bash.exe', 0.27)]; mean 6.3767 ms
  - contaminated: J-A@W1 pass 0 seq 50 (original): before 3.75 % after 9.6 %; build before [] after []; top after [('claude.exe', 0.78), ('Taskmgr.exe', 0.29), ('claude.exe', 0.06)]; mean 19.6889 ms
  - contaminated: D-L5@W1 pass 0 seq 51 (original): before 2.85 % after 8.4 %; build before [] after []; top after [('claude.exe', 0.35), ('Taskmgr.exe', 0.33), ('steamwebhelper.exe', 0.04)]; mean 10.6562 ms
  - contaminated: D-L4L2@W4 pass 0 seq 57 (original): before 1.84 % after 7.48 %; build before [] after []; top after [('grep.exe', 3.18), ('Taskmgr.exe', 0.29), ('claude.exe', 0.18)]; mean 8.8889 ms
  - contaminated: D-L5-npoff@W8 pass 0 seq 65 (original): before 0.94 % after 13.89 %; build before [] after []; top after [('Taskmgr.exe', 0.39), ('claude.exe', 0.14), ('steamwebhelper.exe', 0.04)]; mean 8.0200 ms
  - contaminated: D-L4L2@W1 pass 0 seq 2 (rerun): before 2.97 % after 22.5 %; build before [] after []; top after [('claude.exe', 0.41), ('Taskmgr.exe', 0.41), ('steamwebhelper.exe', 0.08)]; mean 10.4956 ms
  - contaminated: JOLT56-T@W4 pass 1 seq 12 (original): before 2.05 % after 12.29 %; build before [] after []; top after [('Taskmgr.exe', 0.23), ('browser.exe', 0.12), ('claude.exe', 0.06)]; mean 3.6685 ms
  - contaminated: D-L4L2@W16 pass 1 seq 26 (original): before 1.44 % after 5.54 %; build before [] after []; top after [('claude.exe', 0.43), ('Taskmgr.exe', 0.27), ('claude.exe', 0.14)]; mean 8.0269 ms
  - re-run: JOLT-T@W1 pass 0 seq 1 reason contaminated: valid True, contaminated False, mean 15.7983 ms
  - re-run: D-L4L2@W1 pass 0 seq 2 reason contaminated: valid True, contaminated True, mean 10.4956 ms
  - re-run: J-A@W1 pass 0 seq 4 reason contaminated: valid True, contaminated False, mean 19.3372 ms
  - re-run: D-L5@W1 pass 0 seq 5 reason contaminated: valid True, contaminated False, mean 10.7008 ms
  - re-run: JOLT-T@W2 pass 0 seq 6 reason contaminated: valid True, contaminated False, mean 8.6778 ms
  - re-run: JOLT56-T@W2 pass 0 seq 8 reason contaminated: valid True, contaminated False, mean 5.7175 ms
  - re-run: JOLT-T@W4 pass 0 seq 10 reason contaminated: valid True, contaminated False, mean 5.1056 ms
  - re-run: JOLT56-T@W4 pass 0 seq 12 reason contaminated: valid True, contaminated False, mean 3.5194 ms
  - re-run: D-L5@W4 pass 0 seq 13 reason contaminated: valid True, contaminated False, mean 6.1061 ms
  - re-run: D-L4L2@W8 pass 0 seq 15 reason contaminated: valid True, contaminated False, mean 7.6930 ms
  - re-run: J-A@W8 pass 0 seq 16 reason contaminated: valid True, contaminated False, mean 9.0695 ms
  - re-run: D-L5@W8 pass 0 seq 18 reason contaminated: valid True, contaminated False, mean 5.3502 ms
  - re-run: JOLT-T@W16 pass 0 seq 20 reason contaminated: valid True, contaminated False, mean 3.1896 ms
  - re-run: JOLT56-T@W16 pass 0 seq 22 reason contaminated: valid True, contaminated False, mean 2.3998 ms
  - re-run: JOLT-T@W1 pass 0 seq 24 reason contaminated: valid True, contaminated False, mean 15.0678 ms
  - re-run: D-L4L2@W1 pass 0 seq 25 reason contaminated: valid True, contaminated False, mean 10.5832 ms
  - re-run: JOLT56-T@W1 pass 0 seq 26 reason contaminated: valid True, contaminated False, mean 9.5183 ms
  - re-run: J-A@W1 pass 0 seq 27 reason contaminated: valid True, contaminated False, mean 19.4827 ms
  - re-run: D-L5@W1 pass 0 seq 28 reason contaminated: valid True, contaminated False, mean 10.6958 ms
  - re-run: JOLT-T@W2 pass 0 seq 29 reason contaminated: valid True, contaminated False, mean 8.4564 ms
  - re-run: D-L4L2@W2 pass 0 seq 30 reason contaminated: valid True, contaminated False, mean 8.9444 ms
  - re-run: JOLT56-T@W2 pass 0 seq 31 reason contaminated: valid True, contaminated False, mean 5.7830 ms
  - re-run: D-L4L2@W4 pass 0 seq 34 reason contaminated: valid True, contaminated False, mean 8.2605 ms
  - re-run: JOLT-T@W8 pass 0 seq 37 reason contaminated: valid True, contaminated False, mean 3.5661 ms
  - re-run: D-L5@W8 pass 0 seq 41 reason contaminated: valid True, contaminated False, mean 5.3447 ms
  - re-run: J-A@W1 pass 0 seq 50 reason contaminated: valid True, contaminated False, mean 19.3712 ms
  - re-run: D-L5@W1 pass 0 seq 51 reason contaminated: valid True, contaminated False, mean 10.6840 ms
  - re-run: D-L4L2@W4 pass 0 seq 57 reason contaminated: valid True, contaminated False, mean 8.2075 ms
  - re-run: D-L5-npoff@W8 pass 0 seq 65 reason contaminated: valid True, contaminated False, mean 8.0580 ms
  - re-run: JOLT56-T@W4 pass 1 seq 12 reason contaminated: valid True, contaminated False, mean 3.5791 ms
  - re-run: D-L4L2@W16 pass 1 seq 26 reason contaminated: valid True, contaminated False, mean 8.0837 ms
- warm-ups (untimed): [{'pass': 0, 'mean_ms': 9.0527674, 'valid': True}, {'pass': 1, 'mean_ms': 7.6914066, 'valid': True}]
- affinity masks read back: ['0xffff']; target_env: ['msvc']; profiles: [['D-L4L2', 'dev'], ['D-L5', 'dev'], ['D-L5-npoff', 'dev'], ['J-A', 'dev']]
- hashes per row: {'JOLT-T': ['0xee15b89965ec747'], 'JOLT56-T': ['0xb8522b4e3fc62cfe'], 'J-A': ['0x32d5e235342b4143'], 'D-L5': ['0x32d5e235342b4143'], 'D-L4L2': ['0x32d5e235342b4143'], 'D-L5-npoff': ['0x32d5e235342b4143']}
- exe sha256 per row: {'JOLT-T': ['29b23ad1'], 'JOLT56-T': ['918fd2b7'], 'J-A': ['ef9325ef'], 'D-L5': ['26d17a10'], 'D-L4L2': ['368d4104'], 'D-L5-npoff': ['26d17a10']}
- void steps non-zero: []; drops non-zero: []; disarmed ring traffic non-zero: []
- Jolt threads (row, W, printed): [['JOLT-T', 1, 1], ['JOLT-T', 2, 2], ['JOLT-T', 4, 4], ['JOLT-T', 8, 8], ['JOLT-T', 16, 16], ['JOLT56-T', 1, 1], ['JOLT56-T', 2, 2], ['JOLT56-T', 4, 4], ['JOLT56-T', 8, 8], ['JOLT56-T', 16, 16]]
- expect_pose gate (row, W, result): [['D-L4L2', 1, 'match'], ['D-L4L2', 2, 'none'], ['D-L4L2', 4, 'none'], ['D-L4L2', 8, 'match'], ['D-L4L2', 16, 'none'], ['D-L5', 1, 'match'], ['D-L5', 2, 'match'], ['D-L5', 4, 'match'], ['D-L5', 8, 'match'], ['D-L5', 16, 'match'], ['D-L5-npoff', 8, 'match'], ['J-A', 1, 'none'], ['J-A', 8, 'none']]
- % Processor Performance (PDH, machine total, median over processes) by pass: {'0': 129.2, '1': 130.45}

### Wall clock

- window start 2026-09-21T03:47:46.181+03:00, end 2026-09-21T04:42:16.242+03:00, status `complete`; binaries after: all match
- first process 2026-09-21T03:50:18.009+03:00, last process end 2026-09-21T04:41:58.110+03:00
- pass 0: 2026-09-21T03:50:08.426+03:00 -> 2026-09-21T04:28:09.535+03:00
- pass 1: 2026-09-21T04:30:35.667+03:00 -> 2026-09-21T04:41:58.110+03:00
- machine before: {'time': '2026-09-21T03:47:46.735+03:00', 'powercfg': 'Power Scheme GUID: 8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c  (High performance)', 'battery_and_proc_count': '{"BatteryStatus":2,"EstimatedChargeRemaining":100}\r\n249'}
- machine after: {'time': '2026-09-21T04:42:16.631+03:00', 'powercfg': 'Power Scheme GUID: 8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c  (High performance)', 'battery_and_proc_count': '{"BatteryStatus":2,"EstimatedChargeRemaining":100}\r\n264'}
- cells per round: 23; pass-0 order: ['JOLT-T@W1', 'D-L4L2@W1', 'JOLT56-T@W1', 'J-A@W1', 'D-L5@W1', 'JOLT-T@W2', 'D-L4L2@W2', 'JOLT56-T@W2', 'D-L5@W2', 'JOLT-T@W4', 'D-L4L2@W4', 'JOLT56-T@W4', 'D-L5@W4', 'JOLT-T@W8', 'D-L4L2@W8', 'J-A@W8', 'JOLT56-T@W8', 'D-L5@W8', 'D-L5-npoff@W8', 'JOLT-T@W16', 'D-L4L2@W16', 'JOLT56-T@W16', 'D-L5@W16']

## 7. Deviations from the task / protocol, and open points for the analyst

1. **Contamination level.** 32 of 205 receipts over 5 % (P0b: 2 of 266), all from `claude.exe` agent sessions
   and one `browser.exe` burst, none from builds; 31 re-runs; one slot lost (`D-L4L2@W1` K=5). The gate is a
   bracketing measurement: 13 used pass-0 processes are >5 % above their cell median and carry an elevated
   during-process witness. The medians are robust to that; the min-max ranges in 12 cells are not. Several
   claims therefore hold under IQR and SE only (marked in section 4). Which reading the claim rule means is
   still the OPEN question from P0b (`p0b/diag_report.md` section 6).
2. **My own monitoring** touched three receipts and one timed process (section 3); all were flagged and re-run
   by the protocol, and none is in the used set. Between 04:08 and 04:42 I polled only at 04:19:31, 04:29:43
   and 04:39:56 (short `grep`/`tail` calls); the 04:39:56 call overlapped `JOLT56-T@W4` p1s58 and `D-L4L2@W4`
   p1s59 (witness 0.64 / 0.67 %, both used, both within 2.3 % of their cell median).
3. **D-L5 at W=1 reads +1.3 % over D-L4L2** (claimed under SE only). Different binaries; the flag's W=1 path is
   the serial loop by design. If the analyst wants this resolved, a same-binary `runner_l5 --parallel-np off`
   at W=1 (K=6) prices it; not run here (not in the task's row list).
4. **`D-L4L2` is AllPairs under `broadphase_select: Manual`** (the build report's correction stands at
   `de06b6c9` too, `logs/14`): C2's Auto band is not exercised by any row.
5. The window's pass 0 ran 03:50-04:28 at the tail of the agents' activity; pass 1 (04:30-04:42) was quiet
   (2 contaminations, receipts median lower). Per-pass medians are in the cells table; the largest pass-0 /
   pass-1 median differences are `JOLT-T@W8` 3.883 / 3.481 (+11.5 %), `D-L5-npoff@W8` 8.262 / 7.839 (+5.4 %),
   `D-L5@W16` 5.749 / 5.499 (+4.5 %), `JOLT56-T@W16` 2.445 / 2.343 (+4.3 %), `JOLT-T@W1` 15.798 / 15.175
   (+4.1 %); every other cell is within +-2.3 %.
6. Not run: armed rows (no profile terms in this window; the task did not ask for them), an Auto-broadphase row
   (no runner flag), J-P1 (retired at L4).

## 8. Files

- `win3/window_report.md` (this file), `win3/rows.json`, `win3/rows_l5.json`, `win3/wait_log.txt` (step A),
  `win3/lanes_done.flag` (the orchestrator's), `win3/bin/` (`runner_l4l2.exe`, `runner_tip.exe`,
  `runner_l5.exe`, `SHA256SUMS` 5 entries)
- `win3/raw/`: `runs.jsonl` (171 records = 2 warm-ups + 138 originals + 31 re-runs), `pass-00/` (99 process
  dirs = 1 warm-up + 69 + 29 re-runs), `pass-01/` (72 = 1 + 69 + 2), `receipts/` (2 untimed Jolt `-receipt` runs + `receipts.json`), `manifest.json`,
  `window_state.json`, `window_log.txt`, `wait_log.txt` (idle rule), `analysis.json`, `tables.md`,
  `WINDOW_DONE`
- `win3/logs/`: `13_build_runner_l5.log`, `14_l5_usage_and_knobs.log`, `15_l5_pose_500_gate.log`,
  `wait_flag_stdout.log`, `window3_stdout.log` (+ the build stage's `01`-`12`)
- `win3/dry/l5/` (untimed pose-gate runs), `win3/test/` (the untimed rehearsal -- not measurements)
- `win3/tools/`: `window3_run.py`, `wait_idle3.ps1`, `wait_flag.py`, `analyze_win3.py`, `render_tables.py`,
  `build_l5.sh`, the P0/P0b copies (`window_run.py`, `analyze_window.py`, `window.py`, `wait_idle.ps1`,
  `launch_after_idle.py`, `receipts_summary.py`, `window_lib/`, `driver.py`, `lib/driver.py`)
- Created outside `win3/`: `D:/wt/mq-de06b6c9` (detached worktree at `de06b6c9`, clean) and
  `D:/wt/_targets/mq-de06b6c9` (parity build of one bench). Nothing else.
