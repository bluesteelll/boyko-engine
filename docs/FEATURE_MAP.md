# Feature map — where to find what (branch `ecs`)

First point of contact for agents. When you need to know *where* a particular
piece of functionality lives, start here, then go to
[SYSTEMS.md](SYSTEMS.md) for details and finally to the source.

**Legend:**
- ✅ Implemented and tested
- ⚠️ Implemented with documented caveats
- 📋 Planned (tracked in `docs/ROADMAP-PHASE-2-PLUS.md`)
- ❌ Not implemented (deliberately — see linked rationale)

> The `ecs` branch builds clean and is verified green by CI on every push:
> `cargo check --all-targets`, `cargo clippy --all-targets -- -D warnings`,
> `cargo test --workspace` (223 tests pass), `cargo +nightly miri test`
> (~8000 s, Stacked Borrows clean). See [SYSTEMS.md](SYSTEMS.md) §13 for
> the gate report.

---

## Memory and allocation

| What you want to do | Where | Method / type |
|---------------------|-------|---------------|
| Allocate a block of N bytes | [memory/arena.rs](../crates/boyko_ecs/src/ecs/memory/arena.rs) ✅ | `Arena::allocate_layout(layout)` |
| Allocate with explicit alignment | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::allocate_aligned(size, align)` |
| Find best-fit free block | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::find_best_fit` |
| Return memory to the pool (auto-merge) | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::insert` |
| Align an address/size | [memory/utils.rs](../crates/boyko_ecs/src/ecs/memory/utils.rs) ✅ | `align_up(value, alignment)` |
| Defragment the free-block list | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::defragment` |
| Get memory statistics | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::get_memory_stats` |
| Free the arena | [memory/arena.rs](../crates/boyko_ecs/src/ecs/memory/arena.rs) ✅ | `impl Drop for Arena` (M-001 closed) |

`MemFreeBlockMaster` uses `BTreeMap<size, Vec<idx>>` plus `start_map` /
`end_map` for O(1) coalesce on insert (M-012 closed: was `HashMap`,
allocator-killer in hot path). `M-018` closed via a reverse index for O(1)
block-position lookup.

---

## Type-erased component storage

| What you want to do | Where | Method |
|---------------------|-------|--------|
| Create a pool for a registered component | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::new(arena, component_id, num_chunks, components_per_chunk)` |
| Append a component (raw bytes) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::add(&[u8])` |
| Append a component (typed, TypeId-guarded) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::add_typed::<T: Component>(value)` |
| Read a component (typed, TypeId-guarded) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::get_typed::<T>(idx)` / `get_mut_typed::<T>(idx)` (C-004 closed) |
| Read a component (raw pointer) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `get_raw(idx)` / `get_raw_mut(idx)` |
| Overwrite an existing slot | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `set_component(idx, &[u8])` / `set_component_typed::<T>(idx, value)` |
| Remove a component (swap with last) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::swap_remove(idx)` (runs `drop_fn` if registered) |
| Iterate all slots as `&[Unit]` | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `chunk_units(chunk_index)` (M-019 closed) |
| Get the underlying buffer pointer (for per-entity iter) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `buffer_ptr()` — SAFETY-contracted accessor used by `Query::iter_one/iter_two` |

`ComponentPool::add_typed`, `set_component_typed`, `get_typed`,
`get_mut_typed` all `debug_assert_eq!(TypeId::of::<T>(),
self.component_type_id)` — catches mis-typed access before any byte
operation (audit C-004 closed).

Drop discipline (M-001 cont. / M-004): pools store a cached `drop_fn:
Option<DropFn>` from the registry; `swap_remove` / `pop` / `set_component`
all invoke it on the displaced slot. Verified by 16 dedicated tests in
`tests/drop_fn.rs`.

---

## Chunks (metadata windows into the pool's buffer)

