# Physics step on `merge/ke16-into-ecsnative` (D:/wt/joltab): serial structure, instruments, Jolt parity, and what sleeping can and cannot do

This is a read-only analysis. Nothing was built or run, and nothing in D:/wt/joltab was touched. Line references are to that tree unless another path is given.

## 0. Direct answer: can sleeping close the remaining gap to Jolt?

**No, for three reasons from the code:**

1. **Both sides of the parity number have sleeping turned off on purpose.**
   - Jolt: `D:/tmp/jolt/JoltPhysics/PerformanceTest/PyramidScene.h:44` sets `settings.mAllowSleeping = false; // No sleeping to force the large island to stay awake`.
   - boyko: `D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs:217` sets `cfg.sleeping = false`.
   - The scene measures an awake large island. No sleeping mechanism can move the 2.47×.
2. **boyko's sleep (O8) only skips the solve and integrate of a frozen island.**
   - Gather, broadphase, narrowphase, graph build and apply still visit every body and pair. None of `systems.rs:292-462` or `:1021-1040` reads the sleep state.
   - The only skip is in `solver/colored.rs:1600-1608`.
   - Integration still runs over all rows and is then undone for frozen rows (`colored.rs:3276-3297`, `:3407-3426`).
   - Jolt's broadphase pair finding only walks active bodies (`Jolt/Physics/PhysicsSystem.cpp:697`, `:857-904`), so sleeping bodies drop out of it.
   - My inference, not measured: with sleeping on in both engines, a resting pile costs Jolt close to nothing, while boyko keeps paying broadphase, narrowphase and graph build. The ratio would get worse, not better.
3. **The gap looks like serial work, and sleeping doesn't touch it.** The Amdahl fit puts 0.43–0.53 of a step in serial work. The tree has a large serial surface (§1). How that splits between stages has never been measured. The tool to measure it already exists (§2).

## Corrections to the brief

These change how the 2026-09-10 numbers read.

| # | Brief says | Tree says |
|---|---|---|
| C1 | "broadphase sequential below MIN_PARALLEL_BODIES=4096" | The bench never sets `broadphase`, so it runs the default `AllPairs` (`resources.rs:451`). That is an O(n²) serial loop at every W (`systems.rs:304-322`). The bench's `cfg.parallel_broadphase = workers > 1` (`bench:216`) does nothing on that path; it is only read on the `Grid` path (`systems.rs:330-336`). `MIN_PARALLEL_BODIES` (`resources.rs:664`, used at `:1734`) is never reached. `docs/OPEN-QUESTIONS.md:5803-5804` names the wrong reason. The measured cost is about 0.9 ms at 1240 bodies (`OPEN-QUESTIONS.md:334`): about 4.5 % of T(1)=20.3 ms and 7.5 % of T(8)=11.9 ms. |
| C2 | The 09-10 row is "msvc" | It was almost certainly windows-gnu. The entry only says `rustc 1.98.1` (`OPEN-QUESTIONS.md:5751`). The same-day sibling entry says `x86_64-pc-windows-gnu` (`:5829`). The msvc recipe commit `97bcf826` is **not** an ancestor of the doc commit `ca582e72`. The TLS fix `e6115223` **is**, but it only halves the per-spawn `thread_local` cost on gnu. `51631371` also made the steal gate differ by host. boyko vs Jolt at W>1 has never been measured on msvc. |
| C3 | The ratios describe the current tree | They predate A7a and A7b. S5 alone adds +10 % at W=1 and +7 % at W=4 on this scene. The current ratio is unmeasured. |
| C4 | "default PhysicsPlugin schedule" | No production crate wires physics. Only tests and benches call `add_physics_*`; `boyko_app`, `boyko_demo` and `src/` do not. "Default" means whatever the caller picks. No public entry combines the colored solve with SDF, scene sync or soft bodies: `add_physics_colored_solve` passes `with_sdf=false, soft=false, scene_sync=false` (`plugin.rs:282-289`). |

## 1. The step as the parity bench wires it (`add_physics_colored_solve`)

The systems are chained with `.after` (`plugin.rs:556-643`). The executor runs one physics system per round, so stages never overlap.

