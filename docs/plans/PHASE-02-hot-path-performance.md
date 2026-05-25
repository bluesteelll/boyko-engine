# Phase 2 — Hot-path performance

**Status:** ✅ DONE for sub-phases 2a, 2b, 2c, 2d, 2e
**Branch:** `ecs`
**Closed audit IDs:** Q-011, Q-012, Q-013, Q-015, Q-016, Q-017,
C-010, M-019, C-012, C-013, C-015, M-012, M-018, C-016, M-007, Q-026.

## Goal

Bring every per-frame hot path under the constraints of `CLAUDE.md`
principles #1 (zero overhead), #3 (cache), #5 (zero allocations).
The engine compiles and is correct after Phase 1; this phase makes
it **competitive on per-frame microbench** with Bevy / flecs.

## Why now

A correctness-only ECS that allocates a `Vec` per query call is
useless for 100k entities at 60 fps. Every per-entity allocation
costs 30–60 ns of malloc + free, dominating the cost of the actual
component work. Phase 2 establishes the no-alloc steady state that
later phases (queries, events, scheduler) can rely on.

## Sub-phases

### Phase 2a — Eliminate hot-path Vec allocations

**Audit IDs closed:** Q-011, Q-012, Q-013, Q-015, Q-016, Q-017,
C-010, M-019, C-012, C-013, C-015.

**Key fixes:**

- **Q-011 `QueryState` cache + `ArchetypeGeneration`** —
  cache-aligned (`#[repr(C, align(64))]`) state object stores matched
  archetype IDs in an inline `ArchetypeBitSet`. Bumped on every
  `create_archetype`; `Query::iter` is `&self` so multiple queries
  can iterate in parallel without re-matching.
  **Measured:** warm path ≈ 3.6 ns vs 77 ns one-shot — ~21× speedup
  (Windows x86_64, N = 10k / 100k).
- **Q-012 `ComponentSet::component_ids` static slice** —
  unit `()` returns `&[]`, single-component returns from
  `[OnceLock<&'static [ComponentId]>; 512]`, tuples up to N=8 via
  `Box::leak` (cold cost only, query construction is not the hot path).
- **Q-013 `find_archetypes_*_into` zero-alloc steady state** —
  every registry / `ArchetypeMaster` find method now has a
  `_into(out: &mut Vec<ArchetypeId>)` sibling. The original methods
  are thin wrappers for callers that don't have a scratch buffer.
- **C-010 `create_entity(&[(ComponentId, &[u8])])`** — replaced the
  per-call `Vec` allocation with a borrowed slice; bench harness
  in `benches/allocator.rs` confirms the path is alloc-free.
- **M-019 `chunk_units` returns `&[Unit]`** — slice instead of a
  freshly allocated `Vec<*const u8>`.
- **C-012 / C-013 `iter_entities` is O(active)** — `SparseMap`
  gained a dense iterator; reuses the existing dense layout instead
  of probing every slot.
- **C-015 `ArchetypeRegistry` O(1)** — `total_count` cached,
  reverse `ArchetypeId → pattern` map for O(1) `signature` lookup,
  `unregister_archetype` no longer clones `active_patterns`.

### Phase 2b — Cache / layout improvements

**Audit IDs closed:** M-012, M-018, C-016, M-007 (closed by 2c
cleanup).

**Key fixes:**

- **M-012 `MemFreeBlockMaster` `BTreeMap`** — replaced the two
  `HashMap<usize, usize>` (start_map / end_map) with `BTreeMap`.
  Ordered traversal also unlocks future best-fit allocator work.
- **M-018 reverse-index `Vec`** — O(1) free-block removal instead
  of the previous O(N) `iter().position()` scan in `mem_size_tree`.
- **C-016 `ComponentMask` precheck** — `Archetype::create_entity`
  now folds the input into a temporary mask once and compares against
  `self.signature.mask`. O(8) instead of O(N×M).

### Phase 2c — Infrastructure

**Status:** ✅ DONE.

**Items landed:**

- `criterion` wired into `boyko_ecs/Cargo.toml` `[dev-dependencies]`.
- Bench harnesses: `benches/component_id.rs`, `benches/swap_remove.rs`,
  `benches/query_iter.rs`, `benches/allocator.rs`,
  `benches/event_dispatch.rs`.
- GitHub Actions CI at `.github/workflows/ci.yml`: check / test /
  clippy / bench-compile / Miri.
- Nightly + Miri configured; CI runs `event_attribute` and `drop_fn`
  sweeps (clean) and a full sweep (informational).
- Orphan-file cleanup: `sparse_iter`, `multi_pool_sparse_iter`,
  `sparse_iter_component_pool`, `iterators` stub all deleted.
  `FEATURE_MAP.md` / `SYSTEMS.md` / `ARCHITECTURE.md` corrected.

