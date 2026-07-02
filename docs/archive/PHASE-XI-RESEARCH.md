# Phase X.I — ComponentPool Row-Capacity Growth: Code Inventory

Project-analyst inventory at `fb7cf1e`. The engine's LAST hard ceiling: every pool is
fixed at `num_chunks × components_per_chunk` rows; an archetype that outgrows its pools
fails (direct API errors, EVERY deferred-command apply path PANICS).

## 1. ComponentPool facts (`memory/component_pool.rs`)

- Fields: `arena: *const Arena` (:35, dead — "future growth Phase 3"), `buffer: NonNull<u8>`
  (:38, write-once, ONE arena block — `arena.allocate_layout` at :141 is **the only
  production arena client in the engine**), `buffer_capacity_bytes` (:43, dead),
  `max_components` (:46, THE ceiling = chunks×per_chunk :118), `len` (:49),
  `chunks: Vec<Chunk>` (:52, sized once :155-159), `component_layout` (:61), `drop_fn`,
  `added_ticks`/`changed_ticks: Box<[UnsafeCell<Tick>]>` (:83/:90 — STORE2 global-heap,
  sized to max_components :161-173, documented "never reallocates" :73-74, :977-980).
- Buffer align = max(align_of::<T>, SIMD_BUFFER_ALIGN=32) (:128-129, X.A; debug-asserted
  :148-153) — any new allocation scheme must preserve.
- `row_ptr(idx) = buffer.add(idx*stride)` (:224-235) — single funnel, 15 callers
  (add :267, add_typed :317, pop :344, swap_remove :397+, get_raw :464, get_raw_mut :478,
  set_component :599/:669, unit_ptr :804, drop_at :838, write_at :873,
  swap_remove_no_drop :919, write_at_unchecked :1153, fill_ticks raw loop :1270, Drop :1344).
  Its SAFETY cites X.F address stability — text load-bearing. X.B identity tests pin
  `get_raw(i) == buffer_ptr()+i*stride` (:1632-1700, :1937-1959).
- `max_components` consumers: add/add_typed full checks :253/:301; capacity :691;
  is_full :773 (→ `can_push_entity_components`, component_pool_bundle.rs:141);
  remaining_capacity :779; can_reserve :1092 (→ `Archetype::reserve_capacity`,
  archetype.rs:797); write_at debug :1131; commit_units debug :1191; tick sizing :168.
- **Full-pool behavior today**: `add`→None (:253), `create_entity`→false (archetype.rs:467)
  → `EcsError::ArchetypeRejectedEntity` (ecs_master.rs:680-691); `reserve_capacity(n)` →
  `Err(ArchetypePoolCapacityExceeded)` (archetype.rs:795-807, error.rs:88-95);
  **SpawnAtCommand::apply PANICS** (spawn_at_command.rs:174-175), **SpawnBatchCommand
  PANICS** (spawn_batch_command.rs:337), **insert-migration PANICS**
  (migration_helpers.rs:245-246), **remove-migration PANICS** (:571-577).
- Tick-buffer escapes: `added_ticks_ptr`/`changed_ticks_ptr` (:983/:994) →
  `Archetype::tick_column_base` (archetype.rs:521-527) → query Fetch per archetype
  boundary (data.rs:698+, filter fetches). NOT cached cross-frame — every set_table
  re-reads; growth in apply window has no live Fetch (ALLOC1). Documented
  "lifetime-stable" promises need renegotiation.
- `Chunk` (chunk.rs): start_index/capacity/is_dirty — **vestigial: dirty flags written by
  every mutation, read by NOBODY; zero external callers of chunk accessors**. Query-side
  "chunks" are row ranges, not these objects. `commit_units` SAFETY claims chunks "NEVER
  mutated" (:1206-1212) — amend if growing.
- SEND10 (:1293-1297) already reserves the growth contract: apply-window-only via ALLOC1.

## 2. Archetype facts (`core/archetype/archetype.rs`)

- `Column{ptr,stride,_reserved}` 16 B repr(C), `columns:[Column;512]` at offset 0; struct
  size PINNED 8480 B by const assert (:213) — adding Archetype fields trips it.
- `refresh_column` (:353-370) only after add_pool; **`refresh_all_columns` (:379-392)
  `#[cold]` DEAD — the designated relocation hook if relocate-on-grow is chosen** (would
  also need a ComponentPool::buffer mutator — none exists). U10: data ops never refresh
  (:349-351) — in-place extension preserves verbatim.
- `entity_ids: Vec<EntityId>` (:190) — plain heap Vec, Vec::new() start, ordinary doubling,
  NO ceiling, NOT coupled to pool rows (a residual realloc-doubling class, 8 B/row).
- Row flow: `create_entity` two-phase commit (can_push :467 → push lockstep → ticks →
  entity_ids.push :499 → current_index+=1). `reserve_capacity(n)` (:795-807) = the single
  capacity guard funnel for ALL batch/command/migration paths — **the natural grow hook**.

