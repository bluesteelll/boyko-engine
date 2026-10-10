# Physics perf campaign, window 7 wave 2 — the W8 headline over four blocks, C3b's t_q and C2, the tree thresholds between 64 and 256, and G-TW's canary resolution (2026-09-25)

This directory holds the receipts for:
- `docs/MEASUREMENT-QUEUE.md` section 15.5, the `RESULT, 2026-09-25, window 7 wave 2` block;
- four dated addenda of 2026-09-25 (wave 2) in `docs/physics/perf-campaign/levers/00-RULINGS.md`, each appended to
  the paragraph it amends, so no line of that file moves:
  - the tree-broadphase block, the G4/G5 paragraph: **the tree thresholds**;
  - the tree-broadphase block, the C3b paragraph: **C2 is not built**;
  - the L9 block, the G-TW bullet: **G-TW's canary resolution is demonstrated**;
  - the per-contact parity block, the headline bullet: **the W8 headline's reading**.

It is a subdirectory of window 7's record because its Q1 pools window 7's blocks A and B (`../raw/`,
`P1A-jolt` and `P1B-jolt`) with this run's blocks C and D, and because it reuses window 7's binaries and fixtures in
place.

**Read `analysis.md` first** — the results-analyst's reduction, recomputed from `raw/` with its own script
(`tools/analyze7b.py`), not from the driver's `tools/reduce7b.py`. The orchestrator's four rulings are quoted in it and
applied. The plan the run executed (every item's source, rows, bars and rule) is `plan.md`.

**`analysis.md` may not be in this commit.** As in windows 6 and 7, the harness blocks report files for the analyst's
role, and the recording step (the same agent) could not write it either. If it is absent here, the orchestrator adds
it in the next commit, verbatim from the text the analyst returned (its first line is the one-line verdict string).
The analyst's tables, `analyst/tables.txt` (byte-identical to `analyst_run.log`) and `analyst/reduction.json`, are in
this commit and carry every number `analysis.md` quotes.

The design notes the plan and the analysis cite by scratch path — `c3b/design.md`, `treebp/g4_g5_recipe.md` — are the
orchestrator's working files under
`C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad/`
(window 6's `README.md` says the same); they are not in the tree. The earlier windows it reads as context are
`docs/measurements/2026-09-24-physics-window6/` (C3b's cells) and `docs/measurements/2026-09-22-broadphase-tree/` (the
G4 grid, queue section 11).

## What was measured

Four items, run as six blocks in this order: Q1C, Q2C, Q3, Q4, Q2D, Q1D (`plan.md` § 4; `rows7b.json` holds the rows).

| item | question | blocks | binaries |
|---|---|---|---|
| Q1 | The W8 headline over four separated blocks: our default row against Jolt v5.6.0 at W 4/8/16. HOLDS iff claimed in A, B, C, D and pooled over A-D, same direction | Q1C (first), Q1D (last); A, B = window 7's P1A-jolt, P1B-jolt | trk, j56 |
| Q2 | C3b's t_q block-shift test (window 6's +11.96 %): a layout term (window 6 P5g's launch context) against a block term (C vs D); and C2's letter (t_q(J, W=1) ≥ 0.235 ms on the default row), read on the default row and on window 6's cfg-A row | Q2C, Q2D | par, tip |
| Q3 | The tree thresholds between 64 and 256 (the recipe's refinement run, `g4_g5_recipe.md:131-132`) | Q3 | g4r |
| Q4 | G-TW's canary resolution: four canary rungs at 0.5 / 1 / 1.5 / 2 × G-TW's own J-A W1 bar R = 7.542 % | Q4 | c4 |

| row | block | binaries | args (+ `--workers W --steps 500 --window 0..500 --csv --pose-out --label --expect-pose <fixture>`; Jolt: `-t=W -i=500`) | W | armed | pose |
|---|---|---|---|---|---|---|
| `H-trk-tree` | Q1C, Q1D | trk | `--scene jolt --gap 0.5 --cfg default --broadphase tree` | 4, 8, 16 | no | J500 |
| `H-jolt56` | Q1C, Q1D | j56 | `-s=Pyramid -q=Discrete -f` | 4, 8, 16 | — | hash `0xb8522b4e3fc62cfe` |
| `H-trk-ap` | Q1C, Q1D | trk | `--scene jolt --gap 0.5 --cfg default --broadphase allpairs` | 4, 8, 16 | no | J500 |
| `C3b-TA-armed` | Q2C, Q2D | par, tip | `--scene jolt --gap 0.5 --cfg a --broadphase tree` (window 6 P3's layout: cwd 163, 648 characters) | 1, 8 | yes | J500 |
| `C3b-TA-pad06` | Q2C, Q2D | par, tip | the same (window 6 P5g's layout: cwd 166, 654 characters) | 1 | yes | J500 |
| `C3b-TD-armed` | Q2C, Q2D | par, tip | `--scene jolt --gap 0.5 --cfg default --broadphase tree` (C2's letter row) | 1 | yes | J500 |
| `G4-refine` | Q3 | g4r | `--bench --noplot '^bp_g4_(uniform\|disparity)/((all_pairs\|tree)/(64\|96\|112\|128\|136\|144\|152\|160\|168\|176\|192\|256)\|tree_rowwalk/(64\|128\|256))$'` (54 benchmarks) | — | — | — |
| `L9-JA-on` | Q4 | c4 | `--scene jolt --gap 0.5 --cfg a --contact-reuse on` (G-TW's own `on` row, the reference) | 1 | no | JAon500c4 |
| `L9-JC-r050` … `r200` | Q4 | c4 | the same + `--canary-frac 0.036 / 0.071 / 0.107 / 0.142 --canary-ref-ns <the latest valid L9-JA-on window mean>` | 1 | no | JAon500c4 |

Q2's metric window is [100,500) of `phys_bp_query_ns` (the per-process median); Q1's and Q4's is the process mean of
`wall_ns` over [0,500) (Jolt: of `Time (ms)`), with [0,100) and [100,500) beside it.

**Protocol** (`plan.md` § 1, `rows7b.json` → `protocol`): window 7's, verbatim, with two additions.
- **Cell and claim rule.** A cell is the MEDIAN over K separate processes of each process's statistic. Spread: r =
  min–max and i = IQR, with the median's SE s = 1.2533·SD/√K beside them, each relative to the median. B against A is
  claimed iff |B/A − 1| > 2·√(sA² + sB²) under BOTH r and s; i is printed. No claim when either side has K < 3.
  Against a constant bar b (Q2's 0.235 / 0.21 / 0.186 ms): claimed iff |m/b − 1| > 2r AND > 2s of the cell.
- **K and ordering.** K = 6 for Q1, Q2 and Q4: two passes × three rounds. Pass 0 orders the cells by W ascending, row
  by row inside a W group with each row's binaries in their listed order; pass 1 is the whole list reversed; each pass
  opens with one untimed warm-up. **Q3: K = 3** (the recipe's K), one pass × three rounds, no warm-up (criterion warms
  each benchmark: 3 s warm-up, 5 s, 10 flat samples), `CRITERION_HOME` per process.
- **Idle rule before EVERY pass** (`tools/wait_idle7.ps1`, window 7's verbatim): three consecutive 60-s polls with no
  build or toolchain process, no process under `D:/wt/_targets` or `D:/wt/mq-*`, and a 10-s CPU below 5 %; up to 30
  polls; a timeout stops the run (exit 3).
- **Receipts, re-run, void.** A 10-s receipt opens each pass and a 5-s receipt sits between processes. A process whose
  receipt before or after reads above 5 %, or that is invalid, is re-run ONCE at the end of its pass. A build or lane
  process at any receipt or during a process voids the pass.
- **Launch** P-none: suspended, the affinity mask read back, resumed; no affinity call.
- **New: launch-context lengths.** A row may pin its process-directory length per W; the driver pads the leaf with
  `x` so the cwd, and with it the `--csv` / `--pose-out` arguments, have exactly the pinned length. Q1 pins window 7's
  A/B lengths (163/161/161 at W 4/8, 164/162/162 at W16); Q2 pins window 6's P3 (163) and P5g (166). Every record
  keeps `cwd_len_target`, `cwd_len_actual`, `cwd_len_miss` and `cmdline_chars`. 0 misses in this run.
- **New: priority skip and a STOP flag.** Before a block, the driver reserves the estimated time of every later block
  of a higher priority (Q1 > Q2 > Q3 > Q4) against the cutoff (11:45), and skips the block if it does not fit. The
  flag file `STOP` stops the driver before its next process. Neither fired.
- **Validity on every runner process:** exit 0 and `void_steps` 0; `expect_pose: match` against the row's fixture and
  the hash equal to it; `workers` = W, msvc, the armed flag the row expects; drops 0 on armed rows and no disarmed ring
  traffic; the row's broadphase; TreeDiag `static_rebuilds 1, members 1, evictions 0` on tree rows; `canary_ns`
  printed on the canary rows. **On every Jolt process:** exit 0, one stat line with threads = W and hash
  `0xb8522b4e3fc62cfe`, the patch banner, 500 frames. **On every criterion process:** exit 0, every expected id has a
  `time:` estimate, and no other id is printed.

**When and where.** On 2026-09-25, before any timed pass: the instrument's build 05:24:04–05:24:11, the untimed pose
gate 05:33–05:36 (its criterion cell ran 05:43–05:53, `gate_criterion.json`), the rehearsals and the dry run
(`plan.md` § 6). The run was launched once at 06:07:48, with no flags (`raw/shell_log.txt`), and timed
06:12:15–07:44:30 +03:00; `WINDOW_DONE` reads `exit 0` / `complete` at 07:44:36. `raw/window_state.json` reads
`"voided_processes": 0`, `"binaries_after": "all match"`, 238 timed processes (one of them a re-run) and 10 warm-ups,
and the power scheme High performance. The machine
is the owner's workstation (Ryzen 9 5900HS, 8C/16T; 16 logical CPUs in every receipt).

| block | pass 0 | pass 1 |
|---|---|---|
| Q1C | 06:12–06:15 | 06:17–06:20 |
| Q2C | 06:23–06:27 | 06:30–06:34 |
| Q3 | 06:37–07:07 | — |
| Q4 | 07:09–07:13 | 07:15–07:19 |
| Q2D | 07:22–07:26 | 07:28–07:33 |
| Q1D | 07:35–07:39 | 07:41–07:44 |

**The idle rule** (`wait_log.txt`): 11 waits, 35 polls. The first reached idle after 5 polls (polls 1–2 read 9.28 /
7.29 % with a `python` process on top); every other wait reached idle in the minimum 3. No poll saw a build process or
a process under `D:/wt/_targets` or `D:/wt/mq-*`; the process count was 275–286; D: stayed at 122.6 GB free.

**Counts.** `raw/runs.jsonl` holds 259 records: 248 processes (10 warm-ups, 237 originals, 1 re-run) and 11 pass
markers. All 237 slots are used, one by its re-run (`Q1D-p0 #022 r2 H-trk-tree W8`: the original's after-receipt read
16.88 %, `RuntimeBroker.exe`); 0 are dropped, 0 passes voided. Every used process passes every validity check above
(the analyst re-derives them from the files, `analysis.md` § 0).

**Receipts.** 260 distinct: median 1.12 %, p90 3.09 %, max 16.88 %, 1 over 5 %. No build or lane process at any
receipt or during any process. The during-process witness (`others_busy_pct`, recorded, not gated) reads medians of
0.59–0.69 % in every pass; one process reads 14.25 % (`Q1D-p1 #024 r2 H-trk-tree W8`, `msedge.exe` on top), and it is
the one that sets block D's W8 range (`analysis.md` § 1.3).

**The pose gate** (untimed, `gate/gate_run.log`, `gate/gate_all.json`, `tools/gate7b.py`): 27 processes, 0 failing.
- Every timed runner cell once with its row's `--expect-pose`, `--csv`, `--pose-out`, `--label`, armed and canary
  flags: 19/19 OK. J500 `0x32d5e235342b4143` on trk, par and tip; JAon500c4 `0x30c5438bc6ad9ffa` on c4 (the canary
  moves no pose; `canary_ns` 600,617 / 1,184,550 / 1,785,167 / 2,369,100 ns).
- Jolt at W 4/8/16: hash `0xb8522b4e3fc62cfe`, threads = W, the banner, 500 frames (3/3).
- Criterion (`gate/gate_criterion.json`): the timed filter at full settings, exit 0, 54 ids and no other, and every
  size of both families printed its LeafList and RowWalk kernel receipt (1/1).
- **Red controls:** a 501-step twin against the 500-step fixture exits 4 (`expect mismatch`) on trk, par, tip and c4
  (4/4). The gate can fail.
- `gate/estimates_runner.json` holds the per-process walls the schedule used. `gate/probe_g4_grid23/` keeps the
  stdout/stderr of the untimed 17–256 grid probe (0.5 s / 1 s criterion settings) that placed both crossovers between
  128 and 160 and so chose the timed grid (`plan.md` § 4 Q3); it is an indication, not a result.

## Binaries

| key | exe (sha256 prefix, `bin/SHA256SUMS`) | commit | what | provenance |
|---|---|---|---|---|
| trk | `win7/bin/runner_93b2615b.exe` `24d52719` | `93b2615b` | the trunk `integ/unified`, reuse off by default | window 7's, run in place (the same exe path as blocks A and B) |
| j56 | `D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe` `918fd2b7` | Jolt v5.6.0 | P0's Distribution build | windows 3 and 7's, run in place |
| par | `win6/bin/runner_6dd1f916.exe` `228f3514` | `6dd1f916` | C3b's parent (query RowWalk) | window 6's, run in place (window 6 P3/P5g's exe path) |
| tip | `win6/bin/runner_983480a9.exe` `f93e5fec` | `983480a9` | C3b's tip (query LeafList) | window 6's, run in place |
| c4 | `win7/bin/runner_989ca0f0.exe` `c4a75f82` | `989ca0f0` | `u/phys-l9-c4`, reuse on by default | window 7's, run in place |
| g4r | `bin/bpbench_g4ref_93b2615b.exe` `f96a9c11` | `93b2615b` + `G4_SIZES` | **the Q3 instrument** (below); never committed | built here |

- Every reused sha256 was re-checked against its own window's `SHA256SUMS` (`tools/mkrows7b.py` asserts it), before
  every pass, and at the end (`"binaries_after": "all match"`). No executable is in the tree.
- `93b2615b` and `983480a9` carry identical `broadphase_tree` code (`git diff --stat` empty, `plan.md` § 4 Q2), so the
  default row's t_q on the tip is comparable in kind with window 7's P4 on the trunk.

**The Q3 instrument** (`bin/COMMIT.txt`, `tools/build_g4ref.sh`, `logs/build_g4ref_broadphase.log`):
- The tree is `git -C D:/wt/lighttable archive 93b2615bcea4873e2d3af6b560c6e7997740f67f | tar -x` into the scratch
  `win7b/trees/93b2615b…_g4ref/`, never built in a worktree. `tar -d` against the archive reports only
  `crates/boyko_physics/benches/broadphase.rs` (`variant/tree_compare.txt`).
- **Its diff is one line** (`variant/g4ref.diff`), the recipe's own remedy (`g4_g5_recipe.md:131`: "add the sizes
  between them to `G4_SIZES` (a one-line edit)"):

  ```diff
  @@ -296,7 +296,7 @@
   // ── G4: the tree broadphase (design C3; module docs, "G4 of the tree broadphase") ────────────

   /// G4 pair-finding sizes of the `uniform` and `disparity` families: the design's list.
  -const G4_SIZES: [usize; 7] = [17, 64, 128, 256, 1_000, 10_000, 100_000];
  +const G4_SIZES: [usize; 27] = [17, 24, 32, 40, 48, 56, 64, 72, 80, 88, 96, 104, 112, 120, 128, 136, 144, 152, 160, 168, 176, 184, 192, 208, 224, 240, 256];
   /// G4 `scene` sizes, in dynamic boxes (the floor is one more row): J, and the design's 10k and
   /// 100k.
   const G4_SCENE_SIZES: [usize; 3] = [1_240, 10_000, 100_000];
  ```

- `TREE_BRUTE_MAX_ROWS` (`broadphase_tree/mod.rs:144`, = 64) is NOT changed: every G4 tree arm forces the tree path
  with `set_brute_max_rows(0)` (`benches/broadphase.rs:406`, `:436`), so the constant is the recipe's OUTPUT, not an
  input. `AUTO_TREE_LO/HI` do not exist in code.
- The build: `cargo bench --no-run -p boyko-physics --bench broadphase` (the `bench` profile, as window 6's bpbench),
  `stable-x86_64-pc-windows-msvc` (rustc 1.98.1 `48a229cea` at record time), `CARGO_INCREMENTAL=0`,
  `CARGO_BUILD_JOBS=8`, `TMP`/`TEMP` under `D:/wt/_targets/tmp`, `CARGO_TARGET_DIR=D:/wt/_targets/win7b-g4ref`,
  `RUSTFLAGS` unset. Exit 0; its `Compiling boyko-physics` line names the exported tree
  (`…\win7b\trees\93b2615bcea4873e2d3af6b560c6e7997740f67f_g4ref\crates\boyko_physics`).
- **The lock:** `D:/wt/joltab/Cargo.lock`, LF sha256 `5f8de754e85aeebd7838ff01156cbfc5f815e89b8052a1f50dd8a7e090191bcb`
  (the one the trunk tracks), copied in; the log reads the same hash before and after the build.

## Layout

| Path | What it is |
|---|---|
| `analysis.md` | The analyst's reduction, with the orchestrator's four rulings quoted and applied (see the note at the top). § 0 the selection, the input checks, the run, the driver comparison; § 1 Q1; § 2 Q2; § 3 Q3; § 4 Q4; § 5 what cannot be claimed; the FOLLOW-UP work items |
| `plan.md` | The run's plan: launch, protocol, binaries, the pose gate, each item's source, rows, rule and power, the estimated duration, the rehearsals, what was left out |
| `rows7b.json` | The run list: the protocol block (with the canary's R and rise/injected), the six binaries, the six blocks, the 12 rows (args, W, windows, armed, broadphase, pose ref, pinned lengths) |
| `run_window.sh` | The launcher: the driver, then the reduction, then `WINDOW_DONE` |
| `dryrun.txt`, `wait_log.txt`, `progress.txt`, `WINDOW_DONE` | The dry-run schedule, the idle-rule poll log, the per-pass completion lines, the completion marker |
| `bin/SHA256SUMS`, `bin/COMMIT.txt` | The six binaries' sha256 by the exact spelling `rows7b.json` uses; the commit, tree, diff, command, target dir, `Compiling` line and lock of the instrument |
| `logs/build_g4ref_broadphase.log` | The instrument's build log |
| `variant/g4ref.diff`, `variant/tree_compare.txt` | The instrument's one-line diff against the archived trunk file, and `tar -d`'s report that no other file differs |
| `gate/` | The untimed pose gate: `gate_run.log`, `gate_all.json`, `gate_criterion.json`, `estimates_runner.json`, `fixtures/fixtures.json` (the two fixtures by hash and path: window 7's `../gate/fixtures/J500.pose` and `JAon500c4.pose`), `probe_g4_grid23/` (the untimed grid probe's stdout/stderr) |
| `raw/runs.jsonl` | One record per process (259 with the pass markers): args, cwd and its lengths, exit, receipts before/after, the during-process witness, the driver's statistics, the binary's hash |
| `raw/<block>-p<n>_060748/<seq>_r<k>_<row>_<bin>_W<w>[x…][_warmup\|_rerun]/` | One directory per process (248): `stdout.txt` (the SUMMARY line), `stderr.txt`, the per-step `run.csv` (armed rows carry the span columns) or Jolt's `per_frame_discrete_thW.csv`; Q3's three processes keep criterion's `criterion/<id>/{base,new}/{estimates,sample,tukey,benchmark}.json` |
| `raw/manifest_1790305668.json`, `raw/window_state.json`, `raw/window_log.txt`, `raw/driver_stdout.txt`, `raw/shell_log.txt`, `raw/passes_done.txt`, `raw/driver_done.txt` | The launch's manifest, the run's start/end state, the window log, the driver's stdout, the launcher's log, the passes done, the driver's exit status |
| `raw/reduction.json`, `raw/tables.md`, `raw/reduce_stdout.txt` | The driver's untimed reduction (`tools/reduce7b.py`), run by `run_window.sh` after the last pass |
| `analyst/reduction.json`, `analyst/tables.txt`, `analyst_run.log` | The analyst's reduction (`tools/analyze7b.py`) and its tables (`analyst_run.log` is the same text, byte-identical) |
| `tools/` | `window7b_run.py` (the driver), `wait_idle7.ps1` (the idle rule), `gate7b.py` (the pose gate), `mkrows7b.py` (writes `rows7b.json`), `reduce7b.py` (the driver's reduction), `analyze7b.py` (the analyst's), `build_g4ref.sh` (the instrument's build), `joltprof.py` (imported by the driver), `record_dedup_poses7b.py` (this record's pose deduplication), `lib/driver.py`, `window_lib/pdhperf.py` |

**Pose files: one per distinct pose, not one per process.** At record time the 209 `pose.bin` files under `raw/` were
hashed (`tools/record_dedup_poses7b.py`); each of the two distinct files is byte-identical to a committed window-7
fixture, and each maps to exactly one `pose_hash` in the processes' `stdout.txt`:

| `pose_hash` | fixture (`../gate/fixtures/`) | sha256 prefix | processes | kept in |
|---|---|---|---|---|
| `0x32d5e235342b4143` | `J500.pose` | `eff361e1` | 177 | `raw/Q1C-p0_060748/000_r-1_H-trk-tree_trk_W8_warmup/` |
| `0x30c5438bc6ad9ffa` | `JAon500c4.pose`, `JDon500c4.pose` | `e268d7a5` | 32 | `raw/Q4-p0_060748/000_r-1_L9-JA-on_c4_W1_warmup/` |

The other 207 files were byte-identical and are not committed; each process's `pose_hash` and `expect_pose: "match"`
remain in its `stdout.txt` and in `runs.jsonl`.

**Not kept in the tree:** the executables; the exported tree (`win7b/trees/…_g4ref/`) and every target dir; the
untimed rehearsals (`test/`, `test_logs/`); the gate's per-process directories (`gate/cells/`, `gate/red/`) and the
grid probe's criterion directory, whose results are in `gate_run.log`, `gate_all.json` and the probe's stdout; the
original `broadphase.rs` copy (`variant/broadphase.rs.orig`, which is `git show 93b2615b:crates/boyko_physics/benches/broadphase.rs`);
the empty `driver.log`, `tools/__pycache__/`, and the 207 duplicate pose files.

## How to re-run

1. **Binaries.** Take trk, c4 (window 7), par, tip (window 6) and Jolt by hash from their windows' records; never
   rebuild them for a comparison against these rows. Rebuild the instrument with `tools/build_g4ref.sh` (export
   `93b2615b`, apply `variant/g4ref.diff`, copy the tracked lock in), and quote its `Compiling boyko-physics (path)`
   line.
2. **Rows and gate:** `python -B tools/mkrows7b.py` writes `rows7b.json`; `python -B tools/gate7b.py` re-runs the pose
   gate and the red controls.
3. **Run:** `bash run_window.sh` once, in the background, with no agent working and rust-analyzer off.
   `--dry-run [--start HH:MM]` prints the schedule, `--test` / `--test-real` rehearse, `--resume` skips the passes in
   `raw/passes_done.txt`, `--cutoff HH:MM` sets the hard stop, and the flag file `STOP` stops the driver before its
   next process.
4. **Reduce:** `python -B tools/reduce7b.py` regenerates `raw/reduction.json` and `raw/tables.md` (it reads window 7's
   blocks A and B from the scratch `win7/raw/`); `python -B tools/analyze7b.py` writes `analyst/` and resolves window
   7's record from this directory's parent.
5. **The machine must be quiet.** The idle rule and the 5-s receipts gate each pass and process; the void rule gates
   build and lane processes. A foreground application is seen only in the during-process witness, which this protocol
   records but does not gate — block D's W8 outlier is the example (`analysis.md` § 1.3, FOLLOW-UP 1).
