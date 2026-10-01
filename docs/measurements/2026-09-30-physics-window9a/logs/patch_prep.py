"""Patch PREP.md for the Rapier block (2026-09-30). Run from the window dir:  python -B logs/patch_prep.py"""
p = 'PREP.md'
s = open(p, encoding='utf-8').read()


def rep(old, new):
    global s
    assert s.count(old) == 1, (s.count(old), old[:100])
    s = s.replace(old, new)


# ---- title + launch header
rep("# Physics window 9a: prep (2026-09-30; audit round 1 applied). PARENT = trunk 3d9433ae, TIP = u/phys-tree-c4 50e31f1a. Ready to launch. Nothing timed for a verdict; no engine timing number was used.",
    "# Physics window 9a: prep (2026-09-30; audit round 1 applied; **the Rapier block C4-RAPIER and C4-BR at K 3 were added later the same day: see \"Rapier block\" below, which supersedes the numbers of the first cut**). PARENT = trunk 3d9433ae, TIP = u/phys-tree-c4 50e31f1a. Ready to launch. Nothing timed for a verdict; no engine timing number was used.")
rep("""**Recommended cutoff = launch time + 392 min** (TOTAL 326.5 min x 1.2 = 391.8, rounded up): e.g. launch 20:00 -> `--cutoff 02:32`. The driver rolls a cutoff
earlier than now - 1 h to the next day. If the cutoff bites, blocks skip whole in priority order (the criterion blocks are last and
re-runnable: `--resume`). With C4-BR at K 3 (below) the total is 224.7 min: cutoff = launch + 270 min (20:00 -> 00:30).""",
    """**Recommended cutoff = launch time + 313 min** (TOTAL 260.2 min x 1.2 = 312.2, rounded up; C4-RAPIER in, C4-BR at K 3): e.g. launch 20:00 -> `--cutoff 01:13`. The driver rolls a cutoff
earlier than now - 1 h to the next day. If the cutoff bites, blocks skip whole in priority order (the criterion blocks are last and
re-runnable: `--resume`). (First cut, before the Rapier block: TOTAL 326.5 min, launch + 392 min; C4-BR at K 3 without Rapier: 224.7 min.)""")

# ---- the new section, before "## Binaries"
new_section = """## Rapier block C4-RAPIER, and C4-BR at K 3 (2026-09-30, after the audit; orchestrator rulings (a) (b) (c)) - READY
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
   3 rounds: 60 timed + 2 warm-ups, 0 invalid; in every round the EXECUTED order has the four configurations adjacent at every W (W groups 16, 8, 4, 2, 1 = the reversed pass 0); both exes used, each by its own path and sha256 (`python -B tools/rehearsal_summary9a.py <dir>`). (Pass-cells of a 1-round rehearsal have K 1 and "do not gate": an artefact, as before.)
   **Selftest: `python -B tools/selftest9a.py` = 256 cases, 256 PASS, 0 FAIL, 0 SKIP (was 148; gate/selftest9a.txt)**: +the block's structure (order, 20 cells, adjacency x3 passes, K, two warm-ups, pins and fixtures with negative controls, swapped rs8/rs4 exes red), the validator on the 20 REAL gate cells (all green) and 40 mutations
   naming their rule (wrong pool_threads, `counters_enabled` true in a timed row, wrong arm features / exe sha256, a pose mismatch, a missing SUMMARY, ... each red for its own rule only), the `--test` shape, an end-to-end rehearsal (1 injected hot slot re-run once, 2 warm-ups, executed order) and the WIRING case; C4-BR's K 3 + its marks.
   The old Rapier cases of section 6 (the data-driven schema, pre-fix harness exe) were REPLACED, not deleted: they tested an API that no longer exists; the placement cases of section 8 now run over an overlay GENERATED from the real one (test/rapier_overlay/rows9a.extra.json is rewritten by the selftest).

**Files changed / new (backups of every changed file: logs/*.before_rapier.*):** tools/window9a_run.py (VOID_NAMES, HANG_S_RAPIER, overlay `rapier` key, per-binary `pose_ref`, `rapier_check` / `validate_rapier`, `warmups_of`, `verify_rapier_pins`), tools/gate9a.py (`record` carries cwd/args; `part_rapier` per-binary fixture, cells under gate/rapier/cells),
tools/selftest9a.py, tools/mkrows9a.py (BR K 3 default, `--br-k9`), tools/wait_idle9a.ps1 (two names; parse-checked with a zero-poll run), run_window9a.sh (header comment only), bin/SHA256SUMS, bin/COMMIT.txt, rows9a.json (regenerated: only BR block / row note / own_k / one deviation differ),
gate/fixtures/fixtures.json (+4), gate/estimates.json (+20). New: rows9a.extra.json, tools/mkextra9a.py, tools/xcheck_rapier9a.py, tools/rehearsal_summary9a.py, gate/rapier/, gate/xcheck_rapier9a.txt, logs/mutate_rapier.{py,out}, test/rapier9a/.
**Left as it was:** rows9a.extra.example.json (a placement template; the selftest still checks its claims), `raw/` (empty), no STOP / WINDOW_DONE / progress.txt.

"""
rep("## Binaries (bin/SHA256SUMS", new_section + "## Binaries (bin/SHA256SUMS")

