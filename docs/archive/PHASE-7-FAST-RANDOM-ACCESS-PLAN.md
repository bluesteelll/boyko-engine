# Phase 7 — Fast Random Access (design, Round 2)

## Changes from Round 1

| Critic ID | Severity | Change |
|-----------|----------|--------|
| **C1** | Critical | Narrowed `Entity::generation` from `usize` to `u32` (option a). Updated `boyko_utils::identifiers::primitives::Generation` alias and `Slot::generation` accordingly. Documented wrap window: 2^32 deallocations per slot ≈ 13.6 years at 10/sec — astronomical at realistic rates. Removed all `as u32` truncation casts; `Entity.generation()` now natively matches `EntityInland.generation`. |
| **C2** | Critical | Dropped `Pin<Box<...>>` wrapper. Field is now `Box<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>`. Stability invariant U1 rewritten: real reason is "private field + Box heap allocation pointer is stable unless reassigned, and we never reassign". |
| **C3** | Critical | Pinned down slab construction recipe: `Box::<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>::new_uninit().assume_init()`. Explicit `unsafe` block with SAFETY comment. Verified Rust 2024 / 1.93+ compatibility (`Box::new_uninit` stable since 1.82). |
| **C4** | Critical | Added explicit `*mut Archetype` minting recipe via raw pointer arithmetic from slab base — no `&mut MaybeUninit` reborrow. Added invariant **U11 — Pointer minting recipe**. Added Miri test `phase7_miri_archetype_ptr_no_retag_ub`. |
| **C5** | Critical | Added exhaustive `ComponentPoolBundle` method audit table. Rewrote U10 to state pool buffer ptr/stride is write-once at `add_pool`. Refresh called ONLY on `add_pool`. |
| **C6** | Critical | Corrected MAX_ARCHETYPES rationale: cite `crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs::MAX_ARCHETYPES = 1024` — matches existing ArchetypeBitSet capacity used by query system. Defined capacity-overflow behavior: bundle layer returns `Result<u16, BundleFullError>`; `ArchetypeMaster::create_archetype` / `EcsMaster::create_archetype` panic via `expect("invariant: bundle below MAX_ARCHETYPES")` (matches "should never happen" semantics; avoids `ArchetypeId` → `EcsResult<ArchetypeId>` cascade across every test/bench/caller). `EcsError::ArchetypeBundleFull` is NOT added in Phase 7 — deferred for a future `try_create_archetype` API if needed. |
| **C7** | Critical | Added explicit `Drop` body for `ArchetypeBundle` that walks the occupancy bitset and calls `drop_in_place` per occupied slot. Added invariant **U12 — Drop discipline**. Added Miri test `phase7_miri_bundle_drop_runs_archetype_drop_for_occupied_only`. |
| **W1** | Important | Revised `iter_entities` cache profile claim in D3: `active_ids` walk is hot sequential, but per-entity `entities_inland[id.0]` is random access with potential L1/L2 misses for sparse populations after churn. Bench acceptance criteria reflect realistic cost. |
| **W2** | Important | Revised perf table target from "3 cache lines / ~12 ns" to "3-4 lines / 12-16 ns average; 3 lines best case for low `ComponentId.0`". Hot path narrative updated. |
| **W3** | Important | Added explicit pre-condition: design assumes `MAX_COMPONENTS × MAX_ARCHETYPES × size_of::<Column>() ≤ 16 MB`. Beyond that, switch to sparse columns or boxed columns. Documented threshold in `constants.rs`. |
| **W4** | Important | Added doc-comment note on `get_component_raw_mut`: "EntityInland is Copy; we copy 16 B to drop the EntityMaster borrow before reborrowing the slab as &mut Archetype." |
| **W5** | Important | Dropped `_pad: u32` forward-compat-as-discriminant claim. Renamed to `_reserved: u32` with "may be repurposed when Phase 8 design lands; do not rely on layout" doc-comment. Phase 8 sparse-set scheme deferred to tagged-pointer or separate dispatch. |
| **W6** | Important | Specified in-place archetype construction: `add_archetype_from_components` mints raw slot pointer, then per-field `addr_of_mut!(...).write(...)`. No stack-allocated `Archetype` temporary. Added invariant **U13 — In-place archetype construction**. Test `create_1000_archetypes_no_stack_overflow`. |
| **W7** | Important | Spelled out `create_entity` choreography. Added `ArchetypeMaster::archetype_ptr_for(id) -> Option<*mut Archetype>`. Added invariant **U14 — Archetype_ptr cast to &mut for create_entity**. |
| **W8** | Important | Resolved by C6 (same root cause). |
| **M1** | Migration | Added "Test migration recipe" section. Listed affected test files (`archetype.rs:423,465,505,547`, `entity_master.rs:338,354,372,380,393,423,425,427,435`, `tests/drop_fn.rs:658`). Provided `pub(crate) fn dangling_for_test()` constructor for unit tests that need raw inland without full EcsMaster. |
| **M2** | Migration | Rewrote D9 with per-step build state. Used **shim approach**: steps add `register_entity_with_ptr` as new method, leave old `register_entity` in place; step that switches callers; final step removes shim. Each commit compiles + tests pass. |
| **F1** | Forward-compat | Updated Phase 9 readiness section: documented `Archetype: Sync` requirement and its blockers (`Arena: Sync`, `ComponentPool: Sync`). Phase 7 does NOT establish these — only documents. |

---

## Goal & target metrics

**Goal**: collapse `EcsMaster::get_component_raw(entity, comp_id)` from a 9-cache-line, ~40 ns multi-indirection chain into a 3-4 cache-line, ~12-16 ns straight pointer arithmetic — matching Bevy's `Table`/`Column` model — without breaking generation-based ABA, archetype-pointer stability, or future scheduler safety.

**Target metrics** (release, single-core, AMD Zen3/Intel Alder Lake-class, hot ICache):

| Operation | Today | Target | How |
|-----------|-------|--------|-----|
| `get_component_raw` | ~40 ns / 9 lines | **~12-16 ns / 3-4 lines** | Direct entity slot + cached column ptr+stride |
| `get_component_raw_mut` | ~40 ns | **~12-16 ns** | Same path, `*mut` cast under `&mut self` |
| `has_entity` | ~15 ns / 3 lines | **~5 ns / 1 line** | Single slot load + null + gen check |
| `set_component_raw` | ~45 ns | **~15-18 ns** | Same fast lookup + memcpy |
| `iter_entities` | O(active), good locality on dense | **O(active), realistic profile** | `active_ids` hot sequential + per-entity `entities_inland[id.0]` random |
| `create_entity` | ~150 ns | **≤ 160 ns** | One extra slot-ptr write; column table already built |
| Cache misses per random lookup | ~5-7 | **2-3** | Slab + cached columns |
| Allocations per lookup | 0 | **0** | Unchanged |
| `EntityInland` size | 24 B | **16 B** | Pack ptr+u32+u32 |
| `Entity` size | 24 B | **16 B** | Generation narrowed to u32 (C1) |

---

## Decisions & rationale (D1-D10)

### D1. Stable archetype storage — **`Box<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>`** with `[u64; 16]` occupancy

**Decision**: option (b) — a single heap-allocated fixed-size slab of 1024 `MaybeUninit<Archetype>` slots, with a 1024-bit occupancy bitset (`[u64; 16]` = 128 B). Address of any inhabited slot is computed as `slab_base + slot_index * size_of::<Archetype>()` and is **stable for the lifetime of `ArchetypeBundle`**.

**Why pointers are stable** (C2 fix):
- `Box<T>` allocates `T` on the heap. The pointer returned by `Box` does not change unless `Box` is reassigned or dropped.
- The `slots` field is **private** — no external code can reassign it.
- `ArchetypeBundle` never reassigns its own `slots` field after `new()`.
- Therefore: as long as `ArchetypeBundle` is not dropped, `slab_base + offset` is a stable address.
- `Pin` adds no value here: `MaybeUninit<Archetype>` is `Unpin` (no `PhantomPinned`, no manual `!Unpin` impl), so `Pin<Box<[MaybeUninit<Archetype>; N]>>` collapses to `Box<[MaybeUninit<Archetype>; N]>` for all pinning operations. We drop the decorative `Pin`.

**Why slab over alternatives**:
- `*const Archetype` stored in `EntityInland` MUST stay valid across `create_archetype` calls. `Vec<Archetype>::push` reallocates on grow → pointers dangle. Disqualified.
- `Vec<Box<Archetype>>`: each `Box` is stable but adds a pointer-chase on iteration (Phase 9 scheduler iterates archetypes for system dispatch). Slab keeps archetypes contiguous → iteration walks the bitset and accesses contiguous slots.
- Bitset iteration uses `trailing_zeros` (TZCNT on x86_64) — O(popcount(occupied)) for sparse, O(1024/64) = O(16) worst case. Cache-warm.

**Capacity rationale** (C6 fix):
- `MAX_ARCHETYPES = 1024` is defined in `crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs`. It matches `ArchetypeBitSet`'s width used by `QueryState`. The slab width = the bitset width by construction; the query layer's existing dedup bitmap defines the bound.
- Capacity overflow behavior: `add_archetype` returns `Result<u16, BundleFullError>` at the bundle level. Higher layers (`ArchetypeMaster::create_archetype`, `EcsMaster::create_archetype`) **panic** with a clear message via `expect("invariant: archetype bundle below MAX_ARCHETYPES")`. The panic discipline matches Phase 6's `EventBufferFull` (which is `debug_assert!` then `Err` — but `create_archetype` is a per-archetype-type one-shot setup call, not a per-frame hot path, so panic-on-misuse is preferable to changing every caller and test from `ArchetypeId` to `EcsResult<ArchetypeId>` for a "should never happen" condition). The `BundleFullError` variant on the bundle layer stays as a typed-failure handle for any future `try_create_archetype` API that wants to recover instead of panic. **The `EcsError::ArchetypeBundleFull` variant is NOT added in Phase 7** — defer until a Result-returning surface is needed.

**Pre-condition** (W3 fix):
This inline-Column design assumes `MAX_COMPONENTS × MAX_ARCHETYPES × size_of::<Column>() ≤ 16 MB`. Current: 512 × 1024 × 16 B = 8 MB. Beyond 16 MB total slab, the design MUST switch to sparse columns or boxed columns. Threshold documented in `crates/boyko_ecs/src/ecs/core/constants.rs`.

**Slab construction recipe** (C3 fix):

```rust
// File: archetype_bundle.rs
impl ArchetypeBundle {
    #[cold]
    pub fn new() -> Self {
        // SAFETY (slab init):
        //   `Box::<T>::new_uninit()` allocates space for `T` on the heap and returns
        //   `Box<MaybeUninit<T>>`. For arrays, the resulting allocation is uninitialized
        //   memory of the correct size and alignment, sized via heap allocator — no
        //   stack-frame construction of the 8.6 MB temporary.
        //
        //   `assume_init()` is sound because:
        //     1. T = [MaybeUninit<Archetype>; MAX_ARCHETYPES] is itself a container
        //        of MaybeUninit; the array's "initialized" state is satisfied by any
        //        bit pattern (every element is MaybeUninit, requires no validity).
        //     2. We track per-slot initialization separately via the `occupied` bitset.
        //   This pattern is stable since Rust 1.82 (`Box::new_uninit`); boyko-engine
        //   targets Rust 2024 / 1.93+.
        let slots = unsafe {
            Box::<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>::new_uninit()
                .assume_init()
        };

        Self {
            slots,
            occupied: [0u64; 16],
            id_to_slot: Vec::new(),
            free_slots: Vec::with_capacity(16),
            count: 0,
        }
    }
}
```

