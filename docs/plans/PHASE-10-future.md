# Phase 10 — Future work / backlog

**Status:** ⚪ DRAFT — collection of items that are out of scope for
Phases 1-9 but worth tracking so they don't get lost. Each item is
a future architect-cycle candidate; no commitment to timeline.
**Branch (when active):** `ecs`.

## Why this file exists

Phases 1-9 have a clean dependency chain. Phase 10 is the bucket for
**everything else** the audit identified as deferred, plus everything
the team decides during the build-out of Phases 6-9 that didn't fit
into the current cycle. Without this file, ideas evaporate.

When an item here grows enough scope to warrant its own architect
cycle, promote it to `PHASE-11-topic.md` (or similar) and remove it
from this file.

## Categories

### 10a — Change detection

**Scope:** track which components changed in the last frame, allow
queries to filter on `Changed<T>` / `Added<T>`.

**Reference:** Bevy implements per-component `Tick` records updated
by `Mut<T>` smart pointer deref. Each `World` has a global tick
counter; query iteration compares per-component tick against the
last-system-run tick.

**Cost:**

- Per-component `ComponentTick { added: u32, changed: u32 }` = 8 B
  per entity per component. Order-of-magnitude memory increase per
  archetype.
- `Mut<T>` deref must bump the tick on every mutation. Adds branch
  on the hot mutation path.

**Decision needed:** is the memory + per-mutation cost worth the
ergonomic gain of `Changed<T>` filtering? Many games achieve the
same result with explicit "dirty" event types written by hand.

**Trigger to promote out of backlog:** a user-driven workflow that
demands `Changed<T>` (e.g. expensive recomputation only on changed
inputs) appears.

### 10b — Reflection / serialisation

**Scope:** reflect components at runtime (component name, field
list, field types) and serialise / deserialise to a portable format
(e.g. for scene save/load, networking).

**Reference:** Bevy `Reflect` derive macro emits a runtime
`TypeRegistration` per component. Cost: ~200-1000 bytes per type for
metadata; cold path so D-cache impact is minimal but compile-time
balloons.

**Sub-scope:**

- 10b-1 Reflection registry (type name, fields, accessors).
- 10b-2 Bevy-style scene format (RON / JSON serialisation).
- 10b-3 Network-friendly format (zero-copy, binary, partial updates).

**Decision needed:** is the engine targeting a single-binary use
case (no scenes / no network)? If yes, this entire category is
unnecessary.

### 10c — Hierarchies / `Parent` / `Children`

**Scope:** built-in entity hierarchies à la Bevy `Parent` /
`Children` components, with consistent insertion / removal /
despawn-recurse.

**Reference:** Bevy's hierarchy is a regular pair of components
maintained by `Commands` extension methods.

**Decision needed:** baked into the engine, or shipped as a separate
crate built on top of the public API? Lean towards the latter: keep
`boyko_ecs` minimal, ship a `boyko_hierarchy` companion crate.

### 10d — Transforms / GlobalTransform propagation

**Scope:** standard `Transform` + `GlobalTransform` pair, propagated
hierarchically once per frame.

**Reference:** Bevy `bevy_transform`. Notable for being a great
benchmark for the scheduler — a flat array of 100 k transforms is
the canonical "this should saturate cores" workload.

**Dependency:** Phases 9 (scheduler) + 10c (hierarchies).

### 10e — Asset / resource management

**Scope:** `Handle<T>` smart-pointer style asset loading. Out of scope
for an ECS-only crate; would live in a sibling crate.

### 10f — `loom`-tested concurrency primitives

**Scope:** strengthen the existing `EventDispatcher` lanes and the
future Phase 9 work-stealing pool with full `loom` model-checking.
Phase 6 currently does not run loom — its design is correct by
construction (single-writer per lane, single-drainer), but a small
loom suite would catch any regression.

**Trigger:** Phase 9 will force this anyway. Optionally pre-empt
that for Phase 6 retroactively.

### 10g — Q-007 EventPool resurrection (currently blocked Phase 3d)

**Scope:** if a use case appears that requires the `EventPool` /
`EventPoolBundle` API (currently commented out in source), apply
the Phase 1b drop-discipline pattern (`drop_fn` invoked on `clear`
and `swap_remove`).

**Trigger to unblock:** explicit product decision.

### 10h — Q-020 Participants/Parameters collapse (currently deferred Phase 4b)

