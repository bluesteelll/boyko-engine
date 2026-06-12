# Boyko Engine

**Boyko Engine** is a high-performance Entity Component System (ECS) game engine written in Rust 2024 edition. It targets **ultimate performance, cache optimization (data and instruction), and native parallelism** — with no compromises in favor of convenience.

## Status

**Early development.** The original memory subsystem lives on `master`; the full ECS layer (archetypes, queries, systems, the parallel scheduler, change detection, tags) is developed on the `ecs` branch and has not been merged yet.

Individual pages note which branch they describe: the memory pages document `master`, while the newer pages (scheduler, change detection, tags, storage trade-offs) document `ecs`.

## Why another ECS?

Existing Rust ECS frameworks (Bevy, hecs, legion) make trade-offs in favor of ergonomics. Boyko Engine takes the opposite stance: the public API is built on top of a maximally fast core, with zero runtime overhead in hot paths.

Key design choices:

- **Arena-based allocation** — no global allocator calls in hot paths; one 64 MB pre-allocated region serves all components.
- **Adaptive chunk sizing** — chunk capacity adjusts to component size for optimal cache-line utilization (tiny components pack 2048/chunk, large ones 256/chunk).
- **Two-level addressing** — `UnitId { chunk: u32, inland: u32 }` keeps indices compact (8 bytes) and cache-friendly.
- **Lock-free by design** — no `Mutex`/`RwLock` in hot paths; parallelism through partitioning and atomics.
- **Measured inlining** — `#[inline]` applied deliberately, not blanket; generics over `dyn Trait`.
- **Documented unsafe** — every `unsafe` block carries a `// SAFETY:` comment with explicit invariants.

## Reading guide

- **New to ECS?** Start with [Design Principles](architecture/principles.md).
- **Curious how memory works?** See [Arena Allocator](memory/arena.md).
- **Looking for the API?** See the [API reference](https://bluesteelll.github.io/boyko-engine/api/boyko_ecs/index.html) (auto-generated from `cargo doc`).
- **Want to contribute?** See [Contributing](contributing.md).

## Project links

- **Source code**: [github.com/bluesteelll/boyko-engine](https://github.com/bluesteelll/boyko-engine)
- **Issues**: [issue tracker](https://github.com/bluesteelll/boyko-engine/issues)
- **API documentation**: [auto-generated rustdoc](https://bluesteelll.github.io/boyko-engine/api/)

## License

See the repository for licensing information.