**Trade-off**:
- ~8.6 MB upfront vs ~0 today. Trivial against the 64 MB arena.
- Slot recycling on `remove_archetype` doesn't shrink the slab — same as `Vec` behavior.
- Manual `Drop` for occupied slots (drop loop walks the occupancy bitset — see D8 / C7).

### D2. `Entity` and `EntityInland` layout — `generation: u32` everywhere (C1 fix)

**Decision**: narrow `Entity::generation` from `usize` to `u32`. Update `boyko_utils::identifiers::primitives::Generation` from `pub type Generation = usize;` to `pub type Generation = u32;`. `Slot::generation` follows the alias.

**Why unified `u32`** (C1 fix):
- The audit (M-016) previously chose `usize` to avoid ABA collisions. The narrowing must be analyzed:
  - Wrap window: 2^32 = 4,294,967,296 deallocations of a **single slot** before generation collision.
  - Sustained dealloc rate of 10/sec/slot is already pathological — at that rate, wrap takes 4.3 × 10^9 / 10 / 86400 / 365 ≈ **13.6 years per slot**.
  - Realistic ECS workloads: ≤ 1 dealloc/sec/slot for entities that recycle frequently → wrap at 136 years.
  - For comparison, MTBF for cosmic-ray-induced bit flips on consumer RAM is ~1000 hours; ABA-via-32-bit-wrap is several orders of magnitude rarer than hardware soft errors.
- The wrap window is **per-slot**, not global. A million slots cycling independently does not reduce per-slot wrap; ABA detection is per-slot by design.
- This eliminates ALL `as u32` truncation: `Entity.generation()` returns `u32`, `EntityInland.generation: u32` compares natively without casting.

**`Entity` layout (after)**:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entity {
    id: EntityId,           // EntityId(usize) — unchanged
    generation: u32,        // narrowed from usize
}
// size: 16 B (8 + 4 + 4 padding), align 8
```

**`EntityInland` layout (after)**:
```rust
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EntityInland {
    /// Raw provenance pointer into ArchetypeBundle's slab.
    /// NULL ⇔ dead slot (never registered or deallocated).
    /// Stored as *mut (not *const) so `&mut EcsMaster` can transitively
    /// cast to `&mut Archetype` without provenance laundering (D7).
    archetype_ptr: *mut Archetype,  // 8 B, offset 0
    unit_index: u32,                // 4 B, offset 8  — index into Archetype.entity_ids
    generation: u32,                // 4 B, offset 12 — matches Entity.generation natively
}
const _: () = assert!(std::mem::size_of::<EntityInland>() == 16);
const _: () = assert!(std::mem::align_of::<EntityInland>() == 8);
const _: () = assert!(std::mem::offset_of!(EntityInland, archetype_ptr) == 0);
const _: () = assert!(std::mem::offset_of!(EntityInland, unit_index) == 8);
const _: () = assert!(std::mem::offset_of!(EntityInland, generation) == 12);
```

**Alternatives rejected** (option b/c from critic):
- **(b) Keep `Entity::generation: usize`, store `usize` in `EntityInland`**: 24 B inland; only ~2.6 inlands per cache line vs 4. Loses 1.5× per-line density. ✗
- **(c) Truncate with `debug_assert!(entity.generation() <= u32::MAX as usize)`**: still requires casting in hot path; debug-only protection is asymmetric with release behavior. ✗

**Trade-offs accepted**:
- 13.6-year-per-slot ABA wrap window is documented as accepted bound.
- `Entity` shrinks from 24 B to 16 B (saved 8 B per user-held handle).
- Migration impact: every test/example that constructs `Entity::new(id, gen: usize)` must use `Entity::new(id, gen: u32)`. Most call sites already pass `0` or small literals; the change is type-inferred where possible.

### D3. `EntityMaster` — direct `Vec<EntityInland>` indexed by `EntityId.0`, parallel `active_ids: Vec<EntityId>`

```rust
pub struct EntityMaster {
    /// Sparse-indexed by EntityId.0. Dead slots have archetype_ptr == null.
    /// Length grows monotonically with the maximum-ever EntityId.
    entities_inland: Vec<EntityInland>,

    /// Reverse map: for active entity at EntityId.0,
    /// gives the index into active_ids. u32::MAX = "not active".
    sparse_to_active: Vec<u32>,

    /// Dense list of currently-active EntityIds. Drives O(active) iter_entities.
    active_ids: Vec<EntityId>,

    /// Free list of recycled EntityIds.
    free_entity_ids: Vec<EntityId>,

    /// Next fresh EntityId.
    next_entity_id: EntityId,
}
```

**iter_entities cache profile** (W1 fix):
- `active_ids` walk is **hot sequential**: 8 B per active entity, 8 entries per cache line. Linear scan, HW prefetcher friendly.
- `entities_inland[id.0]` lookup per active entity is **random access**: for sparse populations after churn (e.g., 1M entity IDs allocated, 10K active), each lookup may hit a cold L2/L3 line. Worst case: 10K × 25 ns LLC miss = 250 µs.
- For dense populations (no recycling), `id.0` values cluster and lookups hit warm L1d lines.
- Bench acceptance criteria (D10) reflect this: `bench_iter_entities_sparse_post_churn` measures the realistic cost, not the best case.

Cost analysis per operation:
- Hot read path (`get_component_raw`): only touches `entities_inland[id.0]` — 1 cache line. ✓
- `register_entity`: writes `entities_inland[id.0]`, pushes to `active_ids`, writes `sparse_to_active[id.0] = active_ids.len() - 1`. 3 cache lines touched. Once per entity creation, dominated by `Archetype::create_entity` anyway.
- `deallocate_entity`: same 3 lines + swap_remove from `active_ids` + update `sparse_to_active` of the swapped entity. Still O(1).
- `iter_entities`: walks `active_ids` (hot sequential) + per-entity `entities_inland` (random access). Acceptable; documented.

**Decision**: keep `active_ids` + `sparse_to_active` parallel index. Pay 4 B/slot to keep `iter_entities` O(active) and `register/deallocate` O(1). This is non-negotiable: dropping `iter_entities` to O(capacity) would degrade `query_entities` perf on sparse populations far more than the random-access cost.

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
    /// Reserved for future use. Layout-stable; may be repurposed when Phase 8
    /// design lands. **Do not rely on this field for any current dispatch.**
    _reserved: u32,         // 4 B, offset 12
}
const _: () = assert!(std::mem::size_of::<Column>() == 16);
const _: () = assert!(std::mem::align_of::<Column>() == 8);
```

**Why packed (W5 fix on `_reserved`)**:
- Single cache line holds 4 `Column` entries → for an archetype with 4 components in adjacent IDs, all hot data fits in 1 line.
- The `_reserved: u32` slot brings `Column` to 16 B (power-of-two stride, fast `c × 16` indexing). Renamed from `_pad` to `_reserved`. Document explicitly: "Phase 8 may repurpose; do not rely on layout."
- Phase 8 sparse-set dispatch will use a separate scheme: tagged pointer (bit 0 of `ptr` since arena alignment ≥ 8), OR an out-of-band `column_kinds: [u8; MAX_COMPONENTS]` array, OR a parallel `SparseColumn` table. The choice is deferred to Phase 8; Round 2 of Phase 7 makes no commitment.

**`Archetype` new field layout**:
```rust
pub struct Archetype {
    /// 8 KB inline hot lookup table. Indexed by ComponentId.0.
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

**Estimated `Archetype` size**: 8192 + ~200 = ~**8.4 KB**. Slab of 1024 = ~8.6 MB. Acceptable.

**Refresh contract** (D5/C5): `columns[c]` is the single source of truth for the read path.

```rust
impl Archetype {
    /// Re-syncs columns[component_id.0] with the current pool state. Called
    /// after `component_pools.add_pool(...)`. NOT called on data-only mutations
    /// (push, swap_remove, etc — see C5 audit in D5).
    #[inline]
    fn refresh_column(&mut self, component_id: ComponentId) {
        match self.component_pools.get_pool(component_id) {
            Some(pool) => {
                self.columns[component_id.0] = Column {
                    ptr: pool.buffer_ptr() as *mut u8,
                    stride: pool.component_layout().size() as u32,
                    _reserved: 0,
                };
            }
            None => {
                self.columns[component_id.0] = Column::null();
            }
        }
    }

    /// Refreshes the entire columns table from component_pools. Called only
    /// on future arena-grow events. Not on hot path.
    #[cold]
    fn refresh_all_columns(&mut self) {
        for col in self.columns.iter_mut() {
            *col = Column::null();
        }
        for &cid in self.component_ids.iter() {
            self.refresh_column(cid);
        }
    }
}
```

### D5. `ComponentPoolBundle` — kept for mutation, bypassed on read

The bundle still owns `pools: Vec<ComponentPool>` (memory ownership and per-pool mutation). The **read path** bypasses `sparse_indexes` entirely — `Archetype::columns[component_id.0]` is the answer.

**Method audit — when does each `ComponentPoolBundle` method mutate pool ptr/stride?** (C5 fix)

| Method | Mutates `buffer_ptr` / `component_layout`? | `refresh_column` required? |
|---|---|---|
| `add_pool` | **Yes** — creates new pool, sets `buffer_ptr` from arena alloc | **Yes**, for the new pool's component_id |
| `push_entity_components` | No — writes data into existing pool buffer | No |
| `can_push_entity_components` | No — read-only check | No |
| `swap_remove_unit` | No — moves data within pool, base ptr unchanged | No |
| `pop_entity` | No — same as swap_remove | No |
| `add_component` | No — same as push | No |

**No `remove_pool` exists today.** If a future phase adds one, that phase MUST call `refresh_column(removed_id)` to NULL the column entry.

**Invariant U10 codification** (C5 fix):
```
The pool's `buffer_ptr` and `component_layout` are write-once at `add_pool`
and never change thereafter (until future arena-grow refresh).
Therefore `refresh_column` is only called on:
  (a) `add_pool` for the new pool's component_id, AND
  (b) future arena-grow API (not in Phase 7 MVP).
```

This invariant is enforced by:
- `debug_assert!` at the end of `add_pool` verifying the bundle's pool buffer matches the new column.
- `debug_assert!` at the end of every `Archetype` mutation method verifying `columns[c].ptr == get_pool(c).map_or(null, |p| p.buffer_ptr())` for every component_id in `self.component_ids`.

### D6. Read-only path API — new fast `get_component_raw`

```rust
impl EcsMaster {
    /// Fast random access: 3-4 cache lines, ~12-16 ns average.
    /// 1. entities_inland[id.0]   — 1 line
    /// 2. archetype.columns[c.0]  — 1 line (same line as deref for c.0 < 4)
    /// 3. ptr.add(unit * stride)  — arithmetic; final line is the component itself
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
        if inland.generation != entity.generation() {  // u32 == u32, no cast
            return None;
        }

        // Line 2: archetype.columns[c.0]
        // SAFETY: U1, U2, U3 (see Invariants section).
        let archetype = unsafe { &*inland.archetype_ptr };

