# FM — C1 record: friction per manifold behind `PhysicsConfig::friction_model` (as cut and built)

C1 builds the per-manifold friction model of [01-DESIGN.md](01-DESIGN.md) switched off: the default
is `FrictionModel::PerPoint`, and every pin of the trunk is unchanged. Branch `u/phys-fm-c1` from
`264e5b2d`. The cut is `D:/tmp/phys-orch/fm/cut.md` (its addenda 1-3 record the developer's
decisions, the red-first readings and every deviation); the implementation report is
`fm/impl.md`; the mutation logs are `fm/mut/`. Code is cited by symbol. This file is not in
`tests/internal_docs_anchors.rs`'s `GATED_DOCS`; its anchors are not machine-checked.

---

## 1. Commits

| commit | subject | what |
|---|---|---|
| C1a `15862463` | `refactor(physics): FM (1a) - PhysicsConfig::friction_model …` | `FrictionModel`, the field after `relax_iterations` ("It changes values" doc), `Default` = `PerPoint`; the step_inputs round trip and DA9's row (Unobservable until C1b) |
| C1b `6d7ec26e` | `perf(physics): FM (1b) - friction per manifold behind PhysicsConfig::friction_model …` | the mechanism (D1-D16): the patch block, `<const FM: bool>` instances picked at the callers, fixed-slot warm records, `carry_fm`, the toggle's cold zeroing in both stores, depth-1 cohorts on today's code, `FrictionSeedStats`, `for_each_friction_seed`, `layout_census`; the unit, G1, friction-behaviour, W-invariance and Miri gates |
| C1c `e4513ef1` | `test(physics): FM (1c) - the sleep-skip compare sees friction …` | `Obs.fseeds` / `fstats`; FM-on fixture arms; T-SLEEP-TOGGLE A/B/C; `sr_region_store`'s FM fold and arms (cut Q1) |
| C1d `7a7f7fa0` | `bench(physics): FM (1d) - the runner's --friction and its armed layout census; colored_solve's fm_depth_ab` | the parity runner's `--friction`, `config.friction_model`, the armed `layout_census`; `colored_solve`'s `fm_depth_ab` group |
| C1f | `docs(physics): FM (1f) - C1 record …` | this directory |
| r1 `a7b3cb39`, `8220e92c`, `823041ff`, and this file's §7 | `test(physics): FM r1 …` | round 1's fixes (triage r1 F1, F2, F3, W1); test files only, see §7 |
| sync `190aedce` | `merge: integ/unified into u/phys-fm-c1 (trunk sync)` | the trunk with `sl` (`d3701e6b`) and render OBS; one conflict (a `use` list), both sides kept; §8.1 |
| C1e `e43e7411` | `test(physics): FM (1e) - frozen_island_warm_start's FM arms on SL's B1-exact re-expression` | the FM twins of B1-exact (both centres) and of the crowd test; test file only; §8.2 |
| r41 `d025f93c` | `perf(physics): FM ruling 41 - restore the pre-FM inlining of the FM = false instances …` | `#[inline(never)]` on the two warm kernels; `seed` and `FillCtx::source` split per `FM`; §8.4 |
| r42 `2954ec62` | `perf(physics): FM ruling 42 - remedy D2: the kernels' patch blocks test …` | the four patch-block sites of `solve_color_k` / `solve_color_avx2_k` re-test the const; §8.5 |
| sync 2 `f1d0fbd0` | `merge: integ/unified (0dac974f) into u/phys-fm-c1 (trunk sync 2, ruling 42 …)` | render RD1 over the sl merge; no conflict, no `boyko_physics` file; §8.5 |

**Close (§8):** `sl` merged; the trunk sync (`190aedce`) and C1e (`e43e7411`) are in, every pin
byte-identical on the synced tree. The first codegen receipt (cut §3.8) was RED with a STOP on Tier
1's status rule (§8.3). Ruling 41's fix (`d025f93c`) restored the pre-FM inlining; the amended
receipt then STOPped on `solve_color_dispatch::<false>`'s residue (§8.4). Ruling 42 applied the
measured remedy D2 (`2954ec62`), and the lane synced again over render RD1 (`f1d0fbd0`). **The final
receipt PASSes (§8.5):** Tiers 0 and 1 are exact against PARENT `0dac974f`, and every other residue
is inside rulings 41/42's admitted classes. No value moved.

---

## 2. As built — where the decisions live