| What | Where | Method |
|------|-------|--------|
| Create chunk metadata | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::new(start_index, capacity)` |
| Get start_index | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::start_index()` |
| Get capacity | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::capacity()` |
| Mark dirty | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::mark_dirty()` |
| Read dirty flag | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::is_dirty()` |
| Clear dirty flag | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::clear_dirty_flag()` |

> `Chunk` stores metadata only — the actual component data lives in
> `ComponentPool::buffer`. The dirty flag is written by pool mutations
> but not yet read by any consumer; it is groundwork for the change-detection
> phase (planned, see roadmap).

---

## Direct component pointer (Unit)

| What | Where | Method |
|------|-------|--------|
| Construct a Unit | [memory/id_unit.rs](../crates/boyko_ecs/src/ecs/memory/id_unit.rs) ✅ | `Unit::new(ptr: *mut u8)` |
| Get the pointer | [memory/id_unit.rs](../crates/boyko_ecs/src/ecs/memory/id_unit.rs) ✅ | `Unit::ptr()` |

`Unit` is `#[repr(transparent)]` — single `*mut u8`, identical layout
to a raw pointer (M-005 closed: dead `buffer_index` field removed;
M-006 false alarm: `*mut u8` already opts out of `Send + Sync`).

---

## Per-entity component iteration (Query)

| What | Where | Type / method |
|------|-------|---------------|
| Archetype-level cached query | [core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs), [core/iters/query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs) ✅ | `Query` / `QueryState` / `QueryStateIter` — yields `&Archetype` per match |
| Per-entity, 1 component | [core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs) ✅ | `query.iter_one::<A>()` yields `&A` (Phase 2d) |
| Per-entity, 2 components | [core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs) ✅ | `query.iter_two::<A, B>()` yields `(&A, &B)` (Phase 2d) |
| Per-entity, ≥3 components | — | 📋 Phase 2d-extension |
| Per-entity mutable | — | 📋 Phase 2d-extension |
| Generic tuple syntax `Query::<(&A, &B)>::iter()` | — | 📋 Phase 2d-final (Bevy WorldQuery shape) |
| Per-archetype batched slice (sequential) | [core/iters/query/chunk_iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/chunk_iter.rs), [core/iters/query/chunked_data.rs](../crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs) ✅ | `Query::for_each_chunk(\|slice\| ...)` — yields one contiguous `&[T]` (or tuple of slices) per matched archetype. Compile-time bounds: `D: ChunkedQueryData` + `F: ArchetypalQueryFilter` (Phase X.A) |
| Per-archetype batched slice (parallel) | [core/iters/query/par_chunk.rs](../crates/boyko_ecs/src/ecs/core/iters/query/par_chunk.rs) ✅ | `Query::par_for_each_chunk(\|slice\| ..., BatchingStrategy)` — sub-archetype-range fan-out via `boyko_threadpool::scope`; closure must be `Fn + Send + Sync` (Phase X.A) |
| Direct-API batched (no SystemParam) | [core/iters/query/query_view.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs) ✅ | `QueryView::for_each_chunk` / `QueryView::par_for_each_chunk` — same shape on `EcsMaster::query<D, F>()` (Phase X.A) |

Cache properties (Q-011 closed): `QueryState` is reusable across frames;
warm-path iter is one pointer load + comparison + slice walk
(~3.6 ns vs 77 ns for one-shot `Query` construction, ~21× speedup
measured in `benches/query_iter.rs`).

ABA-safety (Phase 5c): `QueryState` survives `master.clear()` and
`remove_archetype()` without losing correctness, via a dual-counter
design (`generation` for creation deltas, `structural_generation` for
removal/clear-triggered full rebuilds). A recycled `ArchetypeId` with
an unrelated component set will NOT be surfaced to a stale `QueryState`.

---

## Archetype-level discovery (lower-level lookups)

