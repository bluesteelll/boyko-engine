# Research: Broadphase redesign for `boyko_physics` (lever outside the plan; tree `D:/wt/joltab`)

**How this was done.** I read the tree files listed below and wrote nothing. I have no shell, so I did not check the HEAD commit myself. The P0b numbers were taken at `dbd85977`. The Jolt source was read locally at `D:/tmp/jolt/JoltPhysics`, which is v5.3.0 (`Core.h:8-10`). "arith." means I worked it out from code or counters and did not measure it.

## Brief summary (TL;DR)

- **Every engine surveyed keeps statics in their own structure. boyko does not.**
  - Jolt uses a separate quadtree per BroadPhaseLayer (the Pyramid scene uses `NON_MOVING` and `MOVING`).
  - Box2D v3 has three trees, one per body type.
  - Bepu has an `ActiveTree` and a `StaticTree`; the static tree also holds sleeping bodies.
  - PhysX ABP keeps separate static, dynamic and kinematic box managers, each split into updated and sleeping.
  - Unity Physics has a static BVH that is rebuilt only when a static changes.
  - boyko puts everything in one set, and the slab's bounding sphere (radius √5001 ≈ 70.7 m) reaches every box. So **all 1,240 boxes are paired with the slab**: 1,240 of the 9,561 pairs, of which at most about 225 (the bottom layer) touch it (arith.).
  - The same radius stretches the Grid's world box to about 141 m per side, which makes the cells about 13 m wide against 3.46 m box bounds (arith., from `resources.rs:1045-1058,1109-1112`). That is a candidate cause of the measured 2.5× Grid loss.
- **No reference engine revisits pairs where neither body moved.**
  - Jolt's `FindCollidingPairs` is called only on the active-body list.
  - Box2D queries only moved proxies.
  - Rapier walks only changed leaves.
  - PhysX ABP: "if none of the involved objects have been updated, the pair is just sleeping: keep it and skip it".
  - Result: Jolt's whole step with everything asleep takes 2.2 µs, against boyko's 1.93 ms broadphase alone.
- **Engines make parallel pair finding deterministic in four ways:**
  - sort afterwards: Jolt sorts contacts per island by a hash key; Box2D main sorts all pair keys with QuickSort;
  - merge per-query lists in a fixed order (Box2D v3.1);
  - collect in order (Rapier, using rayon);
  - leave the order nondeterministic unless a flag is set (Bepu `Deterministic`).
  - boyko's contract is pairs sorted by `(min, max)` dense row. That is the same thing as Box2D main's sorted 64-bit keys.
- **In boyko the pair COUNT affects simulation values.** The box-box axis cache is sized `next_pow2(2·pairs.len())` and is cleared whenever it grows or passes 50 % load (`axis_cache.rs:261-281,383`). So a tighter but still conservative predicate (AABB instead of sphere) keeps the same manifold set but can change the hints, and therefore the pose bytes. Only a broadphase that emits exactly the AllPairs pair set is bit-identical to today.
- **Measured anchors:**
  - AllPairs: 2.53–2.73 ns per test in the scene (769,420 tests).
  - Jolt's incremental quadtree rebuild on the same pyramid: 66 µs of thread time per step.
  - Terdiman's box pruning: 10k random boxes, 11,715 overlaps, 2,413 K-cycles single-threaded with AVX (about 0.67 ms at 3.6 GHz, arith.).
  - boyko Grid at 10k: 8.0 ms (uniform) and 15.8 ms (size disparity).

## Approaches in state-of-the-art engines

### Jolt v5.3.0 (source read locally)
- **Structure.**
  - One `QuadTree` per BroadPhaseLayer (`BroadPhaseQuadTree.cpp:46-49`).
  - A node is 128 B and holds its 4 children's bounds as SoA `atomic<float>[4]` arrays, tested 4-wide with `AABox4VsBox` (`QuadTree.h:97-139,152`; `QuadTree.cpp:1453-1461`).
  - The docs recommend at least one static layer and one dynamic layer (`Docs/Architecture.md:463`). PerformanceTest puts the floor in `NON_MOVING`, and `NON_MOVING` collides only with `MOVING` (`PerformanceTest/Layers.h:22-28,87-93`).
