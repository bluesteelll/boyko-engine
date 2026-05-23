# boyko-engine systems catalog (branch `ecs`)

Reference for every subsystem, listing code locations, key types, methods, and invariants. Used by agents for navigation.

**Status legend:**
- ✅ Implemented
- ⚠️ Present, but with issues / incomplete / does not compile
- 📋 Planned

> ⚠️ The `ecs` branch currently **does not compile**. The descriptions below reflect both the intent and the actual code, but running/testing is blocked by the build.

---

## 1. Identifiers (ID types) ✅

**Files:**
- [crates/boyko_ecs/src/ecs/identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs)
- [crates/boyko_utils/src/identifiers/primitives.rs](../crates/boyko_utils/src/identifiers/primitives.rs)
- [crates/boyko_utils/src/identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs)

All IDs are unified as `usize`:
```rust
pub type EntityId            = usize;
pub type ArchetypeId         = usize;
pub type ChunkId             = usize;
pub type InlandChunkId       = usize;
pub type ComponentId         = usize;
pub type InlandUnitId        = usize;
pub type InlandPoolId        = usize;
pub type InlandComponentId   = usize;
pub type InlandArchetypeId   = usize;
pub type Generation          = usize;
```

`Slot` (in boyko_utils):
```rust
pub struct Slot {
    index: usize,
    generation: Generation,
}
```
Used as a "shared key" for sparse-map structures. `Entity` implements `From<Slot> + Into<Slot>`.

---

## 2. Memory subsystem ✅

### 2.1. Arena ✅

**File:** [crates/boyko_ecs/src/ecs/memory/arena.rs](../crates/boyko_ecs/src/ecs/memory/arena.rs)

Same as on master — a 64 MB pre-allocated arena with `MemFreeBlockMaster` for best-fit allocation.

```rust
pub struct Arena {
    ptr: NonNull<u8>,
    capacity: usize,
    cursor: UnsafeCell<usize>,         // ⚠️ unused
    layout: Layout,
    free_blocks: UnsafeCell<MemFreeBlockMaster>,
}
```

### 2.2. Chunk (type-erased) ✅

**File:** [crates/boyko_ecs/src/ecs/memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs)

**Sharply changed vs master.** Now it's simply a metadata structure, with no data and no `<T>`:

```rust
pub struct Chunk {
    start_index: usize,    // position in the pool's shared buffer
    capacity: usize,
    is_dirty: bool,        // change flag (for change detection)
}
```

Data lives in `ComponentPool::buffer`; chunks are "windows" into that buffer.

### 2.3. ComponentPool (type-erased) ✅

**File:** [crates/boyko_ecs/src/ecs/memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs)

```rust
pub struct ComponentPool {
    arena: NonNull<Arena>,
    buffer: NonNull<u8>,                  // single allocated buffer for all components
    buffer_capacity_bytes: usize,
    max_components: usize,
    units: Vec<Unit>,                     // densely packed direct pointers
    pub chunks: Vec<Chunk>,               // metadata for windows into buffer
    components_per_chunk: usize,
    component_id: usize,
    component_layout: Layout,             // size + align from ComponentRegistry
}
```

Key idea: the pool allocates **one large block** in the arena, then hands out slots inside it to components. `units` is a densely packed array of direct pointers into `buffer`. On swap_remove the last `Unit` is moved.

### 2.4. MemFreeBlockMaster ✅

**File:** [crates/boyko_ecs/src/ecs/memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs)

Same as on master. `BTreeMap<size, Vec<idx>>` + `start_map`/`end_map` for O(1) merging of adjacent blocks.

### 2.5. Unit ✅

**File:** [crates/boyko_ecs/src/ecs/memory/id_unit.rs](../crates/boyko_ecs/src/ecs/memory/id_unit.rs)

```rust
pub struct Unit {
    ptr: *mut u8,          // direct pointer into ComponentPool::buffer
    buffer_index: usize,   // position (for bounds-checking / returning to the pool)
}
```

Replaces `UnitId` from master. **Not Sync/Send** by default because of `*mut u8`.

### 2.6. Iterators ✅

- [sparse_iter_component_pool.rs](../crates/boyko_ecs/src/ecs/memory/sparse_iter_component_pool.rs) — `ComponentPoolSparseIter`, `ComponentPoolSparseIterMut`, `ComponentPtr`, `ComponentMutPtr`
- [multi_pool_sparse_iter.rs](../crates/boyko_ecs/src/ecs/memory/multi_pool_sparse_iter.rs) — `MultiPoolSparseIter`, `MultiPoolSparseIterMut` for simultaneous iteration over multiple pools (components of a single entity)
- [iterators.rs](../crates/boyko_ecs/src/ecs/memory/iterators.rs) — ⚠️ **empty file** (stub)

---

## 3. Component subsystem ✅