| What | Where | Method |
|------|-------|--------|
| Find archetypes containing all of `[ids]` | [core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs) ✅ | `find_archetypes_with_components(ids)` / `..._into(out)` |
| Find archetypes whose mask is a superset | [core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs) ✅ | `find_matching_archetypes(&mask)` / `..._into(out)` |
| Exact-mask match | [core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs) ✅ | `find_exact_match(&mask)` / `..._into(out)` |
| With include/exclude/optional filters | [core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs) ✅ | `find_with_filter(&inc, &exc, &opt)` / `..._into(out)` |
| Get archetype signature by id | [core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs) ✅ | `get_archetype_signature(id)` (O(1) via reverse map, C-015 closed) |
| Count of registered archetypes | [core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs) ✅ | `len()` (O(1) cached, C-015 closed) |

All `find_*_into(out)` variants reuse the caller's `Vec` (zero-alloc
warm path, Q-013 closed). The `<=3 components` path uses a stack-only
`[u8; 3]` relevant-block buffer with insertion-sort + dedup (Phase 2a).

---

## Components

| What you want to do | Where | How |
|---------------------|-------|-----|
| Define a component type | [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) ✅ | `#[derive(Component)] struct MyComp { … }` |
| Get the unique ID for a component | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::component_id() -> ComponentId` (lazy, OnceLock-per-type) |
| Get component size (compile-time inline const) | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::SIZE` (`const`), `MyComp::mem_size()` (fn) |
| Get alignment | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::ALIGN`, `MyComp::alignment()` |
| Get full Layout | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::layout()` (`const fn`) |
| Get TypeId | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::type_id()` |
| Get type name (debug) | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::debug_type_name()` |
| Register a layout explicitly (escape hatch) | [core/component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs) ✅ | `register_layout::<T>(id)` (test/macro-only) |
| Fetch a layout from the registry | [core/component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs) ✅ | `get_layout(id)`, `get_layout_unchecked(id)` |
| Build a ComponentMask | [core/component/component_mask.rs](../crates/boyko_ecs/src/ecs/core/component/component_mask.rs) ✅ | `ComponentMask::new()` + `set(id)` / `from_components(&[id])` |
| Bundle pools for one archetype | [core/component/component_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs) ✅ | `ComponentPoolBundle` |
| Two-phase push (validate then commit) | [core/component/component_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs) ✅ | `can_push_entity_components(&[(id, &[u8])])` + `push_entity_components(&[(id, &[u8])])` (C-009 closed) |
| Type-safe spawn (no manual byte slicing) | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `ecs.spawn_one::<A>(arch_id, value)` / `spawn_two::<A, B>(arch_id, a, b)` (Phase 2e) |
| ZST components | — | ❌ ZSTs not supported (`debug_assert!(size > 0)` in `ComponentPool::new`). Tracked as Phase 2-future enhancement. |

ID assignment (C-003 closed): a per-type `OnceLock<ComponentId>` caches
the result of `register_new::<Self>()` — first call mints a fresh ID
from a global `AtomicUsize`, every subsequent call returns the cached
value. Cross-thread safe. **IDs are unstable across processes** —
serialised IDs require a startup warm-up contract (call
`Type::component_id()` for every persisted type before deserialising).

---

## Entities

