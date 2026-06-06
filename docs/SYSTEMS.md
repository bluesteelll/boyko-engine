# boyko-engine systems catalog (branch `ecs`)

Reference for every subsystem on the `ecs` branch — code locations,
key types, methods, and invariants. Agents read this for navigation.
For "where to find X", start in [FEATURE_MAP.md](FEATURE_MAP.md);
for cross-crate architecture, see [ARCHITECTURE.md](ARCHITECTURE.md);
for the closure status of every audit finding, see
[ROADMAP-PHASE-2-PLUS.md](ROADMAP-PHASE-2-PLUS.md).

**Status legend:**
- ✅ Implemented and tested
- ⚠️ Implemented with documented caveats
- 📋 Planned
- ❌ Not implemented (deliberate)

> The `ecs` branch builds clean; 223 tests pass; clippy `-D warnings`
> clean; Miri-verified. See §13 for the gate report.

---

## 1. Identifiers (ID types) ✅

**Files:**
- [crates/boyko_ecs/src/ecs/identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs)
- [crates/boyko_utils/src/identifiers/primitives.rs](../crates/boyko_utils/src/identifiers/primitives.rs)
- [crates/boyko_utils/src/identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs)

ID types in `boyko_ecs` are strongly-typed newtypes
(`#[repr(transparent)]`, zero runtime cost, but the compiler refuses to
mix them — passing an `EntityId` where a `ComponentId` is expected is
a type error). Audit C-017 (Phase 4b) closed the historical
"all-`usize`-aliases" hole.

```rust
// boyko_ecs::ecs::identifiers::primitives — generated via define_id! macro
pub struct EntityId(pub usize);
pub struct ArchetypeId(pub usize);
pub struct ChunkId(pub usize);
pub struct InlandChunkId(pub usize);
pub struct ComponentId(pub usize);
pub struct InlandUnitId(pub usize);
pub struct InlandPoolId(pub usize);
pub struct InlandComponentId(pub usize);
pub struct InlandArchetypeId(pub usize);

// kept as alias — paired only with EntityId, never crossed with other IDs:
pub type Generation = usize;
```

Each newtype derives `Debug + Default + Clone + Copy + PartialEq + Eq +
Hash + PartialOrd + Ord`, has `const fn new(usize) -> Self` + `const fn
get(self) -> usize`, implements `From<usize>` + `From<Self> for usize`,
and hand-rolls `Display` as `EntityId(42)`.

`Slot` (boyko_utils) is the shared key type for sparse-set
collections:
```rust
pub struct Slot {
    index: usize,
    generation: Generation,
}
```
`Entity` implements `From<Slot>` / `Into<Slot>` for compatibility with
`SparseSlotMap`.

---

## 2. Memory subsystem ✅

### 2.1. Arena ✅

**File:** [crates/boyko_ecs/src/ecs/memory/arena.rs](../crates/boyko_ecs/src/ecs/memory/arena.rs)

64 MB pre-allocated arena backed by `MemFreeBlockMaster` for best-fit
allocation.

```rust
pub struct Arena {
    ptr: NonNull<u8>,
    capacity: usize,                          // used for OOB debug-assert (M-008 closed)
    layout: Layout,
    free_blocks: UnsafeCell<MemFreeBlockMaster>,
}
```

- Has `impl Drop for Arena` (M-001 closed — the original code leaked 64 MB per `EcsMaster::new`).
- Lives behind `Box<Arena>` inside `EcsMaster` so child structures
  (`ArchetypeMaster`, `Archetype`, `ComponentPool`) holding raw pointers
  to it remain valid across moves of the owning `EcsMaster` (C-001 closed).
- Child structures store the arena as `*const Arena` (raw provenance,
  Phase 3a Miri retag fix — `NonNull` retag tags would invalidate
  earlier-derived pointers in Stacked Borrows).
- `!Send + !Sync` by construction (raw pointer field); single-threaded
  use is the architectural assumption.

### 2.2. Chunk (type-erased) ✅

**File:** [crates/boyko_ecs/src/ecs/memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs)

Metadata-only — actual data lives in `ComponentPool::buffer`. Chunks
describe windows into that buffer.

```rust
pub struct Chunk {
    start_index: usize,    // position in the pool's shared buffer
    capacity: usize,
    is_dirty: bool,        // change flag — wired but not yet consumed (groundwork)
}
```

`mark_dirty` / `is_dirty` / `clear_dirty_flag` are present and called
by every pool mutation; no consumer reads the flag yet — it's
groundwork for the planned change-detection phase.

### 2.3. ComponentPool (type-erased) ✅

**File:** [crates/boyko_ecs/src/ecs/memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs)

