# Physics window 9a: prep (2026-09-30; audit round 1 applied; **the Rapier block C4-RAPIER and C4-BR at K 3 were added later the same day: see "Rapier block" below, which supersedes the numbers of the first cut**). PARENT = trunk 3d9433ae, TIP = u/phys-tree-c4 50e31f1a. Ready to launch. Nothing timed for a verdict; no engine timing number was used.
W = C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win9a
**Launch:** `bash $W/run_window9a.sh --cutoff HH:MM [--resume]`; preview `--dry-run --start HH:MM [--cutoff HH:MM]` (it prints the recommended cutoff); stop: create `$W/STOP`.
**Recommended cutoff = launch time + 313 min** (TOTAL 260.2 min x 1.2 = 312.2, rounded up; C4-RAPIER in, C4-BR at K 3): e.g. launch 20:00 -> `--cutoff 01:13`. The driver rolls a cutoff
earlier than now - 1 h to the next day. If the cutoff bites, blocks skip whole in priority order (the criterion blocks are last and
re-runnable: `--resume`). (First cut, before the Rapier block: TOTAL 326.5 min, launch + 392 min; C4-BR at K 3 without Rapier: 224.7 min.)
**What the 52-min margin covers (audit-1 W1, `--dry-run` prints it: "RE-RUN PRICE"; numbers as of 2026-09-30 with C4-RAPIER and BR at K 3 - the first cut said 65 min / 40 min / 272 min):** TOTAL leaves the re-run stages out. The MODELLED re-run time is **31 min** (window 8b's slot rate, 108 re-runs / 786 slots = 14 %, x the timed
wall; a model, unmeasured for the 5-8 min criterion processes) and fits in the margin. The WORST CASE, every pass spending its whole allowance, is **229 min** (AB 21, RAPIER 14, G5 7, BR 28, G4 132, kd 27) and does NOT fit: it is a bound,
not an expectation. What a noisy window loses is what runs last (G4, kd), whole passes at a time, and `--resume` recovers it. To let G4 finish its re-run stages on a noisy night widen the cutoff (launch + 480 min costs nothing on a quiet night:
the cutoff only bites when needed); that is the owner's call, the recommendation stays at the ruling-conform 1.2 x.
Outputs as window 8b (raw/runs.jsonl, progress.txt, raw/counts.txt, WINDOW_DONE 0/2/3/5). **The idle rule cannot pass while another lane
builds or runs anything under D:/wt/_targets** (it did not during this prep: gate/wait_idle_smoke.txt; the machine ran at 60-65 % CPU).

## Rapier block C4-RAPIER, and C4-BR at K 3 (2026-09-30, after the audit; orchestrator rulings (a) (b) (c)) - READY
The reserved Rapier slot is filled (steps 1-4 of "Adding the Rapier rows" below, done) and the three rulings are applied. **Untimed only**: every process below was a gate or a rehearsal;
no engine timing number was read as a result (disclosure at the end of this file). No git worktree was edited; the harness dir D:/tmp/rapier-parity was only READ (no file in it is newer than this session's start; its exes were
copied into bin/). D: 69 GB free at the end; nothing is left running.

