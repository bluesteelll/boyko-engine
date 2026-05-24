# Feature map — where to find what (branch `ecs`)

First point of contact for agents. When looking for where a particular piece of functionality is implemented, check here first. For details go to [SYSTEMS.md](SYSTEMS.md), then to the code.

**Legend:**
- ✅ Implemented
- ⚠️ Implemented, but has issues / build does not pass
- 📋 Planned, not yet written

> ⚠️ The branch build is currently broken. Most features are implemented in code, but can't be run until the build is fixed.

---

## Memory and allocation

| What you want to do | Where to look | Method / type |
|---------------------|---------------|---------------|
| Allocate a block of N bytes | [memory/arena.rs](../crates/boyko_ecs/src/ecs/memory/arena.rs) ✅ | `Arena::allocate_layout(layout)` |
| Allocate with alignment | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::allocate_aligned(size, align)` |
| Find best-fit free block | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::find_best_fit` |
| Return memory to the pool | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::insert` (with automatic merging) |
| Align an address/size | [memory/utils.rs](../crates/boyko_ecs/src/ecs/memory/utils.rs) ✅ | `align_up(value, alignment)` |
| Free the arena | — ⚠️ | `impl Drop for Arena` **is missing** (leak) |
| Defragment the free-block list | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::defragment` |
| Get memory statistics | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::get_memory_stats` |

---

## Type-erased component storage

| What you want to do | Where | Method |
|---------------------|-------|--------|
| Create a pool for a component with a known ComponentId | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::new(arena, component_id, num_chunks, components_per_chunk)` |
| Add a component to the pool (via byte slice) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::add(...)` |
| Get a component by index | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::get(...)` |
| Remove a component (swap) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::swap_remove(...)` |
| Get component size/alignment from the registry | [core/component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs) ✅ | `get_layout`, `get_component_size`, `get_component_alignment` |

---

## Chunks (metadata windows into the pool's buffer)

| What | Where | Method |
|------|-------|--------|
| Create chunk metadata | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::new(start_index, capacity)` |
| Get start_index in the pool's buffer | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::start_index()` |
| Chunk capacity | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::capacity()` |
| Mark a chunk as modified | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::mark_dirty()` |
| Check the change flag | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::is_dirty()` |
| Clear the change flag | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::clear_dirty_flag()` |

> Note: `Chunk` now stores metadata only, no data. The data lives in `ComponentPool::buffer`.

---

## Direct component pointer (Unit)

| What | Where | Method |
|------|-------|--------|
| Create a Unit | [memory/id_unit.rs](../crates/boyko_ecs/src/ecs/memory/id_unit.rs) ✅ | `Unit::new(ptr, buffer_index)` |
| Get the pointer | [memory/id_unit.rs](../crates/boyko_ecs/src/ecs/memory/id_unit.rs) ✅ | `Unit::ptr()` |
| Get the position in the buffer | [memory/id_unit.rs](../crates/boyko_ecs/src/ecs/memory/id_unit.rs) ✅ | `Unit::buffer_index()` |

---

## Component iteration

| What | Where | Type/method |
|------|-------|-------------|
| Archetype-level cached query (matched-archetype iter) | [core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs), [core/iters/query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs) ✅ | `Query` / `QueryState` / `QueryStateIter` |
| Per-entity tuple iter (`Query::<(&T, &U)>::iter()`) | — | ❌ **not implemented** — see Phase 2d in `docs/ROADMAP-PHASE-2-PLUS.md`. Previous orphan implementation (`sparse_iter`, `multi_pool_sparse_iter`, `sparse_iter_component_pool`) was removed in Phase 2c (per-entity heap alloc, recursive next, undefined typed adapters). |

---

## Components

