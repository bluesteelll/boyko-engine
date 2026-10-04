# V2 speculative contacts — the parity runner's fixtures at the flip

The pose fixtures the trunk's pose gates read, re-recorded for V2's contact rule (speculative
contacts at `d = 20 mm` plus the approach-velocity margin, owner V2a/V2b and rulings 2026-09-30
items 9-10; design record `docs/physics/perf-campaign/levers/V2-speculative/01-DESIGN.md`).
The fixtures from before V2 are never overwritten: window 8's
(`docs/measurements/2026-09-27-physics-window8/gate/fixtures/`), window 6's, L9's and L10's stay
the references of the overlap-only rule, which every runner row reproduces with
`--speculative-distance 0 --speculative-velocity-cap 0`.

## How they were recorded

`gate/record_v2_fixtures.sh <runner> <this directory>` with the flip's parity
runner (`cargo build -p boyko-physics --release --bench jolt_parity_pyramid`; its sha256 is in
`fixtures.json`), every row at `--workers 1` with `--pose-out`. Each `*_W8.pose` is a COPY of its
`*_W1.pose` (critique W5): a W8 gate row then checks worker-count identity against the W1 bytes,
never a W8 recording against itself. Rows and arguments are the ones the L9/L10/W6/W8 gate scripts
run, verbatim; `w8/JSonToff500` is new (the fidelity gate's fourth anchor: gap 0.5, sleeping on,
contact reuse off, 500 steps).

| directory | replaces (the pre-V2 references) |
|---|---|
| `l9-reuse-on/` | `2026-09-23-l9-contact-reuse/fixtures-c4/` |
| `l9-reuse-off/` | `2026-09-23-l9-contact-reuse/fixtures/` |
| `l10-reuse-on/` | `2026-09-23-l10-sleeping/fixtures/` |
| `l10-reuse-off/` | `2026-09-23-l10-sleeping/fixtures-reuse-off/` |
| `w6/` | `2026-09-24-physics-window6/gate/fixtures/` (+ `R1100`) |
| `w8/` | `2026-09-27-physics-window8/gate/fixtures/` and window 8b's `JSonT500` / `RT500` |

