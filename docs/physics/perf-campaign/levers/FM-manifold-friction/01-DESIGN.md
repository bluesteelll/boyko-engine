# FM — friction per manifold: the design (rev 2, as amended by the C1 cut)

FM replaces the colored solver's per-point friction with per-manifold friction behind
`PhysicsConfig::friction_model`. Each contact point keeps its normal row. A manifold of two or more
points gets **one friction patch**: two tangent rows at per-body friction centres, clamped jointly
to the disc `mu * sum(lambda_n)`, and one twist row about the normal, clamped to
`mu * sum(d_r * lambda_n,r)`. This is the model of Jolt 5.6 and of Rapier 0.36's default
`FrictionModel::Simplified` (and Bepu v2). It removes the 1.644x friction-row factor against Jolt:
per friction-bearing sweep the rows go from `3 * Np` to `Np + 3 * Nm - N1`.

C1 builds it switched off (`FrictionModel::PerPoint` is the default) and value-neutral at the
default. The flip, the losing centre's deletion and the moved pins are C3's, after the screens (C2).

**Sources.** The design rev 2 `D:/tmp/phys-orch/fm/design_r2.md` (derivations, the price model, the
screens, §9-§14), its critique `fm/critique_final.md`, the C1 cut `fm/cut.md` (§1.4-§1.5 amend the
design; its addenda 1-3 record the developer's decisions and readings), the research
`fm/research_web.md` and `fm/research_tree.md`, and the rulings
`phys-next/RULINGS-2026-10-01.md` (items 21, 24, 25). Code is cited by symbol, never by line. This
file is not in `tests/internal_docs_anchors.rs`'s `GATED_DOCS`; its anchors are not machine-checked.

---

## 1. Goal, bars and non-goals

**Goal.** Fewer friction rows on multi-point manifolds, with the comparison engines' model.

**Bars** (ruling 25): bit identity across worker counts; the J-T 26-run gate; the 88-run ensemble no
worse than the trunk V2 control; the red control still fails; ruling 24's sleep criterion in its FM
form (design §9.2 item 5); zero cost with the switch off; no heap, `HashMap` or `dyn` on the hot
path; every `unsafe` with a `// SAFETY:` comment. Ruling 21 adds: maximum speed for every
interaction type (D16).

**Non-goals.** The reference solver (`SoftStepSolver` stays per-point Coulomb, byte-unchanged, and
ignores the field); IB's FIB axis; new friction physics (softness, bias, anisotropic mu);
world-space friction warm storage; the L12 effective-mass cache; width-sorted cohort packing (a PC
line); a speculative-manifold skip; the frozen-velocity defect (lever SL).

---

## 2. Decisions

### D1 — the model
Per-point normal rows plus, for each manifold of width >= 2 in a cohort of depth >= 2, one patch:
tangent rows along `t1`, `t2` at the per-body centres `rc_a`, `rc_b`, solved as 1-D rows and then
clamped to the disc `|lambda_t| <= mu * s_n`; one angular twist row along `n`, clamped to
`|lambda_tw| <= mu * s_d`; `s_n = sum lambda_n,i`, `s_d = sum d_i * lambda_n,i`, `mu` the pair's
friction as today. Width-1 manifolds behave exactly as today wherever they sit (D2, D16).
Rejected: PhysX-style patch anchors (4 rows), a MuJoCo-style elliptic cone.

### D2 — order: normal rows, then twist, then tangent; the limits read this sweep's lambda_n
Per lane per sweep: the normal rows in point order, accumulating `s_n += lambda_n'` and
`s_d += d_r * lambda_n'` in registers; then the twist row; then the tangent pair. Normal-first (Box2D
v3, Rapier, Bepu, Box3D) rather than Jolt's friction-first because (1) it gives the **width-1
identity**: `s_n = +0 + lambda_n' = lambda_n'` (lambda_n' is never -0) and `rc = ra_0 / 1 = ra_0`
bitwise, so a width-1 lane's tangent step is today's per-point friction op for op; (2) no new
per-lane state; (3) no first-sweep friction gap; (4) the rigid friction rows end the sweep with zero
residual, the soft biased normal rows are corrected by the next sweep. Twist before tangent: the
stick/slip pair gets the last word and reads the twist's omega.

### D3 — the friction centre: C-ALL (default), C-TOUCH (a hidden screen arm); the twist arm
- **C-ALL:** `rc_a = (((ra_0 + ra_1) + ra_2) + ra_3) / c`, `rc_b` likewise; the sum starts from rank
  0's value, never from +0; the division is IEEE `S / c` (never `S * (1/c)`). On an SDF sentinel
  lane `rb = 0`, so `rc_b = 0`.
- **C-TOUCH:** weight 1 iff `sep_r <= 0` (`-0 <= 0` holds); the first-contributor rule
  `acc = blend(acc + ra_r, ra_r, first)` never adds a masked +0, so a single touching `ra_0` keeps its
  -0 bits; `c_t = sum w_r`; `c_t = 0` (wholly speculative) falls back to C-ALL.
- **Twist arm:** `d_r = |delta - (delta . n) n|`, `delta = ra_r - rc_a`, the same for both centres;
  `d_r = 0` on padding ranks.
- C-ALL is the default because both comparison engines ship it and it is continuous in a point's
  separation crossing 0. C-TOUCH is built because, under V2, 3,868 of 8,501 J-T manifolds per step
  are wholly speculative, and the selection rule (design §9.2 item 6) needs both centres in one
  binary. Switching centres is not a form change (D15). C3 deletes the loser.

### D4 — tangent solve: two 1-D rows plus a disc clamp
Today's ops and `effective_mass` / `effective_mass_x8`; keeps D2's width-1 identity. Rejected:
Rapier's exact 2x2 block.

### D5 — the twist row
`m_tw = 1 / (n . (Ia^-1 n) + n . (Ib^-1 n))`, 0 if the sum is not positive (`twist_mass`,
`twist_mass_x8`); `w = (omega_b - omega_a) . n`;
`new = min_sel(max_sel(lambda_tw - m_tw * w, (mu * s_d) * -1), mu * s_d)` with
`max_sel(a, b) = if a > b { a } else { b }` (VMAXPS's tie and NaN semantics; `contact::max_sel`,
`min_sel`); applied as `omega_a += Ia^-1 ((n * delta) * -1)`, `omega_b += Ib^-1 (n * delta)` under
`multi && movable` (`BodyEffective::apply_angular_impulse`, `simd::apply_angular_blend_x8`). A
patched cohort always has a `multi` lane (depth is the widest lane), so the twist always runs there.

### D6 — friction stays rigid and runs in every sweep, as today

### D7 — layout: one extra patch `RankBlock` per cohort of depth >= 2, at `rank_base + depth`
The patch has exactly a `RankBlock`'s shape (ten `[f32; 8]` rows, 320 B), so it rides the existing
`blocks` column with the cohort's own consecutive, pairwise-disjoint block range; no new scratch id.
`CohortColumns::layout` allots `depth + u32::from(fm && depth >= 2)` blocks per cohort.

| `RankBlock` row | patch meaning | written by | read by |
|---|---|---|---|
| `ra[0..3]` | `rc_a` | fill | kernel, warm apply |
| `rb[0..3]` | `rc_b` (0 on a sentinel lane) | fill | kernel, warm apply |
| `sep` | `lambda_t1` | fill, kernel | kernel, warm apply, store |
| `ni` | `lambda_t2` | fill, kernel | kernel, warm apply, store |
| `ti1` | `lambda_tw` (+0 on lanes of width < 2) | fill, kernel | kernel, warm apply, store |
| `ti2` | zero (reserved: L12's cached masses) | fill | — |

In a patched cohort the rank blocks keep `ra`, `rb`, `sep`, `ni`; their `ti1` holds `d_r`
(read-only in sweeps) and `ti2` is zero. Restitution and `vn0` index ranks `0..width` only; the
fill writes the patch's `vn0` row and padding lanes zero (cut D-C13, G3). A depth-1 cohort has no
patch. With FM off `layout` sizes the column exactly as today. Accessors:
`RankBlock::{patch_triple, set_patch_triple}`.

### D8 — the patch geometry is computed in the fill
A scalar oracle (`patch_geometry`) plus an 8-lane pass under `simd_solve`
(`simd::patch_geometry_x8::<TOUCH>`), bit-identical to each other; absent lanes are blended by
`present`, so an absent lane's 0/0 never escapes. `FmFill.centre` selects the accumulation inside the
FM branch only.

### D9 — warm storage: the same 64 B `WarmRecord`, the triple in fixed slots
Normals per feature id as today; **lambda_t1 -> `t1[0]`, lambda_t2 -> `t2[0]`, lambda_tw -> `t1[1]`**;
every other tangent slot +0. Two properties by construction: a one-point record has the same bytes
in both forms; a record whose tangent words are all zero is valid in both forms. The read side's
form is a solver fact (`stored_fm`), never a per-record byte. Accessors:
`WarmRecord::{friction, set_friction, set_normal, has_fid}`.

- **Seed rule:** normals per fid through `WarmLookup::seed`; the triple through
  `WarmLookup::friction_seed` — the triple of the first record of Lemma W's walk (descending manifold
  index over the run) holding at least one of the manifold's feature ids, with lambda_tw replaced by
  +0 when `count < 2`; no hit seeds `[+0; 3]`. In a depth-1 cohort the pair goes to rank 0's
  `ti1`/`ti2`. For a width-1 manifold whose source record has one point this equals `seed_of`'s
  tangent pair bit for bit; a manifold that shrank from 4 points to 1 takes the manifold triple by
  design (not an identity).
- **Lemma W-F** (in `warm_records.rs`'s module doc): the friction seed is a pure function of the
  stream and the read side.
- **Carry:** `WarmLookup::carry` is unchanged; `WarmLookup::carry_fm` (out of line) compacts the
  normal hits as `carry` does and copies the source triple, without the count < 2 zeroing, iff at
  least one point hit. `capture_moved_in` and the store's frozen branch (serial and the region's
  Store blocks) take it under FM. **Idempotence** (`carry_fm(carry_fm(r)) == carry_fm(r)`, all 16
  words) is what makes held = Off on a frozen manifold; a proptest covers it.

### D10 — warm apply: one merged impulse per (lane, body) on patched cohorts
Per lane: `F = t1 * lambda_t1 + t2 * lambda_t2`; `P = ((n * s_n) + (t1 * lambda_t1)) + (t2 * lambda_t2)`
with `s_n` from rank 0's value; `Q_a = sum lambda_n,r * ra_r`. Torque on A: width >= 2
`(((Q_a x n) + (rc_a x F)) + n * lambda_tw) * -1`, width 1 today's `ra_0 x (P * -1)`, blended by
`multi` before the `I^-1` multiply; B the same without the sign. Depth-1 cohorts run today's apply.
The `(k_lo, k_hi)` cohort range of SR-B C1 is kept (`warm_apply_scalar::<FM>`,
`warm_apply_avx2::<FM>`, `warm_apply_patch_scalar`, `warm_apply_patch_x8`).

### D11 — store
Patched lanes: `set_normal(r, ni)` for `r < width`, then `set_friction(triple)`
(`store_solved_fm`, out of line, shared by `store_and_swap` and `store_block`). Depth-1 lanes:
today's `set_impulses(0, [ni, ti1, ti2])`, whose bytes are the FM form at count 1.

### D12 — the switch
`#[non_exhaustive] pub enum FrictionModel { PerPoint, PerManifold, #[doc(hidden)] PerManifoldTouch }`
(`boyko_physics::resources::FrictionModel`); `fm = model != PerPoint`, the centre is C-TOUCH iff
`PerManifoldTouch`. The config is latched whole per step, so a write lands at the next broadphase.
The FM-off code is a `<const FM: bool>` instance of today's functions (cut D-C1:
`solve_color_k`, `solve_color_avx2_k`, `solve_color`, `solve_color_avx2`, `solve_color_dispatch`,
`solve_all_colors`, `warm_apply_scalar`, `warm_apply_avx2`, `fill_range`, `fill_cohort`), picked once
per call at the callers (`solve_colored_inner`, `build_columns`, `SolveStages::run_block`'s Fill /
Warm / Biased-Relax / Store arms); FM work inside an FM-off enclosing function is a call to an
out-of-line FM function, never an inlined body (cut D-C3). Rejected: no switch (the d = 0 contract's
five multi-point pins), a compile-time preset.
**D12a (C3's):** the overlap-only bridge becomes `speculative_distance = 0`,
`speculative_velocity_cap = 0` **and** `friction_model = PerPoint`.

### D13 — restitution, speculative manifolds, sensors, kinematics, SDF, sleeping
Restitution is normal-only and unchanged. A wholly speculative manifold's patch has zero capacity
until a normal row pushes, but its rows still execute (a per-cohort skip is not value-neutral: the
clamped `new_t * 0` yields -0). Kinematic and static sides have zero inverse mass. Frozen manifolds
are not laid out; their records are carried by `carry_fm`; the held store follows D15.

### D14 — counters, stats, seed diagnostics
`WarmSeedStats` is untouched (its point counts are normal hits under FM).
`ColoredSoftStepSolver::friction_seed_stats() -> Option<FrictionSeedStats { manifold_hits,
manifold_carry_hits }>` (`Some` iff the last solve ran the per-manifold form; `manifold_carry_hits`
is physical, cut D-C5). `for_each_warm_seed` reports `[lambda_n, +0, +0]` under FM.
`for_each_friction_seed(manifolds, sets, f(key, Option<[f32; 3]>))` reports, per logical manifold
(stream, then kept), the triple the next step would seed. `#[doc(hidden)] layout_census() ->
LayoutCensus { cohorts, cohorts_by_depth, rank_slots, patch_blocks, multi_lanes }` is computed on
demand from the heads and the block column (no state written by `layout`). `#[cfg(test)]`:
`friction_census` (`FrictionSeedCensus`), taken beside the serial `setup_digest` and in the region's
Digest block.

### D15 — the model toggle: friction is zeroed once in every warm store
Every warm record the engine can read is valid in the current step's form:
- **The read side:** `if fm != self.stored_fm { self.friction_form_changed(fm) }` in
  `solve_colored_inner` (cold, never inlined) zeroes the read side's tangent words
  (`warm_records::zero_friction`) and records the form.
- **The held store:** `SleepSets` keeps `kept_fric` (the form bit, `friction_model != PerPoint`,
  critique O1); its prologue, after the epoch update and before any restore or D-H drain, runs the
  cold `kept_friction_form_changed` on a form change.
- A centre change (`PerManifold` <-> `PerManifoldTouch`) is not a form change: neither store zeroes.
- **Why held = Off across a toggle without a flush:** both worlds zero at the same step, and a
  zeroed record is a fixed point of both carries on a frozen manifold. Normals stay warm; friction is
  cold for exactly one step (documented on the field).
- Rejected: a per-record form byte (a per-record test on the FM-off path); a sleep-epoch flush (it
  would change no value, so no gate could see it); converting instead of zeroing.

### D16 — depth-1 cohorts keep the per-point path under FM
In every FM instance the per-cohort body is `if FM && depth >= 2 { patch body } else { today's body }`
(kernels, warm apply, store); the fill writes today's rank rows for a depth-1 cohort except that the
friction seed comes from `friction_seed`. Value-neutral: for a width-1 lane the patch path computes
today's values op for op (D2), so a width-1 manifold gets the same values whichever cohort it lands
in. Only depth 1: a depth-2 exception would make the friction model depend on packing.

---

## 3. Kernel shape (AVX2, one cohort, `FM = true`)

```text
gather bodies once; load the head; t2 = n x t1
if depth < 2: today's rank loop with per-point friction
else:
  present = width > 0; multi = width > 1; s_n = 0; s_d = 0
  for r in 0..depth:                                 // normal rows only
     load ra, rb, sep, ni, d
     normal row, today's ops (V2's s_eff and speculative dlambda) -> ni'
     s_n = blendv(s_n, s_n + ni', active); s_d = blendv(s_d, s_d + d * ni', active)
     store ni = blendv(old, ni', active)
  patch = block(rank_base + depth): rc_a, rc_b, lambda_t1, lambda_t2, lambda_tw
  twist row (D5); lambda_tw = blendv(lambda_tw, new, multi)
  tangent rows: today's friction ops at (rc_a, rc_b), limit mu * s_n, masks present & movable
  store the triple (blendv by present)
scatter once
```

The scalar oracle (`solve_color_k::<SPEC, K3, true>`) is the same per lane, group-major; scalar
workers touch distinct lane elements of the patch through raw projections, never a `&mut` over the
block (red-first M-S4: a whole-block `&mut` is a data race between workers under Stacked and Tree
Borrows).

Ops per cohort: today 611 per rank; FM at depth >= 2 303 per rank + 420 per patch; FM at depth 1
611. Read lines per cohort-sweep +50 % at depth 2, +25 % at depth 4; dirty lines 4 -> 4 at depth 2,
8 -> 6 at depth 4 (design D7, `fm_price_r2.py`).

---

## 4. Determinism

1. **Scalar = SIMD under FM, both centres:** IEEE add/sub/mul/div/sqrt, no FMA (the colored and the
   `contact.rs` censuses), VMAXPS/VMINPS clamp semantics, `blendv` accumulators, zero padding.
2. **W invariance:** cohorts, depths, cuts, waves and region blocks are W-invariant; the patch step
   is a pure function of its lane; the seed walk reads only the stream and the read side; D15's
   zeroing is a function of the latched config.
3. **One-point identity:** FM on equals FM off for a scene whose manifolds all stay one-point for the
   whole run (every cohort depth 1, D16), and lane by lane for width-1 lanes inside patched cohorts.
4. **FM off is byte-identical:** the `FM = false` instances are today's code, `layout` sizes today's
   column, the records are today's bytes, and the two form compares are not taken in a run that
   never toggles.

---

## 5. What FM cannot claim

Coulomb exactness under combined slide and spin (the disc and the twist box are clamped
independently: 1x-2x the per-point dissipation for a symmetric patch; G-P4 bounds it); trajectory
continuity with per-point friction for manifolds of >= 2 points or that shrink to one point; a
per-stage parity with Jolt; any wall number before the timed A/B; torsional friction on one-point
contacts; warm friction across a model toggle (cold for one step).

---

## 6. Risks (design §12, risk 8 as corrected by the cut)

1. The hold gate fails under FM: the switch stays off; nothing is re-tuned.
2. The sleep criterion fails (twist chatter delays freezes): measured in ruling 24's unit.
3. The perf model is optimistic: the W1 refutation at 0.66 ms is pre-registered.
4. The contact-set confound in end-to-end timing: the per-cohort colour cost attributes.
5. The patch's traffic: read lines up, dirty lines down; the A/B shows any regression.
6. Kernel instance growth (2 -> 4 per path); the codegen receipt records the binary.
7. The d = 0 contract amendment (D12a) needs the orchestrator's acceptance.
8. **Lemma W-F edge cases.** On one record, `carry_fm` is idempotent (proptest). Across a run of >= 2
   records (duplicate-pair streams, hand-built only), the friction rule "first record with a fid
   hit" can propagate along fid chains (m1{a,b}, m2{b,c}, m3{c,d}: capture gives T(R2), Off's next
   carry T(R3)), so held = Off is not claimed there and a `debug_assert!` at the capture refuses it.
   M9 and G1 S-e cover the seed walk on non-strict sides; G1 S-e is direct drive and never captures.
9. IB x FM ordering.
10. Mixed cohorts (width-1 lanes inside depth >= 2 cohorts) pay the merged warm form's dual torque.
11. The basis-seed switch at |n.z| = 0.999: exposure unchanged.
12. Depth-2 cohorts may be slower (x0.893-x1.104 per cohort-sweep, modelled): the `fm_depth_ab`
    depth-2 row decides; no value-neutral exception exists.
13. A toggle costs one friction-cold step (documented on the field).

---

## 7. The plan after C1

- **C2 — the screens** (untimed, the orchestrator's, on C1's binaries with `--friction manifold` and
  `--friction manifold-touch`): design §9.1 (the structural screen; its K1 reads the runner's
  `layout_census.cohorts_by_depth`) and §9.2 (the fidelity screen, both centres, the per-point
  control in the same lane).
- **C3 — the flip:** `Default` becomes `PerManifold` with the winning centre; the loser deleted; the
  overlap-only bridge gains `friction_model = PerPoint` (D12a); every moved pin by its rule (design
  §9.3); `cargo test --workspace --all-targets --no-fail-fast`.
- **C4** re-records the V2 fixtures and fidelity anchors; **C5** is the record.
- **Timing** is outside the lane: same-binary A/B rows (`--friction point|manifold`) on J-T and the
  dyn scenes at W 1/2/4/8/16, plus `colored_solve`'s `fm_depth_ab` at depths 1-4 (design Q3).
