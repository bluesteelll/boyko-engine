# Anti-Aliasing

> Four anti-aliasing modes, selected with one resource: FXAA, SMAA, 2× supersampling and TAA (with
> an optional RCAS sharpen). The default is off.

## The modes

`AaConfig { mode: AaMode }` in `boyko_render`
([`aa_config.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/aa_config.rs))
selects the mode:

| `AaMode` | What it does | Selected |
|----------|--------------|----------|
| `Off` (default) | No AA pass; the present samples the lit image directly. | — |
| `Fxaa` | One full-screen pass that finds luma edges and blends across them. No history, no jitter. | at run time |
| `Smaa` | SMAA 1x at the HIGH preset: edge detection, blending weights, then neighbourhood blending (three passes and two lookup textures). Sharper diagonals and corners than FXAA. | at run time |
| `Ssaa` | The whole frame renders at 2× per axis (4× the pixels), then a linear-light box filter downsamples it. Treats geometry, shading and texture aliasing alike. | at boot |
| `Taa` | Jitters the camera by a sub-pixel offset each frame and accumulates a reprojected history. | at run time, on a path that supports it |

All modes write one shared `aa_out` image that the present pass then samples.

```rust,ignore
use boyko_app::prelude::*;
use boyko_render::{AaConfig, AaMode, SharpenMode, TaaConfig};

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("aa", 1280, 720));
    app.insert_resource(AaConfig { mode: AaMode::Taa });
    // Optional: sharpen the TAA output with RCAS.
    app.insert_resource(TaaConfig { sharpen: SharpenMode::Rcas, ..TaaConfig::default() });
    app.add_startup_system(setup);
    app.run();
}
# fn setup() {}
```

The SMAA shaders and lookup textures are third-party code under the MIT license; see
`THIRD-PARTY-NOTICES.md` in the repository root.

## Supersampling (SSAA)

SSAA changes the size of every render target, so it is fixed when the host boots. Ask for it on
the host instead of through `AaConfig`:

```rust,ignore
app.add_plugins(EnginePlugins::window("ssaa", 1280, 720).with_ssaa_scale(2));
```

Setting `BOYKO_AA=ssaa` in the environment does the same. Only a scale of `2` is honoured. At boot
the host checks the doubled size against the device's largest image dimension and a VRAM estimate.
If either check fails, it boots without SSAA instead of failing. Once the host has decided, it locks
the mode: an armed SSAA boot always runs `Ssaa`, and an `AaConfig` asking for `Ssaa` on a boot that
did not arm it gets `Off`.

## Temporal AA (TAA)

TAA spreads the cost of supersampling over time:

1. Each frame the projection is shifted by a sub-pixel offset from an 8-sample Halton (2, 3)
   sequence.
2. The resolve reprojects the previous result (the history) with the camera's motion, samples it
   with a Catmull-Rom filter, clips it against the current frame's local colour range (variance
   clipping), and blends it with the new frame.
3. Optionally, an RCAS pass (contrast-adaptive sharpening, after AMD FidelityFX CAS) restores
   detail the blend softened.

`TaaConfig` exposes the tunable parts:

| Field | Default | Meaning |
|-------|---------|---------|
| `jitter_scope` | `RasterAndBasis` | Jitters the raster and the camera basis the SDF marcher, the resolve and the shadow lookups use, so SDF pixels are anti-aliased too. `RasterOnly` jitters meshes only. |
| `clamp`, `clamp_space`, `clip`, `variance_gamma` | variance, RGB, toward the centre, `1.0` | How far the history may stray from the current frame. |
| `default_blend`, `min_blend`, `blend` | `0.1`, `0.015`, confidence-adaptive | How much of the new frame enters the history. |
| `luma_weight` | `true` | Weights samples by inverse luminance, so one bright sample cannot dominate. |
| `history_filter` | Catmull-Rom | The history reconstruction filter. |
| `sharpen`, `rcas_sharpness` | `None`, `0.25` | `SharpenMode::Rcas` adds the sharpen pass at that strength. |

Limits of the current TAA:

- **Motion comes from the camera only** (`MvSource::CameraOnly`). Objects that move on their own are
  reprojected as if they were static, so they can ghost or blur.
- **History is discarded only when the reprojected sample lands off-screen or behind the camera.**
  There is no depth-based disocclusion test yet.
- Quality in motion has not been signed off; static and slow camera moves are the tested case.

## Support by render path

| Path | FXAA, SMAA, SSAA | TAA |
|------|------------------|-----|
| Deferred | yes | yes |
| Visibility Buffer | yes | yes, every leg set |
| Forward, Forward+ | no: the Forward recorder has no AA pass, so the mode is set to `Off` | forced off at boot, with a `boyko-W3006` warning |

See [Render Paths & Visibility Buffer](render-paths.md).

## Trying the modes

`scripts\run-vb-lab.ps1` starts the `vb_lab` example, which reads `BOYKO_AA` (`off`, `fxaa`,
`smaa`, `taa`) and `BOYKO_TAA_SHARPEN=none`. Rendering has no frame-time benchmark, so this page
does not rank the modes by cost; their pass counts above are the structural difference.

## See also

- [Render Paths & Visibility Buffer](render-paths.md): which paths support AA.
- [Windowed Host](../app/windowed-host.md): `EnginePlugins::with_ssaa_scale`.
- [Shader eDSL](shader-edsl.md): how the engine's shaders are authored.
- Source: [`aa_config.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/aa_config.rs),
  [`taa_config.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/taa_config.rs),
  [`present/passes/`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_rhi_vulkan/src/present/passes).
