# Physics tooling lane 1 (PHYS-TOOL-1): the record

Orchestrator rulings 64 (4), 66, 57 (2) and 71 (4), the last AMENDED by ruling 74 (A); ruling 77 closed
item (4). Value-neutral: no simulation value moved, no pin moved. Nothing was timed: every result below is
an exit code, a count, a hash or a byte comparison.

Branch `u/phys-tooling-1` from `integ/unified` `0bd689596565769e1d31a4c80e46591fe352938a`. Cut:
`D:/tmp/phys-orch/phys-tool-1/cut.md` (its addenda carry rulings 74 and 77 verbatim); critique: `critique.md`;
implementation report: `impl.md`, all in that directory. Raw logs stay under
`D:/tmp/phys-orch/phys-tool-1/` (`runs/`, `k/`, `red/`, `u4/`); this file carries the receipts.

| commit | item |
|---|---|
| `627fec96` | (2) `sleep_gates.py` names an absent output in a FAIL row; (3) GATE-KIT.md's `sleep_gates.py` tally |
| `71602537` | (1) the runner's counter oracle models L10's T4 release |
| `0fe76d6c` | (5) PC-SL2B-1: the runner header names patch v3 / dyn3 and the sleeping-on rows |
| `dff92ce4` | the record; `scenes/01-DESIGN.md` section 10.9's PC-SL2B-1 marker |
| `138db54f` | (4) closed by ruling 77: the UG-15 scope statement in `crates/boyko_symcensus/ug15/RECIPES.md`; the markers in `IB-budget/01-P0R-RECORD.md` (section 5, PC-P0R-8) and in the unified plan (03 section 6, 02 Phase F rung F2) |
| this commit | review r1 W1 (a) and W2: V11 (c2) and RF-5a byte-check the parity runner only (RECIPES.md "Codegen units", `01-P0R-RECORD.md` section 5 and PC-P0R-8, section 4 and PC-PT1-11 here; the probe is PC-PT1-13); F1's legs-recipe sentence names the three subjects re-run under it |

Item (4) stopped on two findings (section 4). Ruling 77 closed it with the scope statement; the re-pin on
the shipped codegen is deferred (PC-PT1-10).

## 1. Item (1): the runner's counter oracle with sleeping on (ruling 64 (4))

