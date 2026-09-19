# Research: L11, the per-contact solve setup (`build_columns`, `warm_start_apply`, `store_and_swap`)

How this was gathered: I read the tree at `D:/wt/joltab` with Read and Grep. This role has no shell, so graphify could not run, and I did not check that the tree is at `f236ebdd`. Every measurement comes from `docs/measurements/2026-09-19-physics-p0/` (ANALYSIS.md and `p0b/raw/window/driver_reduce/reduction.json`). Every "gain" below is arithmetic on those spans, not a new measurement.

## Brief summary (TL;DR)
- **Setup is about a third of boyko's solve cost per manifold at W=1, and about two thirds of the solve span at W=8.**
  - J-B row (simd on, measured) at W=1: build 1.038 + warm apply 0.614 + store 0.270 = **1.92 ms per step**. That is 0.425 µs per manifold out of a 1.244 µs per-manifold solve.
  - At W=8 the same three spans total 1.90 ms and all run serially. The solve span is 2.98 ms, so they are 64 % of it. The parallel wide colours take 0.96 ms.
- **None of the four reference engines does a hash lookup per contact point per step.**
  - Box2D v3, Box3D, Rapier and Bepu keep the impulses inside a persistent per-pair record. They match points by feature id (at most 4×4 compares), inside the parallel narrowphase.
  - Jolt does one `Find` plus one `Create` per *manifold* in a double-buffered lock-free map. It writes impulses back through a stored handle, with no second lookup.
  - boyko does 16.9k probes plus 16.9k inserts per step, and refills a table of about 1.5 MiB (arithmetic, below).
- **All four prepare constraints once per step, in parallel, directly in the solver's layout.**
  - Box2D writes 8-lane wide structs (AoSoA). Box3D uses one lane per manifold, with the 4 points stored wide.
  - Bepu's narrowphase workers write each constraint in place into its SIMD bundle.
  - Jolt computes the constraint properties inside its narrowphase jobs, then sorts the contacts per island.
  - boyko builds 26 per-point SoA columns serially. Its AVX2 kernel then gathers 20 scalars per lane, per point index, in every one of the 12 sweeps.
- **Warm start is a separate pass in every reference engine.**
  - Box2D runs it as per-colour stages, Bepu as a per-batch sweep, Jolt per island before the iterations.
  - Folding boyko's warm apply into the first sweep changes the order of float additions on each body, so it is **not bit-identical**. The argument is below.
  - Witness: warm apply costs 9.1 ns per point-apply both on S16 (64 points) and on J (16.9k points). So it is limited by compute and dependency latency, not by memory.
- **Effective masses: two schools.**
  - Box2D, Box3D, Jolt and Rapier compute them once per step. For boyko that changes values, because boyko refreshes world inertia every substep.
  - Bepu 2.4 dropped its cached "projection" buffer and recomputes on the fly. boyko also recomputes, 3 masses per point per sweep.

## boyko today (tree facts)

### Measured spans (ms per step; the (i)/(ii) marks follow ANALYSIS.md, noted under the table)
| span | cfg-A W=1 | cfg-A W=8 | J-B (simd) W=1 | J-B W=8 |
|---|---|---|---|---|
| solve_build (`build_bodies` + `build_columns`) | 0.970 | 0.920 | 1.038 | 1.021 |
| warm_apply (4 substeps) | 0.613 | 0.621 | 0.614 | 0.624 |
| store (`store_and_swap`) | 0.266 | 0.208 | 0.270 | 0.254 |
| restitution / write_back | 0.008 / 0.003 | 0.008 / 0.003 | 0.008 / 0.003 | 0.008 / 0.003 |
| wide colours | 12.205 | 2.241 | 3.576 | 0.957 |
| solve span | 14.179 | 4.145 | 5.619 | 2.983 |

- **Where the numbers come from:**
  - (i) The cfg-A spans are ANALYSIS.md §2.
  - (ii) The J-B spans are `reduction.json:905-983`.
- **Workload per step:**
  - 4,524.2 manifolds and 16,887.8 points.
  - 9,561 broadphase pairs.
  - 10.79 colours, of which 9.11 are wide.
  - 4 substeps × (1 biased + 2 relax) = 12 sweeps (`resources.rs:452-453`).
- **Cost per point (J-B, W=1):**
  - build: 61.5 ns per point (229 ns per manifold);
  - warm apply: 9.1 ns per point-apply (136 ns per manifold per step);
  - store: 16.0 ns per point (60 ns per manifold).
