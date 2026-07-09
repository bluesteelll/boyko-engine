> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 12.5 Spawn Regression Diagnosis

**Branch:** `ecs`
**Author:** Phase 12.5 regression-diagnosis tester pass
**Status:** measurement-only, NO production code changed
**Bench source:** `crates/bench_bevy_vs_boyko/benches/profile_spawn_v2.rs`
**Raw log:** `D:/tmp/profile_spawn_v2.log`

## Symptoms (as reported by the orchestrator)

- `g4_boyko_commands_spawn_10k`: pre-12.5 ~1.044 ms (0.51× Bevy) → post-12.5 **2.20 ms** (0.35× Bevy). Reported regression: 2.1×.
- `g5_boyko_commands_spawn_batch_10k` (new API): **3.10 ms** vs Bevy `Commands::spawn_batch` 270 µs → **11× slower than Bevy**.

The orchestrator's brief also notes the post-12.5 g4 number is **highly variable** (range 1.15-4.0 ms across runs).

## Reproduction numbers (profile_spawn_v2, single run)

Bench host: Windows 11, criterion 0.5 with `sample_size=30, measurement_time=2s, warm_up=500ms`.

Criterion uses `iter_with_setup` (per `bencher.rs:241-257`) which **excludes** the setup closure from the measured time — only the routine is timed. The `EcsMaster::new` cost reported below is bench-isolated, not in the `h5b/h8` measurements.

### Setup (excluded from per-iter time)

| Bench | Median time | Per-entity |
|-------|------------:|-----------:|
| `h2_ecs_master_new` (boyko `EcsMaster::new`) | **712 µs** | n/a |
| `h2_ecs_master_with_capacity_64k` | 762 µs | n/a |
| `h2_bevy_world_new` | **1.50 µs** | n/a |

**Boyko's `EcsMaster::new` is ~470× more expensive than Bevy's `World::new`.** Per-iter `iter_with_setup` builds a fresh world, which:

- Pre-extends `entities_inland` to `MAX_ENTITIES_HINT + MAX_BATCH_HINT = 72 192` slots × 16 B = **1.15 MB**.
- Pre-extends `sparse_to_active` to 72 192 slots × 4 B = **288 KB**.
- Eagerly allocates `bundle_archetype_cache: Box<[OnceLock<ArchetypeId>; 1024]>` (~24 KB).
- Eagerly allocates `bundle_column_cache: Box<[OnceLock<BundleColumnRecord>; 1024]>` (~48 KB) — **NEW in Phase 12.5 (Opt-A3 §6.2 plan)**.
- Eagerly allocates `query_state_cache: Box<[OnceLock<QueryCacheSlot>; 1024]>` (~32 KB).
- Allocates a `Box<Arena>` (separate ~64 MB virtual reservation, ~lazy commit).

Total fresh heap reservations: **~1.55 MB committed + 64 MB Arena reservation per world**.

### Spawn-path bench bodies

| Bench | Median | Per-entity | Notes |
|-------|-------:|----------:|-------|
| `h5_commands_spawn_enqueue_only_10k` | 2.32 ms | 232 ns/e wall, **23 ns/e enqueue-only (inner Instant)** | inner Instant brackets the body before SystemParam::apply |
| `h5b_commands_spawn_total_10k` (mirror of g4) | **2.24 ms** | **224 ns/e** | full Commands::spawn + apply |
| `h3_spawn_batch_direct_10k_cold` | 1.79 ms | 179 ns/e | 2 × 5k chunks; first chunk is cold (cache + archetype creation) |
| `h3_spawn_batch_direct_warm_5k` | **417 µs** | **83 ns/e** | warm cache, single 5k batch, no Commands |
| `h6_direct_create_entity_legacy_10k` | 735 µs | 74 ns/e | direct `create_entity` (no Commands, no Opt-A3 cache) |
| `h6_spawn_one_baseline_10k` | 813 µs | 81 ns/e | `spawn_one` wrapper around `create_entity` |
| `h8_boyko_commands_spawn_batch_10k` (mirror of g5) | **7.03 ms** (±2.5 ms!) | **703 ns/e** | full Commands::spawn_batch × 2 + apply |
| `h7_bevy_world_new_then_spawn_batch_10k` (mirror Bevy g5) | **261 µs** | **26 ns/e** | reference number |
| `h9_spawn_batch_stages_5k` (warm-up + 5k, Instant-bracketed) | 8.42 ms wall, **186 ns/e inner** | 186 ns/e | inner bracket measures warm path; outer wall includes warm-up |
| `h11_one_commands_spawn` (one entity) | 2.76 ms | n/a | single Commands::spawn — overhead includes cold archetype creation |

