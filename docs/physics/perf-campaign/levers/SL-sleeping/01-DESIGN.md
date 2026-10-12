# Lever SL (sleeping): the as-built design

Lane `u/phys-sl`, cut from `264e5b2d` (`integ/unified`), built 2026-10-11 and 2026-10-12. This file is
the design record of what the lane built.

- **Part A** is the as-built delta. It lists every place where the cut or the build differs from design
  rev 2, and the rulings that decided them. The cut is the session scratch `D:/tmp/phys-orch/sl/cut.md`
  (sha256 `17bedade3d132a849d46e0ea4732db3f21169ea75216286009c0307ee404768d`), with its three addenda.
- **Part B** is design rev 2 verbatim, as `critique_final.md` approved it.
- **Where Part A and Part B disagree, Part A is what the code does.**

The lane's other records, in this directory:

| file | what |
|---|---|
| `02-SCREENS.md` | S-SL1 / S-SL2 / S-SL3 (untimed): receipts, paths, the pre-registrations quoted in full |
| `03-PINS.md` | every pin moved by A7-R1's rule (old / new / command / masked equality), every pin kept, with readings |
| `04-LANE-RECORD.md` | commits, gates and red-firsts per commit, the review and test rounds, rulings 39 and 40, the PC lines, the quiet-window commands for SL-AB |

