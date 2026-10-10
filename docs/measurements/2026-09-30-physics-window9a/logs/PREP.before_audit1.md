# Physics window 9a: prep (2026-09-30). PARENT = trunk 3d9433ae, TIP = u/phys-tree-c4 50e31f1a. Ready to launch. Nothing timed for a verdict; no timing number was used.
W = C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win9a
**Launch:** `bash $W/run_window9a.sh --cutoff HH:MM [--resume]`; preview `--dry-run --start HH:MM`; stop: create `$W/STOP`.
**Recommended cutoff = launch time + 392 min** (TOTAL 326.5 min x 1.2 = 391.8): e.g. launch 20:00 -> `--cutoff 02:32`. The driver rolls a cutoff
earlier than now - 1 h to the next day. If the cutoff bites, blocks skip whole in priority order (the criterion blocks are last and
re-runnable: `--resume`). With C4-BR at K 3 (below) the total is 224.7 min: cutoff = launch + 270 min.
Outputs as window 8b (raw/runs.jsonl, progress.txt, raw/counts.txt, WINDOW_DONE 0/2/3/5). **The idle rule cannot pass while another lane
builds or runs anything under D:/wt/_targets** (it did not during this prep: gate/wait_idle_smoke.txt; the machine ran at 60-65 % CPU).

## Binaries (bin/SHA256SUMS + rows9a.json `sha256_pin`; bin/COMMIT.txt; rustc 1.98.1 msvc, --locked, lock unchanged, RUSTFLAGS unset)
| key | file | sha256 | source |
|---|---|---|---|
| tip | bin/runner_tip_50e31f1a.exe | 8d6e7d4173857ca83b47cd350890b8e178451fb48f030ac3d25aaef33cd268c3 | git archive 50e31f1a -> D:/wt/_targets/w9a-trees/tip, `--profile parity`, target w9a-tip |
| parent | bin/runner_parent_3d9433ae.exe | a09fca085613f966e09b3e8cd78ed711aa1c51f069cfab649834d93f88988478 | git archive 3d9433ae -> .../parent, same build, target w9a-parent |
| g4rT | bin/broadphase_g4rT_50e31f1a.exe | 19c9eb1f67cc728505525b5d669adfd2db133a1ae461217fcab7f386bc7c0bcf | TIP export + bin/g4ref.patch (the G4_SIZES line, by content), bench profile, target w9a-g4rT |
| g4r7 | bin/bpbench_g4ref_93b2615b.exe | f96a9c11ebd72d4d581784cf37728fd6da15bc63b28414d7e895667753033fd4 | window 7's exe, byte copy (sha checked) |
| g4r8b | bin/broadphase_g4ref_16191fda.exe | b887850f38633d62419344c41080d1bada92bc931c3307989b6f6fb314a33b6d | window 8b's exe, byte copy (sha checked) |
| j56 | D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe | 918fd2b70ecc634cc0574cb676c56a3c929a46a51173cf9aa76fdd628e2ef0ad | Jolt 5.6, run in place |
- **Provenance:** `git merge-base u/phys-tree-c4 integ/unified` = 3d9433ae = integ/unified (so PARENT is the literal trunk, S4 ON, no patch; window 8b's S4-off
  parent and its `runner_tip_16191fda.exe` are NOT reused: the trunk moved). D:/wt/lighttable (HEAD 50e31f1a) and D:/wt/joltab (HEAD 3d9433ae) are clean; only
  `git archive` / `git status` ran there. Cargo.lock c00bf4c4... before = after every build = 8b's lock. g4rT's tree differs from the TIP export in exactly one file
  (benches/broadphase.rs, +90 bytes; 17,586 files in both). Each build 1-2 min (`logs/build_*.log`).
- **Swap guards (a swapped exe is red three ways):** the sha pin; the runner `config` receipt `tree_brute_max_rows` (tip 128, parent 144) and `broadphase_select` "Manual",
  checked on EVERY process (`expect_config`); the row's own broadphase / TreeDiag expectation.