| # | System | Where | Threads | Gate |
|---|---|---|---|---|
| 1 | `physics_integrate` | `systems.rs:136-171` | does nothing | Returns early under `SolverOwned` (`:144-146`), because `owns_integration()` is true (`colored.rs:3469`, `plugin.rs:527-533`) |
| 2 | `physics_gather` | `systems.rs:205-270` | 1 | Serial query walk |
| 3 | `select_broadphase` | `broadphase_policy.rs:173-193` | 1 | In `Manual` (the default) it only counts bodies |
| 4 | `physics_broadphase` | `systems.rs:292-343` | 1 on AllPairs | Grid: the CSR build is serial; the emit is parallel only if `parallel_broadphase && n≥4096 && lanes≥2` (`resources.rs:1734-1754`) |
| 5 | `physics_narrowphase` | `systems.rs:369-462` | 1, always | No pool code |
| 6 | `physics_build_graph` | `systems.rs:1021-1040` → `ConstraintGraph::build` `resources.rs:2635` | 1 | Islands and greedy coloring |
| 7 | `physics_solve_colored` | `systems.rs:1077-1099` → `colored.rs:3219-3444` | mixed (table below) | |
| 8 | `physics_apply` | `systems.rs:1154-1186` | 1 | |

The only pool dispatch sites in rigid physics are `resources.rs:1750` (Grid emit) and `colored.rs:2812` (color solve).

**Inside the solve, per step:**

| Phase | Where | Times per step | Threads |
|---|---|---|---|
| `build_bodies` and `build_columns` (per point: warm-table probe plus pushes into 26 columns; canonical order) | `colored.rs:3265`, `:1548-1673` | 1 | 1 |
| `apply_gravity` | `:3345-3347` | 4 | 1 (SIMD) |
| `warm_start_apply` over **all** slots | `:3354`, `:1805` | 4 | 1 |
| `solve_all_colors`, biased | `:3357-3366` | 4 | per color, gated (below) |
| `position_integrate` (scalar by a measured decision) and `refresh_inertia` | `:3372-3384` | 4 each | 1 |
| `solve_all_colors`, relax | `:3387-3398` | 8 | per color, gated |
| `apply_restitution` loop over all slots | `:3402`, `:3042-3092` | 1 | 1 |
| `store_and_swap` (one hash insert per point) | `:3405`, `:3110-3126` | 1 | 1 |
| `write_back` | `:3434`, `:3130-3142` | 1 | 1 |

**Gates:**

| Knob | Value | Where |
|---|---|---|
| `parallel_solve` | default false (`resources.rs:491`); bench sets `W>1` (`bench:215`) | Whole-step gate: `parallel_solve && widest_color_slots ≥ 256` (`colored.rs:3337-3338`) |
| `MIN_PARALLEL_SLOTS_PER_COLOR` | 256 | `colored.rs:228`; colors below it run inline on the calling worker (`:2780-2799`) |
| `CHUNKS_PER_WORKER` / `MIN_SLOTS_PER_CHUNK` | 6 / 64 | `:257` / `:310`; `n_chunks = min(W·6, slots/64)`, capped at the group count; fewer than 2 chunks runs inline (`:2833-2846`) |
| `parallel_broadphase` | default false (`resources.rs:483`) | Does nothing on AllPairs |
| `simd_solve` | default false (`:470`) | The bench leaves it off |
| `simd` (O1) | default true (`:465`) | |

**Serial work that the named candidates missed:** 4× `warm_start_apply` over every slot, `build_columns`, `store_and_swap`, the restitution loop, gather, apply and graph build. The named ones were narrowphase, broadphase, inline colors and chunk quantisation. How the time splits across all of these is unmeasured.

## 2. Existing instruments for a per-stage profile

| Instrument | Where | What it covers | Cost when off | Cost when on |
|---|---|---|---|---|
| `SystemSpan`, one span per system run | `boyko_ecs/src/ecs/core/profiling/zones.rs:151-205`; opened at `schedule.rs:1390` (inline) and `:1563` (worker); ids minted in `schedule_builder.rs:469-479` | One span per physics system | Removed at compile time unless the tier is ≥ Deep. The default `BOYKO_PROFILE` (unset → `dev`, tier 2 = Deep; `boyko_diag/build.rs:195`, `profile.rs:250`) compiles it in; then it costs one `.bss` load and a branch per system. `shipping` removes it (`profile.rs:252`). | 2 `rdtsc` plus one 24-byte ring push per system run. At 8 systems per step that is negligible against 12–20 ms (estimate). |
| `RoundProbe` | `zones.rs:222`, `schedule.rs:700` | Executor dispatch rounds, **not** the solver's `pool.scope` waves | same | 2 samples per round |
| `zone!` / `declare_zone!` | `boyko_diag/src/profiling_abi.rs:530`, `:641` | Any code site | Folded at compile time or one load | Guard open and close |
| `Profiler::lifetime(zone)` | `profiling/store.rs:1051` | Per-zone accumulator for the whole session | — | — |
| KE16 / `sleeping_pipeline` / `row_identity_churn` benches | `benches/ke16_solve_in_system.rs` etc. | Wall time of the whole solve or step only | n/a | n/a |

