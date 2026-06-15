# boyko-engine architecture (branch `ecs`)

> This documentation reflects the state of the **`ecs` branch** — the cumulative
> result of Phases 2 → 18, the 9.x executor-soundness series, the X.x perf
> series, and Phases 14a/14b. Comparison with `master` is at the end. For the
> per-subsystem catalog see [SYSTEMS.md](SYSTEMS.md); for the "where is X?"
> lookup see [FEATURE_MAP.md](FEATURE_MAP.md).

## Goals and non-goals

**Goals:**
- Performance on par with (and where measured, beating) state-of-the-art ECS
  engines (Bevy / flecs / Unity DOTS / EnTT).
- Cache locality via type-erased columnar storage + archetype-grouping, an
  inline per-archetype column table for O(1) random component access (Phase 7),
  and a SIMD-aligned columnar batched API (`for_each_chunk`, Phase X.A).
- Lock-free parallelism: a custom Chase-Lev work-stealing pool drives a
  Bevy-class conflict-graph scheduler (Phase 9), proven sound under loom + Miri.
- Minimal per-entity / per-component footprint.
- Zero-cost generics — no dynamic dispatch in the hot path.

**Non-goals (current stage):** scripting (Lua/Wasm scripting), component
hot-reload, serialization (deferred until the model stabilizes), cross-platform
beyond x86_64 (wasm32 compiles but is not the perf target).

## Workspace layout

The workspace is six crates (`Cargo.toml` `members`):

```
boyko-engine/
├── Cargo.toml                            # workspace (6 members) + [profile.bench] + thin binary
├── src/main.rs                           # entry point (library-shaped project)
├── crates/
│   ├── boyko_ecs/                        # ECS core
│   │   ├── Cargo.toml                    # deps: boyko-utils, boyko-threadpool, fixedbitset,
│   │   │                                 #       crossbeam-queue, crossbeam-utils, static_assertions,
│   │   │                                 #       optional mimalloc; unix→libc. (boyko-macros is DEV-only)
│   │   └── src/
│   │       ├── lib.rs                    # pub mod ecs; pub mod prelude; re-exports App/Plugin/EcsError
│   │       ├── prelude.rs                # curated re-exports (NO derives — macro-cycle)
│   │       └── ecs/
│   │           ├── mod.rs                # core, memory, error, constants, identifiers
│   │           ├── constants.rs          # sizes / thresholds / SIMD-align (capacities live by their registries)
│   │           ├── error.rs              # EcsError / EcsResult (no anyhow)
│   │           ├── identifiers/primitives.rs   # EntityId / ArchetypeId / ComponentId / … newtypes
│   │           ├── memory/
│   │           │   ├── vm.rs                 # VmReservation — the per-OS reserve/commit primitive (X.G; sole backing since X.J)
│   │           │   ├── component_pool.rs     # type-erased SELF-GROWING pool, [data|added|changed] reservation (X.I)
│   │           │   └── utils.rs              # align_up  (arena.rs + free_mem_block.rs DELETED in X.J)
│   │           └── core/
│   │               ├── app/              # App builder + Plugin + Plugins + AppExit (Phase 18); multi-schedule frame driver Main+Fixed (Phase 20)
│   │               ├── archetype/        # Archetype (inline columns), signature, registry, bundle slab, master
│   │               ├── bundle/           # Bundle trait + derive + type registry + column cache
│   │               ├── change_detection/ # Tick + check_ticks (Phase 10)
│   │               ├── commands/         # CommandQueue + Spawn/Insert/Remove/Despawn/Batch + migration_helpers
│   │               ├── component/        # Component trait, mask, registry, pool bundle, hooks/, observers/
│   │               ├── ecs_master/       # EcsMaster — top-level facade
│   │               ├── entity/           # Entity, EntityInland (slab ptr), EntityMaster
│   │               ├── iters/            # QueryState, ArchetypeBitSet, LegacyQuery, component_set, query/
│   │               ├── events/           # Event trait + dispatcher + buffer + registry + participants/parameters
│   │               ├── resources/        # Resources slab + Resource trait + registry
│   │               ├── schedule/         # Schedule + ScheduleBuilder + ConflictGraph + ordering + conditions
│   │               ├── state/            # States + State/NextState + transitions (Phase 17)
│   │               ├── time/             # Time + FixedTime + fixed_advance — fixed-timestep clock (Phase 20)
│   │               └── system/           # System/IntoSystem/FunctionSystem + SystemParam + params/
│   ├── boyko_macros/                     # proc-macros (DEV-dep of boyko_ecs)
│   │   └── src/lib.rs                    # #[derive(Component/Resource/Bundle/SystemSet)] + #[event]
│   ├── boyko_utils/                      # reusable collections
│   │   └── src/{bit_mask/, sparse_map/, identifiers/}
│   ├── boyko_threadpool/                 # Chase-Lev work-stealing pool (on crossbeam-deque)
│   │   └── src/{thread_pool, scope, worker, tls, sync}.rs
│   ├── boyko_demo/                       # wgpu+egui sandbox (dogfoods the API; wasm32-capable)
│   └── bench_bevy_vs_boyko/              # cross-engine comparison benches (pulls bevy)
└── docs/                                 # internal documentation
```

