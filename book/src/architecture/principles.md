# Design Principles

Boyko Engine is built on nine non-negotiable principles, numbered 0 through 8. Every line of code, every API decision, and every architectural trade-off is evaluated against them. Principle 0 is the most load-bearing — it shapes where data is allowed to live.

## 0. One unified engine — `boyko_ecs` is THE SDK for logic and data

There is one engine. Every system — physics, render, input, lighting, UI — is a first-class part of it: **components plus systems on the ECS's own storage**, never a subsystem glued on the side with its own data structures.

This means **no parallel data system**. Durable per-entity, per-element, or bulk subsystem data lives in the ECS's storage:

- **`ComponentPool` columns** — the default per-entity store.
- **`Resource`-owned columns** — for engine-global state.
- **Dense (non-fragmenting) components** — for the "one contiguous buffer for all instances" cases (solver state, GPU instances).

It never lives in a side `std::Vec` or `HashMap`. A capability a subsystem needs is promoted to a **first-class kernel feature** used uniformly by every system, not a per-crate adapter.

```rust
use boyko_ecs::prelude::*;    // the `Component` trait (and the rest of the public surface)
use boyko_macros::Component;  // the derive macro is NOT re-exported by the prelude

// ✅ Physics solver state is a dense component in the kernel — one contiguous
//    buffer, iterated by the physics systems on the engine's own scheduler.
#[derive(Component)]
struct SolverBody {
    inv_mass: f32,
    linear_velocity: [f32; 3],
}

// ❌ A `Vec<SolverBody>` owned by a physics crate, indexed in parallel with
//    the ECS, is a parallel data system — forbidden. It desynchronizes from
//    the kernel and was the root cause of the O11-SP4 colored-solve data race.
```

Why this is also the fast path: the kernel storage (`ComponentPool` on a `VmReservation`, SIMD-aligned, address-stable, recomputed row pointers) *is* the cache-optimal storage. "ECS-native" and "cache-optimal" are the same thing, so deep integration costs no performance.

**Legitimate exceptions** (not violations): the ECS storage implementation itself; FFI / GPU / OS-contiguity buffers (Vulkan `*const T + count`, swapchain images, the OS input ring); lock-free threadpool internals; and truly transient function-local scratch.

## 1. Zero runtime overhead

All abstractions are compile-time. Generic code is monomorphized into direct calls — no virtual dispatch, no dynamic lookup in hot paths.

```rust
// ❌ Avoided in hot paths:
fn process(components: &mut [Box<dyn Component>]) { ... }

// ✅ Preferred:
fn process<T: Component>(components: &mut [T]) { ... }
```

## 2. Data-Oriented Design

Data structures are designed around access patterns, not conceptual models. Struct of Arrays (SoA) beats Array of Structs (AoS) wherever multiple entities are processed together. Hot and cold fields are split.

```rust
// ❌ AoS — wastes cache lines if only `position` is read:
struct Entity {
    position: Vec3,
    velocity: Vec3,
    metadata: HashMap<String, String>,
}
let entities: Vec<Entity>;

// ✅ SoA — each component type gets its own dense column. In the kernel a
//    column is a type-erased `ComponentPool` (one per component type), keyed
//    by `ComponentId`; row `i` of every column belongs to the same entity:
//
//      pools[POSITION_ID] -> [ pos_0 | pos_1 | pos_2 | ... ]   (tight bytes)
//      pools[VELOCITY_ID] -> [ vel_0 | vel_1 | vel_2 | ... ]
//      pools[METADATA_ID] -> [ meta_0 | meta_1 | ... ]
//
// `ComponentPool` is type-erased (no `<T>`); a system reads a column back as
// `&[T]` through its typed `Query`. A loop that only touches `Position` streams
// the position column and never loads a velocity byte.
```

## 3. Cache optimization (data and instruction)

Modern CPUs spend the bulk of their time waiting on cache. Boyko Engine optimizes **both** the data cache (L1d/L2/L3) and the instruction cache (L1i) — neglecting either one bottlenecks the other.

### Data cache (L1d / L2 / L3)

Layout is engineered for sequential, predictable access:

- **`#[repr(C)]`** where memory layout matters (FFI, `transmute`, `memcpy`, predictable cache footprint).
- **Cache-line alignment (64 bytes)** for structures shared between threads — prevents false sharing, where two cores ping-pong a cache line they don't logically share.
- **Dense per-type columns**: each component type is stored as one contiguous byte buffer (a `ComponentPool`), row `i` at `buffer + i * stride`. No chunking, no gaps — streaming reads hit L1d at maximum rate.
- **Stride-clamped reservation**: a pool reserves address space for `clamp(POOL_TARGET_DATA_BYTES / stride, POOL_MIN_ROWS, POOL_MAX_ROWS)` rows — roughly `1 GiB / stride` on 64-bit syscall arms, clamped to `[65_536, 16_777_216]` rows (a zero-stride ZST pool routes to the max) — so tiny components get many rows and large components get fewer, without per-size chunk classes to manage.
- **Cache-set staggering**: pool data bases are offset off the bare reservation base so different columns land in different L1/L2 cache sets, avoiding set aliasing when several columns are iterated together.
- **Hot/cold splits**: rarely-accessed fields are pulled into separate storage so hot fields fill cache lines fully.
- **Software prefetching** is applied where access patterns are predictable but the CPU prefetcher fails (e.g., pointer-chasing through indices).
- **Non-temporal stores** for streaming writes that should bypass the cache (large buffer initialization, frame buffer writes).
- **Working-set sizing**: critical hot loops are designed so their working set fits in **L1d (~32 KB)** or at worst **L2 (~256–512 KB)**. When the working set exceeds L3, the design is revisited — random access to multi-megabyte structures destroys throughput.

