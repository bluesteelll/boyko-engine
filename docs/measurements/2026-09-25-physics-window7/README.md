# Physics perf campaign, window 7 — the Jolt headline in one window, L9 C4's timed gate, G9's last cells, and a stage-by-stage comparison with Jolt (2026-09-25)

This directory holds the receipts for:
- `docs/MEASUREMENT-QUEUE.md` section 15, the `RESULT, 2026-09-25, window 7` block;
- the four dated lines of 2026-09-25 in `docs/physics/perf-campaign/levers/00-RULINGS.md`:
  - the L9 block: C4's G-TW passes;
  - the L11 block: G9's J-A record, G9 closed;
  - the per-contact parity block: the Jolt headline, and where the per-manifold gap lives, from Jolt's profile.

**Read `analysis.md` first.** It is the results-analyst's reduction, recomputed from `raw/` with its own script
(`tools/analyze7.py`), not from the tester's `tools/reduce7.py`. The plan the window executed, with every item's
source, rows and bars, is `plan.md`.

**The analyst's text is recorded unchanged, and it may not be in this commit.** As in window 6, the harness blocks
report files for the analyst's role. So the text was not written to the scratch directory `win7/`, and this
recording step could not write it here either.
- If `analysis.md` is absent from this commit, the orchestrator adds it in the next commit, verbatim from the text
  the analyst returned (its first line is the one-line verdict string).
- Its closing "Files are in …" paragraph describes the scratch directory at the time of writing. Wherever it
  writes `win7/<path>`, read `<path>` under this directory.
- Its other inputs are committed:
  - window 3 (the Jolt v5.6.0 constants) is `docs/measurements/2026-09-21-physics-window3/`;
  - window 6 is `docs/measurements/2026-09-24-physics-window6/`.
- The Jolt source lines it cites (`PhysicsSystem.cpp`, `ContactConstraintManager.cpp`, `PhysicsSettings.h`,
  `PerformanceTest.cpp`) are in the Jolt v5.6.0 worktree `D:/tmp/jolt/wt-v5.6.0-parity`, not in this tree.

The analyst's tables are `analyst/tables.txt` (byte-identical to `analyst_run.log`) and `analyst/reduction.json`.
The tester's reduction is `raw/reduction.json`, rendered as `raw/tables.md`. `analysis.md` § 0 states that every
cell and comparison in it reproduces: 151 cells and 124 comparisons, 0 differences.

## What was measured

Five items, run as six blocks in this order: P1A-jolt, P2-L9GTW, P3-G9JA, P4-trkspans, P5-joltprof, P1B-jolt
(`plan.md` § 4; `rows7.json` holds the rows):

| item | question | blocks | binaries |
|---|---|---|---|
| P1 | The Jolt headline in the same window. Our default row against Jolt v5.6.0 at W 1/2/4/8/16, per block and pooled. The W8 claim holds only if claimed pooled and in both blocks, same direction (window 6 `analysis.md:412`, item 15). Also per manifold, each side's own count | `P1A-jolt` (first), `P1B-jolt` (last) | trk, j56 |
| P2 | L9 C4's timed gate G-TW (`levers/L9-contact-reuse/02-DESIGN-REV1.md:405-414`): off against on in one binary, the realized-gain rule, the canary, the per-contact table | `P2-L9GTW` | c4 |
| P3 | G9's last three cells: J-A at W 2/4/16, tip against parent (`00-RULINGS.md` "G9 on C3 CLOSED by ruling": "a record") | `P3-G9JA` | g9p, g9t |
| P4 | Our per-stage spans on the default row at W 1/8 | `P4-trkspans` | trk |
| P5 | Jolt's per-stage profile at W 1/8, from its own profiler | `P5-joltprof` | j56p |