## Inter-crate dependencies

```
boyko-engine (thin binary)
    ├── boyko_ecs
    │       ├── boyko_utils
    │       └── boyko_threadpool ──→ crossbeam-deque, crossbeam-utils
    ├── boyko_macros ──→ boyko_ecs        (for the ::boyko_ecs::… paths the derives emit)
    └── boyko_utils

boyko_ecs (dev-dependencies): boyko_macros, criterion, proptest, trybuild, rand
boyko_demo ──→ boyko_ecs, boyko_macros, wgpu, eframe/egui
bench_bevy_vs_boyko ──→ boyko_ecs, boyko_macros, bevy, criterion
```

External runtime deps of `boyko_ecs`: `fixedbitset` (scheduler conflict/condition
bitsets), `crossbeam-queue` / `crossbeam-utils`, `static_assertions` (compile-time
`Send`/`Unpin` pins), optional `mimalloc` (bench-only), `libc` (Unix
`mmap`/`mprotect` for `VmReservation`). **`anyhow` and `ctor` were removed**
(C-019 / lazy-mint ID model).

> **The `boyko-macros` cycle (Phase 18).** `boyko_macros` depends on `boyko_ecs`
> (its derives expand to `::boyko_ecs::…` paths), so `boyko_ecs` can only keep it
> as a **dev-dependency** — a normal dependency would form a cycle. Therefore the
> derives are unusable inside `boyko_ecs` lib code (`AppExit` hand-impls
> `Resource`) and the public `prelude` omits them (users
> `use boyko_macros::{Component, …}` directly).

## Architecture layers

```
┌────────────────────────────────────────────────────────────────────┐
│  Layer 5: Game / User Code                                          │
└────────────────────────────────────────────────────────────────────┘
                                ↑
┌────────────────────────────────────────────────────────────────────┐
│  Layer 4: Application facade                                        │
│  App + Plugin + Plugins + AppExit (Phase 18)                       │
└────────────────────────────────────────────────────────────────────┘
                                ↑
┌────────────────────────────────────────────────────────────────────┐
│  Layer 3: Scheduling & systems                                     │
│  Schedule / ScheduleBuilder / ConflictGraph (Tarjan+Kahn)         │
│  System / IntoSystem / FunctionSystem / SystemParam               │
│  Commands · States · run conditions · ordering/sets                │
│  boyko_threadpool (Chase-Lev work-stealing + Scope)               │
└────────────────────────────────────────────────────────────────────┘
                                ↑
┌────────────────────────────────────────────────────────────────────┐
│  Layer 2: ECS API                                                  │
│  EcsMaster · typed Query<D, F> (+ par_iter / for_each_chunk)      │
│  Bundle · Resources · EventDispatcher / EventReader / EventWriter │
│  change detection (Tick / Added / Changed / Ref / Mut)            │
│  component hooks & observers                                       │
└────────────────────────────────────────────────────────────────────┘
                                ↑
┌────────────────────────────────────────────────────────────────────┐
│  Layer 1: ECS Core                                                 │
│  Entity / EntityMaster / EntityInland (slab ptr)                  │
│  Archetype (inline columns) / ArchetypeMaster / Registry / slab   │
│  Component (trait + derive) / ComponentMask / ComponentRegistry   │
│  Event (trait + derive) / Participants / Parameters               │
└────────────────────────────────────────────────────────────────────┘
                                ↑
┌────────────────────────────────────────────────────────────────────┐
│  Layer 0: Type-Erased Memory  +  Utils (boyko_utils)              │
│  VmReservation (reserve/commit) → ComponentPool (type-erased,     │
│       self-growing, row = buffer+i*stride, + per-row Tick columns)│
│  BitSet<T> / BitSet256 · SparseMap / SparseSlotMap · Slot · IDs   │
└────────────────────────────────────────────────────────────────────┘
```

