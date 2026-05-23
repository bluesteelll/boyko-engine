# boyko-engine — Phase 2+ Roadmap

Forward-looking implementation plan covering everything from the
2026-05-23 audit (`docs/AUDIT-2026-05-23.md`) that has not been
addressed by Phase 1a / Phase 1b commits on the `ecs` branch.

Authoritative source for "what's next" — read this first when starting
any new session targeting the `ecs` branch. The audit is the source
of truth for findings; this file is the source of truth for
*sequencing*, *grouping*, *dependencies*, and *expected effort*.

---

## Current status (as of last commit on `ecs`)

- 25 commits on `ecs` branch (Q-011 adds 5: 11a-11e).
- `cargo check --all-targets`: green, 0 errors.
- `cargo test --all-targets`: **138/138 debug** (was 130, +8 new QueryState tests).
- Author: `Celtokisa <bluesteelll@hotmail.com>`. No AI co-author tags.
- All artifacts in English.
- Q-011 (QueryState cache): **DONE**. Warm-path ~21x speedup measured (3.6 ns vs 77 ns).

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
| 14 × E0133 | Rust 2024: unsafe ops inside unsafe fn without explicit unsafe block | 1a |
| Q-001 | Event derive layout-mismatch UB | 1b-finish |
| Q-002 (re-check) | `from_bytes` unaligned read — resolved via redesign | 1b-finish |
| Q-017 | `ParticipantBuffer::push` double allocation | 1b-finish (absorbed by Q-001) |
| W6 | Dead `push_raw` / `get_raw` on both buffers | 1b-finish (absorbed by Q-001) |

**Counts**: 16 critical (of 27) closed, 0 important (of 37), 1 informational (of 25), plus 14 E0133.

### Remaining open (~70 findings)

- 🔴 Critical: 14 (was 15; Q-001 closed)
- 🟡 Important: 37
- 🟢 Informational: 24

Grouped into Phases 1b-finish through 5 below.

---

## Phase 1b — Finish — CLOSED

Q-001 has been implemented and merged (6 substeps: 6a–6d committed, 6e–6f absorbed into 6d
commit). Phase 1b is fully closed.

### Q-001 — Event derive layout-mismatch UB — CLOSED

- **Status**: ✅ CLOSED. Commits on `ecs` branch: `c12cba7` (6a), `a618e6e` (6b), `5f35c70` (6c), `6ba2d38` (6d/6f).
- **Category**: 🔴 Critical memory safety (UB in macro-generated cast)
- **Audit ref**: `AUDIT-2026-05-23.md` § Q-001 (line ~187)
- **Approved strategy**: Strategy (a) — `#[event]` attribute macro
  rewrites the user struct into `{ participants, parameters }`
  native nested fields. Zero unsafe in accessors. Removes the
  `self as *const Self as *const Self::Participants` UB cast.
- **In scope (absorbed)**:
  - **Q-017** — `to_bytes() -> Vec<u8>` double allocation in buffer push — CLOSED
  - **W6 from previous critic** — `get_raw` / `push_raw` removal (zero callers) — CLOSED
- **Out of scope (deferred)**:
  - **Q-019** — `ParticipantBuffer` lacks TypeId check on `get` — deferred to Phase 4b
- **Implementation order**: 6 substeps (6a-6f), each ending with a
  compilable tree and passing tests
- **Effort estimate**: 1 focused session (architect Round 3 — small
  clarifications, critic Round 3, developer, reviewer, tester,
  results-analyst)
- **Files**: `boyko_macros/src/lib.rs`, `events/event.rs`,
  `events/participants/{participants,_buffer}.rs`,
  `events/parameters/{parameters,_buffer}.rs`,
  `events/event_registry.rs`, `tests/derive_event.rs`

---

## Phase 2 — Hot path performance

Target the `cargo bench`-relevant findings. None of these are
correctness blockers — the engine works today — but each one
violates CLAUDE.md principles #1 (zero overhead), #5 (minimum
allocations), or #3 (cache optimization). Together they account
for the difference between "ECS that builds" and "ECS that
competes with Bevy on per-frame microbench".

