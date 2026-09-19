VERDICT: APPROVED_WITH_CHANGES; BLOCKING=0; IMPORTANT=5

# Architecture review: L8, friction per manifold patch (D2)

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED. There are no blocking remarks. The determinism and bit-identity core holds. Five Important remarks are all local: four are measurement or gate specifications, and one is a small cost fix on the count-1 path. W1 should be fixed before C0 records its fixtures.

**What I checked, and it holds:**
- **Today's clamp and op order match the plan.** The circle clamp is `len_sq > mf² && len_sq > 0` (`colored.rs:2154-2159`, `:2559-2570`). The normal clamp never yields −0 (`:2106-2114`, `:3176-3181`). The warm apply is `(n·λn + t1·λt1) + t2·λt2` (`:1886-1888`). `apply_impulse` is `v + p·m⁻¹; ω + I⁻¹(r×p)` (`contact.rs:108-111`).
- **The count-1 reduction holds on both paths.**
  - `rb = Vec3::ZERO` on sentinel lanes (`colored.rs:1779`), so `rc_b = 0` agrees with today's rb.
  - `S/1.0` is exact, and `+0 + λn'` is exact because λn' is never −0.
  - Masked ranks and lanes change nothing.
- **Research claims checked on the web.**
  - Jolt v5.6.0: 2 linear rows plus 1 angular row at the averaged point.
  - Jolt #2121: the midpoint lever-arm error.
  - Bepu master: penetration, then tangent, then twist; binary centre weights; twist limit `μ·Σ dist·λn`.
  - Box3D: normals, then twist, then central friction; friction skipped under bias; limit `leverArm·normalImpulse`.
- **Groups are single manifolds** (`colored.rs:46-53`), so a per-lane patch is well-defined.

## 🔴 Critical
None.

## 🟡 Important

#### W1. The J and R timing rows run with sleeping on at the L10 base, and there is no pre-registered pass rule for the contact-set confound
**Where:** G-T. "Rows: J (default configuration)… R, R-S and R-ref"; the "H8 count moves by more than 2 %" clause.

**Problem, part 1 (sleeping):**
- `--cfg default` inherits `PhysicsConfig::sleeping` and never forces it off (`benches/jolt_parity_pyramid.rs:737-744`).
- L10 C1b flips that default to `true` (`L10-sleeping/04-DESIGN-REV2.md:45`, `:433`). L10's own research says "the R row inherits the new default unless the runner changes" (`L10-sleeping/01-RESEARCH.md:126`).
- The plan names L10's tip as its base and does not pass `--sleeping off`.
- P0 measured these scenes with sleeping on:
  - J-Son is all asleep from step 264 (`ANALYSIS.md:152`);
  - R-S first freezes at step 248 (`:150`).

**Problem, part 2 (contact set):** L8 changes values, and a solver value change has already moved the contact set by 12 %: R has 6,662 manifolds against R-ref's 5,949 (`ANALYSIS.md:145`).

**Consequence:**
- With sleeping on, 59 % of the [100,500) window has no solve, and 47 % of [0,500).
- The predicted ΔT(1) of −0.64 … −1.06 ms therefore dilutes to about −0.26 … −0.43 ms. The low end fails the plan's own "ΔT(1) ≤ −0.38 ms" gate even if the kernel behaves exactly as modelled.
- Each step the freeze point moves shifts the window mean by about 0.02 ms, in either direction, and has nothing to do with the kernel.
- With sleeping off, the ±2 % flag still admits about ±0.2 ms of confound (2 % of T(1)) against a 0.38 ms gate.
- Above 2 %, "the per-manifold stage figures become the primary claim", but no pass/fail rule is written for them. Per-manifold colour cost also moves with points per manifold, which the lever itself can change.

**Confidence:** CONFIRMED for the configuration path (runner lines and L10 lines above). The size of the contact-set shift is PLAUSIBLE; the D1 precedent is measured.

**What is needed:**
- Pin `--sleeping off` explicitly on every row that gates the lever. Sleeping-on rows become witnesses.
- Pre-register, before C0, a pass rule that the lever's own trajectory change cannot move. Two examples:
  - the solve run on an identical contact stream in both binaries (`solve_colored` is `pub`);
  - per-sweep cost normalised by both points and manifolds, with its own SE.

