# boyko-engine - Phase 2+ Roadmap

> **SUPERSEDED (2026-06-10).** This roadmap predates the phase-based planning that
> followed (Phases 6-19, X.A-X.E, F4 - all landed; see
> [PHASE-13-ROADMAP.md](archive/PHASE-13-ROADMAP.md) for the final status). Live audit
> findings were either fixed in later phases or re-triaged in the post-14b
> bug-backlog cleanup. Kept as a historical record of the 2026-05-23 audit
> sequencing - do NOT use it as a source of "what's next".

Forward-looking implementation plan covering everything from the
2026-05-23 audit (`docs/AUDIT-2026-05-23.md`) that has not been
addressed by Phase 1a / Phase 1b commits on the `ecs` branch.

Authoritative source for "what's next" â€” read this first when starting
any new session targeting the `ecs` branch. The audit is the source
of truth for findings; this file is the source of truth for
*sequencing*, *grouping*, *dependencies*, and *expected effort*.

---

## Current status (as of last commit on `ecs`)

- 28 commits on `ecs` branch (M-012 adds 3: 12a-12c).
- `cargo check --all-targets`: green, 0 errors.
- `cargo test --all-targets`: **152/153 debug** (was 138; +14 new M-012 tests, 1 ignored stress test).
- Author: `Celtokisa <bluesteelll@hotmail.com>`. No AI co-author tags.
- All artifacts in English.
- Q-011 (QueryState cache): **DONE**. Warm-path ~21x speedup measured (3.6 ns vs 77 ns).
- M-012 (HashMap -> BTreeMap in MemFreeBlockMaster): **DONE**. 14 unit tests added; bench infrastructure in `benches/allocator.rs`. Baseline numbers to be captured post-merge.

### Closed by Phase 1a / 1b

| Audit ID | Title | Phase |
|----------|-------|-------|
| C-001 | Self-referential `EcsMaster` (dangling `NonNull<Arena>`) | 1a |
| M-001 | `Arena` without `Drop` (64 MB leak per `EcsMaster::new`) | 1a |
| M-001 cont. | Type-erased component Drop never called | 1b |
| M-002 / C-002 / Q-004 / Q-010 | `static mut` race in component & event registries | 1a |
| C-003 / M-015 | `ComponentId` collision across parallel-compile units | 1b |
| Q-005 | `EventId` collision (mirror C-003) | 1b |
| C-004 partial | TypeId-check on `add` / `set_component` (typed API only) | 1b |
| M-003 / C-018 | `UnsafeCell` aliasing notes, missing SAFETY comments | 1a |
| M-004 | `swap_remove` without Drop, second-chunk dirty marker | 1b |
| M-008 | Dead fields in `Arena` (`cursor` removed, `capacity` used in debug-assert) | 1a |
| C-008 | `debug_assert!(pop_entity())` skipped pool pop in release | 1a |
| Q-002 | `ptr::read` on byte-aligned `Vec<u8>` buffer | 1a |
| Q-022 | `Archetype::pop` did not shrink `entity_ids` | 1a |
| Q-026 partial | `#[inline(always)]` on `Component` / `Event` trait defaults | 1b |
| 14 Ã— E0133 | Rust 2024: unsafe ops inside unsafe fn without explicit unsafe block | 1a |
| Q-001 | Event derive layout-mismatch UB | 1b-finish |
| Q-002 (re-check) | `from_bytes` unaligned read â€” resolved via redesign | 1b-finish |
| Q-017 | `ParticipantBuffer::push` double allocation | 1b-finish (absorbed by Q-001) |
| W6 | Dead `push_raw` / `get_raw` on both buffers | 1b-finish (absorbed by Q-001) |

**Counts**: 16 critical (of 27) closed, 0 important (of 37), 1 informational (of 25), plus 14 E0133.

### Remaining open (~70 findings)

- ðŸ”´ Critical: 14 (was 15; Q-001 closed)
- ðŸŸ¡ Important: 37
- ðŸŸ¢ Informational: 24

Grouped into Phases 1b-finish through 5 below.

---

## Phase 1b â€” Finish â€” CLOSED

Q-001 has been implemented and merged (6 substeps: 6aâ€“6d committed, 6eâ€“6f absorbed into 6d
commit). Phase 1b is fully closed.

### Q-001 â€” Event derive layout-mismatch UB â€” CLOSED

