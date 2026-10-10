# Research: L5 parallel narrowphase, with L4 (`parallel_solve` default) and L2 (Grid thresholds)

Tree read: `D:/wt/joltab` at e2bcbcb5. Nothing was built, run or written. graphify is not installed here, so I used Grep/Read. Line numbers below are for this tree; they have moved since the 03-TREE-REPORT (for example, the narrowphase was at `systems.rs:369-462` there and is at `:375-478` now).

## Brief summary (TL;DR)
- **Every reference engine makes its parallel narrowphase deterministic in one of three ways:**
  - **Per-pair records updated in place**, so no merge is needed. Box2D v3, Box3D, Rapier and Avian do this, and they record state changes in per-worker bitsets that are merged with OR and then walked in index order.
  - **Per-worker buffers sorted afterwards by a pair key.** Jolt sorts contacts by a hash key, Bepu sorts by `CollidablePair`, and Avian sorts by the original pair index.
  - **Per-work-item streams read back in index order.** This is the shape of Unity Physics' `NativeStream`, documented as deterministic; I did not verify the per-index semantics.
  - boyko has no persistent per-pair record, because manifolds are rebuilt every step. That leaves shapes 2 and 3, or per-pair slots plus a compaction pass. The repository's own unification design already specifies that last one (§10.4).
- **Each pair's work in `physics_narrowphase` is already a pure function** of `(bodies[a], bodies[b], hint)`. Only three things are shared and mutable: the `out` push, the `sensor_out` push and `axis_cache.set`.
  - Reads of the axis cache (`probe`) are pure. If no `set` runs during the parallel phase, reading the live table is the same as reading a pre-step snapshot. So the steady-state snapshot costs no extra pass. On frames where rows moved, the cold pre-read already runs serially today.
- **Two exactness details that O3 did not state:**
  - Linear-probing slot positions depend on insertion order. Lookups do not, but the table bytes do, so a byte-equal table gate needs the commit to be serial and in pair order.
  - The `set` predicate is "`box_box_contact` returned `Some`", including sensor pairs. That is not the push predicate, which is "`Some` and `count > 0`, not a sensor".
- **P0 numbers for L5:** t_np(1) = 3.142 ms and t_np(8) = 2.946 ms. The predicted gain is 2.57 ms (28 % of T(8)). Jolt's FindCollisions at W=8 is 0.67–0.90 ms (parallel, with a pair-cache replay).
- **The in-tree parallel precedents cost one `pool.scope` each:** the Grid emit (size to an upper bound, write disjoint sub-ranges through `solve_base`, then truncate) and the solver's colour dispatch. One scope is one boxed `ScopeShared` plus a 4 KiB block chunk.
  - The allocation census pins exact per-step scope counts: S1c is 133 scopes / 229 chunks, and S1b is 1 / 1.
- **L4 and L2 are one-line constant changes, each with a known blast radius.**
  - L4: the solver has no `lanes < 2` shortcut, so at W=1 wide colours still go through `pool.scope`. J-P1 measured +0.60 %, which is not claimed (SE bar 0.84 %).
  - L2: only `Auto` mode reads `GRID_LO`/`GRID_HI`, and `Manual` is the default, so parity is not affected. `broadphase_select_p3.rs` sizes its scenes from these constants.
  - An in-repo decision predates P0 and makes Auto the default at rung P1. P0 contradicts it.

## Approaches in state-of-the-art engines

