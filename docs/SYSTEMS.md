# boyko-engine systems catalog (branch `ecs`)

Reference for every subsystem, listing code locations, key types, methods, and invariants. Used by agents for navigation.

**Status legend:**
- ✅ Implemented
- ⚠️ Present, but with issues / incomplete
- 📋 Planned

> The `ecs` branch builds clean (`cargo check --all-targets`, `cargo clippy --all-targets -- -D warnings`), passes 210+ tests (workspace), and is verified UB-free under Miri. Previous "does not compile" warnings refer to the historical state before Phase 1a — see `docs/AUDIT-2026-05-23.md` for the chronology and `docs/ROADMAP-PHASE-2-PLUS.md` for what remains open.

---

## 1. Identifiers (ID types) ✅

**Files:**
- [crates/boyko_ecs/src/ecs/identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs)
- [crates/boyko_utils/src/identifiers/primitives.rs](../crates/boyko_utils/src/identifiers/primitives.rs)
- [crates/boyko_utils/src/identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs)

All IDs except `Generation` are `#[repr(transparent)] pub struct X(pub usize)`
newtypes (audit C-017 closed — Phase 4b). Zero runtime cost, but the compiler
refuses to mix them (e.g. `archetype.has_component_id(entity.id())` is a
type error now).
```rust
// generated via the `define_id!` macro in identifiers/primitives.rs:
pub struct EntityId(pub usize);
pub struct ArchetypeId(pub usize);
pub struct ChunkId(pub usize);
pub struct InlandChunkId(pub usize);
pub struct ComponentId(pub usize);
pub struct InlandUnitId(pub usize);
pub struct InlandPoolId(pub usize);
pub struct InlandComponentId(pub usize);
pub struct InlandArchetypeId(pub usize);

// kept as a plain alias — only ever paired with EntityId inside Entity,
// never crossed with another ID type:
pub type Generation = usize;
```
Each newtype: `#[repr(transparent)]`, derives `Debug, Default, Clone, Copy,
PartialEq, Eq, Hash, PartialOrd, Ord`, `const fn new(usize)` + `const fn
get(self) -> usize`, `From<usize>` + `From<Self> for usize`, manual `Display`
rendering as `EntityId(42)`.

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
    capacity: usize,                   // used for OOB debug-assert (M-008 closed)
    layout: Layout,
    free_blocks: UnsafeCell<MemFreeBlockMaster>,
}
```

`Arena` has an `impl Drop` (M-001 closed) and is stored as `Box<Arena>`
inside `EcsMaster` so child pointers remain stable across moves (C-001 closed,
Phase 3a Miri retag fix).

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
    chunks: Vec<Chunk>,                   // private; expose via `chunks()` accessor (C-023 closed)
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

### 2.6. Per-pool iterators ❌ not implemented

Previously this section claimed three orphan files (`sparse_iter_component_pool.rs`,
`multi_pool_sparse_iter.rs`, `iterators.rs`) as "✅ implemented". They were
disconnected from `mod.rs`, did not compile (referenced undefined
`TypedMultiComponentIter` / `TypedComponentIter`), allocated `Box<[ComponentPtr]>`
per entity, and used recursive traversal. **Removed in Phase 2c.**

The replacement — a zero-alloc per-entity iterator built on top of `QueryState`
(`Query::<(&T, &U)>::iter()`, Bevy `WorldQuery` shape) — is tracked as Phase 2d
(see `docs/ROADMAP-PHASE-2-PLUS.md`).

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

Default-method `#[inline]` (cross-crate hint) only. `#[inline(always)]` was demoted workspace-wide in Phase 5a — see CLAUDE.md principle 7.

### 3.2. ComponentMask

**File:** [crates/boyko_ecs/src/ecs/core/component/component_mask.rs](../crates/boyko_ecs/src/ecs/core/component/component_mask.rs)

A high-level wrapper over `BitSet512` for the "which components an archetype contains" mask.

### 3.3. ComponentPoolBundle

**File:** [crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs)

A collection of type-erased `ComponentPool`s within one archetype (one per `ComponentId`). Has `swap_remove_unit` returning `EcsResult<()>` (C-019 closed) and a two-phase `can_push_entity_components` / `push_entity_components` API (C-009 closed).

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

⚠️ `ComponentId` values are unstable across processes (assigned in first-call order via `register_new::<T>()` — C-003 closed, see component_registry.rs for the startup warm-up contract).

