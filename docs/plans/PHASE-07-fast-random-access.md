# Phase 7 — Fast random component access

**Status:** 🟡 PLANNED — architectural plan approved (architect → critic
3 rounds + R2 follow-up patches). Implementation not started.
**Branch:** `ecs`.
**Detailed architectural plan:** [`docs/PHASE-7-FAST-RANDOM-ACCESS-PLAN.md`](../PHASE-7-FAST-RANDOM-ACCESS-PLAN.md)
(1483 lines, all SAFETY contracts U1 – U14, per-step migration recipe).
**Latest commit on plan:** `2e73d55` (R2 critic follow-ups).

## Goal — what success looks like

Collapse `EcsMaster::get_component_raw(entity, comp_id)` from
~40 ns / 9 cache lines into ~12-16 ns / 3-4 cache lines, matching
Bevy's `Table` / `Column` model. Same effect on `get_component_mut`,
`set_component`, `has_entity`. Cost paid in one-time slab allocation
(~8.4 MB heap) and a slightly larger `Archetype` struct (~8.4 KB).

## Why now — user-driven prioritisation

Saved in `feedback-foundations-before-apis`: **optimise primitives
before designing higher-level APIs**. Phase 8 (system API, query DSL,
`SystemParam`) is meaningless syntactic sugar if the underlying
`get_component` is 4× slower than the reference (Bevy ~10 ns).

User quote (Russian original): *"Мне кажется самое главное сначала
это спроектировать максимально быстрый random доступ, нет?"*

## Target metrics (all hot, release, AMD Zen3 / Intel Alder Lake)

| Operation | Today | Target |
|-----------|-------|--------|
| `get_component_raw` | ~40 ns / 9 lines | **~12-16 ns / 3-4 lines** |
| `get_component_raw_mut` | ~40 ns | **~12-16 ns** |
| `has_entity` | ~15 ns / 3 lines | **~5 ns / 1 line** |
| `set_component_raw` | ~45 ns | **~15-18 ns** |
| `create_entity` | ~150 ns | **≤ 160 ns** (5 % budget) |
| Cache misses / random lookup | ~5-7 | **2-3** |
| `EntityInland` size | 24 B | **16 B** |
| `Entity` size | 24 B | **16 B** |
| `Archetype` size | small | **~8.4 KB** (inline column table) |
| Per-bundle slab allocation | 0 | **~8.4 MB** (one-time) |

## High-level design — five load-bearing decisions

| D | Decision | Why |
|---|----------|-----|
| D1 | `Box<[MaybeUninit<Archetype>; 1024]>` slab with `[u64; 16]` occupancy | Stable pointers across `create_archetype` calls; no `Vec::push` reallocation invalidating pointers stored in `EntityInland`. |
| D2 | `generation: u32` everywhere (`Entity`, `EntityInland`, `Slot`) | Eliminates the `as u32` truncation around the existing hot path. Wrap window = 2^32 deallocs/slot ≥ 13.6 years at 10 deallocs/sec — orders of magnitude rarer than cosmic-ray soft errors. |
| D3 | New `EntityMaster` layout: `entities_inland: Vec<EntityInland>` + `active_ids` + `sparse_to_active` | Removes the `SparseMap` lookup from the random-access path. Replaces 3 cache lines of indirection with 1. |
| D4 | `Archetype.columns: [Column; MAX_COMPONENTS]` inline | Avoids the `ComponentPoolBundle` chase. `Column { ptr, stride, _reserved }` packs to 16 B. |
| D5 | Pointer minting recipe — raw arithmetic from slab base only, no `&mut MaybeUninit` reborrow | Stops Stacked Borrows / Tree Borrows retag UB (precedent: Phase 3a Miri retag fix). |

Plus 9 numbered SAFETY invariants U1–U14 in the detailed plan covering
slab stability, drop discipline, in-place construction, pointer
minting, generation match, and bundle capacity.

## 10-step implementation plan — actionable checklist

Each step compiles independently and `cargo test --all-targets`
passes after each commit. The full description of every step lives
in the detailed plan at §D9; this is the operational checklist
agents follow.