## Data flow: spawning an entity (direct API)

```
User → EcsMaster::create_entity(archetype_id, &[(ComponentId, &[u8])])
    ├─ Guard (C-007): archetype_master.has_archetype(archetype_id)?
    │      └─ no → Err(EcsError::ArchetypeNotFound) before any allocation
    ├─ EntityMaster::allocate_entity()
    │      └─ recycle from free_entity_ids, or fetch_add(next_entity_id)
    │         → Entity { id, generation }
    ├─ ArchetypeMaster::get_archetype_mut(id) → &mut Archetype
    ├─ Archetype::create_entity(entity_id, &mut inland, components)
    │      ├─ Two-phase commit (C-009): bundle.can_push_entity_components(...)
    │      │      └─ false → entity_master.rewind_allocate(entity);
    │      │                  Err(EcsError::ArchetypeRejectedEntity)
    │      ├─ bundle.push_entity_components(...) — lockstep memcpy into the pool columns
    │      └─ fill the per-row added_ticks/changed_ticks with the world's tick
    └─ EntityMaster::register_entity_with_ptr(entity, &mut *archetype, row)
              └─ write the EntityInland fast-store slot { archetype_ptr, row, gen };
                 live_count += 1
```

Failure-path correctness (C-007 + C-009): the guard means no EntityIDs leak on
early rejection; the two-phase commit makes a partial push structurally
impossible (`can_push` walks every pool read-only first, then `push` runs only
after a universal go-ahead).

After registration, the same structural op fires component lifecycle **hooks**
(Phase 14a) then **observers** (Phase 14b) for each `add` / `insert` kind — gated
by the archetype's `ArchetypeFlags` `u16` bit-test (a world with no callback pays
one `test`/`jz`). The symmetric `replace` / `remove` fire on despawn / migration.
The deferred (`Commands`) spawn/insert/remove paths fire at the same kinds from
their apply sites, and the Phase-22 dynamic-tag migration paths
(`migrate_entity_attach_ids` / `migrate_entity_detach_ids` / `retag_in_place`)
joined the ledger. Full catalog (registry, the 4 cold `fire_*_observers` dispatch
fns, the **10** fire sites, and the OBS-FIRE-LOOP Tree-Borrows invariant):
[SYSTEMS.md §3.6](SYSTEMS.md).

## Data flow: a parallel frame (`Schedule::run` / `App::run`)

Since Phase 20 the `App` frame driver (`App::update_with_delta`) wraps 1..N of
these runs per frame in the binding D1 order: ① `Time::advance_with` →
② margin-aware all-schedule check-ticks pass (`CHECK_TICK_PREEMPT_MARGIN`) →
③ gated event swap (`EventUpdatePolicy`) → ④ `fixed_advance` catch-up loop
(0..16 Fixed `Schedule::run`s at the defaults) → ⑤ the Main run below. Each
run stays an opaque unit; all inter-run work holds the dispatcher's
`&mut EcsMaster` with zero workers in flight. Driver cost: 14 ns/frame +
5 ns/substep ([PHASE-20-RESULTS.md](PHASE-20-RESULTS.md)).

```
App::run / Schedule::run(&mut world)
    ├─ world.change_tick.fetch_add(1)            # Phase 10: new this_run
    ├─ run_state_transitions(&mut world)         # Phase 17, gated by state_entries.is_empty()
    ├─ pool.install(|| {                         # boyko_threadpool dispatcher entry
    │     loop {                                 # dispatch rounds (Phase 9 §5.4)
    │       1. apply-window drain — if the gate proves running==0, run apply(&mut world)
    │          for every completed system (Commands flush, structural mutation, tick bump)
    │       2. all completed → break
    │       3. mint a fresh UnsafeEcsCell from &mut world (per round; O3)
    │       4. try_dispatch_ready — for each system with pred_remaining==0, not running,
    │          no conflict against running, and run_if conditions true:
    │            - exclusive (fn(&mut EcsMaster)): run inline on the dispatcher (EXC1)
    │            - concurrent: scope.spawn(worker runs run_unsafe(cell_copy))
    │       5. nothing dispatched & still running → park_timeout (100 µs backstop)
    │     }
    │  })
    └─ run_check_ticks_scan every CHECK_TICK_THRESHOLD ticks  # Phase 10 wraparound clamp
```

Per-system `last_run`/`this_run` ticks are written via `System::set_change_ticks`
(the Phase-10 C1 dispatcher→system channel). Run conditions are evaluated in a
separate `evaluate_ready_conditions` pass at the apply-window boundary
(`running==0`, race-free). Events sit OUTSIDE the conflict graph (Option A) — a
worker writes to its own per-`E` TLS lane; `update_events` swaps lanes at the
frame boundary.

