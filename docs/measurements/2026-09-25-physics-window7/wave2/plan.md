# Window 7 wave 2: the plan (Q1-Q4), prepared 2026-09-25 05:05-06:10

Script: `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/7dc37fc3-7b1e-46f1-89da-ccd0dba7c788/scratchpad/win7b/run_window.sh` (LF).

This file only prepares the run. No timed pass has been run. Every number marked "untimed" came from one process run
while this agent was working (claude.exe load, other gate processes). **These numbers are not results.**

## 0. How to launch, and what comes out

- **Launch.** `bash <win7b>/run_window.sh` once, in the background, with no agent working and rust-analyzer/LSP off.
  Estimated **96.3 min** plus idle waits beyond the 135-s minimum. **Latest start for the whole run: about 10:08**
  (11:45 − 96 min); a later start is handled by the priority skip (below).
- **Flags.** `--dry-run [--start HH:MM]` prints the schedule, the minutes per block and per item, what the priority
  skip would drop, and the launch-context lengths of every padded row; runs nothing. `--resume` skips the passes in
  `raw/passes_done.txt` and restores the canary reference from `raw/runs.jsonl`. `--cutoff HH:MM` (default **11:45**).
  `--blocks a,b`. `--test` (12 steps), `--test-real` (real steps and poses, one round, one pass).
- **Hard stop, two ways.** (1) `--cutoff 11:45`: no process starts whose estimated end (the gate's untimed wall + 0.4 s
  + 5 s) passes 11:45; the running process finishes. (2) **The flag file `win7b/STOP`**: create it (any content) and
  the driver stops before its next process or pass (checked in `ensure_time`; not during an idle wait, which can take
  up to 30 min). Both give `WINDOW_DONE` exit 2; `--resume` continues.
- **Priority.** Q1 1, Q2 2, Q3 3, Q4 4. Before a block starts, the driver reserves the estimated time of every LATER
  block of a strictly higher priority; if the block does not fit before the cutoff with that reserve, it is **skipped**
  (log, progress line `SKIPPED for priority`, a `block_skipped` record) so the closing Q2D and Q1D are never crowded
  out. Dry run at `--start 10:20`: Q4 is skipped, everything else fits by 11:43.
- **Checkpoints.** Every process appends one record to `raw/runs.jsonl`; files under `raw/<block>-p<n>_<runtag>/<seq>_r<k>_<row>_<bin>_W<w>[x...]/`.
  Every finished pass: `'<HH:MM:SS> <item> <row> p<n> done'` in `progress.txt` and `<block>-p<n>` in `raw/passes_done.txt`.
  Idle waits log to `wait_log.txt`; the driver log is `raw/window_log.txt`.
- **At the end.** The untimed reduction (`tools/reduce7b.py` -> `raw/tables.md`, `raw/reduction.json`), then
  `WINDOW_DONE`: line 1 `exit <n>` (**0** complete; **2** cut at 11:45, cut by the STOP flag, or complete except blocks
  skipped for priority; **3** stopped: idle never came in 30 min / a binary changed / 20 voids; **5** driver crashed),
  line 2 the status, line 3 the time, line 4 `reduction exit <n> (raw/tables.md)`.

## 1. Protocol (window 7's, verbatim; `tools/window7b_run.py` = window7_run.py plus the changes listed)

- **Order.** Q1C -> Q2C -> Q3 -> Q4 -> Q2D -> Q1D. Q1's two blocks open and close the run (separated by about 72 min),
  Q2's are separated by Q3 and Q4 (about 44 min).
- **Passes.** K = 6 per cell = 2 passes x 3 rounds (Q1, Q2, Q4). Pass 0: W ascending, inside a W group row by row
  (rows7b.json order), each row's binaries in order (arms alternate inside the W group); pass 1: the whole list
  reversed; one untimed warm-up per pass. **Q3: 1 pass x 3 rounds = K 3** (the recipe's K, `analysis.md` 6.1 of
  window 4), no warm-up (criterion warms each benchmark itself).