### Step 0 — `Generation = u32` cascade (`boyko_utils`)

**Files:**
- `crates/boyko_utils/src/identifiers/primitives.rs`
- `crates/boyko_utils/src/identifiers/slot.rs`
- `crates/boyko_utils/src/sparse_map/sparse_slot_map.rs`

**Action:** change `pub type Generation = usize;` to `u32`. Update
`SparseSlotMap::push_dense` signature `generation: usize` →
`generation: Generation`. Numeric literals (`0`) continue to compile
via inference.

**Acceptance:** `cargo check -p boyko_utils` clean; existing tests
pass. No new tests required.

### Step 1 — `Entity::generation: u32`

**Files:** `crates/boyko_ecs/src/ecs/core/entity/entity.rs`

**Action:** narrow `Entity::generation` to `u32`. Update
`Entity::new`, `generation()`, `with_id`, `increment_generation`,
`is_same`, `From<Slot>`, `From<Entity> for Slot`.

**Acceptance:** `cargo check -p boyko_ecs` clean. Update any test that
passed `1usize` for generation to `1u32` (or `1` with type inference).

### Step 2 — Add new `EntityInland` parallel struct

**Files:** `crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs`

**Action:** add the new struct as `EntityInlandFast`
`{ archetype_ptr: *mut Archetype, unit_index: u32, generation: u32 }`.
Add layout asserts (size 16, align 8, offsets 0 / 8 / 12). Add
`#[cfg(test)] pub(crate) fn dangling_for_test(...)`. **Do not** delete
the old `EntityInland` yet.

**Acceptance:** both structs co-exist; `cargo check` clean.

### Step 3 — `Archetype.columns` + `create_entity` signature

**Files:**
- `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs`
- `crates/boyko_ecs/src/ecs/core/iters/query.rs` (3 call sites: 741, 760, 886)
- `crates/boyko_ecs/benches/archetype.rs` (2 call sites: 71, 109)
- `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` tests (4 sites: 424, 466, 728, 764)

**Action:**

1. Add `pub struct Column { ptr: *mut u8, stride: u32, _reserved: u32 }`
   with `Column::null()` const and `#[repr(C)]`. Const-assert all-zero
   layout (`ptr == null`, `stride == 0`, `_reserved == 0`).
2. Add `columns: [Column; MAX_COMPONENTS]` field to `Archetype`
   at offset 0. Archetype size grows to ~8.4 KB.
3. Add `Archetype::refresh_column(c)` /
   `Archetype::refresh_all_columns()`. Called inside
   `create_by_ids` and `register_component` after `add_pool`.
4. **Delete** `Archetype::init_entity_inland(...)` (was setting
   `archetype_id` on the old inland — new inland has no such field).
5. **Change** `Archetype::create_entity(&mut self, entity_id, &mut EntityInland, ...)`
   to `Archetype::create_entity(&mut self, entity_id, &mut new_unit_index: u32, ...)`.
   Update 7 call sites listed above.

Read path **still** uses `component_pools.get_pool` until step 8.

**Acceptance:** `cargo test --all-targets` green; benches compile.

### Step 4 — `ArchetypeBundle` slab rewrite

**Files:** `crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs`

**Action:** rewrite per D9 step 4 in the detailed plan:

1. Replace `archetype_to_index: SparseMap` /
   `Vec<Archetype>` with
   `slots: Box<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>`,
   `occupied: [u64; 16]`,
   `id_to_slot: Vec<u16>`,
   `free_slots: Vec<u16>`,
   `count: usize`.
2. Slab construction via
   `Box::<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>::new_uninit().assume_init()`
   in `unsafe` block with SAFETY comment (C3 fix).