- **Witness: which costs grow with the working set (my inference).**
  - S16 is one 16-box tower: 64 points, 16 manifolds. There: build 41.9 ns per point, warm 9.1 ns per apply, store 5.9 ns per point (`reduction.json:825-903`).
  - R has 22,958 points. There: build 64.8, warm 9.2, store 20.1 ns.
  - So about 15–20 ns per point of the build and about 10 ns per point of the store grow with the working set. That is the probe or insert into the 1.5 MiB table, plus 26 column write streams.
  - About 42 ns per point of the build, and all of the warm apply, do not grow with it.
- **The resting world (R-S, everything asleep):** build 0.034 ms; store 0.316 ms (W=1) and 0.366 ms (W=8). This store cost is the B1 carry of 22,968 frozen points (one get plus one insert each) plus the table refill.

### Code path (`crates/boyko_physics/src/solver/`)
- **`build_columns`, `colored.rs:1587-1729`.** It walks colours, then each colour's manifolds in ascending order (from the graph's CSR).
  - A frozen manifold gets only a carry tag (`:1648-1656`).
  - The canonical list is emitted from `manifold_base` (`:1707-1716`). `manifold_fill` first pushes M sentinels (`:1325-1336`).
- **Per manifold (`push_manifold_points`, `:1737-1817`):**
  - `tangent_basis`, which does one `normalize`;
  - two `max` operations for the materials;
  - `remap.manifold_pair`;
  - `cols.build_view()`, which builds and drops **26 views per manifold** (`:1775`, `:1014-1044`; the cost is not measured).
- **Per point:**
  - `vn_initial`, which needs two point velocities;
  - `point_keys`, which packs `(a:24 | b:24 | fid:16)` (`warm_start.rs:148-159`);
  - `warm_read.get`, a Fibonacci-hash linear probe (`warm_start.rs:318-324`, `:378-400`);
  - `push_point` into 26 columns (`:1388-1431`).
- **Columns: 31 `ScratchColumn`s** (`:871-938`, `:947-1001`).
  - Values that belong to the manifold are stored once per point: normal (3), t1 (3), t2 (3), friction, restitution, body_a, body_b, sentinel.
  - `t2 = n × t1` (`contact.rs:67`), so t2 can be derived from the other two.
  - The warm key (`u64`) and `vn_initial` are read only by the store and restitution passes.
- **Warm table.**
  - `WarmEntry` is 24 B (`warm_start.rs:93-100`). The table size is `next_pow2(2·points)`, which gives 65,536 slots ≈ 1.5 MiB at 16.9k points (arithmetic). It never shrinks (`:274-288`).
  - `rebuild` refills every slot on every step (`:274-288`).
  - Keys differ only in their low `fid` bits, so the points of one manifold land on unrelated slots.
- **`store_and_swap`, `colored.rs:3229-3267`.**
  - It rebuilds the table, inserts in canonical order, runs the B1 `carry_frozen` (`:3282-3319`, one get plus one insert per frozen point), then swaps the two tables.
  - Canonical order is needed only so the *slot layout* comes out byte-identical. What a lookup returns depends only on the key set (`colored.rs:3203-3209`; `warm_start.rs:220-226`).
  - The test hook `raw_slots` compares layouts (`warm_start.rs:297-302`).
- **`warm_start_apply`, `:1881-1918`.** It is scalar and walks every slot in slot order (colour order). Writes are guarded by movability. It runs once per substep, after gravity (`:3557-3565`).
- **Substep order (`:3547-3617`):** gravity → warm apply → biased sweep → position integrate + `refresh_inertia` → 2 relax sweeps. Restitution, store and write-back run after the substep loop (`:3619-3664`).
- **The AVX2 kernel, `solve_color_avx2`, `:2228-2653`.**
  - One lane is one manifold group; each rank is one point index within the groups.
  - Per rank it gathers 20 columns into stack arrays with scalar loads (`:2441-2468`).
  - It computes **3 `effective_mass_x8` per rank per sweep** (`:2485`, `:2540-2541`).
  - Impulses are scattered back per rank (`:2611-2621`). Bodies are gathered as `BodyEffective`: inverse mass, a full `Mat3` and 2 `Vec3`s (`:2352-2363`, `:2661-2684`).
  - Chunk boundaries are snapped to multiples of `COHORT = 8` from the colour's start (`:2998-3030`, `:324-332`). So which groups form a cohort does not depend on W.
