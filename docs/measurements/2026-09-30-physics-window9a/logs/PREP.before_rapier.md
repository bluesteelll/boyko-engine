# Physics window 9a: prep (2026-09-30; audit round 1 applied). PARENT = trunk 3d9433ae, TIP = u/phys-tree-c4 50e31f1a. Ready to launch. Nothing timed for a verdict; no engine timing number was used.
W = C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win9a
**Launch:** `bash $W/run_window9a.sh --cutoff HH:MM [--resume]`; preview `--dry-run --start HH:MM [--cutoff HH:MM]` (it prints the recommended cutoff); stop: create `$W/STOP`.
**Recommended cutoff = launch time + 392 min** (TOTAL 326.5 min x 1.2 = 391.8, rounded up): e.g. launch 20:00 -> `--cutoff 02:32`. The driver rolls a cutoff
earlier than now - 1 h to the next day. If the cutoff bites, blocks skip whole in priority order (the criterion blocks are last and
re-runnable: `--resume`). With C4-BR at K 3 (below) the total is 224.7 min: cutoff = launch + 270 min (20:00 -> 00:30).
**What the 65-min margin covers (audit-1 W1, `--dry-run` prints it: "RE-RUN PRICE"):** TOTAL leaves the re-run stages out. The MODELLED re-run time is **40 min** (window 8b's slot rate, 108 re-runs / 786 slots = 14 %, x the timed
wall; a model, unmeasured for the 5-8 min criterion processes) and fits in the margin. The WORST CASE, every pass spending its whole allowance, is **272 min** (AB 21, G5 7, BR 84, G4 132, kd 27) and does NOT fit: it is a bound,
not an expectation. What a noisy window loses is what runs last (G4, kd), whole passes at a time, and `--resume` recovers it. To let G4 finish its re-run stages on a noisy night widen the cutoff (launch + 480 min costs nothing on a quiet night:
the cutoff only bites when needed); that is the owner's call, the recommendation stays at the ruling-conform 1.2 x.
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
  (benches/broadphase.rs, +90 bytes; 17,586 files in both). Each build 1-2 min (`logs/build_*.log`). The audit re-checked all of it independently (exports byte for byte against `git archive`, 6/6 sha, exes equal to the build outputs); audit round 1 changed no binary.
- **Swap guards (a swapped exe is red three ways):** the sha pin; the runner `config` receipt `tree_brute_max_rows` (tip 128, parent 144) and `broadphase_select` "Manual",
  checked on EVERY process (`expect_config`); the row's own broadphase / TreeDiag expectation.

## Protocol (rows9a.json `protocol`)
Ruling 1: K 9 = 3 passes x 3 rounds, **p0 reversed, p1 forward, p2 reversed**, warm-up per pass (AB, G5), idle rule per pass, receipts, busy 5 %, others-busy 2 %, P-none,
placement receipt, --cutoff/--resume, a block that does not fit is skipped whole. Claim rule (analysis): IQR AND SE in every clean block and pooled, min-max beside, STRONG when it also passes.
**Own K:** C4-G4-kd 1 pass x 3 rounds (the Q3 recipe, F3 keep/freeze, not gating).
**Ruling 8 (W8B) - implemented in tools/window9a_run.py (run_pass, pending_reruns, passcell_table, rerun_allowance, rerun_admitted):** a slot is unclean when its latest attempt has a receipt before/after > 5 %, or others-busy
> 2 %, or a build process during the run, or is invalid. At the end of the pass the driver re-runs unclean slots in WAVES (each wave takes every slot still unclean, original order; a slot
leaves the stage at its first clean valid attempt) until every pass-cell has all its slots clean or **the pass ends = 4 waves, or the per-pass wall allowance is spent** (**audit-1 W1: the allowance is
`max(0.5 x the pass's own timed wall, 4 waves x 1.3 x the block's longest process)`**, see below), or the cutoff / STOP flag. Records: `attempt: "rerun"` + `rerun_no` 1.. + `rerun_reason`; leaf `_R`, `_R2`, ...; one `{"passcell": true, pc_row, pc_binary, pc_W, k_target, k_clean, reruns, gates, short}`
per (pass, cell) (no `row` key, like the R4 records). counts.txt prints the census and one `SHORT ... does not gate` line per short cell. **The reduction's slot rule: the original if clean and valid,
else the FIRST clean valid re-run, else the slot is dropped; a pass-cell with fewer than 3 clean slots does not gate.** (8b: 26 of 786 slots dropped after ONE re-run; 8b ran 108 re-runs.)
- **The cutoff / STOP flag RAISE out of `run_pass` (audit-1 O2; the first cut of this file said they "end the re-run stage", which was wrong):** a pass cut inside its re-run stage - all rounds finished, re-runs pending - writes NO passcell
  records and NO pass_done record, and `--resume` re-runs that pass whole (inherited from 8b). Its process records stay in runs.jsonl. `counts9a.py` now prints `incomplete passes ...` and one `INCOMPLETE <block>-p<n>` line for every (run, block, pass, attempt)
  with process records and neither pass_done nor voided_pass. **The analysis uses passes with a pass_done record only; a pass-cell exists only for a closed pass.** (Selftest cases over synthetic records.)