**Problems collecting the numbers from `jolt_parity_pyramid`:**

- **No fold.** The bench calls a bare `Schedule::run` (`bench:248-251`), so nothing folds the samples. It would need to insert a `Profiler`, call `arm` (`store.rs:747`), and call `fold_frame` after each run (`profiling/mod.rs:152-158`).
- **No public way to find a system's zone id.** `SystemMeta::zone()` is public (`system_meta.rs:283`), but `Schedule.systems` is `pub(crate)` (`schedule.rs:124`). Today the ids can only be inferred from build order; anything cleaner needs a kernel accessor.
- **No split inside the solve.** Separating `build_columns`, warm start, color waves and store needs zone sites in `colored.rs`. `boyko_physics/Cargo.toml` has no `boyko_diag` dependency, so that is an edit to the crate being measured.
- **Solver waves are not counted anywhere.** The counters are private (`OPEN-QUESTIONS.md:5795-5797`).

## 3. Is the Jolt parity comparison like for like?

| Aspect | boyko (`jolt_parity_pyramid.rs`) | Jolt (`PerformanceTest`) | Same? |
|---|---|---|---|
| Geometry, 1240 boxes, floor, zero convex radius | transcribed (`:130-197`) | `PyramidScene.h:27-46` | yes |
| Sleeping | off (`:217`) | off (`:44`) | yes |
| dt, collision steps, gravity | 1/60, 1, −9.81 | `cpp:52`, `Update(cDeltaTime, 1, …)` `:380`, default gravity | yes |
| Iterations | 4 substeps × (1 biased + 2 relax) = 12 sweeps, plus 4 warm-start applies | 10 velocity + 2 position (`PhysicsSettings.h:78-81`) | same sweep count, different work per sweep |
| Friction | 0.5 (`:144`, `:180`), combined with max (`colored.rs:1702`) | 0.2 default (`BodyCreationSettings.h:103`) | **no** |
| Damping | none | 0.05 linear and angular (`:105-106`) | **no** |
| Speculative contacts | only points with separation ≤ 0 (`OPEN-QUESTIONS.md:180-183`) | 0.02 m (`PhysicsSettings.h:47`) | **no** |
| Contact reuse | full SAT and clip on every pair, every step | Pair cache skips collision detection when relative motion is < 1 mm and < 2° (`PhysicsSettings.h:64-67`, `PhysicsSystem.cpp:1014-1017`) | **no, and it matters most on a resting pile** |
| Broadphase / narrowphase | serial AllPairs / serial | parallel jobs; `OptimizeBroadPhase` before timing (`cpp:336`) | **no** |
| Thread count W | pool of W workers | W−1 workers plus the calling thread (`cpp:299-308`) | equivalent |
| Which steps are timed | 20 warm steps, then Criterion (sample size 20, default warm-up and measurement) on a world that is never reset, so the window depends on W (`OPEN-QUESTIONS.md:5867-5869`) | 500 steps from t=0 including the collapse, each `Update` timed (`cpp:90`, `:368-385`) | **no** |
| Build | bench profile `lto=false`, cgu=1, x86-64-v3 (`Cargo.toml:114-127`) | Distribution, LTO on, AVX2, MinGW g++ (`build/CMakeCache.txt:28,34,349,403-406`) | **no** |
| Motion quality (Discrete / LinearCast) | n/a | not recorded anywhere | unknown |

**Verdict:** the 2.47× is a real wall-clock ratio between two configurations. It is **not** "same work, worse dispatch."
- T(1)/T(W) cancels each engine's per-step constant, but not its serial/parallel mix, and that mix is exactly what differs: Jolt has a parallel broadphase and narrowphase plus the pair cache; boyko has a serial O(n²) broadphase and a serial full narrowphase.
- T(1) ≈ 1.0 is equal time for different amounts of work (`OPEN-QUESTIONS.md:5412-5414`).
- The bench already runs the colored path, so two settings can be A/B-tested without changing any simulation value. `broadphase = Grid` produces the same pair set (`broadphase_policy.rs:42-47`). `simd_solve = true` is bit-identical to the scalar colored solve (`resources.rs:194-198`). Either one changes the configuration being compared, and has to be stated as such.

## 4. Narrowphase

