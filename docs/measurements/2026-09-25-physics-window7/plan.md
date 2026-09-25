# Window 7: the plan for the quiet window of 2026-09-25

The owner declared the machine quiet at 00:29 (Moscow), no duration given. Hard stop 04:15.

Script: `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/7dc37fc3-7b1e-46f1-89da-ccd0dba7c788/scratchpad/win7/run_window.sh`.

This file only prepares the window. No timed pass has been run. Every number marked "untimed" below comes from one
gate process run while this agent was working. **These numbers are not results.**

## 0. How to launch, and what comes out

- **Launch.** `bash <win7>/run_window.sh` once, in the background, with no agent working (claude.exe costs 6-8 % CPU;
  the idle rule needs cpu10 < 5 %). Keep rust-analyzer and the LSP off.
- **Rehearsal flags.** `--dry-run [--start HH:MM]` prints the schedule and runs nothing; `--test` is the 12-step
  rehearsal under `test/window/`; `--resume` skips the passes listed in `raw/passes_done.txt` (after a stop or a cut;
  it also restores the canary reference from `raw/runs.jsonl`); `--cutoff HH:MM` (default 04:15); `--blocks a,b`.
- **Checkpoints.** Every process appends one record to `raw/runs.jsonl` as it ends; its files go to
  `raw/<block>-p<n>_<runtag>/<seq>_r<k>_<row>_<bin>_W<w>/`. Every finished pass appends
  `'<HH:MM:SS> <item> <row> p<n> done'` to `progress.txt` and `<block>-p<n>` to `raw/passes_done.txt`. Idle waits log
  to `wait_log.txt`; the driver log is `raw/window_log.txt`.
- **At the end.** The untimed reduction (`tools/reduce7.py` -> `raw/tables.md`, `raw/reduction.json`), then
  `WINDOW_DONE`: line 1 `exit <n>` (0 complete, 2 cut at the hard stop, 3 stopped: idle never came in 30 min /
  a binary changed / 20 voids, 5 driver crashed), line 2 the status (`complete`, `cut at 04:15:00 after <pass> r<k>
  <row>#<bin>@W<w> (next would have been ...)`, `STOP ...`).

## 1. Protocol (window 6's, verbatim; `tools/window7_run.py` = window6_run.py plus the Jolt kinds)

- **Order.** Blocks P1A-jolt -> P2-L9GTW -> P3-G9JA -> P4-trkspans -> P5-joltprof -> P1B-jolt.
- **Passes.** Each block is 2 passes x 3 rounds, K = 6 per cell. Pass 0: W ascending; inside a W group row by row
  (rows7.json order), each row's binaries in order, so the arms alternate inside the W group. Pass 1: the whole list
  reversed. One untimed warm-up per pass.
