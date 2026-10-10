## 8. G5 — run record and results (timed, quiet machine; orchestrator ruling 08:05: run G5 despite the 1.2 stop rules)

### 8.1 Timeline, the idle receipts, and two deviations (reported, not asked)

All times +03:00. `wait_log.txt` (standalone receipts and the pointers), `raw/wait_log.txt` (the driver's two idle waits), `raw/window_log.txt` (173 lines), `logs/06_window4_run.{pid,stdout,stderr}.txt` (stderr empty), `logs/11_claude_ui_cpu_g5.txt`, `logs/12_idle_receipt_g5.pid.txt`.

- 08:03 — D: 83 GB free; `sha256sum -c bin/SHA256SUMS`: `runner_c3.exe` OK, both bench exes OK (re-checked 10:13, OK; the driver verified them before each pass and after the window: `binaries_after: all match`). No rebuild.
- 08:04:15 — the UI host window had been restored since 05:57 (`IsIconic` false; pids 28784 / 28700 = 6.8 CPU-s over 10 s, about 4.2 % of 16 CPUs); minimized per section 8's launch recipe (28784 gone, 28700 2.02 → 0.09 s).
- 08:04:35–08:17:46 — the standalone idle receipt (`wait_idle3.ps1 -MaxPolls 90`, appended to `wait_log.txt` after `# ---- RESUME 08:04:35`): **IDLE after 14 polls**; polls 12–14 cpu10 = **3.33 / 2.19 / 3.11 %**, build procs 0, lane-target procs 0. What the earlier polls saw: poll 3 (08:06:36) **8 build processes** (`cargo` ×2, `rustc` ×6; cpu10 54.76 %); polls 9–11 `runner_parent` / `runner_tip` (another window's parity runners, see below); poll 7 a `python(38344)` at 10 s; the UI host at 2–4.5 s per 10 s on polls 2–7 (its window was restored again within a minute of 08:04:28 — not by me).
- **Deviation 1 — a second timed window was running, and I waited for it.** 08:19:13 the G5 driver was launched (pid 28248) after re-minimizing the UI host (08:18:49–08:19:13: 28784 3.91 s + 28700 3.48 s → none). Its idle wait saw, from poll 3, `runner_tip` / `runner_parent` processes: `scratchpad/win4b/tools/window4b_run.py` (pid 18716, another agent's window, launched 08:18:13: 30 cells × 2 passes × 6 rounds = 360 main + 48 armed + 2 canary timed processes, each block behind its own idle rule). Those runners live under the scratchpad, so the lane-target rule (`D:\wt\_targets`, `D:\wt\mq-*`) cannot see them, and their W=1/W=2 cells (about 5 s wall, 1–2 cores, 5-s receipts between) read 3–4 % on a 10-s poll — the rule could have passed by chance and both windows would have been timed over each other's processes. **08:29:54 I stopped my driver (pid 28248 and its `wait_idle3` child; nothing had been timed — it was still in its first idle wait)** and parked that launch's `manifest.json` / `window_log.txt` / `wait_log.txt` under `raw/launch1_aborted_0819/`. `win4b` wrote `WINDOW_DONE` (status complete) at 10:12:24. My 90-minute idle budget, counted from 08:30, would have expired at 10:00 with `win4b`'s armed block still running; I extended the wait to its completion (a known, finite, orchestrator-launched noise source, and the owner's window is "at least" to 10:50), hard cap 10:30. During the wait my only activity was a 30-s `sleep` poll loop on its log.
- 10:13:16 — no `runner_*` / `python` / `cargo` / `rustc` process; sha256 OK; 10:13:30 the UI host was already quiet (30548 0.14 s; 34656 minimized again); **10:13:54 the G5 driver relaunched (pid 6796)**.
- **Pass 0**: idle after 3 polls (cpu10 **2.06 / 1.20 / 0.95 %**, 0 build, 0 lane-target) at 10:16:05; binaries verified; opening 10-s receipt **1.18 %** (build procs none); warm-up `T-D-tree` W=8 (untimed) 10:16:16–10:16:23, 3.678 ms; 78 originals 10:16:23–10:28:57; **0 contaminated, 0 invalid, no re-runs**.
- **Pass 1**: idle after 3 polls (**1.83 / 1.65 / 1.38 %**) at 10:31:07; opening receipt **0.69 %**; warm-up 3.730 ms; 78 originals (reversed order) 10:31:24–10:44:00; **1 invalid (seq 75, section 8.3), re-run 10:44:00–10:44:05 valid**; `WINDOW END` 10:44:10, status `complete`.
- Machine: `High performance`, `BatteryStatus 2, 100 %` at both pass openings and at the end; process count 241–247.
- **Deviation 2 (the UI host)**: minimized three times (08:04:28, 08:19:13, 10:13:54; `logs/11_claude_ui_cpu_g5.txt`), restored twice by something other than me; during the timed passes `claude.exe` appears in `others_top5` of 196 records at ≤ 0.1 s each (window minimized) — the run itself was not affected. The window is minimized at the time of writing.

### 8.2 Load receipts and the process census

`raw/runs.jsonl` — 159 records (sha256 `d754008c3f6688a45dd1…`): 2 warm-ups + 156 originals + 1 re-run; **used 156, excluded slots 0** (window-3 selection: original if clean and valid, else its re-run). `raw/pass-00/`, `raw/pass-01/` (one directory per process: `stdout.txt`, `stderr.txt`, `run.csv`, `pose.bin`). `raw/receipts/` is empty (no Jolt row — v5.6.0 is read from window 3, never re-run; no v5.3.0 anywhere).

| item | value |
|---|---|
| exits | every process 0 |
| `receipt_before` / `receipt_after` (5 s) over all 159 | **0.44–3.97 %** / **0.44–3.97 %**; `build_procs_busy` empty on every receipt; contaminated 0 |
| `waited_s` | 0 on every process (no busy before-receipt) |
| build process during a timed process (`others_top5` names) | **none** on all 159 (the task's void rule: no pass voided) |
| `others_busy_pct` during timed processes | 0.0–**6.49 %**; one process > 3 %: pass 1 round 0 seq 6 `R-allpairs` W=8 (10:32:04–08), `python.exe` pid 16036 = 3.75 CPU-s during its 3.75-s wall (not mine — my only activity was `sleep`; unidentified, gone by 10:44); its receipts 1.92 / 1.10 % ⇒ used by the protocol; its mean 7.440 ms is the cell's max (the other five 6.69–7.00) |
| `others_top5` names over the window | `claude.exe` (196 records, ≤ 0.1 s), `steamwebhelper.exe` 91, `steam.exe` 87, `Telegram.exe` 61, `nvsphelper64.exe` 41, `browser.exe` 37, `explorer.exe` 31, `nvcontainer.exe` 16, `NVIDIA Overlay.exe` 16, `AmneziaVPN.exe` 16, `backgroundTaskHost.exe` 7, `sleep.exe` 3 (my poll loop) |
| wall per process | 0.029 s (S16) – 9.914 s (T-A W=1) |
| `mask_readback` | `0xffff` on every process (P-none, 16 logical CPUs) |

### 8.3 Pose receipts (recipe 2.2 items 1–2 on the timed rows; reducer `gate_inputs`)

- Every used process: `expect_pose ∈ {match, none}` (`expect_pose_not_match_or_none: []`); `void_steps` 0, `first_void` null everywhere; `config.tree_brute_max_rows` 64 and `target_env msvc` on all 156; `tree_diag_violations: []` (every J and R `tree` process `static_rebuilds 1, members 1`, all other counters 0; every `allpairs` and `s16` process all-zero).
- One hash per (scene, cfg) across kinds and W over the used set: J (cfg default and cfg a; tree, allpairs, armed, canary; W 1/2/4/8/16) `0x32d5e235342b4143`; R `0xee2a67a98434919a`; S16 `0x71313833f6a8e645`.
- **The one INVALID process (a determinism fact for the analyst, not a claim): pass 1 round 2 seq 75 `T-D-allpairs` W=1** (`--scene jolt --gap 0.5 --cfg default --broadphase allpairs --workers 1`, 10:43:16–10:43:21, exit 0, receipts 0.85 / 0.68 %, others 0.22 %, `void_steps 0`) reported `pose_hash 0xf6e397d168e0e8b8`. Its `final_manifolds 4515` and `final_pairs 9559` equal every other J process's; `final_top_y` = 28.990150 vs 28.990126; its `pose.bin` differs from the re-run's in **42,845 of 64,480 bytes, first differing body 0, all 1240 bodies touched** — a numerical, not structural, divergence on a one-worker AllPairs process (SUMMARY: `workers 1`, `pool_workers 1, dispatcher 1`, `parallel_solve true, parallel_narrowphase true, colored true, simd_solve true`). The re-run (10:44:00–05) hashed `0x32d5e235342b4143`. 1 of 13 `T-D-allpairs` W=1 processes (12 originals + the re-run); all other 120 used J processes of this window, both warm-ups, and the 15 J processes of the 500-step pose gate hashed `0x32d5e235342b4143`. The invalid record was not used as an `--expect-pose` reference (the driver stores only valid records; seq 76 `T-D-tree` W=1 matched against pass 1 round 1's seq 49). Files: `raw/pass-01/075_r2_T-D-allpairs_W1/` and `…_rerun/`.

### 8.4 Cells (median over K=6 of the process's [0,500) window mean, ms/step; range = (max−min)/median; IQR; SE = 1.2533·SD/√K; `[0,100)` / `[100,500)` sub-window medians). `raw/g5_reduction.json` (sha256 `ba76f61be9a1d76a34f9…`), printed tables `logs/13_reduce_g5.txt`.

| cell | K | median ms | min | max | range % | IQR % | SE % | [0,100) | [100,500) | pose |
|---|---|---|---|---|---|---|---|---|---|---|
| T-A-tree@W1 | 6 | **17.8887** | 17.7909 | 18.1075 | 1.77 | 0.48 | 0.32 | 17.568 | 17.956 | 0x32d5…4143 |
| T-A-allpairs@W1 | 6 | **19.5711** | 19.4435 | 19.6971 | 1.30 | 0.34 | 0.22 | 19.308 | 19.629 | same |
| T-A-tree@W2 | 6 | **10.5878** | 10.5113 | 10.6917 | 1.70 | 0.76 | 0.32 | 10.474 | 10.622 | same |
| T-A-allpairs@W2 | 6 | **12.2700** | 12.1556 | 12.2920 | 1.11 | 0.80 | 0.26 | 12.097 | 12.308 | same |
| T-A-tree@W4 | 6 | **6.8738** | 6.8258 | 7.0866 | 3.79 | 1.37 | 0.73 | 6.824 | 6.886 | same |
| T-A-allpairs@W4 | 6 | **8.5305** | 8.5081 | 8.7359 | 2.67 | 1.46 | 0.59 | 8.463 | 8.547 | same |
| T-A-tree@W8 | 6 | **5.0234** | 5.0006 | 5.0812 | 1.60 | 0.51 | 0.29 | 4.991 | 5.032 | same |
| T-A-allpairs@W8 | 6 | **6.6421** | 6.5817 | 6.7404 | 2.39 | 1.26 | 0.47 | 6.587 | 6.656 | same |
| T-A-tree@W16 | 6 | **4.9258** | 4.8693 | 5.1730 | 6.17 | 0.91 | 1.15 | 4.916 | 4.925 | same |
| T-A-allpairs@W16 | 6 | **6.4863** | 6.4383 | 6.5832 | 2.23 | 0.90 | 0.42 | 6.469 | 6.490 | same |
| T-D-tree@W1 | 6 | **9.0262** | 8.9176 | 9.0917 | 1.93 | 0.97 | 0.38 | 8.966 | 9.034 | same |
| T-D-allpairs@W1 | 6 | **10.5537** | 10.4639 | 10.7378 | 2.60 | 1.05 | 0.49 | 10.445 | 10.581 | same (the re-run replaces seq 75) |
| T-D-tree@W8 | 6 | **3.6987** | 3.6880 | 3.7706 | 2.23 | 0.79 | 0.44 | 3.697 | 3.699 | same |
| T-D-allpairs@W8 | 6 | **5.4905** | 5.3267 | 5.5493 | 4.05 | 2.15 | 0.84 | 5.312 | 5.535 | same |
| R-tree@W1 | 6 | **12.3082** | 12.2217 | 12.7038 | 3.92 | 1.17 | 0.75 | 12.440 | 12.292 | 0xee2a…919a |
| R-allpairs@W1 | 6 | **13.9059** | 13.6707 | 14.0529 | 2.75 | 0.32 | 0.46 | 13.989 | 13.885 | same |
| R-tree@W8 | 6 | **5.3097** | 5.0855 | 5.4814 | 7.46 | 5.38 | 1.65 | 5.204 | 5.320 | same |
| R-allpairs@W8 | 6 | **6.8474** | 6.6937 | 7.4401 | 10.90 | 2.65 | 2.00 | 6.927 | 6.781 | same (max = the others-6.49 % process) |
| S16-tree@W1 | 6 | **0.0448** | 0.0445 | 0.0458 | 3.08 | 1.83 | 0.65 | 0.0488 | 0.0439 | 0x7131…e645 |
| S16-allpairs@W1 | 6 | **0.0450** | 0.0445 | 0.0459 | 3.01 | 1.59 | 0.60 | 0.0484 | 0.0441 | same |
| T-A-tree-armed@W1 | 6 | **17.8700** | 17.7608 | 18.1505 | 2.18 | 1.08 | 0.44 | 17.692 | 17.946 | 0x32d5…4143 |
| T-A-allpairs-armed@W1 | 6 | **19.5546** | 19.2706 | 19.7255 | 2.33 | 0.71 | 0.41 | 19.195 | 19.630 | same |
| T-A-tree-armed@W8 | 6 | **5.0186** | 5.0118 | 5.2985 | 5.71 | 2.85 | 1.28 | 5.020 | 5.021 | same |
| T-A-allpairs-armed@W8 | 6 | **6.6947** | 6.6057 | 7.0288 | 6.32 | 2.19 | 1.21 | 6.635 | 6.710 | same |
| T-C-tree@W1 | 6 | **18.7639** | 18.6803 | 18.9201 | 1.28 | 0.45 | 0.24 | 18.539 | 18.824 | same |
| T-C-tree@W8 | 6 | **5.3397** | 5.2935 | 6.0387 | 13.96 | 1.74 | 2.78 | 5.336 | 5.308 | same |

Per-process means (pass, round, seq, ms) of the cells whose range exceeds 5 %: `R-allpairs@W8` (0,0,21) 6.694 (0,1,47) 6.881 (0,2,73) 6.998 **(1,0,6) 7.440** (1,1,32) 6.814 (1,2,58) 6.779; `R-tree@W8` 5.221 5.086 5.451 5.129 5.398 5.481; `T-A-allpairs-armed@W8` 6.709 6.799 6.681 6.613 **(1,1,30) 7.029** 6.606; `T-A-tree-armed@W8` 5.203 5.018 **(0,2,74) 5.299** 5.012 5.019 5.013; `T-C-tree@W8` 5.301 5.402 5.306 5.294 5.373 **(1,2,55) 6.039**; `T-A-tree@W16` **(0,0,25) 5.173** 4.927 4.869 4.884 4.943 4.924. The `[0,100)` vs `[100,500)` sub-windows agree within 2 % on every J and R cell (S16: 10 % — a 45-µs step).

### 8.5 Comparisons (the section 1.1 statistic: claimed iff |ratio − 1| > 2·hypot(spread_A, spread_B), printed under range / IQR / SE). Window-3 reference cells (`D-L5`, `JOLT56-T`, K=6 each) read from `…/2026-09-21-physics-window3/raw/analysis.json`; Jolt v5.6.0 only.

| comparison (B vs A) | A median | B median | ratio | effect % | bar range / IQR / SE % | claimed (range / IQR / SE) |
|---|---|---|---|---|---|---|
| T-A-tree vs T-A-allpairs @W1 | 19.5711 | 17.8887 | 0.9140 | −8.60 | 4.39 / 1.18 / 0.78 | yes / yes / yes |
| T-A-tree vs T-A-allpairs @W2 | 12.2700 | 10.5878 | 0.8629 | −13.71 | 4.07 / 2.19 / 0.83 | yes / yes / yes |
| T-A-tree vs T-A-allpairs @W4 | 8.5305 | 6.8738 | 0.8058 | −19.42 | 9.28 / 4.00 / 1.88 | yes / yes / yes |
| **T-A-tree vs T-A-allpairs @W8 (headline)** | 6.6421 | 5.0234 | 0.7563 | **−24.37** | 5.75 / 2.72 / 1.11 | yes / yes / yes |
| T-A-tree vs T-A-allpairs @W16 | 6.4863 | 4.9258 | 0.7594 | −24.06 | 13.12 / 2.56 / 2.45 | yes / yes / yes |
| T-D-tree vs T-D-allpairs @W1 | 10.5537 | 9.0262 | 0.8553 | −14.47 | 6.47 / 2.86 / 1.23 | yes / yes / yes |
| T-D-tree vs T-D-allpairs @W8 | 5.4905 | 3.6987 | 0.6737 | −32.63 | 9.26 / 4.59 / 1.91 | yes / yes / yes |
| R-tree vs R-allpairs @W1 | 13.9059 | 12.3082 | 0.8851 | −11.49 | 9.57 / 2.43 / 1.75 | yes / yes / yes |
| R-tree vs R-allpairs @W8 | 6.8474 | 5.3097 | 0.7754 | −22.46 | 26.41 / 12.00 / 5.19 | **no** / yes / yes |
| S16-tree vs S16-allpairs @W1 | 0.0450 | 0.0448 | 0.9962 | −0.38 | 8.61 / 4.84 / 1.77 | no / no / no |
| T-A-tree-armed vs T-A-allpairs-armed @W1 | 19.5546 | 17.8700 | 0.9139 | −8.61 | 6.38 / 2.59 / 1.20 | yes / yes / yes |
| T-A-tree-armed vs T-A-allpairs-armed @W8 | 6.6947 | 5.0186 | 0.7496 | −25.04 | 17.04 / 7.19 / 3.52 | yes / yes / yes |
| T-A-tree-armed vs T-A-tree (arming cost) @W1 | 17.8887 | 17.8700 | 0.9990 | −0.10 | 5.62 / 2.36 / 1.08 | no / no / no |
| T-A-tree-armed vs T-A-tree (arming cost) @W8 | 5.0234 | 5.0186 | 0.9990 | −0.10 | 11.87 / 5.79 / 2.62 | no / no / no |
| T-C-tree vs T-A-tree @W1 (the canary's step rise) | 17.8887 | 18.7639 | 1.0489 | +4.89 | 4.37 / 1.31 / 0.79 | yes / yes / yes |
| T-C-tree vs T-A-tree @W8 | 5.0234 | 5.3397 | 1.0630 | +6.30 | 28.10 / 3.62 / 5.60 | no / yes / yes |
| **T-D-allpairs (this binary) vs window-3 D-L5 @W1 (the bridge)** | 10.7379 | 10.5537 | 0.9828 | −1.72 | 5.57 / 2.57 / 1.08 | no / no / **yes** |
| **T-D-allpairs (this binary) vs window-3 D-L5 @W8 (the bridge)** | 5.3474 | 5.4905 | 1.0268 | +2.68 | 15.20 / 5.49 / 3.00 | no / no / no |
| T-D-tree vs window-3 Jolt v5.6.0 @W1 | 9.8282 | 9.0262 | 0.9184 | −8.16 | 37.76 / 6.89 / 7.04 | no / yes / yes |
| **T-D-tree vs window-3 Jolt v5.6.0 @W8** | 2.5693 | 3.6987 | 1.4396 | **+43.96** | 48.73 / 6.37 / 9.56 | no / yes / yes |
| T-A-tree vs window-3 Jolt v5.6.0 @W1 | 9.8282 | 17.8887 | 1.8201 | +82.01 | 37.73 / 6.69 / 7.03 | yes / yes / yes |
| T-A-tree vs window-3 Jolt v5.6.0 @W2 | 5.7705 | 10.5878 | 1.8348 | +83.48 | 5.24 / 2.31 / 1.00 | yes / yes / yes |
| T-A-tree vs window-3 Jolt v5.6.0 @W4 | 3.5814 | 6.8738 | 1.9193 | +91.93 | 11.06 / 5.46 / 2.21 | yes / yes / yes |
| T-A-tree vs window-3 Jolt v5.6.0 @W8 | 2.5693 | 5.0234 | 1.9552 | +95.52 | 48.64 / 6.26 / 9.54 | yes / yes / yes |
| T-A-tree vs window-3 Jolt v5.6.0 @W16 | 2.3885 | 4.9258 | 2.0623 | +106.23 | 16.29 / 7.07 / 3.21 | yes / yes / yes |

Window-3 reference cells as read: `D-L5@W1` 10.7379 (range 1.01 %, SE 0.23 %), `D-L5@W8` 5.3474 (6.43 / 1.24 %); `JOLT56-T` @W1 9.8282 (18.78 / 3.50 %), @W2 5.7705 (1.99 / 0.38 %), @W4 3.5814 (4.02 / 0.83 %), @W8 2.5693 (24.26 / 4.76 %), @W16 2.3885 (5.32 / 1.12 %).

### 8.6 The armed rows — `phys_bp_*` spans, Δbp, t_q, the structure receipts (recipe 2.3 gate inputs; per process the median over [100,500) of the per-step value, then the median over K; ms)

| cell | verify | build | query (t_q) | assemble | **Σ four spans** | `sys_physics_broadphase` | per-process Σ |
|---|---|---|---|---|---|---|---|
| T-A-tree-armed @W1 | 0.0078 | 0.0267 | **0.4140** | 0.0264 | **0.4756** | 0.4762 | 0.4756 0.4727 0.4734 0.4781 0.4755 0.4795 |
| T-A-tree-armed @W8 | 0.0066 | 0.0228 | **0.4110** | 0.0261 | **0.4662** | 0.4665 | 0.4687 0.4672 0.4659 0.4643 0.4664 0.4661 |
| T-C-tree @W1 | 0.0074 | 0.0259 | 0.4141 | 0.0260 | 0.4735 | 0.4741 | 0.4708 0.4762 0.4661 0.4756 0.4776 0.4713 |
| T-C-tree @W8 | 0.0066 | 0.0223 | 0.4097 | 0.0259 | 0.4646 | 0.4648 | 0.4637 0.4696 0.4644 0.4629 0.4647 0.4664 |
| T-A-allpairs-armed @W1 | – | – | – | – | – | **2.0291** (per process 2.0493 2.0137 2.0446 1.9550 2.0659 1.9831) | |
| T-A-allpairs-armed @W8 | – | – | – | – | – | **1.9543** (1.9544 1.9588 1.9541 1.9475 1.9533 1.9548) | |

- **Δbp** (`sys_physics_broadphase` allpairs − tree): @W1 mean over [0,500) 2.0341 − 0.5167 = **1.5174 ms**, median over [100,500) 2.0291 − 0.4762 = **1.5529 ms**; @W8 1.9541 − 0.4700 = **1.4841 ms** (mean), 1.9543 − 0.4665 = **1.4878 ms** (median). The recipe's realized-Δbp(8) requirement is ≥ 1.03 ms.
- **The tree span** (Σ of the four `phys_bp_*` spans, median over [100,500), then over K): @W1 **0.4756 ms** vs the recipe's limit **0.36 ms**; @W8 **0.4662 ms** vs **0.35 ms** — both above the limit (the recipe's gate reads "else stop"; the window had completed; recorded here for the analyst, next to G4's `bp_g4_scene/tree/j100` 0.4429 ms bench figure of section 7.1). t_q (the query span) is 0.414 / 0.411 ms of it (mean over [100,500) 0.452 / 0.413).
- Arming cost: `T-A-tree-armed` vs `T-A-tree` −0.10 % at both W (not claimed under any spread).
- Structure, every armed process (tree-armed, allpairs-armed, canary; W 1 and 8): `void_steps 0`, `first_void null`; tree rows `span_n_sets = [(1,1,1,1)]` (each of the four spans once per step), `queried + members = 1241` on every step, `members ∈ {0, 1}` (0 on step 0 before the first static build, 1 after); allpairs-armed rows all four `_n` = 0, `queried + members = 0`.

### 8.7 The canary (`T-C-tree`, `--canary-frac 0.05 --canary-ref-ns <T-A-tree window mean at the same W, most recent valid process>`)

| W | `--canary-ref-ns` used (pass 0 r0/r1/r2, pass 1 r0/r1/r2) | `canary_ns` = F·T per process | `sys_parity_canary_ns` median per process (ms) | T-C-tree median | T-A-tree median | step rise |
|---|---|---|---|---|---|---|
| 1 | 18107483 / 17914429 / 17790907 / 17790907 / 17863065 / 17825052 | 905374 895721 889545 889545 893153 891253 (median 0.8922 ms) | 0.9057 0.8961 0.8898 0.8899 0.8935 0.8916 | 18.7639 | 17.8887 | **+0.8752 ms** (claimed under all three spreads, section 8.5) |
| 8 | 5000595 / 5038960 / 5081182 / 5081182 / 5014144 / 5010739 | 250030 251948 254059 254059 250707 250537 (median 0.2513 ms) | 0.2503 0.2522 0.2543 0.2543 0.2510 0.2508 | 5.3397 | 5.0234 | **+0.3163 ms** (claimed under IQR and SE, not under range — `T-C-tree@W8` range 13.96 % from its seq 55 process at 6.039 ms) |

The canary's own span reads F·T on every process (span median vs `canary_ns` within 0.1 %); the step rise at W=1 is 0.875 ms against F·T = 0.892 ms, at W=8 0.316 ms against 0.251 ms.

### 8.8 G5 facts next to the fired G4 rules (no claims)

- The G4 1.2 stop rules fired (`J snapshot` 0.4429 ms > 0.30; `compaction` 3.2–4.9× its limits; `high_jumper/100000` +4.8 %) and the orchestrator ruled to run G5 regardless; C4's constants and the default flip are not decided from this window unless the analyst shows the fired rules do not touch them. G5's own tree-span gate input is above its limit at both W (8.6) — the same quantity the `J snapshot` rule measured.
- The bridge: `T-D-allpairs` on this binary vs window 3's `D-L5`: −1.72 % at W=1 (claimed only under SE), +2.68 % at W=8 (not claimed).
- `T-D-tree@W8` 3.6987 ms against Jolt v5.6.0's 2.5693 ms: +43.96 %.
