# Phase 7 — Fast Random Access (design)

## Goal & target metrics

**Goal**: collapse `EcsMaster::get_component_raw(entity, comp_id)` from a 9-cache-line, ~40 ns multi-indirection chain into a 3-cache-line, ~10-15 ns straight pointer arithmetic — matching Bevy's `Table`/`Column` model — without breaking generation-based ABA, archetype-pointer stability, or future scheduler safety.

**Target metrics** (release, single-core, AMD Zen3/Intel Alder Lake-class, hot ICache):

| Operation | Today | Target | How |
|-----------|-------|--------|-----|
| `get_component_raw` | ~40 ns / 9 lines | **~12 ns / 3 lines** | Direct entity slot + cached column ptr+stride |
| `get_component_raw_mut` | ~40 ns | **~12 ns** | Same path, `*mut` cast under `&mut self` |
| `has_entity` | ~15 ns / 3 lines | **~5 ns / 1 line** | Single slot load + null + gen check |
| `set_component_raw` | ~45 ns | **~15 ns** | Same fast lookup + memcpy |
| `iter_entities` | O(active) | **O(active), unchanged** | Keep parallel `active_ids` index |
| `create_entity` | ~150 ns | **≤ 160 ns** | One extra slot-ptr write; column table already built |
| Cache misses per random lookup | ~5-7 | **2-3** | Slab + cached columns |
| Allocations per lookup | 0 | **0** | Unchanged |
| `EntityInland` size | 24 B | **16 B** | Pack ptr+u32+u32 |

---

## Decisions & rationale (D1-D10)

### D1. Stable archetype storage — **Pinned slab `Box<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>`** with `BitSet<u64>[16]` occupancy

**Decision**: option (b) — a single pinned, fixed-size slab of 1024 `MaybeUninit<Archetype>` slots, with a 1024-bit occupancy bitset (`[u64; 16]` = 128 B). Address of any inhabited slot is computed by base + slot_index × size_of::<Archetype>() and is **stable for the lifetime of `ArchetypeBundle`**.

**Why**:
- `*const Archetype` stored in `EntityInland` MUST stay valid across `create_archetype` calls. `Vec<Archetype>::push` reallocates on grow → pointers dangle. Disqualified.
- `Vec<Box<Archetype>>`: each `Box` is stable but adds a pointer-chase on iteration (Phase 9 scheduler iterates archetypes for system dispatch). Slab keeps archetypes contiguous → iteration walks the bitset and accesses contiguous slots.
- `Pin<Box<[MaybeUninit<Archetype>; 1024]>>` allocates `1024 × size_of::<Archetype>()` upfront. Estimate of `Archetype` after Phase 7 (see D4): ~5 KB (current ~120 B + 4 KB columns table). Slab = **~5 MB upfront, sub-allocated lazily**. On a modern engine this is acceptable (memory is the cheapest resource).
- Bitset iteration uses `trailing_zeros` (LZCNT/TZCNT) — O(popcount(occupied)) for sparse, O(1024/64) = O(16) worst case. Cache-warm.