- **Status**: âœ… CLOSED. Commits on `ecs` branch: `c12cba7` (6a), `a618e6e` (6b), `5f35c70` (6c), `6ba2d38` (6d/6f).
- **Category**: ðŸ”´ Critical memory safety (UB in macro-generated cast)
- **Audit ref**: `AUDIT-2026-05-23.md` Â§ Q-001 (line ~187)
- **Approved strategy**: Strategy (a) â€” `#[event]` attribute macro
  rewrites the user struct into `{ participants, parameters }`
  native nested fields. Zero unsafe in accessors. Removes the
  `self as *const Self as *const Self::Participants` UB cast.
- **In scope (absorbed)**:
  - **Q-017** â€” `to_bytes() -> Vec<u8>` double allocation in buffer push â€” CLOSED
  - **W6 from previous critic** â€” `get_raw` / `push_raw` removal (zero callers) â€” CLOSED
- **Out of scope (deferred)**:
  - **Q-019** â€” `ParticipantBuffer` lacks TypeId check on `get` â€” deferred to Phase 4b
- **Implementation order**: 6 substeps (6a-6f), each ending with a
  compilable tree and passing tests
- **Effort estimate**: 1 focused session (architect Round 3 â€” small
  clarifications, critic Round 3, developer, reviewer, tester,
  results-analyst)
- **Files**: `boyko_macros/src/lib.rs`, `events/event.rs`,
  `events/participants/{participants,_buffer}.rs`,
  `events/parameters/{parameters,_buffer}.rs`,
  `events/event_registry.rs`, `tests/derive_event.rs`

---

## Phase 2 â€” Hot path performance

Target the `cargo bench`-relevant findings. None of these are
correctness blockers â€” the engine works today â€” but each one
violates CLAUDE.md principles #1 (zero overhead), #5 (minimum
allocations), or #3 (cache optimization). Together they account
for the difference between "ECS that builds" and "ECS that
competes with Bevy on per-frame microbench".

**Prerequisite for this entire phase**: wire `criterion` into
`crates/boyko_ecs/Cargo.toml` `[dev-dependencies]` and create
`crates/boyko_ecs/benches/` with at least one harness file.
Without baseline numbers, every "fix" in this phase is a guess.

### Phase 2a â€” Eliminate hot-path Vec allocations (10 findings)

These all violate "no `Vec::new()` in hot path" from CLAUDE.md.
Each `Vec` allocation in the per-frame loop is ~30-60 ns of malloc
+ free. At 100k entities / 60 fps, eight allocations per entity
per frame = 0.5-1 ms of pure malloc overhead.

| ID | Site | Fix | Status |
|----|------|-----|--------|
| **Q-011** | `Query::with_*` rebuilds `Vec<&Archetype>` per call | `QueryState` cache + archetype-generation tracking (Bevy pattern) | âœ… **DONE** (2026-05-23, ~21x warm-path speedup measured) |
| **Q-012** | `ComponentSet::component_ids() -> Vec<ComponentId>` | `&'static [ComponentId]` or const generic `[ComponentId; N]` | âœ… **DONE** (2026-05-24) |
| **Q-013** | `find_archetypes_with_*` 4 allocations per call | `SmallVec` or reusable scratch buffer | âœ… **DONE** (2026-05-24) |
| **Q-015** | `MultiPoolSparseIter::next_raw` per-entity `Vec<ComponentPtr>` | tuple via generics or `[ComponentPtr; N]` | âœ… **closed by Phase 2c** (orphan deleted) â€” subsumed by Phase 2d |
| **Q-016** | `SparseIter::new` `Vec<usize>::collect(0..N)` | `Range`-based iterator, no materialization | âœ… **closed by Phase 2c** (orphan deleted) â€” subsumed by Phase 2d |
| **Q-017** | `ParticipantBuffer::push` double allocation | Absorbed by Q-001 (Phase 1b finish) | âœ… closed |
| **C-010** | `EcsMaster::create_entity(Vec<(ComponentId, &[u8])>)` API allocates per call | Builder pattern OR const-generic fixed-size args | open |
| **M-019** | `ComponentPool::get_chunk_component_pointers -> Vec<*const u8>` | Return `&[Unit]` slice | open |
| **C-012 / C-013** | `EntityMaster::iter_entities` is O(N total) for K active | Use dense-iteration over `SparseMap` | open |
| **C-015** | `ArchetypeRegistry::len()` recomputes count; double loops in `unregister_archetype` | Cache `total_count`, reverse map `ArchetypeId â†’ pattern`, don't `.clone()` | open |