### Phase 2d — Per-entity query iter (bounded)

**Status:** ✅ DONE for 1-arity and 2-arity. Extensions tracked
below as Phase 2d-extension.

**What shipped:**

- `Query::iter_one<A: Component>() -> QueryIterOne<'_, A>` —
  yields `&A`.
- `Query::iter_two<A, B: Component>() -> QueryIterTwo<'_, A, B>` —
  yields `(&A, &B)`.
- Zero-alloc per row (pointer-bump cursors + `remaining` counter),
  archetype-major order, loop-based (no recursion — closes Q-009).
- Reuses `QueryState::matched_ids()` from Phase 2a.
- Six tests in `query.rs` lock down the contract.
- `ComponentPool::buffer_ptr()` accessor added with `// SAFETY:`.

**Why bounded first:** the pointer-bump skeleton is the load-bearing
piece. Mutable variants and higher arities are mechanical extensions
once the immutable path is stable.

#### Phase 2d-extension — open

| Sub-ticket | Scope | Notes |
|------------|-------|-------|
| `iter_one_mut` / `iter_two_mut` | Mutable variants | Needs aliasing discipline — `&mut` cannot be juggled the same way as `&` in pointer-bump. Either `*mut` + lifetime gymnastics or split-borrow per row. |
| Arities 3 – 8 | `iter_three` … `iter_eight` | Macro-emitted family OR generic `Fetch` trait. |
| `Query::iter::<(&T, &U)>` | Variadic tuple-trait pattern | Replaces named-arity methods. Likely Phase 2d-final once arities + mut settle. |
| Change-detection / `With` / `Without` / `Changed` | Filter combinators | Out of Phase 2d — depends on Phase 3+ change detection (currently not on roadmap). |

### Phase 2e — `ComponentTuple` ergonomic spawn API

**Status:** ✅ DONE for 1-arity and 2-arity (commit `017cd61`).

**What shipped:**

- `EcsMaster::spawn_one::<A: Component>(arch_id, A) -> EcsResult<Entity>`.
- `EcsMaster::spawn_two::<A, B: Component>(arch_id, (A, B)) -> EcsResult<Entity>`.
- Mirrors the bounded pattern of Phase 2d — generalised variadic
  trait deferred until both `iter_*` and `spawn_*` arities settle.

## Exit criteria — all met

- [x] `cargo bench --all-features` runs end-to-end (criterion).
- [x] Hot loop has 0 allocations per entity per tick (verified via
      `cargo bench` reports and Miri leak counters).
- [x] `Query::iter` warm-path measured ≈ 21× faster than the
      one-shot baseline.
- [x] `Archetype::create_entity` precheck dropped from O(N×M)
      to O(8).
- [x] `MemFreeBlockMaster` no longer uses `HashMap`.
- [x] Orphan-iterator files deleted; docs corrected.

## What this phase did NOT do

- It did **not** make `Query` faster on the **cold** path —
  archetype matching still scans active patterns when `QueryState`
  is invalidated. The dual-generation fix is in Phase 5c.
- It did **not** lower per-entity random-access cost — `get_component`
  is still ~40 ns through 9 cache lines. That is Phase 7.
- It did **not** add change detection — deferred to Phase 10.
- It did **not** add mutable iterators or arities ≥ 3 — deferred to
  Phase 2d-extension.

## Cross-phase dependencies

- **C-010** in Phase 2a unlocked **C-004 full** (Phase 4c) because
  the bulk byte-API was the main internal caller of `add(&[u8])`.
- **Q-011 `QueryState`** matured into the dual-generation
  ABA-safe variant in Phase 5c.
- **Phase 2c bench infrastructure** is the entrance criterion
  for every later phase — no perf-related landing without
  criterion baseline.

## References

- Audit: [`docs/AUDIT-2026-05-23.md`](../AUDIT-2026-05-23.md) §
  Q-011 … Q-017, C-010 … C-016, M-007 … M-019.
- Legacy roadmap: [`docs/ROADMAP-PHASE-2-PLUS.md`](../ROADMAP-PHASE-2-PLUS.md)
  §§ Phase 2a–2e.
- Commits (selected): `11aeef8` (Q-011a), `150501b` (Q-011b),
  `1201c59` (Q-011c), `5ad2603` (Q-011d), `bd63b7b` (Q-012),
  `cf159c6` / `33744d0` (Q-013), `017cd61` (Phase 2e),
  `f00d4bf` (Phase 2d), `6deb762` (M-019), `5e93a8d` (C-010),
  `1ffb5c6` (C-012 / C-013), `f29da6d` (C-015), `bb0dc03` (C-016),
  `537fcaa` (M-018).