| What you want to do | Where | How |
|---------------------|-------|-----|
| Define a new component | [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) ✅ | `#[derive(Component)] struct MyComp { ... }` |
| Get the unique ID for a component type | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::component_id()` |
| Get the component size (compile-time) | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::mem_size()` |
| Get alignment | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::alignment()` |
| Get TypeId | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::type_id()` |
| Get the type name | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::debug_type_name()` |
| Register a component layout | [core/component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs) ✅ | `register_layout::<T>(component_id)` (invoked by the macro) |
| Get a layout from the registry | [core/component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs) ✅ | `get_layout(id)`, `get_layout_unchecked(id)` |
| Bitmask of a component set | [core/component/component_mask.rs](../crates/boyko_ecs/src/ecs/core/component/component_mask.rs) ✅ | `ComponentMask` (built on `BitSet512`) |
| Bundle pools of different types for one archetype | [core/component/component_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs) ✅ | `ComponentPoolBundle` |
| Tuple-based bundle | — | ❌ **not implemented** — see Phase 2e in `docs/ROADMAP-PHASE-2-PLUS.md`. The orphan `containers/tuple/` directory (empty files, unwired from `mod.rs`) was removed in Q-024. |

---

## Entities

| What you want to do | Where | How |
|---------------------|-------|-----|
| Create an Entity directly | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ | `Entity::new(id, generation)` |
| Create with generation = 0 | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ | `Entity::with_id(id)` |
| Compare two Entities (id + generation) | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ | `e1 == e2` |
| Allocate an entity / reuse an ID | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::allocate_entity()` |
| Register an entity in an archetype | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::register_entity(entity, archetype_id, unit_index)` |
| Get EntityInland (metadata) | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::get_entity_inland(entity)` |
| Update unit_index after a swap | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::update_entity_unit_index(entity, new_idx)` |
| Delete an entity | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::deallocate_entity(entity)` → `Option<EntityInland>` |
| Check entity validity (id+generation) | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::is_entity_valid(entity)` |
| Iterate over active entities | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::iter_entities()` |

---

## Archetypes

| What you want to do | Where | How |
|---------------------|-------|-----|
| Create an archetype from a set of `ComponentId` | [core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs) ✅ | `Archetype::create_by_ids(id, &[ComponentId], &arena)` |
| Manage all archetypes | [core/archetype/archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs) ✅ | `ArchetypeMaster` |
| Create / get an archetype by components | [core/archetype/archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs) ✅ | `ArchetypeMaster::get_or_create_archetype(&[ComponentId])` |
| Find an archetype by signature/mask | [core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs) ✅ | `ArchetypeRegistry::find_exact_match(&ComponentMask)` |
| Bitmask "which components make it up" | [core/archetype/archetype_signature.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_signature.rs) ✅ | `ArchetypeSignature` (built on `ComponentMask`) |
| Component bundle for batch operations | [core/archetype/archetype_bundle.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs) ✅ | `ArchetypeBundle` |
| Create an entity in an archetype | [core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs) ✅ | `Archetype::create_entity(entity_id, &mut EntityInland, Vec<(ComponentId, &[u8])>)` |
| Remove an entity from an archetype | [core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs) ✅ | `Archetype::remove_entity(&EntityInland)` |

---

## ECS top-level API

| What | Where |
|------|-------|
| Create an ECS world | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `EcsMaster::new()` |
| Create with pre-allocated capacity | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `EcsMaster::with_capacity(entities, archetypes)` |
| Create an entity with components | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `EcsMaster::create_entity(archetype_id, Vec<(ComponentId, &[u8])>)` → `anyhow::Result<Entity>` |
| Delete an entity | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `EcsMaster::delete_entity(entity)` |
| Create/get an archetype | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `EcsMaster::get_or_create_archetype(&[ComponentId])` |

---

## Queries

| What | Where |
|------|-------|
| Query by ComponentId | [core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs) ✅ | `Query::with_component_ids(master, &[ComponentId])` |
| Query by mask (inclusive) | [core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs) ✅ | `Query::with_mask(master, &mask)` |
| Query with exact mask match | [core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs) ✅ | `Query::with_exact_mask(master, &mask)` |
| Query from a ready-made set of archetypes | [core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs) ✅ | `Query::from_archetypes(Vec<&Archetype>, master)` |
| **Cached persistent query (Q-011)** | [core/iters/query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs) ✅ | `QueryState` — warm-path ~21x faster than one-shot `Query` |
| Warm-path delta update | [core/iters/query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs) ✅ | `QueryState::update_archetypes(&master)` |
| Generation-aware stale detection | [core/archetype/generation.rs](../crates/boyko_ecs/src/ecs/core/archetype/generation.rs) ✅ | `ArchetypeGeneration` + `ArchetypeMaster::archetype_generation()` |
| 1024-bit inline dedup bitset | [core/iters/archetype_bit_set.rs](../crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs) ✅ | `ArchetypeBitSet` (128 B, 2 cache lines, no heap) |
| Tuple trait for query parameters | [core/iters/component_set.rs](../crates/boyko_ecs/src/ecs/core/iters/component_set.rs) ✅ | `ComponentSet::component_ids() -> &'static [ComponentId]` (Q-012) |