| What you want to do | Where | How |
|---------------------|-------|-----|
| Construct an Entity literal | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ | `Entity::new(id, generation)` |
| Construct with generation = 0 | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ | `Entity::with_id(id)` |
| Compare entities (id + generation) | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ | `e1 == e2` (`PartialEq`, derives include both fields) |
| Allocate an entity (recycle if available) | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::allocate_entity() -> Entity` |
| Register into an archetype | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `register_entity(entity, archetype_id, unit_index)` |
| Get the internal metadata | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `get_entity_inland(entity) -> Option<&EntityInland>` |
| Validate an entity (id + generation match) | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `is_entity_valid(entity) -> bool` |
| Deallocate (bumps generation) | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `deallocate_entity(entity)` |
| Iterate only ACTIVE entities | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `iter_entities()` — O(active), not O(capacity) (C-012/013 closed) |
| Rewind a fresh allocation on failure | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `rewind_allocate(entity)` — internal C-007 guard plumbing |

Entity fields are private (C-023 closed) — access via `entity.id()` /
`entity.generation()` getters. `Entity` derives `Clone + Copy + Hash +
Eq + PartialEq`.

The `id`/`generation` pair is the load-bearing ABA defence at the
entity layer: after `deallocate_entity` the generation bumps, so any
stale `Entity` value still in user code (including event payloads that
captured an entity reference in a prior frame) fails `is_entity_valid`
silently. `SparseSlotMap` (boyko_utils) has a parallel ABA fix at the
slot layer — see M-016 (Phase 5b).

---

## High-level facade (EcsMaster)

| What you want to do | Where | Method |
|---------------------|-------|--------|
| Construct an ECS instance | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `EcsMaster::new()` / `with_capacity(entity_cap, arch_cap)` |
| Create an archetype | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `create_archetype(&[ComponentId])` / `get_or_create_archetype(...)` |
| Spawn an entity (raw byte API) | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `create_entity(archetype_id, &[(ComponentId, &[u8])])` (C-010 slice API) |
| Spawn an entity (typed) | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `spawn_one::<A>(arch_id, a)` / `spawn_two::<A, B>(arch_id, a, b)` |
| Delete an entity | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `delete_entity(entity) -> bool` |
| Read a component (raw) | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `get_component_raw(entity, component_id)` / `get_component_raw_mut(...)` |
| Write a component (raw bytes) | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `set_component_raw(entity, component_id, &[u8])` |
| Check entity existence | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `has_entity(entity)` |
| Check component presence | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `has_component(entity, component_id)` |
| Iterate entities | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `iter_entities()` |
| Query entity IDs by components | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `query_entities(&[ComponentId])` (allocates Vec; for hot paths use `Query::iter_one/two`) |
| Drop everything | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `clear()` |

Spawn / fallible paths return `EcsResult<T>` — see
[core/error.rs](../crates/boyko_ecs/src/ecs/error.rs) for the
`#[non_exhaustive] enum EcsError` (C-019 closed: was `anyhow::Result`).
Variants: `ArchetypeNotFound`, `EntityNotFound`,
`ComponentPoolFull`, `UnknownComponentForArchetype`,
`ArchetypeRejectedEntity`, `PoolSwapRemoveFailed`.

The two-phase commit pattern (C-007 + C-009) guarantees that a failed
spawn leaves `EntityMaster`'s counters untouched — no leaked EntityIDs.

---

## Events

| What you want to do | Where | How |
|---------------------|-------|-----|
| Define an event type | [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) ✅ | `#[event] struct DamageEvent { #[participant(components = "…")] victim: Entity, #[parameter] amount: f32 }` |
| Get the event ID | [core/events/event.rs](../crates/boyko_ecs/src/ecs/core/events/event.rs) ✅ | `DamageEvent::event_id()` (lazy, OnceLock-per-type, mirror of Component) |
| Read event registry metadata | [core/events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs) ✅ | `get_event_info(id)`, `get_event_layout(id)`, `get_event_participants(id)` |
| Validate event types at runtime | [core/events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs) ✅ | `validate_event_types::<E>(id)` |
| Iterate registered events | [core/events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs) ✅ | `iter_registered_events()` |
| Participants trait | [core/events/participants/participants.rs](../crates/boyko_ecs/src/ecs/core/events/participants/participants.rs) ✅ | `Participants: Copy`, `ParticipantInfo` |
| Buffer for participants (per-type) | [core/events/participants/participants_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/participants/participants_buffer.rs) ✅ | `ParticipantBuffer<P>` — `Vec<MaybeUninit<u8>>` + TypeId guard (Q-019 closed) |
| Parameters trait | [core/events/parameters/parameters.rs](../crates/boyko_ecs/src/ecs/core/events/parameters/parameters.rs) ✅ | `Parameters: Copy` |
| Buffer for parameters (per-type) | [core/events/parameters/parameters_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/parameters/parameters_buffer.rs) ✅ | `ParametersBuffer<P>` — same shape + TypeId guard |
| Dispatch / queue events between systems | — | ❌ **Not implemented.** Q-025 deleted the commented-out `event_pool` / `event_pool_bundle`. No dispatcher, no reader, no per-frame queue. Building this is its own feature (would require double-buffer + reader cursor design). |
| Event-payload ABA across frames | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ (user-driven) | `Entity` payloads in events carry `(id, generation)`. Systems consuming events MUST call `master.is_entity_valid(event.victim)` — automatic rejection requires the missing dispatcher. |