3. New methods:
   - `get_archetype_ptr(id) -> Option<*mut Archetype>` (C4 pointer
     minting recipe — raw arithmetic from `self.slots.as_ptr()`, no
     `&mut MaybeUninit` reborrow).
   - `add_archetype_from_components(...) -> Result<u16, BundleFullError>`
     — in-place via `addr_of_mut!(...).write(...)`,
     `write_bytes(addr_of_mut!((*slot_ptr).columns), 0, MAX_COMPONENTS)`
     for column init (W6 fix — no 8 KB stack temporary).
   - `iter_occupied_ptrs()` returning `impl Iterator<Item = *mut Archetype>`.
4. Old API (`get_archetype`, `get_archetype_mut`, `add_archetype`,
   `remove_archetype`, `iter`, `len`, `is_empty`, `clear`) re-implemented
   over the new backing.
5. Add `BundleFullError` newtype error.
6. **`impl Drop` body:** walk `occupied` bitset (BLSR /
   `word & word.wrapping_sub(1)`), call `drop_in_place` per occupied
   slot (C7 / U12 fix).

**Acceptance:** all existing callers work; `cargo test --all-targets`
green. Add Miri test
`phase7_miri_archetype_ptr_no_retag_ub`. Add Miri test
`phase7_miri_bundle_drop_runs_archetype_drop_for_occupied_only`.

### Step 5 — `ArchetypeMaster` wrappers

**Files:** `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs`

**Action:**

- Add `pub fn get_archetype_ptr(&self, id) -> Option<*mut Archetype>`
  wrapping the bundle method.
- Add `pub fn archetype_ptr_for(&self, id) -> Option<*mut Archetype>`
  alias used by the `create_entity` choreography (W7 fix).
- Add `pub fn add_archetype_and_get_ptr(...)` returning
  `(ArchetypeId, *mut Archetype)` for the create-archetype path
  (used by step 7).
- Keep all existing methods; their internal impls now go through
  the new bundle.

**Acceptance:** `cargo test --all-targets` green.

### Step 6 — `EntityMaster` shim

**Files:** `crates/boyko_ecs/src/ecs/core/entity/entity_master.rs`

**Action:**

- Add new fields:
  - `entities_inland_fast: Vec<EntityInlandFast>` (parallel to old
    `entities` / `entity_map`).
  - `sparse_to_active: Vec<u32>` (sparse → dense index in `active_ids`).
  - `active_ids: Vec<EntityId>` (dense list for D3 iteration).
- Add new method `register_entity_with_ptr(entity, archetype_ptr,
  unit_index: u32)` writing to **both** old store (`entity_map` /
  `entities`) and new store (`entities_inland_fast` / `sparse_to_active`
  / `active_ids`). Old `register_entity` continues to work.

**Acceptance:** `cargo test --all-targets` green. Both stores stay
in sync invariant-wise; existing tests verify the old path.

### Step 7 — `EcsMaster` fast read methods + typed wrappers + new `create_entity`

**Files:** `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs`

**Action:**

- Add `get_component_raw_fast(entity, comp_id) -> Option<*const u8>`
  using the new inland + column path:

  ```rust
  pub fn get_component_raw_fast(&self, entity: Entity, comp_id: ComponentId) -> Option<*const u8> {
      let inland = self.entity_master.entities_inland_fast.get(entity.id().0)?;
      if inland.archetype_ptr.is_null() { return None; }
      if inland.generation != entity.generation() { return None; }
      // SAFETY (U11/U14): pointer is from slab; slab address stable; &EcsMaster gives shared access.
      let archetype = unsafe { &*inland.archetype_ptr };
      let column = archetype.columns[comp_id.0];
      if column.ptr.is_null() { return None; }
      Some(unsafe { column.ptr.add(inland.unit_index as usize * column.stride as usize) as *const u8 })
  }
  ```

- Symmetric `get_component_raw_mut_fast`, `has_entity_fast`,
  `set_component_raw_fast`.
- Typed wrappers `get_component<T: Component>(entity) -> Option<&T>`,
  `get_component_mut<T: Component>(entity) -> Option<&mut T>` doing
  `T::component_id()` lookup + cast.
