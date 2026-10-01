# Research: Rapier (rapier3d) as a second parity comparator for the J-T pyramid

No build, run or process was started. This was web and source research only, so the D: free-space check did not apply. Line numbers are 1-based at tag `v0.36.0` (commit `b716d375efc0201003f0cd9ef7168eee0b62c177`). They come from the Sourcegraph stream API plus 1; that API was checked to be 0-based. The GitHub-page reads gave inconsistent line numbers and were not used.

## Brief summary (TL;DR)
- **Version:** the latest release is **rapier3d 0.36.0**. The CHANGELOG dates it "24 September 2026"; crates.io published it at 2026-09-25T08:16Z. **0.35.0 (2026-08-08) was a rewrite**: persistent contact graph, a staged multithreaded solver, island-based sleeping, new contact defaults, and the `simd-stable` feature removed. Any Rapier number from ≤0.34 describes a different engine, so pin `=0.36.0`. The per-step counters (pairs, contacts, constraints) are only filled from 0.36.0.
- **Solver:** substepped soft PGS. Each step runs `num_solver_iterations` (default **4**) substeps with `dt/4`. Each substep runs:
  - one biased pass (`num_internal_pgs_iterations` = 1), which by default solves **normal rows only**;
  - position integration;
  - one unbiased pass (`num_internal_stabilization_iterations` = 1), which solves normal + friction.
  - Contact softness is 30 Hz / ζ=10, and **60 Hz against fixed bodies**. The bias cap is 3.0 m/s. There is **no penetration slop** in the bias.
- **Contacts:** speculative. `normalized_prediction_distance` = **0.02**, the same as Jolt's `mSpeculativeContactDistance`. At most 4 solver points per manifold. The default friction model is **Simplified**: per manifold, 2 tangent rows + 1 twist row, which is Jolt's 5.6 layout. `Coulomb` gives 2 rows per point, which is ours.
  - Contact **recycling is on by default with a 5 cm drift threshold**. Ours and Jolt's cache use 1 mm.
- **Threads:** add the `parallel` feature (rayon) and call `configure_thread_pool(W)`. One awake island is graph-coloured (128 colours + a serial overflow colour) and run by W persistent workers in completion-gated stages with work-stealing.
  - The docs claim results do not depend on thread count. A bitwise cross-thread-count test exists only under `enhanced-determinism`, which **cannot be combined with `simd8`**.
- **Equal-work unit:** Rapier exposes no row counter, but the row-iteration count follows directly from the parameters (formula below). With `Coulomb`, `friction_in_bias_pass=true` and 2 stabilisation iterations, Rapier's per-step row count is **exactly our 36·Np**.

## Approaches in state-of-the-art engines

### Rapier 0.36.0

**Solver (substeps / PGS)**
- `init.rs:113,119`: `num_solver_iterations = base + max_extra; params.dt /= num_solver_iterations`. Solver iterations are substeps.
- Per-substep order (`staged_island_solver/worker.rs`, substep loop):
  1. apply velocity increments (gravity, plus gyroscopic correction when enabled);
  2. constraint update fused with warm-start, colour by colour;
  3. `for pgs_iter in 0..num_pgs_iterations` → `solve_pass(wo_bias=false)`;
  4. integrate positions, with a per-substep speed cap;
  5. `for stab_iter in 0..num_internal_stabilization_iterations` (`worker.rs:713`) → `solve_pass(wo_bias=true)`;
  6. restitution runs once after all substeps, and only if a body is bouncy.
- Friction gating (`solve.rs:100`): `solve_friction = wo_bias || friction_in_bias_pass || num_internal_stabilization_iterations == 0`.
- Normal row (`contact_with_coulomb_friction.rs`, `update`): `rhs_wo_bias = max(dist,0)/dt`; `rhs_bias = clamp(dist·erp_inv_dt, -max_corrective_velocity, 0)`. `cfm_factor` = 1 (rigid) when separated. Static vs dynamic softness is chosen by whether one side is a fixed body.
- `erp_inv_dt = ω/(dt·ω + 2ζ)` (`SpringCoefficients::erp_inv_dt`).

**IntegrationParameters defaults** (`src/dynamics/integration_parameters.rs`, `impl Default` at L406)