| decision | symbols |
|---|---|
| D12 switch | `resources::FrictionModel`, `PhysicsConfig::friction_model`; `solve_colored_inner` (`let fm = …`, the centre into `FmFill`) |
| D-C1 instances, picks at callers | `solve_color_k`, `solve_color_avx2_k` (`<SPEC, K3, FM>`), `solve_color`, `solve_color_avx2`, `solve_color_dispatch`, `solve_all_colors`, `warm_apply_scalar`, `warm_apply_avx2`, `fill_range`, `fill_cohort`; picks in `solve_colored_inner`, `build_columns`, `SolveStages::{fill_block, run_block}` |
| D7 layout | `CohortColumns::layout(graph, fm)`; `RankBlock::{patch_triple, set_patch_triple}`; `CohortColumns::census` |
| D3 / D8 geometry | `patch_geometry` (scalar), `simd::patch_geometry_x8::<TOUCH>`; `fill_patch_geometry`, `fill_patch_geometry_x8`; `FrictionCentre`, `FmFill` |
| D5 twist | `contact::{twist_mass, max_sel, min_sel}`, `BodyEffective::apply_angular_impulse`; `simd::{twist_mass_x8, apply_angular_blend_x8}` |
| D9 warm records | `WarmRecord::{friction, set_friction, set_normal, has_fid}`; `WarmLookup::{friction_seed, carry_fm}`; Lemma W-F in `warm_records.rs`'s module doc |
| D10 warm apply | `warm_apply_patch_scalar`, `warm_apply_patch_x8` |
| D11 store | `store_solved_fm` (out of line); `store_and_swap` and `store_block` pick it under `fm` |
| D14 diagnostics | `ColoredSoftStepSolver::{friction_seed_stats, for_each_friction_seed, layout_census}`; `FrictionSeedStats`, `LayoutCensus`; `for_each_warm_seed`'s `[lambda_n, +0, +0]` under `stored_fm` |
| D15 toggle | `ColoredSoftStepSolver::friction_form_changed` (cold), `warm_records::zero_friction` (cold), `SleepSets::kept_fric` and `kept_friction_form_changed` (cold) |
| O2 (cut) | `capture_moved_in`'s `debug_assert!` on a run of at most one record under FM |

