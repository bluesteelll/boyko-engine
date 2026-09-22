DONE — 452 timed processes (410 originals + 42 re-runs), 5 untimed warm-ups; 0 invalid, 0 dropped slots; K = 12 in every cell except the canary (K = 1 per binary, by design).

# Window 4b — L11 C2 (tip f8873aae) against its parent C0 (146a1125), G9 timing under the P0 protocol (2026-09-22)

Timed 08:20:42–10:12:19 +03:00 on the owner's workstation. No claims are made here; every number is a cell, an effect under the two readings, or gate arithmetic on them. Raw: `raw/runs.jsonl` (one record per process), `raw/main-p0/`, `raw/main-p1/`, `raw/armed-p0/`, `raw/armed-p1/`, `raw/canary-p0/` (per-process `stdout.txt`, `stderr.txt`, `run.csv`, `pose.bin`), `raw/manifest.json`, `raw/window_state.json`, `raw/window_log.txt`, `raw/wait_log.txt`, `raw/analysis.json`, `raw/tables.md`, `raw/sensitivity.md`.

## 0. What ran

| item | value |
|---|---|
| tip | `bin/runner_tip.exe` sha256 `29dbd993ea817c1125f36a09e1b06efc65116ae0e0edadaeb9d602991eac3df0`, commit `f8873aaeef02474e95a205f543d57511fe010691` (branch `perf/physics-l11-solve-setup`: C0 146a1125 + C1 691891c4 + C2 f8873aae, `git status` clean), built in `D:/wt/lighttable` into `D:/wt/_targets/vkval-msvc` — `Compiling boyko-physics v0.1.0 (D:\wt\lighttable\crates\boyko_physics)` (`logs/03_build_tip.log`) |
| parent | `bin/runner_parent.exe` sha256 `8dfd01435495a7e38956fbe04b07eca918438c8948c45b5288bc2000e419970c`, commit `146a1125edffdfbfd078e5343d6226379ed04a70` (C0: test-only digest + the J-As runner row), `git archive 146a1125 \| tar -x` into `scratchpad/win4b/parent_tree/`, the tip's `Cargo.lock` copied in (the archive carries none; the freshly resolved lock differed only in `libredox` 0.1.24→0.1.25, a redox-only crate, and the copy recompiled nothing), built into `D:/wt/_targets/l11-parent-msvc` (cold, 47 s) — `Compiling boyko-physics v0.1.0 (C:\Users\flint\AppData\Local\Temp\claude\D--claude-BoykoEngine\d9f1cf70-4eed-44f3-9e33-2efbf030fc1a\scratchpad\win4b\parent_tree\crates\boyko_physics)` (`logs/01_build_parent.log`) |
| build prefix | `PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=6 TMP=D:/wt/_targets/tmp TEMP=D:/wt/_targets/tmp`, `cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid`; rustc 1.98.1 (48a229cea 2026-09-01) msvc; no `RUSTFLAGS` (`x86-64-v3` from `.cargo/config.toml`, byte-identical in both trees) |
| hashes / commits | `bin/SHA256SUMS`, `bin/COMMIT.txt`; the binaries were re-verified against both before every pass and after the window (`all match`) |
| window-4 runner | none running at 08:05 (no `runner_c3.exe`, `window4_run.py`, cargo/rustc/link/miri process; CPU 3–10 % background); the builds started at 08:06 |
| disk | D: 82 GB free before and after (`df -h`), C: 29 GB |
| machine | 16 logical CPUs, `High performance` scheme, AC (battery 100 %), affinity mask read back `0xffff` in all 452 processes (P-none) |
| tools | `tools/window4b_run.py` (adapted from window 3's `window3_run.py`), `tools/wait_idle4b.ps1` (window 3's idle script + `miri`, `cargo-miri`, `runner_c3`, the bench exe name), `tools/pose_gate.py`, `tools/reduce.py`; `tools/lib/driver.py` = window 3's copy (helpers only) |
| rows | `rows.json`: J-As (`--cfg as`) and J-A (`--cfg a`) at W 1/2/4/8/16, 500 steps, window [0,500); R (`--scene rest --solver colored`, plus `--parallel-solve` at W=8 as P0's row spelled it — a no-op since L4) at W 1/8, 1100 steps, window [600,1100), armed; R-S (`--scene rest --sleeping --frozen-by 300`) at W 1/8, 800 steps, window [300,800), armed; S16 (`--scene s16 --cfg a`) at W 1, 300 steps, armed; J-As-a (`--cfg as --arm-profiler`) at W 1/8; J-C (`--cfg as --arm-profiler --canary-frac 0.05 --canary-ref-ns <this binary's latest J-As-a W1 window mean>`) once per binary at W 1. `--parallel-np` not passed (as window 3's rows). Jolt not run (v5.6.0's window-3 cells are the comparison, never re-run) |
| protocol | window-3 block: cell = MEDIAN over K of the process window mean (ms/step); 5-s receipt before/after every process, > 5 % or a build process ⇒ re-run once at the end of the pass; 10-s receipt and the idle rule (0 build/miri/lane-test processes on 3 consecutive 60-s polls with 10-s CPU < 5 %) before every pass; one untimed warm-up (tip J-As W=8) per pass; parent/tip interleaved inside a W group, row by row; pass 1 the whole list reversed; K=12 = 2 passes × 6 rounds in the main block AND in the armed block (time did not press); the canary block after the armed block |
| pose assertion | every process ran with `--expect-pose gate/<row>_W1_parent.pose` (the parent's untimed gate pose) and its summary `pose_hash` was compared with `rows.json`: 452/452 `expect_pose: "match"`, 452/452 pose = the C0 tester's value, 452/452 exit 0, 452/452 `void_steps 0`; armed: `drops_total 0` and `solve_on_dispatcher_steps 0` in all 187; R-S `first_frozen_step 248` in all 54; J-As-a `waves_total 54660` in all 49 |

## 1. Pose gate (untimed, `logs/04_pose_gate.log`, `gate/gate.json`)

Every row on the parent at its own step count, W=1 and W=8, then the tip with `--expect-pose` against the parent's pose file; the runner's printed config compared field by field between the binaries (equal in all 14 pairs).

| row | steps | parent pose W1 / W8 | tip `--expect-pose` W1 / W8 | task-expected (C0 tester) |
|---|---|---|---|---|
| J-As | 500 | `0x32d5e235342b4143` / same | exit 0 `match` / exit 0 `match` | `0x32d5e235342b4143` = |
| J-A | 500 | `0x32d5e235342b4143` / same | exit 0 `match` / exit 0 `match` | `0x32d5e235342b4143` = |
| R | 1100 | `0x87e561d20589d4a5` / same | exit 0 `match` / exit 0 `match` | `0x87e561d20589d4a5` = |
| R-S | 800 | `0x2a2b7926a48aab00` / same | exit 0 `match` / exit 0 `match` | `0x2a2b7926a48aab00` = |
| S16 | 300 | `0x8877dbb1192e9b92` / same | exit 0 `match` / exit 0 `match` | `0x8877dbb1192e9b92` = |
| J-As-a (armed) | 500 | `0x32d5e235342b4143` / same | exit 0 `match` / exit 0 `match` | = |
| J-C (armed, no canary at the gate) | 500 | `0x32d5e235342b4143` / same | exit 0 `match` / exit 0 `match` | = |

Red control: the tip at **501** steps against the parent's 500-step J-As pose → **exit 4**, `expect_pose: "mismatch: 64480 vs 64480 bytes, first differing body Some(0)"`, pose `0x9336b30a06a7d8af`. The gate can fail.

The pose.bin files are W-independent (one sha256 per row across W=1 and W=8 on both binaries: J `eff361e1…`, R `c133a50e…`, R-S `100975d7…`, S16 `43d18f22…`), which is why one gate file per row serves every W in the timed passes.

## 2. Load receipts and contamination

- 452 timed processes; 5-s receipt BEFORE: median 1.31 %, max 4.96 %, > 5 %: **0** (the runner waits while the last receipt is busy — 1626 s of such waiting in total, all inside pass main-p1); AFTER: median 1.35 %, max 12.35 %, > 5 %: **42** ⇒ 42 re-runs at the end of their pass (main-p0: 4, main-p1: 37, armed-p1: 1, canary: 0). Every re-run came back clean, so no slot was dropped and every cell has K = 12.
- Build processes: none in any receipt, none seen during any process (the during-process CPU table names no cargo / rustc / link / miri / lane-test process).
- During-process witness `others_busy_pct` (CPU-seconds of every other process over wall × 16): median 0.36 %, max 8.71 %, > 5 % in 13 processes, of which 5 are in the chosen set (the other 8 were re-run for their receipt anyway). The protocol reports this witness and does not gate on it (window 3 § 6.3); § 4 shows every cell with those 5 excluded — no median moves by more than 0.016 ms and no effect changes sign or claim status.
- What was busy: the receipts' top processes are `claude.exe` (2–3 % of the machine each, two instances — the Claude Code host, not this window's runner), `Telegram.exe` (up to 2.9 %), `browser.exe`, `explorer.exe`, `steam.exe`; the > 5 % receipts are single 0.5-s samples of 30–47 % (a burst the per-process CPU table does not attribute — likely a process whose times this user cannot read). Pass main-p1 (08:55–09:57) carried most of it; pass main-p0 (08:20–08:52) and the armed block (09:59–10:12) were quiet. § 4 splits every cell by pass: the parent's W ≥ 2 medians are 0.4–1.4 ms higher in pass 1 than in pass 0, the tip's 0.1–0.4 ms higher, so the pass-1 effects are larger than the pass-0 ones; the all-K medians sit between, and every gate below passes under pass 0 alone as well.
- Idle rule: 5 waits, each reached after exactly 3 polls (cpu10 0.7–4.1 %, build procs 0, lane-target procs 0, D: 81.6 GB).
- Effective clock witness (PDH `% Processor Performance`, `perf %` column): 122–135 % of nominal in every cell, parent and tip alike within a cell (AC, High performance).

## 3. Cells, effects, per-stage spans, gates, canary (`tools/reduce.py` → `raw/tables.md`, reproduced verbatim)

Per-stage spans: per process the median over the steps named of `<zone>_ns` from the armed CSV, then the median over K; the µs-per-manifold columns divide by the design's 4,524.2 (the [0,500) mean manifold count; this window's [0,500) mean is 4,524.246 and its [100,500) median 4,519 on both binaries). The `warm_apply` gate rows are C3's lever (not in the C2 tip) — reported, not gated, as the task states. `2*SE` is `2*hypot(SE_parent, SE_tip)`.

timed processes 452 (re-runs 42), warm-ups 5, invalid 0, contaminated before 0 / after 42 / build-proc-during 0, dropped slots 0

### Cells (ms/step; MEDIAN over K of the process window mean; min-max; IQR; SE of the median)

| row | W | binary | K | median | mean | min | max | IQR | SE_med | poses | others busy % (med/max) | perf % (med) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| J-A | 1 | parent | 12 | 19.5823 | 19.6563 | 19.2746 | 20.3348 | 0.1845 | 0.0969 | 0x32d5e235342b4143 | 0.29/1.40 | 132.1 |
| J-A | 1 | tip | 12 | 18.9323 | 18.9547 | 18.5974 | 19.4261 | 0.2576 | 0.0801 | 0x32d5e235342b4143 | 0.30/2.26 | 132.2 |
| J-A | 2 | parent | 12 | 12.3248 | 12.6201 | 12.1694 | 13.7449 | 0.9812 | 0.2147 | 0x32d5e235342b4143 | 0.57/3.21 | 131.5 |
| J-A | 2 | tip | 12 | 11.4646 | 11.5652 | 11.3899 | 12.0541 | 0.1873 | 0.0810 | 0x32d5e235342b4143 | 0.44/1.89 | 132.3 |
| J-A | 4 | parent | 12 | 8.5293 | 8.8368 | 8.4404 | 10.3378 | 0.6904 | 0.2211 | 0x32d5e235342b4143 | 0.53/4.90 | 129.4 |
| J-A | 4 | tip | 12 | 7.7689 | 7.9242 | 7.6518 | 8.6748 | 0.4307 | 0.1311 | 0x32d5e235342b4143 | 0.54/3.50 | 130.5 |
| J-A | 8 | parent | 12 | 6.5921 | 6.9049 | 6.5519 | 7.9915 | 0.8950 | 0.1951 | 0x32d5e235342b4143 | 0.29/4.19 | 127.3 |
| J-A | 8 | tip | 12 | 5.9100 | 6.1096 | 5.7645 | 6.9744 | 0.7510 | 0.1511 | 0x32d5e235342b4143 | 0.79/3.55 | 129.2 |
| J-A | 16 | parent | 12 | 6.6626 | 7.1032 | 6.4121 | 8.2012 | 1.4505 | 0.2630 | 0x32d5e235342b4143 | 1.02/3.25 | 127.1 |
| J-A | 16 | tip | 12 | 5.6600 | 5.9900 | 5.5902 | 7.0456 | 0.9359 | 0.1946 | 0x32d5e235342b4143 | 0.57/6.25 | 129.3 |
| J-As | 1 | parent | 12 | 10.6934 | 10.8421 | 10.4381 | 11.6195 | 0.3172 | 0.1396 | 0x32d5e235342b4143 | 0.35/3.94 | 131.7 |
| J-As | 1 | tip | 12 | 8.6973 | 8.7704 | 8.3954 | 9.3084 | 0.3256 | 0.0970 | 0x32d5e235342b4143 | 0.28/1.50 | 131.9 |
| J-As | 2 | parent | 12 | 7.6592 | 8.0061 | 7.6237 | 9.1785 | 0.9365 | 0.2088 | 0x32d5e235342b4143 | 0.33/5.50 | 130.6 |
| J-As | 2 | tip | 12 | 6.2214 | 6.4006 | 6.1691 | 7.0624 | 0.5127 | 0.1139 | 0x32d5e235342b4143 | 0.56/3.31 | 130.6 |
| J-As | 4 | parent | 12 | 6.1756 | 6.4405 | 6.0421 | 7.6457 | 0.6210 | 0.1891 | 0x32d5e235342b4143 | 0.49/2.17 | 127.5 |
| J-As | 4 | tip | 12 | 4.9225 | 5.0127 | 4.8896 | 5.3343 | 0.2664 | 0.0637 | 0x32d5e235342b4143 | 0.52/3.14 | 128.1 |
| J-As | 8 | parent | 12 | 5.3425 | 5.6494 | 5.2151 | 6.9026 | 0.8310 | 0.2119 | 0x32d5e235342b4143 | 0.39/3.67 | 125.2 |
| J-As | 8 | tip | 12 | 4.2399 | 4.3747 | 4.2041 | 5.0045 | 0.1936 | 0.1005 | 0x32d5e235342b4143 | 0.35/6.80 | 126.2 |
| J-As | 16 | parent | 12 | 5.4691 | 5.7800 | 5.3440 | 6.7371 | 1.0291 | 0.1961 | 0x32d5e235342b4143 | 0.50/2.76 | 125.5 |
| J-As | 16 | tip | 12 | 4.3567 | 4.5216 | 4.3288 | 5.2007 | 0.3211 | 0.1147 | 0x32d5e235342b4143 | 0.29/3.86 | 126.8 |
| J-As-a | 1 | parent | 12 | 10.6584 | 10.6587 | 10.5487 | 10.8609 | 0.0780 | 0.0289 | 0x32d5e235342b4143 | 0.20/0.53 | 133.4 |
| J-As-a | 1 | tip | 12 | 8.5539 | 8.6075 | 8.3838 | 8.9601 | 0.2050 | 0.0642 | 0x32d5e235342b4143 | 0.22/1.32 | 134.8 |
| J-As-a | 8 | parent | 12 | 5.2754 | 5.2730 | 5.2310 | 5.3113 | 0.0619 | 0.0107 | 0x32d5e235342b4143 | 0.23/0.44 | 125.7 |
| J-As-a | 8 | tip | 12 | 4.2421 | 4.2455 | 4.2197 | 4.2943 | 0.0185 | 0.0075 | 0x32d5e235342b4143 | 0.34/0.72 | 126.7 |
| J-C | 1 | parent | 1 | 11.1528 | 11.1528 | 11.1528 | 11.1528 | 0.0000 | 0.0000 | 0x32d5e235342b4143 | 0.18/0.18 | 133.7 |
| J-C | 1 | tip | 1 | 8.7517 | 8.7517 | 8.7517 | 8.7517 | 0.0000 | 0.0000 | 0x32d5e235342b4143 | 0.31/0.31 | 135.5 |
| R | 1 | parent | 12 | 14.2999 | 14.5117 | 13.7437 | 15.7120 | 1.1556 | 0.2461 | 0x87e561d20589d4a5 | 0.34/4.12 | 130.8 |
| R | 1 | tip | 12 | 11.1608 | 11.1892 | 10.7602 | 11.6416 | 0.5455 | 0.1111 | 0x87e561d20589d4a5 | 0.29/1.11 | 132.1 |
| R | 8 | parent | 12 | 6.7808 | 7.0553 | 6.6717 | 8.1955 | 0.5938 | 0.1909 | 0x87e561d20589d4a5 | 0.45/3.07 | 123.9 |
| R | 8 | tip | 12 | 5.2456 | 5.4744 | 5.1038 | 6.3535 | 0.8401 | 0.1646 | 0x87e561d20589d4a5 | 0.35/4.01 | 126.7 |
| R-S | 1 | parent | 12 | 6.0207 | 6.2307 | 5.9504 | 7.1272 | 0.4697 | 0.1284 | 0x2a2b7926a48aab00 | 0.38/3.23 | 131.9 |
| R-S | 1 | tip | 12 | 5.9281 | 5.9775 | 5.8391 | 6.2996 | 0.1688 | 0.0490 | 0x2a2b7926a48aab00 | 0.43/2.91 | 131.8 |
| R-S | 8 | parent | 12 | 3.2338 | 3.5823 | 3.1934 | 5.7238 | 0.5092 | 0.2602 | 0x2a2b7926a48aab00 | 0.33/8.68 | 121.9 |
| R-S | 8 | tip | 12 | 3.1136 | 3.1628 | 3.0883 | 3.4575 | 0.0165 | 0.0446 | 0x2a2b7926a48aab00 | 0.55/4.51 | 123.1 |
| S16 | 1 | parent | 12 | 0.0810 | 0.0813 | 0.0779 | 0.0857 | 0.0030 | 0.0008 | 0x8877dbb1192e9b92 | 0.00/5.97 | 129.9 |
| S16 | 1 | tip | 12 | 0.0800 | 0.0806 | 0.0785 | 0.0844 | 0.0032 | 0.0007 | 0x8877dbb1192e9b92 | 0.00/3.03 | 130.2 |

### Effects, tip - parent (ms/step). SE reading: claimed iff |delta| > 2*hypot(SE_p, SE_t). Min-max reading: intervals disjoint.

| row | W | parent | tip | delta ms | % | 2*SE | claimed (SE) | min-max disjoint | IQR disjoint | direction |
|---|---|---|---|---|---|---|---|---|---|---|
| J-A | 1 | 19.5823 | 18.9323 | -0.6500 | -3.32 | 0.2514 | YES | no | yes | tip faster |
| J-A | 2 | 12.3248 | 11.4646 | -0.8601 | -6.98 | 0.4590 | YES | yes | yes | tip faster |
| J-A | 4 | 8.5293 | 7.7689 | -0.7603 | -8.91 | 0.5141 | YES | no | yes | tip faster |
| J-A | 8 | 6.5921 | 5.9100 | -0.6820 | -10.35 | 0.4936 | YES | no | yes | tip faster |
| J-A | 16 | 6.6626 | 5.6600 | -1.0026 | -15.05 | 0.6544 | YES | no | no | tip faster |
| J-As | 1 | 10.6934 | 8.6973 | -1.9961 | -18.67 | 0.3400 | YES | yes | yes | tip faster |
| J-As | 2 | 7.6592 | 6.2214 | -1.4378 | -18.77 | 0.4757 | YES | yes | yes | tip faster |
| J-As | 4 | 6.1756 | 4.9225 | -1.2531 | -20.29 | 0.3991 | YES | yes | yes | tip faster |
| J-As | 8 | 5.3425 | 4.2399 | -1.1026 | -20.64 | 0.4690 | YES | yes | yes | tip faster |
| J-As | 16 | 5.4691 | 4.3567 | -1.1124 | -20.34 | 0.4544 | YES | yes | yes | tip faster |
| J-As-a | 1 | 10.6584 | 8.5539 | -2.1046 | -19.75 | 0.1408 | YES | yes | yes | tip faster |
| J-As-a | 8 | 5.2754 | 4.2421 | -1.0333 | -19.59 | 0.0261 | YES | yes | yes | tip faster |
| J-C | 1 | 11.1528 | 8.7517 | -2.4010 | -21.53 | 0.0000 | YES | yes | yes | tip faster |
| R | 1 | 14.2999 | 11.1608 | -3.1392 | -21.95 | 0.5401 | YES | yes | yes | tip faster |
| R | 8 | 6.7808 | 5.2456 | -1.5352 | -22.64 | 0.5041 | YES | yes | yes | tip faster |
| R-S | 1 | 6.0207 | 5.9281 | -0.0927 | -1.54 | 0.2748 | no | no | no | tip faster |
| R-S | 8 | 3.2338 | 3.1136 | -0.1201 | -3.72 | 0.5280 | no | no | yes | tip faster |
| S16 | 1 | 0.0810 | 0.0800 | -0.0009 | -1.17 | 0.0021 | no | no | no | tip faster |

### Per-stage spans (armed rows; per process the median over the steps named, then the median over K; ms and us per manifold on 4,524.2)

#### J-As-a W=1 (steps [100, 500))

| stage | parent ms | tip ms | delta ms | % | 2*SE ms | claimed (SE) | parent us/manifold | tip us/manifold |
|---|---|---|---|---|---|---|---|---|
| phys_solve_build_ns | 0.8287 | 0.2752 | -0.5535 | -66.79 | 0.0059 | YES | 0.1832 | 0.0608 |
| phys_gravity_ns | 0.0075 | 0.0072 | -0.0004 | -4.85 | 0.0001 | YES | 0.0017 | 0.0016 |
| phys_warm_apply_ns | 0.6125 | 0.5188 | -0.0936 | -15.29 | 0.0014 | YES | 0.1354 | 0.1147 |
| phys_integrate_ns | 0.0643 | 0.0644 | +0.0001 | +0.17 | 0.0019 | no | 0.0142 | 0.0142 |
| phys_pass_biased_ns | 1.1750 | 0.8471 | -0.3279 | -27.90 | 0.0322 | YES | 0.2597 | 0.1872 |
| phys_pass_relax_ns | 2.3117 | 1.6755 | -0.6362 | -27.52 | 0.0646 | YES | 0.5110 | 0.3703 |
| phys_color_wide_ns | 3.4722 | 2.5129 | -0.9593 | -27.63 | 0.0967 | YES | 0.7675 | 0.5554 |
| phys_color_narrow_ns | 0.0129 | 0.0094 | -0.0035 | -27.37 | 0.0003 | YES | 0.0028 | 0.0021 |
| phys_restitution_ns | 0.0057 | 0.0032 | -0.0024 | -42.89 | 0.0000 | YES | 0.0013 | 0.0007 |
| phys_store_ns | 0.1568 | 0.0205 | -0.1363 | -86.92 | 0.0246 | YES | 0.0347 | 0.0045 |
| phys_write_back_ns | 0.0026 | 0.0025 | -0.0000 | -1.76 | 0.0002 | no | 0.0006 | 0.0006 |
| phys_np_dispatch_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | 0.0000 | 0.0000 |
| phys_np_compact_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | 0.0000 | 0.0000 |
| phys_np_axis_commit_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | 0.0000 | 0.0000 |
| sys_physics_integrate_ns | 0.0001 | 0.0000 | -0.0001 | -70.91 | 0.0000 | YES | 0.0000 | 0.0000 |
| sys_physics_gather_ns | 0.0260 | 0.0235 | -0.0025 | -9.59 | 0.0006 | YES | 0.0057 | 0.0052 |
| sys_select_broadphase_ns | 0.0001 | 0.0001 | -0.0000 | -28.57 | 0.0000 | YES | 0.0000 | 0.0000 |
| sys_physics_broadphase_ns | 1.9194 | 1.8784 | -0.0410 | -2.14 | 0.0107 | YES | 0.4242 | 0.4152 |
| sys_physics_narrowphase_ns | 2.9990 | 2.9752 | -0.0238 | -0.79 | 0.0127 | YES | 0.6629 | 0.6576 |
| sys_physics_build_graph_ns | 0.1152 | 0.1145 | -0.0006 | -0.55 | 0.0014 | no | 0.0255 | 0.0253 |
| sys_physics_solve_colored_ns | 5.2136 | 3.4201 | -1.7934 | -34.40 | 0.1035 | YES | 1.1524 | 0.7560 |
| sys_physics_apply_ns | 0.0141 | 0.0127 | -0.0014 | -9.95 | 0.0013 | YES | 0.0031 | 0.0028 |
| u_ns | 0.0013 | 0.0010 | -0.0003 | -23.80 | 0.0001 | YES | 0.0003 | 0.0002 |
| g_ns | 0.0929 | 0.0385 | -0.0544 | -58.55 | 0.0144 | YES | 0.0205 | 0.0085 |
| r_ns | 0.0041 | 0.0026 | -0.0015 | -36.57 | 0.0002 | YES | 0.0009 | 0.0006 |
| sys_sum_ns | 10.4065 | 8.4314 | -1.9751 | -18.98 | 0.1186 | YES | 2.3002 | 1.8636 |
| wall_ns | 10.5859 | 8.4782 | -2.1077 | -19.91 | 0.1270 | YES | 2.3398 | 1.8740 |

counters (median): manifolds parent 4519.0 / tip 4519.0; pairs parent 9559.0 / tip 9559.0; colors parent 11.0 / tip 11.0; wide_colors parent 9.0 / tip 9.0; waves parent 108.0 / tip 108.0; phys_np_points parent 17054.0 / tip 17054.0; phys_slots_wide parent 17013.5 / tip 17013.5; phys_slots_narrow parent 36.0 / tip 36.0

#### J-As-a W=8 (steps [100, 500))

| stage | parent ms | tip ms | delta ms | % | 2*SE ms | claimed (SE) | parent us/manifold | tip us/manifold |
|---|---|---|---|---|---|---|---|---|
| phys_solve_build_ns | 0.8393 | 0.3054 | -0.5339 | -63.61 | 0.0058 | YES | 0.1855 | 0.0675 |
| phys_gravity_ns | 0.0124 | 0.0122 | -0.0002 | -1.89 | 0.0001 | YES | 0.0027 | 0.0027 |
| phys_warm_apply_ns | 0.6245 | 0.5329 | -0.0917 | -14.68 | 0.0016 | YES | 0.1380 | 0.1178 |
| phys_integrate_ns | 0.0784 | 0.0810 | +0.0026 | +3.29 | 0.0020 | YES | 0.0173 | 0.0179 |
| phys_pass_biased_ns | 0.3055 | 0.2242 | -0.0813 | -26.60 | 0.0036 | YES | 0.0675 | 0.0496 |
| phys_pass_relax_ns | 0.5770 | 0.4089 | -0.1681 | -29.13 | 0.0076 | YES | 0.1275 | 0.0904 |
| phys_color_wide_ns | 0.8633 | 0.6191 | -0.2442 | -28.28 | 0.0113 | YES | 0.1908 | 0.1369 |
| phys_color_narrow_ns | 0.0147 | 0.0105 | -0.0043 | -28.96 | 0.0004 | YES | 0.0033 | 0.0023 |
| phys_restitution_ns | 0.0059 | 0.0032 | -0.0026 | -44.62 | 0.0000 | YES | 0.0013 | 0.0007 |
| phys_store_ns | 0.1529 | 0.0253 | -0.1276 | -83.43 | 0.0042 | YES | 0.0338 | 0.0056 |
| phys_write_back_ns | 0.0027 | 0.0027 | -0.0000 | -0.19 | 0.0000 | no | 0.0006 | 0.0006 |
| phys_np_dispatch_ns | 0.4977 | 0.4953 | -0.0024 | -0.48 | 0.0016 | YES | 0.1100 | 0.1095 |
| phys_np_compact_ns | 0.0194 | 0.0184 | -0.0009 | -4.78 | 0.0004 | YES | 0.0043 | 0.0041 |
| phys_np_axis_commit_ns | 0.0036 | 0.0046 | +0.0010 | +27.21 | 0.0000 | YES | 0.0008 | 0.0010 |
| sys_physics_integrate_ns | 0.0000 | 0.0000 | +0.0000 | +0.00 | 0.0000 | no | 0.0000 | 0.0000 |
| sys_physics_gather_ns | 0.0251 | 0.0249 | -0.0002 | -0.70 | 0.0003 | no | 0.0055 | 0.0055 |
| sys_select_broadphase_ns | 0.0001 | 0.0001 | +0.0000 | +0.00 | 0.0000 | no | 0.0000 | 0.0000 |
| sys_physics_broadphase_ns | 1.9078 | 1.9068 | -0.0010 | -0.05 | 0.0038 | no | 0.4217 | 0.4215 |
| sys_physics_narrowphase_ns | 0.5260 | 0.5227 | -0.0033 | -0.63 | 0.0017 | YES | 0.1163 | 0.1155 |
| sys_physics_build_graph_ns | 0.1150 | 0.1167 | +0.0017 | +1.51 | 0.0012 | YES | 0.0254 | 0.0258 |
| sys_physics_solve_colored_ns | 2.6125 | 1.6015 | -1.0109 | -38.70 | 0.0165 | YES | 0.5774 | 0.3540 |
| sys_physics_apply_ns | 0.0129 | 0.0131 | +0.0002 | +1.67 | 0.0013 | no | 0.0029 | 0.0029 |
| u_ns | 0.0011 | 0.0011 | -0.0001 | -8.28 | 0.0001 | YES | 0.0003 | 0.0002 |
| g_ns | 0.0334 | 0.0320 | -0.0014 | -4.24 | 0.0004 | YES | 0.0074 | 0.0071 |
| r_ns | 0.0046 | 0.0038 | -0.0007 | -16.10 | 0.0001 | YES | 0.0010 | 0.0008 |
| sys_sum_ns | 5.2077 | 4.1891 | -1.0185 | -19.56 | 0.0174 | YES | 1.1511 | 0.9259 |
| wall_ns | 5.2414 | 4.2217 | -1.0197 | -19.45 | 0.0174 | YES | 1.1585 | 0.9331 |

counters (median): manifolds parent 4519.0 / tip 4519.0; pairs parent 9559.0 / tip 9559.0; colors parent 11.0 / tip 11.0; wide_colors parent 9.0 / tip 9.0; waves parent 108.0 / tip 108.0; phys_np_points parent 17054.0 / tip 17054.0; phys_slots_wide parent 17013.5 / tip 17013.5; phys_slots_narrow parent 36.0 / tip 36.0

#### J-C W=1 (steps [100, 500))

| stage | parent ms | tip ms | delta ms | % | 2*SE ms | claimed (SE) | parent us/manifold | tip us/manifold |
|---|---|---|---|---|---|---|---|---|
| phys_solve_build_ns | 0.8321 | 0.2742 | -0.5580 | -67.05 | 0.0000 | YES | 0.1839 | 0.0606 |
| phys_gravity_ns | 0.0074 | 0.0071 | -0.0003 | -3.76 | 0.0000 | YES | 0.0016 | 0.0016 |
| phys_warm_apply_ns | 0.6107 | 0.5202 | -0.0905 | -14.82 | 0.0000 | YES | 0.1350 | 0.1150 |
| phys_integrate_ns | 0.0632 | 0.0669 | +0.0036 | +5.76 | 0.0000 | YES | 0.0140 | 0.0148 |
| phys_pass_biased_ns | 1.1706 | 0.8030 | -0.3676 | -31.40 | 0.0000 | YES | 0.2587 | 0.1775 |
| phys_pass_relax_ns | 2.2960 | 1.5913 | -0.7047 | -30.69 | 0.0000 | YES | 0.5075 | 0.3517 |
| phys_color_wide_ns | 3.4504 | 2.3841 | -1.0663 | -30.90 | 0.0000 | YES | 0.7626 | 0.5270 |
| phys_color_narrow_ns | 0.0127 | 0.0089 | -0.0039 | -30.26 | 0.0000 | YES | 0.0028 | 0.0020 |
| phys_restitution_ns | 0.0057 | 0.0032 | -0.0025 | -44.37 | 0.0000 | YES | 0.0013 | 0.0007 |
| phys_store_ns | 0.1663 | 0.0201 | -0.1462 | -87.89 | 0.0000 | YES | 0.0368 | 0.0045 |
| phys_write_back_ns | 0.0026 | 0.0025 | -0.0000 | -1.52 | 0.0000 | YES | 0.0006 | 0.0006 |
| phys_np_dispatch_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | 0.0000 | 0.0000 |
| phys_np_compact_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | 0.0000 | 0.0000 |
| phys_np_axis_commit_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | 0.0000 | 0.0000 |
| sys_physics_integrate_ns | 0.0002 | 0.0001 | -0.0001 | -68.75 | 0.0000 | YES | 0.0000 | 0.0000 |
| sys_physics_gather_ns | 0.0265 | 0.0236 | -0.0029 | -10.84 | 0.0000 | YES | 0.0058 | 0.0052 |
| sys_select_broadphase_ns | 0.0001 | 0.0001 | -0.0000 | -28.57 | 0.0000 | YES | 0.0000 | 0.0000 |
| sys_physics_broadphase_ns | 1.9192 | 1.8605 | -0.0588 | -3.06 | 0.0000 | YES | 0.4242 | 0.4112 |
| sys_physics_narrowphase_ns | 2.9936 | 2.9548 | -0.0388 | -1.30 | 0.0000 | YES | 0.6617 | 0.6531 |
| sys_physics_build_graph_ns | 0.1159 | 0.1163 | +0.0004 | +0.34 | 0.0000 | YES | 0.0256 | 0.0257 |
| sys_physics_solve_colored_ns | 5.1855 | 3.2961 | -1.8894 | -36.44 | 0.0000 | YES | 1.1462 | 0.7285 |
| sys_physics_apply_ns | 0.0126 | 0.0126 | -0.0000 | -0.07 | 0.0000 | YES | 0.0028 | 0.0028 |
| u_ns | 0.0013 | 0.0009 | -0.0004 | -27.36 | 0.0000 | YES | 0.0003 | 0.0002 |
| g_ns | 0.0799 | 0.0364 | -0.0435 | -54.47 | 0.0000 | YES | 0.0177 | 0.0080 |
| r_ns | 0.0038 | 0.0025 | -0.0013 | -34.07 | 0.0000 | YES | 0.0008 | 0.0006 |
| sys_sum_ns | 10.8894 | 8.6927 | -2.1967 | -20.17 | 0.0000 | YES | 2.4069 | 1.9214 |
| wall_ns | 11.0829 | 8.7301 | -2.3527 | -21.23 | 0.0000 | YES | 2.4497 | 1.9296 |

counters (median): manifolds parent 4519.0 / tip 4519.0; pairs parent 9559.0 / tip 9559.0; colors parent 11.0 / tip 11.0; wide_colors parent 9.0 / tip 9.0; waves parent 108.0 / tip 108.0; phys_np_points parent 17054.0 / tip 17054.0; phys_slots_wide parent 17013.5 / tip 17013.5; phys_slots_narrow parent 36.0 / tip 36.0

#### R W=1 (steps [600, 1100))

| stage | parent ms | tip ms | delta ms | % | 2*SE ms | claimed (SE) | parent us/manifold | tip us/manifold |
|---|---|---|---|---|---|---|---|---|
| phys_solve_build_ns | 1.3484 | 0.4303 | -0.9181 | -68.09 | 0.2686 | YES | - | - |
| phys_gravity_ns | 0.0080 | 0.0075 | -0.0005 | -6.42 | 0.0004 | YES | - | - |
| phys_warm_apply_ns | 0.8447 | 0.7316 | -0.1131 | -13.39 | 0.0079 | YES | - | - |
| phys_integrate_ns | 0.0651 | 0.0653 | +0.0002 | +0.35 | 0.0027 | no | - | - |
| phys_pass_biased_ns | 1.8030 | 1.2470 | -0.5560 | -30.84 | 0.0555 | YES | - | - |
| phys_pass_relax_ns | 3.5128 | 2.4601 | -1.0527 | -29.97 | 0.1107 | YES | - | - |
| phys_color_wide_ns | 5.2598 | 3.6689 | -1.5909 | -30.25 | 0.1649 | YES | - | - |
| phys_color_narrow_ns | 0.0512 | 0.0364 | -0.0148 | -28.87 | 0.0018 | YES | - | - |
| phys_restitution_ns | 0.0080 | 0.0048 | -0.0032 | -40.25 | 0.0006 | YES | - | - |
| phys_store_ns | 0.4100 | 0.0348 | -0.3752 | -91.50 | 0.1354 | YES | - | - |
| phys_write_back_ns | 0.0030 | 0.0026 | -0.0004 | -14.41 | 0.0011 | no | - | - |
| phys_np_dispatch_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | - | - |
| phys_np_compact_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | - | - |
| phys_np_axis_commit_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | - | - |
| sys_physics_integrate_ns | 0.0001 | 0.0001 | +0.0000 | +0.00 | 0.0001 | no | - | - |
| sys_physics_gather_ns | 0.0297 | 0.0257 | -0.0040 | -13.46 | 0.0025 | YES | - | - |
| sys_select_broadphase_ns | 0.0001 | 0.0001 | -0.0000 | -29.17 | 0.0000 | no | - | - |
| sys_physics_broadphase_ns | 2.0022 | 1.9790 | -0.0233 | -1.16 | 0.0857 | no | - | - |
| sys_physics_narrowphase_ns | 3.6649 | 3.5679 | -0.0971 | -2.65 | 0.1048 | no | - | - |
| sys_physics_build_graph_ns | 0.2193 | 0.2067 | -0.0125 | -5.72 | 0.0188 | no | - | - |
| sys_physics_solve_colored_ns | 8.1042 | 5.0216 | -3.0827 | -38.04 | 0.4644 | YES | - | - |
| sys_physics_apply_ns | 0.0156 | 0.0159 | +0.0003 | +1.77 | 0.0027 | no | - | - |
| u_ns | 0.0015 | 0.0013 | -0.0002 | -14.23 | 0.0003 | no | - | - |
| g_ns | 0.0922 | 0.0909 | -0.0013 | -1.44 | 0.0520 | no | - | - |
| r_ns | 0.0057 | 0.0040 | -0.0017 | -30.24 | 0.0003 | YES | - | - |
| sys_sum_ns | 14.0612 | 10.9048 | -3.1564 | -22.45 | 0.5616 | YES | - | - |
| wall_ns | 14.1992 | 11.0662 | -3.1330 | -22.06 | 0.5273 | YES | - | - |

counters (median): manifolds parent 6661.0 / tip 6661.0; pairs parent 9570.0 / tip 9570.0; colors parent 16.0 / tip 16.0; wide_colors parent 14.0 / tip 14.0; waves parent 168.0 / tip 168.0; phys_np_points parent 22961.0 / tip 22961.0; phys_slots_wide parent 22725.0 / tip 22725.0; phys_slots_narrow parent 237.0 / tip 237.0

#### R W=8 (steps [600, 1100))

| stage | parent ms | tip ms | delta ms | % | 2*SE ms | claimed (SE) | parent us/manifold | tip us/manifold |
|---|---|---|---|---|---|---|---|---|
| phys_solve_build_ns | 1.2305 | 0.4940 | -0.7365 | -59.86 | 0.2147 | YES | - | - |
| phys_gravity_ns | 0.0128 | 0.0128 | +0.0000 | +0.16 | 0.0004 | no | - | - |
| phys_warm_apply_ns | 0.8531 | 0.7473 | -0.1059 | -12.41 | 0.0141 | YES | - | - |
| phys_integrate_ns | 0.0788 | 0.0836 | +0.0047 | +5.99 | 0.0029 | YES | - | - |
| phys_pass_biased_ns | 0.4853 | 0.3311 | -0.1542 | -31.77 | 0.0245 | YES | - | - |
| phys_pass_relax_ns | 0.9373 | 0.6309 | -0.3064 | -32.69 | 0.0487 | YES | - | - |
| phys_color_wide_ns | 1.3546 | 0.9135 | -0.4412 | -32.57 | 0.0722 | YES | - | - |
| phys_color_narrow_ns | 0.0575 | 0.0422 | -0.0153 | -26.66 | 0.0026 | YES | - | - |
| phys_restitution_ns | 0.0081 | 0.0048 | -0.0033 | -40.74 | 0.0006 | YES | - | - |
| phys_store_ns | 0.2936 | 0.0436 | -0.2500 | -85.15 | 0.1005 | YES | - | - |
| phys_write_back_ns | 0.0029 | 0.0027 | -0.0002 | -7.34 | 0.0005 | no | - | - |
| phys_np_dispatch_ns | 0.5895 | 0.5852 | -0.0042 | -0.72 | 0.0298 | no | - | - |
| phys_np_compact_ns | 0.0281 | 0.0276 | -0.0005 | -1.73 | 0.0123 | no | - | - |
| phys_np_axis_commit_ns | 0.0035 | 0.0045 | +0.0010 | +28.22 | 0.0002 | YES | - | - |
| sys_physics_integrate_ns | 0.0001 | 0.0000 | -0.0000 | -10.00 | 0.0001 | no | - | - |
| sys_physics_gather_ns | 0.0255 | 0.0254 | -0.0001 | -0.52 | 0.0014 | no | - | - |
| sys_select_broadphase_ns | 0.0001 | 0.0001 | +0.0000 | +0.00 | 0.0000 | no | - | - |
| sys_physics_broadphase_ns | 1.9124 | 1.9185 | +0.0061 | +0.32 | 0.0317 | no | - | - |
| sys_physics_narrowphase_ns | 0.6286 | 0.6231 | -0.0055 | -0.87 | 0.0435 | no | - | - |
| sys_physics_build_graph_ns | 0.1822 | 0.1851 | +0.0029 | +1.62 | 0.0121 | no | - | - |
| sys_physics_solve_colored_ns | 3.8921 | 2.3608 | -1.5314 | -39.35 | 0.3827 | YES | - | - |
| sys_physics_apply_ns | 0.0116 | 0.0130 | +0.0015 | +12.53 | 0.0014 | YES | - | - |
| u_ns | 0.0012 | 0.0011 | -0.0001 | -10.59 | 0.0002 | no | - | - |
| g_ns | 0.0342 | 0.0329 | -0.0013 | -3.85 | 0.0028 | no | - | - |
| r_ns | 0.0069 | 0.0057 | -0.0012 | -17.44 | 0.0002 | YES | - | - |
| sys_sum_ns | 6.6600 | 5.1289 | -1.5311 | -22.99 | 0.4896 | YES | - | - |
| wall_ns | 6.6941 | 5.1627 | -1.5314 | -22.88 | 0.4922 | YES | - | - |

counters (median): manifolds parent 6661.0 / tip 6661.0; pairs parent 9570.0 / tip 9570.0; colors parent 16.0 / tip 16.0; wide_colors parent 14.0 / tip 14.0; waves parent 168.0 / tip 168.0; phys_np_points parent 22961.0 / tip 22961.0; phys_slots_wide parent 22725.0 / tip 22725.0; phys_slots_narrow parent 237.0 / tip 237.0

#### R-S W=1 (steps [300, 800))

| stage | parent ms | tip ms | delta ms | % | 2*SE ms | claimed (SE) | parent us/manifold | tip us/manifold |
|---|---|---|---|---|---|---|---|---|
| phys_solve_build_ns | 0.0332 | 0.0480 | +0.0148 | +44.44 | 0.0072 | YES | - | - |
| phys_gravity_ns | 0.0056 | 0.0055 | -0.0001 | -2.05 | 0.0005 | no | - | - |
| phys_warm_apply_ns | 0.0000 | 0.0000 | +0.0000 | +0.00 | 0.0000 | no | - | - |
| phys_integrate_ns | 0.0641 | 0.0649 | +0.0008 | +1.24 | 0.0030 | no | - | - |
| phys_pass_biased_ns | 0.0018 | 0.0021 | +0.0003 | +15.44 | 0.0002 | YES | - | - |
| phys_pass_relax_ns | 0.0034 | 0.0040 | +0.0005 | +15.28 | 0.0002 | YES | - | - |
| phys_color_wide_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | - | - |
| phys_color_narrow_ns | 0.0022 | 0.0037 | +0.0015 | +67.74 | 0.0002 | YES | - | - |
| phys_restitution_ns | 0.0000 | 0.0000 | +0.0000 | +0.00 | 0.0000 | no | - | - |
| phys_store_ns | 0.2978 | 0.1281 | -0.1697 | -56.98 | 0.1356 | YES | - | - |
| phys_write_back_ns | 0.0009 | 0.0009 | +0.0000 | +0.00 | 0.0000 | no | - | - |
| phys_np_dispatch_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | - | - |
| phys_np_compact_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | - | - |
| phys_np_axis_commit_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | - | - |
| sys_physics_integrate_ns | 0.0000 | 0.0000 | +0.0000 | +0.00 | 0.0000 | no | - | - |
| sys_physics_gather_ns | 0.0245 | 0.0242 | -0.0003 | -1.32 | 0.0026 | no | - | - |
| sys_select_broadphase_ns | 0.0001 | 0.0001 | +0.0000 | +8.33 | 0.0000 | no | - | - |
| sys_physics_broadphase_ns | 1.8887 | 1.8880 | -0.0007 | -0.04 | 0.0138 | no | - | - |
| sys_physics_narrowphase_ns | 3.4508 | 3.4809 | +0.0301 | +0.87 | 0.0664 | no | - | - |
| sys_physics_build_graph_ns | 0.1821 | 0.1845 | +0.0024 | +1.31 | 0.0104 | no | - | - |
| sys_physics_solve_colored_ns | 0.4283 | 0.2782 | -0.1502 | -35.06 | 0.1425 | YES | - | - |
| sys_physics_apply_ns | 0.0046 | 0.0049 | +0.0004 | +7.65 | 0.0003 | YES | - | - |
| u_ns | 0.0007 | 0.0007 | -0.0000 | -2.73 | 0.0002 | no | - | - |
| g_ns | 0.0336 | 0.0341 | +0.0005 | +1.55 | 0.0037 | no | - | - |
| r_ns | 0.0030 | 0.0023 | -0.0007 | -22.98 | 0.0002 | YES | - | - |
| sys_sum_ns | 5.9477 | 5.8590 | -0.0887 | -1.49 | 0.2447 | no | - | - |
| wall_ns | 5.9809 | 5.8940 | -0.0869 | -1.45 | 0.2487 | no | - | - |

counters (median): manifolds parent 6675.0 / tip 6675.0; pairs parent 9570.0 / tip 9570.0; colors parent 16.0 / tip 16.0; wide_colors parent 0.0 / tip 0.0; waves parent 0.0 / tip 0.0; phys_np_points parent 22968.0 / tip 22968.0; phys_slots_wide parent 0.0 / tip 0.0; phys_slots_narrow parent 0.0 / tip 0.0

#### R-S W=8 (steps [300, 800))

| stage | parent ms | tip ms | delta ms | % | 2*SE ms | claimed (SE) | parent us/manifold | tip us/manifold |
|---|---|---|---|---|---|---|---|---|
| phys_solve_build_ns | 0.0334 | 0.0485 | +0.0152 | +45.47 | 0.0081 | YES | - | - |
| phys_gravity_ns | 0.0055 | 0.0056 | +0.0001 | +1.35 | 0.0015 | no | - | - |
| phys_warm_apply_ns | 0.0000 | 0.0000 | +0.0000 | +0.00 | 0.0000 | no | - | - |
| phys_integrate_ns | 0.0653 | 0.0655 | +0.0002 | +0.32 | 0.0040 | no | - | - |
| phys_pass_biased_ns | 0.0018 | 0.0022 | +0.0004 | +21.96 | 0.0003 | YES | - | - |
| phys_pass_relax_ns | 0.0035 | 0.0041 | +0.0006 | +17.76 | 0.0002 | YES | - | - |
| phys_color_wide_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | - | - |
| phys_color_narrow_ns | 0.0022 | 0.0039 | +0.0017 | +75.12 | 0.0003 | YES | - | - |
| phys_restitution_ns | 0.0000 | 0.0000 | +0.0000 | +100.00 | 0.0000 | YES | - | - |
| phys_store_ns | 0.2546 | 0.1295 | -0.1250 | -49.12 | 0.3107 | no | - | - |
| phys_write_back_ns | 0.0009 | 0.0009 | +0.0000 | +0.00 | 0.0001 | no | - | - |
| phys_np_dispatch_ns | 0.6079 | 0.6055 | -0.0025 | -0.41 | 0.0571 | no | - | - |
| phys_np_compact_ns | 0.0340 | 0.0322 | -0.0018 | -5.19 | 0.0205 | no | - | - |
| phys_np_axis_commit_ns | 0.0033 | 0.0043 | +0.0011 | +31.91 | 0.0002 | YES | - | - |
| sys_physics_integrate_ns | 0.0001 | 0.0001 | +0.0000 | +8.33 | 0.0001 | no | - | - |
| sys_physics_gather_ns | 0.0259 | 0.0254 | -0.0005 | -2.11 | 0.0034 | no | - | - |
| sys_select_broadphase_ns | 0.0001 | 0.0000 | -0.0000 | -20.00 | 0.0000 | no | - | - |
| sys_physics_broadphase_ns | 1.9020 | 1.8991 | -0.0029 | -0.15 | 0.0814 | no | - | - |
| sys_physics_narrowphase_ns | 0.6514 | 0.6497 | -0.0018 | -0.27 | 0.0881 | no | - | - |
| sys_physics_build_graph_ns | 0.1828 | 0.1841 | +0.0013 | +0.72 | 0.0321 | no | - | - |
| sys_physics_solve_colored_ns | 0.3951 | 0.2817 | -0.1133 | -28.69 | 0.3431 | no | - | - |
| sys_physics_apply_ns | 0.0047 | 0.0051 | +0.0003 | +7.29 | 0.0007 | no | - | - |
| u_ns | 0.0008 | 0.0007 | -0.0000 | -4.69 | 0.0004 | no | - | - |
| g_ns | 0.0329 | 0.0335 | +0.0006 | +1.79 | 0.0065 | no | - | - |
| r_ns | 0.0031 | 0.0024 | -0.0007 | -23.81 | 0.0003 | YES | - | - |
| sys_sum_ns | 3.1756 | 3.0556 | -0.1201 | -3.78 | 0.5948 | no | - | - |
| wall_ns | 3.2092 | 3.0899 | -0.1194 | -3.72 | 0.6020 | no | - | - |

counters (median): manifolds parent 6675.0 / tip 6675.0; pairs parent 9570.0 / tip 9570.0; colors parent 16.0 / tip 16.0; wide_colors parent 0.0 / tip 0.0; waves parent 0.0 / tip 0.0; phys_np_points parent 22968.0 / tip 22968.0; phys_slots_wide parent 0.0 / tip 0.0; phys_slots_narrow parent 0.0 / tip 0.0

#### S16 W=1 (steps [0, 300))

| stage | parent ms | tip ms | delta ms | % | 2*SE ms | claimed (SE) | parent us/manifold | tip us/manifold |
|---|---|---|---|---|---|---|---|---|
| phys_solve_build_ns | 0.0023 | 0.0010 | -0.0012 | -54.88 | 0.0001 | YES | - | - |
| phys_gravity_ns | 0.0002 | 0.0002 | -0.0000 | -13.64 | 0.0000 | YES | - | - |
| phys_warm_apply_ns | 0.0023 | 0.0019 | -0.0004 | -15.69 | 0.0000 | YES | - | - |
| phys_integrate_ns | 0.0010 | 0.0010 | +0.0000 | +3.09 | 0.0000 | no | - | - |
| phys_pass_biased_ns | 0.0149 | 0.0151 | +0.0002 | +1.22 | 0.0001 | YES | - | - |
| phys_pass_relax_ns | 0.0292 | 0.0296 | +0.0005 | +1.58 | 0.0002 | YES | - | - |
| phys_color_wide_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | - | - |
| phys_color_narrow_ns | 0.0436 | 0.0443 | +0.0007 | +1.65 | 0.0003 | YES | - | - |
| phys_restitution_ns | 0.0000 | 0.0000 | -0.0000 | -33.33 | 0.0000 | YES | - | - |
| phys_store_ns | 0.0003 | 0.0001 | -0.0003 | -82.40 | 0.0000 | YES | - | - |
| phys_write_back_ns | 0.0000 | 0.0000 | +0.0000 | +0.00 | 0.0000 | no | - | - |
| phys_np_dispatch_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | - | - |
| phys_np_compact_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | - | - |
| phys_np_axis_commit_ns | 0.0000 | 0.0000 | +0.0000 | +nan | 0.0000 | no | - | - |
| sys_physics_integrate_ns | 0.0000 | 0.0000 | +0.0000 | +33.33 | 0.0000 | YES | - | - |
| sys_physics_gather_ns | 0.0005 | 0.0005 | +0.0000 | +3.74 | 0.0000 | no | - | - |
| sys_select_broadphase_ns | 0.0000 | 0.0000 | +0.0000 | +0.00 | 0.0000 | no | - | - |
| sys_physics_broadphase_ns | 0.0004 | 0.0004 | -0.0000 | -1.25 | 0.0000 | no | - | - |
| sys_physics_narrowphase_ns | 0.0094 | 0.0096 | +0.0002 | +2.37 | 0.0001 | YES | - | - |
| sys_physics_build_graph_ns | 0.0005 | 0.0005 | -0.0000 | -1.42 | 0.0000 | no | - | - |
| sys_physics_solve_colored_ns | 0.0508 | 0.0495 | -0.0013 | -2.53 | 0.0004 | YES | - | - |
| sys_physics_apply_ns | 0.0002 | 0.0002 | -0.0000 | -4.55 | 0.0000 | no | - | - |
| u_ns | 0.0005 | 0.0005 | -0.0000 | -0.06 | 0.0000 | no | - | - |
| g_ns | 0.0168 | 0.0170 | +0.0001 | +0.74 | 0.0010 | no | - | - |
| r_ns | 0.0005 | 0.0004 | -0.0001 | -11.18 | 0.0000 | YES | - | - |
| sys_sum_ns | 0.0619 | 0.0607 | -0.0012 | -1.93 | 0.0004 | YES | - | - |
| wall_ns | 0.0790 | 0.0777 | -0.0013 | -1.58 | 0.0013 | no | - | - |

counters (median): manifolds parent 16.0 / tip 16.0; pairs parent 31.0 / tip 31.0; colors parent 2.0 / tip 2.0; wide_colors parent 0.0 / tip 0.0; waves parent 0.0 / tip 0.0; phys_np_points parent 64.0 / tip 64.0; phys_slots_wide parent 0.0 / tip 0.0; phys_slots_narrow parent 64.0 / tip 64.0

### Gates (G9)

| gate | delta ms | threshold ms (0.6 x lower predicted) / 2*SE | verdict |
|---|---|---|---|
| T1_J-As | -1.9961 | -0.4860 / 0.3400 | PASS |
| T8_J-As | -1.1026 | -0.3660 / 0.4690 | PASS |
| cfgA_not_slower_W1 | -0.6500 | - / 0.2514 | PASS (not claimed slower) |
| cfgA_not_slower_W2 | -0.8601 | - / 0.4590 | PASS (not claimed slower) |
| cfgA_not_slower_W4 | -0.7603 | - / 0.5141 | PASS (not claimed slower) |
| cfgA_not_slower_W8 | -0.6820 | - / 0.4936 | PASS (not claimed slower) |
| cfgA_not_slower_W16 | -1.0026 | - / 0.6544 | PASS (not claimed slower) |
| R_W1_not_slower | -3.1392 | - / 0.5401 | PASS (not claimed slower) |
| R_W8_not_slower | -1.5352 | - / 0.5041 | PASS (not claimed slower) |
| R-S_W1_not_slower | -0.0927 | - / 0.2748 | PASS (not claimed slower) |
| R-S_W8_not_slower | -0.1201 | - / 0.5280 | PASS (not claimed slower) |
| S16_W1_not_slower | -0.0009 | - / 0.0021 | PASS (not claimed slower) |
| J-As_W1_not_slower | -1.9961 | - / 0.3400 | PASS (not claimed slower) |
| J-As_W2_not_slower | -1.4378 | - / 0.4757 | PASS (not claimed slower) |
| J-As_W4_not_slower | -1.2531 | - / 0.3991 | PASS (not claimed slower) |
| J-As_W8_not_slower | -1.1026 | - / 0.4690 | PASS (not claimed slower) |
| J-As_W16_not_slower | -1.1124 | - / 0.4544 | PASS (not claimed slower) |
| solve_build_W1 | -0.5535 | -0.1740 / 0.0059 | PASS |
| warm_apply_W1 | -0.0936 | -0.1140 / 0.0014 | FAIL (C3 lever, NOT measurable in this window (C2 tip); reported, not gated) |
| store_W1 | -0.1363 | -0.0660 / 0.0246 | PASS |
| wide_colours_not_slower_W1 | -0.9593 | - / 0.0967 | PASS (not claimed slower) [faster claimed] |
| solve_build_W8 | -0.5339 | -0.1740 / 0.0058 | PASS |
| warm_apply_W8 | -0.0917 | -0.1140 / 0.0016 | FAIL (C3 lever, NOT measurable in this window (C2 tip); reported, not gated) |
| store_W8 | -0.1276 | -0.0660 / 0.0042 | PASS |
| wide_colours_not_slower_W8 | -0.2442 | - / 0.0113 | PASS (not claimed slower) [faster claimed] |

### Canary

- parent: canary_ns 543045 (ref 10860897.4); span median [100,500) 543355.0 ns, rel err +0.0006 (ok <= 5 %: True); T(J-C) 11.1528 ms vs J-As-a median 10.6584 ms: rise +0.4943 ms (canary 0.5430 ms; (rise - canary)/T = -0.0046); seen: True
- tip: canary_ns 428103 (ref 8562060.8); span median [100,500) 428572.0 ns, rel err +0.0011 (ok <= 5 %: True); T(J-C) 8.7517 ms vs J-As-a median 8.5539 ms: rise +0.1978 ms (canary 0.4281 ms; (rise - canary)/T = -0.0269); seen: False

### Load receipts

- 452 timed processes; 5-s receipt before: median 1.31 %, max 4.96 %, > 5 %: 0; after: median 1.35 %, max 12.35 %, > 5 %: 42
- during-process witness (others_busy_pct): median 0.36 %, max 8.71 %, > 5 %: 13
- build processes seen in receipts: none; during a process: none; total wait for quiet 1626 s

## 4. Sensitivity (`raw/sensitivity.md`, reproduced verbatim)

### Sensitivity: during-process witness (others_busy_pct > 5 %) and per-pass split

- chosen processes with others_busy_pct > 5 %: 5 of 410: J-A#tip@W16 p0 seq180 (6.25 %, mean 7.0456), J-As#tip@W8 p1 seq71 (6.8 %, mean 5.0045), R-S#parent@W8 p1 seq96 (8.68 %, mean 5.7238), J-As#parent@W2 p1 seq140 (5.5 %, mean 9.1785), S16#parent@W1 p1 seq142 (5.97 %, mean 0.0857)

| row | W | binary | median all K | median excl. others>5% (K) | median pass 0 (K) | median pass 1 (K) |
|---|---|---|---|---|---|---|
| J-A | 1 | parent | 19.5823 | 19.5823 (12) | 19.5455 (6) | 19.6827 (6) |
| J-A | 1 | tip | 18.9323 | 18.9323 (12) | 18.9473 (6) | 18.8681 (6) |
| J-A | 2 | parent | 12.3248 | 12.3248 (12) | 12.2361 (6) | 12.9610 (6) |
| J-A | 2 | tip | 11.4646 | 11.4646 (12) | 11.5233 (6) | 11.4523 (6) |
| J-A | 4 | parent | 8.5293 | 8.5293 (12) | 8.4683 (6) | 8.9991 (6) |
| J-A | 4 | tip | 7.7689 | 7.7689 (12) | 7.6687 (6) | 8.0770 (6) |
| J-A | 8 | parent | 6.5921 | 6.5921 (12) | 6.5609 (6) | 7.2409 (6) |
| J-A | 8 | tip | 5.9100 | 5.9100 (12) | 5.7732 (6) | 6.3307 (6) |
| J-A | 16 | parent | 6.6626 | 6.6626 (12) | 6.5108 (6) | 7.9096 (6) |
| J-A | 16 | tip | 5.6600 | 5.6508 (11) | 5.6378 (6) | 6.0368 (6) |
| J-As | 1 | parent | 10.6934 | 10.6934 (12) | 10.6872 (6) | 10.8184 (6) |
| J-As | 1 | tip | 8.6973 | 8.6973 (12) | 8.6738 (6) | 8.9231 (6) |
| J-As | 2 | parent | 7.6592 | 7.6438 (11) | 7.6305 (6) | 8.3871 (6) |
| J-As | 2 | tip | 6.2214 | 6.2214 (12) | 6.1756 (6) | 6.4692 (6) |
| J-As | 4 | parent | 6.1756 | 6.1756 (12) | 6.0629 (6) | 6.6309 (6) |
| J-As | 4 | tip | 4.9225 | 4.9225 (12) | 4.9015 (6) | 5.0982 (6) |
| J-As | 8 | parent | 5.3425 | 5.3425 (12) | 5.2364 (6) | 5.9878 (6) |
| J-As | 8 | tip | 4.2399 | 4.2300 (11) | 4.2116 (6) | 4.4002 (6) |
| J-As | 16 | parent | 5.4691 | 5.4691 (12) | 5.3837 (6) | 6.3353 (6) |
| J-As | 16 | tip | 4.3567 | 4.3567 (12) | 4.3499 (6) | 4.5819 (6) |
| J-As-a | 1 | parent | 10.6584 | 10.6584 (12) | 10.6279 (6) | 10.6775 (6) |
| J-As-a | 1 | tip | 8.5539 | 8.5539 (12) | 8.5388 (6) | 8.6064 (6) |
| J-As-a | 8 | parent | 5.2754 | 5.2754 (12) | 5.2734 (6) | 5.2758 (6) |
| J-As-a | 8 | tip | 4.2421 | 4.2421 (12) | 4.2478 (6) | 4.2341 (6) |
| J-C | 1 | parent | 11.1528 | 11.1528 (1) | 11.1528 (1) | nan (0) |
| J-C | 1 | tip | 8.7517 | 8.7517 (1) | 8.7517 (1) | nan (0) |
| R | 1 | parent | 14.2999 | 14.2999 (12) | 13.9678 (6) | 15.0342 (6) |
| R | 1 | tip | 11.1608 | 11.1608 (12) | 11.2105 (6) | 11.0881 (6) |
| R | 8 | parent | 6.7808 | 6.7808 (12) | 6.7313 (6) | 7.2232 (6) |
| R | 8 | tip | 5.2456 | 5.2456 (12) | 5.1111 (6) | 5.5743 (6) |
| R-S | 1 | parent | 6.0207 | 6.0207 (12) | 6.0100 (6) | 6.4223 (6) |
| R-S | 1 | tip | 5.9281 | 5.9281 (12) | 5.9121 (6) | 6.0279 (6) |
| R-S | 8 | parent | 3.2338 | 3.2333 (11) | 3.2260 (6) | 3.6963 (6) |
| R-S | 8 | tip | 3.1136 | 3.1136 (12) | 3.1137 (6) | 3.1136 (6) |
| S16 | 1 | parent | 0.0810 | 0.0809 (11) | 0.0803 (6) | 0.0825 (6) |
| S16 | 1 | tip | 0.0800 | 0.0800 (12) | 0.0798 (6) | 0.0820 (6) |

Effects tip - parent under the exclusion (others_busy_pct <= 5 % only) and per pass:

| row | W | delta all K | delta excl. | delta pass 0 | delta pass 1 |
|---|---|---|---|---|---|
| J-A | 1 | -0.6500 | -0.6500 | -0.5983 | -0.8146 |
| J-A | 2 | -0.8601 | -0.8601 | -0.7128 | -1.5087 |
| J-A | 4 | -0.7603 | -0.7603 | -0.7996 | -0.9221 |
| J-A | 8 | -0.6820 | -0.6820 | -0.7877 | -0.9102 |
| J-A | 16 | -1.0026 | -1.0118 | -0.8730 | -1.8728 |
| J-As | 1 | -1.9961 | -1.9961 | -2.0134 | -1.8952 |
| J-As | 2 | -1.4378 | -1.4224 | -1.4549 | -1.9179 |
| J-As | 4 | -1.2531 | -1.2531 | -1.1615 | -1.5327 |
| J-As | 8 | -1.1026 | -1.1125 | -1.0248 | -1.5877 |
| J-As | 16 | -1.1124 | -1.1124 | -1.0339 | -1.7534 |
| J-As-a | 1 | -2.1046 | -2.1046 | -2.0891 | -2.0711 |
| J-As-a | 8 | -1.0333 | -1.0333 | -1.0256 | -1.0418 |
| J-C | 1 | -2.4010 | -2.4010 | -2.4010 | +nan |
| R | 1 | -3.1392 | -3.1392 | -2.7573 | -3.9462 |
| R | 8 | -1.5352 | -1.5352 | -1.6201 | -1.6489 |
| R-S | 1 | -0.0927 | -0.0927 | -0.0979 | -0.3944 |
| R-S | 8 | -0.1201 | -0.1197 | -0.1123 | -0.5827 |
| S16 | 1 | -0.0009 | -0.0009 | -0.0006 | -0.0004 |

## 5. G9 gate arithmetic, read against `02-DESIGN-REV1.md` § Gates (all-K medians; pass-0-only deltas in brackets where they differ by more than 0.1 ms)

| G9 item | measured (tip − parent) | gate | arithmetic |
|---|---|---|---|
| parent vs tip, K=12 interleaved, receipts per process | 410 slots × K=12, 452 processes, 452 receipt pairs, 42 re-runs, 0 dropped | — | done |
| T(1) J-As | −1.996 ms (10.693 → 8.697; −18.7 %; 2·SE 0.340; min-max disjoint) [pass 0: −2.013] | ≤ −0.486 (0.6 × −0.81) | PASS |
| T(8) J-As | −1.103 ms (5.343 → 4.240; −20.6 %; 2·SE 0.469; min-max disjoint) [pass 0: −1.025; pass 1: −1.588] | ≤ −0.366 (0.6 × −0.61) | PASS |
| J-As at W 2 / 4 / 16 | −1.438 / −1.253 / −1.112 ms (−18.8 / −20.3 / −20.3 %), each > 2·SE and min-max disjoint | not claimed slower | PASS |
| solve_build (J-As-a) | W1 −0.554 ms (0.829 → 0.275; 0.183 → 0.061 µs/manifold); W8 −0.534 ms (0.839 → 0.305) | ≤ −0.174 (0.6 × −0.29) | PASS at both W |
| store (J-As-a) | W1 −0.136 ms (0.157 → 0.021; 0.035 → 0.0045 µs/manifold); W8 −0.128 ms (0.153 → 0.025) | ≤ −0.066 (0.6 × −0.11) | PASS at both W |
| warm_apply (J-As-a) | W1 −0.094 ms (0.613 → 0.519; 0.135 → 0.115 µs/manifold); W8 −0.092 ms | C3's lever, NOT in this tip | reported, not gated (below 0.6 × −0.19 = −0.114 as expected before C3) |
| wide colours (J-As-a `phys_color_wide`) | W1 −0.959 ms (3.472 → 2.513; −27.6 %; 2·SE 0.097); W8 −0.244 ms (0.863 → 0.619; −28.3 %; 2·SE 0.011) | not claimed slower | PASS; measured above SE at both W (the design says the −10…−25 % is claimed only if measured above SE — the measured −28 % is above SE and beyond that range) |
| cfg-A (J-A) at W 1 / 2 / 4 / 8 / 16 | −0.650 / −0.860 / −0.760 / −0.682 / −1.003 ms (−3.3 / −7.0 / −8.9 / −10.4 / −15.1 %), each > 2·SE; min-max disjoint only at W2 [pass 0: −0.598 / −0.713 / −0.800 / −0.788 / −0.873] | not claimed slower at any W | PASS at every W |
| R at W 1 / 8 | −3.139 / −1.535 ms (−22.0 / −22.6 %), > 2·SE, min-max disjoint | (not in G9's gate list; "not slower" reading) | not slower |
| R-S at W 1 / 8 | −0.093 / −0.120 ms (−1.5 / −3.7 %), NOT > 2·SE (0.275 / 0.528) | (not in G9's gate list) | not claimed either way |
| S16 at W 1 | −0.0009 ms (−1.2 %), NOT > 2·SE (0.0021) | (not in G9's gate list) | not claimed either way |
| the canary seen | parent: span reads the spin (543,355 vs 543,045 ns, +0.06 %), wall rise +0.494 ms against the J-As-a median for a 0.543 ms spin ((rise − spin)/T = −0.5 %). tip: span reads the spin (428,572 vs 428,103 ns, +0.11 %), wall rise +0.198 ms for a 0.428 ms spin ((rise − spin)/T = −2.7 %); the one J-C tip process (8.752 ms) lies inside the J-As-a tip cell's min-max (8.384–8.960), so at K=1 the rise is not resolvable against the process-to-process spread. | seen | span: seen on both binaries; wall rise: seen on the parent, within the spread on the tip (K=1 per binary, as the task specified) |

Per-stage totals (J-As-a, W=1): setup = build + warm + store 1.598 → 0.815 ms (0.353 → 0.180 µs/manifold); solve span `sys_physics_solve_colored` 5.214 → 3.420 ms (−34.4 %); `g_ns` (executor gap) 0.093 → 0.038 ms; `u_ns` (unzoned residue) 0.0013 → 0.0010 ms; broadphase 1.919 → 1.878 ms (−2.1 %, > 2·SE 0.011) and narrowphase 2.999 → 2.975 ms (−0.8 %, > 2·SE 0.013) — both outside the lever, both moved by less than 0.05 ms. Wall 10.586 → 8.478 ms. Counters identical on both binaries (manifolds 4,519, pairs 9,559, colours 11 / wide 9, waves 108, points 17,054, wide slots 17,013.5, narrow slots 36).

Against the design's target table (P0 J-B per-stage ms / µs per manifold → predicted after L11): solve_build 1.038 / 0.229 → predicted 0.25–0.55 / 0.055–0.122; measured parent 0.829 / 0.183, tip 0.275 / 0.061 (inside). store 0.270 / 0.060 → predicted 0.04–0.08 / 0.009–0.018; measured parent 0.157 / 0.035, tip 0.021 / 0.0045 (below the predicted range). warm_apply 0.614 / 0.136 → predicted 0.15–0.30 after C3; measured parent 0.613 / 0.135, tip 0.519 / 0.115 (C3 not landed). wide colours 3.576 / 0.790 → predicted 2.68–3.22 / 0.59–0.71; measured parent 3.472 / 0.768, tip 2.513 / 0.555 (below the predicted range). T(1) J-As "today" derived 11.04–11.14 → predicted 8.67–9.79; measured parent 10.693, tip 8.697 (inside). T(8) projected 5.5–5.7 → 3.93–4.69; measured parent 5.343, tip 4.240 (inside). R-S store 0.32–0.37 → predicted 0.05–0.10; measured W1 0.298 → 0.128 ms (−57 %, > 2·SE 0.136), W8 0.255 → 0.130 ms (2·SE 0.311, NOT > 2·SE: the parent R-S W8 cell has one 5.72 ms process against a 3.23 median, and its store spread follows). The parent's store and wide-colour values are lower than P0's J-B values (P0's J-B is cfg-B = Grid + simd; this window's rows are cfg-As = AllPairs + simd, and the parent tree carries L4/L5/A1b above P0's tip), so the "P0 ms" column of the design table is not this window's parent.

## 6. Facts the analyst should see (no claims)

1. R-S (everything frozen, the B1 carry path): the tip's `phys_solve_build` is +0.015 ms (0.033 → 0.048 ms, +44 %, > 2·SE 0.007) at W1 and +0.015 ms at W8, `phys_color_narrow` +0.0015 ms (+68 %), `phys_pass_biased/relax` +0.0003/+0.0005 ms — the only stages slower on the tip in any row, all at the microsecond level; R-S `phys_store` is −0.170 ms (W1). End to end R-S is −0.093 ms, not claimed.
2. The tip's R-S store (0.128 / 0.130 ms) is above the design's predicted 0.05–0.10 ms for the resting world.
3. S16 (16 manifolds): `phys_solve_build` 2.3 → 1.0 µs, `phys_store` 0.3 → 0.05 µs, `phys_pass_biased/relax` +0.2/+0.5 µs (+1.2/+1.6 %, > 2·SE), `phys_color_narrow` +0.7 µs (+1.7 %); wall −1.3 µs (−1.6 %), not > 2·SE.
4. The armed rows' cells are much tighter than the disarmed ones (J-As-a W8: SE 0.011 / 0.008 ms, min-max 0.08 / 0.07 ms; J-As W8 disarmed: SE 0.212 / 0.101, min-max 1.69 / 0.80 ms) because the armed block ran 09:59–10:12 on a quiet machine while the disarmed main block's pass 1 (08:55–09:57) carried the background load of § 2. The disarmed cell medians are nevertheless within 0.15 ms of the armed ones (J-As W1 10.693 / 8.697 vs J-As-a 10.658 / 8.554; W8 5.343 / 4.240 vs 5.275 / 4.242).
5. The pass split (§ 4) moves the parent more than the tip at W ≥ 2, so the pass-1 effects are the larger ones (J-A W16 −0.87 in pass 0, −1.87 in pass 1; J-As W8 −1.02 / −1.59). The pass-0 deltas are the cleaner estimate for the multi-threaded cells; every gate passes on them as well (listed in § 5).
6. J-As W=16 is not faster than W=8 on either binary (parent 5.469 vs 5.343, tip 4.357 vs 4.240); J-A W=16 vs W=8: parent 6.663 vs 6.592, tip 5.660 vs 5.910 (the tip's W16 is 0.25 ms faster than its W8; pass 0 alone: 5.638 vs 5.773).

## 7. Not measured here, and why

- `warm_apply` gate (−0.19 ms): C3's lever, not in the C2 tip; the measured −0.094 ms is reported above and does not meet 0.6 × −0.19.
- Jolt v5.6.0: not run (owner ruling); its window-3 cells (`D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/tables.md`, `JOLT56-T` medians over K=6: 9.828 / 5.770 / 3.581 / 2.569 / 2.388 ms at W 1/2/4/8/16; P0b's 9.650 / 5.708 / 3.557 / 2.493 / 2.447) are the comparison, from a different window and a different day; this window's tip J-As is 8.697 / 6.221 / 4.923 / 4.240 / 4.357 ms and J-A 18.932 / 11.465 / 7.769 / 5.910 / 5.660 ms at the same W, with the cross-window caveat window 3 § 3 states.
- Committed-memory receipts (design § "What moves": about 2 MB): not part of this task.
- C4 (parallel setup) and the serial/parallel A/B at W=8: not in this tip.

## 8. Deletions and what remains

- Deleted by absolute path after the window: `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad/win4b/parent_tree` and `D:/wt/_targets/l11-parent-msvc` (the parent binary survives as `bin/runner_parent.exe` with its sha256 in `bin/SHA256SUMS`).
- Kept under `scratchpad/win4b/`: `bin/`, `gate/`, `raw/`, `logs/`, `tools/`, `rows.json`, `window_report.md`, plus the untimed rehearsal `test/window/` and `dry/`.
- No commit was made; `D:/wt/lighttable` is untouched (`git status` clean before and after; only `D:/wt/_targets/vkval-msvc` was written by the tip build).