# ---- binaries table: the two Rapier exes
rep("""| j56 | D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe | 918fd2b70ecc634cc0574cb676c56a3c929a46a51173cf9aa76fdd628e2ef0ad | Jolt 5.6, run in place |""",
    """| j56 | D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe | 918fd2b70ecc634cc0574cb676c56a3c929a46a51173cf9aa76fdd628e2ef0ad | Jolt 5.6, run in place |
| rs8 | bin/rapier_parity_simd8_736c2a06.exe | 736c2a069f14a2ad4846cb787cb14f080ca2f1401896130dc7188f4410c9b307 | Rapier 0.36 parity harness, arm simd8 (8 lanes), byte copy of D:/tmp/rapier-parity/gate/bin/rapier-parity-simd8.exe (2026-09-30) |
| rs4 | bin/rapier_parity_simd4_1e5136ca.exe | 1e5136cada9777ab1bdf5eb1abb463bde6e395caaa5a58282779eb13ae1e0b77 | the same harness, arm simd4 (4 lanes: rapier3d without `simd8`), byte copy of rapier-parity-simd4.exe |""")

# ---- blocks list: BR at K 3, TOTAL
rep("""3. **C4-BR** 3 cells x K 9 = 27 criterion processes, 152.7 min: g4r7, g4r8b, g4rT adjacent in every round (p0/p2 reversed), 8 ids `^bp_g4_(uniform|disparity)/(all_pairs|tree)/(144|256)$`, CRITERION_HOME per process.
   **C4-BR at K 3** (the lane's Q5 recommendation; 9 processes, 50.9 min, TOTAL 224.7 min, cutoff = launch + 270 min): `cp alt/rows9a.br_k3.json rows9a.json` (generated by `python -B tools/mkrows9a.py --br-k3` then copied; alt/dryrun_br_k3.txt),
   then `--dry-run`. `python -B tools/mkrows9a.py` regenerates the K 9 default byte-identically (it overwrites rows9a.json). **Decision for the orchestrator (not blocking): K 9 is the letter of ruling 1 and is what this prep ships.**""",
    """3. **C4-BR** 3 cells, **K 3 = 1 pass x 3 rounds (orchestrator ruling (b), 2026-09-30): DIAGNOSTIC ONLY, NOT claim-bearing** = 9 criterion processes, 50.9 min: g4r7, g4r8b, g4rT adjacent in every round (the one pass is reversed), 8 ids `^bp_g4_(uniform|disparity)/(all_pairs|tree)/(144|256)$`, CRITERION_HOME per process.
   (The first cut shipped K 9 = 27 processes, 152.7 min; the letter is `python -B tools/mkrows9a.py --br-k9` -> alt/rows9a.br_k9.json, alt/dryrun_br_k9.txt, TOTAL 362.0 min with Rapier. The mark is in the block note, the row note, `protocol.own_k` and `_derived.deviations`.)""")