---

## 4. Entity subsystem ✅

### 4.1. Entity

**File:** [crates/boyko_ecs/src/ecs/core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs)

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Entity {
    id: EntityId,            // newtype wrapping `usize` (C-017 closed)
    generation: usize,
}
```

Fields are private (C-023 closed) — access via `Entity::id()` / `Entity::generation()`.
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
- `create_entity(archetype_id, &[(ComponentId, &[u8])]) -> EcsResult<Entity>` (C-010 / C-019 closed)
- `delete_entity(entity) -> bool`

Library API uses the domain-typed `EcsResult` (C-019 closed). Variants today: `ArchetypeNotFound`, `EntityNotFound`, `ComponentPoolFull`, `UnknownComponentForArchetype`, `ArchetypeRejectedEntity`, `PoolSwapRemoveFailed` — see `crates/boyko_ecs/src/ecs/error.rs`. Marked `#[non_exhaustive]` so new variants land without major-version bumps.

---

## 7. Query subsystem ✅

### 7.1. Query

**File:** [crates/boyko_ecs/src/ecs/core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs)

```rust
pub struct Query<'a> {
    state: QueryState,          // one-shot cache built at construction time
    master: &'a ArchetypeMaster,
}
```

Constructors: `from_archetypes`, `with_component_ids`, `with_mask`, `with_exact_mask`.

`Query<'a>` is the one-shot query path: at construction it builds a `QueryState`,
calls `update_archetypes` once, then serves `iter()` from the cached `matched_ids`.
Storing `Vec<ArchetypeId>` (not `Vec<&Archetype>`) eliminates a latent
stale-reference UB from `swap_remove` in `ArchetypeBundle`.

### 7.2. QueryState (Q-011)

**File:** [crates/boyko_ecs/src/ecs/core/iters/query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs)

```rust
#[repr(C, align(64))]
pub struct QueryState {
    // Cache line 0 (hot):
    generation: ArchetypeGeneration,    // last-synced generation
    matched_ids: Vec<ArchetypeId>,      // IDs that passed the filter
    // Lines 1-3 (cold):
    include: ComponentMask,
    exclude: ComponentMask,
    optional: ComponentMask,
    // Lines 4-5 (coldest):
    matched_archetypes: ArchetypeBitSet,  // O(1) dedup during update
}
```

The long-lived cache for hot-path iteration. Key APIs:
- `update_archetypes(&master)` — delta-classifies newly minted archetypes since the last call. O(new_archetypes) work.
- `iter(&mut self, master)` — Bevy-style split: requires `&mut self` for the generation check, delegates to `iter_cached` on warm path.
- `reset()` — clears the cache for reuse after `master.clear()`.

**Warm-path cost**: one `generation` load + compare; if unchanged, pure slice walk + per-id `get_archetype` (SparseMap O(1)).
**Benchmark (2026-05-23)**: ~3.6 ns vs ~77 ns for the one-shot `Query` path (~21x faster for the two-archetype test setup).

**Generation tracking**: uses `ArchetypeGeneration` (monotonic `NonZeroUsize`), bumped on every `create_archetype`. Never reset by `clear()` — stale `QueryState` detection is based on `state.generation <= master.archetype_generation()`. A `debug_assert!` catches post-`clear()` reuse in debug builds.

**`ArchetypeBitSet`**: 1024-bit inline bitset (128 B, no heap) for O(1) dedup. `insert`/`contains` panic in all builds when `id >= MAX_ARCHETYPES`.

### 7.3. ComponentSet

**File:** [crates/boyko_ecs/src/ecs/core/iters/component_set.rs](../crates/boyko_ecs/src/ecs/core/iters/component_set.rs)

```rust
pub trait ComponentSet {
    fn component_ids() -> &'static [ComponentId];
}
```

Describes the set of components for a query — implemented for `()` and tuple types `A` through `(A..H)`.
Returns a `&'static [ComponentId]`: no allocation after first call per type (Q-012).
Single-component types use `SINGLE_COMPONENT_CACHE[id]` (lock-free per-id OnceLock).
Tuple types use `Box::leak` per call (generic statics are shared across monomorphizations in Rust).

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

### 8.2. EventPool / EventPoolBundle ❌ not implemented