```rust
pub struct ComponentPool {
    arena: *const Arena,                  // raw provenance (Phase 3a Miri fix)
    buffer: NonNull<u8>,                  // single block allocated from arena
    buffer_capacity_bytes: usize,
    max_components: usize,
    len: usize,                           // live row count; row i at buffer + i*stride (X.B)
    chunks: Vec<Chunk>,                   // metadata only; private (C-023 closed)
    components_per_chunk: usize,
    component_id: usize,
    component_layout: Layout,             // cached from registry
    drop_fn: Option<DropFn>,              // type-erased Drop (M-001 cont. / M-004)
    component_type_id: TypeId,            // typed-API validation (C-004)
}
```

**API:**
- Raw byte: `add(&[u8])`, `set_component(idx, &[u8])`, `get_raw(idx) ->
  Option<*const u8>`, `get_raw_mut(idx) -> Option<*mut u8>`.
- Typed (TypeId-guarded — C-004 closed): `add_typed::<T>(value)`,
  `set_component_typed::<T>(idx, value)`, `get_typed::<T>(idx) ->
  Option<&T>`, `get_mut_typed::<T>(idx) -> Option<&mut T>`.
- Removal: `swap_remove(idx)` (runs `drop_fn`), `pop()` (runs `drop_fn`).
- Iteration: `buffer_ptr() -> *const u8` (the dense base; row `i` is at
  `buffer_ptr() + i*stride` — Phase 2d per-entity iter contract, SAFETY
  documented). (Phase X.B removed the redundant `chunk_units` / `Vec<Unit>`.)

ZST components are rejected at `new` with a `debug_assert!` — adding
ZST support is a planned Phase 2-future enhancement.

### 2.4. MemFreeBlockMaster ✅

**File:** [crates/boyko_ecs/src/ecs/memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs)

`BTreeMap<size, Vec<idx>>` (M-012 closed — original used `HashMap`,
which is a per-allocation killer in hot paths). `start_map` and
`end_map` index free blocks by their boundaries for O(1) coalesce on
insert. `M-018` closed via a reverse `block_idx -> position` map for
O(1) `swap_remove`-from-bucket.

### 2.5. Row addressing (no `Unit` — removed in Phase X.B) ✅

