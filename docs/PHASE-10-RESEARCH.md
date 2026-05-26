# Research: Change Detection in ECS Engines (Phase 10)

## §1 Executive Summary

Boyko-engine has no Tick infrastructure yet. The architect must choose a storage strategy for change detection from four options. The state-of-the-art ECS landscape splits cleanly into three camps:

- **Per-row ticks (Bevy / hecs-fork)** — 8 bytes per (entity, component): one `Tick` for added, one for changed. Stored as two parallel `Vec<UnsafeCell<Tick>>` columns next to the component data. O(n) filter cost per row; cache-friendly because ticks-only iteration touches only the tick columns. False positives are accepted as a design tradeoff (any `&mut T` deref bumps the changed tick whether or not the value differs).
- **Per-chunk ticks (Unity DOTS)** — 4-8 bytes per (chunk, component). 16 KB chunks aggregate ~100-200 entities. Filter checks 1 tick per chunk × 100+ entities. Coarse-grained false positives (whole chunk marked changed if any entity in it is touched, or if a system held write access without writing). Hardcoded 2-component filter limit. Bumped immediately when a write query executes.
- **Push-based / signal-based (flecs hooks, EnTT signals)** — no per-row state; observers fire on `set`/`modified`. Zero scanning overhead, but observer dispatch has constant per-event cost (function pointer + collector list maintenance) and changes are visible only via the observer callback or buffered "collector" entity list. flecs adds a separate cached **per-table dirty counter** for query-level `changed()` checks (no per-entity granularity).

The empirically dominant tradeoff: **storage cost vs filter cost vs false-positive rate.** Bevy pays 8 B/(entity·component) for exact per-row granularity; Unity DOTS pays ~50 B per chunk for coarse-grained granularity at near-zero filter cost; flecs hands the user a manual `modified()` call site.

Boyko's Phase 7 architecture has no chunk concept — archetypes ARE the storage unit, and the column tables are at offset 0 inside `Archetype` with direct base+stride access. This shapes the storage decision: per-row (option A) or per-archetype (option B/C). The architect should consult §8 for the option matrix and §6 for parallel-scheduler integration.

