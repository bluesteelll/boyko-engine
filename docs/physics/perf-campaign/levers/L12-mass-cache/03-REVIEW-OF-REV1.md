# L12 effective-mass cache per inertia epoch - architecture review

VERDICT: APPROVED_WITH_CHANGES; BLOCKING=0; IMPORTANT=4

# Architecture review: L12, the effective-mass cache per inertia epoch (rev 1)

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED. There are 4 Important remarks and no Blocking one. Each fix is local to the build-if rule or the gates, and the design itself stays as it is.

I found no determinism hole. I checked the design's claims against `D:/wt/joltab`:

- **Epoch schedule.** The substep order is gravity → warm apply → biased sweep → integrate + `refresh_inertia` → relax × R (`colored.rs:3547-3617`). That gives 5 epochs, with 5 sweeps that compute and 7 that load. The design's formula S·R − 1 for R ≥ 1 is correct.
- **Every sweep goes through one dispatch.** All four call sites use `solve_color_dispatch` (`:2758`, `:2885`, `:3089`, `:3119`). So `simd` is the same for every sweep of a step, and the scalar path never touches the new column.
- **Writers of the mass inputs.** `inv_inertia` is written only by `build_bodies` (`:1557-1562`) and by `refresh_inertia` (`simd.rs:151-157` plus its AVX2 twin), which is called only at `:3599`. `inv_mass` is written only by `build_bodies`. Soft-body coupling reads `BodyState`, not `BodyEffective` (`soft/coupling.rs:263-290`). L9 does not touch the solver (L9 design `:35`). L5 and the tree broadphase touch no solve input. The B1 carry and L10's frozen islands are outside the sweeps. SDF sentinels gather `IMMOVABLE_AT_REST` (`:2361`).
- **FP op count.** I recounted the 246 FP ops per rank from `simd.rs:655-682`: 82 per direction (48 multiplies, 31 adds or subtracts, 1 divide, 1 compare, 1 blend).
- **No FMA risk.** rustc does not fuse a multiply and an add unless the code asks for it, and the MSVC host does not change that.

## 🟡 Important

### W1. The build-if bar is about 1 % of T(1), but the campaign's recorded rule is 5 %, and the design cites no override
- **Where:** the short answer "Build-if"; Expected gain "Build-if"; the "decision" row of the commit sequence.
- **Problem:** Earlier rulings set the build-if at ≥ 5 % of T(W) plus the SE bar:
  - `levers/00-RULINGS.md:28-29`, for L5's parallel compaction: "built only under the campaign build-if rule (≥ 5 % of T(W) and above the SE bar), not at the plan's 1 %".
  - L11's conditional C4 uses the same 5 % rule (L11 D10).
  - The per-contact override at `:115-116` is written for L9 only, and it overrides "the W=8 build-if that §9 applied".
- **The design's own numbers never reach 5 %:**
  - 1.5–4.2 % of T(1) if the kernel is issue-bound;
  - 0.8–2.4 % of T(8);
  - after L8, ×0.56–0.69 of that.
- **Consequence:** Under the recorded rule, L12 is not built in any regime its own table allows. As written, the lane would build C0, C1 (two kernel variants, G1–G8, M1–M11) and T1 against a bar that an earlier ruling rejected by name.
- **Confidence:** CONFIRMED (the text of the rulings file and of the design).
- **What is needed:** Cite or request a ruling that L9's per-contact override also covers L12, with the SE-only bar at W=1. Without that ruling, state the 5 % bar, which closes L12 at C0.

### W2. T1's control arm is not the parent, so the build-if subtracts the wrong term
- **Where:** D4 ("the compute variant always stores"), D1 (`load = cache && valid`), T1, and the build-if "Δ_T1 − 11 µs".
- **Problem:** Write C for a compute sweep, S for the stores that sweep adds, and L for a load sweep.
  - `with_epoch_mass_cache(false)` runs all 12 sweeps as compute + store. The parent runs 12 compute sweeps with no store and no column.
  - T1 measures Δ_T1 = off − on = 7C + 7S − 7L.
  - The true gain against the parent is 7C − 5S − 7L.
  - So Δ_T1 overstates the true gain by 12·S, plus the off arm's extra column traffic. That arm writes 12 × 232 KB per step with read-for-ownership, about 5.6 MB, where the parent writes nothing.
  - The design subtracts 11 µs, which is its bound for 5·S. By its own store rate, 12·S is 19–26 µs. If the write traffic is exposed at W=1, the traffic adds up to about 40–50 µs more, by the design's own rate ("2.78 MB, ≤ 19–26 µs").
