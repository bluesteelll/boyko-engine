# Physics throughput vs Jolt, measured first (Rev 1)

## 0. Direct answer to the owner's question
No, sleeping will not close the remaining gap. Three reasons, each from code I checked:

1. **The parity number has sleeping off on both sides, on purpose.** Jolt sets `PyramidScene.h:44` `mAllowSleeping = false`, and our bench sets `jolt_parity_pyramid.rs:217` `cfg.sleeping = false`. The scene measures how fast each engine steps one large awake island. A sleep mechanism cannot change that ratio.
2. **Even when it is on, our sleeping skips only the solve and the integrate.**
   - Frozen manifolds are skipped at `colored.rs:1606`.
   - Integration runs over every row and is then undone for frozen rows (`colored.rs:3282-3297`, `:3414-3426`).
   - Gather, broadphase, narrowphase, graph build and apply still visit every body and pair.
   - Jolt drops sleeping bodies from pair finding entirely. So a sleeping resting pile costs Jolt almost nothing, while we keep paying the collision half. Sleeping on both sides would widen the gap on that scene.
3. **The gap looks like serial and dispatch time, and sleeping touches neither.**
   - Amdahl on the 2026-09-10 row: f ≈ 0.48–0.53 (W=2/4/8), so S(1) ≈ 9.7–10.7 ms of T(1) = 20.3 ms.
   - The fit cannot separate true serial work from parallel loss. The profile in §2 does that.

What sleeping *can* do is cut the cost of a default world that holds resting piles. That is a different metric, and it is owner decision D3 (§4). It also depends on B1: the reading below suggests a frozen island loses its warm-start impulses.

## Context (facts this plan relies on; corrections C1–C4 from the tree report accepted)
- Solver structure checked: `solve_colored_inner`, `colored.rs:3219-3444`.
  - Per step, serially: build once, then per substep ×4 gravity, `warm_start_apply` over every slot, and integrate.
  - `solve_all_colors` ×12. Wide colors (≥256 slots) dispatch one `pool.scope` each. Narrow colors run inline.
  - Then restitution, `store_and_swap`, write-back.
- Dispatch count, W=8/16 (09-10 census on the shipped pool): 331.6 allocations and 1.27 MB per step. At one 4 KiB block per scope that is ≈ 310 scopes per step, and the count does not depend on W.
- **B1 confirmed by reading, not yet by test:**
  - Frozen manifolds never enter `columns` (`colored.rs:1606`).
  - `store_and_swap` rebuilds `warm_write` from `columns` alone (`:3115-3123`), and `rebuild` zeroes every slot (`warm_start.rs:261-275`).
  - So after the first frozen step, the island's impulses are gone.
  - By contrast, Box2D v3 moves a sleeping island's contacts, impulses included, into its sleeping solver set.
- `warm_start_apply` is already movability-guarded "for the moment this runs in parallel" (`colored.rs:1813-1840`). Slots are laid out in color order (`:1597-1618`). This matters for lever L7.
- The pool has no barrier or broadcast primitive; only `scope` exists (`thread_pool.rs:307`, `:503`).
- Constraint: nothing is built or run until the owner opens a quiet window. The work lands on the merged tip of D:/wt/joltab, after the in-flight merge commits, in its own lane.

## 1. Harness verdict: not like for like. Fixes before any ratio is quoted again

