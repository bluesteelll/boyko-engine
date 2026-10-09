# Boyko Engine

**Boyko Engine** is a high-performance game engine written in Rust (2024 edition), built around an Entity Component System (ECS) core. It targets **ultimate performance, cache optimization (data and instruction), and native parallelism** — with no compromises in favor of convenience.

The engine is **ECS-native end to end**: every subsystem — render, physics, lighting, UI, input — is built from components and systems on the kernel's own storage, not glued on the side with its own data structures.

## Status

**Pre-release and under active development.** The full engine is on the `master` branch, and this book describes it. No crate is published on crates.io yet; see [Installation](getting-started/installation.md) for how to depend on the engine.

## What's in the box

Boyko Engine is a Cargo workspace of 33 member crates plus the root package. The major pieces:

- **ECS kernel** ([`boyko_ecs`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_ecs)) — type-erased component storage, archetypes, a typed `Query<D, F>` DSL, `Commands`/`EntityCommands`, events, resources, lifecycle hooks + observers, custom triggers and entity-scoped observers, required components, entity cloning and prefabs, generic relations (`ChildOf`/`Children`), states, schedule ordering/sets, run conditions, change detection, dense (non-fragmenting) components, an asset store with `AssetServer` loading, an `App` + `Plugin` facade, fixed timestep, and multi-world support.
- **Memory primitives** ([`boyko_memory`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_memory)) — `VmReservation` (reserve/commit/release of virtual memory) and `VmColumn` (a typed, address-stable column), the layer every kernel store is built on.
- **Derives & utilities** ([`boyko_macros`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_macros), [`boyko_utils`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_utils)) — `#[derive(Component/Bundle)]`, `#[event]`, the hierarchical `state_chart!` macro, plus `BitSet`/`SparseMap`/`Slot` collections.
- **Parallel scheduler** ([`boyko_threadpool`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_threadpool)) — a Chase-Lev work-stealing pool that runs non-conflicting systems concurrently.
- **Host layer** ([`boyko_app`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_app)) — the OS loop, device boot, the windowed runner and `EnginePlugins`: the entry point for a windowed app. See [Windowed Host](app/windowed-host.md).
- **Math & scene** ([`boyko_math`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_math), [`boyko_scene`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_scene)) — vector/matrix math and `Transform`/`Camera` standard-library components.
- **In-house render** ([`boyko_rhi`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_rhi) + [`boyko_rhi_vulkan`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_rhi_vulkan) + [`boyko_render`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_render)) — a from-scratch RHI over raw-FFI Vulkan, GPU-resident component columns, selectable render paths, and a lighting stack.
- **SDF** ([`boyko_sdf_math`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_sdf_math) + [`boyko_shaderdsl`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_shaderdsl)) — signed-distance-field sphere tracing, a GPU brick atlas, and a Rust-hosted shader eDSL that single-sources the field math for both the CPU oracle and the GPU shaders.
- **Image decoding** ([`boyko_image`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_image)) — an in-house PNG decoder (zlib/DEFLATE) with no third-party dependencies.
- **In-house physics** ([`boyko_physics`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_physics)) — a 3D TGS-Soft rigid solver plus soft bodies, built without external FFI.
- **ECS-native UI** ([`boyko_ui`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_ui) + [`boyko_fontbake`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_fontbake)) — widgets are entities; MSDF text atlases are baked in-house.
- **Input & serialization** ([`boyko_input`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_input), [`boyko_serialize`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_serialize)) — rebindable action mapping over raw input events, and codegen (not reflection) binary serialization.
- **Diagnostics** ([`boyko_log`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_log), [`boyko_diag`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_diag)) — in-house structured logging with `boyko-Cnnnn` diagnostic codes, and the clock and lane substrate that logging and the profiler share.
- **Editor reflection** ([`boyko_reflect`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_reflect), experimental) — runtime reflection behind a Cargo feature, for editor builds only; shipped builds do not contain it.
- **Aether DSL** ([`aether`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/aether) + [`aether_lang`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/aether_lang)) — one `aether!` macro whose constructs expand to the hand-written API. See [Aether](aether/overview.md).
- **Apps & benches** ([`boyko_demo`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_demo), [`bench_bevy_vs_boyko`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/bench_bevy_vs_boyko)) — an eframe/wgpu sandbox that dogfoods the ECS API (separate from the engine renderer), and head-to-head benchmarks against Bevy.

## Why another engine?

Most Rust ECS frameworks (Bevy, hecs, legion) trade some raw throughput for ergonomics. Boyko Engine takes the opposite stance: a deliberately Bevy-shaped public API (`Query<D, F>`, `Commands`, `App`/`Plugin`, `States`) sitting on top of a maximally fast core, with zero runtime overhead in hot paths. The project benchmarks directly against Bevy ([`bench_bevy_vs_boyko`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/bench_bevy_vs_boyko)) to keep that claim honest; see [Benchmarks](reference/benchmarks.md) for the harnesses and the published results.

Key design choices:

- **Per-pool virtual-memory storage** — each component column owns a virtual-address reservation (`VmReservation`) that is lazily committed on demand, so spawning never calls the global allocator on the hot path. Reservations round to 64 KiB; commits step by 4 KiB pages, doubling up to 64 MiB slabs. Column base addresses are stable, so cached row pointers never dangle.
- **Direct-pointer entity records** — the fast-path location record is `EntityInland { archetype_ptr, unit_index, generation }` (16 bytes). `get_component` dereferences straight into the archetype slab with no sparse-map indirection.
- **Struct-of-Arrays, cache-line aligned** — components live in SIMD-aligned columns (`#[repr(C)]` where layout matters), with hot/cold field splits and per-pool base staggering to avoid cache-set conflicts across columns.
- **Lock-free by design** — no `Mutex`/`RwLock` in hot paths; parallelism comes from partitioning and atomics on the work-stealing scheduler.
- **Measured inlining** — `#[inline]` is applied deliberately, never blanket; generics are preferred over `dyn Trait`.
- **Documented unsafe** — every `unsafe` block carries a `// SAFETY:` comment with explicit invariants.

## Reading guide

- **Setting up?** Start with [Installation](getting-started/installation.md), then [Your First App](getting-started/first-app.md).
- **New to ECS?** Read [Design Principles](architecture/principles.md).
- **Curious how memory works?** See [Per-Pool Virtual Memory](memory/arena.md) for the reserve/commit storage model.
- **Looking for the API?** See the [API reference](https://bluesteelll.github.io/boyko-engine/api/boyko_ecs/index.html) (auto-generated from `cargo doc`).
- **Want a shorter notation?** [Aether](aether/overview.md) is the engine's authoring DSL: one `aether!` macro whose constructs expand to exactly the hand-written surface this book documents.
- **Want to contribute?** See [Contributing](contributing.md).

## Project links

- **Source code**: [github.com/bluesteelll/boyko-engine](https://github.com/bluesteelll/boyko-engine)
- **Issues**: [issue tracker](https://github.com/bluesteelll/boyko-engine/issues)
- **API documentation**: [auto-generated rustdoc](https://bluesteelll.github.io/boyko-engine/api/)

## License

Licensed under the Apache License, Version 2.0 ([LICENSE](https://github.com/bluesteelll/boyko-engine/blob/master/LICENSE)). Some files are third-party and remain under their own licenses; they are listed in [NOTICE](https://github.com/bluesteelll/boyko-engine/blob/master/NOTICE) and [THIRD-PARTY-NOTICES.md](https://github.com/bluesteelll/boyko-engine/blob/master/THIRD-PARTY-NOTICES.md).

Copyright 2025-2026 Celtokisa