`event_pool.rs` and `event_pool_bundle.rs` were fully wrapped in block comments with
`//TODO: rework` markers and were never wired into `events/mod.rs`. They have been
deleted (Q-025). See a future event-pool ticket for the planned rework.

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

## 9. Containers (boyko_ecs::core::containers) ❌ not implemented

### ComponentTuple ❌ not implemented

Previously this section claimed two orphan files (`containers/tuple/component_tuple.rs`,
`containers/tuple/component_tuple_trait.rs`) as "✅ implemented". Both files were
0-byte stubs, `containers/mod.rs` did not exist, and `core/mod.rs` never declared
`pub mod containers;` — the compiler never saw the subtree. **Removed in Q-024.**

The replacement — a tuple-based ergonomic bundle API (planned name `ComponentTuple`)
— is tracked as Phase 2e (see `docs/ROADMAP-PHASE-2-PLUS.md`).

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

## 13. Current build state ✅

The branch builds clean as of the latest commit. All workspace gates pass:

- `cargo check --all-targets` — clean
- `cargo clippy --all-targets -- -D warnings` — clean (no warnings)
- `cargo test --workspace` — 210 tests pass (161 lib + 40 integration + 9 utils), 4 ignored stress / doctest entries
- `cargo +nightly miri test --package boyko-ecs --lib` — clean (Phase 3a Miri retag UB fix verified, ~8000 s wall clock on Windows x86_64)

CI (`.github/workflows/ci.yml`) gates check, test, clippy, miri and bench-compile on every push to `ecs`.

Historical "does not compile" warnings refer to the pre-Phase-1a state (audit `docs/AUDIT-2026-05-23.md`, baseline commit before `508398c`). All 89 audit findings are either closed or tracked as deliberate tickets in `docs/ROADMAP-PHASE-2-PLUS.md`.

---

## 14. Benchmarks

**Location:** `crates/boyko_ecs/benches/`

Three benchmark files, all using `criterion` (v0.5, `harness = false`):

| File | What it measures | Phase 2 relevance |
|------|-----------------|-------------------|
| `component_id.rs` | `Component::component_id()` hot path (~524 ps) | C-003 fix validation |
| `swap_remove.rs` | `EcsMaster::delete_entity` for N = 100 / 1k / 10k | M-004 fix validation |
| `query_iter.rs` | `Query::with_component_ids` rebuild + entity scan | Q-011 baseline |

Run locally:
```powershell
# Compile only (fast):
cargo bench --no-run

# Run one bench with short warmup:
cargo bench --package boyko-ecs --bench component_id -- --warm-up-time 1 --measurement-time 2

# Run all benches:
cargo bench --package boyko-ecs
```

**Do not commit** `target/criterion/` — covered by `/target/` in `.gitignore`.

Component ID ranges reserved for bench files to avoid `OnceLock` registry collisions:
- `component_id.rs` — uses `#[derive(Component)]` → `register_new` assigns IDs automatically.
- `swap_remove.rs` — 480-489 (fixed via `register_layout`).
- `query_iter.rs` — 470-479 (fixed via `register_layout`).

---

## 15. CI (GitHub Actions)

**File:** `.github/workflows/ci.yml`

Triggers on push/PR to `master` and `ecs`, plus `workflow_dispatch`.

| Job | Status | Notes |
|-----|--------|-------|
| `check` | Gating | `cargo check --all-targets`, `RUSTFLAGS=-D warnings` |
| `test` (debug) | Gating | `cargo test --all-targets` |
| `test` (release) | Gating | `cargo test --all-targets --release` |
| `clippy` | Gating (with `\|\| true`) | Pre-existing `boyko_utils` violations; remove `\|\| true` after Phase 5 cleanup |
| `bench-compile` | Gating | `cargo bench --no-run` — verifies criterion harness compiles |
| `miri` | Informational | `continue-on-error: true`; runs `event_attribute` + `drop_fn` (known clean), then full sweep. Gate fully after Phase 3a. |

Cache: `Swatinem/rust-cache@v2` on all jobs. See also `docs.yml` for the separate documentation deploy workflow.

---

## 16. Project style and conventions

- Comment languages: a mix of Russian and English. Should be unified.
- Doc-comments via `///`, internal via `//`.
- `#[inline]` / `#[inline(always)]` — measured (see CLAUDE.md principle 7).
- `expect("invariant: ...")` instead of `unwrap()` where panic is possible by design.
- `debug_assert!` for invariant checks in the hot path.