**Schedule (dryrun.txt, `--dry-run --start 20:00 --cutoff 01:13`; shell and python dry-runs byte-identical):**
| prio | block | cells x K | timed processes + warm-ups | block minutes | note |
|---|---|---|---|---|---|
| 1 | C4-AB | 34 x 9 | 306 + 3 | 50.1 | unchanged |
| **1.5** | **C4-RAPIER** | **20 x 9** | **180 + 6** | **35.5** (11.8 per pass) | new; 2 exes x 2 configs x W 1/2/4/8/16; warm-ups RP-D rs8 W8 + RP-D rs4 W8 per pass |
| 2 | C4-G5 | 10 x 9 | 90 + 3 | 22.3 | unchanged |
| 3 | C4-BR | 3 x **3** | 9 + 0 | 50.9 | **K 3 = 1 pass x 3 rounds, DIAGNOSTIC ONLY, NOT claim-bearing** (was K 9, 152.7 min) |
| 4 | C4-G4 | 1 x 9 | 9 + 0 | 83.5 | unchanged |
| 5 | C4-G4-kd | 1 x 3 | 3 + 0 | 17.9 | unchanged |
**TOTAL 260.2 min** (first cut 326.5; BR at K 3 without Rapier 224.7; with BR at the letter AND Rapier 362.0 = `alt/dryrun_br_k9.txt`). RE-RUN PRICE (not in TOTAL): model 31 min, worst case 229 min; the 52-min margin of 1.2 x covers the
modelled 31 min, not the worst case (as before). Per-cell estimates come from the untimed gate walls of `gate9a.py rapier` (gate/estimates.json +20 cells); the Rapier block's allowance is 4.6 min per pass (floor 1.2).
**Launch line:** `bash $W/run_window9a.sh --cutoff HH:MM` with **HH:MM = launch time + ceil(TOTAL x 1.2) = launch + 313 min** (launch 20:00 -> `--cutoff 01:13`; `bash $W/run_window9a.sh --dry-run --start HH:MM` prints the same "RECOMMENDED --cutoff" line).
`rows9a.extra.json` is merged automatically (the header of the dry-run says "rows9a.json + rows9a.extra.json"); nothing else to copy or set. `--resume`, the STOP flag and WINDOW_DONE are as before.

