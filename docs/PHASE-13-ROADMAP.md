# Phase 13+ Roadmap

After closing Phase 12.5 + Phase 12.6, the boyko-engine ECS is in a state
where remaining performance gaps can be addressed via **focused refactors**
of specific subsystems — no full architectural redesign is required. This
document captures the strategic plan for the next iterations.

## Current state (post Phase 12.6)

**3 of 4 head-to-head benches** beat Bevy by ≥1.10×:
- 50 empty systems: **1.72× win**.
- par_iter 10k: **2.93× win**.
- query iter (direct API): **1.00× parity** (closed from 0.88× loss).
- spawn_batch warm path: **1.35× of Bevy** (close to parity from 11× regression).

`EcsMaster::new`: **9-31× improvement** (712 µs → 23-75 µs) via lazy
allocation; remaining 23-75 µs is Arena 64 MB heap commit.

**667/667 tests pass**. Build + clippy clean. Phase 11 EntityCommands chaining,
Phase 12.5 panic-recovery semantics, Phase 12.5 NCD6 const-fold all intact.

## Architectural assessment

The current architecture is sound:
- **Archetype + chunked component pool** storage (matches Bevy/flecs leading practice).
- **Atomic counter Entity ID** allocation (Phase 11 EntityCounter newtype).
- **Per-row Tick storage** with `Box<[UnsafeCell<Tick>]>` (Phase 10 change detection).
- **Bevy-class parallel scheduler** with Chase-Lev work-stealing, conflict graph,
  Tarjan SCC, Kahn topological sort, apply-window barrier (Phase 9).
- **Static Bundle Cache** via `Box<[OnceLock<ArchetypeId>; 1024]>` (Phase 8.5).
- **CommandQueue** with hoisted `catch_unwind` + `CursorSync` RAII (Phase 12.5 + 12.6).
- **Events as SystemParam** with cached `NonNull<EventBuffer<E>>` (Phase 12).

None of these subsystems need fundamental rework. All remaining performance
residuals can be closed by **targeted changes** to specific subsystems.

## Feature roadmap (additive, non-disruptive)

Each of these can be implemented WITHOUT disturbing existing perf wins.
They are also independent of each other — any order is workable.

### Phase 13 — `Local<T>` SystemParam — ✅ DONE

Per-system private state slot. **Landed** (commits `f6b4807` impl+wiring,
`1db3e5c` tests). `Local<'s, T>` (`T: Send + Sync + Default + 'static`),
`#[repr(transparent)]` over `&'s mut T`, `type State = T` living in
`FunctionSystem::state` (NOT `SystemMeta` — the original roadmap phrasing
was loose), default-initialized once and persisted across runs of a cached
system. Declares zero access → no conflict-graph edge → never blocks
parallelism. A strict structural subset of the Phase 12 `EventReader`
(no cached pointer, no `unsafe` block inside methods); critic round skipped
on that basis. Plan: `docs/PHASE-13-LOCAL-PLAN.md`; research:
`docs/PHASE-13-RESEARCH.md`. 5 integration tests + 4 unit + 1 trybuild,
Miri 5/5 clean, full suite 668 pass. Decisions: A1 (`State = T`, no
`SyncCell`), B1 (`Default`, `FromWorld` deferred — backward-compatible
widening). Reachable at `boyko_ecs::ecs::core::system::Local`.

### Phase 14 — Observers / lifecycle hooks
Component spawn / despawn / insert / remove callbacks. CRITICAL DESIGN
CONSTRAINT: callbacks must be `#[cold]` and opt-in to avoid hot-path
pollution. Default-disabled per component type; enable via builder API.
~2-3 weeks.

### Phase 15 — Schedule sets / system orderings
Bevy-style `before` / `after` / `in_set` constraints with topological
resolution. Phase 9's `ConflictGraph` + Kahn already handles dependency
edges; this is mostly a builder-API addition. ~1-2 weeks.

### Phase 16 — Run conditions
`.run_if(cond)` predicates on systems. Lazy evaluation per frame. Cheap
to add via SystemMeta extension. ~1 week.

### Phase 17 — States / state transitions
Tagged enum states + `OnEnter` / `OnExit` schedule labels. Composes with
Phase 15 / 16. ~2-3 weeks.

### Phase 18 — Plugin system
Modular crate composition via `App::add_plugin(MyPlugin)`. Bevy-style. No
ECS core changes; pure builder API. ~1-2 weeks.

### Phase 19 — Hierarchies / Parent-Child
Entity relationship via `Parent` / `Children` components + propagation
schedule. Could integrate with Phase 14 observers for auto-despawn-children.
~3-4 weeks.

## Performance polish (focused refactors, interleavable with features)

Each one is a **targeted refactor of one subsystem**. They do NOT require
architectural changes and can land at any time between feature phases.
None of them blocks each other.

### Phase X.A — `Query::for_each_chunk` batched API

**Goal**: close Phase 12.6 Residual 2 (query iter ≥1.10× Bevy).

**Design**: add `Query::for_each_chunk<F>(&mut self, f: F) where F: FnMut(&[T])`
that yields per-archetype contiguous slices instead of per-row items. Allows
LLVM auto-vectorization when paired with `core::intrinsics::fadd_algebraic`
(nightly) or explicit `std::simd` reductions.

**Scope**:
- New entry point in `crates/boyko_ecs/src/ecs/core/iters/query/query.rs`.
- Existing `iter()` / `iter_mut()` untouched (no API break).
- Bench harness change: drop per-element `black_box`, use slice reduction.