**Prerequisite for this entire phase**: wire `criterion` into
`crates/boyko_ecs/Cargo.toml` `[dev-dependencies]` and create
`crates/boyko_ecs/benches/` with at least one harness file.
Without baseline numbers, every "fix" in this phase is a guess.

### Phase 2a — Eliminate hot-path Vec allocations (10 findings)

These all violate "no `Vec::new()` in hot path" from CLAUDE.md.
Each `Vec` allocation in the per-frame loop is ~30-60 ns of malloc
+ free. At 100k entities / 60 fps, eight allocations per entity
per frame = 0.5-1 ms of pure malloc overhead.

| ID | Site | Fix | Status |
|----|------|-----|--------|
| **Q-011** | `Query::with_*` rebuilds `Vec<&Archetype>` per call | `QueryState` cache + archetype-generation tracking (Bevy pattern) | ✅ **DONE** (2026-05-23, ~21x warm-path speedup measured) |
| **Q-012** | `ComponentSet::component_ids() -> Vec<ComponentId>` | `&'static [ComponentId]` or const generic `[ComponentId; N]` | ✅ **DONE** (2026-05-24) |
| **Q-013** | `find_archetypes_with_*` 4 allocations per call | `SmallVec` or reusable scratch buffer | ✅ **DONE** (2026-05-24) |
| **Q-015** | `MultiPoolSparseIter::next_raw` per-entity `Vec<ComponentPtr>` | tuple via generics or `[ComponentPtr; N]` | open |
| **Q-016** | `SparseIter::new` `Vec<usize>::collect(0..N)` | `Range`-based iterator, no materialization | open |
| **Q-017** | `ParticipantBuffer::push` double allocation | Absorbed by Q-001 (Phase 1b finish) | ✅ closed |
| **C-010** | `EcsMaster::create_entity(Vec<(ComponentId, &[u8])>)` API allocates per call | Builder pattern OR const-generic fixed-size args | open |
| **M-019** | `ComponentPool::get_chunk_component_pointers -> Vec<*const u8>` | Return `&[Unit]` slice | open |
| **C-012 / C-013** | `EntityMaster::iter_entities` is O(N total) for K active | Use dense-iteration over `SparseMap` | open |
| **C-015** | `ArchetypeRegistry::len()` recomputes count; double loops in `unregister_archetype` | Cache `total_count`, reverse map `ArchetypeId → pattern`, don't `.clone()` | open |

### Q-011 — QueryState cache + archetype-generation tracking — DONE ✅

- **Status**: ✅ DONE. Commits on `ecs` branch: `11aeef8` (11a), `150501b` (11b), `1201c59` (11c), `5ad2603` (11d).
- **Measured speedup**: ~21x on warm path vs one-shot `Query::with_component_ids` (3.6 ns vs 77 ns, `query_state_iter` vs `query_iter/entity_count`, N=10k/100k on Windows x86_64).
- **Implementation**:
  - `ArchetypeGeneration`: monotonic `NonZeroUsize`, bumped on every `create_archetype`, never reset by `clear()`.
  - `ArchetypeBitSet`: 1024-bit inline bitset (128 B, no heap) for O(1) dedup.
  - `QueryState`: `#[repr(C, align(64))]`, hot fields in cache line 0, Bevy-style `&mut update + &self iter` split.
  - `Query<'a>`: now delegates to a one-shot `QueryState`; stores `Vec<ArchetypeId>` (eliminates stale-ref UB from `swap_remove`).

**Effort estimate**: 3-5 sessions for remaining. Q-011, Q-012, Q-013 done.
Others are localized.

### Q-012 — ComponentSet::component_ids() static slice — DONE ✅