- **Loop:** pairs in `(min,max)` order (`systems.rs:394-461`). For a box-box pair: `read_hint` (`:431`), then `box_box_contact` (`:432-434`), then `axis_cache.set` (`:438`), then a push to `manifolds` or `sensor_overlaps` (`:444-459`).
- **S5 realized-patch check:** `box_box.rs:673-688`.
  - When the SAT's best edge beats the best face by more than `SAT_EPS` (1e-5, `:60`), the face patch is built first.
  - The edge wins only if `edge.depth < patch_depth − FACE_AXIS_PREFERENCE` (0.005 m, `:122`; `patch_depth` at `:740-744`).
  - Hysteresis (`:695-704`, ratio 1.05 at `:55`): an edge hint never holds a pair whose best axis is a face.
  - A held face that realizes no patch falls back to the already-built best face (`:710-722`).
  - The cold fallback is `edge_fallback` (`:723`, `:778-807`).
  - On the resting pile: about 1470 face-first builds and about 5.2 fallbacks per step (`MEASUREMENT-QUEUE.md:389-396`).
- **Pairs are independent.** `box_box_contact` depends only on the two poses and the hint. The counters in it are `cfg(test)` only (`:746-756`). No per-pair heap allocation.
- **What blocks splitting it across workers:**
  1. `BoxAxisCache` is one in-place open-addressed table that is read and then written inside the same loop (`axis_cache.rs:15-31`). `set` inserts on a miss with linear probing (`:319-354`), so concurrent inserts race and change where entries land.
  2. Manifolds are emitted in pair order, and that order feeds `ConstraintGraph::build` and the canonical warm-start store. A split needs per-pair output slots plus prefix compaction, the same count-then-emit shape the Grid emit already uses (`resources.rs:1769+`).
  3. The output columns are single-threaded `ScratchBuildView`s.
- **Half of a split already exists.** `prefetch_remapped` (`axis_cache.rs:394-422`) copies every pair's hint into a per-pair column before any write, but only on frames where rows changed. Each key is touched once per frame (`:17-22`). So always prefetching and moving the `set`s into a serial pass in pair order would reproduce today's table exactly. That is a direction, not a design.
- **Size (my estimate, not counted):** about 9–11k candidate pairs, from the bounding radius √3 (`systems.rs:1198-1203`) and a pitch of 2. At step 600 on the zero-gap h15 pile there are 6671 manifolds and 22 975 points (`MEASUREMENT-QUEUE.md:199-200`).

## 5. Colored solver vs the reference solver

| | `SoftStepSolver` (reference) | `ColoredSoftStepSolver` |
|---|---|---|
| Entry | `add_physics_systems/_sdf/_soft/_with_scene_sync::<S>` | `add_physics_colored_solve` only (`plugin.rs:282-289`) |
| Order | manifold order, serial | color order; per-color parallel under the §1 gates |
| Sleeping, `simd_solve` | no effect | O8 sleeping; O7 AVX2 (`colored.rs:1905-1926`) |
| SDF, scene sync, soft bodies | available | **no public entry** |
| Results | reference values | different values, checked against tolerances (`plugin.rs:268-277`) |

- **What `add_physics_colored_solve` changes:**
  - sets `colored = true`;
  - inserts `ConstraintGraph` and `IslandSleep` (`plugin.rs:480-495`);
  - registers `physics_build_graph` (`:607-617`);
  - registers `physics_solve_colored` in place of `physics_solve_step` (`:625-629`).
- **Tests that would move if colored became the default:** the ones wired to `SoftStepSolver` that pin values: `softstep.rs`, `simd_o1.rs`, and `sdf_collision.rs` and `soft_body_sp1.rs` through `add_physics_sdf::<S>`. The last two have no colored entry at all. I found these by grepping for `to_bits` / hex / `GOLDEN`; I did not open each pin to confirm it.
- **`simd_solve`:** off by default (`resources.rs:470`), read once per step (`colored.rs:3307`). With parallel dispatch, chunks snap to 8-group cohorts (`:2899-2919`).

## 6. Friction today

- **Rows:** for every contact point, 1 normal row plus a 2-DOF coupled tangent cone, `|λt| ≤ μ·λn` (`colored.rs:2037-2082`; reference solver `soft_step.rs:600-720`).
- **Per-manifold values stored per point:** the tangent basis is computed once per manifold (`:1697`) but stored per point (`:1380-1385`). Likewise μ = max(μa, μb) (`:1702`, `:1387`).
- **Warm start:** keyed per point as `pack(a,b,feature_id)` (`:1728-1747`; layout at `warm_start.rs:113-128`).
- **Effective masses** for the normal and both tangents are recomputed on every sweep (`:2003-2007`, `:2042-2051`).
- **Scale:** a 4-point patch costs 4 normal plus 8 tangent row solves per sweep, times 12 sweeps.

