# Physics perf campaign, window 3 — the measured default row (L4+L2, and L5) against the P0b bridge and Jolt (2026-09-21)

Receipts for `docs/MEASUREMENT-QUEUE.md` section 10, the `RESULT, 2026-09-21, window 3` block. Read
`analysis.md` first: it is the project-analyst's reduction, recomputed from `raw/` without the tester's
scripts, and it carries the block copied into the queue. `window_report.md` is the tester's own report of
the same window; `build_report.md` is the build stage (nothing timed there).

## What was measured

One window under the protocol ruled after P0 (queue section 10, "Protocol ruling"): a cell = the MEDIAN
over K separate processes of the [0,500) window mean (ms/step); spread = min-max, IQR, with the median's
SE beside them; a comparison is claimed iff |effect| > 2 * hypot(spread_A, spread_B), printed under all
three readings; 5-s load receipts before and after every process (> 5 % busy => that process re-run once
at the end of its pass), a 10-s receipt and three quiet 60-s polls before each pass; no band-based void;
placement P-none. Two passes x three rounds, Jolt and boyko alternating inside a W group, pass 1 reversed,
one untimed warm-up per pass. Timed 03:50:08-04:42:16 +03:00 on the owner's workstation (Ryzen 9 5900HS,
8C/16T, High performance, AC), after both lanes that were running had finished (`lanes_done.flag`,
`wait_log.txt`).

