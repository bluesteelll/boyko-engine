# Feature map — where to find what (branch `ecs`)

First point of contact for agents. When you need to know *where* a particular
piece of functionality lives, start here, then go to
[SYSTEMS.md](SYSTEMS.md) for details and finally to the source.

**Legend:**
- ✅ Implemented and tested
- ⚠️ Implemented with documented caveats
- 📋 Planned / filed as a future phase
- ❌ Not implemented (deliberately — see linked rationale)

> The `ecs` branch builds clean. The current state is the cumulative result of
> Phases 2 → 19 plus the 9.x executor-soundness series, the X.x perf series
> (X.A `for_each_chunk`, X.B `Unit` removal, X.C arena lazy-commit, X.D
> EntityMaster slot reduction, X.E bench methodology), Phases 14a/14b
> (hooks + observers), and Phase 19 (parent-child hierarchies on the hook
> substrate). Each phase's authoritative record is its
> `docs/PHASE-*-RESULTS.md`. Line numbers below are verified against the
> current source; if one drifts, the file path is still correct.

> **Crate layout:** `boyko_ecs` (core) · `boyko_macros` (derives) ·
> `boyko_utils` (collections) · `boyko_threadpool` (Chase-Lev work-stealing
> pool, on crossbeam-deque primitives) · `boyko_demo` (wgpu+egui sandbox,
> dogfoods the public API) · `bench_bevy_vs_boyko` (comparison benches).

---

## Quick "I want to …" index