**Where per-patch (anchor) friction would plug in:**
- **Build:** `push_manifold_points` (`:1680-1767`) has the manifold-level data. The group CSR (`group_start` / `color_group_start`, `:1619-1633`) already has one entry per manifold, so it can carry per-patch friction columns.
- **Solve:** solve friction once per group after that group's normal points in `solve_color` (`:1956-2083`). The AVX2 kernel already maps one lane to one manifold group (`:2086-2090`).
- **Warm start:** needs a new per-manifold key class.
- **Reference solver:** needs the same change if the two solvers must match.
- **Results:** this changes simulation values, unlike Grid or `simd_solve`.

## 7. Sleeping

- **Defaults:** off (`resources.rs:494`); threshold `1e-4` on speed² (`:428`); 60 frames (`:433`). It only works on the colored path, because `IslandSleep` is only inserted there (`plugin.rs:480-495`).
- **Work per step when on:**
  - `rekey_rows` (`resources.rs:3290-3312`);
  - `begin_step`, O(rows), two `island_of` calls per row (`:3456-3535`);
  - skipping frozen islands' manifolds (`colored.rs:1606-1608`);
  - capturing frozen rows (`:3282-3297`);
  - integrating every row, then restoring the frozen ones (`:3407-3426`);
  - `write_back_awake` (`:3152-3167`);
  - `end_step`, O(rows) twice (`resources.rs:3582-3637`).
  - Gather, broadphase, narrowphase, graph build and apply are **not** skipped.
- **Possible defect (from reading the code; no test covers it; effect unmeasured):** a frozen island loses its warm-start impulses.
  - Frozen manifolds are never pushed into the solver's columns (`colored.rs:1606-1608`).
  - `store_and_swap` clears the whole table (`warm_start.rs:261-275`) and re-inserts only those columns (`colored.rs:3115-3123`).
  - So after the first frozen step, the frozen island's accumulated impulses are gone. When it wakes, a loaded pile starts from zero contact impulse.
  - This matters if the plan leans on sleeping. The discriminating test: wake a frozen pile with `wake_all` and compare its first-step normal impulses with a pile that never slept.
- **Principle-0 note:** `IslandSleep` keeps per-row state in `std::Vec` (`resources.rs:3096-3112`: `asleep`, `below_count`, `frozen_islands`, `energy`).
- **Tests that set sleeping explicitly:**
  - Off: `alloc_frame_attribution.rs:1936`, `alloc_frame_census.rs:1714`, `apply_row_alignment.rs:344`, A7-R1 in `sleep_settles_box_piles.rs:1693`, and the parity bench (`:217`).
  - On: `sleep_settles_box_piles.rs:588`, `support_loss_wakes_sleepers.rs:241`, `sleeping_pipeline_o8.rs:116`.
  - Toggled: `row_identity_remap.rs:357/495`, `row_keyed_state_defect_a.rs:235`, and the `row_identity_churn` / `sleeping_pipeline` benches.
  - Every other colored-path test assumes off implicitly, for example `bodytype_determinism_golden.rs` (90 steps) and `colored_acceptance_o5.rs` (up to 300 steps). A default flip would move any of them in which an island stays under the threshold for 60 straight steps.

## Key files
- D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs
- D:/wt/joltab/crates/boyko_physics/src/plugin.rs
- D:/wt/joltab/crates/boyko_physics/src/systems.rs
- D:/wt/joltab/crates/boyko_physics/src/resources.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs
- D:/wt/joltab/crates/boyko_physics/src/narrowphase/box_box.rs
- D:/wt/joltab/crates/boyko_physics/src/narrowphase/axis_cache.rs
- D:/wt/joltab/crates/boyko_ecs/src/ecs/core/profiling/zones.rs
- D:/tmp/jolt/JoltPhysics/PerformanceTest/PyramidScene.h
- D:/tmp/jolt/JoltPhysics/PerformanceTest/PerformanceTest.cpp
- D:/tmp/jolt/JoltPhysics/Jolt/Physics/PhysicsSettings.h
- D:/tmp/jolt/JoltPhysics/Jolt/Physics/PhysicsSystem.cpp
- D:/tmp/jolt/build/CMakeCache.txt