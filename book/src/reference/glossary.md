# Glossary

A reference of terms used throughout Boyko Engine and ECS architecture in general.

## A

**Archetype** — A unique combination of component types. All entities with the same set of components belong to the same archetype, and their components are stored together for cache-friendly iteration. *(Available on the `ecs` branch.)*

**Arena** — A pre-allocated, contiguous memory region from which all engine allocations are served. See [Arena Allocator](../memory/arena.md).

**AoS** (Array of Structs) — Memory layout where each entity is a single struct containing all its data. Cache-unfriendly when only some fields are accessed. The opposite of SoA.

## B

**Best-fit** — An allocation strategy that finds the *smallest* free block large enough to satisfy a request. Reduces fragmentation compared to first-fit.

**Bitmask / BitSet** — A compact representation of a set, where each bit indicates the presence of an element. Used in Boyko Engine to encode "which components an archetype contains".

## C

**Cache line** — The smallest unit of memory the CPU transfers between RAM and cache, typically 64 bytes on x86_64. Performance-critical data structures align to cache-line boundaries to avoid wasted transfers and false sharing.

**Cache, D-cache (data cache)** — CPU's hierarchy of data caches: L1d (~32 KB, ~4 cycles), L2 (~256–512 KB, ~12 cycles), L3 (megabytes, ~40 cycles). Hot loops aim to keep their working set in L1d for maximum throughput.

**Cache, I-cache (instruction cache)** — Separate cache for executable instructions, typically L1i ~32 KB on x86_64. Bloated hot paths (e.g., from aggressive inlining) cause I-cache misses, stalling the front-end of the CPU pipeline.

**Chunk** — A fixed-capacity buffer holding components of one type. See [Arena Allocator](../memory/arena.md) for how chunks are allocated. The actual storage type is `Chunk<T>` in [`memory/chunk.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/memory/chunk.rs).

**Component** — A piece of data attached to an entity. In Boyko Engine, components are POD-like structs implementing the `Component` trait via `#[derive(Component)]`.

**ComponentId** — A unique numeric identifier (`usize`) for a component type, assigned at compile time by the derive macro. *Not stable across rebuilds.*

**Compaction** — Process of removing gaps in storage to improve cache locality.

## D

**DOD** (Data-Oriented Design) — A design philosophy that prioritizes memory layout and data access patterns over object-oriented abstractions. See [Design Principles](../architecture/principles.md).

## E

**ECS** (Entity Component System) — An architectural pattern that separates data (components) from behavior (systems), with entities acting as identifiers that group components.

**Empty archetype** — The archetype with zero components. On the `ecs` branch entities may legally hold no components: removing the last component migrates the entity here instead of despawning it, and `spawn_empty()` creates entities here directly. See [Tags](../concepts/tags.md). *(Available on the `ecs` branch.)*

**Entity** — A lightweight identifier (an `id: u32` + `generation: u16` in Boyko Engine) that represents a "thing" in the game world. Entities themselves hold no data — their data lives in components.

**Existence-based processing** — Encoding state as component *presence* rather than a data field, so systems filter at archetype granularity instead of branching per row. The rationale behind tags. See [Storage Trade-offs](../architecture/storage-tradeoffs.md).

## F

**False sharing** — A performance bug where two threads write to different variables that happen to share a cache line, causing the cache line to bounce between cores. Mitigated by padding shared structures to cache-line boundaries.

**Free-block tracker** — A data structure that records which regions of an arena are currently unallocated. Boyko Engine uses `MemFreeBlockMaster`.

## G

**Generation** — A counter incremented each time an entity ID is reused. Combined with the ID, it disambiguates "old" references to deleted entities from references to new entities that reuse the slot.

## H

**Hot/cold split** — Separating frequently-accessed (hot) fields from rarely-accessed (cold) ones, putting hot fields together for cache efficiency.

## L

**Lock-free** — Concurrent code that makes guaranteed forward progress without using mutexes or other blocking primitives. Achieved through atomic operations.

## M

**Memory ordering** — In Rust atomics, the constraint on how loads and stores are visible across threads. Options: `Relaxed`, `Acquire`, `Release`, `AcqRel`, `SeqCst`.

**Monomorphization** — Rust's process of generating a specialized version of generic code for each concrete type used, eliminating runtime dispatch overhead.

## P

**PGO** (Profile-Guided Optimization) — A compiler optimization technique where the binary is built twice: first an instrumented version that collects runtime profiling data, then a final version that uses that data to make decisions about inlining, branch layout, and function placement. Typically yields 5–25% performance improvement on representative workloads.

**Prefetching** — Loading data into cache *before* it's needed. Hardware prefetchers detect sequential and stride access patterns; software prefetching (`_mm_prefetch` intrinsic) is used when patterns are predictable but the hardware can't see them (e.g., pointer-chasing through indices).

**Pool** — A pre-allocated collection of slots for objects of one type. See `ComponentPool<T>` for Boyko Engine's implementation.

**POD** (Plain Old Data) — A type with simple memory layout (no internal pointers, no destructor) that can be safely copied with `memcpy`.

## Q

**Query** — A specification of "which entities to operate on" based on which components they have. Queries iterate over all matching archetypes. *(Available on the `ecs` branch.)*

## R

**Repr** — Rust attribute (`#[repr(C)]`, `#[repr(align(N))]`, etc.) that controls memory layout of a struct. Required for FFI, transmute, and predictable cache behavior.

## S

**SIMD** (Single Instruction, Multiple Data) — CPU instructions that operate on multiple values simultaneously. Boyko Engine aims for SIMD-friendly data layout to enable auto-vectorization.

**SoA** (Struct of Arrays) — Memory layout where each field becomes a separate array. Cache-friendly when iterating over one field across many entities. The opposite of AoS.

**Sparse set** — A data structure pairing a dense array (for fast iteration) with a sparse array (for O(1) lookup by ID). Used by some ECS engines for component storage (e.g., EnTT).

**System** — A function that operates on entities matching a query. In a typical ECS frame, systems run in scheduled order, possibly in parallel.

**swap_remove** — An O(1) deletion strategy: replace the removed element with the last one and shrink the array. Breaks ordering but avoids shifting.

## T

**Tag** — A zero-sized component: it carries no data, only the fact of its presence on an entity. Stored as a tick-only pool (8 B/row), so `Added<Tag>`/`Changed<Tag>` work like on any component. See [Tags](../concepts/tags.md). *(Available on the `ecs` branch.)*

**TagId** — The handle of a *dynamic* tag, minted at runtime from a string name (`world.register_tag("name")`). A transparent, one-way-bridgeable wrapper over `ComponentId`. See [Dynamic Tags](../concepts/dynamic-tags.md). *(Available on the `ecs` branch.)*

## U

**`UnitId`** — Boyko Engine's two-level component address: `{ chunk: u32, inland: u32 }`. Compact (8 bytes) and cache-friendly for chunked storage.

**Unsafe** — Rust code that bypasses the borrow checker or memory-safety guarantees. In Boyko Engine, every `unsafe` block carries a `// SAFETY:` comment explaining the invariants.

## W

**Working set** — The amount of memory actively touched by a hot section of code. Designing for working sets that fit in L1d (~32 KB) or L2 (~256–512 KB) is a primary lever for performance. If the working set exceeds L3, throughput is bound by main memory bandwidth.

## Z

**Zero-cost abstraction** — A higher-level construct that compiles to the same machine code as hand-written low-level code. The cornerstone of Rust's performance model and a core principle of Boyko Engine.