**Aliasing discipline (Phase 9 SCH7 / ALLOC1):** `&mut EcsMaster` (used by
`apply`) is held only inside the apply window, where the gate proves no worker
cell aliases it. All structural growth (pool/store frontier commits, container
growth) happens on the dispatcher or during `ScheduleBuilder::build`, never
from a worker — the `IN_SYSTEM_RUN` TLS flag (ALLOC1) lets context-restricted
paths `debug_assert!` the discipline.

## Key architectural decisions

### 1. Type-erased `ComponentPool` (dropping generic `<T>`)

**Where:** [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs)

The pool stores **raw bytes** (`buffer: NonNull<u8>`) + the `Layout` from the
global `ComponentRegistry`, not a generic `<T>`. An archetype holds many
*different* component types, so a `Vec<ComponentPool<?>>` would otherwise need
`Box<dyn>` or an enum. Type erasure via `Layout` is the standard approach (Bevy
`Table`, flecs `ecs_table_t`). Cost: `get`/`add` rely on the "ComponentId matches
the right type" invariant (a debug-only `TypeId` guard catches mismatches, C-004)
and access uses `unsafe { &*(ptr as *const T) }` with a SAFETY comment.

### 2. Computed row addressing + per-row tick columns

**Where:** [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs)

`master` used `UnitId { chunk, inland }` (two-level). `ecs` first cached a
`Unit { ptr }` per row; **Phase X.B eliminated that cache** (every entry equalled
`buffer + i*stride`) — rows are now `buffer.as_ptr().add(i * stride)` (the private
`row_ptr(i)`) from the pool's stable write-once reservation base, with `len` the
live-row count. Net-removes `unsafe`, saves 8 B/row + one alloc/pool, zero
read-path cost. **Phase 10** then added the parallel `added_ticks` /
`changed_ticks` columns (`Box<[UnsafeCell<Tick>]>`). See
[PHASE-XB-RESULTS.md](PHASE-XB-RESULTS.md).

### 3. Inline per-archetype column table (Phase 7 fast random access)

**Where:** [archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs)

`Archetype` places an 8 KB `columns: [Column; MAX_COMPONENTS]` table at offset 0
(`#[repr(C)]`, size pinned). `Column { ptr, stride }` is the pool's base pointer +
component size; a random component lookup is a single dependent load
`*(arch + c*16)` plus `ptr.add(row * stride)` — no `ComponentPoolBundle` sparse
map on the hot path. `EntityInland` stores a **direct `*mut Archetype` slab
pointer** so `get_component_raw` reaches the table without a `SparseMap`
indirection (~3 ns/lookup). The `ArchetypeBundle` slab gives every `Archetype` a
stable heap address so those pointers + per-`(D,F)` caches stay valid.

### 4. Reserve/commit virtual-memory backing (Phases X.C → X.F → X.G/X.H → X.I/X.J)

**Where:** [memory/vm.rs](../crates/boyko_ecs/src/ecs/memory/vm.rs)

Every storage owner backs itself with a `VmReservation`: a write-once
virtual-address reservation (`VirtualAlloc(MEM_RESERVE, PAGE_NOACCESS)` on
Windows via hand-declared FFI, `mmap(PROT_NONE)` on Unix via `libc`) committed
lazily in geometric frontier slabs (`MEM_COMMIT` / `mprotect(RW)`, demand-zero)
— **growth never reallocates, copies, or moves a base**. A
`cfg(any(miri, not(any(windows, unix))))` fallback arm eagerly `alloc_zeroed`s
the full reserve (commit = no-op) so Miri / wasm32 model the same bookkeeping;
`Drop` uses the per-cfg-arm matching deallocator (M-001). The lineage: X.C
taught the shared Arena lazy commit, X.F gave it the huge reserve + frontier
slabs, X.G extracted `VmReservation` for the entity `InlandStore`, X.I gave
every `ComponentPool` its own `[data|added|changed]` reservation — and X.J
**deleted the then-client-less shared Arena** (+ `MemFreeBlockMaster`)
outright. See [PHASE-XI-RESULTS.md](PHASE-XI-RESULTS.md) +
[PHASE-XJ-RESULTS.md](PHASE-XJ-RESULTS.md).

### 5. Global `ComponentRegistry` / `EventRegistry` / `ResourceRegistry` (lazy IDs)