- **The effective mass depends only on `(dir, ra, rb, inv_mass, inv_inertia)`** (`contact.rs:131-148`), never on velocity.
- **Separation is fixed at gather time.** The bias is `max(bias_rate·sep, −MAX)` in every sweep (`:2094-2098`, `:2500`).
- **Manifold order and keys:**
  - Body-body manifolds come out sorted by the `(min, max)` pair (`systems.rs:344-347`, `:400-466`). SDF manifolds follow, one per dynamic row, in row order (`systems.rs:606-634`). So a key per manifold, `(a, b)` or `(a, SENTINEL)`, is unique within a step.
  - Feature ids are distinct within a manifold. The A7-N1 check enforces this (`box_box.rs:1215-1233`; `narrowphase/mod.rs:124-129`, `:242-243`).
  - The graph assigns colours greedily, first-fit, in manifold order; the CSR is stable (`resources.rs:2870-2990`).
- **Row translation works per pair.** An order flip or a new body is a miss (`row_identity.rs:140-157`).
- **FMA stays banned:** "`do-not-fuse`" (`docs/physics/FMA-DETERMINISM.md:16-23`).

## Approaches in state-of-the-art engines

### Box2D v3 (and Box3D)
- **Approach:** a persistent contact record (`b2ContactSim`) holds its manifold, and the manifold holds the per-point impulses. The narrowphase re-matches points in place; a parallel prepare stage writes the SIMD constraints.
- **Algorithm:**
  - `b2UpdateContact` saves `oldManifold`, computes the new one, and copies `normalImpulse` and `tangentImpulse` wherever the `id` matches (nested loop with `break`) [3].
  - It runs inside `b2CollideTask` via `b2ParallelFor`, `minRange = 64`. Changes in touching state go into per-worker bitsets, which are OR-merged and processed serially [6].
  - The only hash is the broadphase `pairSet`, checked for candidate pairs from moved proxies. New pairs are sorted, then created serially [7].
  - Solver stages [2]:
    - `b2_stagePrepareContacts` runs once per step, split into `contactPrepareDim.count` blocks (`maxBlockCount = 4·workerCount`, `minContactsPerBlock = 4`).
    - Then, per substep: `IntegrateVelocities`, **`WarmStart` × `activeColorCount`**, `Solve` × iterations × colours, `IntegratePositions`, `Relax` × colours.
    - Finally `StoreImpulses`, split into blocks.
  - Warm start is its own stage, not fused with the solve. Velocity integration is also a separate stage [2].
  - `b2StoreImpulses_Wide` writes back straight into `contactSims[..]->manifold.points` [1].
- **Data structures:**
  - `b2ContactConstraintWide` [1]: `indexA/B[8]`; wide fields `invMassA/B`, `invIA/B`, `normal`, `anchorA/B` per point, `normalMass`, `tangentMass`, `baseSeparation`, the impulses, `friction`, `restitution`.
  - The masses are computed **once** in `b2PrepareContacts_Wide` and never recomputed in the iterations [1].
  - Padding lanes point to a zeroed dummy contact, `b2_zeroContactSim`. The scatter skips null and non-dynamic lanes [1].
  - The body state `b2BodyState` is 32 B, "designed for fast conversion to and from SIMD via scatter-gather". The AVX2 path loads 8 of them as an 8×8 transpose [8][1].
  - Records live in per-colour arrays (`b2Array_Emplace(color->contactSims)`, swap-remove with a `localIndex` fix-up). A contact keeps its colour while it touches [5].
  - Box3D's `b3ContactConstraintWide` (4 lanes, SSE): **one lane per manifold**, `pointCounts[4]`, `points[B3_MAX_MANIFOLD_POINTS]` wide.
    - The constraint itself stores `invIA/invIB` as `b3SymMatrix3W`, plus `tangentMass` (2×2), `twistMass` and `frictionImpulse`/`twistImpulse` per manifold.
    - The masses are not recomputed during the solve [11].
    - `b3UpdateContact` matches `normalImpulse` by `featureId` and marks the matched old point used (`featureId = UINT32_MAX`) [12].
- **Trade-offs:**
  - Gains: no per-step lookup; setup is parallel and linear; the solve does wide loads.
  - Costs: once-per-step masses and inverse inertia (substeps reuse them); records have to be maintained in the colour arrays.
- **Numbers:**
  - Large pyramid, 5050 bodies, 14,950 contacts, 4 workers on a 7950X: AVX2 0.90 ms, SSE2 1.02 ms, scalar 1.91 ms [9]. This is 2D, so not comparable per contact.
  - No per-stage prepare or store numbers are published: **no reliable information found.**
