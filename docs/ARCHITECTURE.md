# boyko-engine architecture (branch `ecs`)

> This documentation reflects the state of the **`ecs` branch**. Comparison with master is at the end.

## Goals and non-goals

**Goals:**
- Performance on par with state-of-the-art ECS engines (Bevy / flecs / Unity DOTS / EnTT)
- Cache locality via type-erased chunked storage + archetype-grouping of components
- Lock-free parallelism with work partitioning by chunks/archetypes
- Minimal per-entity / per-component footprint
- Zero-cost generics — no dynamic dispatch in the hot path

**Non-goals (at the current stage):**
- Scripting support (Lua, Wasm)
- Component hot-reload
- Serialization/deserialization (deferred until the model stabilizes)
- Cross-platform support beyond x86_64

## Workspace layout

```
boyko-engine/
├── Cargo.toml                            # workspace + main binary
├── src/main.rs                           # entry point (empty)
├── crates/
│   ├── boyko_ecs/                        # ECS core
│   │   ├── Cargo.toml                    # deps: rand, anyhow, boyko-utils
│   │   └── src/
│   │       ├── lib.rs                    # pub mod ecs;
│   │       └── ecs/
│   │           ├── mod.rs                # core, memory, constants, identifiers
│   │           ├── constants.rs          # sizes, alignment, thresholds
│   │           ├── identifiers/
│   │           │   └── primitives.rs     # type aliases: EntityId, ArchetypeId, ComponentId, ...
│   │           ├── core/
│   │           │   ├── component/        # type-erased: Component trait, ComponentMask, ComponentPoolBundle, ComponentRegistry
│   │           │   ├── entity/           # Entity, EntityInland, EntityMaster (recycling)
│   │           │   ├── archetype/        # Archetype, ArchetypeMaster, ArchetypeRegistry, ArchetypeSignature, ArchetypeBundle
│   │           │   ├── ecs_master/       # EcsMaster — top-level facade
│   │           │   ├── iters/            # Query, SparseIter, ComponentSet
│   │           │   ├── events/           # Event trait + EventPool/EventRegistry, Participants, Parameters
│   │           │   └── containers/tuple/ # ComponentTuple for batch operations
│   │           └── memory/
│   │               ├── arena.rs              # 64 MB arena with best-fit allocator
│   │               ├── free_mem_block.rs     # free-block tracker
│   │               ├── chunk.rs              # type-erased: metadata only (start_index, capacity, dirty)
│   │               ├── component_pool.rs     # type-erased: NonNull<u8> + Vec<Unit> + chunks
│   │               ├── id_unit.rs            # Unit { ptr: *mut u8, buffer_index }
│   │               ├── utils.rs              # align_up
│   │               ├── sparse_iter_component_pool.rs   # iterator over a pool
│   │               ├── multi_pool_sparse_iter.rs       # iterator across multiple pools
│   │               └── iterators.rs          # ⚠️ empty stub file
│   ├── boyko_macros/                     # proc-macros
│   │   ├── Cargo.toml                    # deps: syn, quote, proc-macro2, boyko-ecs
│   │   └── src/lib.rs                    # #[derive(Component)] + #[derive(Event)]
│   └── boyko_utils/                      # reusable collections
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── identifiers/
│           │   ├── primitives.rs         # Generation = usize
│           │   └── slot.rs               # Slot { index, generation }
│           ├── bit_mask/
│           │   ├── bit_storage.rs        # BitStorage trait
│           │   ├── bit_mask.rs           # BitMask<T: BitStorage>
│           │   ├── bit_set.rs            # BitSet<T: BitInteger> + iterator
│           │   └── bit_set512.rs         # BitSet512 — fixed 8×u64
│           └── sparse_map/
│               ├── sparse_collection.rs  # SparseCollection trait
│               ├── sparse_map.rs         # SparseMap<U>
│               └── sparse_slot_map.rs    # SparseSlotMap<U>
└── docs/                                 # internal documentation
```

## Inter-crate dependencies

```
boyko-engine (main binary)
    ├── boyko_ecs
    │       └── boyko_utils       ← new on ecs
    └── boyko_macros
            └── boyko_ecs         (for paths in macro-expanded code)
```

External dependencies:
- `boyko_ecs`: `rand`, `anyhow`, `boyko-utils`
- `boyko_macros`: `syn`, `quote`, `proc-macro2`, `boyko-ecs`
- `boyko_utils`: (none)