- **Consequence:**
  - The build-if leans toward "continue" by up to about 0.5–0.7 of its own 70–90 µs bar.
  - Claim (a), "≥ 0.6 × Δ_T1", leans the other way, toward a false red.
  - The `mass_cache = false` "rollback" is slower than the parent, because it stores in all 12 sweeps. It is not a rollback.
- **Confidence:** The algebra is CONFIRMED from the design's text. The size of the bias is PLAUSIBLE.
- **What is needed:**
  - Either a T1 control whose kernel matches the parent's (no store), or a measured S with the correct 12·S subtraction.
  - State that rolling back means reverting C1, not setting the flag.

### W3. The C0 stop rule cannot fire
- **Where:** Expected gain, "Stop rule at C0"; C0 (b)–(d).
- **Problem:** t_rank = (wide + narrow) / sweeps / ranks. That total includes work L12 does not touch:
  - the cohort-entry gather of 16 bodies (L11 keeps `stage_body_state`, 16 floats per body);
  - the cohort-exit scatter;
  - per-colour zone and loop overhead.

  L11's review put gather + scatter at 100–160 ns per cohort (`L11-solve-setup/03-REVIEW-OF-REV1.md:66`). At a depth of about 4 that is 25–40 ns per rank, i.e. 80–180 cycles at 3.3–4.6 GHz. The rule stops when t_rank < c_chain(L8) + 20 c. For that to happen, the rank loop would have to run faster than its own velocity chain.
- **Consequence:**
  - C0 always goes on to C1. The decision is left to T1, after every C1 gate and mutation has been built. The rule's stated purpose ("closed without writing C1") is never served.
  - The same inflated t_rank is what puts the headline "issue-bound" row at 320–530 cycles.
- **Confidence:** CONFIRMED as arithmetic on the design's own definitions. The gather-cost input is PLAUSIBLE.
- **What is needed:** The C0 decision should use a quantity that isolates the marginal cost of the mass work. Two directions: a bench-only upper-bound probe in which the masses are not computed (a probe may change values), or a span that times the rank loop alone. Otherwise, drop the C0 rule and say that T1 decides.

### W4. G2 and G3 likely run the inline path at every W
- **Where:** G2 "at W ∈ {1, 2, 4, 8, 16}"; G3 "hash equal across W"; the M6 row lists "G3 (hash across W)" as a detector.
- **Problem:**
  - S-a is a 6-layer pyramid. A colour dispatches only at ≥ `MIN_PARALLEL_SLOTS_PER_COLOR` = 256 slots (`colored.rs:240`). The whole step runs inline when no colour reaches that (`:3544-3545`).
  - A 3D 6-layer pyramid has 91 boxes. Scaled from J's density (4,519 manifolds for 1,240 bodies, 16,888 points, about 11 wide colours), that is about 1.2k slots over about 11 colours. The widest colour is probably below 256. A 2D 6-layer pyramid (21 boxes) certainly is.
  - If so, W ∈ {2…16} run exactly the same code as W=1.
- **Consequence:**
  - Those arms can fail only on defects that do not depend on W, which breaks the "every gate can fail" rule for them.
  - A mass computed on one thread and loaded on another is then checked by values only through G6 (J at W=8) and by Miri only through G8b's synthetic case.
- **Precedent:** `threshold_inline_vs_parallel_dispatch_is_bit_identical` asserts that its scene's widest colour exceeds the threshold (`colored_tests.rs:1073-1089`).
- **Confidence:** PLAUSIBLE. S-a is defined by L11's C0 and does not exist in the tree yet.
- **What is needed:** Each W>1 arm asserts that dispatch really happened, on a scene that dispatches: either widest colour ≥ 256 with n_chunks ≥ 2, or a scope/chunk counter delta > 0. Ideally, it also asserts that at least one cohort's compute sweep and load sweep ran in different tasks.

## 🟢 Optional
- **O1. Two rows of the mutation table name detectors that cannot catch them.**
  - **M7 (red-first scene).** A box pyramid with ω = 0 and identity rotations still starts rotating. Gauss-Seidel over 4 off-centre points applies unequal angular impulses, so the inertia bits change after the first substep and `mass_changed_lanes > 0` stays green. A control where inertia cannot change by construction would work: rotation-locked bodies (`inv_inertia_local = ZERO`) or resting spheres with ra ∥ n. PLAUSIBLE.
  - **M4 "red in G1".** G1 drives `MassSchedule` with a synthetic event list that starts with `writer`. A missing `writer()` call in `build_bodies` is invisible to it. Either G1 replays an event trace recorded by the real step, or G1 comes off M4's row. CONFIRMED.