| Parameter | Default | Line |
|---|---|---|
| `dt` | 1/60 | L409 |
| `contact_softness` | `contact_defaults()` = 30 Hz, ζ=10 | L60 |
| `static_contact_softness` | `contact_static_defaults()` = 60 Hz, ζ=10 | L70 |
| `warmstart_coefficient` | 1.0 | |
| `num_internal_pgs_iterations` | 1 | L414 |
| `num_internal_stabilization_iterations` | 1 | L415 |
| `num_solver_iterations` | 4 | L416 |
| `normalized_allowed_linear_error` | 0.005 | |
| `normalized_max_corrective_velocity` | 3.0 | |
| `normalized_prediction_distance` | 0.02 | L422 |
| `normalized_max_linear_velocity` | 400 | |
| `max_ccd_substeps` | 1 | |
| `contact_clustering` | true | L425 |
| `contact_recycling` | true | |
| `normalized_contact_recycle_distance` | 0.05 | |
| `friction_in_bias_pass` | false | |
| `warmstart_joints` | false | |
| `length_unit` | 1 | |
| `friction_model` | `FrictionModel::default()` = `Simplified` | L434; enum L16, `#[default] Simplified` L25 |

- `allowed_linear_error` is documented as "NOT a deadzone on the position-correction bias". Its only uses are in CCD (`ccd_solver.rs:130,182`). The doc on the `allowed_linear_error()` helper method still says "0.001 … won't attempt to correct"; that is doc rot.
- There is no `num_additional_friction_iterations` in 0.36. The nearest knobs are `friction_in_bias_pass` and the per-body `additional_pgs_iterations` / `additional_solver_iterations`.

**Contact generation**
- `pair_update.rs:329-355`: parry `contact_manifolds(..., effective_prediction_distance)`, where the distance = `prediction_distance + contact_skin_sum` (+ soft-CCD sweep if enabled; default 0).
- Solver-contact filter (`pair_update.rs:585`): keep a point if `dist < prediction`, or if it closes to below `prediction` within `dt`.
- Reduction to `MAX_MANIFOLD_POINTS = 4` (`lib.rs:291`) in `manifold_reduction.rs`: deepest point, then farthest, then two extremal points.
- Clustering only applies when a pair has more than one manifold (`pair_update.rs:470`), so it does not apply to box-box pairs.
- Recycle drift is `recycle_distance` for active pairs, `min(recycle, prediction)` otherwise (`pair_update.rs:728`).

**Friction combine**
- `CoefficientCombineRule` default is `Average` (`coefficient_combine_rule.rs:39`). The effective rule is `max(rule1, rule2)` by priority: GeometricMean > ClampedSum > Max > Multiply > Min > Average.
- **The collider default friction is 0.5** (`collider.rs:1189`). Default restitution is 0 and `contact_skin` is 0.

**Multithreading**
- Features: `parallel = ["dep:rayon","std","parry3d/parallel"]` (`crates/rapier3d/Cargo.toml`).
- `PhysicsPipeline::configure_thread_pool(n)` (`physics_pipeline/mod.rs:268`) builds a rayon pool with threads named `rapier-worker-{i}`. `step` then runs inside `pool.install`. Without it, the step uses the pool of the calling thread.
- **`num_threads()` (L300) returns `None` unless a dedicated pool is set**, even though its doc says otherwise.
- The solver uses `rayon::current_num_threads()` workers (`solve.rs:382`). Workers are spawned with `rayon::in_place_scope`; worker 0 runs inline (`init.rs:662`).
- Staged solver module doc (verbatim): "contacts are colored so same-color constraints touch pairwise-disjoint bodies (SIMD-packed per color); persistent workers claim batches between spin-barrier stages … Deterministic: results depend only on the coloring, not on batch distribution."
  - `LAYOUT_REF_WORKERS = 8` fixes the parallel-vs-serial colour split independent of pool size.
  - `NUM_SOLVER_COLORS = 129`: 128 colours + overflow (`solver_contact_graph.rs:12`).
  - `StageSync` advances "when all its work units completed, not when all workers arrive". It spins 10,000 times, then calls `yield_now` (`sync.rs:118`).
- Awake bodies are solved as one active set; `min_island_size` was removed (0.35-beta changelog).
- The broad phase is a parry BVH with `par_chunks` updates. The narrow phase uses `rayon::broadcast` (`narrow_phase/contacts.rs:246`).

**Determinism**
- Changelog 0.35-beta: "`enhanced-determinism` can now be combined with `parallel`, with results bitwise identical for any thread-pool size … It remains incompatible with `simd8`". The incompatibility is enforced by a compile error at `lib.rs:19-21`.
- `enhanced-determinism` = `simba/libm_force` + `parry3d/enhanced-determinism`, and it uses `IndexMap` instead of hashbrown (`utils/mod.rs:168`).
- `determinism.mdx`: default builds are "locally deterministic". Parallel results "don't depend on the number of threads".
- `tests/parallel_path_parity.rs` is `#![cfg(all(feature="enhanced-determinism", feature="serde-serialize"))]`. Without that feature, thread-count independence is claimed but not tested.