| I want to … | Go to |
|-------------|-------|
| Build an app with a frame loop | [App + Plugin facade](#app--plugin-facade) |
| Define a component / resource / bundle / event / system-set | [Macros](#macros-derives) |
| Spawn / despawn / mutate entities directly | [EcsMaster facade](#high-level-facade-ecsmaster) |
| Spawn / despawn deferred (inside a system) | [Commands](#commands--entitycommands-deferred-mutation) |
| Iterate entities with components | [Typed Query DSL](#typed-query-dsl-queryd-f) |
| SIMD/batched columnar iteration | [for_each_chunk](#chunked--parallel-iteration) |
| Run systems in parallel | [Schedule + scheduler](#schedule--parallel-scheduler) |
| Order systems / group into sets | [Ordering & sets](#system-ordering--sets) |
| Conditionally run systems | [Run conditions](#run-conditions-run_if) |
| Application states / state machines | [States](#states) |
| React to component add/remove | [Hooks & observers](#component-lifecycle-hooks--observers) |
| Parent-child hierarchies | [Hierarchies](#hierarchies-parent-child) |
| Detect changed/added components | [Change detection](#change-detection-tick--addedt--changedt) |
| Send/read events between systems | [Events](#events) |
| Shared global data | [Resources](#resources) |
| Low-level component byte storage | [Type-erased component storage](#type-erased-component-storage) |
| Allocate raw memory | [Memory and allocation](#memory-and-allocation) |

---

## High-level facade (EcsMaster)

The world object. Owns the entity manager, archetype manager, arena, resources,
event dispatcher, change-detection tick, the deferred-hook queue, and the
per-`(D,F)` query/bundle caches.

**File:** [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs)

| What you want to do | Method (line) |
|---------------------|---------------|
| Construct an ECS instance | `EcsMaster::new()` (409) / `with_capacity(entity_cap, arch_cap)` (465) |
| Create an archetype | `create_archetype(&[ComponentId])` (544) / `get_or_create_archetype(...)` (551) |
| Spawn (raw byte API) | `create_entity(arch_id, &[(ComponentId, &[u8])]) -> EcsResult<Entity>` (584) |
| Spawn (typed, 1–2 comps) | `spawn_one::<A>(arch, a)` (965) / `spawn_two::<A, B>(arch, a, b)` (1001) |
| Spawn many (typed bundle) | `spawn_batch::<B, I>(iter) -> EcsResult<Vec<Entity>>` (2535) |
| Delete an entity | `delete_entity(entity) -> bool` (1119) |
| Read a component (raw) | `get_component_raw(entity, id)` (1209) |
| Mutate a component (change-tracked) | `get_component_mut::<T>(entity) -> Option<Mut<'_, T>>` (1386) |
| Write a component (raw bytes) | `set_component_raw(entity, id, &[u8])` (1305) |
| Check entity / component presence | `has_entity` (1449) / `has_component` (1467) |
| Counts | `entity_count` (1504) / `archetype_count` (1510) |
| Iterate entities (cold inspection) | `iter_entities()` (1522) — O(capacity) fast-store scan |
| Query entity IDs by components | `query_entities(&[ComponentId]) -> Vec<Entity>` (1527) — allocates; prefer the typed `Query` |
| Direct typed query (no SystemParam) | `query::<D, F>() -> QueryView<'_, D, F>` (2614) |
| Run a closure as a system once | `run_system::<F, M, Out>(system) -> Out` (1821) |
| Run a pre-built cached `System` | `run_cached_system::<S>(&mut system) -> S::Out` (1853) |
| Resources | `insert_resource::<R>` (1957) / `resource::<R>() -> &R` (2140) (+ `_mut`) |
| Hooks (runtime) | `register_component_hooks::<C>() -> ComponentHooksBuilder` (2016) |
| Observers | `observe_on_{add,insert,replace,remove}::<C>(runner)` (2065/2073/2083/2092), `add_observer` (2104), `remove_observer` (2121) |
| States (direct) | `insert_state` (2225) / `init_state` (2243) / `state::<S>()` (2254) / `set_next_state` (2277) |
| Events (direct) | `send_event::<E>(thread_index, event)` (1710) / `update_events()` (1728) |
| Drop everything | `clear()` (2740) |

Spawn / fallible paths return `EcsResult<T>` — see
[core/error.rs](../crates/boyko_ecs/src/ecs/error.rs) for the
`#[non_exhaustive] enum EcsError` (C-019 closed: the historical `anyhow`
dependency is gone). The two-phase commit pattern (C-007 + C-009) guarantees a
failed spawn leaks no EntityIDs.

---

## App + Plugin facade

Builder over `EcsMaster` + `ScheduleBuilder` + `Schedule` + `ThreadPool`
(Phase 18). `App::new().add_plugins(..).add_systems_cfg(..).run()`. Re-exported
at the crate root: `boyko_ecs::{App, Plugin, Plugins, AppExit}`.

**Files:** [core/app/](../crates/boyko_ecs/src/ecs/core/app/) —
`app.rs`, `plugin.rs`, `plugins.rs`, `app_exit.rs`.

| What you want to do | Where | Method (line) |
|---------------------|-------|---------------|
| Construct an app | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `App::new()` (80) / `with_threads(n)` (87) / `with_pool(Arc<ThreadPool>)` (93) |
| Add a plugin / plugin tuple | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `add_plugin::<P>` (224); `add_plugins((A, B, ..))` via the sealed `Plugins` trait ([plugins.rs](../crates/boyko_ecs/src/ecs/core/app/plugins.rs), 1..=12 + nesting) |
| Insert a resource / state | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `insert_resource` (117) / `init_state` (123) / `insert_state` (136) |
| Add systems (ordered) | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `add_systems_cfg(\|b: &mut ScheduleBuilder\| …)` (162) — full Phase-15/16/17 chaining |
| Add a system (unordered) | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `add_systems(system)` (180) |
| Add a one-shot startup system | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `add_startup_system(system)` (199) — runs once before the loop |
| Run the loop | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `run()` (329) (until `AppExit(true)`), `run_n(frames)` (303), `update()` (287) |
| The plugin trait | [plugin.rs](../crates/boyko_ecs/src/ecs/core/app/plugin.rs) ✅ | `trait Plugin { fn build(&self, &mut App); fn name(&self) -> &'static str }` — `'static`, NOT `Send + Sync`; consumed at build |
| Exit signal | [app_exit.rs](../crates/boyko_ecs/src/ecs/core/app/app_exit.rs) ✅ | `AppExit(bool)` resource (hand-impls `Resource` — see [PHASE-18-RESULTS.md](PHASE-18-RESULTS.md) macro-cycle note) |

`App` is `!Send + !Sync` (single-threaded-owned). DEFERRED: SubApps,
`PluginGroup`/`DefaultPlugins`, multi-schedule label map, `set_runner` — see
[PHASE-18-RESULTS.md](PHASE-18-RESULTS.md).

---

## Macros (derives)

**File:** [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs).
`boyko_macros` is a **dev-dependency** of `boyko_ecs` (cycle constraint, Phase
18) — import derives directly: `use boyko_macros::{Component, Resource, Bundle, SystemSet};`.

| Macro | What it generates |
|-------|-------------------|
| `#[derive(Component)]` ✅ | `Component` impl (lazy `component_id()` via per-type `OnceLock`) + inherent `SIZE`/`ALIGN`/`layout()` consts. Optional `#[component(on_add = path, …)]` binds Phase-14a lifecycle hooks (mutually exclusive with the runtime builder). |
| `#[derive(Resource)]` ✅ | `Resource` impl (lazy `resource_id()`); panics if the type is already a `Component` (audit M6). |
| `#[derive(Bundle)]` ✅ | `Bundle` impl over a named struct (sealed; `Send + Sync + Unpin + 'static`). Tuple bundles were dropped in Phase 8.5 — named structs only. |
| `#[derive(SystemSet)]` ✅ | `SystemSet` impl for fieldless enums (variant → discriminant). Data-carrying variants / unions / generics rejected (Phase 15). |
| `#[event]` ✅ | Rewrites a user struct with `#[participant(...)]` / `#[parameter]` fields into a two-field `{ participants, parameters }` native layout + `Event` impl. |

---

## Entities

| What you want to do | Where | How |
|---------------------|-------|-----|
| Construct an Entity literal | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ | `Entity::new(id, generation)` / `with_id(id)` |
| Compare entities (id + generation) | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ | `e1 == e2` — compares BOTH fields (load-bearing ABA defence) |
| Allocate an entity (recycle if available) | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::allocate_entity()` (102) — recycles from `free_entity_ids`, else `fetch_add` on the atomic |
| Register into the fast store | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `register_entity_with_ptr(entity, *mut Archetype, row)` / `register_batch(...)` |
| Validate an entity (gen-checked) | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `is_entity_valid(entity)` / `get_entity(id)` |
| Deallocate (bumps generation) | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `deallocate_entity(entity) -> bool` (decrements `live_count` on success only) |
| Iterate only LIVE entities | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `iter_entities()` — **O(capacity)** scan of `entities_inland`, skips `is_null()` slots (cold/inspection API; Phase X.D removed the dense `active_ids` index) |

`EntityMaster` (Phase 7 + X.D + X.G) is four fields (`#[repr(C)]`, hot cluster
on cache line 0): `entities_inland: InlandStore` (the hot fast store, indexed
by `EntityId.0`, `is_null()` ⇔ dead — since Phase X.G an address-stable
reserve/commit store: lazy 1 GiB reservation, frontier slab commits, growth
copies/writes NOTHING), `next_entity_id: AtomicUsize`, `live_count: usize`,
`free_entity_ids`. The fast-store record is
[`EntityInland`](../crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs)
= 16 B `{ archetype_ptr: *mut Archetype, unit_index: u32, generation: u32 }`
— a **direct slab pointer** (no `SparseMap` indirection on the hot read path);
`NULL` is all-zero bytes (demand-zero pages = free NULL fill, invariant J).
See [SYSTEMS.md §4](SYSTEMS.md) + [PHASE-XD-RESULTS.md](PHASE-XD-RESULTS.md) +
[PHASE-XG-RESULTS.md](PHASE-XG-RESULTS.md).

The `id`/`generation` pair is the ABA defence at the entity layer.
`SparseSlotMap` (boyko_utils) has a parallel slot-layer ABA fix (M-016).

---

## Components

| What you want to do | Where | How |
|---------------------|-------|-----|
| Define a component type | [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) ✅ | `#[derive(Component)] struct MyComp { … }` |
| Get the unique ID | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::component_id() -> ComponentId` (lazy, per-type `OnceLock`) |
| Size / align / layout / type id / name | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::SIZE` / `ALIGN` / `layout()`; trait `mem_size()` / `alignment()` / `type_id()` / `debug_type_name()` |
| Fetch a layout from the registry | [core/component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs) ✅ | `get_layout(id)`, `get_layout_unchecked(id)` |
| Register a layout explicitly (escape hatch) | [core/component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs) ✅ | `register_new::<T>()` (production) / `register_layout::<T>(id)` (test) |
| Build a ComponentMask | [core/component/component_mask.rs](../crates/boyko_ecs/src/ecs/core/component/component_mask.rs) ✅ | `ComponentMask::new()` + `set(id)` |
| Pools for one archetype | [core/component/component_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs) ✅ | `ComponentPoolBundle` (two-phase `can_push_*` + `push_*`) |
| ZST components | — | ❌ rejected at `ComponentPool::new` (`debug_assert!(size > 0)`); also ZST resources/events rejected at compile time. |

ID assignment (C-003): a per-type `OnceLock<ComponentId>` caches
`register_new::<Self>()` (first call mints from a global `AtomicUsize`, also
registering the `Layout`). **IDs are unstable across processes** — external-ID
consumers must warm up the registry at startup. `MAX_COMPONENTS = 512`
([component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs):50).

---

## Bundles (typed multi-component spawn payloads)

**Files:** [core/bundle/](../crates/boyko_ecs/src/ecs/core/bundle/) —
`bundle.rs`, `bundle_type_registry.rs`, `bundle_column_cache.rs`.

| What you want to do | Where | How |
|---------------------|-------|-----|
| Define a bundle | [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) ✅ | `#[derive(Bundle)] struct SpawnBundle { pos: Position, vel: Velocity }` |
| The bundle trait | [bundle/bundle.rs](../crates/boyko_ecs/src/ecs/core/bundle/bundle.rs):183 ✅ | `trait Bundle: BundleSealed + Send + Sync + Unpin + 'static` — `component_ids()`, `for_each_component_bytes(FnMut)` |
| Per-bundle-type ID | [bundle/bundle_type_registry.rs](../crates/boyko_ecs/src/ecs/core/bundle/bundle_type_registry.rs) ✅ | `BundleTypeId` (lazy); `MAX_BUNDLE_TYPES = 1024` (84) |
| Cached `(BundleType → ArchetypeId, columns)` | [bundle/bundle_column_cache.rs](../crates/boyko_ecs/src/ecs/core/bundle/bundle_column_cache.rs) ✅ | `BundleColumnCache` (Phase 8.5/12.5; sub-ns warm lookups, lazy via `OnceLock`) |

Tuple bundles were intentionally dropped (Phase 8.5) — named `#[derive(Bundle)]`
structs only, so the column cache has a stable per-type address. See
[PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md](PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md).

---

## Typed Query DSL (`Query<D, F>`)

The Bevy-shape typed query (Phase 8b). `Query<'w, 's, D, F>` is a `SystemParam`;
`D: QueryData`, `F: QueryFilter`. Drives iteration through the Phase-7 inline
column table (`archetype.columns[c].ptr.add(row * stride)`).

**Files:** [core/iters/query/](../crates/boyko_ecs/src/ecs/core/iters/query/) —
see `mod.rs` for the re-export surface.

| What you want | Where | Type / method |
|---------------|-------|---------------|
| The query SystemParam | [query/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query.rs):53 ✅ | `Query<'w, 's, D, F = ()>` |
| Per-row iteration | [query/iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/iter.rs) ✅ | `for x in &q` / `for x in &mut q`; `QueryIter` (83) / `QueryIterMut` (306) |
| Data leaves | [query/data.rs](../crates/boyko_ecs/src/ecs/core/iters/query/data.rs) ✅ | `&T`, `&mut T`, `Ref<T>` (629), `Mut<T>` (901), tuples 1..=12 |
| Read-only marker | [query/data.rs](../crates/boyko_ecs/src/ecs/core/iters/query/data.rs):253 ✅ | `ReadOnlyQueryData` (gates `&q` IntoIterator) |
| Filters | [query/filter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/filter.rs) ✅ | `With<C>` (300), `Without<C>` (402), `Added<C>` (521), `Changed<C>` (741), `Or<F>` (925), tuples |
| Direct-API query (no SystemParam) | [query/query_view.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs):68 ✅ | `QueryView<'w, D, F>` via `EcsMaster::query::<D, F>()` |
| Per-`(D,F)` archetype-match cache | [query/state.rs](../crates/boyko_ecs/src/ecs/core/iters/query/state.rs):45 ✅ | `QueryDataState<D, F>` (wraps the Phase-5c `QueryState`) |
| Per-`(D,F)` type interning | [query/query_type_registry.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query_type_registry.rs) ✅ | `QueryTypeId` / `QueryTypeKey`; `MAX_QUERY_TYPES = 1024` (4096 with `big_query_table`) |

The legacy archetype-yielding query (`Query::iter_one`/`iter_two`/
`with_component_ids`) survives as
[`LegacyQuery`](../crates/boyko_ecs/src/ecs/core/iters/legacy_query.rs) for
back-compat. New code uses the typed `Query<D, F>`.

### Chunked / parallel iteration

| What | Where | Method |
|------|-------|--------|
| Sequential per-archetype columnar slice | [query/chunk_iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/chunk_iter.rs) ✅ | `Query::for_each_chunk(\|slice\| …)` (also on `QueryView`) — flecs-style batched API (Phase X.A) |
| Parallel per-archetype-subrange | [query/par_chunk.rs](../crates/boyko_ecs/src/ecs/core/iters/query/par_chunk.rs) ✅ | `Query::par_for_each_chunk(\|slice\| …, BatchingStrategy)` |
| Parallel per-row | [query/par_iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs) ✅ | `Query::par_iter()` / `par_iter_mut()` → `ParQuery` (136) / `ParQueryMut` (185); `MIN_ARCHETYPE_FOR_PARALLEL` (Phase 9) |
| Chunked-data bound | [query/chunked_data.rs](../crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs):72 ✅ | `ChunkedQueryData` (`&T`/`&mut T`/`()` + tuples) — `Changed`/`Added`/`Ref`/`Mut` deliberately excluded |
| Archetypal-filter bound | [query/filter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/filter.rs):1681 ✅ | `ArchetypalQueryFilter` (`With`/`Without`/`Or`/tuples) |

`for_each_chunk` lands a credible multi-component SIMD win (boyko 1.28–1.34×
Bevy, native-SIMD) — see [PHASE-X.A-RESULTS.md](PHASE-X.A-RESULTS.md).

---

## SystemParam + Resources + IntoSystem

The ergonomic system machinery (Phases 8a/8c).

**Files:** [core/system/](../crates/boyko_ecs/src/ecs/core/system/).

| What you want | Where | Type / method |
|---------------|-------|---------------|
| The system trait | [system/system.rs](../crates/boyko_ecs/src/ecs/core/system/system.rs):? ✅ | `trait System { type Out; fn name; fn access; unsafe fn run_unsafe(UnsafeEcsCell); fn apply; fn set_change_ticks; }` (`Out` defaults to `()` via `SystemBox`) |
| Function → system | [system/into_system.rs](../crates/boyko_ecs/src/ecs/core/system/into_system.rs):47 ✅ | `trait IntoSystem<In, Out, Marker>`; `FunctionSystem` + markers `IsFunctionSystem` (67) / `ExclusiveSystemMarker` (154) |
| The SystemParam trait | [system/system_param.rs](../crates/boyko_ecs/src/ecs/core/system/system_param.rs) ✅ | `unsafe trait SystemParam` (GAT-based, two-phase `init_state` + `init_access`); tuples 0..=12 ([params/tuple_impl.rs](../crates/boyko_ecs/src/ecs/core/system/params/tuple_impl.rs)) |
| Read a resource | [system/params/res.rs](../crates/boyko_ecs/src/ecs/core/system/params/res.rs):40 ✅ | `Res<'w, R>` |
| Mutate a resource | [system/params/resmut.rs](../crates/boyko_ecs/src/ecs/core/system/params/resmut.rs):42 ✅ | `ResMut<'w, R>` |
| Per-system local state | [system/params/local.rs](../crates/boyko_ecs/src/ecs/core/system/params/local.rs):62 ✅ | `Local<'s, T>` (Phase 13) |
| Conflict / access surface | [system/system_meta.rs](../crates/boyko_ecs/src/ecs/core/system/system_meta.rs), [system/filtered_access_set.rs](../crates/boyko_ecs/src/ecs/core/system/filtered_access_set.rs) ✅ | `SystemMeta` (carries `last_run`/`this_run` ticks), `Access`, `FilteredAccessSet` |
| The worker-side world cell | [system/unsafe_ecs_cell.rs](../crates/boyko_ecs/src/ecs/core/system/unsafe_ecs_cell.rs) ✅ | `UnsafeEcsCell<'w>` (Copy, by-value receivers — Phase 8a C1) |

### Resources storage

| What | Where | Method |
|------|-------|--------|
| The slab | [core/resources/resources.rs](../crates/boyko_ecs/src/ecs/core/resources/resources.rs):100 ✅ | `Resources` — `insert::<R>` (154), `remove::<R>` (252), `contains::<R>` (370); clear-bit-first protocol (Phase 8a C3) |
| The trait | [core/resources/resource.rs](../crates/boyko_ecs/src/ecs/core/resources/resource.rs) ✅ | `trait Resource: Send + Sync + 'static` |
| The registry | [core/resources/resource_registry.rs](../crates/boyko_ecs/src/ecs/core/resources/resource_registry.rs) ✅ | lazy ids; `RESOURCE_SLOT_COUNT = 256` (51) |

---

## Commands + EntityCommands (deferred mutation)

Per-system byte-arena queue flushed via `SystemParam::apply` after the body
returns. No `Box<dyn Command>`, no per-command alloc (Phases 8d/11).

**Files:** [core/system/params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs),
[core/commands/](../crates/boyko_ecs/src/ecs/core/commands/).

| What you want | Where | Method (line) |
|---------------|-------|---------------|
| The SystemParam | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):95 ✅ | `Commands<'s>` |
| Spawn (chainable) | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):162 ✅ | `commands.spawn(bundle) -> EntityCommands` → `.insert(extra).id()` |
| Despawn | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):198 ✅ | `commands.despawn(entity)` |
| Address an existing entity | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):175 ✅ | `commands.entity(entity) -> EntityCommands` |
| Spawn many | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):246 ✅ | `commands.spawn_batch(iter)` |
| Custom command | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):123 ✅ | `commands.add::<C: Command>(cmd)` |
| The chainable handle | [params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs):73 ✅ | `EntityCommands<'a, 's>` — `.insert(..)`, `.remove::<C>()`, `.despawn()`, `.id()` |
| The queue + cmd structs | [commands/](../crates/boyko_ecs/src/ecs/core/commands/) ✅ | `CommandQueue` (CursorSync RAII panic-recovery), `SpawnAtCommand` / `InsertCommand` / `RemoveCommand` / `DespawnCommand` / `SpawnBatchCommand` / `SendEventCommand`; entity-id reservation via `EntityCounter` ([params/entity_counter.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_counter.rs):75) |

---

## Schedule + parallel scheduler

Bevy-class multi-system executor (Phase 9) on the custom
[`boyko_threadpool`](../crates/boyko_threadpool/) (Chase-Lev work-stealing +
`Scope` fork/join). Conflict graph + Tarjan SCC + Kahn topo + apply-window
barrier.

**Files:** [core/schedule/](../crates/boyko_ecs/src/ecs/core/schedule/).

| What you want | Where | Method (line) |
|---------------|-------|---------------|
| Build a schedule | [schedule/schedule_builder.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs) ✅ | `ScheduleBuilder::new(Arc<ThreadPool>)` (138); `add_system(system) -> SystemConfig` (165); `build(&mut world) -> Schedule` (307) / `try_build(...)` (330, diagnostics) |
| Run a frame | [schedule/schedule.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule.rs):? ✅ | `Schedule::run(&mut world)` — bumps tick, runs state pass, dispatches |
| Conflict bitsets + DAG | [schedule/conflict_graph.rs](../crates/boyko_ecs/src/ecs/core/schedule/conflict_graph.rs) ✅ | `ConflictGraph` |
| Per-frame scratch | [schedule/executor_scratch.rs](../crates/boyko_ecs/src/ecs/core/schedule/executor_scratch.rs) ✅ | `ExecutorScratch` (`pred_remaining`, `running`, `completed`, out-of-line completion channel — Phase 9.3c) |
| Erased system slot | [schedule/system_box.rs](../crates/boyko_ecs/src/ecs/core/schedule/system_box.rs) ✅ | `SystemBox` (1-cache-line `Out=()` hot slot) + `BoolSystem` (conditions) |
| The thread pool | [boyko_threadpool/](../crates/boyko_threadpool/src/lib.rs) ✅ | `ThreadPool` / `ThreadPoolBuilder` / `Scope` — `install` (dispatcher) vs `scope` (worker-safe, used by `par_iter`) |

**Soundness:** the executor is proven sound and Tree-Borrows-clean (Phase
9.1/9.2/9.3 — loom + Miri). `Arena` stays `!Send + !Sync`; allocation is
restricted to the dispatcher + `ScheduleBuilder::build` (ALLOC1 TLS guard).
See [PHASE-9.2-RESULTS.md](PHASE-9.2-RESULTS.md), [PHASE-9.3c-RESULTS.md](PHASE-9.3c-RESULTS.md).

### System ordering & sets

| What | Where | Method |
|------|-------|--------|
| Order one system | [schedule/system_config.rs](../crates/boyko_ecs/src/ecs/core/schedule/system_config.rs) ✅ | `.before(set)` / `.after(set)` / `.in_set(set)` (value-based) |
| Configure a set | [schedule/schedule_builder.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs):205 ✅ | `configure_set(set) -> ConfigureSet` (`.before`/`.after`/`.in_set` + hierarchy) |
| Set ids / derive | [schedule/system_set.rs](../crates/boyko_ecs/src/ecs/core/schedule/system_set.rs) ✅ | `SystemSetId` (interned from `(TypeId, discriminant)`); `#[derive(SystemSet)]` on fieldless enums |
| Build diagnostics | [schedule/schedule_builder.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs):330 ✅ | `try_build()` → `ScheduleBuildError` (`OrderingCycle` B9001, `SetHierarchyCycle` B9002, …) |
| Topo / Tarjan plumbing | [schedule/ordering.rs](../crates/boyko_ecs/src/ecs/core/schedule/ordering.rs) ✅ | `OrderingEdge` / `SystemKey` (Phase 9 scaffold completed in Phase 15) |

See [PHASE-15-RESULTS.md](PHASE-15-RESULTS.md).

### Run conditions (`.run_if`)

| What | Where | Method |
|------|-------|--------|
| Condition on a system / set | [schedule/system_config.rs](../crates/boyko_ecs/src/ecs/core/schedule/system_config.rs) ✅ | `.run_if(cond)` where `cond: impl IntoSystem<(), bool, M>` |
| Built-in conditions | [schedule/common_conditions.rs](../crates/boyko_ecs/src/ecs/core/schedule/common_conditions.rs) ✅ | `run_once`, `in_state`, `on_enter`, `on_exit`, `on_transition` |
| Executor integration | [schedule/schedule.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule.rs) ✅ | `evaluate_ready_conditions` pass at the apply-window barrier (0%-gate via `has_condition` bitset) |

`run_if` conditions are pure predicates (no `apply`). Eager AND fold (no
short-circuit). Tick-aware conditions (`Changed`/`Added`/`Ref`) work correctly
since Phase 16.1 ✅: a condition's window advances only on a frame it is
evaluated, and a gated system's ticks advance only on a frame it runs, so
dormant changes are never silently missed (Bevy "since-last-actual-run"
parity). See [PHASE-16-RESULTS.md](PHASE-16-RESULTS.md) +
[PHASE-16.1-RESULTS.md](PHASE-16.1-RESULTS.md).

---

## States

Application/game states layered on the single `Schedule` (Phase 17).

**Files:** [core/state/](../crates/boyko_ecs/src/ecs/core/state/).

| What | Where | How |
|------|-------|-----|
| The marker trait | [state/states.rs](../crates/boyko_ecs/src/ecs/core/state/states.rs) ✅ | `trait States: Send + Sync + Clone + PartialEq + Eq + Hash + 'static` (hand-impl, no derive) |
| Current / queued value | [state/state.rs](../crates/boyko_ecs/src/ecs/core/state/state.rs), [state/next_state.rs](../crates/boyko_ecs/src/ecs/core/state/next_state.rs) ✅ | `State<S>` (current), `NextState<S>` (`Unchanged`/`Pending(S)`) |
| Run conditions | [schedule/common_conditions.rs](../crates/boyko_ecs/src/ecs/core/schedule/common_conditions.rs) ✅ | `in_state(s)` / `on_enter(s)` / `on_exit(s)` / `on_transition(a, b)` |
| Transition pass | [state/transition_record.rs](../crates/boyko_ecs/src/ecs/core/state/transition_record.rs) ✅ | `StateTransitionRecord<S>` + `apply_state_transition::<S>`; runs once per `Schedule::run` (0%-gate via `state_entries.is_empty()`) |
| Generic-resource id trap fix | [state/state_resource_registry.rs](../crates/boyko_ecs/src/ecs/core/state/state_resource_registry.rs) ✅ | `TypeId`-keyed registry (avoids the rust#22991 `State<S>`-aliases-one-slot trap) |
| Builder / world entry | builder `init_state`/`insert_state`; `EcsMaster::{insert_state, init_state, state, set_next_state}` ✅ | see [App](#app--plugin-facade) + [EcsMaster](#high-level-facade-ecsmaster) |

See [PHASE-17-RESULTS.md](PHASE-17-RESULTS.md).

---

## Component lifecycle hooks & observers

Two reactive-callback mechanisms firing at the four structural-op kinds —
**add / insert / replace / remove**. A despawn fires `replace` + `remove` per
dying component (no separate despawn kind). Both gate on the per-archetype
`ArchetypeFlags` `u16` bit-test → a world with no callback pays one `test`/`jz`
and zero allocation ("0% when unused").

| What you want to do | Where | How |
|---------------------|-------|-----|
| **Hooks** — ONE write-once callback per component *type* | [core/component/hooks/](../crates/boyko_ecs/src/ecs/core/component/hooks/) ✅ | `#[component(on_add = path, …)]` derive XOR runtime `EcsMaster::register_component_hooks::<C>()` (Phase 14a — [PHASE-14-RESULTS.md](PHASE-14-RESULTS.md)) |
| **Observers** — `add`/`remove`-able LIST per `(kind, component)` | [core/component/observers/mod.rs](../crates/boyko_ecs/src/ecs/core/component/observers/mod.rs):136 ✅ | `EcsMaster::observe_on_{add,insert,replace,remove}::<C>(runner)` (Phase 14b) |
| Register an observer by `ComponentId` | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs):2104 ✅ | `add_observer(kind, cid, runner) -> ObserverId` |
| Remove an observer | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs):2121 ✅ | `remove_observer(id) -> bool` (recomputes archetype bits on last-of-kind removal) |
| The observer runner / context | [core/component/observers/mod.rs](../crates/boyko_ecs/src/ecs/core/component/observers/mod.rs):75 ✅ | `ObserverFn = unsafe fn(DeferredEcsMaster<'_>, ObserverContext)`; mutate only via the view's deferred `commands()` |
| The 4 cold dispatch fns | [core/component/observers/dispatch.rs](../crates/boyko_ecs/src/ecs/core/component/observers/dispatch.rs):115 ✅ | `fire_on_{add,insert,replace,remove}_observers` (`#[cold] #[inline(never)]`, wired at 7 fire sites) |

A **hook** is a single fn-ptr in the process-global `HOOKS` table (staleness-
panics if an archetype with `C` already exists); an **observer** is one of a
runtime-mutable per-world list (no staleness panic). At each fire site hooks run
first, then observers. Full catalog: [SYSTEMS.md §3.6](SYSTEMS.md).

---

## Hierarchies (parent-child)

Bevy-0.16 relationship model on the hooks substrate (Phase 19). `ChildOf` (FK on
the child, source of truth) + `Children` (reverse collection on the parent), kept
consistent by component hooks; default-recursive despawn cascade.

**Files:** [core/hierarchy/](../crates/boyko_ecs/src/ecs/core/hierarchy/) —
`mod.rs` (components + hand-impl `Component` + hook registration), `commands.rs`
(Link/Unlink/Clear deferred commands + the `ChildOf`/`Children` hook bodies),
`bundles.rs` (1-field `Bundle` newtypes routing the first-child insert through the
audited `migrate_entity_insert`).

| What you want to do | Where | How |
|---------------------|-------|-----|
| Parent component (FK on child) | [hierarchy/mod.rs](../crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs) ✅ | `ChildOf(pub Entity)` — insert links, overwrite reparents, remove unlinks |
| Children collection (read-only) | [hierarchy/mod.rs](../crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs) ✅ | `Children` — `as_slice()` / `len()` / `is_empty()` / `contains()`; maintained reactively, never written by user code |
| Add a child / children | [params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs) ✅ | `commands.entity(parent).add_child(c)` / `.add_children(&[..])`; `Commands::add_child(p, c)` |
| Set / clear parent | [params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs) ✅ | `.set_parent(p)` / `.remove_parent()` |
| Remove specific / all children | [params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs) ✅ | `.remove_children(&[..])` (listed only) / `.clear_children()` (all) |
| Despawn keeping children | [ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs):1142 ✅ | `despawn_without_children(e)` — opt out of the default recursive cascade |
| Recursive despawn (default) | [ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs):1119 ✅ | `delete_entity(e)` / `commands.despawn(e)` cascades to all descendants |

`Children` consistency is at the deferred-hook-queue drain (same-frame apply
window). Guards: self-ref + dangling-parent are reactively rejected (the bad
`ChildOf` is removed, the collection untouched); deep `ChildOf` cycles are a
documented footgun (only self-ref is checked). Sibling order is unspecified
(`swap_remove`); an emptied `Children` is retained (no archetype thrash). The
net new `unsafe` for the whole feature is **one** (the `MaybeUninit` cascade
buffer). DEFERRED: transform propagation, parallel tree walk, `iter_descendants`,
a generic `Relationship` trait. See [PHASE-19-RESULTS.md](PHASE-19-RESULTS.md).

> The cascade exposed **BUG-P19-TB-1**, a pre-existing latent Tree-Borrows UB in
> the deferred command-queue re-entrant drain (`commands/command_queue.rs`
> `apply_via_raw_twin` cached a `NonNull<Vec>` foreign-written by a re-entrant
> `push`). Fixed by walking a stack-local `mem::take`'d copy of the queue (the
> audited `apply` on a disjoint allocation). See
> [BUG-P19-TB-1-PLAN.md](BUG-P19-TB-1-PLAN.md).

---

## Change detection (Tick / `Added<T>` / `Changed<T>`)

Bevy-style per-row tick storage (Phase 10).

**Files:** [core/change_detection/](../crates/boyko_ecs/src/ecs/core/change_detection/).

| What you want | Where | How |
|---------------|-------|-----|
| The tick type | [change_detection/tick.rs](../crates/boyko_ecs/src/ecs/core/change_detection/tick.rs) ✅ | `Tick(u32)` — `is_newer_than`; `MAX_CHANGE_AGE` / `CHECK_TICK_THRESHOLD` |
| Per-row tick storage | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs):83 ✅ | `ComponentPool::{added_ticks, changed_ticks}: Box<[UnsafeCell<Tick>]>` |
| Filter on added/changed | [query/filter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/filter.rs) ✅ | `Added<C>` (521) / `Changed<C>` (741) |
| Read with change info | [query/data.rs](../crates/boyko_ecs/src/ecs/core/iters/query/data.rs) ✅ | `Ref<T>` (629, immutable + flags), `Mut<T>` (901, deref-guard bumps the tick) |
| Frame bump + wraparound scan | [change_detection/check_ticks.rs](../crates/boyko_ecs/src/ecs/core/change_detection/check_ticks.rs) ✅ | `run_check_ticks_scan`; `EcsMaster::change_tick: AtomicU32` bumped per `Schedule::run` |

0% measurable overhead on queries that use no change detection. See
[PHASE-10-CHANGE-DETECTION-PLAN.md](PHASE-10-CHANGE-DETECTION-PLAN.md).

---

## Events

A full double-buffered event dispatcher (Phase 6) plus the `EventReader` /
`EventWriter` SystemParam wrappers (Phase 12). **Note:** earlier revisions of
these docs said "no dispatcher" — that is now stale; the dispatcher exists.

**Files:** [core/events/](../crates/boyko_ecs/src/ecs/core/events/),
[core/system/params/event_reader.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_reader.rs),
[core/system/params/event_writer.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_writer.rs).

| What you want | Where | How |
|---------------|-------|-----|
| Define an event type | [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) ✅ | `#[event] struct DamageEvent { #[participant(...)] victim: Entity, #[parameter] amount: f32 }` |
| Read events in a system | [params/event_reader.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_reader.rs):87 ✅ | `EventReader<'s, E>` → `EventIter` (245) (cursor checkpointed on partial iter) |
| Write events in a system | [params/event_writer.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_writer.rs):89 ✅ | `EventWriter<'s, E>` (per-lane TLS routing; parallel writers OK) |
| The dispatcher | [events/event_dispatcher.rs](../crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs) ✅ | `EventDispatcher` — `send_event::<E>` (274), `send::<E>(thread_index, ..)` (292), `update_events()` (436, frame swap) |
| The double-buffer | [events/event_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/event_buffer.rs) ✅ | `EventBuffer<E>` — split cache-line lanes (Phase 12 false-sharing fix) |
| Config / capacity | [events/event_config.rs](../crates/boyko_ecs/src/ecs/core/events/event_config.rs) ✅ | `EventConfig`; `MAX_EVENT_THREADS = 64`, `MAX_EVENT_CAPACITY = 16384` ([constants.rs](../crates/boyko_ecs/src/ecs/constants.rs)) |
| Registry / metadata | [events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs) ✅ | lazy `event_id()`; `MAX_EVENTS = 256` (51) |
| Participants / parameters | [events/participants/](../crates/boyko_ecs/src/ecs/core/events/participants/), [events/parameters/](../crates/boyko_ecs/src/ecs/core/events/parameters/) ✅ | `Participants` / `Parameters` traits + TypeId-guarded buffers (Q-019) |

Events sit OUTSIDE the conflict graph (Option A) — parallel writers of the same
`E` are OK via per-lane TLS routing. See [PHASE-12-RESULTS via memory] and
[PHASE-6-EVENT-DISPATCH-PLAN.md](PHASE-6-EVENT-DISPATCH-PLAN.md).

---

## Resources

See [SystemParam + Resources](#systemparam--resources--intosystem) above for the
storage + `Res`/`ResMut` params, and [EcsMaster](#high-level-facade-ecsmaster)
for `insert_resource` / `resource`.

---

## Archetypes (lower-level discovery)

| What | Where | Method |
|------|-------|--------|
| The archetype | [core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs):122 ✅ | `Archetype` — inline `columns: [Column; 512]` at offset 0 (Phase 7 fast read path), `entity_ids`, `flags`, `signature` |
| Hot column entry | [core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs):28 ✅ | `Column { ptr: *mut u8, stride: u32 }` (16 B; `is_null()` ⇔ absent) |
| Remove outcome | [core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs):85 ✅ | `enum RemoveOutcome { Last, Swapped { moved_entity }, PoolFailure }` (C-006) |
| The manager | [core/archetype/archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs):19 ✅ | `ArchetypeMaster` — owns the `ObserverRegistry` (71); dual gen counters |
| Slab storage | [core/archetype/archetype_bundle.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs) ✅ | `ArchetypeBundle` (stable-address slab + sparse id map) |
| Signature | [core/archetype/archetype_signature.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_signature.rs) ✅ | `ArchetypeSignature { mask, block_summary, section_summary }` |
| Discovery (registry) | [core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs) ✅ | `find_archetypes_with_components` / `find_matching_archetypes` / `find_with_filter` (+ `_into(out)` variants) |
| ABA-safe match cache | [core/iters/query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs), [core/iters/archetype_bit_set.rs](../crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs) ✅ | `QueryState` (dual gen counters), `ArchetypeBitSet` (1024-bit dedup) |

The dual-generation design (`generation` for creation deltas,
`structural_generation` for removal/clear) is the load-bearing ArchetypeId-ABA
fix (Phase 5c). `MAX_ARCHETYPES = 1024`.

---

## Type-erased component storage

| What you want to do | Where | Method |
|---------------------|-------|--------|
| Create a pool | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::new(_arena, component_id, n, m)` — explicit row ceiling `n × m` EXACTLY (X.I D2 mapping; arena param vestigial → X.J); `with_default_sizes` = byte-targeted clamp sizing |
| Grow a pool (automatic) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `#[cold] grow_rows` — per-pool `VmReservation [data\|added\|changed]`, slab doubling 64 KiB…64 MiB, ticks lockstep, idempotent, O(1) in live rows, bases never move (Phase X.I). 1M-entity single-archetype ramp **2.24× faster than Bevy**, worst-batch spike **0.022×** ([PHASE-XI-RESULTS.md](PHASE-XI-RESULTS.md)) |
| Committed-rows frontier (diagnostics) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `committed_rows()`; `capacity()` = reserve ceiling |
| Append a component (raw bytes) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `add(&[u8])` |
| Append a component (typed, TypeId-guarded) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `add_typed::<T>(value)` |
| Read a component (typed) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `get_typed::<T>(idx)` / `get_mut_typed::<T>(idx)` (C-004) |
| Read a component (raw) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `get_raw(idx)` / `get_raw_mut(idx)` |
| Overwrite a slot | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `set_component(idx, &[u8])` (runs `drop_fn` on the old value) |
| Remove (swap with last) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `swap_remove(idx)` / `pop()` (run `drop_fn`) |
| Address row `i`'s bytes | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs):? ✅ | private `row_ptr(i)` = `buffer.as_ptr().add(i * stride)` (Phase X.B removed the `Vec<Unit>` cache) |
| Live-row count | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs):49 ✅ | the `len` field / `count()` |
| Dense base pointer | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `buffer_ptr()` — SIMD-aligned (`SIMD_BUFFER_ALIGN = 32`, Phase X.A) |

Type erasure: the pool stores raw bytes + the `Layout` from the
`ComponentRegistry`. Drop discipline: a cached `drop_fn: Option<DropFn>` runs on
`swap_remove` / `pop` / `set_component` / `Drop` (M-004). **Phase X.B** deleted
the parallel `units: Vec<Unit>` (each entry == `buffer + i*stride`) — rows are
now computed arithmetic, which net-removed `unsafe`. **Phase 10** added the
per-row tick columns; **Phase X.I** moved them into the pool's own reservation
(`[data | added | changed]` sub-regions), made the pool self-growing, and
DELETED the chunk machinery (`memory/chunk.rs` — the dirty flags were
written-never-read; a per-mutation `udiv` died with them). See
[PHASE-XB-RESULTS.md](PHASE-XB-RESULTS.md), [PHASE-XI-RESULTS.md](PHASE-XI-RESULTS.md).

---

## Memory and allocation

| What you want to do | Where | Method |
|---------------------|-------|--------|
| Construct the arena | [memory/arena.rs](../crates/boyko_ecs/src/ecs/memory/arena.rs) ✅ | `Arena::with_reserve(reserve, initial_commit)` — 4 GiB reserve + lazy slab commit, GROWS on demand, addresses never move (Phase X.F); `with_capacity(c)` = eager back-compat; `EcsMaster::with_arena_reserve(bytes)` knob |
| Grow the arena (automatic) | [memory/arena.rs](../crates/boyko_ecs/src/ecs/memory/arena.rs) ✅ | `#[cold] grow_then_retry` — commit next slab at the frontier + retry; panic only at reserve exhaustion. Growth-crossing spawn **1.75× faster than Bevy** ([PHASE-XF-RESULTS.md](PHASE-XF-RESULTS.md)) |
| Allocate a block | [memory/arena.rs](../crates/boyko_ecs/src/ecs/memory/arena.rs) ✅ | `allocate_layout(layout)` / `allocate(size)` |
| Free the arena | [memory/arena.rs](../crates/boyko_ecs/src/ecs/memory/arena.rs) ✅ | `impl Drop` — per-cfg-arm matching deallocator (M-001) |
| Best-fit free-block tracking | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::allocate_aligned` / `find_best_fit` / `insert` (O(1) coalesce) / `defragment` |
| Align an address/size | [memory/utils.rs](../crates/boyko_ecs/src/ecs/memory/utils.rs) ✅ | `align_up(value, alignment)` |
| Reserve/commit a VM range | [memory/vm.rs](../crates/boyko_ecs/src/ecs/memory/vm.rs) ✅ | `VmReservation::{reserve, reserve_unzeroed, commit}` — the shared per-OS primitive under the arena, `InlandStore`, and every `ComponentPool` (X.G/X.H/X.I) |

`Arena` is `!Send + !Sync` by construction. Backing acquisition (Phase X.F):
reserve-only syscall (`VirtualAlloc(MEM_RESERVE, PAGE_NOACCESS)` on Windows,
`mmap(PROT_NONE)` on Unix) + lazy geometric slab commits at the frontier
(`MEM_COMMIT`/`mprotect`); Miri / wasm32 / exotic targets eagerly allocate the
full reserve from the global allocator (growth = watermark bump). `Arena::new`
≈ 762 ns (reserve-only, zero commit charge). See
[PHASE-XC-RESULTS.md](PHASE-XC-RESULTS.md) + [PHASE-XF-RESULTS.md](PHASE-XF-RESULTS.md).

---

## Identifiers

| What | Where | Type |
|------|-------|------|
| Entity / archetype / component / etc. IDs | [identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs) ✅ | `#[repr(transparent)] EntityId(usize)` + siblings (C-017: strongly-typed newtypes, defined via one `define_id!` macro) |
| Generation counter | [identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs) ✅ | `Generation = usize` (alias — only paired with `EntityId`) |
| Slot (boyko_utils) | [boyko_utils/identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs) ✅ | `Slot { index, generation }` |

Newer dense-table sizing newtypes (`ResourceId`, `BundleTypeId`, `QueryTypeId`,
`ObserverId`, `SystemSetId`) live next to their subsystems, not in
`primitives.rs`.

---

## boyko_utils (reusable collections)

| What | Where | Type |
|------|-------|------|
| Dense sparse set (usize keys) | [boyko_utils/sparse_map/sparse_map.rs](../crates/boyko_utils/src/sparse_map/sparse_map.rs) ✅ | `SparseMap<U>` (used by `ArchetypeBundle`/registry; `EntityMaster` moved off it in Phase 7) |
| Generation-tracked slot map | [boyko_utils/sparse_map/sparse_slot_map.rs](../crates/boyko_utils/src/sparse_map/sparse_slot_map.rs) ✅ | `SparseSlotMap<U>` (ABA-fixed via tombstone+gen, M-016) |
| Trait abstraction | [boyko_utils/sparse_map/sparse_collection.rs](../crates/boyko_utils/src/sparse_map/sparse_collection.rs) ✅ | `SparseCollection<K, V>` |
| Bitset (generic word size) | [boyko_utils/bit_mask/bit_set.rs](../crates/boyko_utils/src/bit_mask/bit_set.rs) ✅ | `BitSet<T: BitInteger>` |
| Fixed 256-bit set | [boyko_utils/bit_mask/bit_set_256.rs](../crates/boyko_utils/src/bit_mask/bit_set_256.rs) ✅ | `BitSet256` (+ `pop_lowest_set_bit`) — Phase 6, backs resource/event lane masks |
| Identifier primitives | [boyko_utils/identifiers/](../crates/boyko_utils/src/identifiers/) ✅ | `Generation`, `Slot` |

---

## boyko_demo (dogfooding sandbox)

A wgpu+egui GPU-instanced sandbox exercising the public API (particles / boids /
physics via Phase-17 states, real `Schedule::run` + `par_iter` + zero-AoS-copy
`for_each_chunk` → GPU upload). Compiles for wasm32 too.

**Files:** [crates/boyko_demo/src/](../crates/boyko_demo/src/) — `app.rs`,
`sim/` (systems, grid, modes, runner), `render/`, `ui/`. See
[DEMO-PLAN.md](DEMO-PLAN.md) + [DEMO-DOGFOODING.md](DEMO-DOGFOODING.md).

---

## What is NOT in the engine (deliberately / deferred)

| Missing | Why / where tracked |
|---------|--------------------|
| ZST components / resources / events | ❌ rejected (debug-assert / compile-time guard); a Phase-2-future enhancement |
| `Option<Res<R>>` SystemParam → `resource_exists` condition | 📋 deferred (Phase 16 residual) |
| ~~Tick-aware run conditions (`Changed`/`Added`)~~ | ✅ LANDED — Phase 16.1 (dormancy-correct ticks, [PHASE-16.1-RESULTS.md](PHASE-16.1-RESULTS.md)) |
| `for_each_chunk` with `Changed`/`Added`/`Ref`/`Mut` | ❌ gated out at compile time; use `iter()` — Phase 13.X `ChunkedTickedQueryData` |
| Multi-schedule label map / SubApps / `PluginGroup` | 📋 deferred (Phase 18 boundaries) |
| Single-dep prelude including derives | 📋 deferred — needs the `boyko-macros` cycle refactor (Phase 18) |
| 5× `for_each_chunk` headline on a wide/SIMD-heavy workload | 📋 Phase X.A.2 (credible 1.3× multi-component win already landed) |
| Auto sync-point insertion (coalesced command flush) | 📋 deferred (per-system apply window already a sync point) — Phase 15 residual |
| `Participants`/`Parameters` split revisit (Q-020) | ❌ deferred — no participant-filtered dispatch use case yet |

---

## Tests / benchmarks at a glance

Per the latest phase results, the `boyko-ecs` test suite is ~918 passing debug /
903 release (`cargo test -p boyko-ecs`, Phase 19 baseline; ~983 workspace) across
in-module `#[cfg(test)]` units + the integration files under
`crates/boyko_ecs/tests/`. Miri (`-Zmiri-tree-borrows`, `-Zmiri-ignore-leaks` for
the spawn-reaching suites) is clean for the change-detection / hooks / observers /
hierarchies / states / executor-soundness suites. For the exact gate per phase,
read the relevant `docs/PHASE-*-RESULTS.md`.

**Benchmarks** (criterion, `harness = false`) live in
[crates/boyko_ecs/benches/](../crates/boyko_ecs/benches/) (see the `[[bench]]`
list in [Cargo.toml](../crates/boyko_ecs/Cargo.toml)) and the cross-engine
comparison in [crates/bench_bevy_vs_boyko/](../crates/bench_bevy_vs_boyko/).
Methodology (deterministic `[profile.bench]` codegen + opt-in `bench-alloc`
mimalloc + the median-of-N `bench.ps1`) is in
[BENCHMARKING.md](BENCHMARKING.md) (Phase X.E).
