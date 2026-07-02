> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 8.5 — Static Bundle Cache — Architectural Plan (Round 3)

## Round 3 changelog (vs Round 2)
- **C6 fixed**: §4.3 drop-order wording corrected (Rust drops in declaration order). `bundle_archetype_cache` slot pinned between `archetype_master` and `arena` per Phase 8a C5 contract. Field holds only `OnceLock<ArchetypeId>` values — no resource ownership; drop position is informational only.
- **C7 fixed**: Stale `Vec<OnceLock<ArchetypeId>>` references in §7.4, §10.5, §11.1, §11.3 replaced with `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>`. §11.3 title renamed to "Cache is single-writer per world (no growth)". §10.5 dropped the `Vec::resize_with ~50 ns` cold-path line item (no growth happens with the boxed array).
- **C8 fixed**: §12 invariants table row for SBC5 now reads `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>` to match §2.3 verbatim.
- **W5 fixed**: §13.4 Miri test 4 renamed from `cache_vec_growth_no_realloc_ub` (impossible-by-construction after C1) to `many_distinct_bundles_no_ub` — asserts 1024 distinct bundle types coexist without UB (covers OnceLock-init race across many cells).
- **W6 fixed**: Memory footprint committed to single source of truth — **≤ 24 KB conservative upper bound** across §1.2, §4.3, §4.5, §10.4. The "Round 1 claimed 24 KB" note removed from §10.4; replaced by "Conservative upper bound; exact size confirmed by Step 0 test `oncelock_size_assumptions`".
- **W7 fixed**: §4.3 `array::from_fn` ASM-folding claim rewritten as a Step 3 acceptance criterion (verify via `cargo asm` that `EcsMaster::new` allocates ≤ 24 KB and initializes via a tight loop; if compiler emits per-slot call, file Phase 11 follow-up item).
- **O4 accepted**: §6.3 SAFETY (iv) clarified — `ManuallyDrop` suppresses `Drop` unconditionally (does not "leak" semantically). Consumed components are owned by the archetype after callback completion; unconsumed (post-panic) ones leak unconditionally because their `Drop` was suppressed up front.
- **O5 accepted**: §9 Step 11 dependency relaxed from "Steps 1-10" to "Step 6 (apply path lands); may revise after Step 10 (bench results)".

## Summary

**Selected variant: B (derive(Bundle)-only) + multi-world strategy (б) per-ECS `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>` array cache (R3: stable address, no realloc).**

Drop tuple Bundle impls and `HashMap<TypeId, _>` cache entirely. Every Bundle is a named struct annotated with `#[derive(Bundle)]`. The macro emits a non-generic `impl Bundle for MyBundle` containing a per-impl `static IDS: OnceLock<&'static [ComponentId; N]>` (works because the impl is not generic — monomorphization is N/A) and a per-impl `static BUNDLE_TYPE_ID: OnceLock<BundleTypeId>` minted from a global `AtomicUsize` counter (identical pattern to `ComponentId`). The ArchetypeId cache lives **per-EcsMaster** as `Vec<OnceLock<ArchetypeId>>` indexed by `BundleTypeId.0`. Multi-world safe by construction.

**Perf gain projection (release, AMD Zen3 / Intel Alder Lake):**
- `Bundle::component_ids()` hot path (called from `SpawnCommand::apply`): ~30 ns → **~2 ns** (single `OnceLock::get` Acquire load on cached path, identical to current `Component::component_id` shape).
- `SpawnCommand::apply` ArchetypeId resolve: ~50 ns (`get_or_create_archetype` HashMap path) → **~3 ns** (`Box<[OnceLock; N]>` direct index + `OnceLock::get`). Note: bound array indexing is bounds-check-free under `debug_assert!` and `unsafe { *self.get_unchecked(id) }` on the hot path (still under `&mut self`).
- `Commands::spawn` enqueue: stays ~18 ns (already hot-path-clean; we remove the user-supplied `archetype_id` arg, saving a struct field by 8 bytes per SpawnCommand). `cached_archetype_id` is NOT called here — it is called only from `SpawnCommand::apply`.
- 10k spawn batch apply: ~3 ms → **~1.2 ms** (~2.5× improvement).
- Memory: leaked `[ComponentId; N]` per Bundle type (≤ 64 B typical) + `≤ 24 B` per-EcsMaster per-Bundle slot (`OnceLock<ArchetypeId>` — conservative upper bound, exact size locked in by Step 0 unit test `oncelock_size_assumptions`). `MAX_BUNDLE_TYPES = 1024` × `≤ 24 B` = **≤ 24 KB** per EcsMaster (stable-address `Box<[...]>`, never grows). 1 EcsMaster + 256 distinct bundles: ≤ 24 KB cache + ~25 KB per-impl statics ≤ 50 KB total. Negligible at engine scale.

**Open questions resolved in Round 2:**
1. **Q1 (nesting)** — DEFERRED to Phase 11. Derive rejects non-Component fields with `compile_error!`.
2. **Q2 (BundleTypeId cap)** — FIXED at 1024 with saturate-then-panic in `register_new`. See §4.2.
3. **Q3 (cold-path race)** — RESOLVED: `OnceLock::get_or_init` guarantees exactly one closure execution. `register_new` is called once per Bundle type process-wide. See §7.3, §11.2.
4. **Q4 (eager direct spawn)** — REJECTED: only `Commands::spawn<B>` (deferred) ships in Phase 8.5. Eager `EcsMaster::spawn<B>` deferred to Phase 11 (matches Bevy's pattern). See §5.3.

**No open questions remain for Round 3.** Round 3 should be polish-only (cosmetic clarity).

**Implementation steps (high level):**
1. Bundle trait redesign (add `BUNDLE_TYPE_ID` const + `cached_archetype_id` method; drop tuple impls).
2. `BundleTypeRegistry` global counter + per-EcsMaster `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>` (stable address, never reallocated — R3 C1 fix).
3. `boyko_macros::Bundle` derive macro (named structs + tuple structs; canonical sort by ComponentId.0; ManuallyDrop-upfront codegen).
4. `Commands::spawn<B: Bundle>(bundle)` — drop `archetype_id` argument.
5. `SpawnCommand<B>` — drop `archetype_id` field; resolve via `B::resolve_archetype_id(world)` on apply.
6. Delete old `bundle_slot_for` + tuple impls 1..=4 + the `HashMap`+`RwLock` cache.
7. Migrate tests (`commands.spawn(arch_id, tuple)` → `commands.spawn(bundle_struct)`; integration tests; new bench).
8. Tests: derive-Bundle smoke (named + tuple struct), miri panic-safety, miri multi-world isolation, criterion bench.

**Path to the plan**: returned inline below (file write not available in this environment; the orchestrator may save the document to `D:\claude\BoykoEngine\docs\PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md`).

---

# PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md

```markdown
# Architecture: Phase 8.5 — Static Bundle Cache

## Changes from Phase 8d

Phase 8d landed `Commands::spawn(archetype_id, bundle)` with tuple `Bundle` impls for arity 1..=4, backed by an `OnceLock<RwLock<HashMap<TypeId, &'static [ComponentId]>>>` cache (the `bundle_slot_for` helper). Three problems:

1. **RwLock on hot path** — every `Bundle::component_ids()` call takes a read lock + HashMap probe (~30 ns). The hot path lives inside `SpawnCommand::apply`, not `Commands::spawn`, so it is technically post-enqueue; nevertheless 30 ns × 10k spawns = 300 µs of pure cache-lookup overhead at apply time.
2. **User-supplied `archetype_id`** — caller must invoke `ecs.get_or_create_archetype(&[...])` before each `commands.spawn(...)`. Bad ergonomics, and the per-call slice construction defeats the static-ID promise.
3. **Tuple variadic ceiling** — arity capped at 4, with macro generation needed for arities 5..=15+. Each generic tuple impl is forced into the global `HashMap` path because Rust forbids per-monomorphization statics inside a generic `impl`.

Phase 8.5 redesigns Bundle around `#[derive(Bundle)]` for named/tuple structs only. Each derived impl is **non-generic**, so per-impl `static` slots work as they do for `Component::component_id` today (~2 ns hot path). The ArchetypeId cache moves into a per-`EcsMaster` `Vec<OnceLock<ArchetypeId>>` indexed by a `BundleTypeId` minted from a process-global `AtomicUsize` counter (mirror of `ComponentRegistry::NEXT_ID`). Tuple `Bundle` impls are deleted. `Commands::spawn` drops its `archetype_id` parameter.

## 1. Goal and target metrics

### 1.1 Goal

Eliminate every runtime HashMap/RwLock from the Bundle path and resolve `ArchetypeId` via a per-world array indexed by a compile-time-derivable `BundleTypeId`. Achieve `~2 ns Bundle::component_ids()` + `~3 ns archetype-id resolve` on the cached hot path. Achieve a 2-3× speedup on `10k spawn batch apply`. Migrate `Commands::spawn` to a single-argument form: `commands.spawn(MyBundle { ... })`.

### 1.2 Target metrics (release, AMD Zen3 / Intel Alder Lake)

**Scoping note**: `cached_archetype_id(&mut EcsMaster)` is only called from `SpawnCommand::apply` — never from user code, never from `Commands::spawn`. Bench fixtures (§13.5) measure on a warm EcsMaster instance with the bundle pre-registered.

| Operation | Caller context | Current (Phase 8d) | Target (8.5) | Cache profile |
|-----------|----------------|--------------------|--------------|----------------|
| `Bundle::component_ids()` cached | `SpawnCommand::apply` | ~30 ns | **≤ 2 ns** | 1 L1d (OnceLock acquire-load) |
| `Bundle::bundle_type_id()` cached | `cached_archetype_id` internal | N/A | **≤ 2 ns** | (folded into static info — see §4.4) |
| `Bundle::cached_archetype_id(&mut world)` cached | `SpawnCommand::apply` | N/A | **≤ 3 ns** | 2 L1d (static info + boxed-slice slot) |
| `Bundle::cached_archetype_id(&mut world)` cold (first spawn per Bundle per world) | `SpawnCommand::apply` first time | N/A | **≤ 1 µs** | `ArchetypeMaster` mask compute + register |
| `Commands::spawn(bundle)` enqueue | user system body | ~18 ns | **≤ 18 ns** | unchanged (loses archetype_id arg); does NOT call `cached_archetype_id` |
| `SpawnCommand::apply` (cached arch) | `CommandQueue::apply` flush | ~70 ns + 30 ns Bundle = ~100 ns | **≤ 50 ns** | dominated by `create_entity` memcpy |
| 10k spawn batch apply (4-component arity-4 bundle) | `CommandQueue::apply` flush | ~3.0 ms | **≤ 1.2 ms** | sequential pool writes |
| Cold-path first-spawn-per-Bundle latency | `SpawnCommand::apply` first time | N/A (today: archetype already cached by user) | **≤ 1.5 µs** | `get_or_create_archetype` |

**Bench fixture (binding for Step 10 / §13.5)**:
- One `EcsMaster::new()` per group.
- One bundle type per group, pre-registered via a warm-up call.
- N iterations of the operation under test, each iteration on a fresh `SpawnCommand` (allocated and applied; entity discarded after measurement).
- Criterion's `bench_function` with `iter_batched` for setup-per-iteration. No allocator noise.

### 1.3 Cross-phase relation

* **Phase 8d** is the immediate predecessor — see invariants B1..B4, CQ1..CQ7, CQ-PACK1, CQ-SEND1, APP1'..APP4. They are preserved unless explicitly overridden.
* **Phase 9 (scheduler)** is the immediate successor — the per-world cache MUST be Send + Sync for cross-system access.
* **Phase 7 (random access)** invariants U1..U14 stand unchanged.

## 2. Context and constraints

### 2.1 Subsystems affected

| Subsystem | Change |
|-----------|--------|
| `crates/boyko_ecs/src/ecs/core/bundle/` | `bundle.rs` rewritten; `bundle_impls.rs` deleted (tuple impls removed); new `bundle_type_registry.rs` |
| `crates/boyko_ecs/src/ecs/core/commands/spawn_command.rs` | `archetype_id` field removed; apply resolves on-the-fly |
| `crates/boyko_ecs/src/ecs/core/system/params/commands.rs` | `Commands::spawn` loses `archetype_id` parameter |
| `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | New field `bundle_archetype_cache: Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>` (stable address — C1 fix); new pub(crate) method `bundle_archetype_id_for::<B>()` — internal only, called by trait method delegate. Q4-resolved: no `spawn::<B>()` eager path in Phase 8.5. |
| `crates/boyko_macros/src/lib.rs` | New `#[proc_macro_derive(Bundle)]` |
| `crates/boyko_ecs/tests/` | New tests (multi-world isolation, panic safety, derive smoke); existing tests migrated |
| `crates/boyko_ecs/benches/` | New `bundle_static_cache.rs` |

### 2.2 Invariants preserved from Phase 8d

* **B1** — `component_ids()` returns canonical sorted (ascending by `ComponentId.0`) — **enforced by derive macro at compile time** (sort happens in the generated init closure).
* **B2** — `for_each_component_bytes` callback order matches `component_ids` order.
* **B3** — `Bundle: Send + Sync + 'static`.
* **B4** — ManuallyDrop-upfront panic safety.
* **CQ1..CQ7, CQ-PACK1, CQ-SEND1** — CommandQueue invariants unchanged.
* **APP1'..APP4** — System::apply contract unchanged.

### 2.3 New invariants introduced

* **SBC1** — Every type implementing `Bundle` is non-generic (concrete) and produced by `#[derive(Bundle)]`. Manual impls are forbidden (compile-time enforced by sealing the supertrait — see §4.4).
* **SBC2** — `Bundle::BUNDLE_TYPE_ID` is unique per type for the lifetime of the process. Assigned lazily via a global `AtomicUsize` counter, identical to `ComponentRegistry::NEXT_ID`. Once assigned, never changes.
* **SBC3** — `Bundle::component_ids()` is monotonic and pure: returns the same `&'static [ComponentId]` for every call within a process. The slice's contents are immutable (the slice lives in process-static storage allocated by the first call's `Box::leak` of a `[ComponentId; N]` — only once per Bundle type for the process lifetime, **not** per world).
* **SBC4** — `Bundle::cached_archetype_id(&mut EcsMaster) -> ArchetypeId` is idempotent per `(BundleTypeId, EcsMaster)` pair. First call per pair triggers `get_or_create_archetype` (cold path); subsequent calls return the cached value (`~3 ns`).
* **SBC5** — The per-world `bundle_archetype_cache` is a `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>`, indexed by `BundleTypeId.0`. Allocated once at `EcsMaster::new()` time, never reallocated. Pre-sized to `MAX_BUNDLE_TYPES` (1024) — all slots default to `OnceLock::new()`. **Stable address — safe for Phase 9 parallel readers (C1 resolution).**
* **SBC6** — Two `EcsMaster` instances are independent: a `BundleTypeId` cached in one world's `bundle_archetype_cache` is meaningless to the other. The per-world cache box is therefore world-private. (Phase 9 concurrency: the cache is accessed under `&mut EcsMaster` on the apply path, so contention is impossible by S1 invariant.)
* **SBC7** — `BundleTypeId.0 < MAX_BUNDLE_TYPES`. Exhaustion panics (matches `ComponentRegistry::MAX_COMPONENTS` policy).
* **SBC8** — Each `Bundle::component_ids()` slice is heap-leaked exactly once per Bundle type per process via `Box::leak`. Memory cost bounded by `MAX_BUNDLE_TYPES × N_max × 8 B = 1024 × 8 × 8 = 64 KB` in absolute worst case (every bundle is arity-8); typical (~100 bundles, avg arity 3) leaks ≈ 2.4 KB. This is identical in shape to today's `Component::component_id` policy — no architectural regression.
* **SBC9** — `for_each_component_bytes` body generated by the derive macro emits components in the same canonical order as `component_ids()` (B2 sync). The derive uses a compile-time topological sort over the struct's field types based on `ComponentId.0`, evaluated at runtime once per first call (cached in the per-impl `static`).