- **Determinism:** 2 threads give the same result as 8. Bitsets are OR-merged, FMA is off, and the trig functions are custom [10].
- **Sources:** [1]–[12]

### Jolt (v5.3.0 pinned; master read)
- **Approach:** a double-buffered cache of manifolds keyed per sub-shape pair. The constraint setup happens inside the narrowphase jobs.
- **Algorithm:**
  - Key: `SubShapeIDPair{body1, sub1, body2, sub2}` plus its hash. Then `mReadCache->Find(key, key_hash)` and `mWriteCache->Create(...)`, **once per manifold** [13].
  - Points match by **position in each body's local space**, within `mContactPointPreserveLambdaMaxDistSq = Square(0.01f)` (1 cm) on both bodies. A miss sets lambda to 0 [13][16].
  - `CalculateNonPenetrationConstraintProperties` and `CalculateFrictionConstraintProperties` run inside `AddContactConstraint` and `GetContactsFromCache`, which are narrowphase jobs.
  - Constraints are allocated by an atomic `fetch_add` of `(size<<32)+1` into one contiguous AoS buffer [13].
  - The `SetupVelocityConstraints` job serves non-contact constraints only [15]. In P0's Jolt profile it takes 0.0005 ms (`p0b/raw/window/analysis.json:2712-2717`).
  - Per island or split: `SortContacts(mSortKey)` runs **for determinism**, then `WarmStartVelocityConstraints`, then the velocity steps. `StoreAppliedImpulses` writes back through `mCachedManifoldHandle`, with no lookup [15][13].
