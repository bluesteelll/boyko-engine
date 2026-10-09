# Installation

> What you need to build boyko-engine, run its examples, and use it from your own crate.

boyko-engine is a Cargo workspace of 33 member crates plus the root package. Nothing is on
crates.io yet: you either clone the repository or depend on it through git.

## Platforms and CPU

| Platform | What works |
|----------|------------|
| Windows x86_64 | Everything, including the GPU path: the Vulkan renderer, the window and the examples. |
| Linux x86_64 | No GPU path yet: the Vulkan loader and the window are Windows-only. The CPU-side crates (the ECS, physics, math, scene and the rest) compile there. |
| macOS, wasm32 | Not supported. |

**CPU.** The repository's
[`.cargo/config.toml`](https://github.com/bluesteelll/boyko-engine/blob/master/.cargo/config.toml)
builds the x86_64 Windows and Linux targets with `-C target-cpu=x86-64-v3`: AVX2, FMA, BMI1/BMI2,
F16C, LZCNT and MOVBE. The binaries therefore hit an illegal instruction on CPUs older than Intel Haswell (2013)
or AMD Excavator (2015).

## GPU

The renderer targets **Vulkan 1.3** through the system loader (`vulkan-1.dll`). At boot the device
must support:

- `samplerAnisotropy` and `geometryShader` (Vulkan 1.0 core features);
- `dynamicRendering` and `shaderDemoteToHelperInvocation` (Vulkan 1.3 features);
- the five descriptor-indexing features of the bindless texture path;
- the basic and ballot subgroup operations in compute shaders. Core Vulkan does not guarantee
  ballot (`VK_SUBGROUP_FEATURE_BALLOT_BIT`); the GPU particle simulation uses it.

The boot also checks a few storage-image formats and the per-stage descriptor limits. A device that
misses any of these fails fast with a `BootError` that names the missing piece. The tables are
`REQUIRED_CORE`, `REQUIRED_V13` and `REQUIRED_SUBGROUP_OPERATIONS` in
[`device.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/src/device.rs).

Two capabilities are optional:

- The **Visibility Buffer** render path needs two more descriptor-indexing features. Without them
  the boot falls back to the Deferred path instead of failing.
- **Hardware ray-query shadows** need the `hwrt` Cargo feature and a ray-tracing GPU.

## Toolchain

- **Rust:** stable. [`rust-toolchain.toml`](https://github.com/bluesteelll/boyko-engine/blob/master/rust-toolchain.toml)
  pins the `stable` channel, and every crate uses edition 2024. No minimum supported Rust version
  is declared.
- **Vulkan SDK:** not needed to build or run. The compiled SPIR-V is committed and embedded at
  compile time.
- **DXC:** only to change shaders. The shader sync tests look for the DXC of Vulkan SDK
  1.4.350.0; see [Shader eDSL](../rendering/shader-edsl.md).
- **Validation layer:** optional. The windowed host does not request it unless
  `BOYKO_ENABLE_VALIDATION` is set, so a machine without the SDK still boots.

## Building the repository

```powershell
git clone https://github.com/bluesteelll/boyko-engine
cd boyko-engine
cargo build --release
```

The root manifest lists every member as a default member, so a bare `cargo build` covers the whole
workspace. That includes `boyko_demo`, an eframe/wgpu sandbox with its own dependency tree. To
build only the windowed host and its examples, select the package:

```powershell
cargo build --release -p boyko-app --examples
```

`--release` uses fat LTO (`[profile.release] lto = "fat"` in the root
[`Cargo.toml`](https://github.com/bluesteelll/boyko-engine/blob/master/Cargo.toml)).

## Running the examples

The examples live in `crates/boyko_app/examples` and need Windows and a Vulkan 1.3 GPU.

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

The playground's PBR material folders (`assets/materials/`) and downloaded models
(`assets/models/`) are not in the repository. Without them the playground renders untextured and
skips the models.

[Render Paths & Visibility Buffer](../rendering/render-paths.md) explains `BOYKO_RENDER_PATH` and
`BOYKO_GEOMETRY_LEGS`, the two variables `run-scene.ps1` sets.

## Using the engine from your crate

Depend on the crates through git. For the ECS alone:

```toml
[dependencies]
boyko-ecs = { git = "https://github.com/bluesteelll/boyko-engine" }
boyko-macros = { git = "https://github.com/bluesteelll/boyko-engine" }
```

For a windowed app, add the host crate as well. `boyko-render` holds the renderer's configuration
resources (`RenderPathConfig`, `SsaoConfig`, `AaConfig`, ...), which the host crate does not
re-export:

```toml
[dependencies]
boyko-app = { git = "https://github.com/bluesteelll/boyko-engine" }
boyko-render = { git = "https://github.com/bluesteelll/boyko-engine" }
```

### The two imports

The ECS prelude exports the traits and types, but not the derive macros. `boyko_macros` is only a
dev-dependency of `boyko_ecs`, so its prelude cannot re-export them. Import the derives you use
from `boyko_macros` directly:

```rust,ignore
use boyko_ecs::prelude::*;                  // App, Commands, Query, Res, ...
use boyko_macros::{Component, Resource};   // #[derive(Component)], #[derive(Resource)], ...
```

A windowed scene adds `use boyko_app::prelude::*;`, which brings `EnginePlugins`, the mesh, light
and camera types, `Commands`, the non-send resource parameters and the math vocabulary. It does not
bring `Query`, `Res` or `ResMut`; those come from the ECS prelude. [Your First App](first-app.md) builds a complete ECS
program, and [Windowed Host](../app/windowed-host.md) builds a scene.

### Build settings that do not follow the dependency

Cargo reads `.cargo/config.toml` from the directory you build in and its parents, and profiles from
your own workspace's root manifest. Two of the repository's settings therefore stay behind:

- **AVX2.** Without `-C target-cpu=x86-64-v3`, the `cfg(target_feature = "avx2")` code paths compile
  out and the scalar fallbacks run. To keep them, add this to your project's `.cargo/config.toml`:

  ```toml
  [build]
  rustflags = ["-C", "target-cpu=x86-64-v3"]
  ```

- **Fat LTO.** Set `lto = "fat"` in your own `[profile.release]` if you want the configuration
  the repository builds with.

## Next steps

- [Your First App](first-app.md): components, systems and the app loop.
- [Windowed Host](../app/windowed-host.md): `EnginePlugins`, the examples and the renderer settings.
- [Benchmarks](../reference/benchmarks.md): the harnesses and where the results are published.
- [Contributing](../contributing.md): the test and lint commands.