**Deviations from the cut** (each in the cut's addenda): the toggle tests write at `HELD_BY_LATE`
(under FM the Tree reuse-off cell holds from step 258, not by 150); RF-srs is an equivalent mutant
(a frozen manifold keeps its points in order, so `carry` = `carry_fm` there), re-aimed as RF-srs'
and RF-srs''; N-T2's red is on the per-point `seeds`, not `fseeds`; M12, M-C3 and M11 are
equivalent mutants (derivations in addendum 1 W5 and addendum 2); RF-b0's S-c stays green (every
S-c cohort is depth 1, D16); RF-i leaves DA9 green (SleepSets' prologue still reads the model),
confirmed by RF-i'; M5 re-aimed as M5b.

---

## 3. Gates and tallies

All on msvc (`stable-x86_64-pc-windows-msvc`), `CARGO_BUILD_JOBS=2`, one test process at a time.

**Every commit:** `cargo check` and `cargo clippy --workspace --all-targets -- -D warnings` clean;
UG-10 `internal_docs_anchors` 24/24; `ignore_reasons_census` 12/12 (399 sites, 1881 `.rs` files —
no FM test is ignored).

**C1a:** step_inputs debug 5/5, release 4/4; DA9 3/3 both profiles; P1 and P3 unchanged.

**C1b** (both profiles unless noted): `boyko-physics --lib` debug 356 passed / 1 ignored, release
354 / 1; P1 release `0xe5d8a7d5f4694f08` / `0x78121c47f5c6e8b2` / `0xb583189fa681f3a6` /
`0xa38620b38cbca8d3`, debug `0x211297c3d16c603c` / `0x0fc5719433dad377` / `0x839d94268d67b09f` /
`0xc7eb531b1e1ac19b`; P2 A7-R1 1/1 (release, its pinned bits); P3 3/3; P6 "GATE: GREEN - 19
scenes"; P7 2/2; P8 1/1; `v2_speculative_d0_identity` 10/10; `v2_speculative_contacts` 8/8;
`colored_acceptance_o5` 9/9; `sdf_collision` 13/13; `frozen_island_warm_start` 6/6 (unedited);
`sr_region_worker_invariance` 1/1; `sr_region_store` 1/1; `sr_region_counters` 3/3;
`friction_manifold` 21/21; `default_world_worker_invariance` 3/3; DA9 3/3 (`friction_model`
Observable on all three shapes). Miri (nightly msvc): G-S4
`patch_scalar_workers_write_disjoint_lanes` passes Stacked Borrows and Tree Borrows (its mutation
M-S4 reds both); the other named Miri units (cut addendum 1, W4: `patch_width1_lanes_equal_per_point`,
`patch_probe_scalar_equals_x8`, the SB-route FM region unit with `simd_solve` off and on) are the
tester's legs.

**C1c:** `sleep_skip_bit_identity`, the whole file: debug 63/63, release 63/63 (the five FM
tests among them; J-Son FM W1 holds 2 246 775 kept triples with a non-zero lambda_tw over its run);
`sr_region_store` 2/2 both profiles. No source file changed.

**C1d:** runner exe (parity profile) built; P4 `runner_pins` 12/12; P5 `pose_gates_v2` and
`pose_gates_d0` each: both negatives exit 4, runs 264/264, checks 26/26 and 17/17, `ALL_DONE`; P11
`ours_gates` 49/49, `s5_runner_gate` 54/54; the FM receipts 21/21 — (1) `--friction point` = the
JT500 V2 fixture's pose bytes, its SUMMARY intact; (2) manifold != point after 100 steps; (3) W
2/4/8/16 = W 1 pose bytes after 500 steps under `manifold` and `manifold-touch`; (4) armed W 8, 200
steps: `manifold` cohorts 195 931 = sum of `cohorts_by_depth` [168, 0, 100, 195 663], `patch_blocks`
195 763 = the depth >= 2 cohorts; `point` `patch_blocks` 0; (5) `--solver reference` refuses
`--friction` (exit 2); receipt 6, `colored_solve -- --test fm_depth_ab` (criterion's test mode, no
timing read): 16 rows, 16 `Success`. A pre-commit defect (the census argument written before the S5
dispatch arguments in the SUMMARY's positional format, shifting every later key) was found on the
P4 rows' SUMMARY and fixed before any gate was counted; receipt 1b and RF-d5 now guard it.

---

## 4. Friction-behaviour readings (C0's gates under FM, both centres; `friction_manifold.rs`)

Every C0 bound is met by both centres: G-P1 incline stick / slide / acceleration, G-P2 twist stop and
hold (and the COM drift bound), G-P3 yaw while sliding, G-P7 single-box slide; G-P4 slide-and-spin
stop ratio 0.788 under both centres; the G-P6 edge-resting spin receipt stops in 29 steps (C-ALL),
27 (C-TOUCH) and 31 (per point). G-P5 (count-1 scenes) keeps its per-point pins under FM in every
cell; G-P5w (625 spheres in one layer, the region at W 8) equals per point on every step.

---

## 5. Red-first evidence (predicted before each run)

C1a: RF-a1, RF-a2, RF-a3 red as predicted. C1b: RF-b0, M1, M2, M3, M5b, M6, Mmu0, M7, M8', M9, M10,
M15, M-T1, M-T2, M-T3, N-IDEM, N-T1, RF-i (+ RF-i'), N-D1, N-D1b, M-D16, M-FMA x2, M-S4 (Stacked
Borrows and Tree Borrows) red; M12, M-C3, M11 equivalent. C1c: N-W2, N-T2, RF-O1, RF-vac,
RF-srs' and RF-srs'' red; RF-srs equivalent. C1d: RF-d1 (receipt 2: A == B), RF-d2 (receipt 4 on
both arms), RF-d3 (receipt 5 exits 0), RF-d4 (`fm_depth_ab`'s anti-vacuity at w = 2) and RF-d5
(receipt 1b) red. The outputs are quoted in each commit's message and in
`fm/mut/c1{b,c,d}_results*.md`.

---

## 6. Parked, and what C2 reads

- **C1e, the trunk sync, the codegen receipt:** done after `sl` merged (§8). The first receipt
  stopped at Tier 1's status rule (§8.3). After ruling 41's fix, the amended receipt stopped on
  `solve_color_dispatch::<false>`'s residue (§8.4). After ruling 42's D2 and the second sync, the
  final receipt passes (§8.5).
- **C2 inputs:** the runner's `--friction point|manifold|manifold-touch` and, on armed colored rows,
  `layout_census` (`steps`, `cohorts`, `cohorts_by_depth`, `rank_slots`, `patch_blocks`,
  `multi_lanes`, summed over the metric window) — SCREEN-L's K1 reads `cohorts_by_depth`;
  `colored_solve`'s `fm_depth_ab` rows (depths 1-4 x {PerPoint, PerManifold} x {single, 4 workers})
  for the timed A/B (design Q3), outside the lane.

---

## 7. Round 1 (tester r1, review r1, triage r1): the fixes

Four items were confirmed (`fm/triage_r1.md`). All four are test-only, and no production byte
changed. Each red-first was predicted before its run (`fm/tools/r1_*_mutations.py`,
`fm/mut/r1_results.md`, `fm/logs/mut/r1-*`). Every mutated file was restored by content, md5
checked. The branch first took a trunk sync over two `boyko_ui` test commits (`6e8912a7`, no
physics file). That is not the trunk sync of §6 and §8.1 (`190aedce`), which came after `sl`'s
merge.

- **F1** (`a7b3cb39`): the FM arms of `friction_manifold.rs` could pass with no FM engaged.
  - Pre-fix, the harness's model write was dropped (r1-F1-pre): `running 21 tests`, 21 passed, and
    G-P4 read "stop step 52, ratio 1" for both forms.
  - Two harness premises now close it. The route premise (`Sim::step`): on every colored step,
    `friction_seed_stats().is_some()` equals the arm's form. The engagement premise (`trace_on`):
    the box holds a manifold of two points or more on some step, and every such step lays out a
    friction patch (`layout_census`).
  - Reds, each against 21 tests:
    - r1-F1-route (write dropped): 11 FM tests red, 10 ok.
    - r1-F1-route-rev (every written model `PerManifold`): the 10 tests with a per-point colored
      arm red, 11 ok.
    - r1-F1-engage (the census's patch term at 0): the 9 FM behaviour tests red, 12 ok.
    - r1-F1-multi (the multi-point test at 5): the same 9 red, 12 ok.
  - Not caught: the route premise reads the form bit, not the centre. A harness that turned
    `PerManifoldTouch` into `PerManifold` stays green (r1-F1-route-rev's Touch arms).
    The solver exposes no read of its centre.
- **F2** (`8220e92c`): `sr_region_sb_route_runs_every_kind_on_both_threads_fm` was red in both
  Miri models, because `warm` had one block.
  - A colour's warm entry cuts its cohorts at a point quota (`cohort_cuts`). With two-point stacked
    pairs, colour 0's only cut fell after its last cohort.
  - The stacked pairs now hold three points. Both models pass, with `simd_solve` off and on.
- **F3** (`8220e92c`): `patch_probe_scalar_equals_x8` was red in both Miri models.
  - At 2 cases the coverage read `touch(of 4) [0, 1, 2, 2, 0]`.
  - Miri now runs 3 cases, which reach every named case (`[0, 1, 2, 4, 1]`). Both models pass.
  - Native runs keep 160 cases.
- **W1** (`823041ff`): eight headers now name the mutants measured red on them.
  - The two gates with no receipt are now shown able to fail. M2 (the tangent limit at 2μ) reds
    `g_p1_incline_accel_fm` (acceleration −2.8e-7 against [1.73, 1.91]) and
    `g_p3_yaw_while_sliding_fm` (stop distance 0.758 m against 1.529 m ±5 %).
  - Corrected: G-P1 stick (M1), G-P1 slide (M2, not M1), G-P2 drift (M5b), G-P3 (M2; M5 leaves
    it green).
  - `patch_probe` names M15 and M-T2. M-T1 and M-T3 were run only on the hand cases, which name
    them.
  - M11, M-C3 and RF-srs are now marked equivalent in their headers, and `sr_region_store` names
    RF-srs' and RF-srs''.
  - The derivations behind the three equivalent mutants:
    - **M11** (plain `f32::max` / `min` in the scalar twist clamp). `maxnum` / `minnum` lower on
      x86-64 to the compare-selects of `max_sel` / `min_sel`. These are equal bit for bit on every
      finite and ±0 input, and differ only on a NaN limit, which finite inputs never produce. The
      selects stay, because IEEE leaves the ±0 tie unspecified.
    - **M-C3** (the twist row at width 1). At width 1 the twist's limit is `+0` and its Δλ `+0`, so
      it adds only signed zeros to ω. A `−0` spin component reaches it only if it survived the
      lane's warm apply and normal row unchanged, which the gates' scenes never produce.
    - **RF-srs** (the region's pass 2 with the per-point `carry` under FM). A frozen manifold keeps
      its points in order, so `carry` copies `carry_fm`'s record bit for bit.

**Round 1's gates** (msvc; logs in `fm/logs/`):
- `friction_manifold`, release and debug: `running 21 tests`, 21 passed.
- `sr_region_store`, release: 2/2.
- Native `patch_probe`, release: ok, with coverage `[6, 70, 199, 229, 89]` as before.
- Miri SB and TB, the two units: `running 1 test`, ok in each leg (kit gate `fm/k/r1-miri-1`,
  `VERDICT GREEN exit_code 0`).
- `cargo check` and `clippy --workspace --all-targets -D warnings`: clean.

---

## 8. Close (2026-10-12): the trunk sync, C1e and the codegen receipt

The close run's report is `fm/close_dev.md`; its logs are `fm/close/` and the receipt's files
`fm/cg/` (all under `D:/tmp/phys-orch/`).

### 8.1 The trunk sync (`190aedce`)

`integ/unified` = `d3701e6b` carries `sl` (SL-2a, SL-1, C2b, SL-1b; rulings 39 and 40) and render
OBS (`410c30ef`, no physics file).
- One textual conflict: `solver/colored.rs`'s `use crate::resources::{..}`. Both names were kept
  (FM's `FrictionModel`, SL's `SealArm`).
- Read hunk by hunk where the cut expected conflicts, and auto-merged:
  - the runner: FM's `--friction` refusal is in `validate`, SL's lift in `validate_dyn`. SL's
    `sleep` member is a named argument at the SUMMARY's end, so FM's positional `layout_census`
    argument keeps its slot;
  - `solve_colored_inner`: SL's seal argument, `begin_step_armed`, the SL-INV capture assert and the
    settle, beside FM's form latch, D15's cold zeroing and `fm_fill`;
  - `resources.rs`, `sleep_sets.rs`, `colored_tests.rs` (SL's `PIN_S_B` re-pin);
  - `sleep_skip_bit_identity.rs`: SL's LB5 re-expression beside FM's DA9 row, `Obs.fseeds` /
    `fstats` and FM arms.
- `sl`'s values are the reference after the sync (cut §4): `PIN_S_B` `0xe9bd_c7f6_80f7_80ca`;
  JSonT500 `0xc637_a051_7ada_bab7`, JSonToff500 `0x4aec_2a5f_78f4_88d0`; the E3 sleeping-on
  fixtures. Cut §4 compares them at the sync merge against the lane's HEAD.
- Three first-parent commits after the sync touch `crates/boyko_physics`
  (`git log --first-parent 190aedce..HEAD -- crates/boyko_physics`). C1e edits a test file; r41
  and r42 edit source:
  - C1e (`e43e7411`): the one test file, `frozen_island_warm_start.rs` (§8.2).
  - r41 (`d025f93c`): `solver/colored.rs`, `solver/warm_records.rs` and `solver/colored_tests.rs`.
    It puts `#[inline(never)]` on `warm_apply_scalar` / `warm_apply_avx2`. It also gives
    `WarmLookup::seed` and `FillCtx::source` a const `FM` parameter that their bodies do not read
    (§8.4). Value-neutral: an inlining attribute and an unread const parameter change no
    arithmetic. The final receipt reads the per-point bodies exact against PARENT: Tier 0 `carry`,
    and Tier 1's two warm kernels and `fill_range::<false>` (§8.5).
  - r42 (`2954ec62`): `solver/colored.rs`. The four patch-block sites of `solve_color_k` /
    `solve_color_avx2_k` test `if FM && patched` (§8.5). Value-neutral: `patched` already implies
    `FM`, so the condition is unchanged in both instances.
  - The second sync (`f1d0fbd0`) touches no `boyko_physics` file.
- The sleeping-on values did not move. The tester's close run (`fm/close_test.md` §(3) and F-1;
  every run `dirty 0`) was on HEAD `72f8ecfc`, which contains r41 and r42. It reads:
  - P10 4/4: JSonT500 `0xc637a0517adabab7` and JSonToff500 `0x4aec2a5f78f488d0`, both `holds`;
  - `sleep_skip_bit_identity` 63/63 in both profiles;
  - P5's V2 and d0 sets, 264/264 each, with the armed sleeping-on rows among the runs;
  - `PIN_S_B` (`g1_pins::g1_s_b_sleep_wake_pins`) ok in the lib run of both profiles.
- `v2_speculative_fidelity.rs` and `sleep_skip_bit_identity.rs`, which carry the anchors, are
  unchanged since `190aedce` (`git diff --stat 190aedce HEAD` on the two is empty).

Gates on the synced tree (`fm/close/logs/sync/summary.txt`, 53 rows, every exit 0):
- check and clippy (`--workspace --all-targets -D warnings`): clean.
- `boyko-physics --lib`: debug 366 run (365 passed, 1 deferred ignore); release 364 (363 + 1).
- Pins:
  - P1, both profiles: the trunk's hashes.
  - P2: A7-R1's D_max 0.0007244835 / 0.0005266388 / 0.0006670445 m. It runs inside
    `sleep_settles_box_piles`, which reads 16/16 with its generator ignored.
  - P3 3/3 and P6 "GATE: GREEN — 19 scenes", both profiles; P7 2/2; P8 1/1.
  - P9 49/49. The ledger's last row is now `OBS-C6 5 7`, so the pool-site columns are unchanged.
  - P10 4/4 at the synced anchors.
- Both profiles: `v2_speculative_d0_identity` 10/10, `v2_speculative_contacts` 8/8,
  `colored_acceptance_o5` 9/9, `sdf_collision` 13/13, `frozen_island_warm_start` 6/6,
  `sr_region_worker_invariance` 1/1, `sr_region_store` 2/2, `sr_region_counters` 3/3,
  `friction_manifold` 21/21, `default_world_worker_invariance` 3/3, `sleep_skip_bit_identity`
  63/63.
- `sl`'s own tests over the merged solve path, both profiles: `sleep_velocity_zero` 4/4,
  `sleep_write_wake` 6/6, `sleep_seal` 17/17, `row_identity_remap` 12/12.
- The runner:
  - P4 12/12;
  - P5, both sets: runs 264/264, both negatives exit 4, checks 26/26 and 17/17, `ALL_DONE`;
  - P11: `ours_gates` 49/49, `s5_runner_gate` 54/54;
  - the FM receipts 21/21, and `fm_depth_ab` in test mode.
- UG-10 24/24; `ignore_reasons_census` 12/12.

### 8.2 C1e (`e43e7411`)

`frozen_island_warm_start.rs` gains three tests. The harness now takes the friction model;
the per-point tests pass `PerPoint`, so their assertions are unchanged.

- `a_woken_pile_steps_exactly_as_its_twin_stepped_from_the_frozen_state_fm` and `_fm_touch`: SL's
  B1-exact scenario with both worlds on `PerManifold` / `PerManifoldTouch`. Premises come first:
  - the sleeper ran the per-manifold form on every frozen step, and both worlds on the wake step;
  - the twin laid out a patch (`patch_blocks` 2);
  - the first frozen step carried a triple for each of the column's 5 manifolds;
  - the twin's wake step was warm at every point, with `manifold_hits == manifolds` (5 == 5).
  Then the woken pile's state equals the twin's step from the frozen state, bit for bit.
- `frozen_islands_carried_beside_a_parallel_solve_are_bit_identical_across_worker_counts_fm`: W1
  and W8 are bit-identical on every step, in bodies, warm-seed statistics and friction-seed counts.
  - Every tallied step ran FM (154/154). `patch_blocks` reached 8. The solver's own friction carry
    reached 64 triples on a step.
  - The wake's `manifold_hits` equals its manifolds (88).
  - Reported, not asserted: no physical friction carry fell on a step where a colour reached the
    dispatch floor.
- `manifold_carry_hits` is physical (cut Q2). On held steps the held store keeps the records, so the
  count reads 0 there. The arms' first unmutated run asked more and was red on that premise. The
  corrected premises ran before the red-first.
- Red-first M8 (`carry_fm` drops the triple), predicted before the run: 9 tests, 7 passed, 2 FAILED.
  The two failures are the FM exactness arms, at the exactness assertion. The first difference is
  box 0, field 0, and every premise held.
  - Design §8 (f)'s "`manifold_carry_hits` changes" was withdrawn before the run. The count is of
    carries with a point hit, which M8 leaves alone.
- Gates: the file, release 9/9 and debug 9/9; clippy on the target, clean.
  - P2, P10 and P5's sleeping-on rows ran on the synced tree. C1e changes no engine or runner byte.

### 8.3 The codegen receipt — RED, STOP (`fm/cg/receipt.md`)

PARENT = `git merge-base HEAD integ/unified` = `d3701e6b`; TIP = `e43e7411`. Both were built as cut
§3.8 step 2 (release, no LTO, one CGU, exports deleted), with the tiers and addendum 3's note as
pre-registered.
- **Step 4:** PARENT has no out-of-line `solve_color{,_avx2}{,_k}`, as predicted.
- **Tier 3 passes:** all 16 region-monomorph functions are equal, and `disarmed.py` reads PASS.
- **Tier 1:**
  - `solve_all_colors` ↔ `::<false>` is equal.
  - **STOP:** `warm_apply_scalar` and `warm_apply_avx2` are out of line in PARENT and have no symbol
    in TIP. TIP outlines `warm_start_apply` (2724 instructions) and inlines all four kernels in it.
  - `fill_range::<false>` differs (1096 → 814 instructions): `FillCtx::source` is now out of line.
  - `solve_color_dispatch::<false>` differs (4851 → 4852 instructions, 120 hunks). No symbol
    reference changed: register allocation and scheduling only. *(Withdrawn in §8.4: the tool's
    normalisation hid a changed constant.)*
- **Tier 0 red:** `WarmLookup::carry` now calls an out-of-line `WarmLookup::seed`.
- **Tier 2 red:**
  - the warm-kernel calls moved into `warm_start_apply`;
  - the inlined `solve_all_colors::<true>` copy in `solve_colored_inner` raises its zone-handle
    references (forbidden by rule; FM arm only);
  - bounds-check references rose.
  - `solve_region`, `broadphase_sets` and the three constructors pass.
- **Addendum W1's "every other function" red:** `Resources::insert` and
  `insert_physics_resources` of the solver (its size went 3576 → 3584), and the `resource_id`
  initialisers of `PhysicsConfig`, `SleepSets` and the solver (their `TypeId`: rustc hashes an
  ADT's fields). `FillCtx::source` and `WarmLookup::seed` show as new symbols called from Tier
  0/1 bodies.
- **Tier 4 red:** 15 removed and 67 unclassified added lines, at the cut's own caller-side picks and
  at the FM-only parameters and locals of the `<const FM: bool>` instances.
- **Red-first:** R1 (built), R3, R4 and R5 red as predicted. R2 is not applicable, because TIP has no
  `warm_apply_avx2::<false>`.

Three causes cover every red, and none moves a value:
1. A helper that gained a second caller or instantiation under FM is now outlined by LLVM:
   `seed`, `FillCtx::source`, `warm_start_apply`.
2. The FM fields change three resources' size and `TypeId`.
3. Tier 4's named transformations do not cover the picks the cut placed at the callers.

The lane stops here and makes no change. Any fix needs an architect's decision:
- inlining attributes on the outlined helpers (principle 7: this receipt is the measurement);
- or a receipt amendment for causes 2 and 3.

### 8.4 Ruling 41: the inlining restored, the receipt amended and re-run — RED, STOP (`fm/cg/receipt2.md`)

Ruling 41 (`phys-next/RULINGS-2026-10-01.md`, item 41) set three steps:
1. Restore the pre-FM inlining of the FM = false instances, with the least attributes.
2. Amend the receipt for causes 2 and 3 before re-running.
3. Admit a Tier 0/1 residue that changes no symbol reference. A residue that changes a reference on
   the FM = false path stays a STOP.

The run's report is `fm/close_dev2.md`; its files are `fm/close2/` and `fm/cg/`.

**The fix (`d025f93c`).** The candidates were measured on lib asm built from the working tree,
exactly as the receipt builds it:
- `#[inline(never)]` on `warm_apply_scalar` and `warm_apply_avx2`. Every instance is out of line
  again: the FM = false ones are 251 and 818 instructions, equal to PARENT's. `warm_start_apply` is
  inlined into its two callers again.
- `WarmLookup::seed` and `FillCtx::source` gain a `const FM: bool` that their bodies do not read.
  The per-point callers take the `false` instance, so it keeps exactly its pre-FM callers and LLVM's
  pre-FM decision:
  - `carry` is 644 instructions, equal to PARENT;
  - `fill_range::<false>` is 1096, equal to PARENT.
- `#[inline(always)]` on `seed` and `source` was measured first and rejected: `carry` kept 7 hunks
  and `fill_range::<false>` 176.
- Gates at the default (cut C1b (a), both profiles) are byte-identical to the synced references:
  - lib 366 / 364 run;
  - P1 `0x211297c3d16c603c` / `0xe5d8a7d5f4694f08`, with the reuse-off and d0 hashes unchanged;
  - P2's three D_max; P3; P6; P7; P8;
  - `d0_identity`, `v2_contacts`, `o5`, `sdf`, `frozen_island_warm_start`, `sr_region_*` and
    `default_world_worker_invariance`;
  - `friction_manifold` 21/21, with the count-1 pins.
  - Check and clippy are clean; UG-10 24/24; census 12/12.

**The amendment** (cut addendum 5 and its correction 1, written before the re-run):
- **M1, the normalisation.** A displacement is now a number not glued to a name. The first form read
  `__real@3f800000(%rip)` as `__real@3fD(%rip)`, which hid any changed constant in a Tier 0/1 body.
  This withdraws §8.3's "no symbol reference changed" for `solve_color_dispatch`.
- **M2:** Tier 4 pairs a removed line with the FM = false copy first.
- **M3:** the output names carry an index.
- **M4:** an aid that decodes panic `Location`s.
- **B (ruling 41 (2) (b)):** nine named functions are admitted when their census is unchanged
  except for constant-pool names. They are `Resources::insert`, `insert_physics_resources` and the
  `resource_id` initialiser, for each of the three types whose fields FM changed.
- **C (ruling 41 (2) (c)):**
  - `fm-pick` admits 72 lines, traced to the cut's named picks under 15 pick ids;
  - `r41-attr` admits the ruling's two attributes;
  - every other line stays red.
- **D (ruling 41 (3)):** a Tier 0/1 residue is admitted only when both hold:
  - it equals, byte for byte, the first receipt's `solve_color_dispatch` residue, pinned under M1;
  - no changed line names a symbol, a constant or rodata.
- **E:** each Tier 2 red is classed by path.

**The re-run.** PARENT `d3701e6b` was rebuilt, and its md5 equals the first receipt's
(`5cccad2e…`). TIP is `d025f93c` (asm md5 `dbcf3783…`).

| tier | reading |
|---|---|
| step 4 | PARENT has no out-of-line kernel symbol, as before |
| 0 `carry` | exact, 644 / 644 |
| 1 | `solve_all_colors`, `warm_apply_scalar`, `warm_apply_avx2` and `fill_range` are exact, and the status rule holds. **`solve_color_dispatch::<false>` is RED — a STOP:** its residue is the pinned one (4851 → 4852 instructions, 120 hunks), but 7 changed lines name constants. `__xmm@00000007000000060000000300000007` (a `vpermps` index vector) goes 0 → 1. This is a different SLP vectorisation, not register allocation. |
| 2 | `run_block`, `solve_region`, `broadphase_sets` and the constructors pass. `solve_colored_inner` is red; its reds are classed in the two items below the table. |
| 3 | 16 / 16 equal; the disarmed walk PASSes |
| every other function | 5 differ, and all 5 are B-admitted; the FM-only functions are ok |
| 4 | PASS: 72 `fm-pick` lines + 2 `r41-attr`, 0 table entries unused |

`solve_colored_inner`'s Tier 2 reds, by path:
- **FM = true path:**
  - the inlined `solve_all_colors::<true>` copy, which mirrors PARENT's own inlined `::<false>` copy:
    `PHYS_COLOR_WIDE` / `NARROW`, `zone_id`, the `LANE` TLS ×2, `LANES`, `CELLS`, one `ARM_MASK`,
    and four bounds checks at `solve_all_colors`' lines;
  - `build_columns`' FM reduce: one `slice_index_fail` in an fm region.
- **FM = false path:** six more `__imp_ARM_MASK` loads. The substep loop's zone opens reload the
  profiler's arm-mask address where PARENT held it in `%r12`. That is register allocation: it adds
  no zone, call or panic site.

**Red-first:**
- R1, built end to end: 4862 instructions, 133 hunks, "not the pinned one". TIP and R1-TIP differ in
  that function alone.
- R2 (now applicable), R3, R4 and R5 red as predicted.
- The amendment's RA-b1, RA-b2, RA-pin, RA-c1, RA-c2' and RA-attr red as predicted.

**The cause, and a measured remedy** (scratch exports, not committed):
- **D1:** with the FM code deleted from `solve_color_k` and `solve_color_avx2_k`,
  `solve_color_dispatch::<false>` equals PARENT (4851 instructions, 0 hunks). So the residue comes
  from the kernels' dead FM code:
  - `let patched = FM && depth >= 2;` is a runtime local;
  - so `if patched { … }` reaches LLVM in the FM = false instance;
  - it folds away there, but it still changes the SLP vectorizer's choices.
- **D2:** `if patched {` becomes `if FM && patched {` at the four sites.
  - `solve_color_dispatch::<false>` then equals PARENT, and every other Tier 0/1 body stays exact.
  - Against `d025f93c`'s asm, only that one function differs; the FM = true copies are unchanged.
  - rustc does not lower a block behind a switch on a const parameter that monomorphisation makes
    dead.

**The lane stops** (resolved by ruling 42, §8.5). Ruling 41 (3) makes a residue that changes a reference on the FM = false path a
STOP. D2 is a source change in the hot kernels, outside step 1's inlining scope. The `ARM_MASK`
reloads are a register-allocation count change in a Tier 2 body on the FM = false path. Both are the
orchestrator's call.

The trunk has since moved to `0dac974f`, render RD1. It touches `boyko_app`, `boyko_rhi_vulkan` and
`boyko_shaderdsl`, and no file of `boyko_physics`. `boyko_physics` depends on `boyko_shaderdsl`
through `boyko_sdf_math`. RD1's change there only adds lines: a new depth leaf, its emitter and a
bin. Whether the lib's asm moves at the next sync was not measured; cut §3.8 step 1 re-takes the
receipt after that sync.

### 8.5 Ruling 42: remedy D2, the second trunk sync, the final receipt — PASS (`fm/cg/receipt3.md`)

Ruling 42 (`phys-next/RULINGS-2026-10-01.md`, item 42) set four steps:
1. Apply D2.
2. Admit the six `ARM_MASK` reloads as register allocation: "changes a reference" means a new
   referenced symbol or a new reference kind, not more loads of an already-referenced address.
   FM-ZC in window IB(+FM) insures them.
3. Accept the `fm-pick` table's broad reading (`K`, `FC`).
4. Sync with the trunk again and re-take the receipt on the new merge-base.

The run's report is `fm/close_dev3.md`; its files are `fm/close3/` and `fm/cg/`.

**D2 (`2954ec62`).** The four patch-block sites of `solve_color_k` and `solve_color_avx2_k` test
`if FM && patched {`; a comment at both `let patched` sites says why and cites the receipt. The
const switch is resolved at monomorphisation, so the FM = false instance never lowers the blocks.
`patched` already implies `FM`, so no value moves in either instance.
- Gates at the default (cut C1b (a), both profiles) are byte-identical to the synced references
  (kit `fm/close3/k/d2-verify-1`, 31 / 31 runs exit 0):
  - lib 366 / 364 run;
  - P1 `0x211297c3d16c603c` / `0xe5d8a7d5f4694f08`, with the reuse-off and d0 hashes unchanged;
  - P2's three D_max (0.0007244835 / 0.0005266388 / 0.0006670445); P3; P6; P7; P8;
  - `d0_identity`, `v2_contacts`, `o5`, `sdf`, `frozen_island_warm_start`, `sr_region_*` and
    `default_world_worker_invariance`;
  - `friction_manifold` 21/21, with the count-1 pins.
  - Check and clippy are clean; UG-10 24/24; census 12/12.

**The second sync (`f1d0fbd0`).** `integ/unified` = `0dac974f` adds render RD1 on top of the sl merge.
It touches `boyko_app`, `boyko_rhi_vulkan` and `boyko_shaderdsl`, and no file of `boyko_physics`,
`boyko_sdf_math`, `Cargo.toml` or `Cargo.lock`. The merge has no conflict. Check and clippy are clean
on the merged tree.

**The amendment** (cut addendum 6, written before the sync and the builds):
- **(a)** Tiers 0 and 1 must be exact. The ruling-41 (3) pin is withdrawn.
- **(b)** `solve_colored_inner`'s census changes outside the Tier 2 table are admitted only as
  receipt 2's ten. Both PARENT's and TIP's bodies, funclets included, must equal receipt 2's, so the
  instructions classed there are the ones read now. The ten are:
  - on the FM = true path, the inlined `solve_all_colors::<true>` copy and `build_columns`' FM
    reduce;
  - on the FM = false path, the six `ARM_MASK` reloads.
- **(c)** Tier 4 is unchanged.

**The receipt.** PARENT = `0dac974f`; its asm is byte-identical to receipt 2's `d3701e6b` (md5
`5cccad2e…`): RD1 moves no byte of the physics lib. TIP = `f1d0fbd0` (md5 `71a86df8…`). It equals
the scratch D2 build function for function under the receipt's normal form.

| tier | reading |
|---|---|
| step 4 | PARENT has no out-of-line kernel symbol |
| 0 `carry` | exact, 644 / 644 |
| 1 | **all five exact**: `solve_color_dispatch` 4851 / 4851, `solve_all_colors` 115 / 115, `warm_apply_scalar` 251 / 251, `warm_apply_avx2` 818 / 818, `fill_range` 1096 / 1096; the status rule holds |
| 2 | `run_block`, `solve_region`, `broadphase_sets` and the constructors pass. `solve_colored_inner`: the table's 8 rises plus the ten carried changes, admitted (equal to receipt 2's, on receipt 2's bodies) |
| 3 | 16 / 16 equal; the disarmed walk PASSes |
| every other function | 5 differ, all B-admitted; the 15 FM-only functions are ok |
| 4 | PASS: 72 `fm-pick` lines, 0 table entries unused; 2 `r41-attr`; D2's lines are comments and fm regions |

Verdict: PASS, exit 0. TIP has 176 666 instructions to PARENT's 162 485.

**Red-first**, each red as predicted:
- R1, built end to end: Tier 1 `solve_color_dispatch` 4861 instructions, 13 hunks, "no pinned
  residue".
- R2-R5.
- The amendment's own:
  - RA6-exact: receipt 2's TIP reds on Tier 1 alone;
  - RA6-body: one register in `solve_colored_inner` → the carried set red;
  - RA6-arm: a seventh `ARM_MASK` reload → red.

**Not run here** (the tester's): §3.7's final-HEAD workspace and both-profile physics runs, P4/P5/P11
and the FM receipts on the runner, and Miri (no `unsafe` changed). The tester ran them on HEAD
`72f8ecfc`, GREEN (`fm/close_test.md`):
- the debug workspace run, 870 of 870 targets ok;
- the release leg of the 79 physics targets, all ok;
- P4 12/12; P5's two sets, 264/264 each; P11 49/49 and 54/54; the FM receipts 21/21;
- Miri SB and TB.