- **O2. D6 overstates the existing differential coverage.** Of the four differentials cited, only `:1626` runs a full step.
  - `:1454-1535` calls the kernel directly, once per colour.
  - `:2009` (1d) and `:2101` (the proptest) are single-sweep direct calls.
  - With `::<false>`, those three never execute a load. G5 carries that weight, and the design should say so. Consider giving 1c/1d a compute-then-load pair too: the cone clamp and the degenerate lanes are where a misindexed load would hide.
- **O3. G4's census is too narrow.** It matches the `inv_inertia` / `inv_mass` tokens in `colored.rs`. It misses:
  - whole-row writes such as `eff_rows[r] = …` or `*b = BodyEffective{…}`;
  - writes inside `simd.rs` functions that already receive `&mut [BodyEffective]` (`apply_gravity`, `:3553-3554`);
  - writers of the other mass inputs (ra/rb, n/t1, and L8's patch anchors) outside the fill.

  A census of the mutable access sites would be tighter than one over field names. G2 is the value backstop.
- **O4. T1's per-mode split.** `Rl_on = 4c′ + 4l` with c′ = B_off/4 uses the biased sweep's compute cost for the relax compute sweeps. The biased branch has extra ops (`:2498-2507`), so the "two estimates of l" disagree by construction. Use c′_R = Rl_off / 8.
- **O5. The reserve formula fits L11's shape only.** "3 × block reserve + cohort bound" is L11-shaped. After L8 it should be R_g·blocks + even_up(C + R_g)·cohorts. A push past the reserve panics (`scratch_ids.rs:90-91`). State the formula in terms of `MASS_LANES_PER_*`.

## Positive (keep)
- **The epoch key is exact and minimal.** Rejecting once-per-step masses is correct, because inertia is refreshed every substep.
- **The scalar oracle and restitution keep recomputing (D6).** That makes every multi-sweep differential a cache-against-recompute check.
- **The column layout is right.** Per-cohort runs are line-aligned against false sharing. Stores are temporal, with the reason given. There is no state across steps and no id, so replay holds.
- **C0b is separated from L12 (D8).** This stops the A/B from measuring inlining and caching together.
- **The honesty is right.** The design states the regime uncertainty, gates W≥2 only as "not slower", and gives a "cannot claim" list.
- **P0 inputs match.** P(1) = 3.576 ms and E(8) = 0.467 are P0's figures (ANALYSIS `:138`).
- **Topics absent with no consequence:**
  - loom: no new atomics protocol;
  - software prefetch: the access is sequential;
  - PGO;
  - FMA: covered by the source census and C0's binary census.

## Open questions
1. **Sleeping in the measured rows.** After L10 flips the default, J with sleeping on is all asleep from step 264 (P0 §6), inside the [100, 500) window. Pin `sleeping` for T1's capture and T2's J rows. Also note that under L10's logical `Manifolds` view, the per-contact denominator counts sleeping manifolds that are not solved.
2. **OQ1 belongs to L8.** Computing friction masses at fill time (Box3D) costs 1 evaluation per step instead of 5 per step with L12. The choice belongs to L8, decided on performance and on the incline stick/slip bounds, not on keeping L12's scope. No L8 design exists under `levers/`. L12's C1 layout should be fixed only after L8 closes.
3. **If T2 fails claim (b) with C1 landed**, is C1 reverted? See W2: the flag does not restore the parent's cost.

Files:
- D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/simd.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/contact.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/colored_tests.rs
- D:/wt/joltab/crates/boyko_physics/src/scratch_ids.rs
- D:/wt/joltab/crates/boyko_physics/src/soft/coupling.rs
- D:/wt/joltab/docs/physics/perf-campaign/levers/00-RULINGS.md
- D:/wt/joltab/docs/physics/perf-campaign/levers/L11-solve-setup/02-DESIGN-REV1.md
- D:/wt/joltab/docs/physics/perf-campaign/levers/L11-solve-setup/03-REVIEW-OF-REV1.md
- D:/wt/joltab/docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md
- D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md