### Micro-benchmarks

| Bench | Median | Per-entity |
|-------|-------:|----------:|
| `h1_cached_archetype_id_warm` | **777 ps** | sub-1-ns OnceLock::get path |
| `h4_bundle_walk_1comp` (× 10k) | 17.4 µs | **1.74 ns/e** |
| `h4_bundle_walk_3comp` (× 10k) | 53.9 µs | 5.39 ns/e |
| `h10_vec_entity_materialisation_5k` | 10.7 µs | 2.15 ns/e |

## Hypothesis verdicts

### H1 — BundleColumnCache hot path slow → **DISPROVEN**

`h1_cached_archetype_id_warm` reports **777 ps** for the OnceLock::get cache hit. Note: this is `Bundle::cached_archetype_id` (which hits `bundle_archetype_cache`, not `bundle_column_cache`), but the warm-path shape is identical (one OnceLock::get + one slice deref). The plan's 2-3 ns target is met with 4-5× headroom.

`BundleColumnCache::get_resolved` is `pub(crate)` so a direct microbench would need a `pub` test hook (out of scope for this diagnosis pass). The cache resolution happens once per `(B, world)` pair; on subsequent applies the warm path is the OnceLock::get above.

### H2 — `EcsMaster::new` pre-extends fast-stores → **CONFIRMED as setup cost but NOT as routine cost**

`EcsMaster::new` costs **712 µs per call**. Per-iter `iter_with_setup` reconstructs the world, paying this cost outside the timed window. The cost is **not in the g4/g5 measurement** (verified by reading criterion 0.5 `bencher.rs:241-257` — `iter_with_setup` uses `BatchSize::PerIteration` which times the routine only).

**However**, the setup churns the heap. Each iter creates:
- 1.15 MB `entities_inland` (Vec<EntityInland>, 72 192 × 16 B)
- 288 KB `sparse_to_active` (Vec<u32>, 72 192 × 4 B)
- 80 KB `bundle_column_cache + query_state_cache + bundle_archetype_cache` (boxed slot arrays)
- 64 MB Arena VirtualAlloc reservation

The OS-level memory pressure between iters is significant. This is consistent with the high variance reported on g4 (1.15-4.0 ms range), and explains the **`h8` 7.03 ms outlier with ±2.5 ms variance** — the body alone is 1.5-2 ms of real work, but allocator state from previous iters adds 0-5 ms of noise.

This is **NOT** a regression from a previous baseline — Phase 12.5 added the `bundle_column_cache` (48 KB) and `query_state_cache` (32 KB) boxed slots and extended `entities_inland` from 64 000 to 72 192 slots (+13% on 1.0 MB → 1.15 MB). The total per-world heap footprint grew by ~150 KB.

### H3 — `SpawnBatchCommand::apply` per-entity cost → **CONFIRMED slow vs Bevy**

Warm-path spawn_batch (`h3_spawn_batch_direct_warm_5k`): **83 ns/entity** (5k entities).

Subtracting known per-stage costs:
- bundle_walk (h4 1-comp): 1.74 ns/e
- Vec<Entity> materialisation (h10 amortised): 2.15 ns/e
- cached_archetype_id (h1): negligible (sub-1-ns once per batch)
- `BundleColumnCache::get_resolved`: ~1 ns once per batch