Part A names code by symbol, not by line; Part B's `file:line` are those of `f277269d`, as its own
header says (the cut's §1 re-located them on `264e5b2d`).

## A.1 The rungs, as cut and as built

| rung | commit | as cut (`cut.md` §3) | as built |
|---|---|---|---|
| SL-2a | C1 `f1f733f6` | the runner accepts `--dyn` with sleeping on; a `sleep` SUMMARY object on colored sleeping-on rows; an explicit `--sleeping` on every pinned runner row of the gate scripts; `--perturb B:A:D` (Q3) | as cut, except three items. **No `--perturb`** (ruling 39 Q3 = no: the screens and T-SL10p carry the perturbation themselves). **`ours_gates.sh` and `existing_unchanged.py` stay flagless** until SL-3 (Q5 fallback, PC-SL-16). **Two refusal rows of `l10/runner_checks_c3c.sh` stay flagless**, because their subject is the unset flag (PC-SL-16). No engine source changed: `git diff --stat f1f733f6^ f1f733f6 -- crates/boyko_physics/src` is empty |
| SL-1 | C2 `57d6fe62` | the rest state, the settle (the zero at sleep), the seal, the fold on `rest`, the counters, docs, re-expressions, re-pins | as cut, with C-1..C-11 (A.2), the critique's resolutions (A.3) and the deviations in A.4 |
| C2b | `0ae0ef3e` | not in the cut (ruling 40) | LB5 (`tests/sleep_skip_bit_identity.rs` `s3_mid_step_dt_write_is_invisible`) re-expressed, so its anti-vacuity clause (ii) is reachable under SL (A.5) |
| SL-1b | C3 `3bbd9334` | the write-wake, the C-2 capture assert, the T1/H-03 re-expression (C-10) | as cut (A.4) |
| fix rounds | `53ee9865` .. `493be29a` | — | tests and doc comments only. No non-doc line of engine source changed (`review_close.md`: the non-`///` +/- lines of `resources.rs` and `sleep_sets.rs` are 0). Trunk sync `a56810a0` (boyko_ui tests only) |

## A.2 The cut's corrections to rev 2 (cut §3.0; binding on the build)

- **C-1: an `end_step` unlatch leaves `rest`.**
  - Rev 2's §3.2 had "end_step | island frozen this step | rest unchanged". That is wrong.
  - Without the fix, a frozen island whose energy unlatched it would stay frozen through the fold on
    `rest`. The energy comes from a user write, or from a direct-drive caller.
  - Built rule: after the latch update in `end_step`, `if solved || !l.asleep { l.rest = REST_NONE }`.
    So `rest != NONE ⇒ asleep` holds on every unlatch path.
  - Guard: T-SL18 (mutation M-SL14).
- **C-2: the capture assert.**
  - The assert is `debug_assert!(seal.is_none() || !b.simulated || (v == 0 && ω == 0))`, in the freeze
    capture of `solve_colored_inner`.
  - It is guarded by `seal.is_none()`: direct drive has no write test, so a direct-drive caller may
    legally write a frozen row.
  - Built at C3; its message names the row.
  - Release codegen is unchanged: `solve_colored_inner` has 6171 instructions at C2 and at C3.
- **C-3: M-SL12's killer.**
  - Rev 2's lateral speculative pair at d + 0.01 mm is withdrawn: it needs a sub-micron gap tuned against
    a velocity no test controls.
  - Built: the pure helper `attempt_event(prev, now)` in `tests/sleep_settles_box_piles.rs`. It is the only
    reader of `contact_wakes` and `settle_rekeys` in `settle_until_frozen`, and its unit is
    `the_attempt_event_counts_a_settle_rekey`.
  - Limitation: a mutation that bypasses the helper is a review item, not a gate.
- **C-4.** `settle`'s `debug_assert!(touched.get(row))` is its contract and is not weakened. Direct-drive
  units pass a `TouchedMask` with the settling rows set.
- **C-5: one list for the seal's config.**
  - `pub(crate) struct SealEpoch(SleepEpoch)` is built by `SleepEpoch::of(SleepSkip::Off, false, cfg, field)`.
  - The mode and warm fields are pinned constants, so the seal is mode-free.
  - Every config field `SleepEpoch::of` reads is in the seal by construction. A later knob added there
    joins both epochs.
  - The epoch is built only in `settle`'s cold body and once per attempt in `seal_holds`, never per step.
  - Guard: T-SL20.
- **C-6: an explicit disarmed value.**
  - The seal's input is `Option<SealArm<'_>>`, with `SealArm { cfg, field }`. `None` is disarmed.
  - `solve_colored` and the public `solve_colored_sleeping` pass `None`. Only `solve_colored_held` (the
    pipeline, `physics_solve_colored`) passes `Some`.
  - `IslandSleep` stores `seal_epoch: Option<SealEpoch>`, so no `UNSET == UNSET` comparison exists.
  - `settle(.., None)` zeroes, stashes nothing and never sets `seal_pending`.
  - A disarmed begin runs no write test and never seals. At C3 this was made structural:
    `begin_step_armed` replaces `bodies` with `&[]` when `arm` is `None`.
  - The field is `inputs.sdf_field()`, unconditionally. That is a superset of what the narrowphase
    reads; an edit of a field nothing reads can only unseal, which is the trunk rule.
  - Guards: T-SL19a, T-SL19b.
- **C-7: the perturbation.**
  - The trunk runner has no one-ulp hook. It exists only in the CS prototype patch (`cs_proto.diff:632-664`,
    env `BOYKO_F0_PERTURB`).
  - T-SL10p builds its own J-T lattice in the test file.
  - The hook steps the f32 BITS by ±1. For a negative component, `+1` increases the magnitude.
  - Fallback seed order: b581zm, b658xp, b813xm, b968xp, b1046zp.
- **C-8 and C-9** became ruling 39 Q4 (PIN_S_B re-pinned) and Q1 + Q2 (the fixture mirror E3). See
  `03-PINS.md`.
- **C-10: T1/H-03.** A new `Y_SPAWN_AT_REST = Vec3::ZERO` is used by T1 and the colored H-03. The serial
  sleeping-off H-03 keeps `Y_SPAWN_VELOCITY`, so no sleeping-off scene moves.
- **C-11: no new profiling zone.** `settle` runs inside the existing `zone!(PHYS_SLEEP_END)` block and
  `seal_holds` inside `PHYS_SLEEP_BEGIN`. The runner's armed zone census and `profiling_zone_counts` are
  unchanged.
- **O4: the seal accounting.**
  - The counters are `settles` (armed settles), `seal_held` and `seal_breaks`.
  - `rekey_rows`:
    1. a `seal_live` that no begin consumed (an early-return step) counts a break;
    2. then `take(seal_pending)`: Identity sets `seal_live = true`; anything else counts a break.
  - The armed begin: `take(seal_live)`, then `seal_holds`, then `seal_held += 1` or `seal_breaks += 1`.
  - `arm_seal_for_test()` is `seal_live = take(seal_pending)`.
  - The debug invariant `seal_held + seal_breaks + pending + live == settles` is asserted at the end of
    `rekey_rows`, of the begin body and of an armed `settle`. An armed `settle` that finds an unclassified
    pending seal counts it a break first.
  - `seal_breaks` counts every unsealed evaluation, with or without a key change.
- **W2.** A7-R1's three pins must not move: A7-R1 forces sleeping off. The sleeping-on drift bound for
  SL-V belongs to the sl-v lane (PC-SL-13).
- **W4a: T-SL11s, the spawn killer for M-SL9a.**
  - On the first step after a settle, a static box is spawned overlapping a pile box, appended after
    every watched row.
  - Anti-vacuity: the row count rose by one, and the last row's position bits are the spawn position.
  - Asserts: `contact_wakes` rose, `settle_rekeys` did not, `seal_breaks` rose, and the pile is awake.
- **W4b: T-SL7 produces C2** (a row latched while its island still has an unlatched member).
  - Column A rests from step 0. Box B is spawned at step 20, at rest, touching A laterally.
  - Over the window, A is latched (`is_row_latched`) and awake, `write_wakes == 0`, `rows_put_to_sleep`
    is flat, and A's velocities are not bit-zero.
  - An empty window is a failing assert.
- **W4c: T-SL10.** The bodies carry `v = (1e-3, 0, 0)` and `ω = (0, 5e-4, 0)` before `settle`, so M-SL11
  (the stash before the zero) breaks every seal in debug.
- **O2: (ii-dyn).**
  - (ii-dyn) is Σ solved slots, L ≤ P.
  - `frozen_v_max` voids a J-T row and is a STOP on any `--dyn` row. With SL-1b it must read 0 everywhere.
  - Rev 2's "Q1 = no" branch (SL-1 without SL-1b) is not built (ruling 27).
- **Route 2's exit (the SUB-ISLAND case)** joins §10.1's failure routes. It is quoted in `02-SCREENS.md`.
  Expected count 0; the screens read 0.

## A.3 The critique's remarks, resolved (cut Addendum 1)

- **W1: (ii-dyn) blocks only on each S-SL3 row's structural prefix `[0, s*)`.**
  - `s*` is the first step on which, in either arm, a row woke whose island had settled (or latched) at
    an earlier step.
  - Before `s*` the arms solve the same islands on every step by construction, so the bar there is a
    structural property, not a comparison of two chaotic runs.
  - From `s*` on, (ii-dyn) is reported, not blocking: rev 2 §10.3's "blocking for SL-3, reported for SL-1".
  - A prefix red is a STOP, with both arms' per-step solved-slot columns.
- **W2: `begin_step_upto(graph, n, upto)` keeps its three-argument signature and is the DISARMED entry**
  (`bodies = &[]`, `arm = None`).
  - The armed body is `begin_step_armed(graph, n, upto, bodies, arm)`, called only by `solve_colored_inner`'s
    pipeline arm.
  - `begin_step` stays `begin_step_upto(graph, n_rows, self.wake_requests)`.
  - So a test that reaches the sleep state by accident reaches the disarmed path.
- **W3: C2's prerequisites** (Q1, Q2, Q4, Q6) were rulings, not mechanics. Ruling 39 answered them (A.5).
- **The critique's open questions:**
  - T-SL7's empty window is a failing assert.
  - A live seal reaching a disarmed begin is consumed and counted a break.
  - Every `*_rises` / `*_steps` list in the runner's `sleep` object is `step + 1`, the `first_frozen_step`
    convention. So a kick applied before run `s` write-wakes at `s + 1`.
  - E3's `logs/record.tsv` carries the unchanged sleeping-off rows too.

## A.4 Deviations at build, each with its reason (cut Addenda 2 and 3)

**SL-1 (C2):**
- **`settle(bodies, touched, arm)` has no `graph` parameter.**
  - Membership is the latch's `rest == REST_SETTLED`. `end_step`'s cold `mark_settled` pass sets it for
    every member of a `sleeps` island.
  - So the zero, the stash and `rows_put_to_sleep` are one set by construction.
- **T-SL10p's seed is `b581zm`** (C-7's first fallback), not `b38xp`.
  - `b38xp` rises on the parent exactly as the CS sweep recorded (146 / 1240 / 206).
  - Under SL its first attempt changes no count: the zero removes that flicker on its own. So M-SL9 could
    never red the test on it.
  - `b581zm` reds on the parent identically. On C2 it freezes at 146, with 1240 re-keys, 1 seal held and
    0 breaks.
- **T-SL17 uses two islands.** With one island, the second begin meets `REST_FROZEN` rows, which the trunk
  rule governs whatever `seal_live` says.
- **T-SL7's window ends at the step before the merged island's settle.** It is asserted contiguous from A's
  latch to that settle, so M-SL4 also breaks the window's extent.
- **The topology-churn oracle counts its C2 states:** `latched_not_resting > 0` after the proptest.
- **E3 mirrors the files the gates read**, not every file of the record directories. Window 6's `J500`,
  `JAon500`, `R1100`, `Ron1100`, `S16-300` and `fixtures.json` are not mirrored: no gate reads them.
- **The `use` block of `resources.rs` changed** (imports the locked code needs). It is a textual neighbour
  of fm's lock.

**SL-1b (C3):**
- **The write test sits in `begin_step_armed`'s island branch,** after the no-island `continue` and before
  the count comparison. A no-island or past-graph row already takes `rest = REST_NONE` and is awake, so a
  write on it is not counted.
- **`velocity_written(b)`** is a private `#[inline]` free function: six `!= 0.0` tests. `-0.0` reads as
  zero; NaN reads as a write. Since fix round 2, T-SL30 gates both signs of every component, NaN and
  `-0.0`.
- **T-SL3 to T-SL5 assert the O1 split exactly:** the `(write, contact)` wake rises are `(1, 0)`.
- **T-SL5 needs no probe run.** One run detects the settle step `F` (`rows_put_to_sleep` rose) and writes
  before `F + 1`.
- **T-SL6 and T-SL6b share one soft scene:** a box on the rigid floor, the SDF floor at `y = -5`, and the
  S5 soft cube spawned 0.3 m above the box once it froze.
- **M-SL8 is concrete:** the write test moved into `end_step`, which reads the post-solve snapshot.

## A.5 The rulings that shaped the build

Their full text is in `04-LANE-RECORD.md` §5.

- **Ruling 39** (2026-10-11; C2's prerequisites):
  - Q4: PIN_S_B re-pins by A7-R1's rule, and its pin block names the exception.
  - Q1 + Q2: the new mirror `docs/measurements/2026-10-11-sl-sleeping/`; no record directory is edited.
  - Q3: no runner `--perturb`.
  - Q5: the flagless fallback stands.
  - Q6: `solve_colored_held`'s parameter, the `solve_colored_sleeping` doc, the plugin and scratch_ids doc
    lines, and UG-10 anchor-only fixes are allowed.
  - Q7: merge on the untimed gates and screens; SL-AB rides the next quiet window.
- **Ruling 40** (2026-10-11; the LB5 STOP), option (a):
  - Under SL a frozen dynamic row's velocity is exactly zero. L9's `is_fast` reads only the two bodies'
    velocities, so it cannot flip on a frozen pair of two dynamic rows, and LB5's clause (ii) became
    unreachable on the towers.
  - C2b gives tower 2 a kinematic neighbour, 5 mm off its middle cube, moving away at 1 mm/s. `3 dt² v²`
    is 8.3e-10 at `DT` and 3 at `DT_W`, against `(τ_eff/2)²` = 2.5e-7.
  - The engine never integrates a kinematic pose, so the box stays put; it rests and anchors tower 2's
    held island.
  - Clause (ii) reads `frozen (9, 9), pairs [19, 19], reused [12, 9]` on Tree and Grid, at W1 and W8.
  - Red-first: with the D9b defect re-introduced, LB5 reds at step 150.

## A.6 Findings against rev 2's text

- **§3.5's soft-load wording.**
  - Rev 2 says a rigid island under a resting soft load cycles ("about 60 awake steps, a settle, a
    write-wake, and again"), and "is never frozen for more than the one step the write is read on".
  - As built, the box is frozen on NO step after the first reaction: each settle is write-woken on the
    very next step. T-SL6b's window reads 587 steps, 1 frozen step (the first reaction's own), 9 settles
    and 10 write wakes.
  - The code's doc says "never frozen on two consecutive steps", which is what T-SL6b asserts.
- **§10.1's arm-D prediction** `rises(D) ≥ rises(P)`, paired by seed, is falsified on one seed.
  - D re-wakes on 5 of P's 6 flickering trajectories, and not on `b38xp`, whose flicker the zero alone
    removes. D never rises where P does not (0 of 28).
  - So the seal's measured job on J-T is the five first-attempt freezes (1240 re-keys each).
    `02-SCREENS.md` F2.
- **SL-1 alone is not safe on S-KICK.** Without SL-1b a kick writes a velocity into a frozen row: L's
  `frozen_v_max` is 21.43 on S-KICK, and L+b's is 0. The two rungs ship together, in one merge.
  `02-SCREENS.md` F3.
- **§10.3's S-KICK expectation** ("the pile then stays awake") holds for 20 of the 24 kick events, which
  hit an already-awake pile. Write wakes fire on 4 events (65 rows). `02-SCREENS.md` F4, PC-SL-22.

## A.7 Tests added after rev 2 (fix rounds 1 and 2)

| id | test | round, item |
|---|---|---|
| T-SL21 | lib `resources::tests::sl::the_seal_compares_every_user_writable_field_by_bits_and_no_derived_one`: 33 compared data, each changed alone by a one-bit flip, in both argument orders; the derived `bp_margin` and `inv_inertia` must not unseal | fix r1, F2 |
| T-SL22 .. T-SL29 | `tests/sleep_seal.rs`: a static rotated (22), resized (23), slid along z (24), given a velocity (25) or a spin (26) into a settled pile; a support turned sensor (27), parked on the SDF floor (28), made immovable (29) | fix r1, F2 |
| T-SL27c | `tests/sleep_seal.rs` `a_support_given_a_test_local_tag_on_the_first_step_after_a_settle_keeps_the_seal`: T-SL27's control, an archetype move that does not reorder the walk keeps the seal | fix r2, W1 = N2 |
| T-SL30 | lib `resources::tests::sl::the_write_test_reads_both_signs_of_every_component_and_a_nan_and_neg_zero_as_zero` | fix r2, N1 |
| T-SL3 | widened to 12 writes (each component, each sign) × 4 arms = 48 cells | fix r1 F1, fix r2 N1 |

`seal_equal`'s doc (fix r1) names what each compared field can reach:
- **Fields that decide whether a manifold exists**, each with its scene: position (T-SL12, T-SL13, T-SL24),
  rotation (T-SL22), shape (T-SL23), linear and angular velocity (T-SL25, T-SL26), `is_sensor` (T-SL27),
  `simulated` (T-SL28), `inv_mass` (T-SL29).
- **Fields compared conservatively:**
  - `inv_inertia_local`, derived from `shape` and `inv_mass`;
  - `kinematic`, which the tree broadphase reads only to pick a set whose candidates equal all-pairs;
  - `restitution` and `friction`, which only the solve reads.

## A.8 Where the code is

- `crates/boyko_physics/src/row_identity.rs`: `SleepLatch.rest` (was `_pad`), `REST_NONE`, `REST_SETTLED`,
  `REST_FROZEN`. The 8 B assert is unchanged.
- `crates/boyko_physics/src/resources.rs`:
  - `IslandScratch.sleeps`, `SealEpoch`, `SealArm`;
  - `IslandSleep`'s stash (a `ScratchColumn<(u32, BodyState)>` on `colored_frozen_rows_id()`), `seal_epoch`,
    `seal_cursor`, `settle_pending`, `seal_pending`, `seal_live`;
  - the counters `write_wakes`, `rows_put_to_sleep`, `settles`, `settle_rekeys`, `seal_held`, `seal_breaks`,
    their getters, and `is_row_latched`;
  - `rekey_rows`, `begin_step`, `begin_step_upto` (disarmed), `begin_step_armed`, `velocity_written`,
    `end_step`, `mark_settled`, `settle` / `settle_cold`, `take_seal` / `seal_holds`;
  - `#[cfg(test)]` hooks `rest_of` and `arm_seal_for_test`.
- `crates/boyko_physics/src/sleep_sets.rs`: `seal_equal`.
- `crates/boyko_physics/src/solver/colored.rs`: `solve_colored_inner(.., seal)`, `solve_colored_held(.., field)`,
  the `settle` call inside `PHYS_SLEEP_END`, and the C-2 capture assert.
- `crates/boyko_physics/src/systems.rs`: `physics_solve_colored` passes `inputs.sdf_field()`.
- `crates/boyko_physics/benches/jolt_parity_pyramid.rs`: `validate_dyn`, and the `sleep` SUMMARY object.
  - Its keys: `contact_wakes`, `contact_wake_rises`, `remap_resets`, `frozen_v_max`, `rows_put_to_sleep`,
    `settles`, `settle_steps`, `settle_rekeys`, `seal_held`, `seal_breaks`, `write_wakes`, `write_wake_rises`.
  - Each list holds at most 256 entries, then a `_truncated` flag.
- New tests: `tests/sleep_velocity_zero.rs`, `tests/sleep_seal.rs`, `tests/sleep_write_wake.rs`, and
  `src/resources_tests.rs` `mod sl`.
- Re-expressed tests:
  - `tests/frozen_island_warm_start.rs` (B1);
  - `tests/row_identity_remap.rs` (T1, the colored H-03);
  - `src/solver/colored_tests.rs` (`topology_churn…`, PIN_S_B);
  - `tests/sleep_settles_box_piles.rs` (G2 and G7 through `attempt_event`);
  - `tests/sleep_skip_bit_identity.rs` (LB5);
  - `tests/v2_speculative_fidelity.rs` (the two sleeping-on anchors).

---

# Part B: design rev 2, verbatim

Copied from the session file `D:/tmp/phys-orch/sl/design_r2.md`
(sha256 `ef76ef0895e241705aa993d616f45bc24d38ea217db774c439550f6803fae585`). One change: a local Cargo
registry path is written with `$CARGO_HOME` in place of the user's profile directory. Everything below
this line is rev 2's text, including its own verdict line, and Part A above supersedes it where they differ.

DESIGN READY: SL rev 2 - the settle zeroes a sleeping island's velocity on every step it goes to sleep (Jolt/Rapier semantics), and a seal re-keys, never wakes, a count change on the first step after a settle when no external input changed, so the zero's one-way margin shrink cannot wake its own island (J-T W_sleep <= parent per seed by construction; bar (ii) blocking, failure route written before the first run). Jolt 5.6 today: sleeping off, ours/Jolt = 0.874 / 0.972 / 1.043 / 1.133 / 1.363 at W1/2/4/8/16 (9b, W1 the only claim); sleeping on, nothing has been measured for Jolt 5.6 (its only sleeping reading is v5.3.0), and an untimed run S-SL0 fixes that.

# Lever SL, design rev 2: sleeping on by default, plus the stored-velocity defect

**Base.** Trunk `f277269d` (`D:/wt/joltab`, integ/unified), with the crate root `D:/wt/joltab/crates/boyko_physics/`. Every `file:line` without a directory is relative to that root. The design targets the trunk plus the SR-B cut (`D:/tmp/phys-orch/sr-b/cut.md`) and the S5 cut (`D:/tmp/phys-orch/s5/cut.md`). Neither cut moves a statement this lever reads or edits (§4.0).

**What was done.** Nothing was built, run or timed. "(inference)" marks a claim read from code but not executed. Inputs saved beside this file:
- critique r1, verbatim: `critique_r1.md`;
- rev 1: `design_r1.md`;
- research: `research_web.md`, `research_tree.md`.

Rev 2 is a complete design and replaces rev 1.

---

## Changes from rev 1 (each finding of critique r1 -> its resolution)

| finding | what rev 1 said | resolution in rev 2 | where |
|---|---|---|---|
| **C1.1** analysis | "one count draw per freeze, as today" (§2.3 C5, §3.4 A3, §17 R1) | **Corrected.** At zero velocity every V2 margin is at its minimum. So the first count after a settle can only *lose* manifolds against what the trunk would count at the same poses: a one-way shift. It lands on **every** freeze attempt. A3 (zero one step later) would move the same shift to the next comparison. | §2.3, §3.6 A3, §17 R1 |
| **C1.2** bar (ii) waived | "If (ii) fails … the fix still lands" | **Blocking for SL-1**, and tightened from the mean to **every seed**. A failure route is written before the first run, and no branch of it lands SL-1 with a failed bar. | §10.1 |
| **C1.3** design the draw, or measure | — | **Both.** **The seal** (§3.4): on the first step after a settle, a count change *re-keys* instead of waking when the step is sealed. Sealed means the rows are unchanged, every watched row's user-writable state is bit-equal to what the settle step wrote, and the narrowphase config and SDF edits are unchanged. A sealed count change is provably internal: the zero's shrink, or the settle step's own motion. So on J-T, W_sleep(SL-1) ≤ W_sleep(parent) **per seed, by construction**. S-SL1 checks this with falsifiable receipts: seal breaks = 0, first frozen step = the parent's first attempt, identical positions on runs where the parent does not rise. Arm D (settle without the seal) measures C1's size, paired by seed. Off ≡ Sets and W-identity hold (§6). | §3.4, §6, §10.1 |
| C1, G1 wording | "applied once" | "applied on every step an island goes to sleep, i.e. every attempt" | §1 |
| C1, §11 | "about 1.0× the parent's" | "≤ the parent's per seed on J-T, by construction; equal where the parent does not rise" | §11 |
| **W1.a** in-sample model | "the model 8b validates" | "validated" is withdrawn. The model is a bookkeeping identity, and 8b only shows its terms were read consistently. Everything in §0.2 and §11 is labelled cross-build, cross-window arithmetic. | §0.2, §11 |
| **W1.b** floor | 0.1254 / 0.1337 ms, pre-V2, including transition steps | Steady floor 0.1085 / 0.1177 ms, plus an explicit transition term. A V2 range is added for the manifold-dependent pieces. The ratio is about 50×, not 58×. | §0.2, §11 |
| **W1.c** A_jolt from P0, build unknown | "build not stated, primary v5.3.0" | **Resolved from the tree.** P0's `JOLT-SLP` row runs **v5.3.0** (`tools/physics_parity/driver.py:145`, binary key `'v530'`; v5.6.0 is only `JOLT56-T`, `:143`). Its "vs timed" ratio divides by v5.3.0's mean: 1.8941 / 0.1205 = 15.72 ms, which is v5.3.0's W1 (`p0b/window_report.md:192`, `ANALYSIS.md:38`). So **no Jolt 5.6 sleeping-on reading exists**. A range over A_jolt is given, and **S-SL0** (untimed `-allow_sleep` on 9b's exact Jolt 5.6 binary) fixes it. | §0.2, §10.0 |
| **W1.d** "ahead at W1-W2" in the headline | headline | Removed from the headline. Now conditional on Jolt 5.6's freeze frame (S-SL0) and on SL-V's 30-frame cell passing its bars. The "zero rises" premise is now structural on sealed steps (the seal), not assumed. | §0.2, §11 |
| O1 red-first robustness | — | T-SL3/4/5 use lateral writes on a body with no neighbour within reach. The Q1 claim is qualified: a kick toward a neighbour already wakes on the trunk through the count. S-SL3 splits `contact_wakes` from `write_wakes` per event. | §9.1, §10.3, §18 Q1 |
| O2 soft load | — | Stated. A rigid island under a resting soft load cycles: 60 awake steps, a settle, a write-wake. On the trunk, the frozen body instead accumulates `dl`. Covered by T-SL6b and recorded as PC-SL-7. | §3.5, §9.1 |
| O3 write-test cost | "one byte test" | Priced at 0.5–2 µs per frozen step (inference), a new strided pass. Recorded as PC-SL-8. | §5, §13 |
| O4 B1 twin | zero once | The twin is re-zeroed on **every** settle event | §8.2 |
| O5 proptest oracle | — | Re-expressed to the fold's predicate (`rest`), plus a churn branch that latches through `end_step` and settles | §8.2 |
| O6 doc site | — | `sleep_sets.rs:2054-2057` added | §4.3 |
| O7 order | S5 missing | SR-B → S5 → SL-1 → IB → combined screen → CS → SL-V → 9c → SL-3 (ruling 20's spine) | §16 |
| open Q1 flip bar | flip before 9c | SL-3 lands **after** 9c. It is decided by 9c's on/off pairs: no scene CLAIMED slower with sleeping on at any W, otherwise an owner question. | §7, §18 Q6 |
| open Q2 SL-V arms | 3 cells × CS × FM × CR | **One stage after the combined screen**, on the chosen contact set. Under the seal, each candidate's freeze step is its own onset + `sleep_frames` on quiet scenes, so the dwell shifts every candidate equally and cannot reorder CS (inference; checked in S-SL4). | §10.4 |
| open Q3 patched parent | lift on the parent binary | **SL-2a** (the runner refusal lifts and explicit flags, with no simulation change) lands **before** SL-1. The parent is therefore an unpatched trunk executable. A byte-identity receipt is required. | §3.7, §10.3, §16 |

---

## 0. Current numbers against Jolt 5.6 (the owner's question)

### 0.1 Measured: window 9b, J-T, sleeping off on both sides

**Run conditions.** Window 9b ran on 2026-10-05:
- ours: trunk `c335b4c6`, flagless V2;
- Jolt 5.6: `D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe`;
- scene: J-T, 1,240 boxes;
- window: [100,500), K = 9 in-block.

Source: `D:/tmp/phys-orch/win9b/analysis.md:19-46` and `:150-160`.

| W | ours ms/step | Jolt 5.6 ms/step | ours/Jolt | verdict (ruling 1) | run 195738 alone (post hoc) | ours/Jolt per velocity row-iteration |
|---|---|---|---|---|---|---|
| 1 | 8.228 | 9.419 | **0.874** | ours faster, CLAIMED | 0.876, ours STRONG | 0.444 STRONG |
| 2 | 5.473 | 5.631 | 0.972 | NOT CLAIMED | 0.962, ours CLAIMED | 0.494 STRONG |
| 4 | 3.774 | 3.617 | 1.043 | NOT CLAIMED (Jolt faster in every pass) | 1.062, Jolt CLAIMED | 0.530 STRONG |
| 8 | 2.976 | 2.626 | **1.133** | NOT CLAIMED (Jolt faster in every pass) | 1.179, Jolt STRONG | 0.576 CLAIMED |
| 16 | 3.274 | 2.402 | **1.363** | NOT CLAIMED (Jolt faster in every pass) | 1.368, Jolt STRONG | 0.693 NOT CLAIMED |

- **Why Jolt wins at W ≥ 4 although each of our rows is cheaper.** We run 1.967× Jolt's row-iterations: 1,113,116 against 565,790 per step.
- **Fidelity is not equal.**
  - Jolt holds 37 of the 44 runs of the F0g gap sweep.
  - Our trunk V2 holds the 26-run gate and 87/88 of the ensemble (ruling 23b).
- **Rapier 0.36's fastest holding configuration**, for the owner's second reference: it is 2.01 / 2.25 / 2.51 / 2.94 / 3.15× faster than ours at W1/2/4/8/16, CLAIMED STRONG ×5 (`win9b/analysis.md:27-28`).
- **What the levers in flight must still close** (9b §5 arithmetic):
  - W8 is closed by SR + S5.
  - W16 needs IB as well.
  - W4 (+4.2 %) has no priced lever.

### 0.2 Sleeping on: nothing is measured on V2, for either engine

| input | value | source | what it is not |
|---|---|---|---|
| Jolt: all-asleep frame, asleep tail | frame 127; tail 2.17 µs (W1) / 2.29 µs (W8) | `docs/measurements/2026-09-19-physics-p0/ANALYSIS.md:152-154` | **Jolt v5.3.0**, not 5.6 (`tools/physics_parity/driver.py:145`; `p0b/window_report.md:192`, `:198`) |
| ours: V2 freeze step, sleeping on, trunk | median 144, max 148 with reuse on (0/17 rises); 142–210 with reuse off (6/17 rise once, +60 each) | `D:/tmp/phys-orch/cs/proto/sleep_sweep/SWEEP.md:15-16`, `:34-35` | untimed; before SL |
| ours: frozen floor | steady 0.1085 / 0.1177 ms (W1/W8). The [274,500) slice reads 0.1254 / 0.1337 because it contains move-in steps 274-277 at 1.90 / 1.77 / 0.39 / 0.67 ms (W1). | `docs/measurements/2026-09-28-physics-window8b/analysis/jsondm1.md:145`, `:4-5`, `:111-115` | **pre-V2** (Tree + Sets). V2 nearly doubles the held contact set: 8,501 against 4,468 manifolds, 30.9k against 16.6k points (`win9b/analysis.md:155`). That set feeds the store and graph pieces of the floor (`jsondm1.md:147-149`). |
| steady floors compared | ours ≈ 50× Jolt's tail (0.1085 / 0.00217; 0.1177 / 0.00229) | the rows above | cross-build (v5.3.0) and pre-V2 |

**Arithmetic (cross-build, cross-window; not a claim; derivation in §11).**
- **Assumptions.** SL-1 is in, the dwell stays at 60 frames, and there are no rises: the seal guarantees that on J-T (§3.4).
- **Estimates against Jolt.** Jolt 5.6 is estimated at v5.3.0's freeze frame (A_jolt = 27 awake steps in the window). We would be **1.59–1.64× slower at W1, 1.86–1.95× at W2, 2.13–2.27× at W4, 2.45–2.62× at W8 and 2.88–3.07× at W16**.
- **Sensitivity.** Jolt 5.6's freeze frame is unmeasured. Over A_jolt ∈ [14, 40] the ratio spans 1.08–3.15× at W1, 1.66–5.00× at W8 and 1.95–4.74× at W16.
- **The two causes.**
  1. A 60-frame dwell (1.0 s at J-T's 60 Hz) where Jolt, Rapier and Box2D use 0.5 s.
  2. A frozen pile that still costs about 0.11–0.14 ms per step against Jolt's 2.2 µs.
- **With a 30-frame dwell** (SL-V, if it passes its bars), W1 becomes 0.43–1.32× Jolt over the same range, and 0.63–0.69× at v5.3.0's frame. "Ours ahead at W1" therefore depends on Jolt 5.6 needing ≥ about 19 awake steps after step 100. Only S-SL0 can say.

**What fixes this without a timing window.** S-SL0 (§10.0): one untimed `-allow_sleep` J-T run of the exact Jolt 5.6 binary 9b used. It reads Jolt 5.6's per-frame active-body count and replaces A_jolt with a measurement. **What decides:** window 9c, in-block, every scene with sleeping off and on (§15).

**Plain answer for the owner:**
- **Sleeping off (measured).** We beat Jolt 5.6 only at W1 (−12.6 %). We are level at W2. Jolt is ahead by 4 / 13 / 36 % at W4 / W8 / W16.
- **Sleeping on.** There is no Jolt 5.6 number yet. The arithmetic puts us 1.6–3.1× slower, and both causes are known: the dwell, and the frozen floor.

---

## 1. Goal and non-goals

**Goals**
- **G1, the defect.** A simulated dynamic body has linear and angular velocity exactly zero on every step its island is frozen, as in Jolt 5.6, Rapier 0.36, Box2D v3 and PhysX 5.
  - The zero is applied on **every step an island goes to sleep**, i.e. at every freeze attempt, inside that step, before user code runs.
- **G2, a neutral draw.** The zero must not make an island's own freeze attempt more likely to fail. The seal (§3.4) ensures this on every step where no external input changed.
- **G3, wake rules for default-on correctness.** Each trigger a default-on world meets is wired or recorded. A velocity written into a sleeping body wakes its island on the next step (SL-1b, owner-gated, §18 Q1).
- **G4, the flip.** `PhysicsConfig::sleeping` defaults to `true` (`resources.rs:731-733`), under L10's C1b rule and 9c's on/off bar (§7).
- **G5, the values.** Recommend `sleep_frames` and `sleep_threshold` from one untimed screen under ruling 24's criterion (§10.4).
- **G6, window 9c.** Every scene runs with sleeping off and on, for ours, Jolt and Rapier (§15).

**Non-goals**
- **N1.** No new sleep metric: no position term, no extent-scaled ω (PC-SL-2).
- **N2.** No reduction of the frozen floor (PC-SL-1, §18 Q5).
- **N3.** No transform-write wake, no per-entity wake API, no kinematic wake.
  - The seal does not add any of them.
  - It only refuses to re-key when such a write lands on the one step it governs (§3.4).
- **N4.** L10's held-island machinery (Sets) is unchanged. Only doc text is touched.
- **N5.** Sleeping-off bytes do not move.
- **N6.** No timing in this lane. The screens are untimed; 9c times.

---

## 2. Facts the design rests on (verified on f277269d and in the local engine sources)

### 2.1 Where our freeze path keeps the velocity

1. **The latch.**
   - `end_step` accumulates each island's max |v|² + |ω|² from the post-solve snapshot (`resources.rs:4606-4623`).
   - Below threshold, it ticks every member row's debounce and latches `asleep = below_count >= frames` (`:4638-4642`).
   - Otherwise it resets the row (`:4643-4646`).
2. **The freeze decision.** The next `begin_step_upto` freezes an island iff every member row is `asleep` (fold, `:4516-4520`). That fold runs after the per-row count comparison (`:4508-4514`).
3. **Capture and restore.**
   - Every non-fast step captures the gathered `BodyState` of each frozen row (`solver/colored.rs:5245-5263`).
   - After the substeps the capture is put back, and "the slept body keeps its frozen velocity" (`:5461-5482`).
4. **Write-back.**
   - `write_back_awake` skips frozen rows (`colored.rs:4971-4986`), so `physics_apply` never writes them (`systems.rs:2181-2213`).
   - The component therefore keeps v_N, the solver's velocity on the latch step, for the whole sleep.
5. **This was a choice, now reversed.**
   - L10 D13: "the at-rest velocity is kept" (`docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md:188`).
   - Ruling 25 moves the defect to SL.
6. **The size.**
   - By construction |v|² + |ω|² < 1e-4 (`resources.rs:609-615`), so |v| < 10 mm/s.
   - Measured on V2: up to 1.76 mm/s (`research_tree.md` §3).
7. **The round trip is exact.**
   - The gather copies `position`, `rotation`, `linear_velocity` and `angular_velocity` verbatim (`resources.rs:4772-4775`).
   - The apply writes those four verbatim for touched rows (`systems.rs:2195-2200`).
   - So what the post-solve snapshot holds for a touched row is, bit for bit, what the next gather reads, unless something else writes the component in between.
   - The other gathered fields (`inv_mass`, `restitution`, `friction`, `shape`, `is_sensor`, `simulated`, `kinematic`) come from components that the apply never writes (`resources.rs:4756-4785`).

### 2.2 What the reference engines do (local sources, read)

| engine | at sleep | on a velocity write to a sleeper | does the pile's contact set get re-tested while asleep? | evidence |
|---|---|---|---|---|
| Jolt 5.6 | `DeactivateBodies` zeroes both velocities | `SetLinearVelocity` / `AddLinearVelocity` activate unless the value is near zero | **No.** Pairs are found only for active bodies, and the speculative distance is a constant 0.02 with no velocity term | `D:/tmp/jolt/wt-v5.6.0-parity/Jolt/Physics/Body/BodyManager.cpp:559-561`; `Body/BodyInterface.cpp:595-608`, `:624-638`; `PhysicsSystem.cpp:962-966` (`FindCollidingPairs(active_bodies, …)`); `PhysicsSettings.h:50`; `PhysicsSystem.cpp:1101` |
| Jolt 5.6, sleep test | the island sleeps on the last collision step if every body's 3 points stayed within 0.03 × 0.5 s = 15 mm for 0.5 s; sensors never sleep | — | — | `PhysicsSystem.cpp:2511-2563`; `Body/Body.cpp:145-184`; `PhysicsSettings.h:95`, `:102`, `:123` |
| Rapier 0.36 | `RigidBody::sleep()` sets the velocities to zero for every body of a sleeping island | `set_linvel(v, wake_up)` and `apply_impulse(.., wake_up)` | sleeping bodies are skipped by the solver; contact start wakes | `$R/dynamics/rigid_body.rs:915-918`, `:1016-1030`; `rigid_body_components.rs:1362-1377`, `:1487` (`$R` = `$CARGO_HOME/registry/src/index.crates.io-1949cf8c6b5b557f/rapier3d-0.36.0/src`) |
| Box2D v3 | a sleeping set keeps no velocity; a wake starts from `b2_identityBodyState` | `SetLinearVelocity` wakes on a non-zero write | sleeping sets are not simulated | `research_web.md` [11] |
| PhysX 5 | "the linear and angular velocity is zero" while asleep | — | — | `research_web.md` [13] |

**Placement.** All of them zero the velocity inside the step that puts the body to sleep, and make a velocity write wake it.

**Re-testing.** None of them re-tests a sleeping pile's contact set every step with a velocity-dependent margin. Ours does, under Off's semantics, which Sets reproduces through the logical count. That is where C1 comes from, and it is why copying the reference engines' *placement* does not give their *semantics* (critique r1, C1).

### 2.3 The margin coupling (corrected; critique r1 C1)

**V2's margins read velocity, and every velocity term can only widen a margin.**
- **Per-body broadphase margin:** `d/2 + min(cap, (|v| + |ω|R)·h)` (`narrowphase/speculative.rs:122-130`).
- **Per-axis SAT margin:** `widen(−Δv·n + spin)` with `spin = |ω_A|R_A + |ω_B|R_B ≥ 0` (`:185`, `:214-218`). Here `widen(x) = d + min(cap, x·h)` for x > 0, and d otherwise (`:208-212`).
- **Per-point margin:** `widen(−(v_B(p) − v_A(p))·n)` (`:220-224`).
- **At zero velocity every one of them is exactly `d`.**
- **Existence.** Box-box existence is "a function of the two poses and the pair's speculative margin alone" (`narrowphase/box_box.rs:42-45`). Past a non-separating SAT, every path ends in a contact (`:1245-1348`). So the margin enters a box-box pair's existence only through the SAT early-out and the broadphase candidacy, and both are monotone in velocity.

**The timeline of one freeze attempt.** The island is solved at N, every member latches, and it is frozen from N+1 if nothing changes.

| step | what the count `K` is computed from | trunk | SL-1 without the seal (rev 1) |
|---|---|---|---|
| N (gather) | poses p_N, margins from v_{N−1} | K_N stored as the key | K_N stored as the key |
| N (end) | — | v_N kept in the component | v zeroed in the component (the settle) |
| N+1 (gather) | poses p_{N+1}, margins from … | v_N | **0**, every margin at its minimum |
| N+1 (begin) | K_{N+1} compared with K_N | margin drift v_{N−1} → v_N, both tiny and nearly equal, plus pose drift: either sign | **the full margin term removed**, plus pose drift: biased toward losing manifolds |

**Consequences** (each CONFIRMED by the cited lines):
- **A one-way shift.** Without the seal, SL's draw is the trunk's draw plus a shift that only removes. A pair whose SAT gap at p_{N+1} lies in (d, d + its v_N margin term] loses its manifold. Any change of the island's count wakes every latched row with a different key (`resources.rs:4508-4514`).
- **It lands on every attempt.** If an attempt fails (a rise), the pile's 60-frame countdown restarts, and the next attempt settles and shifts again.
- **Size.**
  - The shift's width is the margin term at v_N: ≤ (0.01 + 0.01 × 1.73) / 60 ≈ 0.46 mm for a J-T box at the threshold.
  - At V2's measured 1.76 mm/s plus rest-level spin it is about 0.03–0.06 mm (inference).
  - How many J-T pairs sit in that band above d is **unknown**. V2 adds about 4,000 speculative manifolds at rest (`win9b/analysis.md:155`).
  - The trunk V2 already rises in 6/17 reuse-off runs at its smaller draw (`SWEEP.md:16`).
- **Cost.** Every rise is a whole-pile wake that costs exactly 60 awake steps (`SWEEP.md:34-35`). On J-T [100,500) one added rise costs +1.22 ms at W1 and +0.43 ms at W8 on the sleeping-on mean (§11).
- **The research named this the main risk** and required it shown harmless before landing (`research_tree.md:188-195`, `:339`).

**Rev 1 was wrong** to call this "one count draw per freeze, as today". Rev 2 removes the shift's effect structurally (§3.4) and measures its raw size (S-SL1 arm D, §10.1).

### 2.4 Constraints on any fix, read from our code

- **C1, do not erase a user write.**
  - S-KICK and S-SHOOT write `RigidBody` between steps, as velocity changes (`D:/tmp/phys-orch/dyn-scenes/cut.md:24-28`).
  - The capture copies whatever the gather read (`colored.rs:5258-5260`).
  - So a zero applied at the capture would erase a kick.
- **C2, zeroing at a per-row latch flip perturbs awake islands.**
  - Rows latch per row (`resources.rs:4638-4642`).
  - A latched row inside an island that still has an unlatched member is still being solved.
- **C3, the zero must reach the component on the step it is made.**
  - Rows of an island solved at N are touched by `write_back_awake` (`colored.rs:4980-4983`).
  - A first-frozen row is not touched.
- **C4, mode independence.**
  - `tests/sleep_skip_bit_identity.rs` compares Off and Sets on every step.
  - So every decision must come from inputs both modes have. Off keeps no previous-step baseline (`systems.rs:280-286`).
- **C5, the guard must not hide an external change.**
  - The trunk keeps the attempt-step comparison because "skipping it would leave a support removed on that step undetected" (`resources.rs:4373-4378`).
  - Whatever replaces a wake on that step must still wake on every external change that the comparison catches today.

---

## 3. The design

### 3.1 The invariant

**SL-INV.** On every step an island is frozen, each of its simulated dynamic member rows enters the solve with `linear_velocity` and `angular_velocity` component-wise `== 0.0`.

Three parts deliver it and keep it neutral:
- **SL-1a, the settle:** the zero.
- **SL-1s, the seal:** makes the zero unable to wake its own island.
- **SL-1b, the write-wake:** keeps SL-INV exact under user writes. Owner-gated.

### 3.2 The rest state

`SleepLatch`'s spare byte (`row_identity.rs:251`) becomes `rest: u8`:

| value | name | meaning |
|---|---|---|
| 0 | `REST_NONE` | not resting: never settled, solved since, or woken |
| 1 | `REST_SETTLED` | the row's island went to sleep at the last `end_step`, and the settle zeroed its velocity. Its stored key was sampled on that step's gather, before the zero (a moving basis). Lives only from `end_step` N to `begin_step` N+1. |
| 2 | `REST_FROZEN` | the row has passed a `begin_step` since its settle. Its stored key was sampled with the settled, zero velocities. |

**Invariant:** `rest != REST_NONE ⇒ asleep`.

**Transitions:**

| where | condition | effect |
|---|---|---|
| `begin_step` | global wake (`resources.rs:4452-4457`) | asleep = false, below = 0, rest = NONE |
| `begin_step` | row has no island (`:4489-4494`) | key = sentinel, rest = NONE |
| `begin_step` [SL-1b] | rest ≠ NONE, simulated, gathered velocity ≠ 0 | asleep = false, below = 0, rest = NONE, `write_wakes += 1` |
| `begin_step` | asleep, key differs, rest = SETTLED, **sealed** (§3.4) | **re-key:** the latch is kept, `settle_rekeys += 1` |
| `begin_step` | asleep, key differs, otherwise | as the trunk: asleep = false, below = 0, rest = NONE, `contact_wakes += 1` |
| `begin_step` | after the above, rest = SETTLED | rest = FROZEN |
| `begin_step` fold | any member with rest ≠ FROZEN | the island is active |
| `end_step` | island frozen this step | rest unchanged |
| `end_step` | island solved this step | rest = NONE, then the settle may set SETTLED |
| `end_step` settle | island solved this step and every member latched after the update | rest = SETTLED for every member, `rows_put_to_sleep += members` |

### 3.3 SL-1a, the settle

**1. `end_step`** (the step N on which the island was solved). Existing loops gain an AND and a store, plus an O(islands) scan.
- **The island flag.** `sleeps` = "solved this step" (`frozen == 0`) AND "every member row is latched after this update".
  - It is seeded in the reset loop (`resources.rs:4601-4603`).
  - It is cleared by any member that ends the update unlatched (`:4627-4647`).
- **The row state.** In the latch loop, a row of a solved island gets `rest = REST_NONE`; a row of a frozen island keeps its `rest`.
- **The settling pass.** If any island has `sleeps`, a `#[cold] #[inline(never)]` pass sets `rest = REST_SETTLED` on its members and raises `settle_pending`.

**2. `settle(bodies, graph, touched, epoch)`** (new). The solve calls it right after `end_step`, before `physics_apply`. It is `#[inline]`, with an early return unless `settle_pending`. Its cold body does three things:
- **(a) The zero.** It writes `+0.0` into both velocities of every member row with `simulated && inv_mass != 0.0` of every `sleeps` island in the post-solve snapshot, with `debug_assert!(touched.get(row))` (C3).
  - So `physics_apply` writes the zero to the component on step N itself.
  - Positions and rotations are untouched.
- **(b) The seal stash** (§3.4): the watched rows, copied after the zero.
- **(c)** It records `epoch`, copies the latch's gather cursor into the seal's cursor, sets `seal_pending`, and clears `settle_pending`.

**3. The fold reads `rest`, not `asleep`.** An island is frozen iff every member row has `rest == REST_FROZEN` after the per-row updates.
- **In normal flow this is the same decision as the trunk's:** every member of an island that settled at N is SETTLED, then FROZEN at N+1.
- **It differs in one case: rows that latched while their island had an unlatched member.**
  - If such rows end up in an island whose count equals their stored key, the trunk freezes them with their solved velocity. This is the "adds exactly as many as it removes" gap (`resources.rs:4402-4403`).
  - Under SL they are solved for one more step, settle at that step's `end_step`, and freeze with zero velocity.

**4. Every unlatch path clears `rest`** (table above). `rekey_rows` carries the whole `SleepLatch` through row moves (`:4296-4315`).

**5. Why the sleep step, not a later one.**
- This is Jolt's `DeactivateBodies` placement and Rapier's `RigidBody::sleep` placement.
- User code runs only after it, so no user write can land between the latch and the zero (C1).
- The seal then makes the placement's consequence (§2.3) harmless.

### 3.4 SL-1s, the seal (the C1 resolution)

**Rule.** On the first step after a settle (rows in `REST_SETTLED`), if the island's manifold count differs from the stored key **and the step is sealed**, the row is re-keyed: its latch is kept, the new count is stored, and the row becomes `REST_FROZEN`. On every other step, and on an unsealed first step, a count difference wakes exactly as on the trunk.

**Sealed** means all five of the following hold. Each is evaluated from inputs that Off and Sets share.
1. **Exactly one gather later, rows stable.** The seal cursor, copied from the latch cursor at the settle, classifies as `RowRemap::Identity` against this gather. `classify` gives `Identity` for the next gather only when the gathered rows are the same entities in the same order (`row_identity.rs:589-613`). So any spawn, despawn or archetype move anywhere in the world unseals the step. A skipped gather gives `Reset`, which also unseals.
2. **Same narrowphase configuration.** `SealEpoch` is equal. It holds the bits of `speculative_distance`, `speculative_velocity_cap`, `contact_reuse`, `contact_reuse_distance`, `dt` and `sdf_narrowphase`, plus the SDF field's edit count and an FNV-1a 64 hash of every edit's bits (`sleep_sets.rs:376-392`'s `edit_bits`). This is L10's `SleepEpoch` (`sleep_sets.rs:284-320`) **without** `mode` and `warm`: those differ between Off and Sets, or do not reach a manifold's existence.
3. **Every watched row unchanged.** The watched rows are:
   - every member row of an island that settled at N;
   - every row that is not an island member, i.e. `!is_dynamic_row(inv_mass)` on the gathered state (`solver/contact.rs:38-40`): statics, kinematic bodies, and any body the user gave zero inverse mass.

   Each watched row's gathered state at N+1 must equal the stash, taken from the post-solve snapshot after the zero, on every **user-writable** field: `position`, `rotation`, `linear_velocity`, `angular_velocity`, `inv_mass`, `inv_inertia_local`, `restitution`, `friction`, `shape`, `is_sensor`, `simulated`, `kinematic`. The comparison is by bits.

   Two fields are **excluded because they are derived:**
   - `inv_inertia`: the gather recomputes it from `rotation` and `inv_inertia_local` (`resources.rs:4764-4768`), and the solver's inertia refresh need not match the gather's bits;
   - `bp_margin`: the gather recomputes it from the velocities, the shape and (2)'s config (`systems.rs:311`).

   A new function `seal_equal` destructures `BodyState` exhaustively, as `bit_equal` does (`sleep_sets.rs:394-447`), so a field added later fails to compile until someone classifies it.
4. **No SDF field edit.** This is folded into (2).
5. **No held manifold is needed.** The rule looks only at rows, never at manifold lists. So it does not depend on where Sets keeps a frozen island's manifolds (C4).

**Why a sealed count change is internal (proof sketch).**
- **What Off's narrowphase reads for a pair:** the two gathered `BodyState`s, the narrowphase configuration, the SDF field, and its own caches (reuse records, axis hints). L10's epoch documentation lists exactly these as the inputs a held island's Off outputs depend on (`sleep_sets.rs:284-292`).
- **Where the island's pairs end.** At N+1 every pair of the island has its endpoints among the island's members and the non-member rows. A movable non-member endpoint would union into the island at N+1. That island would then hold a row that is not `REST_SETTLED`, so the fold keeps it active on that step, and its sleep is decided by `end_step`'s energy as always.
- **What a sealed step leaves.** Every input of every pair of the island at N+1 equals step N's input except three things, all produced by the engine:
  - (a) the members' velocities, which the settle zeroed (and the `bp_margin` derived from them);
  - (b) the members' poses, which step N's solve produced;
  - (c) the narrowphase caches carried from N. For box-box pairs, a count change from caches at fixed poses is already a defect (`resources.rs:4392-4396`).

**Why re-keying an internal change is safe.**
1. **The island is at rest.** It stayed below the energy threshold for `sleep_frames` consecutive steps (the latch).
2. **Nothing external changed** (the seal). Every external change the trunk's comparison catches on that step still unseals it and wakes as before. That covers a despawned support (1), a moved or resized support or member (3), a config or SDF change (2), and a velocity write (3, and SL-1b). C5 holds.
3. **What flips is a d-boundary speculative pair.**
   - Under V2 (d = 20 mm, `resources.rs:630`) a manifold whose existence flips under internal motion of at most v_th·h is a pair whose SAT gap is within that distance of d. That distance is 0.17 mm at the default threshold and 0.5 mm at SL-V's 9e-4.
   - Such a pair sits about 20 mm apart and carries no load at rest. Its speculative point acts only if the pair closes by about s/h ≈ 1.2 m/s in one step.
   - A load-bearing contact (s ≤ 0) cannot cross d.
   - Under the overlap-only rule (d = 0, not shipped) the flipping pair is a knife-edge touch moving at most 0.17 mm. The frozen pose then differs from a supported one by at most that much.
4. **Jolt re-tests nothing while asleep** (`PhysicsSystem.cpp:962-966`). The sealed re-key is strictly more conservative than the reference.

**What the seal changes beyond C1 (a consequence, Q3).**
- The trunk's own attempt-step onset draws, v_{N−1} → v_N plus the motion of step N, are also internal. On a sealed step they re-key too.
- On J-T, which has no external input, every attempt is sealed. A pile therefore freezes at its first attempt.
- This removes V2's own 6/17 reuse-off rises (`SWEEP.md:16`) and every CS candidate's (B+T 15/32, A+T 14/32, `SWEEP.md:187`).
- Ruling 24 found every such re-wake invisible and cost-only: 117 intervals, max corner displacement 1.91 mm, 0 visible (`SWEEP.md:234-236`). So this is a perf decision the orchestrator makes (§18 Q3).

**What the seal does not cover (recorded, §12).** On an *unsealed* first step after a settle the trunk rule applies, with the shifted draw of §2.3. Unsealed steps are:
- a spawn or despawn anywhere in the world;
- any write to any watched row;
- a config change;
- an SDF edit.

A world that writes a kinematic or static row every frame therefore never seals, and keeps rev 1's behaviour. Kinematic motion is unbuilt today (`components.rs:64-74`, per research), and no shipped scene writes a watched row every frame. Narrowing the watch to the island's actual partners, so that unrelated writes and spawns do not unseal, is PC-SL-6.

**G2/G7 must count re-keys.**
- Their event today is "a step on which `contact_wakes` rose" (`tests/sleep_settles_box_piles.rs:285-310`).
- Under the seal, a sealed count change at an attempt is a re-key, not a wake. Counted the old way, the gates would read 0 by construction: a gate that could not fail.
- **Re-expressed:** an event is a step on which `contact_wakes` **or** `settle_rekeys` rose, i.e. a count change at a latch attempt however it is resolved. The budgets stay 0 (§8.2).

### 3.5 SL-1b, the velocity write-wake (owner-gated: L10 open question 1, `04-DESIGN-REV2.md:630-631`)

**Rule.** In `begin_step_upto`'s per-row loop, **before** the count comparison (so a kick is counted as a write, O1): a row with `rest != REST_NONE`, `simulated`, and a gathered velocity with any component `!= 0.0` is woken. −0.0 reads as zero; NaN reads as a write.

Under SL-1a, a resting row's velocity is exactly zero unless something other than the solver wrote it. So the test is exact, and needs no baseline and no change ticks.

**Consequences:**
- **A kick into a frozen body is solved on the next step.**
  - On the trunk, a kick whose margin change alters the island's count already wakes on that step: a kick toward a neighbour within d + v·h, with the cap at 0.5 m (`resources.rs:636`).
  - A lateral kick with no neighbour in reach wakes only one step later, through `end_step`'s energy (inference: `colored.rs:5258`, `:5475-5481`, `resources.rs:4638-4646`).
  - SL-1b removes that lag (matching Jolt's `AddLinearVelocity`, `BodyInterface.cpp:624-638`, and Rapier's `set_linvel(.., true)`). S-SL3 splits the two causes per event (O1).
- **A sub-threshold write wakes the body.** On the trunk it is kept, frozen, forever.
- **The soft→rigid reaction wakes a sleeper.**
  - `physics_soft_rigid_apply` writes `linear_velocity += dl` whenever the reaction is non-zero (`soft/coupling.rs:543-559`).
  - Soft bodies never sleep (no sleep state in `src/soft/`). So **a rigid island under a resting soft load cycles: about 60 awake steps, a settle, a write-wake, and again.** It is never frozen for more than the one step the write is read on.
  - On the trunk the same island stays frozen while its component accumulates `dl` every step. The stored velocity grows without bound, and a later wake launches it from that velocity.
  - SL-1b trades that defect for the cycle. It is stated in the docs, pinned by T-SL6b, and recorded as PC-SL-7 (soft-body sleep).
- **A non-simulated dynamic row is exempt.** The engine never writes it (`colored.rs:4980`).
- **SL-INV becomes a debug invariant** (the capture assert, §4.4).

**Under Sets.** A written held row fails R3 (`sleep_sets.rs:1193-1199`), so D1 restores its record before the narrowphase (`:1236-1237`). `begin_step` therefore sees the same inputs in both modes.

### 3.6 Alternatives rejected

| alternative | why not |
|---|---|
| **A1.** Zero at every freeze capture (`colored.rs:5258`) | It erases user writes (C1). S-KICK lands its first event on a pile frozen since about step 144; 62 boxes' kicks would vanish. |
| **A2.** Zero per row at the latch flip (`resources.rs:4642`) | It zeroes latched rows of islands that are still being solved (C2). |
| **A3.** Zero at the first frozen step N+1 (research option B) | (i) Frozen rows are not touched, so it needs a write-back path and a `bp_margin` recompute for them. (ii) A user write between N and N+1 cannot be told from v_N without a baseline that Off does not keep. (iii) **Corrected from rev 1:** it does not avoid the shift. It moves the same one-way shift (v_N → 0) to the comparison at N+2. There the poses are frozen, so the shift is the *only* change between the two counts. |
| **A4.** No rest state: freeze iff latched and v == 0 | The coincidence island of §3.3 item 3 would be unlatched, with a debounce restart, instead of solved one more step. It also needs a second O(rows) pass every step. |
| **A5.** ECS change ticks at the gather | The apply's own write of the zero sets the tick that the next gather sees (W6, `resources.rs:4344-4346`). Working around that needs the previous step's touched mask carried through a row remap. |
| **A6.** Keep L10's baseline under Off too, and compare it in `begin_step` | That is a general transform-write wake (N3). The seal compares only on the one step it governs, against its own stash. |
| **A7.** Zero inside `end_step` | Same semantics; rejected for churn only. There are 62 direct-drive call sites in `colored_tests.rs` and 14 in `resources_tests.rs` (grep). |
| **A8. A "confirm step"** (critique direction 1): on a count change at N+1, solve once more from rest, re-key on the zero basis, then compare by the trunk rule | It costs one full solved step on every attempt where the zero changes the count. Against a parent that does not rise (V2 reuse on: 0/17, `SWEEP.md:15`), that fails ruling 24 (ii)'s letter by about +0.7 % W_sleep per attempt (inference: 1 of about 144 awake steps). Without an external-change guard it also cannot tell a support despawned on that step whose one-step response is below the threshold. Example: a 2 m box that starts to pivot with its CoM < 16 cm past the edge gives ω² < 1e-4 after one step at 60 Hz (inference). |
| **A9. A zero-basis recount at the settle** (critique direction 1, read literally): store, as the key, the count the island would have at v = 0 | Exact only by replicating the narrowphase's existence rule outside it: broadphase sphere test, 15-axis SAT at d, reuse refresh, sphere and SDF drop rules (`box_box.rs:42-45`, `reuse.rs:621`). It would also have to track CS, which changes those rules in flight (ruling 20). It is fragile by construction. |
| **A10. A velocity-free key:** count only manifolds that exist at the zero basis | Needs a per-manifold flag from every generator, a per-island second count, and L10's `IslandInfo` to carry a second held count (`resources.rs:3184-3195`; no spare bytes). That touches the narrowphase, the graph and Sets, all in flight. |
| **A11. The settle without a guard** (rev 1) | C1. Kept only as the diagnostic arm D of S-SL1, built from a scratch patch and never committed. |
| **A12. A seal over all rows of the world** | Any write or kick to any awake body anywhere would unseal (S-KICK). Awake islands' rows are excluded because a write to them cannot change a settling island's count: they are separate islands at N, and touching at N+1 would merge them and keep the island active. |

### 3.7 The rungs

| rung | content | moves | lands |
|---|---|---|---|
| **SL-2a** | Runner: lift the `--dyn` + `--sleeping on` refusal (`dyn-scenes/cut.md:344`). Explicit `--sleeping` flags on every pinned row and gate script (§7). No simulation code. | no pin. Receipt: every sleeping-off row byte-identical to the previous trunk executable | after dyn-scenes, before SL-1 |
| **SL-1** | The rest state, the settle (SL-1a), the seal (SL-1s), the fold on `rest`, the diagnostics, the capture assert (debug, with SL-1b only), the doc corrections, the G2/G7 re-expression, the B1 and oracle re-expressions, the runner receipts | sleeping-on pins only, each by its rule (§8.2) | after SR-B and S5, before IB (§16) |
| **SL-1b** | The velocity write-wake, and the T1/H-03 re-expression | sleeping-on behaviour on writes; no committed pin predicted | with SL-1 if the owner says yes (Q1) |
| **SL-V** | The values `sleep_frames` / `sleep_threshold`, chosen by S-SL4, plus the settle limits derived from them | every sleeping-on pin again, by the same rules | after CS |
| **SL-2b** | Jolt `-dyn` + `-allow_sleep` lift (`dyn-scenes/cut.md:467`); the Rapier sleeping arm; the 9c driver accepting `allow_sleep=1` | no pin | before 9c |
| **SL-3** | The flip `sleeping: true`, under L10's C1b rule | no pin, by the rule | **after 9c**, iff 9c's on/off bar holds (§7) |

---

## 4. Exact code sites and the change at each

### 4.0 Mapping onto SR-B and S5

**SR-B.**
- It builds its region table "serially after the freeze capture" (`sr-b/cut.md:157`).
- It keeps "the swap and the cursor stamp; the held carry additions; the freeze restore; the write-back; `end_step`" serial after the region, in that order (`:234`).
- So `rekey_rows` and `begin_step_upto` (trunk `colored.rs:5110-5147`) sit before SR-B's build, unchanged.
- The capture assert sits in the capture loop that SR-B keeps.
- `settle` goes right after `end_step`, at the end of SR-B's serial tail.

**S5.** It locks `sleep_skip_bit_identity.rs`, `step_inputs.rs`'s test module, `ug02-pins.tsv` and `alloc_frame_attribution.rs` (ruling 19 Q1). SL edits none of them (T-SL8 is a new file).

The lines below are the trunk's.

### 4.1 `row_identity.rs`

- **`:239-252` `SleepLatch`.** `_pad: u8` becomes `pub(crate) rest: u8`, with doc text taken from §3.2's table.
- **New consts.** `pub(crate) const REST_NONE: u8 = 0; REST_SETTLED: u8 = 1; REST_FROZEN: u8 = 2;`.
- **`:256` `FRESH`, `:260-267` `new`:** `rest: REST_NONE`.
- **`:270-273`:** the 8 B / align 4 assert must still hold.

### 4.2 `resources.rs`

**`IslandScratch` (`:4030-4046`).**
- `_p: [u8; 3]` becomes `sleeps: u8, _p: [u8; 2]`.
- `CANDIDATE` and `THAWED` get `sleeps: 0`.
- The `:4048-4051` assert stays.

**New `SealEpoch`.**
- **Type and fields:** `#[derive(Clone, Copy, Debug, PartialEq, Eq)]`, 32 B. It holds the bits of `speculative_distance`, `speculative_velocity_cap`, `contact_reuse_distance` and `dt`; `contact_reuse` and the `sdf_narrowphase` kernel as `u8`; the SDF edit count; the edit-list FNV-1a 64; and a `set: u8` that makes `UNSET` differ from every built epoch.
- **Constructors:** `UNSET`, and `fn of(cfg: &PhysicsConfig, field: Option<&SdfField>) -> Self`. The latter reuses `sleep_sets::edit_bits`, made `pub(crate)`.
- **Doc:** "L10's `SleepEpoch` without the mode and warm fields, which differ between Off and Sets and do not decide whether a manifold exists."

**`IslandSleep` fields (`:3964-4025`) and constructor (`:4071-4089`).**
- **Diagnostic counters**, each with a `pub fn` getter like `contact_wakes` (`:4137-4139`): `write_wakes: u64`, `rows_put_to_sleep: u64`, `settle_rekeys: u64`, `seal_breaks: u64`. `seal_breaks` counts first steps after a settle on which a `REST_SETTLED` row's key differed and the seal did not hold.
- **Private state:** `settle_pending: bool`, `seal_pending: bool`, `seal_live: bool`, `seal_cursor: RemapCursor`, `seal_epoch: SealEpoch`.
- **`seal_stash: ScratchColumn<(u32, BodyState)>`.** A kernel column under a new synthetic id registered in `scratch_ids.rs`, like `sleep_latch_prev_id()`. Its element type is the solver's freeze capture's (`colored.rs:1891-1895`). It is written only on settle steps, in row order.

**`latch_fingerprint` (`:4144-4154`).** It hashes `u32::from(l.asleep) | (u32::from(l.below_count) << 8) | (u32::from(l.rest) << 24)`, then the key. Off and Sets are compared on it (`tests/sleep_skip_bit_identity.rs:858`); no absolute value is pinned anywhere (grep: one consumer).

**`force_sleep_row` (`:4200-4206`, `#[cfg(test)]`).** It also sets `rest = REST_FROZEN`, so a direct-drive test that uses the hook keeps the trunk's comparison rule.

**New test hooks (`#[cfg(test)]`):**
- `rest_of(row) -> u8`;
- `arm_seal_for_test()`: sets `seal_live = seal_pending`, so direct-drive tests can exercise the seal without a gather.

**`rekey_rows` (`:4259-4281`).** After the stamp: `self.seal_live = core::mem::take(&mut self.seal_pending) && matches!(self.seal_cursor.peek(rows), RowRemap::Identity);`.

**`begin_step` (`:4415-4418`, direct drive).** It calls the shared body with `bodies = &[]` and `SealEpoch::UNSET`. No write test, and no seal: `seal_live` is only set by `rekey_rows` or the hook.

**`begin_step_upto` (`:4423-4533`).** New signature: `(&mut self, graph, n_rows, upto, bodies: &[BodyState], epoch: SealEpoch)`.
- **Before the loop:** `let sealed = self.seal_live && self.seal_holds(bodies, epoch); self.seal_live = false;`.
  - `seal_holds` is `#[cold] #[inline(never)]`. It returns true iff `epoch == self.seal_epoch` and every stash entry `(row, s)` satisfies `bodies.get(row).is_some_and(|b| seal_equal(&s, b))`.
- **The wake loop (`:4452-4457`)** also sets `rest = REST_NONE`.
- **The no-island branch (`:4489-4494`)** sets `rest = REST_NONE`.
- **The per-row loop (`:4488`)** gains the row index (`enumerate`). Its order:
  1. **[SL-1b]** `if l.rest != REST_NONE && let Some(b) = bodies.get(row) && b.simulated && moved(b) { l.asleep = false; l.below_count = 0; l.rest = REST_NONE; *write_wakes += 1; }`. Here `moved` is the six component-wise `!= 0.0` tests.
  2. **The comparison:**

     ```
     if l.asleep && l.island_key != key {
         if l.rest == REST_SETTLED && sealed { *settle_rekeys += 1; }
         else {
             broke |= l.rest == REST_SETTLED;
             l.asleep = false; l.below_count = 0; l.rest = REST_NONE; *contact_wakes += 1;
         }
     }
     ```
  3. `l.island_key = key;` and `if l.rest == REST_SETTLED { l.rest = REST_FROZEN }`.
  4. **The fold:** `if l.rest != REST_FROZEN { islands[i].frozen = 0 }`.
- **After the loop:** `*seal_breaks += u64::from(broke)`.

**`end_step` (`:4580-4648`).**
- **Reset loop (`:4601-4603`):** `s.sleeps = u8::from(s.frozen == 0)`.
- **Latch loop (`:4627-4647`),** after the `asleep` update:
  - if the island was solved (`islands[isl].frozen == 0`): `l.rest = REST_NONE`, and `if !l.asleep { islands[isl].sleeps = 0 }`;
  - if it was frozen: leave `l.rest`.
- **After the loop:**
  - `self.settle_pending = islands[..n_islands].iter().any(|s| s.sleeps != 0)`;
  - if set, a `#[cold] #[inline(never)]` pass sets `rest = REST_SETTLED` and counts `rows_put_to_sleep` for every dynamic row whose island has `sleeps`.

**New `pub(crate) fn settle(&mut self, bodies: &mut [BodyState], graph: &ConstraintGraph, touched: &TouchedMask, epoch: SealEpoch)`.**
- `#[inline]`, with an early return unless `settle_pending`.
- **Cold body (§3.3 item 2):**
  - (a) the zero, with the `touched` debug assert;
  - (b) `seal_stash.clear()`, then push `(row, bodies[row])` for every row with `!is_dynamic_row(bodies[row].inv_mass)` or whose island `sleeps`;
  - (c) `seal_epoch = epoch; seal_cursor = cursor; seal_pending = true; settle_pending = false`.
- **Note on (c).** The latch cursor was stamped with this step's gather in this step's `rekey_rows` (`:4280`). Copying it records the gather the stash belongs to.

**Docs.**
- **`:460-501`:** the `sleeping` doc gains "a sleeping body's velocity is zero; the step that puts its island to sleep zeroes it (SL)". The "Default OFF" paragraph is rewritten at SL-3.
- **`:617-618`:** "half a second at 120 Hz" becomes "1.0 s at 60 Hz, 0.94 s at the engine's 64 Hz default, 0.5 s at 120 Hz" (R9).
- **`:4317-4411` (`begin_step`'s doc):**
  - add a "# The settle and the seal" section;
  - the "Onset flicker" paragraph (`:4373-4378`) gains: "On the first step after a settle, a count change re-keys when the step is sealed (SL): no external input changed, so the change is the settle's own or the settle step's motion. Otherwise the comparison wakes as before."
  - `:4360-4363`: "only touching manifolds reach the graph" becomes "touching and V2 speculative manifolds";
  - "Not covered" (`:4398-4411`): strike "no gate covers SDF + sleeping" (`tests/sdf_sleep_settles_and_wakes.rs` exists); with SL-1b, "a user write to a sleeping body that changes no contact" becomes "a pose-only write", and the soft→rigid bullet becomes the cycle of §3.5.
- **`:4535-4579` (`end_step`):** "restored-low velocities" becomes "zeroed velocities"; add a "# The settle" section.

### 4.3 `sleep_sets.rs`

- **`seal_equal(a: &BodyState, b: &BodyState) -> bool`** (new, `pub(crate)`, beside `bit_equal` at `:394-447`):
  - an exhaustive destructure;
  - `inv_inertia: _` and `bp_margin: _` are ignored, each with its "derived from" comment (§3.4 item 3);
  - every other field is compared by bits.
- **`edit_bits` (`:376-392`)** becomes `pub(crate)`.
- **Doc sites:**
  - `:314-316` (`vcap_bits`): "a held row keeps the velocity it froze with" becomes "a held row's velocity is zero (SL); a kinematic anchor's is not, and the margin reads it";
  - `:2054-2057` (O6), in `v2_the_epoch_carries_both_speculative_parameters`'s doc: the same correction.
  - The tests' logic is unchanged.

### 4.4 `solver/colored.rs`

- **`solve_colored_inner` (`:5089-5098`)** gains `seal: SealEpoch`.
  - `solve_colored_sleeping` (public, direct) passes `SealEpoch::of(config, None)`.
  - `solve_colored_held` takes it from its caller (§4.5).
  - `solve_colored` (sleeping off) passes `UNSET` and never reads it.
- **`:5140-5143`.** Both arms call `sleep.begin_step_upto(graph, n_rows, upto, scratch.bodies(), seal)`. The direct-drive arm passes `upto = sleep.wake_requests()`, as `begin_step` does today (`resources.rs:4416`).
- **`:5258-5260` [debug, with SL-1b].** Before the push: `debug_assert!(!b.simulated || (b.linear_velocity == Vec3::ZERO && b.angular_velocity == Vec3::ZERO), "invariant: a frozen row's velocity is zero (SL-INV)")`.
- **`:5500-5503`.** After `end_step`:

  ```
  let SolverScratch { bodies, touched, .. } = &mut *scratch;
  sleep.settle(bodies.build_view().as_mut_slice(), graph, touched, seal);
  ```

  The two fields are disjoint borrows, and `sleep_view` is dead by then (`:5498-5499`).
- **Comments.** `:5221-5229` and `:5461-5468`: "with its velocity" / "keeps its frozen velocity" become "keeps its zero velocity (SL)".

### 4.5 `systems.rs`

- **`physics_solve_colored` (`:2092-2124`).** On the sleeping arm, compute `SealEpoch::of(cfg, field)` and pass it to `solve_colored_held`.
  - `field` is the latched SDF field the step's narrowphase read: the same `field` the L10 prologue receives (`:518`).
  - The sleeping-off arm is unchanged, byte for byte.

### 4.6 `benches/jolt_parity_pyramid.rs` (the runner; IB moves it into a package, so whichever lands second rebases)

- **SUMMARY gains** (`:2652`, `:2697`):
  - `rows_put_to_sleep`, `write_wakes`, `settle_rekeys`, `seal_breaks`;
  - `frozen_v_max`: the max over steps of |v|² + |ω|² over frozen simulated rows, read untimed beside `first_frozen_step` (`:2218-2264`). It must be `0` on every sleeping-on row and `null` with sleeping off.
- **SL-2a** (its own commit): lifts the `--dyn` + `--sleeping on` refusal and adds the explicit flags of §7.
- **The CSV is unchanged.** A comparator that diffs SUMMARY JSON across lane and parent ignores added keys. The dyn lane's `existing_unchanged.py` compares the JSON minus timing keys (`dyn-scenes/cut.md:417-426`), so SL's gate passes it the list of new keys.

### 4.7 Tests

- **New files:** `tests/sleep_velocity_zero.rs`, `tests/sleep_write_wake.rs`, `tests/sleep_seal.rs`.
- **New unit tests in `resources_tests.rs`:** T-SL2, T-SL10, T-SL17.
- **Re-expressed:**
  - `tests/frozen_island_warm_start.rs` (B1);
  - `tests/row_identity_remap.rs` T1 and H-03 (with SL-1b);
  - `colored_tests.rs::topology_churn_freeze_is_pure_function_of_row_latch` (O5);
  - `tests/sleep_settles_box_piles.rs` G2/G7 event count.

---

## 5. Data layout and kernel shape

**Memory.**
- `rest` and `sleeps` take the spare bytes of 8 B elements that already exist (`row_identity.rs:270-273`, `resources.rs:4048-4051`).
- New on the resource:
  - one kernel column, `seal_stash`, of 164 B elements (4 B row + 160 B state), sized to the watched rows: 1,241 on J-T (about 204 KB), committed on the first settle;
  - one 32 B `SealEpoch`;
  - four counters and five scalars.
- No `Vec`, `HashMap`, `dyn` or `Box`, and no `unsafe`.

**Work per step, sleeping on:**

| piece | when | cost (inference) |
|---|---|---|
| `rest` byte update, fold on `rest` | every step, in existing loops | ≈ 0 |
| [SL-1b] write test | every step, `rest != NONE` rows only, i.e. frozen piles | One 24 B read per resting row, in a 160 B-strided pass over the gathered rows. v and ω sit in one 64 B line (offsets 84–108 of 160). On J-T that is 1,240 lines (≈ 79 KB), from L2 behind the gather that just wrote them: **0.5–2 µs per frozen step** (O3). |
| `sleeps` flag | every step, `end_step`, existing loop | ≈ 0 |
| settle (zero + `rest` pass + stash) | **only** on a step an island goes to sleep: once per attempt | O(rows) select + 164 B copy per watched row: about 204 KB on J-T, roughly 20–60 µs once per attempt |
| `seal_holds` | **only** on the step after a settle | O(watched) compare: about 204 KB read, roughly 20–60 µs once per attempt |

**Shape.**
- Everything is serial, like every sleep piece today. SR-B keeps the tail serial (`sr-b/cut.md:234`).
- Sleeping off executes none of it: `physics_solve_colored`'s off arm calls `solve_colored` with `sleep = None` (`systems.rs:2116-2123`, `colored.rs:5010-5019`).

**Inlining.**
- `settle` is `#[inline]` with an early return; its body is `#[cold] #[inline(never)]`.
- `seal_holds` is `#[cold] #[inline(never)]`.
- Both are rare branches (principle 7).

---

## 6. Determinism and bit identity

**Across W.**
- `rekey_rows`, `begin_step_upto`, `end_step` and `settle` are serial loops in row order.
- **Their inputs:**
  - the latch, permuted identically at every W;
  - the graph, whose build is W-invariant;
  - the gathered snapshot;
  - the post-solve snapshot, bit-identical across W by the existing gates;
  - the gather cursor;
  - the configuration and SDF edits.
- They write constants (`+0.0`, the `rest` values) and integers, and compare floats by bits only.
- **Gates:**
  - `tests/default_world_sleep_worker_invariance.rs` (sleeping on, relative, no absolute pin);
  - `frozen_island_warm_start.rs`'s cross-W test;
  - T-SL9 at W ∈ {1, 2, 4, 8}.

**Off ≡ Sets.** Every input of every decision is mode-free:
- **The seal's rows-stable test.** `rekey_rows` runs in both modes at the same point (`colored.rs:5110-5112`) on the same `RowIdentity`.
- **The watched set.** It uses the raw `inv_mass` and Off's island ids. Under Sets, held islands keep Off's ids and members (`resources.rs:3422-3436`).
- **The stash.** It is the post-solve snapshot, equal in both modes by L10's theorem.
- **`SealEpoch`.** It is the configuration and SDF edits, with no mode and no warm flag.
- **The count.** It is the logical count, held manifolds included (`resources.rs:4501`).
- **A settling island has no held manifold.** At N it was solved, so its members were not all latched at `begin_step` N; a held island that is woken is restored (D1/D2). The seal never reads manifold lists anyway (§3.4 item 5).
- **The fingerprint.** `latch_fingerprint` includes `rest`, so `sleep_skip_bit_identity.rs` sees it.
- **T-SL8** compares the new counters and the latch under writes and seal breaks in both modes.

**Run to run.** The argument of `resources.rs:3943-3952` applies: integers and exact compares.

**What is NOT bit-identical to the parent:** sleeping-on trajectories, by design (§8). Sleeping-off bytes are identical by construction, because no SL statement runs with `sleep = None`.

---

## 7. The switch and the default flip

**SL-1, SL-1b and the seal add no runtime switch.** They change the semantics of the sleeping arm. `sleeping: false` stays the off switch, at zero cost.
- A runtime "keep the stored velocity" switch would preserve the defect.
- A runtime "no seal" switch would preserve C1.
- The screens compare against the parent executable and against the scratch-patch arm D instead (§10.1).

**SL-3, the flip.**
- `resources.rs:731-733` becomes `sleeping: true`, with the comment: "Default ON since SL (owner ruling 25). A world that must stay byte-identical to the O6/O7 colored solve opts out with `sleeping: false`."
- The `:460-501` "Default OFF" paragraph is rewritten.
- `plugin.rs:663-666` and `:672-675` comments are updated.
- `PhysicsConfig::default().colored` stays `false` (`:708`). The reference solver ignores `sleeping` (`:462-466`).

**When and on what bar** (critique open question 1).
- **When.** SL-3 lands **after 9c**. Every 9c row carries an explicit `--sleeping` flag (ruling 25(d)), so the flip changes no 9c row.
- **The bar.** It is L10 C1b's "no measurable awake cost" (ruling 8), extended to every interaction type (ruling 21). Pre-registered in 9c's prep:
  - S-LAND [0,100) on vs off at every W is **not CLAIMED slower** (ruling 1's letter);
  - the same for S-KICK and S-SHOOT [200,800);
  - J-T [100,500) and S-SLIDE on is not CLAIMED slower than off.
- **Why after 9c.** The serial bookkeeping's share grows as SR, S5 and IB shrink the step (§13). 9c measures it on the final trunk, where that share is largest.
- **Before 9c, the untimed structural bar (ii-dyn) of S-SL3** must also hold (§10.3).
- **If any 9c pair is CLAIMED slower,** it is an owner question with the numbers: flip anyway, do not flip, or open PC-SL-3 (parallel bookkeeping). Ruling 21 puts "maximum speed for every interaction type" against ruling 25's "sleeping on by default", and only the owner trades them.

**The rule for tests** (L10 C1b, `04-DESIGN-REV2.md:578-582`).
- A test that goes red at the flip and whose subject is not sleeping gets an explicit `sleeping: false`. Its pin stays byte-identical, and its first freeze step is recorded in the commit as the reason.
- A test whose subject is sleeping is re-asserted by its own rule.

**The rule for the runner.** `--cfg default` keeps its contract, "follows the shipped default" (`jolt_parity_pyramid.rs:1157-1164`). So SL-2a gives every pinned row and gate script an explicit flag, before SL-1:
- `runner_pins.sh`'s flagless rows (J500, J500off, R1100 in their `_v2` / `_d0` forms);
- SR-B's SR-JD (`sr-b/cut.md:545`);
- the dyn-scenes rows (`dyn-scenes/cut.md:566-569`, `:591-593`);
- the L10 and V2 pose-gate scripts;
- the window drivers' row JSON.

`--cfg a|as|b` already force sleeping off unless asked (`:1154-1155`).

---

## 8. Pins

### 8.1 Must stay byte-identical (SL-2a, SL-1, SL-1b, SL-3)

All of these run with sleeping off, explicitly or through `--cfg a|as|b`:
- `default_world_pyramid_determinism.rs`: `PINNED_FINAL_HASH` and its reuse-off and `_D0` arms;
- `v2_speculative_d0_identity.rs` (12 pins);
- `bodytype_determinism_golden.rs`;
- the census and attribution pins;
- `runner_pins.sh`'s sleeping-off rows;
- `v2_speculative_fidelity.rs` `ANCHOR_REUSE_ON_SLEEP_OFF` (`:112`) and `_OFF_SLEEP_OFF` (`:114`);
- the 26-run gate and the red control (`:395-445`);
- the 88-run ensemble;
- every 9b row (`--sleeping off`);
- the SR-B, S5 and dyn-scenes fixtures.

Relative gates stay green with no edit:
- `sleep_skip_bit_identity.rs` (Off against Sets);
- `default_world_sleep_worker_invariance.rs`.

### 8.2 Move by rule

**The rule is A7-R1's own** (`sleep_settles_box_piles.rs:368-371`). The value moves only with a commit that changes values by design. That commit re-reads it, re-pins it in the same commit, and names the lever and the old value.

**Why the pose pins move.** The pose bytes include velocity (`jolt_parity_pyramid.rs:1957`). The lane gate records, per moved pin, whether positions and rotations are unchanged, i.e. a velocity-only move (G-SL4).

| pin / premise | site | under SL-1 (+1b) | rule |
|---|---|---|---|
| `ANCHOR_REUSE_ON_SLEEP_ON` `0x130c_76cb_6b46_3ab8`, `ANCHOR_REUSE_OFF_SLEEP_ON` | `v2_speculative_fidelity.rs:117`, `:122` | **move, velocity bytes only.** The unperturbed rows do not rise on the trunk: frozen at 147 (reuse on) and 142 (reuse off) (`SWEEP.md:45`, `:62`). Positions and rotations are predicted identical. | A7-R1's rule; `j_t_stands_with_sleeping_on` (`:423-436`) must still hold |
| L10/V2 pose-gate sleeping-on rows (G4/G4p/G4f Son-J1000, RS800) | `docs/measurements/2026-09-30-v2-speculative/gate/l10/` | **move** (velocity). Also the freeze step, where the trunk row rose at an unforced attempt. | same; the scripts are committed docs |
| L10 lane fixtures | `docs/measurements/2026-09-23-l10-sleeping/fixtures/` | **move** | same |
| A7-R1 `0x3a3d_eb44`, `_REUSE_OFF` `0x3a0a_0e22`, `_D0` `0x3a2e_dc99` | `sleep_settles_box_piles.rs:372`, `:382`, `:390` | D_max is a position reading over [600, 3000). It moves **iff** the trunk scene rose at an attempt inside that window, because the seal freezes it at the first attempt. Otherwise unchanged. | the file's rule |
| A7-R2 freeze reading (V2's 86 / 0 / 0) | `sleep_settles_box_piles.rs` | predicted unchanged: no rise on the trunk | re-read and reported |
| **G2/G7 events** | `:285-310`, the counter at `:1051-1066` and `:2342` | **re-expressed (keeps its power):** an event is a step on which `contact_wakes` **or** `settle_rekeys` rose. Budgets stay **0**. Predicted 0: the generator piles start touching (`BOX_SEPARATION = 0`, `:251`), so their pairs sit at s ≈ 0 or a box apart, none near d (inference). | ruling 24(iii): a move above 0 goes to the owner with the numbers, never re-pinned silently |
| settle limits `MEDIUM_SETTLE_LIMIT = 2 × 61`, `G7_SETTLE_LIMIT = 8 × 61` | `:266-284` | unchanged: freezes can only come earlier | re-derived only at SL-V |
| **B1 exact** and the pre-freeze equality | `frozen_island_warm_start.rs:716-750`, `:462-477` | **red by premise:** the twin steps from v_{F−1}, the sleeper wakes from 0 | **Re-expressed:** detect every settle step (`rows_put_to_sleep()` rises) and write `Vec3::ZERO` into the twin's box velocities right after **each** of them (O4). Before each write the equality is asserted on all fields except the sleeper's velocities on the settle step, which must be exactly `+0.0`; after it, on all fields. The wake-step state must equal the twin's. B1's mutation list (`:44-56`) is re-run and must stay red. |
| **T1 and H-03** (with SL-1b) | `row_identity_remap.rs:588-665`, `:780-783`, `:892-945` | **weakened by SL-1b.** The mutant shows as "frozen at its spawn velocity −0.25 m/s", and SL-1b wakes that velocity. | **Re-expressed:** `Y_SPAWN_VELOCITY = Vec3::ZERO` (spawn at rest in contact), asserting "awake on the first step AND velocity bits ≠ zero after it". M3b + M-A must be red on both. |
| **`topology_churn_freeze_is_pure_function_of_row_latch`** (O5) | `colored_tests.rs:3942-4022` | stays green only because `force_sleep_row` sets both bits, so it no longer pins the fold | **Re-expressed:** the oracle reads `rest_of(row) == REST_FROZEN`, the fold's predicate. A churn branch latches rows through `end_step` + `settle` (`rest = SETTLED`), and the next `begin_step` must keep their island active only through the SETTLED → FROZEN step, never through `asleep` alone. |
| `sleep_settles_box_piles.rs:1353-1356` phase 2 ("keep its exact pose, velocity and rotation") | — | green: the zero is applied before the hold | — |
| `colored_tests.rs` sleeping unit tests (`:2953-3240`, `:4135-4160`, `:4355-4375`, `:5850-5875`) | — | predicted green: direct drive (no write test, no seal), and `force_sleep_row` sets `REST_FROZEN`. A red would be a test that latched through `end_step` and changed the count on the next `begin_step`. Such a test now sees a SETTLED row with no seal, i.e. the trunk rule, so it stays green unless it also arms the seal. | tester's red set |
| `alloc_frame_census.rs` sleeping-on scenes (`:2597`, `:2773`) | — | **predicted green, to be checked.** `seal_stash` is a kernel column that commits pages on the first settle, not a heap allocation (inference). Any census red is a STOP, not a re-pin. | tester |
| CS sweep baselines (`cs/proto/sleep_sweep`) | scratch | re-run as S-SL1's control | — |

**SL-V** moves every sleeping-on pin again by the same rules, and re-derives the settle limits that are functions of `sleep_frames`.

**SL-3** moves no pin (§7).

---

## 9. Gates, red-first tests, mutations

### 9.1 Red-first: written first, run on the trunk, red with the stated reason

| id | test | asserts | predicted red on f277269d |
|---|---|---|---|
| T-SL1 | `sleep_velocity_zero.rs::a_pile_that_falls_asleep_has_zero_velocity`. Pipeline `add_physics_colored_solve`, sleeping on, a 4-box column on the J-T floor; Off and Sets; W1 and W8. | on every frozen step, every box's component velocities == 0.0; positions frozen | red: frozen boxes keep their last solved velocity. An anti-vacuity line prints the trunk's value; the test is void if the trunk reads exactly zero. |
| T-SL2 | `resources_tests.rs::an_island_that_never_went_to_sleep_does_not_freeze` (direct drive) | Rows 0 and 1 latch through `end_step` inside island {0,1,2} while row 2 is unlatched. The next graph files {0,1} alone, with the same count as their key. `begin_step` must NOT freeze {0,1}. The following `end_step` + `settle` settles it, and the next `begin_step` freezes it. | red: the trunk freezes {0,1} at once |
| **T-SL10p** | `sleep_velocity_zero.rs::a_pile_whose_first_attempt_flickers_freezes_on_it` (release-only, like the J-T tests in `sleep_settles_box_piles.rs`). J-T at gap 0.5, V2 defaults, contact reuse off, sleeping on, with the runner's one-ulp perturbation `b38xp` applied to box 38's spawn x. | the pile freezes at its first attempt; `contact_wakes == 0`; `settle_rekeys >= 1`; `seal_breaks == 0` | red: the trunk rises at step 146 and freezes at 206 with 1,240 contact wakes (`SWEEP.md:63`). An anti-vacuity line prints the trunk's first rise; the test is void if the trunk does not rise. |
| T-SL3 [1b] | `sleep_write_wake.rs::a_velocity_written_into_a_sleeping_body_is_solved_on_the_next_step`. A lone 4-box column with **no lateral neighbour within d + cap**; Off and Sets; W1 and W8; linear and angular variants. | a lateral write (0.5, 0, 0) m/s to the top box: on the next step the row is awake, its island solved, and the box has moved | red: the trunk keeps it frozen on that step. The write is lateral with no neighbour, so the count does not change (O1). |
| T-SL4 [1b] | `…::a_sub_threshold_write_wakes_a_sleeping_body`. A **lone box** on the floor; a lateral write of 5 mm/s (\|v\|² = 2.5e-5). | woken on the next step | red: the trunk never wakes it |
| T-SL5 [1b] | `…::a_write_right_after_the_island_goes_to_sleep_is_kept`. Lone box. A probe run finds the settle step F; the real run writes a lateral v_k right after step F. | at F+1 the row is awake and solved with v_k | red: the trunk freezes at F+1 and solves at F+2 |
| T-SL6 [1b] | `…::a_soft_body_reaction_wakes_a_sleeping_rigid_body` (`add_physics_soft(.., true)`) | a sleeping box pushed by a soft body wakes | red: the recorded gap (`plugin.rs:434-436`) |
| **T-SL6b** [1b] | `…::a_rigid_body_under_a_resting_soft_load_never_freezes_with_a_velocity` (O2) | on every frozen step SL-INV holds; `write_wakes` rises; the documented cycle: no two consecutive frozen steps while the reaction is non-zero | red: on the trunk the frozen box's stored velocity accumulates `dl` (`soft/coupling.rs:552-559`) |

### 9.2 Guards: green on the trunk, must stay green, each kills a named mutation

All of them run Off and Sets, at W1 and W8, except T-SL9.

| id | test | asserts | kills |
|---|---|---|---|
| T-SL7 | `sleep_velocity_zero.rs::a_latched_row_in_an_island_still_solved_keeps_its_velocity_and_latch` | a pile frozen, then a late joiner lands gently; during the joiner's debounce the pile's rows stay `asleep` and their velocities are the solver's | M-SL2 |
| T-SL8 | `sleep_write_wake.rs::off_and_sets_agree_under_writes` (new file; S5 locks `sleep_skip_bit_identity.rs`) | a schedule of writes on frozen bodies, plus T-SL11..T-SL15's events. Equal on every step: `latch_fingerprint`, every awake bit, all six counters, the full pose bytes. | M-SL4..M-SL13 (mode leaks) |
| T-SL9 | `sleep_velocity_zero.rs::the_settle_is_bit_identical_across_worker_counts` | W ∈ {1, 2, 4, 8}: identical poses, latches and counters through a freeze, a hold, a write-wake, a seal break and a re-freeze | W leaks |
| **T-SL11** | `sleep_seal.rs::a_support_despawned_on_the_first_step_after_a_settle_wakes_the_pile` | detect the settle step (`rows_put_to_sleep` rises), despawn the supporting box before the next step: the pile is awake at the next step and falls; `seal_breaks` rose | M-SL9a (seal ignores the rows-stable test) |
| **T-SL12** | `…::a_static_support_moved_on_the_first_step_after_a_settle_wakes_the_pile` | move the static under the pile down by 0.5 m on that step: wakes and falls | M-SL9b (stash omits non-member rows) |
| **T-SL13** | `…::a_member_teleported_on_the_first_step_after_a_settle_wakes_its_island` | lift the top box by 1 m on that step: its contacts vanish, the count differs, the island wakes and the box falls | M-SL9c (seal ignores members' poses) |
| **T-SL14** | `…::a_contact_config_change_on_the_first_step_after_a_settle_is_not_rekeyed` | set `speculative_distance = 0` on that step on a pile with speculative pairs: the count differs; `contact_wakes` rose, `settle_rekeys` did not | M-SL9d (epoch omitted) |
| **T-SL15** | `…::a_frozen_island_written_on_another_islands_first_post_settle_step_still_wakes` | island B frozen long ago; island A settles at N; at N+1 lift a box of B by 1 m: B wakes (its rows are `REST_FROZEN`, so the trunk rule applies) | M-SL10 (re-key applied to FROZEN rows) |
| **T-SL16** | `…::an_sdf_edit_under_a_settled_box_on_the_first_step_after_a_settle_wakes_it` (`add_physics_sdf`) | an edit that removes the field under the box on that step: wakes | M-SL9e (SDF omitted from the epoch) |
| **T-SL17** | `resources_tests.rs::a_seal_is_used_only_on_the_step_right_after_its_settle` (direct drive with the hooks) | settle, then two `begin_step`s, the second with a count change: the second uses the trunk rule | M-SL13 (stale seal) |
| T-SL10 | `resources_tests.rs::a_count_change_on_the_first_step_after_a_settle_is_rekeyed_when_sealed` (direct drive) | `end_step` latches {0,1,2}, then `settle` with bodies, then `arm_seal_for_test`, then `begin_step` with the same bodies and a count one less: frozen, latches kept, `settle_rekeys == 3`, `contact_wakes == 0`. The same with one stash row's position changed by one ulp: trunk rule, `seal_breaks == 1`. | M-SL9 (seal always false), M-SL11 |

### 9.3 Mutations (debug build; each must turn the named test red; the developer writes them, the tester runs them)

| id | mutation | red in |
|---|---|---|
| M-SL1 | the `settle` call deleted | T-SL1 |
| M-SL2 | zero every row latched by this `end_step`, whatever its island | T-SL7 |
| M-SL3 | fold on `asleep` instead of `rest` | T-SL2 |
| M-SL4 [1b] | write test on `asleep` rows instead of `rest != NONE` rows | T-SL7 (the latch restarts) |
| M-SL5 [1b] | write test ignores `angular_velocity` | T-SL3, angular variant |
| M-SL6 | the contact-count unlatch leaves `rest` set | `support_loss_wakes_sleepers.rs` |
| M-SL7 | `settle` before `write_back_awake` | T-SL1 |
| M-SL8 [1b] | the write test reads the post-solve snapshot instead of the gathered one | T-SL3 |
| M-SL9 | `sealed = false` always (the rev-1 behaviour) | T-SL10, T-SL10p |
| M-SL9a–e | the seal ignores rows-stable / non-member rows / members' poses / the config epoch / the SDF part | T-SL11 / T-SL12 / T-SL13 / T-SL14 / T-SL16 |
| M-SL10 | re-key on any latched row with a key difference when sealed, not only `REST_SETTLED` | T-SL15 |
| M-SL11 | stash taken **before** the zero (v_N in the stash) | T-SL10 (every seal breaks on the members' velocity), T-SL10p |
| M-SL12 | G2/G7 count `contact_wakes` only | a generator variant that injects a lateral speculative pair at d + 0.01 mm. It must count one event. |
| M-SL13 | `seal_live` not cleared by `begin_step` | T-SL17 |
| M-B1 | B1's own list on the re-expressed exactness test | must stay red as before (`frozen_island_warm_start.rs:44-56`) |
| M-T1 | M3b + M-A on the re-expressed T1 and H-03 | must be red |

### 9.4 Lane gates (tester; `--workspace --all-targets --no-fail-fast`, debug and release, plus the device-free ignored leg)

- **G-SL1.** The full workspace run. The red set must equal §8.2's predicted set, each red for the predicted reason. Any other red is a STOP.
- **G-SL2.** clippy `-D warnings`.
- **G-SL3.** Miri, scoped to the new unit tests in `resources_tests.rs` (no pool), with a stated budget (memory: "tester ran Miri on the whole kernel").
- **G-SL4, runner lane vs parent, same toolchain.**
  - **Sleeping-off rows must be byte-identical** in pose hash, CSV non-timing columns and SUMMARY (minus timing keys and minus SL's added keys):
    - J500 W1/W8, J500off, R1100, S16 `--cfg a`;
    - both overlap-only rows;
    - `--dyn kick|shoot|slide --sleeping off`.
  - **Sleeping-on rows are expected to differ.** The gate prints:
    - a positions-and-rotations-only hash (fields 0-2 and 6-9 of the 13-field pose record);
    - `first_frozen_step`, `settle_rekeys`, `seal_breaks` and `frozen_v_max`.
  - `frozen_v_max` must be 0, and `seal_breaks` must be 0 on J-T.
- **G-SL5.** `runner_pins.sh`, `pose_gates_d0.sh` and `pose_gates_v2.sh`, with their sleeping-on rows re-pinned by §8.2's rule. Both negatives exit 4.

---

## 10. Screens (untimed)

### 10.0 S-SL0: Jolt 5.6's sleeping-on freeze, untimed (resolves critique W1)

- **Binary.** Exactly 9b's: `D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe`, sha256 `918fd2b70ecc634cc0574cb676c56a3c929a46a51173cf9aa76fdd628e2ef0ad` (`D:/tmp/phys-orch/win9b/PREP.md:102`). It is the same binary as P0's v5.6.0 Distribution (`p0b/window_report.md:395`), and it carries parity patch v1 with `-allow_sleep` and the per-frame receipt (`docs/measurements/2026-09-30-physics-window9a/README.md:59`, `p0/build_report.md:220-221`).
- **Arguments.** The `JOLT-SLP` row's (`tools/physics_parity/driver.py:145`: the timed Jolt arguments + `-allow_sleep`, 1,000 steps), plus the receipt flag for the per-frame CSV (Frame, Manifolds, Points, Active Bodies, Top Y).
- **Runs.** W1 and W8, three processes each.
- **Read.** Only the Active Bodies and Top Y columns; no ms value is opened. So it needs no quiet window.
- **Report:**
  - the first frame with 0 active bodies;
  - A_eff = Σ over frames [100,500) of active / 1,240 (Jolt's awake-equivalent steps in the window);
  - whether it differs between W1 and W8;
  - Top Y at frame 500 (the pile holds).
- **Use.** It replaces A_jolt in §0.2 and §11. Jolt's sleeping-on cost per awake-equivalent step stays an input that only 9c measures.

### 10.1 S-SL1: J-T sleeping arm, blocking for SL-1 (C1)

**Design.** `sweep.py`'s design (`cs/proto/sleep_sweep/PREREG.md`): 16 one-ulp seeds plus the unperturbed run, × reuse on/off = 34 runs, 500 steps, V2 defaults, sleeping on.
- **Counts** are W-invariant by the identity gates, so the screen runs at W1, plus one W8 replay per arm as an identity spot check.
- **Arms:**
  - **P**: the parent trunk executable, carrying SR-B, S5 and SL-2a;
  - **L**: SL-1;
  - **L+b**: SL-1 + SL-1b, if the owner says yes to Q1;
  - **D** (diagnostic, not a bar): SL-1 with the seal compiled out, from a scratch patch outside the repository that sets `sealed = false`. It measures C1's raw size.

**Receipts per run:**
- first frozen step, contact wakes, rises (with the steps they fire on), `settle_rekeys`, `seal_breaks`;
- W_sleep: exact row-iterations over [0,500) from the per-step slot histogram (`SWEEP.md:189-191`);
- `frozen_v_max`, `rows_put_to_sleep`, `write_wakes` (0 on J-T);
- holds (0 boxes > 0.5 m) and drift;
- the C3b motion protocol for any rise (`PREREG-C3b.md`);
- the positions-and-rotations hash.

**Bars, pre-registered before the first run. All are blocking for SL-1.**
- **(i)** 0 visible re-wakes (C3b bands).
- **(ii)** **Per seed:** W_sleep(L) ≤ W_sleep(P) in **both** reuse arms. The mean ratio is ≤ 1.00 with reuse on (ruling 24(ii)'s letter) and reported with reuse off.
- **(iii)** G2/G7 re-expressed events = 0. Generator 0/30 and 0/56 counting re-keys (S-SL2).
- **(iv)** Holds 34/34.
- **(v)** `seal_breaks == 0` on every run. J-T has no external input, so a break means the seal compares a field the solve or the apply changes.
- **(vi)** `frozen_v_max == 0`.
- **(vii)** On every run where P does not rise, L's positions-and-rotations hash equals P's. The settle changes only velocity bytes, and the seal only re-keys.
- **(viii)** On every run, L's first frozen step equals P's **first attempt**: P's first frozen step if P does not rise, else P's first rise step.

**Predicted:**
- (ii) holds with equality on every P run that does not rise: all 17 with reuse on (`SWEEP.md:15`) and 11 with reuse off.
- On P's six rising runs, L is cheaper by 60 awake steps each.
- Arm D shows rises(D) ≥ rises(P), paired by seed: C1's size, reported to the orchestrator.

**Route if a bar fails (written now, before the first run). No branch lands SL-1 with a failed bar.**
1. **(v) fails** (`seal_breaks > 0` on J-T). The seal compares a field the engine itself changes.
   - STOP. The developer finds the field.
   - It may be excluded only if it is **derived** from compared fields, as `inv_inertia` and `bp_margin` are. A field a user can write stays compared.
   - Re-run S-SL1 from scratch.
2. **(vii) or (viii) fails while (v) holds.** L's trajectory diverged from P's before P's first attempt, e.g. a small island settled earlier and later merged, or a defect. STOP; diagnose run by run.
3. **(ii) fails while (v), (vii) and (viii) hold.** By construction this means a frozen L pile later woke with no external input: a count change at fixed poses, the defect class of `resources.rs:4392-4396`. STOP; diagnose as a defect, not a value.
4. **(iii) fails.** The generator's count changes at attempts now include the settle's shift. Ruling 24(iii) applies: to the owner with the numbers and arm D's, no re-pin.
5. **(i) or (iv) fails.** STOP.

If SL-1 cannot pass, the orchestrator receives the per-run receipts and arm D. Sleeping stays opt-in, and the stored velocity stays a recorded defect, until the cause is fixed.

### 10.2 S-SL2: the generator scene

- `flicker_redraw_distribution`: h4 with 30 draws, h5 with 56 draws.
- Events counted as `contact_wakes` **or** `settle_rekeys` rising. They must stay 0, and the freeze step must stay 61 (`SWEEP.md:289`).
- Arm D reported beside.

### 10.3 S-SL3: dynamic scenes, sleeping on

**The parent.** P is the trunk executable at SL-2a's commit, so the lift is native and P is not a patched build (critique open question 3). SL-2a's own receipt: every `--sleeping off` dyn row is byte-identical to the pre-SL-2a executable, on pose hash, CSV non-timing columns and SUMMARY.

**Runs.** S-LAND [0,100), S-KICK, S-SHOOT, S-SLIDE with `--sleeping on`, at W1 (counts) plus a W8 replay, for P, L (and L+b).

**Per step:** awake rows, frozen islands, `rows_put_to_sleep`, `contact_wakes`, `write_wakes`, `settle_rekeys`, `seal_breaks`, `frozen_v_max` (must be 0), solved slots.

**Per kick event:**
- the step on which each kicked box's island is first solved;
- whether that island's count changed on the event step;
- whether it woke through `write_wakes` or `contact_wakes`.

This splits the claim of §3.5 (O1).

**Bars:**
- **Sanity:** `dyn_sanity.py`'s B1-B3 (`dyn-scenes/cut.md:103-118`). A failure for ours is a STOP (ruling 22 Q7).
- **(ii-dyn):** Σ solved slots over each scene's window, L ≤ P per scene. **Blocking for SL-3**, reported for SL-1.
  - A failure is read through `seal_breaks`. Breaks on spawn steps (S-SHOOT) or on kicked members are expected, and on those steps the shifted draw returns.
  - The excess is priced, and the orchestrator decides between landing SL-3 as is and opening PC-SL-6 (a perf fork, ruling 24).

**Expected (inference):**
- S-KICK's first event (step 200) lands on a pile frozen since about step 144. The pile then stays awake, because a kick every 25 steps is shorter than any engine's dwell.
- S-SHOOT's first projectile merges with the frozen pile and wakes it.
- S-SLIDE freezes only at its end.

### 10.4 S-SL4: the values (SL-V), one stage (critique open question 2)

**When.** After the combined structural screen has picked the contact set (CS × FM × CR, ruling 25(c)), on that contact set and on IB's shipped preset.

**Cells.** `sleep_frames` ∈ {60, 45, 30} × `sleep_threshold` ∈ {1e-4, 4e-4, 9e-4}.
- 4e-4 is |v| ≤ 2 cm/s.
- 9e-4 is 3 cm/s, Jolt's point speed in our units (`resources.rs:511-522`).

**Each cell runs:**
- S-SL1's 34 runs (P = the 60 / 1e-4 cell);
- S-SL2;
- S-SL3's four scenes;
- the G2/G7 tests;
- A7-R1's bound (< 10 mm, an acceptance bound that is never tightened, `sleep_settles_box_piles.rs:351-352`);
- A7-R2's freeze bar.

**Rule.**
- Keep the cells that pass S-SL1 (i)-(viii) against the 60 / 1e-4 cell and (ii-dyn) on every scene.
- Rank them by mean W_sleep with reuse on.
- A G2/G7 move above 0 goes to the owner.
- If none passes, keep 60 / 1e-4.

**Why one stage suffices (inference, checked here).** Under the seal, a candidate's freeze on a quiet scene is its below-threshold onset + `sleep_frames`, with no rises. Changing the dwell shifts every candidate by the same steps and cannot reorder CS's ranking. The cell table reports each candidate's onset, so the check is direct.

**Prior.** 30 / 1e-4 (0.5 s at 60 Hz, as Jolt, Rapier and Box2D). It moves the first attempt from about step 144 to about 114 (V2's onset ≈ 84 = 144 − 60, `SWEEP.md:35`).
- Rev 1's R8 (an earlier attempt rises more) is structurally removed on sealed steps.
- What remains to screen is **fidelity at an earlier freeze**: A7-R1's drift bound and the 34/34 hold.

**The decision** is a perf fork the orchestrator decides (ruling 24).

### 10.5 Fidelity screen

- **F1.** The 26-run gate (sleeping off): byte-identical anchors (`v2_speculative_fidelity.rs:112-114`) under SL-2a, SL-1, SL-1b and SL-3.
- **F2.** The 88-run ensemble (sleeping off): ≥ 87/88, no worse than the trunk control (ruling 23b). Unchanged by construction; re-run once at SL-V.
- **F3.** The red control (`v2_speculative_fidelity.rs:438+`) still fails.
- **F4. Sleeping-on holds:**
  - `j_t_stands_with_sleeping_on` (2 runs, `:423-436`);
  - S-SL1's 34/34;
  - for SL-3, the 26-run set re-run with `--sleeping on`, as a **flip bar**: the shipped default must hold the pile (§18 Q6).
- **F5.** The lever's own tests (§9) and L10's sleeping suites: `sleep_skip_bit_identity`, `frozen_island_warm_start`, `support_loss_wakes_sleepers`, `sdf_sleep_settles_and_wakes`, `sleeping_pipeline_o8`, `sleep_settles_box_piles` (including its release-only tests).

---

## 11. Expected saving per W (cross-build, cross-window arithmetic; not a claim)

### 11.1 The model

The J-T [100,500) sleeping-on mean is a bookkeeping identity, not a validated model. 8b's agreement only shows that its terms were read consistently (critique W1):

`mean = (A·(C_W + b) + (400 − A)·F_W + T_W) / 400`

| symbol | meaning | value | source / caveat |
|---|---|---|---|
| A | awake steps in [100,500) | 44 (freeze at 144) | V2 reuse-on median (`SWEEP.md:15`). Under SL-1 every J-T run freezes at the parent's first attempt (§10.1 viii): 142–157 (`SWEEP.md:35`). |
| C_W | sleeping-off step | 8.228 / 5.473 / 3.774 / 2.976 / 3.274 ms (W 1/2/4/8/16) | 9b, measured |
| b | awake bookkeeping | 0.020 ms (W1/2/4), 0.021 (W8/16) | 8b armed spans 19.60 / 21.16 µs (`jsondm1.md:141-142`); pre-V2, W1 and W8 only |
| F_W | steady frozen floor | low: 0.1085 / 0.110 / 0.112 / 0.1177 / 0.1177. High: low + 0.024. | Low is 8b's steady floor (`jsondm1.md:145`); W2/W4 are interpolated and W16 is taken as W8's (serial pieces). The high end assumes V2 doubles the manifold-dependent pieces (solve 12.7 + graph 8.7 µs at W1, `jsondm1.md:147-149`). |
| T_W | move-in transition, total per freeze | W1 4.3–8.6 ms; W2/W4 3–8; W8/W16 2–6 | W1 is 8b's steps 274-277 minus the floor (`jsondm1.md:111-115`), up to 2× for V2's set. W2–W16 are inference. |
| Jolt | `A_J·C_J + (400 − A_J)·tail`, over 400 | C_J = 9b's Jolt 5.6 means; tail 2.17 / 2.29 µs | **A_J and the tail are v5.3.0 readings**. A_J = 27 from frame 127. This is an upper bound: Jolt's partly-asleep steps cost less than C_J. |

### 11.2 Results

| W | sleeping off, 9b (ms) | ours sleeping on, SL-1, 60 frames | ours/Jolt at A_J = 27 | ours/Jolt over A_J ∈ [14, 40] | ours, 30 frames (A = 14) | ours/Jolt at 30 frames, A_J = 27 | … over A_J ∈ [14, 40] |
|---|---|---|---|---|---|---|---|
| 1 | 8.228 | 1.01–1.05 | 1.59–1.64 | 1.08–3.15 | 0.40–0.44 | 0.63–0.69 | 0.43–1.32 |
| 2 | 5.473 | 0.71–0.74 | 1.86–1.95 | 1.26–3.73 | 0.31–0.34 | 0.80–0.89 | 0.54–1.72 |
| 4 | 3.774 | 0.52–0.56 | 2.13–2.27 | 1.44–4.34 | 0.25–0.28 | 1.01–1.15 | 0.68–2.21 |
| 8 | 2.976 | 0.44–0.47 | 2.45–2.62 | 1.66–5.00 | 0.22–0.26 | 1.25–1.43 | 0.84–2.73 |
| 16 | 3.274 | 0.47–0.50 | 2.88–3.07 | 1.95–4.74 | 0.23–0.27 | 1.43–1.63 | 0.97–2.51 |

### 11.3 Per added rise

Per rise per run on J-T [100,500): +60·(C_W + b − F_W)/400 = **+1.22 / +0.81 / +0.55 / +0.43 / +0.48 ms** at W 1/2/4/8/16.
- Without the seal (arm D), each rise C1 adds costs that much. One rise doubles the W8 estimate.
- With the seal, J-T rises are 0 on every run by construction (§10.1 v, viii).

### 11.4 What each rung buys

- **SL-1 (+1b).** Sleeping off: no change; 9b's table stands. Sleeping on: W_sleep ≤ the parent's per seed on J-T.
  - Equal with reuse on, where the trunk never rose.
  - With reuse off, each of the trunk's six rising runs is 60 awake steps cheaper. Mean A ≈ 65 → ≈ 44, which is about −0.43 ms at W1 on the mean (reuse off is not the shipped default).
  - Its main value is correctness and Jolt/Rapier semantics.
- **SL-V at 30 frames** (if it passes). It halves the sleeping-on J-T mean again. Whether that puts W1-W2 ahead of Jolt 5.6 depends on S-SL0 (A_J ≥ ~19).
- **SL-3 (the flip).** It changes what a game gets by default: the shipped J-T resting pile drops 85–88 %.
  - Every awake step pays b ≈ 20 µs, i.e. +0.24 / 0.37 / 0.53 / 0.67 / 0.61 % of 9b's step at W 1/2/4/8/16.
  - The share grows as SR, S5 and IB shrink the step, which is why the flip waits for 9c (§7).
- **What remains at W ≥ 4 is the floor.** At W8 with 30 frames, F is 46–55 of the 89–103 ms total over 400 steps. With a Jolt-like floor, W8 would sit at about 0.11 ms (0.6× Jolt at A_J = 27). That is PC-SL-1 / Q5.

---

## 12. Waking rules a default-on world needs, and how each is wired

| trigger | ours after SL | wiring | Jolt 5.6 / Rapier 0.36 | status |
|---|---|---|---|---|
| contact with an awake body, including a new V2 speculative manifold | woken the same step (wake-on-merge) | the graph unions every stream manifold (`resources.rs:3571-3583`); fold (`:4516-4520`); under Sets the D2 scan restores a held island whose stream pair reaches a non-resting row (`sleep_sets.rs:1256`) | Jolt: a new contact constraint activates (per research); Rapier: contact start wakes | wired |
| new body (S-SHOOT projectile) | awake (`SleepLatch::FRESH`), merges into what it hits; also unseals that step's first-post-settle comparison | gather `is_added` (`systems.rs:294`); latch permute (`resources.rs:4296-4315`); the seal's rows-stable test | `CreateAndAddBody(.., Activate)`; Rapier active on insert | wired |
| user velocity or impulse write (S-KICK) | **SL-1b: woken the next step, never erased.** Without SL-1b: woken on the event step if the write changes the count, else one step late; a sub-threshold write is kept while frozen | gather → `begin_step_upto` write test on resting rows (§3.5) | `BodyInterface.cpp:595-638`; `rigid_body.rs:1016-1030` | SL-1b (Q1) |
| soft→rigid reaction | SL-1b: woken; a resting soft load cycles (§3.5) | same | — | SL-1b + PC-SL-7 |
| user transform write | wakes iff the island's manifold count changes. On the first step after a settle, any write to a watched row unseals, so the same rule holds there too. A count-preserving teleport leaves the body frozen at the new pose; under Sets R3 restores its record. | count key (`resources.rs:4508-4514`); seal (§3.4) | Jolt `SetPosition(.., EActivation)`; Rapier `set_translation(.., wake_up)`; Box2D `SetTransform` does not wake | recorded gap (Q8) |
| supporting body despawned | woken by the count change, except a step that adds as many manifolds as it removes. On the first step after a settle the despawn unseals (rows not stable), so this holds there too. | count key; Sets D1a (`sleep_sets.rs:1240-1241`); `tests/support_loss_wakes_sleepers.rs`; T-SL11 | Jolt does not wake (`Docs/Architecture.md:421`, per research); Rapier wakes | wired (better than Jolt) |
| kinematic body moving against a sleeper | not woken: kinematic bodies are no island member and kinematic motion is unbuilt (`components.rs:64-74`, per research). A write to one on the first post-settle step unseals. | — | Jolt `MoveKinematic` activates; Rapier kinematic bodies sleep only at zero velocity (`rigid_body_components.rs:1489-1493`) | recorded gap; only boyko_physics tests use `Kinematic` (grep) |
| sensors | sensor pairs are computed every step (`sleep_sets.rs:151-158`); a dynamic sensor has no solver manifold, hence no island, hence is always awake | `sensor_pair` | Jolt: sensors never sleep (`Body.cpp:148`) | parity |
| gravity change (S-SLIDE) | not a wake and not an epoch input (`sleep_sets.rs:289-292`); S-SLIDE sets gravity before step 0 | — | Jolt `SetGravity` only stores it | parity |
| narrowphase config or SDF edit | wakes iff it changes the count; on the first step after a settle it unseals (T-SL14, T-SL16) | `SealEpoch` | — | wired |
| explicit wake | `IslandSleep::wake_all` (`resources.rs:4168-4170`) | — | per body in both | no per-entity API (Q8) |
| static ↔ dynamic flip at runtime | caller contract: pair it with `wake_all` (`resources.rs:4228-4238`); not reachable today | — | — | contract |

---

## 13. The cost of sleep bookkeeping when nothing sleeps

**Sleeping off: zero for every SL piece.**
- The sleep-skip prologue returns early (`sleep_sets.rs:944-948`).
- The solve takes `sleep = None` (`systems.rs:2116-2123`).
- The baseline is not kept (`systems.rs:284`).

**Sleeping on, nothing frozen: priced, not zero.** Every step runs these serial O(rows) pieces:
- the classify (`sleep_sets.rs:1151-1219`);
- `begin_step`, two passes (`resources.rs:4488-4532`);
- the capture scan (`colored.rs:5251-5261`);
- `write_back_awake`'s test (`:4977`);
- `end_step`, two passes (`resources.rs:4606-4647`);
- `build_with_held(.., true)` (`systems.rs:2035-2042`).

8b measured them at 19.60 µs (W1) and 21.16 µs (W8) per step, pre-V2.

**SL's increment:**
- **Every step:** one byte update per in-island row in existing loops, and one O(islands) scan. ≈ 0, not separately resolvable.
- **[SL-1b] frozen steps only:** the write test, 0.5–2 µs per frozen step on J-T (inference, §5, O3). Against the frozen floor that is +0.5–2 %. Against Jolt's whole asleep tail (2.2 µs) it is the same order.
  - Fusing it into the gather, which already writes every row, or into R3's pass is recorded as PC-SL-8. It would serve a future floor lever (PC-SL-1).
- **Once per freeze attempt:** the stash and the seal check, about 20–60 µs each (inference). Amortised over [100,500) that is < 0.3 µs per step.

**Jolt's equivalent** is fused into its per-island bounds job on the last collision step, and runs in parallel (`PhysicsSystem.cpp:2511-2563`). Ours would be PC-SL-3.

**Window 9c measures the awake cost in-block at every W**, through the S-LAND, S-KICK and S-SHOOT on/off pairs (§7, §15).

---

## 14. Tests and pins on the default world that assume sleeping off (for SL-3)

Source: L10's C1b list (`04-DESIGN-REV2.md:578-582`) plus `research_tree.md` §9. The tester's `--workspace --no-fail-fast` red set at the flip decides; every red gets the C1b treatment (§7).

**Cannot move (horizon < 61 steps):**
- `default_world_colored_simd.rs` (10 steps);
- `default_world_worker_invariance.rs` (60);
- `default_world_pyramid_determinism.rs` debug arm (60);
- `scene_sync_s5.rs`, `soft_rigid_apply_row_alignment.rs`, `narrowphase_carry.rs` (60).

**Do not read `cfg.sleeping`.** They use the reference or no-op solver, or drive `solve_colored` directly with `sleep = None` (`colored.rs:5010-5019`):
- `softstep.rs`, `simd_o1.rs`, `integrate_determinism.rs`, `broadphase_select_p3.rs`;
- boyko_render's `bundles_s6_integration.rs` and `interp_pair_b1.rs`;
- `colored_rigid_scratch_*`, `large_island_gate_p2.rs`, `ke16_*`, `colored_parallel_alloc_o6.rs`, `colored_simd_parallel_o7.rs`.

**Set the flag explicitly already:**
- `apply_row_alignment.rs:344`, `alloc_frame_attribution.rs:2005`, `alloc_frame_census.rs:2160/2597/2658`, `broadphase_tree_scenes.rs:313`;
- `frozen_island_warm_start.rs:290`, `row_identity_remap.rs:363`, `row_keyed_state_defect_a.rs:235`, `sdf_sleep_settles_and_wakes.rs:174`;
- `sleep_settles_box_piles.rs:682/1906/2166`, `sleep_skip_bit_identity.rs:674`, `sleeping_pipeline_o8.rs:116`, `support_loss_wakes_sleepers.rs:250`;
- `v2_speculative_contacts.rs:395`, `v2_speculative_fidelity.rs:326`, `default_world_sleep_worker_invariance.rs:186`.

**Horizon ≥ 61 steps and no flag. Predicted red, or green with a weakened premise:**

| test | horizon | prediction | treatment |
|---|---|---|---|
| `contact_reuse.rs` | 120 steps | RED: it asserts 100 % reuse hits on a resting stack, and Sets skips held pairs | `sleeping: false` |
| `v2_speculative_d0_identity.rs` | 120 steps, 12 pins | a scene at rest for 60+ steps moves | `sleeping: false` |
| `narrowphase_parallel_equivalence.rs` | 160 / 600 steps | green, but weakened (it compares skipped pairs after the freeze) | `sleeping: false` |
| `default_world_pyramid_determinism.rs` release arm | 120 steps | unchanged at 60 frames (J-T freezes ≥ 142); RED under SL-V at 30 frames (freeze ≈ 114 < 120) | `sleeping: false`; doc (`:8`) updated in any case |

**Other tests whose subject is not sleeping** (C1b's `sleeping: false` if red):
- `bodytype_determinism_golden.rs` (90 steps, 1 pin);
- `colored_acceptance_o5.rs`, `colored_acceptance_simd_o7.rs`, `constraint_graph_o4_world.rs`, `colored_solve_zero_alloc_o5.rs`;
- `contact_reuse_bounds.rs`, `bodytype_bits_toggle.rs`, `sdf_collision.rs`, `body_set_selection.rs`;
- `soft_body_sp1.rs` / `soft_body_sp2.rs`. With SL-1b, a soft→rigid reaction now wakes a sleeper, and a resting load cycles.

**Other crates.**
- Playground and `_hud_probe` use `PhysicsPlugin::new()` at 120 Hz, so their props sleep after 0.5 s. No pin, but visible behaviour changes.
- Before the flip, the lane greps every `PhysicsPlugin::new` and `add_physics_colored_solve` caller in boyko_app, boyko_demo, boyko_render and `src/`, and lists any with a physics golden.

**Docs at the flip:**
- `resources.rs:460-501`, `:731-733`;
- `plugin.rs:375-377` (strike the SDF warning), `:434-436` (rewritten with SL-1b), `:663-666`, `:672-675`;
- `docs/SYSTEMS.md`, `docs/FEATURE_MAP.md`, the MEASUREMENT-QUEUE R row;
- the book page, by the doc-writer.

---

## 15. Window 9c protocol: every scene with sleeping off AND on

**Ours.**
- Every row carries `--sleeping off` or `--sleeping on`; there is no flagless row.
- "On" means the values SL-V shipped, `sleep_skip` Sets, and SL-1 (+1b).
- Receipts per row:
  - the CSV `awake` column;
  - SUMMARY `first_frozen_step`, `contact_wakes`, `rows_put_to_sleep`, `write_wakes`, `settle_rekeys`, `seal_breaks`;
  - `frozen_v_max == 0`. A non-zero value voids the row.

**Jolt 5.6.**
- "On" means `-allow_sleep`: Jolt's shipped 0.5 s / 0.03 m/s with `mAllowSleeping` (`PhysicsSettings.h:95`, `:102`, `:123`). The binary is the S-SL0 one.
- SL-2b lifts the dyn patch's refusal of `-dyn` with `-allow_sleep` (`dyn-scenes/cut.md:467`).
- The 9c driver accepts `allow_sleep=1` in the banner on "on" rows.
- Kicks go through `AddLinearVelocity`, which activates; projectiles through `CreateAndAddBody(Activate)`.
- S-SL0's per-frame active count is the reference for 9c's Jolt "on" rows' awake fraction.

**Rapier 0.36.**
- "On" means the library defaults (`rigid_body_components.rs:1362-1376`), in the `dyn/` sibling crate.
- Required changes:
  - `can_sleep(true)`;
  - `set_linvel(.., true)` on kicks;
  - the per-step awake count printed instead of the "every body awake" void (`dyn-scenes/cut.md:705`).
- These rows are outside FROZEN-9b, so ruling 13's label "against the RP-F set" applies to Rapier's "on" rows.

**Defaults, not equalised criteria:**

| engine | dwell | threshold |
|---|---|---|
| ours | 60 frames (pending SL-V) | speed² 1e-4 |
| Jolt | 0.5 s | 3 cm/s point |
| Rapier | 0.5 s | 5 cm/s drift |

**Windows and metrics.** S-LAND [0,100), S-KICK and S-SHOOT [200,800), S-SLIDE [0,500), J-T [100,500). Beside each sleeping-on window mean, the record carries:
- the per-step awake count of every engine;
- each engine's all-asleep step;
- the asleep tail compared alone, never pooled (`docs/MEASUREMENT-QUEUE.md:939-941`).

**The flip bar** (§7) is pre-registered here.

**Fidelity bars, unchanged** (ruling 25): the 26-run gate (sleeping off), the 88-run ensemble, the red control, and each scene's sanity bar in both arms.

---

## 16. Interplay and order

**SR-B** (in flight, `D:/wt/ui`).
- SL-1 rebases on it.
- The order of the serial tail is unchanged (`sr-b/cut.md:234`), and so is the capture loop (`:157`).
- Both edit `frozen_island_warm_start.rs`, in disjoint hunks: SR-B its dispatch assertion at `:1286-1299` (`cut.md:79`, `:467`); SL `run_scenario` and the B1-exact test.
- SR-B's SR-JD row is flagless (`:545`) and gets `--sleeping off` in SL-2a.

**S5.**
- Its locks (ruling 19 Q1) do not overlap SL's files.
- S5's C4 lands after SR-B (ruling 19 Q3). SL-1 follows S5 in ruling 20's order.

**dyn-scenes.**
- SL-2a lifts our runner's `--dyn` + `--sleeping on` refusal (`cut.md:344`) and puts explicit `--sleeping off` on the existing rows.
- SL-2b lifts Jolt's (`:467`) and adds Rapier's sleeping arm.

**IB.**
- IB changes the iteration budget, so it moves every trajectory, sleeping-on freeze steps included.
- With SL-1 landed first, IB re-pins the sleeping-on anchors once, by IB's rule.
- IB moves the runner into a package (`ib_rev4.md:25`, per research); SL's runner hunks rebase onto whichever lands first.

**CS.**
- Ruling 24's criterion compares CS candidates' W_sleep with V2's sleeping arm.
- With SL-1 in the trunk before Screen 2, the control and every candidate carry the settle and the seal. On J-T the seal removes every candidate's attempt-step rises (B+T 15/32, A+T 14/32, T 4/32 seeded, `SWEEP.md:187`).
- So ruling 24's sleeping arm then ranks candidates by their onset alone. **This changes the inputs of ruling 24's B+T-vs-Z choice** (Z's 10/40 and 27/83 generator events become re-keys under the re-expressed count, still events). The combined screen must run with SL-1 in.
- CS changes the velocity-dependent margins (`cs/design_r2.md:41-44`, per research). The seal does not depend on them.
- CS adds `contact_wakes`, `contact_wake_rises` and `first_frozen_step` to the SUMMARY (`design_r2.md:513`); SL's fields go beside them.

**FM.** Twist friction changes ω at rest (`fm/research_tree.md:253-255`), and so our sleep metric; S-SL4 runs on the chosen FM setting. FM's carried records ride the frozen-manifold carry, which SL does not touch.

**CR.** No interaction. Frozen manifolds are not laid out (`cr/research_tree.md:54`), CR leaves the sleep criterion untouched (`:224`), and SL changes no colouring input.

**Proposed order** (ruling 20's spine; the synthesis of ruling 25(c) decides):
1. SR-B.
2. S5.
3. SL-2a (flags and lifts, no simulation change).
4. SL-1 (+1b per Q1).
5. IB.
6. The combined screen: CS × FM × CR, with SL-1 in.
7. CS (+FM/CR).
8. SL-V (one stage).
9. SL-2b.
10. 9c.
11. SL-3, decided by 9c's on/off pairs.
12. Records.

---

## 17. Risks

| # | risk | mitigation |
|---|---|---|
| R1 | **The settle shifts the first count after a settle toward removal, at every attempt (C1).** Rev 1 wrongly called it "as today". | The seal makes the shift unable to wake its own island on sealed steps (§3.4). S-SL1 (ii), (v), (vii), (viii) are blocking. Arm D measures the raw shift. |
| R2 | **On unsealed first steps the shifted draw returns.** Unsealed means a spawn, despawn, config change or SDF edit anywhere, or a write to a watched row. | Priced by S-SL3's `seal_breaks` and (ii-dyn). PC-SL-6 narrows the watch. On J-T there are none. |
| R3 | **The seal hides an external change.** | The watched set covers every row a settling island's pairs can end on, plus the epoch. Proof in §3.4. T-SL11..T-SL16 kill one mutation per input. Internal caches at fixed poses are a defect class with their own gates (`resources.rs:4392-4396`). |
| R4 | **The seal leaks:** the solve or the apply changes a compared field, and every attempt breaks. | S-SL1 (v) `seal_breaks == 0` on J-T is a STOP. Excluding a field needs a "derived from compared fields" argument (§10.1 route 1). |
| R5 | **G2/G7 lose their power.** | Re-expressed to count re-keys. M-SL12 must be red. |
| R6 | **The seal also removes the trunk's own attempt-step rises.** That moves the freeze step on runs that rose, and the pins with it. | By rule (§8.2). A perf fork the orchestrator decides (Q3). Ruling 24 found those rises invisible and cost-only. |
| R7 | **SL-1b blinds T1/H-03's mutant.** | Re-expression at spawn v = 0; M3b + M-A re-run red. |
| R8 | **B1-exact's re-expression loses power.** | B1's mutation list re-run; "×0.9999" must stay red; the twin is re-zeroed on every settle (O4). |
| R9 | **Off ≢ Sets under writes or seals.** | T-SL8 at W1 and W8; the fingerprint includes `rest`; every seal input is mode-free (§6). |
| R10 | **The flip silently changes flagless runner rows.** | Explicit flags in SL-2a, before SL-1. G-SL4 lists every pinned row by flag. |
| R11 | **The awake cost of sleeping on is serial and unresolved on V2; its share grows with SR, S5 and IB.** | SL-3 waits for 9c's on/off pairs (§7); PC-SL-3. |
| R12 | **The frozen floor dominates sleeping-on rows at W ≥ 4** (≈ 50× Jolt, pre-V2). | Out of SL (N2); Q5. |
| R13 | **A resting soft load keeps a rigid island cycling** under SL-1b. | Stated; T-SL6b; PC-SL-7. |
| R14 | **`sleep_frames` counts frames.** 60 frames is 1.0 s at 60 Hz, 0.94 s at the engine's 64 Hz default (`boyko_ecs/.../fixed_time.rs:78`), 0.5 s at 120 Hz. | Doc fix in SL-1; the unit stays frames (Q9). |
| R15 | **The stash grows with the number of static rows** (a large level). | Copied and compared once per attempt only. PC-SL-6 restricts it to the island's partners. |
| R16 | **A test that latches through `end_step` and arms the seal by hand behaves differently from the pipeline.** | The `arm_seal_for_test` hook is the only way in, and the tests that use it are named (T-SL10, T-SL17). |

---

## 18. Questions only the orchestrator or the owner can decide, each with a recommendation

**Q1 (owner; L10 open question 1, `04-DESIGN-REV2.md:630-631`). Should a velocity written into a sleeping body wake it on the next step (SL-1b)?**
- **Recommendation: yes.** It is Jolt's, Rapier's and Box2D's rule (§2.2).
- Its effects:
  - It makes SL-INV exact: without it, a sub-threshold write stays stored.
  - It removes the one-step lag for writes that do not change the island's count. A kick toward a neighbour already wakes on the event step on the trunk; S-SL3 splits the two per event (O1).
  - It closes the soft→rigid gap, at the cost of a resting-load cycle (PC-SL-7).
  - It costs 0.5–2 µs per frozen step (§13).
- If the answer is no, SL-1 lands alone and 9c records the lag.

**Q2 (orchestrator). Placement.**
- **Recommendation:** SL-2a then SL-1, after SR-B and S5, before IB (§16).
- It moves only sleeping-on pins, and it must be in the trunk before the combined screen so that ruling 24's sleeping arm reads the sealed behaviour for every CS candidate.

**Q3 (orchestrator; a perf fork under ruling 24). The seal also removes the trunk's own attempt-step onset rises on sealed steps.**
- **Recommendation: accept.**
  - It is the mechanism that makes the settle neutral. The settle's own shift and the trunk's onset draw cannot be told apart on that step.
  - It cannot remove a wake caused by any external input.
  - It is strictly more conservative than Jolt, which re-tests nothing while asleep.
  - Ruling 24 found every such rise invisible and cost-only.
- The alternative, keeping the trunk's draw and paying C1, is arm D's measured price.

**Q4 (orchestrator; critique open question 2). Should SL-V be one stage or multiplied arms?**
- **Recommendation:** one stage after the combined screen, on the chosen contact set, {60, 45, 30} × {1e-4, 4e-4, 9e-4} (§10.4).
- Choose the cheapest cell that passes. Expect 30 / 1e-4.
- A G2/G7 move above 0 goes to the owner. Keep 60 / 1e-4 if nothing passes.

**Q5 (owner). The frozen floor (0.11–0.14 ms against Jolt's 2.2 µs, pre-V2) decides the sleeping-on comparison at W ≥ 4 (§11).**
- Removing it means not walking frozen rows in the gather, classify, broadphase and apply. That is a new structural lever, and ruling 25(a) says "nothing else".
- **Recommendation:**
  - record PC-SL-1 now;
  - have 9c's prep (or a quiet window) read the V2 floor's zone split at W1/8/16 with the existing zones;
  - ask the owner, with that split, whether to open "SL-F" after CS.
- Without it, 9c's sleeping-on rows at W ≥ 4 are predicted Jolt-faster even at 30 frames.

**Q6 (orchestrator; critique open question 1). The flip's placement and bar.**
- **Recommendation:** SL-3 after 9c, decided by 9c's pre-registered on/off pairs (§7).
- Add the 26-run set with `--sleeping on` as a flip bar (the shipped default must hold the pile), reported beside the unchanged sleeping-off gate.
- A scene CLAIMED slower with sleeping on goes to the owner.

**Q7 (orchestrator). What should the runner's `--cfg default` mean after the flip?**
- **Recommendation:** keep "the shipped default", with every pinned or gate row carrying an explicit flag (SL-2a).

**Q8 (owner, record only). Should SL add transform-write wake, a per-entity wake API, or kinematic wake?**
- **Recommendation: record, do not build.**
  - No scene or application uses them, and kinematic motion is unbuilt.
  - The seal already refuses to re-key when such a write lands on the step it governs.
- If wanted later, the ECS-native form is a `Wake` enable tag that the gather reads.

**Q9 (orchestrator). Frames or seconds for the dwell?**
- **Recommendation:** keep frames, and fix the doc (R14). Seconds is PC-SL-4.

**Q10 (orchestrator; critique open question 3). S-SL3's control.**
- **Recommendation:** SL-2a lands first, so the parent is an unpatched trunk executable, with SL-2a's byte-identity receipt (§10.3).

### PC lines recorded, not started (ruling 18)

| id | what |
|---|---|
| PC-SL-1 | The frozen-floor lever (Q5). |
| PC-SL-2 | A position term in the sleep metric: Box2D v3's `0.5·|Δx|/dt`, Rapier's drift. |
| PC-SL-3 | Parallel sleep bookkeeping, as Jolt fuses it into the island job (`PhysicsSystem.cpp:2511-2563`). |
| PC-SL-4 | The dwell in seconds. |
| PC-SL-5 | A dynamic body with no contact never sleeps (`resources.rs:3995-3999`, `:4612-4614`), unlike Jolt. |
| PC-SL-6 | A seal watched per island partner and carried through a row remap, so that unrelated spawns, despawns and writes do not unseal. |
| PC-SL-7 | Soft-body sleep, or a soft-rigid sleep coupling, so that a resting soft load lets its rigid support sleep. |
| PC-SL-8 | Fuse SL-1b's write test into the gather (which writes every row) or into R3's pass, to take it off the frozen floor. |

---

## 19. Sources

**Ours, at f277269d** (`D:/wt/joltab/crates/boyko_physics/`):
- `src/resources.rs`: `:460-528`, `:609-640`, `:708`, `:720-737`, `:3095-3200`, `:3302-3373`, `:3422-3436`, `:3571-3583`, `:3596-3720`, `:3943-4051`, `:4053-4089`, `:4091-4214`, `:4228-4238`, `:4259-4315`, `:4317-4533`, `:4535-4648`, `:4659-4785`, `:4971-5025`;
- `src/row_identity.rs`: `:181-223`, `:239-273`, `:275-321`, `:589-613`;
- `src/manifold.rs`: `:54-121`;
- `src/narrowphase/speculative.rs`: `:1-34`, `:99-131`, `:157-273`;
- `src/narrowphase/box_box.rs`: `:1-60`, `:1140-1197`, `:1245-1348`, `:1496-1519`;
- `src/solver/colored.rs`: `:4964-4986`, `:5005-5075`, `:5089-5504`;
- `src/solver/contact.rs`: `:25-59`;
- `src/systems.rs`: `:225-320`, `:480-540`, `:1995-2043`, `:2060-2124`, `:2181-2213`;
- `src/sleep_sets.rs`: `:274-447`, `:930-960`, `:1140-1259`, `:2048-2060`;
- `src/soft/coupling.rs`: `:540-562`;
- `src/plugin.rs`: `:430-438`;
- `src/solver/colored_tests.rs`: `:3935-4023`;
- `tests/sleep_settles_box_piles.rs`: `:200-399`, `:917`, `:1051-1116`, `:2291-2342`;
- `tests/v2_speculative_fidelity.rs`: `:110-126`, `:395-445` (via rev 1 and the critique);
- `benches/jolt_parity_pyramid.rs`: `:1150-1164`, `:1957`, `:2218-2264`, `:2652`, `:2697` (via rev 1).

**Measurements and tools** (`D:/wt/joltab/`):
- `docs/measurements/2026-09-19-physics-p0/ANALYSIS.md`: `:25-45`, `:140-160`;
- `docs/measurements/2026-09-19-physics-p0/p0b/window_report.md`: `:55-70`, `:185-200`, `:255-270`, `:390-398`;
- `docs/measurements/2026-09-19-physics-p0/p0/build_report.md`: `:108`, `:220-221`;
- `docs/measurements/2026-09-28-physics-window8b/analysis/jsondm1.md`: `:1-45`, `:100-155`;
- `docs/measurements/2026-09-30-physics-window9a/README.md`: `:59`;
- `tools/physics_parity/driver.py`: `:58-59`, `:143`, `:145`;
- `docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md`: `:184-192`, `:572-605`, `:625-636` (via rev 1).

**Orchestration** (`D:/tmp/phys-orch/`):
- `phys-next/RULINGS-2026-10-01.md`: rulings 18–25, `:101-237`;
- `win9b/analysis.md`: `:1-50`, `:145-175`;
- `win9b/PREP.md`: `:102`;
- `cs/proto/sleep_sweep/SWEEP.md`: `:15-16`, `:34-35`, `:45-78`, `:148-237`, `:283-298`, `:394-408`;
- `dyn-scenes/cut.md`: `:20-30`, `:340-346`, `:464-469`;
- `sl/research_tree.md`: `:1-60`, `:170-347`;
- `sl/critique_r1.md`, `sl/design_r1.md`.

**Jolt 5.6** (`D:/tmp/jolt/wt-v5.6.0-parity/Jolt/Physics/`):
- `PhysicsSettings.h`: `:48-50`, `:95`, `:102`, `:123`;
- `PhysicsSystem.cpp`: `:962-970`, `:1099-1102`, `:2511-2563`;
- `Body/BodyManager.cpp`: `:555-562`;
- `Body/BodyInterface.cpp`: `:595-638` (via rev 1, verified by the critique);
- `Body/Body.cpp`: `:145-184` (via rev 1).

**Rapier 0.36** (`$R` as in §2.2): `dynamics/rigid_body.rs:915-918`, `:1016-1030`; `dynamics/rigid_body_components.rs:1362-1377`, `:1487`, `:1489-1493` (via rev 1, verified by the critique).

**Box2D v3, PhysX 5:** `research_web.md` [11]–[13]. Used only where they agree with Jolt and Rapier.