### Jolt Physics (master; P0 compared against v5.3.0 and v5.6.0)
- **Approach:** job-based `FindCollisions`. Each job claims 16 active bodies at a time with `mActiveBodyReadIdx.fetch_add(cActiveBodiesBatchSize)`, queries the broadphase for them, and pushes the pairs into its own ring queue `mBodyPairQueues[mJobIndex]`. An idle job steals from any queue with a CAS on `mReadIdx`. More jobs are spawned when at least `cNarrowPhaseBatchSize` (16) pairs are queued. If a queue is full, the pair is processed on the spot [1][3].
- **Job count:** `max(mc==1 ? 1 : 2, min(ceil(active/16), mc))` [1].
- **Pair cache:** used when `mUseBodyPairContactCache` (default true) is on and neither body's cache is invalid. A hit needs the relative pose to have moved by at most `mBodyPairCacheMaxDeltaPositionSq = Square(0.001f)` (1 mm) and `mBodyPairCacheCosMaxDeltaRotationDiv2 = 0.99984769…` (about 2°). A hit copies the whole `CachedBodyPair` with `memcpy` and skips collision detection [2][4].
- **Output:** `AddContactConstraint` takes its index with a `fetch_add` on `mNumConstraintsAndNextConstraintOffset`, so the array order depends on thread timing [2].
  - The manifold cache is double-buffered: `ManifoldCache mCache[2]`, "one cache to read from and one to write to", swapped by `mCacheWriteIdx ^= 1` [2][5].
  - Each job's `ContactAllocator` derives from `LFHMAllocatorContext`, and `ManifoldCache::cAllocatorBlockSize` is 4096 [5].
- **Determinism:** `SortContacts` sorts by `mSortKey`, a hash of `SubShapeIDPair{body IDs, sub-shape IDs}`, with the body IDs as tie-breakers. It runs inside `JobSolveVelocityConstraints`, commented "Sort contacts to give a deterministic simulation" [1][2]. The architecture doc says the thread count does not affect determinism, given the same API call order and the same binary [6].
- **Trade-offs:** no transient order to preserve, but every island pays a sort, and determinism rests on a hash key. The earlier survey (04 [3][4]) reports that beyond about 16 cores the lock-free contact-cache operations dominate.

### Box2D v3 (main) and Box3D
- **Approach:** contacts are persistent per-pair records (`b2ContactSim`) stored in the constraint graph's colour arrays (touching) plus the awake set (not touching).
  - `b2Collide` builds `collideSpans` over those arrays, then calls `b2ParallelFor(world, &b2CollideTask, contactCount, minRange=64, …)`. The comment reads: "Task should take at least 40us on a 4GHz CPU (10K cycles)" [7].
  - Box3D gathers indices from all colours into one array and uses `minRange = 20` [8].
- **Dispatch:** `b2ParallelFor` targets `blocksPerWorker = 8`, and workers claim blocks through an atomic cursor, `b2AtomicFetchAddInt(&shared->nextBlock, 1)` [9].
- **Per-pair write:** `b2UpdateContact` writes only its own `contactSim`. The per-pair collision cache (`&contactSim->cache`) lives inside the record. Warm-start impulses are carried by matching point `id` against the old manifold [10].
  - Box3D keeps `b3ContactCache` (`satCache`/`simplexCache`) inside the contact as well [11]. This is the counterpart of boyko's `BoxAxisCache`, but it is stored per contact rather than in a shared hash table.
- **State changes:** `b2SetBit(&taskContext->contactStateBitSet, contactId)` for disjoint, started-touching and stopped-touching. The main thread merges with `b2InPlaceUnion` and walks the set bits in ascending order with `ctz` [7].
  - The author explains why: "Each thread can create new contacts and add them to the world array … depends on how fast each thread does its work", and the solver is order-dependent [12].
- **Sleeping:** only the graph colours and the awake set are iterated. Sleeping islands' contacts sit in their own solver sets [7].
- **Contact recycling:** new on main. It skips `b2UpdateContact` below relative-motion thresholds. This changes values, so it belongs to L9, not L5 [7].

### Rapier (master)
- **Approach:** `rayon::broadcast` plus `AtomicUsize` cursor claiming, `const BLOCK: usize = 64`, over the contact-graph edges in `update_candidates`. Each edge is updated in place through a raw pointer [13].
- **Determinism:** hooks and events run serially after the parallel pass. "The reduce order is scheduler-dependent: sort so the fully-updated edge list is deterministic". Transitions are applied "in sorted edge-id order" [13].

### Bepu Physics v2
- **Approach:** a per-worker `OverlapWorker { CollisionBatcher, PendingConstraintAddCache, PendingSetAwakenings }`. The source says: "All of the pair storage is thread local and requires no synchronization." The previous frame's pair-to-constraint mapping is treated as read-only during the narrowphase [14].
- **Determinism:** when `Simulation.Deterministic` is set, pending constraint adds are sorted by `CollidablePair` before `DeterministicAdd`. The source notes: "Collidable pair comes first; deterministic flushes rely the memory layout to sort". Otherwise it uses `FlushSequentially` in generation order [15].

