# Glossary

A reference of terms used throughout Boyko Engine and ECS architecture in general.

## A

**Aether** — The engine's declarative DSL, the `aether!` macro: components, events, systems, plugins, state machines, materials and scenes, expanded at compile time into ordinary engine code. See [Aether overview](../aether/overview.md).

**Address stability** — The guarantee that, once a row is written into a `ComponentPool`, its address never moves for the life of the pool. Each pool reserves a large virtual region up front (`VmReservation`) and only commits pages on growth, so growth never copies bytes and pointers stay valid. See [`vm.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_memory/src/vm.rs) and [`component_pool.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/memory/component_pool.rs).

**Archetype** — A unique combination of component types. All entities with the same set of components belong to the same archetype, and their components are stored together for cache-friendly iteration.

**AoS** (Array of Structs) — Memory layout where each entity is a single struct containing all its data. Cache-unfriendly when only some fields are accessed. The opposite of SoA.

## B

**Best-fit** — A general allocator-theory strategy that finds the *smallest* free block large enough to satisfy a request, reducing fragmentation compared to first-fit. *Boyko Engine does not use a best-fit/first-fit allocator:* each `ComponentPool` grows a single dense virtual region monotonically (commit-on-growth), so there are no free blocks to fit.

**Bitmask / BitSet** — A compact representation of a set, where each bit indicates the presence of an element. Used in Boyko Engine to encode "which components an archetype contains".

**`boyko-` diagnostic code** — An engine diagnostic's stable id: `boyko-`, a class letter, and four digits. `B` marks a panic, `E` an error, `W` a warning — for example `boyko-B1801`, a plugin added twice. Every live code has a page in [`docs/diagnostics/`](https://github.com/bluesteelll/boyko-engine/tree/master/docs/diagnostics).

## C

**Cache line** — The smallest unit of memory the CPU transfers between RAM and cache, typically 64 bytes on x86_64. Performance-critical data structures align to cache-line boundaries to avoid wasted transfers and false sharing.

**Cache, D-cache (data cache)** — CPU's hierarchy of data caches: L1d (~32 KB, ~4 cycles), L2 (~256–512 KB, ~12 cycles), L3 (megabytes, ~40 cycles). Hot loops aim to keep their working set in L1d for maximum throughput.

**Cache, I-cache (instruction cache)** — Separate cache for executable instructions, typically L1i ~32 KB on x86_64. Bloated hot paths (e.g., from aggressive inlining) cause I-cache misses, stalling the front-end of the CPU pipeline.