- **Idle rule before EVERY pass** (`tools/wait_idle7.ps1`, window 7's verbatim): no build/toolchain process, no process
  under `D:\wt\_targets\` or `D:\wt\mq-*`, cpu10 < 5 %, on 3 consecutive polls 60 s apart; up to 30 polls; a timeout
  STOPS the run (exit 3; `--resume` continues).
- **Receipts, void rule, launch, re-run once, validity**: window 7's (`win7/plan.md` §1), unchanged. Runner validity
  adds nothing new; the criterion process is valid iff exit 0, every expected id has a `time:` estimate and no other id
  printed.
- **Launch-context lengths (new).** A row may pin its process-directory length per W (`cwd_len`); the driver pads the
  leaf with `x` so the cwd and therefore `--csv` / `--pose-out` have exactly the pinned length; re-runs are suffixed
  `_R` so they still fit; every record keeps `cwd_len_target`, `cwd_len_actual`, `cwd_len_miss`, `cmdline_chars`. The
  exe and `--expect-pose` files are the earlier windows' own paths (run in place). Dry run: every pinned cell hits its
  target on the original and on the re-run (Q1: 163/161/161 at W4/8, 164/162/162 at W16 = window 7's; Q2: 163 = window
  6 P3, 166 = window 6 P5g), and the command lines equal the earlier windows' character counts exactly (637/635/38,
  641/639/39; 648 and 654). A miss happens only if a voided pass is re-run AND a process of it re-run; it is flagged,
  and Q2's cells exclude it.
- **Statistic, claim rule.** Window 7's: per cell the median over K, [min-max], r = range/median, i = IQR/median,
  s = 1.2533 sd/sqrt(K)/median; **B vs A is CLAIMED iff |B/A − 1| > 2 hypot(sA, sB) under BOTH r and s** (i printed),
  no claim when either side has K < 3. **Against a constant bar b: CLAIMED iff |m/b − 1| > 2 r and > 2 s of the cell.**

## 2. Binaries (`bin/SHA256SUMS`, `bin/COMMIT.txt`)

| key | exe (run in place) | commit | sha256 | used by |
|---|---|---|---|---|
| trk | win7/bin/runner_93b2615b.exe | 93b2615b trunk | 24d52719... | Q1 (= window 7 A/B) |
| j56 | D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe | Jolt v5.6.0 | 918fd2b7... | Q1 (= windows 3, 7) |
| par | win6/bin/runner_6dd1f916.exe | 6dd1f916 C3b parent, RowWalk | 228f3514... | Q2 (= window 6) |
| tip | win6/bin/runner_983480a9.exe | 983480a9 C3b tip, LeafList | f93e5fec... | Q2 (= window 6) |
| c4 | win7/bin/runner_989ca0f0.exe | 989ca0f0 L9 C4 | c4a75f82... | Q4 (= window 7 P2) |
| g4r | win7b/bin/bpbench_g4ref_93b2615b.exe | 93b2615b + G4_SIZES (instrument) | f96a9c11... | Q3 |

- Every reused sha256 was re-checked against its window's own SHA256SUMS (`tools/mkrows7b.py` asserts it).
- **Build (Q3 only).** Exported tree `win7b/trees/93b2615b..._g4ref/` (`git -C D:/wt/lighttable archive`), trunk lock
  copied in (LF 5f8de754..., identical to the archived one, unchanged after the build), the exact prefix, RUSTFLAGS
  unset, `CARGO_TARGET_DIR=D:/wt/_targets/win7b-g4ref`, `cargo bench --no-run -p boyko-physics --bench broadphase`
  (bench profile, as window 6's bpbench), exit 0 (`logs/build_g4ref_broadphase.log`). **The variant's diff is ONE
  line** (`variant/g4ref.diff`; `git archive | tar -d` finds only that file, `variant/tree_compare.txt`):
  `const G4_SIZES: [usize; 7] = [17, 64, 128, 256, 1_000, 10_000, 100_000];` ->
  `[usize; 27] = [17, 24, 32, ..., 128, 136, 144, 152, 160, 168, 176, 184, 192, 208, 224, 240, 256]`.
- **Why not a TREE_BRUTE_MAX_ROWS variant.** `TREE_BRUTE_MAX_ROWS` (broadphase_tree/mod.rs:144, = 64) is a
  compile-time constant but the recipe's OUTPUT: every G4 tree arm forces the tree path with `set_brute_max_rows(0)`
  (benches/broadphase.rs:406, :436), and the constants are read off `all_pairs/n` vs `tree/n`
  (`g4_g5_recipe.md:131-132`). `AUTO_TREE_LO/HI` do not exist in code (`MEASUREMENT-QUEUE.md:1102-1107`: "commit only
  after the refinement run"). The recipe's own remedy is "add the sizes between them to `G4_SIZES` (a one-line edit)"
  (`:131`), which is the only change made.
- **Disk.** D: 124 GB free before, 123 GB after (target dir 263 MB); C: 38 GB.

## 3. Pose gate (untimed, run now): 27 processes, 0 failing (`gate/gate_run.log`, `gate/gate_all.json`)

- **Every timed runner cell once** with its row's own `--expect-pose` file, `--csv`, `--pose-out`, `--label`, the armed
  flag and the canary flags: 19/19 OK — exit 0, `expect_pose` = match, void_steps 0, workers = W, target_env msvc,
  broadphase and TreeDiag as the row's (Tree: static_rebuilds 1, members 1, evictions 0), drops_total 0 when armed.
  - J500 `0x32d5e235342b4143` on trk (H-trk-tree, H-trk-ap at W 4/8/16), par and tip (C3b-TA-armed W1/8,
    C3b-TA-pad06 W1, C3b-TD-armed W1). The J500 files of windows 6 and 7 are byte-identical (sha256 eff361e1...).
  - JAon500c4 `0x30c5438bc6ad9ffa` on c4 (L9-JA-on and all four canary rungs; the canary does not move the pose);
    `canary_ns` printed: 600,617 / 1,184,550 / 1,785,167 / 2,369,100 ns at the reference 16,683,800 ns.
- **Jolt** v5.6.0 at W 4/8/16: exit 0, one stat line, threads = W, hash `0xb8522b4e3fc62cfe`, patch banner, 500
  frames (3/3).
- **Criterion** (Q3): one process with the timed filter at full criterion settings: exit 0, 54 ids, no other id; every
  size of both families printed its kernel receipt (`LeafList` and `RowWalk`), i.e. passed the bench's own oracle
  assertion (tree steady-state pair set = AllPairs under both kernels) (1/1).
- **Red controls.** A 501-step twin against the 500-step fixture exits 4 (`expect mismatch`) on trk, par, tip and c4
  (4/4): the gate can fail.

## 4. Items

### Q1 - settle the W8 headline (blocks Q1C first, Q1D last; 11.7 min each)
- **Source.** Window 7 `raw/tables.md` P1 W8: A 0.8355 not claimed (r bar 59.38 %: block A's tree cell ranged
  2.3290-2.9626), B 0.8721 CLAIMED, pooled 0.8111 not claimed -> "does NOT hold". Window 6 `analysis.md:412`
  (item 15): absolute-bar decisions from two separated blocks or pooled.
- **Rows** (window 7 P1's, verbatim; 500 steps, [0,500) plus [0,100) and [100,500)), **W 4/8/16**, order inside a W
  group H-trk-tree, H-jolt56, H-trk-ap: `--scene jolt --gap 0.5 --cfg default --broadphase tree` (trk; OUR default
  row), Jolt `-s=Pyramid -q=Discrete -f -t=W -i=500` (j56), `--broadphase allpairs` (trk). Same exe paths, same
  fixture path and the same command-line and cwd lengths as window 7 A/B.
- **Rule (the reducer applies it):** ours/Jolt at each W **HOLDS iff it is CLAIMED in block A, in B, in C, in D AND
  pooled over A-D (K up to 24), all in the same direction**; otherwise "does NOT hold" with the per-block Y/n list.
  Printed beside it: C, D and pooled C+D (this run alone), the [0,100) / [100,500) pooled sub-windows, per manifold
  (Jolt 8,489.0 from window 7's -receipt), block terms B/A, C/A, D/C, D/B per row, the window term against window 3.
- **Can it fail?** Yes: it failed in window 7 (not claimed in A). It turns RED (does not hold) whenever any one block's
  range is as wide as window 7's block A.
- **Untimed indication** (gate, agent running): tree 3.89 / 5.00 / 2.55 ms, allpairs 4.82 / 4.33 / 4.39, Jolt 3.68 /
  2.77 / 2.45 (W 4/8/16). The W8 tree value (5.00 vs window 7's 2.19) shows how far agent load moves a single process.

### Q2 - C3b t_q block-shift test (blocks Q2C and Q2D; 14.7 min each)
- **Source.** Window 6 `analysis.md:229-237` (t_q(W1) 0.2102 [0.2095-0.2120] in P3 against 0.2354 [0.2325-0.2399] in
  P5g, +11.96 % Y/Y/Y, same binary and arguments; the command lines differ only in the output paths, 648 against 654
  characters; "untested candidate: an address-layout effect of the launch context"), `:246` (C2 "not robust"), `:403`
  (item 11(b)); `c3b/design.md:222` (> 0.21 claimed under SE), `:226` (<= 0.186), `:231` and `:372` (C2 iff
  t_q(J, W=1) >= 0.235 ms "on the default row"), `:374` (c_q > 150 ns -> F3).
- **Binaries.** par 6dd1f916 against tip 983480a9, window 6's own exes in place.
- **Rows** (armed, 500 steps, metric [100,500)):
  - `C3b-TA-armed` W1 and W8: `--scene jolt --gap 0.5 --cfg a --broadphase tree --arm-profiler` = window 6's row;
    **plain layout = window 6 P3's** (cwd 163, command line 648 characters, the same exe and fixture paths);
  - `C3b-TA-pad06` W1: **the padded-path variant** = the same process in **window 6 P5g's layout** (cwd 166, 654
    characters: every path argument 3 characters longer, the label the same length);
  - `C3b-TD-armed` W1: the **default row** armed (`--cfg default --broadphase tree`), because the C2 rule's letter names
    the default row while window 6 read t_q off cfg-A. (Window 7 P4 read the trunk default row's t_q(W1) = 0.2043; the
    broadphase_tree code is identical between 983480a9 and 93b2615b, `git diff --stat` empty.)
- **Statistic.** t_q = per-process median over [100,500) of `phys_bp_query_ns`, median over K (window 6's); tree span =
  Σ of the four `phys_bp_*` medians; the step [0,500) beside it. Pooled K = 12 per cell over C and D.
- **Rules (the reducer applies them):**
  - **Block term** = D vs C, same row, binary and layout; **layout term** = pad06 vs armed, same block and binary. If
    the layout term is claimed (tip W1) the window-6 shift is a launch-context effect; if only the block term is, it
    is a machine-state term; the pooled K=12 is then the decision figure.
  - **C2**: BUILT iff t_q(J, W=1) on the tip is >= 0.235 ms **claimed above the bar in C, in D and pooled**; NOT BUILT
    iff claimed below in C, D and pooled; otherwise UNRESOLVED (the orchestrator decides). Read on the default row
    (the letter) and on the cfg-A plain row (window 6's reading); the padded layout printed beside for robustness.
  - Also printed per row: > 0.21 claimed (attribution A), <= 0.186 claimed (into the band), c_q = t_q/1,240 > 150 ns
    (F3), tree span <= 0.36 / 0.35 ms (W1/W8), tip/par < 1 (the kernel ships).
- **Can it fail?** The layout test has the power to see window 6's shift: +12 % against a bar of about 2 x hypot(1.2,
  3.1) % = 6.6 % at window 6's spreads. The C2 test has three outcomes, and "claimed below in every block" is not the
  default: at window 6's spreads a 0.2102 cell would be claimed below 0.235, a 0.2354 cell would not.
- **Untimed indication** (rehearsal and gate, K=1, agent running): tip t_q W1 cfg-A 0.250 (plain) / 0.241-0.252
  (padded), default row 0.208; par 0.499. Not results.

### Q3 - the tree thresholds between 64 and 256 (block Q3; 31.1 min)
- **Source.** `c3b/design.md:244` ("the refinement run between 64 and 256 must be taken after F1"; the design predicts
  "the tree gets cheaper at every n, so the crossovers move down"); window 6 `analysis.md:404` (item 11(c));
  `treebp/g4_g5_recipe.md:127-132` (recipe 1.4, the rule); window 4 `analysis.md` 6.1/6.2 (the grid 17/64/128/256/1k,
  K = 3 for the refinement, log-log crossovers 111 uniform / 138 disparity, L2's procedure); `MEASUREMENT-QUEUE.md:1097-1107`.
- **Row.** `G4-refine` on the instrument g4r: `--bench --noplot` with the filter
  `^bp_g4_(uniform|disparity)/((all_pairs|tree)/(64|96|112|128|136|144|152|160|168|176|192|256)|tree_rowwalk/(64|128|256))$`
  = 54 benchmarks per process (both families, all_pairs vs the shipped `tree` = LeafList at 12 sizes, plus the
  same-binary `tree_rowwalk` = C1's kernel at window 4's grid points as the bridge), criterion's own settings (3 s warm-up,
  5 s, 10 flat samples), CRITERION_HOME per process. K = 3.
- **Sizes, and why.** The recipe's range is 64-256; the first untimed probe of the full 17-256 grid (0.5 s/1 s
  criterion settings, `gate/probe_g4_grid23/`) put both crossovers between 128 and 160 (uniform all_pairs/tree 0.887 at
  128, 1.017 at 144; disparity 0.921 at 128, 0.977 at 144, 1.080 at 160), so the timed grid is step 8 from 128 to 176,
  plus 64, 96, 112, 192, 256 for the band's ends and the bridge. **This moves UP from window 4's 111/138, against the
  design's prediction** - an untimed indication, not a result; the RowWalk bridge arm tests it in the same binary.
- **Rule (recipe 1.4, pinned by `g4_g5_recipe.md:131-132`; the reducer applies it):** per family, LO_f = the largest n
  at which `all_pairs` is not claimed slower than `tree`; HI_f = the smallest n at which `tree` is claimed faster;
  **TREE_BRUTE_MAX_ROWS = min(LO_uniform, LO_disparity)**; **AUTO_TREE_LO / AUTO_TREE_HI = the wider band (min LO_f,
  max HI_f)**, LO >= TREE_BRUTE_MAX_ROWS. Printed beside it, as window 4 did: **L2's procedure** (HI = the larger
  family's log-log crossover to two significant figures, LO = 0.9 HI), the monotonicity of each family, and a flag
  when the answer sits on the grid edge. Bridge lines: LeafList/RowWalk at 64/128/256 and all_pairs/tree_rowwalk
  against window 4's 0.619/1.137/1.997 (uniform) and 0.500/0.946/1.543 (disparity).
- **Can it fail?** The rule can return "claimed slower at the smallest size" or "no crossover in the grid (extend)".
  A synthetic code-path check (the gate's run x 1, 1.01, 0.99, `test/q3synthetic/`, labelled synthetic) produced
  LO/HI 152/160, L2 150/135, so every branch of the reducer ran; it is not a measurement.
- **Untimed indication** (second gate run, full settings): noisy under agent load - `uniform/tree/168` read 77 µs
  against about 14 µs in its neighbours - which is why this item needs the quiet machine and K = 3.

### Q4 - G-TW's canary resolution (block Q4; 12.5 min)
- **Source.** Window 7 `raw/tables.md` P2: the canary injected 0.8447 ms, step rise +0.8961 ms (5.38 %), NOT SEEN under
  the two-spread rule (r bar 10.96 %, armed rows); G-TW's own J-A W1 [0,500) bar was **R = 2 hypot(3.352, 1.728) % =
  7.542 %** (off r 3.352 %, on r 1.728 %). L9 `02-DESIGN-REV1.md:405-412` (G-TW, "the canary must be seen"; the SE bar
  2.0 % at W=1 "against a predicted 4-8 %").
- **Rows** (c4 989ca0f0, W1, unarmed, the SAME rows as G-TW's own `on` arm): `L9-JA-on` (`--scene jolt --gap 0.5
  --cfg a --contact-reuse on`, the reference) and four canary rungs, same arguments plus `--canary-frac F
  --canary-ref-ns <the latest valid L9-JA-on window mean>` (window 7's recipe): **F = k R / 1.0609 for k = 0.5, 1, 1.5,
  2 -> 0.036, 0.071, 0.107, 0.142** (1.0609 = window 7's rise/injected, 0.8961/0.8447). Expected rises 3.8 / 7.5 / 11.3
  / 15.1 % of about 16.7 ms = 0.63 / 1.26 / 1.89 / 2.52 ms. Unarmed, because G-TW's rows are unarmed (window 7's armed
  canary pair had a 10.96 % bar against the unarmed rows' 7.54 %); window 7 already showed on this binary that the
  canary's span reads its spin (0.8450 vs 0.8447 ms).
- **Rule (the reducer applies it):** a rung is SEEN iff its rise over the reference is claimed upward (r and s) and
  `canary_ns` is printed. **G-TW's resolution is DEMONSTRATED iff the 1.5 R and 2 R rungs are SEEN**; the smallest
  rung seen with every larger rung seen is the resolution demonstrated in this block (1 R = the gate's window-7
  resolution; 0.5 R = the low end of the size L9's design says G-TW guards, 4 %).
- **Can it fail?** Yes: window 7's own canary was not seen. G-TW's two-spread resolution at every other W was 24-73 %
  (window 7: J-A W2 26.2 %, W8 48.2 %; J-D W1 28.1 %; R W1 17.9 %), so this item demonstrates the W1 J-A gate only.
- **Untimed indication** (gate): reference 16.82 ms, rungs 17.44 / 17.87 / 18.60 / 19.08 ms (rises 0.61 / 1.04 / 1.78
  / 2.25 ms against injected 0.60 / 1.18 / 1.79 / 2.37).

## 5. Estimated duration (`dryrun.txt`; per process: the gate's untimed wall + 0.4 s + 5 s; per pass >= 135 s idle + 10 s + one warm-up)

| block | item | prio | cells | K | timed processes | minutes |
|---|---|---|---|---|---|---|
| Q1C | Q1 | 1 | 9 | 6 | 54 | 11.7 |
| Q2C | Q2 | 2 | 8 | 6 | 48 | 14.7 |
| Q3 | Q3 | 3 | 1 | 3 | 3 (54 benchmarks each, ~9.5 min per process) | 31.1 |
| Q4 | Q4 | 4 | 5 | 6 | 30 | 12.5 |
| Q2D | Q2 | 2 | 8 | 6 | 48 | 14.7 |
| Q1D | Q1 | 1 | 9 | 6 | 54 | 11.7 |
| **total** | | | | | 237 | **96.3** |

Per item: Q1 23.4, Q2 29.4, Q3 31.1, Q4 12.5 min. Within the 150-minute budget; longer idle waits, re-runs and
voids are not included. Started by 10:08 everything fits before 11:45; later, the priority skip drops Q4 first,
then Q3.

## 6. Rehearsals (untimed; `test_logs/`, `test/real/`)

- `--test` (12 steps, one round, one pass, every block): exit 0 `complete`, 18 progress lines, reduction exit 0
  (`test_logs/plain_test/`).
- `--test --test-cutoff-s 1800` (the priority skip): exit 2 `complete except the blocks skipped for priority: Q3`
  (Q3 needed 30.7 + 17.8 min of reserve against a 30-min cutoff) (`test_logs/priority_skip_cutoff1800/`).
- STOP flag: the flag created after Q2C's pass -> exit 2 `cut by the STOP flag after Q2C-p0 r0 ... (next would have
  been Q3-p0 r0 G4-refine#g4r@W1)`; then `--resume` without the flag: the two done passes skipped, exit 0 `complete`
  (`test_logs/stop_flag_then_resume/`).
- `--test-real --blocks Q1C,Q2C,Q4` (real steps, windows and pose checks): exit 0 `complete`, 22 timed processes, 0
  invalid, reduction exit 0; window 7's A/B records load into Q1 (`test/real/tables.md`). The test paths are 6
  characters longer than `raw/`, so the pinned lengths miss there by design; `reduce7b.py --test-real --keep-misses`
  exercised every Q2 line (`test/real/reduce_keep_misses.txt`).
- Dry run: every pinned length hits its target and every command line equals the earlier window's character count.

## 7. Left out, and why

- Q1 at W 1/2: the task names W8 (W4, W16 if the budget allows); blocks A/B have them, the reducer reads W 4/8/16.
- Q2 at W8 for the padded and default rows: the window-6 shift was W1-only (W8 reproduced, -1.7 %); W8 plain is kept as
  the control.
- Q3 sizes 17-56 and 184-240: the probe placed both crossovers in 128-160; 64 and 256 keep the recipe's range and the
  bridge. If the timed crossover lands on a grid edge, the reducer says so ("extend the grid").
- Q4 at other W: G-TW's resolution there is 24-73 % (window 7), not a canary-sized question.
- K = 12 for G9 and the F3 A/B (window 6 items 11(a), 13): not in this task.