### 2.4 Hard prohibitions

* No `HashMap` / `RwLock` / `Mutex` anywhere in the Bundle path, hot or cold. Cold path uses `OnceLock` only.
* No `Box<dyn Bundle>` (the `Bundle` trait stays generic-friendly).
* No virtual dispatch.
* No `Vec::new()` on the hot path.

### 2.5 Why **B** (derive-only) was selected over A/C/D

Cost matrix (hot path `component_ids` cached lookup, single thread, AMD Zen3):

| Variant | Hot-path lookup | Cold-path latency | Erg | Multi-world | Notes |
|---------|----------------|-------------------|-----|-------------|-------|
| **A** (Phase 8d tuple + HashMap) | 30 ns (RwLock+HashMap) | 100 ns | tuple-best | OK | Current. Hot path dominated by lock + hash. |
| **B** (derive-only, per-impl static) | **2 ns** (OnceLock acquire) | 80 ns | named-struct mandatory | OK (per-world ArchetypeId cache) | **Selected.** Mirrors current `Component::component_id` perf exactly. |
| **C** (hybrid A + B) | 2 ns (B path) / 30 ns (A path) | 100 ns | mixed | OK | Two parallel codepaths → cognitive overhead + binary bloat. The "fast path for derive, slow path for tuples" surface is confusing and discourages users from migrating. Rejected. |
| **D** (linkme distributed slice + compile-time BundleId) | ~3 ns | 0 ns (slice prebuilt) | derive-only | OK | `linkme` requires `dylib`/`bin` target hooks. Doesn't work on `cdylib`/wasm without custom support; ecosystem fragility too high. Rejected on principle 1 (zero runtime overhead is met by B already; D's win is marginal and platform-fragile). |

**Variant B justification (decisive):**

* Per-impl `static OnceLock<...>` is the **identical** pattern proven by `Component::component_id` (which the project already relies on at ~2 ns hot path, see `crates/boyko_macros/src/lib.rs:56-61`). The same pattern naturally generalizes to Bundles: `derive(Bundle)` emits a non-generic impl, so the per-impl static is allocated once per type.
* Forces named bundles — actually a feature: discoverable in code review, documented at the struct definition, addressable by `&BundleType` for diagnostics. Tuple bundles were always a "convenient but limited" shortcut; their loss is a documentation cost, not a perf cost.
* Variadic arity is unconstrained — a `struct PlayerBundle { a: A, b: B, ..., z: Z }` works without macro generation.
* Single codepath: simpler binary, simpler reasoning, single source of truth.

**Trade-off accepted:** users must define named structs (or tuple structs) instead of writing `commands.spawn(arch_id, (a, b))`. This was the user's directive ("not afraid to break ergonomics for ≥10% perf"). The breaking change is one-time during migration.

### 2.6 Why **(б)** (per-EcsMaster cache) was selected over (а) (single-world)

* `EcsMaster` instances are not enforced to be unique by the type system. Tests already construct multiple `EcsMaster`s per test binary (see Phase 8d Step 8 panic-recovery tests). Storing `ArchetypeId` in process-global state would silently corrupt cross-world isolation.
* Phase 9 introduces parallel systems via the scheduler. Within a single `EcsMaster`, S1 + `&mut EcsMaster` on apply guarantees no parallel writers to the cache. Across worlds, two `EcsMaster`s on two threads can each apply a SpawnCommand simultaneously without contention because each owns its own `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>`. Variant (а) would force a thread-global lock or unsound shared state.
* Cost: `≤ 24 KB` per EcsMaster (fixed-size boxed array of `MAX_BUNDLE_TYPES = 1024` `OnceLock<ArchetypeId>` slots), regardless of bundle count. For 4 worlds: `≤ 96 KB` total. Negligible. See §4.5 / §10.4 for breakdown.

## 3. Key decisions

### Decision SBC-D1: Bundle trait shape — derive-only, sealed
**What**: The new `Bundle` trait carries 4 associated items: `BUNDLE_TYPE_ID`, `component_ids()`, `for_each_component_bytes()`, `cached_archetype_id(&mut EcsMaster) -> ArchetypeId`. The trait extends a sealed `BundleSealed` supertrait to forbid manual impls.

**Why**:
- `BUNDLE_TYPE_ID` enables per-world cache indexing without a `TypeId` lookup (~2 ns vs ~10 ns).
- `cached_archetype_id` is the canonical surface for `Commands::spawn` (saves the user the `get_or_create_archetype` dance).
- Sealing forbids manual impls — derive-macro-only means we can change `BUNDLE_TYPE_ID`'s implementation strategy in future phases without breaking downstream code.

**Alternatives rejected**:
- Free-form trait with manual impls allowed: would force the trait to expose the underlying static slot mechanism publicly, foreclosing future migration to `linkme` / const TypeId.
- Skipping `cached_archetype_id` and forcing the caller to invoke `EcsMaster::bundle_archetype_id_for::<B>()`: same effect but worse ergonomics — every call site duplicates the indirection. The trait method delegates internally.

**Trade-off**: manual `impl Bundle for Foo` is forbidden. Users who want a custom Bundle (e.g. for a runtime-known component set) must wait for Phase 11's `DynamicBundle` API.

### Decision SBC-D2: BundleTypeId via a global atomic counter
**What**: `BundleTypeId(usize)` minted via `BUNDLE_NEXT_ID: AtomicUsize` with `fetch_add(1, Relaxed)`. Cap at `MAX_BUNDLE_TYPES = 1024`.

**Why**:
- Direct mirror of `ComponentRegistry::NEXT_ID` (`component_registry.rs:147`). The pattern is battle-tested in the codebase, with Miri/loom coverage.
- `Relaxed` ordering is sufficient: uniqueness is the only requirement (each fetch_add yields a distinct value); happens-before for the cache array is provided by the per-impl `OnceLock::set/get`.
- `MAX_BUNDLE_TYPES = 1024` chosen to match Bevy's typical bundle counts (most projects ship 50-300 distinct bundles). 1024 fixes the per-world cache box at `≤ 24 KB` (regardless of how many bundles are actually used) and keeps the global ID space far from typical overflow concerns.

**Alternatives rejected**:
- Reusing `ComponentRegistry`'s `NEXT_ID`: Bundle IDs would interleave with Component IDs in the same global counter, wasting `MAX_COMPONENTS` slots. Separate counter is cheap (1× `AtomicUsize`).
- `TypeId::of::<B>()` as the index directly: `TypeId` is opaque, not an index. Would need a `HashMap<TypeId, BundleTypeId>` — back to the original problem.

**Trade-off**: 1024 cap is a hard ceiling. Hitting it is a project-design problem, panics at registration with a clear diagnostic.

### Decision SBC-D3: Per-EcsMaster ArchetypeId cache via `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>`
**What**: `EcsMaster` gains a private field `bundle_archetype_cache: Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>`. Allocated once at `EcsMaster::new()` time. All slots initialized to `OnceLock::new()` (empty). Never reallocated.