### Q-011 â€” QueryState cache + archetype-generation tracking â€” DONE âœ…

- **Status**: âœ… DONE. Commits on `ecs` branch: `11aeef8` (11a), `150501b` (11b), `1201c59` (11c), `5ad2603` (11d).
- **Measured speedup**: ~21x on warm path vs one-shot `Query::with_component_ids` (3.6 ns vs 77 ns, `query_state_iter` vs `query_iter/entity_count`, N=10k/100k on Windows x86_64).
- **Implementation**:
  - `ArchetypeGeneration`: monotonic `NonZeroUsize`, bumped on every `create_archetype`, never reset by `clear()`.
  - `ArchetypeBitSet`: 1024-bit inline bitset (128 B, no heap) for O(1) dedup.
  - `QueryState`: `#[repr(C, align(64))]`, hot fields in cache line 0, Bevy-style `&mut update + &self iter` split.
  - `Query<'a>`: now delegates to a one-shot `QueryState`; stores `Vec<ArchetypeId>` (eliminates stale-ref UB from `swap_remove`).

**Effort estimate**: 3-5 sessions for remaining. Q-011, Q-012, Q-013 done.
Others are localized.

### Q-012 â€” ComponentSet::component_ids() static slice â€” DONE âœ…

- **Status**: âœ… DONE. Commit `bd63b7b` (12a).
- **Implementation**:
  - `()` returns `&[]` (zero cost, no heap).
  - Single-component types: `SINGLE_COMPONENT_CACHE[component_id]` â€” a global
    `[OnceLock<&'static [ComponentId]>; 512]` array. Lock-free after first init
    per component type. Pointer-stable across calls.
  - Tuple types (arity 2â€“8): `Box::leak` per call. Rust does not create
    per-monomorphization statics for generic fn bodies; a shared OnceLock would
    cache the first tuple's IDs for all distinct tuples â€” a correctness bug.
    Query construction is not the per-frame hot path (QueryState caches archetype
    IDs), so one small heap alloc per `Query::with::<T>()` is acceptable.
  - 5 call sites in `query.rs` / `query_state.rs` adjusted (removed `& / let` bindings).
  - 5 unit tests in ID range 495-499.

### Q-013 â€” find_archetypes_with_* allocations â€” DONE âœ…

- **Status**: âœ… DONE. Commits `cf159c6` (13a), `33744d0` (13b), plus 13c in this session.
- **13a**: `find_archetypes_with_few_components_into`: stack `[u8; 3]` + inline
  insertion-sort-with-dedup replaces `Vec::with_capacity + sort_unstable + dedup`.
  Also uses `signature.contains(&query)` instead of re-iterating `components`.
- **13b**: `_into` siblings for all 5 registry `find_*` methods and all 4
  `ArchetypeMaster` wrappers. Original methods are now thin wrappers; backward
  compat preserved. `find_with_filter_into` uses `retain()` for in-place filtering.
- **13c**: `query_one_shot` bench group added to `benches/query_iter.rs` covering
  `with_typed` (Q-012 cache + registry scan) and `find_into` (zero-alloc steady state).

### Phase 2b â€” Cache / layout improvements (4 findings)

| ID | Site | Fix |
|----|------|-----|
| **M-012** | `start_map: HashMap<usize, usize>`, `end_map: HashMap<usize, usize>` in `MemFreeBlockMaster` | `BTreeMap` for ordered key access, or `Vec<Option<usize>>` | âœ… **DONE** (2026-05-24) |
| **M-018** | `O(N)` linear search in `mem_size_tree` indices vec | `HashSet` or reverse mapping |
| **C-016** | `O(NÃ—M)` "missing component" check in `Archetype::create_entity` | `ComponentMask` comparison against signature (O(8) for 512 bits) |
| **M-007** | `Box<[ComponentPtr]>` returned per-entity from `MultiPoolSparseIter` | âœ… **closed by Phase 2c** (orphan deleted) â€” subsumed by Phase 2d zero-alloc design |

**Effort estimate**: 2-3 sessions. M-012 is the largest (changes
the `MemFreeBlockMaster` internal data structure).

### Phase 2c â€” Infrastructure (prerequisite) â€” CLOSED âœ…