- **Incremental update.**
  - When a body moves, `NotifyBodiesAABBChanged` widens the leaf and its ancestors in place, lock-free, and marks the tree dirty (`QuadTree.cpp:943-973`).
  - `UpdatePrepare` rebuilds only subtrees marked changed; unchanged nodes are reinserted whole, and the top 5 levels are always rebuilt (`QuadTree.cpp:322-327,368-376`).
  - It runs as a job alongside the step, one dirty layer per step round-robin (`BroadPhaseQuadTree.cpp:112-124`; `PhysicsSystem.cpp:249-256`). The old tree stays valid until `UpdateFinalize` swaps them (`QuadTree.cpp:402-416`).
  - P0b measured `UpdateBroadPhasePrepare` at **0.066–0.067 ms of thread time per step** on this pyramid (`p0b/window_report.md:307,320`).
- **Pair finding.**
  - Each `FindCollisions` job takes batches of 16 active bodies with a CAS on `mActiveBodyReadIdx` (`PhysicsSystem.cpp:856-907`; `PhysicsSystem.h:301`).
  - For each body it walks every layer tree that the filter allows (`BroadPhaseQuadTree.cpp:559-596`).
  - The query box is the body's AABB expanded by `mSpeculativeContactDistance` (0.02). Each hit is re-checked against the exact current bounds, because tree nodes may have been widened (`QuadTree.cpp:1419-1420,1437-1439`).
  - Pairs go to per-job ring queues. A full queue is processed on the spot, and idle jobs steal narrowphase work from other queues. Broadphase and narrowphase are interleaved with no barrier between them (`PhysicsSystem.cpp:877-893,916-949`; `Architecture.md:787-796`).
- **Deduplication and sleeping.**
  - `sFindCollidingPairsCanCollide` accepts a pair only if `A.activeIndex < B.activeIndex`. An inactive or static body has index 0xFFFFFFFF, so active-vs-sleeping and active-vs-static pairs are found exactly once, and sleeping-vs-sleeping pairs are never visited (`Body.inl:30-79`).
- **Determinism.** Pairs are discovered in a nondeterministic order. When `mDeterministicSimulation` is on (the default), each island's constraints and contacts are sorted by `mSortKey`, then by body IDs (`PhysicsSystem.cpp:1414-1422`; `ContactConstraintManager.cpp:1438-1454`; `PhysicsSettings.h:99`).
- **Trade-off.** Every overlapping pair is re-found each step; the cost is absorbed by the body-pair manifold cache. The tree quality depends on batched adds or `OptimizeBroadPhase`: adding one body at a time can build a bad tree and "in the worst case can lead to missed collisions" (`Architecture.md:44`).

### Box2D v3 (fetched from GitHub; v3.1.0 tag and `main` as of 2026-09-19)
- **Trees.** Three dynamic AABB trees, indexed by body type (static, kinematic, dynamic) (`main` `broad_phase.c`). Insertion uses a greedy SAH descent plus rotations; the rebuild uses binned SAH with 8 bins or a median split, and touches only nodes marked moved (`dynamic_tree.c`, summarised by the fetch tool).
- **Enlarged AABBs (v3.1 `solver.c`).**
  - A shape's AABB is its tight AABB plus `B2_SPECULATIVE_DISTANCE`, which is 4×`B2_LINEAR_SLOP` = 0.02 m.
  - Only when the fat AABB no longer contains it is a new fat AABB made, with `B2_AABB_MARGIN` = 0.05 m (`constants.h`).
  - Enlarged bodies are recorded in per-worker bitsets, OR-merged, and then `b2BroadPhase_EnlargeProxy` runs serially. The source comment: "Update the move array here for determinism because bullets are processed below in non-deterministic order."
- **v3.1 pair finding.**
  - A move buffer (`moveSet` plus `moveArray`). `b2FindPairsTask` runs in parallel with a minimum range of 64, and each query proxy gets its own `moveResults[i]` linked list.
  - Deduplication: when both proxies are moving, only the lower key reports the pair. Any pair already in `pairSet` is skipped: "contact exists".
  - Contacts are then created serially "in deterministic order", walking `moveResults` in move-array order.