| row | block | binaries | args (+ `--workers W --steps N --window a..b --csv --pose-out --label --expect-pose gate/fixtures/<ref>.pose`; Jolt: `-t=W -i=500`) | W | steps [window] (metric windows) | armed | pose ref |
|---|---|---|---|---|---|---|---|
| `H-trk-tree` | P1A, P1B | trk | `--scene jolt --gap 0.5 --cfg default --broadphase tree` | 1, 2, 4, 8, 16 | 500 [0,500) ([0,100), [100,500)) | no | J500 |
| `H-jolt56` | P1A, P1B | j56 | `-s=Pyramid -q=Discrete -f` | 1, 2, 4, 8, 16 | 500 frames, the same windows | — | hash `0xb8522b4e3fc62cfe` |
| `H-trk-ap` | P1A, P1B | trk | `--scene jolt --gap 0.5 --cfg default --broadphase allpairs` | 1, 2, 4, 8, 16 | the same | no | J500 |
| `L9-JA-off` / `-on` | P2 | c4 | `--scene jolt --gap 0.5 --cfg a --contact-reuse off` / `on` | 1, 2, 4, 8, 16 | 500 [0,500) ([0,100), [100,500)) | no | J500 / JAon500c4 |
| `L9-JD-off` / `-on` | P2 | c4 | `--scene jolt --gap 0.5 --cfg default --contact-reuse off` / `on` | 1, 2, 4, 8, 16 | the same | no | J500 / JDon500c4 |
| `L9-R-off` / `-on` | P2 | c4 | `--scene rest --solver colored --contact-reuse off` / `on` | 1, 2, 4, 8, 16 | 1100 [600,1100) | no | R1100 / Ron1100c4 |
| `L9-JA-a-off` / `-on` | P2 | c4 | as `L9-JA-off` / `-on` | 1, 8 | 500 [0,500) ([100,500)) | yes | J500 / JAon500c4 |
| `L9-JC` (canary) | P2 | c4 | `--cfg a --contact-reuse on --canary-frac 0.05 --canary-ref-ns <the latest valid L9-JA-a-on W1 window mean>` | 1 | 500 [0,500) ([100,500)) | yes | JAon500c4 |
| `G9-JA-mid` | P3 | g9p, g9t | `--scene jolt --gap 0.5 --cfg a` | 2, 4, 16 | 500 [0,500) | no | J500 |
| `S-trk-tree-a` | P4 | trk | `--scene jolt --gap 0.5 --cfg default --broadphase tree` | 1, 8 | 500 [0,500) ([100,500)) | yes | J500 |
| `P-jolt56-prof` | P5 | j56p | `-s=Pyramid -q=Discrete -f -p` | 1, 8 | 500 frames; dumps at 100/200/300/400 | — | hash `0xb8522b4e3fc62cfe` |

**Protocol** (`plan.md` § 1, `rows7.json` → `protocol`). It is window 6's, verbatim, with Jolt added as a process
kind:
- **Cell and claim rule.** A cell is the MEDIAN over K separate processes of each process's statistic:
  - ours: the mean of `wall_ns` over the row's window;
  - Jolt: the mean of `Time (ms)` of `per_frame_discrete_thW.csv`, window 3's statistic;
  - armed rows also keep every span column's per-process mean and median over each metric window.

  Spread: r = min–max and i = IQR, with the median's SE s = 1.2533·SD/√K beside them, each relative to the median.
  B against A is claimed iff |B/A − 1| > 2·√(sA² + sB²) under BOTH r and s; i is printed as well. There is no claim
  when either side has K < 3.
- **K and ordering.** **K = 6**: every block is two passes × three rounds. Pass 0 orders the cells by W ascending,
  row by row inside a W group with each row's binaries in their listed order, so the arms alternate inside the W
  group. Pass 1 is the whole list reversed. Each pass opens with one untimed warm-up process.
- **Idle rule before EVERY pass** (`tools/wait_idle7.ps1`, window 6's `wait_idle6.ps1` plus the gcc and cmake
  toolchain names). It needs three consecutive 60-s polls with all of:
  - no build process (cargo, rustc, link, lld-link, lld, rust-lld, dxc, clippy-driver, cl, msbuild, miri,
    cargo-miri, cc1plus, cc1, lto1, lto-wrapper, ld, collect2, mingw32-make, cmake, and the three bench exe names);
  - no process whose image is under `D:/wt/_targets` or `D:/wt/mq-*`;
  - a 10-s CPU below 5 %.

  A 30-poll timeout STOPS the window (exit 3; `--resume` continues).
