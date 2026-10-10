# Physics perf campaign, window 4b — L11 C2 (the cohort-shaped solve setup) against its parent C0, under the design's G9 (2026-09-22)

Receipts for `docs/MEASUREMENT-QUEUE.md` section 12, the `RESULT, 2026-09-22, window 4b` block, and for the
dated line in the L11 block of `docs/physics/perf-campaign/levers/00-RULINGS.md`. Read `analysis.md` first: it
is the results-analyst's reduction, recomputed from `raw/` without the tester's scripts, and it carries the
block copied into the queue (its section 6 is the draft; the queue block is that draft with this directory's
path). `window_report.md` is the tester's own report of the same window. The design and the gate it was run
under: `docs/physics/perf-campaign/levers/L11-solve-setup/02-DESIGN-REV1.md`, section "Gates", item "G9,
timing (P0 protocol)".

`analysis.md` is verbatim from the analyst's scratch directory `win4b/`; wherever it writes `win4b/<path>`,
read `<path>` under this directory. Its window-3 inputs are quoted from
`docs/measurements/2026-09-21-physics-window3/` at `2ce03b66` (the window-3 record commit on
`merge/ke16-into-ecsnative`; it is not on this lane, whose merge-base with that branch is A1b `8af0e3b9`).

## What was measured

One window under the protocol ruled after P0 and used by window 3 (queue section 10, "Protocol ruling"):
a cell = the MEDIAN over K separate processes of the process's window mean (ms/step); spread = min-max, IQR,
with the median's SE (1.2533·SD/√K) beside them; a comparison is claimed iff |effect| > 2·hypot(spread_A,
spread_B), printed under all three readings — G9 names the SE form as its gate; 5-s load receipts before and
after every process (> 5 % busy or a build process ⇒ that process re-run once at the end of its pass); a
10-s receipt and three quiet 60-s polls (`tools/wait_idle4b.ps1`: 0 build / miri / lane-test processes, 10-s
CPU < 5 %) before each of the five blocks; no band-based void; placement P-none. **K = 12** = two passes ×
six rounds, parent and tip interleaved inside each W group row by row, pass 1 the whole list reversed, one
untimed warm-up (tip `J-As` at W=8) per pass; the armed block after the main block, the canary after the
armed block. Every process, parent and tip, ran with `--expect-pose gate/<row>_W1_parent.pose` (the parent's
pose from the untimed gate; the pose is W-independent) and its printed `pose_hash` was compared with
`rows.json`. Timed 08:20:42–10:12:19 +03:00 on the owner's workstation (Ryzen 9 5900HS, 8C/16T, High
performance, AC; the owner's agent sessions, Telegram and a browser open — see contamination), after the
window-4 runner had finished (no `runner_c3.exe` / `window4_run.py` / build process at 08:05; the builds
started 08:06). Jolt was **not run**: the owner's ruling of 2026-09-21 makes Jolt v5.6.0 the only reference,
and its window-3 cells (`918fd2b7`, K=6: 9.828 / 5.770 / 3.581 / 2.569 / 2.388 ms at W = 1 / 2 / 4 / 8 / 16)
are reused, never re-run.