| row | binary (sha256 prefix, `bin/SHA256SUMS`) | tree | args | W |
|---|---|---|---|---|
| `D-L4L2` | `runner_l4l2.exe` `368d4104` | `aac562a7` (= L2 = C2; L4 `caac7d06` + L2 are the only code commits above the P0 tip `dbd85977`) | `--scene jolt --gap 0.5 --cfg default` | 1, 2, 4, 8, 16 |
| `D-L5` | `runner_l5.exe` `26d17a10` | `de06b6c9` (L5 C4, on top of C3 `b8d9ab8f`) | `--scene jolt --gap 0.5 --cfg default` | 1, 2, 4, 8, 16 |
| `D-L5-npoff` | `runner_l5.exe` `26d17a10` | `de06b6c9` | the same plus `--parallel-np off` (same-binary A/B) | 8 |
| `J-A` | `runner_tip.exe` `ef9325ef` (P0's binary, re-used) | `dbd85977` (the P0 tip) | `--scene jolt --gap 0.5 --cfg a` | 1, 8 (the bridge to P0b) |
| `JOLT-T` | Jolt v5.3.0 Distribution `29b23ad1` (P0's build, not rebuilt) | - | `-s=Pyramid -q=Discrete -f` | 1, 2, 4, 8, 16 |
| `JOLT56-T` | Jolt v5.6.0 Distribution `918fd2b7` (P0's build, not rebuilt) | - | `-s=Pyramid -q=Discrete -f` | 1, 2, 4, 8, 16 |

23 cells, 138 slots, 31 re-runs; K = 6 in 22 cells and K = 5 in `D-L4L2@W1` (both attempts of one slot
were contaminated). `--cfg default` at both boyko trees is `PhysicsConfig::default()` as shipped: colored,
`simd_solve` on, `parallel_solve` on, AllPairs under `broadphase_select: Manual`, sleeping off; at
`de06b6c9` also `parallel_narrowphase` on. Every boyko process reports the one 500-step pose
`0x32d5e235342b4143`; Jolt one hash per exe. Two untimed Jolt `-receipt` runs (`raw/receipts/`) give
Jolt's own manifold counts, v5.6.0's for the first time.

The boyko runners were built with cargo profile `parity` (inherits `release`: fat LTO, default CGU), rustc
1.98.1 `x86_64-pc-windows-msvc`, `RUSTFLAGS` unset, `CARGO_INCREMENTAL=0`, from detached worktrees
`D:/wt/mq-<sha>` into target dirs `D:/wt/_targets/mq-<sha>` (`logs/04_build_runner_l4l2.log`,
`logs/13_build_runner_l5.log`, `logs/build_l4l2.sh`, `tools/build_l5.sh`).

## Layout

| Path | What it is |
|---|---|
| `analysis.md` | The reduction the queue block is copied from: cells, spreads, every comparison under the three readings, the bridge to P0b, the L4+L5 gate, the owner's W=1 question, open points |
| `window_report.md` | The tester's report of the window: what ran, receipts and contamination, comparisons, deviations |
| `build_report.md` | The build stage (`runner_l4l2`, the tree, what `--cfg default` prints, the untimed dry runs); nothing timed |
| `rows.json`, `rows_l5.json` | The run list the window executed, with the protocol block and the expected poses; `rows_l5.json` is the C4 block, filled once `de06b6c9` landed |
| `bin/SHA256SUMS` | The five binaries' sha256 (three boyko runners by name, the two Jolt exes by path) |
| `raw/runs.jsonl` | One record per process (171 = 2 warm-ups + 138 originals + 31 re-runs): args, exit, receipts before/after, the during-process witness, the window mean, pose/hash, structural counters |
| `raw/pass-00/`, `raw/pass-01/` | One directory per process: `stdout.txt`, `stderr.txt`, the per-step CSV (`run.csv` for boyko, `per_frame_discrete_thW.csv` for Jolt) and, for boyko, `pose.bin` (all 98 are one identical file, sha256 `eff361e1...`) |
| `raw/receipts/` | The two untimed Jolt `-receipt` runs and `receipts.json` |
| `raw/manifest.json`, `raw/window_state.json`, `raw/window_log.txt`, `raw/wait_log.txt`, `raw/WINDOW_DONE` | The window's manifest, its start/end state (power scheme, process count, binary re-check), its log, the idle-rule poll log, the completion marker |
| `raw/analysis.json`, `raw/tables.md` | The tester's reduction (`tools/analyze_win3.py`) and its rendered tables (`tools/render_tables.py`); `analysis.md` reproduces them independently |
| `wait_log.txt`, `lanes_done.flag` | The lane-completion poll (step A) and the flag it waited for |
| `logs/` | The build and gate logs: worktree, ancestry, defaults grep, builds, usage text, config summaries, the 500-step pose gates with their red controls, the Jolt dry runs, the exe diff |
| `tools/` | The scripts, copied (not imported in place): `window3_run.py` + `wait_idle3.ps1` + `wait_flag.py` (this window), `analyze_win3.py` + `render_tables.py` (its reduction), `build_l5.sh`, and the P0/P0b originals they were adapted from (`window_run.py`, `analyze_window.py`, `window.py`, `wait_idle.ps1`, `launch_after_idle.py`, `receipts_summary.py`, `window_lib/pdhperf.py`, `driver.py` = `lib/driver.py`, a byte-identical copy of `tools/physics_parity/driver.py`) |

Not kept in the tree: the executables (hashes in `bin/SHA256SUMS`), the untimed rehearsals (`dry/`,
`test/`), and the analyst's own reduction scripts (`analysis.md` names them under its scratch directory;
every number it quotes is in `raw/`).

## How to re-run

1. Build the runners into `bin/` and hash them. For a commit `<sha8>` that exists in some checkout:
   `tools/build_l5.sh <sha8>` creates a detached worktree `D:/wt/mq-<sha8>` and builds
   `cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid` into
   `D:/wt/_targets/mq-<sha8>` (the script takes the worktree from `D:/wt/l5np`; point it at any checkout that
   has the commit). Copy the exe to `bin/runner_<name>.exe` and regenerate `bin/SHA256SUMS`. The Jolt exes are
   P0's builds (`docs/measurements/2026-09-19-physics-p0/p0/build_report.md`); never rebuild them for a
   comparison against these rows.
2. Check the rows: `rows.json` (and `rows_l5.json`) name each exe, its sha256, its commit, args, the W list and
   the expected 500-step pose. Before any timed process, run the pose gate once per boyko runner at W 1/8/16
   with `--expect-pose` against a reference pose and a 501-step red control (`logs/15_l5_pose_500_gate.log`).
3. Run the window: `python -B tools/window3_run.py` from anywhere (the script resolves `rows.json`, `bin/` and
   `raw/` relative to its own directory; it refuses to start if `tools/lib/driver.py` differs from
   `tools/physics_parity/driver.py` at the path in `TREE_DRIVER`). It waits for the idle rule
   (`tools/wait_idle3.ps1`: no build process, no process from `D:/wt/_targets` or `D:/wt/mq-*`, 10-s CPU
   < 5 %, three consecutive quiet polls) before each pass, verifies the binaries against `bin/SHA256SUMS`, and
   writes `raw/`. `WIN3_TEST=1` is an untimed rehearsal under `test/`.
4. Reduce: `python -B tools/analyze_win3.py [raw_dir]` writes `<raw_dir>/analysis.json`;
   `python -B tools/render_tables.py [raw_dir] > tables.md` renders it. Running them on this directory's
   `raw/` regenerates the two files that are already there.
5. Quote nothing that is not in `raw/`. The machine must be quiet: the 5-s receipts gate each process, and a
   burst inside a process is only visible through the during-process witness (`others_busy_pct` in
   `runs.jsonl`), which this window reports but does not gate (`analysis.md` section 6, point 3).