**SIMD**
- SIMD is always on through `wide`, 4 lanes. `simd8` gives 8 lanes, f32 only, and needs AVX2 (`x86-64-v3` provides it).
- `parallel` and `simd8` combine without restriction. `enhanced-determinism` and `simd8` do not.
- `block-solver` (2×2 blocks) is off by default in 3D.

**Sleeping / CCD**
- Sleeping: `RigidBodyBuilder::can_sleep(false)` (`rigid_body.rs:2057`) → `RigidBodyActivation::cannot_sleep()`, which sets both thresholds to −1 (`rigid_body_components.rs:1404`). Defaults are 0.05 u/s and 0.5 s; sleep is decided per island.
- CCD: `max_ccd_substeps = 0` "disables **all** CCD" (field doc near L298). By default, fast bodies sweep against fixed colliders automatically.
- Also default-on: `gyroscopic_forces_enabled` (builder default true, `rigid_body.rs:1732`; setter L2118).
- Angular cap is π/4 per step (47.1 rad/s at 60 Hz), the same value as Jolt's `mMaxAngularVelocity = 0.25π·60`.

**Counting API**
- `NarrowPhase::contact_pairs()` (`queries.rs:194`) iterates every graph edge, touching or not. This equals `counters.cd.ncontact_pairs` (`solve.rs:158`) and is the broad-phase pair count.
- `ContactPair::manifolds()` (L492) gives raw manifolds. `solver_manifolds()` (L499) gives what the solver sees (clusters if clustered).
- `ContactManifoldData::solver_contacts` (L735) holds `SolverContact { dist, … }`. `dist` is as of the last *full* update, so it can be stale on recycled pairs.
- `PhysicsPipeline::counters` is a public field ("benchmarking only", mod.rs:47), enabled by default and reset at each step start (`substep.rs:325`):
  - `counters.solver.nconstraints` = solver manifolds + joints;
  - `counters.solver.ncontacts` = solver points;
  - both summed over CCD substeps (`init.rs:608-609`).
  - Timers stay 0 unless the `profiler` feature is on.
- There is no row counter.

**Published benchmarks**
- No dimforge-published native pyramid benchmark against Jolt or PhysX was found. The 2025 review and the Q2-2026 report give no Rapier-vs-Jolt numbers.
- The only numbers are third-party runs through **Godot integrations**: godot.rapier.rs v0.35 post and the Godot forum post of 2026-08-09, both by ughuuu.
  - Scene: 3D pyramid of 3,795 bodies (benchmarks-repo README), Jolt v5.5.0 via Godot, macOS arm64 builds.
  - Blog: Rapier 14.6 ms (2.8 cores) vs Jolt 26.9 ms (4.7 cores). Forum: 19 ms vs 24 ms.
  - Hardware was not stated.
- Rapier's own `examples3d/stress_tests/pyramid3.rs` is a variant of Jolt's pyramid layout, but deliberately uses shrunken 1.95 cubes on a 2.25 pitch. Its comment: "a perfectly symmetric brick is a degenerate, marginally-stable configuration". J-T is exactly that symmetric brick (pitch 2.0 = box size).

## Comparative table

| Aspect | Rapier 0.36 default | ours J-T | Jolt 5.6 J-T |
|---|---|---|---|
| Steps | 4 substeps × (1 biased + integrate + 1 relax) | 4 × (1 biased + integrate + 2 relax) (`colored.rs:5123-5198`) | 1 warm-start + 10 velocity + 2 position (DOSSIER F3) |
| Friction in biased pass | no | yes ("normal + friction", `colored.rs:5147`) | n/a |
| Friction rows | 2 + twist per manifold | 2 per point | 2 + twist per manifold |
| Softness | 30 Hz/ζ10; fixed bodies 60 Hz | 30 Hz / 10 (`resources.rs:605-606`) | Baumgarte 0.2, 0.02 slop |
| Bias cap | 3.0 | 4 m/s (DOSSIER L51) | — |
| Speculative distance | 0.02 | 0 today (V2: 20 mm) | 0.02 |
| Contact reuse | 5 cm drift | 1 mm | 1 mm / cos(1°) (`PhysicsSettings.h:68,71`) |
| Gyroscopic | on | none | off (`BodyCreationSettings.h:103`) |
| Row-iterations per step | 8Np + 12Nm | 36Np | 12Np + 30Nm |

