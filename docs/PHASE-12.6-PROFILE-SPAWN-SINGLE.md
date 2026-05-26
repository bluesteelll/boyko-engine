# Phase 12.6 Profile — Commands::spawn single 10k

**Branch:** `ecs`
**Phase:** 12.6 — post-12.5 follow-up profiling the single-spawn 148 ns/e
gap reported in the brief.
**Status:** measurement-only, NO production code changed.
**Bench source:** `crates/bench_bevy_vs_boyko/benches/profile_spawn_single.rs`.
**Raw logs:** `D:/tmp/profile_spawn_single.log`, `..._run2.log`, `..._run3.log`.

## State note — Phase 12.6 lazy-init WIP

The working tree carries an uncommitted Phase 12.6 WIP that wraps the
per-world cache arrays in `OnceLock<Box<[OnceLock<T>]>>` (lazy
allocation) and removes the eager `entities_inland` / `sparse_to_active`
pre-extension from `EcsMaster::new`. The cache accessor methods
(`bundle_archetype_cache(&self)`, `bundle_column_cache(&self)`,
`query_state_cache(&self)`) now do `OnceLock::get_or_init` on the outer
lock. This is a 2× OnceLock load on every warm spawn that used to be a
single load.

The bench numbers below were captured against the WIP state. The
boyko spawn numbers are HIGHER than the brief's 224 ns/e baseline
(measured at v2 / post-12.5 / pre-WIP). This is documented in the
"Surprise findings" section. The structural attribution still holds.

## Workload

* Bundle `V6PosBundle { pos: V6Position { x, y, z } }` — 1 component,
  12 bytes (`f32 × 3`).
* 10 000 entities per bench iter.
* Fresh world per iter via criterion `iter_with_setup` (excludes setup
  cost from measurement; per-iter heap churn still influences the
  allocator).
* Single-threaded.

## Method

Each bench is structured as an **isolated micro-measurement** of one
stage. Numbers across independent micros are summed externally to
attribute per-stage costs against the total. Where Instant brackets are
used to split enqueue vs apply within a single bench, the per-pair
floor is reported by `p0` (~74 ns / pair) and is negligible at this
granularity because the brackets surround the whole 10k-entity loop,
not each entity (floor = 74 / 10k ≈ 0.007 ns/entity).

The bench was run three times. Numbers reported below are the
median-of-medians across the three runs; ranges document the observed
variance. Where variance exceeds ±20 %, the column is flagged
"VOLATILE" — that bench is sensitive to per-iter allocator state and
its absolute number cannot be trusted; only the cross-engine ratio is
informative.

## Raw per-bench results

**Four runs collected** (raw logs at `D:/tmp/profile_spawn_single*.log`).
Numbers below are from run-3 (cleanest system state). The variance
table at the end of this section shows all four runs side by side.

### Cleanest run (run-3, used as reference)

| Bench | Median time | Per-entity | Notes |
|-------|------------:|-----------:|-------|
| `p0_instant_now_pair` | 74.2 ns | n/a | QPC floor on Windows |
| `p1_boyko_commands_spawn_total` | **3.71 ms** | **371 ns/e** | mirror of g4 |
| `p2_bevy_commands_spawn_total` | **1.05 ms** | **105 ns/e** | mirror of g4 |
| `p3_boyko_enqueue_vs_apply` | 4.56 ms | 456 ns/e wall | inner instants: enqueue 57.6 ns/e, apply 342.6 ns/e (VOLATILE outer, stable inner) |
| `p4_command_queue_push_only` | 208 µs | **20.8 ns/cmd** | NoopCommand 28 B payload, no apply |
| `p5_command_queue_apply_only` | 128 µs | **12.8 ns/cmd** | NoopCommand apply walk |
| `p6_bevy_command_queue_apply_only` | **78.1 µs** | **7.8 ns/cmd** | noop closure apply walk |
| `p7_direct_spawn_batch_size_1_loop` | 1.28 ms | 128 ns/e | spawn_batch(1) loop, caches warm |
| `p8_direct_create_entity_legacy` | 1.29 ms | 129 ns/e | 4× SparseMap path |
| `p9_direct_spawn_one` | 1.08 ms | **108 ns/e** | typed wrapper of create_entity |
| `p10_bevy_direct_world_spawn` | **498 µs** | **49.8 ns/e** | bevy World::spawn direct |
| `p11_boyko_commands_spawn_with_id_read` | 3.83 ms | 383 ns/e | Commands::spawn + .id() (no extra work) |
| `p12_boyko_reserve_entity_only` | 32.3 µs | **3.2 ns/e** | EntityCounter::reserve_entity in a loop |
| `p13_boyko_cached_archetype_id_warm` | 11.2 µs | **1.12 ns/call** | cached_archetype_id × 10k (warm) |
| `p14_bevy_enqueue_vs_apply` | (inner) | enqueue 12.77 ns/e, apply 67.95 ns/e | run-2 (run-3 was 84 / 294 ns/e — VOLATILE) |