### Avian (Bevy ECS-native physics)
- **Approach:** `par_for_each(contact_graph.active_pairs_mut(), 64, …)` with `ThreadLocal<RefCell<NarrowPhaseThreadContext>>`, where each context holds `contact_status_bits: BitVec` and `recycled_count`. The contexts are merged serially with OR [16].
- **Determinism:** since 0.3, contact constraints are pushed into per-thread buffers, drained, then sorted "using the original pair indices to retain determinism". The 0.3 post reports the narrowphase "4.5x as fast" [17].

### Unity Physics (DOTS)
- It is stateless ("does not cache anything frame-to-frame") [18], which is the closest model to boyko's per-step rebuild.
- `NativeStream` is documented as "a deterministic data streaming supporting parallel reading and parallel writing" and is constructed with a `foreachCount` [19]. I did not verify the per-index block semantics or the narrowphase job structure.

### PhysX
- I found no reliable information on the CPU narrowphase job structure. `eENABLE_ENHANCED_DETERMINISM` exists, and it keeps the simulation unchanged when non-interfering actors are added [20].

## Comparative table

| Aspect | Jolt | Box2D v3 / Box3D | Rapier | Bepu | Avian | boyko today |
|---|---|---|---|---|---|---|
| Output storage | Atomic append + double-buffered lock-free cache | Persistent per-pair record, updated in place | Graph edge, updated in place | Per-worker caches | Persistent pairs + per-thread constraint buffers | One `ScratchColumn<Manifold>`, cleared and refilled each step |
| How order is fixed | `SortContacts` by hash key | Bitset OR + ascending `ctz` walk | Sort by edge id | Sort by `CollidablePair` | Sort by pair index; bitset OR | Serial push in pair order |
| Per-pair cache | `mCache[2]`, read old / write new | Inside the contact record | In the edge | Previous mapping read-only | In the pair | Single in-place table, read then overwrite |
| Granularity | 16 bodies; spawn at 16 pairs | 64 (Box2D) / 20 (Box3D), ≥ 40 µs per task, 8 blocks per worker | 64 edges | Batcher | 64 pairs | — |
| Work claiming | Atomic + CAS stealing | Atomic block cursor | Atomic cursor | Per-worker | Rayon-style | Work-stealing Chase-Lev, pre-cut chunks (solver: lanes × 6) |
| Sleeping pairs | Only active bodies queried | Awake set only | — | — | Active pairs | All pairs every step |

## Key algorithms and techniques
- **Contiguous pair ranges concatenated in chunk order.** This produces exactly the serial stream for any partition. Sorting by pair index (Avian) gives the same order at O(M log M).
  - The in-tree template is the Grid emit (`resources.rs:1787-1954`): serial prefix, `resize(m + reserve)` (`:1857-1861`), workers write disjoint `[lo, hi)` through the `out.solve_base()` raw base (`:1879`), then `truncate` after the join (`:1920-1935`).
- **Per-pair slot plus a flag plus a prefix-sum compaction.** This is order-preserving stream compaction [21], and it is the unification design's S3 shape (see Applicability).
- **Read-only snapshot, deferred write.** Jolt double-buffers; Bepu treats the old mapping as read-only.
  - In boyko, `probe` (`axis_cache.rs:160-179`) is a pure read over `as_read_slice`, and `set` (`:319-354`) is the only writer.
  - The review confirms that a key is written only by its own pair, and that `begin_frame` (`:261-281`) is the only clear.
- **Bitset or list of state changes merged serially.** In boyko the only equivalent is the axis commit list `(k, axis)`, because there are no begin/end-touch events.

## Current code paths and data (file:line, all in `D:/wt/joltab/crates/…`)

