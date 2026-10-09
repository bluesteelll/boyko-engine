# Boyko Engine

A Rust ECS game engine built for throughput, cache locality (data and instruction) and native
parallelism.

Boyko Engine is a game engine written in Rust (edition 2024) around an Entity Component System.
Every subsystem — rendering, physics, lighting, input, UI — is built from components and systems on
the ECS's own column storage, not bolted on beside it. The public API is deliberately Bevy-shaped:
`App`, `Plugin`, `Query<D, F>`, `Commands`, `States`. The renderer is an in-house RHI over
hand-written Vulkan FFI. The physics is an in-house TGS-Soft solver whose parallel stages give
bit-identical results for any worker count. Beyond the procedural macros (`syn`, `quote`,
`proc-macro2`), the engine crates depend on a handful of small third-party crates: the `crossbeam`
family, `fixedbitset`, `static_assertions`, `bytemuck`, `ttf-parser`, and `windows-sys` or `libc`
for the OS.

## Status

- **Pre-release.** All 34 packages are at version `0.1.0`, and none is published on crates.io.
- **Under active development.**
- **The GPU path is Windows-only today.** See [Requirements and platforms](#requirements-and-platforms).

## Features

Status words: **Shipped** — in the tree, wired and tested. **Opt-in** — shipped, but off until you
turn it on. **Experimental** — in the tree, but partial or not wired into the default host.

### ECS kernel — Shipped

- Per-pool virtual-memory column storage: each column reserves address space and commits pages on
  demand, so row addresses stay stable. Columns are SIMD-aligned.
- Archetypes, with 16-byte direct-pointer entity records and generation-checked entity ids.
- A typed `Query<D, F>` with filters, `Added` / `Changed`, `par_iter` and chunked iteration.
- Systems, `Commands`, resources and events (`#[event]`, `EventReader` / `EventWriter`).
- A parallel scheduler that builds a conflict graph and runs non-conflicting systems on a Chase-Lev
  work-stealing pool, with ordering, system sets and run conditions.
- Change detection, states and the `state_chart!` macro, lifecycle hooks and observers,
  hierarchies, generic relations, required components, entity cloning and prefabs, tags, enable
  tags, dense (non-fragmenting) components, and multiple worlds.
- `App` and `Plugin`, with a fixed timestep (64 Hz by default) and render interpolation.
- Assets: `Assets<T>`, handles, an `AssetServer` and loaders.
- Derives and macros: `Component`, `Resource`, `Bundle`, `SystemSet`, `#[event]`, `state_chart!`
  and more, in [`boyko_macros`](crates/boyko_macros).

### Rendering

- **Shipped:** four render paths — Deferred (the default), Forward, Forward+ and Visibility Buffer —
  times three geometry legs (mesh, SDF, or both), chosen once at boot.
- **Shipped:** hybrid mesh + SDF rendering. Analytic SDF shapes are sphere-traced into the same
  G-buffer as the meshes.
- **Shipped:** textured PBR, bindless textures and GPU-resident component columns.
- **Shipped:** a shader eDSL. Shader math is written once in Rust, evaluated on the CPU as the test
  oracle, and printed as HLSL. `*_spv_sync` tests recompile shaders and compare the result with the
  committed SPIR-V byte for byte.
- **Shipped:** in-house loaders for PNG (with its own zlib/DEFLATE decoder), OBJ, binary glTF
  (`.glb`, static pose) and RON materials.
- **Shipped:** directional, point and spot lights and a sky ambient.
- **Opt-in:** cascaded sun shadows, a point/spot shadow atlas, SSAO, SDF-probe DDGI, anti-aliasing
  (FXAA, SMAA, SSAA, TAA with RCAS sharpening), two-phase HZB occlusion culling, clustered (froxel)
  light culling (`LightingConfig::clusters_enabled`, read once at boot), and GPU particles.
- **Opt-in** (`--features hwrt` and a ray-tracing GPU): hardware ray-query shadows, and a
  spatial/temporal denoiser for them.

Some opt-in passes depend on the render path. SSAO, DDGI, anti-aliasing and the shadow denoiser need
the Deferred or Visibility Buffer path; Forward and Forward+ switch them off. DDGI also needs the
SDF geometry leg. HZB occlusion and clustered light culling run only on the Visibility Buffer path.

### Physics

- **Shipped:** rigid bodies with sphere and oriented-box colliders.
- **Shipped:** a graph-coloured TGS-Soft solver with an AVX2 kernel: warm starting, Coulomb friction,
  restitution, sensors (triggers) and speculative contacts.
- **Shipped:** three broadphases — a BVH tree (the default), all-pairs, and a uniform grid.
- **Shipped:** a parallel solve and narrowphase whose results are bit-identical for any worker count.
- **Shipped:** `PhysicsPlugin`, which wires the pipeline into an `App`'s fixed schedule.
- **Opt-in:** island sleeping, and contacts against analytic SDF shapes.
- **Experimental:** soft bodies (an XPBD pass, with optional soft-rigid coupling). They are off by
  default. `PhysicsPlugin::soft` or `PhysicsPlugin::soft_colored` turns them on, as do the
  `add_physics_soft*` builder functions.

### UI

- **Shipped as a library:** an ECS-native UI in which widgets are entities. It has layout, `.ui`
  markup with hot reload, data binding, focus and interaction, world-space HUDs and tweens, plus an
  in-house MTSDF font baker.
- **Not yet drawn by the windowed host.** `boyko-app` does not compose the UI pass, so today the
  UI renders only in the GPU tests.

### Tooling

- **Shipped:** structured logging with compile-time level ceilings and numbered diagnostic codes.
- **Shipped:** a profiler, and the `BOYKO_PROFILE` build axis (`dev`, `editor`, `shipping`,
  `shipping-min`, `off`).
- **Shipped:** binary world save and load, generated by code, not by reflection.
- **Shipped:** rebindable input action mapping.
- **Experimental:** runtime reflection for an editor, absent from shipped builds.

### Languages

- **Experimental:** Aether, the `aether!` authoring DSL. It covers components, tags, bundles,
  systems, events, plugins, state machines, materials and scenes, and expands to the hand-written
  API.

### Not implemented yet

Joints; capsule, convex and mesh colliders; SSR, bloom, volumetrics and transparent materials;
skinning and animation; audio; an editor; virtual geometry.

## Requirements and platforms

- **CPU:** x86-64 with AVX2. [`.cargo/config.toml`](.cargo/config.toml) builds this repository with
  `-C target-cpu=x86-64-v3` (AVX2, FMA, BMI1/BMI2, F16C, LZCNT, MOVBE). The binaries hit an illegal
  instruction on CPUs older than Intel Haswell (2013) or AMD Excavator (2015).
- **Rendering:** Windows x86_64 and a Vulkan 1.3 GPU. Boot requires `samplerAnisotropy`,
  `geometryShader`, `dynamicRendering`, `shaderDemoteToHelperInvocation` and descriptor indexing
  (bindless textures), and fails fast on a device that lacks one. The Visibility Buffer path needs
  two more descriptor-indexing features; without them it falls back to Deferred.
- **Linux x86_64:** no GPU path yet — the Vulkan loader and the window are Windows-only. The
  CPU-side crates (the ECS, physics, math, scene and the rest) compile there.
- **macOS and wasm32:** not supported.
- **Rust:** stable ([`rust-toolchain.toml`](rust-toolchain.toml) pins the `stable` channel),
  edition 2024. No minimum supported Rust version is declared.
- **No Vulkan SDK needed** to build or run. The compiled SPIR-V is committed and embedded at compile
  time, and the engine needs only the system Vulkan loader (`vulkan-1.dll`). You need DXC from
  Vulkan SDK 1.4.350.0 only to change shaders.

## Getting started

```powershell
git clone https://github.com/bluesteelll/boyko-engine
cd boyko-engine
cargo build --release
```

The examples need Windows and a Vulkan 1.3 GPU:

| Command | What it shows |
|---------|---------------|
| `cargo run -p boyko-app --example clear` | A window cleared to one colour: the smallest host. |
| `cargo run -p boyko-app --example room` | A floor, four cubes, a sun with cascaded shadows, a sky fill and a point light. |
| `cargo run -p boyko-app --example viewer` | `room` with a fly camera (WASD + mouse). |
| `cargo run -p boyko-app --example showcase` | Meshes, a live SDF sphere, cascaded shadows and point + spot shadows, with a fly camera. |
| `cargo run -p boyko-app --example sdf_room` | `room` plus one live SDF sphere in the same G-buffer. |
| `cargo run -p boyko-app --example punctual_room` | `room` with the point/spot shadow atlas on. |
| `cargo run -p boyko-app --example bounce` | A cube bouncing on a 64 Hz fixed timestep, drawn interpolated. |
| `cargo run --release -p boyko-app --example playground` | The physics playground: shoot balls at a pyramid of cubes. |
| `scripts\run-scene.ps1 -Scene paradigm_lab -Path vb` | One scene in any render path (`-Path deferred\|forward\|forwardplus\|vb`) and geometry leg (`-Legs both\|mesh\|sdf`). |
| `scripts\run-vb-lab.ps1` | The Visibility Buffer test bed, with SSAO, DDGI and TAA on. |
| `cargo run -p boyko_demo --release` | An eframe/wgpu sandbox that exercises the ECS API. It does not use the engine's renderer. |

Environment variables the windowed examples read:

| Variable | Values |
|----------|--------|
| `BOYKO_RENDER_PATH` | `deferred` (default), `forward`, `forwardplus`, `vb` |
| `BOYKO_GEOMETRY_LEGS` | `both` (default), `mesh`, `sdf` |
| `BOYKO_AA` | `ssaa` asks for 2× supersampling, which boot grants when the device passes its size and memory check; `vb_lab` also reads `off`, `fxaa`, `smaa`, `taa` |
| `BOYKO_LOG` | `off`, `error`, `warn`, `info`, `debug`, `trace` — turns logging on at that level |

The playground's PBR material folders (`assets/materials/`) and downloaded models
(`assets/models/`) are not in the repository. Without them the playground renders untextured and
skips the models.

## Using the ECS from your crate

The engine is not on crates.io yet; depend on it through git:

```toml
[dependencies]
boyko-ecs = { git = "https://github.com/bluesteelll/boyko-engine" }
boyko-macros = { git = "https://github.com/bluesteelll/boyko-engine" }
```

You need **two** imports. The prelude exports the traits and types, but not the derive macros:
`boyko_macros` is only a dev-dependency of `boyko_ecs`, so its prelude cannot re-export them. Import
the derives you use from `boyko_macros` directly.

```rust
use boyko_ecs::prelude::*;
use boyko_macros::Component;

/// A plain data component. SoA-stored in a column; `#[repr(C)]` pins the layout.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Position {
    x: f32,
    y: f32,
}