## 3. Reader tolerance matrix (THE design decider)

ALL paths capture one base per column per archetype, then index `[0, entity_count)`:
typed iter (data.rs set_table :357-405/:531-604; iter.rs current_len :282), for_each_chunk
(WHOLE-archetype slice, chunk_iter.rs:171-173 — demo's zero-copy GPU upload relies on
single-slice), par_iter (ranges over fresh entity_count :296-326), legacy pointer-bump
(:333-341), random access (columns[c].ptr.add(unit_index*stride), ecs_master.rs:1320 —
the ~3 ns Phase-7 path).

- **(a) In-place buffer extension: tolerated by EVERYTHING with zero changes** (all
  re-derive entity_count per use; no cross-window caching).
- **(b) Multi-base/segmented pools: BREAKS random access** (unit_index→segment = extra
  dependent load on get_component_raw/_mut/set — the X.G chunked-indirection disqualifier
  applies on the COMPONENT side too), breaks typed per-row iter and legacy bump without
  per-segment re-dispatch, fractures for_each_chunk's single-slice contract; only the
  fetch_chunk paths adapt naturally (segment-aligned splitting).

## 4. Arena adjacency (kills naive in-place)

- free_mem_block start_map/end_map could support `try_extend_in_place` (mirror of the X.F
  end_map probe :245-249), BUT: **no deallocation path exists** (insert called only from
  seed + grow_then_retry); pools allocate back-to-back per archetype
  (create_by_ids :277-283), bump-style across archetypes (best-fit on one tail block).
  ⇒ free space after a pool = ≤31 B align spill, or the frontier (last pool only).
  **In-place extension succeeds for ~1/N pools; first growth of any non-last pool collides
  with the next pool's live block.**
- ⇒ Universal address-stable growth requires **per-pool virtual headroom**: per-pool
  `VmReservation`s (the InlandStore pattern at pool granularity) — `VmReservation` is
  one-object-one-range (no fixed-address/multi-range API). Relocate-on-grow keeps the
  packed shared arena at the price of memcpy (the class X.F/X.G deleted) + reviving
  refresh_all_columns + rewriting the row_ptr/U6/U10/STORE2 invariant lattice + X.B tests.
- NOTE: pools are the arena's ONLY client — a per-pool-reservation design orphans the
  shared Arena (retire vs keep-for-fallback is an architect decision).

## 5. Size classes / constants

constants.rs:60-97: DEFAULT_CHUNKS_PER_POOL=128; per-chunk tiny 2048 / small 1024 /
medium 512 / large 256 ⇒ ceilings 262,144 / 131,072 / 65,536 / 32,768 rows.
DEFAULT_COMPONENTS_PER_CHUNK=1024 is dead. Only readers: with_default_sizes (:197-203) +
get_optimal_chunk_capacity (:206-216) via component_pool_bundle.rs:66. No user knob.
Bench workarounds for the ceiling: bundle_static_cache.rs:239-254, query_dsl.rs:441-442.

## 6. Tests pinning today's behavior

drop_fn.rs:426-430 (add_typed None at capacity, value dropped once); two in-file pool
proptests drive to capacity() and skip (component_pool.rs:1864-1871, :2067-2074). **Nothing
pins ArchetypePoolCapacityExceeded / full-pool ArchetypeRejectedEntity / the apply panics**
— the panic surface is untested; the None-at-capacity tests need re-spec if add grows.

## 7. Migrations + hooks

Insert/remove migrations allocate 1 row in TARGET pools — same ceiling, panic today
(migration_helpers.rs:245/:571). Hooks/observers fire AFTER rows exist and never allocate
pool memory; deferred-drain re-entrancy (14a/14b) means growth must be plain `&mut self`
window mutation (it is, by SEND10). Growth slots in reserve_capacity/add BEFORE any hook.

## 8. Prior art shapes

X.F grow_then_retry funnel (arena.rs:397-504; sufficiency-before-state-change, GROW1);
X.G InlandStore ensure/grow_to + lazy reservation + Deref-zero-churn; X.H VmReservation
(memory/vm.rs:85-330) — instantiate N times for per-pool; X.A SIMD-32 base contract +
single-slice for_each_chunk contract; X.B row_ptr identity test pins.

## Bottom line (facts)

1. In-place extension: transparent to every reader.
2. Shared-arena packing makes naive in-place work for ~1/N pools — insufficient alone.
3. Universal no-move growth ⇒ per-pool reservations (VA cost: ~1 GiB-class per pool is
   free on 64-bit; fallback arm needs a separate story — eager per-pool reserves are
   fatal on wasm/Miri).
4. Whatever grows data must grow added/changed tick Boxes in lockstep (their never-realloc
   promise renegotiated or VM-reserved alongside — e.g. one reservation per pool with
   [data | added | changed] sub-regions and three frontiers).
5. Insertion funnels: add/add_typed full branches (:253/:301) + Archetype::reserve_capacity
   (:795-807). The deferred-apply PANICS become growth.
