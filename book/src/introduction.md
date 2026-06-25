# Boyko Engine

**Boyko Engine** is a high-performance game engine written in Rust (2024 edition), built around an Entity Component System (ECS) core. It targets **ultimate performance, cache optimization (data and instruction), and native parallelism** — with no compromises in favor of convenience.

The engine is **ECS-native end to end**: every subsystem — render, physics, lighting, UI, input — is built from components and systems on the kernel's own storage, not glued on the side with its own data structures.

## Status

**Active and building green.** The full engine lives on the `ecs` branch (this is what these docs describe). It is the cumulative result of Phases 2 → 22 plus the executor-soundness and memory perf series, and it builds clean. The `master` branch is the historical snapshot of the original memory subsystem only — it predates archetypes, queries, systems, and everything above them.

If you are reading the source, you want the `ecs` branch.

## What's in the box

Boyko Engine is an 18-crate Cargo workspace. The major pieces:

- **ECS kernel** ([`boyko_ecs`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_ecs)) — type-erased component storage, archetypes, a typed `Query<D, F>` DSL, `Commands`/`EntityCommands`, events, resources, lifecycle hooks + observers, required components, entity cloning, generic relations (`ChildOf`/`Children`), states, schedule ordering/sets, run conditions, change detection, dense (non-fragmenting) components, an `App` + `Plugin` facade, fixed timestep, and multi-world support.
- **Derives & utilities** ([`boyko_macros`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_macros), [`boyko_utils`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_utils)) — `#[derive(Component/Bundle)]`, `#[event]`, plus `BitSet`/`SparseMap`/`Slot` collections.
- **Parallel scheduler** ([`boyko_threadpool`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_threadpool)) — a Chase-Lev work-stealing pool that runs non-conflicting systems concurrently.
- **Math & scene** ([`boyko_math`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_math), [`boyko_scene`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_scene)) — vector/matrix math and `Transform`/`Camera` standard-library components.
- **In-house render** ([`boyko_rhi`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_rhi) + [`boyko_rhi_vulkan`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_rhi_vulkan) + [`boyko_render`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_render)) — a from-scratch RHI over raw-FFI Vulkan, GPU-resident component columns, and clustered lighting.
- **SDF** ([`boyko_sdf_math`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_sdf_math) + [`boyko_shaderdsl`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_shaderdsl)) — signed-distance-field sphere tracing, a GPU brick atlas, and a Rust-hosted shader eDSL that single-sources the field math for both the CPU oracle and the GPU shaders.
- **In-house physics** ([`boyko_physics`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_physics)) — a 3D TGS-Soft rigid solver plus soft bodies, built without external FFI.
- **ECS-native UI** ([`boyko_ui`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_ui) + [`boyko_fontbake`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_fontbake)) — widgets are entities; MSDF text atlases are baked in-house.
- **Input & serialization** ([`boyko_input`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_input), [`boyko_serialize`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_serialize)) — an OS input ring exposed as ECS state, and codegen (not reflection) binary serialization.
- **Apps & benches** ([`boyko_demo`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/boyko_demo), [`bench_bevy_vs_boyko`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/bench_bevy_vs_boyko)) — a GPU-instanced sandbox that dogfoods the public API, and head-to-head benchmarks against Bevy.

## Why another engine?

Most Rust ECS frameworks (Bevy, hecs, legion) trade some raw throughput for ergonomics. Boyko Engine takes the opposite stance: a deliberately Bevy-shaped public API (`Query<D, F>`, `Commands`, `App`/`Plugin`, `States`) sitting on top of a maximally fast core, with zero runtime overhead in hot paths. The project benchmarks directly against Bevy ([`bench_bevy_vs_boyko`](https://github.com/bluesteelll/boyko-engine/tree/ecs/crates/bench_bevy_vs_boyko)) to keep that claim honest.

Key design choices:

- **Per-pool virtual-memory storage** — each component column owns a virtual-address reservation (`VmReservation`) that is lazily committed on demand, so spawning never calls the global allocator on the hot path. Columns grow by slab-doubling in bytes (clamped to 64 KiB … 64 MiB — e.g. 1024 → 2048 → 4096 rows for a 64-byte component) and their base addresses are stable, so cached row pointers never dangle.
- **Direct-pointer entity records** — the fast-path location record is `EntityInland { archetype_ptr, unit_index, generation }` (16 bytes). `get_component` dereferences straight into the archetype slab with no sparse-map indirection.
- **Struct-of-Arrays, cache-line aligned** — components live in SIMD-aligned columns (`#[repr(C)]` where layout matters), with hot/cold field splits and per-pool base staggering to avoid cache-set conflicts across columns.
- **Lock-free by design** — no `Mutex`/`RwLock` in hot paths; parallelism comes from partitioning and atomics on the work-stealing scheduler.
- **Measured inlining** — `#[inline]` is applied deliberately, never blanket; generics are preferred over `dyn Trait`.
- **Documented unsafe** — every `unsafe` block carries a `// SAFETY:` comment with explicit invariants.

## Reading guide

- **New to ECS?** Start with [Design Principles](architecture/principles.md).
- **Curious how memory works?** See [Memory](memory/arena.md) for the per-pool virtual-memory reserve/commit storage model.
- **Looking for the API?** See the [API reference](https://bluesteelll.github.io/boyko-engine/api/boyko_ecs/index.html) (auto-generated from `cargo doc`).
- **Want to contribute?** See [Contributing](contributing.md).

## Project links

- **Source code**: [github.com/bluesteelll/boyko-engine](https://github.com/bluesteelll/boyko-engine)
- **Issues**: [issue tracker](https://github.com/bluesteelll/boyko-engine/issues)
- **API documentation**: [auto-generated rustdoc](https://bluesteelll.github.io/boyko-engine/api/)

## License

See the repository for licensing information.
