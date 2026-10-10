# Research: Would a sleeping mechanism close boyko's remaining performance gap with Jolt? Contact cost, narrowphase parallelism and sleep defaults in Jolt, PhysX, Box2D v3/Box3D, Rapier and Bepu

## Brief summary (TL;DR)
- **Sleeping cannot close the measured gap, because Jolt's pyramid benchmark switches sleeping off.** `PyramidScene.h` sets `settings.mAllowSleeping = false; // No sleeping to force the large island to stay awake` for every box [1]. Box2D's own pyramid benchmarks also call `b2World_EnableSleeping(worldId, false)` [14]. boyko already has island sleeping (O8, `PhysicsConfig::sleeping`, default `false`, `D:\wt\joltab\crates\boyko_physics\src\resources.rs:323,494`), and the parity bench sets `cfg.sleeping = false` (`D:\wt\joltab\crates\boyko_physics\benches\jolt_parity_pyramid.rs:217`). Turning sleeping on for boyko only would change what is being measured, not how fast the engine is.
- **Where the other engines get their speed on an awake pile:**
  - They skip narrowphase work for pairs that barely moved. Jolt's body-pair cache skips collision detection when the relative pose changed by less than 1 mm and less than 2° [2][3]. Box3D skips the full SAT when the cached axis still holds [16]. Rapier 0.35 `contact_recycling` skips pairs that drifted less than 0.05 [20]. Gregorius (Valve): "expect a speed-up by an order of magnitude or more" [24].
  - They run narrowphase in parallel: Jolt in batches of 16 bodies [4], Box3D with a minimum of 20 contacts per task [17], Rapier in blocks of 64 pairs [21].
  - They solve fewer friction rows per manifold.
  - They split large islands (Jolt) or colour the constraint graph (Box3D, Rapier) [5][18][22].
- **boyko's narrowphase** runs the full 15-axis SAT and clip for every pair on every step. The manifold buffer is "cleared and refilled each step" (`D:\wt\joltab\crates\boyko_physics\src\systems.rs:345-365`). `axis_cache.rs` only biases which axis is chosen; it never skips the SAT (`D:\wt\joltab\crates\boyko_physics\src\narrowphase\axis_cache.rs:1-13`).
- **Friction model: the industry has moved to friction per manifold.**
  - Jolt v5.6.0 (July 2026): "15 % faster, 40% less memory for Pyramid test" [6].
  - Rapier 0.29: simplified friction model, "25% speedup on scenes involving many contacts (like large stacks)" [19].
  - Box3D and Bepu: friction at the manifold centre plus twist [13][23].
  - PhysX: patch friction is the only model left, "up to four scalar solver constraints per pair" [8][9].
  - boyko: friction per point (`D:\wt\joltab\crates\boyko_physics\src\solver\soft_step.rs:704-715`), with `MAX_CONTACT_POINTS = 4` (`D:\wt\joltab\crates\boyko_physics\src\math.rs:34`).
- **The bench is pinned to Jolt v5.3.0,** which still uses friction per point [7]. Re-pinning to v5.6 or later would, if the upstream 15% holds on this machine (unmeasured), move the boyko/Jolt ratio against boyko.

## Approaches in current engines

### Jolt Physics (v5.3.0 is the bench pin; master is v5.6+)
- **PerformanceTest settings** [1][2][10][11]:
  - Pyramid: 15 layers, 1240 boxes, half-extent 1, convex radius 0 ("to force more collisions"). Sleeping is forced off per body.
  - Time step `cDeltaTime = 1/60`, `Update(cDeltaTime, 1, …)` (one collision step), `max_iterations = 500`.
  - Solver defaults: `mNumVelocitySteps = 10`, `mNumPositionSteps = 2`, `mBaumgarte = 0.2`, speculative distance 0.02, penetration slop 0.02.
  - Feature defaults: `mUseBodyPairContactCache = true`, `mUseManifoldReduction = true`, `mUseLargeIslandSplitter = true`.
  - Body defaults: `mFriction = 0.2`, linear and angular damping 0.05.
  - The `-no_sleep` flag exists, but it does not matter for Pyramid.
- **Thread-count meaning** [12]:
  - PerformanceTest prints `num_threads + 1`.
  - The thread pool comment says: "the number of concurrent jobs is 1 more because the main thread will also run jobs while waiting for a barrier".
  - So Jolt's "threads = 1" means the main thread only.