| Task | Detail | Status |
|------|--------|--------|
| Wire `criterion` | `boyko_ecs/Cargo.toml` `[dev-dependencies]`; `benches/component_id.rs`, `benches/swap_remove.rs`, `benches/query_iter.rs` | âœ… Done |
| GitHub Actions CI | `.github/workflows/ci.yml` â€” check / test / clippy / bench-compile / miri | âœ… Done |
| Install nightly + Miri | `cargo +nightly miri setup`; CI runs `event_attribute` + `drop_fn` (clean) then full sweep (informational) | âœ… Done (CI) |
| (optional) `loom` | For lock-free patterns introduced in Phase 4 | deferred |

---

## Phase 3 â€” Memory safety hardening (remaining)

Findings that don't block normal operation but harden the engine
against edge cases, future refactors, and adversarial inputs.

### Phase 3a â€” Entity lifecycle correctness (3 findings) â€” DONE âœ…

| ID | Site | Issue | Fix | Status |
|----|------|-------|-----|--------|
| **C-006** | `EcsMaster::delete_entity` | Fragile swap-update logic; `Option<EntityId>` ambiguous (3 meanings) | `enum RemoveOutcome { Last, Swapped { moved_entity }, PoolFailure }` | âœ… DONE |
| **C-007** | `EcsMaster::create_entity` early return after `allocate_entity` | EntityId leaked on `?` in `get_archetype_mut` failure | Guard pattern: validate archetype before allocate; `rewind_allocate` on failure | âœ… DONE |
| **C-009** | `ComponentPoolBundle::add_entity_components` | No rollback on partial-pool failure â†’ pool desync | Two-phase commit: `can_push_entity_components` + `push_entity_components` | âœ… DONE |

**Implementation** (4 commits):
- Miri retag fix: `NonNull<Arena>` â†’ `*const Arena` raw provenance throughout.
  `.cargo/config.toml`: `MIRIFLAGS=-Zmiri-tree-borrows` (Tree Borrows required
  for `UnsafeCell`-containing `Arena` with raw pointer aliasing).
- C-006: `RemoveOutcome` enum in `archetype.rs`; compile-time size assertion.
- C-009: `can_push_entity_components` + `push_entity_components`; deleted
  `add_entity_components` (zero callers).
- C-007: `EntityMaster::rewind_allocate` (pub(crate)); guard in
  `EcsMaster::create_entity`; `ArchetypeMaster::has_archetype`.
  CI `continue-on-error` removed from Miri job (Phase 3a Miri gate closed).

**Test delta**: +19 total (4 C-006 + 5 C-009 + 5 C-007 + 5 rewind_allocate tests).
**Miri**: all tests pass under Tree Borrows (`-Zmiri-tree-borrows`).

### Phase 3b â€” Iterators / Containers cleanup (3 findings)

| ID | Site | Issue | Decision needed |
|----|------|-------|-----------------|
| **Q-008** | `core/iters/sparse_iter.rs`, `memory/multi_pool_sparse_iter.rs`, `memory/sparse_iter_component_pool.rs`, `core/containers/*` | Orphan files: not in `mod.rs`, have compile errors when included, documented as "âœ… implemented" in `FEATURE_MAP.md` | âœ… **DONE â€” Phase 2c** (2026-05-24): all 4 orphan files deleted, `memory/iterators.rs` stub removed, `pub mod iterators;` unwired from `memory/mod.rs`, `FEATURE_MAP.md` / `SYSTEMS.md` / `ARCHITECTURE.md` corrected. Per-entity iter replacement tracked as Phase 2d (see below). |
| **Q-009** | `SparseIter::next_raw` recursive call without TCO | Stack overflow risk on 10k+ empty archetypes | âœ… **closed by Phase 2c** (orphan deleted) â€” subsumed by Phase 2d (loop-based design required from the start). |
| **M-010** | `boyko_utils/bit_mask/{bit_mask, bit_set512, bit_storage}.rs` fully `/* commented out */` | 1000+ lines dead code; `SYSTEMS.md` claims they exist | âœ… **DONE â€” Phase 5b** (2026-05-24): 3 files (1080 LOC) deleted; `mod bit_storage;` unwired. Consistent with Q-008 / Q-024 / Q-025 cleanups. |

### Phase 2d â€” Per-entity query iter (Q-026 + Q-008-replacement)

**Status**: âœ… **DONE (bounded)** â€” Phase 2d core landed 2026-05-24. Two read-only methods on `Query<'a>` ship:

  - `iter_one<A: Component>() -> QueryIterOne<'_, A>` yielding `&A`
  - `iter_two<A, B: Component>() -> QueryIterTwo<'_, A, B>` yielding `(&A, &B)`