- **Receipts.** A 10-s receipt opens each pass, and a 5-s receipt sits between processes. A process whose receipt
  before or after reads above 5 % busy, or that is invalid, is re-run ONCE at the end of its pass.
- **Void rule.** A build or lane process seen at any receipt, or started during a process, voids the whole pass.
  The pass is then re-run from its start after the idle rule.
- **Launch** P-none: suspended, the affinity mask read back, resumed; no affinity call.
- **Validity on every runner process:**
  - exit 0 and `void_steps` 0;
  - `expect_pose: match` against the row's fixture, and the hash equal to it;
  - `workers` = W, `target_env` msvc, and the armed flag the row expects;
  - `drops_total` 0 and no disarmed ring traffic;
  - the row's broadphase kind;
  - TreeDiag `static_rebuilds 1, members 1, evictions 0` on tree rows and all zero on AllPairs rows;
  - `canary_ns` printed on the canary row.
- **Validity on every Jolt process:** exit 0, one stat line with threads = W and hash `0xb8522b4e3fc62cfe`, the
  patch banner `boyko-parity-patch v1 … allow_sleep=0, receipt=0`, and 500 frames. The profiled build also needs
  its dumps at iterations 100/200/300/400.
- **Hard stop.** No process starts whose estimated end passes the cutoff. `plan.md` names 04:15; the launcher ran
  with `--cutoff 10:00`, then `--cutoff 10:30` on the resume (`raw/shell_log.txt`). The cutoff was not reached.

**When and where.**
- Before any timed pass, on 2026-09-25:
  - the builds, 00:30–00:36;
  - the untimed pose gate, 00:41–00:50;
  - the rehearsals, 00:51–00:59, and the dry run at 01:01.
- The timed window ran 01:09:37–01:29:50 and 02:54:16–05:03:57 +03:00.
  - `raw/window_state.json` (of the resumed run) reads `"status": "complete"`, `"exit": 0`,
    `"voided_processes": 0`, `"binaries_after": "all match"`.
  - `raw/window_log.txt` records `binaries after all match` at the stop as well.
- The machine was the owner's workstation (Ryzen 9 5900HS, 8C/16T; 16 logical CPUs in every receipt). The power
  scheme read High performance at the resume (`raw/window_state.json`).
- The passes ended at (`progress.txt`):

  | block | pass 0 | pass 1 |
  |---|---|---|
  | P1A | 01:29:50 | 03:12:26 |
  | P2 | 04:00:15 | 04:20:17 |
  | P3 | 04:26:07 | 04:34:56 |
  | P4 | 04:38:06 | 04:41:16 |
  | P5 | 04:44:40 | 04:48:05 |
  | P1B | 04:56:01 | 05:03:57 |

- `WINDOW_DONE` reads `exit 0` / `complete` / the reduction's exit 0.

**The interruption and its handling** (`analysis.md` § 0; `wait_log.txt`, `raw/shell_log.txt`, `raw/window_log.txt`):
- **The first launch** was at 01:02:16. P1A-jolt-p0 ran 01:09:37–01:29:50.
- **The stop.** The owner then started a game.
  - The idle rule before P1A-jolt-p1 read 16.49 % at 01:29:50, then 15.97–27.13 % with `Inscryption` on top from
    01:30:50 through its 30th poll at 01:58:50.
  - It timed out at 01:59:50, and the driver stopped with exit 3: "STOP before P1A-jolt-p1: idle rule not met in
    30 min". That `WINDOW_DONE` is kept as `WINDOW_DONE.prev-1790293553`.
- **The resume** was at 02:45:53 (`--resume`). It skipped P1A-jolt-p0 (`raw/passes_done.txt`) and restored the
  canary references from `raw/runs.jsonl`.
  - Idle came at 02:54:06, after 9 polls; poll 1 read 47.53 % with `rust-analyzer` on top.
  - The window then completed.