**Where:** [component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs),
[events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs),
[resources/resource_registry.rs](../crates/boyko_ecs/src/ecs/core/resources/resource_registry.rs)

`static [OnceLock<…>; N]` storage; IDs minted lazily on the first
`T::component_id()` / `E::event_id()` / `R::resource_id()` call (per-type
`OnceLock<Id>` memoizes). This replaced the `static mut` races (M-002 / C-002)
AND the `#[ctor::ctor]` startup-registration model (the `ctor` dep is gone) with
its fragile global-init ordering. IDs are first-call order — stable per-process,
NOT across processes (external-ID consumers must warm up the registry at
startup); per-slot collisions panic in debug AND release. Type-vs-type
exclusivity (a type cannot be both Component and Resource, M6) is checked at
`register_resource_new`. Capacities: `MAX_COMPONENTS = 512`, `MAX_EVENTS = 256`,
`RESOURCE_SLOT_COUNT = 256`.

### 6. `EntityMaster` — recycling + direct fast store (Phase 7 + X.D)

**Where:** [entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs)

Four fields: `free_entity_ids` (LIFO recycle), `next_entity_id: AtomicUsize`,
`entities_inland: Vec<EntityInland>` (the hot fast store, indexed by `EntityId.0`,
`is_null()` ⇔ dead), and a plain `live_count: usize`. Phase 7 dropped the
`SparseMap<EntityInland>` indirection; **Phase X.D** dropped the EnTT-style
`active_ids` + `sparse_to_active` (their only consumer was the cold
`iter_entities`, and the despawn swap-remove they needed was deleted with them),
net-removing `unsafe` and shedding −12 B/entity. `Generation` bumps on
deallocation (the ABA defence). Workers touch only `next_entity_id` (via the
`EntityCounter` atomic-RMW newtype); all other mutation is dispatcher-`&mut self`
inside the apply window. See [PHASE-XD-RESULTS.md](PHASE-XD-RESULTS.md).

### 7. Domain error type `EcsError`

**Where:** [error.rs](../crates/boyko_ecs/src/ecs/error.rs)

`#[non_exhaustive] pub enum EcsError` + `pub type EcsResult<T>` replaced the
historical `anyhow::Result` (C-019), so callers can pattern-match concrete
variants. Hand-rolled `Display` + `std::error::Error` (no `thiserror`).
`#[non_exhaustive]` keeps the door open for new variants without a major bump.

### 8. Newtype identifiers

**Where:** [identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs)

Every core ID is a `#[repr(transparent)] X(usize)` newtype via one `define_id!`
macro (C-017) — zero runtime cost, but mixing them is a type error.
`Generation = usize` stays an alias (only paired with `EntityId`). Subsystem-local
sizing IDs (`ResourceId`, `BundleTypeId`, `QueryTypeId`, `ObserverId`,
`SystemSetId`) live next to their owners.

### 9. Dual-generation QueryState (ABA-safety across remove/clear)

**Where:** [archetype/archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs),
[iters/query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs)

`ArchetypeMaster` carries `generation` (bumps on `create_archetype`) +
`structural_generation` (bumps on `remove_archetype` / `clear`). `QueryState`
snapshots both: a structural mismatch drops the dedup bitset + `matched_ids` and
fully rebuilds; a creation-only delta classifies just the new IDs; both equal =
warm slice walk. Without `structural_generation`, a recycled `ArchetypeId` (after
`clear` resets `next_archetype_id`) could silently leak into a stale query. The
per-`(D,F)` typed cache `QueryDataState<D, F>` wraps this (Phase 8b), interned by
`QueryTypeId`. (Phase 5c.)

### 10. Parallel scheduler on a custom thread pool (Phase 9 + 9.x soundness)

**Where:** [schedule/](../crates/boyko_ecs/src/ecs/core/schedule/),
[boyko_threadpool/](../crates/boyko_threadpool/)