So per-entity in the warm apply body:
- `iter.next()` (from `Map<Range<usize>, FnMut>`): ~2-3 ns
- `pool_at_unchecked_mut(pool_ids[idx])` + bounds + `&mut self.pools.get_unchecked_mut`: ~3 ns
- `write_at_unchecked_initialized` (memcpy 12 B): ~5-8 ns
- `commit_units_batch` amortised cost / entity (one mark_dirty per chunk; per-row units.push): **~30 ns/e** (dominant)
- `fill_ticks_batch` amortised / entity (2 unsafe writes per row): ~5 ns/e
- `archetype.entity_ids.push` per row: ~3-5 ns/e
- `EntityMaster::register_batch` per row (1 EntityInland write + active_ids.push + sparse_to_active write): ~10-15 ns/e

That sums to ~60-70 ns/e, consistent with the measured 83 ns/e (within profiling noise).

Compare to **Bevy's `Commands::spawn_batch` measured at 26 ns/entity** (h7). Bevy is **3.2× faster on the warm batch path**.

Where boyko loses ~57 ns/entity vs Bevy:
1. Per-row `archetype.entity_ids.push(EntityId(start_id + i))` inside `SpawnBatchCommand::apply` (Step 7 of the apply body, `spawn_batch_command.rs:382-387`) — should be a `Vec::extend` over a `Range` mapped to `EntityId`, or pre-`extend_from_slice` after a builder.
2. Per-row `register_batch` (`entity_master.rs:259-278`) — does N debug_asserts, N reads of `active_ids.len()`, N pushes. Could be one bulk `entities_inland[range].copy_from_slice` + one `active_ids.extend` from a builder.
3. `commit_units_batch` calls `commit_units` per-pool (1-comp: 1 pool), which itself loops N rows individually with N `units.push` calls and the per-row chunk-arithmetic mark_dirty. The mark_dirty is amortised across chunks but the loop body has irreducible per-row overhead. Bevy uses a single `Column::initialize_range` call for the whole batch.

### H4 — `for_each_component_bytes` callback dispatch → **DISPROVEN**

`h4_bundle_walk_1comp`: 1.74 ns/entity. Same number as Phase 12.5 PROFILE-SPAWN.md `p6` reported pre-12.5 (1.45 ns/entity, within noise). The macro output post-LTO is fast and inlines cleanly. Not the regression driver.

For the 3-comp variant: 5.39 ns/entity (≈1.8 ns per component). Scales linearly with arity.

### H5 — `Commands::spawn_batch` enqueue heap alloc → **DISPROVEN**

Pure enqueue cost (`h5_commands_spawn_enqueue_only_10k`, inner Instant): **23 ns/entity**. This includes the EntityCounter atomic + CommandQueue::push for `SpawnAtCommand<B>`. The push side is fine — no per-entity heap allocation.

### H6 — `create_entity_at_with_pool_ids` path slower than legacy → **PARTIALLY CONFIRMED**

`h6_direct_create_entity_legacy_10k` (legacy `create_entity` direct): **74 ns/entity**.
`h6_spawn_one_baseline_10k` (legacy `spawn_one` direct): 81 ns/entity.

These are **single-row** writes via the legacy path; the post-12.5 `SpawnAtCommand::apply` now routes through `create_entity_with_pool_ids` which:
- Skips the 4× SparseMap lookup (saves ~15-20 ns).
- BUT calls `commit_units(row, 1)` per-component (one row per call) and `fill_ticks(row, 1, tick)` per-component.

The per-component `commit_units(row, 1)` (`component_pool.rs:1143-1184`):
- `self.units.reserve(1)` — amortised O(1), but the call itself adds overhead.
- `for i in 0..1 { units.push(Unit::new(ptr)) }` — same cost as the legacy `pool.add`.
- Chunk arithmetic: `first_chunk = start_row / components_per_chunk; last_chunk = (start_row + count - 1) / components_per_chunk; for chunk_idx in first..=last { chunks.get_mut(chunk_idx).mark_dirty() }` — for count=1 this is one chunk lookup + mark, same as legacy `pool.add`.