- **No process ran between 01:29:50 and 02:54:06.** `Inscryption` appears in no poll after 01:58:50 and in no
  receipt or during-process top-5 of any process.
- **What the idle rule did not hold back: the owner's browser.** `browser.exe` tops the during-process witness of
  138 processes, 02:55–04:00. That is all of P1A-jolt-p1 and P2-L9GTW-p0.
  - Their used processes read a witness median of 2.28 % and 1.90 %, against 0.54–0.66 % in every pass from 04:02
    on.
  - P1A-jolt-p0 ran with an agent session active: `claude.exe` topped 29 of the window's 88 receipts over 5 %.
  - The protocol gates the receipts, not this witness, so these processes are used. `analysis.md` §§ 0–2 reads their
    effect: block A is the loaded block of P1, and P2's pass 0 widens G-TW's cells.
- Between the 01:29:50 and 01:30:50 polls, D: free space rose from 46.72 to 123.06 GB. No timed process was running.

**Jolt was run in this window**, unlike window 6. The unprofiled rows use P0's Distribution build, unchanged and run
in place (sha256 `918fd2b7…`, equal to window 3's `bin/SHA256SUMS`). P5 uses a profiled build of the same source.

## Binaries

| key | exe (sha256 prefix, `bin/SHA256SUMS`) | commit | what | built from / provenance | `Compiling boyko-physics` path | log |
|---|---|---|---|---|---|---|
| trk | `runner_93b2615b.exe` `24d52719` | `93b2615b` | the trunk `integ/unified`: Phase B + the thin-box fix + D-M0; contact reuse OFF by default | exported tree `trees/93b2615b…/`, target `D:/wt/_targets/win7-93b2615b…`, `Finished parity … in 59.31s` | `(C:\Users\flint\AppData\Local\Temp\claude\D--claude-BoykoEngine\7dc37fc3-7b1e-46f1-89da-ccd0dba7c788\scratchpad\win7\trees\93b2615bcea4873e2d3af6b560c6e7997740f67f\crates\boyko_physics)` | `logs/build_93b2615b…_jolt_parity_pyramid.log` |
| c4 | `runner_989ca0f0.exe` `c4a75f82` | `989ca0f0` | `u/phys-l9-c4`: trunk `6c40b3e6` + the L9 C4 flip (reuse ON by default); `--contact-reuse on/off` gives both arms | `trees/989ca0f0…/`, target `D:/wt/_targets/win7-989ca0f0…`, 59.28 s | `(…\scratchpad\win7\trees\989ca0f0575e054b89bb9c288ad95569ce493734\crates\boyko_physics)` | `logs/build_989ca0f0…_jolt_parity_pyramid.log` |
| g9p | `runner_0ca312bd.exe` `3993684b` | `0ca312bd` | window 5's G9 parent | copied byte-identical from window 6's `bin/` (= its `SHA256SUMS`) | window 5's | window 5's `logs/build_parent.log` |
| g9t | `runner_cbd86a65.exe` `9c7caff1` | `cbd86a65` | window 5's G9 tip (L11 C3) | copied byte-identical from window 6's `bin/` | window 5's | window 5's `logs/build_tip.log` |
| j56 | `D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe` `918fd2b7` | Jolt v5.6.0 (`wt-v5.6.0-parity`, `v5.6.0-dirty`, the repository patch) | P0's Distribution build, not rebuilt; run in place with its MinGW DLLs | = window 3's `bin/SHA256SUMS` | — | — |
| j56p | `jolt56prof/PerformanceTest.exe` `aa23db26` | the same source | the same compiler, generator and CMake options + `PROFILER_IN_DISTRIBUTION=ON`; P5 only | `tools/build_jolt_prof.sh`, out-of-source `D:/wt/_targets/win7-jolt56prof`, 00:34:42–00:36:27, exit 0 | — | `logs/build_jolt56prof.log` |

**How the two new runners were built** (`bin/COMMIT.txt`, `tools/build_one.sh`):
- Each commit was exported with `git -C D:/wt/lighttable archive <sha> | tar -x` into the scratch
  `win7/trees/<sha>/`, never built in a worktree.
- The build command: `cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid`
  (`parity` inherits `release`: fat LTO, default CGU). One `CARGO_TARGET_DIR` per tree, both 00:30:36 → 00:31:49.
- Toolchain: rustc 1.98.1 (`48a229cea` 2026-09-01), `stable-x86_64-pc-windows-msvc`. `CARGO_INCREMENTAL=0`,
  `CARGO_BUILD_JOBS=8`, `TMP`/`TEMP` under `D:/wt/_targets/tmp`. `RUSTFLAGS` unset (`x86-64-v3` comes from each
  tree's `.cargo/config.toml`).
- Each log's `Compiling boyko-physics` line names its exported tree, so each binary compiled the commit it is named
  for.
- **The lock.** Each tree got `D:/wt/joltab/Cargo.lock`, LF sha256
  `5f8de754e85aeebd7838ff01156cbfc5f815e89b8052a1f50dd8a7e090191bcb`. That is the lock both commits track:
  `git show 93b2615b:Cargo.lock` and `git show 989ca0f0:Cargo.lock` hash the same.
  - The build used no `--locked`, and **cargo did not prune it**: the `lock after build` line reads the same LF
    sha256 in both logs.
  - Window 6's older trees pruned it to `530cc386…`; that does not apply here.
- **The two commits' physics differ only by C4.** `93b2615b` = `6c40b3e6` + D-M0 (ECS kernel only; no
  `crates/boyko_physics` change), and `989ca0f0` = `6c40b3e6` + the C4 flip (11 files, +78/−26). So c4's reuse-off
  arm runs trk's physics code.
- The two copied runners were re-checked against window 6's `SHA256SUMS` before the copy. Every binary, including
  the two Jolt exes, was re-checked against `bin/SHA256SUMS` after the window (`"binaries_after": "all match"`).

**The profiled Jolt build** (`tools/build_jolt_prof.sh`, `logs/build_jolt56prof.log`, `plan.md` § 2):
- the WinLibs MinGW cmake/gcc of P0's build, on `D:/tmp/jolt/wt-v5.6.0-parity/Build` (unmodified);
- Distribution, with `-DPROFILER_IN_DISTRIBUTION=ON`, the PerformanceTest target only.
- Its CMakeCache differs from `build-v5.6.0-dist`'s only in that option and the `TARGET_*` switches.
- Nothing was written into the Jolt worktree or its sources.
- It is used ONLY for P5's per-stage shares. Its overhead reads +36.1 % at W1 and +18.7 % at W8 against the
  unprofiled block B (`analysis.md` § 5), so no profiled time is a Jolt time.

## The idle rule, the receipts and the counts

**The idle-rule log** (`wait_log.txt`): 13 waits (one before each of the 12 passes, plus the one that timed out),
80 polls.
- 9 waits reached idle in the minimum three polls. Before P1A-jolt-p0 it took 8 (`claude` processes), before the
  resumed P1A-jolt-p1 9, and before P3-G9JA-p1 6 (one poll at 5.53 %).
- One wait timed out: 30 polls, 01:29:50–01:59:50, the game.
- **No poll saw a build process or a process under `D:/wt/_targets` or `D:/wt/mq-*`.** The process count was
  271–286.

**Counts** (`analysis.md` § 0).
- `raw/runs.jsonl` holds 542 records: 530 processes (12 warm-ups, 450 originals, 68 re-runs) and 12 pass markers.
  0 passes were voided.
- 431 of the 450 slots are used, 49 of them by their re-run. **19 slots are dropped** because both attempts were hot,
  15 of them in the loaded P1A-jolt block. Those cells are K = 5, 4 or 3 (listed in `analysis.md` § 0).
- Every one of the 431 used processes passes every validity check above.

**Receipts.** 628 distinct receipts: median 1.92 %, p90 5.68 %, max 21.98 %, 88 over 5 %.
- `browser.exe` topped 55 of the 88, and `claude.exe` 29.
- No build or lane process was present at any receipt or during any process.
- The during-process witness (`others_busy_pct`) over the used set, by pass, is in `analysis.md` § 0.

**The pose gate** (untimed, 00:41:09–00:49:39; `gate/gate_run.log`, `gate/gate_all.json`, `tools/gate7.py`): 112 processes,
0 failing.
- **Base:** J `--cfg default` with `--broadphase tree` and `allpairs` at W 1/8 reads `0x32d5e235342b4143` on trk,
  on c4 with `--contact-reuse off`, and on g9p and g9t. Rest `--cfg default` reads `0xee2a67a98434919a` on all four.
  c4's shipped default (no flag) reads `0x30c5438bc6ad9ffa`.
- **L9's C0 fixtures** (`docs/measurements/2026-09-23-l9-contact-reuse/fixtures/`, 600 steps) match on c4 with
  reuse off and on trk: 10 and 10 (the design's `02-DESIGN-REV1.md:414`).
- **Every timed runner cell once with `--expect-pose`:** 53 of 53. These runs recorded the fixtures and the
  per-process time estimates (`gate/estimates_runner.json`).
- **Jolt** at W 1/2/4/8/16 reads hash `0xb8522b4e3fc62cfe` with threads = W. The profiled build reads the same hash
  with 5 dumps each at W 1/8.
- **The `-receipt` runs** (`gate/jolt_receipt/`, `gate/jolt_receipt.json`) give 8,489.0 manifolds per frame over
  [100,500) at W 1 and 8, window 3's receipt exactly. This is the per-manifold denominator.
- **Red controls:** a 501-step J-A run against the 500-step J fixture exits 4 (`expect mismatch`, pose
  `0x9336b30a06a7d8af`) on trk, c4, g9p and g9t. The gate can fail.

## What was reused from windows 3, 5 and 6, and why

- **Window 5's two runners, g9p and g9t** (`0ca312bd`, `cbd86a65`), byte-identical via window 6's `bin/`. G9's last
  three cells are a record against the same pair of binaries windows 5 and 6 ran G9 on.
- **P0's Jolt Distribution build** (`918fd2b7…`, window 3's), not rebuilt. The owner's ruling of 2026-09-21 makes
  Jolt v5.6.0 the only reference, and window 3's JOLT56-T cells are this build's. Window 3's cells are read only
  as the window term (`analysis.md` § 1), never pooled.
- **Row spellings.**
  - The P1 rows spell window 3's default row and JOLT56-T (`-s=Pyramid -q=Discrete -f`).
  - The L9 rows spell window 6's P2b/P5d/P5e/P5f rows, now on the C4 binary.
  - `G9-JA-mid` spells window 6's `G9-JA` at the missing W.
  - The canary is P0's recipe on the L9 binary.
- **Fixtures.** All five are byte-identical to files already committed, so their hashes tie this window to the
  earlier ones:
  - `J500` = the P0 J pose (window 3's `pose.bin`, sha256 `eff361e1…`);
  - `JAon500c4` = `JDon500c4` = window 6's `JAon500` (`e268d7a5…`);
  - `R1100` = window 4b/6's (`c133a50e…`);
  - `Ron1100c4` = window 6's `Ron1100` (`54b26c34…`).

  The four new names were recorded by this window's gate on this window's binaries.
- **The protocol block** of windows 3–6, unchanged, so windows 3 to 7 are read with one statistic and one claim rule.
- **The tooling lineage.**
  - `tools/window7_run.py` is window 6's `window6_run.py` plus the Jolt kinds, and `tools/wait_idle7.ps1` is
    `wait_idle6.ps1` plus the toolchain names. Both ancestors are committed under
    `docs/measurements/2026-09-24-physics-window6/tools/`.
  - `tools/lib/driver.py` and `tools/window_lib/pdhperf.py` are carried over from window 6.
  - `tools/joltprof.py` is the tester's Jolt profile parser. The analyst's `tools/analyze7.py` has its own parser.

## Layout

| Path | What it is |
|---|---|
| `analysis.md` | The analyst's reduction (see the note at the top). § 0 the selection, the input checks, the interruption, the driver comparison; § 1 P1; § 2 P2 (G-TW, its resolution, the canary, the per-contact table); § 3 P3; § 4 P4; § 5 P5; § 6 the stage-by-stage comparison per manifold; § 7 what cannot be claimed; the FOLLOW-UP work items; § 8 open points |
| `plan.md` | The window's plan: launch, protocol, binaries, the pose gate, each item's source, rows and bars, the estimated duration, the rehearsals, what was left out |
| `rows7.json` | The run list: the protocol block, the six binaries, the six blocks with their warm-up cells, the 15 rows (args, W, steps, windows, armed, broadphase, pose ref or Jolt hash) |
| `run_window.sh` | The launcher (resolves everything relative to its own directory): the driver, then the reduction, then `WINDOW_DONE` |
| `dryrun.txt` | The dry-run schedule (`run_window.sh --dry-run`) |
| `wait_log.txt`, `progress.txt`, `WINDOW_DONE`, `WINDOW_DONE.prev-1790293553` | The idle-rule poll log (both launches), the per-pass completion lines (the STOP line included), the completion marker, and the stop's marker |
| `bin/SHA256SUMS`, `bin/COMMIT.txt` | The binaries' sha256 (the Jolt exes and the three MinGW DLLs beside the profiled one included); the commit, tree, command, target dir, `Compiling` line and lock of each. No executable is in the tree |
| `logs/` | The two runner build logs, the profiled Jolt build log, `builds_done.txt` |
| `gate/` | The untimed pose gate: `gate_run.log`, `gate_all.json` (one record per gate process), `estimates_runner.json`, `fixtures/` (the five `.pose` files every timed runner process asserted against, with `fixtures.json`), and `jolt_receipt.json` + `jolt_receipt/j56_W{1,8}/` (the `-receipt` runs whose CSVs give Jolt's manifold and point counts) |
| `raw/runs.jsonl` | One record per process (542 with the pass markers): args, exit, receipts before/after, the during-process witness, the statistics, `summary` / `jolt` / `profile`, the binary's hash |
| `raw/<block>-p<n>_<runtag>/<seq>_r<k>_<row>_<bin>_W<w>[_warmup\|_rerun]/` | One directory per process (530): `stdout.txt`, `stderr.txt`, the per-step `run.csv` (armed rows carry the span columns) or Jolt's `per_frame_discrete_thW.csv`; the P5 processes also keep their five `profile_chart_discrete_thW_it{0,100,200,300,400}.html` dumps. Run tag `010216` is the first launch, `024553` the resume |
| `raw/manifest_1790287336.json`, `raw/manifest_1790293553.json`, `raw/window_state.json`, `raw/window_log.txt`, `raw/driver_stdout.txt`, `raw/shell_log.txt`, `raw/passes_done.txt`, `raw/driver_done.txt` | The two launches' manifests, the resumed run's start/end state, the window log (both launches), the driver's stdout, the launcher's log, the passes done, the driver's exit status |
| `raw/reduction.json`, `raw/tables.md`, `raw/reduce_stdout.txt` | The tester's untimed reduction (`tools/reduce7.py`), run by `run_window.sh` after the last pass |
| `analyst/reduction.json`, `analyst/tables.txt`, `analyst_run.log` | The analyst's reduction (`tools/analyze7.py`) and its tables (`analyst_run.log` is the same text, byte-identical) |
| `tools/` | `window7_run.py` (the driver), `wait_idle7.ps1` (the idle rule), `gate7.py` (the pose gate), `mkrows7.py` (writes `rows7.json`), `reduce7.py` (the tester's reduction), `joltprof.py` (the tester's profile parser), `analyze7.py` (the analyst's), `build_one.sh` (one runner build from an exported tree), `build_jolt_prof.sh` (the profiled Jolt build), `record_dedup_poses.py` (this record's pose deduplication), `lib/driver.py`, `window_lib/pdhperf.py` |

**Pose files: one per distinct pose, not one per process.**
- **`raw/` keeps one `pose.bin` for each of the four poses the window produced**, the first in path order. At record
  time the sha256 of all 446 `pose.bin` files in the scratch `raw/` was computed (`tools/record_dedup_poses.py`),
  and each of the four hashes maps to exactly one `pose_hash` in the processes' `stdout.txt`:

  | `pose_hash` | fixture | sha256 prefix | processes | kept in |
  |---|---|---|---|---|
  | `0x32d5e235342b4143` | J500 | `eff361e1` | 285 | `raw/P1A-jolt-p0_010216/000_r-1_H-trk-tree_trk_W8_warmup/` |
  | `0x30c5438bc6ad9ffa` | JAon500c4, JDon500c4 | `e268d7a5` | 89 | `raw/P2-L9GTW-p0_024553/002_r0_L9-JA-on_c4_W1/` |
  | `0x87e561d20589d4a5` | R1100 | `c133a50e` | 37 | `raw/P2-L9GTW-p0_024553/005_r0_L9-R-off_c4_W1/` |
  | `0xc8bbe34cf6a8afc6` | Ron1100c4 | `54b26c34` | 35 | `raw/P2-L9GTW-p0_024553/006_r0_L9-R-on_c4_W1/` |

- The other 442 process directories wrote byte-identical files, and those are not committed. Each process's
  `pose_hash` and `expect_pose: "match"` remain in `runs.jsonl` and its `stdout.txt`. The four kept files are also
  byte-identical to the `gate/fixtures/*.pose` of the same name.

**Not kept in the tree:**
- the executables and DLLs (their hashes are in `bin/SHA256SUMS`);
- the exported trees (`win7/trees/<sha>/`) and every target dir;
- the untimed rehearsals (`test/`: the 12-step control-flow rehearsal, the real-steps rehearsal, the idle-rule
  probe);
- the gate's per-process directories (`gate/base/`, `cells/`, `l9fix/`, `red/`, `probe/`), whose results are in
  `gate_run.log` and `gate_all.json`;
- the empty `driver.log` and `driver_resume.log`, `tools/__pycache__/`, and the 442 duplicate pose files above.

## How to re-run

1. **Build** into `bin/` and hash.
   - For `93b2615b` and `989ca0f0`: `tools/build_one.sh <sha>` exports the commit from `D:/wt/lighttable` into
     `trees/<sha>/`, copies the tracked lock in and builds the parity runner. Point `W` and the `git -C` path at this
     directory and at any checkout that has the commit. Quote each build's `Compiling boyko-physics (path)` line.
   - g9p and g9t are copies: take them from window 6's record by hash, and never rebuild them for a comparison
     against these rows.
   - Jolt: run P0's Distribution build by hash. `tools/build_jolt_prof.sh` rebuilds the profiled one out of source.
2. **Gate the poses:** `python -B tools/gate7.py` writes `gate/`, including the fixtures, the Jolt receipt and the
   red controls.
3. **Run the window:** `bash run_window.sh` once, with no agent working.
   - `--dry-run [--start HH:MM]` prints the schedule, `--test` is an untimed rehearsal under `test/`, and
     `--resume` skips the passes in `raw/passes_done.txt`.
   - It waits for the idle rule before each pass, voids and re-runs a pass on any build or lane process, writes
     `raw/`, then runs `tools/reduce7.py` and writes `WINDOW_DONE`.
4. **Reduce:** `python -B tools/reduce7.py` regenerates `raw/reduction.json` and `raw/tables.md`, and
   `python -B tools/analyze7.py` writes `analyst/`. Both resolve this directory relative to their own location.
5. **Quote nothing that is not in `raw/`.** The machine must be quiet.
   - The idle rule and the 5-s receipts gate each pass and process, and the void rule gates build and lane
     processes.
   - A user application in the foreground is seen only through the during-process witness, which this protocol
     reports but does not gate. This window's P1A-jolt-p1 and P2-L9GTW-p0 are the example (`analysis.md` § 0,
     FOLLOW-UP item 8).