A Bevy-class executor: `ScheduleBuilder` builds a `ConflictGraph` (per-system
`Access` bitsets) + an ordering DAG (Tarjan SCC + Kahn topo), and `Schedule::run`
dispatches non-conflicting systems concurrently on the custom Chase-Lev
work-stealing pool (`boyko_threadpool`, built on `crossbeam_deque`). The
apply-window barrier (C4) keeps `&mut EcsMaster` (used by `apply`) from aliasing
any worker `UnsafeEcsCell`. `EcsMaster` / `UnsafeEcsCell` / `Resources` /
`EntityMaster` / `ArchetypeMaster` / `EventDispatcher` / `ComponentPool` /
`Archetype` carry explicit `unsafe impl Send + Sync` gated by the aliasing +
no-alloc-in-system contracts; structural allocation stays restricted to the
dispatcher + build (ALLOC1 TLS discipline; the shared `!Send + !Sync` Arena
that anchored the historical wording was retired in X.J — the SEND1
justification is updated in place). The whole pool + `Scope`
fork/join + parallel `Schedule::run` is proven sound and Tree-Borrows-clean
(Phase 9.1/9.2/9.3 — loom + Miri). See
[PHASE-9-PARALLEL-SCHEDULER-PLAN.md](PHASE-9-PARALLEL-SCHEDULER-PLAN.md),
[PHASE-9.2-RESULTS.md](PHASE-9.2-RESULTS.md), [PHASE-9.3c-RESULTS.md](PHASE-9.3c-RESULTS.md).

### 11. Bevy-shape ergonomic system API (Phases 8a–8d, 11, 12, 13)

**Where:** [system/](../crates/boyko_ecs/src/ecs/core/system/),
[commands/](../crates/boyko_ecs/src/ecs/core/commands/)

GAT-based `SystemParam` (two-phase `init_state` + `init_access` conflict
detection), `Res`/`ResMut`/`Local`/`Query`/`Commands`/`EventReader`/`EventWriter`
leaves + tuples 0..=12, `IntoSystem`/`FunctionSystem` (function-as-system, no
turbofish), and a per-system byte-arena `Commands` queue flushed via
`SystemParam::apply` (no `Box<dyn Command>`, no per-command alloc). Entity-id
reservation uses an atomic counter so `commands.spawn(b).insert(x).id()` chains
(Phase 11). The `UnsafeEcsCell` worker cell is `Copy` with by-value receivers
(Phase 8a C1 — no `&self` retag).

### 12. Change detection, hooks/observers, states, conditions

Layered on the above without disturbing the hot path:
- **Change detection** (Phase 10): per-row `Tick` columns + `Added`/`Changed`
  filters + `Ref`/`Mut`; 0% overhead when unused (`NEEDS_CHANGE_DETECTION` const
  elision).
- **Hooks & observers** (Phases 14a/14b): reactive callbacks at the 4 structural
  kinds, gated by the per-archetype `ArchetypeFlags` u16 (0% when unused).
- **States** (Phase 17): `State`/`NextState` + `in_state`/`on_enter`/… conditions
  + a built-in transition pass (0% when no state registered).
- **Run conditions** (Phase 16) + **ordering/sets** (Phase 15): `.run_if`,
  `before`/`after`/`in_set`, `#[derive(SystemSet)]`; both designed around the
  executor's 0%-gate (a `has_condition` bitset / a `state_entries.is_empty()`
  early-out, leaving the no-feature dispatch path byte-identical).

### 13. Tags share the ComponentId space; storage is tick-only (Phase 22)

**Where:** [component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs)
(`TagId` :214, mint protocol :496-609 — the planned `identifiers/tag_id.rs`
was NOT created, a recorded deviation),
[ecs_master/tag_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/tag_api.rs),
[memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs),
[query/tag_terms.rs](../crates/boyko_ecs/src/ecs/core/iters/query/tag_terms.rs),
[core/iters/query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs).

Three load-bearing choices:

- **Dynamic tags are ordinary `ComponentId`s** (name-keyed mint into the shared
  512-slot registry, sentinel TypeId). A separate id namespace would have grown
  `Archetype` past its 8480 B pin and forked the hottest matching code; sharing
  means mask build, `find_exact_match`, hooks, observers and migration work
  unmodified (Bevy precedent). One-way public bridge
  `TagId::component_id() -> ComponentId` reaches the id-keyed hook/observer
  surfaces.
- **ZST storage keeps the tick pair (8 B/row)** in a data-less pool rather than
  going signature-only (flecs-style 0 B/row): `Added<Tag>`/`Changed<Tag>` stay
  genuinely functional with zero filter changes — the 0-byte alternative is a
  compile-but-lie (the #56 bug class). Columns stay non-null (dangling aligned
  base), preserving the Phase-7 single-dependent-load read path.
- **Runtime query terms filter at archetype granularity through one funnel**:
  per-view `TagTerms` (never the shared interned `QueryState` — QS1 stays
  term-agnostic) + the `_pre_terms` rename sweep over every matched-list
  accessor, so an un-migrated driver fails to compile instead of silently
  bypassing terms (the Phase-14b enumeration-by-memory lesson, structurally
  enforced). Cost: one predicted branch per archetype transition; the inner
  row loop is byte-identical.