- **Status**: ✅ DONE. Commit `bd63b7b` (12a).
- **Implementation**:
  - `()` returns `&[]` (zero cost, no heap).
  - Single-component types: `SINGLE_COMPONENT_CACHE[component_id]` — a global
    `[OnceLock<&'static [ComponentId]>; 512]` array. Lock-free after first init
    per component type. Pointer-stable across calls.
  - Tuple types (arity 2–8): `Box::leak` per call. Rust does not create
    per-monomorphization statics for generic fn bodies; a shared OnceLock would
    cache the first tuple's IDs for all distinct tuples — a correctness bug.
    Query construction is not the per-frame hot path (QueryState caches archetype
    IDs), so one small heap alloc per `Query::with::<T>()` is acceptable.
  - 5 call sites in `query.rs` / `query_state.rs` adjusted (removed `& / let` bindings).
  - 5 unit tests in ID range 495-499.

### Q-013 — find_archetypes_with_* allocations — DONE ✅

- **Status**: ✅ DONE. Commits `cf159c6` (13a), `33744d0` (13b), plus 13c in this session.
- **13a**: `find_archetypes_with_few_components_into`: stack `[u8; 3]` + inline
  insertion-sort-with-dedup replaces `Vec::with_capacity + sort_unstable + dedup`.
  Also uses `signature.contains(&query)` instead of re-iterating `components`.
- **13b**: `_into` siblings for all 5 registry `find_*` methods and all 4
  `ArchetypeMaster` wrappers. Original methods are now thin wrappers; backward
  compat preserved. `find_with_filter_into` uses `retain()` for in-place filtering.
- **13c**: `query_one_shot` bench group added to `benches/query_iter.rs` covering
  `with_typed` (Q-012 cache + registry scan) and `find_into` (zero-alloc steady state).

### Phase 2b — Cache / layout improvements (4 findings)

| ID | Site | Fix |
|----|------|-----|
| **M-012** | `start_map: HashMap<usize, usize>`, `end_map: HashMap<usize, usize>` in `MemFreeBlockMaster` | `BTreeMap` for ordered key access, or `Vec<Option<usize>>` |
| **M-018** | `O(N)` linear search in `mem_size_tree` indices vec | `HashSet` or reverse mapping |
| **C-016** | `O(N×M)` "missing component" check in `Archetype::create_entity` | `ComponentMask` comparison against signature (O(8) for 512 bits) |
| **M-007** | `Box<[ComponentPtr]>` returned per-entity from `MultiPoolSparseIter` | Const-generic array, tuple, or pre-allocated buffer |

**Effort estimate**: 2-3 sessions. M-012 is the largest (changes
the `MemFreeBlockMaster` internal data structure).

### Phase 2c — Infrastructure (prerequisite) — CLOSED ✅

| Task | Detail | Status |
|------|--------|--------|
| Wire `criterion` | `boyko_ecs/Cargo.toml` `[dev-dependencies]`; `benches/component_id.rs`, `benches/swap_remove.rs`, `benches/query_iter.rs` | ✅ Done |
| GitHub Actions CI | `.github/workflows/ci.yml` — check / test / clippy / bench-compile / miri | ✅ Done |
| Install nightly + Miri | `cargo +nightly miri setup`; CI runs `event_attribute` + `drop_fn` (clean) then full sweep (informational) | ✅ Done (CI) |
| (optional) `loom` | For lock-free patterns introduced in Phase 4 | deferred |

---

## Phase 3 — Memory safety hardening (remaining)

Findings that don't block normal operation but harden the engine
against edge cases, future refactors, and adversarial inputs.

### Phase 3a — Entity lifecycle correctness (3 findings)

| ID | Site | Issue | Fix |
|----|------|-------|-----|
| **C-006** | `EcsMaster::delete_entity` | Fragile swap-update logic; `Option<EntityId>` ambiguous (3 meanings) | `enum RemoveOutcome { Last, Swapped(EntityId), Failed }` |
| **C-007** | `EcsMaster::create_entity` early return after `allocate_entity` | EntityId leaked on `?` in `get_archetype_mut` failure | Guard pattern: allocate after preconditions |
| **C-009** | `ComponentPoolBundle::add_entity_components` | No rollback on partial-pool failure → pool desync | Two-phase commit OR pre-check capacity + atomic add |

**Dependency**: C-007 and C-009 are coupled (both touch `create_entity`).
Best done together. C-006 is independent.

**Effort estimate**: 1-2 sessions.