- **Idle rule before EVERY pass** (`tools/wait_idle7.ps1` = wait_idle6.ps1 plus the gcc/cmake toolchain names): no
  process named cargo, rustc, link, lld-link, lld, rust-lld, dxc, clippy-driver, cl, msbuild, miri, cargo-miri,
  cc1plus, cc1, lto1, lto-wrapper, ld, collect2, mingw32-make, cmake, or the three bench exe names; no process whose
  image is under `D:\wt\_targets\` or `D:\wt\mq-*`; cpu10 < 5 %; on 3 consecutive polls 60 s apart; up to 30 polls.
  A timeout STOPS the window (exit 3; `--resume` continues).
- **Receipts.** 10 s opening each pass, 5 s between processes (the receipt after process i is the one before i+1);
  machine CPU, top-5, presence snapshot. A receipt > 5 % busy before or after marks the process contaminated; a
  contaminated or invalid process is re-run ONCE at the end of its pass.
- **Void rule.** A build/lane process seen at any receipt or started during a process VOIDS the whole pass: the pass
  is abandoned, the idle rule re-run, the pass re-run from its start (records kept and marked, excluded).
- **Launch.** P-none (suspended, mask read back, resumed; no affinity call), as windows 3-6.
- **Validity per process.** Runners: exit 0, void_steps 0, `--expect-pose` = `match` and the pose hash = the fixture,
  workers = W, target_env msvc, the armed flag and `drops_total` 0 on armed rows, no disarmed ring traffic,
  `config.broadphase` = the row's, TreeDiag (tree rows: static_rebuilds 1, members 1, evictions 0; AllPairs rows: all 0);
  the canary row must print `canary_ns`. Jolt: exit 0, one stat line, threads = W, hash = 0xb8522b4e3fc62cfe (window
  3's), the patch banner `boyko-parity-patch v1 ... allow_sleep=0, receipt=0`, 500 frames in the per_frame csv; the
  profiled build also needs its four dumps at frames 100/200/300/400.
- **Statistic.** Per process: the mean of the per-step wall over the row's window (ours: `wall_ns` of run.csv; Jolt:
  `Time (ms)` of per_frame_discrete_thW.csv, window 3's statistic), plus every span/count column's mean and median over
  each metric window. Per cell: the **median over K**, [min-max], and the relative spreads r = (max - min)/median,
  i = IQR/median, s = SE of the median/median = 1.2533 sd/sqrt(K)/median.
- **Claim rule.** B against A is claimed iff |B/A - 1| > 2 sqrt(sA^2 + sB^2) under BOTH r and s (i printed); no
  claim when either side has K < 3.
- **Hard stop.** No process starts whose estimated end (the gate's untimed wall + 0.4 s + 5 s) passes 04:15; the
  current process always finishes, so a cut loses only the tail.

## 2. Binaries (`bin/`, `bin/SHA256SUMS`, `bin/COMMIT.txt`)

| key | exe | commit | sha256 | what |
|---|---|---|---|---|
| trk | bin/runner_93b2615b.exe | 93b2615b | 24d52719... | trunk integ/unified: Phase B + thin-box fix + D-M0; contact reuse OFF by default |
| c4 | bin/runner_989ca0f0.exe | 989ca0f0 | c4a75f82... | u/phys-l9-c4 = 6c40b3e6 + the C4 flip (reuse ON by default); `--contact-reuse on/off` gives both arms from one binary |
| g9p | bin/runner_0ca312bd.exe | 0ca312bd | 3993684b... | window 5's G9 parent, copied from win6/bin (= win6 SHA256SUMS) |
| g9t | bin/runner_cbd86a65.exe | cbd86a65 | 9c7caff1... | window 5's G9 tip, copied from win6/bin (= win6 SHA256SUMS) |
| j56 | D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe | Jolt v5.6.0 | 918fd2b7... | P0's Distribution build, **not rebuilt**: sha256 = window 3's `bin/SHA256SUMS`; run in place (its MinGW DLLs beside it; outside the lane prefixes) |
| j56p | bin/jolt56prof/PerformanceTest.exe | Jolt v5.6.0 | aa23db26... | the same source, compiler and options + `PROFILER_IN_DISTRIBUTION=ON`; P5 only |

- **Builds (ours).** `git -C D:/wt/lighttable archive <sha> | tar -x -C win7/trees/<sha>/`; `D:/wt/joltab/Cargo.lock`
  (LF sha256 5f8de754...) copied in, no `--locked`; `PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 TMP/TEMP=D:/wt/_targets/tmp`, RUSTFLAGS unset;
  `cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid`,
  `CARGO_TARGET_DIR=D:/wt/_targets/win7-<sha>`; both 00:30:36 -> 00:31:49, exit 0, `Finished parity ... in 59.3s`.
  Each log: `Compiling boyko-physics v0.1.0 (C:\Users\flint\AppData\Local\Temp\claude\D--claude-BoykoEngine\7dc37fc3-7b1e-46f1-89da-ccd0dba7c788\scratchpad\win7\trees\<sha>\crates\boyko_physics)`.
  **The lock was NOT pruned** this time: LF sha256 after each build = 5f8de754... in both trees (window 6's trees pruned it to 530cc386...).
- **Build (Jolt, P5).** `tools/build_jolt_prof.sh`: the WinLibs MinGW cmake/gcc of P0's build, `-S D:/tmp/jolt/wt-v5.6.0-parity/Build`
  (unmodified; `git describe` v5.6.0-dirty), `-B D:/wt/_targets/win7-jolt56prof`, Distribution, `-DPROFILER_IN_DISTRIBUTION=ON`,
  target PerformanceTest only; 00:34:42 -> 00:36:27, exit 0. CMakeCache diff against build-v5.6.0-dist: only
  PROFILER_IN_DISTRIBUTION and the TARGET_* switches (IPO, AVX2/FMADD/F16C/LZCNT/TZCNT, GENERATE_DEBUG_SYMBOLS,
  FLOATING_POINT_EXCEPTIONS identical). Nothing was written into the Jolt worktree or Jolt's library sources.
- **Disk.** D: 48 GB free before, 47 GB after; C: 39 GB free.

## 3. Pose gate (untimed, run now): 112 processes, 0 failing (`gate/gate_run.log`, `gate/gate_all.json`)

- **Base** (500 steps, W 1/8): J `--cfg default --broadphase tree` and `allpairs` -> `0x32d5e235342b4143` on trk, on c4
  with `--contact-reuse off`, on g9p and g9t (16/16); rest `--cfg default` -> `0xee2a67a98434919a` on all four (8/8).
  The c4 shipped default (no flag, reuse on) reads `0x30c5438bc6ad9ffa` at W 1/8 (recorded).
- **L9's C0 fixtures** (600 steps, `docs/measurements/2026-09-23-l9-contact-reuse/fixtures` of 989ca0f0): J-A, J-D, R
  (W8 with `--parallel-solve`), R-S, J-Son at W 1/8, `--expect-pose` = `match` on c4 with `--contact-reuse off` (10/10:
  the design's "every C4 reuse-off row equals C0's hash", `02-DESIGN-REV1.md:414`) and on trk (10/10).
- **Every timed runner cell once with `--expect-pose`** (53/53). Fixtures (`gate/fixtures/fixtures.json`), each recorded
  NOW from this window's own binary and then required of every other W:

  | ref | hash | rows | note |
  |---|---|---|---|
  | J500 | 0x32d5e235342b4143 | every J reuse-off row (trk default tree/allpairs, armed; c4 J-A/J-D off; g9p/g9t J-A) | bytes = window 6's J500.pose (eff361e1...) |
  | JAon500c4 | 0x30c5438bc6ad9ffa | c4 J-A `--contact-reuse on`, W 1-16, armed, canary | recorded on 989ca0f0; bytes = window 6's JAon500 (e268d7a5...) |
  | JDon500c4 | 0x30c5438bc6ad9ffa | c4 J-D on, W 1-16 | recorded on 989ca0f0; same bytes as JAon500c4 (one hash across cfg a/default, as window 6) |
  | R1100 | 0x87e561d20589d4a5 | c4 R off (1100 steps), W 1-16 | = window 4b/6 bytes |
  | Ron1100c4 | 0xc8bbe34cf6a8afc6 | c4 R on, W 1-16 | recorded on 989ca0f0; bytes = window 6's Ron1100 |

- **Jolt.** v5.6.0 at W 1/2/4/8/16 (`-f`, 500 its): hash `0xb8522b4e3fc62cfe` = window 3's `expected_hash_500`, threads = W
  (5/5). The profiled build at W 1/8 (`-f -p`): the same hash, 5 dumps each (2/2). `-receipt` at W 1 and 8
  (`gate/jolt_receipt.json`): manifolds 8,489.0 per frame over [100,500), 7,042.23 over [0,100), final 8,489 = window 3's
  receipt exactly; the same hash.
- **Red controls.** A 501-step J-A twin with `--expect-pose J500` exits 4 (`expect mismatch`) on trk, c4 (reuse off),
  g9p and g9t (4/4).
- **Re-runs.** No mismatch occurred, so no memory-fault re-run was needed.

## 4. Items

### P1 - the Jolt headline in the same window (blocks P1A-jolt first, P1B-jolt last; ~17 min each)
- **Source.** Window 3 (`docs/measurements/2026-09-21-physics-window3/README.md:27`: JOLT56-T = v5.6.0 Distribution
  918fd2b7, `-s=Pyramid -q=Discrete -f`, W 1-16; `rows.json`: `-t=W -i=500`, `expected_hash_500 0xb8522b4e3fc62cfe`;
  `analysis.md:42`: per manifold per step over [100,500), each side's own receipted count). Window 6
  (`docs/measurements/2026-09-24-physics-window6/analysis.md:354`: "Every Jolt ratio is cross-window against window
  3"; `:412`, item 15: absolute-bar decisions from two separated blocks or pooled K=12, the +/-5 % W8 window term).
- **Rows** (500 steps, window [0,500), sub-windows [0,100) and [100,500)), W 1/2/4/8/16, order inside a W group
  H-trk-tree, H-jolt56, H-trk-ap (Jolt between our two rows):
  - `H-trk-tree` (trk): `--scene jolt --gap 0.5 --cfg default --broadphase tree` - OUR default row; reuse off on this tree.
  - `H-jolt56` (j56): `-s=Pyramid -q=Discrete -f -t=W -i=500`.
  - `H-trk-ap` (trk): `--cfg default --broadphase allpairs` - the AllPairs reference.
- **Claims.** ours/Jolt at every W, per block (A, B) and pooled (K=12). **The W8 claim HOLDS iff it is claimed pooled
  AND in block A AND in block B, in the same direction**; the same test is printed for every W. Also: tree vs
  allpairs; the block term (B vs A for each row, same window); T(1)/T(W); the window term against window 3's
  JOLT56-T cells (9.828 / 5.770 / 3.581 / 2.569 / 2.388 ms; context only).
- **Per manifold.** [100,500): ours = the per-step `manifolds` column of run.csv, mean over [100,500), per process
  (window 3: 4,519.2575; SUMMARY `final_manifolds` also recorded); Jolt = its `-receipt` count 8,489.0 (gate, W1 = W8).
  us/manifold = T[100,500) x 1000 / M; the per-manifold ratio = ours/Jolt. The counts are constants, so its claim is
  the [100,500) step claim.
- **Untimed indication** (one process each, agent running): tree 6.55 / 4.49 / 3.16 / 2.58 / 2.83 ms; allpairs 8.34 /
  6.31 / 5.07 / 4.70 / 4.91; Jolt 10.69 / 6.26 / 4.15 / 3.10 / 2.98 (W 1/2/4/8/16).

### P2 - L9 C4's timed gate G-TW (block P2-L9GTW, ~44 min)
- **Source.** `docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md:384` (C4), `:405` (G-TW: "a
  claimed regression at any W blocks"), `:407-414` (L9b: same binary C4, off vs on, on J-A, J-D and R at every W; the
  realized-gain rule; the J [0,100) moving-scene witness "not claimed slower"; the canary must be seen; the per-contact
  table at W 1 and 8; `on` rows one hash per row across W). Window 6 `analysis.md:387-389` (item 6) and `:129-130`
  (prediction 0.9897 x 4,515 x (435.56 - [100, 60]) ns = [1.4994, 1.6782] ms; bar 0.6 x 1.4994 = 0.8997 ms).
- **Binary.** c4 only (989ca0f0), both arms from the one binary via `--contact-reuse off|on`.
- **Rows** (arms alternate inside each W group):
  - `L9-JA-off/on`: `--cfg a`, 500 steps, [0,500) + [0,100) + [100,500), W 1/2/4/8/16; poses J500 / JAon500c4.
  - `L9-JD-off/on`: `--cfg default` (AllPairs, window 6's spelling), same windows, W 1-16; J500 / JDon500c4.
  - `L9-R-off/on`: `--scene rest --solver colored`, 1100 steps, [600,1100), W 1-16; R1100 / Ron1100c4.
  - `L9-JA-a-off/on`: armed `--cfg a`, W 1/8, [100,500): the per-contact table.
  - `L9-JC`: the canary, armed `--cfg a --contact-reuse on --canary-frac 0.05 --canary-ref-ns <the latest valid
    L9-JA-a-on window mean, same binary, W1>`, W1 (window 4b/6's recipe on the L9 binary).
- **Rules.**
  - G-TW: any on-vs-off ratio > 1 CLAIMED on J-A, J-D or R at any W ([0,500), [600,1100), and the J [0,100)
    sub-window) BLOCKS.
  - Realized gain: dT(1)[100,500) on J-A = off - on >= 0.8997 ms AND claimed -> PASS; else refuted.
  - The canary is seen iff its step rise is claimed (both spreads; SE-only printed) and its span
    (`sys_parity_canary_ns`) is within 5 % of the injected `canary_ns`.
  - The armed table: per stage (wall, bp, np, graph, solve, solve_build, colours wide/narrow, warm_apply, store,
    sys_sum) ms/step, us/manifold (own count) and us/pair, off and on, W1 and W8; beside P5's Jolt stages.
- **Untimed indication.** J-A off/on W1 19.07 / 17.32 ms; J-D 8.49 / 6.63; R 11.61 / 8.64.

### P3 - the remainder of L11's G9 (block P3-G9JA, ~11 min)
- **Source.** Window 6 `analysis.md:407` (item 12: J-A at W 2/4/16, g9p against g9t, K=6);
  `levers/L11-solve-setup/02-DESIGN-REV1.md:359-360` (G9: parent against tip, "not claimed slower"; its K=12 letter is
  window 6's open item 13, not decided here).
- **Rows.** `G9-JA-mid`: `--scene jolt --gap 0.5 --cfg a`, 500 steps, [0,500), W 2/4/16, g9p vs g9t, pose J500.
- **Rule.** tip vs parent not claimed slower at each W.

### P4 - our per-stage armed spans (block P4-trkspans, ~7 min)
- **Row.** `S-trk-tree-a` (trk): the default row armed, `--scene jolt --gap 0.5 --cfg default --broadphase tree
  --arm-profiler --csv`, W 1/8, 500 steps, metric window [100,500), pose J500.
- **Statistic.** Per process, the MEDIAN over steps [100,500) of every `phys_*` / `sys_*` span (the mean beside it);
  per cell the median over K. Counts (manifolds, pairs, np/bp pairs) per step. No bar: it pairs with P5.

### P5 - Jolt per-stage profile (block P5-joltprof, ~7 min; built)
- **How.** The profiled build (j56p) with the harness's own `-p`: PerformanceTest dumps a one-frame profile chart
  every 100th iteration (`PerformanceTest/PerformanceTest.cpp:515-519`; `Jolt/Core/Profiler.cpp` NextFrame/DumpInternal:
  the samples are reset every frame, so each dump is one frame), so frames 100/200/300/400 lie in [100,500). The dump
  is written at the next frame's NextFrame, outside the harness's timed region. `tools/joltprof.py` parses each chart:
  per scope, cpu = the profiler's own aggregate (sum over threads, children included), wall = the union of its
  intervals across threads.
- **Row.** `P-jolt56-prof`: `-s=Pyramid -q=Discrete -f -p -t=W -i=500`, W 1/8, hash 0xb8522b4e3fc62cfe.
- **Stages printed.** PhysicsSystem::Update; UpdateBroadPhasePrepare/Finalize; the FindCollisions job (broadphase
  pair finding + narrowphase + contact-constraint add, one job in Jolt) with FindCollidingPairs (bp),
  "Add Constraint From Cached Manifold" and sCollideConvexVsConvex (np) inside it, and np = FindCollisions -
  FindCollidingPairs (cpu); islands; SetupVelocityConstraints; SolveVelocityConstraints; SolvePositionConstraints;
  IntegrateVelocity. Per process the mean over its four frames; per cell the median over K.
- **Caveat.** Jolt's contact-constraint setup happens inside narrowphase (at contact add), so "setup" is not a
  separate stage in Jolt; the profiler costs time: untimed [0,500) 15.02 ms profiled vs 10.69 ms unprofiled at W1
  (+40 %), 3.90 vs 3.10 at W8. Read the split as shares; never quote a profiled time as Jolt's step time.

## 5. Estimated duration (dry run, `dryrun.txt`, assumed start 01:10)

Per process: the gate's untimed wall + 0.4 s launch + 5 s receipt. Per pass: at least 135 s idle rule, a 10-s opening
receipt and one warm-up. Re-runs, voids and longer idle waits are not included.

| block | item | cells | timed processes | minutes | ends |
|---|---|---|---|---|---|
| P1A-jolt | P1 | 15 | 90 | 17.0 | 01:27 |
| P2-L9GTW | P2 | 35 | 210 | 43.5 | 02:10 |
| P3-G9JA | P3 | 6 | 36 | 11.2 | 02:21 |
| P4-trkspans | P4 | 2 | 12 | 6.6 | 02:28 |
| P5-joltprof | P5 | 2 | 12 | 7.1 | 02:35 |
| P1B-jolt | P1 | 15 | 90 | 17.0 | 02:52 |
| **total** | | 75 | 450 | **102.5** | **02:52** |

- Within the 150-minute budget; 83 minutes of slack to the 04:15 hard stop for a 01:10 start.
- P2 is larger than window 6's "about 15 minutes" (item 6): "every W" is 3 scenes x 2 arms x 5 W = 30 cells, plus the
  armed table (4) and the canary (1).

## 6. Rehearsals (untimed)

- `--test` (12 steps, one round, one pass, every block): exit 0 `complete`, 18 progress lines, reduction exit 0.
- `--test-real` (REAL steps, windows and pose checks, one round, one pass, no idle wait; blocks P1A, P2, P4, P5,
  `test/real/`): exit 0 `complete`, 77 timed processes, 0 voids, reduction exit 0; every section of the reduction
  (P1 per block/pooled with the per-manifold line, P2 off/on + armed table + canary + realized-gain line, P4 spans, P5
  Jolt stages) printed from real-format records. Many slots were contaminated (this agent was working), as expected.
- `wait_idle7.ps1` probed once (1 poll): parses, logs, exits 1 on the poll limit.

## 7. Left out, and why

- P2's L9a rows (C0 against C2, `02-DESIGN-REV1.md:408`): not in the task; C2's binary is not built here.
- G9's K=12 letter (window 6 item 13): the task sets K=6.
- The tree default flip (window 6 item 10) and F3: not built.
- Jolt v5.3.0: the task names v5.6.0 only.