### 3.1. Component trait

**File:** [crates/boyko_ecs/src/ecs/core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs)

```rust
pub trait Component: 'static + Sized {
    fn component_id() -> ComponentId;
    fn debug_type_name() -> &'static str { type_name::<Self>() }
    fn type_id() -> TypeId { TypeId::of::<Self>() }
    fn mem_size() -> usize { size_of::<Self>() }
    fn alignment() -> usize { align_of::<Self>() }
}
```

`mem_size()` was renamed from `size()` on master.

⚠️ All methods carry `#[inline(always)]`, which triggers a warning in newer Rust versions for required trait methods (see the build notes below).

### 3.2. ComponentMask

**File:** [crates/boyko_ecs/src/ecs/core/component/component_mask.rs](../crates/boyko_ecs/src/ecs/core/component/component_mask.rs)

A high-level wrapper over `BitSet512` for the "which components an archetype contains" mask.

### 3.3. ComponentPoolBundle

**File:** [crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs)

A collection of type-erased `ComponentPool`s within one archetype (one per `ComponentId`). Has `swap_remove_unit` returning `anyhow::Result<()>`.

### 3.4. ComponentRegistry (global static)

**File:** [crates/boyko_ecs/src/ecs/core/component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs)

Global storage for `ComponentLayout { layout: Layout, type_name, ... }`.

API:
- `register_layout<T: 'static>(component_id)` — invoked by the macro during `#[derive(Component)]`
- `get_layout(component_id) -> Option<&'static ComponentLayout>`
- `get_layout_unchecked(component_id) -> &'static ComponentLayout` (unsafe fast path)
- `get_component_size`, `get_component_alignment`, `get_component_memory_layout`

### 3.5. `#[derive(Component)]` macro

**File:** [crates/boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs)

Uses an `AtomicUsize` counter to assign `ComponentId`. Generates:
- `impl Component for T { fn component_id() -> ComponentId { N } }`
- Registers the layout in the registry (via `register_layout::<T>(N)`)

⚠️ `ComponentId` is unstable across builds (depends on macro expansion order).

---

## 4. Entity subsystem ✅

### 4.1. Entity

**File:** [crates/boyko_ecs/src/ecs/core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs)

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Entity {
    pub id: EntityId,        // usize
    pub generation: usize,
}
```

Implements `From<Slot> + Into<Slot>` for compatibility with sparse collections.

### 4.2. EntityInland

**File:** [crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs)

Internal representation known to `EntityMaster`:
```rust
pub struct EntityInland {
    archetype_id: ArchetypeId,
    unit_index: InlandPoolId,    // entity position within the archetype
    generation: Generation,
}
```

### 4.3. EntityMaster

**File:** [crates/boyko_ecs/src/ecs/core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs)

```rust
pub struct EntityMaster {
    free_entity_ids: Vec<EntityId>,           // for recycling
    entities: Vec<Entity>,                    // all entities (including deleted)
    entity_map: SparseMap<EntityInland>,      // O(1) lookup by EntityId
    next_entity_id: EntityId,
    active_count: usize,
}
```

Methods: `allocate_entity`, `register_entity`, `update_entity_inland`, `update_entity_unit_index`, `deallocate_entity`, `get_entity_inland(_mut)`, `is_entity_valid`, `iter_entities`, `clear`, `compact`, `memory_usage`.

There are unit tests at the bottom of the file (`test_entity_allocation`, `test_entity_registration`, `test_entity_deallocation_and_reuse`, `test_entity_inland_update`).

---

## 5. Archetype subsystem ✅

### 5.1. Archetype

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs)

```rust
pub struct Archetype {
    id: ArchetypeId,
    component_pools: ComponentPoolBundle,
    current_index: usize,
    signature: ArchetypeSignature,
    arena: NonNull<Arena>,
    component_ids: Vec<ComponentId>,
    entity_ids: Vec<EntityId>,                // indexed by unit_index
}
```

Key methods: `new(id, &arena)`, `create_by_ids(id, &[ComponentId], &arena)`, `register_component`, `create_entity`, `remove_entity`, `init_entity_inland`, `id()`.

### 5.2. ArchetypeSignature

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_signature.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_signature.rs)

```rust
pub struct ArchetypeSignature {
    pub mask: ComponentMask,
}
```

The archetype's component mask. Used for `Query` matching.

### 5.3. ArchetypeBundle

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs)

Bundle of components for batch operations (creating several entities at once).

### 5.4. ArchetypeRegistry

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs)

Archetype registry, lookup by `ComponentMask`/`ArchetypeSignature`. Methods: `find_exact_match`, etc.

### 5.5. ArchetypeMaster

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs)

Top-level archetype manager. Methods: `new`, `with_capacity`, `create_archetype`, `get_or_create_archetype`, `get_archetype(_mut)`, `find_archetypes_with_components`, `find_matching_archetypes`, `archetype_registry`.

---

## 6. EcsMaster (top-level API) ✅

**File:** [crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs)

```rust
pub struct EcsMaster {
    entity_master: EntityMaster,
    archetype_master: ArchetypeMaster,
    arena: Arena,
}
```

API:
- `new() -> Self`
- `with_capacity(entity_capacity, archetype_capacity) -> Self`
- `create_archetype(component_ids) -> ArchetypeId`
- `get_or_create_archetype(component_ids) -> ArchetypeId`
- `create_entity(archetype_id, Vec<(ComponentId, &[u8])>) -> anyhow::Result<Entity>`
- `delete_entity(entity) -> bool`

⚠️ Uses `anyhow::Result` — debatable for a library API, to be revisited when stabilizing.

---

## 7. Query subsystem ✅

### 7.1. Query

**File:** [crates/boyko_ecs/src/ecs/core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs)

```rust
pub struct Query<'a> {
    archetypes: Vec<&'a Archetype>,
}
```

Constructors: `from_archetypes`, `with_component_ids`, `with_mask`, `with_exact_mask`.

Stores direct references to `&Archetype` — maximum perf during iteration, no indirection.

### 7.2. SparseIter / SparseIterMut

**File:** [crates/boyko_ecs/src/ecs/core/iters/sparse_iter.rs](../crates/boyko_ecs/src/ecs/core/iters/sparse_iter.rs)

Iterators over query results.

### 7.3. ComponentSet

**File:** [crates/boyko_ecs/src/ecs/core/iters/component_set.rs](../crates/boyko_ecs/src/ecs/core/iters/component_set.rs)

```rust
pub trait ComponentSet { /* ... */ }
```

Describes the set of components for a query — likely implemented for tuple types (`(A, B, C)`).

---

## 8. Event subsystem ✅

### 8.1. Event trait

**File:** [crates/boyko_ecs/src/ecs/core/events/event.rs](../crates/boyko_ecs/src/ecs/core/events/event.rs)

```rust
pub type EventId = u64;

