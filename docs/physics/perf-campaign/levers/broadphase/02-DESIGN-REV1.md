# Architecture: Tree broadphase (`BroadphaseKind::Tree`) for `boyko_physics`

Read-only design. Tree `D:/wt/joltab` @ `e2bcbcb5`. Line numbers refer to that tree. "arith." means computed from code or counters, not measured.

## Goal

- **Replace the serial O(n²) AllPairs broadphase** with a broadphase that:
  - emits **exactly AllPairs' pair set, in exactly its `(min, max)` order**;
  - never queries static or sleeping bodies for pairs;
  - runs its per-body work in parallel;
  - keeps all durable state in ECS `ScratchColumn`s;
  - allocates nothing on the hot path.
- **Targets (arith.; the bench gate G4 checks them before any end-to-end run):**

| row | today (measured) | target |
|---|---|---|
| J broadphase at W=1 | 2.100 ms | 0.16–0.24 ms |
| J broadphase at W=8 | 1.945 ms | 0.07–0.13 ms |
| all-asleep floor, broadphase share (with C5) | 1.93 ms | ≤ 0.03 ms |
| 10k bodies at W=1 | 70.2 ms AllPairs, 8.0 / 15.8 ms Grid | 1.9–2.4 ms |

## Context and constraints

- **The pair count feeds values.** `BoxAxisCache::begin_frame(pairs.len())` sizes the hint table as `next_pow2(2·P)` and clears it when it grows (`axis_cache.rs:261-281`). A tighter pair set would change clear timing, then the hints, then the pose bytes. **So only the exact AllPairs set is bit-identical.**
- **The predicate** (`systems.rs:319-321`) is `delta.length_squared() <= (r_i + r_j)²`, where `r = body_bounding_radius` (`:1218-1223`). `Vec3::dot` evaluates `x*x + y*y + z*z` left to right (`boyko_math/src/vec.rs:177-178`).
- **J scene** (`jolt_parity_pyramid.rs:667-697`):
  - the floor is spawned first, so it is **row 0**: static, r = √5001 ≈ 70.7;
  - 1,240 boxes follow, half-extent 1.0, r = √3;
  - so all 1,240 slab pairs have min row 0.
  - Structural count: 4,270 in-layer + 4,060 inter-layer + 1,240 slab = 9,570 pairs (arith.), against 9,561 measured.
- **Rows are unstable.** `RemapCursor` / `RowRemap` classify every gather as `Identity`, `Rows` or `Reset` (`row_identity.rs:114-239`).
- **The sleep hint:** `IslandSleep::is_row_awake(row)` (`resources.rs:3242`) is the mask from step t−1. `begin_step` can unlatch a row later in step t (`:3480-3568`).
- **Kinematic poses** are only moved by the user (`resources.rs:3714-3717`).
- **Scene sync** can move statics every step (`plugin.rs:590-603`).
- **KE16 is an ancestor.** P0b measured E(8) = 0.681 from a system body, so the nested-scope serialisation defect no longer applies.
- **Allocation census S1c pins** `scope (133,133)` and `chunk (229,229)`, and asserts `(scope−1) % passes == 0` (`alloc_frame_census.rs:1800-1822, 2317-2325`).

## Key decisions

### D1. Emit exactly AllPairs' set: a conservative cull, then the same f32 predicate
- **What:**
  - Cull bounds use `|r|` plus the slack ε = (‖p‖∞ + |r|)·2⁻²⁰ + 2⁻¹²⁶. That is ≥ 16u against a proven need of ~4.01u, covering the roundings in `fl(p−r)` and `fl(r_a+r_b)`.
  - The leaf test **is** the AllPairs expression, on bit copies of `position` and of `r = body_bounding_radius(b)`.
- **Why:**
  - The axis-cache coupling above.
  - The predicate is symmetric bitwise: (−d)² = d², and f32 addition is commutative.
  - So the lane result equals the scalar result whichever side queries.
- **Rejected:**
  - *AABB predicate.* Changes values. It saves only about 1,015 early-out box-box SATs per step (≤ ~0.1 ms serial narrowphase, ≤ 0.02 ms after L5).
  - *`Collider.layer`/`mask` filtering.* A semantic change; AllPairs ignores both fields.
- **Trade-off:** the ~1,015 non-touching slab pairs still reach the narrowphase.