**Ruling (a): the block.** `C4-RAPIER`, priority 1.5 (after C4-AB, before C4-G5), K 9 = 3 passes x 3 rounds, pass order p0 reversed / p1 forward / p2 reversed. Rows RP-D (rapier-default) and RP-M (matched) are PARSED from
gate/rapier/window9a_rows.md (the reviewed file; `tools/mkextra9a.py` writes rows9a.extra.json and never edits by hand), each with binaries rs8, rs4 and W 1/2/4/8/16, 500 steps, window [0,500], metric windows [0,100) and [100,500).
The forward order inside a W group is RP-D rs8, RP-D rs4, RP-M rs8, RP-M rs4 (the doc's), so **the four configurations are adjacent within each W in each round** - proved three ways: selftest cases over `cell_order` / `pass_order` for
all 3 passes (plus a permuted-order negative control), the `--dry-run` cell lists, and the EXECUTED order of the `--test` rehearsals (rounds 0..2). **Adjacency to our C4-JD / C4-JT rows is NOT required and does not hold**: C4-AB runs all
its passes before C4-RAPIER starts, so ours `J-T` (C4-AB, 20:00-20:50 in the dry-run timeline) and a Rapier cell (C4-RAPIER, 20:50-21:25) are from a few minutes to ~70 min apart in wall time. The doc's protocol line "ours and Rapier stay adjacent in every round" is therefore waived by ruling (a); the analysis must read
R-WALL-D / R-WALL-M across blocks with that drift in mind (the rule's IQR/SE spreads still apply; nothing in the driver compensates).
**One warm-up per exe per pass** (window9a_rows.md, Protocol): the driver takes `warmup` (one triple) as before or `warmups` (a list); C4-RAPIER carries `warmups` [RP-D rs8 W8, RP-D rs4 W8] (a driver change: `warmups_of`, `run_pass`, `block_estimate`,
`--dry-run`; C4-AB / C4-G5 keep their single `warmup` and print exactly as before; selftest cases incl. mutation proofs).

**Ruling (b): C4-BR at K 3.** `python -B tools/mkrows9a.py` (the default) now generates C4-BR with block `passes` 1 x `rounds` 3 = K 3 (one reversed pass: g4rT, g4r8b, g4r7 each round) and marks it DIAGNOSTIC ONLY / NOT claim-bearing in the block
note, the C4-BR row note, `protocol.own_k` and `_derived.deviations` of rows9a.json (a selftest case reads all three). The letter of ruling 1 (K 9) is `python -B tools/mkrows9a.py --br-k9` (alt/rows9a.br_k9.json, alt/dryrun_br_k9.txt); the
old `--br-k3` spelling is accepted and means the default. The default regenerates byte-identically (checked). Consequences for the reading: BR's three exes are compared from ONE pass of three rounds (no pass-to-pass replication, no claim); the
analysis reads BR as a bracket receipt only. C4-G4's "check against BR first" (ruling 5) is a diagnostic reading, as M9's verdict (b) already said.

**Ruling (c): the validator.** `validate_rapier` / `rapier_check` in tools/window9a_run.py implement EXACTLY the void rules V1-V9 of gate/rapier/window9a_rows.md (written from the rules' text; the driver no longer reads the previous data-driven `expect_summary` / `expect_present` fields, which rows9a.extra.example.json, a placement template, still shows):
V1 exit != 0 / crash / hang (a Rapier process alive past `HANG_S_RAPIER` = 300 s is terminated by its own handle); V2 not exactly one parsable SUMMARY; V3 pool receipts == W (min/max, install_receipt [W,W], loop_on_pool_worker, and the CSV
`pool_threads` column == W on all 500 rows); V4 build receipt (rapier3d 0.36.0, the arm's features/arm/lanes, avx2+fma+bmi2 exactly, no debug assertions, msvc, **the exe's sha256 computed from the file that ran** == the pin, source hashes == the pinned sources);
V5 the `config` object == the (arm, cfg) timed pin, `counters_enabled` false included; V6 scene identity all true + spawn hash + perturb null + not void; V7 final pose bytes == the arm's fixture (+ the exe's own `expect_pose` match and the pose hash);
V8 the CSV is exactly `step,wall_ns,pool_threads` + 500 rows, steps 0..499, wall_ns > 0 (only its sign is read); V9 no `--receipt` (launch args, SUMMARY args, `receipt_gates`) and the prep twin of (cfg, arm, W) passed. `--test` skips V7 only (12-step rehearsals have no 500-step fixture).
Where the rules' text is silent the choice is the stricter one (listed in `rapier_check`'s docstring): V3 also requires the SUMMARY `workers` and `threads.pool_threads_requested` == W (the harness's void_check.py does too; window9a_rows.md lists four V3 clauses, not these two),
V4 msvc + debug_assertions false + `target_features` exactly {avx2, fma, bmi2}, V6 1241 colliders, V7 SUMMARY `pose_hash` == the pin, an empty pose/fixture is a V7 failure, JSON `true` is never the integer 1.
**Cross-check against the reference (tools/xcheck_rapier9a.py -> gate/xcheck_rapier9a.txt): 1,279 checks, 0 disagreeing** (gate/rapier/void_check.py is a byte copy of the harness's, sha256 equal; the rule SETS are compared, not the text):
A. the harness's 20 g2t timed-shape twins: both VALID (20/20). B. the harness's own 33 gv mutations (gate/rapier/mutations_ref.py, a verbatim excerpt of analyze.py, checked equal at run time) on its two baselines (66) and on all 20 twins (660): identical rule sets.
C. the harness's real g5 red-control runs (44 logs: exit 4 pose flips / trunc / pad / other cfg / steps 501, exit 2 missing fixture, exit 3 counters off under --receipt, exit 0 counters forced on in a timed shape = V5): identical rule sets. D. 51 extra mutations x 4 twins (204 cases) for rules the 33 do not reach
(workers/requested, target_env, debug assertions, lanes, an extra target feature, an absent config field, install step, colliders, void flag, receipt_gates, empty/extra CSV rows ...): identical. E. 210 two-mutation combinations: identical.
F. seven driver-only cases (launch args carry --receipt, an empty fixture, a hang flag, steps 12 vs 500 rows, JSON true at W1, `test=True` skips V7 only, `test=True` still flags V1): red in the driver as required.
**The validator can go red (mutation proof, logs/mutate_rapier.py -> logs/mutate_rapier.out): 42 of 42 mutated copies of the driver are caught** by the selftest and/or the cross-check (V1-V9 clauses dropped one by one, the exe hash never computed, the fixture fixed to RD8,
hang / exit ignored, the pins' sha not checked, the fixture sha / fixtures.json hash not checked, warm-ups truncated, `test` skipping everything, `run_one` never calling the validator, `run_one` calling the RUNNER validator ...). Control: the unmutated driver is green in both gates.
**A test that could not fail was found and fixed while doing this:** the first version had nothing proving `run_one` applies the validator to a REAL process (the unit cases call `validate_rapier` directly), so a `run_one` that skipped it stayed green; the mutation run showed it, and an end-to-end
WIRING case now runs the block with the simd4 exe under the simd8 key and demands every real process INVALID with V4 (and ruling 8 re-running those invalid slots for its 4 waves). Likewise a `test` flag that skipped every rule was caught only by the cross-check until pure `--test`-shape cases were added.

**Steps 1-4 of "Adding the Rapier rows" (done):**
1. bin/: `rapier_parity_simd8_736c2a06.exe` (key rs8, arm simd8) and `rapier_parity_simd4_1e5136ca.exe` (key rs4, arm simd4), byte copies (sha256 checked three ways: the ruling, the harness's SHA256SUMS, gate/rapier/pins.json) with lines in bin/SHA256SUMS and entries in bin/COMMIT.txt
   (sources = the harness's SOURCES.sha256, re-checked equal to its current files). The `det` exe (f34291c5...) is a receipt build and is NOT a window arm: not copied. Underscore names on purpose: the idle rule and the witness list name the harness's own hyphenated exes (wait_idle9a.ps1 `$build`, the driver's `VOID_NAMES`), so a copy under that name would void its own window.
2. rows9a.extra.json (generated: 2 rows, 2 binaries, 1 block, `rapier` = pins path + its sha256), gate/rapier/ (pins.json, void_check.py, mutations_ref.py, window9a_rows.md, SOURCES.sha256, features_*.txt, the 20 g2t twins, the 44 g5 logs = byte copies), gate/fixtures/RD8 RD4 RM8 RM4 .pose
   (sha256 == the pins' fixture_sha256; RD4 == RD8 and RM4 == RM8 as files, so the pose check cannot tell the arms apart - V4 does) + their hashes in fixtures.json. `verify_fixtures` re-checks the pins file's sha256, every fixture's sha256 and hash at every start and in `--dry-run` (selftest negative controls).
3. `python -B tools/gate9a.py rapier RP-D RP-M` (every row id at each W, 500 steps, `--expect-pose`, the driver's own `validate_rapier`): **20/20 PASS**, exit 0 for all (logs/gate_rapier.out, gate/gate_rapier.json; cells under gate/rapier/cells/; walls -> gate/estimates.json as schedule estimates only). `--dry-run`: 6 blocks, "binaries: all match", "fixture hashes: every pose_ref recorded",
   "estimates: every cell has an untimed dry sample".
4. `--test` rehearsals of C4-RAPIER (`W9A_TEST_RAW=test/rapier9a/rehearsal_{1r,3r} W9A_TEST_HOT=none#none@W0:0:0 python -B tools/window9a_run.py --test --blocks C4-RAPIER [--test-rounds 3]`): exit 0 both; 1 round: 20 timed + 2 warm-ups (RP-D rs8 W8, RP-D rs4 W8), 0 invalid, 0 non-zero exits, 20 pass-cells;
   3 rounds: 60 timed + 2 warm-ups, 0 invalid; in every round the EXECUTED order has the four configurations adjacent at every W (W groups 16, 8, 4, 2, 1 = the reversed pass 0); both exes used, each by its own path and sha256 (`python -B tools/rehearsal_summary9a.py <dir>`). Through the shell launcher: `W9A_TEST_HOT=none#none@W0:0:0 bash run_window9a.sh --test --blocks C4-RAPIER`: WINDOW_DONE `exit 0 / complete`, counts exit 0, 20 timed + 2 warm-ups, 0 invalid (kept as test/rapier9a/shell_rehearsal). (Pass-cells of a 1-round rehearsal have K 1 and "do not gate": an artefact, as before.)
   **Selftest: `python -B tools/selftest9a.py` = 256 cases, 256 PASS, 0 FAIL, 0 SKIP (was 148; gate/selftest9a.txt)**: +the block's structure (order, 20 cells, adjacency x3 passes, K, two warm-ups, pins and fixtures with negative controls, swapped rs8/rs4 exes red), the validator on the 20 REAL gate cells (all green) and 40 mutations
   naming their rule (wrong pool_threads, `counters_enabled` true in a timed row, wrong arm features / exe sha256, a pose mismatch, a missing SUMMARY, ... each red for its own rule only), the `--test` shape, an end-to-end rehearsal (1 injected hot slot re-run once, 2 warm-ups, executed order) and the WIRING case; C4-BR's K 3 + its marks.
   The old Rapier cases of section 6 (the data-driven schema, pre-fix harness exe) were REPLACED, not deleted: they tested an API that no longer exists; the placement cases of section 8 now run over an overlay GENERATED from the real one (test/rapier_overlay/rows9a.extra.json is rewritten by the selftest).

**Files changed / new (backups of every changed file: logs/*.before_rapier.*):** tools/window9a_run.py (VOID_NAMES, HANG_S_RAPIER, overlay `rapier` key, per-binary `pose_ref`, `rapier_check` / `validate_rapier`, `warmups_of`, `verify_rapier_pins`), tools/gate9a.py (`record` carries cwd/args; `part_rapier` per-binary fixture, cells under gate/rapier/cells),
tools/selftest9a.py, tools/mkrows9a.py (BR K 3 default, `--br-k9`), tools/wait_idle9a.ps1 (two names; parse-checked with a zero-poll run), run_window9a.sh (header comment only), bin/SHA256SUMS, bin/COMMIT.txt, rows9a.json (regenerated: only BR block / row note / own_k / one deviation differ),
gate/fixtures/fixtures.json (+4), gate/estimates.json (+20). New: rows9a.extra.json, tools/mkextra9a.py, tools/xcheck_rapier9a.py, tools/rehearsal_summary9a.py, gate/rapier/, gate/xcheck_rapier9a.txt, logs/mutate_rapier.{py,out}, test/rapier9a/.
**Left as it was:** rows9a.extra.example.json (a placement template; the selftest still checks its claims), `raw/` (empty), no STOP / WINDOW_DONE / progress.txt.

## Binaries (bin/SHA256SUMS + rows9a.json `sha256_pin`; bin/COMMIT.txt; rustc 1.98.1 msvc, --locked, lock unchanged, RUSTFLAGS unset)
| key | file | sha256 | source |
|---|---|---|---|
| tip | bin/runner_tip_50e31f1a.exe | 8d6e7d4173857ca83b47cd350890b8e178451fb48f030ac3d25aaef33cd268c3 | git archive 50e31f1a -> D:/wt/_targets/w9a-trees/tip, `--profile parity`, target w9a-tip |
| parent | bin/runner_parent_3d9433ae.exe | a09fca085613f966e09b3e8cd78ed711aa1c51f069cfab649834d93f88988478 | git archive 3d9433ae -> .../parent, same build, target w9a-parent |
| g4rT | bin/broadphase_g4rT_50e31f1a.exe | 19c9eb1f67cc728505525b5d669adfd2db133a1ae461217fcab7f386bc7c0bcf | TIP export + bin/g4ref.patch (the G4_SIZES line, by content), bench profile, target w9a-g4rT |
| g4r7 | bin/bpbench_g4ref_93b2615b.exe | f96a9c11ebd72d4d581784cf37728fd6da15bc63b28414d7e895667753033fd4 | window 7's exe, byte copy (sha checked) |
| g4r8b | bin/broadphase_g4ref_16191fda.exe | b887850f38633d62419344c41080d1bada92bc931c3307989b6f6fb314a33b6d | window 8b's exe, byte copy (sha checked) |
| j56 | D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe | 918fd2b70ecc634cc0574cb676c56a3c929a46a51173cf9aa76fdd628e2ef0ad | Jolt 5.6, run in place |
| rs8 | bin/rapier_parity_simd8_736c2a06.exe | 736c2a069f14a2ad4846cb787cb14f080ca2f1401896130dc7188f4410c9b307 | Rapier 0.36 parity harness, arm simd8 (8 lanes), byte copy of D:/tmp/rapier-parity/gate/bin/rapier-parity-simd8.exe (2026-09-30) |
| rs4 | bin/rapier_parity_simd4_1e5136ca.exe | 1e5136cada9777ab1bdf5eb1abb463bde6e395caaa5a58282779eb13ae1e0b77 | the same harness, arm simd4 (4 lanes: rapier3d without `simd8`), byte copy of rapier-parity-simd4.exe |
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
0. **Block list as of 2026-09-30:** C4-AB, **C4-RAPIER (prio 1.5, new)**, C4-G5, C4-BR (K 3), C4-G4, C4-G4-kd; the C4-RAPIER cells and minutes are in "Rapier block" above. The numbered list below is the first cut's, kept for its cell detail.
1. **C4-AB** 34 cells, 306 + 3 warm-ups, 50.1 min. Per W group, in this order (a reversed pass keeps every adjacency): C4-JA (parent, tip; W 1/8/16, cfg a, AllPairs control), C4-JT
   (parent, tip; W 1/8/16, `--broadphase tree` control; in TIP an A/A twin of J-D), **C4-jolt56 (j56; W 1/2/4/8/16)**, **C4-JD (tip; W 1/2/4/8/16, the new default, `--sleeping off`)**, C4-JDap
   (tip; `--broadphase allpairs`, the same-binary row of record), C4-JDpar (parent; the literal parent default), C4-rung (tip; W 8/16; `--canary-frac 1 --canary-ref-ns 60000`). The Jolt cell is adjacent to
   C4-JD#tip at every W and to C4-JT#tip at W 1/8/16 in every round (selftest cases); C4-JD and C4-JDap are adjacent. Warm-up C4-JD#tip@W8.
2. **C4-G5** 10 cells, 90 + 3 warm-ups, 22.3 min (TIP): C4-JD-armed / C4-JDap-armed (`--arm-profiler`) W 1/8; C4-R / C4-Rap (rest, RT500) W 1/8; C4-S16 / C4-S16ap (16 rows, W1; R4 pose compare). Warm-up C4-JD-armed@W8.
3. **C4-BR** 3 cells, **K 3 = 1 pass x 3 rounds (orchestrator ruling (b), 2026-09-30): DIAGNOSTIC ONLY, NOT claim-bearing** = 9 criterion processes, 50.9 min: g4r7, g4r8b, g4rT adjacent in every round (the one pass is reversed), 8 ids `^bp_g4_(uniform|disparity)/(all_pairs|tree)/(144|256)$`, CRITERION_HOME per process.
   (The first cut shipped K 9 = 27 processes, 152.7 min; the letter is `python -B tools/mkrows9a.py --br-k9` -> alt/rows9a.br_k9.json, alt/dryrun_br_k9.txt, TOTAL 362.0 min with Rapier. The mark is in the block note, the row note, `protocol.own_k` and `_derived.deviations`.)
4. **C4-G4** 9 processes, 83.5 min (g4rT): 36 ids `^bp_g4_(uniform|disparity)/(all_pairs|tree)/(96|104|...|160)$` (sizes 96..160 step 8). Rule (rows9a.json `lane_rules`): LO = the largest n with all_pairs not claimed slower than tree,
   HI = the smallest n with tree claimed faster, per family; TREE_BRUTE_MAX_ROWS = min LO, AUTO_TREE_LO/HI = (min LO, max HI); **bottom edge:** LO < 96 -> report, keep 128/136, extend the grid down;
   **top edge:** LO >= 160, or no HI in the grid -> report, keep 128/136 marked NOT RE-READ, extend up (168..256 are in the patch); never set a constant from an edge. The tree arm calls `set_brute_max_rows(0)`, so the re-read does not depend on TREE_BRUTE_MAX_ROWS = 128 (audit-checked).
5. **C4-G4-kd** 3 processes (own K 3), 17.9 min (g4rT): `^bp_g4_(uniform|disparity)/tree_kd/(96|112|128)$`, 6 ids, recorded for F3's keep/freeze (ruling 4), not gating.
**TOTAL 260.2 min** (dryrun.txt, regenerated 2026-09-30 with C4-RAPIER and BR at K 3; the first cut's 326.5 min is superseded; each pass includes >= 135 s of idle rule; runner/Jolt/Rapier cells from the gate's dry samples, criterion cells from the model below); **re-run stages are NOT in it** (price in "Rapier block").
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
- Nothing dropped from the lane's rows. Rapier: added 2026-09-30 ("Rapier block" above). The C4-BR "g4rX" M9 intermediates do not exist (M9's verdict was (b): placement only, no code cause).
- DM1, S4-AB, SPLIT, F3, J-Son-T, omega blocks of 8b are not part of 9a and were removed from the copy (win8b is untouched).

## Untimed gate (gate/, logs/gate_*.out): all passed
- **Fixtures (TIP W1, 500 steps; `gate9a.py fixtures`):** JT500 0x30c5438bc6ad9ffa, JA500 0x30c5438bc6ad9ffa, RT500 0x6cbe24bf8fafda26 - each **byte-equal to window 8b's fixture** (C4 moves no pose: J-D = J-T after the flip); S16500 0x4470add61a854f4c (new).
- **Rows 44/44** (`gate9a.py rows`, the DRIVER's own validators; not re-run in audit round 1: rows, validators, binaries and estimates are byte-unchanged - re-running it would overwrite gate/estimates.json with loaded walls and move the TOTAL for nothing; the audit re-ran it independently, 0 red, and the selftest re-validates the recorded cells on every run): every runner row once at each of its W, 500 steps, `--expect-pose`: exit 0, pose = fixture, expect_pose match, void 0, config receipt, TreeDiag, canary_ns 60000 on the rung rows,
  armed rows: setup steps/tasks 0/0 at W1 and 500/15980 at W8 (both binaries S4 ON); the 5 Jolt cells exit 0, one stat line, threads = W, hash **0xb8522b4e3fc62cfe** at W 1/2/4/8/16, 500 frames, banner
  `boyko-parity-patch v1: damping 0 (Pyramid), no_pair_cache=0, allow_sleep=0, receipt=0` (sleeping off, no receipts, the Pyramid scene, `-q=Discrete`).
- **Pre-flight (cut.md 5), 5 W:** TIP flagless = Tree, tree_brute_max_rows 128, JT500 hash; PARENT flagless = AllPairs, 144, same hash; **TIP `--broadphase allpairs` == PARENT flagless byte for byte, and TIP flagless == PARENT flagless byte for byte, at W 1/2/4/8/16**;
  `--arm-profiler` TIP flagless W1/W8: void steps 0. **Red controls 4/4 exit 4** (501 steps vs JT500; `--contact-reuse off` vs JT500; both exes). R4: C4-S16 vs C4-S16ap pose bytes equal.
- **Criterion:** `--list` of every (row, exe) = exactly the expected ids: C4-BR 8/8 on g4r7, g4r8b and g4rT; C4-G4 36/36; C4-G4-kd 6/6; full-size samples of all 5 (row, exe) pairs (CRITERION_HOME per process, the driver's `criterion_check`: every expected id on stdout AND on disk, none extra): 8/8 on g4r7, g4r8b, g4rT, 36/36, 6/6, exit 0 (gate_criterion.json, gate_list.json).
- **Selftest (tools/selftest9a.py, gate/selftest9a.txt): 148 cases, 148 PASS, 0 FAIL, 0 SKIP (was 101; +47 in audit round 1) - AT THE FIRST CUT; now 256 cases, 256 PASS (see "Rapier block"); every rule can go red.** Real-record cases (relabelled exes, flipped rows, mutated summaries, TreeDiag, canary, S4, Jolt hash/threads/banner/stat lines, criterion missing/extra ids,
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

## Adding the Rapier rows (the reserved-slot recipe; STEPS 1-4 WERE DONE 2026-09-30, see "Rapier block" above - the text below is kept for the placement recipe (B), which the selftest still checks, and is NOT the state of the tree)
Rows came from D:/tmp/rapier-parity/gate/window9a_rows.md (now a copy at gate/rapier/window9a_rows.md; the schema in step 2 below - `expect_summary` / `expect_present` - was REPLACED by the V1-V9 validator: a rapier row now carries `pose_ref` per binary, `config_pin`, `rows_pin`, and its binary an `arm`). The harness is runner-shaped (`--cfg --workers --steps --window --csv --pose-out --expect-pose --label`, one `SUMMARY {json}`, exit 0/2/3/4).
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
- **Rapier step (2026-09-30), timing-shaped output that was displayed, all unused:** (1) while listing the top-level keys of ONE harness gate log SUMMARY (g2t, simd8 matched W4, the harness's own structural run) the `window_mean_ns` and `window_steps_per_s` fields were printed;
  no number was kept or used (the validators read only `wall_ns > 0`; xcheck prints rule sets only); (2) the timestamps of `logs/gate_rapier.out` (one line per untimed gate process) and the per-cell schedule estimates in dryrun.txt are process WALLS of untimed gate runs, i.e. the schedule's inputs (gate/estimates.json), not a comparison of arms or engines and not read as one.
  No engine, arm or config was compared on time. The two `--test` rehearsals and the selftest's end-to-end cases ran 12-step processes.
- Timing-shaped output that was displayed while inspecting files, all taken under load and unused: the 8 criterion time lines of g4r7's sample, one Jolt steps/s line, and whole-process walls (schedule estimates only: estimates.json). No verdict, no statistic. Audit round 1 read and printed only schedule estimates
  (allowances, the dry-run minutes, both derived from gate/estimates.json) and the wall of the selftest itself (~45 s, not an engine measurement); no engine timing was read.
- **Observation for the orchestrator, NOT used (one sample per exe, machine at 5-25 % CPU from other lanes):** the four 1-id criterion rehearsal processes each took about 2.5 min by the window log's timestamps, below the model's 265 s startup ("The criterion cost"). That is consistent with the audit's reading of the estimates as upper bounds; no estimate was re-fitted on it, so the dry-run total keeps the model's number and a quiet window may finish BR / G4 earlier than scheduled.
- No git worktree was edited or checked out (exports only; `git status --short` clean in D:/wt/lighttable and D:/wt/joltab before and after, re-checked in audit round 1). No commit. win8b is untouched (files copied out). Pre-audit copies of every file edited in audit round 1: logs/*.before_audit1.*.
- Remove after the window: D:/wt/_targets/w9a-tip, w9a-parent, w9a-g4rT, w9a-trees (~2.5 GB). Nothing is left running.
