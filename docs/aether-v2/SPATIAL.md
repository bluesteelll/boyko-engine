# `boyko_spatial` — the spatial index

Unblocks the N-to-N query class the machine design deliberately refuses (R-Q): enemy-senses-enemy,
flocking, nearest-of-N-towers, AoE target selection. Adjudicated from two competing designs;
rationale → [`DECISIONS.md`](DECISIONS.md) §P1–P8.

## Architecture

A new crate `boyko_spatial`: an **unbounded spatial hash** (bucket = hash of the integer cell
coordinate; memory O(N), a teleport costs one hash, no world bounds, no AABB pass), fixed cell size
derived from a compile-time census of the radii the program actually uses, CSR counting-sort with
**payload permutation** (the 16-byte `{x, y, z, layer}` items are permuted, not indices — the query
walk reads contiguous memory). All storage on `ScratchColumn` — **zero kernel changes in Phase 1**.

`boyko_physics::BroadphaseGrid` is NOT reused (no entities in it, physics-only population, pair-set
output, own internal Vecs) and NOT replaced — different contract (AABBs, substep cadence, pair
enumeration). The convergence path is shared **code**, not a shared instance: the cell-hash + CSR +
key-range-scatter building blocks are designed for three consumers (gameplay now; the physics
broadphase migrating onto the same substrate later; coarse world-streaming cells later). Render
culling is excluded on purpose — this engine's cull is GPU-resident (host-extracted frustum planes
pushed to the cull shader; HZB occlusion on GPU), so a CPU hash accelerates nothing there.

Inputs are table components: `SpatialLayer(u32)` (presence = membership; layer mask), plus the
published word `SpatialUser(u32)` (team / hp bucket / leaf — so a query hit carries enough to
filter WITHOUT touching foreign columns). Never dense (the chunked driver const-rejects dense
terms; a const-assert pins it).

## Phases

| Phase | Contents |
|---|---|
| **1** | crate + components + serial build (fused gather+key via `for_each_chunk_entities` → histogram → prefix sum → stable scatter) + `within_radius` / `k_nearest<K≤16>` / `nearest` / `within_aabb` + the linear fallback for a fat query + determinism gates + a `zero_alloc` row + benches. **Mandatory: own-cell acceptance** — two walked cells can hash to one bucket, so a member could be yielded twice (double damage, nondeterministically rare); accept a member only while iterating its own cell, and pin it with a forced-collision test |
| **2** | `along_segment` (DDA); `SpatialRadius` + a coarse 8× level for extended members; retire the demo grid in `boyko_demo`, port the boids |
| **3** | parallel build: `par_for_each_chunk_entities` (does not exist yet — R4/K-item) + key-range ownership (each worker owns a bucket range; zero atomics; **byte-identical to the serial build by construction**), gated by `build(1) == build(W)` before shipping |
| **4** | Aether surface: `near` — blocked on the per-entity machine landing (R5); the grid and its `Res` surface ship earlier and are usable from plain systems immediately |

## Consumption and refusals

From a machine guard or any system: a plain `res<SpatialGrid>` read — no new scheduler concept, no
`joins` extension (the pattern is already shipped twice: the physics solver reads its grid
resource; the boids demo needed no core change). The hit carries position + layer + the `user`
word.

Refusals that keep the O(N) fake unwritable: R-Q already bans `query<…>` in guards — its diagnostic
gains the cost statement ("O(N) per row is O(N²) per frame; at 10k that is 10^8 pair tests") and
the three blessed forms (`near` / `res<>` / an event). Additional: mutation of the grid resource
from a guard; a non-literal radius without `radius_hint`; a radius sweeping more than
`max_sweep_buckets` buckets against the census cell size (compile-time refusal).

## Envelope

Comfortable at 10–30k probing entities; 100k probers need `every N` throttling (the wall is query
fan-out, not the build — both competing designs converged on this). Not provided: true raycast
against surfaces (that is narrowphase / SDF / hwrt territory), reading foreign components (R-Q
stands), cross-frame handles into the index, extended geometry in v1, replacement of the physics
broadphase, and sparse species under ~50 members (a flat `each` wins there and stays blessed).
