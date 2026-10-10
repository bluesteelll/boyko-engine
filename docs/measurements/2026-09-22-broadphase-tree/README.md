# Physics perf campaign, window 4 — the Tree broadphase (C3): G4 bench and build-if, G5 end to end, against AllPairs and Jolt v5.6.0 (2026-09-22)

Receipts for `docs/MEASUREMENT-QUEUE.md` section 11, the `RESULT, 2026-09-22, window 4` block, and for the
2026-09-22 ruling line in `docs/physics/perf-campaign/levers/00-RULINGS.md` (the broadphase rev 2 block). Read
`analysis.md` first: it is the results-analyst's reduction, recomputed from `raw/` by its own script
(`tools/analyze_win4.py`), and it carries the block copied into the queue. `window_report.md` is the tester's
own report of the same window (builds, the pose gate, the run record, the tester's reduction); nothing in it is
quoted by the queue block except where `analysis.md` reproduces it.

## What was measured

The tree broadphase's G4 and G5 gates from `docs/physics/perf-campaign/levers/broadphase/04-DESIGN-REV2.md`,
run to the C3 developer's recipe (`g4_g5_recipe.md`, binding; the tester's report cites it by section), on one
binary tree: lane `D:/wt/mq-de06b6c9`, branch `perf/physics-tree-broadphase` at **`a46b8287`** (= C1 `ecbfe416`
+ C3). The integration line `merge/ke16-into-ecsnative` carries this tree broadphase at `c52ad183`: the diff
`a46b8287..c52ad183` in `crates/boyko_physics/src` is 10 files, +381/−158 — the A1b `RowIdentity` merge —
and restricted to `src/broadphase_tree/` it is `tests.rs` only, +21/−5 (`logs/01_joltab_physics_diff.txt`); the
line's later commit `71e76228` touched `CLAUDE.md` only. The timed binaries are the lane's, not the line's.

- **G4** (the criterion benches, cargo `bench` profile: lto off, CGU 1): `bp_g4_uniform`, `bp_g4_disparity`,
  `bp_g4_scene` (n ∈ {17, 64, 128, 256, 1k, 10k, 100k}; the scene family n ∈ {1240, j100, 10k, 100k}) × arms
  `all_pairs`, `grid_w1`, `grid_w8`, `tree`; `bp_g4_maintenance` (m ∈ {1240, 10k, 100k} × 7 arms); K = 3
  whole-group processes, groups interleaved u d s m. `row_identity_churn`: 6 arms × sleeping {off, on} ×
  broadphase {allpairs, tree} = 24 cells, K = 6, one cell per process, allpairs/tree alternating, order reversed
  on odd k. 156 processes + 10 protocol re-runs (`k<k>r`), 117 cells × K, 0 excluded slots. Timed
  03:48:22–07:54:32 +03:00.
- **G5** (the parity runner, cargo `parity` profile: inherits `release`, fat LTO, default CGU, zone tier `dev`):
  11 rows, 26 cells, K = 6 — `T-A-tree` / `T-A-allpairs` (`--cfg a`, W 1/2/4/8/16), `T-D-tree` / `T-D-allpairs`
  (`--cfg default`, W 1/8), `R-tree` / `R-allpairs` (`--scene rest`, W 1/8), `S16-tree` / `S16-allpairs` (W 1),
  `T-A-tree-armed` / `T-A-allpairs-armed` (`--arm-profiler`, W 1/8), `T-C-tree` (the canary, `--canary-frac 0.05`,
  `canary_of: T-A-tree`, W 1/8). `--broadphase` is the same-binary A/B. 156 timed processes + 2 untimed warm-ups
  + 1 re-run (one invalid pose, below), 0 contaminated, 0 excluded slots. Timed 10:16:16–10:44:10 +03:00.

