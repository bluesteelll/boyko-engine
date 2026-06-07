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
│   │           │   ├── iters/            # Query, QueryState, ArchetypeBitSet, ComponentSet
│   │           │   ├── events/           # Event trait + EventPool/EventRegistry, Participants, Parameters
│   │           │   # (containers/tuple/ removed in Q-024 — orphan 0-byte stubs,
│   │           │   #  never wired into mod.rs; ComponentTuple planned as Phase 2e)
│   │           └── memory/
│   │               ├── arena.rs              # 64 MB arena with best-fit allocator
│   │               ├── free_mem_block.rs     # free-block tracker
│   │               ├── chunk.rs              # type-erased: metadata only (start_index, capacity, dirty)
│   │               ├── component_pool.rs     # type-erased: NonNull<u8> + len + chunks; row i at buffer+i*stride (X.B)
│   │               └── utils.rs              # align_up
│   │           # (Per-entity component iterators are intentionally absent. The old
│   │           #  sparse_iter / multi_pool_sparse_iter / sparse_iter_component_pool
│   │           #  files were removed in Phase 2c — see Phase 2d ticket for the
│   │           #  zero-alloc replacement built on top of QueryState.)
│   ├── boyko_macros/                     # proc-macros
│   │   ├── Cargo.toml                    # deps: syn, quote, proc-macro2, boyko-ecs
│   │   └── src/lib.rs                    # #[derive(Component)] + #[event]
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
│  Arena → ComponentPool (type-erased, row = buffer+i*stride)    │
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
User → EcsMaster::create_entity(archetype_id, &[(ComponentId, &[u8])])
    ├─ Guard (C-007): archetype_master.has_archetype(archetype_id)?
    │      └─ no → return Err(EcsError::ArchetypeNotFound) before any allocation
    ├─ EntityMaster::allocate_entity()
    │      └─ either take from free_entity_ids,
    │         or bump next_entity_id → Entity { id, generation }
    ├─ ArchetypeMaster::get_archetype_mut(id) → &mut Archetype
    ├─ Archetype::create_entity(entity_id, &mut inland, components)
    │      ├─ Two-phase commit (C-009): bundle.can_push_entity_components(...)
    │      │      └─ false → rewind_allocate(entity);
    │      │                  return Err(EcsError::ArchetypeRejectedEntity)
    │      └─ bundle.push_entity_components(...) — atomic across pools
    │             └─ for each (component_id, &[u8]):
    │                    ComponentPool::add(...) → memcpy into arena buffer
    └─ EntityMaster::register_entity(entity, archetype_id, unit_index)
              └─ entity_map.insert(entity.id, EntityInland { archetype_id, unit_index, generation })

Failure path correctness (C-007 + C-009): the guard means EntityIds are never
leaked on early rejection; the two-phase commit means a partial push (some
pools succeeded, some failed) is structurally impossible — `can_push` walks
every pool in read-only mode first, then `push` only runs after universal
go-ahead.
```

After the fast-store registration, the same structural op fires component
lifecycle **hooks** (Phase 14a) and then **observers** (Phase 14b) for each
`add` / `insert` kind — gated by the archetype's `ArchetypeFlags` `u16`
bit-test, so a world with no callback pays one `test`/`jz`. The symmetric
`replace` / `remove` fire happens on despawn / migration. The full catalog
(registry, the 4 cold `fire_*_observers` dispatch fns, the 7 fire sites, and
the OBS-FIRE-LOOP Tree-Borrows invariant) is in [SYSTEMS.md §3.6](SYSTEMS.md).

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

### 2. Computed row addressing (`buffer + i*stride`)

**Where:** [crates/boyko_ecs/src/ecs/memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs)

On `master` there was `UnitId { chunk: u32, inland: u32 }` — two-level addressing. The `ecs` branch first cached a direct `Unit { ptr: *mut u8 }` per row; **Phase X.B then eliminated even that cache**: rows are addressed by `buffer.as_ptr().add(i * stride)` (the private `row_ptr(i)`) from the pool's stable, write-once arena base, with `len` the live-row count.

**Trade-off:** the per-row `Vec<Unit>` was pure redundancy (every entry equalled `buffer + i*stride`), so removing it saves 8 B/row + one heap allocation per pool and **net-removes `unsafe`**, with zero read-path cost (the recompute is one multiply+add; the hot iteration / random-access paths already used `column.ptr.add`). Behavior-preserving + Miri-clean. See [PHASE-XB-RESULTS.md](PHASE-XB-RESULTS.md).

### 3. Global `ComponentRegistry` / `EventRegistry`

**Where:** [component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs), [event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs)

`static` storage with per-slot `OnceLock<ComponentLayout>` / `OnceLock<EventInfo>`. Registration is invoked lazily on first call to `T::component_id()` / `E::event_id()` from `#[derive(Component)]` / `#[event]`-generated code. Each type carries a static `OnceLock<Id>` that memoizes the assigned ID after first initialization.

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

### 5. Domain error type `EcsError`

**Where:** [error.rs](../crates/boyko_ecs/src/ecs/error.rs)

`#[non_exhaustive] pub enum EcsError` + `pub type EcsResult<T>` — replaces the historical `anyhow::Result` from `EcsMaster` (C-019 closed). Variants: `ArchetypeNotFound`, `EntityNotFound`, `ComponentPoolFull`, `UnknownComponentForArchetype`, `ArchetypeRejectedEntity`, `PoolSwapRemoveFailed`. Hand-rolled `Display` + `std::error::Error` — no `thiserror` dep.