`ComponentPool` rows are addressed by **computed arithmetic**, not a stored
pointer. Row `i`'s bytes are at `buffer.as_ptr().add(i * component_layout.size())`,
exposed internally via the private `#[inline] unsafe fn row_ptr(&self, i)`.
`len: usize` is the live-row count (rows `[0, len)` are initialized & dense;
`swap_remove` keeps them dense by moving the last row's bytes into the hole).

Phase X.B **deleted** the former `Unit { ptr: *mut u8 }` wrapper (`id_unit.rs`)
and its parallel `units: Vec<Unit>`: every entry was provably equal to
`buffer + i*stride`, so the cache was pure redundancy (8 B/row + one heap
allocation per pool). The change is behavior-preserving, **net-removes `unsafe`**
(the old `commit_units` raw-write-into-`Vec`-spare-capacity loop is gone), and is
Miri-Tree-Borrows clean. The two hot paths (random access via `column.ptr.add`,
query iteration via `fetch.base.add`) never used `units`, so iteration is
unaffected. See [docs/PHASE-XB-RESULTS.md](PHASE-XB-RESULTS.md).

### 2.6. Per-pool iterators ❌ deliberately absent

The historical `sparse_iter_component_pool.rs`, `multi_pool_sparse_iter.rs`,
and `iterators.rs` stubs were orphan files (never wired into `mod.rs`,
allocated `Box<[ComponentPtr]>` per entity, used recursive `next_raw`,
had four undefined typed-adapter types). All removed in Phase 2c
(Q-008). The clean replacement — `Query::iter_one/iter_two` —
ships in §7 below, built on top of `QueryState`.

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

Methods use `#[inline]` (default-method cross-crate hint). No
`#[inline(always)]` — Phase 5a demoted every previous always-attribute
workspace-wide per CLAUDE.md principle 7.

### 3.2. ComponentMask

**File:** [crates/boyko_ecs/src/ecs/core/component/component_mask.rs](../crates/boyko_ecs/src/ecs/core/component/component_mask.rs)

512-bit mask (`[BitSet<u64>; 8]`) representing "which components are
present in an archetype". `blocks` field is private; access via
`block(i)` (C-023 closed). `MAX_COMPONENTS = 512` enforced by
`debug_assert!` in `set` / `unset` / `contains` (M-009 / C-011 closed
— original code had a `% 8` mask-down-to-block bug that silently
wrapped IDs past 511 into block 0).

### 3.3. ComponentPoolBundle

**File:** [crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs)

Collection of type-erased `ComponentPool`s within one archetype (one
per `ComponentId`). Two-phase commit API:

- `can_push_entity_components(&[(ComponentId, &[u8])]) -> bool` —
  validates every pool exists and has capacity. Read-only.
- `push_entity_components(&[(ComponentId, &[u8])]) -> usize` — appends
  in lockstep across pools after `can_push` returned true.
- `swap_remove_unit(...) -> EcsResult<()>` (C-019 closed — was
  `anyhow::Result`).

Two-phase commit (C-009 closed) eliminates the historical partial-push
scenario where pool A succeeded and pool B failed, leaving the archetype
in an inconsistent state.

### 3.4. ComponentRegistry (global static)

**File:** [crates/boyko_ecs/src/ecs/core/component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs)

Global lock-free store of `ComponentLayout { layout, type_name, type_id,
drop_fn: Option<DropFn> }` keyed by `ComponentId`. Backing storage:
`static LAYOUTS: [OnceLock<ComponentLayout>; MAX_COMPONENTS]`
(M-002 / C-002 / Q-004 / Q-010 closed: were `static mut` races).

**API:**
- `register_new::<T>() -> ComponentId` (production path — called from
  the `#[derive(Component)]`-generated `component_id()` via a per-type
  `OnceLock<ComponentId>`).
- `register_layout::<T>(id)` — test/macro escape hatch with an explicit ID.
- `get_layout(id) -> Option<&'static ComponentLayout>`,
  `get_layout_unchecked(id) -> &'static ComponentLayout` (unsafe fast path).
- `get_component_size`, `get_component_alignment`, `get_component_memory_layout`.

**IDs are unstable across processes** — assigned in first-call order
via a global `AtomicUsize`. Code that ingests `ComponentId`s from
external sources (network, save files, scripts) MUST warm up the
registry by calling `T::component_id()` for every persisted type
before the first external ID arrives. Collisions between different
types at the same slot panic in both debug and release.

### 3.5. `#[derive(Component)]` macro

**File:** [crates/boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs)

Generates:
- `impl T { const SIZE; const ALIGN; const fn layout() -> Layout }`
- `impl Component for T { fn component_id() -> ComponentId { *PER_TYPE_ONCE_LOCK.get_or_init(|| ComponentId(register_new::<Self>())) } … }`
- `register_layout::<Self>(N)` is no longer triggered from the macro
  in the production path — `register_new` does both ID assignment AND
  layout registration atomically.

---

## 4. Entity subsystem ✅

### 4.1. Entity

**File:** [crates/boyko_ecs/src/ecs/core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity {
    id: EntityId,            // newtype wrapping usize (C-017 closed)
    generation: usize,       // bumped on every deallocate_entity
}
```

Fields private (C-023 closed) — access via `entity.id()` /
`entity.generation()`. Implements `From<Slot> + Into<Slot>` for
sparse-collection compatibility. Equality compares BOTH fields, so
`Entity{id=5, gen=0} != Entity{id=5, gen=1}` — load-bearing ABA defence.

### 4.2. EntityInland

**File:** [crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs)

Internal record kept by `EntityMaster`:

```rust
pub struct EntityInland {
    archetype_id: ArchetypeId,
    unit_index: InlandPoolId,    // position inside the archetype's pool bundle
    generation: usize,
}
```

### 4.3. EntityMaster

**File:** [crates/boyko_ecs/src/ecs/core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs)

```rust
pub struct EntityMaster {
    free_entity_ids: Vec<EntityId>,              // LIFO recycle queue (EM2)
    next_entity_id: AtomicUsize,                 // fresh-id minting (EM1/EM6)
    pub(crate) entities_inland: Vec<EntityInland>, // index = EntityId.0; is_null() ⇔ dead
    live_count: usize,                           // # live entities (Phase X.D)
}
```

Phase 7 replaced the old `entities` + `SparseMap<EntityInland>` pair with a
single direct-indexed `entities_inland: Vec<EntityInland>` (the hot
`get_component_raw` fast store; `is_null()` is the single liveness +
generation source of truth). **Phase X.D** then removed the EnTT-style
`active_ids` (dense live list) and `sparse_to_active` (sparse→dense map):
their only consumer was the cold `iter_entities` API, and the despawn
swap-remove they required was deleted with them. The live count is now a
plain `usize` (`live_count`), dispatcher-only.

**API:**
- `allocate_entity() -> Entity` (recycles from `free_entity_ids` first,
  else `fetch_add` on the `next_entity_id` atomic).
- `register_entity_with_ptr(entity, *mut Archetype, unit_idx)` — writes the
  Phase-7 fast-store slot; `live_count += 1`.
- `register_batch(start, *mut Archetype, start_row, n)` — bulk fast-store
  write for a contiguous reserved id range; `live_count += n`.
- `deallocate_entity(entity) -> bool` — bumps generation in place, nulls the
  slot, recycles the id; `live_count -= 1` (success path only — a no-op on a
  stale/never-registered handle never decrements).
- `is_entity_valid(entity)` / `get_entity(id) -> Option<Entity>` —
  generation-checked liveness read straight from `entities_inland`.
- `iter_entities() -> impl Iterator<Item = Entity>` — **O(capacity)**: scans
  `entities_inland`, skips `is_null()` slots, yields entities in ascending
  `EntityId` order. Cold inspection/test API only (zero hot callers; real
  iteration goes through `Query`/archetype storage). Phase X.D traded the
  O(active) dense walk for this O(capacity) scan to shed per-spawn/despawn
  writes + 12 B/entity.
- `entity_count()` / `is_empty()` — `live_count`-backed (O(1)).
- `rewind_allocate(entity) -> bool` — internal C-007 plumbing for the
  guard pattern in `EcsMaster::create_entity`. Restores
  `next_entity_id` if the just-allocated id was fresh; for recycled IDs
  it's a no-op and the caller falls back to `deallocate_entity`.

---

## 5. Archetype subsystem ✅

### 5.1. Archetype

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs)

```rust
pub struct Archetype {
    id: ArchetypeId,
    signature: ArchetypeSignature,
    component_pools: ComponentPoolBundle,
    entity_ids: Vec<EntityId>,        // parallel to pool dense indices
    current_index: usize,             // = entity_ids.len() = pool dense len
}
```

**API:**
- `create_entity(entity_id, &mut EntityInland, &[(ComponentId, &[u8])]) -> bool`
  (C-010 slice API). Uses `can_push` + `push` two-phase commit
  internally (C-009).
- `remove_entity(&EntityInland) -> RemoveOutcome` — explicit enum
  return replaces the historical `Option<EntityId>` ambiguity (C-006):
  ```rust
  pub enum RemoveOutcome {
      Last,                                       // removed was the tail
      Swapped { moved_entity: EntityId },         // tail swapped into removed slot
      PoolFailure,                                // pool rejected; archetype unchanged
  }
  ```
  Sized to 16 bytes (locked by `const _: () = { assert!(...) }`).
- `pop(&mut EntityInland) -> bool` — extracts last entity. C-008 closed:
  the historical `debug_assert!(pop_entity())` was a release-mode bug
  (pool pop never ran). Q-022 closed: `entity_ids.pop()` is called
  alongside.
- `has_component_id`, `has_components`, `component_mask`, `component_ids` —
  cheap accessors backed by `ArchetypeSignature::mask`.
- `init_entity_inland(&mut EntityInland)` — populates the inland
  record's archetype slice before the actual create.

### 5.2. ArchetypeSignature

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_signature.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_signature.rs)

```rust
pub struct ArchetypeSignature {
    mask: ComponentMask,
    block_summary: BitSet<u8>,        // 1 byte: which 64-bit blocks of mask are non-empty
    section_summary: BitSet<u32>,     // 4 bytes: same idea, coarser
}
```

Hierarchical filter accelerator: `block_summary` and `section_summary`
are derived from `mask` via `Self::new(mask)`. Fields private
(C-023 closed) — mutating mask alone would corrupt the summaries, so
the only mutation path is to build a fresh `ArchetypeSignature`. Accessors
return references / copies.

### 5.3. ArchetypeRegistry

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs)

```rust
pub struct ArchetypeRegistry {
    block_groups: SparseMap<Vec<(ArchetypeId, ArchetypeSignature)>>,   // keyed by 8-bit block pattern
    active_patterns: Vec<u8>,
    total_count: usize,                                                // O(1) len cache (C-015)
    id_to_location: SparseMap<(u8, usize)>,                            // O(1) signature lookup + unregister (C-015)
}
```

C-015 closed: `len()` is now O(1) (debug-asserted against a slow
recompute), `unregister_archetype` is O(1) via the reverse map (was
O(N) with `.clone()` of `active_patterns`), `get_archetype_signature`
is O(1).

The component-discovery API is the entrypoint for everything in
[FEATURE_MAP.md § Archetype-level discovery](FEATURE_MAP.md#archetype-level-discovery-lower-level-lookups);
both `_into(out: &mut Vec)` and allocating wrappers are exposed. Small
queries (≤3 components) use a stack-only relevant-block buffer (Q-013
follow-up, Phase 2a).

### 5.4. ArchetypeBundle

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs)

Owning storage for `Vec<Archetype>` + `SparseMap<ArchetypeId,
inland_idx>` for O(1) lookup by id. `swap_remove`-on-remove redirects
the moved archetype's inland_idx in the sparse map.

### 5.5. ArchetypeMaster

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs)

```rust
pub struct ArchetypeMaster {
    archetypes: ArchetypeBundle,
    registry: ArchetypeRegistry,
    arena: *const Arena,                          // raw provenance (Phase 3a)
    next_archetype_id: ArchetypeId,
    generation: ArchetypeGeneration,              // bumps on every create_archetype
    structural_generation: ArchetypeGeneration,   // bumps on remove + clear (Phase 5c)
}
```

The dual-generation design (Phase 5c) is the load-bearing ArchetypeId-ABA
fix:
- `generation` signals "the set of archetypes grew → reclassify the deltas"
  (QueryState fast-path: delta-add only).
- `structural_generation` signals "the set shrank → cached IDs may be
  dead, dedup bitset may have stale markers" (QueryState slow-path:
  full rebuild, drops bitset, reclassifies every live archetype).

Without `structural_generation`, the historical hole was: a `QueryState`
whose `iter()` was called after `remove_archetype` + `clear` +
`create_archetype` (which recycles `next_archetype_id` back to 1) could
silently surface an unrelated archetype matching the recycled ID. See
the regression test
`query_state::aba_recycled_archetype_id_after_clear_does_not_leak_into_query`.

**API:**
- `create_archetype(&[ComponentId])` / `get_or_create_archetype(...)`
- `remove_archetype(id) -> bool` (bumps `structural_generation` on success)
- `has_archetype(id)` / `get_archetype(id)` / `get_archetype_mut(id)`
- `find_archetypes_with_components` / `find_matching_archetypes` /
  `find_exact_match` / `find_with_filter` (+ `..._into(out)` variants).
- `archetype_generation()` / `structural_generation()` — accessors
  used by `QueryState`.
- `add_existing_archetype(arch)` — migration / deserialisation hook.
- `clear()` — bumps `structural_generation`; safe across outstanding
  `QueryState`s.

---

## 6. EcsMaster (top-level facade) ✅

**File:** [crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs)

```rust
pub struct EcsMaster {
    entity_master: EntityMaster,
    archetype_master: ArchetypeMaster,
    arena: Box<Arena>,                  // stable heap address; dropped last
}
```

Field drop order matters: `entity_master` drops first, then
`archetype_master` (which drops every `ComponentPool` — each invoking
the type-erased `drop_fn` on its slots), then `arena`. Arena lives
behind `Box` so its address is stable across moves of the owning
`EcsMaster` (C-001 closed). Child structures store a raw `*const Arena`
minted via `&raw const *arena_box` to avoid the Stacked Borrows retag
UB that `NonNull::from(&*arena_box)` triggers when multiple pools are
constructed in sequence (Phase 3a Miri fix).

**API (full surface in [FEATURE_MAP.md § EcsMaster](FEATURE_MAP.md#high-level-facade-ecsmaster)):**
- `new()` / `with_capacity(entity_cap, arch_cap)`
- `create_archetype(...)` / `get_or_create_archetype(...)`
- `create_entity(arch, &[(id, bytes)]) -> EcsResult<Entity>` — guard pattern
- `spawn_one::<A>(arch, A) -> EcsResult<Entity>` — Phase 2e
- `spawn_two::<A, B>(arch, A, B) -> EcsResult<Entity>` — Phase 2e
- `delete_entity(entity) -> bool`
- `get_component_raw` / `get_component_raw_mut` / `set_component_raw`
- `has_entity` / `has_component` / `get_entity_archetype_id`
- `entity_count` / `archetype_count` / `recycled_entity_count`
- `iter_entities()` (O(capacity) fast-store scan — cold/inspection) /
  `query_entities(&[id])` (allocating Vec — hot paths prefer
  `Query::iter_one/iter_two`)
- `get_components_raw` / `get_components_raw_mut` — batch raw access
- `entity_master[_mut]()` / `archetype_master[_mut]()` / `arena()` —
  field accessors for power users
- `clear()`

Error type: [`core/error.rs`](../crates/boyko_ecs/src/ecs/error.rs)
defines `#[non_exhaustive] pub enum EcsError` (C-019 closed) with six
variants:
- `ArchetypeNotFound(ArchetypeId)`
- `EntityNotFound(EntityId)`
- `ComponentPoolFull { component_id }`
- `UnknownComponentForArchetype { archetype_id, component_id }`
- `ArchetypeRejectedEntity { archetype_id }`
- `PoolSwapRemoveFailed`

Hand-rolled `Display` + `std::error::Error` — no `thiserror` dep.

---

## 7. Query subsystem ✅

### 7.1. Query

**File:** [crates/boyko_ecs/src/ecs/core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs)

`Query<'a>` is the one-shot wrapper:

```rust
pub struct Query<'a> {
    state: QueryState,
    master: &'a ArchetypeMaster,
}
```

Constructors:
- `from_archetypes(archetypes: Vec<&Archetype>, master)` — back-compat.
- `with_component_ids(master, &[ComponentId])`
- `with_mask(master, &ComponentMask)` — superset match.
- `with_exact_mask(master, &ComponentMask)`
- `with::<T: ComponentSet>(master)` — typed tuple.
- `with_filters(master, &include, &exclude, &optional)`
- `with_type_filters::<Inc, Exc, Opt>(master)`

Iteration:
- `iter()` — `QueryStateIter<'_>` yielding `&Archetype`.
- `IntoIterator for &Query<'a>` — for-loop sugar.
- `archetypes() -> Vec<&Archetype>` — materialised (slow path, kept for
  back-compat).
- `iter_one::<A: Component>() -> QueryIterOne<'_, A>` yields `&A` per entity (Phase 2d).
- `iter_two::<A, B: Component>() -> QueryIterTwo<'_, A, B>` yields `(&A, &B)` per entity (Phase 2d).

Per-entity iterators use pointer-bump cursors (`*const A` + `remaining:
usize`) — zero allocation per row, single pointer add + deref per yield.

### 7.2. QueryState

**File:** [crates/boyko_ecs/src/ecs/core/iters/query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs)

```rust
#[repr(C, align(64))]
pub struct QueryState {
    generation: ArchetypeGeneration,             // last seen creation counter
    structural_generation: ArchetypeGeneration,  // last seen structural counter (Phase 5c)
    matched_ids: Vec<ArchetypeId>,
    include: ComponentMask,
    exclude: ComponentMask,
    optional: ComponentMask,
    matched_archetypes: ArchetypeBitSet,         // 1024-bit dedup
}
```

Cache-line layout: hot fields (gens + matched_ids) in line 0; the three
masks (192 B) span lines 1–3; the dedup bitset (128 B) is the coldest.

**Hot path** (both gens unchanged): pure slice walk over `matched_ids`,
one `get_archetype` per element. Measured ~3.6 ns vs ~77 ns for
one-shot Query construction in `benches/query_iter.rs` (~21× speedup;
Q-011 closed).

**Cold paths**:
- Structural mismatch (Phase 5c): drop bitset + matched_ids, fully
  reclassify every live archetype. Triggered by `remove_archetype` /
  `clear()`.
- Creation-only delta: classify only newly-minted IDs since last sync.

`QueryState` is safe across `master.clear()` and `remove_archetype()`
without manual reset — the structural counter triggers the rebuild
path automatically. `reset()` remains available for manual capacity
shrink / filter rebuild.

### 7.3. ArchetypeBitSet

**File:** [crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs](../crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs)

1024-bit inline bitset (`[u64; 16]`, 128 B, no heap). Used by
`QueryState` for O(1) dedup of newly-seen archetype IDs. `insert` /
`contains` `panic!` (release-included) when `id >= MAX_ARCHETYPES = 1024`.

### 7.4. ComponentSet

**File:** [crates/boyko_ecs/src/ecs/core/iters/component_set.rs](../crates/boyko_ecs/src/ecs/core/iters/component_set.rs)

```rust
pub trait ComponentSet {
    fn component_ids() -> &'static [ComponentId];
}
```

Returns a `&'static` slice — no allocation after first call per type
(Q-012 closed; was `Vec<ComponentId>` allocated per call). Implemented
for `()` and tuple types `(A,)` through `(A, B, C, D, E, F, G, H)`.
Single-component types use a `SINGLE_COMPONENT_CACHE[id]` lock-free
per-id `OnceLock`; tuple types use `Box::leak` per (type, monomorph).

---

## 8. Event subsystem ✅ (registry + buffers only — no dispatcher)

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

The `#[event]` attribute macro (Q-001 closed) rewrites the user struct
into a two-field native layout:

```rust
struct MyEvent {
    participants: MyEventParticipants,
    parameters: MyEventParameters,
}
```

All accessors are safe typed-field reads — no `self as *const Self as
*const Self::Participants` UB cast (the original derive's load-bearing
unsoundness).

### 8.2. EventPool / EventPoolBundle ❌ not implemented

The pre-existing `event_pool.rs` and `event_pool_bundle.rs` were
entirely wrapped in `/* */` block comments with `//TODO: rework`
markers and never wired into `events/mod.rs`. Deleted in Q-025 (Phase
4a). The replacement — a proper event dispatcher with reader cursors
and per-frame double-buffering — would be its own feature (planned
Phase X). See [ROADMAP-PHASE-2-PLUS.md](ROADMAP-PHASE-2-PLUS.md).

### 8.3. EventRegistry (global)

**File:** [crates/boyko_ecs/src/ecs/core/events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs)

```rust
static EVENT_INFO: [OnceLock<EventInfo>; MAX_EVENTS] = ...;       // MAX_EVENTS = 256
static NEXT_EVENT_ID: AtomicUsize = AtomicUsize::new(0);
```

Mirrors `component_registry` (M-002 / Q-010 closed: were `static mut`).
`register_event_new::<E>()` is the production path called from
`#[event]`-generated `E::event_id()` via per-type `OnceLock`.
`register_event::<E>(id)` is the test escape hatch. Slot collisions
between different types panic in both debug and release.

Same startup warm-up contract applies as for ComponentId: serialised
EventIds require the warm-up call before deserialisation.

### 8.4. Participants / Parameters

**Files:**
- [core/events/participants/participants.rs](../crates/boyko_ecs/src/ecs/core/events/participants/participants.rs)
- [core/events/participants/participants_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/participants/participants_buffer.rs)
- [core/events/parameters/parameters.rs](../crates/boyko_ecs/src/ecs/core/events/parameters/parameters.rs)
- [core/events/parameters/parameters_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/parameters/parameters_buffer.rs)

Per-event-type typed buffers built on `Vec<MaybeUninit<u8>>`. Both
carry a `TypeId` field set at construction and `debug_assert_eq!` on
every typed `get<P>` / `push<P>` (Q-019 closed — type confusion was
previously possible: same `size_of` but different layout would pass
the only check).

> Q-020 (Participants/Parameters split overengineered?) — deliberately
> deferred. The split survives because Q-001 made it sound and Q-019
> made it type-checked; no event-dispatcher feature today uses the
> "filter subscribers by participant set" capability that motivated
> the split. Reopen when a real use case appears. See
> `ROADMAP-PHASE-2-PLUS.md` Phase 4b for the full rationale.

---

## 9. Error handling ✅

**File:** [crates/boyko_ecs/src/ecs/error.rs](../crates/boyko_ecs/src/ecs/error.rs)

Library-style domain error (C-019 closed — was `anyhow::Result`):

```rust
#[non_exhaustive]
pub enum EcsError {
    ArchetypeNotFound(ArchetypeId),
    EntityNotFound(EntityId),
    ComponentPoolFull { component_id: ComponentId },
    UnknownComponentForArchetype { archetype_id: ArchetypeId, component_id: ComponentId },
    ArchetypeRejectedEntity { archetype_id: ArchetypeId },
    PoolSwapRemoveFailed,
}
pub type EcsResult<T> = Result<T, EcsError>;
```

`#[non_exhaustive]` so new variants can land without major-version
bumps. Hand-rolled `Display` + `std::error::Error` impls — no
`thiserror` dep. The `anyhow` crate has been dropped from
`boyko_ecs/Cargo.toml`.

---

## 10. boyko_utils ✅

### 10.1. BitSet

**File:** [crates/boyko_utils/src/bit_mask/bit_set.rs](../crates/boyko_utils/src/bit_mask/bit_set.rs)

`BitSet<T: BitInteger>` generic over the backing integer (u8 / u32 /
u64). `set` / `unset` / `contains` / iter / bitwise combinators.

The previously commented `bit_mask.rs`, `bit_set512.rs`, and
`bit_storage.rs` (1080 LOC fully wrapped in `/* */`) were deleted in
M-010 (Phase 5b) — they defined a "BitStorage" trait abstraction never
used by anyone.

### 10.2. SparseMap

**File:** [crates/boyko_utils/src/sparse_map/sparse_map.rs](../crates/boyko_utils/src/sparse_map/sparse_map.rs)

```rust
pub struct SparseMap<U> {
    sparse: Vec<Option<usize>>,
    dense: Vec<U>,
    indices: Vec<usize>,
}
```

`insert` / `swap_remove` / `contains` / `get` / `get_mut` / `len`.
Exposes `active_indices() -> &[usize]` and `iter_dense() -> slice::Iter<U>`
for O(active) iteration over live entries. (`EntityMaster` no longer uses
`SparseMap` — it moved to a direct `Vec<EntityInland>` in Phase 7 and dropped
its dense index in Phase X.D; `SparseMap` is retained for other sparse stores.)

### 10.3. SparseSlotMap

**File:** [crates/boyko_utils/src/sparse_map/sparse_slot_map.rs](../crates/boyko_utils/src/sparse_map/sparse_slot_map.rs)

Generation-tracked variant keyed by `Slot { index, generation }`.

The sparse layout encodes three states per index:
- `None` — pristine, next allocation requires `generation = 0`.
- `Some(Slot { idx: usize::MAX, gen })` — TOMBSTONE; next allocation must
  use `gen` (the bumped successor of the prior occupant's generation).
- `Some(Slot { idx: dense_idx, gen })` — occupied; current generation `gen`.

`remove` writes a tombstone with `generation.wrapping_add(1)` so any
stale `Slot` from before the remove is rejected by `contains` / `get` /
future `insert`. `create_slot(idx)` reads the stored generation
(tombstone or pristine baseline) so the next valid insert carries a
fresh generation — closes the ABA hole the old code documented but
never actually fixed (M-016 closed in Phase 5b).

Test coverage (9 tests):
- Pristine non-zero gen rejected
- Insert-then-replace returns old
- ABA stale-slot rejected after remove+reinsert (regression)
- Tombstone removal returns None
- swap_remove rewires moved sparse entry
- Tombstone wrong-gen insert rejected

### 10.4. SparseCollection trait

**File:** [crates/boyko_utils/src/sparse_map/sparse_collection.rs](../crates/boyko_utils/src/sparse_map/sparse_collection.rs)

Trait abstraction `SparseCollection<K, V>` implemented by both
`SparseMap` and `SparseSlotMap`. Kept as a future extension point.

### 10.5. Slot

**File:** [crates/boyko_utils/src/identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Slot {
    index: usize,
    generation: Generation,
}
```

`new(idx, gen)` / `index()` / `generation()` / `increment_generation()`.
Shared key for sparse-set + slot-map structures.

---

## 11. `#[derive(Component)]` and `#[event]` macros ✅

**File:** [crates/boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs)

### Component derive

Emits a `Component` impl whose `component_id()` lazily caches the
result of `register_new::<Self>()` via per-type `OnceLock<ComponentId>`,
plus three inherent constants (`SIZE`, `ALIGN`, `layout()`).

### #[event] attribute

Rewrites a user struct with `#[participant(components = "...")] field:
Entity` and `#[parameter] field: T` fields into:

```rust
struct MyEvent {
    participants: MyEventParticipants,
    parameters: MyEventParameters,
}

#[derive(Clone, Copy)]
struct MyEventParticipants {
    field: Entity, // ...
}

#[derive(Clone, Copy)]
struct MyEventParameters {
    field: T, // ...
}
```

Plus the `Event` impl (typed-field accessors, `event_id` via per-type
`OnceLock`, etc.). Validates: no generic structs, named fields only,
every field has exactly one marker. Compile-fail UI tests in
`tests/ui/event_attribute/*.rs`.

---

## 12. Constants

**File:** [crates/boyko_ecs/src/ecs/constants.rs](../crates/boyko_ecs/src/ecs/constants.rs)

| Name | Value | Used where |
|------|-------|------------|
| `DEFAULT_ARENA_SIZE` | 64 MB | `Arena::new`, `EcsMaster::with_capacity` |
| `MAX_COMPONENTS` | 512 | `ComponentMask` bounds + `ComponentRegistry` slot count |
| `MAX_EVENTS` | 256 | `EventRegistry` slot count |
| `MAX_ARCHETYPES` | 1024 | `ArchetypeBitSet` width |
| `DEFAULT_CHUNKS_PER_POOL` | 4 | `ComponentPool::with_default_sizes` |
| `TINY/SMALL/MEDIUM_COMPONENT_THRESHOLD` | 16 / 64 / 256 B | `ComponentPool::get_optimal_chunk_capacity` |
| `TINY/SMALL/MEDIUM/LARGE_COMPONENTS_PER_CHUNK` | 256 / 128 / 32 / 8 | same |
| `MIN_ALIGNMENT` | 8 B | currently unused (groundwork for explicit-alignment API) |

---

## 13. Current build state ✅

All workspace gates pass as of the latest commit:

| Gate | Status |
|------|--------|
| `cargo check --all-targets` | clean |
| `cargo clippy --all-targets -- -D warnings` | clean (0 warnings) |
| `cargo test --workspace` | **223 tests pass**, 2 ignored stress, 2 ignored doctests |
| `cargo +nightly miri test --package boyko-ecs --lib` | clean (143 lib tests, ~8000 s wall) |
| `cargo bench --no-run` | compiles |

CI (`.github/workflows/ci.yml`) runs check + test + clippy on every
push to `ecs`; Miri runs with no `continue-on-error` (real gate, not
warning).

Historical "does not compile" warnings in older revisions of this
document refer to the pre-Phase-1a state (baseline before commit
`508398c`). All 89 audit findings (`docs/AUDIT-2026-05-23.md`) are
closed, subsumed, or explicitly deferred with rationale in
`docs/ROADMAP-PHASE-2-PLUS.md`.

---

## 14. Benchmarks ✅

**Location:** [crates/boyko_ecs/benches/](../crates/boyko_ecs/benches/)

Five criterion benches (`harness = false`):

| File | What it measures | Notes |
|------|------------------|-------|
| [`component_id.rs`](../crates/boyko_ecs/benches/component_id.rs) | `Component::component_id()` hot path | C-003 validation |
| [`swap_remove.rs`](../crates/boyko_ecs/benches/swap_remove.rs) | `EcsMaster::delete_entity` for N = 100 / 1k / 10k | M-004 validation |
| [`query_iter.rs`](../crates/boyko_ecs/benches/query_iter.rs) | `Query::with_component_ids` rebuild vs `QueryState` warm | ~21× speedup measured (Q-011) |
| [`archetype.rs`](../crates/boyko_ecs/benches/archetype.rs) | `Archetype::create_entity` for 2 / 8 component widths | C-010 / C-016 |
| [`allocator.rs`](../crates/boyko_ecs/benches/allocator.rs) | `MemFreeBlockMaster::allocate_aligned` | M-012 validation |

Run locally:
```powershell
# Compile only (fast):
cargo bench --no-run

# Run one bench with short warmup:
cargo bench --package boyko-ecs --bench query_iter -- --warm-up-time 1 --measurement-time 2
```