**Alternatives rejected**:
- **(a) `Vec<Box<Archetype>>`**: per-archetype heap alloc, extra indirection on iteration, fragmented address space. Memory savings are illusory because `Archetype` is 5 KB so heap fragmentation eats it back. ✗
- **(c) Custom slab with linked pages**: only justified if `MAX_ARCHETYPES` is uncapped. Today it is 1024 (hard limit by `ComponentMask`'s 512-bit width — there cannot be more distinct archetypes than usefully describable). ✗
- **`pin-project`/`pinned-init` crate**: external dep; we own the invariant. ✗

**Trade-off**:
- ~5 MB upfront vs ~0 today. Trivial against the 64 MB arena.
- Slot recycling on `remove_archetype` doesn't shrink the slab — same as `Vec` behavior.
- Manual `Drop` for occupied slots (drop loop walks the occupancy bitset).

### D2. `EntityInland` layout — 16 B: `(ptr: *mut Archetype, unit_index: u32, generation: u32)`

```rust
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EntityInland {
    /// Raw provenance pointer into ArchetypeBundle's pinned slab.
    /// NULL ⇔ dead slot (never registered or deallocated).
    /// Stored as *mut (not *const) so `&mut EcsMaster` can transitively
    /// cast to `&mut Archetype` without provenance laundering (D7).
    archetype_ptr: *mut Archetype,  // 8 B, offset 0
    unit_index: u32,                // 4 B, offset 8  — index into Archetype.entity_ids
    generation: u32,                // 4 B, offset 12 — matches Entity.generation truncated
}
// size_of::<EntityInland>() == 16, align_of == 8
// 4 EntityInlands per cache line — register_entity touches one line
```

**Why**:
- `archetype_ptr` (8 B) replaces `archetype_id: ArchetypeId(usize)` (8 B): same size, eliminates the SparseMap → Vec → indirection chain in the read path. The bundle still owns the address space, but reads go pointer-direct.
- `unit_index: u32` instead of `InlandPoolId(usize)`: a single archetype caps at `INITIAL_ENTITY_CAPACITY * MAX_EXPANSION_FACTOR = 1024 × 8 = 8192` entities (current constants). `u32::MAX = 4.3 B` is **3 orders of magnitude** above any realistic count. Saves 4 B and lets us pack into 16 B.
- `generation: u32` instead of `usize`: 4 B saved per slot. 2^32 = 4 B deallocations of a single slot before wrap. At 10 dealloc/sec/slot (extreme), wrap takes 13.6 years. ABA risk per-slot is below cosmic-ray-bit-flip rate. Niche → can pack with `unit_index` into a single 8-byte word if PGO shows benefit (deferred — D2-future).
- NULL sentinel for "dead": branchless `Option<NonNull>` semantics. `null` is a value the slab can never produce (slab base ≠ 0, slot stride ≠ 0). Saves the 1-byte discriminant of `Option<...>`.
- `#[repr(C)]`: stable field order for `unsafe` reinterpretation if we ever need to atomically read both u32s as one u64 (lock-free read in Phase 9).

**Layout asserts** (place in `entity_inland.rs`):
```rust
const _: () = assert!(std::mem::size_of::<EntityInland>() == 16);
const _: () = assert!(std::mem::align_of::<EntityInland>() == 8);
// Offsets — load-bearing for the read path's pointer arithmetic
const _: () = assert!(std::mem::offset_of!(EntityInland, archetype_ptr) == 0);
const _: () = assert!(std::mem::offset_of!(EntityInland, unit_index) == 8);
const _: () = assert!(std::mem::offset_of!(EntityInland, generation) == 12);
```

**Alternatives rejected**:
- **`archetype_idx: u16` (slab slot index)**: 2 B vs 8 B. Saves 6 B per inland → 6 KB per 1024 entities. But requires bundle-base + idx*stride arithmetic on every lookup, which means `EntityInland` access still costs the bundle-base pointer load. Strictly worse on cache (one extra load per hit). ✗
- **Tagged pointer `(archetype_ptr | dead_bit)`**: clever but no real win — null is already a "dead" tag at zero cost. ✗
- **`generation: u16`**: 2 B too few — 65K dealloc wraps occur per slot per minute under stress. ✗

**Trade-off**:
- 16 B vs current 24 B — 33% reduction. More inlands per cache line for swap_remove path.
- Pointer adds Miri/UB risk if invalidated. Mitigated by slab stability invariant (D1).

### D3. `EntityMaster` — direct `Vec<EntityInland>` indexed by `EntityId.0`, parallel `active_ids: Vec<EntityId>`

```rust
pub struct EntityMaster {
    /// Sparse-indexed by EntityId.0. Dead slots have archetype_ptr == null.
    /// Length grows monotonically with the maximum-ever EntityId.
    /// `Vec::push` only happens on fresh ID allocation; recycled IDs reuse existing slots.
    entities_inland: Vec<EntityInland>,

    /// Dense list of currently-active EntityIds. Used by `iter_entities` (O(active)).
    /// Each active entity's index in `active_ids` is stored... nowhere — see below.
    active_ids: Vec<EntityId>,

    /// Free list of recycled EntityIds.
    free_entity_ids: Vec<EntityId>,

    /// Next fresh EntityId.
    next_entity_id: EntityId,
}
```

**iter_entities behavior**: walk `active_ids`. For each `EntityId`, load `entities_inland[id.0]` and produce `Entity { id, generation: inland.generation as usize }`. O(active). No regression vs current `SparseMap::active_indices`.

**Question: where does `active_ids` index live, so swap-remove on deallocate is O(1)?**

Storing `active_id_index: u32` in `EntityInland` would push it to 20 B → 24 B alignment. We avoid that by using a **separate sparse index** stored alongside, NOT in `EntityInland`:

```rust
pub struct EntityMaster {
    entities_inland: Vec<EntityInland>,             // 16 B/slot, hot read path
    /// Reverse map: for active entity at sparse_to_active[id.0],
    /// gives the index into active_ids. u32::MAX = "not active".
    sparse_to_active: Vec<u32>,                     // 4 B/slot, cold (only touched on register/deallocate)
    active_ids: Vec<EntityId>,                      // 8 B/active entity
    free_entity_ids: Vec<EntityId>,
    next_entity_id: EntityId,
}
```

Cost analysis:
- Hot read path (`get_component_raw`): only touches `entities_inland[id.0]` — 1 cache line. ✓
- `register_entity`: writes `entities_inland[id.0]`, pushes to `active_ids`, writes `sparse_to_active[id.0] = active_ids.len() - 1`. 3 cache lines touched. Once per entity creation, dominated by `Archetype::create_entity` anyway.
- `deallocate_entity`: same 3 lines + swap_remove from `active_ids` + update `sparse_to_active` of the swapped entity. Still O(1).
- `iter_entities`: walks `active_ids` only — touches 1 line per 8 entities (good locality). 

Memory: per EntityId slot, 16 + 4 = **20 B** vs current ~16 (`Entity` 16 B + `SparseMap` ~24 B / active) — slight reduction overall and dramatically better locality.

**Decision**: keep `active_ids` + `sparse_to_active` parallel index. Pay 4 B/slot to keep `iter_entities` O(active) and `register/deallocate` O(1). This is non-negotiable: dropping `iter_entities` to O(capacity) would degrade `query_entities` perf on sparse populations.

### D4. `Archetype::columns` — packed `[Column; MAX_COMPONENTS]` array, 16 B per entry

```rust
/// Pre-resolved component pointer + stride for the hot read path.
/// `ptr.is_null()` ⇔ this archetype has no pool for the component_id at this index.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Column {
    /// Base pointer to the component pool's buffer (== ComponentPool::buffer_ptr()).
    /// NULL when the column is absent.
    ptr: *mut u8,           // 8 B, offset 0
    /// Component size in bytes (== ComponentPool::component_layout().size()).
    /// `unit_index * stride` gives the offset into ptr.
    stride: u32,            // 4 B, offset 8
    /// Reserved for future change-detection tick or flags. Zero today.
    _pad: u32,              // 4 B, offset 12
}
const _: () = assert!(std::mem::size_of::<Column>() == 16);
const _: () = assert!(std::mem::align_of::<Column>() == 8);
```

**Why packed (vs separate `column_ptrs: [*mut u8; 512]` + `column_strides: [u32; 512]`)**:
- Single cache line holds 4 `Column` entries → for an archetype with 4 components in adjacent IDs, all hot data fits in 1 line.
- Separate arrays would force **2 cache misses per lookup**: one for the ptr array, one for the stride array, at unrelated 4 KB / 2 KB offsets respectively.
- Total per-archetype overhead: 512 × 16 B = **8 KB columns table**. Times 1024 archetypes worst case = 8 MB. Acceptable.
- Stride read happens unconditionally in the lookup; pairing with ptr is the right axis.

**`Archetype` new field layout**:
```rust
pub struct Archetype {
    /// 8 KB — hot read path lookup table. Indexed by ComponentId.0.
    /// Placed FIRST so it's at fixed offset 0 from a *const Archetype.
    columns: [Column; MAX_COMPONENTS],          // 8192 B, offset 0
    // --- everything below is cold for random access ---
    id: ArchetypeId,                            // 8 B
    component_pools: ComponentPoolBundle,       // owns the actual pools
    current_index: usize,
    signature: ArchetypeSignature,
    arena: *const Arena,
    component_ids: Vec<ComponentId>,
    entity_ids: Vec<EntityId>,
}
```

**Estimated `Archetype` size**: 8192 + ~200 = ~**8.4 KB**. Slab of 1024 = ~8.6 MB. Still acceptable.

**Refresh contract** (D5): `columns[c]` is the single source of truth for the read path. Any operation that mutates a `ComponentPool`'s buffer pointer (today: pool creation only; future: arena defrag, pool growth) MUST invoke `Archetype::refresh_column(c)` immediately afterward.

```rust
impl Archetype {
    /// Re-syncs columns[component_id.0] with the current pool state. Called
    /// after `component_pools.add_pool(...)` and reserved for future arena grow.
    #[inline]
    fn refresh_column(&mut self, component_id: ComponentId) {
        match self.component_pools.get_pool(component_id) {
            Some(pool) => {
                self.columns[component_id.0] = Column {
                    ptr: pool.buffer_ptr() as *mut u8,
                    stride: pool.component_layout().size() as u32,
                    _pad: 0,
                };
            }
            None => {
                self.columns[component_id.0] = Column { ptr: core::ptr::null_mut(), stride: 0, _pad: 0 };
            }
        }
    }

    /// Refreshes the entire columns table from component_pools. Called on
    /// future arena-grow events. Not on hot path.
    #[cold]
    fn refresh_all_columns(&mut self) {
        for col in self.columns.iter_mut() {
            *col = Column { ptr: core::ptr::null_mut(), stride: 0, _pad: 0 };
        }
        // Walk only the populated component_ids to keep this O(N) not O(MAX_COMPONENTS).
        for &cid in self.component_ids.iter() {
            self.refresh_column(cid);
        }
    }
}
```

### D5. `ComponentPoolBundle` — kept for mutation, bypassed on read

- The bundle still owns `pools: Vec<ComponentPool>` (it must, for memory ownership and per-pool mutation paths like `swap_remove`, `add_typed`, etc.).
- The `sparse_indexes: SparseMap<InlandPoolId>` is still consulted on **mutation** paths (`add_component`, `set_component`, `swap_remove_unit`).
- The **read path** bypasses `sparse_indexes` entirely — `Archetype::columns[component_id.0]` is the answer.

**Invariant** (must hold at every entry/exit of any `Archetype`/`ComponentPoolBundle` method):
```
For all component_id ∈ [0, MAX_COMPONENTS):
  archetype.columns[component_id.0].ptr != null
    ⇔
  archetype.component_pools.get_pool(component_id).is_some()

And when columns[c].ptr != null:
  columns[c].ptr   == component_pools.get_pool(c).unwrap().buffer_ptr() as *mut u8
  columns[c].stride == component_pools.get_pool(c).unwrap().component_layout().size() as u32
```

Enforced by `debug_assert!` in `Archetype::create_by_ids` epilogue and in every test.

### D6. Read-only path API — new fast `get_component_raw`

```rust
impl EcsMaster {
    /// Fast random access: 3 cache lines, ~12 ns.
    /// 1. entities_inland[id.0]   — 1 line
    /// 2. archetype.columns[c.0]  — 1 line
    /// 3. ptr.add(unit * stride)  — arithmetic only; final line is the component itself
    #[inline]
    pub fn get_component_raw(&self, entity: Entity, component_id: ComponentId) -> Option<*const u8> {
        // Line 1: entity_master.entities_inland[entity.id().0]
        let inland = self.entity_master.entities_inland.get(entity.id().0)?;

        // Combined null + generation check. Both fields are in the same cache line
        // already loaded by the get() above; the compiler folds these into a single
        // branch path. Order chosen so null check dominates (cold path = dead slot).
        if inland.archetype_ptr.is_null() {
            return None;
        }
        if inland.generation != entity.generation() as u32 {
            return None;
        }

        // Line 2: archetype.columns[c.0]
        // SAFETY: U1, U2, U3 (see Invariants section).
        let archetype = unsafe { &*inland.archetype_ptr };

        // component_id.0 < MAX_COMPONENTS validated by the type system (debug_assert).
        debug_assert!(component_id.0 < MAX_COMPONENTS);
        // SAFETY: U4. `columns` length is MAX_COMPONENTS; bounded by debug_assert.
        let column = unsafe { archetype.columns.get_unchecked(component_id.0) };
        if column.ptr.is_null() {
            return None;
        }

        // Line 3: component itself.
        // SAFETY: U5, U6.
        Some(unsafe {
            column.ptr.add((inland.unit_index as usize) * (column.stride as usize)) as *const u8
        })
    }
}
```

**Typed wrapper**:
```rust
impl EcsMaster {
    #[inline]
    pub fn get_component<T: Component>(&self, entity: Entity) -> Option<&T> {
        let raw = self.get_component_raw(entity, T::component_id())?;
        // SAFETY: U7.
        Some(unsafe { &*(raw as *const T) })
    }
}
```

The TypeId debug-assert from `ComponentPool::get_typed` is **not** replicated here — to do so would require a registry lookup (one extra cache line for the layout entry) in debug, which is acceptable but not free. We add an opt-in `debug_assert_typeid_matches::<T>(component_id)` helper in `component_registry` and call it once at the start of `get_component<T>` in debug only. Release builds skip it entirely.

### D7. Mutable path — `archetype_ptr: *mut Archetype`

`EntityInland::archetype_ptr` is **`*mut Archetype`** (not `*const`). Rationale:
- Under `&mut EcsMaster`, the caller has unique borrow on the whole engine, including (transitively) the slab. Casting to `&mut Archetype` is sound.
- Storing `*mut` everywhere avoids provenance laundering on the mutable path.

```rust
impl EcsMaster {
    #[inline]
    pub fn get_component_raw_mut(&mut self, entity: Entity, component_id: ComponentId) -> Option<*mut u8> {
        let inland = *self.entity_master.entities_inland.get(entity.id().0)?;
        if inland.archetype_ptr.is_null() { return None; }
        if inland.generation != entity.generation() as u32 { return None; }
        debug_assert!(component_id.0 < MAX_COMPONENTS);
        // SAFETY: U1-U4, plus &mut self gives exclusive access to the slab.
        let archetype = unsafe { &mut *inland.archetype_ptr };
        let column = unsafe { archetype.columns.get_unchecked(component_id.0) };
        if column.ptr.is_null() { return None; }
        // SAFETY: U5, U6. `&mut self` ⇒ exclusive access to the component.
        Some(unsafe {
            column.ptr.add((inland.unit_index as usize) * (column.stride as usize))
        })
    }
}
```

**Why we read `inland` by value (`*self...get`)**: the borrow of `entity_master` is released before we take `&mut *archetype_ptr`. Otherwise the borrow checker rejects the simultaneous borrow chain.

### D8. Drop semantics

**EcsMaster field order** stays unchanged: `events`, `entity_master`, `archetype_master`, `arena`. Validation:
- `EntityMaster::Drop`: drops `Vec<EntityInland>`. Each `EntityInland` is `Copy` (raw pointer + u32 + u32); no destructors. The pointers dangle on drop completion, but since they are **never dereferenced** during `EntityMaster::Drop`, no UB. ✓
- `ArchetypeMaster::Drop` → `ArchetypeBundle::Drop`: must walk the occupancy bitset and `ptr::drop_in_place` each occupied `MaybeUninit<Archetype>` slot. `Archetype::Drop` is implicit (just drops `Vec`s and `ComponentPoolBundle`). `ComponentPool::Drop` invokes `drop_fn` on every component (existing behavior, unchanged).
- `Arena::Drop`: last, frees the backing buffer. ✓

Invariant added: `EntityMaster::Drop` MUST NOT dereference any `archetype_ptr`. Documented in the field doc-comment and enforced by inspection.

### D9. Migration impact — 9 step ordered plan

Each step compiles independently. The order is **load-bearing** — earlier steps add new types/APIs without removing old ones; later steps switch call sites and finally remove dead code.

| Step | Files | What changes | Compiles? |
|------|-------|--------------|-----------|
| **0** | `entity_inland.rs` | Add a parallel new struct `EntityInlandFast { archetype_ptr, unit_index: u32, generation: u32 }` next to the old one. Add layout asserts. **Do not remove old `EntityInland`.** | ✓ |
| **1** | `archetype.rs` | Add `pub struct Column`. Add `columns: Box<[Column; MAX_COMPONENTS]>` field to `Archetype` (Box-allocated so Archetype's stack frame stays small). Add `Archetype::refresh_column` / `refresh_all_columns`. In `create_by_ids` and `register_component`, call `refresh_column(c)` after `add_pool`. **Read path still goes through `component_pools.get_pool`.** | ✓ |
| **2** | `archetype_bundle.rs` | Rewrite from scratch with the pinned slab. Old API surface preserved: `get_archetype`, `get_archetype_mut`, `add_archetype`, `remove_archetype`, `iter`, `len`, `is_empty`, `clear`. New helpers: `get_archetype_ptr(archetype_id) -> Option<*mut Archetype>`, `iter_occupied_ptrs() -> impl Iterator<Item = *mut Archetype>`. | ✓ |
| **3** | `entity_master.rs` | Replace `entity_map: SparseMap<EntityInland>` and `entities: Vec<Entity>` with `entities_inland: Vec<EntityInlandFast>`, `sparse_to_active: Vec<u32>`, `active_ids: Vec<EntityId>`. Keep all public methods working (`register_entity`, `deallocate_entity`, `is_entity_valid`, `iter_entities`, etc.) but signatures change: `register_entity` now takes `archetype_ptr: *mut Archetype` instead of `archetype_id: ArchetypeId`. Internal old `EntityInland` is unused after this. | ✓ |
| **4** | `ecs_master.rs` | Rewrite `create_entity` to: validate archetype, allocate entity, get `archetype_ptr` from `archetype_master.archetype_bundle().get_archetype_ptr(archetype_id)`, call into archetype, register entity with the pointer. Rewrite `delete_entity`: read inland by pointer, deref archetype directly. Rewrite `get_component_raw`, `get_component_raw_mut`, `set_component_raw`, `has_entity`, `has_component`, `get_entity_archetype_id` to the new fast path. | ✓ |
| **5** | `archetype_master.rs` | Add `get_archetype_ptr(id) -> Option<*mut Archetype>` wrapper. `clear()` is simplified (slab clear via occupancy bitset, no `ArchetypeBundle::new`). | ✓ |
| **6** | `entity_inland.rs` | Replace old `EntityInland` with the fast one (rename `EntityInlandFast` → `EntityInland`). Remove `archetype_id`/`set_archetype_id`/`set_generation`/`update`/`increment_generation` accessors that are not needed by callers anymore. | ✓ |
| **7** | `archetype.rs` | Switch `Archetype::create_entity`/`init_entity_inland` to take `unit_index` via the new EntityInland. Confirm `columns` invariant holds via debug_assert at end of `create_by_ids`. | ✓ |
| **8** | Tests + bench harness | Update tests that touched `EntityInland::archetype_id()` (replace with helper `EntityInland::archetype_ptr()`). Add Phase 7 micro-benchmarks. | ✓ |

After step 8, the old `entity_map: SparseMap` is gone; the `archetype_to_index: SparseMap` in the bundle is gone; the read path uses zero sparse-map lookups.

### D10. Benchmarks

Add to existing criterion harness at `crates/boyko_ecs/benches/random_access.rs`:

| Bench | What | Acceptance |
|-------|------|------------|
| `bench_get_component_raw_hot` | 10K entities in 1 archetype, random shuffled access of 1K lookups, hot cache | ≤ 15 ns/op (target 10-12) |
| `bench_get_component_raw_cold` | Same as above but flush cache before each lookup | ≤ 80 ns/op (3 cache misses × ~25 ns) |
| `bench_get_component_typed` | Typed wrapper, debug + release builds | release ≤ 13 ns/op |
| `bench_has_entity` | Just generation check, no component | ≤ 5 ns/op |
| `bench_set_component_raw` | 8 B component write | ≤ 18 ns/op |
| `bench_iter_entities_10k` | After mass create/delete pattern (50% alive) | parity with current (no regression > 5%) |
| `bench_create_entity_10k` | Sequential creates | ≤ 5% regression vs current |
| `bench_get_component_stale_generation` | Lookup with wrong generation (must return None fast) | ≤ 8 ns/op |
| `bench_get_component_missing_component` | Lookup for ComponentId not in archetype (column.ptr == null) | ≤ 10 ns/op |

Bench should be run with `RUSTFLAGS="-C target-cpu=native"` and `cargo bench --bench random_access`.

---

## Data structures

### EntityInland (new)
```rust
// File: crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs
//
// PHASE 7 layout — 16 B total. Hot read path touches this struct exclusively
// for entity → archetype + slot resolution.

use crate::ecs::core::archetype::archetype::Archetype;

/// Compact, per-entity routing record. Stored in `EntityMaster::entities_inland`,
/// indexed by `EntityId.0`. Dead entities are represented by `archetype_ptr == null`.
///
/// # Layout (load-bearing)
///
/// ```text
/// offset 0 .. 8 : archetype_ptr  (*mut Archetype, NULL = dead)
/// offset 8 .. 12: unit_index     (u32, index into Archetype.entity_ids / column rows)
/// offset 12..16: generation      (u32, must match Entity.generation truncated)
/// total          : 16 B, aligned 8
/// ```
///
/// 4 inlands fit in one 64-byte cache line — good locality for swap-remove path.
///
/// # Invariants
///
/// - `archetype_ptr.is_null()` ⇒ entity is dead; `unit_index` / `generation` are stale
///   (but reading them is safe — they are POD).
/// - `archetype_ptr != null` ⇒ points into `ArchetypeBundle`'s pinned slab; valid for
///   the lifetime of `EcsMaster` (D1 stability invariant).
/// - `generation == Entity.generation() as u32` ⇔ this inland matches the user-held
///   `Entity` handle (ABA-prevention via 32-bit counter).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EntityInland {
    pub(crate) archetype_ptr: *mut Archetype,
    pub(crate) unit_index: u32,
    pub(crate) generation: u32,
}

// Layout sanity — these break the build if anyone changes the struct.
const _: () = assert!(std::mem::size_of::<EntityInland>() == 16);
const _: () = assert!(std::mem::align_of::<EntityInland>() == 8);
const _: () = assert!(std::mem::offset_of!(EntityInland, archetype_ptr) == 0);
const _: () = assert!(std::mem::offset_of!(EntityInland, unit_index) == 8);
const _: () = assert!(std::mem::offset_of!(EntityInland, generation) == 12);

impl EntityInland {
    /// Constructs a dead inland (NULL archetype_ptr). Used as initial value
    /// in `EntityMaster::entities_inland` when growing the Vec.
    #[inline]
    pub const fn dead() -> Self {
        Self {
            archetype_ptr: core::ptr::null_mut(),
            unit_index: 0,
            generation: 0,
        }
    }

    /// Constructs an active inland. Caller must guarantee `archetype_ptr`
    /// points to a live slot in `ArchetypeBundle`'s pinned slab.
    #[inline]
    pub fn new(archetype_ptr: *mut Archetype, unit_index: u32, generation: u32) -> Self {
        Self { archetype_ptr, unit_index, generation }
    }

    /// Returns true iff this inland represents a live, registered entity.
    #[inline]
    pub fn is_alive(&self) -> bool {
        !self.archetype_ptr.is_null()
    }

    #[inline]
    pub fn archetype_ptr(&self) -> *mut Archetype { self.archetype_ptr }

    #[inline]
    pub fn unit_index(&self) -> u32 { self.unit_index }

    #[inline]
    pub fn generation(&self) -> u32 { self.generation }

    #[inline]
    pub fn set_unit_index(&mut self, unit_index: u32) { self.unit_index = unit_index; }
}
```

### Column
```rust
// File: crates/boyko_ecs/src/ecs/core/archetype/archetype.rs
//
// PHASE 7 — pre-resolved component pointer + stride for fast random access.

/// Hot-path lookup entry. One per (Archetype × ComponentId) pair.
/// `ptr.is_null()` ⇔ this archetype has no pool for the component ID.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Column {
    pub(crate) ptr: *mut u8,
    pub(crate) stride: u32,
    pub(crate) _pad: u32,
}