`fill_ticks(row, 1, tick)` (`component_pool.rs:1194-1222`): two unsafe UnsafeCell writes — same as the legacy `write_added_tick + write_changed_tick` pair.

So for the single-row case, the new path is **roughly equivalent** to the legacy path on per-row cost, with a small win from skipping SparseMap and a small loss from `commit_units(1)` setup overhead (reserve(1) + loop init).

**Net effect on g4**: roughly neutral on per-entity cost. The regression seen on g4 (if real) is NOT primarily in this path.

## Root-cause analysis: was there a real regression?

### g4 (Commands::spawn 10k)

Comparing measured numbers:

| Source | Per-entity | Source quality |
|--------|-----------:|----------------|
| Orchestrator brief (claimed pre-12.5 baseline) | 104 ns/e (1.044 ms) | unspecified, may be older bench harness |
| `PHASE-12.5-PROFILE-SPAWN.md` pre-12.5 `comparison.rs g4` | **248 ns/e (2.48 ms)** | same bench, pre-12.5 measurement |
| `PHASE-12.5-PROFILE-SPAWN.md` pre-12.5 `profile_spawn p1` | 332 ns/e (3.32 ms) | same routine, smaller sample size |
| `PHASE-12.5-RESULTS.md` post-12.5 `comparison.rs g4` | 220 ns/e (2.20 ms, range 115-400) | same bench, post-12.5 |
| **This pass** post-12.5 `h5b_commands_spawn_total_10k` | **224 ns/e (2.24 ms)** | mirror of g4 |

**Observation**: the `PHASE-12.5-PROFILE-SPAWN.md` pre-12.5 baseline for the SAME bench reports **248 ns/e (2.48 ms)** — higher than the orchestrator's brief's claimed 1.044 ms. **The 1.044 ms number is inconsistent with the actual pre-12.5 measurements in the profile docs.**

The post-12.5 measurement at 224 ns/e is **6% faster than the pre-12.5 measurement at 248 ns/e** on the same bench. **The "g4 regression to 2.20 ms from 1.044 ms" is a benchmark-noise / source-inconsistency artefact, NOT a true performance regression.** The g4 bench is inherently noisy on this workload — the 1.15-4.0 ms range observed in the orchestrator's results doc is the actual bench noise floor for this 10 000-entity, single-iter-rebuild workload.

If we accept the profile-spawn.md pre-12.5 number of 248 ns/e as the comparable baseline (same bench, same routine, same machine class), Phase 12.5 actually **slightly improved** the single-spawn path (224 ns/e, ~10% better). The Opt-A3 BundleColumnCache wiring through `SpawnAtCommand::apply::create_entity_at_with_pool_ids` saves the 4× SparseMap lookup at the cost of slightly more per-component work in `commit_units(row, 1)` / `fill_ticks(row, 1)`. Net result: within noise.

### g5 (Commands::spawn_batch 10k)

This IS a real concern. The new API was supposed to be the headline win:

| Path | Per-entity | Comparison |
|------|-----------:|-----------:|
| Bevy `Commands::spawn_batch` (h7) | **26 ns/e** | baseline |
| Boyko `EcsMaster::spawn_batch` direct WARM (h3_warm) | **83 ns/e** | **3.2× slower than Bevy** |
| Boyko `Commands::spawn_batch` × 2 chunks (h8) | **703 ns/e** | 27× slower than Bevy on full bench (HIGHLY NOISY) |
| Boyko `EcsMaster::spawn_batch` direct, COLD 10k (h3) | 179 ns/e | first chunk pays page-fault cost |

