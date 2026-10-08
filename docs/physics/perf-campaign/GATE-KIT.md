# Physics gate scripts: exit codes, root rule, and how lanes call them

Lane gate-kit, 2026-10-08. Tooling only: no engine source, no test assertion, no hash, fixture or row
set changed. On a green run every gate script prints the same bytes as before (stdout and its verdict
files), so every pinned value and tally on record still applies.

## What changed and why

The two gate directories are `docs/measurements/2026-09-30-v2-speculative/gate/` (the runner pins, the
L10 pose gates on the V2 and the d = 0 fixtures, and now the S5 runner gate) and
`docs/physics/perf-campaign/levers/scenes/gate/` (the dynamic-scene gates: ours, Jolt 5.6, Rapier).

| reason | before | now |
|---|---|---|
| (a) exit codes | every script exited 0 whatever its rows said; a failing run read as green to anyone trusting `$?` | each exits 0 only when green, 1 when red, 2 when it could not run (contract below) |
| (b) fixture paths | the L10 scripts, their generated `_v2` copies and the generator hard-coded `D:/wt/merge`, so lanes copied the directory and rewrote the paths with `sed` (a missed `sed` silently read another worktree's fixtures) | the scripts resolve the checkout they live in (root rule below); they run in place |
| (b) line endings | the V2 gate directory had no attribute, so `core.autocrlf` checked its scripts out with CRLF and lanes ran `dos2unix` | `docs/measurements/2026-09-30-v2-speculative/gate/.gitattributes` holds `*.sh text eol=lf` (the scenes gates already had theirs) |
| (c) S5's runner gate | lived only in a scratch directory | `docs/measurements/2026-09-30-v2-speculative/gate/s5_runner_gate.sh`, committed from S5's r3 copy with the same rows and row format |
| (d) waiting on long gates | ad-hoc sleep loops in an agent's background shell, whose reaping kills the children | the machine-local kit (last section) |

## The exit-code contract

| code | meaning |
|---|---|
| 0 | the script ran to its end, every row or check passed, every negative control got its wanted exit, and the tally is complete: the count equals the script's expected count |
| 1 | the script ran to its end and is red: a FAIL row, a negative control at the wrong exit, a count that is not the expected one, or (wrappers) a child that is red or whose green line is missing from its section of `all.txt` |
| 2 | the script could not run: bad usage, the root cannot be resolved or is not a checkout, the runner exe or another input is missing, a fixture is missing, or a helper it needs failed. Decided before the first row, and no tally is printed. The pose wrappers report a child's 2 after all three children ran (each child decides its own 2 before its first row) |

Every new message goes to stderr and appears only on a failing path, so a green run's output is unchanged.

| script | expected count (a constant in the script) | 2 also when | notes |
|---|---|---|---|
| `runner_pins.sh` | rows 12 (2 W x 6) | - | reads no fixture |
| `l10/pose_gates_sync.sh`, `pose_gates_sync_v2.sh` | runs 264 (2 W x (2 arms x 61 + 10)), and both negative controls at exit 4 | a fixture of L9, L9C4, L10, L10OFF or W6 missing | the negative-control lines are unchanged and are now part of the exit code |
| `l10/runner_checks_c3c.sh`, `runner_checks_c3c_v2.sh` | checks 26 (14 + 2 W x 6) | an L10 fixture missing | |
| `l10/runner_checks_off.sh`, `runner_checks_off_v2.sh` | checks 17 (7 + 2 W x 5) | an L10OFF fixture missing | |
| `pose_gates_v2.sh`, `pose_gates_d0.sh` | the five green lines, each whole and inside its own child's section of `all.txt` | a child exited 2 | `ALL_DONE` is still the last line in every case: it means the wrapper ran to its end, not that it passed |
| `runner_d0.sh` | - | - | the runner's own code; 127 when `RUNNER_D0_EXE` is unset or not a file (outside the runner's codes 0/2/3/4/101, so a row that wants 2 cannot pass on it) |
| `s5_runner_gate.sh` | rows 54 (7 W x 2 arms x 3 + 6 R1100 + 6 RowWalk) | a fixture missing | |
| `ours_gates.sh` | checks 12 per program, +1 for kick's prefix (49 for the default four) | an unknown program, `dyn_spec_ref.py` failed | a missing pin fixture is a FAIL row (exit 1), not 2 |
| `jolt_gates.sh` | checks 9 + 11 per program (53 for the default four); pass + the SAME/DIFFERS receipts = checks | an unknown program; a missing input: the exe, the patch, the clean v5.6.0 repository, the 9b exe, the 9b banner, the G6 source; `dyn_spec_ref.py` failed | the cross-W receipts never fail (Jolt promises no W-independence); a green run reads `checks 53, pass 49, fail 0` |
| `rapier_gates.py` | at least one check; G2 must select at least one row (211 for `--gate all`) | an argparse error, an invalid `--root`, a missing input (the Rapier tree, `frozen9b.json`, `pins.json`, the dyn or frozen exe directory) | an uncaught exception stays Python's 1 |

**Changing a count.** A lane that changes a script's row set changes its expected count in the same
commit, says so in the commit message, and re-derives the arithmetic in the comment beside it. A count is
never loosened to make a run pass: a dropped row or a lost child tally is exactly what the count catches.

The python checkers the scenes gates call (`compare_dumps.py`, `dyn_sanity.py`, `pose_pin.py`) already
exited non-zero on a FAIL and are unchanged. The recorders (`record_v2_fixtures.sh`, `receipts/receipts.sh`)
keep exiting 0: they record, they do not gate.