Entities may hold zero components (the lazy EMPTY archetype; remove-last is an
ordinary migration edge). Accepted ceilings, all loud: 512 shared ids, 2^N tag
combinations within `MAX_ARCHETYPES = 1024` (the fragmentation mitigation —
the EnableTag enable-bit backend — landed; see decision 14), 8 dynamic terms,
bundle arity 16. Each tag pool reserves 128 MiB of address space per hosting
archetype (2 MiB cfg fallback) — zero resident until commit.

### 14. EnableTag — the enable-bit, non-fragmenting tag backend

**Where:** [component/enable/](../crates/boyko_ecs/src/ecs/core/component/enable/)
(`enable_store.rs` = `EnablePage`/`EnableColumn`/`EnableStore`, `enable_presence.rs`
= the `EnablePresence` cull oracle),
[component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs)
(`StorageKind` + `STORAGE_KIND` table + `EnableTagId`),
[ecs_master/enable_tag_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs),
[query/filter_enable.rs](../crates/boyko_ecs/src/ecs/core/iters/query/filter_enable.rs)
+ [query/enable_terms.rs](../crates/boyko_ecs/src/ecs/core/iters/query/enable_terms.rs).
Design: [ENABLE-TAG-PLAN.md](ENABLE-TAG-PLAN.md) +
[ENABLE-TAG-PLAN-AMENDMENT-D7.md](ENABLE-TAG-PLAN-AMENDMENT-D7.md).

The **second tag storage path** alongside decision 13's signature/table backend.
A component id is classified once at registration as `StorageKind::{Table,
Bitset}` (a cold parallel `[AtomicU8; 512]`, kept out of the 56 B
`ComponentLayout`). A `Bitset` (EnableTag) id is filtered out of every archetype
signature and owns no `ComponentPool`; its presence is a single per-row bit in a
paged per-archetype bitset (`EnablePage` = 512 B / 4096 rows, the bit's home is
`(archetype, row)` so it rides the existing swap-remove / migration loops). Three
load-bearing choices:

- **Toggle is O(1), not a migration** (flecs `CanToggle`): `enable`/`disable`
  flip one `AtomicU64` bit — no archetype migration, no `structural_generation`
  bump, no hook/observer fire, no deferred drain. This is the fragmentation
  answer for high-churn transient flags. The trade-off is no per-row tick
  storage, so `Added<T>`/`Changed<T>` are compile-rejected on a bitset tag
  (the decision-13 "compile-but-lie" guard, via `Component::STORAGE_IS_BITSET`).
- **The cull oracle is a bounded `contains`, never a driver**: `EnablePresence`
  (per-world, one lazily-published `Box<[AtomicU64; 16]>` per tag) answers
  O(1) "does archetype A own a column for tag T?" over an *already-bounded*
  matched set. There is deliberately no presence-driven enumeration — a sole
  `Enabled<T>` with no positive bound is candidate-seeded from a *bounded*
  presence snapshot (the D7 global scan), and `ASSERT_SHAPE` const-rejects the
  unbounded shape. A lock-free `epoch` / `enable_generation` pair invalidates
  the culled set when an archetype gains a column.
- **v1 is `&mut`-exclusive; the atomics reserve the parallel seam**: a toggle
  takes `&mut EcsMaster`, which makes the `Relaxed` enable-bit / `enable_generation`
  stores sound with no live worker (decision-10 apply-window discipline). The
  `AtomicU64` words and the `EnablePresence` `Acquire`/`Release` epoch are the
  forward seam for the deferred D7 `&self` worker-marking toggle (which must add
  a loom proof before relaxing the receiver). Full catalog +
  invariants: [SYSTEMS.md §3.8](SYSTEMS.md).

## Pool sizing (Phase X.I — the size classes are gone)

The `master`-era size classes (`TINY/…/LARGE_COMPONENTS_PER_CHUNK`) and the
chunk machinery are DELETED. A pool's ceiling is byte-targeted:
`reserve_rows(stride) = clamp(POOL_TARGET_DATA_BYTES / stride, POOL_MIN_ROWS,
POOL_MAX_ROWS)` (1 GiB / 2^16 / 2^24 on syscall arms — see
[constants.rs](../crates/boyko_ecs/src/ecs/constants.rs)); committed memory
grows in doubling slabs on demand and never moves. Pool backing buffers are
lifted to `SIMD_BUFFER_ALIGN = 32` (Phase X.A) so column starts are
AVX2-loadable (trivially satisfied — every reservation base is ≥ 4096-aligned).

