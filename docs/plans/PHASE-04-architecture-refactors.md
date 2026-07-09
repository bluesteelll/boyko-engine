# Phase 4 — Architecture refactors

**Status:** ✅ DONE for 4a and 4b; 🔒 BLOCKED-BY-DEPENDENCY for 4c.
**Branch:** `ecs`
**Closed audit IDs:** C-017, C-019, C-023, Q-019.
**Deferred:** Q-020.
**Open:** C-004 (full).

## Goal

Tighten the public API surface so that it (a) prevents categorically
illegal call sites at compile time, (b) replaces the catch-all
`anyhow::Result` with a domain error, and (c) hides invariants behind
constructors. None of these change behaviour — they make incorrect
behaviour impossible to express.

## Why after Phase 3

Each refactor touches every test and most internal callers. Doing
them earlier would have forced rework on every UB / leak fix from
Phases 1 and 3. Done in Phase 4, the refactors land once.

## Sub-phases

### Phase 4a — Type-safety wins

**Status:** ✅ DONE.
**Audit IDs closed:** C-017, C-019, C-023.

**Key fixes:**

- **C-017 newtype identifiers** —
  `EntityId(usize)`, `ArchetypeId(usize)`, `ComponentId(usize)`,
  `Generation(usize)` are now `#[repr(transparent)]` newtypes with
  `pub(crate)` constructors. `archetype.has_component_id(entity.id())`
  no longer type-checks.
- **C-019 domain `EcsError`** — added `#[non_exhaustive]`
  `enum EcsError { … }` with the concrete failure modes (archetype
  not found, component not registered, capacity exceeded, …) plus
  `pub type EcsResult<T> = Result<T, EcsError>`. `anyhow` dropped
  from `boyko_ecs/Cargo.toml`. Tests assert the concrete variant.
- **C-023 encapsulation** — `pub` fields on `ComponentPool::chunks`,
  `ArchetypeSignature::{mask, block_summary, section_summary}`,
  `Entity::{id, generation}`, and `ComponentMask::blocks` are now
  private with accessor methods. Constructors maintain the
  documented invariants.

### Phase 4b — Event system review

**Status:** ✅ DONE for Q-019; Q-020 intentionally deferred.

**Q-019 closed (`ParticipantBuffer` / `ParametersBuffer` TypeId
guard):**

- Both buffers carry `TypeId` alongside size.
- `debug_assert_eq!` on typed access (`push`, `get`).
- Eight new tests cover correct round-trip and the wrong-type panic.

**Q-020 deferral rationale (re-stated):**

The split survives because:

1. Q-001 already made it sound (native nested fields, no UB cast).
2. Q-019 already guards type confusion.
3. The audit's "overengineered" framing assumed a Bevy-style
   subscriber model where dispatch filters by participant set.
   `boyko_ecs` does not implement that filtering today and has no
   committed timeline for it.
4. Collapsing the split now would force:
   - Rewrite of the `#[event]` macro.
   - Migration of every `Event::Participants` / `Event::Parameters`
     assoc-type consumer.
   - Doc churn across `SYSTEMS.md` and the public mdBook.

The ticket reopens the moment a real use case for participant-
filtered dispatch appears, or a competing audit demands it. Phase 6
event dispatch did **not** require collapse.

### Phase 4c — Raw byte-API hardening

**Status:** 🔒 BLOCKED — depends on Phase 2a C-010 (DONE) plus a
plan-of-record for moving every internal `add(&[u8])` caller to a
typed path.
**Audit IDs open:** C-004 full.

**Why blocked even though C-010 is done:**

C-010 changed the public `create_entity` signature to take a
borrowed slice; the underlying `ComponentPool::add(&[u8])` raw API
remained. Phase 4c is the work of:

1. Auditing every internal caller of `ComponentPool::add(&[u8])` and
   `ComponentPool::set_component(idx, &[u8])`.
2. Migrating them to `add_typed::<T>(value: T)` /
   `set_component_typed::<T>(idx, value: T)`.
3. Marking the raw API `pub(crate)` or deleting it once the caller
   count is zero.

**Sequencing note:** Phase 7 changes the layout of `Archetype` (adds
the column array, deletes `init_entity_inland`, changes
`create_entity` signature). Several call sites Phase 4c would need
to migrate are about to be rewritten by Phase 7 anyway. Therefore
Phase 4c is **paused until after Phase 7** so we touch each site
once.

**Trigger to unblock:** Phase 7 implementation complete + a single
fresh audit run on `pub fn add(&[u8])` / `pub fn set_component`
in `component_pool.rs` to see what callers remain.

## Exit criteria

### 4a — all met

- [x] No `usize` IDs leak into public signatures.
- [x] `cargo expand` shows the newtypes have zero runtime cost.
- [x] Public API returns `EcsResult<T>` exclusively. `anyhow` is
      not in `Cargo.toml`.
- [x] No `pub` fields remain on `ComponentPool`, `ArchetypeSignature`,
      `Entity`, `ComponentMask`.

### 4b — all met (within scope)

- [x] Both event buffers carry `TypeId`.
- [x] `debug_assert_eq!` covers both push and get.
- [x] Eight new tests added.
- [x] Q-020 deferral rationale recorded in this file.

### 4c — pending

- [ ] Audit `ComponentPool::add(&[u8])` callers after Phase 7.
- [ ] Migrate remaining callers to typed paths.
- [ ] Mark raw API `pub(crate)` or delete.

## What this phase did NOT do

- It did **not** redesign `Participants` / `Parameters` (Q-020 deferred).
- It did **not** introduce a builder for `create_entity` — slice-based
  signature is sufficient.
- It did **not** rename any public type or method.
- It did **not** stabilise the public API for crates outside the
  workspace — that is a post-Phase-9 decision.

## Cross-phase dependencies

- **Phase 1a / 1b** must precede Phase 4a (the newtype migration
  touches every UB-related fix site).
- **Phase 2a C-010** is a precondition for Phase 4c (the bulk
  byte-API entry point is now slice-based).
- **Phase 7** is the precondition for the remaining Phase 4c work
  (see "Sequencing note" above).

## References

- Audit: [`docs/AUDIT-2026-05-23.md`](../AUDIT-2026-05-23.md) §
  C-004, C-017, C-019, C-023, Q-019, Q-020.
- Legacy roadmap: [`docs/ROADMAP-PHASE-2-PLUS.md`](../ROADMAP-PHASE-2-PLUS.md)
  §§ Phase 4a–4c.
- Commits (selected): `8f7d02f` (C-017), `1db0560` (C-019),
  `80facb3` (C-019 follow-up assertion), `5526927` (C-023),
  `41321a8` (Q-019), `95a91b6` (typed read wrappers C-004 partial).
