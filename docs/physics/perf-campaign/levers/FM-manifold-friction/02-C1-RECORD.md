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

**Parked (cut Q5):** C1e (`frozen_island_warm_start`'s FM arms on `sl`'s B1-exact re-expression)
needs the `sl` lane merged; the trunk sync and the codegen receipt (cut §3.8) follow it. See §6.

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

- **C1e** waits for `sl`'s merge: FM twins of `sl`'s B1-exact re-expression and of the crowd test in
  `frozen_island_warm_start.rs`; red-first M8 (`carry_fm` drops the triple) at the wake step.
- **The trunk sync** (`merge --no-ff integ/unified`): the runner's `--friction` hunks union with
  `sl`'s SL-2a validate lift and SUMMARY keys; C1b's gates and P2/P10 re-run on the synced tree's pin
  values.
- **The codegen receipt** (cut §3.8, Tiers 0-4 with the pair map, R1-R5 red-firsts) is taken after
  the last trunk sync; addendum 3 pre-registers two more Tier 2/4 admissions
  (`for_each_warm_seed`'s diagnostic mapping, `use` lists gaining names).
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
physics file). That is not the sync §6 means, which still follows `sl`'s merge.

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