**L5: `physics_narrowphase`** (`boyko_physics/src/systems.rs:375-478`)
- It clears `manifolds` (`:382`) and `sensor_overlaps` (`:385`), then calls `begin_frame_synced` (`:391-392`). On a frame where rows changed, that runs the cold `prefetch_remapped` (`axis_cache.rs:392-422`); in all cases it runs `begin_frame` (grow, or clear once load passes 0.5).
- It takes `out` / `sensor_out` build views and `&mut axis_cache` for the whole loop (`:396-398`). The loop runs in pair order (`:400-467`):
  - `is_overlap` (`:411`);
  - the dispatch: sphere-sphere (`:414-416`, `:487-520`), sphere-box and box-sphere with `flip_manifold` (`:417-432`, `:532-539`), and box-box (`:433-447`);
  - for box-box, `read_hint(prefetched, k, a, b)` (`:437`), then `set(a, b, axis)` inside `.map` whenever the result is `Some` (`:444`);
  - the push only when `count > 0`, routed to `sensor_out` or `out` (`:450-466`).
- The counters `PHYS_NP_PAIRS/MANIFOLDS/POINTS` are computed after the loop from the output (`:469-477`). They stay valid after a merge.
- `ColliderShape` has only `Sphere | Box` (`components.rs:127-145`). There is **no capsule**.
- `box_box_contact` (`narrowphase/box_box.rs:653-734`) is documented "ZERO unsafe, no heap allocation, deterministic" (`:44`); so are `sphere_box.rs:5` and `narrowphase/mod.rs:5`. The only statics are the `#[cfg(test)]` thread-locals `FALLBACKS`/`HELD_FACE_YIELDS` (`:746-756`).
  - **Their only reader, `face_preference_never_loses_a_contact`, calls `box_box_contact` directly on the test thread (`:2063-2070`)**, so parallel dispatch does not affect it. The review's O3 undercount would only apply to a future test that reads them through the system at W > 1.
- `Manifold` is 152 B, enforced by a const assert (`manifold.rs:84-122`), and `MAX_CONTACT_POINTS = 4` (`math.rs:34`). At most one manifold per pair.
- `Manifolds` resource (`resources.rs:2268-2354`):
  - The reserve is `capacity.max(scratch_reserve_rows(152))` (`:2321`). Natively that is about 7.06M rows (1 GiB / 152 B, `boyko_ecs/src/ecs/constants.rs:59-60`); under Miri it is about 27.6k rows (4 MiB, `:68-69`).
  - A pair-count-sized staging buffer fits under both ceilings.
- `ScratchColumn`:
  - `solve_base` gives a provenance-preserving write base (`boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:166-171`).
  - `ScratchBuildView` is `!Send` and publishes its length on `Drop` (`…/scratch/views.rs:32-65`). It has `resize` (`:184-198`) and `truncate` (`:208-212`).
  - `ScratchSolveView` is `Copy + Send + Sync` and exposes only `row_ptr` (`:241-320`).
  - **Tree-Borrows rule:** the Grid emit takes its build view only after the join, because "taking it while the workers' raw base was live would invalidate that base" (`resources.rs:1917-1919`).
- **Adding an ECS-owned staging column** (`boyko_physics/src/scratch_ids.rs`):
  - the narrowphase cohort `NARROWPHASE_COLUMN_COUNT = 3` (`:889`) grows by one;
  - the broadphase+narrowphase union is 17 wide (13 + 1 + 3; `:365`, `:924-932`) against `POOL_STAGGER_LINES = 64`;
  - the scratch region has about 38 ids of headroom (`:725-729`);
  - layouts are registered in `register_narrowphase_column_layouts` (`:947-952`).
- **Dispatch pattern to copy** (`solver/colored.rs`):
  - fall back inline below 256 slots (`:2880`);
  - `try_with_active_pool` (`:2911`), with `None` meaning inline (`:3118`);
  - `lanes = num_threads()` (`:2927`, KE16 App-1);
  - `n_chunks = min(lanes × 6, span / 64).clamp(1, groups)` (`:2932-2934`), and `n_chunks < 2` means inline (`:2943-2945`);
  - chunk bounds from a lazy cut iterator with no per-step `Vec` (`:2979-3038`);
  - `pool.scope` + `spawn` (`:3040-3107`).
  - The comment at `:274-321` measured negative scaling at 16 lanes when chunks were too fine.
