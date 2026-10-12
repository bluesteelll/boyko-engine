# Lever SL: the lane record

Lane `u/phys-sl` (worktree `D:/wt/sl`), cut from `integ/unified` `264e5b2d`, built 2026-10-11 and
2026-10-12. The plan was the session cut `D:/tmp/phys-orch/sl/cut.md` with its three addenda, and rulings
39 and 40. No timing was taken in the lane: every claim below is structural (bits, counts, codegen), and
the timed reading is the SL-AB block of section 6.

The design is `01-DESIGN.md`, the screens are `02-SCREENS.md`, and the pins are `03-PINS.md`. The session
reports cited here live under `D:/tmp/phys-orch/sl/`.

## 1. Commits

| commit | sha | what |
|---|---|---|
| C1 = SL-2a | `f1f733f6` | The parity runner accepts `--dyn` with sleeping on (still refused with it: `--threshold`, `--frozen-by`, `--solver reference`, and `--scene-dump` with sleeping on). A `sleep` SUMMARY object is appended only on colored sleeping-on rows, read untimed after each step. Every pinned runner row of the V2 gate scripts names `--sleeping`; the three `*_v2.sh` are regenerated. No engine source |
| C2 = SL-1 | `57d6fe62` | The rest state, the settle (a sleeping island's velocity is zero), the seal, the fold on `rest`, the counters, the docs; the re-expressions (B1, `topology_churn…`, G2/G7); the re-pins (`03-PINS.md`); the fixture mirror E3 `docs/measurements/2026-10-11-sl-sleeping/`. Its message named LB5 as a KNOWN RED (the STOP that ruling 40 resolved) |
| C2b | `0ae0ef3e` | LB5 (`s3_mid_step_dt_write_is_invisible`) re-expressed on `towers_beside_a_moving_kinematic` (ruling 40) |
| C3 = SL-1b | `3bbd9334` | The write-wake, the C-2 capture assert, the T1/H-03 re-expression (C-10), the `write_wake_rises` runner key, the SL-1b docs and `plugin.rs`'s `add_physics_soft` warning |
| trunk sync | `a56810a0` | `integ/unified` `9cd4ad6a` merged in. Git's automatic merge: the tree equals `git merge-tree`'s, and it touches only `crates/boyko_ui/tests/{p4_bind_zero_alloc,zero_alloc}.rs` |
| fix r1 F1 | `53ee9865` | T-SL3 writes each of the six velocity components alone |
| fix r1 F2 | `73e50133` | T-SL21 (unit), T-SL22..T-SL29 (scenes), the `seal_equal` docs |
| fix r1 O1 | `7bef44e2` | `begin_step`'s docs name `begin_step_armed` as the solve's entry (doc only) |
| fix r1 | `75a1ccb5` | T-SL27 checks the floor's `Sensor` by `has_component`, without naming `AddOutcome`. UG-10's `documents_that_count_the_tree_are_re_measured` had gone red: `docs/REFLECTION-PLAN-ECS.md` counts the lines naming `AddOutcome` under `crates/` (43), and T-SL27 had made them 45 |
| fix r2 N1 | `3b30bdcb` | T-SL30 (unit) and T-SL3 in both signs of every component |
| fix r2 W1 | `6c9e0368` | T-SL27's order guard reads the walk before the floor's move; T-SL27c controls its attribution |
| fix r2 O1 | `493be29a` | `rekey_rows` states the rows-stable test the code applies (doc only) |
| C4 (this record) | the commit that adds this directory | docs only |

From `3bbd9334` to `493be29a` no non-doc line of engine source changed.

## 2. Gates per commit

All builds: msvc stable, `CARGO_TARGET_DIR=D:/wt/_targets/sl-msvc`, `CARGO_BUILD_JOBS=2`, `--locked`. Long
gates ran through the kit (`D:/tmp/phys-orch/kit`), with outputs under `D:/tmp/phys-orch/sl/k/<gate>`.

**C1 `f1f733f6`** (`impl_c1.md`; kit `summary.sh`: `SUMMARY GREEN: 9 gates`):
- check and clippy `--workspace --all-targets -D warnings`: OK.
- Byte receipt `existing_unchanged.py` C1 against the base exe, `--only` 10 rows: `rows 10, same 10, diff 0`.
  The sleeping-on row `J500sleep_W1` differs only by the `sleep` key (scratch compare); its pose stays
  `0x130c76cb6b463ab8` and its first frozen step 147.
- P4 `rows 12, pass 12, fail 0`; P11 `s5_runner_gate.sh` 54/54 and `ours_gates.sh` 49/49; P5 both sets
  264 / 26 / 17 with both negatives exit 4.
- UG-10: 24 passed.
- `git diff --stat f1f733f6^ f1f733f6 -- crates/boyko_physics/src` is empty.
- The census of runner rows without `--sleeping`: 6 lines, the two PC-SL-16 refusal rows and the
  self-check, each in `l10/` and its `_v2` copy (the parent's census: 42 lines).

**C2 `57d6fe62`** (`impl_c2.md`):
- check and clippy: OK.
- Zero cost when off: `existing_unchanged.py` C2 against C1, 10 rows: `rows 10, same 10, diff 0`. The
  codegen receipt is in `03-PINS.md` §6.
- P4 12/12; P5 both sets 264 / 26 / 17 on the re-pointed fixtures, both negatives exit 4; P11 54/54 and
  49/49; P10 4/4 with the two re-pinned anchors; P6 `GATE: GREEN — 19 scenes`, debug and release; P2
  unmoved.
- `sleep_settles_box_piles` release: `running 17 tests`, 16 passed, 1 ignored (the generator).
- `sleep_velocity_zero`: 3 + 1 ignored (debug), 4 (release). `sleep_seal` 8. lib 348 + 1 ignored;
  `g1_pins` release 7. `frozen_island_warm_start` 6. The other sleeping binaries: all green.
- **`sleep_skip_bit_identity` (debug): 57 passed, 1 failed: LB5, the STOP that ruling 40 resolved.**
- UG-10 24; `ignore_reasons_census` 12 (400 sites; C2 adds 1, `slow:` on T-SL10p);
  `production_reachability_census` 13; `physics_vec_side_store_census` 5.

**C2b `0ae0ef3e`** (`impl.md`): LB5 green on Tree and Grid, W1 and W8; the whole file 58/58 (debug);
check and clippy OK.

**C3 `3bbd9334`** (`impl.md`):
- check and clippy: OK.
- Debug, 25 targets by name: lib 348 (1 ignored), apply_row_alignment 11, broadphase_tree_s5 4,
  broadphase_tree_scenes 10, default_world_sleep_worker_invariance 1, friction_manifold 10,
  frozen_island_warm_start 6, profiling_bit_identity 1, profiling_zone_counts 1, row_identity_remap 12,
  row_keyed_state_defect_a 8, sdf_sleep_settles_and_wakes 1, sleep_seal 8, sleep_settles_box_piles 9
  (8 ignored), sleep_skip_bit_identity 58, sleep_velocity_zero 3 (1 ignored), sleep_write_wake 6,
  sleeping_pipeline_o8 2, sr_region_store 1, sr_region_worker_invariance 1, support_loss_wakes_sleepers 14,
  v2_speculative_contacts 8, v2_speculative_fidelity 0 (4 ignored), soft_body_sp1 15, soft_body_sp2 17.
- Release: svz 4, seal 8, write 6, piles 16 (1 ignored), `g1_pins` 7, B1 6, P10 4/4, P6 debug and
  release GREEN.
- P4, P5 (both sets), P11 (49/49, 54/54): green, on the C3 exe built before the commit from the
  committed sources (`bin/runner_C3_3bbd9334.exe`).
- Zero cost when off: `existing_unchanged.py` C3 against C2, 10 rows: SAME 10. The sleeping-on rows J500
  W1/W8, J500 reuse off and R1100 are unchanged: pose bytes equal, SUMMARY equal but for the 2 new keys,
  `write_wakes 0`, `write_wake_rises []`.
- UG-10 24; the three root censuses 12 / 13 / 5; P9 `SR-B-C6 5 7`.
- **No pin moved at C3.**

**Test round 1, on `3bbd9334`** (`test_r1.md`):
- The workspace run: 854 test targets, 6763 passed, 0 failed by result line, 202 ignored.
  - One crash: `boyko_rhi_vulkan` `sdf_gbuffer_hybrid`, `0xc0000005` at 4 test threads, 43/0/8 at 1.
  - It has no `boyko-physics` edge, and the lane does not touch that crate. It is outside the lane.
- Every pin P1-P11 unmoved, or at its declared new value.
- The device-free ignored leg: `running 1 test`, passed.
- Miri, `resources::tests::sl::`: SB (`MIRIFLAGS="-Zmiri-disable-isolation"`) and TB
  (`MIRIFLAGS="-Zmiri-tree-borrows"`), `running 7 tests`, 7/7 each.
- The developer's 22 mutations: 22 of 22 red their named test (section 4).
- The tester's own mutations: two classes escaped (F1, F2), and triage round 1 confirmed both.

**Test round 2, on `75a1ccb5`** (`test_r2.md`): F1 and F2 hold; Miri SB and TB 8/8. The tester's own
mutations found N1 (the write test's sign, NaN and `-0.0` were ungated) and N2 (T-SL27's `order_kept`
guard could not fail). Triage round 2 confirmed both; the reviewer reported N2 as W1.

**Test round 3, on `493be29a`** (`test_r3.md`): GREEN.
- N1 and W1 = N2 are red-first on both sides.
- 21 of the tester's 32 mutated runs are caught by a named gate.
- Miri SB and TB: 9/9 on `resources::tests::sl::`.
- Its findings F1-F4 are outside the delta: PC-SL-18 .. PC-SL-20.

**The close run, on `493be29a`** (`test_close.md`): GREEN.
- check and clippy clean; the head's parity exe sha256 is
  `003f33e62d28cba49c27436d2cac13c68c643b1139557c08a6935d488759ad42`.
- The workspace run: `cargo test --workspace --tests --no-fail-fast --locked -- --test-threads=4`,
  `WS_EXIT=0`.
  - 854 test targets, the same 854 as round 1 by name; 6817 passed, 0 failed, 210 ignored.
  - Three rows differ from round 1, each explained: lib 351 (+2 units), `sleep_seal` 17 (+9), and
    `sdf_gbuffer_hybrid` 43/0/8 (its crash did not reproduce).
- P1-P6, P10, P11, UG-10 (24) and `ignore_reasons_census`: green, with the tallies of `03-PINS.md` §5.
  The census reads 400 sites (210 plain, 190 cfg_attr), 1884 `.rs` files walked, 0 waivers.
- The sleeping-off byte receipt: SAME 10 of 10, against the round-1 parent exe and against a parent exe
  rebuilt from `264e5b2d`.

**The screens, on `493be29a`:** PASS (`02-SCREENS.md`).

**The reviews:**
- Round 1 (`review_r1.md`): APPROVED, two optional items and two open questions.
- Round 2 (`review_r2.md`): CHANGES REQUESTED, W1 = N2.
- Round 3 (`review_r3.md`): APPROVED, two optional items.
- The close (`review_close.md`): APPROVED, 0 / 0 / 0. The trunk-sync merge is git's automatic one, and both
  sides survive it byte for byte.

## 3. Red-firsts (each run on the parent engine; the commit holds the test and the code)

**C1 (SL-2a):**
1. The parent exe with `--scene jolt --dyn kick --sleeping on --steps 10` exits 2 ("--dyn, --sanity and
   --scene-dump run the colored solver with sleeping off: every engine runs the scenes awake"). C1 exits 0
   and prints the `sleep` object.
2. **The flip mutation**, a scratch `PhysicsConfig::default().sleeping = true` in its own target:
   - the parent's flagless `runner_pins.sh` reads `rows 12, pass 0, fail 12`;
   - C1's scripts with the same exe read `rows 12, pass 12, fail 0`;
   - the flipped J500 row reads `0x130c76cb6b463ab8`, so the mutation compiled in.
3. `--perturb`: not applicable (ruling 39 Q3 = no).

**C2 (SL-1), on C1's engine:**
- T-SL1: `freeze_step 78, frozen_steps 31, frozen_speed_sq_max 1.5590354e-8, nonzero_steps 31`. RED, as
  predicted.
- T-SL2: `island {0, 1} frozen true`. RED, as predicted.
- T-SL10p (release): `b38xp` and `b581zm` both read `contact_wake_rises [146]`, first frozen 206, 1240
  contact wakes. RED, as predicted.
  - On C2, `b581zm` reads `settle steps [(145, 1240)]`, first frozen 146, counters
    `[0, 0, 1240, 1, 1240, 1, 0]`.
- The guards' red is the mutation table (section 4).

**C2b (ruling 40):** with the D9b defect re-introduced (`physics_narrowphase_colored` reads the live
`PhysicsConfig`), LB5 reds: "Sets diverges from Off after step 150: manifolds[10] of 12/12". With the latch
restored it is green.

**C3 (SL-1b), on C2's engine** (`FAILED. 1 passed; 5 failed`, each the cut's predicted red):

| test | on C2's engine | on C3 |
|---|---|---|
| T-SL3 Kick | `column awake false, top moved false, (write, contact) wake rises (0, 0)` | `column awake true, top moved true, (1, 0)` |
| T-SL4 | `awake false, (0, 0)` | `awake true, (1, 0)` |
| T-SL5 | `awake false, solved false (x unchanged, v.x 0.5)` | `awake true, solved true (x -0.0000122 -> 0.0074665, v.x 0.4183), (1, 0)` |
| T-SL6 | `next step frozen true, write wakes 0 -> 0` | `write wakes 0 -> 1` |
| T-SL6b | "SL-INV: the box entered frozen load step 14 with velocity (.., -2.3870664, ..)" | 587 steps, 1 frozen, settles +9, write wakes +10 |
| T-SL8 | green (expected on the parent) | equal on every step: 104 / 104 / 87 / 81 / 666 steps, W1 and W8 |

T1/H-03 (C-10):
- The re-expressed file is green on C2's engine, 12/12.
- On C3 under the pair M-A + M3b, the re-expressed T1 and both H-03 red.
- The OLD T1 stays green under that pair: the write-wake hides the carry, which is C-10's reason.

**Fix rounds** (`fix_r1.md`, `fix_r2.md`, `test_r3.md`):
- F1: each single-component drop of the write test reds its own T-SL3 cells ("4 of 24 cells failed").
- F2: each of the 13 warning-free `seal_equal` mutants reds T-SL21 with exactly its own blind list, and
  the scene mutants red T-SL22..T-SL29.
- N1: A_POS, A_NAN and A_NZ are green on `75a1ccb5` and red on `493be29a` (T-SL30, T-SL3).
- W1 = N2: D_ORDER is green on `75a1ccb5` and red on the head (`left: Some(false)`). R_STABLE_NEVER reds
  T-SL27c with the order kept.

## 4. Mutations

The developer's 22 (`D:/tmp/phys-orch/sl/c2/mutations.py`, each an exact replacement) were run by test round
1 in release, one at a time, reverted by content with an md5 check. **22 of 22 red their named test:**

| mutation | named test red |
|---|---|
| M-SL1 (no settle call), M-SL7 (settle before the write-back) | T-SL1 |
| M-SL2 (zero every row this `end_step` latched), M-SL4 (write test on `asleep` rows) | T-SL7 |
| M-SL3 (fold on `asleep`) | T-SL2 and `topology_churn…` |
| M-SL6 (the contact unlatch keeps `rest`) | `support_loss_wakes_sleepers` |
| M-SL9 (`sealed = false`), M-SL11 (stash before the zero) | T-SL10 and T-SL10p |
| M-SL9a (rows-stable ignored) | T-SL11s |
| M-SL9b / 9c / 9d / 9e (non-members not stashed / members' poses ignored / epoch ignored / SDF edits out of the epoch) | T-SL12 / T-SL13 / T-SL14 / T-SL16 |
| M-SL10 (re-key FROZEN rows) | T-SL15 |
| M-SL12 (count `contact_wakes` only) | `the_attempt_event_counts_a_settle_rekey` |
| M-SL13 / M-SL14 / M-SL15a / M-SL15b | T-SL17 / T-SL18 / T-SL19a / T-SL19b |
| M-SL5 (ω ignored) | T-SL3 Spin |
| M-SL8 (the test reads the post-solve snapshot), M-SL1b0 (no write test) | T-SL3 |

- In debug, M-SL5, M-SL8 and M-SL1b0 red through the C-2 capture-assert panic inside the solve. The test
  then fails at `SCENE_TIMEOUT` (600 s), so they were run in release.
- B1's own mutation list (`frozen_island_warm_start.rs` header) reds the re-expressed B1 as its header
  states.
- The tester's own mutations across the three rounds found the gate gaps that fix rounds 1 and 2 closed
  (F1, F2, N1, N2), and the open items PC-SL-18 .. PC-SL-20.

## 5. Rulings 39 and 40 (verbatim)

From the session file `D:/tmp/phys-orch/phys-next/RULINGS-2026-10-01.md` (sha256
`751dd3360aa8c57fa4a5fdbd9339fec6b44ad5c18c62827fb92f9bca60c7c8ae`):

> 39. **SL lane C2 prerequisites (orchestrator, 2026-10-11; sl/cut.md section 7 + addendum W3; the lane STOPPED after C1 = SL-2a f1f733f6):**
>     (Q4) **PIN_S_B re-pins at C2 by A7-R1's rule** (old/new/command/reason in the commit; PC-SL-11's clause added). The pin block's
>     "a red pin is a defect, never a re-bless" guards against UNEXPLAINED moves; SL is an owner-approved value-changing lever (ruling 25;
>     docs/physics/perf-campaign/00-RULINGS.md owner D2) whose purpose IS the stored-velocity defect, and Q4 = no would keep that defect in
>     the public `solve_colored_sleeping` and split one solver's semantics. The pin block's text is amended to say so (an approved lever's
>     intended move follows A7-R1's rule; any other red is a defect). (Q1 + Q2) **Fixtures go to a NEW mirror
>     `docs/measurements/2026-10-11-sl-sleeping/`** (V2's precedent: originals read, never edited): both sets' directories mirrored whole,
>     sleeping-off files byte copies, sleeping-on files re-recorded, with record_sl_fixtures.sh, logs/record.tsv, SHA256SUMS and a README
>     old/new table; NO record directory is edited (this overrides the lock's in-place L10 grant). The lock is extended to: that new
>     directory; the dir variables and hash literals in gate/l10/*.sh; make_v2_scripts.py DIRS/HASHES and the regenerated *_v2.sh; the
>     pose_gates_d0.sh header; plus an assertion that no SL d0 path survives into a _v2 script. Tallies 264/26/17 unchanged.
>     (Q3) **No runner --perturb** (close-out: no scope growth; the screens and T-SL10p carry the perturbation themselves, as SL-2a did).
>     (Q5) the fallback stands (flagless until SL-3; PC-SL-16). (Q6) **Allowed outside the lock:** `solve_colored_held`'s added parameter
>     (colored.rs ~:4659-4669) and the public `solve_colored_sleeping` doc (~:4620-4639) - neither is in fm's lock set; doc-only lines
>     plugin.rs:375-377, :434-436, scratch_ids.rs:292-294; anchor-only fixes in GATED_DOCS that UG-10 reds because SL shifted a cited line
>     (old->new in the commit message); a docs/ru anchor red is a STOP (frozen). (Q7) merge bar = the untimed gates and screens; SL-AB rides
>     the next quiet window under cut section 5's pre-registered rule (sleeping stays opt-in until SL-3). Critique W1/W2/W3 resolutions in the
>     addendum are accepted. Ruling 24 still holds: a G2/G7 value move is reported to the owner before merge.
>
> 40. **SL C2 STOP on LB5 (orchestrator, 2026-10-11; sl/cut.md Addendum 2):** option **(a)**. SL's lock is extended to the ONE test
>     `s3_mid_step_dt_write_is_invisible` (LB5) in crates/boyko_physics/tests/sleep_skip_bit_identity.rs (plus a shared helper only if it
>     is new and used by LB5 alone); fm's hunks in that file (the DA9 row, Obs.fseeds/fstats) are disjoint - union at fm's trunk sync.
>     LB5 is re-expressed so its anti-vacuity clause (ii) is REACHABLE under SL (a frozen pair whose `is_fast` can flip: the kinematic
>     neighbour carrying a velocity, or another reachable shape), keeping the D9b class it guards: red-first = re-introduce the D9b defect
>     (Off classing frozen pairs fast while Sets skips them) and show the re-expressed LB5 red; green on C2b; clause (ii) asserted, never
>     weakened. (c) waiver REFUSED (an anti-vacuity guard is not waived to merge); (b) refused (the trunk would carry a known red until fm).
>     If NO reachable world can flip `is_fast` on a frozen pair under SL (e.g. a kinematic body cannot carry a velocity on this tree),
>     STOP again with the evidence - do not invent a different guard. C3 (SL-1b) follows C2b.

## 6. The merge bar, and the quiet-window commands for SL-AB

The cut's §5, verbatim (`cut.md` sha256 `17bedade3d132a849d46e0ea4732db3f21169ea75216286009c0307ee404768d`).
Ruling 39 Q7 adopted its merge bar: merge on the untimed gates and screens; SL-AB rides the next quiet
window under the pre-registered rule.

> **Merge bar recommendation (Q7):**
> - **Merge on the untimed gates and screens.** SL changes no sleeping-off value. The only off-path delta is one `None` argument.
>   Sleeping is opt-in. Precedent: ruling 32.
> - **The SL-AB block** rides the next quiet window (the owed closed-app window, or 9c prep, whichever comes first).
> - **The pre-registered rule:**
>   - SL-JT-off or SL-DYN-off CLAIMED slower at any W → the orchestrator stops fm's merge and decides between a revert of SL's
>     merge commit and a fix lane;
>   - a CLAIMED-slower sleeping-on cell → recorded for SL-3 (O-6), not a revert.
>
> **Prep (untimed):**
> - Build two exes from exports, one target at a time, deleting each after its copy:
>   `git -C D:/wt/sl archive <sha> | tar -x -C D:/wt/_scratch/sl-win/<sha>`, then
>   `cd D:/wt/_scratch/sl-win/<sha> && PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc
>   CARGO_TARGET_DIR=D:/wt/_targets/sl-win-<sha> CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo bench --no-run --locked --profile parity -p
>   boyko-physics --bench jolt_parity_pyramid`.
>   - `<sha>` = C1 (key `slP`) and the merged lane tip (key `slT`).
>   - Copy the exes to `<W>/bin/runner_sl{P,T}_<sha8>.exe`, with sha256 pins.
> - **Block SL-AB** goes into the window's rows generator. Use window SR's protocol (ruling 1 of W8: K 9 = 3 passes × 3 rounds; ruling
>   8 of W8B; RECEIPT_S; the idle rule; in-block and adjacent), `kind: runner`, binaries `[slP, slT]`, W {1, 8}:
>
> | row | args | steps | metric windows | pose_ref |
> |---|---|---|---|---|
> | SL-JT-off | `--scene jolt --gap 0.5 --cfg default --broadphase tree --sleeping off` | 500 | [0,100), [100,500) | JT500 V2 `0x441a568e91a4f9c9` (both exes) |
> | SL-JT-on | `… --sleeping on` | 500 | [0,100), [100,500) | slP `0x130c76cb6b463ab8`; slT = the E3 re-record |
> | SL-DYN-off-{kick,shoot,slide} | `--scene jolt --gap 0.5 --cfg default --dyn <p> --sleeping off` | 800/800/500 | [200,800) / [200,800) / [0,500) | pins.json ours |
> | SL-DYN-on-{kick,shoot,slide} | `… --sleeping on` | same | same | slP's and slT's own untimed gate runs (recorded at prep) |
>
> - **Pre-registered readings:**
>   - SL-JT-off and SL-DYN-off: tip vs parent NOT CLAIMED slower at W1 and W8 (the zero-cost claim).
>   - SL-JT-on and SL-DYN-on: reported, with each row's `sleep.*` receipts. The expectation is no change: the freeze step is
>     equal on J-T, and the settle and seal cost about 20-60 µs once per attempt.
> - **Size:** 32 cells × 9 ≈ 288 processes, about 40 min. This is an estimate; the prep's dry run replaces it.
> - **Launch:** per the window's own `PREP.md` (`Start-Process`, `--cutoff`, no agent alive, the Claude app minimized or closed, ruling 37 R10).

**Resolved for the record** (facts read by the lane; nothing here changes the cut's rows or rule):
- **The prep command above needs its directory first** (record review W1): `tar -x -C` does not create it, so GNU tar
  exits 2 ("Cannot open: No such file or directory") as quoted. Run `mkdir -p D:/wt/_scratch/sl-win/<sha>` before
  `git -C D:/wt/sl archive <sha> | tar -x -C D:/wt/_scratch/sl-win/<sha>`. If `D:/wt/sl` no longer exists when the
  window is prepared, `git -C D:/claude/BoykoEngine archive <sha>` reads the same commit.
- `slP` = C1 `f1f733f6`. `slT` = the merged lane tip, i.e. the commit the lane merges as. Its engine and
  runner code equals `493be29a`'s (and `3bbd9334`'s): every later commit is docs or tests, unless the
  merge's trunk sync brings a physics change.
- **Pose references the lane's own runs already read** (the prep re-reads its own builds, as the cut says):

  | row | slP (`f1f733f6`) | slT (`493be29a`) | source |
  |---|---|---|---|
  | SL-JT-off | `0x441a568e91a4f9c9` | `0x441a568e91a4f9c9` | P4 `J500`; the sleeping-off byte receipt |
  | SL-JT-on | `0x130c76cb6b463ab8` | `0xc637a0517adabab7` | E3 `v2/w8/JSonT500`. S-SL1's `none` reuse-on row (the same argv plus `--contact-reuse on --workers 1 --arm-profiler` and its output flags) reads the same pair on C1 and on the head |
  | SL-DYN-off kick / shoot / slide | `0x5c27d4ede389f12a` / `0x49e2bf025baa5294` / `0xe627f35c282605ba` | the same | `docs/physics/perf-campaign/levers/scenes/pins.json` ours; P11 green on the head |
  | SL-DYN-on kick / shoot / slide | `0x1dcb064143de7501` / `0x7a20a407e3eebdc3` / `0x54319f083eea058e` | `0xca3161f209b56e41` / `0x0a3bbe0a97f62147` / `0x39656286b0c2c3cc` | S-SL3 (W1 = W8 = the `--sanity` run; 800 / 800 / 500 steps; `02-SCREENS.md` §4) |

- **PC-SL-17 applies to SL-DYN-on-kick and SL-DYN-on-shoot.** An armed (`--arm-profiler`) run of those rows
  exits 3 (void) on both exes, while the unarmed `--sanity` run exits 0. Whether the window's runner rows
  arm the profiler is for the prep's dry run to show.
- The expectation for SL-JT-on: the freeze step is equal on J-T (147 on both exes, S-SL1 `none` reuse on).
  For SL-DYN-on, S-SL3 read the whole-run solved slots of slT at or below slP's (−0.135 % on KICK,
  −0.154 % on SHOOT, equal on SLIDE). These are counts, not time.
- Ruling 38 (2026-10-10, the owner's command) sets the owed closed-app window: the Claude desktop app
  CLOSED, not minimized. If SL-AB rides that window, that condition holds in place of the cut's
  "minimized or closed".

## 7. PC lines (recorded, not started)

PC-SL-1 .. PC-SL-8 are design rev 2's (§18), PC-SL-9 .. PC-SL-15 the cut's (§6), PC-SL-16 the cut's first
addendum, and PC-SL-17 the screens'. **PC-SL-18 .. PC-SL-23 are numbered by this record:** they are the
review, test and screen findings the lane routed as PC-line candidates (physics close-out: findings become
PC lines, no scope growth).

| id | what | source |
|---|---|---|
| PC-SL-1 | The frozen-floor lever: stop walking frozen rows in the gather, classify, broadphase and apply (0.11-0.14 ms against Jolt's 2.2 µs, pre-V2). Read the V2 floor's zone split at W1/8/16 first, then ask the owner whether to open "SL-F" after CS | design §18 Q5 |
| PC-SL-2 | A position term in the sleep metric: Box2D v3's `0.5·|Δx|/dt`, Rapier's drift | design §18 |
| PC-SL-3 | Parallel sleep bookkeeping, as Jolt fuses it into the island job (`PhysicsSystem.cpp:2511-2563`) | design §18 |
| PC-SL-4 | The dwell in seconds, not frames | design §18 Q9 |
| PC-SL-5 | A dynamic body with no contact never sleeps, unlike Jolt | design §18 |
| PC-SL-6 | A seal watched per island partner and carried through a row remap, so that unrelated spawns, despawns and writes do not unseal. **Note (review r1, open question 1):** the stash watches every non-member row, so an animated kinematic anywhere breaks every seal. S-SL1's arm D runs on J-T (one static, no kinematic), which says nothing about scenes with many statics or animated kinematics: SL-3's concern | design §18; `review_r1.md` |
| PC-SL-7 | Soft-body sleep, or a soft-rigid sleep coupling, so that a resting soft load lets its rigid support sleep (as built, the support is frozen on no step after the first reaction: `01-DESIGN.md` A.6) | design §18 |
| PC-SL-8 | Fuse SL-1b's write test into the gather (which writes every row) or into R3's pass, to take it off the frozen floor | design §18 |
| PC-SL-9 | Window rows files carry flagless ours rows (`win-land/rows_land.json` LJ-JD: `--cfg default` with no `--sleeping`). Every future rows generator (9c prep) emits an explicit `--sleeping` | cut §6 |
| PC-SL-10 | CS's planned SUMMARY keys (`contact_wakes`, `contact_wake_rises`) exist from SL-2a under `sleep.*`. CS reads them there and does not re-add them | cut §6 |
| PC-SL-11 | The G1 pins' "a red pin is a defect, never a re-bless" needed a clause for value-changing levers. **Done in C2** (ruling 39 Q4; `03-PINS.md` §1) | cut §6 |
| PC-SL-12 | After the merge, the orchestrator updates the machine-local pin text: the kit README's P10 row and `pins_line.txt` take SL's two anchors (`0xc637a0517adabab7`, `0x4aec2a5f78f488d0`) and the E3 fixture paths | cut §6 |
| PC-SL-13 | SL-V's sleeping-on drift bound per cell (critique W2, second half) belongs to the sl-v lane (plan §2.9) | cut §6 |
| PC-SL-14 | The `plugin.rs` and `scratch_ids.rs` doc lines, if Q6 = no. **Moot:** ruling 39 Q6 allowed them, and C2/C3 wrote them | cut §6 |
| PC-SL-15 | The book page on the sleeping semantics (the zero at sleep, the write-wake), for the doc-writer only | cut §6 |
| PC-SL-16 | `l10/runner_checks_c3c.sh`'s `refuse_skip_default_cfg_sleeping_unset` and `refuse_skip_cfg_a_unset` stay flagless (their subject is the unset flag). At SL-3's flip the first turns from a refusal (exit 2) into an accepted row, so SL-3 re-expresses it by its own rule. `ours_gates.sh` (BASE, `pre_dyn`, `pre_jt`) and `existing_unchanged.py`'s `ROWS` stay flagless until SL-3 (the Q5 fallback) | cut Addendum 1 |
| PC-SL-17 | The runner: an armed `--dyn kick|shoot --sleeping on` row exits 3 on C1, C2 and the head, because the anti-vacuity recount expects `rows - members` rows queried and a sleeping row is neither (`phys_bp_queried + phys_bp_members` is conserved). Every armed sleeping-on dyn row is "void" for any gate that reads the exit code. SL-3's flip and the 9c rows generator should know; the fix is in the runner's recount, not the engine | `02-SCREENS.md` F1 |
| PC-SL-18 | Two documented seal-lifetime legs of `rekey_rows` are executed by no test: a live seal that no begin consumed counts a break (the solver's early return), and a pending seal meeting a missed gather (`Reset`). Mutants placing `unreachable!` in each leg pass every SL test, and R_LIVE_FREE, R_LIVE_KEEP and R_RESET_OK escape. A tests-only closure: a pipeline scene that settles a pile, runs one step with no dynamic body, and asserts `seal_breaks == 1`, `seal_held == 0` and that the next count change wakes; and one that toggles sleeping off for a step between a settle and the next sleeping step. S-SL1 did not reach these legs (34/34 read settles 1, held 1, breaks 0) | `test_r3.md` F1 |
| PC-SL-19 | The write test's `b.simulated` term is ungated: with it removed, the SL set and eight further sleeping binaries stay green. A killer: a settled pile whose member loses `Simulated` and carries a non-zero velocity, expecting `write_wakes` flat and the pile frozen | `test_r3.md` F2 |
| PC-SL-20 | Test-side hardening, each backed by another gate today. (a) The write test is gated per component, never across components: a state with two components of opposite sign and equal magnitude must read as written (T-SL30). (b) T-SL30's `velocity(b, c)` helper needs a self-check that its six `c` reach six distinct fields. (c) T-SL3's `Sign::of` is unchecked. (d) T-SL27c prints `settle_rekeys` but does not assert it. (e) T-SL8a's single assert lets the first differing change mask the rest, so its 16th change (`TagFloor`) is not shown able to fail alone. (f) `resources_tests.rs` asserts the sign of `f32::NAN`; build both NaNs from bits (review r3 O1). (g) `sleep_seal.rs`'s header says every test requires the trunk's wake, but T-SL27c and T-SL8a are controls (review r3 O2). (h) `rekey_rows`'s doc says an archetype move breaks the seal only when it reorders the walk; a move that removes `Collider` or `RigidBodyMass` also breaks it, through `len` (review r3 O2) | `test_r3.md` F3, F4; `review_r3.md` O1, O2 |
| PC-SL-21 | `sync_rows`'s "Not reachable today (`inv_mass` is stable after spawn)" (pre-existing trunk text) is contradicted by T-SL29, which writes `RigidBodyMass.inv_mass = 0` at runtime and which the gather reads each step. The hazard that caveat describes (static → dynamic with a stale latch across no-dynamic-body early returns) is therefore user-reachable: a doc and contract question | `review_r2.md` open question 2; `triage_r2.md` |
| PC-SL-22 | A dyn row that exercises the write-wake under load: S-KICK's write wakes fire on 4 of its 24 events, because 20 events hit an already-awake pile. A window that wants the write path under load needs a kick schedule that lets the pile fall asleep between events | `02-SCREENS.md` F4 |
| PC-SL-23 | S-SL1's bar (i) (0 visible re-wakes) was vacuous on the shipped arms: L and L+b re-wake on none of the 34 runs. If the owner wants (i) to carry weight, it needs a seed set that makes L re-wake: a new screen, not this one | `02-SCREENS.md` §6 |

Not SL's, recorded so they are not lost:
- `boyko_rhi_vulkan` `sdf_gbuffer_hybrid` crashed with `0xc0000005` at 4 test threads in test round 1 (43/0/8
  at 1 thread). It did not reproduce in the close run. It has no `boyko-physics` edge; it belongs to a
  render/RHI line.
- A failing proptest inside a mutated run wrote `crates/boyko_physics/proptest-regressions/` into the lane
  tree (moved out by the tester). The directory is not git-ignored.
- A process lesson from the screens (F5) and from test round 1: two trees built into one shared target reuse
  an rlib or an exe when the sources' mtimes are older than the dep-info. Every arm build needs a
  `Compiling` line and an exe hash distinct from the other arms'.

## 8. Owed beyond this lane

- **The union with the trunk.** `integ/unified` moved to `410c30ef` (a render merge, zero path overlap with
  the lane's diff) during the close run. The lane's tests did not run on that union. The merge re-syncs
  and runs the root census gates on the union (a clean path list is not a compiling union).
- **SL-AB** in the next quiet window (section 6). A tip that is CLAIMED slower on a sleeping-off row stops
  fm's merge, and the orchestrator decides between a revert of SL's merge commit and a fix lane.
- **The device-needing ignored leg** (135 tests): the orchestrator's or the owner's. No SL code reaches a
  device.
- **SYSTEMS.md / FEATURE_MAP.md rows** for `SealEpoch` and `settle` go to the records lane (doc tidy), not
  this lane (cut §6).
- **PC-SL-12's machine-local pin text,** after the merge.
- **SL-3** (the default flip) waits for 9c, and is decided by 9c's pre-registered on/off pairs.