const _: () = assert!(std::mem::size_of::<Column>() == 16);
const _: () = assert!(std::mem::align_of::<Column>() == 8);

impl Column {
    #[inline]
    pub const fn null() -> Self {
        Self { ptr: core::ptr::null_mut(), stride: 0, _pad: 0 }
    }
}
```

### Archetype (new column table)
```rust
pub struct Archetype {
    /// 8 KB hot lookup table. Indexed by `ComponentId.0`. Heap-allocated via
    /// `Box<[Column; MAX_COMPONENTS]>` so the `Archetype` stack frame stays
    /// at ~200 B even though the table is large; the slab in ArchetypeBundle
    /// then holds pointer-sized struct + 8 KB heap allocation per occupied slot.
    ///
    /// PHASE 7 invariant: columns[c].ptr.is_null() ⇔ !component_pools.contains(c).
    /// When non-null: columns[c].ptr == component_pools.get_pool(c).unwrap().buffer_ptr()
    /// and columns[c].stride == component_pools.get_pool(c).unwrap().component_layout().size().
    columns: Box<[Column; MAX_COMPONENTS]>,

    id: ArchetypeId,
    component_pools: ComponentPoolBundle,
    current_index: usize,
    signature: ArchetypeSignature,
    arena: *const Arena,
    component_ids: Vec<ComponentId>,
    entity_ids: Vec<EntityId>,
}
```

Reason for `Box<[Column; MAX_COMPONENTS]>` rather than inline `[Column; MAX_COMPONENTS]`:
- Inline: `Archetype` is 8.5 KB. Slab of 1024 = 8.7 MB upfront. Acceptable but heavy.
- Boxed: `Archetype` is ~200 B. Slab of 1024 = ~200 KB upfront; the 8 KB column table is allocated only when a slot is occupied. Saves 8 MB of unused memory for unused slots. Cost: one extra pointer deref to reach `columns[c]` — **but it's the same line as `archetype`'s own cache line, prefetched by the deref of `archetype_ptr`**.

Actually wait — the box deref adds a load. Let me reconsider. The cache trace:
- Inline: `*archetype_ptr` brings in line N. `columns` starts at offset 0 of `Archetype`, so reading `columns[c]` requires loading line N + (c × 16 / 64) → 2nd cache line. **2 lines total.**
- Boxed: `*archetype_ptr` brings in line N (which holds the box pointer at offset 0). Dereffing the box brings in line M (the columns heap allocation), then reading `columns[c]` is the same line M. **2 lines total**, plus the cost of the box-pointer load chain.

The boxed version costs **one extra cache miss** if line M is cold. Verdict: **inline `[Column; MAX_COMPONENTS]`** with the 8 MB upfront cost. We're optimizing for hot path, and memory is cheap.

Revised:
```rust
pub struct Archetype {
    /// 8 KB inline hot lookup table. Indexed by ComponentId.0.
    columns: [Column; MAX_COMPONENTS],   // offset 0, size 8192