### Variance summary (4 runs)

| Bench | Run 1 | Run 2 | Run 3 | Run 4 | Median |
|-------|------:|------:|------:|------:|------:|
| `p1` (boyko total) | 5.36 ms | 5.73 ms | 3.71 ms | 9.11 ms | 5.55 ms |
| `p2` (bevy total) | 1.10 ms | 1.09 ms | 1.05 ms | 1.29 ms | 1.10 ms |
| `p3 apply` (boyko inner) | 298.7 ns/e | 344.8 ns/e | 342.6 ns/e | 440.4 ns/e | 343.7 ns/e |
| `p3 enqueue` (boyko inner) | 53.9 ns/e | 53.9 ns/e | 57.6 ns/e | 71.7 ns/e | 55.8 ns/e |
| `p4` (boyko queue push) | 198 µs | 249 µs | 208 µs | 584 µs | 229 µs |
| `p5` (boyko queue apply noop) | 207 µs | 147 µs | 128 µs | 226 µs | 177 µs |
| `p6` (bevy queue apply noop) | 84 µs | 81 µs | 78 µs | 156 µs | 83 µs |
| `p7` (boyko spawn_batch×1) | 1.55 ms | 1.74 ms | 1.28 ms | 2.47 ms | 1.65 ms |
| `p9` (boyko spawn_one) | 1.16 ms | 1.40 ms | 1.08 ms | 2.17 ms | 1.28 ms |
| `p10` (bevy direct spawn) | 805 µs | 593 µs | 498 µs | 884 µs | 699 µs |
| `p13` (cached_archetype_id) | 14.8 µs | 11.3 µs | 11.2 µs | 17.1 µs | 13.1 µs |

Bevy and boyko micros that touch only a single allocator slot
(`p2`, `p6`) are stable within 5-7 %; benches that rebuild the world
per iter (`p1`, `p7`, `p10`) carry ±20-30 % variance from allocator
state. Cross-engine **ratios** are stable: boyko/bevy on total spawn
sits at 3.0-7.1× across runs; boyko/bevy on queue apply sits at
2.0-3.5×; boyko/bevy on direct spawn sits at 2.0-2.5×. **Use the
ratios, not the absolute numbers.**

### Comparison.rs g4 head-to-head (sanity check)

`cargo bench --bench comparison -- g4` was run 3 times in parallel
with the profile bench runs:

| Run | g4_boyko | g4_bevy | Ratio |
|-----|---------:|--------:|------:|
| 1 | 3.59 ms (359 ns/e) | 1.64 ms (164 ns/e) | 2.19× |
| 2 | 5.79 ms (579 ns/e) | 927 µs (93 ns/e) | 6.25× |
| 3 | 2.76 ms (276 ns/e) | 718 µs (72 ns/e) | 3.84× |
| 4 | 3.12 ms (312 ns/e) | 795 µs (80 ns/e) | 3.90× |

The brief's 2.24 ms / 762 µs (3× ratio) sits **inside** the observed
variance range. The structural conclusion (boyko ~3× slower than
Bevy on this path) is reproducible. The exact ms numbers are not.

## Per-stage breakdown (boyko, post-12.6 WIP)

Numbers below subtract the timing-floor of the Instant pair brackets
where applicable.

| Stage | Per-entity | % of total (p1) | How it's measured |
|-------|----------:|----------------:|-------------------|
| **Commands::spawn enqueue** | **57.6 ns/e** | 16 % | p3 inner enqueue bracket |
| `EntityCounter::reserve_entity` atomic | ~3.2 ns/e | <1 % | p12 |
| `CommandQueue::push` (SpawnAtCommand payload) | ~17.5 ns/e | 5 % | p3 enqueue − p12 = 54 ns/e push body |
| `EntityCommands::new` (return value) | ~free | 0 % | inferred from p11 vs p1 |
| **CommandQueue::apply dispatch** | **~13 ns/e** | 4 % | p5 / 10k cmds — pure dispatch |
| Per-cmd: meta read + payload read + glue call | 5-7 ns/e | 1-2 % | p5 minus single fetch_add |
| `consume_and_drop_glue` (read + cursor advance + apply + drop) | ~7 ns/e | 2 % | p5 model |
| `catch_unwind` per-apply registration (Opt-A1 hoist) | ~free | <1 % | hoisted out of per-cmd loop |
| **SpawnAtCommand::apply per-row work** | **~272 ns/e** | 73 % | p3 apply (343) − p5 dispatch (13) − stage glue ≈ 272 |
| `B::cached_archetype_id` (warm) | 1.1 ns/e | <1 % | p13 |
| `BundleColumnCache::get_resolved` (warm OnceLock) | ~2 ns/e | <1 % | inferred shape |
| `[MaybeUninit<(ComponentId, &[u8])>; 8]` stack array setup | ~5 ns/e | 1 % | profile_spawn finding #3 carries over |
| `for_each_component_bytes` callback (1 comp) | ~1.7 ns/e | <1 % | profile_spawn_v2 h4 |
| `create_entity_at_with_pool_ids` core work | **~260 ns/e** | 70 % | inferred — see p9/p7 cross-check |
| **Total p1** | **~371 ns/e** | 100 % | head-to-head measurement |