## Key algorithms and techniques
- **Row-iterations per step (derived from `worker.rs` / `solve.rs`, no joints):** `R = S·[P·(Np + fib·F) + Q·(Np + F)]`.
  - S = `num_solver_iterations`, P = pgs iterations, Q = stabilisation iterations, fib = `friction_in_bias_pass`.
  - F = 3·Nm for Simplified, 2·Np for Coulomb.
  - Np = Σ `solver_contacts.len()`; Nm = solver manifolds with ≥1 point.
  - Warm-start is not counted, which is the DOSSIER convention.
- **Illustration only (not a measurement):** if Rapier's contact set matched Jolt's receipt (Np 31,112, Nm 8,489), defaults would give about 351k rows against Jolt's 566k velocity rows. The contact set must be measured, not assumed.

## Pitfalls and mistakes
1. **Friction:** the default is 0.5, not 0.2. Set it on the floor and on every box.
2. **Floor stiffness:** `static_contact_softness` is 60 Hz, so the floor is stiffer than ours.
3. **Recycling:** at 5 cm, most narrow-phase work in a settled pile is skipped. That is an unequal-work advantage.
4. **Default friction passes:** `friction_in_bias_pass=false` together with Simplified friction gives fewer rows than ours.
5. **Thread receipt:** `num_threads()` is `None` without a dedicated pool.
6. **W semantics:** Rapier's pool has W threads and the caller blocks. Jolt's tag is `th(N+1)`.
7. **Split step cost:**
   - The deferred BVH optimisation runs as `rayon::spawn` during the solve and is joined in the next step (`solve.rs:67,82-91`). Part of each step's cost lands in the following step; on a 1-thread pool it runs inline.
   - Workers spin before yielding, which inflates CPU-time "cores used" numbers.
8. **Counters:** they are empty on 0.35.x. `SolverContact.dist` can be stale on recycled pairs, which affects any "wholly speculative" classification.
9. **Features:** `simd8` + `enhanced-determinism` does not compile. Cargo features unify per build, so each feature arm needs its own build.
10. **Quoting the Godot numbers:** they are not comparable to J-T (different scene, Godot integration, arm64, Jolt 5.5).
11. **Query pipeline:** it has been ephemeral since 0.28 and has no update cost. `final_broad_phase_time` (the AABB refresh "to keep user scene queries valid") is inside `step`.

## Relevant academic works
None consulted in this pass.

## Applicability to boyko-engine — compact spec

**Cargo (harness at `D:/tmp/rapier-parity`)**
- `rapier3d = { version = "=0.36.0", default-features = true, features = ["parallel"] }`
- Optional harness features:
  - `simd8 = ["rapier3d/simd8"]`: 8-lane arm, the analogue of our 8-lane `simd_solve`.
  - `det = ["rapier3d/enhanced-determinism"]`: needed for a bitwise thread-count receipt; mutually exclusive with `simd8`.
- `[profile.release] lto="fat", codegen-units=1`. Rapier's own release profile has these commented out, and it does not propagate to dependents anyway.
- MSRV is 1.86, edition 2024.

**Scene (all arms)**
- From `PyramidScene.h:34-49`.
- Floor: `RigidBodyBuilder::fixed()` at (0,−1,0) + `ColliderBuilder::cuboid(50,1,50)`.
- 15 layers, i = 0..14. For j, k in [i/2, 15−(i+1)/2):
  - `cuboid(1,1,1)` at (−15+2j+(i odd?1:0), 1+2.5i, −15+2k+(i odd?1:0));
  - 1,240 bodies.
- Every collider: `.friction(0.2).restitution(0.0)`. Optionally `.density(1000.0)` (Jolt default); pyramid3.rs notes density does not matter dynamically.
- Bodies: `.can_sleep(false)`.
- `gravity (0,−9.81,0)`, `dt = 1/60`, `max_ccd_substeps = 0`.
- Run W with `world.configure_thread_pool(W)`. Receipt: `num_threads()==Some(W)`.

**(a) Rapier defaults**
- `IntegrationParameters::default()` plus only the scene invariants above (`max_ccd_substeps=0`).
- Body defaults are kept, including gyroscopic on.
- Rows = 8Np + 12Nm.