pub trait Event: 'static + Sized {
    type Participants: Participants;
    type Parameters: Parameters;

    fn event_id() -> EventId;
    fn event_name() -> &'static str;
    fn new(participants: Self::Participants, parameters: Self::Parameters) -> Self;
    fn participants(&self) -> &Self::Participants;
    fn participants_mut(&mut self) -> &mut Self::Participants;
    fn parameters(&self) -> &Self::Parameters;
    fn parameters_mut(&mut self) -> &mut Self::Parameters;
    fn layout() -> Layout { Layout::new::<Self>() }
    fn type_id() -> TypeId { TypeId::of::<Self>() }
}
```

The `#[event]` macro rewrites the user struct into a two-field native layout:
```rust
struct MyEvent {
    pub participants: MyEventParticipants,
    pub parameters: MyEventParameters,
}
```
All accessors are safe typed-field reads — no unsafe pointer casts (Q-001 resolved).

### 8.2. EventPool / EventPoolBundle

- [event_pool.rs](../crates/boyko_ecs/src/ecs/core/events/event_pool.rs) — `EventPool`, `EventPoolIter<'a, E>`
- [event_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/events/event_pool_bundle.rs) — `EventPoolBundle`

### 8.3. EventRegistry (global)

**File:** [event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs)

API: `register_event<E>`, `get_event_info`, `get_event_layout`, `get_participants_layout`, `get_parameters_layout`, `get_event_participants`, `get_event_type_name`, `is_event_registered`, `registered_event_count`, `iter_registered_events`, `get_event_type_ids`, `validate_event_types<E>`.

### 8.4. Participants

- [participants.rs](../crates/boyko_ecs/src/ecs/core/events/participants/participants.rs) — `Participants: 'static + Sized + Copy` trait, `ParticipantInfo`
- [participants_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/participants/participants_buffer.rs) — `ParticipantBuffer` with `Vec<MaybeUninit<u8>>` storage

`Participants` requires `Copy`. `ParticipantBuffer::push` uses `ptr::copy_nonoverlapping`
directly from `&P`. `push_raw` / `get_raw` have been removed (W6).

### 8.5. Parameters

- [parameters.rs](../crates/boyko_ecs/src/ecs/core/events/parameters/parameters.rs) — `Parameters: 'static + Sized + Copy` trait
- [parameters_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/parameters/parameters_buffer.rs) — `ParametersBuffer` with `Vec<MaybeUninit<u8>>` storage

Same storage model as `ParticipantBuffer`. `push_raw` / `get_raw` removed.

### 8.6. `#[event]` attribute macro

**File:** [crates/boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) — `#[proc_macro_attribute] pub fn event(...)`.