rep("""**TOTAL 326.5 min** (dryrun.txt; each pass includes >= 135 s of idle rule; runner/Jolt cells from the gate's dry samples, criterion cells from the model below); **re-run stages are NOT in it** (price above).""",
    """**TOTAL 260.2 min** (dryrun.txt, regenerated 2026-09-30 with C4-RAPIER and BR at K 3; the first cut's 326.5 min is superseded; each pass includes >= 135 s of idle rule; runner/Jolt/Rapier cells from the gate's dry samples, criterion cells from the model below); **re-run stages are NOT in it** (price in "Rapier block").""")
rep("""1. **C4-AB** 34 cells, 306 + 3 warm-ups, 50.1 min.""", """0. **Block list as of 2026-09-30:** C4-AB, **C4-RAPIER (prio 1.5, new)**, C4-G5, C4-BR (K 3), C4-G4, C4-G4-kd; the C4-RAPIER cells and minutes are in "Rapier block" above. The numbered list below is the first cut's, kept for its cell detail.
1. **C4-AB** 34 cells, 306 + 3 warm-ups, 50.1 min.""")

# ---- omitted / unavailable
rep("Nothing dropped from the lane's rows. Rapier: reserved (below).", "Nothing dropped from the lane's rows. Rapier: added 2026-09-30 (\"Rapier block\" above).")

# ---- untimed gate: the selftest count of the first cut
rep("""- **Selftest (tools/selftest9a.py, gate/selftest9a.txt): 148 cases, 148 PASS, 0 FAIL, 0 SKIP (was 101; +47 in audit round 1); every rule can go red.**""",
    """- **Selftest (tools/selftest9a.py, gate/selftest9a.txt): 148 cases, 148 PASS, 0 FAIL, 0 SKIP (was 101; +47 in audit round 1) - AT THE FIRST CUT; now 256 cases, 256 PASS (see "Rapier block"); every rule can go red.**""")

# ---- the reserved-slot recipe: mark as done
rep("""## Adding the Rapier rows (reserved slot; no code change)
Rows come from D:/tmp/rapier-parity/window9a_rows.md after its fix round (the file does not exist yet). The harness is runner-shaped""",
    """## Adding the Rapier rows (the reserved-slot recipe; STEPS 1-4 WERE DONE 2026-09-30, see "Rapier block" above - the text below is kept for the placement recipe (B), which the selftest still checks, and is NOT the state of the tree)
Rows came from D:/tmp/rapier-parity/gate/window9a_rows.md (now a copy at gate/rapier/window9a_rows.md; the schema in step 2 below - `expect_summary` / `expect_present` - was REPLACED by the V1-V9 validator: a rapier row now carries `pose_ref` per binary, `config_pin`, `rows_pin`, and its binary an `arm`). The harness is runner-shaped""")

# ---- disclosure
rep("""## Disclosure and cleanup
""", """## Disclosure and cleanup
- **Rapier step (2026-09-30), timing-shaped output that was displayed, all unused:** (1) while listing the top-level keys of ONE harness gate log SUMMARY (g2t, simd8 matched W4, the harness's own structural run) the `window_mean_ns` and `window_steps_per_s` fields were printed;
  no number was kept or used (the validators read only `wall_ns > 0`; xcheck prints rule sets only); (2) the timestamps of `logs/gate_rapier.out` (one line per untimed gate process) and the per-cell schedule estimates in dryrun.txt are process WALLS of untimed gate runs, i.e. the schedule's inputs (gate/estimates.json), not a comparison of arms or engines and not read as one.
  No engine, arm or config was compared on time. The two `--test` rehearsals and the selftest's end-to-end cases ran 12-step processes.
""")
open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('patched', len(s))