- **Narrowphase parallelism** [3][4]:
  - Number of FindCollisions jobs: `max(max_concurrency==1?1:2, min(ceil(active/cActiveBodiesBatchSize), max_concurrency))`.
  - Each job claims bodies with `mActiveBodyReadIdx.fetch_add(cActiveBodiesBatchSize)`. Found body pairs go into per-job queues (`mBodyPairQueues`).
  - Batch constants: `cActiveBodiesBatchSize = 16`, `cNarrowPhaseBatchSize = 16`, `cApplyGravityBatchSize = 64`, `cIntegrateVelocityBatchSize = 64`, `cSetupVelocityConstraintsBatchSize = 256`. `cMaxConcurrency = 32`.
  - On a body-pair cache hit, `GetContactsFromCache` copies the previous frame's manifolds and collision detection is skipped [3].
- **Large islands** [5]:
  - `LargeIslandSplitter` implements Chen et al. 2007, section "PARALLELIZATION METHODOLOGY".
  - 32 splits (a `uint32` mask); the last split is non-parallel.
  - An island is split when it has more than 128 constraints plus contacts (`cLargeIslandTreshold`).
  - Splits with fewer than 32 items are merged into the non-parallel split (`cSplitCombineTreshold`). Batch size is 16.
- **Manifold:**
  - `MaxContactPoints = 4` ("Max 4 contact points are needed for a stable manifold").
  - `PruneContactPoints` keeps 4 points: first the point with the largest weighted distance from the centre of mass (combined with penetration depth), then the furthest point from it, then the furthest points on each side of that segment to maximise area. Input capacity is 64 [13a].
- **Friction:**
  - v5.3.0: every `WorldContactPoint` has `mNonPenetrationConstraint`, `mFrictionConstraint1` and `mFrictionConstraint2`, i.e. 3 rows per point [7].
  - v5.6.0: 2 linear rows plus 1 angular row per manifold, applied at the average contact point. Linear limit = μ·Σ normal impulse; angular limit = μ·Σ(distance to the average point × normal impulse). "15 % faster, 40% less memory for Pyramid test" [6].
  - The friction constraint is solved before non-penetration ("non-penetration is more important than friction") [13b].
- **Published scaling** [15]:
  - Ragdoll scene: 4.9× at 8 threads, 5.7× at 16 threads (c6i Xeon).
  - Beyond about 16 cores, "the lock free operations that are used to manage the contact cache dominate".
  - AMD across core complexes: 27% slower at 8 threads when jobs are scheduled over 4 core complexes instead of 1.
  - Update of 10 Mar 2023: "Removed remark that Jolt does not scale well when all objects are on a single pile. This has been fixed."

### PhysX 5
- **Friction** [8][9]:
  - Patch friction (`ePATCH`) is the default: "Up to two contact points … selected as friction anchors", with 2 perpendicular axes per anchor.
  - Described as "only up to four scalar solver constraints per pair" and "the most stable results at low solver iteration counts and … quite inexpensive". This is a search-result snippet from the PhysX 3.4 docs; I did not fetch that page.
  - 5.6.0-107.0: `PxFrictionType` deprecated; ONE_DIRECTIONAL and TWO_DIRECTIONAL removed; `eIMPROVED_PATCH_FRICTION` removed and now always behaves as if set.
  - At most 32 friction patches per contact manager.
- **TGS vs PGS** [8][25]:
  - TGS applies friction on every iteration. PGS applies it only in the last 3 position iterations and in velocity iterations, unless `eENABLE_FRICTION_EVERY_ITERATION` is set.
  - "Each TGS iteration is generally a little slower than PGS."
  - Default iterations: 4 position, 1 velocity.
- **Sleeping defaults** [26]: `wakeCounterResetValue = 20*0.02` (0.4 s); sleep threshold `5e-5·speed²`.
- **Contact generation** [27]: PCM "potentially generates fewer contacts" and may affect stacking in tall stacks.
- Not researched: PhysX CPU narrowphase and solver job structure.