- **Data structures:**
  - `CachedContactPoint` holds `mPosition1/2` and `mNonPenetrationLambda`.
  - `CachedManifold` holds `mContactNormal`, `mFrictionLambda[2]` and `mAngularFrictionLambda` (master's friction per manifold).
  - `CachedBodyPair` holds `mDeltaPosition` and `mDeltaRotation`.
  - Two `LockFreeHashMap`s, used read-old / write-new [14].
- **Trade-offs:**
  - Gains: setup overlaps collision detection and runs in parallel; one lookup per manifold.
  - Costs: a lock-free map. The published scaling PDF says that beyond about 16 cores "the lock free operations that are used to manage the contact cache dominate" (`04-PRACTICE-SURVEY.md` [15]).
- **Determinism conditions:** the same order of API calls and the same binary [17]. I did not find a sentence about thread count.
- **Measured (P0):**
  - FindCollisions, which includes this setup and the pair-cache replay: 5.80 ms of profiled thread time at W=1. The analyst's timed bracket is 3.0–4.6 ms, i.e. **0.35–0.54 µs per manifold** for broadphase + narrowphase + setup.
  - boyko's matching figure is bp + np + build + store = (2.10 + 3.14 + 1.04 + 0.27) / 4519 = 1.45 µs.
- **Sources:** [13]–[17]; ANALYSIS.md §3.

### Bepu v2
- **Approach:** the constraint *is* the persistent record. The narrowphase updates it in place.
- **Algorithm:**
  - `PairCache.IndexOf(pair)` returns the constraint handle. If the type is unchanged, `Solver.ApplyDescriptionWithoutWaking` followed by `accessor.ScatterNewImpulses` runs in the narrowphase worker.
  - If the type changed, a removal is enqueued and a pending add is requested. New pairs go through `PairCache.Add(workerIndex, …)`, with a `DeterministicAdd` flush [18][19].
  - `RedistributeImpulses` matches by feature id. Old impulses that do not match are summed and split equally over the unmatched new contacts [18].
  - The solver per substep: `IncrementalUpdate` (depth from velocities), then `WarmStart` per batch ("warmstart doesn't try to store out anything"), then `Solve` per batch per iteration.
  - Velocity integration is **embedded in the warm start** of the first batch that touches each body ("integration responsibilities") [21].
- **Data structures:**
  - `Contact4PrestepData` is AOSOA (`ConvexContactWide`×4, a `Vector3Wide` normal, material properties).
  - `Contact4AccumulatedImpulses` holds `Penetration0..3`, `Tangent` (`Vector2Wide`) and `Twist` [20].
  - v2.4: "Constraint type batches no longer have a 'projection' buffer; anything loaded from it is now recalculated on the fly" [22].
- **Trade-offs:**
  - Gains: no setup stage at all, and the least memory traffic. The author's opinion: "the primary determinant of performance in the solver is convergence per byte memory bandwidth" [23].
  - Costs: math is recomputed in every iteration.
- **Numbers:** v2.4 changelog: "Whole frame speedups in excess of 2x were not uncommon". That covers many changes at once and cannot be attributed to this one [22].

### Rapier (0.35)
- **Approach:** impulses live in the manifold points, and the solver constraints are rebuilt each step.
- **Algorithm:**
  - `ContactWithCoulombFrictionBuilder::generate` runs per step and packs `SIMD_WIDTH` manifolds per constraint. It computes the jacobians, `projected_mass` and `rhs`, and reads `data.warmstart_impulse` and `warmstart_tangent_impulse` from the manifold points.
  - `update` runs per substep and recomputes `dist` and `rhs`, and scales the warm start. Writeback goes to `manifold.points[..].data.warmstart_impulse` [24].
  - Across frames, parry matches points by `fid1`/`fid2` in a nested loop with no `break`, or by position [26].
  - "a parallel build fans the colored constraints across `num_threads` workers"; "Chunks are collected in order, so the reduction stays deterministic" [25].
  - I did not verify where the warm start sits in Rapier's substep order.

## Comparative table

| Aspect | Box2D v3 / Box3D | Jolt | Bepu v2 | Rapier 0.35 | boyko (tree) |
|---|---|---|---|---|---|
| Impulses between steps | inside the contact record's manifold | double-buffered lock-free map | inside the constraint slot | manifold point data | double-buffered open-addressed table **per point** |
| Lookups per step | 0 per contact | 1 Find + 1 Create per manifold | 1 `IndexOf` per pair | none observed | 1 probe + 1 insert **per point**, plus the refill |
| Point matching | feature id (Box3D marks matched points used) | local position ≤ 1 cm | feature id; unmatched impulse redistributed | feature id or position | implied by the key `(a, b, fid)` |
| Where matching and setup run | parallel collide + parallel prepare stage | narrowphase jobs | narrowphase workers, in place | parallel build | serial `build_columns` |
| Layout | wide AoSoA per colour (8 lanes AVX2; Box3D lane = manifold) | AoS, atomic append | AOSOA bundles | SIMD_WIDTH manifolds | 26 per-point SoA columns; kernel gathers per rank |
| Effective mass | once per step | once per step | on the fly | once per step; rhs per substep | on the fly, 3 per point per sweep |
| Warm start | separate per-colour stage per substep | per island before iterations | separate per-batch sweep, integration folded in | not verified | serial all-slot pass per substep |
| Store | parallel, into the records | per island, via handle | in place | into manifold points | serial canonical hash insert + B1 carry |
| Determinism device | OR-merged bitsets; sorted new pairs | `SortContacts` per island | `DeterministicAdd` | ordered chunk reduction | canonical store order; colouring is a pure function |

## Key algorithms and techniques
- **A persistent record plus feature-id re-match** (Box2D, Box3D, Bepu, parry): at most 4×4 compares per manifold, with no hash per point. Semantics differ between engines:
  - Box2D stops at the first match.
  - Box3D marks each matched old point as used.
  - parry lets the last match win.
  - Bepu redistributes unmatched impulses.
- **One lookup per manifold** (Jolt): a single hash access per manifold instead of one per point; points then matched within the entry.
- **Wide prepare with padding lanes** (Box2D): each wide constraint is a pure function of its contacts, so any split into blocks gives the same bytes.
- **Integration folded into warm start** (Bepu): gravity or velocity integration is done by the first batch that touches each body.
- **Hash tables whose layout does not depend on insertion order:**
  - Ordered, history-independent linear probing (Blelloch and Golovin, FOCS 2007) gives a table state that does not depend on the order of operations.
  - Phase-concurrent hash tables (Shun and Blelloch, SPAA 2014) are "based on linear probing, and rel[y] on history-independence for determinism" [27].
  - Such a layout would remove the serial constraint that `store_and_swap` has today. I read these at abstract level only.
- **Merge-join on the sorted stream (already in the tree's design docs):**
  - The L10 rev 2 design (D4) finds each pair's replay source by "a monotone merge-join of the current stream against `pairs_prev` / `manifolds_prev` (translated through `inv` on Rows steps, jumper entries skipped)". D12 states that translation is monotone for rows other than jumper rows (`levers/L10-sleeping/04-DESIGN-REV2.md:109-113`, `:182-186`).
  - The manifold stream is already sorted by pair (above).

## Bit-identity of the specific asks (derived from the tree; the architect has to prove them)
1. **Parallel build, per colour or per chunk.**
   - Each slot's 26 values are a pure function of `(m, bodies, bodies_eff, warm_read, remap)` (`colored.rs:1737-1817`), and `warm_read` is only read during the build.
   - Each slot's position is the colour offset plus a prefix sum within the colour. Both come from the graph's CSR, which is a pure function of the manifolds.
   - The per-colour point prefix is not recorded today: `color_offsets` is built incrementally (`:1679`), so a counting pass would be needed.
   - `canonical`, `manifold_base` and `group_start` are written per manifold or per group, so writes do not overlap. The `WarmSeedStats` fields are sums.
   - ⇒ Any partition gives the same bytes. This is the same property Box2D's per-block prepare relies on.
2. **Store.** Values depend only on the key set; slot layout depends on insertion order, which is why IM-2b inserts serially in canonical order. A parallel insert into the current table would break the layout while keeping the values.
3. **Warm apply per colour, run in parallel:** bit-identical. For each body the adds happen in order of colour, then point, whichever thread runs them (plan L7 row, `01-PLAN-REV1.md:180`).
   - An **8-wide** version is bit-identical under O7's op-for-op, no-FMA rule, provided each body keeps the scalar sequence `n·ni + t1·ti1 + t2·ti2`, then `·(-1)` for body A (`:1886-1916`).
4. **Folding warm apply into the first sweep: not bit-identical.**
   - Take a body X that appears in colours c0 < c1.
   - Today the order is warm(c0), warm(c1), solve(c0), where solve(c0) sees warm(c1).
   - Fused, the order is warm(c0), solve(c0), warm(c1), so solve(c0) no longer sees warm(c1).
   - No reference engine does this.
5. **A warm entry per manifold with an in-entry feature-id scan gives the same values as today's per-point key**, on two conditions:
   - feature ids are distinct within a manifold (the A7-N1 invariant);
   - translation stays per pair (`manifold_pair`, `colored.rs:1770`).

   Only the table layout would change.
6. **Caching effective masses per inertia "epoch" is bit-identical; once per step (Box2D/Jolt) is not.**
   - Within one substep the biased sweep and the relax sweeps see two different inertia states: `refresh_inertia` runs between them (`:3597-3600`).
   - So a cache would compute 8 mass sets per step instead of 12. It would add 3 floats per point of memory traffic for each set. That is arithmetic; no bench was run.
7. **Deriving t2 in the kernel** with the same operation as `contact.rs:67` gives the same bits and drops 3 columns.

## Expected gain (arithmetic on P0 spans, not measured)
- **Per manifold, W=1.**
  - boyko with simd (derived T 11.04–11.14 ms): 2.44 µs per manifold. Jolt v5.3.0: 1.86 µs; v5.6.0: 1.14 µs, normalised with v5.3.0's manifold count.
  - Setup is 0.425 µs of the 2.44.
  - **Upper bound, setup reduced to zero:** 2.02 µs, which is 1.08× v5.3.0 and 1.77× v5.6.0. So L11 alone cannot reach per-contact parity with v5.6.0.
  - What remains per manifold: bp 0.465, np 0.695 and the kernel 0.79 µs.
- **Pieces at W=1:**
  - store: at most 0.27 ms;
  - the part of the build that grows with the working set (S16 against J): about 0.33 ms;
  - warm apply: 0.61 ms. If the O7 colour ratio of 3.41× carried over, it would be about 0.18 ms, saving about 0.43 ms. That is an analogy; it has not been measured.
  - The share of the build spent on the probe is **not measured**. A sub-zone or a microbench is needed.
- **W=8.**
  - The three serial spans total 1.90 ms.
  - One fork-join over the build, with E = 0.68 carried over, takes it from 1.02 to about 0.19 ms (−0.83 ms).
  - Warm apply per colour (L7 formula, ANALYSIS §9): −0.26 ms with ω(8), up to −0.47 ms with ω₁. The per-colour warm pass adds 4 × 9.11 = 36.4 waves; at 6.54 µs each that is about 0.24 ms of dispatch.
  - A store with no serial hash: at most −0.25 ms.
  - Total: −1.3 to −1.5 ms against the projected T(8) of 5.5–5.7 ms (simd + L5). That gives 4.0–4.4 ms, which is 1.13–1.25× v5.3.0 and 1.6–1.8× v5.6.0.
- **Resting world (R-S):** the store's B1 carry costs 0.32–0.37 ms per step, which is 70 % of that world's solve. L10 rev 2 moves it into its held store.

## Interactions
- **L8 (friction per patch).**
  - The warm data becomes one normal impulse per point, plus 2 tangent impulses and 1 twist per manifold. Jolt master (`mFrictionLambda[2]`, `mAngularFrictionLambda`), Box3D (`frictionImpulse`, `twistImpulse`) and Bepu (`Tangent`, `Twist`) all keep those three per manifold.
  - That is a natural fit for a warm entry per manifold. The plan's L8 row already expects "a new warm-key class" (`01-PLAN-REV1.md:181`).
  - L8 also cuts warm apply work by up to ρ = 0.399.
  - If L11 ships first, the column set and the key change twice.
- **L5.**
  - The manifold stream stays byte-identical: output in pair order, then serial compaction (`L5 02-DESIGN-REV1.md:178-190`).
  - A lookup inside the narrowphase chunks is safe only against a read-only table. Colour order is not known until the graph is built, so seeds would have to be indexed by manifold.
  - The scope census is ruled: "+1 scope per step" is accepted for L5 (`levers/00-RULINGS.md:23-27`). A parallel build would add another scope to that census.
- **L10 rev 2.**
  - The kept warm entries are owned by `SleepSets` (D5), so the carry disappears for held islands.
  - D4's merge-join and D12's monotone translation are precedents for a store without hashing.
  - Public views stay logical (ruling B1).
- **Tree broadphase.** It emits exactly the AllPairs set (`broadphase/04-DESIGN-REV2.md:52`), so L11's inputs do not change. The tree's `stage2_rows` accessor (jumper rows) "lands in the bp lane".
- **B1 carry.** Today it is one get plus one insert per frozen point (`colored.rs:3246-3256`). Box2D keeps impulses in place in the record when contacts sleep (per the L10 research).
- **U7 `PairCache`** (unification design D5, keyed by slot and double-buffered; `PHYSICS-ECS-UNIFICATION-DESIGN.md:159-168`) is the structural fix, but the owner ruled "refactor last".
- **L6 and L7.** The dispatch cost ω per wave decides whether a per-colour warm pass is worth it.

## Pitfalls and mistakes
- **Folding warm start into the first sweep** (argument above), and **computing masses or inverse inertia once per step**: both change values.
  - Box3D's symmetric `invI` (6 floats) would change bits unless boyko's `Mat3` is bitwise symmetric. I did not check that.
- **A parallel insert into the linear-probe table** breaks IM-2b's layout guarantee. Values survive; the `raw_slots` layout gates do not.
- **Matching semantics differ between engines:** position ≤ 1 cm (Jolt), redistribution of unmatched impulses (Bepu), marking matched points (Box3D). Adopting any of them is a value change.
- **Lock-free caches limit scaling past about 16 cores** (Jolt's own PDF).
- **An AoSoA layout also changes the scalar oracle** `solve_color` (`:2028-2174`), and with it the {scalar, simd} bit oracle. It also changes the restitution, store and warm passes, which walk the per-point columns.
- **The kernel's C1 rule has to be restated** for any per-manifold arrays: every gathered slot must lie within the worker's own span (`:2211-2224`).
- **The table never shrinks** (`warm_start.rs:274-288`), so the refill cost follows the largest contact count ever seen.
- **Principle 0:** Box2D allocates its wide constraints from a per-step stack arena. The boyko equivalent has to be a `ScratchColumn` or a resource-owned column.

## Relevant academic works
- Shun and Blelloch, "Phase-concurrent hash tables for determinism", SPAA 2014: a table state independent of operation order within a phase [27] (abstract only).
- Blelloch and Golovin, "Strongly History-Independent Hashing", FOCS 2007: linear probing with a unique representation (search summary only; not read).
- Chen et al., 2007: the basis of Jolt's island splitter (`04-PRACTICE-SURVEY.md`).

## Applicability to boyko-engine
- **Usable directly:**
  - a wide or AoSoA prepare with padding lanes (Box2D);
  - store written straight into its own record (Box2D, Jolt handle);
  - per-colour parallel warm start (Box2D stage, equivalent to L7);
  - integration folded into warm start (Bepu).
- **Needs adapting:**
  - The persistent record: boyko keys by row and rebuilds colours every step. The candidates are one entry per manifold, a merge-join (the L10 precedent), or U7 later.
  - Setup inside the narrowphase: blocked by the colour order, which only exists after the graph build.
- **Does not fit without a value change:** once-per-step masses and inertia (Box2D, Jolt, Rapier); matching by position (Jolt).

## Open questions for the architect
1. Must L11 stay bit-identical, or does the owner's approval of value-changing levers cover it (once-per-step masses, a different matching rule)?
2. Should L11 come before or after L8, given that the warm key class and the column set change in both?
3. Given "refactor last", is a merge-join store (L10 D4-style) enough, or should L11 wait for U7? How should jumper rows and order flips fall back?
4. How much of the build is the probe? The witness bounds it at about 0.33 ms, but a sub-zone or a microbench is needed before choosing.
5. Should the scalar oracle read the new layout, or keep a separate mirror (cost)? And which gates pin the table layout (`raw_slots`) as opposed to the values?
6. How should Jolt be bracketed per stage for the per-contact metric, since its setup cost sits inside FindCollisions?

## Sources
[1] https://raw.githubusercontent.com/erincatto/box2d/main/src/contact_solver.c — the wide constraint, prepare, gather, scatter, store
[2] https://raw.githubusercontent.com/erincatto/box2d/main/src/solver.c — stage order, block sizing
[3] https://raw.githubusercontent.com/erincatto/box2d/main/src/contact.c — feature-id impulse matching
[4] https://raw.githubusercontent.com/erincatto/box2d/main/src/contact.h — `b2ContactSim`
[5] https://raw.githubusercontent.com/erincatto/box2d/main/src/constraint_graph.c — records per colour
[6] https://raw.githubusercontent.com/erincatto/box2d/main/src/physics_world.c — parallel collide, bitset merge
[7] https://raw.githubusercontent.com/erincatto/box2d/main/src/broad_phase.c — `pairSet`, sorted creation
[8] https://raw.githubusercontent.com/erincatto/box2d/main/src/body.h — 32 B `b2BodyState`
[9] https://box2d.org/posts/2024/08/simd-matters/ — SIMD benchmark numbers
[10] https://box2d.org/posts/2024/08/determinism/ — multithreaded determinism
[11] https://raw.githubusercontent.com/erincatto/box3d/main/src/contact_solver.c — lane = manifold, central friction
[12] https://raw.githubusercontent.com/erincatto/box3d/main/src/contact.c — `featureId` matching
[13] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ContactConstraintManager.cpp
[14] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ContactConstraintManager.h
[15] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/PhysicsSystem.cpp
[16] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/PhysicsSettings.h
[17] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Docs/Architecture.md
[18] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/CollisionDetection/NarrowPhaseConstraintUpdate.cs
[19] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/CollisionDetection/ContactConstraintAccessor.cs
[20] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/Constraints/Contact/ContactConvexTypes.cs
[21] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/Solver_Solve.cs
[22] https://raw.githubusercontent.com/bepu/bepuphysics2/master/Documentation/changelog.md
[23] https://github.com/bepu/bepuphysics2/issues/104 — the author's bandwidth argument (opinion)
[24] https://raw.githubusercontent.com/dimforge/rapier/master/src/dynamics/solver/contact_constraint/contact_with_coulomb_friction.rs
[25] https://raw.githubusercontent.com/dimforge/rapier/master/src/pipeline/physics_pipeline/solve.rs
[26] https://raw.githubusercontent.com/dimforge/parry/master/src/query/contact_manifolds/contact_manifold.rs
[27] https://dl.acm.org/doi/10.1145/2612669.2612687 — Shun and Blelloch 2014 (abstract via search result)

Tree documents:
- `D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md`
- `D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/p0b/raw/window/driver_reduce/reduction.json`
- `D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/p0b/raw/window/analysis.json`
- `D:/wt/joltab/docs/physics/perf-campaign/01-PLAN-REV1.md`
- `D:/wt/joltab/docs/physics/perf-campaign/04-PRACTICE-SURVEY.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/00-RULINGS.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L5-narrowphase/02-DESIGN-REV1.md`
- `D:/wt/joltab/docs/physics/FMA-DETERMINISM.md`
- `D:/claude/BoykoEngine/docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md`

Source files:
- `D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/warm_start.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/contact.rs`
- `D:/wt/joltab/crates/boyko_physics/src/systems.rs`
- `D:/wt/joltab/crates/boyko_physics/src/resources.rs`
- `D:/wt/joltab/crates/boyko_physics/src/row_identity.rs`
- `D:/wt/joltab/crates/boyko_physics/src/manifold.rs`