---

## Events

| What | Where | How |
|------|-------|-----|
| Define an event | [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) ✅ | `#[event] struct MyEvent { #[participant(...)] e: Entity, #[parameter] x: f32 }` |
| Event trait | [core/events/event.rs](../crates/boyko_ecs/src/ecs/core/events/event.rs) ✅ | `Event` with `type Participants`, `type Parameters` |
| Register an event | [core/events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs) ✅ | `register_event::<E>(event_id)` |
| Get metadata | [core/events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs) ✅ | `get_event_info(id)`, `get_event_layout(id)` |
| Validate event types | [core/events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs) ✅ | `validate_event_types::<E>(id)` |
| Event pool | — ❌ not implemented — see future event-pool ticket | `EventPool` |
| Bundle of heterogeneous pools | — ❌ not implemented — see future event-pool ticket | `EventPoolBundle` |
| Participants trait (Copy-bounded) | [core/events/participants/participants.rs](../crates/boyko_ecs/src/ecs/core/events/participants/participants.rs) ✅ | `Participants: Copy`, `ParticipantInfo` |
| Buffer for participants | [core/events/participants/participants_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/participants/participants_buffer.rs) ✅ | `ParticipantBuffer` (storage: `Vec<MaybeUninit<u8>>`) |
| Parameters trait (Copy-bounded) | [core/events/parameters/parameters.rs](../crates/boyko_ecs/src/ecs/core/events/parameters/parameters.rs) ✅ | `Parameters: Copy` |
| Buffer for parameters | [core/events/parameters/parameters_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/parameters/parameters_buffer.rs) ✅ | `ParametersBuffer` (storage: `Vec<MaybeUninit<u8>>`) |

---

## Bit operations (boyko_utils)

| What | Where | Type |
|------|-------|------|
| Generic bitset | [boyko_utils/src/bit_mask/bit_set.rs](../crates/boyko_utils/src/bit_mask/bit_set.rs) ✅ | `BitSet<T: BitInteger>` |
| Bit iterator | [boyko_utils/src/bit_mask/bit_set.rs](../crates/boyko_utils/src/bit_mask/bit_set.rs) ✅ | `BitSetIterator<T>` |
| Fixed 512-bit (8×u64) | [boyko_utils/src/bit_mask/bit_set512.rs](../crates/boyko_utils/src/bit_mask/bit_set512.rs) ✅ | `BitSet512` |
| Component mask (built on BitSet512) | [core/component/component_mask.rs](../crates/boyko_ecs/src/ecs/core/component/component_mask.rs) ✅ | `ComponentMask` |
| Generic BitMask | [boyko_utils/src/bit_mask/bit_mask.rs](../crates/boyko_utils/src/bit_mask/bit_mask.rs) ✅ | `BitMask<T: BitStorage>` |
| Trait for bit storage | [boyko_utils/src/bit_mask/bit_storage.rs](../crates/boyko_utils/src/bit_mask/bit_storage.rs) ✅ | `BitStorage` |

---

## Sparse maps (boyko_utils)

| What | Where | Type |
|------|-------|------|
| Generic sparse map | [boyko_utils/src/sparse_map/sparse_map.rs](../crates/boyko_utils/src/sparse_map/sparse_map.rs) ✅ | `SparseMap<U>` |
| Sparse slot map with generation | [boyko_utils/src/sparse_map/sparse_slot_map.rs](../crates/boyko_utils/src/sparse_map/sparse_slot_map.rs) ✅ | `SparseSlotMap<U>` |
| Trait for sparse collections | [boyko_utils/src/sparse_map/sparse_collection.rs](../crates/boyko_utils/src/sparse_map/sparse_collection.rs) ⚠️ | `SparseCollection<K, V>` (declared, but unused) |