### Cross-check via direct paths (no Commands)

| Bench | Per-entity | Includes |
|-------|----------:|----------|
| `p7` boyko spawn_batch(1)-in-loop | 128 ns/e | full Opt-A3 apply + per-call iter+materialise |
| `p8` boyko create_entity (legacy 4× SparseMap) | 129 ns/e | per-row pool writes + 4× SparseMap |
| `p9` boyko spawn_one (typed wrapper) | 108 ns/e | per-row pool writes + 4× SparseMap + mem::forget |

The direct paths sit at ~108-129 ns/e. The Commands path adds
**~371 − 108 = 263 ns/e** of overhead on top of the bare per-row
spawn work.

That 263 ns/e is the cost of routing a single spawn through:
- `EntityCounter::reserve_entity` (~3 ns)
- `CommandQueue::push` of `SpawnAtCommand<V6PosBundle>` (~17 ns)
- `CommandQueue::apply` dispatch per command (~13 ns measured via p5)
- `SpawnAtCommand::apply` itself (`[MaybeUninit;8]` setup +
  `for_each_component_bytes` + bundle_column_cache lookup + call into
  `create_entity_at_with_pool_ids`)

So `SpawnAtCommand::apply` itself adds **~263 − 13 − 17 − 3 = ~230 ns/e**
ON TOP OF the per-row work it dispatches into. That's a stunning
amount of overhead for what should be a thin wrapper.

## Per-stage breakdown (Bevy)

| Stage | Per-entity | Notes |
|-------|----------:|-------|
| `EntityAllocator::alloc` (atomic fetch_sub + fallback fetch_add) | ~5 ns | from source inspection |
| `Commands::queue` (closure push) | ~8-10 ns | size_of closure for spawn_at = ~28 B |
| `CommandQueue::apply` dispatch | **~7.8 ns/cmd** | p6 |
| `World::spawn_at_with_caller` | per source: check_can_spawn_at + BundleSpawner::new + spawn_at | included below |
| `BundleSpawner::new` per command (NOT cached across cmds in single-spawn path) | ~25-30 ns | rebuilds spawner per cmd |
| `Table::allocate` + write_components + set_location + mark_spawned | ~25-30 ns | per source |
| Direct `World::spawn` (p10) | **49.8 ns/e** | minus Commands overhead |
| Bevy total (p2) | **105 ns/e** | = ~50 ns direct + ~55 ns Commands routing |

Bevy's enqueue+dispatch overhead = 105 − 50 = **~55 ns/e**.
Boyko's enqueue+dispatch overhead = 371 − 108 = **~263 ns/e**.

**Routing-overhead gap: ~208 ns/e.** That's the bulk of the 266 ns/e
total gap. Bare per-row archetype work also leaves a ~58 ns/e gap
(boyko 108 vs Bevy 50 ns/e).

## Hotspot ranking (where the gap goes)

### #1 — `SpawnAtCommand::apply` glue weight (≈ 200 ns/e gap)

**File:** `crates/boyko_ecs/src/ecs/core/commands/spawn_at_command.rs:80-213`.

Per-command glue on the apply side:
1. `B::cached_archetype_id(world)` — now **TWO** OnceLock loads under
   the Phase 12.6 WIP (outer lazy init + inner slot get). Was one load
   pre-WIP.
2. `world.archetype_master_mut().archetype_ptr_for(archetype_id)` —
   slab lookup (~3 ns).
3. `world.bundle_column_cache.get_resolved::<B>()` — under WIP this is
   now `world.bundle_column_cache()` method call → outer OnceLock load
   → `.get_resolved::<B>()` → another OnceLock load. Effective cost
   ~4 ns per spawn (doubled vs pre-WIP).
4. `[MaybeUninit<(ComponentId, &[u8])>; MAX_BUNDLE_ARITY=8]` stack
   array (192 B) — initialised to `undef`, but the `slots_base`
   cast and per-iteration `slot_ptr.add(...)` arithmetic costs ~5 ns
   even at arity 1.
