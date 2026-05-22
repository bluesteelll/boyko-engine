# Design Principles

Boyko Engine is built on eight non-negotiable principles. Every line of code, every API decision, and every architectural trade-off is evaluated against them.

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

// ✅ SoA — each component pool is a tight array:
let positions: ComponentPool<Position>;
let velocities: ComponentPool<Velocity>;
let metadata: ComponentPool<Metadata>;
```

## 3. Cache optimization (data and instruction)

Modern CPUs spend the bulk of their time waiting on cache. Boyko Engine optimizes **both** the data cache (L1d/L2/L3) and the instruction cache (L1i) — neglecting either one bottlenecks the other.

### Data cache (L1d / L2 / L3)

Layout is engineered for sequential, predictable access:

- **`#[repr(C)]`** where memory layout matters (FFI, `transmute`, `memcpy`, predictable cache footprint).
- **Cache-line alignment (64 bytes)** for structures shared between threads — prevents false sharing, where two cores ping-pong a cache line they don't logically share.
- **Chunked storage** keeps components of the same type contiguous, enabling streaming reads with maximum L1d hit rate.
- **Adaptive chunk sizing**: tiny components are densely packed for high L1 utilization; large components use smaller chunks to avoid wasted space.
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

For working sets exceeding a few megabytes, **TLB pressure** becomes a factor. Boyko Engine's 64 MB arena fits in modern 2 MB huge pages — future versions may explicitly request huge pages from the OS to reduce TLB misses on large iterations.

See [Arena Allocator](../memory/arena.md) for the foundation of D-cache optimization, and [Design Principle 7](#7-measured-inlining) for the I-cache implications of inlining.

## 4. Lock-free parallelism

There are no `Mutex`, `RwLock`, `RefCell`, or `Rc` in hot paths. Parallelism is achieved through:

- **Data partitioning** — different threads work on different chunks.
- **Atomic operations** — when shared state is needed.
- **System scheduling** — Rust's borrow checker is enforced at the scheduler level so that systems with conflicting component access never run simultaneously.

## 5. Minimal allocations

No allocations in frame loops. Everything is pre-allocated:

- Arena holds 64 MB by default — all entity/component memory comes from here.
- Component pools pre-allocate all their chunks during construction.
- Vectors used internally have capacity hints from the start.

Allocations happen at setup, not during gameplay.

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
// SAFETY: `index < self.count` is checked above. The slot at `index` was
// previously written by `add()` or `set()`, so it contains a valid `T`.
unsafe { Some(&*self.data.as_ptr().add(index)) }
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

- [Arena Allocator](../memory/arena.md) — the memory foundation.
- [API reference](https://bluesteelll.github.io/boyko-engine/api/) — auto-generated rustdoc.
- [Contributing](../contributing.md) — how to extend the engine while preserving these principles.