    // --- cold fields below ---
    id: ArchetypeId,
    component_pools: ComponentPoolBundle,
    current_index: usize,
    signature: ArchetypeSignature,
    arena: *const Arena,
    component_ids: Vec<ComponentId>,
    entity_ids: Vec<EntityId>,
}
```

Slab footprint: 1024 × ~8.4 KB = ~8.6 MB. Allocated once via `Box<[MaybeUninit<Archetype>; 1024]>`. **This is the right trade-off.**

Wait — can we even fit this in `MaybeUninit`? Yes, Rust accepts `MaybeUninit<T>` for any T regardless of size. Direct stack-allocating the slab is what we must avoid (it would blow stack on `EcsMaster::new`), but `Box::new_uninit_slice` and similar APIs handle it cleanly.

### ArchetypeBundle (new pinned slab)
```rust
// File: crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs
//
// PHASE 7 rewrite. Pinned slab of 1024 archetype slots. Stable addresses.
// Bitset-based occupancy. No Vec<Archetype>.

use std::mem::MaybeUninit;
use std::pin::Pin;
use boyko_utils::bit_mask::bit_set::BitSet;

const MAX_ARCHETYPES: usize = 1024;

/// Pinned slab of Archetype slots. Addresses are stable for the bundle's lifetime.
///
/// # Storage
///
/// `slots`: `Pin<Box<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>>` — 1024 archetype
/// slots, allocated upfront in `new()`. Each slot is either:
///   - **Occupied** (`occupied[slot_idx] == 1`): contains a fully-initialized
///     `Archetype`. `&*ptr` of its address is sound.
///   - **Empty** (`occupied[slot_idx] == 0`): uninitialized memory. MUST NOT
///     be read or dropped.
///
/// `occupied`: 16-element `[u64; 16]` bitset (1024 bits). bit `i` is set iff
/// slot `i` is occupied. Iteration uses `trailing_zeros` for sparse scans.
///
/// `id_to_slot`: `Vec<u16>`, indexed by `ArchetypeId.0`. `u16::MAX` (`0xFFFF`)
/// = no archetype with this id. The vector grows monotonically with the max
/// archetype id ever assigned; slots are freed on `remove_archetype` but the
/// `id_to_slot` entry is set to sentinel rather than swap-removed (otherwise
/// the indirection breaks under reuse).
///
/// `free_slots`: `Vec<u16>` of recycled slot indices for reuse on `add_archetype`.
pub struct ArchetypeBundle {
    /// 1024 × size_of::<Archetype>() ≈ 8.6 MB. Pinned via Box; address stable.
    slots: Pin<Box<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>>,
    /// 1024-bit occupancy. 128 B.
    occupied: [u64; 16],
    /// ArchetypeId.0 → slot index (u16::MAX = absent).
    id_to_slot: Vec<u16>,
    /// LIFO stack of freed slot indices.
    free_slots: Vec<u16>,
    /// Number of currently-occupied slots (== popcount(occupied)).
    count: usize,
}