Owner's workstation (Ryzen 9 5900HS, 8C/16T, High performance, AC), rustc 1.98.1 `x86_64-pc-windows-msvc`,
`RUSTFLAGS` unset, `CARGO_INCREMENTAL=0`, `-C target-cpu=x86-64-v3` (the `[target]` baseline), built from the
lane's tree (`Compiling boyko-physics v0.1.0 (D:\wt\mq-de06b6c9\crates\boyko_physics)` in both build logs — the
lane's, not the caller's).

### The protocol block (window 3's, verbatim; the recipe's 1.1 and 2.1)

A cell = the MEDIAN over K separate processes (G5: the [0,500) window mean, ms/step; G4: criterion's
`median.point_estimate`); spread = min-max range and IQR, with the median's SE (1.2533·SD/√K) beside them; a
comparison is claimed iff |effect| > 2·hypot(spread_A, spread_B), printed under all three readings (r / i / s);
5-s load receipts before and after every process (> 5 % busy ⇒ that process re-run once at the end of its pass or
block; the original kept), a during-process witness (`others_busy_pct`) reported but not gated; a 10-s receipt
and three consecutive quiet 60-s polls before each pass (the idle rule: 0 build processes, 0 processes from
`D:\wt\_targets` / `D:\wt\mq-*`, cpu10 < 5 %); no band-based void; placement P-none (mask `0xffff`). G5: two
passes × three rounds, tree/allpairs alternating inside a W group, pass 1 reversed, one untimed warm-up per pass
(`T-D-tree` W=8); the binaries verified against `bin/SHA256SUMS` before each pass and after the window. The P0
driver `tools/physics_parity/driver.py` (a byte-identical copy at `tools/lib/driver.py`) runs each process and
writes the record; `tools/window4_run.py` is window 3's `window3_run.py` with the edits `tools/window4_run.diff`
lists (the tree driver path, the `canary_of` field threaded through, the warm-up row, a per-call 90-minute idle
budget); `tools/g4_run.py` is the G4 driver (same receipts, witness and idle rule, `CRITERION_HOME = raw/g4`).

### The idle receipts

| stage | receipt | what it saw |
|---|---|---|
| first launch 02:05–03:34 | `wait_log.txt` polls 1–89 | never idle: other lanes' `cargo`/`rustc`/`clippy-driver` bursts and `python` reducers; TIMEOUT, nothing timed; the untimed preparation (builds, pose gate, rehearsals) was done meanwhile |
| G4 standalone 03:40:39–03:42:49 | `wait_log.txt` after `# ---- RESUME 03:40` | IDLE after 3 polls: cpu10 2.18 / 4.64 / 1.26 %, 0 build, 0 lane-target processes |
| G4 driver | `raw/g4/wait_log.txt` | before block A: 6 polls (poll 3 = 6.27 %, the Claude desktop app) → idle 03:48:11; before A's re-run 3 polls → 05:27:11; before block B 3 polls → 05:38:11; before B's re-runs 3 polls → 07:47:03; no build process on any poll |
| G5 standalone 08:04:35–08:17:46 | `wait_log.txt` after `# ---- RESUME 08:04:35` | IDLE after 14 polls (polls 12–14: 3.33 / 2.19 / 3.11 %); poll 3 saw 8 build processes (54.76 %), polls 9–11 another window's parity runners |
| G5 first launch 08:19:13 | `raw/launch1_aborted_0819/` | STOPPED by the tester at 08:29:54 during its idle wait — a second timed window (`scratchpad/win4b`, 360 processes, runners under the scratchpad and so invisible to the lane-target rule) was running; nothing timed; waited for its `WINDOW_DONE` (10:12:24) |
| G5 relaunch 10:13:54 | `raw/wait_log.txt`, `raw/window_log.txt` | pass 0: idle after 3 polls (2.06 / 1.20 / 0.95 %) at 10:16:05, opening 10-s receipt 1.18 %; pass 1: 3 polls (1.83 / 1.65 / 1.38 %) at 10:31:07, opening receipt 0.69 %; `window_state.json`: High performance, 245 → 241 processes, `binaries_after: all match`, `status: complete` |

