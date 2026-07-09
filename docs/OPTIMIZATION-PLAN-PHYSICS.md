# Architecture: boyko_physics production-scale optimization

> Final plan — folds in the architecture-critic review. Save as `docs/OPTIMIZATION-PLAN-PHYSICS.md` and execute. Resolution of every critic remark is recorded in the **Changelog vs critic review** section at the end; each critical/important is also resolved inline in the phase or decision it touches.

## Goal

Take the shipped physics path — single-threaded scalar TGS-Soft `SoftStepSolver` + O(n²) all-pairs broadphase (`systems.rs:188-212`) — to production scale (10k–100k+ rigid bodies, large resting scenes) **without ever breaking bit-determinism** (`solver_is_deterministic` / IM-2). Every phase is OPTIMIZATION: the same converged physics class, faster. Targets (criterion is the oracle — every number below is a *target to validate*, attributed to its source; no number is a claimed result):

| Lever | Today | Target | Source basis |
|---|---|---|---|
| Broadphase | O(n²): ~n²/2 pair tests/step | O(n + candidate pairs) | uniform grid / counting sort |
| Solve threading | 1 thread | ~workers× on the colored solve (bounded by island/color count) | Box2D-v3 coloring + the Phase-9 pool |
| Solve width (SIMD) | scalar | ~2–2.3× AVX2 on the contact solve | Box2D "SIMD Matters" |
| Resting scenes | every body solved every step | sleeping islands ≈ free | Box2D islands/sleep |

The determinism gate is **stricter than Box2D** (which guarantees thread-count-invariance but rebuilds its baseline freely): we require **bit-identical** output across {1 thread, N threads, SIMD-on, SIMD-off} AND run-to-run stability. There is **no stored golden baseline today** (`solver_is_deterministic` is a run-twice-in-one-process A==B bit check — W1); where this plan needs a stored regression guard it adds one explicitly.

## Context and constraints

- **Affected subsystems:** `boyko_physics` (`systems.rs` broadphase/narrowphase/solve, `resources.rs` scratch, `solver/*`, `narrowphase/*`, `sdf_query.rs`, `math.rs`); `boyko_threadpool` (consumed read-only via `scope`/`spawn`); `boyko_sdf_math` (batched eval added, scalar stays the sole oracle); NO `boyko_ecs` core edits (the `plugin.rs` precedent — physics makes zero core edits).
- **Invariants preserved (load-bearing — each verified against the shipped code):**
  - **IM-1** gather/apply row addressing: `physics_gather` pushes one `BodyState` per query row in archetype-row order (systems.rs:169-173); `physics_apply` re-walks the SAME query in the SAME order; `BodyIndex(i)` IS the dense array position; the warm-start key `pack(...)` and every manifold index are that same dense row. **No phase may make the live set non-contiguous in the gathered array** (C1 — sleeping is re-specced to honor this).
  - **IM-2** determinism: fixed contact-set order, fixed point order `0..count`, normal-before-friction, fixed substeps/relax, fixed float op order (no reduction reorder), no atomics/fast-math in the solve.
  - **IM-2b (NEW, from C2)** warm-start store order: `store_and_swap` MUST iterate the canonical `(manifold order, point index 0..count)` sequence (the current `self.points[]` flatten order) **independent of the parallel/SIMD solve dispatch order**. The open-addressed table is linear-probed (warm_start.rs:270-291); two distinct keys with colliding home slots resolve in insertion order, so a thread-dependent store order would make next-frame seeds thread-count-dependent — a frame-delayed determinism break. The store reads converged impulses from `points[]` but visits them in canonical order, never in solve order.
  - **C1** SDF sentinel `b_is_sentinel`/`body_b == u32::MAX` rides the one-sided immovable path (soft_step.rs:504-508, 542); never indexes `bodies_eff[u32::MAX]`.
  - **C2-integration** ownership: the owning solver is the SOLE integrator (DYNAMIC only); `physics_integrate` early-returns under `IntegrationMode::SolverOwned` (systems.rs:116-118).
  - **0%-gate:** a body-only / single-threaded / SIMD-off world stays byte-identical. New paths are opt-in (a config flag or `cfg`), never a rewrite of the scalar generic path. **The shipped `SoftStepSolver` and its `PointConstraint` layout are byte-untouched** — the colored/SIMD path is a SEPARATE solver impl (C3, Decision 7).
  - **No new `unsafe`** except the SIMD kernels (O1+), each with `// SAFETY:` + a differential scalar oracle.
- **The determinism boundary (RESEARCH-FAST-MATH):** physics keeps **exact `sqrt`/`1/x`, no `rsqrt`/`rcp`, no `algebraic_*`, no FMA contraction (`mul_add` forbidden in the deterministic path)**. SIMD speedup comes ONLY from width with a pinned lane-reduction order. The same boundary binds `boyko_sdf_math` (CPU/GPU golden source of truth).

---

## Key decisions

### Decision 1: Broadphase = bounded uniform grid (CSR counting-sort), candidate emission then the SAME feasibility predicate as all-pairs

**What:** Replace the O(n²) double loop with a **uniform spatial grid** over a per-step world AABB (cell size ≈ 2× the median body bounding diameter). Bodies bucket into every cell their AABB spans. The grid emits **candidate** pairs (bodies sharing ≥1 cell, deduplicated), then applies the **exact same feasibility predicate the shipped all-pairs uses** — the sphere-bound test `delta.length_squared() <= (rA+rB)²` (systems.rs:198-200) — before pushing into `ContactPairs`. The grid replaces the O(n²) *iteration*, NOT the feasibility predicate (C4).

**Why for OUR system:**
- **Linear, alloc-free, deterministic.** CSR build: per-worker/serial count → prefix sum → scatter into a flat `cell_bodies: Vec<u32>`, all capacity-reused (principle 5). Counting sort is O(n), branch-light, cache-sequential on the scatter (stride-prefetcher friendly).
- **Determinism is trivial to pin:** bodies insert in dense-row order; within a cell the slice is row-sorted; candidate pairs emit by scanning cells in cell-index order then are passed through the sphere-bound filter; the surviving pair list is sorted by `(min,max)` — a pure function of positions+rows, identical every run, and **bit-identical to the all-pairs output set after the filter** (the C4-corrected gate).
- **Parallelizable deterministically on the Phase-9 pool:** count/scatter are disjoint-range; the count uses per-worker local histograms merged in **fixed worker order** (a deterministic combine, NOT shared atomic counters — scope.rs has no shared counters, the merge model is correct). Pair emission partitions cells across workers into per-worker buffers concatenated in fixed worker order, then filtered+sorted — bit-identical to single-thread.

**Alternatives rejected:**
- **SAP:** incremental sorted-axis state carries frame history → determinism-fragile; insertion-sort tie-break under equal coordinates is a hidden non-determinism source; degrades on clustered scenes.
- **Spatial hash of objects (open-addressed by cell key):** probe/emission order is a determinism burden; a dense grid over a bounded AABB is simpler and the rigid world is bounded each step. **Reserved for soft-body self-collision (O11/SP3)** where the world is genuinely unbounded per-particle.
- **BVH:** build/refit determinism tension + log-depth pointer chasing; reserved for the SDF-brick AS, not rigid broadphase.

**Trade-off:** A uniform grid is weak under extreme size disparity. Mitigation: bodies spanning more than `MAX_CELL_SPAN` cells go into a small `oversized` list tested against all others (an n·k residual, k = oversized count) — the standard grid escape hatch, measured-gated. **Crossover note (O1-optional):** the shipped all-pairs inner loop is one sub + one dot + one compare; the grid's counting-sort + scatter + dedup + filter + sort carries a real constant, so the crossover is "measure, expect O(100s)," NOT a promised 200. `AllPairs` stays default below a measured threshold.