## Protocol (rows9a.json `protocol`)
Ruling 1: K 9 = 3 passes x 3 rounds, **p0 reversed, p1 forward, p2 reversed**, warm-up per pass (AB, G5), idle rule per pass, receipts, busy 5 %, others-busy 2 %, P-none,
placement receipt, --cutoff/--resume, a block that does not fit is skipped whole. Claim rule (analysis): IQR AND SE in every clean block and pooled, min-max beside, STRONG when it also passes.
**Own K:** C4-G4-kd 1 pass x 3 rounds (the Q3 recipe, F3 keep/freeze, not gating).
**Ruling 8 (W8B) - implemented in tools/window9a_run.py (run_pass, pending_reruns, passcell_table):** a slot is unclean when its latest attempt has a receipt before/after > 5 %, or others-busy
> 2 %, or a build process during the run, or is invalid. At the end of the pass the driver re-runs unclean slots in WAVES (each wave takes every slot still unclean, original order; a slot
leaves the stage at its first clean valid attempt) until every pass-cell has all its slots clean or **the pass ends = 4 waves, or 0.5 x the pass's own timed wall spent on re-runs, or the
cutoff / STOP flag**. Records: `attempt: "rerun"` + `rerun_no` 1.. + `rerun_reason`; leaf `_R`, `_R2`, ...; one `{"passcell": true, pc_row, pc_binary, pc_W, k_target, k_clean, reruns, gates, short}`
per (pass, cell) (no `row` key, like the R4 records). counts.txt prints the census and one `SHORT ... does not gate` line per short cell. **The reduction's slot rule: the original if clean and valid,
else the FIRST clean valid re-run, else the slot is dropped; a pass-cell with fewer than 3 clean slots does not gate.** (8b: 26 of 786 slots dropped after ONE re-run; 8b ran 108 re-runs.)

## Blocks (priority order), rows, timed processes, minutes (dryrun.txt; K 9 unless noted)
1. **C4-AB** 34 cells, 306 + 3 warm-ups, 50.1 min. Per W group, in this order (a reversed pass keeps every adjacency): C4-JA (parent, tip; W 1/8/16, cfg a, AllPairs control), C4-JT
   (parent, tip; W 1/8/16, `--broadphase tree` control; in TIP an A/A twin of J-D), **C4-jolt56 (j56; W 1/2/4/8/16)**, **C4-JD (tip; W 1/2/4/8/16, the new default, `--sleeping off`)**, C4-JDap
   (tip; `--broadphase allpairs`, the same-binary row of record), C4-JDpar (parent; the literal parent default), C4-rung (tip; W 8/16; `--canary-frac 1 --canary-ref-ns 60000`). The Jolt cell is adjacent to
   C4-JD#tip at every W and to C4-JT#tip at W 1/8/16 in every round (selftest cases); C4-JD and C4-JDap are adjacent. Warm-up C4-JD#tip@W8.