Load receipts: G5 — 314 5-s receipts, median 1.00 %, p90 1.84 %, max 3.97 %, **0 over 5 %**, 0 with a build
process; witness over the 156 used: median 0.29 %, max 6.49 % (one process, `R-allpairs` W=8 pass 1 seq 6, a
`python.exe` burst inside its 3.75-s wall; bracketing receipts 0.90 / 0.78 %, so the protocol used it — it is
that cell's maximum). G4 — 332 receipts, median 0.61 %, p90 1.79 %, max 9.54 %, 10 after-receipts over 5 %
(the Claude desktop app's window, visible until 05:57:55; every one re-run clean as `k<k>r`), 0 build processes.
The tester's two operational deviations (the aborted 08:19 launch; the desktop app minimized and restored) left
no timed process in doubt (`analysis.md` section 9, point 6).

### The binaries

| name | sha256 (`bin/SHA256SUMS`) | tree | profile | used for |
|---|---|---|---|---|
| `runner_c3.exe` (= `jolt_parity_pyramid-39550833a2ead4ba.exe`) | `19456ab4…` | `a46b8287` (`bin/COMMIT.txt`) | `parity` | every G5 row; the pose gate |
| `broadphase-595045019352a67f.exe` | `efb84636…` | `a46b8287` | `bench` | G4 pair finding, scene, maintenance |
| `row_identity_churn-ec1df0bd330016d8.exe` | `d8ba73fc…` | `a46b8287` | `bench` | G4 churn |
| Jolt v5.6.0 `PerformanceTest.exe` (Distribution) | `918fd2b7…` (window 3's `bin/SHA256SUMS`) | — | — | **not run**: its window-3 cells are reused (below) |

The exes are not in the tree (the root `.gitignore` does not exclude `*.exe`, so they were not copied rather than
ignored); the hashes are. Every G5 record carries the runner's sha256 in its SUMMARY (`19456ab4…` × 159), and the
G4 driver logged both bench digests before timing (`raw/g4/g4_log.txt` line 1).

### Reused from window 3, and why

- **Jolt v5.6.0's cells** (W 1/2/4/8/16: 9.828, 5.770, 3.581, 2.569, 2.388 ms; K = 6) are recomputed by
  `tools/analyze_win4.py` from `docs/measurements/2026-09-21-physics-window3/raw/runs.jsonl` with the same
  selection rule, never re-run — the owner's ruling of 2026-09-21 makes v5.6.0 the only reference and the recipe
  says "reused, never re-run"; no v5.3.0 row exists anywhere in this window. Jolt's manifold count (8,489.0 per
  step over [100,500)) is read from window 3's untimed `-receipt` CSV
  (`.../raw/receipts/JOLT56-T_receipt_W1/receipt_discrete_th1.csv`). `raw/receipts/` here is therefore empty
  of Jolt rows.
- **Window 3's `D-L5` cells** (`de06b6c9`, W1 10.738 / W8 5.347 ms) are the bridge: `T-D-allpairs` on this
  binary is `D-L5` re-taken (same physics code plus the C3 commit), and the gate "no claimed shift" is read on it
  (`analysis.md` section 2.2).
- **The expected pose** `0x32d5e235342b4143` for every `--cfg default` J row is window 3's `D-L5` hash, and the
  `--cfg a` rows hash the same 500-step pose (the pose gate, `logs/04_pose_gate.txt`; the recipe expected a
  separate cfg-A hash, which held only for its 20-step dry run). R `0xee2a67a98434919a`, S16
  `0x71313833f6a8e645`. One `pose.bin` per scene across all 158 valid processes (sha256 `eff361e1…` J,
  `89f084c2…` R, `82c192b5…` S16; `logs/04_pose_files_sha256.txt`).
- **The tools**: `window3_run.py`, `wait_idle3.ps1`, `wait_flag.py`, `analyze_win3.py`, `render_tables.py`,
  `lib/driver.py`, `window_lib/pdhperf.py` are window 3's, copied unmodified; `window4_run.py` and `g4_run.py`
  are the adaptations named above.

### The one invalid process

`raw/pass-01/075_r2_T-D-allpairs_W1/` (`--cfg default --broadphase allpairs --workers 1`, exit 0, clean receipts)
hashed `0xf6e397d168e0e8b8` against the row's `0x32d5e235342b4143`; its per-step CSV is identical to its twins
through step 438 and diverges at step 439 in `top_y` (pairs never differ). The protocol re-ran it
(`..._rerun`, valid) and the re-run is the used process. It is a solver-side W=1 determinism event on the
shipped default's one-worker-pool path, not the tree's (`analysis.md` section 9.1), and it is left in `raw/` as
found.

## Layout

| Path | What it is |
|---|---|
| `analysis.md` | The results-analyst's reduction, verbatim: the headline against Jolt v5.6.0, every G5 row under the three readings, the armed spans and Δbp, the canary, the G4 tables, the stop rules and gates with their arithmetic, what cannot be claimed, the C4 constants, the D6 decision, the investigation list, open questions |
| `window_report.md` | The tester's report: builds, the pose gate, the tools, the untimed rehearsals, the G4 and G5 run records with receipts, the tester's own reduction (which `analysis.md` reproduces independently) |
| `rows.json` | The G5 run list the window executed, with the protocol block, the expected poses and the `expect_pose_of` / `canary_of` twins |
| `wait_log.txt` | The standalone idle receipts (the first launch's 89 polls, the 03:40 and 08:04 receipts) and the pointers to the drivers' own waits |
| `bin/SHA256SUMS`, `bin/COMMIT.txt` | The three binaries' sha256 and the commit with the two `Compiling boyko-physics (D:\wt\mq-de06b6c9\…)` receipt lines |
| `logs/` | Disk, commit, ancestry and the integration-line diff (`00`, `01_*`), the two builds (`02`, `03`), the pose gate summary and pose sha256s (`04_*`), the drivers' stdout (`05`, `06`), the UI-host CPU probes (`08`, `11`), the tester's reductions (`09`, `10`, `13`) and the staged G5 section (`14`) |
| `gate/` | The pose gate: 28 processes' stdout / stderr / exit code (`*.out`, `*.err`, `*.rc`) and the four reference pose files it wrote |
| `raw/runs.jsonl` | One record per G5 process (159 = 2 warm-ups + 156 originals + 1 re-run): args, exit, receipts before / after, the during-process witness, the window mean, pose hash, `expect_pose`, TreeDiag counters, the runner's SUMMARY |
| `raw/pass-00/`, `raw/pass-01/` | One directory per process: `stdout.txt`, `stderr.txt`, the per-step `run.csv` (the window means, the sub-windows and, on the armed and canary rows, the `phys_bp_*` and `sys_*` spans are read from it) and `pose.bin` |
| `raw/g5_reduction.json`, `raw/manifest.json`, `raw/window_state.json`, `raw/window_log.txt`, `raw/wait_log.txt`, `raw/WINDOW_DONE`, `raw/receipts/receipts.json`, `raw/launch1_aborted_0819/` | The tester's G5 reduction, the manifest, the start/end state, the driver's log and idle-rule polls, the completion marker, the (empty) Jolt receipt index, and the aborted 08:19 launch's artefacts |
| `raw/g4/runs.jsonl` | One record per G4 process (166 = 156 originals + 10 re-runs): the command, the baseline name, exit, receipts, the witness, the structural receipt lines parsed from stderr |
| `raw/g4/<group>/<arm>/<param>/k<k>[r]/estimates.json` | criterion's estimates per cell and process (460 files; `median.point_estimate` is the cell input) |
| `raw/g4/logs/` | stdout + stderr of every G4 process (332 files; the stderr carries the structural receipts — pairs, members, rebuilds, the `(oracle ok)` lines) |
| `raw/g4/g4_log.txt`, `raw/g4/wait_log.txt`, `raw/g4/g4_reduction.json`, `raw/g4/G4_DONE`, `raw/g4/stop1_0334/` | The G4 driver's log and idle polls, the tester's G4 reduction, the completion marker, and the first launch's stop artefacts (03:34, nothing timed) |
| `analyst/reduction.json`, `analyst/tables.txt` | The analyst's script output: every cell, per-process value, span, comparison, gate, constant and receipt figure, and the printed tables |
| `tools/` | The scripts as run: `g4_run.py`, `window4_run.py` (+ `window4_run.diff` against `window3_run.py`), `pose_gate.sh`, `claude_ui_cpu.ps1`, `reduce_g4.py`, `reduce_g5.py`, `_rehearsal_list.py` (this window); `analyze_win4.py` (the analyst's reduction); window 3's copies; `record_copy.py` (what this directory keeps and drops) |

Not kept in the tree: the executables (hashes in `bin/SHA256SUMS`), the untimed rehearsals (`test/`,
`rehearsal/`), the drivers' pid files and a `.bak` of the report, criterion's `benchmark.json` / `sample.json`
/ `tukey.json`, and criterion's `new/` directory per cell (117 of them, each verified byte-identical to the
last-run `k` baseline of its cell before being dropped — `tools/record_copy.py`). Every number quoted in
`analysis.md` is in `raw/`, in window 3's `raw/`, or in `logs/`.

## How to re-run

1. Build the binaries and hash them. From a checkout at `a46b8287` (or a tree with the same
   `crates/boyko_physics`), with the lane prefix of the recipe (`PATH="$HOME/.cargo/bin:$PATH"
   RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_TARGET_DIR=<dir> CARGO_INCREMENTAL=0`, never
   `RUSTFLAGS`): `cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid` (copy the exe
   to `bin/runner_c3.exe`) and `cargo bench --no-run -p boyko-physics --bench broadphase --bench
   row_identity_churn` (run by path from `<dir>/release/deps/`). Quote the `Compiling boyko-physics (<path>)` line
   and regenerate `bin/SHA256SUMS`. Do not rebuild Jolt: its cells are window 3's.
2. Gate before timing: `tools/pose_gate.sh` (28 untimed processes: one `--pose-out` per scene and cfg, `--expect-pose`
   at W 1/8/16 for tree, allpairs and grid, a 501-step red control per scene that must exit 4, the usage-text and
   self-check controls); compare with `logs/04_pose_gate.txt`.
3. Run G4: `python -B tools/g4_run.py` (waits for the idle rule before each block and each block's re-runs; writes
   `raw/g4/`; `G4_TEST=1` is an untimed rehearsal). Then G5: `python -B tools/window4_run.py` (resolves `rows.json`,
   `bin/` and `raw/` relative to its own directory; refuses to start if `tools/lib/driver.py` differs from the tree's
   `tools/physics_parity/driver.py` at `TREE_DRIVER`; `WIN4_TEST=1` is an untimed rehearsal). Minimize the Claude
   desktop app first (`powershell -File tools/claude_ui_cpu.ps1 -Minimize`) — it was this window's only contaminant.
4. Reduce: `python -B tools/analyze_win4.py` from this directory regenerates `analyst/reduction.json` byte for byte
   (verified when this directory was recorded; `analyst/tables.txt` differs only by the line that hashes the absent
   `bin/runner_c3.exe`). It reads window 3's `raw/runs.jsonl` by absolute path (`WIN3_RUNS`). The tester's
   `tools/reduce_g4.py` / `tools/reduce_g5.py` regenerate `raw/g4/g4_reduction.json` / `raw/g5_reduction.json`.
5. Quote nothing that is not in `raw/`. The machine must be quiet: the idle rule cannot see a timed window whose
   runners live outside `D:\wt\_targets` / `D:\wt\mq-*` (the 08:19 event above), and the 5-s receipts cannot see a
   burst shorter than a process (the `R-allpairs` W=8 witness) — both are protocol questions `analysis.md`
   section 9 raises, not gates.