Zero-alloc per row (pointer-bump cursors + `remaining` counter), archetype-major order, loop-based (no recursion). Reuses `QueryState::matched_ids()` cache from Phase 2a. Six new tests in `query.rs` lock down the contract. `ComponentPool::buffer_ptr()` accessor added with SAFETY contract.

**Open (Phase 2d-extension, separate tickets):**

| Subticket | Scope | Notes |
|-----------|-------|-------|
| `iter_one_mut` / `iter_two_mut` | Mutable variants | Needs aliasing-discipline rework â€” `&mut` references can't be juggled the same way as `&` in the pointer-bump pattern; will need either internal-only `*mut` + lifetime gymnastics, or splitborrow per row. |
| Arities â‰¥ 3 | `iter_three`, `iter_four`, ... up to N=8 or N=12 | Mechanical extension of the 2-arity template; can be either a macro-emitted family or a generic `Fetch` trait. |
| `Query::iter::<(&T, &U)>` | Generic tuple-trait pattern (Bevy `WorldQuery` / hecs `Fetch`) | Replaces the named-arity methods with a unified entry. Requires variadic-tuple trait impls. Likely Phase 2d-final once arities and mut variants settle. |
| Per-entity change-detection / filter combinators | `With<T>`, `Without<T>`, `Changed<T>` | Out of Phase 2d entirely â€” depends on Phase 3+ change-detection infrastructure. |

**Why bounded first step**: get the user-visible API in front of consumers immediately at 2-arity (covers the vast majority of `(Position, Velocity)`-style game loops), then iterate on the trait machinery without blocking real usage. The pointer-bump skeleton is the load-bearing piece; mut/arity extensions are mechanical follow-ons.

**Effort estimate**: Q-009 is 30 minutes. Q-008 + M-010 each block
on a product decision; once decided, 1-3 sessions depending on
direction.

### Phase 2e â€” ComponentTuple ergonomic bundle API (Q-024-future)

**Status**: open â€” design pending.

| ID | Site | Issue | Solution direction |
|----|------|-------|--------------------|
| **Q-024** | `containers/tuple/` (removed) | Orphan 0-byte stubs `component_tuple.rs` + `component_tuple_trait.rs` were never wired into `mod.rs`; removed in Q-024 cleanup | Design `ComponentTuple`: a type-safe tuple wrapper that maps to `ComponentId` bundles, enabling ergonomic `world.spawn((Position { .. }, Velocity { .. }))` API without raw byte slices. Mirror Phase 2d's bounded approach: ship `spawn_one<A>(arch_id, A)` and `spawn_two<A, B>(arch_id, (A, B))` first, generalize via tuple trait later. Cycle: researcher â†’ architect â†’ critic â†’ developer â†’ tester â†’ results-analyst. |

**Dependency**: same tuple-over-generics infrastructure as Phase 2d. The current bounded `iter_one` / `iter_two` could be left as-is; Phase 2e adds parallel `spawn_one` / `spawn_two` for symmetry.

**Effort estimate**: 1 bounded session for 1-2 arity, 1 more if generalizing the trait.

### Phase 3c â€” Unit / pool internals (3 findings)

| ID | Site | Issue | Fix |
|----|------|-------|-----|
| **M-005** | `Unit { ptr: *mut u8, buffer_index: usize }` | `buffer_index` never read â€” wastes ~50% of unit-index memory | Remove field; `Vec<NonNull<u8>>` instead of `Vec<Unit>` |
| **M-006** | `Unit { ptr: *mut u8, .. }` | No `PhantomData<&'a mut [u8]>`; allows use-after-free of Unit outliving Arena | Add lifetime parameter or remove Unit type per M-005 |
| **M-016** | `SparseSlotMap::remove` | ABA prevention is broken â€” `new_generation` computed but never stored | Store generation separately or use sentinel-Slot |

**Effort estimate**: 1 session for all three. M-005 is the
"prerequisite" â€” it deletes the `Unit` type, after which M-006 is moot.

### Phase 3d â€” Event lifecycle (1 finding)

| ID | Site | Issue | Status |
|----|------|-------|--------|
| **Q-007** | `EventPool::clear`, `swap_remove`, `Drop` | No `drop_fn` invocation on stored events | **Blocked**: `event_pool.rs` and `event_pool_bundle.rs` are fully commented out in the codebase. When EventPool is uncommented in a future product decision, apply the same `drop_fn` pattern from `ComponentPool` (M-001 cont. / M-004). |