#### W2. G-S6 ("empty `soft_step.rs` diff") cannot pass once `WarmSeedStats` gains fields
**Where:** D1, G-S6, the Files table ("**no diff** (a gate)"), and the `WarmSeedStats` "+ 2 fields".

**Problem:** `soft_step.rs:483-492` builds `WarmSeedStats { … }` as an exhaustive struct literal, and `colored.rs:1691-1700` does the same. The struct is `pub`, with no `#[non_exhaustive]` and no `..Default` (`row_identity.rs:258-292`).

**Consequence:** C2 does not compile (E0063) unless `soft_step.rs` is edited, which breaks G-S6 as the plan defines it. The change is also a breaking public-API change for any external literal.

**Confidence:** CONFIRMED.

**What is needed:** one of the following.
- Keep the new counters off `WarmSeedStats`, for example as a colored-solver-only accessor.
- Or restate G-S6 as "no behavioural diff": list the literal's two `: 0` fields and keep R-ref's pose hash as the value gate.

#### W3. Count-1 cohorts pay for the patch step with no benefit, and no timing row can see it
**Where:** the ops table ("no new branches (masks)"), D8, D6, and the G-T row list.

**Problem:** by the plan's own op model, a cohort of depth 1 goes from 519 ops (L11) to 213 + 311 + 110 = 634 ops, which is +22 % kernel math. On top of that:
- the fill geometry adds 144 ops per cohort, even though for count 1 the result is just `rc = ra₀, d = 0`;
- the warm apply computes both torque forms and blends them.

Every G-T row (J, J-A, R, R-S, R-ref, S16) is a box scene.

**Consequence:** a sphere pile is 100 % count-1 manifolds, and spheres are one of the only two collider shapes (`systems.rs:622-633`). Such a scene keeps bit-identical output but runs about 22 % more kernel math (arith.), and nothing measures it.