impl ArchetypeBundle {
    #[cold]
    pub fn new() -> Self;

    /// Returns `*mut Archetype` for the given archetype_id, or None if absent.
    /// **THIS IS THE HOT API** — used by EntityMaster to fill EntityInland.
    #[inline]
    pub fn get_archetype_ptr(&self, archetype_id: ArchetypeId) -> Option<*mut Archetype>;

    /// Safe shared-borrow accessor.
    #[inline]
    pub fn get_archetype(&self, archetype_id: ArchetypeId) -> Option<&Archetype>;

    /// Safe exclusive-borrow accessor.
    #[inline]
    pub fn get_archetype_mut(&mut self, archetype_id: ArchetypeId) -> Option<&mut Archetype>;

    /// Inserts an Archetype into a free slot. Returns the assigned slot index
    /// (for ArchetypeMaster's debug bookkeeping).
    pub fn add_archetype(&mut self, archetype: Archetype) -> u16;

    /// Constructs an Archetype in place in a free slot — avoids the Archetype
    /// stack copy. Returns the assigned slot index.
    pub fn add_archetype_from_components(
        &mut self,
        archetype_id: ArchetypeId,
        component_ids: &[ComponentId],
        arena: &Arena,
    ) -> u16;

    /// Drops the archetype at the given id and frees its slot. Returns true on
    /// success. Note: this DOES NOT compact `slots` — the slab is fixed.
    pub fn remove_archetype(&mut self, archetype_id: ArchetypeId) -> bool;

    /// Iterates occupied archetype slots. Uses bitset trailing_zeros traversal
    /// for O(occupied) cost.
    pub fn iter(&self) -> impl Iterator<Item = &Archetype>;

    /// Same as `iter`, raw pointers — for Phase 9 parallel scheduler hand-off.
    pub fn iter_occupied_ptrs(&self) -> impl Iterator<Item = *mut Archetype>;

    #[inline]
    pub fn len(&self) -> usize { self.count }

    #[inline]
    pub fn is_empty(&self) -> bool { self.count == 0 }

    /// Drops all occupied archetypes and clears occupancy.
    pub fn clear(&mut self);
}

impl Drop for ArchetypeBundle {
    /// Walks `occupied` bitset and drops each live `Archetype` in place.
    /// Empty slots remain `MaybeUninit` and are not touched.
    fn drop(&mut self);
}
```

### EntityMaster (new)
```rust
// File: crates/boyko_ecs/src/ecs/core/entity/entity_master.rs

pub struct EntityMaster {
    /// Sparse-indexed by EntityId.0. `entities_inland[i].archetype_ptr.is_null()`
    /// ⇔ slot i is dead (never allocated or deallocated).
    /// PHASE 7 hot read path: 1 cache line per random lookup.
    entities_inland: Vec<EntityInland>,

    /// Reverse index from EntityId.0 to position in `active_ids`. `u32::MAX` = absent.
    /// Touched only on register / deallocate; cold for random reads.
    sparse_to_active: Vec<u32>,

    /// Dense list of currently-active EntityIds. Drives O(active) `iter_entities`.
    active_ids: Vec<EntityId>,

    /// LIFO free list for recycled EntityIds.
    free_entity_ids: Vec<EntityId>,

    /// Next fresh EntityId.
    next_entity_id: EntityId,
}

impl EntityMaster {
    pub fn new() -> Self;
    pub fn with_capacity(capacity: usize) -> Self;

    /// Allocates a fresh or recycled EntityId. Returned Entity carries the
    /// current generation (incremented at last deallocate, if any).
    /// PHASE 7: generation read from entities_inland[id.0].generation, not from a
    /// separate `entities: Vec<Entity>` array.
    pub fn allocate_entity(&mut self) -> Entity;

    /// Registers an active entity with its archetype pointer and unit_index.
    /// Writes `entities_inland[id.0] = EntityInland { archetype_ptr, unit_index, generation }`.
    /// Also pushes id to `active_ids` and updates `sparse_to_active[id.0]`.
    pub fn register_entity(
        &mut self,
        entity: Entity,
        archetype_ptr: *mut Archetype,
        unit_index: u32,
    );

    /// Marks an entity dead: writes `archetype_ptr = null` and bumps generation.
    /// Removes from `active_ids` via swap_remove + updates the swapped entity's
    /// `sparse_to_active` entry. Pushes id to free list.
    pub fn deallocate_entity(&mut self, entity: Entity) -> Option<EntityInland>;

    /// Fast `&EntityInland` accessor by Entity handle. Validates generation.
    /// **Hot path:** 1 cache line, branchless ish.
    #[inline]
    pub fn get_entity_inland(&self, entity: Entity) -> Option<&EntityInland>;

    /// Updates only the unit_index field. Used after swap_remove in an archetype.
    pub fn update_entity_unit_index(&mut self, entity: Entity, new_unit_index: u32) -> bool;

    /// Returns true iff entity exists and generation matches.
    #[inline]
    pub fn is_entity_valid(&self, entity: Entity) -> bool;

    pub fn get_entity(&self, entity_id: EntityId) -> Option<Entity>;
    pub fn iter_entities(&self) -> impl Iterator<Item = Entity> + '_;
    pub fn entity_count(&self) -> usize { self.active_ids.len() }
    pub fn recycled_entity_count(&self) -> usize { self.free_entity_ids.len() }
    pub fn next_entity_id(&self) -> EntityId { self.next_entity_id }
    pub fn capacity(&self) -> usize { self.entities_inland.len() }
    pub fn clear(&mut self);