**Chunk** — A *SIMD-iteration batch*: a fixed-width slice of contiguous rows handed to a closure by `Query::for_each_chunk` / `par_for_each_chunk` so the body can vectorize over many rows at once. It is **not** a per-type storage buffer — storage is one dense byte buffer per `ComponentPool`. See [`chunked_data.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs).

**Colored solve** — The default rigid-body solver (`ColoredSoftStepSolver`). Each step's contacts are grouped into islands and greedy-colored so that no color shares a dynamic body, then swept color by color; the SIMD and parallel paths work on a color's body-disjoint contacts. See [Physics](../simulation/physics.md#the-tgs-soft-solver).

**Compaction** — Process of removing gaps in storage to improve cache locality.

**Component** — A piece of data attached to an entity. In Boyko Engine, components are POD-like structs implementing the `Component` trait via `#[derive(Component)]`.

**ComponentId** — A unique numeric identifier (a `usize` newtype) for a component type. The derive macro emits a `component_id()` that assigns the id **lazily at runtime on first use**, via a per-type `OnceLock` that calls `register_new::<Self>()` — which `fetch_add`s a process-global atomic `NEXT_ID`. The id therefore depends on first-touch (registration) order. *Not stable across runs.*

**CoreSchedule** — `CoreSchedule::{Main, Fixed}`: the once-per-frame schedule and the fixed-timestep schedule an `App` runs. See [App & Plugins](../app/plugins.md#coreschedule).

**CSM** (cascaded shadow maps) — The sun's shadow rendered into several depth maps, each covering a slice of the view distance. Off by default; `CsmConfig::cascade_count` > 0 enables it. See [Lighting](../rendering/lighting.md#shadows-and-ao-the-sources).

## D

**DDGI** (dynamic diffuse global illumination) — Indirect diffuse light from a grid of probes updated at run time. Boyko Engine's probes trace the SDF field. Off by default; `DdgiConfig::ddgi_indirect` enables it. See [Lighting](../rendering/lighting.md#shadows-and-ao-the-sources).

**Dense component** — A component declared `#[component(storage = "dense")]`: one contiguous column holds every instance across all archetypes, so it never fragments the archetype space. Used where one buffer for all instances matters, such as GPU interpolation pairs. See [Components](../concepts/components.md).

**DOD** (Data-Oriented Design) — A design philosophy that prioritizes memory layout and data access patterns over object-oriented abstractions. See [Design Principles](../architecture/principles.md).

## E

**ECS** (Entity Component System) — An architectural pattern that separates data (components) from behavior (systems), with entities acting as identifiers that group components.

**eDSL** (shader embedded DSL) — `boyko_shaderdsl`: shader math written once in Rust, generic over a scalar backend, and instantiated over `f32` (the host oracle) and `Emit` (the HLSL printer). See [Shader eDSL](../rendering/shader-edsl.md).

**Empty archetype** — The archetype with zero components. Entities may legally hold no components: removing the last component migrates the entity here instead of despawning it, and `spawn_empty()` creates entities here directly. See [Tags](../concepts/tags.md).

**Enable tag** — A component declared `#[component(storage = "bitset")]`: a per-entity bit with no column. `enable` / `disable` are O(1) with no archetype migration; the filters `Enabled<T>` / `Disabled<T>` select on the bit, and `IsEnabled<T>` reads it without filtering. See [Enable Tags](../concepts/enable-tags.md).

**Entity** — A lightweight identifier (an `id: EntityId` + `generation: u32` in Boyko Engine, where `EntityId` is a `#[repr(transparent)]` newtype over `usize`) that represents a "thing" in the game world. Entities themselves hold no data — their data lives in components.

**Existence-based processing** — Encoding state as component *presence* rather than a data field, so systems filter at archetype granularity instead of branching per row. The rationale behind tags. See [Storage Trade-offs](../architecture/storage-tradeoffs.md).

## F

**False sharing** — A performance bug where two threads write to different variables that happen to share a cache line, causing the cache line to bounce between cores. Mitigated by padding shared structures to cache-line boundaries.

**FixedTime** — The fixed-step clock resource: the timestep (64 Hz by default) and the accumulated overstep that the `Fixed` schedule's catch-up loop spends. See [Time & Fixed Timestep](../app/time.md).

**Framegraph** (render dependency graph, RDG) — The on-screen frame's pass graph in `boyko_rhi_vulkan`: each pass declares the resources it reads and writes, and compiling the graph derives the barrier set. See [RHI](../rendering/rhi.md#synchronization-the-framegraph).

## G

**G-buffer** — The Deferred path's per-pixel attribute targets (albedo, normal, material, view distance) that the lighting resolve reads. See [Rendering overview](../rendering/overview.md#how-a-frame-flows).

**Generation** — A counter incremented each time an entity ID is reused. Combined with the ID, it disambiguates "old" references to deleted entities from references to new entities that reuse the slot.

**Geometry leg** — Which geometry producers a frame has: `GeometryLegs::{Both, Mesh, Sdf}`, `Both` by default. A dropped leg allocates and records nothing. See [SDF rendering](../rendering/sdf.md#the-sdf-leg-under-each-render-path).

## H

**Hooks and observers** — Hooks are per-component lifecycle callbacks (`on_add`, `on_insert`, `on_replace`, `on_remove`, `on_despawn`) that the engine runs as a component's lifecycle changes. Observers are callbacks registered on the world that react to triggered events. See [Lifecycle Hooks & Observers](../concepts/hooks-and-observers.md).

**Hot/cold split** — Separating frequently-accessed (hot) fields from rarely-accessed (cold) ones, putting hot fields together for cache efficiency.

**HZB** (hierarchical Z-buffer) — A mip pyramid of the depth buffer (a `min` reduce under reverse-Z) that two-phase occlusion culling tests instances against. Off by default; the culling runs on the Visibility Buffer path. See [Rendering overview](../rendering/overview.md#what-ships-today).

## I

**Island sleep** — Per-island deactivation in the rigid solver (`PhysicsConfig::sleeping`, off by default): an island whose fastest body stays below a speed² threshold for a debounce window freezes, and a frozen, clean island skips collision and solve until its inputs change. See [Physics](../simulation/physics.md#scaling--the-performance-paths).

## L

**Lock-free** — Concurrent code that makes guaranteed forward progress without using mutexes or other blocking primitives. Achieved through atomic operations.

## M

**Memory ordering** — In Rust atomics, the constraint on how loads and stores are visible across threads. Options: `Relaxed`, `Acquire`, `Release`, `AcqRel`, `SeqCst`.

**Monomorphization** — Rust's process of generating a specialized version of generic code for each concrete type used, eliminating runtime dispatch overhead.

**MSDF / MTSDF** (multi-channel signed distance field) — A glyph-atlas encoding whose three color channels keep corners sharp at any scale; MTSDF adds a true SDF in the alpha channel. `boyko_fontbake` bakes MTSDF atlases. See [Text & MSDF](../ui/text-msdf.md).

## P

**PGO** (Profile-Guided Optimization) — A compiler optimization technique where the binary is built twice: first an instrumented version that collects runtime profiling data, then a final version that uses that data to make decisions about inlining, branch layout, and function placement.

**Prefetching** — Loading data into cache *before* it's needed. Hardware prefetchers detect sequential and stride access patterns; software prefetching (`_mm_prefetch` intrinsic) is used when patterns are predictable but the hardware can't see them (e.g., pointer-chasing through indices).

**Pool** — A pre-allocated, dense column of rows for components of one type. Boyko Engine's implementation is the type-erased `ComponentPool` ([`component_pool.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/memory/component_pool.rs)) — a `NonNull<u8>` byte buffer plus a cached `component_layout: Layout`, built via `ComponentPool::new(component_id, reserve_rows)` on a `VmReservation`.

**POD** (Plain Old Data) — A type with simple memory layout (no internal pointers, no destructor) that can be safely copied with `memcpy`.

## Q

**Query** — A specification of "which entities to operate on" based on which components they have. Queries iterate over all matching archetypes.

## R

**Relation** — A typed, one-to-many link between entities: you write one foreign-key component on the source, and the engine maintains the reverse index on the target. `ChildOf` / `Children` is the canonical instance. See [Relations](../concepts/relations.md).

**Render path** — How geometry becomes lit pixels: `RenderPath::{Deferred, Forward, ForwardPlus, VisibilityBuffer}`, `Deferred` by default, chosen once at boot. See [Rendering overview](../rendering/overview.md#how-a-frame-flows).

**Repr** — Rust attribute (`#[repr(C)]`, `#[repr(align(N))]`, etc.) that controls memory layout of a struct. Required for FFI, transmute, and predictable cache behavior.

**Run condition** — A predicate attached with `run_if` that decides, each frame, whether a system runs (for example `in_state`). See [Run Conditions](../scheduling/run-conditions.md).

## S

**ScratchColumn** — `ScratchColumn<T>`: a `ComponentPool`-backed, address-stable, `Copy`-only scratch column. Solver and render staging use it instead of a `std::Vec`, so transient data also lives in the kernel's own storage.

**SIMD** (Single Instruction, Multiple Data) — CPU instructions that operate on multiple values simultaneously. Boyko Engine aims for SIMD-friendly data layout to enable auto-vectorization.

**SoA** (Struct of Arrays) — Memory layout where each field becomes a separate array. Cache-friendly when iterating over one field across many entities. The opposite of AoS.

**Solve region** — With `parallel_solve` on, one physics step's setup and all of its substeps run as one region on the threadpool, cut into blocks. The cut changes only where work runs, never a result bit. See [Physics](../simulation/physics.md#scaling--the-performance-paths).

**Sparse set** — A data structure pairing a dense array (for fast iteration) with a sparse array (for O(1) lookup by ID). Used by some ECS engines for component storage (e.g., EnTT).

**Speculative contact** — A contact kept while two shapes are still apart, within a margin `d + min(cap, approach · dt)`. It can stop an approach but never pushes the bodies apart. On by default. See [Physics](../simulation/physics.md#contacts-speculative-margin-and-reuse).

**System** — A function that operates on entities matching a query. In a typical ECS frame, systems run in scheduled order, possibly in parallel.

**System set** — A named label that groups systems for ordering (`in_set`, `configure_set`, `before_set` / `after_set`), so plugins in different crates can order against each other by name. See [System Ordering & Sets](../scheduling/ordering-and-sets.md).

**swap_remove** — An O(1) deletion strategy: replace the removed element with the last one and shrink the array. Breaks ordering but avoids shifting.

## T

**Tag** — A zero-sized component: it carries no data, only the fact of its presence on an entity. Stored as a tick-only pool (8 B/row), so `Added<Tag>`/`Changed<Tag>` work like on any component. See [Tags](../concepts/tags.md).

**TGS-Soft** — Temporal Gauss-Seidel "Soft Step": the substepped sequential-impulse rigid-body solver scheme in `boyko_physics` (the Box2D v3 lineage), with soft-constraint penetration recovery. See [Physics](../simulation/physics.md#the-tgs-soft-solver).

**TagId** — The handle of a *dynamic* tag, minted at runtime from a string name (`world.register_tag("name")`). A transparent, one-way-bridgeable wrapper over `ComponentId`. See [Dynamic Tags](../concepts/dynamic-tags.md).

## U

**`InlandUnitId`** — Boyko Engine's single-level row address: a `#[repr(transparent)]` newtype over `usize` naming a row index inside one `ComponentPool` ([`primitives.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/identifiers/primitives.rs)). Because each pool is one dense, address-stable column, a flat row index is all the addressing storage needs.

**Unsafe** — Rust code that bypasses the borrow checker or memory-safety guarantees. In Boyko Engine, every `unsafe` block carries a `// SAFETY:` comment explaining the invariants.

## V

**Visibility Buffer** — The render path that rasterizes only instance and triangle ids into a `R32G32_UINT` target, then shades in compute against a bindless per-mesh geometry table. It needs `shaderStorageBufferArrayNonUniformIndexing` and falls back to Deferred without it. See [Rendering overview](../rendering/overview.md#how-a-frame-flows).

## W

**Working set** — The amount of memory actively touched by a hot section of code. Designing for working sets that fit in L1d (~32 KB) or L2 (~256–512 KB) is a primary lever for performance. If the working set exceeds L3, throughput is bound by main memory bandwidth.

## Z

**Zero-cost abstraction** — A higher-level construct that compiles to the same machine code as hand-written low-level code. The cornerstone of Rust's performance model and a core principle of Boyko Engine.