---

## Phase 4 â€” Architectural refactors

These change public API surface. Each one is a discussion topic
with the user before implementation â€” none are strictly "fixes",
they are design improvements.

### Phase 4a â€” Type-safety wins (3 findings)

| ID | Site | Fix |
|----|------|-----|
| **C-017** | `EntityId`, `ArchetypeId`, `ComponentId`, `Generation` all type-alias to `usize` â€” no compile-time prevention of mixups | Newtype wrappers with `#[repr(transparent)]`; constructors via `pub(crate)` |
| **C-019** | `EcsMaster` returns `anyhow::Result` from library API | Domain-specific `enum EcsError { ArchetypeNotFound(ArchetypeId), ComponentNotRegistered(ComponentId), ... }`; remove `anyhow` from `boyko_ecs/Cargo.toml` |
| **C-023** | `ComponentPool.chunks: pub Vec<Chunk>`, `ArchetypeSignature.{mask, block_summary, section_summary}: pub`, `Entity.{id, generation}: pub`, `ComponentMask.blocks: pub` | Private fields + accessor methods; constructors that maintain invariants |

**Effort estimate**: 2-3 sessions (each is small per-file but
touches many call sites).

### Phase 4b â€” Event system review (2 findings)

| ID | Site | Question | Approach | Status |
|----|------|----------|----------|--------|
| **Q-020** | `Participants` and `Parameters` split â€” overengineered? | Architectural decision: keep split (current design â€” addressed via Q-001 native nested fields) OR collapse into single Event type (Bevy style). | **DEFERRED** â€” see decision note below. |
| **Q-019** | `ParticipantBuffer::get<P>` lacks TypeId check | Store `TypeId` in buffer alongside `participant_size`; `debug_assert_eq!` in `get`. | âœ… **DONE â€” Phase 4a** (2026-05-24): both `ParticipantBuffer` and `ParametersBuffer` carry `TypeId` and `debug_assert_eq!` on typed access. 8 new tests cover correct round-trip and wrong-type panics. |

**Q-020 deferral rationale (2026-05-24)**: the split survives because Q-001 already made it sound (native nested fields, no UB cast) and Q-019 already guards type confusion. The audit's "overengineered" framing assumed a Bevy-style subscriber model where events filter by participant set at dispatch time â€” `boyko_ecs` does not implement that filtering today and has no committed timeline for it (event dispatch / subscriber registration are not in any open phase). Collapsing the split now would buy zero functional simplification on the consumer side while costing:
  - one breaking macro change (`#[event]` would have to be rewritten or replaced)
  - migration of every `Event::Participants` / `Event::Parameters` assoc-type consumer
  - documentation churn across `SYSTEMS.md` and the public mdBook

Reopen this ticket the moment a real use case for participant-filtered dispatch appears (or a competing audit demands it). Until then, the current design is preserved without further work.

**Effort estimate**: Phase 4b closed (Q-019 done, Q-020 deferred-by-decision).

### Phase 4c â€” Remaining type-erasure hardening (1 finding)

| ID | Site | Fix |
|----|------|-----|
| **C-004 full** | Raw `ComponentPool::add(&[u8])` and `set_component(idx, &[u8])` still bypass TypeId checks (only typed API checks) | Migrate all internal callers (`Archetype::create_entity`, bulk APIs) to typed paths. Once raw API has no internal users, mark `pub(crate)` or remove. |

**Dependency**: requires C-010 (Phase 2a â€” `create_entity` API
redesign) since the bulk byte-API is the main internal caller.

---

## Phase 5 â€” Cleanup, style, micro-perf

Low-risk hygiene work. Not blocking anything but improves
maintainability and `cargo clippy --all-targets -- -D warnings`.

### Phase 5a â€” Inline policy sweep (3 findings)

| ID | Site | Fix |
|----|------|-----|
| **C-014 / M-020 / Q-029** | `#[inline(always)]` on required trait methods, trivial getters, `*_unchecked` accessors throughout the codebase. ~200 clippy pedantic warnings on this. | Replace with plain `#[inline]` unless profiler shows measurable improvement. Per principle #7: measured inlining only. |

**Effort estimate**: 1 focused session. Mechanical pass.

### Phase 5b â€” Component mask boundary (2 findings)