### Phase 3b — Iterators / Containers cleanup (3 findings)

| ID | Site | Issue | Decision needed |
|----|------|-------|-----------------|
| **Q-008** | `core/iters/sparse_iter.rs`, `memory/multi_pool_sparse_iter.rs`, `memory/sparse_iter_component_pool.rs`, `core/containers/*` | Orphan files: not in `mod.rs`, have compile errors when included, documented as "✅ implemented" in `FEATURE_MAP.md` | **Product decision**: hook up + fix the compile errors, OR delete + update docs |
| **Q-009** | `SparseIter::next_raw` recursive call without TCO | Stack overflow risk on 10k+ empty archetypes | Convert to `loop { match ... }` |
| **M-010** | `boyko_utils/bit_mask/{bit_mask, bit_set512, bit_storage}.rs` fully `/* commented out */` | 1000+ lines dead code; `SYSTEMS.md` claims they exist | **Product decision**: uncomment + integrate, OR delete |

**Effort estimate**: Q-009 is 30 minutes. Q-008 + M-010 each block
on a product decision; once decided, 1-3 sessions depending on
direction.

### Phase 3c — Unit / pool internals (3 findings)

| ID | Site | Issue | Fix |
|----|------|-------|-----|
| **M-005** | `Unit { ptr: *mut u8, buffer_index: usize }` | `buffer_index` never read — wastes ~50% of unit-index memory | Remove field; `Vec<NonNull<u8>>` instead of `Vec<Unit>` |
| **M-006** | `Unit { ptr: *mut u8, .. }` | No `PhantomData<&'a mut [u8]>`; allows use-after-free of Unit outliving Arena | Add lifetime parameter or remove Unit type per M-005 |
| **M-016** | `SparseSlotMap::remove` | ABA prevention is broken — `new_generation` computed but never stored | Store generation separately or use sentinel-Slot |

**Effort estimate**: 1 session for all three. M-005 is the
"prerequisite" — it deletes the `Unit` type, after which M-006 is moot.

### Phase 3d — Event lifecycle (1 finding)

| ID | Site | Issue | Status |
|----|------|-------|--------|
| **Q-007** | `EventPool::clear`, `swap_remove`, `Drop` | No `drop_fn` invocation on stored events | **Blocked**: `event_pool.rs` and `event_pool_bundle.rs` are fully commented out in the codebase. When EventPool is uncommented in a future product decision, apply the same `drop_fn` pattern from `ComponentPool` (M-001 cont. / M-004). |

---

## Phase 4 — Architectural refactors

These change public API surface. Each one is a discussion topic
with the user before implementation — none are strictly "fixes",
they are design improvements.

### Phase 4a — Type-safety wins (3 findings)

| ID | Site | Fix |
|----|------|-----|
| **C-017** | `EntityId`, `ArchetypeId`, `ComponentId`, `Generation` all type-alias to `usize` — no compile-time prevention of mixups | Newtype wrappers with `#[repr(transparent)]`; constructors via `pub(crate)` |
| **C-019** | `EcsMaster` returns `anyhow::Result` from library API | Domain-specific `enum EcsError { ArchetypeNotFound(ArchetypeId), ComponentNotRegistered(ComponentId), ... }`; remove `anyhow` from `boyko_ecs/Cargo.toml` |
| **C-023** | `ComponentPool.chunks: pub Vec<Chunk>`, `ArchetypeSignature.{mask, block_summary, section_summary}: pub`, `Entity.{id, generation}: pub`, `ComponentMask.blocks: pub` | Private fields + accessor methods; constructors that maintain invariants |

**Effort estimate**: 2-3 sessions (each is small per-file but
touches many call sites).

### Phase 4b — Event system review (2 findings)

| ID | Site | Question | Approach |
|----|------|----------|----------|
| **Q-020** | `Participants` and `Parameters` split — overengineered? | Architectural decision: keep split (current design — addressed via Q-001 native nested fields) OR collapse into single Event type (Bevy style). After Q-001 lands, evaluate. |
| **Q-019** | `ParticipantBuffer::get<P>` lacks TypeId check | Store `TypeId` in buffer alongside `participant_size`; `debug_assert_eq!` in `get`; consider `assert!` if cheap. Deferred here from Q-001 (Phase 1b-finish) — the buffer storage migration to `Vec<MaybeUninit<u8>>` was the precondition. |