## The root rule

Each script that reads fixtures finds them in ONE checkout, chosen like this:

| script | how the root is given | default |
|---|---|---|
| `l10/*.sh`, `*_v2.sh`, `pose_gates_v2.sh`, `pose_gates_d0.sh`, `s5_runner_gate.sh` | `--root DIR` as the FIRST argument | the git toplevel of the script's own directory |
| `jolt_gates.sh` | the third positional argument `<repo>` (the checkout whose patch G1 applies) | the git toplevel of the script's own directory |
| `rapier_gates.py` | the option `--root DIR` (anywhere), validated with `git rev-parse` | the script's own checkout (six levels above its directory, the root `pose_pin.py` uses) |
| `ours_gates.sh` | none | `pose_pin.py`'s file-relative root: the script's own checkout |
| `runner_pins.sh` | none | it reads no fixture |

- The bash scripts share one block of identical text between `# gate-root BEGIN` and `# gate-root END`.
  It ignores an inherited `GIT_DIR` / `GIT_WORK_TREE`, converts the root to the mixed form `D:/...` (the
  native runner reads it; MSYS argument conversion is never relied on), and exits 2 when the root cannot
  be resolved, contains a space, or has no `docs/measurements`.
- The wrappers resolve the root once and pass `--root` to every child.
- `make_v2_scripts.py` writes the `_v2` scripts with `$ROOT/<repo-relative path>` fixture directories and
  asserts that no `<drive>:/wt/` checkout path survives in its output. The `_v2` scripts are generated, never edited by hand: edit
  `l10/` and re-run the generator.
- **Never copy a gate script.** A copy outside a checkout exits 2 (`cannot resolve the repo root`) unless
  it is given `--root`; a copy inside another checkout reads that checkout. Run the scripts in place from
  the lane's worktree.

## Line endings

`docs/measurements/2026-09-30-v2-speculative/gate/.gitattributes` holds `*.sh text eol=lf`, covering
`l10/` too; `docs/physics/perf-campaign/levers/scenes/.gitattributes` already covered the scenes gates.
Every gate script is checked out with LF in every checkout, so no `dos2unix` step remains. A checkout
made before this change keeps its CRLF copy of an unchanged file until git rewrites it; every gate script
but `record_v2_fixtures.sh` (a recorder) was changed by this lane, so a merge or switch rewrites them.
Whether the repository should carry a root `.gitattributes` is an open owner decision
(`docs/OPEN-QUESTIONS.md`, the entry "No `.gitattributes`, and the worktree rule just armed what that
costs"), which this lane does not take.

## How a lane calls the gates

In place, from the lane's worktree `${WT}`, quoting the exit code and the tally line together:

    bash ${WT}/docs/measurements/2026-09-30-v2-speculative/gate/runner_pins.sh <exe> <out>       # rows 12, pass 12, fail 0
    bash ${WT}/docs/measurements/2026-09-30-v2-speculative/gate/pose_gates_v2.sh <exe> <out_v2>   # <out_v2>/all.txt
    bash ${WT}/docs/measurements/2026-09-30-v2-speculative/gate/pose_gates_d0.sh <exe> <out_d0>   # <out_d0>/all.txt
    bash ${WT}/docs/measurements/2026-09-30-v2-speculative/gate/s5_runner_gate.sh <exe> <out>     # rows 54, pass 54, fail 0
    bash ${WT}/docs/physics/perf-campaign/levers/scenes/gate/ours_gates.sh <exe> <out>            # checks 49, pass 49, fail 0
    bash ${WT}/docs/physics/perf-campaign/levers/scenes/gate/jolt_gates.sh <jolt dyn exe> <out>   # checks 53, pass 49, fail 0
    python ${WT}/docs/physics/perf-campaign/levers/scenes/gate/rapier_gates.py <out> --gate all   # checks 211, fail 0

Each `all.txt` reads both `neg ... exit=4 (want 4)` lines, `runs 264, pass 264, fail 0`,
`checks 26, pass 26, fail 0`, `checks 17, pass 17, fail 0` and `ALL_DONE`. The l10 scripts are never run
bare against the V2 runner: they carry the pre-V2 fixtures and run only through `pose_gates_d0.sh`.

## The kit (machine-local)

Gates that run longer than about two minutes go through the kit at `D:/tmp/phys-orch/kit/` on the owner's
machine (outside the repository; its `README.md` has every command, budget and expected tally, and its
self-test). `launch.ps1` starts one gate detached in its own minimized console under a supervisor that
enforces a budget in minutes (a Job object, so the kill reaches every process the gate started) and
writes `exit_code` then `DONE`, or `exit_code` 124 then `KILLED` (a finished gate's leftovers keep running,
as under a plain shell). `wait.sh <out>` waits one bounded slice:
0 = the gate finished (**not** that it passed: read its `VERDICT` line), 3 = still running, 4 = killed,
dead or over budget. `summary.sh <summary.tsv> <out>...` writes one row per gate and exits 0 only when
every gate is DONE with exit code 0. Whether to version the kit in the repository is a later decision.

## What did not change

Every pinned hash in every gate script, every fixture, `fixtures.json`, `logs/record.tsv`, `pins.json`,
every row set, every row's text in `pose_gates.tsv`, `runner_checks*.tsv`, `pins.tsv`, `rows.tsv` and
`gates.tsv`, and the tally lines themselves.