2. **C4-G5** 10 cells, 90 + 3 warm-ups, 22.3 min (TIP): C4-JD-armed / C4-JDap-armed (`--arm-profiler`) W 1/8; C4-R / C4-Rap (rest, RT500) W 1/8; C4-S16 / C4-S16ap (16 rows, W1; R4 pose compare). Warm-up C4-JD-armed@W8.
3. **C4-BR** 3 cells x K 9 = 27 criterion processes, 152.7 min: g4r7, g4r8b, g4rT adjacent in every round (p0/p2 reversed), 8 ids `^bp_g4_(uniform|disparity)/(all_pairs|tree)/(144|256)$`, CRITERION_HOME per process.
   **C4-BR at K 3** (the lane's Q5 recommendation; 9 processes, 50.9 min, TOTAL 224.7 min, cutoff = launch + 270 min): `cp alt/rows9a.br_k3.json rows9a.json` (generated by `python -B tools/mkrows9a.py --br-k3`; alt/dryrun_br_k3.txt),
   then `--dry-run`. `python -B tools/mkrows9a.py` regenerates the K 9 default byte-identically. **Decision for the orchestrator (not blocking): K 9 is the letter of ruling 1 and is what this prep ships.**
4. **C4-G4** 9 processes, 83.5 min (g4rT): 36 ids `^bp_g4_(uniform|disparity)/(all_pairs|tree)/(96|104|...|160)$` (sizes 96..160 step 8). Rule (rows9a.json `lane_rules`): LO = the largest n with all_pairs not claimed slower than tree,
   HI = the smallest n with tree claimed faster, per family; TREE_BRUTE_MAX_ROWS = min LO, AUTO_TREE_LO/HI = (min LO, max HI); **bottom edge:** LO < 96 -> report, keep 128/136, extend the grid down;
   **top edge:** LO >= 160, or no HI in the grid -> report, keep 128/136 marked NOT RE-READ, extend up (168..256 are in the patch); never set a constant from an edge.
5. **C4-G4-kd** 3 processes (own K 3), 17.9 min (g4rT): `^bp_g4_(uniform|disparity)/tree_kd/(96|112|128)$`, 6 ids, recorded for F3's keep/freeze (ruling 4), not gating.
**TOTAL 326.5 min** (dryrun.txt; each pass includes >= 135 s of idle rule; runner/Jolt cells from the gate's dry samples, criterion cells from the model below).
**Block order differs from the cut** (cut: BR, G4, G4-kd, AB, G5): the merge gate (AB + G5, ~70 min, deterministic) runs FIRST so a cutoff cannot skip it when the criterion blocks run long. BR still precedes G4
(ruling 5's "check the all-pairs slowdown against window 7 first" is about reading G4). `python -B tools/mkrows9a.py --lane-order` restores the cut's order.

## The criterion cost (finding for the orchestrator)
A criterion process builds every group's scenes before it applies the filter, so the per-process STARTUP dominates: `--list` alone takes ~2.7 min per exe (measured, loaded); a 1-id `--test` process of g4rT took
12.2 min under this prep's heaviest load and **265.6 s (4.4 min) on a lighter one (others-busy 17.7 %)**; the 8-id BR samples took 7.5 min (g4r7, the older exe) and 14.6 min (g4r8b) under the heavy load. The cut priced BR at 1.8 min per process (13 s per id); with the startup it is ~5.3 min. Estimates
come from a model, not from the loaded samples: **est = 265 s + 6.6 s x ids** (window 8b's quiet window ran 60 ids in 653-668 s; one common load factor 2.8 is assumed to solve startup and per-id from this prep's
loaded measurements; +-30 %, tools/gate9a.py `part_estimates`). BR at K 9 is 47 % of the window. The instrument could build its scenes lazily (a bench-source change), which would cut BR to ~50 min at K 9, but
that changes the instrument M9 compares, so it was not done.

## Deviations from the lane's window9_c4_rows.json (amended; the cut's original is not used) - all in rows9a.json `_derived` and tools/mkrows9a.py
- C4-JOLT is not a block: the Jolt row is folded into C4-AB at W 1/2/4/8/16 (orchestrator instruction). C4-G4-kd is its own block (the driver takes K from the block). C4-G5 is included (the merge gate reads "and G5 recorded").
- **C4-S16 / C4-S16ap get pose_ref S16500** (recorded at pre-flight from TIP `--broadphase allpairs`: 0x4470add61a854f4c; the Tree arm equals it byte for byte). The lane's file had null, which would have passed `--expect-pose None.pose`.
- **C4-S16 expects TreeDiag static_rebuilds 0 / members 0** (16 rows <= 128: the Tree never builds, which is the brute-path proof). The lane's null meant the default {1,1,0}, which reds; the gate found it.
- **Armed rows: `--arm-profiler` is removed from the lane's args** (the driver appends it once for every armed row; the lane file had it twice on the command line, which the runner tolerated). Re-gated after the change: 4/4.
- g4r7 / g4r8b are byte copies under bin/ (the lane file pointed into two other sessions' scratchpads). Block order and K as above.

## Omitted / unavailable
- Nothing dropped from the lane's rows. Rapier: reserved (below). The C4-BR "g4rX" M9 intermediates do not exist (M9's verdict was (b): placement only, no code cause).
- DM1, S4-AB, SPLIT, F3, J-Son-T, omega blocks of 8b are not part of 9a and were removed from the copy (win8b is untouched).

## Untimed gate (gate/, logs/gate_*.out): all passed
- **Fixtures (TIP W1, 500 steps; `gate9a.py fixtures`):** JT500 0x30c5438bc6ad9ffa, JA500 0x30c5438bc6ad9ffa, RT500 0x6cbe24bf8fafda26 - each **byte-equal to window 8b's fixture** (C4 moves no pose: J-D = J-T after the flip); S16500 0x4470add61a854f4c (new).
- **Rows 44/44** (`gate9a.py rows`, the DRIVER's own validators): every runner row once at each of its W, 500 steps, `--expect-pose`: exit 0, pose = fixture, expect_pose match, void 0, config receipt, TreeDiag, canary_ns 60000 on the rung rows,
  armed rows: setup steps/tasks 0/0 at W1 and 500/15980 at W8 (both binaries S4 ON); the 5 Jolt cells exit 0, one stat line, threads = W, hash **0xb8522b4e3fc62cfe** at W 1/2/4/8/16, 500 frames, banner
  `boyko-parity-patch v1: damping 0 (Pyramid), no_pair_cache=0, allow_sleep=0, receipt=0` (sleeping off, no receipts, the Pyramid scene, `-q=Discrete`).
- **Pre-flight (cut.md 5), 5 W:** TIP flagless = Tree, tree_brute_max_rows 128, JT500 hash; PARENT flagless = AllPairs, 144, same hash; **TIP `--broadphase allpairs` == PARENT flagless byte for byte, and TIP flagless == PARENT flagless byte for byte, at W 1/2/4/8/16**;
  `--arm-profiler` TIP flagless W1/W8: void steps 0. **Red controls 4/4 exit 4** (501 steps vs JT500; `--contact-reuse off` vs JT500; both exes). R4: C4-S16 vs C4-S16ap pose bytes equal.
- **Criterion:** `--list` of every (row, exe) = exactly the expected ids: C4-BR 8/8 on g4r7, g4r8b and g4rT; C4-G4 36/36; C4-G4-kd 6/6; full-size samples of all 5 (row, exe) pairs (CRITERION_HOME per process, the driver's `criterion_check`: every expected id on stdout AND on disk, none extra): 8/8 on g4r7, g4r8b, g4rT, 36/36, 6/6, exit 0 (gate_criterion.json, gate_list.json).
- **Selftest (tools/selftest9a.py, gate/selftest9a.txt): 101 cases, 101 PASS, 0 FAIL, 0 SKIP; every rule can go red.** Real-record cases (relabelled exes, flipped rows, mutated summaries, TreeDiag, canary, S4, Jolt hash/threads/banner/stat lines, criterion missing/extra ids,
  verify_binaries with swapped exes and with the pins removed, fixtures, block order, adjacency incl. a permuted order) plus **the new K = 3 rule: pure cases and 6 end-to-end rehearsals with real 12-step processes** (`W9A_TEST_HOT` injects a hot pattern;
  no injection -> 3 processes; round 1 hot once -> 1 re-run, K 3; **hot twice -> the second re-run makes it clean (8b would have dropped it)**; never clean -> 4 waves then the stage ends, K 2 of 3, gates False; two slots (a clean slot leaves the stage);
  allowance 0 s -> no re-run) and counts9a.py's SHORT / "does not gate" lines. **Mutation proof** (logs/mutate_e2e.sh): each of four mutated copies of the FINAL driver (waves cap 1: 3 e2e cases red; gates always True: 2 e2e + 2 counts cases; a re-run rule that re-runs clean slots: 5 e2e; a census that counts unclean slots as clean: 2 e2e + 2 counts) turns cases red (logs/mutate_e2e.out, mut_*.txt).
- **Driver `--test`, every block:** C4-AB + C4-G5: exit 0, 44 timed + 2 warm-ups, 0 invalid, R4 1/1 (test/rehearsal_AB_G5); C4-BR (3 exes) + C4-G4: 4 criterion processes, 0 invalid (test/rehearsal_crit; its 50-min
  timeout killed the last block, so C4-G4-kd was rehearsed on its own: exit 0, 1 process, 0 invalid, test/rehearsal_kd); the reserved Rapier block (test/rapier_rehearsal, below). The shell launcher (`run_window9a.sh --test --blocks C4-G5`: WINDOW_DONE `exit 0 complete`, counts exit 0;
  test/rehearsal_shell_G5) and `--dry-run` through the shell ran once.
  Rehearsals set `W9A_TEST_HOT` to a no-op injection so a busy machine's real receipts cannot re-run criterion processes (12 min each under load).

## Adding the Rapier rows (reserved slot; no code change)
Rows come from D:/tmp/rapier-parity/window9a_rows.md after its fix round. The harness is runner-shaped (`--cfg --workers --steps --window --csv --pose-out --expect-pose --label`, one `SUMMARY {json}`, exit 0/2/3/4).
0. (Exercised: `gate9a.py rapier RAP-matched` with the scratch overlay ran the 5 W cells through `validate_rapier`, 5/5 PASS; its estimates were discarded.)
1. Copy the exe into `$W/bin` (e.g. `rapier_parity_simd8_<sha8>.exe`), add `sha256 *bin/<file>` to `bin/SHA256SUMS` and an entry to `bin/COMMIT.txt` (source state: the harness's SOURCES.sha256).
2. `cp rows9a.extra.example.json rows9a.extra.json` and edit: the `rapier` binary (`kind: "rapier"`, `sha256_pin`), the block (the example's priority 4.5 runs it after C4-BR and before C4-G4; choose the position, the driver merges and sorts by priority), one row per cfg
   (`rapier-default`, `matched`) with `expect_summary` (dotted SUMMARY paths; `"$W"` = the row's W; e.g. `workers`, `threads.pool_threads_min/max`, `void: false`, `voids: []`, `target_env: "msvc"`, `pose_hash`: the final-pose hash the rows file gives) and
   `expect_present`. A `<FILL...` left in place makes the row RED (nothing passes silently). To run a Rapier row adjacent to J-D each round, give it `"blocks": ["C4-AB"]` instead (it then shares C4-AB's K 9 and passes).
3. `python -B tools/gate9a.py rapier <ids>` (each row once at each W through the driver's `validate_rapier`; walls -> gate/estimates.json), then `bash run_window9a.sh --dry-run --start HH:MM` (the header prints "rows9a.json + rows9a.extra.json").
4. `W9A_EXTRA=<overlay> W9A_TEST_RAW=<dir> W9A_TEST_HOT=none#none@W0:0:0 python -B tools/window9a_run.py --test --blocks C4-RAPIER` rehearses the block (proved with the 2026-09-30 pre-fix simd8 harness and the scratch overlay
   test/rapier_overlay: 26 processes incl. re-runs, 0 invalid, test/rapier_rehearsal). `W9A_EXTRA` / `W9A_TEST_RAW` are honoured only under `--test` / `--dry-run`.
The window's own timing and the CSV: `run.csv` needs a `wall_ns` column (the harness's `step,wall_ns,...` CSV has it).

## Disclosure and cleanup
- Timing-shaped output that was displayed while inspecting files, all taken under load and unused: the 8 criterion time lines of g4r7's sample, one Jolt steps/s line, and whole-process walls (schedule estimates only: estimates.json). No verdict, no statistic.
- No git worktree was edited or checked out (exports only; `git status --short` clean in D:/wt/lighttable and D:/wt/joltab before and after). No commit. win8b is untouched (files copied out).
- Remove after the window: D:/wt/_targets/w9a-tip, w9a-parent, w9a-g4rT, w9a-trees (~2.5 GB). Nothing is left running.