| # | Aspect | Verdict | Fix |
|---|---|---|---|
| H1 | Timed window | **Broken.** 20 warm steps, then Criterion sampling by time on a world that is never reset. The simulated window depends on W (`OPEN-QUESTIONS.md:5867-5869`), so T(1)/T(W) compares different stretches of the collapse. Jolt times steps 0..500 from t=0. | Replace Criterion with a fixed-window runner (`harness = false`): spawn, then 500 steps, one `Instant` pair per `schedule.run`, no warm-up. Report Jolt's metric (500 / Σt), a per-frame CSV (Jolt: `-f`), and sub-windows [0,100) and [100,500). |
| H2 | Friction | 0.5 vs 0.2. **Fix.** | Bench `:144`, `:181` → 0.2. Jolt's combine √(0.2·0.2) equals our max(0.2, 0.2). |
| H3 | Damping | Jolt 0.05 linear and angular; we have no rigid-body damping. **Fix on Jolt's side.** Adding an engine feature just to match a benchmark is rejected. | Jolt patch: `mLinearDamping = mAngularDamping = 0` in `PyramidScene.h`. |
| H4 | Motion quality | Not recorded on 09-10. PerformanceTest runs both Discrete and LinearCast unless told otherwise. **Fix.** | Jolt `-s=Pyramid -q=Discrete -t=W -f`. |
| H5 | Build | Our bench profile has `lto=false`; Jolt is LTO. **Fix.** | Root `[profile.parity]`: `inherits = "bench"`, `lto = "fat"`. No RUSTFLAGS. The headline is LTO on both sides. |
| H6 | Tree and toolchain | The 09-10 row was very likely windows-gnu (C2) and predates A7a/A7b (C3). **Fix.** | Every run records: HEAD, the `rustc -vV` host line (must say msvc), `merge-base --is-ancestor` for the S5 commit and for KE16, and the Jolt binary's SHA-256 and compiler. No receipt, no quoted ratio. |
| H7 | boyko configuration | The `parallel_broadphase` knob does nothing on AllPairs (C1), and the bench comment claiming otherwise is wrong. | Two rows, both simulation-identical and therefore both like for like: **cfg-A** = today's (colored, `parallel_solve = W>1`, AllPairs, `simd_solve` off); **cfg-B** = cfg-A + `Grid` + `simd_solve`. The runner asserts equal final-pose bytes between them. Fix the comment. |
| H8 | Work equality | Speculative contacts (Jolt 0.02 m vs our separation ≤ 0) and Jolt's manifold reduction change the contact set. **Accepted, with a receipt check.** | Untimed receipt runs: per frame, manifold count and top-box y on both sides (Jolt through a counting `ContactListener` behind `-receipt`). If resting-window manifold counts differ by more than 10 %, the headline also carries time per manifold per step. |
| H9 | Sleeping; iterations and substeps | **Kept.** Both have sleeping off, which is the scene's purpose. Both do 12 sweeps (Jolt 10+2, ours 4×(1+2)). They are different algorithms; equalising them would measure a configuration neither engine ships. Each engine runs its defaults. | None. Report time per manifold-sweep beside the ratio. |
| H10 | Jolt pair cache | Kept on: it is an engine feature. | Diagnostic arm `-no_pair_cache` (patch: `mUseBodyPairContactCache = false`) prices the contact-reuse lever on Jolt's side. |
| H11 | Jolt version | v5.3.0 stays the headline, for continuity of the series. | Second column v5.6.x, same recipe. It measures Jolt's own per-manifold-friction change on this machine (their figure: −15 % on Pyramid). |
| H12 | Protocol | 09-10 protocol kept. | Prebuilt binaries invoked by path. Interleaved by W (Jolt, boyko), order reversed on alternate passes. 5 passes, medians. Load receipt before and after each timed region. |

**Rules for quoting:**
- boyko/Jolt = median over passes of mean step time, per W, with the cfg named, on the msvc host, with H6 receipts attached.
- Every number from 2026-09-09 and 2026-09-10 is labelled "pre-A7, likely gnu" and never compared against new rows.

## 2. P0: the per-stage profile (gates every lever)

### Decision 1: the instrument is the kernel's own profiler, not `Instant`s in physics code
- **What:**
  - System level: the existing `SystemSpan` (`zones.rs:151-205`).
  - Inside the solve: permanent `zone!` sites. boyko_physics gains a `boyko_diag` dependency; it is already transitive via boyko_ecs, so no new crate enters the graph. lib.rs gains the `profiling_partition!(Engine)` line.
  - A cold kernel accessor maps system names to zone ids.
  - The runner folds after each step, **outside** the step's `Instant` pair, and diffs `LifetimeAcc.total/count` per zone to get per-step values.
- **Why:**
  - `SystemSpan` is already compiled into the default `dev` tier, so the 09-10 numbers already paid its disarmed cost (one load and a branch).
  - Armed, a span costs 2 `rdtsc` plus a push into a per-thread lane ring: ≈ 40 ns, with no shared writes. At ≈ 340 samples per step that is ≈ 14 µs, about 0.1 % of T(8). The A/A tests below verify this rather than assume it.
  - It sees wave counts and the solve split, which no wall-clock bench can.
  - It stays in the engine and folds away under `shipping`. That follows Principle 0: a capability is a kernel feature, not a per-crate adapter.
