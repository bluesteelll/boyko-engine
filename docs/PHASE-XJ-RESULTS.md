# Phase X.J — Retire the Client-less Arena: Results

Branch `ecs`, commit `1ee2c46`. No standalone plan doc — X.J executed the
backlog filed in [PHASE-XI-RESULTS.md](PHASE-XI-RESULTS.md) (Honest residuals
item 6), which this phase closes. Premise: Phase X.I made every
`ComponentPool` own its backing memory (per-pool `VmReservation`), leaving the
shared `Arena` with **zero production clients** — X.J deletes it outright and
sweeps the dead identifiers/constants. Net **−2,999 LOC** (478 insertions,
3,477 deletions).

## What was deleted

- `memory/arena.rs` (1,053 LOC) + `memory/free_mem_block.rs` (801 LOC) — the
  shared-arena policy layer and its best-fit free-block tracker.
  `VmReservation` (`vm.rs`) stays as the single per-OS backing primitive
  (every `ComponentPool` + the entity `InlandStore`);
  `VmReservation::reserve_unzeroed` went with its sole client.
- `tests/arena_growth.rs` (255) + `tests/miri_arena_growth.rs` (91) — the
  arena-only X.F growth suites; `benches/allocator.rs` (212); the arena groups
  of `benches/arena_new.rs` (163 — the `ecs_master_new` gate group moved
  unchanged to `benches/ecs_master_new.rs`).
- The parameter-and-field chain: `EcsMaster`'s `arena: Box<Arena>` + the
  raw-provenance two-phase mint + `with_arena_reserve()` / `arena()`;
  `Archetype`'s vestigial `arena: *const Arena` (size stays 8480 B — tail
  padding under align 32 absorbs the 8 B);
  `ComponentPoolBundle::with_component_ids` / `add_pool` drop the `&Arena`.
- Constants/ids with zero readers: `DEFAULT_ARENA_RESERVE`,
  `ARENA_MIN_SLAB` / `ARENA_MAX_SLAB`, the master-era compaction class
  (`COMPACTION_THRESHOLD`, `MIN_COMPONENTS_FOR_COMPACTION`,
  `INITIAL_FREE_SLOTS_CAPACITY`, `MAX_EMPTY_CHUNKS_RATIO`), and the dead
  `ChunkId` / `InlandChunkId`. `ARENA_COMMIT_GRANULE` → `COMMIT_GRANULE`
  (live readers: vm.rs, inland_store.rs, pool layout math).

## Constructor collapse (the X.I ★R1-9 follow-up)

`ComponentPool::new(component_id, num_chunks, components_per_chunk)` →
**`ComponentPool::new(component_id, reserve_rows)`**. The X.I D2 mapping was
`reserve_rows = n × m` EXACTLY, so every call site collapsed mechanically by
multiplying the pair; the clamp-bypass contract (★R1-9) is unchanged and
re-documented. `with_default_sizes(component_id)` likewise loses the arena.

## Unsafe-surface reduction

`ArchetypeMaster::new()` / `with_capacity()` are **safe `fn`s again** (+ a
`Default` impl) — their `unsafe` contract existed only for the deleted
`arena_ptr` parameter. The SEND1 `Send + Sync` justification on `EcsMaster`
was rewritten in place (the historical `!Send + !Sync` Arena interior is
gone; the manual impls remain authoritative for the raw-pointer-bearing
subsystems). The `miri_phase8cd` / `miri_phase14a` suite headers now document
the required `-Zmiri-ignore-leaks` (deliberate bounded #53-class
`BundleColumnRecord` leaks, proven pre-existing by the X.I tester).

## Gates (all green)

| Gate | Result |
|---|---|
| `cargo check` + `clippy -D warnings` (workspace, all targets) | clean |
| `cargo test --workspace --all-targets` (debug) | **1,064 passed / 0 failed** |
| `cargo test -p boyko-ecs --release --lib` | 564 passed |
| wasm32-unknown-unknown check of `boyko_demo` | clean |
| Miri (TB + ignore-leaks) | `miri_pool_growth` 5/5, `miri_entity_store` 1/1, `miri_fixed_loop` 1/1 (the arena Miri suites were deleted with the arena) |
| `ecs_master_new` gate (≤ 7.5 µs) | **4.24 µs vs 6.61 µs pre-X.J (−36%)** — the dead reserve-only acquisition is gone |
| X.B/X.I pool pin tests | green (changed only by the mechanical ctor collapse) |

## Lesson

A subsystem whose last client left should be deleted in the same breath, not
"kept for later": the arena cost a constructor syscall on every world
(−36% of `EcsMaster::new` recovered here), an `unsafe` constructor contract
two structs away (`ArchetypeMaster::new`), and ~3 KLOC of code + tests +
benches that every audit had to re-read. Deletion was mechanical precisely
because X.I had already proven the replacement at its gates.