---

## Identifiers / Slots

| What | Where | Type |
|------|-------|------|
| All ID types in boyko_ecs | [boyko_ecs/src/ecs/identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs) ✅ | `EntityId`, `ArchetypeId`, `ComponentId`, ... |
| Generation | [boyko_utils/src/identifiers/primitives.rs](../crates/boyko_utils/src/identifiers/primitives.rs) ✅ | `Generation = usize` |
| Slot { index, generation } | [boyko_utils/src/identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs) ✅ | `Slot` |

---

## Systems and scheduler 📋

**Not implemented** — the next major feature. When it lands, this section will include:

- System registration
- Dependency graph
- Parallel execution
- Stage/phase API

---

## Resources / global state 📋

**Not implemented** — analog of `Resource` in Bevy / singletons in Unity.

---

## Change detection 📋

**Not implemented** as a full-fledged feature. There is a stub — the `Chunk::is_dirty` flag.

---

## Serialization 📋

**Not implemented** — deferred.

---

## Tests and benchmarks

| What | Where |
|------|-------|
| Unit tests | ⚠️ only present in [entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) (4 tests). Coverage is minimal. |
| Integration tests | 📋 **missing** — target: `crates/boyko_ecs/tests/*.rs` |
| Benchmarks | 📋 **missing** — target: `crates/boyko_ecs/benches/*.rs` (via `criterion`) |
| Loom tests for lock-free | 📋 **missing** |
| Property-based (proptest) | 📋 **missing** |

⚠️ **Running any tests is currently impossible — the build does not pass.**

---

## Style and infrastructure

| What | Where |
|------|-------|
| Workspace config | [Cargo.toml](../Cargo.toml) |
| ECS core Cargo.toml | [crates/boyko_ecs/Cargo.toml](../crates/boyko_ecs/Cargo.toml) |
| Proc-macro Cargo.toml | [crates/boyko_macros/Cargo.toml](../crates/boyko_macros/Cargo.toml) |
| Utils Cargo.toml | [crates/boyko_utils/Cargo.toml](../crates/boyko_utils/Cargo.toml) |
| All engine constants | [crates/boyko_ecs/src/ecs/constants.rs](../crates/boyko_ecs/src/ecs/constants.rs) |
| Agent rules | [../CLAUDE.md](../CLAUDE.md) |
| Architecture overview | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Detailed systems catalog | [SYSTEMS.md](SYSTEMS.md) |
| TODO list (by author) | [TODOI.md](TODOI.md) |

---

## Cheat sheet: "couldn't find it — where to look?"

1. **Is it about memory / allocation / Arena?** → "Memory" section + [SYSTEMS.md §2](SYSTEMS.md)
2. **Is it about type-erased ComponentPool / Unit / Chunk?** → sections above + [SYSTEMS.md §2.3-2.6](SYSTEMS.md)
3. **Is it about components / Component derive / Registry?** → "Components" section + [SYSTEMS.md §3](SYSTEMS.md)
4. **Is it about entities / EntityMaster / generation?** → "Entities" section + [SYSTEMS.md §4](SYSTEMS.md)
5. **Is it about archetypes?** → "Archetypes" + [SYSTEMS.md §5](SYSTEMS.md)
6. **Is it about the top-level API (EcsMaster)?** → "ECS top-level API" + [SYSTEMS.md §6](SYSTEMS.md)
7. **Is it about query / iteration?** → "Queries" + [SYSTEMS.md §7](SYSTEMS.md)
8. **Is it about events?** → "Events" + [SYSTEMS.md §8](SYSTEMS.md)
9. **Is it about BitSet / SparseMap / boyko_utils?** → "Bit operations" / "Sparse maps" sections + [SYSTEMS.md §10](SYSTEMS.md)
10. **Doesn't seem to exist at all?** → check the 📋 "Planned" section above. If it's not there either — nobody has described the feature yet, that's `architect`'s job.
11. **Build broken, can't verify?** → task `cargo check ecs` in TaskList. See also [TODOI.md](TODOI.md).