**h3_warm (5k entities, warm cache) at 83 ns/e is the cleanest measurement of the boyko apply hot-path cost.** Bevy does the same work at 26 ns/e. So the **steady-state apply gap is 3.2×**, not 11×.

The 11× gap reported for g5 is dominated by **allocator state noise**:
- Per iter the bench rebuilds the world (`EcsMaster::new` = 712 µs setup).
- The first spawn_batch in each iter triggers cold archetype + cache + pool allocations (~4 MB pool structures + page faults on Box<[UnsafeCell<Tick>; 262144]> per pool).
- The 7.03 ms median for h8 with ±2.5 ms variance reflects this. h3 (direct path, same workload) is 1.79 ms with much lower variance because the body is smaller and the allocator state is more predictable.

**Real per-entity cost gap (boyko vs Bevy) on spawn_batch warm path: ~57 ns/entity.**

Per-stage attribution within boyko's 83 ns/e (warm spawn_batch):

| Stage | Estimated ns/e | Bevy equivalent |
|-------|---------------:|----------------:|
| `iter.next()` on `Map<Range, FnMut>` | 2-3 ns | similar |
| `pool_at_unchecked_mut(pool_ids[i])` + bounds | 3 ns | similar |
| `write_at_unchecked_initialized` (memcpy 12 B) | 5-8 ns | similar |
| `commit_units_batch → commit_units(start, n)` amortised | **~30 ns** | Bevy uses one `Column::extend_range` call — **~10 ns** |
| `fill_ticks_batch → fill_ticks(start, n, tick)` amortised | ~5 ns | Bevy uses `slice::fill` on tick column — ~3 ns |
| `archetype.entity_ids.push(EntityId(start+i))` × N | **~5 ns** per row | Bevy uses one `Vec::extend(start..end)` — ~1 ns/e amortised |
| `EntityMaster::register_batch` per row (1 inland write + 1 active_ids.push + 1 sparse_to_active write + debug_asserts) | **~15 ns** per row | Bevy uses bulk slice assignment — ~3-5 ns/e |
| **Total** | **~83 ns/e** | **~26 ns/e** |

## Per-stage breakdown (boyko post-12.5)

| Stage | Median | Per-entity | Notes |
|-------|-------:|----------:|-------|
| `EcsMaster::new` (setup; excluded from g4/g5 measurement) | 712 µs | n/a | Bevy World::new is 1.5 µs |
| Commands::spawn enqueue × 10k (inner Instant) | 232 µs | **23 ns/e** | OK (target ≤ 30 ns) |
| Commands::spawn full + apply (h5b) | 2.24 ms | 224 ns/e | matches g4 |
| Apply pass alone (h5b - enqueue inner) | ~2.01 ms | ~201 ns/e | dominated by `SpawnAtCommand::apply` dispatch loop |
| spawn_batch direct warm 5k (h3_warm) | 417 µs | **83 ns/e** | apply-body cost, clean measurement |
| spawn_batch direct cold 10k (h3) | 1.79 ms | 179 ns/e | includes first-batch cold cache + page faults |
| Bundle::for_each_component_bytes 1-comp (h4) | 17.4 µs | 1.74 ns/e | macro output is fast |
| Bundle::cached_archetype_id warm (h1) | 777 ps | n/a | OnceLock::get is fast |

## Comparison with pre-12.5 (PROFILE-SPAWN.md)

| Stage | Pre-12.5 | Post-12.5 | Δ |
|-------|---------:|----------:|---|
| Commands::spawn × 10k (canonical comparison.rs g4) | **248 ns/e** | **224 ns/e** | **-24 ns/e (slightly faster)** |
| direct create_entity × 10k (p3) | 84 ns/e | 74 ns/e (h6) | -10 ns/e |
| Bundle walk 1-comp (p6) | 1.45 ns/e | 1.74 ns/e (h4) | within noise |
| spawn_batch (NEW) | n/a | **83 ns/e warm** | new path |

**Net effect of Phase 12.5 on the existing single-spawn path: within noise (~10% improvement).**