## Architecture layers

```
┌────────────────────────────────────────────────────────────────┐
│  Layer 4: Game/User Code (uses ECS API)                        │
└────────────────────────────────────────────────────────────────┘
                                ↑
┌────────────────────────────────────────────────────────────────┐
│  Layer 3: ECS API                                              │
│  EcsMaster, ArchetypeMaster, Query, Event, EventRegistry       │
└────────────────────────────────────────────────────────────────┘
                                ↑
┌────────────────────────────────────────────────────────────────┐
│  Layer 2: ECS Core                                             │
│  Entity, EntityMaster, EntityInland                            │
│  Archetype, ArchetypeRegistry, ArchetypeSignature              │
│  Component (trait + derive), ComponentMask, ComponentRegistry  │
│  Event (trait + derive), Participants, Parameters              │
└────────────────────────────────────────────────────────────────┘
                                ↑
┌────────────────────────────────────────────────────────────────┐
│  Layer 1: Type-Erased Memory                                   │
│  Arena → ComponentPool (type-erased) → Chunk → Unit            │
│  MemFreeBlockMaster (free-block tracker)                       │
└────────────────────────────────────────────────────────────────┘
                                ↑
┌────────────────────────────────────────────────────────────────┐
│  Layer 0: Utils (boyko_utils)                                  │
│  BitSet<T>, BitMask<T>, BitSet512                              │
│  SparseMap<U>, SparseSlotMap<U>                                │
│  Slot, identifiers/primitives                                  │
└────────────────────────────────────────────────────────────────┘
```

## Data flow when creating an entity

```
User → EcsMaster::create_entity(archetype_id, components)
    ├─ EntityMaster::allocate_entity()
    │      └─ either take from free_entity_ids,
    │         or bump next_entity_id → Entity { id, generation=0 }
    ├─ ArchetypeMaster::get_archetype_mut(id) → &mut Archetype
    ├─ Archetype::create_entity(entity_id, &mut inland, components)
    │      └─ for each pair (component_id, &[u8]):
    │             ComponentPoolBundle::get_pool_mut(component_id)
    │               → ComponentPool::add(...)
    │                   ├─ if needed: ComponentRegistry::get_layout(id)
    │                   ├─ on first allocation: arena.allocate_layout(...)
    │                   ├─ ptr::copy(src=bytes, dst=buffer + offset, size=layout.size())
    │                   └─ units.push(Unit { ptr, buffer_index })
    └─ EntityMaster::register_entity(entity, archetype_id, unit_index)
              └─ entity_map.insert(entity.id, EntityInland { archetype_id, unit_index, generation })
```

## Key architectural decisions

### 1. Type-erased `ComponentPool` (dropping generic `<T>`)

**Where:** [crates/boyko_ecs/src/ecs/memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs)

On master, `ComponentPool<T: Component>` was generic — one pool per type. On ecs, the pool stores **raw bytes** (`buffer: NonNull<u8>`, `buffer_capacity_bytes: usize`) and operates via `ComponentId` + `Layout` from the global `ComponentRegistry`.

**Why:**
- An archetype contains many **different** component types. With a generic pool you can't put `Vec<ComponentPool<?>>` without `Box<dyn Trait>` or an enum.
- Type erasure via `Layout` is the standard approach (Bevy `Table`, flecs `ecs_table_t`).

**Cost:**
- Each `add` / `get` loses compile-time type checking — correctness relies on the invariant that "ComponentId matches the right type".
- Component access requires `unsafe { &*(ptr as *const T) }` with a SAFETY comment.

### 2. Direct pointer in `Unit` instead of two-level addressing

**Where:** [crates/boyko_ecs/src/ecs/memory/id_unit.rs](../crates/boyko_ecs/src/ecs/memory/id_unit.rs)

On master there was `UnitId { chunk: u32, inland: u32 }` — 8 bytes, requiring address computation on every access. On ecs, `Unit { ptr: *mut u8, buffer_index: usize }` — 16 bytes, but component access is direct (`*ptr`).

**Trade-off:** doubling the index size in exchange for removing indirection on reads.

### 3. Global `ComponentRegistry` / `EventRegistry`

**Where:** [component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs), [event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs)

`static` storage with per-slot `OnceLock<ComponentLayout>` / `OnceLock<EventInfo>`. Registration is invoked lazily on first call to `T::component_id()` / `E::event_id()` from `#[derive(Component)]` / `#[derive(Event)]`-generated code. Each type carries a static `OnceLock<Id>` that memoizes the assigned ID after first initialization.