| ID | Site | Fix |
|----|------|-----|
| **M-009 / C-011** | `ComponentMask::set/unset/contains` uses `(component_id / 64) % 8` â€” silently wraps for `component_id >= 512` | Add `debug_assert!(component_id < 512)` at top; remove `% 8` (the assert makes it unnecessary) |

**Effort estimate**: 30 minutes.

### Phase 5c â€” Dead / duplicate code (3 findings)

| ID | Site | Fix |
|----|------|-----|
| **M-013** | `MemFreeBlockMaster::defragment` builds an `index_map: HashMap<usize, usize>` and never reads it | Delete the dead variable, or wire it into a notify callback |
| **M-017** | `SparseMap::swap_remove` and `SparseMap::pop_swap_remove` are byte-identical duplicates | Delete one; have the other call it |
| **Q-021** | `Archetype::create_entity` does not check for duplicate `ComponentId` in input | `debug_assert!` via `ComponentMask` building |

**Effort estimate**: 1 session.

### Phase 5d â€” Minor style / API ergonomics (10+ findings)

| ID | Description |
|----|-------------|
| **C-020** | `is_entity_valid` 3-way check; reorder for cache locality |
| **C-021** | `EntityMaster::with_capacity(capacity / 4)` magic constant â€” document or constify |
| **C-022** | `wrapping_add(1)` on generation â€” theoretically ABA after 2^64 ops; `debug_assert!(new_gen != 0)` |
| **C-024** | `ArchetypeMaster::add_pool(&self, arena: &Arena, ...)` â€” arena passed even though `ArchetypeMaster` holds `NonNull<Arena>` |
| **C-025** | Logic duplication between `EcsMaster::query_entities` / `ArchetypeMaster::find_*` / `Query` |
| **C-026** | Comments describe "what" not "why" in several places |
| **C-027** | Tests depend on global registry â€” flaky under `cargo test` parallelism (partly addressed by ID range isolation but full fix requires per-test reset) |
| **C-028..C-031** | Micro-opts: `ComponentMask::is_empty` recreation, `block_groups` preallocate, `NonNull::as_ref` vs `as_ptr` consistency |
| **Q-018** | `EventPool::clear` and `swap_remove` inconsistency on Drop semantics (when uncommented) |
| **Q-023** | `EventPool` race in `clear` (when uncommented) |
| **Q-026** | `EventId = u64` while `MAX_EVENTS = 256` â€” narrow to `u16` or `u8` |
| **Q-027** | `with_filters(include, exclude, optional)` â€” three `ComponentMask` args in a row, easy to confuse â€” builder pattern |
| **Q-028** | API ergonomics for query construction |
| **M-021..M-027** | Misc: `Arena::allocate_layout` panics instead of `Result`; `align_up` no overflow check; orphan-file references; mixed comment languages (now fixed); `Unit.buffer_index: usize` could be `u32` (resolved by M-005) |

**Effort estimate**: 2-3 sessions, mechanical work, can be batched.

---

## Suggested execution order

### Completed
- **Phase 1b finish â€” Q-001** âœ… (commits `c12cba7`, `a618e6e`, `5f35c70`, `6ba2d38`)
- **Phase 2c infrastructure** âœ… (`criterion` wired, CI workflow, docs updated)

### Next session

### Sessions 2-4
3. **Phase 2a hot-path Vec allocations** â€” start with **Q-011 QueryState
   cache** (biggest impact, hardest). Then batch the smaller ones.
4. **Phase 2b cache/layout** â€” `M-012` HashMap â†’ BTreeMap is the
   single biggest cache win. Do it standalone with benchmarks.

### Sessions 5-7
5. **Phase 3a entity lifecycle** (C-006, C-007, C-009)
6. **Phase 3b orphan files decision** (Q-008, M-010 â€” user input required)
7. **Phase 3c unit/pool internals** (M-005, M-006, M-016)

### Sessions 8-10
8. **Phase 4a type-safety** (C-017, C-019, C-023)
9. **Phase 4b event system review** (Q-019, Q-020 â€” design discussion first)
10. **Phase 4c C-004 full** (after Phase 2a C-010 lands)

### Session 11+
11. **Phase 5 cleanup** â€” batch into 2-3 sessions, mechanical work.

### Permanently deferred / scoped out
- **Q-007** (EventPool Drop): blocked on uncommenting `event_pool.rs` â€”
  not Phase 1-5 work.
- **Public mdBook + audit-report Russian â†’ English translation**:
  tracked as task #36 in TaskList, low priority.