---

## Query / filter masks

| What | Where | Type / method |
|------|-------|---------------|
| 512-bit fixed component mask | [core/component/component_mask.rs](../crates/boyko_ecs/src/ecs/core/component/component_mask.rs) ✅ | `ComponentMask` (built on `[BitSet<u64>; 8]`) |
| Hierarchical mask summary | [core/archetype/archetype_signature.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_signature.rs) ✅ | `ArchetypeSignature { mask, block_summary: BitSet<u8>, section_summary: BitSet<u32> }` — private fields, accessors only (C-023 closed) |
| Build component IDs from a tuple type | [core/iters/component_set.rs](../crates/boyko_ecs/src/ecs/core/iters/component_set.rs) ✅ | `trait ComponentSet { fn component_ids() -> &'static [ComponentId] }` — `&'static` slice cached per type (Q-012 closed) |
| 1024-bit archetype dedup bitset | [core/iters/archetype_bit_set.rs](../crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs) ✅ | `ArchetypeBitSet` — inline 128 B, used by `QueryState` |

---

## Identifiers

| What | Where | Type |
|------|-------|------|
| Entity / archetype / component / chunk IDs | [identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs) ✅ | `#[repr(transparent)] pub struct EntityId(pub usize)` and siblings (C-017 closed: were aliases to `usize`, now strongly-typed newtypes) |
| Generation counter | [identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs) ✅ | `pub type Generation = usize` (kept as alias — never crossed with other ID types) |
| Slot (boyko_utils) | [boyko_utils/identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs) ✅ | `Slot { index: usize, generation: Generation }` |

Each newtype: `Debug + Default + Clone + Copy + PartialEq + Eq + Hash
+ PartialOrd + Ord + Display`, plus `const fn new` / `const fn get`,
`From<usize>` / `From<Self> for usize`. Defined via a single
`define_id!` macro to keep ten near-identical types DRY.

---

## boyko_utils (reusable collections)

| What | Where | Type |
|------|-------|------|
| Dense sparse set (usize keys) | [boyko_utils/sparse_map/sparse_map.rs](../crates/boyko_utils/src/sparse_map/sparse_map.rs) ✅ | `SparseMap<U>` |
| Sparse-set iteration over active indices | [boyko_utils/sparse_map/sparse_map.rs](../crates/boyko_utils/src/sparse_map/sparse_map.rs) ✅ | `active_indices() -> &[usize]`, `iter_dense() -> slice::Iter<U>` (C-012/013 closed) |
| Generation-tracked slot map | [boyko_utils/sparse_map/sparse_slot_map.rs](../crates/boyko_utils/src/sparse_map/sparse_slot_map.rs) ✅ | `SparseSlotMap<U>` (ABA-fixed via tombstone+gen, M-016 closed) |
| Trait abstraction | [boyko_utils/sparse_map/sparse_collection.rs](../crates/boyko_utils/src/sparse_map/sparse_collection.rs) ✅ | `SparseCollection<K, V>` |
| Bitset (generic over storage word size) | [boyko_utils/bit_mask/bit_set.rs](../crates/boyko_utils/src/bit_mask/bit_set.rs) ✅ | `BitSet<T: BitInteger>` |
| Identifier primitives | [boyko_utils/identifiers/primitives.rs](../crates/boyko_utils/src/identifiers/primitives.rs), [boyko_utils/identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs) ✅ | `Generation`, `Slot` |