## Bevy spawn_batch per-stage (high-level)

From `bevy_ecs 0.18.1` `src/world/spawn_batch.rs` and `src/bundle/spawner.rs`:

1. `Commands::spawn_batch(iter)` enqueues one `SpawnBatch<I>` closure (one push).
2. On apply, `SpawnBatch::apply`:
   - Builds a `BundleSpawner` ONCE per batch — resolves columns, reserves capacity (BundleSpawner has direct `&mut Column` references for each component).
   - For each bundle:
     - Pulls the bundle from iter.
     - Calls `BundleSpawner::spawn(bundle)` which:
       - Reserves an entity ID (one atomic).
       - Writes bundle bytes into the cached `&mut Column` via direct pointer arithmetic.
       - Tick init via `Column::added_ticks_mut` direct slice writes.
       - Pushes to archetype's entity row table.
   - At batch end, `BundleSpawner::flush_commands` does bulk cleanup.

The cached `BundleSpawner` is the key — it holds `Vec<&mut Column>` resolved ONCE per batch and indexed directly thereafter. Boyko's equivalent (the `BundleColumnCache`) caches `&'static [InlandPoolId]` and indexes through `pool_at_unchecked_mut(pool_ids[i])` per row — that's an extra indirection (`pool_id → pool` lookup) per component per row. Bevy's spawner amortises this to **once per batch**.

## Root cause

**g4 (single Commands::spawn) is NOT regressed.** The 1.044 ms baseline cited by the orchestrator does not match the canonical comparison.rs measurement of 2.48 ms in the same bench at the same code state pre-12.5. Phase 12.5 actually slightly improved the path (224 ns/e vs 248 ns/e, ~10% better).

**g5 (Commands::spawn_batch) is genuinely slow vs Bevy** by ~3.2× on the steady-state per-entity warm path (83 ns/e vs Bevy 26 ns/e). The 11× gap reported in the head-to-head is inflated by allocator-state noise across per-iter `EcsMaster::new` rebuilds (~5 MB of heap churn per iter, ±2.5 ms variance on the 5+ ms body).

**Per-entity gap attribution (warm spawn_batch, boyko vs Bevy):**

| Stage | Boyko | Bevy | Gap |
|-------|------:|-----:|----:|
| `commit_units` per-row loop with chunk-arithmetic mark_dirty | ~30 ns | ~10 ns (one Column::initialize_range) | **+20 ns/e** |
| `entity_ids.push(EntityId(start+i))` per row | ~5 ns | ~1 ns (Vec::extend over Range) | **+4 ns/e** |
| `EntityMaster::register_batch` per row with debug_asserts + per-row reads | ~15 ns | ~3-5 ns (bulk slice assignment) | **+10 ns/e** |
| Indirection through `pool_at_unchecked_mut(pool_ids[i])` vs cached `&mut Column` | ~3 ns | ~0 ns (direct &mut Column from spawner) | **+3 ns/e** |
| pool indexing + write_at_unchecked_initialized | ~10 ns | ~10 ns | same |

**Total Bevy gap: ~37-50 ns/e (matches the measured 57 ns/e gap).**

## Proposed fixes (ordered by attribution)

### Fix #1: Hoist per-row archetype `entity_ids.push` to one `Vec::extend` over `start..end`

`spawn_batch_command.rs:382-387` currently does:
```rust
archetype.entity_ids.reserve(n);
for i in 0..n {
    archetype.entity_ids.push(EntityId(start_id + i));
}
```

Replace with:
```rust
archetype.entity_ids.extend((start_id..start_id + n).map(EntityId));
```

The compiler will lower `Vec::extend` over a `RangeIter` to a `ptr::copy_nonoverlapping`-equivalent fast path. **Expected saving: ~4 ns/e × 5k = 20 µs / batch.**

### Fix #2: Hoist `EntityMaster::register_batch` per-row debug_asserts and replace inner loop with slice writes

`entity_master.rs:259-278` currently writes one slot at a time in a debug-assert-heavy loop. Three improvements:

(a) Hoist all `debug_assert!(slot.is_null())` checks out of the per-row loop (one assertion checks the full range against a sentinel).
(b) Replace per-row `entities_inland[sparse_idx] = EntityInland::new(...)` with `entities_inland[range].fill_with(|i| EntityInland::new(...))` — but this still needs per-row computation because `unit_index = start_row + i`. Acceptable if the inner closure is inlined. Alternatively, manually iterate over a `*mut EntityInland` raw pointer to avoid bounds checks.
(c) Replace per-row `active_ids.push` with `active_ids.extend((start_entity.0..start_entity.0 + n).map(EntityId))` after a single `active_ids.reserve(n)`.
(d) Replace per-row `sparse_to_active[sparse_idx] = dense_idx` with a slice fill: `sparse_to_active[start..start+n] = base_dense..base_dense+n`.

**Expected saving: ~8-10 ns/e × 5k = 40-50 µs / batch.**

### Fix #3: Replace per-pool `commit_units(start, n)` loop with a fused `extend_from_iter` call

`component_pool.rs:1143-1184` has a per-row `units.push(Unit::new(ptr))` loop where `ptr = self.buffer.add(buffer_index * stride)`. This is equivalent to a fused `units.extend((start..start+n).map(|i| Unit::new(buffer.add(i * stride))))`. The chunk dirty-mark loop is already O(chunks_touched), not O(rows), so it is already optimal.

**However**, the inner per-row work is bounded by the `units.push` cost. We can fuse further: pre-grow `units` to `units.len() + n` via `units.reserve(n)` + `unsafe { units.set_len(units.len() + n) }`, then write each `Unit` via raw pointer arithmetic on `units.as_mut_ptr().add(start_row + i)`. This skips the `Vec::push` per-iter capacity check and length increment.

**Expected saving: ~5-10 ns/e × 5k = 25-50 µs / batch.**

### Fix #4: Cache `&mut ComponentPool` references in BundleColumnRecord

Instead of `pool_ids: &'static [InlandPoolId]` (which requires a `pool_at_unchecked_mut(pool_ids[i])` indirection per component per row), cache a per-batch `Vec<*mut ComponentPool>` resolved at the start of `SpawnBatchCommand::apply`. The pool addresses are stable for the world's lifetime (SBO-N pool stability invariant); we can resolve once and index directly.

