# V2 — speculative contacts: the design record

Lane `u/phys-v2-spec`, cut from `16191fda` (the trunk merge of u/phys-w8s). This is the record of
what V2 builds and why, written before the code (commit C0); the results are appended at the end
(commit C7). The evidence is the F0 verdict and its F0d / F0e experiments (session scratch
`jolt-gap/f0/`: `VERDICT.md`, `speculative/`, `onset/`, `f0e/`); the owner rulings are
`phys-next/RULINGS-2026-09-30-V2.md`. The architect's cut (`v2-spec/cut.md`) and its critique are
folded in; where this record departs from the cut it says so.

## 1. Why

J-T (Jolt's `PyramidScene`, 1240 boxes, 0.5 m gap) collapses on landing on our engine: 0 of 32
trajectories hold (26 boxes past 0.5 m, the top box 475 mm off, 28.8°), where Jolt 5.6 holds (0,
51 mm, 0.54°). A contact existed only on overlap: the layers land at 3–12.5 m/s, 29 of 49 layer-8
boxes land on 1–3 of their 4 supports (the others 0–7 mm above), get kicked sideways, and the outer
ring slides off. With Box2D v3's rule — keep a point while its separation is at most `d`, solve a
positive separation as a speculative bias with no push — the same engine holds 16 of 16 (F0d, S5
and S20). The pose-hash gates cannot see any of this: they compare runs with each other.

**Owner (2026-09-30): V2a = yes** (speculative contacts become our contact rule), **V2b = 20 mm**
(Jolt's `mSpeculativeContactDistance`, Box2D v3.1's `B2_SPECULATIVE_DISTANCE`). Every contact pose
pin moves once, each by its own rule.

## 2. The rule

A contact point is kept while its separation `s <= d`, `d = PhysicsConfig::speculative_distance`
(default `DEFAULT_SPECULATIVE_DISTANCE = 0.02` m from C5; `0` before). The solver treats a point
whose current separation is positive as speculative: `bias = s / h`, mass scale 1, impulse scale 0,
in the biased pass and in the relaxation pass alike (Box2D v3 `contact_solver.c`); every other
point takes today's soft rule. **`d = 0` is the pre-V2 engine, bit for bit** — the value a
cross-window bridge runs (`--speculative-distance 0` in the parity runner).

### 2.1 Configuration

- `PhysicsConfig::speculative_distance: f32` — numerics-changing; finite and `>= 0`
  (debug-asserted where the narrowphase builds its step parameters, as `ReuseStep::new` does for τ).
  Latched once per step with the rest of the configuration (`StepInputs`, L10 D9b).
- L10's held-island epoch (`SleepEpoch`) carries its bits: a runtime change of `d` changes what a
  held island's pairs would produce under `Off`, so it restores every held island (DA9's row is
  Observable; red-first: the DA9 row added before the epoch edit reds).

### 2.2 Narrowphase — every pair type

The step's `d` rides in `ReuseStep` (already carried to both narrowphase paths); a pair with a
sensor on either side uses `d_eff = 0` (Box2D's sensors use no margin; otherwise every sensor would
report overlaps 20 mm early), and so does a sensor's SDF test.