- **Threadpool:**
  - `PoolInner::scope` boxes one `ScopeShared` (`boyko_threadpool/src/thread_pool.rs:331`);
  - spawn cells go into the per-scope `ScopeBlock` (`scope.rs:1257-1307`);
  - since KE16, a worker's push goes to its own stealable lane (`worker.rs:722-774`);
  - the census says a scope frame costs "1 + (one 4 KiB chunk…)" (`boyko_physics/tests/alloc_frame_census.rs:275-286`).
  - My memory note "a scope opened from a worker serialises" (2026-08-30) is superseded by KE16. P0 measured wide colours going from 12.2 ms to 2.24 ms at W=8 from inside a system body.
- **Census pins that an L5 scope would move:**
  - S1b (1 scope / 1 chunk, exact) and S1c (133 / 229, exact) (`alloc_frame_census.rs:295-303`);
  - the claim "every other physics system … ZERO heap acquisitions" (`:333-334`);
  - the arms set `parallel_solve` and `parallel_broadphase` explicitly (`:1767-1768`, `:1869-1901`).
- **Schedule order** (`plugin.rs:628-699`): select → broadphase → narrowphase → SDF (`:645-646`, only through `add_physics_sdf`) → build_graph (`.after(sdf)`, `:663-665`) → solve.
- **SDF stage** `physics_narrowphase_sdf` (`systems.rs:588-635`):
  - it returns early on an empty field (`:595-597`) and reads the kernel once (`:600`);
  - the per-body loop in row order skips any body that is not simulated and dynamic (`:606-634`);
  - it appends to both streams after the body-body output.
  - It is **not registered by the parity runner**, so P0 did not measure it.
  - `sdf_narrowphase` defaults to `Scalar`. Its `Avx2` arm is known to diverge on ±0 and must not be flipped (`resources.rs:481-489`).
- **Sleeping:**
  - frozen islands skip only solve and integrate; narrowphase still visits every pair (03-TREE-REPORT:14);
  - the island contact key is the island's manifold count, and a change wakes the island (`resources.rs:3491-3549`), so L5 must leave the manifold stream identical;
  - a later frozen-pair skip (L10) would have to keep that count. Box2D does this by keeping sleeping contacts in the sleeping solver set [7].
- **Warm start is already double-buffered** (`warm_read`/`warm_write`, `colored.rs:1456-1459`, `:1501-1502`). The axis table is deliberately a single table (`axis_cache.rs:15-31`).

**L4:**
- `parallel_solve: false` at `resources.rs:501`; its doc comment "Default OFF" is at `:267-288`, and there is a code comment at `:498-500`.
- The whole-step gate is `parallel_solve && widest_color_slots() >= 256` (`colored.rs:3544-3545`).
- **There is no `lanes < 2` shortcut in the solver** (`:2927-2945`), unlike the Grid path (`resources.rs:1761-1765`). At W=1 a wide colour is still cut into up to 6 chunks and dispatched.
- Existing gates:
  - `default_world_worker_invariance.rs:191-282` crosses W with `parallel_solve` off and on;
  - `colored_solve_zero_alloc_o5.rs:144-210` uses the default config but attaches no pool and has narrow colours, so it is unaffected;
  - the census arms set the flag explicitly.
  - Other allocation-counting files, which should be checked for default-config worlds on pools with W > 1: `large_island_gate_p2.rs`, `alloc_frame_attribution.rs`, `constraint_graph_o4_world.rs`, `broadphase_grid.rs`, `soft_*_alloc.rs`.
- The runner's cfg-A already sets `parallel_solve = W>1` (`benches/jolt_parity_pyramid.rs:728-739`).

**L2:**
- `GRID_LO = 96` and `GRID_HI = 192` (`broadphase_policy.rs:79,88`), with a const assert `LO < HI` (`:90`).
- `select_broadphase` reads only `bodies_len()` (`:182`) and returns early unless the mode is `Auto` (`:186-188`). The default mode is `Manual` (`resources.rs:461`), so parity is untouched.
- Signals that exist, but only in `BroadphaseGrid` and only from the previous build: the oversized list for bodies spanning more than `MAX_CELL_SPAN = 8` cells (`resources.rs:619`, `:895-900`), and the median radius used as the cell floor (`:1060-1112`). `select_broadphase` has no parameter that reaches the grid.
- In `tests/broadphase_select_p3.rs`, body counts are derived from the constants (`:115`, `:155`, `:173`, `:194`, `:251`, `:281`); the file has no Miri gating, and its pool is one thread (`:98`).
- **Conflict:** `D:/claude/BoykoEngine/docs/physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md:90` decides "Broadphase default **Auto**" at rung P1. It predates P0, which measured Grid 2.46× (W=1) and 2.59× (W=8) slower on J.

