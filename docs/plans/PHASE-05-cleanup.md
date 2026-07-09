# Phase 5 — Cleanup, style, micro-perf

**Status:** ✅ DONE for 5a, 5b, 5c. 🟡 OPEN for 5d.
**Branch:** `ecs`
**Closed audit IDs:** C-014, M-020, Q-029 (inline policy), M-009,
C-011 (mask), M-013, M-017, Q-021 (dead code + duplicates), plus the
Phase 5c ABA fix on `ArchetypeId` recycling.

## Goal

Mechanical hygiene work that the previous phases deferred because
each item was either (a) easy enough to batch later, or (b) blocked
on a decision now resolved. The point of Phase 5 is to leave a
clean baseline before Phase 6 starts adding subsystems on top.

## Why split into 5a–5d

Each sub-phase is self-contained and tests independently. They can
land in any order; the actual sequencing followed clippy gate
priority and observed regressions.

## Sub-phases

### Phase 5a — Inline policy sweep

**Status:** ✅ DONE.
**Audit IDs closed:** C-014, M-020, Q-029.

**What landed:**

- Mechanical pass over `boyko_ecs` and `boyko_utils`. 68
  `#[inline(always)]` sites demoted to plain `#[inline]`.
- Required trait methods, trivial getters, and `*_unchecked`
  accessors no longer carry `#[inline(always)]` unless the
  profiler explicitly showed a regression on the demotion.
- New convention recorded in `CLAUDE.md` principle #7 and in
  `memory/feedback-inlining-nuanced.md`.

**Why mechanical:** every site was a `(file, line, decision)`
tuple. No design change, no behaviour change. Benches show no
regression — the optimiser inlines small bodies regardless of
the hint when LTO is on.

### Phase 5b — Component mask boundary

**Status:** ✅ DONE.
**Audit IDs closed:** M-009, C-011.

**What landed:**

- `ComponentMask::set/unset/contains` now `debug_assert!` the
  bound `component_id < 512` and drops the `% 8` mod.
- Existing tests cover the boundary; new tests cover the
  out-of-range panic.

### Phase 5c — Dead / duplicate code + dual-generation `QueryState`

**Status:** ✅ DONE.
**Audit IDs closed:** M-013, M-017, Q-021. Plus a non-audit ABA
fix on `ArchetypeId` recycling that was discovered during the
Phase 6 design review.

**Dead-code cleanup:**

- **M-013** — `MemFreeBlockMaster::defragment` dropped the unused
  `index_map: HashMap<usize, usize>` it built and never read.
- **M-017** — `SparseMap::swap_remove` and `pop_swap_remove`
  were byte-identical; one is now a thin wrapper.
- **Q-021** — `Archetype::create_entity` `debug_assert!` panics on
  duplicate `ComponentId` in the input. Detected via the
  Phase 2b `ComponentMask` precheck.

**Dual-generation `QueryState` fix (`68d7890`):**

The Phase 2a `QueryState` used a single `ArchetypeGeneration` to
invalidate cached match results. But two separate events bump the
generation: **creation** of a new archetype (legitimate match
update) and **structural mutation** of an existing archetype (no
match update needed). Phase 5c split the bump:

- `creation_generation` — bumped when a new archetype is registered.
  Forces match re-scan.
- `structural_generation` — bumped on metadata mutations.
  Queries do not invalidate.

Without the split, queries unnecessarily re-scanned all archetypes
every frame in scenarios that mutated archetype metadata. With the
split, only legitimate matches re-scan.

Plus an ABA fix: `ArchetypeId`s do not recycle, but the cache key
combines `(ArchetypeId, creation_generation)` so a recycled-and-
re-registered ID could no longer alias to a stale cache hit.

### Phase 5d — Minor style / ergonomics

**Status:** 🟡 OPEN — 14+ small items batchable into 2-3 sessions.
**Audit IDs:** C-020, C-021, C-022, C-024, C-025, C-026, C-027,
C-028–C-031, Q-018, Q-023, Q-026, Q-027, Q-028, M-021–M-027.

**Items grouped by character of fix:**

#### 5d-1 — Reorderings + small constants

| ID | Site | Fix |
|----|------|-----|
| C-020 | `is_entity_valid` 3-way check | Reorder for cache locality (hot field first). |
| C-021 | `EntityMaster::with_capacity(capacity / 4)` | Document the `/ 4` or make it `const DEFAULT_FREELIST_RATIO`. |
| C-022 | `wrapping_add(1)` on generation | `debug_assert!(new_gen != 0)` to catch the theoretical 2^64 wraparound. |
| Q-026 | `EventId = u64` while `MAX_EVENTS = 256` | Narrow to `u16` or `u8` (saves 6–7 bytes per event call). |

#### 5d-2 — API ergonomics / small dedup