- **Box-box**, five sites and the hysteresis sign (`narrowphase/box_box.rs`, `reuse.rs`):
  `separates` (`depth < -d`, which also drives `sep_still_holds`, L9a's carried axis);
  `refresh_edge` (`depth < -d`); the clip keep (`separation <= d`); reuse `refresh_face`
  (`separation <= d` — the same `d` as the clip keep, or a record would not refresh to its own
  contact); `patch_depth`'s fold starts at `-d` (not `NEG_INFINITY`: at `d = 0` only the sign of a
  zero differs, which its one consumer cannot see, and a NaN point keeps today's answer); the
  axis hysteresis compares against `hysteresis_floor(last) = last * 1.05` for a negative depth and
  `last / 1.05` otherwise (today's test inverts for negative depths; at `d = 0` a hint depth is never
  negative, so the `else` arm is today's bits, NaN included). `x < -0.0 == x < 0.0` and
  `x <= 0.0 == x <= -0.0` for every `x`, so each comparison is today's at `d = 0`. The public
  `box_box_contact` keeps its signature as the `d = 0` wrapper, so every narrowphase unit oracle
  (`box_box_fallback_depth.rs`, `narrowphase_census.rs`, A7-N11 against `pre_l9`) is unedited.
- **Sphere-sphere**: `separation >= d` drops the pair (keeping the site's strict operator).
- **Sphere-box** (both row orders): `dist >= r + d`; `sphere_box_contact` stays the public `d = 0`
  wrapper beside the `d` form.
- **SDF**: sphere `separation >= d`, box scalar and AVX2 `dist >= d` (the O9 arm stays
  manifold-equal to the scalar one).
- The thin-box lane's `BestFace` point (one speculative point, `s > 0`) exists at `d = 0` today. So
  the solver's speculative branch is keyed on `d > 0`, never on `s > 0` alone (below).

### 2.3 Broadphase — the margin at gather

Every broadphase calls `body_bounding_radius(&BodyState)` directly (AllPairs, Tree, Grid and the
brute-force oracles), so a margin passed at the call sites would edit the tree-c4 lane's files. The
one in-bounds route: `BodyState.bp_margin` (`= d / 2`, written by `physics_gather` into every row
it pushes), and `body_bounding_radius` returns `shape_radius + bp_margin` (`r + 0.0 == r`: every
pair set at `d = 0` is today's). The predicate `sphere_bound_feasible` and the broadphase kernels are
untouched. `physics_gather` reads the live configuration, one stage before the broadphase latches
it, so `bp_margin` is the one broadphase input read before the latch; nothing writes the
configuration between the two stages of a step (the same argument as `dt`, which the gather stamps).
L10's resting test compares `bp_margin`'s bits like every other `BodyState` field.

### 2.4 Solver — the speculative branch, current separation, no new stage

Per lane, in the scalar kernel and the AVX2 cohort kernel op for op, and in the reference
`SoftStepSolver`:

```
s0     = sep (gather time)
dA, dB = the bodies' accumulated step movement (dp: Vec3, dq: Quat); a sentinel / static /
         padding lane reads (0, IDENTITY)
s_cur  = s0 + dot(((dpB - dpA) + (rot(dqB, rb) - rb)) - (rot(dqA, ra) - ra), n)   -- no FMA
s_eff  = if K3 || s0 > 0 { s_cur } else { s0 }      -- K3 = CURRENT_SEPARATION_ALL_POINTS
spec   = s_eff > 0
d_lambda = spec ? -m * (vn + s_eff * inv_h)
                : today's formula on s_eff (soft bias in the biased pass, -m * vn in relax)
```

- Box2D v3's own form is `s = dot(dp + rot(dqB, rB) - rot(dqA, rA), n) + (s0 - dot(rB - rA, n))`;
  the relative form above is the same quantity without the cancellation of two large anchor
  rotations. `inv_h = 1 / h` is computed once per step in `f32` and handed to every kernel.
- **Bit identity at `d = 0` by construction.** The kernels are `const SPEC: bool, const K3: bool`
  instances; `d > 0` selects `SPEC = true`, anything else `SPEC = false`, whose body is today's
  instruction stream. In `SPEC = true, K3 = false`, a lane with `s0 <= 0` computes today's formula
  on `s0` with today's ops, so the per-lane select cannot change its bits. The fork is one branch
  per kernel call, inside the kernel function.
- **The step parameters reach the kernels as one `&SolveStep`** (`bias_rate`, `mass_coeff`,
  `impulse_coeff`, `inv_h`, `spec`, the delta view) in place of the three coefficients: a reference,
  so the parallel colour task keeps its 104 B capture and its scope's cell budget (the frame census
  pins one chunk per colour scope; a larger capture would move it at `d = 0` too). Pure forwarding
  through the dispatch functions; no dispatch decision changes.
- **Where Δ lives (Principle 0).** A `ScratchColumn<BodyDelta>` per solver (`BodyDelta { dp, dq }`,
  28 B, `#[repr(C, align(32))]`), NOT inside `BodyEffective` (exactly one 64 B line, read by every
  kernel gather). Two synthetic ids, placed clear of the solver cohort's stagger slots (the kernels
  and the integrate sweep it beside the body mirrors).
- **Who writes Δ: the existing integrate.** `simd::position_integrate_tracked` is
  `position_integrate_scalar` plus `dp += v·h; dq = dq.integrate(ω, h)` under the same guard; each
  solver calls it instead of the untracked one iff `d > 0`. No new stage, no new loop, no serial
  pass (F0d's per-substep serial pass is NOT shipped). Δ is reset to `(0, IDENTITY)` for every row
  once per step (iff `d > 0`). Rows the integrate skips (static, kinematic, L10's held rows, a
  frozen row's restore) keep `(0, IDENTITY)` or are never read by a laid-out lane.
- **Restitution guard.** Box2D skips restitution for a point whose total normal impulse is 0 ("the
  total normal impulse is 0 for speculative points"); without it a speculative point that never
  touches but carries `vn0 < -threshold` gets a bounce before any contact. `RankBlock` has no room
  for a total, so both solvers skip a point with `s0 > 0 && λn == 0` — only when `d > 0`, so `d = 0`
  is today's.
- **K3 — current separation for penetrating points too.** One `pub(crate) const
  CURRENT_SEPARATION_ALL_POINTS` in `solver/soft_step.rs`, beside `MAX_BIAS_VELOCITY`; both solvers
  pass it as the kernels' `K3`; the unit gates instantiate both values. Introduced `false` (the
  spec's default: speculative points only). **F0e(ii) decides it** (the owner's ruling: "included
  only if F0e(ii) shows it tightens the pile"): on J-T at `d = 20 mm`, K3 takes the ensemble's max
  drift from 189–260 mm (S20; 203–236 for the exact form on speculative points only, XS20) to
  106–153 mm with reuse on and 111–268 with it off, and the boxes past 0.1 m from 55–95 to 2–6, 8/8
  holding in both cells; on the harsher 1.0 m drop K3 turns 0/8 into 8/8 with reuse on
  (`f0e/report_gap05.txt`, `f0e/adv/verify.txt`). So C5 sets it `true` with the default flip, and
  every pin moves once.
- **Kinematic bodies.** The colored integrate does not move them, so a kinematic's Δ is 0 within a
  step; a speculative contact against a moving kinematic can close fully while the kinematic also
  approaches. Recorded as a limitation (PC-V2-9), not fixed here.

### 2.5 The reference solver

Under the default `d = 20 mm` the reference `SoftStepSolver` receives speculative points; with the
old rule its relaxation pass would treat them as rigid "ghost" contacts (F0d NS20: 0/8, bodies held
off the floor). It takes the same rule (its own Δ column, the tracked integrate, the shared scalar
helper, the restitution guard), so it keeps running the engine's contact rule.

## 3. Commits and their gates

Every commit: `cargo check` and `clippy -D warnings` over the workspace, `boyko-physics --tests`
in debug and in release (the release run is what executes the six `sleep_settles_box_piles`
tests that debug ignores: A7-R1, A7-R2, G2, G6, G7 and `simd_solve_on_off_bit_identical`),
`internal_docs_anchors`, `ignore_reasons_census`, and the trunk pins.

- **C0** this record.
- **C1** red-first, test only, on the parent: `v2_speculative_fidelity.rs` (the J-T gate: 8
  one-ulp seeds × contact reuse on/off × sleeping off/on, pass iff every cell holds 8/8 with no box
  past 0.5 m and max drift under 0.5 m; anchors = the runner's JT500 / JToff500 / JSonT500 fixtures;
  `deferred:` until C5, then `slow:`), RED on the parent; `v2_speculative_d0_identity.rs` (one small
  scene per pair type, per-step poses + manifold stream + sensor overlaps hashed and pinned, W1 and
  W8, each with an anti-vacuity count of its pair type), GREEN on the parent by construction.
- **C2** the field (default 0) + the narrowphase at every site + the sensor rule + the epoch + the
  runner's `--speculative-distance` flag; explicit `d = 0` arms added now and kept live across the
  flip (the pyramid ×2 at today's pins, A7-R1 reuse-on, GOLDEN's setup, `bp_query_counts`, the census
  S1f twin, the `support_loss` `e_*` arms whose premise needs `d < τ_eff`); narrowphase gates N1–N5
  at `d > 0`, each with its red.
- **C3** the broadphase margin; B1 (a sphere pair at 1.01 m centre distance, `d = 0.02`: every
  broadphase emits it and the narrowphase makes one speculative point; red before the margin) and B2
  (Grid == AllPairs with margins, proptest).
- **C4** the solver (both kernels, the reference, Δ columns, tracked integrate, restitution guard,
  `SolveStep`); S1 scalar == AVX2 with speculative lanes and Δ, both K3; S2 the landing; S3 the
  current separation (mutation `s_cur = s0` reds); S3-SDF (the sentinel's Δ_B must be `(0, I)`;
  mutation `Δ_B = Δ[ib]` reds); S4 the relaxation pass; S5 the restitution guard; S6 {1, N} and
  W 1/2/4/8/16 at `d = 0.02`; Miri on the kernels.
- **C5** THE value change: `speculative_distance = 0.02`, K3 = true; the fidelity gate RED at C4 →
  GREEN; every cargo pin it moves, re-derived by its own rule in the same commit.
- **C6** the window fixtures re-recorded (`docs/measurements/2026-09-30-v2-speculative/`), the pose-gate
  scripts for `d = 0` and for the V2 fixtures, the work receipts.
- **C7** the results appended here; the UG-10 anchors each commit moved are fixed in that commit.

## 4. Decisions on the cut's questions (Q1–Q11)

The cut's recommendations are adopted: Q1 the lock-set extension (only files in no other open lane's
set: `solver/simd.rs`, `scratch_ids.rs`, `sleep_sets.rs`, `step_inputs.rs`, `BodyState` and its
literal sites, the runner, `docs/OPEN-QUESTIONS.md` / `docs/SYSTEMS.md` line numbers, and the
colour dispatch's parameter forwarding); Q2 `BodyState.bp_margin`; Q3 the reference solver takes the
rule; Q4 the restitution guard; Q5 sensors at `d_eff = 0`; Q6 the new fixtures in their own
measurement directory, never overwriting window 8's or L9/L10/W6's (they are the `d = 0` references);
Q7 GOLDEN keeps its value through `speculative_distance = 0` in its setup and records the V2
default's value in its doc; Q8 K3 per F0e(ii), above; Q9 the flip and every cargo re-pin in one
commit; Q10 the `d = 0` arms listed in C2; Q11 zero cost when off is zero extra WORK (census, pose)
and the `SPEC = false` instruction stream, not zero instructions (the narrowphase compares against a
loaded `-d`, the gather adds `+0.0` per radius), priced by window 9's `V2-RES` row.

## 5. The critique (CHANGES_REQUESTED, 0 critical / 5 important), resolved

- **W1** — the release-only `sleep_settles_box_piles` tests are part of the `d = 0` proof: the
  release `boyko-physics --tests` run is a standing gate of every code commit (§3).
- **W2 (a)** — the G1 identity pins (`colored_tests.rs`, "a red pin is a defect, never a re-bless")
  drive the narrowphase through `narrowphase_serial` (`ReuseStep::OFF`, `d = 0`): their solver
  configuration sets `speculative_distance = 0` too, so both sides run the `d = 0` rule, the one they
  were pinned under; no direct-drive helper mixes a `d = 0` narrowphase with a `d > 0` solver.
  **(b)** — `sleep_settles_box_piles.rs`'s G2/G7 budgets, `MEASURED_P_*`, the settle limits and G4's
  mover premise are re-derived at C5 by their generator (`flicker_redraw_distribution`) and their own
  rules, as A7b did. **(c)** — the reference solver's acceptance gates (`tests/softstep.rs` and the
  files that wire `SoftStepSolver`) are tolerance gates: they run the new rule at C5 and are read as
  the census of reds, like every other file.
- **W3** — the SDF sentinel lane: S3-SDF drops a box onto an SDF floor at `d = 0.02` and requires the
  current separation to close; its mutation (the sentinel's `Δ_B` read from `Δ[ib]`, i.e. body A's)
  reds.
- **W4** — Miri: every target's full `MIRIFLAGS` per leg (Stacked Borrows and Tree Borrows),
  including `-Zmiri-disable-isolation -Zmiri-ignore-leaks` where the target's recipe requires them,
  run first on the parent with the same filters, so a red that already exists there is not
  attributed to V2.
- **W5** — window 9: `V2-RES` is two rows (the parent binary with no flag, the V2 binary with
  `--speculative-distance 0`, both against JT500); the `_v2` pose-gate script substitutes the hashes
  as well as the fixture paths; the V2 W8 fixtures are compared with the W1 recordings, never
  recorded from W8.
- **O1** anchors fixed in the commit that moves them; **O2** the literal census counted by site
  (and `sleep_sets.rs`'s `bit_equal` compares `bp_margin`'s bits — a pattern, not a literal);
  **O3** above (§2.3); **O4** every identity scene counts its pair type; **O5** `deferred:` until C5;
  **O6** noted; **O7** the DA9 row perturbs `d` relatively (`+= 0.01`), so it perturbs on both sides
  of the flip; **O8** the debug assert on `d`.

## 6. Pins

Moved on purpose at C5, each by its own file's rule (old → new, command, "V2 C5, owner V2a/V2b"):
`default_world_pyramid_determinism.rs` `PINNED_FINAL_HASH` and `_REUSE_OFF` (both profiles; the
old values survive as the `d = 0` arms), `sleep_settles_box_piles.rs` `A7_R1_D_MAX_BITS` (and the
reuse-off value), A7-R2's freeze step, G2/G7/G8, `alloc_frame_census` S1c/S1e (the fourth re-pin
form, attributed by a same-binary `d = 0` twin, S1f), `bp_query_counts` (its own precedent), the
fidelity gate's anchors (= the C6 fixtures), and whatever else the C5 census of reds names under its
own rule. Everything at `d = 0` stays byte-identical: the `d = 0` arms, `v2_speculative_d0_identity`,
the L10 pose gates run with `--speculative-distance 0` against the committed fixtures (264/264, both
negatives exit 4, 26/26, 17/17), the narrowphase unit oracles, the soft-body baselines. Every
run-vs-run gate stays unedited and green at the new default (W 1/2/4/8/16, the {1, N} oracle,
`simd_solve` on/off, the sleep-skip modes, the reuse twins' own comparisons).

## 7. Results

Appended at C7.