Component IDs are minted lazily on first call to `T::component_id()`; see the module-level docs of `crates/boyko_ecs/src/ecs/core/component/component_registry.rs` for the assignment algorithm, collision detection, and the startup warm-up contract for external-ID consumers. Event IDs follow the same model in `event_registry.rs`.

**Why:**
- Type erasure requires runtime metadata (size, align, TypeId).
- A single source of truth for all `ComponentPool` instances.
- Lazy per-type `OnceLock` eliminates `#[ctor::ctor]` startup registration and the associated fragile global-init ordering. The `ctor` dependency has been removed.

**Properties:**
- `ComponentId` is assigned in first-call order — stable for the process lifetime but not across processes. External-ID consumers must warm up the registry at startup.
- Per-slot collision detection panics immediately (both debug and release) if two types claim the same ID slot, preventing silent misidentification.

### 4. `EntityMaster` with recycling via a free list

**Where:** [entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs)

`free_entity_ids: Vec<EntityId>` for reusing slots. `entity_map: SparseMap<EntityInland>` for O(1) lookup by `EntityId`.

`Generation` is incremented on deallocation — preventing stale references.

### 5. `anyhow::Result` in `EcsMaster`

**Where:** [ecs_master.rs:9](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs)

`anyhow` is used to propagate top-level errors. ⚠️ A debatable choice for a library — `anyhow` is typically for applications. When stabilizing the API it's worth replacing with a domain-specific error type.

### 6. Adaptive chunk size based on component size

Same as on master — `TINY/SMALL/MEDIUM/LARGE_COMPONENTS_PER_CHUNK` (see [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs)).

## Multi-threading model (design goal)

Target model:
1. **Read-heavy parallelism**: multiple threads iterate over different `Query` instances in parallel — the Rust borrow checker, via a system scheduler, guarantees the absence of component access conflicts.
2. **Partitioned writes**: when one archetype is processed in parallel — divide chunks between threads (1 thread = 1+ chunk).
3. **Lock-free infrastructure**: allocations, registry, archetype access — via atomics.

Current state:
- `Arena` — `UnsafeCell<MemFreeBlockMaster>`, **not thread-safe** for multi-writer.
- `ComponentPool` mutability via `&mut self`.
- `ComponentRegistry` / `EventRegistry` — `static` storage, registration thread-safety needs verification.
- No scheduler yet.

## Performance goals

Target benchmarks (require validation via criterion benches after the build is fixed):

| Operation | Target | Notes |
|-----------|--------|-------|
| `Arena::allocate_aligned` (no fragmentation) | ≤ 50 ns | BTreeMap lookup + 2 HashMap ops |
| `ComponentPool::add` (space available in chunk) | ≤ 10 ns | Type-erased: pointer + memcpy + Vec::push(Unit) |
| Component access via `Unit::ptr` | ≤ 2 ns | Direct pointer dereference |
| Linear iteration over a pool | ~32 GB/s for tiny components | Sequential through buffer |
| `EcsMaster::create_entity` | ≤ 150 ns | EntityMaster + ArchetypeMaster + ComponentPool::add × N |
| Query construction (cached signature) | ≤ 50 ns | Archetype filter by mask |

These numbers are targets. No benchmarks exist yet.

## What differs from the `master` branch

| Aspect | master | ecs |
|--------|--------|-----|
| ComponentPool | `ComponentPool<T: Component>` (generic) | type-erased + `ComponentRegistry` |
| Chunk | `Chunk<T>` stores data | `Chunk` — metadata only (start_index, capacity, dirty) |
| Addressing | `UnitId { chunk: u32, inland: u32 }` | `Unit { ptr: *mut u8, buffer_index: usize }` |
| Entity ID | `u32` + generation `u16` | `usize` + generation `usize` |
| EntityMaster | ⚠️ missing | ✅ with recycling |
| Archetype | ⚠️ empty stub file | ✅ full implementation |
| EcsMaster | ⚠️ empty stub file | ✅ present, uses anyhow |
| Query | ⚠️ missing | ✅ present |
| Event subsystem | ⚠️ missing | ✅ present (with Participants + Parameters) |
| boyko_utils | ⚠️ missing | ✅ present (BitSet, SparseMap, Slot) |
| Build | ✅ builds | ❌ does not compile |