- **`main` pair finding.**
  - Moved flags propagate to the root. `b2GatherMovedSiblings` lists sibling pairs where either sibling moved. Their subtrees are collided (`b2SelfPairsTask`, grain 64), citing "Real-time collision detection section 6.3.2". A node pair where neither side moved is pruned in `b2TestPair`.
  - The dynamic tree is crossed with the static and kinematic trees from 64 breadth-first seeds (`b2CrossPairsTask`).
  - Candidates go into per-worker `pairKeys`, are filtered against `pairSet`, concatenated, and QuickSorted. The comment: "Pairs arrive in deterministic order but scrambled… sorting them here improves solver performance."
  - Static proxies are never marked moved unless contact creation is forced. The dynamic and kinematic trees are rebuilt in `b2UpdateTreesTask`, in parallel with the narrowphase.
- **Persistent pairs and sleeping.**
  - A contact exists from fat-AABB overlap until `b2CollideTask` finds the fat AABBs disjoint and sets `b2_simDisjoint`, after which the contact is destroyed serially (v3.1 `world.c`).
  - Only the awake set's contacts are collided. When an island sleeps, its touching contacts move into the sleeping set and its non-touching ones into the disabled set (`solver_set.c`).
  - Erin Catto's determinism post names cross-thread ordering as "a hidden random number generator" and describes merging per-worker bit arrays with OR ([determinism post](https://box2d.org/posts/2024/08/determinism/)).