`fixtures.json` lists every row: arguments, exit code, pose hash, the pose file's sha256 (and its
W8 copy's), `first_frozen_step`, the final manifold count and the tree's diag, read from each row's
`SUMMARY` line (the full runner outputs carry wall-clock fields from a shared machine and are not
committed; they are in the lane's scratch). `logs/record.tsv` is the recorder's table.

`receipts/`: the work receipts window 9 re-reads (untimed counts): armed W1 rows over steps
[100, 500) of the synced tip's runner and of the parent `16191fda`'s — manifolds, points, colours,
colour passes and velocity rows, J-T with contact reuse on and off, at V2's default and with both
overlap-only flags. The overlap-only rule's receipts equal the parent's exactly.

## The gate scripts (`gate/`)

The pose gates that read these fixtures, committed beside them (they lived only in session
scratch, and a temp-directory cleanup deleted the trunk's originals on 2026-10-01; the three in
`gate/l10/` were restored byte-for-byte from the session transcripts that created them —
`pose_gates_sync.sh` and `runner_checks_off.sh` of the L10 merge, `runner_checks_c3c.sh` of L10
C3c — two of the three cross-checked against a later `cat` of the live files). Every fixture path
in them is `D:/wt/merge/...`, as in the originals: run a copy with the paths replaced on another
checkout, and strip carriage returns first on a `core.autocrlf=true` checkout.

| script | what it runs | want |
|---|---|---|
| `l10/pose_gates_sync.sh`, `l10/runner_checks_c3c.sh`, `l10/runner_checks_off.sh` | the trunk's L10 pose gates on the pre-V2 fixtures, unchanged | (through `pose_gates_d0.sh`) |
| `pose_gates_d0.sh <exe> <out>` | the three above, every row through `runner_d0.sh` (both overlap-only flags appended) | 264/264, both negatives exit 4, 26/26, 17/17 |
| `make_v2_scripts.py` | writes `*_v2.sh` from `l10/`: the five fixture directories re-pointed here and the six pinned hashes replaced by this directory's recordings of the same rows | — |
| `pose_gates_v2.sh <exe> <out>` | the three `*_v2.sh` | 264/264, both negatives exit 4, 26/26, 17/17 (the rest pile's `Sets` rows exited VOID before the probe fix below) |
| `runner_pins.sh <exe> <out>` | the trunk's runner pins J500, J500 reuse-off, R1100 at W 1 and 8: flagless they read V2's recordings, with both flags the pre-V2 pins | 12/12 |
| `record_v2_fixtures.sh <exe> <dir>` | the recorder that wrote this directory | 35 rows |

## The rows that exited VOID, and the probe that voided them

Under the flip's runner (its sha256 in `fixtures.json`, which keeps every exit code as recorded)
four rows recorded their pose but exited 3 (VOID): `l9-reuse-on/R-S`, `l10-reuse-on/R-S`,
`l10-reuse-on/R-on` and `w6/RSOffp800` — the rest pile with sleeping on, contact reuse on, in the
`Sets` sleep-skip mode. The runner's L9 probe voided every reuse-on row in which no pair reused its
record over steps [100, 500). Under V2 the rest pile freezes at step 86 (before V2: step 250), so
from step 87 every row is held and the narrowphase collides no box pair at all in that window. The
runner's own pair classes (W1, 600 steps, `--window`; "collided" is `sep_hits + reused + full`, the
box pairs the narrowphase ran, and the candidate pairs also count the ones sleep-skip held):

| rule, sleep-skip mode | window | candidate pairs | box pairs collided | records reused | first frozen step |
|---|---|---|---|---|---|
| V2, `Sets` | [100, 500) | 0 | 0 | 0 | 86 |
| V2, `Sets` | [0, 100) | 842,160 | 823,020 | 709,565 | 86 |
| V2, `Off` | [100, 500) | 3,828,000 | 3,828,000 | 3,422,000 | 86 |
| overlap-only (both flags), `Sets` | [0, 100) | 957,000 | 957,000 | 748,333 | 250 |
| overlap-only (both flags), `Sets` | [100, 500) | 1,454,640 | 1,435,500 | 1,161,882 | 250 |

The gates count a nonzero exit as a FAIL, so `pose_gates_v2.sh` read 228/264, both negatives exit
4, 19/26 and 17/17, every one of the 43 failures this VOID with its pose matching (`expect_pose:
match`, or the pinned hash).

**The probe since the V2 lane's triage round 1** (option (a) of the lane's ruling question: exempt
a window that collided no box pair). A reuse-on row is void iff its window collided a box pair and
reused no record there — the rule before, unchanged on every such row — or iff its window collided
no box pair and its whole run reused no record. A window with nothing to collide says nothing about
reuse; the whole-run clause keeps the void for a reuse flag that never reaches the narrowphase,
which collides pairs on its first steps and reuses none. The runner's VOID message names the clause
that fired. Shown able to fail by a scratch mutation that hands the narrowphase reuse OFF whatever
the configuration says (the step's `ReuseStep::new` in `narrowphase_step`,
`crates/boyko_physics/src/systems.rs`; restored by content and md5-checked): the rest pile in `Sets` exits 3 through the whole-run clause (its window collided
no pair; its pose is the reuse-off hash `0xe99ed04348f5d898`), and in `Off` through the window
clause. The probe reads counters after the run, so it moves no pose. On the fixed runner
`pose_gates_v2.sh` reads 264/264, both negatives exit 4, 26/26 and 17/17, `pose_gates_d0.sh`
264/264, both negatives exit 4, 26/26 and 17/17, and `runner_pins.sh` 12/12, with no row of the
three reporting a VOID.