        debug_assert!(component_id.0 < MAX_COMPONENTS);
        // SAFETY: U4. `columns` length is MAX_COMPONENTS; bounded by debug_assert.
        let column = unsafe { archetype.columns.get_unchecked(component_id.0) };
        if column.ptr.is_null() {
            return None;
        }

        // Line 3 (or 4): component itself.
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

### D7. Mutable path — `archetype_ptr: *mut Archetype`

`EntityInland::archetype_ptr` is **`*mut Archetype`** (not `*const`).

```rust
impl EcsMaster {
    /// EntityInland is Copy; we copy 16 B to drop the EntityMaster borrow
    /// before reborrowing the slab as &mut Archetype. (W4)
    #[inline]
    pub fn get_component_raw_mut(&mut self, entity: Entity, component_id: ComponentId) -> Option<*mut u8> {
        let inland = *self.entity_master.entities_inland.get(entity.id().0)?;
        if inland.archetype_ptr.is_null() { return None; }
        if inland.generation != entity.generation() { return None; }
        debug_assert!(component_id.0 < MAX_COMPONENTS);
        // SAFETY: U1-U4, plus &mut self gives exclusive access to the slab.
        // SAFETY (U14): archetype_ptr was minted via raw-pointer recipe (U11);
        //   no &mut MaybeUninit reborrow exists; the cast is sound under
        //   single-threaded &mut EcsMaster.
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

### D8. Drop semantics — explicit Drop body for ArchetypeBundle (C7 fix)

**EcsMaster field order** stays unchanged: `events`, `entity_master`, `archetype_master`, `arena`. Validation:
- `EntityMaster::Drop`: drops `Vec<EntityInland>`. Each `EntityInland` is `Copy` (raw pointer + u32 + u32); no destructors. The pointers dangle on drop completion, but since they are **never dereferenced** during `EntityMaster::Drop`, no UB. ✓
- `ArchetypeMaster::Drop` → `ArchetypeBundle::Drop`: must walk the occupancy bitset and `ptr::drop_in_place` each occupied `Archetype` slot.
- `Arena::Drop`: last, frees the backing buffer. ✓

**Explicit `Drop for ArchetypeBundle`** (C7 fix):

```rust
impl Drop for ArchetypeBundle {
    /// SAFETY (U12 — Drop discipline):
    ///   - `occupied` bitset tracks which slots are initialized.
    ///   - For each set bit, the slot at index `bit` has been initialized
    ///     via `MaybeUninit::write` (or in-place construction U13) and
    ///     has not been dropped before.
    ///   - We hold `&mut self` (exclusive); no other borrow exists.
    ///   - `drop_in_place` runs each `Archetype::Drop` exactly once.
    ///   - Empty slots (`occupied[w] & bit == 0`) remain `MaybeUninit`
    ///     and are never touched.
    ///   - After this loop, `Box`'s auto-Drop frees the (uninit-as-far-as-Box-knows)
    ///     slab memory.
    fn drop(&mut self) {
        const WORDS: usize = MAX_ARCHETYPES / 64; // 16
        let slab_base: *mut MaybeUninit<Archetype> = self.slots.as_mut_ptr();

        for word_idx in 0..WORDS {
            let mut word = self.occupied[word_idx];
            while word != 0 {
                let bit = word.trailing_zeros() as usize;
                let slot_idx = word_idx * 64 + bit;
                // SAFETY (U12): see method docstring.
                unsafe {
                    let slot_mu: *mut MaybeUninit<Archetype> = slab_base.add(slot_idx);
                    let slot: *mut Archetype = slot_mu as *mut Archetype;
                    slot.drop_in_place();
                }
                word &= word - 1; // BLSR: clear lowest set bit
            }
        }
        // Box's auto-Drop now frees the slab memory.
    }
}
```

**Invariant U12 — Drop discipline**: `ArchetypeBundle::Drop` calls `drop_in_place` exactly once per occupied slot, computed from the occupancy bitset. Empty slots remain `MaybeUninit` and are never touched. `Box` auto-Drop then frees the slab memory.

Invariant added: `EntityMaster::Drop` MUST NOT dereference any `archetype_ptr`. Documented in the field doc-comment and enforced by inspection.

### D9. Migration impact — 10 step ordered plan with per-step build state (M2 fix)

Each step compiles independently AND tests pass. Shim approach: add new APIs first, switch callers, remove shims last.

| Step | Files | What changes | Build state |
|------|-------|--------------|-------------|
| **0** | `boyko_utils/src/identifiers/primitives.rs`, `boyko_utils/src/identifiers/slot.rs`, `boyko_utils/src/sparse_map/sparse_slot_map.rs` | Change `pub type Generation = usize;` → `pub type Generation = u32;`. `Slot::generation` follows alias. **Cascade fix** (C1 follow-up from R2 critique): `SparseSlotMap::push_dense(&mut self, external_idx: usize, value: U, generation: usize)` parameter type changes to `generation: Generation` — current `usize` literal `0` callers via `Slot::new(idx, 0)` continue to compile via inference, but any `let gen: usize` annotations explicitly need retyping. | All call sites that construct Slot with usize literals continue to compile via inference. Tests pass. |
| **1** | `crates/boyko_ecs/src/ecs/core/entity/entity.rs` | Change `Entity::generation: usize` → `u32`. Update `Entity::new`, `generation()`, `with_id`, `increment_generation`, `is_same`, `From<Slot>` / `From<Entity> for Slot`. (C1) | Code that previously took `usize` returns from `entity.generation()` now gets `u32`. Test files that pass `1usize` as generation must be updated to `1u32` or `1` (inferred). |
| **2** | `crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs` | Add new struct `EntityInlandFast { archetype_ptr: *mut Archetype, unit_index: u32, generation: u32 }`. Add layout asserts. Add `dangling_for_test()` constructor (M1). **Do not remove old `EntityInland`.** | Both structs co-exist; compiles. |
| **3** | `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` (+ N2 cascade: `query.rs`, `benches/archetype.rs`, archetype tests) | Add `pub struct Column`. Add `columns: [Column; MAX_COMPONENTS]` field to `Archetype` (inline, offset 0). Add `Archetype::refresh_column` / `refresh_all_columns`. In `create_by_ids` and `register_component`, call `refresh_column(c)` after `add_pool`. **Read path still goes through `component_pools.get_pool`.** **Method-signature cascade** (N2 follow-up from R2 critique): the existing `Archetype::init_entity_inland(&self, inland: &mut EntityInland)` (which calls `inland.set_archetype_id(self.id)`) is obsoleted by the new EntityInland having no `archetype_id` field — DELETE the method. The existing `Archetype::create_entity(&mut self, entity_id, &mut EntityInland, ...)` signature changes to `Archetype::create_entity(&mut self, entity_id, &mut new_unit_index: u32, ...)` — caller no longer fills an inland, just receives the unit index. Update 7 call sites: `query.rs:741, 760, 886`, `benches/archetype.rs:71, 109`, `archetype.rs` tests at `:424, 466, 728, 764`. | Archetype now ~8.4 KB; sub-MVP read path uses old path; tests + benches updated; all green. |
| **4** | `crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs` | Rewrite with slab. Add new methods: `get_archetype_ptr(id) -> Option<*mut Archetype>`, `add_archetype_from_components`, `iter_occupied_ptrs`. Old API (`get_archetype`, `get_archetype_mut`, `add_archetype`, `remove_archetype`, `iter`, `len`, `is_empty`, `clear`) preserved with new internal impl. New error type `BundleFullError`. (C2, C3, C6) | All existing callers work; slab live; tests pass. |
| **5** | `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs` | Add `get_archetype_ptr(id) -> Option<*mut Archetype>` wrapper. Add `archetype_ptr_for(id)` alias for create_entity choreography (W7). Add `add_archetype_and_get_ptr` for the create-archetype path. | Both old and new APIs work; tests pass. |
| **6** | `crates/boyko_ecs/src/ecs/core/entity/entity_master.rs` | Add **new method** `register_entity_with_ptr(entity, archetype_ptr, unit_index: u32)` as shim alongside existing `register_entity(entity, archetype_id, InlandPoolId)`. Add new fields `entities_inland_fast: Vec<EntityInlandFast>`, `sparse_to_active: Vec<u32>`, `active_ids: Vec<EntityId>`. Old `entity_map` and `entities` co-exist with new fields. Writes update BOTH stores. Reads go through old path. (M2 shim) | Both representations sync'd; tests pass on old path. |
| **7** | `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | Add new fast read methods `get_component_raw_fast`, `get_component_raw_mut_fast`, `has_entity_fast` reading the new fields. Old methods still in place. New typed wrappers `get_component<T>`, `get_component_mut<T>`. Switch `create_entity` to use `add_archetype_and_get_ptr` + `register_entity_with_ptr` (W7 choreography). | Old slow path still works for read; new fast path live alongside; tests for both pass. |
| **8** | `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | Switch existing `get_component_raw`, `get_component_raw_mut`, `set_component_raw`, `has_entity`, `has_component`, `get_entity_archetype_id` to the fast path internals (delete `*_fast` aliases). | Read path now exclusively fast; tests pass. |
| **9** | `entity_master.rs`, `entity_inland.rs`, `archetype_master.rs` | Remove shims: old `entity_map`, `entities`, old `register_entity`, old `EntityInland`. Rename `EntityInlandFast` → `EntityInland`. Remove old archetype-id-based register path. Remove `archetype_to_index: SparseMap` from `ArchetypeBundle` (replaced by `id_to_slot`). | Code minimal; tests pass with new API only. |
| **10** | Tests + bench harness | Update tests that referenced removed APIs (e.g. `EntityInland::archetype_id()`). Apply test migration recipe (M1). Add Phase 7 micro-benchmarks (D10). | All tests + benches pass. |

After step 10, the old `entity_map: SparseMap` is gone; the `archetype_to_index: SparseMap` in the bundle is gone; the read path uses zero sparse-map lookups.

**create_entity choreography (W7 fix)** — embedded into step 7:

```rust
fn create_entity(&mut self, archetype_id: ArchetypeId, ...) -> EcsResult<Entity> {
    if !self.archetype_master.has_archetype(archetype_id) {
        return Err(EcsError::ArchetypeNotFound(archetype_id));
    }
    // Pointer obtained before further borrows; bundle's slab is private,
    // so this short-lived call doesn't conflict with the &mut chain.
    let archetype_ptr = self.archetype_master.archetype_ptr_for(archetype_id)
        .expect("invariant: just verified existence");
    let entity = self.entity_master.allocate_entity();
    // SAFETY (U14): archetype_ptr is valid (just verified existence in the bundle);
    //   single-threaded &mut EcsMaster gives exclusive access to the slab;
    //   no other live borrow into the slab exists.
    let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
    let mut new_unit_index: u32 = 0;
    if !archetype.create_entity(entity.id(), &mut new_unit_index, components) {
        self.entity_master.rewind_allocate(entity);
        return Err(EcsError::ArchetypeCapacityExceeded(archetype_id));
    }
    // archetype_ptr is the same pointer — no fresh borrow needed.
    self.entity_master.register_entity_with_ptr(entity, archetype_ptr, new_unit_index);
    Ok(entity)
}
```

**Add_archetype in-place construction (W6 fix)** — embedded into step 4:

```rust
impl ArchetypeBundle {
    pub fn add_archetype_from_components(
        &mut self,
        archetype_id: ArchetypeId,
        component_ids: &[ComponentId],
        arena: &Arena,
    ) -> Result<u16, BundleFullError> {
        let slot_idx = match self.free_slots.pop() {
            Some(idx) => idx,
            None => {
                if self.count >= MAX_ARCHETYPES {
                    return Err(BundleFullError);
                }
                self.count as u16
            }
        };

        // SAFETY (U11 — pointer minting recipe):
        //   Mint via raw pointer arithmetic from the slab base. No &mut MaybeUninit
        //   borrow is created; the resulting *mut Archetype carries the Box's heap
        //   allocation provenance directly.
        let slab_base: *mut MaybeUninit<Archetype> = self.slots.as_mut_ptr();
        // SAFETY: slot_idx < MAX_ARCHETYPES (checked above).
        let slot_ptr_mu: *mut MaybeUninit<Archetype> = unsafe {
            slab_base.add(slot_idx as usize)
        };
        let slot_ptr: *mut Archetype = slot_ptr_mu as *mut Archetype;

        // SAFETY (U13 — in-place archetype construction):
        //   slot_ptr is uninitialized memory of size_of::<Archetype>(), aligned
        //   per the array element layout. We initialize each field exactly once
        //   via addr_of_mut!.write() before setting the occupancy bit. No
        //   stack-allocated 8.4 KB Archetype temporary is constructed.
        unsafe {
            use core::ptr::addr_of_mut;
            // Initialize the columns array in place via `write_bytes` — `Column::null()`
            // is all-zero bits (ptr = null, stride = 0, _reserved = 0; verified by
            // const_assert at the Column definition), so a memset to 0 is the
            // semantically-correct value for every slot. This avoids constructing an
            // 8 KB `[Column; 512]` source-level temporary, regardless of whether the
            // compiler RVO-optimizes the array literal form (W6 closure note).
            //
            // SAFETY (U13.a): destination is uninit-but-aligned memory of exact size
            //   `size_of::<[Column; MAX_COMPONENTS]>()`. Writing zero bytes is sound
            //   for any `T: Copy + AllZerosIsValid`, and `Column { ptr: null_mut(),
            //   stride: 0, _reserved: 0 }` is exactly the zero-bit pattern.
            core::ptr::write_bytes(
                addr_of_mut!((*slot_ptr).columns).cast::<Column>(),
                0u8,
                MAX_COMPONENTS,
            );
            addr_of_mut!((*slot_ptr).id).write(archetype_id);
            // ComponentPoolBundle::new() does not allocate the pools array large;
            // it's a small struct with internal Vec<ComponentPool> empty initially.
            addr_of_mut!((*slot_ptr).component_pools).write(ComponentPoolBundle::new());
            addr_of_mut!((*slot_ptr).current_index).write(0);
            addr_of_mut!((*slot_ptr).signature).write(ArchetypeSignature::from_ids(component_ids));
            addr_of_mut!((*slot_ptr).arena).write(arena as *const Arena);
            addr_of_mut!((*slot_ptr).component_ids).write(component_ids.to_vec());
            addr_of_mut!((*slot_ptr).entity_ids).write(Vec::new());
        }

        // Now register component pools and refresh columns. We hold a unique
        // pointer; convert to &mut for the field setup ergonomically.
        // SAFETY (U13 continuation): all fields are now initialized; &mut is sound.
        let archetype: &mut Archetype = unsafe { &mut *slot_ptr };
        for &cid in component_ids {
            archetype.register_component_inplace(cid, arena);
        }

        // Set occupancy bit AFTER full initialization.
        self.occupied[slot_idx as usize / 64] |= 1u64 << (slot_idx as usize % 64);
        // Update id_to_slot.
        if (archetype_id.0 as usize) >= self.id_to_slot.len() {
            self.id_to_slot.resize(archetype_id.0 as usize + 1, u16::MAX);
        }
        self.id_to_slot[archetype_id.0 as usize] = slot_idx;
        if self.free_slots.is_empty() && (slot_idx as usize) == self.count {
            self.count += 1;
        } else {
            self.count += 1;
        }
        Ok(slot_idx)
    }
}
```

**Pointer minting recipe (C4 fix)** — used in step 5 (`archetype_ptr_for`):

```rust
impl ArchetypeBundle {
    /// Returns *mut Archetype for the given archetype_id, or None if absent.
    /// SAFETY (U11 — pointer minting recipe): pointer minted via raw arithmetic
    ///   from the slab base. NO `&mut MaybeUninit<Archetype>` reborrow is created
    ///   along the way; the *mut carries the Box's heap allocation provenance
    ///   directly. Subsequent `&self` or `&mut self` reads through this pointer
    ///   will not retag against any stale borrow stack — there is no stale stack
    ///   because no & or &mut reference to the slab element ever existed.
    #[inline]
    pub fn get_archetype_ptr(&self, archetype_id: ArchetypeId) -> Option<*mut Archetype> {
        let slot_idx = *self.id_to_slot.get(archetype_id.0 as usize)?;
        if slot_idx == u16::MAX { return None; }
        // SAFETY (U11): see above. `self.slots.as_ptr()` returns *const without
        //   creating a &reference. We cast through *mut explicitly. The slab
        //   address is stable (D1 + private field).
        let slab_base: *const MaybeUninit<Archetype> = self.slots.as_ptr();
        let slot_ptr_mu: *const MaybeUninit<Archetype> = unsafe {
            slab_base.add(slot_idx as usize)
        };
        Some(slot_ptr_mu as *mut Archetype)
    }
}
```

**Test migration recipe (M1 fix)** — used in step 10:

Affected test files identified by code search:
- `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` lines 423, 465, 505, 547 — direct `EntityInland::new(arch.id(), InlandPoolId(0), 0)` construction
- `crates/boyko_ecs/src/ecs/core/entity/entity_master.rs` lines 338, 354, 372, 380, 393, 423, 425, 427, 435 — same pattern
- `crates/boyko_ecs/tests/drop_fn.rs` line 658 — same pattern

**Rewrite patterns**:

1. **Tests that need full ECS flow (preferred)**: restructure to use `EcsMaster::new()` → `create_archetype` → `create_entity`. Idiomatic, exercises the real path.

2. **Tests that need a raw `EntityInland` without an EcsMaster**: use the new `EntityInland::dangling_for_test()` helper:

```rust
impl EntityInland {
    /// Returns an EntityInland with a dangling but non-null archetype_ptr.
    /// The pointer must NEVER be dereferenced. Used only for tests that
    /// exercise inland-arithmetic without involving an actual Archetype.
    ///
    /// # Safety
    ///
    /// Returned inland is NOT registered in any EntityMaster; dereferencing
    /// its archetype_ptr is UB. Use only for layout / equality / accessor tests.
    #[cfg(test)]
    pub(crate) fn dangling_for_test(unit_index: u32, generation: u32) -> Self {
        // NonNull::dangling() has a safe API; cast to *mut.
        // SAFETY: pointer is never dereferenced in test code that uses this.
        Self {
            archetype_ptr: core::ptr::NonNull::<Archetype>::dangling().as_ptr(),
            unit_index,
            generation,
        }
    }
}
```

Test rewrites for `archetype.rs:423` etc. use option (1). Tests in `entity_master.rs:338-435` exercise pure EntityMaster mechanics → use option (2) with `dangling_for_test`.

### D10. Benchmarks

Add to existing criterion harness at `crates/boyko_ecs/benches/random_access.rs`:

| Bench | What | Acceptance |
|-------|------|------------|
| `bench_get_component_raw_hot` | 10K entities in 1 archetype, random shuffled access of 1K lookups, hot cache | ≤ 16 ns/op (target 12-14) |
| `bench_get_component_raw_cold` | Same as above but flush cache before each lookup | ≤ 90 ns/op (3-4 cache misses × ~25 ns) |
| `bench_get_component_typed` | Typed wrapper, debug + release builds | release ≤ 16 ns/op |
| `bench_has_entity` | Just generation check, no component | ≤ 5 ns/op |
| `bench_set_component_raw` | 8 B component write | ≤ 18 ns/op |
| `bench_iter_entities_dense_10k` | 10K active, no churn | parity with current |
| `bench_iter_entities_sparse_post_churn` | Allocate 1M IDs, keep 10K active randomly; iterate (W1) | documented (likely 2-5× slower than dense) |
| `bench_create_entity_10k` | Sequential creates | ≤ 5% regression vs current |
| `bench_get_component_stale_generation` | Lookup with wrong generation (must return None fast) | ≤ 8 ns/op |
| `bench_get_component_missing_component` | Lookup for ComponentId not in archetype (column.ptr == null) | ≤ 10 ns/op |
| `bench_create_1000_archetypes_no_stack_overflow` | Sequential create_archetype; verify W6 in-place construction (no stack overflow on Windows 1 MB stack) | completes successfully |

Bench should be run with `RUSTFLAGS="-C target-cpu=native"` and `cargo bench --bench random_access`.

---

## Data structures

### EntityInland (new)
```rust
// File: crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs

use crate::ecs::core::archetype::archetype::Archetype;

/// Compact, per-entity routing record. Stored in `EntityMaster::entities_inland`,
/// indexed by `EntityId.0`. Dead entities are represented by `archetype_ptr == null`.
///
/// # Layout (load-bearing)
///
/// ```text
/// offset 0 .. 8 : archetype_ptr  (*mut Archetype, NULL = dead)
/// offset 8 .. 12: unit_index     (u32, index into Archetype.entity_ids / column rows)
/// offset 12..16: generation      (u32, must match Entity.generation natively — no cast)
/// total          : 16 B, aligned 8
/// ```
///
/// 4 inlands fit in one 64-byte cache line — good locality for swap-remove path.
///
/// # Invariants
///
/// - `archetype_ptr.is_null()` ⇒ entity is dead; `unit_index` / `generation` are stale
///   (but reading them is safe — they are POD).
/// - `archetype_ptr != null` ⇒ points into `ArchetypeBundle`'s heap slab; valid for
///   the lifetime of `EcsMaster` (D1 stability invariant).
/// - `generation == Entity.generation()` ⇔ this inland matches the user-held
///   `Entity` handle (ABA-prevention via 32-bit counter; wrap window ≥ 13.6 years per slot).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EntityInland {
    pub(crate) archetype_ptr: *mut Archetype,
    pub(crate) unit_index: u32,
    pub(crate) generation: u32,
}