`#[non_exhaustive]` keeps the door open for new variants without major-version bumps. Callers can pattern-match on the concrete variant (the whole point of switching off `anyhow`'s erased `Error`).

### 6. Newtype identifiers

**Where:** [identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs)

Every ID type (`EntityId`, `ArchetypeId`, `ComponentId`, `ChunkId`, `InlandPoolId`, etc.) is a `#[repr(transparent)] pub struct X(pub usize)` newtype defined via a single `define_id!` macro (C-017 closed). Zero runtime cost, but the compiler refuses to mix them: `archetype.has_component_id(entity.id())` is a type error.

`Generation` stays a `type alias = usize` — it is only ever paired with an `EntityId` inside `Entity` and never crossed with another ID kind.

### 7. Dual-generation QueryState (ABA-safety across remove/clear)

**Where:** [archetype/archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs), [iters/query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs)

`ArchetypeMaster` carries two monotonic counters:
- `generation` — bumps on every `create_archetype` (creation deltas).
- `structural_generation` — bumps on `remove_archetype` and `clear()` (structural changes).

`QueryState` snapshots both. On `iter()`:
- Structural mismatch → drop the dedup bitset + matched_ids, **full rebuild** (reclassify every live archetype). This is the load-bearing piece — without it, a freshly created archetype reusing a recycled `ArchetypeId` (after `clear()` resets `next_archetype_id`) would be skipped by the stale bitset and silently absent from results.
- Creation-only delta → original delta-add path (skip already-classified IDs via bitset).
- Both equal → warm path (one comparison + slice walk).

This eliminates the ArchetypeId-ABA hazard while keeping the ~21× Q-011 warm-path speedup (Phase 5c).

### 8. Adaptive chunk size based on component size

Same as on master — `TINY/SMALL/MEDIUM/LARGE_COMPONENTS_PER_CHUNK` (see [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs)).

## Multi-threading model (design goal)

Target model:
1. **Read-heavy parallelism**: multiple threads iterate over different `Query` instances in parallel — the Rust borrow checker, via a system scheduler, guarantees the absence of component access conflicts.
2. **Partitioned writes**: when one archetype is processed in parallel — divide chunks between threads (1 thread = 1+ chunk).
3. **Lock-free infrastructure**: allocations, registry, archetype access — via atomics.

Current state:
- `Arena` — `UnsafeCell<MemFreeBlockMaster>` + `!Send + !Sync`, **single-threaded by construction**.
- `ComponentPool` mutability via `&mut self`.
- `ComponentRegistry` / `EventRegistry` — `static [OnceLock<...>; N]` + `AtomicUsize` counter — lock-free, cross-thread safe (M-002 / C-002 / Q-004 / Q-010 closed).
- No system scheduler yet — planned Phase 4+.

## Performance goals

Targets (with the current `criterion` harness in [benches/](../crates/boyko_ecs/benches/) — see [SYSTEMS.md §14](SYSTEMS.md#14-benchmarks)):

| Operation | Target | Status |
|-----------|--------|--------|
| `Arena::allocate_aligned` (no fragmentation) | ≤ 50 ns | benched in `allocator.rs` (M-012 validation) |
| `ComponentPool::add` (space available) | ≤ 10 ns | benched implicitly in `archetype.rs` create paths |
| Component access via `row_ptr` (`buffer + i*stride`) | ≤ 2 ns | computed offset + deref (X.B) |
| Linear iter over a pool | ~32 GB/s for tiny components | per-entity `Query::iter_one/iter_two` (Phase 2d) is the foundation |
| `EcsMaster::create_entity` | ≤ 150 ns | benched in `archetype.rs` for 2 / 8 component widths |
| `QueryState` warm-path iter | ≤ 5 ns | **measured ~3.6 ns** in `query_iter.rs` (vs ~77 ns one-shot Query construction → ~21× speedup, Q-011 closed) |

## What differs from the `master` branch

| Aspect | master | ecs |
|--------|--------|-----|
| ComponentPool | `ComponentPool<T: Component>` (generic) | type-erased + `ComponentRegistry` |
| Chunk | `Chunk<T>` stores data | `Chunk` — metadata only (start_index, capacity, dirty) |
| Addressing | `UnitId { chunk: u32, inland: u32 }` | computed `buffer + i*stride` (`row_ptr(i)`; the cached `Unit { ptr }` was removed in Phase X.B) |
| Entity ID | `u32` + generation `u16` | newtype `EntityId(usize)` + generation `usize` (C-017 closed) |
| EntityMaster | ⚠️ missing | ✅ id recycling + direct `Vec<EntityInland>` fast store (O(capacity) `iter_entities` after Phase X.D slot reduction) |
| Archetype | ⚠️ empty stub file | ✅ full implementation |
| EcsMaster | ⚠️ empty stub file | ✅ present, returns domain `EcsResult` (C-019 closed) |
| Query | ⚠️ missing | ✅ archetype-level cached query; per-entity `Query::<(&T, &U)>::iter()` is Phase 2d-open |
| Event subsystem | ⚠️ missing | ✅ present (with Participants + Parameters; TypeId-guarded buffers per Q-019) |
| boyko_utils | ⚠️ missing | ✅ present (BitSet, SparseMap, SparseSlotMap with M-016 ABA fix, Slot) |
| Build | ✅ builds | ✅ builds clean (check + clippy + test + miri all green) |