/// Runs ONCE before the frame loop: spawn one entity carrying a `Position`.
fn setup(mut commands: Commands) {
    commands.spawn(Position { x: 1.0, y: 2.0 });
}

/// Runs every frame: read every `Position` and print it.
fn print_positions(query: Query<&Position>) {
    for pos in query.iter() {
        println!("position = ({}, {})", pos.x, pos.y);
    }
}

fn main() {
    App::new()
        .add_startup_system(setup)
        .add_systems(print_positions)
        .run_n(3); // run exactly 3 frames, then return
}
```

It prints `position = (1, 2)` three times.

Two build settings stay behind in this repository. Cargo reads `.cargo/config.toml` from the
directory you build in and its parents, and profiles from your own workspace's root manifest:

- **AVX2.** Without `-C target-cpu=x86-64-v3`, the AVX2 code paths compile out and the scalar
  fallbacks run. To keep them, add this to your project's `.cargo/config.toml`:

  ```toml
  [build]
  rustflags = ["-C", "target-cpu=x86-64-v3"]
  ```

- **Fat LTO.** This workspace builds `--release` with `lto = "fat"`. Set it in your own
  `[profile.release]` if you want it.

## Workspace layout

33 member crates plus the root package ([`Cargo.toml`](Cargo.toml)).

Engine:

- [`crates/boyko_ecs`](crates/boyko_ecs) — the ECS kernel: storage, archetypes, queries, systems,
  scheduler, commands, events, change detection, hooks and observers, relations, states, assets,
  `App` / `Plugin`.
- [`crates/boyko_memory`](crates/boyko_memory) — the virtual-memory reserve/commit primitive and
  column, one layer below the ECS pools.
- [`crates/boyko_macros`](crates/boyko_macros) — derives and macros: `Component`, `Bundle`,
  `Resource`, `SystemSet`, `#[event]`, `ui!`, `state_chart!` and more.