- Switch `EcsMaster::create_entity` to the W7 choreography:

  ```rust
  fn create_entity(&mut self, archetype_id, components) -> EcsResult<Entity> {
      if !self.archetype_master.has_archetype(archetype_id) {
          return Err(EcsError::ArchetypeNotFound(archetype_id));
      }
      let archetype_ptr = self.archetype_master.archetype_ptr_for(archetype_id).expect("...");
      let entity = self.entity_master.allocate_entity();
      // SAFETY (U14): just verified; single-threaded &mut EcsMaster.
      let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
      let mut new_unit_index: u32 = 0;
      if !archetype.create_entity(entity.id(), &mut new_unit_index, components) {
          self.entity_master.rewind_allocate(entity);
          return Err(EcsError::ArchetypeCapacityExceeded(archetype_id));
      }
      self.entity_master.register_entity_with_ptr(entity, archetype_ptr, new_unit_index);
      Ok(entity)
  }
  ```

**Acceptance:** old slow path still works; new fast path passes
both old and new tests.

### Step 8 — Switch existing methods to fast path

**Files:** `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs`

**Action:** make `get_component_raw`, `get_component_raw_mut`,
`set_component_raw`, `has_entity`, `has_component`,
`get_entity_archetype_id` use the fast-path bodies directly.
Delete the `*_fast` aliases.

**Acceptance:** read path exclusively fast; all tests pass.

### Step 9 — Remove shims

**Files:**
- `crates/boyko_ecs/src/ecs/core/entity/entity_master.rs`
- `crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs`
- `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs`

**Action:**

- Delete old `entity_map: SparseMap` and old `entities: Vec<Entity>`.
- Delete old `register_entity(entity, archetype_id, InlandPoolId)`.
- Delete old `EntityInland` struct.
- Rename `EntityInlandFast` → `EntityInland`. Rename
  `entities_inland_fast` → `entities_inland`.
- Delete `archetype_to_index: SparseMap` from `ArchetypeBundle`
  (already replaced by `id_to_slot` in step 4).

**Acceptance:** code minimal; all tests pass; no `_fast` suffix
remains in the codebase.

### Step 10 — Tests + benches

**Files:**
- `crates/boyko_ecs/benches/random_access.rs` (new file)
- Various test files for migration recipe M1

**Action:**

1. Apply test migration recipe (M1):
   - `archetype.rs:423, 465, 505, 547` — restructure via real
     `EcsMaster` flow.
   - `entity_master.rs:338, 354, 372, 380, 393, 423, 425, 427, 435`
     — use `EntityInland::dangling_for_test`.
   - `tests/drop_fn.rs:658` — same pattern.
2. Add criterion benches per D10 table:
   - `bench_get_component_raw_hot` — 10 K entities / 1 archetype,
     random shuffled, hot cache. **≤ 16 ns/op**.
   - `bench_get_component_raw_cold` — flush cache before each.
     **≤ 90 ns/op**.
   - `bench_get_component_typed` — release **≤ 16 ns/op**.
   - `bench_has_entity` — **≤ 5 ns/op**.
   - `bench_set_component_raw` — **≤ 18 ns/op**.
   - `bench_iter_entities_dense_10k` — parity with current.
   - `bench_iter_entities_sparse_post_churn` — documented baseline.
   - `bench_create_entity_10k` — **≤ 5 % regression**.
   - `bench_get_component_stale_generation` — **≤ 8 ns/op**.
   - `bench_get_component_missing_component` — **≤ 10 ns/op**.
   - `bench_create_1000_archetypes_no_stack_overflow` — completes.

**Acceptance:**

- All metrics in the D10 table met or documented.
- `cargo bench --bench random_access` runs end-to-end.
- `cargo +nightly miri test phase7_miri_*` clean.

## Critical SAFETY contracts (cross-referenced to detailed plan)