| ID | Site | Fix |
|----|------|-----|
| C-024 | `ArchetypeMaster::add_pool(&self, arena: &Arena, …)` | `ArchetypeMaster` already holds `*const Arena`; drop the parameter. |
| C-025 | `EcsMaster::query_entities` / `ArchetypeMaster::find_*` / `Query` duplication | Centralise via `QueryState`. |
| C-026 | Several "what" comments instead of "why" | Mechanical pass; delete redundant comments. |
| Q-027 | `with_filters(include, exclude, optional)` three same-type masks | Builder pattern. |
| Q-028 | Query construction ergonomics | Same builder, generalise. |
| M-021 | `Arena::allocate_layout` panics on overflow | Return `EcsResult<NonNull<u8>>`. |
| M-022 | `align_up` no overflow check | `checked_add` + return `EcsResult`. |
| M-024 | Mixed comment languages | Already fixed mid-flight; verify on full sweep. |
| M-027 | `Unit.buffer_index: usize → u32` | Closed by Phase 3c M-005 — verify no regressions. |

#### 5d-3 — Test isolation hardening (C-027)

| Sub-task | Status |
|----------|--------|
| ID-range partitioning for tests | ✅ Done in earlier phases. |
| Per-test registry reset | 🟡 Open. Requires either `#[serial_test]` or a per-binary fresh-process strategy. |

#### 5d-4 — Micro-optimisations

| ID | Site | Fix |
|----|------|-----|
| C-028 | `ComponentMask::is_empty` recreation | Cache or use bitor-tree. |
| C-029 | `block_groups` preallocate | `Vec::with_capacity` at struct creation. |
| C-030 | `NonNull::as_ref` vs `as_ptr` consistency | Pick one per function family. |
| C-031 | Several small constant-fold opportunities | Pass through with `cargo asm`. |

#### 5d-5 — Event-related (mostly cold-path)

| ID | Site | Note |
|----|------|------|
| Q-018 | `EventPool::clear` / `swap_remove` Drop inconsistency | Blocked by 3d (`EventPool` is dead code). Re-evaluate if it ever resurrects. |
| Q-023 | `EventPool::clear` race | Same blocker. |

#### 5d-6 — Arena boundary / containers

| ID | Site | Fix |
|----|------|-----|
| M-023 | Orphan-file references in docs | Already fixed by Phase 2c / 3b cleanup; verify zero references on full sweep. |
| M-025–M-026 | Misc memory accounting / debug printing | Low priority; batch with 5d-1. |

**Effort estimate for 5d (entire phase):** 2-3 focused sessions.

## Exit criteria

### 5a–5c — all met

- [x] No `#[inline(always)]` survives without a profiler-driven
      justification.
- [x] `cargo clippy --all-targets -- -D warnings` clean (after the
      pre-existing-debt sweeps `883e27a` and `3b0804b`).
- [x] `cargo test --all-targets` green; 260+ tests workspace-wide.
- [x] Dead code identified by `cargo check` warnings is gone.
- [x] `QueryState` ABA-safe with `(creation_gen, structural_gen)`.

### 5d — pending

- [ ] All 5d-1 reorderings + constants landed.
- [ ] 5d-2 ergonomics batched into one PR per group.
- [ ] 5d-3 per-test reset strategy decided + implemented.
- [ ] 5d-4 micro-opts validated against criterion baselines.

## What this phase did NOT do

- It did **not** introduce new features. Everything in Phase 5 is
  a hygiene improvement to existing surfaces.
- It did **not** touch documentation in `book/src/*` — English
  translation deferred to post-Phase-9.
- It did **not** unblock 3d (EventPool resurrection) — explicit
  product decision required.

## Cross-phase dependencies

- **5d** waits on Phase 7 because several call sites flagged by
  C-025 (`Query` / `EcsMaster::query_entities` duplication) will
  be touched by Phase 7's `EcsMaster` fast-path methods. Landing
  5d first means redoing the work.
- **5a** demoted 68 inlines; Phase 6 / 7 must follow the new
  policy (no `#[inline(always)]` without measured justification).

## References

- Audit: [`docs/AUDIT-2026-05-23.md`](../AUDIT-2026-05-23.md) §
  C-014, M-020, Q-029, M-009, C-011, M-013, M-017, Q-021,
  C-020–C-031, Q-018, Q-023, Q-026–Q-028, M-021–M-027.
- Legacy roadmap: [`docs/ROADMAP-PHASE-2-PLUS.md`](../ROADMAP-PHASE-2-PLUS.md)
  §§ Phase 5a–5d.
- Commits (selected): `a4127bf` (5a), `53c9418` (5b), `8823535`
  (5c M-013 / M-017 / Q-021), `68d7890` (5c QueryState dual-gen).