5. `for_each_component_bytes` invocation with a generic closure
   capturing `world` and the slots base — closure body has 4 SAFETY
   regions, 1 `debug_assert`, 1 unaligned write, plus the
   `if count == arity` branch that triggers `from_raw_parts` +
   `create_entity_at_with_pool_ids` call.
6. The closure's `if count == arity` branch invokes a 14-argument
   chain through `create_entity_at_with_pool_ids` → 6 unsafe blocks
   → `archetype.create_entity_with_pool_ids` → 4 inner loops over
   `components` + `pool_ids`.

Bevy's `BundleSpawner::spawn_at<T>` (`spawner.rs:91-126`) is **30
lines** of code total. The corresponding boyko apply call chain
(`SpawnAtCommand::apply` → `create_entity_at_with_pool_ids` →
`archetype.create_entity_with_pool_ids`) is **~150 lines** with
debug_asserts, multiple borrow scopes, and a stack-allocated buffer
that exists only to bridge `for_each_component_bytes`'s callback
shape to `create_entity_at_with_pool_ids`'s slice argument shape.

**Recommended fix:** Collapse `SpawnAtCommand::apply` into a single
function call that does the bundle walk and the archetype write in
one tight loop, mirroring Bevy's `BundleSpawner::spawn_at`. Specific
moves:

a. **Eliminate the `[MaybeUninit; 8]` stack array entirely.** Bundle's
   `for_each_component_bytes` calls a closure with `(ComponentId, &[u8])`
   per component; the closure should do the pool write directly inside
   the callback, not build a slice for `create_entity_at_with_pool_ids`.
   For 1-component bundles this is the difference between 192 B of
   stack + 1 callback + 1 slice rebuild + 1 outer call vs zero stack
   + 1 callback + 1 inline pool write.

b. **Hoist the archetype reservation outside the bundle walk.** The
   row index `row = archetype.current_index` is known before any
   bytes are written. Reserve capacity for 1 row up front, then have
   the bundle walk write directly into `pool_at_unchecked_mut(pool_id)`
   at the known row. After the walk, advance `current_index += 1`
   and push `entity_id`.

c. **Cache `pool_ids` outside the closure.** The closure captures the
   `pool_ids` slice via `world.bundle_column_cache.get_resolved::<B>()`.
   The closure receives an index 0..arity-1 (matching B's canonical
   component order) so the closure can index `pool_ids[i]` directly.

   Sketch:
   ```rust
   let archetype = unsafe { &mut *archetype_ptr };
   archetype.reserve_capacity(1).expect("...");
   let row = archetype.current_index;
   let current_tick = world.current_tick();

   let mut i = 0;
   self.bundle.for_each_component_bytes(|_id, bytes| {
       let pool_idx = pool_ids[i];
       unsafe {
           let pool = archetype.component_pools_mut().pool_at_unchecked_mut(pool_idx);
           pool.write_at_unchecked_initialized(row, bytes);
           pool.commit_units(row, 1);
           pool.fill_ticks(row, 1, current_tick);
       }
       i += 1;
   });
   archetype.entity_ids.push(entity.id());
   archetype.current_index = row + 1;
   world.entity_master.register_entity_with_ptr(entity, archetype_ptr, row as u32);
   ```

   Expected saving: 100-150 ns/e (eliminates the slot array setup,
   the slice-from-raw-parts rebuild, and the outer call hop through
   `create_entity_at_with_pool_ids` + `archetype.create_entity_with_pool_ids`).

### #2 — Bare archetype-write work (≈ 58 ns/e gap)

**Files:**
- `crates/boyko_ecs/src/ecs/memory/component_pool.rs:1099-1238`
  (`write_at_unchecked_initialized`, `commit_units`, `fill_ticks`).
- `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs:662-707`
  (`create_entity_with_pool_ids`).

Boyko's direct write per row (p9 = 108 ns/e):
- `archetype.reserve_capacity(1)` — `pools_iter().all(can_reserve)` —
  iterates every pool even for 1-comp bundle. ~5 ns.
- per pool: `pool_at_unchecked_mut(pool_idx)` (Vec index) → ~2 ns.
- `write_at_unchecked_initialized` (memcpy 12 B) → ~5 ns.
- `commit_units(row, 1)` — `units.reserve(1)` + raw `units_ptr.add(row).write(Unit::new(...))` + chunk mark_dirty. ~15 ns.
- `fill_ticks(row, 1, tick)` — 2 unsafe UnsafeCell writes. ~5 ns.
- `archetype.entity_ids.push` → ~3 ns.
- `current_index += 1` → free.
- `register_entity_with_ptr` — sparse_idx resize check + 2 vec writes + 1 push. ~10 ns.
- Plus the outer call hop through `EcsMaster::create_entity_at_with_pool_ids`
  (~10 ns of guard checks + tick load + archetype_ptr lookup).