**Confidence:** CONFIRMED (arithmetic from the plan's own table).

**What is needed:** one of the following.
- A cohort-uniform skip when no lane is `multi`. The masked ops change no value, so G-P5 still holds.
- Or a sphere-pile row gated "not claimed slower".

#### W4. "Both solvers, same analytic bounds" has no rule for when the untouchable reference fails a bound
**Where:** D1 ("G-P1…G-P4, which run on both with the same analytic bounds"), the G-P3 row, and C0.

**Problem:**
- The G-P3 row expects the per-point base may fail ("red-first if C0's per-point run fails"). If it does, `SoftStepSolver` is also per point and byte-frozen, so its G-P3 arm stays red permanently.
- G-P2's fixed band [25, 33] and its 1 mm COM-drift bound have the same exposure on the reference.

**Consequence:** C0/C2 cannot land green as written. The likely workaround, quietly dropping or widening the reference arm, loosens a bound one test at a time — the "gate that could not pass" class this repo already records.

**Confidence:** CONFIRMED for G-P3 (the plan's text contradicts itself under an outcome it anticipates); PLAUSIBLE for G-P2.

**What is needed:**
- For each G-P gate, state whether the reference arm is a gate or a receipt.
- State the rule if the reference misses: report it; never adjust the colored bound to match.

#### W5. There is no sleep-time gate; the only freeze bounds left are loose liveness limits
**Where:** "Rest and sleep": "A7-R2 and G8 must freeze within their tests' existing limits; the steps are read again".

**Problem:**
- A7-R2's limit is `LONG_SETTLE_LIMIT = 6000` (`sleep_settles_box_piles.rs:246`). The observed freeze is at step 248, so the limit leaves 24× headroom.
- The only tight tripwire is the runner's R-S `--frozen-by 300`. It voids a timing row; it does not fail a test.
- The brief asked for a box-pile sleep-time gate among the new physical gates.

**Consequence:** if patch friction delays freezing (for example, twist chatter at rest keeping ω² above threshold), every listed gate still passes. L10's headline sleeping gain (R −57 %) then erodes with no red.

**Confidence:** CONFIRMED (the text and the file constant).

**What is needed:** pre-registered bands on the A7-R2, G8 and R-S freeze steps, in the style of A7-R1: confirm / report / STOP.

## 🟢 Optional
- **O1. vn0 takes L12's slot.** D7 puts `vn0` (cold: restitution only, once per step) into the RankBlock's last free 32 B. L12 is ruled next and needs exactly one cached normal mass per rank, plus three per lane, while the patch has one `_spare`. L12 then either grows the block to a 384 B stride (+20 % of 13.3 MB streamed per step, the cost D1 itself cited) or re-moves vn0. Consider keeping vn0 cold, or state L12's layout now.
- **O2. Pin moves at C1 and C2.** C1 "setup digest extended" re-pins L11's G1 digest constants (S-a…S-g × W × simd), in a commit that claims zero pin moves. The multi-point S-a, S-b, S-d and S-g pins also move at C2 but are not in the red set.
- **O3. FMA census coverage.** The census scans only `simd.rs` (`simd.rs:1581-1586`) and `colored.rs` (`colored_tests.rs:3225`). The new `contact.rs` helpers are outside it. The scalar-vs-SIMD proptest would catch a `mul_add`; say which gate owns it, or extend the census.
- **O4. Absent lanes in the geometry pass.** On an absent lane, `S/c` with `c = 0` yields NaN. State that rc is blended by `present` (G3/M12 would catch it, but the pseudo-code omits the blend).
- **O5. Wording against L10.** "They count kept manifolds exactly as with sleeping off" should read "as under `SleepSkip::Off`" (L10 rev 2 exactness). Taken literally, it contradicts the new `manifold_carry_hits` assert. Also say whether `tw` is carried when a frozen record keeps fewer than 2 hits.
- **O6. A7-R1 reference value.** "Reported beside 0.7180" is out of date at this base: L9b (value-changing) precedes L8. Report against the C0 base reading.
- **O7. G-P4's stop time.** Define it as linear and angular both at rest. If it is linear only, M3 (twist dropped) is not red.

## Positive (keep)
- **Count-1 bit identity by construction:** `ra₀/1`, `+0 + λn'`, masked twist, D6's single branch. G-P5 plus the −0.0 unit test (M-C3, M7) can actually fail.
- **D9:** `max_sel`/`min_sel` with VMAXPS/VMINPS semantics. This is the right call where ±0 ties do occur.
- **D5:** the seed goes through L11's single lookup routine (the W1 ruling), with no new key class.
- **D1:** keeping `SoftStepSolver` as the Coulomb reference, with R-ref's hash pinned.
- **Analytic gates:** G-P2's and G-P4's derivations hold (patch = Coulomb for pure spin; the triangle-inequality bounds).
- **Gain arithmetic:** it reproduces from the plan's inputs, including W=8 holding L(8) constant (consistent with P0's E(8) = 0.467), and C0 has a ΔK refutation step.
- **The "cannot claim" list is honest.**

**Topics absent, with no consequence:**
- prefetch: cohort streams ascend;
- non-temporal stores: every line is re-read each sweep;
- loom: no new atomics;
- PGO;
- relax-only friction: correctly split off as OQ3.

## Open questions
1. Is G-P2's [25, 33] band fixed, or registered from the base? A soft-contact ramp on the base could read 34.
2. OQ1 (the W=8 gate) is for the orchestrator. Under W1 it matters less than the choice of a confound-robust pass rule.

**Files:**
- `D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/soft_step.rs`
- `D:/wt/joltab/crates/boyko_physics/src/row_identity.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/contact.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/simd.rs`
- `D:/wt/joltab/crates/boyko_physics/src/systems.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/sleep_settles_box_piles.rs`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L10-sleeping/01-RESEARCH.md`
- `D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md`

Sources:
- [Jolt issue #2121](https://github.com/jrouwe/JoltPhysics/issues/2121)
- [Jolt v5.6.0 release notes](https://newreleases.io/project/github/jrouwe/JoltPhysics/release/v5.6.0)
- [Bepu ContactConvexTypes.cs](https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/Constraints/Contact/ContactConvexTypes.cs)
- [Box3D contact_solver.c](https://raw.githubusercontent.com/erincatto/box3d/main/src/contact_solver.c)