## Multi-threading model (current state)

1. **Read-heavy parallelism**: non-conflicting systems run concurrently; the
   conflict graph (derived from each system's declared `Access`) guarantees the
   absence of aliasing component access. Achieved (Phase 9).
2. **Partitioned writes within one system**: `Query::par_iter` /
   `par_for_each_chunk` fan archetype rows/subranges across workers via
   `boyko_threadpool::scope`.
3. **Lock-free infrastructure**: registries are `static [OnceLock; N]` +
   `AtomicUsize`; the pool is Chase-Lev work-stealing.

Constraints, by construction:
- Structural allocation (frontier commits, container growth) only on the
  dispatcher / at build (ALLOC1 TLS discipline).
- Structural mutation (`apply`, archetype growth, entity-vec growth) runs only in
  the apply window where `running == 0`.
- Events route per-worker-TLS lane; `update_events` swaps at the frame boundary.

## Performance posture (vs Bevy, measured)

Per [PHASE-12.6-RESULTS.md](PHASE-12.6-RESULTS.md) +
[PHASE-X.A-RESULTS.md](PHASE-X.A-RESULTS.md), on the comparison harness in
[crates/bench_bevy_vs_boyko/](../crates/bench_bevy_vs_boyko/):

| Workload | Result |
|----------|--------|
| 50 empty systems (scheduler dispatch) | **~1.72× faster than Bevy** |
| `par_iter` 10k | **~2.93× faster than Bevy** |
| query iter (direct API) | **~parity** (inner loop byte-identical in asm) |
| `for_each_chunk` 3-component reduction (native SIMD) | **~1.28–1.34× faster** |
| `EcsMaster::new` | ~4.24 µs (was 712 µs — the 12.6/X.C/X.F lazy-init chain + the X.J arena retirement) |
| spawn (single / batch) | structurally constrained vs Bevy on single; batch warm path ~1.35× of Bevy |

Bench methodology (deterministic `[profile.bench] codegen-units = 1`, opt-in
`bench-alloc` mimalloc, median-of-N `bench.ps1`) is in
[BENCHMARKING.md](BENCHMARKING.md) (Phase X.E).

## What differs from the `master` branch

| Aspect | master | ecs |
|--------|--------|-----|
| ComponentPool | `ComponentPool<T>` (generic, fixed) | type-erased + `ComponentRegistry`, SELF-GROWING per-pool `VmReservation [data\|added\|changed]` (X.I) |
| Chunk | `Chunk<T>` stores data | DELETED (X.I) — vestigial metadata, written-never-read |
| Row addressing | `UnitId { chunk, inland }` | computed `buffer + i*stride` (`Unit` cache removed Phase X.B) |
| Random component access | two-level lookup | inline per-archetype `Column` table at offset 0 (Phase 7) |
| Memory backing | shared `Arena` on global `alloc` | per-pool `VmReservation` reserve/commit (`VirtualAlloc`/`mmap`, X.G/X.I); the shared Arena RETIRED in X.J |
| Entity ID | `u32` + gen `u16` | newtype `EntityId(usize)` + gen `usize` (C-017) |
| EntityMaster | ⚠️ missing | ✅ recycling + direct `Vec<EntityInland>` slab-ptr fast store (Phase 7 + X.D) |
| Archetype / EcsMaster | ⚠️ stub | ✅ full implementation |
| Query | ⚠️ missing | ✅ typed `Query<D, F>` DSL + par_iter + for_each_chunk + change filters |
| Systems / scheduler | ❌ none | ✅ SystemParam + IntoSystem + Commands + parallel Schedule (Phase 8/9) |
| States / conditions / ordering | ❌ none | ✅ States + `.run_if` + sets/ordering (Phases 15/16/17) |
| Hooks / observers | ❌ none | ✅ lifecycle hooks + observers (Phases 14a/14b) |
| Change detection | ❌ none | ✅ Tick / Added / Changed / Ref / Mut (Phase 10) |
| Events | ⚠️ registry only | ✅ full double-buffered dispatcher + EventReader/EventWriter (Phases 6/12) |
| App facade | ❌ none | ✅ App + Plugin (Phase 18) |
| Thread pool | ❌ none | ✅ `boyko_threadpool` (Chase-Lev, loom+Miri-proven) |
| Error type | `anyhow::Result` | domain `EcsResult` (C-019) |
| boyko_utils | ⚠️ missing | ✅ BitSet / BitSet256 / SparseMap / SparseSlotMap / Slot |
| Build | ✅ builds | ✅ builds clean (check + clippy + test + Miri) |