## Pitfalls and mistakes
- **Appending in arrival order.** Contacts added by threads in the order they finish give a nondeterministic Gauss-Seidel order. Box2D names this explicitly [12], and Rapier sorts its reduce output for the same reason [13].
- **Axis `set` during the parallel phase.** It races under linear probing and changes slot layout (03-TREE-REPORT:128). Commit order changes the table bytes but not lookup results.
- **Treating the commit set as the push set.** `set` also runs for sensor pairs and for `Some` results with `count == 0` (`systems.rs:441-446` against `:455-464`).
- **A live `&mut` build view** over a column that workers are writing through `solve_base` breaks Tree Borrows (`resources.rs:1917-1919`).
- **Load imbalance.** About 53 % of pairs (5,037 of 9,561) produce no manifold (SAT early-out), so equal pair counts are not equal work. The engines use many small blocks claimed dynamically (16 to 64, 8 blocks per worker).
- **Over-fine chunks.** They caused negative scaling at W=16 before `MIN_SLOTS_PER_CHUNK` existed (`colored.rs:274-304`). T(16) = T(8) today.
- **Profiling.** A `zone!` inside a chunk task contends (review O1), and W4 counts zones per step.
- **I(W) shifts** when work moves to other cores. P0 measured I(8) = −0.459 ms, and W3 requires I(W) to be reported before and after.

## Relevant academic works
- Harris, Sengupta and Owens, "Parallel Prefix Sum (Scan) with CUDA", GPU Gems 3, ch. 39 (2007): order-preserving stream compaction as flags, then exclusive scan, then scatter [21].

## Applicability to boyko-engine
- **Can be used directly:**
  - the Grid emit's "size to an upper bound, write disjoint ranges through `solve_base`, truncate after the join" idiom;
  - the solver's dispatch skeleton (pool probe, inline fallback, `n_chunks < 2` inline, lazy cuts);
  - the purity of every per-pair kernel;
  - the fact that pair order is the output order.
- **Needs adaptation:**
  - Box2D's per-contact cache lives in a persistent record that boyko does not have. U7 `PairCache` is planned as double-buffered (`D:/claude/BoykoEngine/docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:159`, `:307`) but is not built.
  - Transient per-chunk output must be ECS-owned and reuse its capacity (principle 0 and no per-step allocation). That means a registered `ScratchColumn`, not per-chunk `Vec`s or Avian-style `ThreadLocal` buffers.
- **Existing in-repo design for this stage:** unification §10.4 (`…DESIGN.md:646-653`) specifies "Phase 1: … writes `manifold_out[k]`, `valid[k]`, `axis_out[k]`. Phase 2: prefix compaction in pair order".
  - Its rung P1 gate is "manifold multiset and order identical at W ∈ {1,8,16}; G-jolt" (`:799`).
  - Its stated end state is "no physics-local `pool.scope`", using K5a/K5b gangs (`:427`, `:470-471`).
  - Against that, the owner has ruled that refactoring comes last.
- **Arithmetic, not measured:**
  - per-pair cost at W=1 is 3.142 ms / 9,561 = 0.33 µs;
  - Box2D's 40 µs task floor is then about 120 pairs;
  - lanes × 6 at W=8 gives 48 chunks of about 199 pairs;
  - the manifold stream is 4,524 × 152 B = 672 KiB, and a per-pair staging array would be 9,561 × 152 B = 1.39 MiB (L2 is 512 KiB per core and L3 16 MiB on the P0 machine);
  - one extra scope is roughly one wave, with ω₁ = 0.855 µs and ω(8) = 6.54 µs measured on colour waves.