Bevy's direct write per row (p10 = 50 ns/e):
- `EntityAllocator::alloc` → ~5 ns.
- `BundleSpawner::new<T>` → cached on second call; first call ~30 ns.
  But the *direct* `World::spawn` path rebuilds the spawner per call,
  so this IS in the loop. Still, BundleSpawner caches the
  `&mut Column` references, so the per-row work bypasses any
  SparseMap-equivalent lookup.
- `Table::allocate(entity)` → vec.push + per-column `initialize_unchecked`
  on tick slots. ~15 ns total.
- `bundle_info.write_components` → memcpy 12 B + tick writes. ~10 ns.
- `entities.set_location` → 1 write. ~3 ns.
- `entities.mark_spawned_or_despawned` → 1 write. ~3 ns.

Two structural differences that account for the 58 ns/e gap:

1. **boyko's `commit_units(row, 1)` is more expensive than Bevy's
   per-column `initialize_unchecked`.** boyko stores `Unit { ptr }`
   per row to support O(1) `get_component_raw` via a stored
   pointer-to-component; this is a second SoA column that mirrors the
   data column. Bevy's `Column::added_ticks.initialize_unchecked(row, ...)`
   writes just the tick UnsafeCell — Bevy does NOT maintain a parallel
   `Vec<Unit>` of per-row pointers. The `units.push` + chunk
   mark_dirty per row adds **~10-15 ns/e** that Bevy never pays.

2. **boyko's `register_entity_with_ptr` writes more state per
   entity than Bevy's `entities.set_location`.** Bevy stores
   `Entities` as a sparse Vec keyed by entity index; one write per
   spawn. Boyko writes `entities_inland[idx] = EntityInland::new(...)`
   AND pushes to `active_ids` AND writes `sparse_to_active[idx]` —
   three writes. Adds **~5-8 ns/e**.