### Decision 2: Constraint islands + greedy graph coloring — the prerequisite for BOTH SIMD-solve and parallel-solve

**What:** After narrowphase, partition contacts into **islands** (connected components of the contact graph over dynamic bodies; a static/sentinel body is "ground" that does NOT connect islands — Box2D's rule) via union-find over `BodyIndex`. **Greedy-color** constraints so no color contains two constraints sharing a dynamic body (first-fit over a per-color body bitset). Islands gate sleeping (Decision 5) and bound parallel work; colors enable race-free batching.

**Why:** The shipped `solve_velocities` is sequential Gauss-Seidel — a pair `(a,b)` writes BOTH body rows (soft_step.rs:539-543, 593-596), so naive parallel pair-solve RACES. Coloring is THE enabler: within one color every MANIFOLD touches dynamic bodies no other manifold in the color touches → dispatch the color's manifold-GROUPS in parallel AND/OR pack disjoint groups into an AVX2 lane with no cross-lane conflict. **The invariant is manifold-group granular, NOT point granular:** a face-face manifold has up to `MAX_CONTACT_POINTS=4` points that all share the SAME body pair and are contiguous in one color span, so the ≥2 points of one manifold are body-coupled and MUST stay together (one thread / one lane) — only different manifold-groups are body-disjoint. This is the Box2D-v3 architecture (RESEARCH-FAST-MATH §5).

**Determinism (STRICTER than Box2D), pinned exactly:**
1. **Color assignment order** = manifold order (D4) → identical color partition every run.
2. **Intra-color constraint order** = ascending manifold index (stable within color).
3. **Color solve order** = `0..n_colors` sequentially (colors are a Gauss-Seidel sweep across colors). Within a color the manifold-GROUPS touch disjoint bodies, so reorder-of-GROUPS within a color cannot change the *velocity* result — **but the ≥2 points of a single manifold-group share both bodies and are order-coupled, so they must be solved together (one thread / lane), and this order-independence applies ONLY to the velocity accumulators across GROUPS, NOT to the warm-start store** (C2): the store is forced into canonical order by IM-2b. O6/O7 dispatch / pack per manifold-GROUP (the `group_start` + `color_group_start` CSR delimits each group's slot run), never per point.
4. **Lane-reduction order** pinned in the SIMD kernel (Decision 4).

The colored solve REORDERS the contact sweep vs the shipped manifold-order sweep → different (but valid) converged float values. This is the one VALUE-bearing change; it is isolated to O5 and validated against the tolerance-based acceptance gates (W1 — there is no stored bit baseline in `solver_is_deterministic` to "reset"; the static-row bit-identity gate `static_body_unmoved_under_tgs` must stay GREEN, since coloring must not move a static body).

**Alternatives rejected:** Jacobi (poor convergence, changes results — rejected by RESEARCH-SOFT-BODY). Per-island-single-thread only (parallel across islands) leaves the big single-island stacking case serial — the exact case needing SIMD. We do BOTH: islands for sleeping + coarse parallelism, coloring for fine SIMD/parallel within the big island.

### Decision 3: Spatial body sorting (Morton reorder) — deferred, measured-gated, NOT in the critical path

**What:** Optionally reorder the colored solver's body buffer by Morton/Z-order so contacting bodies are contiguous, improving solve gather/scatter locality.

**Why deferred:** It is a pure cache optimization on TOP of a correct colored solver, and it fights IM-1 (dense row = archetype row, load-bearing for gather/apply and warm-start keys). It needs an indirection layer (`solve_order[i]→row`) and entity-keyed warm-start (OQ-2). Worth it ONLY when a profiler shows the colored solve is L2-gather-bound, and ONLY after warm-start is entity-keyed. **Sequenced last (O10), gated on a measured cache-miss profile.** Coloring already gives most of the locality win.

**Trade-off:** Adds an indirection + a sort per step; only pays when gather-bound. Behind `PhysicsConfig.spatial_sort`, off by default (0%-gate), enabled per-scene when measured.

### Decision 4: SIMD = width-only, determinism-SAFE, gated on coloring, scalar oracle mandatory

**What:** AVX2 (8-wide) SoA kernels for: (a) batched quaternion integrate+normalize (in the owning solver's substep loop, W5), (b) batched `R·I⁻¹·Rᵀ` inertia refresh (`refresh_inertia`, the hot owning-path kernel — W5), (c) the **SIMD-batched contact solve within a color** (8 contacts/lane), (d) `sdf_edit_list_x8` for SDF narrowphase. Each behind `cfg(target_feature="avx2")` + a scalar fallback + a differential proptest asserting `simd_bits == scalar_bits` (bit-exact, the `bitset_intersects_avx2` house template).

**Why:** RESEARCH-FAST-MATH: the win is SoA-wide (3–9×), not AoS-SIMD (1.04×). (a)/(b) are the highest determinism-safe ratio. (c) is the headline (Box2D 2–2.3×) but ONLY safe after coloring (a color guarantees 8 disjoint-body contacts → race-free lane packing). Determinism: exact `sqrtps`/`divps` only, NO `rsqrtps`/`rcpps` (Intel-vs-AMD divergence), NO FMA contraction (separate mul+add), pinned horizontal-reduction order (never `reduce_*`).

**The FMA hazard, concretely:** `mul_add` lowers to one rounding on FMA-capable CPUs but two without — same source, different bits per target. The kernels use explicit `a*b + c`. A measurable perf cost ACCEPTED for reproducibility (RESEARCH-FAST-MATH verdict).

**Trade-off:** each kernel doubles the maintenance surface; the differential test is mandatory CI. AVX-512 (16-wide) deferred (downclocking risk).

### Decision 5: Sleeping = per-island energy threshold, IM-1-SAFE (gather always full)

**What (C1-corrected):** Track per-island kinetic energy (mass-normalized linear+angular speed²) with a debounce counter; an island below `sleep_threshold` for `sleep_frames` consecutive frames is marked asleep. **Sleeping skips only the SOLVE and INTEGRATE work for slept islands — `physics_gather` STILL walks every row and pushes every `BodyState` (IM-1 intact); a slept body's velocity is simply not advanced and its contacts are not solved.** The warm-start table is still rebuilt over the live (gathered) contact set, so slept bodies keep their dense-row warm keys — no warm-start thrash (the C1 failure mode is avoided by construction).

**Why this shape:** IM-1 is enforced (the `physics_apply` desync `debug_assert!` and the dense-row warm key are load-bearing). Skipping gather rows would shift every subsequent `BodyIndex`, trip the assert, and re-map every warm key — defeating O8's own "rest state == awake rest state to ε" gate (slept rows would lose their warm seed). Gathering-all costs one streaming pass over slept bodies (cheap, sequential, prefetch-friendly); the saved work is the solve+integrate, which is where the cost is. **O8 therefore does NOT depend on entity-keyed warm-start or row indirection** (the DAG stays `O8 needs O4-islands` only — the C1 dependency inversion is dissolved).

**Wake conditions (C1/W6-corrected):** wake on (i) a new contact between a slept island and an awake/oversized body (a pure function of the broadphase candidate set), (ii) an explicit `wake(Entity)` API or an `ExternalImpulse`/`Wake` marker component, (iii) a config change. **The `Changed<RigidBody>` route is REJECTED (W6):** the owning solver writes `linear_velocity`/`angular_velocity` back through `Mut<RigidBody>` every step for every awake body, so `Changed<RigidBody>` is set every frame by the engine itself and cannot distinguish a gameplay write from the solver's own write. Wake keys only off signals the solver does not itself trip.

**Determinism:** energy compare is exact; the debounce counter is per-island deterministic; wake conditions are pure functions of contact/marker state. Sleep is a perf state, not a physics state — the gate is "sleeping-ON rest state == sleeping-OFF rest state to ε" + "sleeping-ON is run-to-run bit-deterministic," NOT bit-equivalence to sleeping-off (sleeping deliberately stops integrating).

### Decision 6: SDF-at-scale = CPU-authoritative analytic field; batched `sdf_edit_list_x8` (CPU-only accelerator); the zero-readback brick-proxy hand-off

**What:** (a) Now: SIMD-batch the SDF narrowphase corner/center samples via `sdf_edit_list_x8` (the scalar `sdf_edit_list` stays the SOLE CPU↔GPU oracle, RESEARCH-FAST-MATH #4). (b) Future: when the SDF graduates to a GPU brick atlas (PERF-DIRECTIONS RT-4/MEM-D3), the CPU physics MUST NOT read back per-frame — the hand-off is a **CPU-resident collision proxy** (a coarse brick distance cache the CPU samples, refreshed only when the SDF edit-list changes, NOT per-frame) so the rigid solver stays cache-resident and zero-readback (decision rule 4). The analytic edit-list stays CPU truth for moderate edit counts; the proxy is the high-edit-count escape hatch.

**Invariant (W4, explicit):** `sdf_edit_list_x8` is a **CPU narrowphase accelerator ONLY**. It is **NEVER** an input to a GPU golden diff; the scalar `sdf_edit_list` is the **only** CPU↔GPU oracle. Any future code feeding x8 output into a golden comparison is forbidden (gated by a doc-comment invariant + a code-review checklist item). This removes the future footgun the critic flagged.

**Why:** decision rule 3 (CPU for branchy/low-N rigid resolve). Rigid-vs-SDF is low-N branchy → CPU. Batched eval is the near-term lever; the brick proxy is the at-scale answer that respects zero-readback.

### Decision 7 (NEW, from C3): The colored/SIMD path is a SEPARATE `RigidSolver` impl, selected by config — `SoftStepSolver` is byte-untouched

**What:** Introduce `ColoredSoftStepSolver` as a distinct `RigidSolver` implementation. The shipped `SoftStepSolver` (and its AoS `PointConstraint` layout, soft_step.rs:475-601) is **byte-untouched** — it remains the default and the 0%-gate reference. The colored solver owns its own SoA `ContactColumns` layout internally and is selected via `add_physics_parallel`/`PhysicsConfig`.

**Why (resolves the C3 self-contradiction):** The draft simultaneously claimed "SoA-restructure `PointConstraint`" AND "scalar path byte-identical" — impossible, because the scalar Gauss-Seidel reads that struct. Two solvers cleanly separate concerns: the scalar reference solver keeps its AoS layout and stays the bit-baseline; the colored solver is free to use SoA columns, coloring, threads, and SIMD without touching the reference. The cost is two solvers to maintain — accepted, because it is the ONLY way to keep a true 0%-gate AND a swappable backend (the existing `RigidSolver` seam principle).

**Pool plumbing (resolves the second C3 mismatch):** `RigidSolver::solve` carries no pool handle, and the threadpool is owned by the schedule, not passed to the solver. **The colored solve does NOT dispatch from inside `RigidSolver::solve`.** Instead, `physics_build_graph` produces the `ConstraintGraph` into a resource, and a NEW dedicated stage `physics_solve_colored` (registered ONLY by `add_physics_parallel`) holds the pool handle and drives the per-color `pool.scope` dispatch, calling into the colored solver's per-color kernel (a `fn solve_color(&self, color_contacts, bodies_eff, ...)` that is pool-agnostic and order-independent within the color). The `RigidSolver::solve` seam signature is **unchanged**; the parallelism lives in the schedule stage that owns the pool, exactly where the pool already lives. The colored solver exposes the per-color kernel as a public method the stage calls; the stage owns the color loop, the barrier (scope Drop join), and the substep loop.

**Trade-off:** two solver impls (maintenance surface) + a new stage. Accepted over a seam extension (passing `&ThreadPool` into `solve`) because the seam change would force every `RigidSolver` impl (incl. `Noop`) to carry a pool param it ignores, and the stage-owns-pool model matches the existing schedule ownership.

---

## Data structures

```rust
// ── O2: broadphase grid (resources.rs, capacity-reused, zero per-step alloc) ──
#[derive(Resource, Default)]
pub struct BroadphaseGrid {
    // CSR layout: cell_start[c]..cell_start[c+1] indexes `cell_bodies`.
    cell_start:   Vec<u32>,        // prefix sums, len = n_cells + 1   (flat)
    cell_bodies:  Vec<u32>,        // body rows bucketed by cell        (flat)
    counts:       Vec<u32>,        // scratch histogram, reused          (flat)
    oversized:    Vec<u32>,        // bodies spanning > MAX_CELL_SPAN     (flat)
    candidates:   Vec<(BodyIndex, BodyIndex)>, // pre-filter candidates  (flat, reused)
    // Grid frame params recomputed each step from the broadphase AABB pass.
    origin: Vec3, inv_cell: f32, dims: [u32; 3],
    // Per-worker scratch for the parallel build (FIXED worker count = pool width).
    // W2: a FIXED-COUNT outer Vec (never grows/shrinks per frame) of capacity-
    // reused inner Vecs — the only sanctioned nested Vec, because its outer length
    // is the constant worker count, not a data-dependent count.
    local_hist:  Box<[Vec<u32>]>,  // [worker] local histograms (outer len = workers, fixed)
    local_pairs: Box<[Vec<(BodyIndex, BodyIndex)>]>, // [worker] emission buffers (fixed)
}

// ── O4: islands + coloring (resources.rs) — W2: jagged arrays are CSR-FLATTENED ──
#[derive(Resource, Default)]
pub struct ConstraintGraph {
    // Union-find over dynamic body rows (path-compressed, capacity-reused).
    uf_parent: Vec<u32>, uf_rank: Vec<u8>,
    island_of: Vec<u32>,                 // island_of[row] = island id (flat)
    // CSR: island_manifold_start[i]..[i+1] indexes island_manifolds (NO Vec<Vec>).
    island_manifold_start: Vec<u32>,     // flat
    island_manifolds:      Vec<u32>,     // flat, manifold indices grouped by island
    // CSR: color_start[c]..[c+1] indexes color_contacts (NO Vec<Vec>).
    color_start:    Vec<u32>,            // flat, len = n_colors + 1
    color_contacts: Vec<u32>,            // flat, manifold indices grouped by color (manifold order within color)
    // Coloring occupancy scratch: a single flat bitset per color, addressed
    // color_occ[color * words_per_color + body_word]. n_colors is data-dependent,
    // so the OUTER dimension is folded into one flat Vec<u64> (W2 — no Vec<Vec>);
    // grown by reserve, cleared (not dropped) each frame.
    color_occ:    Vec<u64>,              // flat bitset matrix, reused
    words_per_color: u32,
    n_colors: u32,
}

// ── O8: sleeping (resources.rs) — IM-1 SAFE: gather stays full, only solve/integrate skip ──
#[derive(Resource, Default)]
pub struct IslandSleep {
    energy:      Vec<f32>,   // per-island accumulated KE (exact arithmetic) (flat)
    below_count: Vec<u16>,   // consecutive frames below threshold (debounce)  (flat)
    asleep:      Vec<bool>,  // per-island sleep flag                          (flat)
    // body→awake mask drives the SOLVE/INTEGRATE skip (NOT the gather skip).
    awake_rows:  TouchedMask, // reuse the existing growable bitset type
}
```

**W2 resolution (no-per-step-alloc):** every data-dependent-count dimension (colors, islands, candidate pairs) is **CSR-flattened to a single flat `Vec` + offsets** (the same pattern the grid already uses for `cell_bodies`/`cell_start`). The ONLY nested `Vec` is the per-worker grid scratch, whose **outer length is the FIXED worker count** (a `Box<[Vec<_>]>` allocated once at setup, inner Vecs cleared-not-dropped) — so it never reallocates per frame. The `color_occ` bitset matrix is a flat `Vec<u64>` indexed `color*words_per_color + word`, grown by `reserve` and `clear`-ed (capacity reused). The debug alloc-counter gate (below) therefore holds even as color/island counts fluctuate.

`PhysicsConfig` additions (all defaulted to current behavior so the 0%-gate holds): `broadphase: BroadphaseKind` (`AllPairs` default = byte-identical today, `Grid` opt-in), `parallel_solve: bool` (false), `simd_solve: bool` (false), `sleeping: bool` (false), `sleep_threshold: f32`, `sleep_frames: u16`, `spatial_sort: bool` (false). Each is a single runtime `test/jz` (the one-branch floor); when off, the asm is the existing scalar path.

```rust
// O7: SoA contact columns OWNED BY ColoredSoftStepSolver (NOT a restructure of
// PointConstraint — C3/D7). The shipped AoS PointConstraint is untouched.
struct ContactColumns {
    ra_x: Vec<f32>, ra_y: Vec<f32>, ra_z: Vec<f32>,   // anchor A offset (SoA)
    rb_x: Vec<f32>, rb_y: Vec<f32>, rb_z: Vec<f32>,
    normal_mass: Vec<f32>, separation: Vec<f32>,
    normal_impulse: Vec<f32>, tangent1_impulse: Vec<f32>, tangent2_impulse: Vec<f32>,
    body_a: Vec<u32>, body_b: Vec<u32>,               // gather/scatter indices
    // Laid out so a color is one contiguous SoA span (color_start slices it).
}
```

---

## Public API

```rust
// Broadphase stays a system; the kind is chosen via config (no API break).
pub fn physics_broadphase(scratch: Res<SolverScratch>, cfg: Res<PhysicsConfig>,
                          mut grid: ResMut<BroadphaseGrid>, mut pairs: ResMut<ContactPairs>);

// New post-narrowphase stage building the graph (registered only when parallel/simd/sleep on).
pub fn physics_build_graph(manifolds: Res<Manifolds>, scratch: Res<SolverScratch>,
                           mut graph: ResMut<ConstraintGraph>);

// NEW colored-solve STAGE (C3/D7): owns the pool handle, drives the per-color
// scope dispatch + substep loop, calls into the colored solver's per-color kernel.
// Registered ONLY by add_physics_parallel. The RigidSolver::solve seam is UNCHANGED.
pub fn physics_solve_colored(/* graph, scratch, manifolds, cfg, &ThreadPool (stage-owned) */);

// The colored solver is a SEPARATE RigidSolver impl; SoftStepSolver is untouched.
pub struct ColoredSoftStepSolver { /* owns ContactColumns + warm tables */ }
impl ColoredSoftStepSolver {
    /// Pool-agnostic, order-independent ACROSS the manifold-GROUPS of a color (the
    /// kernel the stage calls per color, optionally lane-wide under O7). Each
    /// manifold-group touches bodies no other group in the color touches; the ≥2
    /// points of one group share both bodies and are solved together. O6/O7 must
    /// dispatch / pack per manifold-GROUP (the `group_start` + `color_group_start`
    /// CSR delimits each group's slot run), never per point.
    pub fn solve_color(&self, color_contacts: &[u32], /* bodies_eff, soft, ... */);
}

// Plugin: a single opt-in entry mirroring add_physics_sdf.
pub fn add_physics_parallel<S: RigidSolver + Default>(
    builder: &mut ScheduleBuilder, world: &mut EcsMaster, opts: PhysicsParallelOpts,
) -> PhysicsStageKeys;

// boyko_sdf_math leaf (scalar stays the SOLE oracle; x8 is a CPU-only accelerator — W4):
pub fn sdf_edit_list_x8(edits: &[SdfEdit], pts_soa: &[[f32; 8]; 3]) -> [f32; 8];
pub fn sdf_edit_list_normal_x8(edits: &[SdfEdit], pts_soa: &[[f32; 8]; 3]) -> [[f32; 8]; 3];

// O8: explicit wake API (W6 — NOT Changed<RigidBody>, which the solver itself trips).
pub fn wake(world: &mut EcsMaster, entity: Entity);
```

---

## Algorithms for critical paths

**Grid broadphase build (O2):**
1. **AABB + median pass (W3):** compute the world AABB and a body-size proxy in the **broadphase stage's OWN first linear read** — NOT folded into `physics_gather` (gather computes neither today and is a 0%-gated stage; broadphase is being rewritten anyway, so the extra read lives here, honestly attributed). The cell-size heuristic uses a **cheap deterministic proxy** — `extent / n^(1/3)` clamped, or a fixed-bucket histogram mode — NOT a sort-based exact median (a median needs no exactness for a cell-size heuristic; a sampling median would be non-deterministic unless seeded, so it is rejected in favor of the closed-form proxy). O(n), sequential.
2. Count: per body, `counts[cell] += 1` for each cell its AABB spans; oversized → `oversized`. O(n·avg_span), cache-friendly.
3. Prefix-sum `counts` → `cell_start`. O(n_cells), sequential.
4. Scatter rows into `cell_bodies` at `cell_start[cell]++`. O(n·avg_span), sequential write.
5. **Candidate emit:** per cell, all-pairs within its (small) body slice + each oversized vs all; dedup a pair sharing 2 cells via "emit only at the min shared cell" (branchless). O(candidates).
6. **Feasibility filter (C4):** push a candidate into `ContactPairs` ONLY if it passes the SAME sphere-bound predicate all-pairs uses (`delta.length_squared() <= (rA+rB)²`). Sort the surviving list by `(min,max)`.
Cache: 2/4 streaming; 5 sequential per cell. SIMD: the AABB/cell + sphere-bound math vectorizes; emission is branchy. Complexity: O(n + candidates) vs O(n²). **Determinism + correctness:** the post-filter pair set is bit-identical (same `(min,max)` order) to all-pairs — the C4-corrected gate. The final sort is budgeted explicitly in the bench (O2-optional: for dense scenes the pair count dominates n, so the sort is a real cost; candidate buffer is capacity-reused).

**Coloring (O4):** greedy, manifold order — for each manifold, find the first color where neither dynamic body's bit is set in the flat `color_occ[color]` row; set both bits; append the manifold index to that color's CSR group. O(contacts · colors_touched). Determinism: manifold order + first-fit → identical partition. The sentinel/static body B is NOT marked (a ground body doesn't constrain a color — matches Box2D, keeps islands from collapsing). Cache: the per-color bitset row is contiguous `u64` words (AVX2-testable via `bitset_intersects`).

**Colored solve (O6 parallel / O7 SIMD):** for substep, for color `0..n_colors`: the `physics_solve_colored` STAGE (which owns the pool — C3/D7) processes the color's CSR contact group — O6 via `pool.scope` disjoint-chunk over the color's contacts; O7 packs 8 contacts/lane (gather body velocities by `body_a/body_b`, solve, scatter). Barrier between colors (the scope Drop join — Gauss-Seidel across colors is sequential). **After all substeps, the warm-start store walks `points[]` in CANONICAL `(manifold, point)` order (IM-2b/C2), independent of the color/thread dispatch order.** Determinism: within a color, body-disjointness makes velocity-accumulator order irrelevant; cross-color order is fixed; the warm store order is canonical. Cache: SoA columns stream; gather/scatter by body index is the strided access O10 may later help. SIMD: the normal+friction solve is the 8-wide kernel; the cone clamp uses exact `sqrtps`. **Lane-tail (O7, O3-optional):** partial lanes pad with **inert zero-`inv_mass` dummies** — verified inert because `effective_mass` returns 0 when `k<=0` (contact.rs:119) and `apply_impulse` is a branchless no-op at `inv_mass==0` (contact.rs:80-83); the differential test MUST also confirm the friction `len_sq>0` guard + `sqrt` + restitution path are NaN-free on a zero/zero dummy lane, and the scalar oracle MUST process the same padded lanes so `simd==scalar` covers the tail.

**SIMD integrate/inertia (O1, W5-corrected):** the hot, owning-path kernels are (a) the in-solver per-substep batched `refresh_inertia` (`R·I⁻¹·Rᵀ`, soft_step.rs:453-460, runs ~`substeps×(1+relax)` times) and (b) the solver's internal gravity + position/quaternion integrate loop. Both are embarrassingly parallel per body (no coloring needed — bodies independent), SoA, exact `sqrtps` normalize, pinned reduction. The **standalone `physics_integrate` SIMD is a LOW-PRIORITY bonus** (it early-returns under `SolverOwned`, systems.rs:116-118 — dead code in the production path), not the lead deliverable.

---

## Multithreading model

- **Shared (read-only during a parallel region):** `scratch.bodies` positions/masses (grid build, graph build); manifolds; the graph partition.
- **Shared (written, disjoint by construction):** within a color, body velocities — each worker/lane processes a whole manifold-GROUP and writes bodies no OTHER group in the color touches (the manifold-group-granular coloring invariant) → no synchronization, no atomics. The ≥2 points of a single manifold share both bodies, so a group is the indivisible unit of dispatch / lane-packing (never split a manifold's points across workers/lanes).
- **Thread-local:** per-worker grid histograms + pair buffers (merged in fixed worker order = deterministic combine). Per-worker SIMD scratch.
- **Synchronization points:** (1) grid build → emit (prefix-sum barrier); (2) between colors (scope Drop join — required, Gauss-Seidel cross-color dependency); (3) between substeps. These are `pool.scope` joins (work-stealing wait, no parking deadlock — Phase 9.1-9.3 proven). NO Mutex/RwLock on the hot path.
- **Atomics:** NONE in the solve. The only atomics are the threadpool's own scope-pending counter (loom+Miri-proven). The grid build uses per-worker locals + ordered merge, NOT shared atomic counters.
- **Data-race freedom proof:** coloring guarantees ∀ color, ∀ two MANIFOLD-GROUPS in it, their body rows are pairwise disjoint → concurrent `apply_impulse` writes from DIFFERENT groups target disjoint `BodyEffective` slots. (The ≥2 points of a single manifold-group share both bodies, so a group is dispatched/packed as one unit — never split — and its intra-group point writes are sequential on the owning thread/lane.) The `pool.scope` Drop join is the happens-before edge between colors (Acquire/Release on the pending counter, loom-verified). The warm-start store runs AFTER the join, single-threaded, in canonical order (IM-2b) → no concurrent table writes. Send/Sync: `BodyEffective`/`ContactColumns` columns are `Copy` POD `Send+Sync`; per-chunk closures capture raw slices bounded by the scope. Arena stays !Send/!Sync — the solve touches only `Vec` scratch.
- **Determinism vs thread count:** within-color the ORDER of manifold-groups is irrelevant (disjoint bodies across groups; points within a group always solved together in fixed point order on one thread/lane); cross-color order is fixed; the warm store order is canonical → the float accumulation sequence per body AND the table occupancy are identical regardless of worker count → bit-identical {1,N} threads. **The {1,N} gate uses a scene with forced home-slot collisions** (dense contact count near the table load-factor bound) so the IM-2b store-order guarantee is tested non-vacuously (C2).

---

## Integration

- **Modules touched:** `systems.rs` (broadphase rewrite behind config; new `physics_build_graph` + `physics_solve_colored` stages; `physics_gather` UNCHANGED — W3/C1); `resources.rs` (3 new resources, `PhysicsConfig` fields); `solver/colored_soft_step.rs` (NEW — `ColoredSoftStepSolver` + `ContactColumns` + the per-color kernel; D7); `solver/soft_step.rs` (UNCHANGED except the in-solver `refresh_inertia`/integrate SIMD of O1, which is additive `cfg` behind a scalar oracle — the AoS `PointConstraint` + `solve_velocities` stay byte-identical for the 0%-gate); `solver/contact.rs` (a SoA effective-mass helper for the colored kernel; the scalar `effective_mass`/`apply_impulse` untouched); `narrowphase/*` (batched SDF sampling); `math.rs` (SoA SIMD helpers); `boyko_sdf_math/lib.rs` (`*_x8` additive); `plugin.rs` (`add_physics_parallel` + the colored stage wiring + `wake`).
- **No core ECS edits.** Consumes `pool.scope` as-is.
- **Seam:** `RigidSolver::solve` signature UNCHANGED (C3/D7). The colored path is a separate impl + a dedicated stage that owns the pool; the graph is a resource the stage reads. Verified compatible with `Arena`/`ComponentPool`/`UnitId` (physics touches none) and dense-row IM-1 (grid/graph index dense rows).

---

## Phases (each: dev → review → tester[Miri+proptest+criterion] → commit; critical-path ordered)

**Dependency DAG (C1/C3-corrected):**
`O1 (SIMD integrate/inertia, independent)` ‖ `O2 (grid broadphase) → O3 (parallel grid build)`.
`O2 → O4 (islands+coloring) → O5 (colored sequential solve + value-change isolation) → O6 (parallel colored solve) → O7 (SIMD colored solve)`.
`O8 (sleeping) needs O4 (islands) ONLY` — **NOT** entity-keyed warm-start (C1: gather stays full, no row indirection).
`O9 (batched SDF)` independent (needs O1's SIMD discipline).
`O10 (Morton sort)` last, measured-gated, needs entity-keyed warm-start.
`O11 (soft-body)` forward, forks on O4's coloring.

> **O6 vs O7 data-structure ownership (OQ-4 resolved):** O6 and O7 are NOT independent siblings. O6 (parallel) establishes the `ContactColumns` SoA layout inside `ColoredSoftStepSolver` and the per-color `solve_color` kernel boundary; O7 (SIMD) consumes that SAME SoA layout, swapping the per-color scalar kernel for the 8-wide one. The chain is strictly linear `O5 → O6 → O7`. O6 introduces SoA (so the parallel chunk-solve reads columns, not the AoS struct); O7 widens it. There is no parallel-AoS interim.

### Phase O1 — SIMD-safe in-solver inertia + integrate (independent warm-up; no reorder)
**What (W5-corrected):** SoA AVX2 batched `refresh_inertia` (the per-substep `R·I⁻¹·Rᵀ`, the HOT owning-path kernel) + the solver's internal gravity/position/quaternion integrate loop, behind `cfg(avx2)` + a scalar oracle. The standalone `physics_integrate` SIMD is a low-priority bonus (dead under `SolverOwned`). **Why first:** highest determinism-safe ratio, NO contact-order change (per-body independent → no value change), proves the SIMD discipline on the easy case in code that actually RUNS in the production path. **Win target:** ~1.4–3× on inertia+integrate (RESEARCH normalize-batch 3.24×). **Effort:** M. **Gate:** differential proptest `simd_bits==scalar_bits` over random bodies; Miri (scalar path); criterion A/B on a 10k-body substep; `solver_is_deterministic` UNCHANGED (no reorder). **Conflict:** none — additive `cfg` path; AoS `solve_velocities` untouched. **Production-ready when:** AVX2 build bit-matches scalar on the 10k-body in-solver inertia+integrate, criterion ≥1.3×, no value change.

### Phase O2 — Uniform-grid broadphase (single-threaded first)
**What:** `BroadphaseGrid` + CSR counting-sort build + candidate emit + the all-pairs feasibility filter (C4), behind `PhysicsConfig.broadphase = Grid` (default `AllPairs` = byte-identical). AABB/median in the broadphase stage's OWN pass (W3 — `physics_gather` untouched). **Why:** O(n²)→O(n+candidates). **Win target:** crossover "measure, expect O(100s)" (O1-optional — not promised at 200); large reduction at 10k. **Effort:** L. **Gate:** the **0%-correctness proptest** `post_filter(grid_pairs(scene)) == allpairs_pairs(scene)` bit-identical (same `(min,max)` order) for random scenes — the C4-corrected load-bearing gate; criterion A/B grid-vs-allpairs @{100,1k,10k} (anti-vacuity: assert `pairs.len()>0`); Miri; oversized-body edge (1 giant + 1k tiny); empty/1-body/all-coincident edges; **zero per-step alloc** (debug counting-allocator assert — W2). **Conflict:** none — opt-in, all-pairs default. **Production-ready when:** post-filter grid pairs == all-pairs on 1000 random scenes bit-identical, criterion shows a measured crossover, zero per-step alloc verified.

### Phase O3 — Parallel grid build
**What:** per-worker histograms (fixed-count `Box<[Vec<u32>]>` — W2) + ordered merge + partitioned candidate emit via `pool.scope`, then the serial feasibility filter+sort (or a partitioned filter with ordered concat). **Why:** broadphase is a non-trivial fraction at scale; embarrassingly parallel. **Win target:** ~workers× on the build at 100k. **Effort:** M. **Gate:** **bit-identical post-filter pairs {1 worker, N workers}**; Miri (scalar build) + native MT; criterion scaling; anti-vacuity. **Conflict:** determinism — ordered merge (no shared atomic counters). **Production-ready when:** N-worker pair set == 1-worker bit-identical on 1000 scenes; criterion scales ≥0.7× linear to 4 workers at 100k.

### Phase O4 — Islands + greedy coloring (the enabler; NO solve change yet)
**What:** `physics_build_graph` (union-find islands + greedy coloring, CSR-flattened — W2); the solver STILL solves sequentially in manifold order (the shipped `SoftStepSolver`). **Why:** land + test the partition in isolation before any solve reorder. **Effort:** L. **Gate:** coloring-correctness proptest (no color shares a dynamic body — re-scan); island correctness (connected components == reference BFS); determinism (same partition every run); zero per-step alloc (W2 CSR); Miri; **`solver_is_deterministic` AND `static_body_unmoved_under_tgs` STILL pass (solve untouched)**. **Conflict:** none (partition only). **Production-ready when:** coloring invariant holds on 1000 random graphs, partition bit-deterministic, solve output byte-unchanged.

### Phase O5 — Colored sequential solve + value-change isolation (W1-corrected, no "baseline reset" ceremony)
**What:** Solve in color order, still single-threaded, via the NEW `ColoredSoftStepSolver` with its `ContactColumns` SoA (D7 — `SoftStepSolver` stays the default/reference). This REORDERS the contact sweep → new converged values. **Why:** isolate the value change here, before threads/SIMD, so the converged-value change is not entangled with a parallelism bug. **W1 correction:** there is NO stored baseline in `solver_is_deterministic` to "reset" — it is a run-twice A==B check that coloring keeps green automatically (both runs colored identically). The ceremony is: **confirm the tolerance-based acceptance gates still pass** (`stacking_is_stable`, `softstep_resolves_penetration`, `box_box_friction_3d`, restitution — they assert tolerances/inequalities, which absorb a converged-value change) AND **`solver_is_deterministic` stays green** AND **`static_body_unmoved_under_tgs` stays bit-identical** (coloring must NOT move a static body). If a bit-exact golden regression guard is wanted, ADD one explicitly here (a new snapshot test) and define future changes against it. **Effort:** M. **Gate:** the colored solve is run-to-run bit-identical; all rest/stack/friction/restitution acceptance gates pass; `solver_is_deterministic` + `static_body_unmoved_under_tgs` green; the IM-2b warm-store canonical order is in place (even single-threaded, so O6 inherits it); Miri. **Conflict:** the converged-value change — isolated, validated against tolerance gates, documented in the CHANGELOG (no non-existent "baseline reset"). **Production-ready when:** colored solver passes all acceptance + determinism gates, is run-to-run bit-identical, the value change is documented.

### Phase O6 — Parallel colored solve (no SIMD)
**What:** the `physics_solve_colored` stage (owns the pool — C3/D7) `pool.scope` disjoint-chunks each color's CSR contact group across workers; barrier between colors; warm store after the join in canonical order (IM-2b). **Why:** the big single-island case (pyramid/stack) goes multi-thread. **Win target:** ~workers× on solve, bounded by color count. **Effort:** L. **Gate:** **bit-identical {1 thread, N threads} on a FORCED-COLLISION dense scene** (C2 — exercises the IM-2b warm-store order non-vacuously); the rest/stack/friction gates; Miri (scalar) + native MT; a scope stress test (loom NOT needed — no new atomics, reuses the proven join); criterion scaling; anti-vacuity (contacts actually dispatched). **Conflict:** data-race — proven free by coloring; determinism — proven by within-color order-independence + canonical warm store. **Production-ready when:** N-thread == 1-thread bit-identical on the stacking/pyramid AND the forced-collision scene, criterion ≥0.6× linear to 4 workers on a 10k-body pyramid.

### Phase O7 — SIMD-batched colored solve (the headline)
**What:** swap the per-color scalar kernel for the AVX2 8-wide normal+friction solve over the SAME `ContactColumns` SoA O6 established (OQ-4 — strictly after O6), behind `cfg(avx2)` + `PhysicsConfig.simd_solve` + a scalar oracle. Exact `sqrtps`/`divps`, NO `mul_add`, pinned lane order; inert zero-`inv_mass` lane-tail padding (O3-optional verified). **Why:** Box2D 2–2.3×; compounds with O6. **Win target:** ~2–2.3× atop O6. **Effort:** XL. **Gate:** the mandatory differential `simd_bits == scalar_bits` (bit-exact, incl. partial-lane tails — scalar oracle processes the same padded lanes); rest/stack/friction; bit-identical {SIMD-on, SIMD-off}; Miri (scalar); criterion A/B; anti-vacuity. **Conflict:** FMA/rsqrt determinism — forbidden by construction; lane-tail NaN — inert padding + the explicit O3-optional NaN check on the dummy lane. **Production-ready when:** AVX2 solve bit-matches scalar on 1000 colored scenes incl. partial lanes, criterion ≥1.8× atop O6, the colored bit-result is unchanged vs the O5 scalar colored result (width-only).

### Phase O8 — Sleeping / deactivation (IM-1 SAFE — C1-corrected)
**What:** `IslandSleep` energy tracking + debounce + wake; **slept islands skip only SOLVE + INTEGRATE — `physics_gather` STILL walks every row (IM-1 intact), warm keys stay dense-row (no thrash).** Wake via the explicit `wake()` API / `ExternalImpulse` marker / a new-contact-with-awake-body signal — **NOT `Changed<RigidBody>`** (W6 — the solver trips it itself every frame). **Why:** the big win for large resting scenes (settled worlds ≈ free). **Effort:** L (no longer blocked on OQ-2 — C1 resolution dissolved the dependency). **Gate:** a dropped stack sleeps after settling then a new body wakes it; sleeping-ON rest state == sleeping-OFF rest state to ε; sleeping-ON run-to-run bit-deterministic; no wake/sleep oscillation (debounce proptest); the `physics_apply` desync `debug_assert!` NEVER fires with sleeping on (IM-1 gate); warm-start hit-rate on a slept stack ≈ the awake hit-rate (proves no thrash); Miri; criterion on a 10k settled scene. **Conflict:** determinism — energy exact, wake pure-function; IM-1 — gather stays full (the C1 fix). **Production-ready when:** a 10k resting scene costs ≤5% of an awake step, wake/sleep deterministic + oscillation-free, rest state matches no-sleep to ε, IM-1 assert clean, warm-start un-thrashed.

### Phase O9 — Batched SDF narrowphase (`sdf_edit_list_x8`)
**What:** `boyko_sdf_math::sdf_edit_list_x8` + the box-corner/sphere-center SDF samples batched 8-wide; scalar `sdf_edit_list` STAYS the SOLE CPU↔GPU oracle; **x8 is a CPU-only accelerator, NEVER fed to a golden diff (W4 invariant).** **Why:** the only near-term SDF lever; box-vs-SDF samples 8 corners → a natural 8-wide batch. **Effort:** M. **Gate:** `x8_bits == scalar_bits` differential proptest (bit-exact); the rung 8/9/10/11 GPU goldens STILL bit-exact (scalar unchanged); `cpu_gpu_sdf_agreement` holds; the W4 invariant asserted (doc-comment + review checklist: x8 is never a golden input); Miri; criterion A/B on box-vs-SDF. **Conflict:** the scalar leaf MUST stay the oracle — x8 is a separate fn. **Production-ready when:** x8 bit-matches scalar on 1000 points, GPU goldens unchanged, the CPU-only-accelerator invariant documented, criterion ≥1.5× on box-SDF narrowphase.

### Phase O10 — Spatial body sort (measured-gated, optional)
**What:** Morton reorder of the colored solver's body buffer + warm-start re-key to a content-defined (Entity) key (OQ-2), behind `PhysicsConfig.spatial_sort`. **Why:** cache locality on the colored solve when gather-bound at scale. **Effort:** L (needs entity-keyed warm-start). **Gate:** rest/stack gates; run-to-run bit-determinism (sort is a pure function of position+row); criterion cache-miss A/B (the ONLY justification — land only on a measured win). **Conflict:** IM-1 — handled via an indirection layer (`solve_order[i]→row`) confined to the colored solver + entity-keyed warm-start (the colored solver's own warm table, NOT the shipped `SoftStepSolver`'s). **Production-ready when:** a profiler shows the colored solve is L2-gather-bound at scale AND the sort yields a measured criterion win; otherwise SHELVE.

### Phase O11 (forward) — Soft-body perf path (RESEARCH-SOFT-BODY SP1–SP5)
**What:** colored-Gauss-Seidel XPBD soft-body (a separate position pass after rigid), reusing O4's coloring infra + the open-addressed spatial-hash broadphase variant for self-collision (SP3, the genuinely-unbounded case Decision 1 reserved) + SDF collision via `sdf_edit_list`/`_x8`. **Why:** the forward item; coloring (O4) is its prerequisite. **Effort:** XL (its own SP1–SP5 sub-roadmap). **Gate:** per SP-phase (drop-cube-rests, `soft_is_deterministic`, rest-volume, self-collision, parallel bit-identical, GPU break-even). **Production-ready when:** SP1–SP4 land per their gates; SP5 GPU only past a MEASURED break-even.

---

## Critical-path ordering

1. **O1** (independent, ships any time — start in parallel with O2).
2. **O2 → O3** (broadphase: correctness gate is the foundation; parallel build follows).
3. **O4 → O5** (the coloring enabler, then the isolated value change — the riskiest correctness step, done single-threaded).
4. **O6 → O7** (parallel, then SIMD — strictly linear on the shared SoA layout, OQ-4).
5. **O8** (sleeping — needs O4 islands only; can land after O4, in parallel with O5/O6).
6. **O9** (batched SDF — independent, needs O1's discipline).
7. **O10** (Morton — last, measured-gated, may SHELVE).
8. **O11** (soft-body — forward fork on O4).

The single highest-risk transition is **O5** (the only converged-value change). It is deliberately single-threaded and isolated so a value regression is never confounded with a parallelism or SIMD bug.

---

## Metrics and validation (cross-cutting)

- **Benches (criterion, `[profile.bench] codegen-units=1`, median-of-N per Phase X.E):** broadphase grid-vs-allpairs @{100,1k,10k,100k} (incl. the final-sort cost, O2-optional); build scaling {1,2,4 workers}; colored solve scalar-vs-parallel-vs-SIMD on a pyramid/stack @{1k,10k}; sleeping settled-scene cost; SDF x8-vs-scalar. NEVER compare PGO/mimalloc against a non-PGO baseline.
- **Mandatory unit/proptest:** `post_filter(grid)==allpairs` (C4 0%-correctness); coloring invariant (no shared dynamic body); island==BFS-reference; `simd_bits==scalar_bits` (every SIMD kernel, incl. partial-lane tails — O3-optional); `{1,N}-thread bit-identical on a FORCED-COLLISION scene` (C2/IM-2b); debounce no-oscillation; the IM-1 `physics_apply` desync assert clean under sleeping (C1).
- **Mandatory determinism gates:** `solver_is_deterministic` run-to-run + cross-thread-count bit-identical; `static_body_unmoved_under_tgs` bit-identical (coloring must not move a static body — W1). No non-existent "baseline reset" — if a stored golden guard is wanted it is ADDED explicitly in O5.
- **Miri:** scalar paths every phase (SIMD intrinsics native-only — Miri runs the scalar fallback, the differential test covers SIMD); MT solve paths native-only (the pool is loom+Miri-proven 9.1-9.3, per the P1/W2 precedent).
- **debug_assert! invariants:** broadphase pairs sorted `(min,max)` (existing, extended); coloring invariant re-scan in debug; per-worker merge order fixed; CSR offset monotonicity + group-length consistency; **no per-step alloc (debug counting allocator — W2)**; island id < n_islands; SIMD lane-tail padding inert (zero `inv_mass`); **the IM-2b warm-store walks canonical order (debug check the store index sequence == `0..points.len()`)**; the IM-1 gather-row count == apply-row count under sleeping (C1).

---

## Production-ready (exit criteria for the whole campaign)

The physics path is production-scale-ready when ALL hold:
- **Correctness:** O2's post-filter grid pairs == all-pairs bit-identical on 1000 random scenes; O4's coloring/island invariants hold on 1000 random graphs.
- **Determinism:** {1,N}-thread AND {SIMD-on/off} bit-identical on a forced-collision dense scene (C2); `solver_is_deterministic` + `static_body_unmoved_under_tgs` green across all phases; the IM-2b canonical warm-store proven.
- **0%-gate:** the default world (`AllPairs`, single-thread, SIMD-off, the byte-untouched `SoftStepSolver`) is byte-identical to today; every new path is opt-in.
- **Scale wins (criterion-measured, not asserted here):** broadphase shows a real measured crossover and a large reduction at 10k+; the colored+SIMD solve shows ≥1.8× (SIMD) atop the parallel scaling on a 10k pyramid; a 10k resting scene with sleeping costs ≤5% of an awake step.
- **IM-1 intact:** the `physics_apply` desync assert never fires under any feature (incl. sleeping — C1); warm-start un-thrashed on slept scenes.
- **Soundness:** no new `unsafe` outside the SIMD kernels, each `// SAFETY:`-documented with a differential scalar oracle; Miri clean on all scalar paths.
- **SDF integrity:** the scalar `sdf_edit_list` remains the SOLE CPU↔GPU oracle; x8 is a documented CPU-only accelerator never fed to a golden diff (W4); the rung 8/9/10/11 GPU goldens unchanged.

---

## Changelog vs critic review

**Criticals (all resolved):**
- **C1 (sleeping breaks IM-1):** Decision 5 / O8 re-specced — sleeping skips ONLY solve+integrate; `physics_gather` stays full so `BodyIndex`=archetype row and dense-row warm keys are intact. O8 now depends on O4-islands ONLY (the false dependency on entity-keyed warm-start / row indirection is dissolved). New gates: IM-1 desync assert clean + warm-start un-thrashed on slept scenes.
- **C2 (warm-store order determinism):** added **IM-2b** — `store_and_swap` MUST walk canonical `(manifold, point)` order independent of solve dispatch order (the linear-probed table is insertion-order-sensitive). The {1,N} gate now uses a FORCED-COLLISION dense scene to test it non-vacuously. The within-color order-independence claim is narrowed to velocity accumulators only.
- **C3 (seam/scalar-path contradiction):** added **Decision 7** — the colored/SIMD path is a SEPARATE `ColoredSoftStepSolver` impl; the shipped `SoftStepSolver`+`PointConstraint` are byte-untouched (true 0%-gate). Pool plumbing resolved: a NEW `physics_solve_colored` STAGE owns the pool and drives the per-color `scope` dispatch; `RigidSolver::solve` signature is unchanged.
- **C4 (grid==all-pairs unsatisfiable):** Decision 1 / O2 corrected — the grid emits candidates then applies the SAME sphere-bound feasibility predicate all-pairs uses; the gate is `post_filter(grid) == allpairs` bit-identical. The grid replaces the iteration, not the predicate.

**Importants (all resolved):**
- **W1 (no baseline to reset):** O5 ceremony re-scoped — `solver_is_deterministic` has no stored baseline (run-twice A==B); coloring keeps it green. The check is the tolerance-based acceptance gates + `static_body_unmoved_under_tgs` bit-identity; a stored golden guard is ADDED explicitly only if wanted. The "one-time baseline reset" language is removed.
- **W2 (nested-Vec alloc):** all data-dependent-count dimensions CSR-flattened to flat `Vec`+offsets; `color_occ` is a flat bitset matrix; the only nested Vec is the FIXED-worker-count `Box<[Vec<_>]>` grid scratch (cleared-not-dropped). Debug counting-allocator gate added.
- **W3 (gather piggyback):** the AABB/cell-size pass moved into the broadphase stage's OWN read (`physics_gather` untouched — 0%-gate preserved); the cell-size proxy is a closed-form `extent/n^(1/3)` (deterministic), NOT a sampling median.
- **W4 (SDF x8 vs GPU oracle):** explicit invariant added — `sdf_edit_list_x8` is a CPU-only accelerator, NEVER a golden-diff input; scalar `sdf_edit_list` is the SOLE CPU↔GPU oracle (doc-comment + review-checklist gated).
- **W5 (O1 targets dead code):** O1 re-scoped to the in-solver hot kernels (`refresh_inertia` per-substep + the solver's internal integrate loop); the standalone `physics_integrate` SIMD demoted to a low-priority bonus (dead under `SolverOwned`).
- **W6 (Changed<RigidBody> wake unsound):** rejected — the solver writes velocities back every frame, tripping `Changed` itself. Wake now keys off an explicit `wake()` API / `ExternalImpulse` marker / new-contact-with-awake signal only.

**Optionals folded in:** O1-optional (crossover stated as "measure, expect O(100s)," not 200; `AllPairs` stays default below a measured threshold); O2-optional (final pair-sort budgeted explicitly in the bench, candidate buffer capacity-reused); O3-optional (inert zero-`inv_mass` lane-tail padding verified against `effective_mass` k>0 + branchless `apply_impulse`, with an explicit NaN check on the friction/sqrt/restitution path on a zero/zero dummy lane).

**Open questions resolved:** OQ-4 (O6/O7 ownership) — strictly linear `O5→O6→O7`; O6 establishes `ContactColumns` SoA, O7 widens the same layout (no parallel-AoS interim).

**Preserved (critic-validated):** the determinism boundary (exact sqrt, no rsqrt/rcp/mul_add, width-only SIMD, pinned lane order); coloring-before-SIMD sequencing; the O5 value-change isolation; SAP/BVH/Jacobi rejections; ordered-merge-not-shared-atomics for the parallel grid build; O10 deferred/measured-gated; SDF CPU-authoritative + zero-readback brick-proxy.

---

Files relevant to executing this plan (all absolute):
- `D:\claude\BoykoEngine\crates\boyko_physics\src\systems.rs` (broadphase 188-212; gather 153-175 — UNCHANGED per W3/C1; integrate gate 116-118 — W5; the new `physics_build_graph` + `physics_solve_colored` stages)
- `D:\claude\BoykoEngine\crates\boyko_physics\src\resources.rs` (`BodyState`, `SolverScratch`, `PhysicsConfig`, `TouchedMask`; the 3 new resources)
- `D:\claude\BoykoEngine\crates\boyko_physics\src\solver\soft_step.rs` (`solve_velocities` 475-601, `refresh_inertia` 453-460 — O1 target, `store_and_swap` 430-448 — IM-2b/C2; the shipped solver stays byte-untouched except O1's additive `cfg` kernels)
- `D:\claude\BoykoEngine\crates\boyko_physics\src\solver\colored_soft_step.rs` (NEW — `ColoredSoftStepSolver` + `ContactColumns` + `solve_color`, D7)
- `D:\claude\BoykoEngine\crates\boyko_physics\src\solver\contact.rs` (`effective_mass` 119, `apply_impulse` 80-83 — O3-optional lane-tail inertness; a new SoA helper for the colored kernel)
- `D:\claude\BoykoEngine\crates\boyko_physics\src\solver\warm_start.rs` (probed open-addressing 247-291 — IM-2b store order; entity-keying for O10)
- `D:\claude\BoykoEngine\crates\boyko_physics\src\manifold.rs` (`BodyIndex`, `SDF_SENTINEL`, `Manifold`)
- `D:\claude\BoykoEngine\crates\boyko_physics\src\sdf_query.rs` + `D:\claude\BoykoEngine\crates\boyko_sdf_math\src\lib.rs` (SDF batched eval, O9 — scalar stays the SOLE oracle, W4)
- `D:\claude\BoykoEngine\crates\boyko_physics\src\plugin.rs` (`add_physics_parallel` + colored-stage wiring + `wake`, C3)
- `D:\claude\BoykoEngine\crates\boyko_threadpool\src\thread_pool.rs` + `scope.rs` (`scope`/`spawn` join = the deterministic parallel primitive; no shared counters)
- `D:\claude\BoykoEngine\crates\boyko_physics\tests\softstep.rs` (`solver_is_deterministic` 554-611 = run-twice no baseline — W1; `static_body_unmoved_under_tgs` 848 = bit-identity gate; `stacking_is_stable` tolerance gate)
- Destination: `D:\claude\BoykoEngine\docs\OPTIMIZATION-PLAN-PHYSICS.md`

Note on process: the mandatory `graphify` CLI could not be run (the Bash tool is disabled in this context, so the PreToolUse `graphify query` step was impossible). I grounded the four most contested critic points (C1 gather/IM-1, C2 warm-store order, C3 solve structure, C4 sphere-bound predicate, W5 integrate gate) via direct reads of `systems.rs` and `soft_step.rs` — the shipped code matches the critic's reads exactly, so every critical and important is valid and folded in.