| binary (sha256 prefix, `bin/SHA256SUMS`) | commit | tree / target dir | `Compiling boyko-physics` line | log |
|---|---|---|---|---|
| `runner_parent.exe` `8dfd0143` | `146a1125` (L11 C0: the test-only setup digest + the `J-As` runner row; **no solver change**, so it has the tip's runner interface) | `git archive 146a1125 \| tar -x` into the scratch `parent_tree/`, the tip's `Cargo.lock` copied in (the archive carries none; the fresh lock differed only in `libredox` 0.1.24 → 0.1.25 and the copy recompiled nothing, `logs/02_build_parent_tiplock.log`); `D:/wt/_targets/l11-parent-msvc`, cold, 47.07 s | `(C:\Users\flint\...\scratchpad\win4b\parent_tree\crates\boyko_physics)` | `logs/01_build_parent.log` |
| `runner_tip.exe` `29dbd993` | `f8873aae` (L11 C2 = HEAD of `perf/physics-l11-solve-setup`: C0 `146a1125` + C1 `691891c4` + C2 `f8873aae`; `git status` clean) | `D:/wt/lighttable`; `D:/wt/_targets/vkval-msvc`, 44.27 s | `(D:\wt\lighttable\crates\boyko_physics)` | `logs/03_build_tip.log` |
| Jolt v5.6.0 Distribution `918fd2b7` | — | not run; window 3's cells reused (owner ruling) | — | — |

Both runners: `cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid` (`parity`
inherits `release`: fat LTO, default CGU), rustc 1.98.1 `x86_64-pc-windows-msvc`
(`RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc`), `RUSTFLAGS` unset (`x86-64-v3` from `.cargo/config.toml`,
byte-identical in both trees), `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=6` (`logs/build_parent.sh`,
`logs/build_tip.sh`, `bin/COMMIT.txt`). Zone tier `dev`. The binaries were re-verified against
`bin/SHA256SUMS` before every block and after the window (`raw/window_state.json`).

| row | binaries | args | W | steps (window) | armed | expected pose (the C0 tester's) |
|---|---|---|---|---|---|---|
| `J-As` | parent, tip | `--scene jolt --gap 0.5 --cfg as` | 1, 2, 4, 8, 16 | 500 (0..500) | no | `0x32d5e235342b4143` |
| `J-A` | parent, tip | `--scene jolt --gap 0.5 --cfg a` | 1, 2, 4, 8, 16 | 500 (0..500) | no | `0x32d5e235342b4143` |
| `R` | parent, tip | `--scene rest --solver colored` (+ `--parallel-solve` at W=8, as P0's row spelled it — a no-op since L4) | 1, 8 | 1100 (600..1100) | yes | `0x87e561d20589d4a5` |
| `R-S` | parent, tip | `--scene rest --sleeping --frozen-by 300` | 1, 8 | 800 (300..800) | yes | `0x2a2b7926a48aab00` |
| `S16` | parent, tip | `--scene s16 --cfg a` | 1 | 300 (0..300) | yes | `0x8877dbb1192e9b92` |
| `J-As-a` | parent, tip | `--scene jolt --gap 0.5 --cfg as --arm-profiler` | 1, 8 | 500 (0..500) | yes | `0x32d5e235342b4143` |
| `J-C` | parent, tip (K = 1 each) | `J-As-a` + `--canary-frac 0.05 --canary-ref-ns <that binary's latest J-As-a W1 window mean>` | 1 | 500 (0..500) | yes | `0x32d5e235342b4143` |

`--cfg as` prints `simd_solve true`, `parallel_solve` / `parallel_broadphase` / `parallel_narrowphase` =
(W > 1), AllPairs under `broadphase_select` Manual, sleeping off; `--cfg a` the same with `simd_solve false`;
`R` and `R-S` print `PhysicsConfig::default()` (`parallel_solve true`, `parallel_narrowphase true`,
`parallel_broadphase false`; `R-S` `sleeping true`). The printed config is field-equal between the two
binaries in all 14 (row, W) pairs (`logs/04_pose_gate.log`). `--parallel-np` was not passed (as window 3's rows).

34 cells (K = 12 in every one) + the two K = 1 canary processes: 410 slots, 42 re-runs, 452 timed processes,
5 untimed warm-ups (457 records in `raw/runs.jsonl`), 0 non-zero exits, 0 invalid, 0 dropped slots. Every
used process prints its row's expected pose and `expect_pose: "match"` (410/410 in the used set; 452/452
timed); the poses, counters (J rows 4,524.246 manifolds per step over [0,500), 109.32 waves; R 6,662.3;
R-S 6,675 all frozen; S16 16) and structural fields (`void_steps` 0, `drops_total` 0, disarmed ring traffic
0, `pool_workers` = W, affinity mask `0xffff`) are identical on both binaries, as the lever's bit-identity
requires. Pose gate (untimed, `logs/04_pose_gate.log`, `gate/`): every row on the parent at its own step
count at W 1 and 8, the tip with `--expect-pose` against the parent's file in all 14 pairs, and a 501-step
red control on the tip (exit 4, `mismatch`, pose `0x9336b30a06a7d8af`) — the gate can fail.

**Contamination.** 499 distinct 5-s receipts: median 1.44 % busy, p90 4.81 %, max 12.35 %, **42 over 5 %**
(4 in main pass 0, 37 in main pass 1, 1 in armed pass 1, 0 in armed pass 0 and the canary), **0 with a build
process**; every hot original has a clean re-run. Main pass 1 (08:55–09:57) carried nearly all the
background (`claude.exe`, `Telegram.exe`, a browser); main pass 0 (08:20–08:52), the armed block
(09:59–10:09) and the canary (10:12) were quiet. Where a claim holds under IQR and SE but not under the
pooled min-max, that is the reason; pass 0 alone claims every `J-As` cell under all three readings
(`analysis.md` section 1). Idle rule: five waits, each reached after exactly three polls (`raw/wait_log.txt`).

## Layout

| Path | What it is |
|---|---|
| `analysis.md` | The reduction the queue block is copied from: selection and input checks, every cell and comparison under the three readings, the four G9 gates against the design's bars, the per-stage µs per manifold, the R / R-S / S16 facts, the bridge to window 3, the Jolt v5.6.0 reference, what cannot be claimed, the draft queue block, open points |
| `window_report.md` | The tester's report: what ran, the pose gate, receipts and contamination, cells, effects, per-stage spans, the sensitivity split, G9 arithmetic, facts, deletions |
| `rows.json` | The run list the window executed, with the protocol block, both binaries (exe, sha256, commit) and the expected pose per row |
| `bin/SHA256SUMS`, `bin/COMMIT.txt` | The two runners' sha256 and the commit, tree, target dir and `Compiling` line each was built from. The executables are not in the tree |
| `raw/runs.jsonl` | One record per process (457 = 5 warm-ups + 410 originals + 42 re-runs): args, exit, receipts before/after, the during-process witness, the window mean, pose / `expect_pose`, structural counters, the binary's hash |
| `raw/main-p{0,1}/`, `raw/armed-p{0,1}/`, `raw/canary-p0/` | One directory per process: `stdout.txt`, `stderr.txt`, the per-step `run.csv` (armed rows carry the zone columns) and `pose.bin` (one file per row across every W and both binaries: J `eff361e1…`, R `c133a50e…`, R-S `100975d7…`, S16 `43d18f22…`) |
| `raw/manifest.json`, `raw/window_state.json`, `raw/window_log.txt`, `raw/wait_log.txt`, `raw/WINDOW_DONE` | The window's manifest, its start/end state (power scheme, process count, the binary re-check), its log, the idle-rule poll log, the completion marker |
| `raw/analysis.json`, `raw/tables.md`, `raw/sensitivity.md` | The tester's reduction (`tools/reduce.py`), its rendered tables and the sensitivity split (during-process witness, per pass); `analysis.md` reproduces them independently |
| `analyst/reduction.json`, `analyst/tables_analyst.md`, `analyst/run.log` | The analyst's reduction (`tools/analyze_win4b.py`): every cell, comparison, per-process value and span, receipt figure, window 3's recomputed cells, the bridge, the Jolt reference |
| `gate/` | The untimed pose gate: `<row>_W{1,8}_parent.pose` (the files every timed process asserted against), one directory per gate run (parent and tip at W 1 / 8, and the 501-step red control `J-As_W1_tip_red501/`), `gate.json` |
| `logs/` | The builds (`01_build_parent.log`, `02_build_parent_tiplock.log`, `03_build_tip.log`, `build_parent.sh`, `build_tip.sh`), the pose gate (`04_pose_gate.log`), the window's stdout (`05_window.log`), the reduction's stderr (`06_reduce.err`, empty) |
| `tools/` | The scripts, copied (not imported in place): `window4b_run.py` + `wait_idle4b.ps1` (this window), `pose_gate.py` (the gate), `reduce.py` (the tester's reduction), `analyze_win4b.py` (the analyst's), `assemble.py` + `head.txt` / `mid.txt` / `tail.txt` (the tester's report assembly), and the window-3 originals they were adapted from (`window3_run.py`, `wait_idle3.ps1`, `analyze_win3.py`, `rows_win3_original.json`, `rows_l5_template_c{3,4}.json`, `build_l5.sh`, `wait_flag.py`, `render_tables.py`, `analyze_window.py`, `window.py`, `window_run.py`, `wait_idle.ps1`, `launch_after_idle.py`, `receipts_summary.py`, `driver.py`, `window_lib/pdhperf.py`, `lib/driver.py` = window 3's byte-identical copy of `tools/physics_parity/driver.py`) |

Not kept in the tree: the executables (hashes in `bin/SHA256SUMS`), the untimed rehearsals (`dry/`, `test/`),
the parent's exported tree and its target dir (deleted by absolute path after the window; the parent binary
survives only as the hash), and `__pycache__/`.

## How to re-run

1. Build the runners into `bin/` and hash them. The tip is this lane at `f8873aae`; the parent is
   `git archive 146a1125 | tar -x` into a scratch directory with the tip's `Cargo.lock` copied in
   (`logs/build_parent.sh`, `logs/build_tip.sh`: the build prefix, `--profile parity`, a cold
   `CARGO_TARGET_DIR` per tree, `cd` into the tree in the same command). Quote each build's
   `Compiling boyko-physics (path)` line — the parent binary must come from the exported tree, the tip from
   the lane. Copy the exes to `bin/runner_{parent,tip}.exe` and regenerate `bin/SHA256SUMS` and
   `bin/COMMIT.txt`. Never rebuild Jolt for a comparison against these rows; v5.6.0's window-3 cells are the
   reference.
2. Gate the poses: `python -B tools/pose_gate.py` runs every row on the parent at W 1 / 8, the tip with
   `--expect-pose` against the parent's file, and the 501-step red control; it writes `gate/`. The script
   carries the scratch directory as an absolute path (`SP`) — point it at this directory.
3. Run the window: `python -B tools/window4b_run.py` from anywhere (it resolves `rows.json`, `bin/`, `gate/`
   and `raw/` relative to its own directory). It waits for the idle rule (`tools/wait_idle4b.ps1`) before each
   block, verifies the binaries against `bin/SHA256SUMS`, and writes `raw/`. `WIN4B_TEST=1` is an untimed
   rehearsal under `test/`; `WIN4B_ARMED_ROUNDS` overrides the armed block's rounds per pass (6 here).
4. Reduce: `python -B tools/reduce.py [raw_dir]` writes `<raw_dir>/analysis.json` and prints the tables
   (`raw/tables.md`); `python -B tools/analyze_win4b.py` writes `analyst/` beside `raw/` — its `WIN3_RAW` is
   the absolute path of window 3's `raw/runs.jsonl` on the tree that holds it (`D:/wt/joltab/...` here);
   point it at a checkout of `2ce03b66`. Running them on this directory regenerates the files already here.
5. Quote nothing that is not in `raw/`. The machine must be quiet: the 5-s receipts gate each process, and a
   burst inside a process is visible only through the during-process witness (`others_busy_pct` in
   `runs.jsonl`), which this window reports and splits by pass (`raw/sensitivity.md`) but does not gate.