### Instruction cache (L1i)

Hot code paths must fit in the instruction cache (typically **32 KB**) to sustain throughput:

- **Measured inlining, not aggressive** (see principle 7) — blind `#[inline(always)]` bloats hot paths and evicts useful instructions.
- **`#[cold]` / `#[inline(never)]`** on error paths, panic helpers, and rarely-taken branches keeps them out of the hot path entirely.
- **Branch density** is monitored — every conditional consumes instruction bytes and may flush the pipeline on misprediction.
- **Loop bodies are minimized**: excessive unrolling is counter-productive once it pushes the body out of L1i.
- **Profile-Guided Optimization (PGO)**: when a representative workload exists, building with `-Cprofile-use=...` lets the compiler place hot functions adjacently, reorder branches for predicted paths, and skip inlining decisions that hurt icache utilization. PGO outperforms hand-tuned attributes.

### TLB and large allocations

For working sets exceeding a few megabytes, **TLB pressure** becomes a factor. Each `ComponentPool` reserves a large contiguous region of address space (a 1 GiB target on 64-bit syscall arms) and commits pages lazily, so a hot column's resident pages are physically contiguous — a natural fit for 2 MB huge pages. Future versions may explicitly request huge pages from the OS per reservation to reduce TLB misses on large iterations.

See [Storage Trade-offs](storage-tradeoffs.md) for the data-model decisions behind these columns, and [Design Principle 7](#7-measured-inlining) for the I-cache implications of inlining.

## 4. Lock-free parallelism

There are no `Mutex`, `RwLock`, `RefCell`, or `Rc` in hot paths. Parallelism is achieved through:

- **Data partitioning** — different threads work on disjoint row ranges of the same columns.
- **Atomic operations** — when shared state is needed.
- **System scheduling** — Rust's borrow checker is enforced at the scheduler level so that systems with conflicting component access never run simultaneously.

## 5. Minimal allocations

No allocations in frame loops. Memory is reserved up front and committed on demand:

- **Per-pool virtual reservation**: each `ComponentPool` owns its own `VmReservation`. At construction it reserves a large region of address space (a 1 GiB data target on 64-bit syscall arms) with **no commit charge and zero resident bytes** — the shared engine-wide arena was retired (Phase X.J); there is no single 64 MB region anymore.
- **Lazy commit, stable addresses**: pages are committed on demand at the frontier of the reservation as rows are added. Growth is O(1) in live rows — no bytes are copied and previously returned pointers never move, so a column can grow mid-frame without invalidating any in-flight query pointer. There are no chunks (the `Chunk` type was removed); each pool is one dense contiguous buffer.
- **Capacity hints**: internal vectors are sized from the start so they never reallocate on the hot path.

The OS reservation happens at setup; page commits happen on the first growth past the committed frontier, not on every spawn during gameplay.

## 6. SIMD-friendly layout

Data is structured so the compiler can auto-vectorize, and explicit SIMD (`std::simd` or `core::arch` intrinsics) can be applied where profiling demands it:

- Components stored as plain old data (POD) where possible.
- No interior pointers — references are encoded as indices.
- Alignment guarantees enable aligned loads/stores.

## 7. Measured inlining

Function call overhead is real in hot loops — but so is **excessive** inlining. Aggressive `#[inline(always)]` everywhere bloats the binary, increases L1 instruction cache pressure, raises register pressure, and can ultimately make hot code **slower**, not faster.

The engine uses inlining attributes deliberately:

- **`#[inline]`** — applied to cross-crate functions and generic methods. Without it, the function body is unavailable to the calling crate (unless LTO is enabled), preventing inlining where it would help.
- **`#[inline(always)]`** — used only when a profiler or assembly inspection (`cargo asm`, `cargo rustc -- --emit asm`) demonstrates that the compiler's heuristic fails to inline and that this measurably affects performance. Each occurrence carries a comment justifying it.
- **`#[cold]` / `#[inline(never)]`** — applied to error paths, panic helpers, and rarely-taken branches. This keeps the hot path compact in the instruction cache.

By default, we trust the compiler. Rust's inliner is conservative but well-tuned. Measurements drive inlining decisions, not doctrine. The combination of generics, monomorphization, and the compiler's default inliner already yields the same machine code as hand-written specialized routines in the overwhelming majority of cases — explicit `#[inline]` is a targeted tool, not a default.

## 8. Documented unsafe

`unsafe` is used liberally where it enables performance gains — but every block carries a `// SAFETY:` comment explaining the invariants the caller must uphold:

```rust
// SAFETY: `idx < self.len` is checked above, so the row is initialized.
// `buffer` is write-once and never relocates (growth only commits fresh
// pages at the frontier), so this pointer is valid for the pool's lifetime.
unsafe { &*self.buffer.as_ptr().add(idx * self.component_layout.size()).cast::<T>() }
```

This makes the codebase auditable and ensures `unsafe` is never used carelessly.

## What we will not do

To keep the principles honest:

- **No `Box<dyn Trait>`** in hot paths.
- **No `HashMap`** where an array indexed by `ComponentId` works.
- **No `Vec::new()` / `format!()` / `clone()` of large data** inside frame loops.
- **No "general-purpose" abstractions** that pay performance for flexibility we don't need.
- **No "we'll optimize later"**. Performance is a primary feature, designed in from day one.

## See also

- [Storage Trade-offs](storage-tradeoffs.md) — where data lives and what each storage choice costs.
- [API reference](https://bluesteelll.github.io/boyko-engine/api/) — auto-generated rustdoc.
- [Contributing](../contributing.md) — how to extend the engine while preserving these principles.