    // C-007 rewind (unchanged semantics).
    pub(crate) fn rewind_allocate(&mut self, entity: Entity) -> bool;
}
```

---

## Public API (new + changed signatures)

### Changed
| Old | New |
|-----|-----|
| `EntityInland::new(archetype_id, unit_index: InlandPoolId, generation)` | `EntityInland::new(archetype_ptr: *mut Archetype, unit_index: u32, generation: u32)` |
| `EntityInland::archetype_id() -> ArchetypeId` | `EntityInland::archetype_ptr() -> *mut Archetype` |
| `EntityInland::unit_index() -> InlandPoolId` | `EntityInland::unit_index() -> u32` |
| `EntityMaster::register_entity(entity, archetype_id, unit_index: InlandPoolId)` | `EntityMaster::register_entity(entity, archetype_ptr: *mut Archetype, unit_index: u32)` |
| `EntityMaster::update_entity_unit_index(entity, InlandPoolId)` | `EntityMaster::update_entity_unit_index(entity, u32)` |

### New
| Signature | Purpose |
|-----------|---------|
| `ArchetypeBundle::get_archetype_ptr(ArchetypeId) -> Option<*mut Archetype>` | Hot lookup, returns slab address |
| `ArchetypeMaster::get_archetype_ptr(ArchetypeId) -> Option<*mut Archetype>` | Thin wrapper |
| `Archetype::columns(&self) -> &[Column; MAX_COMPONENTS]` | Read-only column view (for tests) |
| `Archetype::refresh_column(&mut self, ComponentId)` | Re-sync column entry after pool mutation |
| `Archetype::refresh_all_columns(&mut self)` | Re-sync all columns (Phase-future: arena grow) |
| `EcsMaster::get_component<T: Component>(entity) -> Option<&T>` | Typed fast read |
| `EcsMaster::get_component_mut<T: Component>(entity) -> Option<&mut T>` | Typed fast write |
| `Column::null() -> Column` | Sentinel constructor |
| `EntityInland::dead() -> EntityInland` | Sentinel constructor |
| `EntityInland::is_alive(&self) -> bool` | Branchless null check helper |

### Unchanged public API (preserved)
- `EcsMaster::create_entity / spawn_one / spawn_two`
- `EcsMaster::delete_entity`
- `EcsMaster::has_entity / has_component / get_entity_archetype_id`
- `EcsMaster::iter_entities / entity_count / archetype_count`
- `EcsMaster::query_entities` (unchanged signature; internal impl uses fast path)
- `EcsMaster::events / send_event / events_of / update_events`

---

## Invariants & SAFETY contracts

Numbered for the developer to paste verbatim into the SAFETY blocks.

### Slab stability invariants (set on construction, maintained until Drop)

**U1** — *Archetype pointer validity*:
`EntityInland::archetype_ptr` either equals `core::ptr::null_mut()` OR is the slab base address + (slot_index × size_of::<Archetype>()) for some `slot_index` in `[0, MAX_ARCHETYPES)`. The slab is allocated once via `Box::pin` and its address is stable for the bundle's entire lifetime. No `Vec::push` reallocation can invalidate this pointer.

**U2** — *Archetype slot initialization*:
When `EntityInland::archetype_ptr != null`, the slot at that address has been fully initialized via `MaybeUninit::write` in `ArchetypeBundle::add_archetype` / `add_archetype_from_components` and has not been dropped via `MaybeUninit::assume_init_drop`. Bit `slot_index` of `ArchetypeBundle::occupied` is set.

**U3** — *Archetype lifetime ≥ EntityInland reachability*:
`EntityMaster` and `ArchetypeBundle` are both owned by `EcsMaster`. Field drop order is `events` → `entity_master` → `archetype_master` → `arena`. Therefore `entity_master` (which holds `EntityInland`s with archetype pointers) drops BEFORE `archetype_master` (which owns the slab). `EntityMaster::Drop` does NOT dereference any `archetype_ptr` (documented in field comment + enforced by code review).

### Column lookup invariants

**U4** — *Columns array bounds*:
`component_id.0 < MAX_COMPONENTS` is checked by `debug_assert!` at every call site. The release-build `get_unchecked` is sound because `MAX_COMPONENTS` is hard-capped by `ComponentRegistry` (panics on `>= MAX_COMPONENTS`), and `ComponentId` newtype only contains values that have been minted through the registry.

**U5** — *Column ptr provenance*:
When `Archetype::columns[c].ptr != null`, that pointer was minted from `ComponentPool::buffer_ptr()` which itself returns `self.buffer.as_ptr()` where `buffer` is a `NonNull<u8>` obtained from `Arena::allocate_layout`. The arena allocation lives for the arena's lifetime, which transitively outlives the archetype (drop order: archetype before arena).

**U6** — *Column offset bounds*:
`unit_index < archetype.entity_count() <= max_components_per_pool`. The pool was allocated `max_components × component_layout.size()` bytes; the offset `unit_index × stride` is therefore within the pool's allocation. `unit_index` is `u32`; widening to `usize` for multiplication never overflows on 64-bit platforms (u32 × u32 ≤ u64 ≤ usize).

### Typed cast invariants

**U7** — *Typed pointer cast*:
`column.stride == size_of::<T>()` and `column.ptr` is aligned to at least `align_of::<T>()` because the underlying `ComponentPool::buffer` was allocated with `component_layout.align()`, which equals `align_of::<T>()` after the registry layout was set via `register_layout::<T>` (TypeId match enforced by debug_assert).

### Pointer/integer arithmetic

**U8** — *swap_remove pointer rewrite*:
When `Archetype::remove_entity` performs a swap_remove, the moved entity's `EntityInland.unit_index` is updated via `EntityMaster::update_entity_unit_index`. The archetype_ptr does NOT change (same archetype). The component pool's buffer is unchanged in base address — only contents shift. No column refresh needed.

### Mutable casts

**U9** — *Exclusive `&mut Archetype` from `*mut`*:
Under `&mut EcsMaster`, casting `*mut Archetype` to `&mut Archetype` is sound because:
- `EcsMaster` transitively owns the slab.
- No other live `&Archetype` exists (the `&mut EcsMaster` borrow excludes all aliasing).
- The slab pointer is valid (U1, U2).
- Single-threaded execution (Arena `!Send + !Sync` propagates).

### Column refresh discipline

**U10** — *Columns ⇔ pools invariant*:
After every call that may add or remove a `ComponentPool` from `ComponentPoolBundle`, the corresponding `Archetype::columns[c]` entry is refreshed via `refresh_column(c)` BEFORE the function returns. Today this is only `register_component` and `create_by_ids`. Future events (arena grow, pool shrink) MUST call `refresh_column` or `refresh_all_columns` immediately after the pool mutation. `debug_assert!` in archetype methods checks the invariant.

---

## Performance characteristics

### Hot path walkthrough — `get_component_raw(entity, comp_id)` after Phase 7

```
                                                                       Cache line  Cumulative ns
1. inland = self.entity_master.entities_inland.get(entity.id().0)?;    [Line 1]    ~3 ns
   - Loads 16 B from Vec<EntityInland>; index check folds into the Option.
   - 4 inlands per line; high temporal locality for sequential workloads.

2. inland.archetype_ptr.is_null() → no                                 same line   ~3 ns
   inland.generation == entity.gen → yes                               same line

3. archetype = unsafe { &*inland.archetype_ptr };                       [Line 2]    ~6 ns
   - Loads first 64 B of Archetype struct from the slab.
   - For columns at offset 0..8192, columns[c.0] is at offset c.0*16.
     For c.0 ∈ [0, 4), columns[c.0] is in Line 2 (the same line we just loaded).
     For c.0 ∈ [4, 8), columns[c.0] is in the next line.

4. column = archetype.columns.get_unchecked(c.0)                       [Line 2 or 3] ~9 ns
   column.ptr.is_null() → no
   Reads (ptr, stride) — 12 B in one access.

5. column.ptr.add(unit_index * stride)                                  [Line 4]   ~12 ns
   - Pure arithmetic; one shift + one add (the multiplication folds for power-of-2 strides).
   - The COMPONENT itself is now loaded — this is the 3rd "data" line (1st was inland, 2nd was archetype/columns).
   - On x86_64, a memory access at a known offset triggers HW prefetch for next-cache-line.