### D2. Implicit packed 8-wide BVH
- **What:**
  - Leaves are Morton-sorted: 30-bit codes over the set's centre bounds, key `(morton << 32) | item_index`.
  - Groups of 8 consecutive leaves form one leaf node; every 8 nodes form a parent. Children are implicit: node k at level L owns 8k..8k+7 at level L−1.
  - All levels live in **one column**.
- **Why:**
  - Handles size disparity; the Grid's measured 2.5× loss is on a scene with size disparity.
  - Prunes on all three axes, which matters in piles.
  - 8 lanes match AVX2.
  - No pointers, O(n) radix build.
  - One query kernel serves all three sets.
  - At J the tree is 179 nodes × 192 B ≈ 34 KB, so it stays in L1/L2.
- **Rejected:**
  - *SAP / box pruning.* On this pile it yields 150–190k 1D candidates (Terdiman's stack case). Its output is not owned per row, so every pair needs a global sort. Its bipartite pass against statics walks every static.
  - *Uniform or hashed grid.* Cell size cannot fit mixed sizes; that is the Grid's failure.
  - *Incremental pointer BVH (Box2D/Jolt).* Tree quality depends on insertion order, and it needs refit/rebuild machinery. Jolt's incremental prepare costs **66 µs/step on this pyramid (measured)**, against our from-scratch build at 15–25 µs (arith.).
- **Trade-off:**
  - The active tree is rebuilt every step, O(n_active).
  - Morton groups are loose for sparse clusters. That only costs speed: the output is independent of tree shape.

### D3. Three sets by change rate; persistent pairs cached; every persistent row bit-verified each step
- **What:**
  - **Active set:** built every step.
  - **Static set:** `inv_mass == 0 && !kinematic`; rebuilt only when it changes.
  - **Sleeper set (C5):** rows frozen in the last solve; rebuilt on a removal, and batched on additions.
  - Pairs among non-active rows are cached as sorted lists: **SS** (static–static), and **SL** (frozen–frozen plus frozen–static).
  - Each step, every persistent row's `(x, y, z, r)` bits and class are compared with the record taken at rebuild. Any mismatch, any class change, or `RowRemap ≠ Identity` rebuilds that set. A static rebuild forces a sleeper rebuild, because SL holds frozen–static pairs.
- **Why:**
  - Statics and sleepers are never queried.
  - Correctness does not depend on the hint: a wrong hint costs a rebuild, never a pair.
- **Rejected:**
  - *One tree for everything.* A 100k-static level would be re-sorted every step.
  - *Trusting the latch.* Its timing makes it a hint only.
  - *ECS change ticks.* `physics_apply` rewrites every `RigidBody` each step, so every tick fires.
  - *Remapping persistent rows on `Rows`.* Rebuild instead: it is cold, and U5's stable slots remove the need.
- **Trade-off:**
  - An O(N) verify pass, ~1–3 ns per row.
  - A static moved every step (for example by scene sync) costs a static rebuild every step. Such bodies should be marked `Kinematic`.

### D4. Re-find active pairs every step (Jolt's model), with no persistent pair set
- **Why:**
  - A persistent set (Box2D/Rapier/PhysX) needs pair identity across row changes, which does not exist before U5.
  - Its gain on a resting awake pile is ≈ 0.05–0.1 ms at W=8 (arith.), below R's SE bar of 1.3 %. It could never be claimed.

### D5. Row-owned output in lexicographic order, with no global sort
- **What:**
  - Each active row gets a **128 B line-aligned block**: its forward partners (> row, sorted), plus "rev" entries for non-active partners with a lower row.
  - A serial pass counts per row, prefix-sums over rows, and buckets rev entries by their min row. Rev entries arrive in increasing active-row order, so each bucket is already sorted.
  - A copy pass writes each row's segment. A non-active row's segment is a 3-way merge of SS, SL and its rev bucket.
- **Rejected:**
  - *Global radix sort of pair keys:* 2–3 serial passes over 9.5k–77k keys.
  - *Two-pass count-then-emit (the Grid's shape):* doubles the dominant query cost.
  - *Per-chunk buffers:* capacity is unknown, so they need an overflow protocol anyway, plus atomics for rev entries.
- **Trade-off:**
  - N × 128 B of address space; one line written per active row per step.
  - More than 31 partners in a row goes to a cold serial spill.

### D6. Parallelism: one `pool.scope` wave for queries; a copy wave only above a threshold
- **What:**
  - Chunks are contiguous Morton leaf ranges: min(W·4, n_active / 32) of them.
  - Classify, build, prefix and rev scatter stay serial.
- **Why:** Morton ranges keep each worker's tree nodes hot, and the leaf→row map is a bijection, so no two writers share a block.
- **Rejected:**
  - *Fusing broadphase with narrowphase (Jolt's interleave):* breaks the pair-order contract to save ≤ 1 wave (ω(8) = 6.54 µs, measured).
  - *Parallel radix sort and prefix now:* inert at 1,241 rows, and no gated scene reaches 8k (trigger in the open questions).

### D7. Defaults and policy
- `Tree` becomes the default kind.
- Below `TREE_BRUTE_MAX_ROWS` (calibrated by G4) it runs the shared exact AllPairs loop.
- Auto's high side is retargeted from Grid to Tree.
- Grid stays for the SP2 coupling (forced there) and for Manual use.
- The `parallel_broadphase` default flips to `true`.

### D8. Tree safety
- A row is tree-safe when x, y, z and r are finite and ‖p‖∞ + |r| ≤ 2⁶⁰. That keeps every intermediate finite: bound² ≤ 2¹²², |d|² < 2¹²⁶.
- **Negative radius:** bounds use `|r|`, which stays conservative because |r_a + r_b| ≤ |r_a| + |r_b|.
- Rows with a NaN position or radius are excluded, which is exact: the predicate is always false for them.
- Any other non-safe row sends the step to a **cold AllPairs fallback**, which is exact, and bumps `fallbacks`.

## Data structures

```rust
/// 8 children (internal) or 8 leaves (level 0), SoA: one AVX2 register per plane.
#[repr(C, align(32))]                   // column base is 64-aligned ⇒ every node line-aligned (192 = 3·64)
pub(crate) struct Node8 { p: [[f32; 8]; 6] }  // 192 B
// internal: min_x,min_y,min_z,max_x,max_y,max_z — empty lane +inf/−inf (never overlaps)
// leaf:     x,y,z,r,row_bits,_ — empty lane x=+inf (len²=inf fails), row=u32::MAX (fails `row > a` as i32 −1)

pub(crate) struct PackedBvh8 {
    nodes: ScratchColumn<Node8>,       // level 0 (leaf groups) first, then 1.. up to the root
    level_start: [u32; 13],            // 8^12 > 2^31 leaves
    levels: u8,
    leaves: u32,
}

/// Per-ROW output block; one writer per step (the chunk owning the row's leaf).
#[repr(C, align(32))]                   // 128 B = 2 lines, line-aligned via column base
struct RowBlock { fwd: u16, rev: u16, partners: [u32; 31] }
// fwd == SPILLED(0xFFFF) ⇒ partners[0..3] = (aux start, fwd len, rev len)

/// Per-ROW record: persistent sphere bits and class (verify) + per-step counts.
#[repr(C, align(32))]
struct RowRec { x: f32, y: f32, z: f32, r: f32, class: u8, _p: [u8; 3], cnt: u32, off: u32, rev: u32 } // 32 B

/// Active item in row order (build input; also the scratch for cold rebuilds).
#[repr(C, align(32))]
struct Item { x: f32, y: f32, z: f32, r: f32, row: u32, _p: [u32; 3] }  // 32 B, never straddles a line

#[derive(Resource)]
pub struct BroadphaseTree {
    active: PackedBvh8, statics: PackedBvh8, sleepers: PackedBvh8,   // 3 columns
    sort_a: ScratchColumn<u64>, sort_b: ScratchColumn<u64>,          // radix ping-pong
    items: ScratchColumn<Item>,
    blocks: ScratchColumn<RowBlock>,
    rows: ScratchColumn<RowRec>,
    ss_pairs: ScratchColumn<(BodyIndex, BodyIndex)>,                 // sorted, persistent
    sl_pairs: ScratchColumn<(BodyIndex, BodyIndex)>,                 // sorted, persistent (C5)
    aux: ScratchColumn<u32>,                                         // [spill | rev buckets], per step
    cursor: RemapCursor,                                             // 16 B
    pending_sleepers: u32, pending_steps: u32,
    diag: TreeDiag,                                                  // rebuilds, spills, fallbacks: u64 each
}
```

- **Columns:** 11 `ScratchColumn`s, forming a new cohort `BROADPHASE_TREE` of 11 ids.
- **Stagger constraints:** the cohort width must be ≤ 64. Its stagger slots must also avoid `SCRATCH_ID_BODY_STATE` (co-swept by classify) and `SCRATCH_ID_CONTACT_PAIRS` (co-swept by assembly).
- **Traversal scratch is function-local:**
  - stack `[u32; 96]` (the bound is 7·levels + 1);
  - partner buffer `[u32; 32]`;
  - radix histogram `[u32; 1024]`.

## Public API

```rust
pub enum BroadphaseKind { AllPairs, Grid, Tree }            // Tree = default (C4)
pub struct BroadphaseTree;                                 // Resource
impl BroadphaseTree {
    pub fn with_capacity(rows: usize) -> Self;
    pub fn static_rebuilds(&self) -> u64; pub fn sleeper_rebuilds(&self) -> u64;
    pub fn spills(&self) -> u64; pub fn fallbacks(&self) -> u64;   // non-vacuity for gates
}
/// The exact AllPairs predicate, the single definition every path calls (Grid's `feasible` delegates).
#[inline] pub fn sphere_bound_feasible(pa: Vec3, ra: f32, pb: Vec3, rb: f32) -> bool;
/// The exact double loop (the Tree's small-n path and the gates' oracle).
pub fn all_pairs_into(bodies: &[BodyState], out: &mut ContactPairs);
pub fn physics_broadphase(scratch: Res<SolverScratch>, cfg: Res<PhysicsConfig>,
    grid: ResMut<BroadphaseGrid>, tree: ResMut<BroadphaseTree>,
    sleep: Option<Res<IslandSleep>>,   // C5; KE4 optional params (boyko_ecs/tests/ke4_optional_resource_params.rs)
    pairs: ResMut<ContactPairs>);
```

## Algorithms for the critical paths (per step, `Tree` arm)

| # | step | complexity | cache and branching | SIMD |
|---|---|---|---|---|
| 0 | **Cursor.** `cursor.remap(rows)`. If not `Identity`, mark both persistent sets dirty and disable the sleep hint for this step. | O(1) | — | — |
| 1 | **Classify and verify.** One pass over the N rows: compute r, choose the class, compare persistent rows' bits with `RowRec`, write `RowRec`, set the dirty/pending flags. | O(N) | Streams `BodyState` (2 lines per row) and writes `RowRec`. One predictable branch per row. | scalar (the sqrt is shared with AllPairs' own function) |
| 2 | **Rebuild static and sleeper sets if dirty**, or when pending sleepers ≥ max(64, n_sl/8) or have waited ≥ 8 steps. `#[cold] #[inline(never)]`: gather members, build, self-query; SS and SL are then sorted in place with `sort_unstable`. | O(n_s log n_s) on change only | cold, out of line | query kernel |
| 3 | **Active build.** Collect the ACTIVE rows of `RowRec` into `items` (row order); reduce min/max bounds; 30-bit Morton keys; 3-pass LSD radix on bits 32..61 (stable, so ties fall back to row order); gather leaves; build levels bottom-up with exact `min`/`max`. | O(n_a) | Sequential, except the leaf gather, which reads from `items` (40 KB, in L1). | bounds, levels |
| 4 | **Query** each active leaf a: traverse the active tree keeping `row > a`, then the static tree and the sleeper tree with no row filter; apply the exact predicate; partners > a are forward, < a are rev. Insertion-sort (≤ 31), then write `blocks[a]`. More than 31 sets `SPILLED`. | O(n_a · (log n + k)) | Tree in L1/L2. At J: ~10–17 8-wide tests per query. Branches: one lane-mask loop (tzcnt). | overlap: 6 compares + and. Exact test: `t=dx*dx; t=t+dy*dy; t=t+dz*dz; hit = t <= (r+q_r)*(r+q_r)` with `LE_OQ`; mul+add only, no FMA. |
| 5 | **Serial pass A:** resolve spills (cold re-query into `aux`); count rev entries per min row into `RowRec.rev`. | O(n_a + R) | Header reads, row order | — |
| 6 | **Serial pass B:** per row, `cnt = fwd` (active) or `ss + sl + rev` (non-active; walked with SS/SL cursors); exclusive prefix into `off`; P = total. Rev scatter into the `aux` CSR in active-row order, so buckets arrive sorted. | O(N + P_nn + R) | streaming | — |
| 7 | **Copy:** `out.resize(P)`. Active row: its block → `(row, p)` pairs. Non-active row: 3-way merge. | O(P) | streaming | — |

- **Output:** `ContactPairs` is the lexicographic exact set.
- **Checks kept:** `systems.rs:344-348` (the sorted-order `debug_assert` and the pair counter) stay as they are.
- **J at W=1 (arith.), by phase:**

| phase | cost |
|---|---|
| classify | 2.5–5 µs |
| build | 15–25 µs |
| query | 1,240 × 100–150 ns = 124–186 µs |
| serial A/B and copy | 16–27 µs |
| **total** | **0.16–0.24 ms** |

## Multithreading model

- **Model:** a single system. The query wave (and the copy wave above `PAR_COPY_MIN_PAIRS`) goes through `try_with_active_pool` and `pool.scope`, exactly as the Grid does (`resources.rs:1761-1771`). It runs serial when there is 1 lane, when `n_a < PAR_QUERY_MIN`, or when `parallel_broadphase` is off.
- **Shared, read-only during a wave:** the three trees, `items`, `RowRec`, `ss_pairs`, `sl_pairs`, `aux`.
- **Written in the query wave:** `blocks[row]`, only for rows of the chunk's own leaves.
  - The leaf→row map is injective (`debug_assert`: `items` rows strictly increasing).
  - So the write sets are disjoint.
  - A block is exactly 2 whole lines, so there is no false sharing.
- **Written in the copy wave:** `out[off[r0]..off[r1])` per row-range chunk. The ranges are disjoint because `off` is monotone.
  - At most one line per chunk boundary is shared between two writers. That is ≤ 32 lines per step; accepted.
- **No atomics, no locks.**
- **Synchronisation:** the scope's `Drop` join is the only point. Serial passes 5 and 6 run after it.
- **Pointers:**
  - Workers receive bases from `ScratchColumn::solve_base()` (provenance-preserving), inside a `Copy` pointer struct with `unsafe impl Send + Sync` (the `EmitPtrs` pattern, `resources.rs:1809-1816`).
  - No `build_view` is taken while bases are live; a view is taken only after the join (the Grid's Tree Borrows discipline, `:1913-1920`).
- **Why this is race-free:** each byte written in a wave has one owning chunk, fixed before the wave (the leaf ranges, or the `off` ranges). Every other location a worker touches is immutable for the whole scope.

## Determinism argument

1. The cull is conservative (D1): every pair in S = {(i, j) : i < j, pred} reaches a leaf test.
2. Every leaf test is the AllPairs f32 expression, on the same bits, symmetric bitwise. So exactly S passes.
3. Ownership emits each pair exactly once:

| pair kind | found by |
|---|---|
| active–active | the query of the smaller row (`row > a` filter) |
| active–non-active | the active row's query; stored as forward or rev by comparing rows |
| non-active–non-active | the persistent SS/SL lists, valid because every member's bits are verified equal to those at rebuild |

4. Placement uses only integer counts, a prefix over rows, and per-row sorted lists.

**Therefore the output is a function of S alone.** It is independent of:
- W, chunking and scheduling;
- tree shape and Morton quantisation;
- rebuild history, the hint, and AVX2 against the scalar kernel.

It is bit-identical to AllPairs on any machine. That covers replay determinism, and the axis cache sees the same `P`.

## Expected gain (from P0b; arith. unless marked)

- **Cost model:** t_bp(W) = c_cls·N + c_build·n_a + c_q·n_a/(W·E) + ω·[W>1] + c_asm·(N+P).

| parameter | value | basis |
|---|---|---|
| N / n_a / P | 1,241 / 1,240 / 9,561 | measured |
| E | 0.681 | the solve's E(8), measured; used as a proxy here |
| ω | 6.54 µs | ω(8), measured |
| c_cls | 2–4 ns | arith. |
| c_build | 12–20 ns | arith. |
| c_q | 100–150 ns | arith.; checked by G4 |
| c_asm | 1.5–2.5 ns | arith. |

- **Gain per row.** Before-values: broadphase measured at W=1/8 (2.100 / 1.945); T is disarmed cfg-A.

| row, W | T before (measured) | bp after | Δ | share of T | SE bar (K=6) |
|---|---|---|---|---|---|
| J, 1 | 19.671 | 0.16–0.24 | 1.86–1.94 | 9.5–9.9 % | 2.0 % |
| J, 2 | 13.711 | ≈0.10–0.16 | 1.85–1.95 | 13.5–14.2 % | 1.4 % |
| J, 4 | 10.701 | ≈0.08–0.14 | 1.85–1.95 | 17.3–18.2 % | 1.2 % |
| J, 8 | 9.162 | 0.07–0.13 | 1.82–1.88 | 19.9–20.5 % | 0.75 % |
| J, 16 | 9.172 | = W=8 | 1.82–1.88 | ~20 % | 0.5 % |
| R, 1 | 14.226 (bp 2.08) | 0.16–0.24 | 1.84–1.92 | 12.9–13.5 % | 3.3 % |
| R, 8 | 9.933 | 0.07–0.13 | ~1.8–1.9 | 18–19 % | 1.3 % |

- **Sleeping floor (C5):**
  - Broadphase becomes classify plus the SS/SL copy ≈ 0.02–0.03 ms, against 1.93 ms measured.
  - J-Son tail: 5.334 → ≈3.43 ms (−36 %).
  - R-S floor: 6.049 → ≈4.15 ms (−31 %).
  - It is bit-identical to today's sleeping-on runs.
- **Projection (not a claim):**
  - ANALYSIS §9 gives cfg-A + simd + L5 = 5.5–5.7 ms at W=8.
  - Minus Δ(8), that is 3.6–3.9 ms, i.e. 1.03–1.10× Jolt v5.3.0 (3.533).
- **10k bodies (bench, uniform), assuming ~7.7 pairs per body (P ≈ 77k):**
  - c_q is 150–200 ns there, because the tree lives in L2.
  - W=1 ≈ 1.9–2.4 ms, against AllPairs 70.2 ms and Grid 8.0 ms (uniform) / 15.8 ms (size disparity), all measured.
  - W=8 ≈ 0.5–0.8 ms, of which the serial build and prefix are ≈ 0.2–0.4 ms.
- **100k bodies:** ≈ 25 ms at W=1 and ≈ 6 ms at W=8 (c_q ≈ 250 ns, tree in L3).

## Integration (every site)

**New files:**
- `D:/wt/joltab/crates/boyko_physics/src/broadphase_tree/mod.rs`: the resource; classify, rebuilds, assembly, waves.
- `D:/wt/joltab/crates/boyko_physics/src/broadphase_tree/bvh.rs`: `PackedBvh8` build and query.
- `D:/wt/joltab/crates/boyko_physics/src/broadphase_tree/kernel.rs`: the 8-lane overlap and exact tests. The AVX2 path is `cfg(target_feature="avx2")`; the scalar fallback also serves Miri.
- `D:/wt/joltab/crates/boyko_physics/tests/broadphase_tree.rs`: gates G0, G1 and G3.
- `D:/wt/joltab/crates/boyko_physics/tests/broadphase_tree_scenes.rs`: gate G2; `cfg(not(miri))`, release-sized like `default_world_pyramid_determinism.rs`.

**Edits:**

| file:line | change |
|---|---|
| `src/lib.rs:66` | add `pub mod broadphase_tree;` |
| `src/systems.rs:277-349` | Doc, signature and a `Tree` arm. The AllPairs arm `:309-327` stays verbatim as the oracle. `:344-348` unchanged. |
| `src/resources.rs:45-53` | add the `Tree` variant |
| `src/resources.rs:149-154`, `:225-249` | docs |
| `src/resources.rs:458` | default kind → `Tree` (C4) |
| `src/resources.rs:493` | `parallel_broadphase: true` (C4) |
| `src/resources.rs:1486-1493` | Grid's `feasible` delegates to `sphere_bound_feasible` (same expression) |
| `src/broadphase_policy.rs:68-90` | `GRID_LO`/`GRID_HI` → `AUTO_TREE_LO`/`AUTO_TREE_HI`, set from G4 |
| `src/broadphase_policy.rs:190-192` | Auto's high side → `Tree` |
| `src/broadphase_policy.rs:197-219` | tests |
| `src/plugin.rs:513-517` | The coupling path also sets `broadphase_select: Manual`, so Auto can never override its forced Grid. |
| `src/plugin.rs:526` | insert `BroadphaseTree::with_capacity(INITIAL_BODY_CAPACITY)` |
| `src/scratch_ids.rs` after `:396` | The `BROADPHASE_TREE` cohort (11 ids) below the row-identity cohort; width and stagger `const` asserts; register function. The floor assert `:731-739` moves to the new cohort's bottom. |
| `src/profiling.rs:99`, `:130` | zones `phys_bp_classify`, `phys_bp_build`, `phys_bp_query`, `phys_bp_assemble`; counters `phys_bp_active`, `phys_bp_rebuilds`, `phys_bp_spills` (tier Deep, never inside a chunk task, per ruling O1) |
| `benches/broadphase.rs` (at `:205`) | Tree arms |
| `benches/jolt_parity_pyramid.rs:723-749` | `--broadphase allpairs\|tree\|grid` override (sets Manual plus the kind) after `configure`; doc `:53-55` |
| `docs/SYSTEMS.md`, `docs/FEATURE_MAP.md` | entries |

**Untouched:**
- `narrowphase/axis_cache.rs`: its input `P` is identical.
- `solver/*`
- `soft/coupling.rs`: it still reads Grid.

## Commit sequence (each green)

- **C1 `feat(physics): tree broadphase, serial`**
  - The module, resource, `Tree` arm (not the default), static set, spill, fallback, brute path.
  - The scratch cohort and the predicate extraction.
  - Gates G0, G1 and G2 land and are green.
- **C2 `perf(physics): tree broadphase parallel query and copy`**
  - The waves behind `parallel_broadphase`; zones and counters.
  - G3; the Tree case in `profiling_zone_counts.rs`.
- **C3 `bench(physics): tree arms and the runner's --broadphase flag`**
  - No behaviour change.
  - **Quiet window (the owner confirms first):** G4, then G5 on C3's binary.
- **C4 `perf(physics): tree broadphase by default`**
  - Constants written from G4.
  - Defaults flip: kind `Tree`, `parallel_broadphase` on. Auto retargeted; the coupling path pinned to Manual.
  - Moved tests re-measured in the same commit (below).
  - G5 results recorded under `docs/measurements/<date>-broadphase-tree/`.
- **C5 `perf(physics): sleeper set`**
  - The sleeper tree, SL list and hint (`Option<Res<IslandSleep>>`).
  - Hint mutations added to G1; the R-S / J-Son oracle added to G2.
  - The G5 floor A/B.

## Gates (each shown able to fail)

- **G0 — kernel equals scalar.** A proptest compares the lane booleans with `sphere_bound_feasible` over:
  - exact-boundary constructions (|d| = r_a + r_b ± k ulp on a random axis);
  - coordinates up to ±1e6, subnormals, ±0, negative radii.
  - **Mutation M0:** the dz term via `_mm256_fmadd_ps` must go red.
- **G1 — pair set equals AllPairs, on every step.**
  - Single-step proptest over random worlds: n ≤ 300, sizes 1e−3..1e3, 0–30 % static, 0–10 % kinematic, duplicate positions, NaN rows, boundary pairs, and at least one body with ≥ 40 partners.
  - Multi-step scripts: teleport or reshape a static; toggle `Kinematic`; spawn and despawn (row remaps); a **random** sleep hint every step.
  - Assertion: `Vec` equality with `all_pairs_into` on the same snapshot, every step.
  - Named mutations, each required red:

| mutation | change |
|---|---|
| M1 | bounds use r·(1−2⁻²⁰) |
| M2 | bounds use `r`, not `|r|` |
| M3 | rev entries dropped |
| M4 | verify skipped |
| M5 | no rebuild on remap |
| M6 | hint trusted without verification |
| M7 | entries past 31 dropped |

- **G2 — scene oracle, every step.**
  - Scenes: J for 600 steps, sleeping off; R-S for 600 steps, sleeping on.
  - Tree equals `all_pairs_into` on each step's snapshot.
  - **Non-vacuity:** `static_rebuilds ≥ 1`; `sleeper_rebuilds ≥ 1` on R-S; `spills == 0`; `fallbacks == 0`; P within 9,5xx on J.
  - M3 goes red at step 0 of J (the 1,240 slab pairs).
- **G3 — order is independent of W.**
  - Parallel output equals serial output, bitwise, for W ∈ {2, 3, 4, 8} and chunk counts {1, 2, 7, 64}, asserting the wave really dispatched ≥ 2 chunks.
  - Pose-byte hashes: Tree at W ∈ {1, 2, 4, 8, 16} equals serial AllPairs, over J 600 steps and R-S 600 steps (runner `--expect-pose`).
  - **Mutation M8:** emitting rows in chunk-completion order must go red at W ≥ 2.
- **G4 — bench (build-if).**
  - Tree / AllPairs / Grid at n ∈ {17, 64, 128, 256, 1k, J snapshot, 10k, 100k} × {uniform, size disparity} × W ∈ {1, 8}.
  - **Stop and investigate if either fails:**
    - J snapshot at W=1 must be ≤ 0.30 ms (predicted 0.16–0.24);
    - Tree must be ≤ Grid at every point.
  - Outputs: `TREE_BRUTE_MAX_ROWS`, `AUTO_TREE_LO`/`HI`, `PAR_QUERY_MIN` and `PAR_COPY_MIN_PAIRS` (starting values 96 / 64 / 128 / 256 / 32,768).
- **G5 — end to end, under the P0 protocol.**
  - Same binary, `--broadphase allpairs` against `tree`.
  - Rows: J (`--cfg default`) and R, W ∈ {1, 2, 4, 8, 16}, K=6. Each process runs at W; order interleaved and reversed on alternate passes; receipts gate each process; the canary J-C must be seen.
  - Claim if |effect| > 2·√(SE_A² + SE_B²).
  - Armed runs at W=1 and W=8 give the broadphase span and I(W). The **realized Δbp(8) must be ≥ 0.6 × 1.82 ms**.
  - S16: no claimed regression (the brute path).
  - Pose hash equal.
  - For C5: J-Son tail [264, 1000) and R-S at W=1 and W=8, with the floor claimed and the pose hash equal to the pre-C5 sleeping-on hash.

## What moves (all bit-identical, so no value pins move)

**`alloc_frame_census.rs`:**
- `:1800-1822`: the structural claim becomes `(scope − 1 − BP_WAVES) % passes == 0`. BP_WAVES = 1 at J with parallel on. Must be red under the mutation "broadphase opens 2 waves".
- `:2317-2325` (release S1c): re-pinned from the 4,352-step long run, as the pin's own comment prescribes, with zero upward headroom. Expected: scope 134, chunks 229 + (Tree chunks at W=4); dispatch max re-measured.
- `:2334-2342` (debug S1c, 385 bodies): unchanged if 385 < `PAR_QUERY_MIN`; otherwise re-pinned the same way.
- `:2510-2518`: the prose saying `parallel_broadphase` is inert.

**Other files:**
- `alloc_frame_attribution.rs:99-100`, `:2055-2077`: the D2 "+0.000 / inert" statement is re-measured on the same pile.
- `broadphase_select_p3.rs`, `broadphase_policy.rs` tests: Auto's high side is now Tree, with new constants.
- `profiling_zone_counts.rs`: exact counts for the new zones, 1 per step each; the rebuild counter is structural. `phys_bp_pairs` is unchanged.
- `default_world_pyramid_determinism.rs:6-8`, `:26-29`: docs only (the default is no longer AllPairs).
- `soft_body_sp2.rs`: must stay green with no edit; this checks the coupling Manual pin.

## What it cannot claim

- **Nothing in the narrowphase, graph or solve.** The pair set is unchanged, including the ~1,015 non-touching slab pairs.
- **Statics and sleepers are still touched once per step** by the O(N) classify/verify pass (~1–3 ns per row), on top of the gather's own O(N). The sleeping floor keeps np 2.92, graph 0.12 and solve 0.30 ms; those belong to L10.
- **10k and 100k figures are bench-only.** No gated end-to-end scene is that large. The serial build and prefix there (~40–50 % of the W=8 broadphase, arith.) are not addressed.
- **Grid's 2.5× loss:** the cause (the slab inflating its cells) is not measured, and Grid is not fixed.
- **W=16 gains equal W=8's.**
- **Jolt parity is a projection.**
- **The cfg-A headline** pins AllPairs by definition; new rows must name the broadphase.
- **A static moved every step** costs a rebuild every step; no gated scene covers it.

## Open questions

1. **Scratch-id headroom.** My estimate is about 13 ids between the row-identity cohort's bottom and `MAX − 128`, and the cohort needs 11.
   - If the floor assert fires at C1, `SCRATCH_REGION_MIN_ID` moves to `MAX − 144`, after re-running the census in that constant's docs (142 production ids against 368 remaining slots).
   - A C1 unit test also asserts the column bases are 64-aligned.
2. **L10 seam.** Dropping frozen-only (SL) pairs from `ContactPairs` needs L10 to size the axis cache as `P + |SL|` and to carry those manifolds. Otherwise `P` changes values. The SL list is exposed `pub(crate)` for that.
3. **Deferred parallel build and prefix.** Build them when G4 shows serial build plus prefix > 20 % of the broadphase at W=8, at 10k. A ≥ 8k-body end-to-end scene would then be needed to gate it.