Removed in M-010 cleanup (Phase 5b): `bit_mask.rs`, `bit_set512.rs`,
`bit_storage.rs` were all fully wrapped in `/* */` comments and never
wired in — 1080 LOC of dead code dropped.

---

## What is NOT in the engine (deliberately)

| Missing | Why / where tracked |
|---------|--------------------|
| Per-entity iter with arity ≥ 3 | 📋 Phase 2d-extension |
| Per-entity mutable iter (`iter_mut_*`) | 📋 Phase 2d-extension |
| Generic tuple syntax `Query::iter::<(&A, &B)>` | 📋 Phase 2d-final (Bevy WorldQuery shape) |
| `spawn_three+` ergonomic API | 📋 Phase 2e-extension |
| Tuple-based `world.spawn((A{…}, B{…}))` generic | 📋 Phase 2e-extension |
| Event dispatch / reader / per-frame queue | 📋 Phase X (would also bring double-buffer event API to prevent inter-frame ABA on event payloads beyond what `Entity::generation` already provides) |
| `Changed<T>` / `Added<T>` / change detection | 📋 Phase 3+ (chunk `is_dirty` flag is the existing groundwork) |
| Parallel scheduler / system runner | 📋 Phase 4+ |
| Command buffer (deferred mutations during query iter) | 📋 Phase 3+ |
| ZST components | 📋 small Phase 2-future enhancement |
| Q-020 — split `Participants` / `Parameters` revisited | ❌ deliberately deferred — see `ROADMAP-PHASE-2-PLUS.md` Phase 4b rationale (no use case for participant-filtered dispatch in any committed phase) |

---

## Tests / benchmarks at a glance

| File | What it covers | Count |
|------|----------------|-------|
| `crates/boyko_ecs/src/**/*.rs` `#[cfg(test)]` | Unit tests inside every module | 174 (+ 2 ignored stress) |
| `tests/component_id_concurrency.rs` | Cross-thread ComponentId uniqueness (C-003) | 3 |
| `tests/derive_component.rs` | `#[derive(Component)]` macro output | 5 |
| `tests/drop_fn.rs` | Type-erased Drop glue (M-001 cont. / M-004) | 16 |
| `tests/drop_safety.rs` | EcsMaster Drop ordering (C-001) | 4 |
| `tests/event_attribute.rs` | `#[event]` macro round-trip | 11 |
| `tests/event_attribute_ui.rs` | Compile-fail UI tests for `#[event]` | 1 |
| boyko_utils unit tests | SparseMap + SparseSlotMap + Slot | 9 |
| Total | | **223 passing** + 4 ignored (2 stress + 2 doctests) |

Benchmarks (criterion, `cargo bench`):
- `component_id.rs` — `Component::component_id()` hot path (C-003 validation)
- `swap_remove.rs` — `EcsMaster::delete_entity` for N = 100 / 1k / 10k (M-004)
- `query_iter.rs` — `Query::with_component_ids` rebuild vs `QueryState` cache (Q-011, measured ~21× warm-path speedup)
- `archetype.rs` — `Archetype::create_entity` 2 / 8 component-id widths
- `allocator.rs` — `MemFreeBlockMaster::allocate_aligned` (M-012)

Miri verification: full lib test suite runs clean under `cargo +nightly miri test --package boyko-ecs --lib` (~2 h on Windows x86_64), confirms Stacked Borrows soundness of the raw-arena-pointer scheme (Phase 3a fix).