### Box2D v3 and Box3D (Erin Catto)
- **Box2D v3 manifold** [28]: at most 2 points (2D); `tangentImpulse` per point plus one `rollingImpulse` per manifold.
- **Box3D** (released 2026-06-30) [29]:
  - Manifold "may be 1 to 4 valid points". The reduction keeps 4: the best touching point, the farthest point, then maximum area, with a `bias = 0.95` "pecking order" [16].
  - Friction is central: a 2D tangent impulse at the manifold centre plus `twistImpulse`. `rollingImpulse` is used only for spheres and capsules [13][23b].
- **Box3D defaults** [30]:
  - World: `enableSleep = true`. Body: `enableSleep = true`, `sleepThreshold = 0.05 m/s`, `enableContactRecycling = true` ("improves performance but may lead to ghost collision").
  - `contactHertz = 30`, `contactDampingRatio = 10`, `contactSpeed = 3 m/s`, friction 0.6.
- **SAT cache** [16]: a cached separating axis gives an early out ("Cache hit, shapes are separated"). A cached touching axis within `linearSlop` gives "Cache hit, contact points generated" and the full SAT is skipped. Whether clipping still runs on that path: not reliably determined.
- **Graph colouring** [18]:
  - "Solver using graph coloring. Islands are only used for sleep."
  - Colours are assigned first-fit. Constraints that do not fit go to `B3_OVERFLOW_INDEX`.
  - `B3_DYNAMIC_COLOR_COUNT = COUNT - 4` reserves the last colours for constraints against static bodies, "to reduce tunneling".
  - Contacts enter the constraint graph only once they are touching.
- **Stages and SIMD** [17][31]:
  - Solver stages are split into blocks, "Target 4 blocks per worker", minimum 32 bodies / 4 contacts / 4 joints per block, claimed through an atomic sync index.
  - `B3_SIMD_WIDTH 4` (SSE2 or Neon). I saw no 8-wide path in `core.h`.
- **Narrowphase** [17]:
  - `b3ParallelFor(…, b3CollideTask, contactCount, minRange=20, …)`: "Task should take at least 40us".
  - Touching state changes are recorded in per-worker bitsets, OR-merged, then processed serially in bit order for determinism.
  - `b3Profile` has separate `collide`, `solve`, `sleepIslands`, `splitIslands` and other fields.
- **Box2D v3 numbers** [32][33]:
  - Large pyramid (5050 bodies, 14,950 contacts, 8 colours), 4 workers on a 7950X: AVX2 0.90 ms, SSE2 1.02 ms, scalar 1.91 ms.
  - Persistent islands on "Pyramids": DFS 0.69 ms vs persistent 0.01 ms. At most one island is split per step, and splitting runs in parallel with other work ("a few microseconds").

### Rapier (0.35, August 2026)
- **Solver** [22]: "Solve the single awake island. The staged solver is the only solver: a parallel build fans the colored constraints across `num_threads` workers". `min_island_size` was removed.
- **Sleeping** [20][34]:
  - Rewritten around persistent islands.
  - Defaults: `normalized_linear_threshold 0.05` (was 0.4), angular 0.5 rad/s, `time_until_sleep 0.5 s` (was 2.0).
- **Narrowphase** [21]: "Broadcast parallel-for … race a shared cursor for fixed blocks" of 64 pairs.
- **Contact clustering and recycling** (both on by default) [20]:
  - Clustering (3D): manifolds whose normals agree within about 5.1° (cos 0.996) are merged, then reduced to 4 points.
  - Recycling: a pair with pose drift below 0.05 (10× linear slop) skips contact determination.
- **Friction** [19][20]: `friction_model` Simplified is the default in 3D ("significantly faster … but less accurate"; 25% on large stacks). Coulomb gives one friction constraint per contact point.
- **SIMD** [34]: an optional `simd8` feature (8 lanes, AVX2).
- **Determinism** [34]: `enhanced-determinism` combined with `parallel` gives bitwise-identical results for any thread-pool size.

### Bepu Physics v2
- **Contact4 (convex)** [23]: 4 penetration rows, a 2D tangent friction at the manifold centre, and 1 twist row, i.e. 7 scalar rows.
- **SIMD batching** [35][36]:
  - Constraints are stored AOSOA in bundles of `Vector<float>.Count`.
  - A body appears at most once per constraint batch; this is asserted through `batchReferencedHandles`.
  - Constraints beyond `FallbackBatchThreshold` go to a fallback batch "allowing greater parallelism at the cost of convergence speed".
  - "The cost of the solver stage is linear with the number of iterations."