**Scope:** if a use case appears for participant-filtered event
dispatch (Bevy-style subscriber model), the split survives. If a
competing audit demands collapse, this is the architect-cycle to
rewrite the `#[event]` macro into a single-struct Event type.

**Trigger to unblock:** see Phase 4b plan file.

### 10i — Public mdBook + audit English translation (task #36)

**Scope:** translate the deferred Russian content in
`docs/AUDIT-2026-05-23.md` and `book/src/*` into English.

**Why deferred:** per
[`feedback-language-english-only`](../../../../../Users/flint/.claude/projects/D--claude-BoykoEngine/memory/feedback-language-english-only.md),
all artifacts are English. The audit is a historical snapshot and
translating it would mutate the historical record; per-finding
translations live in commit messages instead. The mdBook is a
fast-moving surface — translating before the API stabilises means
re-translating.

**Trigger to unblock:** Phase 9 lands → API stabilises → assign
`doc-writer` agent to a single translation batch.

### 10j — Phase 5d minor style / ergonomics (currently open)

**Scope:** the remaining items in `PHASE-05-cleanup.md` § 5d.
Mechanical work; batched into 2-3 sessions. Some items wait on
Phase 7 because their call sites are about to be rewritten.

**Trigger:** Phase 7 lands.

### 10k — Phase 4c full C-004 typed migration (currently blocked)

**Scope:** see `PHASE-04-architecture-refactors.md` § 4c. Migrate
remaining internal callers of `ComponentPool::add(&[u8])` to typed
paths, then mark the raw API `pub(crate)` or delete.

**Trigger:** Phase 7 lands → audit `ComponentPool::add(&[u8])` caller
list → migrate.

### 10l — Phase 2d-extension (mutable iters + arities ≥ 3)

**Scope:** see `PHASE-02-hot-path-performance.md` § Phase 2d-extension.
Mechanical extension of the pointer-bump pattern.

**Trigger:** Phase 7 lands; Phase 8 design clarifies whether the
variadic tuple-trait pattern is the right surface (likely yes, in
which case 2d-extension folds into Phase 8c).

### 10m — Spatial indexing / broadphase

**Scope:** built-in spatial index (BVH / sparse grid) for queries like
"all entities within radius R of point P". Out of scope for the
ECS core; lives in a companion crate built on top.

### 10n — Determinism guarantee

**Scope:** if the engine targets multiplayer / replay, the entire
system run must be deterministic across runs. Several places need
audit:

- Hash-based registries currently use `OnceLock` keyed by `TypeId`
  — non-deterministic across runs. Use stable component IDs.
- Per-thread `ThreadLane` IDs in Phase 6 — assignment order needs
  to be deterministic.
- Phase 9 work-stealing — by definition non-deterministic. Replay
  needs a single-threaded fallback.

**Trigger to unblock:** a deterministic-replay use case appears.

### 10o — Web / WASM target

**Scope:** does the engine build under `wasm32-unknown-unknown`?
Most of the code is portable but:

- Phase 6 `MAX_THREADS = 64` assumption fails on single-threaded
  WASM. Need a feature gate `single_threaded` to drop the multi-lane
  machinery.
- Phase 9 scheduler must have a single-threaded mode (probably
  exists naturally, but needs benches).

**Trigger:** WASM target requested.

### 10p — Pooled `Commands` allocator

**Scope:** when Phase 9 lands, profile the per-system `Commands`
buffer. If allocations during system bodies become a hotspot, replace
with a slab allocator (per-frame ring buffer).

## Promotion criteria

Item leaves Phase 10 (becomes its own `PHASE-NN.md`) when:

1. **User requests** it explicitly.
2. **Benchmarks identify** it as a frame-time bottleneck.
3. **Audit** finds a correctness concern that maps to one of these
   items.
4. **External dependency** changes — e.g. Rust gains a stable
   feature that simplifies an item.

When promoted:

1. Create `PHASE-NN-topic.md` using the template at the end of
   `PHASE-08-system-api.md`.
2. Remove the item from this file.
3. Add a row to `docs/plans/README.md`.
4. Run architect → critic cycle before the first developer dispatch.

## References

- Audit: [`docs/AUDIT-2026-05-23.md`](../AUDIT-2026-05-23.md) §§
  Q-020, Q-007 (open), C-027 (test isolation).
- Memory: deferred items from
  `feedback-language-english-only.md` and `MEMORY.md` index.
- Phase 4b, 5d, 3d plan files for the specific deferred items.