- **Rejected alternatives:**
  - *Probe systems between physics systems:* each adds an executor round, and at W>1 a wake. That perturbs exactly the dispatch term being measured, and it cannot see inside the solve.
  - *An external sampling profiler:* it loses the wall-clock structure at each W (idle waits are misattributed), and it cannot be gated. Kept only as a fallback if closure fails.
  - *Inferring zone ids from build order:* ids are minted from a global counter, and the runner builds several schedules.

### Zone set (tier Deep, the same gate as `SystemSpan`)

| Zone | Site | Runs per step |
|---|---|---|
| (SystemSpan) gather, select_broadphase, broadphase, narrowphase, build_graph, solve_colored, apply | executor | 1 each |
| `phys_solve_build` | `build_bodies` + `build_columns` | 1 |
| `phys_gravity`, `phys_warm_apply`, `phys_integrate` (position + inertia) | substep loop | 4 each |
| `phys_pass_biased` / `phys_pass_relax` | around `solve_all_colors` | 4 / 8 |
| `phys_color_wide` / `phys_color_narrow` | per color, classed by `color_slots < MIN_PARALLEL_SLOTS_PER_COLOR` (the inline gate's own predicate, which does not depend on W) | colors × 12 |
| `phys_restitution`, `phys_store`, `phys_write_back` | tail | 1 each |
| `phys_sleep_begin`, `phys_sleep_freeze` (capture + restore), `phys_sleep_end` | sleeping on only | 1 / 2 / 1 |
| counters `phys_slots_wide`, `phys_slots_narrow`, `phys_np_pairs`, `phys_np_manifolds`, `phys_np_points`, `phys_bp_pairs` | once per step | 1 each |

### Scenes and arms (boyko; msvc; `[profile.parity]`)

| Id | Scene | Config | W | Window (steps) | Purpose |
|---|---|---|---|---|---|
| J-A | Jolt pyramid, gap 0.5, sleeping off | cfg-A | 1, 2, 4, 8, 16 | 0..500 | Disarmed run = parity row; armed run = profile |
| J-B | same | cfg-B | 1, 8 | 0..500 | Per-stage price of Grid and `simd_solve` |
| J-P1 | same | cfg-A with `parallel_solve` on | 1 | 0..500 | ω₁: per-wave cost with no cross-thread wake |
| J-C | same, plus a canary system `.after(narrowphase).before(build_graph)` spinning 0.05·T(W) | cfg-A | 1, 8 | 0..500 | Proves the gate can fail |
| J-Son | same, sleeping on (default threshold and frames) | cfg-A | 1, 8 | 0..1000 | Against Jolt `-allow_sleep`: the sleeping floors |
| J-S0 | same, sleeping on, `sleep_threshold = 0` | cfg-A | 1 | 0..500 | Bookkeeping cost; the A4-hoist gate |
| R | resting pile, gap 0 (`sleeping_pipeline.rs`'s `pyramid_sleeping_off` scene), colored, `PhysicsConfig::default` | serial / `parallel_solve` on | 1 / 8 | 600..1100 | Default-configuration resting cost per stage |
| R-ref | same scene through `add_physics_systems::<SoftStepSolver>` | default | 1 | 600..1100 | Prices D1 |
| R-S | same scene, sleeping on | default | 1, 8 | 300..800; the runner asserts every dynamic row is frozen at step 300 (A7-R2 froze at 248), otherwise the row is void | Sleeping floor F |
| S16 | 16-box stack | cfg-A | 1, 8 | 0..300 | Small-scene regression guard |

Also run in the same window:
- The broadphase crossover (`cargo bench -p boyko-physics --bench broadphase`, MEASUREMENT-QUEUE §4.4). It calibrates `GRID_LO`/`GRID_HI`.
- The pre-instrument binary of the same tip, for A/A2.

**Jolt arms:**
- Timed: v5.3.0 Distribution, damping 0.
- `-no_pair_cache`.
- `-allow_sleep`.
- A `Release` build with `-p` at W=1 and W=8, used for stage *shares* only (FindCollisions vs solve).
- v5.6.x.
- `-receipt`, untimed.

**Estimated quiet-machine time:** about 40 minutes (5 passes ≈ 5.5 min each, plus receipts and polls). The owner confirms the window first.

### Derived quantities
For each span s, t_s(W) is the median over passes of the window mean. A pass-level sketch of the arithmetic follows the definitions.

- 𝒮 (serial set) = {gather, select_bp, broadphase, narrowphase, build_graph, apply} ∪ {solve_build, gravity, warm_apply, integrate, restitution, store, write_back, narrow colors, sleep_\*}.
- **S(W) = Σ_{s∈𝒮} t_s(W). The measured serial fraction is f = S(1)/T(1).** It replaces the Amdahl fit. The fit, f_fit from T(1) and T(8), is printed beside it: f_fit − f is what the fit misattributed.
- P(1) = Σ wide-color spans at W=1 with `parallel_solve` off. At that setting inline execution is pure work.
- **Identity:** T(W) = S(1) + I(W) + P(1)/W + L(W) + g(W) + u(W), where:
  - I(W) = S(W) − S(1): interference (SMT, stealing and spinning workers, cache).
  - L(W) = t_wide(W) − P(1)/W: parallel loss.
  - g = T − Σ system spans: executor gap.
  - u = solve span − Σ in-solve zones: unzoned residue.
- waves = count of wide spans per step; E(W) = P(1)/(W·t_wide(W)); ω(W) = L(W)/waves.
- **Gap attribution at W=8:** boyko's terms beside Jolt's stage shares from the Jolt profile run, plus the pair-cache price Δ_J = T_J(no cache) − T_J.

```text
# Sketch of the reduction for one pass (the driver takes medians over passes).
# Needs the armed profile rows of that pass; W1_off = W=1 with parallel_solve off.
serial = [gather, select_bp, broadphase, narrowphase, build_graph, apply,
          solve_build, gravity, warm_apply, integrate, restitution, store,
          write_back, narrow_colors, sleep_*]
system_spans = [gather, select_bp, broadphase, narrowphase, build_graph,
                solve_colored, apply]
in_solve     = [solve_build, gravity, warm_apply, integrate, pass_biased,
                pass_relax, restitution, store, write_back, sleep_*]
S1 = sum(t[s][W1_off] for s in serial)
P1 = t[wide_colors][W1_off]
f  = S1 / T[W1_off]                        # the measured serial fraction
for W in (2, 4, 8, 16):
    S_W   = sum(t[s][W] for s in serial)
    I     = S_W - S1                       # interference
    L     = t[wide_colors][W] - P1 / W     # parallel loss
    g     = T[W] - sum(t[s][W] for s in system_spans)
    u     = t[solve_colored][W] - sum(t[s][W] for s in in_solve)
    E     = P1 / (W * t[wide_colors][W])
    omega = L / waves[W]
    # T[W] should equal S1 + I + P1/W + L + g + u (checked in the gates below)
```

### Proof that the instrument does not perturb the step (every check can fail)
- **A/A0 (band).** Two disarmed J-A runs per pass, paired by step index. B(W) = max over passes of |median_k(t₂/t₁) − 1|. If B(W) > 2 %, the window is not quiet: void the window.
- **A/A1 (perturbation).** Armed vs disarmed, same pairing. Pass iff |median − 1| ≤ max(B(W), 0.5 %) at every W.
- **A/A2 (permanent sites).** Disarmed instrumented vs the pre-instrument binary at W=1 and W=8, same criterion. If it fails, the physics zones move to a tier `dev` does not compile.
- **Canary.** On J-C vs J-A, all three must hold:
  - the canary span reads 0.05·T ± 5 %;
  - step wall time rises by the same amount within the A/A0 band;
  - the A/A1 statistic computed on J-C vs J-A **fails**.
  The canary exceeds the 2 % band ceiling by construction, so a green here means the gate is blind.
- **Determinism.** Armed, disarmed, cfg-A and cfg-B final-pose bytes are all equal.
- **Closure.** Any of these stops the analysis as an instrument defect:
  - u(W) > 3 % of the solve span;
  - g < 0;
  - Σ system spans > step wall time;
  - the region overflow counter ≠ 0;
  - wave counts that differ across W ≥ 2, or that disagree with 12 × (colors with ≥ 256 slots) computed from the `phys_slots_*` counters.

## 3. Levers (ranked by P0; nothing is built before P0 reports)
"Build if" means the predicted gain at the target W is at least 5 % of T there **and** at least twice the A/A band, so its gate can see it.

| # | Lever | Predicted gain (formula; bracket today) | Build if P0 shows | Simulation values | Cost / risk | Gate | Decides |
|---|---|---|---|---|---|---|---|
| L1 | A4 `island_of` hoist | Up to +2.44 % back on the awake sleeping-on row (measured). 0 on parity. | Always; already prescribed | Bit-identical | Trivial | §8 R2 on J-S0: B/A ≤ 1 within A/A. Sleep suites green. | me |
| L2 | Broadphase `Auto`/Grid default, with the crossover calibrated | t_bp − t_grid. **Upper bound 0.9 ms** (09-09): ≤ 4.5 % of T(1), ≤ 7.5 % of T(8) | J-B broadphase span drops; crossover below 1240 | Bit-identical (`production_grid_equals_all_pairs`) | Low | That test, J-B, S16 no regression | me |
| L3 | `simd_solve = true` by default on the colored path | Measured 1.96× (09-06, pre-A7): ≈ 0.49·(t_wide + t_narrow) at W=1; ≈ 0.49·(P(1)/8 + t_narrow) at W=8. Mostly a W=1 lever. | J-B and R improve at W ∈ {1, 8}, no regression at 2/4/16 | Bit-identical to the scalar colored solve | Low | O7 bit suite, J-B | me. Affects no default world until D1. |
| L4 | `parallel_solve = true` by default | Measured directly: J-A at W vs W=1 | t_wide(W) < t_wide(1) at every W ≥ 2 on J and R; S16 flat | Bit-identical ({1,N}) | Trivial | {1,N} oracles, S16 | me |
| L5 | Parallel narrowphase | t_np(1)·(1 − 1/(E(8)·8)). Using E from the colored solve is conservative: this is one fork-join per step, not ≈ 310. | Clears the rule. Prior: most likely the largest serial stage (≈ 6.7k manifolds, ≈ 23k points, full SAT and clip per pair). Unmeasured. | **Bit-identical by construction:** emit in pair order (count, prefix sum, emit: the Grid emit's shape); read every hint from a pre-step snapshot (`prefetch_remapped` made unconditional); commit the axis cache serially in pair order. This is exact because each key is touched once per frame and linear-probe lookups for distinct keys do not depend on insertion order. | Medium: per-chunk output views (today single-threaded `ScratchBuildView`s) | Manifold stream, axis table and pose bytes equal for serial vs W ∈ {1, 8, 16} over 600 steps on J and R; realized gain ≥ 0.6× predicted | me |
| L6 | Color dispatch: narrow colors, chunk quantisation, per-wave cost | ≤ L(W) + t_narrow(W)·(1 − 1/W) − waves·ω_b | L(8) + t_narrow(8) ≥ 10 % of T(8). Fork: if ω(8)·waves ≥ 0.5·L(8), build a new pool primitive (one persistent phased region per solve pass: W tasks walk the colors with an atomic block cursor and a spin-then-park barrier, the Box3D stage / Rapier broadcast shape), then re-derive `MIN_PARALLEL_SLOTS_PER_COLOR` and `MIN_SLOTS_PER_CHUNK` from its microbenched ω_b. Otherwise retune the constants only (chunk count a multiple of W). | Bit-identical (holds for any partition) | High (new primitive plus loom models) / low (constants) | {1,N} oracles, loom, P0 re-run | me |
| L7 | In-solve serial phases: `warm_start_apply` per wide color, in parallel | t_warm(1)·(1 − 1/(E·W)) − 4·waves·ω | Only after L6, otherwise ω makes it a loss, and only if t_warm(8) ≥ 3 % of T(8). `build_columns` and `store_and_swap` are deferred: the canonical-order hash insert blocks them. | **Bit-identical.** Slots are color-ordered and a dynamic body appears once per color, so each body's float-add order stays the serial one. The movability guard is already in place. | Low after L6 | {1,N}, pose bytes | me |
| L8 | Friction per patch (D2) | ρ = 1 − (points + 3·manifolds)/(3·points) of the row solves, from P0 counters: 0.376 at 22 975 points / 6671 manifolds. Upper bound ≈ ρ·(t_colors + t_warm). External: Jolt −15 % on this exact scene (the v5.6 column re-measures it here); Rapier −25 %. | t_colors(8) + t_warm(8) ≥ 25 % of T(8) after L5–L7 | **Changes values** (both solvers, a new warm-key class, the AVX2 kernel) | High | New incline stick/slip threshold at tan θ = μ, twist-stop, A7-R1/R2 rest; moved pins listed from the red set | **owner** |
| L9 | Contact reuse, the Jolt pair-cache analogue (D4) | ≤ h·t_np(W). External: Δ_J from `-no_pair_cache`. | t_np still ≥ 20 % of T(8) after L5, and Δ_J ≥ 10 % of Jolt's step | **Changes values** (replayed manifolds; the ghost-contact risk Box3D documents) | Medium | Tolerance-set gates plus A7 rest tests | **owner** |
| L10 | Sleeping on by default (D3) | **Parity: exactly 0.** Default resting world: T_R − F, F from R-S. Today F ≥ gather + bp + np + graph + apply + bookkeeping. | Owner wants resting-world cost cut. Prerequisites: B1 green; D1 (sleeping exists only on the colored path); to bring F toward Jolt's floor, extend the skip to broadphase, narrowphase and graph for frozen–frozen pairs (my work, only if D3 = yes). J-Son vs Jolt `-allow_sleep` shows how far apart the floors are. | **Changes values** | Medium | A7-R1/R2, `support_loss_wakes_sleepers`, the sleep suites. The colored tests that assume sleeping off and go red on the flip are the moved-pin list. | **owner** |

D1 (the colored solver as the default path) **changes values** relative to `SoftStepSolver`.
- R vs R-ref prices it.
- Prerequisite, my work: wire SDF, soft bodies and scene sync onto the colored path, which today has no public entry (`plugin.rs:282-289`).
- Pins expected to move: `softstep.rs`, `simd_o1.rs`, `sdf_collision.rs`, `soft_body_sp1.rs` (unconfirmed). The flip's red set is the authoritative list.

## 4. Order of work

**Step 0. Bugs and record hygiene. No machine load; all mine.**
1. **B1 test first.** Settle and freeze a pile (sleeping on), keep it frozen K steps, `wake_all`, step once.
   - Assert the warm-seed `seeded` count (`colored.rs:1595`) equals the point count on the wake step. Today I expect 0.
   - Secondary check: the pile's downward velocity on the wake step is no larger than on the step before freezing.
   - If red, the fix gets its own short plan: carry frozen manifolds' warm entries (through the row remap) into `store_and_swap` in canonical order, consistent with C3.
2. L1 (A4 hoist).
3. Instrument, H1–H12 harness and runner, Jolt patch and builds.
4. Record hygiene. Annotate the `OPEN-QUESTIONS.md` 09-10 entry: C1, the broadphase reason at `:5803`; C2, gnu not msvc, and W>1 never measured on msvc. Add a MEASUREMENT-QUEUE §10 recipe. Repoint the `jolt_parity_pyramid -- full_step/N` cross-checks in §8/§9 to the runner.

**Step 1. The quiet window** (owner confirms it): P0 plus the parity re-run.

**Step 2. Mine, bit-identical, ranked by P0's predicted gain / cost:** L2, L3, L4 (config flips), then L5, L6, L7 as each clears its build-if rule. Re-profile after each.

**Step 3. Owner decisions, the only genuine values calls,** each presented with its P0/P2 price: D1 colored default, D2 friction per patch, D3 sleeping default on, D4 contact reuse.

**Step 4.** Re-run parity under H1–H12 and quote with receipts.

## Implementation plan (developer)
1. `crates/boyko_physics/tests/frozen_island_warm_start.rs`: the B1 test.
2. `crates/boyko_physics/src/resources.rs` `begin_step` (`:3490-3523`): hoist the `island_of` read slice next to `island_starts()`.
3. `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`: add `pub fn system_zones(&self) -> impl Iterator<Item = (&'static str, u16)> + '_`. It is cold (setup only) and yields nothing when `SYSTEM_ZONES_COMPILED` is false.
4. `crates/boyko_physics/Cargo.toml`: path dependency on `boyko_diag`. `src/lib.rs`: `boyko_diag::profiling_partition!(Engine);`. New `src/profiling.rs`: `declare_zone!` for the zone table in §2. Sites go in `solver/colored.rs` (`solve_colored_inner`, `solve_all_colors` color loop) and `systems.rs` (np and bp counters).
5. Root `Cargo.toml`: `[profile.parity]`. `crates/boyko_physics/Cargo.toml`: the `jolt_parity_pyramid` bench becomes `harness = false`.
6. Rewrite `benches/jolt_parity_pyramid.rs` as the fixed-window runner.
   - Flags: `--workers --steps --scene {jolt,rest,s16} --gap --sleeping --threshold --solver {colored,reference} --cfg {a,b} --parallel-solve --arm-profiler --canary-frac --csv`.
   - Keeps the `n == 1240` anti-vacuity check. Adds the freeze-step assert for R-S and the wave-invariance check.
   - Output: per-step CSV with wall ns, per-zone ns and counts, and a final pose hash.
7. `crates/boyko_physics/benches/jolt_parity/pyramid_scene.patch`, plus the build recipe in the bench's header: damping 0, `-no_pair_cache`, `-allow_sleep`, `-receipt`. Builds: v5.3.0 Distribution and Release, v5.6.x Distribution.
8. `tools/` driver.
   - Interleaves runs, collects receipts, reduces results, evaluates A/A0–A/A2, the canary, closure and determinism.
   - Launches the prebuilt binaries by path and reads their output as bytes, not `text=True`.

## Validation
- `boyko_ecs`: `system_zones_are_unique_and_assigned` (dev tier), and an empty iterator when the tier is folded.
- `boyko_physics`: `physics_zones_count_exactly`.
  - Armed, 3 steps on a small colored pile.
  - Exact counts: build = 3, gravity = warm = integrate = 12, biased = 12, relax = 24.
  - wide + narrow color spans = n_colors·12·3, with the 256-slot split recomputed independently from `color_offsets`.
- `instrumented_step_is_bit_identical_armed_and_disarmed`: 100 steps, pose bytes.
- The B1 test.
- Every lever gate in §3; the bit-identical levers use ≥ 600-step pose-byte oracles on J and R.
- No new `debug_assert!`: the instrument adds no invariants on the hot path.

## Open questions
1. If R-S does not freeze by step 300 on the tip (A7-R2 was measured on the kernel as committed), the R-S row is void and D3 is priced from J-Son alone.
2. The v5.6.x recipe may need new CMake options. If Jolt's defaults changed beyond friction, record them in the H6 receipt rather than patching them away.
3. A/A2 needs the pre-instrument binary built from the merged tip, so the instrument lands only after the in-flight merge commits.

Checklist: no new runtime data structures (P0 is instrumentation plus a harness), so repr, alignment and false sharing are N/A. The new pool primitive (L6) and the B1 fix each get their own plan with layout and loom sections if P0 and the test trigger them.

Key files:
- D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/warm_start.rs
- D:/wt/joltab/crates/boyko_physics/src/resources.rs
- D:/wt/joltab/crates/boyko_physics/src/broadphase_policy.rs
- D:/wt/joltab/crates/boyko_ecs/src/ecs/core/profiling/zones.rs
- D:/wt/joltab/crates/boyko_ecs/src/ecs/core/profiling/lifetime.rs
- D:/wt/joltab/crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs
- D:/wt/joltab/docs/OPEN-QUESTIONS.md
- D:/wt/joltab/docs/MEASUREMENT-QUEUE.md
- D:/tmp/jolt/JoltPhysics/PerformanceTest/PerformanceTest.cpp
- D:/tmp/jolt/JoltPhysics/PerformanceTest/PyramidScene.h