const _: () = assert!(std::mem::size_of::<EntityInland>() == 16);
const _: () = assert!(std::mem::align_of::<EntityInland>() == 8);
const _: () = assert!(std::mem::offset_of!(EntityInland, archetype_ptr) == 0);
const _: () = assert!(std::mem::offset_of!(EntityInland, unit_index) == 8);
const _: () = assert!(std::mem::offset_of!(EntityInland, generation) == 12);

impl EntityInland {
    #[inline]
    pub const fn dead() -> Self {
        Self {
            archetype_ptr: core::ptr::null_mut(),
            unit_index: 0,
            generation: 0,
        }
    }

    #[inline]
    pub fn new(archetype_ptr: *mut Archetype, unit_index: u32, generation: u32) -> Self {
        Self { archetype_ptr, unit_index, generation }
    }

    #[inline]
    pub fn is_alive(&self) -> bool { !self.archetype_ptr.is_null() }

    #[inline]
    pub fn archetype_ptr(&self) -> *mut Archetype { self.archetype_ptr }

    #[inline]
    pub fn unit_index(&self) -> u32 { self.unit_index }

    #[inline]
    pub fn generation(&self) -> u32 { self.generation }

    #[inline]
    pub fn set_unit_index(&mut self, unit_index: u32) { self.unit_index = unit_index; }