**(b1) Boyko-mirror (row formula identical to ours: 36Np)**
- `num_solver_iterations=4`, `num_internal_pgs_iterations=1`, `num_internal_stabilization_iterations=2`
- `friction_in_bias_pass=true`, `friction_model=FrictionModel::Coulomb`
- `contact_softness` default (30 Hz, ζ10), `static_contact_softness = SpringCoefficients::contact_defaults()` (30 Hz)
- `normalized_max_corrective_velocity=4.0`
- `normalized_prediction_distance=0.02` (= Jolt, = our V2 20 mm)
- `normalized_contact_recycle_distance=0.001` (= our τ and Jolt's cache)
- `warmstart_coefficient=1.0`; clustering default (no effect on box-box)
- bodies `.gyroscopic_forces_enabled(false)`

**(b2) Jolt-row-equal (arithmetic only, unvalidated)**
- Simplified friction, fib=false, prediction 0.02, recycle 0.001.
- Jolt's 12Np + 30Nm is matched by (S,P,Q) = (2,1,5) or (1,2,10).
- Either changes Rapier's substep dt, and so its soft-contact discretisation.

**Per-step counts (read after `step`)**
- pairs = `contact_pairs().count()`, cross-check with `counters.cd.ncontact_pairs`.
- Over `solver_manifolds()` with non-empty `solver_contacts`: manifolds, points, `dist > 0` points, and wholly-speculative manifolds.
- Raw pre-reduction points = `manifolds()[..].points.len()`.
- Cross-check with `counters.solver.{nconstraints,ncontacts}`.
- Rows from the formula.
- Floor vs box split by `data.rigid_body1/2`; lateral vs vertical by `data.normal`.

## Open questions for the architect
- Should (a) keep Rapier's default contact reuse (5 cm) and gyroscopic setting, or should the J-T invariants include reuse = 1 mm?
- Which arm is the headline: (a), (b1), or a (b2) variant?
- Do we score Rapier's pose fidelity on J-T even though dimforge calls the symmetric brick marginally stable?
- Which W convention do we use against Rapier (pool size vs Jolt's N+1)?
- Should `simd8` be a separate arm, given that it rules out the `det` receipt?

## Sources
[1] https://raw.githubusercontent.com/dimforge/rapier/master/CHANGELOG.md — 0.35/0.36 changes: rewrite, defaults, features, determinism
[2] https://crates.io/api/v1/crates/rapier3d — 0.36.0 publish time, feature list
[3] https://github.com/dimforge/rapier/pull/1017 — 0.36.0 release PR
[4] https://raw.githubusercontent.com/dimforge/rapier/v0.36.0/src/dynamics/integration_parameters.rs — defaults, FrictionModel, SpringCoefficients
[5] https://raw.githubusercontent.com/dimforge/rapier/v0.36.0/src/dynamics/solver/staged_island_solver/{mod,init,worker,solve,sync}.rs — substep loop, friction gating, threading
[6] https://raw.githubusercontent.com/dimforge/rapier/v0.36.0/src/geometry/narrow_phase/pair_update.rs and src/geometry/contact_pair.rs — contact generation, counting API
[7] https://raw.githubusercontent.com/dimforge/rapier/v0.36.0/src/counters/ — counters
[8] https://raw.githubusercontent.com/dimforge/rapier/v0.36.0/src/pipeline/physics_pipeline/{mod,solve}.rs, src/pipeline/physics_world.rs — thread pool, step, deferred BVH
[9] https://raw.githubusercontent.com/dimforge/rapier/v0.36.0/src/dynamics/{rigid_body,rigid_body_components,coefficient_combine_rule}.rs, src/geometry/collider.rs — body and collider defaults
[10] https://raw.githubusercontent.com/dimforge/rapier/v0.36.0/website/docs/user_guides/templates/determinism.mdx and crates/rapier3d/tests/parallel_path_parity.rs — determinism claims and scope
[11] https://raw.githubusercontent.com/dimforge/rapier/v0.36.0/examples3d/stress_tests/pyramid3.rs and examples3d/b3d_large_pyramid.rs — dimforge's pyramid scenes
[12] https://godot.rapier.rs/blog/v0-35-0/ and https://forum.godotengine.org/t/physics-engine-comparison-rapier-vs-godot-vs-box2d-3d-vs-jolt/142786 — third-party Godot benchmarks
[13] https://github.com/Ughuuu/benchmarks-repo — scene sizes for [12]
[14] https://dimforge.com/blog/2026/01/09/the-year-2025-in-dimforge/ — no Jolt/PhysX comparison
[15] https://sourcegraph.com/.api/search/stream — line numbers at `v0.36.0`
[16] Local files:
- D:/tmp/jolt/wt-v5.6.0-parity/Jolt/Physics/PhysicsSettings.h:45-114
- D:/tmp/jolt/wt-v5.6.0-parity/Jolt/Physics/Body/BodyCreationSettings.h:103-112
- D:/tmp/jolt/wt-v5.6.0-parity/PerformanceTest/PyramidScene.h:34-49
- D:/wt/joltab/crates/boyko_physics/src/resources.rs:603-606
- D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs:5123-5198
- C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/jolt-gap/DOSSIER.md:1-112