**The defect was the oracle's, not the engine's.** `BroadphaseTree::run` emits `phys_bp_queried` and
`phys_bp_members` at the end of its query and assembly, where every row is queried or a member
(`q + m = N`; `crates/boyko_physics/src/broadphase_tree/mod.rs`, the three `counter!` calls). After them,
inside the same broadphase system, L10's epilogue restores islands and T4 releases their rows from the
sleeper set (`systems.rs`, `broadphase_sets`: `tree.release`, then `sets.note_tree(.., released)`;
`members[Z] -= released`). The runner read `BroadphaseTree::diag().members` after the step, so it expected
`N - (m - released)`. A strike that wakes part of a held pile (or all of it) therefore voided the row: the
kick released 1 row, a projectile the whole 1,240-box pile (SCREEN-AM's push, likewise). The brief's
"the engine queries the awake rows only" is the same arithmetic seen from the other side.

**The model** (`jolt_parity_pyramid.rs`): `Shape::bp_released` = `SleepSkipStats::released_rows`, read with
the stats `step_shape` already read; the `const fn expected_bp_counters(rows, members_after, released, sets)`
gives `(q, m) = (N - (members_after + released), members_after + released)` on `Sets` steps and the old
formula elsewhere, `None` on an underflow, which voids a tree-path step (never a wrap). Off the tree path
both rows expect 0 samples, so their value is not read. The two BP rows read the helper; no other row moves.
A stdout-only witness, accumulated in the untimed per-step `SleepSets` block on `Sets` rows:

    tree release (L10 T4): <n> steps released <r> sleeper rows after the broadphase's counters, first at step <s> (<r0> rows)

(`<s>` counts from 0, as the VOID line does; `none (0 rows)` without a release.) No SUMMARY key and no CSV
column moved: `existing_unchanged.py` reads 11 of 11 rows SAME against the parent.

**OQ2 (audit, ruling 66; ruling 74 (C) Q11) is settled on the trunk's own V2 build, with no AM code:**
the void on `--dyn kick` and `--dyn shoot` with sleeping on and `Sets` is AM-independent. The trigger is a
strike that wakes held rows; push, AM's program, is the same class. Ruling 74 (B)'s first branch (no void on
the parent) did not arise, so the run-level red-first stands.

### 1.1 The receipts, one per strike program (ruling 74 (D)), sleeping on, V2 defaults

`R` = `--scene jolt --dyn <p> --sleeping on --workers W --steps 800 --window 200..800 --arm-profiler --csv
run.csv --pose-out final.pose` (`none`: no `--dyn`, `--steps 500 --window 100..500`; `slide`: `--steps 500
--window 0..500`), one process per fresh directory, `scripts/rprog.sh`. The runner copies:

| copy | sha256 | built from |
|---|---|---|
| parent | `384f9f4c106f9ab9105a07383a7676f838a1faaa16f4be6d4f3ed27fad6d8b05` | `0bd68959` (= sl-2b's `ours-t1` copy) |
| C2 | `7e17e8687a6ffafd154c02893cf99f72e6bc3651faff81d057d1cbccfc98e1af` | `71602537`'s tree |
| head | `cd33e087c81da5eda4a9de142763b606bab7dae00a8c6b063c85aa8425ac7488` | `0fe76d6c`'s tree (C3 is doc-only; the link differs) |

| program | runner, W | exit | void steps | the line | pose_hash | `final.pose` sha256 |
|---|---|---|---|---|---|---|
| kick | parent, 1 | 3 | 2 (454, 543) | `VOID: step 454: counter phys_bp_queried recorded (samples, value) = (1, 1239), the step has (1, 1240)` | `0xca3161f209b56e41` | `8a23006d257bcba1…` |
| kick | C2 and head, 1; C2, 8 | 0 | 0 | `tree release (L10 T4): 2 steps released 2 sleeper rows …, first at step 454 (1 rows)` | `0xca3161f209b56e41` | `8a23006d257bcba1…` |
| shoot | parent, 1 | 3 | 3 (258, 633, 771) | `VOID: step 258: counter phys_bp_queried recorded (samples, value) = (1, 6), the step has (1, 1246)` | `0x0a3bbe0a97f62147` | `1ecec8ef7030c599…` |
| shoot | C2 and head, 1; C2, 8 | 0 | 0 | `tree release (L10 T4): 3 steps released 1242 sleeper rows …, first at step 258 (1240 rows)` | `0x0a3bbe0a97f62147` | `1ecec8ef7030c599…` |
| slide (control) | parent, 1; C2, 1 | 0 | 0 | C2: `0 steps released 0 …, first at step none (0 rows)` | `0x39656286b0c2c3cc` | `1fefcd73f1d86b1f…` |
| none (control) | parent, 1; C2, 1 | 0 | 0 | C2: `0 steps released 0 …, first at step none (0 rows)` | `0xc637a0517adabab7` | `b8626bd97ccfac37…` |

Every pose equals its `pins.json` `sleeping_on.ours` fixture (`--expect-pose` passes; the sha256 column is
the fixture's), so the void never moved a value: the parent's voided run was the pinned one. The model
holds step for step: the parent's void steps are exactly the head's release steps (2 for kick, 3 for
shoot), its first void step is the first release, and its deficit is that step's released rows (1; 1246 -
6 = 1240). W8 equals W1 byte for byte.

**Mutations** (scratch, restored by content + `touch`, md5 equal; `scratch_ledger.tsv`): M1a, the members row
back on `members_after`: kick exit 3, `VOID: step 454: counter phys_bp_members recorded (1, 2), the step has
(1, 1)`. M1b, the engine's counter `queried + [Z non-empty]`: kick and none exit 3 at step 150, the first
step with a non-empty Z, before any strike, so the identity is still checked independently of the release
term. M1c, the release term dropped in the helper: 4 of the runner's 7 tests fail.

## 2. Items (2) and (3): `sleep_gates.py`'s absent outputs; GATE-KIT.md's tally

`--pins-from` on an OUTDIR without a leg's runs raised in `pose_pin.check_pose` (`os.path.isfile(None)`)
and printed no tally (sl-2b `test_r1.md` O1). Now `check_pose` takes a None pose as a FAIL item, and each
pin row names the W runs it reads that left no `out.txt` / `rc.txt` (`absent output: <dirs> (no
out.txt/rc.txt)`), for every engine; the exit contract is unchanged (exit 1, a complete tally). The
selftest is 79 -> 82 cases.

| run | parent `0bd68959` | `627fec96` |
|---|---|---|
| `--legs ours --pins-from D:/tmp/phys-orch/sl-2b/k/sleep_jolt/out` | exit 1, a `TypeError` traceback from `pose_pin.py:46`, no tally | exit 1, 4 FAIL rows naming `…/ours/<p>/W1, …/W2, …/W4, …/W8, …/W16`, `checks 4, pass 0, receipts 0, fail 4 (expected 4)` |
| the same, `--legs jolt,ours` | not run (the same call path) | exit 1, `checks 8, pass 4, receipts 0, fail 4 (expected 8)` |
| `--pins-from D:/tmp/phys-orch/sl-2b/t1/out_t1` (all legs) | | exit 0, `checks 40, pass 40, receipts 0, fail 0 (expected 40)` |
| `--selftest` | `selftest 79 cases, 79 pass` (sl-2b `test_r1.md` T8) | `selftest 82 cases, 82 pass`; M2a (no None branch) 79 pass, M2b (no absent naming) 80 pass |

GATE-KIT.md's `sleep_gates.py` row quotes the measured line `checks 415, pass 345, receipts 70, fail 0
(expected 415)`, which this lane's head runner also reads (section 5).

## 3. Item (5): PC-SL2B-1 closed

The runner's module doc says sleeping on is window 9c's "on" arm, a parity row with every engine's own
sleeping (`scenes/01-DESIGN.md` section 10), and "Build (Jolt)" carries the v3 hunks J-1..J-5, the dyn3
worktree and build (`e8f48439…`, `exes.jolt56-dyn`) for every 9c Jolt row, and v2 (`9df09901…`,
`exes.jolt56-dyn-v2`) for LAND and SR.

## 4. Item (4): UG-15 on the shipped codegen, narrowed by ruling 74 (A), closed by ruling 77

**What ran.** A scratch driver over `boyko-symcensus`'s library, never an edit of the crate (K unchanged):
`D:/tmp/phys-orch/phys-tool-1/driver/` (`src/main.rs` sha256 `56adce0b…`, the exe `273819aa…`; its lock
pruned from the workspace's, `syn 2.0.119`, `quote 1.0.47`, `proc-macro2 1.0.107` as the workspace's).
Per leg it builds the subject once under `release` through IB's existing field `Request::codegen_units` =
`retain::profile_codegen_units(Cargo.toml, "release")` = 16, not set by the profile (`objbuild::build`
passes `-C codegen-units=16` and its `check_final_codegen_units` held on every final line), then runs
leg (2)'s own `pins::check_with` on that object. Mode `leg` builds as the legs do today (unset, so
`--emit=obj` codegens one unit). Receipts: `D:/tmp/phys-orch/phys-tool-1/u4/` (`leg2-<subject>-<mode>.txt`,
`builds.txt`, `img/`). Heavy lock about 27 minutes in all.

| leg (2) subject | its claim (03 §6 leg (2)) | at cgu = 16, explicit | the legs' recipe (cgu unset) |
|---|---|---|---|
| `clear` | the P29 set, R-1 `register_new::<Transform>`, R-2 `Transform::component_id`, the dispensers D-2..D-4, H-3 `Schedule::run` (10 pins) | **holds** (0 moved, 0 absent, 0 contradictions) | not re-run |
| `boyko_demo` | the P29 set and the dispensers D-2..D-4 (7 pins) | **holds** | holds |
| `phase9_scheduler` | H-3, the `Schedule::run` dispatch (14 pins) | **holds** | not re-run |
| `swap_remove` | H-2, the hot body behind `swap_remove/10k` (5 pins) | **differs**: RED PinSet | holds |
| `query_dsl` | H-1, the hot body behind `query_ref_iter/10k`; R-4 `register_layout` (6 pins) | **differs**: RED PinSet | holds |

All 42 frozen pins were read. Every build ran `--locked` (cargo may not rewrite `Cargo.lock`) on a clean
tree (`0 uncommitted path(s)` in every receipt header).

**Finding F1: two leg (2) subjects' pins differ at the explicit 16.** On `swap_remove` (candidate H-2)
`EcsMaster::delete_entity` is identical, but the criterion harness bodies around it move (`iter_batched`,
`Routine::warm_up`), two pinned names are absent (`Routine::bench`, `bench_swap_remove::{closure#0}::{closure#0}`)
and a new name is located but not frozen. On `query_dsl` (H-1) `register_layout::<Tag>` (R-4) is identical,
but `EcsMaster::run_system::<…>` is ABSENT: it is inlined into `Bencher::iter`, which grows from 119 B to
2,296 B, and two names are newly located (`FunctionSystem`'s drop glue, a criterion `Vec<f64>` collect). Both subjects hold on this tree under the legs'
recipe, so the move is the codegen-units effect, not drift since the capture. A second `swap_remove` build
at 16 gave the same object byte for byte (`9f45de93…`) and the same verdict lines: the 16 build is
reproducible (probe (iv) arm A's premise).

**Finding F2: the explicit `-C codegen-units=16` is not the shipped codegen on `boyko_demo` and `clear`.**
`plain::check_c2` (`.text` raw bytes) against a plain `cargo build --release --locked`:

| subject | the leg object at 16 | the legs' recipe | the plain release (twice: reproducible) |
|---|---|---|---|
| `boyko_demo` | `.text` `eeea7f9e…`, 0x827942 B | `394a44ee…` | `5f16bda6…`, 0x8291c2 B |
| `clear` | `3644b1b9…` (image 5,584,896 B) | | `2d7b7c1b…` (image 5,593,088 B) |

On `boyko_demo` the final rustc lines differ only in the leg's `--emit=obj --print link-args -C
codegen-units=16`, and two further builds isolate it: `cargo rustc --release -- -C codegen-units=16` (no `--emit=obj`) gives the leg's
`.text` `eeea7f9e…`, and so does `-- --emit=obj -C codegen-units=16`. So the EXPLICIT flag, not
`--emit=obj`, separates the leg object from the shipped default, although both say 16. A plausible cause
(not verified here): rustc merges small codegen units only when the count is its default, not when the user
states it. IB P0r's RF-5a found the explicit 16 equal to the plain release on the parity runner, and V11 (c2)
re-checks the runner by bytes on every `unit` run. Neither ever built a plain probe (`unit.rs`'s
`plain_build` and `check_c2` take the runner only; RF-5a's re-run receipt, `ib-p0r/k/rf5r_receipt.txt`,
retains one plain image, `plain-runner`),
so IB's other subject, the probe, has no such proof: its `.text` at the explicit 16 has never been compared
with a plain release probe (PC-PT1-13). Ruling 57 (1)'s field, generalised to the UG-15 legs, would census
a codegen that is not the shipped one on `boyko_demo` and `clear` (here it would also turn `swap_remove`
and `query_dsl` red).

**Ruling 77 (the orchestrator's answer to the stop).**

1. Item (4) closes with the scope statement in `crates/boyko_symcensus/ug15/RECIPES.md`, "The UG-15 legs'
   scope". The legs census codegen-units = 1 objects, and their pins are identity claims between
   identically built trees at that codegen. F1 and F2 are recorded there with their files. The
   shipped-codegen question needs an instrument that builds the EXACT shipped recipe (no explicit
   codegen-units flag) and compares two arms built that way. That is deferred to the unified plan's F2
   end-state gates as PC-PT1-10, with both findings attached.
2. F2 bears on ruling 57 (1)'s premise for IB, that an explicit effective value reproduces the shipped
   bytes. IB's V11 (c2) byte gate stays the per-subject proof, and the `ib` lane re-runs it at its base
   (PC-PT1-11). `RECIPES.md`'s "Codegen units" paragraph now says the RF-5a calibration is per subject.
3. The markers (cut Q4, markers only), each a dated note (in the plan, inline, so no line shifts):
   * `IB-budget/01-P0R-RECORD.md`: section 5 and PC-P0R-8;
   * `UNIFIED-SYSTEM-PLAN-03-GATES.md` section 6: `:183` (probe (iv), the profile), `:188` (leg (7)'s
     `.text` check) and `:205` (PC-18's `.text` numbers);
   * `UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md`, Phase F rung F2: PC-PT1-10 joins the end-state gates.

   The counts at 03 `:24` and `:36`, 02 `:73` and `:82`, and 00 `:129` stay unmarked: nothing was
   re-frozen, so the 42 frozen pins and the 6 recorded-absent pairs are unchanged.

No file under `crates/boyko_symcensus/src/` or its `Cargo.toml` changed, so K is unchanged
(`src/key.rs`: K covers `Cargo.toml` and `src/**`; `RECIPES.md` is outside it).

## 5. The gates (each commit's own in its message)

On `0fe76d6c`'s tree with the head runner (kit summary `k/summary_b2.tsv`: 8 gates DONE, exit 0): runner unit
tests 7 of 7; `dyn_scenes_hook_order` 2 of 2; `existing_unchanged.py` head vs parent `rows 11, same 11,
diff 0`; P4 `runner_pins.sh` `rows 12, pass 12, fail 0`; P5 `pose_gates_v2.sh` and `pose_gates_d0.sh` each
`runs 264, pass 264, fail 0`, both negatives `exit=4 (want 4)`, `checks 26, pass 26, fail 0`, `checks 17, pass
17, fail 0`, `ALL_DONE`; P11 `ours_gates.sh` `checks 49, pass 49, fail 0` and `s5_runner_gate.sh` `rows 54, pass
54, fail 0`; `sleep_gates.py` (dyn3 Jolt, the Rapier arms, this runner) `checks 415, pass 345, receipts 70,
fail 0 (expected 415)`; `cargo check` and `clippy --workspace --all-targets --locked -D warnings` exit 0.
No file of `crates/boyko_physics`, `boyko_ecs`, `boyko_threadpool` or `boyko_memory` changed, and no
`unsafe` or atomic was added.

## 6. PC lines

* **PC-PT1-1 (the AM lane).** Its re-gate (ruling 66) runs a runner built from a trunk with this oracle;
  `push` gets the model by a trunk sync. Addendum-01's 13 excluded runs stay excluded in SCREEN-AM's record.
* **PC-PT1-6 (same class, outside UG-15).** `crates/profile_fixture/tests/profile_axis_census.rs` builds
  `-- --emit=obj` with codegen-units unset (a codegen-units = 1 object); F2 says an explicit 16 would not
  fix that either. For the diagnostics owner.
* **PC-PT1-7 (kit README, orchestrator tooling).** The `jolt` row names `build-v5.6.0-dyn-dist` (9c's is
  dyn3); no `sleep_gates` row (its line is section 5's); the cargo CmdFile example says JOBS=4 (ruling 69:
  2).
* **PC-PT1-8 (trunk pin header).** P9's ledger last row on `0bd68959` is `PX0-8` (`5 7`), not `SR-B-C6`;
  the values are equal.
* **PC-PT1-10 (UG-15 on the shipped codegen; the unified plan's F2, ruling 77 (1)).** First an instrument that
  builds each subject by the EXACT shipped recipe (no explicit `-C codegen-units`) and compares two arms
  built that way; then the legs' re-pin, each moved pin re-derived under its own rule and declared.
  Attached: F1 (section 4, the leg (2) pins at the explicit 16 differ on `swap_remove` and `query_dsl`)
  and F2 (section 4, the explicit 16 is not the plain release `.text` on `boyko_demo` and `clear`).
  Raw receipts: `D:/tmp/phys-orch/phys-tool-1/u4/` and `driver/`.
* **PC-PT1-11 (the `ib` lane, IB P1-P4, the physics roadmap's P6; ruling 77 (2)).** At its base, before
  it relies on IB's codegen-units field, re-run V11 (c2) (`ug15 unit`, RECIPES.md "IB V11 commands"),
  and STOP if (c2) is red. (c2) byte-checks the parity runner only; the probe is PC-PT1-13.
* **PC-PT1-12 (`objbuild.rs`'s module docs).** They still read "recorded as an owed decision". This
  lane does not edit the instrument (K would move). The next change that moves K anyway (IB P2,
  PC-P0R-4) can update them to cite RECIPES.md's scope statement.
* **PC-PT1-13 (the orchestrator, open; review r1 W1 (b)).** IB's probe (`physics_probe`) is a census subject
  (IB V9 G-SYM (ii)) built at the explicit 16, and no receipt compares its `.text` with a plain release probe:
  V11 (c2) and RF-5a checked the parity runner only, and `ug15 unit` cannot build a plain probe today. F2
  shows the explicit 16 can differ from the shipped codegen on a final crate with a short source (`clear`,
  a 15-line example; the probe's bin is 216 lines); whether the probe is in that class is unmeasured. A ruling decides whether PC-PT1-11 gains a probe-side `.text`
  comparison (an instrument change, so K moves) or IB's probe census is scoped like the UG-15 legs.
  Files: `D:/tmp/phys-orch/phys-tool-1/review_r1.md` (W1), `triage_r1.md` (W1).

## 7. Quiet-window commands

None owed: the lane makes no timing claim. Every runner change is after the step's `Instant` pair
(`dyn_scenes_hook_order` 2 of 2), no engine source changed, and the UG-15 work is a scratch driver.