    /// Test-only constructor returning an EntityInland with a dangling but
    /// non-null archetype_ptr. The pointer must NEVER be dereferenced.
    #[cfg(test)]
    pub(crate) fn dangling_for_test(unit_index: u32, generation: u32) -> Self {
        Self {
            archetype_ptr: core::ptr::NonNull::<Archetype>::dangling().as_ptr(),
            unit_index,
            generation,
        }
    }
}
```

### Column
```rust
// File: crates/boyko_ecs/src/ecs/core/archetype/archetype.rs

/// Hot-path lookup entry. One per (Archetype × ComponentId) pair.
/// `ptr.is_null()` ⇔ this archetype has no pool for the component ID.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Column {
    pub(crate) ptr: *mut u8,
    pub(crate) stride: u32,
    /// Reserved field — keeps `Column` at 16 B for power-of-two stride.
    /// May be repurposed when Phase 8 design lands. **Do not rely on layout.**
    pub(crate) _reserved: u32,
}

const _: () = assert!(std::mem::size_of::<Column>() == 16);
const _: () = assert!(std::mem::align_of::<Column>() == 8);

impl Column {
    #[inline]
    pub const fn null() -> Self {
        Self { ptr: core::ptr::null_mut(), stride: 0, _reserved: 0 }
    }
}
```

### Archetype (new column table)
```rust
pub struct Archetype {
    /// 8 KB inline hot lookup table. Indexed by ComponentId.0.
    /// PHASE 7 invariant U10: columns[c].ptr.is_null() ⇔ !component_pools.contains(c).
    /// When non-null: columns[c].ptr == component_pools.get_pool(c).unwrap().buffer_ptr()
    /// and columns[c].stride == component_pools.get_pool(c).unwrap().component_layout().size().
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

### ArchetypeBundle (new slab)
```rust
// File: crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs

use std::mem::MaybeUninit;

const MAX_ARCHETYPES: usize = 1024;
// Source: crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs::MAX_ARCHETYPES.
// Matches the ArchetypeBitSet width used by QueryState — the bitset width = the
// slab width by construction. Do not change in isolation.

/// Returned by `add_archetype` / `add_archetype_from_components` when the slab
/// is at capacity. Higher layers (`ArchetypeMaster::create_archetype`,
/// `EcsMaster::create_archetype`) panic via `expect(...)` on this error
/// (their public signatures stay `-> ArchetypeId`, no `EcsError::ArchetypeBundleFull`).
/// Reserved as a typed handle for any future `try_create_archetype` API that
/// wants to recover instead of panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundleFullError;

impl std::fmt::Display for BundleFullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ArchetypeBundle at capacity ({} slots)", MAX_ARCHETYPES)
    }
}

impl std::error::Error for BundleFullError {}

/// Heap-allocated slab of Archetype slots. Addresses are stable for the bundle's lifetime
/// (D1, U1: private field + Box heap pointer is stable unless reassigned, and we never
/// reassign).
pub struct ArchetypeBundle {
    /// 1024 × size_of::<Archetype>() ≈ 8.6 MB. Heap-allocated; address stable.
    slots: Box<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>,
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
    pub fn new() -> Self {
        // SAFETY (slab init, C3 fix): `Box::<T>::new_uninit()` allocates `T`'s memory
        //   on the heap and returns Box<MaybeUninit<T>>. For T = [MaybeUninit<A>; N],
        //   the inner MaybeUninit layer permits any bit pattern; per-slot init tracked
        //   separately via the `occupied` bitset. No stack-frame copy of 8.6 MB.
        //   Stable since Rust 1.82; boyko-engine targets 1.93+.
        let slots = unsafe {
            Box::<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>::new_uninit().assume_init()
        };
        Self {
            slots,
            occupied: [0u64; 16],
            id_to_slot: Vec::new(),
            free_slots: Vec::with_capacity(16),
            count: 0,
        }
    }

    /// Returns `*mut Archetype` for the given archetype_id, or None if absent.
    /// **HOT API** — used by EcsMaster::create_entity choreography (W7).
    /// SAFETY (U11): pointer minted via raw arithmetic from slab base.
    #[inline]
    pub fn get_archetype_ptr(&self, archetype_id: ArchetypeId) -> Option<*mut Archetype>;

    /// Safe shared-borrow accessor.
    #[inline]
    pub fn get_archetype(&self, archetype_id: ArchetypeId) -> Option<&Archetype>;

    /// Safe exclusive-borrow accessor.
    #[inline]
    pub fn get_archetype_mut(&mut self, archetype_id: ArchetypeId) -> Option<&mut Archetype>;

    /// Inserts an Archetype into a free slot. Returns the assigned slot index.
    /// Returns Err(BundleFullError) at capacity. (C6)
    pub fn add_archetype(&mut self, archetype: Archetype) -> Result<u16, BundleFullError>;

    /// Constructs an Archetype in place in a free slot — avoids 8.4 KB stack copy.
    /// (W6 in-place construction; U13 invariant.)
    pub fn add_archetype_from_components(
        &mut self,
        archetype_id: ArchetypeId,
        component_ids: &[ComponentId],
        arena: &Arena,
    ) -> Result<u16, BundleFullError>;

    /// Drops the archetype at the given id and frees its slot. Returns true on success.
    pub fn remove_archetype(&mut self, archetype_id: ArchetypeId) -> bool;

    /// Iterates occupied archetype slots. O(occupied) cost via bitset.
    pub fn iter(&self) -> impl Iterator<Item = &Archetype>;

    /// Raw pointers — for Phase 9 parallel scheduler hand-off.
    pub fn iter_occupied_ptrs(&self) -> impl Iterator<Item = *mut Archetype>;

    #[inline] pub fn len(&self) -> usize { self.count }
    #[inline] pub fn is_empty(&self) -> bool { self.count == 0 }

    /// Drops all occupied archetypes and clears occupancy.
    pub fn clear(&mut self);
}

impl Drop for ArchetypeBundle {
    /// See D8 / U12 — walks occupancy bitset, drop_in_place each occupied slot.
    /// Box's auto-Drop then frees the slab memory.
    fn drop(&mut self);
}
```

### EntityMaster (new)
```rust
// File: crates/boyko_ecs/src/ecs/core/entity/entity_master.rs

pub struct EntityMaster {
    /// Sparse-indexed by EntityId.0. `entities_inland[i].archetype_ptr.is_null()`
    /// ⇔ slot i is dead. PHASE 7 hot read path: 1 cache line per random lookup.
    entities_inland: Vec<EntityInland>,

    /// Reverse index from EntityId.0 to position in `active_ids`. `u32::MAX` = absent.
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
    pub fn allocate_entity(&mut self) -> Entity;

    /// PHASE 7: registers an active entity with its archetype pointer and unit_index.
    /// (M2 shim: introduced as new method in step 6; renames to `register_entity` in step 9.)
    pub fn register_entity_with_ptr(
        &mut self,
        entity: Entity,
        archetype_ptr: *mut Archetype,
        unit_index: u32,
    );

    /// Marks an entity dead: writes `archetype_ptr = null` and bumps generation.
    pub fn deallocate_entity(&mut self, entity: Entity) -> Option<EntityInland>;

    /// Fast `&EntityInland` accessor by Entity handle. Validates generation.
    #[inline]
    pub fn get_entity_inland(&self, entity: Entity) -> Option<&EntityInland>;

    /// Updates only the unit_index field. Used after swap_remove in an archetype.
    pub fn update_entity_unit_index(&mut self, entity: Entity, new_unit_index: u32) -> bool;

    #[inline]
    pub fn is_entity_valid(&self, entity: Entity) -> bool;

    pub fn get_entity(&self, entity_id: EntityId) -> Option<Entity>;
    pub fn iter_entities(&self) -> impl Iterator<Item = Entity> + '_;
    pub fn entity_count(&self) -> usize { self.active_ids.len() }
    pub fn recycled_entity_count(&self) -> usize { self.free_entity_ids.len() }
    pub fn next_entity_id(&self) -> EntityId { self.next_entity_id }
    pub fn capacity(&self) -> usize { self.entities_inland.len() }
    pub fn clear(&mut self);

    pub(crate) fn rewind_allocate(&mut self, entity: Entity) -> bool;
}
```

---

## Public API (new + changed signatures)

### Changed
| Old | New |
|-----|-----|
| `Entity::generation() -> usize` | `Entity::generation() -> u32` (C1) |
| `Entity::new(id, gen: usize)` | `Entity::new(id, gen: u32)` (C1) |
| `Slot::generation() -> usize` (via Generation alias) | `Slot::generation() -> u32` (C1) |
| `EntityInland::new(archetype_id, unit_index: InlandPoolId, generation)` | `EntityInland::new(archetype_ptr: *mut Archetype, unit_index: u32, generation: u32)` |
| `EntityInland::archetype_id() -> ArchetypeId` | `EntityInland::archetype_ptr() -> *mut Archetype` |
| `EntityInland::unit_index() -> InlandPoolId` | `EntityInland::unit_index() -> u32` |
| `EntityMaster::register_entity(entity, archetype_id, unit_index: InlandPoolId)` | `EntityMaster::register_entity_with_ptr(entity, archetype_ptr: *mut Archetype, unit_index: u32)` (renamed to `register_entity` in final step) |
| `EntityMaster::update_entity_unit_index(entity, InlandPoolId)` | `EntityMaster::update_entity_unit_index(entity, u32)` |
| `ArchetypeBundle::add_archetype(arch) -> ArchetypeId` | `ArchetypeBundle::add_archetype(arch) -> Result<u16, BundleFullError>` (C6) |

### New
| Signature | Purpose |
|-----------|---------|
| `ArchetypeBundle::get_archetype_ptr(ArchetypeId) -> Option<*mut Archetype>` | Hot lookup, returns slab address (C4: minted via raw arithmetic) |
| `ArchetypeBundle::add_archetype_from_components(...) -> Result<u16, BundleFullError>` | In-place construction (W6) |
| `ArchetypeBundle::iter_occupied_ptrs() -> impl Iterator<Item = *mut Archetype>` | Phase 9 scheduler hand-off |
| `ArchetypeMaster::archetype_ptr_for(ArchetypeId) -> Option<*mut Archetype>` | W7 choreography |
| `ArchetypeMaster::add_archetype_and_get_ptr(...) -> Result<(ArchetypeId, *mut Archetype), ...>` | W7 choreography |
| `Archetype::columns(&self) -> &[Column; MAX_COMPONENTS]` | Read-only column view (for tests) |
| `Archetype::refresh_column(&mut self, ComponentId)` | Re-sync column entry after pool add (D5/C5) |
| `Archetype::refresh_all_columns(&mut self)` | Re-sync all columns (Phase-future: arena grow) |
| `EcsMaster::get_component<T: Component>(entity) -> Option<&T>` | Typed fast read |
| `EcsMaster::get_component_mut<T: Component>(entity) -> Option<&mut T>` | Typed fast write |
| `Column::null() -> Column` | Sentinel constructor |
| `EntityInland::dead() -> EntityInland` | Sentinel constructor |
| `EntityInland::is_alive(&self) -> bool` | Branchless null check helper |
| `EntityInland::dangling_for_test(u32, u32)` | Test-only (cfg(test)) constructor (M1) |
| `BundleFullError` | Public error type at the `ArchetypeBundle` layer (C6, never reaches `EcsError` in Phase 7 — higher layers panic instead) |

### Unchanged public API (preserved)
- `EcsMaster::create_entity / spawn_one / spawn_two`
- `EcsMaster::delete_entity`
- `EcsMaster::has_entity / has_component / get_entity_archetype_id`
- `EcsMaster::iter_entities / entity_count / archetype_count`
- `EcsMaster::query_entities` (unchanged signature; internal impl uses fast path)
- `EcsMaster::events / send_event / events_of / update_events`

---

## Invariants & SAFETY contracts

Numbered for the developer to paste verbatim into the SAFETY blocks. **U11-U14 are new in Round 2.**

### Slab stability invariants

**U1** — *Archetype pointer validity* (revised C2):
`EntityInland::archetype_ptr` is either `core::ptr::null_mut()` OR equals `slab_base + (slot_index × size_of::<Archetype>())` for some `slot_index` in `[0, MAX_ARCHETYPES)`. **Stability mechanism**: `ArchetypeBundle::slots` is `Box<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>`. `Box::<T>::new_uninit()` allocates `T` on the heap; the resulting pointer is stable unless the `Box` is reassigned or dropped. The `slots` field is **private** to `ArchetypeBundle` — no external code reassigns it, and we never reassign internally. Therefore `slab_base + offset` is stable for the bundle's entire lifetime. (`Pin` adds no value because `MaybeUninit<Archetype>` is `Unpin`.)

**U2** — *Archetype slot initialization*:
When `EntityInland::archetype_ptr != null`, the slot at that address has been fully initialized via in-place construction (U13) in `ArchetypeBundle::add_archetype_from_components` and has not been dropped. Bit `slot_index` of `ArchetypeBundle::occupied` is set.

**U3** — *Archetype lifetime ≥ EntityInland reachability*:
`EntityMaster` and `ArchetypeBundle` are both owned by `EcsMaster`. Field drop order: `events` → `entity_master` → `archetype_master` → `arena`. Therefore `entity_master` (holding `EntityInland`s with archetype pointers) drops BEFORE `archetype_master` (owning the slab). `EntityMaster::Drop` does NOT dereference any `archetype_ptr` (documented in field comment + enforced by code review).

### Column lookup invariants

**U4** — *Columns array bounds*:
`component_id.0 < MAX_COMPONENTS` is checked by `debug_assert!` at every call site. Release-build `get_unchecked` is sound because `MAX_COMPONENTS` is hard-capped by `ComponentRegistry` (panics on `>= MAX_COMPONENTS`), and `ComponentId` newtype only contains values minted through the registry.

**U5** — *Column ptr provenance*:
When `Archetype::columns[c].ptr != null`, that pointer was minted from `ComponentPool::buffer_ptr()` which returns `self.buffer.as_ptr()` where `buffer` is a `NonNull<u8>` obtained from `Arena::allocate_layout`. The arena allocation lives for the arena's lifetime, which transitively outlives the archetype (drop order: archetype before arena).

**U6** — *Column offset bounds*:
`unit_index < archetype.entity_count() ≤ max_components_per_pool`. The pool was allocated `max_components × component_layout.size()` bytes; the offset `unit_index × stride` is therefore within the pool's allocation. `unit_index` is `u32`; widening to `usize` for multiplication never overflows on 64-bit platforms (u32 × u32 ≤ u64 ≤ usize).

### Typed cast invariants

**U7** — *Typed pointer cast*:
`column.stride == size_of::<T>()` and `column.ptr` is aligned to at least `align_of::<T>()` because the underlying `ComponentPool::buffer` was allocated with `component_layout.align()`, which equals `align_of::<T>()` after the registry layout was set via `register_layout::<T>` (TypeId match enforced by debug_assert).

### Pointer/integer arithmetic

**U8** — *swap_remove pointer rewrite*:
When `Archetype::remove_entity` performs swap_remove, the moved entity's `EntityInland.unit_index` is updated via `EntityMaster::update_entity_unit_index`. The archetype_ptr does NOT change (same archetype). The component pool's buffer is unchanged in base address — only contents shift. No column refresh needed.

### Mutable casts

**U9** — *Exclusive `&mut Archetype` from `*mut`*:
Under `&mut EcsMaster`, casting `*mut Archetype` to `&mut Archetype` is sound because:
- `EcsMaster` transitively owns the slab.
- No other live `&Archetype` exists (the `&mut EcsMaster` borrow excludes all aliasing).
- The slab pointer is valid (U1, U2).
- Single-threaded execution (Arena `!Send + !Sync` propagates).

### Column refresh discipline

**U10** — *Columns ⇔ pools invariant* (revised C5):
The pool's `buffer_ptr` and `component_layout` are **write-once** at `ComponentPoolBundle::add_pool` and never change thereafter (until future arena-grow refresh). `refresh_column(c)` is called only on:
- `add_pool` for the newly-added component_id, AND
- future arena-grow API (not in Phase 7 MVP).

Data-only mutations (`push_entity_components`, `swap_remove_unit`, `pop_entity`, `add_component`) do NOT change pool ptr/stride; no refresh required. `debug_assert!` in every `Archetype` mutation method verifies `columns[c].ptr == get_pool(c).map_or(null, |p| p.buffer_ptr())` for every `component_id ∈ self.component_ids`.

### Pointer minting recipe (NEW, C4)

**U11** — *Pointer minting recipe*:
All `*mut Archetype` pointers stored in `EntityInland` MUST be minted via raw pointer arithmetic from `ArchetypeBundle::slots.as_mut_ptr()` (or `as_ptr()` cast through `*mut`). The minting code MUST NOT create any intermediate `&mut MaybeUninit<Archetype>` or `&MaybeUninit<Archetype>` reference, because such a reference would push a SharedReadOnly or Unique borrow onto the slot's borrow stack, which subsequent `&self` reads via the stored `*mut` would invalidate under Stacked Borrows (Phase 3a M-009 audit).

**Canonical recipe**:
```rust
let slab_base: *mut MaybeUninit<Archetype> = self.slots.as_mut_ptr();
let slot_ptr_mu: *mut MaybeUninit<Archetype> = unsafe { slab_base.add(slot_idx) };
let slot_ptr: *mut Archetype = slot_ptr_mu as *mut Archetype;
// Initialize via raw pointer writes (U13).
// SUBSEQUENT reads/writes via slot_ptr will not retag against a stale borrow.
```

**Anti-pattern (DO NOT)**:
```rust
let slot_ref: &mut MaybeUninit<Archetype> = &mut self.slots[idx];   // Wrong!
slot_ref.write(archetype);
let ptr: *mut Archetype = slot_ref.assume_init_mut() as *mut Archetype;
// `ptr` now carries the `&mut MaybeUninit<Archetype>` borrow stack;
// subsequent &self reads through `ptr` will retag against stale borrow → UB.
```

### Drop discipline (NEW, C7)

**U12** — *Drop discipline*:
`ArchetypeBundle::Drop` calls `drop_in_place` exactly once per occupied slot, computed from the `occupied` bitset. The loop walks each `u64` word; for each set bit, it computes `slot_idx = word_idx × 64 + bit`, mints `slot_ptr: *mut Archetype` via the U11 recipe, and calls `slot_ptr.drop_in_place()`. Empty slots remain `MaybeUninit` and are never touched. After the loop, `Box`'s auto-Drop frees the slab memory.

### In-place archetype construction (NEW, W6)

**U13** — *In-place archetype construction*:
`ArchetypeBundle::add_archetype_from_components` does NOT construct an `Archetype` on the stack and then move it into the slot. Instead, it mints `slot_ptr: *mut Archetype` via U11 and initializes each field exactly once via `core::ptr::addr_of_mut!((*slot_ptr).field).write(value)`. The occupancy bit is set ONLY AFTER all fields are initialized. This avoids the 8.4 KB stack-frame temporary that would otherwise blow the 1 MB Windows stack at high call depths.

### Archetype_ptr cast to &mut for create_entity (NEW, W7)

**U14** — *Archetype_ptr cast to &mut for create_entity*:
In `EcsMaster::create_entity`, after obtaining `archetype_ptr: *mut Archetype` from `ArchetypeMaster::archetype_ptr_for(id)`, the cast `unsafe { &mut *archetype_ptr }` is sound because:
- `archetype_ptr` is valid (just verified `has_archetype(id)` returned true).
- `archetype_ptr` was minted via U11 (no stale borrow stack).
- `&mut EcsMaster` excludes any other borrow into the slab.
- The bundle is single-threaded (Arena `!Send + !Sync` propagates).
- The `entity_master` borrow has been released before this cast (entity allocation completed; `register_entity_with_ptr` is called after the cast goes out of scope).

---

## Performance characteristics

### Hot path walkthrough — `get_component_raw(entity, comp_id)` after Phase 7

```
                                                                       Cache line  Cumulative ns
1. inland = self.entity_master.entities_inland.get(entity.id().0)?;    [Line 1]    ~3 ns
   - Loads 16 B from Vec<EntityInland>; index check folds into the Option.
   - 4 inlands per line; high temporal locality for sequential workloads.

2. inland.archetype_ptr.is_null() → no                                 same line   ~3 ns
   inland.generation == entity.gen → yes (u32 == u32, no cast)         same line

3. archetype = unsafe { &*inland.archetype_ptr };                       [Line 2]    ~6 ns
   - Loads first 64 B of Archetype struct from the slab.
   - For columns at offset 0..8192, columns[c.0] is at offset c.0 * 16.
     For c.0 ∈ [0, 4):    columns[c.0] is in Line 2 (the same line we just loaded).
     For c.0 ∈ [4, 8):    columns[c.0] is in Line 3 (next line).
     For c.0 ∈ [8, 16):   columns[c.0] is in Line 4.
     ...
   - Best case (low ComponentId): 1 line for archetype+column. Average across uniform
     ComponentId distribution: ~1.5 lines (W2 adjustment).

4. column = archetype.columns.get_unchecked(c.0)                       [Line 2 or 3] ~9 ns
   column.ptr.is_null() → no
   Reads (ptr, stride) — 12 B in one access.

5. column.ptr.add(unit_index * stride)                                  [Line 4]   ~12-16 ns
   - Pure arithmetic; one shift + one add (the multiplication folds for power-of-2 strides).
   - The COMPONENT itself is now loaded — the 3rd or 4th "data" line.

TOTAL: 3-4 data cache lines, ~12-16 ns average; 3 lines / ~12 ns best case (W2).
```

**Branch behavior**:
- `inland.archetype_ptr.is_null()`: branch on a value already in a register from step 1. Predictable (almost always taken-not = alive entity in steady state).
- `inland.generation != entity.gen`: also predictable (almost always equal in steady state).
- `column.ptr.is_null()`: predictable (queries are typed; component_id ∈ archetype is the common case).

### Pessimization paths

| Scenario | Behavior |
|----------|----------|
| Dead entity (stale handle) | Returns None at step 2; ~3 ns. |
| Component absent | Returns None at step 4; ~9 ns. |
| First access (cold cache) | Lines 1, 2/3, 4 all cold → ~25 ns/line × 3-4 = ~75-100 ns. |
| Random access pattern across many archetypes | Each archetype access is a fresh slab line; mitigated by slab being contiguous (sequential archetype indices share lines). |
| High `ComponentId.0` (e.g., 256) | columns[256] is at offset 4096 = line 64 within Archetype; cold miss adds ~25 ns to the lookup. (W2 case) |

### Comparison to today (40 ns / 9 lines)

| Step | Old (cache lines) | New (cache lines) | Saved |
|------|------------------:|------------------:|------:|
| Entity validity check | 3 | 1 (combined with deref) | 2 |
| Archetype lookup | 2 | 0 (direct ptr) | 2 |
| Pool lookup | 3 | 0 (in columns) | 3 |
| Component address | 1 | 1 | 0 |
| **Total** | **9** | **3-4** | **5-6** |

Empirical estimate ratio: 14 ns / 40 ns = **~2.9× speedup** average. Matches Bevy reports of "~10ns / 3 cache lines" for the best-case path.

---

## Memory budget (per master)

Updated for C1 (Entity narrowing) and Box-not-Pin (C2).

| Structure | Old | New | Delta |
|-----------|----:|----:|------:|
| `ArchetypeBundle::archetypes: Vec<Archetype>` | ~120 B × N archetypes (heap) | — | — |
| `ArchetypeBundle::archetype_to_index: SparseMap` | ~(64 B + 24 B × N) | — | — |
| **New** `ArchetypeBundle::slots` (heap slab) | — | 1024 × ~8.4 KB = **~8.6 MB** | +8.6 MB |
| **New** `ArchetypeBundle::occupied` (bitset) | — | 128 B | +128 B |
| **New** `ArchetypeBundle::id_to_slot: Vec<u16>` | — | 2 B × max_id | +trivial |
| **New** `ArchetypeBundle::free_slots: Vec<u16>` | — | 2 B × removed | +trivial |
| **New** `Archetype::columns: [Column; 512]` | — | 8 KB inline (in slab) | included in slab |
| `EntityMaster::entities: Vec<Entity>` | 24 B × E (Entity was 24 B) | — | -24E B |
| `EntityMaster::entity_map: SparseMap<EntityInland>` | ~24 B sparse + 24 B × E_active dense | — | — |
| **New** `EntityMaster::entities_inland: Vec<EntityInland>` | — | 16 B × E | +16E B |
| **New** `EntityMaster::sparse_to_active: Vec<u32>` | — | 4 B × E | +4E B |
| **New** `EntityMaster::active_ids: Vec<EntityId>` | — | 8 B × E_active | +8E_active B |
| `Entity` (user-held handle) | 24 B | 16 B (C1) | -8 B per handle |
| Net entity overhead per fresh slot | ~48 B (incl. SparseMap dense) | 20 B + 8 B if active | **-20 to -36 B / slot** |

**Net change**:
- +8.6 MB upfront for slab (one-time, regardless of usage).
- -20 to -36 B per entity (savings scale with entity count; bigger with C1).
- Crossover: at ~300K entities, the per-entity savings cancel the slab cost. For typical games (100K-1M entities), this is a net wash or slight reduction in heap usage with **much** better cache behavior.

**Absolute upper bound** (with C1 narrowing):
- Slab: 8.6 MB
- 1M entities: 20 B × 1M = 20 MB for `entities_inland + sparse_to_active`
- 1M active: +8 MB for `active_ids`
- **Total ECS bookkeeping**: ~36 MB for 1M entities + 1024 archetypes.

---

## Forward compatibility

### Phase 8 (hybrid sparse-set components)

If we later add sparse-set storage for tag/rare components, the dispatch scheme is **NOT** the `_reserved` field (W5 fix). Options for Phase 8:
- **Tagged pointer**: bit 0 of `column.ptr` marks SparseSet (arena buffers align ≥ 8, so bit 0 is free). `column.ptr & 1 == 1` ⇒ pointer is `(SparseColumn*)(ptr & !1)`.
- **Parallel kinds array**: `Archetype::column_kinds: [u8; MAX_COMPONENTS]` — 0 = Table, 1 = SparseSet, 2 = absent. 512 B per archetype overhead.
- **Separate SparseColumn table**: `Archetype::sparse_columns: SmallVec<...>` consulted when `column.ptr` is null and sparse storage might exist.

Phase 7 makes no commitment. `_reserved: u32` exists only to keep `Column` at 16 B; do not rely on its layout.

### Phase 9 (parallel scheduler)

- `get_component_raw` is `&self` — multiple workers can hit it in parallel without synchronization.
- `*const Archetype` (cast from `*mut`) deref under `&self` is sound if no `&mut EcsMaster` exists during system execution.
- For per-column borrow-checking during system dispatch, the scheduler reads `archetype.signature.mask()` to compute conflicts. No new design needed.

**Phase 9 prerequisites NOT established by Phase 7** (F1 fix):
- `Archetype: Sync` is required for `&Archetype` to cross threads. Currently `Archetype: !Sync` because:
  - `Arena: !Sync` (interior-mutable arena state).
  - `ComponentPool` contains raw pointers without `Sync` markers.
  - `ComponentPoolBundle` inherits both.
- Achieving `Archetype: Sync` requires:
  - `Arena: Sync` — either interior locking (slow), or post-setup immutability + explicit `unsafe impl Sync`.
  - `ComponentPool: Sync` — achievable since `&ComponentPool` would be read-only after setup; needs `unsafe impl Sync` with documented invariant.
- Phase 7 does NOT add these `Sync` impls. They are deferred to Phase 9.
- `EntityInland` carries `*mut Archetype`; for Phase 9 worker hand-off, `EntityInland: Send` is sufficient (transfer of pointer across threads is sound; deref still requires `Archetype: Sync` per above). The `unsafe impl Send for EntityInland` is deferred until Phase 9 actually requires worker access.

### Future arena grow

Today (Phase 7 MVP): the arena is fixed 64 MB; pool buffers never move. The `refresh_column` API exists but is only called on pool creation (`add_pool`). **No refresh-on-grow logic is shipped in Phase 7** — `refresh_all_columns` is marked `#[cold]` and reserved for future use.

If `Arena` ever grows by reallocation, the future protocol:
- Arena exposes `current_base_ptr()` and `version: u64`.
- `Archetype::on_arena_grew()` calls `refresh_all_columns`.
- `ComponentPool::on_arena_relocated(old_base, new_base)` recomputes `self.buffer`.

---

## Tests required

### Unit tests

**`entity.rs`** (NEW for C1)
- `entity_generation_is_u32`
- `entity_size_is_16_bytes`
- `entity_increment_generation_wraps_at_u32_max`

**`entity_inland.rs`**
- `entity_inland_dead_has_null_ptr`
- `entity_inland_dead_is_not_alive`
- `entity_inland_layout_size_is_16`
- `entity_inland_layout_field_offsets_match_design`
- `entity_inland_dangling_for_test_has_nonnull_ptr` (NEW for M1)

**`archetype.rs`**
- `column_null_is_default`
- `archetype_columns_initially_all_null`
- `archetype_create_by_ids_populates_columns_for_listed_components`
- `archetype_register_component_populates_one_column`
- `archetype_columns_invariant_holds_after_create_by_ids`
- `archetype_columns_ptr_matches_pool_buffer_ptr`
- `archetype_columns_stride_matches_pool_layout_size`
- `archetype_columns_for_missing_component_is_null`
- `archetype_swap_remove_does_not_invalidate_columns` (NEW for C5/U10)

**`archetype_bundle.rs`**
- `bundle_new_has_zero_occupied`
- `bundle_add_archetype_returns_stable_ptr`
- `bundle_add_then_get_archetype_ptr_returns_same_address` — *load-bearing for U1*
- `bundle_address_stable_across_many_inserts`
- `bundle_remove_archetype_frees_slot_for_reuse`
- `bundle_remove_does_not_invalidate_other_pointers`
- `bundle_iter_visits_all_occupied`
- `bundle_iter_skips_empty_slots`
- `bundle_drop_drops_all_occupied_archetypes` (sentinel ComponentPool with side-effecting Drop)
- `bundle_drop_does_not_drop_empty_slots` (MaybeUninit safety; NEW assertion for C7)
- `bundle_at_capacity_returns_err` (1024 archetypes; NEW for C6)
- `bundle_at_capacity_one_over_returns_err` (NEW for C6)
- `bundle_new_does_not_overflow_stack` (NEW for C3; runs on Windows with 1 MB default stack)
- `create_1000_archetypes_no_stack_overflow` (NEW for W6 in-place construction)

**`entity_master.rs`**
- `entity_master_allocate_assigns_sequential_ids`
- `entity_master_register_writes_inland`
- `entity_master_deallocate_nulls_archetype_ptr`
- `entity_master_deallocate_bumps_generation`
- `entity_master_recycled_id_keeps_generation_continuity`
- `entity_master_is_entity_valid_returns_false_for_stale_handle`
- `entity_master_iter_entities_O_active_with_sparse_population`
- `entity_master_iter_entities_yields_correct_set_after_recycle`
- `entity_master_rewind_allocate_unchanged_semantics`
- `entity_master_generation_wraps_at_u32_max` (NEW for C1; uses test-only generation injection)

**`ecs_master.rs`**
- `get_component_raw_returns_correct_ptr_for_first_entity`
- `get_component_raw_returns_none_for_dead_entity`
- `get_component_raw_returns_none_for_wrong_generation`
- `get_component_raw_returns_none_for_absent_component`
- `get_component_raw_handles_recycled_entity_correctly` (ABA test)
- `get_component_after_swap_remove_returns_correct_ptr`
- `get_component_typed_returns_T_reference`
- `get_component_typed_debug_asserts_typeid_match`
- `set_component_raw_updates_value`
- `set_component_raw_returns_false_for_dead_entity`
- `has_entity_for_active_returns_true`
- `has_entity_for_dead_returns_false`
- `has_component_returns_true_when_present`
- `delete_entity_then_create_recycles_slot_and_works`
- `create_archetype_at_capacity_returns_archetype_bundle_full_error` (NEW for C6)

### Integration tests (`tests/phase7_random_access.rs`)

- `phase7_lookup_chain_invariants`
- `phase7_aba_safety`
- `phase7_address_stability_under_archetype_churn`
- `phase7_columns_refresh_after_register_component`
- `phase7_bundle_full_error_propagates` (NEW for C6)
- `phase7_generation_collision_at_u32_wrap_caught_by_test_seed` (NEW for C1 / T2)

### Property tests (`proptest`)

- `prop_get_component_raw_consistent_with_slow_path`
- `prop_generation_check_rejects_stale_handles`
- `prop_iter_entities_yields_exact_active_set` (NEW; ties to W1)

### Miri tests

Run via `cargo +nightly miri test` after enabling `#[cfg(miri)]` markers:

- `phase7_miri_slab_address_stability_under_box`
- `phase7_miri_maybe_uninit_discipline`
- `phase7_miri_null_ptr_sentinel_correctness`
- `phase7_miri_mut_archetype_cast_under_mut_ecs_master`
- `phase7_miri_drop_order_entity_master_before_archetype_master`
- `phase7_miri_archetype_ptr_no_retag_ub` (NEW for C4) — create EcsMaster, create archetype, spawn entity, take `&self` borrow, dereference stored `archetype_ptr`; Miri's Stacked Borrows checker must not fire.
- `phase7_miri_bundle_drop_runs_archetype_drop_for_occupied_only` (NEW for C7) — sentinel ComponentPool with side-effecting drop_fn incrementing a static counter; assert counter matches occupied count post-Drop.
- `phase7_miri_in_place_construction_no_uninit_read` (NEW for W6) — exercises `add_archetype_from_components`; Miri must not flag uninit read.

### Loom tests (Phase 9 prep, optional now)

Not required for Phase 7 (single-threaded). Marker tests added with `#[ignore]`:
- `loom_get_component_raw_under_shared_self`

### Criterion benchmarks

See D10 table.

---

## Open questions / future work

1. **Q-001 (deferred)**: Should `EntityInland::unit_index` and `generation` be combined into a single `u64` for atomic loads? Required if Phase 9 wants lock-free entity-validity reads from worker threads. Recommend: keep separate fields for Phase 7; revisit at Phase 9.

2. **Q-002 (deferred)**: Should `Archetype::columns` migrate from inline `[Column; 512]` to a packed dynamic structure (e.g., `SmallVec<[Column; 16]>` with hash lookup) to reduce per-archetype overhead from 8 KB to ~256 B for typical archetypes? Reduces slab from 8.6 MB to ~256 KB. Cost: random access becomes O(log N) per archetype. Recommend: defer until profiling shows the 8 KB hot table actually wastes ICache/DCache pressure.

3. **Q-003**: When `MAX_COMPONENTS` grows from 512 to e.g. 1024, Column table doubles to 16 KB per archetype. Slab grows to ~17 MB. Hard wall at `MAX_COMPONENTS × MAX_ARCHETYPES × size_of::<Column>() > 16 MB` (W3 threshold). Document in `constants.rs`.

4. **Q-004**: `Archetype::component_pools.sparse_indexes` is now redundant with `columns` for the read path. We keep it for the mutation path because it gives us a quick "pool index in Vec<ComponentPool>" answer. Flagged for review in Phase 8 pool reorganization.

5. **Q-005**: `Vec<EntityInland>` grows by `Vec::push`. When it reallocates, the address of `entities_inland[i]` for an old `i` changes. Search confirms: no caller holds a long-lived reference into it — every `get_entity_inland` returns either `&EntityInland` (lifetime bound to the function call) or `EntityInland` (by Copy). After resize, the next access goes through `.get(id)` again. ✓ Safe.

6. **Q-006**: Future `delete_archetype` when an archetype becomes empty — entities in OTHER archetypes are unaffected. The freed slot goes onto `free_slots` and may be reused. Correct behavior.

7. **Q-007 (NEW for C1)**: If a future profiling shows the 32-bit generation wrap is becoming a bottleneck (e.g., a leak-checker / fuzzer hits the wrap deliberately), the upgrade path is: bump `Generation` alias to `u64`, accept the 4 B per `Entity` and 4 B per `EntityInland` cost. The `_reserved: u32` in `Column` is unaffected. Migration is mechanical.

---

## Changelog vs Round 1

| Item | Round 1 | Round 2 | Why |
|------|---------|---------|-----|
| `Entity::generation` type | `usize` (preserved) | `u32` (narrowed) | C1: silent truncation = ABA escape; pick consistent representation |
| `Slot::generation` / `Generation` alias | `usize` | `u32` | C1: must match Entity |
| Slab field type | `Pin<Box<[MaybeUninit<Archetype>; 1024]>>` | `Box<[MaybeUninit<Archetype>; 1024]>` | C2: `Pin` is decorative; real invariant is private + Box stability |
| Slab construction | Unspecified | `Box::<...>::new_uninit().assume_init()` recipe | C3: avoid 8.6 MB stack-frame on construction |
| Pointer minting | Not specified | Explicit recipe via raw arithmetic from slab base | C4: avoid stale Stacked Borrows from `&mut MaybeUninit` reborrow |
| `refresh_column` coverage | "after pool addition" (vague) | Exhaustive ComponentPoolBundle method audit table | C5: developer needs unambiguous contract |
| MAX_ARCHETYPES rationale | "ComponentMask 512-bit limit" (wrong) | Cites `archetype_bit_set.rs::MAX_ARCHETYPES` | C6: corrected attribution |
| `add_archetype` overflow | Unspecified | Returns `Result<u16, BundleFullError>`; debug panics | C6: defined behavior |
| `ArchetypeBundle::Drop` body | Sketched as "walks bitset" | Full code with U12 invariant | C7: prevents drop_fn leak |
| `iter_entities` cache profile | "good locality O(active)" | "hot sequential active_ids + potentially cold random per-entity inland lookup" | W1: realistic |
| Hot path target | "3 lines / ~12 ns" | "3-4 lines / ~12-16 ns average" | W2: realistic for any ComponentId |
| Slab size budget | Implicit | Pre-condition: ≤ 16 MB total | W3: explicit limit |
| `get_component_raw_mut` doc | None | "EntityInland is Copy; copy 16 B to drop EntityMaster borrow" | W4 |
| Column `_pad` | "reserved for Phase 8 discriminant" (misleading) | Renamed `_reserved`, "may be repurposed, do not rely" | W5 |
| `add_archetype_from_components` body | Not specified | In-place via `addr_of_mut!().write()`; U13 invariant | W6: avoid 8.4 KB stack temp |
| `create_entity` choreography | Sketched | Explicit; uses `archetype_ptr_for` + `register_entity_with_ptr` | W7 |
| Migration steps | 9 steps, "each compiles" claim | 10 steps with shim approach; per-step build state | M2: each commit compiles and tests pass |
| Test migration | Not addressed | Section listing affected files + `dangling_for_test` helper | M1 |
| Phase 9 prereqs | "Send + scheduler enforces" | "`Archetype: Sync` required, NOT established by Phase 7" | F1 |
| SAFETY invariants count | U1-U10 | U1-U14 (added U11, U12, U13, U14) | C4, C7, W6, W7 |

---

## Files affected (absolute paths)

- `D:\claude\BoykoEngine-ecs\crates\boyko_utils\src\identifiers\primitives.rs` — change `Generation` alias to `u32` (C1)
- `D:\claude\BoykoEngine-ecs\crates\boyko_utils\src\identifiers\slot.rs` — propagates Generation alias change (C1)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\entity\entity.rs` — narrow `generation: usize` to `u32` (C1)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\entity\entity_inland.rs` — full rewrite
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\entity\entity_master.rs` — full rewrite
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\archetype\archetype.rs` — add `Column`, `columns: [Column; 512]`, `refresh_column`, `refresh_all_columns`, `register_component_inplace`
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\archetype\archetype_bundle.rs` — full rewrite with heap slab (C2, C3, C6, C7, W6)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\archetype\archetype_master.rs` — add `archetype_ptr_for`, `add_archetype_and_get_ptr` (W7); propagate `BundleFullError` (C6)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs` — rewrite read/write methods to fast path; add typed wrappers; `create_archetype` panics on `BundleFullError` via `expect(...)` (C6 — no new EcsError variant)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\component\component_pool_bundle.rs` — minor (no API change; bundle bypassed on read)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\constants.rs` — document slab-size pre-condition (W3)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\iters\query.rs` / `query_state.rs` — adapt to changed `iter_entities` (signature preserved)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\tests\drop_fn.rs` — apply test migration recipe (M1)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\benches\random_access.rs` — **new file**
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\tests\phase7_random_access.rs` — **new file**

## Self-check against the plan-readiness checklist

- **Structure**: ✓ all sections present with Round 2 fixes.
- **Data structures**: ✓ every field has type + comment + role; `repr(C)` specified; size + offset asserts included.
- **API**: ✓ minimal surface, no `dyn Trait`, lifetimes implicit-and-trivial.
- **Multithreading**: ✓ today single-threaded; Phase 9 prep documented honestly (F1).
- **Correctness**: ✓ 14 SAFETY invariants (U1-U14); generation/version explicit; drop order proven; minting recipe + drop discipline + in-place construction + create_entity choreography codified.
- **Integration**: ✓ 10-step ordered migration with per-step build state and shims.
- **Validation**: ✓ ~50 unit tests, 6 integration tests, 3 proptests, 8 miri tests, criterion harness specified.

Ready for `architecture-critic` Round 2 review.

---

## Round 2 summary (for parent agent spot-check)

Round 2 patches the Round 1 plan to address all critic findings without redesigning. Architecture (slab + bitset, hot/cold split, NULL-sentinel, refresh contract) is preserved.

**Critical fixes (C1-C7) — all addressed**:
1. C1: `Entity::generation` narrowed from `usize` to `u32` (option a); `boyko_utils::Generation` alias also narrowed; ABA wrap window documented at ≥13.6 years per slot. Eliminates silent truncation collision.
2. C2: `Pin<Box<...>>` dropped (decorative — `MaybeUninit<Archetype>` is `Unpin`); replaced with plain `Box<...>`. U1 invariant rewritten to cite "private field + Box heap stability" as the real mechanism.
3. C3: Slab construction recipe pinned down to `Box::<...>::new_uninit().assume_init()` (stable since Rust 1.82). No stack-frame 8.6 MB temp.
4. C4: Explicit pointer-minting recipe via raw arithmetic from slab base; new invariant U11 forbids `&mut MaybeUninit` reborrow. Miri test added.
5. C5: Exhaustive `ComponentPoolBundle` method audit table; U10 rewritten to state pool ptr/stride is write-once at `add_pool`.
6. C6: MAX_ARCHETYPES correctly attributed to `archetype_bit_set.rs`; `add_archetype` returns `Result<u16, BundleFullError>` at the bundle layer; `ArchetypeMaster::create_archetype` / `EcsMaster::create_archetype` panic on overflow via `expect(...)` to preserve their `-> ArchetypeId` signature and avoid cascading `EcsResult<ArchetypeId>` through every test / bench / caller. `EcsError::ArchetypeBundleFull` is NOT introduced in Phase 7 — deferred for any future `try_create_archetype` Result-returning API.
7. C7: Explicit `Drop` body with bitset-walk + `drop_in_place`; new invariant U12; Miri test added.

**Important fixes (W1-W8) — all addressed**: realistic `iter_entities` cache profile (W1), 3-4 lines target (W2), 16 MB slab pre-condition (W3), `get_component_raw_mut` doc note (W4), `_pad` renamed `_reserved` with no Phase 8 promise (W5), in-place archetype construction with U13 (W6), `create_entity` choreography with U14 (W7), W8 same as C6.

**Migration (M1-M2)**: test migration recipe with `dangling_for_test` helper; 10-step plan with shim approach (each commit compiles + tests pass).

**Forward-compat (F1)**: Phase 9 `Archetype: Sync` requirement documented honestly as NOT established by Phase 7.

**New invariants added**: U11 (pointer minting), U12 (drop discipline), U13 (in-place construction), U14 (archetype_ptr cast to &mut for create_entity). Numbering contiguous.

**Files relevant to the task**:
- D:\claude\BoykoEngine-ecs\docs\PHASE-7-FAST-RANDOM-ACCESS-PLAN.md (the file to be replaced with the Round 2 plan above)
- D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\iters\archetype_bit_set.rs (MAX_ARCHETYPES source, C6)
- D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\entity\entity.rs (Entity::generation type, C1)
- D:\claude\BoykoEngine-ecs\crates\boyko_utils\src\identifiers\slot.rs (Slot::generation, C1)
- D:\claude\BoykoEngine-ecs\crates\boyko_utils\src\identifiers\primitives.rs (Generation alias, C1)