**Expected gain**: 5-20× on suitable workloads (per orlp.net `fadd_algebraic`
+ Nick Wilcox `chunks_exact` measurements). On the current bench: not
applicable (the bench uses per-element `black_box`).

**Sound rationale**: flecs C API works exactly like this (`ecs_field` returns
columnar slice; user owns inner loop). Bevy never shipped a batched API
([issue #1990](https://github.com/bevyengine/bevy/issues/1990) open since 2021).
This would be a real boyko differentiator.

**Estimated cost**: 1 week.

### Phase X.B — `ComponentPool::Vec<Unit>` parallel storage elimination

**Goal**: close Phase 12.6 Residual 3 (Commands::spawn single 3× slower).

**Current**: every `ComponentPool` maintains a `Vec<Unit>` parallel to the
component data buffer. `units[i].ptr()` returns the byte pointer for row
`i`. This costs ~5-10 ns/entity on the spawn hot path (push + cache line touch).

**Refactor**: compute `buffer.ptr() + i * stride` on every random-access
read. Eliminates the parallel `Vec<Unit>` entirely.

**Scope**:
- `ComponentPool::get_raw` / `get_typed_at` / `set_component` / `swap_remove`
  — change `units[i].ptr()` → `unsafe { buffer.ptr().add(i * stride) }`.
- ~10 files touched, all inside the `ecs/memory/component_pool.rs` neighbourhood.
- Phase 11 `swap_remove_index_no_drop` may also benefit.

**Expected gain**: ~5-10 ns/entity on Commands::spawn hot path. Will also
reduce ComponentPool memory footprint by ~24 B/row.

**Estimated cost**: 1-2 weeks (cross-cut on read paths needs careful audit).

### Phase X.C — Arena `VirtualAlloc(MEM_RESERVE)`

**Goal**: close Phase 12.6 `EcsMaster::new` residual 23-75 µs.

**Current**: `Arena::with_capacity(64 MB)` eagerly commits 64 MB via
global allocator on world creation. Most of that is never used.

**Refactor**: switch to `VirtualAlloc(MEM_RESERVE)` on Windows /
`mmap(MAP_NORESERVE, PROT_NONE)` on Linux. Reserve virtual address range
without committing physical pages. Pages commit lazily on first write
(OS page-fault handler).

**Scope**:
- `crates/boyko_ecs/src/ecs/memory/arena.rs` only.
- Two cfg-gated impls: Windows + Unix.

**Expected gain**: `EcsMaster::new` drops to ≤ 5 µs (just the field
initialization + a few `mmap`/`VirtualAlloc` syscalls).

**Estimated cost**: 3-5 days.

### Phase X.D — `EntityMaster::register_entity_with_ptr` slot reduction

**Goal**: close another part of Phase 12.6 Residual 3.

**Current**: writes 3 slots per spawn (`entities_inland` + `active_ids` +
`sparse_to_active`). Bevy writes 1.

**Refactor** (speculative — needs careful design):
- `entities_inland` (queries / fast-path lookup) — REQUIRED.
- `active_ids` (iteration) — possibly defer-batched at frame end.
- `sparse_to_active` (despawn O(1) swap-remove) — possibly lazy.

**Estimated gain**: ~5-8 ns/entity.

**Estimated cost**: 1-2 weeks (despawn invariant rework).

### Phase X.E — Multi-run bench methodology

**Goal**: extract structural perf signals from per-iter `EcsMaster::new`
allocator variance (±20-30% on g4 / g5 benches).

**Approach**:
- Adopt median-of-medians across multiple criterion runs.
- Or: switch to a bench harness that builds the world ONCE and runs N
  iterations of the work without per-iter setup.
- Or: PGO build (`cargo pgo` or manual `-Cprofile-use`).

**Estimated cost**: 3-5 days.

## When full architectural redesign would actually be needed

Only if pursuing:
1. **Hybrid sparse-set + archetype storage** (EnTT/Bevy hybrid model).
   Useful for add/remove-heavy components, NOT for dense iteration which
   we already match Bevy on. Defer indefinitely.
2. **SoA→AoS transition** or radical layout change. Not justified.
3. **Burst-style code generation** (Unity DOTS pattern). Out-of-scope —
   would be a separate project, not Phase 13.

**None of these are required to close Phase 12.6 residuals.**

## Recommendation

Interleave feature phases (13-19) with perf-polish phases (X.A through X.E)
as opportunities arise. A reasonable cadence:
- Phase 13: Local<T> (3-5 days).
- Phase X.A: `Query::for_each_chunk` (1 week) — gives a real boyko
  differentiator vs Bevy on SIMD-amenable workloads.
- Phase 14: Observers (2-3 weeks) — with `#[cold]` hot-path discipline.
- Phase X.B: `Vec<Unit>` elimination (1-2 weeks).
- Phase 15: Schedule sets (1-2 weeks).
- Phase X.C: Arena `VirtualAlloc` (3-5 days).
- Phase 16-19: remaining features at convenient pace.
- Phase X.D / X.E: polish as needed.

Total: ~3-4 months of feature + perf work without any architectural redesign.

## Constraint: feature phases must respect hot-path discipline

When adding features (especially Observers / hooks), the design MUST:
- Default-disable lifecycle callbacks per component type.
- `#[cold]` + `#[inline(never)]` on callback dispatch sites.
- Compile-time elision (`if const { HAS_HOOKS }`) where possible.
- No additional indirection on the spawn / iter hot paths unless feature is
  explicitly enabled.

This keeps the door open for the Phase X polish to land cleanly without
fighting accumulated hook overhead.