## Open questions for the architect
1. What gates the L5 dispatch: a new flag, `parallel_solve`, or always on at W ≥ 2? This decides whether S1b's and S1c's exact census pins move.
2. Which output layout: per-pair slots plus compaction, dense per-chunk runs at pair offsets in one staging column with a memmove compaction, or `manifolds` itself sized to the pair count and compacted in place?
3. Should the axis commit be serial in pair order after the join? Its cost is not measured; it is the `set` share of today's loop.
4. How should chunks be sized, given SAT early-out imbalance: lanes × k, or a fixed pair floor?
5. Should broadphase and narrowphase share one scope? Unification defers that fusion to a timing (`DECISIONS.md:93`).
6. Should the SDF stage get the same treatment in the same lane? Its cost is not measured.
7. Serial and parallel results are bit-identical only if every thread has the same FP environment (MXCSR, FTZ/DAZ). I found no source on how the reference engines handle this, and should say so explicitly.
8. For L2: should the thresholds move to the uniform crossover (about 1,109) or the disparity one (about 2,978)? Should a disparity signal block Grid, given that one is only available from the previous frame's grid? And how does this reconcile with `DECISIONS.md:90`?
9. For L4: J-P1 at W=1 shows +0.60 %, which is not claimed. Should the solver get the same `lanes < 2` inline path the Grid has? It is bit-identical either way.

## Sources
[1] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/PhysicsSystem.cpp — FindCollisions jobs, queues, stealing, the SortContacts call site
[2] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ContactConstraintManager.cpp — sort key, cache-hit conditions, fetch_add allocation, `mCache` swap
[3] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/PhysicsSystem.h — batch constants (16 / 16 / 64 / 256)
[4] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/PhysicsSettings.h — pair-cache thresholds (1 mm, about 2°)
[5] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ContactConstraintManager.h — `ContactAllocator`, `mCache[2]`, 4096-B blocks
[6] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Docs/Architecture.md — determinism conditions
[7] https://raw.githubusercontent.com/erincatto/box2d/main/src/physics_world.c — b2Collide, collideSpans, minRange 64, bitset merge, recycling
[8] https://raw.githubusercontent.com/erincatto/box3d/main/src/physics_world.c — b3Collide, minRange 20, bitsets
[9] https://raw.githubusercontent.com/erincatto/box2d/main/src/parallel_for.c — 8 blocks per worker, atomic cursor
[10] https://raw.githubusercontent.com/erincatto/box2d/main/src/contact.c — b2UpdateContact writes only its own contact; per-contact cache
[11] https://raw.githubusercontent.com/erincatto/box3d/main/src/contact.c — `b3ContactCache` (SAT/simplex) inside the contact
[12] https://box2d.org/posts/2024/08/determinism/ — why arrival order breaks determinism; bit arrays OR-merged
[13] https://raw.githubusercontent.com/dimforge/rapier/master/src/geometry/narrow_phase/contacts.rs — broadcast + cursor, BLOCK 64, sort by edge id
[14] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/CollisionDetection/NarrowPhase.cs — per-worker state; old mapping read-only
[15] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/CollisionDetection/NarrowPhasePendingConstraintAdds.cs — deterministic sort by `CollidablePair`
[16] https://raw.githubusercontent.com/avianphysics/avian/main/src/collision/narrow_phase/system_param.rs — par_for_each 64, ThreadLocal bit vectors
[17] https://joonaa.dev/blog/08/avian-0-3 — per-thread buffers sorted by pair index; 4.5×
[18] https://docs.unity3d.com/Packages/com.unity.physics@1.4/manual/concepts-simulation.html — stateless pipeline
[19] https://docs.unity3d.com/Packages/com.unity.collections@0.4/api/Unity.Collections.NativeStream.html — deterministic parallel stream
[20] https://nvidia-omniverse.github.io/PhysX/physx/5.1.0/_build/physx/latest/struct_px_scene_flag.html — `eENABLE_ENHANCED_DETERMINISM`
[21] https://developer.nvidia.com/gpugems/gpugems3/part-vi-gpu-computing/chapter-39-parallel-prefix-sum-scan-cuda — scan and stream compaction
[22] D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md; D:/wt/joltab/docs/physics/perf-campaign/{00-RULINGS,01-PLAN-REV1,02-REVIEW-OF-REV1,03-TREE-REPORT,04-PRACTICE-SURVEY}.md — measured inputs and prior survey
[23] D:/claude/BoykoEngine/docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md, …-DECISIONS.md — the in-repo S3 design and the Auto-default decision (main checkout, not present in joltab)