- [`crates/boyko_utils`](crates/boyko_utils) — `BitSet` / `BitSet256`, `SparseMap` /
  `SparseSlotMap`, `Slot`.
- [`crates/boyko_threadpool`](crates/boyko_threadpool) — the Chase-Lev work-stealing pool, with a
  loom-checked wake protocol.
- [`crates/boyko_diag`](crates/boyko_diag) — the diagnostics substrate: clock, lane topology,
  build-profile ceiling. No dependencies.
- [`crates/boyko_log`](crates/boyko_log) — in-house structured logging with compile-time ceilings.
- [`crates/boyko_math`](crates/boyko_math) — SIMD-aligned, bit-deterministic POD math.
- [`crates/boyko_scene`](crates/boyko_scene) — `Transform` / `GlobalTransform`, propagation,
  cameras, visibility.
- [`crates/boyko_physics`](crates/boyko_physics) — in-house 3D rigid and soft-body physics.
- [`crates/boyko_sdf_math`](crates/boyko_sdf_math) — `no_std` analytic SDF field math, shared by the
  GPU path and the CPU physics.
- [`crates/boyko_input`](crates/boyko_input) — rebindable action mapping over raw input events.
- [`crates/boyko_serialize`](crates/boyko_serialize) — binary world save and load.
- [`crates/boyko_reflect`](crates/boyko_reflect) — build-gated editor reflection, absent from shipped
  builds.