---

## Cross-cutting concerns

### Test isolation
The global `OnceLock<ComponentLayout>` registry is shared across
test binaries. Phase 1a fixed inter-test pollution by partitioning
ID ranges (`ecs_master: 100-109`, `query: 200-209`, `archetype_master:
300-309`, `archetype: 400-409`). Future tests must claim a new range:
- `entity_master`: 450-459 (reserved)
- `events` integration: 500-509 (reserved)
- `drop_fn` integration: 600-699 (already used by drop_fn tests)

Document this convention in `component_registry.rs` module doc.

### Documentation policy
Per `feedback-language-english-only`: every artifact written into the
repository is in English. Chat with the user is in Russian. This
applies to roadmaps, audit reports, commit messages, doc-comments,
inline comments, SAFETY blocks, panic messages, expect strings.

`docs/AUDIT-2026-05-23.md` and `book/src/*` remain Russian by intentional decision (task #36):

  - `AUDIT-2026-05-23.md` is a **historical snapshot** of the codebase on the
    audit date. Translating it would mutate the historical record. Per-finding
    translations are integrated into the matching closure commit's message
    (in English), so a reader following an audit ID has English context at
    each fix's commit. The original Russian audit stays as the authentic
    source-of-truth document.
  - `book/src/*` is public-facing mdBook content. Its eventual English
    translation is the `doc-writer` agent's responsibility once the public
    API stabilises (the surface still shifts every phase â€” translating now
    means re-translating after every API rename). Tracked as a single
    follow-up batch task rather than a per-page deferment.

Neither blocks `ecs` correctness or CI. Status: **intentionally deferred** until either (a) `doc-writer` is dispatched explicitly for the book pass, or (b) the audit reaches "archived / closed" status.

### Git policy
- Commits authored by `Celtokisa <bluesteelll@hotmail.com>` only.
- **No `Co-Authored-By: Claude` tags** â€” ever.
- Commits grouped by logical findings (not per-file).
- Branch: `ecs`. Never `--force` or `--no-verify` without explicit
  user permission.

### Benchmarks budget
After Phase 2c (criterion wired), each Phase 2+ fix declares a target:
- Hot path: â‰¤ documented baseline + 0% (no regression).
- Per-frame allocations: 0 mallocs per entity-iteration tick.
- Cold path (registration, archetype creation): not measured unless
  bottleneck appears.

### Miri pass
After nightly + Miri installed, run `cargo +nightly miri test
--all-targets` at every Phase milestone. Required to catch:
- Padding-byte UB in event byte-buffer round-trips (Q-001 area)
- Aliasing in raw-pointer paths (Pool, Arena)
- ABA in `SparseSlotMap` (M-016 area)
- Drop-order issues in `ComponentPool::Drop` / `Arena::Drop`

---

## Architectural decisions still open

Topics that need explicit user buy-in before architect spends a
round on them:

1. **Q-008** â€” Orphan iterator/container files: implement or delete?
2. **M-010** â€” Commented-out `BitMask` / `BitSet512` / `BitStorage`:
   resurrect or delete?
3. **Q-020** â€” `Participants` / `Parameters` trait split: keep or
   collapse after Q-001 lands?
4. **C-019** â€” Replace `anyhow::Result` with `EcsError`? (Affects
   public API; one-time cost is high.)
5. **C-017** â€” Newtype wrappers for all ID types? (Affects every
   public signature; touches every test.)
6. **Schedule / scheduler** â€” Beyond the audit: when will
   `boyko-engine` add a `System` abstraction and parallel system
   scheduler? Several Phase 2 fixes (Q-011 QueryState in particular)
   are sized around what the eventual scheduler needs.

---

## How to read this file

- Each phase is a coherent unit of work, ideally one session
  per sub-phase (a, b, c, d).
- "Effort estimate" is for the full pipeline: architect â†’ critic â†’
  developer â†’ reviewer â†’ tester â†’ results-analyst â†’ commit.
- "Audit ID" references `docs/AUDIT-2026-05-23.md` â€” read the audit
  section before starting any fix to get the original problem
  statement and proposed fix.
- Dependencies are explicit ("Phase X depends on Phase Y"). Do not
  start a dependent phase before its prerequisites.

When in doubt, start a session with: "Read
`docs/ROADMAP-PHASE-2-PLUS.md`. We are now working on [phase]."
The roadmap gives the agent context the audit alone cannot.