## Comparison table

| Aspect | Jolt 5.3 / 5.6 | PhysX 5 | Box3D | Rapier 0.35 | Bepu v2 | boyko (tree) |
|---|---|---|---|---|---|---|
| Max points per manifold | 4 | patches, ≤32 per pair | 4 | 4 after clustering | 4 | 4 |
| Friction rows, 4-point manifold | 12 / 7 | 4 normal + ≤4 friction | 7 (+ rolling for sphere/capsule) | Simplified (count not verified) | 7 | 12 (per point) |
| Skip narrowphase for barely-moved pairs | yes (1 mm / 2°) | PCM keeps contacts across frames | SAT cache | recycling (0.05) | not checked | no (hysteresis bias only) |
| Parallel narrowphase | jobs, batch 16 | yes (details not checked) | parallel-for, ≥20 contacts per task | blocks of 64 | not checked | sequential system |
| Large-pile solve | island splitter, 32 splits | not checked | colouring + overflow | colouring, one active set | batches + fallback batch | colouring |
| Sleep default | on (Pyramid forces off) | on (0.4 s) | on | on (0.5 s) | not checked | **off** |

## Key techniques
- **Temporal-coherence skip:** reuse the previous frame's manifold or separating axis when the relative pose changes less than a tolerance. Gregorius: "Contact performance is not about SAT vs GJK, but to not call any of those geometric algorithms at all if possible" [24].
- **Reduction to 4 points:** start from the deepest or a stable point, take the farthest point, then maximise area ("Maximize area, not distance") [24][13a].
- **Friction at the manifold centre plus twist** (Jolt 5.6, Box3D, Bepu): rows per manifold go from 3P to P+3, i.e. 12 → 7 when P = 4 (arithmetic from the cited structs).
- **Deterministic parallel narrowphase:** per-worker bitsets merged serially (Box3D), or hooks run serially after the parallel pass (Rapier).

## Pitfalls
- **Comparing sleep-on against sleep-off** measures different work. Both reference benches disable sleeping [1][14].
- **Box3D's recycling "may lead to ghost collision"** [30]. Jolt's cache replays manifolds, so correctness depends on the tolerance [2].
- **The pinned Jolt version changes the baseline:** 5.3 uses friction per point, 5.6 per manifold [6][7].
- **Contact caches limit scaling at high core counts** because of the lock-free cache operations [15].

## Relevant academic works
- Chen et al., "High-Performance Physical Simulations on Next-Generation Architecture with Many Cores", Intel Technology Journal, 2007. Basis of Jolt's island splitter [5].
- Moravanszky & Terdiman, "Fast Contact Reduction for Dynamics Simulation", Game Programming Gems 4, 2004, pp. 253-263. Not read [37].
- Catto, "Contact Manifolds", GDC 2007 [38]. Gregorius, "Robust Contact Creation for Physics Simulations", GDC 2015 [24].

## Applicability to boyko-engine
- **Facts in the tree:**
  - Sleeping already exists and ships OFF.
  - The narrowphase recomputes every pair on every step.
  - Friction is per point.
  - `MIN_PARALLEL_BODIES = 4096` (`resources.rs:664`), and the pyramid has 1240 bodies.
  - `MIN_SLOTS_PER_CHUNK = 64` (`D:\wt\joltab\crates\boyko_physics\src\solver\colored.rs:310`).
- **Bench differences from Jolt's scene:**
  - Friction 0.5 in the bench vs Jolt's default 0.2 (`jolt_parity_pyramid.rs:144,181`). Jolt also applies 0.05 damping.
  - The bench measures a steady state after 20 warm steps (`:247`). PerformanceTest runs 500 steps starting from the initial drop.
- **Why sleep does not transfer here:** in Box3D and Box2D v3, islands serve only for sleeping, and the solver's parallelism comes from colouring [18]. So sleeping and pile throughput are separate axes.

## Open questions for the architect
- The per-stage timing (one `Instant` per system at W=1 and W=8) is still missing. It decides whether the serial fraction is the narrowphase, the broadphase or colour quantisation.
- Does Jolt's body-pair cache actually hit on the settled pyramid? Plausible, but unmeasured. A Jolt profile would show FindCollisions' share of the step.
- Thread semantics: Jolt W = main thread + W−1 workers. For boyko W = `num_threads(workers)` workers; I did not check whether the calling thread also runs jobs.
- Re-pin the bench to Jolt v5.6 or later (the friction model changed)?