**Effort estimate**: Q-020 is a design discussion (no code).
Q-019 is 1 session.

### Phase 4c — Remaining type-erasure hardening (1 finding)

| ID | Site | Fix |
|----|------|-----|
| **C-004 full** | Raw `ComponentPool::add(&[u8])` and `set_component(idx, &[u8])` still bypass TypeId checks (only typed API checks) | Migrate all internal callers (`Archetype::create_entity`, bulk APIs) to typed paths. Once raw API has no internal users, mark `pub(crate)` or remove. |

**Dependency**: requires C-010 (Phase 2a — `create_entity` API
redesign) since the bulk byte-API is the main internal caller.

---

## Phase 5 — Cleanup, style, micro-perf

Low-risk hygiene work. Not blocking anything but improves
maintainability and `cargo clippy --all-targets -- -D warnings`.

### Phase 5a — Inline policy sweep (3 findings)

| ID | Site | Fix |
|----|------|-----|
| **C-014 / M-020 / Q-029** | `#[inline(always)]` on required trait methods, trivial getters, `*_unchecked` accessors throughout the codebase. ~200 clippy pedantic warnings on this. | Replace with plain `#[inline]` unless profiler shows measurable improvement. Per principle #7: measured inlining only. |

**Effort estimate**: 1 focused session. Mechanical pass.

### Phase 5b — Component mask boundary (2 findings)

| ID | Site | Fix |
|----|------|-----|
| **M-009 / C-011** | `ComponentMask::set/unset/contains` uses `(component_id / 64) % 8` — silently wraps for `component_id >= 512` | Add `debug_assert!(component_id < 512)` at top; remove `% 8` (the assert makes it unnecessary) |

**Effort estimate**: 30 minutes.

### Phase 5c — Dead / duplicate code (3 findings)

| ID | Site | Fix |
|----|------|-----|
| **M-013** | `MemFreeBlockMaster::defragment` builds an `index_map: HashMap<usize, usize>` and never reads it | Delete the dead variable, or wire it into a notify callback |
| **M-017** | `SparseMap::swap_remove` and `SparseMap::pop_swap_remove` are byte-identical duplicates | Delete one; have the other call it |
| **Q-021** | `Archetype::create_entity` does not check for duplicate `ComponentId` in input | `debug_assert!` via `ComponentMask` building |

**Effort estimate**: 1 session.

### Phase 5d — Minor style / API ergonomics (10+ findings)

| ID | Description |
|----|-------------|
| **C-020** | `is_entity_valid` 3-way check; reorder for cache locality |
| **C-021** | `EntityMaster::with_capacity(capacity / 4)` magic constant — document or constify |
| **C-022** | `wrapping_add(1)` on generation — theoretically ABA after 2^64 ops; `debug_assert!(new_gen != 0)` |
| **C-024** | `ArchetypeMaster::add_pool(&self, arena: &Arena, ...)` — arena passed even though `ArchetypeMaster` holds `NonNull<Arena>` |
| **C-025** | Logic duplication between `EcsMaster::query_entities` / `ArchetypeMaster::find_*` / `Query` |
| **C-026** | Comments describe "what" not "why" in several places |
| **C-027** | Tests depend on global registry — flaky under `cargo test` parallelism (partly addressed by ID range isolation but full fix requires per-test reset) |
| **C-028..C-031** | Micro-opts: `ComponentMask::is_empty` recreation, `block_groups` preallocate, `NonNull::as_ref` vs `as_ptr` consistency |
| **Q-018** | `EventPool::clear` and `swap_remove` inconsistency on Drop semantics (when uncommented) |
| **Q-023** | `EventPool` race in `clear` (when uncommented) |
| **Q-026** | `EventId = u64` while `MAX_EVENTS = 256` — narrow to `u16` or `u8` |
| **Q-027** | `with_filters(include, exclude, optional)` — three `ComponentMask` args in a row, easy to confuse — builder pattern |
| **Q-028** | API ergonomics for query construction |
| **M-021..M-027** | Misc: `Arena::allocate_layout` panics instead of `Result`; `align_up` no overflow check; orphan-file references; mixed comment languages (now fixed); `Unit.buffer_index: usize` could be `u32` (resolved by M-005) |

