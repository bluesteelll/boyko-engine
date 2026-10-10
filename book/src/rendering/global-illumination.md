# Global Illumination (SDF DDGI)

> Diffuse indirect light from a grid of irradiance probes, updated every frame by marching the SDF
> scene. Off by default.

## What it is

Direct lighting alone leaves every surface the lights cannot see at the flat sky ambient. Dynamic
Diffuse Global Illumination (DDGI) stores the light arriving at a grid of probes and lets every
pixel interpolate it, so light bounces: a red wall tints the floor beside it.

boyko-engine's version is built on the SDF geometry leg:

- the probes **trace the SDF edit list**, not the triangle meshes. Bounced light therefore comes
  from SDF shapes; meshes receive it but do not reflect it back into the probes;
- each traced hit is shaded with the scene's direct lights and the SDF soft shadow;
- the result lands in an octahedral irradiance atlas and a two-moment depth atlas, one tile per
  probe;
- the deferred resolve (and, on the Visibility Buffer path, the split shader) samples the probes
  around each pixel and adds the irradiance as diffuse indirect light.

The code is in `boyko_render` (`ddgi_config.rs`, `ddgi_update.rs`, `ddgi_plugin.rs`) and in the
shaders `sdf_probe_update.comp.hlsl` and `ddgi_resolve.hlsli` of `boyko_rhi_vulkan`.

## Turning it on

`EnginePlugins` adds `DdgiPlugin` with a disabled `DdgiConfig`. Overwrite it before `run`:

```rust,ignore
use boyko_app::prelude::*;
use boyko_render::DdgiConfig;

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("gi", 1280, 720));
    app.insert_resource(DdgiConfig {
        ddgi_indirect: true,
        origin: [-16.0, -2.0, -16.0], // world corner of probe (0, 0, 0)
        spacing: 2.0,                 // world units between neighbouring probes
        dims: [16, 8, 16],            // probes per axis
    });
    app.add_startup_system(setup);
    app.run();
}
# fn setup() {}
```

The values above are the defaults, so `DdgiConfig { ddgi_indirect: true, ..DdgiConfig::default() }`
gives the same grid. The grid is **fixed in world space**: it does not follow the camera, so place
it over the part of the scene that needs bounce light. A zero dimension or a spacing that is not a
positive normal number disables GI rather than reaching the GPU.

`scripts\run-vb-lab.ps1` runs a scene with DDGI on; the `vb_lab` example reads `BOYKO_GI` to toggle
it.

## Requirements

| Condition | Why |
|-----------|-----|
| The SDF geometry leg (`GeometryLegs::Both` or `Sdf`) | The probes march the SDF scene. |
| Deferred, or Visibility Buffer with `Both` legs | Forward and Forward+ force GI off at boot. See [Render Paths](render-paths.md). |
| The device supports storage writes to `B10G11R11` and `RG16F` images | Otherwise the atlas cannot be written, and GI stays off instead of failing the boot. |

## Tuning the update

`DdgiUpdateConfig` controls how the probes are refreshed:

| Field | Default | Meaning |
|-------|---------|---------|
| `rays_per_probe` | `64` | Rays each probe traces per update, up to `GI_MAX_RAYS` (128). |
| `subset_n` | `2` | Each frame updates `1 / subset_n` of the probes, round-robin. `1` updates every probe every frame. Must divide the probe count. |
| `gi_max_it` | `64` | Sphere-tracing steps per ray. One of the precompiled variants. |
| `hysteresis` | `0.95` | The share of the previous value a probe keeps per update: `lerp(fresh, previous, hysteresis)`. Higher is steadier and slower to react. |

The ray directions are a Fibonacci set that rotates a little every frame, so successive updates
see different directions and the temporal blend converges instead of flickering.

## Trade-offs

- **SDF-only bounce.** Meshes do not contribute to the probes. A scene built from meshes alone gets
  no indirect light from this system.
- **A fixed volume.** Outside the grid there is no GI. Larger areas need a larger `spacing`, which
  blurs the result.
- **Diffuse only.** There is no specular GI or reflection system; screen-space reflections are not
  implemented.
- **Rendering has no frame-time benchmark**, so this page gives no cost figures; `subset_n` and
  `rays_per_probe` are the knobs that trade cost for responsiveness.

## See also

- [SDF Rendering](sdf.md): the edit list the probes march.
- [Lighting](lighting.md): the direct lights the probes shade with.
- [Shadows & Ambient Occlusion](shadows-and-ao.md): the other lighting terms.
- Source: [`ddgi_config.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/ddgi_config.rs),
  [`ddgi_update.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/ddgi_update.rs),
  [`sdf_probe_update.comp.hlsl`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/shaders/sdf_probe_update.comp.hlsl).