`#[derive(Event)]` has been deleted (Q-001). Use `#[event]` instead. Per-field markers:
- `#[participant(components = "TypeA, TypeB")]` — declares a participant entity field
- `#[parameter]` — declares a plain data parameter field

---

## 9. Containers (boyko_ecs::core::containers) ✅

### ComponentTuple

- [containers/tuple/component_tuple.rs](../crates/boyko_ecs/src/ecs/core/containers/tuple/component_tuple.rs)
- [containers/tuple/component_tuple_trait.rs](../crates/boyko_ecs/src/ecs/core/containers/tuple/component_tuple_trait.rs)

Tuple-based component bundle for batch operations / ergonomic API.

---

## 10. boyko_utils — reusable collections ✅

### 10.1. BitMask family

- [bit_mask/bit_storage.rs](../crates/boyko_utils/src/bit_mask/bit_storage.rs) — `BitStorage` trait
- [bit_mask/bit_mask.rs](../crates/boyko_utils/src/bit_mask/bit_mask.rs) — `BitMask<T: BitStorage>` (598 lines — large)
- [bit_mask/bit_set.rs](../crates/boyko_utils/src/bit_mask/bit_set.rs) — `BitSet<T: BitInteger>` + iterator
- [bit_mask/bit_set512.rs](../crates/boyko_utils/src/bit_mask/bit_set512.rs) — `BitSet512` (8×u64 = 512 bits)

`ComponentMask` in boyko_ecs is built on top of `BitSet512`.

### 10.2. SparseMap family

- [sparse_map/sparse_collection.rs](../crates/boyko_utils/src/sparse_map/sparse_collection.rs) — `SparseCollection<K, V>` trait (⚠️ trait declared, but unused in the code)
- [sparse_map/sparse_map.rs](../crates/boyko_utils/src/sparse_map/sparse_map.rs) — `SparseMap<U>` (general)
- [sparse_map/sparse_slot_map.rs](../crates/boyko_utils/src/sparse_map/sparse_slot_map.rs) — `SparseSlotMap<U>` (with generation-based slots)

`EntityMaster::entity_map: SparseMap<EntityInland>` uses this.

### 10.3. identifiers

- [identifiers/primitives.rs](../crates/boyko_utils/src/identifiers/primitives.rs) — `Generation = usize`
- [identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs) — `Slot { index, generation }`

---

## 11. Planned subsystems 📋

- **Scheduler / System runner** — execution of user systems, dependency graph, work-stealing
- **Change detection** — tracking component changes (`is_dirty` in `Chunk` — a stub)
- **Resource management** — global resources
- **Command buffer** — deferred operations
- **Serialization** — deferred
- **Hot-reload** — not a goal

---

## 12. Constants (constants.rs) ✅

Same as on master — see [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs).

| Constant | Value | Use |
|----------|-------|-----|
| `DEFAULT_ARENA_SIZE` | 64 MB | `Arena::new()` |
| `CACHE_LINE_SIZE` | 64 B | `Arena::with_capacity` |
| `MIN_ALIGNMENT` | 8 B | ⚠️ unused |
| `DEFAULT_COMPONENTS_PER_CHUNK` | 1024 | |
| `DEFAULT_CHUNKS_PER_POOL` | 128 | `ComponentPool::with_default_sizes` |
| `TINY/SMALL/MEDIUM/LARGE_COMPONENTS_PER_CHUNK` | 2048 / 1024 / 512 / 256 | `ComponentPool` |
| `TINY/SMALL/MEDIUM_COMPONENT_THRESHOLD` | 16 / 64 / 256 B | `ComponentPool` |
| `INITIAL_ENTITY_CAPACITY` | 1024 | ⚠️ possibly unused |
| `GROWTH_FACTOR / MAX_EXPANSION_FACTOR / ...` | | ⚠️ stubs, unused |

---

## 13. Current build state ⚠️

The branch does not compile at the time of writing. The last fix attempt — `299a6b6 Blanket trait impl error fixed` — did not fully work.

Known problem spots visible from commits and code:
- Many `unused import` warnings — in boyko_ecs and boyko_utils
- `#[inline]` attribute cannot be used on required trait methods — in [component.rs:5](../crates/boyko_ecs/src/ecs/core/component/component.rs) and similar files. This is an **error in newer Rust versions**.
- `unused variable` in `archetype_registry.rs`, `archetype_master.rs`
- Possible blanket trait impl collisions (judging by the last commit message)

The full list of errors will be collected via `cargo check ecs` (see the TaskList).

---

## 14. Project style and conventions

- Comment languages: a mix of Russian and English. Should be unified.
- Doc-comments via `///`, internal via `//`.
- `#[inline]` / `#[inline(always)]` — measured (see CLAUDE.md principle 7).
- `expect("invariant: ...")` instead of `unwrap()` where panic is possible by design.
- `debug_assert!` for invariant checks in the hot path.