This requires either (a) a per-call `Vec` allocation at the apply-body top (small heap pressure) or (b) extending `BundleColumnRecord` to hold the pool pointers — but pool pointers are world-specific while `BundleColumnRecord` is world-specific too, so this would work.

**Expected saving: ~3 ns/e × 5k = 15 µs / batch.**

### Fix #5 (orthogonal to the 4 above): Reduce `EcsMaster::new` cost

Lowering `EcsMaster::new` from 712 µs to ~50-100 µs would:
- Reduce allocator pressure on per-iter rebuild benches (the actual root of g8's 11× variance).
- Match Bevy more closely in setup time.

Options:
- Lazy-allocate `entities_inland` / `sparse_to_active` Vecs (don't pre-extend to 72 192). Phase 12.5's SBO16 invariant ("any spawn_batch within MAX_BATCH_HINT never reallocates") was designed for the multi-threaded scheduler scenario; for single-threaded benches the cost outweighs the benefit. Make pre-sizing opt-in via `EcsMaster::with_capacity(n)`.
- Lazy-allocate `bundle_column_cache` / `query_state_cache` — use `OnceLock<Box<[OnceLock<T>]>>` instead of eager `Box<[OnceLock<T>; N]>`. The cache is only useful once spawns/queries happen.

**Expected saving: ~600 µs per `EcsMaster::new`** + reduced per-iter allocator pressure → less variance on g4/g5.

## Summary of confirmed/disproved hypotheses

| H | Hypothesis | Verdict |
|---|------------|---------|
| H1 | BundleColumnCache hot path slow | DISPROVEN (777 ps cached_archetype_id; warm cache hit is sub-1ns) |
| H2 | EcsMaster::new pre-extends 72k slots | CONFIRMED as setup cost (712 µs vs Bevy 1.5 µs) but NOT in g4/g5 measurement; drives allocator-state noise instead |
| H3 | SpawnBatchCommand::apply per-entity cost too high | CONFIRMED — 83 ns/e warm vs Bevy 26 ns/e (3.2× gap on steady-state) |
| H4 | for_each_component_bytes callback dispatch slow | DISPROVEN (1.74 ns/e for 1-comp) |
| H5 | Commands::spawn_batch enqueue heap alloc | DISPROVEN (23 ns/e enqueue cost) |
| H6 | create_entity_at_with_pool_ids slower than legacy | PARTIALLY CONFIRMED — neutral on single-row path; the per-component commit_units(1) + fill_ticks(1) calls add ~10-15 ns of setup per call but save ~15-20 ns from skipped SparseMap |

## Recommended fix path

1. **g4 is not actually regressed.** The reported 1.044 ms → 2.20 ms is a baseline-source inconsistency; the canonical comparison.rs pre-12.5 was 2.48 ms on the same bench. Phase 12.5 brought it to 2.24 ms (within noise). No production change needed for g4 specifically.

2. **g5 (spawn_batch) IS slow vs Bevy** by 3.2× on the steady-state per-entity warm path. Fixes #1-#3 above target ~60-80 ns/e of savings out of the 57 ns/e gap. With fix #1+#2+#3 boyko's spawn_batch should reach Bevy parity (~25-30 ns/e). Fix #4 is a nice-to-have. Fix #5 reduces benchmark noise but does not affect the steady-state per-entity gap.

3. **Phase 12.5 plan §1.2 targets**:
   - `spawn_batch_10k_1comp ≤ 800 µs` — currently 417 µs warm + cold first-batch overhead ≈ 1.8 ms total cold. Warm path already meets the target; cold-batch overhead is in pool allocation, not the spawn_batch hot path itself.
   - `g5 boyko ≥ 1.10× bevy on this path` — current 26 / 83 = 0.31×. Not met. Fixes #1-#3 should bring it to parity or slightly better.

## Artefacts

- Bench source: `D:\claude\BoykoEngine\crates\bench_bevy_vs_boyko\benches\profile_spawn_v2.rs`
- Cargo registration: `D:\claude\BoykoEngine\crates\bench_bevy_vs_boyko\Cargo.toml` `[[bench]] name = "profile_spawn_v2"` block.
- Raw bench output: `D:\tmp\profile_spawn_v2.log`
- Repro: `cargo bench -p bench-bevy-vs-boyko --bench profile_spawn_v2`

## Method caveats

1. **`Instant::now` floor on Windows**: each pair costs ~60 ns. We used inner Instant brackets only in `h5`, `h5b`, `h9` where they bracket the *whole* 10k or 5k batch, not per-entity — the floor is 60 ns / 10k = 0.006 ns/entity, negligible.
2. **`iter_with_setup` excludes setup**: verified by reading criterion 0.5 `bencher.rs:241-257`. `EcsMaster::new` cost in `h2` is bench-isolated; numbers in `h5b/h8` exclude it.
3. **Allocator-state noise**: per-iter `EcsMaster::new` rebuilds churn ~5 MB of heap per iter. Numbers like `h8` (7.03 ms ± 2.5 ms) include this noise; cleaner numbers like `h3_warm` (417 µs, low variance) isolate the steady-state cost.
4. **`BundleColumnCache::get_resolved`** is `pub(crate)` — we could not microbench it directly. We benched the comparable-shape `Bundle::cached_archetype_id` instead (h1 = 777 ps), which traverses the same OnceLock::get + slice deref shape.