Key references: [Bevy PR #6547 (Split Component Ticks)](https://github.com/bevyengine/bevy/pull/6547), [Bevy PR #3956 (deterministic change lifespan)](https://github.com/bevyengine/bevy/pull/3956), [Unity DOTS chunk versioning](https://gametorrahod.com/change-version/), [flecs change tracking example](https://github.com/SanderMertens/flecs/blob/master/examples/cpp/queries/change_tracking/src/main.cpp).

---

## §2 Bevy `ComponentTicks` Deep Dive

### Storage shape (post-PR #6547)

Bevy's `Column` is "a type-erased contiguous container for data of a homogeneous type" with three parallel buffers per column:

- A `BlobVec` of components (the data itself).
- `Vec<UnsafeCell<Tick>>` for **added ticks**.
- `Vec<UnsafeCell<Tick>>` for **changed ticks**.

All three share index `i` for entity at row `i`. Source: [Column docs](https://docs.rs/bevy/latest/bevy/ecs/storage/struct.Column.html) confirms "An element shares its data across these buffers by using the same index."

Before PR #6547, both ticks were stored interleaved as `ComponentTicks { added: Tick, changed: Tick }` (8 bytes contiguous). The split was driven by the observation that most queries only need the **changed** ticks; loading the full 8 bytes wasted half the cache traffic. The split returns a `TickCells { added: &UnsafeCell<Tick>, changed: &UnsafeCell<Tick> }` to consumers. Reported gains: `busy_systems` benchmark improved 32-106%. The split also enabled autovectorization for `for_each` and `par_for_each` query iterations. Source: [PR #6547](https://github.com/bevyengine/bevy/pull/6547).

### `Tick` type and atomic discipline

`Tick` is a `#[repr(transparent)]` wrapper around `u32`. Key methods (from [Tick docs](https://docs.rs/bevy/latest/bevy/ecs/component/struct.Tick.html)):

- `is_newer_than(last_run: Tick, this_run: Tick) -> bool` — the core filter predicate. Renamed in [PR #7561](https://github.com/bevyengine/bevy/pull/7561) from the misleading `is_older_than`. The implementation pattern uses **wrapping subtraction**:
  ```rust
  // semantics: tick falls in the half-open window (last_run, this_run]
  this_run.wrapping_sub(last_run) > tick.wrapping_sub(last_run)
  ```
  Both `tick.wrapping_sub(last_run)` and `this_run.wrapping_sub(last_run)` produce relative ages bounded by `MAX_CHANGE_AGE`, so the comparison is well-defined across the u32 wraparound.

- `check_tick(&mut self, tick: Tick) -> bool` — clamps if the tick's age exceeds `MAX_CHANGE_AGE`. Returns true if a clamp happened.

### MAX_CHANGE_AGE and the check_tick scan

`CHECK_TICK_THRESHOLD = 518_400_000` (from [CHECK_TICK_THRESHOLD docs](https://docs.rs/bevy/latest/bevy/ecs/change_detection/constant.CHECK_TICK_THRESHOLD.html)). This is the "(arbitrarily chosen) minimum number of world tick increments between `check_tick` scans."

`MAX_CHANGE_AGE = u32::MAX - (2 * CHECK_TICK_THRESHOLD - 1)` ≈ 3,258,166,895 ≈ 75% of `u32::MAX`. Source: [PR #3956 commentary](https://github.com/bevyengine/bevy/pull/3956).

Wraparound strategy:
- Every time the world's change tick advances by ≥ `CHECK_TICK_THRESHOLD`, `Schedule` runs a `check_ticks` scan that walks every stored `Tick` (in columns and in system metadata) and clamps any whose age exceeds `MAX_CHANGE_AGE` down to exactly `MAX_CHANGE_AGE`.
- This bounds the relative age universally, so the wrapping-subtraction comparison cannot yield false positives.
- A new system starts with `last_run = current_tick - MAX_CHANGE_AGE` (per PR #3956) to ensure it detects everything as changed on first run — never with `last_run = 0`, which is wraparound-unsafe.

### Per-system `SystemChangeTick`

From [SystemChangeTick docs](https://docs.rs/bevy/latest/bevy/ecs/system/struct.SystemChangeTick.html):
- `this_run()` — the World change tick observed by the system at dispatch time.
- `last_run()` — the World change tick observed by the system the previous time it ran.
- Implements `ReadOnlySystemParam`. `Copy`. Safe for parallel access.

The `last_run` field lives in `SystemMeta` and is updated when the system finishes running. The World's change tick is bumped — typically — once per system invocation, not per component write.

### Atomic discipline for `World::change_tick`

Bevy's world holds `change_tick: AtomicU32` (referenced indirectly in [Issue #15683](https://github.com/bevyengine/bevy/issues/15683) discussing migration to u64; the u32 design persists). It is bumped via `increment_change_tick()` — typically once per system run, returning the incremented value as the system's `this_run`. Atomic contention is bounded by system count, not component count.

The per-component write path **does not** bump the global atomic. Instead, `Mut<T>::deref_mut` writes the system's already-known `this_run` into the column's `changed_ticks[row]` slot through the `UnsafeCell<Tick>`. The `UnsafeCell` is required because the column gives out shared references during iteration; the actual write is an unsynchronized `u32` store on a row that the aliasing model guarantees is mutually exclusive (per system access declaration).

### Memory cost

8 bytes per (entity, component) — two `Tick = u32`. At 100k entities × 50 components on average → 40 MB tick overhead. PR #6547 split makes half of that (`added_ticks`) cold in normal iteration; queries that only check `Changed<T>` touch only the `changed_ticks` half: 20 MB hot.

### False positives

From [Bevy Cheat Book](https://bevy-cheatbook.github.io/programming/change-detection.html): "Simply accessing components via a mutable query, or resources via ResMut, without actually performing a `&mut` access, will _not_ trigger it." But once `&mut` is performed, the changed tick is bumped — Bevy does not compare bytes. The mitigation pattern is `set_if_neq` (from [PR #5373](https://github.com/bevyengine/bevy/pull/5373)) on `DetectChangesMut`: only writes if `new != current`, avoiding the spurious tick bump.

### Cost / archetypal classification

`Added<T>` and `Changed<T>` are **non-archetypal**. From [Changed docs](https://docs.rs/bevy/latest/bevy/ecs/query/struct.Changed.html): "if query (with T component filter) matches million entities, Changed<T> filter will iterate over all of them even if none of them were changed." This is acknowledged as O(n) where n = matching entities — see [Issue #5097](https://github.com/bevyengine/bevy/issues/5097), which proposes redundantly storing change ticks at the column-level and archetype-level to enable early-exit.

---

## §3 flecs: Push (Observers) + Pull (Per-Table Dirty Counters)

flecs uses **two distinct mechanisms** instead of one unified tick system.

### Push: OnSet observers and hooks

From [Observers Manual](https://github.com/SanderMertens/flecs/blob/master/docs/ObserversManual.md):
- `OnSet` event fires whenever `set` or `modified` is called on a component. **Direct mutation through systems does NOT fire OnSet** — the system must explicitly call `modified` to notify observers.
- `OnAdd` / `OnRemove` fire on component lifecycle, independent of value mutation.
- **Hooks** (`on_add`, `on_set`, `on_remove`) are one-per-component-type fast callbacks; **observers** can match complex queries and arbitrary events.

This is "push" semantics: the cost is paid at write time (one function-pointer dispatch per observer per write). There is no per-row tick state.

### Pull: per-table dirty counters

From [Queries.md](https://github.com/SanderMertens/flecs/blob/master/docs/Queries.md):
> "Queries have a builtin mechanism for tracking changes per matched table. Change detection works by storing a list of counters on tracked tables, where each counter tracks changes for a component in the table. When a component in the table changes, the corresponding counter is increased. An additional counter is stored for changes that add or remove entities to the table."

API: queries are built with `.detect_changes()`; check via `query.changed()` (any matched table changed) or `it.changed()` (current iterator table changed). Counters are stored per **table** (the flecs analogue of archetype), not per row.

Granularity: **table-level**. False positives at the table level. Counters are tracked only for tables matched by change-detection queries — opt-in cost model.

Limitations explicitly documented: change detection does not fire for write-only (`EcsOut`) or filter (`EcsInOutNone`) terms, nor for tag terms, nor for unmatched terms.

### flecs 4.1 optimization

From [Sander Mertens' blog](https://ajmmertens.medium.com/flecs-4-1-is-out-fab4f32e36f6): "Change detection overhead has been reduced by 2x for trivial queries in Flecs v4.1. Additionally, Flecs no longer inserts Modified commands if there are no OnSet hooks/observers, which can cut frame times in half in applications that use set frequently."

### Push vs pull trade-offs

| Property | Push (observers) | Pull (filter / ticks) |
|----------|------------------|----------------------|
| Latency | Immediate (sync callback) | End-of-frame / next system |
| Per-write CPU | Constant (callback dispatch) | Zero |
| Per-frame scan CPU | Zero | O(matched_entities) for per-row, O(matched_tables) for table-level |
| Memory overhead | O(observers × matches) | O(entities × tracked_components) for per-row, O(tables × components) for table-level |
| Reordering safety | Fragile (observer execution order matters) | Robust (filters see consistent snapshot) |
| Filter composability | Hard (must register every variant) | Easy (filters compose at query type level) |

Bevy chose pull because it composes with the type-driven `Query<D, F>` DSL and decouples write-side from observer state. EnTT pioneered the push side and now offers both (signals + observers). flecs offers both side-by-side.

---

## §4 Unity DOTS: Chunk-Level Versioning

Source: [Chunk's Change Version](https://gametorrahod.com/change-version/), [Unity ECS chunk versioning Medium](https://medium.com/@5argon/unity-ecs-creating-an-efficient-system-with-chunk-iteration-didaddorchange-didchange-221427f5361b), [Unity DOTS docs](https://docs.unity3d.com/Packages/com.unity.entities@1.0/manual/systems-entityquery-filters.html).

### Three version levels

1. **Global system version** — an integer maintained by `EntityManager`. Increments before each system update.
2. **Last system version** — per-system snapshot of the global version from the previous run. Exposed as `LastSystemVersion`.
3. **Chunk change version** — per (chunk, component). Updated when data is written with write access permission.

### When the version bumps

> "Instantly when a query is executed related to that component and it has write permission, then, chunks returned for that query will all get their change version updated."

Critically, **write permission is sufficient — actual mutation is not required.** This is the explicit false-positive design choice. The rationale: chunk-level granularity means actual byte-comparison is too costly relative to the optimization win.

Additional sources of change-version bumps:
- New chunks (version 0 — always treated as changed).
- Chunk movement due to archetype changes (component add/remove).
- Any entity in the chunk changes → whole chunk marked.

### Filter API and hard 2-component limit

`.WithChangeFilter<T>()` on `SystemAPI.Query` — limited to 2 components.
`.AddChangedVersionFilter()` on `QueryBuilder` — same 2-component limit (additive but capped internally).
Manual `chunk.DidChange<T>(lastSystemVersion)` inside `IJobChunk` — unlimited.

### Cost model

- Storage: 1 `uint` (4 B) per (chunk, component) — for a 16 KB chunk with ~100 entities × 10 components = 40 B per chunk. Per-entity overhead: **0** (it's chunk-amortized).
- Filter cost: 1 version compare per chunk. For 1M entities in 10k chunks: 10k compares vs Bevy's 1M.
- False-positive rate: high under coarse access patterns; mitigated by the "previous component" mirror pattern (store the prior value in a sibling component and do a byte-equality fast-path inside the system).

### Why DOTS works with chunks but Bevy doesn't

DOTS chunks are **fixed-size 16 KB pages** within an archetype, each holding multiple entities. Bevy and boyko have archetypes-as-Vec — entire archetype is one contiguous storage, no sub-archetype chunk concept. To get Unity's granularity, Bevy/boyko would either:
- Add a chunk abstraction inside archetype (large refactor — Phase 7's column-at-offset-0 design assumes flat archetype storage).
- Use the archetype as the granularity unit (option B in §8 — coarser still than DOTS chunks because archetypes can hold millions of entities).

---

## §5 EnTT: Signals + Observers + Reactive Mixin

Source: [Signal docs (EnTT wiki)](https://github.com/skypjack/entt/wiki/Events,-signals-and-everything-in-between), [Observer source (skypjack.github.io)](https://skypjack.github.io/entt/observer_8hpp_source.html), [DeepWiki Signal](https://deepwiki.com/skypjack/entt/5-signal-and-event-system).

### Signal API

```cpp
registry.on_construct<T>().connect<&handler>();
registry.on_update<T>().connect<&handler>();
registry.on_destroy<T>().connect<&handler>();
```

Three lifecycle signals per component type. `on_update` fires on `registry.patch<T>(entity, ...)` or `registry.replace<T>(...)`. Direct mutation through `registry.get<T>(entity)` does **not** fire — same caveat as flecs.

### Delegate / sigh memory

> "The delegate achieves zero overhead by storing only two pointers: one for the callable and one for the instance/payload."
> "Total memory for signal handlers is 24 bytes (vector) + 16 bytes × number of delegates."

So per-component signal cost = sizeof(vector header) + 16 × (listeners). Zero if no listener connected — checked via fast `vector.empty()` skip.

### Observer + collector

```cpp
entt::observer obs{registry, entt::collector.update<Position>()};
// Later — iterate only the entities whose Position was updated:
for (auto entity : obs) { ... }
```

The `collector` builds matchers that the `observer` translates into automatic registry connections. After dispatch, the observer holds a deduplicated `set<entity>` of entities matching the collector's rules. Reading `obs` consumes (or persists, depending on usage) the set.

Variants:
- **Observing matcher**: tracks updates and add events.
- **Grouping matcher**: tracks entities that "would have entered the given group."
- `.where<X>(entt::exclude<Y>)` — refinement filter applied at observe time.

### Lock-free? No

EnTT signals are single-threaded by design. Concurrent registry mutation from worker threads is undefined unless externally synchronized. The signal dispatch is a vector iteration — not thread-safe under concurrent writes.

For a parallel scheduler, EnTT's pattern is: workers stage changes; a single-threaded "merge" step on the main thread fires the signals. This is the same shape as Bevy's apply-window deferred commands but applied to a different mechanism.

---

## §6 Atomic Tick Storage Under Phase 9 Parallelism

Phase 9 (just landed) runs multiple systems concurrently via Chase-Lev work-stealing thread pool. Apply-window barrier flushes structural changes between waves. Conflict graph already prevents two systems from acquiring overlapping mutable component views simultaneously.

### What's safe under Phase 9 today (without ticks)

Mutually exclusive component writes — confict graph guarantees no two systems write the same `(archetype, component)` simultaneously. Component bytes are sound.

### What ticks add

If we store ticks as `Vec<UnsafeCell<Tick>>` parallel to the component column, the same conflict-graph reasoning applies: a system that holds a write access to component `C` is the **only** writer to `column.changed_ticks` for `C`. No atomic needed for the per-row tick store. The `UnsafeCell` documents the interior mutability across the shared archetype reference.

The **global** change tick (the counter source) is the only shared write site. Two designs:

**Design G1 — atomic per-system bump (Bevy pattern)**:
- World holds `change_tick: AtomicU32`.
- Each system, at dispatch, does `let this_run = world.change_tick.fetch_add(1, Relaxed)` and snapshots it as `SystemMeta::this_run`.
- Cost: 1 atomic per system per frame. At 100 systems × 60 FPS = 6,000 atomics/sec. Trivial.
- Memory ordering: `Relaxed` suffices because the value's only role is comparison against per-row tick stores, and Phase 9 already enforces happens-before via the conflict graph for the data races we care about.

**Design G2 — per-Schedule pre-allocated tick range**:
- Schedule pre-allocates a tick range `[base, base + N_systems)` at frame start.
- Each system gets its slot by topological order index.
- No runtime atomics. The base counter advances by `N_systems` per frame.
- Trade-off: tick value reflects topological order, not actual run order — semantically identical for "changed since my last run" queries; subtle if you want "changed by system X specifically" introspection (rare).

Bevy chose G1 because it generalizes to ad-hoc tick consumers (one-shot systems, exclusive systems, queries from main thread). For boyko's tight schedule contract, G2 is simpler but G1 is the safer default.

### Per-component-write atomic? No

Bevy explicitly does not bump an atomic per write. The `Mut<T>::deref_mut` writes `this_run` (a local `u32` captured from `SystemMeta`) directly into the column's `UnsafeCell<Tick>` slot — no atomic. Soundness rests entirely on the access declaration: the system declared a write to `C`, the conflict graph ensures no other system reads or writes `(archetype, row).changed_ticks` for that `C`, and the `UnsafeCell` is the bridge.

For boyko this maps cleanly: the Phase 9 conflict graph already provides exactly this guarantee for component data. The tick stores extend it to a parallel column without new synchronization primitives.

### check_tick scan and the apply window

Bevy runs `check_ticks` once per frame **outside** any system — naturally fits boyko's apply-window. The scan is `&mut World` and the system world is quiesced. Cost per scan: O(total_ticks_stored). At 40 MB / 8 B per pair = 5M ticks × cache-line bandwidth ≈ few ms cold, much less if recently touched. Frequency: every `CHECK_TICK_THRESHOLD` (518.4M) world ticks. At 60 FPS × 100 systems/frame = 6000 ticks/sec → scan fires every ~24 hours of continuous play. Effectively never on a real frame budget.

---

## §7 False Positive Analysis

Three sources of false `Changed<T>` triggers, ordered by frequency:

### F1 — `&mut T` access without value change

Bevy's `Mut<T>::deref_mut` bumps `changed_tick` on every mutable deref. A system that takes `&mut T` and reads the value without modifying it triggers `Changed<T>` for that entity. Mitigations:

- `set_if_neq` (Bevy [PR #5373](https://github.com/bevyengine/bevy/pull/5373)) — compares and writes only on inequality. Adds `PartialEq` requirement.
- `bypass_change_detection` — escape hatch giving raw `&mut T` without the guard. From Bevy [PR #5635](https://github.com/bevyengine/bevy/pull/5635).
- User discipline — read via `&T` queries, write via `&mut T` only when actually mutating.

### F2 — Coarse storage granularity (DOTS only)

If storage is per-chunk or per-archetype, any write to any entity in that bucket marks all entities. Mitigated by the "previous component" mirror pattern.

### F3 — Spawning vs adding

`Added<T>` fires for any entity where `T` was just inserted, including via `spawn()`. Some users want "added via explicit `insert`" but not "spawned" — Bevy does not distinguish. [Issue #15070](https://github.com/bevyengine/bevy/issues/15070) discusses re-introducing a `Mutated<T>` filter that catches mutations but not initial adds, separating semantics.

### "Deep" change detection (byte comparison)

No mainstream ECS does this automatically because:
- Byte equality is wrong for `Vec`/`String`/anything with allocations.
- `PartialEq` adds a generic bound on every component.
- For POD components, `set_if_neq` is equivalent and explicit.

boyko could offer `ChangedDeep<T: PartialEq>` as a separate filter but the standard `Changed<T>` should follow Bevy's deref-bump semantics for predictability.

---

## §8 Storage Layout Options A/B/C/D — Comparison Matrix

| Option | Description | Bytes / entity / component | Filter cost | False positives | Implementation complexity |
|--------|-------------|--------------------------|-------------|-----------------|--------------------------|
| **A. Per-row ticks (Bevy)** | Two `Vec<UnsafeCell<Tick>>` parallel to each Column: `added_ticks[row]`, `changed_ticks[row]` | 8 (or 4 if changed-only) | O(matched_rows) — 1 compare per row, branch-predictable when all-cold | Only F1 (`&mut` without modify) | Medium — touches every `push`/`swap_remove`/`pop` path in `ComponentPool` |
| **B. Per-archetype ticks** | Single `(added_tick, changed_tick)` per archetype | 0 per entity (archetype-amortized) | O(matched_archetypes) — typically ≤ 1024 in boyko | High: any write to any entity in the archetype marks all | Low — one extra field in `Archetype` |
| **C. Per-column-per-archetype ticks** | One `(added, changed)` tick per `(archetype, component)` slot — 16 B per column in archetype | 0 per entity | O(matched_archetypes × tracked_components) — still bounded by archetype count | Medium: writes to component C in archetype A mark all entities in A as Changed<C>, but writes to other components don't pollute | Low — add to `Column` struct |
| **D. Hybrid B+A** | Per-archetype version (B) PLUS opt-in per-row ticks (A) for components flagged `#[derive(Component, PreciseChangeDetection)]` | 0 default; 8 when opted-in | O(matched_archetypes) fast-path; O(matched_rows) for precise components | Configurable | High — two parallel mechanisms with user-controlled selection |

### Detailed analysis

**Option A — per-row**

Storage: 8 B × N_entities × N_tracked_components. At 100k × 50 → 40 MB. Cache behavior: ticks are a separate Vec, so iterating "ticks only" (the filter pre-check before fetching components) loads only the tick column — 32 entities per cache line. Iterating "data + ticks" doubles fetches: one cache line for component data, one for tick — but still SoA, still SIMD-friendly. Bevy reported 32-106% wins from splitting added/changed (PR #6547) because most filter paths touch only one of the two.

Filter cost per row: a single u32 compare wrapped in `wrapping_sub`. Compiler typically lowers to 2 instructions; branch is highly predictable (almost always "not changed" in steady state). At ~1 cycle ≈ 0.3 ns/row on modern x86. 1M entities → 0.3 ms.

**Option B — per-archetype**

Storage: 16 B × N_archetypes. At 1024 archetypes (boyko's slab cap) → 16 KB total. Negligible.

Filter cost: 1 compare per matched archetype. Archetype count is the matched archetype list length from `QueryState`, typically ≤ a few dozen. Effectively free.

False-positive blast radius: archetypes can hold up to millions of entities in boyko. A single `&mut Position` write to one entity marks all entities in that archetype as `Changed<Position>`. Far worse than Unity DOTS (16 KB chunks ≈ 100 entities). This is the dealbreaker for option B in isolation.

**Option C — per-column-per-archetype**

Storage: 16 B × N_archetypes × max-N_columns-per-arch. At 1024 archetypes × 50 components per arch on average → 800 KB. Still negligible.

Filter cost: 1 compare per (matched_archetype, tracked_component). Same order as option B but slightly more compares.

False-positive radius: same as option B — entire archetype's worth of entities marked changed if any one is touched. Only the per-component dimension is gained.

**Option D — hybrid (B/C default + A opt-in)**

User chooses per-component whether to pay the per-row cost. `#[derive(Component)]` → archetype-level only. `#[derive(Component, PreciseTracking)]` → per-row ticks. Combines the cheap default with the precise option.

Implementation: two filter trait impls — `Changed<T>` consults `T::TRACKING_GRANULARITY` const and dispatches to archetype-level or row-level path. Per-row storage only allocated for opted-in components.

Trade-off: two code paths to maintain, two filter algorithms, surface area for bugs. Bevy considered hierarchical storage ([Issue #5097](https://github.com/bevyengine/bevy/issues/5097)) — proposing redundant ticks at archetype AND column AND row level for early-exit — but the PR has not landed; the complexity has been the blocker.

### Empirical alignment

- Bevy: A. Established, audited, parallelism-proven. False-positive rate low (only F1).
- Unity DOTS: chunk-level — analogue of B-with-finer-granularity. Accepted false-positive rate.
- flecs: pull side is table-level (analogue of B). push side (observers) is orthogonal.
- EnTT: signals are push (zero storage); observer collector materializes lists per-write.
- hecs (fork): A via parallel `Previous<T>` component copying — exact byte-comparison change detection but doubles storage and requires `Clone + PartialEq`.

---

## §9 `Ref<T>` / `Mut<T>` Deref Guard Pattern

From [Mut docs](https://docs.rs/bevy/latest/bevy/ecs/change_detection/struct.Mut.html), [Ref docs](https://docs.rs/bevy/latest/bevy/ecs/prelude/struct.Ref.html).

### `Mut<T>` shape

```
struct Mut<'w, T> {
    value: &'w mut T,
    ticks: TicksMut<'w>,   // &mut Tick for changed, &mut Tick for added,
                            //   plus copies of last_run / this_run
}

impl<T> Deref for Mut<'_, T> { type Target = T; fn deref(&self) -> &T { self.value } }

impl<T> DerefMut for Mut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // The hot path: bump the tick on first mutable deref.
        *self.ticks.changed = self.ticks.this_run;
        self.value
    }
}
```

The actual code is more elaborate (handles `bypass_change_detection`, `set_if_neq`, etc.) but the core is: **DerefMut writes `this_run` into the changed tick slot**. No comparison. No predicate. Just an unconditional store.

Cost: 1 store per `&mut T` access. Monomorphization eliminates the guard for `&mut T` queries where the user never deref-muts (e.g., reads via `.into_inner()` after early-checking `is_changed()`).

### `Ref<T>` shape

```
struct Ref<'w, T> {
    value: &'w T,
    ticks: Ticks<'w>,      // &Tick for changed, &Tick for added, last_run, this_run
}

impl<T> Ref<'_, T> {
    pub fn is_changed(&self) -> bool { self.ticks.changed.is_newer_than(self.ticks.last_run, self.ticks.this_run) }
    pub fn is_added(&self)   -> bool { self.ticks.added.is_newer_than(self.ticks.last_run, self.ticks.this_run) }
}

impl<T> Deref for Ref<'_, T> { type Target = T; fn deref(&self) -> &T { self.value } }
```

Pure read-only. The change-detection bits are exposed as methods. Replaces `&T` in `Query<Ref<T>>` slots when the user wants to ask change questions without filtering.

### Simplifications for boyko

For options B/C (archetype-level), `Mut<T>` doesn't need per-row tick references — just an `*mut Tick` to the archetype's tick slot. For option A, the per-row pointer is mandatory.

The `set_if_neq` mitigation requires `PartialEq` and is opt-in — Bevy implements it as a separate method on `DetectChangesMut`, not in `DerefMut`. boyko can mirror this exactly.

`into_inner` consumes the `Mut<T>` and produces `&'w mut T` while bumping the tick — used when forwarding to APIs that take `&mut T` directly.

---

## §10 Integration with Phase 8b `Query<D, F>` Filter Pipeline

Boyko's current `QueryFilter` trait (read from filter.rs):

```
unsafe trait QueryFilter: Sized {
    type State: Send + Sync + 'static;
    type Fetch<'w>: Copy;
    const IS_ARCHETYPAL: bool;
    fn init_state(world: &mut EcsMaster) -> Self::State;
    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet);
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool;
    fn aggregate_include(...);
    fn aggregate_exclude(...);
    unsafe fn set_table_readonly<'w>(fetch, state, archetype: *const Archetype);
    unsafe fn set_table_mut<'w>(fetch, state, archetype: *mut Archetype);
    unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool;
}
```

The current Phase 8b filters (`()`, `With<C>`, `Without<C>`, tuple-as-AND, `Or<(...)>`) all have `IS_ARCHETYPAL = true`. The iterator's hot loop already has `if const { F::IS_ARCHETYPAL } { return true; }` const-folding away `filter_fetch` entirely when applicable.

### Adding `Added<C>` / `Changed<C>`

Both have `IS_ARCHETYPAL = false`. The `Fetch<'w>` must cache (for option A):
- Base pointer to the archetype's `added_ticks` column (for `Added`) or `changed_ticks` column (for `Changed`).
- A copy of the system's `last_run` and `this_run` ticks (taken from system metadata at iteration start).

`filter_fetch(fetch, row) → bool` then reads `fetch.tick_base[row]` and computes `tick.is_newer_than(last_run, this_run)`.

`matches_component_set` returns true (the archetype must contain `C` — same archetypal predicate as `With<C>` because you cannot filter "changed" on an absent component).

`init_access` declares a component read of `C` (the filter reads the tick column, which conceptually depends on the component's lifecycle).

`aggregate_include` sets the bit for `C` — Added/Changed entail "the component is present in the archetype."

`IS_ARCHETYPAL = false` causes the const-fold branch in the iterator to take the per-row predicate path. Existing `With<C>` siblings remain archetypal-only.

### `Or<(Added<A>, Changed<B>)>`

Current code (filter.rs line 578): `IS_ARCHETYPAL = true && Added::IS_ARCHETYPAL && Changed::IS_ARCHETYPAL = false`. The const-fold detects the mixed case and falls through to per-row OR fold. Note the existing comment in filter.rs:

> "Phase 10's `Changed<C>` / `Added<C>` filters will retrofit `Or` to short-circuit on the first per-row `filter_fetch` match"

The existing tuple-as-AND impl (line 480-489) already short-circuits using `&&`. The `Or` impl (line 651-654) already short-circuits using `||`. Both compose correctly with non-archetypal elements once `Added`/`Changed` land.

### `Ref<T>` / `Mut<T>` as SystemParam

These are `QueryData` impls, not `QueryFilter`. They cache one extra pointer (the tick column base) alongside the data pointer. The per-row `fetch(fetch, row)` returns `Ref<'w, T> { value, ticks }` instead of `&'w T`. The `Mut<T>` impl additionally captures `this_run` for the deref-guard write path.

`IS_READ_ONLY` for `Ref<T>` = true (no writes, just tick reads). For `Mut<T>` = false (potential tick write via deref guard).

### Per-archetype state for Added/Changed

For option B/C (archetype-level), `set_table_readonly` / `set_table_mut` reads the archetype's tick once and stores it in `Fetch<'w>`. `filter_fetch(_, _)` returns the cached bool — no per-row work. `IS_ARCHETYPAL` could even be `true` for option B/C because the decision is invariant within the archetype.

This is a strong simplification argument for option B/C: filters remain archetypal, the const-fold optimization in the iterator stays effective, and the per-row hot loop is unchanged.

### Cost analysis for the hot loop

Phase 8b's `QueryIter::next` already calls `D::fetch(fetch, row)` and (for non-archetypal F) `F::filter_fetch(fetch, row)`. Adding `Changed<C>`:

- Option A per-row: extra L1d load (tick) + extra compare (`u32::wrapping_sub` × 2 + `>`) + branch. Predictable, autovectorizable on `for_each`. Cost: 1-2 ns/row cold, ~0.3 ns/row hot. Bevy benchmarks confirm this order of magnitude (PR #6547 commentary).
- Option B/C: zero per-row cost, archetype-level branch only.

---

## §11 Comparison Table — Proposed boyko vs Bevy vs flecs vs Unity DOTS

| Aspect | Bevy | flecs (pull) | Unity DOTS | Proposed boyko (option pending) |
|--------|------|-------------|------------|--------------------------------|
| **Granularity** | Per row | Per table (= archetype) | Per chunk (16 KB) | A: per row / B: per archetype / C: per (archetype, column) / D: hybrid |
| **Tick type** | `Tick(u32)` wrapping | `int32_t` counter, unbounded | `uint` version | TBD — likely `u32` mirror, `u64` candidate |
| **Storage cost per (entity, component)** | 8 B (2 × u32) | 0 (per-table) | 0 (per-chunk amortized) | A: 8 B / B,C: 0 |
| **Filter cost per row** | ~0.3 ns (predictable u32 compare) | 0 (table-level check) | 0 (chunk-level check) | A: ~0.3-1 ns / B,C: 0 |
| **Bump site** | `Mut<T>::deref_mut` (no atomic) | Explicit `modified()` or `set()` | Write query execution (immediate) | A: deref guard / B,C: per-archetype counter |
| **Global counter** | `AtomicU32` per system bump | Global int32, no atomic | uint, no atomic | TBD — atomic per system or per-Schedule preallocation |
| **Wraparound handling** | `MAX_CHANGE_AGE` = `u32::MAX - (2 × CHECK_TICK_THRESHOLD - 1)`, `check_ticks` scan every 518.4M ticks | None — int32, unbounded counter | None — uint reset per app session | TBD — same Bevy pattern recommended for u32 |
| **False positives** | F1 only (`&mut` without modify) | F1 only on user push side | Chunk-coarse + write-permission-only | A: F1 / B,C: archetype-wide blast |
| **Parallel safety** | UnsafeCell + conflict graph | Single-thread default | Job system + access tracking | Phase 9 conflict graph (already exists) |
| **`Or<F>` composability** | Native via filter type system | Manual via per-table flags | Limited (2-component hard cap) | Native via existing Phase 8b `Or` impl |
| **`Ref<T>` / `Mut<T>` available** | Yes | No | N/A | Yes (planned Phase 10E) |
| **Removal detection** | `RemovedComponents<T>` separate event buffer | `OnRemove` observer | Removed entities in chunk version logic | Out of Phase 10 scope (deferred) |
| **Bevy versions referenced** | 0.14+ (recent split-column work); `u64` migration proposed [Issue #15683](https://github.com/bevyengine/bevy/issues/15683) but blocked | 4.1+ | 1.0+ | n/a |

---

## §12 Concrete Number Targets for Phase 10 Acceptance

Based on cross-engine empirical data — targets the architect can validate the design against:

**Storage overhead**
- Option A: ≤ 8 B per (entity, tracked component). At 100k × 50 → 40 MB total (acceptable per most game budgets — Bevy ships this).
- Option B: ≤ 16 B per archetype. At 1024 archetypes → 16 KB total. Negligible.
- Option C: ≤ 16 B per (archetype, column). 800 KB worst-case. Negligible.

**Filter latency targets**
- Option A `Changed<T>` cost per row when tick is recent (predicted): ≤ 1 ns/row (Bevy reports compatible numbers; autovectorizes when inlined into `for_each`).
- Option A `Changed<T>` cost per row when tick is far (cold): ≤ 2 ns/row (cache miss to fetch tick column on archetype boundary, then sequential).
- Option B/C: ≤ 100 ns total per query iteration regardless of entity count (archetype-level check only).

**Tick maintenance**
- Per-system tick bump: 1 `Relaxed` atomic load+add — ≤ 5 ns at low contention. Bounded by N_systems.
- `check_ticks` scan: fires every `CHECK_TICK_THRESHOLD = 518.4M` ticks. At 60 FPS × 100 systems = 6,000 ticks/sec → scan once per ~24 hours. Cost is O(stored_ticks) on a quiesced world — Bevy reports < 10 ms for typical worlds.

**False-positive ceiling** (specific to option A)
- `&mut T` without modify in a system loop: 100% false-positive rate on those rows. Acceptable per Bevy contract; users use `set_if_neq` if they care.
- `Added<T>` survives the next `Schedule::run`: by definition true positive on first observation, then false on subsequent runs until `check_ticks` rotates. Standard.

**Parallel scheduler integration**
- Phase 9 conflict graph already enforces mutual exclusion on `(archetype, component)` access. Tick stores piggyback on this — no new atomics on the per-row write path.
- Global `change_tick: AtomicU32` — 1 atomic per system per frame. At 100 systems × 60 FPS → 6,000 atomics/sec. Trivially noise-floor.

**Wraparound budget**
- Bevy `MAX_CHANGE_AGE` ≈ 3.26B ticks. At 1k ticks/sec sustained (heavy schedule) → 37 days continuous before risk. The `check_ticks` scan handles the case anyway. Effectively infinite.

---

## §13 Open Architectural Questions for the Architect

1. **Storage option (A/B/C/D).** The single most consequential decision. §8 details. The author's recommendation discipline is not part of this brief — the architect chooses given the perf-first philosophy. Key signal: Bevy's Issue #5097 explicitly notes O(n) per-row scan is a real pain point at scale; redundant hierarchical storage (close to option D) is proposed but unshipped. Unity DOTS sidesteps with chunks; flecs sidesteps with table-level. Bevy alone pays the per-row cost — by far the most expressive but also the most expensive.

2. **`Tick` size: u32 or u64?** Bevy stays on u32 + wraparound machinery (Issue #15683 documents the u64 migration is blocked). For boyko: u32 saves 50% storage; u64 eliminates `MAX_CHANGE_AGE` and `check_ticks` scan entirely. The architect must decide if the 4 B savings (option A: 4 × N_entities × N_components → 20 MB at 100k × 50) justifies the wraparound complexity. AtomicU64 is platform-conditional (some 32-bit targets lack it) — boyko targets x86_64 per CLAUDE.md so this is moot.

3. **Per-system tick assignment policy.** G1 (atomic increment per system, Bevy pattern) vs G2 (pre-allocated range from Schedule). §6 details. G1 supports ad-hoc tick consumers; G2 is lock-free.

4. **`Added` vs `Changed` semantics intersection.** Bevy's `Changed<T>` fires for both add and modify (PR #15070 proposes a third filter `Mutated<T>` for "modified but not added"). Does boyko want the same Bevy semantics or a clean split from day one? Architect call.

5. **Removal detection (`RemovedComponents<T>`) scope.** Bevy has it as a separate event buffer. Phase 10 brief mentions Added/Changed only. Out of scope or in?

6. **Resource change detection.** Bevy applies the same `Ref<T>`/`Mut<T>` pattern to `Res<T>`/`ResMut<T>`. Phase 8a's `Resources` should integrate the same model. In scope for Phase 10 or deferred?

7. **`set_if_neq` and `bypass_change_detection`.** Both are Bevy escape hatches for the false-positive problem. Worth shipping in Phase 10 or as a Phase 10.5 polish?

8. **`#[derive(Component, NoChangeTracking)]` opt-out.** Bevy supports this for components that never benefit from tracking (e.g., never queried with Changed). Saves the 8 B storage. In scope?

9. **Per-row state for non-archetypal filters in `Fetch<'w>`.** Current Phase 8b filter `Fetch<'w>` is `()` for archetypal filters. Adding Changed/Added requires it to hold the tick column base pointer and the `(last_run, this_run)` snapshot. The `Copy` requirement on `Fetch<'w>` (current filter.rs:77) is compatible. No trait-shape changes required.

10. **Tick column zero-initialization.** When an entity is added to an archetype, `added_ticks[row]` and `changed_ticks[row]` must be initialized to the current `this_run` — the entity's "I just appeared" mark. The existing `Archetype::create_entity` path doesn't know about ticks. Phase 10 must thread the current tick through entity creation.

---

## §14 References

### Bevy ECS

- [bevy_ecs source root](https://github.com/bevyengine/bevy/tree/main/crates/bevy_ecs/src) — top-level module organization.
- [Tick docs (docs.rs)](https://docs.rs/bevy/latest/bevy/ecs/component/struct.Tick.html) — Tick type, MAX_CHANGE_AGE, is_newer_than.
- [Column docs](https://docs.rs/bevy/latest/bevy/ecs/storage/struct.Column.html) — split tick storage.
- [Changed filter docs](https://docs.rs/bevy/latest/bevy/ecs/query/struct.Changed.html) — non-archetypal warning.
- [Added filter docs](https://docs.rs/bevy/latest/bevy/ecs/query/struct.Added.html) — non-archetypal warning.
- [Mut docs](https://docs.rs/bevy/latest/bevy/ecs/change_detection/struct.Mut.html) — deref guard pattern.
- [SystemChangeTick docs](https://docs.rs/bevy/latest/bevy/ecs/system/struct.SystemChangeTick.html) — per-system tick API.
- [CHECK_TICK_THRESHOLD docs](https://docs.rs/bevy/latest/bevy/ecs/change_detection/constant.CHECK_TICK_THRESHOLD.html) — 518,400,000 value.
- [DetectChangesMut docs (set_if_neq)](https://docs.rs/bevy/latest/bevy/ecs/change_detection/trait.DetectChangesMut.html).
- [Bevy Cheat Book — Change Detection](https://bevy-cheatbook.github.io/programming/change-detection.html) — user-facing semantics.
- [PR #6547 — Split Component Ticks](https://github.com/bevyengine/bevy/pull/6547) — the storage split, 32-106% benchmarks.
- [PR #3956 — Make change lifespan deterministic](https://github.com/bevyengine/bevy/pull/3956) — MAX_CHANGE_AGE, check_tick.
- [PR #7561 — Rename is_older_than to is_newer_than](https://github.com/bevyengine/bevy/pull/7561) — corrected naming.
- [PR #5373 — Add set_if_neq](https://github.com/bevyengine/bevy/pull/5373) — false-positive mitigation.
- [PR #5635 — bypass_change_detection](https://github.com/bevyengine/bevy/pull/5635) — escape hatch.
- [Issue #5097 — Accelerate change detection by redundantly storing ticks](https://github.com/bevyengine/bevy/issues/5097) — proposed hierarchical storage (option D direction).
- [Issue #15683 — Use u64 for change ticks](https://github.com/bevyengine/bevy/issues/15683) — u32→u64 migration discussion.
- [Issue #15070 — Separate Changed and Added, re-adding Mutated](https://github.com/bevyengine/bevy/issues/15070) — semantics split discussion.
- [PR #11173 — Fair change detection benchmarking](https://github.com/bevyengine/bevy/pull/11173) — benchmark methodology.

### flecs

- [Observers Manual](https://github.com/SanderMertens/flecs/blob/master/docs/ObserversManual.md) — OnSet, hooks.
- [Queries.md (change tracking section)](https://github.com/SanderMertens/flecs/blob/master/docs/Queries.md) — per-table dirty counters, query.changed().
- [change_tracking example](https://github.com/SanderMertens/flecs/blob/master/examples/cpp/queries/change_tracking/src/main.cpp) — API usage.
- [flecs 4.1 release notes](https://ajmmertens.medium.com/flecs-4-1-is-out-fab4f32e36f6) — 2× change-detection overhead reduction, Modified skip optimization.
- [flecs FAQ](https://github.com/SanderMertens/flecs/blob/master/docs/FAQ.md).

### Unity DOTS

- [Chunk's Change Version (gametorrahod)](https://gametorrahod.com/change-version/) — implementation reference, false positives.
- [Unity ECS chunk versioning (5argon, Medium)](https://medium.com/@5argon/unity-ecs-creating-an-efficient-system-with-chunk-iteration-didaddorchange-didchange-221427f5361b) — DidAddOrChange semantics.
- [EntityQuery filters docs](https://docs.unity3d.com/Packages/com.unity.entities@1.0/manual/systems-entityquery-filters.html) — WithChangeFilter API, 2-component limit.
- [Filtering data docs](https://docs.unity3d.com/Packages/com.unity.entities@1.0/manual/iterating-entities-foreach-filtering.html).

### EnTT

- [EnTT main repo](https://github.com/skypjack/entt).
- [Crash Course: ECS](https://skypjack.github.io/entt/md_docs_2md_2entity.html) — signals, observers.
- [Signal source header](https://skypjack.github.io/entt/observer_8hpp_source.html) — observer.hpp.
- [Events, signals wiki](https://github.com/skypjack/entt/wiki/Events,-signals-and-everything-in-between).
- [Signal and Event System (DeepWiki)](https://deepwiki.com/skypjack/entt/5-signal-and-event-system) — delegate / sigh memory overhead.

### Other Rust ECS

- [hecs ChangeTracker source](https://github.com/Ralith/hecs/blob/master/src/change_tracker.rs) — Previous<T>-mirror byte-comparison approach.
- [smokku/hecs fork with bevy-ported change tracking](https://github.com/smokku/hecs).
- [hecs change tracking issue #174](https://github.com/Ralith/hecs/issues/174).
- [Shipyard repo](https://github.com/leudz/shipyard) — alternative Rust ECS, sparse-set based.

### boyko-engine local files (consulted, no edits)

- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\filter.rs` — current `QueryFilter` trait shape; note the existing line 386-388 comment anticipating Phase 10.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\data.rs` — current `QueryData` trait shape, `ReadFetch` / `WriteFetch` reference for adding tick column pointers.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype.rs` — `Column` struct (16 B with `_reserved: u32`), `Archetype` with columns at offset 0, `entity_count`, `current_index`. Note `Column._reserved` is "reserved for Phase 8; do not rely on its current value" — Phase 10 might consume those 4 bytes for per-(archetype,column) tick if option C is chosen, but a separate parallel array is cleaner.
- `D:\claude\BoykoEngine\docs\PHASE-9-PARALLEL-SCHEDULER-PLAN.md` — Phase 9 conflict graph contract (referenced for §6).

Sources:
- [Bevy ECS source root](https://github.com/bevyengine/bevy/tree/main/crates/bevy_ecs/src)
- [Tick docs (docs.rs/bevy)](https://docs.rs/bevy/latest/bevy/ecs/component/struct.Tick.html)
- [Column docs (docs.rs/bevy)](https://docs.rs/bevy/latest/bevy/ecs/storage/struct.Column.html)
- [Changed filter docs](https://docs.rs/bevy/latest/bevy/ecs/query/struct.Changed.html)
- [Added filter docs](https://docs.rs/bevy/latest/bevy/ecs/query/struct.Added.html)
- [Mut docs](https://docs.rs/bevy/latest/bevy/ecs/change_detection/struct.Mut.html)
- [SystemChangeTick docs](https://docs.rs/bevy/latest/bevy/ecs/system/struct.SystemChangeTick.html)
- [CHECK_TICK_THRESHOLD docs](https://docs.rs/bevy/latest/bevy/ecs/change_detection/constant.CHECK_TICK_THRESHOLD.html)
- [DetectChangesMut trait](https://docs.rs/bevy/latest/bevy/ecs/change_detection/trait.DetectChangesMut.html)
- [Bevy Cheat Book — Change Detection](https://bevy-cheatbook.github.io/programming/change-detection.html)
- [Bevy PR #6547 — Split Component Ticks](https://github.com/bevyengine/bevy/pull/6547)
- [Bevy PR #3956 — Deterministic change lifespan](https://github.com/bevyengine/bevy/pull/3956)
- [Bevy PR #7561 — Rename is_newer_than](https://github.com/bevyengine/bevy/pull/7561)
- [Bevy PR #5373 — set_if_neq](https://github.com/bevyengine/bevy/pull/5373)
- [Bevy PR #5635 — bypass_change_detection](https://github.com/bevyengine/bevy/pull/5635)
- [Bevy Issue #5097 — Hierarchical tick storage](https://github.com/bevyengine/bevy/issues/5097)
- [Bevy Issue #15683 — u64 for change ticks](https://github.com/bevyengine/bevy/issues/15683)
- [Bevy Issue #15070 — Separate Changed/Added/Mutated](https://github.com/bevyengine/bevy/issues/15070)
- [Bevy PR #11173 — Fair change detection benchmarking](https://github.com/bevyengine/bevy/pull/11173)
- [flecs Observers Manual](https://github.com/SanderMertens/flecs/blob/master/docs/ObserversManual.md)
- [flecs Queries.md](https://github.com/SanderMertens/flecs/blob/master/docs/Queries.md)
- [flecs change_tracking example](https://github.com/SanderMertens/flecs/blob/master/examples/cpp/queries/change_tracking/src/main.cpp)
- [flecs 4.1 release blog (Sander Mertens)](https://ajmmertens.medium.com/flecs-4-1-is-out-fab4f32e36f6)
- [Unity DOTS chunk version (gametorrahod)](https://gametorrahod.com/change-version/)
- [Unity ECS chunk versioning (5argon, Medium)](https://medium.com/@5argon/unity-ecs-creating-an-efficient-system-with-chunk-iteration-didaddorchange-didchange-221427f5361b)
- [Unity DOTS EntityQuery filters](https://docs.unity3d.com/Packages/com.unity.entities@1.0/manual/systems-entityquery-filters.html)
- [EnTT main repo](https://github.com/skypjack/entt)
- [EnTT Crash Course: ECS](https://skypjack.github.io/entt/md_docs_2md_2entity.html)
- [EnTT observer.hpp source](https://skypjack.github.io/entt/observer_8hpp_source.html)
- [EnTT Events, signals wiki](https://github.com/skypjack/entt/wiki/Events,-signals-and-everything-in-between)
- [EnTT Signal and Event System (DeepWiki)](https://deepwiki.com/skypjack/entt/5-signal-and-event-system)
- [hecs ChangeTracker source](https://github.com/Ralith/hecs/blob/master/src/change_tracker.rs)
- [smokku/hecs fork with change tracking](https://github.com/smokku/hecs)
- [hecs Issue #174 — change tracking](https://github.com/Ralith/hecs/issues/174)
- [Shipyard repo](https://github.com/leudz/shipyard)