- [`crates/boyko_rhi`](crates/boyko_rhi) — the backend-agnostic RHI: traits and a handle registry,
  no FFI.
- [`crates/boyko_rhi_vulkan`](crates/boyko_rhi_vulkan) — the hand-written Vulkan backend: FFI,
  frame graph, Win32 window and swapchain.
- [`crates/boyko_render`](crates/boyko_render) — the ECS-to-RHI bridge: GPU columns, render paths,
  lighting, shadows, GI, AA, particles, loaders.
- [`crates/boyko_shaderdsl`](crates/boyko_shaderdsl) — the Rust shader eDSL: CPU oracle and HLSL
  printer.
- [`crates/boyko_image`](crates/boyko_image) — the in-house PNG decoder (zlib/DEFLATE), with no
  third-party dependencies.
- [`crates/boyko_fontbake`](crates/boyko_fontbake) — the load-time MTSDF font baker.
- [`crates/boyko_ui`](crates/boyko_ui) — the ECS-native UI.
- [`crates/boyko_app`](crates/boyko_app) — the host layer: OS loop, device boot, windowed runner,
  `EnginePlugins`, and the examples.
- [`crates/aether_lang`](crates/aether_lang), [`crates/aether`](crates/aether),
  [`crates/aether_tests`](crates/aether_tests) — the Aether DSL: parser and expander, the `aether!`
  macro, and its integration tests.

Apps, benchmarks, tools and test fixtures:

- [`crates/boyko_demo`](crates/boyko_demo) — an eframe/wgpu sandbox for the ECS API, separate from
  the engine renderer.
- [`crates/bench_bevy_vs_boyko`](crates/bench_bevy_vs_boyko) — criterion comparisons against
  `bevy_ecs`.
- [`tools/prof_decode`](tools/prof_decode) — prints a profiling telemetry stream as text.
- [`crates/boyko_symcensus`](crates/boyko_symcensus) — a dev-only instrument that censuses the
  symbols of post-LTO builds.
- [`crates/reflect_fixture`](crates/reflect_fixture), [`crates/reflect_dogfood`](crates/reflect_dogfood),
  [`crates/profile_fixture`](crates/profile_fixture),
  [`crates/profile_fixture_log`](crates/profile_fixture_log) — test fixtures for the reflection and
  profiling gates.
- The root package `boyko-engine` — a placeholder binary; its [`tests/`](tests) hold
  repository-wide census tests.

## Documentation