## Sources
[1] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/PerformanceTest/PyramidScene.h
[2] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/PhysicsSettings.h
[3] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/PhysicsSystem.cpp
[4] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/PhysicsSystem.h ; …/PhysicsUpdateContext.h
[5] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/LargeIslandSplitter.h
[6] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Docs/ReleaseNotes.md ; https://github.com/jrouwe/JoltPhysics/releases/tag/v5.6.0 ; https://gamefromscratch.com/jolt-physics-5-6-released/
[7] https://raw.githubusercontent.com/jrouwe/JoltPhysics/v5.3.0/Jolt/Physics/Constraints/ContactConstraintManager.h
[8] https://nvidia-omniverse.github.io/PhysX/physx/5.4.0/_api_build/struct_px_friction_type.html
[9] https://raw.githubusercontent.com/NVIDIA-Omniverse/PhysX/main/physx/CHANGELOG.md
[10] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/PerformanceTest/PerformanceTest.cpp
[11] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Body/BodyCreationSettings.h
[12] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Core/JobSystemThreadPool.h
[13] https://raw.githubusercontent.com/erincatto/box3d/main/src/contact_solver.h ; [13a] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Collision/ManifoldBetweenTwoFaces.cpp ; [13b] …/Constraints/ContactConstraintManager.cpp (master)
[14] https://raw.githubusercontent.com/erincatto/box2d/main/shared/benchmarks.c
[15] https://jrouwe.nl/jolt/JoltPhysicsMulticoreScaling.pdf
[16] https://raw.githubusercontent.com/erincatto/box3d/main/src/convex_manifold.c
[17] https://raw.githubusercontent.com/erincatto/box3d/main/src/physics_world.c
[18] https://raw.githubusercontent.com/erincatto/box3d/main/src/constraint_graph.c ; …/constraint_graph.h
[19] https://dimforge.com/blog/2026/01/09/the-year-2025-in-dimforge/
[20] https://raw.githubusercontent.com/dimforge/rapier/master/src/dynamics/integration_parameters.rs ; …/src/geometry/contact_clustering.rs
[21] https://raw.githubusercontent.com/dimforge/rapier/master/src/geometry/narrow_phase/contacts.rs
[22] https://raw.githubusercontent.com/dimforge/rapier/master/src/pipeline/physics_pipeline/solve.rs
[23] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/Constraints/Contact/ContactConvexTypes.cs ; [23b] https://raw.githubusercontent.com/erincatto/box3d/main/include/box3d/types.h
[24] http://media.steampowered.com/apps/valve/2015/DirkGregorius_Contacts.pdf
[25] https://nvidia-omniverse.github.io/PhysX/physx/5.6.1/docs/RigidBodyDynamics.html
[26] https://github.com/NVIDIAGameWorks/PhysX-3.4/blob/master/PhysX_3.4/Include/PxSceneDesc.h (search-result snippet)
[27] https://nvidia-omniverse.github.io/PhysX/physx/5.6.1/docs/AdvancedCollisionDetection.html
[28] https://raw.githubusercontent.com/erincatto/box2d/main/include/box2d/collision.h
[29] https://box2d.org/posts/2026/06/announcing-box3d/ ; https://github.com/erincatto/box3d
[30] https://raw.githubusercontent.com/erincatto/box3d/main/src/types.c ; …/include/box3d/types.h
[31] https://raw.githubusercontent.com/erincatto/box3d/main/src/solver.c ; …/src/core.h
[32] https://box2d.org/posts/2024/08/simd-matters/
[33] https://box2d.org/posts/2023/10/simulation-islands/
[34] https://raw.githubusercontent.com/dimforge/rapier/master/CHANGELOG.md ; …/src/dynamics/rigid_body_components.rs
[35] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/Solver.cs
[36] https://raw.githubusercontent.com/bepu/bepuphysics2/master/Documentation/PerformanceTips.md
[37] https://catdir.loc.gov/catdir/toc/ecip0410/2003023519.html
[38] https://box2d.org/files/ErinCatto_ContactManifolds_GDC2007.pdf