TOTAL: 3 data cache lines, ~12 ns (matches Bevy Table model).
```

**Branch behavior**:
- `inland.archetype_ptr.is_null()`: branch on a value already in a register from step 1. Predictable (almost always taken-not = alive entity in steady state).
- `inland.generation != entity.gen`: also predictable (almost always equal in steady state).
- `column.ptr.is_null()`: predictable (queries are typed; component_id ∈ archetype is the common case).

**Branchless candidate** (not in MVP, deferred): replace the three checks with a single `is_null` on the final computed pointer, using `select_unpredictable` to fold paths. PGO determines worth.

### Pessimization paths

| Scenario | Behavior |
|----------|----------|
| Dead entity (stale handle) | Returns None at step 2; ~3 ns. |
| Component absent | Returns None at step 4; ~9 ns. |
| First access (cold cache) | Lines 1, 2/3, 4 are all cold misses → ~25 ns/line × 3 = ~75 ns. |
| Random access pattern across many archetypes | Each archetype access is a fresh slab line; mitigated by slab being contiguous (sequential archetype indices share lines). |

### Comparison to today (40 ns / 9 lines)

| Step | Old (cache lines) | New (cache lines) | Saved |
|------|------------------:|------------------:|------:|
| Entity validity check | 3 | 1 (combined with deref) | 2 |
| Archetype lookup | 2 | 0 (direct ptr) | 2 |
| Pool lookup | 3 | 0 (in columns) | 3 |
| Component address | 1 | 1 | 0 |
| **Total** | **9** | **2-3** | **6-7** |

Empirical estimate ratio: 12 ns / 40 ns = **3.3× speedup**. Matches Bevy reports of "~10ns / 3 cache lines".

---

## Memory budget (per master)

| Structure | Old | New | Delta |
|-----------|----:|----:|------:|
| `ArchetypeBundle::archetypes: Vec<Archetype>` | ~120 B × N archetypes (heap) | — | — |
| `ArchetypeBundle::archetype_to_index: SparseMap` | ~(64 B + 24 B × N) | — | — |
| **New** `ArchetypeBundle::slots` (pinned slab) | — | 1024 × ~8.4 KB = **~8.6 MB** | +8.6 MB |
| **New** `ArchetypeBundle::occupied` (bitset) | — | 128 B | +128 B |
| **New** `ArchetypeBundle::id_to_slot: Vec<u16>` | — | 2 B × N | +2N B |
| **New** `ArchetypeBundle::free_slots: Vec<u16>` | — | 2 B × removed | +trivial |
| **New** `Archetype::columns: [Column; 512]` | — | 8 KB inline (in slab) | included in slab |
| `EntityMaster::entities: Vec<Entity>` | 16 B × E | — | -16E B |
| `EntityMaster::entity_map: SparseMap<EntityInland>` | ~24 B sparse + 24 B × E_active dense | — | — |
| **New** `EntityMaster::entities_inland: Vec<EntityInland>` | — | 16 B × E | +16E B |
| **New** `EntityMaster::sparse_to_active: Vec<u32>` | — | 4 B × E | +4E B |
| **New** `EntityMaster::active_ids: Vec<EntityId>` | — | 8 B × E_active | +8E_active B |
| Net entity overhead per fresh slot | ~40-48 B (incl. SparseMap dense) | 20 B + 8 B if active | **-12 to -28 B / slot** |

**Net change**:
- +8.6 MB upfront for slab (one-time, regardless of usage).
- -12 to -28 B per entity (savings scale with entity count).
- Crossover: at ~700K entities, the per-entity savings cancel the slab cost. For typical games (100K-1M entities), this is a net wash or slight reduction in heap usage with **much** better cache behavior.

---

## Forward compatibility

### Phase 8 (hybrid sparse-set components)

If we later add sparse-set storage for tag/rare components (Bevy `SparseSet` storage), the `Column` type extends with a discriminant:
```rust
enum Column {
    Table { ptr: *mut u8, stride: u32 },
    SparseSet { sparse_set_ptr: *mut SparseStorage },
}
```
This doesn't break Phase 7 — table columns retain the fast path; sparse columns get a slower lookup. Today's `Column = { ptr, stride, _pad }` keeps the `_pad: u32` exactly for this future tag.

### Phase 9 (parallel scheduler)

- `get_component_raw` is `&self` — multiple workers can hit it in parallel without synchronization.
- `*const Archetype` (cast from `*mut`) deref under `&self` is sound if no `&mut EcsMaster` exists during system execution (which is the scheduler's job to enforce — only one `&mut self` borrow window between system batches).
- For per-column borrow-checking during system dispatch, the scheduler reads `archetype.signature.mask()` (an existing field) to compute conflicts. No new design needed.
- `EntityInland` is `Copy + Send` if we mark it `unsafe impl Send` (it contains a raw pointer that is Send-safe under the same scheduler discipline). Deferred until Phase 9 actually requires it.

### Future arena grow

If `Arena` ever grows by reallocation, all `ComponentPool::buffer` pointers may invalidate. To support this:
- Arena exposes `current_base_ptr() -> *mut u8` and `version: u64` (bumped on grow).
- `Archetype` caches `arena_version: u64`. On every external-mutation operation (or via explicit `refresh()` call), Archetype compares versions and runs `refresh_all_columns` if mismatched.
- Pool buffer relocations are propagated by `ComponentPool::on_arena_relocated(old_base, new_base)` which recomputes `self.buffer` and triggers `Archetype::refresh_column(self.component_id)`.

Today (Phase 7 MVP): the arena is fixed 64 MB; pool buffers never move. The `refresh_column` API exists but is only called on pool creation. **No refresh-on-grow logic is shipped in Phase 7** — `refresh_all_columns` is marked `#[cold]` and reserved for future use.

---

## Tests required

### Unit tests (in respective `mod tests` blocks)

**`entity_inland.rs`**
- `entity_inland_dead_has_null_ptr`
- `entity_inland_dead_is_not_alive`
- `entity_inland_layout_size_is_16`
- `entity_inland_layout_field_offsets_match_design`

**`archetype.rs`**
- `column_null_is_default`
- `archetype_columns_initially_all_null`
- `archetype_create_by_ids_populates_columns_for_listed_components`
- `archetype_register_component_populates_one_column`
- `archetype_columns_invariant_holds_after_create_by_ids` — for every component_id, column.ptr.is_null() XOR component_pools.contains(component_id) returns true at exactly one path.
- `archetype_columns_ptr_matches_pool_buffer_ptr`
- `archetype_columns_stride_matches_pool_layout_size`
- `archetype_columns_for_missing_component_is_null`

**`archetype_bundle.rs`**
- `bundle_new_has_zero_occupied`
- `bundle_add_archetype_returns_stable_ptr`
- `bundle_add_then_get_archetype_ptr_returns_same_address` — *load-bearing for U1*
- `bundle_address_stable_across_many_inserts` — insert 100 archetypes, snapshot first ptr, insert more, verify first ptr unchanged
- `bundle_remove_archetype_frees_slot_for_reuse`
- `bundle_remove_does_not_invalidate_other_pointers` — *load-bearing for U1*
- `bundle_iter_visits_all_occupied`
- `bundle_iter_skips_empty_slots`
- `bundle_drop_drops_all_occupied_archetypes` (use a sentinel component with side-effecting Drop)
- `bundle_drop_does_not_drop_empty_slots` (MaybeUninit safety)
- `bundle_at_capacity_returns_err_or_panics` (1024 archetypes)

**`entity_master.rs`**
- `entity_master_allocate_assigns_sequential_ids`
- `entity_master_register_writes_inland`
- `entity_master_deallocate_nulls_archetype_ptr`
- `entity_master_deallocate_bumps_generation`
- `entity_master_recycled_id_keeps_generation_continuity`
- `entity_master_is_entity_valid_returns_false_for_stale_handle`
- `entity_master_iter_entities_O_active_with_sparse_population` — create 1000, delete 500, ensure iter visits exactly 500
- `entity_master_iter_entities_yields_correct_set_after_recycle`
- `entity_master_rewind_allocate_unchanged_semantics` (C-007 regression)

**`ecs_master.rs`**
- `get_component_raw_returns_correct_ptr_for_first_entity`
- `get_component_raw_returns_none_for_dead_entity`
- `get_component_raw_returns_none_for_wrong_generation`
- `get_component_raw_returns_none_for_absent_component`
- `get_component_raw_handles_recycled_entity_correctly` (ABA test)
- `get_component_after_swap_remove_returns_correct_ptr` — swap_remove an entity; the moved one's lookup must return the new address
- `get_component_typed_returns_T_reference`
- `get_component_typed_debug_asserts_typeid_match`
- `set_component_raw_updates_value`
- `set_component_raw_returns_false_for_dead_entity`
- `has_entity_for_active_returns_true`
- `has_entity_for_dead_returns_false`
- `has_component_returns_true_when_present`
- `delete_entity_then_create_recycles_slot_and_works`