## The re-run allowance (audit-1 W1: a finding of the pre-launch audit, confirmed and fixed)
The first cut sized the allowance as 0.5 x the pass wall alone. On a ONE-cell criterion block that is 1.5 processes: C4-G4 allowed 762 s against a 508 s re-run, C4-G4-kd 465 s against 310 s, so both could take exactly ONE re-run
(a second is admitted only when the first ended under 254 s, impossible) - a slot whose first re-run was also hot was dropped, i.e. 8b's behaviour, the case ruling 8 removes, in the block that sets the shipped constants. Fixed in `rerun_allowance`:
`allowance = max(0.5 x rounds x per-round estimate, 4 waves x 1.3 x the block's longest process (estimate + receipt))`. The floor makes ANY single slot able to reach the full wave depth (4), and the 1.3 is the stated accuracy of the criterion
estimates (+-30 %, "The criterion cost" below): a process 25 % slower than estimated still leaves the fourth re-run admitted. Runner blocks keep the fraction (it is the larger term there). `--dry-run` prints, per block:
| block | allowance / pass | = max(fraction, floor) | back-to-back re-runs of the longest cell (at 25 % over its estimate) | worst case over the block's passes |
|---|---|---|---|---|
| C4-AB | 7.1 min | (7.1, 1.3) | 28 (22) | 21.3 min |
| C4-G5 | 2.4 min | (2.4, 1.4) | 9 (7) | 7.3 min |
| C4-BR | 28.0 min | (24.2, 28.0) | 5 (4) | 84.0 min |
| C4-G4 | 44.0 min | (12.7, 44.0) | 5 (4) | 132.1 min |
| C4-G4-kd | 26.9 min | (7.8, 26.9) | 5 (4) | 26.9 min |
The 4-wave cap still bounds a slot at 4 re-runs; the allowance bounds the whole stage (G4: 3 slots, so 4 re-runs, not 12). **Proof it can go red:** pure cases over the REAL blocks and estimates (a single always-hot slot reaches depth 4 at 25 % over its estimate; the pre-fix
fraction-only formula measures depth 1 on C4-G4 and C4-G4-kd and is kept as a negative control), the admission predicate's boundary, and three end-to-end rehearsals under the REAL allowance with real 12-step processes (`W9A_TEST_RERUN_S`
charges every re-run its own estimate, virtually: one slot hot 3 times -> 3 re-runs and K 3; never clean -> the 4-wave cap ends it, the allowance would admit 5; three slots hot once each -> all three re-run). Mutation proof below.

