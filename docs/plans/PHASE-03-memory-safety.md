# Phase 3 — Memory-safety hardening

**Status:** ✅ DONE for sub-phases 3a, 3b, 3c; 🔒 BLOCKED for 3d.
**Branch:** `ecs`
**Closed audit IDs:** C-006, C-007, C-009, Q-008, M-010, Q-009,
M-005, M-006, M-016.

## Goal

Eliminate the second tier of memory-safety issues — those that do not
crash the engine on its first call but cause silent corruption, leaks,
or use-after-free under realistic workloads. Phase 1 closed the
showstoppers; Phase 3 closes the long-tail.

## Why after Phase 2

Most Phase 3 fixes touch the same code paths as Phase 2 hot-path
optimisations. Doing 3 first would have required redoing the
allocator and entity lifecycle a second time when the perf path
landed. Done after 2, each Phase 3 fix can build on the post-2 layout.

## Sub-phases

### Phase 3a — Entity lifecycle correctness

**Status:** ✅ DONE.
**Audit IDs closed:** C-006, C-007, C-009.

**Key fixes:**

- **C-006 `Archetype::remove_entity`** — replaced the ambiguous
  `Option<EntityId>` return (which conflated "last slot", "swap
  happened", and "pool failure") with
  `enum RemoveOutcome { Last, Swapped { moved_entity }, PoolFailure }`.
  Compile-time `size_of` assertion ensures the enum stays in
  one cache word.
- **C-007 `EcsMaster::create_entity` rollback** — `EntityMaster`
  gained `pub(crate) fn rewind_allocate(&mut self)`. The new guard
  validates the archetype before allocate; on
  `get_archetype_mut` failure the allocate is rewound. No leaked
  `EntityId`.
- **C-009 two-phase commit in `ComponentPoolBundle`** — split into
  `can_push_entity_components` (precheck capacity across all pools)
  and `push_entity_components` (commit). The old
  `add_entity_components` is deleted.

**Miri retag fix (foundational, landed with Phase 3a):**

- Replaced `NonNull<Arena>` with `*const Arena` raw provenance
  throughout `Archetype` and `ArchetypeMaster`. Tree Borrows
  requires raw pointers for `UnsafeCell`-containing types that
  are aliased across calls.
- `.cargo/config.toml`: `MIRIFLAGS = -Zmiri-tree-borrows`.
- CI `continue-on-error` removed from the Miri job — Phase 3a Miri
  gate closed.

**Test delta:** +19 tests (4 × C-006, 5 × C-009, 5 × C-007, 5 ×
`rewind_allocate`).

### Phase 3b — Iterator / container cleanup

**Status:** ✅ DONE.
**Audit IDs closed:** Q-008, M-010 (closed by Phase 5b cleanup),
Q-009 (subsumed by Phase 2d loop-based design).

**Key fixes:**

- **Q-008 orphan files** — deleted:
  - `core/iters/sparse_iter.rs`
  - `memory/multi_pool_sparse_iter.rs`
  - `memory/sparse_iter_component_pool.rs`
  - `memory/iterators.rs` (empty stub)
  - `core/containers/*` (never wired)
  Per-entity iteration replacement landed as Phase 2d's
  `iter_one` / `iter_two`.
- **M-010 commented-out bit-mask files** — deleted:
  - `boyko_utils/src/bit_mask/bit_mask.rs` (598 lines, fully in `/* */`)
  - `boyko_utils/src/bit_mask/bit_set512.rs` (423 lines)
  - `boyko_utils/src/bit_mask/bit_storage.rs` (60 lines)
  Total: 1080 LOC of dead code removed. `mod bit_storage;` unwired.
- **Q-009 recursive `next_raw`** — closed transitively: the
  replacement iterators are loop-based by design.

### Phase 3c — Unit / pool internals

**Status:** ✅ DONE.
**Audit IDs closed:** M-005, M-006, M-016.

**Key fixes:**

- **M-005 / M-006 `Unit::buffer_index` removed** — the field was
  never read; `Unit` is now `{ ptr: *mut u8 }` (8 bytes per slot
  instead of 16). M-006 was moot once the type became single-field.
- **M-016 `SparseSlotMap` ABA** — the bumped `new_generation` is
  now actually stored on `remove`. Test
  `m016_sparse_slot_map_aba_safe` locks the regression.

### Phase 3d — Event lifecycle

**Status:** 🔒 BLOCKED.
**Audit IDs open:** Q-007.

**Why blocked:** the source-of-truth files `event_pool.rs` and
`event_pool_bundle.rs` are fully commented out. Phase 6 implemented
a different event system (`EventDispatcher` + `EventBuffer`) that
already handles `drop_fn` correctly. If the legacy `EventPool`
path is ever resurrected as part of a different API surface,
Phase 3d resumes with the same pattern Phase 1b applied to
`ComponentPool`.

**Trigger to unblock:** explicit product decision to bring back
`EventPool`. None on the roadmap.

## Exit criteria

| Criterion | Status |
|-----------|--------|
| `cargo test --all-targets` clean | ✅ |
| `cargo +nightly miri test` clean (tree-borrows) | ✅ |
| All audit IDs in 3a–3c marked closed | ✅ |
| No orphan files claiming "✅ implemented" in docs | ✅ |
| 3d Q-007 has documented blocker | ✅ |

## What this phase did NOT do

- It did **not** introduce newtype IDs — Phase 4a.
- It did **not** convert `anyhow::Result` to a domain error — Phase 4a.
- It did **not** make `EventPool` correct — that path is dead code.
- It did **not** add change detection / `Mut<T>` smart pointers —
  Phase 10 backlog.

## Cross-phase dependencies

- Phase 3a's `RemoveOutcome` enum is a precursor to the cleaner
  `delete_entity` flow that Phase 7's `EntityInland` rewrite
  preserves.
- Phase 3a's Miri retag work is the **template** for Phase 7's
  pointer-minting recipe (raw `*mut` provenance only,
  no `NonNull` reborrows for cross-`UnsafeCell` access).

## References

- Audit: [`docs/AUDIT-2026-05-23.md`](../AUDIT-2026-05-23.md) §
  C-006, C-007, C-009, Q-007, Q-008, Q-009, M-005, M-006, M-010,
  M-016.
- Legacy roadmap: [`docs/ROADMAP-PHASE-2-PLUS.md`](../ROADMAP-PHASE-2-PLUS.md)
  §§ Phase 3a–3d.
- Commits (selected): `c47ce1a` (C-007 + Miri gate), `a79a8ae` (C-009),
  `8dd97f0` (C-006 / `RemoveOutcome`), `139c61a` (M-016),
  `f205eed` (M-005), `3629362` (M-010 / Phase 5b cleanup).
