# Shadows & Ambient Occlusion

> Four shadow sources and two ambient-occlusion sources. The SDF marcher's own soft shadow and AO
> are on by default; the rest are turned on by inserting a config resource (and, for ray-query
> shadows, a build feature).

## The shadow sources

A pixel's direct-light visibility combines every source the resolved [render path](render-paths.md)
arms (`ShadowSources` in `ResolvedRenderPath`):

| Source | Light | Casters | Turn it on | Default |
|--------|-------|---------|------------|---------|
| Cascaded shadow maps (CSM) | the directional light | meshes with `ShadowCaster` | `CsmConfig { cascade_count: 1..=4, .. }` | off (`cascade_count: 0`) |
| Punctual shadow atlas | point and spot lights with `CastsPunctualShadow` | meshes with `ShadowCaster` | `ShadowConfig { enabled: true, .. }` | off |
| SDF soft march | direct lights | SDF shapes (the edit list) | needs the SDF geometry leg | on |
| Ray-query shadows (`hwrt`) | the directional light | meshes, through a per-frame TLAS | build with `--features hwrt`, run on a ray-query GPU, enable CSM | off |

Capability is structural: a mesh casts into the cascades and the atlas because it carries
`ShadowCaster`, and a spot or point light gets an atlas slot because it carries
`CastsPunctualShadow`. There is no
`cast_shadows` field to set: the markers are the switch.

```rust,ignore
use boyko_app::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("shadows", 800, 600));
    // Both configs default to disabled; insert them before `run`.
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
    app.add_startup_system(setup);
    app.run();
}

fn setup(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let cube = meshes.cube(dev.get(), 1.0);
    // This cube stamps into the sun's cascades and into the point light's atlas faces.
    commands
        .spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(0.0, 0.5, 0.0))))
        .insert(ShadowCaster);

    commands
        .spawn(PointLightObject {
            transform: Transform::from_translation(Vec3::new(2.8, 3.4, 1.6)),
            global: GlobalTransform::IDENTITY,
            light: PointLight::new([2.8, 3.4, 1.6], [1.0, 0.78, 0.52], 130.0, 11.0),
        })
        .insert(CastsPunctualShadow);
    // ... a DirectionalLightObject for the sun, a floor, a camera ...
}
```

The `punctual_room` and `showcase` examples show complete scenes.

### Cascaded shadow maps

`CsmConfig` fits up to `MAX_CASCADES` (4) cascades over the view range
([`csm_config.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/csm_config.rs)):

| Field | Default | Meaning |
|-------|---------|---------|
| `cascade_count` | `0` | Number of cascades; `0` disables CSM. |
| `resolution` | `2048` | Texels per cascade side. |
| `shadow_distance` | `30.0` | The far end of the last cascade, in world units. |
| `lambda` | `0.8` | Split blend: `0` uniform, `1` logarithmic. |
| `fit_mode` | `CatchAll` | `CatchAll` fits all but the last cascade to the casters and keeps the last for the rest of the range; `Shrink` fits every cascade to the casters; `Fixed` splits the camera range and ignores the casters. |
| `pcf_kernel` | `Tent13` | Edge filter: the 13-tap tent, or the sharper `Cross5`. |
| `normal_bias`, `depth_bias_constant`, `depth_bias_slope` | tuned | Acne control. |

### Punctual shadow atlas

Spot and point lights share one atlas of `M_SLOTS` (16) layers of `SHADOW_DIM` (512) texels by
default. A spot light takes one layer; a point light takes six, one per cube face. Each frame the
lights carrying `CastsPunctualShadow` are ranked by a priority estimate, and layers are handed out
until the pool is full. A light that does not fit gets no shadow that frame.
`ShadowConfig::dim` and the two depth-bias terms tune it.

### SDF soft shadows

The SDF geometry leg marches its own soft shadow through the edit list. It applies wherever the SDF
leg exists, on every render path. See [SDF Rendering](sdf.md).

On the Visibility Buffer path, `LightingConfig::vb_sdf_mesh_shadow` and `vb_sdf_mesh_ao` also let
SDF geometry shadow and occlude meshes. They are requests: the host arms them only when the boot
resolved a Visibility Buffer path with both legs on a device that can store the term's `RG8` image,
and they must be set before `run`.

### Ray-query shadows and the denoiser

Built with `--features hwrt` and run on a device that supports ray queries, the resolve traces the
directional light's mesh shadow with `rayQuery` against a top-level acceleration structure (TLAS)
rebuilt every frame, in place of the cascade-map lookup. The trace sits where the CSM sample would
be, so CSM must still be enabled. On any other build or device, the cascade map is sampled.
`RayShadowConfig` tunes the trace:

| Field | Meaning |
|-------|---------|
| `ray_count` | Rays per pixel; fixed when the pipeline is built, so a change needs a restart. |
| `cone_radius` | The cone the rays sample: the size of the light, and so the penumbra width. |
| `tmin`, `tmax`, `bias` | Ray extent and self-intersection offset. |

`ShadowDenoiseConfig::mode` adds a separate visibility pass and filters it: `None` (default),
`Spatial` (an edge-avoiding à-trous filter), `Temporal` (reprojection with a variance clamp) or
`Both`. The denoiser exists only in `hwrt` builds on a ray-query device, and it runs on frames where
the TLAS is non-empty and CSM is enabled with at least one caster. While it is armed, the denoised
visibility takes the place of the SDF soft-march term; the two are never combined. Under Forward and
Forward+ it is forced off; under the Visibility Buffer it needs the mesh leg.

## Ambient occlusion

| Source | What it darkens | Turn it on | Default |
|--------|-----------------|------------|---------|
| SDF marcher AO | SDF surfaces | part of the SDF leg | on |
| SSAO | every pixel, from the depth and normal buffers | `SsaoConfig { quality: Low \| Medium \| High, .. }` | off |

SSAO is a screen-space, HBAO-style pass followed by an edge-avoiding à-trous denoise:

```rust,ignore
use boyko_render::{SsaoConfig, SsaoQuality};

app.insert_resource(SsaoConfig { quality: SsaoQuality::High, ..SsaoConfig::default() });
```

`SsaoConfig::atrous_levels` (default `3`) sets the number of denoise passes; `0` turns the denoise
off. SSAO runs on the Deferred path and, with the mesh leg, on the Visibility Buffer path. Forward
and Forward+ force it off.

A material can also carry its own AO texture; see [Materials & Textures](materials-and-textures.md).

## Trade-offs

- **Everything is opt-in.** A default world pays nothing for shadow maps, SSAO or ray queries. The
  cost is that a scene must insert the configs it wants.
- **Configs are read at boot.** The boot-time resolve decides which sources a path arms from these
  configs, so insert them before `run`; a startup system is too late.
- **Ray queries cover the sun only.** Point and spot lights use the atlas or the SDF soft march.
- **Rendering has no frame-time benchmark**, so this page does not compare the sources' costs.

## See also

- [Lighting](lighting.md): the lights themselves and the light table.
- [Render Paths & Visibility Buffer](render-paths.md): which sources each path supports.
- [Global Illumination (SDF DDGI)](global-illumination.md): indirect light.
- Source: [`csm_config.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/csm_config.rs),
  [`shadow_atlas.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/shadow_atlas.rs),
  [`ssao_config.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/ssao_config.rs),
  [`shadow_denoise_config.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/shadow_denoise_config.rs),
  [`ray_shadow_config.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/ray_shadow_config.rs).