| ID | Topic | Plan §|
|----|-------|-------|
| U1 | Slab address stability | D1 |
| U2 | Slab slot lifetime ⊇ EcsMaster lifetime | D1 |
| U3 | Generation matches Entity ⇔ alive | D2 |
| U4 | EntityInland 16 B layout | D2 |
| U5 | Column 16 B layout, all-zero = null | D4 |
| U6 | Pool buffer ptr/stride write-once at add_pool | D4 |
| U7 | refresh_column called only on add_pool | D4 |
| U8 | iter_occupied_ptrs sees occupied bit ⇒ initialized slot | D1 |
| U9 | unit_index < archetype.current_index for alive entity | D3 |
| U10 | Pool buffer remains stable across pushes | D4 |
| U11 | Pointer minting via raw arithmetic only — no &mut MaybeUninit reborrow | D5 |
| U12 | Drop walks occupancy bitset; calls drop_in_place per occupied slot | D1 |
| U13 | In-place construction via addr_of_mut!.write() | D1 |
| U14 | archetype_ptr cast to &mut for create_entity inside &mut EcsMaster | D5 |

Every invariant has a matching `// SAFETY:` block at the call site
spelling out which invariant it relies on. **No `unsafe` block lands
without one.**

## Risks and mitigations

| Risk | Mitigation |
|------|------------|
| Stacked Borrows retag UB on pointer minting | Recipe D5 + Miri test `phase7_miri_archetype_ptr_no_retag_ub`. |
| Stack overflow on `add_archetype` (8.4 KB struct) | W6 fix: `write_bytes` + per-field `addr_of_mut!.write()`, never construct stack-allocated `Archetype`. |
| Drop discipline regression (orphaned `drop_fn` calls) | C7 fix: explicit Drop body + Miri test for the case. |
| ABA via `u32` generation wrap | 13.6 years per slot at 10 dealloc/sec — accepted, documented. |
| Cold-path regression on `create_entity` | Bench `bench_create_entity_10k`; 5 % budget. |
| Bundle capacity overflow | `Result<u16, BundleFullError>` at bundle layer; `expect(...)` at the upper layers (Phase 7 design). `try_create_archetype` API deferred. |

## Out of scope (deferred to later phases)

- **Mutable iterators / arities ≥ 3** — Phase 2d-extension.
- **System API surface** (`SystemParam`, query DSL) — Phase 8.
- **Parallel scheduler** — Phase 9.
- **Change detection** (`Changed<T>`, `Mut<T>`) — Phase 10 backlog.
- **`EcsError::ArchetypeBundleFull`** — only when a Result-returning
  `try_create_archetype` API is requested. Phase 7 panics instead.

## Cross-phase dependencies

- **Phase 5c dual-generation `QueryState`** is **preserved** —
  Phase 7 does not change `QueryState` semantics.
- **Phase 6 event dispatcher** layout is independent — Phase 7
  does not touch `EventDispatcher` or `EventBuffer`.
- **Phase 4c (C-004 full)** is paused **until after Phase 7** —
  see PHASE-04-architecture-refactors.md for rationale.

## How to launch implementation

When the user explicitly says *"go ahead with Phase 7 implementation"*
(or equivalent), the orchestrator should:

1. Re-read this file and the detailed plan to refresh context.
2. Dispatch `developer` with the 10-step checklist above as the brief.
3. Cycle developer ↔ code-reviewer for each step.
4. After step 10, dispatch `tester` for the bench harness.
5. Cycle `results-analyst` for the final verdict.

The developer must commit per step, never batching. Each commit:
- Author: `Celtokisa <bluesteelll@hotmail.com>` (no `Co-Authored-By`).
- Compiles cleanly.
- Passes `cargo test --all-targets`.
- Includes any new tests needed for that step.

## References

- Detailed plan:
  [`docs/PHASE-7-FAST-RANDOM-ACCESS-PLAN.md`](../PHASE-7-FAST-RANDOM-ACCESS-PLAN.md).
- User feedback driving prioritisation:
  [`feedback-foundations-before-apis`](../../../../../Users/flint/.claude/projects/D--claude-BoykoEngine/memory/feedback-foundations-before-apis.md)
  (memory file).
- Source files to be modified — listed per step above.
- Plan revision commits: `e5e271f` (R1), `260d3a3` (R2),
  `2e73d55` (R2 follow-up).