## Blocks (priority order), rows, timed processes, minutes (dryrun.txt; K 9 unless noted)
1. **C4-AB** 34 cells, 306 + 3 warm-ups, 50.1 min. Per W group, in this order (a reversed pass keeps every adjacency): C4-JA (parent, tip; W 1/8/16, cfg a, AllPairs control), C4-JT
   (parent, tip; W 1/8/16, `--broadphase tree` control; in TIP an A/A twin of J-D), **C4-jolt56 (j56; W 1/2/4/8/16)**, **C4-JD (tip; W 1/2/4/8/16, the new default, `--sleeping off`)**, C4-JDap
   (tip; `--broadphase allpairs`, the same-binary row of record), C4-JDpar (parent; the literal parent default), C4-rung (tip; W 8/16; `--canary-frac 1 --canary-ref-ns 60000`). The Jolt cell is adjacent to
   C4-JD#tip at every W and to C4-JT#tip at W 1/8/16 in every round (selftest cases); C4-JD and C4-JDap are adjacent. Warm-up C4-JD#tip@W8.
2. **C4-G5** 10 cells, 90 + 3 warm-ups, 22.3 min (TIP): C4-JD-armed / C4-JDap-armed (`--arm-profiler`) W 1/8; C4-R / C4-Rap (rest, RT500) W 1/8; C4-S16 / C4-S16ap (16 rows, W1; R4 pose compare). Warm-up C4-JD-armed@W8.
3. **C4-BR** 3 cells x K 9 = 27 criterion processes, 152.7 min: g4r7, g4r8b, g4rT adjacent in every round (p0/p2 reversed), 8 ids `^bp_g4_(uniform|disparity)/(all_pairs|tree)/(144|256)$`, CRITERION_HOME per process.
   **C4-BR at K 3** (the lane's Q5 recommendation; 9 processes, 50.9 min, TOTAL 224.7 min, cutoff = launch + 270 min): `cp alt/rows9a.br_k3.json rows9a.json` (generated by `python -B tools/mkrows9a.py --br-k3` then copied; alt/dryrun_br_k3.txt),
   then `--dry-run`. `python -B tools/mkrows9a.py` regenerates the K 9 default byte-identically (it overwrites rows9a.json). **Decision for the orchestrator (not blocking): K 9 is the letter of ruling 1 and is what this prep ships.**
4. **C4-G4** 9 processes, 83.5 min (g4rT): 36 ids `^bp_g4_(uniform|disparity)/(all_pairs|tree)/(96|104|...|160)$` (sizes 96..160 step 8). Rule (rows9a.json `lane_rules`): LO = the largest n with all_pairs not claimed slower than tree,
   HI = the smallest n with tree claimed faster, per family; TREE_BRUTE_MAX_ROWS = min LO, AUTO_TREE_LO/HI = (min LO, max HI); **bottom edge:** LO < 96 -> report, keep 128/136, extend the grid down;
   **top edge:** LO >= 160, or no HI in the grid -> report, keep 128/136 marked NOT RE-READ, extend up (168..256 are in the patch); never set a constant from an edge. The tree arm calls `set_brute_max_rows(0)`, so the re-read does not depend on TREE_BRUTE_MAX_ROWS = 128 (audit-checked).
5. **C4-G4-kd** 3 processes (own K 3), 17.9 min (g4rT): `^bp_g4_(uniform|disparity)/tree_kd/(96|112|128)$`, 6 ids, recorded for F3's keep/freeze (ruling 4), not gating.
**TOTAL 326.5 min** (dryrun.txt; each pass includes >= 135 s of idle rule; runner/Jolt cells from the gate's dry samples, criterion cells from the model below); **re-run stages are NOT in it** (price above).
**Block order differs from the cut** (cut: BR, G4, G4-kd, AB, G5): the merge gate (AB + G5, ~70 min, deterministic) runs FIRST so a cutoff cannot skip it when the criterion blocks run long. BR still precedes G4
(ruling 5's "check the all-pairs slowdown against window 7 first" is about reading G4). `python -B tools/mkrows9a.py --lane-order` restores the cut's order. (The header of run_window9a.sh said BR->G4->kd->AB->G5 until audit-1 O1; fixed.)

## The criterion cost (finding for the orchestrator)
A criterion process builds every group's scenes before it applies the filter, so the per-process STARTUP dominates: `--list` alone takes ~2.7 min per exe (measured, loaded); a 1-id `--test` process of g4rT took
12.2 min under this prep's heaviest load and **265.6 s (4.4 min) on a lighter one (others-busy 17.7 %)**; the 8-id BR samples took 7.5 min (g4r7, the older exe) and 14.6 min (g4r8b) under the heavy load. The cut priced BR at 1.8 min per process (13 s per id); with the startup it is ~5.3 min. Estimates
come from a model, not from the loaded samples: **est = 265 s + 6.6 s x ids** (window 8b's quiet window ran 60 ids in 653-668 s; one common load factor 2.8 is assumed to solve startup and per-id from this prep's
loaded measurements; +-30 %, tools/gate9a.py `part_estimates`). BR at K 9 is 47 % of the window. The instrument could build its scenes lazily (a bench-source change), which would cut BR to ~50 min at K 9, but
that changes the instrument M9 compares, so it was not done. **The audit read these estimates as upper bounds** (a quiet run is at most as slow as the loaded samples; the model is anchored to 8b's 60-id wall): the allowance floor above carries the 1.3 for the +-30 % in either direction.

## Deviations from the lane's window9_c4_rows.json (amended; the cut's original is not used) - all in rows9a.json `_derived` and tools/mkrows9a.py
- C4-JOLT is not a block: the Jolt row is folded into C4-AB at W 1/2/4/8/16 (orchestrator instruction). C4-G4-kd is its own block (the driver takes K from the block). C4-G5 is included (the merge gate reads "and G5 recorded").
- **C4-S16 / C4-S16ap get pose_ref S16500** (recorded at pre-flight from TIP `--broadphase allpairs`: 0x4470add61a854f4c; the Tree arm equals it byte for byte). The lane's file had null, which would have passed `--expect-pose None.pose`.
- **C4-S16 expects TreeDiag static_rebuilds 0 / members 0** (16 rows <= 128: the Tree never builds, which is the brute-path proof). The lane's null meant the default {1,1,0}, which reds; the gate found it.
- **Armed rows: `--arm-profiler` is removed from the lane's args** (the driver appends it once for every armed row; the lane file had it twice on the command line, which the runner tolerated). Re-gated after the change: 4/4.
- g4r7 / g4r8b are byte copies under bin/ (the lane file pointed into two other sessions' scratchpads). Block order and K as above. The audit diffed rows9a.json field by field against the lane's amended file: only these deviations.
- Audit round 1 (rows unchanged; the JSON diff against the pre-audit rows9a.json and alt/rows9a.br_k3.json shows only `protocol.rerun_rule` and the `_derived.deviations` text): W1 the allowance floor, W2 `insert_after`, O1 the launcher header, O2 the INCOMPLETE line.

## Omitted / unavailable
- Nothing dropped from the lane's rows. Rapier: reserved (below). The C4-BR "g4rX" M9 intermediates do not exist (M9's verdict was (b): placement only, no code cause).
- DM1, S4-AB, SPLIT, F3, J-Son-T, omega blocks of 8b are not part of 9a and were removed from the copy (win8b is untouched).

## Untimed gate (gate/, logs/gate_*.out): all passed
- **Fixtures (TIP W1, 500 steps; `gate9a.py fixtures`):** JT500 0x30c5438bc6ad9ffa, JA500 0x30c5438bc6ad9ffa, RT500 0x6cbe24bf8fafda26 - each **byte-equal to window 8b's fixture** (C4 moves no pose: J-D = J-T after the flip); S16500 0x4470add61a854f4c (new).
- **Rows 44/44** (`gate9a.py rows`, the DRIVER's own validators; not re-run in audit round 1: rows, validators, binaries and estimates are byte-unchanged - re-running it would overwrite gate/estimates.json with loaded walls and move the TOTAL for nothing; the audit re-ran it independently, 0 red, and the selftest re-validates the recorded cells on every run): every runner row once at each of its W, 500 steps, `--expect-pose`: exit 0, pose = fixture, expect_pose match, void 0, config receipt, TreeDiag, canary_ns 60000 on the rung rows,
  armed rows: setup steps/tasks 0/0 at W1 and 500/15980 at W8 (both binaries S4 ON); the 5 Jolt cells exit 0, one stat line, threads = W, hash **0xb8522b4e3fc62cfe** at W 1/2/4/8/16, 500 frames, banner
  `boyko-parity-patch v1: damping 0 (Pyramid), no_pair_cache=0, allow_sleep=0, receipt=0` (sleeping off, no receipts, the Pyramid scene, `-q=Discrete`).
- **Pre-flight (cut.md 5), 5 W:** TIP flagless = Tree, tree_brute_max_rows 128, JT500 hash; PARENT flagless = AllPairs, 144, same hash; **TIP `--broadphase allpairs` == PARENT flagless byte for byte, and TIP flagless == PARENT flagless byte for byte, at W 1/2/4/8/16**;
  `--arm-profiler` TIP flagless W1/W8: void steps 0. **Red controls 4/4 exit 4** (501 steps vs JT500; `--contact-reuse off` vs JT500; both exes). R4: C4-S16 vs C4-S16ap pose bytes equal.
- **Criterion:** `--list` of every (row, exe) = exactly the expected ids: C4-BR 8/8 on g4r7, g4r8b and g4rT; C4-G4 36/36; C4-G4-kd 6/6; full-size samples of all 5 (row, exe) pairs (CRITERION_HOME per process, the driver's `criterion_check`: every expected id on stdout AND on disk, none extra): 8/8 on g4r7, g4r8b, g4rT, 36/36, 6/6, exit 0 (gate_criterion.json, gate_list.json).
- **Selftest (tools/selftest9a.py, gate/selftest9a.txt): 148 cases, 148 PASS, 0 FAIL, 0 SKIP (was 101; +47 in audit round 1); every rule can go red.** Real-record cases (relabelled exes, flipped rows, mutated summaries, TreeDiag, canary, S4, Jolt hash/threads/banner/stat lines, criterion missing/extra ids,
  verify_binaries with swapped exes and with the pins removed, fixtures, block order, adjacency incl. a permuted order) plus **the K = 3 rule: pure cases and 9 end-to-end rehearsals with real 12-step processes** (`W9A_TEST_HOT` injects a hot pattern;
  no injection -> 3 processes; round 1 hot once -> 1 re-run, K 3; **hot twice -> the second re-run makes it clean (8b would have dropped it)**; never clean -> 4 waves then the stage ends, K 2 of 3, gates False; two slots (a clean slot leaves the stage);
  allowance 0 s -> no re-run; **and, audit-1, the same under the REAL per-block allowance: 3 cases, see above**) and counts9a.py's SHORT / "does not gate" / INCOMPLETE lines. New in audit round 1: the allowance cases over every real block (26), the overlay-placement cases (15, over the driver's own `--dry-run`
  with generated overlays, incl. refusals of a bad or misused `insert_after`), and the INCOMPLETE cases. **The pure cases now load the driver from `--driver` too** (before, only the end-to-end cases saw a mutated copy, so a mutation of a pure rule proved nothing).
  **Mutation proof** (logs/mutate_audit1.sh -> logs/mutate_audit1.out; logs/mutate_e2e.sh -> logs/mutate_e2e.out): each of twelve mutated copies of the FINAL driver / counts9a.py turns cases red - the four of the first cut (waves cap 1: 6 e2e + 4 other; gates always True: 3 + 2; a re-run rule that re-runs clean slots: 7 + 4; a census that counts unclean slots as clean: 3 + 4) and eight new:
  floor term dropped (9 red, the C4-G4 and C4-BR depth cases among them), loop falls back to the fraction inline (3 e2e - only an end-to-end case can see this one), admission always true (5), slack 1.0 (4), wave cap 9 (4), `insert_after` ignored (5), `insert_after` inserts before the anchor (6), counts9a INCOMPLETE rule removed (2).
  **A test that could not fail was found and fixed while doing this:** the first version of two INCOMPLETE cases returned the (empty) list of lines it should have found as its own failure list, so they stayed green when the line was absent; the mutation run showed 0 red, the cases were repaired, and the same shape in the older pass-cell census case was repaired too.
- **Driver `--test`, every block, on the FINAL driver (test/a1_*; `W9A_TEST_HOT` a no-op injection so a busy machine's real receipts cannot re-run criterion processes, 12 min each under load; the rehearsal runs 1 round of 1 pass, so its pass-cells have K 1 and "do not gate" - an artefact of the rehearsal, not a finding):**
  C4-AB + C4-G5: exit 0, 46 processes (44 timed + 2 warm-ups), 0 invalid, 0 non-zero exits, R4 1/1 equal (test/a1_AB_G5); the shell launcher `run_window9a.sh --test --blocks C4-G5`: WINDOW_DONE `exit 0 complete`, counts exit 0 (test/a1_shell_G5);
  the reserved Rapier block as its own block (pre-fix harness, scratch overlay): exit 0, 6 processes incl. 1 warm-up, W 1/2/4/8/16, 0 invalid (test/a1_rapier); **Rapier inside C4-AB with `insert_after: "C4-jolt56"`: exit 0, 39 timed, 0 invalid, and the EXECUTED order (records by seq, reversed pass 0) has C4-JD#tip and C4-jolt56#j56 on either side of the Rapier cell at every W (test/a1_ab_rapier)**;
  C4-BR (3 exes) + C4-G4 in one invocation: exit 0, 4 criterion processes (g4rT, g4r8b, g4r7, then g4rT on G4), every expected id on stdout AND on disk, 0 invalid, 0 non-zero exits, 2 passes closed (test/a1_crit); C4-G4-kd on its own (as in the first cut, so one long block cannot time out the next): exit 0, 1 process, 0 invalid (test/a1_kd).
  `--dry-run` through the shell is byte-identical to dryrun.txt.

## Adding the Rapier rows (reserved slot; no code change)
Rows come from D:/tmp/rapier-parity/window9a_rows.md after its fix round (the file does not exist yet). The harness is runner-shaped (`--cfg --workers --steps --window --csv --pose-out --expect-pose --label`, one `SUMMARY {json}`, exit 0/2/3/4).
0. (Exercised: `gate9a.py rapier RAP-matched` with the scratch overlay ran the 5 W cells through `validate_rapier`, 5/5 PASS; its estimates were discarded.)
1. Copy the exe into `$W/bin` (e.g. `rapier_parity_simd8_<sha8>.exe`), add `sha256 *bin/<file>` to `bin/SHA256SUMS` and an entry to `bin/COMMIT.txt` (source state: the harness's SOURCES.sha256).
2. `cp rows9a.extra.example.json rows9a.extra.json` and edit: the `rapier` binary (`kind: "rapier"`, `sha256_pin`), one row per cfg (`rapier-default`, `matched`) with `expect_summary` (dotted SUMMARY paths; `"$W"` = the row's W; e.g. `workers`, `threads.pool_threads_min/max`,
   `void: false`, `voids: []`, `target_env: "msvc"`, `pose_hash`: the final-pose hash the rows file gives) and `expect_present`. A `<FILL...` left in place makes the row RED (nothing passes silently). **Placement (audit-1 W2 corrected this step; every claim is a selftest case over the driver's own `--dry-run`, and (B) was also executed in a rehearsal):**
   - **(A) Own block (the example as shipped):** the block `priority` orders BLOCKS: **3.5 = after C4-BR and before C4-G4** (the example's value; the first cut said "4.5 runs it after C4-BR and before C4-G4", which was wrong: 4.5 runs it AFTER C4-G4, before C4-G4-kd). The Rapier cells run in their own passes, own warm-up, K 9.
   - **(B) Inside C4-AB (shared K 9 and passes; drop the block, set `"blocks": ["C4-AB"]`):** the position inside the block is the ROW-LIST order (cell_order walks the rows inside every W group). An overlay row WITHOUT `insert_after` is APPENDED: the LAST cell of every W group, after C4-JDpar#parent at W1/2/4 and after C4-rung#tip at W8/16 -
     **never next to C4-JD#tip** (the first cut claimed "adjacent to J-D each round"; wrong). To choose the neighbour give the NEW row `"insert_after": "<row id>"` (added in audit round 1; unknown anchor, or an anchor on a row that replaces an existing id, is refused at start):
     `"insert_after": "C4-jolt56"` -> Rapier sits BETWEEN C4-jolt56#j56 and C4-JD#tip at every W (mirrored in the reversed passes 0 and 2): adjacent to J-D each round, and to Jolt. **The cost:** Jolt and J-D#tip stop being neighbours; Jolt keeps C4-JT#tip at W1/8/16 and at W2/4 (no J-T cell) is next to no J-D/J-T cell.
     `"insert_after": "C4-JD"` would put it between J-D and J-Dap and break the merge-gate pair. J-D has exactly two neighbours (Jolt, J-Dap); no position gives Rapier J-D without taking one of them. Pick with the comparison you need, then look at the printed order.
   - Either way, `--dry-run` prints every pass's cell order (`each round: ...`): **read it before launching**.
3. `python -B tools/gate9a.py rapier <ids>` (each row once at each W through the driver's `validate_rapier`; walls -> gate/estimates.json), then `bash run_window9a.sh --dry-run --start HH:MM` (the header prints "rows9a.json + rows9a.extra.json"; the re-run price and the recommended cutoff are re-printed for the new total).
4. `W9A_EXTRA=<overlay> W9A_TEST_RAW=<dir> W9A_TEST_HOT=none#none@W0:0:0 python -B tools/window9a_run.py --test --blocks C4-RAPIER` (or `--blocks C4-AB` for placement B) rehearses it (proved with the 2026-09-30 pre-fix simd8 harness and scratch overlays: 6 and 39 processes, 0 invalid).
   `W9A_EXTRA` / `W9A_TEST_RAW` are honoured only under `--test` / `--dry-run`. Then `python -B tools/selftest9a.py` (the placement cases read only rows9a.json plus generated overlays, not your overlay).
The window's own timing and the CSV: `run.csv` needs a `wall_ns` column (the harness's `step,wall_ns,...` CSV has it).

## Open question (audit) and what to read after the first G5 pass
C4-S16 / C4-S16ap processes run about 0.05 s, so the 2 % others-busy witness is about one 15.6 ms scheduler tick of CPU from any other process; window 8b had no process shorter than 0.61 s, so how often these runs are flagged is UNMEASURED.
Nothing was changed for it (no threshold was moved on a guess). After the first C4-G5 pass read `raw/counts.txt`: the `witness` column and the `SHORT ... does not gate` lines for C4-S16 / C4-S16ap. G5's allowance (2.4 min = 9 re-runs of its longest cell, and an S16 re-run costs ~5.5 s) absorbs a moderate rate; if
the S16 cells are flagged wholesale, their pass-cells simply do not gate (K < 3) and the R4 pose compare (bytes, not timing) still stands.

## Disclosure and cleanup
- Timing-shaped output that was displayed while inspecting files, all taken under load and unused: the 8 criterion time lines of g4r7's sample, one Jolt steps/s line, and whole-process walls (schedule estimates only: estimates.json). No verdict, no statistic. Audit round 1 read and printed only schedule estimates
  (allowances, the dry-run minutes, both derived from gate/estimates.json) and the wall of the selftest itself (~45 s, not an engine measurement); no engine timing was read.
- **Observation for the orchestrator, NOT used (one sample per exe, machine at 5-25 % CPU from other lanes):** the four 1-id criterion rehearsal processes each took about 2.5 min by the window log's timestamps, below the model's 265 s startup ("The criterion cost"). That is consistent with the audit's reading of the estimates as upper bounds; no estimate was re-fitted on it, so the dry-run total keeps the model's number and a quiet window may finish BR / G4 earlier than scheduled.
- No git worktree was edited or checked out (exports only; `git status --short` clean in D:/wt/lighttable and D:/wt/joltab before and after, re-checked in audit round 1). No commit. win8b is untouched (files copied out). Pre-audit copies of every file edited in audit round 1: logs/*.before_audit1.*.
- Remove after the window: D:/wt/_targets/w9a-tip, w9a-parent, w9a-g4rT, w9a-trees (~2.5 GB). Nothing is left running.