- **The book:** <https://bluesteelll.github.io/boyko-engine/> — start with
  [Installation](https://bluesteelll.github.io/boyko-engine/getting-started/installation.html) and
  [Your First App](https://bluesteelll.github.io/boyko-engine/getting-started/first-app.html).
- **The API reference:** <https://bluesteelll.github.io/boyko-engine/api/>, generated by
  `cargo doc`.
- **Internal maps:** [`docs/FEATURE_MAP.md`](docs/FEATURE_MAP.md),
  [`docs/SYSTEMS.md`](docs/SYSTEMS.md) and [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). They are
  written for the engine's development workflow and are partly out of date; where they disagree
  with the code, the code wins.

## Benchmarks

Measured results, each with its date and commit: [docs/BENCHMARKS.md](docs/BENCHMARKS.md).

- **Harnesses:** [`crates/boyko_ecs/benches`](crates/boyko_ecs/benches) (31 targets),
  [`crates/boyko_physics/benches`](crates/boyko_physics/benches) (17, including the
  `jolt_parity_pyramid` comparison with Jolt Physics) and
  [`crates/bench_bevy_vs_boyko/benches`](crates/bench_bevy_vs_boyko/benches) (9, against
  `bevy_ecs`).
- **Profiles:** `release` is the shipped build (fat LTO). `bench` turns LTO off and uses one codegen
  unit, for reproducible A/B comparisons. A number taken with `cargo bench` therefore does not
  describe the shipped build, so a result must name the profile it was taken under.
- **Methodology:** [`docs/BENCHMARKING.md`](docs/BENCHMARKING.md).

## Testing

```powershell
cargo test --workspace --all-targets --no-fail-fast
```

`--workspace` selects every member from any directory. `--no-fail-fast` keeps one failing test
target from hiding every target after it.

- **Ignored tests.** Tests that need a GPU, a window, a feature flag or a long wall clock are
  `#[ignore]`d with a reason. The reason starts with a class: `gpu`, `gpu-windowed`, `gpu-cap`,
  `feature`, `solo`, `slow` and a few more. The one exception is a test ignored only in release
  builds: it runs in every debug run, so its reason names no class. This census prints how many
  sites each class has:

  ```powershell
  cargo test -p boyko-engine --test ignore_reasons_census -- --nocapture
  ```

- **GPU tests** need a Vulkan device. Run them one test binary at a time with
  `-- --ignored --test-threads=1`; each file's header names the environment variables it reads.
- **Miri:** `cargo +nightly miri test -p <crate>`. [`.cargo/config.toml`](.cargo/config.toml) turns
  on Tree Borrows. The development host uses `nightly-x86_64-pc-windows-msvc`.
- **Loom:** lock-free protocols are model-checked by the `loom_*.rs` tests in
  [`crates/boyko_threadpool/tests`](crates/boyko_threadpool/tests) and
  [`crates/boyko_ecs/tests`](crates/boyko_ecs/tests). They compile only under `--cfg loom`; the
  `loom` job in [`.github/workflows/ci.yml`](.github/workflows/ci.yml) shows the flags.
- **Golden images:** `scripts\golden.ps1` checks rendered frames against the SHA-256 pins in
  [`goldens/PINS.toml`](goldens/PINS.toml). The pins are byte-identity hashes blessed on one NVIDIA
  RTX 3060, so they hold for that device.

## Design principles

1. **One engine.** Every subsystem is components and systems on the ECS's own storage. No subsystem
   keeps a parallel data store beside it.
2. **Zero runtime overhead.** No `dyn Trait`, `Box`, `HashMap` or `Vec::new()` on the hot path
   without a justification.
3. **Data-oriented design.** Struct of Arrays, with hot and cold fields split.
4. **Both caches.** Data layout for the D-cache; a compact hot path for the I-cache.
5. **Lock-free parallelism.** No `Mutex`, `RwLock` or `RefCell` on the hot path.
6. **Minimal allocation.** Preallocate during setup and reuse during gameplay.
7. **SIMD-friendly layout.** Data is laid out ready for vectorisation.
8. **Measured inlining.** `#[inline(always)]` only where a profiler or the assembly shows it
   matters.
9. **Justified `unsafe`.** Every `unsafe` block carries a `// SAFETY:` comment that states its
   invariants.

The build enforces part of this. [`clippy.toml`](clippy.toml) bans `HashMap`, `HashSet`, `Mutex`,
`RwLock`, `Rc` and `RefCell`, and the workspace denies that lint. Each exception in shipping code is
registered in [`docs/HOT-PATH-EXCEPTIONS.md`](docs/HOT-PATH-EXCEPTIONS.md) and checked by
[`scripts/check_hotpath_exceptions.py`](scripts/check_hotpath_exceptions.py); test code, benches and
examples are exempt. The
[Design Principles](https://bluesteelll.github.io/boyko-engine/architecture/principles.html) page in
the book explains the reasoning.

## Contributing

Read the book's [Contributing](https://bluesteelll.github.io/boyko-engine/contributing.html) page.
Report bugs and ask questions in the
[issue tracker](https://github.com/bluesteelll/boyko-engine/issues).

## License

Licensed under the Apache License, Version 2.0 ([LICENSE](LICENSE)). Some files are third-party and
remain under their own licenses; they are listed in [NOTICE](NOTICE) and
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

Copyright 2025-2026 Celtokisa