**Effort estimate**: 2-3 sessions, mechanical work, can be batched.

---

## Suggested execution order

### Completed
- **Phase 1b finish — Q-001** ✅ (commits `c12cba7`, `a618e6e`, `5f35c70`, `6ba2d38`)
- **Phase 2c infrastructure** ✅ (`criterion` wired, CI workflow, docs updated)

### Next session

### Sessions 2-4
3. **Phase 2a hot-path Vec allocations** — start with **Q-011 QueryState
   cache** (biggest impact, hardest). Then batch the smaller ones.
4. **Phase 2b cache/layout** — `M-012` HashMap → BTreeMap is the
   single biggest cache win. Do it standalone with benchmarks.

### Sessions 5-7
5. **Phase 3a entity lifecycle** (C-006, C-007, C-009)
6. **Phase 3b orphan files decision** (Q-008, M-010 — user input required)
7. **Phase 3c unit/pool internals** (M-005, M-006, M-016)

### Sessions 8-10
8. **Phase 4a type-safety** (C-017, C-019, C-023)
9. **Phase 4b event system review** (Q-019, Q-020 — design discussion first)
10. **Phase 4c C-004 full** (after Phase 2a C-010 lands)

### Session 11+
11. **Phase 5 cleanup** — batch into 2-3 sessions, mechanical work.

### Permanently deferred / scoped out
- **Q-007** (EventPool Drop): blocked on uncommenting `event_pool.rs` —
  not Phase 1-5 work.
- **Public mdBook + audit-report Russian → English translation**:
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

`docs/AUDIT-2026-05-23.md` and `book/src/*` are still Russian
(deferred translation, task #36). Translate inline as findings get
closed (e.g., when finishing Q-001, translate audit § Q-001 paragraph).

### Git policy
- Commits authored by `Celtokisa <bluesteelll@hotmail.com>` only.
- **No `Co-Authored-By: Claude` tags** — ever.
- Commits grouped by logical findings (not per-file).
- Branch: `ecs`. Never `--force` or `--no-verify` without explicit
  user permission.

### Benchmarks budget
After Phase 2c (criterion wired), each Phase 2+ fix declares a target:
- Hot path: ≤ documented baseline + 0% (no regression).
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

1. **Q-008** — Orphan iterator/container files: implement or delete?
2. **M-010** — Commented-out `BitMask` / `BitSet512` / `BitStorage`:
   resurrect or delete?
3. **Q-020** — `Participants` / `Parameters` trait split: keep or
   collapse after Q-001 lands?
4. **C-019** — Replace `anyhow::Result` with `EcsError`? (Affects
   public API; one-time cost is high.)
5. **C-017** — Newtype wrappers for all ID types? (Affects every
   public signature; touches every test.)
6. **Schedule / scheduler** — Beyond the audit: when will
   `boyko-engine` add a `System` abstraction and parallel system
   scheduler? Several Phase 2 fixes (Q-011 QueryState in particular)
   are sized around what the eventual scheduler needs.

---

## How to read this file

- Each phase is a coherent unit of work, ideally one session
  per sub-phase (a, b, c, d).
- "Effort estimate" is for the full pipeline: architect → critic →
  developer → reviewer → tester → results-analyst → commit.
- "Audit ID" references `docs/AUDIT-2026-05-23.md` — read the audit
  section before starting any fix to get the original problem
  statement and proposed fix.
- Dependencies are explicit ("Phase X depends on Phase Y"). Do not
  start a dependent phase before its prerequisites.

When in doubt, start a session with: "Read
`docs/ROADMAP-PHASE-2-PLUS.md`. We are now working on [phase]."
The roadmap gives the agent context the audit alone cannot.