### Integration tests (`tests/phase7_random_access.rs`)

- `phase7_lookup_chain_invariants` — for 1000 random entities across 10 archetypes, verify `get_component_raw` matches the slow path (via `archetype.component_pools.get_pool`).
- `phase7_aba_safety` — create entity, delete, create new at same id with new generation; old handle's get_component_raw returns None.
- `phase7_address_stability_under_archetype_churn` — create N archetypes, snapshot all entity_inland's archetype_ptrs, delete half the archetypes (the entities in those archetypes go too), create new archetypes; surviving entities' `archetype_ptr`s unchanged.
- `phase7_columns_refresh_after_register_component` — create archetype with [A], add entity, then register C; columns[C] now valid, columns[A] unchanged.

### Property tests (`proptest`)

- `prop_get_component_raw_consistent_with_slow_path` — random entity/component sequences vs ground truth.
- `prop_generation_check_rejects_stale_handles` — random create/delete/recreate sequences.

### Miri tests

- `miri test --features miri-test` for SAFETY invariant validation:
  - Slab address stability under `Box::pin`
  - `MaybeUninit` discipline (no read of uninit, no double-drop)
  - Null-pointer-as-sentinel correctness
  - `*mut Archetype` cast under `&mut` (Stacked Borrows)
  - Drop order (entity_master → archetype_master)

### Loom tests (Phase 9 prep, optional now)

Not required for Phase 7 (single-threaded). Marker tests added with `#[ignore]` for the future:
- `loom_get_component_raw_under_shared_self` — concurrent readers, single &self.

### Criterion benchmarks (see D10 table)

---

## Open questions / future work

1. **Q-001 (deferred)**: Should `EntityInland::unit_index` and `generation` be combined into a single `u64` for atomic loads? Required if Phase 9 wants lock-free entity-validity reads from worker threads. Trade-off: minor codegen complication for non-atomic accesses today vs major win under Phase 9. Recommend: keep separate fields for Phase 7; revisit at Phase 9.

2. **Q-002 (deferred)**: Should `Archetype::columns` migrate from inline `[Column; 512]` to a packed dynamic structure (e.g., `SmallVec<[Column; 16]>` with hash lookup) to reduce per-archetype overhead from 8 KB to ~256 B for typical archetypes? Reduces slab from 8.6 MB to ~256 KB. Cost: random access becomes O(log N) per archetype instead of O(1). Recommend: defer until profiling shows the 8 KB hot table actually wastes ICache/DCache pressure.

3. **Q-003**: When `MAX_COMPONENTS` grows from 512 to e.g. 1024 (future), Column table size doubles to 16 KB per archetype. Slab grows to ~17 MB. Hard wall when MAX_COMPONENTS × MAX_ARCHETYPES × size_of::<Column>() > working-set budget. **Mitigation**: hash-based component lookup (Q-002) becomes necessary. Document the threshold (~1024) in `constants.rs`.

4. **Q-004**: `Archetype::component_pools.sparse_indexes` is now redundant with `columns`. We keep it for the mutation path because it's a `SparseMap<InlandPoolId>` and gives us a quick "pool index in Vec<ComponentPool>" answer, which `columns` doesn't provide (columns only has the buffer ptr, not the index in `pools`). Decision: keep `sparse_indexes` until Phase 8 reorganizes the pool storage; flagged for review.

5. **Q-005**: `Vec<EntityInland>` grows by `Vec::push`. When it reallocates, the address of `entities_inland[i]` for an old `i` changes. Does anyone hold a long-lived reference into it? Search confirms: NO — every `get_entity_inland` returns either `&EntityInland` (with lifetime bound to the function call) or `EntityInland` (by Copy). After resize, the next access goes through `.get(id)` again. ✓ Safe.

6. **Q-006**: Future `delete_archetype` after entities migrate out — when an archetype becomes empty, do we GC its slot? If yes, the entities in OTHER archetypes are unaffected (their inlands point to different slots). But the freed slot index goes onto `free_slots` and may be reused by a new archetype. This is the correct behavior. Not a problem.

---

## Changelog vs sketch in user's spec

| Item | Spec | Plan | Why |
|------|------|------|-----|
| `Archetype::columns` location | Suggested as field; layout TBD | Inline `[Column; 512]`, placed at offset 0 of `Archetype` | Inline avoids one cache miss vs `Box`; offset 0 means `*archetype_ptr` brings columns into the same cache line as the dereference |
| `archetype_ptr` mutability | `*const Archetype` with reborrow on `&mut self` | `*mut Archetype` stored directly | Simpler provenance story; under `&mut self` the cast to `&mut` is sound (U9) |
| `MAX_ARCHETYPES` enforcement | Mentioned constant | Slab is fixed 1024 slots; `add_archetype` panics at capacity | Hard cap matches existing `ComponentMask` 512-bit limit |
| `sparse_to_active` | Not in spec | Added | Required to keep `iter_entities` O(active) without putting active_id_index into EntityInland (which would bloat to 24 B) |
| EntityInland layout | u32+u32+ptr, 16 B | Same | Confirmed via offset asserts |
| `columns` packing | Suggested struct OR separate arrays | Packed `Column` struct, 16 B | Single cache line per 4 adjacent columns; co-located ptr+stride |
| `refresh_column` API | Not explicit | Added with `#[cold] refresh_all_columns` for future arena grow | Documents the invariant maintenance contract |
| `Box<[Column; 512]>` vs inline | Considered both | Inline | Eliminates one pointer chase; memory cost 8 MB upfront is acceptable |
| `EntityMaster::entities: Vec<Entity>` | Removed in spec | Removed | `Entity` reconstructed from `(id, entities_inland[id.0].generation)` |
| Step-by-step migration | 9 steps with intermediate compile points | Confirmed; each step is independently compilable | Avoids "big bang" risk |

---

## Files affected (absolute paths)

- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\entity\entity_inland.rs` — full rewrite
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\entity\entity_master.rs` — full rewrite
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\entity\entity.rs` — minor (Entity::generation accessor unchanged; consider narrowing to u32 in a follow-up)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\archetype\archetype.rs` — add `Column`, `columns: [Column; 512]`, `refresh_column`, `refresh_all_columns`; update `create_by_ids` and `register_component`
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\archetype\archetype_bundle.rs` — full rewrite with pinned slab
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\archetype\archetype_master.rs` — add `get_archetype_ptr` wrapper; adapt `create_archetype`/`clear` to slab semantics
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs` — rewrite `get_component_raw`, `get_component_raw_mut`, `set_component_raw`, `has_entity`, `has_component`, `get_entity_archetype_id`, `create_entity`, `delete_entity`; add typed `get_component<T>` / `get_component_mut<T>`
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\component\component_pool_bundle.rs` — minor (no API change; bundle now bypassed on read but still used for mutation)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\iters\query.rs` and `query_state.rs` — adapt to changed `iter_entities` if needed (signature preserved, behavior unchanged)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\benches\random_access.rs` — **new file** (Phase 7 bench harness)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\tests\phase7_random_access.rs` — **new file** (integration)

## Self-check against the plan-readiness checklist

- Cell **Structure**: cleared.
- Cell **Data structures**: every field has type + comment + role; `repr(C)` specified; size + offset asserts included; no false sharing concerns since hot lookups are read-only.
- Cell **API**: minimal surface, no `dyn Trait`, lifetimes implicit-and-trivial.
- Cell **Multithreading**: today single-threaded; Phase 9 prep documented.
- Cell **Correctness**: 10 SAFETY invariants (U1-U10); generation/version explicit; drop order proven.
- Cell **Integration**: 9-step ordered migration; each step compiles.
- Cell **Validation**: ≥30 unit tests, 4 integration tests, proptest, miri, criterion harness specified.

Ready for `architecture-critic` review.