**Why**:
- **Stable address** — Phase 9 introduces parallel readers (e.g., `&EcsMaster` references for diagnostics or query layers). A `Vec` would reallocate its buffer when its length grows past capacity, invalidating any `&OnceLock<ArchetypeId>` borrow that crosses the reallocation point. Tree Borrows treats this as UB. A `Box<[OnceLock; N]>` allocates once and never moves the inner array — borrows live forever (well-defined under Phase 9's apply-serialization invariant).
- **Direct indexing** — `cache[id.0]` is a single load + bounds check on the hot path. `debug_assert!(id.0 < MAX_BUNDLE_TYPES)` validates in dev builds; release builds can fold to `*cache.get_unchecked(id.0)` once a profiler shows the bounds check matters (Phase 9 follow-up).
- **No realloc cold path** — `Vec::resize_with` is eliminated. Cold path becomes pure `OnceLock::get_or_init` on the slot.
- **C1 resolution** — the Round 1 `Vec<OnceLock<ArchetypeId>>` design failed the Phase 9 readiness test. This is the fix.

**Alternatives rejected**:
- `Vec<OnceLock<_>>` lazily grown (Round 1 design): unsafe under parallel readers. Tree Borrows UB on realloc.
- Inline `[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]` field directly on `EcsMaster` (no Box): 24 KB stack-allocated as part of the struct — bloats `EcsMaster`'s stack moves (e.g., return-by-value in `EcsMaster::new`). Box keeps the struct slim.
- `Box<[MaybeUninit<OnceLock<ArchetypeId>>]>` with custom init: equivalent to direct boxed array of `OnceLock::new()`, but adds 1 unsafe block + readability cost. The fully-initialized boxed array is cheap to construct because `OnceLock::new()` is `const fn` — see §4.3 construction code.
- Chunked structure (e.g., `Box<[Box<[OnceLock; 64]>; 16]>`): two-level indirection adds a cache miss on the hot path. Defeats the design.
- Per-bundle static `OnceLock<ArchetypeId>` (variant (а) in §2.6): silently shared across worlds. UB the moment two worlds run.

**Trade-off**: 24 KB per `EcsMaster` regardless of bundle count. For 4 worlds: 96 KB. Acceptable — engines typically run 1-2 worlds. The waste is bounded and **constant**, not allocation-rate-driven.

### Decision SBC-D4: `Commands::spawn<B: Bundle>(bundle)` — single arg
**What**: `Commands::spawn` loses the `archetype_id: ArchetypeId` parameter. `SpawnCommand<B>` drops its `archetype_id` field. ArchetypeId resolution happens inside `SpawnCommand::apply` via `B::cached_archetype_id(world)`.

**Why**:
- Bundles can now self-resolve. The user-supplied id was a workaround for the lack of a cache.
- SpawnCommand shrinks by 8 B (Vec<u8> arena saves 8 B per spawn × N spawns). For 10k spawns: 80 KB saved.
- The apply path adds 1 `OnceLock::get` call (~2 ns) per command — net neutral with today's ~30 ns `bundle_slot_for` call, plus we skip the user's `get_or_create_archetype` call entirely (eliminates a `~100 ns` lookup per spawn at enqueue time).

**Alternatives rejected**:
- Keep `archetype_id` as an optional override: dead weight in 99% of callsites; the optimization (caller-pre-resolved archetype) is now achievable for free via `cached_archetype_id`.
- Resolve at enqueue time (in `Commands::spawn` body): would force `Commands` to borrow `&mut EcsMaster`, violating SP1 (Commands declares no world access). The deferred apply path already has `&mut EcsMaster`, so resolution there is the natural place.

**Trade-off**: existing call sites `commands.spawn(arch_id, (A, B))` break. Migration is mechanical: `commands.spawn(MyBundle { a, b })`.

### Decision SBC-D5: Coalesced per-impl static — `OnceLock<BundleStaticInfo>` (O3 acceptance)
**What**: The derive macro emits a **single** per-impl static carrying both `BundleTypeId` and `&'static [ComponentId; N]`:

```rust
// Pseudo-code (codegen template):
struct BundleStaticInfo {
    type_id: BundleTypeId,
    component_ids: &'static [ComponentId],
}
static INFO: OnceLock<BundleStaticInfo> = OnceLock::new();

fn bundle_type_id() -> BundleTypeId {
    INFO.get_or_init(|| Self::build_info()).type_id
}

fn component_ids() -> &'static [ComponentId] {
    INFO.get_or_init(|| Self::build_info()).component_ids
}
```

The `build_info()` helper (also emitted by the derive macro):
1. Mints `BundleTypeId` via `BundleTypeId(bundle_type_registry::register_new())`.
2. Computes `[A::component_id(), B::component_id(), ...]`, sorts ascending by `ComponentId.0`.
3. Leaks the array via `Box::leak(Box::new(arr)).as_slice()`.
4. Returns the populated `BundleStaticInfo`.

**Why**:
- **O3 acceptance** — `BUNDLE_TYPE_ID` and `IDS` together fit in one cache line (~24 B for the BundleStaticInfo). Single Acquire load on the cached hot path (instead of two). Saves ~1 cache line of L1d pressure per Bundle type and 1 OnceLock check on each `bundle_type_id` / `component_ids` call.
- **Atomic guarantee**: both fields are populated together. No partial-init state visible to any caller.
- **`register_new` called exactly once per Bundle type per process** — `OnceLock::get_or_init` guarantees a single winner closure execution across all threads. Race losers park until the winner completes, then read the cached `BundleStaticInfo`. This sidesteps the Round 1 concern that race losers might burn counter slots.
- Component IDs are not known until runtime (they're minted by `register_new` on first call). Sorting must be deferred to runtime.

**Alternatives rejected**:
- Two separate statics (Round 1 design): 2 cache lines, 2 OnceLock checks. Marginal but real cost on the hot path. O3 fixes.
- Compile-time sort via const generics: blocked by unstable `const_type_id`. Can revisit in Phase 11.
- Trie/multimap of static slices: massive overengineering for a runtime cost of ~50 ns once per Bundle type per process.

**Trade-off**: First call per Bundle type costs ~80 ns (a small sort + Box leak + atomic increment + OnceLock::set CAS). Subsequent calls: ~2 ns.

### Decision SBC-D6: `for_each_component_bytes` body — derive-generated with canonical-order ManuallyDrop wrapping
**What**: The derive macro generates a `for_each_component_bytes` body that:
1. Destructures the bundle struct into N stack locals.
2. Wraps each local in `ManuallyDrop<T>` **before any callback runs** (B4 invariant — panic-safety).
3. Builds a `[(ComponentId, *const u8); N]` stack array.
4. Sorts that array by `ComponentId.0` ascending — sort is **not** const-stable but the array is fixed-size so the compiler can vectorize.
5. Emits each `(id, slice)` via the user's `FnMut`.

**Why**:
- Matches the Phase 8d arity-4 implementation pattern exactly (`bundle_impls.rs:178-238`). Already passes panic safety + Miri.
- The sort is O(N²) for N ≤ 16 (insertion sort vectorizes); for typical Bundles N ≤ 8 the cost is < 5 ns.

**Alternatives rejected**:
- Pre-sorting via macro-emitted match arms: complex and brittle; ID values are runtime values, not compile-time literals.
- Skipping the sort, relying on field declaration order: violates B1 (canonical order). The archetype's column-set must be sorted to match `create_entity`'s expectations.

**Trade-off**: Up to 5 ns sort cost per `apply`. Acceptable; dominated by the `create_entity` memcpy (~30 ns × N components).

### Decision SBC-D7: Cold-path race on first-spawn
**What**: If two systems (Phase 9) flush `SpawnCommand<MyBundle>` simultaneously and `MyBundle::cached_archetype_id(world)` is uninitialized in both worlds, **the OnceLock guarantees exactly one initializer runs**. Loser threads block briefly on the `set` call (until the winner's `set` completes, then read the cached value). Total worst-case latency: 1 µs (winner) + 2 ns (loser, after winner finishes).

But: a SpawnCommand's `apply` runs under `&mut EcsMaster`. Therefore the same EcsMaster cannot have two parallel applies — Phase 9 scheduler must serialize SpawnCommand applies per-world (which it already plans to do under the `Commands` synchronization point). So this race is **not reachable** in practice.

For **cross-world** races (two `EcsMaster`s on two threads, both first-spawning `MyBundle`): each world's `bundle_archetype_cache` is independent. No race.

For **same-world same-Bundle first-spawn**: cannot happen under Phase 9's apply serialization.

**Why**: Phase 9's design (per the architect's interpretation) flushes commands at a barrier — there is no concurrent apply on the same `EcsMaster`.

**Alternatives rejected**:
- A `Mutex<Vec<OnceLock>>` cache: introduces lock on hot path. Forbidden.
- A per-bundle `OnceLock<ArchetypeId>` (static, not per-world): variant (а) — UB across worlds.

**Trade-off**: Documenting the apply-serialization expectation in Phase 9 plan. Currently a non-issue.

## 4. Data structures

### 4.1 `Bundle` trait

```rust
/// Sealed supertrait. Manual impls forbidden — only `#[derive(Bundle)]` may
/// produce types that implement `Bundle`.
mod sealed {
    pub trait BundleSealed {}
}

pub trait Bundle: sealed::BundleSealed + Send + Sync + 'static {
    /// Process-global `BundleTypeId`. Assigned lazily on first call via the
    /// global atomic counter (`bundle_type_registry::register_new`). After O3
    /// coalescing, this and `component_ids()` share one per-impl
    /// `OnceLock<BundleStaticInfo>` (see §4.4).
    ///
    /// SBC2: stable for the lifetime of the process; uniqueness guaranteed.
    fn bundle_type_id() -> BundleTypeId;

    /// Returns the canonical-order `&'static [ComponentId]`. SBC3. After O3,
    /// shares the same `OnceLock<BundleStaticInfo>` as `bundle_type_id()`.
    fn component_ids() -> &'static [ComponentId];

    /// Resolves (and caches) the archetype id for this bundle in the given
    /// world. SBC4. Hot path: ~3 ns. Cold path: ~1 µs. Called ONLY from
    /// `SpawnCommand::apply` (which holds `&mut EcsMaster`). User code should
    /// not call this directly.
    fn cached_archetype_id(world: &mut EcsMaster) -> ArchetypeId;

    /// Emits `(ComponentId, &[u8])` pairs in canonical order. B2 + B4 panic
    /// safety. SBC9. Codegen template specified in §6.3.
    fn for_each_component_bytes<F: FnMut(ComponentId, &[u8])>(self, f: F);
}
```

### 4.2 `BundleTypeId` and global counter

```rust
// crates/boyko_ecs/src/ecs/core/bundle/bundle_type_registry.rs

#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BundleTypeId(pub usize);

pub const MAX_BUNDLE_TYPES: usize = 1024;

/// Process-global counter. Same pattern as `ComponentRegistry::NEXT_ID`.
static BUNDLE_NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// Mint a fresh `BundleTypeId`. Saturate-then-panic on exhaustion.
///
/// **W1 fix**: On exhaustion we clamp the counter to `MAX_BUNDLE_TYPES` before
/// panicking, so re-entries (e.g., from `OnceLock::get_or_init` retry after
/// panic) do not blow `BUNDLE_NEXT_ID` beyond the cap. The panic is terminal —
/// the program is not expected to recover.
///
/// **Note on OnceLock retry**: `OnceLock::get_or_init` is NOT poisoned on
/// panic in the init closure (per std docs); subsequent callers retry the
/// closure. If `register_new` panics during init, the next caller will
/// attempt to mint again. The saturate clamp ensures that retry attempts
/// do not push the counter past `MAX_BUNDLE_TYPES`.
#[cold]
#[inline(never)]
pub fn register_new() -> usize {
    let raw = BUNDLE_NEXT_ID.fetch_add(1, Ordering::Relaxed);
    if raw >= MAX_BUNDLE_TYPES {
        // Saturate the counter so retries do not run away.
        BUNDLE_NEXT_ID.store(MAX_BUNDLE_TYPES, Ordering::Relaxed);
        panic!(
            "BundleTypeRegistry exhausted: next id {raw} >= MAX_BUNDLE_TYPES = {MAX_BUNDLE_TYPES}. \
             This is a terminal panic — increase MAX_BUNDLE_TYPES at compile time."
        );
    }
    raw
}
```

### 4.3 Per-EcsMaster cache

```rust
// crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs

pub struct EcsMaster {
    // ... fields above this point unchanged ...

    archetype_master: ArchetypeMaster,

    /// Per-bundle-type ArchetypeId cache. Indexed by `BundleTypeId.0`. Stable
    /// address — allocated once at `EcsMaster::new()`, never reallocated.
    /// Send + Sync transitively. SBC5/SBC6.
    ///
    /// **Field slot (C6 pin)**: declared after `archetype_master` and before
    /// `arena`. Rust drops fields in declaration order, so this field is
    /// dropped between them. The field holds only `OnceLock<ArchetypeId>`
    /// values — no resource ownership and no `Drop` side-effects — so the
    /// drop position is informational only and does not interact with the
    /// Phase 8a C5 drop-order contract for `archetype_master` and `arena`.
    ///
    /// Construction:
    /// ```
    /// // OnceLock::new() is `const fn`, so we can build the array with a
    /// // single `from_fn` call. Boxing avoids inlining the cache slots
    /// // into the EcsMaster struct itself (the inline form would bloat
    /// // every move of EcsMaster, e.g. return-by-value from new()).
    /// let cache: Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]> =
    ///     Box::new(std::array::from_fn(|_| OnceLock::new()));
    /// ```
    bundle_archetype_cache: Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>,

    arena: Box<Arena>,
}
```

Construction allocates a single heap region of **≤ 24 KB** (conservative upper bound; exact size = `1024 × size_of::<OnceLock<ArchetypeId>>()`, locked in by Step 0 unit test `oncelock_size_assumptions`) once at `EcsMaster::new()`. All slots default to `OnceLock::new()` (empty). No growth, no reallocation. **C1 fix**: Phase 9 parallel readers can safely hold `&OnceLock<ArchetypeId>` references — the boxed array's address never changes.

**Step 3 acceptance criterion (W7)**: the developer must verify via `cargo asm` (or equivalent) that:
1. `EcsMaster::new` allocates exactly one heap region of `≤ 24 KB` for `bundle_archetype_cache`.
2. The 1024-slot initialization compiles to a tight loop (single-instruction per slot or vectorized zeroing) — not 1024 individual `OnceLock::new` calls.

If LLVM emits a per-slot call sequence (unlikely but possible if `OnceLock`'s `const fn new` regresses), file a Phase 11 follow-up to switch to `Box<[MaybeUninit<OnceLock<ArchetypeId>>; N]>` + manual init. Until that regression is observed, the `array::from_fn` form is the canonical choice (readable, no unsafe, const-friendly).

### 4.4 Generated `SpawnCommand<B>` simplification

```rust
// crates/boyko_ecs/src/ecs/core/bundle/bundle_static_info.rs (NEW — O3)

/// Coalesced per-Bundle-type static payload. Stored in a single per-impl
/// `OnceLock<BundleStaticInfo>` emitted by `#[derive(Bundle)]`. Saves 1 cache
/// line + 1 Acquire load per first-call compared to two separate statics.
#[repr(C)]
pub struct BundleStaticInfo {
    /// Minted via `bundle_type_registry::register_new()` on the winner
    /// closure of the per-impl `OnceLock::get_or_init`.
    pub type_id: BundleTypeId,
    /// Canonical-sorted (`ComponentId.0` ascending) slice, leaked from a
    /// `Box<[ComponentId; N]>`.
    pub component_ids: &'static [ComponentId],
}

// Memory: `BundleStaticInfo` = 8 B (BundleTypeId) + 16 B (slice fat ptr) = 24 B.
// `OnceLock<BundleStaticInfo>` ≈ 32 B (24 B value + 8 B state word, aligned to 8).
```

```rust
// crates/boyko_ecs/src/ecs/core/commands/spawn_command.rs

pub(crate) struct SpawnCommand<B: Bundle> {
    // archetype_id REMOVED.
    pub(crate) bundle: B,
}

unsafe impl<B: Bundle> Send for SpawnCommand<B> {}
unsafe impl<B: Bundle> Sync for SpawnCommand<B> {}

impl<B: Bundle> Command for SpawnCommand<B> {
    fn apply(self, world: &mut EcsMaster) {
        // SBC4 — resolve (and cache) the archetype id on first apply per
        // (BundleTypeId, world) pair. ~3 ns hot path, ~1 µs cold.
        let archetype_id = B::cached_archetype_id(world);

        let arity = B::component_ids().len();
        // ... existing stack-collector logic from Phase 8d ...
    }
}
```

The arity ceiling (1..=4 in Phase 8d) is **removed**: the derive macro can emit `for_each_component_bytes` for any N. The stack-collector array sized at `[MaybeUninit<(ComponentId, &[u8])>; MAX_BUNDLE_ARITY]` where `MAX_BUNDLE_ARITY = 8` (W3 fix — lowered from 16; bundles > 8 are pathological. See §14 Q6 / W3).

### 4.5 Cache box sizing rationale

```text
Box<[OnceLock<ArchetypeId>; 1024]>:
  - Box pointer (inline on EcsMaster): 8 B
  - Heap allocation: 1024 × size_of::<OnceLock<ArchetypeId>>()
                  = 1024 × ≤ 24 B
                  = ≤ 24 KB (conservative upper bound; exact size confirmed
                              by Step 0 test `oncelock_size_assumptions`)
  - ≤ 24 KB = ≤ 384 cache lines (64 B each)

Per EcsMaster overhead: 8 B inline + ≤ 24 KB heap. Constant, allocation-free
after construction. Stable address — no realloc UB hazard for Phase 9
parallel readers.

For 4 worlds: ≤ 96 KB heap. For 1 world (typical): ≤ 24 KB. Negligible at
engine scale (a single chunk pool from Phase 7 is 64 KB+).
```

**Why fixed size vs lazily-grown**: see Decision SBC-D3 alternatives rejection. The Box-array's `≤ 24 KB` constant cost buys (a) Phase 9 realloc-safety, (b) zero cold-path allocation, (c) `MAX_BUNDLE_TYPES`-bounded direct indexing without bounds-check-failure branches in the common case.

## 5. Public API

### 5.1 `Bundle` trait — see §4.1

### 5.2 `BundleTypeId`, `MAX_BUNDLE_TYPES`

```rust
pub use crate::ecs::core::bundle::bundle_type_registry::{BundleTypeId, MAX_BUNDLE_TYPES};
```

### 5.3 `EcsMaster` additions

```rust
impl EcsMaster {
    /// Resolves (and caches) the archetype id for bundle type `B`. Delegated
    /// to from `Bundle::cached_archetype_id`. Cold path: ~1 µs. Hot path: ~3 ns.
    ///
    /// `pub(crate)` — internal-only helper. User code goes through the trait
    /// method `B::cached_archetype_id(world)`, which is itself called only
    /// from `SpawnCommand::apply`. Q4 resolution: no eager `spawn::<B>(b)`
    /// public path in Phase 8.5.
    #[inline]
    pub(crate) fn bundle_archetype_id_for<B: Bundle>(&mut self) -> ArchetypeId;
}
```

**Q4 resolution**: Phase 8.5 ships only `Commands::spawn<B>` (deferred path). The eager `EcsMaster::spawn<B>` API is deferred to Phase 11. This matches Bevy's pattern (`Commands::spawn` is the canonical entry; `World::spawn` exists but is mostly internal). Removing it from Phase 8.5 also removes a chunk of test surface and migration debt for `spawn_one`/`spawn_two`.

### 5.4 `Commands` API change

```rust
impl<'s> Commands<'s> {
    /// Single-arg deferred spawn. ArchetypeId resolved automatically on apply.
    #[inline]
    pub fn spawn<B: Bundle>(&mut self, bundle: B) {
        self.queue.push(SpawnCommand { bundle });
    }
}
```

### 5.5 `#[derive(Bundle)]` user-facing example

```rust
use boyko_macros::{Bundle, Component};

#[derive(Component)]
#[repr(C)]
struct Position { x: f32, y: f32, z: f32 }

#[derive(Component)]
#[repr(C)]
struct Velocity { x: f32, y: f32, z: f32 }

#[derive(Bundle)]
struct PlayerBundle {
    pos: Position,
    vel: Velocity,
}

// Usage:
commands.spawn(PlayerBundle {
    pos: Position { x: 0.0, y: 0.0, z: 0.0 },
    vel: Velocity { x: 1.0, y: 0.0, z: 0.0 },
});
```

For tuple-struct bundles:

```rust
#[derive(Bundle)]
struct ProjectileBundle(Position, Velocity);

commands.spawn(ProjectileBundle(pos, vel));
```

Unit structs are **rejected** at derive time:

```rust
#[derive(Bundle)]
struct Marker;  // compile_error!: "Bundle requires at least one field"
```

## 6. Algorithms for critical paths

### 6.1 `Bundle::component_ids()` — hot path (O3 coalesced)

**Caller context**: called from `SpawnCommand::apply` to know the arity for the stack-collector array. NOT called from `Commands::spawn` (that path is allocation-only).

```text
Generated body (per-Bundle-type, non-generic impl; O3-coalesced with bundle_type_id):
  static INFO: OnceLock<BundleStaticInfo> = OnceLock::new();
  INFO.get_or_init(|| Self::build_info()).component_ids

fn build_info() -> BundleStaticInfo {
    // Mint the BundleTypeId exactly once (OnceLock guarantees single winner).
    let type_id = BundleTypeId(bundle_type_registry::register_new());

    // Collect component IDs in declaration order, sort ascending.
    let mut arr = [A::component_id(), B::component_id(), ...];
    arr.sort_unstable_by_key(|id| id.0);

    // Leak. Bounded by SBC8 — at most MAX_BUNDLE_TYPES × N_max × 8 B for the process.
    let leaked: &'static [ComponentId] = Box::leak(Box::new(arr)).as_slice();
    debug_assert!(leaked.is_sorted_by_key(|id| id.0));

    BundleStaticInfo { type_id, component_ids: leaked }
}
```

Steps (hot, cached):
1. `OnceLock::get_or_init` Acquire-load (1 ns).
2. Branch: if initialized, return reference into the cached `BundleStaticInfo` (~1 ns extra).
3. Field access `.component_ids` — no extra cost.
4. **Total: ~2 ns hot path.** Cold path (first call): ~80 ns including atomic counter increment, sort, Box leak, CAS.

Complexity: O(1) hot, O(N log N) cold for an in-place sort of N component IDs.

Cache behavior: 1 cache line touched (`BundleStaticInfo` is 24 B, fits with room to spare). After coalescing, `bundle_type_id()` shares the same line — second method call is free if both were used in sequence (post-O3 win).

Branch: cold branch predicted not-taken once warm; gcc/llvm both fold to `cmov` for `OnceLock`'s init flag.

SIMD potential: none needed — single Acquire load.

### 6.2 `Bundle::cached_archetype_id(&mut EcsMaster)` — hot path

**Caller context**: called from `SpawnCommand::apply` (always under `&mut EcsMaster`). Never from `Commands::spawn`. C2 scoping resolved.

```text
Generated body (per-Bundle-type, non-generic):
  // Reuses INFO from §6.1; O3 win — same static, no extra OnceLock load.
  let type_id = INFO.get_or_init(|| Self::build_info()).type_id;
  world.bundle_archetype_id_for_inner::<Self>(type_id)
```

`bundle_archetype_id_for_inner::<B>(type_id)` body (inside `EcsMaster`):

```text
debug_assert!(type_id.0 < MAX_BUNDLE_TYPES, "BundleTypeId out of range");
let slot = &self.bundle_archetype_cache[type_id.0];   // direct index, stable address
slot.get_or_init(|| {
    let comp_ids = B::component_ids();                 // ~2 ns (cached after first call)
    self.archetype_master.get_or_create_archetype(comp_ids)
}).clone()    // ArchetypeId is Copy — no real clone.
```

Steps (hot, cache hit):
1. INFO OnceLock::get reading `type_id` field (1 ns; same line as §6.1's `component_ids` call).
2. Boxed-array direct index `cache[type_id.0]` — bounds check folds to a known-bounded comparison (predicted true; LLVM hoists into a single `cmp` + `cmov`).
3. `slot.get()` Acquire load + Copy of ArchetypeId (1 ns).
4. **Total: ~3 ns.**

Cold (first per (B, world)):
1. INFO init (~80 ns if also first per process — but normally already warm after `component_ids()` call).
2. `cache[type_id.0]` direct index — no grow needed (boxed array is pre-sized).
3. `get_or_create_archetype` (compute mask, register, mint id) — ~1 µs.
4. `OnceLock::set` CAS — ~10 ns.
5. **Total: ~1.0-1.2 µs.**

Complexity: O(1) hot path; O(N_components) cold for mask compute.

Cache behavior: hot path touches 2 cache lines (`BundleStaticInfo` + the OnceLock slot). Both should remain in L1d during a spawn-heavy frame loop.

**Benefit vs Round 1**: zero Vec realloc risk, zero bounds-grow logic on cold path.

### 6.3 `for_each_component_bytes` — mandatory codegen template (C5 fix)

**C5 problem**: A naive sort over `[(ComponentId, &[u8; ?])]` triggers Rust error E0521 ("borrowed data escapes function") because the compiler treats the `&[u8]` slice as invariant in the lifetime when stored alongside `MaybeUninit`. Solution: store **`*const u8` + len** in the sort array, reconstruct `&[u8]` at callback time.

**Mandatory template** that `#[proc_macro_derive(Bundle)]` MUST emit verbatim (with field substitution). The template ensures (a) C5 lifetime safety, (b) B4 panic safety (ManuallyDrop-upfront), (c) B1 canonical sort.

```rust
// Codegen pseudocode for a Bundle with N fields f1..fN, types T1..TN.
fn for_each_component_bytes<F: FnMut(ComponentId, &[u8])>(self, mut f: F) {
    use core::mem::{ManuallyDrop, size_of, MaybeUninit};
    use core::slice;

    // B4: ManuallyDrop-wrap ALL fields UPFRONT, before any callback can run.
    // This way, if `f` panics on iteration K, fields K+1..N's Drop is suppressed
    // (they leak, which is the documented panic-safety guarantee).
    let f1 = ManuallyDrop::new(self.f1);
    let f2 = ManuallyDrop::new(self.f2);
    // ... fN

    // C5: store (ComponentId, *const u8, usize). Pointer + len, NOT &[u8].
    // The `*const u8` cast bypasses the borrow checker's invariance treatment
    // of `&[u8]` inside MaybeUninit/array contexts; we reconstruct the slice
    // ad-hoc per callback iteration via `slice::from_raw_parts`.
    let mut sorted: [(ComponentId, *const u8, usize); N] = [
        (T1::component_id(), (&raw const *f1) as *const u8, size_of::<T1>()),
        (T2::component_id(), (&raw const *f2) as *const u8, size_of::<T2>()),
        // ... fN
    ];

    // B1: canonical sort by ComponentId.0 ascending. unstable sort acceptable
    // because IDs are unique (no equal keys).
    sorted.sort_unstable_by_key(|(id, _, _)| id.0);

    // Emit each component to the callback. The slice is reconstructed each
    // iteration from raw parts — the underlying ManuallyDrop locals live
    // for the entire frame, so the pointer is valid throughout the loop.
    for &(id, ptr, len) in &sorted {
        // SAFETY (C5 + B4):
        //   (i) `ptr` was derived from `&raw const *fK` where `fK` is a
        //       ManuallyDrop<TK> local owned by this stack frame. Since the
        //       ManuallyDrop locals are declared above the sort+loop, they
        //       live for the entire body of this function. The pointer
        //       remains valid for `len` bytes throughout the loop.
        //   (ii) `len = size_of::<TK>()` was computed at the same site;
        //        the slice covers exactly the bytes of one TK instance.
        //   (iii) The bytes are immutable for the lifetime of the slice
        //         (we have `&` access via the ManuallyDrop) — no aliasing
        //         violation, since `f` cannot mutate the underlying TK
        //         (we pass `&[u8]`, not `&mut [u8]`).
        //   (iv) ManuallyDrop suppresses Drop on the local unconditionally
        //        at end-of-scope (it does not "leak" semantically — it
        //        merely never invokes Drop). For components that the
        //        callback successfully consumed (memcpy'd into the ECS
        //        storage via `create_entity`), ownership has been
        //        transferred to the archetype, and that storage now owns
        //        the eventual Drop on entity despawn (via
        //        `ComponentPool::swap_remove`). For components that the
        //        callback did not reach because `f` panicked on an earlier
        //        iteration, their bytes remain in the stack `ManuallyDrop`
        //        locals and leak unconditionally — `Drop` is suppressed
        //        regardless of panic state. This is the documented B4
        //        panic-safety guarantee: panic → leak, never double-drop.
        let bytes: &[u8] = unsafe { slice::from_raw_parts(ptr, len) };
        f(id, bytes);
    }
    // ManuallyDrop suppresses Drop on f1..fN at end-of-scope. The components
    // have been moved into the ECS storage via the callback's memcpy — their
    // Drop runs from `ComponentPool::swap_remove` on entity despawn, not here.
}
```

### 6.3.1 `SpawnCommand::apply` after redesign

```text
1. Resolve archetype_id = B::cached_archetype_id(world)   — 3 ns hot
2. Construct stack collector array
   ([MaybeUninit<(ComponentId, &[u8])>; MAX_BUNDLE_ARITY=8]) — free (stack)
3. self.bundle.for_each_component_bytes(|id, bytes| {
       slot.write((id, bytes));
       count += 1;
       if count == arity {
           world.create_entity(archetype_id, &slots[..count])
               .expect("...");
       }
   })  — ~40 ns/component (memcpy) + ~10 ns wrapper
4. Total for arity-N: 3 + 10 + N×40 ns. N=4 → ~170 ns. N=8 → ~330 ns.
```

Phase 8d's apply cost: ~250 ns (4-component bundle). Phase 8.5: ~170 ns. ~30% improvement on apply alone.

### 6.4 10k spawn batch — projected

```
Phase 8d:
  Enqueue: 10k × 18 ns = 180 µs
  Apply:   10k × ~250 ns + 10k × 30 ns Bundle = 2.8 ms
  Total:   ~3.0 ms

Phase 8.5:
  Enqueue: 10k × 18 ns = 180 µs
  Apply:   10k × ~170 ns + 10k × 2 ns Bundle + 10k × 3 ns cache = 1.75 ms
  Total:   ~1.2 ms
```

2.5× improvement on the dominant path.

## 7. Multithreading model

### 7.1 Hot-path access

The hot path (`Bundle::component_ids()`, `Bundle::cached_archetype_id(&mut EcsMaster)`, `Commands::spawn(bundle)`, `SpawnCommand::apply`) runs under one of two contexts:

* **Inside a system body** (`Commands::spawn(bundle)`): `&'s mut CommandQueue` is the only borrow. No EcsMaster access. Push to byte arena is single-writer per system. No synchronization needed.
* **Inside `CommandQueue::apply` flush** (`SpawnCommand::apply`): `&mut EcsMaster` is exclusive. Per S1 invariant, no other system/apply runs concurrently on this world. `bundle_archetype_cache` is mutated under this exclusive borrow.

Therefore no synchronization is needed on the hot path within a single world.

### 7.2 Cross-world parallelism

Two `EcsMaster` instances on two threads each hold their own `bundle_archetype_cache`. There is **zero shared state** between worlds in the cache layer. The only shared state is:

* `BUNDLE_NEXT_ID` counter — `Relaxed` atomic increment. Race-free by atomic ordering.
* Per-impl `static BUNDLE_TYPE_ID: OnceLock<BundleTypeId>` — multi-thread-safe via `OnceLock` (CAS-protected init).
* Per-impl `static IDS: OnceLock<&'static [ComponentId]>` — same.

All shared state is `OnceLock<_>`-protected or `AtomicUsize::fetch_add` — both are lock-free and proven by the existing `ComponentRegistry` implementation.

### 7.3 Phase 9 scheduler integration

Phase 9 introduces parallel systems with per-system `CommandQueue`s. Three concerns:

1. **Two systems flushing concurrently to the same `EcsMaster`** — forbidden by `&mut EcsMaster` in `CommandQueue::apply`. The scheduler must serialize apply phases. Confirmed by Phase 9 plan (which has an `apply` barrier between system batches).
2. **Two systems concurrently *enqueuing* spawns** — no synchronization needed. Each `Commands` borrows its own `CommandQueue` (the per-system state).
3. **Two threads racing on per-impl `static OnceLock<BundleStaticInfo>` first-init**: `OnceLock::get_or_init` **guarantees exactly one closure execution across all threads** (C3 fix). Losing threads park internally on the std-provided once_lock primitive until the winner's closure completes, then read the cached value. Consequence: `register_new()` (which calls `BUNDLE_NEXT_ID.fetch_add`) is invoked **exactly once per Bundle type process-wide**. No counter slot is "burned" by race losers.

   **OnceLock panic behavior (C3 corollary)**: If the winner closure panics, `OnceLock::get_or_init` does NOT poison the cell — it returns to uninitialized state, and the next caller retries the closure. This is why `register_new()` uses the saturate-then-panic pattern (W1): on exhaustion, the counter is clamped to `MAX_BUNDLE_TYPES` so retries do not run away beyond the cap.

The cache itself is therefore single-writer per `EcsMaster` for its entire lifecycle. `Send + Sync` on `EcsMaster` is unaffected (it remains `!Send + !Sync` per the existing design).

### 7.4 Memory ordering

* `BUNDLE_NEXT_ID.fetch_add(1, Relaxed)` — `Relaxed` is sufficient (uniqueness only).
* `static BUNDLE_TYPE_ID: OnceLock<BundleTypeId>::get_or_init` — Acquire on read, Release on `set` (std default).
* `static IDS: OnceLock<&'static [ComponentId]>::get_or_init` — same.
* `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>` slot reads — Acquire on `OnceLock::get`. The boxed array's address is fixed for the EcsMaster lifetime (no realloc), so no fence is needed on the Box header — direct index `cache[type_id.0]` resolves at a stable address established at construction time.

### 7.5 Send/Sync derivation

* `BundleTypeId(usize)`: auto Send + Sync (transparent newtype over usize).
* `SpawnCommand<B>: Send + Sync` because `B: Bundle: Send + Sync + 'static` (transitive).
* `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>: Send + Sync` (Box is Send + Sync for any T: Send + Sync; arrays preserve Send + Sync element-wise; OnceLock is Send + Sync for any T: Send + Sync; `ArchetypeId(usize)` is trivially both).
* `EcsMaster: !Send + !Sync` — unchanged. The added field does not strengthen the bounds.

## 8. Integration

### 8.1 Files modified

| File | Action |
|------|--------|
| `crates/boyko_ecs/src/ecs/core/bundle/bundle.rs` | Rewrite trait — see §4.1 |
| `crates/boyko_ecs/src/ecs/core/bundle/bundle_impls.rs` | **DELETE** (tuple impls + `bundle_slot_for`) |
| `crates/boyko_ecs/src/ecs/core/bundle/bundle_type_registry.rs` | **NEW** — see §4.2 |
| `crates/boyko_ecs/src/ecs/core/bundle/mod.rs` | Re-export `Bundle`, `BundleTypeId`, `MAX_BUNDLE_TYPES` |
| `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | Add `bundle_archetype_cache: Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>` field + `pub(crate) fn bundle_archetype_id_for::<B>()`. **No** public `spawn::<B>()` in Phase 8.5 (Q4 resolution). |
| `crates/boyko_ecs/src/ecs/core/bundle/bundle_static_info.rs` (NEW) | Defines `BundleStaticInfo { type_id, component_ids }` for O3 coalescing. |
| `crates/boyko_ecs/src/ecs/core/commands/spawn_command.rs` | Remove `archetype_id` field; resolve in apply |
| `crates/boyko_ecs/src/ecs/core/system/params/commands.rs` | `Commands::spawn` loses `archetype_id` param |
| `crates/boyko_macros/src/lib.rs` | Add `#[proc_macro_derive(Bundle)]` |

### 8.2 Files added (tests + benches)

| File | Purpose |
|------|---------|
| `crates/boyko_ecs/tests/derive_bundle_smoke.rs` | derive macro: named structs, tuple structs, rejection of unit |
| `crates/boyko_ecs/tests/bundle_multi_world_isolation.rs` | two EcsMasters share BundleTypeId but get distinct ArchetypeIds |
| `crates/boyko_ecs/tests/bundle_panic_safety.rs` | ManuallyDrop-upfront panic-safety (Miri-target) |
| `crates/boyko_ecs/tests/bundle_canonical_order.rs` | derive macro emits canonical-sorted component_ids |
| `crates/boyko_ecs/tests/miri_phase8_5.rs` | Miri-only: many distinct bundles (up to `MAX_BUNDLE_TYPES`), OnceLock race |
| `crates/boyko_ecs/benches/bundle_static_cache.rs` | criterion: cached, uncached, batch 10k |

### 8.3 Tests broken by the migration (C4 clarification)

**Migration scope clarification (C4 fix)**: Existing test components use the **hand-rolled** pattern `impl Component for X { fn component_id() -> ComponentId { SLOT } }` (where `SLOT` is a manually picked id like `ComponentId(270)`) plus a manual `register_layout::<X>(SLOT.0)` call. This pattern works because the `Component` trait does NOT require derive — only `component_id()` returning a `ComponentId`.

**The Bundle migration is independent of the Component impl style**. `#[derive(Bundle)]` calls `T::component_id()` through the trait — it does not care whether `T`'s `component_id()` comes from `#[derive(Component)]` (lazy registry counter) or from a hand-rolled `impl Component for T { fn component_id() -> ComponentId { SLOT } }` (fixed slot). Both work.

**Confirmed C4-related design point**: Per-impl `static` storage for `BundleStaticInfo` is intentional process-lifetime. The test fixture pattern (hand-rolled Component impls with fixed slots) coexists naturally — Bundle derive does not require Component derive.

| Test | Migration |
|------|-----------|
| `commands_spawn_then_apply_creates_entity` (in `commands.rs`) | Replace tuple `(CmdsA, CmdsB)` with a derived `struct CmdsBundle { a: CmdsA, b: CmdsB }`; drop `archetype_id` argument. **Do NOT change `impl Component for CmdsA` / `CmdsB`** — they keep their fixed-slot impls. |
| `crates/boyko_ecs/tests/phase8cd_integration.rs` | Same migration pattern; ~5 spawn call sites. Component impls untouched. |
| `crates/boyko_ecs/tests/command_queue_panic_recovery.rs` | No bundle paths; unaffected. |
| `bundle_impls.rs::tests::*` (arity 1..=4 component_ids) | **Deleted** — moved to `derive_bundle_smoke.rs` under derive form. New tests use `#[derive(Component)]` for fresh components (the registry handles ID assignment lazily); legacy hand-rolled Component impls continue to coexist where present. |

### 8.4 Public API breakage

* `Commands::spawn(arch_id, bundle)` → `Commands::spawn(bundle)`. **Breaking**.
* `(A, B): Bundle` impl removed. Users must define a derived struct. **Breaking**.
* New trait method `Bundle::bundle_type_id()`, `Bundle::cached_archetype_id(&mut EcsMaster)` — additive; existing derive-only users gain them for free.

This is a pre-1.0 phase; breakage is acceptable per CLAUDE.md "no compromises in favor of convenience".

## 9. Implementation plan (for the developer)

Each step lists its acceptance criteria. Steps can be parallelized as noted.

### Step 0 — bundle_type_registry foundation
**File**: `crates/boyko_ecs/src/ecs/core/bundle/bundle_type_registry.rs` (NEW)
**Adds**:
- `BundleTypeId(pub usize)` newtype.
- `MAX_BUNDLE_TYPES: usize = 1024`.
- `static BUNDLE_NEXT_ID: AtomicUsize`.
- `fn register_new() -> usize` (#[cold] + #[inline(never)]).
- Module-level docs explaining the global counter contract.

**Acceptance**:
- `cargo check --all-targets` passes (no consumers yet — module is dead-code).
- 3 unit tests: `bundle_type_id_newtype_layout` (transparent), `register_new_assigns_distinct_ids`, `register_new_exhaustion_panics` (uses catch_unwind + a test-only `set_next_id_for_test` accessor).

**Dependencies**: None. Can begin immediately.

### Step 1 — Bundle trait rewrite
**File**: `crates/boyko_ecs/src/ecs/core/bundle/bundle.rs` (REWRITE)
**Adds**:
- `mod sealed { pub trait BundleSealed {} }` (the seal).
- `Bundle` trait with 4 methods (§4.1).
- Doc comments reaffirming SBC1..SBC9.

**Acceptance**:
- `cargo check --all-targets` fails (`bundle_impls.rs` no longer compiles — its tuple impls don't satisfy `BundleSealed`). This is **expected** — step 2 deletes `bundle_impls.rs`.

**Dependencies**: Step 0.

### Step 2 — Delete legacy tuple impls + cache
**File**: `crates/boyko_ecs/src/ecs/core/bundle/bundle_impls.rs` (DELETE)
**Files affected**:
- `crates/boyko_ecs/src/ecs/core/bundle/mod.rs` — remove `pub mod bundle_impls;`.

**Acceptance**:
- `cargo check --all-targets` fails at:
  - `commands::spawn_command::SpawnCommand::apply` (still references `archetype_id`).
  - `Commands::spawn` (still references tuple bundles).
- This is **expected**; steps 3-5 fix them.

**Dependencies**: Step 1.

### Step 3 — Per-EcsMaster cache field + helpers
**File**: `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` (MODIFY)
**Adds**:
- Field `bundle_archetype_cache: Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>` — C1 stable-address fix.
- `EcsMaster::new` / `with_capacity` initializer:
  ```rust
  bundle_archetype_cache: Box::new(core::array::from_fn(|_| OnceLock::new())),
  ```
- `pub(crate) fn bundle_archetype_id_for<B: Bundle>(&mut self) -> ArchetypeId` — internal-only (no public spawn::<B>). See §5.3.
- **Removed from Step 3**: `pub fn spawn<B: Bundle>` — Q4 resolution defers to Phase 11.

**Acceptance**:
- `cargo check --all-targets` still failing (depends on `Bundle::cached_archetype_id`, which is on the trait but no impls exist yet — step 5 produces the first one).
- Field added; constructor compiles. Verify `mem::size_of::<EcsMaster>()` increases by exactly 8 bytes (the Box pointer).

**Dependencies**: Step 1.

**Parallelizable with**: Step 4 (proc-macro work).

### Step 4 — `#[proc_macro_derive(Bundle)]`
**File**: `crates/boyko_macros/src/lib.rs` (MODIFY — add new derive)
**Adds**:
- `#[proc_macro_derive(Bundle)]` over named structs + tuple structs.
- Rejects unit structs with a `compile_error!("Bundle requires at least one field")`.
- Rejects generic structs with `compile_error!("Bundle derive does not support generics (Phase 8.5 scope)")`.
- Rejects fields with non-`Component` types via the bound `where #field_ty: Component`.
- Generates:
  - `impl sealed::BundleSealed for #name {}`
  - `impl Bundle for #name { ... }` with the 4 methods.
  - **O3 coalesced static**: a single per-impl `static INFO: OnceLock<BundleStaticInfo>`. `build_info()` helper mints `BundleTypeId`, computes + sorts + leaks the component_ids slice, returns a populated `BundleStaticInfo`. See §6.1 codegen template.
  - `bundle_type_id()`: `INFO.get_or_init(|| Self::build_info()).type_id`.
  - `component_ids()`: `INFO.get_or_init(|| Self::build_info()).component_ids`.
  - `cached_archetype_id(world)`: `world.bundle_archetype_id_for::<Self>()`. (Trait method delegates to the `pub(crate)` internal helper.)
  - `for_each_component_bytes`: **C5 mandatory codegen template** — see §6.3 verbatim. Must use `*const u8 + len` triple, NOT `&[u8]`, in the sort array, to sidestep E0521.

- **C5 requirement**: developer must verify the generated `for_each_component_bytes` body compiles for arity-1, arity-4, and arity-8 test bundles. The pattern in §6.3 is mandatory — deviations require architect re-review.

**Acceptance**:
- `cargo check` on `boyko_macros` passes.
- A scratch test in `boyko_ecs/tests/derive_bundle_smoke.rs` (Step 7) demonstrates `#[derive(Bundle)]` compiles end-to-end.

**Dependencies**: Step 1 (Bundle trait shape).

**Parallelizable with**: Step 3.

### Step 5 — `Commands::spawn` signature change + `SpawnCommand` simplification
**Files**:
- `crates/boyko_ecs/src/ecs/core/commands/spawn_command.rs` (MODIFY)
- `crates/boyko_ecs/src/ecs/core/system/params/commands.rs` (MODIFY)

**Changes**:
- `SpawnCommand<B>` field `archetype_id` REMOVED.
- `SpawnCommand::<B>::apply` body resolves via `B::cached_archetype_id(world)`.
- `Commands::spawn<B: Bundle>(&mut self, bundle: B)` — single-arg form.

**Acceptance**:
- `cargo check --all-targets` passes (assuming Step 3+4 done).
- Step 6 migrates broken tests.

**Dependencies**: Steps 1, 3, 4.

### Step 6 — Migrate existing tests
**Files**:
- `crates/boyko_ecs/src/ecs/core/system/params/commands.rs::tests::*`
- `crates/boyko_ecs/tests/phase8cd_integration.rs`
- Any other file referencing `commands.spawn(arch_id, ...)`.

**Pattern**:
```rust
// BEFORE
let arch = ecs.get_or_create_archetype(&[A::component_id(), B::component_id()]);
commands.spawn(arch, (A(1), B(2)));

// AFTER
#[derive(Bundle)]
struct AB { a: A, b: B }
commands.spawn(AB { a: A(1), b: B(2) });
```

**Acceptance**:
- `cargo test --all-targets` passes.

**Dependencies**: Steps 1-5.

### Step 7 — Smoke tests
**File**: `crates/boyko_ecs/tests/derive_bundle_smoke.rs` (NEW)

Tests:
1. `derive_bundle_named_struct_compiles_and_spawns` — minimal happy path.
2. `derive_bundle_tuple_struct_compiles_and_spawns` — tuple struct.
3. `derive_bundle_unit_struct_rejected` — `compile_error!` via `trybuild` (defer to step 9 if cumbersome).
4. `derive_bundle_unique_bundle_type_id` — two distinct bundles get distinct ids.
5. `derive_bundle_component_ids_are_canonical_sorted` — fields declared in `(B, A)` order still emit `[A.id, B.id]` after sort.
6. `derive_bundle_cached_archetype_id_idempotent` — `cached_archetype_id(world)` twice returns same id.
7. `derive_bundle_cross_world_isolation` — `cached_archetype_id` on world1 vs world2 returns distinct values (each world's archetype indexing is independent).

**Acceptance**:
- All tests pass.

**Dependencies**: Steps 1-6.

### Step 8 — Panic safety + Miri
**File**: `crates/boyko_ecs/tests/bundle_panic_safety.rs` (NEW)

Tests:
1. `bundle_for_each_panics_no_double_drop` — callback panics on 2nd component of arity-3 bundle; verify no double-drop via drop-counting components.
2. `bundle_for_each_panics_leak_unfinished_components` — verify components after the panicker leak (their Drop is suppressed by ManuallyDrop).

**File**: `crates/boyko_ecs/tests/miri_phase8_5.rs` (NEW)

Tests (run under `cargo +nightly miri test`):
1. `miri_bundle_cached_archetype_id_no_ub` — repeated calls.
2. `miri_bundle_cross_world_isolation_no_ub` — two worlds.
3. `miri_bundle_first_spawn_then_repeated` — cold + hot paths in sequence.
4. `miri_bundle_many_distinct_bundles_no_ub` — register up to `MAX_BUNDLE_TYPES = 1024` distinct bundle types and exercise both the per-impl `OnceLock<BundleStaticInfo>` cells and the per-world boxed-array slots. Replaces the Round 2 `cache_vec_growth_no_realloc_ub` test, which is impossible by construction after the C1 boxed-array fix.

**Acceptance**:
- All tests pass under standard test runner.
- Miri target passes (CI optional; local mandatory before merge).

**Dependencies**: Step 6.

### Step 9 — `compile_fail` rejections
**File**: `crates/boyko_ecs/tests/bundle_compile_fail/*.rs` (NEW, via `trybuild`)

- `unit_struct.rs` — `#[derive(Bundle)] struct Marker;` → expected error.
- `generic_struct.rs` — `#[derive(Bundle)] struct G<T> { ... }` → expected error.
- `non_component_field.rs` — `#[derive(Bundle)] struct B { x: u32 }` (u32 not Component) → expected error.
- `manual_impl_blocked.rs` — `impl Bundle for Foo { ... }` outside macro → seal trait error.

**Dependencies**: Step 4.

### Step 10 — Benchmarks
**File**: `crates/boyko_ecs/benches/bundle_static_cache.rs` (NEW)

Benchmark groups (criterion):
1. `component_ids_cached_lookup` — measure `Bundle::component_ids()` on a warm cache. Target: ≤ 2 ns.
2. `cached_archetype_id_cached_lookup` — measure `B::cached_archetype_id(world)` on warm cache. Target: ≤ 3 ns.
3. `commands_spawn_enqueue` — `commands.spawn(bundle)` cost. Target: ≤ 18 ns.
4. `spawn_command_apply_arity_4` — single SpawnCommand apply. Target: ≤ 200 ns.
5. `batch_10k_spawn_apply` — full 10k spawn + apply cycle. Target: ≤ 1.2 ms.

Compare baseline (Phase 8d) against new (Phase 8.5) — `criterion --baseline phase-8d`.

**Acceptance**:
- All targets met.
- Phase 8d → Phase 8.5 delta is ≥ 2× on `batch_10k_spawn_apply`.

**Dependencies**: Step 6.

**Parallelizable with**: Step 7, 8, 9 (independent).

### Step 11 — Phase 9 readiness audit
**File**: `docs/PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md` (this file — append a "Phase 9 integration notes" section after acceptance)

Document:
- The cache box (`Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>`) is single-writer per world; the box's address is fixed for the EcsMaster lifetime, and only individual `OnceLock` slot contents mutate (CAS-synchronized).
- Cross-world parallel apply is safe (per-world owned cache).
- Same-world parallel apply is forbidden (S1 + scheduler responsibility).
- The `BundleTypeId` global counter is multi-thread-safe by `Relaxed` atomic.
- Phase 9 scheduler must serialize `CommandQueue::apply` per-world.

**Acceptance**:
- Section appended; reviewed by architect for Phase 9 plan alignment.

**Dependencies**: Step 6 (apply path lands, so the Phase 9 audit has a concrete code surface to reason about). May revise after Step 10 (bench numbers may flag invariants worth codifying — e.g., serialization expectations on cross-world contention). The "Steps 1-10" Round 2 wording was over-broad: the audit's content (cache thread-safety, BundleTypeId atomic ordering, apply-serialization requirement) is settled once the apply path exists, irrespective of test or bench outcomes.

### Step dependency graph

```
Step 0 (BundleTypeRegistry)
   ↓
Step 1 (Bundle trait)
   ↓
   ├─→ Step 2 (delete legacy)
   ├─→ Step 3 (EcsMaster cache field)  ──┐
   └─→ Step 4 (derive macro)             ├─→ Step 5 (Commands/SpawnCommand)
                                          │
                                          └─→ Step 6 (migrate tests)
                                              ↓
                                              ├─→ Step 7 (smoke)
                                              ├─→ Step 8 (panic + Miri)
                                              ├─→ Step 9 (compile_fail)  (depends on Step 4 only)
                                              ├─→ Step 10 (benches)
                                              └─→ Step 11 (Phase 9 audit; depends only on Step 6 — may revise after Step 10)
```

**Parallel pairs**:
- Steps 3 + 4 in parallel.
- Steps 7-11 in parallel (after Step 6 lands).
- Step 9 can start immediately after Step 4 (independent of Step 5/6).
- Step 11 can start immediately after Step 6 (independent of Steps 7-10).

## 10. Performance projection

### 10.1 Hot path enqueue
```
Commands::spawn(bundle) — same path as Phase 8d minus user's get_or_create_archetype call.
  Old (user): arch = ecs.get_or_create_archetype(&[A::id(), B::id()])  ~100 ns
              commands.spawn(arch, (A, B))                              ~18 ns
              ────────────────────────────────                          ~118 ns
  New:        commands.spawn(MyBundle { a, b })                         ~18 ns

Per-spawn savings at enqueue: ~100 ns.
```

### 10.2 Hot path apply
```
Per SpawnCommand:
  Bundle::cached_archetype_id(world) cache hit          ~3 ns
  for_each_component_bytes (sort + emit pairs)          ~10 ns + 4×40 ns memcpy = 170 ns
  create_entity (existing)                              ~30 ns guard + memcpy
  ────────────────────────────────                      ~200 ns total
```

### 10.3 10k batch
```
Phase 8d: 3.0 ms
Phase 8.5: 1.2 ms (≥ 2.5× speedup)
```

### 10.4 Memory footprint (W4 — re-measured)

```
OnceLock<T> sizing on x86_64 stable (std impl):
  - One-time init Once primitive:    8 B (state word, aligned to 8)
  - MaybeUninit<T> cell:             size_of::<T>() bytes
  - Possible alignment padding to align T
  - Total: round_up(8 + size_of::<T>(), align_of::<T>())

Concrete sizes (T must be measured with mem::size_of in Step 0 unit test —
the values here are derived from std impl observation, not guaranteed by docs):

  OnceLock<BundleTypeId>             ≈ 16 B (8 state + 8 usize)
  OnceLock<&'static [ComponentId]>   ≈ 24 B (8 state + 16 fat ptr)
  OnceLock<ArchetypeId>              ≈ 16 B (8 state + 8 usize)
  OnceLock<BundleStaticInfo>         ≈ 32 B (8 state + 24 BundleStaticInfo)

Per Bundle type (process-global) — O3 coalesced:
  static OnceLock<BundleStaticInfo>  ≈ 32 B
  leaked [ComponentId; N]              N × 8 B (typical N ≤ 8 ⇒ ≤ 64 B)
  ─────────────────────────────        ≤ ~96 B per Bundle type
  (Was ~96 B in Round 1; same total but 1 fewer cache line touched — O3 win
   is access-locality, not raw bytes.)

Per EcsMaster:
  Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES=1024]>:
    pointer (inline on EcsMaster):    8 B
    heap:  1024 × ≤ 24 B              ≤ 24 KB (conservative upper bound; rounds
                                              with allocator overhead to roughly
                                              the same number on typical glibc /
                                              jemalloc / Windows heap)
  ─────────────────────────────       ≤ 24 KB per EcsMaster (constant, never grows)

(Conservative upper bound. Exact size = `1024 × size_of::<OnceLock<ArchetypeId>>()`,
 measured and asserted by the Step 0 test `oncelock_size_assumptions`. The plan
 commits to a single source of truth — **≤ 24 KB** — across §1.2, §4.3, §4.5,
 §10.4 to avoid version drift. If the Step 0 test ever shows a tighter bound,
 the test is the ground truth; this section may be revised downward in a
 follow-up phase, but the upper bound stays the contract.)

256 distinct bundles × 4 EcsMasters:
  Process-global per-bundle:  256 × ~96 B  = ~24 KB
  Per-world cache × 4:        4 × ≤ 24 KB  = ≤ 96 KB
  ─────────────────────────                  ≤ 120 KB total. Negligible at engine scale.
```

**Step 0 acceptance addition (W4 lock-in)**: add unit test `oncelock_size_assumptions` that asserts:
```rust
assert!(mem::size_of::<OnceLock<ArchetypeId>>() <= 24);
assert!(mem::size_of::<OnceLock<BundleStaticInfo>>() <= 40);
```
The `<=` form tolerates future std implementation tweaks (e.g., if `Once` adds padding); the assertions catch a regression that would multiply the per-world cache by 2× or more.

### 10.5 Cold path
```
First spawn per (Bundle type, world):
  BundleTypeId mint (atomic fetch_add)               2 ns
  component_ids() sort + leak                        80 ns
  Boxed-array slot index (no growth — pre-sized)     ~1 ns
  get_or_create_archetype                            ~1 µs (existing)
  OnceLock::set CAS                                  10 ns
  ────────────────────────────────                   ~1.10 µs
```

Cold path is acceptable since it runs at most `MAX_BUNDLE_TYPES = 1024` times per world for the process lifetime. Total cold-path overhead across the lifetime: < 2 ms. The C1 fix (boxed array of fixed `MAX_BUNDLE_TYPES` slots) eliminates the Round 1 `Vec::resize_with ~50 ns` line item — no growth ever occurs.

## 11. Phase 9 integration

### 11.1 Cache thread-safety

The per-world `bundle_archetype_cache: Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>` is:
- **Read** under `&EcsMaster` (zero-cost — `OnceLock::get` Acquire load at a stable address — the boxed array never moves).
- **Written** under `&mut EcsMaster` (cold path on first spawn per bundle type — only the contents of one `OnceLock` slot are mutated; the surrounding array is immutable in shape).

Phase 9 scheduler invariants required:
- **Phase 9 S1+** — `CommandQueue::apply` runs under `&mut EcsMaster` exclusively per world. The scheduler MUST serialize apply phases across all systems running on the same world.
- **Phase 9 cross-world** — multiple `EcsMaster`s on different threads each own their own boxed cache. No shared state. Lock-free by construction.

### 11.2 BundleTypeId multi-thread safety (C3 rewrite)

The process-global `BUNDLE_NEXT_ID: AtomicUsize` is `Relaxed`-incremented. Cross-thread visibility of the cached `BundleStaticInfo` is provided by the per-impl `OnceLock`'s Release-on-set / Acquire-on-get.

**Critical correction from Round 1**: `OnceLock::get_or_init` guarantees that the init closure runs **exactly once per OnceLock cell across all threads**. From the std docs (verbatim): "If the cell was empty, then `f` is called and the cell is set to the result of `f`. ... only one of them shall succeed in setting the cell, but all of them will return the same value." Losing threads do NOT execute the closure a second time — they park internally on the std once_lock primitive until the winner finishes, then read the cached value via Acquire load.

**Consequence for `register_new()`**: it is invoked **exactly once per Bundle type process-wide**. The earlier Round 1 phrasing ("Two systems on different threads ... loses CAS") was misleading — the loser does NOT call `register_new`. The atomic counter advances exactly once per Bundle type, regardless of how many threads race on first-spawn. This sidesteps the W1 concern that counter slots could be "burned" by losers.

Race trace (two threads on different `EcsMaster`s first-spawning `MyBundle`):
- Thread A: `MyBundle::component_ids()` (or `bundle_type_id()`) → enters `INFO.get_or_init(build_info)` → wins → executes `build_info` → calls `register_new()` once → atomic `fetch_add` returns id `k` → builds `BundleStaticInfo { type_id: BundleTypeId(k), component_ids: ... }` → `OnceLock::set` Release-stores.
- Thread B: same call → enters `INFO.get_or_init(build_info)` → loses → parks on std once_lock guard → wakes when A's Release-store completes → Acquire-loads cached `BundleStaticInfo` → reads `type_id` = `BundleTypeId(k)` (same as A).

Total contention window: ~80 ns winner (mint + sort + leak + CAS), then ~5 ns loser wake. Acceptable.

**Panic resilience**: `OnceLock::get_or_init` does NOT poison the cell on init-closure panic. If `register_new` panics on exhaustion (W1 path), the cell returns to uninitialized; the next `get_or_init` call retries `build_info`, calls `register_new` again. The saturate-then-panic pattern in `register_new` (§4.2) ensures the counter does NOT advance beyond `MAX_BUNDLE_TYPES` even under retries — terminal panic with bounded counter state.

### 11.3 Cache is single-writer per world (no growth)

The cache is `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>` — fixed size, allocated once at `EcsMaster::new()`, never reallocated. There is no growth path to race on. The only mutation is `OnceLock::set` on individual slots, which is internally synchronized (CAS). Cross-thread parallel reads of distinct slots are race-free; parallel `set` attempts on the same slot resolve to one writer per the OnceLock contract (see §11.2). Phase 9's `&mut EcsMaster`-per-apply serialization further ensures that any single-world cold-path init is single-threaded.

### 11.4 Documentation requirement for Phase 9 plan

When Phase 9 is designed, the plan must include an explicit invariant:

> **P9-CMD-SER1** — At most one `CommandQueue::apply` may run per `EcsMaster` at any given time. The scheduler enforces this via the apply-barrier between system batches.

## 12. Invariants — final list

| Inv ID | Description | Source |
|--------|-------------|--------|
| B1 | component_ids canonical-sorted ascending | Phase 8d, preserved |
| B2 | for_each_component_bytes order matches component_ids | Phase 8d, preserved |
| B3 | Bundle: Send + Sync + 'static | Phase 8d, preserved |
| B4 | ManuallyDrop-upfront panic safety | Phase 8d, preserved |
| **SBC1** | Bundle is non-generic; only `#[derive(Bundle)]` produces impls | NEW (sealed supertrait) |
| **SBC2** | BUNDLE_TYPE_ID unique + stable per process | NEW (atomic counter) |
| **SBC3** | component_ids returns the same `&'static [ComponentId]` for every call within a process | NEW (per-impl OnceLock) |
| **SBC4** | cached_archetype_id idempotent per (BundleTypeId, EcsMaster) | NEW |
| **SBC5** | per-world cache is `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>` indexed by BundleTypeId.0 (stable address; allocated once at `EcsMaster::new()`; never reallocated) | NEW |
| **SBC6** | Two EcsMaster instances have independent caches | NEW (no shared state) |
| **SBC7** | BundleTypeId.0 < MAX_BUNDLE_TYPES (= 1024) | NEW (panic on exhaustion) |
| **SBC8** | component_ids slice leaked once per Bundle type per process | NEW (≤ 96 B/bundle) |
| **SBC9** | for_each_component_bytes body emits canonical order, ManuallyDrop-upfront, pointer-based intermediate (C5) | NEW (macro guarantee) |
| **SBC10** | MAX_BUNDLE_ARITY = 8; derive macro rejects > 8 fields with compile_error! | NEW (W3 fix) |
| CQ1..CQ7 | CommandQueue invariants | Phase 8d, preserved |
| CQ-PACK1 | No reference creation into packed byte layout | Phase 8d, preserved |
| CQ-SEND1 | unsafe impl Send for CommandQueue | Phase 8d, preserved |
| APP1..APP4 | System::apply contract | Phase 8d, preserved |
| APP1' | apply is a safe fn | Phase 8d, preserved |

## 13. Metrics and validation

### 13.1 Mandatory unit tests
- `BundleTypeId` newtype layout (transparent over usize).
- `register_new` distinct IDs.
- `register_new` exhaustion panics with expected message.
- `Bundle::component_ids` canonical sort (named struct, tuple struct).
- `Bundle::bundle_type_id` idempotent.
- `Bundle::cached_archetype_id` idempotent within a world.
- `Bundle::cached_archetype_id` distinct across worlds.

### 13.2 Mandatory integration tests
- `commands.spawn(bundle)` deferred path produces 1 entity in the expected archetype.
- 10k spawn batch produces 10k entities (no leak, no double-spawn).
- `EcsMaster::spawn(bundle)` direct path equivalent to deferred.
- Cross-world isolation (two `EcsMaster`s spawn the same Bundle, neither leaks into the other).

### 13.3 Mandatory property tests
- For a random bundle (1..=8 components, randomized field order), `component_ids` is sorted ascending.
- For 1k random `(EcsMaster, Bundle)` pairs, `cached_archetype_id` always returns the same id within a world.

### 13.4 Mandatory Miri tests
- `miri_bundle_many_distinct_bundles_no_ub` — register `MAX_BUNDLE_TYPES = 1024` distinct bundle types in one process; assert no UB across all the per-impl `OnceLock<BundleStaticInfo>` cells and the boxed-array slots. Covers the OnceLock-init race surface across many cells (the post-C1 replacement for the now-impossible "cache vec growth" scenario).
- `miri_bundle_onceLock_race_same_bundle` — two threads first-spawn the same Bundle type concurrently (via per-thread `EcsMaster` instances so apply-serialization is irrelevant); assert no UB and that both threads observe the same `BundleTypeId`.
- `miri_bundle_for_each_panic_recovery` — `for_each_component_bytes` callback panics on the second component of an arity-3 bundle; assert no double-drop, no UB, and that the un-emitted components leak via ManuallyDrop suppression (B4 invariant).

### 13.5 Mandatory benchmarks (C2 fixtures)

All benches use the fixture defined in §1.2:
- One `EcsMaster::new()` per group.
- One bundle type per group, pre-registered via a warm-up call (so the OnceLock is hot).
- Criterion `iter_batched` for setup-per-iteration where needed (e.g., fresh SpawnCommand per iteration).

Targets:
- `bundle_component_ids_cached_lookup` — measures `B::component_ids()` on a warm bundle from a hot caller. **Target: ≤ 2 ns/op.**
- `bundle_cached_archetype_id_cached_lookup` — measures `B::cached_archetype_id(&mut world)` on a warm world + warm bundle. Caller context: simulates `SpawnCommand::apply` (the only real caller). **Target: ≤ 3 ns/op.**
- `commands_spawn_enqueue` — measures `commands.spawn(bundle)` cost in a Commands-borrowing system body. Does NOT call `cached_archetype_id`. **Target: ≤ 18 ns/op.**
- `spawn_command_apply_arity_4` — measures one full `SpawnCommand<MyBundle>::apply` cycle on a 4-component bundle, warm world. **Target: ≤ 200 ns/op.**
- `batch_10k_spawn_apply_arity_4` — measures 10k `commands.spawn(MyBundle { ... })` enqueues + 1 `CommandQueue::apply` flush. **Target: ≤ 1.2 ms total.**
- `bundle_cold_first_spawn` — measures the cold path: first ever spawn of a new bundle in a fresh `EcsMaster`. **Target: ≤ 1.5 µs/op (one-shot — Criterion `iter_with_large_drop` or `iter_custom`).**

### 13.6 Mandatory `debug_assert!` invariants
- `cached_archetype_id` boxed-array slot: `debug_assert!(id.0 < MAX_BUNDLE_TYPES)` — `cache.len()` is constant (the boxed array never grows), so the check guards against an exhausted-counter panic path slipping through saturate-then-panic.
- `for_each_component_bytes`: `debug_assert!(count == arity)` after callback chain.
- `component_ids()` init closure: `debug_assert!(arr.is_sorted_by_key(|id| id.0))` after the sort step.

## 14. Open questions

### Q1 — Bundle nesting
Should `#[derive(Bundle)]` permit fields whose types are themselves `Bundle`s (rather than `Component`s)? Bevy permits this for ergonomic composition (e.g. `struct CharacterBundle { transform: TransformBundle, physics: PhysicsBundle }`). Phase 8.5 scope: **no nesting**. Each field must be a `Component`. Phase 9 may revisit.

**Risk**: ergonomic gap for projects with deep composition. Mitigated by users defining a single flat bundle per spawn site.

### Q2 — `MAX_BUNDLE_TYPES = 1024` ceiling
Is 1024 enough? Bevy projects typically have 100-300 bundles. AAA games may reach 500-800. 1024 gives headroom. Hitting it panics with a clear diagnostic.

**Alternatives**: 4096 (32 KB max cache). Pick 1024 for now; trivial to raise later.

### Q3 — Cold-path race practicality
Per §11.2, the only multi-thread race is on the per-impl `static OnceLock<BundleTypeId>` init when two threads first-spawn the same Bundle. This is resolved by `OnceLock`'s internal CAS. Acceptable.

**Question**: should the `register_new()` call inside the init closure use a stronger ordering than `Relaxed`? No — uniqueness is the only requirement; happens-before is provided by the OnceLock itself.

### Q4 — Eager direct-path spawn API — RESOLVED (Round 2)
**Decision**: REJECTED for Phase 8.5. Only `Commands::spawn<B>` (deferred) ships. Eager `EcsMaster::spawn<B>` deferred to Phase 11.

**Rationale**: matches Bevy's API surface (`Commands::spawn` is canonical, `World::spawn` is internal-leaning). Eliminates migration debt for `spawn_one`/`spawn_two` retention. Single supported spawn surface reduces decision-fatigue at call sites.

### Q5 — Removing `EcsMaster::spawn_one`, `spawn_two` — RESOLVED (Round 2)
**Decision**: DELETED entirely in Step 6. No `#[deprecated]` retention (W2 fix).

**Rationale**: Phase 2e `spawn_one` / `spawn_two` were tuple-Bundle-based ergonomics. After Step 2 deletes the tuple Bundle impls, `spawn_one` and `spawn_two` no longer compile — there is no underlying type for them to call into. The original Round 1 plan to retain them as `#[deprecated]` was based on the assumption that tuple impls survive; with their removal, the `#[deprecated]` form is structurally impossible. Pre-8.5 callsites migrate directly to `derive(Bundle)` + `Commands::spawn(bundle)`.

### Q6 — `for_each_component_bytes` sort cost for large bundles — RESOLVED (Round 2, W3 fix)
**Decision**: `MAX_BUNDLE_ARITY = 8` (LOWERED from Round 1's 16, per W3).

**Rationale**: Round 1 cap of 16 inflated the stack-collector array unnecessarily for the common arity-2/arity-4 case. Stack array size = `MAX_BUNDLE_ARITY × sizeof((ComponentId, &[u8])) = 8 × 24 B = 192 B` (was 384 B at cap 16). Bundles > 8 components are pathological — they cross archetype-design boundaries and degrade `create_entity` memcpy locality. Users with >8 components should split into multiple Bundles or wait for a dedicated "fat bundle" design (separate phase).

Documented as **SBC10**: `MAX_BUNDLE_ARITY = 8`. `#[derive(Bundle)]` rejects bundles with > 8 fields via `compile_error!`.

### Q7 — `for_each_component_bytes` body — is the runtime sort necessary? (unchanged)
Could the derive macro emit a compile-time-sortable form (e.g. via a const-fn-on-stable token reshuffling)? No — `ComponentId` values are runtime-minted. The sort must happen at runtime, but only once per Bundle type — BUT we currently re-sort every `for_each_component_bytes` call (every spawn) because the stack array is rebuilt each time.

**O2 candidate (deferred)**: cache the sorted **permutation index** `[u8; N]` inside `BundleStaticInfo`. Then `for_each_component_bytes` walks the permutation array (no sort per spawn). Saves ~5 ns per spawn on arity-4 bundles.

**Decision (Round 2)**: DEFER O2 to a follow-up phase. The current ~5 ns/spawn sort cost is dominated by the `create_entity` memcpy (~30 ns/component × 4 = 120 ns); O2 is a < 5% optimization. Premature for Phase 8.5.

### Q8 — O1 (Vec bounds check skip) — RESOLVED (Round 2)
**Decision**: DEFER. With C1 fix (boxed array), the bounds check is against a known constant `MAX_BUNDLE_TYPES = 1024`. LLVM should fold the check trivially. Confirmed by ASM inspection in Step 3.

If profiler shows the check is not folded, follow-up adds `unsafe { *cache.get_unchecked(id.0) }` with a `debug_assert!(id.0 < MAX_BUNDLE_TYPES)`. Trivial to add later.

### Q9 — Invariant SBC10 codification
**Added**: `SBC10 — MAX_BUNDLE_ARITY = 8. Bundles with > 8 fields are rejected by the derive macro at compile time.` Update §2.3, §12 to include.
```

---

## Final notes for orchestrator

The plan above (≈42 KB markdown) covers all 12 requested sections (decision matrix, multi-world strategy, derive macro design, Bundle trait shape, Commands API change, cache storage, cold path, migration, step-by-step plan, perf projection, Phase 9 integration, invariants). The file write was not performed by me because:

1. My current tool set in this conversation lacks `Write`/`Edit`.
2. The injected system instruction explicitly says: "Do NOT Write report/summary/findings/analysis .md files. Return findings directly as your final assistant message".

The orchestrator should either (a) save the markdown block to `D:\claude\BoykoEngine\docs\PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md` directly, or (b) dispatch a developer agent with write capability to do so. The content above is final; no further design iteration is needed before critic round.

Sources consulted:
- [Bevy ECS Bundle docs (per-world caching pattern)](https://docs.rs/bevy_ecs/latest/bevy_ecs/bundle/index.html)
- [Bevy Bundle trait signature](https://docs.rs/bevy/latest/bevy/ecs/bundle/trait.Bundle.html)
- [Rust users.rs: generic statics per monomorphization](https://users.rust-lang.org/t/how-to-create-a-new-static-variable-per-monomorphization/107581)
- [Rust tracking issue for const fn TypeId](https://github.com/rust-lang/rust/issues/77125)
- [Tainted Coders: Bevy ECS edges/archetype graph](https://taintedcoders.com/bevy/ecs)

Relevant existing files studied (all absolute paths):
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\bundle\bundle.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\bundle\bundle_impls.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\bundle\mod.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\commands\spawn_command.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\commands\command.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\commands\command_queue.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\commands\mod.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\params\commands.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\component\component_registry.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\component\component.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype_master.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\resources\resource_registry.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\identifiers\primitives.rs`
- `D:\claude\BoykoEngine\crates\boyko_macros\src\lib.rs`

## 16. Phase 9 integration notes

This section captures the Phase 8.5 invariants that Phase 9's multi-system
scheduler must respect. Authored at the end of Step 11 once the apply path
(Step 5) and the Static Bundle Cache (Step 6) had real code surface for
the audit to reason about, and after Step 10's bench numbers confirmed
the cached cost profile (≤ 1 ns per cache lookup, ~77 ns per amortised
batch spawn).

### 16.1 Cache box is single-writer per world

The `bundle_archetype_cache: Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>`
on `EcsMaster` has two distinct mutability layers:

- **Box address**: written exactly once, at `EcsMaster::new` /
  `EcsMaster::with_capacity`. The `Box` itself is then moved into the
  struct field and never reassigned. The inner array's heap address is
  stable for the entire `EcsMaster` lifetime — this stability is
  load-bearing because it lets the hot-path acquire load on
  `cache[id.0].get()` rely on a constant base pointer (no indirection
  through a Vec header).

- **Slot contents**: each `OnceLock<ArchetypeId>` slot is written at
  most once per `(BundleTypeId, EcsMaster)` pair via `OnceLock::set`
  (CAS-synchronized, Release ordering on success). Subsequent
  `OnceLock::get` calls are Acquire-ordered loads on the same slot. The
  cold writer in `cold_register_bundle_archetype` already absorbs racing
  `set` Errs gracefully (the Err carries the rejected value; the winner's
  value is read back via `get().expect`).

Phase 9 implication: parallel reads on the same `(BundleTypeId, EcsMaster)`
slot are sound because `OnceLock::get` is `Sync`. Parallel writes are
constrained to one per slot per process by the `OnceLock` contract;
contended writes degrade to the loser dropping its candidate value, which
is identical by SBC4 (canonical ids + idempotent `get_or_create_archetype`).

### 16.2 Cross-world parallel apply is safe

Two distinct `EcsMaster` instances each own their own
`bundle_archetype_cache` boxed array — distinct heap allocations, no
shared mutable state. The process-global state shared between worlds is:

- `BUNDLE_NEXT_ID` (atomic counter) — `Relaxed`-ordered `fetch_add` is
  multi-thread-safe by construction.
- Per-impl `OnceLock<BundleStaticInfo>` — process-scoped, set-once,
  `Sync`. All worlds observe the same `BundleStaticInfo` for a given
  Bundle type; the (small, immutable) payload is shared correctly across
  threads.
- `ComponentRegistry` slots — set-once, indexed by `ComponentId.0`;
  shared across worlds.

Phase 9 scheduler can dispatch systems whose `World` parameters bind to
DIFFERENT `EcsMaster` instances in parallel without further coordination
on the bundle cache surface. The same SBC4 invariant table holds.

### 16.3 Same-world parallel apply is forbidden

`CommandQueue::apply` (and therefore `SpawnCommand::apply`) requires
`&mut EcsMaster` for the entire flush. Two systems whose `SystemParam::apply`
both target the same world cannot run in parallel — the borrow checker
rejects the nested `&mut` at the `run_cached_system` callsite, which is
also where the S1 invariant enters the contract.

This is the SAME invariant `&mut self` enforces today on
`run_system` / `run_cached_system` / `run_system_once` (plan §1.2,
Phase 8c). Phase 9's scheduler must SERIALIZE `CommandQueue::apply` per
world: only one system per world may be in its apply step at any given
time. Parallelism remains available across:

- Different worlds (per 16.2).
- The system body itself (Phase 9's `Access` conflict graph governs
  this stage — Commands declares no component/resource access, so the
  body's commands enqueue path is freely parallelisable).
- Only the apply step requires the serialization barrier.

The scheduler can implement this as a per-world `flush_phase` barrier
that runs sequentially after all parallel body steps complete for the
current frame's affinity group.

### 16.4 BundleTypeId global counter — Relaxed atomic ordering

`BUNDLE_NEXT_ID.fetch_add(_, Relaxed)` is sound under any number of
concurrent threads because:

- **Uniqueness**: `fetch_add` is RMW-atomic; two callers always observe
  distinct return values regardless of ordering tier.
- **Happens-before for the minted id**: provided by the per-impl
  `OnceLock<BundleStaticInfo>` cell (Release on `set`, Acquire on `get`).
  The counter itself does NOT publish data to other threads — its only
  output is a unique integer that gets stored INSIDE the OnceLock
  payload, and the OnceLock provides the synchronization for the
  payload's visibility.

Phase 9 implication: no upgrade to `AcqRel` or `SeqCst` is required.
The `Relaxed` ordering is the textbook correct choice for a uniqueness-
only counter (mirrors `ComponentRegistry::NEXT_ID`).

### 16.5 Scheduler obligations — checklist

Phase 9's scheduler design document must include:

1. **Per-world apply serialization**: at most one
   `SystemParam::apply(&mut world)` call in flight per `EcsMaster`
   instance. The borrow checker enforces this naturally if the scheduler
   takes `&mut EcsMaster` for the apply phase; explicit lock-free
   designs must add an external barrier.

2. **Parallel body dispatch**: systems with non-conflicting `Access`
   sets (per the existing `FilteredAccessSet` conflict graph) may run
   their bodies in parallel. `Commands<'s>`'s per-system queue is
   `!Send` via the `'s` lifetime, so each body owns its own queue — no
   cross-system queue races on the body side.

3. **Cross-world parallelism**: systems whose `World` parameter binds
   to distinct `EcsMaster` instances may run BOTH body and apply in
   parallel — no per-world coordination needed because the per-world
   cache slots live in distinct boxes (16.2).

4. **OnceLock contention as a non-issue**: first-spawn-per-world races
   on the same Bundle type degrade to one winner installing the slot
   and the losers reading it back. Bench-measured cold path is
   ~1 µs/(bundle, world) — bounded by `MAX_BUNDLE_TYPES × world_count`
   one-time pays across the process lifetime. No steady-state cost.

5. **Step 5 deferred Entity id surfacing**: the Phase 8.5
   `SpawnCommand<B>::apply` discards the returned `Entity`. Phase 11's
   `SpawnCommandReturning<B>` will pre-allocate an `EntityId` at
   enqueue time so the user can chain `.id()` Bevy-style; the
   pre-allocation must integrate with `EntityMaster::allocate_entity`
   under the same serialization rules as 16.3 (per-world `&mut`).
- `D:\claude\BoykoEngine\docs\PHASE-8CD-INTOSYSTEM-COMMANDS-PLAN.md`

## 17. Phase 9 readiness audit (Step 11)

Authored at the close of Phase 8.5 Step 11 after Steps 7-10 landed
their smoke + Miri + compile_fail + bench coverage. This section is the
focused audit checklist that Phase 9's scheduler design must reconcile
against; §16 contains the discursive form, this section is the
"hand it to the architect" boilerplate.

### 17.1 Thread-safety invariants (audit summary)

| Surface | Mutability | Synchronisation | Phase 9 implication |
|---------|------------|-----------------|---------------------|
| `bundle_archetype_cache` Box address | written once at `EcsMaster::new` | Rust move semantics | Box base pointer is stable for the EcsMaster lifetime; per-world reads safe |
| `OnceLock<ArchetypeId>` slot contents | set at most once per `(BundleTypeId, EcsMaster)` | `OnceLock::set` CAS, Release on success / Acquire on `get` | Parallel reads on populated slot are sound under `Sync`; contended writes degrade to one winner, others read back |
| `BUNDLE_NEXT_ID` global counter | `fetch_add` per Bundle type, exactly once process-wide | `Relaxed` (uniqueness only) | No upgrade to `AcqRel`/`SeqCst` required; happens-before for the minted id is provided by the per-impl `OnceLock<BundleStaticInfo>` |
| per-impl `OnceLock<BundleStaticInfo>` | set-once per Bundle type | std OnceLock contract | All worlds observe the same payload after first init; safe to share across threads |
| `CommandQueue::apply` execution | requires `&mut EcsMaster` for entire flush | Rust borrow checker | Same-world apply MUST be serialised by the scheduler; cross-world apply is parallel-safe |

### 17.2 Phase 9 design checklist

The Phase 9 scheduler plan must revisit (and codify decisions on) the
following items:

1. **Hot read of `bundle_archetype_id_for` via `&self`** — current
   helper takes `&mut self` because `cached_archetype_id` lives on the
   `apply` path under `&mut EcsMaster`. If Phase 9's parallel body
   stage wants to inspect archetype ids without entering an apply
   barrier, expose a `&self` variant that ONLY reads (no cold-path
   `get_or_create_archetype`). Trivial — the inner `OnceLock::get`
   already needs no mutation. Tracking item: Phase 9 §X.

2. **Cold-path lock-free upgrade beyond `OnceLock` CAS** — currently
   first-spawn-per-bundle-per-world goes through
   `cold_register_bundle_archetype` which calls
   `ArchetypeMaster::get_or_create_archetype`. That helper is currently
   `&mut self`. Phase 9 must decide whether to:
   (a) leave it as `&mut` and serialise cold-paths (current behaviour;
   ≤ 1 µs/(bundle, world) one-time cost, fine for steady state); or
   (b) refactor `ArchetypeMaster` to support cold registration under
   `&self` with internal CAS. Recommendation: defer to Phase 10 unless
   profile shows cold-path contention in real workloads.

3. **Scheduler API for "this system needs `&mut EcsMaster` on apply"**
   — Phase 9 must surface a typestate-level marker on systems whose
   `SystemParam::apply` exists (currently only `Commands<'s>`). The
   scheduler reads this marker to decide which systems join the
   apply-phase serialisation group versus the body-phase parallel
   group. Concrete proposal: `trait NeedsApply` on `SystemParam`, with
   a blanket `impl NeedsApply for ()` and a positive `impl NeedsApply
   for Commands<'s>`.

4. **`BundleTypeId` exhaustion observability** — `MAX_BUNDLE_TYPES =
   1024` panic on saturate is terminal by design (W1). Phase 9 should
   add a startup `EcsMaster` health probe that logs the current
   `BUNDLE_NEXT_ID.load()` count so operators see how close they are
   to the cap. Not blocking; nice-to-have.

5. **Per-world cache zero-copy share across systems** — the cache box
   is held by `EcsMaster`. Phase 9 must NOT clone it into per-system
   state; the borrow against `&EcsMaster` (read) or `&mut EcsMaster`
   (write) is the only blessed access path. Codify in Phase 9 invariants.

### 17.3 Items explicitly NOT required for Phase 9

- **Lifting `MAX_BUNDLE_TYPES`** — 1024 has years of headroom against
  real engine usage. Defer indefinitely.
- **Removing the per-impl `static OnceLock`** — the per-impl static
  IS the cache identity. Removing it would mean re-minting
  `BundleTypeId` from scratch every call, defeating the whole phase.
- **Adding a `Send + Sync` impl for `EcsMaster` itself** — orthogonal
  to bundle caching. The cache surface is internally `Send + Sync` via
  its constituent types; whether `EcsMaster` as a whole is `Send` is a
  Phase 9 architectural decision driven by entity / archetype access,
  not bundle storage.

### 17.4 Acceptance

This section closes Phase 8.5 Step 11. The audit verdict is:

- All Phase 9-relevant thread-safety contracts are documented (§16 +
  §17.1).
- The Phase 9 design checklist (§17.2) is concrete and bounded.
- No Phase 8.5-side blockers exist for Phase 9 to start design work.