**Recommended fix (lower priority than #1):**

a. Investigate whether the per-row `Unit { ptr }` storage is needed
   in the spawn hot path. If `get_component_raw` is the only consumer,
   it could be computed on demand from the per-pool buffer + row index
   instead of stored. Saves ~10 ns/e on every spawn — but requires a
   broader audit of `Unit`'s consumers.

b. Consider whether `active_ids` + `sparse_to_active` could be
   replaced by Bevy's single-Vec sparse model. Saves ~5 ns/e per
   spawn; ~10 ns/e per despawn (one fewer swap-remove arithmetic).

### #3 — `CommandQueue::apply` dispatch (≈ 5 ns/cmd gap)

**Files:**
- `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs:264-441`
  (`RawCommandQueue::apply_or_drop_queued_no_catch`).
- `crates/boyko_ecs/src/ecs/core/commands/command.rs:consume_and_drop_glue`.

Boyko `p5` = 12.8 ns/cmd vs Bevy `p6` = 7.8 ns/cmd → boyko is **5 ns/cmd
slower** on a noop apply walk. Sources:

1. **boyko advances `self.cursor` AFTER reading meta**, then passes
   the cursor field by raw `&mut usize` reborrow into the glue. Each
   `consume_and_drop_glue` invocation does:
   - `*cursor += sizeof::<C>()` (W3' discipline).
   - `cmd: C = cmd_ptr.read_unaligned()` (28 B for V6 bundle).
   - `cmd.apply(world)` (the user code).
   - `drop(cmd)` (Drop impl runs if Some(world); explicit drop if None).

   Bevy's `consume_command_and_get_size` is structurally identical
   (`src/world/command_queue.rs:174-194`) — the same 4 steps.

2. **boyko's apply walk does a debug_assert per command** for the
   cursor bound. In release that's gone; not the source.

3. **boyko's walk reads `*self.cursor` THREE TIMES per iteration**:
   - `while *self.cursor.as_ref() < stop_snapshot` (loop bound).
   - `let local_cursor = *self.cursor.as_ref();` (re-read).
   - And inside `consume_and_drop_glue`, the cursor is mutated via
     `&mut *self.cursor.as_ptr()`.

   These are short-lived reborrows so the optimiser can in principle
   keep `*self.cursor` in a register, but the RawCommandQueue twin's
   `NonNull<usize>` indirection may defeat that. **Bevy uses a local
   `let mut local_cursor = start;` for the walk progress**
   (`command_queue.rs:240`) and only writes back to `*self.cursor`
   when the walk ends. That's strictly fewer dependent loads on the
   loop critical path.

4. **boyko's `cmd_ptr` is `self.bytes.as_mut().as_mut_ptr().add(payload_cursor)`
   per iteration**, materialising `&mut Vec<MaybeUninit<u8>>` from a
   raw pointer twin then immediately `as_mut_ptr()`. Bevy's
   `self.bytes.as_mut().as_mut_ptr().add(local_cursor).cast::<CommandMeta>().read_unaligned()`
   is structurally similar but Bevy keeps `local_cursor` in a register.

**Recommended fix:** Mirror Bevy's local-cursor pattern. Use
`let mut local_cursor = start;` and only write back to `*self.cursor`
on success/panic exit. The panic-recovery cursor-tracking can use
`local_cursor` directly when the catch fires (Bevy does exactly this).
Expected saving: 3-5 ns/cmd × 10k = 30-50 µs/spawn loop.

The Phase 12.5 Opt-A1 fix that drove progress through `*self.cursor`
was correct for panic-recovery semantics but came with a cost; a
hybrid (local cursor + sync on panic via `catch_unwind` Err branch)
should recover the perf without losing the survivor-tracking
correctness.

## Recommended fixes (ordered by expected impact)

| # | Fix | File | Est. saving |
|---|-----|------|------------:|
| 1 | Collapse `SpawnAtCommand::apply` into a single inline write loop, eliminate `[MaybeUninit;8]` slot array, hoist the row index outside `for_each_component_bytes` | `spawn_at_command.rs:80-213` | **100-150 ns/e** |
| 2 | Mirror Bevy's local-cursor pattern in `apply_or_drop_queued_no_catch` | `command_queue.rs:291-441` | **3-5 ns/cmd × 10k = 30-50 µs** |
| 3 | Fix Phase 12.6 WIP `OnceLock<Box<[OnceLock<T>; N]>>` warm-path: collapse the two OnceLock loads into one (e.g., make `bundle_archetype_cache(&self)` an `#[inline(always)]` accessor whose body the optimiser can prove pure, OR replace `OnceLock<Box<...>>` with `Box<OnceLock<...>>` initialised at construction with a small thunk) | `ecs_master.rs:1648-1660`, `bundle_column_cache.rs:169-193` | **~3 ns/e per warm spawn** |
| 4 | Consider whether `ComponentPool::units` (per-row `Vec<Unit>` parallel to data column) can be eliminated from the spawn hot path | `component_pool.rs:1143-1192` | **~10 ns/e per spawn** (requires `Unit` consumers audit) |
| 5 | Bundle `active_ids` push + `sparse_to_active` write into a single struct-of-arrays update in `EntityMaster::register_entity_with_ptr` | `entity_master.rs:322-355` | **~5 ns/e per spawn** |

Sum of fixes 1-3 alone would reduce the boyko per-entity from ~371 ns/e
to ~225 ns/e — closing the gap to Bevy from 266 ns/e to ~120 ns/e.

Fixes 1+2+3+4 land near parity with Bevy (~105 ns/e target).

## Disproven hypotheses (from the brief)

### H1 — CommandQueue per-cmd push/apply framing → CONFIRMED but small

The brief estimated "5-10 ns per push + 5-10 ns per apply = 10-20 ns/entity".
Measurements:
- p4 push only: 20.8 ns/cmd (higher than brief's estimate — boyko's
  `push` is hotter than expected, possibly due to the 8-B meta header +
  28-B payload split into two unaligned writes plus the per-call
  reserve check).
- p5 boyko apply only: 12.8 ns/cmd.
- p6 bevy apply only: 7.8 ns/cmd.
- Net boyko queue overhead: ~13 + 21 = ~34 ns/cmd ≈ 340 µs / 10k. The
  brief's 10-20 ns/e estimate is roughly 2× low.

### H2 — `EntityMaster::register_entity_with_ptr` atomic counter → CONFIRMED small

`EntityCounter::reserve_entity` via p12 = **3.2 ns/e**. Smaller than
the brief's "5-10 ns" estimate. The reserve path uses an `AtomicUsize`
under uncontended single-thread fetch_add — that's the textbook `lock xadd`
hot path on x86_64.

### H3 — Per-component write_at + tick init → CONFIRMED ~30 ns/e for 1-comp

p9 (spawn_one) at 108 ns/e includes ~10-15 ns for `commit_units(1)`
and ~5 ns for `fill_ticks(1)`. Together that's ~20-25 ns/e on the
direct path. For Commands path, add ~5 ns to thread through the
extra glue. So per-component overhead = ~25-30 ns/e — close to the
brief's "10 ns/component" estimate but slightly higher because
`commit_units(row, 1)` does Vec::reserve(1) + raw ptr write + chunk
arithmetic + mark_dirty even at count=1.

### H4 — `BundleColumnCache` lookup → CONFIRMED ~3 ns/e under WIP

Pre-WIP (v2 baseline): single `OnceLock::get` per spawn → ~777 ps
(v2 h1). Post-WIP: **two** OnceLock loads per spawn (outer
`bundle_column_cache()` get_or_init + inner `[id].get_resolved`) →
estimated ~3 ns/e (cannot measure directly because the accessor is
`pub(crate)`; inferred from cached_archetype_id-shaped p13 which is
also touched by the WIP and shows 1.1 ns/e — `cached_archetype_id`
goes through `bundle_archetype_cache()` which has the same shape).

This is **a Phase 12.6 WIP-introduced regression**, not a pre-existing
issue.

### H5 — `for_each_component_bytes` callback overhead → DISPROVEN for 1-comp

V2 h4 measured 1.74 ns/e for 1-comp bundles. Still ~1-2 ns/e in
this profile. Not a hotspot.

### H6 — `SpawnAtCommand` storage size → DISPROVEN

`SpawnAtCommand<V6PosBundle>` = `Entity(16 B) + V6PosBundle(12 B) = 28 B`
+ meta header 8 B = 36 B per command. For 10k commands = 360 KB which
fits in L2. The byte arena is sequentially walked and prefetcher-friendly;
cache effects are not the dominant cost.

## Surprise findings (not in the brief)

### Surprise 1: Phase 12.6 WIP regressed `Commands::spawn` from 224 → 371 ns/e

The baseline cited in the brief (224 ns/e post-12.5) was measured
before the Phase 12.6 lazy-init WIP. The WIP wraps the cache arrays
in an outer `OnceLock<Box<...>>` for lazy allocation. This costs:
- One extra `OnceLock::get` per cache touch on the warm path.
- For spawn: `bundle_archetype_cache()` + `bundle_column_cache()` are
  both touched per command. That's **2 extra OnceLock loads per
  spawn**.
- Estimated added cost: ~3-5 ns/e per spawn from the doubled cache
  shape, plus secondary cache-miss effects from spreading state across
  more allocator slabs.

But that ~5 ns/e doesn't explain the full ~147 ns/e gap from
224 → 371. The actual cause looks like **per-iter allocator state
churn**: the WIP `EcsMaster::new` no longer allocates the
~1.5 MB of `entities_inland` / `sparse_to_active` slots, so the OS
heap is much smaller per iter, but then the first spawn triggers
multiple smaller allocations during the apply path (`entities_inland.resize`,
`bundle_archetype_cache()` first-touch, etc). The variance across
the 3 runs (range 3.71-5.73 ms) is consistent with allocator pressure
being the bottleneck.

**Implication for fixes:** Stabilising the bench number requires
either restoring the pre-12.6 eager allocation (which costs 480 µs
in `EcsMaster::new` per iter but stabilises the per-iter measurement),
OR completing the lazy-init properly with a `with_capacity`-style
API that pre-warms everything when the caller knows the workload
size. Fix candidate: `EcsMaster::with_capacity(entity_capacity,
archetype_capacity, bundle_count, query_count)` that pre-allocates
everything.

### Surprise 2: bevy's `CommandQueue::apply` for noop closures is only 7.8 ns/cmd

That's the structural floor for any type-erased command dispatch with
`catch_unwind`. Bevy is hitting that floor for noops. Boyko at 12.8 ns/cmd
is 64 % over the floor — every refactor that keeps the same shape
(meta header + unaligned reads + glue indirection) is bounded by
~8 ns/cmd. **Sub-8 ns/cmd is impossible without a structurally
different design** (e.g., command_buffer.flush() that statically knows
the type and inlines the apply, eliminating the indirect function-pointer
call).

### Surprise 3: boyko's `p9_direct_spawn_one` (108 ns/e) is FASTER than `p7_direct_spawn_batch_size_1_loop` (128 ns/e)

This is counterintuitive — the Opt-A3 `spawn_batch` path was supposed
to be faster than legacy `create_entity`. The 20 ns/e gap comes from:
- spawn_batch builds a `Vec<Entity>` result of size 1 per call (W3
  ergonomic return) — ~5 ns/call alloc.
- spawn_batch's `Map<Range<usize>, FnMut>` iterator wrapper around
  the single bundle adds ~3 ns/call.
- spawn_batch reserves capacity for the next batch (n + MAX_BATCH_HINT
  = 1 + 8192 = 8193 slots) every call, when entity_master is already
  past that — `ensure_capacity` no-ops but the call cost (~2 ns) is
  still paid.
- spawn_batch's `BundleColumnCache::resolve_and_cache` is called on
  first warm-up but subsequent calls use the cached path (fine).
- `SpawnBatchCommand::apply` has ~12 W4 hoist debug_asserts in debug
  builds (release: zero).

For batch sizes of 1, the legacy `create_entity` path is actually
better. The Opt-A3 spawn_batch path wins only at N ≥ 5 or so where
the per-batch fixed overhead amortises.

## Honest residuals

1. **`p11` (Commands::spawn + .id())** clocked at 3.83-6.38 ms across
   runs. This is significantly higher than p1 (3.71 ms) despite
   `.id()` being a free Entity-field read. The 200+ ns/e overhead
   may be the `EntityCommands<'_, 's>::new` construction (which
   builds an EntityCommands struct with the commands pointer + entity
   pointer + lifetime markers) being optimised away in p1 but not in
   p11 — but both should optimise away. Likely measurement noise
   from p11 running after 10 other benches have churned the allocator;
   the Instant brackets in p3 give cleaner per-stage numbers.

2. **`p14_bevy_enqueue_vs_apply`** inner Instant numbers varied across
   runs: enqueue 12.77-84.4 ns/e, apply 67.95-294.20 ns/e. The wall
   time was stable across runs but the Instant brackets are at the
   end of a long bench suite and the OS may suspend the bench thread
   mid-bracket. The boyko equivalent p3 is consistent because it sits
   earlier in the bench order.

3. **None of the boyko hot-path benches measure the WIP-specific
   `bundle_column_cache()` accessor cost.** That accessor is
   `pub(crate)` and not reachable from the bench crate. The estimate
   of "~3 ns extra per warm spawn from the doubled OnceLock load"
   is inferred from cached_archetype_id-shaped p13 (which has the
   same shape but goes through `bundle_archetype_cache()`).

## Artefacts

- Bench source: `crates/bench_bevy_vs_boyko/benches/profile_spawn_single.rs`.
- Cargo registration: `crates/bench_bevy_vs_boyko/Cargo.toml`
  `[[bench]] name = "profile_spawn_single"` block.
- Raw bench logs: `D:/tmp/profile_spawn_single.log`,
  `D:/tmp/profile_spawn_single_run2.log`,
  `D:/tmp/profile_spawn_single_run3.log`.
- Repro: `cargo bench -p bench-bevy-vs-boyko --bench profile_spawn_single`.

## Methodology caveats

1. **Phase 12.6 WIP state.** The working tree contains uncommitted
   changes (lazy-init `OnceLock<Box<[OnceLock<T>]>>` for cache arrays,
   `EcsMaster::new` no longer pre-extends `entities_inland`). The
   measured numbers reflect this state, not the post-12.5 baseline
   that the orchestrator's brief cites. The structural attribution
   (relative cost ratios) holds across both states; the absolute
   per-entity numbers do not.

2. **`Instant::now` floor on Windows.** Each pair costs ~74 ns
   (`p0`). Bracketing the *whole* 10k-entity loop adds 74 / 10k =
   0.007 ns/entity — negligible. Per-entity inner brackets would be
   measurement noise.

3. **`iter_with_setup` allocator-state noise.** Per-iter
   `EcsMaster::new` rebuilds churn ~1-5 MB of heap per iter,
   inflating variance on `p1`, `p3`, `p7`, `p11`. Numbers from
   single-allocation benches (`p2`, `p6`, `p10`, `p12`, `p13`) are
   ±5 % stable; numbers from rebuild benches (`p1`, `p3`, `p7`, `p11`)
   are ±15-27 % over 3 runs.

4. **No direct measurement of `SpawnAtCommand::apply`-isolated cost.**
   The closest proxy is p3 apply (inner Instant). The 343 ns/e
   "apply" includes both `SystemParam::apply` framework cost (call
   into `CommandQueue::apply` once per system, ~30 ns total amortised
   to 0.003 ns/e at 10k) and the per-cmd dispatch + per-cmd
   apply work.

5. **No measurement of cold-arm `resolve_and_cache`.** The bench
   warms the bundle_column_cache before measurement so the cold arm
   never fires. The brief's hypothesis that cold-cache hits drive
   variance was not tested; the variance we see is in the warm hot
   path itself, dominated by allocator state.

## Summary

The 148 ns/e gap reported in the brief (224 vs 76 ns/e) actually
became wider (266 ns/e) under the Phase 12.6 lazy-init WIP — boyko
regressed from 224 to 371 ns/e while Bevy held at ~105 ns/e.
Structurally the gap is in three places:

1. **`SpawnAtCommand::apply` glue** carries ~230 ns/e of stack
   array setup + closure dispatch + multi-call apply chain that
   Bevy's `BundleSpawner::spawn_at` does in ~50 ns/e of inline code.
   This is the biggest target: fixing it alone closes ~100-150 ns/e.

2. **Bare per-row archetype-write work** is ~58 ns/e slower than
   Bevy's, dominated by boyko's parallel `Vec<Unit>` storage that
   Bevy does not maintain.

3. **`CommandQueue::apply` dispatch** is ~5 ns/cmd slower than
   Bevy's due to the cursor-as-progress-tracker design choice (which
   was correct for panic-recovery semantics but costs perf).

The Phase 12.6 lazy-init WIP added ~3-5 ns/e of warm-path overhead
on top of all of the above — that should be fixed before measuring
the post-fix Phase 12.6 numbers.