### PhysX 5 (documentation and `BpBroadPhaseABP.cpp`)
- **Docs.**
  - eSAP is "great… when many objects are sleeping" but degrades "when all objects are moving".
  - eABP gives "the best performance on average… a good default choice".
  - ePABP "is only faster for large scenes… also uses more memory".
  - The GPU broadphase is an incremental sweep-and-prune ([5.4 docs](https://nvidia-omniverse.github.io/PhysX/physx/5.4.0/docs/RigidBodyCollision.html)).
- **ABP source.**
  - Static, dynamic and kinematic `BoxManager`s, each split into `mUpdatedBoxes` and `mSleepingBoxes`.
  - Each frame the X axis is radix-sorted; the sort is not incremental.
  - Queries: a complete box pruning of updated dynamics against each other; bipartite passes for updated vs sleeping and for dynamic vs static.
  - Pairs persist in a hash pair manager with isNew/isUpdated flags. A pair where neither object was updated is kept and skipped.
- **Terdiman's box pruning series** (Pierre Terdiman wrote the ABP code). 10k random boxes, 11,715 overlaps: 66,245 K-cycles at baseline, 2,413 K-cycles for the AVX version on an i7-6850K ([part 15](http://www.codercorner.com/blog/?p=1908), 2018-05-03). His own warning: a vertical stack of equal boxes drives the sweep axis's "pruning power… to zero" ([part 17](http://www.codercorner.com/blog/?p=1978)).

### Bepu v2 (source)
- `ActiveTree` holds "wakeful bodies"; `StaticTree` holds "sleeping bodies and statics" (`BroadPhase.cs`).
- Pairs come from an `ActiveTree` self test plus an `ActiveTree` × `StaticTree` test. Workers take jobs through an `Interlocked` counter, and per-worker handlers call `NarrowPhase.HandleOverlap` directly, so no pair list is ever built (`CollidableOverlapFinder.cs`).
- The order is nondeterministic unless `Simulation.Deterministic` is set, which has "a slight performance impact" (`QuestionsAndAnswers.md`).
- A comment in Bepu's own source calls its static-tree refinement "enormously inefficient", because "static and inactive objects do not move every frame".

### Rapier (master, `broad_phase_bvh/update.rs`; docs.rs 0.35.3)
- A single dynamic BVH now serves both queries and the broadphase, replacing the hierarchical SAP ([2025 review](https://dimforge.com/blog/2026/01/09/the-year-2025-in-dimforge/)).
- Only modified colliders are updated. A change-detection margin is used; the adaptive variant is 12.5 % of the smallest AABB extent.
- It chooses between a partial refit (ancestors of changed leaves) and a full refit when `(updated + prev)·16 ≥ leaf_count`. `optimize_incremental` is deferred so it overlaps the narrowphase and solver.
- New pairs come from `traverse_bvtt_single_tree_parallel` gated by change flags. Pairs persist in a map, and removals are checked only for pairs next to a changed collider.
- Determinism: "rayon's ordered collect keeps the new pairs in traversal order", and stale pairs are sorted.

### Unity Physics (DOTS)
- Two BVHs, one static and one dynamic. The static one is updated only when a static changes.
- The dynamic one is rebuilt from scratch by default; incremental update is optional.
- The simulation is stateless ([docs 1.4](https://docs.unity3d.com/Packages/com.unity.physics@1.4/manual/concepts-simulation.html)).

## Comparative table

| Aspect | Jolt 5.3 | Box2D v3 (3.1 / main) | PhysX ABP | Bepu v2 | Rapier | boyko today |
|---|---|---|---|---|---|---|
| Statics | own layer tree | own tree, never "moved" | own manager | StaticTree | same BVH, flags | mixed into AllPairs |
| Structure | 4-wide quadtree, widen then background rebuild | binary SAH tree, fat AABB 0.05 m | per-frame X radix sort plus box pruning | binary tree, refit and refine | BVH, partial or full refit | O(n²) loop; opt-in uniform grid |
| Queried bodies | active list only | moved proxies only | updated only | active tree only | changed leaves | all rows, every step |
| Pair set | full re-find each step, manifold cache | persistent, new pairs only | persistent hash | direct to narrowphase | persistent map | full recompute each step |
| Parallel | CAS batches of 16, queue per job | per proxy (3.1), per worker plus sort (main) | PABP variant | job counter | rayon | none (Grid emit only at n ≥ 4096) |
| Determinism | sort per island | ordered merge / global sort | not read | optional flag | ordered collect | `(min, max)` sort |
| Sleepers | skipped | skipped | "just sleeping: keep it" | moved to StaticTree | not "changed" | visited |

## Key algorithms and techniques

- **Deduplication rules.**
  - Jolt: A's active index < B's, with inactive bodies at ∞.
  - Box2D 3.1: lower key when both proxies are moving.
  - Box2D main: every leaf pair has a single lowest common ancestor, so the sibling-subtree traversal emits each pair exactly once without a hash (Ericson, section 6.3.2).
  - boyko: `i < j` on dense rows.
- **Deterministic parallel emission patterns seen:**
  - count, prefix sum, then emit into disjoint ranges (already in the tree: the Grid's O3 Pass A/B, `resources.rs:1787+`);
  - per-chunk outputs concatenated in chunk order (Rapier; also the unification design's S2 row);
  - per-worker buffers followed by a sort (Box2D main);
  - fixed-order merge of per-query lists (Box2D 3.1);
  - OR-merged bitsets (Box2D).
- **Margins.** Jolt: speculative 0.02 m plus in-place node widening. Box2D: fat AABBs with a 0.05 m margin plus 0.02 m speculative. Rapier: adaptive skin.
- **Split between build and query.** Jolt refits in a background job and queries against the last finished tree. Box2D and Rapier overlap the tree rebuild with the narrowphase.

## Pitfalls and mistakes

- **Large statics under a sphere bound.**
  - The slab alone produces 1,240 candidates and inflates the Grid geometry (see TL;DR).
  - Even without the slab, the measured uniform crossover is n* ≈ 1,109 (Grid/AllPairs = 1.107 at 1k). The Grid would therefore be close to break-even at n = 1,240.
- **Dense piles defeat one-axis pruning.** This is Terdiman's stack case. For the pyramid along x, about 300 boxes overlap each box's x-interval, giving about 186k 1D candidates against 769k for AllPairs (arith.).
- **A cull must be conservative with respect to the f32 sphere predicate.** The float result of `p ± r` can reject a pair that the rounded `length_squared() <= bound²` accepts. The Grid avoids this by re-applying the exact predicate (`feasible`, `resources.rs:1486-1489`).
- **Pair count changes values.** `begin_frame(pairs.len())` sizes the table and clears it on growth (`axis_cache.rs:261-281,383`).
- **Rows are not stable.** A despawn swap-removes; a spawn or migration shifts rows (`row_identity.rs:3-9`). Persistent leaves or pairs keyed by row need `RowIdentity` remapping until U5's stable slots.
- **Sleep depends on seeing frozen contacts.** Wake-on-contact-change compares each island's manifold count every step (`resources.rs:3524-3555`). Skipping frozen pairs therefore needs those manifolds, or their counts, to be carried forward; Box2D keeps them in the sleeping set.
  - The latch visible at broadphase time is the one from step t−1's `end_step` (`resources.rs:3660-3667`). `begin_step` can unlatch it later in step t.
- **Cross-thread push order is nondeterministic.** Box2D's post calls it a hidden random number generator.
- **The existing gate is thin.** `production_grid_equals_all_pairs` is one step over 24 clustered spheres plus one isolated sphere, all static: no boxes and no size disparity (`tests/broadphase_grid.rs:500-543`).

## Relevant academic works
- Ericson, *Real-Time Collision Detection*, 2005, section 6.3.2 (simultaneous tree traversal). Cited in Box2D's code; I did not read it.
- Tracy, Buss, Woods, "Efficient Large-Scale Sweep and Prune Methods with AABB Insertion and Removal", IEEE VR 2009. The fetch failed (TLS certificate); not read.
- Serpa and Rodrigues, "Broadmark" and the KD-tree broadphase paper, Computer Graphics Forum 2019. The results were not retrievable (403).
- Catto, "Dynamic Bounding Volume Hierarchies", GDC 2019, and Rouwe, "Architecting Jolt Physics", GDC 2022 notes. I found both PDFs but could not render them (no poppler); not read.

## Tree facts this lever touches (file:line in `D:/wt/joltab/crates/boyko_physics`)

**Broadphase system and its output**
- `physics_broadphase`: `src/systems.rs:297-349`.
  - The AllPairs arm is `:309-327` and is marked "kept VERBATIM… DO NOT refactor" (`:307-308`).
  - The Grid arm is `:335-341`.
  - A debug assert checks the sorted order (`:344-347`), and a pair counter follows (`:348`).
- `body_bounding_radius` computes `half_extents.length()` (a sqrt) for boxes, called twice per test: `src/systems.rs:1218-1223`.
- `ContactPairs`: `src/resources.rs:561-611`.
  - Its only consumer is `physics_narrowphase` (`src/systems.rs:375-478`), which iterates in pair order (`:400`).
  - It also feeds the axis cache prefetch (`:391-392`).
  - `set` is called only when a contact exists (`:437-446`).
  - `box_box_contact` returns `None` when separated (`narrowphase/box_box.rs:294-297`).

**Config**
- `BroadphaseKind` and `BroadphaseSelectMode`: `src/resources.rs:45-53,68-78`.
- `PhysicsConfig` fields: `broadphase` (`:154`), `broadphase_select` (`:165`) and `parallel_broadphase` (`:249`, a no-op on AllPairs).

**BroadphaseGrid**
- 13 `ScratchColumn`s: `src/resources.rs:716-835`.
- `recompute_geometry`: `:1032-1181`.
- `build`: candidates, then the feasibility filter, then `sort_unstable` (`:1190-1230`).
- `build_csr`: `:1243-1354`.
- Oversized-body handling: `MAX_CELL_SPAN` = 8 (`:619`), `COARSE_CELL_FACTOR` (`:635`).
- `build_parallel`: `MIN_PARALLEL_BODIES` = 4096 (`:675,1745`), `CHUNKS_PER_WORKER` = 4 (`:667`), `try_with_active_pool` (`:1761`).

**Policy and wiring**
- Policy: `GRID_LO` = 96 (`src/broadphase_policy.rs:79`), `GRID_HI` = 192 (`:88`), `select_broadphase` (`:173-193`).
- Wiring (`src/plugin.rs`):
  - resources inserted at `:520` (ContactPairs), `:526` (BroadphaseGrid), `:534` (PhysicsStats);
  - the soft coupling forces the Grid (`:513-517`);
  - schedule edges `:628-633`.
- The soft coupling reads the Grid's cell structure: `src/soft/coupling.rs:310-349` and `src/soft/solver.rs:153`.

**Storage and data**
- Storage cohort: `BROADPHASE_COLUMN_COUNT` = 13, with cohort width ≤ `POOL_STAGGER_LINES` = 64 (`src/scratch_ids.rs:365-396`); `broadphase_column_id` (`:743-763`); `scratch_reserve_rows` (`:98-107`).
- A `ScratchColumn` reserve is a hard ceiling (`src/resources.rs:3995-3998`). Persistent `ScratchColumn` content is already used by the axis cache.
- `BodyState` is AoS (156 B stride per the unification design) and carries no AABB, layer or mask: `src/resources.rs:3679-3735`.
- `Collider.layer` and `mask` exist but no broadphase uses them (`src/components.rs:153-168`). The bench sets `layer: 1, mask: 1` (`benches/jolt_parity_pyramid.rs:647`).
- A static row has `inv_mass == 0` (`is_dynamic_row`, `src/solver/contact.rs:38-40`). Kinematic rows also have zero `inv_mass`, and AllPairs emits static–static pairs.

**Sleep and graph**
- `IslandSleep`: the `asleep` flags and several other fields are a side `Vec` (`src/resources.rs:3108-3159`); `begin_step` is `:3480-3568`.
- The frozen skip happens only in the solve (`src/solver/colored.rs:1863-1871`).
- The graph is a pure function of manifold order (`src/systems.rs:1038-1057`; `src/resources.rs:2655-2665`).

**Tests, benches and the unification design**
- Tests: `tests/broadphase_grid.rs:95` (proptest) and `:535`; `tests/broadphase_sizeclass_p8.rs:111-205`; `benches/broadphase.rs:205` (O3 Gate 7).
- Unification design (main checkout, dated 2026-09-10): `D:/claude/BoykoEngine/docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md`.
  - D1, stable slots: `:102-120`.
  - D3, a 16 B `BodyPose` read by the broadphase: `:141-147,337-348`.
  - S2 AllPairs: per-chunk outputs concatenated in order (`:401`).
  - `sqrt` count: 2 × 769,420 → 1,241 (`:93`).
  - Commit `5aef7b39` records the owner's answer "refactor last".

## Expected cost (measured where marked; everything else is arith.)

| Approach | n = 1,240 pyramid (all awake) | n = 10k | Basis |
|---|---|---|---|
| AllPairs today, serial | **2.10 ms (W=1), 1.95 ms (W=8), measured** | **70.2 ms, measured** (bench, uniform); 126–137 ms at the in-scene cost per test | ANALYSIS §2; window_report:339-341 |
| Same kernel, parallel | ≈ 0.39 ms at W=8 (= 2.10 ms / (8 × 0.681)) | ≈ 12.9 ms at W=8 | L5 formula, E(8) = 0.681 |
| Grid today | **5.17 ms, measured** | **8.0 ms uniform / 15.8 ms disparity, measured**; 4.33 ms parallel at w4 | window_report:45,344-353 |
| Box pruning with full re-sort | about 186k 1D candidates on the pile (degenerate case) | about 0.67–1.1 ms single-threaded on a random scene (Terdiman) | codercorner |
| Incremental tree (Jolt style) | rebuild **66 µs, measured (Jolt)**. The query share of Jolt's FindCollisions (0.67–0.90 ms at W=8, including the narrowphase) is **not split out** | no source found | window_report:307,320 |
| Sleepers skipped | cost 0 for the sleeping part; **Jolt's whole all-asleep step is 2.2 µs, measured** | proportional to the active count | ANALYSIS §6 |

The floor for writing out 9,561 pairs is 76 KB of output (arith.).

## Applicability to boyko-engine (facts only, no design)
- Directly reusable in the tree:
  - the count, prefix sum, emit pattern and its `pool.scope` dispatch (`resources.rs:1787+`). P0b measured parallel scaling from a system body: P(1) 12.2 ms → 2.24 ms at W=8;
  - the `feasible` re-check;
  - persistent `ScratchColumn`s;
  - the proptest harnesses.
- Constraints that bind a new design:
  - rows are not stable;
  - the pair count affects values;
  - the Grid's cell structure is required by the SP2 soft coupling;
  - sleep relies on per-step manifold counts.

## Open questions for the architect
1. Keep the sphere predicate, which keeps the pair set identical to AllPairs? Or switch to an AABB predicate, which drops about 1,015 non-touching slab pairs (arith.) but changes values through the axis cache?
2. Re-find all pairs every step (Jolt, boyko) or keep a persistent pair set (Box2D, Rapier, PhysX)? A persistent set needs pair identity that survives row changes before U5.
3. Which rows count as "static", and how is a static that moves detected? Kinematic rows also have `inv_mass == 0`.
4. Should the broadphase use `Collider.layer`/`mask`? Filtering would change the pair set compared with AllPairs.
5. How is the frozen-pair skip reconciled with the manifold-count wake key?
6. Jolt's broadphase-only time on this pyramid could be taken from the `-p` HTML dumps (per-layer `JPH_PROFILE`, `BroadPhaseQuadTree.cpp:588`). Those dumps are not under `p0b/raw`.

## Sources
- Jolt v5.3.0 source (local `D:/tmp/jolt/JoltPhysics`): `PhysicsSystem.cpp`, `BroadPhaseQuadTree.cpp`, `QuadTree.{h,cpp}`, `Body.inl`, `Docs/Architecture.md`, `PerformanceTest/{PyramidScene.h,Layers.h}`.
- Box2D v3.1 source: [broad_phase.c](https://raw.githubusercontent.com/erincatto/box2d/v3.1.0/src/broad_phase.c), [solver.c](https://raw.githubusercontent.com/erincatto/box2d/v3.1.0/src/solver.c), [constants.h](https://raw.githubusercontent.com/erincatto/box2d/v3.1.0/src/constants.h), [world.c](https://raw.githubusercontent.com/erincatto/box2d/v3.1.0/src/world.c), [solver_set.c](https://raw.githubusercontent.com/erincatto/box2d/v3.1.0/src/solver_set.c).
- Box2D `main` source: [broad_phase.c](https://raw.githubusercontent.com/erincatto/box2d/main/src/broad_phase.c), [dynamic_tree.c](https://raw.githubusercontent.com/erincatto/box2d/main/src/dynamic_tree.c).
- [Box2D determinism post](https://box2d.org/posts/2024/08/determinism/).
- [PhysX 5.4 rigid-body collision docs](https://nvidia-omniverse.github.io/PhysX/physx/5.4.0/docs/RigidBodyCollision.html); [BpBroadPhaseABP.cpp](https://raw.githubusercontent.com/NVIDIA-Omniverse/PhysX/main/physx/source/lowlevelaabb/src/BpBroadPhaseABP.cpp).
- [Codercorner part 15](http://www.codercorner.com/blog/?p=1908), [part 17](http://www.codercorner.com/blog/?p=1978).
- Bepu: [BroadPhase.cs](https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/CollisionDetection/BroadPhase.cs), [CollidableOverlapFinder.cs](https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/CollisionDetection/CollidableOverlapFinder.cs), [Q&A](https://raw.githubusercontent.com/bepu/bepuphysics2/master/Documentation/QuestionsAndAnswers.md).
- Rapier: [update.rs](https://raw.githubusercontent.com/dimforge/rapier/master/src/geometry/broad_phase_bvh/update.rs), [BroadPhaseBvh docs](https://docs.rs/rapier3d/latest/rapier3d/geometry/struct.BroadPhaseBvh.html), [Dimforge 2025 review](https://dimforge.com/blog/2026/01/09/the-year-2025-in-dimforge/).
- [Unity Physics simulation pipeline](https://docs.unity3d.com/Packages/com.unity.physics@1.4/manual/concepts-simulation.html).
- [Broadmark](https://ppgia-unifor.github.io/Broadmark/); [Catto GDC 2019 slides](https://box2d.org/files/ErinCatto_DynamicBVH_GDC2019.pdf) (located, not read).
- In-tree measurement inputs:
  - `D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md`
  - `D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/p0b/window_report.md`
  - `D:/wt/joltab/docs/physics/perf-campaign/00-RULINGS.md`
  - `D:/wt/joltab/docs/physics/perf-campaign/03-TREE-REPORT.md`